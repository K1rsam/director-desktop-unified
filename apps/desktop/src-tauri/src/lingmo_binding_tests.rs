use super::*;
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store.create("offline imported binding", "p").unwrap();
        Self { dir, store }
    }

    fn asset(&self, key: &str, lingmo_type: &str) -> Value {
        self.store
            .save_asset(
                json!({"id":key,"variantId":"v-original","name":"同名不决定身份",
            "kind":if lingmo_type == "SE" {"audio"} else {"other"},"lingmoType":lingmo_type,
            "preparation":{"mode":if lingmo_type == "SE" {"AUDIO"} else {"REFERENCE"}}}),
            )
            .unwrap()
    }

    fn image(&self, asset: &Value) -> Value {
        let path = self.dir.path().join(format!("{}.png", s(asset, "id")));
        image::RgbImage::from_pixel(2, 2, image::Rgb([30, 60, 90]))
            .save(&path)
            .unwrap();
        let media = self
            .store
            .import_media(
                &path,
                "image",
                Some(s(asset, "id")),
                Some(s(asset, "variantId")),
            )
            .unwrap();
        self.store
            .adopt(s(asset, "id"), s(asset, "variantId"), s(&media, "id"))
            .unwrap();
        media
    }

    fn wav(&self, asset: &Value) -> Value {
        // A real one-second mono PCM WAV keeps the test independent of providers.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36u32 + 16000).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.resize(16044, 0);
        let path = self.dir.path().join("offline.wav");
        std::fs::write(&path, bytes).unwrap();
        self.store
            .import_media(
                &path,
                "audio",
                Some(s(asset, "id")),
                Some(s(asset, "variantId")),
            )
            .unwrap()
    }
}

#[test]
fn imported_placeholder_flag_and_original_placeholder_identity_both_block() {
    let asset = json!({"id":"a","importedPlaceholderMediaId":"placeholder"});
    assert!(placeholder_media(&asset, &json!({"id":"placeholder"})));
    assert!(placeholder_media(
        &asset,
        &json!({"id":"new","placeholder":true})
    ));
    assert!(!placeholder_media(&asset, &json!({"id":"real"})));
    assert!(!placeholder_media(&json!({}), &json!({})));
}

#[test]
fn om_remains_other_domain_but_a_real_image_uses_an_image_slot() {
    let fixture = Fixture::new();
    let asset = fixture.asset("overlay", "OM");
    fixture.image(&asset);
    let bound = bind_imported_media(&fixture.store, &asset, &json!({}), 2).unwrap();
    assert_eq!(bound["kind"], "image");
    assert_eq!(bound["lingmoType"], "OM");
    assert_eq!(bound["bindingSource"], "current-adoption");
    assert_eq!(
        fixture.store.active_asset("overlay").unwrap()["kind"],
        "other"
    );
}

#[test]
fn a_sound_label_never_silently_uses_an_adopted_image() {
    let fixture = Fixture::new();
    let asset = fixture.asset("sound", "SE");
    fixture.image(&asset);
    assert!(bind_imported_media(&fixture.store, &asset, &json!({}), 0)
        .unwrap_err()
        .contains("SE"));
}

#[test]
fn imported_placeholder_identity_blocks_and_new_real_adoption_restores_binding() {
    let fixture = Fixture::new();
    let mut asset = fixture.asset("other", "OM");
    let first = fixture.image(&asset);
    asset["importedPlaceholderMediaId"] = first["id"].clone();
    assert!(bind_imported_media(&fixture.store, &asset, &json!({}), 0)
        .unwrap_err()
        .contains("占位"));
    let path = fixture.dir.path().join("real.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([99, 80, 70]))
        .save(&path)
        .unwrap();
    let real = fixture
        .store
        .import_media(&path, "image", Some("other"), Some("v-original"))
        .unwrap();
    fixture
        .store
        .adopt("other", "v-original", s(&real, "id"))
        .unwrap();
    assert_eq!(
        bind_imported_media(&fixture.store, &asset, &json!({}), 0).unwrap()["mediaId"],
        real["id"]
    );
}

#[test]
fn sound_uses_explicit_media_identity_and_audio_marker_without_image_adoption() {
    let fixture = Fixture::new();
    let asset = fixture.asset("sound", "SE");
    let audio = fixture.wav(&asset);
    // If the host cannot probe audio metadata, binding must stay blocked.
    let bound = bind_imported_media(&fixture.store, &asset, &json!({"mediaId":audio["id"]}), 0);
    if audio["metadataVerified"] != true {
        assert!(bound.unwrap_err().contains("时长"));
        return;
    }
    let bound = bound.unwrap();
    assert_eq!(bound["kind"], "audio");
    assert_eq!(bound["bindingSource"], "unit-media-ref");
    assert!(fixture
        .store
        .adopted("sound", "v-original")
        .unwrap()
        .is_none());
    let prompt = compile_bound_prompt(
        &json!({"assetRefs":[],"prompt":"雷声{{asset:sound:v-original}}后回头"}),
        &[asset],
        &[bound],
        &[],
    )
    .unwrap();
    assert_eq!(prompt, "雷声@音频1后回头");
}

#[test]
fn normal_workflow_still_requires_real_stage_one_source() {
    let fixture = Fixture::new();
    let snapshot = bind(
        &fixture.store,
        &json!({"id":"unit","version":1,"prompt":"完整文字", "duration":5,"assetRefs":[]}),
        "bh",
        "offline",
    )
    .unwrap();
    assert!(snapshot["problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["code"] == "MISSING_SOURCE"));
}

#[test]
fn forged_external_origin_cannot_bypass_the_saved_markdown_source_contract() {
    let fixture = Fixture::new();
    assert!(bind(&fixture.store, &json!({"id":"unit","origin":"imported-storyboard","version":1,
        "prompt":"伪造外部来源","duration":5,"assetRefs":[],"importSource":{"markdownSha256":"fake","byteStart":0,"byteEnd":18}}), "bh", "offline").is_err());
}

#[test]
fn real_saved_import_binds_without_stage_one_and_frozen_provenance_is_rechecked() {
    let fixture = Fixture::new();
    let text = "第1集\n1-1\n场景：阳台\n片段1\n时长：4秒\n[场景] @[阳台SC]\n分镜1｜0-4s｜4s\n画面：阳台原文完整保留。\n";
    let mut request = json!({"text":text});
    let preview = crate::lingmo_import::preview(&fixture.store, &request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    crate::lingmo_import::import(&fixture.store, &request).unwrap();
    let asset = fixture.store.values("assets").unwrap().remove(0);
    fixture.image(&asset);
    let unit = fixture.store.values("units").unwrap().remove(0);
    assert!(fixture.store.source().unwrap().is_null());
    let snapshot = bind(&fixture.store, &unit, "bh", "offline").unwrap();
    assert_eq!(snapshot["problems"], json!([]), "{snapshot}");
    assert!(s(&snapshot, "prompt").contains("@图片1"));
    assert_eq!(snapshot["sourcePrompt"], unit["prompt"]);
    assert_eq!(snapshot["importSource"], unit["importSource"]);
    validate_snapshot(&fixture.store, &snapshot).unwrap();
    let mut corrupted_snapshot = snapshot.clone();
    corrupted_snapshot["importSource"]["byteStart"] = json!(999);
    assert!(validate_snapshot(&fixture.store, &corrupted_snapshot)
        .unwrap_err()
        .contains("来源"));
    fixture
        .store
        .conn()
        .unwrap()
        .execute(
            "DELETE FROM metadata WHERE key LIKE 'lingmo.original:%'",
            [],
        )
        .unwrap();
    assert!(validate_snapshot(&fixture.store, &snapshot).is_err());
}
