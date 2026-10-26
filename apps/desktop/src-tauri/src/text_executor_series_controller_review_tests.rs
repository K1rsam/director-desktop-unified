use super::*;

async fn fixture()->(tempfile::TempDir,SavedTextExecutor,Value,String){
    let(mut input,_,review,raw,mut auth,_)=crate::pipeline::series::series_controller_fixture().await;
    let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();
    store.create("controller fixture","test").unwrap();store.ensure_workflow_storage().unwrap();
    input["source"]=store.save_source(input["source"].clone()).unwrap();
    input["settings"]["textProvider"]=json!("codex");
    let parent=json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input});
    let run=store.add_job("workflow","storyboard.run","local","local",&parent,"partially_blocked").unwrap();
    let executor=SavedTextExecutor{store,settings:input["settings"].clone(),workflow_id:Some(run.clone())};
    let snapshot=executor.request_domain_snapshot(&review).unwrap();let sha=crate::pipeline::digest(&snapshot.to_string());
    let job=executor.store.add_job("text","reviewer","codex","offline",&snapshot,"completed").unwrap();
    executor.store.update_job(&job,"completed",&json!({"result":raw,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
    executor.store.event(&job,"text_received_raw",&json!({"requestHash":sha,"value":raw})).unwrap();
    executor.store.checkpoint(&run,&step_key(&review),&review.stage,&sha,"accepted",Some(&job),Some(&raw),None).unwrap();
    auth["reviewJobId"]=json!(job);
    (temp,executor,auth,job)
}
fn preserved(executor:&SavedTextExecutor)->Value{
    let c=executor.store.conn().unwrap();let mut q=c.prepare("SELECT snapshot,COALESCE(result,''),COALESCE(receipt,'') FROM jobs ORDER BY rowid").unwrap();
    json!(q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap())
}
#[tokio::test]
async fn series_controller_review_durable_freeze_reopens_idempotently_without_rewriting_evidence(){
    let(_temp,executor,auth,_job)=fixture().await;let before=preserved(&executor);
    assert_eq!(executor.authorize_series_controller_review(&auth).unwrap()["reused"],false);
    assert_eq!(preserved(&executor),before);
    let context=executor.saved_series_controller_review().unwrap().unwrap();
    let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
    assert_eq!(reopened.saved_series_controller_review().unwrap(),Some(context));
    assert_eq!(reopened.authorize_series_controller_review(&auth).unwrap()["reused"],true);
    assert_eq!(preserved(&executor),before);assert_eq!(executor.store.jobs().unwrap().len(),2);
    assert!(!executor.store.root.join("text-runs").exists());
    let mut changed=auth.clone();changed["issues"][0]["problem"]=json!("different");
    assert!(reopened.authorize_series_controller_review(&changed).is_err());
    let n:i64=executor.store.conn().unwrap().query_row("SELECT count(*) FROM events WHERE type=?1",[EVENT],|r|r.get(0)).unwrap();assert_eq!(n,1);
}
#[tokio::test]
async fn series_controller_review_unknown_existing_slot_and_tampering_hold_before_new_writes(){
    for fault in ["unknown","occupied","raw","candidate","source","event"]{
        let(_temp,executor,mut auth,job)=fixture().await;
        match fault{
            "unknown"=>executor.store.update_job(&job,"submission_unknown",&json!({})).unwrap(),
            "occupied"=>{let snap=json!({"input":{"workflowProfile":crate::asset_graph::PROFILE,"sourceSha256":auth["sourceSha256"],"executionRoleId":format!("{}/repair/review-1",s(&auth,"episodeId"))}});executor.store.add_job("text","repair","codex","offline",&snap,"rejected").unwrap();},
            "raw"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET result='{}' WHERE id=?1",[job]).unwrap();},
            "candidate"=>auth["candidateSha256"]=json!("wrong"),
            "source"=>auth["sourceSha256"]=json!("wrong"),
            "event"=>{executor.store.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1 AND type='text_received_raw'",[job]).unwrap();},_=>unreachable!()}
        let before=executor.store.jobs().unwrap();assert!(executor.authorize_series_controller_review(&auth).is_err(),"{fault}");
        assert_eq!(executor.store.jobs().unwrap(),before);assert!(!executor.store.root.join("text-runs").exists());
    }
}

#[tokio::test]
async fn series_controller_review_explicit_later_resume_unpauses_only_the_bound_adopted_delivery(){
    let(_temp,executor,auth,_)=fixture().await;
    executor.authorize_series_controller_review(&auth).unwrap();
    let run=executor.workflow_id.as_deref().unwrap();
    let context=executor.saved_series_controller_review().unwrap().unwrap();
    let original=executor.store.job(run).unwrap()["snapshot"].clone();
    assert!(executor.series_controller_stop_required().unwrap());
    executor.continue_completed_controller_review().unwrap();
    assert!(executor.series_controller_stop_required().unwrap(),"unfinished repair cannot release the stop");
    let output=crate::pipeline::series::series_controller_delivery_fixture(&original["input"],&context).await;
    let delivery=&output["seriesDeliveries"][0];
    let c=executor.store.conn().unwrap();
    for (prefix,value) in [("series.delivery",delivery.clone()),("series.input",original["input"].clone()),
        ("series.publication",json!({"workflowId":run,"bundleSha256":delivery["adoptionReceipt"]["bundleSha256"]}))]{
        c.execute("INSERT INTO metadata(key,value) VALUES(?1,?2)",params![format!("{prefix}:{}",s(&auth,"episodeId")),value.to_string()]).unwrap();
    }
    executor.store.update_job(run,"partially_blocked",&json!({"result":output})).unwrap();
    let before=preserved(&executor);
    executor.continue_completed_controller_review().unwrap();
    assert!(!executor.series_controller_stop_required().unwrap());
    assert_eq!(executor.saved_series_controller_review().unwrap(),Some(context));
    assert_eq!(preserved(&executor),before);assert_eq!(executor.store.job(run).unwrap()["snapshot"],original);
    executor.continue_completed_controller_review().unwrap();
    let n:i64=c.query_row("SELECT count(*) FROM events WHERE type=?1",[CONTINUE_EVENT],|r|r.get(0)).unwrap();assert_eq!(n,1);
    c.execute("UPDATE metadata SET value='{}' WHERE key=?1",[format!("series.delivery:{}",s(&auth,"episodeId"))]).unwrap();
    assert!(executor.series_controller_stop_required().is_err(),"a changed adopted proof cannot silently release the stop");
}
