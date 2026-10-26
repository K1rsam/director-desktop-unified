use super::*;
#[path = "visual_review_lifecycle_tests.rs"]
mod lifecycle;

fn passed(count: usize) -> Value {
    json!({"decision":"PASS","visualRead":true,"observations":(1..=count).map(|i|json!({"index":i,"visibleDetails":"候选灰色长袍衣领和黑色腰带清晰可见且各视角一致"})).collect::<Vec<_>>(),
        "checks":(["source_fidelity","identity_consistency","spatial_consistency","view_layout","costume_and_state","image_integrity"].map(|id|json!({"id":id,"result":"pass","evidence":"逐图观察的具体可见部位符合本次完整设计的要求"}))),
        "scriptScore":97,"identityScore":96,"spaceScore":96,"overallScore":96,"issues":[],"anchorSummary":"图片中的灰色立领、黑色腰带以及长袖的轮廓一致"})
}
#[test]
fn high_score_cannot_replace_observations_or_each_hard_check() {
    let valid = passed(2);
    assert!(review_passes(&valid, 2));
    for field in ["visualRead", "observations", "checks", "anchorSummary"] {
        let mut v = valid.clone();
        v.as_object_mut().unwrap().remove(field);
        assert!(!review_passes(&v, 2), "{field}");
    }
    for i in 0..6 {
        let mut v = valid.clone();
        v["checks"][i]["result"] = json!("uncertain");
        assert!(!review_passes(&v, 2));
    }
    let mut duplicate = valid.clone();
    duplicate["observations"][1]["index"] = json!(1);
    assert!(!review_passes(&duplicate, 2));
    let mut issue = valid.clone();
    issue["issues"] = json!(["七视图缺少侧面"]);
    assert!(!review_passes(&issue, 2));
    let mut weak = valid;
    weak["spaceScore"] = json!(94);
    assert!(!review_passes(&weak, 2));
}
#[test]
fn channel_selection_never_silently_changes_to_vision_sdk() {
    assert!(selected(
        &json!({"textProvider":"codex_sdk","textModel":"gpt-6.1-sol","reasoning":"medium"})
    )
    .is_ok());
    for v in [
        json!({"textProvider":"codex_cli","textModel":"gpt-6.1-sol","reasoning":"medium"}),
        json!({"textProvider":"codex_sdk","textModel":"other","reasoning":"medium"}),
        json!({"textProvider":"codex_sdk","textModel":"gpt-6.1-sol","reasoning":"high"}),
    ] {
        assert!(selected(&v).is_err());
    }
}
#[test]
fn candidate_key_does_not_spend_again_after_user_selected_same_media() {
    let a = json!({"assetId":"a","mediaId":"m","previousAdoptedMediaId":null,"assetVersion":2});
    let mut b = a.clone();
    b["previousAdoptedMediaId"] = json!("m");
    assert_eq!(review_key_basis(&a), review_key_basis(&b));
    b["assetVersion"] = json!(3);
    assert_ne!(review_key_basis(&a), review_key_basis(&b));
}
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store.create("visual review offline", "p").unwrap();
        Self { dir, store }
    }
    fn asset(&self, id: &str) -> Value {
        self.store.save_asset(json!({"id":id,"variantId":"v1","kind":"character","name":id,"workflowProfile":crate::asset_graph::PROFILE,"filename":format!("{id}.png"),"adoptedDesign":{"full":"原文中的灰色长袍和黑腰带","status":"ADOPTED","designRevision":1},"designRevision":1,"referenceRequirements":[],"imagePrompt":{"text":"灰色长袍，黑腰带，七视图","status":"ready","basedOnDesignRevision":1},"promptStatus":"ready"})).unwrap()
    }
    fn candidate(&self, a: &Value, policy: &str) -> Value {
        let file = self.dir.path().join(format!("{}.png", s(a, "id")));
        image::RgbImage::new(32, 32).save(&file).unwrap();
        let media = self
            .store
            .import_media(&file, "image", Some(s(a, "id")), Some(s(a, "variantId")))
            .unwrap();
        let job=self.store.add_job("image",s(a,"id"),"offline","offline",&json!({"workflowProfile":crate::asset_graph::PROFILE,"assetId":a["id"],"variantId":a["variantId"],"assetVersion":a["version"],"promptBody":crate::pipeline::asset_workflow::prompt_text(a),"references":[],"imageAdoptionPolicy":policy}),"downloaded").unwrap();
        self.store.attach_result(&job, s(&media, "id")).unwrap();
        media
    }
}
#[test]
fn adoption_status_visual_review_refuses_unusable_original_mother_without_model() {
    let f = Fixture::new();
    let mother = f.asset("mother");
    let base = f.candidate(&mother, "manual");
    f.store.adopt("mother", "v1", s(&base, "id")).unwrap();
    let child = f.asset("child");
    let image = f.candidate(&child, "manual");
    // Freeze a synthetic original request with its exact adopted mother.
    let job = f
        .store
        .jobs()
        .unwrap()
        .into_iter()
        .find(|j| j["entityId"] == "child")
        .unwrap();
    let mut snapshot = job["snapshot"].clone();
    snapshot["references"] = json!([{"assetId":"mother","variantId":"v1","mediaId":base["id"],"sha256":base["sha256"],"usage":"reference"}]);
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
            rusqlite::params![snapshot.to_string(), s(&job, "id")],
        )
        .unwrap();
    let original_jobs = f.store.jobs().unwrap();
    for (status, blocked) in [
        (None, false),
        (Some("adopted"), false),
        (Some("approved"), false),
        (Some("needs_repair"), true),
        (Some("unknown"), true),
        (Some("submission_unknown"), true),
        (Some("candidate"), true),
    ] {
        let mut projection = f.store.active_asset("mother").unwrap();
        if let Some(status) = status {
            projection["imageStatus"] = json!(status);
        } else {
            projection.as_object_mut().unwrap().remove("imageStatus");
        }
        f.store
            .conn()
            .unwrap()
            .execute(
                "UPDATE assets SET data=?1 WHERE id='mother'",
                [projection.to_string()],
            )
            .unwrap();
        let basis = candidate_basis(&f.store, "child", s(&image, "id"));
        if blocked {
            assert!(
                basis.unwrap_err().contains("VISUAL_REFERENCE_CHANGED"),
                "{status:?}"
            );
        } else {
            assert_eq!(basis.unwrap()["references"][0]["mediaId"], base["id"]);
        }
        assert_eq!(
            f.store.adopted("mother", "v1").unwrap().as_deref(),
            base["id"].as_str()
        );
        assert!(f.store.adopted("child", "v1").unwrap().is_none());
        assert_eq!(f.store.active_asset("mother").unwrap(), projection);
    }
    assert_eq!(f.store.jobs().unwrap(), original_jobs);
}

#[test]
fn ordinary_downloaded_candidate_does_not_self_delegate() {
    let f = Fixture::new();
    let a = f.asset("a");
    let m = f.candidate(&a, "manual");
    assert!(f.store.due_visual_reviews().unwrap().is_empty());
    assert_eq!(
        candidate_basis(&f.store, "a", s(&m, "id")).unwrap()["assetId"],
        "a"
    );
    assert!(f
        .store
        .set_visual_review_delegation(
            true,
            &json!({"textProvider":"codex_sdk","textModel":"gpt-6.1-sol","reasoning":"medium"})
        )
        .is_err());
    assert!(f
        .store
        .set_visual_review_delegation(false, &json!({}))
        .is_ok());
}
#[test]
fn review_persistence_never_changes_adoption_and_unknown_is_not_due_again() {
    let f = Fixture::new();
    let a = f.asset("a");
    let m = f.candidate(&a, "delegated-review");
    let before = f.store.adopted("a", "v1").unwrap();
    // Synthetic test-only metadata proves the queue's once-only guard. It is
    // not a capability receipt and cannot pass the public opt-in/adoption gate.
    f.store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO metadata(key,value) VALUES('visual-review.delegation',?1)",
            [json!({"enabled":true,"projectRoot":f.store.root,"projectId":"p"}).to_string()],
        )
        .unwrap();
    assert_eq!(f.store.due_visual_reviews().unwrap().len(), 1);
    f.store
        .record_visual_review(s(&m, "id"), &json!({"status":"unknown","adopted":false}))
        .unwrap();
    assert!(f.store.due_visual_reviews().unwrap().is_empty());
    assert_eq!(f.store.adopted("a", "v1").unwrap(), before);
    assert_eq!(
        f.store.get("media", s(&m, "id")).unwrap()["visualReview"]["status"],
        "unknown"
    );
    assert!(f
        .store
        .adopt_visual_review(&json!({"status":"passed","adopted":true}))
        .is_err());
}
#[test]
fn changed_design_and_candidate_bytes_invalidate_review_basis() {
    let f = Fixture::new();
    let a = f.asset("a");
    let m = f.candidate(&a, "manual");
    assert!(candidate_basis(&f.store, "a", s(&m, "id")).is_ok());
    let mut changed = a;
    changed["adoptedDesign"]["full"] = json!("新设计，不沿用旧图");
    f.store.save_asset(changed).unwrap();
    assert!(candidate_basis(&f.store, "a", s(&m, "id")).is_err());
}
#[test]
fn frozen_image_is_exact_input_and_invalid_data_is_rejected() {
    let f = Fixture::new();
    let path = allocate(&f.store, "offline-images").unwrap();
    let input = f.dir.path().join("input.png");
    image::RgbImage::new(32, 16).save(&input).unwrap();
    let frozen = freeze_image(&path, 1, "candidate", &json!({"id":"m"}), &input).unwrap();
    assert_eq!(frozen["width"], 32);
    assert_eq!(frozen["height"], 16);
    assert_eq!(frozen["sha256"], hash_file(&input).unwrap().0);
    assert_eq!(
        fs::read(s(&frozen, "path")).unwrap(),
        fs::read(&input).unwrap()
    );
    fs::write(&input, b"not an image").unwrap();
    assert!(freeze_image(&path, 2, "reference", &json!({}), &input).is_err());
}
#[test]
fn offline_execution_policy_never_permits_paid_visual_inference() {
    for action in ["images.reviewCapability", "images.reviewCandidate"] {
        assert!(crate::execution_policy::check(&json!({"action":action})).is_err());
        assert!(crate::execution_policy::check_standard(&json!({"action":action})).is_ok());
        assert!(crate::execution_policy::check_native_action(&json!({"action":action})).is_err());
    }
}

#[test]
fn md_only_review_material_contains_exact_bound_storyboard_text() {
    let f = Fixture::new();
    let md="第1集\n1-1\n场景：门厅\n人物：甲\n片段1\n时长：4秒\n[人物] @[甲CH]\n分镜1｜0-3s｜3s\n画面：甲穿灰衣站定。\n片段2\n时长：4秒\n[场景] @[门厅SC]\n分镜1｜0-3s｜3s\n画面：门在左，窗在右。\n";
    crate::lingmo_import::import(&f.store, &json!({"text":md,"expectedSha256":digest(md)}))
        .unwrap();
    let asset = f
        .store
        .values("assets")
        .unwrap()
        .into_iter()
        .find(|a| a["kind"] == "scene")
        .unwrap();
    let input = f
        .store
        .image_workflow_input(&json!({"assetIds":[asset["id"]]}), &json!({}))
        .unwrap();
    let material = crate::pipeline::asset_workflow::visual_review_material(&input, &asset).unwrap();
    assert_eq!(material["sourceKind"], "imported-md");
    let text = material["evidence"].to_string();
    assert!(text.contains("门在左，窗在右"));
    assert!(!text.contains("甲穿灰衣站定"));
    assert!(material.get("lockedSource").is_none());
    assert!(!material["sourceUnitBindings"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn imported_unknown_keeps_original_summary_and_cannot_start_new_review() {
    let f = Fixture::new();
    let a = f.asset("a");
    let m = f.candidate(&a, "manual");
    let original = json!({"status":"unknown","output":{"partial":"保留原审核回包"},"attemptDirectory":"C:/previous-machine/review"});
    f.store
        .record_visual_review(s(&m, "id"), &original)
        .unwrap();
    assert!(ensure_original_available(&f.store, s(&m, "id"))
        .unwrap_err()
        .contains("VISUAL_ORIGINAL_EVIDENCE_MISSING"));
    preserve_missing_evidence(&f.store, s(&m, "id"), "原机器证据未随包").unwrap();
    let projection = f.store.get("media", s(&m, "id")).unwrap()["visualReview"].clone();
    assert_eq!(projection["originalReview"]["output"], original["output"]);
    assert_eq!(projection["evidenceUnavailable"], true);
    assert!(ensure_original_available(&f.store, s(&m, "id")).is_err());
    f.store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO metadata(key,value) VALUES('visual-review.delegation',?1)",
            [
                json!({"enabled":true,"projectId":"p","projectRoot":"C:/previous-machine/project"})
                    .to_string(),
            ],
        )
        .unwrap();
    assert_eq!(
        f.store.visual_review_delegation().unwrap()["enabled"],
        false
    );
}

#[test]
fn placeholder_is_not_a_real_candidate_even_through_backend() {
    let f = Fixture::new();
    let a = f.asset("a");
    let m = f.candidate(&a, "manual");
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE media SET data=json_set(data,'$.placeholder',json('true')) WHERE id=?1",
            [s(&m, "id")],
        )
        .unwrap();
    assert!(candidate_basis(&f.store, "a", s(&m, "id"))
        .unwrap_err()
        .contains("VISUAL_CANDIDATE_NOT_IMAGE"));
}
