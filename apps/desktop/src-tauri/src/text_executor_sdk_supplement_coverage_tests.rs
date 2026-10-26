async fn coverage_restore_fixture(
    v2: bool,
) -> (
    tempfile::TempDir,
    SavedTextExecutor,
    Vec<(TextRequest, Value)>,
) {
    if v2 {
        let (temp, executor, chain, _origin, _job) = hyphen_replay_fixture().await;
        executor.accept(&chain[1].0, &chain[1].1).await.unwrap();
        (temp, executor, chain)
    } else {
        let (temp, executor, chain, _origin) = fixture().await;
        save_received(&executor, &chain[1].0, &chain[1].1);
        (temp, executor, chain)
    }
}

#[tokio::test]
async fn sdk_series_supplement_review_coverage_restores_v1_v2_saved_digest_or_current_without_dispatch(
) {
    for v2 in [false, true] {
        for historical in [false, true] {
            let (_temp, executor, chain) = coverage_restore_fixture(v2).await;
            let current = &chain[2].0;
            assert!(current.input.get("reviewCoverageBinding").is_some());
            let legacy = crate::pipeline::series::legacy_supplement_coverage_request(current)
                .unwrap()
                .unwrap();
            assert!(legacy.input.get("reviewCoverageBinding").is_none());
            assert!(legacy.input.get("reviewDigestBinding").is_some());
            let frozen = if historical { &legacy } else { current };
            let (job, snapshot) = save_received(&executor, frozen, &chain[2].1);
            let original = executor.sdk_job(&job).unwrap();
            let files = file_state(&executor, &job);
            let events_before = events(&executor.store.conn().unwrap(), &job).unwrap();
            let count = executor.store.jobs().unwrap().len();
            let reopened = SavedTextExecutor {
                store: Store::open(executor.store.root.clone()).unwrap(),
                settings: executor.settings.clone(),
                workflow_id: executor.workflow_id.clone(),
            };
            for _ in 0..2 {
                let restored = reopened
                    .prepare_transport_request(current.clone())
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&restored).unwrap(),
                    serde_json::to_value(frozen).unwrap()
                );
                assert_eq!(reopened.request_snapshot(&restored).unwrap(), snapshot);
                assert_eq!(reopened.execute(restored).await.unwrap(), chain[2].1);
                assert_eq!(reopened.sdk_job(&job).unwrap(), original);
            }
            assert_eq!(reopened.store.jobs().unwrap().len(), count);
            assert_eq!(file_state(&reopened, &job), files);
            assert_eq!(
                events(&reopened.store.conn().unwrap(), &job).unwrap(),
                events_before
            );
            assert_eq!(
                std::fs::read_dir(reopened.store.root.join("sdk-worker"))
                    .unwrap()
                    .count(),
                1
            );
            reopened
                .store
                .update_job(&job, "submission_unknown", &json!({}))
                .unwrap();
            let before = reopened.store.jobs().unwrap();
            let restored = reopened
                .prepare_transport_request(current.clone())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&restored).unwrap(),
                serde_json::to_value(frozen).unwrap()
            );
            assert!(reopened.sdk_commit(&restored, &chain[2].1, None).is_err());
            assert_eq!(
                reopened.store.jobs().unwrap(),
                before,
                "request restoration cannot settle UNKNOWN"
            );
        }
    }
}

#[tokio::test]
async fn sdk_series_supplement_review_coverage_restore_rejects_input_schema_budget_and_account_drift(
) {
    let (_temp, mut executor, chain) = coverage_restore_fixture(true).await;
    let current = &chain[2].0;
    let legacy = crate::pipeline::series::legacy_supplement_coverage_request(current)
        .unwrap()
        .unwrap();
    let (job, _) = save_received(&executor, &legacy, &chain[2].1);
    let before = executor.store.jobs().unwrap();
    let files = file_state(&executor, &job);
    for fault in [
        "coverage",
        "instructions",
        "candidate",
        "task",
        "schema",
        "budget",
        "model",
    ] {
        let mut changed = current.clone();
        match fault {
            "coverage" => {
                changed.input["expectedReviewCoverage"]["reviewedIssueIndexes"] = json!([])
            }
            "instructions" => changed.input["reviewResponseContract"] = json!("auto pass"),
            "candidate" => changed.input["candidateSha256"] = json!("0".repeat(64)),
            "task" => changed.input["task"] = json!("different task"),
            "schema" => changed.schema["additionalProperties"] = json!(true),
            "budget" => changed.input["seriesAssetSupplement"]["maximumAttempts"] = json!(20),
            "model" => changed.model = "different".into(),
            _ => unreachable!(),
        }
        assert!(
            executor.prepare_transport_request(changed).await.is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        assert_eq!(file_state(&executor, &job), files, "{fault}");
    }
    executor.settings["textSdkIdentity"]["accountFingerprint"] = json!("f".repeat(64));
    assert!(executor
        .prepare_transport_request(current.clone())
        .await
        .is_err());
    assert_eq!(executor.store.jobs().unwrap(), before);
}
