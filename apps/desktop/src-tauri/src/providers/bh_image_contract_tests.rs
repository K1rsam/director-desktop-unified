//! Synthetic HTTP contracts only; no account, upstream generation or private input.
//! Source: SHUO BH template image executions (six exact model tokens), including
//! image2-high's constant quality_level and gpt-image-2's qualityLevel selector.
use super::*;
use crate::providers::wire_tests::fixture;

const IMAGE_MODELS: &[(&str, &[&str])] = &[
    ("gemini-3-pro-image-preview", &["1K", "2K", "4K"]),
    ("gemini-3.1-flash-image-preview", &["1K", "2K", "4K"]),
    ("image2k4k", &["2K", "4K"]),
    ("image4k", &["4K"]),
    ("image2-high", &["1K", "2K", "4K"]),
    ("gpt-image-2", &[]),
];

fn row(id: &str, resolutions: &[&str]) -> Value {
    // Actual seed shape: image rows have reference/resolutions but omit ratios.
    json!({"id":id,"category":"image","status":"active","billing":"per_image",
        "unit":"CNY","price":{"amount":0.01},"reference":{"images":9,"videos":0,"audios":0},
        "resolutions":resolutions})
}
fn catalog(rows: Vec<Value>) -> Value {
    json!({"ok":true,"catalog_version":"synthetic-image-contract-v1","count":rows.len(),"models":rows})
}
fn config(endpoint: String) -> ProviderConfig {
    let env = format!(
        "DIRECTOR_BH_IMAGE_FIXTURE_{}",
        uuid::Uuid::new_v4().simple()
    );
    std::env::set_var(&env, "synthetic-only-not-a-provider-key");
    ProviderConfig {
        kind: "bh".into(),
        endpoint: Some(endpoint),
        credential_env: Some(env),
        cli_path: None,
        bridge_token_env: None,
        chrome_binding: None,
        browser_transport: None,
        timeout_seconds: 5,
        adapted_models: vec![],
    }
}
fn input(model: &str, output: &str) -> GenerationRequest {
    GenerationRequest {
        request_id: "synthetic-image-request".into(),
        kind: "image".into(),
        model: model.into(),
        prompt: "synthetic image contract".into(),
        duration: None,
        ratio: "16:9".into(),
        resolution: output.into(),
        references: vec![],
        options: json!({"count":1}),
    }
}
fn expected_wire(model: &str, output: &str) -> Value {
    let mut wire = json!({"model":model,"prompt":"synthetic image contract","n":1,
        "size":"16:9","response_format":"url"});
    if model == "gpt-image-2" {
        wire["quality_level"] = json!(output);
    } else {
        wire["resolution"] = json!(output);
        wire["quality"] = json!(output);
        if model == "image2-high" {
            wire["quality_level"] = json!("high");
        }
    }
    wire
}

#[test]
fn real_image_seed_shape_projects_only_the_six_explicit_execution_contracts() {
    let cfg = config("http://127.0.0.1:1/v1".into());
    let caps = parse_catalog(
        &cfg,
        catalog(
            IMAGE_MODELS
                .iter()
                .map(|(id, sizes)| row(id, sizes))
                .collect(),
        ),
    )
    .unwrap();
    assert_eq!(caps.models.len(), 6);
    for model in &caps.models {
        assert!(model.adapted, "{}", model.id);
        assert_eq!(model.images, 9);
        assert_eq!(model.ratios, IMAGE_RATIOS);
        assert_eq!(
            model.raw["desktopParameterContract"]["ratiosSource"],
            "execution-manifest"
        );
        if model.id == "gpt-image-2" {
            assert_eq!(model.resolutions, IMAGE_QUALITY_LEVELS);
            assert_eq!(model.raw["resolutions"], json!([]));
            assert_eq!(
                model.raw["desktopParameterContract"]["outputParameter"],
                "quality_level"
            );
            assert_eq!(
                model.raw["desktopParameterContract"]["defaultOutput"],
                "medium"
            );
        }
    }
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn catalogue_projection_respects_explicit_limits_and_does_not_guess_model_protocols() {
    let cfg = config("http://127.0.0.1:1/v1".into());
    let mut restricted = row("image2-high", &["2K"]);
    restricted["allowed_sizes"] = json!(["9:16"]);
    let mut empty = row("image4k", &["4K"]);
    empty["ratios"] = json!([]);
    let mut wrong_kind = row("gpt-image-2", &["720P"]);
    wrong_kind["category"] = json!("video");
    wrong_kind["durations"] = json!({"min":4,"max":15});
    wrong_kind["ratios"] = json!(["16:9"]);
    let mut unknown = row("future-image2-high", &["2K"]);
    unknown["ratios"] = json!(["16:9"]);
    let caps = parse_catalog(&cfg, catalog(vec![restricted, empty, wrong_kind, unknown])).unwrap();
    assert!(caps.models[0].adapted);
    assert_eq!(caps.models[0].ratios, ["9:16"]);
    assert_eq!(caps.models[0].resolutions, ["2K"]);
    assert_eq!(
        caps.models[0].raw["desktopParameterContract"]["ratiosSource"],
        "live_catalog"
    );
    assert!(caps.models.iter().skip(1).all(|model| !model.adapted));
    let changed = parse_catalog(&cfg, catalog(vec![row("gpt-image-2", &["2K"])])).unwrap();
    assert!(
        !changed.models[0].adapted,
        "changed pixel-control protocol needs a new mapping"
    );
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn prepare_compiles_exact_wire_for_all_six_manifest_models_without_generating() {
    for (id, sizes) in IMAGE_MODELS {
        let output = if *id == "gpt-image-2" {
            "medium"
        } else {
            sizes[0]
        };
        let f = fixture(vec![catalog(vec![row(id, sizes)])]);
        let cfg = config(f.endpoint.clone());
        let request = input(id, output);
        let prepared = super::super::prepare(&cfg, &request).unwrap();
        assert_eq!(prepared.wire, expected_wire(id, output), "{id}");
        assert_eq!(
            serde_json::to_value(&prepared.request).unwrap(),
            serde_json::to_value(&request).unwrap()
        );
        let reopened: PreparedRequest =
            serde_json::from_value(serde_json::to_value(&prepared).unwrap()).unwrap();
        assert_eq!(reopened.wire, prepared.wire);
        f.thread.join().unwrap();
        let sent = f.requests.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].0.starts_with("GET /v1/models/catalog "));
        std::env::remove_var(cfg.credential_env.unwrap());
    }
}

#[test]
fn gpt_quality_levels_are_not_sent_as_pixel_resolution_or_generic_quality() {
    for level in IMAGE_QUALITY_LEVELS {
        let f = fixture(vec![catalog(vec![row("gpt-image-2", &[])])]);
        let cfg = config(f.endpoint.clone());
        let prepared = super::super::prepare(&cfg, &input("gpt-image-2", level)).unwrap();
        assert_eq!(prepared.wire, expected_wire("gpt-image-2", level));
        assert!(prepared.wire.get("resolution").is_none());
        assert!(prepared.wire.get("quality").is_none());
        f.thread.join().unwrap();
        std::env::remove_var(cfg.credential_env.unwrap());
    }
}

#[test]
fn incompatible_gpt_pixel_preset_requires_selection_before_upload_or_generation() {
    let f = fixture(vec![catalog(vec![row("gpt-image-2", &[])])]);
    let cfg = config(f.endpoint.clone());
    let error = super::super::prepare(&cfg, &input("gpt-image-2", "2K")).unwrap_err();
    assert_eq!(error.code, "OUTPUT_CAPACITY");
    assert!(!error.submission_unknown);
    f.thread.join().unwrap();
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn new_image_wire_keeps_frozen_reference_order_and_complete_signed_urls() {
    let first = "__FIXTURE_ORIGIN__/first.png?Signature=first%2Bsignature&Expires=4102444800";
    let second = "__FIXTURE_ORIGIN__/second.png?Signature=second%2Fsignature&Expires=4102444800";
    let f = fixture(vec![
        catalog(vec![row("image2-high", &["2K"])]),
        json!({"url":first,"kind":"image"}),
        json!({"__rawBody":b"synthetic first image bytes".to_vec()}),
        json!({"url":second,"kind":"image"}),
        json!({"__rawBody":b"synthetic second image bytes".to_vec()}),
    ]);
    let cfg = config(f.endpoint.clone());
    let dir = tempfile::tempdir().unwrap();
    let first = first.replace("__FIXTURE_ORIGIN__", f.endpoint.trim_end_matches("/v1"));
    let second = second.replace("__FIXTURE_ORIGIN__", f.endpoint.trim_end_matches("/v1"));
    let mut request = input("image2-high", "2K");
    for (id, order) in [("second", 1), ("first", 0)] {
        let path = dir.path().join(format!("{id}.png"));
        let bytes = format!("synthetic {id} image bytes").into_bytes();
        fs::write(&path, &bytes).unwrap();
        request.references.push(MediaReference {
            media_id: id.into(),
            kind: "image".into(),
            path,
            sha256: hex_hash(&bytes),
            order,
            usage: "reference".into(),
            duration_seconds: None,
        });
    }
    let prepared = super::super::prepare(&cfg, &request).unwrap();
    let mut expected = expected_wire("image2-high", "2K");
    expected["images"] = json!([first, second]);
    assert_eq!(prepared.wire, expected);
    assert_eq!(prepared.request.references[0].media_id, "first");
    assert_eq!(prepared.upload_receipts[0].media_id, "first");
    assert!(prepared
        .upload_receipts
        .iter()
        .all(|receipt| !receipt.upstream_read_confirmed));
    f.thread.join().unwrap();
    assert_eq!(f.requests.lock().unwrap().len(), 5);
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn previous_frozen_image_wire_is_never_silently_recompiled_during_submission() {
    let f = fixture(vec![
        catalog(vec![row("image2-high", &["2K"])]),
        json!({"data":[{"url":"https://fixture.invalid/original.png"}]}),
    ]);
    let cfg = config(f.endpoint.clone());
    let legacy_wire = json!({"model":"image2-high","prompt":"synthetic image contract",
        "n":1,"resolution":"2K","quality":"2K","size":"16:9","images":[],"response_format":"url"});
    let prepared = PreparedRequest {
        provider: "bh".into(),
        request: input("image2-high", "2K"),
        capability_version: "synthetic-image-contract-v1".into(),
        upload_receipts: vec![],
        wire: legacy_wire.clone(),
        estimated_cost: Some(0.01),
        prepared_at: now(),
    };
    let receipt = super::super::submit(&cfg, &prepared).unwrap();
    assert_eq!(receipt.task_id, "sync:synthetic-image-request");
    f.thread.join().unwrap();
    let sent = f.requests.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent[1].0.starts_with("POST /v1/images/generations "));
    assert_eq!(
        serde_json::from_str::<Value>(&sent[1].1).unwrap(),
        legacy_wire
    );
    assert_eq!(prepared.wire, legacy_wire);
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn malformed_image_submission_stays_unknown_and_has_only_one_generation_post() {
    let f = fixture(vec![
        catalog(vec![row("image2-high", &["2K"])]),
        catalog(vec![row("image2-high", &["2K"])]),
        json!("invalid-json"),
    ]);
    let cfg = config(f.endpoint.clone());
    let prepared = super::super::prepare(&cfg, &input("image2-high", "2K")).unwrap();
    let error = super::super::submit(&cfg, &prepared).unwrap_err();
    assert!(error.submission_unknown);
    f.thread.join().unwrap();
    assert_eq!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(route, _)| route.starts_with("POST /v1/images/generations "))
            .count(),
        1
    );
    std::env::remove_var(cfg.credential_env.unwrap());
}

fn with_reference(dir: &Path) -> GenerationRequest {
    let path = dir.join("synthetic.png");
    let bytes = b"synthetic reference bytes";
    fs::write(&path, bytes).unwrap();
    let mut request = input("image2-high", "2K");
    request.references.push(MediaReference {
        media_id: "synthetic-reference".into(),
        kind: "image".into(),
        path,
        sha256: hex_hash(bytes),
        order: 0,
        usage: "reference".into(),
        duration_seconds: None,
    });
    request
}

#[test]
fn signed_upload_validation_rejects_internal_arbitrary_and_expired_targets_without_reading_them() {
    let cfg = config("https://api.7tai.cc/v1".into());
    for url in [
        "http://127.0.0.1/private?Signature=x&Expires=4102444800",
        "https://example.invalid/x?Signature=x&Expires=4102444800",
        "https://fixture.oss-cn-shenzhen.aliyuncs.com.evil.invalid/x?Signature=x&Expires=4102444800",
        "https://user:private@fixture.oss-cn-shenzhen.aliyuncs.com/x?Signature=x&Expires=4102444800",
        "https://fixture.oss-cn-shenzhen.aliyuncs.com:444/x?Signature=x&Expires=4102444800",
        "https://fixture.oss-cn-shenzhen.aliyuncs.com/x?Signature=x&Expires=4102444800&Expires=1",
        "https://fixture.oss-cn-shenzhen.aliyuncs.com/x?Signature=x&Expires=1",
        "https://fixture.oss-cn-shenzhen.aliyuncs.com/x?x-oss-signature=x&x-oss-date=20200101T000000Z&x-oss-expires=60",
        "https://fixture.oss-cn-shenzhen.aliyuncs.com/x?x-oss-signature=x",
    ] {assert!(upload_url(&cfg, url).is_err(), "unexpected accepted target");}
    assert!(upload_url(
        &cfg,
        "https://fixture.oss-cn-shenzhen.aliyuncs.com/x?Signature=x&Expires=4102444800"
    )
    .is_ok());
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn uploaded_hash_mismatch_stops_before_generation_and_upload_contains_model() {
    let f = fixture(vec![
        catalog(vec![row("image2-high", &["2K"])]),
        json!({"url":"__FIXTURE_ORIGIN__/wrong.png?Signature=synthetic&Expires=4102444800","kind":"image"}),
        json!({"__rawBody":b"XXXXXXXXXXXXXXXXXXXXXXXXX".to_vec()}),
    ]);
    let cfg = config(f.endpoint.clone());
    let dir = tempfile::tempdir().unwrap();
    let error = super::super::prepare(&cfg, &with_reference(dir.path())).unwrap_err();
    assert_eq!(error.code, "UPLOAD_READBACK_MISMATCH");
    assert!(!error.submission_unknown);
    f.thread.join().unwrap();
    let sent = f.requests.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert!(sent[1].1.contains("name=\"model\""));
    assert!(sent[1].1.contains("image2-high"));
    assert!(sent[2].0.starts_with("GET /wrong.png?"));
    let headers = f.headers.lock().unwrap();
    assert!(headers[1]
        .to_ascii_lowercase()
        .contains("authorization: bearer synthetic-only-not-a-provider-key"));
    assert!(!headers[2].to_ascii_lowercase().contains("authorization:"));
    assert!(!headers[2].to_ascii_lowercase().contains("cookie:"));
    assert!(!sent
        .iter()
        .any(|(route, _)| route.contains("/images/generations")));
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn untrusted_uploaded_url_never_causes_a_readback_get() {
    let f = fixture(vec![
        catalog(vec![row("image2-high", &["2K"])]),
        json!({"url":"https://example.invalid/private?Signature=x&Expires=4102444800","kind":"image"}),
    ]);
    let cfg = config(f.endpoint.clone());
    let dir = tempfile::tempdir().unwrap();
    let error = super::super::prepare(&cfg, &with_reference(dir.path())).unwrap_err();
    assert_eq!(error.code, "UPLOAD_URL_INVALID");
    f.thread.join().unwrap();
    assert_eq!(f.requests.lock().unwrap().len(), 2);
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn proof_survives_reopen_and_missing_changed_or_expired_proof_cannot_submit() {
    let f = fixture(vec![
        catalog(vec![row("image2-high", &["2K"])]),
        json!({"url":"__FIXTURE_ORIGIN__/reference.png?Signature=synthetic&Expires=4102444800","kind":"image"}),
        json!({"__rawBody":b"synthetic reference bytes".to_vec()}),
    ]);
    let cfg = config(f.endpoint.clone());
    let dir = tempfile::tempdir().unwrap();
    let original = super::super::prepare(&cfg, &with_reference(dir.path())).unwrap();
    let original_wire = original.wire.clone();
    let reopened: PreparedRequest =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    validate_upload_proofs(&cfg, &reopened).unwrap();
    assert_eq!(
        reopened.upload_receipts[0].raw["localReadbackVerified"],
        true
    );
    assert!(!reopened.upload_receipts[0].upstream_read_confirmed);
    for mutation in ["missing", "hash", "model", "url", "expired"] {
        let mut changed = reopened.clone();
        let receipt = &mut changed.upload_receipts[0];
        match mutation {
            "missing" => receipt.raw = Value::Null,
            "hash" => receipt.raw["sha256"] = json!("changed"),
            "model" => receipt.raw["model"] = json!("gpt-image-2"),
            "url" => {
                receipt.url = Some(
                    receipt
                        .url
                        .as_ref()
                        .unwrap()
                        .replace("reference.png", "other.png"),
                )
            }
            "expired" => {
                receipt.url = Some(receipt.url.as_ref().unwrap().replace("4102444800", "1"))
            }
            _ => unreachable!(),
        }
        assert!(
            validate_upload_proofs(&cfg, &changed).is_err(),
            "{mutation}"
        );
        assert_eq!(changed.wire, original_wire);
    }
    f.thread.join().unwrap();
    assert_eq!(f.requests.lock().unwrap().len(), 3);
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn legacy_upload_receipt_roundtrip_omits_new_empty_proof_and_preserves_hash() {
    let old = json!({"mediaId":"synthetic","sha256":"same","order":0,"kind":"image","url":null,
        "fileId":null,"uploadedAt":"2026-10-20T00:00:00Z","upstreamReadConfirmed":false});
    let old_hash = hex_hash(old.to_string().as_bytes());
    let receipt: UploadReceipt = serde_json::from_value(old.clone()).unwrap();
    assert!(receipt.raw.is_null());
    let saved = serde_json::to_value(receipt).unwrap();
    assert_eq!(saved, old);
    assert_eq!(hex_hash(saved.to_string().as_bytes()), old_hash);
}

#[test]
fn known_task_success_alias_preserves_raw_receipt_and_never_submits() {
    let result = "https://fixture.invalid/original.mp4?signature=preserved%2Fvalue";
    let f = fixture(vec![
        json!({"task_id":"synthetic-original","status":"success","result_url":result}),
    ]);
    let cfg = config(f.endpoint.clone());
    let response = super::super::query(&cfg, "synthetic-original").unwrap();
    assert_eq!(response.status, "succeeded");
    assert_eq!(response.result_urls, [result]);
    assert_eq!(response.raw["status"], "success");
    f.thread.join().unwrap();
    let sent = f.requests.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .0
        .starts_with("GET /v1/video/generations/synthetic-original "));
    std::env::remove_var(cfg.credential_env.unwrap());
}

#[test]
fn frame_mode_rejects_ordinary_images_audio_and_video_before_any_upload() {
    for kind in ["image", "audio", "video"] {
        let model = json!({"id":"bh2.0-720p","status":"active","kind":"video","billing":"per_second","unit":"CNY",
            "price":{"amount":0.5},"durations":{"min":4,"max":15},"allowed_sizes":["16:9"],"resolutions":["720P"],
            "reference":{"images":9,"videos":3,"audios":3},"supports_first_last_frame":true});
        let f = fixture(vec![catalog(vec![model])]);
        let cfg = config(f.endpoint.clone());
        let dir = tempfile::tempdir().unwrap();
        let mut request = input("bh2.0-720p", "720P");
        request.kind = "video".into();
        request.duration = Some(5);
        request.options = json!({});
        for (id, reference_kind, usage) in [
            ("first", "image", "first_frame"),
            ("mixed", kind, "reference"),
        ] {
            let path = dir.path().join(format!("{id}.fixture"));
            let bytes = format!("synthetic {id} media").into_bytes();
            fs::write(&path, &bytes).unwrap();
            request.references.push(MediaReference {
                media_id: id.into(),
                kind: reference_kind.into(),
                path,
                sha256: hex_hash(&bytes),
                order: request.references.len() as u32,
                usage: usage.into(),
                duration_seconds: None,
            });
        }
        let error = super::super::prepare(&cfg, &request).unwrap_err();
        assert_eq!(error.code, "FRAME_MODE", "{kind}");
        assert!(!error.submission_unknown);
        f.thread.join().unwrap();
        assert_eq!(f.requests.lock().unwrap().len(), 1);
        std::env::remove_var(cfg.credential_env.unwrap());
    }
}
