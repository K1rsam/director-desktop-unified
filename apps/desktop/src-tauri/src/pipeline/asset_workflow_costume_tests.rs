use super::*;

fn input(registered: bool) -> Value {
    let mut input = fixture();
    input["assetPromptRequestVersion"] = json!(REQUEST_V2);
    input["costumePromptLayoutContract"] = json!(COSTUME_PROMPT_V1);
    input["selection"] = json!({"assetIds":["suit"]});
    if registered {
        input["assets"][0]["adoptedMediaId"] = json!("mother-media");
        input["assets"][0]["identityLayoutSupply"] = json!({"version":"asset-inspected-layout-supply/v1",
            "status":"registered","assetId":"base","variantId":"base-v1","mediaId":"mother-media","sha256":"a".repeat(64),
            "width":1254,"height":1254,"identityStrip":{"x":0,"y":0,"width":1254,"height":472},
            "identityCopyMode":"full-width-strip-original-pixels","individualFaceSlotsRequired":false,
            "fileBytesAndDimensionsVerified":true,"imageEvidence":"NO_PIXELS_READ","visualQualityApproved":false});
    }
    input
}
fn request(input: &Value) -> TextRequest {
    build_request(
        input,
        &input["assets"][1],
        &input["imageQueue"]["items"][1],
        "offline",
        "medium",
    )
    .unwrap()
}

#[test]
fn registered_strip_reaches_author_as_metadata_without_requiring_individual_face_slots() {
    let input = input(true);
    let before = input.clone();
    let r = request(&input);
    let composition = &r.input["costumeComposition"];
    let registered = &composition["motherReferences"][0]["registeredLayout"];
    assert_eq!(registered["width"], 1254);
    assert_eq!(registered["height"], 1254);
    assert_eq!(registered["identityStrip"]["height"], 472);
    assert_eq!(composition["individualFaceSlotsRequired"], false);
    assert_eq!(composition["imageEvidence"], "NO_PIXELS_READ");
    assert_eq!(composition["visualQualityApproved"], false);
    assert!(s(&r.input, "instruction").contains("禁止再要求三张脸各自槽位坐标"));
    assert!(s(&r.input, "instruction").contains("不应写进本次 unresolved"));
    assert!(registered.get("path").is_none());
    assert_eq!(input, before);
}

#[test]
fn legacy_frozen_requests_do_not_gain_new_layout_instructions_or_fields() {
    let mut old = input(true);
    old.as_object_mut()
        .unwrap()
        .remove("costumePromptLayoutContract");
    let before = old.clone();
    let r = request(&old);
    assert!(r.input.get("costumeComposition").is_none());
    assert!(!s(&r.input, "instruction").contains(COSTUME_PROMPT_V1));
    assert_eq!(old, before);
    let mut new = old.clone();
    new["costumePromptLayoutContract"] = json!(COSTUME_PROMPT_V1);
    assert_ne!(
        request_fingerprint(&r).unwrap(),
        request_fingerprint(&request(&new)).unwrap()
    );
    new["costumePromptLayoutContract"] = json!("future/v99");
    assert!(build_request(
        &new,
        &new["assets"][1],
        &new["imageQueue"]["items"][1],
        "offline",
        "medium"
    )
    .is_err());
}

#[test]
fn geometry_updates_do_not_repurchase_completed_clothing_words() {
    let mut input = input(true);
    let before = request(&input);
    input["assets"][0]["adoptedMediaId"] = json!("new-mother");
    input["assets"][0]["identityLayoutSupply"]["mediaId"] = json!("new-mother");
    input["assets"][0]["identityLayoutSupply"]["sha256"] = json!("b".repeat(64));
    input["assets"][0]["identityLayoutSupply"]["width"] = json!(1536);
    input["assets"][0]["identityLayoutSupply"]["height"] = json!(1536);
    input["assets"][0]["identityLayoutSupply"]["identityStrip"] =
        json!({"x":0,"y":0,"width":1536,"height":512});
    let after = request(&input);
    assert_ne!(before.input, after.input);
    assert_eq!(
        request_fingerprint(&before).unwrap(),
        request_fingerprint(&after).unwrap()
    );
    input["assets"][0]["identityLayoutSupply"] = Value::Null;
    assert_eq!(
        request_fingerprint(&before).unwrap(),
        request_fingerprint(&request(&input)).unwrap()
    );
    assert_eq!(source_basis(&before), source_basis(&after));
}

#[tokio::test]
async fn missing_mother_or_unregistered_layout_allows_complete_words_without_image_account() {
    let input = input(false);
    let recorder = Recorder::default();
    let output = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["status"], "completed", "{output}");
    assert_eq!(output["promptCount"], 1);
    assert_eq!(output["imagePlan"][0]["status"], "waiting_reference");
    assert_eq!(output["imageGenerationSubmissions"], 0);
    assert!(!s(&output["assets"][0]["imagePrompt"], "text").is_empty());
    let r = &recorder.requests.lock().unwrap()[0];
    assert_eq!(
        r.input["costumeComposition"]["promptMayProceedWithoutLayout"],
        true
    );
    assert_eq!(
        r.input["costumeComposition"]["motherReferences"][0]["registeredLayout"],
        Value::Null
    );
    assert_eq!(r.input["imageEvidence"], "NO_PIXELS_READ");
}

#[tokio::test]
async fn author_unresolved_is_preserved_instead_of_being_silently_cleared() {
    let input = input(true);
    let recorder = Recorder {
        failure: Some(("suit", "unresolved")),
        ..Default::default()
    };
    let output = run_asset_prompts(&input, &recorder, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["status"], "partially_blocked");
    assert_eq!(output["promptCount"], 0);
    assert_eq!(
        output["blocked"][0]["unresolved"],
        json!(["该分镜未说明两位同名人物对应的身份"])
    );
    let original = recorder.accepted.lock().unwrap()[0].clone();
    assert_eq!(original["unresolved"], output["blocked"][0]["unresolved"]);
    let mut mechanical = original.clone();
    mechanical["unresolved"] = json!(["缺少母图真实像素尺寸"]);
    let r = request(&input);
    let copy = mechanical.clone();
    assert_eq!(
        finalize_output(&r, &input["assets"][1], &mechanical).unwrap()["status"],
        "blocked"
    );
    assert_eq!(mechanical, copy);
}

#[test]
fn stale_or_malformed_layout_metadata_is_deferred_not_forged_or_a_text_block() {
    for field in [
        "assetId",
        "variantId",
        "mediaId",
        "sha256",
        "width",
        "fileBytesAndDimensionsVerified",
        "imageEvidence",
    ] {
        let mut input = input(true);
        input["assets"][0]["identityLayoutSupply"][field] = Value::Null;
        let r = request(&input);
        assert_eq!(
            r.input["costumeComposition"]["motherReferences"][0]["registeredLayout"],
            Value::Null,
            "{field}"
        );
        assert_eq!(
            r.input["costumeComposition"]["promptMayProceedWithoutLayout"],
            true
        );
    }
}
