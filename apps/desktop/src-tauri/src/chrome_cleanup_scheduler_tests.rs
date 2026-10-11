use super::*;
use crate::providers::{
    DownloadReceipt, GenerationRequest, PreparedRequest, QueryResult, SubmitReceipt,
};
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct CleanupOnly {
    calls: AtomicUsize,
}
impl Channels for CleanupOnly {
    fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> {
        panic!("no catalog in cleanup test")
    }
    fn prepare(
        &self,
        _: &ProviderConfig,
        _: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        panic!("cleanup cannot prepare")
    }
    fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        panic!("cleanup cannot generate")
    }
    fn query(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("cleanup cannot query")
    }
    fn download(&self, _: &ProviderConfig, _: &str, _: &Path) -> ProviderResult<DownloadReceipt> {
        panic!("cleanup cannot download")
    }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("cleanup cannot reconcile")
    }
    fn release_completed_tab(
        &self,
        _: &ProviderConfig,
        _: &PreparedRequest,
        _: &QueryResult,
    ) -> ProviderResult<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"status":"closed"}))
    }
}
#[test]
fn scheduler_cleanup_uses_original_channel_job_and_project_leases_without_new_media_operations() {
    let temp = tempfile::tempdir().unwrap();
    let channels = Arc::new(CleanupOnly::default());
    let engine = Engine::with_channels(temp.path().join("home"), channels.clone()).unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"offline cleanup scheduler"}))
        .unwrap();
    let store = engine.store(s(&project, "id")).unwrap();
    store.save_settings(&json!({"chromeEndpoint":"http://offline.invalid","chromeAccountId":"offline","dreaminaPath":"offline-never-executed","dreaminaAccountId":"offline"})).unwrap();
    for key in [
        "catalog_attempt:dreamina",
        "catalog_attempt:bh",
        "catalog_attempt:chrome",
        "backed_up_at",
    ] {
        store.conn().unwrap().execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,now()]).unwrap();
    }
    let (key, _) = tasks::chrome_cleanup_tests::completed(&store, "original-task");
    let before = store.job(&key).unwrap();
    let channel = engine.acquire_keys(vec!["channel:chrome".into()]).unwrap();
    engine.tick().unwrap();
    assert_eq!(channels.calls.load(Ordering::SeqCst), 0);
    drop(channel);
    let job = engine.acquire_keys(vec![format!("job:{key}")]).unwrap();
    engine.tick().unwrap();
    assert_eq!(channels.calls.load(Ordering::SeqCst), 0);
    drop(job);
    engine.tick().unwrap();
    assert!(engine.await_channel_shutdown(8));
    assert_eq!(channels.calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.job(&key).unwrap(), before);
    engine.tick().unwrap();
    assert!(engine.await_channel_shutdown(8));
    assert_eq!(channels.calls.load(Ordering::SeqCst), 1);
    let restored = Engine::with_channels(temp.path().join("home"), channels.clone()).unwrap();
    restored.tick().unwrap();
    assert!(restored.await_channel_shutdown(8));
    assert_eq!(channels.calls.load(Ordering::SeqCst), 1);
}
