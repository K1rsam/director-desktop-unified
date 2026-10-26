use super::*;

#[path = "text_executor_series_frame_render_schema_tests.rs"]
mod schema_recovery;

async fn fixture() -> (
    tempfile::TempDir,
    SavedTextExecutor,
    TextRequest,
    TextRequest,
    Value,
) {
    let (mut input, legacy, projected, _, review, _) =
        crate::pipeline::series::series_frame_render_fixture().await;
    input["settings"]["textProvider"] = json!("codex_sdk");
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store
        .create("frame render fixture", "frame-render-test")
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"input":input}),
            "running",
        )
        .unwrap();
    let saved = SavedTextExecutor {
        store,
        settings: input["settings"].clone(),
        workflow_id: Some(run),
    };
    let request: TextRequest = serde_json::from_value(review["request"].clone()).unwrap();
    persist(&saved, &request, &review["raw"], "completed", true);
    (temp, saved, legacy, projected, review)
}
fn persist(
    saved: &SavedTextExecutor,
    request: &TextRequest,
    raw: &Value,
    state: &str,
    checkpoint: bool,
) -> String {
    let mut snapshot = saved.request_domain_snapshot(request).unwrap();
    snapshot["executorIdentity"] = json!({"provider":"codex_sdk","synthetic":true});
    let hash = crate::pipeline::digest(&snapshot.to_string());
    let job = saved
        .store
        .add_job(
            "text",
            &request.stage,
            "codex_sdk",
            &request.model,
            &snapshot,
            state,
        )
        .unwrap();
    let accepted = state == "completed";
    // Match normal SavedTextExecutor::accept, which does not add a migration
    // hash to the ordinary desktop-runner receipt.
    saved.store.update_job(&job,state,&json!({"result":raw,"receipt":{"contract":"desktop-runner/v1","validation":if accepted{"accepted"}else{"pending"}}})).unwrap();
    saved
        .store
        .event(
            &job,
            "text_received_raw",
            &json!({"requestHash":hash,"value":raw}),
        )
        .unwrap();
    if checkpoint {
        saved
            .store
            .checkpoint(
                saved.workflow_id.as_deref().unwrap(),
                &step_key(request),
                &request.stage,
                &hash,
                if accepted { "accepted" } else { state },
                Some(&job),
                Some(raw),
                None,
            )
            .unwrap();
    }
    job
}
fn exact(a: &TextRequest, b: &TextRequest) {
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
}

#[tokio::test]
async fn series_frame_render_saved_selector_is_readonly_and_preserves_all_frozen_states() {
    let (_temp, saved, legacy, projected, review) = fixture().await;
    let before = saved.store.jobs().unwrap();
    exact(
        &saved
            .saved_series_frame_render(legacy.clone(), projected.clone(), &review)
            .unwrap(),
        &projected,
    );
    assert_eq!(saved.store.jobs().unwrap(), before);
    for use_new in [false, true] {
        for state in [
            "queued",
            "running",
            "submission_unknown",
            "received",
            "rejected",
            "completed",
        ] {
            let (_temp, saved, legacy, projected, review) = fixture().await;
            let selected = if use_new {
                projected.clone()
            } else {
                legacy.clone()
            };
            persist(&saved, &selected, &json!({"synthetic":true}), state, true);
            let before = saved.store.jobs().unwrap();
            let reopened = SavedTextExecutor {
                store: Store::open(saved.store.root.clone()).unwrap(),
                settings: saved.settings.clone(),
                workflow_id: saved.workflow_id.clone(),
            };
            exact(
                &reopened
                    .saved_series_frame_render(legacy, projected, &review)
                    .unwrap(),
                &selected,
            );
            assert_eq!(saved.store.jobs().unwrap(), before);
            assert!(!saved.store.root.join("sdk-worker").exists());
        }
    }
}

#[tokio::test]
async fn series_frame_render_saved_selector_holds_drift_orphan_and_unaccepted_review() {
    for fault in [
        "orphan",
        "checkpoint",
        "snapshot",
        "provider",
        "review-event",
        "review-receipt",
        "parent",
        "projected",
    ] {
        let (_temp, saved, legacy, mut projected, review) = fixture().await;
        let c = saved.store.conn().unwrap();
        match fault {
            "orphan" => {
                persist(&saved, &legacy, &Value::Null, "submission_unknown", false);
            }
            "checkpoint" => {
                let job = persist(&saved, &legacy, &Value::Null, "submission_unknown", true);
                c.execute(
                    "UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",
                    [job],
                )
                .unwrap();
            }
            "snapshot" => {
                let job = persist(&saved, &legacy, &Value::Null, "received", true);
                c.execute("UPDATE jobs SET snapshot='{}' WHERE id=?1", [job])
                    .unwrap();
            }
            "provider" => {
                let job = persist(&saved, &legacy, &Value::Null, "received", true);
                c.execute("UPDATE jobs SET provider='different' WHERE id=?1", [job])
                    .unwrap();
            }
            "review-event" => {
                c.execute(
                    "UPDATE events SET data='{}' WHERE type='text_received_raw'",
                    [],
                )
                .unwrap();
            }
            "review-receipt" => {
                c.execute(
                    "UPDATE jobs SET state='submission_unknown' WHERE kind='text'",
                    [],
                )
                .unwrap();
            }
            "parent" => {
                c.execute("UPDATE jobs SET snapshot='{}' WHERE kind='workflow'", [])
                    .unwrap();
            }
            "projected" => {
                projected.input["seriesFrameRenderRepair"]["ratio"] = json!("9:16");
            }
            _ => unreachable!(),
        }
        let before = saved.store.jobs().unwrap();
        assert!(
            saved
                .saved_series_frame_render(legacy, projected, &review)
                .is_err(),
            "{fault}"
        );
        assert_eq!(saved.store.jobs().unwrap(), before, "{fault}");
        assert!(!saved.store.root.join("sdk-worker").exists());
    }
}

#[tokio::test]
async fn series_frame_render_ordinary_and_hash_bound_review_receipts_reopen_without_rebuying_review(
) {
    for with_hash in [false, true] {
        let (_temp, saved, legacy, projected, review) = fixture().await;
        if with_hash {
            let c = saved.store.conn().unwrap();
            let (job, hash): (String, String) = c
                .query_row(
                    "SELECT text_job_id,request_hash FROM workflow_steps WHERE workflow_id=?1",
                    [saved.workflow_id.as_deref().unwrap()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            c.execute(
                "UPDATE jobs SET receipt=?1 WHERE id=?2",
                params![
                    json!({"contract":"desktop-runner/v1","validation":"accepted","acceptedRequestHash":hash}).to_string(),
                    job
                ],
            )
            .unwrap();
        }
        let before = saved.store.jobs().unwrap();
        let reopened = SavedTextExecutor {
            store: Store::open(saved.store.root.clone()).unwrap(),
            settings: saved.settings.clone(),
            workflow_id: saved.workflow_id.clone(),
        };
        for _ in 0..2 {
            exact(
                &reopened
                    .saved_series_frame_render(legacy.clone(), projected.clone(), &review)
                    .unwrap(),
                &projected,
            );
        }
        assert_eq!(saved.store.jobs().unwrap(), before);
        let c = saved.store.conn().unwrap();
        let counts: (i64, i64, i64) = c
            .query_row(
                "SELECT (SELECT COUNT(*) FROM jobs WHERE kind='text'),(SELECT COUNT(*) FROM workflow_steps),(SELECT COUNT(*) FROM events WHERE type='text_received_raw')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(counts, (1, 1, 1));
        assert!(!saved.store.root.join("sdk-worker").exists());
    }
}

#[tokio::test]
async fn series_frame_render_review_receipt_missing_hash_is_only_valid_for_plain_acceptance() {
    for fault in [
        "null-hash",
        "empty-hash",
        "wrong-hash",
        "migration-without-hash",
        "extended-without-hash",
        "wrong-contract",
        "pending-validation",
    ] {
        let (_temp, saved, legacy, projected, review) = fixture().await;
        let mut receipt = json!({"contract":"desktop-runner/v1","validation":"accepted"});
        match fault {
            "null-hash" => receipt["acceptedRequestHash"] = Value::Null,
            "empty-hash" => receipt["acceptedRequestHash"] = json!(""),
            "wrong-hash" => receipt["acceptedRequestHash"] = json!("0".repeat(64)),
            "migration-without-hash" => receipt["migrationPolicy"] = json!("synthetic-migration"),
            "extended-without-hash" => receipt["revalidatedFrom"] = json!("rejected"),
            "wrong-contract" => receipt["contract"] = json!("unknown-contract"),
            "pending-validation" => receipt["validation"] = json!("pending"),
            _ => unreachable!(),
        }
        saved
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET receipt=?1 WHERE kind='text'",
                [receipt.to_string()],
            )
            .unwrap();
        let before = saved.store.jobs().unwrap();
        assert!(
            saved
                .saved_series_frame_render(legacy, projected, &review)
                .is_err(),
            "{fault}"
        );
        assert_eq!(saved.store.jobs().unwrap(), before, "{fault}");
        assert!(!saved.store.root.join("sdk-worker").exists());
    }
}
