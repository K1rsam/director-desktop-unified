//! A rejected planner can authorize only its exact, already bounded repair.
//! This is separate from the general accepted-predecessor handoff policy.
use super::*;

struct RejectedPlan {
    job: String,
    key: String,
    request_hash: String,
    snapshot: String,
    prepared: String,
    raw: String,
    receipt: String,
    error: String,
    request: TextRequest,
}

fn evidence(store: &Store, c: &rusqlite::Connection, executor: &SavedTextExecutor, node: &RejectedPlan) -> Result<Value> {
    let snapshot: Value=serde_json::from_str(&node.snapshot).map_err(|_|HELD)?;
    if hash(&snapshot)!=node.request_hash || snapshot.to_string()!=node.snapshot
        || step_key(&node.request)!=node.key || !planner_repairable_rejection(&node.error) {
        return Err(HELD.into());
    }
    let job=executor.sdk_job(&node.job)?;
    let raw=executor.sdk_raw(&job,&snapshot)?;
    if raw.to_string()!=node.raw || job["status"]!="rejected" || job["error"]!=node.error
        || job["prepared"]["sdkOriginWorkflowId"]!=executor.workflow_id.as_deref().unwrap_or("") {
        return Err(HELD.into());
    }
    let dir=store.root.join("text-runs").join(&node.job);
    let attempt=FrozenAttempt::load(&dir.join("sdk-attempt.json"))?;
    if attempt.state!=crate::text_transport::AttemptState::Completed || attempt.backend!=crate::text_transport::Backend::CodexSdk
        || attempt.workflow_id!=executor.workflow_id.as_deref().unwrap_or("") || attempt.logical_node_id!=node.key
        || serde_json::to_value(&attempt.request).map_err(|_|HELD)?!=serde_json::to_value(&node.request).map_err(|_|HELD)? {
        return Err(HELD.into());
    }
    let (event_id,event_data):(i64,String)=c.query_row(
        "SELECT id,data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",
        [&node.job],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|HELD)?;
    Ok(json!({"jobId":node.job,"nodeKey":node.key,"stage":node.request.stage,"requestHash":node.request_hash,
        "snapshotSha256":crate::pipeline::digest(&node.snapshot),"preparedSha256":crate::pipeline::digest(&node.prepared),
        "resultSha256":crate::pipeline::digest(&node.raw),"receiptSha256":crate::pipeline::digest(&node.receipt),
        "rejectedResultSha256":hash(&raw),"rejectionErrorSha256":crate::pipeline::digest(&node.error),
        "attemptFileSha256":crate::store::hash_file(&dir.join("sdk-attempt.json"))?.0,
        "compiledFileSha256":crate::store::hash_file(&dir.join("sdk-compiled.json"))?.0,
        "rawFileSha256":crate::store::hash_file(&dir.join("result.json"))?.0,
        "rawEventId":event_id,"rawEventDataSha256":crate::pipeline::digest(&event_data)}))
}

pub(super) fn predecessor(store: &Store,c: &rusqlite::Connection,run: &str,snapshot: &Value)->Result<Value> {
    let current:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
    if current.stage!="storyboardPlan" || current.skill!="episode-emotion-planner"
        || current.input["storyboardCreativeProfile"]!="original-production-contract/v1"
        || current.input["planRepair"]["version"]!="rejected-plan-repair/v1" {
        return Err(HELD.into());
    }
    let parent:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|r.get(0)).map_err(|_|HELD)?;
    let parent:Value=serde_json::from_str(&parent).map_err(|_|HELD)?;
    let source:crate::pipeline::LockedSource=serde_json::from_value(parent["input"]["source"].clone()).map_err(|_|HELD)?;
    let settings=parent["input"]["settings"].clone();
    if settings["textProvider"]!="codex_sdk" || settings["textModel"]!=current.model || settings["reasoning"]!=current.reasoning {
        return Err(HELD.into());
    }
    let mut q=c.prepare("SELECT j.id,w.node_key,w.request_hash,j.snapshot,j.prepared,j.result,j.receipt,j.error FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage='storyboardPlan' AND w.state='rejected' AND j.kind='text' AND j.provider='codex_sdk' AND j.state='rejected' AND j.error=w.error AND w.result=j.result AND json_extract(j.receipt,'$.validation')='rejected' AND json_extract(j.receipt,'$.contract')='desktop-runner/v1' AND json_extract(j.receipt,'$.migrationPolicy') IS NULL AND json_extract(j.snapshot,'$.input.episode.id')=?2").map_err(|_|HELD)?;
    let records=q.query_map(params![run,s(&current.input["episode"],"id")],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?))).map_err(|_|HELD)?;
    let mut nodes=Vec::new();
    for row in records {
        let (job,key,request_hash,snapshot,prepared,raw,receipt,error)=row.map_err(|_|HELD)?;
        let request:TextRequest=serde_json::from_str(&snapshot).map_err(|_|HELD)?;
        nodes.push(RejectedPlan{job,key,request_hash,snapshot,prepared,raw,receipt,error,request});
        if nodes.len()>3 {return Err(HELD.into());}
    }
    let matching=|expected:&Value|->Result<&RejectedPlan>{
        let matches:Vec<_>=nodes.iter().filter(|n|serde_json::to_string(&n.request).ok().is_some_and(|text|crate::pipeline::digest(&text)==*expected)).collect();
        if matches.len()!=1 {return Err(HELD.into());}Ok(matches[0])
    };
    let proof=&current.input["planRepair"];
    let origin=matching(&proof["originDomainRequestSha256"])?;
    let previous=matching(&proof["previousRequestSha256"])?;
    let origin_raw:Value=serde_json::from_str(&origin.raw).map_err(|_|HELD)?;
    let previous_raw:Value=serde_json::from_str(&previous.raw).map_err(|_|HELD)?;
    crate::pipeline::validate_planner_repair_request_bound(&source,&origin.request,&origin_raw,&origin.error,&previous.request,&previous_raw,&previous.error,&current)?;
    let executor=SavedTextExecutor{store:store.clone(),settings,workflow_id:Some(run.to_owned())};
    let origin_evidence=evidence(store,c,&executor,origin)?;
    let mut prior=evidence(store,c,&executor,previous)?;
    prior["originalEvidenceSha256"]=json!(hash(&origin_evidence));
    Ok(prior)
}
