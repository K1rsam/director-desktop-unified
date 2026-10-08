//! Synthetic project settings/guard CAS and inactive Stage3 recovery. No CLI,
//! SDK, account or external request is used by these tests.
use super::*;

fn fixture() -> (tempfile::TempDir, Store, Value) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("project")).unwrap();
    store.create("offline refinement CAS", "fixture").unwrap();
    let source = pipeline::prepare_source("第1集\n场景：门口\n甲转身。\n").unwrap();
    store
        .save_source(serde_json::to_value(source).unwrap())
        .unwrap();
    let source = store.source().unwrap();
    let episode = source["episodes"][0]["id"].clone();
    store
        .save_unit(json!({"id":"existing-manual-unit","episodeId":episode,
        "prompt":"用户原人工稿","sourceRefs":[],"assetRefs":[]}))
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    store
        .add_job(
            "text",
            "original-unknown",
            "codex",
            "offline",
            &json!({"originalFrozen":true}),
            "submission_unknown",
        )
        .unwrap();
    let input = json!({"source":source,"foundation":store.foundation().unwrap(),
        "assets":store.values("assets").unwrap(),"existingUnits":store.values("units").unwrap(),
        "settings":store.settings().unwrap(),"selection":{"episodeIds":[episode]}});
    (directory, store, input)
}

fn snapshot(input: &Value, saved_settings_sha256: &str) -> Value {
    json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.refine",
        "sourceVersion":input["source"]["version"],"input":input,
        "savedSettingsSha256":saved_settings_sha256,"guard":crate::workflow::guard_from_input(input)})
}

#[test]
fn refinement_preflight_settings_drift_is_rejected_before_creating_a_workflow() {
    for changed in [
        json!({"textModel":"changed-model"}),
        json!({"reasoning":"high"}),
    ] {
        let (_directory, store, mut input) = fixture();
        let original_settings = input["settings"].clone();
        let original_jobs = store.jobs().unwrap();
        let mut calls = 0;
        let error = freeze_production_execution_settings(
            &store,
            "storyboard.refine",
            &mut input,
            |settings| {
                calls += 1;
                assert_eq!(*settings, original_settings);
                store.save_settings(&changed)?;
                let mut observed = settings.clone();
                observed["textCliVersion"] = json!("codex-cli 0.160.0");
                Ok(observed)
            },
        )
        .unwrap_err();
        assert!(error.contains("精修预检期间变化"), "{error}");
        assert_eq!(calls, 1, "only a synthetic inspection occurred");
        assert_eq!(input["settings"], original_settings);
        assert_eq!(store.jobs().unwrap(), original_jobs);
        assert!(store.settings().unwrap() != original_settings);
    }
}

#[test]
fn refinement_reservation_freezes_original_settings_separately_from_observed_identity_after_reopen()
{
    let (_directory, store, mut input) = fixture();
    let original_settings = input["settings"].clone();
    let original_jobs = store.jobs().unwrap();
    let hash =
        freeze_production_execution_settings(&store, "storyboard.refine", &mut input, |settings| {
            let mut observed = settings.clone();
            observed["textCliVersion"] = json!("codex-cli 0.160.0");
            observed["textCliExecutable"] = json!("offline-observed-path-never-executed");
            Ok(observed)
        })
        .unwrap();
    assert_eq!(hash, pipeline::digest(&original_settings.to_string()));
    assert_ne!(hash, pipeline::digest(&input["settings"].to_string()));
    let frozen = snapshot(&input, &hash);
    let run = store
        .reserve_storyboard_refinement_workflow(&frozen)
        .unwrap();
    let reopened = Store::open(store.root.clone()).unwrap();
    let job = reopened.job(&run).unwrap();
    assert_eq!(job["status"], "queued");
    assert_eq!(job["entityId"], "storyboard.refine");
    assert_eq!(job["snapshot"], frozen);
    assert_eq!(reopened.settings().unwrap(), original_settings);
    assert_eq!(reopened.jobs().unwrap().len(), original_jobs.len() + 1);
    for old in original_jobs {
        assert_eq!(reopened.job(s(&old, "id")).unwrap(), old);
    }
    assert_eq!(
        reopened.get("units", "existing-manual-unit").unwrap()["prompt"],
        "用户原人工稿"
    );
}

#[test]
fn refinement_atomic_reservation_rechecks_settings_source_and_units_after_preflight() {
    for mutation in ["settings", "source", "unit", "guard"] {
        let (_directory, store, mut input) = fixture();
        let hash = freeze_production_execution_settings(
            &store,
            "storyboard.refine",
            &mut input,
            |settings| Ok(settings.clone()),
        )
        .unwrap();
        let mut frozen = snapshot(&input, &hash);
        match mutation {
            "settings" => store
                .save_settings(&json!({"textModel":"changed-after-preflight"}))
                .unwrap(),
            "source" => {
                let changed = pipeline::prepare_source("第1集\n场景：门口\n甲停步。\n").unwrap();
                store
                    .save_source(serde_json::to_value(changed).unwrap())
                    .unwrap();
            }
            "unit" => {
                let mut unit = store.get("units", "existing-manual-unit").unwrap();
                unit["prompt"] = json!("用户刚保存的新人工稿");
                store.save_unit(unit).unwrap();
            }
            "guard" => frozen["guard"]["units"] = json!({}),
            _ => unreachable!(),
        }
        let jobs = store.jobs().unwrap();
        let unit = store.get("units", "existing-manual-unit").unwrap();
        let current_source = store.source().unwrap();
        let current_settings = store.settings().unwrap();
        let error = store
            .reserve_storyboard_refinement_workflow(&frozen)
            .unwrap_err();
        assert!(!error.is_empty(), "{mutation}");
        assert_eq!(
            store.jobs().unwrap(),
            jobs,
            "{mutation}: no workflow or text attempt"
        );
        assert_eq!(store.get("units", "existing-manual-unit").unwrap(), unit);
        assert_eq!(store.source().unwrap(), current_source);
        assert_eq!(store.settings().unwrap(), current_settings);
    }
}

#[test]
fn inactive_standard_runtime_blocks_storyboard_and_refinement_resume_before_events_or_dispatch() {
    let (_directory, store, input) = fixture();
    let mut engine = Engine::with_channels(
        store.root.join("isolated-engine"),
        Arc::new(crate::execution_policy::OfflineChannels),
    )
    .unwrap();
    Arc::get_mut(&mut engine).unwrap().standard_native = true;
    for stage in ["storyboard.run", "storyboard.refine"] {
        let frozen = snapshot(&input, &pipeline::digest(&input["settings"].to_string()));
        let mut frozen = frozen;
        frozen["stage"] = json!(stage);
        let run = store
            .add_job("workflow", stage, "local", "offline", &frozen, "failed")
            .unwrap();
        let before = store.jobs().unwrap();
        let events: u64 = store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let error = engine
            .resume_production_checked(
                store.clone(),
                &json!({"action":"workflows.resume","workflowId":run}),
                false,
            )
            .unwrap_err();
        assert!(error.contains("STAGE3_ADAPTER_PENDING"), "{stage}: {error}");
        assert_eq!(store.jobs().unwrap(), before);
        assert_eq!(
            store
                .conn()
                .unwrap()
                .query_row("SELECT count(*) FROM events", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            events
        );
        assert!(
            engine.active.lock().unwrap().is_empty(),
            "no worker acquired ownership"
        );
        check_standard_workflow_resume(true, &store.job(&run).unwrap()).unwrap();
    }
    for unrelated in ["foundation.run", "images.design", "images.produce"] {
        check_standard_workflow_resume(false, &json!({"entityId":unrelated})).unwrap();
    }
}
