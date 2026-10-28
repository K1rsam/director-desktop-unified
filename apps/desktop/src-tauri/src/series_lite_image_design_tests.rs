use super::*;

struct Validator {
    calls: Vec<bool>,
}
impl RegistryValidator for Validator {
    fn validate(&mut self, registry: &Value, require_locked: bool) -> Result<()> {
        self.calls.push(require_locked);
        crate::series_lite_registry::validate_registry(registry, require_locked)?;
        for asset in registry["assets"].as_array().unwrap() {
            validate_design_level(asset)?;
        }
        validate_image_designs(registry)
    }
}
fn golden() -> Value {
    serde_json::from_str(include_str!("series_lite_image_design_golden.json")).unwrap()
}

#[test]
fn frozen_node_image_design_oracle_success_and_rejection() {
    let fixture = golden();
    assert_eq!(
        fixture["sourceCommit"],
        "f855227c8406b38093d03c777fe24a0b0b4f87dc"
    );
    let source = include_bytes!(
        "../../../../rules/series-lite/20261028-v0.4.1/scripts/asset-image-design.mjs"
    );
    assert_eq!(
        fixture["sourceSha256"].as_str(),
        Some(format!("{:x}", Sha256::digest(source)).as_str())
    );
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let registry = &case["registry"];
        let before = registry.clone();
        let mut validator = Validator { calls: vec![] };
        let actual = match case["op"].as_str().unwrap() {
            "level" => validate_design_level(&case["asset"]).map(|_| json!(true)),
            "validate" => validate_image_designs(registry).map(|_| json!(true)),
            "resolve" => resolve_image_design(registry, case["assetId"].as_str().unwrap()),
            "prepare" => prepare_image_design_batch(registry, &case["input"]),
            "hash" => image_hash(&case["value"]).map(|v| json!(v)),
            "record" => record_image_design(registry, &case["input"], &mut validator)
                .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
            _ => panic!("unknown oracle operation"),
        };
        assert_eq!(*registry, before, "input mutated: {name}");
        if case["expected"]["ok"] == true {
            let actual = actual.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                canonical(&actual).unwrap(),
                canonical(&case["expected"]["value"]).unwrap(),
                "{name}"
            );
            if case["op"] == "record" {
                assert!(validator.calls.iter().all(|v| *v), "{name}");
            }
        } else {
            let error = actual.expect_err(name);
            assert_eq!(
                error.split(':').next().unwrap(),
                case["expected"]["code"].as_str().unwrap(),
                "{name}: {error}"
            );
        }
    }
}

#[test]
fn recording_does_not_touch_narrative_asset_rows_or_evidence() {
    let fixture = golden();
    let c = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "reverse-parent-record")
        .unwrap();
    let mut validator = Validator { calls: vec![] };
    let result = record_image_design(&c["registry"], &c["input"], &mut validator).unwrap();
    assert!(result.changed);
    assert!(!result.asset_revisions_changed);
    for key in ["id", "sourceHash", "assets", "requests", "changes"] {
        assert_eq!(result.registry[key], c["registry"][key], "{key}");
    }
    assert_eq!(validator.calls, vec![true, true]);
    let before = result.registry.clone();
    let retry = record_image_design(&before, &c["input"], &mut validator).unwrap();
    assert!(!retry.changed);
    assert_eq!(retry.registry, before);
}

#[test]
fn validator_failure_never_returns_a_publishable_snapshot() {
    struct Reject;
    impl RegistryValidator for Reject {
        fn validate(&mut self, _: &Value, _: bool) -> Result<()> {
            Err("REAL_REGISTRY_REJECTED".into())
        }
    }
    let fixture = golden();
    let c = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "reverse-parent-record")
        .unwrap();
    let before = c["registry"].clone();
    assert_eq!(
        record_image_design(&before, &c["input"], &mut Reject).unwrap_err(),
        "REAL_REGISTRY_REJECTED"
    );
    assert_eq!(before, c["registry"]);
    struct RejectPostimage(usize);
    impl RegistryValidator for RejectPostimage {
        fn validate(&mut self, _: &Value, _: bool) -> Result<()> {
            self.0 += 1;
            if self.0 == 2 {
                Err("POSTIMAGE_REJECTED".into())
            } else {
                Ok(())
            }
        }
    }
    assert_eq!(
        record_image_design(&before, &c["input"], &mut RejectPostimage(0)).unwrap_err(),
        "POSTIMAGE_REJECTED"
    );
    assert_eq!(before, c["registry"]);
}

#[test]
fn js_safe_integer_metadata_accepts_equivalent_decimal_spelling() {
    let fixture = golden();
    let c = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "reverse-parent-record")
        .unwrap();
    let mut input = c["input"].clone();
    input["baseRevision"] = json!(1.0);
    for e in input["entries"].as_array_mut().unwrap() {
        e["assetRevision"] = json!(1.0);
    }
    let result =
        record_image_design(&c["registry"], &input, &mut Validator { calls: vec![] }).unwrap();
    assert_eq!(
        canonical(&serde_json::to_value(result).unwrap()).unwrap(),
        canonical(&c["expected"]["value"]).unwrap()
    );
}
