//! Offline engine route checks with durable original role/raw/adoption records.
use super::*;

#[test]
fn frozen_series_workflow_resume_audits_its_own_rule_pack_without_replacing_history() {
    let mut input = json!({});
    pipeline::series::mark_legacy_input(&mut input).unwrap();
    input.as_object_mut().unwrap().remove("seriesDesignSourceUses");
    let rules = json!(pipeline::series::rule_snapshots_for(&input).unwrap());
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run",
        "input":input,"rules":rules});
    let job = json!({"kind":"workflow","entityId":"storyboard.run","snapshot":snapshot});
    let original = job.clone();
    assert!(job["snapshot"]["input"].get("seriesDesignSourceUses").is_none());
    let current = production_rules_for_snapshot(&job["snapshot"]).unwrap();
    assert_eq!(current, rules);
    assert!(workflow_resume_rule_audit(&job,&current).unwrap().is_none());
    assert_eq!(job,original);
    let mut tampered=job.clone();
    tampered["snapshot"]["rules"][0]["content"]=json!("changed original method");
    assert!(workflow_resume_rule_audit(&tampered,&current).is_err());
    assert_eq!(production_rules_for_snapshot(&json!({"stage":"storyboard.run","input":{}})).unwrap(),
        production_rules("storyboard.run"));
}

fn engine_fixture() -> (tempfile::TempDir, Arc<Engine>, Store, String) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::with_channels(
        directory.path().join("home"),
        Arc::new(crate::execution_policy::OfflineChannels),
    )
    .unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"新版分集复用路由离线验收"}))
        .unwrap();
    let pid = s(&project, "id").to_owned();
    let store = engine.store(&pid).unwrap();
    (directory, engine, store, pid)
}

#[tokio::test]
async fn expanded_series_route_preserves_scope_and_uses_current_tail_version_for_cas() {
    let (_directory, _engine, store, _pid) = engine_fixture();
    let seeded = pipeline::series::seed_engine_series_fixture(&store, 1).await;
    let episodes = seeded["productionScope"]["episodeIds"].clone();
    let request = json!({"action":"storyboard.run","episodeIds":episodes});
    let mut input = production_input_for_new_workflow(&store, &request).unwrap();
    assert_eq!(input["seriesDesignSourceUses"],"series-design-source-uses/v1");
    assert_eq!(input["seriesAssetSupplementVersion"],"series-asset-supplement/v3");
    assert_eq!(seeded["settings"]["ratio"], "16:9");
    assert_eq!(input["settings"]["ratio"], seeded["settings"]["ratio"]);
    assert_eq!(
        existing_storyboard_start(&store, &input).unwrap(),
        StoryboardStartBoundary::New
    );
    let jobs = store.jobs().unwrap().len();
    assert!(!prepare_series_continuation(&store, &mut input).unwrap());
    assert_eq!(input["selection"]["episodeIds"], json!([episodes[1]]));
    assert_eq!(input["productionScope"]["episodeIds"], episodes);
    assert_eq!(input["reusedEpisodeIds"], json!([episodes[0]]));
    assert!(input["previousBoundary"]["version"].as_i64().is_some());
    let guard = crate::workflow::guard_from_input(&input);
    {
        let mut connection = store.conn().unwrap();
        let tx = connection.transaction().unwrap();
        crate::workflow::verify_guard(&tx, &guard).unwrap();
    }
    assert_eq!(store.jobs().unwrap().len(), jobs);
    let mut edited = store
        .get("units", s(&input["previousBoundary"], "id"))
        .unwrap();
    edited["exitState"] = json!("用户改动后新的尾态，旧 guard 不能发布续集");
    store.save_unit(edited).unwrap();
    let mut connection = store.conn().unwrap();
    let tx = connection.transaction().unwrap();
    assert!(crate::workflow::verify_guard(&tx, &guard).is_err());
}

#[tokio::test]
async fn recovery_builder_and_busy_start_do_not_allocate_new_asset_identity() {
    let (_directory, engine, store, _pid) = engine_fixture();
    let seeded = pipeline::series::seed_engine_series_fixture(&store, 1).await;
    let episodes = seeded["productionScope"]["episodeIds"].clone();
    store
        .save_asset(json!({"id":"unreserved","name":"杯子","variantId":"base","kind":"PROP"}))
        .unwrap();
    let request = json!({"action":"storyboard.run","episodeIds":[episodes[1]]});
    let original = production_input(&store, &request).unwrap();
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","sourceVersion":original["source"]["version"],"input":original});
    let old = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &snapshot,
            "interrupted",
        )
        .unwrap();
    let before = store.values("assets").unwrap();
    let new = production_input_for_new_workflow(&store, &request).unwrap();
    assert_eq!(new["seriesDesignSourceUses"],"series-design-source-uses/v1");
    assert_eq!(new["seriesAssetSupplementVersion"],"series-asset-supplement/v3");
    assert!(snapshot["input"].get("seriesDesignSourceUses").is_none(),"a resumed legacy snapshot must not receive new supply semantics");
    assert!(snapshot["input"].get("seriesAssetSupplementVersion").is_none());
    assert_eq!(
        existing_storyboard_start(&store, &new).unwrap(),
        StoryboardStartBoundary::Resume(old.clone())
    );
    assert_eq!(store.values("assets").unwrap(), before);
    assert_eq!(store.job(&old).unwrap()["snapshot"], snapshot);
    let jobs = store.jobs().unwrap().len();
    let _lease = engine.acquire_pipeline(&store).unwrap();
    assert!(engine
        .spawn_production(
            store.clone(),
            json!({"action":"storyboard.run","episodeIds":[episodes[0]]})
        )
        .is_err());
    assert_eq!(
        store.values("assets").unwrap(),
        before,
        "busy workflow must not increment asset versions before checking ownership"
    );
    assert!(store
        .get("assets", "unreserved")
        .unwrap()
        .get("placeholderOrdinal")
        .is_none());
    assert_eq!(store.jobs().unwrap().len(), jobs);
}

#[tokio::test]
async fn already_adopted_full_scope_returns_without_workflow_or_account_preflight() {
    let (_directory, engine, store, _pid) = engine_fixture();
    let seeded = pipeline::series::seed_engine_series_fixture(&store, 2).await;
    let before = store.jobs().unwrap();
    let units = store.values("units").unwrap();
    // Deliberately invalid textProvider proves no text preflight or account is
    // consulted when every selected episode already has a valid adoption.
    assert_eq!(
        store.settings().unwrap()["textProvider"],
        "forbidden-in-offline-test"
    );
    let worker_store = store.clone();
    let request =
        json!({"action":"storyboard.run","episodeIds":seeded["productionScope"]["episodeIds"]});
    tokio::task::spawn_blocking(move || engine.spawn_production(worker_store, request))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.jobs().unwrap(), before);
    assert_eq!(store.values("units").unwrap(), units);
    let reuse_events: i64 = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE type='series_adopted_episodes_reused'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reuse_events, 1);
}
