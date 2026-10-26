use super::*;
use std::sync::Mutex;

fn costume_fixture() -> (Fixture, Value) {
    let f = Fixture::new();
    f.import_md();
    f.asset("mother");
    let mut child = f
        .store
        .values("assets")
        .unwrap()
        .into_iter()
        .find(|a| a["lingmoType"] == "CH")
        .unwrap();
    // Store::save_asset persists a complete record. A partial test value would
    // discard the parser token and provenance that own the exact MD filename.
    assert_eq!(child["origin"], "imported-storyboard");
    assert_eq!(child["token"], "@[甲CH]");
    child["expectedVersion"] = child["version"].clone();
    child["identityId"] = json!("person");
    child["imageTask"] = json!({"kind":"costume"});
    child["referenceRequirements"] = json!([{"targetAssetId":"mother","targetVariantId":"v1","role":"identity","required":true}]);
    let child = f.store.save_asset(child).unwrap();
    (f, child)
}
fn input(f: &Fixture, child: &Value) -> Value {
    let input = f
        .store
        .image_workflow_input(
            &json!({"action":"images.design","assetIds":[child["id"]]}),
            &json!({}),
        )
        .unwrap();
    assert_eq!(input["imageSource"]["sourceKind"], "imported-md");
    assert!(input["imageSource"].get("lockedSource").is_none());
    let item = input["imageQueue"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["assetId"] == child["id"])
        .unwrap();
    assert_eq!(item["filename"], "甲CH.png");
    input
}
fn register(f: &Fixture) -> Value {
    let path = f.dir.path().join("mother.png");
    image::RgbaImage::from_fn(12, 12, |x, y| {
        image::Rgba([30 + x as u8, 40 + y as u8, 70, 255])
    })
    .save(&path)
    .unwrap();
    let media = f
        .store
        .import_image_as_current(&path, "mother", Some("v1"))
        .unwrap();
    f.store
        .save_identity_layout(
            &json!({"assetId":"mother","variantId":"v1","mediaId":media["id"],
        "expectedSha256":media["sha256"],"topHeight":4,"confirmed":true}),
        )
        .unwrap();
    media
}
fn mother(input: &Value) -> &Value {
    input["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "mother")
        .unwrap()
}

#[derive(Default)]
struct LayoutAuthor {
    requests: Mutex<Vec<TextRequest>>,
    outputs: Mutex<Vec<Value>>,
}
#[async_trait]
impl TextExecutor for LayoutAuthor {
    async fn execute(&self, r: TextRequest) -> Result<Value> {
        self.requests.lock().unwrap().push(r.clone());
        let mut output = OfflineText::default().execute(r.clone()).await?;
        if r.input["costumePromptLayoutContract"] != asset_workflow::COSTUME_PROMPT_V1 {
            output["unresolved"] = json!(["未提供真实母图像素尺寸和上下分界，派发前需登记"]);
        }
        self.outputs.lock().unwrap().push(output.clone());
        Ok(output)
    }
}

#[tokio::test]
async fn registered_geometry_is_supplied_to_author_without_image_service_or_visual_claim() {
    let (f, child) = costume_fixture();
    let media = register(&f);
    let input = input(&f, &child);
    let evidence = &mother(&input)["identityLayoutSupply"];
    assert_eq!(evidence["status"], "registered");
    assert_eq!(evidence["mediaId"], media["id"]);
    assert_eq!(evidence["width"], 12);
    assert_eq!(evidence["identityStrip"]["height"], 4);
    assert_eq!(evidence["imageEvidence"], "NO_PIXELS_READ");
    assert_eq!(evidence["visualQualityApproved"], false);
    let author = LayoutAuthor::default();
    let output = asset_workflow::run_asset_prompts(&input, &author, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["promptCount"], 1, "{output}");
    let requests = author.requests.lock().unwrap();
    assert_eq!(
        requests[0].input["costumeComposition"]["motherReferences"][0]["registeredLayout"],
        *evidence
    );
    assert!(evidence.get("path").is_none());
    assert!(f.store.jobs().unwrap().is_empty());
    assert_eq!(f.store.values("media").unwrap().len(), 1);
}

#[tokio::test]
async fn missing_and_stale_layouts_defer_dispatch_but_do_not_block_complete_words() {
    for stale in [false, true] {
        let (f, child) = costume_fixture();
        if stale {
            let media = register(&f);
            fs::write(s(&media, "path"), b"changed original media").unwrap();
        }
        let input = input(&f, &child);
        assert_eq!(
            mother(&input)["identityLayoutSupply"]["status"],
            "deferred-to-image-dispatch"
        );
        let output = asset_workflow::run_asset_prompts(
            &input,
            &LayoutAuthor::default(),
            "offline",
            "medium",
        )
        .await
        .unwrap();
        assert_eq!(output["status"], "completed", "{output}");
        assert_eq!(output["promptCount"], 1);
        assert_eq!(output["imageGenerationSubmissions"], 0);
        assert!(f.store.jobs().unwrap().is_empty());
        let mut generation = json!({"imagePresentation":{"layoutKind":"costume"},"prompt":"完整服装提示词",
            "referenceRequirements":child["referenceRequirements"],"references":[]});
        assert!(crate::asset_identity_layout::freeze(&f.store, &mut generation).is_err());
    }
}

#[tokio::test]
async fn public_word_recompile_preserves_completed_unpublished_raw_and_original_workflow() {
    let (f, child) = costume_fixture();
    register(&f);
    let mut old_input = input(&f, &child);
    old_input
        .as_object_mut()
        .unwrap()
        .remove("costumePromptLayoutContract");
    for asset in old_input["assets"].as_array_mut().unwrap() {
        asset
            .as_object_mut()
            .unwrap()
            .remove("identityLayoutSupply");
    }
    let old_input_before = old_input.clone();
    let author = LayoutAuthor::default();
    let old_workflow = f.workflow(&old_input);
    let old_output = asset_workflow::run_asset_prompts(&old_input, &author, "offline", "medium")
        .await
        .unwrap();
    let published = f
        .store
        .finish_image_prompts(&old_input, &old_output, &old_workflow)
        .unwrap();
    assert_eq!(published["promptCount"], 0);
    assert_eq!(published["status"], "partially_blocked");
    let raw = author.outputs.lock().unwrap()[0].clone();
    let old_request = author.requests.lock().unwrap()[0].clone();
    let old_text = f
        .store
        .add_job(
            "text",
            "assetDesign:synthetic",
            "offline",
            "offline",
            &serde_json::to_value(&old_request).unwrap(),
            "completed",
        )
        .unwrap();
    f.store
        .update_job(
            &old_text,
            "completed",
            &json!({"result":raw,"receipt":{"validation":"accepted"}}),
        )
        .unwrap();
    let raw_file = f.dir.path().join("original-raw.json");
    fs::write(&raw_file, raw.to_string()).unwrap();
    let raw_bytes = fs::read(&raw_file).unwrap();
    let before_workflow = f.store.job(&old_workflow).unwrap();
    let before_text = f.store.job(&old_text).unwrap();
    // This is the same public Store + pipeline chain called by images.design;
    // a fresh input uses the new supply contract, never edits the old result.
    let current = input(&f, &child);
    let run = f.workflow(&current);
    let output = asset_workflow::run_asset_prompts(&current, &author, "offline", "medium")
        .await
        .unwrap();
    let published = f
        .store
        .finish_image_prompts(&current, &output, &run)
        .unwrap();
    assert_eq!(published["promptCount"], 1, "{published}");
    assert_eq!(published["status"], "completed");
    assert_ne!(author.requests.lock().unwrap()[1].input, old_request.input);
    assert_eq!(f.store.job(&old_workflow).unwrap(), before_workflow);
    assert_eq!(f.store.job(&old_text).unwrap(), before_text);
    assert_eq!(old_input, old_input_before);
    assert_eq!(fs::read(raw_file).unwrap(), raw_bytes);
    assert_eq!(
        raw["unresolved"],
        json!(["未提供真实母图像素尺寸和上下分界，派发前需登记"])
    );
    assert!(!s(
        &f.store.active_asset(s(&child, "id")).unwrap()["imagePrompt"],
        "text"
    )
    .is_empty());
    let again = input(&f, &child);
    let output = asset_workflow::run_asset_prompts(&again, &author, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["promptCount"], 1);
    assert_eq!(
        author.requests.lock().unwrap().len(),
        2,
        "completed words must be cached"
    );
    assert!(f
        .store
        .jobs()
        .unwrap()
        .iter()
        .all(|j| j["kind"] != "image" && j["kind"] != "video"));
}
