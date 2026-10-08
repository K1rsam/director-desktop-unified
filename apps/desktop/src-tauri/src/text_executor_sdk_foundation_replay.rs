//! One SDK F2 domain regression: revalidate original bytes, never dispatch again.
use super::*;
use crate::text_transport::{AttemptState, Backend, FrozenAttempt};

const POLICY: &str = "sdk-f2-scene-episode-projection-replay/v1";
const REJECTION: &str = "资产用途集号与本次场引用不一致";

fn eligible(snapshot: &Value) -> bool {
    snapshot["stage"] == "foundation:F2"
        && snapshot["skill"] == "foundation-asset-catalog"
        && snapshot["executorIdentity"]["provider"] == "codex_sdk"
        && snapshot["input"]["method"] == "foundation-delivery/v2"
        && snapshot["input"]["modelInputProfile"] == "foundation-compact/v1"
        && snapshot["input"]["catalogueContract"] == "light-catalogue/v1"
        && snapshot["input"]["sourceReferenceEncoding"] == "row-coordinates/v1"
        && snapshot["input"]["delivery"]["method"] == "foundation-delivery/v2"
        && snapshot["input"]["delivery"]["stage"] == "F2"
        && snapshot["input"]["delivery"]["phase"] == "GENERATE_F2"
        && snapshot["input"]["delivery"]["scope"] == snapshot["input"]["scope"]
        && snapshot["input"]["delivery"]["batchIndex"] == snapshot["input"]["batchIndex"]
}

fn source_bound(snapshot: &Value, parent: &Value) -> bool {
    let source = &parent["input"]["source"];
    let Ok(locked) = serde_json::from_value::<crate::pipeline::LockedSource>(source.clone()) else {
        return false;
    };
    let input = &snapshot["input"];
    if crate::pipeline::digest(&locked.text) != locked.sha256
        || input["sourceSha256"] != locked.sha256
        || parent["sourceVersion"] != parent["input"]["sourceVersion"]
        || parent["sourceVersion"].as_u64().is_none()
        || source["version"] != parent["sourceVersion"]
        || input["source"]["columns"]
            != json!([
                "rowId",
                "line",
                "episodeId",
                "sceneId",
                "kind",
                "speaker",
                "channel",
                "start",
                "end",
                "text"
            ])
    {
        return false;
    }
    let Some(rows) = input["source"]["rows"].as_array().filter(|r| !r.is_empty()) else {
        return false;
    };
    let mut seen = std::collections::HashSet::new();
    rows.iter().all(|tuple| {
        let Some(parts) = tuple.as_array().filter(|r| r.len() == 10) else {
            return false;
        };
        let Some(row) = locked.rows.iter().find(|row| parts[0] == row.id) else {
            return false;
        };
        let (Some(start), Some(end)) = (parts[7].as_u64(), parts[8].as_u64()) else {
            return false;
        };
        let range = crate::pipeline::SourceRange {
            start: start as usize,
            end: end as usize,
        };
        seen.insert((row.id.as_str(), start, end))
            && parts[1] == row.number
            && parts[2] == json!(row.episode_id)
            && parts[3] == json!(row.scene_id)
            && parts[4] == row.kind
            && parts[5] == json!(row.dialogue.as_ref().map(|d| d.speaker.as_str()))
            && parts[6] == json!(row.dialogue.as_ref().map(|d| d.channel.as_str()))
            && start < end
            && range.start >= row.range.start
            && range.end <= row.range.end
            && crate::pipeline::slice_utf16(&locked.text, &range)
                .ok()
                .as_deref()
                == parts[9].as_str()
    })
}

fn audit_valid(audit: &Value) -> bool {
    if !audit.as_object().is_some_and(|f| f.len() == 21)
        || audit["policy"] != POLICY
        || audit["stage"] != "foundation:F2"
        || audit["previousError"] != REJECTION
        || audit["generationRequestsSent"] != 0
        || audit["acceptedAutomatically"] != false
        || [
            "workflowId",
            "nodeKey",
            "originalJobId",
            "requestHash",
            "originalAttemptSha256",
            "originalCompiledSha256",
            "originalRawTextSha256",
        ]
        .iter()
        .any(|k| s(audit, k).is_empty())
    {
        return false;
    }
    let parse = |key| serde_json::from_str::<Value>(s(audit, key));
    let (Ok(snapshot), Ok(raw), Ok(parent), Ok(receipt), Ok(prepared), Ok(event), Ok(sdk_event)) = (
        parse("originalSnapshotJson"),
        parse("originalResultJson"),
        parse("originalWorkflowSnapshotJson"),
        parse("originalReceiptJson"),
        parse("originalPreparedJson"),
        parse("originalRawEventDataJson"),
        parse("originalSdkRawEventDataJson"),
    ) else {
        return false;
    };
    let Ok(request) = serde_json::from_value::<TextRequest>(snapshot.clone()) else {
        return false;
    };
    let rules = parent["rules"].as_array();
    let settings = &parent["input"]["settings"];
    eligible(&snapshot)
        && snapshot.to_string() == s(audit, "originalSnapshotJson")
        && crate::pipeline::digest(s(audit, "originalSnapshotJson")) == s(audit, "requestHash")
        && parent["stage"] == "foundation.run"
        && frozen_text_parent_matches("foundation.run", &parent, &request)
        && settings["textProvider"] == "codex_sdk"
        && settings["textSdkIdentity"] == snapshot["executorIdentity"]
        && settings["textModel"] == request.model
        && settings["reasoning"] == request.reasoning
        && snapshot["executorPolicy"] == CLI_POLICY_VERSION
        && source_bound(&snapshot, &parent)
        && snapshot["ruleSnapshot"]["primary"]["id"] == request.skill
        && rules.is_some_and(|rules| rules.contains(&snapshot["ruleSnapshot"]["primary"]))
        && std::iter::once(&snapshot["ruleSnapshot"]["primary"])
            .chain(
                snapshot["ruleSnapshot"]["methods"]
                    .as_array()
                    .into_iter()
                    .flatten(),
            )
            .all(|rule| {
                !s(rule, "content").is_empty()
                    && rule["sha256"] == crate::pipeline::digest(s(rule, "content"))
                    && rules.is_some_and(|rules| rules.contains(rule))
            })
        && audit["nodeKey"] == step_key(&request)
        && receipt == json!({"contract":"desktop-runner/v1","validation":"rejected"})
        && prepared.as_object().is_some_and(|f| f.len() == 7)
        && prepared["sdkDispatchStarted"] == true
        && prepared["sdkOriginWorkflowId"] == audit["workflowId"]
        && prepared["sdkAttemptId"] == audit["originalJobId"]
        && prepared["sdkRequestHash"] == audit["requestHash"]
        && prepared["sdkRawTextSha256"] == audit["originalRawTextSha256"]
        && event == json!({"requestHash":audit["requestHash"],"value":raw})
        && audit["originalRawEventId"]
            .as_i64()
            .is_some_and(|id| id > 0)
        && audit["originalSdkRawEventId"]
            .as_i64()
            .is_some_and(|id| id > 0)
        && sdk_event["domainAccepted"] == false
        && sdk_event["parseError"].is_null()
        && sdk_event["rawText"].as_str().is_some_and(|text| {
            crate::pipeline::digest(text) == s(audit, "originalRawTextSha256")
                && serde_json::from_str::<Value>(text).ok().as_ref()
                    == Some(&sdk_event["parsedJson"])
        })
}

fn files_bound(root: &Path, audit: &Value) -> bool {
    let base = root.join("text-runs").join(s(audit, "originalJobId"));
    if [
        ("sdk-attempt.json", "originalAttemptSha256", 4_194_304),
        ("sdk-compiled.json", "originalCompiledSha256", 2_097_152),
        ("result.json", "originalRawTextSha256", 1_048_576),
    ]
    .iter()
    .any(|(file, key, limit)| {
        crate::store::hash_file(&base.join(file))
            .ok()
            .is_none_or(|(hash, size)| hash != s(audit, key) || size > *limit)
    }) {
        return false;
    }
    let (Ok(attempt), Ok(snapshot), Ok(sdk_event), Ok(prepared)) = (
        FrozenAttempt::load(&base.join("sdk-attempt.json")),
        serde_json::from_str::<Value>(s(audit, "originalSnapshotJson")),
        serde_json::from_str::<Value>(s(audit, "originalSdkRawEventDataJson")),
        serde_json::from_str::<Value>(s(audit, "originalPreparedJson")),
    ) else {
        return false;
    };
    let Ok(request) = serde_json::from_value::<TextRequest>(snapshot.clone()) else {
        return false;
    };
    let Ok(compiled) = compile_codex_input(&request) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(base.join("sdk-compiled.json")) else {
        return false;
    };
    let expected = json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)});
    attempt.backend == Backend::CodexSdk
        && attempt.state == AttemptState::Completed
        && attempt.dispatch_started
        && attempt.attempt_id == s(audit, "originalJobId")
        && attempt.workflow_id == s(audit, "workflowId")
        && attempt.logical_node_id == s(audit, "nodeKey")
        && attempt.runtime_identity == snapshot["executorIdentity"]
        && serde_json::to_value(&attempt.request).ok() == serde_json::to_value(&request).ok()
        && attempt.raw_result.as_ref() == Some(&sdk_event)
        && serde_json::from_slice::<Value>(&bytes).ok().as_ref() == Some(&expected)
        && prepared["compiledPromptSha256"] == expected["promptSha256"]
        && prepared["compiledSchemaSha256"] == expected["schemaSha256"]
}

fn database_bound(
    c: &rusqlite::Connection,
    audit: &Value,
    accepted: bool,
    prepared: &Value,
    receipt: &Value,
    result: &Value,
) -> Result<bool> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id JOIN jobs p ON p.id=w.workflow_id WHERE j.id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND j.snapshot=?2 AND j.result=?3 AND j.prepared=?4 AND j.receipt=?5 AND j.state=?6 AND j.error IS ?7 AND w.workflow_id=?8 AND w.node_key=?9 AND w.stage='foundation:F2' AND w.request_hash=?10 AND w.state=?11 AND w.result=?12 AND w.error IS ?7 AND p.kind='workflow' AND p.entity_id='foundation.run' AND p.snapshot=?13 AND EXISTS(SELECT 1 FROM events e WHERE e.job_id=j.id AND e.type='text_received_raw' AND e.id=?14 AND e.data=?15 AND e.id=(SELECT MAX(id) FROM events WHERE job_id=j.id AND type='text_received_raw')) AND EXISTS(SELECT 1 FROM events e WHERE e.job_id=j.id AND e.type='sdk_raw_result' AND e.id=?16 AND e.data=?17 AND e.id=(SELECT MAX(id) FROM events WHERE job_id=j.id AND type='sdk_raw_result')))",
        params![s(audit,"originalJobId"),s(audit,"originalSnapshotJson"),s(audit,"originalResultJson"),prepared.to_string(),receipt.to_string(),if accepted {"completed"} else {"rejected"},if accepted {None} else {Some(REJECTION)},s(audit,"workflowId"),s(audit,"nodeKey"),s(audit,"requestHash"),if accepted {"accepted"} else {"rejected"},result.to_string(),s(audit,"originalWorkflowSnapshotJson"),audit["originalRawEventId"].as_i64(),s(audit,"originalRawEventDataJson"),audit["originalSdkRawEventId"].as_i64(),s(audit,"originalSdkRawEventDataJson")],
        |r|r.get(0)).map_err(|e|e.to_string())
}

impl SavedTextExecutor {
    pub(super) fn sdk_f2_migrated_receipt_bound(
        &self,
        job: &Value,
        snapshot: &Value,
    ) -> Result<bool> {
        let receipt = &job["receipt"];
        if receipt["migrationPolicy"] != POLICY {
            return Ok(false);
        }
        let audit = &receipt["migrationAudit"];
        let result = &job["prepared"]["sdkDomainResult"];
        let original: Value =
            serde_json::from_str(s(audit, "originalResultJson")).map_err(|e| e.to_string())?;
        let mut expected_prepared: Value =
            serde_json::from_str(s(audit, "originalPreparedJson")).map_err(|e| e.to_string())?;
        expected_prepared["sdkDomainResult"] = result.clone();
        expected_prepared["sdkDomainResultSha256"] =
            json!(crate::pipeline::digest(&result.to_string()));
        Ok(audit_valid(audit)
            && files_bound(&self.store.root, audit)
            && job["snapshot"] == *snapshot
            && snapshot.to_string() == s(audit, "originalSnapshotJson")
            && audit["originalJobId"] == job["id"]
            && receipt
                == &json!({"contract":"desktop-runner/v1","validation":"accepted","migrationPolicy":POLICY,"migrationAudit":audit,"acceptedRequestHash":audit["requestHash"],"validatedResultSha256":crate::pipeline::digest(&result.to_string())})
            && job["prepared"] == expected_prepared
            && crate::pipeline::foundation_delivery::accepted_scope_reason_raw_matches(
                &snapshot["input"],
                &original,
                result,
            )
            && database_bound(
                &self.store.conn()?,
                audit,
                true,
                &expected_prepared,
                receipt,
                result,
            )?)
    }

    pub(super) fn sdk_f2_replay_commit(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        job: &Value,
        value: &Value,
        reason: Option<&str>,
    ) -> Result<bool> {
        if job["status"] != "rejected" || job["error"] != REJECTION || !eligible(snapshot) {
            return Ok(false);
        }
        let audit = self.sdk_f2_replay_audit(request, snapshot, job)?;
        if !self
            .store
            .commit_sdk_f2_episode_replay(&audit, value, reason)?
        {
            return Err("SDK_F2_REPLAY_HELD: 原冻结节点变化；保留原任务且不重发".into());
        }
        Ok(true)
    }

    fn sdk_f2_replay_audit(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        job: &Value,
    ) -> Result<Value> {
        let held = || {
            "SDK_F2_REPLAY_HELD: 原 F2 请求、父流程、回包、回执或接收证据不匹配；保留原任务且不重发"
                .to_string()
        };
        let run = self.workflow_id.as_deref().ok_or_else(held)?;
        let c = self.store.conn()?;
        let parent: String = c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='foundation.run'",[run],|r|r.get(0)).map_err(|_|held())?;
        let (event_id,event):(i64,String) = c.query_row("SELECT id,data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",[s(job,"id")],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|held())?;
        let (sdk_id,sdk_event):(i64,String) = c.query_row("SELECT id,data FROM events WHERE job_id=?1 AND type='sdk_raw_result' ORDER BY id DESC LIMIT 1",[s(job,"id")],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|held())?;
        let path = self.store.root.join("text-runs").join(s(job, "id"));
        let audit = json!({"policy":POLICY,"stage":"foundation:F2","previousError":REJECTION,
            "workflowId":run,"nodeKey":step_key(request),"originalJobId":job["id"],"requestHash":crate::pipeline::digest(&snapshot.to_string()),
            "originalSnapshotJson":snapshot.to_string(),"originalResultJson":job["result"].to_string(),"originalPreparedJson":job["prepared"].to_string(),"originalReceiptJson":job["receipt"].to_string(),"originalWorkflowSnapshotJson":parent,
            "originalRawEventId":event_id,"originalRawEventDataJson":event,"originalSdkRawEventId":sdk_id,"originalSdkRawEventDataJson":sdk_event,
            "originalAttemptSha256":crate::store::hash_file(&path.join("sdk-attempt.json"))?.0,"originalCompiledSha256":crate::store::hash_file(&path.join("sdk-compiled.json"))?.0,"originalRawTextSha256":crate::store::hash_file(&path.join("result.json"))?.0,
            "generationRequestsSent":0,"acceptedAutomatically":false});
        Ok(audit)
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
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("isolated")).unwrap();
        store
            .create("合成 SDK F2 用途恢复", "synthetic-f2")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        let locked = crate::pipeline::prepare_source(
            "第一集\n场景：路口\n△甲向前走。\n第二集\n场景：门前\n△甲停步。\n",
        )
        .unwrap();
        let source = store.save_source(json!(locked)).unwrap();
        let locked: crate::pipeline::LockedSource = serde_json::from_value(source.clone()).unwrap();
        let settings = crate::text_sdk::offline_settings("happy");
        let rules = crate::pipeline::snapshot("foundation-asset-catalog", &["screenplay-fidelity"])
            .unwrap();
        let scope: Vec<_> = locked.episodes.iter().map(|e| e.id.clone()).collect();
        let request = TextRequest {
            stage: "foundation:F2".into(),
            skill: "foundation-asset-catalog".into(),
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            input: json!({"method":"foundation-delivery/v2","modelInputProfile":"foundation-compact/v1",
                "catalogueContract":"light-catalogue/v1","sourceReferenceEncoding":"row-coordinates/v1","sourceSha256":locked.sha256,
                "scope":scope,"batchIndex":0,"batch":"synthetic-batch","delivery":{"method":"foundation-delivery/v2","stage":"F2","phase":"GENERATE_F2","scope":scope,"batchIndex":0,"batchCount":1,"fragment":null},
                "source":{"columns":["rowId","line","episodeId","sceneId","kind","speaker","channel","start","end","text"],"rows":locked.rows.iter().map(|r|json!([r.id,r.number,r.episode_id,r.scene_id,r.kind,r.dialogue.as_ref().map(|d|d.speaker.as_str()),r.dialogue.as_ref().map(|d|d.channel.as_str()),r.range.start,r.range.end,r.text])).collect::<Vec<_>>()},
                "scenes":locked.scenes.iter().map(|s|json!([s.episode_id,s.id,s.title])).collect::<Vec<_>>() }),
            schema: json!({"type":"object","properties":{"ok":{"type":"boolean"},"facts":{"type":"array","items":{"type":"string"}}},"required":["ok","facts"],"additionalProperties":false}),
            rule_snapshot: rules.clone(),
        };
        let parent_rules: Vec<_> = std::iter::once(rules["primary"].clone())
            .chain(rules["methods"].as_array().unwrap().iter().cloned())
            .collect();
        let run = store.add_job("workflow","foundation.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run","sourceVersion":source["version"],"rules":parent_rules,"input":{"source":source,"sourceVersion":source["version"],"settings":settings}}),"running").unwrap();
        let executor = SavedTextExecutor {
            store,
            settings,
            workflow_id: Some(run.clone()),
        };
        let snapshot = executor.request_snapshot(&request).unwrap();
        let hash = crate::pipeline::digest(&snapshot.to_string());
        let compiled = compile_codex_input(&request).unwrap();
        let job = executor
            .store
            .add_job(
                "text",
                "foundation:F2",
                "codex_sdk",
                "codex_sdk",
                &snapshot,
                "rejected",
            )
            .unwrap();
        let raw = json!({"ok":true,"facts":["合成原始事实"]});
        let envelope = json!({"domainAccepted":false,"parseError":null,"parsedJson":raw,"rawText":raw.to_string()});
        let prepared = json!({"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":run,"sdkRawTextSha256":crate::pipeline::digest(&raw.to_string()),"sdkRequestHash":hash});
        let receipt = json!({"contract":"desktop-runner/v1","validation":"rejected"});
        let c = executor.store.conn().unwrap();
        c.execute(
            "UPDATE jobs SET result=?1,error=?2,prepared=?3,receipt=?4 WHERE id=?5",
            params![
                raw.to_string(),
                REJECTION,
                prepared.to_string(),
                receipt.to_string(),
                job
            ],
        )
        .unwrap();
        c.execute(
            "INSERT INTO workflow_steps VALUES(?1,?2,?3,?4,'rejected',?5,?6,?7,?8)",
            params![
                run,
                step_key(&request),
                request.stage,
                hash,
                job,
                raw.to_string(),
                REJECTION,
                now()
            ],
        )
        .unwrap();
        executor
            .store
            .event(&job, "sdk_raw_result", &envelope)
            .unwrap();
        executor
            .store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":hash,"value":raw}),
            )
            .unwrap();
        let base = executor.store.root.join("text-runs").join(&job);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("result.json"), raw.to_string()).unwrap();
        crate::store::atomic_json(&base.join("sdk-compiled.json"),&json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":prepared["compiledPromptSha256"],"schemaSha256":prepared["compiledSchemaSha256"]})).unwrap();
        let mut attempt = FrozenAttempt::prepare(
            &job,
            &run,
            &step_key(&request),
            Backend::CodexSdk,
            request.clone(),
            &snapshot["executorIdentity"],
        )
        .unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        attempt.state = AttemptState::Completed;
        attempt.raw_result = Some(envelope);
        attempt.persist(&base.join("sdk-attempt.json")).unwrap();
        (temp, executor, request, job, raw)
    }

    fn preserved(executor: &SavedTextExecutor, job: &str) -> Value {
        let pair: (String, String) = executor
            .store
            .conn()
            .unwrap()
            .query_row("SELECT snapshot,result FROM jobs WHERE id=?1", [job], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        let hashes: Vec<_> = ["sdk-attempt.json", "sdk-compiled.json", "result.json"]
            .iter()
            .map(|f| {
                crate::store::hash_file(&executor.store.root.join("text-runs").join(job).join(f))
                    .unwrap()
            })
            .collect();
        json!({"snapshotBytes":pair.0,"rawBytes":pair.1,"files":hashes})
    }

    #[tokio::test]
    async fn sdk_f2_episode_replay_accept_restart_preserves_original_evidence_zero_dispatch() {
        let (_temp, executor, request, job, raw) = fixture();
        let before = preserved(&executor, &job);
        assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
        executor.accept(&request, &raw).await.unwrap();
        let adopted = executor.sdk_job(&job).unwrap();
        assert_eq!(adopted["status"], "completed");
        assert_eq!(adopted["receipt"]["migrationPolicy"], POLICY);
        assert_eq!(adopted["prepared"]["sdkDomainResult"], raw);
        let snapshot = executor.request_snapshot(&request).unwrap();
        assert!(executor
            .sdk_f2_migrated_receipt_bound(&adopted, &snapshot)
            .unwrap());
        let restart = SavedTextExecutor {
            store: Store::open(executor.store.root.clone()).unwrap(),
            settings: executor.settings.clone(),
            workflow_id: executor.workflow_id.clone(),
        };
        for _ in 0..2 {
            assert_eq!(restart.execute(request.clone()).await.unwrap(), raw);
            restart.accept(&request, &raw).await.unwrap();
        }
        assert_eq!(preserved(&restart, &job), before);
        assert_eq!(restart.store.jobs().unwrap().len(), 2);
        assert!(
            !restart.store.root.join("sdk-worker").exists(),
            "zero-call replay never starts the SDK worker"
        );
        let n:i64=restart.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='sdk_f2_episode_replay_accepted'",[&job],|r|r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn sdk_f2_episode_replay_secondary_rejection_is_diagnostic_not_retry_permission() {
        let (_temp, executor, request, job, raw) = fixture();
        let before = preserved(&executor, &job);
        for _ in 0..2 {
            assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
            executor
                .reject(&request, &raw, "合成第二级领域拒绝")
                .await
                .unwrap();
        }
        let old = executor.sdk_job(&job).unwrap();
        assert_eq!(old["error"], REJECTION);
        assert_eq!(
            old["receipt"],
            json!({"contract":"desktop-runner/v1","validation":"rejected"})
        );
        assert_eq!(old["status"], "rejected");
        let n:i64=executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='sdk_f2_episode_replay_rejected'",[&job],|r|r.get(0)).unwrap();
        assert_eq!(n, 1);
        executor.accept(&request, &raw).await.unwrap();
        assert_eq!(preserved(&executor, &job), before);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }

    #[tokio::test]
    async fn sdk_f2_episode_replay_rejects_changed_facts_unknown_stage_and_profile() {
        let (_temp, executor, request, job, raw) = fixture();
        let before = preserved(&executor, &job);
        assert!(executor
            .accept(&request, &json!({"ok":true,"facts":["改写事实"]}))
            .await
            .unwrap_err()
            .contains("DOMAIN_RESULT_CHANGED"));
        for change in ["profile", "stage", "source", "rules", "provider"] {
            let mut changed = request.clone();
            match change {
                "profile" => changed.input["modelInputProfile"] = json!("other/v1"),
                "stage" => changed.stage = "foundation:Q1".into(),
                "source" => changed.input["sourceSha256"] = json!("different"),
                "rules" => changed.rule_snapshot["primary"]["sha256"] = json!("different"),
                _ => changed.model = "different".into(),
            }
            assert!(executor.accept(&changed, &raw).await.is_err(), "{change}");
        }
        executor
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                [&job],
            )
            .unwrap();
        assert!(executor.accept(&request, &raw).await.is_err());
        assert_eq!(preserved(&executor, &job), before);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }

    #[test]
    fn sdk_f2_episode_replay_store_proof_and_cas_reject_tampered_or_newer_evidence() {
        let (_temp, executor, request, job, raw) = fixture();
        let snapshot = executor.request_snapshot(&request).unwrap();
        let audit = executor
            .sdk_f2_replay_audit(&request, &snapshot, &executor.sdk_job(&job).unwrap())
            .unwrap();
        assert!(audit_valid(&audit));
        assert!(files_bound(&executor.store.root, &audit));
        for field in [
            "previousError",
            "workflowId",
            "nodeKey",
            "originalJobId",
            "requestHash",
            "originalSnapshotJson",
            "originalResultJson",
            "originalPreparedJson",
            "originalReceiptJson",
            "originalWorkflowSnapshotJson",
            "originalRawEventDataJson",
            "originalSdkRawEventDataJson",
            "originalAttemptSha256",
            "originalCompiledSha256",
            "originalRawTextSha256",
        ] {
            let mut forged = audit.clone();
            forged[field] = json!("changed");
            assert!(
                !executor
                    .store
                    .commit_sdk_f2_episode_replay(&forged, &raw, None)
                    .unwrap_or(false),
                "{field}"
            );
        }
        let c = executor.store.conn().unwrap();
        c.execute(
            "UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",
            [&job],
        )
        .unwrap();
        assert!(!executor
            .store
            .commit_sdk_f2_episode_replay(&audit, &raw, None)
            .unwrap());
        c.execute(
            "UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",
            params![s(&audit, "requestHash"), job],
        )
        .unwrap();
        executor
            .store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":audit["requestHash"],"value":raw}),
            )
            .unwrap();
        assert!(
            !executor
                .store
                .commit_sdk_f2_episode_replay(&audit, &raw, None)
                .unwrap(),
            "even an identical newer raw event invalidates the old CAS proof"
        );
        assert_eq!(executor.sdk_job(&job).unwrap()["status"], "rejected");
    }
}

impl Store {
    /// Only the original SDK F2 node is advanced. Raw/request/transport files are never written.
    fn commit_sdk_f2_episode_replay(
        &self,
        audit: &Value,
        result: &Value,
        reason: Option<&str>,
    ) -> Result<bool> {
        if !audit_valid(audit) || !files_bound(&self.root, audit) {
            return Err("SDK_F2_REPLAY_PROOF_INVALID".into());
        }
        let prepared: Value =
            serde_json::from_str(s(audit, "originalPreparedJson")).map_err(|e| e.to_string())?;
        let receipt: Value =
            serde_json::from_str(s(audit, "originalReceiptJson")).map_err(|e| e.to_string())?;
        let raw: Value =
            serde_json::from_str(s(audit, "originalResultJson")).map_err(|e| e.to_string())?;
        if reason.is_some() && result != &raw {
            return Err("SDK_F2_REPLAY_REJECTION_NOT_ORIGINAL_RAW".into());
        }
        let snapshot: Value =
            serde_json::from_str(s(audit, "originalSnapshotJson")).map_err(|e| e.to_string())?;
        if reason.is_none()
            && !crate::pipeline::foundation_delivery::accepted_scope_reason_raw_matches(
                &snapshot["input"],
                &raw,
                result,
            )
        {
            return Err(
                "SDK_F2_REPLAY_DOMAIN_RESULT_CHANGED: 仅允许原冻结引用规范化，不改事实".into(),
            );
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if !database_bound(&tx, audit, false, &prepared, &receipt, &raw)?
            || !files_bound(&self.root, audit)
        {
            return Ok(false);
        }
        let event = json!({"policy":POLICY,"migrationAudit":audit,"generationRequestsSent":0,"validation":if reason.is_none(){"accepted"}else{"rejected"},"error":reason,"validatedResultSha256":crate::pipeline::digest(&result.to_string()),"originalEvidencePreserved":true});
        let event_kind = if reason.is_none() {
            "sdk_f2_episode_replay_accepted"
        } else {
            "sdk_f2_episode_replay_rejected"
        };
        if reason.is_none() {
            let mut next = prepared;
            next["sdkDomainResult"] = result.clone();
            next["sdkDomainResultSha256"] = json!(crate::pipeline::digest(&result.to_string()));
            let next_receipt = json!({"contract":"desktop-runner/v1","validation":"accepted","migrationPolicy":POLICY,"migrationAudit":audit,"acceptedRequestHash":audit["requestHash"],"validatedResultSha256":crate::pipeline::digest(&result.to_string())});
            if tx.execute("UPDATE jobs SET state='completed',error=NULL,prepared=?1,receipt=?2,updated_at=?3 WHERE id=?4 AND state='rejected'",params![next.to_string(),next_receipt.to_string(),now(),s(audit,"originalJobId")]).map_err(|e|e.to_string())? != 1 { return Err("SDK_F2_REPLAY_CAS".into()); }
            if tx.execute("UPDATE workflow_steps SET state='accepted',error=NULL,result=?1,updated_at=?2 WHERE workflow_id=?3 AND node_key=?4 AND text_job_id=?5 AND state='rejected' AND request_hash=?6",params![result.to_string(),now(),s(audit,"workflowId"),s(audit,"nodeKey"),s(audit,"originalJobId"),s(audit,"requestHash")]).map_err(|e|e.to_string())? != 1 { return Err("SDK_F2_REPLAY_CHECKPOINT_CAS".into()); }
        }
        // A new rejection is diagnostic only: retain the original rejection and
        // original bytes. It does not grant another model attempt or general replay.
        tx.execute("INSERT INTO events(job_id,type,data,created_at) SELECT ?1,?2,?3,?4 WHERE NOT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type=?2 AND data=?3)",params![s(audit,"originalJobId"),event_kind,event.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }
}
