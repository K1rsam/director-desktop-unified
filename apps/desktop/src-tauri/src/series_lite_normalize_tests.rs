use super::*;
fn golden() -> Value {
    serde_json::from_str(include_str!("series_lite_normalize_golden.json")).unwrap()
}
#[test]
fn frozen_normalize_and_pack_oracle_parity() {
    let f = golden();
    for c in list(&f["cases"]) {
        if c["operation"] == "plan" {
            continue;
        }
        let x = &c["input"];
        let got = if c["operation"] == "pack" {
            pack_event_units(&x["draft"], &x["options"])
        } else {
            normalize_draft(&x["draft"], &x["source"], &x["options"])
        };
        check(c, got);
    }
}
#[test]
fn frozen_event_plan_oracle_parity() {
    let f = golden();
    for c in list(&f["cases"]) {
        if c["operation"] != "plan" {
            continue;
        }
        let x = &c["input"];
        check(c, normalize_event_plan(&x["plan"], &x["source"]));
    }
}
fn check(c: &Value, got: Result<Value>) {
    if let Some(error) = c.get("error") {
        let got = got.unwrap_err();
        assert_eq!(got.name, error["name"].as_str().unwrap(), "{}", c["name"]);
        assert_eq!(
            got.code.as_deref(),
            error.get("code").and_then(Value::as_str),
            "{}",
            c["name"]
        );
        // Engine-generated TypeError wording is implementation-specific; never
        // turn that rejection into success. All domain codes AND messages match.
        if got.code.is_some() {
            assert_eq!(
                got.message,
                error["message"].as_str().unwrap(),
                "{}",
                c["name"]
            );
        }
    } else {
        let got = got.unwrap_or_else(|e| panic!("{}: {e:?}", c["name"]));
        assert_eq!(
            crate::series_lite_registry::canonical(&got).unwrap(),
            crate::series_lite_registry::canonical(&c["output"]).unwrap(),
            "{}: got {got}",
            c["name"]
        );
    }
}
#[test]
fn normalization_preserves_source_windows_ids_and_prior_bindings() {
    let f = golden();
    let c = &f["cases"][0];
    let x = &c["input"];
    let before = x.clone();
    let out = normalize_draft(&x["draft"], &x["source"], &x["options"]).unwrap();
    assert_eq!(*x, before);
    let shots: Vec<_> = list(&x["draft"]["units"])
        .iter()
        .flat_map(|u| list(&u["shots"]))
        .collect();
    let actual = list(&out["draft"]["units"][0]["shots"]);
    assert_eq!(shots.len(), actual.len());
    for (a, b) in shots.iter().zip(actual) {
        for key in [
            "id",
            "duration",
            "sourceRefs",
            "description",
            "dialogueParts",
        ] {
            assert_eq!(a[key], b[key], "{key}");
        }
    }
    assert_eq!(
        out["draft"]["units"][0]["bindings"],
        x["draft"]["units"][0]["bindings"]
    );
    assert_eq!(
        out["draft"]["units"][0]["imageBindings"],
        x["draft"]["units"][0]["imageBindings"]
    );
    assert_eq!(
        out["draft"]["fantasyRealization"][0]["screenSpans"],
        x["draft"]["fantasyRealization"][0]["screenSpans"]
    );
}
#[test]
fn normalization_failure_does_not_partially_mutate_input() {
    let f = golden();
    for c in list(&f["cases"])
        .iter()
        .filter(|c| c.get("error").is_some())
    {
        let before = c["input"].clone();
        let x = &c["input"];
        let result = if c["operation"] == "plan" {
            normalize_event_plan(&x["plan"], &x["source"])
        } else {
            normalize_draft(&x["draft"], &x["source"], &x["options"])
        };
        assert!(result.is_err(), "{}", c["name"]);
        assert_eq!(*x, before);
    }
}
