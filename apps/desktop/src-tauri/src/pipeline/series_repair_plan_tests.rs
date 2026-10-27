use super::*;

pub(super) fn fixture_with_frozen_sound_sources() -> (Value, Value) {
    let (mut input, mut authored) = fixture();
    let locked = super::super::super::prepare_source(
        "第一集\n场景：门口\n甲：来了。\n场景：窗边\n甲：等下。\n",
    )
    .unwrap();
    let eid = &locked.episodes[0].id;
    let refs: Vec<_> = locked
        .rows
        .iter()
        .filter(|r| r.kind == "body")
        .map(|r| r.id.clone())
        .collect();
    let scene_ids: Vec<_> = locked.scenes.iter().map(|s| s.id.clone()).collect();
    assert_eq!(scene_ids.len(), 2);
    let mut second_scene = input["assets"][1].clone();
    second_scene["id"] = json!("s2");
    second_scene["name"] = json!("窗边");
    let mut sound = input["assets"][0].clone();
    sound["id"] = json!("voice");
    sound["name"] = json!("画外声音");
    sound["kind"] = json!("voice");
    sound["preparation"] = json!({"mode":"AUDIO"});
    input["assets"]
        .as_array_mut()
        .unwrap()
        .extend([second_scene, sound]);
    for asset in input["assets"].as_array_mut().unwrap() {
        asset["applicability"] = json!({"episodes":[eid],"scenes":scene_ids});
        asset["adoptedDesign"]["assetId"] = asset["id"].clone();
        asset["adoptedDesign"]["sourceSha256"] = json!(locked.sha256);
        asset["adoptedDesign"]["sourceRefs"] = json!(refs);
    }
    let unit_template = authored["draft"]["units"][0].clone();
    let event_template = authored["plan"]["events"][0].clone();
    let package_template = authored["plan"]["workPackages"][0].clone();
    let mut units = vec![];
    let mut events = vec![];
    let mut packages = vec![];
    let mut provisional = vec![];
    for (i, sid) in scene_ids.iter().enumerate() {
        let rows: Vec<_> = locked
            .rows
            .iter()
            .filter(|r| r.kind == "body" && r.scene_id.as_deref() == Some(sid))
            .collect();
        let local: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
        let quote = rows.iter().find_map(|r| r.dialogue.as_ref()).unwrap();
        let event_id = format!("E{}", i + 1);
        let package_id = format!("W{}", i + 1);
        let scene_asset = if i == 0 { "s" } else { "s2" };
        let boundary = json!({"kind":if i==0 {"episode_start"} else {"time_space_jump"},"reason":"对应原文场次边界"});
        let mut unit = unit_template.clone();
        unit["id"] = json!(format!("U{}", i + 1));
        unit["episodeId"] = json!(eid);
        unit["eventId"] = json!(event_id);
        unit["sceneId"] = json!(sid);
        unit["scene"] = json!(if i == 0 { "门口" } else { "窗边" });
        unit["boundaryBefore"] = boundary.clone();
        unit["sourceRefs"] = json!(local);
        unit["assetRefs"] = json!([
            {"assetId":"a","variantId":"base","use":"character","order":1,"sourceRefs":local},
            {"assetId":scene_asset,"variantId":"base","use":"scene","order":2,"sourceRefs":local},
            {"assetId":"voice","variantId":"base","use":"audio","order":3,"sourceRefs":local}
        ]);
        unit["imageBindings"] = json!(["a", scene_asset]);
        unit["shots"][0]["id"] = json!(format!("S{}", i + 1));
        unit["shots"][0]["sourceRefs"] = json!(local);
        let part = &mut unit["shots"][0]["dialogueParts"][0];
        part["lineId"] = json!(quote.id);
        part["quoteStart"] = json!(quote.range.start);
        part["quoteEnd"] = json!(quote.range.end);
        units.push(unit);
        let mut event = event_template.clone();
        event["id"] = json!(event_id);
        event["sceneId"] = json!(sid);
        event["sourceRefs"] = json!(local);
        event["boundaryBefore"] = boundary;
        events.push(event);
        let mut package = package_template.clone();
        package["id"] = json!(package_id);
        package["eventId"] = json!(event_id);
        package["sourceRefs"] = json!(local);
        packages.push(package);
        provisional.push(json!({"id":format!("P{}",i+1),"packageId":package_id,"sourceRefs":local,"secondsRange":[3.2,3.2]}));
    }
    authored["plan"]["events"] = json!(events);
    authored["plan"]["workPackages"] = json!(packages);
    authored["plan"]["provisionalUnits"] = json!(provisional);
    authored["draft"]["units"] = json!(units);
    input["source"] = json!(locked);
    input["selection"]["episodeIds"] = json!([eid]);
    input["productionScope"]["episodeIds"] = json!([eid]);
    input["productionScope"]["sourceSha256"] = json!(locked.sha256);
    mark_new_legacy_fixture(&mut input);
    (input, authored)
}

pub(crate) async fn series_repair_replay_fixture() -> (Value, TextRequest, Value, TextRequest, Value)
{
    let (input, mut authored) = fixture_with_frozen_sound_sources();
    authored["plan"]["workPackages"][0]["emotionContract"]["contextTone"] =
        json!("工作包只刻画开口前的迟疑，事件仍整体由等待进入确认");
    let mut executor = Offline::new(authored);
    executor.review_rounds = 1;
    let output = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(output["status"], "completed");
    let calls = executor.requests.lock().unwrap();
    let repair = calls
        .iter()
        .find(|r| r.stage == "seriesRepair")
        .unwrap()
        .clone();
    let reviewer = calls
        .iter()
        .find(|r| r.stage == "seriesReview")
        .unwrap()
        .clone();
    let cache = executor.cache.lock().unwrap();
    let raw = cache[&digest(&serde_json::to_string(&repair).unwrap())].clone();
    let review_raw = cache[&digest(&serde_json::to_string(&reviewer).unwrap())].clone();
    (input, repair, raw, reviewer, review_raw)
}

#[tokio::test]
async fn frozen_repair_plan_keeps_distinct_beat_contract_and_original_raw() {
    let (input, request, raw, reviewer, review_raw) = series_repair_replay_fixture().await;
    let before = json!({"schemaVersion":"series-episode-candidate/v1",
        "plan":binding(&request,"plan"),"selection":binding(&request,"selection"),"draft":binding(&request,"draft")});
    assert_ne!(
        before["plan"]["workPackages"][0]["beatContracts"][0]["emotionContract"],
        before["plan"]["workPackages"][0]["emotionContract"]
    );
    assert!(raw["plan"]["workPackages"][0]
        .get("beatContracts")
        .is_none());
    let saved = serde_json::to_vec(&raw).unwrap();
    let frozen = serde_json::to_vec(&request).unwrap();
    let source = source(&input).unwrap();
    let episode = &source.episodes[0];
    let assets = binding(&request, "assets").as_array().unwrap();
    let old = validate_candidate(&raw, &source, episode, assets, &input).unwrap();
    assert_eq!(
        assert_local_repair(&before, &old, &review_raw["issues"]).unwrap_err(),
        repair::PLAN_CHANGED
    );
    let checked = repair::validate_scoped_candidate(
        &request,
        &raw,
        &source,
        episode,
        assets,
        &input,
        Some((&before, &review_raw["issues"])),
        None,
    )
    .unwrap();
    assert_eq!(checked["plan"], before["plan"]);
    assert_ne!(checked["draft"], before["draft"]);
    assert_eq!(checked["draft"]["units"][1], before["draft"]["units"][1]);
    for i in 0..2 {
        assert!(raw["draft"]["units"][i].get("soundSources").is_none());
        assert_eq!(
            before["draft"]["units"][i]["soundSources"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            checked["draft"]["units"][i]["soundSources"],
            before["draft"]["units"][i]["soundSources"]
        );
    }
    let proof =
        revalidate_review_repair_response(&request, &raw, &reviewer, &review_raw, &input).unwrap();
    assert_eq!(proof["validatedCandidateSha256"], hash(&checked));
    assert_eq!(serde_json::to_vec(&raw).unwrap(), saved);
    assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
}

#[tokio::test]
async fn frozen_repair_plan_rejects_writable_changes_selection_unknown_and_schema_drift() {
    let (input, request, raw, reviewer, review_raw) = series_repair_replay_fixture().await;
    for fault in [
        "event-emotion",
        "package-emotion",
        "source-order",
        "unknown-plan",
        "unknown-package",
        "selection",
        "schema",
        "issues",
        "review",
        "role",
        "protected-camera",
        "protected-voice-window",
        "protected-source",
        "protected-delete",
        "unit-order",
        "forged-sound-sources",
        "unknown-unit-field",
        "repaired-unit-removes-sound-scope",
    ] {
        let mut changed = raw.clone();
        let mut req = request.clone();
        let mut review = review_raw.clone();
        match fault {
            "event-emotion" => {
                changed["plan"]["events"][0]["emotionContract"]["contextTone"] = json!("changed")
            }
            "package-emotion" => {
                changed["plan"]["workPackages"][0]["emotionContract"]["contextTone"] =
                    json!("changed")
            }
            "source-order" => changed["plan"]["events"][0]["sourceRefs"] = json!([]),
            "unknown-plan" => changed["plan"]["sourceFingerprints"] = json!({"forged":true}),
            "unknown-package" => changed["plan"]["workPackages"][0]["beatContracts"] = json!([]),
            "selection" => changed["selection"] = json!(["different"]),
            "schema" => {
                req.schema["properties"]["plan"]["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("emotionArc");
            }
            "issues" => {
                review["issues"][0]["requestedChange"] = json!("changed");
            }
            "review" => review["candidateSha256"] = json!("other"),
            "role" => {
                req.input["executionRoleId"] = json!(format!(
                    "{}/validation-repair/1",
                    s(&request.input, "executionRoleId")
                ))
            }
            "protected-camera" => {
                changed["draft"]["units"][1]["shots"][0]["camera"] = json!("changed")
            }
            "protected-voice-window" => {
                changed["draft"]["units"][1]["shots"][0]["dialogueParts"][0]["voiceEnd"] =
                    json!(1.4)
            }
            "protected-source" => changed["draft"]["units"][1]["sourceRefs"] = json!([]),
            "protected-delete" => {
                changed["draft"]["units"].as_array_mut().unwrap().remove(1);
            }
            "unit-order" => changed["draft"]["units"].as_array_mut().unwrap().swap(0, 1),
            "forged-sound-sources" => changed["draft"]["units"][1]["soundSources"] = json!([]),
            "unknown-unit-field" => changed["draft"]["units"][1]["unknown"] = json!(true),
            "repaired-unit-removes-sound-scope" => {
                changed["draft"]["units"][0]["sourceRefs"] = json!([])
            }
            _ => unreachable!(),
        }
        let before = serde_json::to_vec(&changed).unwrap();
        assert!(
            revalidate_review_repair_response(&req, &changed, &reviewer, &review, &input).is_err(),
            "{fault}"
        );
        assert_eq!(serde_json::to_vec(&changed).unwrap(), before, "{fault}");
    }
}

#[tokio::test]
async fn frozen_repair_plan_full_independent_review_and_replay_keep_one_author() {
    let (input, authored) = fixture_with_frozen_sound_sources();
    let (_directory, input, mut executor) = durable_from(input, authored);
    executor.generator.authored["plan"]["workPackages"][0]["emotionContract"]["contextTone"] =
        json!("distinct preserved beat");
    executor.generator.review_rounds = 1;
    let output = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    let count = executor.store.jobs().unwrap().len();
    let replay = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(output, replay);
    assert_eq!(executor.store.jobs().unwrap().len(), count);
    let calls = executor.generator.requests.lock().unwrap();
    assert_eq!(
        calls.iter().filter(|r| r.stage == "seriesAuthor").count(),
        1
    );
    assert_eq!(
        calls.iter().filter(|r| r.stage == "seriesReview").count(),
        2
    );
    assert_eq!(
        calls.iter().filter(|r| r.stage == "seriesRepair").count(),
        1
    );
    assert_eq!(calls.last().unwrap().stage, "seriesReview");
    assert_eq!(
        binding(calls.last().unwrap(), "plan"),
        &output["seriesDeliveries"][0]["bundle"]["plan"]
    );
    validate_delivery(&input, &output["seriesDeliveries"][0]).unwrap();
}
