//! Synthetic durable schema refusals exercise the existing verifier end to end.
//! No real source, SDK process, model call or production database is used.
use super::*;
use crate::text_transport::{Backend, FrozenAttempt, Observation, ObservationKind};

const RETRY: &str = "sdkSchemaRejectionRetry";
const CONTINUE: &str = "sdkSchemaContinuation";

fn hash(value: &Value) -> String {
    crate::pipeline::digest(&value.to_string())
}
fn seal(mut value: Value) -> Value {
    value["sha256"] = json!(hash(&value));
    value
}
fn events(saved: &SavedTextExecutor, job: Option<&str>) -> Value {
    let c = saved.store.conn().unwrap();
    let mut q = c
        .prepare("SELECT id,job_id,type,data FROM events WHERE ?1 IS NULL OR job_id=?1 ORDER BY id")
        .unwrap();
    let rows = q
        .query_map([job], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .map(|row| {
            let (id, _job, kind, data) = row.unwrap();
            json!({"id":id,"type":kind,"data":serde_json::from_str::<Value>(&data).unwrap()})
        })
        .collect::<Vec<_>>();
    json!(rows)
}
fn parent_hash(saved: &SavedTextExecutor) -> String {
    hash(
        &saved
            .store
            .job(saved.workflow_id.as_deref().unwrap())
            .unwrap()["snapshot"],
    )
}
fn accept_raw(
    saved: &SavedTextExecutor,
    request: &TextRequest,
    snapshot: &Value,
    job: &str,
    raw: &Value,
) {
    let run = saved.workflow_id.as_deref().unwrap();
    let mut attempt = FrozenAttempt::prepare_sdk_strict_schema(
        job,
        run,
        &step_key(request),
        request.clone(),
        &snapshot["executorIdentity"],
    )
    .unwrap();
    let compiled = compile_sdk_attempt_input(&attempt).unwrap();
    let dir = saved.store.root.join("text-runs").join(job);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sdk-compiled.json"), json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)}).to_string()).unwrap();
    attempt.begin_dispatch(Backend::CodexSdk).unwrap();
    attempt.observe(Observation { attempt_id:job.into(), sequence:1, kind:ObservationKind::Completed, thread_id:Some("synthetic-frame-schema".into()), turn_id:None,
        raw_result:Some(json!({"rawText":raw.to_string(),"parsedJson":raw,"parseError":null,"domainAccepted":false})) }).unwrap();
    attempt.persist(&dir.join("sdk-attempt.json")).unwrap();
    std::fs::write(dir.join("result.json"), raw.to_string()).unwrap();
    saved
        .store
        .event(
            job,
            "text_received_raw",
            &json!({"requestHash":hash(snapshot),"value":raw}),
        )
        .unwrap();
    saved.store.update_job(job,"completed",&json!({"prepared":{"sdkRequestHash":hash(snapshot),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":run,
        "compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),"sdkRawTextSha256":hash(raw),"sdkDomainResult":raw,"sdkDomainResultSha256":hash(raw)},
        "result":raw,"error":null,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
    saved
        .store
        .checkpoint(
            run,
            &step_key(request),
            &request.stage,
            &hash(snapshot),
            "accepted",
            Some(job),
            Some(raw),
            None,
        )
        .unwrap();
}

/// Materialize the same durable proof shape as the independently tested schema
/// reservation. Only the real public read verifier is used by the selector.
fn refused_then_accepted(
    saved: &SavedTextExecutor,
    request: &TextRequest,
    raw: &Value,
) -> (String, String, Value) {
    let run = saved.workflow_id.as_deref().unwrap();
    let domain = saved.request_domain_snapshot(request).unwrap();
    let mut snapshot = domain.clone();
    snapshot["executorIdentity"] = saved.settings["textSdkIdentity"].clone();
    let origin = saved
        .store
        .add_job(
            "text",
            &request.stage,
            "codex_sdk",
            "synthetic-never-dispatch",
            &snapshot,
            "submission_unknown",
        )
        .unwrap();
    let prepared = FrozenAttempt::prepare_sdk_rules_first(
        &origin,
        run,
        &step_key(request),
        request.clone(),
        &snapshot["executorIdentity"],
    )
    .unwrap();
    let (attempt, rows) = crate::text_sdk::test_refusal_files(&saved.store.root, &prepared);
    let compiled = compile_sdk_attempt_input(&prepared).unwrap();
    saved.store.update_job(&origin,"submission_unknown",&json!({"prepared":{"sdkDispatchStarted":true,"sdkAttemptId":origin,"sdkOriginWorkflowId":run,"sdkRequestHash":hash(&snapshot),
        "compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema)}})).unwrap();
    for row in rows.as_array().unwrap() {
        saved
            .store
            .event(&origin, s(row, "type"), &row["data"])
            .unwrap();
    }
    let evidence = crate::text_sdk::schema_rejection_evidence(
        &saved.store.root,
        &attempt,
        &events(saved, Some(&origin)),
    )
    .unwrap();
    assert_eq!(evidence["retryableUniqueItems"], true);
    let proof = seal(
        json!({"version":"sdk-schema-refusal-successor/v1","workflowId":run,"nodeKey":step_key(request),"domainRequestSha256":hash(&domain),
        "originJobId":origin,"originSnapshotSha256":hash(&snapshot),"parentSnapshotSha256":parent_hash(saved),"originalSettingsSha256":hash(&saved.settings),
        "effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":{},"evidence":evidence,"attempt":1,"maximumAttempts":1}),
    );
    let mut successor = snapshot.clone();
    successor[RETRY] = proof.clone();
    // A proved refusal is terminal before its corrected successor becomes
    // active. Keep the real live-job uniqueness constraint in this fixture.
    saved.store.conn().unwrap().execute(
        "UPDATE jobs SET state='rejected',error='SDK_SCHEMA_REJECTED',receipt=?1 WHERE id=?2",
        params![json!({"contract":"desktop-runner/v1","validation":"rejected","transportValidation":"schema_rejected","evidenceSha256":hash(&evidence)}).to_string(),origin],
    ).unwrap();
    let next = saved
        .store
        .add_job(
            "text",
            &request.stage,
            "codex_sdk",
            "synthetic-never-dispatch",
            &successor,
            "queued",
        )
        .unwrap();
    // The refused original and all its sidecar bytes remain separate records.
    saved.store.conn().unwrap().execute("UPDATE jobs SET state='rejected',error='SDK_SCHEMA_REJECTED',receipt=?1 WHERE id=?2",params![
        json!({"contract":"desktop-runner/v1","validation":"rejected","transportValidation":"schema_rejected","evidenceSha256":hash(&evidence),"successorProofSha256":proof["sha256"],"successorJobId":next}).to_string(),origin]).unwrap();
    saved
        .store
        .event(
            &origin,
            "sdk_schema_rejection_successor",
            &json!({"proof":proof,"successorJobId":next}),
        )
        .unwrap();
    accept_raw(saved, request, &successor, &next, raw);
    assert_eq!(
        saved.sdk_schema_saved_snapshot(request, &domain).unwrap(),
        Some(successor.clone())
    );
    assert_eq!(
        saved
            .sdk_raw(&saved.sdk_job(&next).unwrap(), &successor)
            .unwrap(),
        *raw
    );
    (origin, next, successor)
}
fn continue_from(
    saved: &SavedTextExecutor,
    request: &TextRequest,
    raw: &Value,
    anchor_id: &str,
) -> (String, Value) {
    let anchor = saved.sdk_job(anchor_id).unwrap();
    let domain = saved.request_domain_snapshot(request).unwrap();
    let mut snapshot = domain.clone();
    snapshot["executorIdentity"] = anchor["snapshot"]["executorIdentity"].clone();
    snapshot[CONTINUE] = seal(
        json!({"version":"sdk-schema-refusal-continuation/v1","workflowId":saved.workflow_id,"nodeKey":step_key(request),
        "domainRequestSha256":hash(&domain),"originalSettingsSha256":hash(&saved.settings),"parentSnapshotSha256":parent_hash(saved),
        "effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":{},"anchor":{"jobId":anchor["id"],"snapshotSha256":hash(&anchor["snapshot"]),
        "preparedSha256":hash(&anchor["prepared"]),"receiptSha256":hash(&anchor["receipt"]),"resultSha256":hash(&anchor["result"]),"retryProofSha256":anchor["snapshot"][RETRY]["sha256"]}}),
    );
    let job = saved
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
    accept_raw(saved, request, &snapshot, &job, raw);
    assert_eq!(
        saved.sdk_schema_saved_snapshot(request, &domain).unwrap(),
        Some(snapshot.clone())
    );
    (job, snapshot)
}

struct Fixture {
    _temp: tempfile::TempDir,
    saved: SavedTextExecutor,
    legacy: TextRequest,
    projected: TextRequest,
    review: Value,
    origin: String,
    anchor: String,
    target: String,
}
async fn schema_fixture(wrapper: &str, reviewer: bool, use_new: bool) -> Fixture {
    let (mut input, legacy, projected, repair_raw, review, output) =
        crate::pipeline::series::series_frame_render_fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store
        .create("frame schema synthetic", "frame-schema-synthetic")
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let settings = &mut input["settings"];
    settings["textProvider"] = json!("codex_sdk");
    settings["textModel"] = json!(legacy.model);
    settings["reasoning"] = json!(legacy.reasoning);
    settings["textSdkIdentity"] = crate::text_sdk::test_refusal_identity(temp.path());
    let settings = settings.clone();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1","input":input}),
            "running",
        )
        .unwrap();
    let saved = SavedTextExecutor {
        store,
        settings,
        workflow_id: Some(run),
    };
    let request = if reviewer {
        serde_json::from_value(review["request"].clone()).unwrap()
    } else if use_new {
        projected.clone()
    } else {
        legacy.clone()
    };
    let raw = if reviewer {
        &review["raw"]
    } else {
        &repair_raw
    };
    let (origin, anchor, target) = if wrapper == RETRY {
        assert!(
            !reviewer,
            "the reviewer schema has no uniqueItems; use a genuine author refusal anchor instead"
        );
        let (origin, target, _) = refused_then_accepted(&saved, &request, raw);
        (origin, target.clone(), target)
    } else {
        let role = output["seriesDeliveries"][0]["adoptionReceipt"]["roleReceipts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["role"] == "author")
            .unwrap();
        let author: TextRequest = serde_json::from_value(role["request"].clone()).unwrap();
        let (origin, anchor, _) = refused_then_accepted(&saved, &author, &role["raw"]);
        let (target, _) = continue_from(&saved, &request, raw, &anchor);
        (origin, anchor, target)
    };
    Fixture {
        _temp: temp,
        saved,
        legacy,
        projected,
        review,
        origin,
        anchor,
        target,
    }
}
fn durable(saved: &SavedTextExecutor) -> Value {
    let c = saved.store.conn().unwrap();
    let mut q = c.prepare("SELECT workflow_id,node_key,request_hash,state,text_job_id,result,error FROM workflow_steps ORDER BY workflow_id,node_key").unwrap();
    let steps = q
        .query_map([], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?
            ]))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect::<Vec<_>>();
    json!({"jobs":saved.store.jobs().unwrap(),"steps":steps,"events":events(saved,None)})
}

#[tokio::test]
async fn series_frame_render_schema_continuation_review_reopens_without_rebuying_review() {
    let f = schema_fixture(CONTINUE, true, false).await;
    let before = durable(&f.saved);
    let original_file = f
        .saved
        .store
        .root
        .join("sdk-worker")
        .join(&f.origin)
        .join("receipt.json");
    let original_bytes = std::fs::read(&original_file).unwrap();
    let reopened = SavedTextExecutor {
        store: Store::open(f.saved.store.root.clone()).unwrap(),
        settings: f.saved.settings.clone(),
        workflow_id: f.saved.workflow_id.clone(),
    };
    for _ in 0..2 {
        exact(
            &reopened
                .saved_series_frame_render(f.legacy.clone(), f.projected.clone(), &f.review)
                .unwrap(),
            &f.projected,
        );
    }
    assert_eq!(durable(&f.saved), before);
    assert_eq!(std::fs::read(original_file).unwrap(), original_bytes);
    assert!(!f.saved.store.root.join("sdk-worker/worker.lock").exists());
}

#[tokio::test]
async fn series_frame_render_schema_wrapped_existing_repairs_keep_frozen_requests_in_all_states() {
    for wrapper in [RETRY, CONTINUE] {
        for use_new in [false, true] {
            for state in [
                "queued",
                "running",
                "submission_unknown",
                "received",
                "rejected",
                "completed",
            ] {
                let f = schema_fixture(wrapper, false, use_new).await;
                let c = f.saved.store.conn().unwrap();
                c.execute(
                    "UPDATE jobs SET state=?1 WHERE id=?2",
                    params![state, f.target],
                )
                .unwrap();
                c.execute(
                    "UPDATE workflow_steps SET state=?1 WHERE text_job_id=?2",
                    params![
                        if state == "completed" {
                            "accepted"
                        } else {
                            state
                        },
                        f.target
                    ],
                )
                .unwrap();
                let before = durable(&f.saved);
                let selected = f
                    .saved
                    .saved_series_frame_render(f.legacy.clone(), f.projected.clone(), &f.review)
                    .unwrap();
                exact(&selected, if use_new { &f.projected } else { &f.legacy });
                assert_eq!(durable(&f.saved), before, "{wrapper}/{state}/{use_new}");
                assert!(!f.saved.store.root.join("sdk-worker/worker.lock").exists());
            }
        }
    }
}

#[tokio::test]
async fn series_frame_render_schema_proof_drift_and_unknown_review_are_held_without_writes() {
    for fault in [
        "null-proof",
        "bad-seal",
        "dual-proof",
        "source",
        "checkpoint",
        "orphan",
        "raw-event",
        "origin-file",
        "origin-stop",
        "anchor-unknown",
        "review-unknown",
    ] {
        let f = schema_fixture(CONTINUE, true, false).await;
        let c = f.saved.store.conn().unwrap();
        match fault {
            "null-proof" | "bad-seal" | "dual-proof" | "source" => {
                let mut snapshot = f.saved.sdk_job(&f.target).unwrap()["snapshot"].clone();
                match fault {
                    "null-proof" => snapshot[CONTINUE] = Value::Null,
                    "bad-seal" => snapshot[CONTINUE]["sha256"] = json!("0".repeat(64)),
                    "dual-proof" => snapshot[RETRY] = Value::Null,
                    "source" => snapshot["input"]["sourceSha256"] = json!("changed-source"),
                    _ => unreachable!(),
                }
                c.execute(
                    "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                    params![snapshot.to_string(), f.target],
                )
                .unwrap();
                c.execute(
                    "UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",
                    params![hash(&snapshot), f.target],
                )
                .unwrap();
            }
            "checkpoint" => {
                c.execute(
                    "UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",
                    [&f.target],
                )
                .unwrap();
            }
            "orphan" => {
                c.execute(
                    "DELETE FROM workflow_steps WHERE text_job_id=?1",
                    [&f.target],
                )
                .unwrap();
            }
            "raw-event" => {
                c.execute(
                    "UPDATE events SET data='{}' WHERE job_id=?1 AND type='text_received_raw'",
                    [&f.target],
                )
                .unwrap();
            }
            "origin-file" => std::fs::write(
                f.saved
                    .store
                    .root
                    .join("sdk-worker")
                    .join(&f.origin)
                    .join("receipt.json"),
                "{}",
            )
            .unwrap(),
            "origin-stop" => {
                c.execute(
                    "UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_process_tree'",
                    [&f.origin],
                )
                .unwrap();
            }
            "anchor-unknown" => {
                c.execute(
                    "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                    [&f.anchor],
                )
                .unwrap();
            }
            "review-unknown" => {
                c.execute(
                    "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                    [&f.target],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = durable(&f.saved);
        assert!(
            f.saved
                .saved_series_frame_render(f.legacy, f.projected, &f.review)
                .is_err(),
            "{fault}"
        );
        assert_eq!(durable(&f.saved), before, "{fault}");
        assert!(!f.saved.store.root.join("sdk-worker/worker.lock").exists());
    }
}

#[tokio::test]
async fn series_frame_render_schema_retry_proof_drift_cannot_reselect_or_recreate_old_node() {
    for fault in [
        "null-proof",
        "origin",
        "budget",
        "origin-file",
        "checkpoint",
        "orphan",
    ] {
        let f = schema_fixture(RETRY, false, true).await;
        let c = f.saved.store.conn().unwrap();
        match fault {
            "null-proof" | "origin" | "budget" => {
                let mut snapshot = f.saved.sdk_job(&f.target).unwrap()["snapshot"].clone();
                if fault == "null-proof" {
                    snapshot[RETRY] = Value::Null;
                } else {
                    let mut proof = snapshot[RETRY].clone();
                    proof.as_object_mut().unwrap().remove("sha256");
                    if fault == "origin" {
                        proof["originJobId"] = json!("different-origin");
                    } else {
                        proof["attempt"] = json!(2);
                    }
                    snapshot[RETRY] = seal(proof);
                }
                c.execute(
                    "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                    params![snapshot.to_string(), f.target],
                )
                .unwrap();
                c.execute(
                    "UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",
                    params![hash(&snapshot), f.target],
                )
                .unwrap();
            }
            "origin-file" => std::fs::write(
                f.saved
                    .store
                    .root
                    .join("sdk-worker")
                    .join(&f.origin)
                    .join("receipt.json"),
                "{}",
            )
            .unwrap(),
            "checkpoint" => {
                c.execute(
                    "UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",
                    [&f.target],
                )
                .unwrap();
            }
            "orphan" => {
                c.execute(
                    "DELETE FROM workflow_steps WHERE text_job_id=?1",
                    [&f.target],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = durable(&f.saved);
        assert!(
            f.saved
                .saved_series_frame_render(f.legacy, f.projected, &f.review)
                .is_err(),
            "{fault}"
        );
        assert_eq!(durable(&f.saved), before, "{fault}");
        assert!(!f.saved.store.root.join("sdk-worker/worker.lock").exists());
    }
}
