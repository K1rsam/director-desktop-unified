//! One-use continuation of an original request that the pinned old bridge
//! conclusively rejected before constructing the SDK/runtime. No new attempt.
use super::*;
use crate::text_transport::{AttemptState, FrozenAttempt, ObservationKind, TextTransport};
const FIELD: &str = "sdkLocalPreparationRecovery";
const EVENT: &str = "sdk_local_preparation_recovery_reserved";
const REPAIR_FIELD: &str = "sdkWrapperPreparationRepair";
const REPAIR_EVENT: &str = "sdk_wrapper_preparation_repair_reserved";
const HELD: &str = "SDK_LOCAL_PREPARATION_RECOVERY_HELD: 恢复意图已占用或证据不完整；保留原请求、未知状态与预算，不重复提交";

fn events(c: &rusqlite::Connection, job: &str) -> Result<Value> {
    let mut q=c.prepare("SELECT id,type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let rows=q.query_map([job],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
    let mut out=Vec::new();
    for row in rows {let (id,kind,data)=row.map_err(|e|e.to_string())?;if out.len()>=2048{return Err(HELD.into());}out.push(json!({"id":id,"type":kind,"data":serde_json::from_str::<Value>(&data).map_err(|e|e.to_string())?}));}
    Ok(json!(out))
}
fn hash(v: &Value) -> String { crate::pipeline::digest(&v.to_string()) }

impl SavedTextExecutor {
    fn sdk_wrapper_repair_bound(&self, job:&Value)->Result<Option<Value>> {
        let Some(saved)=job["prepared"].get(REPAIR_FIELD) else{return Ok(None);};
        let proof=&saved["proof"];let evidence=&saved["evidence"];let id=s(job,"id");
        let rows:Vec<_>=events(&self.store.conn()?,id)?.as_array().ok_or(HELD)?.iter().filter(|e|e["type"]==REPAIR_EVENT).cloned().collect();
        if rows.len()!=1||rows[0]["data"]!=*saved||saved["originalSettingsSha256"]!=hash(&self.settings)
            || proof["version"]!="sdk-wrapper-preparation-repair/v1"||proof["attemptId"]!=id
            || proof["priorRecoveryProofSha256"]!=hash(&job["prepared"][FIELD]["proof"])
            || proof["evidenceSha256"]!=hash(evidence)||proof["effectiveIdentitySha256"]!=hash(&saved["effectiveIdentity"])
            || proof["failedReceiptSha256"]!=evidence["failedReceiptSha256"]||proof["runtimeInventorySha256"]!=evidence["runtimeInventorySha256"]
            || proof["sdkEvents"]!=0||proof["processTreeStopProven"]!=true||proof["beforeOfficialRuntimeSpawn"]!=true
            || !crate::text_sdk::local_recovery_identity_bound(&job["snapshot"]["executorIdentity"],&saved["effectiveIdentity"]) {return Err(HELD.into());}
        if ["received","completed","rejected"].contains(&s(job,"status")) {
            let attempt=FrozenAttempt::load(&self.store.root.join("text-runs").join(id).join("sdk-attempt.json"))?;
            crate::text_sdk::repaired_receipt_bound(&self.store.root,&attempt,saved)?;
        }Ok(Some(saved.clone()))
    }
    pub(super) fn sdk_local_recovery_bound(&self, job: &Value) -> Result<Option<Value>> {
        let Some(saved)=job["prepared"].get(FIELD) else{return Ok(None);};
        let proof=&saved["proof"]; let evidence=&saved["evidence"];
        let id=s(job,"id");
        let event:Option<String>=self.store.conn()?.query_row("SELECT data FROM events WHERE job_id=?1 AND type=?2",params![id,EVENT],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let count:i64=self.store.conn()?.query_row("SELECT count(*) FROM events WHERE job_id=?1 AND type=?2",params![id,EVENT],|r|r.get(0)).map_err(|e|e.to_string())?;
        if count!=1 || event.as_deref()!=Some(saved.to_string().as_str()) || saved["originalSettingsSha256"]!=hash(&self.settings)
            || proof["version"]!="sdk-local-preparation-recovery/v1" || proof["attemptId"]!=id
            || proof["evidenceSha256"]!=hash(evidence) || proof["originalReceiptSeal"]!=evidence["originalReceiptSeal"]
            || proof["effectiveIdentitySha256"]!=hash(&saved["effectiveIdentity"])
            || proof["sdkEvents"]!=0 || proof["processTreeStopProven"]!=true
            || !crate::text_sdk::local_recovery_identity_bound(&job["snapshot"]["executorIdentity"],&saved["effectiveIdentity"])
            || job["prepared"]["sdkDispatchStarted"]!=true || job["prepared"]["sdkRequestHash"]!=hash(&job["snapshot"])
            || job["prepared"]["sdkAttemptId"]!=id || job["prepared"]["sdkOriginWorkflowId"]!=self.workflow_id.as_deref().unwrap_or("standalone") {return Err(HELD.into());}
        if ["received","completed","rejected"].contains(&s(job,"status")) {
            let attempt=FrozenAttempt::load(&self.store.root.join("text-runs").join(id).join("sdk-attempt.json"))?;
            crate::text_sdk::recovered_receipt_bound(&self.store.root,&attempt,saved)?;
        }
        self.sdk_wrapper_repair_bound(job)?;
        Ok(Some(saved.clone()))
    }
    pub(super) async fn sdk_recover_local_preparation(&self, request:&TextRequest, snapshot:&Value, job:&Value, original:&FrozenAttempt) -> Result<Option<Value>> {
        if snapshot["executorIdentity"]["bridgeSha256"]!=crate::text_sdk::LOCAL_PATH_BRIDGE_SHA {return Ok(None);}
        let run=self.workflow_id.as_deref().ok_or(HELD)?;let id=s(job,"id");
        if original.attempt_id!=id || original.workflow_id!=run || original.logical_node_id!=step_key(request)
            || original.runtime_identity!=snapshot["executorIdentity"] || serde_json::to_value(&original.request).map_err(|e|e.to_string())?!=serde_json::to_value(request).map_err(|e|e.to_string())? {return Err(HELD.into());}
        let compiled=compile_sdk_attempt_input(original)?;
        let expected=json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)});
        let compiled_path=self.store.root.join("text-runs").join(id).join("sdk-compiled.json");
        let meta=std::fs::symlink_metadata(&compiled_path).map_err(|_|HELD)?;
        if !meta.is_file() || meta.is_symlink() || meta.len()>2_097_152{return Err(HELD.into());}
        let stored:Value=serde_json::from_slice(&std::fs::read(compiled_path).map_err(|_|HELD)?).map_err(|_|HELD)?;
        if stored!=expected || job["prepared"]["compiledPromptSha256"]!=expected["promptSha256"] || job["prepared"]["compiledSchemaSha256"]!=expected["schemaSha256"]
            || job["prepared"]["sdkRequestHash"]!=hash(snapshot) || job["prepared"]["sdkAttemptId"]!=id || job["prepared"]["sdkDispatchStarted"]!=true
            || job["prepared"]["sdkOriginWorkflowId"]!=run {return Err(HELD.into());}
        let mut first=false;
        let saved=if let Some(saved)=self.sdk_local_recovery_bound(job)? {saved} else {
            let before=events(&self.store.conn()?,id)?;
            let evidence=crate::text_sdk::local_preparation_evidence(&self.store.root,original,&before)?;
            let verified=crate::text_sdk::prepare_local_recovery_handoff(&self.settings).await?;
            self.sdk_parent_binding()?;self.sdk_checkpoint_binding(request,snapshot)?;
            let proof=json!({"version":"sdk-local-preparation-recovery/v1","attemptId":id,"attemptEnvelopeHash":original.attempt_envelope_hash,
                "originalReceiptSeal":evidence["originalReceiptSeal"],"evidenceSha256":hash(&evidence),
                "effectiveIdentitySha256":hash(&verified.identity),"sdkEvents":0,"processTreeStopProven":true});
            let saved=json!({"proof":proof,"evidence":evidence,"effectiveIdentity":verified.identity,
                "resourceHashes":verified.resource_hashes,"originalSettingsSha256":hash(&self.settings)});
            let mut c=self.store.conn()?;
            let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
            let changed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk' AND state='submission_unknown' AND snapshot=?2 AND prepared=?3 AND result IS NULL AND receipt IS NULL)",params![id,snapshot.to_string(),job["prepared"].to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
            let other:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id<>?1 AND kind='text' AND provider='codex_sdk' AND state NOT IN ('completed','rejected') AND json_extract(prepared,'$.sdkDispatchStarted')=1)",[id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !changed || other || events(&tx,id)?!=before || crate::text_sdk::local_preparation_evidence(&self.store.root,original,&before)?!=saved["evidence"] {return Err(HELD.into());}
            crate::text_sdk::verify_local_recovery_bytes(&snapshot["executorIdentity"],&saved["effectiveIdentity"],&saved["resourceHashes"])?;
            let mut prepared=job["prepared"].clone();prepared[FIELD]=saved.clone();
            tx.execute("UPDATE jobs SET prepared=?2,updated_at=?3 WHERE id=?1",params![id,prepared.to_string(),now()]).map_err(|e|e.to_string())?;
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![id,EVENT,saved.to_string(),now()]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e|e.to_string())?;first=true;saved
        };
        if !first && saved["effectiveIdentity"]["bridgeSha256"]==crate::text_sdk::WRAPPER_FAILED_BRIDGE_SHA
            && saved["effectiveIdentity"]["supervisorSha256"]==crate::text_sdk::WRAPPER_FAILED_SUPERVISOR_SHA {
            return self.sdk_repair_wrapper_preparation(request,snapshot,job,original,&saved).await.map(Some);
        }
        crate::text_sdk::verify_local_recovery_bytes(&snapshot["executorIdentity"],&saved["effectiveIdentity"],&saved["resourceHashes"])?;
        let config=crate::text_sdk::Config::from_frozen_identity(&self.settings,&saved["effectiveIdentity"],&self.store.root)?.for_request(&self.store.root,request)?;
        let transport=self.sdk_transport(config,id);
        // A reserved intent never dispatches again, including a crash before
        // the RPC. It may only recover an already durable original result.
        let observed=if first {transport.recover_local_preparation(original,&saved["proof"]).await} else {transport.reconcile(original).await};
        transport.close().await?;
        let observation=observed?;
        if observation.kind!=ObservationKind::Completed {return Err(HELD.into());}
        let file=self.store.root.join("text-runs").join(id).join("sdk-attempt.json");
        let mut attempt=FrozenAttempt::load(&file)?;
        attempt.observe(observation)?;
        crate::text_sdk::recovered_receipt_bound(&self.store.root,&attempt,&saved)?;
        attempt.persist(&file)?;
        if attempt.state!=AttemptState::Completed{return Err(HELD.into());}
        Ok(Some(self.sdk_receive(request,snapshot,id,&attempt)?))
    }
    async fn sdk_repair_wrapper_preparation(&self,request:&TextRequest,snapshot:&Value,job:&Value,original:&FrozenAttempt,previous:&Value)->Result<Value> {
        let id=s(job,"id");let mut first=false;
        let saved=if let Some(saved)=self.sdk_wrapper_repair_bound(job)?{saved}else{
            let before=events(&self.store.conn()?,id)?;
            let evidence=crate::text_sdk::wrapper_preparation_evidence(&self.store.root,original,previous,&before)?;
            let verified=crate::text_sdk::prepare_local_recovery_handoff(&self.settings).await?;
            self.sdk_parent_binding()?;self.sdk_checkpoint_binding(request,snapshot)?;
            let proof=json!({"version":"sdk-wrapper-preparation-repair/v1","attemptId":id,"attemptEnvelopeHash":original.attempt_envelope_hash,
                "priorRecoveryProofSha256":hash(&previous["proof"]),"failedReceiptSha256":evidence["failedReceiptSha256"],"runtimeInventorySha256":evidence["runtimeInventorySha256"],
                "evidenceSha256":hash(&evidence),"effectiveIdentitySha256":hash(&verified.identity),"sdkEvents":0,"processTreeStopProven":true,"beforeOfficialRuntimeSpawn":true});
            let saved=json!({"proof":proof,"evidence":evidence,"effectiveIdentity":verified.identity,"resourceHashes":verified.resource_hashes,"originalSettingsSha256":hash(&self.settings)});
            let mut c=self.store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
            let same:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk' AND state='submission_unknown' AND snapshot=?2 AND prepared=?3 AND result IS NULL AND receipt IS NULL)",params![id,snapshot.to_string(),job["prepared"].to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
            let other:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id<>?1 AND kind='text' AND provider='codex_sdk' AND state NOT IN ('completed','rejected') AND json_extract(prepared,'$.sdkDispatchStarted')=1)",[id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !same||other||events(&tx,id)?!=before||crate::text_sdk::wrapper_preparation_evidence(&self.store.root,original,previous,&before)?!=saved["evidence"]{return Err(HELD.into());}
            crate::text_sdk::verify_local_recovery_bytes(&snapshot["executorIdentity"],&saved["effectiveIdentity"],&saved["resourceHashes"])?;
            let mut prepared=job["prepared"].clone();prepared[REPAIR_FIELD]=saved.clone();
            tx.execute("UPDATE jobs SET prepared=?2,updated_at=?3 WHERE id=?1",params![id,prepared.to_string(),now()]).map_err(|e|e.to_string())?;
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![id,REPAIR_EVENT,saved.to_string(),now()]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e|e.to_string())?;first=true;saved
        };
        crate::text_sdk::verify_local_recovery_bytes(&snapshot["executorIdentity"],&saved["effectiveIdentity"],&saved["resourceHashes"])?;
        let transport=self.sdk_transport(crate::text_sdk::Config::from_frozen_identity(&self.settings,&saved["effectiveIdentity"],&self.store.root)?.for_request(&self.store.root,request)?,id);
        let observed=if first{transport.recover_local_preparation(original,&saved["proof"]).await}else{transport.reconcile(original).await};
        transport.close().await?;let observed=observed?;if observed.kind!=ObservationKind::Completed{return Err(HELD.into());}
        let file=self.store.root.join("text-runs").join(id).join("sdk-attempt.json");let mut attempt=FrozenAttempt::load(&file)?;
        attempt.observe(observed)?;
        crate::text_sdk::recovered_receipt_bound(&self.store.root,&attempt,previous)?;
        crate::text_sdk::repaired_receipt_bound(&self.store.root,&attempt,&saved)?;
        attempt.persist(&file)?;self.sdk_receive(request,snapshot,id,&attempt)
    }
}
