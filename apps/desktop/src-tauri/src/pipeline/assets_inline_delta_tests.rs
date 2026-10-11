use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Designer(Mutex<Vec<Value>>);
#[async_trait]
impl TextExecutor for Designer {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        if request.stage == "assetVisualFacts" {
            return Ok(json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":[],"issues":[]}));
        }
        self.0.lock().unwrap().push(request.input.clone());
        let a = &request.input["asset"];
        let parent = request.input["parentAsset"]["id"].as_str().into_iter().collect::<Vec<_>>();
        Ok(json!({"schemaVersion":"asset-design/v1","assetId":a["id"],"variantId":a["variantId"],"prompt":"合成离线设计","freeDesignScope":"中性背景","sourceRefs":a["sourceRefs"],"referenceVariantIds":parent}))
    }
}

fn fixture(two: bool) -> Value {
    let source = prepare_source("第一集\n场景：门口\n△甲黑发白衣，换蓝衣戴帽后右手受伤，衣服改为红色。\n").unwrap();
    let row = &source.rows[2].id;
    let mut assets = vec![
        json!({"id":"base","baseAssetId":"person","variantId":"base","prompt":"手写白衣基础","promptOrigin":"MANUAL","sourceRefs":[row],"adoptedMediaId":"base-image"}),
        json!({"id":"blue","baseAssetId":"person","variantId":"blue","sourceAssetId":"base","preparation":{"mode":"INLINE"},"sourceRefs":[row]}),
    ];
    if two {
        assets.push(json!({"id":"hat","baseAssetId":"person","variantId":"hat","sourceAssetId":"blue","preparation":{"mode":"INLINE"},"sourceRefs":[row]}));
    }
    assets.push(json!({"id":"child","baseAssetId":"person","variantId":"hurt","sourceAssetId":if two {"hat"} else {"blue"},"sourceRefs":[row]}));
    let f = |id: &str, owner: &str, variant: Option<&str>, attribute: &str, value: &str| {
        json!({"id":id,"assetId":owner,"variantId":variant,"attribute":attribute,"value":value,"scope":if variant.is_none(){"STABLE"}else{"STATE"},"claim":"SOURCE","sourceRefs":[row],"quotes":["原文引用不复制"]})
    };
    let mut facts = vec![f("hair","person",None,"hair","黑发"),f("white","person",Some("base"),"clothes","白衣"),f("blue","person",Some("blue"),"clothes","蓝衣"),f("hurt","person",Some("hurt"),"hand","伤口")];
    if two { facts.push(f("hat","person",Some("hat"),"headwear","帽子")); }
    facts.push(f("future","person",Some("future"),"clothes","未来绿衣"));
    facts.push(f("other","other",Some("blue"),"clothes","他人服装"));
    json!({"source":source,"assets":assets,"selection":{"assetIds":["child"]},"foundation":{"facts":facts}})
}

fn ids(value: &Value) -> Vec<&str> {
    value.as_array().unwrap().iter().map(|v|text(v,"id")).collect()
}

async fn run(input: &Value) -> (Value, Value) {
    let designer = Designer::default();
    let output = run_asset_design(input,&designer,"offline","medium").await.unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(),"{output}");
    let requests = designer.0.lock().unwrap();
    assert_eq!(requests.len(),1);
    (requests[0].clone(),output)
}

#[tokio::test]
async fn inline_delta_one_and_two_layers_include_only_effective_changes_and_direct_reference() {
    for two in [false,true] {
        let mut input = fixture(two);
        freeze_inline_delta_profile(&mut input);
        let before = input.clone();
        let (request,output) = run(&input).await;
        let delta = ids(&request["deltaFacts"]);
        assert!(delta.contains(&"blue") && delta.contains(&"hurt"));
        assert_eq!(delta.contains(&"hat"),two);
        assert!(!delta.contains(&"white") && !delta.contains(&"hair") && !delta.contains(&"future") && !delta.contains(&"other"));
        assert_eq!(request["parentDesign"]["assetId"],"base");
        assert_eq!(request["parentAsset"]["id"],if two {"hat"}else{"blue"});
        assert_eq!(request["inlineReference"]["version"],INLINE_DELTA_PROFILE);
        assert_eq!(output["designs"][0]["referenceAssetIds"],json!([if two {"hat"}else{"blue"}]));
        assert_eq!(output["designs"][0]["frozenContext"]["inlineDeltaContext"]["version"],INLINE_DELTA_PROFILE);
        assert_eq!(output["designs"][0]["designReceipt"]["basis"]["inlineDeltaContext"]["version"],INLINE_DELTA_PROFILE);
        assert_eq!(output["imagePlan"][0]["status"],"ready");
        assert_eq!(input,before,"new design cannot rewrite original input");
    }
}

#[tokio::test]
async fn inline_delta_current_attribute_overrides_intermediate_state() {
    let mut input = fixture(true);
    let row = input["source"]["rows"][2]["id"].clone();
    input["foundation"]["facts"].as_array_mut().unwrap().push(json!({"id":"red","assetId":"person","variantId":"hurt","attribute":"clothes","value":"红衣","scope":"STATE","claim":"SOURCE","sourceRefs":[row]}));
    freeze_inline_delta_profile(&mut input);
    let (request,_) = run(&input).await;
    let delta = ids(&request["deltaFacts"]);
    assert!(delta.contains(&"red") && delta.contains(&"hat") && delta.contains(&"hurt"));
    assert!(!delta.contains(&"blue") && !delta.contains(&"white"));
}

#[tokio::test]
async fn inline_delta_legacy_frozen_input_and_normal_derived_contract_remain_unchanged() {
    let legacy = fixture(true);
    let before = legacy.clone();
    let (request,output) = run(&legacy).await;
    assert_eq!(ids(&request["deltaFacts"]),vec!["hurt"]);
    assert!(request["inlineReference"].get("version").is_none());
    assert!(output["designs"][0]["frozenContext"].get("inlineDeltaContext").is_none());
    assert!(output["designs"][0]["designReceipt"]["basis"].get("inlineDeltaContext").is_none());
    assert_eq!(legacy,before);
    let mut normal = fixture(false);
    normal["assets"][1]["preparation"] = json!({"mode":"REFERENCE"});
    normal["assets"][1]["prompt"] = json!("手写蓝衣");
    normal["assets"][1]["promptOrigin"] = json!("MANUAL");
    let (original,old_output) = run(&normal).await;
    freeze_inline_delta_profile(&mut normal);
    let (new,new_output) = run(&normal).await;
    assert_eq!(original,new,"normal non-INLINE model input must be byte-value stable");
    assert_eq!(old_output["designs"],new_output["designs"]);
    assert_eq!(ids(&new["deltaFacts"]),vec!["hurt"]);
}

#[test]
fn inline_delta_wrong_subject_unknown_source_and_unknown_profile_never_enter_delta() {
    let mut input = fixture(false);
    freeze_inline_delta_profile(&mut input);
    input["assets"][1]["baseAssetId"] = json!("other");
    let compute = |input: &Value| {
        let assets = input["assets"].as_array().unwrap();
        let by_id = assets.iter().map(|a|(text(a,"id"),a)).collect();
        let parents = assets.iter().map(|a|(text(a,"id").to_owned(),resolve_asset_dependency(assets,a).unwrap())).collect();
        let known = input["source"]["rows"].as_array().unwrap().iter().filter_map(|r|r["id"].as_str()).collect();
        design_facts(input,assets.last().unwrap(),&by_id,&parents,&known)
    };
    let (_,delta) = compute(&input).unwrap();
    assert_eq!(ids(&json!(delta)),vec!["hurt"]);
    input["assets"][1]["baseAssetId"] = json!("person");
    input["foundation"]["facts"][2]["sourceRefs"] = json!(["unknown-source"]);
    assert!(compute(&input).unwrap_err().contains("锁定原文"));
    input[INLINE_DELTA_PROFILE_FIELD] = json!("unknown-version");
    assert!(compute(&input).unwrap_err().contains("合同版本无效"));
}

#[tokio::test]
async fn inline_delta_stops_at_actual_non_inline_design_not_older_inline_ancestor() {
    let mut input = fixture(true);
    input["assets"][2]["preparation"] = json!({"mode":"REFERENCE"});
    input["assets"][2]["prompt"] = json!("手写蓝衣戴帽父稿");
    input["assets"][2]["promptOrigin"] = json!("MANUAL");
    let row = input["source"]["rows"][2]["id"].clone();
    let assets = input["assets"].as_array_mut().unwrap();
    assets.last_mut().unwrap()["sourceAssetId"] = json!("coverage");
    assets.insert(3,json!({"id":"coverage","baseAssetId":"person","variantId":"coverage","sourceAssetId":"hat","preparation":{"mode":"INLINE"},"sourceRefs":[row]}));
    input["foundation"]["facts"].as_array_mut().unwrap().push(json!({"id":"coverage","assetId":"person","variantId":"coverage","attribute":"sleeve","value":"袖口破损","scope":"STATE","claim":"SOURCE","sourceRefs":[row]}));
    freeze_inline_delta_profile(&mut input);
    let (request,_) = run(&input).await;
    assert_eq!(request["parentDesign"]["assetId"],"hat");
    assert_eq!(request["inlineReference"]["effectiveInlineAssetIds"],json!(["coverage"]));
    let delta = ids(&request["deltaFacts"]);
    assert!(delta.contains(&"coverage") && delta.contains(&"hurt"));
    assert!(!delta.contains(&"blue") && !delta.contains(&"hat") && !delta.contains(&"white"));
}

#[tokio::test]
async fn inline_delta_receipt_prevents_old_cache_reuse_without_touching_active_jobs() {
    let mut input = fixture(false);
    let (_,legacy) = run(&input).await;
    input["assets"].as_array_mut().unwrap().last_mut().unwrap().clone_from(&legacy["assets"][0]);
    assert!(!automatic_design_stale(&input,input["assets"].as_array().unwrap().last().unwrap()));
    freeze_inline_delta_profile(&mut input);
    let row = input["assets"].as_array().unwrap().last().unwrap();
    assert!(automatic_design_stale(&input,row));
    input["activeImageAssetIds"] = json!(["child"]);
    assert!(!automatic_design_required(&input,input["assets"].as_array().unwrap().last().unwrap()));
    input["activeImageAssetIds"] = json!([]);
    let (_,fresh) = run(&input).await;
    input["assets"].as_array_mut().unwrap().last_mut().unwrap().clone_from(&fresh["assets"][0]);
    assert!(!automatic_design_stale(&input,input["assets"].as_array().unwrap().last().unwrap()));
}

#[tokio::test]
async fn historical_inline_ai_donor_requires_exact_receipt_and_retains_prompt() {
    let mut input = fixture(false);
    input["assets"][1]["prompt"] = json!("历史已接受蓝衣");
    input["assets"][1]["promptOrigin"] = json!("AI");
    let receipt = super::super::asset_design_contract::design_receipt(&input,&input["assets"][1]).unwrap();
    input["assets"][1]["design"] = json!({"schemaVersion":"asset-design/v1","assetId":"blue","variantId":"blue","prompt":"历史已接受蓝衣","sourceSha256":input["source"]["sha256"],"sourceRefs":input["assets"][1]["sourceRefs"],"referenceVariantIds":["base"],"referenceAssetIds":["base"],"freeDesignScope":"中性背景","designReceipt":receipt});
    let before = input.clone();
    let (request,_) = run(&input).await;
    assert_eq!(request["parentDesign"]["assetId"],"blue");
    assert_eq!(request["parentDesign"]["origin"],"AI");
    assert_eq!(input,before);
    input["assets"][1]["design"]["designReceipt"]["sha256"] = json!("tampered");
    let designer = Designer::default();
    let result = run_asset_design(&input,&designer,"offline","medium").await.unwrap();
    assert!(!result["blocked"].as_array().unwrap().is_empty());
    assert!(designer.0.lock().unwrap().is_empty());
    assert_eq!(input["assets"][1]["prompt"],"历史已接受蓝衣");
}

#[tokio::test]
async fn design_only_plan_uses_real_inline_image_donor_and_reports_missing_adoption() {
    let mut input = fixture(true);
    let (_,ready) = run(&input).await;
    assert_eq!(ready["imagePlan"][0]["dependencies"],json!(["hat"]));
    assert_eq!(ready["imagePlan"][0]["status"],"ready");
    input["assets"][0]["adoptedMediaId"] = json!("");
    let (_,waiting) = run(&input).await;
    assert_eq!(waiting["imagePlan"][0]["status"],"waitingAdoption");
    assert_eq!(waiting["imagePlan"][0]["missingAdoption"],json!(["hat"]));
    input["assets"][1]["adoptedMediaId"] = json!("inline-manual-media");
    input["assets"][1]["prompt"] = json!("手写蓝衣");
    input["assets"][1]["promptOrigin"] = json!("MANUAL");
    let (_,ready) = run(&input).await;
    assert_eq!(ready["imagePlan"][0]["status"],"ready");
}

#[tokio::test]
async fn inline_delta_bad_dependency_chains_block_locally_without_holding_independent_branch() {
    let mut input = fixture(false);
    let row = input["source"]["rows"][2]["id"].clone();
    input["assets"].as_array_mut().unwrap().extend([
        json!({"id":"cycle-a","variantId":"a","sourceAssetId":"cycle-b","preparation":{"mode":"INLINE"},"sourceRefs":[row]}),
        json!({"id":"cycle-b","variantId":"b","sourceAssetId":"cycle-a","sourceRefs":[row]}),
        json!({"id":"missing","variantId":"m","sourceAssetId":"not-registered","sourceRefs":[row]}),
        json!({"id":"free","variantId":"base","sourceRefs":[row]}),
    ]);
    input["selection"]["assetIds"] = json!(["child","cycle-b","missing","free"]);
    freeze_inline_delta_profile(&mut input);
    let designer = Designer::default();
    let output = run_asset_design(&input,&designer,"offline","medium").await.unwrap();
    let requests = designer.0.lock().unwrap();
    assert_eq!(requests.len(),2);
    assert!(requests.iter().any(|r|r["asset"]["id"]=="free"));
    assert!(requests.iter().any(|r|r["asset"]["id"]=="child"));
    assert!(output["blocked"].as_array().unwrap().iter().any(|r|r["assetId"]=="cycle-b"));
    assert!(output["blocked"].as_array().unwrap().iter().any(|r|r["assetId"]=="missing"));
    assert!(!output["blocked"].as_array().unwrap().iter().any(|r|r["assetId"]=="free"));
}
