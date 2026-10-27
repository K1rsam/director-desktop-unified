//! Explicit desktop continuation contract; frozen jobs and accepted replies stay immutable.
use crate::pipeline::digest;
use crate::store::{now, parse, s, Result, Store};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

pub(crate) const PROFILE: &str = "foundation-three-lane-trial/v1";
const EVENT: &str = "foundation_parallel_trial_authorized";

fn frozen_identity(snapshot: &Value, current_source: &Value) -> Result<()> {
    if snapshot["runnerVersion"] != "desktop-runner/v1"
        || snapshot["stage"] != "foundation.run"
        || snapshot["foundationMethod"] != "foundation-delivery/v2"
        || !snapshot["input"]["foundationPhase"].is_null()
        || snapshot["input"]["source"]["sha256"] != current_source["sha256"]
        || snapshot["input"]["source"]["version"] != current_source["version"]
        || current_source["sha256"].as_str().is_none()
        || current_source["version"].as_u64().is_none()
    {
        return Err("三路续跑要求原制作基础流程、相同原文版本和完整冻结记录；未修改原任务".into());
    }
    Ok(())
}

fn accepted_prefix(c: &rusqlite::Connection, run: &str) -> Result<Vec<Value>> {
    let mut q = c.prepare("SELECT w.node_key,w.stage,w.request_hash,w.state,w.text_job_id,w.result,j.state,j.snapshot,j.result FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 ORDER BY w.rowid").map_err(|e|e.to_string())?;
    let rows=q.query_map([run],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<String>>(8)?))).map_err(|e|e.to_string())?;
    let mut prefix = vec![];
    for row in rows {
        let (key,stage,hash,state,job,result,job_state,request,raw)=row.map_err(|e|e.to_string())?;
        if stage!="foundation:F2" || state!="accepted" || job_state.as_deref()!=Some("completed") || raw.as_deref().is_none_or(|v|v.is_empty()||v=="null") {
            return Err("三路切换仅接受全部已保存的F2前缀；在途、未知、未接受回包或已开始核对须先按原流程处理".into());
        }
        let request=parse(request.ok_or("缺少原文本冻结请求")?)?;
        let result=parse(result.ok_or("缺少已接受原回包")?)?;
        let input=&request["input"];
        if request["stage"]!="foundation:F2" || input["delivery"]["method"]!="foundation-delivery/v2"
            || input["delivery"]["batchIndex"].as_u64()!=Some(prefix.len() as u64)
            || !input["delivery"]["fragment"].is_null() || input.get("foundationParallelTrial").is_some()
            || input["scope"].as_array().is_none_or(|v|v.is_empty()) {
            return Err("三路切换前缀不是连续完整集批次，原记录保留".into());
        }
        prefix.push(json!({"nodeKey":key,"requestHash":hash,"textJobId":job,"inputSha256":digest(&input.to_string()),"resultSha256":digest(&result.to_string()),"sourceSha256":input["sourceSha256"],"batchIndex":prefix.len(),"scope":input["scope"]}));
    }
    if prefix.is_empty() { return Err("至少先保存一批资产整理，才能冻结共同基线".into()); }
    Ok(prefix)
}

fn make_contract(run: &str, snapshot: &Value, prefix: &[Value]) -> Result<Value> {
    let selection = snapshot["input"]["selection"]["episodeIds"].as_array().ok_or("缺原任务范围")?;
    let covered:Vec<_>=prefix.iter().flat_map(|p|p["scope"].as_array().into_iter().flatten().cloned()).collect();
    if covered.len()>=selection.len() || selection[..covered.len()]!=covered
        || prefix.iter().any(|p|p["sourceSha256"]!=snapshot["input"]["source"]["sha256"]) {
        return Err("已保存批次必须是原选定范围的连续前缀，且仍有未提交集数".into());
    }
    let mut contract=json!({"version":PROFILE,"workflowId":run,"workflowSnapshotSha256":digest(&snapshot.to_string()),
        "sourceSha256":snapshot["input"]["source"]["sha256"],"sourceVersion":snapshot["input"]["source"]["version"],
        "concurrency":3,"replayBatchCount":prefix.len(),"seedBatchCount":1,"historyEpisodeIds":prefix[0]["scope"],
        "acceptedPrefix":prefix,"remainingEpisodeIds":&selection[covered.len()..]});
    contract["authorizationSha256"]=json!(digest(&contract.to_string()));
    Ok(contract)
}

#[cfg(test)]
pub(crate) fn preview_contract(c: &rusqlite::Connection, run: &str, snapshot: &Value) -> Result<Value> {
    make_contract(run,snapshot,&accepted_prefix(c,run)?)
}

fn verify_contract(c: &rusqlite::Connection, run: &str, snapshot: &Value, contract: &Value) -> Result<()> {
    let mut unhashed=contract.clone();
    let hash=unhashed.as_object_mut().ok_or("三路续跑记录无效")?.remove("authorizationSha256").ok_or("三路续跑缺摘要")?;
    if hash!=digest(&unhashed.to_string()) || contract["version"]!=PROFILE || contract["workflowId"]!=run
        || contract["workflowSnapshotSha256"]!=digest(&snapshot.to_string()) || contract["concurrency"]!=3
        || contract["sourceSha256"]!=snapshot["input"]["source"]["sha256"] {
        return Err("三路续跑授权与原冻结任务不一致，未提交".into());
    }
    let prefix=contract["acceptedPrefix"].as_array().filter(|v|!v.is_empty()).ok_or("三路续跑缺原检查点")?;
    if make_contract(run,snapshot,prefix)?!=*contract { return Err("三路续跑范围或共同基线记录不一致".into()); }
    for p in prefix {
        let row:Option<(String,String,String,String,String,String)>=c.query_row(
            "SELECT w.state,w.request_hash,w.text_job_id,w.result,j.snapshot,j.state FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",params![run,s(p,"nodeKey")],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|e.to_string())?;
        let (state,hash,job,result,request,job_state)=row.ok_or("原保存检查点已缺失，未重发")?;
        if state!="accepted"||job_state!="completed"||hash!=s(p,"requestHash")||job!=s(p,"textJobId")
            || digest(&parse(result)?.to_string())!=s(p,"resultSha256")
            || digest(&parse(request)?["input"].to_string())!=s(p,"inputSha256") {
            return Err("原已接受批次发生变化，保留记录并停止三路续跑".into());
        }
    }
    Ok(())
}

/// Called under the normal project pipeline lease, before starting any provider work.
pub(crate) fn authorize(store: &Store, run: &str, requested: &Value) -> Result<()> {
    if requested!=&json!({"version":PROFILE,"concurrency":3}) {return Err("不支持的三路试用设置".into());}
    let mut c=store.conn()?;
    let tx=c.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let (raw,state):(String,String)=tx.query_row("SELECT snapshot,state FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='foundation.run'",[run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if !["failed","interrupted","blocked","partially_blocked"].contains(&state.as_str()) {return Err("请等待当前批次停止并保存后，再启用三路续跑".into());}
    let snapshot=parse(raw)?;
    let source:String=tx.query_row("SELECT data FROM sources ORDER BY version DESC LIMIT 1",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    frozen_identity(&snapshot,&parse(source)?)?;
    let saved:Option<String>=tx.query_row("SELECT data FROM events WHERE job_id=?1 AND type=?2 ORDER BY rowid DESC LIMIT 1",params![run,EVENT],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if let Some(saved)=saved {
        verify_contract(&tx,run,&snapshot,&parse(saved)?)?;
    } else {
        let active:i64=tx.query_row("SELECT COUNT(*) FROM jobs WHERE kind='text' AND state IN ('queued','preparing','prepared','submitting','running','submission_unknown','unknown','received')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if active>0{return Err("项目仍有在途或未知文本请求，请先核对原任务；未启用三路".into());}
        let prefix=accepted_prefix(&tx,run)?;
        let contract=make_contract(run,&snapshot,&prefix)?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![run,EVENT,contract.to_string(),now()]).map_err(|e|e.to_string())?;
    }
    tx.commit().map_err(|e|e.to_string())
}

pub(crate) fn attach(store: &Store, run: &str, snapshot: &Value, context: &mut Value) -> Result<()> {
    let c=store.conn()?;
    let saved:Option<String>=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type=?2 ORDER BY rowid DESC LIMIT 1",params![run,EVENT],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if let Some(saved)=saved {
        frozen_identity(snapshot,&store.source()?)?;
        let contract=parse(saved)?;
        verify_contract(&c,run,snapshot,&contract)?;
        context["foundationParallelTrial"]=contract;
    }
    Ok(())
}

/// Rechecked by the SDK executor, not trusted merely because a request carries a marker.
pub(crate) fn validate_request(store: &Store, run: &str, marker: &Value) -> Result<()> {
    let c=store.conn()?;
    let count:i64=c.query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type=?2",params![run,EVENT],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count!=1 {return Err("三路请求缺少唯一持久授权，未提交".into());}
    let snapshot=store.job(run)?["snapshot"].clone();
    let mut context=json!({});attach(store,run,&snapshot,&mut context)?;
    let contract=context["foundationParallelTrial"].as_object().ok_or("缺三路授权")?;
    if contract.iter().any(|(key,value)| marker.get(key)!=Some(value))
        || marker["laneId"].as_u64().is_none_or(|n|n>=3)
        || marker["packetId"].as_str().is_none_or(|v|v.is_empty()||v.len()>512||v.chars().any(|c|c.is_control()||c=='/'||c=='\\'))
        || marker["basisSha256"].as_str().is_none_or(|v|v.len()!=64||!v.bytes().all(|b|b.is_ascii_hexdigit())) {
        return Err("三路请求与冻结授权、通道或批次身份不一致，未提交".into());
    }
    Ok(())
}

pub(crate) fn receipt_time(store: &Store, run: &str, input: &Value) -> Result<String> {
    validate_request(store,run,&input["foundationParallelTrial"])?;
    let c=store.conn()?;
    let mut query=c.prepare("SELECT j.snapshot,MIN(e.created_at) FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id JOIN events e ON e.job_id=j.id AND e.type='text_received_raw' AND json_extract(e.data,'$.requestHash')=w.request_hash WHERE w.workflow_id=?1 AND w.state='accepted' AND j.state='completed' AND w.stage='foundation:Q1' AND json_extract(j.snapshot,'$.input.foundationParallelTrial.packetId')=?2 GROUP BY j.id,j.snapshot").map_err(|e|e.to_string())?;
    let rows=query.query_map(params![run,s(&input["foundationParallelTrial"],"packetId")],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?;
    let without_transport_retry=|value:&Value| {let mut v=value.clone();if let Some(m)=v.as_object_mut(){m.remove("textTransportRetry");}v};
    let mut found=vec![];
    for row in rows {
        let (snapshot,time)=row.map_err(|e|e.to_string())?;
        if without_transport_retry(&parse(snapshot)?["input"])==without_transport_retry(input) {
            chrono::DateTime::parse_from_rfc3339(&time).map_err(|_|"原回包缺有效时间")?;
            found.push(time);
        }
    }
    if found.len()!=1 {return Err("并行核对缺少唯一原回包时间，已保存结果不重发".into());}
    Ok(found.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Store, String, Value) {
        let dir=tempfile::tempdir().unwrap();
        let store=Store::open(dir.path().into()).unwrap();store.create("三路恢复测试","test").unwrap();
        store.ensure_workflow_storage().unwrap();
        let source=crate::pipeline::prepare_source("第1集\n场景：门口\n小王：开始。\n第2集\n场景：门口\n小王：继续。\n第3集\n场景：门口\n小王：结束。\n").unwrap();
        store.save_source(serde_json::to_value(source).unwrap()).unwrap();
        let source=store.source().unwrap();
        let ids:Vec<_>=source["episodes"].as_array().unwrap().iter().map(|e|e["id"].clone()).collect();
        let snapshot=json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run","foundationMethod":"foundation-delivery/v2","input":{"source":source,"selection":{"episodeIds":ids}}});
        let run=store.add_job("workflow","foundation.run","local","local",&snapshot,"failed").unwrap();
        let request=json!({"stage":"foundation:F2","input":{"sourceSha256":source["sha256"],"scope":[ids[0]],"delivery":{"method":"foundation-delivery/v2","batchIndex":0,"fragment":null}}});
        let raw=json!({"original":"accepted result retained"});
        let text=store.add_job("text","foundation:F2","codex_sdk","account",&request,"completed").unwrap();
        store.update_job(&text,"completed",&json!({"result":raw,"receipt":{"original":true}})).unwrap();
        store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,result,updated_at) VALUES(?1,'f2:0','foundation:F2','request-hash','accepted',?2,?3,?4)",params![run,text,raw.to_string(),now()]).unwrap();
        (dir,store,run,snapshot)
    }
    fn request()->Value {json!({"version":PROFILE,"concurrency":3})}
    fn frozen_records(store:&Store)->Vec<(String,String,Option<String>,Option<String>)> {
        let c=store.conn().unwrap();let mut q=c.prepare("SELECT id,snapshot,result,receipt FROM jobs ORDER BY id").unwrap();
        q.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(|r|r.unwrap()).collect()
    }
    #[test]
    fn explicit_trial_is_idempotent_keeps_original_bytes_and_rehydrates_after_reopen() {
        let (_dir,store,run,snapshot)=fixture();let old=frozen_records(&store);
        let mut legacy=json!({});attach(&store,&run,&snapshot,&mut legacy).unwrap();assert!(legacy.get("foundationParallelTrial").is_none());
        authorize(&store,&run,&request()).unwrap();authorize(&store,&run,&request()).unwrap();
        assert_eq!(old,frozen_records(&store));
        let reopened=Store::open(store.root.clone()).unwrap();let mut context=json!({});attach(&reopened,&run,&snapshot,&mut context).unwrap();
        let trial=&context["foundationParallelTrial"];
        assert_eq!(trial["replayBatchCount"],1);assert_eq!(trial["concurrency"],3);
        assert_eq!(trial["historyEpisodeIds"].as_array().unwrap().len(),1);
        assert_eq!(trial["remainingEpisodeIds"].as_array().unwrap().len(),2);
        assert_eq!(store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE type=?1",[EVENT],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    #[test]
    fn trial_never_changes_active_or_unknown_requests_to_start_parallel() {
        for state in ["submitting","submission_unknown","received"] {
            let (_dir,store,run,_)=fixture();
            store.add_job("text","pending","codex_sdk","account",&json!({}),state).unwrap();
            assert!(authorize(&store,&run,&request()).unwrap_err().contains("在途或未知"));
            assert_eq!(store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE type=?1",[EVENT],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
    }
    #[test]
    fn changed_checkpoint_and_tampered_receipt_fail_closed() {
        let (_dir,store,run,snapshot)=fixture();authorize(&store,&run,&request()).unwrap();
        store.conn().unwrap().execute("UPDATE workflow_steps SET result='{}' WHERE workflow_id=?1",[&run]).unwrap();
        assert!(attach(&store,&run,&snapshot,&mut json!({})).unwrap_err().contains("发生变化"));
        let (_dir,store,run,snapshot)=fixture();authorize(&store,&run,&request()).unwrap();
        store.conn().unwrap().execute("UPDATE events SET data=json_set(data,'$.concurrency',9) WHERE type=?1",[EVENT]).unwrap();
        assert!(attach(&store,&run,&snapshot,&mut json!({})).is_err());
    }
    #[test]
    fn source_drift_and_nonprefix_or_fragmented_checkpoints_cannot_enable_trial() {
        let (_dir,store,run,snapshot)=fixture();authorize(&store,&run,&request()).unwrap();
        let changed=crate::pipeline::prepare_source("第1集\n场景：新地点\n小王：新内容。\n").unwrap();
        store.save_source(serde_json::to_value(changed).unwrap()).unwrap();
        assert!(attach(&store,&run,&snapshot,&mut json!({})).is_err());
        for path in ["$.input.delivery.batchIndex","$.input.delivery.fragment"] {
            let (_dir,store,run,_)=fixture();
            store.conn().unwrap().execute("UPDATE jobs SET snapshot=json_set(snapshot,?1,2) WHERE kind='text'",[path]).unwrap();
            assert!(authorize(&store,&run,&request()).unwrap_err().contains("连续完整"));
        }
    }
    #[test]
    fn receipt_time_uses_bound_original_response_and_progress_separates_finalization() {
        let (_dir,store,run,snapshot)=fixture();authorize(&store,&run,&request()).unwrap();
        let mut context=json!({});attach(&store,&run,&snapshot,&mut context).unwrap();
        let mut marker=context["foundationParallelTrial"].clone();
        marker["laneId"]=json!(0);marker["packetId"]=json!("scope:Q1:0");marker["basisSha256"]=json!("a".repeat(64));
        let input=json!({"foundationParallelTrial":marker,"foundationParallelFinal":{"version":"foundation-parallel-final/v1"},"scope":snapshot["input"]["selection"]["episodeIds"],"delivery":{"method":"foundation-delivery/v2","batchIndex":0}});
        let text=store.add_job("text","foundation:Q1","codex_sdk","account",&json!({"stage":"foundation:Q1","input":input}),"completed").unwrap();
        store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,result,updated_at) VALUES(?1,'q1:final','foundation:Q1','qhash','accepted',?2,'{}',?3)",params![run,text,now()]).unwrap();
        for (time,hash) in [("2026-10-27T00:00:00+00:00","unrelated"),("2026-10-27T03:00:00+00:00","qhash"),("2026-10-27T04:00:00+00:00","qhash")] {
            store.conn().unwrap().execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'text_received_raw',?2,?3)",params![text,json!({"requestHash":hash,"value":{}}).to_string(),time]).unwrap();
        }
        assert_eq!(receipt_time(&store,&run,&input).unwrap(),"2026-10-27T03:00:00+00:00");
        let mut drift=input.clone();drift["unexpected"]=json!(true);assert!(receipt_time(&store,&run,&drift).is_err());
        let progress=store.workflow_progress().unwrap();
        let node=progress.iter().flat_map(|p|p["nodes"].as_array().into_iter().flatten()).find(|n|n["id"]=="q1:final").unwrap();
        assert_eq!(node["input"]["parallelTrial"]["kind"],"final");
        assert_eq!(node["input"]["parallelTrial"]["laneId"],0);
        assert!(node["input"].get("foundationParallelTrial").is_none(),"do not expose full authorization or source to the progress UI");
    }
}
