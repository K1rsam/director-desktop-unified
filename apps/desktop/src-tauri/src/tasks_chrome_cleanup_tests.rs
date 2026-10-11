//! Synthetic cleanup receipts and files only; never opens a browser or account.
use super::*;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

struct Cleanup {
    outcomes: Mutex<VecDeque<Value>>,
    releases: AtomicUsize,
}
impl Cleanup {
    fn new(outcomes: Vec<Value>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into()),
            releases: AtomicUsize::new(0),
        }
    }
}
impl Channels for Cleanup {
    fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> {
        panic!("cleanup never checks catalog")
    }
    fn prepare(
        &self,
        _: &ProviderConfig,
        _: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        panic!("cleanup never prepares")
    }
    fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        panic!("cleanup never submits")
    }
    fn query(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("cleanup never queries")
    }
    fn download(&self, _: &ProviderConfig, _: &str, _: &Path) -> ProviderResult<DownloadReceipt> {
        panic!("cleanup never downloads")
    }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("cleanup never reconciles")
    }
    fn release_completed_tab(
        &self,
        _: &ProviderConfig,
        prepared: &PreparedRequest,
        result: &QueryResult,
    ) -> ProviderResult<Value> {
        assert_eq!(prepared.request.request_id, result.task_id);
        self.releases.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected repeat cleanup");
        if let Some(code) = outcome["error"].as_str() {
            return Err(ProviderError::new(code, "synthetic failure"));
        }
        Ok(outcome)
    }
}
pub(crate) fn completed(store: &Store, task: &str) -> (String, ProviderConfig) {
    let asset = store
        .save_asset(json!({"id":task,"name":task,"variantId":"base","prompt":"synthetic image"}))
        .unwrap();
    let snapshot = json!({"requestId":task,"kind":"image","model":"chatgpt-image","prompt":"synthetic image","ratio":"16:9","resolution":"native","references":[],"options":{},"assetId":task,"variantId":"base","assetVersion":asset["version"]});
    let key = store
        .add_job(
            "image",
            task,
            "chrome",
            "offline-account",
            &snapshot,
            "submitted",
        )
        .unwrap();
    let prepared = PreparedRequest {
        provider: "chrome".into(),
        request: generation_request(&snapshot).unwrap(),
        capability_version: "fixture".into(),
        upload_receipts: vec![],
        wire: json!({"sourceClientId":"owned-hidden-tab","metadata":{"promptSha256":"frozen-hash"}}),
        estimated_cost: None,
        prepared_at: now(),
    };
    store.update_job(&key,"submitted",&json!({"providerTaskId":task,"prepared":prepared,"receipt":{"taskId":task,"frozen":true}})).unwrap();
    let file = store.root.join(format!("{task}.png"));
    image::RgbImage::from_pixel(3, 2, image::Rgb([7, 13, 29]))
        .save(&file)
        .unwrap();
    let media = store.import_media(&file, "image", None, None).unwrap();
    store.attach_download_result(&key, s(&media, "id")).unwrap();
    store.complete_download(&key,&json!({"taskId":task,"resultUrls":["https://offline.test/fixture"],"raw":{"nativeProof":{"status":"matched_complete","proofVersion":2,"requestId":task,"promptSha256":"frozen-hash","conversationId":"frozen-conversation"}}}),&[s(&media,"id").into()]).unwrap();
    let cfg = ProviderConfig {
        kind: "chrome".into(),
        endpoint: None,
        credential_env: None,
        cli_path: None,
        bridge_token_env: None,
        timeout_seconds: 5,
        adapted_models: vec![],
        chrome_binding: None,
    };
    (key, cfg)
}
fn fixture() -> (tempfile::TempDir, Store, String, ProviderConfig) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("offline cleanup", "p").unwrap();
    let (key, cfg) = completed(&store, "original-task");
    (temp, store, key, cfg)
}
fn state(store: &Store, key: &str) -> Value {
    let raw: String = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [cleanup_key(key)],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
fn due(store: &Store, key: &str) {
    let mut value = state(store, key);
    value["nextAttemptAt"] = json!(0);
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE metadata SET value=?2 WHERE key=?1",
            rusqlite::params![cleanup_key(key), value.to_string()],
        )
        .unwrap();
}
fn events(store: &Store, key: &str) -> i64 {
    store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='browser_tab_cleanup'",
            [key],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn busy_completion_survives_reopen_then_confirms_closed_without_generation_or_duplicate_events() {
    let (_temp, store, key, cfg) = fixture();
    let before = store.job(&key).unwrap();
    let dirty: String = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key='changed_at'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let fake = Cleanup::new(vec![
        json!({"status":"retained","reason":"idle_not_proven"}),
        json!({"status":"retained","reason":"idle_not_proven"}),
        json!({"status":"close_requested"}),
        json!({"status":"already_closed"}),
    ]);
    queue_browser_tab_cleanup(&store, &key).unwrap();
    run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
    assert_eq!(state(&store, &key)["status"], "pending");
    assert_eq!(events(&store, &key), 1);
    assert!(due_browser_tab_cleanups(&store).unwrap().is_empty());
    let reopened = Store::open(store.root.clone()).unwrap();
    due(&reopened, &key);
    run_browser_tab_cleanup(&reopened, &key, &cfg, &fake).unwrap();
    assert_eq!(events(&reopened, &key), 1);
    due(&reopened, &key);
    run_browser_tab_cleanup(&reopened, &key, &cfg, &fake).unwrap();
    assert_eq!(state(&reopened, &key)["status"], "pending");
    due(&reopened, &key);
    run_browser_tab_cleanup(&reopened, &key, &cfg, &fake).unwrap();
    assert_eq!(state(&reopened, &key)["status"], "done");
    queue_browser_tab_cleanup(&reopened, &key).unwrap();
    run_browser_tab_cleanup(&reopened, &key, &cfg, &fake).unwrap();
    assert!(due_browser_tab_cleanups(&reopened).unwrap().is_empty());
    assert_eq!(fake.releases.load(Ordering::SeqCst), 4);
    assert_eq!(events(&reopened, &key), 3);
    assert_eq!(reopened.job(&key).unwrap(), before);
    let after_dirty: String = reopened
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key='changed_at'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after_dirty, dirty);
}

#[test]
fn missing_corrupt_and_unlinked_media_never_close_the_page() {
    for failure in ["missing", "corrupt", "unlinked"] {
        let (_temp, store, key, cfg) = fixture();
        let before = store.job(&key).unwrap();
        let media = &before["resultMediaIds"][0];
        let object = store.get("media", media.as_str().unwrap()).unwrap();
        let path = store.verify_media(&object).unwrap();
        if failure == "missing" {
            std::fs::remove_file(path).unwrap();
        } else if failure == "corrupt" {
            std::fs::write(path, b"corrupt").unwrap();
        }
        queue_browser_tab_cleanup(&store, &key).unwrap();
        if failure == "unlinked" {
            store
                .conn()
                .unwrap()
                .execute("DELETE FROM job_media WHERE job_id=?1", [&key])
                .unwrap();
        }
        let fake = Cleanup::new(vec![]);
        run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
        assert_eq!(state(&store, &key)["status"], "retained");
        assert_eq!(fake.releases.load(Ordering::SeqCst), 0);
        assert_eq!(events(&store, &key), 1);
        run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
        assert_eq!(events(&store, &key), 1);
        for field in [
            "status",
            "snapshot",
            "prepared",
            "receipt",
            "providerTaskId",
            "result",
            "queryAttempts",
            "downloadAttempts",
        ] {
            if field != "result" || failure != "unlinked" {
                assert_eq!(
                    store.job(&key).unwrap()[field],
                    before[field],
                    "{failure}/{field}"
                );
            }
        }
    }
}

#[test]
fn ownership_or_conversation_changes_stop_automatic_cleanup_and_unknown_jobs_never_close() {
    for reason in [
        "ownership_changed",
        "conversation_changed",
        "ownership_or_idle_not_proven",
    ] {
        let (_temp, store, key, cfg) = fixture();
        let before = store.job(&key).unwrap();
        let fake = Cleanup::new(vec![json!({"status":"retained","reason":reason})]);
        queue_browser_tab_cleanup(&store, &key).unwrap();
        run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
        assert_eq!(state(&store, &key)["status"], "retained");
        assert!(due_browser_tab_cleanups(&store).unwrap().is_empty());
        assert_eq!(store.job(&key).unwrap(), before);
    }
    let (_temp, store, known, cfg) = fixture();
    let snapshot = store.job(&known).unwrap()["snapshot"].clone();
    let key = store
        .add_job(
            "image",
            "original-task",
            "chrome",
            "offline-account",
            &snapshot,
            "submission_unknown",
        )
        .unwrap();
    queue_browser_tab_cleanup(&store, &key).unwrap();
    assert!(!due_browser_tab_cleanups(&store).unwrap().contains(&key));
    let fake = Cleanup::new(vec![]);
    run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
    assert_eq!(fake.releases.load(Ordering::SeqCst), 0);
}

#[test]
fn legacy_completions_queue_once_while_unproved_or_nonchrome_history_stays_untouched() {
    let (_temp, store, key, cfg) = fixture();
    let before = store.job(&key).unwrap();
    assert_eq!(due_browser_tab_cleanups(&store).unwrap(), vec![key.clone()]);
    assert_eq!(state(&store, &key)["legacyBackfill"], true);
    let fake = Cleanup::new(vec![json!({"status":"closed"})]);
    run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
    assert!(due_browser_tab_cleanups(&store).unwrap().is_empty());
    assert_eq!(store.job(&key).unwrap(), before);
    assert_eq!(events(&store, &key), 1);
    let (other, _) = completed(&store, "unproved-history");
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET result=json_remove(result,'$.raw.nativeProof') WHERE id=?1",
            [&other],
        )
        .unwrap();
    let (nonchrome, _) = completed(&store, "other-provider");
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET provider='dreamina' WHERE id=?1",
            [&nonchrome],
        )
        .unwrap();
    assert!(due_browser_tab_cleanups(&store).unwrap().is_empty());
}

#[test]
fn first_disconnected_observation_is_not_claimed_as_a_confirmed_close() {
    let (_temp, store, key, cfg) = fixture();
    let before = store.job(&key).unwrap();
    queue_browser_tab_cleanup(&store, &key).unwrap();
    let fake = Cleanup::new(vec![json!({"status":"already_closed"})]);
    run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
    assert_eq!(state(&store, &key)["status"], "retained");
    assert_eq!(
        state(&store, &key)["outcome"]["reason"],
        "page_absence_unproven"
    );
    assert_eq!(store.job(&key).unwrap(), before);
    assert!(due_browser_tab_cleanups(&store).unwrap().is_empty());
}

#[test]
fn transport_failures_retry_independently_while_invalid_conversation_identity_stops() {
    let (_temp, store, key, cfg) = fixture();
    let before = store.job(&key).unwrap();
    let fake = Cleanup::new(vec![
        json!({"error":"BRIDGE_OFFLINE"}),
        json!({"error":"BRIDGE_OFFLINE"}),
        json!({"error":"IMAGE_PROVENANCE"}),
    ]);
    queue_browser_tab_cleanup(&store, &key).unwrap();
    run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
    assert_eq!(state(&store, &key)["status"], "pending");
    assert!(
        state(&store, &key)["nextAttemptAt"].as_i64().unwrap() > chrono::Utc::now().timestamp()
    );
    due(&store, &key);
    run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
    assert_eq!(events(&store, &key), 1);
    due(&store, &key);
    run_browser_tab_cleanup(&store, &key, &cfg, &fake).unwrap();
    assert_eq!(state(&store, &key)["status"], "retained");
    assert!(due_browser_tab_cleanups(&store).unwrap().is_empty());
    assert_eq!(events(&store, &key), 2);
    assert_eq!(store.job(&key).unwrap(), before);
}

#[test]
fn legacy_cleanup_projection_has_no_workspace_history_limit_and_does_not_hash_originals() {
    let (_temp, store, key, _cfg) = fixture();
    let before = store.job(&key).unwrap();
    let c = store.conn().unwrap();
    c.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1300) INSERT INTO jobs(id,kind,entity_id,provider,state,task_id,account_id,snapshot,prepared,result,created_at,updated_at) SELECT 'synthetic-history-'||x,j.kind,j.entity_id,j.provider,j.state,j.task_id,j.account_id,j.snapshot,j.prepared,j.result,'2000','2000' FROM n JOIN jobs j ON j.id=?1",[&key]).unwrap();
    c.execute("INSERT INTO job_media(job_id,media_id) SELECT j.id,jm.media_id FROM jobs j JOIN job_media jm ON jm.job_id=?1 WHERE j.id LIKE 'synthetic-history-%'",[&key]).unwrap();
    let media = store
        .get("media", before["resultMediaIds"][0].as_str().unwrap())
        .unwrap();
    let path = store.verify_media(&media).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(due_browser_tab_cleanups(&store).unwrap().len(), 1301);
    assert_eq!(due_browser_tab_cleanups(&store).unwrap().len(), 1301);
    assert_eq!(store.job(&key).unwrap(), before);
    assert_eq!(events(&store, &key), 0);
}
