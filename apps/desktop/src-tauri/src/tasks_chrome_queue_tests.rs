use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fake {
    store: Store,
    key: String,
    releases: AtomicUsize,
    busy: bool,
    download_fail: bool,
}
impl Channels for Fake {
    fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> { panic!("no upstream") }
    fn prepare(&self, _: &ProviderConfig, _: &GenerationRequest) -> ProviderResult<PreparedRequest> {
        assert!(self.busy);Err(ProviderError::new("BRIDGE_BUSY","browser queue wait"))
    }
    fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> { panic!("never submit") }
    fn query(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> { panic!("never query") }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> { panic!("never reconcile") }
    fn download(&self, _: &ProviderConfig, _: &str, path: &Path) -> ProviderResult<DownloadReceipt> {
        if self.download_fail { return Err(ProviderError::new("DOWNLOAD_FAILED","fixture")); }
        image::RgbImage::from_pixel(3,2,image::Rgb([1,2,3])).save(path).unwrap();
        let (sha256,bytes)=crate::store::hash_file(path).unwrap();
        Ok(DownloadReceipt{path:path.into(),sha256,bytes,mime:Some("image/png".into())})
    }
    fn release_completed_tab(&self, _: &ProviderConfig, _: &PreparedRequest, result: &QueryResult) -> ProviderResult<Value> {
        self.releases.fetch_add(1,Ordering::SeqCst);
        let job=self.store.job(&self.key).unwrap();
        assert_eq!(job["status"],"downloaded");assert_eq!(job["providerTaskId"],result.task_id);
        let media=job["resultMediaIds"].as_array().unwrap();assert_eq!(media.len(),1);
        self.store.verify_media(&self.store.get("media",media[0].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(self.store.adopted("asset","base").unwrap().as_deref(),media[0].as_str());
        Err(ProviderError::new("BRIDGE_OFFLINE","cleanup fails after media commit"))
    }
}
fn fixture(state: &str) -> (tempfile::TempDir, Store, String, ProviderConfig, QueryResult) {
    let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();
    store.create("offline chrome","p").unwrap();
    let asset=store.save_asset(json!({"id":"asset","variantId":"base","name":"fixture","prompt":"fixture"})).unwrap();
    let snapshot=json!({"requestId":"original-task","kind":"image","model":"chatgpt-image","prompt":"fixture",
        "ratio":"16:9","resolution":"native","references":[],"options":{"count":1},"assetId":"asset",
        "variantId":"base","assetVersion":asset["version"],"autoSubmit":true});
    let key=store.add_job("image","asset","chrome","offline-account",&snapshot,state).unwrap();
    let request=generation_request(&snapshot).unwrap();
    let p=PreparedRequest{provider:"chrome".into(),request,capability_version:"fixture".into(),
        upload_receipts:vec![],wire:json!({"sourceClientId":"original-tab"}),estimated_cost:None,prepared_at:now()};
    if state!="queued" {store.update_job(&key,state,&json!({"prepared":p,"providerTaskId":"original-task","receipt":{"original":true}})).unwrap();}
    let cfg=ProviderConfig{kind:"chrome".into(),endpoint:None,credential_env:None,cli_path:None,bridge_token_env:None,timeout_seconds:5,adapted_models:vec![]};
    let result=QueryResult{task_id:"original-task".into(),status:"succeeded".into(),result_urls:vec!["fixture".into()],error:None,actual_cost:None,raw:json!({"original":true})};
    (temp,store,key,cfg,result)
}
#[test]
fn browser_busy_preflight_waits_without_submission_or_recovery_budget() {
    let (_temp,store,key,cfg,_)=fixture("queued");
    let fake=Fake{store:store.clone(),key:key.clone(),releases:AtomicUsize::new(0),busy:true,download_fail:false};
    let before=store.job(&key).unwrap();run_generation(&store,&key,&cfg,"offline-account",&fake).unwrap();
    let after=store.job(&key).unwrap();assert_eq!(after["status"],"queued");
    assert!(after["nextAttemptAt"].as_i64().unwrap()>chrono::Utc::now().timestamp());
    for field in ["snapshot","prepared","receipt","providerTaskId","queryAttempts","downloadAttempts"] {assert_eq!(after[field],before[field],"{field}");}
}
#[test]
fn completed_tab_cleanup_runs_after_media_and_adoption_commit_failure_is_nonfatal() {
    let (_temp,store,key,cfg,result)=fixture("succeeded");
    let fake=Fake{store:store.clone(),key:key.clone(),releases:AtomicUsize::new(0),busy:false,download_fail:false};
    let before=store.job(&key).unwrap();download_result(&store,&key,&cfg,&fake,&result).unwrap();
    let after=store.job(&key).unwrap();assert_eq!(after["status"],"downloaded");assert_eq!(fake.releases.load(Ordering::SeqCst),1);
    for field in ["snapshot","prepared","receipt","providerTaskId"] {assert_eq!(after[field],before[field],"{field}");}
    download_result(&store,&key,&cfg,&fake,&result).unwrap();assert_eq!(fake.releases.load(Ordering::SeqCst),1);
}
#[test]
fn failed_download_keeps_original_page_for_recovery() {
    let (_temp,store,key,cfg,result)=fixture("succeeded");
    let fake=Fake{store:store.clone(),key:key.clone(),releases:AtomicUsize::new(0),busy:false,download_fail:true};
    download_result(&store,&key,&cfg,&fake,&result).unwrap();
    assert_eq!(store.job(&key).unwrap()["status"],"download_failed");assert_eq!(fake.releases.load(Ordering::SeqCst),0);
}
