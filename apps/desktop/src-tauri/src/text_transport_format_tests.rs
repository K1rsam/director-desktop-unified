use super::*;
use crate::text_executor::{compile_codex_input, compile_sdk_attempt_input};

fn request() -> TextRequest {
    TextRequest {
        stage: "fixture".into(), skill: "fixture".into(),
        model: "gpt-6.1-sol".into(), reasoning: "medium".into(),
        input: json!({"source":"原文：不改词\n😀 \"quote\" \\ path","nested":[null,false,0,1.25]}),
        schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),
        rule_snapshot: json!({"fullRule":"完整规则与来源不省略","nested":["第一条","第二条"]}),
    }
}
fn legacy() -> FrozenAttempt {
    FrozenAttempt::prepare("attempt_fixture", "workflow_fixture", "node_fixture",
        Backend::CodexSdk, request(), &json!({"synthetic":true})).unwrap()
}
fn ordered() -> FrozenAttempt {
    FrozenAttempt::prepare_sdk_rules_first("attempt_fixture", "workflow_fixture", "node_fixture",
        request(), &json!({"synthetic":true})).unwrap()
}

#[test]
fn sdk_input_format_v2_preserves_identity_compilation_and_disk_bytes() {
    let original = legacy();
    let before = serde_json::to_value(&original).unwrap();
    assert_eq!(before["version"], "text-attempt/v2");
    assert!(before.get("compiledInputFormat").is_none());
    let compiled = compile_sdk_attempt_input(&original).unwrap();
    let legacy_compiled = compile_codex_input(&original.request).unwrap();
    assert_eq!(compiled.prompt.as_bytes(), legacy_compiled.prompt.as_bytes());
    assert_eq!(compiled.schema.as_bytes(), legacy_compiled.schema.as_bytes());
    assert!(compiled.prompt.starts_with("{\"input\":"));
    let expected_envelope = json!({"version":"attempt-identity/v1","attemptId":"attempt_fixture",
        "workflowId":"workflow_fixture","logicalNodeId":"node_fixture",
        "semanticRequestHash":original.semantic_request_hash,"executionFingerprint":original.execution_fingerprint});
    assert_eq!(original.attempt_envelope_hash, digest(&expected_envelope.to_string()));
    let disk = tempfile::tempdir().unwrap(); let file = disk.path().join("legacy.json");
    original.persist(&file).unwrap(); let bytes = std::fs::read(&file).unwrap();
    let reopened = FrozenAttempt::load(&file).unwrap();
    assert_eq!(serde_json::to_value(&reopened).unwrap(), before);
    reopened.persist(&file).unwrap();
    assert_eq!(std::fs::read(file).unwrap(), bytes);
}

#[test]
fn sdk_input_format_new_v3_changes_only_top_level_order_and_reopens_exactly() {
    let original = legacy(); let next = ordered();
    assert_eq!(next.semantic_request_hash, original.semantic_request_hash);
    assert_eq!(next.execution_fingerprint, original.execution_fingerprint);
    assert_ne!(next.attempt_envelope_hash, original.attempt_envelope_hash);
    let old_wire = compile_sdk_attempt_input(&original).unwrap();
    let new_wire = compile_sdk_attempt_input(&next).unwrap();
    assert!(new_wire.prompt.starts_with("{\"rules\":"));
    assert_eq!(serde_json::from_str::<Value>(&old_wire.prompt).unwrap(), serde_json::from_str::<Value>(&new_wire.prompt).unwrap());
    assert_eq!(old_wire.prompt.len(), new_wire.prompt.len());
    assert_eq!(old_wire.prompt.encode_utf16().count(), new_wire.prompt.encode_utf16().count());
    assert_eq!(old_wire.schema, new_wire.schema);
    let temp = tempfile::tempdir().unwrap(); let file = temp.path().join("new.json");
    next.persist(&file).unwrap(); let bytes = std::fs::read(&file).unwrap();
    let reopened = FrozenAttempt::load(&file).unwrap();
    assert_eq!(compile_sdk_attempt_input(&reopened).unwrap().prompt, new_wire.prompt);
    reopened.persist(&file).unwrap(); assert_eq!(std::fs::read(file).unwrap(), bytes);
}

#[test]
fn sdk_input_format_old_bridge_keeps_v2_and_new_bridge_explicitly_selects_v3() {
    for (inspect, version) in [
        (json!({"capabilities":{"execute":"offline_fixture_only"}}), "text-attempt/v2"),
        (json!({"capabilities":{"compiledInputFormats":["legacy-json/v1"]}}), "text-attempt/v2"),
        (json!({"capabilities":{"compiledInputFormats":["legacy-json/v1","rules-first-json/v1"]}}), "text-attempt/v3"),
    ] {
        let value = FrozenAttempt::prepare_sdk_for_bridge("attempt_fixture", "workflow_fixture", "node_fixture", request(), &json!({"synthetic":true}), &inspect).unwrap();
        assert_eq!(value.version, version);
        if version == "text-attempt/v2" { assert_eq!(serde_json::to_value(value).unwrap(), serde_json::to_value(legacy()).unwrap()); }
    }
}

#[test]
fn sdk_input_format_tampering_and_unversioned_format_are_rejected_before_compilation() {
    let old = legacy(); let new = ordered();
    for base in [&old, &new] {
        for mode in ["format", "version", "envelope", "source", "backend"] {
            let mut changed = base.clone();
            match mode {
                "format" => changed.compiled_input_format = if base.compiled_input_format.is_some() { None } else { Some("rules-first-json/v1".into()) },
                "version" => changed.version = "text-attempt/v99".into(),
                "envelope" => changed.attempt_envelope_hash = "tampered".into(),
                "source" => changed.request.input["source"] = json!("modified"),
                "backend" => changed.backend = Backend::CodexCli,
                _ => unreachable!(),
            }
            assert!(compile_sdk_attempt_input(&changed).is_err(), "{mode}");
        }
    }
    let mut unknown = new; unknown.compiled_input_format = Some("rules-first-json/v2".into());
    assert!(compile_sdk_attempt_input(&unknown).err().unwrap().contains("SDK_COMPILED_FORMAT_UNSUPPORTED"));
}

#[test]
fn sdk_input_format_v2_rejects_added_null_format_even_with_recomputed_disk_envelope() {
    let temp = tempfile::tempdir().unwrap(); let file = temp.path().join("legacy.json");
    legacy().persist(&file).unwrap();
    let mut disk: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    disk["payload"]["compiledInputFormat"] = Value::Null;
    disk["sha256"] = json!(digest(&disk["payload"].to_string()));
    std::fs::write(&file, disk.to_string()).unwrap();
    assert!(FrozenAttempt::load(&file).unwrap_err().contains("SDK_COMPILED_FORMAT_UNSUPPORTED"));
}
