//! The normal Stage1 projection may leave base description empty. Stage2 reads
//! its exact registered identity's production note without changing saved rows.
use async_trait::async_trait;
use director_core::{
    pipeline::{
        self, asset_design_contract as contract, FoundationProposal, TextExecutor, TextRequest,
    },
    store::Store,
    workflow::guard_from_input,
};
use serde_json::{json, Value};
use std::sync::Mutex;

const IDENTITY_DESCRIPTION: &str = "甲为村里的维修工；衣装和当前状态须按本场原文处理。";
const CHILD_DESCRIPTION: &str = "本场换装形象的独立说明，不覆盖基础身份。";

fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store
        .create("synthetic identity material", "fixture")
        .unwrap();
    let source = store
        .save_source(json!(pipeline::prepare_source(
            "第一集\n场景：门口\n△甲站在门边。\n△甲换上蓝色外衣。\n"
        )
        .unwrap()))
        .unwrap();
    let place =
        json!({"episodeId":source["episodes"][0]["id"],"sceneId":source["scenes"][0]["id"]});
    let proposal: FoundationProposal = serde_json::from_value(json!({
        "schemaVersion":"foundation/v1","sourceSha256":source["sha256"],
        "overview":"合成制作资料","direction":"","episodes":[{
            "episodeId":source["episodes"][0]["id"],"summary":"甲换装。","context":"",
            "sourceRefs":[source["rows"][2]["id"],source["rows"][3]["id"]]
        }],"assets":[{"id":"person","name":"甲","kind":"CH","aliases":[],
            "description":IDENTITY_DESCRIPTION,"sourceRefs":[source["rows"][2]["id"]],
            "variants":[
                {"id":"base","assetRowId":"person-base","name":"基础","description":"",
                 "sourceVariantId":null,"sourceRefs":[source["rows"][2]["id"]],
                 "applicability":[place],"scope":{"kind":"SCENES","scenes":[place],"reason":"基础身份在本场实际出现。"},"preparation":{"mode":"REFERENCE"}},
                {"id":"blue","assetRowId":"person-blue","name":"蓝衣","description":CHILD_DESCRIPTION,
                 "sourceVariantId":"base","sourceRefs":[source["rows"][3]["id"]],
                 "applicability":[place],"scope":{"kind":"SCENES","scenes":[place],"reason":"本场明确更换外衣。"},"preparation":{"mode":"DERIVE"}}
            ]}],"facts":[],"issues":[],"updatedEpisodeIds":[source["episodes"][0]["id"]]
    })).unwrap();
    let locked = serde_json::from_value(source.clone()).unwrap();
    pipeline::validate_foundation(&locked, &proposal).unwrap();
    let rows = pipeline::flatten_foundation_assets(&proposal);
    assert_eq!(
        rows[0]["description"], "",
        "reproduces the real projection boundary"
    );
    let input = json!({"source":source,"foundation":store.foundation().unwrap(),"assets":[],"selection":{"episodeIds":[source["episodes"][0]["id"]]}});
    store
        .publish_foundation(&guard_from_input(&input), json!(proposal), rows, None)
        .unwrap();
    (temp, store)
}

fn raw_input(store: &Store) -> Value {
    json!({"source":store.source().unwrap(),"foundation":store.foundation().unwrap(),
        "assets":store.values("assets").unwrap(),"settings":store.settings().unwrap(),
        "selection":{"assetIds":["person-blue"]},"imageProductionPolicy":"automatic"})
}

fn new_input(store: &Store) -> Value {
    let mut input = raw_input(store);
    contract::freeze_identity_production_materials(&mut input).unwrap();
    input
}

#[derive(Default)]
struct Designer {
    requests: Mutex<Vec<TextRequest>>,
}

#[async_trait]
impl TextExecutor for Designer {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        self.requests.lock().unwrap().push(request.clone());
        if request.stage == "assetVisualFacts" {
            return Ok(
                json!({"schemaVersion":"asset-visual-facts-wire/v1", "coveredPacketRef":request.input["coveredPacketRef"],"facts":[],"issues":[]}),
            );
        }
        assert_eq!(request.stage, "assetDesign");
        assert_eq!(request.rule_snapshot["primary"]["version"], "2.0.0");
        let asset = &request.input["asset"];
        let reference = request.input["parentAsset"]["id"].as_str();
        Ok(
            json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],
            "prompt":"合成完整形象设计。","freeDesignScope":"普通美术补足。",
            "sourceRefs":[asset["sourceRefs"][0]],"referenceVariantIds":reference.into_iter().collect::<Vec<_>>() }),
        )
    }
}

async fn assert_normal_chain_preserves_description(user_description: Option<&str>) {
    let (_temp, store) = fixture();
    if let Some(description) = user_description {
        let mut asset = store.active_asset("person-base").unwrap();
        asset["description"] = json!(description);
        store.save_asset(asset).unwrap();
    }
    let rows_before = store.values("assets").unwrap();
    let input = new_input(&store);
    let frozen = input.to_string();
    let executor = Designer::default();
    let output = pipeline::run_asset_design(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    assert_eq!(
        output["facts"],
        json!([]),
        "identity text is never copied into facts"
    );
    assert_eq!(output["assets"].as_array().unwrap().len(), 2);
    {
        let requests = executor.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            3,
            "one A15 source pass, then the exact base and child"
        );
        let visual = requests
            .iter()
            .find(|request| request.stage == "assetVisualFacts")
            .unwrap();
        assert_eq!(
            visual.input[contract::IDENTITY_MATERIALS_FIELD]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            visual.input[contract::IDENTITY_MATERIALS_FIELD][0]["identityId"],
            "person"
        );
        assert_eq!(
            visual.input[contract::IDENTITY_MATERIALS_FIELD][0]["description"],
            IDENTITY_DESCRIPTION
        );
        assert_eq!(
            visual.input["identityProductionMaterialPolicy"],
            contract::IDENTITY_MATERIAL_POLICY
        );
        assert!(visual.input["identities"][0].get("description").is_none());
        for request in requests
            .iter()
            .filter(|request| request.stage == "assetDesign")
        {
            let asset = &request.input["asset"];
            assert_eq!(
                asset["identityProductionMaterial"]["description"],
                IDENTITY_DESCRIPTION
            );
            assert_eq!(
                asset["identityProductionMaterial"]["provenance"],
                "FOUNDATION_IDENTITY_DESCRIPTION_NOT_VISUAL_FACT"
            );
            assert_eq!(request.input["facts"], json!([]));
            assert_eq!(request.input["deltaFacts"], json!([]));
            if asset["id"] == "person-base" {
                assert_eq!(asset["description"], user_description.unwrap_or(""));
                assert!(request.input["parentAsset"].is_null());
            } else {
                assert_eq!(asset["description"], CHILD_DESCRIPTION);
                assert_eq!(request.input["parentAsset"]["id"], "person-base");
                assert_eq!(
                    request.input["parentAsset"]["description"],
                    user_description.unwrap_or("")
                );
                assert_eq!(
                    request.input["parentAsset"]["identityProductionMaterial"],
                    asset["identityProductionMaterial"]
                );
            }
        }
    }
    assert_eq!(input.to_string(), frozen);
    assert_eq!(
        store.values("assets").unwrap(),
        rows_before,
        "reading/freeze does not edit rows"
    );
    store
        .publish_asset_design(
            input["source"]["version"].as_i64().unwrap(),
            input["foundation"]["version"].as_i64().unwrap(),
            &output,
        )
        .unwrap();
    let reopened = Store::open(store.root.clone()).unwrap();
    assert_eq!(
        reopened.active_asset("person-base").unwrap()["description"],
        user_description.unwrap_or("")
    );
    assert_eq!(
        reopened.active_asset("person-blue").unwrap()["description"],
        CHILD_DESCRIPTION
    );
    let current = new_input(&reopened);
    for asset in current["assets"].as_array().unwrap() {
        assert!(contract::design_receipt_current(&current, asset));
        assert_eq!(
            asset["design"]["frozenContext"]["assetIdentityProductionMaterial"]["description"],
            IDENTITY_DESCRIPTION
        );
    }
    let replay = Designer::default();
    let output = pipeline::run_asset_design(&current, &replay, "offline", "medium")
        .await
        .unwrap();
    assert!(output["designs"].as_array().unwrap().is_empty());
    assert!(
        replay.requests.lock().unwrap().is_empty(),
        "unchanged save/reopen sends no new calls"
    );
}

#[tokio::test]
async fn empty_base_description_reads_identity_material_through_normal_projection_and_restart() {
    assert_normal_chain_preserves_description(None).await;
}

#[tokio::test]
async fn user_and_variant_descriptions_are_retained_beside_identity_material() {
    assert_normal_chain_preserves_description(Some("用户另写的制作说明，不得由基础身份文本覆盖。"))
        .await;
}

#[tokio::test]
async fn adopted_manual_prompts_are_not_regenerated_when_new_identity_material_becomes_available() {
    let (_temp, store) = fixture();
    for mut asset in store.values("assets").unwrap() {
        asset["prompt"] = json!("人工确认的历史形象，按用户采用保留。");
        asset["promptOrigin"] = json!("MANUAL");
        store.save_asset(asset).unwrap();
    }
    let before = store.values("assets").unwrap();
    let input = new_input(&store);
    let executor = Designer::default();
    let output = pipeline::run_asset_design(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    assert!(output["designs"].as_array().unwrap().is_empty());
    assert!(executor.requests.lock().unwrap().is_empty());
    assert_eq!(store.values("assets").unwrap(), before);
}

#[tokio::test]
async fn missing_flat_scope_reason_reaches_a15_and_design_cards_without_editing_the_manual_parent()
{
    let (_temp, store) = fixture();
    for mut asset in store.values("assets").unwrap() {
        asset["scope"]["reason"] = Value::Null;
        if asset["id"] == "person-base" {
            asset["prompt"] = json!("人工已采用父形象。");
            asset["promptOrigin"] = json!("MANUAL");
        }
        store.save_asset(asset).unwrap();
    }
    let parent = store.active_asset("person-base").unwrap();
    let input = new_input(&store);
    assert_eq!(
        input[contract::SCOPE_MATERIALS_FIELD]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert!(input["assets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|asset| asset["scope"]["reason"].is_null()));
    let executor = Designer::default();
    let output = pipeline::run_asset_design(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    {
        let requests = executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "manual parent is not redesigned");
        let visual = &requests[0];
        assert_eq!(visual.stage, "assetVisualFacts");
        let scope_material = &visual.input[contract::SCOPE_MATERIALS_FIELD][0];
        assert_eq!(scope_material["assetId"], "person-blue");
        assert_eq!(scope_material["scope"]["reason"], "本场明确更换外衣。");
        assert_eq!(
            scope_material["provenance"],
            "FOUNDATION_SCOPE_REASON_NOT_VISUAL_FACT"
        );
        assert_eq!(
            scope_material["packetScope"]["scenes"][0]["sceneId"],
            visual.input["identities"][0]["versions"][0]["applicability"]["scenes"][0]
        );
        let design = &requests[1];
        assert_eq!(
            design.input["asset"]["scope"]["reason"],
            "本场明确更换外衣。"
        );
        assert_eq!(
            design.input["parentAsset"]["scope"]["reason"],
            "基础身份在本场实际出现。"
        );
        assert_eq!(
            design.input["asset"]["scopeProductionMaterial"]["assetId"],
            "person-blue"
        );
        assert_eq!(design.input["facts"], json!([]));
        assert_eq!(design.input["deltaFacts"], json!([]));
    }
    assert_eq!(output["assets"].as_array().unwrap().len(), 1);
    assert!(
        output["assets"][0]["scope"]["reason"].is_null(),
        "only model cards receive the projected reason"
    );
    store
        .publish_asset_design(
            input["source"]["version"].as_i64().unwrap(),
            input["foundation"]["version"].as_i64().unwrap(),
            &output,
        )
        .unwrap();
    assert_eq!(
        store.active_asset("person-base").unwrap(),
        parent,
        "parent hashes and saved prose stay exact"
    );
    let reopened = Store::open(store.root.clone()).unwrap();
    let current = new_input(&reopened);
    let child = current["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["id"] == "person-blue")
        .unwrap();
    assert!(
        contract::design_receipt_current(&current, child),
        "prompt-only counters do not invalidate scope materials"
    );
    let reuse = Designer::default();
    let result = pipeline::run_asset_design(&current, &reuse, "offline", "medium")
        .await
        .unwrap();
    assert!(result["blocked"].as_array().unwrap().is_empty());
    assert!(reuse.requests.lock().unwrap().is_empty());
    // The original new context may carry a reason recovered from historical
    // evidence while current stored foundation still has the legacy null.
    let mut legacy_foundation = current.clone();
    for variant in legacy_foundation["foundation"]["assets"][0]["variants"]
        .as_array_mut()
        .unwrap()
    {
        variant["scope"]["reason"] = Value::Null;
    }
    assert!(contract::validate_resume_facts(&input, &output, &legacy_foundation).is_ok());
    let mut edited = current.clone();
    edited["foundation"]["assets"][0]["variants"][1]["scope"]["reason"] =
        json!("用户已改变本形象制作取舍。");
    assert!(contract::validate_resume_facts(&input, &output, &edited).is_err());
}

#[tokio::test]
async fn another_identity_with_the_same_display_name_never_supplies_a_fallback_description() {
    let (_temp, store) = fixture();
    let mut input = raw_input(&store);
    input["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|asset| asset["id"] == "person-base");
    input["assets"][0]["identityId"] = json!("other");
    input["assets"][0]["baseAssetId"] = json!("other");
    input["assets"][0]["scope"]["reason"] = Value::Null;
    input["selection"]["assetIds"] = json!(["person-base"]);
    contract::freeze_identity_production_materials(&mut input).unwrap();
    assert!(input[contract::IDENTITY_MATERIALS_FIELD]
        .as_object()
        .unwrap()
        .is_empty());
    let executor = Designer::default();
    let output = pipeline::run_asset_design(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    let requests = executor.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in &*requests {
        assert!(request
            .input
            .get("identityProductionMaterialPolicy")
            .is_none());
        assert!(request.input["asset"]
            .get("identityProductionMaterial")
            .is_none());
        assert!(request
            .input
            .get(contract::IDENTITY_MATERIALS_FIELD)
            .is_none());
    }
}

#[tokio::test]
async fn legacy_frozen_requests_and_accepted_a15_cache_do_not_gain_or_resend_identity_material() {
    let (_temp, store) = fixture();
    let legacy = raw_input(&store);
    let original_bytes = legacy.to_string();
    let first = Designer::default();
    let output = pipeline::run_asset_design(&legacy, &first, "offline", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    let second = Designer::default();
    pipeline::run_asset_design(&legacy, &second, "offline", "medium")
        .await
        .unwrap();
    let bytes = |executor: &Designer| {
        executor
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| serde_json::to_string(request).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        bytes(&first),
        bytes(&second),
        "legacy request/fingerprint reconstruction remains exact"
    );
    assert!(!original_bytes.contains(contract::IDENTITY_MATERIAL_PROFILE_FIELD));
    assert_eq!(legacy.to_string(), original_bytes);
    store
        .publish_asset_design(
            legacy["source"]["version"].as_i64().unwrap(),
            legacy["foundation"]["version"].as_i64().unwrap(),
            &output,
        )
        .unwrap();
    let current = new_input(&store);
    let old_receipts = current["foundation"]["visualFactBatches"].clone();
    let reuse = Designer::default();
    let result =
        pipeline::asset_visual_facts::run_asset_visual_facts(&current, &reuse, "offline", "medium")
            .await
            .unwrap();
    assert!(result["blocked"].as_array().unwrap().is_empty(), "{result}");
    assert_eq!(result["visualFactBatches"], old_receipts);
    assert!(
        reuse.requests.lock().unwrap().is_empty(),
        "identity addendum does not rerun a source-only accepted A15 packet"
    );
    assert!(
        contract::identity_production_material(&legacy, &legacy["assets"][0])
            .unwrap()
            .is_none()
    );
}

#[test]
fn related_identity_material_versions_invalidate_design_and_resume_without_mutating_the_snapshot() {
    let (_temp, store) = fixture();
    let original = new_input(&store);
    let bytes = original.to_string();
    let mut designed = original.clone();
    for index in 0..designed["assets"].as_array().unwrap().len() {
        let receipt = contract::design_receipt(&original, &original["assets"][index]).unwrap();
        designed["assets"][index]["design"] = json!({"designReceipt":receipt});
    }
    let mut unrelated = designed.clone();
    unrelated["foundation"]["assets"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"another","name":"甲","description":"同名其它主体的制作说明"}));
    contract::freeze_identity_production_materials(&mut unrelated).unwrap();
    assert!(contract::design_receipt_current(
        &unrelated,
        &unrelated["assets"][0]
    ));
    assert!(contract::validate_resume_facts(&original, &json!({}), &unrelated).is_ok());
    let mut changed = designed.clone();
    changed["foundation"]["assets"][0]["description"] = json!("用户更新的主体制作说明。");
    contract::freeze_identity_production_materials(&mut changed).unwrap();
    assert!(!contract::design_receipt_current(
        &changed,
        &changed["assets"][0]
    ));
    assert!(!contract::design_receipt_current(
        &changed,
        &changed["assets"][1]
    ));
    assert!(
        contract::validate_resume_facts(&original, &json!({}), &changed)
            .unwrap_err()
            .contains("主体制作说明")
    );
    assert_eq!(original.to_string(), bytes);
}

#[test]
fn invalid_identity_or_source_material_is_rejected_without_overwriting_asset_rows() {
    let (_temp, store) = fixture();
    let original = new_input(&store);
    let mut wrong = original.clone();
    wrong[contract::IDENTITY_MATERIALS_FIELD]["person"]["identityId"] = json!("other");
    assert!(contract::identity_production_material(&wrong, &wrong["assets"][0]).is_err());
    let mut wrong_source = raw_input(&store);
    wrong_source["foundation"]["sourceSha256"] = json!("different-locked-source");
    let bytes = wrong_source.to_string();
    assert!(contract::freeze_identity_production_materials(&mut wrong_source).is_err());
    assert_eq!(wrong_source.to_string(), bytes);
    let mut duplicate = raw_input(&store);
    let repeated = duplicate["foundation"]["assets"][0].clone();
    duplicate["foundation"]["assets"]
        .as_array_mut()
        .unwrap()
        .push(repeated);
    assert!(contract::freeze_identity_production_materials(&mut duplicate).is_err());
    let mut conflicting = raw_input(&store);
    conflicting["assets"][0]["identityId"] = json!("other");
    assert!(contract::freeze_identity_production_materials(&mut conflicting).is_err());
    assert_eq!(
        store.values("assets").unwrap(),
        original["assets"].as_array().unwrap().clone()
    );
}
