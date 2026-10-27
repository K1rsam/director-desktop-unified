//! Opt-in private, read-only replay. A missing response stops before any provider.
use super::*;
use std::sync::Mutex;

#[derive(Clone)]
struct SavedRebase {
    id: String,
    request: TextRequest,
    snapshot: Value,
    result: Value,
    state: String,
    error: Option<String>,
}

fn exact_request(a: &TextRequest, b: &TextRequest) -> bool {
    a.stage == b.stage
        && a.skill == b.skill
        && a.input == b.input
        && a.schema == b.schema
        && a.rule_snapshot == b.rule_snapshot
        && a.model == b.model
        && a.reasoning == b.reasoning
}

#[tokio::test]
#[ignore = "explicit saved first-wave database and local report directory; zero model calls"]
async fn real_saved_wave_readonly_revalidation() {
    let db = std::env::var("DIRECTOR_FOUNDATION_SAVED_WAVE_DB").expect("readonly source database");
    let workflow = std::env::var("DIRECTOR_FOUNDATION_SAVED_WAVE_RUN").expect("original workflow");
    let out = std::path::PathBuf::from(
        std::env::var("DIRECTOR_FOUNDATION_SAVED_WAVE_REPORT").expect("existing local directory"),
    )
    .canonicalize()
    .unwrap();
    let local = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.local")
        .canonicalize()
        .unwrap();
    assert!(out.starts_with(local));
    let c = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let parent: String = c
        .query_row(
            "SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow'",
            [&workflow],
            |r| r.get(0),
        )
        .unwrap();
    let parent: Value = serde_json::from_str(&parent).unwrap();
    let input = &parent["input"];
    let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
    let mut q=c.prepare("SELECT j.id,j.snapshot,CASE WHEN w.state='accepted' THEN w.result ELSE j.result END,j.state,w.state,j.error FROM workflow_steps w JOIN jobs j ON w.text_job_id=j.id WHERE w.workflow_id=?1 AND w.stage='foundation:F2' AND j.state IN ('completed','rejected') AND j.result IS NOT NULL ORDER BY w.rowid").unwrap();
    let rows: Vec<(String, String, String, String, String, Option<String>)> = q
        .query_map([&workflow], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut originals = vec![];
    let mut rebases = vec![];
    for (id, snapshot, result, state, node_state, error) in &rows {
        let request: TextRequest = serde_json::from_str(snapshot).unwrap();
        let result: Value = serde_json::from_str(result).unwrap();
        assert!(
            request.input.get("foundationConflictResolution").is_none(),
            "fixture must precede new decision dispatch"
        );
        if request.input.get("foundationParallelRebase").is_some() {
            assert!(
                state == "completed" && node_state == "accepted"
                    || state == "rejected" && node_state == "rejected",
                "old rebase receipt and node state must agree"
            );
            rebases.push(SavedRebase {
                id: id.clone(),
                request,
                snapshot: serde_json::from_str(snapshot).unwrap(),
                result,
                state: state.clone(),
                error: error.clone(),
            });
        } else {
            originals.push((request, result));
        }
    }
    assert_eq!(
        originals.len(),
        6,
        "exact captured prefix and first wave only"
    );
    let mut context = json!({"foundation":input["foundation"],"assets":input["assets"],"selection":input["selection"],"foundationModelInputProfile":input["foundationModelInputProfile"],"foundationPhase":input["foundationPhase"],"confirmedDecisions":input["confirmedDecisions"],"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundationDecisionPolicy":input["foundationDecisionPolicy"],"automaticDecisions":input["automaticDecisions"],"userReviewRequirements":input["userReviewRequirements"],"settings":{"confirmedDirection":input["foundation"]["productionDirection"]}});
    crate::pipeline::asset_coverage::inherit(&mut context, input).unwrap();
    // Continuation must consume the authorization actually frozen before the
    // first parallel wave. A fresh preview inspects today's accepted rows and
    // would incorrectly try to authorize already-parallel/rejected work again.
    // Keep this connection read-only; do not construct Store or mint a contract.
    let mut authorization_query=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type='foundation_parallel_trial_authorized' ORDER BY id").unwrap();
    let authorizations: Vec<String> = authorization_query
        .query_map([&workflow], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        authorizations.len(),
        1,
        "one original persistent trial authorization required"
    );
    let authorization: Value = serde_json::from_str(&authorizations[0]).unwrap();
    let mut unhashed = authorization.clone();
    let authorization_sha = unhashed
        .as_object_mut()
        .unwrap()
        .remove("authorizationSha256")
        .unwrap();
    assert_eq!(authorization_sha, digest(&unhashed.to_string()));
    assert_eq!(
        authorization["version"],
        crate::foundation_trial_store::PROFILE
    );
    assert_eq!(authorization["workflowId"], workflow);
    assert_eq!(
        authorization["workflowSnapshotSha256"],
        digest(&parent.to_string())
    );
    assert_eq!(authorization["sourceSha256"], input["source"]["sha256"]);
    assert_eq!(authorization["sourceVersion"], input["source"]["version"]);
    assert_eq!(authorization["concurrency"], 3);
    assert_eq!(authorization["seedBatchCount"], 1);
    let prefix = authorization["acceptedPrefix"].as_array().unwrap();
    assert_eq!(
        prefix.len(),
        3,
        "captured original serial prefix has three nodes"
    );
    assert_eq!(authorization["replayBatchCount"], prefix.len());
    assert_eq!(authorization["historyEpisodeIds"], prefix[0]["scope"]);
    let covered: Vec<Value> = prefix
        .iter()
        .flat_map(|p| p["scope"].as_array().unwrap().iter().cloned())
        .collect();
    let selected = input["selection"]["episodeIds"].as_array().unwrap();
    assert!(covered.len() < selected.len());
    assert_eq!(&selected[..covered.len()], covered.as_slice());
    assert_eq!(
        authorization["remainingEpisodeIds"],
        json!(&selected[covered.len()..])
    );
    for (index, p) in prefix.iter().enumerate() {
        let (state,request_hash,job,result,snapshot,job_state):(String,String,String,String,String,String)=c.query_row(
            "SELECT w.state,w.request_hash,w.text_job_id,w.result,j.snapshot,j.state FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
            rusqlite::params![workflow,p["nodeKey"].as_str().unwrap()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
        let snapshot: Value = serde_json::from_str(&snapshot).unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(state, "accepted");
        assert_eq!(job_state, "completed");
        assert_eq!(p["batchIndex"], index);
        assert_eq!(p["sourceSha256"], input["source"]["sha256"]);
        assert_eq!(p["requestHash"], request_hash);
        assert_eq!(p["textJobId"], job);
        assert_eq!(p["inputSha256"], digest(&snapshot["input"].to_string()));
        assert_eq!(p["resultSha256"], digest(&result.to_string()));
        assert_eq!(snapshot["input"], originals[index].0.input);
        assert_eq!(result, originals[index].1);
        assert!(snapshot["input"].get("foundationParallelTrial").is_none());
    }
    for (request, _) in &originals[prefix.len()..] {
        let marker = &request.input["foundationParallelTrial"];
        for (key, value) in authorization.as_object().unwrap() {
            assert_eq!(
                marker.get(key),
                Some(value),
                "saved parallel node must carry the original {key}"
            );
        }
    }
    context["foundationParallelTrial"] = authorization;
    struct Replay {
        originals: Vec<(TextRequest, Value)>,
        rebases: Vec<SavedRebase>,
        seen: Mutex<Vec<usize>>,
        legacy_seen: Mutex<Vec<String>>,
        conflict_contexts: Mutex<Vec<Value>>,
        next: Mutex<Vec<TextRequest>>,
        workflow: String,
    }
    #[async_trait::async_trait]
    impl TextExecutor for Replay {
        async fn execute(&self, r: TextRequest) -> Result<Value, String> {
            let matching = self
                .originals
                .iter()
                .position(|(old, _)| exact_request(old, &r));
            if let Some(at) = matching {
                self.seen.lock().unwrap().push(at);
                return Ok(self.originals[at].1.clone());
            }
            if let Some(old) = self
                .rebases
                .iter()
                .find(|old| exact_request(&old.request, &r))
            {
                assert_eq!(
                    old.state, "completed",
                    "rejected legacy repair must never execute again"
                );
                self.legacy_seen.lock().unwrap().push(old.id.clone());
                return Ok(old.result.clone());
            }
            self.next.lock().unwrap().push(r);
            Err("SAVED_WAVE_PREVIEW_ONLY_NO_MODEL".into())
        }

        async fn foundation_conflict_context(
            &self,
            legacy: &TextRequest,
            manifest: &Value,
        ) -> Result<Value, String> {
            let matches: Vec<_> = self
                .rebases
                .iter()
                .filter(|old| exact_request(&old.request, legacy))
                .collect();
            assert!(
                matches.len() <= 1,
                "one exact frozen old repair per conflict"
            );
            for field in [
                "originalInputSha256",
                "originalResultSha256",
                "currentBasisSha256",
            ] {
                assert_eq!(
                    legacy.input["foundationParallelRebase"][field], manifest[field],
                    "{field}"
                );
            }
            assert_eq!(legacy.input["scope"], manifest["scope"]);
            assert_eq!(legacy.input["sourceSha256"], manifest["sourceSha256"]);
            assert_eq!(
                legacy.stage,
                format!("foundation:{}", manifest["stage"].as_str().unwrap())
            );
            let mut old_paths = legacy.input["foundationParallelRebase"]["conflicts"]
                .as_array()
                .unwrap()
                .clone();
            old_paths.sort_by_key(Value::to_string);
            let mut new_paths = manifest["conflicts"].as_array().unwrap().clone();
            new_paths.sort_by_key(Value::to_string);
            assert_eq!(old_paths, new_paths);
            let old = matches.first().copied();
            let accepted = old.filter(|old| old.state == "completed");
            let count = usize::from(old.is_some());
            let next = if accepted.is_some() {
                Value::Null
            } else {
                json!(count + 1)
            };
            let previous:Vec<_>=old.into_iter().map(|old|json!({"jobId":old.id,"workflowId":self.workflow,"legacy":true,"status":old.state,"requestSha256":value_sha(&old.snapshot).unwrap(),"resultSha256":value_sha(&old.result).unwrap(),"rawSha256":value_sha(&old.result).unwrap(),"error":old.error})).collect();
            let mut marker = json!({"version":"foundation-conflict-resolution/v1","attempt":next,"maximumAttempts":2,"legacyJobId":old.map(|old|old.id.clone()),"previousAttemptSha256":old.map(|old|value_sha(&old.snapshot).unwrap()),"legacyConsumedAttempts":count});
            for field in [
                "sourceSha256",
                "originalInputSha256",
                "originalResultSha256",
                "currentBasisSha256",
                "issueSetSha256",
                "causeSha256",
                "manifestSha256",
            ] {
                marker[field] = manifest[field].clone();
            }
            let recovery = json!({"legacyAccepted":accepted.map(|old|json!({"raw":old.result,"request":old.request,"jobId":old.id})),"existingResolution":null,"consumedAttempts":count,"nextAttempt":next,"previousAttempts":previous,"marker":marker});
            self.conflict_contexts
                .lock()
                .unwrap()
                .push(json!({"legacyRequest":legacy,"manifest":manifest,"recovery":recovery}));
            Ok(recovery)
        }
    }
    let model = originals[0].0.model.clone();
    let reasoning = originals[0].0.reasoning.clone();
    let replay = Replay {
        originals,
        rebases,
        seen: Mutex::new(vec![]),
        legacy_seen: Mutex::new(vec![]),
        conflict_contexts: Mutex::new(vec![]),
        next: Mutex::new(vec![]),
        workflow: workflow.clone(),
    };
    let result = run(&source, &context, &replay, &model, &reasoning).await;
    let stop = result
        .err()
        .unwrap_or_else(|| "UNEXPECTED_COMPLETION".into());
    let seen = replay.seen.lock().unwrap().clone();
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    let requests = replay.next.lock().unwrap();
    let legacy_seen = replay.legacy_seen.lock().unwrap().clone();
    let contexts = replay.conflict_contexts.lock().unwrap();
    if let Some(last) = contexts.last() {
        std::fs::write(
            out.join("saved-wave-conflict-context.json"),
            serde_json::to_vec_pretty(last).unwrap(),
        )
        .unwrap();
    }
    // Verify every saved row again on this same READ_ONLY connection, including
    // rejected data, rather than mutating states to simulate a recovered node.
    let after: Vec<(String, String, String, String, String, Option<String>)> = q
        .query_map([&workflow], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows, after,
        "all captured states, snapshots and raw results stay unchanged"
    );
    let report = json!({"kind":"READ_ONLY_SAVED_WAVE_REVALIDATION_NOT_CLIENT_ACCEPTANCE","readOnly":true,"modelCalls":0,"replayedNodeIndexes":seen,"legacyAcceptedReplayed":legacy_seen,"legacyRejectedPreserved":replay.rebases.iter().filter(|r|r.state=="rejected").map(|r|r.id.clone()).collect::<Vec<_>>(),"savedRowsUnchanged":true,"nextRequests":*requests,"stop":stop});
    std::fs::write(
        out.join("saved-wave-revalidation.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert_eq!(
        unique,
        vec![0, 1, 2, 3, 4, 5],
        "all exact frozen nodes must be replayed; {stop}"
    );
    assert!(
        stop.contains("SAVED_WAVE_PREVIEW_ONLY_NO_MODEL"),
        "unexpected domain failure: {stop}"
    );
    assert_eq!(
        requests.len(),
        1,
        "stop at first genuinely new decision without invoking a provider"
    );
    let next = &requests[0];
    assert!(next.input.get("foundationConflictResolution").is_some());
    assert!(next.input.get("foundationParallelRebase").is_none());
    let last = contexts
        .last()
        .expect("new decision has a frozen conflict context");
    let legacy: TextRequest = serde_json::from_value(last["legacyRequest"].clone()).unwrap();
    assert_eq!(
        next.input["source"], legacy.input["source"],
        "full current batch source unchanged"
    );
    assert_eq!(next.input["scope"], legacy.input["scope"]);
    assert_eq!(
        next.rule_snapshot, legacy.rule_snapshot,
        "complete rule package retained"
    );
    assert_eq!(next.input["foundationConflictManifest"], last["manifest"]);
    let rejected: Vec<_> = replay
        .rebases
        .iter()
        .filter(|r| r.state == "rejected")
        .collect();
    if !rejected.is_empty() {
        assert_eq!(
            rejected.len(),
            1,
            "fixture contains one consumed rejected repair"
        );
        assert!(exact_request(&rejected[0].request, &legacy));
        assert_eq!(last["recovery"]["consumedAttempts"], 1);
        assert_eq!(last["recovery"]["nextAttempt"], 2);
        assert_eq!(next.input["foundationConflictResolution"]["attempt"], 2);
        let accepted: Vec<_> = replay
            .rebases
            .iter()
            .filter(|r| r.state == "completed")
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(
            legacy_seen, accepted,
            "all earlier accepted repairs replayed exactly"
        );
        assert!(!legacy_seen.contains(&rejected[0].id));
    } else if replay.rebases.is_empty() {
        assert_eq!(
            next.input["foundationConflictResolution"]["attempt"], 1,
            "six-node legacy fixture starts first sparse decision"
        );
        assert!(legacy_seen.is_empty());
    }
}
