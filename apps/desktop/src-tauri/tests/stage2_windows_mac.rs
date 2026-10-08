//! Source-only synthetic regressions for the Windows/Mac Stage2 integration.
use async_trait::async_trait;
use director_core::{
    pipeline::{self, TextExecutor, TextRequest},
    store::Store,
};
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Default)]
struct IntegratedDesigner {
    calls: Mutex<Vec<TextRequest>>,
}
#[async_trait]
impl TextExecutor for IntegratedDesigner {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        self.calls.lock().unwrap().push(request.clone());
        if request.stage == "assetVisualFacts" {
            return Ok(
                json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":[],"issues":[]}),
            );
        }
        assert_eq!(request.stage, "assetDesign");
        assert_eq!(request.rule_snapshot["primary"]["version"], "2.0.0");
        let asset = &request.input["asset"];
        let evidence = request.input["evidence"].as_array().unwrap();
        let body = evidence.iter().find(|row| row["kind"] != "format").unwrap();
        let parent = request.input["parentAsset"]["id"].as_str();
        Ok(
            json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":format!("{}：保留身份、体态与源年龄，协调眉眼、发型轮廓、棉麻衣装。",asset["id"].as_str().unwrap()),"freeDesignScope":"仅补足未限定造型","sourceRefs":[body["id"]],"referenceVariantIds":parent.into_iter().collect::<Vec<_>>()}),
        )
    }
}

fn input(store: &Store, selected: &str) -> Value {
    json!({"source":store.source().unwrap(),"foundation":store.foundation().unwrap(),"assets":store.values("assets").unwrap(),"settings":store.settings().unwrap(),"selection":{"assetIds":[selected]},"imageProductionPolicy":"automatic"})
}

#[tokio::test]
async fn empty_fact_scope_reads_exact_scene_and_keeps_mac_design_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("synthetic scope", "fixture").unwrap();
    let source = store
        .save_source(json!(pipeline::prepare_source(
            "第一集\n场景：湖边\n△一条带毒的变异鲟鱼游过。\n第二集\n场景：屋内\n△小王穿着红衣。\n"
        )
        .unwrap()))
        .unwrap();
    let marker = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "format" && row["sceneId"].is_string())
        .unwrap();
    let body = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["text"].as_str().unwrap().contains("鲟鱼"))
        .unwrap();
    store
        .save_settings(&json!({"projectVisualStyle":"细腻CG国漫","ratio":"3:2"}))
        .unwrap();
    store.save_asset(json!({"id":"fish","baseAssetId":"fish-identity","variantId":"base","kind":"BEAST","sourceVariantId":null,"preparation":{"mode":"REFERENCE"},"sourceRefs":[marker["id"]],"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceSha256":source["sha256"]}})).unwrap();
    let frozen = input(&store, "fish");
    let executor = IntegratedDesigner::default();
    let output = pipeline::run_asset_design(&frozen, &executor, "offline", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    {
        let calls = executor.calls.lock().unwrap();
        let request = calls
            .iter()
            .find(|request| request.stage == "assetDesign")
            .unwrap();
        assert_eq!(
            request.input["sourceScopeContext"]["version"],
            "asset-scene-context/v1"
        );
        assert_eq!(request.input["imagePresentation"]["composition"], "");
        assert!(request.input["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == body["id"]
                && row["text"] == body["text"]
                && row["evidenceUses"] == json!(["SOURCE_CONTEXT"])));
        assert!(!request.input["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["text"].as_str().unwrap().contains("红衣")));
    }
    assert_eq!(
        output["assets"][0]["design"]["sourceRefs"],
        json!([body["id"]])
    );
    assert_eq!(
        output["assets"][0]["design"]["designReceipt"]["version"],
        "asset-design-context/v2"
    );
    store
        .publish_asset_design(source["version"].as_i64().unwrap(), 0, &output)
        .unwrap();
    let current = input(&store, "fish");
    assert!(!pipeline::automatic_design_required(
        &current,
        &current["assets"][0]
    ));
    let reuse = IntegratedDesigner::default();
    let replay = pipeline::run_asset_design(&current, &reuse, "offline", "medium")
        .await
        .unwrap();
    assert!(replay["designs"].as_array().unwrap().is_empty());
    assert!(reuse.calls.lock().unwrap().is_empty());
    assert_eq!(frozen["source"], source);
}

#[tokio::test]
async fn eleven_forms_preserve_direct_parents_mac_receipts_manual_design_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let store = Store::open(root.clone()).unwrap();
    store.create("synthetic chain", "fixture").unwrap();
    let source = store
        .save_source(json!(pipeline::prepare_source(
            "第一集\n场景：门口\n△六十岁的小王穿着棉衣，小李与他交谈。\n"
        )
        .unwrap()))
        .unwrap();
    let row = source["rows"][2]["id"].clone();
    store.save_settings(&json!({"projectVisualStyle":"细腻CG国漫","ratio":"3:2","productionDirection":"朴素材质完成度","productionDirectionConfirmed":true})).unwrap();
    store.save_asset(json!({"id":"form-00","baseAssetId":"person","variantId":"person:base","kind":"CH","prompt":"人工确认：六十岁、深肤色、宽颧颊、朴素棉衣。","promptOrigin":"MANUAL","sourceVariantId":null,"sourceRefs":[row],"preparation":{"mode":"REFERENCE"}})).unwrap();
    store.save_asset(json!({"id":"other-person","baseAssetId":"other","variantId":"other:base","kind":"CH","prompt":"人工对照：短圆脸、齐整短发与粗布衣装。","promptOrigin":"MANUAL","sourceRefs":[row],"preparation":{"mode":"REFERENCE"}})).unwrap();
    for level in (1..11).rev() {
        store.save_asset(json!({"id":format!("form-{level:02}"),"baseAssetId":"person","variantId":format!("person_V_{level:02}"),"kind":"CH","sourceVariantId":format!("person_V_{:02}",level-1),"sourceAssetId":format!("form-{:02}",level-1),"sourceRefs":[row],"preparation":{"mode":"DERIVE"}})).unwrap();
    }
    let manual = store.active_asset("form-00").unwrap();
    let comparison = store.active_asset("other-person").unwrap();
    let frozen = input(&store, "form-10");
    let executor = IntegratedDesigner::default();
    let output = pipeline::run_asset_design(&frozen, &executor, "offline", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    assert_eq!(output["assets"].as_array().unwrap().len(), 10);
    {
        let calls = executor.calls.lock().unwrap();
        let designs: Vec<_> = calls
            .iter()
            .filter(|request| request.stage == "assetDesign")
            .collect();
        assert_eq!(designs.len(), 10);
        for (index, request) in designs.iter().enumerate() {
            assert_eq!(
                request.input["asset"]["id"],
                format!("form-{:02}", index + 1)
            );
            assert_eq!(
                request.input["parentDesign"]["assetId"],
                format!("form-{index:02}")
            );
            assert_eq!(
                request.input["parentAsset"]["id"],
                format!("form-{index:02}")
            );
            assert_eq!(request.input["designMode"], "DERIVED");
            assert_eq!(request.input["projectVisualStyle"], "细腻CG国漫");
            assert_eq!(
                request.input["imagePresentation"]["composition"],
                pipeline::asset_design_contract::CH_PRESENTATION
            );
            assert_eq!(request.input["imagePresentation"]["ratio"], "3:2");
            assert_eq!(
                request.input["comparisonDesigns"].as_array().unwrap().len(),
                1
            );
            assert_eq!(
                request.input["comparisonDesigns"][0]["assetId"],
                "other-person"
            );
        }
    }
    store
        .publish_asset_design(source["version"].as_i64().unwrap(), 0, &output)
        .unwrap();
    let reopened = Store::open(root).unwrap();
    assert_eq!(reopened.active_asset("form-00").unwrap(), manual);
    assert_eq!(reopened.active_asset("other-person").unwrap(), comparison);
    let current = input(&reopened, "form-10");
    for asset in current["assets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|asset| asset["promptOrigin"] == "AI")
    {
        assert!(pipeline::asset_design_contract::design_receipt_current(
            &current, asset
        ));
        let prompt = pipeline::asset_design_contract::compile_image_prompt(
            asset["prompt"].as_str().unwrap(),
            "细腻CG国漫",
            &pipeline::asset_design_contract::image_presentation(&current["settings"], asset),
        );
        assert_eq!(
            prompt
                .matches(pipeline::asset_design_contract::CH_PRESENTATION)
                .count(),
            1
        );
    }
    let reuse = IntegratedDesigner::default();
    let replay = pipeline::run_asset_design(&current, &reuse, "offline", "medium")
        .await
        .unwrap();
    assert!(replay["designs"].as_array().unwrap().is_empty());
    assert!(reuse.calls.lock().unwrap().is_empty());
    let mut changed_parent = reopened.active_asset("form-00").unwrap();
    changed_parent["expectedVersion"] = changed_parent["version"].clone();
    changed_parent["prompt"] = json!("人工确认后的新棉衣剪裁。");
    reopened.save_asset(changed_parent).unwrap();
    let changed = input(&reopened, "form-10");
    let leaf = changed["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["id"] == "form-10")
        .unwrap();
    assert!(!pipeline::asset_design_contract::design_receipt_current(
        &changed, leaf
    ));
    assert_eq!(
        frozen["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["id"] == "form-00")
            .unwrap(),
        &manual
    );
}
