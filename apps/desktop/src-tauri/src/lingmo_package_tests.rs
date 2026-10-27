use super::*;
use crate::lingmo_import;

struct Fixture {
    directory: tempfile::TempDir,
    package: PathBuf,
    input: Value,
}
impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("producer")).unwrap();
        store.create("合成完整包", "synthetic-package").unwrap();
        let input = series::seed_engine_series_fixture(&store, 2).await;
        let deliveries = store.series_deliveries().unwrap();
        let package = directory.path().join("package");
        series::export_delivery_package(&input, &deliveries, &package).unwrap();
        Self {
            directory,
            package,
            input,
        }
    }
    fn store(&self) -> Store {
        let store = Store::open(self.directory.path().join("consumer")).unwrap();
        store.create("合成接续", "synthetic-import").unwrap();
        store
    }
    fn request(&self) -> Value {
        let mut request = json!({"path":self.package.join("全剧_灵墨v7.md")});
        let preview = lingmo_import::preview_file(&request).unwrap();
        request["expectedSha256"] = preview["markdownSha256"].clone();
        request["previewToken"] = preview["previewToken"].clone();
        request
    }
    fn change(&self, name: &str, edit: impl FnOnce(&mut Value)) {
        let path = self.package.join(name);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        edit(&mut value);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&path, &bytes).unwrap();
        if name != "manifest.json" {
            let manifest_path = self.package.join("manifest.json");
            let mut manifest: Value =
                serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
            let row = manifest["files"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|r| r["path"] == name)
                .unwrap();
            row["sha256"] = json!(bytes_hash(&bytes));
            row["bytes"] = json!(bytes.len());
            fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        }
    }
}

#[tokio::test]
async fn complete_package_is_consumed_without_source_or_generation_and_reopens_without_origin() {
    let f = Fixture::new().await;
    let store = f.store();
    let request = f.request();
    let preview = lingmo_import::preview_file(&request).unwrap();
    assert_eq!(preview["handoff"]["sourceKind"], "external-package");
    assert_eq!(preview["handoff"]["completePackageVerified"], true);
    let imported = lingmo_import::import(&store, &request).unwrap();
    let id = s(&imported, "documentId");
    let evidence = stored(&store, id).unwrap().unwrap();
    let locked: LockedSource = serde_json::from_value(f.input["source"].clone()).unwrap();
    assert_eq!(evidence["sourceLock"], json!(locked));
    assert_eq!(evidence["direction"]["settings"]["visualBaseline"], f.input["settings"]["visualBaseline"]);
    assert_eq!(
        evidence["queue"]["items"].as_array().unwrap().len(),
        store.values("assets").unwrap().len()
    );
    assert_eq!(store.source().unwrap(), Value::Null);
    assert!(store.jobs().unwrap().is_empty());
    assert!(store.values("media").unwrap().is_empty());
    let before_assets = store.values("assets").unwrap();
    let before_units = store.values("units").unwrap();
    lingmo_import::import(&store, &request).unwrap();
    assert_eq!(store.values("assets").unwrap(), before_assets);
    assert_eq!(store.values("units").unwrap(), before_units);
    fs::rename(&f.package, f.directory.path().join("source-moved-away")).unwrap();
    let reopened = Store::open(store.root.clone()).unwrap();
    assert_eq!(stored(&reopened, id).unwrap(), Some(evidence));
    assert_eq!(reopened.values("assets").unwrap(), before_assets);
}

#[tokio::test]
async fn complete_package_rejects_different_visual_adoption_even_with_updated_file_hash() {
    let f = Fixture::new().await;
    f.change("资料/direction.json", |value| {
        value["settings"]["visualBaseline"]["revision"] = json!(999);
    });
    let request = json!({"path":f.package.join("全剧_灵墨v7.md")});
    assert!(lingmo_import::preview_file(&request).is_err());
}

#[tokio::test]
async fn catalog_only_upgrade_preserves_edits_media_and_unknown_frozen_request() {
    let f = Fixture::new().await;
    let store = f.store();
    let mut request = json!({"path":f.package.join("全剧_灵墨v7.md"),"handoffMode":"catalog-only"});
    let preview = lingmo_import::preview_file(&request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    assert_eq!(preview["handoff"]["completePackageVerified"], false);
    let first = lingmo_import::import(&store, &request).unwrap();
    assert!(stored(&store, s(&first, "documentId")).unwrap().is_none());
    let mut asset = store.values("assets").unwrap().remove(0);
    asset["expectedVersion"] = asset["version"].clone();
    asset["prompt"] = json!("人类手写提示词保留");
    let asset = store.save_asset(asset).unwrap();
    let image_path = f.directory.path().join("human-image.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([90, 130, 170]))
        .save(&image_path)
        .unwrap();
    let media = store
        .import_media(
            &image_path,
            "image",
            Some(s(&asset, "id")),
            Some(s(&asset, "variantId")),
        )
        .unwrap();
    store
        .adopt(s(&asset, "id"), s(&asset, "variantId"), s(&media, "id"))
        .unwrap();
    let job = store
        .add_job(
            "image",
            s(&asset, "id"),
            "offline",
            "fixture",
            &json!({"original":"immutable"}),
            "submission_unknown",
        )
        .unwrap();
    let before = (
        store.values("assets").unwrap(),
        store.values("units").unwrap(),
        store.job(&job).unwrap(),
        store.values("media").unwrap(),
    );
    let upgraded = lingmo_import::import(&store, &f.request()).unwrap();
    assert_eq!(upgraded["duplicate"], true);
    assert_eq!(upgraded["handoff"]["completePackageVerified"], true);
    assert_eq!(
        (
            store.values("assets").unwrap(),
            store.values("units").unwrap(),
            store.job(&job).unwrap(),
            store.values("media").unwrap()
        ),
        before
    );
}

#[tokio::test]
async fn package_file_byte_corruption_and_missing_file_are_rejected() {
    let f = Fixture::new().await;
    let target = f.package.join("资料/direction.json");
    let original = fs::read(&target).unwrap();
    let mut changed = original.clone();
    changed.push(b' ');
    fs::write(&target, changed).unwrap();
    assert!(load(&f.package).err().unwrap().contains("摘要或长度"));
    fs::write(&target, original).unwrap();
    fs::rename(&target, f.package.join("direction.moved")).unwrap();
    assert!(load(&f.package).is_err());
}

#[tokio::test]
async fn rehashed_queue_expansion_and_direction_drift_do_not_pass() {
    let f = Fixture::new().await;
    f.change("资料/image-queue.json", |v| {
        let item = v["items"][0].clone();
        v["items"].as_array_mut().unwrap().push(item);
        v.as_object_mut().unwrap().remove("queueHash");
        v["queueHash"] = json!(hash(v));
    });
    assert!(load(&f.package).err().unwrap().contains("队列范围"));
    let g = Fixture::new().await;
    g.change("资料/direction.json", |v| {
        v["adoptedDirection"]["visualStyle"] = json!("未经采用的另一风格")
    });
    assert!(load(&g.package).err().unwrap().contains("采用方向"));
}

#[tokio::test]
async fn preview_binds_manifest_and_independent_review_must_match_current_draft() {
    let f = Fixture::new().await;
    let request = f.request();
    let store = f.store();
    f.change("manifest.json", |v| {
        v["annotation"] = json!("preview drift")
    });
    assert!(lingmo_import::import(&store, &request)
        .unwrap_err()
        .contains("重新预览"));
    assert!(store.values("assets").unwrap().is_empty());
    f.change("资料/episodes/0001.json", |v| {
        v["adoptionReceipt"]["review"]["candidateSha256"] = json!("wrong")
    });
    assert!(load(&f.package).err().unwrap().contains("独审"));
}

#[tokio::test]
async fn unsupported_schema_scope_and_path_escape_are_explicit_errors() {
    let f = Fixture::new().await;
    f.change("manifest.json", |v| {
        v["version"] = json!("series-image-queue-v1")
    });
    assert!(load(&f.package).err().unwrap().contains("尚不支持"));
    f.change("manifest.json", |v| {
        v["version"] = json!("desktop-series-delivery-package/v1");
        v["files"][0]["path"] = json!("../outside.md");
    });
    assert!(load(&f.package).err().unwrap().contains("越出"));
    let g = Fixture::new().await;
    g.change("manifest.json", |v| {
        v["productionScope"]["status"] = json!("draft")
    });
    assert!(load(&g.package).err().unwrap().contains("未锁定"));
}

#[tokio::test]
async fn absent_manifest_never_claims_complete_and_explicit_catalog_mode_remains_available() {
    let f = Fixture::new().await;
    fs::rename(
        f.package.join("manifest.json"),
        f.package.join("manifest.disabled"),
    )
    .unwrap();
    let mut request = json!({"path":f.package.join("全剧_灵墨v7.md")});
    let preview = lingmo_import::preview_file(&request).unwrap();
    assert_eq!(preview["handoff"]["level"], "md-and-catalog");
    request["handoffMode"] = json!("complete-package");
    assert!(lingmo_import::preview_file(&request)
        .unwrap_err()
        .contains("缺少 manifest"));
}
