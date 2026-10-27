//! Same-job, same-attempt recovery of a proven local schema preparation error.
//! Reservation is durable before dispatch; a consumed reservation only observes.
use super::*;
use crate::text_transport::{AttemptState,FrozenAttempt,ObservationKind,TextTransport};
const FIELD:&str="sdkSchemaPreparationRecovery";
const EVENT:&str="sdk_schema_preparation_recovery_reserved";
const HELD:&str="SDK_SCHEMA_PREPARATION_RECOVERY_HELD: 恢复证据或一次性意图不完整；保留原未知任务，不重复派发";
fn hash(v:&Value)->String{crate::pipeline::digest(&v.to_string())}
fn events(c:&rusqlite::Connection,id:&str)->Result<Value>{
    let mut q=c.prepare("SELECT id,type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let rows=q.query_map([id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
    let mut out=Vec::new();for row in rows{let(id,kind,text)=row.map_err(|e|e.to_string())?;if out.len()>=2048{return Err(HELD.into());}out.push(json!({"id":id,"type":kind,"data":serde_json::from_str::<Value>(&text).map_err(|_|HELD)?}));}Ok(json!(out))
}
fn reserve(store:&Store,job:&Value,snapshot:&Value,saved:&Value,before:&Value,verify:impl FnOnce()->Result<()>)->Result<()> {
    let id=s(job,"id");let mut c=store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let same:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk' AND state='submission_unknown' AND snapshot=?2 AND prepared=?3 AND result IS NULL AND receipt IS NULL)",params![id,snapshot.to_string(),job["prepared"].to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
    let other:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id<>?1 AND kind='text' AND provider='codex_sdk' AND state NOT IN ('completed','rejected') AND json_extract(prepared,'$.sdkDispatchStarted')=1)",[id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !same||other||events(&tx,id)?!=*before{return Err(HELD.into());}verify()?;
    let mut prepared=job["prepared"].clone();prepared[FIELD]=saved.clone();
    tx.execute("UPDATE jobs SET prepared=?2,updated_at=?3 WHERE id=?1",params![id,prepared.to_string(),now()]).map_err(|e|e.to_string())?;
    tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![id,EVENT,saved.to_string(),now()]).map_err(|e|e.to_string())?;tx.commit().map_err(|e|e.to_string())?;Ok(())
}
impl SavedTextExecutor {
    pub(super) fn sdk_schema_recovery_bound(&self,job:&Value)->Result<Option<Value>>{
        let Some(saved)=job["prepared"].get(FIELD) else{return Ok(None);};let proof=&saved["proof"];let evidence=&saved["evidence"];let id=s(job,"id");
        let rows=events(&self.store.conn()?,id)?;let reservations:Vec<_>=rows.as_array().ok_or(HELD)?.iter().filter(|r|r["type"]==EVENT).collect();
        if reservations.len()!=1 || reservations[0]["data"]!=*saved || saved["originalSettingsSha256"]!=hash(&self.settings)
            || proof["version"]!="sdk-schema-preparation-recovery/v1" || proof["attemptId"]!=id || proof["controllerSha256"]!=crate::text_sdk::schema_controller_sha256()
            || proof["evidenceSha256"]!=hash(evidence) || proof["effectiveIdentitySha256"]!=hash(&saved["effectiveIdentity"])
            || proof["originalReceiptSha256"]!=evidence["originalReceiptSha256"] || proof["originalReceiptSeal"]!=evidence["originalReceiptSeal"]
            || proof["runtimeInventorySha256"]!=evidence["runtimeInventorySha256"] || proof["sdkEvents"]!=0 || proof["processTreeStopProven"]!=true || proof["beforeOfficialRuntimeSpawn"]!=true
            || !crate::text_sdk::schema_recovery_identity_bound(&job["snapshot"]["executorIdentity"],&saved["effectiveIdentity"])
            || job["prepared"]["sdkAttemptId"]!=id || job["prepared"]["sdkRequestHash"]!=hash(&job["snapshot"]) || job["prepared"]["sdkDispatchStarted"]!=true
            || job["prepared"]["sdkOriginWorkflowId"]!=self.workflow_id.as_deref().unwrap_or("standalone") {return Err(HELD.into());}
        let original_rows:Vec<_>=rows.as_array().unwrap().iter().take_while(|r|r["type"]!=EVENT).cloned().collect();
        if json!(original_rows)!=evidence["events"] || hash(&json!(original_rows))!=evidence["eventsSha256"] {return Err(HELD.into());}
        if ["received","completed","rejected"].contains(&s(job,"status")) {
            let attempt=FrozenAttempt::load(&self.store.root.join("text-runs").join(id).join("sdk-attempt.json"))?;
            crate::text_sdk::schema_recovered_receipt_bound(&self.store.root,&attempt,saved)?;
        }Ok(Some(saved.clone()))
    }
    pub(super) async fn sdk_recover_schema_preparation(&self,request:&TextRequest,snapshot:&Value,job:&Value,original:&FrozenAttempt)->Result<Option<Value>>{
        let saved_already=job["prepared"].get(FIELD).is_some();
        if !saved_already {
            if snapshot["executorIdentity"]["bridgeSha256"]!="48af55f1835bd6959ede5801fa8ff2ca9068b267b36ec9f751949efdc9481dc7" {return Ok(None);}
            let receipt=crate::text_sdk::worker_root(&self.store.root,&serde_json::from_value::<TextRequest>(job["snapshot"].clone()).map_err(|_|HELD)?)?.join(s(job,"id")).join("receipt.json");
            let m=std::fs::symlink_metadata(&receipt).map_err(|_|HELD)?;if !m.is_file()||m.is_symlink()||m.len()>4_194_304{return Err(HELD.into());}
            let record:Value=serde_json::from_slice(&std::fs::read(receipt).map_err(|_|HELD)?).map_err(|_|HELD)?;
            if record["error"]["code"]!="ENOENT" {return Ok(None);}
        }
        let run=self.workflow_id.as_deref().ok_or(HELD)?;let id=s(job,"id");
        if original.attempt_id!=id || original.workflow_id!=run || original.logical_node_id!=step_key(request) || original.runtime_identity!=snapshot["executorIdentity"]
            || serde_json::to_value(&original.request).map_err(|_|HELD)?!=serde_json::to_value(request).map_err(|_|HELD)? {return Err(HELD.into());}
        let compiled=compile_sdk_attempt_input(original)?;let compiled_path=self.store.root.join("text-runs").join(id).join("sdk-compiled.json");
        let expected=json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)});
        let m=std::fs::symlink_metadata(&compiled_path).map_err(|_|HELD)?;
        if !m.is_file()||m.is_symlink()||m.len()>2_097_152||serde_json::from_slice::<Value>(&std::fs::read(compiled_path).map_err(|_|HELD)?).map_err(|_|HELD)?!=expected
            || job["prepared"]["compiledPromptSha256"]!=expected["promptSha256"]||job["prepared"]["compiledSchemaSha256"]!=expected["schemaSha256"] {return Err(HELD.into());}
        let mut first=false;
        let saved=if let Some(saved)=self.sdk_schema_recovery_bound(job)?{saved}else{
            let before=events(&self.store.conn()?,id)?;let evidence=crate::text_sdk::schema_preparation_evidence(&self.store.root,original,&before)?;
            let verified=crate::text_sdk::prepare_schema_recovery_handoff(&self.settings).await?;
            self.sdk_parent_binding()?;self.sdk_checkpoint_binding(request,snapshot)?;
            let proof=json!({"version":"sdk-schema-preparation-recovery/v1","attemptId":id,"attemptEnvelopeHash":original.attempt_envelope_hash,
                "controllerSha256":crate::text_sdk::schema_controller_sha256(),"originalReceiptSha256":evidence["originalReceiptSha256"],"originalReceiptSeal":evidence["originalReceiptSeal"],
                "runtimeInventorySha256":evidence["runtimeInventorySha256"],"evidenceSha256":hash(&evidence),"effectiveIdentitySha256":hash(&verified.identity),
                "sdkEvents":0,"processTreeStopProven":true,"beforeOfficialRuntimeSpawn":true});
            let saved=json!({"proof":proof,"evidence":evidence,"effectiveIdentity":verified.identity,"resourceHashes":verified.resource_hashes,"originalSettingsSha256":hash(&self.settings)});
            reserve(&self.store,job,snapshot,&saved,&before,||{
                if crate::text_sdk::schema_preparation_evidence(&self.store.root,original,&before)?!=saved["evidence"]{return Err(HELD.into());}
                crate::text_sdk::verify_schema_recovery_bytes(&snapshot["executorIdentity"],&saved["effectiveIdentity"],&saved["resourceHashes"])
            })?;first=true;saved
        };
        crate::text_sdk::verify_schema_recovery_bytes(&snapshot["executorIdentity"],&saved["effectiveIdentity"],&saved["resourceHashes"])?;
        let config=crate::text_sdk::Config::from_frozen_identity(&self.settings,&saved["effectiveIdentity"],&self.store.root)?.for_request(&self.store.root,request)?;let transport=self.sdk_transport(config,id);
        let observation=if first{transport.recover_schema_preparation(original,&saved["proof"],&saved["evidence"]).await}else{transport.reconcile(original).await};
        transport.close().await?;let observation=observation?;if observation.kind!=ObservationKind::Completed{return Err(HELD.into());}
        let file=self.store.root.join("text-runs").join(id).join("sdk-attempt.json");let mut attempt=FrozenAttempt::load(&file)?;attempt.observe(observation)?;
        crate::text_sdk::schema_recovered_receipt_bound(&self.store.root,&attempt,&saved)?;attempt.persist(&file)?;
        if attempt.state!=AttemptState::Completed{return Err(HELD.into());}Ok(Some(self.sdk_receive(request,snapshot,id,&attempt)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(tempfile::TempDir,Store,Value,Value,Value){
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("schema atomic fixture","schema-atomic").unwrap();
        let snapshot=json!({"synthetic":true});let id=store.add_job("text","schema-node","codex_sdk","synthetic",&snapshot,"submission_unknown").unwrap();
        store.update_job(&id,"submission_unknown",&json!({"prepared":{"sdkDispatchStarted":true,"sdkAttemptId":id,"sdkRequestHash":hash(&snapshot)}})).unwrap();
        let job=store.job(&id).unwrap();let before=events(&store.conn().unwrap(),&id).unwrap();(temp,store,job,snapshot,before)
    }
    #[test]
    fn sdk_schema_reservation_is_atomic_one_use_and_preserves_original_unknown() {
        let (_temp,store,job,snapshot,before)=fixture();let barrier=Arc::new(std::sync::Barrier::new(2));let mut workers=Vec::new();
        for n in 0..2 {let(store,job,snapshot,before,barrier)=(store.clone(),job.clone(),snapshot.clone(),before.clone(),barrier.clone());
            workers.push(std::thread::spawn(move||{barrier.wait();reserve(&store,&job,&snapshot,&json!({"syntheticReservation":n}),&before,||Ok(())).is_ok()}));}
        assert_eq!(workers.into_iter().map(|w|usize::from(w.join().unwrap())).sum::<usize>(),1);
        let current=store.job(s(&job,"id")).unwrap();assert_eq!(current["status"],"submission_unknown");assert_eq!(current["snapshot"],snapshot);
        assert!(current["result"].is_null());assert!(current["receipt"].is_null());
        assert_eq!(events(&store.conn().unwrap(),s(&job,"id")).unwrap().as_array().unwrap().iter().filter(|r|r["type"]==EVENT).count(),1);
        assert!(reserve(&store,&job,&snapshot,&json!({}),&before,||Ok(())).is_err());
    }
    #[test]
    fn sdk_schema_reservation_refuses_drift_other_owner_and_failed_evidence_without_writes() {
        for fault in ["snapshot","prepared","result","receipt","state","other","events","evidence"] {
            let (_temp,store,job,mut snapshot,before)=fixture();let id=s(&job,"id");
            match fault {
                "snapshot"=>snapshot["synthetic"]=json!(false),"prepared"=>{store.conn().unwrap().execute("UPDATE jobs SET prepared='{}' WHERE id=?1",[id]).unwrap();},
                "result"=>{store.conn().unwrap().execute("UPDATE jobs SET result='{}' WHERE id=?1",[id]).unwrap();},"receipt"=>{store.conn().unwrap().execute("UPDATE jobs SET receipt='{}' WHERE id=?1",[id]).unwrap();},
                "state"=>{store.conn().unwrap().execute("UPDATE jobs SET state='running' WHERE id=?1",[id]).unwrap();},
                "other"=>{let other=store.add_job("text","other","codex_sdk","synthetic",&json!({}),"submission_unknown").unwrap();store.update_job(&other,"submission_unknown",&json!({"prepared":{"sdkDispatchStarted":true}})).unwrap();},
                "events"=>{store.event(id,"foreign",&json!({})).unwrap();},"evidence"=>(),_=>unreachable!(),
            }
            let original=store.job(id).unwrap();let event_before=events(&store.conn().unwrap(),id).unwrap();
            assert!(reserve(&store,&job,&snapshot,&json!({}),&before,||if fault=="evidence"{Err("synthetic evidence drift".into())}else{Ok(())}).is_err(),"{fault}");
            assert_eq!(store.job(id).unwrap(),original,"{fault}");assert_eq!(events(&store.conn().unwrap(),id).unwrap(),event_before,"{fault}");
        }
    }
}
