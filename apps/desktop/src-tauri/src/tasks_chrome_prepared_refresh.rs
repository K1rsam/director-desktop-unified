//! Refresh transport proof for the same prepared, demonstrably unsubmitted job.
use super::*;
const LOCAL_REJECTION:&str="浏览器运行身份在准备后变化，保留原请求，未发送";
fn empty(v:&Value)->bool{v.is_null()||v==&json!({})||v==""}
fn hash(v:&Value)->String{crate::pipeline::digest(&v.to_string())}

pub(crate) fn prepared_runtime_refresh_eligible(store:&Store,job:&Value,cfg:&ProviderConfig)->Result<bool>{
    if cfg.kind!="chrome"||job["provider"]!="chrome"||job["kind"]!="image"||job["status"]!="failed"
        ||job["error"]!=LOCAL_REJECTION||!s(job,"providerTaskId").is_empty()||!empty(&job["receipt"])||!job["result"].is_null()
        ||job["prepared"].is_null()||job["snapshot"]["providerConfig"]!=json!(cfg){return Ok(false);}
    binding::validate_generation_snapshot(store,&job["snapshot"])?;
    let p:PreparedRequest=serde_json::from_value(job["prepared"].clone()).map_err(|e|e.to_string())?;
    if p.provider!="chrome"||json!(p.request)!=json!(generation_request(&job["snapshot"])?){return Err("CHROME_PREPARE_REFRESH_HELD: 原准备请求不一致".into());}
    let c=store.conn()?;let mut q=c.prepare("SELECT type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let events=q.query_map([s(job,"id")],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?
        .collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let mut ready=false;let mut submits=0;let mut pending_submit=false;let mut rejected=false;
    for (kind,raw) in events {
        let value:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
        if kind.contains("query")||kind.contains("download")||kind.contains("reconcile")||kind.contains("submit")
            ||value["generationCalled"]==true||value["submissionUnknown"]==true||value["error"]["submissionUnknown"]==true{return Ok(false);}
        let state=s(&value,"state");
        if ["submission_unknown","submitted","processing","succeeded","downloading","downloaded"].contains(&state){return Ok(false);}
        if kind=="state_claim"&&value["from"]=="preparing"&&state=="ready"{ready=true;}
        if kind=="chrome_prepared_runtime_refreshed"{
            if pending_submit||!rejected||value["version"]!="chrome-prepared-runtime-refresh-adoption/v1"||value["requestId"]!=p.request.request_id
                ||value["submitCalled"]!=false||value["retryBudgetsReset"]!=false{return Ok(false);}
            let previous:PreparedRequest=serde_json::from_value(value["originalPrepared"].clone()).map_err(|e|e.to_string())?;
            if json!(previous.request)!=json!(p.request){return Ok(false);}ready=true;rejected=false;
        }
        if kind=="state_claim"&&value["from"]=="ready"&&state=="submitting"{if !ready||pending_submit{return Ok(false);}submits+=1;pending_submit=true;ready=false;}
        if pending_submit&&kind=="state"{
            if state!="failed"||value["error"]!=LOCAL_REJECTION{return Ok(false);}
            pending_submit=false;rejected=true;
        }
    }
    Ok(submits>0&&!pending_submit&&rejected)
}

pub(crate) fn refresh_prepared_runtime(store:&Store,key:&str,cfg:&ProviderConfig,account:&str,channels:&dyn Channels)->Result<bool>{
    let original=store.job(key)?;
    if s(&original,"accountId")!=account||!prepared_runtime_refresh_eligible(store,&original,cfg)?{return Ok(false);}
    let old:PreparedRequest=serde_json::from_value(original["prepared"].clone()).map_err(|e|e.to_string())?;
    let fresh=match channels.refresh_unsubmitted_prepared(cfg,&old){
        Ok(p)=>p,
        Err(e)=>{
            let waiting=e.code=="BRIDGE_BUSY"&&!e.auth_required&&!e.submission_unknown;
            store.event(key,if waiting{"chrome_prepared_runtime_refresh_wait"}else{"chrome_prepared_runtime_refresh_held"},&json!({"error":e,"requestId":original["snapshot"]["requestId"],"generationCalled":false,"originalPreparedSha256":hash(&original["prepared"])}))?;
            if waiting&&store.job(key)?==original{store.claim_job_fields(key,"failed","failed",&json!({"nextAttemptAt":chrono::Utc::now().timestamp()+5}))?;}
            return Ok(false);
        }
    };
    let old_value=json!(old);let fresh_value=json!(fresh);
    let mut comparable_old=old_value.clone();let mut comparable_fresh=fresh_value.clone();
    for value in [&mut comparable_old,&mut comparable_fresh]{
        value.as_object_mut().ok_or("准备格式不一致")?.remove("preparedAt");
        value["wire"].as_object_mut().ok_or("原准备载荷无效")?.remove("preparedRuntimeRefresh");
        for field in ["backgroundEpoch","serverInstanceId","contentEpoch"]{value["wire"]["prepareRuntime"]["runtime"].as_object_mut().ok_or("原准备运行身份无效")?.remove(field);}
    }
    let old_history=old.wire["preparedRuntimeRefresh"].as_array().cloned().unwrap_or_default();
    let history=fresh.wire["preparedRuntimeRefresh"].as_array().ok_or("缺少准备更新证明")?;
    let proof=history.last().ok_or("缺少准备更新证明")?;
    if comparable_old!=comparable_fresh||history.len()!=old_history.len()+1||history[..old_history.len()]!=old_history
        ||proof["version"]!="chrome-prepared-runtime-refresh/v1"||proof["requestId"]!=old.request.request_id
        ||proof["sourceClientId"]!=old.wire["sourceClientId"]||proof["originalPreparedSha256"]!=hash(&old_value)
        ||proof["originalPrepareRuntime"]!=old.wire["prepareRuntime"]||proof["newRuntime"]!=fresh.wire["prepareRuntime"]["runtime"]{
        return Err("CHROME_PREPARE_REFRESH_HELD: 更新改变原请求或缺少完整证明".into());
    }
    if store.job(key)?!=original{return Err("CHROME_PREPARE_REFRESH_HELD: 原任务在核验中变化".into());}
    binding::validate_generation_snapshot(store,&original["snapshot"])?;
    let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
    let changed=tx.execute("UPDATE jobs SET state='ready',prepared=?1,error='',next_attempt_at=0,updated_at=?2 WHERE id=?3 AND state='failed' AND provider='chrome' AND kind='image' AND snapshot=?4 AND prepared=?5 AND account_id=?6 AND (task_id IS NULL OR task_id='') AND (receipt IS NULL OR receipt IN ('','null','{}')) AND result IS NULL",
        rusqlite::params![fresh_value.to_string(),now(),key,original["snapshot"].to_string(),original["prepared"].to_string(),account]).map_err(|e|e.to_string())?;
    if changed!=1{return Err("CHROME_PREPARE_REFRESH_HELD: 原任务写入冲突".into());}
    tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_prepared_runtime_refreshed',?2,?3)",rusqlite::params![key,
        json!({"version":"chrome-prepared-runtime-refresh-adoption/v1","requestId":old.request.request_id,"originalPrepared":old_value,"refreshedPreparedSha256":hash(&fresh_value),"priorError":original["error"],"retryBudgetsReset":false,"prepareCalled":false,"uploadCalled":false,"submitCalled":false}).to_string(),now()]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e|e.to_string())?;store.dirty()?;Ok(true)
}
