fn planner_repair_request(source:&crate::pipeline::LockedSource,origin:&TextRequest,origin_raw:&Value,prior:&TextRequest,raw:&Value,reason:&str,attempt:u64)->TextRequest {
    let mut r=origin.clone();r.input["previousPlan"]=raw.clone();
    r.input["planRepair"]=json!({"version":"rejected-plan-repair/v1","attempt":attempt,"maximumAttempts":2,"sourceSha256":source.sha256,
        "originDomainRequestSha256":crate::pipeline::digest(&serde_json::to_string(origin).unwrap()),"originalPlanSha256":hash(origin_raw),"previousRequestSha256":crate::pipeline::digest(&serde_json::to_string(prior).unwrap()),"previousPlanSha256":hash(raw)});
    crate::pipeline::planner_request_for_handoff_test(r,Some(reason),raw).unwrap()
}
fn rejected_plan_complete(executor:&SavedTextExecutor,request:&TextRequest,snapshot:&Value,job:&str,raw:&Value,reason:&str,handed_off:bool) {
    complete(executor,request,snapshot,job,handed_off);
    let dir=executor.store.root.join("text-runs").join(job);
    let mut attempt=FrozenAttempt::load(&dir.join("sdk-attempt.json")).unwrap();
    attempt.raw_result=Some(json!({"rawText":raw.to_string(),"parsedJson":raw,"domainAccepted":false,"parseError":null}));
    attempt.persist(&dir.join("sdk-attempt.json")).unwrap();std::fs::write(dir.join("result.json"),raw.to_string()).unwrap();
    let mut prepared=executor.sdk_job(job).unwrap()["prepared"].clone();
    prepared.as_object_mut().unwrap().remove("sdkDomainResult");prepared.as_object_mut().unwrap().remove("sdkDomainResultSha256");
    prepared["sdkRawTextSha256"]=json!(hash(raw));
    executor.store.conn().unwrap().execute("UPDATE jobs SET state='rejected',prepared=?1,result=?2,receipt=?3,error=?4 WHERE id=?5",params![prepared.to_string(),raw.to_string(),json!({"contract":"desktop-runner/v1","validation":"rejected"}).to_string(),reason,job]).unwrap();
    executor.store.checkpoint(executor.workflow_id.as_deref().unwrap(),&step_key(request),&request.stage,&hash(snapshot),"rejected",Some(job),Some(raw),Some(reason)).unwrap();
    executor.store.event(job,"text_received_raw",&json!({"requestHash":hash(snapshot),"value":raw})).unwrap();
}
fn planner_repair_handoff(executor:&SavedTextExecutor,request:&TextRequest,template:&Value)->Value {
    let original=executor.request_snapshot(request).unwrap();
    let (policy,prior)=executor.sdk_handoff_predecessor(&original,false).unwrap();assert_eq!(policy,REJECTED_PLAN_POLICY);
    let parent=executor.store.job(executor.workflow_id.as_deref().unwrap()).unwrap()["snapshot"].clone();
    let mut snapshot=original.clone();snapshot["executorIdentity"]=template["executorIdentity"].clone();
    snapshot[FIELD]=seal(json!({"policy":policy,"workflowId":executor.workflow_id,"nodeKey":step_key(request),"stage":request.stage,
        "domainRequestSha256":hash(&domain(&original).unwrap()),"parentSnapshotSha256":hash(&parent),"originalSettingsSha256":hash(&executor.settings),
        "originalIdentity":original["executorIdentity"],"effectiveIdentitySha256":hash(&snapshot["executorIdentity"]),"resourceHashes":template[FIELD]["resourceHashes"],
        "predecessor":prior,"modelCalls":0,"accountModelChecked":true}));
    snapshot
}
fn rejected_planner_fixture(second:bool)->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,String) {
    let (temp,mut executor,_,template)=fixture();
    let source=crate::pipeline::prepare_source("第一集\n场景：大厅\n甲：去走廊。\n场景：走廊\n甲：到了。\n").unwrap();
    let rows:Vec<_>=source.rows.iter().filter(|r|r.kind=="body").cloned().collect();
    let run=executor.store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","input":{"source":source,"settings":executor.settings}}),"partially_blocked").unwrap();
    executor.workflow_id=Some(run);
    let origin=crate::pipeline::planner_request_for_handoff_test(TextRequest{stage:"storyboardPlan".into(),skill:"episode-emotion-planner".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),
        input:json!({"storyboardCreativeProfile":"original-production-contract/v1","episode":source.episodes[0],"sourceRows":rows,"contextRows":[],"settings":{"fantasyMode":"off"}}),schema:json!({"type":"object"}),rule_snapshot:crate::pipeline::snapshot("episode-emotion-planner",&[]).unwrap()},None,&Value::Null).unwrap();
    let refs:Vec<_>=rows.iter().map(|r|r.id.clone()).collect();
    let emotion=json!({"subject":"甲","contextTone":"回应","audienceEffect":"可读","change":"依原文回应","landing":"到达","retain":["全部原话"]});
    let raw=json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"到达回应","entryFacts":[],"fantasyCandidates":[],
        "events":[{"id":"E","sceneId":rows[0].scene_id,"sourceRefs":refs,"boundaryBefore":{"kind":"episode_start","reason":"本集开始"},"emotionContract":emotion}],
        "workPackages":[{"id":"W","eventId":"E","sourceRefs":refs,"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":emotion}],
        "provisionalUnits":[{"id":"P","packageId":"W","sourceRefs":refs,"secondsRange":[3,6]}]});
    let snap=executor.request_snapshot(&origin).unwrap();
    let job=executor.store.add_job("text","rejected-origin","codex_sdk","synthetic",&snap,"received").unwrap();
    let reason="事件跨入另一个真实场景";rejected_plan_complete(&executor,&origin,&snap,&job,&raw,reason,false);
    let first=planner_repair_request(&source,&origin,&raw,&origin,&raw,reason,1);
    let first_snapshot=planner_repair_handoff(&executor,&first,&template);
    if !second {return (temp,executor,first,first_snapshot,job);}
    let first_job=add(&executor,&first,&first_snapshot).unwrap();
    let previous=json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"到达回应","entryFacts":[],"fantasyCandidates":[],
        "events":rows.iter().enumerate().map(|(i,row)|json!({"id":format!("E{i}"),"sceneId":row.scene_id,"sourceRefs":[row.id],"boundaryBefore":{"kind":if i==0{"episode_start"}else{"continuous"},"reason":"原文场景"},"emotionContract":emotion})).collect::<Vec<_>>(),
        "workPackages":rows.iter().enumerate().map(|(i,row)|json!({"id":format!("W{i}"),"eventId":format!("E{i}"),"sourceRefs":[row.id],"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":emotion})).collect::<Vec<_>>(),
        "provisionalUnits":rows.iter().enumerate().map(|(i,row)|json!({"id":format!("P{i}"),"packageId":format!("W{i}"),"sourceRefs":[row.id],"secondsRange":[3,4]})).collect::<Vec<_>>()});
    let next_reason="真实跨场景不能声明 continuous";
    rejected_plan_complete(&executor,&first,&first_snapshot,&first_job,&previous,next_reason,true);
    let second=planner_repair_request(&source,&origin,&raw,&first,&previous,next_reason,2);
    let snapshot=planner_repair_handoff(&executor,&second,&template);
    (temp,executor,second,snapshot,job)
}

#[test]
fn sdk_rejected_planner_handoff_reserves_only_exact_bounded_repair_and_keeps_original_unadopted() {
    for second in [false,true] {
        let (_temp,executor,request,snapshot,origin)=rejected_planner_fixture(second);
        let original=executor.sdk_job(&origin).unwrap();let files=["sdk-attempt.json","sdk-compiled.json","result.json"].map(|n|executor.store.root.join("text-runs").join(&origin).join(n));
        let bytes=files.iter().map(|p|std::fs::read(p).unwrap()).collect::<Vec<_>>();
        let before=executor.store.jobs().unwrap().len();let job=add(&executor,&request,&snapshot).unwrap();
        assert_eq!(executor.store.jobs().unwrap().len(),before+1);assert_eq!(executor.sdk_job(&origin).unwrap(),original);
        assert_eq!(original["status"],"rejected");assert_eq!(executor.sdk_job(&job).unwrap()["prepared"]["sdkDispatchStarted"],false);
        assert!(add(&executor,&request,&snapshot).is_err());
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.sdk_handoff_saved_snapshot(&request,&domain(&snapshot).unwrap()).unwrap(),Some(snapshot));
        for (p,original) in files.iter().zip(bytes){assert_eq!(std::fs::read(p).unwrap(),original);}
    }
}

#[test]
fn sdk_rejected_planner_handoff_source_request_rules_and_budget_drift_are_atomic_holds() {
    for mode in ["source","attempt","originHash","previousHash","raw","issue","model","rules"] {
        let (_temp,executor,mut request,_,_)=rejected_planner_fixture(false);
        match mode {
            "source"=>request.input["sourceRows"][0]["text"]=json!("改写源文"),
            "attempt"=>request.input["planRepair"]["attempt"]=json!(3),
            "originHash"=>request.input["planRepair"]["originDomainRequestSha256"]=json!("b".repeat(64)),
            "previousHash"=>request.input["planRepair"]["previousRequestSha256"]=json!("b".repeat(64)),
            "raw"=>request.input["previousPlan"]["emotionArc"]=json!("替换原回包"),
            "issue"=>request.input["validationIssues"][0]["lockedSceneGroups"]=json!([]),
            "model"=>request.model="another-model".into(),
            _=>request.rule_snapshot["runtimeContract"]["content"]=json!("changed rules"),
        }
        let before=evidence(&executor);let original=executor.request_snapshot(&request).unwrap();
        assert!(executor.sdk_handoff_predecessor(&original,false).is_err(),"{mode}");assert_eq!(evidence(&executor),before);
    }
}

#[test]
fn sdk_rejected_planner_handoff_unknown_missing_raw_event_or_error_never_authorizes_a_new_node() {
    for mode in ["unknown","raw","event","error","receipt"] {
        let (_temp,executor,request,snapshot,origin)=rejected_planner_fixture(false);
        let c=executor.store.conn().unwrap();
        match mode {
            "unknown"=>{c.execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",[&origin]).unwrap();},
            "raw"=>{std::fs::remove_file(executor.store.root.join("text-runs").join(&origin).join("result.json")).unwrap();},
            "event"=>{c.execute("DELETE FROM events WHERE job_id=?1 AND type='text_received_raw'",[&origin]).unwrap();},
            "error"=>{c.execute("UPDATE jobs SET error='unrelated rejection' WHERE id=?1",[&origin]).unwrap();},
            _=>{c.execute("UPDATE jobs SET receipt='{}' WHERE id=?1",[&origin]).unwrap();},
        }
        drop(c);let before=evidence(&executor);assert!(add(&executor,&request,&snapshot).is_err(),"{mode}");assert_eq!(evidence(&executor),before);
    }
}

#[test]
fn sdk_rejected_planner_handoff_does_not_authorize_an_ordinary_next_node_or_reset_second_attempt() {
    let (_temp,executor,mut request,_,_)=rejected_planner_fixture(true);
    let before=evidence(&executor);request.input["planRepair"]["attempt"]=json!(3);
    assert!(executor.sdk_handoff_predecessor(&executor.request_snapshot(&request).unwrap(),false).is_err());
    request.input.as_object_mut().unwrap().remove("planRepair");
    assert!(executor.sdk_handoff_predecessor(&executor.request_snapshot(&request).unwrap(),false).is_err());
    assert_eq!(evidence(&executor),before);
}

#[test]
#[ignore="explicit independent SQLite backup and copied original files; zero model calls or original DB writes"]
fn sdk_rejected_planner_handoff_readonly_firstep_copy_proves_exact_original_repair_predecessor() {
    let root=std::path::PathBuf::from(std::env::var("DIRECTOR_REJECTED_PLANNER_PROOF_COPY").expect("explicit copied fixture root"));
    assert!(root.to_string_lossy().contains("rejected-plan-proof-copy"));
    let store=Store::open(root).unwrap();let run="c72399fa-0a96-40df-a0f1-37227a621ec4";let job="321c8a83-d6eb-42f7-bbcc-a9c52a35939a";
    let parent=store.job(run).unwrap();let source:crate::pipeline::LockedSource=serde_json::from_value(parent["snapshot"]["input"]["source"].clone()).unwrap();
    let executor=SavedTextExecutor{store,settings:parent["snapshot"]["input"]["settings"].clone(),workflow_id:Some(run.into())};
    let file=executor.store.root.join("text-runs").join(job).join("sdk-attempt.json");let before_file=std::fs::read(&file).unwrap();
    let origin=FrozenAttempt::load(&file).unwrap().request;let raw:Value=serde_json::from_slice(&std::fs::read(file.parent().unwrap().join("result.json")).unwrap()).unwrap();
    let repair=planner_repair_request(&source,&origin,&raw,&origin,&raw,"事件跨入另一个真实场景",1);
    let before=evidence(&executor);let original=executor.request_snapshot(&repair).unwrap();
    no_node_or_attempt(&executor.store,&executor.store.conn().unwrap(),run,&step_key(&repair),&original).unwrap();
    let (policy,prior)=executor.sdk_handoff_predecessor(&original,false).unwrap();
    assert_eq!(policy,REJECTED_PLAN_POLICY);assert_eq!(prior["jobId"],job);assert_eq!(prior.as_object().unwrap().len(),16);
    assert_eq!(evidence(&executor),before);assert_eq!(std::fs::read(&file).unwrap(),before_file);
    assert_eq!(executor.sdk_job(job).unwrap()["status"],"rejected");
    println!("firstEP copied original proves exact rejected predecessor for legal attempt1: zero new jobs, unchanged raw/request/node, zero model calls");
}
