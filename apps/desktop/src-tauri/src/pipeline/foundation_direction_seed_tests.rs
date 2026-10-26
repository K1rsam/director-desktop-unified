use super::*;
use crate::store::Store;
use std::sync::Mutex;

const DIRECTION: &str = "自然、克制；保留原句与说者。";

fn legacy_seed() -> Value {
    json!({"schemaVersion":"foundation/v1","episodes":[],"assets":[],"facts":[],"issues":[],
        "overview":"","direction":"","productionDirection":DIRECTION,"directionConfirmed":true,
        "sourceVersion":1,"version":1})
}

fn project() -> (tempfile::TempDir, Store, Value) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().to_owned()).unwrap();
    store.create("方向种子合成回归", "test").unwrap();
    let source =
        prepare_source("第一集\n场景：门口\n△小王穿白衣站在门口。\n小王：来了。\n").unwrap();
    let source = store.save_source(json!(source)).unwrap();
    (directory, store, source)
}

fn input(store: &Store, source: &Value) -> Value {
    json!({"source":source,"sourceVersion":source["version"],"foundation":store.foundation().unwrap(),
        "assets":[],"existingUnits":[],"selection":{"episodeIds":[source["episodes"][0]["id"]]},
        "settings":{"confirmedDirection":store.foundation().unwrap()["productionDirection"]}})
}

#[derive(Default)]
struct SyntheticFoundation {
    calls: Mutex<Vec<TextRequest>>,
    accepted: Mutex<HashMap<String, Value>>,
}

#[async_trait]
impl TextExecutor for SyntheticFoundation {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let key = digest(&serde_json::to_string(&request).unwrap());
        if let Some(value) = self.accepted.lock().unwrap().get(&key) {
            return Ok(value.clone());
        }
        assert_eq!(
            request.input["projectConditions"]["confirmedDirection"],
            DIRECTION
        );
        self.calls.lock().unwrap().push(request.clone());
        Ok(match request.stage.as_str() {
            "foundation:F2" => {
                let row = request.input["source"]["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r[9].as_str().unwrap().contains("白衣"))
                    .unwrap();
                json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":request.input["sourceSha256"],
                    "coverage":request.input["scope"],"assets":[{"name":"小王｜白衣","type":"CH",
                    "episodes":[row[2]],"sceneRefs":[[row[2],row[3]]],"sourceRefs":[row[0]],
                    "role":"主角","preparation":{"mode":"REFERENCE"}}]})
            }
            "foundation:Q1" => json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2",
                "sourceSha256":request.input["sourceSha256"],"coverage":request.input["scope"],"changes":{}}),
            stage => panic!("unexpected synthetic stage {stage}"),
        })
    }

    async fn accept(&self, request: &TextRequest, raw: &Value) -> Result<(), String> {
        self.accepted.lock().unwrap().insert(
            digest(&serde_json::to_string(request).unwrap()),
            raw.clone(),
        );
        Ok(())
    }
}

#[tokio::test]
async fn first_foundation_after_confirmed_direction_uses_locked_source_and_retains_direction() {
    let (_directory, store, source) = project();
    store
        .save_production_direction(DIRECTION, 1, Some(0))
        .unwrap();
    let frozen = input(&store, &source);
    let locked: LockedSource = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(frozen["foundation"]["sourceSha256"], source["sha256"]);
    assert!(
        !legacy_direction_seed(&locked, &frozen),
        "new saves already have a complete source contract"
    );
    existing_foundation(&locked, &frozen).unwrap();
    let executor = SyntheticFoundation::default();
    let result = foundation_delivery::run_foundation_delivery(
        &locked,
        &frozen,
        &executor,
        "synthetic",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(result.source_sha256, locked.sha256);
    assert_eq!(result.direction, DIRECTION);
    assert_eq!(executor.calls.lock().unwrap().len(), 2, "exact F2 then Q1");
    let guard = crate::workflow::guard_from_input(&frozen);
    store
        .publish_foundation(&guard, json!(result), vec![], None)
        .unwrap();
    let reopened = Store::open(store.root.clone()).unwrap();
    let saved = reopened.foundation().unwrap();
    assert_eq!(saved["productionDirection"], DIRECTION);
    assert_eq!(saved["directionConfirmed"], true);
    assert_eq!(saved["sourceSha256"], source["sha256"]);
}

#[test]
fn legacy_direction_seed_never_rebinds_completed_foreign_or_unproven_foundation() {
    let (_directory, _store, raw_source) = project();
    let locked: LockedSource = serde_json::from_value(raw_source.clone()).unwrap();
    let context = json!({"source":raw_source,"sourceVersion":1,"foundation":legacy_seed()});
    let frozen_sha = digest(&context.to_string());
    assert_eq!(
        existing_foundation(&locked, &context)
            .unwrap()
            .source_sha256,
        locked.sha256
    );
    assert_eq!(
        digest(&context.to_string()),
        frozen_sha,
        "projection leaves frozen seed byte identity intact"
    );
    for fault in [
        "source-version",
        "no-version",
        "source-identity",
        "foundation-version",
        "unconfirmed",
        "wrong-sha",
        "empty-sha",
        "null-sha",
        "episode",
        "asset",
        "fact",
        "issue",
        "overview",
        "direction",
        "receipt",
        "unknown",
    ] {
        let mut changed = context.clone();
        match fault {
            "source-version" => changed["sourceVersion"] = json!(2),
            "no-version" => {
                changed.as_object_mut().unwrap().remove("sourceVersion");
                changed["source"].as_object_mut().unwrap().remove("version");
            }
            "source-identity" => changed["source"]["sha256"] = json!("0".repeat(64)),
            "foundation-version" => changed["foundation"]["sourceVersion"] = json!(2),
            "unconfirmed" => changed["foundation"]["directionConfirmed"] = json!(false),
            "wrong-sha" => changed["foundation"]["sourceSha256"] = json!("0".repeat(64)),
            "empty-sha" => changed["foundation"]["sourceSha256"] = json!(""),
            "null-sha" => changed["foundation"]["sourceSha256"] = Value::Null,
            "episode" => changed["foundation"]["episodes"] = json!([{}]),
            "asset" => changed["foundation"]["assets"] = json!([{}]),
            "fact" => changed["foundation"]["facts"] = json!([{}]),
            "issue" => changed["foundation"]["issues"] = json!([{}]),
            "overview" => changed["foundation"]["overview"] = json!("已写出的基础内容"),
            "direction" => changed["foundation"]["direction"] = json!("已有创作结果"),
            "receipt" => changed["foundation"]["adoptionReceipt"] = json!({"status":"accepted"}),
            _ => changed["foundation"]["unknownPrivateData"] = json!({"keep":true}),
        }
        assert!(existing_foundation(&locked, &changed).is_err(), "{fault}");
    }
    let mut valid = context;
    valid["foundation"]["sourceSha256"] = json!(locked.sha256);
    valid["foundation"]["overview"] = json!("合法旧基础不可作为空种子丢掉");
    assert_eq!(
        existing_foundation(&locked, &valid).unwrap().overview,
        "合法旧基础不可作为空种子丢掉"
    );
    valid["foundation"]
        .as_object_mut()
        .unwrap()
        .remove("sourceSha256");
    assert!(existing_foundation(&locked, &valid).is_err());
}

#[tokio::test]
async fn frozen_direction_seed_failed_workflow_reopens_without_rewriting_snapshot_or_rebuying_steps(
) {
    let (_directory, store, source) = project();
    store.save_foundation(legacy_seed()).unwrap();
    let frozen = input(&store, &source);
    let guard = crate::workflow::guard_from_input(&frozen);
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run",
        "foundationMethod":foundation_delivery::METHOD_VERSION,"input":frozen,"guard":guard});
    let job = store
        .add_job(
            "workflow",
            "foundation.run",
            "local",
            "local",
            &snapshot,
            "failed",
        )
        .unwrap();
    store
        .update_job(
            &job,
            "failed",
            &json!({"error":"既有制作基础须先按来源契约导入：missing field `sourceSha256`"}),
        )
        .unwrap();
    let original = store.job(&job).unwrap();
    let reopened = Store::open(store.root.clone()).unwrap();
    let saved = reopened.job(&job).unwrap();
    let locked: LockedSource =
        serde_json::from_value(saved["snapshot"]["input"]["source"].clone()).unwrap();
    let context = &saved["snapshot"]["input"];
    let executor = SyntheticFoundation::default();
    let first = foundation_delivery::run_foundation_delivery(
        &locked,
        context,
        &executor,
        "synthetic",
        "medium",
    )
    .await
    .unwrap();
    let second = foundation_delivery::run_foundation_delivery(
        &locked,
        context,
        &executor,
        "synthetic",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(json!(first), json!(second));
    assert_eq!(first.direction, DIRECTION);
    assert_eq!(executor.calls.lock().unwrap().len(), 2);
    assert_eq!(reopened.job(&job).unwrap(), original);
    assert_eq!(reopened.foundation().unwrap(), frozen["foundation"]);
    assert_eq!(
        reopened.jobs().unwrap().len(),
        1,
        "reuse the existing workflow, no synthetic replacement workflow"
    );
    let mut c = reopened.conn().unwrap();
    let tx = c.transaction().unwrap();
    crate::workflow::verify_guard(&tx, &guard).unwrap();
}
