//! Exact, immutable links from an adopted episode to the predecessor actually
//! supplied to it. Reads never migrate old records or rewrite downstream text.
use crate::pipeline::{digest, series_lite::unit_text_pin};
use crate::store::s;
use crate::workflow_artifacts::{self as artifacts, Adoption, Capability};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;
pub(crate) const VERSION: &str = "desktop-lite-boundary-consumers/v1";
fn held(code:&str)->String {format!("LITE_BOUNDARY_CONSUMER:{code}；保留原稿、原回包和已采用图片")}
fn install(tx:&Transaction<'_>)->Result<()> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS series_lite_boundary_consumers_v1(adoption_id TEXT PRIMARY KEY,episode TEXT NOT NULL,proof TEXT NOT NULL,sha256 TEXT NOT NULL);
        CREATE TRIGGER IF NOT EXISTS lite_boundary_consumer_no_update BEFORE UPDATE ON series_lite_boundary_consumers_v1 BEGIN SELECT RAISE(ABORT,'LITE_BOUNDARY_CONSUMER_IMMUTABLE'); END;
        CREATE TRIGGER IF NOT EXISTS lite_boundary_consumer_no_delete BEFORE DELETE ON series_lite_boundary_consumers_v1 BEGIN SELECT RAISE(ABORT,'LITE_BOUNDARY_CONSUMER_IMMUTABLE'); END;") .map_err(|e|e.to_string())
}
fn proof(c:&Connection,id:&str)->Result<Option<Value>> {
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='series_lite_boundary_consumers_v1')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists {return Ok(None);}
    let row:Option<(String,String)>=c.query_row("SELECT proof,sha256 FROM series_lite_boundary_consumers_v1 WHERE adoption_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    row.map(|(raw,pin)| {
        if digest(&raw)!=pin {return Err(held("PROOF_BYTES_CHANGED"));}
        serde_json::from_str(&raw).map_err(|e|e.to_string())
    }).transpose()
}
fn saved(c:&Connection,episode:&str)->Result<Value> {
    let raw:Option<String>=c.query_row("SELECT value FROM metadata WHERE key=?1",[format!("lite.delivery:{episode}")],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    serde_json::from_str(&raw.ok_or_else(||held("PREVIOUS_DELIVERY_MISSING"))?).map_err(|e|e.to_string())
}
fn adoption(c:&Connection,value:&Value,source_sha:&str)->Result<Adoption> {
    let identity=s(value,"identity");let version=value["version"].as_u64().ok_or_else(||held("ADOPTION_VERSION_MISSING"))?;
    let a=artifacts::adoption(c,identity,version)?.ok_or_else(||held("ADOPTION_MISSING"))?;
    if serde_json::to_value(&a).map_err(|e|e.to_string())?!=*value || a.source_sha256!=source_sha
        || [Capability::Structure,Capability::SourceCoverage,Capability::ContentReview].iter().any(|cap|!a.capabilities.contains(cap)) {
        return Err(held("FINAL_ADOPTION_CHANGED"));
    }
    Ok(a)
}
fn pins(units:&[Value],episode:&str)->Result<Value> {
    let mut out=BTreeMap::new();
    for u in units {
        let id=s(u,"id");
        if id.is_empty() || u["episodeId"]!=episode || u["retired"]==true || out.insert(id.to_owned(),unit_text_pin(u)).is_some() {
            return Err(held("UNIT_SCOPE_INVALID"));
        }
    }
    if out.is_empty() {return Err(held("ADOPTED_UNITS_MISSING"));}
    Ok(json!(out))
}
fn current_pins(c:&Connection,episode:&str)->Result<Value> {
    let mut q=c.prepare("SELECT data FROM units WHERE episode_id=?1 AND COALESCE(json_extract(data,'$.retired'),0)<>1 ORDER BY id").map_err(|e|e.to_string())?;
    let raw=q.query_map([episode],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let units:Vec<Value>=raw.into_iter().map(|r|serde_json::from_str(&r).map_err(|e|e.to_string())).collect::<Result<_>>()?;
    pins(&units,episode)
}
fn source(c:&Connection,source_sha:&str)->Result<()> {
    let current:String=c.query_row("SELECT sha256 FROM sources ORDER BY version DESC LIMIT 1",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if current!=source_sha {return Err(held("SOURCE_CHANGED"));}Ok(())
}
/// Returns the real current previous delivery plus its compact immutable pins.
fn checked_boundary(c:&Connection,boundary:&Value,source_sha:&str)->Result<Option<(Value,Value)>> {
    source(c,source_sha)?;
    match boundary["kind"].as_str() {
        Some("firstInScope")=>Ok(None),
        Some("missingPrevious")=>Err(held("PREVIOUS_NOT_SUPPLIED")),
        Some("supplied")=>{
            let episode=s(boundary,"episodeId");if episode.is_empty(){return Err(held("PREVIOUS_EPISODE_MISSING"));}
            let record=saved(c,episode)?;
            if record["episodeId"]!=episode || record["sourceSha256"]!=source_sha || record["finalDeliveryReady"]!=true
                || record["adoption"]["identity"]!=boundary["adoptionIdentity"] || record["adoption"]["version"]!=boundary["version"]
                || record["nativeSha256"]!=boundary["nativeSha256"] {
                return Err(held("PREVIOUS_CURRENT_ADOPTION_CHANGED"));
            }
            let a=adoption(c,&record["adoption"],source_sha)?;
            let candidate=artifacts::artifact(c,&a.candidate_id)?.ok_or_else(||held("PREVIOUS_CANDIDATE_MISSING"))?;
            let native=s(&candidate.artifact.payload,"text");
            if candidate.payload_sha256!=a.candidate_sha256 || crate::series_lite_edit::sha256(native.as_bytes())!=record["nativeSha256"]
                || boundary.get("previousBundleUtf8").is_some_and(|raw|raw.as_str()!=Some(native)) {
                return Err(held("PREVIOUS_NATIVE_CHANGED"));
            }
            let expected=pins(record["units"].as_array().ok_or_else(||held("PREVIOUS_UNITS_MISSING"))?,episode)?;
            if expected!=current_pins(c,episode)? {return Err(held("PREVIOUS_CURRENT_TEXT_CHANGED"));}
            let evidence=json!({"episodeId":episode,"adoption":a,"nativeSha256":record["nativeSha256"],"units":expected});
            Ok(Some((record,evidence)))
        },
        _=>Err(held("BOUNDARY_KIND_UNSUPPORTED")),
    }
}
pub(crate) fn verify_boundary(c:&Connection,record:&Value,source_sha:&str)->Result<()> {
    if let Some((previous,_))=checked_boundary(c,record,source_sha)? {verify_consumers(c,&previous)?;}
    Ok(())
}
pub(crate) fn register(tx:&Transaction<'_>,episode:&str,frozen_boundary:&Value,adopted:&Adoption,units:&[Value])->Result<()> {
    let actual=adoption(tx,&serde_json::to_value(adopted).map_err(|e|e.to_string())?,&adopted.source_sha256)?;
    let consumer_pins=pins(units,episode)?;
    if consumer_pins!=current_pins(tx,episode)? {return Err(held("CONSUMER_CURRENT_TEXT_CHANGED"));}
    let previous=checked_boundary(tx,frozen_boundary,&adopted.source_sha256)?;
    if let Some((record,_))=&previous {verify_consumers(tx,record)?;}
    let previous=previous.map(|(_,evidence)|evidence).unwrap_or(Value::Null);
    let value=json!({"version":VERSION,"episodeId":episode,"sourceSha256":actual.source_sha256,"adoption":actual,"units":consumer_pins,"boundary":frozen_boundary,"previous":previous});
    install(tx)?;
    if let Some(old)=proof(tx,&adopted.id)? {
        if old!=value {return Err(held("CONSUMER_REGISTRATION_CHANGED"));}return Ok(());
    }
    let raw=value.to_string();
    tx.execute("INSERT INTO series_lite_boundary_consumers_v1 VALUES(?1,?2,?3,?4)",params![adopted.id,episode,raw,digest(&raw)]).map_err(|e|e.to_string())?;
    Ok(())
}
/// Iterative verification follows only registered evidence. It does not infer
/// adjacency from source numbers, rewrite successors or reopen model requests.
pub(crate) fn verify_consumers(c:&Connection,record:&Value)->Result<()> {
    let mut current=record.clone();let mut seen=BTreeSet::new();
    for _ in 0..4096 {
        let id=s(&current["adoption"],"id");
        let Some(bound)=proof(c,id)? else {
            if !current["boundaryConsumersVersion"].is_null() {return Err(held("REGISTERED_CONSUMER_MISSING"));}
            return Ok(());
        };
        if id.is_empty() || !seen.insert(id.to_owned()) {return Err(held("CONSUMER_CYCLE"));}
        let episode=s(&current,"episodeId");let source_sha=s(&current,"sourceSha256");
        if bound["version"]!=VERSION || current["boundaryConsumersVersion"]!=VERSION || bound["episodeId"]!=episode
            || bound["sourceSha256"]!=source_sha || bound["adoption"]!=current["adoption"] || current["finalDeliveryReady"]!=true {
            return Err(held("CONSUMER_BINDING_CHANGED"));
        }
        adoption(c,&current["adoption"],source_sha)?;
        let expected=pins(current["units"].as_array().ok_or_else(||held("CONSUMER_UNITS_MISSING"))?,episode)?;
        if bound["units"]!=expected || current_pins(c,episode)?!=expected {return Err(held("CONSUMER_CURRENT_TEXT_CHANGED"));}
        match checked_boundary(c,&bound["boundary"],source_sha)? {
            Some((previous,evidence))=>{
                if evidence!=bound["previous"] {return Err(held("PREVIOUS_CONSUMPTION_CHANGED"));}
                current=previous;
            },
            None=>{
                if !bound["previous"].is_null(){return Err(held("UNEXPECTED_PREVIOUS_CONSUMER"));}return Ok(());
            },
        }
    }
    Err(held("CONSUMER_CHAIN_LIMIT"))
}

#[cfg(test)]
#[path="series_lite_consumers_tests.rs"]
mod tests;
