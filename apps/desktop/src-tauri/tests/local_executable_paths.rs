use director_core::{
    executable_paths::{self, FILE_NAME, PATH_KEYS},
    portable,
    store::{s, Store},
    Engine,
};
use rusqlite::params;
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

fn paths(root: &Path) -> Value {
    json!({
        "codexPath":root.join("codex.exe"),
        "dreaminaPath":root.join("dreamina.exe"),
        "dreaminaCanvasPath":root.join("dreamina-canvas.exe"),
        "textSdkNodePath":root.join("node.exe"),
    })
}

fn raw_snapshot(store: &Store, job: &str) -> String {
    store
        .conn()
        .unwrap()
        .query_row("SELECT snapshot FROM jobs WHERE id=?1", [job], |row| {
            row.get(0)
        })
        .unwrap()
}

fn assert_paths(settings: &Value, expected: &Value) {
    for key in PATH_KEYS {
        assert_eq!(settings[key], expected[key], "{key}");
    }
}

#[test]
fn new_project_reuses_only_four_explicitly_saved_machine_paths() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let engine = Engine::new_offline_verification(home.clone()).unwrap();
    let first = engine
        .dispatch(json!({"action":"projects.create","title":"项目一"}))
        .unwrap();
    let mut saved = paths(&dir.path().join("installed"));
    saved["textModel"] = json!("project-one-model");
    saved["textProvider"] = json!("codex_sdk");
    saved["visualStyle"] = json!("仅本项目画风");
    saved["accountId"] = json!("synthetic-project-account");
    engine
        .dispatch(json!({"action":"settings.save","projectId":first["id"],"settings":saved}))
        .unwrap();
    engine.dispatch(json!({"action":"source.import","projectId":first["id"],"text":"第1集\n场景：海边\n甲：原文只属于项目一。\n"})).unwrap();
    let second = engine
        .dispatch(json!({"action":"projects.create","title":"项目二"}))
        .unwrap();
    let second_store = engine.store(s(&second, "id")).unwrap();
    let actual = second_store.settings().unwrap();
    assert_paths(&actual, &saved);
    assert_eq!(actual["textModel"], "gpt-6.1-sol");
    assert_eq!(actual["textProvider"], "codex");
    assert!(actual["visualStyle"].is_null());
    assert!(actual["accountId"].is_null());
    assert!(actual["textApiKey"].is_null());
    assert!(second_store.source().unwrap()["text"].is_null());
    assert!(second_store.jobs().unwrap().is_empty());
    let defaults: Value = serde_json::from_slice(&fs::read(home.join(FILE_NAME)).unwrap()).unwrap();
    assert_eq!(defaults["paths"].as_object().unwrap().len(), 4);
    assert_eq!(defaults.as_object().unwrap().len(), 2);
}

#[test]
fn saved_paths_survive_restart_without_switching_existing_project_or_frozen_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let engine = Engine::new_offline_verification(home.clone()).unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"原有项目"}))
        .unwrap();
    let store = engine.store(s(&project, "id")).unwrap();
    let original_paths = paths(&dir.path().join("original-tools"));
    engine
        .dispatch(
            json!({"action":"settings.save","projectId":project["id"],"settings":original_paths}),
        )
        .unwrap();
    let frozen = json!({"providerSettings":original_paths,"request":{"stage":"assetDesign"},"raw":"合成原回包"});
    let job = store
        .add_job(
            "image",
            "synthetic-asset",
            "dreamina",
            "original-account",
            &frozen,
            "submission_unknown",
        )
        .unwrap();
    let bytes = format!("\n {}\n", raw_snapshot(&store, &job));
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
            params![bytes, job],
        )
        .unwrap();
    let other = engine
        .dispatch(json!({"action":"projects.create","title":"另一项目"}))
        .unwrap();
    let new_paths = paths(&dir.path().join("new-tools"));
    engine
        .dispatch(json!({"action":"settings.save","projectId":other["id"],"settings":new_paths}))
        .unwrap();
    drop(engine);
    let reopened = Engine::new_offline_verification(home).unwrap();
    assert_paths(
        &reopened
            .store(s(&project, "id"))
            .unwrap()
            .settings()
            .unwrap(),
        &original_paths,
    );
    assert_eq!(raw_snapshot(&store, &job), bytes);
    assert_eq!(store.job(&job).unwrap()["status"], "submission_unknown");
    let next = reopened
        .dispatch(json!({"action":"projects.create","title":"重开后新项目"}))
        .unwrap();
    assert_paths(
        &reopened.store(s(&next, "id")).unwrap().settings().unwrap(),
        &new_paths,
    );
}

#[test]
fn empty_partial_or_unrelated_saves_do_not_clear_defaults_or_import_private_fields() {
    let dir = tempfile::tempdir().unwrap();
    let original = paths(&dir.path().join("tools"));
    executable_paths::remember(dir.path(), &original).unwrap();
    let new_codex = dir.path().join("other-tools/codex.exe");
    executable_paths::remember(
        dir.path(),
        &json!({
            "codexPath":new_codex,"dreaminaPath":"","dreaminaCanvasPath":null,
            "textSdkNodePath":"node","accountId":"synthetic-account","textModel":"private-model",
            "textApiKey":"synthetic-secret","bhToken":"synthetic-secret",
        }),
    )
    .unwrap();
    let actual = executable_paths::load(dir.path()).unwrap();
    assert_eq!(actual["codexPath"], json!(new_codex));
    for key in ["dreaminaPath", "dreaminaCanvasPath", "textSdkNodePath"] {
        assert_eq!(actual[key], original[key]);
    }
    assert_eq!(actual.as_object().unwrap().len(), 4);
    let bytes = fs::read(dir.path().join(FILE_NAME)).unwrap();
    executable_paths::remember(
        dir.path(),
        &json!({"textModel":"another-model","codexPath":null}),
    )
    .unwrap();
    assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap(), bytes);
}

#[test]
fn opening_existing_projects_does_not_seed_defaults_or_replace_an_explicit_path() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let engine = Engine::new_offline_verification(home.clone()).unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"旧项目"}))
        .unwrap();
    let store = engine.store(s(&project, "id")).unwrap();
    let old_paths = paths(&dir.path().join("older-machine"));
    store.save_settings(&old_paths).unwrap();
    drop(engine);
    let reopened = Engine::new_offline_verification(home.clone()).unwrap();
    reopened
        .dispatch(json!({"action":"workspace.get","projectId":project["id"]}))
        .unwrap();
    let fresh = reopened
        .dispatch(json!({"action":"projects.create","title":"尚未保存本机连接"}))
        .unwrap();
    let fresh_settings = reopened.store(s(&fresh, "id")).unwrap().settings().unwrap();
    for key in PATH_KEYS {
        assert!(fresh_settings[key].is_null(), "{key}");
    }
    assert!(!home.join(FILE_NAME).exists());
    executable_paths::remember(&home, &paths(&dir.path().join("local-tools"))).unwrap();
    executable_paths::apply(&home, &store).unwrap();
    assert_paths(&store.settings().unwrap(), &old_paths);
}

fn old_package(store: &Store, file: &Path, snapshot: &Path) {
    // Deliberately model a pre-fix package that carries foreign active paths.
    store.backup_db(snapshot).unwrap();
    let mut writer = ZipWriter::new(fs::File::create(file).unwrap());
    writer
        .start_file("project.sqlite", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&fs::read(snapshot).unwrap()).unwrap();
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer
        .write_all(
            json!({"format":"director-project-v1","schemaVersion":2,"mediaCount":0})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    writer.finish().unwrap();
}

#[test]
fn importing_an_old_package_uses_local_paths_without_poisoning_defaults_or_frozen_history() {
    let dir = tempfile::tempdir().unwrap();
    let foreign = Store::open(dir.path().join("foreign-project")).unwrap();
    foreign.create("待导入项目", "foreign-project").unwrap();
    let foreign_paths = paths(&dir.path().join("foreign-tools"));
    let mut foreign_settings = foreign_paths.clone();
    foreign_settings["textModel"] = json!("imported-project-model");
    foreign.save_settings(&foreign_settings).unwrap();
    let job = foreign
        .add_job(
            "image",
            "synthetic-asset",
            "dreamina",
            "frozen-account",
            &json!({"providerSettings":foreign_paths,"receipt":{"taskId":"original-cloud-task"}}),
            "submission_unknown",
        )
        .unwrap();
    let frozen = format!("\n {}\n", raw_snapshot(&foreign, &job));
    foreign
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
            params![frozen, job],
        )
        .unwrap();
    let package = dir.path().join("old.director.zip");
    old_package(&foreign, &package, &dir.path().join("old-snapshot.sqlite"));
    let home = dir.path().join("local-home");
    let local = paths(&dir.path().join("local-tools"));
    executable_paths::remember(&home, &local).unwrap();
    let defaults_before = fs::read(home.join(FILE_NAME)).unwrap();
    let engine = Engine::new_offline_verification(home.clone()).unwrap();
    let workspace = engine
        .dispatch(json!({"action":"project.import","path":package}))
        .unwrap();
    let imported = engine.store(s(&workspace["project"], "id")).unwrap();
    assert_paths(&workspace["settings"], &local);
    assert_eq!(workspace["settings"]["textModel"], "imported-project-model");
    assert_eq!(raw_snapshot(&imported, &job), frozen);
    assert_eq!(imported.job(&job).unwrap()["status"], "submission_unknown");
    assert_eq!(fs::read(home.join(FILE_NAME)).unwrap(), defaults_before);
    assert_paths(&foreign.settings().unwrap(), &foreign_paths);
    let cold_home = dir.path().join("machine-without-defaults");
    let cold = Engine::new_offline_verification(cold_home.clone()).unwrap();
    let cold_workspace = cold
        .dispatch(json!({"action":"project.import","path":package}))
        .unwrap();
    for key in PATH_KEYS {
        assert!(cold_workspace["settings"][key].is_null(), "{key}");
    }
    assert!(!cold_home.join(FILE_NAME).exists());
}

#[test]
fn export_omits_current_executable_paths_but_retains_exact_historical_frozen_paths() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("project")).unwrap();
    store.create("导出路径边界", "project").unwrap();
    let current = paths(&dir.path().join("local-tools"));
    store.save_settings(&current).unwrap();
    let job = store
        .add_job(
            "image",
            "synthetic-asset",
            "dreamina",
            "frozen-account",
            &json!({"providerSettings":current}),
            "submission_unknown",
        )
        .unwrap();
    let original = format!("\n {}\n", raw_snapshot(&store, &job));
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
            params![original, job],
        )
        .unwrap();
    let package = dir.path().join("portable.zip");
    portable::export(&store, &package).unwrap();
    let mut archive = ZipArchive::new(fs::File::open(&package).unwrap()).unwrap();
    assert_eq!(archive.len(), 2);
    assert!(archive.by_name(FILE_NAME).is_err());
    let exported_db = dir.path().join("exported.sqlite");
    let mut output = fs::File::create(&exported_db).unwrap();
    std::io::copy(&mut archive.by_name("project.sqlite").unwrap(), &mut output).unwrap();
    drop(output);
    let connection = rusqlite::Connection::open(&exported_db).unwrap();
    for key in PATH_KEYS {
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM settings WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{key}");
    }
    let exported_frozen: String = connection
        .query_row("SELECT snapshot FROM jobs WHERE id=?1", [&job], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(exported_frozen, original);
    assert_paths(&store.settings().unwrap(), &current);
    assert_eq!(raw_snapshot(&store, &job), original);
}

#[test]
fn corrupt_or_extra_field_defaults_fail_closed_and_are_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let valid = paths(&dir.path().join("tools"));
    let mut invalid = valid.clone();
    invalid["textApiKey"] = json!("synthetic-secret");
    let file = dir.path().join(FILE_NAME);
    fs::write(&file, json!({"version":1,"paths":invalid}).to_string()).unwrap();
    let original = fs::read(&file).unwrap();
    assert!(executable_paths::load(dir.path()).is_err());
    assert!(executable_paths::remember(dir.path(), &valid).is_err());
    assert_eq!(fs::read(&file).unwrap(), original);
}

#[test]
fn concurrent_explicit_saves_merge_distinct_paths_without_lost_updates() {
    let dir = tempfile::tempdir().unwrap();
    let expected = paths(&dir.path().join("tools"));
    let workers: Vec<_> = PATH_KEYS
        .into_iter()
        .map(|key| {
            let home = dir.path().to_path_buf();
            let path = expected[key].clone();
            std::thread::spawn(move || {
                executable_paths::remember(&home, &json!({key:path})).unwrap()
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_paths(&executable_paths::load(dir.path()).unwrap(), &expected);
}
