use super::*;

fn fixture() -> (tempfile::TempDir, Store, Value, Value, Value) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("p")).unwrap();
    store.create("restart synthetic", "p").unwrap();
    let identity = json!({"accountFingerprint":"a","sdkImplementationSha256":"sdk","runtimeSha256":"runtime","nodeSha256":"node","supervisorExecutable":"old","supervisorSha256":"old-sha","resourceDirectory":"old-res","bridgeSha256":"old-bridge"});
    let path = allocate(&store, "probe-synthetic-original").unwrap();
    let original = path.join("probe.png");
    image::RgbImage::new(8, 8).save(&original).unwrap();
    let images = vec![
        freeze_image(&path, 1, "probe", &json!({}), &original).unwrap(),
        freeze_image(&path, 2, "probe", &json!({}), &original).unwrap(),
    ];
    manifest(
        &path,
        "capability",
        identity.clone(),
        images,
        "frozen synthetic prompt".into(),
        probe_schema(),
        json!({}),
    )
    .unwrap();
    atomic_json(&path.join("expected.private.json"), &json!({"images":[]})).unwrap();
    atomic_json(
        &path.join("sdk-receipt.json"),
        &json!({"state":"submission_unknown"}),
    )
    .unwrap();
    let pointer = recovery::bind_pointer(&store, &path, false).unwrap();
    let evidence = json!({"version":"synthetic-bound-evidence","originalStatus":"submission_unknown","eventsSha256":"frozen-events"});
    (temp, store, pointer, identity, evidence)
}

#[test]
fn explicit_restart_requires_ack_and_exact_original_identity() {
    let (_temp, _store, pointer, _, _) = fixture();
    let request = json!({"acknowledgePossiblePriorUsage":true,"expectedAttemptId":pointer["attemptId"],"expectedManifestSha256":pointer["manifestSha256"]});
    request_binding(&request, &pointer).unwrap();
    for key in [
        "acknowledgePossiblePriorUsage",
        "expectedAttemptId",
        "expectedManifestSha256",
    ] {
        let mut changed = request.clone();
        changed[key] = Value::Null;
        assert!(request_binding(&changed, &pointer).is_err());
    }
    let mut candidate = pointer.clone();
    candidate["purpose"] = json!("candidate");
    assert!(request_binding(&request, &candidate).is_err());
}

#[test]
fn explicit_restart_reservation_is_idempotent_preserves_unknown_and_one_chain_budget() {
    let (_temp, store, pointer, identity, evidence) = fixture();
    let original = recovery::pointer_path(&store, &pointer).unwrap();
    let bytes = fs::read(original.join("sdk-receipt.json")).unwrap();
    let first = reserve(&store, &pointer, &identity, &evidence).unwrap();
    assert_eq!(
        reserve(&store, &pointer, &identity, &evidence).unwrap(),
        first
    );
    assert_eq!(first["maximumNewCalls"], 1);
    assert_eq!(first["authority"], AUTHORITY);
    let next = materialize(&store, &first).unwrap();
    assert_eq!(materialize(&store, &first).unwrap(), next);
    assert_eq!(
        read(&next.join("manifest.json")).unwrap()["basis"]["supersedesUnknown"]["originalStatus"],
        "submission_unknown"
    );
    assert_eq!(fs::read(original.join("sdk-receipt.json")).unwrap(), bytes);
    assert!(!next.join("dispatch-intent.json").exists());
    let later = first["successor"].clone();
    assert!(
        reserve(&store, &later, &identity, &evidence).is_err(),
        "a descendant cannot acquire a second explicit allowance"
    );
    let mut changed = evidence.clone();
    changed["eventsSha256"] = json!("changed");
    assert!(reserve(&store, &pointer, &identity, &changed).is_err());
}

#[test]
fn explicit_restart_does_not_change_account_sdk_node_or_upstream_runtime() {
    let (_temp, store, pointer, identity, evidence) = fixture();
    for key in [
        "accountFingerprint",
        "sdkImplementationSha256",
        "runtimeSha256",
        "nodeSha256",
    ] {
        let mut changed = identity.clone();
        changed[key] = json!("different");
        assert!(reserve(&store, &pointer, &changed, &evidence).is_err());
    }
    let mut fixed = identity.clone();
    for key in [
        "supervisorExecutable",
        "supervisorSha256",
        "resourceDirectory",
        "bridgeSha256",
    ] {
        fixed[key] = json!(format!("fixed-{key}"));
    }
    let claim = reserve(&store, &pointer, &fixed, &evidence).unwrap();
    assert_eq!(claim["plan"]["baseIdentity"], fixed);
    assert!(
        reserve(&store, &pointer, &identity, &evidence).is_err(),
        "reserved components cannot drift between clicks"
    );
}

#[test]
fn explicit_restart_resumes_reserved_preparation_without_rewriting_frozen_inputs() {
    let (_temp, store, pointer, identity, evidence) = fixture();
    let claim = reserve(&store, &pointer, &identity, &evidence).unwrap();
    let next = materialize(&store, &claim).unwrap();
    let old = read(
        &recovery::pointer_path(&store, &pointer)
            .unwrap()
            .join("manifest.json"),
    )
    .unwrap();
    let new = read(&next.join("manifest.json")).unwrap();
    assert_eq!(new["prompt"], old["prompt"]);
    assert_eq!(new["schema"], old["schema"]);
    for n in 0..2 {
        assert_eq!(new["images"][n]["sha256"], old["images"][n]["sha256"]);
    }
    fs::write(next.join("runner.mjs"), "changed").unwrap();
    assert!(materialize(&store, &claim).is_err());
    assert!(!next.join("dispatch-intent.json").exists());
}

#[test]
fn explicit_restart_requires_exact_stopped_startup_defect_not_generic_unknown() {
    let plan = json!({"purpose":"capability","runnerSha256":LEGACY_DIAGNOSTIC_RUNNER_SHA,"basis":{},"attemptId":"a","model":"gpt-6.1-sol","reasoning":"medium"});
    let sdk = json!({"state":"submission_unknown","error":"VISUAL_TOOL_BLOCKED","finalText":null,"usage":null,"model":"gpt-6.1-sol","reasoning":"medium","visualInputAttached":true});
    let process = json!({"version":"visual-process/v1","attemptId":"a","manifestSha256":"m","maximumModelCalls":1,"tree":{"processTreeStopProven":true,"activeProcesses":0,"totalProcesses":5}});
    let dispatch = json!({"version":"visual-dispatch/v1","attemptId":"a","manifestSha256":"m","maximumModelCalls":1});
    let runtime = json!({"attemptId":"a","manifestSha256":"m","maximumModelCalls":1});
    assert!(bound_stopped(
        &plan, &sdk, &process, &dispatch, &runtime, "m"
    ));
    for field in ["processTreeStopProven", "activeProcesses", "totalProcesses"] {
        let mut bad = process.clone();
        bad["tree"][field] = Value::Null;
        assert!(!bound_stopped(&plan, &sdk, &bad, &dispatch, &runtime, "m"));
    }
    let mut network = sdk.clone();
    network["error"] = json!("tls handshake eof");
    assert!(!bound_stopped(
        &plan, &network, &process, &dispatch, &runtime, "m"
    ));
    let mut completed = sdk.clone();
    completed["state"] = json!("transport_completed");
    assert!(!bound_stopped(
        &plan, &completed, &process, &dispatch, &runtime, "m"
    ));
    let mut second = plan.clone();
    second["basis"]["supersedesUnknown"] = json!({"attemptId":"older"});
    assert!(!bound_stopped(
        &second, &sdk, &process, &dispatch, &runtime, "m"
    ));
}

#[test]
fn explicit_restart_authorization_cannot_mutate_after_reservation() {
    let (_temp, store, pointer, identity, evidence) = fixture();
    let claim = reserve(&store, &pointer, &identity, &evidence).unwrap();
    for key in [
        "authority",
        "maximumNewCalls",
        "acknowledgePossiblePriorUsage",
        "evidence",
        "plan",
        "successor",
    ] {
        let mut changed = claim.clone();
        changed[key] = Value::Null;
        assert!(
            validate_claim(&store, &pointer, &changed, &evidence).is_err(),
            "{key}"
        );
    }
    assert!(create_new_json(&chain_root(&store, &pointer).unwrap().join(CLAIM), &claim).is_err());
}

#[test]
fn explicit_restart_never_reconstructs_missing_frozen_files_after_dispatch() {
    let (_temp, store, pointer, identity, evidence) = fixture();
    let claim = reserve(&store, &pointer, &identity, &evidence).unwrap();
    let next = materialize(&store, &claim).unwrap();
    atomic_json(
        &next.join("dispatch-intent.json"),
        &json!({"synthetic": true}),
    )
    .unwrap();
    fs::remove_file(next.join("runner.mjs")).unwrap();
    assert!(materialize(&store, &claim)
        .unwrap_err()
        .contains("FROZEN_FILE_MISSING"));
    assert!(!next.join("runner.mjs").exists());
    assert_eq!(
        reserve(&store, &pointer, &identity, &evidence).unwrap(),
        claim
    );
}
