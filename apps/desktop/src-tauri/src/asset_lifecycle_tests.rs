use super::*;
use crate::store::Store;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store.create("lifecycle offline", "p").unwrap();
        Self { dir, store }
    }
    fn asset(&self, id: &str, design_parent: Option<&str>) -> Value {
        self.store.save_asset(json!({"id":id,"variantId":"v1","kind":"character","name":id,"identityId":"same-person","workflowProfile":asset_graph::PROFILE,
            "designRevision":1,"adoptedDesign":{"version":"series-full-design/v1","assetId":id,"variantId":"v1","designRevision":1,"status":"ADOPTED","full":format!("{id}完整采用设计"),"adoptionBasis":{"kind":"fixture","revision":1},"designDependencies":design_parent.map(|p|vec![json!({"assetId":p,"designRevision":1})]).unwrap_or_default()},
            "referenceRequirements":[],"imagePrompt":{"text":"独立图片提示词","basedOnDesignRevision":1},"promptStatus":"ready"})).unwrap()
    }
    fn candidate(&self, a: &Value, seed: u8, references: Vec<Value>) -> (Value, String) {
        let path = self.dir.path().join(format!("{}-{seed}.png", s(a, "id")));
        image::RgbImage::from_fn(6, 6, |x, y| image::Rgb([seed, x as u8 * 22, y as u8 * 22]))
            .save(&path)
            .unwrap();
        let media = self
            .store
            .import_media(&path, "image", Some(s(a, "id")), Some(s(a, "variantId")))
            .unwrap();
        let job=self.store.add_job("image",s(a,"id"),"offline","offline",&json!({"workflowProfile":asset_graph::PROFILE,"assetId":a["id"],"variantId":a["variantId"],"assetVersion":a["version"],"references":references,"referenceBindings":references}),"downloaded").unwrap();
        self.store.attach_result(&job, s(&media, "id")).unwrap();
        (media, job)
    }
    fn choose(&self, a: &Value, media: &Value) {
        let prior = self.store.adopted(s(a, "id"), s(a, "variantId")).unwrap();
        self.store
            .adopt(s(a, "id"), s(a, "variantId"), s(media, "id"))
            .unwrap();
        // Also exercises the helper in isolation before the controller hook is installed.
        let mut c = self.store.conn().unwrap();
        let tx = c.transaction().unwrap();
        after_image_adoption(
            &tx,
            s(a, "id"),
            s(a, "variantId"),
            s(media, "id"),
            prior.as_deref(),
        )
        .unwrap();
        tx.commit().unwrap();
    }
}

#[test]
fn md_only_identity_setup_does_not_invent_an_adopted_design_review_gate() {
    let f = Fixture::new();
    let original = f
        .store
        .save_asset(json!({"id":"md-only","variantId":"v1",
        "kind":"character","name":"合成换装形象","origin":"imported-storyboard",
        "token":"@[合成换装形象CH]","workflowProfile":asset_graph::PROFILE}))
        .unwrap();
    let mut next = original.clone();
    next["expectedVersion"] = original["version"].clone();
    next["identityId"] = json!("declared-identity");
    next["imageTask"] = json!({"kind":"costume"});
    next["referenceRequirements"] = json!([]);
    f.store.save_asset_user(next).unwrap();
    let current = f.store.active_asset("md-only").unwrap();
    assert!(current["adoptedDesign"].is_null());
    assert!(
        !asset_graph::design_review_blocked(&current),
        "identity setup is source input, not an adopted design edit: {current}"
    );
    assert!(current["designReviewReasons"].is_null());
    assert!(f.store.jobs().unwrap().is_empty());
    let reopened = Store::open(f.store.root.clone()).unwrap();
    assert_eq!(reopened.active_asset("md-only").unwrap(), current);

    // A real adopted design still requires review after identity changes.
    let adopted = f.asset("designed", None);
    let mut edited = adopted.clone();
    edited["identityId"] = json!("another-identity");
    f.store.save_asset(edited).unwrap();
    assert_eq!(
        f.store.active_asset("designed").unwrap()["designStatus"],
        "needs_review"
    );
}

fn mislabel_md_identity(f: &Fixture, id: &str) -> Value {
    let first=f.store.save_asset(json!({"id":id,"variantId":"v1","kind":"character","name":id,
        "origin":"imported-storyboard","workflowProfile":asset_graph::PROFILE,"imagePrompt":{"text":"原提示词保留"},"promptStatus":"ready"})).unwrap();
    let (media, _) = f.candidate(&first, 23, vec![]);
    f.store.adopt(id, "v1", s(&media, "id")).unwrap();
    let mut updated = f.store.active_asset(id).unwrap();
    updated["identityId"] = json!("declared-identity");
    updated["expectedVersion"] = updated["version"].clone();
    f.store.save_asset_user(updated).unwrap();
    let current = f.store.active_asset(id).unwrap();
    let mut c = f.store.conn().unwrap();
    let tx = c.transaction().unwrap();
    let mut old_state = state(&tx, &current).unwrap();
    old_state["designStatus"] = json!("needs_review");
    old_state["designReviewReasons"] =
        json!([{"kind":"adopted-design-edited","assetId":id,"currentRevision":null}]);
    save(&tx, &current, old_state).unwrap();
    tx.commit().unwrap();
    f.store.active_asset(id).unwrap()
}

#[test]
fn md_only_identity_gate_recovery_is_scoped_idempotent_and_preserves_media_words_history_and_unknown(
) {
    let f = Fixture::new();
    let before = mislabel_md_identity(&f, "selected");
    let other = mislabel_md_identity(&f, "unselected");
    let unknown = f
        .store
        .add_job(
            "image",
            "selected",
            "offline",
            "original-account",
            &json!({"requestId":"original-unknown","prompt":"冻结原提示词"}),
            "submission_unknown",
        )
        .unwrap();
    let jobs = f.store.jobs().unwrap();
    let adoption = f.store.adopted("selected", "v1").unwrap();
    let versions = || {
        let connection = f.store.conn().unwrap();
        let mut query = connection
            .prepare("SELECT data FROM asset_versions WHERE asset_id='selected' ORDER BY version")
            .unwrap();
        let values = query
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        values
    };
    let original_versions = versions();
    let request = json!({"assetIds":["selected"]});
    assert_eq!(
        f.store
            .repair_md_only_identity_design_gates(&request)
            .unwrap(),
        vec!["selected"]
    );
    let current = f.store.active_asset("selected").unwrap();
    assert_eq!(current["designStatus"], "needsDesign");
    assert_eq!(current["designReviewReasons"], json!([]));
    assert!(!asset_graph::design_review_blocked(&current));
    for key in [
        "adoptedDesign",
        "imagePrompt",
        "promptStatus",
        "promptReviewReasons",
        "imageStatus",
        "imageReviewReasons",
        "referenceBindings",
        "version",
    ] {
        assert_eq!(current[key], before[key], "{key}");
    }
    assert_eq!(current["imageStatus"], "needs_review");
    assert_eq!(current["promptStatus"], "needs_review");
    assert_eq!(f.store.adopted("selected", "v1").unwrap(), adoption);
    assert_eq!(f.store.active_asset("unselected").unwrap(), other);
    assert_eq!(versions(), original_versions);
    assert_eq!(f.store.jobs().unwrap(), jobs);
    assert_eq!(
        f.store.job(&unknown).unwrap()["status"],
        "submission_unknown"
    );
    assert!(f
        .store
        .repair_md_only_identity_design_gates(&request)
        .unwrap()
        .is_empty());
    let count: i64 = f
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type='md_only_identity_design_gate_repaired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        Store::open(f.store.root.clone())
            .unwrap()
            .active_asset("selected")
            .unwrap(),
        current
    );
}

#[test]
fn md_only_identity_gate_recovery_requires_complete_never_designed_identity_only_history() {
    for scenario in [
        "adopted-design",
        "revision",
        "missing-history",
        "non-identity-change",
        "no-identity-change",
        "prior-other-gate",
    ] {
        let f = Fixture::new();
        let before = mislabel_md_identity(&f, "a");
        let connection = f.store.conn().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT data FROM asset_versions WHERE asset_id='a' AND version=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut initial = parse(raw).unwrap();
        match scenario {
            "adopted-design" => {
                initial["adoptedDesign"] = json!({"full":"历史完整设计，随后已删除"})
            }
            "revision" => initial["designRevision"] = json!(1),
            "non-identity-change" => {
                initial["designDependencies"] = json!([{"assetId":"original-parent"}])
            }
            "no-identity-change" => initial["identityId"] = before["identityId"].clone(),
            "prior-other-gate" => {
                initial["designStatus"] = json!("needs_review");
                initial["designReviewReasons"] = json!([{"kind":"parent-design-changed"}]);
            }
            _ => {}
        }
        connection
            .execute(
                "UPDATE asset_versions SET data=?1 WHERE asset_id='a' AND version=1",
                [initial.to_string()],
            )
            .unwrap();
        if scenario == "missing-history" {
            connection
                .execute(
                    "DELETE FROM asset_versions WHERE asset_id='a' AND version=1",
                    [],
                )
                .unwrap();
        }
        assert!(
            f.store
                .repair_md_only_identity_design_gates(&json!({"assetIds":["a"]}))
                .unwrap()
                .is_empty(),
            "{scenario}"
        );
        assert_eq!(f.store.active_asset("a").unwrap(), before, "{scenario}");
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM events WHERE type='md_only_identity_design_gate_repaired'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
}

#[test]
fn md_only_identity_gate_recovery_never_clears_other_causes_or_non_null_revision() {
    for reasons in [
        json!([{"kind":"adopted-design-edited","assetId":"a","currentRevision":1}]),
        json!([{"kind":"adopted-design-edited","assetId":"a","currentRevision":null},{"kind":"parent-design-changed","assetId":"parent"}]),
        json!([{"kind":"adopted-design-edited","assetId":"different","currentRevision":null}]),
    ] {
        let f = Fixture::new();
        let current = mislabel_md_identity(&f, "a");
        let mut c = f.store.conn().unwrap();
        let tx = c.transaction().unwrap();
        let mut projected = state(&tx, &current).unwrap();
        projected["designReviewReasons"] = reasons;
        save(&tx, &current, projected).unwrap();
        tx.commit().unwrap();
        let before = f.store.active_asset("a").unwrap();
        assert!(f
            .store
            .repair_md_only_identity_design_gates(&json!({"assetIds":["a"]}))
            .unwrap()
            .is_empty());
        assert_eq!(f.store.active_asset("a").unwrap(), before);
    }
}

#[test]
fn md_only_identity_gate_recovery_rejects_invalid_scope_before_any_repair() {
    let f = Fixture::new();
    let before = mislabel_md_identity(&f, "a");
    for selection in [
        json!([]),
        json!(["a", "a"]),
        json!(["a", "absent"]),
        json!(["a", null]),
    ] {
        assert!(f
            .store
            .repair_md_only_identity_design_gates(&json!({"assetIds":selection}))
            .is_err());
        assert_eq!(f.store.active_asset("a").unwrap(), before);
    }
    assert_eq!(
        f.store
            .repair_md_only_identity_design_gates(&json!({}))
            .unwrap(),
        vec!["a"]
    );
}

#[test]
fn adoption_status_manual_selection_preserves_original_dependency_hold() {
    for (status, blocked) in [
        (None, false),
        (Some("adopted"), false),
        (Some("approved"), false),
        (Some("needs_repair"), true),
        (Some("unknown"), true),
        (Some("submission_unknown"), true),
        (Some("candidate"), true),
    ] {
        let f = Fixture::new();
        let mother = f.asset("mother", None);
        let child = f.asset("child", None);
        let (base, _) = f.candidate(&mother, 20, vec![]);
        f.store.adopt("mother", "v1", s(&base, "id")).unwrap();
        let refs = vec![
            json!({"assetId":"mother","variantId":"v1","mediaId":base["id"],"sha256":base["sha256"]}),
        ];
        let (image, job) = f.candidate(&child, 50, refs.clone());
        let original_job = f.store.job(&job).unwrap();
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
        f.store.adopt("child", "v1", s(&image, "id")).unwrap();
        let adopted = f.store.active_asset("child").unwrap();
        assert_eq!(
            adopted["imageStatus"],
            if blocked { "needs_review" } else { "adopted" },
            "{status:?}"
        );
        assert_eq!(adopted["referenceBindings"], json!(refs));
        assert_eq!(
            f.store.adopted("mother", "v1").unwrap().as_deref(),
            base["id"].as_str()
        );
        assert_eq!(
            f.store.adopted("child", "v1").unwrap().as_deref(),
            image["id"].as_str()
        );
        assert_eq!(f.store.active_asset("mother").unwrap(), projection);
        assert_eq!(f.store.job(&job).unwrap(), original_job);
    }
}

#[test]
fn replacing_mother_marks_only_real_adopted_consumers_and_keeps_original_receipts() {
    let f = Fixture::new();
    let mother = f.asset("mother", None);
    let mut child = f.asset("child", None);
    child["referenceRequirements"] = json!([{"targetAssetId":"mother","role":"identity"}]);
    child = f.store.save_asset(child).unwrap();
    let independent = f.asset("independent", None);
    let (base, _) = f.candidate(&mother, 20, vec![]);
    f.choose(&mother, &base);
    let refs = asset_graph::media_references(&f.store, &f.store.values("assets").unwrap(), &child)
        .unwrap()
        .unwrap();
    let (child_image, job) = f.candidate(&child, 40, refs.clone());
    let frozen = f.store.job(&job).unwrap()["snapshot"].clone();
    f.choose(&child, &child_image);
    let (own, _) = f.candidate(&independent, 60, vec![]);
    f.choose(&independent, &own);
    let safe_before = f.store.active_asset("independent").unwrap();
    let design = f.store.active_asset("child").unwrap()["adoptedDesign"].clone();
    assert_eq!(
        f.store.active_asset("child").unwrap()["referenceBindings"],
        json!(refs)
    );
    let (next, _) = f.candidate(&mother, 80, vec![]);
    f.choose(&mother, &next);
    let affected = f.store.active_asset("child").unwrap();
    assert_eq!(affected["imageStatus"], "needs_review");
    assert_eq!(affected["adoptedDesign"], design);
    assert_eq!(affected["referenceBindings"][0]["mediaId"], base["id"]);
    assert_eq!(
        f.store.adopted("child", "v1").unwrap().as_deref(),
        child_image["id"].as_str()
    );
    assert_eq!(f.store.job(&job).unwrap()["snapshot"], frozen);
    assert_eq!(f.store.active_asset("independent").unwrap(), safe_before);
    assert_eq!(affected["designRevision"], 1);
}

#[test]
fn adopting_an_old_candidate_keeps_its_frozen_mother_and_reports_drift() {
    let f = Fixture::new();
    let mother = f.asset("mother", None);
    let mut child = f.asset("child", None);
    child["referenceRequirements"] = json!([{"targetAssetId":"mother","kind":"identity"}]);
    child = f.store.save_asset(child).unwrap();
    let (base, _) = f.candidate(&mother, 20, vec![]);
    f.choose(&mother, &base);
    let refs = asset_graph::media_references(&f.store, &f.store.values("assets").unwrap(), &child)
        .unwrap()
        .unwrap();
    let (old, _) = f.candidate(&child, 40, refs);
    let (replacement, _) = f.candidate(&mother, 80, vec![]);
    f.choose(&mother, &replacement);
    f.choose(&child, &old);
    let selected = f.store.active_asset("child").unwrap();
    assert_eq!(selected["imageStatus"], "needs_review");
    assert_eq!(selected["referenceBindings"][0]["mediaId"], base["id"]);
    assert_ne!(
        selected["referenceBindings"][0]["mediaId"],
        replacement["id"]
    );
}

#[test]
fn parent_design_change_uses_design_dependencies_not_family_or_media_names() {
    let f = Fixture::new();
    let mother = f.asset("mother", None);
    let child = f.asset("child", Some("mother"));
    let independent = f.asset("same-family", None);
    let safe_before = f.store.active_asset("same-family").unwrap();
    let full = child["adoptedDesign"].clone();
    let old = f.store.active_asset("mother").unwrap();
    let mut next = old.clone();
    next["adoptedDesign"]["full"] = json!("已独审采用的新完整设计");
    next["adoptedDesign"]["designRevision"] = json!(2);
    next["designRevision"] = json!(2);
    next["adoptedDesign"]["adoptionBasis"]["revision"] = json!(2);
    let next = f.store.save_asset(next).unwrap();
    let mut c = f.store.conn().unwrap();
    let tx = c.transaction().unwrap();
    after_asset_save(&tx, Some(&old), &next).unwrap();
    tx.commit().unwrap();
    let affected = f.store.active_asset("child").unwrap();
    assert_eq!(affected["designStatus"], "needs_review");
    assert_eq!(affected["promptStatus"], "needs_review");
    assert_eq!(affected["adoptedDesign"], full);
    assert_eq!(affected["designRevision"], 1);
    assert_eq!(
        f.store.active_asset(s(&independent, "id")).unwrap(),
        safe_before
    );
    assert_eq!(
        f.store.active_asset(s(&mother, "id")).unwrap()["designRevision"],
        2
    );
}

#[test]
fn cosmetic_asset_edit_cannot_erase_dependency_review_or_rewrite_image_bindings() {
    let f = Fixture::new();
    let a = f.asset("a", None);
    let mut c = f.store.conn().unwrap();
    let tx = c.transaction().unwrap();
    let original = json!({"version":"asset-lifecycle/v1","imageStatus":"needs_review","referenceBindings":[{"assetId":"mother","mediaId":"old"}],"imageReviewReasons":[{"kind":"reference-media-changed"}]});
    save(&tx, &a, original.clone()).unwrap();
    tx.commit().unwrap();
    let before = f.store.active_asset("a").unwrap();
    let mut edit = before.clone();
    edit["name"] = json!("新显示名");
    edit["imageStatus"] = json!("adopted");
    edit["referenceBindings"] = json!([]);
    let edited = f.store.save_asset(edit).unwrap();
    let mut c = f.store.conn().unwrap();
    let tx = c.transaction().unwrap();
    after_asset_save(&tx, Some(&before), &edited).unwrap();
    tx.commit().unwrap();
    let saved = f.store.active_asset("a").unwrap();
    assert_eq!(saved["imageStatus"], "needs_review");
    assert_eq!(saved["referenceBindings"], original["referenceBindings"]);
    assert_eq!(saved["name"], "新显示名");
}

#[test]
fn mother_replacement_marks_all_eleven_real_media_levels_without_touching_planned_or_family_branches(
) {
    let f = Fixture::new();
    let mother = f.asset("level-0", None);
    let (base, _) = f.candidate(&mother, 10, vec![]);
    f.choose(&mother, &base);
    let mut frozen = Vec::new();
    for level in 1..11 {
        let mut child = f.asset(&format!("level-{level}"), None);
        child["referenceRequirements"] =
            json!([{"targetAssetId":format!("level-{}",level-1),"role":"appearance"}]);
        child = f.store.save_asset(child).unwrap();
        let refs =
            asset_graph::media_references(&f.store, &f.store.values("assets").unwrap(), &child)
                .unwrap()
                .unwrap();
        let (media, job) = f.candidate(&child, 10 + level, refs);
        f.choose(&child, &media);
        frozen.push((
            child,
            media,
            job.clone(),
            f.store.job(&job).unwrap()["snapshot"].clone(),
        ));
    }
    let mut planned = f.asset("planned-only", None);
    planned["referenceRequirements"] = json!([{"targetAssetId":"level-0","role":"appearance"}]);
    f.store.save_asset(planned).unwrap();
    let planned_before = f.store.active_asset("planned-only").unwrap();
    let independent = f.asset("same-identity-independent", None);
    let (image, _) = f.candidate(&independent, 60, vec![]);
    f.choose(&independent, &image);
    let independent_before = f.store.active_asset("same-identity-independent").unwrap();

    let (replacement, _) = f.candidate(&mother, 90, vec![]);
    f.choose(&mother, &replacement);
    for (asset, media, job, snapshot) in frozen {
        let after = f.store.active_asset(s(&asset, "id")).unwrap();
        assert_eq!(
            after["imageStatus"], "needs_review",
            "all true media descendants need review"
        );
        assert_eq!(
            after["promptStatus"], "ready",
            "media changes do not invalidate text designs"
        );
        assert_eq!(after["adoptedDesign"], asset["adoptedDesign"]);
        assert_eq!(after["designRevision"], 1);
        assert_eq!(
            f.store.adopted(s(&asset, "id"), "v1").unwrap().as_deref(),
            media["id"].as_str()
        );
        assert_eq!(f.store.job(&job).unwrap()["snapshot"], snapshot);
    }
    assert_eq!(
        f.store.active_asset("planned-only").unwrap(),
        planned_before
    );
    assert_eq!(
        f.store.active_asset("same-identity-independent").unwrap(),
        independent_before
    );
}

#[test]
fn design_review_propagates_through_real_design_chain_and_its_adopted_media_consumers() {
    let f = Fixture::new();
    let mother = f.asset("mother", None);
    let child = f.asset("design-child", Some("mother"));
    let grandchild = f.asset("design-grandchild", Some("design-child"));
    let independent = f.asset("same-family-only", None);
    let before_independent = f.store.active_asset(s(&independent, "id")).unwrap();
    let (base, _) = f.candidate(&mother, 10, vec![]);
    f.choose(&mother, &base);
    let mut media_only = f.asset("media-only-child", None);
    media_only["referenceRequirements"] = json!([{"targetAssetId":"mother","role":"identity"}]);
    media_only = f.store.save_asset(media_only).unwrap();
    let refs =
        asset_graph::media_references(&f.store, &f.store.values("assets").unwrap(), &media_only)
            .unwrap()
            .unwrap();
    let (image, job) = f.candidate(&media_only, 50, refs);
    f.choose(&media_only, &image);
    let frozen = f.store.job(&job).unwrap()["snapshot"].clone();

    let mut updated = f.store.active_asset("mother").unwrap();
    updated["adoptedDesign"]["full"] = json!("已核验采用的新设计");
    updated["adoptedDesign"]["designRevision"] = json!(2);
    updated["designRevision"] = json!(2);
    updated["adoptedDesign"]["adoptionBasis"]["revision"] = json!(2);
    f.store.save_asset(updated).unwrap();
    for original in [&child, &grandchild] {
        let affected = f.store.active_asset(s(original, "id")).unwrap();
        assert_eq!(affected["designStatus"], "needs_review");
        assert_eq!(affected["promptStatus"], "needs_review");
        assert_eq!(affected["adoptedDesign"], original["adoptedDesign"]);
        assert_eq!(affected["designRevision"], 1);
    }
    let affected = f.store.active_asset("media-only-child").unwrap();
    assert_eq!(affected["imageStatus"], "needs_review");
    assert_eq!(
        affected["promptStatus"], "ready",
        "a media-only consumer is not a text-design dependency"
    );
    assert_eq!(affected["adoptedDesign"], media_only["adoptedDesign"]);
    assert_eq!(f.store.job(&job).unwrap()["snapshot"], frozen);
    assert_eq!(
        f.store.active_asset("same-family-only").unwrap(),
        before_independent
    );
}

#[test]
fn public_same_image_recheck_restores_three_level_media_chain_without_rewriting_original_jobs() {
    let f = Fixture::new();
    let mother = f.asset("mother", None);
    let (base, _) = f.candidate(&mother, 10, vec![]);
    f.store.adopt("mother", "v1", s(&base, "id")).unwrap();
    let mut chain = Vec::new();
    for (id, parent, seed) in [("child", "mother", 30), ("grandchild", "child", 50)] {
        let mut a = f.asset(id, None);
        a["referenceRequirements"] =
            json!([{"targetAssetId":parent,"targetVariantId":"v1","role":"appearance"}]);
        let a = f.store.save_asset(a).unwrap();
        let refs = asset_graph::media_references(&f.store, &f.store.values("assets").unwrap(), &a)
            .unwrap()
            .unwrap();
        let (media, job) = f.candidate(&a, seed, refs);
        f.store.adopt(id, "v1", s(&media, "id")).unwrap();
        chain.push((a, media, job));
    }
    let unknown = f.store.add_job("image", "child", "offline", "original-account",
        &json!({"requestId":"unresolved-original","assetId":"child","variantId":"v1","prompt":"原冻结请求"}), "submission_unknown").unwrap();
    f.store
        .update_job(
            &unknown,
            "submission_unknown",
            &json!({"error":"原提交未知","receipt":{"raw":"retain-original"}}),
        )
        .unwrap();
    let original_jobs = f.store.jobs().unwrap();
    let adoption = || {
        f.store.conn().unwrap().query_row(
        "SELECT media_id,created_at FROM adoptions WHERE asset_id='child' AND variant_id='v1'", [],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap()
    };
    let original_adoption = adoption();
    let (replacement, _) = f.candidate(&mother, 70, vec![]);
    f.store
        .adopt("mother", "v1", s(&replacement, "id"))
        .unwrap();
    for id in ["child", "grandchild"] {
        assert_eq!(
            f.store.active_asset(id).unwrap()["imageStatus"],
            "needs_review"
        );
    }
    // A recheck while the actual mother still differs must keep the old binding.
    f.store.adopt("child", "v1", s(&chain[0].1, "id")).unwrap();
    assert_eq!(
        f.store.active_asset("child").unwrap()["imageStatus"],
        "needs_review"
    );
    assert_eq!(
        f.store.active_asset("child").unwrap()["referenceBindings"][0]["mediaId"],
        base["id"]
    );
    f.store.adopt("mother", "v1", s(&base, "id")).unwrap();
    let pending = f.store.active_asset("child").unwrap();
    // Automatic backfill must never turn recovery of a matching reference into
    // an implicit approval. Only an explicit selection can perform the recheck.
    let mut c = f.store.conn().unwrap();
    let tx = c.transaction().unwrap();
    assert!(
        !crate::store::set_current_image(&tx, "child", "v1", s(&chain[0].1, "id"), None, true)
            .unwrap()
    );
    tx.commit().unwrap();
    assert_eq!(f.store.active_asset("child").unwrap(), pending);
    for (a, media, _) in &chain {
        f.store.adopt(s(a, "id"), "v1", s(media, "id")).unwrap();
        let reviewed = f.store.active_asset(s(a, "id")).unwrap();
        assert_eq!(reviewed["imageStatus"], "adopted");
        assert_eq!(reviewed["imageReviewReasons"], json!([]));
        assert_eq!(reviewed["adoptedDesign"], a["adoptedDesign"]);
        assert_eq!(
            f.store.adopted(s(a, "id"), "v1").unwrap().as_deref(),
            media["id"].as_str()
        );
        f.store.adopt(s(a, "id"), "v1", s(media, "id")).unwrap();
        assert_eq!(
            f.store.active_asset(s(a, "id")).unwrap(),
            reviewed,
            "healthy duplicate adoption remains idempotent"
        );
    }
    assert_eq!(
        adoption(),
        original_adoption,
        "same-media review retains the actual adoption record"
    );
    for original in original_jobs {
        assert_eq!(
            f.store.job(s(&original, "id")).unwrap(),
            original,
            "raw, snapshot, receipt and UNKNOWN are immutable during local review"
        );
    }
    assert_eq!(
        f.store.job(&unknown).unwrap()["status"],
        "submission_unknown"
    );
    assert_eq!(
        f.store.jobs().unwrap().len(),
        5,
        "local review creates no generation work"
    );
}

#[test]
fn same_image_recheck_never_clears_rejection_unknown_or_non_media_review_causes() {
    let f = Fixture::new();
    let a = f.asset("a", None);
    let (media, job) = f.candidate(&a, 25, vec![]);
    f.store.adopt("a", "v1", s(&media, "id")).unwrap();
    let original_job = f.store.job(&job).unwrap();
    for (status, reason) in [
        ("rejected", "quality-rejected"),
        ("needs_repair", "quality-rejected"),
        ("invalid", "invalid-image"),
        ("submission_unknown", "submission-unknown"),
        ("unknown", "submission-unknown"),
        ("needs_review", "own-design-changed"),
        ("needs_review", "unverified-identity"),
        ("stale", "unknown-cause"),
    ] {
        let current = f.store.active_asset("a").unwrap();
        let mut c = f.store.conn().unwrap();
        let tx = c.transaction().unwrap();
        let mut next = state(&tx, &current).unwrap();
        next["imageStatus"] = json!(status);
        next["imageReviewReasons"] = json!([{"kind":reason,"assetId":"a"}]);
        save(&tx, &current, next.clone()).unwrap();
        tx.commit().unwrap();
        let before = f.store.active_asset("a").unwrap();
        f.store.adopt("a", "v1", s(&media, "id")).unwrap();
        assert_eq!(
            f.store.active_asset("a").unwrap(),
            before,
            "{status}/{reason}"
        );
        let mut c = f.store.conn().unwrap();
        let tx = c.transaction().unwrap();
        assert_eq!(
            state(&tx, &before).unwrap(),
            next,
            "derived rejection/unknown facts remain intact"
        );
        tx.rollback().unwrap();
        assert_eq!(f.store.job(&job).unwrap(), original_job);
    }
}
