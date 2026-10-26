//! Evidence and one-use binding for a pinned SDK schema mkdtemp failure.
//! No empty journal or error string alone grants permission to execute.
use super::*;
use crate::text_transport::AttemptState;

pub(crate) const SCHEMA_FAILED_BRIDGE_SHA: &str = "48af55f1835bd6959ede5801fa8ff2ca9068b267b36ec9f751949efdc9481dc7";
pub(crate) const SCHEMA_RECOVERY_VERSION: &str = "sdk-schema-preparation-recovery/v1";
pub(crate) const SCHEMA_RECOVERY_ROOT: &str = "native-runtime-schema-recovery";
const HELD: &str = "SDK_SCHEMA_PREPARATION_RECOVERY_HELD: 原准备失败证据或一次性恢复链不完整；保留原未知请求，不重发";

fn absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}
fn bytes(path: &Path) -> Result<Vec<u8>> {
    let m=std::fs::symlink_metadata(path).map_err(|_|HELD)?;
    if !m.is_file() || m.is_symlink() || m.len()>4_194_304 {return Err(HELD.into());}
    std::fs::read(path).map_err(|_|HELD.into())
}
fn read(path: &Path) -> Result<Value> {serde_json::from_slice(&bytes(path)?).map_err(|_|HELD.into())}
fn hash(value: &Value) -> String {digest(&value.to_string())}
fn schema_template(runtime: &Path) -> Result<String> {
    // Node path.join normalizes Rust's possible forward-slash spelling. Use
    // the same component-preserving conversion, then remove only its namespace.
    Ok(node_path_argument(&node_state_path_argument(&runtime.join("tmp").join("codex-output-schema-XXXXXX"))?)?.to_string_lossy().into_owned())
}
fn directory(path: &Path, expected: &[&str]) -> Result<()> {
    let m=std::fs::symlink_metadata(path).map_err(|_|HELD)?;
    if !m.is_dir() || m.is_symlink() {return Err(HELD.into());}
    let mut names=std::fs::read_dir(path).map_err(|_|HELD)?.map(|e|e.map(|e|e.file_name().to_string_lossy().into_owned()).map_err(|_|HELD)).collect::<std::result::Result<Vec<_>,_>>()?;
    names.sort(); let mut expected:Vec<_>=expected.iter().map(|s|s.to_string()).collect();expected.sort();
    if names!=expected {return Err(HELD.into());}Ok(())
}
pub(crate) fn schema_runtime_inventory(runtime: &Path) -> Result<Value> {
    directory(runtime,&["runtime-plan.json","tmp","work"])?;
    directory(&runtime.join("tmp"),&[])?;
    directory(&runtime.join("work"),&[".director-text-root"])?;
    let mut list=Vec::new();
    for name in ["runtime-plan.json","work/.director-text-root"] {
        let body=bytes(&runtime.join(name))?;
        if name=="work/.director-text-root" && body!=b"director SDK text execution\n" {return Err(HELD.into());}
        list.push(json!([name,body.len(),crate::store::hash_file(&runtime.join(name))?.0]));
    }Ok(json!(list))
}
pub(crate) fn schema_recovery_identity_bound(old: &Value, new: &Value) -> bool {
    let (Ok(a),Ok(b))=(serde_json::from_value::<SdkIdentity>(old.clone()),serde_json::from_value::<SdkIdentity>(new.clone())) else{return false;};
    if a.bridge_sha256!=SCHEMA_FAILED_BRIDGE_SHA || b.bridge_sha256!=SCHEMA_FAILED_BRIDGE_SHA
        || a.validate_receipt().is_err() || b.validate_receipt().is_err() {return false;}
    let mut expected=old.clone();for field in ["supervisorExecutable","supervisorSha256","resourceDirectory"] {expected[field]=new[field].clone();}
    expected==*new
}

pub(crate) fn schema_recovery_record_bound(dir: &Path, record: &Value, effective: &Value) -> Result<bool> {
    let Some(recovery)=record.get("schemaPreparationRecovery") else{return Ok(false);};let proof=&recovery["proof"];
    if recovery.as_object().is_none_or(|m|m.len()!=2) || record.get("preDispatchRecovery").is_some() || record.get("wrapperPreparationRepair").is_some()
        || recovery["effectiveIdentity"]!=*effective || !schema_recovery_identity_bound(&record["attempt"]["runtimeIdentity"],effective)
        || proof["version"]!=SCHEMA_RECOVERY_VERSION || proof["attemptId"]!=record["attempt"]["attemptId"] || proof["attemptEnvelopeHash"]!=record["attempt"]["attemptEnvelopeHash"]
        || proof["effectiveIdentitySha256"]!=hash(effective) || proof["sdkEvents"]!=0 || proof["processTreeStopProven"]!=true || proof["beforeOfficialRuntimeSpawn"]!=true
        || proof["controllerSha256"]!=schema_controller_sha256()
        || read(&dir.join("schema-preparation-recovery.json"))?!=*proof {return Ok(false);}
    let mut attempt:FrozenAttempt=serde_json::from_value(record["attempt"].clone()).map_err(|_|HELD)?;attempt.begin_dispatch(Backend::CodexSdk)?;
    historical_receipt(dir.parent().and_then(Path::parent).ok_or(HELD)?,&attempt)?;
    let original_bytes=bytes(&dir.join("schema-preparation-original.json"))?;
    let text=std::str::from_utf8(&original_bytes).map_err(|_|HELD)?;let original:Value=serde_json::from_slice(&original_bytes).map_err(|_|HELD)?;
    let seal=s(&original,"receiptEnvelopeSha256");let suffix=format!(",\"receiptEnvelopeSha256\":\"{seal}\"}}");
    let sealed=text.strip_suffix(&suffix).is_some_and(|p|digest(&format!("{p}}}"))==seal);
    let evidence=read(&dir.join("schema-preparation-evidence.json"))?;
    let list=schema_runtime_inventory(&dir.join("native-runtime"))?;let plan=read(&dir.join("native-runtime/runtime-plan.json"))?;
    if !sealed || proof["originalReceiptSha256"]!=crate::store::hash_file(&dir.join("schema-preparation-original.json"))?.0
        || proof["originalReceiptSeal"]!=seal || evidence["originalReceiptSha256"]!=proof["originalReceiptSha256"] || evidence["originalReceiptSeal"]!=seal
        || proof["evidenceSha256"]!=hash(&evidence) || evidence["sdkImplementationSha256"]!=SDK_IMPLEMENTATION_SHA || evidence["schemaPreparationBeforeExec"]!=true
        || proof["runtimeInventorySha256"]!=hash(&list) || evidence["runtimeInventorySha256"]!=hash(&list) || evidence["runtimeInventory"]!=list
        || evidence["eventsSha256"]!=hash(&evidence["events"]) || !schema_events_bound(&original,&evidence["events"])
        || original.as_object().is_none_or(|m|m.len()!=19) || plan.as_object().is_none_or(|m|m.len()!=12)
        || original.get("schemaPreparationRecovery").is_some() || original.get("preDispatchRecovery").is_some() || original.get("wrapperPreparationRepair").is_some()
        || original["state"]!="submission_unknown" || original["error"]["code"]!="ENOENT" || original["lastSequence"]!=0
        || !original["threadId"].is_null() || !original["turnId"].is_null() || !original["rawResult"].is_null() || !original["rawResultSha256"].is_null()
        || !original["usage"].is_null() || original["journalSha256"]!=digest("")
        || !["protocolVersion","attempt","frozenAttemptSha256","requestSnapshotSha256","compiledPrompt","compiledSchema","compiledPromptSha256","compiledSchemaSha256","domainAccepted"]
            .iter().all(|key|original[key]==record[key]) {return Ok(false);}
    let template=schema_template(&dir.join("native-runtime"))?;
    let sdk=Path::new(s(&record["attempt"]["runtimeIdentity"],"resourceDirectory")).join("node_modules/@openai/codex-sdk/dist/index.js");
    Ok(template.encode_utf16().count()>=260 && original["error"]["message"]==format!("ENOENT: no such file or directory, mkdtemp '{template}'")
        && crate::store::hash_file(&sdk)?.0==SDK_IMPLEMENTATION_SHA && plan["identity"]==original["attempt"]["runtimeIdentity"]
        && plan["attemptId"]==original["attempt"]["attemptId"] && plan["semanticRequestHash"]==original["attempt"]["semanticRequestHash"]
        && plan["executionFingerprint"]==original["attempt"]["executionFingerprint"] && plan["compiledPromptSha256"]==original["compiledPromptSha256"]
        && plan["compiledSchemaSha256"]==original["compiledSchemaSha256"])
}
pub(crate) fn schema_recovered_receipt_bound(root: &Path, attempt: &FrozenAttempt, saved: &Value) -> Result<()> {
    historical_receipt(root,attempt)?;let dir=root.join("sdk-worker").join(&attempt.attempt_id);let record=read(&dir.join("receipt.json"))?;
    if record["schemaPreparationRecovery"]!=json!({"proof":saved["proof"],"effectiveIdentity":saved["effectiveIdentity"]})
        || !schema_recovery_record_bound(&dir,&record,&saved["effectiveIdentity"])? {return Err(HELD.into());}Ok(())
}

fn schema_events_bound(record: &Value, events: &Value) -> bool {
    let Some(rows)=events.as_array() else{return false;};let mut ack=None;let mut observed=None;let mut stopped=None;let mut stderr=false;let mut last=0;
    for row in rows {
        let Some(id)=row["id"].as_i64().filter(|id|*id>last) else{return false;};last=id;
        match s(row,"type") {
            "sdk_pre_dispatch_queued" if ack.is_none() && row["data"]["modelCalls"]==0 && row["data"]["workflowId"]==record["attempt"]["workflowId"] && row["data"]["nodeKey"]==record["attempt"]["logicalNodeId"] => (),
            "sdk_dispatch_receipt" if ack.is_none() && observed.is_none()=>ack=Some(&row["data"]),
            "sdk_observation" if ack.is_some() && observed.is_none() && stopped.is_none()=>observed=Some(&row["data"]),
            "sdk_process_tree" if observed.is_some() && stopped.is_none()=>stopped=Some(&row["data"]),
            "sdk_stderr" if stopped.is_some() && !stderr && row["data"]["bytes"]==0 && row["data"]["retainedBytes"]==0 && row["data"]["truncated"]==false=>stderr=true,
            "state"=>(),_=>return false,
        }
    }
    let (Some(ack),Some(observed),Some(stopped))=(ack,observed,stopped) else{return false;};
    stderr && stopped["processTreeStopProven"]==true && stopped["activeProcesses"]==0 && stopped["totalProcesses"]==2
        && ack["ok"]==true && ack["result"]["state"]=="dispatching" && ack["requestId"]==observed["requestId"]
        && ack["attemptId"]==record["attempt"]["attemptId"] && observed["attemptId"]==record["attempt"]["attemptId"] && ack["protocolVersion"]==PROTOCOL && observed["protocolVersion"]==PROTOCOL
        && ["semanticRequestHash","executionFingerprint","attemptEnvelopeHash"].iter().all(|key|ack["result"][key]==record["attempt"][key]&&observed[key]==record["attempt"][key])
        && ["compiledPromptSha256","compiledSchemaSha256"].iter().all(|key|ack["result"][key]==record[key]&&observed[key]==record[key])
        && ["state","error","lastSequence","threadId","turnId","rawResult","usage"].iter().all(|key|observed[key]==record[key])
}

/// The production parent supplies its original persisted events. This function
/// is read-only and requires its genuine Windows Job shutdown event, in order.
pub(crate) fn schema_preparation_evidence(root: &Path, attempt: &FrozenAttempt, events: &Value) -> Result<Value> {
    let historical=historical_receipt(root,attempt)?;
    if attempt.state!=AttemptState::SubmissionUnknown || attempt.last_sequence!=1 || attempt.thread_id.is_some()
        || attempt.turn_id.is_some() || attempt.raw_result.is_some() || attempt.runtime_identity["bridgeSha256"]!=SCHEMA_FAILED_BRIDGE_SHA {return Err(format!("{HELD} [attempt-state]"));}
    let worker=root.join("sdk-worker");let dir=worker.join(&attempt.attempt_id);let runtime=dir.join("native-runtime");
    let meta=std::fs::symlink_metadata(&worker).map_err(|_|HELD)?;
    if !meta.is_dir() || meta.is_symlink() || !absent(&worker.join("worker.lock")) || !absent(&worker.join("worker.lock.recovery")) {return Err(format!("{HELD} [worker-owner]"));}
    directory(&dir,&["receipt.json","native-runtime"])?;
    let original=bytes(&dir.join("receipt.json"))?;let record:Value=serde_json::from_slice(&original).map_err(|_|HELD)?;
    let compiled=crate::text_executor::compile_sdk_attempt_input(attempt)?;
    if record.as_object().is_none_or(|m|m.len()!=19) || record["state"]!="submission_unknown" || record["error"]["code"]!="ENOENT" || record["lastSequence"]!=0
        || !record["threadId"].is_null() || !record["turnId"].is_null() || !record["rawResult"].is_null() || !record["rawResultSha256"].is_null()
        || !record["usage"].is_null() || record["journalSha256"]!=digest("") || record.get("preDispatchRecovery").is_some()
        || record.get("wrapperPreparationRepair").is_some() || record.get("schemaPreparationRecovery").is_some()
        || record["compiledPrompt"]!=compiled.prompt || record["compiledSchema"]!=compiled.schema
        || record["compiledPromptSha256"]!=digest(&compiled.prompt) || record["compiledSchemaSha256"]!=digest(&compiled.schema) {return Err(format!("{HELD} [original-input]"));}
    let list=schema_runtime_inventory(&runtime)?;let plan=read(&runtime.join("runtime-plan.json"))?;
    if plan.as_object().is_none_or(|m|m.len()!=12) || plan["version"]!="sdk-native-runtime/v1" || plan["attemptId"]!=attempt.attempt_id || plan["identity"]!=attempt.runtime_identity
        || plan["model"]!=attempt.request.model || plan["reasoning"]!=attempt.request.reasoning
        || plan["semanticRequestHash"]!=attempt.semantic_request_hash || plan["executionFingerprint"]!=attempt.execution_fingerprint
        || plan["compiledPromptSha256"]!=record["compiledPromptSha256"] || plan["compiledSchemaSha256"]!=record["compiledSchemaSha256"]
        || plan["runtimeSchemaSha256"]!=digest(&serde_json::from_str::<Value>(&compiled.schema).map_err(|_|HELD)?.to_string())
        || std::fs::canonicalize(Path::new(s(&plan,"workingDirectory"))).map_err(|_|HELD)?!=std::fs::canonicalize(runtime.join("work")).map_err(|_|HELD)?
        || std::fs::canonicalize(Path::new(s(&plan,"tempDirectory"))).map_err(|_|HELD)?!=std::fs::canonicalize(runtime.join("tmp")).map_err(|_|HELD)? {return Err(format!("{HELD} [runtime-plan]"));}
    let template=schema_template(&runtime)?;
    if template.encode_utf16().count()<260 || record["error"]["message"]!=format!("ENOENT: no such file or directory, mkdtemp '{template}'") {return Err(format!("{HELD} [schema-template]"));}
    let sdk=Path::new(s(&attempt.runtime_identity,"resourceDirectory")).join("node_modules/@openai/codex-sdk/dist/index.js");
    let sdk_body=bytes(&sdk)?;
    if crate::store::hash_file(&sdk)?.0!=SDK_IMPLEMENTATION_SHA {return Err(format!("{HELD} [sdk-source]"));}
    let sdk_source=std::str::from_utf8(&sdk_body).map_err(|_|HELD)?;
    let preparation=sdk_source.find("await createOutputSchemaFile(turnOptions.outputSchema)").ok_or(HELD)?;
    let execution=sdk_source[preparation..].find("this._exec.run(").ok_or(HELD)?+preparation;
    if preparation>=execution {return Err(HELD.into());}
    if !schema_events_bound(&record,events) {return Err(HELD.into());}
    if bytes(&dir.join("receipt.json"))?!=original || schema_runtime_inventory(&runtime)?!=list
        || !absent(&worker.join("worker.lock")) || !absent(&worker.join("worker.lock.recovery")) {return Err(HELD.into());}
    Ok(json!({"version":"sdk-schema-preparation-evidence/v1","attemptId":attempt.attempt_id,"attemptEnvelopeHash":attempt.attempt_envelope_hash,
        "originalReceiptSha256":crate::store::hash_file(&dir.join("receipt.json"))?.0,"originalReceiptSeal":historical["receiptEnvelopeSha256"],
        "runtimeInventory":list,"runtimeInventorySha256":hash(&list),"events":events,"eventsSha256":hash(events),
        "sdkImplementationSha256":SDK_IMPLEMENTATION_SHA,"schemaPreparationBeforeExec":true,"sdkEvents":0,"processTreeStopProven":true,"beforeOfficialRuntimeSpawn":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seal(path: &Path, record: &Value) {
        let text=record.to_string();let sealed=format!("{},\"receiptEnvelopeSha256\":\"{}\"}}",&text[..text.len()-1],hash(record));
        std::fs::write(path,sealed).unwrap();
    }
    fn fixture() -> (tempfile::TempDir,PathBuf,FrozenAttempt,Value,Value) {
        let temp=tempfile::tempdir().unwrap();let root=temp.path().join("x".repeat(190));
        let p=temp.path().join("synthetic-not-executed").to_string_lossy().into_owned();
        let identity=json!({"version":"official-sdk-identity/v2","provider":"codex_sdk","sdkPackage":"@openai/codex-sdk","sdkVersion":SDK_VERSION,"sdkImplementationSha256":SDK_IMPLEMENTATION_SHA,
            "bridgeVersion":"0.1.0","bridgeSha256":SCHEMA_FAILED_BRIDGE_SHA,"protocolVersion":PROTOCOL,"nodeExecutable":p,"nodeSha256":"a".repeat(64),"nodeVersion":"v24.19.0",
            "runtimePackageVersion":"codex-cli synthetic","runtimeIdentity":format!("official-codex-runtime/codex-cli synthetic/{}","b".repeat(64)),"permissionProfile":NATIVE_POLICY,
            "accountFingerprint":"c".repeat(64),"runtimeExecutable":p,"runtimeSha256":"b".repeat(64),"supervisorExecutable":p,"supervisorSha256":"d".repeat(64),
            "resourceDirectory":std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sdk-bridge")).unwrap(),"credentialHome":p,"codexHome":p});
        let request=crate::pipeline::TextRequest{stage:"fixture".into(),skill:"fixture".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input:json!({"synthetic":true}),schema:json!({"type":"object"}),rule_snapshot:json!({})};
        let prepared=FrozenAttempt::prepare_sdk_rules_first("schema-preparation","schema-workflow","schema-node",request,&identity).unwrap();
        let compiled=crate::text_executor::compile_sdk_attempt_input(&prepared).unwrap();
        let runtime=root.join("sdk-worker/schema-preparation/native-runtime");
        std::fs::create_dir_all(runtime.join("tmp")).unwrap();std::fs::create_dir(runtime.join("work")).unwrap();
        std::fs::write(runtime.join("work/.director-text-root"),"director SDK text execution\n").unwrap();
        let template=schema_template(&runtime).unwrap();
        let record=json!({"protocolVersion":PROTOCOL,"attempt":prepared,"frozenAttemptSha256":hash(&json!(prepared)),"requestSnapshotSha256":hash(&json!(prepared.request)),
            "compiledPrompt":compiled.prompt,"compiledSchema":compiled.schema,"compiledPromptSha256":digest(&compiled.prompt),"compiledSchemaSha256":digest(&compiled.schema),
            "state":"submission_unknown","error":{"code":"ENOENT","message":format!("ENOENT: no such file or directory, mkdtemp '{template}'")},"lastSequence":0,
            "threadId":null,"turnId":null,"rawResult":null,"rawResultSha256":null,"usage":null,"journalSha256":digest(""),"domainAccepted":false});
        let plan=json!({"version":"sdk-native-runtime/v1","attemptId":prepared.attempt_id,"identity":identity,"model":prepared.request.model,"reasoning":prepared.request.reasoning,
            "compiledPromptSha256":record["compiledPromptSha256"],"compiledSchemaSha256":record["compiledSchemaSha256"],
            "runtimeSchemaSha256":digest(&serde_json::from_str::<Value>(&compiled.schema).unwrap().to_string()),"semanticRequestHash":prepared.semantic_request_hash,
            "executionFingerprint":prepared.execution_fingerprint,"workingDirectory":node_path_argument(&runtime.join("work")).unwrap(),"tempDirectory":node_path_argument(&runtime.join("tmp")).unwrap()});
        std::fs::write(runtime.join("runtime-plan.json"),plan.to_string()).unwrap();seal(&runtime.parent().unwrap().join("receipt.json"),&record);
        let mut public=json!({"attemptId":prepared.attempt_id,"semanticRequestHash":prepared.semantic_request_hash,"executionFingerprint":prepared.execution_fingerprint,"attemptEnvelopeHash":prepared.attempt_envelope_hash,
            "compiledPromptSha256":record["compiledPromptSha256"],"compiledSchemaSha256":record["compiledSchemaSha256"],"state":"dispatching","error":null,"lastSequence":0,"threadId":null,"turnId":null,"rawResult":null,"usage":null});
        let ack=json!({"attemptId":prepared.attempt_id,"requestId":"rust_2","protocolVersion":PROTOCOL,"ok":true,"result":public});
        public["state"]=record["state"].clone();public["error"]=record["error"].clone();public["requestId"]=json!("rust_2");public["protocolVersion"]=json!(PROTOCOL);
        let events=json!([{"id":1,"type":"sdk_dispatch_receipt","data":ack},{"id":2,"type":"sdk_observation","data":public},
            {"id":3,"type":"sdk_process_tree","data":{"processTreeStopProven":true,"activeProcesses":0,"totalProcesses":2}},
            {"id":4,"type":"sdk_stderr","data":{"bytes":0,"retainedBytes":0,"truncated":false}},{"id":5,"type":"state","data":{"state":"submission_unknown"}}]);
        let mut attempt=prepared;attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.state=AttemptState::SubmissionUnknown;attempt.last_sequence=1;
        (temp,root,attempt,record,events)
    }
    #[cfg(windows)]
    #[test]
    fn sdk_schema_preparation_evidence_requires_original_pinned_preexec_failure_and_stopped_tree() {
        let (_temp,root,attempt,record,events)=fixture();let file=root.join("sdk-worker/schema-preparation/receipt.json");let before=std::fs::read(&file).unwrap();
        historical_receipt(&root,&attempt).unwrap();
        schema_runtime_inventory(&root.join("sdk-worker/schema-preparation/native-runtime")).unwrap();
        assert!(schema_events_bound(&record,&events),"synthetic parent chain");
        let evidence=schema_preparation_evidence(&root,&attempt,&events).unwrap();
        assert_eq!(evidence["sdkImplementationSha256"],SDK_IMPLEMENTATION_SHA);assert_eq!(evidence["schemaPreparationBeforeExec"],true);
        assert_eq!(evidence["runtimeInventory"].as_array().unwrap().len(),2);assert_eq!(std::fs::read(file).unwrap(),before);
        assert_eq!(attempt.state,AttemptState::SubmissionUnknown);
    }
    #[cfg(windows)]
    #[test]
    fn sdk_schema_preparation_evidence_rejects_error_only_or_any_dispatch_ambiguity() {
        for fault in ["tls","path","raw","journal","process","tmp-file","foreign-file","lock","stop","active","process-count","event","order","sdk-source","identity","plan"] {
            let (_temp,root,mut attempt,mut record,mut events)=fixture();let dir=root.join("sdk-worker/schema-preparation");
            match fault {
                "tls"=>record["error"]["code"]=json!("SDK_STREAM_ERROR"),"path"=>record["error"]["message"]=json!("ENOENT"),"raw"=>record["rawResult"]=json!({"text":"partial"}),
                "journal"=>std::fs::write(dir.join("events.ndjson"),"").unwrap(),"process"=>std::fs::write(dir.join("native-runtime/process-receipt.json"),"{}").unwrap(),
                "tmp-file"=>std::fs::write(dir.join("native-runtime/tmp/unknown"),"{}").unwrap(),"foreign-file"=>std::fs::write(dir.join("unknown"),"{}").unwrap(),
                "lock"=>std::fs::write(root.join("sdk-worker/worker.lock"),"{}").unwrap(),"stop"=>events[2]["data"]["processTreeStopProven"]=json!(false),
                "active"=>events[2]["data"]["activeProcesses"]=json!(1),"process-count"=>events[2]["data"]["totalProcesses"]=json!(3),
                "event"=>events.as_array_mut().unwrap().push(json!({"id":6,"type":"sdk_event","data":{}})),"order"=>events[2]["id"]=json!(1),
                "sdk-source"=>attempt.runtime_identity["resourceDirectory"]=json!(root.to_string_lossy()),"identity"=>attempt.runtime_identity["bridgeSha256"]=json!("f".repeat(64)),
                "plan"=>std::fs::write(dir.join("native-runtime/runtime-plan.json"),"{}").unwrap(),_=>unreachable!(),
            }
            seal(&dir.join("receipt.json"),&record);assert!(schema_preparation_evidence(&root,&attempt,&events).is_err(),"{fault}");
        }
    }
    #[test]
    fn sdk_schema_recovery_identity_cannot_change_bridge_account_or_runtime() {
        let (_temp,_root,attempt,_,_)=fixture();let old=attempt.runtime_identity;let mut current=old.clone();current["supervisorSha256"]=json!("e".repeat(64));
        assert!(schema_recovery_identity_bound(&old,&current));
        for field in ["bridgeSha256","accountFingerprint","runtimeSha256","nodeSha256","credentialHome","permissionProfile"] {
            let mut changed=current.clone();changed[field]=json!("f".repeat(64));assert!(!schema_recovery_identity_bound(&old,&changed),"{field}");
        }
    }
    #[test]
    #[ignore="Explicit private readonly original attempt/events fixture, never opens DB or executes SDK"]
    fn sdk_schema_preparation_actual_original_is_evidence_only() {
        let root=PathBuf::from(std::env::var_os("DIRECTOR_SCHEMA_PREPARATION_ROOT").expect("private root"));
        let id=std::env::var("DIRECTOR_SCHEMA_PREPARATION_ATTEMPT").expect("private attempt");
        let events=read(Path::new(&std::env::var("DIRECTOR_SCHEMA_PREPARATION_EVENTS").expect("private events"))).unwrap();
        let attempt=FrozenAttempt::load(&root.join("text-runs").join(&id).join("sdk-attempt.json")).unwrap();
        let file=root.join("sdk-worker").join(&id).join("receipt.json");let before=std::fs::read(&file).unwrap();
        let evidence=schema_preparation_evidence(&root,&attempt,&events).unwrap();assert_eq!(evidence["beforeOfficialRuntimeSpawn"],true);
        assert_eq!(std::fs::read(file).unwrap(),before);assert_eq!(attempt.state,AttemptState::SubmissionUnknown);
    }
}
