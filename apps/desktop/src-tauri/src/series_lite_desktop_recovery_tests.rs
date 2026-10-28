//! Source-bound runtime regressions; synthetic fixtures never launch a model.
use super::*;
use crate::series_lite_controller_v2::tests as saved;
use crate::series_lite_edit::{canonical, Json};
use crate::series_lite_scene_projection::{SceneSupply, SuppliedAsset, SuppliedScene, SUPPLY};
use crate::series_lite_transport::{RoleContext, TransportIdentity};
use crate::text_executor::VerifiedRawText;
use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
struct NoDispatch {
    calls: AtomicUsize,
    contexts: Mutex<Vec<String>>,
}
#[async_trait]
impl TextExecutor for NoDispatch {
    async fn execute(&self, _: TextRequest) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err("TEST_UNEXPECTED_DISPATCH".into())
    }
    async fn execute_exact(
        &self,
        _: TextRequest,
        _: &TransportIdentity,
        context: &RoleContext,
    ) -> Result<VerifiedRawText> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.contexts
            .lock()
            .unwrap()
            .push(context.context_id.clone());
        Err("TEST_ORIGINAL_UNKNOWN_RECONCILE".into())
    }
}
fn context<'a>(
    store: &'a Store,
    run: &'a str,
    root: &'a Path,
    executor: &'a dyn TextExecutor,
    options: &'a PreparationOptions,
    guard: &'a Value,
) -> Context<'a> {
    Context {
        store,
        run,
        root,
        episode: "1".into(),
        supply: b"[]",
        guard,
        common: Inputs::from([
            ("source".into(), "a".repeat(64)),
            ("supply".into(), "b".repeat(64)),
            ("visualBaseline".into(), "c".repeat(64)),
            ("rulesPack".into(), "d".repeat(64)),
        ]),
        options,
        executor,
        model: "synthetic-no-dispatch",
        reasoning: "medium",
        boundary: json!({"kind":"firstInScope"}),
    }
}
fn complete_fixture() -> (tempfile::TempDir, Vec<u8>, Vec<u8>, Vec<u8>) {
    let root = tempfile::tempdir().unwrap();
    let corpus: Value =
        serde_json::from_str(include_str!("series_lite_execution_golden.json")).unwrap();
    for (path, text) in corpus["files"].as_object().unwrap() {
        let target = root.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, text.as_str().unwrap()).unwrap();
    }
    let mut full: Value =
        serde_json::from_str(corpus["files"]["bundle.json"].as_str().unwrap()).unwrap();
    full["generationBoundaryVersion"] = json!("video-prompt-v1");
    full["episodes"][0]["settings"]["fantasyMode"] = json!("off");
    let events = vec![
        json!({"id":"EV1","sourceRefs":["E01:R1","E01:R2"],"sceneKey":"1-1","boundaryBefore":{"kind":"episode_start","reason":"原源首场开始"},"emotionContract":{"subject":"值守者","contextTone":"戒备","audienceEffect":"读到克制提醒","change":"等待转为提醒","landing":"继续保留面罩与持物","retain":["门未开","原话与右手钥匙保持"]}}),
        json!({"id":"EV2","sourceRefs":["E01:R3","E01:R4"],"sceneKey":"1-2","boundaryBefore":{"kind":"new_event","reason":"原源转入空走廊与系统声"},"emotionContract":{"subject":"空走廊","contextTone":"等待","audienceEffect":"收到门锁确认","change":"等待转为确认","landing":"无人走廊保持","retain":["系统音不具象化","原话原声保持"]}}),
    ];
    full["episodes"][0]["plan"] = json!({"schemaVersion":"performance-core-v4-plan","unitPackingPolicy":"event-first-v1","emotionArc":"从闭门等待到系统确认","events":events,"entryFacts":[],"fantasyCandidates":[]});
    for index in 0..2 {
        full["episodes"][0]["draft"]["units"][index]["eventId"] = events[index]["id"].clone();
        full["episodes"][0]["draft"]["units"][index]["boundaryBefore"] =
            events[index]["boundaryBefore"].clone();
    }
    let mut base = full.clone();
    for field in ["plan", "selection", "draft"] {
        base["episodes"][0][field] = json!({});
    }
    let base = serde_json::to_vec(&base).unwrap();
    let response=serde_json::to_vec(&json!({"plan":full["episodes"][0]["plan"],"selection":full["episodes"][0]["selection"],"draft":full["episodes"][0]["draft"]})).unwrap();
    let exact = parse(&serde_json::to_vec(&full).unwrap()).unwrap();
    let ep = &exact.get("episodes").unwrap().array().unwrap()[0];
    let supply = SceneSupply {
        version: SUPPLY.into(),
        source_hash: s(&full, "sourceHash").into(),
        source_lock_hash: s(&full, "sourceLockHash").into(),
        episode: 1,
        source_value_sha256: sha256(&canonical(ep.get("source").unwrap())),
        registry_snapshot_sha256: "c".repeat(64),
        evidence_sha256: "d".repeat(64),
        scenes: ep
            .get("scenes")
            .unwrap()
            .array()
            .unwrap()
            .iter()
            .map(|scene| {
                let mut protected = scene.object().unwrap().clone();
                protected.remove("bindings");
                SuppliedScene {
                    id: scene.get("id").unwrap().string().unwrap().into(),
                    protected_fields_sha256: sha256(&canonical(&Json::Object(protected))),
                    allowed_assets: scene
                        .get("bindings")
                        .unwrap()
                        .array()
                        .unwrap()
                        .iter()
                        .map(|a| a.string().unwrap().to_owned())
                        .collect(),
                }
            })
            .collect(),
        assets: full["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| SuppliedAsset {
                key: s(a, "key").into(),
                asset_id: s(a, "assetId").into(),
                asset_revision: a["assetRevision"].as_u64().unwrap(),
                full_design_sha256: "e".repeat(64),
                material_source_only: false,
                image_allowed: true,
            })
            .collect(),
    };
    (root, base, response, serde_json::to_vec(&supply).unwrap())
}

#[test]
fn author_compose_and_finisher_patch_keep_exact_source_and_pass_the_local_gate() {
    let (root, base, response, supply) = complete_fixture();
    let (_temp, store, run) = saved::setup();
    let executor = NoDispatch::default();
    let options = PreparationOptions::default();
    let guard = Value::Null;
    let mut ctx = context(&store, &run, root.path(), &executor, &options, &guard);
    ctx.supply = &supply;
    let (candidate, error) = apply(&ctx, Role::Author, &base, &response, &[]);
    assert!(error.is_none(), "{error:?}");
    let candidate = candidate.unwrap();
    mechanical(&ctx, &candidate).unwrap();
    let before = parse(&base).unwrap();
    let authored = parse(&candidate).unwrap();
    for field in ["source", "settings", "scenes"] {
        assert_eq!(
            before.get("episodes").unwrap().array().unwrap()[0]
                .get(field)
                .unwrap(),
            authored.get("episodes").unwrap().array().unwrap()[0]
                .get(field)
                .unwrap()
        );
    }
    assert_eq!(
        before.get("assets").unwrap(),
        authored.get("assets").unwrap()
    );
    let patch=format!("{{\"baseSha256\":\"{}\",\"operations\":[{{\"op\":\"set\",\"path\":\"/episodes/0/draft/units/0/shots/0/duration\",\"expected\":2.5,\"value\":2.500000001}}]}}",sha256(&candidate));
    let (patched, error) = apply(&ctx, Role::Finisher, &candidate, patch.as_bytes(), &[]);
    assert!(error.is_none(), "{error:?}");
    let patched = patched.unwrap();
    assert!(std::str::from_utf8(&patched)
        .unwrap()
        .contains("2.500000001"));
    mechanical(&ctx, &patched).unwrap();
    let after = parse(&patched).unwrap();
    assert_eq!(
        authored.get("episodes").unwrap().array().unwrap()[0]
            .get("source")
            .unwrap(),
        after.get("episodes").unwrap().array().unwrap()[0]
            .get("source")
            .unwrap()
    );
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn malformed_output_and_protected_source_patch_cannot_create_a_candidate() {
    let (root, base, response, supply) = complete_fixture();
    let (_temp, store, run) = saved::setup();
    let executor = NoDispatch::default();
    let options = PreparationOptions::default();
    let guard = Value::Null;
    let mut ctx = context(&store, &run, root.path(), &executor, &options, &guard);
    ctx.supply = &supply;
    let (candidate, error) = apply(&ctx, Role::Author, &base, b"not-json", &[]);
    assert!(candidate.is_none());
    assert!(error.is_some());
    let (candidate, error) = apply(&ctx, Role::Author, &base, &response, &[]);
    assert!(error.is_none());
    let candidate = candidate.unwrap();
    let patch = json!({"baseSha256":sha256(&candidate),"operations":[{"op":"set","path":"/episodes/0/source/sourceRows/0/text","expected":"源文","value":"被模型改写"}]});
    let (changed, error) = apply(
        &ctx,
        Role::Finisher,
        &candidate,
        &serde_json::to_vec(&patch).unwrap(),
        &[],
    );
    assert!(changed.is_none());
    assert!(error.unwrap().contains("patch may only"));
    let drift = String::from_utf8(candidate)
        .unwrap()
        .replace("门锁正常。", "门锁已被改写。");
    let empty_patch = json!({"baseSha256":sha256(drift.as_bytes()),"operations":[]});
    let (_, error) = apply(
        &ctx,
        Role::Finisher,
        drift.as_bytes(),
        &serde_json::to_vec(&empty_patch).unwrap(),
        &[],
    );
    assert!(
        error.is_some(),
        "same-hash self-report cannot replace the frozen source supply"
    );
    assert_eq!(store.values("assets").unwrap().len(), 0);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn mechanical_failure_retains_a_patchable_candidate_but_never_claims_pass() {
    let (root, base, response, supply) = complete_fixture();
    let (_temp, store, run) = saved::setup();
    let executor = NoDispatch::default();
    let options = PreparationOptions::default();
    let guard = Value::Null;
    let mut ctx = context(&store, &run, root.path(), &executor, &options, &guard);
    ctx.supply = &supply;
    let head = root.path().join("registry/current.json");
    let mut changed: Value = serde_json::from_slice(&fs::read(&head).unwrap()).unwrap();
    changed["sha256"] = json!("0".repeat(64));
    fs::write(head, changed.to_string()).unwrap();
    let (candidate, error) = apply(&ctx, Role::Author, &base, &response, &[]);
    assert!(candidate.is_some());
    assert_eq!(error.as_deref(), Some("LITE_REGISTRY_DRIFT"));
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
}
fn budget_issue(origin: &[u8]) -> issues::Issue {
    issues::Issue {
        version: issues::VERSION.into(),
        source_sha256: "a".repeat(64),
        origin_sha256: sha256(origin),
        stage: "series-lite:format".into(),
        code: "LITE_OUTPUT_REVISION".into(),
        object: "1".into(),
        location: "/episodes/1/format".into(),
        evidence: vec![issues::Evidence {
            kind: "synthetic-original".into(),
            reference: "frozen-response".into(),
            sha256: "e".repeat(64),
        }],
        impact: vec!["1".into()],
        repair: issues::Repair::AiRevision,
        message: "合成预算回归".into(),
    }
}
fn reserve_budget(
    store: &Store,
    run: &str,
    issue: &issues::Issue,
    ordinal: u64,
    base: &[u8],
    account: &str,
) -> Revision {
    let mut c = store.conn().unwrap();
    let mut tx = c.transaction().unwrap();
    issues::install(&tx).unwrap();
    let ledger = issues::register(&mut tx, issue).unwrap();
    let reserved = issues::reserve(
        &mut tx,
        &ledger.cause,
        ledger.revision,
        &issues::Attempt {
            attempt_key: format!("recovery:{}:{ordinal}", ledger.cause),
            request_sha256: "f".repeat(64),
            base_sha256: sha256(base),
            workflow_id: run.into(),
            provider: "codex_sdk".into(),
            account: account.into(),
            repair: issues::Repair::AiRevision,
            action: ledger::VERSION.into(),
        },
    )
    .unwrap();
    assert_eq!(reserved.ordinal, ordinal);
    tx.commit().unwrap();
    Revision {
        issue: issue.clone(),
        ordinal,
        basis_job: "synthetic-original-receipt".into(),
    }
}
#[test]
fn finish_revision_is_idempotent_and_two_round_budget_survives_reopen_and_account_switch() {
    let (temp, store, run) = saved::setup();
    let executor = NoDispatch::default();
    let options = PreparationOptions::default();
    let guard = Value::Null;
    let issue = budget_issue(b"origin");
    let first = reserve_budget(&store, &run, &issue, 1, b"origin", "owned-account-A");
    let ctx = context(&store, &run, temp.path(), &executor, &options, &guard);
    let detail = json!({"sameFinding":"still-invalid"});
    finish_revision(&ctx, &first, b"first-result", false, &detail).unwrap();
    let cause = issue.cause().unwrap();
    let once = issues::read(&store.conn().unwrap(), &cause)
        .unwrap()
        .unwrap();
    finish_revision(&ctx, &first, b"first-result", false, &detail).unwrap();
    assert_eq!(
        issues::read(&store.conn().unwrap(), &cause)
            .unwrap()
            .unwrap(),
        once
    );
    assert!(finish_revision(&ctx, &first, b"different-result", false, &detail).is_err());
    let reopened = Store::open(store.root.clone()).unwrap();
    let second = reserve_budget(
        &reopened,
        &run,
        &issue,
        2,
        b"first-result",
        "owned-backup-account-B",
    );
    let ctx = context(&reopened, &run, temp.path(), &executor, &options, &guard);
    finish_revision(
        &ctx,
        &second,
        b"second-result",
        false,
        &json!({"sameFinding":"still-invalid","round":2}),
    )
    .unwrap();
    let final_state = issues::read(&reopened.conn().unwrap(), &cause)
        .unwrap()
        .unwrap();
    assert_eq!(final_state.attempts, 2);
    assert_eq!(final_state.state, "exhausted");
    finish_revision(
        &ctx,
        &second,
        b"second-result",
        false,
        &json!({"sameFinding":"still-invalid","round":2}),
    )
    .unwrap();
    assert_eq!(
        issues::read(&reopened.conn().unwrap(), &cause)
            .unwrap()
            .unwrap(),
        final_state
    );
    let mut c = reopened.conn().unwrap();
    let mut tx = c.transaction().unwrap();
    assert!(issues::reserve(
        &mut tx,
        &cause,
        final_state.revision,
        &issues::Attempt {
            attempt_key: "third-new-account".into(),
            request_sha256: "f".repeat(64),
            base_sha256: sha256(b"second-result"),
            workflow_id: run,
            provider: "codex_sdk".into(),
            account: "account-C".into(),
            repair: issues::Repair::AiRevision,
            action: ledger::VERSION.into()
        }
    )
    .is_err());
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn terminal_saved_step_replays_without_calling_any_executor() {
    let (temp, store, run) = saved::setup();
    let frozen = saved::frozen(&store, &run, Role::Author, "{}");
    let slot = saved::reserve_saved(&store, &frozen, false);
    saved::complete(
        &store,
        &slot,
        "{\"raw\":1.000000000000000001}",
        Some(b"{\"candidate\":1.000000000000000001}"),
        None,
    )
    .await;
    let reopened = Store::open(store.root.clone()).unwrap();
    let executor = NoDispatch::default();
    let options = PreparationOptions::default();
    let guard = Value::Null;
    let ctx = context(&reopened, &run, temp.path(), &executor, &options, &guard);
    let mut cursor = 0;
    let before = reopened.jobs().unwrap();
    let replayed = step(
        &ctx,
        &mut cursor,
        Role::Author,
        "author",
        b"{}",
        None,
        &[],
        None,
        &Value::Null,
    )
    .await
    .unwrap();
    assert_eq!(replayed.record.slot_id, slot.record.slot_id);
    assert_eq!(cursor, 1);
    assert_eq!(
        candidate(&reopened, &replayed.record).unwrap().unwrap(),
        b"{\"candidate\":1.000000000000000001}"
    );
    assert_eq!(
        raw_response(&reopened, &replayed.record).unwrap(),
        b"{\"raw\":1.000000000000000001}"
    );
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(reopened.jobs().unwrap(), before);
}
#[tokio::test]
async fn unknown_step_keeps_original_slot_and_reconciliation_context() {
    let (temp, store, run) = saved::setup();
    let frozen = saved::frozen(&store, &run, Role::Author, "{}");
    let slot = saved::reserve_saved(&store, &frozen, false);
    let (_executor, raw) = saved::receive(&store, &frozen, "{}").await;
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET state='submission_unknown',result=NULL WHERE id=?1",
            [raw.job_id()],
        )
        .unwrap();
    let executor = NoDispatch::default();
    let options = PreparationOptions::default();
    let guard = Value::Null;
    let ctx = context(&store, &run, temp.path(), &executor, &options, &guard);
    let mut cursor = 0;
    let before = store.jobs().unwrap();
    let error = step(
        &ctx,
        &mut cursor,
        Role::Author,
        "author",
        b"{}",
        None,
        &[],
        None,
        &Value::Null,
    )
    .await
    .unwrap_err();
    assert_eq!(error, "TEST_ORIGINAL_UNKNOWN_RECONCILE");
    let rows = ledger::history(&store.conn().unwrap(), &run, "1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].record.slot_id, slot.record.slot_id);
    assert_eq!(store.jobs().unwrap(), before);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        executor.contexts.lock().unwrap().as_slice(),
        [frozen.context.context_id]
    );
    assert_eq!(
        store
            .conn()
            .unwrap()
            .query_row::<u64, _, _>("SELECT COUNT(*) FROM workflow_issue_attempt_v1", [], |r| r
                .get(0))
            .unwrap(),
        0
    );
}
