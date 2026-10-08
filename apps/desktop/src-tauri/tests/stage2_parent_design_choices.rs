//! Synthetic parent-choice provenance and frozen v1 replay. No native/provider calls.
use async_trait::async_trait;
use director_core::pipeline::{
    self, asset_design_contract as contract, asset_production_context as production, TextExecutor,
    TextRequest,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

const PARENT_SCOPE: &str =
    "红腕带仅限工坊作业场，居家态不自动沿用。\n背景纹理属普通制作取舍，不是原文事实。";

fn freeze_v1(input: &mut Value) {
    input[production::PROFILE_FIELD] = json!({"version":production::LEGACY_PROFILE,
        "instructions":production::LEGACY_INSTRUCTIONS,
        "instructionsSha256":pipeline::digest(production::LEGACY_INSTRUCTIONS)});
}

fn fixture(v1: bool) -> Value {
    let source = pipeline::prepare_source("第一集\n场景：工坊\n△甲佩戴红腕带，露出短发，在工作台旁作业。\n场景：屋内\n△甲换上浅色家居服走进屋内。\n场景：院落\n△甲换成深色外套在院落停步。\n").unwrap();
    let make = |id: &str, variant: &str, index: usize| {
        let scene = &source.scenes[index];
        let row = source
            .rows
            .iter()
            .find(|row| row.kind != "format" && row.scene_id.as_deref() == Some(scene.id.as_str()))
            .unwrap();
        json!({"id":id,"identityId":"actor","baseAssetId":"actor","variantId":variant,
            "name":id,"kind":"CH","version":1,"description":"同一主体制作身份",
            "sourceRefs":[row.id],"scope":{"kind":"SCENES","scenes":[{"episodeId":scene.episode_id,"sceneId":scene.id}],"reason":"合成准确形象范围"},
            "applicability":{"episodes":[scene.episode_id],"scenes":[scene.id]},
            "preparation":{"mode":"REFERENCE"}})
    };
    let parent = make("opaque-parent", "actor:base", 0);
    let mut child = make("opaque-child", "home", 1);
    child["sourceAssetId"] = json!("opaque-parent");
    child["preparation"]["mode"] = json!("DERIVE");
    let mut input = json!({"source":source,"assets":[parent,child],
        "foundation":{"sourceSha256":source.sha256,"facts":[]},"settings":{"textProvider":"codex_sdk"}});
    if v1 {
        freeze_v1(&mut input);
    } else {
        production::freeze(&mut input);
    }
    input
}

#[derive(Default)]
struct Designer {
    requests: Mutex<Vec<TextRequest>>,
    accepted: Mutex<HashSet<String>>,
    raw: Mutex<BTreeMap<String, Value>>,
    dispatches: AtomicUsize,
    replay: bool,
}

#[async_trait]
impl TextExecutor for Designer {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let key = serde_json::to_string(&request).unwrap();
        self.requests.lock().unwrap().push(request.clone());
        if self.replay {
            return self
                .raw
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .ok_or("frozen request bytes changed".into());
        }
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        let result = if request.stage == "assetVisualFacts" {
            let rows = request.input["rows"].as_array().unwrap();
            let row = |needle: &str| {
                rows.iter()
                    .find(|row| row["text"].as_str().unwrap().contains(needle))
                    .map(|row| row["id"].clone())
            };
            let mut facts = Vec::new();
            for identity in request.input["identities"].as_array().unwrap() {
                if identity["assetId"] == "actor" {
                    if let Some(reference) = row("红腕带") {
                        facts.push(json!({"assetId":"actor","variantId":null,"attribute":"hair.shape","value":"短发","scope":"STABLE","claim":"SOURCE","sourceRefs":[reference]}));
                        if identity["versions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|variant| variant["variantId"] == "actor:base")
                        {
                            facts.push(json!({"assetId":"actor","variantId":"actor:base","attribute":"accessories.wristband","value":"作业场佩戴红腕带","scope":"STATE","claim":"SOURCE","sourceRefs":[reference]}));
                        }
                    }
                    if let Some(reference) = row("家居服") {
                        if identity["versions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|variant| variant["variantId"] == "home")
                        {
                            facts.push(json!({"assetId":"actor","variantId":"home","attribute":"clothing","value":"浅色家居服","scope":"STATE","claim":"SOURCE","sourceRefs":[reference]}));
                        }
                    }
                    if let Some(reference) = row("深色外套") {
                        if identity["versions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|variant| variant["variantId"] == "yard")
                        {
                            facts.push(json!({"assetId":"actor","variantId":"yard","attribute":"clothing","value":"深色外套","scope":"STATE","claim":"SOURCE","sourceRefs":[reference]}));
                        }
                    }
                } else if identity["assetId"] == "band" {
                    if let Some(reference) = row("红腕带") {
                        facts.push(json!({"assetId":"band","variantId":null,"attribute":"color","value":"红色腕带","scope":"STABLE","claim":"SOURCE","sourceRefs":[reference]}));
                    }
                }
            }
            json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":facts,"issues":[]})
        } else {
            assert_eq!(request.stage, "assetDesign");
            let asset = &request.input["asset"];
            if asset["id"] == "opaque-child" {
                assert!(
                    self.accepted.lock().unwrap().contains("opaque-parent"),
                    "parent must have been accepted before child"
                );
            }
            if asset["id"] == "opaque-grandchild" {
                assert!(self.accepted.lock().unwrap().contains("opaque-child"));
            }
            let is_parent = asset["id"] == "opaque-parent";
            json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],
                "prompt":if is_parent {"同一稳定身份、短发、方正面部；作业场佩戴红腕带。"} else {"同一稳定身份、短发与方正面部，当前形象使用浅色家居服。"},
                "freeDesignScope":if is_parent {PARENT_SCOPE} else {"只补足当前形象未限定的剪裁，保留身份。"},
                "sourceRefs":asset["sourceRefs"],"referenceVariantIds":request.input["parentAsset"]["id"].as_str().into_iter().collect::<Vec<_>>()})
        };
        self.raw.lock().unwrap().insert(key, result.clone());
        Ok(result)
    }
    async fn accept(&self, request: &TextRequest, result: &Value) -> Result<(), String> {
        if request.stage == "assetDesign" {
            self.accepted
                .lock()
                .unwrap()
                .insert(result["assetId"].as_str().unwrap().to_owned());
        }
        Ok(())
    }
}

fn accepted(mut input: Value, output: &Value) -> Value {
    for asset in output["assets"].as_array().unwrap() {
        let saved = input["assets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["id"] == asset["id"])
            .unwrap();
        *saved = asset.clone();
    }
    input["foundation"]["facts"] = output["facts"].clone();
    input["foundation"]["visualFactBatches"] = output["visualFactBatches"].clone();
    input
}

#[tokio::test]
async fn new_parent_design_choices_copy_exact_accepted_limits_and_preserve_all_source_facts() {
    let input = fixture(false);
    let frozen = input.to_string();
    let executor = Designer::default();
    let result = pipeline::run_asset_design(&input, &executor, "synthetic", "medium")
        .await
        .unwrap();
    assert!(result["blocked"].as_array().unwrap().is_empty(), "{result}");
    let requests = executor.requests.lock().unwrap();
    let child = requests
        .iter()
        .find(|r| r.stage == "assetDesign" && r.input["asset"]["id"] == "opaque-child")
        .unwrap();
    assert_eq!(child.input["parentDesign"]["freeDesignScope"], PARENT_SCOPE);
    let proof = &child.input["parentDesign"]["designChoices"];
    assert_eq!(
        proof["usage"],
        "ACCEPTED_PARENT_DESIGN_CHOICES_NOT_SOURCE_FACT"
    );
    assert_eq!(
        proof["freeDesignScopeSha256"],
        pipeline::digest(PARENT_SCOPE)
    );
    assert_eq!(
        child.input["assetProductionContext"]["parentDesignChoices"],
        *proof
    );
    assert!(child.input["assetProductionContext"]["instructions"]
        .as_str()
        .unwrap()
        .contains("不把父稿限定场次的配饰"));
    let wrist = child.input["facts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fact| fact["attribute"] == "accessories.wristband")
        .unwrap();
    assert_eq!(wrist["scope"], "STATE");
    assert_eq!(wrist["claim"], "SOURCE");
    assert_eq!(wrist["variantId"], "actor:base");
    let scopes = child.input["assetProductionContext"]["factEvidenceScopes"]
        .as_array()
        .unwrap();
    let wrist_scope = scopes
        .iter()
        .find(|scope| scope["factId"] == wrist["id"])
        .unwrap();
    assert_eq!(
        wrist_scope["evidenceSceneIds"],
        json!([input["source"]["scenes"][0]["id"]])
    );
    assert_eq!(
        child.input["assetProductionContext"]["applicableScenes"][0]["id"],
        input["source"]["scenes"][1]["id"]
    );
    assert!(child.input["deltaFacts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|fact| fact["variantId"] == "home"));
    assert_eq!(input.to_string(), frozen);
    let child_design = result["designs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|design| design["assetId"] == "opaque-child")
        .unwrap();
    assert_eq!(
        child_design["designReceipt"]["basis"]["assetProductionContext"]["parentDesignChoices"],
        *proof
    );
    assert_eq!(child_design["referenceAssetIds"], json!(["opaque-parent"]));
    assert_eq!(
        result["imagePlan"]
            .as_array()
            .unwrap()
            .iter()
            .find(|plan| plan["assetId"] == "opaque-child")
            .unwrap()["dependencies"],
        json!(["opaque-parent"])
    );
}

#[tokio::test]
async fn legacy_v1_parent_design_choices_keep_seven_field_wire_and_exact_zero_dispatch_replay() {
    let input = fixture(true);
    let frozen = input.to_string();
    assert_eq!(
        pipeline::digest(production::LEGACY_INSTRUCTIONS),
        "918274f96c798388a74c1f022d34146a885ae2134fd469f12b8e1955c6c443e1"
    );
    let original = Designer::default();
    let first = pipeline::run_asset_design(&input, &original, "synthetic", "medium")
        .await
        .unwrap();
    assert!(first["blocked"].as_array().unwrap().is_empty(), "{first}");
    let requests = original.requests.lock().unwrap();
    let child = requests
        .iter()
        .find(|r| r.stage == "assetDesign" && r.input["asset"]["id"] == "opaque-child")
        .unwrap();
    let parent = first["designs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["assetId"] == "opaque-parent")
        .unwrap();
    assert_eq!(
        child.input["parentDesign"],
        json!({"assetId":"opaque-parent","variantId":"actor:base","assetVersion":1,"prompt":parent["prompt"],"origin":"AI","sourceRefs":parent["sourceRefs"],"evidencePolicy":"已保存来源设计是外观一致性约束，不是新的原文事实；未提供图片像素。"})
    );
    assert_eq!(
        child.input["assetProductionContext"]["version"],
        production::LEGACY_PROFILE
    );
    assert_eq!(
        child.input["assetProductionContext"]["instructions"],
        production::LEGACY_INSTRUCTIONS
    );
    assert!(child.input["assetProductionContext"]
        .get("parentDesignChoices")
        .is_none());
    drop(requests);
    let replay = Designer {
        replay: true,
        raw: Mutex::new(original.raw.into_inner().unwrap()),
        ..Default::default()
    };
    let second = pipeline::run_asset_design(&input, &replay, "synthetic", "medium")
        .await
        .unwrap();
    assert_eq!(second, first);
    assert_eq!(replay.dispatches.load(Ordering::SeqCst), 0);
    assert_eq!(input.to_string(), frozen);
    let saved = accepted(input, &first);
    for asset in saved["assets"].as_array().unwrap() {
        assert!(
            !pipeline::automatic_design_required(&saved, asset),
            "v1 accepted receipts remain current under the original frozen profile"
        );
    }
    let mut newly_frozen = saved.clone();
    production::freeze(&mut newly_frozen);
    assert!(
        pipeline::automatic_design_required(&newly_frozen, &newly_frozen["assets"][0]),
        "only new v2 workflows select the new contract"
    );
    for design in first["designs"].as_array().unwrap() {
        production::validate_consumed_prop_designs(design, saved["assets"].as_array().unwrap())
            .unwrap();
    }
}

#[tokio::test]
async fn saved_parent_design_choices_reuse_exact_limits_and_scope_only_edit_invalidates_new_child()
{
    let input = fixture(false);
    let output = pipeline::run_asset_design(&input, &Designer::default(), "synthetic", "medium")
        .await
        .unwrap();
    let mut saved = accepted(input, &output);
    saved["selection"] = json!({"assetIds":["opaque-child"]});
    let frozen_parent = saved["assets"][0].clone();
    let executor = Designer::default();
    executor
        .accepted
        .lock()
        .unwrap()
        .insert("opaque-parent".into());
    let fresh = pipeline::run_asset_design(&saved, &executor, "synthetic", "medium")
        .await
        .unwrap();
    assert!(fresh["blocked"].as_array().unwrap().is_empty(), "{fresh}");
    let requests = executor.requests.lock().unwrap();
    let designs: Vec<_> = requests
        .iter()
        .filter(|r| r.stage == "assetDesign")
        .collect();
    assert_eq!(
        designs.len(),
        1,
        "implicit accepted parent is not regenerated"
    );
    assert_eq!(
        designs[0].input["parentDesign"]["freeDesignScope"],
        PARENT_SCOPE
    );
    assert_eq!(saved["assets"][0], frozen_parent);
    let child = &saved["assets"][1];
    production::validate_consumed_prop_designs(
        &child["design"],
        saved["assets"].as_array().unwrap(),
    )
    .unwrap();
    let raw = child["design"].to_string();
    let old_chain = contract::source_chain(saved["assets"].as_array().unwrap(), child).unwrap();
    let mut changed = saved.clone();
    changed["assets"][0]["design"]["freeDesignScope"] = json!("改变父稿制作限制，正文和事实均未改");
    assert_eq!(
        contract::source_chain(changed["assets"].as_array().unwrap(), &changed["assets"][1])
            .unwrap(),
        old_chain,
        "scope was not part of the old prompt/receipt chain hash"
    );
    assert!(production::validate_consumed_prop_designs(
        &child["design"],
        changed["assets"].as_array().unwrap()
    )
    .is_err());
    assert!(!contract::design_receipt_current(
        &changed,
        &changed["assets"][1]
    ));
    assert_eq!(child["design"].to_string(), raw);
    changed = saved.clone();
    changed["assets"][0]["version"] = json!(99);
    changed["assets"][0]["adoptedMediaId"] = json!("new-adoption");
    production::validate_consumed_prop_designs(
        &child["design"],
        changed["assets"].as_array().unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn new_v2_child_reuses_exact_accepted_v1_parent_without_parent_redesign() {
    let input = fixture(true);
    let first = pipeline::run_asset_design(&input, &Designer::default(), "synthetic", "medium")
        .await
        .unwrap();
    assert!(first["blocked"].as_array().unwrap().is_empty(), "{first}");
    let mut next = accepted(input, &first);
    next["selection"] =
        json!({"assetIds":["opaque-parent","opaque-child"],"explicitAssetIds":["opaque-child"]});
    production::freeze(&mut next);
    // Real engine input has image_closure already applied. During produce or
    // reopen, only the whole frozen profile is copied back, not this new seed.
    next["selection"]
        .as_object_mut()
        .unwrap()
        .remove("explicitAssetIds");
    next["automaticDesign"] = json!(true);
    let parent = next["assets"][0].clone();
    let original_parent_bytes = parent.to_string();
    assert!(
        !pipeline::automatic_design_required(&next, &parent),
        "implicit v1 parent still has valid source/facts/design evidence"
    );
    assert!(
        pipeline::automatic_design_required(&next, &next["assets"][1]),
        "explicit v2 child must receive the new parent restrictions"
    );
    let mut design_only = next.clone();
    design_only
        .as_object_mut()
        .unwrap()
        .remove("automaticDesign");
    let manual_executor = Designer::default();
    manual_executor
        .accepted
        .lock()
        .unwrap()
        .insert("opaque-parent".into());
    let manual_output =
        pipeline::run_asset_design(&design_only, &manual_executor, "synthetic", "medium")
            .await
            .unwrap();
    assert!(
        manual_output["blocked"].as_array().unwrap().is_empty(),
        "{manual_output}"
    );
    assert_eq!(
        manual_executor
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.stage == "assetDesign")
            .count(),
        1,
        "GUI design-only still reuses the closed implicit parent"
    );
    assert_eq!(manual_output["designs"][0]["assetId"], "opaque-child");
    let executor = Designer::default();
    executor
        .accepted
        .lock()
        .unwrap()
        .insert("opaque-parent".into());
    let output = pipeline::run_asset_design(&next, &executor, "synthetic", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    let requests = executor.requests.lock().unwrap();
    let designs: Vec<_> = requests
        .iter()
        .filter(|r| r.stage == "assetDesign")
        .collect();
    assert_eq!(
        designs.len(),
        1,
        "only the explicitly selected child is redesigned"
    );
    assert_eq!(designs[0].input["asset"]["id"], "opaque-child");
    assert_eq!(
        designs[0].input["assetProductionContext"]["version"],
        production::PROFILE
    );
    assert_eq!(
        designs[0].input["parentDesign"]["freeDesignScope"],
        PARENT_SCOPE
    );
    assert_eq!(
        designs[0].input["parentDesign"]["designChoices"]["designReceiptSha256"],
        parent["design"]["designReceipt"]["sha256"]
    );
    assert_eq!(next["assets"][0].to_string(), original_parent_bytes);
    assert_eq!(
        parent["design"]["frozenContext"]["assetProductionContext"]["version"],
        production::LEGACY_PROFILE
    );
    assert_eq!(
        next[production::PROFILE_FIELD]["explicitAssetIds"],
        json!(["opaque-child"])
    );
    let fresh_child = &output["designs"][0];
    let final_assets = accepted(next.clone(), &output);
    production::validate_consumed_prop_designs(
        fresh_child,
        final_assets["assets"].as_array().unwrap(),
    )
    .unwrap();
    let mut changed = next.clone();
    changed["assets"][0]["description"] = json!("后来修改了父身份制作资料");
    assert!(
        pipeline::automatic_design_required(&changed, &changed["assets"][0]),
        "no cross-profile reuse when the actual parent contract changed"
    );
    changed = next.clone();
    changed["assets"][0]["design"]["frozenContext"]["assetProductionContext"]["instructions"] =
        json!("原父冻结合同被修改");
    assert!(pipeline::automatic_design_required(
        &changed,
        &changed["assets"][0]
    ));
}

#[tokio::test]
async fn manual_parent_design_choices_do_not_promote_stale_ai_scope_or_change_parent_reference() {
    let mut input = fixture(false);
    input["selection"] = json!({"assetIds":["opaque-child"]});
    input["assets"][0]["prompt"] = json!("用户当前确认的稳定身份稿");
    input["assets"][0]["promptOrigin"] = json!("MANUAL");
    input["assets"][0]["design"] =
        json!({"schemaVersion":"asset-design/v1","freeDesignScope":"失效AI稿限制，不是当前手写稿"});
    let executor = Designer::default();
    executor
        .accepted
        .lock()
        .unwrap()
        .insert("opaque-parent".into());
    let output = pipeline::run_asset_design(&input, &executor, "synthetic", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    let requests = executor.requests.lock().unwrap();
    let child = requests.iter().find(|r| r.stage == "assetDesign").unwrap();
    assert_eq!(child.input["parentDesign"]["origin"], "MANUAL");
    assert!(child.input["parentDesign"].get("freeDesignScope").is_none());
    assert!(child.input["assetProductionContext"]
        .get("parentDesignChoices")
        .is_none());
    assert_eq!(
        output["designs"][0]["referenceAssetIds"],
        json!(["opaque-parent"])
    );
}

#[tokio::test]
async fn v2_parent_design_choices_keep_outside_prop_under_original_v1_frozen_contract() {
    let mut input = fixture(true);
    let mut prop = input["assets"][0].clone();
    prop["id"] = json!("band-row");
    prop["identityId"] = json!("band");
    prop["baseAssetId"] = json!("band");
    prop["variantId"] = json!("band:base");
    prop["kind"] = json!("PROP");
    input["assets"] = json!([input["assets"][0], prop]);
    let first = pipeline::run_asset_design(&input, &Designer::default(), "synthetic", "medium")
        .await
        .unwrap();
    assert!(first["blocked"].as_array().unwrap().is_empty(), "{first}");
    let mut next = accepted(input, &first);
    production::freeze(&mut next);
    next["selection"] = json!({"assetIds":["opaque-parent"]});
    let original_prop = next["assets"][1].clone();
    let executor = Designer::default();
    let output = pipeline::run_asset_design(&next, &executor, "synthetic", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    let requests = executor.requests.lock().unwrap();
    let designs: Vec<_> = requests
        .iter()
        .filter(|r| r.stage == "assetDesign")
        .collect();
    assert_eq!(designs.len(), 1);
    assert_eq!(
        designs[0].input["assetProductionContext"]["relatedProps"][0]["assetId"],
        "band-row"
    );
    assert!(
        designs[0].input["assetProductionContext"]["relatedProps"][0]["acceptedDesign"].is_object()
    );
    assert_eq!(next["assets"][1], original_prop);
    assert_eq!(
        original_prop["design"]["frozenContext"]["assetProductionContext"]["version"],
        production::LEGACY_PROFILE
    );
}

#[tokio::test]
async fn ancestor_scope_only_change_rejects_v2_grandchild_and_rolls_back_atomic_publication() {
    let mut input = fixture(false);
    let mut grandchild = input["assets"][1].clone();
    let scene = &input["source"]["scenes"][2];
    let row = input["source"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["text"].as_str().unwrap().contains("深色外套"))
        .unwrap();
    grandchild["id"] = json!("opaque-grandchild");
    grandchild["name"] = json!("末级合成形象");
    grandchild["variantId"] = json!("yard");
    grandchild["sourceAssetId"] = json!("opaque-child");
    grandchild["sourceRefs"] = json!([row["id"]]);
    grandchild["scope"] = json!({"kind":"SCENES","scenes":[{"episodeId":scene["episodeId"],"sceneId":scene["id"]}],"reason":"末级合成形象准确场次"});
    grandchild["applicability"] = json!({"episodes":[scene["episodeId"]],"scenes":[scene["id"]]});
    input["assets"].as_array_mut().unwrap().push(grandchild);
    let temporary = tempfile::tempdir().unwrap();
    let store =
        director_core::store::Store::open(temporary.path().join("isolated-parent-publication"))
            .unwrap();
    store
        .create("合成父限制发布验收", "parent-choice-test")
        .unwrap();
    input["source"] = store.save_source(input["source"].clone()).unwrap();
    let saved: Vec<_> = input["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|asset| store.save_asset(asset.clone()).unwrap())
        .collect();
    input["assets"] = json!(saved);
    let first = pipeline::run_asset_design(&input, &Designer::default(), "synthetic", "medium")
        .await
        .unwrap();
    assert!(first["blocked"].as_array().unwrap().is_empty(), "{first}");
    store.publish_asset_design(1, 0, &first).unwrap();
    let mut next = accepted(input, &first);
    next["assets"] = json!([
        store.get("assets", "opaque-parent").unwrap(),
        store.get("assets", "opaque-child").unwrap(),
        store.get("assets", "opaque-grandchild").unwrap()
    ]);
    next["foundation"] = store.foundation().unwrap();
    next["selection"] = json!({"assetIds":["opaque-parent","opaque-child","opaque-grandchild"],"explicitAssetIds":["opaque-grandchild"]});
    production::freeze(&mut next);
    let executor = Designer::default();
    executor
        .accepted
        .lock()
        .unwrap()
        .extend(["opaque-parent".into(), "opaque-child".into()]);
    let pending = pipeline::run_asset_design(&next, &executor, "synthetic", "medium")
        .await
        .unwrap();
    assert!(
        pending["blocked"].as_array().unwrap().is_empty(),
        "{pending}"
    );
    assert_eq!(pending["assets"].as_array().unwrap().len(), 1);
    let last = &pending["assets"][0]["design"];
    production::validate_consumed_prop_designs(last, next["assets"].as_array().unwrap()).unwrap();
    let raw = last.to_string();
    let middle = next["assets"][1].clone();
    let old_chain =
        contract::source_chain(next["assets"].as_array().unwrap(), &next["assets"][2]).unwrap();
    let mut edited = store.get("assets", "opaque-parent").unwrap();
    edited["design"]["freeDesignScope"] = json!("仅改变祖先取舍，不动prompt/原receipt/中间父稿");
    let edited = store.save_asset(edited).unwrap();
    let changed = vec![edited, middle.clone(), next["assets"][2].clone()];
    assert_eq!(changed[1], middle);
    assert_eq!(
        contract::source_chain(&changed, &changed[2]).unwrap(),
        old_chain
    );
    assert!(production::validate_consumed_prop_designs(last, &changed).is_err());
    assert_eq!(
        last.to_string(),
        raw,
        "pending accepted evidence is never rewritten on rejection"
    );
    let mut duplicate = changed.clone();
    duplicate.push(middle);
    assert!(production::validate_consumed_prop_designs(last, &duplicate).is_err());
    let before_child = store.get("assets", "opaque-grandchild").unwrap();
    let before_foundation = store.foundation().unwrap();
    let error = store
        .publish_asset_design(1, before_foundation["version"].as_i64().unwrap(), &pending)
        .unwrap_err();
    assert!(error.contains("制作取舍"), "{error}");
    assert_eq!(
        store.get("assets", "opaque-grandchild").unwrap(),
        before_child
    );
    assert_eq!(store.foundation().unwrap(), before_foundation);
}
