//! In-memory provider acceptance: no browser, account or generation service.
use super::*;
use crate::providers::{DownloadReceipt, GenerationRequest, PreparedRequest, QueryResult, SubmitReceipt};
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Simulated {
    submissions: AtomicUsize,
    queries: AtomicUsize,
}
impl Channels for Simulated {
    fn capabilities(&self, cfg: &ProviderConfig) -> ProviderResult<Capabilities> {
        Ok(serde_json::from_value(json!({"provider":cfg.kind,"version":"offline-chrome-queue",
            "source":"in-memory","checkedAt":now(),"models":[],"canCancel":false,
            "canReconcileUnknown":false,"raw":{}})).unwrap())
    }
    fn prepare(&self, cfg: &ProviderConfig, request: &GenerationRequest) -> ProviderResult<PreparedRequest> {
        Ok(PreparedRequest { provider:cfg.kind.clone(), request:request.clone(),
            capability_version:"offline-chrome-queue".into(), upload_receipts:vec![],
            wire:json!({"offline":true}), estimated_cost:None, prepared_at:now() })
    }
    fn submit(&self, _: &ProviderConfig, prepared: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        Ok(SubmitReceipt { task_id:format!("offline:{}",prepared.request.request_id),
            status:"processing".into(), result_urls:vec![], raw:json!({"offline":true}) })
    }
    fn query(&self, _: &ProviderConfig, task: &str) -> ProviderResult<QueryResult> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        Ok(QueryResult { task_id:task.into(), status:"processing".into(), result_urls:vec![],
            error:None, actual_cost:None, raw:json!({"offline":true}) })
    }
    fn download(&self, _: &ProviderConfig, _: &str, _: &Path) -> ProviderResult<DownloadReceipt> {
        panic!("this synthetic queue test must not fetch media")
    }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("unknown submissions must never be guessed")
    }
}
struct Fixture {
    _temp:tempfile::TempDir,
    engine:Arc<Engine>,
    channels:Arc<Simulated>,
}
impl Fixture {
    fn new() -> Self {
        let temp=tempfile::tempdir().unwrap();
        let channels=Arc::new(Simulated::default());
        let engine=Engine::with_channels(temp.path().join("home"),channels.clone()).unwrap();
        Self { _temp:temp,engine,channels }
    }
    fn project(&self, title:&str) -> Store {
        let project=self.engine.dispatch(json!({"action":"projects.create","title":title})).unwrap();
        let store=self.engine.store(s(&project,"id")).unwrap();
        store.save_settings(&json!({"chromeEndpoint":"http://offline.invalid","chromeAccountId":"offline",
            "dreaminaPath":"offline-never-executed","dreaminaAccountId":"offline"})).unwrap();
        for key in ["catalog_attempt:dreamina","catalog_attempt:bh","catalog_attempt:chrome","backed_up_at"] {
            store.conn().unwrap().execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,now()]).unwrap();
        }
        store
    }
    fn job(&self, store:&Store, provider:&str, state:&str) -> String {
        let asset=id();
        store.save_asset(json!({"id":asset,"baseAssetId":asset,"variantId":"base","name":"offline","kind":"CH","prompt":"offline"})).unwrap();
        let appearance=store.active_asset(&asset).unwrap();
        let (_,account)=self.engine.config(store,provider).unwrap();
        store.add_job("image",&asset,provider,&account,&json!({"requestId":id(),"kind":"image",
            "provider":provider,"assetId":asset,"variantId":"base","assetVersion":appearance["version"],
            "sourceVersion":null,"model":"offline-image","prompt":"offline","ratio":"16:9","resolution":"2K",
            "references":[],"dependencyJobIds":[],"options":{},"autoSubmit":true}),state).unwrap()
    }
    fn tick(&self) {
        self.engine.tick().unwrap();
        assert!(self.engine.await_channel_shutdown(8));
    }
}

#[test]
fn chrome_queue_preserves_original_requests_and_holds_all_unresolved_states() {
    let f=Fixture::new(); let store=f.project("state coverage");
    let pending=f.job(&store,"chrome","queued");
    let original=store.job(&pending).unwrap()["snapshot"].clone();
    for state in ["preparing","submitting","submitted","processing","succeeded","query_failed",
        "download_failed","downloading","submission_unknown"] {
        let blocker=f.job(&store,"chrome",state);
        assert!(!f.engine.chrome_image_queue_ready(&store,&pending).unwrap(),"{state}");
        assert_eq!(store.job(&pending).unwrap()["status"],"queued");
        assert_eq!(store.job(&pending).unwrap()["snapshot"],original);
        store.update_job(&blocker,"downloaded",&json!({})).unwrap();
    }
    for state in ["auth_required","failed","recovery_paused","queued","ready"] {
        let blocker=f.job(&store,"chrome",state);
        store.update_job(&blocker,state,&json!({"receipt":{"original":true}})).unwrap();
        assert!(!f.engine.chrome_image_queue_ready(&store,&pending).unwrap(),"receipt {state}");
        store.update_job(&blocker,"downloaded",&json!({})).unwrap();
    }
    let unrelated=f.job(&store,"chrome","auth_required");
    assert!(f.engine.chrome_image_queue_ready(&store,&pending).unwrap());
    assert_eq!(store.job(&unrelated).unwrap()["status"],"auth_required");
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
}

#[test]
fn chrome_cross_project_queue_is_durable_and_queries_are_not_blocked() {
    let f=Fixture::new(); let a=f.project("active"); let b=f.project("waiting");
    let active=f.job(&a,"chrome","submitted");
    a.update_job(&active,"submitted",&json!({"providerTaskId":"offline:original","receipt":{"original":true}})).unwrap();
    let queued=f.job(&b,"chrome","queued");
    let before=b.job(&queued).unwrap();
    let event_count: i64=b.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1",[&queued],|r|r.get(0)).unwrap();
    f.tick();
    assert_eq!(b.job(&queued).unwrap()["status"],"queued");
    assert!(f.channels.queries.load(Ordering::SeqCst)>0);
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
    let reopened=Engine::with_channels(f.engine.home.clone(),f.channels.clone()).unwrap();
    assert!(!reopened.chrome_image_queue_ready(&b,&queued).unwrap());
    assert!(!reopened.chrome_image_queue_ready(&b,&queued).unwrap());
    assert_eq!(b.job(&queued).unwrap()["updatedAt"],before["updatedAt"]);
    assert_eq!(b.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1",[&queued],|r|r.get::<_,i64>(0)).unwrap(),event_count);
    // Only original download completion releases the next image.
    a.update_job(&active,"downloaded",&json!({})).unwrap();
    b.update_job(&queued,"queued",&json!({"nextAttemptAt":0})).unwrap();
    f.tick();
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),1);
    assert_eq!(b.job(&queued).unwrap()["status"],"submitted");
}

#[test]
fn archived_original_chrome_unknown_holds_new_chrome_but_not_cli() {
    let f=Fixture::new(); let a=f.project("archive"); let b=f.project("active");
    let original=f.job(&a,"chrome","submission_unknown");
    a.update_job(&original,"submission_unknown",&json!({"receipt":{"original":true}})).unwrap();
    let mut registry=f.engine.registry().unwrap();
    registry[0]["archivedAt"]=json!(now());
    atomic_json(&f.engine.home.join("projects.json"),&registry).unwrap();
    let queued=f.job(&b,"chrome","queued");
    let cli=f.job(&b,"dreamina","queued");
    f.tick();
    assert_eq!(b.job(&queued).unwrap()["status"],"queued");
    assert_eq!(b.job(&cli).unwrap()["status"],"submitted");
    assert_eq!(a.job(&original).unwrap()["status"],"submission_unknown");
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),1);
}

#[test]
fn lock_time_recheck_observes_remote_submission_after_initial_gate() {
    let f=Fixture::new(); let a=f.project("first"); let b=f.project("next");
    let queued=f.job(&b,"chrome","queued");
    assert!(f.engine.chrome_image_queue_ready(&b,&queued).unwrap());
    let active=f.job(&a,"chrome","submitting");
    let _guard=f.engine.acquire_keys(vec!["channel:chrome".into()]).unwrap();
    assert!(!f.engine.chrome_image_queue_ready(&b,&queued).unwrap());
    assert_eq!(a.job(&active).unwrap()["status"],"submitting");
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
}

#[test]
fn two_projects_submit_only_one_chrome_image_until_original_download_completion() {
    let f=Fixture::new(); let a=f.project("first batch"); let b=f.project("second batch");
    let first=f.job(&a,"chrome","queued"); let second=f.job(&b,"chrome","queued");
    f.tick();
    f.tick();
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),1);
    let (waiting,waiting_job)=if !s(&a.job(&first).unwrap(),"providerTaskId").is_empty() {
        (&b,&second)
    } else { (&a,&first) };
    assert_eq!(waiting.job(waiting_job).unwrap()["status"],"queued");
    assert!(waiting.job(waiting_job).unwrap()["error"].is_null());
}

#[test]
fn chrome_gate_has_no_history_cap_and_identity_failure_keeps_queue_waiting() {
    let f=Fixture::new(); let a=f.project("long history"); let b=f.project("waiting");
    let original=f.job(&a,"chrome","submission_unknown");
    // More newer records than Store::jobs' bounded UI history, in temporary DB only.
    a.conn().unwrap().execute(r#"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<2100)
        INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at)
        SELECT 'history-'||x,'image','history','chrome','downloaded','offline','{}','2099','2099' FROM n"#,[]).unwrap();
    let queued=f.job(&b,"chrome","queued");
    assert!(!f.engine.chrome_image_queue_ready(&b,&queued).unwrap());
    assert_eq!(a.job(&original).unwrap()["status"],"submission_unknown");
    let mut registry=f.engine.registry().unwrap(); registry[0]["id"]=json!("wrong-identity");
    atomic_json(&f.engine.home.join("projects.json"),&registry).unwrap();
    assert!(!f.engine.chrome_image_queue_ready(&b,&queued).unwrap());
    let job=b.job(&queued).unwrap();
    assert_eq!(job["status"],"queued");
    assert!(s(&job,"error").contains("登记身份不一致"));
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
}
