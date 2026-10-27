use super::*;

pub(crate) fn settings() -> Value {
    let coverage = json!({"form":"自然三维国漫造型","rendering":"连续柔和明暗，不使用赛璐璐色块","materials":"布料与金属有不同反射","colorLighting":"统一冷暖且暗部可读","aspectRatio":"画幅16:9","localExceptions":"局部Q版仅按采用镜头的进入退出回接"});
    let text = format!(" {}。", visual_baseline::VERSION) + &coverage.as_object().unwrap().values().map(|v|v.as_str().unwrap()).collect::<Vec<_>>().join("；") + "。 ";
    json!({"ratio":"16:9","visualStyle":text,"projectVisualStyle":text,"visualBaseline":{"version":visual_baseline::VERSION,"id":"synthetic-baseline","revision":1,"status":"adopted","visualStyle":text,"coverage":coverage,"adoptionBasis":"synthetic explicit project adoption"}})
}

fn current_fixture() -> (Value, Value) {
    let (mut input, authored) = fixture();
    input.as_object_mut().unwrap().remove("seriesSkillVersion");
    input.as_object_mut().unwrap().remove("seriesRulePackSha256");
    for (key, value) in settings().as_object().unwrap() { input["settings"][key] = value.clone(); }
    mark_input(&mut input).unwrap();
    (input, authored)
}

#[test]
fn baseline_requires_real_adoption_six_excerpts_exact_text_and_matching_ratio() {
    let (input, _) = current_fixture();
    visual_baseline::validate(&input).unwrap();
    assert_eq!(visual_style(&input), input["settings"]["visualStyle"].as_str().unwrap());
    for pointer in ["/settings/visualBaseline", "/settings/visualBaseline/adoptionBasis", "/settings/visualBaseline/coverage/materials"] {
        let mut bad = input.clone();
        *bad.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(validate_profile(&bad).is_err(), "{pointer}");
    }
    for (pointer, value) in [
        ("/settings/visualBaseline/status", "candidate"),
        ("/settings/visualStyle", "3D国漫"),
        ("/settings/projectVisualStyle", "3D国漫"),
        ("/settings/ratio", "9:16"),
        ("/settings/visualBaseline/coverage/materials", "另写的摘要"),
    ] {
        let mut bad = input.clone();
        *bad.pointer_mut(pointer).unwrap() = json!(value);
        assert!(validate_profile(&bad).is_err(), "{pointer}");
    }
    let (mut legacy, _) = fixture();
    let before = legacy.clone();
    mark_input(&mut legacy).unwrap();
    assert_eq!(legacy, before, "frozen legacy must not upgrade");
    assert_eq!(rule_version(&legacy).unwrap(), rules::LEGACY_VERSION);
    let mut fresh = json!({"settings":{"visualStyle":"3D国漫"}});
    let before = fresh.clone();
    assert!(mark_input(&mut fresh).is_err());
    assert_eq!(fresh, before, "invalid baseline must not create a partial frozen task");
}

#[tokio::test]
async fn baseline_survives_schema_scoped_review_repair_and_unknown_never_retries() {
    let (input, authored) = current_fixture();
    let mut executor = Offline::new(authored.clone());
    executor.review_rounds = 1;
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["status"], "completed", "{}", output["blocked"]);
    let delivery = &output["seriesDeliveries"][0];
    validate_delivery(&input, delivery).unwrap();
    let calls = executor.requests.lock().unwrap();
    let repair = calls.iter().find(|r|r.stage == "seriesRepair").unwrap();
    assert_eq!(binding(repair,"visual-baseline"), &input["settings"]["visualBaseline"]);
    assert!(!repair.schema.to_string().contains("visualBaselineBinding"));
    drop(calls);
    let mut unknown = Offline::new(authored);
    unknown.unknown = true;
    let stopped = run(&input, &unknown, "offline", "medium").await.unwrap();
    assert_eq!(stopped["status"], "partially_blocked");
    assert_eq!(unknown.requests.lock().unwrap().len(), 1);
    assert!(unknown.published.lock().unwrap().is_empty());
}

#[tokio::test]
async fn baseline_flows_through_all_roles_delivery_export_and_resume_without_generation() {
    let (mut input, authored) = current_fixture();
    // Exercise design and design-review in addition to author/finisher/reviewer.
    input["assets"][0].as_object_mut().unwrap().remove("adoptedDesign");
    let executor = Offline::new(authored.clone());
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["status"], "completed", "{}", output["blocked"]);
    let delivery = &output["seriesDeliveries"][0];
    validate_delivery(&input, delivery).unwrap();
    let b = &input["settings"]["visualBaseline"];
    let requests = executor.requests.lock().unwrap();
    for role in ["design", "design-review", "author", "finisher", "reviewer"] {
        let request = requests.iter().find(|r| r.input["role"] == role).unwrap();
        assert_eq!(binding(request, "visual-baseline"), b);
        assert_eq!(request.input["packet"]["bindings"].as_array().unwrap().iter().filter(|v|v["kind"] == "visual-baseline").count(), 1);
    }
    drop(requests);
    for unit in delivery["bundle"]["draft"]["units"].as_array().unwrap() {
        assert!(s(unit,"prompt").contains(s(b,"visualStyle")));
        assert_eq!(unit["unitDirection"], authored["draft"]["units"][0]["unitDirection"]);
        assert_eq!(unit["visualBaselineBinding"]["sha256"], hash(b));
    }
    assert!(s(delivery,"markdown").contains(s(b,"visualStyle")));
    let mut changed = input.clone();
    changed["settings"]["visualBaseline"]["revision"] = json!(2);
    assert!(validate_delivery(&changed, delivery).is_err());
    let directory = tempfile::tempdir().unwrap();
    let export_path = directory.path().join("mixed-baselines");
    assert!(package::write_with_inputs(&changed, &[delivery.clone()], &[input.clone()], &export_path)
        .unwrap_err().contains("DIRECTION_MISMATCH"));
    assert!(!export_path.exists(), "mixed baseline export must fail before writing");
    let mut bad = delivery.clone();
    bad["bundle"]["settings"]["visualBaseline"]["id"] = json!("another-baseline");
    bad["adoptionReceipt"]["bundleSha256"] = json!(hash(&bad["bundle"]));
    assert!(validate_delivery(&input, &bad).is_err());
    let count = executor.requests.lock().unwrap().len();
    let resumed = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output, resumed);
    assert_eq!(count, executor.requests.lock().unwrap().len());
}

#[tokio::test]
async fn baseline_packet_cannot_substitute_another_valid_adoption() {
    let (input, authored) = current_fixture();
    let executor = Offline::new(authored);
    run(&input, &executor, "offline", "medium").await.unwrap();
    let requests = executor.requests.lock().unwrap();
    let mut request = requests.iter().find(|r|r.stage == "seriesAuthor").unwrap().clone();
    let p = &request.input["packet"];
    let values = p["bindings"].as_array().unwrap().iter().map(|binding| {
        let kind = s(binding,"kind");
        let mut value = p["blobs"][s(binding,"blob")]["value"].clone();
        if kind == "visual-baseline" { value["id"] = json!("different-valid-id"); }
        (kind, value)
    }).collect();
    request.input["packet"] = packet("author", s(&request.input,"episodeId"), &input, values).unwrap();
    let source = source(&input).unwrap();
    assert!(validate_role_packet(&request, &input, &source, &source.episodes[0]).is_err());
}
