//! Synthetic reader-only policy and component verification. No Node/browser calls.
use super::*;
use std::collections::BTreeMap;

struct Runtime { runner: PathBuf, pin: String }

fn runtime(root: &Path, marker: &str) -> Runtime {
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
    let mut aggregate = Sha256::new();
    for name in ["bridge.mjs", "vendor-source.json", "package-lock.json"] { aggregate.update(&files[name]); }
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
fn reader_policy_allows_only_the_reviewed_old_and_current_pin_pair() {
    assert_eq!(READ_ONLY_READER_MIGRATIONS.len(), 1);
    assert_eq!(READ_ONLY_READER_MIGRATIONS[0], (
        "054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
        "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3"));
    assert_eq!(reader_policy().compiled_pin, COMPILED_RUNTIME_PIN.trim());
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
