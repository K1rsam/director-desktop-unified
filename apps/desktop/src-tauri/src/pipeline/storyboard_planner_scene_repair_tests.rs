// Included in production_contract_tests; all fixtures are synthetic unless
// explicitly selected by the private, read-only ignored test below.
struct ScenePlannerRepair {
    inner: Recorded,
    repaired: Value,
    no_progress: bool,
}

#[async_trait]
impl TextExecutor for ScenePlannerRepair {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let value = self.inner.execute(request.clone()).await?;
        if request.stage == "storyboardPlan" && request.input["planRepair"].is_object() && !self.no_progress {
            return Ok(self.repaired.clone());
        }
        Ok(value)
    }
    async fn accept(&self, r: &TextRequest, v: &Value) -> Result<(), String> { self.inner.accept(r,v).await }
    async fn reject(&self, r: &TextRequest, v: &Value, e: &str) -> Result<(), String> {
        if r.stage=="storyboardPlan" {
            self.inner.rejected_raw.lock().await.insert(serde_json::to_string(r).unwrap(),v.clone());
        }
        self.inner.reject(r,v,e).await
    }
    async fn publish_episode(&self, o: &Value, i: &Value) -> Result<(), String> { self.inner.publish_episode(o,i).await }
}

fn planner_scene_pair() -> (LockedSource, Value, Value, Value) {
    let source=prepare_source("第一集\n场景：大厅\n甲：我要去走廊。\n场景：走廊\n甲：我到了。\n").unwrap();
    let rows:Vec<_>=source.rows.iter().filter(|r|r.kind=="body").cloned().collect();
    let request=TextRequest{stage:"storyboardPlan".into(),skill:"episode-emotion-planner".into(),model:"offline".into(),reasoning:"medium".into(),input:json!({"sourceRows":rows}),schema:json!({}),rule_snapshot:json!({})};
    let raw=mock_plan(&request,false);
    let mut repaired=raw.clone();
    for (field,prototype) in [("events",raw["events"][0].clone()),("workPackages",raw["workPackages"][0].clone()),("provisionalUnits",raw["provisionalUnits"][0].clone())] {
        repaired[field]=json!(rows.iter().enumerate().map(|(i,row)|{
            let mut item=prototype.clone();item["sourceRefs"]=json!([row.id]);
            match field {
                "events"=>{item["id"]=json!(format!("E{i}"));item["sceneId"]=json!(row.scene_id);item["boundaryBefore"]=json!({"kind":if i==0{"episode_start"}else{"time_space_jump"},"reason":"原文实际场所变化"});},
                "workPackages"=>{item["id"]=json!(format!("W{i}"));item["eventId"]=json!(format!("E{i}"));},
                _=>{item["id"]=json!(format!("P{i}"));item["packageId"]=json!(format!("W{i}"));item["secondsRange"]=json!([3,4]);}
            }
            item
        }).collect::<Vec<_>>());
    }
    let mut payload=input(&source);creative::mark_input(&mut payload,true);
    (source,payload,raw,repaired)
}

#[tokio::test]
async fn planner_scene_early_crossing_uses_original_planner_without_changing_source_or_adopting_raw() {
    let (source,payload,raw,repaired)=planner_scene_pair();
    let executor=ScenePlannerRepair{inner:Recorded{plan_override:Some(raw.clone()),..Default::default()},repaired,no_progress:false};
    let output=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(output["status"],"completed","{}",output["blocked"]);
    validate_storyboard(&source,&output,&[],&payload["capabilities"]).unwrap();
    let calls=executor.inner.calls.lock().await;
    let plans:Vec<_>=calls.iter().filter(|r|r.stage=="storyboardPlan").collect();
    assert_eq!(plans.len(),2);
    assert_eq!(plans[0].skill,plans[1].skill);
    assert_eq!(plans[0].input["sourceRows"],plans[1].input["sourceRows"]);
    assert_eq!(plans[0].input["episode"],plans[1].input["episode"]);
    assert_eq!(plans[0].schema,plans[1].schema);
    assert_eq!(plans[1].input["previousPlan"],raw);
    assert_eq!(plans[1].input["planRepair"]["attempt"],1);
    assert_eq!(plans[1].input["planRepair"]["maximumAttempts"],2);
    assert_eq!(plans[1].input["validationIssues"][0]["problem"],"事件跨入另一个真实场景");
    let groups=plans[1].input["validationIssues"][0]["lockedSceneGroups"].as_array().unwrap();
    assert_eq!(groups.len(),2);
    assert_eq!(groups.iter().flat_map(|g|g["sourceRefs"].as_array().unwrap().iter().cloned()).collect::<Vec<_>>(),plans[0].input["sourceRows"].as_array().unwrap().iter().map(|r|r["id"].clone()).collect::<Vec<_>>());
    assert!(!executor.inner.accepted.lock().await.contains_key(&serde_json::to_string(plans[0]).unwrap()),"original wrong-scene plan remains unadopted");
    let count=calls.len();drop(calls);
    let replay=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(replay["status"],"completed");
    assert_eq!(executor.inner.calls.lock().await.len(),count,"resume reuses the original rejected plan and adopted repair");
}

#[tokio::test]
async fn planner_scene_early_crossing_identical_repair_stops_and_resume_never_resets_budget() {
    let (_,payload,raw,repaired)=planner_scene_pair();
    let executor=ScenePlannerRepair{inner:Recorded{plan_override:Some(raw.clone()),..Default::default()},repaired,no_progress:true};
    let output=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(output["status"],"partially_blocked");
    assert!(output["blocked"].to_string().contains("PLAN_REPAIR_NO_PROGRESS"),"{}",output["blocked"]);
    assert_eq!(executor.inner.calls.lock().await.len(),2,"original planner plus one unchanged repair, no author");
    let replay=run_storyboard(&payload,&executor,"offline","medium").await.unwrap();
    assert_eq!(replay["status"],"partially_blocked");
    assert_eq!(executor.inner.calls.lock().await.len(),2);
    assert!(executor.inner.publications.lock().await.is_empty());
}

#[test]
fn planner_scene_feedback_keeps_locked_groups_separate_from_raw_and_old_repairs_exact() {
    let (source,_,raw,_)=planner_scene_pair();
    let rows:Vec<_>=source.rows.iter().filter(|r|r.kind=="body").cloned().collect();
    let before=raw.clone();
    let issues=planner_validation_issues("事件跨入另一个真实场景",&raw,&rows);
    assert_eq!(raw,before);
    assert_eq!(issues[0]["evidenceRole"],"LOCKED_SOURCE_COORDINATES_ONLY");
    assert_eq!(issues[0]["mismatchedEvents"][0]["mismatchedSourceRefs"],json!([rows[1].id]));
    assert_eq!(validate_plan(&raw,&rows,&[],&source.scenes).unwrap_err(),"事件跨入另一个真实场景","feedback cannot adopt or program-split the raw plan");
    let reason="真实跨场景不能声明 continuous";
    assert_eq!(planner_validation_issues(reason,&raw,&rows),json!([{"code":"PLAN_EVENT_BOUNDARY","problem":reason}]),"old frozen repair inputs do not change");
    for hard in ["事件身份或场景无效","引用未知、越界或重复","事件必须按源顺序覆盖当前正文恰好一次","工作包越出所属事件","工作包必须依源顺序覆盖正文恰好一次"] {
        assert!(!crate::text_executor::planner_repairable_rejection(hard),"{hard} is not implicitly authorized");
    }
}

#[test]
#[ignore="explicit original firstEP private files; read-only, no DB/model/browser"]
fn planner_scene_readonly_firstep_original_reports_locked_groups_without_rewriting_plan_or_rules() {
    let root=std::path::PathBuf::from(std::env::var("DIRECTOR_PLANNER_SCENE_PRIVATE_ROOT").expect("explicit private root"));
    let workspace_path=root.join("storyboard-workspace.private.json");
    let attempt_path=root.join("home/projects/e0ead03c-90de-4e54-b238-4d3d753324cc/text-runs/321c8a83-d6eb-42f7-bbcc-a9c52a35939a/sdk-attempt.json");
    let raw_path=attempt_path.parent().unwrap().join("result.json");
    let before=[crate::store::hash_file(&workspace_path).unwrap().0,crate::store::hash_file(&attempt_path).unwrap().0,crate::store::hash_file(&raw_path).unwrap().0];
    let workspace:Value=serde_json::from_slice(&std::fs::read(&workspace_path).unwrap()).unwrap();
    let source:LockedSource=serde_json::from_value(workspace["source"].clone()).unwrap();
    let attempt=crate::text_transport::FrozenAttempt::load(&attempt_path).unwrap();
    let request=&attempt.request;let raw_bytes=std::fs::read(&raw_path).unwrap();
    assert_eq!(attempt.raw_result.as_ref().unwrap()["rawText"].as_str().unwrap().as_bytes(),raw_bytes);
    let raw:Value=serde_json::from_slice(&raw_bytes).unwrap();
    let rows:Vec<SourceRow>=serde_json::from_value(request.input["sourceRows"].clone()).unwrap();
    let context:Vec<SourceRow>=serde_json::from_value(request.input["contextRows"].clone()).unwrap();
    let scenes:Vec<_>=source.scenes.iter().filter(|s|request.input["episode"]["id"]==s.episode_id).cloned().collect();
    let reason=validate_plan(&raw,&rows,&context,&scenes).unwrap_err();
    assert_eq!(reason,"事件跨入另一个真实场景");
    assert!(crate::text_executor::planner_repairable_rejection(&reason));
    let issues=planner_validation_issues(&reason,&raw,&rows);
    let groups=issues[0]["lockedSceneGroups"].as_array().unwrap();
    assert_eq!(rows.len(),29);assert_eq!(groups.len(),2);
    assert_eq!(groups[0]["sourceRefs"].as_array().unwrap().len(),16);
    assert_eq!(groups[1]["sourceRefs"].as_array().unwrap().len(),13);
    assert_eq!(issues[0]["mismatchedEvents"][0]["mismatchedSourceRefs"].as_array().unwrap().len(),13);
    let mut repair=request.clone();
    let request_hash=digest(&serde_json::to_string(request).unwrap());let raw_hash=digest(&raw.to_string());
    repair.input["previousPlan"]=raw.clone();repair.input["validationIssues"]=issues;
    repair.input["planRepair"]=json!({"version":"rejected-plan-repair/v1","attempt":1,"maximumAttempts":2,"sourceSha256":source.sha256,"originDomainRequestSha256":request_hash,"originalPlanSha256":raw_hash,"previousRequestSha256":request_hash,"previousPlanSha256":raw_hash});
    let prepared=creative::prepare_request(repair).unwrap();
    for field in ["sourceRows","episode","storyboardProductionContext","storyboardRulePackProfile","storyboardRulePackSha256"] {assert_eq!(prepared.input[field],request.input[field],"{field} remains frozen");}
    assert_eq!(prepared.rule_snapshot["primary"],request.rule_snapshot["primary"]);
    for (field,value) in request.rule_snapshot.as_object().unwrap() {
        if field!="runtimeContract" {assert_eq!(&prepared.rule_snapshot[field],value,"original frozen rule field {field}");}
    }
    assert_eq!(prepared.input["previousPlan"],raw);
    assert_eq!(before,[crate::store::hash_file(&workspace_path).unwrap().0,crate::store::hash_file(&attempt_path).unwrap().0,crate::store::hash_file(&raw_path).unwrap().0]);
    println!("firstEP original verified: 29 locked rows, 16/13 scene groups, 13 crossings; original plan/rules/source unchanged; no model calls");
}
