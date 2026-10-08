//! Offline publication regressions for source episode boundaries and saved edges.
use director_core::{pipeline, store::Store, workflow::guard_from_input};
use rusqlite::params;
use serde_json::{json, Value};

fn fixture() -> (tempfile::TempDir, Store, Value) {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::open(temporary.path().join("isolated-project")).unwrap();
    store
        .create("合成发布依赖验收", "synthetic-dependency-project")
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let source = pipeline::prepare_source(
        "第一集\n场景：门口\n△甲抬手。\n第二集\n场景：门口\n△乙点头。\n第三集\n场景：门口\n△丙转身。\n第四集\n场景：门口\n△丁坐下。\n第五集\n场景：门口\n△戊起身。\n",
    ).unwrap();
    let source = store.save_source(json!(source)).unwrap();
    assert_eq!(source["episodes"].as_array().unwrap().len(), 5);
    (temporary, store, source)
}

fn episode(source: &Value, index: usize) -> String {
    source["episodes"][index]["id"].as_str().unwrap().to_owned()
}

fn unit(source: &Value, index: usize, id: &str) -> Value {
    let episode = episode(source, index);
    let row = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["episodeId"] == episode && row["kind"] == "body")
        .unwrap();
    json!({"id":id,"episodeId":episode,"sceneId":row["sceneId"],"title":"合成动作",
        "sourceRefs":[row["id"]],"assetRefs":[],"exitState":"原动作完成",
        "boundaryBefore":{"kind":"episode_start","reason":"本集源开始"},
        "shots":[{"id":format!("{id}:shot"),"duration":3,"sourceRefs":[row["id"]],"dialogueParts":[]}]})
}

fn input(store: &Store, source: &Value, selected: &[usize], previous: Value) -> Value {
    json!({"source":source,"foundation":{"version":0},"assets":[],
        "existingUnits":store.values("units").unwrap(),"previousBoundary":previous,
        "selection":{"episodeIds":selected.iter().map(|index|episode(source,*index)).collect::<Vec<_>>()},
        "capabilities":{}})
}

fn publish(store: &Store, input: &Value, units: &[Value]) {
    let output = json!({"scope":"episodes","updatedEpisodeIds":input["selection"]["episodeIds"],
        "units":units,"sourceSha256":input["source"]["sha256"],"continuityImpacts":[]});
    store
        .publish_storyboard(&guard_from_input(input), &output, input, None)
        .unwrap();
}

fn dependency(store: &Store, child: &str, parent: &str) {
    store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO unit_dependencies VALUES(?1,?2)",
            params![child, parent],
        )
        .unwrap();
}

fn parents(store: &Store, child: &str) -> Vec<String> {
    let connection = store.conn().unwrap();
    let mut query = connection
        .prepare(
            "SELECT dependency_id FROM unit_dependencies WHERE unit_id=?1 ORDER BY dependency_id",
        )
        .unwrap();
    query
        .query_map([child], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn nonadjacent_final_episode_publication_keeps_incremental_dependency_break() {
    let (_temporary, store, source) = fixture();
    let second = vec![unit(&source, 1, "e2-head"), unit(&source, 1, "e2-tail")];
    let fourth = vec![unit(&source, 3, "e4-head"), unit(&source, 3, "e4-tail")];
    publish(&store, &input(&store, &source, &[1], Value::Null), &second);
    publish(&store, &input(&store, &source, &[3], Value::Null), &fourth);
    assert!(parents(&store, "e4-head").is_empty());
    let all = second.into_iter().chain(fourth).collect::<Vec<_>>();
    publish(&store, &input(&store, &source, &[1, 3], Value::Null), &all);
    assert_eq!(parents(&store, "e2-tail"), vec!["e2-head"]);
    assert_eq!(parents(&store, "e4-tail"), vec!["e4-head"]);
    assert!(
        parents(&store, "e4-head").is_empty(),
        "final workflow publication must not undo the incremental episode gap"
    );
}

#[test]
fn adjacent_episodes_and_same_episode_units_keep_true_predecessor_links() {
    let (_temporary, store, source) = fixture();
    let previous = store.save_unit(unit(&source, 0, "e1-tail")).unwrap();
    let units = vec![
        unit(&source, 1, "e2-head"),
        unit(&source, 1, "e2-tail"),
        unit(&source, 2, "e3-head"),
    ];
    publish(&store, &input(&store, &source, &[1, 2], previous), &units);
    assert_eq!(parents(&store, "e2-head"), vec!["e1-tail"]);
    assert_eq!(parents(&store, "e2-tail"), vec!["e2-head"]);
    assert_eq!(parents(&store, "e3-head"), vec!["e2-tail"]);
}

#[test]
fn nonadjacent_previous_boundary_never_becomes_dependency() {
    let (_temporary, store, source) = fixture();
    let previous = store.save_unit(unit(&source, 0, "e1-tail")).unwrap();
    publish(
        &store,
        &input(&store, &source, &[3], previous),
        &[unit(&source, 3, "e4-head"), unit(&source, 3, "e4-tail")],
    );
    assert!(parents(&store, "e4-head").is_empty());
    assert_eq!(parents(&store, "e4-tail"), vec!["e4-head"]);
}

#[test]
fn nonadjacent_episode_replacement_rewires_external_successors_to_own_tail() {
    let (_temporary, store, source) = fixture();
    for (index, id) in [(1, "old-e2"), (2, "e3-head"), (3, "old-e4"), (4, "e5-head")] {
        store.save_unit(unit(&source, index, id)).unwrap();
    }
    dependency(&store, "e3-head", "old-e2");
    dependency(&store, "e5-head", "old-e4");
    publish(
        &store,
        &input(&store, &source, &[1, 3], Value::Null),
        &[
            unit(&source, 1, "new-e2-tail"),
            unit(&source, 3, "new-e4-tail"),
        ],
    );
    assert!(parents(&store, "new-e4-tail").is_empty());
    assert_eq!(parents(&store, "e3-head"), vec!["new-e2-tail"]);
    assert_eq!(parents(&store, "e5-head"), vec!["new-e4-tail"]);
    assert_eq!(store.get("units", "old-e2").unwrap()["retired"], true);
    assert_eq!(store.get("units", "old-e4").unwrap()["retired"], true);
}

#[test]
fn local_unit_replacement_preserves_ingress_and_external_successor() {
    let (_temporary, store, source) = fixture();
    for id in ["head", "old-middle", "tail"] {
        store.save_unit(unit(&source, 1, id)).unwrap();
    }
    dependency(&store, "old-middle", "head");
    dependency(&store, "tail", "old-middle");
    let mut input = input(&store, &source, &[1], Value::Null);
    input["selection"]["unitIds"] = json!(["old-middle"]);
    let output = json!({"scope":"units","replacedUnitIds":["old-middle"],
        "units":[unit(&source,1,"new-middle-one"),unit(&source,1,"new-middle-two")],"continuityImpacts":[]});
    store
        .publish_storyboard(&guard_from_input(&input), &output, &input, None)
        .unwrap();
    assert_eq!(parents(&store, "new-middle-one"), vec!["head"]);
    assert_eq!(parents(&store, "new-middle-two"), vec!["new-middle-one"]);
    assert_eq!(parents(&store, "tail"), vec!["new-middle-two"]);
    assert!(parents(&store, "head").is_empty());
}
