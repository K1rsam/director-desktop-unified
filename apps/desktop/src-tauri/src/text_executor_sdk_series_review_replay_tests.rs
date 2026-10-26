use super::super::tests::{adopt_raw, reject_synthetic};
use super::*;

async fn fixture() -> (
    tempfile::TempDir,
    SavedTextExecutor,
    TextRequest,
    Value,
    String,
) {
    let (mut input, request, raw) =
        crate::pipeline::series::series_review_entry_replay_fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store
        .create("review entry synthetic", "review-entry-fixture")
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let mut settings = input["settings"].clone();
    settings["textProvider"] = json!("codex_sdk");
    settings["textModel"] = json!("gpt-6.1-sol");
    settings["reasoning"] = json!("medium");
    settings["textSdkIdentity"] = crate::text_sdk::test_refusal_identity(temp.path());
    input["settings"] = settings.clone();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1","input":input}),
            "partially_blocked",
        )
        .unwrap();
    let executor = SavedTextExecutor {
        store,
        settings,
        workflow_id: Some(run),
    };
    let snapshot = executor.request_snapshot(&request).unwrap();
    let job = executor
        .store
        .add_job(
            "text",
            &request.stage,
            "codex_sdk",
            "synthetic-never-dispatch",
            &snapshot,
            "queued",
        )
        .unwrap();
    adopt_raw(&executor, &request, &snapshot, &job, &raw);
    let attempt = FrozenAttempt::load(
        &executor
            .store
            .root
            .join("text-runs")
            .join(&job)
            .join("sdk-attempt.json"),
    )
    .unwrap();
    executor
        .store
        .event(&job, "sdk_raw_result", attempt.raw_result.as_ref().unwrap())
        .unwrap();
    let mut prepared = executor.sdk_job(&job).unwrap()["prepared"].clone();
    prepared.as_object_mut().unwrap().remove("sdkDomainResult");
    prepared
        .as_object_mut()
        .unwrap()
        .remove("sdkDomainResultSha256");
    executor
        .store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET prepared=?1 WHERE id=?2",
            params![prepared.to_string(), job],
        )
        .unwrap();
    reject_synthetic(&executor, &request, &snapshot, &job, REJECTION);
    (temp, executor, request, raw, job)
}

#[tokio::test]
async fn sdk_series_review_entry_revalidation_keeps_revise_raw_and_reopens_without_dispatch() {
    let (_temp, executor, request, raw, job) = fixture().await;
    let old = executor.sdk_job(&job).unwrap();
    let old_files = files(&executor.store.root, &job).unwrap();
    let old_events = events(&executor.store.conn().unwrap(), &job).unwrap();
    let count = executor.store.jobs().unwrap().len();
    assert_eq!(
        executor
            .sdk_execute(&request, &old["snapshot"])
            .await
            .unwrap(),
        raw
    );
    assert_eq!(executor.sdk_job(&job).unwrap(), old);
    executor.accept(&request, &raw).await.unwrap();
    let accepted = executor.sdk_job(&job).unwrap();
    assert_eq!(accepted["status"], "completed");
    assert_eq!(
        accepted["result"]["conclusion"], "revise",
        "accepting review evidence must not fake a pass"
    );
    assert_eq!(accepted["snapshot"], old["snapshot"]);
    assert_eq!(accepted["result"], old["result"]);
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    for _ in 0..2 {
        assert_eq!(reopened.execute(request.clone()).await.unwrap(), raw);
        reopened.accept(&request, &raw).await.unwrap();
    }
    let after = events(&reopened.store.conn().unwrap(), &job).unwrap();
    assert!(events_preserved(&old_events, &after));
    let proofs: Vec<_> = after
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == REPLAY_EVENT)
        .collect();
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0]["data"]["originalJob"]["error"], REJECTION);
    assert_eq!(proofs[0]["data"]["generationRequestsSent"], 0);
    assert_eq!(files(&reopened.store.root, &job).unwrap(), old_files);
    assert_eq!(reopened.store.jobs().unwrap().len(), count);
    assert!(!reopened.store.root.join("sdk-worker").exists());
}

#[tokio::test]
async fn sdk_series_review_entry_revalidation_refuses_unknown_other_error_and_evidence_drift() {
    for fault in [
        "reason",
        "unknown",
        "raw",
        "raw-event",
        "received-event",
        "compiled",
        "attempt",
        "parent",
        "checkpoint",
        "callback",
    ] {
        let (_temp, executor, request, raw, job) = fixture().await;
        let mut callback = raw.clone();
        match fault {
            "reason" => {
                let snapshot = executor.sdk_job(&job).unwrap()["snapshot"].clone();
                reject_synthetic(&executor, &request, &snapshot, &job, "SERIES_OTHER_ERROR");
            }
            "unknown" => executor
                .store
                .update_job(&job, "submission_unknown", &json!({}))
                .unwrap(),
            "raw" | "compiled" | "attempt" => {
                let name = match fault {
                    "raw" => "result.json",
                    "compiled" => "sdk-compiled.json",
                    _ => "sdk-attempt.json",
                };
                std::fs::write(
                    executor.store.root.join("text-runs").join(&job).join(name),
                    "{}",
                )
                .unwrap();
            }
            "raw-event" | "received-event" => {
                let kind = if fault == "raw-event" {
                    "sdk_raw_result"
                } else {
                    "text_received_raw"
                };
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type=?2",
                        params![job, kind],
                    )
                    .unwrap();
            }
            "parent" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE jobs SET snapshot='{}' WHERE id=?1",
                        [executor.workflow_id.as_deref().unwrap()],
                    )
                    .unwrap();
            }
            "checkpoint" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",
                        [&job],
                    )
                    .unwrap();
            }
            "callback" => callback["conclusion"] = json!("pass"),
            _ => unreachable!(),
        }
        let before = executor.sdk_job(&job).unwrap();
        assert!(
            executor.accept(&request, &callback).await.is_err(),
            "{fault}"
        );
        assert_eq!(executor.sdk_job(&job).unwrap(), before, "{fault}");
        assert!(!executor.store.root.join("sdk-worker").exists());
    }
}

#[tokio::test]
async fn sdk_series_review_entry_revalidation_rechecks_migration_proof_after_acceptance() {
    let (_temp, executor, request, raw, job) = fixture().await;
    executor.accept(&request, &raw).await.unwrap();
    executor
        .store
        .conn()
        .unwrap()
        .execute(
            "UPDATE events SET data='{}' WHERE job_id=?1 AND type=?2",
            params![job, REPLAY_EVENT],
        )
        .unwrap();
    assert!(executor.execute(request).await.is_err());
    assert!(!executor.store.root.join("sdk-worker").exists());
}

#[tokio::test]
async fn sdk_series_review_entry_public_cache_rechecks_original_files_and_receipt() {
    for fault in ["raw-file", "compiled-file", "attempt-file", "receipt"] {
        let (_temp, executor, request, raw, job) = fixture().await;
        executor.accept(&request, &raw).await.unwrap();
        match fault {
            "receipt" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute("UPDATE jobs SET receipt='{}' WHERE id=?1", [&job])
                    .unwrap();
            }
            _ => {
                let filename = match fault {
                    "raw-file" => "result.json",
                    "compiled-file" => "sdk-compiled.json",
                    _ => "sdk-attempt.json",
                };
                std::fs::write(
                    executor
                        .store
                        .root
                        .join("text-runs")
                        .join(&job)
                        .join(filename),
                    "{}",
                )
                .unwrap();
            }
        }
        let before = executor.store.jobs().unwrap();
        assert!(executor.execute(request).await.is_err(), "{fault}");
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        assert!(!executor.store.root.join("sdk-worker").exists());
    }
}
