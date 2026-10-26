async fn hyphen_replay_fixture() -> (
    tempfile::TempDir,
    SavedTextExecutor,
    Vec<(TextRequest, Value)>,
    String,
    String,
) {
    let (input, chain) = crate::pipeline::series::series_hyphen_supplement_fixture().await;
    assert_eq!(chain.len(), 5);
    assert_eq!(
        chain[1].0.schema["properties"]["proposals"]["items"]["properties"]["key"],
        json!({"type":"string"})
    );
    let assets = input["assets"].as_array().unwrap().clone();
    let (temp, executor, origin_request, original, next) =
        fixture_request_input(chain[0].0.clone(), Some(input));
    for asset in assets {
        executor.store.save_asset(asset).unwrap();
    }
    let origin = executor
        .sdk_reserve_schema_successor_checked(&origin_request, &original, &next, || Ok(()))
        .unwrap();
    adopt_raw(&executor, &origin_request, &next, &origin, &chain[0].1);
    reject_synthetic(
        &executor,
        &origin_request,
        &next,
        &origin,
        "SERIES_CREATIVE_DEPENDENCY_UNRESOLVED",
    );
    let context = executor
        .freeze_series_supplement_identity_context(
            &origin_request,
            &chain[0].1,
            packet(&chain[1].0, "knownAssetCatalog"),
            CONTRACT_V2,
        )
        .unwrap();
    assert_eq!(context, *packet(&chain[1].0, "identityReservations"));
    let (job, snapshot) = save_received(&executor, &chain[1].0, &chain[1].1);
    let mut prepared = executor.sdk_job(&job).unwrap()["prepared"].clone();
    prepared.as_object_mut().unwrap().remove("sdkDomainResult");
    prepared
        .as_object_mut()
        .unwrap()
        .remove("sdkDomainResultSha256");
    executor
        .store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET prepared=?1 WHERE id=?2",
            params![prepared.to_string(), job],
        )
        .unwrap();
    let attempt = FrozenAttempt::load(
        &executor
            .store
            .root
            .join("text-runs")
            .join(&job)
            .join("sdk-attempt.json"),
    )
    .unwrap();
    executor
        .store
        .event(&job, "sdk_raw_result", attempt.raw_result.as_ref().unwrap())
        .unwrap();
    reject_synthetic(
        &executor,
        &chain[1].0,
        &snapshot,
        &job,
        "SERIES_SUPPLEMENT_IDENTITY_INVALID",
    );
    (temp, executor, chain, origin, job)
}

fn current_hyphen_request(old: &TextRequest) -> TextRequest {
    let mut current = old.clone();
    current.schema["properties"]["proposals"]["items"]["properties"]["key"] =
        json!({"type":"string","pattern":"^[a-z][a-z0-9_-]{0,63}$","minLength":1,"maxLength":64});
    assert_eq!(
        serde_json::to_value(
            crate::pipeline::series::legacy_supplement_design_request(&current)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(old).unwrap()
    );
    current
}

#[tokio::test]
async fn sdk_series_supplement_hyphen_revalidates_original_once_then_requires_independent_review() {
    let (_temp, executor, chain, origin, job) = hyphen_replay_fixture().await;
    let (request, raw) = &chain[1];
    let current = current_hyphen_request(request);
    let old = executor.sdk_job(&job).unwrap();
    let original_author = executor.sdk_job(&origin).unwrap();
    let original_files = file_state(&executor, &job);
    let original_events = events(&executor.store.conn().unwrap(), &job).unwrap();
    let assets_before: i64 = executor
        .store
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM assets", [], |r| r.get(0))
        .unwrap();
    let restored = executor
        .prepare_transport_request(current.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(request).unwrap()
    );
    assert_eq!(executor.execute(restored.clone()).await.unwrap(), *raw);
    assert_eq!(
        executor.sdk_job(&job).unwrap(),
        old,
        "reading original bytes does not accept the design"
    );
    executor.accept(&restored, raw).await.unwrap();
    let saved = executor.sdk_job(&job).unwrap();
    assert_eq!(saved["status"], "completed");
    assert_eq!(saved["snapshot"], old["snapshot"]);
    assert_eq!(saved["result"], old["result"]);
    assert_eq!(
        saved["receipt"]["migrationPolicy"],
        "sdk-series-supplement-opaque-key-revalidation/v1"
    );
    assert!(
        executor
            .sdk_schema_anchor_for(&job, Some(&chain[3].0))
            .is_err(),
        "a valid candidate cannot skip independent review"
    );
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    let jobs_count = reopened.store.jobs().unwrap().len();
    for _ in 0..2 {
        let original = reopened
            .prepare_transport_request(current.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&original).unwrap(),
            serde_json::to_value(request).unwrap()
        );
        assert_eq!(reopened.execute(original.clone()).await.unwrap(), *raw);
        assert_eq!(
            reopened
                .sdk_execute(&original, &saved["snapshot"])
                .await
                .unwrap(),
            *raw
        );
        reopened.accept(&original, raw).await.unwrap();
    }
    assert_eq!(reopened.store.jobs().unwrap().len(), jobs_count);
    let after = events(&reopened.store.conn().unwrap(), &job).unwrap();
    assert_eq!(
        &after.as_array().unwrap()[..original_events.as_array().unwrap().len()],
        original_events.as_array().unwrap()
    );
    let audits: Vec<_> = after
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "sdk_series_supplement_revalidated")
        .collect();
    assert_eq!(audits.len(), 1);
    assert_eq!(
        audits[0]["data"]["originalJob"]["error"],
        "SERIES_SUPPLEMENT_IDENTITY_INVALID"
    );
    assert_eq!(audits[0]["data"]["generationRequestsSent"], 0);
    assert_eq!(file_state(&reopened, &job), original_files);
    assert_eq!(reopened.sdk_job(&origin).unwrap(), original_author);
    assert_eq!(
        reopened
            .store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        assets_before,
        "design validation is not asset adoption"
    );
    assert_eq!(
        std::fs::read_dir(reopened.store.root.join("sdk-worker"))
            .unwrap()
            .count(),
        1,
        "no real SDK worker dispatched"
    );
    assert_eq!(
        reopened.sdk_schema_continuation_job(&chain[2].0).unwrap(),
        Some(job)
    );
    let (review, _) = save_received(&reopened, &chain[2].0, &chain[2].1);
    assert_eq!(
        reopened.sdk_schema_continuation_job(&chain[3].0).unwrap(),
        Some(review)
    );
}

#[tokio::test]
async fn sdk_series_supplement_hyphen_revalidation_rejects_unknown_other_error_and_changed_evidence(
) {
    for fault in [
        "unknown",
        "reason",
        "callback",
        "raw-file",
        "raw-event",
        "identity-proof",
        "origin-raw",
        "account",
    ] {
        let (_temp, mut executor, chain, origin, job) = hyphen_replay_fixture().await;
        let (request, raw) = &chain[1];
        let mut supplied = raw.clone();
        match fault {
            "unknown" => executor
                .store
                .update_job(&job, "submission_unknown", &json!({}))
                .unwrap(),
            "reason" => {
                let snapshot = executor.sdk_job(&job).unwrap()["snapshot"].clone();
                reject_synthetic(
                    &executor,
                    request,
                    &snapshot,
                    &job,
                    "SERIES_SUPPLEMENT_FACT_NOT_SOURCE_QUOTE",
                );
            }
            "callback" => supplied["proposals"][0]["key"] = json!("changed-key"),
            "raw-file" => std::fs::write(
                executor
                    .store
                    .root
                    .join("text-runs")
                    .join(&job)
                    .join("result.json"),
                "{}",
            )
            .unwrap(),
            "raw-event" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_raw_result'",
                        [&job],
                    )
                    .unwrap();
            }
            "identity-proof" => {
                executor.store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='series_supplement_identities_frozen'", [executor.workflow_id.as_deref().unwrap()]).unwrap();
            }
            "origin-raw" => std::fs::write(
                executor
                    .store
                    .root
                    .join("text-runs")
                    .join(&origin)
                    .join("result.json"),
                "{}",
            )
            .unwrap(),
            "account" => {
                executor.settings["textSdkIdentity"]["accountFingerprint"] = json!("f".repeat(64))
            }
            _ => unreachable!(),
        }
        let before = executor.store.jobs().unwrap();
        let evidence = events(&executor.store.conn().unwrap(), &job).unwrap();
        assert!(
            executor.accept(request, &supplied).await.is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        assert_eq!(
            events(&executor.store.conn().unwrap(), &job).unwrap(),
            evidence,
            "{fault}"
        );
    }
}

#[tokio::test]
async fn sdk_series_supplement_hyphen_schema_restore_preserves_unknown_and_rejects_unrelated_changes(
) {
    let (_temp, executor, chain, _origin, job) = hyphen_replay_fixture().await;
    let old = &chain[1].0;
    let current = current_hyphen_request(old);
    executor
        .store
        .update_job(&job, "submission_unknown", &json!({}))
        .unwrap();
    let before = executor.store.jobs().unwrap();
    let restored = executor
        .prepare_transport_request(current.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(old).unwrap()
    );
    assert!(executor.sdk_commit(&restored, &chain[1].1, None).is_err());
    assert_eq!(executor.store.jobs().unwrap(), before);
    for fault in ["input", "model", "schema", "budget"] {
        let mut changed = current.clone();
        match fault {
            "input" => changed.input["task"] = json!("different task"),
            "model" => changed.model = "different".into(),
            "schema" => changed.schema["additionalProperties"] = json!(true),
            "budget" => changed.input["seriesAssetSupplement"]["maximumAttempts"] = json!(20),
            _ => unreachable!(),
        }
        assert!(
            executor.prepare_transport_request(changed).await.is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before);
    }
}

#[tokio::test]
async fn sdk_series_supplement_hyphen_revalidation_receipt_binds_exact_policy_and_original_proof() {
    for fault in ["audit", "duplicate", "policy", "original-event"] {
        let (_temp, executor, chain, _origin, job) = hyphen_replay_fixture().await;
        let (request, raw) = &chain[1];
        executor.accept(request, raw).await.unwrap();
        match fault {
            "audit" => {
                executor.store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_series_supplement_revalidated'", [&job]).unwrap();
            }
            "duplicate" => {
                executor.store.conn().unwrap().execute("INSERT INTO events(job_id,type,data,created_at) SELECT job_id,type,data,created_at FROM events WHERE job_id=?1 AND type='sdk_series_supplement_revalidated'", [&job]).unwrap();
            }
            "policy" => {
                let mut receipt = executor.sdk_job(&job).unwrap()["receipt"].clone();
                receipt["migrationPolicy"] =
                    json!("sdk-series-supplement-source-quote-revalidation/v1");
                executor
                    .store
                    .update_job(&job, "completed", &json!({"receipt":receipt}))
                    .unwrap();
            }
            "original-event" => {
                executor
                    .store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_raw_result'",
                        [&job],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = executor.store.jobs().unwrap();
        let snapshot = executor.sdk_job(&job).unwrap()["snapshot"].clone();
        assert!(
            executor.sdk_execute(request, &snapshot).await.is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before);
    }
}
