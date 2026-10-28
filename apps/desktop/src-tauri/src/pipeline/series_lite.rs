//! Versioned desktop orchestration. Uses the existing saved executor and lease;
//! an episode failure is isolated, while a received/UNKNOWN request stays frozen.
use super::{digest, RuleSnapshot, TextExecutor};
use crate::series_lite_contract::{VerifiedPack, CURRENT_MANIFEST_SHA256, CURRENT_RELEASE};
use crate::series_lite_desktop_runtime::put_exact;
use crate::series_lite_edit::{canonical, parse, sha256};
use crate::store::{s, Store};
use crate::workflow_artifacts as artifacts;
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::{collections::{BTreeMap, BTreeSet}, path::Path};
#[cfg(test)]
use std::fs;

type Result<T> = std::result::Result<T, String>;
pub(crate) const PROFILE: &str = "desktop-series-lite/v1";
const DELIVERY: &str = "desktop-lite-delivery/v1";
pub(crate) fn uses_profile(input: &Value) -> bool { input["workflowProfile"] == PROFILE }
pub(crate) fn mark_input(input: &mut Value) -> Result<()> {
    VerifiedPack::current_embedded()?;
    input["workflowProfile"] = json!(PROFILE);
    input["seriesSkillVersion"] = json!(CURRENT_RELEASE);
    input["seriesRulePackSha256"] = json!(CURRENT_MANIFEST_SHA256);
    input["liteControllerContract"] = json!(crate::series_lite_controller_store::VERSION);
    input["liteRuntimeContract"] = json!(crate::series_lite_controller_v2::VERSION);
    Ok(())
}
pub(crate) fn rule_snapshots_for(input: &Value) -> Result<Vec<RuleSnapshot>> {
    if !uses_profile(input) || input["seriesSkillVersion"] != CURRENT_RELEASE || input["seriesRulePackSha256"] != CURRENT_MANIFEST_SHA256 {
        return Err("LITE_RULE_CONTRACT_CHANGED: 保留原冻结任务，未改用安装版默认规则".into());
    }
    if !input["liteRuntimeContract"].is_null() && input["liteRuntimeContract"]!=crate::series_lite_controller_v2::VERSION {return Err("LITE_RUNTIME_CONTRACT_CHANGED: 保留原执行槽与预算，不改用默认运行器".into());}
    let pack = VerifiedPack::current_embedded()?;
    ["SKILL.md", "references/workflow.md", "references/creative-core.md", "references/asset-production-contract.md"]
        .into_iter().map(|path| {
            let bytes = pack.file_bytes(path)?;
            Ok(RuleSnapshot { id:format!("series-lite:{path}"), version:CURRENT_RELEASE.into(), sha256:sha256(bytes), content:std::str::from_utf8(bytes).map_err(|e|e.to_string())?.into() })
        }).collect()
}
fn install(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS series_lite_execution_intents_v1(run TEXT NOT NULL,episode TEXT NOT NULL,path TEXT NOT NULL,bytes TEXT NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(run,episode));
        CREATE TRIGGER IF NOT EXISTS lite_execution_intent_no_update BEFORE UPDATE ON series_lite_execution_intents_v1 BEGIN SELECT RAISE(ABORT,'LITE_INTENT_IMMUTABLE'); END;
        CREATE TRIGGER IF NOT EXISTS lite_execution_intent_no_delete BEFORE DELETE ON series_lite_execution_intents_v1 BEGIN SELECT RAISE(ABORT,'LITE_INTENT_IMMUTABLE'); END;
        CREATE TABLE IF NOT EXISTS series_lite_unit_replacements_v1(run TEXT NOT NULL,episode TEXT NOT NULL,proof TEXT NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(run,episode));
        CREATE TRIGGER IF NOT EXISTS lite_unit_replacement_no_update BEFORE UPDATE ON series_lite_unit_replacements_v1 BEGIN SELECT RAISE(ABORT,'LITE_REPLACEMENT_IMMUTABLE'); END;
        CREATE TRIGGER IF NOT EXISTS lite_unit_replacement_no_delete BEFORE DELETE ON series_lite_unit_replacements_v1 BEGIN SELECT RAISE(ABORT,'LITE_REPLACEMENT_IMMUTABLE'); END;") .map_err(|e|e.to_string())
}
fn metadata(store: &Store, key: &str) -> Result<Option<Value>> {
    let raw:Option<String> = store.conn()?.query_row("SELECT value FROM metadata WHERE key=?1",[key],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    raw.map(|s|serde_json::from_str(&s).map_err(|e|e.to_string())).transpose()
}
fn saved_delivery(store:&Store, episode:&str) -> Result<Option<Value>> { metadata(store,&format!("lite.delivery:{episode}")) }
fn boundary(store:&Store,input:&Value,index:usize)->Result<Value> {
    let episodes=input["source"]["episodes"].as_array().ok_or("LITE_SOURCE_EPISODES_MISSING")?;
    if index==0 {return Ok(json!({"kind":"firstInScope"}));}
    let prev=s(&episodes[index-1],"id");
    let Some(record)=saved_delivery(store,prev)? else {return Ok(json!({"kind":"missingPrevious","episodeId":prev}));};
    if record["sourceSha256"]!=input["source"]["sha256"] {return Ok(json!({"kind":"missingPrevious","episodeId":prev}));}
    validate_saved(store,&record,input)?;
    let identity=s(&record["adoption"],"identity");
    let version=record["adoption"]["version"].as_u64().ok_or("LITE_PREVIOUS_ADOPTION_VERSION")?;
    let adopted=artifacts::adoption(&store.conn()?,identity,version)?.ok_or("LITE_PREVIOUS_ADOPTION_MISSING")?;
    let candidate=artifacts::artifact(&store.conn()?,&adopted.candidate_id)?.ok_or("LITE_PREVIOUS_CANDIDATE_MISSING")?;
    let raw=s(&candidate.artifact.payload,"text");
    if candidate.payload_sha256!=adopted.candidate_sha256 || sha256(raw.as_bytes())!=record["nativeSha256"] {return Err("LITE_PREVIOUS_CANDIDATE_DRIFT".into());}
    Ok(json!({"kind":"supplied","episodeId":prev,"adoptionIdentity":identity,"version":version,"nativeSha256":record["nativeSha256"],"previousBundleUtf8":raw}))
}
pub(crate) fn current_boundary(store:&Store,input:&Value,episode:&str)->Result<Value> {
    let episodes=input["source"]["episodes"].as_array().ok_or("LITE_SOURCE_EPISODES_MISSING")?;
    let index=episodes.iter().position(|e|e["id"]==episode).ok_or("LITE_SOURCE_EPISODE_MISSING")?;
    if index>0 {
        let previous=s(&episodes[index-1],"id");
        if !saved_delivery(store,previous)?.is_some_and(|r|r["finalDeliveryReady"]==true) {
            return Ok(json!({"kind":"missingPrevious","episodeId":previous}));
        }
    }
    boundary(store,input,index)
}
fn refs(native:&Value,evidence:&Value)->Result<Vec<Value>> {
    let mut keys=BTreeSet::new();
    let mut found=BTreeSet::new();
    for row in native.as_array().ok_or("LITE_NATIVE_SOURCE_REFS")? {
        let key=row.as_str().ok_or("LITE_NATIVE_SOURCE_REF")?;
        let mapping=evidence["sourceMap"]["rowMappings"].as_array().ok_or("LITE_SOURCE_MAP_MISSING")?
            .iter().find(|r|r["standardRowId"]==key).ok_or("LITE_SOURCE_MAP_ROW_MISSING")?;
        for r in mapping["desktopRows"].as_array().ok_or("LITE_DESKTOP_ROWS_MISSING")? {
            let id=s(r,"desktopRowId");if !id.is_empty() {keys.insert(id.to_owned());}
        }
        // Exact native row ordering is separate from the Desktop's whole-row UI.
        found.insert(key.to_owned());
    }
    let mut ordered=Vec::new();
    for r in evidence["sourceMap"]["rowMappings"].as_array().ok_or("LITE_SOURCE_MAP_MISSING")? {
        for d in r["desktopRows"].as_array().into_iter().flatten() {
            if keys.remove(s(d,"desktopRowId")) {ordered.push(d["desktopRowId"].clone());}
        }
    }
    if !keys.is_empty() || (!found.is_empty()&&ordered.is_empty()) {return Err("LITE_SOURCE_MAP_INCOMPLETE".into());}
    Ok(ordered)
}
fn project_units(episode:&str,native_raw:&[u8],delivery:&crate::series_lite_bundle::Delivery,registry:&Value,evidence:&Value,adoption:&artifacts::Adoption,final_ready:bool)->Result<Vec<Value>> {
    if delivery.bundle_sha256 != sha256(native_raw) {return Err("LITE_PROJECTION_NATIVE_DELIVERY_MISMATCH".into());}
    let native=parse(native_raw)?;
    let bundle:Value=serde_json::from_slice(native_raw).map_err(|e|e.to_string())?;
    let units=bundle["episodes"][0]["draft"]["units"].as_array().ok_or("LITE_DRAFT_UNITS_MISSING")?;
    let normalized=crate::series_lite_bundle_standard::normalize_source(&bundle["episodes"][0]["source"])?;
    let mut out=Vec::new();
    for scene in delivery.projection["episodes"][0]["scenes"].as_array().ok_or("LITE_PROJECTION_SCENES")? {
        let original:Vec<_>=units.iter().enumerate().filter(|(_,u)|u["sceneId"]==scene["id"]).collect();
        let projected=scene["units"].as_array().ok_or("LITE_PROJECTION_UNITS")?;
        if original.len()!=projected.len() {return Err("LITE_PROJECTION_COUNT".into());}
        let desktop_scene=evidence["sourceMap"]["sceneMappings"].as_array().ok_or("LITE_SCENE_MAP")?.iter()
            .find(|m|m["originalScene"]==scene["id"] || bundle["episodes"][0]["scenes"].as_array().into_iter().flatten().any(|ns|ns["id"]==scene["id"] && ns["desktopSceneId"]==m["desktopSceneId"]))
            .ok_or("LITE_SCENE_IDENTITY_MISSING")?;
        for ((native_index,u),p) in original.into_iter().zip(projected) {
            let mut source_refs=Vec::new(); let mut seen=BTreeSet::new();
            let mut shots=Vec::new();let mut ticks=0i64;
            for shot in u["shots"].as_array().ok_or("LITE_SHOTS_MISSING")? {
                let mapped=refs(&shot["sourceRefs"],evidence)?;
                for r in &mapped {if seen.insert(r.clone().to_string()){source_refs.push(r.clone());}}
                let duration=crate::series_lite_bundle::time_ticks(&shot["duration"],true,"LITE_SHOT_TIME")?;
                let mut item=shot.clone();
                for part in item["dialogueParts"].as_array_mut().into_iter().flatten() {
                    let line=normalized["dialogue"].as_array().ok_or("LITE_NORMALIZED_DIALOGUE")?.iter().find(|d|d["id"]==part["lineId"]).ok_or("LITE_DIALOGUE_SOURCE_MISSING")?;
                    for key in ["speaker","channel","kind","cue"] {part[key]=line[key].clone();}
                }
                item["sourceRefs"]=json!(mapped);item["startTicks"]=json!(ticks);ticks+=duration;item["endTicks"]=json!(ticks);shots.push(item);
            }
            let asset_refs=u["imageBindings"].as_array().ok_or("LITE_IMAGE_BINDINGS")?.iter().enumerate().map(|(i,k)| {
                let asset=registry["assets"].as_array().ok_or("LITE_REGISTRY_ASSETS")?.iter().find(|a|a["key"]==*k).ok_or("LITE_IMAGE_IDENTITY_MISSING")?;
                let d=&asset["desktopIdentity"];
                if s(d,"desktopAssetId").is_empty()||s(d,"variantId").is_empty(){return Err("LITE_DESKTOP_ASSET_IDENTITY_MISSING".into());}
                Ok(json!({"assetId":d["desktopAssetId"],"variantId":d["variantId"],"designRevision":asset["revision"],"role":match s(asset,"type"){"SC"=>"scene","PR"=>"prop",_=>"character"},"order":i+1}))
            }).collect::<Result<Vec<_>>>()?;
            let mut prompt=crate::series_lite_bundle::render_projected_unit(p)?;
            for a in registry["assets"].as_array().into_iter().flatten() {
                let tag=format!("@[{}{}]",s(a,"importName"),s(a,"type"));
                let d=&a["desktopIdentity"];
                prompt=prompt.replace(&tag,&format!("{{{{asset:{}:{}}}}}",s(d,"desktopAssetId"),s(d,"variantId")));
            }
            let exact_unit=&native.get("episodes")?.array()?[0].get("draft")?.get("units")?.array()?[native_index];
            out.push(json!({"id":format!("lite-unit:{}",digest(&json!([episode,u["id"]]).to_string())),"episodeId":episode,"episodeNumber":bundle["episodes"][0]["number"],
                "sceneId":desktop_scene["desktopSceneId"],"scene":u["scene"],"title":u["title"],"prompt":prompt,"sourceRefs":source_refs,"assetRefs":asset_refs,"imageBindings":asset_refs,"shots":shots,
                "duration":p["duration"],"durationSeconds":p["duration"],"actualDurationTicks":ticks,"initialState":u["initialState"],"exitState":u["exitState"],
                "status":if final_ready{"completed"}else{"needs_review"},"reviewStatus":if final_ready{"adopted"}else{"boundary_pending"},
                "workflowProfile":PROFILE,"seriesLite":{"version":DELIVERY,"nativeUnitId":u["id"],"nativeUnitUtf8":String::from_utf8(canonical(exact_unit)).map_err(|e|e.to_string())?,"nativeSha256":sha256(native_raw),"adoptionIdentity":adoption.identity,"adoptionVersion":adoption.version,"finalDeliveryReady":final_ready},
                "timingProfile":"lingmo-v7-nanoseconds/v1","timing":{"actualTicks":ticks,"containerSeconds":p["duration"]}}));
        }
    }
    Ok(out)
}
fn persist_intent(store:&Store,run:&str,episode:&str,path:&str,bytes:&[u8])->Result<()> {
    let text=std::str::from_utf8(bytes).map_err(|e|e.to_string())?;
    let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;install(&tx)?;
    let old:Option<(String,String,String)>=tx.query_row("SELECT path,bytes,sha256 FROM series_lite_execution_intents_v1 WHERE run=?1 AND episode=?2",params![run,episode],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    if let Some((p,b,h))=old {if p!=path||b.as_bytes()!=bytes||h!=sha256(bytes){return Err("LITE_EXECUTION_INTENT_DRIFT".into());}}
    else {tx.execute("INSERT INTO series_lite_execution_intents_v1 VALUES(?1,?2,?3,?4,?5)",params![run,episode,path,text,sha256(bytes)]).map_err(|e|e.to_string())?;}
    tx.commit().map_err(|e|e.to_string())
}
const REPLACEMENT: &str = "desktop-lite-unit-replacement/v1";
fn replacement(tx:&Transaction<'_>,run:&str,episode:&str)->Result<Option<Value>> {
    let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='series_lite_unit_replacements_v1')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists {return Ok(None);}
    let raw:Option<(String,String)>=tx.query_row("SELECT proof,sha256 FROM series_lite_unit_replacements_v1 WHERE run=?1 AND episode=?2",params![run,episode],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    raw.map(|(body,pin)| {
        if sha256(body.as_bytes())!=pin {return Err("LITE_REPLACEMENT_PROOF_DRIFT".into());}
        serde_json::from_str(&body).map_err(|e|e.to_string())
    }).transpose()
}
fn tx_unit(tx:&Transaction<'_>,id:&str)->Result<Option<Value>> {
    let raw:Option<String>=tx.query_row("SELECT data FROM units WHERE id=?1",[id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    raw.map(|r|serde_json::from_str(&r).map_err(|e|e.to_string())).transpose()
}
/// Publish only an adopted final projection. A provisional candidate stays in
/// artifacts/metadata and cannot replace a user's active or adopted episode.
fn publish_units(tx:&Transaction<'_>,run:&str,episode:&str,guard:&Value,projected:&[Value],adoption:&artifacts::Adoption,final_ready:bool)->Result<Vec<Value>> {
    if !final_ready {return Ok(Vec::new());}
    crate::series_lite_guard::verify(tx,guard)?;
    let stored=artifacts::adoption(tx,&adoption.identity,adoption.version)?.ok_or("LITE_REPLACEMENT_ADOPTION_MISSING")?;
    if stored!=*adoption || adoption.source_sha256!=s(guard,"sourceSha256")
        || [artifacts::Capability::Structure,artifacts::Capability::SourceCoverage,artifacts::Capability::ContentReview].iter().any(|c|!adoption.capabilities.contains(c)) {
        return Err("LITE_REPLACEMENT_FINAL_ADOPTION_REQUIRED".into());
    }
    let (snapshot,state):(String,String)=tx.query_row("SELECT snapshot,state FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let parent:Value=serde_json::from_str(&snapshot).map_err(|e|e.to_string())?;
    if state!="running" || parent["runnerVersion"]!="desktop-runner/v1" || !uses_profile(&parent["input"])
        || parent["input"]["source"]["sha256"]!=guard["sourceSha256"]
        || parent["input"]["source"]["version"]!=guard["sourceVersion"]
        || !parent["input"]["selection"]["episodeIds"].as_array().is_some_and(|ids|ids.contains(&json!(episode)))
        || guard["episodeIds"]!=json!([episode]) {
        return Err("LITE_REPLACEMENT_PARENT_SCOPE_CHANGED".into());
    }
    let candidate=artifacts::artifact(tx,&adoption.candidate_id)?.ok_or("LITE_REPLACEMENT_CANDIDATE_MISSING")?;
    let native_hash=sha256(s(&candidate.artifact.payload,"text").as_bytes());
    if candidate.payload_sha256!=adoption.candidate_sha256 {return Err("LITE_REPLACEMENT_CANDIDATE_CHANGED".into());}
    let mut ids=BTreeSet::new();
    for u in projected {
        let id=s(u,"id");
        if id.is_empty() || !ids.insert(id.to_owned()) || u["episodeId"]!=episode || u["retired"]==true
            || u["seriesLite"]["adoptionIdentity"]!=adoption.identity || u["seriesLite"]["adoptionVersion"]!=adoption.version
            || u["seriesLite"]["nativeSha256"]!=native_hash || u["seriesLite"]["finalDeliveryReady"]!=true {
            return Err("LITE_REPLACEMENT_PROJECTED_SCOPE_CHANGED".into());
        }
        if tx_unit(tx,id)?.is_some_and(|old|old["episodeId"]!=episode) {return Err("LITE_REPLACEMENT_UNIT_ID_COLLISION".into());}
    }
    if ids.is_empty() {return Err("LITE_REPLACEMENT_EMPTY_PROJECTION".into());}
    install(tx)?;
    if replacement(tx,run,episode)?.is_some() {return Err("LITE_REPLACEMENT_ALREADY_RECORDED".into());}
    let mut q=tx.prepare("SELECT data FROM units WHERE episode_id=?1 AND COALESCE(json_extract(data,'$.retired'),0)<>1 ORDER BY id").map_err(|e|e.to_string())?;
    let old=q.query_map([episode],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    drop(q);
    let old:Vec<Value>=old.into_iter().map(|r|serde_json::from_str(&r).map_err(|e|e.to_string())).collect::<Result<_>>()?;
    let frozen=parent["input"]["existingUnits"].as_array().ok_or("LITE_REPLACEMENT_FROZEN_UNITS_MISSING")?;
    let frozen_ids:BTreeSet<_>=frozen.iter().filter(|u|u["episodeId"]==episode && u["retired"]!=true).map(|u|s(u,"id")).collect();
    if frozen_ids!=old.iter().map(|u|s(u,"id")).collect::<BTreeSet<_>>() {return Err("LITE_REPLACEMENT_FROZEN_SCOPE_CHANGED".into());}
    let mut retired=Vec::new();
    for previous in &old {
        let id=s(previous,"id");
        let frozen=frozen.iter().find(|u|u["id"]==id).ok_or("LITE_REPLACEMENT_FROZEN_UNIT_MISSING")?;
        let historical:String=tx.query_row("SELECT data FROM unit_versions WHERE unit_id=?1 AND version=?2",params![id,previous["version"].as_i64()],|r|r.get(0)).map_err(|_|"LITE_REPLACEMENT_ORIGINAL_VERSION_MISSING")?;
        let historical:Value=serde_json::from_str(&historical).map_err(|e|e.to_string())?;
        if previous!=&historical || previous["version"]!=frozen["version"] || unit_text_pin(previous)!=unit_text_pin(frozen)
            || guard["units"][id]!=previous["version"] || parent["guard"]["units"][id]!=previous["version"] {
            return Err("LITE_REPLACEMENT_ORIGINAL_UNIT_CHANGED".into());
        }
        if !ids.contains(id) {
            let mut value=previous.clone();value["retired"]=json!(true);
            value["expectedVersion"]=previous["version"].clone();
            value["liteRetirement"]=json!({"version":REPLACEMENT,"run":run,"episodeId":episode,"sourceSha256":guard["sourceSha256"],"adoptionId":adoption.id,"originalVersion":previous["version"],"replacedBy":ids});
            let after=crate::store::save_unit_tx(tx,value)?;
            retired.push(json!({"before":previous,"after":after}));
        }
    }
    let mut saved=Vec::new();
    for u in projected {
        let mut u=u.clone();u["retired"]=json!(false);u["sourceVersion"]=guard["sourceVersion"].clone();
        saved.push(crate::store::save_unit_tx(tx,u)?);
    }
    let proof=json!({"version":REPLACEMENT,"run":run,"episodeId":episode,"sourceVersion":guard["sourceVersion"],"sourceSha256":guard["sourceSha256"],"adoption":adoption,"nativeSha256":native_hash,"guard":guard,"retiredUnits":retired,"publishedUnits":saved});
    let body=proof.to_string();
    tx.execute("INSERT INTO series_lite_unit_replacements_v1 VALUES(?1,?2,?3,?4)",params![run,episode,body,sha256(body.as_bytes())]).map_err(|e|e.to_string())?;
    Ok(saved)
}
async fn one(store:&Store,run:&str,input:&Value,episode:&str,executor:&dyn TextExecutor,model:&str,reasoning:&str)->Result<Value> {
    let prepared=crate::series_lite_desktop_prepare::prepare(store,run,input,episode,executor,model,reasoning).await?;
    let options=crate::series_lite_job_packet::PreparationOptions{context:if prepared.root.join("previous-boundary.json").is_file(){Some(("previous-boundary.json".into(),None))}else{None},..Default::default()};
    let chain=crate::series_lite_desktop_runtime::execute_chain(store,run,&prepared.root,&prepared.base,&prepared.supply,&prepared.guard,prepared.common_inputs.clone(),&options,executor,model,reasoning).await?;
    let v2=input["liteRuntimeContract"]==crate::series_lite_controller_v2::VERSION;
    let slot_hash=sha256(chain.reviewer.slot_id.as_bytes());
    let bundle_path=if v2{format!("candidate-{}.json",sha256(&chain.candidate))}else{"final-candidate.json".into()};
    let manifest_path=if v2{format!("execution-{slot_hash}.json")}else{"execution-request.json".into()};
    let output_path=if v2{format!("delivery-{slot_hash}")}else{"delivery".into()};
    let intent_episode=if v2{format!("{episode}:slot:{slot_hash}")}else{episode.to_owned()};
    put_exact(&prepared.root,&bundle_path,&chain.candidate)?;
    let manifest=json!({"version":"series-episode-execution-v1","episode":prepared.episode_number,"packet":chain.packet_path,"bundle":bundle_path,"sourceLock":"source-lock.json","registry":"registry"});
    put_exact(&prepared.root,&manifest_path,&serde_json::to_vec(&manifest).map_err(|e|e.to_string())?)?;
    let pack=VerifiedPack::current_embedded()?;
    let intent:Option<(String,String,String)>={
        let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;install(&tx)?;
        let row=tx.query_row("SELECT path,bytes,sha256 FROM series_lite_execution_intents_v1 WHERE run=?1 AND episode=?2",params![run,intent_episode],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())?;row
    };
    let execution=if let Some((_,bytes,pin))=&intent {
        crate::series_lite_execution::reconcile_commit_intent(&prepared.root,&manifest_path,&output_path,bytes.as_bytes(),pin)?
    } else {
        crate::series_lite_execution::run_with_commit_intent(&prepared.root,&manifest_path,&output_path,&pack,None,&mut |path,bytes|persist_intent(store,run,&intent_episode,path,bytes))?
    };
    let registry:Value=serde_json::from_slice(&prepared.registry_snapshot).map_err(|e|e.to_string())?;
    let mut gate=crate::series_lite_bundle_gate::NativeLegacyBundleGate{registry:Some(&registry),source_lock:Some(&prepared.source_lock),season_scope:None};
    let delivery=crate::series_lite_bundle::export_lingmo_v7(&chain.candidate,&mut gate)?;
    let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
    crate::series_lite_guard::verify(&tx,&prepared.guard)?;
    let old_adoption:Option<String>=tx.query_row("SELECT value FROM metadata WHERE key=?1",[format!("lite.delivery:{episode}")],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let previous=old_adoption.as_deref().map(serde_json::from_str::<Value>).transpose().map_err(|e|e.to_string())?;
    let expected_previous=previous.as_ref().filter(|r|r["run"]==run && r["finalDeliveryReady"]!=true).and_then(|r|r["adoption"]["version"].as_u64()).unwrap_or(0);
    let outcome=crate::series_lite_desktop_publish::validate_and_adopt(&tx,store,&crate::series_lite_desktop_publish::AdoptionRequest{run,episode_id:episode,episode_number:prepared.episode_number,workspace_root:&prepared.root,execution_receipt:&execution.receipt_path,execution_receipt_sha256:&execution.receipt_file_sha256,expected_previous,current_inputs:&prepared.common_inputs})?;
    let Some(adoption)=outcome.adoption.as_ref() else {
        tx.commit().map_err(|e|e.to_string())?;
        return Ok(json!({"episodeId":episode,"status":"needs_review","review":outcome.review,"candidateArtifactId":outcome.candidate_artifact_id}));
    };
    let units=project_units(episode,&chain.candidate,&delivery,&registry,&prepared.evidence,adoption,outcome.final_delivery_ready)?;
    let frozen_boundary=crate::series_lite_late_boundary::load(&tx,run,episode)?.map(|v|v["boundary"].clone()).or(crate::series_lite_desktop_prepare::frozen_boundary(&tx,run,episode)?.filter(|v|!v.is_null()))
        .unwrap_or_else(||input["liteBoundaryByEpisode"][prepared.episode_number.to_string()].clone());
    if outcome.final_delivery_ready {crate::series_lite_consumers::verify_boundary(&tx,&frozen_boundary,s(&input["source"],"sha256"))?;}
    let saved=publish_units(&tx,run,episode,&prepared.guard,&units,adoption,outcome.final_delivery_ready)?;
    if outcome.final_delivery_ready {crate::series_lite_consumers::register(&tx,episode,&frozen_boundary,adoption,&saved)?;}
    let record=json!({"version":DELIVERY,"run":run,"episodeId":episode,"episodeNumber":prepared.episode_number,"sourceVersion":input["source"]["version"],"sourceSha256":input["source"]["sha256"],"nativeSha256":outcome.candidate_native_sha256,
        "workspaceRoot":prepared.root,"receiptPath":execution.receipt_path,"receiptSha256":execution.receipt_file_sha256,"candidateArtifactId":outcome.candidate_artifact_id,"adoption":adoption,"finalDeliveryReady":outcome.final_delivery_ready,"boundaryPending":outcome.boundary_pending,
        "review":outcome.review,"actualContexts":outcome.actual_contexts,"markdownSha256":delivery.markdown_sha256,"manifestSha256":delivery.manifest_sha256,"units":saved,"candidateUnits":units,"projectionPublished":outcome.final_delivery_ready,"boundaryConsumersVersion":if outcome.final_delivery_ready{json!(crate::series_lite_consumers::VERSION)}else{Value::Null},"consumedBoundary":frozen_boundary,"guard":prepared.guard,"inputs":prepared.common_inputs});
    let current:Option<String>=tx.query_row("SELECT value FROM metadata WHERE key=?1",[format!("lite.delivery:{episode}")],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let prior:Option<Value>=current.map(|r|serde_json::from_str(&r).map_err(|e|e.to_string())).transpose()?;
    // Keep an earlier adopted pointer when this attempt only has a candidate.
    // The new candidate still has its immutable raw/adoption and full record.
    if outcome.final_delivery_ready || !prior.as_ref().is_some_and(|p|p["finalDeliveryReady"]==true) {
        tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("lite.delivery:{episode}"),record.to_string()]).map_err(|e|e.to_string())?;
    }
    tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",params![format!("lite.delivery-history:{}",adoption.id),record.to_string()]).map_err(|e|e.to_string())?;
    crate::store::mark_changed(&tx)?;
    tx.commit().map_err(|e|e.to_string())?;
    store.event(run,if outcome.final_delivery_ready{"lite_episode_published"}else{"lite_episode_candidate_saved"},&json!({"episodeId":episode,"finalDeliveryReady":outcome.final_delivery_ready,"unitCount":saved.len(),"candidateUnitCount":units.len(),"adoption":adoption}))?;
    Ok(json!({"episodeId":episode,"status":if outcome.final_delivery_ready{"completed"}else{"boundary_pending"},"unitCount":saved.len(),"candidateUnitCount":units.len(),"adoption":adoption}))
}
fn validate_saved(store:&Store,record:&Value,input:&Value)->Result<()> {
    if record["version"]!=DELIVERY || record["sourceSha256"]!=input["source"]["sha256"] {return Err("LITE_SAVED_SOURCE_CHANGED".into());}
    let id=s(&record["adoption"],"identity");let v=record["adoption"]["version"].as_u64().ok_or("LITE_SAVED_ADOPTION_VERSION")?;
    let adopted=artifacts::adoption(&store.conn()?,id,v)?.ok_or("LITE_SAVED_ADOPTION_MISSING")?;
    if serde_json::to_value(&adopted).map_err(|e|e.to_string())?!=record["adoption"] || adopted.source_sha256!=s(record,"sourceSha256") {return Err("LITE_SAVED_ADOPTION_DRIFT".into());}
    let candidate=artifacts::artifact(&store.conn()?,&adopted.candidate_id)?.ok_or("LITE_SAVED_CANDIDATE_MISSING")?;
    if candidate.artifact.id!=s(record,"candidateArtifactId") || candidate.payload_sha256!=adopted.candidate_sha256 || sha256(s(&candidate.artifact.payload,"text").as_bytes())!=record["nativeSha256"] {return Err("LITE_SAVED_CANDIDATE_DRIFT".into());}
    if record["finalDeliveryReady"]==true && [artifacts::Capability::Structure,artifacts::Capability::SourceCoverage,artifacts::Capability::ContentReview].iter().any(|c|!adopted.capabilities.contains(c)) {return Err("LITE_SAVED_FINAL_ADOPTION_REQUIRED".into());}
    let actual=store.values("units")?;
    for u in record["units"].as_array().ok_or("LITE_SAVED_UNITS_MISSING")? {
        if !actual.iter().any(|a|a["id"]==u["id"] && unit_text_pin(a)==unit_text_pin(u)) {return Err("LITE_SAVED_UNIT_EDITED: 人工稿已保留，需核对新版本".into());}
    }
    if record["projectionPublished"]==true {
        let saved_ids:BTreeSet<_>=record["units"].as_array().unwrap().iter().map(|u|s(u,"id")).collect();
        let active_ids:BTreeSet<_>=actual.iter().filter(|u|u["episodeId"]==record["episodeId"]).map(|u|s(u,"id")).collect();
        if saved_ids.is_empty() || active_ids!=saved_ids {return Err("LITE_SAVED_UNIT_SCOPE_EDITED: 当前集稿范围变化，人工稿已保留".into());}
    }
    let root=Path::new(s(record,"workspaceRoot"));
    crate::series_lite_execution::verify_receipt(root,s(record,"receiptPath"),s(record,"receiptSha256"),record["episodeNumber"].as_i64())?;
    // Revalidate only consumed designs; exclude the already-published unit set.
    let mut guard=record["guard"].clone();
    if record["projectionPublished"]!=false {guard["episodeIds"]=json!([]);guard["units"]=json!({});}
    let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
    crate::series_lite_guard::verify(&tx,&guard)?;
    crate::series_lite_consumers::verify_consumers(&tx,record)
}
pub(crate) fn unit_text_pin(u:&Value)->String {
    digest(&json!({"id":u["id"],"episodeId":u["episodeId"],"sceneId":u["sceneId"],"title":u["title"],"prompt":u["prompt"],"sourceRefs":u["sourceRefs"],"assetRefs":u["assetRefs"],"shots":u["shots"],"duration":u["duration"],"durationSeconds":u["durationSeconds"],"initialState":u["initialState"],"exitState":u["exitState"],"seriesLite":u["seriesLite"]}).to_string())
}
pub(crate) fn verify_unit_for_media(store:&Store,unit:&Value)->Result<()> {
    if unit["seriesLite"].is_null() {return Ok(());}
    let record=saved_delivery(store,s(unit,"episodeId"))?.ok_or("LITE_MEDIA_ADOPTED_EPISODE_MISSING")?;
    if record["finalDeliveryReady"]!=true {return Err("本集的内容或跨集接续尚待核对；其它已采用单元可继续配图".into());}
    validate_saved(store,&record,&json!({"source":store.source()?}))?;
    if !record["units"].as_array().is_some_and(|units|units.iter().any(|saved|saved["id"]==unit["id"]&&unit_text_pin(saved)==unit_text_pin(unit))) {return Err("LITE_MEDIA_TEXT_NOT_ADOPTED".into());}
    Ok(())
}
/// Rebind only versions of this workflow's own unchanged text projection.
/// Neither the immutable parent nor the original per-role guard is rewritten.
pub(crate) fn refresh_guard(store:&Store,run:&str,snapshot:&Value)->Result<()> {
    if !uses_profile(&snapshot["input"]) {return Ok(());}
    let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
    let mut guard=snapshot["guard"].clone();
    for ep in snapshot["input"]["selection"]["episodeIds"].as_array().into_iter().flatten() {
        let episode=ep.as_str().ok_or("LITE_GUARD_EPISODE")?;
        let record:Option<String>=tx.query_row("SELECT value FROM metadata WHERE key=?1",[format!("lite.delivery:{episode}")],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let Some(record)=record.map(|r|serde_json::from_str::<Value>(&r).map_err(|e|e.to_string())).transpose()? else {continue;};
        if record["run"]!=run {continue;}
        if let Some(proof)=replacement(&tx,run,episode)? {
            if proof["version"]!=REPLACEMENT || proof["run"]!=run || proof["episodeId"]!=ep.clone()
                || proof["sourceVersion"]!=guard["sourceVersion"] || proof["sourceSha256"]!=guard["sourceSha256"]
                || proof["adoption"]!=record["adoption"] || proof["nativeSha256"]!=record["nativeSha256"]
                || proof["publishedUnits"]!=record["units"] || record["projectionPublished"]!=true || record["finalDeliveryReady"]!=true {
                return Err("LITE_GUARD_REPLACEMENT_BINDING_CHANGED".into());
            }
            let adoption=artifacts::adoption(&tx,s(&record["adoption"],"identity"),record["adoption"]["version"].as_u64().ok_or("LITE_GUARD_REPLACEMENT_ADOPTION")?)?.ok_or("LITE_GUARD_REPLACEMENT_ADOPTION_MISSING")?;
            if serde_json::to_value(&adoption).map_err(|e|e.to_string())?!=proof["adoption"] || !adoption.capabilities.contains(&artifacts::Capability::ContentReview) {return Err("LITE_GUARD_REPLACEMENT_ADOPTION_CHANGED".into());}
            for retired in proof["retiredUnits"].as_array().ok_or("LITE_GUARD_RETIRED_PROOF_MISSING")? {
                let before=&retired["before"];let after=&retired["after"];let id=s(before,"id");
                let frozen=snapshot["input"]["existingUnits"].as_array().into_iter().flatten().find(|u|u["id"]==before["id"]).ok_or("LITE_GUARD_RETIREMENT_NOT_FROZEN")?;
                let actual=tx_unit(&tx,id)?.ok_or("LITE_GUARD_RETIRED_UNIT_REMOVED")?;
                let historical:String=tx.query_row("SELECT data FROM unit_versions WHERE unit_id=?1 AND version=?2",params![id,before["version"].as_i64()],|r|r.get(0)).map_err(|_|"LITE_GUARD_RETIRED_HISTORY_MISSING")?;
                let historical:Value=serde_json::from_str(&historical).map_err(|e|e.to_string())?;
                if id.is_empty() || before["episodeId"]!=*ep || before["version"]!=guard["units"][id]
                    || frozen["version"]!=before["version"] || unit_text_pin(frozen)!=unit_text_pin(before)
                    || historical!=*before || actual!=*after || after["retired"]!=true
                    || after["version"].as_i64()!=before["version"].as_i64().and_then(|v|v.checked_add(1))
                    || after["liteRetirement"]["run"]!=run || after["liteRetirement"]["adoptionId"]!=adoption.id {
                    return Err("LITE_GUARD_RETIRED_UNIT_CHANGED".into());
                }
                guard["units"].as_object_mut().ok_or("LITE_GUARD_UNIT_MAP_MISSING")?.remove(id);
            }
        } else if record["projectionPublished"]==true {
            return Err("LITE_GUARD_REPLACEMENT_PROOF_MISSING".into());
        }
        for original in record["units"].as_array().ok_or("LITE_GUARD_SAVED_UNITS")? {
            let actual=tx_unit(&tx,s(original,"id"))?.filter(|u|u["retired"]!=true).ok_or("LITE_GUARD_PUBLISHED_UNIT_REMOVED")?;
            if unit_text_pin(original)!=unit_text_pin(&actual) {return Err("LITE_GUARD_PUBLISHED_TEXT_CHANGED".into());}
            guard["units"][s(&actual,"id")]=actual["version"].clone();
        }
    }
    crate::series_lite_guard::verify(&tx,&guard)?;
    let (saved,raw):(String,Option<String>)=tx.query_row("SELECT snapshot,prepared FROM jobs WHERE id=?1",[run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let original:Value=serde_json::from_str(&saved).map_err(|e|e.to_string())?;
    if original!=*snapshot {return Err("LITE_PARENT_SNAPSHOT_DRIFT".into());}
    let mut prepared=raw.as_deref().map(serde_json::from_str::<Value>).transpose().map_err(|e|e.to_string())?.unwrap_or(json!({}));
    if prepared["publicationGuard"]!=guard {
        prepared["publicationGuard"]=guard;
        let count=tx.execute("UPDATE jobs SET prepared=?1 WHERE id=?2 AND snapshot=?3 AND prepared IS ?4",params![prepared.to_string(),run,saved,raw]).map_err(|e|e.to_string())?;
        if count!=1 {return Err("LITE_PARENT_GUARD_CAS".into());}
    }
    tx.commit().map_err(|e|e.to_string())
}
pub(crate) async fn run(store:&Store,run:&str,input:&Value,executor:&dyn TextExecutor,model:&str,reasoning:&str)->Result<Value> {
    rule_snapshots_for(input)?;
    let selected=input["selection"]["episodeIds"].as_array().filter(|a|!a.is_empty()).ok_or("LITE_EPISODE_SELECTION_REQUIRED")?;
    let episodes=input["source"]["episodes"].as_array().ok_or("LITE_SOURCE_EPISODES_MISSING")?;
    if selected.iter().map(Value::to_string).collect::<BTreeSet<_>>().len()!=selected.len()||selected.iter().any(|id|!episodes.iter().any(|e|e["id"]==*id)) {return Err("LITE_EPISODE_SELECTION_INVALID".into());}
    let mut results=Vec::new();
    for (index,ep) in episodes.iter().enumerate().filter(|(_,e)|selected.contains(&e["id"])) {
        let id=s(ep,"id");
        let result:Result<Value>=async {if let Some(record)=saved_delivery(store,id)? {
            validate_saved(store,&record,input)?;
            if record["run"]==run && record["boundaryPending"]==true && input["liteRuntimeContract"]==crate::series_lite_controller_v2::VERSION && current_boundary(store,input,id)?["kind"]=="supplied" {
                one(store,run,input,id,executor,model,reasoning).await
            }else{
                Ok(json!({"episodeId":id,"status":if record["finalDeliveryReady"]==true{"completed"}else{"boundary_pending"},"reused":true,"unitCount":record["units"].as_array().map(Vec::len).unwrap_or(0)}))
            }
        } else {
            let mut episode_input=input.clone();
            let number=ep["sourceNumber"].as_str().and_then(super::number_value).ok_or("LITE_SOURCE_NUMBER_MISSING")?;
            episode_input["liteBoundaryByEpisode"]=json!({number.to_string():current_boundary(store,input,id)?});
            one(store,run,&episode_input,id,executor,model,reasoning).await
        }}.await;
        match result {
            Ok(v)=>results.push(v),
            Err(error)=>{let item=json!({"episodeId":id,"status":"needs_attention","code":"LITE_EPISODE_HELD","detail":error,"impact":[id],"retained":true});store.event(run,"lite_episode_held",&item)?;results.push(item);}
        }
    }
    let ready=results.iter().filter(|r|r["status"]=="completed").count();
    let blocked:Vec<_>=results.iter().filter(|r|r["status"]!="completed").map(|r|json!({"episodeId":r["episodeId"],"code":r.get("code").cloned().unwrap_or(json!("LITE_EPISODE_REVIEW_PENDING")),"reason":r.get("detail").cloned().unwrap_or(json!("本集候选或接续待核对，其余集数继续")),"review":r["review"],"impact":[r["episodeId"]]})).collect();
    Ok(json!({"schemaVersion":DELIVERY,"workflowProfile":PROFILE,"status":if ready==results.len(){"completed"}else{"partially_blocked"},"episodes":results,"blocked":blocked,"completedEpisodes":ready,"selectedEpisodes":selected.len(),"selection":input["selection"]}))
}
/// Mixed old/new scopes are explicit; a lite record never enters the old reader.
pub(crate) fn export_delivery(store:&Store,request:&Value)->Result<Option<Value>> {
    let selected=request["episodeIds"].as_array().filter(|a|!a.is_empty()).ok_or("请选择要导出的已采用集数")?;
    let mut records=Vec::new();
    for id in selected {if let Some(record)=saved_delivery(store,id.as_str().ok_or("LITE_EXPORT_EPISODE_ID")?)? {records.push(record);}}
    if records.is_empty(){return Ok(None);}
    if records.len()!=selected.len(){return Err("所选集数混有旧版与lite交付，请分开导出".into());}
    let input=json!({"source":store.source()?});
    let source=store.source()?;
    let order:BTreeMap<_,_>=source["episodes"].as_array().ok_or("LITE_EXPORT_SOURCE")?.iter().enumerate().map(|(i,e)|(s(e,"id").to_owned(),i)).collect();
    records.sort_by_key(|r|order.get(s(r,"episodeId")).copied().unwrap_or(usize::MAX));
    for r in &records {validate_saved(store,r,&input)?;if r["finalDeliveryReady"]!=true{return Err(format!("{} 尚有未核对接续或内容问题，候选保留",s(r,"episodeId")));}}
    let target=Path::new(s(request,"path"));if !target.is_absolute()||!target.is_dir(){return Err("请选择已有本地导出目录".into());}
    Ok(Some(super::series_lite_export::write(store,&records,target)?))
}
#[cfg(test)]
#[path="series_lite_tests.rs"]
mod tests;
#[cfg(test)]
#[path="series_lite_retirement_tests.rs"]
mod retirement_tests;
