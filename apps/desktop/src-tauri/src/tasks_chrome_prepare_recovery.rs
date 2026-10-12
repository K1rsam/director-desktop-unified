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
            if intent["version"]=="chrome-prepare-execution-start/v1" {
                let job=store.job(key)?;let request=generation_request(&job["snapshot"])?;
                if intent["requestId"]!=request.request_id || intent["requestHash"]!=hash(&json!(request)) || intent["configSha256"]!=hash(&json!(cfg)) {return Err(HELD.into());}
                return execution_started(store,key,cfg,"fresh_native_idle");
            }
            // Older provider fixtures/contracts still start execution before
            // persisting an actual intent; never interpret it as queue waiting.
            let value=envelope(store,key,cfg,intent)?;
            execution_started(store,key,cfg,"before_prepare_intent")?;
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
        if kind=="chrome_prepare_recovery_exhausted" && !saved_budget(store,key,&job["snapshot"])?
            .is_some_and(|b|b.get("connectionWaitCorrection").is_some() && !budget_exhausted(&b)) {return Ok(None);}
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
    if budget["snapshotSha256"]!=hash(snapshot) || !["chrome-prepare-recovery-budget/v1","chrome-prepare-recovery-budget/v2"].contains(&s(&budget,"version"))
        || budget["startedAt"].as_i64().is_none() || budget["attempts"].as_u64().is_none() {return Err(HELD.into());}
    if budget["version"]=="chrome-prepare-recovery-budget/v2" {
        if budget.get("executionStartedAt").is_none() || !budget["executionStartedAt"].is_null() && budget["executionStartedAt"].as_i64().is_none()
            || budget["executionStartedAt"].is_null() && budget["attempts"]!=0 {return Err(HELD.into());}
        let cutoff=if let Some(correction)=budget.get("connectionWaitCorrection") {
            let cutoff=correction["throughEventId"].as_i64().ok_or(HELD)?;
            let original=&correction["originalBudget"];
            if original["startedAt"]!=budget["startedAt"] || original["snapshotSha256"]!=budget["snapshotSha256"]
                || connection_wait_correction(store,key,original,Some(cutoff))?.as_ref()!=Some(correction) {return Err(HELD.into());}
            cutoff
        }else{0};
        let raw:Option<String>=store.conn()?.query_row("SELECT data FROM events WHERE job_id=?1 AND type='chrome_prepare_execution_started' AND id>?2 ORDER BY id DESC LIMIT 1",rusqlite::params![key,cutoff],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if let Some(raw)=raw {let event:Value=serde_json::from_str(&raw).map_err(|_|HELD)?;
            if event["version"]!="chrome-prepare-execution-clock/v1" || event["jobId"]!=key || event["snapshotSha256"]!=hash(snapshot)
                || event["requestId"]!=snapshot["requestId"] || event["configSha256"]!=hash(&snapshot["providerConfig"])
                || event["executionStartedAt"]!=budget["executionStartedAt"] || event["attempt"]!=budget["attempts"] {return Err(HELD.into());}
        } else if !budget["executionStartedAt"].is_null() || budget["attempts"]!=0 {return Err(HELD.into());}
        if cutoff==0 && budget["attempts"].as_u64().is_some_and(|n|n>0) {
            if let Some(correction)=connection_wait_correction(store,key,&budget,None)? {
                let mut corrected=budget.clone();corrected["attempts"]=json!(0);corrected["executionStartedAt"]=Value::Null;
                corrected["connectionWaitCorrection"]=correction;return Ok(Some(corrected));
            }
        }
        return Ok(Some(budget));
    }
    // Narrow interpretation of complete legacy recovery cycles. Original
    // prepare errors before this recovery chain are not reclassified.
    if budget["attempts"]==0 {
        if let Some(history)=legacy_queue_only_history(store,key)? {
            let mut upgraded=budget.clone();upgraded["version"]=json!("chrome-prepare-recovery-budget/v2");upgraded["executionStartedAt"]=Value::Null;
            upgraded["legacyQueueOnlyMigration"]=json!({"version":"chrome-queue-budget-migration/v1","originalBudget":budget,"eventHistorySha256":hash(&history),"noPrepareIntent":true,"noExecution":true});
            return Ok(Some(upgraded));
        }
    }
    Ok(Some(budget))
}
// Correct only this proven historical classification bug. Keep the original
// budget and all events; any actual intent, fresh proof or remote work forbids it.
fn connection_wait_correction(store:&Store,key:&str,budget:&Value,through:Option<i64>)->Result<Option<Value>> {
    let job=store.job(key)?;
    if budget["version"]!="chrome-prepare-recovery-budget/v2" || budget.get("connectionWaitCorrection").is_some()
        || budget["attempts"].as_u64().is_none_or(|n|n==0) || !unsubmitted(&job) || !job["prepared"].is_null()
        || !empty(&job["receipt"]) || !job["result"].is_null() {return Ok(None);}
    let c=store.conn()?;let mut q=c.prepare("SELECT id,type,data FROM events WHERE job_id=?1 AND id<=?2 ORDER BY id").map_err(|e|e.to_string())?;
    let rows=q.query_map(rusqlite::params![key,through.unwrap_or(i64::MAX)],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?
        .collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let mut attempts=0u64;let mut pending=false;
    for (_,kind,raw) in &rows {
        let v:Value=serde_json::from_str(raw).map_err(|_|HELD)?;
        if kind=="chrome_prepare_intent" || kind.contains("submit") || kind.contains("query") || kind.contains("download") || kind.contains("reconcile")
            || ["ready","submitting","submitted","processing","submission_unknown","succeeded","downloading","downloaded"].iter().any(|s|v["state"]==*s || v["from"]==*s) {return Ok(None);}
        if pending && kind!="chrome_prepare_error" {return Ok(None);}
        if kind=="chrome_prepare_execution_started" {
            if v["reason"]!="non_queue_prepare_outcome" || v["version"]!="chrome-prepare-execution-clock/v1" || v["generationCalled"]!=false
                || v["jobId"]!=key || v["requestId"]!=job["snapshot"]["requestId"] || v["snapshotSha256"]!=hash(&job["snapshot"])
                || v["configSha256"]!=hash(&job["snapshot"]["providerConfig"]) || v["executionStartedAt"]!=budget["executionStartedAt"]
                || v["attempt"]!=attempts+1 {return Ok(None);}
            attempts+=1;pending=true;
        } else if kind=="chrome_prepare_error" && attempts>0 {
            if !pending || v["error"]["code"]!="BRIDGE_BROWSER_NOT_READY" || v["generationCalled"]!=false
                || v["error"]["submissionUnknown"]!=false || v["error"]["authRequired"]!=false || v["sameRequestId"]!=job["snapshot"]["requestId"] {return Ok(None);}
            pending=false;
        }
    }
    if pending || attempts!=budget["attempts"].as_u64().unwrap() {return Ok(None);}
    let Some((last,_,_))=rows.last() else{return Ok(None);};
    if through.is_some_and(|id|id!=*last){return Ok(None);}
    Ok(Some(json!({"version":"chrome-connection-wait-correction/v1","originalBudget":budget,"throughEventId":last,"eventHistorySha256":hash(&json!(rows)),"noPrepareIntent":true,"noActualExecution":true})))
}
fn legacy_queue_only_history(store:&Store,key:&str)->Result<Option<Value>> {
    let c=store.conn()?;let mut q=c.prepare("SELECT type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let rows=q.query_map([key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if rows.iter().any(|(kind,_)|kind=="chrome_prepare_intent" || kind=="chrome_prepare_execution_started") {return Ok(None);}
    let Some(first)=rows.iter().position(|(kind,_)|kind=="chrome_prepare_recovery_attempt") else{return Ok(None);};
    let request_id=store.job(key)?["snapshot"]["requestId"].clone();let mut stage=0;let mut cycles=0;
    for (kind,raw) in &rows[first..] {
        let v:Value=serde_json::from_str(raw).map_err(|_|HELD)?;
        match (stage,kind.as_str()) {
            (0,"chrome_prepare_recovery_attempt") if v["attempt"]==1 && v["sameRequestId"]==true && v["newTabRetryAllowed"]==false=>stage=1,
            (1,"chrome_prepare_recovery_started") if v["requestId"]==request_id && v["firstSubmitAllowed"]==false=>stage=2,
            (2,"state_claim") if v["state"]=="preparing" && ["failed","preparing","recovery_paused"].contains(&s(&v,"from"))=>stage=3,
            (3,"chrome_prepare_error") if v["error"]["code"]=="BRIDGE_BUSY" && v["generationCalled"]==false && v["error"]["submissionUnknown"]==false && v["error"]["authRequired"]==false && v["sameRequestId"]==request_id=>stage=4,
            (4,"state_claim") if v["from"]=="preparing" && v["state"]=="recovery_paused"=>{stage=0;cycles+=1;},
            (0,"browser_queue_wait") if v["code"]=="BRIDGE_BUSY" && v["generationCalled"]==false=>{},
            _=>return Ok(None),
        }
    }
    Ok((stage==0 && cycles>0).then(||json!(rows)))
}
fn budget_exhausted(budget:&Value)->bool {
    if budget["version"]=="chrome-prepare-recovery-budget/v2" && budget["executionStartedAt"].is_null(){return false;}
    let clock=if budget["version"]=="chrome-prepare-recovery-budget/v2" {&budget["executionStartedAt"]}else{&budget["startedAt"]};
    let elapsed=chrono::Utc::now().timestamp()-clock.as_i64().unwrap();
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
    let budget=saved_budget(store,key,&original["snapshot"])? .unwrap_or_else(||json!({"version":"chrome-prepare-recovery-budget/v2","snapshotSha256":hash(&original["snapshot"]),"startedAt":chrono::Utc::now().timestamp(),"executionStartedAt":null,"attempts":0}));
    if budget_exhausted(&budget) {
        return Err("CHROME_PREPARE_RECOVERY_EXHAUSTED: 原页准备恢复已到有界上限，保留原任务，不开新页".into());
    }
    let mut c=store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let old:Option<String>=tx.query_row("SELECT value FROM metadata WHERE key=?1",[&metadata],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if let Some(raw)=&old {let actual:Value=serde_json::from_str(raw).map_err(|_|HELD)?;let expected=if actual["version"]=="chrome-prepare-recovery-budget/v1" {budget.get("legacyQueueOnlyMigration").map(|v|&v["originalBudget"]).unwrap_or(&budget)}else if actual.get("connectionWaitCorrection").is_none() {budget.get("connectionWaitCorrection").map(|v|&v["originalBudget"]).unwrap_or(&budget)}else{&budget};if actual!=*expected{return Err(HELD.into());}}
    if budget.get("legacyQueueOnlyMigration").is_some() && old.as_ref().is_some_and(|raw|serde_json::from_str::<Value>(raw).ok().is_some_and(|v|v["version"]=="chrome-prepare-recovery-budget/v1")) {
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_prepare_queue_budget_migrated',?2,?3)",rusqlite::params![key,budget["legacyQueueOnlyMigration"].to_string(),now()]).map_err(|e|e.to_string())?;
    }
    if let Some(correction)=budget.get("connectionWaitCorrection") {
        if old.as_ref().is_some_and(|raw|serde_json::from_str::<Value>(raw).ok().is_some_and(|v|v.get("connectionWaitCorrection").is_none())) {
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_prepare_connection_wait_corrected',?2,?3)",rusqlite::params![key,correction.to_string(),now()]).map_err(|e|e.to_string())?;
        }
    }
    tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![metadata,budget.to_string()]).map_err(|e|e.to_string())?;tx.commit().map_err(|e|e.to_string())?;
    Ok(Some(proof))
}
pub(super) fn execution_started(store:&Store,key:&str,cfg:&ProviderConfig,reason:&str)->Result<()> {
    let original=store.job(key)?;let Some(mut budget)=saved_budget(store,key,&original["snapshot"])? else{return Ok(());};
    if original["snapshot"]["providerConfig"]!=json!(cfg){return Err(HELD.into());}
    let old=budget.to_string();
    // One marker per actual recovery cycle. Intent persistence may repeat it.
    let c=store.conn()?;let seen:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type='chrome_prepare_execution_started' AND id>(SELECT COALESCE(MAX(id),0) FROM events WHERE job_id=?1 AND type='chrome_prepare_recovery_started'))",[key],|r|r.get(0)).map_err(|e|e.to_string())?;drop(c);
    if seen{return Ok(());}
    if budget_exhausted(&budget){return Err("CHROME_PREPARE_RECOVERY_EXHAUSTED".into());}
    if budget["version"]=="chrome-prepare-recovery-budget/v2" && budget["executionStartedAt"].is_null(){budget["executionStartedAt"]=json!(chrono::Utc::now().timestamp());}
    budget["attempts"]=json!(budget["attempts"].as_u64().ok_or(HELD)?+1);
    let mut c=store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    if tx.execute("UPDATE metadata SET value=?1 WHERE key=?2 AND value=?3",rusqlite::params![budget.to_string(),format!("chrome_prepare_recovery:{key}"),old]).map_err(|e|e.to_string())?!=1{return Err(HELD.into());}
    tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_prepare_execution_started',?2,?3)",rusqlite::params![key,json!({"version":"chrome-prepare-execution-clock/v1","jobId":key,"snapshotSha256":hash(&original["snapshot"]),"configSha256":hash(&json!(cfg)),"requestId":original["snapshot"]["requestId"],"attempt":budget["attempts"],"executionStartedAt":budget["executionStartedAt"],"reason":reason,"generationCalled":false}).to_string(),now()]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e|e.to_string())?;Ok(())
}
pub(super) fn busy_wait(store:&Store,key:&str)->Result<()> {
    let job=store.job(key)?;let budget=saved_budget(store,key,&job["snapshot"])? .ok_or(HELD)?;
    store.event(key,"chrome_prepare_queue_wait_accounted",&json!({"version":"chrome-prepare-queue-wait/v1","generationCalled":false,"budget":budget,"budgetRefunded":false}))
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
    fn chrome_prepare_recovery_busy_observation_does_not_consume_execution_clock_or_reset_history() {
        let (_temp,store,key,cfg)=fixture();let fake=Fake{proofs:Mutex::new(vec![]),calls:AtomicUsize::new(0),busy:true};
        for _ in 0..4 {resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake).unwrap();assert_eq!(store.job(&key).unwrap()["status"],"recovery_paused");}
        let metadata=format!("chrome_prepare_recovery:{key}");let raw:String=store.conn().unwrap().query_row("SELECT value FROM metadata WHERE key=?1",[&metadata],|r|r.get(0)).unwrap();let mut budget:Value=serde_json::from_str(&raw).unwrap();assert_eq!(budget["attempts"],0);
        budget["startedAt"]=json!(chrono::Utc::now().timestamp()-181);store.conn().unwrap().execute("UPDATE metadata SET value=?1 WHERE key=?2",rusqlite::params![budget.to_string(),metadata]).unwrap();
        assert!(budget["executionStartedAt"].is_null());resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake).unwrap();
        let original_started=budget["startedAt"].clone();execution_started(&store,&key,&cfg,"synthetic_execution").unwrap();
        let raw:String=store.conn().unwrap().query_row("SELECT value FROM metadata WHERE key=?1",[&metadata],|r|r.get(0)).unwrap();budget=serde_json::from_str(&raw).unwrap();assert_eq!(budget["startedAt"],original_started);assert_eq!(budget["attempts"],1);
        let calls=fake.calls.load(Ordering::SeqCst);
        for (started,attempts) in [(chrono::Utc::now().timestamp()-181,1),(chrono::Utc::now().timestamp()+60,1),(chrono::Utc::now().timestamp(),3)] {
            budget["executionStartedAt"]=json!(started);budget["attempts"]=json!(attempts);
            store.conn().unwrap().execute("UPDATE metadata SET value=?1 WHERE key=?2",rusqlite::params![budget.to_string(),format!("chrome_prepare_recovery:{key}")]).unwrap();
            store.conn().unwrap().execute("UPDATE events SET data=json_set(data,'$.executionStartedAt',?1,'$.attempt',?2) WHERE job_id=?3 AND type='chrome_prepare_execution_started'",rusqlite::params![started,attempts,key]).unwrap();
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
    fn legacy_busy_budget(store:&Store,key:&str)->Value {
        store.event(key,"chrome_prepare_recovery_attempt",&json!({"attempt":1,"sameRequestId":true,"newTabRetryAllowed":false})).unwrap();
        store.event(key,"chrome_prepare_recovery_started",&json!({"requestId":"original-request","firstSubmitAllowed":false})).unwrap();
        store.claim_job(key,"failed","preparing").unwrap();
        store.event(key,"chrome_prepare_error",&json!({"error":{"code":"BRIDGE_BUSY","submissionUnknown":false,"authRequired":false},"generationCalled":false,"sameRequestId":"original-request"})).unwrap();
        store.claim_job(key,"preparing","recovery_paused").unwrap();
        store.event(key,"browser_queue_wait",&json!({"code":"BRIDGE_BUSY","generationCalled":false})).unwrap();
        let budget=json!({"version":"chrome-prepare-recovery-budget/v1","snapshotSha256":hash(&store.job(key).unwrap()["snapshot"]),"startedAt":chrono::Utc::now().timestamp()-600,"attempts":0});
        store.conn().unwrap().execute("INSERT INTO metadata(key,value) VALUES(?1,?2)",rusqlite::params![format!("chrome_prepare_recovery:{key}"),budget.to_string()]).unwrap();budget
    }
    #[test] fn legacy_complete_zero_execution_wait_migrates_original_clock_once_and_only_execution_consumes_budget(){
        let (_t,store,key,cfg)=fixture();let original=legacy_busy_budget(&store,&key);let snapshot=store.job(&key).unwrap()["snapshot"].clone();
        assert!(chrome_prepare_recovery_eligible(&store,&store.job(&key).unwrap(),&cfg).unwrap());assert!(!retire_exhausted_chrome_prepare(&store,&store.job(&key).unwrap(),&cfg).unwrap());
        begin(&store,&key,&cfg).unwrap();begin(&store,&key,&cfg).unwrap();
        let b=saved_budget(&store,&key,&snapshot).unwrap().unwrap();assert_eq!(b["startedAt"],original["startedAt"]);assert_eq!(b["attempts"],0);assert!(b["executionStartedAt"].is_null());assert_eq!(b["legacyQueueOnlyMigration"]["originalBudget"],original);
        let count:i64=store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='chrome_prepare_queue_budget_migrated'",[&key],|r|r.get(0)).unwrap();assert_eq!(count,1);
        let fake=Fake{proofs:Mutex::new(vec![]),calls:AtomicUsize::new(0),busy:false};resume_chrome_prepare(&store,&key,&cfg,"original-account",&fake).unwrap();
        let b=saved_budget(&store,&key,&snapshot).unwrap().unwrap();assert_eq!(b["attempts"],1);assert!(b["executionStartedAt"].as_i64().is_some());assert_eq!(b["startedAt"],original["startedAt"]);assert_eq!(store.job(&key).unwrap()["snapshot"],snapshot);
    }
    #[test] fn legacy_wait_migration_rejects_intent_attempt_nonbusy_unknown_request_or_incomplete_cycles(){
        for case in ["intent","attempt","nonbusy","unknown","request","missing"] {
            let (_t,store,key,_cfg)=fixture();let mut old=legacy_busy_budget(&store,&key);let c=store.conn().unwrap();
            match case {"intent"=>{c.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_prepare_intent','{}',?2)",rusqlite::params![key,now()]).unwrap();},"attempt"=>{old["attempts"]=json!(1);c.execute("UPDATE metadata SET value=?1 WHERE key=?2",rusqlite::params![old.to_string(),format!("chrome_prepare_recovery:{key}")]).unwrap();},"nonbusy"=>{c.execute("UPDATE events SET data=json_set(data,'$.error.code','EMPTY_PREFLIGHT') WHERE job_id=?1 AND type='chrome_prepare_error'",[&key]).unwrap();},"unknown"=>{c.execute("UPDATE events SET data=json_set(data,'$.error.submissionUnknown',json('true')) WHERE job_id=?1 AND type='chrome_prepare_error'",[&key]).unwrap();},"request"=>{c.execute("UPDATE events SET data=json_set(data,'$.sameRequestId','other') WHERE job_id=?1 AND type='chrome_prepare_error'",[&key]).unwrap();},_=>{c.execute("DELETE FROM events WHERE job_id=?1 AND type='chrome_prepare_recovery_started'",[&key]).unwrap();}}drop(c);
            let b=saved_budget(&store,&key,&store.job(&key).unwrap()["snapshot"]).unwrap().unwrap();assert_eq!(b["version"],"chrome-prepare-recovery-budget/v1","{case}");assert!(budget_exhausted(&b),"{case}");
        }
    }
    fn disconnected_history(store:&Store,key:&str,cfg:&ProviderConfig)->Value {
        begin(store,key,cfg).unwrap();
        store.event(key,"chrome_prepare_recovery_started",&json!({"requestId":"original-request","firstSubmitAllowed":false})).unwrap();
        store.claim_job(key,"failed","preparing").unwrap();execution_started(store,key,cfg,"non_queue_prepare_outcome").unwrap();
        store.event(key,"chrome_prepare_error",&json!({"error":{"code":"BRIDGE_BROWSER_NOT_READY","submissionUnknown":false,"authRequired":false},"generationCalled":false,"sameRequestId":"original-request"})).unwrap();
        store.claim_job(key,"preparing","recovery_paused").unwrap();
        let raw:String=store.conn().unwrap().query_row("SELECT value FROM metadata WHERE key=?1",[format!("chrome_prepare_recovery:{key}")],|r|r.get(0)).unwrap();serde_json::from_str(&raw).unwrap()
    }
    #[test] fn chrome_disconnected_wait_correction_retains_original_and_cannot_reset_real_execution(){
        let (_t,store,key,cfg)=fixture();let original=disconnected_history(&store,&key,&cfg);let snapshot=store.job(&key).unwrap()["snapshot"].clone();
        store.event(&key,"chrome_prepare_recovery_exhausted",&json!({"version":"chrome-prepare-budget-terminal/v1","generationCalled":false,"budget":original})).unwrap();
        assert!(has_proof(&store,&store.job(&key).unwrap(),&cfg).unwrap(),"proven mistaken exhaustion is recoverable using the original job");
        let corrected=saved_budget(&store,&key,&snapshot).unwrap().unwrap();assert_eq!(corrected["connectionWaitCorrection"]["originalBudget"],original);assert_eq!(corrected["attempts"],0);
        let raw:String=store.conn().unwrap().query_row("SELECT value FROM metadata WHERE key=?1",[format!("chrome_prepare_recovery:{key}")],|r|r.get(0)).unwrap();assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(),original,"inspection has zero writes");
        begin(&store,&key,&cfg).unwrap();begin(&store,&key,&cfg).unwrap();
        let count:i64=store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='chrome_prepare_connection_wait_corrected'",[&key],|r|r.get(0)).unwrap();assert_eq!(count,1);
        store.event(&key,"chrome_prepare_recovery_started",&json!({"requestId":"original-request","firstSubmitAllowed":false})).unwrap();execution_started(&store,&key,&cfg,"fresh_native_idle").unwrap();
        let current=saved_budget(&store,&key,&snapshot).unwrap().unwrap();assert_eq!(current["attempts"],1);assert_eq!(current["startedAt"],original["startedAt"]);assert_eq!(store.job(&key).unwrap()["snapshot"],snapshot);
    }
    #[test] fn chrome_disconnected_correction_refuses_fresh_intent_unknown_nonconnection_and_incomplete_history(){
        for case in ["fresh","intent","unknown","nonconnection","missing","request"] {
            let (_t,store,key,cfg)=fixture();let original=disconnected_history(&store,&key,&cfg);let c=store.conn().unwrap();
            match case {"fresh"=>{c.execute("UPDATE events SET data=json_set(data,'$.reason','fresh_native_idle') WHERE job_id=?1 AND type='chrome_prepare_execution_started'",[&key]).unwrap();},
                "intent"=>{store.event(&key,"chrome_prepare_intent",&json!({})).unwrap();},
                "unknown"=>{c.execute("UPDATE events SET data=json_set(data,'$.error.submissionUnknown',json('true')) WHERE job_id=?1 AND type='chrome_prepare_error'",[&key]).unwrap();},
                "nonconnection"=>{c.execute("UPDATE events SET data=json_set(data,'$.error.code','EMPTY_PREFLIGHT') WHERE job_id=?1 AND type='chrome_prepare_error'",[&key]).unwrap();},
                "missing"=>{c.execute("DELETE FROM events WHERE job_id=?1 AND type='chrome_prepare_error'",[&key]).unwrap();},
                _=>{c.execute("UPDATE events SET data=json_set(data,'$.sameRequestId','other') WHERE job_id=?1 AND type='chrome_prepare_error'",[&key]).unwrap();}}drop(c);
            assert_eq!(saved_budget(&store,&key,&store.job(&key).unwrap()["snapshot"]).unwrap().unwrap(),original,"{case}");
        }
    }
    #[test] fn execution_start_marker_refuses_config_request_or_hash_drift_without_metadata_write(){
        for case in ["config","request","hash"] {
            let (_t,store,key,cfg)=fixture();begin(&store,&key,&cfg).unwrap();let before=saved_budget(&store,&key,&store.job(&key).unwrap()["snapshot"]).unwrap();
            let request=generation_request(&store.job(&key).unwrap()["snapshot"]).unwrap();let mut marker=json!({"version":"chrome-prepare-execution-start/v1","requestId":request.request_id,"requestHash":hash(&json!(request)),"configSha256":hash(&json!(cfg))});
            marker[match case {"config"=>"configSha256","request"=>"requestId",_=>"requestHash"}]=json!("changed");assert!(observer(&store,&key,&cfg)(&marker).is_err());assert_eq!(saved_budget(&store,&key,&store.job(&key).unwrap()["snapshot"]).unwrap(),before);
        }
    }
}
