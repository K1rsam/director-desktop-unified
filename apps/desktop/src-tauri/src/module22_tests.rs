use super::*;
use crate::{lingmo_import, lingmo_package, store::Store};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/module22-native")
}
fn read(name: &str) -> Value {
    serde_json::from_slice(&fs::read(fixture().join(name)).unwrap()).unwrap()
}
fn original_values() -> (Value, Value, Value) {
    (
        read("image-queue.json"),
        read("batch.json"),
        read("registry/v000001.json"),
    )
}
fn validate(q: &Value, b: &Value, r: &Value) -> Result<Value> {
    validate_batch(
        q,
        b,
        r,
        s(&q["assetRegistry"], "sha256"),
        s(q, "sourceHash"),
    )
}
fn copy_fixture(target: &Path, include_map: bool) {
    fn copy(from: &Path, to: &Path, include_map: bool) {
        fs::create_dir_all(to).unwrap();
        for e in fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &to.join(e.file_name()), include_map);
            } else if include_map || e.file_name() != "module22-map.json" {
                fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    copy(&fixture(), target, include_map);
}
#[test]
fn javascript_hash_matches_official_number_utf16_and_integer_key_goldens() {
    for item in read("javascript-hashes.json").as_array().unwrap() {
        assert_eq!(js::stable(&item["value"]).unwrap(), s(item, "stable"));
        assert_eq!(js::hash(&item["value"]).unwrap(), s(item, "sha256"));
    }
    assert_eq!(
        js::stringify_raw(br#"{"z":24.0,"2":2,"a":-0,"1":1,"z":25.0}"#).unwrap(),
        r#"{"1":1,"2":2,"z":25,"a":0}"#
    );
}
#[test]
fn native_batch_matches_real_official_script_fixture_and_keeps_ancestor_readonly() {
    let (q, b, r) = original_values();
    let receipt = validate(&q, &b, &r).unwrap();
    let official = &read("official-validation.json")["queueCheck"];
    for key in ["ok", "legacy", "queueHash", "tasks", "imageApproval"] {
        assert_eq!(receipt[key], official[key]);
    }
    assert_eq!(r["assets"].as_array().unwrap().len(), 5);
    assert_eq!(q["items"].as_array().unwrap().len(), 4);
    assert!(q["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["assetId"] != "A0005"));
}
#[test]
fn unrelated_registry_addition_does_not_expire_queue_but_real_parent_change_does() {
    let (q, b, mut r) = original_values();
    r["revision"] = json!(2);
    let mut unrelated = r["assets"][4].clone();
    unrelated["id"] = json!("A0006");
    unrelated["name"] = json!("独立设计");
    r["assets"].as_array_mut().unwrap().push(unrelated);
    validate(&q, &b, &r).unwrap();
    r["assets"][4]["revision"] = json!(2);
    assert!(validate(&q, &b, &r).unwrap_err().contains("DEPENDENCY"));
    let independent = q["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["type"] == "SC")
        .unwrap()
        .clone();
    validate(&q, &batch(&q, &[independent]), &r).unwrap();
}
#[test]
fn unknown_task_changed_use_hash_filename_and_design_are_rejected() {
    let (q, b, r) = original_values();
    for key in ["taskId", "filename", "assetRevision", "designHash"] {
        let mut bad = b.clone();
        bad["tasks"][0][key] = json!("tampered");
        assert!(validate(&q, &bad, &r).is_err());
    }
    let mut bad = b.clone();
    bad["tasks"][0]["uses"][0]["purpose"] = json!("invented use");
    assert!(validate(&q, &bad, &r).is_err());
    let mut bad = q.clone();
    bad["items"][0]["design"]["text"] = json!("tampered");
    assert!(validate(&bad, &b, &r).is_err());
    let mut current = r.clone();
    current["assets"][0]["design"]["state"] = json!("STALE");
    assert!(validate(&q, &b, &current).is_err());
}
#[test]
fn native_complete_package_import_reopens_and_freezes_original_task_without_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let pack = tmp.path().join("native");
    copy_fixture(&pack, true);
    let loaded = lingmo_package::load(&pack).unwrap();
    assert_eq!(loaded.value["proof"]["sourceFormat"], FORMAT);
    assert_eq!(loaded.value["proof"]["independentDesktopReview"], false);
    let store = Store::open(tmp.path().join("consumer")).unwrap();
    store.create("Native synthetic fixture", "test").unwrap();
    let mut request = json!({"path":pack.join("adopted.md")});
    let preview = lingmo_import::preview_file(&request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    let imported = lingmo_import::import(&store, &request).unwrap();
    assert_eq!(imported["handoff"]["completePackageVerified"], true);
    assert_eq!(store.values("assets").unwrap().len(), 4);
    assert!(store.jobs().unwrap().is_empty());
    assert!(store.source().unwrap().is_null());
    let assets = store.values("assets").unwrap();
    let a = &assets[0];
    assert!(a["adoptedDesign"]["full"].is_object());
    let mut snapshot = json!({"requestId":"synthetic-native-request","assetId":a["id"]});
    freeze_image_batch(&store, a, &mut snapshot).unwrap();
    assert!(!snapshot["module22Batches"].as_array().unwrap().is_empty());
    validate_image_snapshot(&store, &snapshot).unwrap();
    assert!(snapshot["module22Batches"][0]["externalTaskId"]
        .as_str()
        .unwrap()
        .starts_with("IMG-"));
    let before = store.values("assets").unwrap();
    lingmo_import::import(&store, &request).unwrap();
    assert_eq!(before, store.values("assets").unwrap());
    fs::rename(&pack, tmp.path().join("origin-moved")).unwrap();
    let reopened = Store::open(store.root.clone()).unwrap();
    assert!(
        lingmo_package::stored(&reopened, s(&imported, "documentId"))
            .unwrap()
            .is_some()
    );
    validate_image_snapshot(&reopened, &snapshot).unwrap();
    assert!(reopened.jobs().unwrap().is_empty());
    // A damaged pointer must not downgrade new dispatches to nonnative jobs;
    // the original document still declares a complete native package.
    reopened
        .conn()
        .unwrap()
        .execute(
            "DELETE FROM metadata WHERE key=?1",
            [format!("lingmo.package:{}", s(&imported, "documentId"))],
        )
        .unwrap();
    assert!(freeze_image_batch(
        &reopened,
        a,
        &mut json!({"requestId":"new-request-after-package-loss"})
    )
    .unwrap_err()
    .contains("COMPLETE_PACKAGE_MISSING"));
    assert!(validate_image_snapshot(&reopened, &snapshot)
        .unwrap_err()
        .contains("COMPLETE_PACKAGE_MISSING"));
}
#[test]
fn folder_and_direction_entry_discovers_exact_files_without_user_json() {
    let tmp = tempfile::tempdir().unwrap();
    let pack = tmp.path().join("native");
    copy_fixture(&pack, false);
    let preview=lingmo_import::preview_file(&json!({"path":pack.join("adopted.md"),"module22Root":pack,"module22DirectionPath":pack.join("direction.md")})).unwrap();
    assert_eq!(preview["handoff"]["proof"]["sourceFormat"], FORMAT);
    assert_eq!(preview["handoff"]["completePackageVerified"], true);
}

#[test]
fn native_namespace_uses_verified_registry_without_rewriting_extra_catalog_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let pack = tmp.path().join("native");
    copy_fixture(&pack, true);
    let mut catalog = read("adopted.md.assets.json");
    catalog["assetRegistryId"] = json!("another-registry-must-not-own-native-identity");
    catalog["sourceSha256"] = json!("unrelated-optional-catalog-metadata");
    fs::write(
        pack.join("adopted.md.assets.json"),
        serde_json::to_vec_pretty(&catalog).unwrap(),
    )
    .unwrap();
    let store = Store::open(tmp.path().join("consumer")).unwrap();
    store.create("Native namespace fixture", "test").unwrap();
    let mut request = json!({"path":pack.join("adopted.md")});
    let preview = lingmo_import::preview_file(&request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    let imported = lingmo_import::import(&store, &request).unwrap();
    let package = lingmo_package::stored(&store, s(&imported, "documentId"))
        .unwrap()
        .unwrap();
    assert_eq!(package["catalog"], catalog);
    let namespace = format!("assetRegistryId:{}", s(&package["proof"], "registryId"));
    for asset in store.values("assets").unwrap() {
        assert_eq!(asset["handoffRegistry"], namespace);
        let mut snapshot = json!({"requestId":format!("native-namespace-{}",s(&asset,"externalId")),"assetId":asset["id"]});
        freeze_image_batch(&store, &asset, &mut snapshot).unwrap();
        validate_image_snapshot(&store, &snapshot).unwrap();
    }
}
#[test]
fn queue_command_reuses_verified_package_but_each_send_and_next_command_rechecks_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let pack = tmp.path().join("native");
    copy_fixture(&pack, true);
    let store = Store::open(tmp.path().join("consumer")).unwrap();
    store.create("Command cache fixture", "test").unwrap();
    let mut request = json!({"path":pack.join("adopted.md")});
    let preview = lingmo_import::preview_file(&request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    let imported = lingmo_import::import(&store, &request).unwrap();
    let assets = store.values("assets").unwrap();
    let mut context = ImageBatchContext::new(&store);
    let mut snapshots = Vec::new();
    for asset in &assets {
        let mut snapshot = json!({"requestId":format!("shared-command-{}",s(asset,"externalId")),"assetId":asset["id"]});
        freeze_image_batch_with_context(&store, asset, &mut snapshot, &mut context).unwrap();
        validate_image_snapshot(&store, &snapshot).unwrap();
        snapshots.push(snapshot);
    }
    assert_eq!(assets.len(), 4);
    assert_eq!(context.validated_package_count(), 1);
    let other = Store::open(tmp.path().join("other-project")).unwrap();
    assert!(
        freeze_image_batch_with_context(&other, &assets[0], &mut json!({}), &mut context)
            .unwrap_err()
            .contains("COMMAND_STORE_IDENTITY")
    );
    let fingerprint = s(&imported["handoff"]["proof"], "packageFingerprint");
    let stored_source = store.root.join(format!(
        "imports/complete-packages/{fingerprint}/source-original.txt"
    ));
    let original_bytes = fs::read(&stored_source).unwrap();
    fs::write(
        &stored_source,
        "changed after the immutable queue command was frozen",
    )
    .unwrap();
    // Previously validated command data never substitutes for the actual-send
    // boundary, and a subsequent command cannot observe a process-wide cache.
    for snapshot in &snapshots {
        assert!(validate_image_snapshot(&store, snapshot).is_err());
    }
    assert!(
        freeze_image_batch(&store, &assets[0], &mut json!({"requestId":"next-command"})).is_err()
    );
    let mut failed_context = ImageBatchContext::new(&store);
    assert!(freeze_image_batch_with_context(
        &store,
        &assets[0],
        &mut json!({"requestId":"invalid-package"}),
        &mut failed_context
    )
    .is_err());
    fs::write(&stored_source, original_bytes).unwrap();
    assert!(freeze_image_batch_with_context(
        &store,
        &assets[1],
        &mut json!({"requestId":"same-invalid-command"}),
        &mut failed_context
    )
    .is_err());
    freeze_image_batch(
        &store,
        &assets[1],
        &mut json!({"requestId":"fresh-restored-command"}),
    )
    .unwrap();
    assert!(store.jobs().unwrap().is_empty());
}

#[test]
fn catalog_only_season_context_is_browsable_but_cannot_create_prompt_input() {
    let tmp = tempfile::tempdir().unwrap();
    let pack = tmp.path().join("native");
    copy_fixture(&pack, true);
    let store = Store::open(tmp.path().join("consumer")).unwrap();
    store
        .create("Restricted synthetic fixture", "test")
        .unwrap();
    let mut request =
        json!({"path":pack.join("adopted.md"),"handoffMode":"catalog-only","module22Root":pack});
    let preview = lingmo_import::preview_file(&request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    lingmo_import::import(&store, &request).unwrap();
    assert!(store.image_workflow_input(&json!({}), &json!({})).is_err());
    let a = store.values("assets").unwrap().remove(0);
    assert!(freeze_image_batch(&store, &a, &mut json!({"requestId":"blocked"})).is_err());
    assert!(store.jobs().unwrap().is_empty());
}
#[test]
fn immutable_snapshot_and_source_bytes_fail_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let pack = tmp.path().join("native");
    copy_fixture(&pack, true);
    fs::write(pack.join("source-original.txt"), "different bytes").unwrap();
    assert!(lingmo_package::load(&pack).is_err());
}

#[test]
fn season_batch_matches_official_positive_and_eight_negative_cases() {
    let q = read("season/image-queue.json");
    let b = read("season/batch.json");
    let r = read("season/registry/v000001.json");
    let official = read("season/official-validation.json");
    let receipt = validate(&q, &b, &r).unwrap();
    for key in [
        "ok",
        "legacy",
        "queueHash",
        "productionScopeHash",
        "tasks",
        "imageApproval",
    ] {
        assert_eq!(receipt[key], official["receipt"][key]);
    }
    for case in official["cases"].as_array().unwrap() {
        let mut values = json!({"queue":q,"batch":b,"registry":r});
        let target = s(case, "target");
        let pointer = s(case, "pointer");
        if case["remove"] == true {
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            values[target]
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
        } else {
            *values[target].pointer_mut(pointer).unwrap() = case["value"].clone();
        }
        let actual = validate(&values["queue"], &values["batch"], &values["registry"]);
        assert_eq!(
            actual.is_ok(),
            case["expectedOk"].as_bool().unwrap(),
            "{case}: {actual:?}"
        );
    }
}

#[test]
fn complete_season_package_supplies_designs_without_pixels_and_cannot_downgrade_scope() {
    let pack = fixture().join("season");
    let loaded = lingmo_package::load(&pack).unwrap();
    assert_eq!(loaded.value["proof"]["sourceFormat"], FORMAT);
    assert_eq!(loaded.value["queue"]["productionScope"]["status"], "locked");
    assert_eq!(loaded.value["upstreamBundle"]["seasonDelivery"], "preview");
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path().join("complete")).unwrap();
    store.create("Native season fixture", "test").unwrap();
    let mut request = json!({"path":pack.join("adopted.md")});
    let preview = lingmo_import::preview_file(&request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    lingmo_import::import(&store, &request).unwrap();
    let input = store.image_workflow_input(&json!({}), &json!({})).unwrap();
    assert_eq!(input["imageSource"]["sourceKind"], "external-package");
    assert_eq!(rows(&input, "assets").unwrap().len(), 4);
    assert_eq!(
        rows(&input["externalPackages"][0]["registry"], "assets")
            .unwrap()
            .len(),
        5
    );
    assert!(input["externalPackages"][0]["catalog"]
        .get("assetRegistryId")
        .is_none());
    for asset in rows(&input, "assets").unwrap() {
        let basis = crate::pipeline::asset_workflow::manual_prompt_basis(&input, asset).unwrap();
        assert_eq!(basis["sourceKind"], "external-package");
        assert_eq!(basis["basedOnDesignRevision"], 1);
    }
    assert!(store.jobs().unwrap().is_empty());
    assert!(store.source().unwrap().is_null());

    let restricted = Store::open(tmp.path().join("restricted")).unwrap();
    restricted.create("Season MD only fixture", "test").unwrap();
    request["handoffMode"] = json!("catalog-only");
    let preview = lingmo_import::preview_file(&request).unwrap();
    request["previewToken"] = preview["previewToken"].clone();
    lingmo_import::import(&restricted, &request).unwrap();
    assert!(restricted
        .image_workflow_input(&json!({}), &json!({}))
        .is_err());
    assert!(restricted.jobs().unwrap().is_empty());
}

#[test]
#[ignore = "explicit read-only external queue, registry and source paths required"]
fn native_real_module22_queue_readonly_acceptance() {
    use sha2::{Digest, Sha256};
    let queue_path = std::env::var("DIRECTOR_MODULE22_QUEUE").unwrap();
    let registry_dir = std::env::var("DIRECTOR_MODULE22_REGISTRY").unwrap();
    let source_path = std::env::var("DIRECTOR_MODULE22_SOURCE").unwrap();
    let started = std::time::Instant::now();
    let q: Value = serde_json::from_slice(&fs::read(queue_path).unwrap()).unwrap();
    let head: Value =
        serde_json::from_slice(&fs::read(Path::new(&registry_dir).join("current.json")).unwrap())
            .unwrap();
    let current_bytes = fs::read(
        Path::new(&registry_dir).join(format!("v{:06}.json", head["revision"].as_u64().unwrap())),
    )
    .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&current_bytes)),
        s(&head, "sha256")
    );
    let r: Value = serde_json::from_slice(&current_bytes).unwrap();
    let old = fs::read(Path::new(&registry_dir).join(s(&q["assetRegistry"], "snapshot"))).unwrap();
    let source = fs::read(source_path).unwrap();
    let report = validate_batch(
        &q,
        &batch(&q, rows(&q, "items").unwrap()),
        &r,
        &format!("{:x}", Sha256::digest(old)),
        &format!("{:x}", Sha256::digest(source)),
    )
    .unwrap();
    println!(
        "{}",
        json!({"receipt":report,"elapsedMilliseconds":started.elapsed().as_millis(),"imageSubmissions":0,"videoSubmissions":0,"productionFilesModified":false})
    );
}
