use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    key: String,
    cfg: ProviderConfig,
    account: String,
    journal: PathBuf,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("unknown read", "project").unwrap();
    let root = temp.path().join("requests");
    let id = "original-owned-request";
    let cfg:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","timeoutSeconds":5,
        "browserTransport":{"transport":"opencli/v1","stateRoot":root,"profile":"test-profile","accountKey":"a".repeat(64),
            "runtimeSha256":include_str!("../../../../web-image-bridge/runtime.sha256").trim(),"runner":temp.path().join("never-run.mjs")}})).unwrap();
    let account = "chrome:opencli:synthetic-account".to_owned();
    let request:GenerationRequest=serde_json::from_value(json!({"requestId":id,"kind":"image","model":"chatgpt-image","prompt":"synthetic exact prompt",
        "ratio":"16:9","resolution":"native","references":[],"options":{"synthetic":true}})).unwrap();
    let prompt = format!("Generate one image only. {}", request.prompt);
    let prepared = PreparedRequest {
        provider: "chrome".into(),
        request: request.clone(),
        capability_version: "opencli-image/v1".into(),
        upload_receipts: vec![],
        estimated_cost: None,
        prepared_at: now(),
        wire: json!({"transport":"opencli/v1","requestHash":"b".repeat(64),"profile":"test-profile","accountKey":"a".repeat(64),"sendProtocol":"owned-composer-send/v2","composerAdapterSha256":"c".repeat(64)}),
    };
    let mut snapshot = json!(request);
    snapshot["providerConfig"] = json!(cfg);
    snapshot["frozenSettings"] = json!({"synthetic":true});
    let key = store
        .add_job(
            "image",
            "asset",
            "chrome",
            &account,
            &snapshot,
            "submission_unknown",
        )
        .unwrap();
    store.update_job(&key,"submission_unknown",&json!({"prepared":prepared,"error":"original uncertainty","queryAttempts":3,"downloadAttempts":2})).unwrap();
    let journal = root.join(id).join("request.json");
    fs::create_dir_all(journal.parent().unwrap()).unwrap();
    fs::write(&journal,json!({"version":"opencli-task/v1","requestId":id,"request":request,"requestHash":prepared.wire["requestHash"],
        "profile":"test-profile","accountKey":"a".repeat(64),"page":"SYNTHETIC_PAGE","status":"submission_unknown","prompt":prompt,
        "createdAt":"2026-10-25T00:00:00Z","sentAt":"2026-10-25T00:00:05Z","conversationUrl":null,
        "ownedSend":{"version":"owned-composer-send/v2","token":id,"adapterSha256":prepared.wire["composerAdapterSha256"]},
        "sendIntent":{"version":"owned-click-intent/v1","token":id,"requestId":id,"requestHash":prepared.wire["requestHash"],"page":"SYNTHETIC_PAGE","promptSha256":crate::pipeline::digest(&prompt)}}).to_string()).unwrap();
    Fixture {
        _temp: temp,
        store,
        key,
        cfg,
        account,
        journal,
    }
}
#[derive(Clone, Copy)]
enum Reply {
    Unknown,
    Network,
    Auth,
    WrongTask,
    Success,
    Processing,
    Loading,
}
struct ReadOnly {
    journal: PathBuf,
    reply: Mutex<Reply>,
    queries: AtomicUsize,
    submits: AtomicUsize,
}
impl ReadOnly {
    fn new(f: &Fixture, reply: Reply) -> Self {
        Self {
            journal: f.journal.clone(),
            reply: Mutex::new(reply),
            queries: AtomicUsize::new(0),
            submits: AtomicUsize::new(0),
        }
    }
    fn count(&self) -> usize {
        self.queries.load(Ordering::SeqCst)
    }
}
impl Channels for ReadOnly {
    fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> {
        panic!("no capabilities or prepare")
    }
    fn prepare(
        &self,
        _: &ProviderConfig,
        _: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        panic!("no prepare")
    }
    fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        self.submits.fetch_add(1, Ordering::SeqCst);
        panic!("never send")
    }
    fn query(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("must query frozen prepared request")
    }
    fn query_prepared(
        &self,
        _: &ProviderConfig,
        p: &PreparedRequest,
        id: &str,
    ) -> ProviderResult<QueryResult> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        assert_eq!(id, "original-owned-request");
        assert_eq!(id, p.request.request_id);
        let reply = *self.reply.lock().unwrap();
        if matches!(reply, Reply::Network) {
            return Err(ProviderError::new(
                "OPENCLI_READ_TIMEOUT",
                "synthetic network timeout",
            ));
        }
        if matches!(reply, Reply::Auth) {
            return Err(ProviderError::new(
                "ACCOUNT_CHANGED",
                "synthetic other account",
            ));
        }
        let mut result = QueryResult {
            task_id: id.into(),
            status: "unknown".into(),
            result_urls: vec![],
            error: None,
            actual_cost: None,
            raw: json!({"transport":"opencli/v1"}),
        };
        match reply {
            Reply::WrongTask => {
                result.task_id = "foreign-task".into();
                result.status = "succeeded".into();
            }
            Reply::Loading => {
                result.status = "processing".into();
                result.raw["phase"] = json!("loading_original_page");
            }
            Reply::Success | Reply::Processing => {
                let url = "https://chatgpt.com/c/synthetic-owned-conversation";
                let mut saved: Value =
                    serde_json::from_slice(&fs::read(&self.journal).unwrap()).unwrap();
                saved["sendConfirmed"] = json!(true);
                saved["conversationUrl"] = json!(url);
                result.raw["conversationUrl"] = json!(url);
                result.status = "processing".into();
                if matches!(reply, Reply::Success) {
                    saved["status"] = json!("downloaded");
                    result.status = "succeeded".into();
                    result.result_urls = vec![format!("opencli-image:{id}:{}", "d".repeat(64))];
                    result.raw["requestHash"] = p.wire["requestHash"].clone();
                    result.raw["originalIdentityVerified"] = json!(true);
                    result.raw["syntheticOriginalProof"] = json!("retained");
                }
                fs::write(&self.journal, saved.to_string()).unwrap();
            }
            _ => {}
        }
        Ok(result)
    }
    fn download(&self, _: &ProviderConfig, _: &str, _: &Path) -> ProviderResult<DownloadReceipt> {
        panic!("separate existing download lane")
    }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("do not trust journal request without frozen preparation")
    }
}
fn run(f: &Fixture, c: &ReadOnly, at: i64) -> bool {
    run_at(&f.store, &f.key, &f.cfg, &f.account, c, at).unwrap()
}
fn budget(f: &Fixture) -> Budget {
    saved_budget(&f.store, &f.key).unwrap().1.unwrap()
}
fn change_journal(f: &Fixture, mutate: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&fs::read(&f.journal).unwrap()).unwrap();
    mutate(&mut value);
    fs::write(&f.journal, value.to_string()).unwrap();
}

#[test]
fn owned_unknown_success_is_read_only_delayed_and_keeps_original_request_and_retry_budgets() {
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Success);
    let before = f.store.job(&f.key).unwrap();
    assert!(run(&f, &channels, 100));
    assert_eq!(channels.count(), 0);
    assert_eq!(budget(&f).next_attempt_at, 115);
    assert!(!run(&f, &channels, 114));
    assert!(run(&f, &channels, 115));
    let after = f.store.job(&f.key).unwrap();
    assert_eq!(after["status"], "succeeded");
    assert_eq!(after["providerTaskId"], "original-owned-request");
    assert_eq!(after["result"]["raw"]["syntheticOriginalProof"], "retained");
    for field in [
        "snapshot",
        "prepared",
        "receipt",
        "queryAttempts",
        "downloadAttempts",
    ] {
        assert_eq!(after[field], before[field], "{field}");
    }
    assert_eq!(channels.count(), 1);
    assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
    assert_eq!(budget(&f).status, "completed");
    assert!(!run(&f, &channels, 100000));
    assert!(!opencli_unknown_pending(&f.store, &after).unwrap());
}

#[test]
fn five_durable_unknown_reads_survive_reopen_and_exhaust_without_a_loop_or_resubmit() {
    let mut f = fixture();
    let channels = ReadOnly::new(&f, Reply::Network);
    let before = f.store.job(&f.key).unwrap();
    let bytes = fs::read(&f.journal).unwrap();
    assert!(run(&f, &channels, 100));
    for (index, at) in [115, 145, 205, 325, 565].into_iter().enumerate() {
        f.store = Store::open(f.store.root.clone()).unwrap();
        assert!(!run(&f, &channels, at - 1));
        assert!(run(&f, &channels, at));
        assert_eq!(budget(&f).attempts, index + 1);
        assert_eq!(f.store.job(&f.key).unwrap(), before);
        assert_eq!(fs::read(&f.journal).unwrap(), bytes);
        *channels.reply.lock().unwrap() = Reply::Unknown;
    }
    assert_eq!(budget(&f).status, "exhausted");
    assert!(!opencli_unknown_pending(&f.store, &before).unwrap());
    f.store = Store::open(f.store.root.clone()).unwrap();
    for at in [566, 100000, 200000] {
        assert!(!run(&f, &channels, at));
    }
    assert_eq!(channels.count(), 5);
    assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
}

#[test]
fn loading_without_ack_remains_unknown_and_acknowledged_processing_preserves_old_query_budget() {
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Loading);
    let before = f.store.job(&f.key).unwrap();
    run(&f, &channels, 100);
    run(&f, &channels, 115);
    assert_eq!(f.store.job(&f.key).unwrap(), before);
    *channels.reply.lock().unwrap() = Reply::Processing;
    run(&f, &channels, 145);
    let after = f.store.job(&f.key).unwrap();
    assert_eq!(after["status"], "processing");
    assert_eq!(after["providerTaskId"], "original-owned-request");
    assert_eq!(after["queryAttempts"], 3);
    assert_eq!(after["downloadAttempts"], 2);
    assert_eq!(channels.count(), 2);
}

#[test]
fn account_and_response_identity_errors_stop_while_retaining_unknown() {
    for reply in [Reply::Auth, Reply::WrongTask] {
        let f = fixture();
        let channels = ReadOnly::new(&f, reply);
        let before = f.store.job(&f.key).unwrap();
        run(&f, &channels, 100);
        run(&f, &channels, 115);
        assert_eq!(budget(&f).status, "stopped");
        assert_eq!(f.store.job(&f.key).unwrap(), before);
        assert!(!run(&f, &channels, 99999));
        assert_eq!(channels.count(), 1);
    }
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Success);
    let before = f.store.job(&f.key).unwrap();
    assert!(run_at(
        &f.store,
        &f.key,
        &f.cfg,
        "different-account",
        &channels,
        100
    )
    .unwrap());
    assert_eq!(budget(&f).status, "stopped");
    assert_eq!(channels.count(), 0);
    assert_eq!(f.store.job(&f.key).unwrap(), before);
}

#[test]
fn journal_tampering_and_absent_or_changed_click_proofs_never_reach_a_browser() {
    for fault in [
        "request",
        "profile",
        "account",
        "wire-hash",
        "request-id",
        "intent",
        "token",
        "prompt",
        "page",
        "adapter",
        "time",
    ] {
        let f = fixture();
        let channels = ReadOnly::new(&f, Reply::Success);
        change_journal(&f, |v| match fault {
            "request" => v["request"]["prompt"] = json!("changed"),
            "profile" => v["profile"] = json!("other"),
            "account" => v["accountKey"] = json!("other"),
            "wire-hash" => v["requestHash"] = json!("f".repeat(64)),
            "request-id" => v["requestId"] = json!("other"),
            "intent" => {
                v.as_object_mut().unwrap().remove("sendIntent");
            }
            "token" => v["sendIntent"]["token"] = json!("other"),
            "prompt" => v["prompt"] = json!("other"),
            "page" => v["sendIntent"]["page"] = json!("OTHER_PAGE"),
            "adapter" => v["ownedSend"]["adapterSha256"] = json!("e".repeat(64)),
            "time" => v["sentAt"] = json!("invalid"),
            _ => unreachable!(),
        });
        let before = f.store.job(&f.key).unwrap();
        let bytes = fs::read(&f.journal).unwrap();
        assert!(run(&f, &channels, 100), "{fault}");
        assert_eq!(budget(&f).status, "stopped", "{fault}");
        assert_eq!(channels.count(), 0);
        assert_eq!(f.store.job(&f.key).unwrap(), before);
        assert_eq!(fs::read(&f.journal).unwrap(), bytes);
    }
}

#[test]
fn changed_frozen_preparation_configuration_and_existing_budget_binding_cannot_reset_recovery() {
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Success);
    run(&f, &channels, 100);
    let mut job = f.store.job(&f.key).unwrap();
    job["prepared"]["request"]["prompt"] = json!("changed");
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET prepared=?1 WHERE id=?2",
            rusqlite::params![job["prepared"].to_string(), f.key],
        )
        .unwrap();
    assert!(run(&f, &channels, 115));
    assert_eq!(budget(&f).status, "stopped");
    assert_eq!(budget(&f).attempts, 0);
    assert_eq!(channels.count(), 0);
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Success);
    let mut cfg = f.cfg.clone();
    cfg.timeout_seconds += 1;
    assert!(run_at(&f.store, &f.key, &cfg, &f.account, &channels, 100).unwrap());
    assert_eq!(budget(&f).status, "stopped");
    assert_eq!(channels.count(), 0);
}

#[test]
fn manual_reconciliation_reuses_full_request_validation_without_resetting_exhausted_auto_budget() {
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Unknown);
    run(&f, &channels, 100);
    for at in [115, 145, 205, 325, 565] {
        run(&f, &channels, at);
    }
    let before_budget = saved_budget(&f.store, &f.key).unwrap().0;
    *channels.reply.lock().unwrap() = Reply::Success;
    super::super::reconcile(&f.store, &f.key, &f.cfg, &f.account, &channels).unwrap();
    assert_eq!(
        f.store.job(&f.key).unwrap()["providerTaskId"],
        "original-owned-request"
    );
    assert_eq!(f.store.job(&f.key).unwrap()["queryAttempts"], 3);
    assert_eq!(f.store.job(&f.key).unwrap()["downloadAttempts"], 2);
    assert_eq!(saved_budget(&f.store, &f.key).unwrap().0, before_budget);
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Success);
    let before = f.store.job(&f.key).unwrap();
    change_journal(&f, |v| {
        v["request"]["prompt"] = json!("different prompt under same ID")
    });
    assert!(super::super::reconcile(&f.store, &f.key, &f.cfg, &f.account, &channels).is_err());
    assert_eq!(channels.count(), 0);
    assert_eq!(f.store.job(&f.key).unwrap(), before);
}

#[test]
fn compare_and_swap_budget_cannot_reserve_the_same_attempt_twice() {
    let f = fixture();
    let channels = ReadOnly::new(&f, Reply::Unknown);
    run(&f, &channels, 100);
    let (old, mut saved) = saved_budget(&f.store, &f.key).unwrap();
    let b = saved.as_mut().unwrap();
    b.attempts = 1;
    b.next_attempt_at = 145;
    assert!(save_budget(&f.store, &f.key, &old, b, "synthetic_read_claim").unwrap());
    assert!(!save_budget(&f.store, &f.key, &old, b, "synthetic_read_claim").unwrap());
    assert_eq!(budget(&f).attempts, 1);
    assert_eq!(channels.count(), 0);
    assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
}

#[test]
fn configuration_failure_is_durably_stopped_without_network_or_background_wait_loop() {
    let mut f = fixture();
    let channels = ReadOnly::new(&f, Reply::Success);
    let before = f.store.job(&f.key).unwrap();
    assert!(stop_opencli_unknown_read(&f.store, &f.key, "FROZEN_CONFIG_UNAVAILABLE").unwrap());
    let stopped = budget(&f);
    assert_eq!(stopped.status, "stopped");
    assert_eq!(stopped.attempts, 0);
    assert_eq!(stopped.last_code, "FROZEN_CONFIG_UNAVAILABLE");
    f.store = Store::open(f.store.root.clone()).unwrap();
    assert!(!opencli_unknown_pending(&f.store, &before).unwrap());
    assert!(!due_at(&f.store, &before, 9999999999).unwrap());
    assert!(opencli_unknown_jobs(&f.store).unwrap().is_empty());
    assert!(!run(&f, &channels, 9999999999));
    assert!(!stop_opencli_unknown_read(&f.store, &f.key, "FROZEN_CONFIG_UNAVAILABLE").unwrap());
    assert_eq!(f.store.job(&f.key).unwrap(), before);
    assert_eq!(channels.count(), 0);
    assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
}
