//! Bounded decision dispatch over an immutable conflict manifest. The raw
//! catalogue is never resubmitted as editable model output.
use super::*;

fn latest_failure(recovery:&Value, cause:&Value)->Option<String> {
    let previous=recovery["previousAttempts"].as_array()?.last()?;
    if previous["status"]!="rejected" || previous.get("causeSha256").is_some_and(|v|v!=cause) {return None;}
    previous["error"].as_str().filter(|s|!s.trim().is_empty()).map(str::to_owned)
}

fn recovery_budget(recovery:&Value)->Result<u64,String> {
    let consumed=recovery["consumedAttempts"].as_u64().ok_or("FOUNDATION_CONFLICT_RECOVERY_BUDGET_MISSING")?;
    let history=recovery["previousAttempts"].as_array().ok_or("FOUNDATION_CONFLICT_RECOVERY_HISTORY_MISSING")?;
    if history.len() as u64!=consumed || consumed>2 {return Err("FOUNDATION_CONFLICT_RECOVERY_BUDGET_INVALID".into());}
    Ok(consumed)
}

fn bound_marker(recovery:&Value,manifest:&Value,attempt:u64)->Result<Value,String> {
    let marker=recovery.get("marker").filter(|m|m.is_object()).ok_or("FOUNDATION_CONFLICT_RECOVERY_MARKER_MISSING")?;
    if marker.as_object().is_none_or(|m|m.len()!=13)
        || marker["version"]!="foundation-conflict-resolution/v1" || marker["attempt"]!=attempt || marker["maximumAttempts"]!=2
        || marker["legacyConsumedAttempts"].as_u64().is_none_or(|n|n>=attempt)
        || (attempt==1)!=marker["previousAttemptSha256"].is_null()
        || (attempt>1 && !marker["previousAttemptSha256"].as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit()))) {
        return Err("FOUNDATION_CONFLICT_RECOVERY_MARKER_INVALID".into());
    }
    for field in ["sourceSha256","originalInputSha256","originalResultSha256","currentBasisSha256","issueSetSha256","causeSha256","manifestSha256"] {
        if marker[field]!=manifest[field]{return Err(format!("FOUNDATION_CONFLICT_RECOVERY_MARKER_MISMATCH:{field}"));}
    }
    Ok(marker.clone())
}

fn invalid_json_terminal(recovery:&Value,request:&TextRequest)->Result<bool,String> {
    let spent=recovery_budget(recovery)?;
    let Some(last)=recovery["previousAttempts"].as_array().and_then(|a|a.last()) else{return Ok(false)};
    let proof=&last["terminalRejection"];let marker=&request.input["foundationConflictResolution"];
    let Some(text)=proof["rawText"].as_str() else{return Ok(false)};
    Ok(last["legacy"]==false && last["status"]=="rejected" && last["resultSha256"].is_null()
        && !recovery["existingResolution"].is_object() && marker["attempt"]==spent
        && proof["version"]=="foundation-conflict-invalid-json/v1" && proof["code"]=="INVALID_JSON"
        && proof["attemptState"]=="completed" && proof["checkpointState"]=="rejected" && proof["parsedResult"]==false
        && proof["jobId"]==last["jobId"] && proof["workflowId"]==last["workflowId"]
        && proof["attempt"]==marker["attempt"] && proof["causeSha256"]==marker["causeSha256"]
        && proof["manifestSha256"]==marker["manifestSha256"]
        && proof["logicalRequestSha256"]==digest(&serde_json::to_value(request).map_err(|e|e.to_string())?.to_string())
        && proof["requestSha256"]==last["requestSha256"]
        && proof["rawTextSha256"]==digest(text) && last["rawSha256"]==proof["rawTextSha256"]
        && proof["rawEventId"].as_i64().is_some_and(|n|n>0)
        && proof["rawEventSha256"].as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit()))
        && serde_json::from_str::<Value>(text).is_err())
}

pub(super) async fn run(
    manifest: &conflict_resolution::Manifest,
    legacy: &TextRequest,
    mut recovery: Value,
    source: &LockedSource,
    root_source: &LockedSource,
    batch: &Batch,
    basis: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &FoundationProposal,
    context: &Value,
    executor: &dyn TextExecutor,
) -> Result<conflict_resolution::ResolutionOutcome, String> {
    let mut last_failure = latest_failure(&recovery,&manifest.input["causeSha256"]);
    for _ in 1..=2 {
        let consumed=recovery_budget(&recovery)?;
        let existing = recovery.get("existingResolution").filter(|v| v.is_object());
        for previous in recovery["previousAttempts"].as_array().ok_or("FOUNDATION_CONFLICT_RECOVERY_HISTORY_MISSING")? {
            if !matches!(previous["status"].as_str(),Some("completed"|"rejected"))
                && !existing.is_some_and(|v| v["jobId"]==previous["jobId"] && v["status"]==previous["status"]
                    && matches!(v["status"].as_str(),Some("queued"|"received"))) {
                return Err("FOUNDATION_CONFLICT_EXISTING_RESULT_UNRESOLVED".into());
            }
        }
        let attempt = recovery["nextAttempt"].as_u64();
        if existing.is_none() && !recovery["nextAttempt"].is_null() && attempt.is_none() {
            return Err("FOUNDATION_CONFLICT_RECOVERY_ATTEMPT_INVALID".into());
        }
        if existing.is_none() && attempt.is_some_and(|n| !(1..=2).contains(&n) || n!=consumed+1) {
            return Err("FOUNDATION_CONFLICT_RECOVERY_ATTEMPT_INVALID".into());
        }
        if existing.is_none() && attempt.is_none() {
            if consumed!=2 {return Err("FOUNDATION_CONFLICT_RECOVERY_NO_DISPOSITION".into());}
            return conflict_resolution::fallback(manifest, root_source, basis, current, candidate,
                "局部修订预算已用完；原成果保留，相关资产待核对");
        }
        let existing_status=existing.and_then(|v|v["status"].as_str());
        if existing.is_some() && !matches!(existing_status,Some("completed"|"received"|"queued")) {
            return Err("FOUNDATION_CONFLICT_EXISTING_RESULT_UNRESOLVED".into());
        }
        let request = if let Some(existing) = existing {
            let saved: TextRequest = serde_json::from_value(existing["request"].clone()).map_err(|e|e.to_string())?;
            if saved.input["foundationConflictManifest"] != manifest.input || saved.schema != manifest.schema
                || saved.stage!=legacy.stage || saved.skill!=legacy.skill || saved.model!=legacy.model || saved.reasoning!=legacy.reasoning
                || !saved.input["foundationConflictResolution"]["attempt"].as_u64().is_some_and(|n|(1..=2).contains(&n)&&n<=consumed) {
                return Err("FOUNDATION_CONFLICT_FROZEN_REQUEST_CHANGED".into());
            }
            bound_marker(&json!({"marker":saved.input["foundationConflictResolution"]}),&manifest.input,
                saved.input["foundationConflictResolution"]["attempt"].as_u64().ok_or("FOUNDATION_CONFLICT_RECOVERY_ATTEMPT_MISSING")?)?;
            saved
        } else {
            let stage = manifest.input["stage"].as_str().ok_or("冲突清单缺阶段")?;
            let mut input = input_for(stage, source, batch, current, context,
                legacy.input["batchIndex"].as_u64().unwrap_or(0) as usize,
                legacy.input["delivery"]["batchCount"].as_u64().unwrap_or(1) as usize, false, &[]);
            for key in ["registry","candidates","materialCandidates","modelInputProfile","foundationParallelRebase","foundationParallelFinal"] {
                input.as_object_mut().ok_or("冲突输入不是对象")?.remove(key);
            }
            input["foundationConflictManifest"] = manifest.input.clone();
            input["foundationConflictResolution"] = bound_marker(&recovery,&manifest.input,attempt.ok_or("FOUNDATION_CONFLICT_RECOVERY_ATTEMPT_MISSING")?)?;
            input["foundationConflictRepairHistory"] = recovery["previousAttempts"].clone();
            // The frozen request remains canonical. Reuse the existing
            // lossless transport compiler; only typed IDs shrink on the wire.
            input["modelInputProfile"] = json!(crate::pipeline::foundation_wire::PROFILE);
            input["transportInstructions"] = json!("这是版本化局部冲突决议岗位。只按当前schema返回foundationConflictManifest列明事项的决议，不重新输出资产目录或修改原稿。完整本批源文和历史证据供判断；items中的current/candidate和可选目标均为冻结证据。共同名字/别名不等于同一身份；证据不足选择保留独立待核对，不强行绑定。区分资产身份、已有变体用途与真正设计变化；已有设计与历史采用不可覆盖。引用必须来自当前给定来源。仅交schema允许的操作及预期值摘要，由程序应用并校验。");
            TextRequest {stage:legacy.stage.clone(),skill:legacy.skill.clone(),model:legacy.model.clone(),reasoning:legacy.reasoning.clone(),
                input,schema:manifest.schema.clone(),rule_snapshot:legacy.rule_snapshot.clone()}
        };
        let wire_request=if request.input["modelInputProfile"]==crate::pipeline::foundation_wire::PROFILE {
            let wire=crate::pipeline::foundation_wire::compile(&request.stage,&request.input,&request.schema)?;
            let mut transport=request.clone();transport.input=wire.input;transport.schema=wire.schema;transport
        } else {request.clone()};
        let wire_characters=executor.input_transport_characters(&wire_request)?;
        #[cfg(test)]
        eprintln!("FOUNDATION_CONFLICT_TRANSPORT canonical={} wire={} limit={}",executor.input_transport_characters(&request)?,wire_characters,MODEL_TRANSPORT_LIMIT);
        if !matches!(existing_status,Some("completed"|"received")) && wire_characters > MODEL_TRANSPORT_LIMIT {
            if existing_status==Some("queued") {return Err("FOUNDATION_CONFLICT_QUEUED_CAPACITY_HELD".into());}
            return conflict_resolution::fallback(manifest,root_source,basis,current,candidate,
                "冲突决议完整供料超过容量限制；未提交，相关资产待核对");
        }
        let original_request=serde_json::to_value(&request).map_err(|e|e.to_string())?;
        let request = executor.prepare_transport_request(request).await?;
        if existing.is_some() && serde_json::to_value(&request).map_err(|e|e.to_string())?!=original_request {
            return Err("FOUNDATION_CONFLICT_FROZEN_REQUEST_CHANGED".into());
        }
        // Transport errors, queued work and UNKNOWN remain held. They cannot be
        // interpreted as a failed creative decision or consume another dispatch.
        let raw = match executor.execute(request.clone()).await {
            Ok(raw)=>raw,
            Err(error)=>{
                // This machine prefix only selects a read-only verification.
                // It cannot authorize a retry without the exact durable proof.
                if !error.starts_with("SDK_RAW_JSON_INVALID:"){return Err(error);}
                let checked=executor.foundation_conflict_context(legacy,&manifest.input).await?;
                if !invalid_json_terminal(&checked,&request)? {return Err(error);}
                recovery=checked;
                if recovery_budget(&recovery)?==2 {
                    return conflict_resolution::fallback(manifest,root_source,basis,current,candidate,
                        "局部修订原回包格式无效且预算已用完；原始文本保留，相关资产待核对");
                }
                // The immutable malformed text travels in repair history; it
                // is never passed to reject as fabricated parsed JSON.
                last_failure=Some("INVALID_JSON".into());
                continue;
            }
        };
        match conflict_resolution::apply(manifest,&raw,root_source,basis,current,candidate) {
            Ok(outcome) => {
                executor.accept(&request,&raw).await?;
                return Ok(outcome);
            }
            Err(reason) => {
                if existing_status==Some("completed") {
                    return Err(format!("FOUNDATION_CONFLICT_ADOPTED_REVALIDATION_HELD:{reason}"));
                }
                executor.reject(&request,&raw,&reason).await?;
                if last_failure.as_deref() == Some(&reason) {
                    return conflict_resolution::fallback(manifest,root_source,basis,current,candidate,
                        "局部修订重复同因且无进展；停止重发，相关资产待核对");
                }
                last_failure = Some(reason);
                recovery = executor.foundation_conflict_context(legacy,&manifest.input).await?;
                let persisted=recovery_budget(&recovery)?;
                let expected=request.input["foundationConflictResolution"]["attempt"].as_u64().ok_or("FOUNDATION_CONFLICT_RECOVERY_ATTEMPT_MISSING")?;
                if persisted<expected || recovery["existingResolution"].is_object() {
                    return Err("FOUNDATION_CONFLICT_REJECTION_NOT_PERSISTED".into());
                }
                if let Some(durable)=latest_failure(&recovery,&manifest.input["causeSha256"]) {last_failure=Some(durable);}
            }
        }
    }
    conflict_resolution::fallback(manifest,root_source,basis,current,candidate,
        "局部修订达到两轮上限；原成果保留，相关资产待核对")
}

#[cfg(test)]
#[path="foundation_conflict_runner_tests.rs"]
mod tests;
