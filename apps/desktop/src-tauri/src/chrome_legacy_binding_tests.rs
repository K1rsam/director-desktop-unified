//! Isolated legacy queue migration. No host credentials, browser, or network.
use super::*;
use crate::providers::{DownloadReceipt,GenerationRequest,PreparedRequest,QueryResult,SubmitReceipt};
use std::sync::atomic::AtomicUsize;
const PROFILE:&str="b84d4521-56ba-4f84-9948-090f82d639e7";
const OTHER:&str="f913ed04-6d73-4f40-bd74-040418327d79";
#[derive(Default)] struct Fake {submissions:AtomicUsize}
impl Channels for Fake {
    fn capabilities(&self,c:&ProviderConfig)->ProviderResult<Capabilities>{Ok(serde_json::from_value(json!({"provider":c.kind,"version":"offline","source":"offline","checkedAt":now(),"models":[],"canCancel":false,"canReconcileUnknown":false,"raw":{}})).unwrap())}
    fn prepare(&self,c:&ProviderConfig,r:&GenerationRequest)->ProviderResult<PreparedRequest>{
        assert_eq!(c.chrome_binding.as_ref().unwrap().profile_id,PROFILE);
        Ok(PreparedRequest{provider:c.kind.clone(),request:r.clone(),capability_version:"offline".into(),upload_receipts:vec![],wire:json!({"offline":true}),estimated_cost:None,prepared_at:now()})
    }
    fn submit(&self,_:&ProviderConfig,p:&PreparedRequest)->ProviderResult<SubmitReceipt>{self.submissions.fetch_add(1,Ordering::SeqCst);Ok(SubmitReceipt{task_id:format!("offline:{}",p.request.request_id),status:"processing".into(),result_urls:vec![],raw:json!({"offline":true})})}
    fn query(&self,_:&ProviderConfig,_:&str)->ProviderResult<QueryResult>{panic!("no real query")}
    fn download(&self,_:&ProviderConfig,_:&str,_:&Path)->ProviderResult<DownloadReceipt>{panic!("no media")}
    fn reconcile(&self,_:&ProviderConfig,_:&str)->ProviderResult<QueryResult>{panic!("never reconcile")}
}
struct Fixture {_temp:tempfile::TempDir,engine:Arc<Engine>,store:Store,channels:Arc<Fake>,frozen:Value,config:ProviderConfig,account:String}
impl Fixture {
    fn engine(home:PathBuf,channels:Arc<Fake>)->Arc<Engine>{let mut engine=Engine::with_channels(home,channels).unwrap();Arc::get_mut(&mut engine).unwrap().standard_native=true;engine}
    fn new()->Self {
        let temp=tempfile::tempdir().unwrap();let channels=Arc::new(Fake::default());let engine=Self::engine(temp.path().join("home"),channels.clone());
        let store=Store::open(temp.path().join("project")).unwrap();store.create("synthetic migration",&id()).unwrap();
        let mut doc=crate::app_settings::initialize(&engine.home,&json!([]),None).unwrap();
        doc["settings"]["chromeEndpoint"]=json!("http://127.0.0.1:53179");doc["settings"]["chromeAccountId"]=json!("synthetic-account");
        doc["credentialRefs"]["chrome"]=json!(format!("{}:chrome",s(&store.project().unwrap(),"id")));
        atomic_json(&engine.home.join(crate::app_settings::FILE),&doc).unwrap();
        let frozen=engine.effective_settings(&store).unwrap();let (config,account)=engine.config_with_settings(&store,"chrome",&frozen).unwrap();
        let f=Self{_temp:temp,engine,store,channels,frozen,config,account};f.bind(PROFILE,"bundle");f
    }
    fn bind(&self,profile:&str,bundle:&str){self.patch(json!({"chromeProfileId":profile,"chromeProfileEndpoint":"http://127.0.0.1:53179","chromeProfileBundleId":bundle}));}
    fn patch(&self,patch:Value){let doc=crate::app_settings::load(&self.engine.home).unwrap().unwrap();crate::app_settings::save(&self.engine.home,&patch,doc["revision"].as_u64().unwrap()).unwrap();}
    fn job(&self,state:&str)->String {
        let asset=id();self.store.save_asset(json!({"id":asset,"baseAssetId":asset,"variantId":"base","name":"synthetic","kind":"CH","prompt":"synthetic"})).unwrap();
        let a=self.store.active_asset(&asset).unwrap();
        self.store.add_job("image",&asset,"chrome",&self.account,&json!({"requestId":id(),"kind":"image","provider":"chrome","assetId":asset,"variantId":"base","assetVersion":a["version"],"sourceVersion":null,"model":"offline","prompt":"synthetic","ratio":"16:9","resolution":"2K","references":[],"dependencyJobIds":[],"options":{},"autoSubmit":true,"frozenSettings":self.frozen,"providerConfig":self.config}),state).unwrap()
    }
    fn config(&self,key:&str)->Result<(ProviderConfig,String)>{self.engine.generation_config(&self.store,&json!({"id":key}))}
    fn count(&self)->i64{self.store.conn().unwrap().query_row("SELECT COUNT(*) FROM metadata WHERE key LIKE 'legacy_chrome_execution_binding:%'",[],|r|r.get(0)).unwrap()}
}
#[test] fn chrome_legacy_binding_fixes_once_and_survives_reopen_and_profile_switch() {
    let f=Fixture::new();let key=f.job("waiting_dependency");let before=f.store.job(&key).unwrap();
    assert_eq!(f.config(&key).unwrap().0.chrome_binding.unwrap().profile_id,PROFILE);
    f.bind(OTHER,"other-bundle");
    for _ in 0..3 {assert_eq!(f.config(&key).unwrap().0.chrome_binding.unwrap().profile_id,PROFILE);}
    let reopened=Fixture::engine(f.engine.home.clone(),f.channels.clone());let store=Store::open(f.store.root.clone()).unwrap();
    assert_eq!(reopened.generation_config(&store,&json!({"id":key})).unwrap().0.chrome_binding.unwrap().profile_id,PROFILE);
    assert_eq!(store.job(&key).unwrap(),before);assert_eq!(f.count(),1);assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
    let other=f.job("queued");assert_eq!(f.config(&other).unwrap().0.chrome_binding.unwrap().profile_id,OTHER);
}
#[test] fn chrome_legacy_binding_rejects_endpoint_credential_reference_account_and_invalid_bundle() {
    for changed in ["endpoint","credential","account","bundle"] {
        let f=Fixture::new();let key=f.job("queued");let before=f.store.job(&key).unwrap();
        match changed {
            "endpoint"=>{let mut doc=crate::app_settings::load(&f.engine.home).unwrap().unwrap();doc["settings"]["chromeEndpoint"]=json!("http://127.0.0.1:53180");doc["settings"]["chromeProfileEndpoint"]=json!("http://127.0.0.1:53180");atomic_json(&f.engine.home.join(crate::app_settings::FILE),&doc).unwrap();}
            "credential"=>{let mut doc=crate::app_settings::load(&f.engine.home).unwrap().unwrap();doc["credentialRefs"]["chrome"]=json!(format!("global:chrome:{}",id()));atomic_json(&f.engine.home.join(crate::app_settings::FILE),&doc).unwrap();}
            "account"=>f.patch(json!({"chromeAccountId":"different"})),
            _=>{let mut doc=crate::app_settings::load(&f.engine.home).unwrap().unwrap();doc["settings"]["chromeProfileBundleId"]=json!("");atomic_json(&f.engine.home.join(crate::app_settings::FILE),&doc).unwrap();}
        }
        assert!(f.config(&key).is_err(),"{changed}");assert_eq!(f.count(),0);assert_eq!(f.store.job(&key).unwrap(),before);assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
    }
}
#[test] fn chrome_legacy_binding_never_migrates_prepared_task_receipt_unknown_or_prior_dispatch() {
    let f=Fixture::new();
    for evidence in ["prepared","task","receipt","unknown","dispatch","prior_submitting","prior_unknown"] {
        let key=f.job("queued");
        match evidence {
            "prepared"=>f.store.update_job(&key,"queued",&json!({"prepared":{"original":true}})).unwrap(),
            "task"=>f.store.update_job(&key,"queued",&json!({"providerTaskId":"original"})).unwrap(),
            "receipt"=>f.store.update_job(&key,"queued",&json!({"receipt":{"original":true}})).unwrap(),
            "unknown"=>f.store.update_job(&key,"submission_unknown",&json!({})).unwrap(),
            "dispatch"=>f.store.event(&key,"chrome_dispatch_started",&json!({"original":true})).unwrap(),
            "prior_submitting"=>f.store.event(&key,"state_claim",&json!({"from":"ready","state":"submitting"})).unwrap(),
            _=>f.store.event(&key,"state",&json!({"state":"submission_unknown"})).unwrap(),
        }
        let before=f.store.job(&key).unwrap();assert!(f.config(&key).unwrap().0.chrome_binding.is_none(),"{evidence}");assert_eq!(f.store.job(&key).unwrap(),before);
    }
    assert_eq!(f.count(),0);assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
}
#[test] fn chrome_legacy_binding_keeps_frozen_request_and_does_not_duplicate_submit() {
    let f=Fixture::new();let key=f.job("queued");let frozen=f.store.job(&key).unwrap()["snapshot"].clone();
    let (config,account)=f.config(&key).unwrap();
    tasks::run_generation(&f.store,&key,&config,&account,f.channels.as_ref()).unwrap();
    assert_eq!(f.store.job(&key).unwrap()["status"],"submitted");
    f.bind(OTHER,"other-bundle");
    for _ in 0..2 {let (config,account)=f.config(&key).unwrap();tasks::run_generation(&f.store,&key,&config,&account,f.channels.as_ref()).unwrap();}
    assert_eq!(f.channels.submissions.load(Ordering::SeqCst),1);assert_eq!(f.store.job(&key).unwrap()["snapshot"],frozen);assert_eq!(f.count(),1);
}
#[test] fn chrome_legacy_binding_accepts_original_project_credential_reference_after_global_migration() {
    let mut f=Fixture::new();f.frozen.as_object_mut().unwrap().remove(crate::app_settings::SCOPE);
    f.store.save_settings(&f.frozen).unwrap();
    (f.config,f.account)=f.engine.config_with_settings(&f.store,"chrome",&f.frozen).unwrap();
    let key=f.job("queued");let before=f.store.job(&key).unwrap();
    assert_eq!(f.config(&key).unwrap().0.chrome_binding.unwrap().profile_id,PROFILE);
    assert_eq!(f.store.job(&key).unwrap(),before);assert_eq!(f.count(),1);assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
}
#[test] fn chrome_legacy_binding_keeps_identity_when_direct_parent_reference_is_resolved() {
    let f=Fixture::new();let key=f.job("waiting_dependency");
    assert_eq!(f.config(&key).unwrap().0.chrome_binding.unwrap().profile_id,PROFILE);
    let mut snapshot=f.store.job(&key).unwrap()["snapshot"].clone();
    snapshot["references"]=json!([{"mediaId":"offline-parent","assetId":"offline-parent-asset","path":"offline-parent.png","sha256":"offline-parent-hash"}]);
    f.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1,state='queued' WHERE id=?2",rusqlite::params![snapshot.to_string(),key]).unwrap();
    assert_eq!(f.config(&key).unwrap().0.chrome_binding.unwrap().profile_id,PROFILE);
    snapshot["prompt"]=json!("unexpected changed prompt");
    f.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",rusqlite::params![snapshot.to_string(),key]).unwrap();
    assert!(f.config(&key).is_err());assert_eq!(f.count(),1);assert_eq!(f.channels.submissions.load(Ordering::SeqCst),0);
}
