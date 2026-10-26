use super::*;
use crate::{asset_graph, lingmo_import, lingmo_media, pipeline::digest};

const MD: &str = "第1集\n1-1\n场景：门口\n人物：甲、甲礼服\n片段1\n时长：4秒\n@[门口SC] @[甲CH] @[甲礼服CH]\n分镜1｜0-2.7s｜2.7s\n画面：甲开门。\n";

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store
            .create("正式图片导出离线验收", "export-fixture")
            .unwrap();
        let output = dir.path().join("output");
        fs::create_dir(&output).unwrap();
        Self { dir, store, output }
    }
    fn asset(&self, key: &str, name: Option<&str>) -> Value {
        self.store.save_asset(json!({"id":key,"variantId":format!("{key}-v1"),
            "name":format!("可修改的显示名-{key}"),"kind":"character","identityId":"fixture-person",
            "workflowProfile":asset_graph::PROFILE,"assetRevision":7,"designRevision":3,
            "filename":name,"referenceRequirements":[],"prompt":"保留的手写提示词","promptOrigin":"MANUAL"})).unwrap()
    }
    fn media(&self, asset: &Value, seed: u8, format: ImageFormat, adopt: bool) -> Value {
        let path = self.dir.path().join(format!(
            "{}-{seed}.{}",
            s(asset, "id"),
            format.extensions_str()[0]
        ));
        image::RgbImage::from_fn(9, 7, |x, y| {
            image::Rgb([seed, (x * 17) as u8, (y * 19) as u8])
        })
        .save_with_format(&path, format)
        .unwrap();
        self.import(asset, &path, adopt)
    }
    fn import(&self, asset: &Value, path: &Path, adopt: bool) -> Value {
        let media = self
            .store
            .import_media(
                path,
                "image",
                Some(s(asset, "id")),
                Some(s(asset, "variantId")),
            )
            .unwrap();
        if adopt {
            self.store
                .adopt(s(asset, "id"), s(asset, "variantId"), s(&media, "id"))
                .unwrap();
        }
        media
    }
    fn export(&self) -> Value {
        self.store
            .export_current_images(&json!({"path":self.output}))
            .unwrap()
    }
    fn import_md(&self) {
        lingmo_import::import(&self.store, &json!({"text":MD,"expectedSha256":digest(MD)}))
            .unwrap();
    }
    fn raw_asset(&self, mut asset: Value, key: &str, value: Value) {
        asset[key] = value;
        self.store
            .conn()
            .unwrap()
            .execute(
                "UPDATE assets SET data=?1 WHERE id=?2",
                params![asset.to_string(), s(&asset, "id")],
            )
            .unwrap();
    }
    fn raw_media(&self, mut media: Value, key: &str, value: Value) {
        media[key] = value;
        self.store
            .conn()
            .unwrap()
            .execute(
                "UPDATE media SET data=?1 WHERE id=?2",
                params![media.to_string(), s(&media, "id")],
            )
            .unwrap();
    }
    fn metadata(&self, key: &str, value: Value) {
        self.store.conn().unwrap().execute("INSERT INTO metadata VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value.to_string()]).unwrap();
    }
    fn assert_no_jobs(&self) {
        assert!(self.store.job_summaries().unwrap().is_empty());
        let n: i64 = self
            .store
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}

fn manifest(result: &Value) -> Value {
    serde_json::from_slice(&fs::read(s(result, "manifestPath")).unwrap()).unwrap()
}
fn flat_files(result: &Value) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(s(result, "imageDirectory"))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            assert!(
                entry.file_type().unwrap().is_file(),
                "only flat images belong here"
            );
            entry.file_name().to_string_lossy().to_string()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn adoption_status_excludes_unusable_current_images_but_keeps_legacy_selected_candidates() {
    let f = Fixture::new();
    let asset = f.asset("person", Some("合成人物CH.png"));
    let current = f.media(&asset, 34, ImageFormat::Png, true);
    let candidate = f.media(&asset, 78, ImageFormat::Png, false);
    let bytes = fs::read(s(&current, "path")).unwrap();
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
        let before = freeze(&f.store).unwrap();
        let result = f.export();
        assert_eq!(result["count"], if blocked { 0 } else { 1 }, "{status:?}");
        if blocked {
            assert_eq!(result["status"], "no_ready_images");
            assert_eq!(result["missing"][0]["reason"], "needs_review");
            assert!(flat_files(&result).is_empty());
        } else {
            assert_eq!(result["status"], "complete");
            assert_eq!(manifest(&result)["exported"][0]["mediaId"], current["id"]);
            assert_ne!(manifest(&result)["exported"][0]["mediaId"], candidate["id"]);
            assert_eq!(fs::read(Path::new(s(&result, "imageDirectory")).join("合成人物CH.png")).unwrap(), bytes);
        }
        assert_eq!(freeze(&f.store).unwrap(), before, "{status:?}");
        assert_eq!(f.store.adopted("person", s(&asset, "variantId")).unwrap().as_deref(), current["id"].as_str());
    }
    assert_eq!(fs::read(s(&current, "path")).unwrap(), bytes);
    f.assert_no_jobs();
}

#[test]
fn current_png_export_preserves_bytes_versions_and_previous_package_without_writes() {
    let f = Fixture::new();
    let asset = f.asset("person", Some("甲礼服CH.png"));
    let old = f.media(&asset, 11, ImageFormat::Png, true);
    let current = f.media(&asset, 35, ImageFormat::Png, true);
    let candidate = f.media(&asset, 79, ImageFormat::Png, false);
    // Adopted imports retain their historical candidate record; that record
    // must not override the authoritative adoptions row.
    f.metadata(
        &format!(
            "lingmo.media.candidate:{}:{}:{}",
            s(&asset, "id"),
            s(&asset, "variantId"),
            s(&current, "id")
        ),
        json!({"originalCandidate":true}),
    );
    f.store
        .save_settings(
            &json!({"privateChannelFixture":{"credential":"private-fixture-token-never-export"}}),
        )
        .unwrap();
    assert_eq!(
        f.store.settings().unwrap()["privateChannelFixture"]["credential"],
        "private-fixture-token-never-export"
    );
    let before = freeze(&f.store).unwrap();
    let current_bytes = fs::read(s(&current, "path")).unwrap();
    let result = f.export();
    assert_eq!(result["status"], "complete");
    assert_eq!(result["scopeCount"], 1);
    assert_eq!(result["count"], 1);
    assert_eq!(result["missingCount"], 0);
    assert_eq!(result["generationSubmissions"], 0);
    assert_eq!(result["videoSubmissions"], 0);
    assert_eq!(flat_files(&result), vec!["甲礼服CH.png"]);
    let output = Path::new(s(&result, "imageDirectory")).join("甲礼服CH.png");
    assert_eq!(fs::read(&output).unwrap(), current_bytes);
    let report = manifest(&result);
    let row = &report["exported"][0];
    assert_eq!(row["mediaId"], current["id"]);
    assert_ne!(row["mediaId"], old["id"]);
    assert_ne!(row["mediaId"], candidate["id"]);
    assert_eq!(row["sourceSha256"], current["sha256"]);
    assert_eq!(row["outputSha256"], current["sha256"]);
    assert_eq!(row["assetVersion"], asset["version"]);
    assert_eq!(row["assetRevision"], 7);
    assert_eq!(row["designRevision"], 3);
    assert_eq!(row["adoption"]["mediaId"], current["id"]);
    assert_eq!(row["imageAdoptionBasis"]["mediaId"], current["id"]);
    assert_eq!(row["conversion"]["converted"], false);
    assert_eq!(row["filenameBasis"], "asset-filename");
    assert!(!report
        .to_string()
        .contains("private-fixture-token-never-export"));
    assert!(!Path::new(s(&result, "path")).join(".incomplete").exists());
    assert_eq!(freeze(&f.store).unwrap(), before);
    let first_manifest = fs::read(s(&result, "manifestPath")).unwrap();
    let reopened = Store::open(f.store.root.clone()).unwrap();
    let again = reopened
        .export_current_images(&json!({"path":f.output}))
        .unwrap();
    assert_ne!(again["path"], result["path"]);
    assert_eq!(
        fs::read(s(&result, "manifestPath")).unwrap(),
        first_manifest
    );
    assert_eq!(fs::read(&output).unwrap(), current_bytes);
    assert_eq!(fs::read(s(&current, "path")).unwrap(), current_bytes);
    assert_eq!(freeze(&reopened).unwrap(), before);
    f.assert_no_jobs();
}

#[test]
fn jpeg_and_webp_export_to_png_preserves_decoded_pixels_and_dimensions() {
    let f = Fixture::new();
    let mut originals = HashMap::new();
    for (key, format, seed) in [
        ("jpeg", ImageFormat::Jpeg, 24),
        ("webp", ImageFormat::WebP, 67),
    ] {
        let asset = f.asset(key, Some(&format!("{key}CH.png")));
        let media = f.media(&asset, seed, format, true);
        originals.insert(key.to_owned(), media);
    }
    let result = f.export();
    assert_eq!(result["count"], 2);
    for row in rows(&manifest(&result), "exported") {
        let source = &originals[s(&row, "assetId")];
        let output = Path::new(s(&result, "path")).join(s(&row, "outputPath"));
        assert_eq!(
            image::guess_format(&fs::read(&output).unwrap()).unwrap(),
            ImageFormat::Png
        );
        let original_pixels = image::open(s(source, "path")).unwrap().to_rgba8();
        let exported_pixels = image::open(&output).unwrap().to_rgba8();
        assert_eq!(exported_pixels.dimensions(), (9, 7));
        assert_eq!(exported_pixels, original_pixels);
        assert_eq!(row["sourceSha256"], source["sha256"]);
        assert_eq!(row["outputSha256"], hash_file(&output).unwrap().0);
        assert_eq!(row["conversion"]["converted"], true);
        assert_eq!(row["conversion"]["resampled"], false);
    }
    f.assert_no_jobs();
}

#[test]
fn partial_export_reports_candidates_placeholders_review_missing_and_corruption() {
    let f = Fixture::new();
    let good = f.asset("good", Some("正式CH.png"));
    f.media(&good, 17, ImageFormat::Png, true);
    // An obsolete asset-level white-package flag must not hide a real image.
    f.raw_asset(
        f.store.get("assets", "good").unwrap(),
        "placeholder",
        json!(true),
    );
    for (key, value, lie) in [("white", 255, false), ("legacy-white", 252, true)] {
        let asset = f.asset(key, Some(&format!("{key}CH.png")));
        let path = f.dir.path().join(format!("{key}.png"));
        image::RgbaImage::from_pixel(4, 4, image::Rgba([value, value, value, 255]))
            .save(&path)
            .unwrap();
        let media = f.import(&asset, &path, true);
        if lie {
            f.raw_media(media, "placeholder", json!(false));
        }
    }
    let review = f.asset("review", Some("待复核CH.png"));
    f.media(&review, 48, ImageFormat::Png, true);
    f.raw_asset(
        f.store.get("assets", "review").unwrap(),
        "imageStatus",
        json!("needs_review"),
    );
    let candidate = f.asset("candidate", Some("候选CH.png"));
    f.media(&candidate, 52, ImageFormat::Png, false);
    f.asset("nameless", None);
    let absent = f.asset("absent", Some("缺文件CH.png"));
    let media = f.media(&absent, 61, ImageFormat::Png, true);
    fs::remove_file(f.store.absolute_media(&media).unwrap()).unwrap();
    let corrupt = f.asset("corrupt", Some("损坏CH.png"));
    let media = f.media(&corrupt, 72, ImageFormat::Png, true);
    fs::write(
        f.store.absolute_media(&media).unwrap(),
        b"changed after adoption",
    )
    .unwrap();
    let reference = f.asset("reference", Some("参考母版CH.png"));
    f.media(&reference, 85, ImageFormat::Png, true);
    f.raw_asset(
        f.store.get("assets", "reference").unwrap(),
        "referenceOnly",
        json!(true),
    );
    let result = f.export();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["scopeCount"], 9);
    assert_eq!(result["count"], 1);
    assert_eq!(result["missingCount"], 7);
    assert_eq!(result["excludedCount"], 1);
    assert_eq!(flat_files(&result), vec!["正式CH.png"]);
    let issues: HashMap<_, _> = rows(&result, "missing")
        .into_iter()
        .map(|row| (s(&row, "assetId").to_owned(), s(&row, "reason").to_owned()))
        .collect();
    for (key, reason) in [
        ("white", "placeholder"),
        ("legacy-white", "placeholder"),
        ("review", "needs_review"),
        ("candidate", "not_adopted"),
        ("nameless", "filename_missing"),
        ("absent", "media_unavailable"),
        ("corrupt", "media_unavailable"),
    ] {
        assert_eq!(issues.get(key).map(String::as_str), Some(reason), "{key}");
    }
    assert_eq!(result["excluded"][0]["assetId"], reference["id"]);
    assert!(!s(&result["missing"][0], "message").is_empty());
    f.assert_no_jobs();
}

#[test]
fn formal_scope_never_expands_to_unused_assets_or_registered_reference_mothers() {
    let f = Fixture::new();
    let mother = f.asset("mother", Some("母版CH.png"));
    let mut child = f.asset("child", Some("正式礼服CH.png"));
    child["referenceRequirements"] = json!([{"targetAssetId":"mother","targetVariantId":mother["variantId"],"role":"identity-faces","required":true}]);
    let child = f.store.save_asset(child).unwrap();
    let unused = f.asset("unused", Some("正式礼服CH.png"));
    f.store.save_unit(json!({"id":"u1","episodeId":"e1","prompt":"采用稿","imageBindings":[{"assetId":child["id"],"variantId":child["variantId"]}]})).unwrap();
    f.media(&mother, 3, ImageFormat::Png, true);
    f.media(&child, 16, ImageFormat::Png, true);
    f.media(&unused, 29, ImageFormat::Png, true);
    let mother = f.store.get("assets", "mother").unwrap();
    f.store.register_image_reference(&json!({"confirmed":true,"assetId":"mother","expectedVersion":mother["version"],"reason":"正式礼服的身份母版","servesAssetIds":["child"]})).unwrap();
    let scope = f.store.image_production_scope().unwrap();
    assert_eq!(scope["assetIds"], json!(["child"]));
    assert_eq!(scope["referenceOnlyIds"], json!(["mother"]));
    let result = f
        .store
        .export_current_images(&json!({"path":f.output,"assetIds":["unused","mother"]}))
        .unwrap();
    assert_eq!(result["scopeCount"], 1);
    assert_eq!(result["count"], 1);
    assert_eq!(flat_files(&result), vec!["正式礼服CH.png"]);
    assert_eq!(manifest(&result)["exported"][0]["assetId"], "child");
    f.assert_no_jobs();
}

#[test]
fn cross_platform_filename_collisions_block_before_creating_a_package() {
    for (left, right) in [
        ("A.png", "a.PNG"),
        ("ＡCH.png", "ACH.png"),
        ("StraßeCH.png", "STRASSECH.png"),
        ("e\u{301}CH.png", "éCH.png"),
    ] {
        let f = Fixture::new();
        f.asset("a", Some(left));
        f.asset("b", Some(right));
        let error = f
            .store
            .export_current_images(&json!({"path":f.output}))
            .unwrap_err();
        assert!(
            error.starts_with("IMAGE_EXPORT_FILENAME_COLLISION"),
            "{left} / {right}: {error}"
        );
        assert_eq!(fs::read_dir(&f.output).unwrap().count(), 0);
        f.assert_no_jobs();
    }
}

#[test]
fn invalid_cross_platform_filenames_are_rejected_without_sanitizing() {
    for name in [
        "CON.png",
        "aux.extra.png",
        "com¹.png",
        "LPT9.PNG",
        "../甲.png",
        "子目录/甲.png",
        "子目录\\甲.png",
        "C:甲.png",
        "甲?.png",
        "甲.png ",
        ".png",
        "甲.jpg",
        "\n甲.png",
    ] {
        let f = Fixture::new();
        f.asset("a", Some(name));
        let error = f
            .store
            .export_current_images(&json!({"path":f.output}))
            .unwrap_err();
        assert!(
            error.starts_with("IMAGE_EXPORT_FILENAME_INVALID"),
            "{name:?}: {error}"
        );
        assert_eq!(fs::read_dir(&f.output).unwrap().count(), 0);
    }
    assert!(valid_filename("甲｜礼服CH.png").is_ok());
    assert!(valid_filename("COM10.png").is_ok());
}

#[test]
fn exact_adopted_index_filename_is_used_and_conflicting_mapping_is_not_guessed() {
    let f = Fixture::new();
    let asset = f.asset("person", None);
    f.media(&asset, 35, ImageFormat::Png, true);
    let delivery = json!({"assetIndex":{"assets":[{"id":asset["id"],"variantId":asset["variantId"],"filename":"既定正式名称CH.png"}]},
        "bundle":{"draft":{"units":[{"id":"u1","imageBindings":[{"assetId":asset["id"],"variantId":asset["variantId"]}]}]}}});
    f.metadata("series.delivery:e1", delivery);
    let result = f.export();
    assert_eq!(flat_files(&result), vec!["既定正式名称CH.png"]);
    assert_eq!(
        manifest(&result)["exported"][0]["filenameBasis"],
        "adopted-asset-index"
    );
    let mut current = f.store.get("assets", "person").unwrap();
    current["filename"] = json!("另一个名字CH.png");
    f.store.save_asset(current).unwrap();
    let error = f
        .store
        .export_current_images(&json!({"path":f.output}))
        .unwrap_err();
    assert!(
        error.starts_with("IMAGE_EXPORT_FILENAME_MAPPING_CONFLICT"),
        "{error}"
    );
    assert_eq!(fs::read_dir(&f.output).unwrap().count(), 1);
    assert_eq!(flat_files(&result), vec!["既定正式名称CH.png"]);
}

#[test]
fn absent_adoption_creates_an_explicit_empty_report_without_exporting_candidate() {
    let f = Fixture::new();
    let asset = f.asset("a", Some("甲CH.png"));
    f.media(&asset, 3, ImageFormat::Png, false);
    let result = f.export();
    assert_eq!(result["status"], "no_ready_images");
    assert_eq!(result["scopeCount"], 1);
    assert_eq!(result["count"], 0);
    assert_eq!(result["missing"][0]["reason"], "not_adopted");
    assert!(flat_files(&result).is_empty());
    assert!(manifest(&result)["exported"].as_array().unwrap().is_empty());
    f.assert_no_jobs();
}

#[test]
fn changed_adoption_before_publication_aborts_and_only_removes_this_staging_package() {
    let f = Fixture::new();
    let asset = f.asset("a", Some("甲CH.png"));
    let old = f.media(&asset, 4, ImageFormat::Png, true);
    let prior = f.export();
    let prior_manifest = fs::read(s(&prior, "manifestPath")).unwrap();
    let prepared = prepare(&f.store, &json!({"path":f.output})).unwrap();
    let staging = prepared.directory.root.clone();
    let new = f.media(&asset, 26, ImageFormat::Png, true);
    let error = publish(&f.store, prepared).unwrap_err();
    assert!(
        error.starts_with("IMAGE_EXPORT_SNAPSHOT_CHANGED"),
        "{error}"
    );
    assert!(!staging.exists());
    assert_eq!(fs::read(s(&prior, "manifestPath")).unwrap(), prior_manifest);
    assert!(f.store.verify_media(&old).unwrap().is_file());
    assert_eq!(
        f.store
            .adopted(s(&asset, "id"), s(&asset, "variantId"))
            .unwrap()
            .as_deref(),
        Some(s(&new, "id"))
    );
    f.assert_no_jobs();
}

#[test]
fn changed_asset_or_formal_scope_before_publication_never_publishes_stale_mapping() {
    for change_scope in [false, true] {
        let f = Fixture::new();
        let asset = f.asset("a", Some("甲CH.png"));
        f.media(&asset, 10, ImageFormat::Png, true);
        let prepared = prepare(&f.store, &json!({"path":f.output})).unwrap();
        let staging = prepared.directory.root.clone();
        if change_scope {
            f.asset("new-scope", Some("新增SC.png"));
        } else {
            let mut current = f.store.get("assets", "a").unwrap();
            current["filename"] = json!("新正式名称CH.png");
            f.store.save_asset(current).unwrap();
        }
        let error = publish(&f.store, prepared).unwrap_err();
        assert!(
            error.starts_with("IMAGE_EXPORT_SNAPSHOT_CHANGED"),
            "{error}"
        );
        assert!(!staging.exists());
        f.assert_no_jobs();
    }
}

#[test]
fn modified_source_or_export_pixels_before_publication_fail_hash_verification() {
    for change_source in [false, true] {
        let f = Fixture::new();
        let asset = f.asset("a", Some("甲CH.png"));
        let media = f.media(&asset, 12, ImageFormat::Png, true);
        let prepared = prepare(&f.store, &json!({"path":f.output})).unwrap();
        let staging = prepared.directory.root.clone();
        let target = if change_source {
            f.store.absolute_media(&media).unwrap()
        } else {
            staging.join(IMAGE_DIR).join("甲CH.png")
        };
        fs::write(&target, b"not the approved pixels").unwrap();
        let error = publish(&f.store, prepared).unwrap_err();
        assert!(
            if change_source {
                error.contains("媒体哈希不一致")
            } else {
                error.starts_with("IMAGE_EXPORT_OUTPUT_CHANGED")
            },
            "{error}"
        );
        assert!(!staging.exists());
        if change_source {
            assert_eq!(fs::read(target).unwrap(), b"not the approved pixels");
        }
        f.assert_no_jobs();
    }
}

#[test]
fn md_token_export_reimports_as_exact_current_variants_without_generation_or_rewriting() {
    let source = Fixture::new();
    source.import_md();
    let source_assets = source.store.values("assets").unwrap();
    let units = source.store.values("units").unwrap();
    for (i, asset) in source_assets.iter().enumerate() {
        source.media(asset, 17 + i as u8 * 11, ImageFormat::Png, true);
    }
    let result = source.export();
    assert_eq!(result["count"], 3);
    assert_eq!(result["status"], "complete");
    assert_eq!(
        flat_files(&result),
        vec!["甲CH.png", "甲礼服CH.png", "门口SC.png"]
    );
    let report = manifest(&result);
    for row in rows(&report, "exported") {
        assert_eq!(row["filenameBasis"], "source-token-contract");
        let token = s(&row, "token")
            .strip_prefix("@[")
            .unwrap()
            .strip_suffix(']')
            .unwrap();
        assert_eq!(row["filename"], format!("{token}.png"));
    }
    let target = Fixture::new();
    target.import_md();
    let preview = lingmo_media::preview(
        &target.store,
        &json!({"documentId":digest(MD),"directory":result["imageDirectory"]}),
    )
    .unwrap();
    assert_eq!(preview["counts"]["matched"], 3);
    assert_eq!(preview["counts"]["unmatched"], 0);
    assert_eq!(preview["counts"]["ambiguous"], 0);
    let selections: Vec<_> = rows(&preview,"rows").iter().map(|row| json!({"rowId":row["id"],"assetId":row["assetId"],"variantId":row["variantId"],"adopt":true,"expectedAdoptedMediaId":row["expectedAdoptedMediaId"]})).collect();
    let imported = lingmo_media::commit(&target.store, &json!({"documentId":digest(MD),"previewToken":preview["previewToken"],"selections":selections})).unwrap();
    assert_eq!(imported["report"]["adopted"], 3);
    assert_eq!(imported["report"]["failed"], 0);
    assert_eq!(imported["report"]["placeholders"], 0);
    assert_eq!(imported["generationSubmissions"], 0);
    let reopened = Store::open(target.store.root.clone()).unwrap();
    for original in &source_assets {
        let target_asset = reopened
            .values("assets")
            .unwrap()
            .into_iter()
            .find(|a| a["token"] == original["token"])
            .unwrap();
        assert_eq!(target_asset["variantId"], original["variantId"]);
        let source_id = source
            .store
            .adopted(s(original, "id"), s(original, "variantId"))
            .unwrap()
            .unwrap();
        let target_id = reopened
            .adopted(s(&target_asset, "id"), s(&target_asset, "variantId"))
            .unwrap()
            .unwrap();
        assert_eq!(
            source.store.get("media", &source_id).unwrap()["sha256"],
            reopened.get("media", &target_id).unwrap()["sha256"]
        );
    }
    assert_eq!(source.store.values("units").unwrap(), units);
    assert_eq!(
        lingmo_import::original(&reopened, &digest(MD), None).unwrap()["text"],
        MD
    );
    assert!(source.store.source().unwrap().is_null());
    assert!(reopened.source().unwrap().is_null());
    source.assert_no_jobs();
    target.assert_no_jobs();
}

#[test]
fn canonical_and_native_absolute_paths_work_and_empty_scope_has_no_output() {
    let f = Fixture::new();
    no_links(&f.output).unwrap();
    no_links(&f.output.canonicalize().unwrap()).unwrap();
    assert!(no_links(Path::new("relative/path")).is_err());
    let error = f
        .store
        .export_current_images(&json!({"path":f.output}))
        .unwrap_err();
    assert!(error.contains("范围为空"), "{error}");
    assert_eq!(fs::read_dir(&f.output).unwrap().count(), 0);
    assert!(f
        .store
        .export_current_images(&json!({"path":"relative"}))
        .is_err());
    assert!(f
        .store
        .export_current_images(&json!({"path":f.dir.path().join("missing")}))
        .is_err());
}

#[test]
fn recorded_dimensions_disagreeing_with_original_are_reported_without_export() {
    let f = Fixture::new();
    let asset = f.asset("a", Some("甲CH.png"));
    let media = f.media(&asset, 14, ImageFormat::Png, true);
    f.raw_media(media, "width", json!(99));
    let result = f.export();
    assert_eq!(result["status"], "no_ready_images");
    assert_eq!(result["missing"][0]["reason"], "media_unavailable");
    assert!(s(&result["missing"][0], "message").contains("尺寸"));
    assert!(flat_files(&result).is_empty());
}

#[cfg(unix)]
#[test]
fn export_parent_symlink_is_rejected_without_writing_to_its_target() {
    let f = Fixture::new();
    f.asset("a", Some("甲CH.png"));
    let link = f.dir.path().join("linked-output");
    std::os::unix::fs::symlink(&f.output, &link).unwrap();
    let error = f
        .store
        .export_current_images(&json!({"path":link}))
        .unwrap_err();
    assert!(error.starts_with("IMAGE_EXPORT_LINK_PATH"), "{error}");
    assert_eq!(fs::read_dir(&f.output).unwrap().count(), 0);
}
