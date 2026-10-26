//! Durable controller findings are an append-only overlay on an owned workflow.
//! No model dispatch, adopted output, old snapshot, raw, or attempt is rewritten.
use super::*;
use crate::pipeline::series::controller_review::{self as domain, FIELD, HELD};
const EVENT:&str="series_controller_review_supplement_frozen";
const CONTINUE:&str="seriesControllerReviewContinuation";
const CONTINUE_EVENT:&str="series_controller_review_continue_requested";

fn continuation_proof(c:&rusqlite::Connection,run:&str,context:&Value)->Result<Value>{
    let read=|prefix:&str|->Result<Value>{let text:String=c.query_row("SELECT value FROM metadata WHERE key=?1",
        [format!("{prefix}:{}",s(context,"episodeId"))],|r|r.get(0)).map_err(|_|HELD)?;
        serde_json::from_str(&text).map_err(|_|HELD.into())};
    let delivery=read("series.delivery")?;let input=read("series.input")?;let publication=read("series.publication")?;
    crate::pipeline::series::validate_delivery(&input,&delivery)?;
    if delivery["adoptionReceipt"][FIELD]!=*context || publication["workflowId"]!=run
        || publication["bundleSha256"]!=delivery["adoptionReceipt"]["bundleSha256"]{return Err(HELD.into());}
    Ok(json!({"version":"series-controller-review-continuation/v1","workflowId":run,
        "episodeId":context["episodeId"],"contextSha256":context["sha256"],
        "parentSnapshotSha256":context["parentSnapshotSha256"],"sourceSha256":context["sourceSha256"],
        "deliverySha256":crate::pipeline::digest(&delivery.to_string()),"bundleSha256":publication["bundleSha256"],
        "generationRequestsSent":0}))
}

impl SavedTextExecutor {
    pub(super) fn series_controller_stop_required(&self)->Result<bool>{
        let Some(context)=self.saved_series_controller_review()? else{return Ok(true)};
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let(_,_,_,prepared,_)=self.controller_review_parent()?;
        let Some(proof)=prepared.get(CONTINUE) else{return Ok(true)};
        let c=self.store.conn()?;
        if *proof!=continuation_proof(&c,run,&context)?{return Err(HELD.into());}
        let mut q=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2 ORDER BY id").map_err(|e|e.to_string())?;
        let events=q.query_map(params![run,CONTINUE_EVENT],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        if events.len()!=1 || events[0]!=proof.to_string(){return Err(HELD.into());}Ok(false)
    }
    /// Only a later explicit resume releases a successfully adopted one-episode
    /// stop. The first invocation never calls this after the worker finishes.
    pub(crate) fn continue_completed_controller_review(&self)->Result<()> {
        let Some(context)=self.saved_series_controller_review()? else{return Ok(())};
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let(text,_,old_prepared,mut prepared,state)=self.controller_review_parent()?;
        if prepared.get(CONTINUE).is_some(){self.series_controller_stop_required()?;return Ok(());}
        let c=self.store.conn()?;let output:Option<String>=c.query_row("SELECT result FROM jobs WHERE id=?1",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
        let output:Value=output.as_deref().map(serde_json::from_str).transpose().map_err(|_|HELD)?.unwrap_or(Value::Null);
        if !output["blocked"].as_array().into_iter().flatten().any(|b|b["status"]=="waiting_controller" && b["dependsOn"]==context["episodeId"]){return Ok(());}
        if state!="partially_blocked"{return Err(HELD.into());}
        let mut c=self.store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let proof=continuation_proof(&tx,run,&context)?;
        let unknown:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND state NOT IN ('accepted','rejected'))",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
        if unknown{return Err(HELD.into());}
        prepared[CONTINUE]=proof.clone();let timestamp=now();
        let changed=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND snapshot=?4 AND prepared IS ?5 AND state=?6 AND result IS ?7",
            params![prepared.to_string(),timestamp,run,text,old_prepared,state,output.to_string()]).map_err(|e|e.to_string())?;
        if changed!=1{return Err(HELD.into());}
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![run,CONTINUE_EVENT,proof.to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())?;Ok(())
    }
    fn controller_review_origin(&self, job_id:&str)->Result<(TextRequest,Value,Value,String)> {
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let c=self.store.conn()?;
        let row:(String,String,String,String,String,String,String,String,String)=c.query_row(
            "SELECT j.snapshot,j.result,j.receipt,j.state,j.provider,w.node_key,w.request_hash,w.state,w.result FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE j.id=?1 AND j.kind='text' AND w.workflow_id=?2 AND w.stage='seriesReview'",
            params![job_id,run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(|_|HELD)?;
        let(snapshot_text,raw_text,receipt_text,state,provider,node,sha,checkpoint,result)=row;
        let snapshot:Value=serde_json::from_str(&snapshot_text).map_err(|_|HELD)?;
        let raw:Value=serde_json::from_str(&raw_text).map_err(|_|HELD)?;
        let receipt:Value=serde_json::from_str(&receipt_text).map_err(|_|HELD)?;
        let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
        if state!="completed" || checkpoint!="accepted" || provider!=s(&self.settings,"textProvider")
            || snapshot.to_string()!=snapshot_text || sha!=crate::pipeline::digest(&snapshot_text)
            || result!=raw_text || node!=step_key(&request)
            || receipt["contract"]!="desktop-runner/v1" || receipt["validation"]!="accepted"
            || receipt.get("acceptedRequestHash").is_some_and(|v|v!=&json!(sha)) {
            return Err(HELD.into());
        }
        let event:String=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",[job_id],|r|r.get(0)).map_err(|_|HELD)?;
        let event:Value=serde_json::from_str(&event).map_err(|_|HELD)?;
        if event["requestHash"]!=sha || event["value"]!=raw{return Err(HELD.into());}
        let verified=if provider=="codex_sdk" {self.sdk_raw(&self.sdk_job(job_id)?,&snapshot)?}
            else {self.exact_workflow_v1_raw(&request,&snapshot,&sha)?.ok_or(HELD)?};
        if verified!=raw{return Err(HELD.into());}
        Ok((request,raw,snapshot,node))
    }
    fn controller_review_parent(&self)->Result<(String,Value,Option<String>,Value,String)> {
        let c=self.store.conn()?;let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let(text,prepared,state):(String,Option<String>,String)=c.query_row(
            "SELECT snapshot,prepared,state FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",
            [run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|HELD)?;
        let parent:Value=serde_json::from_str(&text).map_err(|_|HELD)?;
        let saved=prepared.as_deref().map(serde_json::from_str).transpose().map_err(|_|HELD)?.unwrap_or(json!({}));
        if parent["runnerVersion"]!="desktop-runner/v1" || parent["input"]["workflowProfile"]!=crate::asset_graph::PROFILE
            || parent["input"]["settings"]!=self.settings || !saved.is_object(){return Err(HELD.into());}
        Ok((text,parent,prepared,saved,state))
    }
    pub(super) fn saved_series_controller_review(&self)->Result<Option<Value>> {
        // All normal/new workflows remain unchanged; this path has no writes.
        let run=match self.workflow_id.as_deref(){Some(run)=>run,None=>return Ok(None)};
        let present:bool=self.store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND json_type(prepared,'$.seriesControllerReviewSupplement') IS NOT NULL)",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !present{return Ok(None);}
        let(_,parent,_,prepared,_)=self.controller_review_parent()?;
        let Some(context)=prepared.get(FIELD) else{return Ok(None)};
        let(req,raw,snapshot,node)=self.controller_review_origin(s(context,"originReviewJobId"))?;
        if context["workflowId"]!=run || context["parentSnapshotSha256"]!=crate::pipeline::digest(&parent.to_string())
            || context["originSnapshotSha256"]!=crate::pipeline::digest(&snapshot.to_string())
            || context["originReviewNodeKey"]!=node {return Err(HELD.into());}
        domain::validate_context(context,&req,&raw,&parent["input"])?;
        let c=self.store.conn()?;let mut q=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2 ORDER BY id").map_err(|e|e.to_string())?;
        let events=q.query_map(params![run,EVENT],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        if events.len()!=1{return Err(HELD.into());}
        let event:Value=serde_json::from_str(&events[0]).map_err(|_|HELD)?;
        if event["context"]!=*context || event["contextSha256"]!=context["sha256"]
            || event["workflowId"]!=run || event["generationRequestsSent"]!=0{return Err(HELD.into());}
        Ok(Some(context.clone()))
    }
    fn controller_next_slot_absent(&self,c:&rusqlite::Connection,context:&Value)->Result<()> {
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let iteration=context["firstReviewIteration"].as_u64().ok_or(HELD)?+1;
        let role=format!("{}/repair/review-{iteration}",s(context,"episodeId"));
        let review_role=format!("{}/reviewer/{iteration}",s(context,"episodeId"));
        let occupied:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND json_extract(snapshot,'$.input.sourceSha256')=?1 AND (json_extract(snapshot,'$.input.executionRoleId')=?2 OR json_extract(snapshot,'$.input.executionRoleId') LIKE ?3 OR json_extract(snapshot,'$.input.executionRoleId')=?4)) OR EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?5 AND state NOT IN ('accepted','rejected')) OR EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND state IN ('submission_unknown','running','dispatching','queued','observed','cancel_requested'))",
            params![context["sourceSha256"].as_str(),role,format!("{role}/%"),review_role,run],|r|r.get(0)).map_err(|e|e.to_string())?;
        if occupied{return Err(HELD.into());}
        check_cli_unknown_hold(&self.store)?;
        let root=self.store.root.join("text-runs");
        if root.exists(){for (i,entry) in std::fs::read_dir(root).map_err(|e|e.to_string())?.enumerate(){
            if i>=20_000{return Err(HELD.into());}
            let path=entry.map_err(|e|e.to_string())?.path().join("sdk-attempt.json");
            if !path.exists(){continue;}
            let attempt=crate::text_transport::FrozenAttempt::load(&path).map_err(|_|HELD)?;
            if attempt.workflow_id==run && (s(&attempt.request.input,"executionRoleId")==role
                || s(&attempt.request.input,"executionRoleId").starts_with(&format!("{role}/"))
                || s(&attempt.request.input,"executionRoleId")==review_role
                || matches!(attempt.state,crate::text_transport::AttemptState::Dispatching|crate::text_transport::AttemptState::Observed|crate::text_transport::AttemptState::SubmissionUnknown|crate::text_transport::AttemptState::CancelRequested)) {
                return Err(HELD.into());
            }
        }}Ok(())
    }
    pub(crate) fn authorize_series_controller_review(&self,auth:&Value)->Result<Value> {
        let _cache=sdk_evidence_cache::scope();
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let(text,parent,prepared_text,mut prepared,state)=self.controller_review_parent()?;
        if !["partially_blocked","blocked","failed","interrupted"].contains(&state.as_str()){return Err(HELD.into());}
        let(req,raw,snapshot,node)=self.controller_review_origin(s(auth,"reviewJobId"))?;
        let context=domain::build_context(auth,&req,&raw,&parent["input"],run,&crate::pipeline::digest(&text),
            &crate::pipeline::digest(&snapshot.to_string()),s(auth,"reviewJobId"),&node)?;
        if let Some(existing)=self.saved_series_controller_review()? {
            if existing!=context{return Err(HELD.into());}
            return Ok(json!({"contextSha256":context["sha256"],"reused":true,"generationRequestsSent":0}));
        }
        let current=self.store.source()?;
        if current["sha256"]!=parent["input"]["source"]["sha256"] || current["version"]!=parent["input"]["source"]["version"] {
            return Err(HELD.into());
        }
        // A controller supplement is only for an unadopted candidate.
        if self.store.series_deliveries()?.iter().any(|d|d["episodeId"]==context["episodeId"]){return Err(HELD.into());}
        let mut c=self.store.conn()?;let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        self.controller_next_slot_absent(&tx,&context)?;
        let prior_events:i64=tx.query_row("SELECT count(*) FROM events WHERE job_id=?1 AND type=?2",params![run,EVENT],|r|r.get(0)).map_err(|e|e.to_string())?;
        if prior_events!=0{return Err(HELD.into());}
        // Re-prove immutable evidence under the publication lock, then CAS.
        let(again,again_raw,again_snapshot,again_node)=self.controller_review_origin(s(auth,"reviewJobId"))?;
        if serde_json::to_value(again).map_err(|e|e.to_string())?!=serde_json::to_value(&req).map_err(|e|e.to_string())?
            || again_raw!=raw || again_snapshot!=snapshot || again_node!=node{return Err(HELD.into());}
        prepared[FIELD]=context.clone();let timestamp=now();
        let changed=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND snapshot=?4 AND prepared IS ?5 AND state=?6",
            params![prepared.to_string(),timestamp,run,text,prepared_text,state]).map_err(|e|e.to_string())?;
        if changed!=1{return Err(HELD.into());}
        let event=json!({"version":"series-controller-review-freeze/v1","workflowId":run,"context":context,
            "contextSha256":context["sha256"],"generationRequestsSent":0});
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![run,EVENT,event.to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())?;
        Ok(json!({"contextSha256":context["sha256"],"reused":false,"generationRequestsSent":0}))
    }
}

#[cfg(test)]
#[path="text_executor_series_controller_review_tests.rs"]
mod tests;
