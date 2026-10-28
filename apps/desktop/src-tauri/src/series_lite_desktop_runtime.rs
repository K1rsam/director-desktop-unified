//! Thin three-role controller over the application's only saved text executor.
//! No independent runner or provider implementation is introduced here.
use crate::pipeline::{digest, TextExecutor, TextRequest};
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_controller_store::{self as ledger, FrozenRole, RoleRecord};
use crate::series_lite_edit::{compose_derived_bytes, sha256};
use crate::series_lite_job_packet::{self as packets, PreparationOptions};
use crate::series_lite_transport::{self as transport, Role};
use crate::series_lite_workspace::NativeWorkspaceScopeVerifier;
use crate::store::Store;
use crate::workflow_artifacts::{self as artifacts, Inputs};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path};
type Result<T> = std::result::Result<T, String>;

pub(crate) struct ChainResult {
    pub candidate: Vec<u8>,
    pub packet_path: String,
    pub reviewer: RoleRecord,
}
/// Idempotent local creation only. Existing bytes are read back, never replaced.
pub(crate) fn put_exact(root:&Path,path:&str,bytes:&[u8])->Result<()> {
    if Path::new(path).components().any(|c|!matches!(c,std::path::Component::Normal(_))) {
        return Err("LITE_LOCAL_PATH_INVALID".into());
    }
    let root=root.canonicalize().map_err(|e|e.to_string())?;
    let target=root.join(path);
    if let Some(parent)=target.parent() {
        // Validate every existing ancestor before creating any child. In
        // particular create_dir_all must not traverse a Windows junction first.
        let mut at=root.clone();
        for component in parent.strip_prefix(&root).map_err(|e|e.to_string())?.components() {
            at.push(component);
            match fs::create_dir(&at) {Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e.to_string())}
            let meta=fs::symlink_metadata(&at).map_err(|e|e.to_string())?;
            #[cfg(windows)] let reparse={use std::os::windows::fs::MetadataExt;meta.file_attributes()&0x400!=0};
            #[cfg(not(windows))] let reparse=false;
            if !meta.is_dir()||meta.file_type().is_symlink()||reparse||!at.canonicalize().map_err(|e|e.to_string())?.starts_with(&root) {return Err("LITE_LOCAL_PATH_ESCAPE".into());}
        }
    }
    match fs::OpenOptions::new().write(true).create_new(true).open(&target) {
        Ok(mut file)=>file.write_all(bytes).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?,
        Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>{
            let metadata=fs::symlink_metadata(&target).map_err(|e|e.to_string())?;
            if !metadata.is_file()||metadata.file_type().is_symlink()||fs::read(&target).map_err(|e|e.to_string())?!=bytes {return Err("LITE_LOCAL_FROZEN_BYTES_CHANGED".into());}
        },
        Err(e)=>return Err(e.to_string()),
    }
    Ok(())
}
fn candidate(store:&Store,record:&RoleRecord)->Result<Vec<u8>> {
    let pin=record.result.as_ref().and_then(|r|r.candidate.as_ref()).ok_or("LITE_CANDIDATE_MISSING")?;
    let item=artifacts::artifact(&store.conn()?,&pin.artifact_id)?.ok_or("LITE_CANDIDATE_MISSING")?;
    let text=item.artifact.payload["text"].as_str().ok_or("LITE_CANDIDATE_TEXT_MISSING")?;
    if item.payload_sha256!=pin.payload_sha256||sha256(text.as_bytes())!=pin.native_sha256 {return Err("LITE_CANDIDATE_DRIFT".into());}
    Ok(text.as_bytes().to_vec())
}
fn packet_path(record:&RoleRecord)->Result<String> {
    let raw=record.frozen.request.input["packetUtf8"].as_str().ok_or("LITE_PACKET_MISSING")?;
    let p:Value=serde_json::from_str(raw).map_err(|e|e.to_string())?;
    let request=Path::new(p["requestOrigin"]["path"].as_str().ok_or("LITE_PACKET_ORIGIN_MISSING")?);
    Ok(request.parent().ok_or("LITE_PACKET_ORIGIN_INVALID")?.join("packet.json").to_string_lossy().replace('\\',"/"))
}
pub(crate) async fn execute_chain(
    store:&Store,run:&str,root:&Path,base:&[u8],supply:&[u8],guard:&Value,
    common:Inputs,options:&PreparationOptions,executor:&dyn TextExecutor,
    model:&str,reasoning:&str,
)->Result<ChainResult> {
    if crate::series_lite_desktop_recovery::enabled(store,run)? {
        return crate::series_lite_desktop_recovery::execute_chain(store,run,root,base,supply,guard,common,options,executor,model,reasoning).await;
    }
    let pack=VerifiedPack::current_embedded()?;
    let parsed=crate::series_lite_edit::parse(base)?;
    let episode=String::from_utf8(crate::series_lite_edit::canonical(parsed.get("episodes")?.array()?[0].get("number")?)).map_err(|e|e.to_string())?;
    let mut current=base.to_vec();
    let mut previous_pin=None;
    let mut final_packet=String::new();
    let mut reviewer=None;
    {
        let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
        ledger::install(&tx)?;tx.commit().map_err(|e|e.to_string())?;
    }
    for role in [Role::Author,Role::Finisher,Role::Reviewer] {
        // Read the persisted slot FIRST. A reopened application consumes the
        // original request/base/context, including received or UNKNOWN nodes.
        let existing=ledger::read_role(&store.conn()?,run,&episode,role)?;
        let slot=if let Some(record)=existing {
            if record.frozen.base_utf8.as_bytes()!=current||record.frozen.common_inputs!=common {return Err("LITE_ROLE_INPUT_DRIFT: 原槽保留，不重签或重发".into());}
            record
        } else {
            let base_path=format!("inputs/{}.json",sha256(&current));
            put_exact(root,&base_path,&current)?;
            let output=format!("roles/{}-{}",role.as_str(),crate::store::id());
            let prepared=packets::prepare(root,&base_path,"registry",role.as_str(),&output,options,&pack,&NativeWorkspaceScopeVerifier)?;
            let compiled=packets::verify(root,&prepared.packet,&pack,&NativeWorkspaceScopeVerifier)?;
            let context=ledger::context_for(&store.conn()?,run,&episode,role)?;
            let carrier=transport::freeze_verified(&compiled,&context,&pack)?;
            let mut input=carrier.input;
            input["textExact"]=json!({"version":"series-lite-exact-text/v1","baseSha256":sha256(&current)});
            let schema=match role {
                Role::Author=>crate::pipeline::series_lite_schema::author(&current)?,
                Role::Finisher=>crate::pipeline::series_lite_schema::finisher(&current)?,
                Role::Reviewer=>crate::pipeline::series_lite_schema::reviewer(&current)?,
            };
            let request=TextRequest{stage:role.stage().into(),skill:format!("series-lite-{}",role.as_str()),model:model.into(),reasoning:reasoning.into(),input,schema,
                rule_snapshot:json!({"contract":pack.contract_identity(),"role":role.as_str(),"responseRule":crate::pipeline::series_lite_schema::response_rule(role.as_str())?})};
            let frozen=FrozenRole{version:ledger::VERSION.into(),workflow_id:run.into(),source_sha256:common.get("source").cloned().ok_or("LITE_SOURCE_PIN_MISSING")?,rule_snapshot_sha256:digest(&request.rule_snapshot.to_string()),request,identity:carrier.identity,context,base_utf8:String::from_utf8(current.clone()).map_err(|e|e.to_string())?,base_sha256:sha256(&current),common_inputs:common.clone(),previous_candidate:previous_pin.clone()};
            let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
            crate::series_lite_guard::verify(&tx,guard)?;
            let saved=ledger::reserve(&tx,&frozen)?;tx.commit().map_err(|e|e.to_string())?;saved
        };
        store.event(run,"lite_activity",&json!({"episode":episode,"role":role.as_str(),"action":if slot.result.is_some(){"checking_original_result"}else{"executing_role"},"slotId":slot.slot_id}))?;
        let frozen=&slot.frozen;
        let raw=executor.execute_exact(frozen.request.clone(),&frozen.identity,&frozen.context).await?;
        let next=if role==Role::Reviewer {None} else if slot.result.is_some() {Some(candidate(store,&slot)?)} else {
            let edited=match role {
                Role::Author=>compose_derived_bytes(&current,&sha256(&current),raw.bytes())?.edit.bytes,
                Role::Finisher=>crate::series_lite_edit::patch_derived_bytes(&current,raw.bytes())?.edit.bytes,
                _=>unreachable!(),
            };
            let projected=crate::series_lite_scene_projection::project(&edited,&sha256(&edited),supply,&sha256(supply))?;
            Some(projected.bytes)
        };
        let record={
            let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
            let v=ledger::record_candidate(&tx,&slot.slot_id,&raw,next.as_deref())?;
            tx.commit().map_err(|e|e.to_string())?;v
        };
        // Transport receipt remains the original native-response envelope. It
        // is never replaced by a parsed/composed draft or by a model PASS.
        executor.accept(&frozen.request,raw.transport_envelope()).await?;
        final_packet=packet_path(&record)?;
        if let Some(next)=next {current=next;previous_pin=record.result.as_ref().and_then(|r|r.candidate.clone());}
        if role==Role::Reviewer {reviewer=Some(record);}
    }
    Ok(ChainResult{candidate:current,packet_path:final_packet,reviewer:reviewer.ok_or("LITE_REVIEW_SLOT_MISSING")?})
}
