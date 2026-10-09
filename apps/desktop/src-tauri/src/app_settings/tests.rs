use super::*;
use crate::store::Store;

fn project(root: &Path, settings: Value) -> (Store, Value) {
    let store = Store::open(root.to_owned()).unwrap();
    store.create("fixture", &id()).unwrap();
    store.save_settings(&settings).unwrap();
    let row = store.project().unwrap();
    (store, row)
}
#[test]
fn empty_registry_roundtrip_has_no_project_and_rejects_stale_save() {
    let root = tempfile::tempdir().unwrap();
    let doc = initialize(root.path(), &json!([]), None).unwrap();
    assert_eq!(doc["revision"], 1);
    assert!(!root.path().join("projects").exists());
    let saved = save(
        root.path(),
        &json!({"textProvider":"codex_sdk","textModel":"custom-model","reasoning":"high"}),
        1,
    )
    .unwrap();
    assert_eq!(load(root.path()).unwrap().unwrap(), saved);
    assert!(save(root.path(), &json!({"textModel":"stale"}), 1)
        .unwrap_err()
        .contains("APP_SETTINGS_CONFLICT"));
    assert_eq!(load(root.path()).unwrap().unwrap(), saved);
    assert!(save(root.path(), &json!({"ratio":"9:16"}), 2).is_err());
}
#[test]
fn selected_migration_reads_only_and_keeps_project_production_settings() {
    let root = tempfile::tempdir().unwrap();
    let (store, row) = project(
        &root.path().join("p"),
        json!({"textProvider":"codex_sdk","textModel":"preferred","reasoning":"high","ratio":"9:16","visualStyle":"project-specific","bhEndpoint":"https://fixture.invalid/v1","bhAccountId":"original"}),
    );
    let before = fs::read(store.root.join("project.sqlite")).unwrap();
    let doc = initialize(
        &root.path().join("home"),
        &json!([row]),
        Some(s(&row, "id")),
    )
    .unwrap();
    assert_eq!(doc["settings"]["textModel"], "preferred");
    assert!(doc["settings"].get("ratio").is_none());
    assert_eq!(doc["credentialRefs"]["bh"], format!("{}:bh", s(&row, "id")));
    assert_eq!(before, fs::read(store.root.join("project.sqlite")).unwrap());
    let settings = effective_from(&doc, &store.settings().unwrap());
    assert_eq!(settings["ratio"], "9:16");
    assert_eq!(settings["visualStyle"], "project-specific");
}
#[test]
fn migration_does_not_choose_conflicting_project_or_credential() {
    let root = tempfile::tempdir().unwrap();
    let (_, a) = project(
        &root.path().join("a"),
        json!({"textModel":"a","textEndpoint":"https://fixture.invalid/v1"}),
    );
    let (_, b) = project(
        &root.path().join("b"),
        json!({"textModel":"b","textEndpoint":"https://fixture.invalid/v1"}),
    );
    let doc = initialize(&root.path().join("home"), &json!([a, b]), None).unwrap();
    assert_eq!(doc["settings"]["textModel"], defaults()["textModel"]);
    assert!(doc["credentialRefs"]["text"].is_null());
    assert!(doc["migration"]["conflictingFields"]
        .as_array()
        .unwrap()
        .contains(&json!("textModel")));
    assert!(doc["migration"]["conflictingFields"]
        .as_array()
        .unwrap()
        .contains(&json!("textCredentialSource")));
}
#[test]
fn missing_archived_and_corrupt_migration_sources_never_block_settings() {
    let root = tempfile::tempdir().unwrap();
    let bad = root.path().join("bad");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("project.sqlite"), b"invalid database").unwrap();
    let rows = json!([{"id":"missing","rootPath":root.path().join("missing")},{"id":"corrupt","rootPath":bad},{"id":"archived","rootPath":root.path().join("archive"),"archivedAt":"2026-10-09"}]);
    for selected in [None, Some("missing"), Some("corrupt"), Some("archived")] {
        let doc = initialize(
            &root.path().join(selected.unwrap_or("all")),
            &rows,
            selected,
        )
        .unwrap();
        assert!(!doc["migration"]["skippedProjects"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(doc["settings"]["textProvider"], "codex");
    }
    assert_eq!(
        fs::read(bad.join("project.sqlite")).unwrap(),
        b"invalid database"
    );
    assert!(!root.path().join("missing/project.sqlite").exists());
}
#[test]
fn rotated_global_credentials_keep_old_frozen_identity_and_never_serialize_secret() {
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &json!([]), None).unwrap();
    let first=save(root.path(),&json!({"textProvider":"api","textApiKey":"synthetic-first","textEndpoint":"http://127.0.0.1:9"}),1).unwrap();
    let mut frozen = effective_from(&first, &json!({}));
    freeze_credentials(&mut frozen);
    let second = save(root.path(), &json!({"textApiKey":"synthetic-second"}), 2).unwrap();
    assert_ne!(
        first["credentialRefs"]["text"],
        second["credentialRefs"]["text"]
    );
    assert_eq!(
        credential(&frozen, "project", "text").unwrap(),
        "synthetic-first"
    );
    assert_eq!(
        credential(&effective_from(&second, &json!({})), "project", "text").unwrap(),
        "synthetic-second"
    );
    assert!(!fs::read_to_string(root.path().join(FILE))
        .unwrap()
        .contains("synthetic-"));
    let public = public(&second, Value::Null, json!([])).to_string();
    assert!(!public.contains("credentialRefs"));
    assert!(!public.contains("synthetic-"));
    let third = save(root.path(), &json!({"textApiKey":""}), 3).unwrap();
    assert_eq!(third["credentialRefs"], second["credentialRefs"]);
}
#[test]
fn mutated_legacy_credential_and_missing_frozen_key_fail_closed() {
    let key = id();
    let reference = format!("{key}:text");
    crate::platform::write_credential(&reference, "original").unwrap();
    let doc = json!({"revision":1,"settings":{},"credentialRefs":{"text":reference}});
    let mut frozen = effective_from(&doc, &json!({}));
    freeze_credentials(&mut frozen);
    crate::platform::write_credential(&reference, "changed").unwrap();
    assert!(credential(&frozen, "other", "text")
        .unwrap_err()
        .contains("已变化"));
    crate::platform::remove_synthetic_credential(&reference);
    let mut unavailable = effective_from(&doc, &json!({}));
    freeze_credentials(&mut unavailable);
    crate::platform::write_credential(&reference, "arrived-later").unwrap();
    assert!(credential(&unavailable, "other", "text").is_err());
    assert_eq!(
        credential(&json!({}), &key, "text").unwrap(),
        "arrived-later"
    );
    let before = unavailable.clone();
    freeze_credentials(&mut unavailable);
    assert_eq!(before, unavailable);
}
#[test]
fn cache_is_cas_bound_and_settings_save_invalidates_catalog() {
    let root = tempfile::tempdir().unwrap();
    initialize(root.path(), &json!([]), None).unwrap();
    cache(
        root.path(),
        "bh",
        1,
        "fixture",
        &json!({"provider":"bh","models":[]}),
    )
    .unwrap();
    let doc = save(
        root.path(),
        &json!({"bhEndpoint":"https://fixture.invalid/v2"}),
        1,
    )
    .unwrap();
    assert_eq!(doc["catalogs"], json!({}));
    assert!(cache(root.path(), "bh", 1, "stale", &json!({})).is_err());
}
#[test]
fn corrupt_global_document_is_preserved_not_replaced_with_defaults() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(FILE), b"bad-json").unwrap();
    assert!(initialize(root.path(), &json!([]), None).is_err());
    assert_eq!(fs::read(root.path().join(FILE)).unwrap(), b"bad-json");
}
