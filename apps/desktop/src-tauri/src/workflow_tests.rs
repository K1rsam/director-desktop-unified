use crate::{
    pipeline,
    store::{s, Store},
    workflow::guard_from_input,
};
use serde_json::{json, Value};

fn project() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().to_owned()).unwrap();
    store.create("制作流程验收", "test").unwrap();
    let source = pipeline::prepare_source(
        "第1集\n场景：门口\n小王：别动。\n第2集\n场景：门口\n小王：我知道。\n",
    )
    .unwrap();
    store
        .save_source(serde_json::to_value(source).unwrap())
        .unwrap();
    (dir, store)
}
fn input(store: &Store) -> Value {
    json!({"source":store.source().unwrap(),"foundation":store.foundation().unwrap(),"assets":store.values("assets").unwrap(),"existingUnits":store.values("units").unwrap(),"selection":{"episodeIds":store.source().unwrap()["episodes"].as_array().unwrap().iter().map(|e|e["id"].clone()).collect::<Vec<_>>()},"capabilities":{"version":"test"}})
}

#[test]
fn audit_only_foundation_update_keeps_frozen_storyboard_inputs_and_all_other_guards() {
    let (_dir, store) = project();
    let source = store.source().unwrap();
    let first = input(&store);
    let initial = guard_from_input(&first);
    store.publish_foundation(&initial,json!({"sourceSha256":source["sha256"],"productionDirection":"既定制作方向","facts":[],"episodes":[],"visualFactBatches":[]}),vec![],None).unwrap();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":frozen,"guard":guard});
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &snapshot,
            "failed",
        )
        .unwrap();
    let old = store.job(&run).unwrap()["snapshot"].clone();
    store.publish_asset_design(1,frozen["foundation"]["version"].as_i64().unwrap(),&json!({"assets":[],"facts":[],"visualFactBatches":[{"sourceSha256":source["sha256"],"status":"accepted","inputFingerprint":"audit-only-receipt"}]})).unwrap();
    store
        .refresh_storyboard_audit_guard(&run, &snapshot)
        .unwrap();
    let saved = store.job(&run).unwrap();
    assert_eq!(saved["snapshot"], old);
    let fresh = &saved["prepared"]["publicationGuard"];
    assert_eq!(
        fresh["foundationVersion"],
        store.foundation().unwrap()["version"]
    );
    let mut connection = store.conn().unwrap();
    let tx = connection.transaction().unwrap();
    crate::workflow::verify_guard(&tx, fresh).unwrap();
    tx.rollback().unwrap();
    let evidence=store.conn().unwrap().query_row("SELECT count(*) FROM events WHERE job_id=?1 AND type='foundation_audit_guard_refreshed'",[&run],|row|row.get::<_,i64>(0)).unwrap();
    store
        .refresh_storyboard_audit_guard(&run, &snapshot)
        .unwrap();
    assert_eq!(store.conn().unwrap().query_row("SELECT count(*) FROM events WHERE job_id=?1 AND type='foundation_audit_guard_refreshed'",[&run],|row|row.get::<_,i64>(0)).unwrap(),evidence);
    assert_eq!(
        store.jobs().unwrap().len(),
        1,
        "no new model or generation task"
    );
}

#[test]
fn creative_source_and_catalogue_changes_cannot_use_audit_only_guard_refresh() {
    for mutation in ["direction", "facts", "source", "asset"] {
        let (_dir, store) = project();
        let source = store.source().unwrap();
        let initial = guard_from_input(&input(&store));
        store.publish_foundation(&initial,json!({"sourceSha256":source["sha256"],"productionDirection":"既定制作方向","facts":[],"episodes":[],"visualFactBatches":[]}),vec![],None).unwrap();
        let frozen = input(&store);
        let guard = guard_from_input(&frozen);
        let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":frozen,"guard":guard});
        let run = store
            .add_job(
                "workflow",
                "storyboard.run",
                "local",
                "local",
                &snapshot,
                "failed",
            )
            .unwrap();
        store.publish_asset_design(1,frozen["foundation"]["version"].as_i64().unwrap(),&json!({"assets":[],"facts":[],"visualFactBatches":[{"sourceSha256":source["sha256"],"status":"accepted","inputFingerprint":"audit-only-receipt"}]})).unwrap();
        match mutation {
            "direction" => store
                .save_production_direction("新的制作方向", 1, None)
                .unwrap(),
            "source" => {
                store
                    .save_source(
                        serde_json::to_value(
                            pipeline::prepare_source("第1集\n场景：门口\n甲：新原文。\n").unwrap(),
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            "asset" => {
                store.save_asset(json!({"id":"new-asset","variantId":"base","name":"新资产","kind":"character","sourceRefs":[]})).unwrap();
            }
            "facts" => {
                let mut basis = store.foundation().unwrap();
                basis["facts"] = json!([{"id":"new-fact","statement":"新事实"}]);
                let g = guard_from_input(&input(&store));
                store.publish_foundation(&g, basis, vec![], None).unwrap();
            }
            _ => unreachable!(),
        }
        let before = store.job(&run).unwrap();
        let _ = store.refresh_storyboard_audit_guard(&run, &snapshot);
        let after = store.job(&run).unwrap();
        assert_eq!(after["snapshot"], before["snapshot"]);
        assert_eq!(after["prepared"], before["prepared"], "{mutation}");
        assert_eq!(store.conn().unwrap().query_row("SELECT count(*) FROM events WHERE job_id=?1 AND type='foundation_audit_guard_refreshed'",[&run],|row|row.get::<_,i64>(0)).unwrap(),0);
    }
}

fn complete_episode(store: &Store, episode: usize) -> Value {
    let source = store.source().unwrap();
    let eid = &source["episodes"][episode]["id"];
    let units:Vec<_>=source["rows"].as_array().unwrap().iter().filter(|r|r["kind"]=="body"&&r["episodeId"]==*eid).enumerate().map(|(index,row)|{
        let quote=&row["dialogue"];
        let parts:Vec<_>=quote["spokenSegments"].as_array().unwrap().iter().enumerate().map(|(i,segment)|{
            let full:Vec<_>=source["text"].as_str().unwrap().encode_utf16().collect();
            let text=String::from_utf16(&full[segment["range"]["start"].as_u64().unwrap() as usize..segment["range"]["end"].as_u64().unwrap() as usize]).unwrap();
            json!({"lineId":quote["id"],"quoteStart":segment["range"]["start"],"quoteEnd":segment["range"]["end"],"text":text,"speaker":quote["speaker"],"channel":quote["channel"],"presentation":"画内","delivery":"自然说出","voiceStart":i as f64*0.5,"voiceEnd":i as f64*0.5+0.4})
        }).collect();
        json!({"id":format!("ready-{episode}-{index}"),"episodeId":eid,"sceneId":row["sceneId"],"title":"完整单集","unitDirection":"源中动作与回应独立完整。","scene":"门口","participants":["小王"],"props":[],"lighting":"侧光","initialState":"小王站在门口。","exitState":"小王保持原站位。","sourceRefs":[row["id"]],"assetRefs":[],"duration":4,"prompt":"源原话与六栏镜表完整独立","shots":[{"id":format!("shot-{episode}-{index}"),"duration":4,"camera":"近景读回应","picture":"小王完成原动作并说出原话。","lighting":"侧光","sound":"环境与原声","transition":"自然切出","sourceRefs":[row["id"]],"contextRefs":[],"dialogueParts":parts}]})
    }).collect();
    json!({"schemaVersion":"complete-episode-delivery/v1","status":"running","scope":"episodes","sourceSha256":source["sha256"],"episodeId":eid,"updatedEpisodeIds":[eid],"replacedUnitIds":[],"units":units,"plans":[{"episodeId":eid,"plan":{"events":[]}}],"fantasyRealization":[],"revisionNotes":[],"unresolved":[],"blocked":[],"finishProvenance":[{"version":"accepted-finish-domain/v1","executionBatchId":"offline-finish","canonicalDomainRequestSha256":pipeline::digest("accepted-offline-domain"),"draftSha256":pipeline::digest("accepted-offline-draft")}],"ruleSnapshots":[]})
}

#[test]
fn per_episode_publication_is_visible_running_atomic_and_idempotent_after_reopen() {
    let (dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let old_second = store.save_unit(unit(&store, "old-second", 1)).unwrap();
    let mut frozen = input(&store);
    frozen["capabilities"] = json!({"minDuration":4,"maxDuration":30,"imageLimit":10});
    let guard = guard_from_input(&frozen);
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1","guard":guard,"input":frozen}),
            "running",
        )
        .unwrap();
    let snapshot = store.job(&run).unwrap()["snapshot"].clone();
    let first = complete_episode(&store, 0);
    store
        .publish_storyboard_episode(&guard, &first, &frozen, &run)
        .unwrap();
    let state = store.job(&run).unwrap();
    assert_eq!(state["status"], "running");
    assert_eq!(state["result"]["units"], first["units"]);
    assert_eq!(
        state["snapshot"], snapshot,
        "source/input/old guard stay frozen"
    );
    assert_eq!(store.get("units", "old-second").unwrap(), old_second);
    assert_eq!(store.get("units", "ready-0-0").unwrap()["version"], 1);
    let next = state["prepared"]["publicationGuard"].clone();
    let reopened = Store::open(dir.path().to_owned()).unwrap();
    reopened
        .publish_storyboard_episode(&next, &first, &frozen, &run)
        .unwrap();
    let events: i64 = reopened
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE job_id=?1 AND type='episode_published'",
            [&run],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
    assert_eq!(
        reopened.unit_versions("ready-0-0").unwrap()["versions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let second = complete_episode(&reopened, 1);
    let mut publication_input = frozen.clone();
    publication_input["previousBoundary"] =
        first["units"].as_array().unwrap().last().unwrap().clone();
    reopened
        .publish_storyboard_episode(&next, &second, &publication_input, &run)
        .unwrap();
    assert_eq!(reopened.job(&run).unwrap()["status"], "running");
    assert_eq!(
        reopened.job(&run).unwrap()["result"]["units"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let dependency: String = reopened
        .conn()
        .unwrap()
        .query_row(
            "SELECT dependency_id FROM unit_dependencies WHERE unit_id='ready-1-0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dependency, "ready-0-0");
    reopened
        .update_job(&run, "failed", &json!({"error":"third episode failed"}))
        .unwrap();
    assert_eq!(
        reopened.job(&run).unwrap()["result"]["units"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "later failure does not erase completed episodes"
    );
    assert_eq!(
        reopened.get("units", "ready-0-0").unwrap()["retired"],
        false
    );
    let final_guard = reopened.job(&run).unwrap()["prepared"]["publicationGuard"].clone();
    let final_output = reopened.job(&run).unwrap()["result"].clone();
    reopened
        .publish_storyboard(&final_guard, &final_output, &frozen, Some(&run))
        .unwrap();
    assert_eq!(reopened.get("units", "ready-0-0").unwrap()["version"], 1);
}

#[test]
fn incomplete_or_changed_source_cannot_publish_an_episode_or_advance_its_guard() {
    let (_dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let mut frozen = input(&store);
    frozen["capabilities"] = json!({"minDuration":4,"maxDuration":30,"imageLimit":10});
    let guard = guard_from_input(&frozen);
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"guard":guard,"input":frozen}),
            "running",
        )
        .unwrap();
    let before = store.job(&run).unwrap();
    let good = complete_episode(&store, 0);
    for kind in ["quote", "coverage", "provenance", "source"] {
        let mut bad = good.clone();
        match kind {
            "quote" => {
                bad["units"][0]["shots"][0]["dialogueParts"][0]["text"] = json!("丢失原词。")
            }
            "coverage" => bad["units"][0]["sourceRefs"] = json!([]),
            "provenance" => bad["finishProvenance"] = json!([]),
            _ => bad["sourceSha256"] = json!("wrong"),
        }
        assert!(store
            .publish_storyboard_episode(&guard, &bad, &frozen, &run)
            .is_err());
        assert_eq!(store.job(&run).unwrap(), before);
        assert!(store.values("units").unwrap().is_empty());
    }
    store
        .publish_storyboard_episode(&guard, &good, &frozen, &run)
        .unwrap();
    let next = store.job(&run).unwrap()["prepared"]["publicationGuard"].clone();
    let mut edited = store.get("units", "ready-0-0").unwrap();
    edited["prompt"] = json!("user edited current accepted unit");
    store.save_unit(edited).unwrap();
    assert!(
        store
            .publish_storyboard_episode(&next, &good, &frozen, &run)
            .is_err(),
        "own publication cannot hide subsequent user edits"
    );
}

#[test]
fn workflow_progress_counts_a_migrated_cached_request_once_without_removing_history() {
    let (_dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1"}),
            "running",
        )
        .unwrap();
    for (key, hash, state) in [
        ("legacy-create", "same", "rejected"),
        ("create:batch-1", "same", "accepted"),
        ("create:batch-2", "different", "accepted"),
    ] {
        store
            .checkpoint(
                &run,
                key,
                "storyboardCreate",
                hash,
                state,
                None,
                Some(&json!({})),
                None,
            )
            .unwrap();
    }
    // Equal timestamps still have a deterministic rowid tie-breaker.
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE workflow_steps SET updated_at='2026-01-01T00:00:00Z' WHERE workflow_id=?1",
            [&run],
        )
        .unwrap();
    let progress = store.workflow_progress().unwrap();
    assert_eq!(progress[0]["completedSteps"], 2);
    assert_eq!(progress[0]["totalSteps"], 2);
    assert_eq!(progress[0]["nodes"][0]["id"], "create:batch-1");
    assert_eq!(progress[0]["nodes"][1]["id"], "create:batch-2");
    let retained: i64 = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM workflow_steps WHERE workflow_id=?1",
            [&run],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 3);
}

#[test]
fn workflow_progress_stage2_tracks_frozen_targets_and_accepted_children_without_publishing() {
    let (_dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let assets = json!([
        {"id":"base","variantId":"base","prompt":"private old text"},
        {"id":"suit","variantId":"suit","sourceAssetId":"base"},
        {"id":"injury","variantId":"injury","sourceAssetId":"suit"},
        {"id":"cover","variantId":"cover","sourceAssetId":"injury","preparation":{"mode":"INLINE"}},
        {"id":"unused","variantId":"other"}
    ]);
    let context = json!({"assets":assets,"selection":{"assetIds":["cover","base"]}});
    let run = store.add_job("workflow","images.design:","local","local",&json!({"runnerVersion":"stage2-runner/v1","stage":"images.design","stage2Context":context}),"running").unwrap();
    let child = store.add_job("text","assetDesign","cli","test",&json!({"input":{"asset":{"id":"base","prompt":"private child"}}}),"completed").unwrap();
    store.checkpoint(&run,"assetDesign:base","assetDesign","old-hash","rejected",Some(&child),None,Some("old rejection")).unwrap();
    store.checkpoint(&run,"assetDesign:base:repair","assetDesign","new-hash","accepted",Some(&child),Some(&json!({"prompt":"private result"})),None).unwrap();
    let before = store.job(&run).unwrap();
    let p = store.workflow_progress().unwrap();
    assert_eq!(p[0]["workflowId"],run);
    assert_eq!(p[0]["stage"],"images.design");
    assert_eq!(p[0]["assetDesignPlan"],json!({"designAssetIds":["base","suit","injury"],"inlineAssetIds":["cover"]}));
    assert_eq!(p[0]["nodes"][1]["input"]["assetId"],"base");
    assert_eq!(p[0]["completedSteps"],1);
    assert!(!p[0].to_string().contains("private"));
    assert!(store.values("assets").unwrap().is_empty());
    assert_eq!(store.job(&run).unwrap(),before);
    let reopened = Store::open(store.root.clone()).unwrap();
    assert_eq!(reopened.workflow_progress().unwrap(),p);
}

#[test]
fn workflow_progress_stage2_has_no_speculative_plan_for_auto_production_or_invalid_dependencies() {
    let (_dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    for (stage, context) in [
        ("images.produce",json!({"assets":[{"id":"a","variantId":"a"}],"selection":{"assetIds":["a"]}})),
        ("images.design",json!({"assets":[{"id":"a","variantId":"a","sourceAssetId":"missing"}],"selection":{"assetIds":["a"]}})),
        ("images.design",json!({"assets":[{"id":"a","variantId":"a","sourceAssetId":"a"}],"selection":{"assetIds":["a"]}})),
        ("images.design",json!({"automaticDesign":true,"assets":[{"id":"a","variantId":"a"}],"selection":{"assetIds":["a"]}})),
        ("images.design",json!({"assets":[{"id":"base","variantId":"base","prompt":"cached design"},{"id":"child","variantId":"child","sourceAssetId":"base"}],"selection":{"assetIds":["child"]}})),
    ] {
        let run=store.add_job("workflow",stage,"local","local",&json!({"runnerVersion":"stage2-runner/v1","stage":stage,"stage2Context":context}),"running").unwrap();
        let p=store.workflow_progress().unwrap();
        assert!(p.iter().find(|p|p["workflowId"]==run).unwrap()["assetDesignPlan"].is_null());
    }
}

#[test]
fn workflow_progress_reads_only_attached_active_cli_safe_heartbeat_fields() {
    let (_dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1"}),
            "running",
        )
        .unwrap();
    let text = store
        .add_job(
            "text",
            "step",
            "codex",
            "local",
            &json!({"input":{"episodeId":"ep1"}}),
            "submitting",
        )
        .unwrap();
    let other = store
        .add_job("text", "other", "codex", "local", &json!({}), "submitting")
        .unwrap();
    store
        .checkpoint(
            &run,
            "plan:ep1",
            "storyboardPlan",
            "hash",
            "running",
            Some(&text),
            None,
            None,
        )
        .unwrap();
    store.event(&text,"codex_heartbeat",&json!({"phase":"executing","elapsedSeconds":120,"outputQuietSeconds":119,"processAlive":true,"needsDiagnosis":false,"pid":42,"privatePrompt":"must not leak","path":"private"})).unwrap();
    store.event(&text,"codex_heartbeat",&json!({"phase":"executing","elapsedSeconds":150,"outputQuietSeconds":149,"processAlive":true,"needsDiagnosis":true,"lastEventType":"turn.completed","lastEventAgeSeconds":3,"pid":42,"privatePrompt":"must not leak","path":"private"})).unwrap();
    store
        .event(
            &other,
            "codex_heartbeat",
            &json!({"phase":"writing_stdin","elapsedSeconds":999}),
        )
        .unwrap();
    let progress = store.workflow_progress().unwrap();
    let execution = &progress[0]["nodes"][0]["execution"];
    assert_eq!(execution["elapsedSeconds"], 150);
    assert_eq!(execution["outputQuietSeconds"], 149);
    assert_eq!(execution["needsDiagnosis"], 1);
    assert_eq!(execution["phase"], "executing");
    assert_eq!(execution["lastEventType"], "turn.completed");
    assert_eq!(execution["lastEventAgeSeconds"], 3);
    assert!(execution.get("observedAt").is_some());
    assert!(execution.get("pid").is_none());
    assert!(execution.get("privatePrompt").is_none());
    assert!(execution.get("path").is_none());
    store
        .checkpoint(
            &run,
            "plan:ep1",
            "storyboardPlan",
            "hash",
            "accepted",
            Some(&text),
            Some(&json!({})),
            None,
        )
        .unwrap();
    assert!(store.workflow_progress().unwrap()[0]["nodes"][0]["execution"].is_null());
}

#[test]
fn workflow_progress_projects_only_bound_original_reconnect_without_changing_jobs() {
    for (provider, kind) in [("codex", "codex_event"), ("codex_sdk", "sdk_event")] {
        let (_dir, store) = project();
        store.ensure_workflow_storage().unwrap();
        let run = store.add_job("workflow", "storyboard.run", "local", "local", &json!({"runnerVersion":"desktop-runner/v1"}), "running").unwrap();
        let text = store.add_job("text", "step", provider, "local", &json!({"input":{"episodeId":"ep1"}}), "submitting").unwrap();
        store.checkpoint(&run,"plan:ep1","storyboardPlan","hash","running",Some(&text),None,None).unwrap();
        let save = |raw: Value| {
            store.event(&text,kind,&if provider == "codex_sdk" {json!({"rawEvent":raw})} else {raw}).unwrap();
        };
        let heartbeat = |event: &str| {
            store.event(&text,"codex_heartbeat",&json!({"phase":"executing","elapsedSeconds":150,"outputQuietSeconds":149,"processAlive":true,"lastEventType":event,"lastEventAgeSeconds":149})).unwrap();
        };
        save(json!({"type":"thread.started","thread_id":"original"}));
        save(json!({"type":"turn.started"}));
        save(json!({"type":"error","message":"Reconnecting... 2/5 (stream disconnected before completion: PRIVATE_URL)"}));
        heartbeat("error");
        let before: (String,i64) = store.conn().unwrap().query_row("SELECT snapshot,(SELECT COUNT(*) FROM events) FROM jobs WHERE id=?1",[&text],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        for _ in 0..2 {
            let progress = store.workflow_progress().unwrap();
            assert_eq!(progress[0]["nodes"][0]["execution"]["reconnect"],json!({"version":"stream-reconnect-progress/v1","attempt":2,"limit":5}));
            assert!(!serde_json::to_string(&progress).unwrap().contains("PRIVATE_URL"));
        }
        let after: (String,i64) = store.conn().unwrap().query_row("SELECT snapshot,(SELECT COUNT(*) FROM events) FROM jobs WHERE id=?1",[&text],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(before,after,"read-only projection cannot change requests or events");
        save(json!({"type":"error","message":"unrecognized real error"}));
        let execution=store.workflow_progress().unwrap()[0]["nodes"][0]["execution"].clone();
        assert!(execution.get("reconnect").is_none());
        assert_eq!(execution["lastEventType"],"error");
        save(json!({"type":"turn.failed","error":{"message":"real terminal failure"}}));
        heartbeat("turn.failed");
        let execution=store.workflow_progress().unwrap()[0]["nodes"][0]["execution"].clone();
        assert!(execution.get("reconnect").is_none());
        assert_eq!(execution["lastEventType"],"turn.failed");
    }
}

fn resolution_snapshot(request: &pipeline::TextRequest) -> Value {
    let mut snapshot = serde_json::to_value(request).unwrap();
    snapshot["executorIdentity"] = json!({"provider":"offline"});
    snapshot["executorPolicy"] = json!("offline-isolation/v1");
    snapshot
}

fn resolution_step(
    store: &Store,
    run: &str,
    key: &str,
    request: &pipeline::TextRequest,
    accepted: bool,
    raw: &Value,
    domain: &Value,
) -> (String, String) {
    let snapshot = resolution_snapshot(request);
    let hash = pipeline::digest(&snapshot.to_string());
    let state = if accepted { "completed" } else { "rejected" };
    let error = (!accepted).then_some(crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION);
    let job = store
        .add_job(
            "text",
            "resolution-fixture",
            "offline",
            "local",
            &snapshot,
            state,
        )
        .unwrap();
    store.update_job(&job,state,&json!({"result":raw,"error":error,"receipt":{"validation":if accepted{"accepted"}else{"rejected"},"contract":"desktop-runner/v1","acceptedRequestHash":if accepted{json!(hash)}else{Value::Null}}})).unwrap();
    // Rejected real receipts omit acceptedRequestHash rather than storing null.
    if !accepted {
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET receipt=?1 WHERE id=?2",
                rusqlite::params![
                    json!({"validation":"rejected","contract":"desktop-runner/v1"}).to_string(),
                    job
                ],
            )
            .unwrap();
    }
    store
        .event(
            &job,
            "text_received_raw",
            &json!({"requestHash":hash,"value":raw}),
        )
        .unwrap();
    store
        .checkpoint(
            run,
            key,
            &request.stage,
            &hash,
            if accepted { "accepted" } else { "rejected" },
            Some(&job),
            Some(domain),
            error,
        )
        .unwrap();
    (job, hash)
}

struct ScopeResolutionFixture {
    dir: tempfile::TempDir,
    store: Store,
    run: String,
    author: pipeline::TextRequest,
    author_job: String,
    author_node: String,
    author_hash: String,
    resolver: pipeline::TextRequest,
    resolver_job: String,
    resolver_node: String,
    resolver_hash: String,
    result: Value,
}

fn scope_resolution_fixture(two_attempts: bool) -> ScopeResolutionFixture {
    let (dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let source = store.source().unwrap();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1","input":{"source":source}}),
            "running",
        )
        .unwrap();
    let episode = source["episodes"][0]["id"].clone();
    let rows: Vec<_> = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "body" && r["episodeId"] == episode)
        .cloned()
        .collect();
    let author = pipeline::TextRequest {
        stage: "storyboardCreate".into(),
        skill: "unit-performance-director".into(),
        model: "offline".into(),
        reasoning: "medium".into(),
        input: json!({"episodeId":episode,"sourceRows":rows,"contextRows":[],"capabilities":{"maxDuration":30},"executionBatch":{"version":"source-production-batch/v1","id":"B1"},"storyboardCreativeProfile":"original-production-contract/v1"}),
        schema: json!({"type":"object"}),
        rule_snapshot: json!({"primary":{"id":"unit-performance-director","version":"original"}}),
    };
    let author_node = "author:batch:B1".to_string();
    let draft = json!({"schemaVersion":"storyboard-draft/v2","units":complete_episode(&store,0)["units"],"fantasyRealization":[],"revisionNotes":[],"unresolved":[]});
    let mut raw = draft.clone();
    raw["privatePrompt"] = json!("DO_NOT_EXPOSE_AUTHOR_RAW");
    let (author_job, author_hash) =
        resolution_step(&store, &run, &author_node, &author, false, &raw, &raw);
    let author_domain = pipeline::digest(&serde_json::to_string(&author).unwrap());
    let raw_sha = pipeline::digest(&raw.to_string());
    let mut base = draft.clone();
    base["units"][0]["id"] = json!("scope-base-before-progress");
    let base_hash = pipeline::finish_base_hash(&base);
    // Freeze the actual delta schema against each attempt's own prior draft.
    // A generic object schema cannot prove the production two-attempt contract.
    let full_schema = json!({"type":"object","properties":{
        "units":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"episodeId":{"type":"string"},"exitState":{"type":"string"}},"required":["id","episodeId","exitState"],"additionalProperties":false}},
        "fantasyRealization":{"type":"array","items":{"type":"object","properties":{},"additionalProperties":false}},
        "revisionNotes":{"type":"array","items":{"type":"string"}},
        "unresolved":{"type":"array","items":{"type":"string"}}
    },"required":["units","fantasyRealization","revisionNotes","unresolved"],"additionalProperties":false});
    let mut resolver = pipeline::TextRequest {
        stage: "storyboardFinish".into(),
        skill: "episode-finishing-editor".into(),
        model: "offline".into(),
        reasoning: "medium".into(),
        input: json!({"episodeId":episode,"sourceRows":rows,"contextRows":[],"capabilities":{"maxDuration":30},"finishMode":"delta","previousDraft":base,"baseDraftHash":base_hash,"executionBatch":{"version":"author-domain-repair/v1","id":"repair:1"},"authorRepair":{"version":"rejected-author-scope-repair/v1","maximumAttempts":2,"attempt":1,"sourceSha256":source["sha256"],"originDomainRequestSha256":author_domain,"originalAuthorRawSha256":raw_sha,"originExecutionBatch":author.input["executionBatch"],"baseDraftHash":base_hash},"storyboardCreativeProfile":"original-production-contract/v1"}),
        schema: pipeline::scope_repair_delta_schema_for_test(&full_schema, &base),
        rule_snapshot: json!({"primary":{"id":"episode-finishing-editor","version":"original"}}),
    };
    let mut lineage = Vec::new();
    if two_attempts {
        let domain = pipeline::digest(&serde_json::to_string(&resolver).unwrap());
        let first_raw = json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":base_hash,"retainedUnitIds":[],"replacements":[],"revisionNotes":[],"unresolved":[]});
        resolution_step(
            &store,
            &run,
            "repair:attempt1",
            &resolver,
            false,
            &first_raw,
            &first_raw,
        );
        let mut progressed = base.clone();
        progressed["units"][0]["id"] = json!("scope-base-after-progress");
        progressed["units"][0]["exitState"] = json!("合成第一轮已进展，仍待范围修订");
        let next_base = pipeline::finish_base_hash(&progressed);
        lineage.push(json!({"attempt":1,"canonicalDomainRequestSha256":domain,"baseDraftHash":base_hash,"draftSha256":next_base,"result":"rejected-scope"}));
        resolver.input["baseDraftHash"] = json!(next_base);
        resolver.input["previousDraft"] = progressed.clone();
        resolver.schema = pipeline::scope_repair_delta_schema_for_test(&full_schema, &progressed);
        resolver.input["authorRepair"]["baseDraftHash"] = json!(next_base);
        resolver.input["authorRepair"]["attempt"] = json!(2);
        resolver.input["executionBatch"]["id"] = json!("repair:2");
    }
    let resolver_domain = pipeline::digest(&serde_json::to_string(&resolver).unwrap());
    let draft_hash=pipeline::digest(&json!({"schemaVersion":draft["schemaVersion"],"units":draft["units"],"fantasyRealization":draft["fantasyRealization"]}).to_string());
    lineage.push(json!({"attempt":if two_attempts{2}else{1},"canonicalDomainRequestSha256":resolver_domain,"baseDraftHash":resolver.input["baseDraftHash"],"draftSha256":draft_hash}));
    let mut materialized = draft;
    materialized["authorRepairProvenance"] = json!({"version":"accepted-author-scope-repair/v1","sourceSha256":source["sha256"],"originDomainRequestSha256":author_domain,"originalAuthorRawSha256":raw_sha,"originalAuthorAccepted":false,"nodes":lineage});
    let wire_raw = json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":resolver.input["baseDraftHash"],"retainedUnitIds":[],"replacements":[],"privatePrompt":"DO_NOT_EXPOSE_FINISH_RAW","revisionNotes":[],"unresolved":[]});
    let mut result = wire_raw.clone();
    result["materializedDraft"] = materialized;
    let resolver_node = if two_attempts {
        "repair:attempt2"
    } else {
        "repair:attempt1"
    }
    .to_string();
    let (resolver_job, resolver_hash) = resolution_step(
        &store,
        &run,
        &resolver_node,
        &resolver,
        true,
        &wire_raw,
        &result,
    );
    ScopeResolutionFixture {
        dir,
        store,
        run,
        author,
        author_job,
        author_node,
        author_hash,
        resolver,
        resolver_job,
        resolver_node,
        resolver_hash,
        result,
    }
}

fn resolution_records(store: &Store, run: &str) -> Value {
    let c = store.conn().unwrap();
    let mut query=c.prepare("SELECT w.node_key,w.request_hash,w.state,w.error,w.result,j.id,j.snapshot,j.state,j.error,j.result,j.receipt FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 ORDER BY w.rowid").unwrap();
    let rows = query
        .query_map([run], |r| {
            Ok(json!((0..11)
                .map(|i| r.get::<_, Option<String>>(i))
                .collect::<std::result::Result<Vec<_>, _>>()?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect::<Vec<_>>();
    json!(rows)
}

#[test]
fn workflow_progress_resolves_scope_author_by_domain_hash_and_keeps_all_original_rows() {
    let f = scope_resolution_fixture(false);
    let before = resolution_records(&f.store, &f.run);
    let progress = f.store.workflow_progress().unwrap();
    let nodes = progress[0]["nodes"].as_array().unwrap();
    let original = nodes.iter().find(|n| n["id"] == f.author_node).unwrap();
    let domain = pipeline::digest(&serde_json::to_string(&f.author).unwrap());
    assert_ne!(domain, f.author_hash);
    assert_eq!(original["requestHash"], f.author_hash);
    assert_eq!(original["status"], "rejected");
    assert_eq!(
        original["error"],
        crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION
    );
    assert_eq!(
        original["resolution"],
        json!({"version":"workflow-step-resolution/v1","kind":"author-scope-repair","workflowId":f.run,"originNodeId":f.author_node,"originRequestHash":f.author_hash,"resolverNodeId":f.resolver_node,"resolverRequestHash":f.resolver_hash,"verified":true})
    );
    assert!(nodes.iter().find(|n| n["id"] == f.resolver_node).unwrap()["resolution"].is_null());
    assert_eq!(resolution_records(&f.store, &f.run), before);
    let rendered = serde_json::to_string(&progress).unwrap();
    assert!(!rendered.contains("DO_NOT_EXPOSE"));
    assert!(!rendered.contains("materializedDraft"));
    assert!(!rendered.contains("executorIdentity"));
    let reopened = Store::open(f.dir.path().to_owned()).unwrap();
    assert_eq!(reopened.workflow_progress().unwrap(), progress);
}

#[test]
fn workflow_progress_resolves_rejected_first_scope_editor_only_with_the_second_verified_lineage() {
    let f = scope_resolution_fixture(true);
    let first_snapshot: String = f.store.conn().unwrap().query_row(
        "SELECT j.snapshot FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key='repair:attempt1'",
        [&f.run], |row| row.get(0),
    ).unwrap();
    let first: pipeline::TextRequest = serde_json::from_str(&first_snapshot).unwrap();
    assert_ne!(first.schema, f.resolver.schema);
    for request in [&first, &f.resolver] {
        let base = &request.input["previousDraft"];
        let hash = pipeline::finish_base_hash(base);
        let ids: Vec<_> = base["units"].as_array().unwrap().iter().map(|unit|unit["id"].clone()).collect();
        assert_eq!(request.input["baseDraftHash"], hash);
        assert_eq!(request.input["authorRepair"]["baseDraftHash"], hash);
        assert_eq!(request.schema.pointer("/properties/baseDraftHash/enum"), Some(&json!([hash])));
        assert_eq!(request.schema.pointer("/properties/retainedUnitIds/items/enum"), Some(&json!(ids)));
        assert_eq!(request.schema.pointer("/properties/replacements/items/properties/replaces/items/enum"), Some(&json!(ids)));
    }
    let before = resolution_records(&f.store, &f.run);
    let progress = f.store.workflow_progress().unwrap();
    let nodes = progress[0]["nodes"].as_array().unwrap();
    for id in [f.author_node.as_str(), "repair:attempt1"] {
        let old = nodes.iter().find(|n| n["id"] == id).unwrap();
        assert_eq!(old["status"], "rejected");
        assert_eq!(old["resolution"]["kind"], "author-scope-repair");
        assert_eq!(old["resolution"]["resolverNodeId"], f.resolver_node);
    }
    assert_eq!(
        nodes.iter().find(|n| n["id"] == "repair:attempt1").unwrap()["input"]
            ["executionBatchVersion"],
        "author-domain-repair/v1"
    );
    assert_eq!(resolution_records(&f.store, &f.run), before);
    let mut bad = f.result.clone();
    bad["materializedDraft"]["authorRepairProvenance"]["nodes"][0]["draftSha256"] =
        json!(pipeline::digest("unrelated-base"));
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE workflow_steps SET result=?1 WHERE workflow_id=?2 AND node_key=?3",
            rusqlite::params![bad.to_string(), f.run, f.resolver_node],
        )
        .unwrap();
    let progress = f.store.workflow_progress().unwrap();
    assert!(progress[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "repair:attempt1")
        .unwrap()["resolution"]
        .is_null());
}

#[test]
fn workflow_progress_scope_resolution_rejects_unbound_snapshot_source_raw_receipt_and_cross_workflow(
) {
    for pollution in [
        "domainUsesSnapshotHash",
        "source",
        "raw",
        "episode",
        "draft",
        "nodeHash",
        "receipt",
        "receiveEvent",
        "crossWorkflow",
        "unaccepted",
    ] {
        let f = scope_resolution_fixture(false);
        let c = f.store.conn().unwrap();
        match pollution {
            "domainUsesSnapshotHash" | "source" | "raw" | "episode" | "draft" => {
                let mut bad = f.result.clone();
                match pollution {
                    "domainUsesSnapshotHash" => {
                        bad["materializedDraft"]["authorRepairProvenance"]
                            ["originDomainRequestSha256"] = json!(f.author_hash)
                    }
                    "source" => {
                        bad["materializedDraft"]["authorRepairProvenance"]["sourceSha256"] =
                            json!(pipeline::digest("other-source"))
                    }
                    "raw" => {
                        bad["materializedDraft"]["authorRepairProvenance"]
                            ["originalAuthorRawSha256"] = json!(pipeline::digest("other-raw"))
                    }
                    "episode" => {
                        bad["materializedDraft"]["units"][0]["episodeId"] = json!("other-episode")
                    }
                    _ => {
                        bad["materializedDraft"]["units"][0]["exitState"] =
                            json!("changed without lineage")
                    }
                }
                c.execute(
                    "UPDATE workflow_steps SET result=?1 WHERE workflow_id=?2 AND node_key=?3",
                    rusqlite::params![bad.to_string(), f.run, f.resolver_node],
                )
                .unwrap();
            }
            "nodeHash" => {
                c.execute("UPDATE workflow_steps SET request_hash=?1 WHERE workflow_id=?2 AND node_key=?3",rusqlite::params![pipeline::digest("not-this-snapshot"),f.run,f.resolver_node]).unwrap();
            }
            "receipt" => {
                c.execute(
                    "UPDATE jobs SET receipt=?1 WHERE id=?2",
                    rusqlite::params![
                        json!({"validation":"accepted","contract":"other"}).to_string(),
                        f.resolver_job
                    ],
                )
                .unwrap();
            }
            "receiveEvent" => {
                f.store
                    .event(
                        &f.author_job,
                        "text_received_raw",
                        &json!({"requestHash":f.author_hash,"value":{"other":true}}),
                    )
                    .unwrap();
            }
            "crossWorkflow" => {
                let other=f.store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","input":{"source":f.store.source().unwrap()}}),"running").unwrap();
                c.execute(
                    "UPDATE workflow_steps SET workflow_id=?1 WHERE workflow_id=?2 AND node_key=?3",
                    rusqlite::params![other, f.run, f.resolver_node],
                )
                .unwrap();
            }
            _ => {
                c.execute(
                    "UPDATE jobs SET state='received' WHERE id=?1",
                    [&f.resolver_job],
                )
                .unwrap();
            }
        }
        let before = resolution_records(&f.store, &f.run);
        let progress = f.store.workflow_progress().unwrap();
        let run = progress.iter().find(|p| p["workflowId"] == f.run).unwrap();
        let old = run["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == f.author_node)
            .unwrap();
        assert!(old["resolution"].is_null(), "{pollution}");
        assert_eq!(old["status"], "rejected");
        assert_eq!(resolution_records(&f.store, &f.run), before);
    }
}

#[test]
fn workflow_progress_scope_resolution_is_fail_closed_when_two_accepted_resolvers_claim_the_same_author(
) {
    let f = scope_resolution_fixture(false);
    let mut request = f.resolver.clone();
    request.input["executionBatch"]["id"] = json!("other-explicit-repair");
    let mut result = f.result.clone();
    result["materializedDraft"]["authorRepairProvenance"]["nodes"][0]
        ["canonicalDomainRequestSha256"] =
        json!(pipeline::digest(&serde_json::to_string(&request).unwrap()));
    let mut raw = result.clone();
    raw.as_object_mut().unwrap().remove("materializedDraft");
    resolution_step(
        &f.store,
        &f.run,
        "other-resolver",
        &request,
        true,
        &raw,
        &result,
    );
    let progress = f.store.workflow_progress().unwrap();
    let old = progress[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == f.author_node)
        .unwrap();
    assert!(old["resolution"].is_null());
    assert_eq!(old["status"], "rejected");
}

#[test]
fn workflow_progress_continuity_supersession_requires_the_real_event_and_exact_previous_tail_pair()
{
    for mode in [
        "valid",
        "missingEvent",
        "wrongEvent",
        "wrongTail",
        "wrongNodeSuffix",
    ] {
        let (_dir, store) = project();
        store.ensure_workflow_storage().unwrap();
        let source = store.source().unwrap();
        let episode = &source["episodes"][1]["id"];
        let run = store
            .add_job(
                "workflow",
                "storyboard.run",
                "local",
                "local",
                &json!({"runnerVersion":"desktop-runner/v1","input":{"source":source}}),
                "running",
            )
            .unwrap();
        let rows: Vec<_> = source["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "body" && r["episodeId"] == *episode)
            .cloned()
            .collect();
        let prior = json!({"id":"tail","episodeId":source["episodes"][0]["id"],"exitState":"old actual tail"});
        let current = json!({"id":"tail","episodeId":source["episodes"][0]["id"],"exitState":"new actual tail"});
        let old = pipeline::TextRequest {
            stage: "storyboardPlan".into(),
            skill: "episode-emotion-planner".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"episode":{"id":episode},"sourceRows":rows,"previousBoundary":prior}),
            schema: json!({"type":"object"}),
            rule_snapshot: json!({"primary":{"id":"episode-emotion-planner","version":"original"}}),
        };
        let mut new = old.clone();
        let revision = json!({"version":"accepted-tail-change/v1","reason":"VIDEO_CARRIER_REPAIR","sourceSha256":source["sha256"],"previousEpisodeId":source["episodes"][0]["id"],"priorTailSha256":pipeline::digest(&prior.to_string()),"currentTailSha256":pipeline::digest(&current.to_string()),"parentRevisionSha256":null});
        new.input["continuityRevision"] = revision.clone();
        new.input["previousBoundary"] = if mode == "wrongTail" {
            json!({"id":"unrelated"})
        } else {
            current
        };
        let old_key = "storyboardPlan:episode:";
        let suffix = if mode == "wrongNodeSuffix" {
            pipeline::digest("other-revision")
        } else {
            pipeline::digest(&revision.to_string())
        };
        let new_key = format!("{old_key}:continuity:{suffix}");
        let raw = json!({"schemaVersion":"storyboard-plan/v2"});
        let (_, old_hash) = resolution_step(&store, &run, old_key, &old, true, &raw, &raw);
        let (_, new_hash) = resolution_step(&store, &run, &new_key, &new, true, &raw, &raw);
        if mode != "missingEvent" {
            let mut event = revision.clone();
            event["episodeId"] = episode.clone();
            event["action"] = json!("recompute_affected_chain");
            event["previouslyAcceptedPlan"] = json!(true);
            if mode == "wrongEvent" {
                event["currentTailSha256"] = json!(pipeline::digest("not-current-tail"));
            }
            store
                .event(&run, "storyboard_continuity_updated", &event)
                .unwrap();
        }
        let before = resolution_records(&store, &run);
        let progress = store.workflow_progress().unwrap();
        let projected = progress[0]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == old_key)
            .unwrap();
        if mode == "valid" {
            assert_eq!(
                projected["resolution"],
                json!({"version":"workflow-step-resolution/v1","kind":"continuity-superseded","workflowId":run,"originNodeId":old_key,"originRequestHash":old_hash,"resolverNodeId":new_key,"resolverRequestHash":new_hash,"verified":true})
            );
        } else {
            assert!(projected["resolution"].is_null(), "{mode}");
        }
        assert_eq!(projected["status"], "accepted");
        assert_eq!(resolution_records(&store, &run), before);
    }
}
fn unit(store: &Store, id: &str, episode: usize) -> Value {
    json!({"id":id,"episodeId":store.source().unwrap()["episodes"][episode]["id"],"title":id,"prompt":"原话与完整演出","duration":4,"assetRefs":[],"exitState":"站在门口"})
}

#[test]
fn partial_episode_drafts_do_not_retire_or_replace_the_existing_complete_episode() {
    let (_dir, store) = project();
    let old = store.save_unit(unit(&store, "old-complete", 0)).unwrap();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let draft = unit(&store, "new-first-batch", 0);
    let output = json!({"scope":"episodes","status":"partially_blocked","updatedEpisodeIds":[],"units":[draft],"blocked":[{"reason":"后批失败"}]});
    store
        .publish_storyboard(&guard, &output, &frozen, None)
        .unwrap();
    assert_eq!(store.values("units").unwrap(), vec![old]);
}

#[test]
fn completed_episode_can_publish_while_the_failed_episode_keeps_old_units() {
    let (_dir, store) = project();
    store.save_unit(unit(&store, "old-first", 0)).unwrap();
    let failed = store.save_unit(unit(&store, "old-second", 1)).unwrap();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let ready = unit(&store, "new-first", 0);
    let incomplete = unit(&store, "new-second-part", 1);
    let output = json!({"scope":"episodes","status":"partially_blocked","updatedEpisodeIds":[ready["episodeId"]],"units":[ready,incomplete]});
    store
        .publish_storyboard(&guard, &output, &frozen, None)
        .unwrap();
    assert_eq!(store.get("units", "old-second").unwrap(), failed);
    assert!(store.get("units", "new-second-part").is_err());
    assert_eq!(store.get("units", "old-first").unwrap()["retired"], true);
    assert_eq!(
        store.get("units", "new-first").unwrap()["title"],
        "new-first"
    );
}

#[test]
fn foundation_publication_preserves_design_and_media_and_is_atomic() {
    let (_dir, store) = project();
    let asset = store
        .save_asset(
            json!({"id":"actor","variantId":"base","prompt":"已确认服装与形象","name":"小王"}),
        )
        .unwrap();
    let image = store.root.join("fixture.png");
    image::RgbImage::new(2, 2).save(&image).unwrap();
    let media = store
        .import_media(&image, "image", Some("actor"), Some("base"))
        .unwrap();
    store.adopt("actor", "base", s(&media, "id")).unwrap();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let foundation = json!({"episodes":[],"assets":[],"facts":[],"direction":"方向"});
    store.publish_foundation(&guard,foundation.clone(),vec![json!({"id":"actor","variantId":"base","prompt":"","name":"小王","identityId":"person"})],None).unwrap();
    assert_eq!(
        store.get("assets", "actor").unwrap()["prompt"],
        asset["prompt"]
    );
    assert_eq!(
        store.adopted("actor", "base").unwrap().as_deref(),
        Some(s(&media, "id"))
    );
    let version = store.foundation().unwrap()["version"].clone();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    assert!(store
        .publish_foundation(
            &guard,
            json!({"direction":"坏批次"}),
            vec![json!("不是资产")],
            None
        )
        .is_err());
    assert_eq!(store.foundation().unwrap()["version"], version);
}
#[test]
fn changing_source_or_design_during_run_rejects_publication() {
    let (_dir, store) = project();
    store
        .save_asset(json!({"id":"actor","variantId":"base","prompt":"旧"}))
        .unwrap();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let mut asset = store.get("assets", "actor").unwrap();
    asset["prompt"] = json!("新");
    store.save_asset(asset).unwrap();
    assert!(store
        .publish_foundation(&guard, json!({"direction":"不能覆盖"}), vec![], None)
        .is_err());
    assert!(store.foundation().unwrap().is_null());
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let mut source = store.source().unwrap();
    source["sha256"] = json!("新源");
    store.save_source(source).unwrap();
    assert!(store
        .publish_foundation(&guard, json!({}), vec![], None)
        .is_err());
}
#[test]
fn selected_episode_replacement_retires_old_units_but_keeps_history() {
    let (_dir, store) = project();
    store.save_unit(unit(&store, "old", 0)).unwrap();
    store.save_unit(unit(&store, "other", 1)).unwrap();
    let mut frozen = input(&store);
    frozen["selection"]["episodeIds"] = json!([store.source().unwrap()["episodes"][0]["id"]]);
    let guard = guard_from_input(&frozen);
    let output = json!({"units":[unit(&store,"new",0)],"scope":"episodes","status":"completed"});
    store
        .publish_storyboard(&guard, &output, &frozen, None)
        .unwrap();
    let ids: Vec<_> = store
        .values("units")
        .unwrap()
        .into_iter()
        .map(|u| s(&u, "id").to_owned())
        .collect();
    assert_eq!(ids, vec!["other", "new"]);
    assert_eq!(store.get("units", "old").unwrap()["retired"], true);
    assert_eq!(
        store.unit_versions("old").unwrap()["versions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn partial_unit_revision_does_not_retire_unselected_siblings() {
    let (_dir, store) = project();
    store.save_unit(unit(&store, "first", 0)).unwrap();
    store.save_unit(unit(&store, "second", 0)).unwrap();
    let mut frozen = input(&store);
    frozen["selection"]["unitIds"] = json!(["first"]);
    let guard = guard_from_input(&frozen);
    store.publish_storyboard(&guard,&json!({"units":[unit(&store,"replacement",0)],"scope":"units","replacedUnitIds":["first"]}),&frozen,None).unwrap();
    assert!(store
        .values("units")
        .unwrap()
        .iter()
        .any(|u| u["id"] == "second"));
}
#[test]
fn accepted_steps_survive_reopen_and_workflow_publication_guard_survives_partial_publish() {
    let (dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1","input":frozen,"guard":guard}),
            "running",
        )
        .unwrap();
    store
        .checkpoint(
            &run,
            "plan:ep1",
            "storyboardPlan",
            "hash",
            "accepted",
            None,
            Some(&json!({"events":[]})),
            None,
        )
        .unwrap();
    let output = json!({"units":[unit(&store,"new",0)],"status":"waiting_decision","waitingDecision":{"episodeId":"next","candidates":[]}});
    store
        .publish_storyboard(&guard, &output, &frozen, Some(&run))
        .unwrap();
    drop(store);
    let reopened = Store::open(dir.path().to_owned()).unwrap();
    assert!(reopened.accepted_step(&run, "hash").unwrap().is_some());
    assert_eq!(reopened.job(&run).unwrap()["status"], "waiting_decision");
    let guard = reopened.job(&run).unwrap()["prepared"]["publicationGuard"].clone();
    assert_eq!(guard["units"]["new"], 1);
    let mut c = reopened.conn().unwrap();
    let tx = c.transaction().unwrap();
    crate::workflow::verify_guard(&tx, &guard).unwrap();
}
#[test]
fn user_changes_to_selected_units_prevent_late_publication_and_unchanged_saves_do_not_bump() {
    let (_dir, store) = project();
    let original = store.save_unit(unit(&store, "u", 0)).unwrap();
    assert_eq!(store.save_unit(original.clone()).unwrap()["version"], 1);
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let mut edited = original;
    edited["prompt"] = json!("用户手改");
    store.save_unit(edited).unwrap();
    assert!(store
        .publish_storyboard(
            &guard,
            &json!({"units":[unit(&store,"u",0)]}),
            &frozen,
            None
        )
        .is_err());
    assert_eq!(store.get("units", "u").unwrap()["prompt"], "用户手改");
}
#[test]
fn explicit_direction_is_versioned_and_rejects_stale_editor() {
    let (_dir, store) = project();
    let source_version = store.source().unwrap()["version"].as_i64().unwrap();
    store
        .save_production_direction("克制表演", source_version, Some(0))
        .unwrap();
    assert_eq!(
        store.foundation().unwrap()["productionDirection"],
        "克制表演"
    );
    assert_eq!(store.foundation().unwrap()["sourceSha256"], store.source().unwrap()["sha256"]);
    assert_eq!(store.foundation().unwrap()["sourceVersion"], source_version);
    serde_json::from_value::<pipeline::FoundationProposal>(store.foundation().unwrap()).unwrap();
    assert!(store
        .save_production_direction("旧编辑器的方向", source_version, Some(0))
        .is_err());
}

#[test]
fn foundation_rebuild_retains_confirmed_direction() {
    let (_dir, store) = project();
    let sv = store.source().unwrap()["version"].as_i64().unwrap();
    store
        .save_production_direction("自然、克制", sv, Some(0))
        .unwrap();
    let frozen = input(&store);
    store
        .publish_foundation(
            &guard_from_input(&frozen),
            json!({"direction":"模型提案","assets":[],"episodes":[],"facts":[]}),
            vec![],
            None,
        )
        .unwrap();
    assert_eq!(
        store.foundation().unwrap()["productionDirection"],
        "自然、克制"
    );
    assert_eq!(store.foundation().unwrap()["directionConfirmed"], true);
}

#[test]
fn revision_expands_required_scope_and_versions_only_affected_continuity() {
    let (_dir, store) = project();
    let frozen = input(&store);
    store
        .publish_storyboard(
            &guard_from_input(&frozen),
            &json!({"units":[unit(&store,"a",0),unit(&store,"b",0),unit(&store,"c",1)]}),
            &frozen,
            None,
        )
        .unwrap();
    let mut frozen = input(&store);
    frozen["selection"]["unitIds"] = json!(["a"]);
    let mut replacement = unit(&store, "merged", 0);
    replacement["exitState"] = json!("已出门");
    store.publish_storyboard(&guard_from_input(&frozen),&json!({"units":[replacement],"scope":"units","replacedUnitIds":["a","b"],"continuityImpacts":[{"nextUnitId":"c"}]}),&frozen,None).unwrap();
    assert_eq!(store.get("units", "a").unwrap()["retired"], true);
    assert_eq!(store.get("units", "b").unwrap()["retired"], true);
    let c = store.get("units", "c").unwrap();
    assert_eq!(c["continuityStale"], true);
    assert_eq!(c["version"], 2);
    assert_eq!(c["prompt"], "原话与完整演出");
    store.invalidate_unit_dependents("merged").unwrap();
    assert_eq!(
        store.get("units", "c").unwrap()["version"],
        2,
        "already stale is a no-op"
    );
    let previous: String = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT dependency_id FROM unit_dependencies WHERE unit_id='c'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(previous, "merged");
}

#[test]
fn incomplete_text_recovers_separately_from_unknown_media_submission() {
    let (_dir, store) = project();
    store.ensure_workflow_storage().unwrap();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({}),
            "running",
        )
        .unwrap();
    let text = store
        .add_job("text", "step", "codex", "local", &json!({}), "received")
        .unwrap();
    store
        .update_job(
            &text,
            "received",
            &json!({"result":{"raw":"保留待校验原回包"}}),
        )
        .unwrap();
    store
        .checkpoint(
            &run,
            "step",
            "storyboardCreate",
            "hash",
            "running",
            Some(&text),
            None,
            None,
        )
        .unwrap();
    let image = store
        .add_job(
            "image",
            "actor",
            "offline",
            "local",
            &json!({}),
            "submitting",
        )
        .unwrap();
    store.recover_interrupted().unwrap();
    assert_eq!(store.job(&run).unwrap()["status"], "interrupted");
    assert_eq!(store.job(&text).unwrap()["status"], "interrupted");
    assert_eq!(
        store.job(&text).unwrap()["result"]["raw"],
        "保留待校验原回包"
    );
    assert!(store.accepted_step(&run, "hash").unwrap().is_none());
    assert_eq!(store.job(&image).unwrap()["status"], "submission_unknown");
}

#[test]
fn adding_asset_during_creation_does_not_overwrite_the_new_catalog_entry() {
    let (_dir, store) = project();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    store
        .save_asset(json!({"id":"new-user-asset","variantId":"base","name":"用户新建"}))
        .unwrap();
    assert!(store
        .publish_foundation(
            &guard,
            json!({}),
            vec![json!({"id":"new-user-asset","variantId":"base","name":"旧模型回包"})],
            None
        )
        .is_err());
    assert_eq!(
        store.get("assets", "new-user-asset").unwrap()["name"],
        "用户新建"
    );
}

#[test]
fn foundation_publication_counts_active_assets_and_does_not_restore_deleted_rows() {
    let (_dir, store) = project();
    let original = store.save_asset(json!({"id":"removed","variantId":"base","name":"测试资产","prompt":"保留历史图片提示词"})).unwrap();
    let deleted = store
        .set_asset_deleted("removed", original["version"].as_i64().unwrap(), true)
        .unwrap();
    let mut frozen = input(&store);
    assert!(frozen["assets"].as_array().unwrap().is_empty());
    // Older frozen input can include a tombstone; the current guard counts only live rows.
    frozen["assets"] = json!([deleted]);
    let guard = guard_from_input(&frozen);
    assert_eq!(guard["assets"], json!({}));
    store
        .publish_foundation(
            &guard,
            json!({"assets":[{"id":"prior-source-identity"}],"episodes":[],"facts":[]}),
            vec![original],
            None,
        )
        .unwrap();
    let current = store.get("assets", "removed").unwrap();
    assert_eq!(
        current, deleted,
        "a stale foundation proposal must not change or restore a deleted asset"
    );
    assert!(store.values("assets").unwrap().is_empty());
    let versions: i64 = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM asset_versions WHERE asset_id='removed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(versions, 2);
}

#[test]
fn deleting_asset_during_creation_rejects_late_publication_without_changing_snapshot() {
    let (_dir, store) = project();
    let original = store
        .save_asset(json!({"id":"removed","variantId":"base","name":"测试资产"}))
        .unwrap();
    let frozen = input(&store);
    let guard = guard_from_input(&frozen);
    let deleted = store
        .set_asset_deleted("removed", original["version"].as_i64().unwrap(), true)
        .unwrap();
    let error = store
        .publish_foundation(
            &guard,
            json!({"direction":"过期模型输出"}),
            vec![original],
            None,
        )
        .unwrap_err();
    assert!(error.contains("已更新"));
    assert_eq!(store.get("assets", "removed").unwrap(), deleted);
    assert!(store.foundation().unwrap().is_null());
    assert_eq!(
        frozen["assets"][0]["version"], 1,
        "submitted frozen inputs remain immutable"
    );
}
