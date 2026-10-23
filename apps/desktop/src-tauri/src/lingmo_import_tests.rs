use super::*;
const TEXT:&str="**【标准化剧本】**\n第1集\n1-1\n场景：门口\n人物：甲\n片段1\n时长：4秒\n[场景] @[门口SC]\n[本单元出场人物] @[甲CH]\n分镜1｜0-2.7s｜2.7s\n画面：甲开门。\n";
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("project")).unwrap();
    store.create("导入验收", "test-project").unwrap();
    (dir, store)
}
fn request(text: &str) -> Value {
    json!({"text":text,"expectedSha256":digest(text)})
}
#[test]
fn transactional_idempotent_import_edit_reopen_and_export_preserve_original() {
    let (dir, store) = setup();
    let before = store.source().unwrap();
    let result = import(&store, &request(TEXT)).unwrap();
    assert_eq!(result["duplicate"], false);
    assert_eq!(store.source().unwrap(), before);
    assert!(store.job_summaries().unwrap().is_empty());
    assert_eq!(import(&store, &request(TEXT)).unwrap()["duplicate"], true);
    assert_eq!(store.values("units").unwrap().len(), 1);
    assert_eq!(documents(&store).unwrap().len(), 1);
    let mut unit = store.values("units").unwrap().remove(0);
    let original_unit = unit.clone();
    validate_unit(&store, &unit).unwrap();
    unit["expectedVersion"] = unit["version"].clone();
    unit["prompt"] = json!(s(&unit, "prompt").replace("甲开门。", "甲推门。"));
    let saved = save_unit(&store, &original_unit, unit).unwrap();
    assert_eq!(saved["version"], 2);
    assert!(s(&saved, "prompt").contains("甲推门。"));
    assert!(s(
        &original(&store, &digest(TEXT), Some(s(&saved, "id"))).unwrap(),
        "text"
    )
    .contains("甲开门。"));
    let reopened = Store::open(store.root.clone()).unwrap();
    assert_eq!(reopened.get("units", s(&saved, "id")).unwrap(), saved);
    let path = dir.path().join("export.director");
    crate::portable::export(&store, &path).unwrap();
    let restored = crate::portable::import(&path, &dir.path().join("restored")).unwrap();
    assert_eq!(documents(&restored).unwrap(), documents(&store).unwrap());
    assert_eq!(restored.get("units", s(&saved, "id")).unwrap(), saved);
    assert_eq!(
        original(&restored, &digest(TEXT), None).unwrap()["text"],
        TEXT
    );
    validate_unit(&restored, &saved).unwrap();
    assert!(restored.job_summaries().unwrap().is_empty());
}
#[test]
fn preview_drift_forged_provenance_and_revision_overwrite_are_rejected() {
    let (_dir, store) = setup();
    let mut req = request(TEXT);
    req["expectedSha256"] = json!("old");
    assert!(import(&store, &req).is_err());
    assert!(store.values("units").unwrap().is_empty());
    import(&store, &request(TEXT)).unwrap();
    let unit = store.values("units").unwrap().remove(0);
    let mut forged = unit.clone();
    forged["importSource"]["lineStart"] = json!(1);
    assert!(validate_unit(&store, &forged).is_err());
    let changed = TEXT.replace("甲开门。", "甲回头。");
    let mut req = request(&changed);
    assert!(import(&store, &req).is_err());
    let pre = preview(&store, &req).unwrap();
    assert_eq!(
        pre["revisionComparison"]["existingDocumentIds"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    req["allowNewRevision"] = json!(true);
    import(&store, &req).unwrap();
    assert_eq!(documents(&store).unwrap().len(), 2);
    assert_eq!(store.get("units", s(&unit, "id")).unwrap(), unit);
    assert!(compile_prompt(&unit, &store.values("assets").unwrap()).is_ok());
}
#[test]
fn source_and_structure_are_immutable_but_text_diagnostics_can_be_saved() {
    let (_dir, store) = setup();
    import(&store, &request(TEXT)).unwrap();
    let original = store.values("units").unwrap().remove(0);
    let mut edit = original.clone();
    edit["expectedVersion"] = edit["version"].clone();
    edit["episodeId"] = json!("forged");
    assert!(save_unit(&store, &original, edit).is_err());
    let mut edit = original.clone();
    edit["expectedVersion"] = edit["version"].clone();
    edit["prompt"] = json!(format!("{}\n@[未知ZZ]", s(&edit, "prompt")));
    let saved = save_unit(&store, &original, edit).unwrap();
    assert!(saved["importDiagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["blocking"] == true));
    assert!(compile_prompt(&saved, &store.values("assets").unwrap()).is_err());
}
#[test]
fn import_preserves_unknown_jobs_and_placeholder_candidates_are_not_auto_adopted() {
    let (dir, store) = setup();
    let job = store
        .add_job(
            "video",
            "old-unit",
            "offline",
            "original-account",
            &json!({"unitId":"old-unit","frozenOriginal":true}),
            "submission_unknown",
        )
        .unwrap();
    let frozen = store.job(&job).unwrap();
    import(&store, &request(TEXT)).unwrap();
    assert_eq!(store.job(&job).unwrap(), frozen);
    let asset = store.values("assets").unwrap().remove(0);
    let path = dir.path().join("white.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([255, 255, 255]))
        .save(&path)
        .unwrap();
    let mut media = store
        .import_media(
            &path,
            "image",
            Some(s(&asset, "id")),
            Some(s(&asset, "variantId")),
        )
        .unwrap();
    media["placeholder"] = json!(true);
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE media SET data=?1 WHERE id=?2",
            params![media.to_string(), s(&media, "id")],
        )
        .unwrap();
    assert_eq!(store.ensure_current_images().unwrap()["selectedCount"], 0);
    assert!(store
        .adopted(s(&asset, "id"), s(&asset, "variantId"))
        .unwrap()
        .is_none());
    assert_eq!(store.job(&job).unwrap(), frozen);
}
/// Explicit local acceptance only: paths arrive through environment, never fixtures.
#[test]
#[ignore]
fn full_private_import_acceptance() {
    let path = std::env::var("LINGMO_ACCEPTANCE_MD").expect("explicit MD path required");
    let report_path = std::env::var("LINGMO_ACCEPTANCE_REPORT").expect("private report required");
    let directory =
        std::env::var("LINGMO_ACCEPTANCE_DIR").expect("new isolated project directory required");
    assert!(
        !Path::new(&directory).exists(),
        "do not overwrite previous evidence"
    );
    let store = Store::open(Path::new(&directory).join("project")).unwrap();
    store
        .create("灵墨完整导入验收", "lingmo-acceptance")
        .unwrap();
    let text = read_text(Path::new(&path)).unwrap();
    let mut req = json!({"path":path,"expectedSha256":digest(&text)});
    let pre = preview(&store, &req).unwrap();
    req["previewToken"] = pre["previewToken"].clone();
    if let Ok(zip) = std::env::var("LINGMO_ACCEPTANCE_IMAGES") {
        req["imageZipPath"] = json!(zip);
    }
    let start = Instant::now();
    let result = import(&store, &req).unwrap();
    let write_ms = start.elapsed().as_secs_f64() * 1000.0;
    let docs = documents(&store).unwrap();
    let doc = &docs[0];
    for (key, count) in [
        ("episodes", 150),
        ("scenes", 371),
        ("fragments", 883),
        ("shots", 5407),
        ("mentionOccurrences", 4736),
        ("assets", 518),
    ] {
        assert_eq!(doc["counts"][key], count, "{key}");
    }
    let all = store.values("units").unwrap();
    assert_eq!(all.len(), 883);
    let mut shots = 0;
    let mut mentions = 0;
    for unit in &all {
        let a = unit["importSource"]["byteStart"].as_u64().unwrap() as usize;
        let b = unit["importSource"]["byteEnd"].as_u64().unwrap() as usize;
        assert_eq!(s(unit, "prompt"), &text[a..b]);
        shots += array(unit, "shots").len();
        mentions += array(unit, "mentions").len();
    }
    assert_eq!(shots, 5407);
    assert_eq!(mentions, 4736);
    let first = &all[0];
    let last = all.last().unwrap();
    assert_eq!(first["duration"], 24.0);
    assert_eq!(first["actualDurationSeconds"], 23.8);
    assert_eq!(last["duration"], 4.0);
    assert_eq!(last["actualDurationSeconds"], 2.7);
    assert_eq!(
        array(doc, "scenes")
            .iter()
            .filter(|v| v["number"] == "150-1")
            .count(),
        1
    );
    let start = Instant::now();
    let scene1 = scene(&store, s(doc, "id"), s(&doc["scenes"][0], "id")).unwrap();
    let first_scene_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert!(!array(&scene1, "units").is_empty());
    let start = Instant::now();
    let final_scene = scene(
        &store,
        s(doc, "id"),
        s(array(doc, "scenes").last().unwrap(), "id"),
    )
    .unwrap();
    let switch_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        array(&final_scene, "assets")
            .iter()
            .filter(|a| a["lingmoType"] == "SC")
            .count(),
        2
    );
    let mut duplicate = req.clone();
    duplicate.as_object_mut().unwrap().remove("imageZipPath");
    assert_eq!(import(&store, &duplicate).unwrap()["duplicate"], true);
    let reopened = Store::open(store.root.clone()).unwrap();
    assert_eq!(reopened.values("units").unwrap(), all);
    assert_eq!(
        original(&reopened, s(doc, "id"), None).unwrap()["text"],
        text
    );
    let export = Path::new(&directory).join("roundtrip.director");
    crate::portable::export(&store, &export).unwrap();
    let restored =
        crate::portable::import(&export, &Path::new(&directory).join("restored")).unwrap();
    assert_eq!(restored.values("units").unwrap(), all);
    assert_eq!(documents(&restored).unwrap(), docs);
    assert_eq!(
        restored.values("assets").unwrap(),
        store.values("assets").unwrap()
    );
    assert!(store.source().unwrap().is_null());
    assert!(store.job_summaries().unwrap().is_empty());
    assert!(restored.job_summaries().unwrap().is_empty());
    let report = json!({"status":"PASS","counts":doc["counts"],"sha256":digest(&text),"previewMs":pre["parseMilliseconds"],"importMs":write_ms,"firstSceneMs":first_scene_ms,"sceneSwitchMs":switch_ms,"result":result,"sourceUntouched":true,"originalBytesVerified":text.len(),"reopenAndExportImport":true,"generationSubmissions":0});
    fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{}", report);
}
