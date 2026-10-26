use super::*;

pub(crate) async fn series_frame_render_fixture(
) -> (Value, TextRequest, TextRequest, Value, Value, Value) {
    let (mut input, authored) = repair_plan_tests::fixture_with_frozen_sound_sources();
    input["settings"]["ratio"] = json!("16:9");
    input
        .as_object_mut()
        .unwrap()
        .remove(frame_render::PROFILE_FIELD);
    let mut executor = Offline::new(authored);
    executor.review_rounds = 1;
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["status"], "completed", "{}", output["blocked"]);
    let calls = executor.requests.lock().unwrap();
    let projected = calls
        .iter()
        .find(|r| r.stage == "seriesRepair")
        .unwrap()
        .clone();
    let review = calls.iter().find(|r| r.stage == "seriesReview").unwrap();
    let cache = executor.cache.lock().unwrap();
    let raw = cache[&digest(&serde_json::to_string(&projected).unwrap())].clone();
    let review_raw = &cache[&digest(&serde_json::to_string(review).unwrap())];
    let receipt = role_receipt(review, review_raw).unwrap();
    let mut legacy = projected.clone();
    legacy
        .input
        .as_object_mut()
        .unwrap()
        .remove("seriesFrameRenderRepair");
    (input, legacy, projected, raw, receipt, output)
}

#[tokio::test]
async fn series_frame_render_review_repair_keeps_raw_old_roles_unaffected_units_and_final_review() {
    let (input, legacy, projected, raw, review, output) = series_frame_render_fixture().await;
    validate_frame_render_projection(&legacy, &projected, &input, &review).unwrap();
    let original_raw = raw.clone();
    let source = source(&input).unwrap();
    let episode = &source.episodes[0];
    let assets = binding(&legacy, "assets").as_array().unwrap();
    let before = json!({"schemaVersion":"series-episode-candidate/v1","plan":binding(&legacy,"plan"),"selection":binding(&legacy,"selection"),"draft":binding(&legacy,"draft")});
    let old = repair::validate_scoped_candidate(
        &legacy,
        &raw,
        &source,
        episode,
        assets,
        &input,
        Some((&before, &review["raw"]["issues"])),
        None,
    )
    .unwrap();
    let new = repair::validate_scoped_candidate(
        &projected,
        &raw,
        &source,
        episode,
        assets,
        &input,
        Some((&before, &review["raw"]["issues"])),
        None,
    )
    .unwrap();
    assert_eq!(original_raw, raw);
    assert_eq!(review["raw"]["conclusion"], "revise");
    assert_eq!(review["raw"]["candidateSha256"], hash(&before));
    assert_eq!(new["plan"], old["plan"]);
    assert_eq!(
        new["draft"]["units"][0]["shots"],
        old["draft"]["units"][0]["shots"]
    );
    assert!(s(&new["draft"]["units"][0], "prompt").starts_with("画幅：16:9\n\n"));
    assert_eq!(new["draft"]["units"][1], before["draft"]["units"][1]);
    assert!(old["draft"]["units"][0].get("promptRender").is_none());
    assert_eq!(
        new["draft"]["units"][0]["promptRender"]["version"],
        "frame-render/v1"
    );
    assert_eq!(
        validate_candidate(&new, &source, episode, assets, &input).unwrap(),
        new
    );
    let delivery = &output["seriesDeliveries"][0];
    assert!(s(delivery, "markdown").contains("；画幅：16:9"));
    assert_eq!(s(delivery, "markdown").matches("；画幅：16:9").count(), 1);
    assert_eq!(delivery["review"]["candidateSha256"], hash(&new));
    validate_delivery(&input, delivery).unwrap();
    let mut tampered = projected.clone();
    tampered.input["seriesFrameRenderRepair"]["ratio"] = json!("9:16");
    assert!(validate_frame_render_projection(&legacy, &tampered, &input, &review).is_err());
    assert!(repair::validate_scoped_candidate(
        &tampered,
        &raw,
        &source,
        episode,
        assets,
        &input,
        Some((&before, &review["raw"]["issues"])),
        None
    )
    .is_err());
}

#[tokio::test]
async fn series_frame_render_new_profile_reopens_without_dispatch_and_exports_matching_ratio() {
    let (mut input, authored) = fixture();
    input["settings"]["ratio"] = json!("9:16");
    mark_input(&mut input).unwrap();
    assert_eq!(input[frame_render::PROFILE_FIELD], "frame-render/v1");
    let (_temp, input, mut executor) = durable_from(input, authored);
    executor.generator.review_rounds = 1;
    let result = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(result["status"], "completed", "{}", result["blocked"]);
    let count = executor.store.jobs().unwrap().len();
    let replay = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(result, replay);
    assert_eq!(executor.store.jobs().unwrap().len(), count);
    let delivered = &result["seriesDeliveries"][0];
    assert!(s(delivered, "markdown").contains("画幅：9:16"));
    assert!(s(&delivered["bundle"]["draft"]["units"][0], "prompt").starts_with("画幅：9:16\n\n"));
    validate_delivery(&input, delivered).unwrap();
    for request in executor.generator.requests.lock().unwrap().iter() {
        assert_eq!(
            request.input[frame_render::PROFILE_FIELD],
            "frame-render/v1"
        );
        assert!(request.input.get("seriesFrameRenderRepair").is_none());
    }
    let mut wrong = input.clone();
    wrong[frame_render::PROFILE_FIELD] = json!("future");
    assert!(validate_profile(&wrong).is_err());
    wrong = input;
    wrong["settings"]["ratio"] = json!("0:9");
    assert!(validate_profile(&wrong).is_err());
}

#[tokio::test]
async fn series_frame_render_rejects_forged_review_hash_scope_raw_metadata_and_adoption() {
    let (input, legacy, projected, raw, review, output) = series_frame_render_fixture().await;
    for field in [
        "ratio",
        "directionSha256",
        "candidateSha256",
        "reviewRequestSha256",
        "reviewRawSha256",
        "issuesSha256",
        "affectedUnitIds",
        "legacyRequestSha256",
        "executorContract",
    ] {
        let mut changed = projected.clone();
        changed.input["seriesFrameRenderRepair"][field] = json!("changed");
        assert!(
            validate_frame_render_projection(&legacy, &changed, &input, &review).is_err(),
            "{field}"
        );
    }
    let mut changed_review = review.clone();
    changed_review["raw"]["conclusion"] = json!("pass");
    assert!(
        validate_frame_render_projection(&legacy, &projected, &input, &changed_review).is_err()
    );
    let mut changed = output["seriesDeliveries"][0].clone();
    let roles = changed["adoptionReceipt"]["roleReceipts"]
        .as_array_mut()
        .unwrap();
    roles.retain(|r| r["role"] != "repair");
    assert!(frame_render::validate_adoption(&input, &changed).is_err());
    let mut raw = raw;
    raw["draft"]["units"][1]["promptRender"] = json!({"version":"frame-render/v1","ratio":"16:9"});
    let source = source(&input).unwrap();
    let before = json!({"schemaVersion":"series-episode-candidate/v1","plan":binding(&legacy,"plan"),"selection":binding(&legacy,"selection"),"draft":binding(&legacy,"draft")});
    assert!(repair::validate_scoped_candidate(
        &projected,
        &raw,
        &source,
        &source.episodes[0],
        binding(&legacy, "assets").as_array().unwrap(),
        &input,
        Some((&before, &review["raw"]["issues"])),
        None
    )
    .is_err());
}

#[tokio::test]
async fn series_frame_render_later_split_inherits_frozen_rendered_source_scope() {
    let (input, _, projected, raw, review, _) = series_frame_render_fixture().await;
    let source = source(&input).unwrap();
    let before = json!({"schemaVersion":"series-episode-candidate/v1","plan":binding(&projected,"plan"),"selection":binding(&projected,"selection"),"draft":binding(&projected,"draft")});
    let checked = repair::validate_scoped_candidate(
        &projected,
        &raw,
        &source,
        &source.episodes[0],
        binding(&projected, "assets").as_array().unwrap(),
        &input,
        Some((&before, &review["raw"]["issues"])),
        None,
    )
    .unwrap();
    let mut later = projected.clone();
    later
        .input
        .as_object_mut()
        .unwrap()
        .remove("seriesFrameRenderRepair");
    let mut split = checked.clone();
    split["draft"]["units"][0]["id"] = json!("new-local-unit-id");
    split["draft"]["units"][0]
        .as_object_mut()
        .unwrap()
        .remove("promptRender");
    frame_render::restore_for_request(
        &later,
        &mut split,
        &input,
        Some((&checked, &review["raw"]["issues"])),
    )
    .unwrap();
    assert_eq!(
        split["draft"]["units"][0]["promptRender"],
        checked["draft"]["units"][0]["promptRender"]
    );
    assert!(split["draft"]["units"][1].get("promptRender").is_none());
    split["draft"]["units"][0]["sourceRefs"] = checked["draft"]["units"][1]["sourceRefs"].clone();
    split["draft"]["units"][0]
        .as_object_mut()
        .unwrap()
        .remove("promptRender");
    frame_render::restore_for_request(
        &later,
        &mut split,
        &input,
        Some((&checked, &review["raw"]["issues"])),
    )
    .unwrap();
    assert!(
        split["draft"]["units"][0].get("promptRender").is_none(),
        "an unrelated source scope must not inherit the renderer"
    );
}
