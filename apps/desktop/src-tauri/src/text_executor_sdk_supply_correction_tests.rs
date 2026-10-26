use super::super::tests::{
    adopt_raw, continuation_snapshot, domain_repair, fixture_request_input,
    fixture_request_input_with_workflow, reject_synthetic,
};
use super::*;

async fn rejected_origin() -> (
    tempfile::TempDir,
    SavedTextExecutor,
    TextRequest,
    Value,
    String,
) {
    let (mut input, chain) = crate::pipeline::series::series_supply_fixture().await;
    input["source"]["version"] = json!(1);
    let assets = input["assets"].as_array().unwrap().clone();
    let source = input["source"].clone();
    let (temp, executor, origin, original, next) =
        fixture_request_input(chain[0].0.clone(), Some(input));
    executor.store.save_source(source).unwrap();
    for asset in assets {
        executor.store.save_asset(asset).unwrap();
    }
    let job = executor
        .sdk_reserve_schema_successor_checked(&origin, &original, &next, || Ok(()))
        .unwrap();
    let raw = chain[0].1.clone();
    adopt_raw(&executor, &origin, &next, &job, &raw);
    reject_synthetic(
        &executor,
        &origin,
        &next,
        &job,
        "SERIES_CREATIVE_DEPENDENCY_UNRESOLVED",
    );
    (temp, executor, origin, raw, job)
}
fn event_count(executor: &SavedTextExecutor) -> i64 {
    executor
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE type IN (?1,?2)",
            params![AUTH_EVENT, SLOT_EVENT],
            |r| r.get(0),
        )
        .unwrap()
}
#[tokio::test]
async fn sdk_supply_correction_unresolved_does_not_automatically_open_budget() {
    let (_temp, executor, origin, raw, _job) = rejected_origin().await;
    let before = executor.store.jobs().unwrap();
    assert!(executor
        .series_supply_correction_context(&origin, &raw)
        .await
        .unwrap()
        .is_none());
    assert_eq!(executor.store.jobs().unwrap(), before);
    assert_eq!(event_count(&executor), 0);
}
#[tokio::test]
async fn sdk_supply_correction_explicit_auth_requires_complete_exhausted_old_chain() {
    let (_temp, executor, origin, raw, _job) = rejected_origin().await;
    let parent = executor
        .store
        .job(executor.workflow_id.as_deref().unwrap())
        .unwrap();
    let auth = json!({"version":AUTH_REQUEST,"expectedWorkflowSnapshotSha256":hash(&parent["snapshot"]),"originRequestSha256":request_hash(&origin).unwrap(),
        "expectedFinalReviewSha256":"a".repeat(64),"auditSha256":"b".repeat(64),"affectedAssets":[],"reason":"synthetic audit","acknowledgeAdditionalTextUsage":true});
    for fault in [
        "missing-chain",
        "no-ack",
        "wrong-version",
        "wrong-workflow",
        "no-reason",
    ] {
        let mut changed = auth.clone();
        match fault {
            "no-ack" => changed["acknowledgeAdditionalTextUsage"] = json!(false),
            "wrong-version" => changed["version"] = json!("other"),
            "wrong-workflow" => changed["expectedWorkflowSnapshotSha256"] = json!("c".repeat(64)),
            "no-reason" => changed["reason"] = json!(""),
            _ => (),
        }
        let before = executor.store.jobs().unwrap();
        assert!(
            executor
                .authorize_series_supply_correction(&changed)
                .is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        assert_eq!(event_count(&executor), 0);
    }
    assert!(executor
        .saved_series_supply_correction_context(&origin, &raw)
        .unwrap()
        .is_none());
}
#[tokio::test]
async fn sdk_supply_correction_orphan_authorization_cannot_be_recreated() {
    let (_temp, executor, origin, raw, _job) = rejected_origin().await;
    executor.store.event(executor.workflow_id.as_deref().unwrap(),AUTH_EVENT,&json!({"originRequestSha256":request_hash(&origin).unwrap(),"contextSha256":"d".repeat(64)})).unwrap();
    let before = executor.store.jobs().unwrap();
    assert!(executor
        .saved_series_supply_correction_context(&origin, &raw)
        .is_err());
    assert_eq!(executor.store.jobs().unwrap(), before);
    assert_eq!(event_count(&executor), 1);
}
#[tokio::test]
async fn sdk_supply_correction_role_without_authorization_never_reserves_or_dispatches() {
    let (_temp, executor, origin, raw, _job) = rejected_origin().await;
    let mut request = origin.clone();
    request.stage = "seriesDesign".into();
    request.skill = "series-design".into();
    request.input["seriesSupplyCorrection"] = json!({"version":CONTRACT,"phase":"design","attempt":0,"maximumAttempts":1,"maximumCorrections":1,
        "originExecutionRoleId":origin.input["executionRoleId"],"originRequestSha256":request_hash(&origin).unwrap(),"originRawSha256":hash(&raw),
        "predecessorExecutionRoleId":origin.input["executionRoleId"],"predecessorRequestSha256":request_hash(&origin).unwrap(),"predecessorRawSha256":hash(&raw),"contextSha256":"e".repeat(64)});
    for fault in ["no-authorization", "budget", "attempt", "mixed-contract"] {
        let mut changed = request.clone();
        match fault {
            "budget" => changed.input["seriesSupplyCorrection"]["maximumAttempts"] = json!(2),
            "attempt" => changed.input["seriesSupplyCorrection"]["attempt"] = json!(2),
            "mixed-contract" => {
                changed.input["seriesAssetSupplement"] =
                    json!({"version":"series-asset-supplement/v2"})
            }
            _ => (),
        }
        let mut snapshot = serde_json::to_value(&changed).unwrap();
        snapshot["executorIdentity"] = executor.settings["textSdkIdentity"].clone();
        let before = executor.store.jobs().unwrap();
        assert!(
            executor
                .sdk_allocate_pre_dispatch(&changed, &snapshot)
                .is_err(),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        assert_eq!(event_count(&executor), 0);
    }
}

fn binding<'a>(request: &'a TextRequest, kind: &str) -> &'a Value {
    let packet = &request.input["packet"];
    let b = packet["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == kind)
        .unwrap();
    &packet["blobs"][s(b, "blob")]["value"]
}
fn save_role(executor: &SavedTextExecutor, request: &TextRequest, raw: &Value) -> (String, Value) {
    let started = std::time::Instant::now();
    let role = s(&request.input, "executionRoleId");
    let phase = |label: &str| {
        eprintln!(
            "[sdk-correction-save-role] role={role} phase={label} elapsed_ms={}",
            started.elapsed().as_millis()
        )
    };
    phase("predecessor-start");
    let predecessor = executor
        .sdk_schema_continuation_job(request)
        .unwrap()
        .unwrap();
    phase("anchor-start");
    let anchor = executor
        .sdk_schema_anchor_for(&predecessor, Some(request))
        .unwrap();
    let identity = executor.sdk_job(&predecessor).unwrap()["snapshot"]["executorIdentity"].clone();
    let snapshot = continuation_snapshot(executor, request, &identity, &anchor);
    phase("continuation-bound-start");
    executor.sdk_schema_continuation_bound(&snapshot).unwrap();
    phase("allocation-start");
    let job = executor
        .sdk_allocate_pre_dispatch(request, &snapshot)
        .unwrap();
    phase("adopt-raw-start");
    adopt_raw(executor, request, &snapshot, &job, raw);
    phase("complete");
    (job, snapshot)
}
async fn authorized_fixture() -> (
    tempfile::TempDir,
    SavedTextExecutor,
    Value,
    Value,
    Vec<(TextRequest, Value)>,
    Value,
    Vec<Value>,
) {
    authorized_fact_fixture(false).await
}
async fn authorized_fact_fixture(
    summarize: bool,
) -> (
    tempfile::TempDir,
    SavedTextExecutor,
    Value,
    Value,
    Vec<(TextRequest, Value)>,
    Value,
    Vec<Value>,
) {
    let workflow = id();
    let started = std::time::Instant::now();
    let (input, legacy, context, calls, delivery) = if summarize {
        crate::pipeline::series::series_supply_correction_fact_fixture(&workflow).await
    } else {
        crate::pipeline::series::series_supply_correction_fixture(&workflow).await
    };
    eprintln!(
        "supply correction synthetic domain chain: {} ms",
        started.elapsed().as_millis()
    );
    let initial_origin = legacy
        .iter()
        .find(|(r, _)| r.stage == "seriesAuthor")
        .unwrap()
        .0
        .clone();
    let first_design = legacy
        .iter()
        .find(|(r, _)| r.input["seriesAssetSupplement"]["phase"] == "design")
        .unwrap();
    let assets = binding(&first_design.0, "knownAssetCatalog")
        .as_array()
        .unwrap()
        .iter()
        .map(|known| {
            // The known catalogue intentionally omits full designs. Seed the
            // durable store from its real prior asset, only taking allocated
            // identity fields from the frozen catalogue.
            let mut asset = input["assets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == known["id"])
                .unwrap()
                .clone();
            for field in ["importName", "filename", "placeholderOrdinal"] {
                asset[field] = known[field].clone();
            }
            asset
        })
        .collect::<Vec<_>>();
    let source = input["source"].clone();
    let (temp, executor, origin, original, next) =
        fixture_request_input_with_workflow(initial_origin, Some(input), Some(&workflow));
    let run = executor.workflow_id.as_deref().unwrap();
    executor.store.save_source(source).unwrap();
    for asset in assets {
        executor.store.save_asset(asset).unwrap();
    }
    let (_, raw) = legacy
        .iter()
        .find(|(r, _)| r.stage == "seriesAuthor")
        .unwrap();
    let job = executor
        .sdk_reserve_schema_successor_checked(&origin, &original, &next, || Ok(()))
        .unwrap();
    adopt_raw(&executor, &origin, &next, &job, raw);
    reject_synthetic(
        &executor,
        &origin,
        &next,
        &job,
        s(
            &delivery["adoptionReceipt"]["supplyContractCorrection"]["origin"],
            "reason",
        ),
    );
    let design = legacy
        .iter()
        .find(|(r, _)| r.input["seriesAssetSupplement"]["phase"] == "design")
        .unwrap();
    let known = binding(&design.0, "knownAssetCatalog");
    eprintln!(
        "supply correction SDK original author ready: {} ms",
        started.elapsed().as_millis()
    );
    let identities = executor
        .freeze_series_supplement_identity_context(
            &origin,
            raw,
            known,
            s(&design.0.input["seriesAssetSupplement"], "version"),
        )
        .unwrap();
    assert_eq!(identities, *binding(&design.0, "identityReservations"));
    let mut old_ids = vec![job];
    for (request, value) in legacy
        .iter()
        .filter(|(r, _)| r.input.get("seriesAssetSupplement").is_some())
    {
        old_ids.push(save_role(&executor, request, value).0);
        eprintln!(
            "supply correction saved {} attempt {}: {} ms",
            request.stage,
            request.input["seriesAssetSupplement"]["attempt"],
            started.elapsed().as_millis()
        );
    }
    executor
        .store
        .update_job(
            run,
            "partially_blocked",
            &json!({"error":"SERIES_SUPPLEMENT_REVIEW_REPAIR_HELD"}),
        )
        .unwrap();
    let parent = executor.store.job(run).unwrap();
    let authorization = json!({"version":AUTH_REQUEST,"expectedWorkflowSnapshotSha256":hash(&parent["snapshot"]),
        "originRequestSha256":context["originRequestSha256"],"expectedFinalReviewSha256":context["legacyReceipts"][5]["rawSha256"],
        "auditSha256":context["authorization"]["auditSha256"],"affectedAssets":context["authorization"]["affectedAssets"],
        "reason":"synthetic independent root-cause audit confirms missing revise contract","acknowledgeAdditionalTextUsage":true});
    verify_affected(&executor.store.conn().unwrap(), &context).unwrap();
    eprintln!(
        "[sdk-correction-authorization] start elapsed_ms={}",
        started.elapsed().as_millis()
    );
    assert_eq!(
        executor
            .authorize_series_supply_correction(&authorization)
            .unwrap()["reused"],
        false
    );
    eprintln!(
        "[sdk-correction-authorization] authorized elapsed_ms={}",
        started.elapsed().as_millis()
    );
    assert_eq!(
        executor
            .saved_series_supply_correction_context(&origin, raw)
            .unwrap(),
        Some(context.clone())
    );
    eprintln!(
        "[sdk-correction-authorization] context-read elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let c = executor.store.conn().unwrap();
    let evidence=old_ids.iter().map(|id|json!({"record":job_record(&c,id).unwrap(),"events":events(&c,id).unwrap(),"step":step_record(&c,run,id).unwrap()})).collect();
    (
        temp,
        executor,
        authorization,
        context,
        calls,
        delivery,
        evidence,
    )
}
#[tokio::test]
async fn sdk_supply_correction_authorization_reopen_is_idempotent_and_never_changes_old_budget() {
    let (_temp, executor, auth, context, _calls, _delivery, old) = authorized_fixture().await;
    let before = executor.store.jobs().unwrap();
    assert_eq!(
        executor.authorize_series_supply_correction(&auth).unwrap()["reused"],
        true
    );
    assert_eq!(executor.store.jobs().unwrap(), before);
    assert_eq!(event_count(&executor), 1);
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    assert_eq!(
        reopened.authorize_series_supply_correction(&auth).unwrap()["reused"],
        true
    );
    let mut different = auth.clone();
    different["auditSha256"] = json!("f".repeat(64));
    assert!(reopened
        .authorize_series_supply_correction(&different)
        .is_err());
    let c = reopened.store.conn().unwrap();
    for original in old {
        let id = s(&original["record"], "jobId");
        assert_eq!(job_record(&c, id).unwrap(), original["record"]);
        assert_eq!(events(&c, id).unwrap(), original["events"]);
        assert_eq!(
            step_record(&c, reopened.workflow_id.as_deref().unwrap(), id).unwrap(),
            original["step"]
        );
    }
    assert!(context["legacyReceipts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["request"]["input"]["seriesAssetSupplement"]["maximumAttempts"] == 2));
}

#[tokio::test]
async fn sdk_supply_fact_revalidation_preserves_raw_budget_and_reaches_only_independent_review() {
    let (_temp, executor, _auth, _context, calls, _delivery, old) =
        authorized_fact_fixture(true).await;
    let (request, raw) = calls
        .iter()
        .find(|(r, _)| {
            r.input["seriesSupplyCorrection"]["phase"] == "design"
                && r.input["seriesSupplyCorrection"]["attempt"] == 0
        })
        .unwrap();
    let (job, snapshot) = save_role(&executor, request, raw);
    let mut prepared = executor.sdk_job(&job).unwrap()["prepared"].clone();
    for key in ["sdkDomainResult", "sdkDomainResultSha256"] {
        prepared.as_object_mut().unwrap().remove(key);
    }
    executor
        .store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET prepared=?1 WHERE id=?2",
            params![prepared.to_string(), job],
        )
        .unwrap();
    let directory = executor.store.root.join("text-runs").join(&job);
    let attempt = FrozenAttempt::load(&directory.join("sdk-attempt.json")).unwrap();
    executor
        .store
        .event(&job, "sdk_raw_result", attempt.raw_result.as_ref().unwrap())
        .unwrap();
    reject_synthetic(
        &executor,
        request,
        &snapshot,
        &job,
        "SERIES_SUPPLY_CORRECTION_FACT_NOT_SOURCE_QUOTE",
    );
    let rejected = executor.sdk_job(&job).unwrap();
    let prior_events = events(&executor.store.conn().unwrap(), &job).unwrap();
    let files = || {
        json!(["sdk-attempt.json", "sdk-compiled.json", "result.json"]
            .iter()
            .map(|name| crate::store::hash_file(&directory.join(name)).unwrap())
            .collect::<Vec<_>>())
    };
    let original_files = files();
    let raw_bytes = std::fs::read(directory.join("result.json")).unwrap();
    let assets = executor.store.values("assets").unwrap();
    let source = executor.store.source().unwrap();
    let job_count = executor.store.jobs().unwrap().len();
    let c = executor.store.conn().unwrap();
    let (source_row, source_json): (i64, String) = c
        .query_row(
            "SELECT rowid,data FROM sources ORDER BY version DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let (asset_id, asset_version): (String, i64) = c
        .query_row("SELECT id,version FROM assets WHERE id='s'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let (auth_id, auth_json): (i64, String) = c
        .query_row(
            "SELECT id,data FROM events WHERE job_id=?1 AND type=?2",
            params![executor.workflow_id, AUTH_EVENT],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let (event_id, event_json): (i64, String) = c.query_row("SELECT id,data FROM events WHERE job_id=?1 AND type='sdk_raw_result' ORDER BY id DESC LIMIT 1", [&job], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    for fault in [
        "reason",
        "unknown",
        "callback",
        "raw-file",
        "raw-event",
        "authorization",
        "source",
        "asset",
        "snapshot",
    ] {
        let mut supplied = raw.clone();
        match fault {
            "reason" => {
                c.execute(
                    "UPDATE jobs SET error='OTHER_REJECTION' WHERE id=?1",
                    [&job],
                )
                .unwrap();
            }
            "unknown" => {
                c.execute(
                    "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                    [&job],
                )
                .unwrap();
            }
            "callback" => supplied["revisions"][0]["brief"] = json!("altered callback"),
            "raw-file" => std::fs::write(directory.join("result.json"), "{}").unwrap(),
            "raw-event" => {
                c.execute("UPDATE events SET data='{}' WHERE id=?1", [event_id])
                    .unwrap();
            }
            "authorization" => {
                c.execute("UPDATE events SET data='{}' WHERE id=?1", [auth_id])
                    .unwrap();
            }
            "source" => {
                let mut changed = source.clone();
                changed["sha256"] = json!("0".repeat(64));
                c.execute(
                    "UPDATE sources SET data=?1 WHERE rowid=?2",
                    params![changed.to_string(), source_row],
                )
                .unwrap();
            }
            "asset" => {
                c.execute(
                    "UPDATE assets SET version=version+1 WHERE id=?1",
                    [&asset_id],
                )
                .unwrap();
            }
            _ => {
                let mut changed = snapshot.clone();
                changed["ruleSnapshot"] = json!({"altered":true});
                c.execute(
                    "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                    params![changed.to_string(), job],
                )
                .unwrap();
            }
        }
        let before_jobs = executor.store.jobs().unwrap();
        let before_events = events(&c, &job).unwrap();
        let current = executor.sdk_job(&job).unwrap();
        assert!(
            !matches!(
                executor
                    .sdk_supplement_replay_commit(request, &snapshot, &current, &supplied, None),
                Ok(true)
            ),
            "{fault}"
        );
        assert_eq!(executor.store.jobs().unwrap(), before_jobs, "{fault}");
        assert_eq!(events(&c, &job).unwrap(), before_events, "{fault}");
        match fault {
            "reason" => {
                c.execute(
                    "UPDATE jobs SET error=?1 WHERE id=?2",
                    params![s(&rejected, "error"), job],
                )
                .unwrap();
            }
            "unknown" => {
                c.execute("UPDATE jobs SET state='rejected' WHERE id=?1", [&job])
                    .unwrap();
            }
            "raw-file" => std::fs::write(directory.join("result.json"), &raw_bytes).unwrap(),
            "raw-event" => {
                c.execute(
                    "UPDATE events SET data=?1 WHERE id=?2",
                    params![event_json, event_id],
                )
                .unwrap();
            }
            "authorization" => {
                c.execute(
                    "UPDATE events SET data=?1 WHERE id=?2",
                    params![auth_json, auth_id],
                )
                .unwrap();
            }
            "source" => {
                c.execute(
                    "UPDATE sources SET data=?1 WHERE rowid=?2",
                    params![source_json, source_row],
                )
                .unwrap();
            }
            "asset" => {
                c.execute(
                    "UPDATE assets SET version=?1 WHERE id=?2",
                    params![asset_version, asset_id],
                )
                .unwrap();
            }
            "snapshot" => {
                c.execute(
                    "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                    params![snapshot.to_string(), job],
                )
                .unwrap();
            }
            _ => (),
        }
        assert_eq!(executor.sdk_job(&job).unwrap(), rejected, "restore {fault}");
    }
    drop(c);
    assert_eq!(executor.execute(request.clone()).await.unwrap(), *raw);
    assert_eq!(
        executor.sdk_job(&job).unwrap(),
        rejected,
        "reading the raw never promotes it"
    );
    executor.accept(request, raw).await.unwrap();
    let completed = executor.sdk_job(&job).unwrap();
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["result"], rejected["result"]);
    assert_eq!(completed["snapshot"], rejected["snapshot"]);
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    assert_eq!(reopened.execute(request.clone()).await.unwrap(), *raw);
    reopened.accept(request, raw).await.unwrap();
    let after_events = events(&reopened.store.conn().unwrap(), &job).unwrap();
    assert_eq!(
        &after_events.as_array().unwrap()[..prior_events.as_array().unwrap().len()],
        prior_events.as_array().unwrap()
    );
    let audits: Vec<_> = after_events
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "sdk_series_supplement_revalidated")
        .collect();
    assert_eq!(audits.len(), 1);
    assert_eq!(
        audits[0]["data"]["version"],
        "sdk-series-correction-source-fact-contract-revalidation/v1"
    );
    assert_eq!(audits[0]["data"]["originalJob"]["error"], rejected["error"]);
    assert_eq!(audits[0]["data"]["generationRequestsSent"], 0);
    assert_eq!(
        audits[0]["data"]["origin"]["correctionSourceFactContract"]
            ["independentReviewStillRequired"],
        true
    );
    assert_eq!(files(), original_files);
    assert_eq!(
        reopened.store.values("assets").unwrap(),
        assets,
        "candidate migration must not adopt designs"
    );
    assert_eq!(reopened.store.source().unwrap(), source);
    assert_eq!(reopened.store.jobs().unwrap().len(), job_count);
    let conn = reopened.store.conn().unwrap();
    for original in old {
        let id = s(&original["record"], "jobId");
        assert_eq!(job_record(&conn, id).unwrap(), original["record"]);
        assert_eq!(events(&conn, id).unwrap(), original["events"]);
        assert_eq!(
            step_record(&conn, reopened.workflow_id.as_deref().unwrap(), id).unwrap(),
            original["step"]
        );
    }
    let (review, _) = calls
        .iter()
        .find(|(r, _)| {
            r.input["seriesSupplyCorrection"]["phase"] == "design-review"
                && r.input["seriesSupplyCorrection"]["attempt"] == 0
        })
        .unwrap();
    assert_eq!(review.input["candidateSha256"], hash(raw));
    assert_eq!(
        reopened.sdk_schema_continuation_job(review).unwrap(),
        Some(job.clone())
    );
    let anchor = reopened.sdk_schema_anchor_for(&job, Some(review)).unwrap();
    let review_snapshot =
        continuation_snapshot(&reopened, review, &snapshot["executorIdentity"], &anchor);
    reopened
        .sdk_schema_continuation_bound(&review_snapshot)
        .unwrap();
    assert_eq!(
        reopened.store.jobs().unwrap().len(),
        job_count,
        "proving the independent review edge is not a new provider request"
    );
}
#[tokio::test]
async fn sdk_supply_correction_full_chain_replays_and_adopts_once_without_rewriting_evidence() {
    let (_temp, executor, _auth, context, calls, mut delivery, old) = authorized_fixture().await;
    let mut saved = Vec::new();
    for (request, raw) in calls.iter().filter(|(r, _)| r.stage != "seriesAuthor") {
        let record = save_role(&executor, request, raw);
        if request.stage == "seriesFinisher" {
            reject_synthetic(
                &executor,
                request,
                &record.1,
                &record.0,
                "SERIES_DIALOGUE_INTERVAL_OUTSIDE_SOURCE",
            );
            let rejected = executor.sdk_job(&record.0).unwrap();
            let repair = domain_repair(request, request, &rejected, 1);
            let repaired_record = save_role(&executor, &repair, raw);
            let original_sha = request_hash(request).unwrap();
            let receipt = delivery["adoptionReceipt"]["roleReceipts"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|r| r["requestSha256"] == original_sha)
                .unwrap();
            *receipt = role_receipt(&repair, raw).unwrap();
            assert_eq!(
                executor.sdk_job(&record.0).unwrap(),
                rejected,
                "domain repair keeps the original finisher raw and rejection"
            );
            saved.push((repair, raw.clone(), repaired_record));
        } else {
            saved.push((request.clone(), raw.clone(), record));
        }
    }
    assert_eq!(
        event_count(&executor),
        6,
        "one authorization and five role slots; final review keeps its own contract"
    );
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    let before = reopened.store.jobs().unwrap();
    for (request, raw, (_job, snapshot)) in &saved {
        eprintln!(
            "[sdk-correction-replay] role={} start",
            s(&request.input, "executionRoleId")
        );
        assert_eq!(reopened.sdk_execute(request, snapshot).await.unwrap(), *raw);
        eprintln!(
            "[sdk-correction-replay] role={} complete",
            s(&request.input, "executionRoleId")
        );
    }
    assert_eq!(reopened.store.jobs().unwrap(), before);
    let run = reopened.workflow_id.as_deref().unwrap();
    let input = reopened.store.job(run).unwrap()["snapshot"]["input"].clone();
    let asset_before = reopened.store.get("assets", "s").unwrap();
    let mut c = reopened.store.conn().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    eprintln!("[sdk-correction-adoption] verify-receipts-start");
    crate::workflow::verify_series_receipts(
        &tx,
        run,
        &json!({"seriesDeliveries":[delivery.clone()]}),
    )
    .unwrap();
    eprintln!("[sdk-correction-adoption] persist-start");
    crate::series_store::persist(
        &tx,
        &input,
        &json!({"seriesDeliveries":[delivery.clone()]}),
        run,
    )
    .unwrap();
    tx.commit().unwrap();
    let adopted = reopened.store.get("assets", "s").unwrap();
    assert_eq!(adopted["version"], 2);
    assert_eq!(adopted["designRevision"], 2);
    assert_ne!(adopted["adoptedDesign"], asset_before["adoptedDesign"]);
    for field in [
        "id",
        "variantId",
        "filename",
        "importName",
        "placeholderOrdinal",
        "imagePrompt",
        "history",
    ] {
        assert_eq!(adopted[field], asset_before[field], "{field}");
    }
    let mut c = reopened.store.conn().unwrap();
    let tx = c.transaction().unwrap();
    crate::series_store::persist(&tx, &input, &json!({"seriesDeliveries":[delivery]}), run)
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(reopened.store.get("assets", "s").unwrap(), adopted);
    for original in old {
        let id = s(&original["record"], "jobId");
        assert_eq!(job_record(&c, id).unwrap(), original["record"]);
        assert_eq!(events(&c, id).unwrap(), original["events"]);
    }
    let origin: TextRequest =
        serde_json::from_value(context["legacyReceipts"][0]["request"].clone()).unwrap();
    // The real author is recovered by its immutable identity; successful adoption
    // must not invalidate historical correction evidence merely by version+1.
    let origin_job = reopened
        .supply_correction_origin(s(&context, "originRequestSha256"))
        .unwrap();
    let author: TextRequest = serde_json::from_value(origin_job["snapshot"].clone()).unwrap();
    assert!(reopened
        .saved_series_supply_correction_context(&author, &origin_job["result"])
        .unwrap()
        .is_some());
    assert_eq!(origin.input["seriesAssetSupplement"]["maximumAttempts"], 2);
}
#[tokio::test]
async fn sdk_supply_correction_unknown_review_or_changed_asset_cannot_mint_a_role() {
    let (_temp, executor, _auth, _context, calls, _delivery, _old) = authorized_fixture().await;
    let roles: Vec<_> = calls
        .iter()
        .filter(|(r, _)| r.input.get("seriesSupplyCorrection").is_some())
        .collect();
    save_role(&executor, &roles[0].0, &roles[0].1);
    let review = &roles[1].0;
    let predecessor = executor
        .sdk_schema_continuation_job(review)
        .unwrap()
        .unwrap();
    let anchor = executor
        .sdk_schema_anchor_for(&predecessor, Some(review))
        .unwrap();
    let identity = executor.sdk_job(&predecessor).unwrap()["snapshot"]["executorIdentity"].clone();
    let snapshot = continuation_snapshot(&executor, review, &identity, &anchor);
    let job = executor
        .sdk_allocate_pre_dispatch(review, &snapshot)
        .unwrap();
    executor
        .store
        .update_job(
            &job,
            "submission_unknown",
            &json!({"prepared":{"sdkDispatchStarted":true}}),
        )
        .unwrap();
    let before = executor.store.jobs().unwrap();
    assert!(executor.sdk_schema_continuation_job(&roles[2].0).is_err());
    assert!(executor
        .sdk_allocate_pre_dispatch(review, &snapshot)
        .is_err());
    assert_eq!(executor.store.jobs().unwrap(), before);
    assert_eq!(event_count(&executor), 3);

    let (_temp, executor, _auth, _context, calls, _delivery, _old) = authorized_fixture().await;
    let first = calls
        .iter()
        .find(|(r, _)| r.input["seriesSupplyCorrection"]["phase"] == "design")
        .unwrap();
    let predecessor = executor
        .sdk_schema_continuation_job(&first.0)
        .unwrap()
        .unwrap();
    let anchor = executor
        .sdk_schema_anchor_for(&predecessor, Some(&first.0))
        .unwrap();
    let identity = executor.sdk_job(&predecessor).unwrap()["snapshot"]["executorIdentity"].clone();
    let snapshot = continuation_snapshot(&executor, &first.0, &identity, &anchor);
    let mut asset = executor.store.get("assets", "s").unwrap();
    asset["imagePrompt"] = json!("manual change retained");
    executor.store.save_asset(asset).unwrap();
    let before = executor.store.jobs().unwrap();
    assert!(executor
        .sdk_allocate_pre_dispatch(&first.0, &snapshot)
        .is_err());
    assert_eq!(executor.store.jobs().unwrap(), before);
    assert_eq!(event_count(&executor), 1);
}

#[tokio::test]
async fn sdk_series_supplement_v3_revises_same_identity_and_reopens_without_correction_authorization(
) {
    normal_sdk_revision_case(false).await;
}
#[tokio::test]
async fn sdk_series_supplement_v3_unpublished_design_predecessor_is_proven_before_single_cas_adoption(
) {
    normal_sdk_revision_case(true).await;
}
async fn normal_sdk_revision_case(unpublished: bool) {
    let (input, calls, delivery) = if unpublished {
        crate::pipeline::series::series_unpublished_revision_fixture().await
    } else {
        crate::pipeline::series::series_normal_revision_fixture().await
    };
    let (origin, raw) = calls
        .iter()
        .find(|(r, _)| r.stage == "seriesAuthor")
        .unwrap();
    let design = calls
        .iter()
        .find(|(r, _)| r.input["seriesAssetSupplement"]["phase"] == "design")
        .unwrap();
    let known = binding(&design.0, "knownAssetCatalog");
    let mut assets = input["assets"].as_array().unwrap().clone();
    for asset in &mut assets {
        let frozen = known
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == asset["id"])
            .unwrap();
        for field in ["importName", "filename", "placeholderOrdinal"] {
            asset[field] = frozen[field].clone();
        }
    }
    let source = input["source"].clone();
    let (_temp, executor, origin, original, next) =
        fixture_request_input(origin.clone(), Some(input));
    executor.store.save_source(source).unwrap();
    for asset in assets {
        executor.store.save_asset(asset).unwrap();
    }
    let author = executor
        .sdk_reserve_schema_successor_checked(&origin, &original, &next, || Ok(()))
        .unwrap();
    let mut saved = Vec::new();
    for (request, value) in calls.iter().take_while(|(r, _)| r.stage != "seriesAuthor") {
        let mut snapshot = executor.request_domain_snapshot(request).unwrap();
        snapshot["executorIdentity"] = executor.settings["textSdkIdentity"].clone();
        let job = executor
            .sdk_allocate_pre_dispatch(request, &snapshot)
            .unwrap();
        adopt_raw(&executor, request, &snapshot, &job, value);
        saved.push((request.clone(), value.clone(), (job, snapshot)));
    }
    adopt_raw(&executor, &origin, &next, &author, raw);
    reject_synthetic(
        &executor,
        &origin,
        &next,
        &author,
        s(
            &delivery["adoptionReceipt"]["assetSupplement"]["origin"],
            "reason",
        ),
    );
    let before_author = executor.sdk_job(&author).unwrap();
    let context = executor
        .freeze_series_supplement_identity_context(
            &origin,
            raw,
            known,
            "series-asset-supplement/v3",
        )
        .unwrap();
    assert_eq!(context, *binding(&design.0, "identityReservations"));
    assert_eq!(context["version"], "series-supplement-identities/v3");
    assert!(executor
        .saved_series_supply_correction_context(&origin, raw)
        .unwrap()
        .is_none());
    for (request, raw) in calls
        .iter()
        .skip_while(|(r, _)| r.stage != "seriesAuthor")
        .skip(1)
    {
        saved.push((
            request.clone(),
            raw.clone(),
            save_role(&executor, request, raw),
        ));
    }
    assert_eq!(
        event_count(&executor),
        0,
        "normal revise spends only its original supplement budget"
    );
    let reopened = SavedTextExecutor {
        store: Store::open(executor.store.root.clone()).unwrap(),
        settings: executor.settings.clone(),
        workflow_id: executor.workflow_id.clone(),
    };
    let before = reopened.store.jobs().unwrap();
    for (request, raw, (_, snapshot)) in &saved {
        assert_eq!(reopened.sdk_execute(request, snapshot).await.unwrap(), *raw);
    }
    assert_eq!(reopened.store.jobs().unwrap(), before);
    assert_eq!(reopened.sdk_job(&author).unwrap(), before_author);
    let run = reopened.workflow_id.as_deref().unwrap();
    let input = reopened.store.job(run).unwrap()["snapshot"]["input"].clone();
    let prior = reopened.store.get("assets", "s").unwrap();
    let mut c = reopened.store.conn().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let output = json!({"seriesDeliveries":[delivery]});
    crate::workflow::verify_series_receipts(&tx, run, &output).unwrap();
    crate::series_store::persist(&tx, &input, &output, run).unwrap();
    tx.commit().unwrap();
    let adopted = reopened.store.get("assets", "s").unwrap();
    assert_eq!(adopted["designRevision"], 2);
    assert_eq!(adopted["version"], 2);
    for field in [
        "id",
        "variantId",
        "filename",
        "importName",
        "placeholderOrdinal",
        "imagePrompt",
        "history",
    ] {
        assert_eq!(adopted[field], prior[field], "{field}");
    }
    assert_eq!(event_count(&reopened), 0);
}
