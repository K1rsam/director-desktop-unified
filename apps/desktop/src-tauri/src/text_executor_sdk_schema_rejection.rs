//! One corrected transport attempt for a proven uniqueItems API refusal. The
//! original logical node, domain request, parent and revision budget stay bound;
//! original receipt/journal/compiled/attempt bytes are never edited.
use super::*;
use crate::text_transport::FrozenAttempt;
const RETRY: &str = "sdkSchemaRejectionRetry";
const CONTINUE: &str = "sdkSchemaContinuation";
const POLICY: &str = "sdk-schema-refusal-successor/v1";
const CONTINUATION: &str = "sdk-schema-refusal-continuation/v1";
const ACCOUNT_CONTINUATION: &str = "sdk-current-account-continuation/v2";
const DOMAIN_REPAIR: &str = "series-domain-repair/v1";
#[path = "text_executor_sdk_series_supplement.rs"]
mod series_supplement;
#[path = "text_executor_sdk_supply_correction.rs"]
mod supply_correction;
#[path = "text_executor_sdk_series_repair_replay.rs"]
mod repair_replay;
#[path = "text_executor_sdk_series_review_replay.rs"]
mod review_replay;
const EVENT: &str = "sdk_schema_rejection_successor";
const HELD: &str = "SDK_SCHEMA_REJECTION_HELD: 确定拒绝后的原节点恢复证明不完整；未新增请求或改写历史";
const REFUSED: &str = "SDK_SCHEMA_REJECTED: 服务端已明确拒绝结构化输出格式；原证据保留，未生成正文";
fn hash(value: &Value) -> String { crate::pipeline::digest(&value.to_string()) }
fn sealed(mut proof: Value) -> Value { proof["sha256"] = json!(hash(&proof)); proof }
fn unsigned_hash(proof: &Value) -> String { let mut value=proof.clone();if let Some(fields)=value.as_object_mut(){fields.remove("sha256");}hash(&value) }
fn domain(snapshot: &Value) -> Result<Value> {
    let mut value=snapshot.clone();let fields=value.as_object_mut().ok_or(HELD)?;
    fields.remove("executorIdentity");fields.remove(RETRY);fields.remove(CONTINUE);Ok(value)
}
fn events(c: &rusqlite::Connection, job: &str) -> Result<Value> {
    let mut q=c.prepare("SELECT id,type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let rows=q.query_map([job],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
    let mut all=Vec::new();for row in rows{let(id,kind,data)=row.map_err(|e|e.to_string())?;if all.len()>=4096{return Err(HELD.into());}all.push(json!({"id":id,"type":kind,"data":serde_json::from_str::<Value>(&data).map_err(|_|HELD)?}));}Ok(json!(all))
}
fn original_events(rows: &Value) -> Value {
    json!(rows.as_array().unwrap_or(&Vec::new()).iter().take_while(|row|row["type"]!="sdk_schema_refused" && row["type"]!=EVENT).cloned().collect::<Vec<_>>())
}
fn parent(c: &rusqlite::Connection, run: &str, settings: &Value) -> Result<String> {
    let raw:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow'",[run],|r|r.get(0)).map_err(|_|HELD)?;
    let value:Value=serde_json::from_str(&raw).map_err(|_|HELD)?;
    if value.pointer("/input/settings").or_else(||value.get("settings"))!=Some(settings){return Err(HELD.into());}Ok(crate::pipeline::digest(&raw))
}
fn anchor_summary(anchor:&Value)->Value {
    json!({"jobId":anchor["id"],"snapshotSha256":hash(&anchor["snapshot"]),"preparedSha256":hash(&anchor["prepared"]),
        "receiptSha256":hash(&anchor["receipt"]),"resultSha256":hash(&anchor["result"]),"retryProofSha256":anchor["snapshot"][RETRY]["sha256"]})
}
fn request_hash(request:&TextRequest)->Result<String> {
    Ok(crate::pipeline::digest(&serde_json::to_string(request).map_err(|_|HELD)?))
}
fn proof_basic(proof: &Value, snapshot: &Value, settings: &Value, run: &str, key: &str, policy: &str) -> bool {
    let account=policy==CONTINUATION && proof["version"]==ACCOUNT_CONTINUATION;
    let identity_bound=if account {
        serde_json::from_value::<TextRequest>(snapshot.clone()).is_ok_and(|request|
            proof.as_object().is_some_and(|fields|fields.len()==11)
            && !proof["accountAuthorization"]["previousNode"].is_null()
            && sdk_account_handoff::shape_bound(&proof["accountAuthorization"],settings,&snapshot["executorIdentity"],&request,CONTINUATION)
            && crate::text_sdk::current_account_receipt_identity_bound(&settings["textSdkIdentity"],&snapshot["executorIdentity"],"schema"))
    }else{proof.get("accountAuthorization").is_none()
        && crate::text_sdk::schema_rejection_receipt_identity_bound(&settings["textSdkIdentity"],&snapshot["executorIdentity"])};
    (proof["version"]==policy || account) && proof["sha256"]==unsigned_hash(proof)
        && proof["workflowId"]==run && proof["nodeKey"]==key
        && proof["domainRequestSha256"]==domain(snapshot).map(|v|hash(&v)).unwrap_or_default()
        && proof["originalSettingsSha256"]==hash(settings)
        && proof["effectiveIdentitySha256"]==hash(&snapshot["executorIdentity"])
        && identity_bound
}
impl SavedTextExecutor {
    fn sdk_schema_origin_evidence(&self, job: &Value, request: &TextRequest) -> Result<Value> {
        let id=s(job,"id");let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let attempt=FrozenAttempt::load(&self.store.root.join("text-runs").join(id).join("sdk-attempt.json"))?;
        if attempt.attempt_id!=id || attempt.workflow_id!=run || attempt.logical_node_id!=step_key(request)
            || attempt.runtime_identity!=job["snapshot"]["executorIdentity"]
            || serde_json::to_value(&attempt.request).map_err(|_|HELD)?!=serde_json::to_value(request).map_err(|_|HELD)?
            || job["prepared"]["sdkDispatchStarted"]!=true || job["prepared"]["sdkAttemptId"]!=id
            || job["prepared"]["sdkOriginWorkflowId"]!=run || job["prepared"]["sdkRequestHash"]!=hash(&job["snapshot"])
            || !job["result"].is_null() {return Err(HELD.into());}
        let rows=original_events(&events(&self.store.conn()?,id)?);
        crate::text_sdk::schema_rejection_evidence(&self.store.root,&attempt,&rows)
    }
    fn sdk_schema_retry_bound(&self, snapshot: &Value) -> Result<()> {
        let proof=&snapshot[RETRY];let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
        if !proof_basic(proof,snapshot,&self.settings,run,&step_key(&request),POLICY)
            || proof["attempt"]!=1 || proof["maximumAttempts"]!=1
            || parent(&self.store.conn()?,run,&self.settings)?!=s(proof,"parentSnapshotSha256") {return Err(HELD.into());}
        let original=self.sdk_job(s(proof,"originJobId"))?;
        if original["status"]!="rejected" || original["receipt"]["transportValidation"]!="schema_rejected"
            || original["receipt"]["successorProofSha256"]!=proof["sha256"]
            || hash(&original["snapshot"])!=s(proof,"originSnapshotSha256")
            || domain(&original["snapshot"])?!=domain(snapshot)?
            || self.sdk_schema_origin_evidence(&original,&request)?!=proof["evidence"] {return Err(HELD.into());}
        let rows=events(&self.store.conn()?,s(proof,"originJobId"))?;
        let matches:Vec<_>=rows.as_array().unwrap().iter().filter(|row|row["type"]==EVENT).collect();
        if matches.len()!=1 || matches[0]["data"]["proof"]!=*proof {return Err(HELD.into());}
        Ok(())
    }
    pub(super) fn sdk_schema_saved_snapshot(&self,request:&TextRequest,expected_domain:&Value)->Result<Option<Value>> {
        #[cfg(test)]
        let _profile = validation_profile::enter("sdk_schema_saved_snapshot", s(&request.input, "executionRoleId"));
        if self.settings["textProvider"]!="codex_sdk" {return Ok(None);}
        let Some(run)=self.workflow_id.as_deref() else{return Ok(None);};
        let c=self.store.conn()?;
        let row:Option<(String,String,String)>=c.query_row("SELECT j.snapshot,w.request_hash,j.id FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND j.kind='text' AND j.provider='codex_sdk'",params![run,step_key(request)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
        let Some((raw,request_hash,job))=row else {
            let orphan:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND provider='codex_sdk' AND (json_extract(snapshot,'$.sdkSchemaRejectionRetry.workflowId')=?1 AND json_extract(snapshot,'$.sdkSchemaRejectionRetry.nodeKey')=?2 OR json_extract(snapshot,'$.sdkSchemaContinuation.workflowId')=?1 AND json_extract(snapshot,'$.sdkSchemaContinuation.nodeKey')=?2))",params![run,step_key(request)],|r|r.get(0)).map_err(|e|e.to_string())?;
            if orphan{return Err(HELD.into());}return Ok(None);
        };
        let saved:Value=serde_json::from_str(&raw).map_err(|_|HELD)?;
        if saved.get(RETRY).is_none() && saved.get(CONTINUE).is_none(){return Ok(None);}
        if saved.get(RETRY).is_some() && saved.get(CONTINUE).is_some(){return Err(HELD.into());}
        if saved.to_string()!=raw || crate::pipeline::digest(&raw)!=request_hash || domain(&saved)?!=*expected_domain {return Err(HELD.into());}
        if saved.get(RETRY).is_some() {
            self.sdk_schema_retry_bound(&saved)?;
            let origin=self.sdk_job(s(&saved[RETRY],"originJobId"))?;
            if origin["receipt"]["successorJobId"]!=job{return Err(HELD.into());}
        } else {self.sdk_schema_continuation_bound(&saved)?;}
        Ok(Some(saved))
    }
    pub(super) fn sdk_schema_job_bound(&self,job:&Value,snapshot:&Value)->Result<()> {
        if snapshot.get(RETRY).is_none() && snapshot.get(CONTINUE).is_none(){return Ok(());}
        let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
        if job["snapshot"]!=*snapshot || self.sdk_schema_saved_snapshot(&request,&domain(snapshot)?)?.as_ref()!=Some(snapshot){return Err(HELD.into());}
        Ok(())
    }
    /// Called only after the SDK worker has closed. It does not buy another
    /// request; workflow.resume later applies the one-use corrected successor.
    pub(super) fn sdk_record_schema_refusal(&self,job:&str,request:&TextRequest,snapshot:&Value)->Result<()> {
        let current=self.sdk_job(job)?;
        let evidence=self.sdk_schema_origin_evidence(&current,request)?;
        self.store.event(job,"sdk_schema_refused",&evidence)?;
        self.store.update_job(job,"rejected",&json!({"error":REFUSED,"receipt":{"contract":"desktop-runner/v1","validation":"rejected","transportValidation":"schema_rejected","evidenceSha256":hash(&evidence)}}))?;
        if let Some(run)=&self.workflow_id {self.store.checkpoint(run,&step_key(request),&request.stage,&hash(snapshot),"rejected",Some(job),None,Some(REFUSED))?;}
        Ok(())
    }
    pub(super) async fn sdk_schema_retry_snapshot(&self,request:&TextRequest,snapshot:&Value)->Result<Option<Value>> {
        if snapshot.get(RETRY).is_some() || snapshot.get(CONTINUE).is_some(){return Ok(None);}
        let Some(run)=self.workflow_id.as_deref() else{return Ok(None);};
        let Some(original)=self.sdk_find(snapshot)? else{return Ok(None);};
        if !["submission_unknown","rejected"].contains(&s(&original,"status")) || !original["result"].is_null(){return Ok(None);}
        let file=crate::text_sdk::worker_root(&self.store.root,request)?.join(s(&original,"id")).join("receipt.json");
        let metadata=std::fs::symlink_metadata(&file).map_err(|_|HELD)?;
        if !metadata.is_file()||metadata.is_symlink()||metadata.len()>4_194_304{return Err(HELD.into());}
        let record:Value=serde_json::from_slice(&std::fs::read(file).map_err(|_|HELD)?).map_err(|_|HELD)?;
        if !["SDK_TURN_FAILED","SDK_SCHEMA_REJECTED"].contains(&s(&record["error"],"code"))
            || !s(&record["error"],"message").contains("invalid_json_schema") {return Ok(None);}
        let evidence=self.sdk_schema_origin_evidence(&original,request)?;
        if evidence["retryableUniqueItems"]!=true{return Err(REFUSED.into());}
        let verified=crate::text_sdk::prepare_schema_rejection_handoff(&self.settings).await?;
        self.sdk_parent_binding()?;self.sdk_checkpoint_binding(request,snapshot)?;
        let mut next=snapshot.clone();next["executorIdentity"]=verified.identity;
        let proof=sealed(json!({"version":POLICY,"workflowId":run,"nodeKey":step_key(request),
            "domainRequestSha256":hash(&domain(snapshot)?),"originJobId":original["id"],"originSnapshotSha256":hash(snapshot),
            "parentSnapshotSha256":parent(&self.store.conn()?,run,&self.settings)?,"originalSettingsSha256":hash(&self.settings),
            "effectiveIdentitySha256":hash(&next["executorIdentity"]),"resourceHashes":verified.resource_hashes,
            "evidence":evidence,"attempt":1,"maximumAttempts":1}));
        next[RETRY]=proof;
        self.sdk_reserve_schema_successor(request,&original,&next)?;
        Ok(Some(next))
    }
    fn sdk_reserve_schema_successor(&self,request:&TextRequest,original:&Value,next:&Value)->Result<String> {
        self.sdk_reserve_schema_successor_checked(request,original,next,||
            crate::text_sdk::verify_schema_rejection_bytes(&original["snapshot"]["executorIdentity"],&next["executorIdentity"],&next[RETRY]["resourceHashes"]))
    }
    fn sdk_reserve_schema_successor_checked(&self,request:&TextRequest,original:&Value,next:&Value,verify_runtime:impl FnOnce()->Result<()>)->Result<String> {
        let proof=&next[RETRY];let run=self.workflow_id.as_deref().ok_or(HELD)?;let key=step_key(request);let origin=s(original,"id");
        if !proof_basic(proof,next,&self.settings,run,&key,POLICY) || proof["attempt"]!=1 || proof["maximumAttempts"]!=1
            || proof["originJobId"]!=origin || proof["originSnapshotSha256"]!=hash(&original["snapshot"]) {return Err(HELD.into());}
        let mut c=self.store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE j.id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND j.state=?2 AND j.snapshot=?3 AND j.prepared=?4 AND j.result IS NULL AND w.workflow_id=?5 AND w.node_key=?6 AND w.request_hash=?7 AND w.state IN ('submission_unknown','rejected') AND w.result IS NULL) AND NOT EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND provider='codex_sdk' AND id<>?1 AND state NOT IN ('completed','rejected') AND json_extract(prepared,'$.sdkDispatchStarted')=1) AND NOT EXISTS(SELECT 1 FROM jobs WHERE json_extract(snapshot,'$.sdkSchemaRejectionRetry.originJobId')=?1) AND NOT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type=?8)",params![origin,s(original,"status"),original["snapshot"].to_string(),original["prepared"].to_string(),run,key,hash(&original["snapshot"]),EVENT],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !exact || parent(&tx,run,&self.settings)?!=s(proof,"parentSnapshotSha256")
            || self.sdk_schema_origin_evidence(original,request)?!=proof["evidence"] {return Err(HELD.into());}
        verify_runtime()?;
        let job=id();let timestamp=now();let request_hash=hash(next);
        let binding=self.sdk_pre_dispatch_binding(request,next,&tx)?;
        tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at) VALUES(?1,'text',?2,'codex_sdk','queued',?3,?4,?5,?5)",params![job,format!("{}:{}:{}",request.stage,request.skill,id()),s(&next["executorIdentity"],"accountFingerprint"),next.to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE jobs SET state='rejected',error=?2,receipt=?3,updated_at=?4 WHERE id=?1",params![origin,REFUSED,json!({"contract":"desktop-runner/v1","validation":"rejected","transportValidation":"schema_rejected","evidenceSha256":hash(&proof["evidence"]),"successorProofSha256":proof["sha256"],"successorJobId":job}).to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE workflow_steps SET request_hash=?3,state='queued',text_job_id=?4,result=NULL,error=NULL,updated_at=?5 WHERE workflow_id=?1 AND node_key=?2",params![run,key,request_hash,job,timestamp]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![origin,EVENT,json!({"proof":proof,"successorJobId":job,"originalStatus":original["status"],"originalError":original["error"],"originalReceipt":original["receipt"],"oldCheckpointHash":hash(&original["snapshot"])}).to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'sdk_pre_dispatch_queued',?2,?3)",params![job,binding.to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[timestamp]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())?;Ok(job)
    }
    /// A domain repair is a new, bounded logical node, never a retry of the
    /// rejected transport. Bind it to the exact received candidate and retain
    /// the original source packet, schema, model and complete creative rules.
    fn sdk_schema_domain_repair_bound(&self,request:&TextRequest,previous:&Value)->Result<()> {
        let repair=&request.input["validationRepair"];
        let attempt=repair["attempt"].as_u64().ok_or(HELD)?;
        let previous_request:TextRequest=serde_json::from_value(previous["snapshot"].clone()).map_err(|_|HELD)?;
        if request.stage!="seriesRepair" || request.skill!="series-repair" || repair["version"]!=DOMAIN_REPAIR
            || !(1..=2).contains(&attempt) || repair["maximumAttempts"]!=2
            || previous["status"]!="rejected" || s(previous,"error").trim().is_empty()
            || previous["receipt"]!=json!({"contract":"desktop-runner/v1","validation":"rejected"})
            || repair["problem"]!=previous["error"] || repair["candidate"]!=previous["result"]
            || previous["result"].is_null() || repair["previousRawSha256"]!=hash(&previous["result"])
            || repair["previousRequestSha256"]!=request_hash(&previous_request)? {return Err(HELD.into());}
        let origin=if attempt==1 {
            if previous_request.input.get("validationRepair").is_some(){return Err(HELD.into());}
            previous_request.clone()
        } else {
            let prior=&previous_request.input["validationRepair"];
            if previous_request.stage!="seriesRepair" || prior["version"]!=DOMAIN_REPAIR
                || prior["attempt"]!=1 || prior["maximumAttempts"]!=2 || prior["originRequestSha256"]!=repair["originRequestSha256"]
                || prior["problem"]==repair["problem"]
                || previous["snapshot"].get(CONTINUE).is_none() {return Err(HELD.into());}
            let origin_job=self.sdk_job(s(&previous["snapshot"][CONTINUE]["anchor"],"jobId"))?;
            serde_json::from_value::<TextRequest>(origin_job["snapshot"].clone()).map_err(|_|HELD)?
        };
        let role=s(&origin.input,"role");
        let role_valid=matches!((origin.stage.as_str(),role),("seriesAuthor","author")|("seriesFinisher","finisher")|("seriesRepair","repair"));
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let frozen=self.store.job(run)?;
        let fantasy=s(&origin.input,"fantasyMode");
        if !role_valid || origin.skill!=format!("series-{role}") || origin.input.get("validationRepair").is_some() || s(&origin.input,"executionRoleId").is_empty()
            || origin.input["workflowProfile"]!=crate::asset_graph::PROFILE || origin.input["freshContext"]!=true
            || frozen["snapshot"]["input"]["workflowProfile"]!=crate::asset_graph::PROFILE
            || origin.input["seriesRuleMaterialization"]!=frozen["snapshot"]["input"]["seriesRuleMaterialization"]
            || frozen["entityId"]!="storyboard.run" || frozen["snapshot"]["runnerVersion"]!="desktop-runner/v1"
            || s(&origin.input,"sourceSha256").is_empty() || origin.input["sourceSha256"]!=frozen["snapshot"]["input"]["source"]["sha256"]
            || !origin.input["packet"].is_object() || origin.input["packet"]["sourceSha256"]!=origin.input["sourceSha256"]
            || !["off","all","recommend"].contains(&fantasy)
            || repair["originRequestSha256"]!=request_hash(&origin)?
            || request.input["executionRoleId"]!=format!("{}/validation-repair/{attempt}",s(&origin.input,"executionRoleId"))
            || s(&request.input,"task").trim().is_empty()
            || request.model!=origin.model || request.reasoning!=origin.reasoning || request.schema!=origin.schema {return Err(HELD.into());}
        let mut expected_input=origin.input.clone();
        for field in ["executionRoleId","task","validationRepair"] {expected_input[field]=request.input[field].clone();}
        let expected_rules=crate::pipeline::series::repair_rules_for(&origin.input,role,fantasy!="off")?;
        let restored=crate::pipeline::series::restore_repair_rule_snapshot(&expected_rules)?;
        let origin_rules=if role=="repair" {&expected_rules}else{&restored["originRules"]};
        if request.input!=expected_input || request.rule_snapshot!=expected_rules || origin.rule_snapshot!=*origin_rules {return Err(HELD.into());}
        Ok(())
    }
    fn sdk_schema_anchor_for(&self,job:&str,request:Option<&TextRequest>)->Result<Value> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        #[cfg(test)]
        let _profile = validation_profile::enter("sdk_schema_anchor_for", job);
        let anchor=self.sdk_job(job)?;
        let supplemental=request.is_some_and(|request|request.input.get("seriesAssetSupplement").is_some() && request.input.get("validationRepair").is_none());
        let correction=request.is_some_and(|request|request.input.get("seriesSupplyCorrection").is_some() && request.input.get("validationRepair").is_none());
        let carried_repair=anchor["snapshot"].get(CONTINUE).is_some()
            && anchor["snapshot"]["stage"]=="seriesRepair" && anchor["snapshot"]["input"]["validationRepair"]["version"]==DOMAIN_REPAIR;
        let carried_supplement=anchor["snapshot"].get(CONTINUE).is_some()
            && anchor["snapshot"]["stage"]=="seriesFinisher" && series_supplement::supported_contract(&anchor["snapshot"]["input"]["seriesAssetSupplement"]["version"])
            && anchor["snapshot"]["input"]["seriesAssetSupplement"]["phase"]=="finisher";
        let carried_correction=anchor["snapshot"].get(CONTINUE).is_some()
            && anchor["snapshot"]["stage"]=="seriesFinisher" && anchor["snapshot"]["input"]["seriesSupplyCorrection"]["version"]==supply_correction::CONTRACT
            && anchor["snapshot"]["input"]["seriesSupplyCorrection"]["phase"]=="finisher";
        if supplemental || correction {
            // This chain has its own exact predecessor contract. A completed
            // design or review is not a general-purpose runtime anchor.
            if anchor["snapshot"].get(RETRY).is_none() && anchor["snapshot"].get(CONTINUE).is_none(){return Err(HELD.into());}
        } else if anchor["status"]=="rejected" {
            if anchor["snapshot"].get(RETRY).is_none() && anchor["snapshot"].get(CONTINUE).is_none(){return Err(HELD.into());}
            self.sdk_schema_domain_repair_bound(request.ok_or(HELD)?,&anchor)?;
        } else if anchor["status"]!="completed" || (anchor["snapshot"].get(RETRY).is_none() && !carried_repair && !carried_supplement && !carried_correction) {return Err(HELD.into());}
        let raw=self.sdk_schema_owned_raw(&anchor)?;
        if supplemental {self.sdk_schema_supplement_bound(request.ok_or(HELD)?,&anchor,&raw)?;}
        if correction {self.sdk_schema_supply_correction_bound(request.ok_or(HELD)?,&anchor,&raw)?;}
        Ok(anchor_summary(&anchor))
    }
    fn sdk_schema_owned_raw(&self,anchor:&Value)->Result<Value> {
        if !["completed","rejected"].contains(&s(anchor,"status")){return Err(HELD.into());}
        let job=s(anchor,"id");
        let saved_request:TextRequest=serde_json::from_value(anchor["snapshot"].clone()).map_err(|_|HELD)?;
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let state=if anchor["status"]=="completed" {"accepted"}else{"rejected"};
        let checkpoint_result=if anchor["status"]=="completed" {&anchor["prepared"]["sdkDomainResult"]}else{&anchor["result"]};
        let owned:bool=self.store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2 AND text_job_id=?3 AND request_hash=?4 AND state=?5 AND result=?6 AND error IS ?7)",
            params![run,step_key(&saved_request),job,hash(&anchor["snapshot"]),state,checkpoint_result.to_string(),anchor["error"].as_str()],|r|r.get(0)).map_err(|e|e.to_string())?;
        let attempt=FrozenAttempt::load(&self.store.root.join("text-runs").join(job).join("sdk-attempt.json"))?;
        if !owned || attempt.workflow_id!=run || attempt.logical_node_id!=step_key(&saved_request) || anchor["prepared"]["sdkOriginWorkflowId"]!=run {return Err(HELD.into());}
        self.sdk_raw(anchor,&anchor["snapshot"])
    }
    #[cfg(test)]
    fn sdk_schema_anchor(&self,job:&str)->Result<Value> {self.sdk_schema_anchor_for(job,None)}
    fn sdk_schema_continuation_bound(&self,snapshot:&Value)->Result<()> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        let run=self.workflow_id.as_deref().ok_or(HELD)?;let proof=&snapshot[CONTINUE];
        let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
        if !proof_basic(proof,snapshot,&self.settings,run,&step_key(&request),CONTINUATION)
            || parent(&self.store.conn()?,run,&self.settings)?!=s(proof,"parentSnapshotSha256")
            // Compare digests before recursively checking a historical anchor;
            // forged self/cyclic references cannot recurse through fake hashes.
            || anchor_summary(&self.sdk_job(s(&proof["anchor"],"jobId"))?)!=proof["anchor"]
            || self.sdk_schema_anchor_for(s(&proof["anchor"],"jobId"),Some(&request))?!=proof["anchor"] {return Err(HELD.into());}
        if proof["version"]==ACCOUNT_CONTINUATION {
            self.sdk_current_account_authorization_bound(snapshot,&proof["accountAuthorization"],CONTINUATION)?;
        }
        Ok(())
    }
    pub(super) fn sdk_schema_current_account_allocation_bound(&self,snapshot:&Value)->Result<()> {
        if snapshot[CONTINUE]["version"]!=ACCOUNT_CONTINUATION {return Ok(());}
        let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
        self.check_sdk_request_hold(&request)?;
        self.sdk_schema_continuation_bound(snapshot)?;
        // Preparation and dispatch check installed bytes. This allocation guard
        // rebinds only the local immutable proof; it adds no third account probe.
        let previous=self.sdk_current_account_authorization_bound(snapshot,&snapshot[CONTINUE]["accountAuthorization"],CONTINUATION)?;
        if !crate::text_sdk::current_account_receipt_identity_bound(&previous,&snapshot["executorIdentity"],"schema") {return Err(HELD.into());}
        Ok(())
    }
    fn sdk_schema_continuation_job(&self,request:&TextRequest)->Result<Option<String>> {
        let Some(run)=self.workflow_id.as_deref() else{return Ok(None);};
        let job=if request.input.get("validationRepair").is_some() {
            let c=self.store.conn()?;
            let mut query=c.prepare("SELECT j.id,j.snapshot FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE w.workflow_id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND j.state='rejected' AND (json_extract(j.snapshot,'$.sdkSchemaRejectionRetry.workflowId')=?1 OR json_extract(j.snapshot,'$.sdkSchemaContinuation.workflowId')=?1)").map_err(|e|e.to_string())?;
            let rows=query.query_map([run],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?;
            let mut found=None;
            for row in rows {let(id,raw)=row.map_err(|e|e.to_string())?;let previous:TextRequest=serde_json::from_str(&raw).map_err(|_|HELD)?;
                if request.input["validationRepair"]["previousRequestSha256"]==request_hash(&previous)? {if found.replace(id).is_some(){return Err(HELD.into());}}
            }
            if found.is_none() {
                let known:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE json_extract(snapshot,'$.sdkSchemaRejectionRetry.workflowId')=?1 OR json_extract(snapshot,'$.sdkSchemaContinuation.workflowId')=?1)",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
                if known{return Err(HELD.into());}
            }
            found
        }else if request.input.get("seriesSupplyCorrection").is_some() {
            self.sdk_schema_supply_correction_predecessor(request)?
        }else if request.input.get("seriesAssetSupplement").is_some() {
            self.sdk_schema_supplement_predecessor(request)?
        }else{
            self.store.conn()?.query_row("SELECT id FROM jobs WHERE kind='text' AND provider='codex_sdk' AND state='completed' AND (json_extract(snapshot,'$.sdkSchemaRejectionRetry.workflowId')=?1 OR (json_extract(snapshot,'$.sdkSchemaContinuation.workflowId')=?1 AND (json_extract(snapshot,'$.input.validationRepair.version')=?2 OR (json_extract(snapshot,'$.input.seriesAssetSupplement.version') IN (?3,?4,?5) AND json_extract(snapshot,'$.input.seriesAssetSupplement.phase')='finisher' AND json_extract(snapshot,'$.stage')='seriesFinisher') OR (json_extract(snapshot,'$.input.seriesSupplyCorrection.version')=?6 AND json_extract(snapshot,'$.input.seriesSupplyCorrection.phase')='finisher' AND json_extract(snapshot,'$.stage')='seriesFinisher')))) ORDER BY rowid DESC LIMIT 1",params![run,DOMAIN_REPAIR,series_supplement::CONTRACT,series_supplement::CONTRACT_V2,series_supplement::CONTRACT_V3,supply_correction::CONTRACT],|r|r.get(0)).optional().map_err(|e|e.to_string())?
        };
        Ok(job)
    }
    pub(super) async fn sdk_schema_continuation_snapshot(&self,request:&TextRequest,original:&Value)->Result<Option<Value>> {
        let Some(run)=self.workflow_id.as_deref() else{return Ok(None);};
        let Some(job)=self.sdk_schema_continuation_job(request)? else{return Ok(None);};
        let occupied:bool=self.store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2)",params![run,step_key(request)],|r|r.get(0)).map_err(|e|e.to_string())?;
        if occupied{return Err(HELD.into());}
        if domain(original)?!=self.request_domain_snapshot(request)? {return Err(HELD.into());}
        let anchor=self.sdk_schema_anchor_for(&job,Some(request))?;
        self.check_sdk_request_hold(request)?;
        // After verified completion (or exact bounded domain rejection), the
        // successor's frozen package is the runtime reference. Do not require
        // the mutable pre-upgrade source
        // resource directory to remain installed forever.
        let reference=self.sdk_current_account_reference()?.ok_or(HELD)?;
        let mut runtime_settings=self.settings.clone();
        runtime_settings["textSdkIdentity"]=reference.0.clone();
        let verified=crate::text_sdk::prepare_current_account_handoff(&runtime_settings,true).await?;
        let authorization=self.sdk_current_account_authorization(request,&verified.identity,Some(&reference),CONTINUATION);
        let mut snapshot=original.clone();snapshot["executorIdentity"]=verified.identity;
        snapshot[CONTINUE]=sealed(json!({"version":CONTINUATION,"workflowId":run,"nodeKey":step_key(request),
            "domainRequestSha256":hash(&domain(original)?),"originalSettingsSha256":hash(&self.settings),
            "parentSnapshotSha256":parent(&self.store.conn()?,run,&self.settings)?,
            "effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":verified.resource_hashes,"anchor":anchor}));
        if let Some(authorization)=authorization {
            let mut proof=snapshot[CONTINUE].clone();proof.as_object_mut().unwrap().remove("sha256");
            proof["version"]=json!(ACCOUNT_CONTINUATION);proof["accountAuthorization"]=authorization;
            snapshot[CONTINUE]=sealed(proof);
        }
        self.sdk_schema_continuation_bound(&snapshot)?;Ok(Some(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("text_executor_sdk_account_schema_tests.rs");
    fn fixture()->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,Value) {
        fixture_request(TextRequest{stage:"seriesAuthor".into(),skill:"synthetic".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input:json!({"sourceSha256":"synthetic"}),schema:json!({"type":"object","properties":{"assets":{"type":"array","items":{"type":"string"},"uniqueItems":true}}}),rule_snapshot:json!({})})
    }
    fn fixture_request(request:TextRequest)->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,Value) {
        fixture_request_input(request,None)
    }
    pub(super) fn fixture_request_input(request:TextRequest,input:Option<Value>)->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,Value) {
        fixture_request_input_with_workflow(request,input,None)
    }
    pub(super) fn fixture_request_input_with_workflow(request:TextRequest,input:Option<Value>,workflow:Option<&str>)->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,Value) {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("schema refusal synthetic","schema-refusal").unwrap();store.ensure_workflow_storage().unwrap();
        let identity=crate::text_sdk::test_refusal_identity(temp.path());
        let mut settings=input.as_ref().and_then(|value|value.get("settings")).cloned().unwrap_or_else(||json!({}));
        for(key,value)in json!({"textProvider":"codex_sdk","textModel":"gpt-6.1-sol","reasoning":"medium","textSdkIdentity":identity}).as_object().unwrap(){settings[key]=value.clone();}
        let parent_input=if let Some(mut input)=input {input["settings"]=settings.clone();json!({"runnerVersion":"desktop-runner/v1","input":input})}
            else if request.input["workflowProfile"]==crate::asset_graph::PROFILE {json!({"runnerVersion":"desktop-runner/v1","input":{"settings":settings,"workflowProfile":crate::asset_graph::PROFILE,"source":{"sha256":request.input["sourceSha256"]}}})}else{json!({"settings":settings})};
        let mut run=store.add_job("workflow","storyboard.run","local","local",&parent_input,"running").unwrap();
        if let Some(expected)=workflow {
            // Synthetic fixture only, before any child/files/checkpoints exist.
            // This avoids rebuilding a full domain chain just to learn a UUID.
            store.conn().unwrap().execute("UPDATE jobs SET id=?1 WHERE id=?2",params![expected,run]).unwrap();run=expected.to_owned();
        }
        let executor=SavedTextExecutor{store,settings,workflow_id:Some(run.clone())};let mut snapshot=executor.request_domain_snapshot(&request).unwrap();snapshot["executorIdentity"]=identity;
        let job=executor.store.add_job("text","synthetic-author","codex_sdk","synthetic-account",&snapshot,"submission_unknown").unwrap();
        let prepared=FrozenAttempt::prepare_sdk_rules_first(&job,&run,&step_key(&request),request.clone(),&snapshot["executorIdentity"]).unwrap();
        let(_,rows)=crate::text_sdk::test_refusal_files(&executor.store.root,&prepared);
        let compiled=compile_sdk_attempt_input(&prepared).unwrap();
        executor.store.update_job(&job,"submission_unknown",&json!({"prepared":{"sdkDispatchStarted":true,"sdkAttemptId":job,"sdkOriginWorkflowId":run,"sdkRequestHash":hash(&snapshot),"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema)}})).unwrap();
        for row in rows.as_array().unwrap(){executor.store.event(&job,s(row,"type"),&row["data"]).unwrap();}
        executor.store.checkpoint(&run,&step_key(&request),&request.stage,&hash(&snapshot),"submission_unknown",Some(&job),None,Some("SDK_SUBMISSION_UNKNOWN")).unwrap();
        let original=executor.sdk_job(&job).unwrap();let evidence=executor.sdk_schema_origin_evidence(&original,&request).unwrap();
        let mut next=snapshot.clone();next["executorIdentity"]["supervisorExecutable"]=json!(temp.path().join("successor.exe"));next["executorIdentity"]["supervisorSha256"]=json!("e".repeat(64));
        next[RETRY]=sealed(json!({"version":POLICY,"workflowId":run,"nodeKey":step_key(&request),"domainRequestSha256":hash(&domain(&snapshot).unwrap()),
            "originJobId":job,"originSnapshotSha256":hash(&snapshot),"parentSnapshotSha256":parent(&executor.store.conn().unwrap(),&run,&executor.settings).unwrap(),
            "originalSettingsSha256":hash(&executor.settings),"effectiveIdentitySha256":hash(&next["executorIdentity"]),"resourceHashes":{},"evidence":evidence,"attempt":1,"maximumAttempts":1}));
        (temp,executor,request,original,next)
    }
    #[test]
    fn sdk_schema_successor_one_use_preserves_original_files_and_reopens_same_queued_job() {
        let(_temp,executor,request,original,next)=fixture();let origin=s(&original,"id");
        let file=executor.store.root.join("sdk-worker").join(origin).join("receipt.json");let before=std::fs::read(&file).unwrap();
        let job=executor.sdk_reserve_schema_successor_checked(&request,&original,&next,||Ok(())).unwrap();
        assert_ne!(job,origin);let old=executor.sdk_job(origin).unwrap();assert_eq!(old["status"],"rejected");assert_eq!(old["snapshot"],original["snapshot"]);assert_eq!(old["prepared"],original["prepared"]);assert!(old["result"].is_null());
        assert_eq!(executor.sdk_job(&job).unwrap()["status"],"queued");assert_eq!(std::fs::read(&file).unwrap(),before);
        assert!(executor.sdk_reserve_schema_successor_checked(&request,&original,&next,||Ok(())).is_err());
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.sdk_schema_saved_snapshot(&request,&domain(&next).unwrap()).unwrap(),Some(next.clone()));
        assert_eq!(reopened.request_snapshot(&request).unwrap(),next);
        let current=reopened.sdk_job(&job).unwrap();assert!(reopened.sdk_pre_dispatch_evidence(&request,&next,&job,&reopened.store.conn().unwrap()).is_ok());
        assert!(current["prepared"].is_null());assert_eq!(reopened.store.jobs().unwrap().len(),3);
    }
    #[test]
    fn sdk_schema_successor_race_allows_one_atomic_reservation() {
        let(_temp,executor,request,original,next)=fixture();let barrier=Arc::new(std::sync::Barrier::new(2));let mut threads=Vec::new();
        for _ in 0..2 {let barrier=barrier.clone();let e=SavedTextExecutor{store:executor.store.clone(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};let(r,o,n)=(request.clone(),original.clone(),next.clone());threads.push(std::thread::spawn(move||{barrier.wait();e.sdk_reserve_schema_successor_checked(&r,&o,&n,||Ok(())).is_ok()}));}
        assert_eq!(threads.into_iter().map(|thread|usize::from(thread.join().unwrap())).sum::<usize>(),1);assert_eq!(executor.store.jobs().unwrap().len(),3);
    }
    #[test]
    fn sdk_schema_successor_refuses_parent_raw_other_owner_stop_and_runtime_drift_without_writes() {
        for fault in ["parent","raw","other","stop","runtime","budget","orphan"] {
            let(_temp,executor,request,original,mut next)=fixture();let origin=s(&original,"id");
            match fault {
                "parent"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET snapshot='{}' WHERE id=?1",[executor.workflow_id.as_deref().unwrap()]).unwrap();},
                "raw"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET result='{}' WHERE id=?1",[origin]).unwrap();},
                "other"=>{let id=executor.store.add_job("text","other","codex_sdk","synthetic",&json!({}),"submission_unknown").unwrap();executor.store.update_job(&id,"submission_unknown",&json!({"prepared":{"sdkDispatchStarted":true}})).unwrap();},
                "stop"=>{executor.store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_process_tree'",[origin]).unwrap();},
                "budget"=>{next[RETRY]["attempt"]=json!(2);let mut proof=next[RETRY].clone();proof.as_object_mut().unwrap().remove("sha256");next[RETRY]=sealed(proof);},
                "orphan"=>{executor.store.add_job("text","orphan","codex_sdk","synthetic",&next,"queued").unwrap();},_=>(),
            }
            let before=executor.store.jobs().unwrap();let rows=events(&executor.store.conn().unwrap(),origin).unwrap();
            assert!(executor.sdk_reserve_schema_successor_checked(&request,&original,&next,||if fault=="runtime"{Err("changed runtime".into())}else{Ok(())}).is_err(),"{fault}");
            assert_eq!(executor.store.jobs().unwrap(),before,"{fault}");assert_eq!(events(&executor.store.conn().unwrap(),origin).unwrap(),rows,"{fault}");
        }
    }
    #[test]
    fn sdk_schema_successor_tampering_or_missing_checkpoint_cannot_forge_recovery() {
        for fault in ["proof","event","receipt","checkpoint"] {
            let(_temp,executor,request,original,next)=fixture();let job=executor.sdk_reserve_schema_successor_checked(&request,&original,&next,||Ok(())).unwrap();
            match fault {
                "proof"=>{let mut changed=next.clone();changed[RETRY]["evidence"]["processTreeStopProven"]=json!(false);executor.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![job,changed.to_string()]).unwrap();},
                "event"=>{executor.store.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1 AND type=?2",params![s(&original,"id"),EVENT]).unwrap();},
                "receipt"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET receipt='{}' WHERE id=?1",[s(&original,"id")]).unwrap();},
                "checkpoint"=>{executor.store.conn().unwrap().execute("DELETE FROM workflow_steps WHERE text_job_id=?1",[job]).unwrap();},_=>(),
            }
            assert!(executor.request_snapshot(&request).is_err(),"{fault}");
        }
    }
    fn adopt_synthetic_successor(executor:&SavedTextExecutor,request:&TextRequest,snapshot:&Value,job:&str) {
        adopt_raw(executor,request,snapshot,job,&json!({"assets":["a"]}));
    }
    pub(super) fn adopt_raw(executor:&SavedTextExecutor,request:&TextRequest,snapshot:&Value,job:&str,raw:&Value) {
        use crate::text_transport::{Backend,Observation,ObservationKind};
        let mut attempt=FrozenAttempt::prepare_sdk_strict_schema(job,executor.workflow_id.as_deref().unwrap(),&step_key(request),request.clone(),&snapshot["executorIdentity"]).unwrap();
        let compiled=compile_sdk_attempt_input(&attempt).unwrap();let dir=executor.store.root.join("text-runs").join(job);std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sdk-compiled.json"),json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)}).to_string()).unwrap();
        let envelope=json!({"rawText":raw.to_string(),"parsedJson":raw,"parseError":null,"domainAccepted":false});
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.observe(Observation{attempt_id:job.into(),sequence:1,kind:ObservationKind::Completed,thread_id:Some("synthetic-new-thread".into()),turn_id:None,raw_result:Some(envelope)}).unwrap();attempt.persist(&dir.join("sdk-attempt.json")).unwrap();
        std::fs::write(dir.join("result.json"),raw.to_string()).unwrap();executor.store.event(job,"text_received_raw",&json!({"requestHash":hash(snapshot),"value":raw})).unwrap();
        executor.store.update_job(job,"completed",&json!({"prepared":{"sdkRequestHash":hash(snapshot),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":executor.workflow_id,
            "compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),"sdkRawTextSha256":hash(&raw),"sdkDomainResult":raw,"sdkDomainResultSha256":hash(&raw)},
            "result":raw,"error":null,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
        executor.store.checkpoint(executor.workflow_id.as_deref().unwrap(),&step_key(request),&request.stage,&hash(snapshot),"accepted",Some(job),Some(&raw),None).unwrap();
    }
    #[test]
    fn sdk_schema_adopted_successor_binds_later_nodes_without_editing_parent_or_old_request() {
        let(_temp,executor,request,original,next)=fixture();let parent_before=executor.store.job(executor.workflow_id.as_deref().unwrap()).unwrap()["snapshot"].clone();
        let job=executor.sdk_reserve_schema_successor_checked(&request,&original,&next,||Ok(())).unwrap();
        assert!(executor.sdk_schema_anchor(&job).is_err(),"queued is not adopted");
        adopt_synthetic_successor(&executor,&request,&next,&job);let anchor=executor.sdk_schema_anchor(&job).unwrap();
        for stage in ["seriesFinish","seriesReview"] {
            let mut later=request.clone();later.stage=stage.into();let mut snapshot=executor.request_domain_snapshot(&later).unwrap();snapshot["executorIdentity"]=next["executorIdentity"].clone();
            let run=executor.workflow_id.as_deref().unwrap();
            snapshot[CONTINUE]=sealed(json!({"version":CONTINUATION,"workflowId":run,"nodeKey":step_key(&later),"domainRequestSha256":hash(&domain(&snapshot).unwrap()),
                "originalSettingsSha256":hash(&executor.settings),"parentSnapshotSha256":parent(&executor.store.conn().unwrap(),run,&executor.settings).unwrap(),
                "effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":{},"anchor":anchor}));
            executor.sdk_schema_continuation_bound(&snapshot).unwrap();let id=executor.sdk_allocate_pre_dispatch(&later,&snapshot).unwrap();
            assert_eq!(executor.request_snapshot(&later).unwrap(),snapshot);assert!(executor.sdk_pre_dispatch_evidence(&later,&snapshot,&id,&executor.store.conn().unwrap()).is_ok());
            let mut changed=snapshot.clone();changed[CONTINUE]["anchor"]["jobId"]=json!("unknown");assert!(executor.sdk_schema_continuation_bound(&changed).is_err());
        }
        assert_eq!(executor.store.job(executor.workflow_id.as_deref().unwrap()).unwrap()["snapshot"],parent_before);
        assert_eq!(executor.sdk_job(s(&original,"id")).unwrap()["snapshot"],original["snapshot"]);
        assert!(executor.sdk_raw(&executor.sdk_job(&job).unwrap(),&next).is_ok());
    }
    fn domain_fixture()->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,Value,String) {
        domain_fixture_for(false)
    }
    fn domain_fixture_for(pooled:bool)->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,Value,String) {
        let rules=crate::pipeline::series::repair_rules("author",false).unwrap();
        let mut request=TextRequest{stage:"seriesAuthor".into(),skill:"series-author".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),
            input:json!({"workflowProfile":crate::asset_graph::PROFILE,"sourceSha256":"synthetic-source","episodeId":"e1","executionRoleId":"e1/author/0","role":"author","task":"complete author","freshContext":true,"fantasyMode":"off","packet":{"sourceSha256":"synthetic-source","sourceRows":["原文不变"],"assets":[{"id":"a","full":"完整设计"}]}}),
            schema:json!({"type":"object","properties":{"assets":{"type":"array","items":{"type":"string"},"uniqueItems":true}}}),rule_snapshot:rules["originRules"].clone()};
        // Mirror the actual frozen rule identity rather than relying on the
        // installed default when a rejected request later enters domain repair.
        let mut parent_input=json!({"workflowProfile":crate::asset_graph::PROFILE,"source":{"sha256":"synthetic-source"}});
        for (field,pack_field) in [("seriesSkillVersion","skillVersion"),("seriesRulePackSha256","packSha256")] {
            request.input[field]=request.rule_snapshot["creativePack"][pack_field].clone();
            parent_input[field]=request.input[field].clone();
        }
        assert_eq!(crate::pipeline::series::rule_pack_hash(&request.input).unwrap(),s(&request.rule_snapshot["creativePack"],"packSha256"));
        if pooled {
            request.input["seriesRuleMaterialization"]=json!("series-rule-content-once/v1");
            parent_input["seriesRuleMaterialization"]=request.input["seriesRuleMaterialization"].clone();
        }
        let(temp,executor,request,original,next)=fixture_request_input(request,Some(parent_input));
        let job=executor.sdk_reserve_schema_successor_checked(&request,&original,&next,||Ok(())).unwrap();
        adopt_synthetic_successor(&executor,&request,&next,&job);
        reject_synthetic(&executor,&request,&next,&job,"SERIES_DIALOGUE_INTERVAL_OUTSIDE_SOURCE");
        (temp,executor,request,original,next,job)
    }
    pub(super) fn reject_synthetic(executor:&SavedTextExecutor,request:&TextRequest,snapshot:&Value,job:&str,reason:&str) {
        let raw=executor.sdk_job(job).unwrap()["result"].clone();
        executor.store.update_job(job,"rejected",&json!({"error":reason,"receipt":{"contract":"desktop-runner/v1","validation":"rejected"}})).unwrap();
        executor.store.checkpoint(executor.workflow_id.as_deref().unwrap(),&step_key(request),&request.stage,&hash(snapshot),"rejected",Some(job),Some(&raw),Some(reason)).unwrap();
    }
    pub(super) fn domain_repair(origin:&TextRequest,previous:&TextRequest,job:&Value,attempt:u64)->TextRequest {
        let mut request=origin.clone();request.stage="seriesRepair".into();request.skill="series-repair".into();
        request.input["executionRoleId"]=json!(format!("{}/validation-repair/{attempt}",s(&origin.input,"executionRoleId")));
        request.input["task"]=json!("按领域问题修正完整当前稿；原文、设计和规则不变。");
        request.input["validationRepair"]=json!({"version":DOMAIN_REPAIR,"attempt":attempt,"maximumAttempts":2,"originRequestSha256":request_hash(origin).unwrap(),"previousRequestSha256":request_hash(previous).unwrap(),"previousRawSha256":hash(&job["result"]),"problem":job["error"],"candidate":job["result"]});
        request.rule_snapshot=crate::pipeline::series::repair_rules_for(&origin.input,s(&origin.input,"role"),false).unwrap();request
    }
    pub(super) fn continuation_snapshot(executor:&SavedTextExecutor,request:&TextRequest,identity:&Value,anchor:&Value)->Value {
        let mut snapshot=executor.request_domain_snapshot(request).unwrap();snapshot["executorIdentity"]=identity.clone();
        let run=executor.workflow_id.as_deref().unwrap();
        snapshot[CONTINUE]=sealed(json!({"version":CONTINUATION,"workflowId":run,"nodeKey":step_key(request),"domainRequestSha256":hash(&domain(&snapshot).unwrap()),
            "originalSettingsSha256":hash(&executor.settings),"parentSnapshotSha256":parent(&executor.store.conn().unwrap(),run,&executor.settings).unwrap(),
            "effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":{},"anchor":anchor}));snapshot
    }
    #[test]
    fn sdk_schema_domain_repair_binds_exact_rejected_raw_and_preserves_original_rejection() {
        let(_temp,executor,origin,original,next,job)=domain_fixture();
        let rejected=executor.sdk_job(&job).unwrap();let rows=events(&executor.store.conn().unwrap(),&job).unwrap();
        let receipt_file=executor.store.root.join("sdk-worker").join(s(&original,"id")).join("receipt.json");let bytes=std::fs::read(&receipt_file).unwrap();
        let repair=domain_repair(&origin,&origin,&rejected,1);
        assert_eq!(executor.sdk_schema_continuation_job(&repair).unwrap(),Some(job.clone()));
        assert!(executor.sdk_schema_anchor(&job).is_err(),"rejected is not a general adopted anchor");
        assert!(executor.sdk_schema_anchor_for(&job,Some(&origin)).is_err(),"arbitrary new author must not spend repair budget");
        let anchor=executor.sdk_schema_anchor_for(&job,Some(&repair)).unwrap();
        let snapshot=continuation_snapshot(&executor,&repair,&next["executorIdentity"],&anchor);
        executor.sdk_schema_continuation_bound(&snapshot).unwrap();
        let new_job=executor.sdk_allocate_pre_dispatch(&repair,&snapshot).unwrap();assert_ne!(new_job,job);
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.request_snapshot(&repair).unwrap(),snapshot);
        assert_eq!(reopened.sdk_job(&job).unwrap(),rejected);assert_eq!(events(&reopened.store.conn().unwrap(),&job).unwrap(),rows);
        assert_eq!(std::fs::read(receipt_file).unwrap(),bytes);
        assert_eq!(reopened.sdk_job(&new_job).unwrap()["status"],"queued");
        assert!(reopened.sdk_pre_dispatch_evidence(&repair,&snapshot,&new_job,&reopened.store.conn().unwrap()).is_ok());
    }
    #[test]
    fn sdk_schema_pooled_rules_preserve_full_rules_scope_and_replay_without_upgrading_history() {
        let(_temp,executor,origin,_original,next,job)=domain_fixture_for(true);
        let rejected=executor.sdk_job(&job).unwrap();let first=domain_repair(&origin,&origin,&rejected,1);
        let legacy=crate::pipeline::series::repair_rules("author",false).unwrap();
        assert_ne!(first.rule_snapshot,legacy);
        assert_eq!(crate::pipeline::series::restore_repair_rule_snapshot(&first.rule_snapshot).unwrap(),legacy);
        let before=executor.store.jobs().unwrap();
        for fault in ["marker","plain","body"] {
            let mut bad=first.clone();match fault {
                "marker"=>{bad.input.as_object_mut().unwrap().remove("seriesRuleMaterialization");},
                "plain"=>bad.rule_snapshot=legacy.clone(),
                "body"=>bad.rule_snapshot["bodies"]=json!({}),_=>unreachable!(),
            }
            assert!(executor.sdk_schema_anchor_for(&job,Some(&bad)).is_err(),"{fault}");assert_eq!(executor.store.jobs().unwrap(),before);
        }
        let anchor=executor.sdk_schema_anchor_for(&job,Some(&first)).unwrap();let snapshot=continuation_snapshot(&executor,&first,&next["executorIdentity"],&anchor);
        executor.sdk_schema_continuation_bound(&snapshot).unwrap();let child=executor.sdk_allocate_pre_dispatch(&first,&snapshot).unwrap();
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.request_snapshot(&first).unwrap(),snapshot);assert_eq!(reopened.sdk_job(&job).unwrap(),rejected);
        adopt_synthetic_successor(&reopened,&first,&snapshot,&child);
        assert!(reopened.sdk_raw(&reopened.sdk_job(&child).unwrap(),&snapshot).is_ok());
        assert_eq!(first.input["validationRepair"]["maximumAttempts"],2);
        let run=reopened.workflow_id.as_deref().unwrap();let mut parent=reopened.store.job(run).unwrap()["snapshot"].clone();parent["input"]["seriesRuleMaterialization"]=Value::Null;
        reopened.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![run,parent.to_string()]).unwrap();
        assert!(reopened.sdk_schema_continuation_bound(&snapshot).is_err(),"frozen parent materialization cannot drift");
    }
    #[test]
    fn sdk_schema_two_domain_repairs_can_continue_to_finisher_without_promoting_rejected_nodes() {
        let(_temp,executor,origin,_original,next,job)=domain_fixture();let rejected=executor.sdk_job(&job).unwrap();
        let first=domain_repair(&origin,&origin,&rejected,1);
        let first_snapshot=continuation_snapshot(&executor,&first,&next["executorIdentity"],&executor.sdk_schema_anchor_for(&job,Some(&first)).unwrap());
        let first_job=executor.sdk_allocate_pre_dispatch(&first,&first_snapshot).unwrap();
        adopt_synthetic_successor(&executor,&first,&first_snapshot,&first_job);
        reject_synthetic(&executor,&first,&first_snapshot,&first_job,"SERIES_CREATIVE_DEPENDENCY_UNRESOLVED");
        let first_rejected=executor.sdk_job(&first_job).unwrap();
        let second=domain_repair(&origin,&first,&first_rejected,2);
        assert_eq!(executor.sdk_schema_continuation_job(&second).unwrap(),Some(first_job.clone()));
        let second_snapshot=continuation_snapshot(&executor,&second,&next["executorIdentity"],&executor.sdk_schema_anchor_for(&first_job,Some(&second)).unwrap());
        let second_job=executor.sdk_allocate_pre_dispatch(&second,&second_snapshot).unwrap();
        adopt_synthetic_successor(&executor,&second,&second_snapshot,&second_job);
        let mut finisher=origin.clone();finisher.stage="seriesFinisher".into();finisher.input["role"]=json!("finisher");finisher.input["executionRoleId"]=json!("e1/finisher/0");
        assert_eq!(executor.sdk_schema_continuation_job(&finisher).unwrap(),Some(second_job.clone()));
        let finisher_snapshot=continuation_snapshot(&executor,&finisher,&next["executorIdentity"],&executor.sdk_schema_anchor_for(&second_job,Some(&finisher)).unwrap());
        executor.sdk_schema_continuation_bound(&finisher_snapshot).unwrap();
        let finisher_job=executor.sdk_allocate_pre_dispatch(&finisher,&finisher_snapshot).unwrap();
        assert_eq!(executor.request_snapshot(&finisher).unwrap(),finisher_snapshot);
        assert_eq!(executor.sdk_job(&finisher_job).unwrap()["status"],"queued");
        assert_eq!(executor.sdk_job(&job).unwrap(),rejected);assert_eq!(executor.sdk_job(&first_job).unwrap(),first_rejected);
        assert!(executor.sdk_schema_anchor_for(&first_job,Some(&domain_repair(&origin,&first,&first_rejected,3))).is_err());
    }
    #[test]
    fn sdk_schema_domain_repair_request_drift_or_budget_bypass_creates_nothing() {
        let(_temp,executor,origin,_original,_next,job)=domain_fixture();let rejected=executor.sdk_job(&job).unwrap();
        let valid=domain_repair(&origin,&origin,&rejected,1);let before=executor.store.jobs().unwrap();
        for fault in ["raw-hash","request-hash","origin-hash","candidate","problem","zero","third","maximum","stage","skill","node","model","reasoning","schema","source","packet","rules","origin-rules","role","extra-input"] {
            let mut request=valid.clone();match fault {
                "raw-hash"=>request.input["validationRepair"]["previousRawSha256"]=json!("wrong"),
                "request-hash"=>request.input["validationRepair"]["previousRequestSha256"]=json!("wrong"),
                "origin-hash"=>request.input["validationRepair"]["originRequestSha256"]=json!("wrong"),
                "candidate"=>request.input["validationRepair"]["candidate"]=json!({"assets":["changed"]}),
                "problem"=>request.input["validationRepair"]["problem"]=json!("tls handshake eof"),
                "zero"=>request.input["validationRepair"]["attempt"]=json!(0),"third"=>request.input["validationRepair"]["attempt"]=json!(3),
                "maximum"=>request.input["validationRepair"]["maximumAttempts"]=json!(3),"stage"=>request.stage="seriesAuthor".into(),"skill"=>request.skill="other".into(),
                "node"=>request.input["executionRoleId"]=json!("other/validation-repair/1"),"model"=>request.model="different".into(),"reasoning"=>request.reasoning="high".into(),
                "schema"=>request.schema=json!({}),"source"=>request.input["sourceSha256"]=json!("other-source"),"packet"=>request.input["packet"]["sourceRows"]=json!(["替换原文"]),
                "rules"=>request.rule_snapshot=json!({}),"origin-rules"=>request.rule_snapshot["originRules"]=json!({}),"role"=>request.input["role"]=json!("reviewer"),
                "extra-input"=>request.input["extra"]=json!("new task"),_=>unreachable!(),
            }
            assert!(executor.sdk_schema_anchor_for(&job,Some(&request)).is_err(),"{fault}");
            assert_eq!(executor.store.jobs().unwrap(),before,"{fault}");
        }
    }
    #[test]
    fn sdk_schema_domain_repair_unknown_checkpoint_raw_and_owner_drift_fail_closed() {
        for fault in ["unknown","checkpoint","owner","compiled","raw","identity","parent-source"] {
            let(_temp,executor,origin,_original,_next,job)=domain_fixture();let repair=domain_repair(&origin,&origin,&executor.sdk_job(&job).unwrap(),1);
            let c=executor.store.conn().unwrap();match fault {
                "unknown"=>{c.execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",[&job]).unwrap();},
                "checkpoint"=>{c.execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&job]).unwrap();},
                "owner"=>{let path=executor.store.root.join("text-runs").join(&job).join("sdk-attempt.json");let mut record:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();record["payload"]["workflowId"]=json!("other-workflow");record["sha256"]=json!(hash(&record["payload"]));std::fs::write(&path,record.to_string()).unwrap();},
                "compiled"=>std::fs::write(executor.store.root.join("text-runs").join(&job).join("sdk-compiled.json"),"{}").unwrap(),
                "raw"=>std::fs::write(executor.store.root.join("text-runs").join(&job).join("result.json"),"{}").unwrap(),
                "identity"=>{let mut snapshot=executor.sdk_job(&job).unwrap()["snapshot"].clone();snapshot["executorIdentity"]["accountFingerprint"]=json!("f".repeat(64));c.execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![job,snapshot.to_string()]).unwrap();},
                "parent-source"=>{let run=executor.workflow_id.as_deref().unwrap();let mut snapshot=executor.store.job(run).unwrap()["snapshot"].clone();snapshot["input"]["source"]["sha256"]=json!("changed");c.execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![run,snapshot.to_string()]).unwrap();},_=>unreachable!(),
            }
            let before=executor.store.jobs().unwrap();assert!(executor.sdk_schema_anchor_for(&job,Some(&repair)).is_err(),"{fault}");assert_eq!(executor.store.jobs().unwrap(),before,"{fault}");
        }
    }
}
