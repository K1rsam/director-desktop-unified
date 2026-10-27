use super::*;
#[test]
fn alias_resolution() {
    let source =
        json!({"key":"E1","sourceRows":[{"id":"E1:R1","line":1,"text":"源"}],"contextRows":[]});
    assert_eq!(
        resolve_source_ref(&source, &json!("L0001"), false).unwrap()["id"],
        "E1:R1"
    );
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("series_lite_bundle_gate_golden.json")).unwrap()
}
#[test]
fn original_validate_draft_codes_coordinates_stats() {
    let f = fixture();
    for c in f["cases"].as_array().unwrap() {
        let x = &c["input"];
        let got = validate_draft(
            &x["draft"],
            &x["source"],
            &x["settings"],
            x.get("selection"),
            &x["plan"],
            x["unitDirectionVersion"].as_str(),
        );
        assert!(
            same(&got, &c["output"]),
            "{}: got {got}; expected {}",
            c["name"],
            c["output"]
        );
    }
}
#[test]
fn original_fantasy_intervals() {
    let f = fixture();
    for c in f["timing"].as_array().unwrap() {
        let got = fantasy_timing(&c["record"], &c["draft"]).unwrap_or(Value::Null);
        assert!(
            same(&got, &c["output"]),
            "{}: got {got}, expected {}",
            c["name"],
            c["output"]
        );
    }
}
#[test]
fn original_alias_and_event_checks() {
    let f = fixture();
    for c in f["aliases"].as_array().unwrap() {
        let got = resolve_source_ref(
            &c["source"],
            &c["reference"],
            c["context"].as_bool().unwrap(),
        );
        if let Some(e) = c.get("error") {
            assert!(
                got.unwrap_err().starts_with(e["code"].as_str().unwrap()),
                "{}",
                c["name"]
            );
        } else {
            assert_eq!(*got.unwrap(), c["output"], "{}", c["name"]);
        }
    }
    for c in f["events"].as_array().unwrap() {
        let got = checked_events(&c["plan"], &c["source"]);
        if let Some(e) = c.get("error") {
            assert!(
                got.unwrap_err().starts_with(e["code"].as_str().unwrap()),
                "{}",
                c["name"]
            );
        } else {
            assert_eq!(got.unwrap().unwrap(), c["output"], "{}", c["name"]);
        }
    }
}
#[test]
fn original_legacy_source_normalization() {
    let f = fixture();
    for c in f["sourceCases"].as_array().unwrap() {
        let got = normalize_legacy_source(&c["input"]);
        if let Some(e) = c.get("error") {
            let err = got.expect_err(c["name"].as_str().unwrap());
            assert!(
                err.starts_with(e["code"].as_str().unwrap()),
                "{}: {err}; expected {e}",
                c["name"]
            );
        } else {
            let actual = got.unwrap_or_else(|err| panic!("{}: {err}", c["name"]));
            assert!(
                same(&actual, &c["output"]),
                "{}: got {actual}; expected {}",
                c["name"],
                c["output"]
            );
        }
    }
}
#[test]
fn complete_native_raw_row_gate_and_md_without_oracle_callback() {
    use crate::series_lite_bundle::{export_lingmo_v7, BundleGate};
    let f: Value = serde_json::from_str(include_str!("series_lite_bundle_golden.json")).unwrap();
    for c in f["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let source_lock = c["options"]
            .get("sourceLock")
            .map(|_| f["sourceLock"]["raw"].as_str().unwrap().as_bytes());
        let mut gate = NativeLegacyBundleGate {
            registry: c["options"].get("registry"),
            source_lock,
            season_scope: None,
        };
        let raw = c["raw"].as_str().unwrap().as_bytes();
        let result = export_lingmo_v7(raw, &mut gate);
        if c.get("error").is_some() {
            assert!(result.is_err(), "{name}: invalid bundle accepted");
        } else {
            let out = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(out.markdown, c["markdown"].as_str().unwrap(), "{name}");
            assert_eq!(out.manifest_json, c["manifest"].as_str().unwrap(), "{name}");
            assert!(
                same(&out.gate_report, &c["report"]),
                "{name}: report {} expected {}",
                out.gate_report,
                c["report"]
            );
            let evidence = gate.check(raw).unwrap();
            assert!(
                same(&json!(evidence.normalized_sources), &c["sources"]),
                "{name}: normalized sources"
            );
        }
    }
}
#[test]
fn standard_package_is_not_downgraded() {
    let raw = json!({"key":"E1","sourceProtocol":"standard-episode-v1","sourceRows":[{"id":"R1","text":"甲：原话。"}]});
    assert!(normalize_legacy_source(&raw)
        .unwrap_err()
        .starts_with("SOURCE_NATIVE_STANDARD_PENDING"));
}
