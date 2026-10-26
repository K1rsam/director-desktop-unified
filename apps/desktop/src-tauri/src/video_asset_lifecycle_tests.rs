//! Offline preparation/send-time validation only. PNGs are local synthetic
//! images; no video channel, network call or submission is used by this fixture.
use super::*;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store.create("offline video lifecycle", "p").unwrap();
        let source = crate::pipeline::prepare_source("第一集\n场景：门口\n△人物走进门口。\n").unwrap();
        store.save_source(json!(source)).unwrap();
        Self { dir, store }
    }

    fn asset(&self, id: &str, design_parent: Option<&str>) -> Value {
        self.store.save_asset(json!({"id":id,"variantId":"v1","kind":"character","name":id,
            "identityId":"same-family","workflowProfile":crate::asset_graph::PROFILE,
            "preparation":{"mode":"REFERENCE"},"designRevision":1,
            "adoptedDesign":{"version":"series-full-design/v1","assetId":id,"variantId":"v1",
                "designRevision":1,"status":"ADOPTED","full":format!("{id}完整设计"),
                "adoptionBasis":{"kind":"fixture","revision":1},
                "designDependencies":design_parent.map(|p|vec![json!({"assetId":p,"designRevision":1})]).unwrap_or_default()},
            "referenceRequirements":[],"imagePrompt":{"text":"离线图片提示词","basedOnDesignRevision":1},
            "promptStatus":"ready"})).unwrap()
    }

    fn candidate(&self, asset: &Value, tone: u8, references: Vec<Value>) -> Value {
        let path = self.dir.path().join(format!("{}-{tone}.png", s(asset, "id")));
        image::RgbImage::from_fn(6, 6, |x, y| image::Rgb([tone, x as u8 * 22, y as u8 * 22]))
            .save(&path).unwrap();
        let media = self.store.import_media(&path, "image", Some(s(asset, "id")), Some("v1")).unwrap();
        let job = self.store.add_job("image", s(asset, "id"), "offline", "offline",
            &json!({"workflowProfile":crate::asset_graph::PROFILE,"assetId":asset["id"],"variantId":"v1",
                "assetVersion":asset["version"],"references":references,"referenceBindings":references}),
            "downloaded").unwrap();
        self.store.attach_result(&job, s(&media, "id")).unwrap();
        media
    }

    fn adopt(&self, asset: &Value, media: &Value) {
        self.store.adopt(s(asset, "id"), "v1", s(media, "id")).unwrap();
    }

    fn unit(&self, id: &str, asset_id: &str) -> Value {
        let source = self.store.source().unwrap();
        let row = source["rows"].as_array().unwrap().iter().find(|r| r["kind"] == "body").unwrap();
        self.store.save_unit(json!({"id":id,"episodeId":row["episodeId"],"sceneId":row["sceneId"],
            "sourceVersion":source["version"],"sourceRefs":[row["id"]],"duration":4,
            "prompt":format!("人物走进门口。{{{{asset:{asset_id}:v1}}}}"),
            "assetRefs":[{"assetId":asset_id,"variantId":"v1","order":0}]})).unwrap()
    }

    fn ready_snapshot(&self, unit: &Value) -> Value {
        let snapshot = bind(&self.store, unit, "bh", "offline").unwrap();
        assert_eq!(snapshot["problems"], json!([]), "{snapshot}");
        assert_eq!(snapshot["assetLifecycleContract"], VIDEO_ASSET_LIFECYCLE_CONTRACT);
        validate_snapshot(&self.store, &snapshot).unwrap();
        snapshot
    }

    fn set_review_projection(&self, id: &str, field: &str, status: &str) {
        let mut asset = self.store.active_asset(id).unwrap();
        asset[field] = json!(status);
        self.store.conn().unwrap().execute("UPDATE assets SET data=?1 WHERE id=?2",
            rusqlite::params![asset.to_string(),id]).unwrap();
    }
}

#[test]
fn adoption_status_gates_new_video_preparation_without_retrofitting_frozen_history() {
    let f = Fixture::new();
    let asset = f.asset("person", None);
    let media = f.candidate(&asset, 30, vec![]);
    f.adopt(&asset, &media);
    let unit = f.unit("consumer", "person");
    let snapshot = f.ready_snapshot(&unit);
    let mut legacy = snapshot.clone();
    legacy.as_object_mut().unwrap().remove("assetLifecycleContract");
    let before_jobs = f.store.jobs().unwrap();
    let bytes = std::fs::read(f.store.verify_media(&media).unwrap()).unwrap();
    for (status, blocked) in [
        (None, false), (Some(""), false), (Some("adopted"), false), (Some("approved"), false),
        (Some("needs_review"), true), (Some("stale"), true), (Some("rejected"), true),
        (Some("invalid"), true), (Some("needs_repair"), true), (Some("unknown"), true),
        (Some("submission_unknown"), true), (Some("candidate"), true), (Some("pending_review"), true),
    ] {
        let mut projection = f.store.active_asset("person").unwrap();
        if let Some(status) = status { projection["imageStatus"] = json!(status); }
        else { projection.as_object_mut().unwrap().remove("imageStatus"); }
        f.store.conn().unwrap().execute("UPDATE assets SET data=?1 WHERE id='person'", [projection.to_string()]).unwrap();
        let prepared = bind(&f.store, &unit, "bh", "offline").unwrap();
        assert_eq!(!prepared["problems"].as_array().unwrap().is_empty(), blocked, "{status:?}");
        assert_eq!(validate_snapshot(&f.store, &snapshot).is_err(), blocked, "{status:?}");
        validate_snapshot(&f.store, &legacy).unwrap();
        assert_eq!(f.store.active_asset("person").unwrap(), projection);
        assert_eq!(f.store.get("units", s(&unit, "id")).unwrap(), unit);
        assert_eq!(f.store.adopted("person", "v1").unwrap().as_deref(), media["id"].as_str());
    }
    assert_eq!(f.store.jobs().unwrap(), before_jobs);
    assert_eq!(std::fs::read(f.store.verify_media(&media).unwrap()).unwrap(), bytes);
}

#[test]
fn adopted_parent_change_blocks_only_real_video_consumers_and_explicit_recheck_restores() {
    let f = Fixture::new();
    let mother = f.asset("mother", None);
    let base = f.candidate(&mother, 10, vec![]);
    f.adopt(&mother, &base);
    let mut child = f.asset("child", None);
    child["referenceRequirements"] = json!([{"targetAssetId":"mother","targetVariantId":"v1","role":"identity"}]);
    child = f.store.save_asset(child).unwrap();
    let references = crate::asset_graph::media_references(&f.store, &f.store.values("assets").unwrap(), &child).unwrap().unwrap();
    let image = f.candidate(&child, 30, references);
    f.adopt(&child, &image);
    let independent = f.asset("same-family-independent", None);
    let independent_image = f.candidate(&independent, 50, vec![]);
    f.adopt(&independent, &independent_image);
    let unit = f.unit("consumer", "child");
    let independent_unit = f.unit("unrelated", "same-family-independent");
    let snapshot = f.ready_snapshot(&unit);
    let independent_snapshot = f.ready_snapshot(&independent_unit);
    let mut legacy = snapshot.clone();
    legacy.as_object_mut().unwrap().remove("assetLifecycleContract");
    let unknown = f.store.add_job("video", "consumer", "offline", "original-account", &legacy, "submission_unknown").unwrap();
    f.store.update_job(&unknown, "submission_unknown", &json!({"receipt":{"raw":"retained-unknown-receipt"},"error":"original unknown"})).unwrap();
    let before_child = f.store.active_asset("child").unwrap();
    let before_independent = f.store.active_asset("same-family-independent").unwrap();
    let next = f.candidate(&mother, 70, vec![]);
    let original_jobs = f.store.jobs().unwrap();
    f.adopt(&mother, &next);
    let stale = f.store.active_asset("child").unwrap();
    assert_eq!(stale["imageStatus"], "needs_review");
    assert_eq!(stale["version"], before_child["version"]);
    assert_eq!(f.store.adopted("child", "v1").unwrap().as_deref(), image["id"].as_str());
    assert_eq!(f.store.get("media", s(&image, "id")).unwrap()["sha256"], image["sha256"]);
    assert!(validate_snapshot(&f.store, &snapshot).unwrap_err().contains("图片待复核"));
    let blocked = bind(&f.store, &unit, "bh", "offline").unwrap();
    assert!(blocked["problems"].as_array().unwrap().iter().any(|p| p["assetId"] == "child" && s(p, "message").contains("待复核")));
    validate_snapshot(&f.store, &independent_snapshot).unwrap();
    f.ready_snapshot(&independent_unit);
    assert_eq!(f.store.active_asset("same-family-independent").unwrap(), before_independent);
    // No retrofit of a new rule into an already frozen historic request.
    validate_snapshot(&f.store, &legacy).unwrap();
    f.adopt(&child, &image);
    assert!(validate_snapshot(&f.store, &snapshot).is_err(), "different current mother remains blocked");
    f.adopt(&mother, &base);
    assert!(validate_snapshot(&f.store, &snapshot).is_err(), "restoring the parent is not automatic image approval");
    f.adopt(&child, &image);
    assert_eq!(f.store.active_asset("child").unwrap()["imageStatus"], "adopted");
    validate_snapshot(&f.store, &snapshot).unwrap();
    f.ready_snapshot(&unit);
    for job in original_jobs {
        assert_eq!(f.store.job(s(&job, "id")).unwrap(), job, "review does not rewrite raw, receipt, frozen request or UNKNOWN");
    }
    assert_eq!(f.store.job(&unknown).unwrap()["status"], "submission_unknown");
}

#[test]
fn adopted_design_change_blocks_semantic_consumer_without_invalidating_same_family() {
    let f = Fixture::new();
    let mother = f.asset("mother", None);
    let child = f.asset("design-child", Some("mother"));
    let independent = f.asset("same-family-independent", None);
    for (asset, tone) in [(&child, 20), (&independent, 40)] {
        let image = f.candidate(asset, tone, vec![]);
        f.adopt(asset, &image);
    }
    let unit = f.unit("consumer", "design-child");
    let snapshot = f.ready_snapshot(&unit);
    let independent_unit = f.unit("independent", "same-family-independent");
    let independent_snapshot = f.ready_snapshot(&independent_unit);
    let before = f.store.active_asset("design-child").unwrap();
    let mut updated = mother;
    updated["designRevision"] = json!(2);
    updated["adoptedDesign"]["designRevision"] = json!(2);
    updated["adoptedDesign"]["full"] = json!("新采用的母设计");
    updated["adoptedDesign"]["adoptionBasis"]["revision"] = json!(2);
    f.store.save_asset(updated).unwrap();
    let affected = f.store.active_asset("design-child").unwrap();
    assert_eq!(affected["designStatus"], "needs_review");
    assert_eq!(affected["version"], before["version"]);
    assert!(validate_snapshot(&f.store, &snapshot).unwrap_err().contains("设计待复核"));
    let blocked = bind(&f.store, &unit, "bh", "offline").unwrap();
    assert!(blocked["problems"].as_array().unwrap().iter().any(|p| p["code"] == "ASSET_DESIGN_REVIEW_REQUIRED"));
    validate_snapshot(&f.store, &independent_snapshot).unwrap();
}

#[test]
fn inline_appearance_checks_actual_donor_and_semantics_but_not_non_consumed_ancestors() {
    let f = Fixture::new();
    let donor = f.asset("donor", None);
    let donor_image = f.candidate(&donor, 20, vec![]);
    f.adopt(&donor, &donor_image);
    let unrelated = f.asset("family-ancestor", None);
    f.set_review_projection(s(&unrelated, "id"), "imageStatus", "needs_review");
    let mut inline = f.asset("inline", None);
    inline["preparation"] = json!({"mode":"INLINE"});
    inline["sourceAssetId"] = json!("donor");
    f.store.save_asset(inline).unwrap();
    let unit = f.unit("inline-unit", "inline");
    let snapshot = f.ready_snapshot(&unit);
    assert_eq!(snapshot["references"][0]["assetId"], "donor");
    f.set_review_projection("donor", "imageStatus", "needs_review");
    assert!(validate_snapshot(&f.store, &snapshot).unwrap_err().contains("图片待复核"));
    assert!(!bind(&f.store, &unit, "bh", "offline").unwrap()["problems"].as_array().unwrap().is_empty());
    f.set_review_projection("donor", "imageStatus", "adopted");
    f.set_review_projection("inline", "designStatus", "needs_review");
    assert!(validate_snapshot(&f.store, &snapshot).unwrap_err().contains("设计待复核"));
    f.set_review_projection("inline", "designStatus", "adopted");
    // An INLINE appearance with its own explicitly adopted image does not
    // consume the older donor. Its parent metadata must not widen the gate.
    let inline = f.store.active_asset("inline").unwrap();
    let own = f.candidate(&inline, 60, vec![]);
    f.adopt(&inline, &own);
    f.set_review_projection("donor", "imageStatus", "needs_review");
    let current = f.ready_snapshot(&unit);
    assert_eq!(current["references"][0]["assetId"], "inline");
}

#[test]
fn inline_text_is_affected_by_design_review_not_an_unused_image_status() {
    let f = Fixture::new();
    let mut text = f.asset("text-interface", None);
    text["kind"] = json!("PROP");
    text["preparation"] = json!({"mode":"INLINE","note":"屏幕显示原文"});
    f.store.save_asset(text).unwrap();
    f.set_review_projection("text-interface", "imageStatus", "needs_review");
    let unit = f.unit("text-unit", "text-interface");
    let snapshot = f.ready_snapshot(&unit);
    assert_eq!(snapshot["references"], json!([]));
    assert_eq!(snapshot["inlineReferences"].as_array().unwrap().len(), 1);
    f.set_review_projection("text-interface", "designStatus", "needs_review");
    assert!(validate_snapshot(&f.store, &snapshot).unwrap_err().contains("设计待复核"));
    let blocked = bind(&f.store, &unit, "bh", "offline").unwrap();
    assert!(blocked["problems"].as_array().unwrap().iter().any(|p| p["code"] == "ASSET_DESIGN_REVIEW_REQUIRED"));
}

#[test]
fn imported_other_image_and_unknown_contract_cannot_bypass_review() {
    let f = Fixture::new();
    let mut other = f.asset("overlay", None);
    other["lingmoType"] = json!("OM");
    other = f.store.save_asset(other).unwrap();
    let image = f.candidate(&other, 20, vec![]);
    f.adopt(&other, &image);
    let unit = f.unit("overlay-unit", "overlay");
    let mut snapshot = f.ready_snapshot(&unit);
    snapshot["assetLifecycleContract"] = json!("unrecognized/v99");
    assert!(validate_snapshot(&f.store, &snapshot).unwrap_err().contains("未知资产复核合同"));
    f.set_review_projection("overlay", "imageStatus", "needs_review");
    let current = f.store.active_asset("overlay").unwrap();
    assert!(bind_imported_media(&f.store, &current, &json!({}), 0).unwrap_err().contains("图片待复核"));
}

// Reproduce old receipt storage only in this isolated synthetic fixture. The
// application must not rewrite an on-site asset or receipt to pass acceptance.
fn retain_legacy_negative(f: &Fixture, media: &Value) {
    let review = json!({"version":"asset-visual-review/v1","status":"needs_repair",
        "mediaId":media["id"],"output":{"decision":"REPAIR"},
        "manifestSha256":"synthetic-retained-receipt","basisSha256":"synthetic-old-basis"});
    f.store.conn().unwrap().execute(
        "INSERT INTO metadata(key,value) VALUES(?1,?2)",
        rusqlite::params![format!("visual-review.media:{}", s(media, "id")), review.to_string()],
    ).unwrap();
}

#[test]
fn retained_negative_review_gates_new_direct_and_inline_preparation_without_changing_history() {
    for mode in ["REFERENCE", "INLINE-own", "INLINE-donor"] {
        let f = Fixture::new();
        let donor = f.asset("donor", None);
        let donor_image = f.candidate(&donor, 20, vec![]);
        f.adopt(&donor, &donor_image);
        let mut consumer = f.asset("consumer", None);
        if mode.starts_with("INLINE") {
            consumer["preparation"] = json!({"mode":"INLINE"});
            consumer["sourceAssetId"] = json!("donor");
            consumer = f.store.save_asset(consumer).unwrap();
        }
        let negative = if mode == "INLINE-donor" {
            donor_image
        } else {
            let own = f.candidate(&consumer, 40, vec![]);
            f.adopt(&consumer, &own);
            own
        };
        let independent = f.asset("same-family-independent", None);
        let independent_image = f.candidate(&independent, 60, vec![]);
        f.adopt(&independent, &independent_image);
        let unit = f.unit("negative-consumer", "consumer");
        let independent_unit = f.unit("independent-unit", "same-family-independent");
        let snapshot = f.ready_snapshot(&unit);
        let independent_snapshot = f.ready_snapshot(&independent_unit);
        let mut legacy = snapshot.clone();
        legacy.as_object_mut().unwrap().remove("assetLifecycleContract");
        let frozen_bytes = serde_json::to_vec(&legacy).unwrap();
        let donor_id = s(&snapshot["references"][0], "assetId");
        let raw_asset = f.store.get("assets", donor_id).unwrap();
        assert_eq!(raw_asset["imageStatus"], "adopted");
        retain_legacy_negative(&f, &negative);
        let jobs = f.store.jobs().unwrap();
        let image_bytes = std::fs::read(f.store.verify_media(&negative).unwrap()).unwrap();

        for _ in 0..2 {
            let prepared = bind(&f.store, &unit, "bh", "offline").unwrap();
            assert_eq!(prepared["references"], json!([]), "{mode}: bad image reached new snapshot");
            let problems = prepared["problems"].as_array().unwrap();
            assert_eq!(problems.len(), 2, "{mode}: unrelated preparation issue: {prepared}");
            assert_eq!(problems[0]["code"], "MATERIAL_SOURCE_UNRESOLVED");
            assert_eq!(problems[0]["assetId"], "consumer");
            assert!(s(&problems[0], "message").contains("图片待复核"));
            assert_eq!(problems[1]["code"], "UNBOUND_PLACEHOLDER");
            assert!(s(&problems[1], "assetId").is_empty());
            assert!(s(&prepared, "prompt").contains("{{asset:consumer:v1}}"));
            assert!(validate_snapshot(&f.store, &snapshot).unwrap_err().contains("图片待复核"));
            validate_snapshot(&f.store, &legacy).unwrap();
            validate_snapshot(&f.store, &independent_snapshot).unwrap();
            f.ready_snapshot(&independent_unit);
        }
        let reopened = Store::open(f.store.root.clone()).unwrap();
        assert!(!bind(&reopened, &unit, "bh", "offline").unwrap()["problems"].as_array().unwrap().is_empty());
        validate_snapshot(&reopened, &legacy).unwrap();
        assert_eq!(serde_json::to_vec(&legacy).unwrap(), frozen_bytes);
        assert_eq!(f.store.get("assets", donor_id).unwrap(), raw_asset);
        assert_eq!(f.store.get("units", s(&unit, "id")).unwrap(), unit);
        assert_eq!(f.store.adopted(donor_id, "v1").unwrap().as_deref(), negative["id"].as_str());
        assert_eq!(f.store.jobs().unwrap(), jobs);
        assert_eq!(std::fs::read(f.store.verify_media(&negative).unwrap()).unwrap(), image_bytes);
    }
}

#[test]
fn retained_negative_review_does_not_hide_existing_deleted_variant_or_scope_errors() {
    for (fault, code) in [
        ("deleted", "ASSET_DELETED"),
        ("variant", "VARIANT_MISMATCH"),
        ("scope", "OUT_OF_SCOPE"),
    ] {
        let f = Fixture::new();
        let asset = f.asset("person", None);
        let image = f.candidate(&asset, 30, vec![]);
        f.adopt(&asset, &image);
        let mut unit = f.unit("consumer", "person");
        let mut raw = f.store.get("assets", "person").unwrap();
        retain_legacy_negative(&f, &image);
        match fault {
            "deleted" => raw["deletedAt"] = json!("2026-10-26T00:00:00Z"),
            "variant" => unit["assetRefs"][0]["variantId"] = json!("missing-version"),
            "scope" => raw["applicability"]["episodes"] = json!(["other-episode"]),
            _ => unreachable!(),
        }
        f.store.conn().unwrap().execute(
            "UPDATE assets SET data=?1 WHERE id='person'", [raw.to_string()],
        ).unwrap();
        let prepared = bind(&f.store, &unit, "bh", "offline").unwrap();
        let problems = prepared["problems"].as_array().unwrap();
        assert_eq!(problems.len(), 2, "{fault}: {prepared}");
        assert_eq!(problems[0]["code"], code);
        assert_eq!(problems[0]["assetId"], "person");
        assert_eq!(problems[1]["code"], "UNBOUND_PLACEHOLDER");
        assert!(s(&problems[1], "assetId").is_empty());
        assert!(s(&prepared, "prompt").contains("{{asset:person:v1}}"));
    }
}
