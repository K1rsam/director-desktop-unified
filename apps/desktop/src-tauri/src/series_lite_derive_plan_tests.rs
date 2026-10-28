use super::*;
use crate::series_lite_edit::{canonical, compose_bytes, compose_derived_bytes, sha256};
use crate::series_lite_edit::{patch_bytes, patch_derived_bytes};

#[test]
fn series_lite_041_derive_matches_pinned_python_complete_bytes_and_rejections() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("series_lite_derive_plan_golden.json")).unwrap();
    assert_eq!(
        oracle["upstreamCommit"],
        "f855227c8406b38093d03c777fe24a0b0b4f87dc"
    );
    assert_eq!(
        oracle["sourceHashes"]["lite_edit.py"],
        sha256(include_bytes!(
            "../../../../rules/series-lite/20261028-v0.4.1/scripts/lite_edit.py"
        ))
    );
    assert_eq!(
        oracle["sourceHashes"]["context_packet.py"],
        sha256(include_bytes!(
            "../../../../rules/series-lite/20261028-v0.4.1/scripts/context_packet.py"
        ))
    );
    for case in oracle["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let base = case["base"].as_str().unwrap().as_bytes();
        let response = case["response"].as_str().unwrap().as_bytes();
        let actual = compose_derived_bytes(base, case["expectedHash"].as_str().unwrap(), response);
        assert_eq!(
            actual.is_ok(),
            case["ok"].as_bool().unwrap(),
            "{name}: {actual:?}"
        );
        if let Ok(actual) = actual {
            assert_eq!(
                actual.edit.bytes,
                case["output"].as_str().unwrap().as_bytes(),
                "{name}"
            );
            assert_eq!(
                actual.edit.sha256,
                case["sha256"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(
                actual.edit.changed,
                case["changed"].as_bool().unwrap(),
                "{name}"
            );
            assert_eq!(actual.edit.base_sha256, sha256(base), "{name}");
            assert_eq!(
                actual.derived_bookkeeping,
                case["derivedBookkeeping"].as_bool().unwrap(),
                "{name}"
            );
            assert_eq!(actual.edit.validation, "not-run");
            assert!(actual
                .bookkeeping_basis
                .contains("not prior capacity planning"));
            let source = parse(base)
                .unwrap()
                .get("episodes")
                .unwrap()
                .array()
                .unwrap()[0]
                .get("source")
                .unwrap()
                .clone();
            assert_eq!(
                parse(&actual.edit.bytes)
                    .unwrap()
                    .get("episodes")
                    .unwrap()
                    .array()
                    .unwrap()[0]
                    .get("source")
                    .unwrap(),
                &source,
                "{name}: frozen source"
            );
        }
    }
}

#[test]
fn series_lite_041_derive_is_opt_in_and_never_validates_authored_dialogue() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("series_lite_derive_plan_golden.json")).unwrap();
    let case = &oracle["cases"][0];
    let base = case["base"].as_str().unwrap().as_bytes();
    let response = case["response"].as_str().unwrap().as_bytes();
    let legacy = compose_bytes(base, &sha256(base), response).unwrap();
    let old = parse(&legacy.bytes).unwrap();
    assert!(!old.get("episodes").unwrap().array().unwrap()[0]
        .get("plan")
        .unwrap()
        .object()
        .unwrap()
        .contains_key("workPackages"));
    let mut changed = parse(response).unwrap();
    let Json::Object(root) = &mut changed else {
        panic!()
    };
    let Json::Object(draft) = root.get_mut("draft").unwrap() else {
        panic!()
    };
    let Json::Array(units) = draft.get_mut("units").unwrap() else {
        panic!()
    };
    let Json::Object(unit) = &mut units[0] else {
        panic!()
    };
    let Json::Array(shots) = unit.get_mut("shots").unwrap() else {
        panic!()
    };
    let Json::Object(shot) = &mut shots[0] else {
        panic!()
    };
    shot.insert(
        "dialogueParts".into(),
        Json::String("not content-reviewed by assembler".into()),
    );
    let assembled = compose_derived_bytes(base, &sha256(base), &canonical(&changed)).unwrap();
    assert!(std::str::from_utf8(&assembled.edit.bytes)
        .unwrap()
        .contains("not content-reviewed by assembler"));
    assert_eq!(assembled.edit.validation, "not-run");
}

fn derived_base() -> Vec<u8> {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("series_lite_derive_plan_golden.json")).unwrap();
    oracle["cases"][0]["output"]
        .as_str()
        .unwrap()
        .as_bytes()
        .to_vec()
}
fn duration_patch(base: &[u8]) -> Vec<u8> {
    format!(r#"{{"baseSha256":"{}","operations":[{{"op":"set","path":"/episodes/0/draft/units/0/shots/0/duration","expected":2.5,"value":3.125000001}}]}}"#,sha256(base)).into_bytes()
}

#[test]
fn series_lite_041_finisher_patch_refreshes_exact_program_indices_without_model_rewriting_them() {
    let base = derived_base();
    let request = duration_patch(&base);
    let before = parse(&base).unwrap();
    let out = patch_derived_bytes(&base, &request).unwrap();
    let after = parse(&out.edit.bytes).unwrap();
    let old = &before.get("episodes").unwrap().array().unwrap()[0];
    let new = &after.get("episodes").unwrap().array().unwrap()[0];
    assert_eq!(
        canonical(
            new.get("plan")
                .unwrap()
                .get("provisionalUnits")
                .unwrap()
                .array()
                .unwrap()[0]
                .get("secondsRange")
                .unwrap()
        ),
        b"[6.625000001,6.625000001]"
    );
    assert_eq!(
        old.get("plan").unwrap().get("workPackages").unwrap(),
        new.get("plan").unwrap().get("workPackages").unwrap()
    );
    assert_eq!(
        old.get("plan").unwrap().get("events").unwrap(),
        new.get("plan").unwrap().get("events").unwrap()
    );
    for key in ["source", "settings", "scenes"] {
        assert_eq!(old.get(key).unwrap(), new.get(key).unwrap(), "{key}");
    }
    for key in ["assets", "sourceHash", "sourceLockHash"] {
        assert_eq!(
            before.object().unwrap().get(key),
            after.object().unwrap().get(key),
            "{key}"
        );
    }
    assert!(out.derived_bookkeeping);
    assert_eq!(out.edit.validation, "not-run");
    let legacy = patch_bytes(&base, &request).unwrap();
    let legacy = parse(&legacy.bytes).unwrap();
    assert_eq!(
        canonical(
            legacy.get("episodes").unwrap().array().unwrap()[0]
                .get("plan")
                .unwrap()
                .get("provisionalUnits")
                .unwrap()
                .array()
                .unwrap()[0]
                .get("secondsRange")
                .unwrap()
        ),
        b"[6,6]",
        "old patch behavior is unchanged"
    );
}

#[test]
fn series_lite_041_finisher_refresh_rejects_drifted_or_authored_base_indices_and_direct_index_patches(
) {
    let base = derived_base();
    let raw = String::from_utf8(base.clone()).unwrap();
    let drift = raw
        .replacen(r#""secondsRange":[6,6]"#, r#""secondsRange":[5,5]"#, 1)
        .into_bytes();
    assert_ne!(drift, base);
    assert!(patch_derived_bytes(&drift, &duration_patch(&drift))
        .unwrap_err()
        .contains("base indices"));
    let request = format!(
        r#"{{"baseSha256":"{}","operations":[{{"op":"set","path":"/episodes/0/plan/provisionalUnits/0/secondsRange","expected":[6,6],"value":[5,5]}}]}}"#,
        sha256(&base)
    );
    assert!(patch_derived_bytes(&base, request.as_bytes())
        .unwrap_err()
        .contains("program-owned"));
    let mut spaced = b"\r\n ".to_vec();
    spaced.extend_from_slice(&base);
    spaced.extend_from_slice(b"\r\n");
    let no_op = format!(r#"{{"baseSha256":"{}","operations":[]}}"#, sha256(&spaced));
    let output = patch_derived_bytes(&spaced, no_op.as_bytes()).unwrap();
    assert!(!output.edit.changed);
    assert_eq!(output.edit.bytes, spaced);
}
