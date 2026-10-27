use super::*;

fn fixture() -> (tempfile::TempDir, Store, String, Value, Value) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().to_owned()).unwrap();
    store
        .create("source analysis synthetic", "fixture")
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let parsed =
        pipeline::prepare_source("第1集\n地点：工坊\n△林川开门。\n林川：我回来了。\n").unwrap();
    let source = store.save_source(json!(parsed)).unwrap();
    let snapshot = json!({"runnerVersion":RUNNER,"stage":"source.analyze","sourceVersion":source["version"],"input":{"source":source,"selection":{}},"continueFoundation":true,"rules":pipeline::source_analysis::rule_snapshot()});
    let run = store
        .add_job(
            "workflow",
            "source.analyze",
            "local",
            "local",
            &snapshot,
            "running",
        )
        .unwrap();
    let rows: Vec<_> = parsed.rows.iter().filter(|r| r.kind == "body").collect();
    let decision:pipeline::BoundaryDecision=serde_json::from_value(json!({"start":rows.first().unwrap().range.start,"end":rows.last().unwrap().range.end,"kind":"body","episodeNumber":1,"sceneTitle":"工坊","reason":"合成测试原文地点字段"})).unwrap();
    let next = pipeline::apply_boundary_decisions(&parsed, &[decision.clone()]).unwrap();
    let output = json!({"schemaVersion":"source-structure-analysis/v1","source":next,"decisions":[decision],"unresolved":[],"usedModel":true,"status":"completed"});
    store
        .checkpoint(
            &run,
            "sourceStructure",
            "sourceStructure",
            "synthetic",
            "accepted",
            None,
            Some(&json!({"groups":[]})),
            None,
        )
        .unwrap();
    (dir, store, run, snapshot, output)
}

fn successor(parent: &str, result: &Value) -> Value {
    let source = &result["source"];
    let ids: Vec<_> = source["episodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    let input = json!({"source":source,"selection":{"episodeIds":ids},"assets":[],"units":[],"foundation":null});
    json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run","sourceAnalysisContinuation":{"version":"source-analysis-continuation/v1","workflowId":parent},"input":input,"guard":crate::workflow::guard_from_input(&input)})
}

#[test]
fn source_analysis_publication_is_atomic_and_idempotent() {
    let (_dir, store, run, snapshot, output) = fixture();
    let frozen = store.job(&run).unwrap()["snapshot"].clone();
    let result = store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap();
    assert_eq!(result["source"]["version"], 2);
    assert_eq!(
        result["source"]["text"],
        snapshot["input"]["source"]["text"]
    );
    assert_eq!(
        result["source"]["sha256"],
        snapshot["input"]["source"]["sha256"]
    );
    assert_eq!(store.source().unwrap(), result["source"]);
    assert_eq!(store.job(&run).unwrap()["status"], "completed");
    assert_eq!(store.job(&run).unwrap()["snapshot"], frozen);
    assert_eq!(
        store
            .publish_source_analysis(&run, &snapshot, &output)
            .unwrap(),
        result
    );
    assert_eq!(store.source().unwrap()["version"], 2);
}

#[test]
fn source_analysis_stale_source_does_not_publish_or_complete() {
    let (_dir, store, run, snapshot, output) = fixture();
    store
        .save_source(json!(pipeline::prepare_source(
            "第1集\n场景：门外\n林川：另一份原文。\n"
        )
        .unwrap()))
        .unwrap();
    let current = store.source().unwrap();
    assert!(store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap_err()
        .contains("SOURCE_VERSION_CHANGED"));
    assert_eq!(store.source().unwrap(), current);
    assert_eq!(store.job(&run).unwrap()["status"], "running");
}

#[test]
fn source_analysis_unknown_preserves_original_node_and_blocks_publication() {
    let (_dir, store, run, snapshot, output) = fixture();
    let child = store
        .add_job(
            "text",
            "sourceStructure",
            "codex_sdk",
            "fixture",
            &json!({"frozen":"original"}),
            "submission_unknown",
        )
        .unwrap();
    store
        .checkpoint(
            &run,
            "sourceStructure",
            "sourceStructure",
            "synthetic",
            "running",
            Some(&child),
            None,
            Some("original unknown"),
        )
        .unwrap();
    let before = store.job(&child).unwrap();
    assert!(store.source_analysis_unknown(&run).unwrap());
    assert!(store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap_err()
        .contains("UNKNOWN_HELD"));
    assert_eq!(store.job(&child).unwrap(), before);
    assert_eq!(store.source().unwrap()["version"], 1);
}

#[test]
fn source_analysis_requires_accepted_model_receipt_and_rejects_rewritten_source() {
    let (_dir, store, run, snapshot, mut output) = fixture();
    store
        .checkpoint(
            &run,
            "sourceStructure",
            "sourceStructure",
            "synthetic",
            "running",
            None,
            None,
            None,
        )
        .unwrap();
    assert!(store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap_err()
        .contains("ACCEPTED_RECEIPT_MISSING"));
    output["source"]["text"] = json!("擅自重写");
    assert!(store
        .publish_source_analysis(&run, &snapshot, &output)
        .is_err());
    assert_eq!(store.source().unwrap()["version"], 1);
}

#[test]
fn source_analysis_returned_receipt_survives_reopen_without_new_request() {
    let (dir, store, run, snapshot, output) = fixture();
    drop(store);
    let reopened = Store::open(dir.path().to_owned()).unwrap();
    let result = reopened
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap();
    drop(reopened);
    let reopened = Store::open(dir.path().to_owned()).unwrap();
    assert_eq!(
        reopened
            .publish_source_analysis(&run, &snapshot, &output)
            .unwrap(),
        result
    );
    assert_eq!(reopened.jobs().unwrap().len(), 1);
}

#[test]
fn source_analysis_successor_reservation_is_single_and_frozen() {
    let (_dir, store, run, snapshot, output) = fixture();
    let result = store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap();
    let child = successor(&run, &result);
    let id = store
        .reserve_foundation_after_source_analysis(&run, &child)
        .unwrap();
    assert_eq!(
        store
            .reserve_foundation_after_source_analysis(&run, &child)
            .unwrap(),
        id
    );
    assert_eq!(
        store.job(&run).unwrap()["result"]["continuation"]["workflowId"],
        id
    );
    assert_eq!(store.job(&id).unwrap()["status"], "queued");
    let mut changed = child.clone();
    changed["changedExecution"] = json!(true);
    assert!(store
        .reserve_foundation_after_source_analysis(&run, &changed)
        .unwrap_err()
        .contains("SUCCESSOR_ALREADY_FROZEN"));
    assert_eq!(store.jobs().unwrap().len(), 2);
}

#[test]
fn source_analysis_existing_successor_keeps_historical_snapshot() {
    let (_dir, store, run, snapshot, output) = fixture();
    let result = store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap();
    let mut child = successor(&run, &result);
    child
        .as_object_mut()
        .unwrap()
        .remove("sourceAnalysisContinuation");
    let id = store
        .add_job(
            "workflow",
            "foundation.run",
            "local",
            "local",
            &child,
            "interrupted",
        )
        .unwrap();
    store
        .link_existing_source_analysis_successor(&run, &id)
        .unwrap();
    store
        .link_existing_source_analysis_successor(&run, &id)
        .unwrap();
    assert_eq!(store.job(&id).unwrap()["snapshot"], child);
    assert_eq!(
        store.job(&run).unwrap()["result"]["continuation"]["existing"],
        true
    );
    let mut other = child.clone();
    other["input"]["selection"]["episodeIds"] = json!(["different"]);
    let other = store
        .add_job(
            "workflow",
            "foundation.run",
            "local",
            "local",
            &other,
            "interrupted",
        )
        .unwrap();
    assert!(store
        .link_existing_source_analysis_successor(&run, &other)
        .is_err());
}

#[test]
fn source_analysis_completion_can_later_continue_without_changing_frozen_request() {
    let (_dir, store, run, mut snapshot, output) = fixture();
    snapshot["continueFoundation"] = json!(false);
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
            rusqlite::params![snapshot.to_string(), run],
        )
        .unwrap();
    store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap();
    store.request_source_analysis_continuation(&run).unwrap();
    assert_eq!(store.job(&run).unwrap()["snapshot"], snapshot);
    assert_eq!(
        store.job(&run).unwrap()["result"]["continuation"]["status"],
        "pending"
    );
    assert_eq!(store.source().unwrap()["version"], 2);
}

#[test]
fn source_analysis_publication_stales_only_unsubmitted_tasks() {
    let (_dir, store, run, snapshot, output) = fixture();
    let pending = store
        .add_job(
            "image",
            "pending-asset",
            "fixture",
            "fixture",
            &json!({}),
            "queued",
        )
        .unwrap();
    let unknown = store
        .add_job(
            "image",
            "unknown-asset",
            "fixture",
            "fixture",
            &json!({}),
            "submission_unknown",
        )
        .unwrap();
    let before = store.job(&unknown).unwrap();
    store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap();
    assert_eq!(store.job(&pending).unwrap()["status"], "stale");
    assert_eq!(store.job(&unknown).unwrap(), before);
}

#[test]
fn source_analysis_selection_rejects_missing_duplicate_and_empty_scope() {
    let (_dir, _store, _run, snapshot, _output) = fixture();
    let source = &snapshot["input"]["source"];
    assert!(validate_selection(source, &json!({})).is_ok());
    for ids in [
        json!([]),
        json!(["missing"]),
        json!([source["episodes"][0]["id"], source["episodes"][0]["id"]]),
    ] {
        assert!(validate_selection(source, &json!({"episodeIds":ids})).is_err());
    }
}

#[test]
fn source_analysis_summary_projects_published_source_without_script_body() {
    let (_dir, store, run, snapshot, output) = fixture();
    let result = store
        .publish_source_analysis(&run, &snapshot, &output)
        .unwrap();
    let summary = store
        .job_summaries()
        .unwrap()
        .into_iter()
        .find(|job| job["id"] == run)
        .unwrap();
    assert_eq!(
        summary["result"]["source"]["version"],
        result["source"]["version"]
    );
    assert_eq!(
        summary["result"]["source"]["sha256"],
        result["source"]["sha256"]
    );
    assert_eq!(summary["result"]["continuation"]["status"], "pending");
    assert!(summary["result"]["source"].get("text").is_none());
}

#[test]
fn source_analysis_local_reparse_completes_with_unavailable_model_channel() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("project")).unwrap();
    store.create("local only synthetic", "fixture").unwrap();
    store
        .save_source(json!(pipeline::prepare_source(
            "第1集\n场景：门口\n林川：回来了。\n"
        )
        .unwrap()))
        .unwrap();
    let engine = Engine::with_channels(
        dir.path().join("home"),
        Arc::new(crate::execution_policy::OfflineChannels),
    )
    .unwrap();
    engine
        .spawn_source_analysis(store.clone(), &json!({"continueFoundation":false}))
        .unwrap();
    let started = std::time::Instant::now();
    loop {
        let jobs = store.jobs().unwrap();
        if jobs
            .iter()
            .any(|job| job["entityId"] == "source.analyze" && job["status"] == "completed")
        {
            assert_eq!(jobs.len(), 1, "no text/model task was submitted");
            assert_eq!(jobs[0]["result"]["usedModel"], false);
            break;
        }
        assert!(
            // This checks local completion and zero model calls, not a timing
            // SLA; parallel native build/tests can temporarily saturate CPUs.
            started.elapsed() < std::time::Duration::from_secs(30),
            "local task failed or stalled: {jobs:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Wait until the worker has released its lease before the temporary store
    // disappears; this test deliberately never starts a foundation successor.
    let started = std::time::Instant::now();
    loop {
        if let Ok(lease) = engine.acquire_pipeline(&store) {
            drop(lease);
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
