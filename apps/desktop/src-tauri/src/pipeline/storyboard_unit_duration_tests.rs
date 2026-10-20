// Included inside production_contract_tests to reuse its snapshot/cache executor.
fn unit_duration_payload() -> (LockedSource, Value) {
    let source=prepare_source("第一集\n场景：门口\n甲：第一句原话。\n甲：第二句原话。\n甲：第三句原话。\n").unwrap();
    let mut payload=input(&source);
    payload["capabilities"]["maxDuration"]=json!(30);
    creative::mark_input(&mut payload,true);
    (source,payload)
}

#[tokio::test]
async fn unit_duration_repair_3030_uses_original_bounded_editor_and_reuses_rejected_author() {
    let (source,payload)=unit_duration_payload();
    let executor=Recorded{author_source_error:Some("UNIT_DURATION"),..Default::default()};
    let output=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(output["status"],"completed","{}",output["blocked"]);
    validate_storyboard(&source,&output,&[],&payload["capabilities"]).unwrap();
    assert!(output["units"].as_array().unwrap().iter().all(|u|u["duration"].as_f64().unwrap()<=30.0));
    let rejected=executor.rejected_values.lock().await;
    let (origin,raw)=rejected.iter().find(|(request,_)|request.stage=="storyboardCreate").unwrap();
    assert_eq!(raw["units"][0]["shots"].as_array().unwrap().len(),10);
    assert!((raw["units"][0]["shots"].as_array().unwrap().iter().map(|s|s["duration"].as_f64().unwrap()).sum::<f64>()-30.3).abs()<1e-8);
    assert!(!executor.accepted.lock().await.contains_key(&serde_json::to_string(origin).unwrap()),"original 30.30-second author stays rejected");
    let calls=executor.calls.lock().await;
    let repairs:Vec<_>=calls.iter().filter(|r|r.input["authorRepair"].is_object()).collect();
    assert_eq!(repairs.len(),1);
    assert_eq!(repairs[0].stage,"storyboardFinish");assert_eq!(repairs[0].skill,"episode-finishing-editor");
    assert_eq!(repairs[0].input["authorRepair"]["maximumAttempts"],2);
    assert!(repairs[0].input["validationIssues"].as_array().unwrap().iter().any(|i|i["code"]=="UNIT_DURATION"));
    assert_eq!(repairs[0].input["sourceRows"],origin.input["sourceRows"]);
    assert_eq!(repairs[0].input["capabilities"]["maxDuration"],30);
    let count=calls.len();drop(calls);drop(rejected);
    let replay=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(replay["status"],"completed","{}",replay["blocked"]);
    assert_eq!(executor.calls.lock().await.len(),count,"same frozen author and repair receipts; no model replay");
}

#[tokio::test]
async fn unit_duration_repair_no_progress_stops_without_accepting_or_restarting_author() {
    let (_,payload)=unit_duration_payload();
    let executor=Recorded{author_source_error:Some("UNIT_DURATION"),source_repair_no_progress:true,..Default::default()};
    let output=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(output["status"],"partially_blocked");
    assert!(output["blocked"].to_string().contains("NO_PROGRESS"),"{}",output["blocked"]);
    assert_eq!(executor.calls.lock().await.iter().filter(|r|r.input["authorRepair"].is_object()).count(),1);
    let count=executor.calls.lock().await.len();
    let replay=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(replay["status"],"partially_blocked");
    assert_eq!(executor.calls.lock().await.len(),count,"failed bounded attempt is not reset by resume");
    assert!(executor.publications.lock().await.is_empty());
}

struct UnitDurationProgress {
    inner: Recorded,
    exhaust: bool,
}
#[async_trait]
impl TextExecutor for UnitDurationProgress {
    async fn execute(&self,request:TextRequest)->Result<Value,String> {
        let mut value=self.inner.execute(request.clone()).await?;
        if request.input["authorRepair"].is_object() {
            let attempt=request.input["authorRepair"]["attempt"].as_u64().unwrap();
            if attempt==1 || self.exhaust {
                let seconds=if self.exhaust {if attempt==1 {30.2}else{30.1}}else{30.1};
                let next=&mut value["replacements"][0]["units"][0];
                let first=next["shots"][0].clone();
                next["shots"]=json!((0..10).map(|i|{
                    let mut shot=first.clone();shot["id"]=json!(format!("progress-{i}"));shot["duration"]=json!(seconds/10.0);
                    if i>0 {shot["dialogueParts"]=json!([]);}shot
                }).collect::<Vec<_>>());
                next["duration"]=json!(seconds);
            }
        }
        Ok(value)
    }
    async fn accept(&self,r:&TextRequest,v:&Value)->Result<(),String>{self.inner.accept(r,v).await}
    async fn reject(&self,r:&TextRequest,v:&Value,e:&str)->Result<(),String>{self.inner.reject(r,v,e).await}
    async fn publish_episode(&self,o:&Value,i:&Value)->Result<(),String>{self.inner.publish_episode(o,i).await}
}

#[tokio::test]
async fn unit_duration_repair_progress_keeps_two_attempt_budget_and_resume_never_resets_it() {
    for exhaust in [false,true] {
        let (source,payload)=unit_duration_payload();
        let executor=UnitDurationProgress{inner:Recorded{author_source_error:Some("UNIT_DURATION"),..Default::default()},exhaust};
        let output=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
        assert_eq!(output["status"],if exhaust{"partially_blocked"}else{"completed"},"{}",output["blocked"]);
        if exhaust {
            assert!(output["blocked"].to_string().contains("LIMIT"));
            assert!(executor.inner.publications.lock().await.is_empty());
        } else {validate_storyboard(&source,&output,&[],&payload["capabilities"]).unwrap();}
        let calls=executor.inner.calls.lock().await;
        let attempts:Vec<_>=calls.iter().filter(|r|r.input["authorRepair"].is_object()).map(|r|r.input["authorRepair"]["attempt"].clone()).collect();
        assert_eq!(attempts,json!([1,2]).as_array().unwrap().clone());
        let count=calls.len();drop(calls);
        let replay=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
        assert_eq!(replay["status"],output["status"]);
        assert_eq!(executor.inner.calls.lock().await.len(),count,"no third attempt/new Author on resume");
    }
}

#[test]
fn unit_duration_repair_candidate_is_unadopted_and_all_source_identity_voice_gates_stay_hard() {
    let (source,payload)=unit_duration_payload();
    let rows:Vec<_>=source.rows.iter().filter(|r|r.kind=="body").cloned().collect();
    let refs:Vec<_>=rows.iter().map(|r|r.id.clone()).collect();
    let request=TextRequest{stage:"storyboardCreate".into(),skill:"unit-performance-director".into(),model:"offline".into(),reasoning:"medium".into(),
        input:json!({"sourceRows":rows,"workPackage":{"eventId":"E1"}}),schema:json!({}),rule_snapshot:json!({})};
    let plan=mock_plan(&request,false);let mut raw=mock_draft(&request,false);
    let first=raw["units"][0]["shots"][0].clone();
    raw["units"][0]["shots"]=json!((0..10).map(|i|{
        let mut shot=first.clone();shot["id"]=json!(format!("unchanged-{i}"));shot["duration"]=json!(3.03);
        if i>0{shot["dialogueParts"]=json!([]);}shot
    }).collect::<Vec<_>>());
    let original=raw.clone();
    let reason=validate_authored(&source,&mut raw.clone(),&[],&payload["capabilities"],&refs,"原画风","P:").unwrap_err();
    assert!(crate::text_executor::author_unit_duration_rejection(&reason));
    let check=|value:&Value|author_validation_candidate(&source,value,&[],&payload["capabilities"],&refs,"原画风","P:",&plan,&[],&reason);
    let (candidate,issues)=check(&raw).unwrap();
    assert_eq!(raw,original,"no shot shortening, rounding or raw replacement by Rust");
    assert_eq!(issues.len(),1);assert_eq!(issues[0]["code"],"UNIT_DURATION");
    assert!((issues[0]["originalDuration"].as_f64().unwrap()-30.3).abs()<1e-8);
    assert_eq!(issues[0]["maximumDuration"].as_f64(),Some(30.0));
    assert_eq!(author_scope_repair_units(&candidate,&issues).unwrap(),HashSet::from(["P:U0".into()]));
    assert!(validate_authored(&source,&mut candidate.clone(),&[],&payload["capabilities"],&refs,"原画风","P:").is_err(),"candidate cannot pass unchanged 30-second acceptance");
    for mode in ["VOICE","WORDS","MISSING","UNKNOWN_SOURCE","UNKNOWN_ASSET","ORDER"] {
        let mut bad=raw.clone();
        match mode {
            "VOICE"=>bad["units"][0]["shots"][0]["dialogueParts"][0]["voiceEnd"]=json!(9),
            "WORDS"=>bad["units"][0]["shots"][0]["dialogueParts"][0]["text"]=json!("原文没有的话"),
            "MISSING"=>bad["units"][0]["shots"][0]["dialogueParts"]=json!([]),
            "UNKNOWN_SOURCE"=>bad["units"][0]["shots"][0]["sourceRefs"]=json!(["unknown-row"]),
            "UNKNOWN_ASSET"=>bad["units"][0]["assetRefs"]=json!([{"assetId":"unknown","variantId":"base","order":1,"use":"character","sourceRefs":[refs[0]]}]),
            "ORDER"=>bad["units"].as_array_mut().unwrap().swap(1,2),
            _=>unreachable!(),
        }
        assert!(check(&bad).is_err(),"{mode} is not excused by a carrier overrun");
    }
}

#[test]
fn unit_duration_repair_classifies_only_original_finite_strict_gate_messages() {
    let known="单元 episode:test:unit:02 时长 30.30 秒超过渠道 30.00 秒；须修订而非截断";
    assert!(crate::text_executor::author_repairable_rejection(known));
    for reason in ["单元 U 时长 NaN 秒超过渠道 30.00 秒；须修订而非截断",
        "单元 U 时长 3.00 秒超过渠道 30.00 秒；须修订而非截断",
        "单元 U 时长 30.30 秒超过渠道 0.00 秒；须修订而非截断",
        "单元 U 时长 30.30 秒超过渠道 30.00 秒；请放宽上限",
        "单元 U 时长 30.3 秒超过渠道 30.00 秒；须修订而非截断",
        "单元 U injected 时长 30.30 秒超过渠道 30.00 秒；须修订而非截断"] {
        assert!(!crate::text_executor::author_unit_duration_rejection(reason),"{reason}");
    }
}

#[test]
#[ignore="explicit original private file fixture; no DB/model/browser or output mutation"]
fn unit_duration_repair_readonly_original_private_candidate_preserves_rejected_3030_raw() {
    let root=std::path::PathBuf::from(std::env::var("DIRECTOR_STAGE3_UNIT_DURATION_PRIVATE_ROOT").expect("explicit private fixture root"));
    let workspace_path=root.join("storyboard-workspace.private.json");
    let attempt_path=root.join("home/projects/dd4a7016-1864-4cc0-9e74-a19fdd8ca6a5/text-runs/6bd98ec7-b775-4eea-828e-def71019aa8c/sdk-attempt.json");
    let raw_path=attempt_path.parent().unwrap().join("result.json");
    let before=[crate::store::hash_file(&workspace_path).unwrap().0,crate::store::hash_file(&attempt_path).unwrap().0,crate::store::hash_file(&raw_path).unwrap().0];
    let workspace:Value=serde_json::from_slice(&std::fs::read(&workspace_path).unwrap()).unwrap();
    let source:LockedSource=serde_json::from_value(workspace["source"].clone()).unwrap();
    let attempt=crate::text_transport::FrozenAttempt::load(&attempt_path).unwrap();
    let request=&attempt.request;
    let raw_bytes=std::fs::read(&raw_path).unwrap();
    assert_eq!(attempt.raw_result.as_ref().unwrap()["rawText"].as_str().unwrap().as_bytes(),raw_bytes);
    let raw:Value=serde_json::from_slice(&raw_bytes).unwrap();
    let projected=materialize_role_result(&request.input,&raw).unwrap();
    let assets=workspace["assets"].as_array().unwrap();
    let refs=strings(&request.input["executionBatch"],"sourceRefs").unwrap();
    let prefix=format!("{}:{}:",text(&request.input,"episodeId"),text(&request.input["executionBatch"],"id"));
    let reason=validate_authored(&source,&mut projected.clone(),assets,&request.input["capabilities"],&refs,"3D国漫",&prefix).unwrap_err();
    assert!(reason.contains("30.30"));assert!(crate::text_executor::author_unit_duration_rejection(&reason));
    let (candidate,issues)=author_validation_candidate(&source,&projected,assets,&request.input["capabilities"],&refs,"3D国漫",&prefix,&request.input["plan"],request.input["selectedFantasy"].as_array().unwrap(),&reason).unwrap();
    assert!(issues.iter().any(|issue|issue["code"]=="UNIT_DURATION"&&issue["maximumDuration"].as_f64()==Some(30.0)));
    assert_eq!(candidate["units"].as_array().unwrap().len(),raw["units"].as_array().unwrap().len());
    assert!(validate_authored(&source,&mut candidate.clone(),assets,&request.input["capabilities"],&refs,"3D国漫",&prefix).is_err());
    let after=[crate::store::hash_file(&workspace_path).unwrap().0,crate::store::hash_file(&attempt_path).unwrap().0,crate::store::hash_file(&raw_path).unwrap().0];
    assert_eq!(before,after);
    println!("private original candidate verified: units={}, issues={}, original bytes unchanged",candidate["units"].as_array().unwrap().len(),issues.len());
}
