use super::*;
use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

fn visual() -> Value {
    let corpus: Value =
        serde_json::from_str(include_str!("series_lite_execution_golden.json")).unwrap();
    parse(corpus["files"]["bundle.json"].as_str().unwrap()).unwrap()["episodes"][0]["settings"]
        ["visualBaseline"]
        .clone()
}
fn setup() -> (tempfile::TempDir, Store, String, Value, String) {
    setup_with_script("第1集\n1-1 门口 日 外\n△甲抬起手。\n甲：门还没开。\n")
}
fn setup_with_script(script: &str) -> (tempfile::TempDir, Store, String, Value, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("synthetic preparation", "synthetic").unwrap();
    store.ensure_workflow_storage().unwrap();
    let locked = crate::pipeline::prepare_source(script).unwrap();
    let ep = locked.episodes[0].id.clone();
    let source = store
        .save_source(serde_json::to_value(&locked).unwrap())
        .unwrap();
    let baseline = visual();
    let input = json!({"source":source,"selection":{"episodeIds":[ep]},"existingUnits":[],"settings":{"visualBaseline":baseline,"visualStyle":baseline["visualStyle"],"maxUnitSeconds":30,"fantasyMode":"off","confirmedDirection":"沿用已采用完整视觉基准"},"liteBoundaryByEpisode":{"1":{"kind":"firstInScope"}}});
    let run=store.add_job("workflow","storyboard.run","local","local",&json!({"input":{"liteControllerContract":crate::series_lite_controller_store::VERSION,"source":input["source"],"selection":input["selection"]}}),"running").unwrap();
    (temp, store, run, input, ep)
}
struct Synthetic {
    store: Store,
    run: String,
    calls: AtomicUsize,
    last: Mutex<Option<(String, String, String)>>,
    bad: bool,
    three_levels: bool,
}
impl Synthetic {
    fn new(store: Store, run: String) -> Self {
        Self {
            store,
            run,
            calls: AtomicUsize::new(0),
            last: Mutex::new(None),
            bad: false,
            three_levels: false,
        }
    }
    fn response(&self, request: &TextRequest) -> Value {
        let row = request.input["sourceRows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "body")
            .unwrap()["id"]
            .clone();
        let d = |name: &str| json!({"brief":format!("{name}定位卡"),"full":format!("{name}的完整必要分镜设定：明确轮廓、行动支点和空间关系；不改源定事实。"),"sourceRefs":[if self.bad{json!("unknown-source-row")}else{row.clone()}],"sourceFacts":[{"statement":"源支持当前集场使用","sourceRefs":[row]}],"productionChoices":["造型和空间补足是制作选择，不冒充源事实"],"dependencies":[]});
        let a = |key: &str, name: &str, kind: &str| json!({"localKey":key,"existingId":null,"name":name,"kind":kind,"appearance":"常态","identityBasis":"合成源本集明确出现的主体","identityParentKey":null,"mediaParentKey":null,"design":d(name)});
        let mut assets = vec![a("hero", "甲", "CH"), a("door", "门口", "SC")];
        let mut keys = vec!["hero", "door"];
        if self.three_levels {
            let mut suit = a("suit", "甲西装", "CH");
            suit["identityParentKey"] = json!("hero");
            suit["design"]["dependencies"] =
                json!([{"key":"hero","purpose":"同一人物的西装设计接续"}]);
            // A text/identity relationship alone must not wait for a parent image.
            let mut injured = a("injured", "甲西装受伤", "CH");
            injured["identityParentKey"] = json!("suit");
            injured["mediaParentKey"] = json!("suit");
            injured["design"]["dependencies"] =
                json!([{"key":"suit","purpose":"伤痕设计基于西装设计"}]);
            assets.extend([suit, injured]);
            keys.extend(["suit", "injured"]);
        }
        json!({"version":VERSION,"assets":assets,"scenes":[{"sceneId":request.input["scenes"][0]["id"],"assetKeys":keys}],"unresolved":[]})
    }
}
#[async_trait]
impl TextExecutor for Synthetic {
    async fn execute(&self, request: TextRequest) -> Result<Value> {
        assert_eq!(request.stage, STAGE);
        self.calls.fetch_add(1, Ordering::SeqCst);
        let raw = self.response(&request);
        let mut snapshot = serde_json::to_value(&request).unwrap();
        snapshot["executorIdentity"] =
            json!({"provider":"codex","executable":"C:\\synthetic-never-launch.exe"});
        let sha = digest(&snapshot.to_string());
        let job = self.store.add_job(
            "text",
            STAGE,
            "codex",
            "synthetic-account",
            &snapshot,
            "received",
        )?;
        let node = format!("synthetic-prepare:{}", digest(&request.input.to_string()));
        self.store.checkpoint(
            &self.run,
            &node,
            STAGE,
            &sha,
            "running",
            Some(&job),
            None,
            None,
        )?;
        self.store
            .update_job(&job, "received", &json!({"result":raw}))?;
        self.store.event(
            &job,
            "text_received_raw",
            &json!({"requestHash":sha,"value":raw}),
        )?;
        *self.last.lock().unwrap() = Some((job, node, sha));
        Ok(raw)
    }
    async fn accept(&self, _: &TextRequest, raw: &Value) -> Result<()> {
        let (job, node, sha) = self.last.lock().unwrap().clone().unwrap();
        self.store.update_job(&job,"completed",&json!({"result":raw,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}}))?;
        self.store.checkpoint(
            &self.run,
            &node,
            STAGE,
            &sha,
            "accepted",
            Some(&job),
            Some(raw),
            None,
        )
    }
    async fn reject(&self, _: &TextRequest, raw: &Value, error: &str) -> Result<()> {
        let (job, node, sha) = self.last.lock().unwrap().clone().unwrap();
        self.store.update_job(&job,"rejected",&json!({"result":raw,"error":error,"receipt":{"contract":"desktop-runner/v1","validation":"rejected"}}))?;
        self.store.checkpoint(
            &self.run,
            &node,
            STAGE,
            &sha,
            "rejected",
            Some(&job),
            Some(raw),
            Some(error),
        )
    }
}
#[tokio::test]
async fn one_preparation_proves_new_design_origin_and_save_reopen_reuses_exact_bytes() {
    let (_temp, store, run, input, ep) = setup();
    let executor = Synthetic::new(store.clone(), run.clone());
    let result = prepare(
        &store,
        &run,
        &input,
        &ep,
        &executor,
        "gpt-6.1-sol",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert!(result.root.join("source-lock.json").is_file());
    assert!(result.root.join("registry/current.json").is_file());
    assert_eq!(
        result.common_inputs["boundary"],
        digest(&json!({"kind":"firstInScope"}).to_string())
    );
    let assets = store.values("assets").unwrap();
    assert_eq!(assets.len(), 2);
    for a in &assets {
        assert_eq!(a["adoptedDesign"]["version"], "series-lite-design/v1");
        assert_eq!(a["prompt"], "");
        assert!(a["adoptedMediaId"].is_null());
        assert!(matches!(a["kind"].as_str(), Some("character" | "scene")));
        assert_eq!(a["workflowProfile"], crate::asset_graph::PROFILE);
        assert!(a["filename"].as_str().unwrap().ends_with(".png"));
        assert!(a["placeholderOrdinal"].as_u64().is_some());
    }
    let c = store.conn().unwrap();
    let designs: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM series_lite_design_origin_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(designs, 2);
    drop(c);
    let reopened = prepare(
        &Store::open(store.root.clone()).unwrap(),
        &run,
        &input,
        &ep,
        &executor,
        "different-current-account-model",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(reopened.base, result.base);
    assert_eq!(reopened.supply, result.supply);
    assert_eq!(reopened.common_inputs, result.common_inputs);
}
#[tokio::test]
async fn proven_designs_with_explicit_scene_ids_reuse_locally_without_a_second_model_job() {
    let (_temp, store, run, mut input, ep) = setup_with_script(
        "第1集\n1-1 门口 日 外\n△甲抬起手。\n甲：门还没开。\n第2集\n2-1 门口 日 外\n△甲再次抬手。\n甲：我再等等。\n",
    );
    let executor = Synthetic::new(store.clone(), run.clone());
    prepare(&store, &run, &input, &ep, &executor, "model", "medium")
        .await
        .unwrap();
    let assets = store.values("assets").unwrap();
    let second_ep = input["source"]["episodes"][1]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let second_scene = input["source"]["episodes"][1]["sceneIds"][0]
        .as_str()
        .unwrap()
        .to_owned();
    input["selection"]["episodeIds"] = json!([second_ep]);
    input["liteBoundaryByEpisode"]["2"] = json!({"kind":"missingPrevious","episodeId":ep});
    input["liteSceneBindings"] =
        json!({second_scene:assets.iter().map(|a|a["id"].clone()).collect::<Vec<_>>()});
    let result = prepare(
        &store, &run, &input, &second_ep, &executor, "model", "medium",
    )
    .await
    .unwrap();
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        result.evidence["mode"],
        "program-only-existing-design-reuse"
    );
    assert_eq!(result.evidence["modelCalls"], 0);
    assert_eq!(result.prepare_job_id, "");
    assert_eq!(store.values("assets").unwrap(), assets);
    let connection = store.conn().unwrap();
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT COUNT(*) FROM jobs WHERE kind='text'", [], |r| r
                .get(0))
            .unwrap(),
        1
    );
    let frozen: Frozen = row(
        &connection,
        "series_lite_prepare_frozen_v1",
        &preparation_id(&run, &second_ep),
    )
    .unwrap()
    .unwrap();
    assert_eq!(frozen.request.stage, LOCAL_STAGE);
    assert!(frozen.request.input["localResult"]["assets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["design"].is_null()));
    drop(connection);
    let reopened = prepare(
        &Store::open(store.root.clone()).unwrap(),
        &run,
        &input,
        &second_ep,
        &executor,
        "changed-account",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(reopened.base, result.base);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn matching_names_without_scene_identity_evidence_do_not_select_local_reuse() {
    let (_temp, store, run, input, ep) = setup();
    let executor = Synthetic::new(store.clone(), run.clone());
    prepare(&store, &run, &input, &ep, &executor, "model", "medium")
        .await
        .unwrap();
    let assets = store.values("assets").unwrap();
    assert!(assets.iter().any(|a| a["name"] == "门口"));
    // The accepted designs cite a single action, not all rows of this scene.
    // Neither a matching title nor matching character text supplies membership.
    assert!(
        deterministic_response(&single_input(&input, &ep), &ep, &assets)
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn image_receiver_accepts_proven_design_and_rejects_body_version_or_origin_tampering() {
    let (_temp, store, run, input, ep) = setup();
    let executor = Synthetic::new(store.clone(), run.clone());
    prepare(&store, &run, &input, &ep, &executor, "model", "medium")
        .await
        .unwrap();
    let asset = store.values("assets").unwrap().remove(0);
    let request = json!({"assetIds":[asset["id"]]});
    let output = store
        .image_workflow_input(&request, &input["settings"])
        .unwrap();
    assert_eq!(
        output["imageQueue"]["items"][0]["filename"],
        asset["filename"]
    );
    for field in ["full", "version", "adoptionBasis"] {
        let mut changed = asset.clone();
        changed["adoptedDesign"][field] = match field {
            "full" => json!("伪造的完整设计"),
            "version" => json!("series-full-design/v1"),
            _ => json!({"kind":"status-only-no-original-proof"}),
        };
        // Even removing the mutable marker cannot downgrade an immutable origin.
        changed.as_object_mut().unwrap().remove("litePrepareOrigin");
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE assets SET data=?1 WHERE id=?2",
                params![changed.to_string(), asset["id"].as_str()],
            )
            .unwrap();
        assert!(
            store
                .image_workflow_input(&request, &input["settings"])
                .unwrap_err()
                .contains("LITE_PREPARE"),
            "{field}"
        );
    }
}
#[tokio::test]
async fn identity_design_and_media_parents_remain_distinct_across_three_levels() {
    let (_temp, store, run, input, ep) = setup();
    let mut executor = Synthetic::new(store.clone(), run.clone());
    executor.three_levels = true;
    prepare(&store, &run, &input, &ep, &executor, "model", "medium")
        .await
        .unwrap();
    let assets = store.values("assets").unwrap();
    let hero = assets.iter().find(|a| a["name"] == "甲").unwrap();
    let suit = assets.iter().find(|a| a["name"] == "甲西装").unwrap();
    let injured = assets.iter().find(|a| a["name"] == "甲西装受伤").unwrap();
    assert_eq!(suit["identityId"], hero["identityId"]);
    assert_eq!(injured["identityId"], hero["identityId"]);
    assert_eq!(
        suit["adoptedDesign"]["designDependencies"][0]["assetId"],
        hero["id"]
    );
    assert_eq!(
        injured["adoptedDesign"]["designDependencies"][0]["assetId"],
        suit["id"]
    );
    assert!(crate::asset_graph::dependencies(&assets, suit)
        .unwrap()
        .is_empty());
    assert_eq!(
        crate::asset_graph::dependencies(&assets, injured).unwrap(),
        vec![suit["id"].as_str().unwrap().to_owned()]
    );
    assert_eq!(injured["sourceVariantId"], suit["variantId"]);
    assert_eq!(
        injured["referenceRequirements"][0]["targetVariantId"],
        suit["variantId"]
    );
    assert!(store
        .adopted(
            suit["id"].as_str().unwrap(),
            suit["variantId"].as_str().unwrap()
        )
        .unwrap()
        .is_none());
    // Narrative intake succeeds without any generated parent image.
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn manual_image_prompt_changes_do_not_invalidate_prepared_text_design() {
    let (_temp, store, run, input, ep) = setup();
    let executor = Synthetic::new(store.clone(), run.clone());
    let first = prepare(
        &store,
        &run,
        &input,
        &ep,
        &executor,
        "gpt-6.1-sol",
        "medium",
    )
    .await
    .unwrap();
    let mut a = store.values("assets").unwrap().remove(0);
    let version = a["version"].as_u64().unwrap() + 1;
    a["prompt"] = json!("用户手写的图片提示词，必须保留");
    a["version"] = json!(version);
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE assets SET version=?1,data=?2 WHERE id=?3",
            params![version, a.to_string(), a["id"].as_str()],
        )
        .unwrap();
    let replay = prepare(
        &store,
        &run,
        &input,
        &ep,
        &executor,
        "gpt-6.1-sol",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.base, replay.base);
    assert!(store
        .values("assets")
        .unwrap()
        .iter()
        .any(|a| a["prompt"] == "用户手写的图片提示词，必须保留"));
}
#[tokio::test]
async fn source_error_has_no_asset_or_partial_prepared_publication() {
    let (_temp, store, run, input, ep) = setup();
    let mut executor = Synthetic::new(store.clone(), run.clone());
    executor.bad = true;
    assert!(prepare(
        &store,
        &run,
        &input,
        &ep,
        &executor,
        "gpt-6.1-sol",
        "medium"
    )
    .await
    .unwrap_err()
    .contains("SOURCE_ANCHOR"));
    assert_eq!(store.values("assets").unwrap().len(), 0);
    let c = store.conn().unwrap();
    assert_eq!(
        c.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM series_lite_prepare_result_v1",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM series_lite_prepare_frozen_v1",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        1
    );
}
#[tokio::test]
async fn saved_source_change_is_held_before_another_model_call() {
    let (_temp, store, run, input, ep) = setup();
    let executor = Synthetic::new(store.clone(), run.clone());
    prepare(
        &store,
        &run,
        &input,
        &ep,
        &executor,
        "gpt-6.1-sol",
        "medium",
    )
    .await
    .unwrap();
    let changed =
        crate::pipeline::prepare_source("第1集\n1-1 门口 日 外\n甲：这是用户更新后的另一句。\n")
            .unwrap();
    store
        .save_source(serde_json::to_value(changed).unwrap())
        .unwrap();
    assert!(prepare(
        &store,
        &run,
        &input,
        &ep,
        &executor,
        "gpt-6.1-sol",
        "medium"
    )
    .await
    .is_err());
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn file_materialization_never_overwrites_an_existing_different_result() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), "candidate.json", b"original").unwrap();
    put(root.path(), "candidate.json", b"original").unwrap();
    assert!(put(root.path(), "candidate.json", b"different")
        .unwrap_err()
        .contains("OUTPUT_ALREADY_DIFFERENT"));
    assert_eq!(
        std::fs::read(root.path().join("candidate.json")).unwrap(),
        b"original"
    );
}
