//! An explicit controller supplement belongs to the original workflow and its
//! next unused review-repair slot. It never edits an independent review receipt.
use super::*;

pub(crate) const FIELD: &str = "seriesControllerReviewSupplement";
pub(crate) const VERSION: &str = "series-controller-review-supplement/v1";
pub(crate) const HELD: &str = "SERIES_CONTROLLER_REVIEW_HELD";
const MODEL_FIELD: &str = "controllerReview";

fn candidate(req: &TextRequest) -> Result<Value, String> {
    Ok(json!({"schemaVersion":"series-episode-candidate/v1",
        "plan":packet_value(&req.input["packet"],"plan")?,
        "selection":packet_value(&req.input["packet"],"selection")?,
        "draft":packet_value(&req.input["packet"],"draft")?}))
}
fn index(req: &TextRequest) -> Result<usize, String> {
    (0..MAX_REPAIRS).find(|n| req.input["executionRoleId"] ==
        format!("{}/reviewer/{n}",s(&req.input,"episodeId"))).ok_or_else(||HELD.into())
}
fn next_boundary(input: &Value, episode: &str, refs: &Value) -> Result<Value, String> {
    let refs = string_list(refs)?;
    if refs.is_empty() { return Ok(Value::Null); }
    let source = source(input)?;
    let at = source.episodes.iter().position(|e|e.id==episode).ok_or(HELD)?;
    let next = source.episodes.get(at+1).ok_or(HELD)?;
    let scene = next.scene_ids.first().ok_or(HELD)?;
    let rows:Vec<_> = source.rows.iter().filter(|r|refs.contains(&r.id)).cloned().collect();
    if refs.len()>12 || rows.len()!=refs.len()
        || rows.iter().map(|r|r.id.clone()).collect::<Vec<_>>()!=refs
        || rows.iter().any(|r|r.kind!="body" || r.episode_id.as_deref()!=Some(&next.id)
            || r.scene_id.as_deref()!=Some(scene))
        || rows.iter().map(|r|r.range.end-r.range.start).sum::<usize>()>2048 {
        return Err(HELD.into());
    }
    Ok(json!({"mode":"read-only-next-episode-entry/v1","episodeId":next.id,
        "sourceSha256":source.sha256,"sourceRefs":refs,"sourceRows":source_view(&rows)}))
}

/// The durable adapter supplies the owned original reviewer, never caller text.
pub(crate) fn build_context(auth: &Value, review: &TextRequest, raw: &Value,
    input: &Value, workflow: &str, parent_sha: &str, snapshot_sha: &str,
    review_job: &str, node: &str) -> Result<Value,String> {
    validate_profile(input)?;
    let source=source(input)?;
    let ep=source.episodes.iter().find(|e|review.input["episodeId"]==e.id).ok_or(HELD)?;
    let before=candidate(review)?;
    let start=index(review)?;
    if auth["version"]!=VERSION || auth["sourceSha256"]!=source.sha256
        || auth["episodeId"]!=ep.id || auth["candidateSha256"]!=hash(&before)
        || auth["reviewJobId"]!=review_job || auth["stopAfterEpisodeId"]!=ep.id
        || !scope(input,&source)?.contains(&ep.id)
        || review.stage!="seriesReview" || review.skill!="series-reviewer"
        || review.input["role"]!="reviewer" || review.input.get("validationRepair").is_some()
        || review.input.get(MODEL_FIELD).is_some() || review.schema!=review_schema() {
        return Err(HELD.into());
    }
    validate_role_packet(review,input,&source,ep)?;
    validate_review(raw,&before,&source,ep,packet_value(&review.input["packet"],"entry")?)?;
    if raw["conclusion"]!="revise" {return Err(HELD.into());}
    let boundary=next_boundary(input,&ep.id,&auth["nextEpisodeSourceRefs"])?;
    let current:HashSet<_>=source.rows.iter().filter(|r|r.kind=="body" && r.episode_id.as_deref()==Some(&ep.id)).map(|r|r.id.as_str()).collect();
    let next:HashSet<_>=boundary["sourceRefs"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let issues=auth["issues"].as_array().filter(|v|!v.is_empty() && v.len()<=16).ok_or(HELD)?;
    let units=before["draft"]["units"].as_array().ok_or(HELD)?;
    let mut ids=HashSet::new();let mut covered=HashSet::new();
    for issue in issues {
        if s(issue,"id").is_empty() || !ids.insert(s(issue,"id")) || issue["severity"]!="must_fix"
            || s(issue,"problem").trim().is_empty() || s(issue,"requestedChange").trim().is_empty()
            || raw["issues"].as_array().into_iter().flatten().any(|old|old["id"]==issue["id"]) {
            return Err(HELD.into());
        }
        let affected=string_list(&issue["unitIds"])?;let shots=string_list(&issue["shotIds"])?;
        let insert=issue.get("allowInsertAfterShotIds").map(string_list).transpose()?.unwrap_or_default();
        if affected.is_empty() || shots.is_empty() || affected.iter().collect::<HashSet<_>>().len()!=affected.len()
            || shots.iter().collect::<HashSet<_>>().len()!=shots.len()
            || insert.iter().collect::<HashSet<_>>().len()!=insert.len() || insert.iter().any(|id|!shots.contains(id))
            || affected.iter().any(|id|!units.iter().any(|u|u["id"]==*id))
            || shots.iter().any(|id|!units.iter().filter(|u|affected.iter().any(|a|u["id"]==*a))
                .flat_map(|u|u["shots"].as_array().into_iter().flatten()).any(|s|s["id"]==*id))
            || string_list(&issue["sourceRefs"])?.iter().any(|r|!current.contains(r.as_str()) && !next.contains(r.as_str())) {
            return Err(HELD.into());
        }
        covered.extend(affected);
    }
    // Existing independent findings must remain repairable, never be discarded.
    if raw["issues"].as_array().ok_or(HELD)?.iter().filter(|i|i["severity"]=="must_fix")
        .flat_map(|i|i["unitIds"].as_array().into_iter().flatten())
        .any(|id|id.as_str().is_none_or(|id|!covered.contains(id))) {return Err(HELD.into());}
    let mut context=json!({"version":VERSION,"workflowId":workflow,"parentSnapshotSha256":parent_sha,
        "sourceSha256":source.sha256,"episodeId":ep.id,"candidateSha256":hash(&before),
        "originReviewJobId":review_job,"originReviewNodeKey":node,"originSnapshotSha256":snapshot_sha,
        "originRequestSha256":digest(&serde_json::to_string(review).map_err(|e|e.to_string())?),
        "originRawSha256":hash(raw),"assetsSha256":hash(packet_value(&review.input["packet"],"assets")?),
        "rulePackSha256":rule_pack_hash(input)?,"firstReviewIteration":start,"maximumRepairs":MAX_REPAIRS,
        "issues":issues,"nextEpisodeBoundary":boundary,"stopAfterEpisodeId":ep.id,
        "provenance":"controller supplement; original independent review unchanged"});
    context["sha256"]=json!(hash(&context));Ok(context)
}
pub(crate) fn validate_context(context:&Value, review:&TextRequest, raw:&Value,input:&Value)->Result<(),String>{
    let refs=context["nextEpisodeBoundary"].get("sourceRefs").cloned().unwrap_or(json!([]));
    let auth=json!({"version":VERSION,"sourceSha256":context["sourceSha256"],"episodeId":context["episodeId"],
        "candidateSha256":context["candidateSha256"],"reviewJobId":context["originReviewJobId"],
        "stopAfterEpisodeId":context["stopAfterEpisodeId"],"issues":context["issues"],"nextEpisodeSourceRefs":refs});
    let rebuilt=build_context(&auth,review,raw,input,s(context,"workflowId"),s(context,"parentSnapshotSha256"),
        s(context,"originSnapshotSha256"),s(context,"originReviewJobId"),s(context,"originReviewNodeKey"))?;
    if rebuilt!=*context {return Err(HELD.into());}Ok(())
}
pub(super) fn for_episode<'a>(input:&'a Value,episode:&str)->Option<&'a Value>{
    input.get(FIELD).filter(|c|c["episodeId"]==episode)
}
pub(super) fn marker(request:&TextRequest)->Option<&Value>{request.input.get(MODEL_FIELD)}
pub(super) fn add(values:&mut Vec<(&'static str,Value)>,context:&Value){
    values.push((MODEL_FIELD,context.clone()));
}
pub(super) fn attach(request:&mut TextRequest,context:&Value){
    request.input[MODEL_FIELD]=context.clone();
    request.input["controllerReviewContract"]=json!("补充意见由主控提交，不冒充旧独审；必须逐项修复/独审本包issues，下一集入口仅供只读接续，不能写入本集原文范围或新增能力。只改shotIds内镜头及必要单元说明；保留全部旧镜头ID、相对顺序以及其它镜头字段。仅可在allowInsertAfterShotIds指定旧镜之后、下一个旧镜之前插入必要邻镜，新增镜使用本授权范围内本集sourceRefs和唯一新ID。保留原话和既有设计，不把多步动作硬塞短镜。独审须逐项给controllerIssueAssessments；没有解决不能通过。");
}
pub(super) fn schema(context:Option<&Value>)->Value{
    let mut schema=review_schema();
    if context.is_some(){
        schema["properties"]["controllerIssueAssessments"]=array(obj(json!({"id":string(),
            "status":choices(&["resolved","unresolved"]),"explanation":string()})));
        schema["required"].as_array_mut().unwrap().push(json!("controllerIssueAssessments"));
    }schema
}
pub(super) fn effective_issues(request:&TextRequest,receipt:&Value,input:&Value)->Result<Value,String>{
    let Some(context)=marker(request) else{return Ok(receipt["raw"]["issues"].clone());};
    if packet_value(&request.input["packet"],MODEL_FIELD)?!=context{return Err(HELD.into());}
    let review:TextRequest=serde_json::from_value(receipt["request"].clone()).map_err(|_|HELD)?;
    let mut issues=receipt["raw"]["issues"].as_array().ok_or(HELD)?.clone();
    if receipt["requestSha256"]==context["originRequestSha256"] {
        validate_context(context,&review,&receipt["raw"],input)?;
        for i in context["issues"].as_array().ok_or(HELD)? {
            issues.push(json!({"id":i["id"],"severity":i["severity"],"unitIds":i["unitIds"],
                "sourceRefs":i["sourceRefs"],"problem":i["problem"],"requestedChange":i["requestedChange"]}));
        }
    }else if marker(&review)!=Some(context){return Err(HELD.into());}
    Ok(json!(issues))
}
pub(super) fn validate_review_context(raw:&Value,before:&Value,source:&LockedSource,episode:&SourceEpisode,
    entry:&Value,context:Option<&Value>)->Result<Value,String>{
    let Some(context)=context else{return validate_review(raw,before,source,episode,entry);};
    if context["version"]!=VERSION || context["sourceSha256"]!=source.sha256 || context["episodeId"]!=episode.id{return Err(HELD.into());}
    let next=string_list(context["nextEpisodeBoundary"].get("sourceRefs").unwrap_or(&json!([])))?;
    let mut local=raw.clone();
    // Cross-episode rows remain review evidence, never candidate/source coverage.
    for issue in local["issues"].as_array_mut().ok_or(HELD)? {
        let refs=string_list(&issue["sourceRefs"])?;
        issue["sourceRefs"]=json!(refs.into_iter().filter(|r|!next.contains(r)).collect::<Vec<_>>());
    }
    validate_review(&local,before,source,episode,entry)?;
    let assessed=raw["controllerIssueAssessments"].as_array().ok_or(HELD)?;
    let expected=context["issues"].as_array().ok_or(HELD)?;
    if assessed.len()!=expected.len(){return Err(HELD.into());}
    let mut seen=HashSet::new();
    for a in assessed {
        if !seen.insert(s(a,"id")) || !expected.iter().any(|i|i["id"]==a["id"])
            || s(a,"explanation").trim().is_empty() || !["resolved","unresolved"].contains(&s(a,"status"))
            || (a["status"]=="unresolved" && (raw["conclusion"]=="pass" || !raw["issues"].as_array().ok_or(HELD)?.iter().any(|i|i["id"]==a["id"] && i["severity"]=="must_fix"))) {
            return Err(HELD.into());
        }
    }Ok(raw.clone())
}
pub(super) fn assert_shots(request:&TextRequest,before:&Value,after:&Value)->Result<(),String>{
    let Some(context)=marker(request) else{return Ok(());};
    let allowed:HashSet<_>=context["issues"].as_array().ok_or(HELD)?.iter()
        .flat_map(|i|i["shotIds"].as_array().into_iter().flatten()).filter_map(Value::as_str).collect();
    let insert:HashSet<_>=context["issues"].as_array().ok_or(HELD)?.iter()
        .flat_map(|i|i["allowInsertAfterShotIds"].as_array().into_iter().flatten()).filter_map(Value::as_str).collect();
    let old=before["draft"]["units"].as_array().ok_or(HELD)?;
    let new=after["draft"]["units"].as_array().ok_or(HELD)?;
    if old.len()!=new.len(){return Err(HELD.into());}
    let shot_schema=&request.schema["properties"]["draft"]["properties"]["units"]["items"]["properties"]["shots"]["items"];
    for (a,b) in old.iter().zip(new){
        let aa=a["shots"].as_array().ok_or(HELD)?;let bb=b["shots"].as_array().ok_or(HELD)?;
        if a["id"]!=b["id"]{return Err("SERIES_CONTROLLER_REVIEW_SHOT_SCOPE".into());}
        let permitted_refs:HashSet<_>=aa.iter().filter(|sht|allowed.contains(s(sht,"id")))
            .flat_map(|sht|sht["sourceRefs"].as_array().into_iter().flatten()).filter_map(Value::as_str).collect();
        let mut at=0;let mut previous_old=None;
        for y in bb {
            if let Some(x)=aa.get(at).filter(|x|x["id"]==y["id"]){
                if !allowed.contains(s(x,"id")) && repair::schema_projection(x,shot_schema)?!=repair::schema_projection(y,shot_schema)? {
                    return Err("SERIES_CONTROLLER_REVIEW_SHOT_SCOPE".into());
                }
                previous_old=Some(s(x,"id"));at+=1;
            }else if aa.iter().any(|x|x["id"]==y["id"])
                || previous_old.is_none_or(|id|!insert.contains(id))
                || string_list(&y["sourceRefs"])?.iter().any(|r|!permitted_refs.contains(r.as_str())) {
                return Err("SERIES_CONTROLLER_REVIEW_SHOT_SCOPE".into());
            }
        }
        if at!=aa.len(){return Err("SERIES_CONTROLLER_REVIEW_SHOT_SCOPE".into());}
    }Ok(())
}

pub(super) fn validate_adoption(input:&Value,delivery:&Value)->Result<(),String>{
    let roles=delivery["adoptionReceipt"]["roleReceipts"].as_array().ok_or(HELD)?;
    let Some(context)=delivery["adoptionReceipt"].get(FIELD) else {
        if roles.iter().any(|r|r["request"]["input"].get(MODEL_FIELD).is_some()){return Err(HELD.into());}
        return Ok(());
    };
    let origin=roles.iter().find(|r|r["requestSha256"]==context["originRequestSha256"]
        && r["rawSha256"]==context["originRawSha256"]).ok_or(HELD)?;
    let request:TextRequest=serde_json::from_value(origin["request"].clone()).map_err(|_|HELD)?;
    validate_context(context,&request,&origin["raw"],input)?;
    let mut repaired=false;let mut final_review=false;
    for (position,r) in roles.iter().enumerate(){
        let req:TextRequest=serde_json::from_value(r["request"].clone()).map_err(|_|HELD)?;
        let Some(mark)=marker(&req) else{continue;};
        if mark!=context || packet_value(&req.input["packet"],MODEL_FIELD)?!=context{return Err(HELD.into());}
        if req.input["role"]=="repair" {
            let review=roles[..position].iter().rev().find(|r|r["role"]=="reviewer").ok_or(HELD)?;
            let issues=effective_issues(&req,review,input)?;
            if packet_value(&req.input["packet"],"issues")?!=&issues{return Err(HELD.into());}
            let iteration=(1..=MAX_REPAIRS).find(|n|s(&req.input,"executionRoleId")==format!("{}/repair/review-{n}",s(context,"episodeId"))
                || s(&req.input,"executionRoleId").starts_with(&format!("{}/repair/review-{n}/validation-repair/",s(context,"episodeId")))).ok_or(HELD)?;
            if iteration as u64<=context["firstReviewIteration"].as_u64().ok_or(HELD)?{return Err(HELD.into());}
            let before=candidate(&req)?;
            let source=source(input)?;let episode=source.episodes.iter().find(|e|e.id==s(context,"episodeId")).ok_or(HELD)?;
            repair::validate_scoped_candidate(&req,&r["raw"],&source,episode,
                delivery["bundle"]["assets"].as_array().ok_or(HELD)?,input,Some((&before,&issues)),
                delivery["adoptionReceipt"]["supplyContractCorrection"].get("episodeSupply")
                    .or_else(||delivery["adoptionReceipt"]["assetSupplement"].get("episodeSupply")))?;
            repaired=true;
        }else if req.input["role"]=="reviewer" {
            if req.schema!=schema(Some(context)) || !repaired{return Err(HELD.into());}
            final_review=r["raw"]["conclusion"]=="pass";
        }else{return Err(HELD.into());}
    }
    if !repaired || !final_review{return Err(HELD.into());}Ok(())
}
