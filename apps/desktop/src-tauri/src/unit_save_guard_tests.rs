//! Synthetic dispatch regression: no model, browser, account, or production home.
use super::super::*;
use rusqlite::types::Value as SqlValue;
use std::collections::BTreeMap;

struct Fixture {
    _temp: tempfile::TempDir,
    engine: Arc<Engine>,
    store: Store,
    project: String,
    unit: Value,
    raw_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::build_with_recovery(
            temp.path().join("home"),
            Arc::new(crate::execution_policy::OfflineChannels),
            false, true, false,
        ).unwrap();
        let project = engine.dispatch(json!({"action":"projects.create","title":"unit save guard synthetic"})).unwrap();
        let project = s(&project, "id").to_owned();
        let store = engine.store(&project).unwrap();
        let source = store.save_source(json!(pipeline::prepare_source("第一集\n场景：门口\n甲：开门。\n").unwrap())).unwrap();
        store.save_asset(json!({"id":"asset-a","variantId":"base","name":"甲","kind":"CH"})).unwrap();
        let unit = store.save_unit(json!({
            "id":"unit-a","episodeId":source["episodes"][0]["id"],"title":"原标题","duration":6.0,
            "prompt":"ORIGINAL PROMPT","channelPromptTemplate":"ORIGINAL TEMPLATE",
            "assetRefs":[],"sourceRefs":[source["rows"][2]["id"]],
            "shots":[
                {"id":"shot-a","duration":3.0,"camera":"原机位","dialogueParts":[{"text":"开门。","speaker":"甲","presentation":"画内","voiceStart":0.1,"voiceEnd":2.0}]},
                {"id":"shot-b","duration":3.0,"camera":"反应镜头","dialogueParts":[]}
            ]
        })).unwrap();
        let unknown = store.add_job("text","unit-a","sdk","synthetic",&json!({"frozen":"source and original request"}),"submission_unknown").unwrap();
        store.update_job(&unknown,"submission_unknown",&json!({"prepared":{"original":"prepared"},"receipt":{"original":"receipt"},"result":{"raw":"ORIGINAL RAW RESULT"}})).unwrap();
        store.event(&unknown,"original_diagnostic",&json!({"raw":"preserve me"})).unwrap();
        // This queued row proves rejection cannot stale a dependent video task.
        // OfflineChannels cannot prepare or submit it.
        store.add_job("video","unit-a","synthetic","synthetic",&json!({"prompt":"frozen prompt"}),"queued").unwrap();
        let raw_path = temp.path().join("original-response.txt");
        fs::write(&raw_path,b"ORIGINAL RAW RESPONSE\r\n").unwrap();
        Self { _temp:temp, engine, store, project, unit, raw_path }
    }

    fn save(&self, unit: Value) -> Result<Value> {
        self.engine.dispatch(json!({"action":"units.save","projectId":self.project,"unit":unit}))
    }

    fn raw_tables(&self) -> BTreeMap<String, Vec<Vec<SqlValue>>> {
        let conn = self.store.conn().unwrap();
        let names:Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").unwrap()
            .query_map([],|row|row.get(0)).unwrap().map(|row|row.unwrap()).collect();
        names.into_iter().map(|name| {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{}\" ORDER BY rowid",name.replace('"',"\"\""))).unwrap();
            let rows = statement.query_map([],|row| (0..row.as_ref().column_count()).map(|col|row.get(col)).collect::<rusqlite::Result<Vec<SqlValue>>>()).unwrap()
                .map(|row|row.unwrap()).collect();
            (name,rows)
        }).collect()
    }

    fn assert_rejected_unchanged(&self, edit: Value) {
        let tables = self.raw_tables();
        let raw = fs::read(&self.raw_path).unwrap();
        let error = self.save(edit).unwrap_err();
        assert!(error.starts_with("UNIT_SHOTS_EDIT_REQUIRES_REFINEMENT:"),"{error}");
        assert_eq!(self.store.get("units","unit-a").unwrap(),self.unit);
        assert_eq!(self.raw_tables(),tables,"all stored versions, prompts, templates, jobs, raw results, events and metadata must remain exact");
        assert_eq!(fs::read(&self.raw_path).unwrap(),raw);
    }
}

#[test]
fn structured_shot_changes_reject_before_any_persistence() {
    let f = Fixture::new();
    for field in ["camera","dialogue","presentation","voice-window","duration","order","empty","null","object"] {
        let mut edit = f.unit.clone();
        edit["expectedVersion"] = f.unit["version"].clone();
        match field {
            "camera" => edit["shots"][0]["camera"] = json!("新机位"),
            "dialogue" => edit["shots"][0]["dialogueParts"][0]["text"] = json!("另一句话。"),
            "presentation" => edit["shots"][0]["dialogueParts"][0]["presentation"] = json!("画外转画内"),
            "voice-window" => edit["shots"][0]["dialogueParts"][0]["voiceStart"] = json!(1.0),
            "duration" => edit["shots"][0]["duration"] = json!(4.0),
            "order" => edit["shots"].as_array_mut().unwrap().reverse(),
            "empty" => edit["shots"] = json!([]),
            "null" => edit["shots"] = Value::Null,
            _ => edit["shots"] = json!({"camera":"wrong shape"}),
        }
        f.assert_rejected_unchanged(edit);
    }
}

#[test]
fn replacement_prompt_or_override_cannot_bypass_structured_guard() {
    let f = Fixture::new();
    for replacements in [json!({"prompt":"NEW"}),json!({"promptOverride":"NEW"}),json!({"prompt":"NEW","promptOverride":"NEW","channelPromptTemplate":"NEW"})] {
        let mut edit = merge(f.unit.clone(), &replacements);
        edit["shots"][0]["camera"] = json!("新机位");
        f.assert_rejected_unchanged(edit);
    }
}

#[test]
fn same_shots_roundtrip_is_noop_and_omitted_shots_allow_title_and_prompt_edits() {
    let f = Fixture::new();
    let tables = f.raw_tables();
    let mut same = f.unit.clone();
    same["expectedVersion"] = f.unit["version"].clone();
    f.save(same).unwrap();
    assert_eq!(f.raw_tables(),tables);
    f.save(json!({"id":"unit-a","expectedVersion":1,"title":"新标题","prompt":"MANUAL TEXT","promptOverride":"MANUAL TEXT"})).unwrap();
    let saved = f.store.get("units","unit-a").unwrap();
    assert_eq!(saved["version"],2);
    assert_eq!(saved["title"],"新标题");
    assert_eq!(saved["prompt"],"MANUAL TEXT");
    assert_eq!(saved["promptOverride"],"MANUAL TEXT");
    assert_eq!(saved["shots"],f.unit["shots"]);
    assert_eq!(saved["channelPromptTemplate"],f.unit["channelPromptTemplate"]);
}

#[test]
fn unchanged_shots_allow_material_binding() {
    let f = Fixture::new();
    let mut edit = f.unit.clone();
    edit["expectedVersion"] = json!(1);
    edit["assetRefs"] = json!([{"assetId":"asset-a","variantId":"base"}]);
    f.save(edit).unwrap();
    let saved = f.store.get("units","unit-a").unwrap();
    assert_eq!(saved["version"],2);
    assert_eq!(saved["assetRefs"][0]["assetId"],"asset-a");
    for field in ["shots","prompt","channelPromptTemplate"] { assert_eq!(saved[field],f.unit[field]); }
}

#[test]
fn javascript_numeric_roundtrip_preserves_stored_shots_without_false_version() {
    let f = Fixture::new();
    let tables = f.raw_tables();
    let mut edit = f.unit.clone();
    edit["shots"][0]["duration"] = json!(3);
    edit["shots"][0]["dialogueParts"][0]["voiceEnd"] = json!(2);
    edit["shots"][1]["duration"] = json!(3);
    f.save(edit.clone()).unwrap();
    assert_eq!(f.raw_tables(),tables);
    edit["title"] = json!("JS 编辑标题");
    f.save(edit).unwrap();
    let saved = f.store.get("units","unit-a").unwrap();
    assert_eq!(saved["title"],"JS 编辑标题");
    assert_eq!(saved["shots"],f.unit["shots"]);
    assert_eq!(saved["version"],2);
    assert!(!super::same_shot_value(&json!(9007199254740993_u64),&json!(9007199254740992.0)));
    assert!(!super::same_shot_value(&json!(u64::MAX),&json!(u64::MAX as f64)));
    assert!(!super::same_shot_value(&json!(i64::MAX),&json!(i64::MAX as f64)));
    assert!(!super::same_shot_value(&json!(3),&json!(3.00001)));
    assert!(!super::same_shot_value(&json!(-1),&json!(0.0)));
    assert!(super::same_shot_value(&json!(-1),&json!(-1.0)));
    assert!(super::same_shot_value(&json!(0),&json!(-0.0)));
}

#[test]
fn legacy_units_without_shots_keep_text_saves_but_cannot_add_shots() {
    let f = Fixture::new();
    f.store.save_unit(json!({"id":"legacy","episodeId":f.unit["episodeId"],"prompt":"legacy","assetRefs":[]})).unwrap();
    f.save(json!({"id":"legacy","expectedVersion":1,"title":"legacy edited"})).unwrap();
    let tables = f.raw_tables();
    for shots in [Value::Null,json!([]),f.unit["shots"].clone()] {
        let error = f.save(json!({"id":"legacy","shots":shots})).unwrap_err();
        assert!(error.starts_with("UNIT_SHOTS_EDIT_REQUIRES_REFINEMENT:"));
        assert_eq!(f.raw_tables(),tables);
    }
}

#[test]
fn new_units_and_direct_production_store_adoption_remain_supported() {
    let f = Fixture::new();
    let mut new = f.unit.clone();
    new["id"] = json!("new-unit");
    new["shots"][0]["camera"] = json!("新单元镜头");
    f.save(new.clone()).unwrap();
    let saved = f.store.get("units","new-unit").unwrap();
    assert_eq!(saved["shots"],new["shots"]);
    assert_eq!(saved["version"],1);
    // The trusted refinement/adoption path retains its lower-level save API.
    let mut adopted = f.unit.clone();
    adopted["shots"][0]["camera"] = json!("已精修的镜头");
    adopted["prompt"] = json!("REFINED PROMPT");
    adopted["channelPromptTemplate"] = json!("REFINED TEMPLATE");
    let saved = f.store.save_unit(adopted.clone()).unwrap();
    assert_eq!(saved["version"],2);
    for field in ["shots","prompt","channelPromptTemplate"] { assert_eq!(saved[field],adopted[field]); }
}
