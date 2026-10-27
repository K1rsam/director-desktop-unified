use super::*;
fn fixtures() -> Value {
    serde_json::from_str(include_str!("series_lite_intake_golden.json")).unwrap()
}
fn execute(c: &Value) -> IntakeResult<Value> {
    let i = &c["input"];
    match st(&c["kind"]) {
        "prepare" => prepare_source(&i["text"], &i["key"], &i["knownSpeakers"]),
        "parse" => parse_script(&i["text"], &i["options"]),
        "index" => create_source_index(&i["text"], &i["sourceId"]),
        "compile" => compile_standard_script(&i["index"], &i["plan"]),
        "lines" => derive_standard_script_lines(&i["prepared"]),
        "version" => standard_script_plan_version(&i["index"], &i["entries"], &i["options"])
            .map(|s| json!(s)),
        "subtitles" => Ok(explicit_subtitles(st(&i["text"]))),
        _ => panic!("unknown case"),
    }
}
fn run(kind: &str) {
    let f = fixtures();
    let mut failures = vec![];
    for c in f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == kind)
    {
        let result = execute(c);
        match (c.get("error"), result) {
            (Some(e), Ok(_)) => failures.push(format!("{} accepted invalid {}", c["name"], e)),
            (None, Err(e)) => failures.push(format!("{} rejected valid {:?}", c["name"], e)),
            (None, Ok(out)) => {
                if crate::series_lite_registry::canonical(&out).unwrap()
                    != crate::series_lite_registry::canonical(&c["output"]).unwrap()
                {
                    std::fs::write(
                        std::env::temp_dir()
                            .join(format!("lite-intake-{}-actual.json", st(&c["name"]))),
                        serde_json::to_vec_pretty(&out).unwrap(),
                    )
                    .unwrap();
                    failures.push(format!("{} output differs", c["name"]));
                }
            }
            (Some(e), Err(actual)) => {
                if c["kind"] != "parse"
                    && st(&e["code"]) != "UNCAUGHT"
                    && (actual.code != st(&e["code"])
                        || actual.field_path.as_deref() != e["fieldPath"].as_str()
                        || actual.source_unit_id.as_deref() != e["sourceUnitId"].as_str()
                        || json!(actual.diagnostics)
                            != e.get("diagnostics").cloned().unwrap_or(json!([])))
                {
                    failures.push(format!(
                        "{} error code/path {:?} != {}",
                        c["name"], actual, e
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn frozen_parse_script() {
    run("parse")
}
#[test]
fn frozen_physical_source_index() {
    run("index")
}
#[test]
fn frozen_compile_standard_script() {
    run("compile")
}
#[test]
fn frozen_derive_standard_lines() {
    run("lines")
}
#[test]
fn frozen_plan_version() {
    run("version")
}
#[test]
fn frozen_explicit_subtitles() {
    run("subtitles")
}

#[test]
fn frozen_prepare_source_end_to_end() {
    run("prepare")
}
#[test]
fn actual_intake_source_to_locked_bundle_and_markdown() {
    let f = fixtures();
    for c in f["bundleCases"].as_array().unwrap() {
        let i = &c["nativeInput"];
        let source = prepare_source(&i["text"], &i["key"], &i["knownSpeakers"]).unwrap();
        assert_eq!(
            hash(&source).unwrap(),
            hash(&c["source"]).unwrap(),
            "{} intake source",
            c["name"]
        );
        let mut gate = crate::series_lite_bundle_gate::NativeLegacyBundleGate {
            registry: None,
            source_lock: c["sourceLockRaw"].as_str().map(str::as_bytes),
            season_scope: None,
        };
        let result = crate::series_lite_bundle::export_lingmo_v7(
            c["raw"].as_str().unwrap().as_bytes(),
            &mut gate,
        );
        if c.get("error").is_some() {
            assert!(result.is_err(), "{} invalid bundle accepted", c["name"]);
        } else {
            let out = result.unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
            assert_eq!(
                out.markdown,
                c["markdown"].as_str().unwrap(),
                "{} markdown",
                c["name"]
            );
            assert_eq!(
                out.manifest_json,
                c["manifest"].as_str().unwrap(),
                "{} manifest",
                c["name"]
            );
            assert_eq!(
                hash(&out.gate_report).unwrap(),
                hash(&c["report"]).unwrap(),
                "{} gate report",
                c["name"]
            );
        }
    }
}
