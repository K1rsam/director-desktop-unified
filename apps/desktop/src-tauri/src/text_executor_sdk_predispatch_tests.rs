//! Synthetic only: no user project, account, model, browser, or production EXE.
use super::*;

fn fixture()->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value){
    let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();
    store.create("合成SDK派发前准备恢复","synthetic-pre-dispatch").unwrap();store.ensure_workflow_storage().unwrap();
    let settings=crate::text_sdk::offline_settings("happy");
    let run=store.add_job("workflow","foundation.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","input":{"settings":settings}}),"running").unwrap();
    let executor=SavedTextExecutor{store,settings,workflow_id:Some(run)};
    let request=TextRequest{stage:"foundation:SDK_PRE_DISPATCH".into(),skill:"asset-art-director".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input:json!({"text":"合成输入，保持原字节","sourceSha256":"synthetic-source"}),schema:json!({"type":"object","properties":{"ok":{"type":"boolean"},"text":{"type":"string"}},"required":["ok","text"],"additionalProperties":false}),rule_snapshot:crate::pipeline::snapshot("asset-art-director",&[]).unwrap()};
    let snapshot=executor.request_snapshot(&request).unwrap();(temp,executor,request,snapshot)
}
fn calls(executor:&SavedTextExecutor)->usize{
    std::fs::read_dir(executor.store.root.join("sdk-worker")).into_iter().flatten().flatten()
        .filter(|entry|entry.path().join("fixture-invocation.json").is_file()).count()
}
fn count(executor:&SavedTextExecutor,kind:&str)->i64{
    executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM jobs WHERE kind=?1",[kind],|r|r.get(0)).unwrap()
}
fn compiled(executor:&SavedTextExecutor,request:&TextRequest,job:&str)->Value{
    let c=compile_codex_input(request).unwrap();let dir=executor.store.root.join("text-runs").join(job);
    std::fs::create_dir_all(&dir).unwrap();let value=json!({"prompt":c.prompt,"schema":c.schema,"promptSha256":crate::pipeline::digest(&c.prompt),"schemaSha256":crate::pipeline::digest(&c.schema)});
    crate::store::atomic_json(&dir.join("sdk-compiled.json"),&value).unwrap();
    json!({"sdkRequestHash":crate::pipeline::digest(&executor.sdk_job(job).unwrap()["snapshot"].to_string()),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":executor.workflow_id,"compiledPromptSha256":value["promptSha256"],"compiledSchemaSha256":value["schemaSha256"]})
}

#[tokio::test]
async fn sdk_pre_dispatch_resumes_same_bound_queued_job_and_preserves_partial_bytes_once(){
    let (_temp,executor,request,snapshot)=fixture();let job=executor.sdk_allocate_pre_dispatch(&request,&snapshot).unwrap();
    let dir=executor.store.root.join("text-runs").join(&job);std::fs::create_dir_all(&dir).unwrap();
    let partial=dir.join(format!("sdk-compiled.json.{}.partial",id()));let original=b"original compiled partial evidence";std::fs::write(&partial,original).unwrap();
    // A shared inspect worker is not evidence that this request was executed.
    std::fs::create_dir_all(executor.store.root.join("sdk-worker")).unwrap();
    let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
    reopened.store.recover_interrupted().unwrap();
    let raw=reopened.execute(request.clone()).await.unwrap();reopened.accept(&request,&raw).await.unwrap();
    assert_eq!(reopened.execute(request).await.unwrap(),raw);assert_eq!(calls(&reopened),1);
    assert_eq!(count(&reopened,"workflow"),1);assert_eq!(count(&reopened,"text"),1);
    assert_eq!(reopened.sdk_job(&job).unwrap()["snapshot"],snapshot);
    assert_eq!(std::fs::read(&partial).unwrap(),original);
    let event:Value=serde_json::from_str(&reopened.store.conn().unwrap().query_row("SELECT data FROM events WHERE job_id=?1 AND type='sdk_pre_dispatch_resumed'",[&job],|r|r.get::<_,String>(0)).unwrap()).unwrap();
    assert_eq!(event["originalJobReused"],true);assert_eq!(event["modelCallsBeforeMarker"],0);
    assert_eq!(event["preservedCompiledArtifacts"].as_array().unwrap().len(),1);
}

#[tokio::test]
async fn sdk_pre_dispatch_legacy_unbound_queued_and_foreign_workflow_never_dispatch(){
    let (_temp,executor,request,snapshot)=fixture();
    let job=executor.store.add_job("text","unbound-legacy","codex_sdk","synthetic-no-account",&snapshot,"queued").unwrap();
    let before=executor.store.jobs().unwrap();assert!(executor.execute(request.clone()).await.unwrap_err().contains("SDK_PRE_DISPATCH_HELD"));
    assert_eq!(executor.store.jobs().unwrap(),before);assert_eq!(calls(&executor),0);
    assert!(!executor.store.root.join("text-runs").join(job).exists());
    let (_temp,executor,request,snapshot)=fixture();let job=executor.sdk_allocate_pre_dispatch(&request,&snapshot).unwrap();
    let other=executor.store.add_job("workflow","foundation.run","local","local",&json!({"input":{"settings":executor.settings}}),"running").unwrap();
    let foreign=SavedTextExecutor{store:executor.store.clone(),settings:executor.settings.clone(),workflow_id:Some(other)};
    assert!(foreign.execute(request).await.unwrap_err().contains("SDK_PRE_DISPATCH_HELD"));
    assert_eq!(executor.sdk_job(&job).unwrap()["status"],"queued");assert_eq!(calls(&executor),0);
}

#[test]
fn sdk_pre_dispatch_all_db_dispatch_or_checkpoint_pollution_fails_closed(){
    for fault in ["state","prepared","receipt","result","task","error","snapshot","checkpoint-running","checkpoint-hash","event","missing-marker","parent"] {
        let (_temp,executor,request,snapshot)=fixture();let job=executor.sdk_allocate_pre_dispatch(&request,&snapshot).unwrap();let c=executor.store.conn().unwrap();
        match fault {
            "state"=>{c.execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",[&job]).unwrap();},
            "prepared"=>{c.execute("UPDATE jobs SET prepared='{}' WHERE id=?1",[&job]).unwrap();},
            "receipt"=>{c.execute("UPDATE jobs SET receipt='{}' WHERE id=?1",[&job]).unwrap();},
            "result"=>{c.execute("UPDATE jobs SET result='{}' WHERE id=?1",[&job]).unwrap();},
            "task"=>{c.execute("UPDATE jobs SET task_id='' WHERE id=?1",[&job]).unwrap();},
            "error"=>{c.execute("UPDATE jobs SET error='unclassified' WHERE id=?1",[&job]).unwrap();},
            "snapshot"=>{c.execute("UPDATE jobs SET snapshot='{}' WHERE id=?1",[&job]).unwrap();},
            "checkpoint-running"=>{c.execute("UPDATE workflow_steps SET state='running' WHERE text_job_id=?1",[&job]).unwrap();},
            "checkpoint-hash"=>{c.execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&job]).unwrap();},
            "event"=>{executor.store.event(&job,"sdk_dispatch_receipt",&json!({"ok":true})).unwrap();},
            "missing-marker"=>{c.execute("DELETE FROM events WHERE job_id=?1",[&job]).unwrap();},
            "parent"=>{c.execute("UPDATE jobs SET snapshot='{}' WHERE id=?1",[executor.workflow_id.as_deref().unwrap()]).unwrap();},
            _=>unreachable!(),
        }
        assert!(executor.sdk_pre_dispatch_evidence(&request,&snapshot,&job,&c).is_err(),"{fault}");
        assert_eq!(calls(&executor),0,"{fault}");
    }
}

#[test]
fn sdk_pre_dispatch_attempt_partial_worker_or_unknown_file_blocks_preparation(){
    let temp=tempfile::tempdir().unwrap();
    assert!(sdk_pre_dispatch_artifacts(temp.path(),"../outside").is_err());
    for fault in ["attempt","attempt-partial","worker","result","unknown-partial"] {
        let (_temp,executor,request,snapshot)=fixture();let job=executor.sdk_allocate_pre_dispatch(&request,&snapshot).unwrap();
        let dir=executor.store.root.join("text-runs").join(&job);std::fs::create_dir_all(&dir).unwrap();
        match fault {
            "worker"=>std::fs::create_dir_all(executor.store.root.join("sdk-worker").join(&job)).unwrap(),
            "attempt"=>std::fs::write(dir.join("sdk-attempt.json"),"{}").unwrap(),
            "attempt-partial"=>std::fs::write(dir.join(format!("sdk-attempt.json.{}.partial",id())),"{}").unwrap(),
            "result"=>std::fs::write(dir.join("result.json"),"{}").unwrap(),
            "unknown-partial"=>std::fs::write(dir.join("sdk-compiled.json.unknown.partial"),"{}").unwrap(),
            _=>unreachable!(),
        }
        assert!(executor.sdk_pre_dispatch_evidence(&request,&snapshot,&job,&executor.store.conn().unwrap()).is_err(),"{fault}");
        assert_eq!(executor.sdk_job(&job).unwrap()["status"],"queued");
    }
}

#[test]
fn sdk_pre_dispatch_marker_cas_rechecks_original_partial_and_db_evidence(){
    for fault in ["partial-changed","late-event","compiled-changed"] {
        let (_temp,executor,request,snapshot)=fixture();let job=executor.sdk_allocate_pre_dispatch(&request,&snapshot).unwrap();let dir=executor.store.root.join("text-runs").join(&job);std::fs::create_dir_all(&dir).unwrap();
        let partial=dir.join(format!("sdk-compiled.json.{}.partial",id()));std::fs::write(&partial,"preserved").unwrap();
        let evidence=executor.sdk_pre_dispatch_evidence(&request,&snapshot,&job,&executor.store.conn().unwrap()).unwrap();let prepared=compiled(&executor,&request,&job);
        match fault {
            "partial-changed"=>std::fs::write(&partial,"changed").unwrap(),
            "late-event"=>executor.store.event(&job,"sdk_event",&json!({"sequence":1})).unwrap(),
            "compiled-changed"=>std::fs::write(dir.join("sdk-compiled.json"),"{}").unwrap(),
            _=>unreachable!(),
        }
        assert!(executor.sdk_mark_pre_dispatch(&request,&snapshot,&job,&prepared,&evidence,true).is_err(),"{fault}");
        let saved=executor.sdk_job(&job).unwrap();assert_eq!(saved["status"],"queued");assert!(saved["prepared"].is_null());assert_eq!(calls(&executor),0);
    }
}

#[tokio::test]
async fn sdk_pre_dispatch_after_marker_crash_is_never_a_preparation_retry(){
    let (_temp,executor,request,snapshot)=fixture();let job=executor.sdk_allocate_pre_dispatch(&request,&snapshot).unwrap();
    let evidence=executor.sdk_pre_dispatch_evidence(&request,&snapshot,&job,&executor.store.conn().unwrap()).unwrap();let prepared=compiled(&executor,&request,&job);
    executor.sdk_mark_pre_dispatch(&request,&snapshot,&job,&prepared,&evidence,true).unwrap();
    assert_eq!(executor.sdk_job(&job).unwrap()["status"],"submitting");
    assert!(executor.execute(request).await.is_err());assert_eq!(calls(&executor),0);assert_eq!(count(&executor,"text"),1);
    assert_eq!(executor.sdk_job(&job).unwrap()["prepared"],prepared);
}
