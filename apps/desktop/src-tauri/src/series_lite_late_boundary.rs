//! One explicit missing->supplied boundary transition. Does not edit the
//! original preparation, creative input pins, candidate or previous review.
use crate::pipeline::digest;
use crate::series_lite_edit::sha256;
use crate::store::{s,Store};
use rusqlite::{params,Connection,OptionalExtension};
use serde_json::{json,Value};
use std::path::Path;
type Result<T> = std::result::Result<T,String>;
const VERSION:&str="lite-late-boundary/v1";
pub(crate) fn load(c:&Connection,run:&str,episode:&str)->Result<Option<Value>> {
    let installed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='series_lite_late_boundary_v1')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !installed{return Ok(None)}
    let raw:Option<(String,String)>=c.query_row("SELECT data,sha256 FROM series_lite_late_boundary_v1 WHERE run=?1 AND episode=?2",params![run,episode],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    raw.map(|(body,pin)| {
        if sha256(body.as_bytes())!=pin{return Err("LITE_LATE_BOUNDARY_DRIFT".into())}
        let record:Value=serde_json::from_str(&body).map_err(|e|e.to_string())?;
        if record["version"]!=VERSION||record["run"]!=run||record["episodeId"]!=episode{return Err("LITE_LATE_BOUNDARY_IDENTITY".into())}
        Ok(record)
    }).transpose()
}
pub(crate) fn freeze(store:&Store,run:&str,episode:&str,root:&Path,original:&Value,supplied:&Value,source_sha:&str)->Result<Value> {
    if original["kind"]!="missingPrevious"||supplied["kind"]!="supplied"{return Err("LITE_LATE_BOUNDARY_TRANSITION".into())}
    let source=store.source()?;
    let episodes=source["episodes"].as_array().ok_or("LITE_LATE_BOUNDARY_SOURCE")?;
    let index=episodes.iter().position(|e|e["id"]==episode).ok_or("LITE_LATE_BOUNDARY_EPISODE")?;
    if source["sha256"]!=source_sha || index==0 || episodes[index-1]["id"]!=supplied["episodeId"] || original["episodeId"]!=supplied["episodeId"] {
        return Err("LITE_LATE_BOUNDARY_NOT_DIRECT_PREDECESSOR".into())
    }
    let previous=crate::series_lite_edit::parse(s(supplied,"previousBundleUtf8").as_bytes())?;
    let tail=previous.get("episodes")?.array()?.first().ok_or("LITE_PREVIOUS_EPISODE_MISSING")?.get("draft")?.get("units")?.array()?.last().ok_or("LITE_PREVIOUS_TAIL_MISSING")?;
    let tail=crate::series_lite_edit::canonical(tail);
    let body=json!({"version":VERSION,"run":run,"episodeId":episode,"sourceSha256":source_sha,"originalBoundarySha256":digest(&original.to_string()),"boundary":supplied});
    let text=body.to_string();
    let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
    crate::series_lite_consumers::verify_boundary(&tx,supplied,source_sha)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS series_lite_late_boundary_v1(run TEXT NOT NULL,episode TEXT NOT NULL,data TEXT NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(run,episode));
      CREATE TRIGGER IF NOT EXISTS lite_late_boundary_no_update BEFORE UPDATE ON series_lite_late_boundary_v1 BEGIN SELECT RAISE(ABORT,'LITE_LATE_BOUNDARY_IMMUTABLE'); END;
      CREATE TRIGGER IF NOT EXISTS lite_late_boundary_no_delete BEFORE DELETE ON series_lite_late_boundary_v1 BEGIN SELECT RAISE(ABORT,'LITE_LATE_BOUNDARY_IMMUTABLE'); END;") .map_err(|e|e.to_string())?;
    if let Some(old)=load(&tx,run,episode)? {
        if old!=body{return Err("LITE_LATE_BOUNDARY_ALREADY_FROZEN".into())}
    }else{
        tx.execute("INSERT INTO series_lite_late_boundary_v1 VALUES(?1,?2,?3,?4)",params![run,episode,text,sha256(text.as_bytes())]).map_err(|e|e.to_string())?;
    }
    tx.commit().map_err(|e|e.to_string())?;
    // Exact full previous native bundle stays SQL-bound. Only its real tail is
    // added to the role packet, alongside the complete current source/draft.
    crate::series_lite_desktop_runtime::put_exact(root,"late-previous-boundary.json",&tail)?;
    Ok(body)
}
