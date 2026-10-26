//! Only a proven absent SDK node can bind a replacement native supervisor.
//! Existing requests, raw results and parent settings are never rewritten.
use super::*;
use crate::text_transport::FrozenAttempt;

const POLICY: &str = "sdk-new-node-supervisor-handoff/v1";
const ACCOUNT_POLICY: &str = "sdk-new-node-current-account/v2";
const FIRST_NODE_POLICY: &str = "sdk-first-node-supervisor-handoff/v1";
const LOCAL_RECOVERY_POLICY: &str = "sdk-new-node-local-recovery-handoff/v1";
const REJECTED_PLAN_POLICY: &str = "sdk-rejected-plan-repair-handoff/v1";
#[path = "text_executor_sdk_rejected_plan_handoff.rs"]
mod rejected_plan;
const FIELD: &str = "sdkSupervisorHandoff";
const HELD: &str =
    "SDK_SUPERVISOR_HANDOFF_HELD: 新节点交接证据不足或原节点已存在；保留原请求、回包和预算，不重发";

fn domain(snapshot: &Value) -> Result<Value> {
    let mut value = snapshot.clone();
    let object = value.as_object_mut().ok_or(HELD)?;
    object.remove("executorIdentity");
    object.remove(FIELD);
    Ok(value)
}
fn hash(value: &Value) -> String {
    crate::pipeline::digest(&value.to_string())
}
fn seal(mut proof: Value) -> Value {
    proof["sha256"] = json!(hash(&proof));
    proof
}
fn base_policy(proof: &Value) -> &str {
    if proof["policy"] == ACCOUNT_POLICY { s(&proof["accountAuthorization"], "basePolicy") }
    else { s(proof, "policy") }
}
fn proof_valid(snapshot: &Value, settings: &Value, run: &str, key: &str) -> bool {
    let proof = &snapshot[FIELD];
    let Some(fields) = proof.as_object() else {
        return false;
    };
    let account = proof["policy"] == ACCOUNT_POLICY;
    let base = base_policy(proof);
    if fields.len() != if account {15} else {14} {
        return false;
    }
    let mut unsigned = proof.clone();
    unsigned.as_object_mut().unwrap().remove("sha256");
    let expected_domain = match domain(snapshot) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Ok(request) = serde_json::from_value::<TextRequest>(snapshot.clone()) else {
        return false;
    };
    [POLICY,LOCAL_RECOVERY_POLICY,FIRST_NODE_POLICY,REJECTED_PLAN_POLICY].contains(&base)
        && proof["sha256"] == hash(&unsigned)
        && proof["workflowId"] == run
        && proof["nodeKey"] == key
        && proof["stage"] == request.stage
        && step_key(&request) == key
        && proof["domainRequestSha256"] == hash(&expected_domain)
        && proof["originalSettingsSha256"] == hash(settings)
        && proof["originalIdentity"] == settings["textSdkIdentity"]
        && proof["effectiveIdentitySha256"] == hash(&snapshot["executorIdentity"])
        && proof["modelCalls"] == 0
        && proof["accountModelChecked"] == true
        && settings["textProvider"] == "codex_sdk"
        && settings["textModel"] == request.model
        && settings["reasoning"] == request.reasoning
        && if account {
            sdk_account_handoff::shape_bound(&proof["accountAuthorization"],settings,&snapshot["executorIdentity"],&request,base)
                && (proof["accountAuthorization"]["previousNode"].is_null() == (base==FIRST_NODE_POLICY))
                && crate::text_sdk::current_account_receipt_identity_bound(&proof["originalIdentity"],&snapshot["executorIdentity"],if base==LOCAL_RECOVERY_POLICY {"local"}else{"supervisor"})
        } else if base == LOCAL_RECOVERY_POLICY {
            crate::text_sdk::local_recovery_identity_bound(&proof["originalIdentity"], &snapshot["executorIdentity"])
        } else { crate::text_sdk::supervisor_handoff_receipt_identity_bound(&proof["originalIdentity"], &snapshot["executorIdentity"]) }
        && proof["resourceHashes"].as_object().is_some_and(|h| {
            h.len() == 9
                && h.values().all(|v| {
                    v.as_str()
                        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                })
        })
        && if base == FIRST_NODE_POLICY {
            proof["predecessor"] == first_node_evidence(run)
        } else if base == REJECTED_PLAN_POLICY {
            request.stage == "storyboardPlan" && request.skill == "episode-emotion-planner"
                && request.input["planRepair"]["version"] == "rejected-plan-repair/v1"
                && proof["predecessor"].as_object().is_some_and(|p|p.len()==16)
        } else { proof["predecessor"].as_object().is_some_and(|p| p.len() == 14) }
}

fn parent_bound(c: &rusqlite::Connection, proof: &Value, settings: &Value) -> Result<bool> {
    let original: Option<String> = c
        .query_row(
            "SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow'",
            [s(proof, "workflowId")],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(original) = original else {
        return Ok(false);
    };
    let value: Value = serde_json::from_str(&original).map_err(|e| e.to_string())?;
    Ok(
        crate::pipeline::digest(&original) == s(proof, "parentSnapshotSha256")
            && value
                .pointer("/input/settings")
                .or_else(|| value.get("settings"))
                == Some(settings),
    )
}

fn predecessor(
    store: &Store,
    c: &rusqlite::Connection,
    run: &str,
    key: &str,
) -> Result<Option<Value>> {
    let row: Option<(String,String,String,String,String,String,String,String,String)> = c.query_row(
        "SELECT j.id,w.node_key,w.stage,w.request_hash,j.snapshot,j.prepared,j.result,j.receipt,w.result FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND w.state='accepted' AND j.kind='text' AND j.provider='codex_sdk' AND j.state='completed' AND j.error IS NULL AND w.error IS NULL AND w.result IS NOT NULL AND j.result IS NOT NULL AND j.prepared IS NOT NULL AND j.receipt IS NOT NULL",
        params![run,key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
    ).optional().map_err(|e| e.to_string())?;
    let Some((job, node, stage, request, snapshot, prepared, result, receipt, accepted)) = row
    else {
        return Ok(None);
    };
    if crate::pipeline::digest(&snapshot) != request {
        return Err(HELD.into());
    }
    let prepared_value: Value = serde_json::from_str(&prepared).map_err(|e| e.to_string())?;
    let accepted_value: Value = serde_json::from_str(&accepted).map_err(|e| e.to_string())?;
    let snapshot_value: Value = serde_json::from_str(&snapshot).map_err(|e| e.to_string())?;
    let previous_request: TextRequest =
        serde_json::from_value(snapshot_value).map_err(|e| e.to_string())?;
    if prepared_value["sdkDomainResult"] != accepted_value
        || prepared_value["sdkDomainResultSha256"] != hash(&accepted_value)
        || prepared_value["sdkOriginWorkflowId"] != run
        || previous_request.stage != stage
        || step_key(&previous_request) != node
    {
        return Err(HELD.into());
    }
    let directory = store.root.join("text-runs").join(&job);
    let attempt = FrozenAttempt::load(&directory.join("sdk-attempt.json"))?;
    if attempt.workflow_id != run
        || attempt.logical_node_id != node
        || attempt.state != crate::text_transport::AttemptState::Completed
        || attempt.backend != crate::text_transport::Backend::CodexSdk
    {
        return Err(HELD.into());
    }
    let attempt_hash = crate::store::hash_file(&directory.join("sdk-attempt.json"))?.0;
    let compiled_hash = crate::store::hash_file(&directory.join("sdk-compiled.json"))?.0;
    let raw_hash = crate::store::hash_file(&directory.join("result.json"))?.0;
    if prepared_value["sdkRawTextSha256"] != raw_hash {
        return Err(HELD.into());
    }
    let event: Option<(i64,String)> = c.query_row(
        "SELECT id,data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",
        [&job], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(|e| e.to_string())?;
    let Some((event_id, event_data)) = event else {
        return Err(HELD.into());
    };
    Ok(Some(
        json!({"jobId":job,"nodeKey":node,"stage":stage,"requestHash":request,
        "snapshotSha256":crate::pipeline::digest(&snapshot),"preparedSha256":crate::pipeline::digest(&prepared),
        "resultSha256":crate::pipeline::digest(&result),"receiptSha256":crate::pipeline::digest(&receipt),
        "acceptedResultSha256":crate::pipeline::digest(&accepted),
        "attemptFileSha256":attempt_hash,"compiledFileSha256":compiled_hash,"rawFileSha256":raw_hash,
        "rawEventId":event_id,"rawEventDataSha256":crate::pipeline::digest(&event_data)}),
    ))
}
fn predecessor_bound(store: &Store, c: &rusqlite::Connection, proof: &Value, snapshot: &Value) -> Result<bool> {
    if base_policy(proof) == FIRST_NODE_POLICY {
        // The absence proof is checked before reservation. After reservation,
        // bind the original first step, allowing later steps without re-sending.
        return c.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_steps first WHERE first.workflow_id=?1 AND first.node_key=?2 AND NOT EXISTS(SELECT 1 FROM workflow_steps older WHERE older.workflow_id=first.workflow_id AND older.rowid<first.rowid))",
            params![s(proof,"workflowId"),s(proof,"nodeKey")], |r| r.get(0),
        ).map_err(|e|e.to_string());
    }
    if base_policy(proof) == REJECTED_PLAN_POLICY {
        return Ok(rejected_plan::predecessor(store,c,s(proof,"workflowId"),snapshot)? == proof["predecessor"]);
    }
    Ok(predecessor(
        store,
        c,
        s(proof, "workflowId"),
        s(&proof["predecessor"], "nodeKey"),
    )?
    .as_ref()
        == Some(&proof["predecessor"]))
}

fn first_node_evidence(run: &str) -> Value {
    json!({"policy":"sdk-workflow-never-dispatched/v1","workflowId":run,"workflowSteps":0,"textJobs":0,"attempts":0})
}

fn first_node_absent(store: &Store, c: &rusqlite::Connection, run: &str) -> Result<()> {
    let occupied: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1) OR EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND (json_extract(prepared,'$.sdkOriginWorkflowId')=?1 OR json_extract(snapshot,'$.sdkSupervisorHandoff.workflowId')=?1 OR json_extract(snapshot,'$.sdkSchemaContinuation.workflowId')=?1 OR json_extract(snapshot,'$.sdkSchemaRejectionRetry.workflowId')=?1))",
        [run], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if occupied { return Err(HELD.into()); }
    let root=store.root.join("text-runs");
    if root.exists() {
        for (index,entry) in std::fs::read_dir(root).map_err(|e|e.to_string())?.enumerate() {
            if index>=20_000 { return Err(HELD.into()); }
            let path=entry.map_err(|e|e.to_string())?.path().join("sdk-attempt.json");
            if path.exists() {
                let attempt=FrozenAttempt::load(&path).map_err(|_|HELD.to_string())?;
                // Any node and any attempt state blocks the first-node route.
                if attempt.workflow_id==run { return Err(HELD.into()); }
            }
        }
    }
    Ok(())
}

fn no_node_or_attempt(
    store: &Store,
    c: &rusqlite::Connection,
    run: &str,
    key: &str,
    snapshot: &Value,
) -> Result<()> {
    let occupied: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2)",
            params![run, key],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if occupied {
        return Err(HELD.into());
    }
    let expected = domain(snapshot)?;
    let mut query = c
        .prepare("SELECT snapshot FROM jobs WHERE kind='text' AND provider='codex_sdk'")
        .map_err(|e| e.to_string())?;
    let rows = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let prior: Value =
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        // Strip known wrappers only to reject an occupied domain. This is not
        // an acceptance path for any historical proof.
        let mut occupied_domain=domain(&prior)?;
        if let Some(fields)=occupied_domain.as_object_mut() {fields.remove("sdkSchemaRejectionRetry");fields.remove("sdkSchemaContinuation");}
        if occupied_domain == expected {
            return Err(HELD.into());
        }
    }
    let unresolved: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND provider='codex_sdk' AND state NOT IN ('completed','rejected') AND json_extract(prepared,'$.sdkDispatchStarted')=1)",
        [], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if unresolved {
        return Err(HELD.into());
    }
    let root = store.root.join("text-runs");
    if root.exists() {
        for (index, entry) in std::fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .enumerate()
        {
            if index >= 20_000 {
                return Err(HELD.into());
            }
            let path = entry
                .map_err(|e| e.to_string())?
                .path()
                .join("sdk-attempt.json");
            if path.exists() {
                let attempt = FrozenAttempt::load(&path).map_err(|_| HELD.to_string())?;
                if attempt.workflow_id == run && attempt.logical_node_id == key {
                    return Err(HELD.into());
                }
            }
        }
    }
    Ok(())
}

impl SavedTextExecutor {
    fn sdk_handoff_predecessor(&self, original: &Value, local_recovery: bool) -> Result<(&'static str, Value)> {
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let c=self.store.conn()?;
        if original["input"].get("planRepair").is_some() {
            if local_recovery { return Err(HELD.into()); }
            return Ok((REJECTED_PLAN_POLICY,rejected_plan::predecessor(&self.store,&c,run,original)?));
        }
        let previous_key: Option<String> = c.query_row("SELECT w.node_key FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.state='accepted' AND j.kind='text' AND j.provider='codex_sdk' AND j.state='completed' ORDER BY w.updated_at DESC,w.rowid DESC LIMIT 1",[run],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let Some(previous_key)=previous_key else {
            if local_recovery { return Err(HELD.into()); }
            first_node_absent(&self.store,&c,run)?;
            return Ok((FIRST_NODE_POLICY,first_node_evidence(run)));
        };
        let prior=predecessor(&self.store,&c,run,&previous_key)?.ok_or(HELD)?;
        let previous_job=self.sdk_job(s(&prior,"jobId"))?;
        if previous_job["snapshot"]["model"]!=self.settings["textModel"]
            || previous_job["snapshot"]["reasoning"]!=self.settings["reasoning"]
            || (previous_job["snapshot"]["executorIdentity"]!=original["executorIdentity"] && previous_job["snapshot"].get(FIELD).is_none()) {
            return Err(HELD.into());
        }
        self.sdk_handoff_job_bound(&previous_job,&previous_job["snapshot"])?;
        self.sdk_raw(&previous_job,&previous_job["snapshot"])?;
        if local_recovery && self.sdk_local_recovery_bound(&previous_job)?.is_none()
            && base_policy(&previous_job["snapshot"][FIELD])!=LOCAL_RECOVERY_POLICY { return Err(HELD.into()); }
        Ok((if local_recovery {LOCAL_RECOVERY_POLICY}else{POLICY},prior))
    }

    pub(super) fn sdk_handoff_saved_snapshot(
        &self,
        request: &TextRequest,
        expected_domain: &Value,
    ) -> Result<Option<Value>> {
        if self.settings["textProvider"] != "codex_sdk" {
            return Ok(None);
        }
        let Some(run) = &self.workflow_id else {
            return Ok(None);
        };
        let c = self.store.conn()?;
        let row: Option<(String,Option<String>,String,String,String,String)> = c.query_row(
            "SELECT j.snapshot,j.prepared,w.request_hash,j.provider,w.stage,j.id FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND j.kind='text'",
            params![run,step_key(request)], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional().map_err(|e| e.to_string())?;
        let Some((snapshot_json, prepared_json, request_hash, provider, stage, job_id)) = row
        else {
            let orphan: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND provider='codex_sdk' AND json_extract(snapshot,'$.sdkSupervisorHandoff.workflowId')=?1 AND json_extract(snapshot,'$.sdkSupervisorHandoff.nodeKey')=?2)",params![run,step_key(request)],|r|r.get(0)).map_err(|e|e.to_string())?;
            if orphan {
                return Err(HELD.into());
            }
            return Ok(None);
        };
        let saved: Value = serde_json::from_str(&snapshot_json).map_err(|e| e.to_string())?;
        if saved.get(FIELD).is_none() {
            return Ok(None);
        }
        let prepared: Value = serde_json::from_str(prepared_json.as_deref().ok_or(HELD)?)
            .map_err(|e| e.to_string())?;
        let created: Option<String> = c.query_row("SELECT data FROM events WHERE job_id=?1 AND type='sdk_supervisor_handoff_frozen' ORDER BY id DESC LIMIT 1",[&job_id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if provider != "codex_sdk"
            || stage != request.stage
            || saved.to_string() != snapshot_json
            || crate::pipeline::digest(&snapshot_json) != request_hash
            || domain(&saved)? != *expected_domain
            || prepared[FIELD] != saved[FIELD]
            || !proof_valid(&saved, &self.settings, run, &step_key(request))
            || !parent_bound(&c, &saved[FIELD], &self.settings)?
            || !predecessor_bound(&self.store, &c, &saved[FIELD], &saved)?
            || created.as_deref() != Some(saved[FIELD].to_string().as_str())
        {
            return Err(HELD.into());
        }
        if saved[FIELD]["policy"]==ACCOUNT_POLICY {
            self.sdk_current_account_authorization_bound(&saved,&saved[FIELD]["accountAuthorization"],base_policy(&saved[FIELD]))?;
        }
        Ok(Some(saved))
    }

    pub(super) fn sdk_handoff_existing_domain(&self, saved: &Value) -> Result<Option<Value>> {
        if saved.get(FIELD).is_none() {
            return Ok(None);
        }
        let request: TextRequest =
            serde_json::from_value(saved.clone()).map_err(|_| HELD.to_string())?;
        let original_domain = domain(saved)?;
        if self
            .sdk_handoff_saved_snapshot(&request, &original_domain)?
            .as_ref()
            != Some(saved)
        {
            return Err(HELD.into());
        }
        Ok(Some(original_domain))
    }

    pub(super) fn sdk_handoff_job_bound(&self, job: &Value, snapshot: &Value) -> Result<()> {
        if snapshot.get(FIELD).is_none() {
            return Ok(());
        }
        let run = self.workflow_id.as_deref().ok_or(HELD)?;
        let request: TextRequest =
            serde_json::from_value(snapshot.clone()).map_err(|_| HELD.to_string())?;
        let saved = self
            .sdk_handoff_saved_snapshot(&request, &domain(snapshot)?)?
            .ok_or(HELD)?;
        let c = self.store.conn()?;
        let owned: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2 AND text_job_id=?3 AND request_hash=?4)",params![run,step_key(&request),s(job,"id"),hash(snapshot)],|r|r.get(0)).map_err(|e|e.to_string())?;
        if saved != *snapshot || job["prepared"][FIELD] != snapshot[FIELD] || !owned {
            return Err(HELD.into());
        }
        Ok(())
    }

    pub(super) async fn sdk_new_node_snapshot(
        &self,
        request: &TextRequest,
        original: &Value,
    ) -> Result<Value> {
        if let Some(snapshot) = self.sdk_schema_continuation_snapshot(request, original).await? {
            return Ok(snapshot);
        }
        if original["executorIdentity"]["version"] != "official-sdk-identity/v2"
            || original.get(FIELD).is_some()
        {
            return Ok(original.clone());
        }
        let Some(run) = &self.workflow_id else {
            return Ok(original.clone());
        };
        no_node_or_attempt(
            &self.store,
            &self.store.conn()?,
            run,
            &step_key(request),
            original,
        )?;
        self.sdk_parent_binding()?;
        self.sdk_request_policy(request)?;
        self.check_sdk_unresolved_hold()?;
        let reference=self.sdk_current_account_reference()?;
        let mut runtime_settings=self.settings.clone();
        if let Some((previous,_))=&reference {runtime_settings["textSdkIdentity"]=previous.clone();}
        let verified=crate::text_sdk::prepare_current_account_handoff(&runtime_settings,false).await?;
        let local_recovery = original["executorIdentity"]["bridgeSha256"] == crate::text_sdk::LOCAL_PATH_BRIDGE_SHA;
        // Even when the supervisor is unchanged, the current account/model
        // check above must complete. Returning to A after B needs its own proof.
        let changed_account=self.sdk_current_account_authorization(request,&verified.identity,reference.as_ref(),POLICY).is_some();
        if verified.identity==original["executorIdentity"] && !changed_account {return Ok(original.clone());}
        let (policy,prior)=self.sdk_handoff_predecessor(original,local_recovery)?;
        let authorization=self.sdk_current_account_authorization(request,&verified.identity,reference.as_ref(),policy);
        let c = self.store.conn()?;
        let original_parent: String = c
            .query_row(
                "SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow'",
                [run],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut snapshot = original.clone();
        snapshot["executorIdentity"] = verified.identity;
        snapshot[FIELD] = seal(
            json!({"policy":policy,"workflowId":run,"nodeKey":step_key(request),"stage":request.stage,
            "domainRequestSha256":hash(&domain(original)?),"parentSnapshotSha256":crate::pipeline::digest(&original_parent),
            "originalSettingsSha256":hash(&self.settings),"originalIdentity":original["executorIdentity"],
            "effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":verified.resource_hashes,
            "predecessor":prior,"modelCalls":0,"accountModelChecked":true}),
        );
        if let Some(authorization)=authorization {
            let mut proof=snapshot[FIELD].clone();proof.as_object_mut().unwrap().remove("sha256");
            proof["policy"]=json!(ACCOUNT_POLICY);proof["accountAuthorization"]=authorization;
            snapshot[FIELD]=seal(proof);
        }
        if !proof_valid(&snapshot, &self.settings, run, &step_key(request)) {
            return Err(HELD.into());
        }
        Ok(snapshot)
    }
}

impl Store {
    pub(super) fn add_sdk_handoff_node_atomic(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        original_settings: &Value,
    ) -> Result<String> {
        let proof = &snapshot[FIELD];
        let run = s(proof, "workflowId");
        let key = step_key(request);
        let executor = SavedTextExecutor {
            store: self.clone(),
            settings: original_settings.clone(),
            workflow_id: Some(run.to_owned()),
        };
        if !proof_valid(snapshot, original_settings, run, &key)
            || domain(snapshot)? != executor.request_domain_snapshot(request)?
        {
            return Err(HELD.into());
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if proof["policy"]==ACCOUNT_POLICY { executor.check_sdk_unresolved_hold()?; }
        if !parent_bound(&tx, proof, original_settings)? {
            return Err(HELD.into());
        }
        if base_policy(proof)==FIRST_NODE_POLICY {
            first_node_absent(self,&tx,run)?;
        } else if !predecessor_bound(self,&tx,proof,snapshot)? { return Err(HELD.into()); }
        if base_policy(proof) == LOCAL_RECOVERY_POLICY {
            let prior = executor.sdk_job(s(&proof["predecessor"],"jobId"))?;
            executor.sdk_raw(&prior,&prior["snapshot"])?;
            if executor.sdk_local_recovery_bound(&prior)?.is_none()
                && base_policy(&prior["snapshot"][FIELD]) != LOCAL_RECOVERY_POLICY { return Err(HELD.into()); }
        }
        no_node_or_attempt(self, &tx, run, &key, snapshot)?;
        if proof["policy"] == ACCOUNT_POLICY {
            let previous=executor.sdk_current_account_authorization_bound(snapshot,&proof["accountAuthorization"],base_policy(proof))?;
            crate::text_sdk::verify_current_account_handoff_bytes(&previous,&snapshot["executorIdentity"],&proof["resourceHashes"],false)?;
        } else if base_policy(proof) == LOCAL_RECOVERY_POLICY {
            crate::text_sdk::verify_local_recovery_bytes(&proof["originalIdentity"], &snapshot["executorIdentity"], &proof["resourceHashes"])?;
        } else { crate::text_sdk::verify_supervisor_handoff_bytes(&proof["originalIdentity"], &snapshot["executorIdentity"], &proof["resourceHashes"])?; }
        let job = id();
        let request_hash = hash(snapshot);
        let prepared = json!({"sdkSupervisorHandoff":proof,"sdkDispatchStarted":false,"sdkRequestHash":request_hash,"sdkOriginWorkflowId":run});
        let timestamp = now();
        tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,prepared,created_at,updated_at) VALUES(?1,'text',?2,'codex_sdk','queued',?3,?4,?5,?6,?6)",params![job,format!("{}:{}:{}",request.stage,request.skill,id()),s(&snapshot["executorIdentity"],"accountFingerprint"),snapshot.to_string(),prepared.to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,result,error,updated_at) VALUES(?1,?2,?3,?4,'running',?5,NULL,NULL,?6)",params![run,key,request.stage,request_hash,job,timestamp]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'sdk_supervisor_handoff_frozen',?2,?3)",params![job,proof.to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[timestamp]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_transport::{AttemptState, Backend};
    use sha2::{Digest, Sha256};
    include!("text_executor_sdk_rejected_plan_handoff_tests.rs");
    include!("text_executor_sdk_account_handoff_tests.rs");

    fn resource_names() -> [&'static str; 9] {
        [
            "src/health.mjs",
            "src/main.mjs",
            "src/bridge.mjs",
            "src/native-runtime.mjs",
            "THIRD_PARTY_NOTICES.md",
            "package-lock.json",
            "node_modules/@openai/codex-sdk/package.json",
            "node_modules/@openai/codex-sdk/dist/index.js",
            "node_modules/@openai/codex-sdk/LICENSE",
        ]
    }
    fn fixture() -> (tempfile::TempDir, SavedTextExecutor, TextRequest, Value) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store
            .create("合成 SDK supervisor 交接", "sdk-handoff-synthetic")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        for file in [
            "node.exe",
            "codex.exe",
            "old-supervisor.exe",
            "new-supervisor.exe",
        ] {
            std::fs::write(temp.path().join(file), file).unwrap();
        }
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sdk-bridge");
        let old_resources = temp.path().join("old-resources");
        let new_resources = temp.path().join("new-resources");
        let mut resource_hashes = serde_json::Map::new();
        for relative in resource_names() {
            let bytes = std::fs::read(source.join(relative)).unwrap();
            for root in [&old_resources, &new_resources] {
                let target = root.join(relative);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(target, &bytes).unwrap();
            }
            resource_hashes.insert(
                relative.into(),
                json!(format!("{:x}", Sha256::digest(&bytes))),
            );
        }
        let path = |name: &str| temp.path().join(name).to_string_lossy().into_owned();
        let sha = |name: &str| crate::store::hash_file(&temp.path().join(name)).unwrap().0;
        let runtime_sha = sha("codex.exe");
        let bridge_sha = crate::pipeline::digest(&format!(
            "{}{}{}",
            include_str!("../../../../sdk-bridge/src/main.mjs"),
            include_str!("../../../../sdk-bridge/src/bridge.mjs"),
            include_str!("../../../../sdk-bridge/src/native-runtime.mjs")
        ));
        let old = json!({"version":"official-sdk-identity/v2","provider":"codex_sdk","sdkPackage":"@openai/codex-sdk","sdkVersion":crate::text_sdk::SDK_VERSION,
            "sdkImplementationSha256":crate::text_sdk::SDK_IMPLEMENTATION_SHA,"bridgeVersion":"0.1.0","bridgeSha256":bridge_sha,"protocolVersion":crate::text_sdk::PROTOCOL,
            "nodeExecutable":path("node.exe"),"nodeSha256":sha("node.exe"),"nodeVersion":"v24.19.0","runtimePackageVersion":"codex-cli 0.162.0-alpha.2",
            "runtimeIdentity":format!("official-codex-runtime/codex-cli 0.162.0-alpha.2/{runtime_sha}"),"permissionProfile":crate::text_sdk::NATIVE_POLICY,
            "accountFingerprint":"a".repeat(64),"runtimeExecutable":path("codex.exe"),"runtimeSha256":runtime_sha,
            "supervisorExecutable":path("old-supervisor.exe"),"supervisorSha256":sha("old-supervisor.exe"),"resourceDirectory":old_resources.to_string_lossy(),
            "credentialHome":temp.path().to_string_lossy(),"codexHome":temp.path().to_string_lossy()});
        let settings = json!({"textProvider":"codex_sdk","textModel":"gpt-6.1-sol","reasoning":"medium","textSdkIdentity":old});
        let run=store.add_job("workflow","foundation.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","input":{"settings":settings,"syntheticSourceHash":"synthetic-locked-source"}}),"running").unwrap();
        let executor = SavedTextExecutor {
            store,
            settings,
            workflow_id: Some(run),
        };
        let mut request = TextRequest {
            stage: "foundation:HANDOFF_PREVIOUS".into(),
            skill: "asset-art-director".into(),
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            input: json!({"text":"合成输入"}),
            schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),
            rule_snapshot: crate::pipeline::snapshot("asset-art-director", &[]).unwrap(),
        };
        let previous = executor.request_snapshot(&request).unwrap();
        let job = executor
            .store
            .add_job(
                "text",
                "synthetic-previous",
                "codex_sdk",
                "synthetic",
                &previous,
                "received",
            )
            .unwrap();
        let previous_key = step_key(&request);
        complete(&executor, &request, &previous, &job, false);
        request.stage = "foundation:HANDOFF_NEXT".into();
        let original = executor.request_snapshot(&request).unwrap();
        let mut effective = old;
        effective["supervisorExecutable"] = json!(path("new-supervisor.exe"));
        effective["supervisorSha256"] = json!(sha("new-supervisor.exe"));
        effective["resourceDirectory"] = json!(new_resources.to_string_lossy());
        let mut snapshot = original.clone();
        snapshot["executorIdentity"] = effective;
        let c = executor.store.conn().unwrap();
        let parent: String = c
            .query_row(
                "SELECT snapshot FROM jobs WHERE id=?1",
                [executor.workflow_id.as_ref().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let prior = predecessor(
            &executor.store,
            &c,
            executor.workflow_id.as_ref().unwrap(),
            &previous_key,
        )
        .unwrap()
        .unwrap();
        snapshot[FIELD] = seal(
            json!({"policy":POLICY,"workflowId":executor.workflow_id,"nodeKey":step_key(&request),"stage":request.stage,
            "domainRequestSha256":hash(&domain(&original).unwrap()),"parentSnapshotSha256":crate::pipeline::digest(&parent),"originalSettingsSha256":hash(&executor.settings),
            "originalIdentity":original["executorIdentity"],"effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":resource_hashes,"predecessor":prior,"modelCalls":0,"accountModelChecked":true}),
        );
        assert!(proof_valid(
            &snapshot,
            &executor.settings,
            executor.workflow_id.as_deref().unwrap(),
            &step_key(&request)
        ));
        (temp, executor, request, snapshot)
    }

    fn complete(
        executor: &SavedTextExecutor,
        request: &TextRequest,
        snapshot: &Value,
        job: &str,
        handed_off: bool,
    ) {
        let raw = json!({"ok":true});
        let compiled = compile_codex_input(request).unwrap();
        let dir = executor.store.root.join("text-runs").join(job);
        std::fs::create_dir_all(&dir).unwrap();
        let mut attempt = FrozenAttempt::prepare(
            job,
            executor.workflow_id.as_deref().unwrap(),
            &step_key(request),
            Backend::CodexSdk,
            request.clone(),
            &snapshot["executorIdentity"],
        )
        .unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        attempt.state = AttemptState::Completed;
        attempt.raw_result = Some(
            json!({"rawText":raw.to_string(),"parsedJson":raw,"domainAccepted":false,"parseError":null}),
        );
        attempt.persist(&dir.join("sdk-attempt.json")).unwrap();
        std::fs::write(dir.join("result.json"), raw.to_string()).unwrap();
        crate::store::atomic_json(&dir.join("sdk-compiled.json"),&json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)})).unwrap();
        let mut prepared = json!({"sdkRequestHash":hash(snapshot),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":executor.workflow_id,"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),"sdkRawTextSha256":hash(&raw),"sdkDomainResult":raw,"sdkDomainResultSha256":hash(&raw)});
        if handed_off {
            prepared[FIELD] = snapshot[FIELD].clone();
        }
        executor.store.conn().unwrap().execute("UPDATE jobs SET state='completed',prepared=?1,result=?2,receipt=?3,error=NULL WHERE id=?4",params![prepared.to_string(),raw.to_string(),json!({"contract":"desktop-runner/v1","validation":"accepted"}).to_string(),job]).unwrap();
        executor
            .store
            .checkpoint(
                executor.workflow_id.as_deref().unwrap(),
                &step_key(request),
                &request.stage,
                &hash(snapshot),
                "accepted",
                Some(job),
                Some(&raw),
                None,
            )
            .unwrap();
        executor
            .store
            .event(
                job,
                "text_received_raw",
                &json!({"requestHash":hash(snapshot),"value":raw}),
            )
            .unwrap();
    }
    fn recovered_chain_fixture() -> (tempfile::TempDir, SavedTextExecutor, TextRequest, Value, String) {
        let (temp,mut executor,request,template)=fixture();
        let effective=template["executorIdentity"].clone();
        executor.settings["textSdkIdentity"]["bridgeSha256"]=json!(crate::text_sdk::LOCAL_PATH_BRIDGE_SHA);
        let run=executor.workflow_id.clone().unwrap();
        let mut parent=executor.store.job(&run).unwrap()["snapshot"].clone();parent["input"]["settings"]=executor.settings.clone();
        executor.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![run,parent.to_string()]).unwrap();
        let prior_id=s(&template[FIELD]["predecessor"],"jobId").to_owned();
        let prior_job=executor.sdk_job(&prior_id).unwrap();let mut prior_snapshot=prior_job["snapshot"].clone();
        prior_snapshot["executorIdentity"]=executor.settings["textSdkIdentity"].clone();
        let prior_request:TextRequest=serde_json::from_value(prior_snapshot.clone()).unwrap();
        executor.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![prior_id,prior_snapshot.to_string()]).unwrap();
        complete(&executor,&prior_request,&prior_snapshot,&prior_id,false);
        let attempt=FrozenAttempt::load(&executor.store.root.join("text-runs").join(&prior_id).join("sdk-attempt.json")).unwrap();
        let compiled=compile_codex_input(&prior_request).unwrap();
        let mut frozen=attempt.clone();frozen.state=AttemptState::Prepared;frozen.dispatch_started=false;frozen.raw_result=None;frozen.last_sequence=0;
        let frozen=json!(frozen);
        let record=json!({"protocolVersion":crate::text_sdk::PROTOCOL,"attempt":frozen,"frozenAttemptSha256":hash(&frozen),"requestSnapshotSha256":hash(&frozen["request"]),
            "compiledPrompt":compiled.prompt,"compiledSchema":compiled.schema,"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),
            "journalSha256":crate::pipeline::digest(""),"lastSequence":0,"threadId":null,"turnId":null,"rawResult":null,"rawResultSha256":null,"usage":null,"domainAccepted":false,
            "state":"submission_unknown","error":{"code":"SDK_STREAM_ERROR","message":"SDK_NATIVE_IDENTITY: attempt directory"}});
        let seal=|value:&Value|{let text=value.to_string();format!("{},\"receiptEnvelopeSha256\":\"{}\"}}",&text[..text.len()-1],hash(value))};
        let original_bytes=seal(&record);
        let evidence=json!({"originalReceiptSeal":hash(&record),"originalReceiptSha256":crate::pipeline::digest(&original_bytes)});
        let proof=json!({"version":"sdk-local-preparation-recovery/v1","attemptId":prior_id,"attemptEnvelopeHash":attempt.attempt_envelope_hash,
            "evidenceSha256":hash(&evidence),"originalReceiptSeal":evidence["originalReceiptSeal"],"effectiveIdentitySha256":hash(&effective),"sdkEvents":0,"processTreeStopProven":true});
        let saved=json!({"proof":proof,"evidence":evidence,"effectiveIdentity":effective,"resourceHashes":template[FIELD]["resourceHashes"],"originalSettingsSha256":hash(&executor.settings)});
        let dir=executor.store.root.join("sdk-worker").join(&prior_id);std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("local-preparation-original.json"),original_bytes).unwrap();
        std::fs::write(dir.join("local-preparation-recovery.json"),proof.to_string()).unwrap();
        let mut after=record;after["state"]=json!("transport_completed");after["error"]=Value::Null;after["rawResult"]=attempt.raw_result.clone().unwrap();after["rawResultSha256"]=json!(hash(&after["rawResult"]));
        after["preDispatchRecovery"]=json!({"proof":proof,"effectiveIdentity":effective});
        std::fs::write(dir.join("receipt.json"),seal(&after)).unwrap();
        let mut prepared=executor.sdk_job(&prior_id).unwrap()["prepared"].clone();prepared["sdkLocalPreparationRecovery"]=saved.clone();
        executor.store.conn().unwrap().execute("UPDATE jobs SET prepared=?2 WHERE id=?1",params![prior_id,prepared.to_string()]).unwrap();
        executor.store.event(&prior_id,"sdk_local_preparation_recovery_reserved",&saved).unwrap();
        let original=executor.request_snapshot(&request).unwrap();let c=executor.store.conn().unwrap();
        let prior=predecessor(&executor.store,&c,&run,&step_key(&prior_request)).unwrap().unwrap();
        let mut snapshot=original.clone();snapshot["executorIdentity"]=effective;
        snapshot[FIELD]=super::seal(json!({"policy":LOCAL_RECOVERY_POLICY,"workflowId":run,"nodeKey":step_key(&request),"stage":request.stage,"domainRequestSha256":hash(&domain(&original).unwrap()),
            "parentSnapshotSha256":hash(&parent),"originalSettingsSha256":hash(&executor.settings),"originalIdentity":executor.settings["textSdkIdentity"],"effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),
            "resourceHashes":template[FIELD]["resourceHashes"],"predecessor":prior,"modelCalls":0,"accountModelChecked":true}));
        (temp,executor,request,snapshot,prior_id)
    }
    #[test]
    fn sdk_local_recovery_completed_original_allows_only_absent_next_workflow_node() {
        let (_temp,executor,request,snapshot,prior)=recovered_chain_fixture();
        let original=executor.sdk_job(&prior).unwrap();let saved=std::fs::read(executor.store.root.join("text-runs").join(&prior).join("sdk-attempt.json")).unwrap();
        assert!(executor.sdk_raw(&original,&original["snapshot"]).is_ok());
        let next=executor.store.add_sdk_handoff_node_atomic(&request,&snapshot,&executor.settings).unwrap();
        assert_eq!(executor.sdk_job(&prior).unwrap(),original);
        assert_eq!(std::fs::read(executor.store.root.join("text-runs").join(&prior).join("sdk-attempt.json")).unwrap(),saved);
        assert_eq!(executor.store.job(&next).unwrap()["prepared"]["sdkDispatchStarted"],false);
        assert!(executor.store.add_sdk_handoff_node_atomic(&request,&snapshot,&executor.settings).is_err());
    }
    #[test]
    fn sdk_wrapper_repair_unknown_to_receive_raw_next_node_and_reopen_preserves_original_chain() {
        use crate::text_transport::{Observation,ObservationKind};
        let (_temp,executor,request,mut next_snapshot,prior)=recovered_chain_fixture();
        let job=executor.sdk_job(&prior).unwrap();let snapshot=job["snapshot"].clone();
        let prior_request:TextRequest=serde_json::from_value(snapshot.clone()).unwrap();
        let dir=executor.store.root.join("sdk-worker").join(&prior);let file=dir.join("receipt.json");
        let mut record:Value=serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();record.as_object_mut().unwrap().remove("receiptEnvelopeSha256");
        let raw=record["rawResult"].clone();
        let effective=next_snapshot["executorIdentity"].clone();let mut failed_identity=effective.clone();
        failed_identity["bridgeSha256"]=json!(crate::text_sdk::WRAPPER_FAILED_BRIDGE_SHA);failed_identity["supervisorSha256"]=json!(crate::text_sdk::WRAPPER_FAILED_SUPERVISOR_SHA);
        assert_ne!(failed_identity["bridgeSha256"],effective["bridgeSha256"]);
        let mut prepared=job["prepared"].clone();
        prepared["sdkLocalPreparationRecovery"]["effectiveIdentity"]=failed_identity.clone();
        prepared["sdkLocalPreparationRecovery"]["proof"]["effectiveIdentitySha256"]=json!(hash(&failed_identity));
        let v1=prepared["sdkLocalPreparationRecovery"].clone();
        executor.store.conn().unwrap().execute("UPDATE events SET data=?2 WHERE job_id=?1 AND type='sdk_local_preparation_recovery_reserved'",params![prior,v1.to_string()]).unwrap();
        std::fs::write(dir.join("local-preparation-recovery.json"),v1["proof"].to_string()).unwrap();
        record["preDispatchRecovery"]=json!({"proof":v1["proof"],"effectiveIdentity":failed_identity});
        record["state"]=json!("dispatching");record["rawResult"]=Value::Null;record["rawResultSha256"]=Value::Null;
        std::fs::create_dir(dir.join("native-runtime")).unwrap();
        std::fs::write(dir.join("native-runtime/runtime-plan.json"),json!({"identity":failed_identity,"attemptId":prior}).to_string()).unwrap();
        let mut saved=crate::text_sdk::test_append_wrapper_repair(&dir,&record,&effective);
        saved["originalSettingsSha256"]=json!(hash(&executor.settings));saved["resourceHashes"]=next_snapshot[FIELD]["resourceHashes"].clone();
        prepared["sdkWrapperPreparationRepair"]=saved.clone();
        executor.store.event(&prior,"sdk_wrapper_preparation_repair_reserved",&saved).unwrap();
        record["wrapperPreparationRepair"]=json!({"proof":saved["proof"],"effectiveIdentity":effective});
        let attempt_file=executor.store.root.join("text-runs").join(&prior).join("sdk-attempt.json");
        let mut attempt=FrozenAttempt::load(&attempt_file).unwrap();attempt.state=AttemptState::SubmissionUnknown;attempt.raw_result=None;attempt.last_sequence=1;attempt.persist(&attempt_file).unwrap();
        let frozen_request=attempt.request.clone();let original_id=attempt.attempt_id.clone();
        executor.store.conn().unwrap().execute("UPDATE jobs SET state='submission_unknown',prepared=?2,result=NULL,receipt=NULL WHERE id=?1",params![prior,prepared.to_string()]).unwrap();
        executor.store.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1 AND type IN ('text_received_raw','sdk_raw_result')",[&prior]).unwrap();
        executor.store.conn().unwrap().execute("UPDATE workflow_steps SET state='running',result=NULL,error=NULL WHERE text_job_id=?1",[&prior]).unwrap();
        let archive=std::fs::read(dir.join("wrapper-preparation-original.json")).unwrap();let old_intent=std::fs::read(dir.join("local-preparation-recovery.json")).unwrap();
        record["state"]=json!("transport_completed");record["rawResult"]=raw.clone();record["rawResultSha256"]=json!(hash(&raw));
        let text=record.to_string();std::fs::write(&file,format!("{},\"receiptEnvelopeSha256\":\"{}\"}}",&text[..text.len()-1],hash(&record))).unwrap();
        attempt.observe(Observation{attempt_id:prior.clone(),sequence:2,kind:ObservationKind::Completed,thread_id:None,turn_id:None,raw_result:Some(raw)}).unwrap();
        crate::text_sdk::recovered_receipt_bound(&executor.store.root,&attempt,&v1).unwrap();
        crate::text_sdk::repaired_receipt_bound(&executor.store.root,&attempt,&saved).unwrap();attempt.persist(&attempt_file).unwrap();
        let received=executor.sdk_receive(&prior_request,&snapshot,&prior,&attempt).unwrap();
        executor.sdk_commit(&prior_request,&received,None).unwrap();
        let accepted=executor.sdk_job(&prior).unwrap();assert_eq!(executor.sdk_raw(&accepted,&snapshot).unwrap(),received);
        assert_eq!(attempt.attempt_id,original_id);assert_eq!(json!(attempt.request),json!(frozen_request));assert_eq!(accepted["prepared"]["sdkDispatchStarted"],true);
        let predecessor=predecessor(&executor.store,&executor.store.conn().unwrap(),executor.workflow_id.as_deref().unwrap(),&step_key(&prior_request)).unwrap().unwrap();
        let mut next_proof=next_snapshot[FIELD].clone();next_proof.as_object_mut().unwrap().remove("sha256");next_proof["predecessor"]=predecessor;next_snapshot[FIELD]=super::seal(next_proof);
        let next=executor.store.add_sdk_handoff_node_atomic(&request,&next_snapshot,&executor.settings).unwrap();
        complete(&executor,&request,&next_snapshot,&next,true);
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        for id in [&prior,&next] {let job=reopened.sdk_job(id).unwrap();assert!(reopened.sdk_raw(&job,&job["snapshot"]).is_ok());}
        assert_eq!(std::fs::read(dir.join("wrapper-preparation-original.json")).unwrap(),archive);assert_eq!(std::fs::read(dir.join("local-preparation-recovery.json")).unwrap(),old_intent);
        assert!(executor.store.add_sdk_handoff_node_atomic(&request,&next_snapshot,&executor.settings).is_err());
        let count:i64=reopened.store.conn().unwrap().query_row("SELECT count(*) FROM events WHERE job_id=?1 AND type='sdk_wrapper_preparation_repair_reserved'",[&prior],|r|r.get(0)).unwrap();assert_eq!(count,1);
    }
    #[test]
    fn sdk_local_recovery_next_node_rejects_unknown_prior_or_missing_durable_recovery_anchor() {
        for mutation in ["unknown","missing-event","archive-changed","account"] {
            let (_temp,executor,request,mut snapshot,prior)=recovered_chain_fixture();
            match mutation {
                "unknown"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",[&prior]).unwrap();},
                "missing-event"=>{executor.store.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1 AND type='sdk_local_preparation_recovery_reserved'",[&prior]).unwrap();},
                "archive-changed"=>std::fs::write(executor.store.root.join("sdk-worker").join(&prior).join("local-preparation-original.json"),"{}").unwrap(),
                "account"=>snapshot["executorIdentity"]["accountFingerprint"]=json!("f".repeat(64)),_=>unreachable!(),
            }
            let before=evidence(&executor);
            assert!(executor.store.add_sdk_handoff_node_atomic(&request,&snapshot,&executor.settings).is_err(),"{mutation}");
            assert_eq!(evidence(&executor),before);
        }
    }
    fn evidence(executor: &SavedTextExecutor) -> Value {
        let c = executor.store.conn().unwrap();
        let mut q = c
            .prepare("SELECT id,snapshot,state,prepared,result,receipt FROM jobs ORDER BY rowid")
            .unwrap();
        let jobs = q
            .query_map([], |r| {
                Ok(json!([
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?
                ]))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();
        let mut q=c.prepare("SELECT workflow_id,node_key,stage,request_hash,state,text_job_id,result,error FROM workflow_steps ORDER BY rowid").unwrap();
        let steps = q
            .query_map([], |r| {
                Ok(json!([
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?
                ]))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();
        json!({"jobs":jobs,"steps":steps})
    }
    fn add(
        executor: &SavedTextExecutor,
        request: &TextRequest,
        snapshot: &Value,
    ) -> Result<String> {
        executor
            .store
            .add_sdk_handoff_node_atomic(request, snapshot, &executor.settings)
    }

    fn first_node_fixture() -> (tempfile::TempDir, SavedTextExecutor, TextRequest, Value) {
        let (temp,mut executor,request,mut snapshot)=fixture();
        let parent=executor.store.job(executor.workflow_id.as_deref().unwrap()).unwrap()["snapshot"].clone();
        let run=executor.store.add_job("workflow","storyboard.refine","local","local",&parent,"failed").unwrap();
        executor.workflow_id=Some(run.clone());
        let original=executor.request_snapshot(&request).unwrap();
        let (policy,prior)=executor.sdk_handoff_predecessor(&original,false).unwrap();
        assert_eq!(policy,FIRST_NODE_POLICY);
        snapshot[FIELD]["policy"]=json!(policy);
        snapshot[FIELD]["workflowId"]=json!(run);
        snapshot[FIELD]["parentSnapshotSha256"]=json!(hash(&parent));
        snapshot[FIELD]["predecessor"]=prior;
        snapshot[FIELD].as_object_mut().unwrap().remove("sha256");
        snapshot[FIELD]=seal(snapshot[FIELD].clone());
        assert!(proof_valid(&snapshot,&executor.settings,&run,&step_key(&request)));
        (temp,executor,request,snapshot)
    }

    fn first_node_stored_evidence(executor:&SavedTextExecutor)->Value {
        let c=executor.store.conn().unwrap();
        let mut query=c.prepare("SELECT id,job_id,type,data FROM events ORDER BY id").unwrap();
        let events=query.query_map([],|r|Ok(json!([r.get::<_,i64>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?]))).unwrap().map(|r|r.unwrap()).collect::<Vec<_>>();
        json!({"stored":evidence(executor),"events":events})
    }

    #[tokio::test]
    async fn sdk_first_node_handoff_reserves_once_continues_normal_chain_and_replays_without_dispatch() {
        let (_temp,executor,request,snapshot)=first_node_fixture();
        let run=executor.workflow_id.as_deref().unwrap();
        let parent_before: String=executor.store.conn().unwrap().query_row("SELECT snapshot FROM jobs WHERE id=?1",[run],|r|r.get(0)).unwrap();
        let before=evidence(&executor);
        let first=add(&executor,&request,&snapshot).unwrap();
        assert_eq!(executor.sdk_job(&first).unwrap()["prepared"]["sdkDispatchStarted"],false);
        assert_eq!(executor.request_snapshot(&request).unwrap(),snapshot);
        for (index,job) in before["jobs"].as_array().unwrap().iter().enumerate() {assert_eq!(&evidence(&executor)["jobs"][index],job);}
        let reserved=first_node_stored_evidence(&executor);
        assert!(add(&executor,&request,&snapshot).is_err());
        assert_eq!(first_node_stored_evidence(&executor),reserved);
        complete(&executor,&request,&snapshot,&first,true);
        let mut next=request.clone();next.stage="foundation:HANDOFF_AFTER_FIRST".into();
        let original=executor.request_snapshot(&next).unwrap();
        let (policy,prior)=executor.sdk_handoff_predecessor(&original,false).unwrap();
        assert_eq!(policy,POLICY);assert_eq!(prior["jobId"],first);
        let mut next_snapshot=original.clone();next_snapshot["executorIdentity"]=snapshot["executorIdentity"].clone();
        let mut proof=snapshot[FIELD].clone();proof.as_object_mut().unwrap().remove("sha256");
        proof["policy"]=json!(policy);proof["predecessor"]=prior;proof["nodeKey"]=json!(step_key(&next));proof["stage"]=json!(next.stage);proof["domainRequestSha256"]=json!(hash(&domain(&original).unwrap()));
        next_snapshot[FIELD]=seal(proof);
        let second=add(&executor,&next,&next_snapshot).unwrap();complete(&executor,&next,&next_snapshot,&second,true);
        let final_evidence=first_node_stored_evidence(&executor);
        for _ in 0..2 {
            let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
            assert_eq!(reopened.request_snapshot(&request).unwrap(),snapshot);
            assert_eq!(reopened.execute(request.clone()).await.unwrap(),json!({"ok":true}));
            assert_eq!(reopened.execute(next.clone()).await.unwrap(),json!({"ok":true}));
            assert_eq!(first_node_stored_evidence(&executor),final_evidence);
            assert!(!executor.store.root.join("sdk-worker").exists());
        }
        let parent_after:String=executor.store.conn().unwrap().query_row("SELECT snapshot FROM jobs WHERE id=?1",[run],|r|r.get(0)).unwrap();
        assert_eq!(parent_after,parent_before);
    }

    #[test]
    fn sdk_first_node_handoff_rejects_other_steps_orphan_jobs_all_attempt_states_and_unknown_submissions() {
        for case in ["step-running","step-rejected","orphan-job","orphan-proof","same-domain","prepared-attempt","unknown-attempt","completed-attempt","malformed-attempt","global-unknown"] {
            let (_temp,executor,request,snapshot)=first_node_fixture();
            let run=executor.workflow_id.as_deref().unwrap();
            match case {
                "step-running"|"step-rejected"=>executor.store.checkpoint(run,"unrelated-first-key","other","hash",if case=="step-running" {"running"}else{"rejected"},None,None,None).unwrap(),
                "orphan-job"|"orphan-proof"|"same-domain"|"global-unknown"=>{
                    let old=if case=="same-domain" {executor.request_snapshot(&request).unwrap()} else if case=="orphan-proof" {json!({"stage":"other","sdkSupervisorHandoff":{"workflowId":run}})} else {json!({"stage":"other"})};
                    let job=executor.store.add_job("text","orphan","codex_sdk","synthetic",&old,if case=="global-unknown" {"submission_unknown"}else{"queued"}).unwrap();
                    if case=="orphan-job"||case=="global-unknown" {
                        let prepared=if case=="orphan-job" {json!({"sdkOriginWorkflowId":run,"sdkDispatchStarted":false})} else {json!({"sdkDispatchStarted":true})};
                        executor.store.conn().unwrap().execute("UPDATE jobs SET prepared=?2 WHERE id=?1",params![job,prepared.to_string()]).unwrap();
                    }
                },
                _=>{
                    let path=executor.store.root.join("text-runs/first-orphan/sdk-attempt.json");std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    if case=="malformed-attempt" {std::fs::write(path,"malformed").unwrap();} else {
                        let mut other=request.clone();other.stage="foundation:OTHER_ORPHAN".into();
                        let mut attempt=FrozenAttempt::prepare("first-orphan",run,&step_key(&other),Backend::CodexSdk,other,&executor.settings["textSdkIdentity"]).unwrap();
                        if case=="unknown-attempt" {attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.state=AttemptState::SubmissionUnknown;}
                        if case=="completed-attempt" {attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.state=AttemptState::Completed;}
                        attempt.persist(&path).unwrap();
                    }
                }
            }
            let before=first_node_stored_evidence(&executor);
            assert!(add(&executor,&request,&snapshot).is_err(),"{case}");
            assert_eq!(first_node_stored_evidence(&executor),before,"{case}");
        }
    }

    #[test]
    fn sdk_first_node_handoff_drift_and_invalid_zero_dispatch_proofs_fail_atomically() {
        for case in ["node","runtime","supervisor","old-resource","new-resource","parent","model","reasoning","account","zero-count","local-recovery"] {
            let (temp,executor,mut request,mut snapshot)=first_node_fixture();
            match case {
                "node"|"runtime"|"supervisor"|"old-resource"|"new-resource"=>{
                    let file=match case {"node"=>"node.exe","runtime"=>"codex.exe","supervisor"=>"new-supervisor.exe","old-resource"=>"old-resources/src/main.mjs",_=>"new-resources/src/bridge.mjs"};
                    std::fs::write(temp.path().join(file),"changed").unwrap();
                },
                "parent"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET snapshot='{}' WHERE id=?1",[executor.workflow_id.as_ref().unwrap()]).unwrap();},
                "model"=>request.model="different-model".into(),
                "reasoning"=>request.reasoning="high".into(),
                "account"=>snapshot["executorIdentity"]["accountFingerprint"]=json!("c".repeat(64)),
                "zero-count"=>{snapshot[FIELD]["predecessor"]["attempts"]=json!(1);snapshot[FIELD].as_object_mut().unwrap().remove("sha256");snapshot[FIELD]=seal(snapshot[FIELD].clone());},
                "local-recovery"=>{assert!(executor.sdk_handoff_predecessor(&executor.request_snapshot(&request).unwrap(),true).is_err());snapshot[FIELD]["policy"]=json!(LOCAL_RECOVERY_POLICY);},
                _=>unreachable!(),
            }
            let before=first_node_stored_evidence(&executor);
            assert!(add(&executor,&request,&snapshot).is_err(),"{case}");assert_eq!(first_node_stored_evidence(&executor),before,"{case}");
        }
    }

    #[test]
    fn sdk_first_node_handoff_saved_proof_event_and_parent_must_remain_bound() {
        for case in ["event","prepared","parent","proof"] {
            let (_temp,executor,request,snapshot)=first_node_fixture();let job=add(&executor,&request,&snapshot).unwrap();let c=executor.store.conn().unwrap();
            match case {
                "event"=>{c.execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_supervisor_handoff_frozen'",[&job]).unwrap();},
                "prepared"=>{c.execute("UPDATE jobs SET prepared='{}' WHERE id=?1",[&job]).unwrap();},
                "parent"=>{c.execute("UPDATE jobs SET snapshot='{}' WHERE id=?1",[executor.workflow_id.as_ref().unwrap()]).unwrap();},
                "proof"=>{let mut changed=snapshot.clone();changed[FIELD]["predecessor"]["attempts"]=json!(1);c.execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![job,changed.to_string()]).unwrap();},
                _=>unreachable!(),
            }
            let before=first_node_stored_evidence(&executor);assert!(executor.request_snapshot(&request).is_err(),"{case}");assert_eq!(first_node_stored_evidence(&executor),before);
        }
    }

    #[test]
    fn sdk_supervisor_handoff_only_three_path_identity_fields_can_change() {
        let (_temp, executor, _request, snapshot) = fixture();
        let old = &executor.settings["textSdkIdentity"];
        let new = &snapshot["executorIdentity"];
        assert!(crate::text_sdk::supervisor_handoff_identity_bound(old, new));
        assert!(!crate::text_sdk::supervisor_handoff_identity_bound(
            old, old
        ));
        for field in [
            "sdkPackage",
            "sdkVersion",
            "sdkImplementationSha256",
            "bridgeVersion",
            "bridgeSha256",
            "protocolVersion",
            "nodeExecutable",
            "nodeSha256",
            "nodeVersion",
            "runtimeExecutable",
            "runtimeSha256",
            "runtimeIdentity",
            "runtimePackageVersion",
            "permissionProfile",
            "accountFingerprint",
            "credentialHome",
            "codexHome",
        ] {
            let mut changed = new.clone();
            changed[field] = json!("changed");
            assert!(
                !crate::text_sdk::supervisor_handoff_identity_bound(old, &changed),
                "{field}"
            );
        }
    }
    #[test]
    fn sdk_supervisor_handoff_new_node_atomically_freezes_proof_and_preserves_parent() {
        let (_temp, executor, request, snapshot) = fixture();
        let before = evidence(&executor);
        let job = add(&executor, &request, &snapshot).unwrap();
        assert_eq!(executor.store.job(&job).unwrap()["snapshot"], snapshot);
        assert_eq!(
            executor.store.job(&job).unwrap()["prepared"][FIELD],
            snapshot[FIELD]
        );
        assert_eq!(executor.request_snapshot(&request).unwrap(), snapshot);
        let after = evidence(&executor);
        assert_eq!(&after["jobs"][0], &before["jobs"][0]);
        assert_eq!(&after["jobs"][1], &before["jobs"][1]);
        assert_eq!(&after["steps"][0], &before["steps"][0]);
        assert_eq!(after["jobs"].as_array().unwrap().len(), 3);
        assert_eq!(after["steps"].as_array().unwrap().len(), 2);
        assert_eq!(
            executor.sdk_handoff_existing_domain(&snapshot).unwrap(),
            Some(executor.request_domain_snapshot(&request).unwrap())
        );
        let before_repeat = evidence(&executor);
        assert!(add(&executor, &request, &snapshot).is_err());
        assert_eq!(evidence(&executor), before_repeat);
        // The actual upgrade case replaces bytes at the same candidate path.
        let (_temp, executor, request, mut snapshot) = fixture();
        let old_path = s(
            &executor.settings["textSdkIdentity"],
            "supervisorExecutable",
        );
        std::fs::write(old_path, "same-path-new-supervisor").unwrap();
        snapshot["executorIdentity"]["supervisorExecutable"] = json!(old_path);
        snapshot["executorIdentity"]["supervisorSha256"] =
            json!(crate::store::hash_file(Path::new(old_path)).unwrap().0);
        snapshot[FIELD]["effectiveIdentitySha256"] = json!(hash(&snapshot["executorIdentity"]));
        snapshot[FIELD].as_object_mut().unwrap().remove("sha256");
        snapshot[FIELD] = seal(snapshot[FIELD].clone());
        assert!(add(&executor, &request, &snapshot).is_ok());
    }
    #[test]
    fn sdk_supervisor_handoff_existing_unknown_or_orphan_node_never_creates_another_task() {
        for case in ["step", "job", "unknown", "attempt"] {
            let (_temp, executor, request, snapshot) = fixture();
            match case {
                "step" => executor
                    .store
                    .checkpoint(
                        executor.workflow_id.as_deref().unwrap(),
                        &step_key(&request),
                        &request.stage,
                        &hash(&snapshot),
                        "running",
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                "job" => {
                    executor
                        .store
                        .add_job(
                            "text",
                            "existing",
                            "codex_sdk",
                            "synthetic",
                            &executor.request_snapshot(&request).unwrap(),
                            "queued",
                        )
                        .unwrap();
                }
                "unknown" => {
                    let job=executor.store.add_job("text","unresolved-different-node","codex_sdk","synthetic",&json!({"stage":"other","executorIdentity":executor.settings["textSdkIdentity"]}),"submission_unknown").unwrap();
                    executor
                        .store
                        .conn()
                        .unwrap()
                        .execute(
                            "UPDATE jobs SET prepared=?1 WHERE id=?2",
                            params![json!({"sdkDispatchStarted":true}).to_string(), job],
                        )
                        .unwrap();
                }
                "attempt" => {
                    let path = executor
                        .store
                        .root
                        .join("text-runs/orphan/sdk-attempt.json");
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    FrozenAttempt::prepare(
                        "orphan",
                        executor.workflow_id.as_deref().unwrap(),
                        &step_key(&request),
                        Backend::CodexSdk,
                        request.clone(),
                        &executor.settings["textSdkIdentity"],
                    )
                    .unwrap()
                    .persist(&path)
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let before = evidence(&executor);
            assert!(add(&executor, &request, &snapshot).is_err(), "{case}");
            assert_eq!(evidence(&executor), before, "{case}");
        }
    }
    #[test]
    fn sdk_supervisor_handoff_drift_in_actual_program_or_resource_bytes_is_atomic_failure() {
        for changed in [
            "node.exe",
            "codex.exe",
            "new-supervisor.exe",
            "new-resources/src/bridge.mjs",
            "old-resources/src/main.mjs",
        ] {
            let (temp, executor, request, snapshot) = fixture();
            std::fs::write(temp.path().join(changed), "tampered").unwrap();
            let before = evidence(&executor);
            assert!(add(&executor, &request, &snapshot).is_err(), "{changed}");
            assert_eq!(evidence(&executor), before);
        }
    }
    #[tokio::test]
    async fn sdk_supervisor_handoff_twice_reopened_completed_node_reads_original_raw_without_worker(
    ) {
        let (temp, executor, request, snapshot) = fixture();
        let job = add(&executor, &request, &snapshot).unwrap();
        complete(&executor, &request, &snapshot, &job, true);
        let before = evidence(&executor);
        let raw_hash = crate::store::hash_file(
            &executor
                .store
                .root
                .join("text-runs")
                .join(&job)
                .join("result.json"),
        )
        .unwrap();
        std::fs::write(
            temp.path().join("new-supervisor.exe"),
            "later-supervisor-upgrade",
        )
        .unwrap();
        for _ in 0..2 {
            let reopened = SavedTextExecutor {
                store: executor.store.clone(),
                settings: executor.settings.clone(),
                workflow_id: executor.workflow_id.clone(),
            };
            assert_eq!(reopened.request_snapshot(&request).unwrap(), snapshot);
            let raw = reopened.execute(request.clone()).await.unwrap();
            assert_eq!(raw, json!({"ok":true}));
            reopened.accept(&request, &raw).await.unwrap();
            assert_eq!(evidence(&executor), before);
            assert!(!executor.store.root.join("sdk-worker").exists());
            assert_eq!(
                crate::store::hash_file(
                    &executor
                        .store
                        .root
                        .join("text-runs")
                        .join(&job)
                        .join("result.json")
                )
                .unwrap(),
                raw_hash
            );
        }
    }
    #[tokio::test]
    async fn sdk_supervisor_handoff_missing_or_unknown_original_attempt_is_never_redispatched() {
        let (_temp, executor, request, snapshot) = fixture();
        let job = add(&executor, &request, &snapshot).unwrap();
        let before = evidence(&executor);
        assert!(executor.execute(request.clone()).await.is_err());
        assert!(executor.execute(request.clone()).await.is_err());
        assert_eq!(evidence(&executor), before);
        assert!(!executor.store.root.join("sdk-worker").exists());
        let path = executor
            .store
            .root
            .join("text-runs")
            .join(&job)
            .join("sdk-attempt.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut attempt = FrozenAttempt::prepare(
            &job,
            executor.workflow_id.as_deref().unwrap(),
            &step_key(&request),
            Backend::CodexSdk,
            request.clone(),
            &snapshot["executorIdentity"],
        )
        .unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        attempt.state = AttemptState::SubmissionUnknown;
        attempt.persist(&path).unwrap();
        executor
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                [&job],
            )
            .unwrap();
        let before = evidence(&executor);
        let frozen = crate::store::hash_file(&path).unwrap();
        assert!(executor.execute(request.clone()).await.is_err());
        assert_eq!(evidence(&executor), before);
        assert_eq!(crate::store::hash_file(&path).unwrap(), frozen);
    }
    #[test]
    fn sdk_supervisor_handoff_proof_parent_model_event_and_previous_receipt_tampering_are_held() {
        for changed in [
            "model",
            "schema",
            "proof",
            "parent",
            "predecessor",
            "frozen-event",
            "prepared",
        ] {
            let (_temp, mut executor, mut request, snapshot) = fixture();
            let job = add(&executor, &request, &snapshot).unwrap();
            let c = executor.store.conn().unwrap();
            match changed {
                "model" => {
                    request.model = "gpt-other".into();
                }
                "schema" => {
                    request.schema = json!({"type":"string"});
                }
                "proof" => {
                    let mut changed = snapshot.clone();
                    changed[FIELD]["modelCalls"] = json!(1);
                    c.execute(
                        "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                        params![changed.to_string(), job],
                    )
                    .unwrap();
                }
                "parent" => {
                    c.execute(
                        "UPDATE jobs SET snapshot='{}' WHERE id=?1",
                        [executor.workflow_id.as_ref().unwrap()],
                    )
                    .unwrap();
                }
                "predecessor" => {
                    c.execute(
                        "UPDATE jobs SET receipt='{}' WHERE id=?1",
                        [s(&snapshot[FIELD]["predecessor"], "jobId")],
                    )
                    .unwrap();
                }
                "frozen-event" => {
                    c.execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_supervisor_handoff_frozen'",[&job]).unwrap();
                }
                "prepared" => {
                    c.execute("UPDATE jobs SET prepared='{}' WHERE id=?1", [&job])
                        .unwrap();
                }
                _ => unreachable!(),
            }
            drop(c);
            let before = evidence(&executor);
            assert!(executor.request_snapshot(&request).is_err(), "{changed}");
            assert_eq!(evidence(&executor), before);
            executor.settings["textModel"] = json!("gpt-other");
            assert!(executor.sdk_handoff_existing_domain(&snapshot).is_err());
        }
    }
    #[tokio::test]
    async fn sdk_supervisor_handoff_headless_does_not_reprobe_account_or_enable_execution() {
        let (_temp, executor, _request, _snapshot) = fixture();
        assert!(!crate::text_sdk::native_execution_enabled());
        let before = evidence(&executor);
        assert!(
            crate::text_sdk::prepare_supervisor_handoff(&executor.settings)
                .await
                .unwrap_err()
                .contains("SDK_LIVE_DISABLED")
        );
        assert_eq!(evidence(&executor), before);
    }
}
