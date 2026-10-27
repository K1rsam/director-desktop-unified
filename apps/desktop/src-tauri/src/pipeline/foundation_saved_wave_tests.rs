//! Opt-in private, read-only replay. A missing response stops before any provider.
use super::*;
use std::sync::Mutex;

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
    let mut q=c.prepare("SELECT j.snapshot,CASE WHEN w.state='accepted' THEN w.result ELSE j.result END FROM workflow_steps w JOIN jobs j ON w.text_job_id=j.id WHERE w.workflow_id=?1 AND w.stage='foundation:F2' AND j.state IN ('completed','rejected') AND j.result IS NOT NULL ORDER BY w.rowid").unwrap();
    let originals: Vec<(TextRequest, Value)> = q
        .query_map([&workflow], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|r| {
            let (a, b) = r.unwrap();
            (
                serde_json::from_str(&a).unwrap(),
                serde_json::from_str(&b).unwrap(),
            )
        })
        .collect();
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
        seen: Mutex<Vec<usize>>,
        next: Mutex<Vec<TextRequest>>,
    }
    #[async_trait::async_trait]
    impl TextExecutor for Replay {
        async fn execute(&self, r: TextRequest) -> Result<Value, String> {
            let matching = self.originals.iter().position(|(old, _)| {
                old.stage == r.stage
                    && old.input == r.input
                    && old.schema == r.schema
                    && old.rule_snapshot == r.rule_snapshot
                    && old.model == r.model
                    && old.reasoning == r.reasoning
            });
            if let Some(at) = matching {
                self.seen.lock().unwrap().push(at);
                return Ok(self.originals[at].1.clone());
            }
            self.next.lock().unwrap().push(r);
            Err("SAVED_WAVE_PREVIEW_ONLY_NO_MODEL".into())
        }
    }
    let model = originals[0].0.model.clone();
    let reasoning = originals[0].0.reasoning.clone();
    let replay = Replay {
        originals,
        seen: Mutex::new(vec![]),
        next: Mutex::new(vec![]),
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
    let report = json!({"kind":"READ_ONLY_SAVED_WAVE_REVALIDATION_NOT_CLIENT_ACCEPTANCE","readOnly":true,"modelCalls":0,"replayedNodeIndexes":seen,"nextRequests":*requests,"stop":stop});
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
    assert!(!requests.is_empty());
    assert!(requests
        .iter()
        .all(|r| r.input.get("foundationParallelRebase").is_some()
            || r.input["batchIndex"].as_u64().is_some_and(|i| i >= 6)));
}
