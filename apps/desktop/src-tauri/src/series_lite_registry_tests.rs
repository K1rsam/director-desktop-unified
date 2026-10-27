use super::*;

#[test]
fn unicode_contract_basics() {
    assert_eq!(unicode::nfkc("Ａﬃ①가A\u{030a}"), "Affi1가Å");
    assert_eq!(unicode::casefold(&unicode::nfkc("Straße")), "strasse");
    assert_eq!(
        unicode::lowercase_en("ΟΣ ΣΟΣ Σ Σ' AΣ\u{0301}"),
        "ος σος σ σ' aς\u{0301}"
    );
    assert!(unicode::python_alnum('Ⅷ'));
    assert!(!unicode::python_alnum('\u{0345}'));
}

#[test]
fn frozen_node_registry_season_and_bundle_oracle() {
    let fixture: Value =
        serde_json::from_str(include_str!("series_lite_registry_golden.json")).unwrap();
    assert_eq!(fixture["sourceCommit"], SOURCE_COMMIT);
    assert_eq!(fixture["hashes"]["assets.mjs"], SOURCE_ASSETS_SHA256);
    assert_eq!(fixture["hashes"]["season-scope.mjs"], SOURCE_SEASON_SHA256);
    assert_eq!(
        fixture["hashes"]["lingmo-asset-names.mjs"],
        SOURCE_IMPORT_NAMES_SHA256
    );
    for c in fixture["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let input = &c["input"];
        let options = &c["options"];
        let result = match c["kind"].as_str().unwrap() {
            "registry" => {
                validate_registry(input, options["requireLocked"].as_bool().unwrap_or(false))
                    .map(|_| json!(true))
            }
            "season" => validate_season_scope(
                input,
                options.get("sourceHash"),
                options.get("index"),
                options["requireLocked"].as_bool().unwrap_or(true),
            )
            .and_then(|_| Ok(json!({"valid":true,"hash":hash_season_scope(input)?}))),
            "bindings" => check_asset_bindings(input, &options["registry"]),
            _ => check_season_bundle(
                input,
                options.get("scope"),
                options["requireComplete"].as_bool().unwrap_or(false),
            ),
        };
        if let Some(error) = c.get("error") {
            let actual = result.expect_err(name);
            assert!(
                actual.starts_with(error["code"].as_str().unwrap()),
                "{name}: expected {error}, got {actual}"
            );
        } else {
            assert_eq!(
                canonical(&result.unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap(),
                canonical(&c["output"]).unwrap(),
                "{name}"
            );
        }
    }
}

#[test]
fn record_uses_with_native_full_validator_matches_frozen_node() {
    let fixture: Value =
        serde_json::from_str(include_str!("series_lite_asset_uses_golden.json")).unwrap();
    for c in fixture["cases"].as_array().unwrap() {
        let result = crate::series_lite_asset_uses::record_uses(
            &c["registry"],
            &c["input"],
            &mut NativeRegistryValidator,
        );
        if let Some(error) = c.get("error") {
            let actual = result.expect_err(c["name"].as_str().unwrap());
            assert!(
                actual.starts_with(error["code"].as_str().unwrap()),
                "{}: {actual}",
                c["name"]
            );
        } else {
            assert_eq!(result.unwrap().registry, c["after"], "{}", c["name"]);
        }
    }
}

#[test]
fn unicode17_python_node_conformance_corpus() {
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("series_lite_registry_unicode17.json"))
        ),
        UNICODE_TABLE_SHA256
    );
    let rows: Value =
        serde_json::from_str(include_str!("series_lite_registry_unicode_golden.json")).unwrap();
    assert!(rows.as_array().unwrap().len() > 9000);
    for r in rows.as_array().unwrap() {
        let s = r[0].as_str().unwrap();
        let normalized = unicode::nfkc(s);
        assert_eq!(normalized, r[1].as_str().unwrap(), "NFKC {s:?}");
        assert_eq!(
            unicode::casefold(&normalized),
            r[2].as_str().unwrap(),
            "casefold {s:?}"
        );
        assert_eq!(
            unicode::lowercase_en(s),
            r[3].as_str().unwrap(),
            "Node lower {s:?}"
        );
        assert_eq!(
            s.chars().map(unicode::python_alnum).collect::<Vec<_>>(),
            r[4].as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_bool().unwrap())
                .collect::<Vec<_>>(),
            "isalnum {s:?}"
        );
    }
}
