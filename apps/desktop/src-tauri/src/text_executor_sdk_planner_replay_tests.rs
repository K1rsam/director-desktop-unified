use super::*;

fn material() -> (crate::pipeline::LockedSource,Value,Value) {
    let source=crate::pipeline::prepare_source("第一集\n场景：门口\n甲：一。\n乙：二。\n甲：三。\n乙：四。\n").unwrap();
    let rows:Vec<_>=source.rows.iter().filter(|r|r.kind=="body").collect();
    let refs:Vec<_>=rows.iter().map(|r|r.id.clone()).collect();
    let emotion=json!({"subject":"甲乙","contextTone":"回应","audienceEffect":"听清变化","change":"逐句回应","landing":"结果","retain":["原句"]});
    let events:Vec<_>=(0..2).map(|i|json!({"id":format!("E{i}"),"sceneId":source.scenes[0].id,"sourceRefs":refs[i*2..i*2+2],"boundaryBefore":{"kind":if i==0{"episode_start"}else{"new_event"},"reason":"原文事件"},"emotionContract":emotion})).collect();
    let packages:Vec<_>=(0..2).map(|i|json!({"id":format!("W{i}"),"eventId":format!("E{i}"),"sourceRefs":refs[i*2..i*2+2],"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":emotion})).collect();
    let units:Vec<_>=(0..2).map(|i|json!({"id":format!("P{i}"),"packageId":format!("W{i}"),"sourceRefs":refs[i*2..i*2+2],"secondsRange":[4,6]})).collect();
    let input=json!({"episode":source.episodes[0],"sourceRows":rows,"contextRows":[],"storyboardCreativeProfile":"original-production-contract/v1","settings":{"fantasyMode":"recommend"}});
    let mut raw=json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"依源回应","events":events,"workPackages":packages,"provisionalUnits":units,"entryFacts":[],"fantasyCandidates":[]});
    raw["workPackages"][0]["sourceRefs"].as_array_mut().unwrap().swap(0,1);
    (source,input,raw)
}

fn fixture() -> (tempfile::TempDir,SavedTextExecutor,TextRequest,String,Value,crate::pipeline::LockedSource) {
    let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("isolated")).unwrap();store.create("合成 Planner 源序恢复","synthetic-planner").unwrap();store.ensure_workflow_storage().unwrap();
    let (locked,input,raw)=material();let source=store.save_source(json!(locked)).unwrap();
    let settings=crate::text_sdk::offline_settings("happy");let rules=crate::pipeline::snapshot("episode-emotion-planner",&[]).unwrap();
    let request=TextRequest {stage:"storyboardPlan".into(),skill:"episode-emotion-planner".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input,schema:json!({"type":"object"}),rule_snapshot:rules.clone()};
    let run=store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","sourceVersion":source["version"],"rules":[rules["primary"]],"input":{"source":source,"settings":settings}}),"partially_blocked").unwrap();
    let executor=SavedTextExecutor {store,settings,workflow_id:Some(run.clone())};let snapshot=executor.request_snapshot(&request).unwrap();let hash=crate::pipeline::digest(&snapshot.to_string());
    let compiled=compile_codex_input(&request).unwrap();let job=executor.store.add_job("text","storyboardPlan","codex_sdk","codex_sdk",&snapshot,"rejected").unwrap();
    let envelope=json!({"domainAccepted":false,"parseError":null,"parsedJson":raw,"rawText":raw.to_string()});
    let prepared=json!({"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":run,"sdkRawTextSha256":crate::pipeline::digest(&raw.to_string()),"sdkRequestHash":hash});
    executor.store.update_job(&job,"rejected",&json!({"result":raw,"error":PLAN_REJECTION,"prepared":prepared,"receipt":{"contract":"desktop-runner/v1","validation":"rejected"}})).unwrap();
    executor.store.checkpoint(&run,&step_key(&request),&request.stage,&hash,"rejected",Some(&job),Some(&raw),Some(PLAN_REJECTION)).unwrap();
    executor.store.event(&job,"sdk_raw_result",&envelope).unwrap();executor.store.event(&job,"text_received_raw",&json!({"requestHash":hash,"value":raw})).unwrap();
    executor.store.conn().unwrap().execute("INSERT INTO metadata VALUES('synthetic-planner-repair-budget','2')",[]).unwrap();
    let base=executor.store.root.join("text-runs").join(&job);std::fs::create_dir_all(&base).unwrap();std::fs::write(base.join("result.json"),raw.to_string()).unwrap();
    crate::store::atomic_json(&base.join("sdk-compiled.json"),&json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":prepared["compiledPromptSha256"],"schemaSha256":prepared["compiledSchemaSha256"]})).unwrap();
    let mut attempt=FrozenAttempt::prepare(&job,&run,&step_key(&request),Backend::CodexSdk,request.clone(),&snapshot["executorIdentity"]).unwrap();attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.state=AttemptState::Completed;attempt.raw_result=Some(envelope);attempt.persist(&base.join("sdk-attempt.json")).unwrap();
    (temp,executor,request,job,raw,locked)
}

fn preserved(executor:&SavedTextExecutor,job:&str)->Value {
    let c=executor.store.conn().unwrap();let pair:(String,String)=c.query_row("SELECT snapshot,result FROM jobs WHERE id=?1",[job],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let files:Vec<_>=["sdk-attempt.json","sdk-compiled.json","result.json"].iter().map(|name|crate::store::hash_file(&executor.store.root.join("text-runs").join(job).join(name)).unwrap()).collect();
    let counts:(i64,i64)=c.query_row("SELECT (SELECT COUNT(*) FROM jobs),(SELECT COUNT(*) FROM workflow_steps)",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let budget:String=c.query_row("SELECT value FROM metadata WHERE key='synthetic-planner-repair-budget'",[],|r|r.get(0)).unwrap();
    json!({"snapshot":pair.0,"raw":pair.1,"files":files,"jobsAndNodes":counts,"budget":budget})
}

async fn validate(executor:&SavedTextExecutor,request:&TextRequest,source:&crate::pipeline::LockedSource)->Result<Value> {
    crate::pipeline::call_validated(executor,&request.stage,&request.skill,&request.model,&request.reasoning,request.input.clone(),request.schema.clone(),&[],|value| {
        *value=crate::pipeline::planner_source_order_replay_result(source,&request.input,value)?;Ok(())
    }).await
}

#[tokio::test]
async fn sdk_planner_order_replay_uses_original_rejected_node_and_reopens_without_dispatch() {
    let (_temp,executor,request,job,raw,source)=fixture();let before=preserved(&executor,&job);let original=executor.sdk_job(&job).unwrap();
    let adopted=validate(&executor,&request,&source).await.unwrap();let completed=executor.sdk_job(&job).unwrap();
    assert_eq!(completed["status"],"completed");assert_eq!(completed["result"],raw);assert_eq!(completed["receipt"]["migrationPolicy"],PLAN_POLICY);
    assert_eq!(completed["prepared"]["sdkDomainResult"],adopted);
    for (key,value) in original["prepared"].as_object().unwrap() {assert_eq!(&completed["prepared"][key],value);}
    assert_eq!(adopted["packageSourceOrderNormalization"]["changes"].as_array().unwrap().len(),1);
    assert_eq!(adopted["packageSourceOrderNormalization"]["originalPlanSha256"],crate::pipeline::digest(&raw.to_string()));
    assert_eq!(preserved(&executor,&job),before);assert!(!executor.store.root.join("sdk-worker").exists());
    let restarted=SavedTextExecutor {store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
    let jobs_before=restarted.store.jobs().unwrap();let event_count=||restarted.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events",[],|r|r.get::<_,i64>(0)).unwrap();let events_before=event_count();
    assert_eq!(validate(&restarted,&request,&source).await.unwrap(),adopted);
    assert_eq!(restarted.store.jobs().unwrap(),jobs_before);assert_eq!(event_count(),events_before);assert_eq!(preserved(&restarted,&job),before);
}

#[test]
fn sdk_planner_order_replay_rejects_wrong_members_missing_duplicates_cross_packages_and_events() {
    let (source,input,raw)=material();
    for change in ["missing","duplicate","unknown","cross-package","reverse-packages","reverse-events","noncontiguous","source-text","creative-tamper"] {
        let mut candidate=raw.clone();let mut request=input.clone();
        match change {
            "missing"=>{candidate["workPackages"][0]["sourceRefs"].as_array_mut().unwrap().pop();},
            "duplicate"=>candidate["workPackages"][0]["sourceRefs"][0]=candidate["workPackages"][0]["sourceRefs"][1].clone(),
            "unknown"=>candidate["workPackages"][0]["sourceRefs"][0]=json!("unknown"),
            "cross-package"=>candidate["workPackages"][0]["sourceRefs"][0]=candidate["workPackages"][1]["sourceRefs"][0].clone(),
            "reverse-packages"=>candidate["workPackages"].as_array_mut().unwrap().reverse(),
            "reverse-events"=>candidate["events"].as_array_mut().unwrap().reverse(),
            "noncontiguous"=>{let last=candidate["workPackages"][1]["sourceRefs"][1].clone();let first=candidate["workPackages"][0]["sourceRefs"][0].clone();candidate["workPackages"][0]["sourceRefs"][0]=last;candidate["workPackages"][1]["sourceRefs"][1]=first;},
            "source-text"=>request["sourceRows"][0]["text"]=json!("changed"),
            "creative-tamper"=>candidate["workPackages"][0]["emotionContract"]["retain"]=json!([]),
            _=>unreachable!(),
        }
        let before=candidate.clone();assert!(crate::pipeline::planner_source_order_replay_result(&source,&request,&candidate).is_err(),"{change}");assert_eq!(candidate,before);
    }
}

#[tokio::test]
async fn sdk_planner_order_replay_unknown_drift_forged_domain_and_atomic_failure_stay_protected() {
    for change in ["unknown","checkpoint","raw-event","compiled","forged-output","atomic"] {
        let (_temp,executor,request,job,raw,source)=fixture();let before=preserved(&executor,&job);
        let expected=crate::pipeline::planner_source_order_replay_result(&source,&request.input,&raw).unwrap();let mut result=expected;
        match change {
            "unknown"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",[&job]).unwrap();},
            "checkpoint"=>{executor.store.conn().unwrap().execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&job]).unwrap();},
            "raw-event"=>executor.store.event(&job,"text_received_raw",&json!({"requestHash":"wrong","value":raw})).unwrap(),
            "compiled"=>std::fs::write(executor.store.root.join("text-runs").join(&job).join("sdk-compiled.json"),b"{}").unwrap(),
            "forged-output"=>result["emotionArc"]=json!("改写剧情"),
            "atomic"=>executor.store.conn().unwrap().execute_batch("CREATE TRIGGER replay_conflict BEFORE UPDATE ON workflow_steps BEGIN SELECT RAISE(ROLLBACK,'synthetic checkpoint conflict'); END;").unwrap(),
            _=>unreachable!(),
        }
        assert!(executor.accept(&request,&result).await.is_err(),"{change}");
        assert_ne!(executor.sdk_job(&job).unwrap()["status"],"completed");assert!(!executor.store.root.join("sdk-worker").exists());
        if change!="compiled" {assert_eq!(preserved(&executor,&job),before,"{change}");}
    }
}

#[test]
#[ignore="requires explicit captured Planner artifact and database; read-only proof, no dispatch or adoption"]
fn sdk_planner_order_captured_original_is_source_bound_and_only_internal_refs_change() {
    let fixture=std::env::var("DIRECTOR_PLANNER_ORDER_CAPTURE").unwrap();let root=PathBuf::from(std::env::var("DIRECTOR_PLANNER_ORDER_ROOT").unwrap());
    let artifact:Value=serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();let job=s(&artifact["prepared"],"sdkAttemptId");let run=s(&artifact["prepared"],"sdkOriginWorkflowId");
    let c=rusqlite::Connection::open_with_flags(root.join("project.sqlite"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (snapshot,raw,prepared,receipt):(String,String,String,String)=c.query_row("SELECT snapshot,result,prepared,receipt FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk' AND state='rejected'",[job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let parent:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|r.get(0)).unwrap();let parent_value:Value=serde_json::from_str(&parent).unwrap();
    let snapshot_value:Value=serde_json::from_str(&snapshot).unwrap();let raw_value:Value=serde_json::from_str(&raw).unwrap();let request:TextRequest=serde_json::from_value(snapshot_value.clone()).unwrap();
    assert_eq!(snapshot_value,artifact["request"]);assert_eq!(raw_value,artifact["raw"]);
    let event=|kind|c.query_row("SELECT id,data FROM events WHERE job_id=?1 AND type=?2 ORDER BY id DESC LIMIT 1",params![job,kind],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap();let (event_id,event_data)=event("text_received_raw");let (sdk_id,sdk_data)=event("sdk_raw_result");let base=root.join("text-runs").join(job);
    let audit=json!({"policy":PLAN_POLICY,"stage":"storyboardPlan","previousError":PLAN_REJECTION,"workflowId":run,"nodeKey":step_key(&request),"originalJobId":job,"requestHash":crate::pipeline::digest(&snapshot),"originalSnapshotJson":snapshot,"originalResultJson":raw,"originalPreparedJson":prepared,"originalReceiptJson":receipt,"originalWorkflowSnapshotJson":parent,"originalRawEventId":event_id,"originalRawEventDataJson":event_data,"originalSdkRawEventId":sdk_id,"originalSdkRawEventDataJson":sdk_data,"originalAttemptSha256":crate::store::hash_file(&base.join("sdk-attempt.json")).unwrap().0,"originalCompiledSha256":crate::store::hash_file(&base.join("sdk-compiled.json")).unwrap().0,"originalRawTextSha256":crate::store::hash_file(&base.join("result.json")).unwrap().0,"generationRequestsSent":0,"acceptedAutomatically":false});
    assert!(audit_valid(&audit),"captured request/source/rules/raw binding");assert!(files_bound(&root,&audit),"captured transport files");
    assert!(database_bound(&c,&audit,false,&serde_json::from_str(&prepared).unwrap(),&serde_json::from_str(&receipt).unwrap(),&raw_value).unwrap());
    let source=serde_json::from_value(parent_value["input"]["source"].clone()).unwrap();let result=crate::pipeline::planner_source_order_replay_result(&source,&request.input,&raw_value).unwrap();
    assert_eq!(result["packageSourceOrderNormalization"]["changes"].as_array().unwrap().len(),1);assert_eq!(result["events"],raw_value["events"]);assert_eq!(result["provisionalUnits"],raw_value["provisionalUnits"]);assert_eq!(result["fantasyCandidates"],raw_value["fantasyCandidates"]);
    assert!(domain_result_matches(&snapshot_value,&parent_value,&raw_value,&result));
    println!("{}",json!({"sourceRows":request.input["sourceRows"].as_array().unwrap().len(),"normalization":result["packageSourceOrderNormalization"],"rawSha256":crate::pipeline::digest(&raw),"normalizedSha256":crate::pipeline::digest(&result.to_string()),"readOnly":true,"generationRequestsSent":0}));
}

#[tokio::test]
#[ignore="requires explicit captured Planner artifact and database; adopts only in an isolated SQLite snapshot, zero dispatch"]
async fn sdk_planner_order_captured_original_adopts_and_reopens_only_in_isolated_copy() {
    let artifact:Value=serde_json::from_slice(&std::fs::read(std::env::var("DIRECTOR_PLANNER_ORDER_CAPTURE").unwrap()).unwrap()).unwrap();
    let original_root=PathBuf::from(std::env::var("DIRECTOR_PLANNER_ORDER_ROOT").unwrap());
    let job=s(&artifact["prepared"],"sdkAttemptId");let run=s(&artifact["prepared"],"sdkOriginWorkflowId");
    let c=rusqlite::Connection::open_with_flags(original_root.join("project.sqlite"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let original_state:Vec<String>=c.query_row("SELECT snapshot,result,prepared,receipt,state FROM jobs WHERE id=?1",[job],|r|(0..5).map(|i|r.get(i)).collect()).unwrap();
    let temp=tempfile::tempdir().unwrap();let root=temp.path().join("isolated-captured-planner");std::fs::create_dir(&root).unwrap();
    // VACUUM INTO reads a consistent SQLite snapshot including committed WAL;
    // the original connection remains SQLITE_OPEN_READ_ONLY throughout.
    c.execute("VACUUM main INTO ?1",[root.join("project.sqlite").to_str().unwrap()]).unwrap();
    let base=root.join("text-runs").join(job);std::fs::create_dir_all(&base).unwrap();
    let original_files:Vec<_>=["sdk-attempt.json","sdk-compiled.json","result.json"].into_iter().map(|name| {
        let original=original_root.join("text-runs").join(job).join(name);
        let hash=crate::store::hash_file(&original).unwrap();std::fs::copy(original,base.join(name)).unwrap();hash
    }).collect();
    let store=Store::open(root.clone()).unwrap();
    let parent:Value=serde_json::from_str(&store.conn().unwrap().query_row("SELECT snapshot FROM jobs WHERE id=?1",[run],|r|r.get::<_,String>(0)).unwrap()).unwrap();
    let executor=SavedTextExecutor {store,settings:parent["input"]["settings"].clone(),workflow_id:Some(run.to_string())};
    let request:TextRequest=serde_json::from_value(artifact["request"].clone()).unwrap();let source=serde_json::from_value(parent["input"]["source"].clone()).unwrap();
    let old=executor.sdk_job(job).unwrap();assert_eq!(old["snapshot"],artifact["request"]);assert_eq!(old["result"],artifact["raw"]);assert_eq!(old["status"],"rejected");
    let audit=executor.sdk_f2_replay_audit(&request,&old["snapshot"],&old).unwrap();
    assert!(audit_valid(&audit));assert!(files_bound(&root,&audit));assert!(database_bound(&executor.store.conn().unwrap(),&audit,false,&old["prepared"],&old["receipt"],&old["result"]).unwrap());
    let state=|| {let c=executor.store.conn().unwrap();json!({
        "jobs":c.query_row("SELECT COUNT(*) FROM jobs",[],|r|r.get::<_,i64>(0)).unwrap(),
        "nodes":c.query_row("SELECT COUNT(*) FROM workflow_steps",[],|r|r.get::<_,i64>(0)).unwrap(),
        "metadata":c.prepare("SELECT key,value FROM metadata WHERE key <> 'changed_at' ORDER BY key").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()})};
    let before=state();let raw=executor.execute(request.clone()).await.unwrap();assert_eq!(raw,artifact["raw"]);
    let adopted=crate::pipeline::planner_source_order_replay_result(&source,&request.input,&raw).unwrap();executor.accept(&request,&adopted).await.unwrap();
    let accepted=executor.sdk_job(job).unwrap();assert_eq!(accepted["status"],"completed");assert_eq!(accepted["result"],raw);assert_eq!(accepted["snapshot"],old["snapshot"]);
    for (key,value) in old["prepared"].as_object().unwrap() {assert_eq!(&accepted["prepared"][key],value);}
    assert!(executor.sdk_f2_migrated_receipt_bound(&accepted,&old["snapshot"]).unwrap());
    let restarted=SavedTextExecutor {store:Store::open(root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
    let event_count=||executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events",[],|r|r.get::<_,i64>(0)).unwrap();let events_before=event_count();
    for _ in 0..2 {assert_eq!(restarted.execute(request.clone()).await.unwrap(),raw);restarted.accept(&request,&adopted).await.unwrap();}
    assert_eq!(restarted.sdk_job(job).unwrap(),accepted);assert_eq!(event_count(),events_before);assert_eq!(state(),before);assert!(!root.join("sdk-worker").exists());
    let final_original:Vec<String>=c.query_row("SELECT snapshot,result,prepared,receipt,state FROM jobs WHERE id=?1",[job],|r|(0..5).map(|i|r.get(i)).collect()).unwrap();assert_eq!(final_original,original_state);
    for (index,name) in ["sdk-attempt.json","sdk-compiled.json","result.json"].into_iter().enumerate() {assert_eq!(crate::store::hash_file(&original_root.join("text-runs").join(job).join(name)).unwrap(),original_files[index]);assert_eq!(crate::store::hash_file(&base.join(name)).unwrap(),original_files[index]);}
    println!("{}",json!({"originalJobId":job,"originalProjectReadOnly":true,"isolatedAcceptAndTwoReopens":true,"sameNode":true,"generationRequestsSent":0,"normalization":adopted["packageSourceOrderNormalization"]}));
}
