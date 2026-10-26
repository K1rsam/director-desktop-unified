// Synthetic only: no provider, credential discovery, or executable is run.
fn account_snapshot(
    executor: &SavedTextExecutor,
    request: &TextRequest,
    template: &Value,
    account: &str,
) -> Value {
    let mut snapshot = executor.request_snapshot(request).unwrap();
    let (base, prior) = executor.sdk_handoff_predecessor(&snapshot, false).unwrap();
    let reference = executor.sdk_current_account_reference().unwrap();
    // Keep exactly the original supervisor/resources: an account-only switch.
    snapshot["executorIdentity"]["accountFingerprint"] = json!(account.repeat(64));
    let authorization = executor
        .sdk_current_account_authorization(
            request,
            &snapshot["executorIdentity"],
            reference.as_ref(),
            base,
        )
        .unwrap();
    let parent = executor
        .store
        .job(executor.workflow_id.as_deref().unwrap())
        .unwrap()["snapshot"]
        .clone();
    snapshot[FIELD] = seal(
        json!({"policy":ACCOUNT_POLICY,"workflowId":executor.workflow_id,"nodeKey":step_key(request),"stage":request.stage,
        "domainRequestSha256":hash(&domain(&snapshot).unwrap()),"parentSnapshotSha256":hash(&parent),"originalSettingsSha256":hash(&executor.settings),
        "originalIdentity":executor.settings["textSdkIdentity"],"effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),
        "resourceHashes":template[FIELD]["resourceHashes"],"predecessor":prior,"modelCalls":0,"accountModelChecked":true,"accountAuthorization":authorization}),
    );
    snapshot
}
fn reseal_account(snapshot: &mut Value) {
    snapshot[FIELD].as_object_mut().unwrap().remove("sha256");
    snapshot[FIELD] = seal(snapshot[FIELD].clone());
}
#[test]
fn sdk_current_account_same_exe_new_node_preserves_v1_parent_raw_and_reopens() {
    let (_temp, executor, request, template) = fixture();
    let snapshot = account_snapshot(&executor, &request, &template, "b");
    assert_eq!(
        snapshot["executorIdentity"]["supervisorExecutable"],
        executor.settings["textSdkIdentity"]["supervisorExecutable"]
    );
    let before = first_node_stored_evidence(&executor);
    let prior = s(&snapshot[FIELD]["predecessor"], "jobId");
    let old = executor.sdk_job(prior).unwrap();
    let bytes = std::fs::read(
        executor
            .store
            .root
            .join("text-runs")
            .join(prior)
            .join("sdk-attempt.json"),
    )
    .unwrap();
    let mut legacy = snapshot.clone();
    legacy[FIELD]["policy"] = json!(base_policy(&snapshot[FIELD]));
    legacy[FIELD]
        .as_object_mut()
        .unwrap()
        .remove("accountAuthorization");
    reseal_account(&mut legacy);
    assert!(
        add(&executor, &request, &legacy).is_err(),
        "v1 does not gain account-switch permission"
    );
    assert_eq!(first_node_stored_evidence(&executor), before);
    let job = add(&executor, &request, &snapshot).unwrap();
    assert_eq!(executor.sdk_job(&job).unwrap()["accountId"], "b".repeat(64));
    assert_eq!(
        executor.sdk_job(&job).unwrap()["prepared"]["sdkDispatchStarted"],
        false
    );
    assert_eq!(executor.request_snapshot(&request).unwrap(), snapshot);
    complete(&executor, &request, &snapshot, &job, true);
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    assert_eq!(reopened.request_snapshot(&request).unwrap(), snapshot);
    assert_eq!(
        reopened
            .sdk_raw(&reopened.sdk_job(&job).unwrap(), &snapshot)
            .unwrap(),
        json!({"ok":true})
    );
    assert_eq!(reopened.sdk_job(prior).unwrap(), old);
    assert_eq!(
        std::fs::read(
            executor
                .store
                .root
                .join("text-runs")
                .join(prior)
                .join("sdk-attempt.json")
        )
        .unwrap(),
        bytes
    );
    assert!(
        !executor.store.root.join("sdk-worker").exists(),
        "historical replay starts no runtime"
    );
}
#[test]
fn sdk_current_account_a_b_c_a_is_local_bounded_and_detects_old_file_drift() {
    let (_temp, executor, mut request, template) = fixture();
    let parent = executor
        .store
        .job(executor.workflow_id.as_deref().unwrap())
        .unwrap();
    let first = s(&template[FIELD]["predecessor"], "jobId").to_owned();
    let mut previous = "a";
    for (index, account) in ["b", "c", "a"].iter().enumerate() {
        request.stage = format!("foundation:ACCOUNT_{index}");
        let snapshot = account_snapshot(&executor, &request, &template, account);
        assert_eq!(
            snapshot[FIELD]["accountAuthorization"]["previousAccountFingerprint"],
            previous.repeat(64)
        );
        let job = add(&executor, &request, &snapshot).unwrap();
        complete(&executor, &request, &snapshot, &job, true);
        previous = account;
    }
    let before = first_node_stored_evidence(&executor);
    validation_profile::begin();
    let reference = executor.sdk_current_account_reference();
    let profile = validation_profile::finish();
    assert_eq!(
        reference.unwrap().unwrap().0["accountFingerprint"],
        "a".repeat(64)
    );
    let raw_calls: u64 = profile["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["name"] == "sdk_raw")
        .map(|row| row["calls"].as_u64().unwrap())
        .sum();
    assert_eq!(
        raw_calls, 4,
        "each of the four account-lineage nodes is checked once: {profile}"
    );
    assert!(
        profile["maxDepth"].as_u64().unwrap() <= 2,
        "account chain must not recurse: {profile}"
    );
    assert_eq!(first_node_stored_evidence(&executor), before);
    assert_eq!(
        executor
            .store
            .job(executor.workflow_id.as_deref().unwrap())
            .unwrap(),
        parent
    );
    assert!(!executor.store.root.join("sdk-worker").exists());
    std::fs::write(
        executor
            .store
            .root
            .join("text-runs")
            .join(first)
            .join("result.json"),
        "{}",
    )
    .unwrap();
    assert!(
        executor.sdk_current_account_reference().is_err(),
        "no proof survives its synchronous scope"
    );
}
#[test]
fn sdk_current_account_first_empty_node_reserves_once_and_queued_identity_is_frozen() {
    let (_temp, executor, request, template) = first_node_fixture();
    let snapshot = account_snapshot(&executor, &request, &template, "b");
    assert_eq!(base_policy(&snapshot[FIELD]), FIRST_NODE_POLICY);
    assert!(snapshot[FIELD]["accountAuthorization"]["previousNode"].is_null());
    let job = add(&executor, &request, &snapshot).unwrap();
    let before = first_node_stored_evidence(&executor);
    let mut changed = snapshot.clone();
    changed["executorIdentity"]["accountFingerprint"] = json!("c".repeat(64));
    assert!(add(&executor, &request, &changed).is_err());
    assert_eq!(
        executor.request_snapshot(&request).unwrap(),
        snapshot,
        "an already queued request is not rebound"
    );
    assert_eq!(executor.sdk_job(&job).unwrap()["status"], "queued");
    assert_eq!(first_node_stored_evidence(&executor), before);
}
#[test]
fn sdk_current_account_proof_unknown_component_and_orphan_faults_leave_no_new_job() {
    for fault in [
        "missing",
        "null",
        "previous",
        "model",
        "reasoning",
        "checked",
        "permission",
        "node-bytes",
        "raw",
        "unknown",
        "schema-orphan",
    ] {
        let (temp, executor, request, template) = fixture();
        let mut snapshot = account_snapshot(&executor, &request, &template, "b");
        match fault {
            "missing" => {
                snapshot[FIELD]
                    .as_object_mut()
                    .unwrap()
                    .remove("accountAuthorization");
            }
            "null" => snapshot[FIELD]["accountAuthorization"] = Value::Null,
            "previous" => {
                snapshot[FIELD]["accountAuthorization"]["previousNode"]["jobId"] =
                    json!("unrelated")
            }
            "model" => snapshot[FIELD]["accountAuthorization"]["model"] = json!("different"),
            "reasoning" => snapshot[FIELD]["accountAuthorization"]["reasoning"] = json!("high"),
            "checked" => {
                snapshot[FIELD]["accountAuthorization"]["accountModelChecked"] = json!(false)
            }
            "permission" => snapshot["executorIdentity"]["permissionProfile"] = json!("different"),
            "node-bytes" => std::fs::write(temp.path().join("node.exe"), "changed").unwrap(),
            "raw" => std::fs::write(
                executor
                    .store
                    .root
                    .join("text-runs")
                    .join(s(&snapshot[FIELD]["predecessor"], "jobId"))
                    .join("result.json"),
                "{}",
            )
            .unwrap(),
            "unknown" => {
                let job = executor
                    .store
                    .add_job(
                        "text",
                        "unknown",
                        "codex_sdk",
                        "synthetic",
                        &json!({"stage":"other"}),
                        "submission_unknown",
                    )
                    .unwrap();
                executor
                    .store
                    .update_job(
                        &job,
                        "submission_unknown",
                        &json!({"prepared":{"sdkDispatchStarted":true}}),
                    )
                    .unwrap();
            }
            "schema-orphan" => {
                let mut orphan = domain(&snapshot).unwrap();
                orphan["executorIdentity"] = snapshot["executorIdentity"].clone();
                orphan["sdkSchemaContinuation"] =
                    json!({"workflowId":executor.workflow_id,"nodeKey":step_key(&request)});
                executor
                    .store
                    .add_job(
                        "text",
                        "orphan",
                        "codex_sdk",
                        "synthetic",
                        &orphan,
                        "queued",
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        reseal_account(&mut snapshot);
        let before = first_node_stored_evidence(&executor);
        assert!(add(&executor, &request, &snapshot).is_err(), "{fault}");
        assert_eq!(first_node_stored_evidence(&executor), before, "{fault}");
    }
}
