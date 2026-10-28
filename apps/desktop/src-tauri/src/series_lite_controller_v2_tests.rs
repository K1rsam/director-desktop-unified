use super::*;
use crate::pipeline::{TextExecutor, TextRequest};
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_job_packet::{assemble, RejectUnverifiedWorkspace};
use crate::series_lite_transport as transport;
use crate::store::Store;
use crate::text_executor::{compile_codex_input, SavedTextExecutor};
use crate::workflow_artifacts::Inputs;

pub(crate) fn setup() -> (tempfile::TempDir, Store, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("v2 synthetic", "synthetic").unwrap();
    store.ensure_workflow_storage().unwrap();
    let run=store.add_job("workflow","storyboard.run","local","local",&json!({"input":{"liteControllerContract":v1::VERSION,"liteRuntimeContract":VERSION,"source":{"sha256":"a".repeat(64)}}}),"running").unwrap();
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    install(&tx).unwrap();
    crate::workflow_issues::install(&tx).unwrap();
    tx.commit().unwrap();
    (temp, store, run)
}
pub(crate) fn frozen(store: &Store, run: &str, role: Role, base: &str) -> FrozenRole {
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
    let prior = history(&c, run, "1").unwrap();
    FrozenRole {
        version: v1::VERSION.into(),
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
        previous_candidate: latest_pin(&prior).cloned(),
    }
}
fn budget(tx: &mut Transaction<'_>, frozen: &FrozenRole, code: &str) -> RevisionBinding {
    let issue = crate::workflow_issues::Issue {
        version: crate::workflow_issues::VERSION.into(),
        source_sha256: frozen.source_sha256.clone(),
        origin_sha256: frozen.base_sha256.clone(),
        stage: frozen.request.stage.clone(),
        code: code.into(),
        object: frozen.context.episode.clone(),
        location: "/draft".into(),
        evidence: vec![crate::workflow_issues::Evidence {
            kind: "raw".into(),
            reference: "synthetic-bound-raw".into(),
            sha256: frozen.base_sha256.clone(),
        }],
        impact: vec!["current-episode".into()],
        repair: crate::workflow_issues::Repair::AiRevision,
        message: "仅合成证据回归".into(),
    };
    let ledger = crate::workflow_issues::register(tx, &issue).unwrap();
    let key = format!("test-attempt:{}", frozen.context.iteration_id);
    let attempt = crate::workflow_issues::Attempt {
        attempt_key: key.clone(),
        request_sha256: digest(&encode(&frozen.request).unwrap()),
        base_sha256: frozen.base_sha256.clone(),
        workflow_id: frozen.workflow_id.clone(),
        account: "synthetic-owned-account".into(),
        provider: "codex".into(),
        repair: crate::workflow_issues::Repair::AiRevision,
        action: VERSION.into(),
    };
    let reserved =
        crate::workflow_issues::reserve(tx, &ledger.cause, ledger.revision, &attempt).unwrap();
    RevisionBinding {
        cause: ledger.cause,
        ordinal: reserved.ordinal,
        attempt_key: key,
    }
}
pub(crate) fn reserve_saved(store: &Store, frozen: &FrozenRole, revision: bool) -> Slot {
    let mut c = store.conn().unwrap();
    let mut tx = c.transaction().unwrap();
    let binding = revision.then(|| budget(&mut tx, frozen, "CONTENT_REPAIR"));
    let saved = reserve(
        &tx,
        frozen,
        if revision {
            "revision"
        } else {
            frozen.context.role.as_str()
        },
        &[],
        binding,
    )
    .unwrap();
    tx.commit().unwrap();
    saved
}
fn node(frozen: &FrozenRole) -> String {
    format!("{}:{}",frozen.request.stage,digest(&json!({"episode":frozen.context.episode,"iteration":frozen.context.iteration_id,"context":frozen.context.context_id}).to_string()))
}
pub(crate) async fn receive(
    store: &Store,
    frozen: &FrozenRole,
    text: &str,
) -> (SavedTextExecutor, VerifiedRawText) {
    // Complete synthetic saved CLI evidence. The named executable does not
    // exist, and the production reader consumes this result without launch.
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
    let node = node(frozen);
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
pub(crate) fn save(
    store: &Store,
    slot: &Slot,
    raw: &VerifiedRawText,
    candidate: Option<&[u8]>,
    invalid: Option<&str>,
) -> Slot {
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    let saved = record_candidate(&tx, &slot.record.slot_id, raw, candidate, invalid).unwrap();
    tx.commit().unwrap();
    saved
}
pub(crate) async fn complete(
    store: &Store,
    slot: &Slot,
    text: &str,
    candidate: Option<&[u8]>,
    invalid: Option<&str>,
) {
    let (executor, raw) = receive(store, &slot.record.frozen, text).await;
    save(store, slot, &raw, candidate, invalid);
    if let Some(error) = invalid {
        executor
            .reject(&slot.record.frozen.request, raw.transport_envelope(), error)
            .await
            .unwrap();
    } else {
        executor
            .accept(&slot.record.frozen.request, raw.transport_envelope())
            .await
            .unwrap();
    }
}

#[test]
fn old_database_read_is_zero_ddl_and_v1_parent_cannot_be_resigned() {
    let c = Connection::open_in_memory().unwrap();
    assert!(history(&c, "missing", "1").unwrap().is_empty());
    assert_eq!(
        c.query_row::<u64, _, _>("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get(0))
            .unwrap(),
        0
    );
    let (_temp, store, run) = setup();
    let f = frozen(&store, &run, Role::Author, "{}");
    store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",params![json!({"input":{"liteControllerContract":v1::VERSION,"source":{"sha256":"a".repeat(64)}}}).to_string(),run]).unwrap();
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    assert!(reserve(&tx, &f, "author", &[], None)
        .unwrap_err()
        .contains("PARENT_NOT_V2"));
}
#[test]
fn reservation_is_append_only_idempotent_and_rejects_context_or_input_drift() {
    let (_temp, store, run) = setup();
    let f = frozen(&store, &run, Role::Author, "{}");
    let first = reserve_saved(&store, &f, false);
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    assert_eq!(
        reserve(&tx, &f, "author", &[], None)
            .unwrap()
            .record
            .slot_id,
        first.record.slot_id
    );
    let mut changed = f.clone();
    changed.request.model = "different".into();
    assert!(reserve(&tx, &changed, "author", &[], None).is_err());
    assert!(tx
        .execute("UPDATE series_lite_role_slots_v2 SET data='{}'", [])
        .is_err());
    assert!(tx
        .execute("DELETE FROM series_lite_role_slots_v2", [])
        .is_err());
    assert!(context_for(&tx, &run, "1", Role::Reviewer).is_err());
    assert_eq!(
        context_for(&tx, &run, "1", Role::Author).unwrap(),
        f.context
    );
}
#[tokio::test]
async fn rejected_raw_repairs_and_all_creative_contributors_survive_reopen_without_approval() {
    let (_temp, store, run) = setup();
    let f = frozen(&store, &run, Role::Author, "{}");
    let first = reserve_saved(&store, &f, false);
    complete(
        &store,
        &first,
        "not-json-native",
        None,
        Some("AUTHOR_FORMAT"),
    )
    .await;
    let repaired = frozen(&store, &run, Role::Author, "{}");
    assert_ne!(repaired.context, f.context);
    assert_eq!(repaired.context.prior_contributors.len(), 1);
    let repair = reserve_saved(&store, &repaired, true);
    let mut base = "{\"exact\":1.000000000000000001}";
    complete(
        &store,
        &repair,
        "{\"creative\":1}",
        Some(base.as_bytes()),
        None,
    )
    .await;
    let finish = frozen(&store, &run, Role::Finisher, base);
    let finisher = reserve_saved(&store, &finish, false);
    assert_eq!(finish.context.prior_contributors.len(), 2);
    base = "{\"exact\":2.000000000000000001}";
    complete(
        &store,
        &finisher,
        "{\"patch\":[]}",
        Some(base.as_bytes()),
        None,
    )
    .await;
    let review = frozen(&store, &run, Role::Reviewer, base);
    let reviewer = reserve_saved(&store, &review, false);
    assert_eq!(review.context.prior_contributors.len(), 3);
    complete(&store, &reviewer, "{\"verdict\":\"revise\"}", None, None).await;
    let again = frozen(&store, &run, Role::Author, base);
    let revision = reserve_saved(&store, &again, true);
    assert_eq!(again.context.prior_contributors.len(), 3);
    complete(
        &store,
        &revision,
        "{\"repair\":[]}",
        Some(b"{\"exact\":3.000000000000000001}"),
        None,
    )
    .await;
    let last = frozen(
        &store,
        &run,
        Role::Reviewer,
        "{\"exact\":3.000000000000000001}",
    );
    let final_review = reserve_saved(&store, &last, false);
    assert_eq!(last.context.prior_contributors.len(), 4);
    assert!(last
        .context
        .prior_contributors
        .iter()
        .all(|c| c.role != Role::Reviewer));
    complete(&store, &final_review, "{\"verdict\":\"pass\"}", None, None).await;
    let reopened = Store::open(store.root.clone()).unwrap();
    let rows = history(&reopened.conn().unwrap(), &run, "1").unwrap();
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0].invalid.as_deref(), Some("AUTHOR_FORMAT"));
    assert_eq!(rows[0].record.transport_state.as_deref(), Some("rejected"));
    for s in &rows {
        let raw = artifacts::artifact(
            &reopened.conn().unwrap(),
            &s.record.result.as_ref().unwrap().raw_artifact_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(raw.artifact.payload["contentReview"], "NOT_PROVEN");
        assert_eq!(raw.artifact.payload["adoptionGranted"], false);
    }
    assert_eq!(
        reopened
            .conn()
            .unwrap()
            .query_row::<u64, _, _>("SELECT COUNT(*) FROM workflow_validations_v1", [], |r| r
                .get(0))
            .unwrap(),
        0
    );
    assert_eq!(
        reopened
            .conn()
            .unwrap()
            .query_row::<u64, _, _>("SELECT COUNT(*) FROM workflow_adoptions_v1", [], |r| r
                .get(0))
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn unknown_cannot_create_next_slot_or_reset_identity_by_account_change() {
    let (_temp, store, run) = setup();
    let f = frozen(&store, &run, Role::Author, "{}");
    let slot = reserve_saved(&store, &f, false);
    let (_executor, raw) = receive(&store, &f, "{}").await;
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
            [raw.job_id()],
        )
        .unwrap();
    assert!(context_for(&store.conn().unwrap(), &run, "1", Role::Finisher).is_err());
    assert_eq!(
        context_for(&store.conn().unwrap(), &run, "1", Role::Author).unwrap(),
        f.context
    );
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    assert_eq!(
        reserve(&tx, &f, "author", &[], None)
            .unwrap()
            .record
            .slot_id,
        slot.record.slot_id
    );
    assert!(record_candidate(&tx, &slot.record.slot_id, &raw, Some(b"{}"), None).is_err());
    assert_eq!(history(&tx, &run, "1").unwrap().len(), 1);
}
#[tokio::test]
async fn repeated_author_requires_budget_and_exact_latest_candidate_pin() {
    let (_temp, store, run) = setup();
    let initial = frozen(&store, &run, Role::Author, "{}");
    let slot = reserve_saved(&store, &initial, false);
    complete(&store, &slot, "{}", Some(b"{\"draft\":1}"), None).await;
    let next = frozen(&store, &run, Role::Author, "{\"draft\":1}");
    let mut c = store.conn().unwrap();
    let mut tx = c.transaction().unwrap();
    assert!(reserve(&tx, &next, "revision", &[], None)
        .unwrap_err()
        .contains("REVISION_BUDGET"));
    let binding = budget(&mut tx, &next, "FORMAT_REPAIR");
    let mut bad = next.clone();
    bad.previous_candidate = None;
    assert!(reserve(&tx, &bad, "revision", &[], Some(binding.clone()))
        .unwrap_err()
        .contains("LATEST_CANDIDATE"));
    let saved = reserve(&tx, &next, "revision", &[], Some(binding.clone())).unwrap();
    let old = reserve(&tx, &next, "revision", &[], Some(binding)).unwrap();
    assert_eq!(saved.record.slot_id, old.record.slot_id);
    assert_eq!(
        tx.query_row::<u64, _, _>("SELECT SUM(attempts) FROM workflow_issue_v1", [], |r| r
            .get(0))
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn raw_and_candidate_tampering_are_detected_and_failed_writes_are_atomic() {
    let (_temp, store, run) = setup();
    let f = frozen(&store, &run, Role::Author, "{}");
    let slot = reserve_saved(&store, &f, false);
    let (executor, raw) = receive(&store, &f, "{\"raw\":1.000000000000000001}").await;
    save(
        &store,
        &slot,
        &raw,
        Some(b"{\"candidate\":1.000000000000000001}"),
        None,
    );
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    assert!(record_candidate(
        &tx,
        &slot.record.slot_id,
        &raw,
        Some(b"{\"candidate\":2}"),
        None
    )
    .is_err());
    assert_eq!(
        tx.query_row::<u64, _, _>("SELECT COUNT(*) FROM workflow_artifacts_v1", [], |r| r
            .get(0))
            .unwrap(),
        2
    );
    tx.commit().unwrap();
    executor
        .accept(&f.request, raw.transport_envelope())
        .await
        .unwrap();
    let c = store.conn().unwrap();
    c.execute("UPDATE jobs SET result='{}' WHERE id=?1", [raw.job_id()])
        .unwrap();
    assert!(history(&c, &run, "1").is_err());
}
#[tokio::test]
async fn rejected_mechanical_candidate_remains_patchable_without_becoming_approved() {
    let (_temp, store, run) = setup();
    let f = frozen(&store, &run, Role::Author, "{}");
    let slot = reserve_saved(&store, &f, false);
    complete(
        &store,
        &slot,
        "{\"draft\":\"returned\"}",
        Some(b"{\"draft\":1.000000000000000001}"),
        Some("LOCAL_GATE_FAILED"),
    )
    .await;
    let rows = history(
        &Store::open(store.root.clone()).unwrap().conn().unwrap(),
        &run,
        "1",
    )
    .unwrap();
    assert_eq!(rows[0].invalid.as_deref(), Some("LOCAL_GATE_FAILED"));
    assert_eq!(rows[0].record.transport_state.as_deref(), Some("rejected"));
    let pin = rows[0]
        .record
        .result
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap();
    let candidate = artifacts::artifact(&store.conn().unwrap(), &pin.artifact_id)
        .unwrap()
        .unwrap();
    assert_eq!(candidate.artifact.payload["validation"], "NOT_PROVEN");
    assert_eq!(
        candidate.artifact.payload["text"],
        "{\"draft\":1.000000000000000001}"
    );
    let next = frozen(
        &store,
        &run,
        Role::Finisher,
        "{\"draft\":1.000000000000000001}",
    );
    assert_eq!(next.previous_candidate.as_ref(), Some(pin));
    assert_eq!(next.context.prior_contributors.len(), 1);
    assert_eq!(
        store
            .conn()
            .unwrap()
            .query_row::<u64, _, _>("SELECT COUNT(*) FROM workflow_validations_v1", [], |r| r
                .get(0))
            .unwrap(),
        0
    );
}
