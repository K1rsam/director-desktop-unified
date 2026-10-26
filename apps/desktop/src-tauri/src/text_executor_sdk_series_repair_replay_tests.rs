use super::super::tests::{adopt_raw, reject_synthetic};
use super::*;

#[tokio::test]
async fn sdk_evidence_cache_keeps_live_guards_inside_one_synchronous_scope() {
    for fault in ["compiled-file", "raw-file", "event", "checkpoint", "parent"] {
        let (_temp, executor, request, raw, id, _review, _successor) = fixture().await;
        // Cache only pure calculations. Even inside the same synchronous
        // scope, changes to files or database evidence must still be detected.
        let _scope = crate::text_executor::sdk_evidence_cache::scope();
        let job = executor.sdk_job(&id).unwrap();
        assert_eq!(executor.sdk_raw(&job, &job["snapshot"]).unwrap(), raw);
        match fault {
            "compiled-file" | "raw-file" => {
                let file = if fault == "compiled-file" {
                    "sdk-compiled.json"
                } else {
                    "result.json"
                };
                std::fs::write(
                    executor.store.root.join("text-runs").join(&id).join(file),
                    "{}",
                )
                .unwrap();
            }
            "event" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type='text_received_raw'",
                        [&id],
                    )
                    .unwrap();
            }
            "checkpoint" => {
                executor.store.conn().unwrap().execute("UPDATE workflow_steps SET request_hash='drift' WHERE workflow_id=?1 AND text_job_id=?2", params![executor.workflow_id.as_deref().unwrap(), id]).unwrap();
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
            _ => unreachable!(),
        }
        // Keep the same separate ownership and raw-evidence gates used by SDK
        // execution/commit; the pure cache cannot authorize any of them.
        let result = executor
            .sdk_parent_binding()
            .and_then(|_| executor.request_snapshot(&request))
            .and_then(|snapshot| {
                executor.sdk_checkpoint_binding(&request, &snapshot)?;
                executor.sdk_raw(&job, &snapshot)
            });
        assert!(result.is_err(), "{fault}");
    }
}

fn save(
    executor: &SavedTextExecutor,
    request: &TextRequest,
    raw: &Value,
    rejected: bool,
) -> String {
    let snapshot = executor.request_snapshot(request).unwrap();
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
    adopt_raw(executor, request, &snapshot, &job, raw);
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
    if rejected {
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
        reject_synthetic(executor, request, &snapshot, &job, REJECTION);
    }
    job
}

async fn fixture() -> (
    tempfile::TempDir,
    SavedTextExecutor,
    TextRequest,
    Value,
    String,
    String,
    String,
) {
    let (mut input, request, raw, reviewer, review_raw) =
        crate::pipeline::series::series_repair_replay_fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store
        .create("frozen review repair synthetic", "repair-fixture")
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
    let review = save(&executor, &reviewer, &review_raw, false);
    let job = save(&executor, &request, &raw, true);
    // A later already-spent validation round remains a distinct, immutable
    // rejected record. Recovering the origin neither resets nor deletes it.
    let mut later = request.clone();
    later.input["executionRoleId"] = json!(format!(
        "{}/validation-repair/1",
        s(&request.input, "executionRoleId")
    ));
    later.input["validationRepair"] = json!({"version":"series-domain-repair/v1","attempt":1,"maximumAttempts":2,
        "originRequestSha256":request_hash(&request).unwrap(),"previousRequestSha256":request_hash(&request).unwrap(),
        "previousRawSha256":hash(&raw),"problem":REJECTION,"candidate":raw});
    let successor = save(&executor, &later, &raw, true);
    (temp, executor, request, raw, job, review, successor)
}

#[tokio::test]
async fn sdk_series_repair_plan_revalidation_preserves_raw_budget_and_reopens_without_dispatch() {
    let (_temp, executor, request, raw, job, review, successor) = fixture().await;
    let old = executor.sdk_job(&job).unwrap();
    let before_review = executor.sdk_job(&review).unwrap();
    let before_successor = executor.sdk_job(&successor).unwrap();
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
    assert_eq!(
        executor.sdk_job(&job).unwrap(),
        old,
        "reading a rejected raw cannot adopt it"
    );
    executor.accept(&request, &raw).await.unwrap();
    let adopted = executor.sdk_job(&job).unwrap();
    assert_eq!(adopted["status"], "completed");
    assert_eq!(adopted["snapshot"], old["snapshot"]);
    assert_eq!(adopted["result"], old["result"]);
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    for _ in 0..2 {
        assert_eq!(reopened.execute(request.clone()).await.unwrap(), raw);
        assert_eq!(
            reopened
                .sdk_execute(&request, &adopted["snapshot"])
                .await
                .unwrap(),
            raw
        );
        reopened.accept(&request, &raw).await.unwrap();
    }
    let after = events(&reopened.store.conn().unwrap(), &job).unwrap();
    assert!(events_preserved(&old_events, &after));
    let audit: Vec<_> = after
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == REPLAY_EVENT)
        .collect();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["data"]["originalJob"]["error"], REJECTION);
    assert_eq!(audit[0]["data"]["generationRequestsSent"], 0);
    assert_eq!(files(&reopened.store.root, &job).unwrap(), old_files);
    assert_eq!(reopened.sdk_job(&review).unwrap(), before_review);
    assert_eq!(reopened.sdk_job(&successor).unwrap(), before_successor);
    assert_eq!(reopened.store.jobs().unwrap().len(), count);
    assert!(!reopened.store.root.join("sdk-worker").exists());
    let later: TextRequest = serde_json::from_value(before_successor["snapshot"].clone()).unwrap();
    assert!(
        reopened.accept(&later, &raw).await.is_err(),
        "successor is not a broadly enabled rejected cache"
    );
}

#[tokio::test]
async fn sdk_series_repair_plan_revalidation_refuses_other_error_unknown_and_frozen_evidence_drift()
{
    for fault in [
        "reason",
        "unknown",
        "raw",
        "raw-event",
        "review-status",
        "review-event",
        "review-file",
        "parent",
        "checkpoint",
        "callback",
    ] {
        let (_temp, executor, request, raw, job, review, _successor) = fixture().await;
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
            "raw" => std::fs::write(
                executor
                    .store
                    .root
                    .join("text-runs")
                    .join(&job)
                    .join("result.json"),
                "{}",
            )
            .unwrap(),
            "raw-event" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_raw_result'",
                        [&job],
                    )
                    .unwrap();
            }
            "review-status" => executor
                .store
                .update_job(&review, "rejected", &json!({"error":"other"}))
                .unwrap(),
            "review-event" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type='text_received_raw'",
                        [&review],
                    )
                    .unwrap();
            }
            "review-file" => std::fs::write(
                executor
                    .store
                    .root
                    .join("text-runs")
                    .join(&review)
                    .join("result.json"),
                "{}",
            )
            .unwrap(),
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
            "callback" => callback["plan"]["emotionArc"] = json!("changed"),
            _ => unreachable!(),
        }
        let before = executor.store.jobs().unwrap();
        let evidence = events(&executor.store.conn().unwrap(), &job).unwrap();
        assert!(
            executor.accept(&request, &callback).await.is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        assert_eq!(
            events(&executor.store.conn().unwrap(), &job).unwrap(),
            evidence,
            "{fault}"
        );
    }
}

#[tokio::test]
async fn sdk_series_repair_plan_revalidation_reopen_requires_exact_single_audit_and_review() {
    for fault in ["audit", "duplicate", "receipt", "original-event", "review"] {
        let (_temp, executor, request, raw, job, review, _successor) = fixture().await;
        executor.accept(&request, &raw).await.unwrap();
        match fault {
            "audit" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type=?2",
                        params![job, REPLAY_EVENT],
                    )
                    .unwrap();
            }
            "duplicate" => {
                executor.store.conn().unwrap().execute("INSERT INTO events(job_id,type,data,created_at) SELECT job_id,type,data,created_at FROM events WHERE job_id=?1 AND type=?2",params![job,REPLAY_EVENT]).unwrap();
            }
            "receipt" => {
                let mut receipt = executor.sdk_job(&job).unwrap()["receipt"].clone();
                receipt["migrationAuditSha256"] = json!("0".repeat(64));
                executor
                    .store
                    .update_job(&job, "completed", &json!({"receipt":receipt}))
                    .unwrap();
            }
            "original-event" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_raw_result'",
                        [&job],
                    )
                    .unwrap();
            }
            "review" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute("UPDATE jobs SET result='{}' WHERE id=?1", [&review])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = executor.store.jobs().unwrap();
        let snapshot = executor.sdk_job(&job).unwrap()["snapshot"].clone();
        assert!(
            executor.sdk_execute(&request, &snapshot).await.is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
    }
}
