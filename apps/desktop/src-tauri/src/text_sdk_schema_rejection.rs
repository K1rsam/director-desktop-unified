//! A deterministic provider schema refusal is not a lost response. This module
//! proves the exact terminal refusal from immutable sidecar and parent evidence;
//! it never dispatches, mutates a receipt, or treats a network error as rejection.
use super::*;
use crate::text_transport::AttemptState;

pub(crate) const REJECTION_POLICY: &str = "sdk-strict-schema-rejection/v1";
const LEGACY_BRIDGE: &str = "48af55f1835bd6959ede5801fa8ff2ca9068b267b36ec9f751949efdc9481dc7";
const HELD: &str = "SDK_SCHEMA_REJECTION_HELD: 原拒绝、进程停止或冻结证据不足；保留 UNKNOWN，不重发";
fn hash(value: &Value) -> String { digest(&value.to_string()) }
fn bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| HELD)?;
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() > limit { return Err(HELD.into()); }
    std::fs::read(path).map_err(|_| HELD.into())
}
fn absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}
fn rejection(event: &Value) -> Option<Value> {
    let message = match s(event, "type") {
        "error" => event["message"].as_str()?,
        "turn.failed" => event["error"]["message"].as_str()?,
        _ => return None,
    };
    if message.len() > 16_384 { return None; }
    let value: Value = serde_json::from_str(message).ok()?;
    (value["type"] == "error" && value["status"] == 400
        && value["error"]["type"] == "invalid_request_error"
        && value["error"]["code"] == "invalid_json_schema"
        && value["error"]["param"] == "text.format.schema"
        && s(&value["error"], "message").starts_with("Invalid schema for response_format 'codex_output_schema': "))
        .then_some(value)
}
fn diagnostic(event: &Value) -> bool {
    let item = &event["item"];
    event.as_object().is_some_and(|fields| fields.len() == 2)
        && event["type"] == "item.completed" && item["type"] == "error"
        && item.as_object().is_some_and(|fields| fields.len() == 3)
        && !s(item,"id").is_empty() && s(item,"id").len() <= 128
        && s(item,"id").bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !s(item,"message").is_empty() && s(item,"message").len() <= 4096
}
fn terminal_refusal(journal: &[Value], record: &Value) -> Result<Value> {
    let mut thread = false; let mut turn = false; let mut refused = None;
    for event in journal {
        if event["duplicate"] != false || refused.is_some() { return Err(HELD.into()); }
        let raw = &event["rawEvent"];
        match s(raw,"type") {
            "thread.started" if !thread && !turn && raw["thread_id"] == record["threadId"]
                && !s(raw,"thread_id").is_empty() => thread = true,
            "turn.started" if thread && !turn => turn = true,
            _ if thread && diagnostic(raw) => (),
            _ if thread && turn => { refused = Some(rejection(raw).ok_or(HELD)?); },
            _ => return Err(HELD.into()),
        }
    }
    let value = refused.ok_or(HELD)?;
    let raw = &journal.last().ok_or(HELD)?["rawEvent"];
    if record["error"]["message"] != raw.get("message").cloned().unwrap_or_else(||raw["error"]["message"].clone()) {
        return Err(HELD.into());
    }
    Ok(value)
}
fn parent_events_bound(events: &Value, journal: &[Value], record: &Value) -> bool {
    let Some(rows) = events.as_array() else { return false; };
    let mut ack = None; let mut observation = None; let mut stopped = None;
    let mut stderr = false; let mut index = 0; let mut last_id = 0;
    for row in rows {
        let Some(id) = row["id"].as_i64().filter(|id| *id > last_id) else { return false; }; last_id = id;
        let data = &row["data"];
        match s(row,"type") {
            "sdk_pre_dispatch_queued" if ack.is_none() && data["modelCalls"] == 0
                && data["workflowId"] == record["attempt"]["workflowId"]
                && data["nodeKey"] == record["attempt"]["logicalNodeId"] => (),
            "sdk_pre_dispatch_resumed" if ack.is_none() && data["modelCallsBeforeMarker"] == 0 => (),
            "sdk_dispatch_receipt" if ack.is_none() && observation.is_none() => ack = Some(data),
            "sdk_event" if ack.is_some() && observation.is_none() => {
                if journal.get(index) != Some(data) { return false; } index += 1;
            },
            "sdk_observation" if ack.is_some() && observation.is_none() && index == journal.len() => observation = Some(data),
            "sdk_process_tree" if observation.is_some() && stopped.is_none() => stopped = Some(data),
            "sdk_stderr" if stopped.is_some() && !stderr && data["truncated"] == false => stderr = true,
            "state" => (),
            _ => return false,
        }
    }
    let (Some(ack),Some(observed),Some(stopped)) = (ack,observation,stopped) else { return false; };
    stderr && stopped["processTreeStopProven"] == true && stopped["activeProcesses"] == 0
        && stopped["totalProcesses"].as_u64().is_some_and(|count| count >= 2)
        && ack["ok"] == true && ack["result"]["state"] == "dispatching"
        && ack["requestId"] == observed["requestId"]
        && ack["attemptId"] == record["attempt"]["attemptId"] && observed["attemptId"] == record["attempt"]["attemptId"]
        && ack["protocolVersion"] == PROTOCOL && observed["protocolVersion"] == PROTOCOL
        && journal.iter().all(|event| event["requestId"] == ack["requestId"])
        && ["semanticRequestHash","executionFingerprint","attemptEnvelopeHash"].iter()
            .all(|key| ack["result"][key] == record["attempt"][key] && observed[key] == record["attempt"][key])
        && ["compiledPromptSha256","compiledSchemaSha256"].iter().all(|key| ack["result"][key] == record[key] && observed[key] == record[key])
        && ["state","error","lastSequence","threadId","turnId","rawResult","usage"].iter().all(|key| observed[key] == record[key])
}

/// Read-only evidence; the original SDK receipt, journal and Rust attempt are
/// retained byte-for-byte even after a successor is formally reserved.
pub(crate) fn schema_rejection_evidence(root: &Path, attempt: &FrozenAttempt, events: &Value) -> Result<Value> {
    historical_receipt(root, attempt)?;
    if !matches!(attempt.state, AttemptState::SubmissionUnknown | AttemptState::Rejected)
        || attempt.turn_id.is_some() || attempt.raw_result.is_some()
        || attempt.runtime_identity["permissionProfile"] != NATIVE_POLICY { return Err(HELD.into()); }
    let worker = worker_root(root,&attempt.request)?;
    if !absent(&worker.join("worker.lock")) || !absent(&worker.join("worker.lock.recovery")) { return Err(HELD.into()); }
    let dir = worker.join(&attempt.attempt_id);
    let record_bytes = bytes(&dir.join("receipt.json"),4_194_304)?;
    let record: Value = serde_json::from_slice(&record_bytes).map_err(|_| HELD)?;
    if !["submission_unknown","transport_rejected"].contains(&s(&record,"state"))
        || !["SDK_TURN_FAILED","SDK_SCHEMA_REJECTED"].contains(&s(&record["error"],"code"))
        || !record["rawResult"].is_null() || !record["rawResultSha256"].is_null() || !record["usage"].is_null()
        || record.as_object().is_none_or(|fields|fields.len()!=19) { return Err(HELD.into()); }
    let compiled = crate::text_executor::compile_sdk_attempt_input(attempt)?;
    if record["compiledPrompt"] != compiled.prompt || record["compiledSchema"] != compiled.schema
        || record["compiledPromptSha256"] != digest(&compiled.prompt) || record["compiledSchemaSha256"] != digest(&compiled.schema) { return Err(HELD.into()); }
    let compiled_bytes = bytes(&root.join("text-runs").join(&attempt.attempt_id).join("sdk-compiled.json"),2_097_152)?;
    if serde_json::from_slice::<Value>(&compiled_bytes).map_err(|_|HELD)? != json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":digest(&compiled.prompt),"schemaSha256":digest(&compiled.schema)})
        || !absent(&root.join("text-runs").join(&attempt.attempt_id).join("result.json")) { return Err(HELD.into()); }
    let journal_bytes = bytes(&dir.join("events.ndjson"),JOURNAL_LIMIT as u64)?;
    let journal_text = std::str::from_utf8(&journal_bytes).map_err(|_|HELD)?;
    let journal = journal_text.lines().map(serde_json::from_str).collect::<std::result::Result<Vec<Value>,_>>().map_err(|_|HELD)?;
    if journal.len() < 3 || journal.len() > EVENT_LIMIT || attempt.last_sequence != (journal.len() as u64 * 2 + 1) { return Err(HELD.into()); }
    let refusal = terminal_refusal(&journal,&record)?;
    if !parent_events_bound(events,&journal,&record) { return Err(HELD.into()); }
    let plan_bytes = bytes(&dir.join("native-runtime/runtime-plan.json"),4_194_304)?;
    let plan: Value = serde_json::from_slice(&plan_bytes).map_err(|_|HELD)?;
    if plan["version"] != "sdk-native-runtime/v1" || plan["attemptId"] != attempt.attempt_id
        || plan["identity"] != attempt.runtime_identity || plan["model"] != attempt.request.model || plan["reasoning"] != attempt.request.reasoning
        || plan["semanticRequestHash"] != attempt.semantic_request_hash || plan["executionFingerprint"] != attempt.execution_fingerprint
        || plan["compiledPromptSha256"] != record["compiledPromptSha256"] || plan["compiledSchemaSha256"] != record["compiledSchemaSha256"] { return Err(HELD.into()); }
    let schema: Value = serde_json::from_str(&compiled.schema).map_err(|_|HELD)?;
    let fixed = crate::text_executor::sdk_strict_schema(&schema);
    let unique = s(&refusal["error"],"message").ends_with(".uniqueItems' is not permitted when strict is true.")
        && fixed != schema && attempt.compiled_input_format.as_deref() != Some("rules-first-json-strict/v2");
    let evidence = json!({"version":REJECTION_POLICY,"attemptId":attempt.attempt_id,"attemptEnvelopeHash":attempt.attempt_envelope_hash,
        "receiptSha256":crate::store::hash_file(&dir.join("receipt.json"))?.0,"receiptSeal":record["receiptEnvelopeSha256"],
        "journalSha256":record["journalSha256"],"eventsSha256":hash(events),"events":events,
        "attemptFileSha256":crate::store::hash_file(&root.join("text-runs").join(&attempt.attempt_id).join("sdk-attempt.json"))?.0,
        "compiledFileSha256":crate::store::hash_file(&root.join("text-runs").join(&attempt.attempt_id).join("sdk-compiled.json"))?.0,
        "runtimePlanSha256":crate::store::hash_file(&dir.join("native-runtime/runtime-plan.json"))?.0,
        "providerRejection":refusal,"processTreeStopProven":true,"modelOutputEvents":0,
        "retryableUniqueItems":unique,"correctedSchemaSha256":digest(&fixed.to_string())});
    if record_bytes != bytes(&dir.join("receipt.json"),4_194_304)? || journal_bytes != bytes(&dir.join("events.ndjson"),JOURNAL_LIMIT as u64)?
        || plan_bytes != bytes(&dir.join("native-runtime/runtime-plan.json"),4_194_304)? { return Err(HELD.into()); }
    Ok(evidence)
}

/// Only the known schema compiler bridge, or this already-corrected bridge,
/// may hand off. No account, model, SDK, Node, runtime or permission change.
pub(crate) fn schema_rejection_identity_bound(old: &Value, new: &Value) -> bool {
    let (Ok(a),Ok(b)) = (serde_json::from_value::<SdkIdentity>(old.clone()),serde_json::from_value::<SdkIdentity>(new.clone())) else { return false; };
    if a.version != "official-sdk-identity/v2" || a.validate_receipt().is_err() || b.validate_receipt().is_err()
        || ![LEGACY_BRIDGE,compiled_native_bridge_sha().as_str()].contains(&a.bridge_sha256.as_str())
        || b.bridge_sha256 != compiled_native_bridge_sha() { return false; }
    let mut expected = old.clone();
    for field in ["bridgeSha256","supervisorExecutable","supervisorSha256","resourceDirectory"] { expected[field] = new[field].clone(); }
    expected == *new
}
/// Historical proof validation is distinct from permission to execute today's
/// package. A later bridge upgrade must not make adopted raw/results unreadable.
pub(crate) fn schema_rejection_receipt_identity_bound(old: &Value, new: &Value) -> bool {
    let (Ok(a),Ok(b)) = (serde_json::from_value::<SdkIdentity>(old.clone()),serde_json::from_value::<SdkIdentity>(new.clone())) else { return false; };
    if a.version != "official-sdk-identity/v2" || b.version != "official-sdk-identity/v2"
        || a.validate_receipt().is_err() || b.validate_receipt().is_err() { return false; }
    let mut expected=old.clone();
    for field in ["bridgeSha256","supervisorExecutable","supervisorSha256","resourceDirectory"] {expected[field]=new[field].clone();}
    expected==*new
}

#[cfg(test)]
pub(crate) fn test_refusal_identity(root: &Path) -> Value {
    let file=root.join("never-executed.exe").to_string_lossy().into_owned();
    json!({"version":"official-sdk-identity/v2","provider":"codex_sdk","sdkPackage":"@openai/codex-sdk","sdkVersion":SDK_VERSION,
        "sdkImplementationSha256":SDK_IMPLEMENTATION_SHA,"bridgeVersion":"0.1.0","bridgeSha256":compiled_native_bridge_sha(),"protocolVersion":PROTOCOL,
        "nodeExecutable":file,"nodeSha256":"a".repeat(64),"nodeVersion":"v24.19.0","runtimePackageVersion":"codex-cli 0.162.0-alpha.2",
        "runtimeIdentity":format!("official-codex-runtime/codex-cli 0.162.0-alpha.2/{}","b".repeat(64)),"permissionProfile":NATIVE_POLICY,
        "accountFingerprint":"c".repeat(64),"runtimeExecutable":file,"runtimeSha256":"b".repeat(64),"supervisorExecutable":file,
        "supervisorSha256":"d".repeat(64),"resourceDirectory":root,"credentialHome":root,"codexHome":root})
}
#[cfg(test)]
pub(crate) fn test_refusal_files(root:&Path,prepared:&FrozenAttempt)->(FrozenAttempt,Value) {
    let compiled=crate::text_executor::compile_sdk_attempt_input(prepared).unwrap();
    let dir=root.join("sdk-worker").join(&prepared.attempt_id);std::fs::create_dir_all(dir.join("native-runtime")).unwrap();
    let text_dir=root.join("text-runs").join(&prepared.attempt_id);std::fs::create_dir_all(&text_dir).unwrap();
    std::fs::write(text_dir.join("sdk-compiled.json"),json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":digest(&compiled.prompt),"schemaSha256":digest(&compiled.schema)}).to_string()).unwrap();
    let error=json!({"type":"error","message":json!({"type":"error","status":400,"error":{"type":"invalid_request_error","code":"invalid_json_schema","param":"text.format.schema","message":"Invalid schema for response_format 'codex_output_schema': 'schema.properties.assets.uniqueItems' is not permitted when strict is true."}}).to_string()});
    let raw_events=vec![json!({"type":"thread.started","thread_id":"synthetic-thread"}),json!({"type":"turn.started"}),error.clone()];
    let journal:Vec<Value>=raw_events.into_iter().enumerate().map(|(i,event)|json!({"protocolVersion":PROTOCOL,"requestId":"rust_2","attemptId":prepared.attempt_id,
        "sequence":i+1,"threadId":"synthetic-thread","turnId":null,"type":"sdk_event","duplicate":false,"rawEvent":event})).collect();
    let journal_text=journal.iter().map(|event|format!("{event}\n")).collect::<String>();std::fs::write(dir.join("events.ndjson"),&journal_text).unwrap();
    let record=json!({"protocolVersion":PROTOCOL,"attempt":prepared,"compiledPrompt":compiled.prompt,"compiledSchema":compiled.schema,
        "compiledPromptSha256":digest(&compiled.prompt),"compiledSchemaSha256":digest(&compiled.schema),"requestSnapshotSha256":hash(&json!(prepared.request)),
        "frozenAttemptSha256":hash(&json!(prepared)),"journalSha256":digest(&journal_text),"rawResultSha256":null,"state":"submission_unknown",
        "threadId":"synthetic-thread","turnId":null,"lastSequence":3,"rawResult":null,"usage":null,"error":{"code":"SDK_TURN_FAILED","message":error["message"]},"domainAccepted":false});
    let text=record.to_string();std::fs::write(dir.join("receipt.json"),format!("{},\"receiptEnvelopeSha256\":\"{}\"}}",&text[..text.len()-1],digest(&text))).unwrap();
    let plan=json!({"version":"sdk-native-runtime/v1","attemptId":prepared.attempt_id,"identity":prepared.runtime_identity,"model":prepared.request.model,"reasoning":prepared.request.reasoning,
        "semanticRequestHash":prepared.semantic_request_hash,"executionFingerprint":prepared.execution_fingerprint,"compiledPromptSha256":record["compiledPromptSha256"],"compiledSchemaSha256":record["compiledSchemaSha256"]});
    std::fs::write(dir.join("native-runtime/runtime-plan.json"),plan.to_string()).unwrap();
    let mut public=json!({"protocolVersion":PROTOCOL,"requestId":"rust_2","attemptId":prepared.attempt_id,"semanticRequestHash":prepared.semantic_request_hash,"executionFingerprint":prepared.execution_fingerprint,"attemptEnvelopeHash":prepared.attempt_envelope_hash,
        "compiledPromptSha256":record["compiledPromptSha256"],"compiledSchemaSha256":record["compiledSchemaSha256"],"state":"dispatching"});
    let mut rows=vec![json!({"id":1,"type":"sdk_dispatch_receipt","data":{"protocolVersion":PROTOCOL,"requestId":"rust_2","attemptId":prepared.attempt_id,"ok":true,"result":public}})];
    for event in journal {rows.push(json!({"id":rows.len()+1,"type":"sdk_event","data":event}));}
    for key in ["state","error","lastSequence","threadId","turnId","rawResult","usage"] {public[key]=record[key].clone();}
    rows.push(json!({"id":5,"type":"sdk_observation","data":public}));rows.push(json!({"id":6,"type":"sdk_process_tree","data":{"processTreeStopProven":true,"activeProcesses":0,"totalProcesses":5}}));
    rows.push(json!({"id":7,"type":"sdk_stderr","data":{"truncated":false,"bytes":0}}));
    let mut attempt=prepared.clone();attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.state=AttemptState::SubmissionUnknown;attempt.thread_id=Some("synthetic-thread".into());attempt.last_sequence=7;
    attempt.persist(&text_dir.join("sdk-attempt.json")).unwrap();(attempt,json!(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn refused() -> Value { json!({"type":"error","message":json!({"type":"error","status":400,"error":{"type":"invalid_request_error","code":"invalid_json_schema","param":"text.format.schema","message":"Invalid schema for response_format 'codex_output_schema': 'schema.properties.assets.uniqueItems' is not permitted when strict is true."}}).to_string()}) }
    #[test]
    fn sdk_schema_rejection_requires_exact_api_contract_not_network_text() {
        assert!(rejection(&refused()).is_some());
        for (path,value) in [("status",json!(500)),("code",json!("rate_limit")),("type",json!("server_error")),("param",json!("input"))] {
            let mut event=refused();let mut body:Value=serde_json::from_str(s(&event,"message")).unwrap();
            if path=="status" {body[path]=value;}else{body["error"][path]=value;}event["message"]=json!(body.to_string());assert!(rejection(&event).is_none());
        }
        for text in ["tls handshake eof","Reconnecting... 2/5 (stream disconnected before completion: tls handshake eof)","invalid_json_schema status 400"] {
            assert!(rejection(&json!({"type":"error","message":text})).is_none());
        }
    }
    #[test]
    fn sdk_schema_rejection_no_output_reconnect_duplicates_or_late_events() {
        let error=refused();let record=json!({"threadId":"thread","error":{"message":error["message"]}});
        let journal=vec![json!({"duplicate":false,"rawEvent":{"type":"thread.started","thread_id":"thread"}}),json!({"duplicate":false,"rawEvent":{"type":"turn.started"}}),json!({"duplicate":false,"rawEvent":error})];
        assert!(terminal_refusal(&journal,&record).is_ok());
        for event in [json!({"type":"item.started","item":{"id":"r","type":"reasoning"}}),json!({"type":"item.completed","item":{"id":"a","type":"agent_message","text":"partial"}}),json!({"type":"error","message":"Reconnecting... 1/5 (stream disconnected before completion: tls handshake eof)"}),json!({"type":"turn.completed","usage":{}})] {
            let mut changed=journal.clone();changed.insert(2,json!({"duplicate":false,"rawEvent":event}));assert!(terminal_refusal(&changed,&record).is_err());
        }
        let mut duplicate=journal.clone();duplicate[1]["duplicate"]=json!(true);assert!(terminal_refusal(&duplicate,&record).is_err());
        let mut after=journal.clone();after.push(journal[1].clone());assert!(terminal_refusal(&after,&record).is_err());
    }
    fn files()->(tempfile::TempDir,FrozenAttempt,Value) {
        let temp=tempfile::tempdir().unwrap();let identity=test_refusal_identity(temp.path());
        let request=crate::pipeline::TextRequest{stage:"fixture".into(),skill:"fixture".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input:json!({"synthetic":true}),schema:json!({"type":"object","properties":{"assets":{"type":"array","items":{"type":"string"},"uniqueItems":true}}}),rule_snapshot:json!({})};
        let prepared=FrozenAttempt::prepare_sdk_rules_first("origin","workflow","node",request,&identity).unwrap();
        let(attempt,events)=test_refusal_files(temp.path(),&prepared);(temp,attempt,events)
    }
    #[test]
    fn sdk_schema_refusal_full_sealed_evidence_is_read_only_and_reopens() {
        let(temp,attempt,events)=files();let dir=temp.path().join("sdk-worker/origin");let before=std::fs::read(dir.join("receipt.json")).unwrap();
        let first=schema_rejection_evidence(temp.path(),&attempt,&events).unwrap();assert_eq!(first["retryableUniqueItems"],true);assert_eq!(first["modelOutputEvents"],0);
        let reopened=FrozenAttempt::load(&temp.path().join("text-runs/origin/sdk-attempt.json")).unwrap();
        assert_eq!(schema_rejection_evidence(temp.path(),&reopened,&events).unwrap(),first);assert_eq!(std::fs::read(dir.join("receipt.json")).unwrap(),before);
    }
    #[test]
    fn sdk_schema_refusal_missing_stop_event_and_modified_files_are_held() {
        for fault in ["stop","active","event","observation","compiled","plan","raw","lock","receipt","journal"] {
            let(temp,attempt,mut events)=files();let dir=temp.path().join("sdk-worker/origin");
            match fault {
                "stop"=>events[5]["data"]["processTreeStopProven"]=json!(false),"active"=>events[5]["data"]["activeProcesses"]=json!(1),
                "event"=>events[2]["data"]["rawEvent"]["type"]=json!("unexpected"),"observation"=>events[4]["data"]["requestId"]=json!("other"),
                "compiled"=>std::fs::write(temp.path().join("text-runs/origin/sdk-compiled.json"),"{}").unwrap(),
                "plan"=>std::fs::write(dir.join("native-runtime/runtime-plan.json"),"{}").unwrap(),
                "raw"=>std::fs::write(temp.path().join("text-runs/origin/result.json"),"{}").unwrap(),
                "lock"=>std::fs::write(temp.path().join("sdk-worker/worker.lock"),"{}").unwrap(),
                "receipt"=>std::fs::write(dir.join("receipt.json"),"{}").unwrap(),
                "journal"=>std::fs::write(dir.join("events.ndjson"),"{}\n").unwrap(),_=>unreachable!(),
            }
            assert!(schema_rejection_evidence(temp.path(),&attempt,&events).is_err(),"{fault}");
        }
    }
    #[test]
    fn sdk_schema_historical_receipt_binding_never_grants_cross_bridge_execution() {
        let temp=tempfile::tempdir().unwrap();let mut old=test_refusal_identity(temp.path());old["bridgeSha256"]=json!(LEGACY_BRIDGE);
        let mut adopted=old.clone();adopted["bridgeSha256"]=json!("f".repeat(64));adopted["supervisorSha256"]=json!("e".repeat(64));
        assert!(schema_rejection_receipt_identity_bound(&old,&adopted));
        assert!(!schema_rejection_identity_bound(&old,&adopted));
        for field in ["accountFingerprint","sdkVersion","nodeSha256","runtimeSha256","permissionProfile"] {
            let mut changed=adopted.clone();changed[field]=json!("1".repeat(64));assert!(!schema_rejection_receipt_identity_bound(&old,&changed),"{field}");
        }
    }
}
