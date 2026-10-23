use super::*;
const MD:&str="**【标准化剧本】**\n第1集\n1-1\n场景：门口\n人物：甲\n片段1\n时长：4秒\n[场景] @[门口SC]\n[本单元出场人物] @[甲CH]\n分镜1｜0-2.7s｜2.7s\n画面：甲开门。\n";
fn fixture() -> (tempfile::TempDir, Arc<Engine>, Value) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new_offline_verification(dir.path().join("app")).unwrap();
    let path = dir.path().join("分镜样本.md");
    fs::write(&path, MD).unwrap();
    let preview = engine
        .dispatch(json!({"action":"lingmo.preview_file","path":path}))
        .unwrap();
    let request = json!({"action":"lingmo.create_project","path":path,"expectedSha256":preview["markdownSha256"],"previewToken":preview["previewToken"]});
    (dir, engine, request)
}
#[test]
fn global_preview_and_import_need_no_project_and_repeat_preserves_edits() {
    let (_dir, engine, request) = fixture();
    assert!(engine.project_list().unwrap().is_empty());
    assert_eq!(
        fs::read_dir(engine.home.join("projects")).unwrap().count(),
        0
    );
    let workspace = engine.dispatch(request.clone()).unwrap();
    assert_eq!(workspace["project"]["title"], "分镜样本");
    assert_eq!(workspace["lingmoImportReport"]["generationSubmissions"], 0);
    assert_eq!(workspace["lingmoDocuments"][0]["counts"]["fragments"], 1);
    let pid = s(&workspace["project"], "id");
    let store = engine.store(pid).unwrap();
    assert_eq!(store.root, engine.home.join("projects").join(pid));
    let source = store.source().unwrap();
    let old = store.values("units").unwrap().remove(0);
    let mut unit = old.clone();
    unit["expectedVersion"] = unit["version"].clone();
    unit["prompt"] = json!(s(&unit, "prompt").replace("甲开门。", "甲推门。"));
    let edited = crate::lingmo_import::save_unit(&store, &old, unit).unwrap();
    let repeated = engine.dispatch(request.clone()).unwrap();
    assert_eq!(repeated["project"]["id"], pid);
    assert_eq!(repeated["lingmoImportReport"]["duplicate"], true);
    assert_eq!(engine.project_list().unwrap().len(), 1);
    assert_eq!(store.get("units", s(&old, "id")).unwrap(), edited);
    assert_eq!(store.source().unwrap(), source);
    assert!(store.job_summaries().unwrap().is_empty());
    drop(engine);
    let reopened =
        Engine::new_offline_verification(store.root.parent().unwrap().parent().unwrap().to_owned())
            .unwrap();
    assert_eq!(reopened.dispatch(request).unwrap()["project"]["id"], pid);
    assert_eq!(reopened.project_list().unwrap().len(), 1);
}
#[test]
fn missing_preview_drift_and_invalid_structure_never_create_a_project() {
    let (_dir, engine, request) = fixture();
    let mut missing = request.clone();
    missing["previewToken"] = Value::Null;
    assert!(engine.dispatch(missing).is_err());
    fs::write(s(&request, "path"), MD.replace("甲开门。", "甲关门。")).unwrap();
    assert!(engine.dispatch(request.clone()).is_err());
    let invalid = format!("{MD}\n片段1\n分镜1｜0-1s｜1s\n画面：重复。\n");
    fs::write(s(&request, "path"), &invalid).unwrap();
    let pre = engine
        .dispatch(json!({"action":"lingmo.preview_file","path":request["path"]}))
        .unwrap();
    let mut req = request;
    req["expectedSha256"] = pre["markdownSha256"].clone();
    req["previewToken"] = pre["previewToken"].clone();
    assert!(engine.dispatch(req).is_err());
    assert!(engine.project_list().unwrap().is_empty());
    assert_eq!(
        fs::read_dir(engine.home.join("projects")).unwrap().count(),
        0
    );
}
#[test]
fn catalog_drift_is_rejected_before_creation() {
    let (_dir, engine, mut request) = fixture();
    let catalog = Path::new(s(&request, "path")).with_extension("json");
    fs::write(&catalog, "{}").unwrap();
    request["catalogPath"] = json!(catalog);
    let preview = engine
        .dispatch(
            json!({"action":"lingmo.preview_file","path":request["path"],"catalogPath":catalog}),
        )
        .unwrap();
    request["previewToken"] = preview["previewToken"].clone();
    fs::write(&catalog, "{\"note\":\"changed\"}").unwrap();
    assert!(engine.dispatch(request).is_err());
    assert_eq!(
        fs::read_dir(engine.home.join("projects")).unwrap().count(),
        0
    );
}
#[test]
fn archived_and_deleted_imports_are_not_silently_restored_or_duplicated() {
    let (_dir, engine, request) = fixture();
    let ws = engine.dispatch(request.clone()).unwrap();
    let pid = s(&ws["project"], "id");
    engine
        .dispatch(json!({"action":"projects.archive","projectId":pid,"expectedVersion":0}))
        .unwrap();
    assert!(engine
        .dispatch(request.clone())
        .unwrap_err()
        .contains("归档"));
    // Reproduce a process interruption after registry commit but before the
    // creation receipt's final state update, then respect explicit deletion.
    let marker_path = Path::new(s(&ws["project"], "rootPath")).join(".lingmo-create.json");
    let mut marker: Value = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    marker["state"] = json!("pending");
    atomic_json(&marker_path, &marker).unwrap();
    engine
        .dispatch(json!({"action":"projects.delete","projectId":pid,"expectedVersion":1}))
        .unwrap();
    assert!(engine.dispatch(request).unwrap_err().contains("已删除"));
    assert!(engine.project_list().unwrap().is_empty());
}
#[test]
fn interrupted_unregistered_import_recovers_same_project_only() {
    let (_dir, engine, request) = fixture();
    let ws = engine.dispatch(request.clone()).unwrap();
    let store = engine.store(s(&ws["project"], "id")).unwrap();
    let marker_path = store.root.join(".lingmo-create.json");
    let mut marker: Value = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    marker["state"] = json!("pending");
    atomic_json(&marker_path, &marker).unwrap();
    atomic_json(&engine.home.join("projects.json"), &json!([])).unwrap();
    let recovered = engine.dispatch(request).unwrap();
    assert_eq!(recovered["project"]["id"], ws["project"]["id"]);
    assert_eq!(engine.project_list().unwrap().len(), 1);
    assert_eq!(store.values("units").unwrap().len(), 1);
    assert!(store.job_summaries().unwrap().is_empty());
}
#[test]
fn concurrent_clicks_and_existing_directory_cannot_duplicate_or_overwrite() {
    let (_dir, engine, request) = fixture();
    let second_client = Engine::new_offline_verification(engine.home.clone()).unwrap();
    let threads = (0..2)
        .map(|index| {
            let e = if index == 0 {
                engine.clone()
            } else {
                second_client.clone()
            };
            let r = request.clone();
            std::thread::spawn(move || e.dispatch(r).unwrap())
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results[0]["project"]["id"], results[1]["project"]["id"]);
    assert_eq!(engine.project_list().unwrap().len(), 1);
    let (_dir, engine, request) = fixture();
    let preview = crate::lingmo_import::validate_project_input(&request).unwrap();
    let identity = pipeline::digest(&format!(
        "lingmo-project/v1:{}:{}",
        s(&preview, "markdownSha256"),
        s(&preview, "previewToken")
    ));
    let root = engine
        .home
        .join("projects")
        .join(format!("lingmo-{identity}"));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("keep.txt"), "keep").unwrap();
    assert!(engine.dispatch(request).is_err());
    assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "keep");
    assert!(!root.join("project.sqlite").exists());
}

#[test]
fn setup_failure_removes_only_new_directory_and_retry_can_succeed() {
    let (_dir, engine, request) = fixture();
    let preserved = engine
        .dispatch(json!({"action":"projects.create","title":"已有项目"}))
        .unwrap();
    let preserved_store = engine.store(s(&preserved, "id")).unwrap();
    let preserved_source = preserved_store.source().unwrap();
    fs::write(
        engine.home.join(crate::executable_paths::FILE_NAME),
        "broken defaults",
    )
    .unwrap();
    assert!(engine
        .dispatch(request.clone())
        .unwrap_err()
        .contains("默认值无效"));
    assert_eq!(engine.project_list().unwrap().len(), 1);
    assert_eq!(
        fs::read_dir(engine.home.join("projects")).unwrap().count(),
        1
    );
    assert_eq!(preserved_store.source().unwrap(), preserved_source);
    fs::write(
        engine.home.join(crate::executable_paths::FILE_NAME),
        "{\"version\":1,\"paths\":{}}",
    )
    .unwrap();
    let ws = engine.dispatch(request).unwrap();
    assert_ne!(ws["project"]["id"], preserved["id"]);
    assert_eq!(engine.project_list().unwrap().len(), 2);
    assert!(ws["jobs"].as_array().unwrap().is_empty());
}

#[test]
fn repeated_global_import_adds_directory_or_zip_media_without_overwriting_edits() {
    use std::io::Write;
    for use_zip in [false, true] {
        let (dir, engine, mut request) = fixture();
        let catalog = dir.path().join("assets.json");
        fs::write(&catalog,json!({"markdownSha256":pipeline::digest(MD),"assets":[
            {"id":"scene-one","name":"门口","type":"SC","token":"门口SC","filename":"scene.png"},
            {"id":"character-one","name":"甲","type":"CH","token":"甲CH"}
        ]}).to_string()).unwrap();
        request["catalogPath"] = json!(catalog);
        let preview=engine.dispatch(json!({"action":"lingmo.preview_file","path":request["path"],"catalogPath":catalog})).unwrap();
        request["previewToken"] = preview["previewToken"].clone();
        let first = engine.dispatch(request.clone()).unwrap();
        let pid = s(&first["project"], "id");
        let store = engine.store(pid).unwrap();
        assert!(store.values("media").unwrap().is_empty());
        let old = store.values("units").unwrap().remove(0);
        let mut edit = old.clone();
        edit["expectedVersion"] = edit["version"].clone();
        edit["prompt"] = json!(s(&edit, "prompt").replace("甲开门。", "甲推门。"));
        let edited = crate::lingmo_import::save_unit(&store, &old, edit).unwrap();
        let images = dir.path().join("images");
        fs::create_dir(&images).unwrap();
        image::RgbImage::from_pixel(4, 3, image::Rgb([12, 34, 56]))
            .save(images.join("scene.png"))
            .unwrap();
        if use_zip {
            let path = dir.path().join("images.zip");
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            zip.start_file("scene.png", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&fs::read(images.join("scene.png")).unwrap())
                .unwrap();
            zip.finish().unwrap();
            request["imageZipPath"] = json!(path);
        } else {
            request["imageDirectory"] = json!(images);
        }
        let supplemented = engine.dispatch(request.clone()).unwrap();
        assert_eq!(supplemented["project"]["id"], pid);
        assert_eq!(supplemented["lingmoImportReport"]["media"]["imported"], 1);
        assert_eq!(supplemented["lingmoImportReport"]["projectReused"], true);
        assert_eq!(supplemented["lingmoImportReport"]["projectCreated"], false);
        assert_eq!(store.values("media").unwrap().len(), 1);
        assert_eq!(store.get("units", s(&old, "id")).unwrap(), edited);
        let retried = engine.dispatch(request.clone()).unwrap();
        assert_eq!(retried["lingmoImportReport"]["media"]["imported"], 1);
        assert_eq!(store.values("media").unwrap().len(), 1);
        assert_eq!(engine.project_list().unwrap().len(), 1);
        assert!(store.job_summaries().unwrap().is_empty());
        // Failed supplementary media must replace the stale success report,
        // while the imported text and all previous successful images survive.
        let broken = dir.path().join("broken.zip");
        fs::write(&broken, b"not a zip archive").unwrap();
        request["imageDirectory"] = Value::Null;
        request["imageZipPath"] = json!(broken);
        let failed = engine.dispatch(request.clone()).unwrap();
        assert_eq!(
            failed["lingmoImportReport"]["media"]["failures"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            failed["lingmoImportReport"]["media"]["textImportPreserved"],
            true
        );
        assert_eq!(store.get("units", s(&old, "id")).unwrap(), edited);
        assert_eq!(store.values("media").unwrap().len(), 1);
        request["imageZipPath"] = Value::Null;
        let reused = engine.dispatch(request).unwrap();
        assert_eq!(
            reused["lingmoImportReport"]["media"],
            failed["lingmoImportReport"]["media"]
        );
        assert_eq!(engine.project_list().unwrap().len(), 1);
    }
}
