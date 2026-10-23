use super::*;
const TEXT:&str="第1集\n1-1\n场景：门口\n人物：甲、甲受伤\n道具：甲\n声音：风声\n片段1\n时长：4秒\n@[门口SC] @[甲CH] @[甲PR] @[甲受伤CH] @[风声SE]\n分镜1｜0-2.7s｜2.7s\n画面：甲开门。\n";
fn setup() -> (tempfile::TempDir, Store, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("project")).unwrap();
    store.create("本地补图验收", "folder-test").unwrap();
    let doc = digest(TEXT);
    crate::lingmo_import::import(&store, &json!({"text":TEXT,"expectedSha256":doc})).unwrap();
    let media = dir.path().join("files");
    fs::create_dir(&media).unwrap();
    (dir, store, media, doc)
}
fn image(path: &Path, color: u8) {
    image::RgbImage::from_pixel(8, 8, image::Rgb([color, color, color]))
        .save(path)
        .unwrap();
}
fn asset(store: &Store, name: &str, ty: &str) -> Value {
    store
        .values("assets")
        .unwrap()
        .into_iter()
        .find(|a| s(a, "name") == name && s(a, "lingmoType") == ty)
        .unwrap()
}
fn preview_folder(store: &Store, path: &Path, doc: &str) -> Value {
    preview(store, &json!({"documentId":doc,"directory":path})).unwrap()
}
fn select(pre: &Value, name: &str, adopt: bool) -> Value {
    let row = values(pre, "rows")
        .into_iter()
        .find(|r| s(r, "filename") == name)
        .unwrap();
    json!({"rowId":row["id"],"assetId":row["assetId"],"variantId":row["variantId"],"adopt":adopt,"expectedAdoptedMediaId":row["expectedAdoptedMediaId"]})
}
fn commit_selections(store: &Store, pre: &Value, selections: Vec<Value>) -> Value {
    commit(store,&json!({"documentId":pre["documentId"],"previewToken":pre["previewToken"],"selections":selections})).unwrap()
}

#[test]
fn typed_exact_matching_no_substrings_or_cross_revision_identity() {
    let (_dir, store, folder, doc) = setup();
    for name in [
        "角色-甲.png",
        "道具-甲.png",
        "甲受伤CH.png",
        "甲.png",
        "角色-甲额外.png",
        "风声SE.png",
    ] {
        image(&folder.join(name), 17);
    }
    fs::write(folder.join("readme.txt"), "not-media").unwrap();
    let text = TEXT.replace("甲开门。", "甲关门。");
    crate::lingmo_import::import(
        &store,
        &json!({"text":text,"expectedSha256":digest(&text),"allowNewRevision":true}),
    )
    .unwrap();
    let pre = preview_folder(&store, &folder, &doc);
    assert_eq!(
        pre["counts"],
        json!({"files":7,"matched":3,"unmatched":3,"ambiguous":0,"unsupported":1})
    );
    for row in values(&pre, "rows")
        .into_iter()
        .filter(|r| s(r, "status") == "matched")
    {
        assert!(s(&row, "assetId").starts_with(&format!("lingmo-{doc}-asset-")));
    }
    assert!(store.values("media").unwrap().is_empty());
    assert!(store.job_summaries().unwrap().is_empty());
}
#[test]
fn duplicate_target_files_require_selection_and_do_not_pick_arbitrarily() {
    let (_dir, store, folder, doc) = setup();
    image(&folder.join("人物-甲.png"), 17);
    image(&folder.join("甲CH.png"), 18);
    let pre = preview_folder(&store, &folder, &doc);
    assert_eq!(pre["counts"]["ambiguous"], 2);
    assert!(values(&pre, "rows").iter().all(|r| r["assetId"].is_null()));
    let a = asset(&store, "甲", "CH");
    let rows = values(&pre, "rows");
    let selections = rows
        .iter()
        .map(|r| json!({"rowId":r["id"],"assetId":a["id"],"variantId":a["variantId"]}))
        .collect();
    let result = commit_selections(&store, &pre, selections);
    assert_eq!(result["report"]["failed"], 2);
    assert!(store.values("media").unwrap().is_empty());
    let result = commit_selections(
        &store,
        &pre,
        vec![json!({"rowId":rows[0]["id"],"assetId":a["id"],"variantId":a["variantId"]})],
    );
    assert_eq!(result["report"]["imported"], 1);
}
#[test]
fn candidates_do_not_auto_adopt_even_after_current_is_cleared_and_shared_media_remains_usable() {
    let (_dir, store, folder, doc) = setup();
    let a = asset(&store, "甲", "CH");
    let b = asset(&store, "甲", "PR");
    let one = folder.join("old.png");
    image(&one, 10);
    let old = store
        .import_image_as_current(&one, s(&a, "id"), None)
        .unwrap();
    image(&folder.join("角色-甲.png"), 12);
    let pre = preview_folder(&store, &folder, &doc);
    let result = commit_selections(&store, &pre, vec![select(&pre, "角色-甲.png", false)]);
    assert_eq!(result["report"]["imported"], 1);
    let mid = s(&result["report"]["rows"][0], "mediaId");
    store.ensure_current_images().unwrap();
    assert_eq!(
        store.adopted(s(&a, "id"), s(&a, "variantId")).unwrap(),
        Some(s(&old, "id").into())
    );
    // An ordinary link to the exact same bytes on another identity is unaffected.
    store
        .link_media(mid, s(&b, "id"), s(&b, "variantId"))
        .unwrap();
    store
        .conn()
        .unwrap()
        .execute("DELETE FROM adoptions WHERE asset_id=?1", [s(&a, "id")])
        .unwrap();
    // Remove old ordinary history to prove the candidate itself is not selected.
    store
        .conn()
        .unwrap()
        .execute(
            "DELETE FROM media_assets WHERE asset_id=?1 AND media_id=?2",
            params![s(&a, "id"), s(&old, "id")],
        )
        .unwrap();
    store.ensure_current_images().unwrap();
    assert_eq!(
        store.adopted(s(&a, "id"), s(&a, "variantId")).unwrap(),
        None
    );
    assert_eq!(
        store.adopted(s(&b, "id"), s(&b, "variantId")).unwrap(),
        Some(mid.into())
    );
}
#[test]
fn explicit_adoption_compare_and_swap_preserves_newer_choice_and_unknown_snapshot() {
    let (_dir, store, folder, doc) = setup();
    let a = asset(&store, "甲", "CH");
    image(&folder.join("角色-甲.png"), 21);
    let pre = preview_folder(&store, &folder, &doc);
    let other = folder.join("other.png");
    image(&other, 22);
    let old = store
        .import_image_as_current(&other, s(&a, "id"), None)
        .unwrap();
    let frozen = json!({"prompt":"冻结原文","references":[{"assetId":a["id"],"variantId":a["variantId"],"mediaId":old["id"]}]});
    let job = store
        .add_job("video", "unit", "mock", "account", &frozen, "unknown")
        .unwrap();
    let job_before = store.job(&job).unwrap();
    let result = commit_selections(&store, &pre, vec![select(&pre, "角色-甲.png", true)]);
    assert_eq!(result["report"]["failed"], 1);
    assert_eq!(result["report"]["rows"][0]["status"], "adoption_conflict");
    assert_eq!(
        store.adopted(s(&a, "id"), s(&a, "variantId")).unwrap(),
        Some(s(&old, "id").into())
    );
    assert_eq!(store.job(&job).unwrap(), job_before);
    let mut selection = select(&pre, "角色-甲.png", true);
    selection["expectedAdoptedMediaId"] = old["id"].clone();
    let result = commit_selections(&store, &pre, vec![selection.clone()]);
    assert_eq!(result["report"]["adopted"], 1);
    let repeated = commit_selections(&store, &pre, vec![selection]);
    assert_eq!(repeated["report"]["skipped"], 1);
    assert_eq!(store.values("media").unwrap().len(), 2);
    assert_eq!(store.job(&job).unwrap(), job_before);
}
#[test]
fn white_placeholder_deduplicates_and_real_replacement_ignores_asset_static_label() {
    let (_dir, store, folder, doc) = setup();
    let mut a = asset(&store, "甲", "CH");
    a["placeholder"] = json!(true);
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE assets SET data=?1 WHERE id=?2",
            params![a.to_string(), s(&a, "id")],
        )
        .unwrap();
    image(&folder.join("角色-甲.png"), 255);
    let pre = preview_folder(&store, &folder, &doc);
    let result = commit_selections(&store, &pre, vec![select(&pre, "角色-甲.png", true)]);
    assert_eq!(result["report"]["placeholders"], 1);
    assert_eq!(result["report"]["adopted"], 0);
    store.ensure_current_images().unwrap();
    assert_eq!(
        store.adopted(s(&a, "id"), s(&a, "variantId")).unwrap(),
        None
    );
    let again = preview_folder(&store, &folder, &doc);
    commit_selections(&store, &again, vec![select(&again, "角色-甲.png", false)]);
    assert_eq!(store.values("media").unwrap().len(), 1);
    image(&folder.join("角色-甲.png"), 45);
    let real = preview_folder(&store, &folder, &doc);
    let result = commit_selections(&store, &real, vec![select(&real, "角色-甲.png", true)]);
    assert_eq!(result["report"]["adopted"], 1);
    assert_eq!(result["report"]["placeholders"], 0);
    assert_eq!(
        store
            .values("assets")
            .unwrap()
            .into_iter()
            .find(|v| v["id"] == a["id"])
            .unwrap()["placeholder"],
        true
    );
}
#[test]
fn per_file_drift_and_invalid_image_preserve_successful_files_and_all_source_units() {
    let (_dir, store, folder, doc) = setup();
    let units = store.values("units").unwrap();
    let source = store.source().unwrap();
    image(&folder.join("角色-甲.png"), 51);
    image(&folder.join("道具-甲.png"), 52);
    fs::write(folder.join("场景-门口.png"), b"invalid image").unwrap();
    let pre = preview_folder(&store, &folder, &doc);
    image(&folder.join("道具-甲.png"), 53);
    let result = commit_selections(
        &store,
        &pre,
        vec![
            select(&pre, "角色-甲.png", false),
            select(&pre, "道具-甲.png", false),
            select(&pre, "场景-门口.png", false),
        ],
    );
    assert_eq!(result["report"]["imported"], 1);
    assert_eq!(result["report"]["failed"], 2);
    assert_eq!(store.values("units").unwrap(), units);
    assert_eq!(store.source().unwrap(), source);
    assert!(store.job_summaries().unwrap().is_empty());
    let reopened = Store::open(store.root.clone()).unwrap();
    let result = commit_selections(&reopened, &pre, vec![select(&pre, "角色-甲.png", false)]);
    assert_eq!(result["report"]["skipped"], 1);
    assert_eq!(reopened.values("media").unwrap().len(), 1);
}
#[test]
fn manual_binding_is_exact_document_type_and_variant_bound() {
    let (_dir, store, folder, doc) = setup();
    image(&folder.join("arbitrary.png"), 60);
    let pre = preview_folder(&store, &folder, &doc);
    let row = &pre["rows"][0];
    let a = asset(&store, "甲", "CH");
    let audio = asset(&store, "风声", "SE");
    let result = commit_selections(
        &store,
        &pre,
        vec![json!({"rowId":row["id"],"assetId":audio["id"]})],
    );
    assert_eq!(result["report"]["failed"], 1);
    let result = commit_selections(
        &store,
        &pre,
        vec![json!({"rowId":row["id"],"assetId":a["id"],"variantId":"wrong"})],
    );
    assert_eq!(result["report"]["failed"], 1);
    let text = TEXT.replace("甲开门。", "甲转身。");
    crate::lingmo_import::import(
        &store,
        &json!({"text":text,"expectedSha256":digest(&text),"allowNewRevision":true}),
    )
    .unwrap();
    let outsider = store
        .values("assets")
        .unwrap()
        .into_iter()
        .find(|v| !s(v, "id").starts_with(&format!("lingmo-{doc}-")))
        .unwrap();
    let result = commit_selections(
        &store,
        &pre,
        vec![json!({"rowId":row["id"],"assetId":outsider["id"]})],
    );
    assert_eq!(result["report"]["failed"], 1);
    let result = commit_selections(
        &store,
        &pre,
        vec![json!({"rowId":row["id"],"assetId":a["id"]})],
    );
    assert_eq!(result["report"]["imported"], 1);
}
#[test]
fn catalog_exact_relative_name_takes_precedence_over_typename() {
    let (_dir, store, folder, doc) = setup();
    let mut a = asset(&store, "甲", "CH");
    a["filename"] = json!("nested/场景-门口.png");
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE assets SET data=?1 WHERE id=?2",
            params![a.to_string(), s(&a, "id")],
        )
        .unwrap();
    fs::create_dir(folder.join("nested")).unwrap();
    image(&folder.join("nested/场景-门口.png"), 71);
    let pre = preview_folder(&store, &folder, &doc);
    assert_eq!(pre["rows"][0]["assetId"], a["id"]);
}
#[test]
fn changed_asset_version_and_forged_preview_fail_closed() {
    let (_dir, store, folder, doc) = setup();
    image(&folder.join("角色-甲.png"), 78);
    let pre = preview_folder(&store, &folder, &doc);
    let a = asset(&store, "甲", "CH");
    store.conn().unwrap().execute("UPDATE assets SET data=json_set(data,'$.version',version+1),version=version+1 WHERE id=?1",[s(&a,"id")]).unwrap();
    let result = commit_selections(&store, &pre, vec![select(&pre, "角色-甲.png", false)]);
    assert_eq!(result["report"]["failed"], 1);
    assert!(store.values("media").unwrap().is_empty());
    assert!(commit(
        &store,
        &json!({"documentId":"wrong","previewToken":pre["previewToken"],"selections":[]})
    )
    .is_err());
    assert!(commit(
        &store,
        &json!({"documentId":doc,"previewToken":"forged","selections":[]})
    )
    .is_err());
}
#[test]
fn explicit_paths_and_safe_unsupported_files_do_not_read_external_links() {
    let (_dir, store, folder, doc) = setup();
    let file = folder.join("角色-甲.png");
    image(&file, 81);
    let pre = preview(&store, &json!({"documentId":doc,"paths":[file,file]})).unwrap();
    assert_eq!(pre["counts"]["files"], 1);
    assert_eq!(pre["counts"]["matched"], 1);
    #[cfg(windows)]
    {
        // Junctions are available without developer-mode symlink privileges.
        let outside = folder.parent().unwrap().join("outside");
        fs::create_dir(&outside).unwrap();
        image(&outside.join("场景-门口.png"), 90);
        let junction = folder.join("external");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let pre = preview_folder(&store, &folder, &doc);
        assert_eq!(pre["counts"]["files"], 2);
        assert_eq!(pre["counts"]["unsupported"], 1);
        // Do not let TempDir deletion traverse the junction.
        fs::remove_dir(junction).unwrap();
    }
}

#[test]
fn historical_catalog_placeholder_flag_is_reclassified_from_real_pixels() {
    let (_dir, store, folder, doc) = setup();
    let file = folder.join("角色-甲.png");
    image(&file, 99);
    let mut old = store.import_media(&file, "image", None, None).unwrap();
    old["placeholder"] = json!(true);
    old.as_object_mut().unwrap().remove("placeholderSource");
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE media SET data=?1 WHERE id=?2",
            params![old.to_string(), s(&old, "id")],
        )
        .unwrap();
    let pre = preview_folder(&store, &folder, &doc);
    let result = commit_selections(&store, &pre, vec![select(&pre, "角色-甲.png", true)]);
    assert_eq!(result["report"]["adopted"], 1);
    assert_eq!(result["report"]["placeholders"], 0);
    assert_eq!(store.values("media").unwrap().len(), 1);
    assert_eq!(
        store.get("media", s(&old, "id")).unwrap()["placeholderSource"],
        "image-pixels/v1"
    );
}

#[test]
fn fully_transparent_black_png_is_placeholder_not_production_art() {
    let (_dir, store, folder, doc) = setup();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]))
        .save(folder.join("角色-甲.png"))
        .unwrap();
    let pre = preview_folder(&store, &folder, &doc);
    let result = commit_selections(&store, &pre, vec![select(&pre, "角色-甲.png", true)]);
    assert_eq!(result["report"]["placeholders"], 1);
    assert_eq!(result["report"]["adopted"], 0);
    store.ensure_current_images().unwrap();
    let a = asset(&store, "甲", "CH");
    assert_eq!(
        store.adopted(s(&a, "id"), s(&a, "variantId")).unwrap(),
        None
    );
    assert_eq!(
        store.values("media").unwrap()[0]["placeholderSource"],
        "image-pixels/v1"
    );
}

#[test]
fn concurrent_identical_commit_has_one_import_and_one_replay_receipt() {
    let (_dir, store, folder, doc) = setup();
    image(&folder.join("角色-甲.png"), 100);
    let pre = preview_folder(&store, &folder, &doc);
    let selection = select(&pre, "角色-甲.png", true);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let handles = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let pre = pre.clone();
                let selection = selection.clone();
                let store = store.clone();
                scope.spawn(move || {
                    barrier.wait();
                    commit_selections(&store, &pre, vec![selection])
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results
            .iter()
            .map(|r| r["report"]["imported"].as_i64().unwrap())
            .sum::<i64>(),
        1
    );
    assert_eq!(
        results
            .iter()
            .map(|r| r["report"]["skipped"].as_i64().unwrap())
            .sum::<i64>(),
        1
    );
    assert_eq!(store.values("media").unwrap().len(), 1);
    assert!(store.job_summaries().unwrap().is_empty());
}

#[test]
#[ignore = "Explicit installed ffprobe; local WAV backfill only, no generation"]
fn installed_ffprobe_audio_folder_backfill_preserves_audio_type_and_source() {
    let (_dir, store, folder, doc) = setup();
    if let Some(path) = std::env::var_os("LINGMO_MEDIA_FFPROBE") {
        let mut settings = store.settings().unwrap();
        settings["ffprobePath"] = json!(path.to_str().unwrap());
        store.save_settings(&settings).unwrap();
    }
    let samples = 16000u32;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + samples * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(32000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((samples * 2).to_le_bytes());
    wav.extend(vec![0u8; (samples * 2) as usize]);
    fs::write(folder.join("声音-风声.wav"), &wav).unwrap();
    let units = store.values("units").unwrap();
    let pre = preview_folder(&store, &folder, &doc);
    assert_eq!(pre["counts"]["matched"], 1);
    let selection = select(&pre, "声音-风声.wav", true);
    let result = commit_selections(&store, &pre, vec![selection.clone()]);
    assert_eq!(result["report"]["imported"], 1, "{result}");
    assert_eq!(result["report"]["adopted"], 0);
    assert_eq!(result["report"]["failed"], 0);
    let media = store.values("media").unwrap().remove(0);
    assert_eq!(media["kind"], "audio");
    assert_eq!(media["metadataVerified"], true);
    assert_eq!(media["durationSeconds"], 1.0);
    let a = asset(&store, "风声", "SE");
    assert_eq!(
        store.adopted(s(&a, "id"), s(&a, "variantId")).unwrap(),
        None
    );
    assert_eq!(store.values("units").unwrap(), units);
    assert!(store.job_summaries().unwrap().is_empty());
    // Recover an old deduplicated file whose first attempt preceded component
    // installation. It must not remain permanently stuck on old probe failure.
    let mut legacy = media.clone();
    legacy["metadataVerified"] = json!(false);
    legacy["metadataError"] = json!("ffprobe previously missing");
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE media SET data=?1 WHERE id=?2",
            params![legacy.to_string(), s(&legacy, "id")],
        )
        .unwrap();
    let recovery = preview_folder(&store, &folder, &doc);
    let recovered = commit_selections(
        &store,
        &recovery,
        vec![select(&recovery, "声音-风声.wav", false)],
    );
    assert_eq!(recovered["report"]["imported"], 1, "{recovered}");
    assert_eq!(recovered["report"]["reused"], 1);
    assert_eq!(
        store.get("media", s(&media, "id")).unwrap()["metadataVerified"],
        true
    );
    let repeated = commit_selections(&store, &pre, vec![selection]);
    assert_eq!(repeated["report"]["skipped"], 1);
    fs::write(folder.join("声音-风声.wav"), b"not a media container").unwrap();
    let invalid = preview_folder(&store, &folder, &doc);
    let result = commit_selections(
        &store,
        &invalid,
        vec![select(&invalid, "声音-风声.wav", false)],
    );
    assert_eq!(result["report"]["failed"], 1);
    assert_eq!(store.values("units").unwrap(), units);
}
