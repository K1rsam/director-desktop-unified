//! Synthetic databases/channels only. No browser, account, SDK or video calls.
use super::*;
use crate::providers::{DownloadReceipt,GenerationRequest,PreparedRequest,QueryResult,SubmitReceipt};
use std::sync::atomic::AtomicUsize;
use std::collections::VecDeque;

#[derive(Default)]
struct Fake {
    submits:Mutex<Vec<GenerationRequest>>,
    prepares:AtomicUsize,
    recoveries:AtomicUsize,
    queries:AtomicUsize,
    downloads:AtomicUsize,
    unknown:AtomicBool,
    fail_download:AtomicBool,
    pending:AtomicBool,
    cleanup_outcomes:Mutex<VecDeque<Value>>,
    cleanup_default:Mutex<Option<Value>>,
    releases:AtomicUsize,
}
impl Channels for Fake {
    fn capabilities(&self,c:&ProviderConfig)->ProviderResult<Capabilities> {
        assert_eq!(c.kind,"chrome");
        Ok(serde_json::from_value(json!({"provider":"chrome","version":"offline/v1","source":"synthetic","checkedAt":now(),
            "models":[{"id":"chatgpt-image","kind":"image","adapted":true,"ratios":["1:1","16:9"],"resolutions":["2K"],"images":8,"videos":0,"audios":0,"requiredImages":0,"raw":{}}],
            "canCancel":false,"canReconcileUnknown":false,"raw":{}})).unwrap())
    }
    fn prepare(&self,c:&ProviderConfig,r:&GenerationRequest)->ProviderResult<PreparedRequest> {
        self.prepares.fetch_add(1,Ordering::SeqCst);
        assert_eq!(c.kind,"chrome");assert_eq!(r.kind,"image");
        Ok(PreparedRequest{provider:"chrome".into(),request:r.clone(),capability_version:"offline/v1".into(),
            upload_receipts:vec![],wire:json!({"sourceClientId":"owned-synthetic-tab","metadata":{"promptSha256":pipeline::digest(&r.prompt)}}),estimated_cost:None,prepared_at:now()})
    }
    fn recover_prepare(&self,c:&ProviderConfig,r:&GenerationRequest,proof:&Value,observe:&crate::providers::PrepareObserver<'_>)->ProviderResult<PreparedRequest> {
        assert_eq!(c.kind,"chrome");assert_eq!(proof["noLocalSubmit"],true);
        self.recoveries.fetch_add(1,Ordering::SeqCst);
        if proof["intent"].is_null() {observe(&json!({"version":"chrome-prepare-intent/v1","mode":"legacy_first_prepare","requestId":r.request_id,"requestHash":pipeline::digest(&json!(r).to_string()),"createdAt":now()}))?;}
        Ok(PreparedRequest{provider:"chrome".into(),request:r.clone(),capability_version:"offline/v1".into(),upload_receipts:vec![],
            wire:json!({"sourceClientId":"owned-synthetic-tab","metadata":{"promptSha256":pipeline::digest(&r.prompt)}}),estimated_cost:None,prepared_at:now()})
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
        let requests=self.submits.lock().unwrap();
        let request=requests.iter().find(|request|request.request_id==id).unwrap();
        Ok(QueryResult{task_id:id.into(),status:"succeeded".into(),result_urls:vec![format!("https://fixture.invalid/{id}.png")],error:None,actual_cost:None,
            raw:json!({"synthetic":true,"nativeProof":{"status":"matched_complete","proofVersion":2,"requestId":id,"promptSha256":pipeline::digest(&request.prompt),"conversationId":"owned-synthetic-conversation"}})})
    }
    fn download(&self,c:&ProviderConfig,_:&str,path:&Path)->ProviderResult<DownloadReceipt> {
        assert_eq!(c.kind,"chrome");self.downloads.fetch_add(1,Ordering::SeqCst);
        if self.fail_download.load(Ordering::SeqCst) {return Err(ProviderError::new("SYNTHETIC_DOWNLOAD","offline download failed"));}
        image::RgbImage::from_pixel(3,2,image::Rgb([self.downloads.load(Ordering::SeqCst) as u8,7,13])).save(path).unwrap();
        let (sha,bytes)=crate::store::hash_file(path).unwrap();
        Ok(DownloadReceipt{path:path.into(),sha256:sha,bytes,mime:Some("image/png".into())})
    }
    fn reconcile(&self,_:&ProviderConfig,_:&str)->ProviderResult<QueryResult> {panic!("unknown submissions are never automatically reconciled/resubmitted");}
    fn release_completed_tab(&self,c:&ProviderConfig,_:&PreparedRequest,_:&QueryResult)->ProviderResult<Value> {
        assert_eq!(c.kind,"chrome");self.releases.fetch_add(1,Ordering::SeqCst);
        Ok(self.cleanup_outcomes.lock().unwrap().pop_front().or_else(||self.cleanup_default.lock().unwrap().clone()).unwrap_or_else(||json!({"status":"closed"})))
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
fn background_chrome_legacy_prepare_recovery_is_ready_only_then_original_gate_before_first_submit() {
    let f=Fixture::new(&[("root",None)]);let j=f.queue();let key=s(&j.jobs[0],"jobId");let original=f.store.job(key).unwrap();
    f.store.claim_job(key,"queued","preparing").unwrap();f.store.claim_job_fields(key,"preparing","failed",&json!({"error":"original timeout"})).unwrap();
    assert!(f.step(&j));assert_eq!(f.store.job(key).unwrap()["status"],"ready");assert_eq!(f.count(),0);
    assert_eq!(f.fake.prepares.load(Ordering::SeqCst),0);assert_eq!(f.fake.recoveries.load(Ordering::SeqCst),1);
    assert_eq!(f.store.jobs().unwrap().len(),1);assert_eq!(f.store.job(key).unwrap()["snapshot"],original["snapshot"]);
    let unknown=f.store.add_job("image","outside","chrome","offline",&json!({}),"submission_unknown").unwrap();
    assert!(!f.step(&j));assert_eq!(f.count(),0,"a new scheduler turn rechecks original unknown gate");
    f.store.update_job(&unknown,"closed_by_user",&json!({})).unwrap();
    assert!(f.step(&j));assert_eq!(f.count(),1);assert_eq!(f.store.job(key).unwrap()["snapshot"]["requestId"],original["snapshot"]["requestId"]);
}

#[test]
fn background_chrome_blocked_children_restore_same_ids_and_wait_for_original_direct_parent_adoption() {
    let f=Fixture::new(&[("root",None),("suit",Some("root")),("injured",Some("suit"))]);let j=f.queue();
    let root=j.jobs.iter().find(|job|job["assetId"]=="root").unwrap();let key=s(root,"jobId");
    f.store.claim_job(key,"queued","preparing").unwrap();f.store.claim_job_fields(key,"preparing","failed",&json!({"error":"original timeout"})).unwrap();
    for saved in j.jobs.iter().filter(|job|job["assetId"]!="root") {f.store.update_job(s(saved,"jobId"),"blocked",&json!({"error":"指定来源图片任务未完成，请处理该来源后重新制作关联图片"})).unwrap();}
    let before=f.store.jobs().unwrap();assert!(!recover_selected_dependency_blocks(&f.request,&f.store,&j).unwrap());
    assert!(f.step(&j));assert_eq!(f.store.job(key).unwrap()["status"],"ready");assert_eq!(f.count(),0);
    for _ in 0..3 {recover_selected_dependency_blocks(&f.request,&f.store,&j).unwrap();}
    for saved in j.jobs.iter().filter(|job|job["assetId"]!="root") {
        let child=f.store.job(s(saved,"jobId")).unwrap();assert_eq!(child["status"],"waiting_dependency");assert_eq!(child["error"],"指定来源图片任务未完成，请处理该来源后重新制作关联图片");
        assert_eq!(child["snapshot"],before.iter().find(|job|job["id"]==child["id"]).unwrap()["snapshot"]);
    }
    f.engine.unblock_image_dependencies_selected(&f.store,Some(&j.jobs.iter().map(|job|s(job,"jobId").to_owned()).collect())).unwrap();
    assert!(j.jobs.iter().filter(|job|job["assetId"]!="root").all(|job|f.store.job(s(job,"jobId")).unwrap()["status"]=="waiting_dependency"));
    for _ in 0..12 {f.step(&j);}
    assert_eq!(f.count(),3);assert_eq!(f.store.jobs().unwrap().len(),3);
    assert!(j.jobs.iter().all(|saved|f.store.job(s(saved,"jobId")).unwrap()["status"]=="downloaded"));
    let sent=f.fake.submits.lock().unwrap();let suit=sent.iter().find(|r|r.request_id==j.jobs.iter().find(|job|job["assetId"]=="suit").unwrap()["requestId"]).unwrap();
    let injured=sent.iter().find(|r|r.request_id==j.jobs.iter().find(|job|job["assetId"]=="injured").unwrap()["requestId"]).unwrap();
    assert_ne!(suit.references[0].media_id,injured.references[0].media_id,"each child uses its direct parent");
}

#[test]
fn background_chrome_dependency_recovery_holds_receipt_unknown_history_parent_or_unrelated_error() {
    for case in ["prepared","receipt","task","history","parent_unknown","parent_no_recovery","error","missing_history"] {
        let f=Fixture::new(&[("root",None),("child",Some("root"))]);let j=f.queue();let root=j.jobs.iter().find(|job|job["assetId"]=="root").unwrap();let child=j.jobs.iter().find(|job|job["assetId"]=="child").unwrap();
        let root_id=s(root,"jobId");let child_id=s(child,"jobId");
        f.store.claim_job(root_id,"queued","preparing").unwrap();f.store.claim_job_fields(root_id,"preparing","failed",&json!({"error":"original timeout"})).unwrap();
        let (cfg,account)=f.engine.generation_config(&f.store,&f.store.job(root_id).unwrap()).unwrap();tasks::resume_chrome_prepare(&f.store,root_id,&cfg,&account,f.fake.as_ref()).unwrap();
        f.store.update_job(child_id,"blocked",&json!({"error":"指定来源图片任务未完成，请处理该来源后重新制作关联图片"})).unwrap();
        match case {
            "prepared"=>f.store.update_job(child_id,"blocked",&json!({"prepared":{}})).unwrap(),
            "receipt"=>f.store.update_job(child_id,"blocked",&json!({"receipt":{"accepted":true}})).unwrap(),
            "task"=>f.store.update_job(child_id,"blocked",&json!({"providerTaskId":"old-task"})).unwrap(),
            "history"=>f.store.event(child_id,"state_claim",&json!({"from":"ready","state":"submitting"})).unwrap(),
            "parent_unknown"=>f.store.update_job(root_id,"submission_unknown",&json!({})).unwrap(),
            "parent_no_recovery"=>{f.store.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1 AND type='chrome_prepare_recovery_started'",[root_id]).unwrap();},
            "missing_history"=>{f.store.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1",[child_id]).unwrap();},
            _=>f.store.update_job(child_id,"blocked",&json!({"error":"different error"})).unwrap(),
        }
        let before=f.store.jobs().unwrap();assert!(!recover_selected_dependency_blocks(&f.request,&f.store,&j).unwrap(),"{case}");assert_eq!(f.store.jobs().unwrap(),before,"{case}");assert_eq!(f.count(),0);
    }
}

#[test]
fn background_chrome_dependency_recovery_keeps_normal_other_reference_branch_and_waits_for_both_parents() {
    let mut f=Fixture::new(&[("root",None),("reference",None),("child",Some("root"))]);
    let mut asset=f.store.active_asset("child").unwrap();asset["design"]=json!({"referenceAssetIds":["reference"]});let asset=f.store.save_asset(asset).unwrap();
    f.request.assets.iter_mut().find(|a|a.asset_id=="child").unwrap().asset_version=asset["version"].as_i64().unwrap();
    let j=f.queue();assert_eq!(j.jobs.len(),3);let root=j.jobs.iter().find(|job|job["assetId"]=="root").unwrap();let child=j.jobs.iter().find(|job|job["assetId"]=="child").unwrap();
    let root_id=s(root,"jobId");let child_id=s(child,"jobId");
    f.store.claim_job(root_id,"queued","preparing").unwrap();f.store.claim_job_fields(root_id,"preparing","failed",&json!({"error":"original timeout"})).unwrap();
    f.store.update_job(child_id,"blocked",&json!({"error":"指定来源图片任务未完成，请处理该来源后重新制作关联图片"})).unwrap();
    let (cfg,account)=f.engine.generation_config(&f.store,&f.store.job(root_id).unwrap()).unwrap();tasks::resume_chrome_prepare(&f.store,root_id,&cfg,&account,f.fake.as_ref()).unwrap();
    let snapshot=f.store.job(child_id).unwrap()["snapshot"].clone();assert_eq!(snapshot["dependencyJobIds"].as_array().unwrap().len(),2);
    assert!(recover_selected_dependency_blocks(&f.request,&f.store,&j).unwrap());assert_eq!(f.store.job(child_id).unwrap()["status"],"waiting_dependency");
    assert_eq!(f.store.job(child_id).unwrap()["snapshot"],snapshot);assert_eq!(f.count(),0);
    f.engine.unblock_image_dependencies_selected(&f.store,Some(&j.jobs.iter().map(|job|s(job,"jobId").to_owned()).collect())).unwrap();
    assert_eq!(f.store.job(child_id).unwrap()["status"],"waiting_dependency");
    for _ in 0..14 {f.step(&j);}
    assert_eq!(f.count(),3);assert_eq!(f.store.jobs().unwrap().len(),3);assert_eq!(f.store.job(child_id).unwrap()["snapshot"]["references"].as_array().unwrap().len(),2);
}

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

fn cleanup_state(store:&Store,key:&str)->Value {
    let raw:String=store.conn().unwrap().query_row("SELECT value FROM metadata WHERE key=?1",[format!("browser_tab_cleanup:{key}")],|row|row.get(0)).unwrap();
    serde_json::from_str(&raw).unwrap()
}
fn fixture_cleanup_due(store:&Store,key:&str) {
    let mut state=cleanup_state(store,key);state["nextAttemptAt"]=json!(0);
    store.conn().unwrap().execute("UPDATE metadata SET value=?1 WHERE key=?2",rusqlite::params![state.to_string(),format!("browser_tab_cleanup:{key}")]).unwrap();
}
fn report(f:&Fixture)->Value {serde_json::from_slice(&fs::read(&f.request.report_path).unwrap()).unwrap()}

#[test]
fn background_chrome_last_image_delayed_idle_close_and_confirmation_complete_without_more_media_work() {
    let mut f=Fixture::new(&[("root",None)]);f.request.max_runtime_seconds=4;
    *f.fake.cleanup_outcomes.lock().unwrap()=VecDeque::from([
        json!({"status":"retained","reason":"idle_not_proven"}),json!({"status":"close_requested"}),json!({"status":"already_closed"})]);
    let j=f.queue();f.step(&j);f.step(&j);let key=s(&j.jobs[0],"jobId").to_owned();
    assert_eq!(cleanup_state(&f.store,&key)["status"],"pending");
    assert!(cleanup_state(&f.store,&key)["nextAttemptAt"].as_i64().unwrap()>chrono::Utc::now().timestamp());
    let store=f.store.clone();let key2=key.clone();
    let clock_fixture=std::thread::spawn(move||{
        // Simulate only fixture clock passage; production 15/60-second timestamps remain untouched.
        let until=Instant::now()+Duration::from_secs(3);let mut advanced=0;
        loop {let state=cleanup_state(&store,&key2);if state["status"]=="done" {break;}
            let attempts=state["attempts"].as_u64().unwrap();
            if attempts>advanced {fixture_cleanup_due(&store,&key2);advanced=attempts;}
            assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(20));
        }
    });
    assert_eq!(run_owned_with_cleanup_budget(f.engine.clone(),&f.request,Duration::from_secs(3)).unwrap(),0);clock_fixture.join().unwrap();
    assert_eq!(f.count(),1);assert_eq!(f.fake.queries.load(Ordering::SeqCst),1);assert_eq!(f.fake.downloads.load(Ordering::SeqCst),1);
    assert_eq!(f.fake.releases.load(Ordering::SeqCst),3);assert_eq!(report(&f)["status"],"completed");assert_eq!(report(&f)["cleanup"]["done"],1);
    assert_eq!(cleanup_state(&f.store,&key)["status"],"done");
}

#[test]
fn background_chrome_reopen_pending_close_resumes_only_confirmation_with_zero_new_submit_query_download() {
    let f=Fixture::new(&[("root",None)]);
    *f.fake.cleanup_outcomes.lock().unwrap()=VecDeque::from([json!({"status":"close_requested"}),json!({"status":"already_closed"})]);
    let j=f.queue();f.step(&j);f.step(&j);let key=s(&j.jobs[0],"jobId");
    assert_eq!(run_owned_with_cleanup_budget(f.engine.clone(),&f.request,Duration::ZERO).unwrap(),0);
    assert_eq!(report(&f)["status"],"completed_cleanup_pending");assert_eq!(report(&f)["imagesComplete"],true);
    assert_eq!(cleanup_state(&f.store,key)["closeRequested"],true);
    // Reopening before the original due time must not perform a cleanup attempt.
    let releases=f.fake.releases.load(Ordering::SeqCst);
    assert_eq!(run_owned_with_cleanup_budget(f.engine.clone(),&f.request,Duration::ZERO).unwrap(),0);
    assert_eq!(f.fake.releases.load(Ordering::SeqCst),releases);
    fixture_cleanup_due(&f.store,key);
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),0);
    assert_eq!(report(&f)["status"],"completed");assert_eq!(report(&f)["cleanup"]["done"],1);
    assert_eq!(f.count(),1);assert_eq!(f.fake.queries.load(Ordering::SeqCst),1);assert_eq!(f.fake.downloads.load(Ordering::SeqCst),1);
}

#[test]
fn background_chrome_user_viewing_page_is_retained_and_pending_is_not_reported_closed() {
    let f=Fixture::new(&[("root",None)]);
    *f.fake.cleanup_default.lock().unwrap()=Some(json!({"status":"retained","reason":"user_viewing_or_visibility_unknown"}));
    let j=f.queue();f.step(&j);f.step(&j);
    assert_eq!(run_owned_with_cleanup_budget(f.engine.clone(),&f.request,Duration::ZERO).unwrap(),0);
    let r=report(&f);assert_eq!(r["status"],"completed_cleanup_pending");assert_eq!(r["imagesComplete"],true);
    assert_eq!(r["cleanup"]["done"],0);assert_eq!(r["cleanup"]["pending"],1);assert_eq!(r["cleanup"]["retained"],0);
    assert_eq!(r["cleanup"]["jobs"][0]["pageDisposition"],"retained");
    assert_eq!(cleanup_state(&f.store,s(&j.jobs[0],"jobId"))["status"],"pending");
    assert_eq!(f.fake.releases.load(Ordering::SeqCst),1);assert_eq!(f.count(),1);
}

#[test]
fn background_chrome_terminal_retained_page_finishes_media_without_false_close_and_reopen_no_actions() {
    let f=Fixture::new(&[("root",None)]);
    *f.fake.cleanup_default.lock().unwrap()=Some(json!({"status":"retained","reason":"ownership_or_idle_not_proven"}));
    let j=f.queue();f.step(&j);f.step(&j);
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),0);
    assert_eq!(report(&f)["status"],"completed_with_retained_pages");assert_eq!(report(&f)["cleanup"]["retained"],1);assert_eq!(report(&f)["cleanup"]["done"],0);
    assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),0);
    assert_eq!(f.fake.releases.load(Ordering::SeqCst),1);assert_eq!(f.count(),1);assert_eq!(f.fake.downloads.load(Ordering::SeqCst),1);
}

#[test]
fn background_chrome_cleanup_does_not_backfill_unselected_downloaded_legacy_job() {
    let f=Fixture::new(&[("root",None)]);let j=f.queue();f.step(&j);f.step(&j);
    let completed=f.store.job(s(&j.jobs[0],"jobId")).unwrap();
    let other=f.store.add_job("image","unselected","chrome",s(&completed,"accountId"),&completed["snapshot"],"downloaded").unwrap();
    f.store.update_job(&other,"downloaded",&json!({"providerTaskId":"unselected-original","prepared":completed["prepared"],"result":completed["result"]})).unwrap();
    // Use the original existing media identity, without any new import/download.
    f.store.conn().unwrap().execute("INSERT OR IGNORE INTO job_media(job_id,media_id) VALUES(?1,?2)",rusqlite::params![other,completed["resultMediaIds"][0].as_str().unwrap()]).unwrap();
    let before=f.store.job(&other).unwrap();assert_eq!(run_owned(f.engine.clone(),&f.request).unwrap(),0);
    let exists:bool=f.store.conn().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key=?1)",[format!("browser_tab_cleanup:{other}")],|row|row.get(0)).unwrap();
    assert!(!exists);assert_eq!(f.store.job(&other).unwrap(),before);assert_eq!(f.count(),1);
}
