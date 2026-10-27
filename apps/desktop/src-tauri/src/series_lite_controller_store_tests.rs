use super::*;
use crate::pipeline::TextExecutor;
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_job_packet::{assemble, RejectUnverifiedWorkspace};
use crate::store::Store;
use crate::text_executor::{compile_codex_input, SavedTextExecutor};

fn setup() -> (tempfile::TempDir, Store, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("controller synthetic", "synthetic").unwrap();
    store.ensure_workflow_storage().unwrap();
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"input":{"liteControllerContract":VERSION,"source":{"sha256":"a".repeat(64)}}}),
            "running",
        )
        .unwrap();
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    install(&tx).unwrap();
    tx.commit().unwrap();
    (temp, store, run)
}
fn frozen(store: &Store, run: &str, role: Role, base: &str) -> FrozenRole {
    let c = store.conn().unwrap();
    let context = context_for(&c, run, "1", role).unwrap();
    let corpus: Value =
        serde_json::from_str(include_str!("series_lite_job_packet_golden.json")).unwrap();
    let name = format!("{}_complete", role.as_str());
    let case = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    for (path, text) in case["files"].as_object().unwrap() {
        let target = temp.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text.as_str().unwrap()).unwrap();
    }
    let pack = VerifiedPack::embedded().unwrap();
    let packet = assemble(
        temp.path(),
        "request.json",
        case["maxBytes"].as_u64().unwrap() as usize,
        &pack,
        &RejectUnverifiedWorkspace,
    )
    .unwrap();
    let carrier = transport::freeze_verified(&packet, &context, &pack).unwrap();
    let mut input = carrier.input;
    input["textExact"] = json!({"version":"series-lite-exact-text/v1","baseSha256":digest(base)});
    let request = TextRequest {
        stage: role.stage().into(),
        skill: format!("series-lite-{}", role.as_str()),
        model: "gpt-6.1-sol".into(),
        reasoning: "medium".into(),
        input,
        schema: json!({"type":"object"}),
        rule_snapshot: json!({"version":"synthetic-frozen-rule/v1","role":role}),
    };
    let previous_candidate = if role == Role::Author {
        None
    } else {
        read_role(&c, run, "1", role_at(order(role) - 1).unwrap())
            .unwrap()
            .unwrap()
            .result
            .unwrap()
            .candidate
    };
    FrozenRole {
        version: VERSION.into(),
        workflow_id: run.into(),
        source_sha256: "a".repeat(64),
        rule_snapshot_sha256: digest(&request.rule_snapshot.to_string()),
        request,
        identity: carrier.identity,
        context,
        base_utf8: base.into(),
        base_sha256: digest(base),
        common_inputs: Inputs::from([
            ("source".into(), "a".repeat(64)),
            ("supply".into(), "b".repeat(64)),
            ("visualBaseline".into(), "c".repeat(64)),
            ("rulesPack".into(), "d".repeat(64)),
        ]),
        previous_candidate,
    }
}
fn reserve_saved(store: &Store, frozen: &FrozenRole) -> RoleRecord {
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    let result = reserve(&tx, frozen).unwrap();
    tx.commit().unwrap();
    result
}
async fn receive(
    store: &Store,
    frozen: &FrozenRole,
    text: &str,
) -> (SavedTextExecutor, VerifiedRawText) {
    // Complete synthetic CLI receipt, consumed through production exact reader.
    // No executable exists and no subprocess/model can be launched by this path.
    let executor = SavedTextExecutor {
        store: store.clone(),
        settings: json!({"textProvider":"codex"}),
        workflow_id: Some(frozen.workflow_id.clone()),
    };
    let mut snapshot = serde_json::to_value(&frozen.request).unwrap();
    snapshot["executorPolicy"] = json!("codex-isolated/v1");
    snapshot["executorIdentity"] =
        json!({"provider":"codex","executable":"C:\\synthetic-never-launch.exe"});
    let hash = digest(&snapshot.to_string());
    let job = store
        .add_job(
            "text",
            "synthetic",
            "codex",
            "synthetic",
            &snapshot,
            "received",
        )
        .unwrap();
    let node = node_key(frozen);
    store
        .checkpoint(
            &frozen.workflow_id,
            &node,
            &frozen.request.stage,
            &hash,
            "running",
            Some(&job),
            None,
            None,
        )
        .unwrap();
    let compiled = compile_codex_input(&frozen.request).unwrap();
    let prompt = compiled.prompt;
    let metadata = json!({"version":"cli-dispatch-format/v1","format":"legacy-json/v1","jobId":job,"workflowId":frozen.workflow_id,"nodeKey":node,"requestHash":hash,"freshSemanticNode":true,"retryOrigin":null,"promptSha256":digest(&prompt),"schemaSha256":digest(&compiled.schema),"promptBytes":prompt.len(),"promptUtf16":prompt.encode_utf16().count()});
    let dir = store.root.join("text-runs").join(&job);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in [
        ("cli-prompt.txt", prompt),
        ("schema.json", compiled.schema),
        ("cli-dispatch.json", metadata.to_string()),
        ("result.json", text.to_string()),
    ] {
        std::fs::write(dir.join(name), text).unwrap();
    }
    store.event(&job, "cli_dispatch_frozen", &metadata).unwrap();
    let envelope = json!({"version":"model-response-utf8/v1","rawText":text,"nativeTextSha256":digest(text),"byteLength":text.len()});
    store
        .event(
            &job,
            "text_received_raw",
            &json!({"requestHash":hash,"value":envelope}),
        )
        .unwrap();
    store.event(&job,"lite_exact_received",&json!({"version":"series-lite-exact-text/v1","requestHash":hash,"nativeTextSha256":digest(text),"byteLength":text.len()})).unwrap();
    store
        .update_job(&job, "received", &json!({"result":envelope}))
        .unwrap();
    let raw = executor
        .execute_exact(frozen.request.clone(), &frozen.identity, &frozen.context)
        .await
        .unwrap();
    (executor, raw)
}
fn save_result(
    store: &Store,
    slot: &RoleRecord,
    raw: &VerifiedRawText,
    candidate: Option<&[u8]>,
) -> RoleRecord {
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    let result = record_candidate(&tx, &slot.slot_id, raw, candidate).unwrap();
    tx.commit().unwrap();
    result
}

#[test]
fn controller_reads_old_database_without_ddl_and_requires_explicit_install() {
    let c = Connection::open_in_memory().unwrap();
    assert!(read(&c, "unknown").unwrap().is_none());
    assert!(read_role(&c, "old", "1", Role::Author).unwrap().is_none());
    let n: i64 = c
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn controller_reservation_freezes_slots_and_rejects_self_reported_history_and_drift() {
    let (_temp, store, run) = setup();
    let frozen = frozen(&store, &run, Role::Author, "{}");
    let slot = reserve_saved(&store, &frozen);
    assert_eq!(slot.ordinal, 1);
    assert_eq!(reserve_saved(&store, &frozen).slot_id, slot.slot_id);
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    for field in ["base", "inputs", "request", "context"] {
        let mut changed = frozen.clone();
        match field {
            "base" => {
                changed.base_utf8 = "{\"x\":1}".into();
                changed.base_sha256 = digest(&changed.base_utf8);
                changed.request.input["textExact"]["baseSha256"] = json!(changed.base_sha256);
            }
            "inputs" => {
                changed
                    .common_inputs
                    .insert("supply".into(), "e".repeat(64));
            }
            "request" => changed.request.model = "different-model".into(),
            _ => changed.context.prior_contributors.push(Contributor {
                role: Role::Author,
                context_id: "invented".into(),
            }),
        }
        assert!(reserve(&tx, &changed).is_err(), "{field}");
    }
    assert!(context_for(&tx, &run, "1", Role::Finisher).is_err());
    assert!(context_for(&tx, &run, "1", Role::Reviewer).is_err());
    let count: i64 = tx
        .query_row("SELECT count(*) FROM series_lite_role_slots_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn controller_three_roles_persist_raw_candidate_and_planned_context_order_without_approval() {
    let (_temp, store, run) = setup();
    let mut base = "{}".to_string();
    let mut contexts = vec![];
    for role in [Role::Author, Role::Finisher, Role::Reviewer] {
        let frozen = frozen(&store, &run, role, &base);
        let slot = reserve_saved(&store, &frozen);
        assert_eq!(frozen.context.prior_contributors.len(), contexts.len());
        for (actual, previous) in frozen.context.prior_contributors.iter().zip(&contexts) {
            assert_eq!(&actual.context_id, previous);
        }
        let (executor, raw) = receive(&store, &frozen, "{\"result\":1.000000000000000001}").await;
        let candidate = if role == Role::Reviewer {
            None
        } else {
            Some(b"{\"candidate\":1.000000000000000001}".as_slice())
        };
        let saved = save_result(&store, &slot, &raw, candidate);
        assert_eq!(saved.transport_state.as_deref(), Some("received"));
        assert_eq!(
            save_result(&store, &slot, &raw, candidate).result,
            saved.result
        );
        if role != Role::Reviewer {
            assert!(context_for(
                &store.conn().unwrap(),
                &run,
                "1",
                role_at(order(role) + 1).unwrap()
            )
            .is_err());
        }
        executor
            .accept(&frozen.request, raw.transport_envelope())
            .await
            .unwrap();
        let read = read_role(&store.conn().unwrap(), &run, "1", role)
            .unwrap()
            .unwrap();
        assert_eq!(read.transport_state.as_deref(), Some("completed"));
        let result = read.result.unwrap();
        let raw_artifact = artifacts::artifact(&store.conn().unwrap(), &result.raw_artifact_id)
            .unwrap()
            .unwrap();
        assert_eq!(raw_artifact.artifact.payload["contentReview"], "NOT_PROVEN");
        assert_eq!(
            raw_artifact.artifact.payload["actualContextIndependence"],
            "NOT_VERIFIED"
        );
        assert_eq!(raw_artifact.artifact.payload["adoptionGranted"], false);
        if role == Role::Reviewer {
            assert_eq!(raw_artifact.artifact.kind, Kind::ReviewRaw);
            assert!(result.candidate.is_none());
            let previous = frozen.previous_candidate.unwrap();
            assert_eq!(
                raw_artifact.artifact.payload["candidateSha256"],
                previous.payload_sha256
            );
            assert_eq!(
                raw_artifact.artifact.payload["candidateNativeSha256"],
                previous.native_sha256
            );
        } else {
            let pin = result.candidate.unwrap();
            assert_ne!(pin.payload_sha256, pin.native_sha256);
            base = String::from_utf8(candidate.unwrap().to_vec()).unwrap();
            contexts.push(frozen.context.context_id);
        }
    }
    let c = store.conn().unwrap();
    let validations: i64 = c
        .query_row("SELECT count(*) FROM workflow_validations_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(validations, 0);
    let adoptions: i64 = c
        .query_row("SELECT count(*) FROM workflow_adoptions_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(adoptions, 0);
}

#[tokio::test]
async fn controller_unknown_reuses_original_slot_and_cannot_open_next_role() {
    let (_temp, store, run) = setup();
    let frozen = frozen(&store, &run, Role::Author, "{}");
    let slot = reserve_saved(&store, &frozen);
    let (_executor, raw) = receive(&store, &frozen, "{}").await;
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET state='submission_unknown',result=NULL WHERE id=?1",
            [raw.job_id()],
        )
        .unwrap();
    let old = context_for(&store.conn().unwrap(), &run, "1", Role::Author).unwrap();
    assert_eq!(old, frozen.context);
    assert_eq!(reserve_saved(&store, &frozen).slot_id, slot.slot_id);
    assert!(context_for(&store.conn().unwrap(), &run, "1", Role::Finisher).is_err());
    let second = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &json!({"input":{"liteControllerContract":VERSION,"source":{"sha256":"a".repeat(64)}}}),
            "running",
        )
        .unwrap();
    let replacement = self::frozen(&store, &second, Role::Author, "{}");
    let mut connection = store.conn().unwrap();
    let tx = connection.transaction().unwrap();
    assert!(reserve(&tx, &replacement).is_err());
    tx.commit().unwrap();
    let count: i64 = store
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM series_lite_role_slots_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn controller_raw_job_mismatch_and_candidate_overwrite_are_atomic() {
    let (_temp, store, run) = setup();
    let frozen = frozen(&store, &run, Role::Author, "{}");
    let slot = reserve_saved(&store, &frozen);
    let (_executor, raw) = receive(&store, &frozen, "{}").await;
    save_result(&store, &slot, &raw, Some(b"{\"x\":1}"));
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    assert!(record_candidate(&tx, &slot.slot_id, &raw, Some(b"{\"x\":2}")).is_err());
    tx.execute("UPDATE jobs SET result='{}' WHERE id=?1", [raw.job_id()])
        .unwrap();
    assert!(record_candidate(&tx, &slot.slot_id, &raw, Some(b"{\"x\":1}")).is_err());
    let count: i64 = tx
        .query_row("SELECT count(*) FROM workflow_artifacts_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn controller_old_parent_cannot_receive_new_role_pins() {
    let (_temp, store, run) = setup();
    let frozen = frozen(&store, &run, Role::Author, "{}");
    store
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET snapshot='{}' WHERE id=?1", [&run])
        .unwrap();
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    assert!(reserve(&tx, &frozen).is_err());
    let count: i64 = tx
        .query_row("SELECT count(*) FROM series_lite_role_slots_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn controller_new_workflow_is_not_permission_to_repeat_received_or_rejected_work() {
    for state in ["reserved", "received", "queued", "completed", "rejected"] {
        let (_temp, store, run) = setup();
        let original = frozen(&store, &run, Role::Author, "{}");
        let slot = reserve_saved(&store, &original);
        if state != "reserved" {
            let (executor, raw) = receive(&store, &original, "{}").await;
            match state {
                "completed" => executor
                    .accept(&original.request, raw.transport_envelope())
                    .await
                    .unwrap(),
                "rejected" => executor
                    .reject(
                        &original.request,
                        raw.transport_envelope(),
                        "synthetic rejected",
                    )
                    .await
                    .unwrap(),
                "queued" => {
                    store
                        .conn()
                        .unwrap()
                        .execute(
                            "UPDATE jobs SET state='queued',result=NULL WHERE id=?1",
                            [raw.job_id()],
                        )
                        .unwrap();
                }
                _ => {}
            }
        }
        let second = store.add_job("workflow","storyboard.run","local","local",
            &json!({"input":{"liteControllerContract":VERSION,"source":{"sha256":"a".repeat(64)}}}),"running").unwrap();
        let replacement = frozen(&store, &second, Role::Author, "{}");
        let mut c = store.conn().unwrap();
        let tx = c.transaction().unwrap();
        assert!(reserve(&tx, &replacement).is_err(), "{state}");
        assert_eq!(
            reserve(&tx, &original).unwrap().slot_id,
            slot.slot_id,
            "original remains readable: {state}"
        );
        let count: i64 = tx
            .query_row("SELECT count(*) FROM series_lite_role_slots_v1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1, "{state}");
    }
}
