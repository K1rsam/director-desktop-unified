use super::*;
#[test]
fn original_planner_full_outputs() {
    let f: Value =
        serde_json::from_str(include_str!("series_lite_intake_plan_golden.json")).unwrap();
    let mut failures = vec![];
    for case in f["cases"].as_array().unwrap() {
        let x = &case["input"];
        let result = prepare_standard_episode(&x["text"], &x["sourceId"], &x["options"]);
        if let Some(e) = case.get("error") {
            match result {
                Err(got) => {
                    if got.code != st(&e["code"])
                        || got.field_path.as_deref() != e.get("fieldPath").and_then(Value::as_str)
                    {
                        failures.push(format!("{}: error {got:?} expected {e}", case["name"]));
                    }
                }
                Ok(_) => failures.push(format!("{}: incorrectly accepted", case["name"])),
            }
        } else {
            match result {
                Ok(got) => {
                    if crate::series_lite_registry::canonical(&got).unwrap()
                        != crate::series_lite_registry::canonical(&case["output"]).unwrap()
                    {
                        let keys = ["index", "plan", "diagnostics", "compiled"]
                            .into_iter()
                            .filter(|k| got[*k] != case["output"][*k])
                            .collect::<Vec<_>>();
                        failures.push(format!(
                            "{}: mismatch {:?}\nGOT={}\nEXPECTED={}",
                            case["name"], keys, got, case["output"]
                        ));
                    }
                }
                Err(e) => failures.push(format!("{}: unexpected {e:?}", case["name"])),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} case failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
