use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;

struct Fixture {
    source:LockedSource,batch:Batch,basis:FoundationProposal,current:FoundationProposal,candidate:FoundationProposal,
    context:Value,manifest:conflict_resolution::Manifest,legacy:TextRequest,
}
fn fixture()->Fixture {
    let source=prepare_source("第1集\n场景：竞技场\n△小王举起重盾。\n").unwrap();
    let scope:Vec<_>=source.episodes.iter().map(|e|e.id.clone()).collect();
    let context=json!({"selection":{"episodeIds":scope}});
    let (_,mut basis,_)=delivery_basis(&source,&context).unwrap();
    let batch=Batch{pieces:whole_rows(&source,&scope),scope,fragment:None};
    let scene=&source.scenes[0];let row=source.rows.iter().find(|r|r.scene_id.as_deref()==Some(scene.id.as_str())).unwrap();
    let place=Applicability{episode_id:scene.episode_id.clone(),scene_id:scene.id.clone()};
    basis.assets.push(FoundationAsset{id:"shield".into(),name:"重盾".into(),kind:"PROP".into(),aliases:vec![],description:"原说明".into(),source_refs:vec![row.id.clone()],
        variants:vec![FoundationVariant{id:"shield:base".into(),name:"重盾".into(),description:"母版原说明".into(),source_variant_id:None,source_refs:vec![row.id.clone()],applicability:vec![place.clone()],
            scope:FoundationScope{kind:"SCENES".into(),scenes:vec![place],reason:None},asset_row_id:None,metadata:Map::from_iter([("isBase".into(),json!(true))])}],metadata:Map::new()});
    let mut current=basis.clone();current.assets[0].description="当前说明".into();
    let mut candidate=basis.clone();candidate.assets[0].description="候选说明".into();
    let input=input_for("F2",&source,&batch,&current,&context,0,1,false,&[]);
    let raw=json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":source.sha256,"coverage":batch.scope,"assets":[{"key":"shield","description":"候选说明"}]});
    let manifest=conflict_resolution::build_manifest(&source,&batch,&basis,&current,&candidate,&input,&raw,&["/assets/shield/description".into()]).unwrap();
    let legacy=TextRequest{stage:"foundation:F2".into(),skill:F2_RULE.into(),model:"synthetic".into(),reasoning:"medium".into(),input,schema:f2_schema(),rule_snapshot:json!({})};
    Fixture{source,batch,basis,current,candidate,context,manifest,legacy}
}
fn marker(m:&Value,attempt:u64)->Value {
    let mut marker=json!({"version":"foundation-conflict-resolution/v1","attempt":attempt,"maximumAttempts":2,
        "legacyJobId":null,"previousAttemptSha256":if attempt==1 {Value::Null}else{json!("f".repeat(64))},"legacyConsumedAttempts":0});
    for k in ["sourceSha256","originalInputSha256","originalResultSha256","currentBasisSha256","issueSetSha256","causeSha256","manifestSha256"]{marker[k]=m[k].clone();}marker
}
fn recovery(f:&Fixture,spent:u64,error:Option<&str>)->Value {
    let history:Vec<_>=(0..spent).map(|n|json!({"jobId":format!("saved-{n}"),"status":"rejected","legacy":false,"error":error.unwrap_or("先前字段错误"),"rawSha256":"e".repeat(64),"causeSha256":f.manifest.input["causeSha256"]})).collect();
    json!({"legacyAccepted":null,"existingResolution":null,"consumedAttempts":spent,"nextAttempt":if spent<2{json!(spent+1)}else{Value::Null},"previousAttempts":history,"marker":marker(&f.manifest.input,spent+1)})
}
fn decision(f:&Fixture)->Value {
    let item=&f.manifest.input["items"][0];
    json!({"version":"foundation-conflict-resolution/v1","manifestSha256":f.manifest.digest,"decisions":[{"conflictId":item["conflictId"],"expectedOldHash":item["expectedOldHash"],"action":"adopt_current","targetAssetId":null,"targetVariantId":null,"take":"current","replacementText":null,"reason":"保持原已采用说明","evidenceRows":[]}]})
}
struct Fake {
    responses:Mutex<VecDeque<Result<Value,String>>>,requests:Mutex<Vec<TextRequest>>,recoveries:Mutex<VecDeque<Value>>,
    rejects:Mutex<Vec<String>>,accepts:Mutex<usize>,context_calls:Mutex<usize>,too_large:bool,mutate_prepared:bool,
}
impl Fake {
    fn new(responses:Vec<Result<Value,String>>)->Self {Self{responses:Mutex::new(responses.into()),requests:Mutex::new(vec![]),recoveries:Mutex::new(VecDeque::new()),rejects:Mutex::new(vec![]),accepts:Mutex::new(0),context_calls:Mutex::new(0),too_large:false,mutate_prepared:false}}
}
#[async_trait::async_trait]
impl TextExecutor for Fake {
    async fn execute(&self,request:TextRequest)->Result<Value,String>{self.requests.lock().unwrap().push(request);self.responses.lock().unwrap().pop_front().expect("unexpected extra execute")}
    async fn accept(&self,_:&TextRequest,_:&Value)->Result<(),String>{*self.accepts.lock().unwrap()+=1;Ok(())}
    async fn reject(&self,_:&TextRequest,_:&Value,reason:&str)->Result<(),String>{self.rejects.lock().unwrap().push(reason.into());Ok(())}
    async fn foundation_conflict_context(&self,_:&TextRequest,_:&Value)->Result<Value,String>{
        *self.context_calls.lock().unwrap()+=1;
        let mut value=self.recoveries.lock().unwrap().pop_front().expect("unexpected context retry");
        if value["syntheticInvalidJsonProof"]==true {
            value.as_object_mut().unwrap().remove("syntheticInvalidJsonProof");
            let requests=self.requests.lock().unwrap();let request=requests.last().unwrap();
            let marker=&request.input["foundationConflictResolution"];
            let row=value["previousAttempts"].as_array_mut().unwrap().last_mut().unwrap();
            let text="{\"decisions\":[ 原始坏文本";
            row["workflowId"]=json!("synthetic-run");row["requestSha256"]=json!("f".repeat(64));row["resultSha256"]=Value::Null;row["rawSha256"]=json!(digest(text));
            row["terminalRejection"]=json!({"version":"foundation-conflict-invalid-json/v1","code":"INVALID_JSON","attemptState":"completed",
                "jobId":row["jobId"],"workflowId":row["workflowId"],"attempt":marker["attempt"],"causeSha256":marker["causeSha256"],"manifestSha256":marker["manifestSha256"],
                "requestSha256":row["requestSha256"],"logicalRequestSha256":digest(&serde_json::to_value(request).unwrap().to_string()),
                "rawText":text,"rawTextSha256":digest(text),"rawEventId":1,"rawEventSha256":"e".repeat(64),"checkpointState":"rejected","parsedResult":false});
            if value["breakInvalidJsonProof"]==true {value["previousAttempts"].as_array_mut().unwrap().last_mut().unwrap()["terminalRejection"]["logicalRequestSha256"]=json!("changed");}
        }
        Ok(value)
    }
    fn input_transport_characters(&self,_:&TextRequest)->Result<usize,String>{Ok(if self.too_large{MODEL_TRANSPORT_LIMIT+1}else{1})}
    async fn prepare_transport_request(&self,mut request:TextRequest)->Result<TextRequest,String>{if self.mutate_prepared {request.input["unexpectedTransportMutation"]=json!(true);}Ok(request)}
}
async fn invoke(f:&Fixture,state:Value,executor:&Fake)->Result<conflict_resolution::ResolutionOutcome,String> {
    run(&f.manifest,&f.legacy,state,&f.source,&f.source,&f.batch,&f.basis,&f.current,&f.candidate,&f.context,executor).await
}
fn existing(f:&Fixture,status:&str)->Value {
    let mut state=recovery(f,1,None);state["nextAttempt"]=Value::Null;
    let mut request=f.legacy.clone();request.schema=f.manifest.schema.clone();request.input["foundationConflictManifest"]=f.manifest.input.clone();request.input["foundationConflictResolution"]=marker(&f.manifest.input,1);
    state["previousAttempts"][0]["status"]=json!(status);
    state["existingResolution"]=json!({"jobId":"saved-0","status":status,"request":request});state
}

#[tokio::test]
async fn conflict_runner_missing_production_marker_never_uses_synthetic_authorization() {
    let f=fixture();let fake=Fake::new(vec![]);let mut state=recovery(&f,0,None);state.as_object_mut().unwrap().remove("marker");
    assert!(invoke(&f,state,&fake).await.err().unwrap().contains("MARKER_MISSING"));assert!(fake.requests.lock().unwrap().is_empty());
    let mut state=recovery(&f,0,None);state["marker"]["manifestSha256"]=json!("forged");
    assert!(invoke(&f,state,&fake).await.err().unwrap().contains("MARKER_MISMATCH"));assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn conflict_runner_restart_remembers_same_cause_and_stops_without_third_dispatch() {
    let f=fixture();let bad=json!({});
    let failure=conflict_resolution::apply(&f.manifest,&bad,&f.source,&f.basis,&f.current,&f.candidate).err().unwrap();
    let fake=Fake::new(vec![Ok(bad)]);
    let result=invoke(&f,recovery(&f,1,Some(&failure)),&fake).await.unwrap();
    assert_eq!(fake.requests.lock().unwrap().len(),1);assert_eq!(fake.rejects.lock().unwrap().len(),1);assert_eq!(*fake.context_calls.lock().unwrap(),0);
    assert_eq!(result.review_items.len(),1);assert!(result.review_items[0].details.as_ref().unwrap()["reason"].as_str().unwrap().contains("重复同因"));
    assert_eq!(result.proposal.assets[0].description,f.current.assets[0].description);
}

#[tokio::test]
async fn conflict_runner_two_round_budget_is_durable_and_no_implicit_third_round_exists() {
    let f=fixture();let bad=json!({});let fake=Fake::new(vec![Ok(bad.clone()),Ok(bad)]);
    let failure=conflict_resolution::apply(&f.manifest,&json!({}),&f.source,&f.basis,&f.current,&f.candidate).err().unwrap();
    fake.recoveries.lock().unwrap().push_back(recovery(&f,1,Some(&failure)));
    let out=invoke(&f,recovery(&f,0,None),&fake).await.unwrap();assert_eq!(out.review_items.len(),1);
    assert_eq!(fake.requests.lock().unwrap().len(),2);assert_eq!(fake.rejects.lock().unwrap().len(),2);
    let resumed=Fake::new(vec![]);let out=invoke(&f,recovery(&f,2,Some(&failure)),&resumed).await.unwrap();assert_eq!(out.review_items.len(),1);assert!(resumed.requests.lock().unwrap().is_empty());
    let mut invalid=recovery(&f,1,None);invalid["nextAttempt"]=json!(3);assert!(invoke(&f,invalid,&resumed).await.err().unwrap().contains("ATTEMPT_INVALID"));
}

#[tokio::test]
async fn conflict_runner_transport_unknown_is_never_a_domain_rejection_or_retry() {
    let f=fixture();let fake=Fake::new(vec![Err("SDK_EXISTING_ATTEMPT_UNRESOLVED".into())]);
    assert!(invoke(&f,recovery(&f,0,None),&fake).await.err().unwrap().contains("SDK_EXISTING_ATTEMPT_UNRESOLVED"));
    assert_eq!(fake.requests.lock().unwrap().len(),1);assert!(fake.rejects.lock().unwrap().is_empty());assert_eq!(*fake.context_calls.lock().unwrap(),0);
    let fake=Fake::new(vec![]);let mut state=recovery(&f,1,None);state["previousAttempts"][0]["status"]=json!("submission_unknown");state["nextAttempt"]=Value::Null;
    assert!(invoke(&f,state,&fake).await.err().unwrap().contains("UNRESOLVED"));assert!(fake.requests.lock().unwrap().is_empty());
    assert!(invoke(&f,existing(&f,"submission_unknown"),&fake).await.err().unwrap().contains("UNRESOLVED"));
}

#[tokio::test]
async fn conflict_runner_saved_decisions_reuse_exact_request_despite_new_capacity_limit() {
    let f=fixture();
    for status in ["completed","received"] {
        let mut fake=Fake::new(vec![Ok(decision(&f))]);fake.too_large=true;
        let state=existing(&f,status);let saved=state["existingResolution"]["request"].clone();
        let out=invoke(&f,state,&fake).await.unwrap();assert!(out.review_items.is_empty());
        assert_eq!(serde_json::to_value(&fake.requests.lock().unwrap()[0]).unwrap(),saved);assert_eq!(*fake.accepts.lock().unwrap(),1);assert!(fake.rejects.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn conflict_runner_accepted_result_cannot_be_rejected_or_silently_repaired() {
    let f=fixture();let fake=Fake::new(vec![Ok(json!({}))]);
    assert!(invoke(&f,existing(&f,"completed"),&fake).await.err().unwrap().contains("ADOPTED_REVALIDATION_HELD"));
    assert!(fake.rejects.lock().unwrap().is_empty());assert_eq!(*fake.context_calls.lock().unwrap(),0);
    let mut fake=Fake::new(vec![]);fake.mutate_prepared=true;
    assert!(invoke(&f,existing(&f,"received"),&fake).await.err().unwrap().contains("FROZEN_REQUEST_CHANGED"));assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn conflict_runner_capacity_and_unpersisted_rejection_do_not_create_extra_attempts() {
    let f=fixture();let mut fake=Fake::new(vec![]);fake.too_large=true;
    let out=invoke(&f,recovery(&f,0,None),&fake).await.unwrap();assert_eq!(out.review_items.len(),1);assert!(fake.requests.lock().unwrap().is_empty());
    assert!(invoke(&f,existing(&f,"queued"),&fake).await.err().unwrap().contains("QUEUED_CAPACITY_HELD"));assert!(fake.requests.lock().unwrap().is_empty());
    let fake=Fake::new(vec![Ok(json!({}))]);fake.recoveries.lock().unwrap().push_back(recovery(&f,0,None));
    assert!(invoke(&f,recovery(&f,0,None),&fake).await.err().unwrap().contains("REJECTION_NOT_PERSISTED"));assert_eq!(fake.requests.lock().unwrap().len(),1);
}

#[tokio::test]
async fn conflict_runner_verified_invalid_json_counts_budget_and_supplies_original_bad_text() {
    let f=fixture();let fake=Fake::new(vec![Err("SDK_RAW_JSON_INVALID: synthetic".into()),Ok(decision(&f))]);
    let mut after=recovery(&f,1,None);after["syntheticInvalidJsonProof"]=json!(true);fake.recoveries.lock().unwrap().push_back(after);
    let result=invoke(&f,recovery(&f,0,None),&fake).await.unwrap();assert!(result.review_items.is_empty());
    let requests=fake.requests.lock().unwrap();assert_eq!(requests.len(),2);
    assert_eq!(requests[1].input["foundationConflictResolution"]["attempt"],2);
    assert_eq!(requests[1].input["foundationConflictRepairHistory"][0]["terminalRejection"]["rawText"],"{\"decisions\":[ 原始坏文本");
    assert!(fake.rejects.lock().unwrap().is_empty());assert_eq!(*fake.accepts.lock().unwrap(),1);
}

#[tokio::test]
async fn conflict_runner_verified_invalid_json_exhaustion_is_local_without_reject_or_third_attempt() {
    let f=fixture();let fake=Fake::new(vec![Err("SDK_RAW_JSON_INVALID: arbitrary wording".into())]);
    let mut after=recovery(&f,2,None);after["syntheticInvalidJsonProof"]=json!(true);fake.recoveries.lock().unwrap().push_back(after);
    let result=invoke(&f,recovery(&f,1,None),&fake).await.unwrap();assert_eq!(result.review_items.len(),1);
    assert_eq!(result.proposal.assets[0].description,f.current.assets[0].description);
    assert_eq!(fake.requests.lock().unwrap().len(),1);assert!(fake.rejects.lock().unwrap().is_empty());assert_eq!(*fake.accepts.lock().unwrap(),0);
}

#[tokio::test]
async fn conflict_runner_invalid_json_error_string_without_matching_proof_cannot_retry() {
    let f=fixture();
    for broken in [false,true] {
        let fake=Fake::new(vec![Err("SDK_RAW_JSON_INVALID: 原文已保留".into())]);
        let mut after=recovery(&f,1,None);
        if broken {after["syntheticInvalidJsonProof"]=json!(true);after["breakInvalidJsonProof"]=json!(true);}
        fake.recoveries.lock().unwrap().push_back(after);
        assert!(invoke(&f,recovery(&f,0,None),&fake).await.err().unwrap().contains("SDK_RAW_JSON_INVALID"));
        assert_eq!(fake.requests.lock().unwrap().len(),1);assert!(fake.rejects.lock().unwrap().is_empty());
    }
    let fake=Fake::new(vec![Err("SDK_SCHEMA_REJECTED: 原格式拒绝".into())]);
    assert!(invoke(&f,recovery(&f,0,None),&fake).await.err().unwrap().contains("SDK_SCHEMA_REJECTED"));
    assert_eq!(*fake.context_calls.lock().unwrap(),0);assert!(fake.rejects.lock().unwrap().is_empty());
}
