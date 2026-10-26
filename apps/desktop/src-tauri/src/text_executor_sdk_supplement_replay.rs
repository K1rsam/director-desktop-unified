//! Revalidate exact, proven supplement-validator false rejections. This accepts
//! original design bytes for independent review, never retries their transport.
use super::*;

const REPLAY: &str = "sdk-series-supplement-source-quote-revalidation/v1";
const KEY_REPLAY: &str = "sdk-series-supplement-opaque-key-revalidation/v1";
const CORRECTION_FACT_REPLAY: &str = "sdk-series-correction-source-fact-contract-revalidation/v1";
const REPLAY_EVENT: &str = "sdk_series_supplement_revalidated";
const REJECTION: &str = "SERIES_SUPPLEMENT_FACT_NOT_SOURCE_QUOTE";
const KEY_REJECTION: &str = "SERIES_SUPPLEMENT_IDENTITY_INVALID";
const REPLAY_HELD: &str =
    "SDK_SERIES_SUPPLEMENT_REVALIDATION_HELD: 原补料回包的冻结证据不完整；保留原拒绝，不重发";

fn eligible(request: &TextRequest) -> bool {
    request.stage == "seriesDesign"
        && request.skill == "series-design"
        && ((supported_contract(&request.input["seriesAssetSupplement"]["version"])
            && request.input["seriesAssetSupplement"]["phase"] == "design"
            && request.input["seriesAssetSupplement"]["attempt"] == 0
            && request.input.get("seriesSupplyCorrection").is_none())
            || correction_facts_eligible(request))
        && request.input.get("validationRepair").is_none()
}

fn correction_facts_eligible(request: &TextRequest) -> bool {
    request.input["seriesSupplyCorrection"]["version"] == super::super::supply_correction::CONTRACT
        && request.input["seriesSupplyCorrection"]["phase"] == "design"
        && request.input["seriesSupplyCorrection"]["attempt"] == 0
        && request.input.get("seriesAssetSupplement").is_none()
}

fn policy(request: &TextRequest, rejection: &Value) -> Option<&'static str> {
    if !eligible(request) {
        return None;
    }
    if correction_facts_eligible(request) {
        return matches!(
            rejection.as_str(),
            Some(REJECTION) | Some("SERIES_SUPPLY_CORRECTION_FACT_NOT_SOURCE_QUOTE")
        )
        .then_some(CORRECTION_FACT_REPLAY);
    }
    match (
        request.input["seriesAssetSupplement"]["version"].as_str(),
        rejection.as_str(),
    ) {
        (Some(CONTRACT), Some(REJECTION)) => Some(REPLAY),
        (Some(CONTRACT_V2), Some(KEY_REJECTION)) => Some(KEY_REPLAY),
        _ => None,
    }
}

fn record(c: &rusqlite::Connection, id: &str) -> Result<Value> {
    c.query_row("SELECT snapshot,result,prepared,receipt,state,error FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk'",[id],|r| {
        Ok(json!({"id":id,"snapshotJson":r.get::<_,String>(0)?,"resultJson":r.get::<_,String>(1)?,
            "preparedJson":r.get::<_,String>(2)?,"receiptJson":r.get::<_,String>(3)?,
            "state":r.get::<_,String>(4)?,"error":r.get::<_,Option<String>>(5)?}))
    }).map_err(|_|REPLAY_HELD.into())
}
fn checkpoint(c: &rusqlite::Connection, run: &str, key: &str) -> Result<Value> {
    c.query_row("SELECT stage,request_hash,state,text_job_id,result,error FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2",params![run,key],|r| {
        Ok(json!({"workflowId":run,"nodeKey":key,"stage":r.get::<_,String>(0)?,"requestHash":r.get::<_,String>(1)?,
            "state":r.get::<_,String>(2)?,"jobId":r.get::<_,String>(3)?,"resultJson":r.get::<_,String>(4)?,"error":r.get::<_,Option<String>>(5)?}))
    }).map_err(|_|REPLAY_HELD.into())
}
fn files(root: &Path, job: &str) -> Result<Value> {
    let base = root.join("text-runs").join(job);
    let mut out = Vec::new();
    for (name, limit) in [
        ("sdk-attempt.json", 4_194_304),
        ("sdk-compiled.json", 2_097_152),
        ("result.json", 1_048_576),
    ] {
        let (sha, size) = crate::store::hash_file(&base.join(name))?;
        if size > limit {
            return Err(REPLAY_HELD.into());
        }
        out.push(json!({"file":name,"sha256":sha,"bytes":size}));
    }
    Ok(json!(out))
}
fn parse(record: &Value, key: &str) -> Result<Value> {
    serde_json::from_str(s(record, key)).map_err(|_| REPLAY_HELD.into())
}
fn receipt(audit: &Value) -> Value {
    json!({"contract":"desktop-runner/v1","validation":"accepted","migrationPolicy":audit["version"],
        "migrationAuditSha256":hash(audit),"acceptedRequestHash":audit["requestHash"],
        "validatedResultSha256":audit["rawSha256"]})
}
fn evidence_events_preserved(original: &Value, current: &Value) -> bool {
    let (Some(old), Some(now)) = (original.as_array(), current.as_array()) else {
        return false;
    };
    let Some(last) = old.last().and_then(|e| e["id"].as_i64()) else {
        return false;
    };
    if now
        .iter()
        .filter(|e| e["id"].as_i64().is_some_and(|id| id <= last))
        .cloned()
        .collect::<Vec<_>>()
        != *old
    {
        return false;
    }
    ["text_received_raw", "sdk_raw_result"].iter().all(|kind| {
        old.iter()
            .rfind(|e| e["type"] == *kind)
            .is_some_and(|event| now.iter().rfind(|e| e["type"] == *kind) == Some(event))
    })
}

impl SavedTextExecutor {
    fn sdk_supplement_revalidation_origin(
        &self,
        request: &TextRequest,
        raw: &Value,
    ) -> Result<Value> {
        if !eligible(request) {
            return Err(REPLAY_HELD.into());
        }
        self.sdk_parent_binding()?;
        self.sdk_request_policy(request)?;
        if correction_facts_eligible(request) {
            let (origin, correction) = self.sdk_supply_fact_revalidation_context(request, raw)?;
            let c = self.store.conn()?;
            return Ok(json!({"record":record(&c,s(&origin,"id"))?,
                "events":events(&c,s(&origin,"id"))?,"files":files(&self.store.root,s(&origin,"id"))?,
                "correctionSourceFactContract":correction}));
        }
        let context = self.sdk_supplement_context(request)?;
        let origin = self.sdk_supplement_find_exact(
            s(context, "originExecutionRoleId"),
            s(context, "originRequestSha256"),
        )?;
        let origin_raw = self.sdk_schema_owned_raw(&origin)?;
        self.sdk_schema_supplement_bound(request, &origin, &origin_raw)?;
        let origin_request: TextRequest =
            serde_json::from_value(origin["snapshot"].clone()).map_err(|_| REPLAY_HELD)?;
        let parent = self
            .store
            .job(self.workflow_id.as_deref().ok_or(REPLAY_HELD)?)?;
        if request.input["seriesAssetSupplement"]["version"] == CONTRACT_V2 {
            crate::pipeline::series::revalidate_supplement_key_response(
                request,
                raw,
                &origin_request,
                &origin_raw,
                &parent["snapshot"]["input"],
            )?;
        } else {
            crate::pipeline::series::revalidate_supplement_design_response(
                request,
                raw,
                &origin_request,
                &origin_raw,
                &parent["snapshot"]["input"],
            )?;
        }
        let c = self.store.conn()?;
        Ok(
            json!({"record":record(&c,s(&origin,"id"))?,"events":events(&c,s(&origin,"id"))?,"files":files(&self.store.root,s(&origin,"id"))?}),
        )
    }

    pub(in crate::text_executor) fn sdk_supplement_replay_commit(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        job: &Value,
        value: &Value,
        reason: Option<&str>,
    ) -> Result<bool> {
        let Some(replay_policy) = policy(request, &job["error"]) else {
            return Ok(false);
        };
        if job["status"] != "rejected" || reason.is_some() {
            return Ok(false);
        }
        let rejection = s(job, "error");
        if value != &job["result"]
            || snapshot != &job["snapshot"]
            || self.sdk_schema_owned_raw(job)? != *value
        {
            return Err(REPLAY_HELD.into());
        }
        let origin = self.sdk_supplement_revalidation_origin(request, value)?;
        if correction_facts_eligible(request) {
            self.sdk_supply_fact_revalidation_current_guard(request)?;
        }
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
            || old["error"] != rejection
            || step
                != json!({"workflowId":run,"nodeKey":key,"stage":request.stage,"requestHash":hash(snapshot),"state":"rejected","jobId":id,"resultJson":value.to_string(),"error":rejection})
            || old["snapshotJson"] != snapshot.to_string()
            || old["resultJson"] != value.to_string()
            || raw_events
                .as_array()
                .is_none_or(|rows| rows.iter().any(|e| e["type"] == REPLAY_EVENT))
        {
            return Err(REPLAY_HELD.into());
        }
        // The raw SDK event is independent evidence, not a new normalized result.
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
        let audit = json!({"version":replay_policy,"workflowId":run,"nodeKey":key,"requestHash":hash(snapshot),"rawSha256":hash(value),
            "originalJob":old,"originalCheckpoint":step,"originalEvents":raw_events,"originalFiles":files(&self.store.root,id)?,
            "workflowSnapshotJson":parent_json,"origin":origin,"generationRequestsSent":0});
        let mut prepared = parse(&old, "preparedJson")?;
        // A rejected callback never carried accepted domain data in production.
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
            || record(&tx, s(&origin["record"], "id"))? != origin["record"]
            || events(&tx, s(&origin["record"], "id"))? != origin["events"]
            || files(&self.store.root, id)? != audit["originalFiles"]
            || files(&self.store.root, s(&origin["record"], "id"))? != origin["files"]
            || parent(&tx, run, &self.settings)? != crate::pipeline::digest(&parent_json)
        {
            return Err(REPLAY_HELD.into());
        }
        // Recheck the frozen identity event/context inside the write lock as well.
        self.sdk_supplement_revalidation_origin(request, value)?;
        if correction_facts_eligible(request) {
            self.sdk_supply_fact_revalidation_current_guard(request)?;
        }
        let changed=tx.execute("UPDATE jobs SET state='completed',error=NULL,receipt=?1,prepared=?2,updated_at=?3 WHERE id=?4 AND state='rejected' AND error=?5 AND snapshot=?6 AND result=?7 AND prepared=?8 AND receipt=?9",
            params![accepted.to_string(),prepared.to_string(),stamp,id,rejection,s(&old,"snapshotJson"),s(&old,"resultJson"),s(&old,"preparedJson"),s(&old,"receiptJson")]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err(REPLAY_HELD.into());
        }
        let changed=tx.execute("UPDATE workflow_steps SET state='accepted',error=NULL,updated_at=?1 WHERE workflow_id=?2 AND node_key=?3 AND stage='seriesDesign' AND text_job_id=?4 AND request_hash=?5 AND state='rejected' AND result=?6 AND error=?7",
            params![stamp,run,key,id,hash(snapshot),value.to_string(),rejection]).map_err(|e|e.to_string())?;
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

    pub(in crate::text_executor) fn sdk_supplement_migrated_receipt_bound(
        &self,
        job: &Value,
        snapshot: &Value,
    ) -> Result<bool> {
        if !matches!(
            job["receipt"]["migrationPolicy"].as_str(),
            Some(REPLAY) | Some(KEY_REPLAY) | Some(CORRECTION_FACT_REPLAY)
        ) {
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
        let mut q = c
            .prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2")
            .map_err(|e| e.to_string())?;
        let proofs = q
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
        let replay_policy = policy(&request, &old["error"]).ok_or(REPLAY_HELD)?;
        let rejection = s(old, "error");
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
        let old_step = json!({"workflowId":run,"nodeKey":key,"stage":request.stage,"requestHash":hash(snapshot),"state":"rejected","jobId":id,"resultJson":raw.to_string(),"error":rejection});
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
        let original_sdk_event = audit["originalEvents"]
            .as_array()
            .and_then(|rows| rows.iter().rfind(|e| e["type"] == "sdk_raw_result"))
            .ok_or(REPLAY_HELD)?;
        if audit.as_object().is_none_or(|fields| fields.len() != 12)
            || audit["version"] != replay_policy
            || audit["generationRequestsSent"] != 0
            || audit["workflowId"] != run
            || audit["nodeKey"] != key
            || audit["requestHash"] != hash(snapshot)
            || audit["rawSha256"] != hash(&raw)
            || old["id"] != id
            || old["state"] != "rejected"
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
            || attempt.raw_result.as_ref() != Some(&original_sdk_event["data"])
            || !evidence_events_preserved(&audit["originalEvents"], &events(&c, id)?)
            || self.sdk_supplement_revalidation_origin(&request, &raw)? != audit["origin"]
        {
            return Err(REPLAY_HELD.into());
        }
        Ok(true)
    }
}
