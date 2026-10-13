//! Narrow evidence for the known pre-SDK Windows path spelling failure.
//! This module never dispatches, changes state or authorizes another attempt.
use super::*;
use crate::text_transport::AttemptState;
pub(crate) const LOCAL_PATH_BRIDGE_SHA: &str = "fc252738882b4fcdca3a52aeed2c50e1613b290ff3297e7f5cfd12f6b108b08a";
const HELD: &str = "SDK_LOCAL_PREPARATION_RECOVERY_HELD: 原任务缺少可证明的本地预备失败；保留未知状态与预算，不重发";

pub(crate) fn local_recovery_identity_bound(old: &Value, new: &Value) -> bool {
    let (Ok(a), Ok(b)) = (serde_json::from_value::<SdkIdentity>(old.clone()), serde_json::from_value::<SdkIdentity>(new.clone())) else { return false; };
    if a.version != "official-sdk-identity/v2" || a.bridge_sha256 != LOCAL_PATH_BRIDGE_SHA
        || a.validate_receipt().is_err() || b.validate_receipt().is_err()
        || ![compiled_native_bridge_sha().as_str(), WRAPPER_FAILED_BRIDGE_SHA].contains(&b.bridge_sha256.as_str()) { return false; }
    let mut expected = old.clone();
    for field in ["bridgeSha256", "supervisorExecutable", "supervisorSha256", "resourceDirectory"] { expected[field] = new[field].clone(); }
    expected == *new
}
fn absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}
fn bounded(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| HELD)?;
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() > 4_194_304 { return Err(HELD.into()); }
    std::fs::read_to_string(path).map_err(|_| HELD.into())
}
pub(crate) fn recovered_receipt_bound(root: &Path, attempt: &FrozenAttempt, saved: &Value) -> Result<()> {
    let dir=root.join("sdk-worker").join(&attempt.attempt_id);
    for path in [root.join("sdk-worker"),dir.clone()] {
        let meta=std::fs::symlink_metadata(path).map_err(|_|HELD)?;
        if !meta.is_dir() || meta.is_symlink(){return Err(HELD.into());}
    }
    historical_receipt(root, attempt)?;
    let original=bounded(&dir.join("local-preparation-original.json"))?;
    if digest(&original)!=s(&saved["evidence"],"originalReceiptSha256") {return Err(HELD.into());}
    let record:Value=serde_json::from_str(&bounded(&dir.join("receipt.json"))?).map_err(|_|HELD)?;
    let intent:Value=serde_json::from_str(&bounded(&dir.join("local-preparation-recovery.json"))?).map_err(|_|HELD)?;
    if attempt.state!=AttemptState::Completed || attempt.raw_result.as_ref()!=Some(&record["rawResult"])
        || record["state"]!="transport_completed" || !record["error"].is_null()
        || record["preDispatchRecovery"]!=json!({"proof":saved["proof"],"effectiveIdentity":saved["effectiveIdentity"]})
        || intent!=saved["proof"] {return Err(HELD.into());}
    Ok(())
}
/// `events` must be the unmodified database journal ordered by id; its hashes
/// are later frozen atomically by the executor before this proof is consumed.
pub(crate) fn local_preparation_evidence(root: &Path, original: &FrozenAttempt, events: &Value) -> Result<Value> {
    let historical = historical_receipt(root, original)?;
    if original.state != AttemptState::SubmissionUnknown || original.last_sequence > 1
        || original.thread_id.is_some() || original.turn_id.is_some() || original.raw_result.is_some()
        || original.runtime_identity["bridgeSha256"] != LOCAL_PATH_BRIDGE_SHA { return Err(HELD.into()); }
    let worker = root.join("sdk-worker"); let dir = worker.join(&original.attempt_id);
    for path in [&worker, &dir] {
        let meta = std::fs::symlink_metadata(path).map_err(|_| HELD)?;
        if !meta.is_dir() || meta.is_symlink() { return Err(HELD.into()); }
    }
    for path in [worker.join("worker.lock"), worker.join("worker.lock.recovery"), dir.join("native-runtime"), dir.join("events.ndjson"), dir.join("local-preparation-recovery.json"), dir.join("local-preparation-original.json")] {
        if !absent(&path) { return Err(HELD.into()); }
    }
    if std::fs::read_dir(&dir).map_err(|_| HELD)?.count() != 1 { return Err(HELD.into()); }
    let file = dir.join("receipt.json"); let bytes = bounded(&file)?;
    let record: Value = serde_json::from_str(&bytes).map_err(|_| HELD)?;
    let compiled = crate::text_executor::compile_sdk_attempt_input(original)?;
    let expected_error = json!({"code":"SDK_STREAM_ERROR","message":"SDK_NATIVE_IDENTITY: attempt directory"});
    if record["state"] != "submission_unknown" || record["error"] != expected_error || record["lastSequence"] != 0
        || !record["threadId"].is_null() || !record["turnId"].is_null() || !record["rawResult"].is_null()
        || !record["usage"].is_null() || !record["rawResultSha256"].is_null() || record.get("preDispatchRecovery").is_some()
        || record["compiledPrompt"] != compiled.prompt || record["compiledSchema"] != compiled.schema
        || record["compiledPromptSha256"] != digest(&compiled.prompt) || record["compiledSchemaSha256"] != digest(&compiled.schema) { return Err(HELD.into()); }
    let rows = events.as_array().ok_or(HELD)?;
    let mut ack = None; let mut observed = None; let mut stopped = None; let mut previous_id = 0;
    for row in rows {
        let event_id = row["id"].as_i64().ok_or(HELD)?;
        if event_id <= previous_id { return Err(HELD.into()); } previous_id = event_id;
        match s(row, "type") {
            "sdk_dispatch_receipt" if ack.is_none() && observed.is_none() => ack = Some(&row["data"]),
            "sdk_observation" if ack.is_some() && observed.is_none() && stopped.is_none() => observed = Some(&row["data"]),
            "sdk_process_tree" if observed.is_some() && stopped.is_none() => stopped = Some(&row["data"]),
            "sdk_stderr" if stopped.is_some() && row["data"]["bytes"] == 0 && row["data"]["retainedBytes"] == 0 && row["data"]["truncated"] == false => (),
            "state" => (),
            _ => return Err(HELD.into()),
        }
    }
    let (Some(ack),Some(observed),Some(stopped))=(ack,observed,stopped) else {return Err(HELD.into());};
    if ack["ok"] != true || ack["result"]["state"] != "dispatching" || ack["requestId"] != observed["requestId"]
        || ack["attemptId"] != original.attempt_id || observed["attemptId"] != original.attempt_id
        || ack["protocolVersion"] != PROTOCOL || observed["protocolVersion"] != PROTOCOL
        || stopped["processTreeStopProven"] != true || stopped["activeProcesses"] != 0 {return Err(HELD.into());}
    for (key, expected) in [("semanticRequestHash",json!(original.semantic_request_hash)),("executionFingerprint",json!(original.execution_fingerprint)),("attemptEnvelopeHash",json!(original.attempt_envelope_hash)),("compiledPromptSha256",json!(digest(&compiled.prompt))),("compiledSchemaSha256",json!(digest(&compiled.schema)))] {
        if observed[key] != expected || ack["result"][key] != expected {return Err(HELD.into());}
    }
    for key in ["state","error","lastSequence","threadId","turnId","rawResult","usage"] { if observed[key] != record[key] {return Err(HELD.into());} }
    if bounded(&file)? != bytes {return Err(HELD.into());}
    Ok(json!({"version":"sdk-local-preparation-evidence/v1","attemptId":original.attempt_id,
        "attemptEnvelopeHash":original.attempt_envelope_hash,"originalReceiptSeal":historical["receiptEnvelopeSha256"],
        "originalReceiptSha256":digest(&bytes),"eventsSha256":digest(&events.to_string()),
        "sdkEvents":0,"processTreeStopProven":true,"nativeRuntimeAbsent":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(crate) fn fixture() -> (tempfile::TempDir,FrozenAttempt,Value,Value) {
        let temp=tempfile::tempdir().unwrap();let p=temp.path().join("synthetic-not-executed").to_string_lossy().into_owned();
        let identity=json!({"version":"official-sdk-identity/v2","provider":"codex_sdk","sdkPackage":"@openai/codex-sdk","sdkVersion":SDK_VERSION,"sdkImplementationSha256":SDK_IMPLEMENTATION_SHA,
            "bridgeVersion":"0.1.0","bridgeSha256":LOCAL_PATH_BRIDGE_SHA,"protocolVersion":PROTOCOL,"nodeExecutable":p,"nodeSha256":"a".repeat(64),"nodeVersion":"v24.19.0",
            "runtimePackageVersion":"codex-cli synthetic","runtimeIdentity":format!("official-codex-runtime/codex-cli synthetic/{}","b".repeat(64)),"permissionProfile":NATIVE_POLICY,
            "accountFingerprint":"c".repeat(64),"runtimeExecutable":p,"runtimeSha256":"b".repeat(64),"supervisorExecutable":p,"supervisorSha256":"d".repeat(64),"resourceDirectory":p,"credentialHome":p,"codexHome":p});
        let request=crate::pipeline::TextRequest{stage:"fixture".into(),skill:"fixture".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input:json!({"synthetic":true}),schema:json!({"type":"object"}),rule_snapshot:json!({})};
        let mut original=FrozenAttempt::prepare_sdk_rules_first("local-preparation","original-workflow","node",request,&identity).unwrap();
        let compiled=crate::text_executor::compile_sdk_attempt_input(&original).unwrap();let frozen=json!(original);
        let record=json!({"protocolVersion":PROTOCOL,"attempt":frozen,"frozenAttemptSha256":digest(&frozen.to_string()),"requestSnapshotSha256":digest(&frozen["request"].to_string()),
            "compiledPrompt":compiled.prompt,"compiledSchema":compiled.schema,"compiledPromptSha256":digest(&compiled.prompt),"compiledSchemaSha256":digest(&compiled.schema),
            "state":"submission_unknown","error":{"code":"SDK_STREAM_ERROR","message":"SDK_NATIVE_IDENTITY: attempt directory"},"threadId":null,"turnId":null,"lastSequence":0,
            "rawResult":null,"rawResultSha256":null,"usage":null,"journalSha256":digest(""),"domainAccepted":false});
        let mut receipt=json!({"attemptId":original.attempt_id,"semanticRequestHash":original.semantic_request_hash,"executionFingerprint":original.execution_fingerprint,"attemptEnvelopeHash":original.attempt_envelope_hash,
            "compiledPromptSha256":record["compiledPromptSha256"],"compiledSchemaSha256":record["compiledSchemaSha256"],"state":"dispatching","threadId":null,"turnId":null,"lastSequence":0,"rawResult":null,"usage":null,"error":null});
        let ack=json!({"attemptId":original.attempt_id,"requestId":"rust_2","protocolVersion":PROTOCOL,"ok":true,"result":receipt});
        receipt["state"]=record["state"].clone();receipt["error"]=record["error"].clone();receipt["requestId"]=json!("rust_2");receipt["protocolVersion"]=json!(PROTOCOL);
        let events=json!([{"id":1,"type":"state","data":{"state":"submitting"}},{"id":2,"type":"sdk_dispatch_receipt","data":ack},
            {"id":3,"type":"sdk_observation","data":receipt},{"id":4,"type":"sdk_process_tree","data":{"processTreeStopProven":true,"activeProcesses":0}},
            {"id":5,"type":"sdk_stderr","data":{"bytes":0,"retainedBytes":0,"truncated":false}},{"id":6,"type":"state","data":{"state":"submission_unknown"}}]);
        original.begin_dispatch(Backend::CodexSdk).unwrap();original.state=AttemptState::SubmissionUnknown;original.last_sequence=1;
        write(temp.path(),&original,&record);
        (temp,original,record,events)
    }
    pub(crate) fn write(root:&Path,attempt:&FrozenAttempt,record:&Value) {
        let dir=root.join("sdk-worker").join(&attempt.attempt_id);std::fs::create_dir_all(&dir).unwrap();
        let text=record.to_string();let sealed=format!("{},\"receiptEnvelopeSha256\":\"{}\"}}",&text[..text.len()-1],digest(&text));
        std::fs::write(dir.join("receipt.json"),sealed).unwrap();
    }
    #[test]
    fn sdk_local_preparation_exact_original_proof_is_read_only_and_retains_unknown() {
        let (temp,attempt,_,events)=fixture();let path=temp.path().join("sdk-worker/local-preparation/receipt.json");let before=std::fs::read(&path).unwrap();
        let proof=local_preparation_evidence(temp.path(),&attempt,&events).unwrap();
        assert_eq!(proof["sdkEvents"],0);assert_eq!(proof["processTreeStopProven"],true);assert_eq!(attempt.state,AttemptState::SubmissionUnknown);
        assert_eq!(std::fs::read(path).unwrap(),before);assert_eq!(std::fs::read_dir(temp.path().join("sdk-worker/local-preparation")).unwrap().count(),1);
    }
    #[test]
    fn sdk_local_preparation_rejects_ambiguous_errors_events_locks_runtime_and_identity_drift() {
        for fault in ["tls","thread","raw","compiled","journal","runtime","lock","reserved","foreign_ack","stop_false","active","sdk_event","duplicate","bridge","order","sequence"] {
            let (temp,mut attempt,mut record,mut events)=fixture();let root=temp.path();
            match fault {
                "tls"=>record["error"]["message"]=json!("tls handshake eof"),"thread"=>record["threadId"]=json!("provider-thread"),"raw"=>record["rawResult"]=json!({"text":"partial"}),
                "compiled"=>record["compiledPrompt"]=json!("changed"),"journal"=>std::fs::write(root.join("sdk-worker/local-preparation/events.ndjson"),"").unwrap(),
                "runtime"=>std::fs::create_dir(root.join("sdk-worker/local-preparation/native-runtime")).unwrap(),"lock"=>std::fs::write(root.join("sdk-worker/worker.lock"),"{}").unwrap(),
                "reserved"=>std::fs::write(root.join("sdk-worker/local-preparation/local-preparation-recovery.json"),"{}").unwrap(),
                "foreign_ack"=>events[1]["data"]["attemptId"]=json!("other"),"stop_false"=>events[3]["data"]["processTreeStopProven"]=json!(false),"active"=>events[3]["data"]["activeProcesses"]=json!(1),
                "sdk_event"=>events.as_array_mut().unwrap().push(json!({"id":7,"type":"sdk_event","data":{}})),
                "duplicate"=>{let mut v=events[1].clone();v["id"]=json!(7);events.as_array_mut().unwrap().push(v);},
                "bridge"=>attempt.runtime_identity["bridgeSha256"]=json!("f".repeat(64)),"order"=>events[3]["id"]=json!(2),"sequence"=>attempt.last_sequence=2,_=>unreachable!(),
            }
            write(root,&attempt,&record);assert!(local_preparation_evidence(root,&attempt,&events).is_err(),"{fault}");
        }
    }
    #[test]
    fn sdk_local_recovery_handoff_only_changes_verified_bridge_supervisor_and_resources() {
        let (_,original,_,_)=fixture();let old=original.runtime_identity;let mut current=old.clone();current["bridgeSha256"]=json!(compiled_native_bridge_sha());
        current["supervisorSha256"]=json!("e".repeat(64));assert!(local_recovery_identity_bound(&old,&current));
        for (field,value) in [("accountFingerprint",json!("e".repeat(64))),("nodeSha256",json!("e".repeat(64))),("sdkVersion",json!("other")),("runtimeSha256",json!("e".repeat(64))),("permissionProfile",json!("unsafe")),("credentialHome",json!("C:/other"))] {
            let mut changed=current.clone();changed[field]=value;assert!(!local_recovery_identity_bound(&old,&changed),"{field}");
        }
        let mut unknown=old;unknown["bridgeSha256"]=json!("f".repeat(64));assert!(!local_recovery_identity_bound(&unknown,&current));
    }
}
