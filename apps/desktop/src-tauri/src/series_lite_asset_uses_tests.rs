use super::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!("series_lite_asset_uses_golden.json")).unwrap()
}
// Test-only closed whitelist: the frozen Node implementation validated these exact
// synthetic snapshots. This is not a Rust full-registry validator for production.
struct OracleValidator {
    allowed: Vec<Value>,
    calls: usize,
}
impl RegistryValidator for OracleValidator {
    fn validate(&mut self, registry: &Value, require_locked: bool) -> Result<()> {
        self.calls += 1;
        need(
            require_locked && self.allowed.contains(registry),
            "ORACLE_VALIDATOR: snapshot not independently verified",
        )
    }
}
fn validator(case: &Value) -> OracleValidator {
    let mut allowed = vec![case["registry"].clone()];
    if case.get("after").is_some() {
        allowed.push(case["after"].clone());
    }
    OracleValidator { allowed, calls: 0 }
}

#[test]
fn node012_oracle_transitions_and_refusals() {
    let fixture = fixture();
    assert_eq!(
        fixture["sourceHashes"]["asset-uses.mjs"],
        SOURCE_ASSET_USES_SHA256
    );
    assert_eq!(fixture["sourceHashes"]["assets.mjs"], SOURCE_ASSETS_SHA256);
    assert_eq!(
        fixture["sourceHashes"]["season-scope.mjs"],
        SOURCE_SEASON_SCOPE_SHA256
    );
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 35);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let before = c["registry"].clone();
        let mut check = validator(c);
        let got = record_uses(&before, &c["input"], &mut check);
        if let Some(error) = c.get("error") {
            let msg = got.expect_err(name);
            assert!(
                msg.starts_with(error["code"].as_str().unwrap()),
                "{name}: wrong refusal {msg}; expected {error}"
            );
        } else {
            let got = got.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(got.registry, c["after"], "{name}: snapshot");
            assert_eq!(
                got.changed,
                c["result"]["changed"].as_bool().unwrap(),
                "{name}"
            );
            assert!(!got.asset_revisions_changed, "{name}");
            assert_eq!(
                got.request_id.as_deref(),
                c["result"]["requestId"].as_str(),
                "{name}"
            );
            assert_eq!(
                got.entries.map(|n| n as u64),
                c["result"]["entries"].as_u64(),
                "{name}"
            );
            assert_eq!(
                check.calls, 2,
                "{name}: pre/post full validator must be invoked"
            );
            assert_eq!(got.validator_calls.len(), 2);
            assert_eq!(got.validation, "requires-full-registry-validation");
            for key in [
                "assets",
                "requests",
                "changes",
                "productionScope",
                "productionScopeHash",
                "sourceHash",
            ] {
                assert_eq!(got.registry[key], before[key], "{name}: preserved {key}");
            }
        }
        assert_eq!(before, c["registry"], "{name}: input mutated");
        for row in c["scopeUses"].as_array().unwrap() {
            assert_eq!(
                asset_scope_uses(&before, row["id"].as_str().unwrap()).unwrap(),
                row["uses"].as_array().unwrap().clone(),
                "{name}: scopeUses"
            );
        }
    }
}

#[test]
fn source_binding_is_explicit_desktop_extension_and_fails_closed() {
    let all = fixture();
    let c = &all["cases"][0];
    let source = SourceUseIndex {
        source_hash: "a".repeat(64),
        references: BTreeMap::from([
            ("E02:R1".into(), (2, "2-1".into())),
            ("E02:R2".into(), (2, "2-1".into())),
        ]),
    };
    record_uses_bound(&c["registry"], &c["input"], &source, &mut validator(c)).unwrap();
    let wrong_hash = SourceUseIndex {
        source_hash: "b".repeat(64),
        references: source.references.clone(),
    };
    assert!(
        record_uses_bound(&c["registry"], &c["input"], &wrong_hash, &mut validator(c))
            .unwrap_err()
            .contains("source hash")
    );
    let wrong_coordinate = SourceUseIndex {
        source_hash: source.source_hash.clone(),
        references: BTreeMap::from([("E02:R1".into(), (3, "3-1".into()))]),
    };
    assert!(record_uses_bound(
        &c["registry"],
        &c["input"],
        &wrong_coordinate,
        &mut validator(c)
    )
    .unwrap_err()
    .contains("another scene"));
    let missing = SourceUseIndex {
        source_hash: source.source_hash,
        references: BTreeMap::new(),
    };
    assert!(record_uses_bound(&c["registry"], &c["input"], &missing, &mut validator(c)).is_err());
}

#[test]
fn full_validator_rejection_at_either_boundary_never_returns_candidate() {
    struct Reject {
        call: usize,
        reject_at: usize,
    }
    impl RegistryValidator for Reject {
        fn validate(&mut self, _: &Value, _: bool) -> Result<()> {
            self.call += 1;
            need(self.call != self.reject_at, "FULL_VALIDATOR_REFUSAL")
        }
    }
    let all = fixture();
    let c = &all["cases"][0];
    for at in [1, 2] {
        let mut validator = Reject {
            call: 0,
            reject_at: at,
        };
        assert_eq!(
            record_uses(&c["registry"], &c["input"], &mut validator).unwrap_err(),
            "FULL_VALIDATOR_REFUSAL"
        );
        assert_eq!(validator.call, at);
    }
}

#[test]
fn annotation_revision_and_fields_are_not_silently_accepted() {
    let all = fixture();
    let good = all["cases"][0]["after"].clone();
    for mode in [
        "duplicate-id",
        "future-revision",
        "extra-field",
        "null-list",
        "future-design",
    ] {
        let mut registry = good.clone();
        match mode {
            "duplicate-id" => {
                let row = registry["usageAnnotations"][0].clone();
                registry["usageAnnotations"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
            }
            "future-revision" => registry["usageAnnotations"][0]["recordedAtRevision"] = json!(99),
            "extra-field" => registry["usageAnnotations"][0]["design"] = json!({}),
            "null-list" => registry["usageAnnotations"] = Value::Null,
            _ => registry["usageAnnotations"][0]["entries"][0]["assetRevision"] = json!(99),
        }
        assert!(validate_usage_annotations(&registry).is_err(), "{mode}");
    }
}
