//! Explicit, private, read-only replay of the CURRENT saved F2 prefix.
//! Register under text_executor; never invoke SavedTextExecutor::execute/accept,
//! SDK transport, passive reconcile, or any mutation. Every cache row uses the
//! native raw/receipt verifier; conflict history uses the native read-only gate.
use super::*;
use crate::pipeline::LockedSource;
use std::collections::BTreeSet;

const STOP: &str = "CURRENT_F2_READONLY_NEXT_REQUEST_NO_DISPATCH";

#[derive(Clone)]
struct SavedNode {
    id: String,
    key: String,
    snapshot: Value,
    request: TextRequest,
    raw: Value,
    adopted: Value,
    state: String,
    job_state: String,
}

fn identical(a: &TextRequest, b: &TextRequest) -> bool {
    serde_json::to_value(a).unwrap() == serde_json::to_value(b).unwrap()
}

fn rows_fingerprint(c: &rusqlite::Connection) -> String {
    let mut records = Vec::new();
    // These are all durable scheduling/result/receipt objects; do not restrict
    // to accepted rows and accidentally miss new allocations or rejected edits.
    for table in ["jobs", "workflow_steps", "events"] {
        let mut q = c
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = q.column_count();
        let rows = q
            .query_map([], |r| {
                let mut values = Vec::new();
                for col in 0..columns {
                    use rusqlite::types::ValueRef;
                    values.push(match r.get_ref(col)? {
                        ValueRef::Null => "N".to_owned(),
                        ValueRef::Integer(v) => format!("I{v}"),
                        ValueRef::Real(v) => format!("R{:016x}", v.to_bits()),
                        ValueRef::Text(v) => format!("T{}", crate::series_lite_edit::sha256(v)),
                        ValueRef::Blob(v) => format!("B{}", crate::series_lite_edit::sha256(v)),
                    });
                }
                Ok(values)
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        records.push((table, rows));
    }
    crate::pipeline::digest(&serde_json::to_string(&records).unwrap())
}

fn load_nodes(saved: &SavedTextExecutor, run: &str) -> Vec<SavedNode> {
    let c = saved.store.conn().unwrap();
    let mut q = c.prepare("SELECT w.node_key,w.request_hash,w.text_job_id,w.state,w.result,j.snapshot,j.state,j.provider FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 ORDER BY w.rowid").unwrap();
    let rows = q
        .query_map([run], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    rows.into_iter()
        .map(
            |(key, hash, id, state, adopted, snapshot_text, job_state, provider)| {
                assert_eq!(
                    provider, "codex_sdk",
                    "current replay supports verified SDK evidence only"
                );
                let snapshot: Value = serde_json::from_str(&snapshot_text).unwrap();
                assert_eq!(hash, crate::pipeline::digest(&snapshot_text));
                assert_eq!(
                    snapshot_text,
                    snapshot.to_string(),
                    "exact canonical frozen snapshot"
                );
                let request: TextRequest = serde_json::from_value(snapshot.clone()).unwrap();
                assert_eq!(key, step_key(&request));
                assert_eq!(
                    request.stage, "foundation:F2",
                    "stop before Q1; fixture has new scope"
                );
                assert!(
                    matches!(
                        (state.as_str(), job_state.as_str()),
                        ("accepted", "completed") | ("rejected", "rejected")
                    ),
                    "unresolved node {id}: {state}/{job_state}"
                );
                saved.sdk_request_policy(&request).unwrap();
                saved.sdk_checkpoint_binding(&request, &snapshot).unwrap();
                let job = saved.sdk_job(&id).unwrap();
                assert_eq!(job["snapshot"], snapshot);
                // Native full raw validation includes frozen attempt, original compiled
                // files, handoff/worker identity, raw text file and bound receive event.
            let raw = saved.sdk_raw(&job, &snapshot).unwrap_or_else(|error| {
                let marker=&request.input["foundationParallelTrial"];
                let expected_worker=crate::text_sdk::worker_root(&saved.store.root,&request);
                let expected_scope=expected_worker.as_ref().ok().map(|path|json!({
                    "version":"foundation-sdk-lane-worker/v1",
                    "authorizationSha256":marker["authorizationSha256"],
                    "laneId":marker["laneId"],"workerRoot":path}));
                let diagnosis=json!({"jobId":id,"nodeKey":key,"jobState":job_state,"checkpointState":state,
                    "stage":request.stage,"batchIndex":request.input["batchIndex"],
                    "sourceSha256":request.input["sourceSha256"],
                    "requestSnapshotSha256":hash,"sdkDispatchStarted":job["prepared"]["sdkDispatchStarted"],
                    "rootUsed":saved.store.root,"expectedWorkerScope":expected_scope,
                    "savedWorkerScope":job["prepared"]["sdkWorkerScope"],
                    "markerValidationError":foundation_parallel::validate_markers(&request).err(),
                    "directWorkerBindingError":saved.sdk_parallel_worker_bound(&job,&snapshot).err(),
                    "rawValidationError":error});
                eprintln!("CURRENT_F2_READONLY_RAW_BINDING_REJECTED {diagnosis}");
                panic!("read-only original evidence rejected for job {id}; see binding diagnostics");
            });
                let adopted: Value =
                    serde_json::from_str(&adopted.expect("terminal node result")).unwrap();
                if state == "accepted" {
                    assert_eq!(job["prepared"]["sdkDomainResult"], adopted);
                    assert_eq!(
                        job["prepared"]["sdkDomainResultSha256"],
                        crate::pipeline::digest(&adopted.to_string())
                    );
                } else {
                    assert_eq!(raw, adopted, "rejected original raw must remain exact");
                }
                SavedNode {
                    id,
                    key,
                    snapshot,
                    request,
                    raw,
                    adopted,
                    state,
                    job_state,
                }
            },
        )
        .collect()
}

struct ReadOnlyReplay {
    saved: SavedTextExecutor,
    nodes: Vec<SavedNode>,
    replayed: Mutex<Vec<String>>,
    accepted: Mutex<Vec<String>>,
    next: Mutex<Vec<Value>>,
    conflicts: Mutex<Vec<Value>>,
    mismatches: Mutex<Vec<Value>>,
}

impl ReadOnlyReplay {
    fn exact(&self, request: &TextRequest) -> Result<Option<&SavedNode>> {
        let matches: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| identical(&n.request, request))
            .collect();
        if matches.len() > 1 {
            return Err("READONLY_DUPLICATE_SAVED_REQUEST".into());
        }
        Ok(matches.into_iter().next())
    }
    fn capture(&self, request: &TextRequest) -> Result<()> {
        // A same-key mismatch is not a new request and must never be presented
        // as successful capacity recovery. Record coordinates, not raw text.
        if let Some(old) = self.nodes.iter().find(|n| n.key == step_key(request)) {
            self.mismatches.lock().unwrap().push(json!({"jobId":old.id,"nodeKey":old.key,"oldDomainSha256":crate::pipeline::digest(&serde_json::to_value(&old.request).unwrap().to_string()),"newDomainSha256":crate::pipeline::digest(&serde_json::to_value(request).unwrap().to_string())}));
            return Err("READONLY_FROZEN_DOMAIN_CHANGED".into());
        }
        let compiled = compile_model_transport(request)?;
        let (input, schema) = compiled
            .as_ref()
            .map(|w| (&w.input, &w.schema))
            .unwrap_or((&request.input, &request.schema));
        let mut budget_request = request.clone();
        budget_request.model = "m".repeat(128);
        budget_request.reasoning = "ultra".into();
        let conservative = transport_payload_characters(&budget_request, input, schema);
        let mut next = self.next.lock().unwrap();
        if !next
            .iter()
            .any(|v| v["request"] == serde_json::to_value(request).unwrap())
        {
            next.push(json!({"request":request,"requestDomainSha256":crate::pipeline::digest(&serde_json::to_value(request).unwrap().to_string()),
                "canonicalTransportCharacters":codex_wire_characters(request,&request.input,&request.schema),
                "actualCodexTransportCharacters":codex_wire_characters(request,input,schema),
                "conservativePlannerCharacters":conservative,"modelTransportLimit":crate::pipeline::foundation_delivery::MODEL_TRANSPORT_LIMIT,
                "completeRegistryPresent":request.input.get("registry").is_some(),"catalogueWireProfile":request.input["foundationCatalogueWire"],
                "submitted":false}));
        }
        Err(STOP.into())
    }
}

#[async_trait::async_trait]
impl TextExecutor for ReadOnlyReplay {
    async fn prepare_transport_request(&self, request: TextRequest) -> Result<TextRequest> {
        // Native preparation is read-only; the thread-bound Store guard denies
        // writes even if a future implementation accidentally adds one.
        self.saved.prepare_transport_request(request).await
    }
    fn input_transport_characters(&self, request: &TextRequest) -> Result<usize> {
        self.saved.input_transport_characters(request)
    }
    async fn execute(&self, request: TextRequest) -> Result<Value> {
        let Some(old) = self.exact(&request)? else {
            self.capture(&request)?;
            unreachable!()
        };
        if old.state != "accepted" || old.job_state != "completed" {
            return Err("READONLY_REJECTED_NODE_MUST_NOT_REEXECUTE".into());
        }
        self.replayed.lock().unwrap().push(old.id.clone());
        Ok(old.raw.clone())
    }
    async fn accept(&self, request: &TextRequest, value: &Value) -> Result<()> {
        let old = self
            .exact(request)?
            .ok_or("READONLY_CANNOT_ADOPT_NEW_RESULT")?;
        if old.state != "accepted" || &old.adopted != value {
            return Err(format!(
                "READONLY_ADOPTED_RESULT_REVALIDATION_CHANGED:{}",
                old.id
            ));
        }
        self.accepted.lock().unwrap().push(old.id.clone());
        Ok(())
    }
    async fn reject(&self, request: &TextRequest, _value: &Value, reason: &str) -> Result<()> {
        self.mismatches
            .lock()
            .unwrap()
            .push(json!({"nodeKey":step_key(request),"validationError":reason}));
        Err(format!("READONLY_REVALIDATION_REJECTED:{reason}"))
    }
    async fn foundation_conflict_context(
        &self,
        legacy: &TextRequest,
        proposal: &Value,
    ) -> Result<Value> {
        // This function validates the REAL current cause ledger; no reconstructed
        // or simulated attempt counts, markers or accepted decision responses.
        let context = self
            .saved
            .foundation_conflict_context_saved(legacy, proposal)?;
        self.conflicts.lock().unwrap().push(json!({"manifestSha256":proposal["manifestSha256"],"causeSha256":proposal["causeSha256"],"scope":proposal["scope"],"recovery":context}));
        Ok(context)
    }
    async fn foundation_parallel_receipt_time(&self, input: &Value) -> Result<String> {
        crate::foundation_trial_store::receipt_time(
            &self.saved.store,
            self.saved.workflow_id.as_deref().unwrap(),
            input,
        )
    }
}

#[test]
#[ignore = "explicit current project/root/workflow/report; forced READ_ONLY, cached F2 evidence only, zero model calls"]
fn foundation_current_saved_f2_readonly_replay_to_next_request() {
    let required =
        |name| std::env::var(name).expect("explicit private read-only replay parameters required");
    let root = PathBuf::from(required("DIRECTOR_CURRENT_F2_REPLAY_ROOT"));
    assert!(root.is_absolute() && root.join("project.sqlite").is_file());
    let run = required("DIRECTOR_CURRENT_F2_REPLAY_WORKFLOW");
    let out = PathBuf::from(required("DIRECTOR_CURRENT_F2_REPLAY_REPORT"))
        .canonicalize()
        .unwrap();
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.local")
        .canonicalize()
        .unwrap();
    assert!(out.starts_with(local) && !out.starts_with(root.canonicalize().unwrap()));
    assert!(
        !out.join("current-f2-replay.json").exists(),
        "never overwrite an earlier evidence report"
    );
    let hashes = || {
        ["project.sqlite", "project.sqlite-wal"]
            .into_iter()
            .map(|name| {
                let p = root.join(name);
                (
                    name,
                    if p.exists() {
                        Some(crate::store::hash_file(&p).unwrap())
                    } else {
                        None
                    },
                )
            })
            .collect::<Vec<_>>()
    };
    let before = hashes();
    let report = Store::with_readonly_connections(&root, || {
        let store = Store { root: root.clone() };
        let before_rows = rows_fingerprint(&store.conn().unwrap());
        let parent = store.job(&run).unwrap();
        let snapshot = &parent["snapshot"];
        let input = &snapshot["input"];
        let saved = SavedTextExecutor {
            store: store.clone(),
            settings: input["settings"].clone(),
            workflow_id: Some(run.clone()),
        };
        saved.sdk_parent_binding().unwrap();
        let nodes = load_nodes(&saved, &run);
        assert!(!nodes.is_empty());
        let c = store.conn().unwrap();
        let mut q=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type='foundation_parallel_trial_authorized' ORDER BY id").unwrap();
        let authorization = q
            .query_map([&run], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            authorization.len(),
            1,
            "reuse the original persistent trial authorization only"
        );
        let authorization: Value = serde_json::from_str(&authorization[0]).unwrap();
        let mut checked_authorization = json!({});
        crate::foundation_trial_store::attach(&store, &run, snapshot, &mut checked_authorization)
            .unwrap();
        assert_eq!(
            checked_authorization["foundationParallelTrial"],
            authorization
        );
        let mut context = json!({"foundation":input["foundation"],"assets":input["assets"],"selection":input["selection"],"foundationModelInputProfile":input["foundationModelInputProfile"],"foundationPhase":input["foundationPhase"],"confirmedDecisions":input["confirmedDecisions"],"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundationDecisionPolicy":input["foundationDecisionPolicy"],"automaticDecisions":input["automaticDecisions"],"userReviewRequirements":input["userReviewRequirements"],"settings":{"confirmedDirection":input["foundation"]["productionDirection"]},"foundationParallelTrial":authorization});
        crate::pipeline::asset_coverage::inherit(&mut context, input).unwrap();
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let replay = ReadOnlyReplay {
            saved,
            nodes,
            replayed: Default::default(),
            accepted: Default::default(),
            next: Default::default(),
            conflicts: Default::default(),
            mismatches: Default::default(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(
            crate::pipeline::foundation_delivery::run_foundation_delivery(
                &source,
                &context,
                &replay,
                s(&input["settings"], "textModel"),
                s(&input["settings"], "reasoning"),
            ),
        );
        let stop = result
            .err()
            .unwrap_or_else(|| "UNEXPECTED_COMPLETION_NO_NEW_REQUEST".into());
        let after_rows = rows_fingerprint(&store.conn().unwrap());
        let replayed: BTreeSet<_> = replay.replayed.lock().unwrap().iter().cloned().collect();
        let accepted: BTreeSet<_> = replay.accepted.lock().unwrap().iter().cloned().collect();
        let expected: BTreeSet<_> = replay
            .nodes
            .iter()
            .filter(|n| n.state == "accepted")
            .map(|n| n.id.clone())
            .collect();
        let originals:Vec<_>=replay.nodes.iter().map(|n|json!({"jobId":n.id,"state":n.state,"nodeKey":n.key,"scope":n.request.input["scope"],"snapshotSha256":crate::pipeline::digest(&n.snapshot.to_string()),"rawSha256":crate::pipeline::digest(&n.raw.to_string()),"adoptedSha256":crate::pipeline::digest(&n.adopted.to_string()),"kind":if n.request.input.get("foundationConflictResolution").is_some(){"decision"}else if n.request.input.get("foundationParallelRebase").is_some(){"legacyRebase"}else{"original"}})).collect();
        let report = json!({"kind":"CURRENT_SAVED_F2_READONLY_REPLAY_NOT_CLIENT_ACCEPTANCE","workflowId":run,"modelCalls":0,"dispatchCalls":0,"storeMutations":0,
            "savedRows":originals,"expectedAccepted":expected,"replayed":replayed,"revalidatedAccepted":accepted,"allAcceptedReplayed":replayed==expected&&accepted==expected,
            "rejectedPreserved":replay.nodes.iter().filter(|n|n.state=="rejected").map(|n|n.id.clone()).collect::<Vec<_>>(),
            "conflictContexts":*replay.conflicts.lock().unwrap(),"nextRequests":*replay.next.lock().unwrap(),"mismatches":*replay.mismatches.lock().unwrap(),
            "stop":stop,"rowsBeforeSha256":before_rows,"rowsAfterSha256":after_rows,"savedRowsUnchanged":before_rows==after_rows});
        report
    });
    let after = hashes();
    let empty = |v: &Option<(String, u64)>| v.as_ref().is_none_or(|(_, bytes)| *bytes == 0);
    let empty_wal_lifecycle_changed =
        before[1] != after[1] && empty(&before[1].1) && empty(&after[1].1);
    let mut report = report;
    report["databaseBytesUnchanged"] = json!(before[0] == after[0]);
    report["walPageBytesUnchanged"] = json!(before[1] == after[1] || empty_wal_lifecycle_changed);
    report["emptyWalLifecycleChanged"] = json!(empty_wal_lifecycle_changed);
    std::fs::write(
        out.join("current-f2-replay.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert_eq!(report["savedRowsUnchanged"], true);
    assert_eq!(report["databaseBytesUnchanged"], true);
    assert_eq!(report["walPageBytesUnchanged"], true);
    assert_eq!(report["allAcceptedReplayed"], true, "{}", report["stop"]);
    assert!(report["mismatches"].as_array().unwrap().is_empty());
    assert!(s(&report, "stop").contains(STOP), "{}", report["stop"]);
    let next = report["nextRequests"].as_array().unwrap();
    assert!(
        (1..=3).contains(&next.len()),
        "one next parallel wave at most"
    );
    for request in next {
        assert_eq!(request["request"]["stage"], "foundation:F2");
        assert_eq!(request["submitted"], false);
        assert!(
            request["conservativePlannerCharacters"].as_u64().unwrap()
                <= request["modelTransportLimit"].as_u64().unwrap()
        );
    }
    eprintln!("CURRENT_F2_READONLY_REPLAY_PASS accepted={} nextRequests={} modelCalls=0 frozenSnapshotsAndRawUnchanged=true",report["expectedAccepted"].as_array().unwrap().len(),next.len());
}
