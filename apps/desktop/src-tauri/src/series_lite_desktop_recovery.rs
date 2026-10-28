//! Bounded controller for new lite tasks. Raw responses and rejected jobs stay
//! immutable. Repairs are additional, budgeted nodes over the existing executor.
use crate::pipeline::{digest, TextExecutor, TextRequest};
use crate::series_lite_bundle::BundleGate;
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_controller_store::{CandidatePin, FrozenRole, RoleRecord};
use crate::series_lite_controller_v2::{self as ledger, RevisionBinding, Slot};
use crate::series_lite_desktop_runtime::{put_exact, ChainResult};
use crate::series_lite_edit::{compose_derived_bytes, parse, patch_derived_bytes, sha256};
use crate::series_lite_job_packet::{self as packets, PreparationOptions};
use crate::series_lite_review::{self as review, BoundaryInput};
use crate::series_lite_transport::{self as transport, Role};
use crate::series_lite_workspace::NativeWorkspaceScopeVerifier;
use crate::store::{s, Store};
use crate::workflow_artifacts::{self as artifacts, Inputs};
use crate::workflow_issues as issues;
use serde_json::{json, Value};
use std::{fs, path::Path};
type Result<T> = std::result::Result<T, String>;

struct Context<'a> {
    store: &'a Store, run: &'a str, root: &'a Path, episode: String,
    supply: &'a [u8], guard: &'a Value, common: Inputs,
    options: &'a PreparationOptions, executor: &'a dyn TextExecutor,
    model: &'a str, reasoning: &'a str, boundary: Value,
}
pub(crate) fn enabled(store: &Store, run: &str) -> Result<bool> {
    let raw: String = store.conn()?.query_row("SELECT snapshot FROM jobs WHERE id=?1", [run], |r|r.get(0)).map_err(|e|e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e|e.to_string())?;
    Ok(v["input"]["liteRuntimeContract"] == ledger::VERSION)
}
fn candidate(store:&Store, r:&RoleRecord)->Result<Option<Vec<u8>>> {
    let Some(pin)=r.result.as_ref().and_then(|r|r.candidate.as_ref()) else {return Ok(None)};
    let a=artifacts::artifact(&store.conn()?,&pin.artifact_id)?.ok_or("LITE_REPAIR_CANDIDATE_MISSING")?;
    let raw=s(&a.artifact.payload,"text").as_bytes().to_vec();
    if a.payload_sha256!=pin.payload_sha256 || sha256(&raw)!=pin.native_sha256 {return Err("LITE_REPAIR_CANDIDATE_DRIFT".into())}
    Ok(Some(raw))
}
fn raw_response(store:&Store,r:&RoleRecord)->Result<Vec<u8>> {
    let result=r.result.as_ref().ok_or("LITE_REPAIR_RAW_MISSING")?;
    let a=artifacts::artifact(&store.conn()?,&result.raw_artifact_id)?.ok_or("LITE_REPAIR_RAW_MISSING")?;
    let raw=s(&a.artifact.payload,"text").as_bytes().to_vec();
    if a.payload_sha256!=result.raw_artifact_payload_sha256 || sha256(&raw)!=result.native_response_sha256 {return Err("LITE_REPAIR_RAW_DRIFT".into())}
    Ok(raw)
}
fn pin(r:&RoleRecord)->Option<CandidatePin> {r.result.as_ref().and_then(|r|r.candidate.clone())}
fn packet_path(r:&RoleRecord)->Result<String> {
    let p:Value=serde_json::from_str(s(&r.frozen.request.input,"packetUtf8")).map_err(|e|e.to_string())?;
    Ok(Path::new(s(&p["requestOrigin"],"path")).parent().ok_or("LITE_PACKET_ORIGIN_MISSING")?.join("packet.json").to_string_lossy().replace('\\',"/"))
}
fn boundary_record(store:&Store,run:&str,number:&str)->Result<Value> {
    let raw:String=store.conn()?.query_row("SELECT snapshot FROM jobs WHERE id=?1",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
    let parent:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
    let native=number.parse::<u32>().map_err(|_|"LITE_EPISODE_NUMBER")?;
    let ep=parent["input"]["source"]["episodes"].as_array().ok_or("LITE_SOURCE_EPISODES")?.iter().find(|e|crate::pipeline::number_value(s(e,"sourceNumber"))==Some(native)).ok_or("LITE_SOURCE_EPISODE")?;
    crate::series_lite_desktop_prepare::frozen_boundary(&store.conn()?,run,s(ep,"id"))?.filter(|v|!v.is_null()).or_else(||parent["input"]["liteBoundaryByEpisode"].get(number).cloned()).ok_or("LITE_BOUNDARY_NOT_FROZEN".into())
}
fn available_boundary(ctx:&Context<'_>)->Result<(String,Value)> {
    let raw:String=ctx.store.conn()?.query_row("SELECT snapshot FROM jobs WHERE id=?1",[ctx.run],|r|r.get(0)).map_err(|e|e.to_string())?;
    let parent:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
    let native=ctx.episode.parse::<u32>().map_err(|_|"LITE_EPISODE_NUMBER")?;
    let ep=parent["input"]["source"]["episodes"].as_array().ok_or("LITE_SOURCE_EPISODES")?.iter().find(|e|crate::pipeline::number_value(s(e,"sourceNumber"))==Some(native)).ok_or("LITE_SOURCE_EPISODE")?;
    let id=s(ep,"id").to_owned();
    Ok((id.clone(),crate::pipeline::series_lite::current_boundary(ctx.store,&parent["input"],&id)?))
}
fn checked_review(ctx:&Context<'_>,raw:&[u8],base:&[u8],prior:&[review::Issue])->Result<review::Checked> {
    let boundary=match s(&ctx.boundary,"kind") {
        "firstInScope"=>BoundaryInput::FirstInScope,
        "missingPrevious"=>BoundaryInput::MissingPrevious,
        "supplied"=>BoundaryInput::Supplied{previous_bytes:s(&ctx.boundary,"previousBundleUtf8").as_bytes(),previous_sha256:s(&ctx.boundary,"nativeSha256")},
        _=>return Err("LITE_BOUNDARY_UNSUPPORTED".into()),
    };
    review::validate(raw,base,&sha256(base),prior,boundary)
}
fn mechanical(ctx:&Context<'_>,raw:&[u8])->Result<()> {
    let head:Value=serde_json::from_slice(&fs::read(ctx.root.join("registry/current.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let revision=head["revision"].as_u64().ok_or("LITE_REGISTRY_REVISION")?;
    let bytes=fs::read(ctx.root.join(format!("registry/v{revision:06}.json"))).map_err(|e|e.to_string())?;
    if sha256(&bytes)!=head["sha256"] {return Err("LITE_REGISTRY_DRIFT".into())}
    let registry:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
    let lock=fs::read(ctx.root.join("source-lock.json")).map_err(|e|e.to_string())?;
    crate::series_lite_bundle_gate::NativeLegacyBundleGate{registry:Some(&registry),source_lock:Some(&lock),season_scope:None}.check(raw).map(|_|())
}
/// A failed semantic candidate may remain patchable. It never receives an
/// adoption merely because the original transport returned valid UTF-8.
fn apply(ctx:&Context<'_>,role:Role,base:&[u8],raw:&[u8],prior:&[review::Issue])->(Option<Vec<u8>>,Option<String>) {
    if role==Role::Reviewer {return (None,checked_review(ctx,raw,base,prior).err())}
    let edited=match role {Role::Author=>compose_derived_bytes(base,&sha256(base),raw).map(|v|v.edit.bytes),_=>patch_derived_bytes(base,raw).map(|v|v.edit.bytes)};
    let edited=match edited {Ok(v)=>v,Err(e)=>return (None,Some(e))};
    let projected=crate::series_lite_scene_projection::project(&edited,&sha256(&edited),ctx.supply,&sha256(ctx.supply));
    let projected=match projected {Ok(v)=>v.bytes,Err(e)=>return (Some(edited),Some(e))};
    let invalid=mechanical(ctx,&projected).err();
    (Some(projected),invalid)
}
struct Revision { issue:issues::Issue, ordinal:u64, basis_job:String }
fn issue(ctx:&Context<'_>,origin:&[u8],slot:&Slot,phase:&str,detail:&Value)->Result<issues::Issue> {
    let r=slot.record.result.as_ref().ok_or("LITE_ISSUE_RAW_MISSING")?;
    Ok(issues::Issue{version:issues::VERSION.into(),source_sha256:ctx.common["source"].clone(),origin_sha256:sha256(origin),stage:format!("series-lite:{phase}"),code:if phase=="content"{"LITE_CONTENT_REVISION"}else{"LITE_OUTPUT_REVISION"}.into(),object:ctx.episode.clone(),location:format!("/episodes/{}/{}",ctx.episode,phase),evidence:vec![issues::Evidence{kind:"original-response".into(),reference:r.raw_artifact_id.clone(),sha256:r.raw_artifact_payload_sha256.clone()},issues::Evidence{kind:"finding".into(),reference:slot.record.slot_id.clone(),sha256:digest(&detail.to_string())}],impact:vec![ctx.episode.clone()],repair:issues::Repair::AiRevision,message:"本集进入有界局部修订，其它集可继续".into()})
}
fn finish_revision(ctx:&Context<'_>,revision:&Revision,result:&[u8],accepted:bool,detail:&Value)->Result<()> {
    let cause=revision.issue.cause()?;
    let mut c=ctx.store.conn()?;let mut tx=c.transaction().map_err(|e|e.to_string())?;
    let ledger=issues::read(&tx,&cause)?.ok_or("LITE_REPAIR_BUDGET_MISSING")?;
    let outcome=issues::Outcome{accepted,result_sha256:sha256(result),progress_sha256:sha256(result),code:if accepted{"LITE_REVISION_RESOLVED"}else{"LITE_REVISION_UNRESOLVED"}.into(),finding_sha256:digest(&detail.to_string())};
    issues::finish(&mut tx,&cause,revision.ordinal,ledger.revision,&outcome,None)?;
    tx.commit().map_err(|e|e.to_string())
}
async fn step(ctx:&Context<'_>,cursor:&mut usize,role:Role,phase:&str,base:&[u8],previous:Option<CandidatePin>,prior:&[review::Issue],revision:Option<&Revision>,note:&Value)->Result<Slot> {
    let history=ledger::history(&ctx.store.conn()?,ctx.run,&ctx.episode)?;
    let existing=history.get(*cursor);
    let slot=if let Some(slot)=existing {
        if slot.phase!=phase || slot.record.frozen.context.role!=role || slot.record.frozen.base_utf8.as_bytes()!=base || slot.record.frozen.common_inputs!=ctx.common || slot.record.frozen.previous_candidate!=previous
            || serde_json::to_value(&slot.prior_issues).map_err(|e|e.to_string())?!=serde_json::to_value(prior).map_err(|e|e.to_string())?
            || slot.revision.as_ref().map(|r|(&r.cause,r.ordinal))!=revision.map(|r|r.issue.cause().map(|c|(c,r.ordinal))).transpose()?.as_ref().map(|(c,o)|(c,*o)) {
            return Err("LITE_RECOVERY_SLOT_INPUT_DRIFT".into())
        }
        // Move the owned record out without relying on Clone for RoleRecord.
        ledger::history(&ctx.store.conn()?,ctx.run,&ctx.episode)?.into_iter().nth(*cursor).ok_or("LITE_RECOVERY_SLOT_MISSING")?
    }else{
        if history.len()!=*cursor {return Err("LITE_RECOVERY_SEQUENCE_DRIFT".into())}
        let context=ledger::context_for(&ctx.store.conn()?,ctx.run,&ctx.episode,role)?;
        let base_path=format!("inputs/{}.json",sha256(base));put_exact(ctx.root,&base_path,base)?;
        let output=format!("roles/v2-{:02}-{}",*cursor+1,role.as_str());
        let mut options=ctx.options.clone();
        if !prior.is_empty() || !note.is_null() {
            let path=format!("inputs/findings-{}.json",digest(&json!({"priorIssues":prior,"finding":note}).to_string()));
            put_exact(ctx.root,&path,&serde_json::to_vec(&json!({"priorIssues":prior,"finding":note,"scope":"只修具体问题，保留源与受保护设置，不全文重交"})).map_err(|e|e.to_string())?)?;
            options.issues=Some((path,None));
        }
        let pack=VerifiedPack::current_embedded()?;
        let prepared=packets::prepare(ctx.root,&base_path,"registry",role.as_str(),&output,&options,&pack,&NativeWorkspaceScopeVerifier)?;
        let compiled=packets::verify(ctx.root,&prepared.packet,&pack,&NativeWorkspaceScopeVerifier)?;
        let carrier=transport::freeze_verified(&compiled,&context,&pack)?;
        let mut input=carrier.input;input["textExact"]=json!({"version":"series-lite-exact-text/v1","baseSha256":sha256(base)});
        let schema=match role {Role::Author=>crate::pipeline::series_lite_schema::author(base)?,Role::Finisher=>crate::pipeline::series_lite_schema::finisher(base)?,Role::Reviewer=>crate::pipeline::series_lite_schema::reviewer(base)?};
        let request=TextRequest{stage:role.stage().into(),skill:format!("series-lite-{}",role.as_str()),model:ctx.model.into(),reasoning:ctx.reasoning.into(),input,schema,rule_snapshot:json!({"contract":pack.contract_identity(),"role":role.as_str(),"responseRule":crate::pipeline::series_lite_schema::response_rule(role.as_str())?,"recoveryContract":ledger::VERSION})};
        let frozen=FrozenRole{version:crate::series_lite_controller_store::VERSION.into(),workflow_id:ctx.run.into(),source_sha256:ctx.common["source"].clone(),rule_snapshot_sha256:digest(&request.rule_snapshot.to_string()),request,identity:carrier.identity,context,base_utf8:String::from_utf8(base.to_vec()).map_err(|e|e.to_string())?,base_sha256:sha256(base),common_inputs:ctx.common.clone(),previous_candidate:previous};
        let mut c=ctx.store.conn()?;let mut tx=c.transaction().map_err(|e|e.to_string())?;
        crate::series_lite_guard::verify(&tx,ctx.guard)?;
        let binding=if let Some(revision)=revision {
            issues::install(&tx)?;let registered=issues::register(&mut tx,&revision.issue)?;
            let cause=registered.cause.clone();let key=format!("lite-revision:{cause}:{}",revision.ordinal);
            // This is the recorded account of the evidence being repaired. The
            // new node's actual account remains in its native dispatch receipt.
            let (provider,account):(String,String)=tx.query_row("SELECT provider,account_id FROM jobs WHERE id=?1",[&revision.basis_job],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
            let reserved=issues::reserve(&mut tx,&cause,registered.revision,&issues::Attempt{attempt_key:key.clone(),request_sha256:digest(&serde_json::to_string(&frozen.request).map_err(|e|e.to_string())?),base_sha256:sha256(base),workflow_id:ctx.run.into(),provider,account,repair:issues::Repair::AiRevision,action:ledger::VERSION.into()})?;
            if reserved.ordinal!=revision.ordinal {return Err("LITE_REPAIR_BUDGET_ORDINAL".into())}
            Some(RevisionBinding{cause,ordinal:reserved.ordinal,attempt_key:key})
        }else{None};
        let slot=ledger::reserve(&tx,&frozen,phase,prior,binding)?;tx.commit().map_err(|e|e.to_string())?;slot
    };
    *cursor+=1;
    if slot.record.result.is_some() && matches!(slot.record.transport_state.as_deref(),Some("completed"|"rejected")) {return Ok(slot)}
    ctx.store.event(ctx.run,"lite_activity",&json!({"episode":ctx.episode,"role":role.as_str(),"action":if revision.is_some(){"local_revision"}else{"executing_role"},"slotId":slot.record.slot_id,"phase":phase}))?;
    // UNKNOWN/network failures leave this same slot unresolved; the next
    // invocation observes/reconciles that node, never allocates a repair.
    let raw=ctx.executor.execute_exact(slot.record.frozen.request.clone(),&slot.record.frozen.identity,&slot.record.frozen.context).await?;
    let (next,invalid)=apply(ctx,role,base,raw.bytes(),prior);
    let saved={let mut c=ctx.store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;let r=ledger::record_candidate(&tx,&slot.record.slot_id,&raw,next.as_deref(),invalid.as_deref())?;tx.commit().map_err(|e|e.to_string())?;r};
    if let Some(error)=invalid {ctx.executor.reject(&saved.record.frozen.request,raw.transport_envelope(),&error).await?}else{ctx.executor.accept(&saved.record.frozen.request,raw.transport_envelope()).await?}
    ledger::history(&ctx.store.conn()?,ctx.run,&ctx.episode)?.into_iter().find(|s|s.record.slot_id==saved.record.slot_id).ok_or("LITE_RECOVERY_RESULT_MISSING".into())
}
async fn valid_step(ctx:&Context<'_>,cursor:&mut usize,role:Role,phase:&str,base:&[u8],previous:Option<CandidatePin>,prior:&[review::Issue])->Result<Slot> {
    let first=step(ctx,cursor,role,phase,base,previous.clone(),prior,None,&Value::Null).await?;
    if first.invalid.is_none(){return Ok(first)}
    let mut current=candidate(ctx.store,&first.record)?.unwrap_or_else(||base.to_vec());
    let origin=current.clone();
    let detail=json!({"code":"LITE_OUTPUT_INVALID","detail":first.invalid});
    let issue=issue(ctx,&origin,&first,phase,&detail)?;
    let basis=first.record.result.as_ref().ok_or("LITE_REPAIR_RAW_MISSING")?.job_id.clone();
    let mut prior_pin=pin(&first.record).or(previous);
    let mut last_error=first.invalid.clone();
    for ordinal in 1..=2 {
        let revision=Revision{issue:issue.clone(),ordinal,basis_job:basis.clone()};
        let repair_role=if role==Role::Reviewer{Role::Reviewer}else if prior_pin.is_some(){Role::Finisher}else{Role::Author};
        let repaired=step(ctx,cursor,repair_role,&format!("{phase}:format"),&current,prior_pin.clone(),prior,Some(&revision),&json!({"code":"LITE_OUTPUT_INVALID","detail":last_error})).await?;
        let next=candidate(ctx.store,&repaired.record)?.unwrap_or_else(||current.clone());
        finish_revision(ctx,&revision,&next,repaired.invalid.is_none(),&json!({"code":"LITE_OUTPUT_INVALID","detail":repaired.invalid}))?;
        if repaired.invalid.is_none(){return Ok(repaired)}
        if next==current && repaired.invalid==last_error {return Err("LITE_REVISION_NO_PROGRESS: 同因且基稿未变，已保留本轮结果，不再重复修订".into())}
        last_error=repaired.invalid.clone();current=next;prior_pin=pin(&repaired.record).or(prior_pin);
    }
    Err(format!("LITE_REVISION_EXHAUSTED: 本集两轮局部修订后仍有问题，候选保留：{}",last_error.unwrap_or_default()))
}
pub(crate) async fn execute_chain(store:&Store,run:&str,root:&Path,base:&[u8],supply:&[u8],guard:&Value,common:Inputs,options:&PreparationOptions,executor:&dyn TextExecutor,model:&str,reasoning:&str)->Result<ChainResult> {
    let parsed=parse(base)?;
    let episode=String::from_utf8(crate::series_lite_edit::canonical(parsed.get("episodes")?.array()?[0].get("number")?)).map_err(|e|e.to_string())?;
    let ctx=Context{store,run,root,episode:episode.clone(),supply,guard,common,options,executor,model,reasoning,boundary:boundary_record(store,run,&episode)?};
    {let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;ledger::install(&tx)?;tx.commit().map_err(|e|e.to_string())?;}
    let mut cursor=0;
    let author=valid_step(&ctx,&mut cursor,Role::Author,"author",base,None,&[]).await?;
    let authored=candidate(store,&author.record)?.ok_or("LITE_AUTHOR_CANDIDATE_MISSING")?;
    let finisher=valid_step(&ctx,&mut cursor,Role::Finisher,"finisher",&authored,pin(&author.record),&[]).await?;
    let mut current=candidate(store,&finisher.record)?.ok_or("LITE_FINISHER_CANDIDATE_MISSING")?;
    let mut current_pin=pin(&finisher.record);
    let mut reviewer=valid_step(&ctx,&mut cursor,Role::Reviewer,"reviewer",&current,current_pin.clone(),&[]).await?;
    let mut checked=checked_review(&ctx,&raw_response(store,&reviewer.record)?,&current,&[])?;
    // A predecessor may have become available after this candidate was made.
    // Add one independently frozen boundary read, without rewriting the draft,
    // original preparation or any of the three already-received responses.
    let mut late_options=options.clone();
    let mut effective_boundary=ctx.boundary.clone();
    if checked.boundary_pending {
        let (desktop_episode,available)=available_boundary(&ctx)?;
        let transition=crate::series_lite_late_boundary::load(&store.conn()?,run,&desktop_episode)?;
        if available["kind"]=="supplied" || transition.is_some() {
            let frozen=crate::series_lite_late_boundary::freeze(store,run,&desktop_episode,root,&ctx.boundary,transition.as_ref().map(|t|&t["boundary"]).unwrap_or(&available),&ctx.common["source"])?;
            effective_boundary=frozen["boundary"].clone();
            late_options.context=Some(("late-previous-boundary.json".into(),None));
        }
    }
    let ctx=Context{store,run,root,episode,supply,guard,common:ctx.common.clone(),options:&late_options,executor,model,reasoning,boundary:effective_boundary};
    if checked.boundary_pending && ctx.boundary["kind"]=="supplied" {
        let prior=checked.report.issues.clone();
        reviewer=valid_step(&ctx,&mut cursor,Role::Reviewer,"boundary:review",&current,current_pin.clone(),&prior).await?;
        checked=checked_review(&ctx,&raw_response(store,&reviewer.record)?,&current,&prior)?;
    }
    if !checked.declared_content_pass {
        let root_issue=issue(&ctx,&current,&reviewer,"content",&serde_json::to_value(&checked.report).map_err(|e|e.to_string())?)?;
        let basis=reviewer.record.result.as_ref().ok_or("LITE_REVIEW_RAW_MISSING")?.job_id.clone();
        for ordinal in 1..=2 {
            let prior=checked.report.issues.clone();
            let revision=Revision{issue:root_issue.clone(),ordinal,basis_job:basis.clone()};
            let repaired=step(&ctx,&mut cursor,Role::Finisher,"content:repair",&current,current_pin.clone(),&prior,Some(&revision),&serde_json::to_value(&checked.report).map_err(|e|e.to_string())?).await?;
            let next=candidate(store,&repaired.record)?.unwrap_or_else(||current.clone());
            current_pin=pin(&repaired.record).or(current_pin);current=next;
            if let Some(error)=repaired.invalid.as_ref() {
                finish_revision(&ctx,&revision,&current,false,&json!({"code":"LITE_CONTENT_PATCH_INVALID","detail":error}))?;
                continue;
            }
            let previous_findings=json!({"issues":checked.report.issues,"unresolved":checked.report.unresolved});
            let unchanged=current==repaired.record.frozen.base_utf8.as_bytes();
            reviewer=valid_step(&ctx,&mut cursor,Role::Reviewer,"content:review",&current,current_pin.clone(),&prior).await?;
            checked=checked_review(&ctx,&raw_response(store,&reviewer.record)?,&current,&prior)?;
            finish_revision(&ctx,&revision,&current,checked.declared_content_pass,&serde_json::to_value(&checked.report).map_err(|e|e.to_string())?)?;
            if checked.declared_content_pass{break}
            if unchanged && previous_findings==json!({"issues":checked.report.issues,"unresolved":checked.report.unresolved}) {
                return Err("LITE_CONTENT_NO_PROGRESS: 文字和具体问题均未改变，本集候选保留，不再重复调用".into())
            }
        }
    }
    if !checked.declared_content_pass {return Err("LITE_CONTENT_REVISION_HELD: 两轮后仍有内容问题，原稿、候选与审核均已保留，其它集继续".into())}
    Ok(ChainResult{candidate:current,packet_path:packet_path(&reviewer.record)?,reviewer:reviewer.record})
}

#[cfg(test)]
#[path="series_lite_desktop_recovery_tests.rs"]
mod tests;
