//! Same-job preparation only. No missing receipt is interpreted as a submit retry.
use super::*;
use rusqlite::OptionalExtension;
const HELD:&str="CHROME_PREPARE_RECOVERY_HELD: 原准备证据不足，保留原请求/错误，不重新开页或提交";
fn hash(value:&Value)->String {crate::pipeline::digest(&value.to_string())}
fn empty(value:&Value)->bool {value.is_null() || value==&json!({}) || value==""}
fn envelope(store:&Store,key:&str,cfg:&ProviderConfig,intent:&Value)->Result<Value> {
    let job=store.job(key)?;
    let request=generation_request(&job["snapshot"])?;
    let request_hash=hash(&serde_json::to_value(request).map_err(|e|e.to_string())?);
    if job["provider"]!="chrome" || job["kind"]!="image" || cfg.kind!="chrome"
        || intent["version"]!="chrome-prepare-intent/v1" || intent["requestHash"]!=request_hash
        || intent["requestId"]!=job["snapshot"]["requestId"] {return Err(HELD.into());}
    let mut value=json!({"version":"chrome-prepare-job-intent/v1","jobId":key,"snapshotSha256":hash(&job["snapshot"]),
        "configSha256":hash(&json!(cfg)),"requestHash":request_hash,"intent":intent});
    value["sha256"]=json!(hash(&value));Ok(value)
}
pub(super) fn observer<'a>(store:&'a Store,key:&'a str,cfg:&'a ProviderConfig)->impl Fn(&Value)->ProviderResult<()>+'a {
    move |intent| {
        let work=||->Result<()> {
            let value=envelope(store,key,cfg,intent)?;
            let c=store.conn()?;
            let old:Option<String>=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type='chrome_prepare_intent' ORDER BY id LIMIT 1",[key],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
            if let Some(old)=old {if old!=value.to_string(){return Err(HELD.into());}return Ok(());}
            store.event(key,"chrome_prepare_intent",&value)
        };
        work().map_err(|_|ProviderError::new("CHROME_PREPARE_PROOF_PERSIST","首次准备运行身份未持久化，未继续开页或准备"))
    }
}
fn proof(store:&Store,key:&str,cfg:&ProviderConfig)->Result<Option<Value>> {
    let job=store.job(key)?;
    if job["provider"]!="chrome" || job["kind"]!="image" || cfg.kind!="chrome" {return Ok(None);}
    if !unsubmitted(&job) || !job["prepared"].is_null() || !empty(&job["receipt"]) || !job["result"].is_null() {return Ok(None);}
    if !["queued","preparing","failed","recovery_paused"].contains(&s(&job,"status")) {return Ok(None);}
    let has_intent:bool=store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type='chrome_prepare_intent')",[key],|row|row.get(0)).map_err(|e|e.to_string())?;
    if s(&job,"status")=="queued" && !has_intent {return Ok(None);}
    if job["snapshot"]["providerConfig"]!=json!(cfg) {return Err(HELD.into());}
    binding::validate_generation_snapshot(store,&job["snapshot"])?;
    let request=generation_request(&job["snapshot"])?;
    let request_hash=hash(&json!(request));
    let c=store.conn()?;
    let mut q=c.prepare("SELECT type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let events=q.query_map([key],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).map_err(|e|e.to_string())?
        .collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let mut begin=false;let mut failed=false;let mut intent=None;
    for (kind,raw) in &events {
        let value:Value=serde_json::from_str(raw).map_err(|_|HELD)?;
        if kind=="chrome_prepare_recovery_exhausted" {return Ok(None);}
        if kind.contains("submit") || kind.contains("query") || kind.contains("download") || kind.contains("reconcile")
            || ["ready","submitting","submitted","processing","submission_unknown","succeeded","downloading","downloaded"].iter()
                .any(|state|value["state"]==*state || value["from"]==*state) {return Ok(None);}
        if kind=="state_claim" && value["from"]=="queued" && value["state"]=="preparing" {begin=true;}
        if kind=="state_claim" && value["from"]=="preparing" && value["state"]=="failed" && begin {failed=true;}
        if kind=="chrome_prepare_intent" {
            let mut unsigned=value.clone();unsigned.as_object_mut().ok_or(HELD)?.remove("sha256");
            if value["sha256"]!=hash(&unsigned) || value["jobId"]!=key || value["snapshotSha256"]!=hash(&job["snapshot"])
                || value["configSha256"]!=hash(&json!(cfg)) || value["requestHash"]!=request_hash
                || value!=envelope(store,key,cfg,&value["intent"])? {return Err(HELD.into());}
            if intent.as_ref().is_some_and(|old|old!=&value["intent"]) {return Err(HELD.into());}
            intent=Some(value["intent"].clone());
        }
    }
    if let Some(intent)=intent {
        return Ok(Some(json!({"version":"chrome-prepare-recovery/v1","requestHash":request_hash,"noLocalSubmit":true,"intent":intent,"legacyNoSubmitAudit":null})));
    }
    // Legacy failures may FIRST finish prepare. No old runtime nonce is invented.
    // They still require the complete original queued→preparing→failed chain.
    if !begin || !failed {return Ok(None);}
    Ok(Some(json!({"version":"chrome-prepare-recovery/v1","requestHash":request_hash,"noLocalSubmit":true,"intent":null,
        "legacyNoSubmitAudit":{"version":"chrome-legacy-no-submit/v1","jobId":key,"snapshotSha256":hash(&job["snapshot"]),
            "configSha256":hash(&json!(cfg)),"eventHistorySha256":hash(&json!(events)),"stateChainProven":true,"preparedNeverCompleted":true}})))
}
pub(crate) fn chrome_prepare_recovery_eligible(store:&Store,job:&Value,cfg:&ProviderConfig)->Result<bool> {
    if proof(store,s(job,"id"),cfg)?.is_none() {return Ok(false);}
    Ok(!saved_budget(store,s(job,"id"),&job["snapshot"])? .as_ref().is_some_and(budget_exhausted))
}
pub(super) fn has_proof(store:&Store,job:&Value,cfg:&ProviderConfig)->Result<bool> {
    Ok(proof(store,s(job,"id"),cfg)?.is_some())
}
fn saved_budget(store:&Store,key:&str,snapshot:&Value)->Result<Option<Value>> {
    let raw:Option<String>=store.conn()?.query_row("SELECT value FROM metadata WHERE key=?1",[format!("chrome_prepare_recovery:{key}")],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
    let Some(raw)=raw else {return Ok(None);};
    let budget:Value=serde_json::from_str(&raw).map_err(|_|HELD)?;
    if budget["snapshotSha256"]!=hash(snapshot) || budget["version"]!="chrome-prepare-recovery-budget/v1"
        || budget["startedAt"].as_i64().is_none() || budget["attempts"].as_u64().is_none() {return Err(HELD.into());}
    Ok(Some(budget))
}
fn budget_exhausted(budget:&Value)->bool {
    let elapsed=chrono::Utc::now().timestamp()-budget["startedAt"].as_i64().unwrap();
    elapsed<0 || elapsed>180 || budget["attempts"].as_u64().unwrap()>=3
}
pub(crate) fn retire_exhausted_chrome_prepare(store:&Store,job:&Value,cfg:&ProviderConfig)->Result<bool> {
    // Never classify a binding/history conflict as a budget terminal. All
    // original zero-submit proofs must still validate before this narrow seal.
    if proof(store,s(job,"id"),cfg)?.is_none() {return Ok(false);}
    let Some(budget)=saved_budget(store,s(job,"id"),&job["snapshot"])? else {return Ok(false);};
    if !budget_exhausted(&budget) {return Ok(false);}
    let mut c=store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let count=tx.execute("UPDATE jobs SET state='failed',next_attempt_at=0,updated_at=?1 WHERE id=?2 AND state=?3 AND snapshot=?4 AND prepared IS NULL AND result IS NULL AND (task_id IS NULL OR task_id='') AND (receipt IS NULL OR receipt IN ('','null','{}'))",
        rusqlite::params![now(),s(job,"id"),s(job,"status"),job["snapshot"].to_string()]).map_err(|e|e.to_string())?;
    if count==1 {tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_prepare_recovery_exhausted',?2,?3)",rusqlite::params![s(job,"id"),
        json!({"version":"chrome-prepare-budget-terminal/v1","code":"CHROME_PREPARE_RECOVERY_EXHAUSTED","snapshotSha256":hash(&job["snapshot"]),"requestId":job["snapshot"]["requestId"],
            "previousState":job["status"],"previousError":job["error"],"budget":budget,"generationCalled":false,"budgetResetAllowed":false,"independentJobsMayContinue":true}).to_string(),now()]).map_err(|e|e.to_string())?;}
    tx.commit().map_err(|e|e.to_string())?;if count==1 {store.dirty()?;}Ok(count==1)
}
pub(super) fn begin(store:&Store,key:&str,cfg:&ProviderConfig)->Result<Option<Value>> {
    let Some(proof)=proof(store,key,cfg)? else {return Ok(None);};
    let metadata=format!("chrome_prepare_recovery:{key}");
    let original=store.job(key)?;
    let mut budget=saved_budget(store,key,&original["snapshot"])? .unwrap_or_else(||json!({"version":"chrome-prepare-recovery-budget/v1","snapshotSha256":hash(&original["snapshot"]),"startedAt":chrono::Utc::now().timestamp(),"attempts":0}));
    if budget_exhausted(&budget) {
        return Err("CHROME_PREPARE_RECOVERY_EXHAUSTED: 原页准备恢复已到有界上限，保留原任务，不开新页".into());
    }
    budget["attempts"]=json!(budget["attempts"].as_u64().unwrap()+1);
    store.conn()?.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![metadata,budget.to_string()]).map_err(|e|e.to_string())?;
    store.event(key,"chrome_prepare_recovery_attempt",&json!({"attempt":budget["attempts"],"sameRequestId":true,"newTabRetryAllowed":false,"legacyFirstOpenRequiresNativeAbsence":true}))?;
    Ok(Some(proof))
}
pub(super) fn busy_wait(store:&Store,key:&str)->Result<()> {
    let metadata=format!("chrome_prepare_recovery:{key}");
    let old:Option<String>=store.conn()?.query_row("SELECT value FROM metadata WHERE key=?1",[&metadata],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
    if let Some(old)=old {let mut budget:Value=serde_json::from_str(&old).map_err(|_|HELD)?;budget["attempts"]=json!(budget["attempts"].as_u64().ok_or(HELD)?.saturating_sub(1));
        store.conn()?.execute("UPDATE metadata SET value=?1 WHERE key=?2 AND value=?3",rusqlite::params![budget.to_string(),metadata,old]).map_err(|e|e.to_string())?;}
    Ok(())
}
pub(crate) fn resume_chrome_prepare(store:&Store,key:&str,cfg:&ProviderConfig,account:&str,channels:&dyn Channels)->Result<()> {
    // Preparation only: the enclosing scheduler must separately recheck its
    // original channel/unknown gate before the first authorized submit.
    let _=super::prepare_generation(store,key,cfg,account,channels,false)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex,atomic::{AtomicUsize,Ordering}};
    struct Fake {proofs:Mutex<Vec<Value>>,calls:AtomicUsize,busy:bool}
    impl Channels for Fake {
        fn capabilities(&self,_:&ProviderConfig)->ProviderResult<Capabilities>{panic!("no catalogue path in fixture");}
        fn prepare(&self,_:&ProviderConfig,_:&GenerationRequest)->ProviderResult<PreparedRequest>{panic!("recovery must not open a fresh page");}
        fn recover_prepare(&self,c:&ProviderConfig,r:&GenerationRequest,proof:&Value,observe:&providers::PrepareObserver<'_>)->ProviderResult<PreparedRequest> {
            self.calls.fetch_add(1,Ordering::SeqCst);self.proofs.lock().unwrap().push(proof.clone());
            if self.busy {return Err(ProviderError::new("BRIDGE_BUSY","original standalone command pending"));}
            if proof["intent"].is_null() {observe(&json!({"version":"chrome-prepare-intent/v1","mode":"legacy_first_prepare","requestId":r.request_id,"requestHash":hash(&json!(r)),"createdAt":now()}))?;}
            Ok(PreparedRequest{provider:c.kind.clone(),request:r.clone(),capability_version:"synthetic".into(),upload_receipts:vec![],wire:json!({"sameOriginalPage":true}),estimated_cost:None,prepared_at:now()})
        }
        fn submit(&self,_:&ProviderConfig,_:&PreparedRequest)->ProviderResult<SubmitReceipt>{panic!("prepare-only restoration never submits");}
        fn query(&self,_:&ProviderConfig,_:&str)->ProviderResult<QueryResult>{panic!("no generation query");}
        fn download(&self,_:&ProviderConfig,_:&str,_:&Path)->ProviderResult<DownloadReceipt>{panic!("no download");}
        fn reconcile(&self,_:&ProviderConfig,_:&str)->ProviderResult<QueryResult>{panic!("unknown must remain held");}
    }
    fn fixture()->(tempfile::TempDir,Store,String,ProviderConfig) {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("synthetic original prepare","fixture").unwrap();
        store.save_source(json!(crate::pipeline::prepare_source("第1集\n场景：门口\n甲：走吧。\n").unwrap())).unwrap();
        let a=store.save_asset(json!({"id":"hero","variantId":"base","name":"甲","prompt":"synthetic image"})).unwrap();
        let cfg:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","endpoint":"http://127.0.0.1:1","bridgeTokenEnv":"SYNTHETIC_NEVER_USED","timeoutSeconds":5})).unwrap();
        let snapshot=json!({"requestId":"original-request","kind":"image","model":"chatgpt-image","prompt":"synthetic image","ratio":"16:9","resolution":"native","options":{},"references":[],"providerConfig":cfg,
            "assetId":"hero","variantId":"base","assetVersion":a["version"],"sourceVersion":store.source().unwrap()["version"],"autoSubmit":true});
        let key=store.add_job("image","hero","chrome","original-account",&snapshot,"queued").unwrap();
        store.claim_job(&key,"queued","preparing").unwrap();store.claim_job_fields(&key,"preparing","failed",&json!({"error":"original safe timeout detail"})).unwrap();
        (temp,store,key,cfg)
    }
    #[test]
    fn chrome_prepare_recovery_legacy_restores_original_job_ready_preserves_snapshot_and_error_history_zero_submit() {
        let (_temp,store,key,cfg)=fixture();let original=store.job(&key).unwrap();let fake=Fake{proofs:Mutex::new(vec![]),calls:AtomicUsize::new(0),busy:false};
        resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake).unwrap();let after=store.job(&key).unwrap();
        assert_eq!(after["status"],"ready");assert_eq!(after["snapshot"],original["snapshot"]);assert!(after["providerTaskId"].is_null());assert_eq!(store.jobs().unwrap().len(),1);assert_eq!(fake.calls.load(Ordering::SeqCst),1);
        assert_eq!(fake.proofs.lock().unwrap()[0]["legacyNoSubmitAudit"]["stateChainProven"],true);
        let original_error:bool=store.conn().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type='state_claim' AND json_extract(data,'$.state')='failed')",[&key],|r|r.get(0)).unwrap();assert!(original_error);
        let previous_error:String=store.conn().unwrap().query_row("SELECT json_extract(data,'$.previousError') FROM events WHERE job_id=?1 AND type='chrome_prepare_recovery_started'",[&key],|r|r.get(0)).unwrap();assert_eq!(previous_error,"original safe timeout detail");
    }
    #[test]
    fn chrome_prepare_recovery_any_prepared_receipt_submit_unknown_or_incomplete_history_remains_held() {
        for change in ["prepared","receipt","task","unknown","ready_history","submit_history","missing_history","snapshot","config"] {
            let (_temp,store,key,mut cfg)=fixture();
            match change {"prepared"=>store.update_job(&key,"failed",&json!({"prepared":{}})).unwrap(),"receipt"=>store.update_job(&key,"failed",&json!({"receipt":{"accepted":true}})).unwrap(),
                "task"=>store.update_job(&key,"failed",&json!({"providerTaskId":"prior-task"})).unwrap(),"unknown"=>store.update_job(&key,"submission_unknown",&json!({})).unwrap(),
                "ready_history"=>store.event(&key,"state_claim",&json!({"from":"preparing","state":"ready"})).unwrap(),"submit_history"=>store.event(&key,"state_claim",&json!({"from":"ready","state":"submitting"})).unwrap(),
                "missing_history"=>{store.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1",[&key]).unwrap();},
                "snapshot"=>{let mut snapshot=store.job(&key).unwrap()["snapshot"].clone();snapshot["assetVersion"]=json!(9);store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",rusqlite::params![snapshot.to_string(),key]).unwrap();},
                _=>cfg.endpoint=Some("http://127.0.0.1:2".into())};
            let fake=Fake{proofs:Mutex::new(vec![]),calls:AtomicUsize::new(0),busy:false};let before=store.jobs().unwrap();let _=resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake);
            assert_eq!(fake.calls.load(Ordering::SeqCst),0,"{change}");assert_eq!(store.jobs().unwrap(),before,"{change}");
        }
    }
    #[test]
    fn chrome_prepare_recovery_busy_is_waiting_not_failure_and_has_fixed_expiry_and_budget() {
        let (_temp,store,key,cfg)=fixture();let fake=Fake{proofs:Mutex::new(vec![]),calls:AtomicUsize::new(0),busy:true};
        for _ in 0..4 {resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake).unwrap();assert_eq!(store.job(&key).unwrap()["status"],"recovery_paused");}
        let metadata=format!("chrome_prepare_recovery:{key}");let raw:String=store.conn().unwrap().query_row("SELECT value FROM metadata WHERE key=?1",[&metadata],|r|r.get(0)).unwrap();let mut budget:Value=serde_json::from_str(&raw).unwrap();assert_eq!(budget["attempts"],0);
        budget["startedAt"]=json!(chrono::Utc::now().timestamp()-181);store.conn().unwrap().execute("UPDATE metadata SET value=?1 WHERE key=?2",rusqlite::params![budget.to_string(),metadata]).unwrap();
        let calls=fake.calls.load(Ordering::SeqCst);assert!(resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake).unwrap_err().contains("EXHAUSTED"));assert_eq!(fake.calls.load(Ordering::SeqCst),calls);
        for (started,attempts) in [(chrono::Utc::now().timestamp()+60,0),(chrono::Utc::now().timestamp(),3)] {
            budget["startedAt"]=json!(started);budget["attempts"]=json!(attempts);
            store.conn().unwrap().execute("UPDATE metadata SET value=?1 WHERE key=?2",rusqlite::params![budget.to_string(),format!("chrome_prepare_recovery:{key}")]).unwrap();
            assert!(resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake).unwrap_err().contains("EXHAUSTED"));assert_eq!(fake.calls.load(Ordering::SeqCst),calls);
        }
    }
    #[test]
    fn chrome_prepare_recovery_saved_first_runtime_survives_restart_and_cannot_rebind_request_config_or_epoch() {
        let (_temp,store,key,cfg)=fixture();let original=store.job(&key).unwrap();let request=generation_request(&original["snapshot"]).unwrap();
        let intent=json!({"version":"chrome-prepare-intent/v1","mode":"new_prepare","requestId":request.request_id,"requestHash":hash(&json!(request)),"runtime":{"backgroundEpoch":"original"},"createdAt":now()});
        observer(&store,&key,&cfg)(&intent).unwrap();let reopened=Store::open(store.root.clone()).unwrap();assert!(proof(&reopened,&key,&cfg).unwrap().is_some());
        let mut changed=intent.clone();changed["runtime"]["backgroundEpoch"]=json!("other");assert!(observer(&reopened,&key,&cfg)(&changed).is_err());
        let fake=Fake{proofs:Mutex::new(vec![]),calls:AtomicUsize::new(0),busy:false};resume_chrome_prepare(&reopened,&key,&cfg,"original-account",&fake).unwrap();
        assert_eq!(fake.proofs.lock().unwrap()[0]["intent"],intent);assert_eq!(reopened.job(&key).unwrap()["snapshot"],original["snapshot"]);assert_eq!(fake.calls.load(Ordering::SeqCst),1);
    }
}
