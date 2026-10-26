//! Exercises the production wrapper validators through the final launch
//! boundary. Only that launch is replaced; no official runtime/model is used.
use super::*;

struct Fixture {
    _temp: tempfile::TempDir,
    attempt_directory: PathBuf,
    plan: RuntimePlan,
    record: Value,
    prompt: String,
}
impl Fixture {
    fn new(recovery: bool) -> Self {
        Self::with_padding(recovery, "")
    }
    fn with_padding(recovery: bool, padding: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let attempt_directory = temp.path().join(padding).join("sdk-worker/attempt-synthetic");
        let runtime = attempt_directory.join("native-runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let mut plan = super::tests::plan(&runtime);
        plan.identity.resource_directory = Some(node_path_argument(&std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sdk-bridge")).unwrap()).unwrap().to_string_lossy().into_owned());
        let actual_sha = crate::store::hash_file(&std::env::current_exe().unwrap()).unwrap().0;
        plan.identity.supervisor_sha256 = Some(actual_sha.clone());
        plan.identity.runtime_sha256 = Some(actual_sha.clone());
        plan.identity.runtime_identity = format!("official-codex-runtime/{}/{actual_sha}", plan.identity.runtime_package_version);
        let effective = json!(plan.identity);
        let mut frozen_identity = effective.clone();
        if recovery {
            frozen_identity["bridgeSha256"] = json!(LOCAL_PATH_BRIDGE_SHA);
            frozen_identity["supervisorSha256"] = json!("a".repeat(64));
        }
        let request = crate::pipeline::TextRequest {
            stage: "fixture".into(), skill: "fixture".into(), model: plan.model.clone(), reasoning: plan.reasoning.clone(),
            input: json!({"synthetic":true}), schema: json!({"type":"object"}), rule_snapshot: json!({}),
        };
        let attempt = FrozenAttempt::prepare_sdk_rules_first("attempt-synthetic", "synthetic-workflow", "node", request, &frozen_identity).unwrap();
        let compiled = crate::text_executor::compile_sdk_attempt_input(&attempt).unwrap();
        plan.semantic_request_hash = attempt.semantic_request_hash.clone();
        plan.execution_fingerprint = attempt.execution_fingerprint.clone();
        plan.compiled_prompt_sha256 = digest(&compiled.prompt);
        plan.compiled_schema_sha256 = digest(&compiled.schema);
        let schema: Value = serde_json::from_str(&compiled.schema).unwrap();
        plan.runtime_schema_sha256 = digest(&schema.to_string());
        std::fs::write(plan.temp_directory.join("schema.json"), schema.to_string()).unwrap();
        let mut record = json!({"protocolVersion":PROTOCOL,"attempt":attempt,"frozenAttemptSha256":digest(&json!(attempt).to_string()),
            "requestSnapshotSha256":digest(&json!(attempt.request).to_string()),"compiledPrompt":compiled.prompt,"compiledSchema":compiled.schema,
            "compiledPromptSha256":plan.compiled_prompt_sha256,"compiledSchemaSha256":plan.compiled_schema_sha256,
            "state":"dispatching","error":null,"lastSequence":0,"threadId":null,"turnId":null,"rawResult":null,
            "rawResultSha256":null,"usage":null,"journalSha256":digest(""),"domainAccepted":false});
        if recovery {
            record["state"] = json!("submission_unknown");
            record["error"] = json!({"code":"SDK_STREAM_ERROR","message":"SDK_NATIVE_IDENTITY: attempt directory"});
            let original = seal(&record);
            let original_seal = serde_json::from_str::<Value>(&original).unwrap()["receiptEnvelopeSha256"].clone();
            let proof = json!({"version":"sdk-local-preparation-recovery/v1","attemptId":attempt.attempt_id,
                "attemptEnvelopeHash":attempt.attempt_envelope_hash,"originalReceiptSeal":original_seal,
                "evidenceSha256":digest("synthetic-evidence"),"effectiveIdentitySha256":digest(&effective.to_string()),
                "sdkEvents":0,"processTreeStopProven":true});
            let dir = runtime.parent().unwrap();
            std::fs::write(dir.join("local-preparation-original.json"), original).unwrap();
            std::fs::write(dir.join("local-preparation-recovery.json"), proof.to_string()).unwrap();
            record["state"] = json!("dispatching"); record["error"] = Value::Null;
            record["preDispatchRecovery"] = json!({"proof":proof,"effectiveIdentity":effective});
        }
        Self { _temp: temp, attempt_directory, plan, record, prompt: compiled.prompt }
    }
    fn attempt_dir(&self) -> PathBuf { self.attempt_directory.clone() }
    fn schema_recovery() -> Self {
        let mut f=Self::with_padding(false,&"x".repeat(190));let dir=f.attempt_dir();let runtime=dir.join("native-runtime");
        // This is the historical ENOENT package, not today's schema compiler.
        // Its receipts stay valid, but a newer wrapper does not gain permission
        // to run that old bridge merely because this fixture is synthetic.
        f.plan.identity.bridge_sha256=schema_recovery::SCHEMA_FAILED_BRIDGE_SHA.into();
        let previous:FrozenAttempt=serde_json::from_value(f.record["attempt"].clone()).unwrap();
        let frozen=FrozenAttempt::prepare_sdk_rules_first(&previous.attempt_id,&previous.workflow_id,&previous.logical_node_id,
            previous.request.clone(),&json!(f.plan.identity)).unwrap();
        f.plan.semantic_request_hash=frozen.semantic_request_hash.clone();f.plan.execution_fingerprint=frozen.execution_fingerprint.clone();
        f.record["attempt"]=json!(frozen);f.record["frozenAttemptSha256"]=json!(digest(&json!(frozen).to_string()));
        let schema=std::fs::read(runtime.join("tmp/schema.json")).unwrap();std::fs::remove_file(runtime.join("tmp/schema.json")).unwrap();
        std::fs::write(runtime.join("work/.director-text-root"),"director SDK text execution\n").unwrap();
        std::fs::write(runtime.join("runtime-plan.json"),serde_json::to_string(&f.plan).unwrap()).unwrap();
        let template=node_path_argument(&node_state_path_argument(&runtime.join("tmp").join("codex-output-schema-XXXXXX")).unwrap()).unwrap();
        f.record["state"]=json!("submission_unknown");f.record["error"]=json!({"code":"ENOENT","message":format!("ENOENT: no such file or directory, mkdtemp '{}'",template.to_string_lossy())});
        let original=seal(&f.record);std::fs::write(dir.join("receipt.json"),&original).unwrap();
        let mut public=json!({"attemptId":f.plan.attempt_id,"semanticRequestHash":f.plan.semantic_request_hash,"executionFingerprint":f.plan.execution_fingerprint,
            "attemptEnvelopeHash":f.record["attempt"]["attemptEnvelopeHash"],"compiledPromptSha256":f.plan.compiled_prompt_sha256,"compiledSchemaSha256":f.plan.compiled_schema_sha256,
            "state":"dispatching","error":null,"threadId":null,"turnId":null,"lastSequence":0,"rawResult":null,"usage":null});
        let ack=json!({"attemptId":f.plan.attempt_id,"requestId":"schema_1","protocolVersion":PROTOCOL,"ok":true,"result":public});
        public["state"]=f.record["state"].clone();public["error"]=f.record["error"].clone();public["requestId"]=json!("schema_1");public["protocolVersion"]=json!(PROTOCOL);
        let events=json!([{"id":1,"type":"sdk_dispatch_receipt","data":ack},{"id":2,"type":"sdk_observation","data":public},
            {"id":3,"type":"sdk_process_tree","data":{"processTreeStopProven":true,"activeProcesses":0,"totalProcesses":2}},
            {"id":4,"type":"sdk_stderr","data":{"bytes":0,"retainedBytes":0,"truncated":false}}]);
        let mut attempt:FrozenAttempt=serde_json::from_value(f.record["attempt"].clone()).unwrap();attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        attempt.state=crate::text_transport::AttemptState::SubmissionUnknown;attempt.last_sequence=1;
        let evidence=schema_preparation_evidence(dir.parent().unwrap().parent().unwrap(),&attempt,&events).unwrap();let effective=json!(f.plan.identity);
        let proof=json!({"version":"sdk-schema-preparation-recovery/v1","attemptId":attempt.attempt_id,"attemptEnvelopeHash":attempt.attempt_envelope_hash,
            "controllerSha256":schema_controller_sha256(),"originalReceiptSha256":evidence["originalReceiptSha256"],"originalReceiptSeal":evidence["originalReceiptSeal"],
            "runtimeInventorySha256":evidence["runtimeInventorySha256"],"evidenceSha256":digest(&evidence.to_string()),"effectiveIdentitySha256":digest(&effective.to_string()),
            "sdkEvents":0,"processTreeStopProven":true,"beforeOfficialRuntimeSpawn":true});
        std::fs::write(dir.join("schema-preparation-original.json"),original).unwrap();std::fs::write(dir.join("schema-preparation-evidence.json"),evidence.to_string()).unwrap();
        std::fs::write(dir.join("schema-preparation-recovery.json"),proof.to_string()).unwrap();
        f.record["state"]=json!("dispatching");f.record["error"]=Value::Null;f.record["schemaPreparationRecovery"]=json!({"proof":proof,"effectiveIdentity":effective});
        let new=dir.join("native-runtime-schema-recovery");std::fs::create_dir_all(new.join("tmp")).unwrap();std::fs::create_dir(new.join("work")).unwrap();
        std::fs::write(new.join("tmp/schema.json"),schema).unwrap();std::fs::write(new.join("work/.director-text-root"),"director SDK text execution\n").unwrap();
        f.plan.working_directory=canonical(&new.join("work")).unwrap();f.plan.temp_directory=canonical(&new.join("tmp")).unwrap();f
    }
    fn repair() -> Self {
        let mut f=Self::new(true);let effective=json!(f.plan.identity);
        let mut prior=effective.clone();prior["bridgeSha256"]=json!(WRAPPER_FAILED_BRIDGE_SHA);prior["supervisorSha256"]=json!(WRAPPER_FAILED_SUPERVISOR_SHA);
        f.record["preDispatchRecovery"]["effectiveIdentity"]=prior.clone();
        f.record["preDispatchRecovery"]["proof"]["effectiveIdentitySha256"]=json!(digest(&prior.to_string()));
        std::fs::write(f.attempt_dir().join("local-preparation-recovery.json"),f.record["preDispatchRecovery"]["proof"].to_string()).unwrap();
        let mut old_plan=json!(f.plan);old_plan["identity"]=prior;
        std::fs::write(f.attempt_dir().join("native-runtime/runtime-plan.json"),old_plan.to_string()).unwrap();
        let saved=test_append_wrapper_repair(&f.attempt_dir(),&f.record,&effective);
        f.record["wrapperPreparationRepair"]=json!({"proof":saved["proof"],"effectiveIdentity":effective});
        let root=f.attempt_dir().join("native-runtime-wrapper-repair");std::fs::create_dir_all(root.join("work")).unwrap();std::fs::create_dir(root.join("tmp")).unwrap();
        std::fs::copy(f.plan.temp_directory.join("schema.json"),root.join("tmp/schema.json")).unwrap();
        f.plan.working_directory=root.join("work");f.plan.temp_directory=root.join("tmp");f
    }
    fn run(&self, args: &[String], prompt: &[u8], launches: &mut usize) -> Result<i32> {
        let file = self.plan.working_directory.parent().unwrap().join("runtime-plan.json");
        let plan = serde_json::to_string(&self.plan).unwrap();
        std::fs::write(&file, &plan).unwrap();
        std::fs::write(self.attempt_dir().join("receipt.json"), seal(&self.record)).unwrap();
        run_runtime_wrapper_plan(&file, &digest(&plan), args, prompt, |plan, schema, input, root| {
            *launches += 1;
            assert_eq!(plan.attempt_id, "attempt-synthetic");
            assert_eq!(digest(&String::from_utf8(input).unwrap()), plan.compiled_prompt_sha256);
            assert_eq!(crate::store::hash_file(schema).unwrap().0, plan.runtime_schema_sha256);
            assert_eq!(root, canonical(&file.parent().unwrap()).unwrap());
            Ok(0)
        })
    }
}

#[cfg(windows)]
#[test]
fn sdk_schema_wrapper_preserves_historical_recovery_proof_without_cross_bridge_execution() {
    let mut f=Fixture::schema_recovery();let dir=f.attempt_dir();let mut launches=0;
    let paths=["schema-preparation-original.json","schema-preparation-evidence.json","schema-preparation-recovery.json","native-runtime/runtime-plan.json"];
    let original:Vec<_>=paths.iter().map(|p|std::fs::read(dir.join(p)).unwrap()).collect();let saved=f.record.clone();
    std::fs::write(dir.join("receipt.json"),seal(&f.record)).unwrap();
    assert!(schema_recovery_record_bound(&dir,&f.record,&json!(f.plan.identity)).unwrap());
    let native_legacy=compiled_native_bridge_sha()==schema_recovery::SCHEMA_FAILED_BRIDGE_SHA;
    let result=f.run(&super::tests::args(&f.plan),f.prompt.as_bytes(),&mut launches);
    if native_legacy {assert_eq!(result.unwrap(),0);}else{assert!(result.unwrap_err().contains("SDK_IDENTITY_MISMATCH"));}
    assert_eq!(launches,usize::from(native_legacy));
    for fault in ["archive","evidence","intent","runtime","controller","identity","request"] {
        match fault {
            "archive"=>std::fs::write(dir.join(paths[0]),"{}").unwrap(),"evidence"=>std::fs::write(dir.join(paths[1]),"{}").unwrap(),
            "intent"=>std::fs::write(dir.join(paths[2]),"{}").unwrap(),"runtime"=>std::fs::write(dir.join(paths[3]),"{}").unwrap(),
            "controller"=>f.record["schemaPreparationRecovery"]["proof"]["controllerSha256"]=json!("f".repeat(64)),
            "identity"=>f.record["schemaPreparationRecovery"]["effectiveIdentity"]["accountFingerprint"]=json!("f".repeat(64)),
            "request"=>f.record["attempt"]["request"]["model"]=json!("other"),_=>unreachable!(),
        }
        std::fs::write(dir.join("receipt.json"),seal(&f.record)).unwrap();
        assert!(!schema_recovery_record_bound(&dir,&f.record,&json!(f.plan.identity)).unwrap_or(false),"{fault}");
        assert!(f.run(&super::tests::args(&f.plan),f.prompt.as_bytes(),&mut launches).is_err(),"{fault}");assert_eq!(launches,usize::from(native_legacy),"{fault}");
        f.record=saved.clone();for (p,b) in paths.iter().zip(&original){std::fs::write(dir.join(p),b).unwrap();}
    }
    for (p,b) in paths.iter().zip(original){assert_eq!(std::fs::read(dir.join(p)).unwrap(),b);}
}

#[cfg(windows)]
#[test]
fn sdk_native_wrapper_full_validation_accepts_owned_long_namespaced_schema_and_rejects_escape() {
    let mut f = Fixture::with_padding(false, &"x".repeat(190));
    f.plan.working_directory = canonical(&f.plan.working_directory).unwrap();
    f.plan.temp_directory = canonical(&f.plan.temp_directory).unwrap();
    let schema = canonical(&f.plan.temp_directory.join("schema.json")).unwrap();
    assert!(schema.to_string_lossy().starts_with(r"\\?\"));
    assert!(f.plan.temp_directory.join("schema.json").to_string_lossy().len() > 260);
    let mut args = super::tests::args(&f.plan);
    args[10] = schema.to_string_lossy().into_owned();
    let original_attempt = f.record["attempt"].clone();
    let mut launches = 0;
    assert_eq!(f.run(&args, f.prompt.as_bytes(), &mut launches).unwrap(), 0);
    assert_eq!(launches, 1);
    assert_eq!(f.record["attempt"], original_attempt);

    let outside = f.attempt_dir().join("foreign-schema.json");
    std::fs::copy(&schema, &outside).unwrap();
    args[10] = canonical(&outside).unwrap().to_string_lossy().into_owned();
    assert!(f.run(&args, f.prompt.as_bytes(), &mut launches).is_err());
    assert_eq!(launches, 1, "a namespaced spelling must not bypass containment");
}

#[cfg(windows)]
#[test]
fn sdk_native_wrapper_full_validation_repair_preserves_v1_and_rejects_broken_lineage() {
    let f=Fixture::repair();let dir=f.attempt_dir();
    let names=["local-preparation-original.json","local-preparation-recovery.json","wrapper-preparation-original.json","native-runtime/runtime-plan.json"];
    let before:Vec<_>=names.iter().map(|name|std::fs::read(dir.join(name)).unwrap()).collect();
    let mut launches=0;f.run(&super::tests::args(&f.plan),f.prompt.as_bytes(),&mut launches).unwrap();assert_eq!(launches,1);
    for (name,bytes) in names.iter().zip(before){assert_eq!(std::fs::read(dir.join(name)).unwrap(),bytes);}
    for fault in ["repair-intent","repair-archive","prior-runtime","prior-identity","repair-proof","new-identity","namespace"] {
        let mut f=Fixture::repair();
        match fault {
            "repair-intent"=>std::fs::write(f.attempt_dir().join("wrapper-preparation-repair.json"),"{}").unwrap(),
            "repair-archive"=>std::fs::write(f.attempt_dir().join("wrapper-preparation-original.json"),"{}").unwrap(),
            "prior-runtime"=>std::fs::write(f.attempt_dir().join("native-runtime/runtime-plan.json"),"{}").unwrap(),
            "prior-identity"=>f.record["preDispatchRecovery"]["effectiveIdentity"]["supervisorSha256"]=json!("f".repeat(64)),
            "repair-proof"=>f.record["wrapperPreparationRepair"]["proof"]["beforeOfficialRuntimeSpawn"]=json!(false),
            "new-identity"=>f.record["wrapperPreparationRepair"]["effectiveIdentity"]["accountFingerprint"]=json!("f".repeat(64)),
            "namespace"=>{f.plan.working_directory=f.attempt_dir().join("native-runtime/work");f.plan.temp_directory=f.attempt_dir().join("native-runtime/tmp");},
            _=>unreachable!(),
        }
        let mut launches=0;assert!(f.run(&super::tests::args(&f.plan),f.prompt.as_bytes(),&mut launches).is_err(),"{fault}");assert_eq!(launches,0,"{fault}");
    }
}
fn seal(value: &Value) -> String {
    let text = value.to_string();
    format!("{},\"receiptEnvelopeSha256\":\"{}\"}}", &text[..text.len()-1], digest(&text))
}

#[cfg(windows)]
#[test]
fn sdk_native_wrapper_full_validation_accepts_frozen_normal_and_proven_recovery_identity() {
    for recovery in [false, true] {
        let f = Fixture::new(recovery);
        let frozen = f.record["attempt"].clone();
        let archive = recovery.then(|| std::fs::read(f.attempt_dir().join("local-preparation-original.json")).unwrap());
        let mut launches = 0;
        assert_eq!(f.run(&super::tests::args(&f.plan), f.prompt.as_bytes(), &mut launches).unwrap(), 0);
        assert_eq!(launches, 1); assert_eq!(f.record["attempt"], frozen);
        if let Some(archive) = archive { assert_eq!(std::fs::read(f.attempt_dir().join("local-preparation-original.json")).unwrap(), archive); }
        assert!(!f.attempt_dir().join("native-runtime/process-receipt.json").exists());
    }
}

#[cfg(windows)]
#[test]
fn sdk_native_wrapper_full_validation_rejects_unproven_identity_and_retains_runtime_gate() {
    for fault in ["missing-recovery", "effective-identity", "missing-intent", "archive-seal", "proof-hash", "account", "runtime", "unknown-bridge", "frozen-model", "plan-model", "plan-fingerprint", "compiled-prompt", "sdk-events", "raw", "schema", "stdin", "argv"] {
        let mut f = Fixture::new(true); let mut args = super::tests::args(&f.plan); let mut prompt = f.prompt.clone();
        match fault {
            "missing-recovery" => { f.record.as_object_mut().unwrap().remove("preDispatchRecovery"); },
            "effective-identity" => f.record["preDispatchRecovery"]["effectiveIdentity"]["supervisorSha256"] = json!("f".repeat(64)),
            "missing-intent" => { std::fs::remove_file(f.attempt_dir().join("local-preparation-recovery.json")).unwrap(); },
            "archive-seal" => { std::fs::write(f.attempt_dir().join("local-preparation-original.json"), "{}").unwrap(); },
            "proof-hash" => f.record["preDispatchRecovery"]["proof"]["effectiveIdentitySha256"] = json!("f".repeat(64)),
            "account" => { f.plan.identity.account_fingerprint = "f".repeat(64); f.record["preDispatchRecovery"]["effectiveIdentity"] = json!(f.plan.identity); },
            "runtime" => { f.plan.identity.runtime_sha256 = Some("f".repeat(64)); f.record["preDispatchRecovery"]["effectiveIdentity"] = json!(f.plan.identity); },
            "unknown-bridge" => f.record["attempt"]["runtimeIdentity"]["bridgeSha256"] = json!("f".repeat(64)),
            "frozen-model" => f.record["attempt"]["request"]["model"] = json!("other-model"),
            "plan-model" => f.plan.model = "other-model".into(),
            "plan-fingerprint" => f.plan.execution_fingerprint = "f".repeat(64),
            "compiled-prompt" => f.record["compiledPromptSha256"] = json!("f".repeat(64)),
            "sdk-events" => f.record["lastSequence"] = json!(1),
            "raw" => f.record["rawResult"] = json!({"text":"must not reuse"}),
            "schema" => { std::fs::write(f.plan.temp_directory.join("schema.json"), "{}").unwrap(); },
            "stdin" => prompt.push('x'),
            "argv" => args.extend(["--image".into(), "not-allowed.png".into()]),
            _ => unreachable!(),
        }
        let mut launches = 0;
        assert!(f.run(&args, prompt.as_bytes(), &mut launches).is_err(), "{fault}");
        assert_eq!(launches, 0, "{fault}");
    }
}
