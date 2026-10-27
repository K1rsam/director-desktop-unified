use super::*;

#[test]
fn series_lite_edit_matches_pinned_python_oracle() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("series_lite_edit_golden.json")).unwrap();
    assert_eq!(golden["upstreamCommit"], SOURCE_COMMIT);
    assert_eq!(
        golden["sourceHashes"]["lite_edit.py"],
        SOURCE_LITE_EDIT_SHA256
    );
    assert_eq!(
        golden["sourceHashes"]["context_packet.py"],
        SOURCE_CONTEXT_PACKET_SHA256
    );
    assert_eq!(
        golden["sourceHashes"]["visual_baseline.py"],
        SOURCE_VISUAL_BASELINE_SHA256
    );
    for case in golden["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let base = case["base"].as_str().unwrap().as_bytes();
        let request = case["request"].as_str().unwrap().as_bytes();
        if case["operation"] == "preflight" {
            let result = preflight_bytes(base);
            assert_eq!(
                result.is_ok(),
                case["ok"].as_bool().unwrap(),
                "{name}: {result:?}"
            );
            if let Ok(result) = result {
                assert_eq!(result.sha256, case["hash"].as_str().unwrap(), "{name}");
            }
        } else {
            let result = if case["operation"] == "compose" {
                compose_bytes(base, case["expectedHash"].as_str().unwrap(), request)
            } else {
                patch_bytes(base, request)
            };
            assert_eq!(
                result.is_ok(),
                case["ok"].as_bool().unwrap(),
                "{name}: {result:?}"
            );
            if let Ok(result) = result {
                assert_eq!(
                    result.bytes,
                    case["output"].as_str().unwrap().as_bytes(),
                    "{name}"
                );
                assert_eq!(result.sha256, case["hash"].as_str().unwrap(), "{name}");
                assert_eq!(result.changed, case["changed"].as_bool().unwrap(), "{name}");
                assert_eq!(result.validation, "not-run");
                assert_eq!(result.base_sha256, sha256(base));
            }
        }
    }
}

#[test]
fn series_lite_edit_output_precondition_refuses_input_or_existing_destination() {
    use std::path::Path;
    let base = Path::new("base.json");
    let response = Path::new("response.json");
    let output = Path::new("candidate-2.json");
    assert!(validate_output_target(base, response, base, false).is_err());
    assert!(validate_output_target(base, response, response, false).is_err());
    assert!(validate_output_target(base, response, output, true).is_err());
    assert!(validate_output_target(base, response, output, false).is_ok());
}

#[test]
fn series_lite_edit_rejects_non_strict_json_and_preserves_numeric_type() {
    for raw in [
        "{\"a\":1,\"a\":2}",
        "NaN",
        "Infinity",
        "{\"x\":1} {}",
        "\u{feff}{}",
        "{\"x\":\"\\ud800\"}",
    ] {
        assert!(parse(raw.as_bytes()).is_err(), "{raw}");
    }
    assert_eq!(
        canonical(&parse(b"{\"b\":1e0,\"a\":-0}").unwrap()),
        b"{\"a\":0,\"b\":1}"
    );
    assert!(!parse(b"1e0").unwrap().is_integer());
    assert!(parse(b"123456789012345678901234567890")
        .unwrap()
        .is_integer());
}
