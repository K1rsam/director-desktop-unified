use super::*;
use async_trait::async_trait;
use std::sync::Mutex;
#[path = "asset_workflow_costume_tests.rs"]
mod costume_layout;

#[derive(Default)]
struct Recorder {
    requests: Mutex<Vec<TextRequest>>,
    accepted: Mutex<Vec<Value>>,
    rejected: Mutex<Vec<Value>>,
    failure: Option<(&'static str, &'static str)>,
}
#[async_trait]
impl TextExecutor for Recorder {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        self.requests.lock().unwrap().push(request.clone());
        let asset = &request.input["asset"];
        if self.failure == Some((s(asset, "id"), "unknown")) {
            return Err("CODEX_SUBMISSION_UNKNOWN: original request retained".into());
        }
        let mut value = json!({"assetId":asset["id"],"variantId":asset["variantId"],
            "promptText":format!("{}：保留已定身份与源约束，明确结构、材质、姿态和实际用途。",s(asset,"name")),
            "designSupplement":if asset["adoptedDesign"].get("full").is_some(){""}else{"依据当前分镜补足未锁定的中性材质与轮廓，未改身份或用途。"},
            "layoutKind":request.input["imagePresentation"]["layoutKind"],
            "sourceRefs":array(&request.input,"evidence").iter().map(|v|v["id"].clone()).collect::<Vec<_>>(),
            "imageEvidence":"NO_PIXELS_READ","unresolved":[]});
        if let Some((id, mode)) = self.failure {
            if s(asset, "id") == id {
                match mode {
                    "identity" => value["assetId"] = json!("another-person"),
                    "pixels" => value["imageEvidence"] = json!("PIXELS_REVIEWED"),
                    "reference" => value["sourceRefs"] = json!(["future-source"]),
                    "unresolved" => {
                        value["unresolved"] = json!(["该分镜未说明两位同名人物对应的身份"])
                    }
                    "nonhuman" => value["layoutKind"] = json!("nonhuman"),
                    _ => {}
                }
            }
        }
        if request.input["assetPromptRequestVersion"] == REQUEST_V2 {
            value["mediaPlan"] = json!({"version":MEDIA_PLAN_VERSION,
                "mode":if array(&request.input,"referenceRequirements").is_empty(){"plan"}else{"preserve"},
                "imageTask":{"kind":value["layoutKind"]},"referenceRequirements":[],
                "rationale":"按当前明确媒体需求保留计划；独立项没有需要额外垫图的证据。"});
        }
        Ok(value)
    }
    async fn accept(&self, _: &TextRequest, value: &Value) -> Result<(), String> {
        self.accepted.lock().unwrap().push(value.clone());
        Ok(())
    }
    async fn reject(&self, _: &TextRequest, value: &Value, _: &str) -> Result<(), String> {
        self.rejected.lock().unwrap().push(value.clone());
        Ok(())
    }
}

fn fixture() -> Value {
    let text="第1集\n1-1\n人物：甲默认状态、甲西装状态\n场景：门厅日间\n甲换上西装走入门厅，门在画面左侧，窗在右侧。\n";
    let evidence = json!([{"id":"md:1","text":text,"start":0,"end":text.len(),"assetIds":["base","suit","hall"]}]);
    let assets = json!([
        {"id":"base","variantId":"base-v1","name":"甲默认状态","kind":"character","baseAssetId":"person-a","sourceRefs":["md:1"],"prompt":"保留旧手工提示词","promptOrigin":"MANUAL"},
        {"id":"suit","variantId":"suit-v1","name":"甲西装状态","kind":"character","baseAssetId":"person-a","sourceRefs":["md:1"],"referenceRequirements":[{"targetAssetId":"base","kind":"identity","mustPreserve":["face"],"mayChange":["clothes"]}],"imageTask":{"kind":"costume"}},
        {"id":"hall","variantId":"hall-v1","name":"门厅日间","kind":"scene","sourceRefs":["md:1"]}
    ]);
    let items=assets.as_array().unwrap().iter().map(|a|json!({"assetId":a["id"],"variantId":a["variantId"],"filename":format!("{}.png",s(a,"id")),"uses":[{"episode":1,"sceneId":"1-1","purpose":"入画"}]})).collect::<Vec<_>>();
    json!({"workflowProfile":PROFILE,"imageSource":{"sourceKind":"imported-md","documentId":digest(text),"text":text,"sha256":digest(text),"evidence":evidence},"assets":assets,"imageQueue":{"version":"desktop-image-queue/v1","items":items},"settings":{"visualStyle":"3D国漫","ratio":"16:9"},"visualDirection":{"version":"direction-1","style":"3D国漫"}})
}

#[test]
fn source_kind_keeps_imported_md_separate_from_the_original_script() {
    let input = fixture();
    let source = &input["imageSource"];
    validate_source(source).unwrap();
    for field in ["text", "sha256", "sourceKind"] {
        let mut bad = source.clone();
        bad[field] = json!("forged");
        assert!(validate_source(&bad).is_err(), "{field}");
    }
    let mut bad = source.clone();
    bad["evidence"][0]["end"] = json!(1);
    assert!(validate_source(&bad).is_err());
    let mut bad = source.clone();
    bad["lockedSource"] = json!({"text":"not the source"});
    assert!(validate_source(&bad).is_err());
    let locked = super::super::prepare_source("第1集\n日 内 门厅\n甲：走。\n").unwrap();
    let original =
        json!({"sourceKind":"original-script","sha256":locked.sha256,"lockedSource":locked});
    validate_source(&original).unwrap();
    let mut forged = original;
    forged["sha256"] = json!("fake");
    assert!(validate_source(&forged).is_err());
}

#[test]
fn new_source_binding_contract_is_not_retrofitted_into_legacy_frozen_prompt_requests() {
    let mut input=fixture();
    let old=build_request(&input,&input["assets"][0],&input["imageQueue"]["items"][0],"offline","medium").unwrap();
    assert!(old.input.get("sourceBindingContract").is_none());
    input["assetPromptSourceContract"]=json!(SOURCE_BINDING_V1);
    input["sourceUnitBindings"]=json!([
        {"unitId":"base-use","assetIds":["base"],"promptSha256":digest("人物单元")},
        {"unitId":"scene-use","assetIds":["hall"],"promptSha256":digest("无关场景单元")}
    ]);
    let current=build_request(&input,&input["assets"][0],&input["imageQueue"]["items"][0],"offline","medium").unwrap();
    assert_eq!(current.input["sourceBindingContract"],SOURCE_BINDING_V1);
    assert_eq!(current.input["sourceUnitBindings"].as_array().unwrap().len(),1);
    assert_eq!(current.input["sourceUnitBindings"][0]["unitId"],"base-use");
    input["assetPromptSourceContract"]=json!("unknown/v999");
    assert!(build_request(&input,&input["assets"][0],&input["imageQueue"]["items"][0],"offline","medium").is_err());
    assert!(old.input.get("sourceBindingContract").is_none());
}

#[test]
fn complete_package_design_parent_and_direction_are_read_only_supply_not_extra_image_tasks() {
    let mut input=fixture();
    input["assetPromptSourceContract"]=json!(SOURCE_BINDING_V1);
    input["assetPromptRequestVersion"]=json!(REQUEST_V2);
    input["sourceUnitBindings"]=json!([]);
    let parent=json!({"id":"external-parent","variantId":"parent-v1","designRevision":3,
        "adoptedDesign":{"version":"series-full-design/v1","assetId":"external-parent","variantId":"parent-v1","designRevision":3,"status":"ADOPTED","full":"固定人物比例与身份设计。"}});
    let original=json!({"id":"external-child","variantId":"child-v1","designRevision":1,
        "adoptedDesign":{"version":"series-full-design/v1","assetId":"external-child","variantId":"child-v1","designRevision":1,"status":"ADOPTED","full":"继承人物比例，穿灰衣。","designDependencies":[{"assetId":"external-parent","designRevision":3}]}});
    let registry=json!({"assets":[parent,original]});
    let direction=json!({"adoptedDirection":{"version":"imported-direction","style":"水墨漫画"},"settings":{"visualStyle":"水墨漫画"}});
    let proof=json!({"level":"desktop-complete-package","packageFingerprint":"f".repeat(64),"manifestSha256":"m".repeat(64),
        "registrySha256":digest(&registry.to_string()),"queueHash":"q".repeat(64),"directionSha256":digest(&direction.to_string())});
    let package=json!({"version":"desktop-complete-handoff/v1","proof":proof,"registry":registry,"direction":direction,
        "queue":{"queueHash":"q".repeat(64),"items":[{"assetId":"external-child"}]}});
    let mut asset=input["assets"][0].clone();
    asset["adoptedDesign"]=original["adoptedDesign"].clone();
    asset["adoptedDesign"]["assetId"]=asset["id"].clone();
    asset["adoptedDesign"]["variantId"]=asset["variantId"].clone();
    asset["handoffRegistryAsset"]=original;
    asset["handoffPackage"]=proof.clone();
    let item=input["imageQueue"]["items"][0].clone();
    input["assets"]=json!([asset]);
    input["imageQueue"]["items"]=json!([item]);
    input["externalPackages"]=json!([package]);
    input["imageSource"]["sourceKind"]=json!("external-package");
    input["imageSource"]["documents"]=json!([{"documentId":"doc-1"}]);
    input["imageSource"]["completePackages"]=json!([{"documentId":"doc-1","proof":proof}]);
    validate_source(&input["imageSource"]).unwrap();
    let request=build_request(&input,&asset,&item,"offline","medium").unwrap();
    assert_eq!(request.input["sourceKind"],"external-package");
    assert_eq!(request.input["parentDesigns"][0]["assetId"],"external-parent");
    assert_eq!(request.input["parentDesigns"][0]["readOnly"],true);
    assert_eq!(request.input["visualDirection"]["version"],"imported-direction");
    assert!(s(&request.input,"visualStyle").contains("水墨漫画"));
    assert_eq!(input["assets"].as_array().unwrap().len(),1);
    assert_eq!(input["imageQueue"]["items"].as_array().unwrap().len(),1);
    input["externalPackages"][0]["registry"]["assets"][0]["adoptedDesign"]["full"]=json!("被篡改的母设计");
    assert!(build_request(&input,&asset,&item,"offline","medium").is_err());
    input["imageSource"]["completePackages"]=json!([]);
    assert!(validate_source(&input["imageSource"]).is_err());
}

#[test]
fn adopted_storyboard_requires_its_handoff_identity_and_hash() {
    let mut source = fixture()["imageSource"].clone();
    source["sourceKind"] = json!("adopted-storyboard");
    assert!(validate_source(&source).is_err());
    source["handoffManifest"] = json!({"assetRegistryId":"registry","assetRegistryRevision":2});
    source["handoffSha256"] = json!(digest(&source["handoffManifest"].to_string()));
    validate_source(&source).unwrap();
    source["handoffManifest"]["assetRegistryRevision"] = json!(3);
    assert!(validate_source(&source).is_err());
}

#[tokio::test]
async fn md_only_authors_all_words_without_image_account_and_preserves_legacy_prompt() {
    let input = fixture();
    let before = input.clone();
    let recorder = Recorder::default();
    let result = run_asset_prompts(&input, &recorder, "offline-model", "medium")
        .await
        .unwrap();
    assert_eq!(input, before);
    assert_eq!(result["promptCount"], 3);
    assert_eq!(result["imageGenerationSubmissions"], 0);
    let assets = result["assets"].as_array().unwrap();
    assert_eq!(assets[0]["prompt"], "保留旧手工提示词");
    assert_eq!(assets[0]["promptOrigin"], "MANUAL");
    assert_eq!(assets[0]["designCandidate"]["sourceKind"], "imported-md");
    assert_eq!(assets[0]["designCandidate"]["adopted"], false);
    assert!(assets[0]["adoptedDesign"].is_null());
    assert!(prompt_text(&assets[0]).contains("七视图"));
    assert_eq!(image_presentation(&assets[0])["ratio"], "1:1");
    assert_eq!(image_presentation(&assets[2])["ratio"], "16:9");
    assert!(prompt_text(&assets[2]).contains("绝对无人"));
    assert_eq!(result["imagePlan"][1]["status"], "waiting_reference");
    assert_eq!(result["imagePlan"][2]["status"], "ready");
    assert_eq!(recorder.requests.lock().unwrap().len(), 3);
    for request in recorder.requests.lock().unwrap().iter() {
        assert_eq!(request.skill, "gpt-image25-asset-workflow");
        assert_eq!(request.input["imageEvidence"], "NO_PIXELS_READ");
        assert_eq!(request.input["visualDirection"]["style"], "3D国漫");
        assert!(request.input.get("imageAccount").is_none());
    }
}

#[tokio::test]
async fn adoption_status_prompt_plan_waits_for_unusable_reference_without_blocking_words() {
    for (state, ready) in [("adopted", true), ("imported-current", true),
        ("unknown", false), ("submission_unknown", false), ("candidate", false),
        ("pending_review", false), ("needs_repair", false), ("needs_review", false)] {
        let mut input = fixture();
        input["assets"][0]["adoptedMediaId"] = json!("existing-mother-media");
        input["assets"][0]["imageStatus"] = json!(state);
        input["imageQueue"]["items"] = json!([input["imageQueue"]["items"][1].clone()]);
        let recorder = Recorder::default();
        let result = run_asset_prompts(&input, &recorder, "offline-model", "medium").await.unwrap();
        assert_eq!(result["promptCount"], 1, "{state}: words remain independent");
        assert_eq!(result["imagePlan"][0]["promptStatus"], "ready");
        assert_eq!(result["imagePlan"][0]["status"], if ready {"ready"} else {"waiting_reference"}, "{state}: {result}");
        assert_eq!(result["imagePlan"][0]["missingReferences"], if ready {json!([])} else {json!(["base"])});
        assert_eq!(result["imageGenerationSubmissions"], 0);
        assert_eq!(recorder.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn explicit_queue_never_grows_to_include_a_missing_mother_image() {
    let mut input = fixture();
    input["imageQueue"]["items"] = json!([input["imageQueue"]["items"][1].clone()]);
    let recorder = Recorder::default();
    let result = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result["promptCount"], 1);
    assert_eq!(result["assets"][0]["id"], "suit");
    assert_eq!(result["imagePlan"][0]["missingReferences"], json!(["base"]));
    assert_eq!(recorder.requests.lock().unwrap().len(), 1);
    input["selection"] = json!({"assetIds":["base"]});
    assert!(run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .is_err());
}

#[tokio::test]
async fn family_relationship_is_not_a_design_or_media_dependency() {
    let mut input = fixture();
    input["assets"][1]
        .as_object_mut()
        .unwrap()
        .remove("referenceRequirements");
    input["assets"][1]
        .as_object_mut()
        .unwrap()
        .remove("imageTask");
    input["assets"][1]["sourceAssetId"] = json!("base");
    let recorder = Recorder::default();
    let result = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result["imagePlan"][1]["status"], "ready");
    let requests = recorder.requests.lock().unwrap();
    let child = requests
        .iter()
        .find(|r| r.input["asset"]["id"] == "suit")
        .unwrap();
    assert_eq!(child.input["parentDesigns"], json!([]));
    assert_eq!(child.input["referenceRequirements"], json!([]));
}

#[tokio::test]
async fn adopted_design_is_preserved_and_versioned_design_dependency_never_waits_for_a_picture() {
    let mut input = fixture();
    let full = json!({"face":"既定轮廓","clothes":"既定灰衣"});
    input["assets"][0]["adoptedDesign"] =
        json!({"full":full,"revision":3,"fullSha256":digest(&full.to_string())});
    input["assets"][1]["designDependencies"] = json!([{"assetId":"base","designRevision":3}]);
    let recorder = Recorder::default();
    let result = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(
        result["assets"][0]["adoptedDesign"],
        input["assets"][0]["adoptedDesign"]
    );
    assert!(result["assets"][0]["designCandidate"].is_null());
    assert_eq!(result["promptCount"], 3);
    input["assets"][1]["designDependencies"][0]["designRevision"] = json!(2);
    let bad = run_asset_prompts(&input, &Recorder::default(), "offline", "medium")
        .await
        .unwrap();
    assert_eq!(bad["promptCount"], 2);
    assert!(s(&bad["blocked"][0], "reason").contains("版本已变化"));
}

#[tokio::test]
async fn series_adoption_contract_supplies_real_revision_and_nested_design_dependency() {
    let mut input = fixture();
    for (index, revision) in [(0, 3), (1, 2)] {
        let asset = &mut input["assets"][index];
        asset["adoptedDesign"] = json!({
            "version":"series-full-design/v1","status":"ADOPTED",
            "assetId":asset["id"],"variantId":asset["variantId"],
            "designRevision":revision,"full":"已采用的完整人物设计，原文约束与美术补足已区分。",
            "designDependencies":[],"adoptionBasis":{"kind":"delegated-text-review"}
        });
        asset["designRevision"] = json!(revision);
    }
    input["assets"][1]["adoptedDesign"]["designDependencies"] =
        json!([{"assetId":"base","designRevision":3}]);
    let recorder = Recorder::default();
    let result = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result["promptCount"], 3);
    assert_eq!(
        result["assets"][0]["imagePrompt"]["basedOnDesignRevision"],
        3
    );
    assert_eq!(
        result["assets"][1]["imagePrompt"]["basedOnDesignRevision"],
        2
    );
    let requests = recorder.requests.lock().unwrap();
    let child = requests
        .iter()
        .find(|r| r.input["asset"]["id"] == "suit")
        .unwrap();
    assert_eq!(child.input["parentDesigns"][0]["designRevision"], 3);
    assert_eq!(
        child.input["parentDesigns"][0]["full"],
        input["assets"][0]["adoptedDesign"]["full"]
    );
    drop(requests);
    input["assets"][1]["adoptedDesign"]["designDependencies"][0]["designRevision"] = json!(2);
    let stale = run_asset_prompts(&input, &Recorder::default(), "offline", "medium")
        .await
        .unwrap();
    assert_eq!(stale["promptCount"], 2);
    assert_eq!(stale["blocked"][0]["assetId"], "suit");
}

#[tokio::test]
async fn unadopted_or_schema_only_design_does_not_masquerade_as_adopted_design() {
    for design in [
        json!({"version":"series-full-design/v1","full":"设计正文","status":"ADOPTED"}),
        json!({"version":"series-full-design/v1","designRevision":1,"full":"设计正文","status":"CANDIDATE"}),
        json!({"revision":1,"full":"设计正文","status":"REJECTED"}),
        json!({"revision":1,"full":"设计正文","state":"candidate"}),
        json!({"revision":1,"full":"设计正文","adopted":false}),
        json!({"revision":1,"full":"设计正文","assetId":"other-person"}),
    ] {
        let mut input = fixture();
        input["assets"][0]["adoptedDesign"] = design.clone();
        let result = run_asset_prompts(&input, &Recorder::default(), "offline", "medium")
            .await
            .unwrap();
        assert_eq!(result["promptCount"], 2, "{design}");
        assert_eq!(result["blocked"][0]["assetId"], "base", "{design}");
    }
}

#[tokio::test]
async fn media_roles_are_compatible_without_turning_generic_references_into_face_locks() {
    let mut input = fixture();
    input["assets"][1]["referenceRequirements"] =
        json!([{"targetAssetId":"base","role":"identity-faces"}]);
    let recorder = Recorder::default();
    let result = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result["promptCount"], 3);
    assert_eq!(
        result["assets"][1]["referenceRequirements"][0]["role"],
        "identity-faces"
    );

    input["assets"][1]["referenceRequirements"] = json!([{"targetAssetId":"base"}]);
    let blocked = run_asset_prompts(&input, &Recorder::default(), "offline", "medium")
        .await
        .unwrap();
    assert_eq!(blocked["promptCount"], 2);
    assert!(s(&blocked["blocked"][0], "reason").contains("身份母图"));
    input["assets"][1]
        .as_object_mut()
        .unwrap()
        .remove("imageTask");
    let generic = run_asset_prompts(&input, &Recorder::default(), "offline", "medium")
        .await
        .unwrap();
    assert_eq!(generic["promptCount"], 3);
    assert_eq!(
        generic["assets"][1]["referenceRequirements"][0]["role"],
        "reference"
    );
    assert_eq!(
        generic["assets"][1]["imagePrompt"]["layoutKind"],
        "single-character"
    );
}

#[tokio::test]
async fn a_local_ambiguity_or_unknown_does_not_resubmit_or_block_other_assets() {
    for mode in ["unknown", "unresolved", "identity", "pixels", "reference"] {
        let recorder = Recorder {
            failure: Some(("base", mode)),
            ..Default::default()
        };
        let result = run_asset_prompts(&fixture(), &recorder, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(result["promptCount"], 2, "{mode}");
        assert_eq!(result["blocked"].as_array().unwrap().len(), 1, "{mode}");
        assert_eq!(
            recorder.requests.lock().unwrap().len(),
            3,
            "failed/unknown requests must not be blindly repeated"
        );
        assert_eq!(
            recorder.rejected.lock().unwrap().len(),
            usize::from(["identity", "pixels", "reference"].contains(&mode))
        );
    }
}

#[tokio::test]
async fn missing_evidence_blocks_only_that_asset_and_preserves_successful_prompt_cache() {
    let mut input = fixture();
    input["imageSource"]["evidence"][0]["assetIds"] = json!(["base", "suit"]);
    input["assets"][2]["sourceRefs"] = json!([]);
    let recorder = Recorder::default();
    let result = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result["promptCount"], 2);
    assert_eq!(result["blocked"][0]["assetId"], "hall");
    for saved in result["assets"].as_array().unwrap() {
        let id = s(saved, "id");
        let old = input["assets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| s(a, "id") == id)
            .unwrap();
        *old = saved.clone();
    }
    let repeated = Recorder::default();
    let result2 = run_asset_prompts(&input, &repeated, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result2["promptCount"], 2);
    assert!(
        repeated.requests.lock().unwrap().is_empty(),
        "writing prompts must not purchase a completed design again"
    );
}

#[tokio::test]
async fn nonhuman_layout_is_selected_from_evidence_without_changing_identity_or_causing_a_cache_miss(
) {
    let recorder = Recorder {
        failure: Some(("base", "nonhuman")),
        ..Default::default()
    };
    let mut input = fixture();
    input["selection"] = json!({"assetIds":["base"]});
    let result = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    let asset = &result["assets"][0];
    assert_eq!(asset["id"], "base");
    assert_eq!(image_presentation(asset)["layoutKind"], "nonhuman");
    input["assets"][0] = asset.clone();
    let replay = Recorder::default();
    run_asset_prompts(&input, &replay, "offline", "medium")
        .await
        .unwrap();
    assert!(replay.requests.lock().unwrap().is_empty());
}

#[test]
fn rules_bind_exact_skill_sources_and_only_applicable_main_sections() {
    assert_eq!(
        digest(MAIN),
        "c739220422736a3f4b00c7df5e688ec08a41d58016671f2eac867fde2252945d"
    );
    assert_eq!(
        digest(HANDOFF),
        "c56b36b998bb78859eea825081094b3f65e19cff8ccd2f0de840bc072e6c57e7"
    );
    let character = rule_snapshot(&json!({"kind":"character"}));
    let scene = rule_snapshot(&json!({"kind":"scene"}));
    assert!(s(&character["primary"], "content").contains("同角色不同服装：分步生成与原像素合成"));
    assert!(s(&scene["primary"], "content").contains("场景主资产硬规则"));
    assert!(!s(&scene["primary"], "content").contains("## 4. 人物标准资产板"));
    for snapshot in [character, scene] {
        assert!(snapshot["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "asset-handoff-22"));
    }
    assert!(!enabled(&json!({})).unwrap());
    assert!(enabled(&json!({"workflowProfile":"future-unknown"})).is_err());
}

#[test]
fn current_presentation_cannot_be_overridden_by_the_video_ratio_and_legacy_contract_is_unchanged() {
    let legacy = json!({"kind":"character"});
    assert_eq!(
        super::super::asset_design_contract::image_presentation(&json!({"ratio":"9:16"}), &legacy)
            ["ratio"],
        "9:16"
    );
    let current = json!({"kind":"character","workflowProfile":PROFILE});
    assert_eq!(
        super::super::asset_design_contract::image_presentation(&json!({"ratio":"16:9"}), &current)
            ["ratio"],
        "1:1"
    );
    let old = json!({"kind":"character","prompt":"原手写词"});
    assert_eq!(prompt_text(&old), "原手写词");
    assert!(compile_preview(&old).is_err());
}

struct MediaPlanner {
    recorder: Recorder,
    plans: HashMap<String, Value>,
}
#[async_trait]
impl TextExecutor for MediaPlanner {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let mut value = self.recorder.execute(request.clone()).await?;
        if let Some(plan) = self.plans.get(s(&request.input["asset"], "id")) {
            value["mediaPlan"] = plan.clone();
            value["layoutKind"] = plan["imageTask"]["kind"].clone();
        }
        Ok(value)
    }
}
fn planned_reference(target: &str, variant: &str, role: &str) -> Value {
    json!({"targetAssetId":target,"targetVariantId":variant,"role":role,
        "mustPreserve":["证据已明确的身份或空间结构"],"mayChange":["本项明确的服装、状态或机位"],
        "evidence":[{"evidenceId":"md:1","quote":"甲换上西装"}]})
}
fn planned(layout: &str, refs: Vec<Value>) -> Value {
    json!({"version":MEDIA_PLAN_VERSION,"mode":"plan","imageTask":{"kind":layout},
        "referenceRequirements":refs,"rationale":"沿明确的同一身份或空间关系，只引用本次状态实际需要的直接母版。"})
}
fn media_fixture() -> Value {
    let mut input = fixture();
    input["assetPromptRequestVersion"] = json!(REQUEST_V2);
    input["assets"][1]
        .as_object_mut()
        .unwrap()
        .remove("imageTask");
    input["assets"][1]["referenceRequirements"] = json!([]);
    input["assets"][1]["sourceAssetId"] = json!("base");
    input["assets"][2]["baseAssetId"] = json!("space-hall");
    input["assets"].as_array_mut().unwrap().extend([
        json!({"id":"injured","variantId":"injured-v1","name":"甲西装受伤状态","kind":"character","baseAssetId":"person-a","sourceAssetId":"suit","sourceRefs":["md:1"],"referenceRequirements":[]}),
        json!({"id":"night","variantId":"night-v1","name":"门厅夜间","kind":"scene","baseAssetId":"space-hall","sourceAssetId":"hall","sourceRefs":["md:1"],"referenceRequirements":[]})
    ]);
    let text="第1集\n甲换上西装，随后西装袖口受伤破损，保持同一人物身份和原西装结构。门厅日间变为夜间，门在左窗在右的拓扑不变。";
    input["imageSource"] = json!({"sourceKind":"imported-md","documentId":digest(text),"text":text,"sha256":digest(text),
        "evidence":[{"id":"md:1","text":text,"start":0,"end":text.len(),"assetIds":["base","suit","hall","injured","night"]}]});
    input["imageQueue"]["items"]=json!(input["assets"].as_array().unwrap().iter().map(|a|json!({"assetId":a["id"],"variantId":a["variantId"],"filename":format!("{}.png",s(a,"id")),"uses":[]})).collect::<Vec<_>>());
    input
}

#[tokio::test]
async fn empty_media_plans_produce_three_level_and_scene_dependencies_without_images_or_scope_expansion(
) {
    let mut input = media_fixture();
    // Only children are selected. Read-only root evidence must not add a task.
    input["imageQueue"]["items"]
        .as_array_mut()
        .unwrap()
        .retain(|item| item["assetId"] != "base" && item["assetId"] != "hall");
    let mut scene = planned_reference("hall", "hall-v1", "topology");
    scene["evidence"][0]["quote"] = json!("门厅日间变为夜间，门在左窗在右的拓扑不变");
    let planner = MediaPlanner {
        recorder: Recorder::default(),
        plans: HashMap::from([
            (
                "suit".into(),
                planned(
                    "costume",
                    vec![planned_reference("base", "base-v1", "identity-faces")],
                ),
            ),
            (
                "injured".into(),
                planned(
                    "single-character",
                    vec![planned_reference("suit", "suit-v1", "appearance")],
                ),
            ),
            ("night".into(), planned("scene", vec![scene])),
        ]),
    };
    let result = run_asset_prompts(&input, &planner, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result["promptCount"], 3);
    assert_eq!(result["blocked"], json!([]));
    assert_eq!(result["imageGenerationSubmissions"], 0);
    let updates = result["assets"].as_array().unwrap();
    for (id, parent) in [("suit", "base"), ("injured", "suit"), ("night", "hall")] {
        let a = updates.iter().find(|a| a["id"] == id).unwrap();
        assert_eq!(a["referenceRequirements"][0]["targetAssetId"], parent);
        assert_eq!(a["mediaPlan"]["sourceKind"], "imported-md");
        assert_eq!(a["mediaPlan"]["imageEvidence"], "NO_PIXELS_READ");
        assert_eq!(a["promptStatus"], "ready");
        assert_eq!(
            result["imagePlan"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["assetId"] == id)
                .unwrap()["status"],
            "waiting_reference"
        );
    }
    assert_eq!(planner.recorder.requests.lock().unwrap().len(), 3);
    for asset in updates {
        *input["assets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| a["id"] == asset["id"])
            .unwrap() = asset.clone();
    }
    let replay = Recorder::default();
    let cached = run_asset_prompts(&input, &replay, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(cached["promptCount"], 3);
    assert!(
        replay.requests.lock().unwrap().is_empty(),
        "new plan must not invalidate its own prompt cache"
    );
}

#[tokio::test]
async fn media_plan_rejects_wrong_identity_variant_and_unbound_evidence_without_guessing_names() {
    for mode in [
        "wrong-person",
        "unknown-identity",
        "variant",
        "quote",
        "evidence",
        "target",
    ] {
        let mut input = media_fixture();
        input["selection"] = json!({"assetIds":["suit"]});
        let mut reference = planned_reference("base", "base-v1", "identity-faces");
        match mode {
            "wrong-person" => input["assets"][0]["baseAssetId"] = json!("different-person"),
            "unknown-identity" => {
                input["assets"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("baseAssetId");
                input["assets"][1]
                    .as_object_mut()
                    .unwrap()
                    .remove("baseAssetId");
            }
            "variant" => reference["targetVariantId"] = json!("not-the-frozen-variant"),
            "quote" => reference["evidence"][0]["quote"] = json!("不在来源里的话"),
            "evidence" => reference["evidence"][0]["evidenceId"] = json!("future-evidence"),
            "target" => reference["targetAssetId"] = json!("outside-frozen-registry"),
            _ => unreachable!(),
        }
        let planner = MediaPlanner {
            recorder: Recorder::default(),
            plans: HashMap::from([("suit".into(), planned("costume", vec![reference]))]),
        };
        let output = run_asset_prompts(&input, &planner, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(output["promptCount"], 0, "{mode}");
        assert_eq!(output["blocked"].as_array().unwrap().len(), 1, "{mode}");
        assert_eq!(planner.recorder.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn media_plan_preserves_prior_plan_manual_prompt_and_legacy_frozen_contract() {
    let legacy = fixture();
    let old_request = build_request(
        &legacy,
        &legacy["assets"][1],
        &legacy["imageQueue"]["items"][1],
        "offline",
        "medium",
    )
    .unwrap();
    assert!(old_request.input.get("assetPromptRequestVersion").is_none());
    assert!(old_request.schema["properties"].get("mediaPlan").is_none());
    let old_raw = Recorder::default()
        .execute(old_request.clone())
        .await
        .unwrap();
    let old = finalize_output(&old_request, &legacy["assets"][1], &old_raw).unwrap();
    assert!(old["asset"].get("mediaPlan").is_none());
    let mut input = legacy;
    input["assetPromptRequestVersion"] = json!(REQUEST_V2);
    input["assets"][0]["imagePrompt"] = json!({"version":PROMPT_VERSION,"origin":"MANUAL","text":"用户完整手写词","sourceSha256":input["imageSource"]["sha256"],"basedOnDesignRevision":null});
    let recorder = Recorder::default();
    let output = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["promptCount"], 3);
    assert_eq!(output["assets"][0]["imagePrompt"]["text"], "用户完整手写词");
    assert_eq!(output["assets"][1]["mediaPlan"]["mode"], "preserve");
    assert_eq!(
        output["assets"][1]["referenceRequirements"][0]["kind"],
        "identity"
    );
    assert_eq!(recorder.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn independently_authored_media_cycle_blocks_only_its_chain_and_keeps_words() {
    let mut input = media_fixture();
    input["selection"] = json!({"assetIds":["base","suit","hall"]});
    let planner = MediaPlanner {
        recorder: Recorder::default(),
        plans: HashMap::from([
            (
                "base".into(),
                planned(
                    "single-character",
                    vec![planned_reference("suit", "suit-v1", "appearance")],
                ),
            ),
            (
                "suit".into(),
                planned(
                    "costume",
                    vec![planned_reference("base", "base-v1", "identity-faces")],
                ),
            ),
        ]),
    };
    let output = run_asset_prompts(&input, &planner, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["promptCount"], 3);
    assert_eq!(output["blocked"].as_array().unwrap().len(), 2);
    assert_eq!(output["assets"][2]["mediaPlan"]["status"], "ready");
    assert_eq!(output["assets"][0]["mediaPlan"]["status"], "needs_review");
    assert_eq!(output["assets"][1]["mediaPlan"]["status"], "needs_review");
}
