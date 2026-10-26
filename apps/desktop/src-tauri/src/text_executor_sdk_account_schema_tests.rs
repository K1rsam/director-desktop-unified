// This fixture writes only synthetic, local receipts. No model or runtime call.
fn schema_account_snapshot(
    executor: &SavedTextExecutor,
    request: &TextRequest,
    identity: &Value,
    anchor: &Value,
    account: &str,
) -> Value {
    let mut current = identity.clone();
    current["accountFingerprint"] = json!(account);
    let mut snapshot = continuation_snapshot(executor, request, &current, anchor);
    let reference = executor.sdk_current_account_reference().unwrap().unwrap();
    let authorization = executor
        .sdk_current_account_authorization(request, &current, Some(&reference), CONTINUATION)
        .unwrap();
    snapshot[CONTINUE].as_object_mut().unwrap().remove("sha256");
    snapshot[CONTINUE]["version"] = json!(ACCOUNT_CONTINUATION);
    snapshot[CONTINUE]["accountAuthorization"] = authorization;
    snapshot[CONTINUE] = sealed(snapshot[CONTINUE].clone());
    snapshot
}
#[test]
fn sdk_current_account_schema_continuation_tracks_actual_previous_not_semantic_anchor() {
    let (_temp, executor, request, original, next) = fixture();
    let origin = executor
        .sdk_reserve_schema_successor_checked(&request, &original, &next, || Ok(()))
        .unwrap();
    adopt_synthetic_successor(&executor, &request, &next, &origin);
    let origin_before = executor.sdk_job(&origin).unwrap();
    let anchor = executor.sdk_schema_anchor(&origin).unwrap();
    let identity = next["executorIdentity"].clone();
    let original_account = s(&identity, "accountFingerprint").to_owned();
    let parent_before = executor
        .store
        .job(executor.workflow_id.as_deref().unwrap())
        .unwrap();
    let mut prior = origin.clone();
    for (index, account) in ["b".repeat(64), "d".repeat(64), original_account]
        .iter()
        .enumerate()
    {
        let mut later = request.clone();
        later.stage = format!("seriesCurrentAccount{index}");
        let snapshot = schema_account_snapshot(&executor, &later, &identity, &anchor, account);
        assert_eq!(snapshot[CONTINUE]["anchor"]["jobId"], origin);
        assert_eq!(
            snapshot[CONTINUE]["accountAuthorization"]["previousNode"]["jobId"],
            prior
        );
        let mut legacy = snapshot.clone();
        legacy[CONTINUE].as_object_mut().unwrap().remove("sha256");
        legacy[CONTINUE]
            .as_object_mut()
            .unwrap()
            .remove("accountAuthorization");
        legacy[CONTINUE]["version"] = json!(CONTINUATION);
        legacy[CONTINUE] = sealed(legacy[CONTINUE].clone());
        if index < 2 {
            assert!(
                executor.sdk_schema_continuation_bound(&legacy).is_err(),
                "old v1 cannot change account"
            );
        }
        executor.sdk_schema_continuation_bound(&snapshot).unwrap();
        let job = executor
            .sdk_allocate_pre_dispatch(&later, &snapshot)
            .unwrap();
        let mut self_reference = snapshot[CONTINUE]["accountAuthorization"].clone();
        self_reference["previousNode"]["jobId"] = json!(job);
        assert!(
            executor
                .sdk_current_account_authorization_bound(&snapshot, &self_reference, CONTINUATION)
                .is_err(),
            "row order rejects self-reference before raw traversal"
        );
        assert_eq!(executor.request_snapshot(&later).unwrap(), snapshot);
        adopt_synthetic_successor(&executor, &later, &snapshot, &job);
        assert!(!executor.store.root.join("sdk-worker").join(&job).exists());
        prior = job;
    }
    let before = executor.store.jobs().unwrap();
    validation_profile::begin();
    let reference = executor.sdk_current_account_reference();
    let profile = validation_profile::finish();
    assert_eq!(reference.unwrap().unwrap().1["jobId"], prior);
    let raw_calls: u64 = profile["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["name"] == "sdk_raw")
        .map(|row| row["calls"].as_u64().unwrap())
        .sum();
    assert!(
        raw_calls <= 8,
        "semantic anchor plus account worklist must remain bounded: {profile}"
    );
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    let latest = reopened.sdk_job(&prior).unwrap();
    assert!(reopened.sdk_raw(&latest, &latest["snapshot"]).is_ok());
    assert_eq!(reopened.store.jobs().unwrap(), before);
    assert_eq!(reopened.sdk_job(&origin).unwrap(), origin_before);
    assert_eq!(
        reopened
            .store
            .job(executor.workflow_id.as_deref().unwrap())
            .unwrap(),
        parent_before
    );
}
#[test]
fn sdk_current_account_schema_unknown_or_bad_authorization_cannot_allocate() {
    for fault in [
        "unknown",
        "missing",
        "model",
        "reasoning",
        "previous",
        "identity",
        "permission",
        "anchor",
    ] {
        let (_temp, executor, request, original, next) = fixture();
        let origin = executor
            .sdk_reserve_schema_successor_checked(&request, &original, &next, || Ok(()))
            .unwrap();
        adopt_synthetic_successor(&executor, &request, &next, &origin);
        let anchor = executor.sdk_schema_anchor(&origin).unwrap();
        let mut later = request.clone();
        later.stage = "seriesNextAccount".into();
        let mut snapshot = schema_account_snapshot(
            &executor,
            &later,
            &next["executorIdentity"],
            &anchor,
            &"b".repeat(64),
        );
        match fault {
            "unknown" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                        [&origin],
                    )
                    .unwrap();
            }
            "missing" => {
                snapshot[CONTINUE]
                    .as_object_mut()
                    .unwrap()
                    .remove("accountAuthorization");
            }
            "model" => snapshot[CONTINUE]["accountAuthorization"]["model"] = json!("different"),
            "reasoning" => snapshot[CONTINUE]["accountAuthorization"]["reasoning"] = json!("high"),
            "previous" => snapshot[CONTINUE]["accountAuthorization"]["previousNode"] = Value::Null,
            "identity" => {
                snapshot[CONTINUE]["accountAuthorization"]["previousIdentitySha256"] =
                    json!("incorrect")
            }
            "permission" => snapshot["executorIdentity"]["permissionProfile"] = json!("different"),
            "anchor" => snapshot[CONTINUE]["anchor"]["jobId"] = json!("missing"),
            _ => unreachable!(),
        }
        snapshot[CONTINUE].as_object_mut().unwrap().remove("sha256");
        snapshot[CONTINUE] = sealed(snapshot[CONTINUE].clone());
        let before = executor.store.jobs().unwrap();
        assert!(
            executor.sdk_schema_continuation_bound(&snapshot).is_err(),
            "{fault}"
        );
        assert!(
            executor
                .sdk_allocate_pre_dispatch(&later, &snapshot)
                .is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
    }
}
#[test]
fn sdk_current_account_same_logical_schema_retry_still_requires_original_account() {
    let (_temp, executor, request, original, mut next) = fixture();
    next["executorIdentity"]["accountFingerprint"] = json!("b".repeat(64));
    next[RETRY].as_object_mut().unwrap().remove("sha256");
    next[RETRY]["effectiveIdentitySha256"] = json!(hash(&next["executorIdentity"]));
    next[RETRY] = sealed(next[RETRY].clone());
    let before = executor.store.jobs().unwrap();
    assert!(executor
        .sdk_reserve_schema_successor_checked(&request, &original, &next, || Ok(()))
        .is_err());
    assert_eq!(executor.store.jobs().unwrap(), before);
}
