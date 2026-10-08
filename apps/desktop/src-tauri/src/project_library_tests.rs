use super::*;
use crate::{
    store::{hash_file, Store},
    Engine,
};
use std::sync::{Arc, Barrier};

fn existing_project(root: &Path, id: &str) -> Store {
    let store = Store::open(root.to_owned()).unwrap();
    store.create(id, id).unwrap();
    store
}
fn registration(id: &str, root: &Path) -> Value {
    json!({"id":id,"title":id,"rootPath":root,"archivedAt":null,"lifecycleVersion":0})
}

#[test]
fn recovering_virtualized_registrations_preserves_primary_lifecycle_and_actual_media() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("normal");
    let legacy = temp.path().join("package/LocalCache/Roaming/workbench");
    let old_root = home.join("projects/old");
    existing_project(&old_root, "old");
    let primary = json!([{"id":"old","title":"archived title","rootPath":old_root,"archivedAt":"2026-10-04","lifecycleVersion":3}]);
    atomic_json(&home.join("projects.json"), &primary).unwrap();
    let physical_root = legacy.join("projects/new");
    let store = existing_project(&physical_root, "new");
    let original = temp.path().join("original.png");
    image::RgbImage::from_pixel(10, 10, image::Rgb([1, 2, 3]))
        .save(&original)
        .unwrap();
    let media = store.import_media(&original, "image", None, None).unwrap();
    atomic_json(
        &legacy.join("projects.json"),
        &json!([
            registration("old", &old_root),
            registration("new", &home.join("projects/new"))
        ]),
    )
    .unwrap();
    let db_hash = hash_file(&physical_root.join("project.sqlite")).unwrap();
    let report = recover(&home, &[legacy.join("projects.json")]).unwrap();
    assert_eq!(report["imported"].as_array().unwrap().len(), 1);
    let rows = read_rows(&home.join("projects.json")).unwrap();
    assert_eq!(rows[0], primary[0]);
    assert_eq!(rows[1]["rootPath"], json!(physical_root));
    assert_eq!(
        Store {
            root: physical_root.clone()
        }
        .verify_media(&media)
        .unwrap(),
        physical_root.join(s(&media, "relativePath"))
    );
    assert_eq!(
        hash_file(&physical_root.join("project.sqlite")).unwrap(),
        db_hash
    );
    assert!(!home.join("projects/new").exists());
    let bytes = fs::read(home.join("projects.json")).unwrap();
    assert_eq!(
        recover(&home, &[legacy.join("projects.json")]).unwrap()["imported"],
        json!([])
    );
    assert_eq!(fs::read(home.join("projects.json")).unwrap(), bytes);
}

#[test]
fn a_deleted_project_is_not_reimported_from_a_consumed_legacy_registry() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("normal");
    let legacy = temp.path().join("legacy");
    existing_project(&legacy.join("projects/new"), "new");
    atomic_json(
        &legacy.join("projects.json"),
        &json!([registration("new", &home.join("projects/new"))]),
    )
    .unwrap();
    recover(&home, &[legacy.join("projects.json")]).unwrap();
    atomic_json(&home.join("projects.json"), &json!([])).unwrap();
    let report = recover(&home, &[legacy.join("projects.json")]).unwrap();
    assert_eq!(report["projectCount"], 0);
    assert_eq!(report["imported"], json!([]));
    assert!(legacy.join("projects/new/project.sqlite").is_file());
}

#[test]
fn missing_or_mismatched_databases_are_never_initialized_or_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("normal");
    let legacy = temp.path().join("legacy");
    existing_project(&legacy.join("projects/wrong"), "actual-id");
    atomic_json(
        &legacy.join("projects.json"),
        &json!([
            registration("wrong", &home.join("projects/wrong")),
            registration("missing", &home.join("projects/missing"))
        ]),
    )
    .unwrap();
    let report = recover(&home, &[legacy.join("projects.json")]).unwrap();
    assert_eq!(report["skipped"].as_array().unwrap().len(), 2);
    assert_eq!(report["imported"], json!([]));
    assert!(!home.join("projects.json").exists());
    assert!(!home.join("projects").exists());
    assert_eq!(
        Store {
            root: legacy.join("projects/wrong")
        }
        .project()
        .unwrap()["id"],
        "actual-id"
    );
}

#[test]
fn a_corrupt_primary_registry_fails_before_legacy_recovery_writes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("normal");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("projects.json"), b"not json").unwrap();
    assert!(recover(&home, &[]).is_err());
    assert_eq!(fs::read(home.join("projects.json")).unwrap(), b"not json");
    assert!(!home.join("project-library-recovery.json").exists());
}

#[test]
fn two_engines_cannot_lose_concurrent_project_registrations() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("shared");
    let engines = [
        Engine::new(home.clone()).unwrap(),
        Engine::new(home.clone()).unwrap(),
    ];
    let barrier = Arc::new(Barrier::new(2));
    let threads:Vec<_>=engines.into_iter().enumerate().map(|(index,engine)| {
        let barrier=barrier.clone(); thread::spawn(move || {
            barrier.wait();
            for n in 0..6 { engine.dispatch(json!({"action":"projects.create","title":format!("client {index} project {n}")})).unwrap(); }
        })
    }).collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let rows = read_rows(&home.join("projects.json")).unwrap();
    assert_eq!(rows.len(), 12);
    for row in rows {
        assert!(Path::new(s(&row, "rootPath"))
            .join("project.sqlite")
            .is_file());
    }
}
