//! Original registered-key fragment replay: no network, dispatch or production writes.
use super::*;
use super::tests::{fixture_kind,preserved};

#[tokio::test]
async fn sdk_fragment_replay_accept_reopen_preserves_raw_and_dispatch_count() {
    let (_temp,executor,request,job,raw)=fixture_kind(false,true);
    let before=preserved(&executor,&job);
    assert_eq!(executor.execute(request.clone()).await.unwrap(),raw);
    executor.accept(&request,&raw).await.unwrap();
    let adopted=executor.sdk_job(&job).unwrap();
    assert_eq!(adopted["receipt"]["migrationPolicy"],FRAGMENT_POLICY);
    assert_eq!(adopted["prepared"]["sdkDomainResult"],raw);
    let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
    for _ in 0..2 {
        assert_eq!(reopened.execute(request.clone()).await.unwrap(),raw);
        reopened.accept(&request,&raw).await.unwrap();
    }
    assert_eq!(preserved(&reopened,&job),before);
    assert_eq!(reopened.store.jobs().unwrap().len(),2);
    assert!(!reopened.store.root.join("sdk-worker").exists());
    let n:i64=reopened.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type=?2",params![job,FRAGMENT.accepted_event],|r|r.get(0)).unwrap();
    assert_eq!(n,1);
}

#[tokio::test]
async fn sdk_fragment_replay_second_rejection_cannot_retry_or_promote() {
    let (_temp,executor,request,job,raw)=fixture_kind(false,true);
    let before=preserved(&executor,&job);
    executor.reject(&request,&raw,"仍存在不兼容片段").await.unwrap();
    assert!(executor.execute(request.clone()).await.unwrap_err().contains("REPLAY_ALREADY_REJECTED"));
    assert!(executor.accept(&request,&raw).await.is_err());
    assert!(executor.reject(&request,&raw,"其它错误").await.is_err());
    assert_eq!(executor.sdk_job(&job).unwrap()["error"],FRAGMENT_REJECTION);
    assert_eq!(preserved(&executor,&job),before);
    assert!(!executor.store.root.join("sdk-worker").exists());
}

#[test]
fn sdk_fragment_replay_requires_exact_error_registered_duplicate_and_context() {
    let (_temp,executor,request,job,raw)=fixture_kind(false,true);
    let snapshot=executor.request_snapshot(&request).unwrap();
    let original=executor.sdk_job(&job).unwrap();
    let audit=executor.sdk_f2_replay_audit(&request,&snapshot,&original).unwrap();
    assert!(audit_valid(&audit));
    assert!(registered_fragment_present(&snapshot,&raw));
    let mut unregistered=snapshot.clone();unregistered["input"]["registry"]=json!([]);
    assert!(!registered_fragment_present(&unregistered,&raw));
    assert!(!registered_fragment_present(&snapshot,&json!({"assets":[raw["assets"][0]]})));
    assert!(replay_policy(&snapshot,&json!("另一错误")).is_none());
    let parent:Value=serde_json::from_str(s(&audit,"originalWorkflowSnapshotJson")).unwrap();
    for field in ["unexpected","sdkDomainResult","sdkSupervisorHandoff","sdkWorkerScope"] {
        let mut forged=original["prepared"].clone();forged[field]=json!({"bad":true});
        assert!(!fragment_context_bound(&snapshot,&parent,&forged,s(&audit,"workflowId")),"{field}");
    }
    let mut changed=parent.clone();changed["input"]["settings"]["textSdkIdentity"]=json!({});
    assert!(!fragment_context_bound(&snapshot,&changed,&original["prepared"],s(&audit,"workflowId")));
    // Old policies still demand exactly their historical seven prepared fields.
    let (_temp,old,req,id,_raw)=fixture_kind(false,false);
    let snap=old.request_snapshot(&req).unwrap();
    let mut proof=old.sdk_f2_replay_audit(&req,&snap,&old.sdk_job(&id).unwrap()).unwrap();
    let mut prepared:Value=serde_json::from_str(s(&proof,"originalPreparedJson")).unwrap();
    prepared["sdkWorkerScope"]=json!({});proof["originalPreparedJson"]=json!(prepared.to_string());
    assert!(!audit_valid(&proof));
}

#[tokio::test]
async fn sdk_fragment_replay_unknown_changed_checkpoint_and_duplicate_event_fail_closed() {
    for change in ["unknown","checkpoint","parent","rawEvent","sdkEvent","result.json","sdk-attempt.json","sdk-compiled.json"] {
        let (_temp,executor,request,job,raw)=fixture_kind(false,true);
        let snapshot=executor.request_snapshot(&request).unwrap();
        let original=executor.sdk_job(&job).unwrap();
        let audit=executor.sdk_f2_replay_audit(&request,&snapshot,&original).unwrap();
        let c=executor.store.conn().unwrap();
        match change {
            "unknown"=>{c.execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",[&job]).unwrap();}
            "checkpoint"=>{c.execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&job]).unwrap();}
            "parent"=>{c.execute("UPDATE jobs SET snapshot='{}' WHERE id=?1",[executor.workflow_id.as_ref().unwrap()]).unwrap();}
            "rawEvent"=>executor.store.event(&job,"text_received_raw",&serde_json::from_str::<Value>(s(&audit,"originalRawEventDataJson")).unwrap()).unwrap(),
            "sdkEvent"=>executor.store.event(&job,"sdk_raw_result",&serde_json::from_str::<Value>(s(&audit,"originalSdkRawEventDataJson")).unwrap()).unwrap(),
            file=>std::fs::write(executor.store.root.join("text-runs").join(&job).join(file),b"polluted").unwrap(),
        }
        assert!(!executor.store.commit_sdk_f2_episode_replay(&audit,&raw,None).unwrap_or(false),"{change}");
        assert!(!executor.store.root.join("sdk-worker").exists());
    }
    let (_temp,executor,request,job,raw)=fixture_kind(false,true);
    executor.accept(&request,&raw).await.unwrap();
    let c=executor.store.conn().unwrap();
    let data:String=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type=?2",params![job,FRAGMENT.accepted_event],|r|r.get(0)).unwrap();
    executor.store.event(&job,FRAGMENT.accepted_event,&serde_json::from_str::<Value>(&data).unwrap()).unwrap();
    assert!(executor.execute(request).await.is_err());
}

#[tokio::test]
async fn sdk_fragment_replay_cas_failure_is_atomic() {
    let (_temp,executor,request,job,raw)=fixture_kind(false,true);
    let before=preserved(&executor,&job);
    executor.store.conn().unwrap().execute_batch("CREATE TRIGGER replay_interrupted BEFORE UPDATE ON workflow_steps BEGIN SELECT RAISE(ROLLBACK,'synthetic fragment checkpoint conflict'); END;").unwrap();
    assert!(executor.accept(&request,&raw).await.unwrap_err().contains("synthetic fragment checkpoint conflict"));
    assert_eq!(executor.sdk_job(&job).unwrap()["status"],"rejected");
    assert_eq!(preserved(&executor,&job),before);
    let n:i64=executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type=?2",params![job,FRAGMENT.accepted_event],|r|r.get(0)).unwrap();
    assert_eq!(n,0);
}

#[test]
#[ignore = "requires explicit captured fragment sqlite/root/job; isolated CAS proof, no production write or dispatch"]
fn sdk_fragment_captured_original_proof_cas_reopen_preserves_raw() {
    let required=|name|std::env::var(name).expect("explicit private proof fixture path required");
    let source_db=PathBuf::from(required("DIRECTOR_SDK_FRAGMENT_PROOF_DB"));
    let source_root=PathBuf::from(required("DIRECTOR_SDK_FRAGMENT_PROOF_ROOT"));
    let job=required("DIRECTOR_SDK_FRAGMENT_PROOF_JOB");
    let source_db_hash=crate::store::hash_file(&source_db).unwrap();
    let temp=tempfile::tempdir().unwrap();let root=temp.path().join("isolated");std::fs::create_dir(&root).unwrap();
    std::fs::copy(&source_db,root.join("project.sqlite")).unwrap();
    let store=Store::open(root.clone()).unwrap();let c=store.conn().unwrap();
    let (snapshot,parent,run):(String,String,String)=c.query_row("SELECT j.snapshot,p.snapshot,p.id FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id JOIN jobs p ON p.id=w.workflow_id WHERE j.id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND j.state='rejected'",[&job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let snapshot:Value=serde_json::from_str(&snapshot).unwrap();let parent:Value=serde_json::from_str(&parent).unwrap();
    let request:TextRequest=serde_json::from_value(snapshot.clone()).unwrap();
    let executor=SavedTextExecutor{store,settings:parent["input"]["settings"].clone(),workflow_id:Some(run)};
    let target=root.join("text-runs").join(&job);std::fs::create_dir_all(&target).unwrap();
    for name in ["sdk-attempt.json","sdk-compiled.json","result.json"] {std::fs::copy(source_root.join("text-runs").join(&job).join(name),target.join(name)).unwrap();}
    let original=executor.sdk_job(&job).unwrap();let before=preserved(&executor,&job);
    let audit=executor.sdk_f2_replay_audit(&request,&snapshot,&original).unwrap();
    assert_eq!(audit["policy"],FRAGMENT_POLICY);
    assert!(audit_valid(&audit),"actual handoff, worker scope and original request/raw proof");
    assert!(files_bound(&root,&audit));
    assert!(database_bound(&executor.store.conn().unwrap(),&audit,false,&original["prepared"],&original["receipt"],&original["result"]).unwrap());
    // Real sdk_raw must refuse the isolated root: frozen workerRoot is absolute.
    // This test proves Store CAS/readback only, not live ancestry or domain acceptance.
    assert!(executor.sdk_parallel_worker_bound(&original,&snapshot).is_err());
    assert!(executor.store.commit_sdk_f2_episode_replay(&audit,&original["result"],None).unwrap());
    let reopened=SavedTextExecutor{store:Store::open(root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
    let adopted=reopened.sdk_job(&job).unwrap();
    assert_eq!(adopted["status"],"completed");
    assert!(database_bound(&reopened.store.conn().unwrap(),&audit,true,&adopted["prepared"],&adopted["receipt"],&original["result"]).unwrap());
    assert!(!reopened.store.commit_sdk_f2_episode_replay(&audit,&original["result"],None).unwrap());
    assert_eq!(preserved(&reopened,&job),before);
    assert!(!root.join("sdk-worker").exists());
    assert_eq!(crate::store::hash_file(&source_db).unwrap(),source_db_hash);
    for name in ["sdk-attempt.json","sdk-compiled.json","result.json"] {assert_eq!(crate::store::hash_file(&source_root.join("text-runs").join(&job).join(name)).unwrap(),crate::store::hash_file(&target.join(name)).unwrap());}
}
