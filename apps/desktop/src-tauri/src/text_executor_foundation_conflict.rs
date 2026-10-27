//! Durable two-round conflict budget. Reads immutable attempts; never repairs a
//! rejected receipt or turns an unknown submission into permission to dispatch.
use super::*;

pub(super) const VERSION: &str = "foundation-conflict-resolution/v1";
const FIELD: &str = "foundationConflictResolution";
const HELD: &str = "FOUNDATION_CONFLICT_HELD";

fn hash(value:&Value)->String { crate::pipeline::digest(&value.to_string()) }
fn sha(value:&Value)->bool {value.as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit()))}
fn sorted_conflicts(value:&Value)->Result<Value> {
    let values=value.as_array().filter(|v|!v.is_empty()).ok_or("FOUNDATION_CONFLICT_ISSUES_MISSING")?;
    let mut paths=values.iter().map(|v|v.as_str().filter(|s|s.starts_with("/assets/")).map(str::to_owned).ok_or("FOUNDATION_CONFLICT_ISSUES_INVALID".to_owned())).collect::<Result<Vec<_>>>()?;
    paths.sort();paths.dedup();
    if paths.len()!=values.len(){return Err("FOUNDATION_CONFLICT_ISSUES_DUPLICATED".into());}
    Ok(json!(paths))
}
fn cause(source:&Value,stage:&str,scope:&Value,original:&Value,conflicts:&Value)->Result<String> {
    Ok(hash(&json!({"sourceSha256":source,"stage":stage,"scope":scope,"originalResultSha256":original,"conflicts":sorted_conflicts(conflicts)?})))
}
fn manifest_bound(value:&Value)->Result<()> {
    if !["F2","Q1"].contains(&s(value,"stage")) || !sha(&value["sourceSha256"])
        || value["scope"].as_array().is_none_or(|a|a.is_empty()||a.iter().any(|v|!v.is_string()))
        || ["originalInputSha256","originalResultSha256","currentBasisSha256","issueSetSha256","causeSha256","manifestSha256"].iter().any(|k|!sha(&value[*k])) {
        return Err(format!("{HELD}: 冲突清单缺冻结身份"));
    }
    let paths=sorted_conflicts(&value["conflicts"])?;
    if value["issueSetSha256"]!=hash(&paths) || value["causeSha256"]!=cause(&value["sourceSha256"],s(value,"stage"),&value["scope"],&value["originalResultSha256"],&paths)? {
        return Err(format!("{HELD}: 冲突原因或问题集合摘要不匹配"));
    }
    let mut unsigned=value.clone();unsigned.as_object_mut().ok_or(HELD)?.remove("manifestSha256");
    if value["manifestSha256"]!=hash(&unsigned){return Err(format!("{HELD}: 冲突清单摘要不匹配"));}
    Ok(())
}

pub(super) fn validate_marker(request:&TextRequest)->Result<()> {
    let Some(marker)=request.input.get(FIELD) else{return Ok(())};
    let manifest=&request.input["foundationConflictManifest"];
    manifest_bound(manifest)?;
    if marker.as_object().is_none_or(|m|m.len()!=13) || marker["version"]!=VERSION || marker["maximumAttempts"]!=2
        || marker["attempt"].as_u64().is_none_or(|n|!(1..=2).contains(&n))
        || marker["legacyConsumedAttempts"].as_u64().is_none_or(|n|n>=marker["attempt"].as_u64().unwrap_or(0))
        || request.stage!=format!("foundation:{}",s(manifest,"stage"))
        || request.input["sourceSha256"]!=manifest["sourceSha256"] || request.input["scope"]!=manifest["scope"]
        || request.input["delivery"]["scope"]!=manifest["scope"]
        || request.input.get("foundationParallelRebase").is_some() || request.input.get("foundationParallelFinal").is_some()
        || request.input.get("foundationParallelTrial").is_none()
        || !(marker["legacyJobId"].is_null()||marker["legacyJobId"].as_str().is_some_and(|s|!s.is_empty()))
        || !(marker["previousAttemptSha256"].is_null()||sha(&marker["previousAttemptSha256"])) {
        return Err(format!("{HELD}: 新冲突节点合同无效"));
    }
    if (marker["attempt"]==1)!=marker["previousAttemptSha256"].is_null()
        || (marker["legacyConsumedAttempts"]==0 && !marker["legacyJobId"].is_null()) {
        return Err(format!("{HELD}: 新冲突节点前驱预算不一致"));
    }
    for key in ["sourceSha256","originalInputSha256","originalResultSha256","currentBasisSha256","issueSetSha256","causeSha256","manifestSha256"] {
        if marker[key]!=manifest[key]{return Err(format!("{HELD}: 新冲突节点{key}未绑定清单"));}
    }
    Ok(())
}

struct History { job:Value, snapshot:Value, run:String, legacy:bool }
enum VerifiedRecord { Parsed(Value), InvalidJson(Value) }
fn passive_state(job:&Value)->bool {
    matches!(s(job,"status"),"submission_unknown"|"submitting"|"running"|"interrupted")
}
fn passive_attempt_bound(entry:&History,run:&str,manifest:&Value,attempt:&crate::text_transport::FrozenAttempt)->Result<TextRequest> {
    use crate::text_transport::Backend;
    let request:TextRequest=serde_json::from_value(entry.snapshot.clone()).map_err(|e|e.to_string())?;
    validate_marker(&request)?;
    attempt.validate_identity()?;
    if entry.legacy || entry.run!=run || entry.job["provider"]!="codex_sdk" || !passive_state(&entry.job)
        || request.input["foundationConflictManifest"]!=*manifest
        || attempt.attempt_id!=s(&entry.job,"id") || attempt.workflow_id!=run
        || attempt.logical_node_id!=step_key(&request) || attempt.backend!=Backend::CodexSdk
        || !attempt.dispatch_started || attempt.runtime_identity!=entry.snapshot["executorIdentity"]
        || serde_json::to_value(&attempt.request).map_err(|e|e.to_string())?!=serde_json::to_value(&request).map_err(|e|e.to_string())? {
        return Err("FOUNDATION_CONFLICT_PASSIVE_BINDING: 仅可观察同流程同清单的原冻结已派发节点".into());
    }
    Ok(request)
}
async fn passive_completed(transport:&dyn crate::text_transport::TextTransport,attempt:&crate::text_transport::FrozenAttempt)->Result<crate::text_transport::Observation> {
    let observation=transport.reconcile(attempt).await?;
    if observation.kind!=crate::text_transport::ObservationKind::Completed || observation.attempt_id!=attempt.attempt_id {
        return Err("FOUNDATION_CONFLICT_UNKNOWN: 被动对账未证明原任务完成；预算保持，不重发".into());
    }
    // Validate the receipt without mutating the original attempt or allocating.
    let mut checked=attempt.clone();let applied=checked.observe(observation.clone())?;
    if checked.state!=crate::text_transport::AttemptState::Completed || checked.raw_result.is_none()
        || (!applied && checked.raw_result!=observation.raw_result) {
        return Err("FOUNDATION_CONFLICT_UNKNOWN: 完成回执缺原回包或原终态不匹配".into());
    }
    Ok(observation)
}
fn next_budget_bound(marker:&Value,history:&[History])->Result<()> {
    let next=history.len()+1;
    let previous=history.last().map(|h|json!(hash(&h.snapshot))).unwrap_or(Value::Null);
    if next>2 || marker["attempt"]!=next || marker["previousAttemptSha256"]!=previous
        || marker["legacyConsumedAttempts"]!=history.iter().filter(|h|h.legacy).count()
        || (!marker["legacyJobId"].is_null() && !history.iter().any(|h|h.legacy && h.job["id"]==marker["legacyJobId"])) {
        return Err("FOUNDATION_CONFLICT_BUDGET_EXHAUSTED: 原冲突两轮预算或前驱已变化，不能换流程重置".into());
    }
    Ok(())
}
impl SavedTextExecutor {
    /// Resume only observation of a new decision whose dispatch already exists.
    /// Never call sdk_execute or preparation-recovery helpers from this boundary.
    pub(super) async fn foundation_conflict_context_resumed(&self,legacy:&TextRequest,proposal:&Value)->Result<Value> {
        let original_error=match self.foundation_conflict_context_saved(legacy,proposal) {
            Ok(context)=>return Ok(context),
            Err(error) if error.contains("FOUNDATION_CONFLICT_UNKNOWN")=>error,
            Err(error)=>return Err(error),
        };
        if self.settings["textProvider"]!="codex_sdk" {return Err(original_error);}
        let _serial=SDK_DISPATCH.get_or_init(||tokio::sync::Mutex::new(())).lock().await;
        // Re-read after acquiring serialization: another callback may have finished.
        match self.foundation_conflict_context_saved(legacy,proposal) {
            Ok(context)=>return Ok(context),
            Err(error) if error.contains("FOUNDATION_CONFLICT_UNKNOWN")=>{},
            Err(error)=>return Err(error),
        }
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let history=self.foundation_conflict_history(proposal)?;
        let pending:Vec<_>=history.iter().filter(|h|passive_state(&h.job)).collect();
        if pending.len()!=1 || history.len()>2 {return Err(original_error);}
        let entry=pending[0];
        let path=self.store.root.join("text-runs").join(s(&entry.job,"id")).join("sdk-attempt.json");
        let original=crate::text_transport::FrozenAttempt::load(&path)?;
        let request=passive_attempt_bound(entry,run,proposal,&original)?;
        self.sdk_parent_binding()?;self.sdk_request_policy(&request)?;
        self.sdk_checkpoint_binding(&request,&entry.snapshot)?;
        self.foundation_conflict_existing_bound(&request,&entry.job,&entry.snapshot)?;
        self.sdk_parallel_worker_bound(&entry.job,&entry.snapshot)?;
        let identity:crate::text_sdk::SdkIdentity=serde_json::from_value(entry.snapshot["executorIdentity"].clone()).map_err(|e|e.to_string())?;
        identity.validate()?;
        let config=crate::text_sdk::Config::from_frozen_identity(&self.settings,&entry.snapshot["executorIdentity"],&self.store.root)?
            .for_request(&self.store.root,&request)?;
        let transport=self.sdk_transport(config,s(&entry.job,"id"));
        let observed=passive_completed(&transport,&original).await;
        let closed=transport.close().await;
        let observation=observed?;closed?;
        let mut latest=crate::text_transport::FrozenAttempt::load(&path)?;
        passive_attempt_bound(entry,run,proposal,&latest)?;
        if self.sdk_job(s(&entry.job,"id"))?!=entry.job {return Err("FOUNDATION_CONFLICT_PASSIVE_RACE: 原任务状态已改变".into());}
        latest.observe(observation)?;latest.persist(&path)?;
        if let Err(error)=self.sdk_receive(&request,&entry.snapshot,s(&entry.job,"id"),&latest) {
            // A completed original can have malformed JSON. Only the fully
            // verified persisted invalid-json terminal may release this hold.
            if let Ok(context)=self.foundation_conflict_context_saved(legacy,proposal) {
                if context["previousAttempts"].as_array().is_some_and(|rows|rows.iter().any(|row|
                    row["jobId"]==entry.job["id"] && row["terminalRejection"]["code"]=="INVALID_JSON")) {
                    return Ok(context);
                }
            }
            return Err(error);
        }
        let context=self.foundation_conflict_context_saved(legacy,proposal)?;
        if context["consumedAttempts"]!=history.len() {return Err("FOUNDATION_CONFLICT_PASSIVE_BUDGET_CHANGED".into());}
        Ok(context)
    }
    fn foundation_conflict_original_bound(&self,proposal:&Value)->Result<()> {
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let c=self.store.conn()?;
        let mut q=c.prepare("SELECT j.id,j.snapshot,w.result FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage=?2 AND w.state='accepted' AND j.state='completed' AND j.kind='text' AND j.provider='codex_sdk'").map_err(|e|e.to_string())?;
        let rows=q.query_map(params![run,format!("foundation:{}",s(proposal,"stage"))],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
        let mut count=0;
        for row in rows {
            let(id,text,result)=row.map_err(|e|e.to_string())?;
            let snapshot:Value=serde_json::from_str(&text).map_err(|e|e.to_string())?;
            if hash(&snapshot["input"])!=s(proposal,"originalInputSha256"){continue;}
            let accepted:Value=serde_json::from_str(&result).map_err(|e|e.to_string())?;
            if hash(&accepted)!=s(proposal,"originalResultSha256") || snapshot["input"]["sourceSha256"]!=proposal["sourceSha256"] || snapshot["input"]["scope"]!=proposal["scope"] {
                return Err("FOUNDATION_CONFLICT_ORIGINAL_MISMATCH".into());
            }
            let entry=History{job:self.sdk_job(&id)?,snapshot,run:run.into(),legacy:false};
            if !matches!(self.foundation_conflict_verified_record(&entry)?,VerifiedRecord::Parsed(_)) {
                return Err("FOUNDATION_CONFLICT_ORIGINAL_CHECKPOINT_MISMATCH".into());
            }
            if entry.job["prepared"]["sdkDomainResult"]!=accepted {return Err("FOUNDATION_CONFLICT_ORIGINAL_CHECKPOINT_MISMATCH".into());}
            count+=1;
        }
        if count!=1{return Err("FOUNDATION_CONFLICT_ORIGINAL_MISSING: 缺少唯一完整已采用原批，不能建立新修订".into());}
        Ok(())
    }
    fn foundation_conflict_history(&self, proposal:&Value)->Result<Vec<History>> {
        manifest_bound(proposal)?;
        let c=self.store.conn()?;
        let mut q=c.prepare("SELECT j.id,j.snapshot,w.workflow_id FROM jobs j LEFT JOIN workflow_steps w ON w.text_job_id=j.id WHERE j.kind='text' AND (json_extract(j.snapshot,'$.input.foundationParallelRebase') IS NOT NULL OR json_extract(j.snapshot,'$.input.foundationConflictResolution') IS NOT NULL) ORDER BY j.rowid").map_err(|e|e.to_string())?;
        let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).map_err(|e|e.to_string())?;
        let mut history=vec![];
        for row in rows {
            let(id,text,run)=row.map_err(|e|e.to_string())?;let snapshot:Value=serde_json::from_str(&text).map_err(|e|e.to_string())?;
            let input=&snapshot["input"];let new=input.get(FIELD);let legacy=new.is_none();
            let matches=if let Some(marker)=new { marker["causeSha256"]==proposal["causeSha256"] }
                else {let marker=&input["foundationParallelRebase"];cause(&input["sourceSha256"],s(&snapshot,"stage").strip_prefix("foundation:").unwrap_or(""),&input["scope"],&marker["originalResultSha256"],&marker["conflicts"]).is_ok_and(|h|proposal["causeSha256"]==h)};
            if !matches{continue;}
            let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|e|e.to_string())?;
            foundation_parallel::validate_markers(&request)?;
            let job=self.sdk_job(&id)?;
            history.push(History{job,snapshot,run:run.ok_or("FOUNDATION_CONFLICT_ORPHAN_ATTEMPT")?,legacy});
        }
        Ok(history)
    }

    fn foundation_conflict_verified_record(&self,entry:&History)->Result<VerifiedRecord> {
        if entry.job["provider"]!="codex_sdk" {
            return Err(format!("{HELD}: 原冲突记录来自其他入口，保留已消耗预算；新决议尚未支持跨入口恢复"));
        }
        let parent=self.store.job(&entry.run)?;
        let executor=SavedTextExecutor{store:self.store.clone(),settings:parent["snapshot"]["input"]["settings"].clone(),workflow_id:Some(entry.run.clone())};
        executor.sdk_parent_binding()?;
        let request:TextRequest=serde_json::from_value(entry.snapshot.clone()).map_err(|e|e.to_string())?;
        executor.sdk_request_policy(&request)?;
        executor.sdk_checkpoint_binding(&request,&entry.snapshot)?;
        if !["completed","rejected","received"].contains(&s(&entry.job,"status")) {
            return Err(format!("FOUNDATION_CONFLICT_UNKNOWN: 原冲突尝试 {} 尚无已核终态，保留原记录且不重发",s(&entry.job,"id")));
        }
        if entry.job["receipt"]["transportValidation"]=="invalid_json" {
            return executor.foundation_conflict_invalid_json(entry).map(VerifiedRecord::InvalidJson);
        }
        executor.sdk_raw(&entry.job,&entry.snapshot).map(VerifiedRecord::Parsed)
    }

    /// Distinct from parsed domain raw: this proof preserves the malformed text
    /// verbatim and never invents JSON for apply/accept/reject.
    fn foundation_conflict_invalid_json(&self,entry:&History)->Result<Value> {
        use crate::text_transport::{AttemptState,Backend,FrozenAttempt};
        let held=||"FOUNDATION_CONFLICT_INVALID_JSON_EVIDENCE_HELD".to_owned();
        let request:TextRequest=serde_json::from_value(entry.snapshot.clone()).map_err(|_|held())?;
        validate_marker(&request)?;
        let job=&entry.job;let snapshot=&entry.snapshot;let id=s(job,"id");let prepared=&job["prepared"];
        if entry.legacy||request.input.get(FIELD).is_none()||job["provider"]!="codex_sdk"||job["status"]!="rejected"
            ||job["receipt"]!=json!({"contract":"desktop-runner/v1","validation":"rejected","transportValidation":"invalid_json"})
            || !job["result"].is_null()||job["error"].as_str().is_none()||job["snapshot"]!=*snapshot
            ||prepared.get("sdkDomainResult").is_some()||prepared.get("sdkDomainResultSha256").is_some() {
            return Err(held());
        }
        self.sdk_handoff_job_bound(job,snapshot)?;self.sdk_schema_job_bound(job,snapshot)?;
        self.sdk_parallel_worker_bound(job,snapshot)?;self.sdk_local_recovery_bound(job)?;
        self.sdk_schema_recovery_bound(job)?;self.sdk_lane_recovery_bound(job)?;self.sdk_input_envelope_bound(job,snapshot)?;
        let dir=self.store.root.join("text-runs").join(id);
        let attempt=FrozenAttempt::load(&dir.join("sdk-attempt.json"))?;attempt.validate_identity()?;
        let request_hash=hash(snapshot);
        if attempt.state!=AttemptState::Completed||attempt.backend!=Backend::CodexSdk||!attempt.dispatch_started
            ||attempt.attempt_id!=id||attempt.workflow_id!=entry.run||attempt.logical_node_id!=step_key(&request)
            ||attempt.runtime_identity!=snapshot["executorIdentity"]
            ||serde_json::to_value(&attempt.request).map_err(|_|held())?!=json!(request)
            ||prepared["sdkRequestHash"]!=request_hash||prepared["sdkAttemptId"]!=id||prepared["sdkDispatchStarted"]!=true {
            return Err(held());
        }
        let compiled=compile_sdk_attempt_input(&attempt)?;
        let compiled_path=dir.join("sdk-compiled.json");
        if std::fs::metadata(&compiled_path).map_err(|_|held())?.len()>2_097_152{return Err(held());}
        let saved:Value=serde_json::from_slice(&std::fs::read(&compiled_path).map_err(|_|held())?).map_err(|_|held())?;
        if saved!=json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)})
            ||prepared["compiledPromptSha256"]!=saved["promptSha256"]||prepared["compiledSchemaSha256"]!=saved["schemaSha256"] {return Err(held());}
        let envelope=attempt.raw_result.as_ref().ok_or_else(held)?;
        let text=envelope["rawText"].as_str().ok_or_else(held)?;let raw_hash=crate::pipeline::digest(text);
        let path=dir.join("result.json");
        if std::fs::metadata(&path).map_err(|_|held())?.len()>1_048_576{return Err(held());}
        if std::fs::read(&path).map_err(|_|held())?!=text.as_bytes()||prepared["sdkRawTextSha256"]!=raw_hash
            ||serde_json::from_str::<Value>(text).is_ok() {return Err(held());}
        let c=self.store.conn()?;
        let mut q=c.prepare("SELECT id,data FROM events WHERE job_id=?1 AND type='sdk_raw_result' ORDER BY id").map_err(|_|held())?;
        let events=q.query_map([id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).map_err(|_|held())?
            .collect::<std::result::Result<Vec<_>,_>>().map_err(|_|held())?;
        if events.len()!=1||serde_json::from_str::<Value>(&events[0].1).map_err(|_|held())?!=*envelope {return Err(held());}
        let received:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type='text_received_raw')",[id],|r|r.get(0)).map_err(|_|held())?;
        let checkpoint:Option<(String,String,String,String,Option<String>,Option<String>)>=c.query_row(
            "SELECT stage,request_hash,state,text_job_id,result,error FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2",
            params![entry.run,step_key(&request)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|held())?;
        if received||checkpoint!=Some((request.stage.clone(),request_hash.clone(),"rejected".into(),id.into(),None,Some(s(job,"error").into()))) {return Err(held());}
        Ok(json!({"version":"foundation-conflict-invalid-json/v1","code":"INVALID_JSON","attemptState":"completed",
            "jobId":id,"workflowId":entry.run,"attempt":request.input[FIELD]["attempt"],"causeSha256":request.input[FIELD]["causeSha256"],
            "manifestSha256":request.input[FIELD]["manifestSha256"],"requestSha256":request_hash,"logicalRequestSha256":hash(&json!(request)),
            "rawText":text,"rawTextSha256":raw_hash,"rawEventId":events[0].0,"rawEventSha256":crate::pipeline::digest(&events[0].1),
            "checkpointState":"rejected","parsedResult":false}))
    }

    /// Compatibility read only. The old CLI/API accepted cache must remain
    /// readable even though allocation of the new decision protocol is SDK-only.
    /// Use the original frozen-request/cache reader, never status alone and never
    /// the mutating CLI cache-claim helper. No new attempt is authorized here.
    fn foundation_conflict_non_sdk_legacy(&self,legacy:&TextRequest,proposal:&Value)->Result<Value> {
        let configured=s(&self.settings,"textProvider");
        let provider=if configured.is_empty(){"codex"}else{configured};
        if !["codex","api"].contains(&provider){return Err(format!("{HELD}: 未知文字入口"));}
        foundation_parallel::validate_markers(legacy)?;
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let snapshot=self.request_snapshot(legacy)?;
        let request_hash=hash(&snapshot);
        let history=self.foundation_conflict_history(proposal)?;
        // A provider switch or new workflow never erases unresolved attempts.
        // This path only reads an old accepted node; it cannot spend any budget.
        if history.iter().any(|h| !["completed","rejected"].contains(&s(&h.job,"status"))) {
            return Err("FOUNDATION_CONFLICT_UNKNOWN: 原冲突存在未核终态；保留原请求和预算，不重发".into());
        }
        let exact=history.iter().filter(|h|h.legacy && h.run==run && h.snapshot==snapshot).collect::<Vec<_>>();
        if exact.len()!=1 || exact[0].job["status"]!="completed" || exact[0].job["provider"]!=provider {
            return Err(format!("{HELD}: 当前入口没有可按原合同复用的已采用重基；新冲突决议仅支持已验证 SDK，原预算保留"));
        }
        let entry=exact[0];
        let managed=provider=="codex" && configured=="codex"
            && crate::text_cli::configured_managed_path(&self.settings).is_some();
        // These are the same readers used before execute's allocation boundary.
        let raw=if managed {self.managed_validation_value(legacy,&snapshot)?}
            else {self.store.accepted_step(run,&request_hash)?}
            .ok_or("FOUNDATION_CONFLICT_LEGACY_CACHE_HELD: 原冻结缓存缺少已采用结果")?;
        let bound:Option<(String,String)>=self.store.conn()?.query_row(
            "SELECT w.result,j.receipt FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND w.stage=?3 AND w.request_hash=?4 AND w.state='accepted' AND w.error IS NULL AND j.id=?5 AND j.kind='text' AND j.provider=?6 AND j.state='completed' AND j.snapshot=?7",
            params![run,step_key(legacy),legacy.stage,request_hash,s(&entry.job,"id"),provider,snapshot.to_string()],
            |r|Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(|e|e.to_string())?;
        let (accepted,receipt)=bound.ok_or("FOUNDATION_CONFLICT_LEGACY_CACHE_HELD: 原节点与冻结任务绑定不完整")?;
        let accepted:Value=serde_json::from_str(&accepted).map_err(|e|e.to_string())?;
        let receipt:Value=serde_json::from_str(&receipt).map_err(|e|e.to_string())?;
        if receipt["validation"]!="accepted" || receipt["contract"]!="desktop-runner/v1"
            || receipt.get("acceptedRequestHash").is_some_and(|v|v!=&json!(request_hash))
            || (!managed && (accepted!=raw || entry.job["result"]!=accepted || !receipt["migrationPolicy"].is_null())) {
            return Err("FOUNDATION_CONFLICT_LEGACY_CACHE_HELD: 原已采用回执或结果绑定不完整".into());
        }
        Ok(json!({"legacyAccepted":{"raw":raw,"request":legacy,"jobId":entry.job["id"]},
            "existingResolution":null,"consumedAttempts":history.len(),"nextAttempt":null,
            "previousAttempts":history.iter().map(|h|json!({"jobId":h.job["id"],"workflowId":h.run,"legacy":h.legacy,
                "provider":h.job["provider"],"status":h.job["status"],"requestSha256":hash(&h.snapshot),"error":h.job["error"]})).collect::<Vec<_>>(),
            "marker":null}))
    }

    /// Read existing legacy attempts and the project-wide cause ledger. Nothing
    /// is allocated here. The same ledger is checked under dispatch serialization.
    pub(super) fn foundation_conflict_context_saved(&self,legacy:&TextRequest,proposal:&Value)->Result<Value> {
        let _evidence_scope=sdk_evidence_cache::scope();
        manifest_bound(proposal)?;
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let old=&legacy.input["foundationParallelRebase"];
        for key in ["originalInputSha256","originalResultSha256","currentBasisSha256"] {
            if old[key]!=proposal[key]{return Err(format!("{HELD}: 原重基{key}不匹配"));}
        }
        if legacy.stage!=format!("foundation:{}",s(proposal,"stage")) || legacy.input["scope"]!=proposal["scope"]
            || legacy.input["sourceSha256"]!=proposal["sourceSha256"] || sorted_conflicts(&old["conflicts"])?!=sorted_conflicts(&proposal["conflicts"])? {
            return Err(format!("{HELD}: 原重基范围不匹配"));
        }
        if self.settings["textProvider"]!="codex_sdk" {
            return self.foundation_conflict_non_sdk_legacy(legacy,proposal);
        }
        self.sdk_parent_binding()?;self.sdk_request_policy(legacy)?;
        crate::foundation_trial_store::validate_request(&self.store,run,&legacy.input["foundationParallelTrial"])?;
        self.foundation_conflict_original_bound(proposal)?;
        let history=self.foundation_conflict_history(proposal)?;
        let mut existing_resolution=Value::Null;let mut legacy_accepted=Value::Null;let mut legacy_job=Value::Null;let mut previous=Value::Null;let mut summaries=vec![];
        for entry in &history {
            let matching_resolution=!entry.legacy && entry.run==run && entry.snapshot["input"][FIELD]["manifestSha256"]==proposal["manifestSha256"];
            let verified=if matching_resolution && entry.job["status"]=="queued" {
                let request:TextRequest=serde_json::from_value(entry.snapshot.clone()).map_err(|e|e.to_string())?;
                self.sdk_checkpoint_binding(&request,&entry.snapshot)?;
                self.foundation_conflict_existing_bound(&request,&entry.job,&entry.snapshot)?;
                // Resume only the normal SDK's proven never-dispatched queued
                // reservation. Handoff/unknown preparations fail closed here.
                self.sdk_pre_dispatch_evidence(&request,&entry.snapshot,s(&entry.job,"id"),&self.store.conn()?)?;
                VerifiedRecord::Parsed(Value::Null)
            } else {self.foundation_conflict_verified_record(entry)?};
            let (raw,terminal)=match verified {VerifiedRecord::Parsed(value)=>(Some(value),None),VerifiedRecord::InvalidJson(proof)=>(None,Some(proof))};
            if entry.legacy && entry.run==run && serde_json::from_value::<TextRequest>(entry.snapshot.clone()).ok().and_then(|r|serde_json::to_value(r).ok())==serde_json::to_value(legacy).ok() {
                legacy_job=entry.job["id"].clone();
                if entry.job["status"]=="completed" {legacy_accepted=json!({"raw":raw,"request":legacy,"jobId":entry.job["id"]});}
            }
            if matching_resolution && ["completed","received","queued"].contains(&s(&entry.job,"status")) {
                if !existing_resolution.is_null(){return Err("FOUNDATION_CONFLICT_DUPLICATE_RESULT".into());}
                existing_resolution=json!({"raw":raw,"request":serde_json::from_value::<TextRequest>(entry.snapshot.clone()).map_err(|e|e.to_string())?,"jobId":entry.job["id"],"status":entry.job["status"]});
            } else if entry.job["status"]=="received" {return Err("FOUNDATION_CONFLICT_UNADOPTED_RESULT: 原已回包节点须先按原合同采用，不分配新尝试".into());}
            previous=json!(hash(&entry.snapshot));
            let mut summary=json!({"jobId":entry.job["id"],"workflowId":entry.run,"legacy":entry.legacy,"status":entry.job["status"],"requestSha256":hash(&entry.snapshot),"resultSha256":raw.as_ref().map(hash),"rawSha256":raw.as_ref().map(hash),"error":entry.job["error"]});
            if let Some(proof)=terminal {summary["rawSha256"]=proof["rawTextSha256"].clone();summary["terminalRejection"]=proof;}
            summaries.push(summary);
        }
        let count=history.len();let next=if legacy_accepted.is_null()&&existing_resolution.is_null()&&count<2 {json!(count+1)}else{Value::Null};
        let mut marker=json!({"version":VERSION,"attempt":next,"maximumAttempts":2,"legacyJobId":legacy_job,"previousAttemptSha256":previous});
        for key in ["sourceSha256","originalInputSha256","originalResultSha256","currentBasisSha256","issueSetSha256","causeSha256","manifestSha256"] {marker[key]=proposal[key].clone();}
        // Twelve fields are semantic; the explicit old-contract count makes
        // upgrades account for consumed legacy repair work without rewriting it.
        marker["legacyConsumedAttempts"]=json!(history.iter().filter(|h|h.legacy).count());
        Ok(json!({"legacyAccepted":legacy_accepted,"existingResolution":existing_resolution,"consumedAttempts":count,"nextAttempt":next,"previousAttempts":summaries,"marker":marker}))
    }

    pub(super) fn foundation_conflict_dispatch_bound(&self,request:&TextRequest)->Result<()> {
        let Some(marker)=request.input.get(FIELD) else{return Ok(())};
        let _evidence_scope=sdk_evidence_cache::scope();
        validate_marker(request)?;
        if self.settings["textProvider"]!="codex_sdk" {return Err(format!("{HELD}: 当前冲突恢复仅支持已验证SDK入口"));}
        self.foundation_conflict_original_bound(&request.input["foundationConflictManifest"])?;
        let history=self.foundation_conflict_history(&request.input["foundationConflictManifest"])?;
        for entry in &history {self.foundation_conflict_verified_record(entry)?;if entry.job["status"]=="received" {return Err("FOUNDATION_CONFLICT_UNADOPTED_RESULT".into());}}
        next_budget_bound(marker,&history)
    }

    /// Existing queued/returned/unknown nodes remain owned by their exact
    /// snapshot. This check grants no submission or retry permission.
    pub(super) fn foundation_conflict_existing_bound(&self,request:&TextRequest,job:&Value,snapshot:&Value)->Result<()> {
        let Some(marker)=request.input.get(FIELD) else{return Ok(())};
        let _evidence_scope=sdk_evidence_cache::scope();
        validate_marker(request)?;
        let proposal=&request.input["foundationConflictManifest"];
        self.foundation_conflict_original_bound(proposal)?;
        let history=self.foundation_conflict_history(proposal)?;
        let position=history.iter().position(|h|h.job["id"]==job["id"]).ok_or("FOUNDATION_CONFLICT_ATTEMPT_MISSING")?;
        let own=&history[position];
        let previous=position.checked_sub(1).map(|i|json!(hash(&history[i].snapshot))).unwrap_or(Value::Null);
        if own.snapshot!=*snapshot || own.run!=self.workflow_id.as_deref().unwrap_or("") || history.len()>2
            || marker["attempt"]!=position+1 || marker["previousAttemptSha256"]!=previous
            || marker["legacyConsumedAttempts"]!=history[..position].iter().filter(|h|h.legacy).count() {
            return Err("FOUNDATION_CONFLICT_EXISTING_BINDING_MISMATCH".into());
        }
        for prior in &history[..position] {
            self.foundation_conflict_verified_record(prior)?;
            if prior.job["status"]=="received" {return Err("FOUNDATION_CONFLICT_UNADOPTED_RESULT".into());}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_transport::{Backend,FrozenAttempt,Observation,ObservationKind,TextTransport};
    use std::sync::atomic::{AtomicUsize,Ordering};
    struct ObserveOnly { result:Observation, executes:AtomicUsize, observations:AtomicUsize }
    #[async_trait::async_trait]
    impl TextTransport for ObserveOnly {
        async fn inspect(&self)->Result<Value>{panic!("passive recovery must not inspect or execute")}
        async fn execute(&self,_:&FrozenAttempt)->Result<Observation>{self.executes.fetch_add(1,Ordering::SeqCst);Err("unexpected execute".into())}
        async fn reconcile(&self,_:&FrozenAttempt)->Result<Observation>{self.observations.fetch_add(1,Ordering::SeqCst);Ok(self.result.clone())}
        async fn cancel(&self,_:&FrozenAttempt)->Result<Observation>{panic!("passive recovery must not cancel")}
    }
    fn passive_fixture()->(History,FrozenAttempt) {
        let r=request();let mut snapshot=json!(r);snapshot["executorIdentity"]=json!({"fixture":"same-frozen-runtime"});
        let mut attempt=FrozenAttempt::prepare("pending","run1",&step_key(&r),Backend::CodexSdk,r,&snapshot["executorIdentity"]).unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        (History{job:json!({"id":"pending","provider":"codex_sdk","status":"submission_unknown"}),snapshot,run:"run1".into(),legacy:false},attempt)
    }
    #[tokio::test]
    async fn foundation_conflict_passive_completion_and_unknown_never_execute_or_spend_budget() {
        for kind in [ObservationKind::Completed,ObservationKind::SubmissionUnknown,ObservationKind::Rejected] {
            let (entry,attempt)=passive_fixture();let before=json!(attempt);
            let manifest=entry.snapshot["input"]["foundationConflictManifest"].clone();
            passive_attempt_bound(&entry,"run1",&manifest,&attempt).unwrap();
            let history=vec![entry];let consumed=history.len();
            let transport=ObserveOnly{result:Observation{attempt_id:"pending".into(),sequence:1,kind:kind.clone(),thread_id:Some("thread".into()),turn_id:None,
                raw_result:if kind==ObservationKind::Completed{Some(json!({"rawText":"{\"decisions\":[]}"}))}else{None}},executes:AtomicUsize::new(0),observations:AtomicUsize::new(0)};
            let result=passive_completed(&transport,&attempt).await;
            assert_eq!(result.is_ok(),kind==ObservationKind::Completed);
            assert_eq!(transport.executes.load(Ordering::SeqCst),0);
            assert_eq!(transport.observations.load(Ordering::SeqCst),1);
            assert_eq!(json!(attempt),before);assert_eq!(history.len(),consumed);
            assert_eq!(history[0].job["status"],"submission_unknown");
        }
    }
    #[test]
    fn foundation_conflict_passive_binding_cannot_cross_run_manifest_or_preparation_boundary() {
        for mutation in ["run","manifest","job","request","runtime","prepared","legacy","provider"] {
            let (mut entry,mut attempt)=passive_fixture();let mut manifest=entry.snapshot["input"]["foundationConflictManifest"].clone();
            match mutation {
                "run"=>entry.run="other".into(),
                "manifest"=>manifest["currentBasisSha256"]=json!("9".repeat(64)),
                "job"=>entry.job["id"]=json!("other"),
                "request"=>attempt.request.input["foreign"]=json!(true),
                "runtime"=>entry.snapshot["executorIdentity"]=json!({"other":true}),
                "prepared"=>attempt.dispatch_started=false,
                "legacy"=>entry.legacy=true,
                _=>entry.job["provider"]=json!("api"),
            }
            assert!(passive_attempt_bound(&entry,"run1",&manifest,&attempt).is_err(),"{mutation}");
        }
    }
    fn proposal()->Value {
        let mut value=json!({"sourceSha256":"a".repeat(64),"stage":"F2","scope":["ep2"],"originalInputSha256":"b".repeat(64),
            "originalResultSha256":"c".repeat(64),"currentBasisSha256":"d".repeat(64),"conflicts":["/assets/a/description"]});
        value["issueSetSha256"]=json!(hash(&value["conflicts"]));
        value["causeSha256"]=json!(cause(&value["sourceSha256"],"F2",&value["scope"],&value["originalResultSha256"],&value["conflicts"]).unwrap());
        value["manifestSha256"]=json!(hash(&value));value
    }
    fn marker(p:&Value)->Value {
        let mut marker=json!({"version":VERSION,"attempt":1,"maximumAttempts":2,"legacyJobId":null,"previousAttemptSha256":null,"legacyConsumedAttempts":0});
        for k in ["sourceSha256","originalInputSha256","originalResultSha256","currentBasisSha256","issueSetSha256","causeSha256","manifestSha256"] {marker[k]=p[k].clone();}marker
    }
    fn request()->TextRequest {
        let p=proposal();
        TextRequest{stage:"foundation:F2".into(),skill:"foundation-asset-catalog".into(),model:"fixture".into(),reasoning:"medium".into(),
            input:json!({"sourceSha256":p["sourceSha256"],"scope":p["scope"],"delivery":{"method":"foundation-delivery/v2","scope":p["scope"],"batchIndex":3},
                "foundationConflictManifest":p,"foundationConflictResolution":marker(&p),
                "foundationParallelTrial":{"version":foundation_parallel::PROFILE,"concurrency":3,"authorizationSha256":"e".repeat(64),"sourceSha256":p["sourceSha256"],"basisSha256":"d".repeat(64),"workflowId":"run1","laneId":0,"packetId":"batch3"}}),schema:json!({"type":"object"}),rule_snapshot:json!({})}
    }
    fn invalid_json_fixture()->(tempfile::TempDir,SavedTextExecutor,History) {
        use crate::text_transport::{AttemptState,Backend,FrozenAttempt};
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();
        store.create("合成坏JSON决议","fixture").unwrap();store.ensure_workflow_storage().unwrap();
        let settings=crate::text_sdk::offline_settings("happy");
        let run=store.add_job("workflow","foundation.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","input":{"settings":settings}}),"running").unwrap();
        let executor=SavedTextExecutor{store,settings,workflow_id:Some(run.clone())};
        let mut r=request();r.input["foundationParallelTrial"]["workflowId"]=json!(run);
        let snapshot=executor.request_snapshot(&r).unwrap();
        let job=executor.store.add_job("text",&r.stage,"codex_sdk","fixture",&snapshot,"rejected").unwrap();
        let mut attempt=FrozenAttempt::prepare(&job,&run,&step_key(&r),Backend::CodexSdk,r.clone(),&snapshot["executorIdentity"]).unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();attempt.state=AttemptState::Completed;
        let text="{\"decisions\":[ 原始坏文本不可改";
        let envelope=json!({"rawText":text,"parsedJson":null,"parseError":"synthetic parse failure","domainAccepted":false});
        attempt.raw_result=Some(envelope.clone());
        let compiled=compile_sdk_attempt_input(&attempt).unwrap();
        let prepared=json!({"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),
            "sdkRequestHash":hash(&snapshot),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":run,"sdkRawTextSha256":crate::pipeline::digest(text),
            "sdkWorkerScope":{"version":"foundation-sdk-lane-worker/v1","authorizationSha256":r.input["foundationParallelTrial"]["authorizationSha256"],"laneId":0,
                "workerRoot":crate::text_sdk::worker_root(&executor.store.root,&r).unwrap()}});
        executor.store.update_job(&job,"rejected",&json!({"prepared":prepared,"error":"不同中文说明不改变机器合同",
            "receipt":{"contract":"desktop-runner/v1","validation":"rejected","transportValidation":"invalid_json"}})).unwrap();
        executor.store.checkpoint(&run,&step_key(&r),&r.stage,&hash(&snapshot),"rejected",Some(&job),None,Some("不同中文说明不改变机器合同")).unwrap();
        executor.store.event(&job,"sdk_raw_result",&envelope).unwrap();
        let dir=executor.store.root.join("text-runs").join(&job);std::fs::create_dir_all(&dir).unwrap();
        attempt.persist(&dir.join("sdk-attempt.json")).unwrap();std::fs::write(dir.join("result.json"),text).unwrap();
        crate::store::atomic_json(&dir.join("sdk-compiled.json"),&json!({"prompt":compiled.prompt,"schema":compiled.schema,
            "promptSha256":prepared["compiledPromptSha256"],"schemaSha256":prepared["compiledSchemaSha256"]})).unwrap();
        let entry=History{job:executor.sdk_job(&job).unwrap(),snapshot,run,legacy:false};(temp,executor,entry)
    }
    #[test]
    fn foundation_conflict_invalid_json_proof_keeps_completed_bad_text_without_parsed_raw() {
        let (_temp,executor,entry)=invalid_json_fixture();let before=executor.store.jobs().unwrap();
        let proof=Store::with_readonly_connections(&executor.store.root,||executor.foundation_conflict_invalid_json(&entry)).unwrap();
        assert_eq!(proof["code"],"INVALID_JSON");assert_eq!(proof["parsedResult"],false);
        assert_eq!(proof["rawText"],"{\"decisions\":[ 原始坏文本不可改");assert_eq!(proof["attempt"],1);
        assert!(entry.job["result"].is_null());assert_eq!(executor.store.jobs().unwrap(),before);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }
    #[test]
    fn foundation_conflict_invalid_json_requires_full_terminal_evidence_not_error_words() {
        for mutation in ["unknown","schema","legacy","parsed","checkpoint","event","duplicate_event","result.json","sdk-attempt.json","sdk-compiled.json"] {
            let (_temp,executor,mut entry)=invalid_json_fixture();let id=s(&entry.job,"id").to_owned();
            match mutation {
                "unknown"=>entry.job["status"]=json!("submission_unknown"),
                "schema"=>entry.job["receipt"]["transportValidation"]=json!("schema_rejected"),
                "legacy"=>entry.legacy=true,
                "parsed"=>entry.job["result"]=json!({}),
                "checkpoint"=>{executor.store.conn().unwrap().execute("UPDATE workflow_steps SET result='{}' WHERE text_job_id=?1",[&id]).unwrap();},
                "event"=>{executor.store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_raw_result'",[&id]).unwrap();},
                "duplicate_event"=>{let event:String=executor.store.conn().unwrap().query_row("SELECT data FROM events WHERE job_id=?1 AND type='sdk_raw_result'",[&id],|r|r.get(0)).unwrap();executor.store.event(&id,"sdk_raw_result",&serde_json::from_str::<Value>(&event).unwrap()).unwrap();},
                file=>std::fs::write(executor.store.root.join("text-runs").join(&id).join(file),b"changed").unwrap(),
            }
            let before=executor.store.jobs().unwrap();
            assert!(executor.foundation_conflict_invalid_json(&entry).is_err(),"{mutation}");assert_eq!(executor.store.jobs().unwrap(),before);
        }
    }
    #[test]
    fn foundation_conflict_marker_freezes_manifest_and_preserves_legacy_key() {
        let r=request();foundation_parallel::validate_markers(&r).unwrap();
        let mut old=r.clone();old.input.as_object_mut().unwrap().remove(FIELD);old.input.as_object_mut().unwrap().remove("foundationConflictManifest");
        assert_ne!(step_key(&r),step_key(&old));
        assert_eq!(crate::text_sdk::worker_root(&std::env::temp_dir(),&r).unwrap(),crate::text_sdk::worker_root(&std::env::temp_dir(),&old).unwrap());
        for field in ["sourceSha256","originalInputSha256","originalResultSha256","currentBasisSha256","issueSetSha256","causeSha256","manifestSha256","attempt","maximumAttempts","legacyConsumedAttempts"] {
            let mut forged=r.clone();forged.input[FIELD][field]=json!("forged");assert!(validate_marker(&forged).is_err(),"{field}");
        }
        let mut forged=r.clone();forged.input["foundationConflictManifest"]["conflicts"]=json!(["/assets/other/description"]);assert!(validate_marker(&forged).is_err());
        let mut forged=r.clone();forged.input["foundationParallelRebase"]=json!({"version":"foundation-parallel-rebase/v1","attempt":1});assert!(validate_marker(&forged).is_err());
    }
    #[test]
    fn foundation_conflict_cause_excludes_runtime_and_basis_but_not_source_or_scope() {
        let p=proposal();let mut changed=p.clone();changed["currentBasisSha256"]=json!("f".repeat(64));changed["workflowId"]=json!("different");changed["account"]=json!("different");
        let key=|v:&Value|cause(&v["sourceSha256"],s(v,"stage"),&v["scope"],&v["originalResultSha256"],&v["conflicts"]).unwrap();
        assert_eq!(key(&p),key(&changed));
        for field in ["sourceSha256","scope","originalResultSha256","conflicts"] {
            let mut changed=p.clone();changed[field]=match field {"scope"=>json!(["ep3"]),"conflicts"=>json!(["/assets/b/description"]),_=>json!("f".repeat(64))};assert_ne!(key(&p),key(&changed));
        }
    }
    #[test]
    fn foundation_conflict_budget_counts_legacy_and_cannot_restart_third_attempt() {
        let mut m=marker(&proposal());assert!(next_budget_bound(&m,&[]).is_ok());
        let history=vec![History{job:json!({"id":"old","status":"rejected"}),snapshot:json!({"old":"frozen"}),run:"old-run".into(),legacy:true}];
        assert!(next_budget_bound(&m,&history).is_err());
        m["attempt"]=json!(2);m["legacyConsumedAttempts"]=json!(1);m["legacyJobId"]=json!("old");m["previousAttemptSha256"]=json!(hash(&history[0].snapshot));
        assert!(next_budget_bound(&m,&history).is_ok());
        let mut both=history;both.push(History{job:json!({"id":"new","status":"rejected"}),snapshot:json!({"new":"frozen"}),run:"another-run".into(),legacy:false});
        m["attempt"]=json!(3);m["previousAttemptSha256"]=json!(hash(&both[1].snapshot));assert!(next_budget_bound(&m,&both).is_err());
    }
    #[test]
    fn foundation_conflict_history_includes_other_workflow_and_holds_unknown_without_writes() {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("合成冲突预算","fixture").unwrap();store.ensure_workflow_storage().unwrap();
        let r=request();let snap=serde_json::to_value(&r).unwrap();
        let parent=store.add_job("workflow","foundation.run","local","local",&json!({"input":{"settings":{}}}),"running").unwrap();
        let job=store.add_job("text","foundation:F2","codex_sdk","fixture",&snap,"submission_unknown").unwrap();
        store.checkpoint(&parent,&step_key(&r),&r.stage,&hash(&snap),"running",Some(&job),None,None).unwrap();
        let exec=SavedTextExecutor{store,settings:json!({}),workflow_id:Some("another-run".into())};
        let before=exec.store.jobs().unwrap();let history=exec.foundation_conflict_history(&proposal()).unwrap();assert_eq!(history.len(),1);assert_eq!(history[0].run,parent);
        assert!(exec.foundation_conflict_verified_record(&history[0]).is_err());assert_eq!(exec.store.jobs().unwrap(),before);assert!(!exec.store.root.join("sdk-worker").exists());
    }
    #[test]
    fn foundation_conflict_history_counts_legacy_even_when_basis_or_workflow_changed() {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("合成旧重基预算","fixture").unwrap();store.ensure_workflow_storage().unwrap();
        let mut r=request();let p=proposal();r.input.as_object_mut().unwrap().remove(FIELD);r.input.as_object_mut().unwrap().remove("foundationConflictManifest");
        r.input["foundationParallelRebase"]=json!({"version":"foundation-parallel-rebase/v1","attempt":1,"originalInputSha256":p["originalInputSha256"],"originalResultSha256":p["originalResultSha256"],"currentBasisSha256":"f".repeat(64),"conflicts":p["conflicts"]});
        let snap=serde_json::to_value(&r).unwrap();let parent=store.add_job("workflow","foundation.run","local","local",&json!({}),"failed").unwrap();
        let job=store.add_job("text","foundation:F2","codex_sdk","fixture",&snap,"rejected").unwrap();
        store.checkpoint(&parent,&step_key(&r),&r.stage,&hash(&snap),"rejected",Some(&job),Some(&json!({})),Some("旧领域拒绝")).unwrap();
        let exec=SavedTextExecutor{store,settings:json!({}),workflow_id:Some("new-run".into())};let history=exec.foundation_conflict_history(&p).unwrap();
        assert_eq!(history.len(),1);assert!(history[0].legacy);assert_eq!(history[0].job["id"],job);
    }
    #[test]
    fn foundation_conflict_readonly_scope_is_nested_root_bound_and_unwinds() {
        let temp=tempfile::tempdir().unwrap();let a=Store::open(temp.path().join("a")).unwrap();a.create("a","a").unwrap();
        let b=Store::open(temp.path().join("b")).unwrap();b.create("b","b").unwrap();
        Store::with_readonly_connections(&a.root,|| {
            let c=a.conn().unwrap();assert_eq!(c.query_row("PRAGMA query_only",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert!(c.execute("UPDATE project SET title='blocked'",[]).is_err());
            b.conn().unwrap().execute("UPDATE project SET title='allowed'",[]).unwrap();
            Store::with_readonly_connections(&b.root,|| {assert!(b.conn().unwrap().execute("UPDATE project SET title='blocked'",[]).is_err());assert!(a.conn().unwrap().execute("UPDATE project SET title='blocked'",[]).is_err());});
            b.conn().unwrap().execute("UPDATE project SET title='allowed2'",[]).unwrap();
        });
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(||Store::with_readonly_connections(&a.root,||panic!("synthetic unwind")))).is_err());
        a.conn().unwrap().execute("UPDATE project SET title='restored'",[]).unwrap();assert_eq!(a.project().unwrap()["title"],"restored");
    }

    fn legacy_cache_fixture(provider:&str)->(tempfile::TempDir,SavedTextExecutor,TextRequest,String,Value) {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();
        store.create("合成旧渠道重基","fixture").unwrap();store.ensure_workflow_storage().unwrap();
        let settings=if provider=="api" {json!({"textProvider":"api","textEndpoint":"http://127.0.0.1:1/never-called"})}
            else {json!({"textProvider":"codex","textCliExecutable":temp.path().join("never-executed.exe"),"textCliVersion":"codex-cli 0.160.1"})};
        let run=store.add_job("workflow","foundation.run","local","local",&json!({"input":{"settings":settings}}),"running").unwrap();
        let executor=SavedTextExecutor{store,settings,workflow_id:Some(run.clone())};
        let p=proposal();let mut r=request();r.input.as_object_mut().unwrap().remove(FIELD);r.input.as_object_mut().unwrap().remove("foundationConflictManifest");
        r.input["foundationParallelTrial"]["workflowId"]=json!(run);
        r.input["foundationParallelRebase"]=json!({"version":"foundation-parallel-rebase/v1","attempt":1,
            "originalInputSha256":p["originalInputSha256"],"originalResultSha256":p["originalResultSha256"],
            "currentBasisSha256":p["currentBasisSha256"],"conflicts":p["conflicts"]});
        let mut snapshot=executor.request_domain_snapshot(&r).unwrap();
        snapshot["executorIdentity"]=if provider=="api" {json!({"provider":"api","endpoint":executor.settings["textEndpoint"]})}
            else {json!({"provider":"codex","executable":executor.settings["textCliExecutable"]})};
        let raw=json!({"assets":[{"id":"original-kept"}]});
        let job=executor.store.add_job("text",&r.stage,provider,"fixture",&snapshot,"completed").unwrap();
        executor.store.update_job(&job,"completed",&json!({"result":raw,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
        executor.store.event(&job,"text_received_raw",&json!({"requestHash":hash(&snapshot),"value":raw})).unwrap();
        executor.store.checkpoint(&run,&step_key(&r),&r.stage,&hash(&snapshot),"accepted",Some(&job),Some(&raw),None).unwrap();
        (temp,executor,r,job,raw)
    }

    #[test]
    fn foundation_conflict_legacy_cli_and_api_accepted_cache_remains_readonly() {
        for provider in ["codex","api"] {
            let (_temp,executor,r,job,raw)=legacy_cache_fixture(provider);
            let before=executor.store.jobs().unwrap();
            Store::with_readonly_connections(&executor.store.root,|| {
                let context=executor.foundation_conflict_context_saved(&r,&proposal()).unwrap();
                assert_eq!(context["legacyAccepted"]["raw"],raw);
                assert_eq!(context["legacyAccepted"]["jobId"],job);
                assert_eq!(context["consumedAttempts"],1);
                assert!(context["nextAttempt"].is_null());assert!(context["marker"].is_null());
            });
            assert_eq!(executor.store.jobs().unwrap(),before);
        }
    }

    #[test]
    fn foundation_conflict_legacy_cache_does_not_trust_completed_or_changed_request() {
        for mutation in ["receipt","checkpoint","result","request"] {
            let (_temp,executor,mut r,job,_)=legacy_cache_fixture("api");
            match mutation {
                "receipt"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET receipt='{}' WHERE id=?1",[&job]).unwrap();},
                "checkpoint"=>{executor.store.conn().unwrap().execute("UPDATE workflow_steps SET state='submission_unknown' WHERE text_job_id=?1",[&job]).unwrap();},
                "result"=>{executor.store.conn().unwrap().execute("UPDATE jobs SET result='{}' WHERE id=?1",[&job]).unwrap();},
                _=>{r.model="different-model".into();},
            }
            let before=executor.store.jobs().unwrap();
            assert!(executor.foundation_conflict_context_saved(&r,&proposal()).is_err(),"{mutation}");
            assert_eq!(executor.store.jobs().unwrap(),before);
        }
    }

    #[test]
    fn foundation_conflict_cross_provider_history_never_resets_budget_or_unknown() {
        let (_temp,executor,r,_,_)=legacy_cache_fixture("api");
        let mut other=serde_json::to_value(&r).unwrap();other["model"]=json!("other");
        let run=executor.store.add_job("workflow","foundation.run","local","local",&json!({}),"failed").unwrap();
        let job=executor.store.add_job("text",&r.stage,"codex","fixture",&other,"rejected").unwrap();
        executor.store.checkpoint(&run,&step_key(&r),&r.stage,&hash(&other),"rejected",Some(&job),Some(&json!({})),Some("old-domain-rejection")).unwrap();
        let context=executor.foundation_conflict_context_saved(&r,&proposal()).unwrap();
        assert_eq!(context["consumedAttempts"],2);assert!(context["nextAttempt"].is_null());
        assert_eq!(context["previousAttempts"][1]["provider"],"codex");
        executor.store.update_job(&job,"submission_unknown",&json!({})).unwrap();
        let before=executor.store.jobs().unwrap();
        assert!(executor.foundation_conflict_context_saved(&r,&proposal()).unwrap_err().contains("FOUNDATION_CONFLICT_UNKNOWN"));
        assert_eq!(executor.store.jobs().unwrap(),before);
    }

    #[test]
    #[ignore="explicit captured root/run/legacy/manifest required; forced READ_ONLY SQL, no generation or adoption"]
    fn foundation_conflict_captured_live_context_is_readonly_and_zero_dispatch() {
        let required=|name|std::env::var(name).expect("explicit captured conflict proof input required");
        let root=PathBuf::from(required("DIRECTOR_FOUNDATION_CONFLICT_PROOF_ROOT"));
        let run=required("DIRECTOR_FOUNDATION_CONFLICT_PROOF_RUN");
        let legacy:TextRequest=serde_json::from_slice(&std::fs::read(required("DIRECTOR_FOUNDATION_CONFLICT_PROOF_LEGACY")).unwrap()).unwrap();
        let proposal:Value=serde_json::from_slice(&std::fs::read(required("DIRECTOR_FOUNDATION_CONFLICT_PROOF_MANIFEST")).unwrap()).unwrap();
        let hashes=||["project.sqlite","project.sqlite-wal"].into_iter().map(|name|{let path=root.join(name);(name,if path.exists(){Some(crate::store::hash_file(&path).unwrap())}else{None})}).collect::<Vec<_>>();
        let before=hashes();
        Store::with_readonly_connections(&root,|| {
            let store=Store{root:root.clone()};
            let parent=store.job(&run).unwrap();
            let executor=SavedTextExecutor{store,settings:parent["snapshot"]["input"]["settings"].clone(),workflow_id:Some(run)};
            let events:i64=executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events",[],|r|r.get(0)).unwrap();
            let jobs:i64=executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM jobs",[],|r|r.get(0)).unwrap();
            let context=executor.foundation_conflict_context_saved(&legacy,&proposal).unwrap();
            assert!(context["legacyAccepted"].is_null());assert!(context["existingResolution"].is_null());
            assert_eq!(context["consumedAttempts"],1);assert_eq!(context["nextAttempt"],2);
            assert_eq!(context["marker"]["legacyConsumedAttempts"],1);
            assert_eq!(executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events",[],|r|r.get::<_,i64>(0)).unwrap(),events);
            assert_eq!(executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM jobs",[],|r|r.get::<_,i64>(0)).unwrap(),jobs);
        });
        let after=hashes();
        assert_eq!(after[0],before[0],"read-only context must preserve database bytes");
        // SQLite READ_ONLY can create/remove an empty WAL while opening a WAL
        // database. It carries no pages. Never normalize a non-empty WAL or a
        // database change; report this sidecar lifecycle separately.
        let empty_wal=|entry:&Option<(String,u64)>|entry.as_ref().is_none_or(|(_,bytes)|*bytes==0);
        let empty_wal_lifecycle_changed=after[1]!=before[1] && empty_wal(&after[1].1) && empty_wal(&before[1].1);
        assert!(after[1]==before[1] || empty_wal_lifecycle_changed,"read-only context must preserve all WAL page bytes");
        eprintln!("READONLY_CONFLICT_CONTEXT_PASS consumedAttempts=1 nextAttempt=2 original_sdk_handoff_and_worker_path_verified=true generationRequests=0 emptyWalLifecycleChanged={empty_wal_lifecycle_changed}");
    }
}
