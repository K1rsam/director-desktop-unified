//! Accept only a proven previous-entry review scope false rejection. The
//! original review remains revise; its raw, frozen request and budget are intact.
use super::*;

const REPLAY: &str = "sdk-series-review-entry-scope-revalidation/v1";
const REPLAY_EVENT: &str = "sdk_series_review_entry_revalidated";
const REJECTION: &str = "SERIES_REVIEW_ISSUE_OUTSIDE_SCOPE";
const REPLAY_HELD: &str =
    "SDK_SERIES_REVIEW_ENTRY_REVALIDATION_HELD: 原独审的冻结入口证据不完整；保留原拒绝，不重发";

fn eligible(request: &TextRequest) -> bool {
    let episode = s(&request.input, "episodeId");
    request.stage == "seriesReview"
        && request.skill == "series-reviewer"
        && request.input["role"] == "reviewer"
        && request.input["freshContext"] == true
        && request.input["workflowProfile"] == crate::pipeline::series::PROFILE
        && !episode.is_empty()
        && (0..=2).any(|n| request.input["executionRoleId"] == format!("{episode}/reviewer/{n}"))
        && request.input.get("validationRepair").is_none()
        && request.input.get("seriesAssetSupplement").is_none()
}

fn record(c: &rusqlite::Connection, id: &str) -> Result<Value> {
    c.query_row("SELECT snapshot,result,prepared,receipt,state,error FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk'",[id],|r|
        Ok(json!({"id":id,"snapshotJson":r.get::<_,String>(0)?,"resultJson":r.get::<_,String>(1)?,
            "preparedJson":r.get::<_,String>(2)?,"receiptJson":r.get::<_,String>(3)?,"state":r.get::<_,String>(4)?,"error":r.get::<_,Option<String>>(5)?})))
        .map_err(|_|REPLAY_HELD.into())
}
fn checkpoint(c: &rusqlite::Connection, run: &str, key: &str) -> Result<Value> {
    c.query_row("SELECT stage,request_hash,state,text_job_id,result,error FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2",params![run,key],|r|
        Ok(json!({"workflowId":run,"nodeKey":key,"stage":r.get::<_,String>(0)?,"requestHash":r.get::<_,String>(1)?,
            "state":r.get::<_,String>(2)?,"jobId":r.get::<_,String>(3)?,"resultJson":r.get::<_,String>(4)?,"error":r.get::<_,Option<String>>(5)?})))
        .map_err(|_|REPLAY_HELD.into())
}
fn files(root: &Path, id: &str) -> Result<Value> {
    let mut out = Vec::new();
    for (name, limit) in [
        ("sdk-attempt.json", 4_194_304),
        ("sdk-compiled.json", 2_097_152),
        ("result.json", 1_048_576),
    ] {
        let (sha, bytes) = crate::store::hash_file(&root.join("text-runs").join(id).join(name))?;
        if bytes > limit {
            return Err(REPLAY_HELD.into());
        }
        out.push(json!({"file":name,"sha256":sha,"bytes":bytes}));
    }
    Ok(json!(out))
}
fn parse(value: &Value, key: &str) -> Result<Value> {
    serde_json::from_str(s(value, key)).map_err(|_| REPLAY_HELD.into())
}
fn receipt(audit: &Value) -> Value {
    json!({"contract":"desktop-runner/v1","validation":"accepted","migrationPolicy":REPLAY,
        "migrationAuditSha256":hash(audit),"acceptedRequestHash":audit["requestHash"],"validatedResultSha256":audit["rawSha256"]})
}
fn events_preserved(original: &Value, current: &Value) -> bool {
    let (Some(old), Some(now)) = (original.as_array(), current.as_array()) else {
        return false;
    };
    let Some(last) = old.last().and_then(|e| e["id"].as_i64()) else {
        return false;
    };
    now.iter()
        .filter(|e| e["id"].as_i64().is_some_and(|id| id <= last))
        .cloned()
        .collect::<Vec<_>>()
        == *old
        && ["text_received_raw", "sdk_raw_result"].iter().all(|kind| {
            old.iter()
                .rfind(|e| e["type"] == *kind)
                .is_some_and(|event| now.iter().rfind(|e| e["type"] == *kind) == Some(event))
        })
}

impl SavedTextExecutor {
    fn sdk_review_entry_revalidation(&self, request: &TextRequest, raw: &Value) -> Result<Value> {
        if !eligible(request) {
            return Err(REPLAY_HELD.into());
        }
        self.sdk_parent_binding()?;
        self.sdk_request_policy(request)?;
        let run = self.workflow_id.as_deref().ok_or(REPLAY_HELD)?;
        let parent = self.store.job(run)?;
        if parent["entityId"] != "storyboard.run"
            || parent["snapshot"]["runnerVersion"] != "desktop-runner/v1"
        {
            return Err(REPLAY_HELD.into());
        }
        crate::pipeline::series::revalidate_review_entry_scope_response(
            request,
            raw,
            &parent["snapshot"]["input"],
        )
    }

    pub(in crate::text_executor) fn sdk_review_entry_replay_commit(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        job: &Value,
        value: &Value,
        reason: Option<&str>,
    ) -> Result<bool> {
        if !eligible(request)
            || job["status"] != "rejected"
            || job["error"] != REJECTION
            || reason.is_some()
        {
            return Ok(false);
        }
        if value != &job["result"]
            || snapshot != &job["snapshot"]
            || self.sdk_schema_owned_raw(job)? != *value
        {
            return Err(REPLAY_HELD.into());
        }
        let domain_proof = self.sdk_review_entry_revalidation(request, value)?;
        let run = self.workflow_id.as_deref().ok_or(REPLAY_HELD)?;
        let id = s(job, "id");
        let key = step_key(request);
        let mut c = self.store.conn()?;
        let old = record(&c, id)?;
        let step = checkpoint(&c, run, &key)?;
        let raw_events = events(&c, id)?;
        if old["receiptJson"]
            != json!({"contract":"desktop-runner/v1","validation":"rejected"}).to_string()
            || old["state"] != "rejected"
            || old["error"] != REJECTION
            || old["snapshotJson"] != snapshot.to_string()
            || old["resultJson"] != value.to_string()
            || step
                != json!({"workflowId":run,"nodeKey":key,"stage":request.stage,"requestHash":hash(snapshot),"state":"rejected","jobId":id,"resultJson":value.to_string(),"error":REJECTION})
            || raw_events
                .as_array()
                .is_none_or(|rows| rows.iter().any(|e| e["type"] == REPLAY_EVENT))
        {
            return Err(REPLAY_HELD.into());
        }
        let attempt = FrozenAttempt::load(
            &self
                .store
                .root
                .join("text-runs")
                .join(id)
                .join("sdk-attempt.json"),
        )?;
        let sdk_event = raw_events
            .as_array()
            .unwrap()
            .iter()
            .rfind(|e| e["type"] == "sdk_raw_result")
            .ok_or(REPLAY_HELD)?;
        if attempt.raw_result.as_ref() != Some(&sdk_event["data"]) {
            return Err(REPLAY_HELD.into());
        }
        let parent_json:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|r.get(0)).map_err(|_|REPLAY_HELD)?;
        let audit = json!({"version":REPLAY,"workflowId":run,"nodeKey":key,"requestHash":hash(snapshot),"rawSha256":hash(value),
            "originalJob":old,"originalCheckpoint":step,"originalEvents":raw_events,"originalFiles":files(&self.store.root,id)?,
            "workflowSnapshotJson":parent_json,"domainProof":domain_proof,"generationRequestsSent":0});
        let mut prepared = parse(&old, "preparedJson")?;
        if prepared.get("sdkDomainResult").is_some()
            || prepared.get("sdkDomainResultSha256").is_some()
        {
            return Err(REPLAY_HELD.into());
        }
        prepared["sdkDomainResult"] = value.clone();
        prepared["sdkDomainResultSha256"] = json!(hash(value));
        let accepted = receipt(&audit);
        let stamp = now();
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if record(&tx, id)? != old
            || checkpoint(&tx, run, &key)? != step
            || events(&tx, id)? != raw_events
            || files(&self.store.root, id)? != audit["originalFiles"]
            || parent(&tx, run, &self.settings)? != crate::pipeline::digest(&parent_json)
        {
            return Err(REPLAY_HELD.into());
        }
        self.sdk_review_entry_revalidation(request, value)?;
        let changed=tx.execute("UPDATE jobs SET state='completed',error=NULL,receipt=?1,prepared=?2,updated_at=?3 WHERE id=?4 AND state='rejected' AND error=?5 AND snapshot=?6 AND result=?7 AND prepared=?8 AND receipt=?9",
            params![accepted.to_string(),prepared.to_string(),stamp,id,REJECTION,s(&old,"snapshotJson"),s(&old,"resultJson"),s(&old,"preparedJson"),s(&old,"receiptJson")]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err(REPLAY_HELD.into());
        }
        let changed=tx.execute("UPDATE workflow_steps SET state='accepted',error=NULL,updated_at=?1 WHERE workflow_id=?2 AND node_key=?3 AND stage='seriesReview' AND text_job_id=?4 AND request_hash=?5 AND state='rejected' AND result=?6 AND error=?7",
            params![stamp,run,key,id,hash(snapshot),value.to_string(),REJECTION]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err(REPLAY_HELD.into());
        }
        tx.execute(
            "INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",
            params![id, REPLAY_EVENT, audit.to_string(), stamp],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub(in crate::text_executor) fn sdk_review_entry_migrated_receipt_bound(
        &self,
        job: &Value,
        snapshot: &Value,
    ) -> Result<bool> {
        if job["receipt"]["migrationPolicy"] != REPLAY {
            return Ok(false);
        }
        let request: TextRequest =
            serde_json::from_value(snapshot.clone()).map_err(|_| REPLAY_HELD)?;
        if !eligible(&request) || job["status"] != "completed" || !job["error"].is_null() {
            return Err(REPLAY_HELD.into());
        }
        let run = self.workflow_id.as_deref().ok_or(REPLAY_HELD)?;
        let id = s(job, "id");
        let key = step_key(&request);
        let c = self.store.conn()?;
        let mut query = c
            .prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2")
            .map_err(|e| e.to_string())?;
        let proofs = query
            .query_map(params![id, REPLAY_EVENT], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if proofs.len() != 1 {
            return Err(REPLAY_HELD.into());
        }
        let audit: Value = serde_json::from_str(&proofs[0]).map_err(|_| REPLAY_HELD)?;
        let old = &audit["originalJob"];
        let raw = parse(old, "resultJson")?;
        let mut prepared = parse(old, "preparedJson")?;
        if prepared.get("sdkDomainResult").is_some()
            || prepared.get("sdkDomainResultSha256").is_some()
        {
            return Err(REPLAY_HELD.into());
        }
        prepared["sdkDomainResult"] = raw.clone();
        prepared["sdkDomainResultSha256"] = json!(hash(&raw));
        let accepted = receipt(&audit);
        let mut expected = old.clone();
        expected["state"] = json!("completed");
        expected["error"] = Value::Null;
        expected["receiptJson"] = json!(accepted.to_string());
        expected["preparedJson"] = json!(prepared.to_string());
        let old_step = json!({"workflowId":run,"nodeKey":key,"stage":request.stage,"requestHash":hash(snapshot),"state":"rejected","jobId":id,"resultJson":raw.to_string(),"error":REJECTION});
        let mut next_step = old_step.clone();
        next_step["state"] = json!("accepted");
        next_step["error"] = Value::Null;
        let parent_json:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|r.get(0)).map_err(|_|REPLAY_HELD)?;
        let attempt = FrozenAttempt::load(
            &self
                .store
                .root
                .join("text-runs")
                .join(id)
                .join("sdk-attempt.json"),
        )?;
        let sdk_event = audit["originalEvents"]
            .as_array()
            .and_then(|rows| rows.iter().rfind(|e| e["type"] == "sdk_raw_result"))
            .ok_or(REPLAY_HELD)?;
        if audit.as_object().is_none_or(|fields| fields.len() != 12)
            || audit["version"] != REPLAY
            || audit["generationRequestsSent"] != 0
            || audit["workflowId"] != run
            || audit["nodeKey"] != key
            || audit["requestHash"] != hash(snapshot)
            || audit["rawSha256"] != hash(&raw)
            || old["id"] != id
            || old["state"] != "rejected"
            || old["error"] != REJECTION
            || old["snapshotJson"] != snapshot.to_string()
            || old["receiptJson"]
                != json!({"contract":"desktop-runner/v1","validation":"rejected"}).to_string()
            || audit["originalCheckpoint"] != old_step
            || audit["workflowSnapshotJson"] != parent_json
            || record(&c, id)? != expected
            || checkpoint(&c, run, &key)? != next_step
            || job["receipt"] != accepted
            || job["prepared"] != prepared
            || job["result"] != raw
            || job["snapshot"] != *snapshot
            || files(&self.store.root, id)? != audit["originalFiles"]
            || attempt.raw_result.as_ref() != Some(&sdk_event["data"])
            || !events_preserved(&audit["originalEvents"], &events(&c, id)?)
            || self.sdk_review_entry_revalidation(&request, &raw)? != audit["domainProof"]
        {
            return Err(REPLAY_HELD.into());
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "text_executor_sdk_series_review_replay_tests.rs"]
mod tests;
