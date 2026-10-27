use super::*;
fn fixture() -> Value {
    serde_json::from_str(include_str!("series_lite_bundle_standard_golden.json")).unwrap()
}
fn compare(c: &Value, actual: Result<Value>) {
    if c.get("error").is_some() {
        assert!(
            actual.is_err(),
            "{} accepted invalid: {:?}",
            c["name"],
            c["error"]
        );
    } else {
        let actual = actual.unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(
            canonical(&actual).unwrap(),
            canonical(&c["output"]).unwrap(),
            "{}",
            c["name"]
        );
    }
}
#[test]
fn frozen_package_proof_oracle() {
    let f = fixture();
    for c in f["packageCases"].as_array().unwrap() {
        compare(c, standard_package_source(&c["pkg"], &c["options"]));
    }
}
#[test]
fn frozen_standard_source_oracle() {
    let f = fixture();
    for c in f["sourceCases"].as_array().unwrap() {
        compare(
            c,
            if c["action"] == "verify" {
                verify_standard_source(&c["input"]).map(|_| Value::Null)
            } else {
                normalize_standard_source(&c["input"])
            },
        );
    }
}
#[test]
fn frozen_inline_performance_oracle() {
    let f = fixture();
    for c in f["inlineCases"].as_array().unwrap() {
        compare(
            c,
            inline_performance_projection(st(&c["source"]), &c["parts"], st(&c["version"]))
                .map(|v| v.unwrap_or(Value::Null)),
        );
    }
}
#[test]
fn all_valid_standard_rows_have_full_bound_projection() {
    let f = fixture();
    for c in f["packageCases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("output").is_some())
    {
        let s = standard_package_source(&c["pkg"], &c["options"]).unwrap();
        verify_standard_source(&s).unwrap();
        for r in s["sourceRows"].as_array().unwrap() {
            let ds = standard_dialogues(r).unwrap();
            assert_eq!(standard_dialogue(r, 1).unwrap(), ds.first().cloned());
            for d in ds {
                assert!(!st(&d["quote"]).is_empty());
            }
            standard_onscreen_texts(r).unwrap();
        }
    }
}
