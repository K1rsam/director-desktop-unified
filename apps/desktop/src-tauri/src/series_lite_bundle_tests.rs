use super::*;
#[test]
fn exact_time() {
    assert_eq!(
        time_ticks(&json!(2.123456789), true, "DURATION").unwrap(),
        2_123_456_789
    );
    assert_eq!(time_text_ticks(2_123_456_789), "2.123456789");
    assert!(time_ticks(&json!(0.1234567891), true, "DURATION").is_err());
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("series_lite_bundle_golden.json")).unwrap()
}
struct OracleGate<'a> {
    case: &'a Value,
}
impl BundleGate for OracleGate<'_> {
    fn check(&mut self, raw: &[u8]) -> Result<GateEvidence> {
        need(
            raw == self.case["raw"].as_str().unwrap().as_bytes(),
            "ORACLE_GATE",
            "unexpected input",
        )?;
        if self.case.get("report").is_none() {
            return Err(format!(
                "{}: {}",
                self.case["error"]["code"].as_str().unwrap(),
                self.case["error"]["message"]
            ));
        }
        Ok(GateEvidence {
            bundle_sha256: hash(raw),
            normalized_sources: self.case["sources"].as_array().unwrap().clone(),
            report: self.case["report"].clone(),
            validator_id: "test-only-frozen-node-exact-input".into(),
        })
    }
}
#[test]
fn original_node_delivery_bytes_manifest_and_rejections() {
    let f = fixture();
    assert_eq!(f["verifiedSources"], 24);
    for c in f["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let raw = c["raw"].as_str().unwrap().as_bytes();
        let result = export_lingmo_v7(raw, &mut OracleGate { case: c });
        if let Some(error) = c.get("error") {
            let err = result.expect_err(name);
            if name != "legacy-profile" {
                assert!(
                    err.starts_with(error["code"].as_str().unwrap()),
                    "{name}: {err}; expected {error}"
                );
            }
        } else {
            let out = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                out.markdown,
                c["markdown"].as_str().unwrap(),
                "{name}: markdown"
            );
            assert_eq!(
                out.markdown_sha256,
                c["markdownSha256"].as_str().unwrap(),
                "{name}: markdown hash"
            );
            assert_eq!(
                out.manifest_json,
                c["manifest"].as_str().unwrap(),
                "{name}: exact manifest bytes"
            );
            assert_eq!(
                out.manifest_sha256,
                c["manifestSha256"].as_str().unwrap(),
                "{name}: manifest hash"
            );
            assert!(
                same(&out.projection, &c["projection"]),
                "{name}: projection"
            );
            verify_lingmo_v7(
                raw,
                &out.markdown,
                &out.manifest_json,
                &mut OracleGate { case: c },
            )
            .unwrap();
        }
    }
}
#[test]
fn independent_parser_matches_original_reader() {
    let f = fixture();
    for c in f["parser"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let result = parse::parse(c["markdown"].as_str().unwrap());
        if let Some(error) = c.get("error") {
            let e = result.expect_err(name);
            assert!(
                e.starts_with(error["code"].as_str().unwrap()),
                "{name}: {e}"
            );
        } else {
            assert!(same(&result.unwrap(), &c["actual"]), "{name}");
        }
    }
}
#[test]
fn exact_time_against_node() {
    let f = fixture();
    for c in f["times"].as_array().unwrap() {
        let result = time_ticks(&c["number"], false, "TIME_PRECISION");
        if c.get("error").is_some() {
            assert!(result.is_err());
        } else {
            let ticks = result.unwrap();
            assert_eq!(ticks as f64, c["ticks"].as_f64().unwrap());
            assert_eq!(time_text_ticks(ticks), c["text"].as_str().unwrap());
        }
    }
}
#[test]
fn gate_mismatch_and_visible_mutation_fail_closed() {
    let f = fixture();
    let c = &f["cases"][0];
    let raw = c["raw"].as_str().unwrap().as_bytes();
    struct Wrong;
    impl BundleGate for Wrong {
        fn check(&mut self, _: &[u8]) -> Result<GateEvidence> {
            Ok(GateEvidence {
                bundle_sha256: "bad".into(),
                normalized_sources: vec![],
                report: json!({"ok":true}),
                validator_id: "wrong".into(),
            })
        }
    }
    assert!(export_lingmo_v7(raw, &mut Wrong)
        .unwrap_err()
        .contains("BUNDLE_GATE_BINDING"));
    let out = export_lingmo_v7(raw, &mut OracleGate { case: c }).unwrap();
    let changed = out.markdown.replace("门左侧站位", "门右侧站位");
    assert_ne!(changed, out.markdown);
    assert!(verify_lingmo_v7(
        raw,
        &changed,
        &out.manifest_json,
        &mut OracleGate { case: c }
    )
    .unwrap_err()
    .starts_with("VERIFY_MISMATCH"));
    let mut manifest: Value = serde_json::from_str(&out.manifest_json).unwrap();
    manifest["assets"][0]["filename"] = json!("wrong.png");
    assert!(verify_lingmo_v7(
        raw,
        &out.markdown,
        &manifest.to_string(),
        &mut OracleGate { case: c }
    )
    .unwrap_err()
    .starts_with("VERIFY_ASSET_INDEX"));
}

#[test]
fn frozen_source_and_asset_sync_receipts_match_node() {
    let f = fixture();
    let c = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "production-registry-sync")
        .unwrap();
    let raw = c["raw"].as_str().unwrap().as_bytes();
    let lock_raw = f["sourceLock"]["raw"].as_str().unwrap().as_bytes();
    assert_eq!(
        source_lock_hash(lock_raw).unwrap(),
        f["sourceLock"]["hash"].as_str().unwrap()
    );
    let report = verify_source_lock(
        raw,
        lock_raw,
        c["sources"].as_array().unwrap(),
        f["sourceLock"]["normalizedSources"].as_array().unwrap(),
    )
    .unwrap();
    assert!(same(&report, &c["report"]["sourceFidelity"]));
    let mut changed = c["sources"].as_array().unwrap().clone();
    changed[0]["sourceRows"][0]["text"] = json!("替换原文");
    assert!(verify_source_lock(
        raw,
        lock_raw,
        &changed,
        f["sourceLock"]["normalizedSources"].as_array().unwrap()
    )
    .unwrap_err()
    .starts_with("SOURCE_LOCK_SOURCE"));
    let registry = &c["options"]["registry"];
    let registry_raw = f["sync"]["registryRaw"].as_str().unwrap().as_bytes();
    let receipt = sync_asset_consumption(
        raw,
        registry,
        f["sync"]["registrySnapshot"].as_str().unwrap(),
        registry_raw,
        &mut OracleGate { case: c },
    )
    .unwrap();
    assert!(same(&receipt, &f["sync"]["receipt"]));
    let mut changed = registry.clone();
    changed["revision"] = json!(99);
    assert!(sync_asset_consumption(
        raw,
        &changed,
        "snapshot",
        registry_raw,
        &mut OracleGate { case: c }
    )
    .unwrap_err()
    .starts_with("ASSET_REGISTRY_BINDING"));
}
#[test]
fn native_mechanical_refusals_are_not_hidden_by_oracle_gate() {
    let f = fixture();
    let baseline = &f["cases"][0];
    let sources = baseline["sources"].as_array().unwrap();
    for name in [
        "bad-version",
        "unresolved",
        "bad-number",
        "scene-order",
        "precision",
        "unit-cap",
        "negative-voice",
        "voice-outside",
        "quote-changed",
        "unknown-image",
        "duplicate-image",
        "cross-scene-source",
        "source-missing",
        "bad-import",
        "duplicate-asset",
    ] {
        let c = f["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let bundle: Value = serde_json::from_str(c["raw"].as_str().unwrap()).unwrap();
        assert!(check_mechanical(&bundle, sources).is_err(), "{name}");
    }
    // This guard is intentional: the local subset must not claim complete author
    // intent validation merely because transport, source quotes and times match.
    let c = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "source-gate-only-intent")
        .unwrap();
    let bundle: Value = serde_json::from_str(c["raw"].as_str().unwrap()).unwrap();
    assert!(check_mechanical(&bundle, sources).is_ok());
    assert!(export_lingmo_v7(
        c["raw"].as_str().unwrap().as_bytes(),
        &mut OracleGate { case: c }
    )
    .is_err());
}
