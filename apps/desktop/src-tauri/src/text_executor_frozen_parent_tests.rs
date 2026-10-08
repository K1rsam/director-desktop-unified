//! Synthetic frozen-parent recovery, never creates or executes a CLI binary.
use super::*;

struct Fixture {
    _directory: tempfile::TempDir,
    executor: SavedTextExecutor,
    request: TextRequest,
    job: String,
    snapshot: Value,
    raw: Value,
}

fn fixture(entity: &str, runner: &str, stage: &str, skill: &str, received: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("project")).unwrap();
    store
        .create("offline exact frozen parent", "fixture")
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let old = crate::text_cli::managed_codex_root()
        .unwrap()
        .join(crate::store::id().replace('-', "")[..16].to_string())
        .join("codex.exe");
    assert!(
        !old.exists(),
        "the original managed version is deliberately absent"
    );
    let settings = json!({"textProvider":"codex","codexPath":old,"textCliExecutable":old,
        "textCliVersion":"codex-cli 0.160.0","textModel":"offline","reasoning":"medium"});
    let parent = if runner == "stage2-runner/v1" {
        let stage = entity.split(':').next().unwrap();
        let request = if entity.ends_with(':') {
            json!({"action":stage})
        } else {
            json!({"action":stage,"episodeId":"fixture-episode"})
        };
        json!({"runnerVersion":runner,"stage":stage,"request":request,"settings":settings,"stage2Context":{"settings":settings}})
    } else {
        json!({"runnerVersion":runner,"stage":entity,"input":{"settings":settings}})
    };
    let run = store
        .add_job(
            "workflow",
            entity,
            "local",
            "local",
            &parent,
            "partially_blocked",
        )
        .unwrap();
    let executor = SavedTextExecutor {
        store,
        settings,
        workflow_id: Some(run),
    };
    let mut request = TextRequest {
        stage: stage.into(),
        skill: skill.into(),
        model: "offline".into(),
        reasoning: "medium".into(),
        input: json!({"episodeId":"fixture-episode","asset":{"id":"fixture-asset","variantId":"base"},
            "sourceRows":[{"id":"fixture-row","text":"甲转身。"}],"sourceSha256":crate::pipeline::digest("甲转身。")}),
        schema: json!({"type":"object"}),
        rule_snapshot: json!({"primary":{"id":skill,"version":"synthetic/v1","content":"offline frozen recovery rules"},"methods":[]}),
    };
    if stage == "assetVisualFacts" {
        request.input["coveredPacketRef"] = json!("fixture-original-packet");
    }
    if let Ok(rules) = crate::pipeline::snapshot(skill, &[]) {
        request.rule_snapshot = rules;
    }
    let mut snapshot = executor.request_domain_snapshot(&request).unwrap();
    snapshot["executorIdentity"] = json!({"provider":"codex","executable":s(&executor.settings,"textCliExecutable"),"version":s(&executor.settings,"textCliVersion")});
    let hash = crate::pipeline::digest(&snapshot.to_string());
    let raw = json!({"originalReceived":true,"sourceRefs":["fixture-row"],"stage":stage});
    let state = if received { "received" } else { "failed" };
    let job = executor
        .store
        .add_job(
            "text",
            "original-frozen-node",
            "codex",
            "local",
            &snapshot,
            state,
        )
        .unwrap();
    if received {
        executor
            .store
            .update_job(&job, state, &json!({"result":raw}))
            .unwrap();
        executor
            .store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":hash,"value":raw}),
            )
            .unwrap();
    }
    executor
        .store
        .checkpoint(
            executor.workflow_id.as_deref().unwrap(),
            &step_key(&request),
            stage,
            &hash,
            if received { "running" } else { "failed" },
            Some(&job),
            received.then_some(&raw),
            None,
        )
        .unwrap();
    Fixture {
        _directory: directory,
        executor,
        request,
        job,
        snapshot,
        raw,
    }
}

fn mark_stopped(f: &Fixture) {
    let error = "文本任务完整运行超时；保留已有事件与结果，不自动重跑";
    f.executor
        .store
        .update_job(&f.job, "failed", &json!({"error":error}))
        .unwrap();
    f.executor
        .store
        .checkpoint(
            f.executor.workflow_id.as_deref().unwrap(),
            &step_key(&f.request),
            &f.request.stage,
            &crate::pipeline::digest(&f.snapshot.to_string()),
            "failed",
            Some(&f.job),
            None,
            Some(error),
        )
        .unwrap();
    f.executor
        .store
        .event(
            &f.job,
            "codex_heartbeat",
            &json!({"format":"codex-runtime-heartbeat/v1","phase":"failed",
        "processAlive":false,"pid":42,"stdinBytes":100,"stdinTotalBytes":100,"resultBytes":0}),
        )
        .unwrap();
}

fn job_count(store: &Store) -> u64 {
    store
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM jobs WHERE kind='text'", [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[tokio::test]
async fn stage2_managed_exact_parent_accepts_and_replays_original_raw_after_restart() {
    for entity in [
        "images.design",
        "images.produce",
        "images.design:",
        "images.produce:",
        "images.design:fixture-episode",
        "images.produce:fixture-episode",
    ] {
        for (stage, skill) in [
            ("assetDesign", "asset-art-director"),
            ("assetVisualFacts", "asset-visual-facts-extractor"),
        ] {
            let f = fixture(entity, "stage2-runner/v1", stage, skill, true);
            assert_eq!(f.executor.request_snapshot(&f.request).unwrap(), f.snapshot);
            let original_event: String = f.executor.store.conn().unwrap().query_row(
                "SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1", [&f.job], |row| row.get(0)).unwrap();
            let normalized = json!({"trustedOfflineMaterialization":true,"sourceRefs":["fixture-row"],"stage":stage});
            f.executor.accept(&f.request, &normalized).await.unwrap();
            let accepted = f.executor.store.job(&f.job).unwrap();
            assert_eq!(accepted["status"], "completed");
            assert_eq!(accepted["snapshot"], f.snapshot);
            let resumed = SavedTextExecutor {
                store: Store::open(f.executor.store.root.clone()).unwrap(),
                settings: f.executor.settings.clone(),
                workflow_id: f.executor.workflow_id.clone(),
            };
            assert_eq!(resumed.execute(f.request.clone()).await.unwrap(), f.raw);
            resumed.accept(&f.request, &normalized).await.unwrap();
            assert_eq!(resumed.store.job(&f.job).unwrap(), accepted);
            let event: String = resumed.store.conn().unwrap().query_row(
                "SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1", [&f.job], |row| row.get(0)).unwrap();
            assert_eq!(event, original_event);
            assert_eq!(job_count(&resumed.store), 1);
            assert!(!Path::new(s(&resumed.settings, "textCliExecutable")).exists());
        }
    }
}

#[tokio::test]
async fn frozen_parent_rejects_foreign_stage_skill_runner_and_arbitrary_workflow_without_replay() {
    for (entity, runner, stage, skill) in [
        (
            "images.arbitrary",
            "stage2-runner/v1",
            "assetDesign",
            "asset-art-director",
        ),
        (
            "images.design:foreign-episode",
            "stage2-runner/v1",
            "assetDesign",
            "asset-art-director",
        ),
        (
            "images.design",
            "desktop-runner/v1",
            "assetDesign",
            "asset-art-director",
        ),
        (
            "images.produce",
            "stage2-runner/v2",
            "assetDesign",
            "asset-art-director",
        ),
        (
            "images.design",
            "stage2-runner/v1",
            "storyboardCreate",
            "unit-performance-director",
        ),
        (
            "images.produce",
            "stage2-runner/v1",
            "assetDesign",
            "unknown-art-skill",
        ),
        (
            "foundation.run",
            "desktop-runner/v1",
            "assetDesign",
            "asset-art-director",
        ),
        (
            "storyboard.run",
            "desktop-runner/v1",
            "storyboardRefine",
            "director-refinement",
        ),
        (
            "storyboard.refine",
            "desktop-runner/v1",
            "storyboardFinish",
            "episode-finishing-editor",
        ),
        (
            "storyboard.refine",
            "desktop-runner/v2",
            "storyboardRefine",
            "director-refinement",
        ),
    ] {
        let f = fixture(entity, runner, stage, skill, true);
        let before = f.executor.store.job(&f.job).unwrap();
        let error = f.executor.execute(f.request.clone()).await.unwrap_err();
        assert!(
            error.contains("CODEX_FROZEN_NODE_MISMATCH"),
            "{entity}/{runner}/{stage}: {error}"
        );
        assert_eq!(f.executor.store.job(&f.job).unwrap(), before);
        assert_eq!(job_count(&f.executor.store), 1);
        assert!(!Path::new(s(&f.executor.settings, "textCliExecutable")).exists());
    }
}

#[tokio::test]
async fn storyboard_refine_stopped_transport_recovery_preserves_hash_evidence_budget_and_accepted_cache(
) {
    let f = fixture(
        "storyboard.refine",
        "desktop-runner/v1",
        "storyboardRefine",
        "director-refinement",
        false,
    );
    mark_stopped(&f);
    let before = f.executor.store.job(&f.job).unwrap();
    let retry = f
        .executor
        .prepare_transport_request(f.request.clone())
        .await
        .unwrap();
    let proof = &retry.input["textTransportRetry"];
    assert_eq!(proof["originJobId"], f.job);
    assert_eq!(
        proof["originRequestHash"],
        crate::pipeline::digest(&f.snapshot.to_string())
    );
    assert_eq!(proof["attempt"], 1);
    assert_eq!(proof["maximumAttempts"], 1);
    assert_ne!(step_key(&retry), step_key(&f.request));
    assert_eq!(
        cli_prompt(&retry, &retry.input),
        cli_prompt(&f.request, &f.request.input)
    );
    f.executor.validate_transport_retry(&retry).unwrap();
    let mut recovered = f.executor.request_domain_snapshot(&retry).unwrap();
    recovered["executorIdentity"] = f.snapshot["executorIdentity"].clone();
    let hash = crate::pipeline::digest(&recovered.to_string());
    let job = f
        .executor
        .store
        .add_job(
            "text",
            "offline exact refine recovery",
            "codex",
            "local",
            &recovered,
            "completed",
        )
        .unwrap();
    f.executor.store.update_job(&job, "completed", &json!({"result":f.raw,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
    f.executor
        .store
        .event(
            &job,
            "text_received_raw",
            &json!({"requestHash":hash,"value":f.raw}),
        )
        .unwrap();
    f.executor
        .store
        .checkpoint(
            f.executor.workflow_id.as_deref().unwrap(),
            &step_key(&retry),
            &retry.stage,
            &hash,
            "accepted",
            Some(&job),
            Some(&f.raw),
            None,
        )
        .unwrap();
    let replay = f
        .executor
        .prepare_transport_request(f.request.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&retry).unwrap()
    );
    assert_eq!(f.executor.execute(replay.clone()).await.unwrap(), f.raw);
    f.executor.accept(&replay, &f.raw).await.unwrap();
    assert!(accepted_transport_retry_proof(
        &f.executor.store,
        f.executor.workflow_id.as_deref().unwrap(),
        &replay
    )
    .unwrap());
    assert_eq!(f.executor.store.job(&f.job).unwrap(), before);
    assert_eq!(job_count(&f.executor.store), 2);
    assert!(!Path::new(s(&f.executor.settings, "textCliExecutable")).exists());
}

#[tokio::test]
async fn storyboard_refine_failed_retry_stays_once_only_and_drift_cannot_forge_a_new_budget() {
    let f = fixture(
        "storyboard.refine",
        "desktop-runner/v1",
        "storyboardRefine",
        "director-refinement",
        false,
    );
    mark_stopped(&f);
    let original = f.executor.store.job(&f.job).unwrap();
    let first = f
        .executor
        .prepare_transport_request(f.request.clone())
        .await
        .unwrap();
    let mut snapshot = f.executor.request_domain_snapshot(&first).unwrap();
    snapshot["executorIdentity"] = f.snapshot["executorIdentity"].clone();
    let hash = crate::pipeline::digest(&snapshot.to_string());
    let error = "文本任务完整运行超时；保留已有事件与结果，不自动重跑";
    let job = f
        .executor
        .store
        .add_job(
            "text",
            "offline failed refine retry",
            "codex",
            "local",
            &snapshot,
            "failed",
        )
        .unwrap();
    f.executor
        .store
        .update_job(&job, "failed", &json!({"error":error}))
        .unwrap();
    f.executor
        .store
        .checkpoint(
            f.executor.workflow_id.as_deref().unwrap(),
            &step_key(&first),
            &first.stage,
            &hash,
            "failed",
            Some(&job),
            None,
            Some(error),
        )
        .unwrap();
    f.executor
        .store
        .event(
            &job,
            "codex_heartbeat",
            &json!({"format":"codex-runtime-heartbeat/v1","phase":"failed",
        "processAlive":false,"pid":43,"stdinBytes":100,"stdinTotalBytes":100,"resultBytes":0}),
        )
        .unwrap();
    let failed = f.executor.store.job(&job).unwrap();
    let prepared = f
        .executor
        .prepare_transport_request(f.request.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&prepared).unwrap(),
        serde_json::to_value(&first).unwrap()
    );
    assert!(f
        .executor
        .execute(prepared)
        .await
        .unwrap_err()
        .contains("TEXT_TRANSPORT_RETRY_EXHAUSTED"));
    let mut drift = f.request.clone();
    drift.input["sourceRows"][0]["text"] = json!("不得改写的另一个原文。");
    assert!(f
        .executor
        .prepare_transport_request(drift)
        .await
        .unwrap_err()
        .contains("CODEX_FROZEN_NODE_MISMATCH"));
    let mut forged = first;
    forged.input["textTransportRetry"]["originJobId"] = json!("different-old-attempt");
    assert!(f
        .executor
        .prepare_transport_request(forged)
        .await
        .unwrap_err()
        .contains("TEXT_TRANSPORT_RETRY_UNBOUND"));
    assert_eq!(f.executor.store.job(&f.job).unwrap(), original);
    assert_eq!(f.executor.store.job(&job).unwrap(), failed);
    assert_eq!(job_count(&f.executor.store), 2);
    assert!(!Path::new(s(&f.executor.settings, "textCliExecutable")).exists());
}

#[tokio::test]
async fn storyboard_refine_unknown_live_received_or_unstopped_attempt_never_gets_a_retry() {
    for mutation in [
        "unknown",
        "live",
        "alive",
        "missingHeartbeat",
        "partialInput",
        "raw",
        "turnCompleted",
        "file",
    ] {
        let f = fixture(
            "storyboard.refine",
            "desktop-runner/v1",
            "storyboardRefine",
            "director-refinement",
            false,
        );
        mark_stopped(&f);
        let c = f.executor.store.conn().unwrap();
        match mutation {
            "unknown" | "live" => {
                c.execute(
                    "UPDATE jobs SET state=?1 WHERE id=?2",
                    params![
                        if mutation == "unknown" {
                            "submission_unknown"
                        } else {
                            "submitting"
                        },
                        f.job
                    ],
                )
                .unwrap();
            }
            "alive" | "partialInput" => {
                let mut event = json!({"format":"codex-runtime-heartbeat/v1","phase":"failed","processAlive":false,"pid":42,"stdinBytes":100,"stdinTotalBytes":100,"resultBytes":0});
                if mutation == "alive" {
                    event["processAlive"] = json!(true);
                } else {
                    event["stdinBytes"] = json!(99);
                }
                f.executor
                    .store
                    .event(&f.job, "codex_heartbeat", &event)
                    .unwrap();
            }
            "missingHeartbeat" => {
                c.execute(
                    "DELETE FROM events WHERE job_id=?1 AND type='codex_heartbeat'",
                    [&f.job],
                )
                .unwrap();
            }
            "raw" => {
                f.executor.store.event(&f.job, "text_received_raw", &json!({"requestHash":crate::pipeline::digest(&f.snapshot.to_string()),"value":f.raw})).unwrap();
            }
            "turnCompleted" => {
                f.executor
                    .store
                    .event(&f.job, "codex_event", &json!({"type":"turn.completed"}))
                    .unwrap();
            }
            "file" => {
                let directory = f.executor.store.root.join("text-runs").join(&f.job);
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(directory.join("result.json"), b"{}").unwrap();
            }
            _ => unreachable!(),
        }
        let before = f.executor.store.job(&f.job).unwrap();
        let prepared = f
            .executor
            .prepare_transport_request(f.request.clone())
            .await
            .unwrap();
        assert!(
            prepared.input.get("textTransportRetry").is_none(),
            "{mutation}"
        );
        let error = f.executor.execute(prepared).await.unwrap_err();
        assert!(
            error.contains("CODEX_EXISTING_ATTEMPT_UNRESOLVED"),
            "{mutation}: {error}"
        );
        assert_eq!(f.executor.store.job(&f.job).unwrap(), before);
        assert_eq!(job_count(&f.executor.store), 1);
        assert!(!Path::new(s(&f.executor.settings, "textCliExecutable")).exists());
    }
}
