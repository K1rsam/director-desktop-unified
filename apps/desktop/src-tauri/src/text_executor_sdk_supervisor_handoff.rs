//! Only an absent later SDK node can bind a replacement native supervisor.
//! Existing requests, raw results and parent settings are never rewritten.
use super::*;
use crate::text_transport::FrozenAttempt;

const POLICY: &str = "sdk-new-node-supervisor-handoff/v1";
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
fn proof_valid(snapshot: &Value, settings: &Value, run: &str, key: &str) -> bool {
    let proof = &snapshot[FIELD];
    let Some(fields) = proof.as_object() else {
        return false;
    };
    if fields.len() != 14 {
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
    proof["policy"] == POLICY
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
        && crate::text_sdk::supervisor_handoff_identity_bound(
            &proof["originalIdentity"],
            &snapshot["executorIdentity"],
        )
        && proof["resourceHashes"].as_object().is_some_and(|h| {
            h.len() == 9
                && h.values().all(|v| {
                    v.as_str()
                        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                })
        })
        && proof["predecessor"]
            .as_object()
            .is_some_and(|p| p.len() == 14)
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
fn predecessor_bound(store: &Store, c: &rusqlite::Connection, proof: &Value) -> Result<bool> {
    Ok(predecessor(
        store,
        c,
        s(proof, "workflowId"),
        s(&proof["predecessor"], "nodeKey"),
    )?
    .as_ref()
        == Some(&proof["predecessor"]))
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
        if domain(&prior)? == expected {
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
            || !predecessor_bound(&self.store, &c, &saved[FIELD])?
            || created.as_deref() != Some(saved[FIELD].to_string().as_str())
        {
            return Err(HELD.into());
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
        let Some(verified) = crate::text_sdk::prepare_supervisor_handoff(&self.settings).await?
        else {
            return Ok(original.clone());
        };
        self.sdk_parent_binding()?;
        self.sdk_request_policy(request)?;
        let c = self.store.conn()?;
        let previous_key: Option<String> = c.query_row("SELECT w.node_key FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.state='accepted' AND j.kind='text' AND j.provider='codex_sdk' AND j.state='completed' ORDER BY w.updated_at DESC,w.rowid DESC LIMIT 1",[run],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let previous_key = previous_key.ok_or(HELD)?;
        let prior = predecessor(&self.store, &c, run, &previous_key)?.ok_or(HELD)?;
        let previous_job = self.sdk_job(s(&prior, "jobId"))?;
        if previous_job["snapshot"]["model"] != self.settings["textModel"]
            || previous_job["snapshot"]["reasoning"] != self.settings["reasoning"]
            || (previous_job["snapshot"]["executorIdentity"] != original["executorIdentity"]
                && previous_job["snapshot"].get(FIELD).is_none())
        {
            return Err(HELD.into());
        }
        // Reuse the genuine original raw/attempt/receipt verifier, including an
        // accepted zero-call F2 replay. No receipt is inferred from a status label.
        self.sdk_handoff_job_bound(&previous_job, &previous_job["snapshot"])?;
        self.sdk_raw(&previous_job, &previous_job["snapshot"])?;
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
            json!({"policy":POLICY,"workflowId":run,"nodeKey":step_key(request),"stage":request.stage,
            "domainRequestSha256":hash(&domain(original)?),"parentSnapshotSha256":crate::pipeline::digest(&original_parent),
            "originalSettingsSha256":hash(&self.settings),"originalIdentity":original["executorIdentity"],
            "effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":verified.resource_hashes,
            "predecessor":prior,"modelCalls":0,"accountModelChecked":true}),
        );
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
        if !parent_bound(&tx, proof, original_settings)? || !predecessor_bound(self, &tx, proof)? {
            return Err(HELD.into());
        }
        no_node_or_attempt(self, &tx, run, &key, snapshot)?;
        crate::text_sdk::verify_supervisor_handoff_bytes(
            &proof["originalIdentity"],
            &snapshot["executorIdentity"],
            &proof["resourceHashes"],
        )?;
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
