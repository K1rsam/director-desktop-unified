//! Synthetic task-list maintenance acceptance; no browser, model, or account access.
use super::*;
use rusqlite::params;

struct Fixture { temp: tempfile::TempDir, engine: Arc<Engine>, store: Store, project: String, home: PathBuf }
impl Fixture {
    fn new() -> Self {
        let temp=tempfile::tempdir().unwrap();
        let home=temp.path().join("home");
        crate::platform::ensure_isolated_home(&home).unwrap();
        let engine=Engine::build_with_recovery(home.clone(),Arc::new(crate::execution_policy::OfflineChannels),false,true,false).unwrap();
        let project=engine.dispatch(json!({"action":"projects.create","title":"task visibility synthetic"})).unwrap();
        let project=s(&project,"id").to_owned();
        let store=engine.store(&project).unwrap();
        store.save_source(json!(pipeline::prepare_source("第一集\n场景：门口\n甲：开门。\n").unwrap())).unwrap();
        Self { temp,engine,store,project,home }
    }
    fn failed(&self, upload:bool) -> String {
        let value=if upload {crate::chrome_terminal::tests::upload_fixture()} else {crate::chrome_terminal::tests::fixture()};
        let key=self.store.add_job("image","asset","chrome","synthetic-account",&value["snapshot"],"failed").unwrap();
        self.store.update_job(&key,"failed",&json!({"providerTaskId":value["providerTaskId"],"prepared":value["prepared"],
            "receipt":{"taskId":value["providerTaskId"],"raw":"ORIGINAL RECEIPT"},"result":value["result"],"error":"ORIGINAL FAILURE"})).unwrap();
        key
    }
    fn action(&self, action:&str, key:&str) -> Result<Value> { self.engine.dispatch(json!({"action":action,"projectId":self.project,"jobId":key})) }
    fn bytes(&self,key:&str)->Vec<rusqlite::types::Value> {
        self.store.conn().unwrap().query_row("SELECT * FROM jobs WHERE id=?1",[key],|row|
            (0..row.as_ref().column_count()).map(|index|row.get(index)).collect()).unwrap()
    }
    fn event_count(&self,key:&str)->i64 {self.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type IN ('task_record_archived','task_record_restored')",[key],|r|r.get(0)).unwrap()}
    fn request_file(&self,keys:&[String],report:&str)->PathBuf {
        let source=self.store.source().unwrap();let path=self.temp.path().join(format!("{report}-request.json"));
        let jobs:Vec<_>=keys.iter().map(|key|json!({"jobId":key,"snapshotSha256":pipeline::digest(&self.store.job(key).unwrap()["snapshot"].to_string())})).collect();
        fs::write(&path,json!({"schemaVersion":"task-record-maintenance/v1","action":"jobs.archive","home":self.home,
            "projectId":self.project,"sourceSha256":source["sha256"],"sourceVersion":source["version"],"jobs":jobs,
            "reportPath":self.temp.path().join(format!("{report}.json"))}).to_string()).unwrap();path
    }
}

#[test] fn task_record_archive_restore_preserves_all_job_bytes_and_queue_inputs() {
    for upload in [false,true] {
        let f=Fixture::new();
        let key=f.failed(upload);let before=f.bytes(&key);let summaries=f.store.job_summaries().unwrap();
        let queued=f.store.add_job("image","asset","chrome","synthetic-account",&json!({"requestId":id(),"autoSubmit":true}),"queued").unwrap();
        let queued_before=f.bytes(&queued);let pending=f.store.pending_jobs().unwrap();
        let workspace=f.action("jobs.archive",&key).unwrap();
        assert_eq!(workspace["jobs"].as_array().unwrap().iter().find(|j|j["id"]==key).unwrap()["taskRecordArchived"],true);
        assert_eq!(f.action("jobs.detail",&key).unwrap()["taskRecordArchived"],true);
        assert_eq!(f.bytes(&key),before);assert_eq!(f.bytes(&queued),queued_before);assert_eq!(f.store.pending_jobs().unwrap(),pending);
        assert!(f.store.job_summaries().unwrap().iter().any(|j|j["id"]==key));
        assert_eq!(f.store.job_summaries().unwrap().iter().find(|j|j["id"]==key).unwrap(),summaries.iter().find(|j|j["id"]==key).unwrap());
        assert!(!retains_image_generation(&f.store.job(&key).unwrap()));
        f.action("jobs.archive",&key).unwrap();assert_eq!(f.event_count(&key),1);
        f.action("jobs.restore",&key).unwrap();f.action("jobs.restore",&key).unwrap();
        assert_eq!(f.action("jobs.detail",&key).unwrap()["taskRecordArchived"],false);
        assert_eq!(f.event_count(&key),2);assert_eq!(f.bytes(&key),before);
    }
}

#[test] fn task_record_rejects_unknown_active_success_and_unproved_failure() {
    let f=Fixture::new();let key=f.failed(false);
    for state in ["queued","preparing","ready","submitting","submitted","processing","submission_unknown","download_failed","downloaded","recovery_paused"] {
        f.store.update_job(&key,state,&json!({})).unwrap();let before=f.bytes(&key);
        for action in ["jobs.archive","jobs.restore"] {assert!(f.action(action,&key).is_err(),"{state}");}
        assert_eq!(f.bytes(&key),before);assert_eq!(f.event_count(&key),0);
    }
    f.store.update_job(&key,"failed",&json!({"result":{"status":"failed","taskId":"different","resultUrls":[]}})).unwrap();
    assert!(f.action("jobs.archive",&key).is_err());assert_eq!(f.event_count(&key),0);
    assert!(f.store.archived_task_record_ids().unwrap().is_empty());
}

#[test] fn task_record_snapshot_compare_and_event_failure_roll_back_visibility_atomically() {
    let f=Fixture::new();let key=f.failed(false);let before=f.bytes(&key);
    assert!(f.engine.dispatch(json!({"action":"jobs.archive","projectId":f.project,"jobId":key,"expectedSnapshotSha256":"wrong"})).is_err());
    f.store.conn().unwrap().execute_batch("CREATE TRIGGER prevent_archive_event BEFORE INSERT ON events WHEN NEW.type='task_record_archived' BEGIN SELECT RAISE(ABORT,'synthetic event rejection'); END;").unwrap();
    assert!(f.action("jobs.archive",&key).is_err());assert_eq!(f.bytes(&key),before);
    assert!(f.store.archived_task_record_ids().unwrap().is_empty());assert_eq!(f.event_count(&key),0);
}

#[test] fn task_record_concurrent_identical_archive_is_idempotent() {
    let f=Fixture::new();let key=f.failed(true);let before=f.bytes(&key);
    let barrier=Arc::new(std::sync::Barrier::new(2));
    let threads:Vec<_>=(0..2).map(|_|{let store=f.store.clone();let key=key.clone();let barrier=barrier.clone();std::thread::spawn(move||{barrier.wait();store.set_task_record_archived(&key,true,None).unwrap()})}).collect();
    let outcomes:Vec<_>=threads.into_iter().map(|t|t.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|value|**value).count(),1);assert_eq!(f.event_count(&key),1);assert_eq!(f.bytes(&key),before);
}

#[test] fn task_record_maintenance_is_leased_idempotent_and_never_recovers_another_job() {
    let f=Fixture::new();let keys=vec![f.failed(false),f.failed(true)];
    let other=f.store.add_job("image","other","chrome","synthetic-account",&json!({"requestId":id(),"autoSubmit":true}),"submitting").unwrap();
    let other_before=f.bytes(&other);let before:Vec<_>=keys.iter().map(|key|f.bytes(key)).collect();
    // A saved grant must not be loaded by this maintenance-only entrypoint.
    fs::write(f.home.join("native-cli-acceptance.json"),"INVALID SAVED GRANT MUST NOT BE READ").unwrap();
    let input=f.request_file(&keys,"maintenance");
    {let _lease=background_chrome_images::HomeExecutionLease::acquire(&f.home).unwrap();
        let error=task_record_maintenance::run_file(&input).unwrap_err();assert!(error.starts_with("HOME_EXECUTION_BUSY"),"{error}");}
    assert_eq!(task_record_maintenance::run_file(&input).unwrap(),0);
    assert_eq!(task_record_maintenance::run_file(&input).unwrap(),0);
    let report:Value=serde_json::from_slice(&fs::read(f.temp.path().join("maintenance.json")).unwrap()).unwrap();
    assert_eq!(report["status"],"completed");assert_eq!(report["generationRequestsSent"],0);
    assert!(!report.to_string().contains("ORIGINAL"));assert_eq!(f.bytes(&other),other_before);
    for (index,key) in keys.iter().enumerate() {assert_eq!(f.bytes(key),before[index]);assert_eq!(f.event_count(key),1);}
}

#[test] fn task_record_maintenance_checks_full_selection_source_and_report_before_any_change() {
    for change in ["source","snapshot","active","unsafe-report","foreign-report"] {
        let f=Fixture::new();let first=f.failed(false);let second=f.failed(true);let input=f.request_file(&[first.clone(),second.clone()],change);
        let mut request:Value=serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        match change {
            "source"=>request["sourceSha256"]=json!("0".repeat(64)),
            "snapshot"=>request["jobs"][1]["snapshotSha256"]=json!("0".repeat(64)),
            "active"=>{f.store.update_job(&second,"processing",&json!({})).unwrap();},
            "unsafe-report"=>request["reportPath"]=json!(f.home.join("projects.json")),
            _=>{fs::write(f.temp.path().join(format!("{change}.json")),"{\"private\":\"unrelated\"}").unwrap();},
        }
        fs::write(&input,request.to_string()).unwrap();
        assert!(task_record_maintenance::run_file(&input).is_err(),"{change}");
        assert!(f.store.archived_task_record_ids().unwrap().is_empty());assert_eq!(f.event_count(&first),0);assert_eq!(f.event_count(&second),0);
    }
}

#[test] fn task_record_archive_metadata_cannot_hide_a_reopened_or_unproved_job() {
    let f=Fixture::new();let key=f.failed(false);f.action("jobs.archive",&key).unwrap();
    f.store.conn().unwrap().execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",params![key]).unwrap();
    assert!(f.store.archived_task_record_ids().unwrap().is_empty());
    assert_eq!(f.action("jobs.detail",&key).unwrap()["taskRecordArchived"],false);
}
