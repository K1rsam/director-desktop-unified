use super::*;

pub(crate) async fn series_controller_fixture()->(Value,Value,TextRequest,Value,Value,Value){
    let (input,authored)=two_episode_fixture();
    let mut old=Offline::new(authored.clone());old.review_rounds=1;old.unknown_repair=true;
    let output=run(&input,&old,"offline","medium").await.unwrap();
    assert_eq!(output["status"],"partially_blocked");
    let req=old.requests.lock().unwrap().iter().find(|r|r.stage=="seriesReview").unwrap().clone();
    let raw=old.cache.lock().unwrap()[&digest(&serde_json::to_string(&req).unwrap())].clone();
    let unit=&binding(&req,"draft")["units"][0];let source=source(&input).unwrap();
    let next=source.rows.iter().find(|r|r.kind=="body" && r.episode_id.as_deref()==Some(&source.episodes[1].id)).unwrap();
    let auth=json!({"version":controller_review::VERSION,"sourceSha256":source.sha256,
        "episodeId":req.input["episodeId"],"candidateSha256":req.input["candidateSha256"],
        "reviewJobId":"review-job","stopAfterEpisodeId":req.input["episodeId"],"nextEpisodeSourceRefs":[next.id],
        "issues":(1..=4).map(|n|json!({"id":format!("C{n}"),"severity":"must_fix","unitIds":[unit["id"]],
            "shotIds":[unit["shots"][0]["id"]],"allowInsertAfterShotIds":[unit["shots"][0]["id"]],
            "sourceRefs":if n==2 {json!([next.id])}else{unit["shots"][0]["sourceRefs"].clone()},
            "problem":"合成定向意见，保留原有内容与原话","requestedChange":"在已授权镜头内明确机位，必要时在本镜后加无对白接镜"})).collect::<Vec<_>>()});
    let context=controller_review::build_context(&auth,&req,&raw,&input,"workflow","parent-sha","snapshot-sha","review-job","node").unwrap();
    (input,authored,req,raw,auth,context)
}

struct ControllerOffline{inner:Offline,context:Value,omit:bool,continue_after:bool}
#[async_trait]
impl TextExecutor for ControllerOffline{
    async fn series_controller_review_context(&self)->Result<Option<Value>,String>{Ok(Some(self.context.clone()))}
    async fn series_controller_review_stop_required(&self)->Result<bool,String>{Ok(!self.continue_after)}
    async fn execute(&self,request:TextRequest)->Result<Value,String>{
        let mut raw=self.inner.execute(request.clone()).await?;
        if request.stage=="seriesReview" && controller_review::marker(&request).is_some(){
            raw["controllerIssueAssessments"]=json!(self.context["issues"].as_array().unwrap().iter().map(|i|
                json!({"id":i["id"],"status":"resolved","explanation":"合成独审已逐条核对本次候选；未调用真实模型"})).collect::<Vec<_>>());
            if self.omit{raw["controllerIssueAssessments"].as_array_mut().unwrap().pop();}
        }
        self.inner.cache.lock().unwrap().insert(digest(&serde_json::to_string(&request).unwrap()),raw.clone());Ok(raw)
    }
    async fn publish_episode(&self,output:&Value,input:&Value)->Result<(),String>{self.inner.publish_episode(output,input).await}
}

#[tokio::test]
async fn series_controller_review_preserves_origin_repairs_once_reviews_all_and_stops_before_next_episode(){
    let(input,authored,origin,raw,_auth,context)=series_controller_fixture().await;
    let old=input.clone();let mut inner=Offline::new(authored);inner.review_rounds=1;
    let executor=ControllerOffline{inner,context:context.clone(),omit:false,continue_after:false};
    let output=run(&input,&executor,"offline","medium").await.unwrap();
    assert_eq!(input,old);assert_eq!(output["status"],"partially_blocked","{}",output["blocked"]);
    assert_eq!(output["seriesDeliveries"].as_array().unwrap().len(),1,"{}",output["blocked"]);
    assert_eq!(output["blocked"][0]["status"],"waiting_controller");
    let delivery=&output["seriesDeliveries"][0];validate_delivery(&input,delivery).unwrap();
    let roles=delivery["adoptionReceipt"]["roleReceipts"].as_array().unwrap();
    assert!(roles.iter().any(|r|r==&role_receipt(&origin,&raw).unwrap()));
    assert_eq!(delivery["review"]["controllerIssueAssessments"].as_array().unwrap().len(),4);
    let calls=executor.inner.requests.lock().unwrap();
    assert!(calls.iter().all(|r|r.input["episodeId"]==context["episodeId"]));
    assert_eq!(calls.iter().filter(|r|r.input["role"]=="repair").count(),1);
    let repair=calls.iter().find(|r|r.input["role"]=="repair").unwrap();
    assert_eq!(binding(repair,"issues").as_array().unwrap().len(),5);
    assert_eq!(binding(repair,"source"),binding(&origin,"source"));
    assert_eq!(binding(repair,"assets"),binding(&origin,"assets"));
    drop(calls);
    let prior=executor.inner.requests.lock().unwrap().len();
    let again=run(&input,&executor,"offline","medium").await.unwrap();
    assert_eq!(again,output);assert_eq!(executor.inner.requests.lock().unwrap().len(),prior,"exact requests replay, no new budget");
    let mut tampered=delivery.clone();tampered["adoptionReceipt"][controller_review::FIELD]["issues"][0]["problem"]=json!("changed");
    assert!(validate_delivery(&input,&tampered).is_err());
}

#[tokio::test]
async fn series_controller_review_missing_assessment_or_unknown_never_adopts_or_advances(){
    for unknown in [false,true]{
        let(input,authored,_,_,_,context)=series_controller_fixture().await;
        let mut inner=Offline::new(authored);inner.review_rounds=1;inner.unknown_repair=unknown;
        let executor=ControllerOffline{inner,context,omit:!unknown,continue_after:false};
        let output=run(&input,&executor,"offline","medium").await.unwrap();
        assert!(output["seriesDeliveries"].as_array().unwrap().is_empty());
        assert_eq!(output["status"],"partially_blocked");
        assert!(executor.inner.published.lock().unwrap().is_empty());
        assert!(executor.inner.requests.lock().unwrap().iter().all(|r|r.input["episodeId"]==executor.context["episodeId"]));
    }
}

pub(crate) async fn series_controller_delivery_fixture(input:&Value,context:&Value)->Value{
    let(_,authored)=two_episode_fixture();let mut inner=Offline::new(authored);inner.review_rounds=1;
    let executor=ControllerOffline{inner,context:context.clone(),omit:false,continue_after:false};
    let output=run(input,&executor,"offline","medium").await.unwrap();
    assert_eq!(output["seriesDeliveries"].as_array().unwrap().len(),1,"{}",output["blocked"]);output
}

#[tokio::test]
async fn series_controller_review_later_explicit_continuation_reuses_target_then_advances(){
    let(input,authored,_,_,_,context)=series_controller_fixture().await;
    let mut inner=Offline::new(authored);inner.review_rounds=1;
    let mut executor=ControllerOffline{inner,context:context.clone(),omit:false,continue_after:false};
    let stopped=run(&input,&executor,"offline","medium").await.unwrap();
    assert_eq!(stopped["blocked"][0]["status"],"waiting_controller");
    let calls=executor.inner.requests.lock().unwrap().len();
    executor.continue_after=true;
    let continued=run(&input,&executor,"offline","medium").await.unwrap();
    assert_eq!(continued["status"],"completed","{}",continued["blocked"]);
    assert_eq!(continued["seriesDeliveries"].as_array().unwrap().len(),2);
    assert_eq!(continued["seriesDeliveries"][0],stopped["seriesDeliveries"][0]);
    assert!(executor.inner.requests.lock().unwrap()[calls..].iter().all(|r|r.input["episodeId"]!=context["episodeId"]));
}

#[tokio::test]
async fn series_controller_review_binds_source_candidate_next_boundary_and_budget(){
    let(input,_,req,raw,auth,context)=series_controller_fixture().await;
    controller_review::validate_context(&context,&req,&raw,&input).unwrap();
    for fault in ["candidate","source","next","shot","insert","budget"]{
        let mut changed=auth.clone();let mut request=req.clone();
        match fault{
            "candidate"=>changed["candidateSha256"]=json!("other"),
            "source"=>changed["sourceSha256"]=json!("other"),
            "next"=>changed["nextEpisodeSourceRefs"]=json!(["foreign-row"]),
            "shot"=>changed["issues"][0]["shotIds"]=json!(["foreign-shot"]),
            "insert"=>changed["issues"][0]["allowInsertAfterShotIds"]=json!(["foreign-shot"]),
            "budget"=>request.input["executionRoleId"]=json!(format!("{}/reviewer/2",s(&req.input,"episodeId"))),_=>unreachable!()}
        assert!(controller_review::build_context(&changed,&request,&raw,&input,"workflow","parent-sha","snapshot-sha","review-job","node").is_err(),"{fault}");
    }
}

#[tokio::test]
async fn series_controller_review_allows_only_explicit_insertion_anchors_and_protects_old_shots(){
    let(input,_,review,raw,_auth,context)=series_controller_fixture().await;
    let source=source(&input).unwrap();let ep=&source.episodes[0];
    let before=json!({"schemaVersion":"series-episode-candidate/v1","plan":binding(&review,"plan"),"selection":binding(&review,"selection"),"draft":binding(&review,"draft")});
    let mut values=base_values(&input,&source,ep,binding(&review,"assets").as_array().unwrap(),binding(&review,"entry"));
    values.extend([("plan",before["plan"].clone()),("selection",before["selection"].clone()),("draft",before["draft"].clone()),("issues",raw["issues"].clone())]);
    controller_review::add(&mut values,&context);
    let mut req=request("repair","seriesRepair",&ep.id,"review-1",&input,values,candidate_schema(&source,ep,binding(&review,"assets").as_array().unwrap(),"off"),"offline","medium").unwrap();
    controller_review::attach(&mut req,&context);
    let mut added=before.clone();let mut shot=added["draft"]["units"][0]["shots"][0].clone();shot["id"]=json!("new-adjacent-shot");shot["dialogueParts"]=json!([]);
    added["draft"]["units"][0]["shots"].as_array_mut().unwrap().insert(1,shot.clone());
    controller_review::assert_shots(&req,&before,&added).unwrap();
    let mut bad=before.clone();bad["draft"]["units"][0]["shots"].as_array_mut().unwrap().insert(0,shot);
    assert!(controller_review::assert_shots(&req,&before,&bad).is_err());
    let mut unbound=req.clone();unbound.input["controllerReview"]["issues"].as_array_mut().unwrap().iter_mut().for_each(|i|i["allowInsertAfterShotIds"]=json!([]));
    assert!(controller_review::assert_shots(&unbound,&before,&added).is_err());
    let mut boundary=before.clone();boundary["draft"]["units"][0]["shots"][0]["sourceRefs"]=context["nextEpisodeBoundary"]["sourceRefs"].clone();
    assert!(validate_candidate(&boundary,&source,ep,binding(&review,"assets").as_array().unwrap(),&input).is_err(),"next episode remains read-only");
}
