//! Explicit zero-dispatch recovery for art-only identity revisions.
use super::*;
use crate::pipeline::storyboard_refinement_identity as identity;
pub(super) fn packet(c: &rusqlite::Connection, input: &Value) -> Result<Option<Value>> {
    let mut ids = HashSet::new();
    let mut origins = Vec::new();
    for context in input["revisionContexts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["episodeId"] == input["episodeId"])
    {
        let id = s(context, "originWorkflowId");
        if !ids.insert(id.to_owned()) {
            continue;
        }
        let raw:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND result IS NOT NULL AND (entity_id='storyboard.run' OR (entity_id='storyboard.refine' AND state='completed' AND json_extract(prepared,'$.refinementAdoption.version')='storyboard-refinement-adoption/v1'))",[id],|r|r.get(0)).map_err(|_|"精修素材登记来源流程缺失")?;
        origins.push((id.to_owned(), crate::store::parse(raw)?));
    }
    identity::build(input, &origins)
}
pub(super) fn freeze(store: &Store, input: &mut Value) -> Result<()> {
    if let Some(packet) = packet(&store.conn()?, input)? {
        input[identity::FIELD] = packet;
    }
    Ok(())
}
/// Re-read authoritative origin bytes before replay; a self-sealed packet alone
/// is not evidence that its referenced accepted workflow still exists unchanged.
pub(super) fn verify(store: &Store, input: &Value) -> Result<()> {
    if input.get(identity::FIELD).is_some()
        && packet(&store.conn()?, input)?.as_ref() != input.get(identity::FIELD)
    {
        return Err("精修重绑定原登记来源已变化，原请求保持".into());
    }
    Ok(())
}
pub(super) fn prepare_resume(store: &Store, run: &str) -> Result<()> {
    let job = store.job(run)?;
    let snapshot = &job["snapshot"];
    if snapshot["stage"] != "storyboard.refine"
        || snapshot["input"].get(identity::FIELD).is_some()
        || job["prepared"].get(identity::RECOVERY).is_some()
    {
        return Ok(());
    }
    if job["status"] != "failed" || job["error"] != "素材使用锚或登记身份依据变化，未采用"
    {
        return Ok(());
    }
    if snapshot["runnerVersion"] != "desktop-runner/v1"
        || snapshot["input"]["settings"]["textProvider"] != "codex_sdk"
        || !job["result"].is_null()
    {
        return Err("精修重绑定恢复只允许原零派发SDK预检失败".into());
    }
    let input = &snapshot["input"];
    let expected = packet(&store.conn()?, input)?.ok_or("原失败没有可验证的美术版本重绑定")?;
    let current = super::refinement_input_with_capabilities(
        store,
        &json!({"episodeId":input["episodeId"],"refinementScope":input["refinementScope"],"reviewNotes":input["reviewNotes"],"preserveNotes":input["preserveNotes"]}),
        input.get("capabilities"),
    )?;
    for field in ["source", "assets", "existingUnits", "revisionContexts"] {
        if current[field] != input[field] {
            return Err("精修重绑定恢复事实已漂移，原请求保持".into());
        }
    }
    if current.get(identity::FIELD) != Some(&expected) {
        return Err("精修重绑定恢复来源已漂移".into());
    }
    let root = store.root.join("text-runs");
    if root.exists() {
        for item in std::fs::read_dir(root).map_err(|e| e.to_string())? {
            let p = item
                .map_err(|e| e.to_string())?
                .path()
                .join("sdk-attempt.json");
            if p.exists() && crate::text_transport::FrozenAttempt::load(&p)?.workflow_id == run {
                return Err("原精修已有冻结执行，不补入重绑定".into());
            }
        }
    }
    let mut c = store.conn()?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let current: (String, Option<String>) = tx
        .query_row(
            "SELECT snapshot,prepared FROM jobs WHERE id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if crate::store::parse(current.0.clone())? != *snapshot
        || current
            .1
            .as_ref()
            .map(|v| crate::store::parse(v.clone()))
            .transpose()?
            .unwrap_or(Value::Null)
            != job["prepared"]
    {
        return Err("原精修恢复前像已变化".into());
    }
    if packet(&tx, input)?.as_ref() != Some(&expected) {
        return Err("原精修登记来源在恢复期间变化".into());
    }
    let guard = job["prepared"]
        .get("publicationGuard")
        .unwrap_or(&snapshot["guard"]);
    crate::workflow::verify_guard(&tx, guard)?;
    let occupied:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1) OR EXISTS(SELECT 1 FROM jobs WHERE kind='text' AND (json_extract(prepared,'$.sdkOriginWorkflowId')=?1 OR json_extract(snapshot,'$.sdkSupervisorHandoff.workflowId')=?1 OR json_extract(snapshot,'$.workflowId')=?1))",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
    if occupied {
        return Err("原精修已有文字节点，不补入重绑定".into());
    }
    let recovery = identity::recovery(snapshot, &expected);
    let mut prepared = job["prepared"]
        .as_object()
        .cloned()
        .map(Value::Object)
        .unwrap_or(json!({}));
    prepared[identity::RECOVERY] = recovery.clone();
    let changed=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND kind='workflow' AND entity_id='storyboard.refine' AND state='failed' AND snapshot=?4 AND prepared IS ?5 AND result IS NULL AND error='素材使用锚或登记身份依据变化，未采用'",rusqlite::params![prepared.to_string(),now(),run,current.0,current.1]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("原精修恢复记录已变化".into());
    }
    tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'refinement_identity_rebinding_prepared',?2,?3)",rusqlite::params![run,json!({"recovery":recovery,"sameWorkflow":true,"modelCalls":0,"originalSnapshotUnchanged":true}).to_string(),now()]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
#[path = "engine_refinement_identity_tests.rs"]
mod tests;
