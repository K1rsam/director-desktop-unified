//! Reopened metadata-only review versions retain creative source provenance.
use super::*;

#[tokio::test]
async fn refinement_continuity_review_reopens_and_remains_a_valid_basis_for_the_next_refinement() {
    let (_dir, store, input, run, candidate) =
        crate::workflow::refinement_continuity::tests::fixture().await;
    let plan = store
        .preview_refinement_continuity(&run, s(&candidate, "candidateSha256"))
        .unwrap();
    let review = crate::workflow::refinement_continuity::tests::review(&plan);
    let original = store.job(&run).unwrap()["prepared"]["refinementAdoption"].clone();
    store
        .adopt_refinement_continuity(&run, s(&candidate, "candidateSha256"), &review)
        .unwrap();
    let reopened = Store::open(store.root.clone()).unwrap();
    let mut current = input.clone();
    current["existingUnits"] = json!(reopened.values("units").unwrap());
    let ids: Vec<_> = current["existingUnits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["id"].clone())
        .collect();
    let contexts = accepted_storyboard_revision_contexts(&reopened, &current, &ids).unwrap();
    assert_eq!(contexts[0]["unitBindings"].as_array().unwrap().len(), 3);
    let next = refinement_input_with_capabilities(
        &reopened,
        &json!({"episodeId":input["episodeId"],"reviewNotes":"下一轮仅审真实声画缺口"}),
        Some(&input["capabilities"]),
    )
    .unwrap();
    assert_eq!(next["existingUnits"], current["existingUnits"]);
    assert_eq!(
        reopened.job(&run).unwrap()["prepared"]["refinementAdoption"],
        original
    );
    let saved_prepared = reopened.job(&run).unwrap()["prepared"].clone();
    for missing in [false, true] {
        let mut prepared = saved_prepared.clone();
        if missing {
            prepared
                .as_object_mut()
                .unwrap()
                .remove("refinementContinuityReview");
        } else {
            prepared["refinementContinuityReview"]["reviewSha256"] = json!("tampered");
        }
        reopened
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET prepared=?1 WHERE id=?2",
                rusqlite::params![prepared.to_string(), run],
            )
            .unwrap();
        assert!(
            accepted_storyboard_revision_contexts(&reopened, &current, &ids).is_err(),
            "missing={missing}: metadata proof cannot be skipped by a creative-body match"
        );
        assert!(
            refinement_input_with_capabilities(
                &reopened,
                &json!({"episodeId":input["episodeId"],"reviewNotes":"不得跳过缺失回执"}),
                Some(&input["capabilities"])
            )
            .is_err(),
            "missing={missing}"
        );
    }
}

fn captured_domain_digest(connection: &rusqlite::Connection) -> String {
    let mut query = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let tables = query
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    drop(query);
    let mut data = Vec::new();
    for table in tables {
        let mut query = connection
            .prepare(&format!(
                "SELECT * FROM \"{}\" ORDER BY rowid",
                table.replace('"', "\"\"")
            ))
            .unwrap();
        let columns = query.column_count();
        let rows = query
            .query_map([], |row| {
                (0..columns)
                    .map(|column| Ok(format!("{:?}", row.get_ref(column)?)))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        data.push((table, rows));
    }
    pipeline::digest(&serde_json::to_string(&data).unwrap())
}

#[test]
#[ignore = "explicit isolated post-review capture; reads next refinement inputs, never dispatches or opens production DB"]
fn isolated_captured_next_refinement_inputs_keep_all_adopted_provenance_without_generation() {
    let root = PathBuf::from(
        std::env::var("DIRECTOR_REFINEMENT_CAPTURED_INPUT_ROOT")
            .expect("isolated post-review captured project root"),
    );
    let marker: Value = serde_json::from_slice(
        &fs::read(root.join("isolated-next-refinement-input.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(marker["purpose"], "isolated-next-refinement-input/v1");
    assert_eq!(
        fs::canonicalize(&root).unwrap(),
        fs::canonicalize(s(&marker, "capturedRoot")).unwrap()
    );
    assert_ne!(
        fs::canonicalize(&root).unwrap(),
        fs::canonicalize(s(&marker, "sourceRoot")).unwrap()
    );
    let database = root.join("project.sqlite");
    assert!(database.is_file());
    let before_file = crate::store::hash_file(&database).unwrap();
    // Construct only a handle: Store::open migrations, Engine startup recovery,
    // account checks, schedulers and all model/provider calls are excluded.
    let store = Store { root };
    let before = captured_domain_digest(&store.conn().unwrap());
    let jobs = store.jobs().unwrap().len();
    let units = store.values("units").unwrap();
    assert_eq!(units.len(), 36);
    assert!(units.iter().all(|unit| unit["continuityStale"] != true));
    let run = s(&marker, "workflowId");
    let saved = store.job(run).unwrap();
    assert_eq!(
        saved["result"]["candidateSha256"],
        marker["candidateSha256"]
    );
    assert_eq!(
        saved["snapshot"]["input"]["source"]["sha256"],
        marker["sourceSha256"]
    );
    assert_eq!(
        saved["prepared"]["refinementContinuityReview"]["afterUnitBindings"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let source = store.source().unwrap();
    assert_eq!(source["sha256"], marker["sourceSha256"]);
    let episodes = source["episodes"].as_array().unwrap();
    assert!(episodes.len() >= 10);
    let mut results = Vec::new();
    for index in [7usize, 8, 9] {
        let episode = &episodes[index]["id"];
        let input = refinement_input_with_capabilities(
            &store,
            &json!({"episodeId":episode,"reviewNotes":"隔离验收下一轮取料；不执行生成"}),
            saved["snapshot"]["input"].get("capabilities"),
        )
        .unwrap();
        assert_eq!(input["existingUnits"], json!(units));
        let selected: Vec<_> = units
            .iter()
            .filter(|unit| unit["episodeId"] == *episode)
            .map(|unit| unit["id"].clone())
            .collect();
        let contexts = accepted_storyboard_revision_contexts(&store, &input, &selected).unwrap();
        assert_eq!(input["revisionContexts"], json!(contexts));
        assert!(!contexts.is_empty());
        assert!(contexts
            .iter()
            .all(|context| context["episodeId"] == *episode
                && context["sourceSha256"] == marker["sourceSha256"]));
        let bindings: Vec<_> = contexts
            .iter()
            .flat_map(|context| context["unitBindings"].as_array().unwrap())
            .collect();
        assert_eq!(bindings.len(), selected.len());
        for unit in units.iter().filter(|unit| unit["episodeId"] == *episode) {
            assert_eq!(
                bindings
                    .iter()
                    .filter(|binding| binding["unitId"] == unit["id"]
                        && binding["unitVersion"] == unit["version"]
                        && binding["unitSha256"] == pipeline::digest(&unit.to_string()))
                    .count(),
                1
            );
        }
        results.push(json!({"episode":index+1,"unitCount":selected.len(),"revisionContexts":contexts.len(),"identityRebinding":input.get(pipeline::storyboard_refinement_identity::FIELD).is_some()}));
    }
    assert_eq!(store.job(run).unwrap(), saved);
    assert_eq!(store.values("units").unwrap(), units);
    assert_eq!(store.jobs().unwrap().len(), jobs);
    assert_eq!(captured_domain_digest(&store.conn().unwrap()), before);
    assert_eq!(crate::store::hash_file(&database).unwrap(), before_file);
    println!(
        "{}",
        json!({"capturedNextRefinementInputs":results,"units":36,"domainTablesUnchanged":true,"databaseFileUnchanged":true,"modelCalls":0,"newJobs":0})
    );
}

#[tokio::test]
async fn refinement_continuity_preview_dispatch_is_read_only_and_offline_adoption_stays_blocked() {
    let (dir, store, _input, run, candidate) =
        crate::workflow::refinement_continuity::tests::fixture().await;
    let engine = Engine::new_offline_verification(dir.path().join("offline-engine-home")).unwrap();
    engine.register(&store).unwrap();
    let project = store.project().unwrap();
    let before = store.job(&run).unwrap();
    let units = store.values("units").unwrap();
    let jobs = store.jobs().unwrap().len();
    let counts = || {
        let connection = store.conn().unwrap();
        (
            connection
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            connection
                .query_row("SELECT COUNT(*) FROM unit_versions", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
        )
    };
    let original_counts = counts();
    let request = json!({"action":"storyboard.refinement.continuity.preview","projectId":project["id"],"workflowId":run,"candidateSha256":candidate["candidateSha256"]});
    let workspace = engine.dispatch(request).unwrap();
    let plan = &workspace["refinementContinuityReviewPreview"];
    assert_eq!(
        *plan,
        store
            .preview_refinement_continuity(&run, s(&candidate, "candidateSha256"))
            .unwrap()
    );
    assert_eq!(plan["affectedUnitBindings"].as_array().unwrap().len(), 2);
    let policy = engine
        .dispatch(json!({"action":"getExecutionPolicy"}))
        .unwrap();
    assert!(policy["allowedActions"]
        .as_array()
        .unwrap()
        .contains(&json!("storyboard.refinement.continuity.preview")));
    assert!(!policy["allowedActions"]
        .as_array()
        .unwrap()
        .contains(&json!("storyboard.refinement.adopt")));
    let error = engine.dispatch(json!({"action":"storyboard.refinement.adopt","projectId":project["id"],"workflowId":run,"candidateSha256":candidate["candidateSha256"],"continuityReview":crate::workflow::refinement_continuity::tests::review(plan)})).unwrap_err();
    assert!(error.contains("OFFLINE_EXECUTION_BLOCKED"), "{error}");
    assert_eq!(store.job(&run).unwrap(), before);
    assert_eq!(store.values("units").unwrap(), units);
    assert_eq!(store.jobs().unwrap().len(), jobs);
    assert_eq!(counts(), original_counts);
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[tokio::test]
async fn refinement_continuity_standard_dispatch_adopts_the_explicit_review_without_starting_a_scheduler(
) {
    let (dir, store, _input, run, candidate) =
        crate::workflow::refinement_continuity::tests::fixture().await;
    let home = dir.path().join("standard-engine-home");
    let engine = Engine::new_standard(home.clone()).unwrap();
    assert!(engine.standard_native);
    assert!(!engine.offline_only);
    assert!(engine.active.lock().unwrap().is_empty());
    engine.register(&store).unwrap();
    let project = store.project().unwrap();
    let original = store.job(&run).unwrap();
    let source = store.source().unwrap();
    let assets = store.values("assets").unwrap();
    let settings = store.settings().unwrap();
    let jobs = store.jobs().unwrap().len();
    let policy = engine
        .dispatch(json!({"action":"getExecutionPolicy"}))
        .unwrap();
    for action in [
        "storyboard.refinement.continuity.preview",
        "storyboard.refinement.adopt",
    ] {
        assert!(
            policy["allowedActions"]
                .as_array()
                .unwrap()
                .contains(&json!(action)),
            "{action}"
        );
    }
    let preview = engine.dispatch(json!({"action":"storyboard.refinement.continuity.preview","projectId":project["id"],"workflowId":run,"candidateSha256":candidate["candidateSha256"]})).unwrap();
    let plan = &preview["refinementContinuityReviewPreview"];
    assert_eq!(store.job(&run).unwrap(), original);
    let request = json!({"action":"storyboard.refinement.adopt","projectId":project["id"],"workflowId":run,"candidateSha256":candidate["candidateSha256"],"continuityReview":crate::workflow::refinement_continuity::tests::review(plan)});
    engine.dispatch(request.clone()).unwrap();
    let accepted = store.job(&run).unwrap();
    assert_eq!(accepted["snapshot"], original["snapshot"]);
    assert_eq!(accepted["result"], original["result"]);
    assert_eq!(
        accepted["prepared"]["refinementAdoption"],
        original["prepared"]["refinementAdoption"]
    );
    assert_eq!(
        accepted["prepared"]["refinementContinuityReview"]["modelCalls"],
        0
    );
    let units = store.values("units").unwrap();
    assert!(units.iter().all(|unit| unit["continuityStale"] != true));
    engine.dispatch(request).unwrap();
    assert_eq!(store.job(&run).unwrap(), accepted);
    assert_eq!(store.values("units").unwrap(), units);
    assert_eq!(store.source().unwrap(), source);
    assert_eq!(store.values("assets").unwrap(), assets);
    assert_eq!(store.settings().unwrap(), settings);
    assert_eq!(store.jobs().unwrap().len(), jobs);
    assert!(engine.active.lock().unwrap().is_empty());
    drop(engine);
    let reopened = Engine::new_standard(home).unwrap();
    let workspace = reopened
        .dispatch(json!({"action":"workspace.get","projectId":project["id"]}))
        .unwrap();
    assert!(workspace["units"]
        .as_array()
        .unwrap()
        .iter()
        .all(|unit| unit["continuityStale"] != true));
    assert_eq!(
        reopened
            .store(s(&project, "id"))
            .unwrap()
            .job(&run)
            .unwrap(),
        accepted
    );
    assert_eq!(store.jobs().unwrap().len(), jobs);
}
