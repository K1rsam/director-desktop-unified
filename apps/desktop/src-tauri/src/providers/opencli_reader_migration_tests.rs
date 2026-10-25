//! Synthetic reader-only policy and component verification. No Node/browser calls.
use super::*;
use std::collections::BTreeMap;

struct Runtime { runner: PathBuf, pin: String }

fn runtime(root: &Path, marker: &str) -> Runtime {
    runtime_with_owned_adapter(root, marker, false)
}

fn runtime_with_owned_adapter(root: &Path, marker: &str, owned: bool) -> Runtime {
    fs::create_dir_all(root).unwrap();
    let vendor_bytes = format!("synthetic vendor {marker}").into_bytes();
    let vendor = json!({"commit":"synthetic-pinned-source","files":[{"path":"reader.js","sha256":hex_hash(&vendor_bytes)}]}).to_string();
    let mut files = BTreeMap::from([
        ("bridge.mjs".to_owned(), format!("// synthetic {marker}; never executed").into_bytes()),
        ("vendor-source.json".to_owned(), vendor.into_bytes()),
        ("package-lock.json".to_owned(), format!("{{\"synthetic\":\"{marker}\"}}").into_bytes()),
        ("package.json".to_owned(), b"{\"private\":true}".to_vec()),
        ("vendor/opencli/reader.js".to_owned(), vendor_bytes),
    ]);
    if owned { files.insert("composer-transport.mjs".to_owned(), format!("// synthetic owned adapter {marker}").into_bytes()); }
    let mut aggregate = Sha256::new();
    let names: Vec<&str> = if owned { vec!["bridge.mjs", "composer-transport.mjs", "vendor-source.json", "package-lock.json"] }
        else { vec!["bridge.mjs", "vendor-source.json", "package-lock.json"] };
    for name in names { aggregate.update(&files[name]); }
    let pin = format!("{:x}", aggregate.finalize());
    files.insert("runtime.sha256".to_owned(), format!("{pin}\n").into_bytes());
    for (name, bytes) in &files {
        let file = root.join(name); fs::create_dir_all(file.parent().unwrap()).unwrap(); fs::write(file, bytes).unwrap();
    }
    let manifest = json!({"version":"director-opencli-runtime/v1","sourceCommit":"synthetic-pinned-source",
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"sha256":hex_hash(bytes)})).collect::<Vec<_>>()});
    fs::write(root.join("runtime-manifest.json"), manifest.to_string()).unwrap();
    Runtime { runner: root.join("bridge.mjs"), pin }
}

fn request(root: &Path) -> GenerationRequest {
    serde_json::from_value(json!({"requestId":"original-request","kind":"image","model":"chatgpt-image",
        "prompt":"synthetic frozen prompt","ratio":"16:9","resolution":"native",
        "references":[{"mediaId":"original-parent","kind":"image","path":root.join("parent.png"),"sha256":"a".repeat(64),"order":0,"usage":"identity"}],
        "options":{"frozen":"kept"}})).unwrap()
}

fn config(root: &Path, old: &Runtime) -> ProviderConfig {
    serde_json::from_value(json!({"kind":"chrome","endpoint":"http://127.0.0.1:19825","cliPath":root.join("not-executed-node.exe"),
        "timeoutSeconds":37,"adaptedModels":["unchanged"],"browserTransport":{"transport":"opencli/v1","runner":old.runner,"runtimeSha256":old.pin,
            "stateRoot":root.join("original-state"),"profile":"original-profile","accountKey":"original-account-digest","opaque":{"frozen":true}}})).unwrap()
}

fn response(request: &GenerationRequest) -> Value {
    json!({"taskId":request.request_id,"status":"succeeded","resultUrls":["opencli-image:original-request:synthetic"],"error":null,
        "actualCost":null,"raw":{"transport":"opencli/v1","requestHash":"original-wire-hash","originalIdentityVerified":true,"proofField":"unchanged"}})
}

#[test]
fn reader_policy_allows_only_the_reviewed_old_and_current_pin_pairs() {
    assert_eq!(READ_ONLY_READER_MIGRATIONS, &[
        ("054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
        ("48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
        ("d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
        ("f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
        ("54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
        ("afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
        ("c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
        ("6b85411ff480f21dd3a36a0f26a33f2761a73d591cc810f58ff7beb0710d1aa2", "5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab"),
    ]);
    for (from,to) in READ_ONLY_READER_MIGRATIONS {assert!(read_only_reader_migration_allowed(from,to));}
    assert!(!read_only_reader_migration_allowed(&"f".repeat(64),COMPILED_RUNTIME_PIN.trim()));
    assert!(!read_only_reader_migration_allowed(READ_ONLY_READER_MIGRATIONS[0].0,&"f".repeat(64)));
    assert_eq!(reader_policy().compiled_pin, COMPILED_RUNTIME_PIN.trim());
}

#[test]
fn reader_saved_historical_audits_do_not_allow_executing_a_historical_reader() {
    let prior="f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70";
    for source in ["054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
        "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3",
        "d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8"] {
        assert!(read_only_reader_audit_allowed(source,prior));
        assert!(!read_only_reader_migration_allowed(source,prior));
        assert!(!reader_policy().migrations.iter().any(|(old,new)|*old==source&&*new==prior));
    }
    assert!(!read_only_reader_audit_allowed(&"f".repeat(64),prior));
    assert!(!read_only_reader_audit_allowed(prior,prior));
    let previous="54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0";
    for source in ["054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
        "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3",
        "d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8",prior] {
        assert!(read_only_reader_audit_allowed(source,previous));
        assert!(!read_only_reader_migration_allowed(source,previous));
        assert!(!reader_policy().migrations.iter().any(|(old,new)|*old==source&&*new==previous));
    }
    assert!(!read_only_reader_audit_allowed(&"f".repeat(64),previous));
    assert!(!read_only_reader_audit_allowed(previous,previous));
    let terminal_reader="afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6";
    for source in ["054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
        "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3",
        "d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8",prior,previous] {
        assert!(read_only_reader_audit_allowed(source,terminal_reader));
        assert!(!read_only_reader_migration_allowed(source,terminal_reader));
        assert!(!reader_policy().migrations.iter().any(|(old,new)|*old==source&&*new==terminal_reader));
    }
    assert!(!read_only_reader_audit_allowed(&"f".repeat(64),terminal_reader));
    assert!(!read_only_reader_audit_allowed(terminal_reader,terminal_reader));
}

#[test]
fn reader_owned_adapter_is_in_the_v1_manifest_and_aggregate_without_weakening_legacy_readers() {
    let temp = tempfile::tempdir().unwrap();
    let legacy = runtime(&temp.path().join("legacy"), "legacy");
    assert!(verify_reader(&legacy.runner, &legacy.pin).is_ok());
    let owned = runtime_with_owned_adapter(&temp.path().join("owned"), "owned", true);
    assert!(verify_reader(&owned.runner, &owned.pin).is_ok());
    for fault in ["missing-file", "missing-manifest-entry", "changed-adapter-with-updated-manifest"] {
        let reader = runtime_with_owned_adapter(&temp.path().join(fault), fault, true);
        let root = reader.runner.parent().unwrap();
        let manifest_path = root.join("runtime-manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        match fault {
            "missing-file" => fs::remove_file(root.join("composer-transport.mjs")).unwrap(),
            "missing-manifest-entry" => manifest["files"].as_array_mut().unwrap().retain(|f| f["path"] != "composer-transport.mjs"),
            "changed-adapter-with-updated-manifest" => {
                let changed = b"changed owned adapter";
                fs::write(root.join("composer-transport.mjs"), changed).unwrap();
                manifest["files"].as_array_mut().unwrap().iter_mut().find(|f| f["path"] == "composer-transport.mjs").unwrap()["sha256"] = json!(hex_hash(changed));
            },
            _ => unreachable!(),
        }
        fs::write(manifest_path, manifest.to_string()).unwrap();
        assert_eq!(verify_reader(&reader.runner, &reader.pin).err().unwrap().code, "OPENCLI_COMPONENT_MISMATCH", "{fault}");
    }
}

#[test]
fn reader_query_changes_only_temporary_reader_and_preserves_original_files_and_raw_proof() {
    let temp = tempfile::tempdir().unwrap(); let root = temp.path();
    let old = runtime(&root.join("old"), "old"); let new = runtime(&root.join("current"), "new");
    let c = config(root, &old); let r = request(root); let before = json!(c); let request_before = json!(r);
    let frozen = root.join("frozen-snapshot.json"); let canonical = root.join("canonical-request.json");
    fs::write(&frozen, before.to_string()).unwrap(); fs::write(&canonical, request_before.to_string()).unwrap();
    let frozen_bytes = fs::read(&frozen).unwrap(); let canonical_bytes = fs::read(&canonical).unwrap();
    let pairs = [(old.pin.as_str(), new.pin.as_str())];
    let mut calls = vec![];
    let result = query_with_reader(&c, &r, "query", ReaderPolicy { compiled_pin:&new.pin, migrations:&pairs }, ||vec![new.runner.clone()], |temporary, operation, original_request| {
        calls.push(operation.to_owned()); assert_eq!(operation, "query"); assert_eq!(json!(original_request), request_before);
        let mut actual = json!(temporary); actual["browserTransport"]["runner"] = before["browserTransport"]["runner"].clone();
        actual["browserTransport"]["runtimeSha256"] = before["browserTransport"]["runtimeSha256"].clone(); assert_eq!(actual, before);
        assert_eq!(temporary.browser_transport.as_ref().unwrap()["runtimeSha256"], new.pin);
        assert_eq!(fs::canonicalize(Path::new(s(temporary.browser_transport.as_ref().unwrap(), "runner"))).unwrap(), fs::canonicalize(&new.runner).unwrap());
        Ok(response(original_request))
    }).unwrap();
    assert_eq!(calls, vec!["query"]); assert_eq!(json!(c), before); assert_eq!(json!(r), request_before);
    assert_eq!(fs::read(frozen).unwrap(), frozen_bytes); assert_eq!(fs::read(canonical).unwrap(), canonical_bytes);
    for (key, value) in response(&r)["raw"].as_object().unwrap() { assert_eq!(&result.raw[key], value); }
    assert_eq!(result.raw["readerMigration"]["readOnly"], true); assert_eq!(result.raw["readerMigration"]["resubmitted"], false);
    assert_eq!(result.raw["readerMigration"]["fromRuntimeSha256"], old.pin); assert_eq!(result.raw["readerMigration"]["toRuntimeSha256"], new.pin);
    assert_eq!(result.raw["readerMigration"]["generationRequestSha256"], crate::pipeline::digest(&request_before.to_string()));
    assert!(!root.join("original-state").exists(), "reader selection never creates or rewrites original state");
}

#[test]
fn reader_reconcile_uses_query_and_keeps_the_same_original_identity() {
    let temp=tempfile::tempdir().unwrap(); let old=runtime(&temp.path().join("old"),"old");let new=runtime(&temp.path().join("current"),"new");
    let c=config(temp.path(),&old);let r=request(temp.path());let pairs=[(old.pin.as_str(),new.pin.as_str())];
    let result=query_with_reader(&c,&r,"reconcile",ReaderPolicy{compiled_pin:&new.pin,migrations:&pairs},||vec![new.runner.clone()],|temporary,operation,original|{
        assert_eq!(operation,"query");assert_eq!(json!(original),json!(r));
        for key in ["stateRoot","profile","accountKey","opaque"] {assert_eq!(temporary.browser_transport.as_ref().unwrap()[key],c.browser_transport.as_ref().unwrap()[key]);}
        Ok(response(original))
    }).unwrap();assert_eq!(result.raw["readerMigration"]["operation"],"reconcile");assert_eq!(result.raw["readerMigration"]["bridgeOperation"],"query");
}

#[test]
fn reader_prepare_submit_and_other_operations_never_locate_a_replacement() {
    let temp=tempfile::tempdir().unwrap();let old=runtime(&temp.path().join("old"),"old");let new=runtime(&temp.path().join("current"),"new");
    let c=config(temp.path(),&old);let r=request(temp.path());let pairs=[(old.pin.as_str(),new.pin.as_str())];
    for operation in ["prepare","submit","recover_prepare","status","connect","download","cancel","unknown"] {
        let (selected,audit)=read_only_reader_config(&c,&r,operation,ReaderPolicy{compiled_pin:&new.pin,migrations:&pairs},||panic!("{operation} must never inspect a replacement reader")).unwrap();
        assert_eq!(json!(selected),json!(c));assert!(audit.is_none());
    }
}

#[test]
fn reader_unknown_pins_and_future_compiled_readers_cannot_use_this_exception() {
    let temp=tempfile::tempdir().unwrap();let old=runtime(&temp.path().join("old"),"old");let new=runtime(&temp.path().join("current"),"new");
    let mut c=config(temp.path(),&old);let r=request(temp.path());let pairs=[(old.pin.as_str(),new.pin.as_str())];
    c.browser_transport.as_mut().unwrap()["runtimeSha256"]=json!("f".repeat(64));
    let (selected,audit)=read_only_reader_config(&c,&r,"query",ReaderPolicy{compiled_pin:&new.pin,migrations:&pairs},||panic!("unknown old pin cannot migrate")).unwrap();assert_eq!(json!(selected),json!(c));assert!(audit.is_none());
    let c=config(temp.path(),&old);
    let (selected,audit)=read_only_reader_config(&c,&r,"query",ReaderPolicy{compiled_pin:&"e".repeat(64),migrations:&pairs},||panic!("unreviewed current pin cannot migrate")).unwrap();assert_eq!(json!(selected),json!(c));assert!(audit.is_none());
}

#[test]
fn reader_non_opencli_and_non_image_requests_keep_their_frozen_configuration() {
    let temp=tempfile::tempdir().unwrap();let old=runtime(&temp.path().join("old"),"old");let new=runtime(&temp.path().join("current"),"new");
    let pairs=[(old.pin.as_str(),new.pin.as_str())];let original=config(temp.path(),&old);
    for fault in ["transport","kind","request-kind","model"] {
        let mut c=original.clone();let mut r=request(temp.path());
        match fault {"transport"=>c.browser_transport.as_mut().unwrap()["transport"]=json!("legacy/v1"),"kind"=>c.kind="other".into(),"request-kind"=>r.kind="video".into(),"model"=>r.model="other-model".into(),_=>unreachable!()}
        let (selected,audit)=read_only_reader_config(&c,&r,"query",ReaderPolicy{compiled_pin:&new.pin,migrations:&pairs},||panic!("ineligible request cannot migrate")).unwrap();assert_eq!(json!(selected),json!(c));assert!(audit.is_none());
    }
}

#[test]
fn reader_missing_or_tampered_current_component_keeps_original_query_available() {
    let temp=tempfile::tempdir().unwrap();let old=runtime(&temp.path().join("old"),"old");let new=runtime(&temp.path().join("current"),"new");
    let c=config(temp.path(),&old);let r=request(temp.path());let pairs=[(old.pin.as_str(),new.pin.as_str())];
    fs::write(&new.runner,"changed even though runtime.sha256 still matches").unwrap();
    for candidates in [vec![],vec![temp.path().join("absent/bridge.mjs")],vec![new.runner.clone()]] {
        let result=query_with_reader(&c,&r,"query",ReaderPolicy{compiled_pin:&new.pin,migrations:&pairs},||candidates,|selected,operation,original|{assert_eq!(json!(selected),json!(c));assert_eq!(operation,"query");Ok(response(original))}).unwrap();
        assert_eq!(result.raw,response(&r)["raw"]);assert!(result.raw.get("readerMigration").is_none());
    }
}

#[test]
fn reader_changed_old_component_is_held_before_any_read_or_send() {
    let temp=tempfile::tempdir().unwrap();let old=runtime(&temp.path().join("old"),"old");let new=runtime(&temp.path().join("current"),"new");
    let c=config(temp.path(),&old);let r=request(temp.path());let pairs=[(old.pin.as_str(),new.pin.as_str())];
    fs::write(&old.runner,"changed original component").unwrap();
    let error=query_with_reader(&c,&r,"query",ReaderPolicy{compiled_pin:&new.pin,migrations:&pairs},||vec![new.runner.clone()],|_,_,_|panic!("unverified old component must not execute")).unwrap_err();
    assert_eq!(error.code,"OPENCLI_COMPONENT_MISMATCH");assert!(!temp.path().join("original-state").exists());
}

#[test]
fn reader_verifies_actual_manifest_files_vendor_anchor_and_canonical_paths() {
    for fault in ["sidecar","file-hash","duplicate","path-escape","missing-bridge","vendor-anchor"] {
        let temp=tempfile::tempdir().unwrap();let reader=runtime(&temp.path().join("runtime"),fault);let parent=reader.runner.parent().unwrap();
        let file=parent.join("runtime-manifest.json");let mut manifest:Value=serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        match fault {
            "sidecar"=>fs::write(parent.join("runtime.sha256"),"f".repeat(64)).unwrap(),
            "file-hash"=>fs::write(parent.join("package.json"),"tampered dependency metadata").unwrap(),
            "duplicate"=>{let first=manifest["files"][0].clone();manifest["files"].as_array_mut().unwrap().push(first);},
            "path-escape"=>manifest["files"].as_array_mut().unwrap().push(json!({"path":"../outside","sha256":"f".repeat(64)})),
            "missing-bridge"=>manifest["files"].as_array_mut().unwrap().retain(|f|f["path"]!="bridge.mjs"),
            "vendor-anchor"=>{let changed=b"changed vendor with a matching untrusted manifest";fs::write(parent.join("vendor/opencli/reader.js"),changed).unwrap();let entry=manifest["files"].as_array_mut().unwrap().iter_mut().find(|f|f["path"]=="vendor/opencli/reader.js").unwrap();entry["sha256"]=json!(hex_hash(changed));},
            _=>unreachable!(),
        }
        fs::write(file,manifest.to_string()).unwrap();assert_eq!(verify_reader(&reader.runner,&reader.pin).err().unwrap().code,"OPENCLI_COMPONENT_MISMATCH","{fault}");
    }
}

#[test]
fn reader_locator_is_bound_to_the_current_executable_hash_and_companion_directory() {
    let temp=tempfile::tempdir().unwrap();let exe=temp.path().join("current.exe");fs::write(&exe,b"synthetic executable; never launched").unwrap();
    let candidates=executable_readers(&exe);let expected=temp.path().join(".client-resources").join(hex_hash(b"synthetic executable; never launched")).join("opencli/bridge.mjs");
    assert_eq!(candidates,vec![expected,temp.path().join("opencli/bridge.mjs")]);
    fs::write(&exe,b"different executable bytes").unwrap();assert_ne!(executable_readers(&exe)[0],candidates[0]);
    assert_eq!(executable_readers(&exe)[1],candidates[1]);
}

#[test]
fn reader_audit_never_overwrites_an_existing_provider_proof_or_migration_field() {
    let temp=tempfile::tempdir().unwrap();let old=runtime(&temp.path().join("old"),"old");let new=runtime(&temp.path().join("current"),"new");
    let c=config(temp.path(),&old);let r=request(temp.path());let before=json!(c);let pairs=[(old.pin.as_str(),new.pin.as_str())];
    for raw in [json!("original non-object raw"),json!({"readerMigration":"original provider value","proof":"keep"})] {
        let mut reply=response(&r);reply["raw"]=raw;
        let error=query_with_reader(&c,&r,"query",ReaderPolicy{compiled_pin:&new.pin,migrations:&pairs},||vec![new.runner.clone()],|_,operation,_|{assert_eq!(operation,"query");Ok(reply.clone())}).unwrap_err();
        assert_eq!(error.code,"OPENCLI_PACKET");assert_eq!(json!(c),before);
    }
}

#[test]
fn original_preflight_reader_audits_survive_account_failure_reader_upgrade() {
    let prior="c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057";
    for (source,_) in &READ_ONLY_READER_MIGRATIONS[..6] {
        assert!(read_only_reader_audit_allowed(source,prior));
        assert!(!read_only_reader_migration_allowed(source,prior));
    }
    assert!(read_only_reader_migration_allowed(prior,COMPILED_RUNTIME_PIN.trim()));
    assert!(!read_only_reader_audit_allowed(&"f".repeat(64),prior));
    assert!(!read_only_reader_audit_allowed(prior,prior));
}

#[test]
fn account_failure_reader_audits_survive_passive_dom_recovery_upgrade() {
    let prior="6b85411ff480f21dd3a36a0f26a33f2761a73d591cc810f58ff7beb0710d1aa2";
    for (source,_) in &READ_ONLY_READER_MIGRATIONS[..7] {
        assert!(read_only_reader_audit_allowed(source,prior));
        assert!(!read_only_reader_migration_allowed(source,prior));
    }
    assert!(read_only_reader_migration_allowed(prior,COMPILED_RUNTIME_PIN.trim()));
    assert!(!read_only_reader_audit_allowed(&"f".repeat(64),prior));
    assert!(!read_only_reader_audit_allowed(prior,prior));
}
