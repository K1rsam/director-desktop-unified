//! A full request can fit the frozen runtime while its doubled local envelope
//! exceeds the old 1 MiB pipe budget. Recover only that proven pre-write case.
use super::*;
use crate::text_transport::{AttemptState,Backend,FrozenAttempt,ObservationKind,TextTransport};
const FIELD:&str="sdkInputFrameRecovery";
const EVENT:&str="sdk_input_frame_recovery_reserved";
const VERSION:&str="sdk-input-frame-prewrite-recovery/v1";
const HELD:&str="SDK_INPUT_FRAME_RECOVERY_HELD: 原输入超限未写入证明不完整；保留原节点与预算，不重新提交";
fn hash(value:&Value)->String {crate::pipeline::digest(&value.to_string())}
fn events(c:&rusqlite::Connection,id:&str)->Result<Value> {
    let mut q=c.prepare("SELECT id,type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let rows=q.query_map([id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
    let mut out=Vec::new();for row in rows {let(id,kind,data)=row.map_err(|e|e.to_string())?;if out.len()>=4096{return Err(HELD.into());}out.push(json!({"id":id,"type":kind,"data":serde_json::from_str::<Value>(&data).map_err(|_|HELD)?}));}Ok(json!(out))
}
fn regular(path:&Path,limit:u64)->Result<Vec<u8>> {
    let meta=std::fs::symlink_metadata(path).map_err(|_|HELD)?;
    if !meta.is_file() || meta.is_symlink() || meta.len()>limit{return Err(HELD.into());}
    std::fs::read(path).map_err(|_|HELD.into())
}
fn absent(path:&Path)->bool {matches!(std::fs::symlink_metadata(path),Err(e) if e.kind()==std::io::ErrorKind::NotFound)}
/// Only reconstructs the original input serialization. This gives no retry
/// permission; the dispatcher below additionally proves exact pre-write failure.
fn input_attempt(original:&FrozenAttempt)->Result<FrozenAttempt> {
    original.validate_identity()?;
    let mut frozen=original.clone();frozen.state=AttemptState::Prepared;frozen.dispatch_started=false;
    frozen.last_sequence=0;frozen.thread_id=None;frozen.turn_id=None;frozen.raw_result=None;frozen.validate_identity()?;Ok(frozen)
}
/// Unforgeable outside this module: created only after the complete persisted
/// pre-write proof (or its reserved archive) has been verified. It permits the
/// original supervisor bytes for this one attempt, never a replacement identity.
#[derive(Clone)]
pub(crate) struct VerifiedInputFrameRecovery { frozen:FrozenAttempt }
impl VerifiedInputFrameRecovery {
    pub(crate) fn identity(&self)->&Value {&self.frozen.runtime_identity}
    pub(crate) fn verify_attempt(&self,attempt:&FrozenAttempt)->Result<()> {
        let frozen=input_attempt(attempt)?;
        if serde_json::to_value(&frozen).map_err(|_|HELD)?!=serde_json::to_value(&self.frozen).map_err(|_|HELD)? {return Err(HELD.into());}
        Ok(())
    }
}
fn step(c:&rusqlite::Connection,run:&str,key:&str,id:&str,sha:&str)->Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2 AND text_job_id=?3 AND request_hash=?4 AND stage='seriesRepair' AND state='submission_unknown' AND result IS NULL AND error='SDK_FRAME_LIMIT')",
        params![run,key,id,sha],|r|r.get(0)).map_err(|e|e.to_string())
}
fn pure_evidence(request:&TextRequest,snapshot:&Value,prepared:&Value,parent:&Value,original:&FrozenAttempt,rows:&Value)->Result<Value> {
    let frozen=input_attempt(original)?;
    let compiled=compile_sdk_attempt_input(&frozen)?;
    let expected=json!({"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),
        "sdkAttemptId":original.attempt_id,"sdkDispatchStarted":true,"sdkOriginWorkflowId":original.workflow_id,"sdkRequestHash":hash(snapshot)});
    if request.stage!="seriesRepair" || request.skill!="series-repair" || request.input["workflowProfile"]!=crate::asset_graph::PROFILE
        || request.input["validationRepair"]["version"]!="series-domain-repair/v1"
        || request.input["validationRepair"]["attempt"].as_u64().is_none_or(|n|!(1..=2).contains(&n))
        || original.state!=AttemptState::SubmissionUnknown || !original.dispatch_started || original.last_sequence!=0
        || original.thread_id.is_some() || original.turn_id.is_some() || original.raw_result.is_some()
        || original.backend!=Backend::CodexSdk || original.runtime_identity!=snapshot["executorIdentity"]
        || original.logical_node_id!=step_key(request) || serde_json::to_value(&original.request).map_err(|_|HELD)?!=serde_json::to_value(request).map_err(|_|HELD)?
        || prepared!=&expected || !frozen_text_parent_matches("storyboard.run",parent,request) {return Err(HELD.into());}
    let envelope=crate::text_sdk::execute_input_envelope(&frozen)?.ok_or(HELD)?;
    let rows=rows.as_array().ok_or(HELD)?;
    if rows.len()!=4 || rows.iter().enumerate().any(|(i,e)|e["id"].as_i64().is_none_or(|n|n<=0)||(i>0 && rows[i-1]["id"].as_i64()>=e["id"].as_i64()))
        || rows[0]["type"]!="sdk_pre_dispatch_queued" || rows[1]["type"]!="sdk_process_tree" || rows[2]["type"]!="sdk_stderr" || rows[3]["type"]!="state"
        || rows[0]["data"]!=json!({"version":"sdk-pre-dispatch-queued/v1","workflowId":original.workflow_id,"nodeKey":original.logical_node_id,"stage":request.stage,"requestSha256":hash(snapshot),
            "parentSnapshotSha256":hash(parent),"settingsSha256":hash(&parent["input"]["settings"]),"modelCalls":0})
        || rows[1]["data"]!=json!({"activeProcesses":0,"memoryLimitBytes":1_073_741_824u64,"processLimit":8,"processTreeStopProven":true,"totalProcesses":2})
        || rows[2]["data"]!=json!({"bytes":0,"format":"sdk-sidecar-stderr/v1","retainedBytes":0,"retainedHex":"","scope":"Node sidecar stderr only; not SDK-internal child stderr","truncated":false})
        || rows[3]["data"]!=json!({"error":"SDK_FRAME_LIMIT","state":"submission_unknown"}) {return Err(HELD.into());}
    Ok(envelope)
}

impl SavedTextExecutor {
    fn sdk_frame_recovery_evidence(&self,request:&TextRequest,snapshot:&Value,job:&Value,original:&FrozenAttempt)->Result<Value> {
        let id=s(job,"id");let run=self.workflow_id.as_deref().ok_or(HELD)?;
        if uuid::Uuid::parse_str(id).is_err() || job["status"]!="submission_unknown" || job["error"]!="SDK_FRAME_LIMIT"
            || !job["receipt"].is_null() || !job["result"].is_null() || job["snapshot"]!=*snapshot
            || original.attempt_id!=id || original.workflow_id!=run || !absent(&self.store.root.join("sdk-worker").join(id)) {return Err(HELD.into());}
        self.sdk_parent_binding()?;self.sdk_checkpoint_binding(request,snapshot)?;self.sdk_schema_job_bound(job,snapshot)?;
        if !step(&self.store.conn()?,run,&step_key(request),id,&hash(snapshot))?{return Err(HELD.into());}
        let parent=self.store.job(run)?;let rows=events(&self.store.conn()?,id)?;
        let envelope=pure_evidence(request,snapshot,&job["prepared"],&parent["snapshot"],original,&rows)?;
        let base=self.store.root.join("text-runs").join(id);
        let body=regular(&base.join("sdk-attempt.json"),4_194_304)?;
        if serde_json::to_value(FrozenAttempt::load(&base.join("sdk-attempt.json"))?).map_err(|_|HELD)?!=serde_json::to_value(original).map_err(|_|HELD)? {return Err(HELD.into());}
        let compiled=compile_sdk_attempt_input(original)?;
        let bytes=regular(&base.join("sdk-compiled.json"),2_097_152)?;
        let expected=json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)});
        if serde_json::from_slice::<Value>(&bytes).map_err(|_|HELD)?!=expected {return Err(HELD.into());}
        Ok(json!({"version":VERSION,"workflowId":run,"nodeKey":step_key(request),"requestSha256":hash(snapshot),"originalPrepared":job["prepared"],
            "parentSnapshotSha256":hash(&parent["snapshot"]),"originalAttemptSha256":crate::store::hash_file(&base.join("sdk-attempt.json"))?.0,
            "originalAttemptBytes":body.len(),"compiledFileSha256":crate::store::hash_file(&base.join("sdk-compiled.json"))?.0,"events":rows,"inputEnvelope":envelope,
            "originalError":"SDK_FRAME_LIMIT","executeBytesWritten":0,"generationRequestsBeforeRecovery":0,"basis":"exact-oversized-execute-before-stdin-write/v1"}))
    }
    fn sdk_frame_recovery_bound(&self,job:&Value,snapshot:&Value)->Result<Option<Value>> {
        let Some(saved)=job["prepared"].get(FIELD) else{return Ok(None);};
        let id=s(job,"id");let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
        let rows=events(&self.store.conn()?,id)?;let records:Vec<_>=rows.as_array().ok_or(HELD)?.iter().filter(|e|e["type"]==EVENT).collect();
        let base=self.store.root.join("text-runs").join(id);let archive=base.join("sdk-input-frame-original.json");
        let original=FrozenAttempt::load(&archive)?;let parent=self.store.job(run)?;
        let envelope=pure_evidence(&request,snapshot,&saved["originalPrepared"],&parent["snapshot"],&original,&saved["events"])?;
        let mut prepared=job["prepared"].clone();for field in [FIELD,"sdkInputEnvelope","sdkRawTextSha256","sdkDomainResult","sdkDomainResultSha256"] {prepared.as_object_mut().ok_or(HELD)?.remove(field);}
        let old_rows=saved["events"].as_array().ok_or(HELD)?;
        if saved.as_object().is_none_or(|m|m.len()!=15) || saved["version"]!=VERSION || saved["workflowId"]!=run || saved["nodeKey"]!=step_key(&request)
            || saved["requestSha256"]!=hash(snapshot) || saved["parentSnapshotSha256"]!=hash(&parent["snapshot"])
            || saved["originalError"]!="SDK_FRAME_LIMIT" || saved["executeBytesWritten"]!=0 || saved["generationRequestsBeforeRecovery"]!=0 || saved["basis"]!="exact-oversized-execute-before-stdin-write/v1"
            || records.len()!=1 || records[0]["data"]!=*saved || original.attempt_id!=id || original.workflow_id!=run
            || rows.as_array().unwrap().get(..old_rows.len())!=Some(old_rows.as_slice())
            || prepared!=saved["originalPrepared"] || job["prepared"]["sdkInputEnvelope"]!=envelope || saved["inputEnvelope"]!=envelope
            || crate::store::hash_file(&archive)?.0!=s(saved,"originalAttemptSha256") || regular(&archive,4_194_304)?.len() as u64!=saved["originalAttemptBytes"].as_u64().ok_or(HELD)?
            || crate::store::hash_file(&base.join("sdk-compiled.json"))?.0!=s(saved,"compiledFileSha256") {return Err(HELD.into());}
        Ok(Some(saved.clone()))
    }
    pub(super) fn sdk_input_envelope_bound(&self,job:&Value,snapshot:&Value)->Result<()> {
        self.sdk_frame_recovery_bound(job,snapshot)?;
        let rows=events(&self.store.conn()?,s(job,"id"))?;
        let chosen:Vec<_>=rows.as_array().ok_or(HELD)?.iter().filter(|e|e["type"]=="sdk_input_envelope_selected").collect();
        let Some(saved)=job["prepared"].get("sdkInputEnvelope") else {return if chosen.is_empty(){Ok(())}else{Err(HELD.into())};};
        let attempt=FrozenAttempt::load(&self.store.root.join("text-runs").join(s(job,"id")).join("sdk-attempt.json"))?;
        let expected=crate::text_sdk::execute_input_envelope(&input_attempt(&attempt)?)?.ok_or(HELD)?;
        let acks:Vec<_>=rows.as_array().unwrap().iter().filter(|e|e["type"]=="sdk_dispatch_receipt").collect();
        if *saved!=expected || chosen.len()!=1 || chosen[0]["data"]!=expected || acks.len()!=1
            || acks[0]["data"]["result"]["inputEnvelope"]!=expected["descriptor"] || chosen[0]["id"].as_i64()>=acks[0]["id"].as_i64() {return Err(HELD.into());}
        Ok(())
    }
    fn reserve_sdk_input_frame(&self,request:&TextRequest,snapshot:&Value,job:&Value,original:&FrozenAttempt,evidence:&Value)->Result<()> {
        use std::io::Write;
        let id=s(job,"id");let run=self.workflow_id.as_deref().ok_or(HELD)?;
        if !absent(&self.store.root.join("sdk-worker/worker.lock")) || !absent(&self.store.root.join("sdk-worker/worker.lock.recovery")) {
            return Err("SDK_INPUT_FRAME_PROBE_STILL_OWNS_LOCK: 本地能力探测尚未释放工作目录；未预约或派发".into());
        }
        let mut c=self.store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let same:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk' AND state='submission_unknown' AND error='SDK_FRAME_LIMIT' AND snapshot=?2 AND prepared=?3 AND receipt IS NULL AND result IS NULL)",params![id,snapshot.to_string(),job["prepared"].to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
        let other:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id<>?1 AND kind='text' AND provider='codex_sdk' AND state NOT IN ('completed','rejected') AND json_extract(prepared,'$.sdkDispatchStarted')=1)",[id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !same || other || !step(&tx,run,&step_key(request),id,&hash(snapshot))? || self.sdk_frame_recovery_evidence(request,snapshot,job,original)?!=*evidence {return Err(HELD.into());}
        let base=self.store.root.join("text-runs").join(id);let body=regular(&base.join("sdk-attempt.json"),4_194_304)?;
        let archive=base.join("sdk-input-frame-original.json");
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&archive) {
            Ok(mut f)=>{f.write_all(&body).map_err(|e|e.to_string())?;f.sync_all().map_err(|e|e.to_string())?;},
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{if regular(&archive,4_194_304)?!=body{return Err(HELD.into());}},Err(e)=>return Err(e.to_string())
        }
        let mut prepared=job["prepared"].clone();prepared[FIELD]=evidence.clone();prepared["sdkInputEnvelope"]=evidence["inputEnvelope"].clone();
        let n=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND prepared=?4",params![prepared.to_string(),now(),id,job["prepared"].to_string()]).map_err(|e|e.to_string())?;
        if n!=1{return Err(HELD.into());}
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![id,EVENT,evidence.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())
    }
    pub(super) async fn sdk_recover_input_frame(&self,request:&TextRequest,snapshot:&Value,job:&Value,original:&FrozenAttempt)->Result<Option<Value>> {
        if job["prepared"].get(FIELD).is_none() && (job["error"]!="SDK_FRAME_LIMIT" || job["prepared"].get("sdkInputEnvelope").is_some()) {return Ok(None);}
        let id=s(job,"id");let prior=self.sdk_frame_recovery_bound(job,snapshot)?;
        let evidence=match &prior {Some(value)=>value.clone(),None=>self.sdk_frame_recovery_evidence(request,snapshot,job,original)?};
        let archived=if prior.is_some(){FrozenAttempt::load(&self.store.root.join("text-runs").join(id).join("sdk-input-frame-original.json"))?}else{original.clone()};
        let permit=VerifiedInputFrameRecovery{frozen:input_attempt(&archived)?};permit.verify_attempt(original)?;
        let config=crate::text_sdk::Config::from_verified_input_frame_recovery(&self.settings,&self.store.root,permit)?;
        let mut transport=crate::text_sdk::OfficialSdkTransport::new(config,Arc::new(|_,_|Ok(())));
        let observed=if prior.is_some() {
            transport.set_observer(self.sdk_observer(id));transport.reconcile(original).await
        } else {
            let frozen=input_attempt(original)?;
            let selected=transport.probe_execute_envelope(&frozen).await?.ok_or(HELD)?;
            if selected!=evidence["inputEnvelope"]{return Err(HELD.into());}
            self.reserve_sdk_input_frame(request,snapshot,job,original,&evidence)?;
            transport.set_observer(self.sdk_observer(id));transport.execute(&frozen).await
        };
        transport.close().await?;
        let observation=observed?;
        if !matches!(observation.kind,ObservationKind::Completed|ObservationKind::Rejected) {return Err(HELD.into());}
        let file=self.store.root.join("text-runs").join(id).join("sdk-attempt.json");let mut attempt=FrozenAttempt::load(&file)?;
        attempt.observe(observation)?;attempt.persist(&file)?;
        if attempt.state==AttemptState::Rejected {
            self.sdk_record_schema_refusal(id,request,snapshot)?;
            return Err("SDK_SCHEMA_REJECTED: 原输入包恢复后收到明确格式拒绝；保留原回执，不重发".into());
        }
        Ok(Some(self.sdk_receive(request,snapshot,id,&attempt)?))
    }
}

#[cfg(test)]
pub(crate) fn test_input_frame_recovery_permit(settings:Value)->(tempfile::TempDir,VerifiedInputFrameRecovery,FrozenAttempt) {
    let(temp,executor,request,id,snapshot,attempt)=tests::fixture_with_settings(settings);
    executor.sdk_frame_recovery_evidence(&request,&snapshot,&executor.sdk_job(&id).unwrap(),&attempt).unwrap();
    let permit=VerifiedInputFrameRecovery{frozen:input_attempt(&attempt).unwrap()};
    (temp,permit,attempt)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(tempfile::TempDir,SavedTextExecutor,TextRequest,String,Value,FrozenAttempt) {
        fixture_with_settings(crate::text_sdk::offline_settings("happy"))
    }
    pub(super) fn fixture_with_settings(settings:Value)->(tempfile::TempDir,SavedTextExecutor,TextRequest,String,Value,FrozenAttempt) {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("合成大输入恢复","synthetic-frame").unwrap();store.ensure_workflow_storage().unwrap();
        let request=TextRequest{stage:"seriesRepair".into(),skill:"series-repair".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),
            input:json!({"workflowProfile":crate::asset_graph::PROFILE,"executionRoleId":"e1/finisher/0/validation-repair/1","freshContext":true,"sourceSha256":"synthetic-source",
                "validationRepair":{"version":"series-domain-repair/v1","attempt":1,"maximumAttempts":2},"completeFrozenMaterial":"完整素材".repeat(20_000)}),
            schema:json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),rule_snapshot:crate::pipeline::series::repair_rules("finisher",false).unwrap()};
        let parent=json!({"runnerVersion":"desktop-runner/v1","input":{"settings":settings,"workflowProfile":crate::asset_graph::PROFILE,"source":{"sha256":"synthetic-source"}}});
        let run=store.add_job("workflow","storyboard.run","local","local",&parent,"running").unwrap();let executor=SavedTextExecutor{store,settings,workflow_id:Some(run.clone())};
        let snapshot=executor.request_snapshot(&request).unwrap();let id=executor.sdk_allocate_pre_dispatch(&request,&snapshot).unwrap();
        let mut attempt=FrozenAttempt::prepare_sdk_strict_schema(&id,&run,&step_key(&request),request.clone(),&snapshot["executorIdentity"]).unwrap();
        assert!(crate::text_sdk::execute_input_envelope(&attempt).unwrap().is_some());
        let compiled=compile_sdk_attempt_input(&attempt).unwrap();let base=executor.store.root.join("text-runs").join(&id);std::fs::create_dir_all(&base).unwrap();
        crate::store::atomic_json(&base.join("sdk-compiled.json"),&json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)})).unwrap();
        let prepared=json!({"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),"sdkAttemptId":id,"sdkDispatchStarted":true,"sdkOriginWorkflowId":run,"sdkRequestHash":hash(&snapshot)});
        executor.store.conn().unwrap().execute("UPDATE jobs SET state='submission_unknown',error='SDK_FRAME_LIMIT',prepared=?1 WHERE id=?2",params![prepared.to_string(),id]).unwrap();
        executor.store.checkpoint(&run,&step_key(&request),&request.stage,&hash(&snapshot),"submission_unknown",Some(&id),None,Some("SDK_FRAME_LIMIT")).unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.state=AttemptState::SubmissionUnknown;attempt.persist(&base.join("sdk-attempt.json")).unwrap();
        executor.store.event(&id,"sdk_process_tree",&json!({"activeProcesses":0,"memoryLimitBytes":1_073_741_824u64,"processLimit":8,"processTreeStopProven":true,"totalProcesses":2})).unwrap();
        executor.store.event(&id,"sdk_stderr",&json!({"bytes":0,"format":"sdk-sidecar-stderr/v1","retainedBytes":0,"retainedHex":"","scope":"Node sidecar stderr only; not SDK-internal child stderr","truncated":false})).unwrap();
        executor.store.event(&id,"state",&json!({"error":"SDK_FRAME_LIMIT","state":"submission_unknown"})).unwrap();
        (temp,executor,request,id,snapshot,attempt)
    }
    #[test]
    fn sdk_large_input_prewrite_recovery_reserves_original_once_and_reopens_without_erasing_unknown() {
        let(_temp,executor,request,id,snapshot,attempt)=fixture();let old=executor.sdk_job(&id).unwrap();
        let original=std::fs::read(executor.store.root.join("text-runs").join(&id).join("sdk-attempt.json")).unwrap();
        let evidence=executor.sdk_frame_recovery_evidence(&request,&snapshot,&old,&attempt).unwrap();
        assert_eq!(evidence["executeBytesWritten"],0);assert!(evidence["inputEnvelope"]["maximumFrameBytes"].as_u64().unwrap()>1_048_576);
        executor.reserve_sdk_input_frame(&request,&snapshot,&old,&attempt,&evidence).unwrap();
        assert!(executor.reserve_sdk_input_frame(&request,&snapshot,&old,&attempt,&evidence).is_err(),"one reservation only");
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};let saved=reopened.sdk_job(&id).unwrap();
        assert_eq!(reopened.sdk_frame_recovery_bound(&saved,&snapshot).unwrap(),Some(evidence));
        assert_eq!(saved["status"],"submission_unknown");assert_eq!(saved["error"],"SDK_FRAME_LIMIT");assert!(saved["result"].is_null());
        assert_eq!(saved["snapshot"],old["snapshot"]);assert_eq!(reopened.store.jobs().unwrap().len(),2);
        assert_eq!(std::fs::read(executor.store.root.join("text-runs").join(&id).join("sdk-input-frame-original.json")).unwrap(),original);
        assert_eq!(std::fs::read(executor.store.root.join("text-runs").join(&id).join("sdk-attempt.json")).unwrap(),original);
        assert!(!executor.store.root.join("sdk-worker").join(&id).exists(),"proof/reservation alone does not start SDK");
    }
    #[test]
    fn sdk_large_input_prewrite_recovery_requires_size_stop_event_and_absent_original_receipt_together() {
        let(_temp,executor,request,id,snapshot,attempt)=fixture();let job=executor.sdk_job(&id).unwrap();
        let parent=executor.store.job(executor.workflow_id.as_deref().unwrap()).unwrap()["snapshot"].clone();let rows=events(&executor.store.conn().unwrap(),&id).unwrap();
        for fault in ["ack","stop","stderr","error","rule","thread","sequence"] {
            let mut changed=rows.clone();let mut frozen=attempt.clone();let mut req=request.clone();
            match fault {
                "ack"=>changed.as_array_mut().unwrap().push(json!({"id":999,"type":"sdk_dispatch_receipt","data":{}})),
                "stop"=>changed[1]["data"]["processTreeStopProven"]=json!(false),
                "stderr"=>changed[2]["data"]["bytes"]=json!(1),"error"=>changed[3]["data"]["error"]=json!("SDK_INPUT_TIMEOUT"),
                "rule"=>req.rule_snapshot=json!({}),"thread"=>frozen.thread_id=Some("possibly-submitted".into()),"sequence"=>frozen.last_sequence=1,_=>unreachable!()
            }
            assert!(pure_evidence(&req,&snapshot,&job["prepared"],&parent,&frozen,&changed).is_err(),"{fault}");
        }
        let worker=executor.store.root.join("sdk-worker").join(&id);std::fs::create_dir_all(&worker).unwrap();
        let before=executor.store.jobs().unwrap();assert!(executor.sdk_frame_recovery_evidence(&request,&snapshot,&job,&attempt).is_err());assert_eq!(executor.store.jobs().unwrap(),before);
        let mut short=request.clone();short.input["completeFrozenMaterial"]=json!("");short.rule_snapshot=json!({"primary":{"id":"synthetic","content":"short"}});
        let short=FrozenAttempt::prepare_sdk_strict_schema(&id,executor.workflow_id.as_deref().unwrap(),&step_key(&short),short,&snapshot["executorIdentity"]).unwrap();
        assert!(crate::text_sdk::execute_input_envelope(&short).unwrap().is_none(),"absence of a worker does not prove a small frame was never sent");
    }
    #[test]
    fn sdk_large_input_preflight_rejects_runtime_prompt_overflow_without_allocating_or_spawning() {
        let(_temp,executor,mut request,id,snapshot,_attempt)=fixture();let before=executor.store.jobs().unwrap();
        request.input["completeFrozenMaterial"]=json!("x".repeat(1_048_577));
        let attempt=FrozenAttempt::prepare_sdk_strict_schema(&id,executor.workflow_id.as_deref().unwrap(),&step_key(&request),request,&snapshot["executorIdentity"]).unwrap();
        assert!(crate::text_sdk::execute_input_envelope(&attempt).unwrap_err().contains("BEFORE_DISPATCH"));
        assert_eq!(executor.store.jobs().unwrap(),before);assert!(!executor.store.root.join("sdk-worker").join(&id).exists());
    }
    #[tokio::test]
    async fn sdk_large_input_actual_probe_closes_before_reservation_then_executes_original_once() {
        let(_temp,executor,request,id,snapshot,attempt)=fixture();let original=executor.sdk_job(&id).unwrap();
        let before=regular(&executor.store.root.join("text-runs").join(&id).join("sdk-attempt.json"),4_194_304).unwrap();
        let result=executor.sdk_recover_input_frame(&request,&snapshot,&original,&attempt).await.unwrap().unwrap();
        assert_eq!(result["ok"],true);assert_eq!(executor.store.jobs().unwrap().len(),2);
        let saved=executor.sdk_job(&id).unwrap();assert_eq!(saved["status"],"received");
        assert!(absent(&executor.store.root.join("sdk-worker/worker.lock")));
        assert_eq!(regular(&executor.store.root.join("text-runs").join(&id).join("sdk-input-frame-original.json"),4_194_304).unwrap(),before);
        let rows=events(&executor.store.conn().unwrap(),&id).unwrap();let rows=rows.as_array().unwrap();
        let position=|kind:&str|rows.iter().position(|r|r["type"]==kind).unwrap();
        assert!(position(EVENT)<position("sdk_input_envelope_selected"));assert!(position("sdk_input_envelope_selected")<position("sdk_dispatch_receipt"));
        assert_eq!(rows.iter().filter(|r|r["type"]==EVENT).count(),1);assert_eq!(rows.iter().filter(|r|r["type"]=="sdk_dispatch_receipt").count(),1);
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.sdk_raw(&reopened.sdk_job(&id).unwrap(),&snapshot).unwrap(),result);
        assert_eq!(events(&reopened.store.conn().unwrap(),&id).unwrap(),json!(rows),"saved receipt replay starts no worker or submission");
    }
    #[tokio::test]
    async fn sdk_large_input_failed_probe_closes_lock_without_reservation_or_dispatch() {
        let(_temp,executor,request,id,snapshot,attempt)=fixture_with_settings(crate::text_sdk::offline_settings("probe-fails"));
        let original=executor.sdk_job(&id).unwrap();let before=events(&executor.store.conn().unwrap(),&id).unwrap();
        let error=executor.sdk_recover_input_frame(&request,&snapshot,&original,&attempt).await.unwrap_err();assert!(error.contains("SYNTHETIC_PROBE_FAILURE"),"{error}");
        assert!(absent(&executor.store.root.join("sdk-worker/worker.lock")));assert!(absent(&executor.store.root.join("sdk-worker").join(&id)));
        assert_eq!(executor.sdk_job(&id).unwrap(),original);assert_eq!(events(&executor.store.conn().unwrap(),&id).unwrap(),before);
        assert!(absent(&executor.store.root.join("text-runs").join(&id).join("sdk-input-frame-original.json")));
    }
}
