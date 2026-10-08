//! Design and visual-fact checkpoints publish atomically with optimistic versions.
use director_core::{pipeline, store::Store};
use serde_json::{json, Value};
fn fixture() -> (tempfile::TempDir, Store, Value, Value) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("isolated")).unwrap();
    store.create("设计发布验收", "design-test").unwrap();
    let source = pipeline::prepare_source("第一集\n场景：门口\n△甲穿白衣。\n").unwrap();
    let source = store.save_source(json!(source)).unwrap();
    let asset=store.save_asset(json!({"id":"a","baseAssetId":"person","variantId":"v","name":"甲","kind":"CH","sourceRefs":[source["rows"][2]["id"]],"prompt":"旧文字"})).unwrap();
    (temp, store, source, asset)
}
fn output(source: &Value, asset: &Value) -> Value {
    let mut design = asset.clone();
    design["prompt"] = json!("新设计文字");
    design["promptOrigin"] = json!("AI");
    json!({"assets":[design],"facts":[{"id":"fact","assetId":"person","variantId":"v","attribute":"服装","value":"白衣","scope":"STATE","claim":"SOURCE","sourceRefs":[source["rows"][2]["id"]],"quotes":["不可信的模型引文"]}],
      "visualFactBatches":[{"status":"accepted","sourceSha256":source["sha256"],"inputFingerprint":"hash","ruleSha256":"rule","facts":[]}]})
}
#[test]
fn accepted_design_and_facts_publish_with_program_copied_quotes() {
    let (_temp, store, source, asset) = fixture();
    let result = output(&source, &asset);
    store.publish_asset_design(1, 0, &result).unwrap();
    assert_eq!(store.get("assets", "a").unwrap()["prompt"], "新设计文字");
    let foundation = store.foundation().unwrap();
    assert_eq!(
        foundation["facts"][0]["quotes"][0],
        source["rows"][2]["text"]
    );
    assert_eq!(foundation["visualFactBatches"][0]["status"], "accepted");
    assert_eq!(foundation["sourceVersion"], 1);
}
#[test]
fn invalid_cache_or_concurrent_edit_cannot_partially_replace_design() {
    let (_temp, store, source, asset) = fixture();
    let mut result = output(&source, &asset);
    result["visualFactBatches"][0]["sourceSha256"] = json!("other");
    assert!(store.publish_asset_design(1, 0, &result).is_err());
    assert_eq!(store.get("assets", "a").unwrap()["prompt"], "旧文字");
    assert!(store.foundation().unwrap().is_null());
    let result = output(&source, &asset);
    let mut newer = asset.clone();
    newer["prompt"] = json!("用户编辑");
    store.save_asset(newer).unwrap();
    assert!(store.publish_asset_design(1, 0, &result).is_err());
    assert_eq!(store.get("assets", "a").unwrap()["prompt"], "用户编辑");
    assert!(store.foundation().unwrap().is_null());
}

#[test]
fn changed_out_of_scope_prop_rolls_back_character_and_fact_publication() {
    let (_temp, store, source, asset) = fixture();
    let prop = store
        .save_asset(json!({
            "id":"watch", "baseAssetId":"watch-identity", "variantId":"base",
            "kind":"PROP", "name":"腕表", "sourceRefs":[source["rows"][2]["id"]],
            "prompt":"蓝色皮带腕表", "design":{"sourceSha256":source["sha256"],
                "designReceipt":{"sha256":"original-prop-receipt","basis":{"sourceChain":[]}}}
        }))
        .unwrap();
    let context = json!({
        "version":pipeline::asset_production_context::PROFILE,
        "sourceSha256":source["sha256"],
        "instructions":pipeline::asset_production_context::INSTRUCTIONS,
        "instructionsSha256":pipeline::digest(pipeline::asset_production_context::INSTRUCTIONS),
        "applicableScenes":[],"factEvidenceScenes":[],"factEvidenceScopes":[],"readSceneIds":[],
        "relatedProps":[{"assetId":prop["id"],"identityId":"watch-identity",
            "variantId":prop["variantId"],
            "assetSemanticVersion":pipeline::digest(&pipeline::asset_design_contract::asset_semantics(&prop).to_string()),
            "sharedSourceRefs":[source["rows"][2]["id"]],
            "acceptedDesign":{"designVersion":pipeline::digest(&json!({"prompt":prop["prompt"],"receipt":prop["design"]["designReceipt"]}).to_string()),
                "designReceiptSha256":"original-prop-receipt"}}]
    });
    let relation = &context["relatedProps"][0];
    let binding = json!({"version":pipeline::asset_production_context::PROFILE,
        "instructionsSha256":context["instructionsSha256"],
        "sceneContextSha256":pipeline::digest(&json!({"applicableScenes":[],"factEvidenceScenes":[],"factEvidenceScopes":[],"readSceneIds":[]}).to_string()),
        "relatedPropsSha256":pipeline::digest(&context["relatedProps"].to_string()),
        "consumedPropDesigns":[{"assetId":prop["id"],"identityId":"watch-identity","variantId":prop["variantId"],
            "assetSemanticVersion":relation["assetSemanticVersion"],"sourceSha256":source["sha256"],
            "sharedSourceRefs":relation["sharedSourceRefs"],"designVersion":relation["acceptedDesign"]["designVersion"],
            "designReceiptSha256":"original-prop-receipt"}]});
    let mut result = output(&source, &asset);
    let proposal = &mut result["assets"][0];
    let basis = json!({
        "assetSemanticVersion":pipeline::digest(&pipeline::asset_design_contract::asset_semantics(proposal).to_string()),
        "sourceChain":pipeline::asset_design_contract::source_chain(&[asset.clone(),prop.clone()],proposal).unwrap(),
        "assetProductionContext":binding
    });
    proposal["design"] = json!({"frozenContext":{"assetProductionContext":context},
        "designReceipt":{"sha256":pipeline::digest(&basis.to_string()),"basis":basis}});
    pipeline::asset_production_context::validate_consumed_prop_designs(
        &proposal["design"],
        &[asset.clone(), prop.clone()],
    )
    .unwrap();
    let mut edited = prop;
    edited["prompt"] = json!("用户改成银色金属腕表");
    store.save_asset(edited).unwrap();
    let error = store.publish_asset_design(1, 0, &result).unwrap_err();
    assert!(error.contains("道具源/形象/设计版本已变化"), "{error}");
    assert_eq!(store.get("assets", "a").unwrap()["prompt"], "旧文字");
    assert_eq!(
        store.get("assets", "watch").unwrap()["prompt"],
        "用户改成银色金属腕表"
    );
    assert!(store.foundation().unwrap().is_null());
}

#[test]
fn consumed_outside_prop_ancestor_edit_rolls_back_character_publication() {
    let (_temp, store, source, asset) = fixture();
    let base_basis = json!({"sourceChain":[]});
    let base = store
        .save_asset(json!({
            "id":"watch-base","baseAssetId":"watch-identity","variantId":"base",
            "kind":"PROP","name":"独立物件基础形象","sourceRefs":[source["rows"][2]["id"]],
            "prompt":"基础物件固定材质",
            "design":{"sourceSha256":source["sha256"],"designReceipt":{
                "sha256":pipeline::digest(&base_basis.to_string()),"basis":base_basis}}
        }))
        .unwrap();
    let mut child = json!({
        "id":"watch-child","baseAssetId":"watch-identity","variantId":"damaged",
        "kind":"PROP","name":"同一物件破损形象","sourceAssetId":"watch-base",
        "sourceRefs":[source["rows"][2]["id"]],"prompt":"继承基础物件的破损稿"
    });
    let child_basis = json!({"sourceChain":pipeline::asset_design_contract::source_chain(&[base.clone(),child.clone()],&child).unwrap()});
    child["design"] = json!({"sourceSha256":source["sha256"],"designReceipt":{
        "sha256":pipeline::digest(&child_basis.to_string()),"basis":child_basis}});
    let child = store.save_asset(child).unwrap();
    let context = json!({
        "version":pipeline::asset_production_context::PROFILE,"sourceSha256":source["sha256"],
        "instructions":pipeline::asset_production_context::INSTRUCTIONS,
        "instructionsSha256":pipeline::digest(pipeline::asset_production_context::INSTRUCTIONS),
        "applicableScenes":[],"factEvidenceScenes":[],"factEvidenceScopes":[],"readSceneIds":[],
        "relatedProps":[{"assetId":child["id"],"identityId":"watch-identity","variantId":child["variantId"],
            "assetSemanticVersion":pipeline::digest(&pipeline::asset_design_contract::asset_semantics(&child).to_string()),
            "sharedSourceRefs":[source["rows"][2]["id"]],
            "acceptedDesign":{"designVersion":pipeline::digest(&json!({"prompt":child["prompt"],"receipt":child["design"]["designReceipt"]}).to_string()),
                "designReceiptSha256":child["design"]["designReceipt"]["sha256"]}}]
    });
    let relation = &context["relatedProps"][0];
    let binding = json!({
        "version":pipeline::asset_production_context::PROFILE,"instructionsSha256":context["instructionsSha256"],
        "sceneContextSha256":pipeline::digest(&json!({"applicableScenes":[],"factEvidenceScenes":[],"factEvidenceScopes":[],"readSceneIds":[]}).to_string()),
        "relatedPropsSha256":pipeline::digest(&context["relatedProps"].to_string()),
        "consumedPropDesigns":[{"assetId":child["id"],"identityId":"watch-identity","variantId":child["variantId"],
            "assetSemanticVersion":relation["assetSemanticVersion"],"sourceSha256":source["sha256"],
            "sharedSourceRefs":relation["sharedSourceRefs"],"designVersion":relation["acceptedDesign"]["designVersion"],
            "designReceiptSha256":child["design"]["designReceipt"]["sha256"]}]
    });
    let mut result = output(&source, &asset);
    let proposal = &mut result["assets"][0];
    let basis = json!({
        "assetSemanticVersion":pipeline::digest(&pipeline::asset_design_contract::asset_semantics(proposal).to_string()),
        "sourceChain":pipeline::asset_design_contract::source_chain(&[asset.clone(),base.clone(),child.clone()],proposal).unwrap(),
        "assetProductionContext":binding
    });
    proposal["design"] = json!({"frozenContext":{"assetProductionContext":context},
        "designReceipt":{"sha256":pipeline::digest(&basis.to_string()),"basis":basis}});
    pipeline::asset_production_context::validate_consumed_prop_designs(
        &proposal["design"],
        &[asset.clone(), base.clone(), child.clone()],
    )
    .unwrap();

    let frozen_child = store.get("assets", "watch-child").unwrap();
    let previous_character = store.get("assets", "a").unwrap();
    let previous_foundation = store.foundation().unwrap();
    let mut changed_base = base;
    changed_base["expectedVersion"] = changed_base["version"].clone();
    changed_base["prompt"] = json!("用户仅改变基础物件材质");
    store.save_asset_user(changed_base).unwrap();
    assert_eq!(store.get("assets", "watch-child").unwrap(), frozen_child);
    let error = store.publish_asset_design(1, 0, &result).unwrap_err();
    assert!(error.contains("道具祖先设计版本已变化"), "{error}");
    assert_eq!(store.get("assets", "a").unwrap(), previous_character);
    assert_eq!(store.get("assets", "watch-child").unwrap(), frozen_child);
    assert_eq!(store.foundation().unwrap(), previous_foundation);
    assert_eq!(
        store.get("assets", "watch-base").unwrap()["prompt"],
        "用户仅改变基础物件材质"
    );
}
