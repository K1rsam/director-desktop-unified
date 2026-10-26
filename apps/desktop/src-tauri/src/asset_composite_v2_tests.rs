use super::*;

fn fixture(size: u32, top: u32) -> (tempfile::TempDir, Value, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mother = dir.path().join("mother.png");
    RgbaImage::from_fn(size, size, |x, y| {
        image::Rgba([(x % 253) as u8, (y % 251) as u8, 29, 255])
    })
    .save(&mother)
    .unwrap();
    let sha = hash(&fs::read(&mother).unwrap());
    let mut snapshot = json!({"imagePresentation":{"layoutKind":"costume"},
        "references":[{"mediaId":"mother-v3","assetId":"mother","kind":"image","path":mother,"sha256":sha,"mediaRevision":3}],
        "identityComposite":{"version":LOWER_ADAPTIVE_PROFILE,"width":size,"height":size,
            "motherMediaId":"mother-v3","motherMediaRevision":3,"motherSha256":sha,
            "identityStrip":{"x":0,"y":0,"width":size,"height":top}}});
    snapshot["identityComposite"]["lowerAdaptation"] =
        lower_adaptation_contract(&snapshot["identityComposite"]).unwrap();
    let lower = dir.path().join("original-download.png");
    let output = dir.path().join("composed-candidate.png");
    (dir, snapshot, lower, output)
}

fn lower_png(path: &Path, size: u32, source_top: u32, top_color: [u8; 4]) {
    RgbaImage::from_fn(size, size, |_, y| {
        image::Rgba(if y < source_top {
            top_color
        } else {
            [11, 79, 131, 255]
        })
    })
    .save(path)
    .unwrap();
}

#[test]
fn adaptive_lower_drops_polluted_top_before_up_or_down_sampling() {
    for (native, source_top) in [(8, 3), (12, 4), (20, 7)] {
        let (_dir, snapshot, lower, output) = fixture(12, 4);
        lower_png(&lower, native, source_top, [255, 0, 255, 255]);
        let original = fs::read(&lower).unwrap();
        let mother_bytes = fs::read(s(&snapshot["references"][0], "path")).unwrap();
        let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
        assert_eq!(result["status"], "composited_candidate", "{result}");
        assert_eq!(result["proof"]["version"], LOWER_ADAPTIVE_PROFILE);
        assert_eq!(result["proof"]["identityResampling"], false);
        assert_eq!(result["proof"]["lowerResampling"], native != 12);
        assert_eq!(result["proof"]["resampling"], native != 12);
        assert_eq!(result["proof"]["outsideRegionsUnchanged"], native == 12);
        assert_eq!(
            result["proof"]["lowerTransform"]["sourceCrop"]["y"],
            source_top
        );
        assert_eq!(
            result["proof"]["lowerTransform"]["interpolation"],
            if native == 12 { "none" } else { "Lanczos3" }
        );
        assert_eq!(result["imageApproved"], false);
        assert_eq!(result["visualReviewRequired"], true);
        let actual = image::open(&output).unwrap().to_rgba8();
        let mother = image::load_from_memory(&mother_bytes).unwrap().to_rgba8();
        assert_eq!(actual.dimensions(), (12, 12));
        for y in 0..12 {
            for x in 0..12 {
                assert_eq!(
                    *actual.get_pixel(x, y),
                    if y < 4 {
                        *mother.get_pixel(x, y)
                    } else {
                        image::Rgba([11, 79, 131, 255])
                    }
                );
            }
        }
        validate_adoption(&snapshot, &result, &output).unwrap();
        assert_eq!(fs::read(&lower).unwrap(), original);
        assert_eq!(
            fs::read(s(&snapshot["references"][0], "path")).unwrap(),
            mother_bytes
        );
        let repeated = compose_downloaded(&snapshot, &lower, &output).unwrap();
        assert_eq!(repeated["proof"]["reused"], true);
        assert_eq!(
            repeated["proof"]["outputSha256"],
            result["proof"]["outputSha256"]
        );
        // Changing any discarded pixel cannot affect even the boundary row.
        lower_png(&lower, native, source_top, [0, 255, 0, 255]);
        let output2 = output.with_file_name("other-top.png");
        let result2 = compose_downloaded(&snapshot, &lower, &output2).unwrap();
        assert_eq!(result2["status"], "composited_candidate");
        assert_eq!(fs::read(&output2).unwrap(), fs::read(&output).unwrap());
        assert_ne!(
            result2["proof"]["lowerSha256"],
            result["proof"]["lowerSha256"]
        );
    }
}

#[test]
fn adaptive_lower_real_dimension_ratio_preserves_all_591888_identity_pixels() {
    let (_dir, snapshot, lower, output) = fixture(1254, 472);
    lower_png(&lower, 1024, 386, [255, 0, 255, 255]);
    let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
    assert_eq!(result["status"], "composited_candidate", "{result}");
    assert_eq!(
        result["proof"]["lowerTransform"]["sourceCrop"],
        json!({"x":0,"y":386,"width":1024,"height":638})
    );
    assert_eq!(
        result["proof"]["lowerTransform"]["target"],
        json!({"x":0,"y":472,"width":1254,"height":782})
    );
    let actual = image::open(&output).unwrap().to_rgba8();
    let mother = image::open(s(&snapshot["references"][0], "path"))
        .unwrap()
        .to_rgba8();
    let mut checked = 0;
    for y in 0..472 {
        for x in 0..1254 {
            assert_eq!(actual.get_pixel(x, y), mother.get_pixel(x, y));
            checked += 1;
        }
    }
    assert_eq!(checked, 591888);
    for x in 0..1254 {
        assert_eq!(*actual.get_pixel(x, 472), image::Rgba([11, 79, 131, 255]));
    }
    validate_adoption(&snapshot, &result, &output).unwrap();
}

#[test]
fn adaptive_lower_does_not_upgrade_frozen_v1_even_with_new_adaptation_fields() {
    let (_dir, mut snapshot, lower, output) = fixture(12, 4);
    snapshot["identityComposite"]["version"] = json!(PROFILE);
    lower_png(&lower, 20, 7, [255, 0, 255, 255]);
    let before = snapshot.clone();
    let original = fs::read(&lower).unwrap();
    let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
    assert_eq!(result["status"], "needs_review");
    assert!(s(&result, "reason").contains("不会自动缩放"), "{result}");
    assert_eq!(snapshot, before);
    assert_eq!(fs::read(&lower).unwrap(), original);
    assert!(!output.exists());
    assert!(validate_adoption(&snapshot, &result, &lower).is_err());
}

#[test]
fn adaptive_lower_rejects_missing_altered_ambiguous_and_non_square_contracts() {
    for case in [
        "missing",
        "filter",
        "ratio",
        "order",
        "slots",
        "both-layouts",
        "non-square",
        "too-small",
        "depth",
    ] {
        let (_dir, mut snapshot, lower, output) = fixture(12, 4);
        lower_png(&lower, 20, 7, [255, 0, 255, 255]);
        match case {
            "missing" => snapshot["identityComposite"]
                .as_object_mut()
                .unwrap()
                .remove("lowerAdaptation")
                .map(|_| ())
                .unwrap(),
            "filter" => {
                snapshot["identityComposite"]["lowerAdaptation"]["filter"] = json!("Nearest")
            }
            "ratio" => {
                snapshot["identityComposite"]["lowerAdaptation"]["sourceTop"]["numerator"] =
                    json!(3)
            }
            "order" => {
                snapshot["identityComposite"]["lowerAdaptation"]["order"] =
                    json!("resize-then-crop")
            }
            "slots" | "both-layouts" => {
                if case == "slots" {
                    snapshot["identityComposite"]
                        .as_object_mut()
                        .unwrap()
                        .remove("identityStrip");
                }
                snapshot["identityComposite"]["slots"] = json!([
                    {"source":{"x":0,"y":0,"width":2,"height":2},"target":{"x":0,"y":0}},
                    {"source":{"x":4,"y":0,"width":2,"height":2},"target":{"x":4,"y":0}},
                    {"source":{"x":8,"y":0,"width":2,"height":2},"target":{"x":8,"y":0}}
                ]);
            }
            "non-square" => RgbaImage::new(20, 19).save(&lower).unwrap(),
            "too-small" => lower_png(&lower, 1, 1, [255, 0, 0, 255]),
            "depth" => DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                20,
                20,
                image::Rgba([1000u16, 2000, 3000, 65535]),
            ))
            .save(&lower)
            .unwrap(),
            _ => unreachable!(),
        }
        let before = fs::read(&lower).unwrap();
        let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
        assert_eq!(result["status"], "needs_review", "{case}: {result}");
        assert_eq!(result["pixelEquality"], false);
        assert!(!output.exists(), "{case}");
        assert_eq!(fs::read(&lower).unwrap(), before);
    }
}

#[test]
fn adaptive_lower_adoption_rechecks_contract_identity_and_mapped_lower_pixels() {
    let (_dir, snapshot, lower, output) = fixture(12, 4);
    lower_png(&lower, 20, 7, [255, 0, 255, 255]);
    let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
    validate_adoption(&snapshot, &result, &output).unwrap();
    for pointer in [
        "/proof/identityResampling",
        "/proof/lowerResampling",
        "/proof/resampling",
        "/proof/lowerMatchesFrozenTransform",
    ] {
        let mut changed = result.clone();
        let value = changed.pointer_mut(pointer).unwrap();
        *value = json!(!value.as_bool().unwrap());
        assert!(
            validate_adoption(&snapshot, &changed, &output).is_err(),
            "{pointer}"
        );
    }
    let mut changed = result.clone();
    changed["proof"]["lowerTransform"]["sourceCrop"]["y"] = json!(6);
    assert!(validate_adoption(&snapshot, &changed, &output).is_err());
    changed = result.clone();
    changed["proof"]["version"] = json!(PROFILE);
    assert!(validate_adoption(&snapshot, &changed, &output).is_err());
    let original = fs::read(&output).unwrap();
    for y in [0, 4] {
        let mut tampered = image::load_from_memory(&original).unwrap().to_rgba8();
        tampered.put_pixel(0, y, image::Rgba([255, 0, 0, 255]));
        tampered.save(&output).unwrap();
        changed = result.clone();
        changed["proof"]["outputSha256"] = json!(hash(&fs::read(&output).unwrap()));
        assert!(
            validate_adoption(&snapshot, &changed, &output).is_err(),
            "tampered row {y}"
        );
    }
    fs::write(&output, original).unwrap();
    validate_adoption(&snapshot, &result, &output).unwrap();
}

#[test]
fn adaptive_lower_public_adoption_and_png_export_keep_identity_pixels_and_download() {
    let (dir, mut snapshot, lower, output) = fixture(12, 4);
    lower_png(&lower, 20, 7, [255, 0, 255, 255]);
    let original = fs::read(&lower).unwrap();
    let store = crate::store::Store::open(dir.path().join("project")).unwrap();
    store.create("合成导出离线验收", "fixture").unwrap();
    store.save_asset(json!({"id":"costume","variantId":"v1","kind":"CH","name":"换装","filename":"换装CH.png"})).unwrap();
    snapshot["assetId"] = json!("costume");
    snapshot["variantId"] = json!("v1");
    let lower_media = store.import_media(&lower, "image", None, None).unwrap();
    let job = store
        .add_job(
            "image",
            "costume",
            "offline",
            "offline",
            &snapshot,
            "downloaded",
        )
        .unwrap();
    store.attach_result(&job, s(&lower_media, "id")).unwrap();
    let mut composed = compose_downloaded(&snapshot, &lower, &output).unwrap();
    let candidate = store.import_media(&output, "image", None, None).unwrap();
    composed["candidateMediaId"] = candidate["id"].clone();
    composed["lowerMediaId"] = lower_media["id"].clone();
    store.attach_result(&job, s(&candidate, "id")).unwrap();
    store
        .update_job(
            &job,
            "downloaded",
            &json!({"result":{"identityComposition":[composed.clone()]}}),
        )
        .unwrap();
    let export_root = dir.path().join("exports");
    fs::create_dir(&export_root).unwrap();
    let before = store
        .export_current_images(&json!({"path":export_root}))
        .unwrap();
    assert_eq!(before["count"], 0);
    assert!(store.adopt("costume", "v1", s(&lower_media, "id")).is_err());
    // Explicit synthetic fixture adoption only; composition never auto-approves.
    store.adopt("costume", "v1", s(&candidate, "id")).unwrap();
    let reopened = crate::store::Store::open(store.root.clone()).unwrap();
    let exported = reopened
        .export_current_images(&json!({"path":export_root}))
        .unwrap();
    assert_eq!(exported["count"], 1, "{exported}");
    let exported_path = Path::new(s(&exported, "imageDirectory")).join("换装CH.png");
    assert_eq!(
        fs::read(&exported_path).unwrap(),
        fs::read(&output).unwrap()
    );
    validate_adoption(&snapshot, &composed, &exported_path).unwrap();
    assert_eq!(fs::read(&lower).unwrap(), original);
    assert_eq!(fs::read(s(&lower_media, "path")).unwrap(), original);
    assert_eq!(composed["imageApproved"], false);
    assert_eq!(exported["generationSubmissions"], 0);
    assert_eq!(exported["videoSubmissions"], 0);
}
