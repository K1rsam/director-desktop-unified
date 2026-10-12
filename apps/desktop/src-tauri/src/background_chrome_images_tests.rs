//! Synthetic databases/channels only. No browser, account, SDK or video calls.
use super::*;
use crate::providers::{DownloadReceipt,GenerationRequest,PreparedRequest,QueryResult,SubmitReceipt};
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Fake {
    submits:Mutex<Vec<GenerationRequest>>,
    queries:AtomicUsize,
    downloads:AtomicUsize,
    unknown:AtomicBool,
    fail_download:AtomicBool,
    pending:AtomicBool,
}
impl Channels for Fake {
    fn capabilities(&self,c:&ProviderConfig)->ProviderResult<Capabilities> {
        assert_eq!(c.kind,"chrome");
        Ok(serde_json::from_value(json!({"provider":"chrome","version":"offline/v1","source":"synthetic","checkedAt":now(),
            "models":[{"id":"chatgpt-image","kind":"image","adapted":true,"ratios":["1:1","16:9"],"resolutions":["2K"],"images":8,"videos":0,"audios":0,"requiredImages":0,"raw":{}}],
            "canCancel":false,"canReconcileUnknown":false,"raw":{}})).unwrap())
    }
    fn prepare(&self,c:&ProviderConfig,r:&GenerationRequest)->ProviderResult<PreparedRequest> {
        assert_eq!(c.kind,"chrome");assert_eq!(r.kind,"image");
        Ok(PreparedRequest{provider:"chrome".into(),request:r.clone(),capability_version:"offline/v1".into(),
            upload_receipts:vec![],wire:json!({"sourceClientId":"owned-synthetic-tab","metadata":{"promptSha256":pipeline::digest(&r.prompt)}}),estimated_cost:None,prepared_at:now()})
    }
    fn submit(&self,c:&ProviderConfig,p:&PreparedRequest)->ProviderResult<SubmitReceipt> {
        assert_eq!(c.kind,"chrome");assert_eq!(p.request.kind,"image");
        self.submits.lock().unwrap().push(p.request.clone());
        if self.unknown.load(Ordering::SeqCst) {return Err(ProviderError::uncertain("synthetic lost submit response"));}
        Ok(SubmitReceipt{task_id:p.request.request_id.clone(),status:"submitted".into(),result_urls:vec![],raw:json!({"synthetic":true})})
    }
    fn query(&self,c:&ProviderConfig,id:&str)->ProviderResult<QueryResult> {
        assert_eq!(c.kind,"chrome");self.queries.fetch_add(1,Ordering::SeqCst);
        if self.pending.load(Ordering::SeqCst) {return Ok(QueryResult{task_id:id.into(),status:"processing".into(),result_urls:vec![],error:None,actual_cost:None,raw:json!({"synthetic":true})});}
        Ok(QueryResult{task_id:id.into(),status:"succeeded".into(),result_urls:vec![format!("https://fixture.invalid/{id}.png")],error:None,actual_cost:None,raw:json!({"synthetic":true})})
    }
    fn download(&self,c:&ProviderConfig,_:&str,path:&Path)->ProviderResult<DownloadReceipt> {
        assert_eq!(c.kind,"chrome");self.downloads.fetch_add(1,Ordering::SeqCst);
        if self.fail_download.load(Ordering::SeqCst) {return Err(ProviderError::new("SYNTHETIC_DOWNLOAD","offline download failed"));}
        image::RgbImage::from_pixel(3,2,image::Rgb([3,7,13])).save(path).unwrap();
        let (sha,bytes)=crate::store::hash_file(path).unwrap();
        Ok(DownloadReceipt{path:path.into(),sha256:sha,bytes,mime:Some("image/png".into())})
    }
    fn reconcile(&self,_:&ProviderConfig,_:&str)->ProviderResult<QueryResult> {panic!("unknown submissions are never automatically reconciled/resubmitted");}
    fn release_completed_tab(&self,c:&ProviderConfig,_:&PreparedRequest,_:&QueryResult)->ProviderResult<Value> {
        assert_eq!(c.kind,"chrome");Ok(json!({"status":"closed"}))
    }
}
struct Fixture { _temp:tempfile::TempDir, engine:Arc<Engine>, store:Store, fake:Arc<Fake>, request:Request }
impl Fixture {
    fn new(ids:&[(&str,Option<&str>)])->Self {
        let temp=tempfile::tempdir().unwrap();let fake=Arc::new(Fake::default());
        let engine=make_engine(temp.path().join("home"),fake.clone(),false).unwrap();
        let project=engine.dispatch(json!({"action":"projects.create","title":"synthetic background Chrome"})).unwrap();
        let pid=s(&project,"id").to_owned();let store=engine.store(&pid).unwrap();
        store.save_source(serde_json::to_value(pipeline::prepare_source("第1集\n场景：门口\n甲：走吧。\n").unwrap()).unwrap()).unwrap();
        store.save_settings(&json!({"chromeEndpoint":"http://127.0.0.1:1","chromeAccountId":"offline","imageProductionPolicy":"automatic","imageRatio":"1:1","imageResolution":"2K"})).unwrap();
        let mut assets=vec![];
        for (id,parent) in ids {
            let mut a=json!({"id":id,"variantId":id,"baseAssetId":"synthetic-subject","name":id,"kind":"CH","prompt":"完整合成人物提示词",
                "preparation":{"mode":if parent.is_some(){"DERIVE"}else{"REFERENCE"}}});
            if let Some(parent)=parent {a["sourceAssetId"]=json!(parent);}
            let a=store.save_asset(a).unwrap();
            assets.push(AssetBinding{asset_id:(*id).into(),variant_id:(*id).into(),asset_version:a["version"].as_i64().unwrap()});
        }
        let source=store.source().unwrap();
        let request=Request{schema_version:CONTRACT.into(),request_id:id(),home:engine.home.clone(),project_id:pid,
            source_sha256:s(&source,"sha256").into(),source_version:source["version"].as_i64().unwrap(),assets,
            selected_asset_ids:ids.iter().map(|(id,_)|(*id).into()).collect(),model:"chatgpt-image".into(),image_production_policy:"automatic".into(),
            report_path:temp.path().join("private-report.json"),stop_path:temp.path().join("stop.flag"),max_runtime_seconds:2,poll_interval_ms:250,resume_job_ids:vec![]};
        Self{_temp:temp,engine,store,fake,request}
    }
    fn queue(&self)->Journal {prepare_queue(&self.engine,&self.request,&self.store).unwrap()}
    fn step(&self,journal:&Journal)->bool {
        // Synthetic channels complete immediately; remove only fixture's timing delay.
        self.store.conn().unwrap().execute("UPDATE jobs SET next_attempt_at=0 WHERE provider='chrome'",[]).unwrap();
        selected_step(&self.engine,&self.request,&self.store,journal).unwrap()
    }
    fn count(&self)->usize {self.fake.submits.lock().unwrap().len()}
}

#[test]
fn background_chrome_three_layers_download_adopt_direct_parent_and_reopen_zero_dispatch() {
    let f=Fixture::new(&[("root",None),("suit",Some("root")),("injured",Some("suit")),("independent",None)]);
    let j=f.queue();assert_eq!(j.jobs.len(),4);
    for _ in 0..16 {f.step(&j);}
    assert_eq!(f.count(),4);
    for target in &j.jobs {let job=f.store.job(s(target,"jobId")).unwrap();assert_eq!(job["status"],"downloaded");assert!(f.store.adopted(s(&job,"entityId"),s(&job["snapshot"],"variantId")).unwrap().is_some());}
    let sent=f.fake.submits.lock().unwrap();
    assert!(sent[0].references.is_empty());
    let suit=j.jobs.iter().find(|j|j["assetId"]=="suit").unwrap();
    let suit_job=f.store.job(s(suit,"jobId")).unwrap();
    let injured=j.jobs.iter().find(|j|j["assetId"]=="injured").unwrap();
    let injured_job=f.store.job(s(injured,"jobId")).unwrap();
    assert_eq!(injured_job["snapshot"]["references"][0]["mediaId"],suit_job["resultMediaIds"][0]);
    drop(sent);
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),0);
    assert_eq!(f.count(),4,"reopen uses journal rather than full-batch requeue");
    let report:Value=serde_json::from_slice(&fs::read(&f.request.report_path).unwrap()).unwrap();assert_eq!(report["status"],"completed");
}

#[test]
fn background_chrome_unknown_holds_other_new_work_and_reopen_never_resubmits() {
    let f=Fixture::new(&[("first",None),("second",None)]);let j=f.queue();
    f.fake.unknown.store(true,Ordering::SeqCst);assert!(f.step(&j));assert!(!f.step(&j));
    assert_eq!(f.count(),1);assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),2);assert_eq!(f.count(),1);
    assert!(j.jobs.iter().any(|j|f.store.job(s(j,"jobId")).unwrap()["status"]=="submission_unknown"));
}

#[test]
fn background_chrome_stop_before_queue_is_zero_jobs_and_zero_calls() {
    let f=Fixture::new(&[("root",None)]);fs::write(&f.request.stop_path,b"stop").unwrap();
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),3);assert!(f.store.jobs().unwrap().is_empty());assert_eq!(f.count(),0);
}

#[test]
fn background_chrome_source_asset_or_request_drift_rejected_before_allocation() {
    for field in ["source","sourceVersion","assetVersion","variant","project","model"] {
        let mut f=Fixture::new(&[("root",None)]);
        match field {"source"=>f.request.source_sha256="0".repeat(64),"sourceVersion"=>f.request.source_version+=1,
            "assetVersion"=>f.request.assets[0].asset_version+=1,"variant"=>f.request.assets[0].variant_id="changed".into(),
            "project"=>f.request.project_id="changed".into(),_=>f.request.model="video-model".into()};
        assert!(run_owned(f.engine.clone(),&f.request).is_err(),"{field}");assert_eq!(f.count(),0);assert!(f.store.jobs().unwrap().is_empty());
    }
}

#[test]
fn background_chrome_freezes_snapshot_except_proven_parent_references() {
    for field in ["prompt","model","sourceVersion","requestId","dependencyJobIds","frozenSettings","assetVersion"] {
        let f=Fixture::new(&[("root",None)]);let j=f.queue();let key=s(&j.jobs[0],"jobId");
        let mut job=f.store.job(key).unwrap();job["snapshot"][field]=json!("tampered");
        f.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",rusqlite::params![job["snapshot"].to_string(),key]).unwrap();
        assert!(selected_step(&f.engine,&f.request,&f.store,&j).is_err(),"{field}");assert_eq!(f.count(),0);
    }
}

#[test]
fn background_chrome_preparing_restart_retains_unknown_tab_without_reprepare() {
    let f=Fixture::new(&[("root",None)]);let j=f.queue();let key=s(&j.jobs[0],"jobId");
    f.store.update_job(key,"preparing",&json!({})).unwrap();
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),2);assert_eq!(f.count(),0);
    assert_eq!(f.store.job(key).unwrap()["status"],"recovery_paused");
}

#[test]
fn background_chrome_submitting_restart_without_receipt_is_unknown_zero_resubmit() {
    let f=Fixture::new(&[("root",None)]);let j=f.queue();let key=s(&j.jobs[0],"jobId");
    f.store.update_job(key,"submitting",&json!({})).unwrap();
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),2);assert_eq!(f.count(),0);
    assert_eq!(f.store.job(key).unwrap()["status"],"submission_unknown");
}

#[test]
fn background_chrome_queue_intent_crash_discovers_exact_jobs_and_never_full_requeues() {
    let f=Fixture::new(&[("root",None)]);let mut j=f.queue();j.phase="queue_intent".into();j.jobs.clear();
    atomic_json(&journal_file(&f.request),&serde_json::to_value(&j).unwrap()).unwrap();
    let recovered=f.queue();assert_eq!(recovered.jobs.len(),1);assert_eq!(f.store.jobs().unwrap().len(),1);
    assert_eq!(recovered.phase,"interrupted_queue_intent");assert_eq!(f.count(),0);
}

#[test]
fn background_chrome_foreign_provider_existing_hold_and_job_drift_cannot_be_adopted() {
    let mut f=Fixture::new(&[("root",None)]);let j=f.queue();let key=s(&j.jobs[0],"jobId").to_owned();
    f.request.resume_job_ids.push(key.clone());
    // New manifest may not mutate its existing journal identity to bypass original ownership.
    assert!(f.queue_safe().is_err());
    f.store.conn().unwrap().execute("UPDATE jobs SET provider='dreamina' WHERE id=?1",[&key]).unwrap();
    assert!(selected_step(&f.engine,&f.request,&f.store,&j).is_err());assert_eq!(f.count(),0);
}
impl Fixture {fn queue_safe(&self)->Result<Journal>{prepare_queue(&self.engine,&self.request,&self.store)}}

#[test]
fn background_chrome_constructor_uses_global_settings_but_never_recovers_other_projects_or_grant() {
    let f=Fixture::new(&[("root",None)]);
    let other=f.engine.dispatch(json!({"action":"projects.create","title":"untouched other project"})).unwrap();let other=f.engine.store(s(&other,"id")).unwrap();
    let key=other.add_job("text","original","codex_sdk","offline",&json!({}),"submitting").unwrap();
    fs::write(f.engine.home.join("native-cli-acceptance.json"),b"invalid grant must never be read").unwrap();
    atomic_json(&f.engine.home.join(crate::app_settings::FILE),&json!({"schemaVersion":"app-settings/v1","revision":3,
        "settings":{"chromeEndpoint":"http://127.0.0.1:9999","chromeAccountId":"global-account"},"credentialRefs":{}})).unwrap();
    let engine=make_engine(f.engine.home.clone(),f.fake.clone(),false).unwrap();
    assert!(engine.standard_native);assert!(engine.native_acceptance.is_none());
    assert_eq!(engine.effective_settings(&f.store).unwrap()["chromeEndpoint"],"http://127.0.0.1:9999");
    assert_eq!(other.job(&key).unwrap()["status"],"submitting");assert_eq!(f.count(),0);
}

#[test]
fn background_chrome_public_dispatch_cannot_forge_internal_batch_binding() {
    let f=Fixture::new(&[("root",None)]);
    for action in ["images.run","images.produce","videos.submit","foundation.run"] {
        assert!(f.engine.dispatch(json!({"action":action,"projectId":f.request.project_id,"backgroundChromeBinding":f.request.binding().unwrap()})).is_err());
    }
    assert!(f.store.jobs().unwrap().is_empty());
}

#[test]
fn background_chrome_only_selected_jobs_no_text_video_other_provider_or_foreign_dependencies() {
    let f=Fixture::new(&[("root",None)]);
    let original=vec![f.store.add_job("text","untouched","codex_sdk","offline",&json!({}),"submitting").unwrap(),
        f.store.add_job("video","untouched-video","dreamina","offline",&json!({}),"queued").unwrap(),
        f.store.add_job("image","untouched-image","dreamina","offline",&json!({}),"waiting_dependency").unwrap(),
        f.store.add_job("image","outside-selection","chrome","offline",&json!({}),"waiting_dependency").unwrap()];
    let before=original.iter().map(|id|f.store.job(id).unwrap()).collect::<Vec<_>>();
    let j=f.queue();for _ in 0..4 {f.step(&j);}
    assert_eq!(f.count(),1);
    for (key,before) in original.iter().zip(before) {assert_eq!(f.store.job(key).unwrap(),before);}
}

#[test]
fn background_chrome_known_receipt_restart_query_download_never_generates_again() {
    let f=Fixture::new(&[("root",None)]);let j=f.queue();assert!(f.step(&j));assert_eq!(f.count(),1);
    let key=s(&j.jobs[0],"jobId");f.store.update_job(key,"submitting",&json!({})).unwrap();
    let recovered=f.queue();assert_eq!(f.store.job(key).unwrap()["status"],"submitted");
    assert!(f.step(&recovered));assert_eq!(f.count(),1);assert_eq!(f.store.job(key).unwrap()["status"],"downloaded");
}

#[test]
fn background_chrome_saved_download_retry_has_zero_new_query_or_submit() {
    let f=Fixture::new(&[("root",None)]);let j=f.queue();f.step(&j);f.fake.fail_download.store(true,Ordering::SeqCst);f.step(&j);
    let queries=f.fake.queries.load(Ordering::SeqCst);assert_eq!(queries,1);
    f.fake.fail_download.store(false,Ordering::SeqCst);f.step(&j);
    assert_eq!(f.fake.queries.load(Ordering::SeqCst),queries);assert_eq!(f.count(),1);
    assert_eq!(f.store.job(s(&j.jobs[0],"jobId")).unwrap()["status"],"downloaded");
}

#[test]
fn background_chrome_home_lease_excludes_second_engine_and_releases_after_drop() {
    let temp=tempfile::tempdir().unwrap();let lease=HomeExecutionLease::acquire(temp.path()).unwrap();
    assert!(HomeExecutionLease::acquire(temp.path()).is_err());drop(lease);assert!(HomeExecutionLease::acquire(temp.path()).is_ok());
}

#[test]
fn background_chrome_cross_process_lease_child() {
    let Some(home)=std::env::var_os("DIRECTOR_SYNTHETIC_CHROME_LEASE_HOME") else {return;};
    let expected=std::env::var("DIRECTOR_SYNTHETIC_CHROME_LEASE_EXPECT").unwrap();
    assert_eq!(HomeExecutionLease::acquire(Path::new(&home)).is_ok(),expected=="free");
}
#[test]
fn background_chrome_os_lease_proves_cross_process_exclusion_and_exit_release() {
    let temp=tempfile::tempdir().unwrap();let lease=HomeExecutionLease::acquire(temp.path()).unwrap();
    let child=|expected:&str| {
        let mut command=std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","engine::background_chrome_images::tests::background_chrome_cross_process_lease_child","--test-threads=1"])
            .env("DIRECTOR_SYNTHETIC_CHROME_LEASE_HOME",temp.path()).env("DIRECTOR_SYNTHETIC_CHROME_LEASE_EXPECT",expected);
        #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
        assert!(command.output().unwrap().status.success());
    };
    child("held");drop(lease);child("free");
}
#[test]
fn background_chrome_timeout_retains_original_pending_job_and_stops_new_actions() {
    let mut f=Fixture::new(&[("root",None)]);f.request.max_runtime_seconds=1;f.fake.pending.store(true,Ordering::SeqCst);
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),3);assert_eq!(f.count(),1);
    let job=f.store.jobs().unwrap().pop().unwrap();assert_eq!(job["status"],"processing");assert!(!s(&job,"providerTaskId").is_empty());
    let report:Value=serde_json::from_slice(&fs::read(&f.request.report_path).unwrap()).unwrap();assert_eq!(report["errorCode"],"BACKGROUND_CHROME_TIMEOUT");
}
#[test]
fn background_chrome_stop_between_operations_keeps_same_remote_id_without_query() {
    let f=Fixture::new(&[("root",None)]);let j=f.queue();f.step(&j);let before=f.store.job(s(&j.jobs[0],"jobId")).unwrap();
    fs::write(&f.request.stop_path,b"stop").unwrap();assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),3);
    assert_eq!(f.count(),1);assert_eq!(f.fake.queries.load(Ordering::SeqCst),0);assert_eq!(f.store.job(s(&j.jobs[0],"jobId")).unwrap(),before);
}

#[test]
fn background_chrome_request_schema_rejects_other_actions_and_relative_paths() {
    let f=Fixture::new(&[("root",None)]);let file=f._temp.path().join("request.json");
    let mut raw=serde_json::to_value(&f.request).unwrap();raw["action"]=json!("videos.submit");atomic_json(&file,&raw).unwrap();assert!(Request::from_file(&file).is_err());
    raw.as_object_mut().unwrap().remove("action");raw["home"]=json!("relative");atomic_json(&file,&raw).unwrap();assert!(Request::from_file(&file).is_err());
}
