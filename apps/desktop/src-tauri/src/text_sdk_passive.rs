//! Passive evidence inspection after a bridge upgrade. No worker, RPC, writes,
//! result adoption or new execution. Completed Rust records use sdk_raw instead.
use super::*;
use crate::text_transport::AttemptState;
use serde_json::value::RawValue;
use std::{collections::BTreeMap, io::Read};

// JSON.stringify turns integral f64 values (e.g. 24.0) into integer tokens.
// Accept only that representation change, within JS's exact integer range.
// Hashes still cover original bytes; strings, keys and every other value match.
fn same_json_value(a: &Value, b: &Value) -> bool {
    if a == b { return true; }
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            matches!((a.as_f64(), b.as_f64()), (Some(x), Some(y))
                if x == y && x.fract() == 0.0 && x.abs() <= 9_007_199_254_740_991.0)
        }
        (Value::Array(a), Value::Array(b)) => a.len() == b.len()
            && a.iter().zip(b).all(|(a,b)| same_json_value(a,b)),
        (Value::Object(a), Value::Object(b)) => a.len() == b.len()
            && a.iter().all(|(key,value)| b.get(key).is_some_and(|other| same_json_value(value,other))),
        _ => false,
    }
}

fn read(path: &Path, limit: usize, optional: bool) -> Result<String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err("SDK_PASSIVE_FILE_LIMIT".into());
    }
    let mut text = String::new();
    std::fs::File::open(path).map_err(|e| e.to_string())?.take(limit as u64 + 1)
        .read_to_string(&mut text).map_err(|e| e.to_string())?;
    if text.len() > limit { return Err("SDK_PASSIVE_FILE_LIMIT".into()); }
    Ok(text)
}

/// Verifies only durable local evidence, and deliberately never returns a
/// Completed observation, even if a late receipt claims transport completion.
pub(crate) fn historical_receipt(root: &Path, original: &FrozenAttempt) -> Result<Value> {
    original.validate_identity()?;
    let identity: SdkIdentity = serde_json::from_value(original.runtime_identity.clone()).map_err(|e| e.to_string())?;
    identity.validate_receipt()?;
    if original.backend != Backend::CodexSdk || !original.dispatch_started
        || original.attempt_id.is_empty() || original.attempt_id.len() > 128
        || !original.attempt_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err("SDK_PASSIVE_ATTEMPT_IDENTITY".into());
    }
    let dir = worker_root(root,&original.request)?.join(&original.attempt_id);
    let file = dir.join("receipt.json");
    let text = read(&file, 4_194_304, false)?;
    let record: Value = serde_json::from_str(&text).map_err(|_| "SDK_PASSIVE_RECEIPT_JSON")?;
    let raw: BTreeMap<String, Box<RawValue>> = serde_json::from_str(&text).map_err(|_| "SDK_PASSIVE_RECEIPT_JSON")?;
    // atomicJson appends the envelope seal last. Hash ORIGINAL serialized bytes,
    // not Rust reserialization (JS key ordering/number spelling may differ).
    let seal = s(&record, "receiptEnvelopeSha256");
    let suffix = format!(",\"receiptEnvelopeSha256\":\"{seal}\"}}");
    let payload = text.strip_suffix(&suffix).ok_or("SDK_PASSIVE_RECEIPT_SEAL")?;
    if seal.len() != 64 || digest(&format!("{payload}}}")) != seal {
        return Err("SDK_PASSIVE_RECEIPT_SEAL".into());
    }
    let frozen_raw = raw.get("attempt").ok_or("SDK_PASSIVE_REQUEST_BINDING")?;
    let frozen_fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(frozen_raw.get()).map_err(|_| "SDK_PASSIVE_REQUEST_BINDING")?;
    let request_raw = frozen_fields.get("request").ok_or("SDK_PASSIVE_REQUEST_BINDING")?;
    let mut prepared = original.clone();
    prepared.state = AttemptState::Prepared; prepared.dispatch_started = false;
    prepared.thread_id = None; prepared.turn_id = None; prepared.last_sequence = 0; prepared.raw_result = None;
    if record["protocolVersion"] != PROTOCOL
        || !same_json_value(&record["attempt"], &json!(prepared))
        || record["frozenAttemptSha256"] != digest(frozen_raw.get())
        || record["requestSnapshotSha256"] != digest(request_raw.get())
        || record["domainAccepted"] != false || !record["turnId"].is_null()
        || original.turn_id.is_some()
        || original.thread_id.as_deref().is_some_and(|id| record["threadId"].as_str() != Some(id)) {
        return Err("SDK_PASSIVE_REQUEST_BINDING".into());
    }
    let journal_file = dir.join("events.ndjson");
    let journal = read(&journal_file, JOURNAL_LIMIT, true)?;
    if record["journalSha256"] != digest(&journal)
        || (!journal.is_empty() && !journal.ends_with('\n')) {
        return Err("SDK_PASSIVE_JOURNAL_SEAL".into());
    }
    let mut count = 0;
    for line in journal.lines() {
        count += 1;
        if count > EVENT_LIMIT as u64 || line.len() > FRAME_LIMIT { return Err("SDK_PASSIVE_JOURNAL_LIMIT".into()); }
        let event: Value = serde_json::from_str(line).map_err(|_| "SDK_PASSIVE_JOURNAL_JSON")?;
        if event["protocolVersion"] != PROTOCOL || event["attemptId"] != original.attempt_id
            || event["sequence"].as_u64() != Some(count) || !event["turnId"].is_null() {
            return Err("SDK_PASSIVE_JOURNAL_IDENTITY".into());
        }
    }
    if record["lastSequence"].as_u64() != Some(count)
        || text != read(&file, 4_194_304, false)?
        || journal != read(&journal_file, JOURNAL_LIMIT, true)? {
        return Err("SDK_PASSIVE_EVIDENCE_CHANGED".into());
    }
    // Do not expose or adopt unvalidated rawResult/usage/error fields. These
    // receipts are evidence only; the unknown ownership and budget remain held.
    Ok(json!({"attemptId":original.attempt_id,"localSequence":count,
        "receiptEnvelopeSha256":seal,"state":"submission_unknown",
        "readOnly":true,"dispatches":0,"remoteObservationSupported":false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, FrozenAttempt, Value) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("not-installed").to_string_lossy().into_owned();
        let identity = json!({"version":"official-sdk-identity/v2","provider":"codex_sdk",
            "sdkPackage":"@openai/codex-sdk","sdkVersion":SDK_VERSION,"sdkImplementationSha256":SDK_IMPLEMENTATION_SHA,
            "bridgeVersion":"0.1.0","bridgeSha256":"a".repeat(64),"protocolVersion":PROTOCOL,
            "nodeExecutable":path,"nodeSha256":"b".repeat(64),"nodeVersion":"v22.0.0",
            "runtimePackageVersion":"codex-cli 0.162.0-alpha.2","runtimeIdentity":format!("official-codex-runtime/codex-cli 0.162.0-alpha.2/{}","c".repeat(64)),
            "permissionProfile":NATIVE_POLICY,"accountFingerprint":"d".repeat(64),
            "runtimeExecutable":path,"runtimeSha256":"c".repeat(64),"supervisorExecutable":path,
            "supervisorSha256":"e".repeat(64),"resourceDirectory":path,"credentialHome":path,"codexHome":path});
        let request = crate::pipeline::TextRequest { stage:"fixture".into(),skill:"fixture".into(),
            model:"synthetic".into(),reasoning:"medium".into(),input:json!({"text":"仅合成测试"}),
            schema:json!({"type":"object"}),rule_snapshot:json!({}) };
        let prepared = FrozenAttempt::prepare("historical", "original", "node", Backend::CodexSdk, request, &identity).unwrap();
        let frozen = json!(prepared);
        let record = json!({"protocolVersion":PROTOCOL,"attempt":frozen,
            "frozenAttemptSha256":digest(&frozen.to_string()),"requestSnapshotSha256":digest(&frozen["request"].to_string()),
            "journalSha256":digest(""),"lastSequence":0,"threadId":null,"turnId":null,
            "rawResult":null,"state":"submission_unknown","domainAccepted":false});
        let mut original = prepared;
        original.begin_dispatch(Backend::CodexSdk).unwrap();
        original.state = AttemptState::SubmissionUnknown;
        (temp, original, record)
    }
    fn write(root: &Path, record: &Value, journal: &str) -> PathBuf {
        let dir = root.join("sdk-worker/historical");
        std::fs::create_dir_all(&dir).unwrap();
        let text = record.to_string();
        let sealed = format!("{},\"receiptEnvelopeSha256\":\"{}\"}}", &text[..text.len()-1], digest(&text));
        std::fs::write(dir.join("receipt.json"), sealed).unwrap();
        if !journal.is_empty() { std::fs::write(dir.join("events.ndjson"), journal).unwrap(); }
        dir
    }
    #[test]
    fn sdk_passive_old_bridge_read_requires_no_components_and_cannot_dispatch() {
        let (temp, attempt, record) = fixture();
        let dir = write(temp.path(), &record, "");
        let before = std::fs::read(dir.join("receipt.json")).unwrap();
        let settings = json!({"textSdkIdentity":attempt.runtime_identity});
        assert_eq!(receipt_identity(&settings).unwrap(),attempt.runtime_identity);
        assert!(identity(&settings).is_err());
        assert!(Config::from_frozen_identity(&settings, &attempt.runtime_identity, temp.path()).is_err());
        let value = historical_receipt(temp.path(), &attempt).unwrap();
        assert_eq!(value["dispatches"],0);
        assert_eq!(value["state"],"submission_unknown");
        assert_eq!(std::fs::read(dir.join("receipt.json")).unwrap(),before);
        assert!(!temp.path().join("sdk-worker/worker.lock").exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(),1);
    }
    #[test]
    fn sdk_passive_late_completed_receipt_never_promotes_unknown() {
        let (temp, attempt, mut record) = fixture();
        record["state"]=json!("transport_completed");
        record["rawResult"]=json!({"unvalidated":"cannot adopt"});
        write(temp.path(), &record, "");
        let value=historical_receipt(temp.path(),&attempt).unwrap();
        assert_eq!(value["state"],"submission_unknown");
        assert!(value.get("rawResult").is_none());
    }
    #[test]
    fn sdk_passive_rejects_seal_request_thread_and_journal_mismatches() {
        for fault in ["seal","request","attempt-hash","cross-attempt","thread","journal-hash","sequence"] {
            let (temp,mut attempt,mut record)=fixture();
            let mut journal=String::new();
            match fault {
                "request" => record["attempt"]["request"]["input"]=json!({"changed":true}),
                "attempt-hash" => record["frozenAttemptSha256"]=json!("0".repeat(64)),
                "cross-attempt" => record["attempt"]["attemptId"]=json!("another-attempt"),
                "thread" => attempt.thread_id=Some("known-thread".into()),
                "journal-hash" => journal="{}\n".into(),
                "sequence" => record["lastSequence"]=json!(1),
                _=>{}
            }
            let dir=write(temp.path(),&record,&journal);
            if fault=="seal" {
                let file=dir.join("receipt.json");
                std::fs::write(&file,std::fs::read_to_string(&file).unwrap().replace("submission_unknown","transport_completed")).unwrap();
            }
            assert!(historical_receipt(temp.path(),&attempt).is_err(),"{fault}");
            assert!(!temp.path().join("sdk-worker/worker.lock").exists());
        }
    }
    #[test]
    fn sdk_passive_identity_still_rejects_package_permissions_and_path_drift() {
        let (_,attempt,_)=fixture();
        for (field,value) in [("sdkImplementationSha256","changed"),("permissionProfile","unsafe"),("bridgeSha256","bad"),("nodeExecutable","relative.exe")] {
            let mut settings=json!({"textSdkIdentity":attempt.runtime_identity});
            settings["textSdkIdentity"][field]=json!(value);
            assert!(receipt_identity(&settings).is_err(),"{field}");
        }
    }
    #[test]
    fn sdk_passive_historical_supervisor_proof_never_grants_cross_bridge_execution() {
        let (_,attempt,_)=fixture();
        let old=attempt.runtime_identity;
        let mut new=old.clone();new["supervisorSha256"]=json!("f".repeat(64));
        assert!(supervisor_handoff_receipt_identity_bound(&old,&new));
        assert!(!supervisor_handoff_identity_bound(&old,&new));
        new["bridgeSha256"]=json!("b".repeat(64));
        assert!(!supervisor_handoff_receipt_identity_bound(&old,&new));
    }
    #[test]
    fn sdk_passive_js_integral_float_spelling_is_not_request_drift() {
        assert!(same_json_value(&json!({"a":[24.0,107.0]}), &json!({"a":[24,107]})));
        assert!(!same_json_value(&json!({"a":24.0}), &json!({"a":25})));
        assert!(!same_json_value(&json!({"a":24.5}), &json!({"a":24})));
        assert!(!same_json_value(&json!({"a":24.0}), &json!({"a":"24"})));
        assert!(!same_json_value(&json!(false), &json!(0)));
        assert!(!same_json_value(&json!({"a":null}), &json!({})));
        assert!(!same_json_value(&json!(9_007_199_254_740_994.0), &json!(9_007_199_254_740_994_i64)));
        assert!(!same_json_value(&json!(-9_007_199_254_740_994.0), &json!(-9_007_199_254_740_994_i64)));
    }
}
