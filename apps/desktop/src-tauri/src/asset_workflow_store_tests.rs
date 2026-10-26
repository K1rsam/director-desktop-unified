use super::*;
#[path = "asset_prompt_source_tests.rs"]
mod prompt_sources;
#[path = "asset_costume_supply_tests.rs"]
mod costume_supply;
use crate::pipeline::{asset_workflow, TextExecutor, TextRequest};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

const MD: &str = "第1集\n1-1\n场景：门厅\n人物：甲\n片段1\n时长：4秒\n[人物] @[甲CH] @[旁白SE]\n分镜1｜0-3s｜3s\n画面：甲穿灰衣站定。\n片段2\n时长：4秒\n[场景] @[门厅SC]\n分镜1｜0-3s｜3s\n画面：门在左，窗在右。\n";

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store.create("asset workflow offline", "p").unwrap();
        Self { dir, store }
    }
    fn import_md(&self) {
        lingmo_import::import(&self.store, &json!({"text":MD,"expectedSha256":digest(MD)}))
            .unwrap();
    }
    fn asset(&self, id: &str) -> Value {
        self.store.save_asset(json!({"id":id,"variantId":"v1","name":id,"kind":"character","identityId":"person","workflowProfile":asset_graph::PROFILE,"filename":format!("{id}.png"),"referenceRequirements":[],"prompt":"原手工提示词","promptOrigin":"MANUAL"})).unwrap()
    }
    fn candidate(&self, asset: &Value, seed: u8) -> Value {
        let path = self
            .dir
            .path()
            .join(format!("{}-{seed}.png", s(asset, "id")));
        image::RgbaImage::from_fn(8, 8, |x, y| {
            image::Rgba([seed, x as u8 * 16, y as u8 * 16, 255])
        })
        .save(&path)
        .unwrap();
        let media = self
            .store
            .import_media(
                &path,
                "image",
                Some(s(asset, "id")),
                Some(s(asset, "variantId")),
            )
            .unwrap();
        let job=self.store.add_job("image",s(asset,"id"),"offline","offline",&json!({"workflowProfile":asset_graph::PROFILE,"assetId":asset["id"],"variantId":asset["variantId"],"assetVersion":asset["version"]}),"downloaded").unwrap();
        self.store.attach_result(&job, s(&media, "id")).unwrap();
        media
    }
    fn workflow(&self, input: &Value) -> String {
        self.store
            .add_job(
                "workflow",
                "images.design:",
                "offline",
                "offline",
                &json!({"stage2Context":input}),
                "running",
            )
            .unwrap()
    }
}

#[derive(Default)]
struct OfflineText {
    calls: AtomicUsize,
}
#[async_trait]
impl TextExecutor for OfflineText {
    async fn execute(&self, r: TextRequest) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let a = &r.input["asset"];
        let mut value = json!({"assetId":a["id"],"variantId":a["variantId"],"promptText":"既定主体完整清晰，保持身份与空间约束。",
            "designSupplement":if a["adoptedDesign"]["full"].is_null(){"依据所给分镜补足中性材质，不改变身份、拓扑和用途。"}else{""},
            "layoutKind":r.input["imagePresentation"]["layoutKind"],"sourceRefs":rows(&r.input,"evidence").iter().map(|e|e["id"].clone()).collect::<Vec<_>>(),
            "imageEvidence":"NO_PIXELS_READ","unresolved":[]});
        if r.input["assetPromptRequestVersion"] == asset_workflow::REQUEST_V2 {
            value["mediaPlan"] = json!({"version":"asset-media-plan/v1","mode":if rows(&r.input,"referenceRequirements").is_empty(){"plan"}else{"preserve"},
                "imageTask":{"kind":value["layoutKind"]},"referenceRequirements":[],"rationale":"当前冻结证据只要求独立项或完整保留既定参考。"});
        }
        Ok(value)
    }
}

#[test]
fn md_only_scope_uses_parser_visual_refs_and_never_invents_original_source() {
    let f = Fixture::new();
    f.import_md();
    f.asset("unused-registry-person");
    assert!(f.store.source().unwrap().is_null());
    let input = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    assert_eq!(input["imageSource"]["sourceKind"], "imported-md");
    assert_eq!(input["imageSource"]["text"], MD);
    assert_eq!(input["imageQueue"]["items"].as_array().unwrap().len(), 2);
    let filenames: Vec<_> = rows(&input["imageQueue"], "items")
        .iter()
        .map(|i| s(i, "filename").to_owned())
        .collect();
    assert_eq!(filenames, vec!["甲CH.png", "门厅SC.png"]);
    assert!(!rows(&input["selection"], "assetIds").contains(&json!("unused-registry-person")));
    assert!(input["imageSource"].get("lockedSource").is_none());
    // A separate old source in the project must not hijack selected MD assets.
    let source =
        crate::pipeline::prepare_source("第1集\n1-1 日 内 旧场景\n乙：原剧本。\n").unwrap();
    f.store
        .save_source(serde_json::to_value(source).unwrap())
        .unwrap();
    let second = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    assert_eq!(second["imageSource"], input["imageSource"]);
}

#[test]
fn native_scope_does_not_expand_from_registry_mentions_or_audio() {
    let f = Fixture::new();
    for id in ["shown", "mentioned", "unused"] {
        f.asset(id);
    }
    f.store
        .save_asset(json!({"id":"voice","variantId":"v1","kind":"audio"}))
        .unwrap();
    let unit = json!({"id":"u","episodeId":"e1","prompt":"采用分镜","imageBindings":[{"assetId":"shown","variantId":"v1"}],"mentions":[{"assetId":"mentioned"},{"assetId":"voice"}],"assetRefs":[{"assetId":"shown","variantId":"v1"},{"assetId":"mentioned","variantId":"v1"}]});
    f.store.save_unit(unit.clone()).unwrap();
    let delivery = json!({"markdown":"采用分镜","assetIndex":{"assets":f.store.values("assets").unwrap()},"bundle":{"draft":{"units":[unit]}}});
    f.store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO metadata VALUES('series.delivery:e1',?1)",
            [delivery.to_string()],
        )
        .unwrap();
    assert_eq!(
        f.store.image_production_scope().unwrap()["assetIds"],
        json!(["shown"])
    );
    assert!(f
        .store
        .image_workflow_input(&json!({"assetIds":["mentioned"]}), &json!({}))
        .is_err());
}

#[tokio::test]
async fn md_prompts_publish_independent_success_and_retain_concurrent_edit_candidate() {
    let f = Fixture::new();
    f.import_md();
    let input = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    let executor = OfflineText::default();
    let output = asset_workflow::run_asset_prompts(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["promptCount"], 2);
    let first = input["selection"]["assetIds"][0].as_str().unwrap();
    let mut edited = f.store.active_asset(first).unwrap();
    edited["prompt"] = json!("用户刚改的原手写提示词");
    f.store.save_asset(edited).unwrap();
    let run = f.workflow(&input);
    let result = f.store.finish_image_prompts(&input, &output, &run).unwrap();
    assert_eq!(result["promptCount"], 1);
    assert_eq!(result["unpublishedCandidates"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.store.active_asset(first).unwrap()["prompt"],
        "用户刚改的原手写提示词"
    );
    assert_eq!(f.store.job(&run).unwrap()["status"], "partially_blocked");
    assert!(f.store.source().unwrap().is_null());
}

#[tokio::test]
async fn current_md_edit_is_named_evidence_and_inflight_edit_only_blocks_its_consumers() {
    let f = Fixture::new();
    f.import_md();
    let input = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    let output =
        asset_workflow::run_asset_prompts(&input, &OfflineText::default(), "offline", "medium")
            .await
            .unwrap();
    let original = f.store.values("units").unwrap().remove(0);
    let mut edited = original.clone();
    edited["prompt"] = json!(s(&original, "prompt").replace("灰衣", "蓝衣"));
    edited["expectedVersion"] = original["version"].clone();
    lingmo_import::save_unit(&f.store, &original, edited).unwrap();
    let run = f.workflow(&input);
    let result = f.store.finish_image_prompts(&input, &output, &run).unwrap();
    assert_eq!(result["promptCount"], 1);
    assert_eq!(result["blocked"].as_array().unwrap().len(), 1);
    let current = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    assert_eq!(
        current["imageSource"]["evidence"][0]["evidenceOrigin"],
        "saved-desktop-unit-edit"
    );
    assert!(s(&current["imageSource"]["evidence"][0], "text").contains("蓝衣"));
    assert_eq!(
        lingmo_import::original(&f.store, &digest(MD), None).unwrap()["text"],
        MD
    );
}

#[tokio::test]
async fn saving_native_image_prompts_does_not_invalidate_their_design_handoff_cache() {
    let f = Fixture::new();
    let mut asset = f.asset("shown");
    asset["adoptedDesign"] = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":"shown","variantId":"v1",
            "full":"既定完整人物设计：身体与衣服结构，准确身份。","designRevision":3,"adoptionBasis":{"kind":"delegated-text-review","revision":3}});
    asset["designRevision"] = json!(3);
    let asset = f.store.save_asset(asset).unwrap();
    let unit = json!({"id":"u","episodeId":"e1","prompt":"原采用分镜","imageBindings":[{"assetId":"shown","variantId":"v1"}]});
    f.store.save_unit(unit.clone()).unwrap();
    let delivery = json!({"markdown":"原采用分镜","assetIndex":{"assets":[{"id":"shown","assetRevision":3,"filename":"shown.png"}]},"bundle":{"draft":{"units":[unit]}}});
    f.store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO metadata VALUES('series.delivery:e1',?1)",
            [delivery.to_string()],
        )
        .unwrap();
    let input = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    let executor = OfflineText::default();
    let output = asset_workflow::run_asset_prompts(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    let run = f.workflow(&input);
    f.store.finish_image_prompts(&input, &output, &run).unwrap();
    let again = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    assert_eq!(again["imageSource"], input["imageSource"]);
    asset_workflow::run_asset_prompts(&again, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let saved = f.store.active_asset("shown").unwrap();
    assert_eq!(saved["adoptedDesign"], asset["adoptedDesign"]);
    assert_eq!(saved["prompt"], "原手工提示词");
    assert_eq!(saved["mediaPlan"]["version"], "asset-media-plan/v1");
    assert_eq!(saved["mediaPlan"]["imageEvidence"], "NO_PIXELS_READ");
    let reopened = Store::open(f.store.root.clone()).unwrap();
    assert_eq!(
        reopened.active_asset("shown").unwrap()["mediaPlan"],
        saved["mediaPlan"]
    );
}

#[tokio::test]
async fn in_flight_parent_identity_change_preserves_media_plan_candidate_and_independent_success() {
    let f = Fixture::new();
    f.import_md();
    let mut parent = f.asset("registered-parent");
    let mut child = f
        .store
        .values("assets")
        .unwrap()
        .into_iter()
        .find(|a| a["lingmoType"] == "CH")
        .unwrap();
    child["identityId"] = json!("person");
    child["referenceRequirements"] = json!([{"targetAssetId":parent["id"],"targetVariantId":parent["variantId"],"role":"identity-faces","mustPreserve":["face"],"mayChange":["clothing"]}]);
    child = f.store.save_asset(child).unwrap();
    let input = f
        .store
        .image_workflow_input(&json!({}), &json!({}))
        .unwrap();
    assert_eq!(
        input["assetPromptRequestVersion"],
        asset_workflow::REQUEST_V2
    );
    assert_eq!(
        rows(&input["imageQueue"], "items").len(),
        2,
        "mother stays outside explicit image scope"
    );
    let output =
        asset_workflow::run_asset_prompts(&input, &OfflineText::default(), "offline", "medium")
            .await
            .unwrap();
    assert_eq!(output["promptCount"], 2);
    parent["identityId"] = json!("a-different-person");
    f.store.save_asset(parent).unwrap();
    let run = f.workflow(&input);
    let result = f.store.finish_image_prompts(&input, &output, &run).unwrap();
    assert_eq!(result["promptCount"], 1);
    assert_eq!(result["unpublishedCandidates"][0]["id"], child["id"]);
    assert!(result["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| s(b, "reason").contains("参考资产身份")));
    assert_eq!(
        f.store.active_asset(s(&child, "id")).unwrap()["referenceRequirements"],
        child["referenceRequirements"]
    );
    assert!(f.store.active_asset(s(&child, "id")).unwrap()["mediaPlan"].is_null());
}

#[test]
fn three_and_eleven_layers_unlock_only_after_real_manual_adoption() {
    for count in [3, 11] {
        let f = Fixture::new();
        let mut chain = Vec::new();
        for i in 0..count {
            let mut a = f.asset(&format!("level-{i}"));
            if i > 0 {
                a["referenceRequirements"] =
                    json!([{"targetAssetId":format!("level-{}",i-1),"kind":"identity"}]);
                a = f.store.save_asset(a).unwrap();
            }
            chain.push(a);
        }
        let independent = f.asset("independent");
        let mut all = chain.clone();
        all.push(independent.clone());
        let mut prior: Option<Value> = None;
        for i in 0..count {
            if i > 0 {
                assert!(asset_graph::media_references(&f.store, &all, &chain[i])
                    .unwrap()
                    .is_none());
                let parent = &chain[i - 1];
                let media = prior.as_ref().unwrap();
                f.store
                    .adopt(s(parent, "id"), s(parent, "variantId"), s(media, "id"))
                    .unwrap();
            }
            let refs = asset_graph::media_references(&f.store, &all, &chain[i])
                .unwrap()
                .unwrap();
            assert_eq!(refs.len(), usize::from(i > 0));
            if i > 0 {
                assert_eq!(refs[0]["mediaId"], prior.as_ref().unwrap()["id"]);
                assert_eq!(refs[0]["sha256"], prior.as_ref().unwrap()["sha256"]);
            }
            prior = Some(f.candidate(&chain[i], (20 + i) as u8));
            f.store.ensure_current_images().unwrap();
            assert!(
                f.store.adopted(s(&chain[i], "id"), "v1").unwrap().is_none(),
                "candidate must not auto-adopt"
            );
            assert!(asset_graph::media_references(&f.store, &all, &independent)
                .unwrap()
                .is_some());
        }
        let frozen = asset_graph::media_references(&f.store, &all, &chain[1])
            .unwrap()
            .unwrap();
        let replacement = f.candidate(&chain[0], 200);
        f.store
            .adopt(s(&chain[0], "id"), "v1", s(&replacement, "id"))
            .unwrap();
        let changed = asset_graph::media_references(&f.store, &all, &chain[1])
            .unwrap()
            .unwrap();
        assert_ne!(frozen[0]["mediaId"], changed[0]["mediaId"]);
        assert_ne!(frozen[0]["sha256"], changed[0]["sha256"]);
        fs::write(
            f.store.absolute_media(&replacement).unwrap(),
            b"corrupt after adoption",
        )
        .unwrap();
        assert!(asset_graph::media_references(&f.store, &all, &chain[1]).is_err());
    }
}

#[test]
fn reference_support_is_explicit_counted_and_does_not_change_narrative_use() {
    let f = Fixture::new();
    let mother = f.asset("mother");
    let mut child = f.asset("child");
    child["referenceRequirements"] = json!([{"targetAssetId":"mother","role":"identity"}]);
    f.store.save_asset(child).unwrap();
    f.store.save_unit(json!({"id":"u","episodeId":"e","prompt":"只有子形象入画","imageBindings":[{"assetId":"child"}]})).unwrap();
    let scope = f.store.image_production_scope().unwrap();
    assert_eq!(scope["assetIds"], json!(["child"]));
    assert_eq!(scope["referenceOnlyIds"], json!([]));
    let request = json!({"assetId":"mother","expectedVersion":mother["version"],"servesAssetIds":["child"],"reason":"仅供子形象锁脸","confirmed":true});
    let result = f.store.register_image_reference(&request).unwrap();
    assert_eq!(result["addedImageCount"], 1);
    assert_eq!(result["generationSubmissions"], 0);
    assert_eq!(result["scope"]["assetIds"], json!(["child"]));
    assert_eq!(result["scope"]["referenceOnlyIds"], json!(["mother"]));
    assert_eq!(result["asset"]["purpose"], mother["purpose"]);
    assert_eq!(result["asset"]["productionPurpose"], "reference-only");
    let mut invalid = request;
    invalid["expectedVersion"] = result["asset"]["version"].clone();
    invalid["servesAssetIds"] = json!(["mother"]);
    assert!(f.store.register_image_reference(&invalid).is_err());
}

#[test]
fn adoption_status_reference_registration_counts_unusable_mother_as_pending() {
    for (status, usable) in [("adopted", true), ("imported-current", true),
        ("unknown", false), ("submission_unknown", false), ("candidate", false),
        ("pending_review", false), ("needs_repair", false), ("needs_review", false)] {
        let f = Fixture::new();
        let mother = f.asset("mother");
        let media = f.candidate(&mother, 42);
        f.store.adopt("mother", "v1", s(&media, "id")).unwrap();
        let mut mother = f.store.active_asset("mother").unwrap();
        mother["imageStatus"] = json!(status);
        // Simulate a persisted adoption projection. save_asset would correctly
        // reapply the existing lifecycle record instead of accepting a caller
        // overriding that projection with an arbitrary status.
        f.store.conn().unwrap().execute("UPDATE assets SET data=?1 WHERE id='mother'", [mother.to_string()]).unwrap();
        let mut child = f.asset("child");
        child["referenceRequirements"] = json!([{"targetAssetId":"mother","role":"identity"}]);
        f.store.save_asset(child).unwrap();
        f.store.save_unit(json!({"id":"u","episodeId":"e","prompt":"子形象入画",
            "imageBindings":[{"assetId":"child"}]})).unwrap();
        let jobs = f.store.jobs().unwrap();
        let result = f.store.register_image_reference(&json!({"assetId":"mother",
            "expectedVersion":mother["version"],"servesAssetIds":["child"],
            "reason":"正式子项需要当前有效身份母图","confirmed":true})).unwrap();
        assert_eq!(result["reusedAdoptedMedia"], usable, "{status}: {result}");
        assert_eq!(result["addedImageCount"], usize::from(!usable), "{status}: {result}");
        assert_eq!(result["generationSubmissions"], 0);
        assert_eq!(f.store.jobs().unwrap(), jobs, "registration never generates a replacement");
        assert_eq!(f.store.adopted("mother", "v1").unwrap().as_deref(), Some(s(&media,"id")));
    }
}

#[test]
fn reference_support_chain_requires_each_registration_and_counts_new_items_once() {
    let f = Fixture::new();
    let root = f.asset("root");
    let mut middle = f.asset("middle");
    middle["referenceRequirements"] = json!([{"targetAssetId":"root","role":"appearance"}]);
    let middle = f.store.save_asset(middle).unwrap();
    let mut leaf = f.asset("leaf");
    leaf["referenceRequirements"] = json!([{"targetAssetId":"middle","role":"appearance"}]);
    f.store.save_asset(leaf).unwrap();
    f.store.save_unit(json!({"id":"u","episodeId":"e","prompt":"仅叶节点入画","imageBindings":[{"assetId":"leaf"}]})).unwrap();
    let root_request = json!({"assetId":"root","expectedVersion":root["version"],"servesAssetIds":["middle"],"reason":"为已登记中间形象提供母图","confirmed":true});
    assert!(f.store.register_image_reference(&root_request).is_err(), "cannot skip an unregistered support node");
    assert_eq!(f.store.image_production_scope().unwrap()["referenceOnlyIds"],json!([]));
    let first = f.store.register_image_reference(&json!({"assetId":"middle","expectedVersion":middle["version"],"servesAssetIds":["leaf"],"reason":"为正式入画形象提供母图","confirmed":true})).unwrap();
    assert_eq!(first["addedImageCount"],1);
    assert_eq!(first["scope"]["assetIds"],json!(["leaf"]));
    assert_eq!(first["scope"]["referenceOnlyIds"],json!(["middle"]));
    let second = f.store.register_image_reference(&root_request).unwrap();
    assert_eq!(second["addedImageCount"],1);
    assert_eq!(second["generationSubmissions"],0);
    assert_eq!(second["scope"]["assetIds"],json!(["leaf"]));
    assert_eq!(second["scope"]["referenceOnlyIds"],json!(["root","middle"]));
    assert_eq!(second["asset"]["purpose"],root["purpose"],"support is not invented narrative use");
    let mut repeated_request=root_request;
    repeated_request["expectedVersion"]=second["asset"]["version"].clone();
    let repeated=f.store.register_image_reference(&repeated_request).unwrap();
    assert_eq!(repeated["addedImageCount"],0);
    assert_eq!(repeated["asset"]["version"],second["asset"]["version"],"same registration is idempotent");
    assert!(f.store.jobs().unwrap().is_empty());

    let mut unit=f.store.get("units","u").unwrap();unit["imageBindings"]=json!([]);
    f.store.save_unit(unit).unwrap();
    let removed=f.store.image_production_scope().unwrap();
    assert_eq!(removed["assetIds"],json!([]));
    assert_eq!(removed["referenceOnlyIds"],json!([]),"no formal use means no live support scope");
    assert!(f.store.image_workflow_input(&json!({"assetIds":["root"]}),&json!({})).is_err());
}

#[test]
fn reference_support_rejects_cycles_and_stale_served_variants() {
    let f=Fixture::new();
    let mut mother=f.asset("mother");
    let mut child=f.asset("child");
    child["referenceRequirements"]=json!([{"targetAssetId":"mother","role":"appearance"}]);
    f.store.save_asset(child).unwrap();
    f.store.save_unit(json!({"id":"u","episodeId":"e","prompt":"子形象入画","imageBindings":[{"assetId":"child"}]})).unwrap();
    mother["referenceRequirements"]=json!([{"targetAssetId":"child","role":"appearance"}]);
    mother=f.store.save_asset(mother).unwrap();
    let mut request=json!({"assetId":"mother","expectedVersion":mother["version"],"servesAssetIds":["child"],"reason":"有据参考用途","confirmed":true});
    assert!(f.store.register_image_reference(&request).unwrap_err().contains("循环"));
    mother["referenceRequirements"]=json!([]);mother=f.store.save_asset(mother).unwrap();request["expectedVersion"]=mother["version"].clone();
    f.store.register_image_reference(&request).unwrap();
    assert_eq!(f.store.image_production_scope().unwrap()["referenceOnlyIds"],json!(["mother"]));
    let mut child=f.store.active_asset("child").unwrap();child["variantId"]=json!("v2");
    f.store.save_asset(child).unwrap();
    assert_eq!(f.store.image_production_scope().unwrap()["referenceOnlyIds"],json!([]),"old served variant does not authorize a new one");
    let mut child=f.store.active_asset("child").unwrap();child["variantId"]=json!("v1");
    f.store.save_asset(child).unwrap();
    assert_eq!(f.store.image_production_scope().unwrap()["referenceOnlyIds"],json!(["mother"]));
    let mut mother=f.store.active_asset("mother").unwrap();mother["variantId"]=json!("v2");
    f.store.save_asset(mother).unwrap();
    assert_eq!(f.store.image_production_scope().unwrap()["referenceOnlyIds"],json!([]),"old registration also cannot authorize a replacement mother variant");
}

#[test]
fn reference_support_chain_resolves_original_md_without_forging_ancestor_visual_use() {
    let f=Fixture::new();f.import_md();
    let formal=f.store.image_production_scope().unwrap();let leaf_id=formal["assetIds"][0].as_str().unwrap();
    let root=f.asset("root-reference");let mut middle=f.asset("middle-reference");
    middle["referenceRequirements"]=json!([{"targetAssetId":"root-reference","role":"reference"}]);
    middle=f.store.save_asset(middle).unwrap();
    let mut leaf=f.store.active_asset(leaf_id).unwrap();leaf["referenceRequirements"]=json!([{"targetAssetId":"middle-reference","role":"reference"}]);
    f.store.save_asset(leaf).unwrap();
    f.store.register_image_reference(&json!({"assetId":"middle-reference","expectedVersion":middle["version"],"servesAssetIds":[leaf_id],"reason":"提供明确参考","confirmed":true})).unwrap();
    f.store.register_image_reference(&json!({"assetId":"root-reference","expectedVersion":root["version"],"servesAssetIds":["middle-reference"],"reason":"提供中间母版所需参考","confirmed":true})).unwrap();
    let input=f.store.image_workflow_input(&json!({"assetIds":["root-reference"]}),&json!({})).unwrap();
    assert_eq!(input["imageSource"]["sourceKind"],"imported-md");
    assert_eq!(input["imageSource"]["text"],MD);
    assert!(input["imageSource"].get("lockedSource").is_none());
    assert_eq!(input["selection"]["assetIds"],json!(["root-reference"]));
    assert_eq!(input["imageQueue"]["items"][0]["uses"],json!([]),"ancestor must not be claimed visible in the MD");
    assert!(f.store.source().unwrap().is_null());
    assert!(f.store.jobs().unwrap().is_empty());
}
