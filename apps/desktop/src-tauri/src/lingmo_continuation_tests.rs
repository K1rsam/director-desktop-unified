use super::*;
use crate::{lingmo_import, store::Store};
use std::{fs, path::Path};

const TEXT: &str = "第1集\n1-1\n场景：门口\n人物：甲、甲礼服\n声音：风声\n片段1\n时长：4秒\n@[门口SC] @[甲CH] @[甲礼服CH] @[风声SE]\n分镜1｜0-2.7s｜2.7s\n画面：甲开门。\n";

fn catalog(text: &str) -> Value {
    json!({"assetRegistryId":"fixture-registry","sourceSha256":"fixed-script-source",
        "markdownSha256":digest(text),"assets":[
        {"id":"SC1","name":"门口","type":"SC","token":"门口SC","assetRevision":3,"filename":"门口SC.png"},
        {"id":"CH1","variantId":"external-base","identityId":"person-a","name":"甲","type":"CH","token":"甲CH","assetRevision":7,"filename":"甲CH.png",
         "adoptedDesign":{"schemaVersion":"series-full-design/v1","assetId":"CH1","variantId":"external-base","designRevision":23,"status":"ADOPTED","full":"原完整设计","adoptionBasis":{"receipt":"original-approved"}}},
        {"id":"CH2","variantId":"external-costume","identityId":"person-a","name":"甲礼服","type":"CH","token":"甲礼服CH","assetRevision":2,"filename":"甲礼服CH.png","baseAssetId":"CH1",
         "adoptedDesign":{"schemaVersion":"series-full-design/v1","assetId":"CH2","variantId":"external-costume","designRevision":8,"status":"ADOPTED","full":"礼服完整设计","designDependencies":[{"assetId":"CH1","variantId":"external-base","assetRevision":7,"designRevision":23}]},
         "referenceRequirements":[{"kind":"face","targetAssetId":"CH1","targetVariantId":"external-base","targetAssetRevision":7}]},
        {"id":"SE1","name":"风声","type":"SE","token":"风声SE","assetRevision":4}
        ],"units":[]})
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store.create("导入接续验收", "continuation").unwrap();
        Self { dir, store }
    }
    fn request(&self, text: &str, catalog: &Value) -> Value {
        let path = self.dir.path().join("fixture.assets.json");
        fs::write(&path, serde_json::to_vec(catalog).unwrap()).unwrap();
        json!({"text":text,"catalogPath":path,"expectedSha256":digest(text),"allowNewRevision":true})
    }
    fn import(&self, text: &str, catalog: &Value) -> Value {
        lingmo_import::import(&self.store, &self.request(text, catalog)).unwrap()
    }
    fn asset(&self, external: &str) -> Value {
        self.store
            .values("assets")
            .unwrap()
            .into_iter()
            .find(|a| a["externalId"] == external)
            .unwrap()
    }
    /// Materialize the installed legacy shape: parser rows + immutable original
    /// existed, but the new handoff fields and alias receipt did not.
    fn make_legacy(&self) {
        let connection = self.store.conn().unwrap();
        for mut asset in self.store.values("assets").unwrap() {
            for field in [
                "handoffRegistry",
                "handoffAsset",
                "handoffValidation",
                "adoptedDesign",
                "designRevision",
                "designDependencies",
                "referenceRequirements",
                "identityId",
                "baseAssetId",
            ] {
                asset.as_object_mut().unwrap().remove(field);
            }
            connection
                .execute(
                    "UPDATE assets SET data=?1 WHERE id=?2",
                    params![asset.to_string(), s(&asset, "id")],
                )
                .unwrap();
        }
        connection
            .execute(
                "DELETE FROM metadata WHERE key LIKE 'lingmo.asset-identities:%'",
                [],
            )
            .unwrap();
    }
}

fn image(path: &Path, seed: u8) {
    image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([seed, x as u8 * 23, y as u8 * 23]))
        .save(path)
        .unwrap();
}

#[test]
fn legacy_import_save_reopen_revision_preserves_images_edits_raw_and_unknown() {
    let f = Fixture::new();
    let old_catalog = catalog(TEXT);
    f.import(TEXT, &old_catalog);
    f.make_legacy();
    let mut asset = f.asset("CH1");
    asset["expectedVersion"] = asset["version"].clone();
    asset["prompt"] = json!("用户手工美术提示词，必须保留");
    asset["filename"] = json!("用户改过的显示文件名.png");
    asset["notes"] = json!({"human":"原图已审核"});
    let saved = f.store.save_asset(asset).unwrap();
    let path = f.dir.path().join("original.png");
    image(&path, 13);
    let media = f
        .store
        .import_media(
            &path,
            "image",
            Some(s(&saved, "id")),
            Some(s(&saved, "variantId")),
        )
        .unwrap();
    f.store
        .adopt(s(&saved, "id"), s(&saved, "variantId"), s(&media, "id"))
        .unwrap();
    let saved = f.store.get("assets", s(&saved, "id")).unwrap();
    let original_bytes = fs::read(s(&media, "path")).unwrap();
    let old_unit = f.store.values("units").unwrap().remove(0);
    let mut edit = old_unit.clone();
    edit["expectedVersion"] = edit["version"].clone();
    edit["prompt"] = json!(s(&edit, "prompt").replace("甲开门。", "甲缓缓推门。"));
    let edited = lingmo_import::save_unit(&f.store, &old_unit, edit).unwrap();
    let job = f
        .store
        .add_job(
            "image",
            s(&saved, "id"),
            "offline",
            "original-account",
            &json!({"assetId":saved["id"],"variantId":saved["variantId"],"frozen":"unchanged"}),
            "submission_unknown",
        )
        .unwrap();
    let frozen = f.store.job(&job).unwrap();
    let original: String = f
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("lingmo.original:{}", digest(TEXT))],
            |r| r.get(0),
        )
        .unwrap();
    let reopened = Store::open(f.store.root.clone()).unwrap();
    let next_text = TEXT.replace("甲开门。", "甲走出门口。");
    let result =
        lingmo_import::import(&reopened, &f.request(&next_text, &catalog(&next_text))).unwrap();
    assert_eq!(result["generationSubmissions"], 0);
    assert_eq!(reopened.values("assets").unwrap().len(), 4);
    assert_eq!(reopened.get("assets", s(&saved, "id")).unwrap(), saved);
    assert_eq!(reopened.get("units", s(&edited, "id")).unwrap(), edited);
    assert_eq!(reopened.job(&job).unwrap(), frozen);
    assert_eq!(
        reopened
            .adopted(s(&saved, "id"), s(&saved, "variantId"))
            .unwrap()
            .as_deref(),
        Some(s(&media, "id"))
    );
    assert_eq!(fs::read(s(&media, "path")).unwrap(), original_bytes);
    let still_original: String = reopened
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("lingmo.original:{}", digest(TEXT))],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(still_original, original);
    let new_unit = reopened
        .values("units")
        .unwrap()
        .into_iter()
        .find(|u| u["importSource"]["markdownSha256"] == digest(&next_text))
        .unwrap();
    assert!(s(&new_unit, "prompt").contains("甲走出门口。"));
    assert_eq!(new_unit["assetRefs"][1]["assetId"], saved["id"]);
    assert_eq!(new_unit["assetRefs"][1]["variantId"], saved["variantId"]);
    let audio = f.asset("SE1");
    assert_eq!(new_unit["audioAssetRefs"][0]["assetId"], audio["id"]);
    assert_eq!(
        new_unit["audioAssetRefs"][0]["variantId"],
        audio["variantId"]
    );
    assert!(lingmo_import::compile_prompt(&new_unit, &reopened.values("assets").unwrap()).is_ok());
    for mention in new_unit["mentions"].as_array().unwrap() {
        let a = reopened.get("assets", s(mention, "assetId")).unwrap();
        assert_eq!(mention["variantId"], a["variantId"]);
        let start = mention["start"].as_u64().unwrap() as usize;
        let end = mention["end"].as_u64().unwrap() as usize;
        assert_eq!(&s(&new_unit, "prompt")[start..end], s(mention, "token"));
    }
    let document = lingmo_import::document(&reopened, &digest(&next_text)).unwrap();
    assert!(document["scenes"][0]["assetIds"]
        .as_array()
        .unwrap()
        .contains(&saved["id"]));
    let twice = Store::open(reopened.root.clone()).unwrap();
    assert_eq!(twice.get("assets", s(&saved, "id")).unwrap(), saved);
    assert_eq!(
        lingmo_import::original(&twice, &digest(&next_text), None).unwrap()["text"],
        next_text
    );
}

#[test]
fn same_version_catalog_tampering_rolls_back_without_touching_prior_import() {
    let f = Fixture::new();
    f.import(TEXT, &catalog(TEXT));
    f.make_legacy();
    let before = f.store.values("assets").unwrap();
    let old_units = f.store.values("units").unwrap();
    let next = TEXT.replace("甲开门。", "甲回头。");
    let mut changed = catalog(&next);
    changed["assets"][1]["adoptedDesign"]["full"] = json!("同资产版本偷偷改设计");
    let error = lingmo_import::import(&f.store, &f.request(&next, &changed)).unwrap_err();
    assert!(error.contains("相同资产版本"), "{error}");
    assert_eq!(f.store.values("assets").unwrap(), before);
    assert_eq!(f.store.values("units").unwrap(), old_units);
    assert_eq!(lingmo_import::documents(&f.store).unwrap().len(), 1);
    assert!(f.store.job_summaries().unwrap().is_empty());
    let missing: i64 = f
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM metadata WHERE key=?1",
            [format!("{MAP_PREFIX}{}", digest(&next))],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(missing, 0);
}

#[test]
fn different_registry_or_asset_revision_never_reuses_equal_names() {
    let f = Fixture::new();
    f.import(TEXT, &catalog(TEXT));
    f.make_legacy();
    let old = f.asset("CH1");
    let next = TEXT.replace("甲开门。", "甲出门。");
    let mut revised = catalog(&next);
    revised["assets"][1]["assetRevision"] = json!(8);
    let result = f.import(&next, &revised);
    assert_eq!(f.store.values("assets").unwrap().len(), 5);
    let scene = lingmo_import::scene(
        &f.store,
        s(&result, "documentId"),
        s(
            &lingmo_import::document(&f.store, s(&result, "documentId")).unwrap()["scenes"][0],
            "id",
        ),
    )
    .unwrap();
    let new_ref = &scene["units"][0]["assetRefs"][1];
    assert_ne!(new_ref["assetId"], old["id"]);
    assert_ne!(new_ref["variantId"], old["variantId"]);
    assert_eq!(f.store.get("assets", s(&old, "id")).unwrap(), old);
    let unrelated = TEXT.replace("甲开门。", "甲回来。");
    let mut other = catalog(&unrelated);
    other["assetRegistryId"] = json!("another-registry");
    f.import(&unrelated, &other);
    assert_eq!(f.store.values("assets").unwrap().len(), 9);
}

#[test]
fn design_revisions_are_checked_in_their_own_domain_and_raw_design_is_retained() {
    let f = Fixture::new();
    let source = catalog(TEXT);
    f.import(TEXT, &source);
    let base = f.asset("CH1");
    let child = f.asset("CH2");
    assert_eq!(base["assetRevision"], 7);
    assert_eq!(base["designRevision"], 23);
    assert_eq!(child["handoffValidation"]["issues"], json!([]));
    assert_eq!(
        child["adoptedDesign"]["designDependencies"][0]["assetId"],
        base["id"]
    );
    assert_eq!(
        child["adoptedDesign"]["designDependencies"][0]["variantId"],
        base["variantId"]
    );
    assert_eq!(
        child["adoptedDesign"]["designDependencies"][0]["designRevision"],
        23
    );
    assert_eq!(
        child["referenceRequirements"][0]["targetAssetId"],
        base["id"]
    );
    assert_eq!(
        child["referenceRequirements"][0]["targetVariantId"],
        base["variantId"]
    );
    assert_eq!(child["handoffAsset"], source["assets"][2]);
    let bad = Fixture::new();
    let mut mismatch = catalog(TEXT);
    mismatch["assets"][2]["adoptedDesign"]["designDependencies"][0]["designRevision"] = json!(7);
    bad.import(TEXT, &mismatch);
    let child = bad.asset("CH2");
    assert_eq!(child["promptStatus"], "needs_review");
    assert!(child["handoffValidation"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue.as_str().unwrap().contains("设计版本不符")));
}

#[test]
fn nested_reference_assets_and_variants_map_without_touching_original_fields() {
    let assets = HashMap::from([("new-asset".into(), json!("retained-asset"))]);
    let variants = HashMap::from([("new-variant".into(), json!("retained-variant"))]);
    let mut unit = json!({"prompt":"new-asset", "filename":"new-asset", "token":"@[new-assetCH]", "byteStart":15,
        "importSource":{"assetId":"new-asset","variantId":"new-variant","byteStart":2},
        "assetRefs":[{"assetId":"new-asset","variantId":"new-variant"}],
        "nested":{"audioAssetRefs":[{"assetId":"new-asset","variantId":"new-variant","token":"new-variant"}],
            "imageBindings":{"shots":[{"assetId":"new-asset","variantId":"new-variant","text":"new-asset"}]}},
        "mentions":[{"assetId":"new-asset","variantId":"new-variant","token":"@[new-assetCH]","start":2,"end":17}],
        "assetIds":["new-asset"]});
    let source = unit.clone();
    replace_ids(&mut unit, &assets, &variants);
    for pointer in [
        "/assetRefs/0",
        "/nested/audioAssetRefs/0",
        "/nested/imageBindings/shots/0",
        "/mentions/0",
    ] {
        let reference = unit.pointer(pointer).unwrap();
        assert_eq!(reference["assetId"], "retained-asset");
        assert_eq!(reference["variantId"], "retained-variant");
    }
    assert_eq!(unit["assetIds"], json!(["retained-asset"]));
    for key in ["prompt", "filename", "token", "byteStart", "importSource"] {
        assert_eq!(unit[key], source[key]);
    }
    assert_eq!(unit["mentions"][0]["token"], source["mentions"][0]["token"]);
    assert_eq!(unit["mentions"][0]["start"], 2);
    assert_eq!(unit["mentions"][0]["end"], 17);
    assert_eq!(
        unit["nested"]["imageBindings"]["shots"][0]["text"],
        "new-asset"
    );
}

#[test]
fn continued_revision_duplicate_backfill_and_folder_preview_use_retained_identity() {
    let f = Fixture::new();
    f.import(TEXT, &catalog(TEXT));
    f.make_legacy();
    let mut base = f.asset("CH1");
    base["filename"] = json!("用户另起的文件名.png");
    base = f.store.save_asset(base).unwrap();
    let old_path = f.dir.path().join("old.png");
    image(&old_path, 19);
    let old_media = f
        .store
        .import_media(
            &old_path,
            "image",
            Some(s(&base, "id")),
            Some(s(&base, "variantId")),
        )
        .unwrap();
    f.store
        .adopt(s(&base, "id"), s(&base, "variantId"), s(&old_media, "id"))
        .unwrap();
    base = f.store.get("assets", s(&base, "id")).unwrap();
    let directory = f.dir.path().join("images");
    fs::create_dir(&directory).unwrap();
    image(&directory.join("甲CH.png"), 29);
    let next = TEXT.replace("甲开门。", "甲拉开门。");
    let mut request = f.request(&next, &catalog(&next));
    request["imageDirectory"] = json!(directory);
    let first = lingmo_import::import(&f.store, &request).unwrap();
    assert_eq!(first["media"]["imported"], 1);
    assert_eq!(first["media"]["failures"], json!([]));
    assert_eq!(f.store.get("assets", s(&base, "id")).unwrap(), base);
    let reopened = Store::open(f.store.root.clone()).unwrap();
    image(&directory.join("甲CH.png"), 39);
    let duplicate = lingmo_import::import(&reopened, &request).unwrap();
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(duplicate["media"]["imported"], 1);
    assert_eq!(duplicate["media"]["failures"], json!([]));
    assert_eq!(reopened.values("assets").unwrap().len(), 4);
    assert_eq!(reopened.values("media").unwrap().len(), 3);
    let links: i64 = reopened
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM media_assets WHERE asset_id=?1 AND variant_id=?2",
            params![s(&base, "id"), s(&base, "variantId")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(links, 3);
    assert_eq!(
        reopened
            .adopted(s(&base, "id"), s(&base, "variantId"))
            .unwrap()
            .as_deref(),
        Some(s(&old_media, "id"))
    );
    assert_eq!(reopened.get("assets", s(&base, "id")).unwrap(), base);
    let preview = crate::lingmo_media::preview(
        &reopened,
        &json!({"documentId":digest(&next),"directory":directory}),
    )
    .unwrap();
    assert_eq!(preview["counts"]["files"], 1);
    assert_eq!(preview["counts"]["matched"], 1);
    let latest =
        crate::lingmo_media::latest(&reopened, &json!({"documentId":digest(&next)})).unwrap();
    assert_eq!(latest["assetCount"], 4);
    assert_eq!(latest["fileCount"], 1);
    assert_eq!(preview["rows"][0]["assetId"], base["id"]);
    assert_eq!(preview["rows"][0]["variantId"], base["variantId"]);
    assert!(reopened.job_summaries().unwrap().is_empty());
}

#[test]
fn corrupted_original_identity_evidence_is_rejected_before_any_revision_write() {
    let f = Fixture::new();
    f.import(TEXT, &catalog(TEXT));
    f.make_legacy();
    let before = f.store.values("assets").unwrap();
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE metadata SET value=json_set(value,'$.text','corrupted') WHERE key=?1",
            [format!("lingmo.original:{}", digest(TEXT))],
        )
        .unwrap();
    let next = TEXT.replace("甲开门。", "甲走过门口。");
    let error = lingmo_import::import(&f.store, &f.request(&next, &catalog(&next))).unwrap_err();
    assert!(error.contains("不可变原件"), "{error}");
    assert_eq!(f.store.values("assets").unwrap(), before);
    assert_eq!(lingmo_import::documents(&f.store).unwrap().len(), 1);
}
