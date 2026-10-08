//! Exact revalidation of one rejected author transport regression, never a retry.
use super::*;

pub(crate) const STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY: &str =
    "storyboard-author-scoped-shot-id-replay/v1";
pub(crate) const STORYBOARD_AUTHOR_SHOT_REJECTION: &str = "模型镜号重复";

pub(crate) fn scoped_author_replay_stage(stage: &str, skill: &str, profile: &Value) -> bool {
    stage == "storyboardCreate"
        && skill == "unit-performance-director"
        && profile == "original-production-contract/v1"
}

/// The Store independently checks this proof before its compare-and-swap.
/// Neither today's rules nor a normalized result can supply missing original bytes.
pub(crate) fn scoped_author_replay_audit_valid(audit: &Value) -> bool {
    let Some(fields) = audit.as_object() else {
        return false;
    };
    if fields.len() != 18
        || audit["policy"] != STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY
        || audit["stage"] != "storyboardCreate"
        || audit["previousError"] != STORYBOARD_AUTHOR_SHOT_REJECTION
        || audit["generationRequestsSent"] != 0
        || audit["acceptedAutomatically"] != false
        || [
            "workflowId",
            "nodeKey",
            "originalJobId",
            "oldRequestHash",
            "newRequestHash",
        ]
        .iter()
        .any(|field| s(audit, field).is_empty())
    {
        return false;
    }
    let snapshot_text = s(audit, "originalSnapshotJson");
    let raw_text = s(audit, "originalResultJson");
    let parent_text = s(audit, "originalWorkflowSnapshotJson");
    let Ok(snapshot) = serde_json::from_str::<Value>(snapshot_text) else {
        return false;
    };
    let Ok(request) = serde_json::from_value::<TextRequest>(snapshot.clone()) else {
        return false;
    };
    let Ok(raw) = serde_json::from_str::<Value>(raw_text) else {
        return false;
    };
    let Ok(parent) = serde_json::from_str::<Value>(parent_text) else {
        return false;
    };
    let Ok(receipt) = serde_json::from_str::<Value>(s(audit, "originalReceiptJson")) else {
        return false;
    };
    let Ok(event) = serde_json::from_str::<Value>(s(audit, "originalRawEventDataJson")) else {
        return false;
    };
    let event_id = audit["originalRawEventId"].as_i64().unwrap_or(0);
    scoped_author_replay_stage(
        &request.stage,
        &request.skill,
        &request.input["storyboardCreativeProfile"],
    ) && snapshot["executorIdentity"]["provider"] == "codex"
        && snapshot.to_string() == snapshot_text
        && audit["oldRequestHash"] == crate::pipeline::digest(snapshot_text)
        && audit["newRequestHash"] == audit["oldRequestHash"]
        && audit["originalResultSha256"] == crate::pipeline::digest(raw_text)
        && audit["originalWorkflowSnapshotSha256"] == crate::pipeline::digest(parent_text)
        && parent["stage"] == "storyboard.run"
        && frozen_text_parent_matches("storyboard.run", &parent, &request)
        && audit["nodeKey"] == step_key(&request)
        && event_id > 0
        && event["requestHash"] == audit["oldRequestHash"]
        && event["value"] == raw
        && source_coverage_replay_receipt_valid(
            &receipt,
            snapshot_text,
            raw_text,
            event_id,
            s(audit, "oldRequestHash"),
            s(audit, "workflowId"),
            s(audit, "nodeKey"),
        )
}

struct ScopedAuthorReplay {
    frozen: FrozenTextReplay,
    accepted: Option<Value>,
    error: Option<String>,
}

struct Evidence {
    job: String,
    raw: String,
    snapshot: String,
    request_hash: String,
    receipt: String,
    event_id: i64,
    event_data: String,
    parent_snapshot: String,
    step_state: String,
    job_state: String,
    step_result: String,
    step_error: Option<String>,
    job_error: Option<String>,
    stage: String,
}

fn replay_evidence(
    connection: &rusqlite::Connection,
    run: &str,
    key: &str,
) -> rusqlite::Result<Evidence> {
    connection.query_row(
        "SELECT j.id,j.result,j.snapshot,w.request_hash,j.receipt,e.id,e.data,parent.snapshot,w.state,j.state,w.result,w.error,j.error,w.stage FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id JOIN jobs parent ON parent.id=w.workflow_id JOIN events e ON e.id=(SELECT MAX(x.id) FROM events x WHERE x.job_id=j.id AND x.type='text_received_raw') WHERE w.workflow_id=?1 AND w.node_key=?2 AND parent.kind='workflow' AND parent.entity_id='storyboard.run' AND j.kind='text' AND j.provider='codex'",
        params![run,key], |row| Ok(Evidence {job:row.get(0)?,raw:row.get(1)?,snapshot:row.get(2)?,request_hash:row.get(3)?,receipt:row.get(4)?,event_id:row.get(5)?,event_data:row.get(6)?,parent_snapshot:row.get(7)?,step_state:row.get(8)?,job_state:row.get(9)?,step_result:row.get(10)?,step_error:row.get(11)?,job_error:row.get(12)?,stage:row.get(13)?}),
    )
}

fn migrated_receipt_bound(evidence: &Evidence, run: &str, key: &str, receipt: &Value) -> bool {
    let audit = &receipt["migrationAudit"];
    let accepted = receipt["validation"] == "accepted";
    scoped_author_replay_audit_valid(audit)
        && receipt
            .as_object()
            .is_some_and(|fields| fields.len() == if accepted { 8 } else { 7 })
        && receipt["migrationPolicy"] == STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY
        && receipt["contract"] == "desktop-runner/v1"
        && receipt["revalidatedFrom"] == "rejected"
        && receipt["previousValidationError"] == STORYBOARD_AUTHOR_SHOT_REJECTION
        && audit["originalJobId"] == evidence.job
        && audit["workflowId"] == run
        && audit["nodeKey"] == key
        && audit["stage"] == evidence.stage
        && audit["oldRequestHash"] == evidence.request_hash
        && audit["originalSnapshotJson"] == evidence.snapshot
        && audit["originalResultJson"] == evidence.raw
        && audit["originalWorkflowSnapshotJson"] == evidence.parent_snapshot
        && audit["originalRawEventId"] == evidence.event_id
        && audit["originalRawEventDataJson"] == evidence.event_data
        && receipt["validatedResultSha256"] == crate::pipeline::digest(&evidence.step_result)
        && if accepted {
            evidence.step_state == "accepted"
                && evidence.job_state == "completed"
                && evidence.step_error.is_none()
                && evidence.job_error.is_none()
                && receipt["acceptedRequestHash"] == evidence.request_hash
        } else {
            receipt["validation"] == "rejected"
                && evidence.step_state == "rejected"
                && evidence.job_state == "rejected"
                && evidence.job_error == evidence.step_error
                && evidence
                    .job_error
                    .as_deref()
                    .is_some_and(|reason| !reason.is_empty())
                && evidence.step_result == evidence.raw
        }
}

/// Read-only durable origin proof for the existing bounded author-repair chain.
/// A policy name alone is never sufficient to resolve a rejected author.
pub(crate) fn scoped_author_replay_receipt_bound(
    connection: &rusqlite::Connection,
    run: &str,
    node: &str,
) -> Result<bool> {
    let evidence = match replay_evidence(connection, run, node) {
        Ok(evidence) => evidence,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    let Ok(receipt) = serde_json::from_str::<Value>(&evidence.receipt) else {
        return Ok(false);
    };
    Ok(migrated_receipt_bound(&evidence, run, node, &receipt))
}

impl SavedTextExecutor {
    fn scoped_author_replay(
        &self,
        request: &TextRequest,
        current: &Value,
        hash: &str,
    ) -> Result<Option<ScopedAuthorReplay>> {
        if request.stage != "storyboardCreate" {
            return Ok(None);
        }
        let Some(run) = self.workflow_id.as_deref() else {
            return Ok(None);
        };
        let key = step_key(request);
        let connection = self.store.conn()?;
        let held = || {
            "SCOPED_AUTHOR_REPLAY_HELD: 原主创请求、节点、回包、最新接收事件或回执不满足精确回放合同；原证据保留，未重新主创".to_string()
        };
        let protected: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND (w.error=?3 OR j.error=?3 OR json_extract(j.receipt,'$.migrationPolicy')=?4 OR EXISTS(SELECT 1 FROM events e WHERE e.job_id=j.id AND e.type IN ('storyboard_author_shot_validation_replay_accepted','storyboard_author_shot_validation_replay_rejected'))))",
            params![run,key,STORYBOARD_AUTHOR_SHOT_REJECTION,STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY], |row| row.get(0),
        ).map_err(|_| held())?;
        if !protected {
            return Ok(None);
        }
        if s(&self.settings, "textProvider") != "codex"
            || !scoped_author_replay_stage(
                &request.stage,
                &request.skill,
                &request.input["storyboardCreativeProfile"],
            )
            || crate::pipeline::digest(&current.to_string()) != hash
        {
            return Err(held());
        }
        let evidence = replay_evidence(&connection, run, &key).map_err(|_| held())?;
        if evidence.snapshot != current.to_string()
            || evidence.request_hash != hash
            || crate::pipeline::digest(&evidence.snapshot) != hash
            || evidence.stage != request.stage
        {
            return Err(held());
        }
        let value: Value = serde_json::from_str(&evidence.raw).map_err(|_| held())?;
        let receipt: Value = serde_json::from_str(&evidence.receipt).map_err(|_| held())?;
        let audit = if receipt["migrationPolicy"].is_null() {
            if evidence.step_state != "rejected"
                || evidence.job_state != "rejected"
                || evidence.step_error.as_deref() != Some(STORYBOARD_AUTHOR_SHOT_REJECTION)
                || evidence.job_error != evidence.step_error
                || evidence.step_result != evidence.raw
            {
                return Err(held());
            }
            json!({"policy":STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY,"stage":request.stage,"workflowId":run,"nodeKey":key,"originalJobId":evidence.job,"previousError":STORYBOARD_AUTHOR_SHOT_REJECTION,"oldRequestHash":hash,"newRequestHash":hash,"originalSnapshotJson":evidence.snapshot,"originalResultJson":evidence.raw,"originalResultSha256":crate::pipeline::digest(&evidence.raw),"originalReceiptJson":evidence.receipt,"originalRawEventId":evidence.event_id,"originalRawEventDataJson":evidence.event_data,"originalWorkflowSnapshotJson":evidence.parent_snapshot,"originalWorkflowSnapshotSha256":crate::pipeline::digest(&evidence.parent_snapshot),"generationRequestsSent":0,"acceptedAutomatically":false})
        } else {
            let audit = receipt["migrationAudit"].clone();
            if !migrated_receipt_bound(&evidence, run, &key, &receipt) {
                return Err(held());
            }
            audit
        };
        if !scoped_author_replay_audit_valid(&audit) {
            return Err(held());
        }
        let accepted = if evidence.step_state == "accepted" {
            if evidence.job_state != "completed"
                || evidence.job_error.is_some()
                || evidence.step_error.is_some()
                || receipt["validation"] != "accepted"
                || receipt["acceptedRequestHash"] != hash
            {
                return Err(held());
            }
            Some(serde_json::from_str(&evidence.step_result).map_err(|_| held())?)
        } else {
            if evidence.step_state != "rejected"
                || evidence.job_state != "rejected"
                || evidence.job_error != evidence.step_error
                || evidence.job_error.as_deref().is_none_or(str::is_empty)
                || receipt["validation"] != "rejected"
                || evidence.step_result != evidence.raw
            {
                return Err(held());
            }
            None
        };
        Ok(Some(ScopedAuthorReplay {
            frozen: FrozenTextReplay {
                job: evidence.job,
                value,
                audit,
            },
            accepted,
            error: evidence.job_error,
        }))
    }

    pub(super) fn scoped_author_replay_value(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        hash: &str,
    ) -> Result<Option<Value>> {
        self.scoped_author_replay(request, snapshot, hash)
            .map(|record| record.map(|record| record.frozen.value))
    }

    pub(super) fn accept_scoped_author_replay(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        hash: &str,
        value: &Value,
    ) -> Result<bool> {
        let Some(record) = self.scoped_author_replay(request, snapshot, hash)? else {
            return Ok(false);
        };
        if let Some(accepted) = record.accepted {
            if accepted != *value {
                return Err("已采用主创回放的规范化结果变化；原请求和回包保留".into());
            }
            return Ok(true);
        }
        if record.error.as_deref() != Some(STORYBOARD_AUTHOR_SHOT_REJECTION) {
            return Err(
                "原主创完整复验发现领域错误，保持未采用；只能由已有有界整编链处理，禁止重新主创"
                    .into(),
            );
        }
        self.commit_scoped_author_replay(&record.frozen, hash, value, None)?;
        Ok(true)
    }

    pub(super) fn reject_scoped_author_replay(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        hash: &str,
        value: &Value,
        reason: &str,
    ) -> Result<bool> {
        let Some(record) = self.scoped_author_replay(request, snapshot, hash)? else {
            return Ok(false);
        };
        if record.accepted.is_some() {
            return Err("已采用主创回放不能由重复校验改写为拒绝".into());
        }
        if record.frozen.value != *value || reason.is_empty() {
            return Err("主创回放拒绝不是原回包或缺少校验原因；原证据保留".into());
        }
        if record.frozen.audit["originalReceiptJson"]
            != self
                .store
                .conn()?
                .query_row(
                    "SELECT receipt FROM jobs WHERE id=?1",
                    [&record.frozen.job],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|e| e.to_string())?
        {
            if record.error.as_deref() != Some(reason) {
                return Err("主创回放的已保存领域拒绝变化；原拒绝及原稿保留".into());
            }
            return Ok(true);
        }
        self.commit_scoped_author_replay(&record.frozen, hash, value, Some(reason))?;
        Ok(true)
    }

    fn commit_scoped_author_replay(
        &self,
        replay: &FrozenTextReplay,
        hash: &str,
        value: &Value,
        reason: Option<&str>,
    ) -> Result<()> {
        let mut receipt = json!({"validation":if reason.is_some(){"rejected"}else{"accepted"},"contract":"desktop-runner/v1","revalidatedFrom":"rejected","migrationPolicy":STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY,"previousValidationError":replay.audit["previousError"],"migrationAudit":replay.audit,"validatedResultSha256":crate::pipeline::digest(&value.to_string())});
        if reason.is_none() {
            receipt["acceptedRequestHash"] = json!(hash);
        }
        if !self.store.commit_text_replay(
            &replay.audit,
            &json!({"error":reason,"receipt":receipt}),
            value,
        )? {
            return Err(
                "主创原任务、最新接收证据或节点绑定已变化；精确回放未提交，未重新生成".into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        tempfile::TempDir,
        SavedTextExecutor,
        TextRequest,
        String,
        Value,
    ) {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::open(temporary.path().join("project")).unwrap();
        store
            .create("synthetic scoped author replay", "synthetic-project")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        let executable = temporary.path().join(if cfg!(windows) {
            "never-run.exe"
        } else {
            "never-run"
        });
        std::fs::write(
            &executable,
            b"invalid synthetic executable: a replay must never spawn this",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let settings = json!({"textProvider":"codex","codexPath":executable,"textCliVersion":"codex-cli 0.162.0-alpha.2"});
        let rows = json!([
            {"id":"row:synthetic:0:4","text":"甲开门。","range":{"start":0,"end":4},"kind":"action","episodeId":"episode:synthetic","sceneId":"scene:one"},
            {"id":"row:synthetic:5:9","text":"乙坐下。","range":{"start":5,"end":9},"kind":"action","episodeId":"episode:synthetic","sceneId":"scene:two"}
        ]);
        let source = json!({"sha256":crate::pipeline::digest("甲开门。\n乙坐下。"),"text":"甲开门。\n乙坐下。","episodes":[{"id":"episode:synthetic"}],"rows":rows});
        let run=store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":{"source":source,"settings":settings}}),"partially_blocked").unwrap();
        let executor = SavedTextExecutor {
            store: store.clone(),
            settings,
            workflow_id: Some(run.clone()),
        };
        let request = TextRequest {
            stage: "storyboardCreate".into(),
            skill: "unit-performance-director".into(),
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            input: json!({"storyboardCreativeProfile":"original-production-contract/v1","episodeId":"episode:synthetic","executionBatch":{"id":"synthetic-B01","version":"source-production-batch/v1"},"sourceRows":rows}),
            schema: json!({"type":"object"}),
            rule_snapshot: crate::pipeline::snapshot("unit-performance-director", &[]).unwrap(),
        };
        let snapshot = executor.request_snapshot(&request).unwrap();
        let hash = crate::pipeline::digest(&snapshot.to_string());
        let raw = json!({"schemaVersion":"storyboard-draft/v2","units":[
            {"id":"unit-one","episodeId":"episode:synthetic","sourceRefs":["row:synthetic:0:4"],"shots":[{"id":"1","sourceRefs":["row:synthetic:0:4"]}]},
            {"id":"unit-two","episodeId":"episode:synthetic","sourceRefs":["row:synthetic:5:9"],"shots":[{"id":"1","sourceRefs":["row:synthetic:5:9"]}]}
        ],"fantasyRealization":[]});
        let job = store
            .add_job(
                "text",
                "storyboardCreate:synthetic",
                "codex",
                "local",
                &snapshot,
                "rejected",
            )
            .unwrap();
        store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":hash,"value":raw}),
            )
            .unwrap();
        store.update_job(&job,"rejected",&json!({"error":STORYBOARD_AUTHOR_SHOT_REJECTION,"result":raw,"receipt":{"validation":"rejected","contract":"desktop-runner/v1"}})).unwrap();
        store
            .checkpoint(
                &run,
                &step_key(&request),
                &request.stage,
                &hash,
                "rejected",
                Some(&job),
                Some(&raw),
                Some(STORYBOARD_AUTHOR_SHOT_REJECTION),
            )
            .unwrap();
        let raw_dir = store.root.join("raw").join(&job);
        std::fs::create_dir_all(&raw_dir).unwrap();
        std::fs::write(
            raw_dir.join("result.json"),
            serde_json::to_vec_pretty(&raw).unwrap(),
        )
        .unwrap();
        std::fs::write(raw_dir.join("request.json"), snapshot.to_string()).unwrap();
        (temporary, executor, request, job, raw)
    }

    fn original_evidence(executor: &SavedTextExecutor, job: &str) -> Value {
        let c = executor.store.conn().unwrap();
        let (snapshot,raw,parent,event):(String,String,String,String)=c.query_row("SELECT j.snapshot,j.result,parent.snapshot,(SELECT data FROM events WHERE job_id=j.id AND type='text_received_raw' ORDER BY id DESC LIMIT 1) FROM jobs j JOIN jobs parent ON parent.id=?2 WHERE j.id=?1",params![job,executor.workflow_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        json!({"snapshot":snapshot,"raw":raw,"parent":parent,"event":event,
            "rawFileSha256":crate::pipeline::digest(&String::from_utf8(std::fs::read(executor.store.root.join("raw").join(job).join("result.json")).unwrap()).unwrap()),
            "requestFileSha256":crate::pipeline::digest(&String::from_utf8(std::fs::read(executor.store.root.join("raw").join(job).join("request.json")).unwrap()).unwrap()),
            "jobCount":executor.store.jobs().unwrap().len()})
    }

    fn normalized(raw: &Value) -> Value {
        let mut result = raw.clone();
        result["units"][0]["shots"][0]["id"] = json!("unit-one:shot:1");
        result["units"][1]["shots"][0]["id"] = json!("unit-two:shot:1");
        result
    }

    fn accepted_fields(audit: &Value, validated: &Value) -> Value {
        json!({"error":null,"receipt":{"validation":"accepted","contract":"desktop-runner/v1","revalidatedFrom":"rejected","migrationPolicy":audit["policy"],"previousValidationError":audit["previousError"],"migrationAudit":audit,"validatedResultSha256":crate::pipeline::digest(&validated.to_string()),"acceptedRequestHash":audit["newRequestHash"]}})
    }

    #[tokio::test]
    async fn scoped_author_acceptance_keeps_raw_request_files_and_repeated_resume_zero_dispatch() {
        let (_temporary, executor, request, job, raw) = fixture();
        let original = original_evidence(&executor, &job);
        let accepted = normalized(&raw);
        assert_ne!(raw, accepted);
        assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
        executor.accept(&request, &accepted).await.unwrap();
        let saved = executor.store.job(&job).unwrap();
        assert_eq!(saved["status"], "completed");
        let receipt = saved["receipt"].clone();
        assert_eq!(
            receipt["migrationPolicy"],
            STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY
        );
        assert_eq!(receipt["migrationAudit"]["generationRequestsSent"], 0);
        let c = executor.store.conn().unwrap();
        let (checkpoint, job_state, step_state, original_raw): (String, String, String, String) = c
            .query_row(
                "SELECT w.result,j.state,w.state,j.result FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
                params![executor.workflow_id, step_key(&request)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (job_state.as_str(), step_state.as_str()),
            ("completed", "accepted")
        );
        assert_eq!(original_raw, original["raw"].as_str().unwrap(), "SQL jobs.result remains the exact original bytes; Store::job adds UI mediaIds only in memory");
        assert_eq!(serde_json::from_str::<Value>(&original_raw).unwrap(), raw);
        assert_eq!(
            serde_json::from_str::<Value>(&checkpoint).unwrap(),
            accepted
        );
        assert!(scoped_author_replay_receipt_bound(
            &c,
            executor.workflow_id.as_deref().unwrap(),
            &step_key(&request)
        )
        .unwrap());
        let events: i64 = c
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        drop(c);
        for _ in 0..2 {
            assert_eq!(
                executor.execute(request.clone()).await.unwrap(),
                raw,
                "every recovery revalidates original transport data"
            );
            executor.accept(&request, &accepted).await.unwrap();
            assert_eq!(executor.store.job(&job).unwrap()["receipt"], receipt);
            assert_eq!(original_evidence(&executor, &job), original);
        }
        assert_eq!(
            executor
                .store
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            events
        );
        assert!(executor
            .reject(&request, &raw, AUTHOR_ASSET_SOURCE_REJECTION)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn scoped_author_secondary_domain_rejection_is_sealed_for_existing_bounded_editor_chain()
    {
        let (_temporary, executor, request, job, raw) = fixture();
        let original = original_evidence(&executor, &job);
        assert!(!author_repairable_rejection(
            STORYBOARD_AUTHOR_SHOT_REJECTION
        ));
        assert!(author_repairable_rejection(AUTHOR_ASSET_SOURCE_REJECTION));
        assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
        executor
            .reject(&request, &raw, AUTHOR_ASSET_SOURCE_REJECTION)
            .await
            .unwrap();
        let saved = executor.store.job(&job).unwrap();
        assert_eq!(saved["status"], "rejected");
        assert_eq!(saved["error"], AUTHOR_ASSET_SOURCE_REJECTION);
        assert_eq!(
            saved["receipt"]["previousValidationError"],
            STORYBOARD_AUTHOR_SHOT_REJECTION
        );
        let c = executor.store.conn().unwrap();
        let (job_state, original_raw): (String, String) = c
            .query_row("SELECT state,result FROM jobs WHERE id=?1", [&job], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(job_state, "rejected");
        assert_eq!(
            original_raw,
            original["raw"].as_str().unwrap(),
            "SQL jobs.result remains the exact original bytes despite the UI result projection"
        );
        assert_eq!(serde_json::from_str::<Value>(&original_raw).unwrap(), raw);
        assert!(
            scoped_author_replay_receipt_bound(
                &c,
                executor.workflow_id.as_deref().unwrap(),
                &step_key(&request)
            )
            .unwrap(),
            "workflow resolution requires this durable proof, not the policy name"
        );
        let (state,error,result):(String,String,String)=c.query_row("SELECT state,error,result FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2",params![executor.workflow_id,step_key(&request)],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(
            (state, error),
            ("rejected".into(), AUTHOR_ASSET_SOURCE_REJECTION.into())
        );
        assert_eq!(serde_json::from_str::<Value>(&result).unwrap(), raw);
        let events: i64 = c
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        drop(c);
        for _ in 0..2 {
            assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
            executor
                .reject(&request, &raw, AUTHOR_ASSET_SOURCE_REJECTION)
                .await
                .unwrap();
            assert!(executor
                .accept(&request, &normalized(&raw))
                .await
                .unwrap_err()
                .contains("保持未采用"));
            assert_eq!(original_evidence(&executor, &job), original);
        }
        assert_eq!(
            executor
                .store
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            events
        );
        assert!(executor
            .reject(
                &request,
                &json!({"editedFacts":true}),
                AUTHOR_ASSET_SOURCE_REJECTION
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn scoped_author_binding_tampering_and_unknown_submission_never_dispatch() {
        for tamper in [
            "snapshotBytes",
            "hash",
            "raw",
            "receipt",
            "latestRaw",
            "provider",
            "jobState",
            "stepState",
            "error",
            "parent",
            "skill",
            "source",
        ] {
            let (_temporary, executor, mut request, job, raw) = fixture();
            let c = executor.store.conn().unwrap();
            match tamper {
                "snapshotBytes" => {
                    c.execute(
                        "UPDATE jobs SET snapshot=snapshot||char(10) WHERE id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "hash" => {
                    c.execute(
                        "UPDATE workflow_steps SET request_hash='forged' WHERE text_job_id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "raw" => {
                    c.execute("UPDATE jobs SET result='{}' WHERE id=?1", [&job])
                        .unwrap();
                    c.execute(
                        "UPDATE workflow_steps SET result='{}' WHERE text_job_id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "receipt" => {
                    c.execute(
                        "UPDATE jobs SET receipt=json_set(receipt,'$.unverified',1) WHERE id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "latestRaw" => {
                    executor
                        .store
                        .event(
                            &job,
                            "text_received_raw",
                            &json!({"requestHash":"other","value":raw}),
                        )
                        .unwrap();
                }
                "provider" => {
                    c.execute("UPDATE jobs SET provider='api' WHERE id=?1", [&job])
                        .unwrap();
                }
                "jobState" => {
                    c.execute(
                        "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "stepState" => {
                    c.execute(
                        "UPDATE workflow_steps SET state='running' WHERE text_job_id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "error" => {
                    c.execute(
                        "UPDATE jobs SET error='other rejection' WHERE id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "parent" => {
                    c.execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.stage','storyboard.refine') WHERE id=?1",[executor.workflow_id.as_deref().unwrap()]).unwrap();
                }
                "skill" => {
                    request.skill = "asset-art-director".into();
                }
                "source" => {
                    request.input["sourceRows"][0]["text"] = json!("篡改原文");
                }
                _ => unreachable!(),
            }
            drop(c);
            let before = original_evidence(&executor, &job);
            assert!(executor.execute(request.clone()).await.is_err(), "{tamper}");
            assert!(
                executor.accept(&request, &normalized(&raw)).await.is_err(),
                "{tamper}"
            );
            assert_eq!(
                original_evidence(&executor, &job),
                before,
                "{tamper}: no new job or raw/request rewrite"
            );
        }
    }

    #[test]
    fn scoped_author_store_cas_rechecks_latest_event_parent_and_receipt_atomically() {
        for tamper in ["latestRaw", "parentBytes", "receiptBytes", "state"] {
            let (_temporary, executor, request, job, raw) = fixture();
            let snapshot = executor.request_snapshot(&request).unwrap();
            let hash = crate::pipeline::digest(&snapshot.to_string());
            let replay = executor
                .scoped_author_replay(&request, &snapshot, &hash)
                .unwrap()
                .unwrap();
            let accepted = normalized(&raw);
            let fields = accepted_fields(&replay.frozen.audit, &accepted);
            let c = executor.store.conn().unwrap();
            match tamper {
                "latestRaw" => {
                    executor
                        .store
                        .event(
                            &job,
                            "text_received_raw",
                            &json!({"requestHash":hash,"value":raw}),
                        )
                        .unwrap();
                }
                "parentBytes" => {
                    c.execute(
                        "UPDATE jobs SET snapshot=snapshot||char(10) WHERE id=?1",
                        [executor.workflow_id.as_deref().unwrap()],
                    )
                    .unwrap();
                }
                "receiptBytes" => {
                    c.execute(
                        "UPDATE jobs SET receipt=receipt||char(10) WHERE id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                "state" => {
                    c.execute(
                        "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                        [&job],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            drop(c);
            let before = original_evidence(&executor, &job);
            assert!(
                !executor
                    .store
                    .commit_text_replay(&replay.frozen.audit, &fields, &accepted)
                    .unwrap(),
                "{tamper}"
            );
            assert_eq!(original_evidence(&executor, &job), before);
            assert_ne!(executor.store.job(&job).unwrap()["status"], "completed");
            assert_ne!(
                executor
                    .store
                    .conn()
                    .unwrap()
                    .query_row("SELECT state FROM jobs WHERE id=?1", [&job], |row| row
                        .get::<_, String>(
                        0
                    ))
                    .unwrap(),
                "completed"
            );
        }
    }

    #[test]
    fn scoped_author_policy_cannot_expand_old_finisher_replay_or_forge_an_origin() {
        let (_temporary, executor, request, job, raw) = fixture();
        let snapshot = executor.request_snapshot(&request).unwrap();
        let hash = crate::pipeline::digest(&snapshot.to_string());
        let replay = executor
            .scoped_author_replay(&request, &snapshot, &hash)
            .unwrap()
            .unwrap();
        let accepted = normalized(&raw);
        let before = original_evidence(&executor, &job);
        for field in [
            "policy",
            "previousError",
            "workflowId",
            "nodeKey",
            "originalRawEventId",
        ] {
            let mut forged = replay.frozen.audit.clone();
            forged[field] = match field {
                "policy" => json!(STORYBOARD_FINISH_COVERAGE_POLICY),
                "previousError" => json!(AUTHOR_ASSET_SOURCE_REJECTION),
                "originalRawEventId" => json!(0),
                _ => json!("foreign identity"),
            };
            let committed = executor.store.commit_text_replay(
                &forged,
                &accepted_fields(&forged, &accepted),
                &accepted,
            );
            assert!(!committed.unwrap_or(false), "{field}");
            assert_eq!(original_evidence(&executor, &job), before);
        }
        assert!(executor
            .store
            .commit_text_replay(
                &replay.frozen.audit,
                &accepted_fields(&replay.frozen.audit, &accepted),
                &accepted
            )
            .unwrap());
        assert!(
            !executor
                .store
                .commit_text_replay(
                    &replay.frozen.audit,
                    &accepted_fields(&replay.frozen.audit, &accepted),
                    &accepted
                )
                .unwrap(),
            "second CAS cannot accept a different checkpoint"
        );
    }

    struct TwoAttemptRepair {
        _directory: tempfile::TempDir,
        executor: SavedTextExecutor,
        author: TextRequest,
        author_job: String,
        author_raw: Value,
        first: TextRequest,
        second: TextRequest,
        first_job: String,
        second_job: String,
        first_raw: Value,
        accepted: Value,
        tail: Value,
    }

    // Seed two already received synthetic model responses. All transitions use
    // the real SavedTextExecutor/CAS contract, never an executable or API call.
    async fn two_attempt_repair(tamper: Option<&str>) -> TwoAttemptRepair {
        let (directory, executor, author, author_job, author_raw) = fixture();
        executor.execute(author.clone()).await.unwrap();
        executor
            .reject(&author, &author_raw, AUTHOR_ASSET_SOURCE_REJECTION)
            .await
            .unwrap();
        let run = executor.workflow_id.as_deref().unwrap();
        let source_sha: String = executor
            .store
            .conn()
            .unwrap()
            .query_row(
                "SELECT json_extract(snapshot,'$.input.source.sha256') FROM jobs WHERE id=?1",
                [run],
                |row| row.get(0),
            )
            .unwrap();
        let domain_hash = crate::pipeline::digest(&serde_json::to_string(&author).unwrap());
        let raw_hash = crate::pipeline::digest(&author_raw.to_string());
        let base = normalized(&author_raw);
        let mut progressed = base.clone();
        progressed["units"][0]["id"] = json!("first-repaired-unit");
        progressed["units"][0]["exitState"] = json!("first synthetic issue corrected");
        let mut final_draft = progressed.clone();
        final_draft["units"][1]["id"] = json!("second-repaired-unit");
        final_draft["units"][1]["exitState"] = json!("second synthetic issue corrected");
        let make_request = |base: &Value, attempt: u64| {
            let base_hash = crate::pipeline::finish_base_hash(base);
            let descriptor = json!({"version":"rejected-author-scope-repair/v1","attempt":attempt,"maximumAttempts":2,"sourceSha256":source_sha,"originDomainRequestSha256":domain_hash,"originExecutionBatch":author.input["executionBatch"],"originalAuthorRawSha256":raw_hash,"baseDraftHash":base_hash,"problemSignature":crate::pipeline::digest(&format!("synthetic-remaining-issue-{attempt}"))});
            let identity = crate::pipeline::digest(&descriptor.to_string());
            let full = json!({"properties":{"units":{"items":{"type":"object","properties":{"id":{"type":"string"}}}},"fantasyRealization":{"type":"array"},"revisionNotes":{"type":"array"},"unresolved":{"type":"array"}}});
            TextRequest {
                stage: "storyboardFinish".into(),
                skill: "episode-finishing-editor".into(),
                model: author.model.clone(),
                reasoning: author.reasoning.clone(),
                input: json!({"storyboardCreativeProfile":"original-production-contract/v1","episodeId":author.input["episodeId"],"sourceRows":author.input["sourceRows"],"capabilities":author.input["capabilities"],"previousDraft":base,"baseDraftHash":base_hash,"finishMode":"delta","authorRepair":descriptor,"validationIssues":[{"code":"ASSET_SOURCE_SCOPE","problem":AUTHOR_ASSET_SOURCE_REJECTION}],"executionBatch":{"version":"author-domain-repair/v1","id":format!("author-domain-repair:{identity}"),"originExecutionBatchId":author.input["executionBatch"]["id"],"sourceRefs":["row:synthetic:0:4","row:synthetic:5:9"]}}),
                schema: crate::pipeline::scope_repair_delta_schema_for_test(&full, base),
                rule_snapshot: crate::pipeline::snapshot("episode-finishing-editor", &[]).unwrap(),
            }
        };
        let mut first = make_request(&base, 1);
        let mut second = make_request(&progressed, 2);
        // The unrelated schema, rule, source or input must stay fully frozen.
        match tamper {
            Some("baseEnum") => {
                second.schema["properties"]["baseDraftHash"]["enum"] = json!(["unbound"])
            }
            Some("retainedEnum") => {
                second.schema["properties"]["retainedUnitIds"]["items"]["enum"] = json!(["foreign"])
            }
            Some("replacesEnum") => {
                second.schema["properties"]["replacements"]["items"]["properties"]["replaces"]
                    ["items"]["enum"] = json!(["foreign"])
            }
            Some("firstBaseEnum") => {
                first.schema["properties"]["baseDraftHash"]["enum"] = json!(["unbound"])
            }
            Some("unitSchema") => {
                second.schema["properties"]["replacements"]["items"]["properties"]["units"]
                    ["items"]["properties"]["id"]["type"] = json!("number")
            }
            Some("extraSchemaField") => second.schema["unboundExtension"] = json!(true),
            Some("baseInput") => {
                second.input["previousDraft"]["units"][0]["exitState"] = json!("unbound base")
            }
            Some("sourceRow") => second.input["sourceRows"][0]["text"] = json!("changed source"),
            Some("rule") => second.rule_snapshot["primary"]["sha256"] = json!("unbound rule"),
            Some("capabilities") => second.input["capabilities"] = json!({"changed":true}),
            Some(other) => panic!("unknown tamper {other}"),
            None => {}
        }
        let delta = |base: &Value, unit_index: usize, next: &Value| json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":crate::pipeline::finish_base_hash(base),"retainedUnitIds":base["units"].as_array().unwrap().iter().enumerate().filter_map(|(index,unit)|(index!=unit_index).then(||unit["id"].clone())).collect::<Vec<_>>(),"replacements":[{"replaces":[base["units"][unit_index]["id"]],"units":[next["units"][unit_index]],"fantasyRealization":[]}],"revisionNotes":[],"unresolved":[]});
        let first_raw = delta(&base, 0, &progressed);
        let second_raw = delta(&progressed, 1, &final_draft);
        let seed = |request: &TextRequest, raw: &Value| {
            let snapshot = executor.request_snapshot(request).unwrap();
            let hash = crate::pipeline::digest(&snapshot.to_string());
            let job = executor
                .store
                .add_job(
                    "text",
                    "synthetic-finisher",
                    "codex",
                    "local",
                    &snapshot,
                    "received",
                )
                .unwrap();
            executor
                .store
                .update_job(&job, "received", &json!({"result":raw}))
                .unwrap();
            executor
                .store
                .event(
                    &job,
                    "text_received_raw",
                    &json!({"requestHash":hash,"value":raw}),
                )
                .unwrap();
            executor
                .store
                .checkpoint(
                    run,
                    &step_key(request),
                    &request.stage,
                    &hash,
                    "running",
                    Some(&job),
                    None,
                    None,
                )
                .unwrap();
            job
        };
        let first_job = seed(&first, &first_raw);
        assert_eq!(executor.execute(first.clone()).await.unwrap(), first_raw);
        executor
            .reject(&first, &first_raw, AUTHOR_ASSET_SOURCE_REJECTION)
            .await
            .unwrap();
        let second_job = seed(&second, &second_raw);
        assert_eq!(executor.execute(second.clone()).await.unwrap(), second_raw);
        let nodes = json!([
            {"attempt":1,"canonicalDomainRequestSha256":crate::pipeline::digest(&serde_json::to_string(&first).unwrap()),"result":"rejected-scope","baseDraftHash":crate::pipeline::finish_base_hash(&base),"draftSha256":crate::pipeline::finish_base_hash(&progressed)},
            {"attempt":2,"canonicalDomainRequestSha256":crate::pipeline::digest(&serde_json::to_string(&second).unwrap()),"baseDraftHash":crate::pipeline::finish_base_hash(&progressed),"draftSha256":crate::pipeline::finish_base_hash(&final_draft)}
        ]);
        final_draft["authorRepairProvenance"] = json!({"version":"accepted-author-scope-repair/v1","sourceSha256":source_sha,"originDomainRequestSha256":domain_hash,"originalAuthorRawSha256":raw_hash,"originalAuthorAccepted":false,"nodes":nodes});
        let mut accepted = second_raw.clone();
        accepted["materializedDraft"] = final_draft.clone();
        executor.accept(&second, &accepted).await.unwrap();
        let unit = final_draft["units"].as_array().unwrap().last().unwrap();
        let tail = json!({"version":"accepted-tail-v1","executionBatchId":author.input["executionBatch"]["id"],"unit":unit,"sourceRefs":unit["sourceRefs"],"evidenceRole":"ACCEPTED_CREATIVE_STATE_ONLY"});
        TwoAttemptRepair {
            _directory: directory,
            executor,
            author,
            author_job,
            author_raw,
            first,
            second,
            first_job,
            second_job,
            first_raw,
            accepted,
            tail,
        }
    }

    #[tokio::test]
    async fn scoped_author_two_finisher_attempts_bind_changed_delta_schema_and_resume_zero_dispatch(
    ) {
        let recorded = two_attempt_repair(None).await;
        let executor = &recorded.executor;
        let run = executor.workflow_id.as_deref().unwrap();
        assert_ne!(
            recorded.first.schema, recorded.second.schema,
            "actual finish_delta_schema changes with the progressed base and unit IDs"
        );
        assert_ne!(
            recorded.first.input["authorRepair"]["problemSignature"],
            recorded.second.input["authorRepair"]["problemSignature"]
        );
        assert!(
            crate::workflow::accepted_author_repair_predecessor(
                &executor.store,
                run,
                &step_key(&recorded.second),
                &recorded.tail
            )
            .unwrap(),
            "two genuine persistent Finisher receipts resolve the still-rejected author"
        );
        let original = original_evidence(executor, &recorded.author_job);
        let bytes = |job: &str| {
            executor
                .store
                .conn()
                .unwrap()
                .query_row(
                    "SELECT snapshot,result FROM jobs WHERE id=?1",
                    [job],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap()
        };
        let first = bytes(&recorded.first_job);
        let second = bytes(&recorded.second_job);
        let count: i64 = executor
            .store
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        for _ in 0..2 {
            assert_eq!(
                executor.execute(recorded.author.clone()).await.unwrap(),
                recorded.author_raw
            );
            executor
                .reject(
                    &recorded.author,
                    &recorded.author_raw,
                    AUTHOR_ASSET_SOURCE_REJECTION,
                )
                .await
                .unwrap();
            assert_eq!(
                executor.execute(recorded.first.clone()).await.unwrap(),
                recorded.first_raw
            );
            executor
                .reject(
                    &recorded.first,
                    &recorded.first_raw,
                    AUTHOR_ASSET_SOURCE_REJECTION,
                )
                .await
                .unwrap();
            assert_eq!(
                executor.execute(recorded.second.clone()).await.unwrap(),
                recorded.accepted
            );
            executor
                .accept(&recorded.second, &recorded.accepted)
                .await
                .unwrap();
            assert!(crate::workflow::accepted_author_repair_predecessor(
                &executor.store,
                run,
                &step_key(&recorded.second),
                &recorded.tail
            )
            .unwrap());
        }
        assert_eq!(original_evidence(executor, &recorded.author_job), original);
        assert_eq!(bytes(&recorded.first_job), first);
        assert_eq!(bytes(&recorded.second_job), second);
        assert_eq!(
            executor
                .store
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            count,
            "recovery creates neither author nor a third editor attempt"
        );
        let c = executor.store.conn().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT state FROM jobs WHERE id=?1",
                [&recorded.author_job],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "rejected"
        );
        assert_eq!(
            c.query_row(
                "SELECT state FROM jobs WHERE id=?1",
                [&recorded.first_job],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "rejected"
        );
        assert_eq!(
            c.query_row(
                "SELECT state FROM jobs WHERE id=?1",
                [&recorded.second_job],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "completed"
        );
    }

    #[tokio::test]
    async fn scoped_author_two_finisher_attempts_reject_unbound_schema_rules_source_and_base() {
        for tamper in [
            "baseEnum",
            "retainedEnum",
            "replacesEnum",
            "firstBaseEnum",
            "unitSchema",
            "extraSchemaField",
            "baseInput",
            "sourceRow",
            "rule",
            "capabilities",
        ] {
            let recorded = two_attempt_repair(Some(tamper)).await;
            assert!(
                !crate::workflow::accepted_author_repair_predecessor(
                    &recorded.executor.store,
                    recorded.executor.workflow_id.as_deref().unwrap(),
                    &step_key(&recorded.second),
                    &recorded.tail
                )
                .unwrap(),
                "{tamper}: accepting a recorded result cannot bypass the frozen two-attempt proof"
            );
        }
    }

    #[tokio::test]
    async fn scoped_author_normalized_checkpoint_or_durable_origin_tampering_is_not_a_cache() {
        for tamper in [
            "normalized",
            "originRawHash",
            "receiptPolicyOnly",
            "latestRaw",
            "parentBytes",
        ] {
            let (_temporary, executor, request, job, raw) = fixture();
            executor.accept(&request, &normalized(&raw)).await.unwrap();
            let c = executor.store.conn().unwrap();
            match tamper {
                "normalized" => {
                    c.execute("UPDATE workflow_steps SET result=json_set(result,'$.units[0].id','forged') WHERE text_job_id=?1",[&job]).unwrap();
                }
                "originRawHash" => {
                    c.execute("UPDATE jobs SET receipt=json_set(receipt,'$.migrationAudit.originalResultSha256','forged') WHERE id=?1",[&job]).unwrap();
                }
                "receiptPolicyOnly" => {
                    c.execute("UPDATE jobs SET receipt=?1 WHERE id=?2",params![json!({"migrationPolicy":STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY,"contract":"desktop-runner/v1","validation":"accepted"}).to_string(),job]).unwrap();
                }
                "latestRaw" => {
                    executor
                        .store
                        .event(
                            &job,
                            "text_received_raw",
                            &json!({"requestHash":"foreign","value":raw}),
                        )
                        .unwrap();
                }
                "parentBytes" => {
                    c.execute(
                        "UPDATE jobs SET snapshot=snapshot||char(10) WHERE id=?1",
                        [executor.workflow_id.as_deref().unwrap()],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                !scoped_author_replay_receipt_bound(
                    &c,
                    executor.workflow_id.as_deref().unwrap(),
                    &step_key(&request)
                )
                .unwrap(),
                "{tamper}: workflow repair proof fails closed"
            );
            drop(c);
            let before = original_evidence(&executor, &job);
            assert!(executor.execute(request.clone()).await.is_err(), "{tamper}");
            assert_eq!(original_evidence(&executor, &job), before);
        }
    }
}
