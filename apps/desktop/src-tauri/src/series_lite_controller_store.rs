//! Durable pins for the first author -> finisher -> independent reviewer chain.
//! No dispatcher, repair budget, model call, content verdict or adoption. Callers
//! own the project lease; installation and writes use their existing transaction.
use crate::pipeline::{digest, TextRequest};
use crate::series_lite_transport::{
    self as transport, Contributor, Role, RoleContext, TransportIdentity,
};
use crate::store::Result;
use crate::text_executor::VerifiedRawText;
use crate::workflow_artifacts::{self as artifacts, Artifact, Inputs, Kind};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const VERSION: &str = "series-lite-controller/v1";
const SLOT_TABLE: &str = "series_lite_role_slots_v1";
const RESULT_TABLE: &str = "series_lite_role_results_v1";
fn err(detail: &str) -> String {
    format!("LITE_CONTROLLER_HELD: {detail}")
}
fn hex(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn encode<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| e.to_string())
}
fn order(role: Role) -> u64 {
    match role {
        Role::Author => 1,
        Role::Finisher => 2,
        Role::Reviewer => 3,
    }
}
fn role_at(n: u64) -> Result<Role> {
    match n {
        1 => Ok(Role::Author),
        2 => Ok(Role::Finisher),
        3 => Ok(Role::Reviewer),
        _ => Err(err("三岗以外的修订必须先接问题预算，不新增隐式槽")),
    }
}
pub(crate) fn slot_id(workflow: &str, episode: &str, role: Role) -> String {
    format!(
        "lite-role:{}",
        digest(&json!([VERSION, workflow, episode, role]).to_string())
    )
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CandidatePin {
    pub artifact_id: String,
    pub payload_sha256: String,
    pub native_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FrozenRole {
    pub version: String,
    pub workflow_id: String,
    pub source_sha256: String,
    pub request: TextRequest,
    pub identity: TransportIdentity,
    pub context: RoleContext,
    /// Complete exact frozen baseline. Never serialize a numeric Value here.
    pub base_utf8: String,
    pub base_sha256: String,
    /// Role-neutral dependencies: source, supply, visualBaseline, rulesPack.
    pub common_inputs: Inputs,
    pub rule_snapshot_sha256: String,
    pub previous_candidate: Option<CandidatePin>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoleResult {
    pub slot_id: String,
    pub job_id: String,
    pub request_sha256: String,
    pub native_response_sha256: String,
    pub raw_artifact_id: String,
    pub raw_artifact_payload_sha256: String,
    pub candidate: Option<CandidatePin>,
}
#[derive(Debug)]
pub(crate) struct RoleRecord {
    pub slot_id: String,
    pub ordinal: u64,
    pub frozen: FrozenRole,
    pub result: Option<RoleResult>,
    /// Read through the bound job/step, never a caller-provided assertion.
    pub transport_state: Option<String>,
}
fn atomic<T>(tx: &Transaction<'_>, f: impl FnOnce() -> Result<T>) -> Result<T> {
    tx.execute_batch("SAVEPOINT lite_controller_write")
        .map_err(|e| e.to_string())?;
    match f() {
        Ok(v) => {
            tx.execute_batch("RELEASE lite_controller_write")
                .map_err(|e| e.to_string())?;
            Ok(v)
        }
        Err(e) => {
            tx.execute_batch("ROLLBACK TO lite_controller_write; RELEASE lite_controller_write")
                .map_err(|r| format!("{e}; {r}"))?;
            Err(e)
        }
    }
}
/// Explicit additive migration only. read/context_for never call this function.
pub(crate) fn install(tx: &Transaction<'_>) -> Result<()> {
    atomic(tx, || {
        artifacts::install(tx)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS series_lite_role_slots_v1(
            id TEXT PRIMARY KEY,workflow_id TEXT NOT NULL,episode TEXT NOT NULL,ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 3),role TEXT NOT NULL,data TEXT NOT NULL,data_sha256 TEXT NOT NULL,
            UNIQUE(workflow_id,episode,ordinal),UNIQUE(workflow_id,episode,role));
            CREATE TABLE IF NOT EXISTS series_lite_role_results_v1(slot_id TEXT PRIMARY KEY REFERENCES series_lite_role_slots_v1(id),job_id TEXT NOT NULL UNIQUE,data TEXT NOT NULL,data_sha256 TEXT NOT NULL);
            CREATE TRIGGER IF NOT EXISTS series_lite_role_slot_immutable_v1 BEFORE UPDATE ON series_lite_role_slots_v1 BEGIN SELECT RAISE(ABORT,'LITE_ROLE_IMMUTABLE'); END;
            CREATE TRIGGER IF NOT EXISTS series_lite_role_slot_no_delete_v1 BEFORE DELETE ON series_lite_role_slots_v1 BEGIN SELECT RAISE(ABORT,'LITE_ROLE_NO_DELETE'); END;
            CREATE TRIGGER IF NOT EXISTS series_lite_role_result_immutable_v1 BEFORE UPDATE ON series_lite_role_results_v1 BEGIN SELECT RAISE(ABORT,'LITE_ROLE_RESULT_IMMUTABLE'); END;
            CREATE TRIGGER IF NOT EXISTS series_lite_role_result_no_delete_v1 BEFORE DELETE ON series_lite_role_results_v1 BEGIN SELECT RAISE(ABORT,'LITE_ROLE_RESULT_NO_DELETE'); END;")
            .map_err(|e|e.to_string())
    })
}
fn installed(c: &Connection) -> Result<bool> {
    let n: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN (?1,?2)",
            params![SLOT_TABLE, RESULT_TABLE],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    match n {
        0 => Ok(false),
        2 => Ok(true),
        _ => Err(err("岗位账本迁移不完整")),
    }
}
fn parent(c: &Connection, run: &str, source: Option<&str>) -> Result<()> {
    let row: Option<(String, String, String)> = c
        .query_row(
            "SELECT kind,entity_id,snapshot FROM jobs WHERE id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (kind, entity, snapshot) = row.ok_or_else(|| err("父流程不存在"))?;
    let snapshot: Value = serde_json::from_str(&snapshot).map_err(|e| e.to_string())?;
    if kind != "workflow"
        || entity != "storyboard.run"
        || snapshot["input"]["liteControllerContract"] != VERSION
        || !snapshot["input"]["source"]["sha256"]
            .as_str()
            .is_some_and(hex)
        || source.is_some_and(|sha| snapshot["input"]["source"]["sha256"] != sha)
    {
        return Err(err("不是新建时冻结的 lite 父流程，禁止给历史任务补签"));
    }
    Ok(())
}
pub(crate) fn frozen_valid(v: &FrozenRole) -> Result<()> {
    if v.version != VERSION
        || v.workflow_id.trim().is_empty()
        || !hex(&v.source_sha256)
        || !hex(&v.base_sha256)
        || digest(&v.base_utf8) != v.base_sha256
        || v.request.stage != v.context.role.stage()
        || v.identity.role != v.context.role
        || digest(&v.request.rule_snapshot.to_string()) != v.rule_snapshot_sha256
        || v.common_inputs.iter().any(|(k, v)| k.is_empty() || !hex(v))
        || ["source", "supply", "visualBaseline", "rulesPack"]
            .iter()
            .any(|k| !v.common_inputs.contains_key(*k))
        || v.common_inputs["source"] != v.source_sha256
    {
        return Err(err("冻结岗位缺完整基稿、规则或共同供料证据"));
    }
    crate::series_lite_edit::parse(v.base_utf8.as_bytes())?;
    let mut input = v.request.input.clone();
    let marker = input
        .as_object_mut()
        .ok_or_else(|| err("请求输入无效"))?
        .remove("textExact")
        .ok_or_else(|| err("缺精确响应合同"))?;
    if marker != json!({"version":"series-lite-exact-text/v1","baseSha256":v.base_sha256}) {
        return Err(err("基稿和精确响应合同不匹配"));
    }
    transport::restore(&input, &v.identity, &v.context)?;
    Ok(())
}
fn load_slot(c: &Connection, id: &str) -> Result<Option<(u64, FrozenRole)>> {
    if !installed(c)? {
        return Ok(None);
    }
    let row:Option<(String,String,u64,String,String,String)>=c.query_row("SELECT workflow_id,episode,ordinal,role,data,data_sha256 FROM series_lite_role_slots_v1 WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|e.to_string())?;
    row.map(|(run, episode, ordinal, role, data, sha)| {
        if digest(&data) != sha {
            return Err(err("岗位冻结记录摘要变化"));
        }
        let frozen: FrozenRole = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        frozen_valid(&frozen)?;
        if frozen.workflow_id != run
            || frozen.context.episode != episode
            || frozen.context.role.as_str() != role
            || order(frozen.context.role) != ordinal
            || slot_id(&run, &episode, frozen.context.role) != id
        {
            return Err(err("岗位身份与持久索引不一致"));
        }
        Ok((ordinal, frozen))
    })
    .transpose()
}
fn load_result(c: &Connection, id: &str) -> Result<Option<RoleResult>> {
    if !installed(c)? {
        return Ok(None);
    }
    let row: Option<(String, String, String)> = c
        .query_row(
            "SELECT job_id,data,data_sha256 FROM series_lite_role_results_v1 WHERE slot_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    row.map(|(job, data, sha)| {
        let result: RoleResult = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        if digest(&data) != sha || result.slot_id != id || result.job_id != job {
            return Err(err("岗位成果记录摘要变化"));
        }
        Ok(result)
    })
    .transpose()
}
fn node_key(frozen: &FrozenRole) -> String {
    format!("{}:{}",frozen.request.stage,digest(&json!({"episode":frozen.context.episode,"iteration":frozen.context.iteration_id,"context":frozen.context.context_id}).to_string()))
}
pub(crate) fn bound_job(
    c: &Connection,
    frozen: &FrozenRole,
    expected: Option<&RoleResult>,
) -> Result<Option<(String, String, Value, Option<Value>)>> {
    let row:Option<(String,String,String,String,String,String,Option<String>,Option<String>,Option<String>,Option<String>)>=c.query_row(
        "SELECT j.id,j.state,j.snapshot,w.request_hash,w.state,j.provider,j.result,j.receipt,w.result,j.error FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND w.stage=?3 AND j.kind='text'",
        params![frozen.workflow_id,node_key(frozen),frozen.request.stage],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional().map_err(|e|e.to_string())?;
    let Some((
        id,
        state,
        snapshot_text,
        hash,
        step_state,
        provider,
        result,
        receipt,
        step_result,
        error,
    )) = row
    else {
        if expected.is_some() {
            return Err(err("原岗位 job/checkpoint 丢失"));
        }
        return Ok(None);
    };
    let snapshot: Value = serde_json::from_str(&snapshot_text).map_err(|e| e.to_string())?;
    let request: TextRequest =
        serde_json::from_value(snapshot.clone()).map_err(|e| e.to_string())?;
    if encode(&request)? != encode(&frozen.request)?
        || digest(&snapshot_text) != hash
        || !["codex", "codex_sdk"].contains(&provider.as_str())
        || expected.is_some_and(|v| v.job_id != id || v.request_sha256 != hash)
    {
        return Err(err("岗位原请求或任务绑定变化"));
    }
    let result: Option<Value> = result
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| e.to_string())?;
    if let Some(value) = result.as_ref() {
        let text = value["rawText"]
            .as_str()
            .ok_or_else(|| err("原响应不是精确文本封套"))?;
        if *value
            != json!({"version":"model-response-utf8/v1","rawText":text,"nativeTextSha256":digest(text),"byteLength":text.len()})
            || expected.is_some_and(|v| v.native_response_sha256 != digest(text))
        {
            return Err(err("原响应文本摘要变化"));
        }
        for (kind, wanted) in [
            (
                "text_received_raw",
                json!({"requestHash":hash,"value":value}),
            ),
            (
                "lite_exact_received",
                json!({"version":"series-lite-exact-text/v1","requestHash":hash,"nativeTextSha256":digest(text),"byteLength":text.len()}),
            ),
        ] {
            let event: String = c
                .query_row(
                    "SELECT data FROM events WHERE job_id=?1 AND type=?2 ORDER BY id DESC LIMIT 1",
                    params![id, kind],
                    |r| r.get(0),
                )
                .map_err(|_| err("原响应缺接收证据"))?;
            if serde_json::from_str::<Value>(&event).map_err(|e| e.to_string())? != wanted {
                return Err(err("原响应接收事件不一致"));
            }
        }
    } else if expected.is_some() {
        return Err(err("已登记岗位原响应丢失"));
    }
    if state == "completed" || step_state == "accepted" {
        let receipt: Value =
            serde_json::from_str(receipt.as_deref().ok_or_else(|| err("缺接受回执"))?)
                .map_err(|e| e.to_string())?;
        if state != "completed"
            || step_state != "accepted"
            || error.is_some()
            || receipt["contract"] != "desktop-runner/v1"
            || receipt["validation"] != "accepted"
            || result.is_none()
            || step_result
                .as_deref()
                .map(serde_json::from_str::<Value>)
                .transpose()
                .map_err(|e| e.to_string())?
                != result
        {
            return Err(err("岗位接受状态与原 job 不一致"));
        }
    }
    Ok(Some((id, state, snapshot, result)))
}
fn history(c: &Connection, run: &str, episode: &str) -> Result<Vec<RoleRecord>> {
    if !installed(c)? {
        return Ok(vec![]);
    }
    let mut q=c.prepare("SELECT id FROM series_lite_role_slots_v1 WHERE workflow_id=?1 AND episode=?2 ORDER BY ordinal").map_err(|e|e.to_string())?;
    let ids = q
        .query_map(params![run, episode], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    ids.iter()
        .enumerate()
        .map(|(i, id)| {
            let record = read(c, id)?.ok_or_else(|| err("岗位历史断链"))?;
            if record.ordinal != i as u64 + 1 {
                return Err(err("岗位历史跳步"));
            }
            Ok(record)
        })
        .collect()
}
/// Stable planned context allocated from ledger order, independent of accounts.
/// This does not prove distinct actual provider threads/sessions: the reviewer
/// verifier must inspect original runtime events for every creative contributor.
/// An existing slot returns its original context, including an unresolved one.
pub(crate) fn context_for(
    c: &Connection,
    run: &str,
    episode: &str,
    role: Role,
) -> Result<RoleContext> {
    parent(c, run, None)?;
    let rows = history(c, run, episode)?;
    let ordinal = order(role);
    if let Some(old) = rows.iter().find(|v| v.ordinal == ordinal) {
        return Ok(old.frozen.context.clone());
    }
    if rows.len() as u64 + 1 != ordinal {
        return Err(err("必须按主创→整编→独审推进"));
    }
    let mut prior = vec![];
    for row in &rows {
        if row.transport_state.as_deref() != Some("completed")
            || row
                .result
                .as_ref()
                .and_then(|r| r.candidate.as_ref())
                .is_none()
        {
            return Err(err(
                "前岗尚未完成原回包验收与候选保存，不新建后岗或绕过UNKNOWN",
            ));
        }
        prior.push(Contributor {
            role: row.frozen.context.role,
            context_id: row.frozen.context.context_id.clone(),
        });
    }
    let id = slot_id(run, episode, role);
    Ok(RoleContext {
        episode: episode.into(),
        role,
        iteration_id: format!("initial:{id}"),
        context_id: format!("context:{id}"),
        prior_contributors: prior,
    })
}
pub(crate) fn read(c: &Connection, id: &str) -> Result<Option<RoleRecord>> {
    let Some((ordinal, frozen)) = load_slot(c, id)? else {
        return Ok(None);
    };
    let result = load_result(c, id)?;
    if let Some(result) = &result {
        let raw = artifacts::artifact(c, &result.raw_artifact_id)?
            .ok_or_else(|| err("原回包成果丢失"))?;
        if raw.payload_sha256 != result.raw_artifact_payload_sha256
            || raw.artifact.payload["nativeSha256"] != result.native_response_sha256
        {
            return Err(err("原回包成果摘要变化"));
        }
        if let Some(pin) = &result.candidate {
            let candidate =
                artifacts::artifact(c, &pin.artifact_id)?.ok_or_else(|| err("候选成果丢失"))?;
            if candidate.payload_sha256 != pin.payload_sha256
                || candidate.artifact.payload["nativeSha256"] != pin.native_sha256
            {
                return Err(err("候选成果摘要变化"));
            }
        }
    }
    let transport_state = bound_job(c, &frozen, result.as_ref())?.map(|r| r.1);
    Ok(Some(RoleRecord {
        slot_id: id.into(),
        ordinal,
        frozen,
        result,
        transport_state,
    }))
}
pub(crate) fn read_role(
    c: &Connection,
    run: &str,
    episode: &str,
    role: Role,
) -> Result<Option<RoleRecord>> {
    read(c, &slot_id(run, episode, role))
}

/// Freeze before execute_exact. No account is added to controller slot identity.
pub(crate) fn reserve(tx: &Transaction<'_>, frozen: &FrozenRole) -> Result<RoleRecord> {
    if !installed(tx)? {
        return Err(err("需显式安装岗位账本"));
    }
    frozen_valid(frozen)?;
    parent(tx, &frozen.workflow_id, Some(&frozen.source_sha256))?;
    let id = slot_id(
        &frozen.workflow_id,
        &frozen.context.episode,
        frozen.context.role,
    );
    if let Some(old) = read(tx, &id)? {
        if encode(&old.frozen)? != encode(frozen)? {
            return Err(err("同岗位槽供料漂移，保留原请求/UNKNOWN，不重签"));
        }
        return Ok(old);
    }
    // This first-chain contract has no explicit new-creation-version allowance.
    // A different parent must not discard received work, race a queued slot, or
    // reset a rejection/repair budget. Resume the existing chain; intentional
    // recreation requires the separately versioned contract, not a new UUID.
    let mut pending = tx.prepare("SELECT id FROM series_lite_role_slots_v1 WHERE workflow_id<>?1 AND episode=?2 AND json_extract(data,'$.sourceSha256')=?3").map_err(|e|e.to_string())?;
    let prior_ids = pending
        .query_map(
            params![
                frozen.workflow_id,
                frozen.context.episode,
                frozen.source_sha256
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if let Some(prior_id) = prior_ids.first() {
        read(tx, prior_id)?.ok_or_else(|| err("原岗位丢失"))?;
        return Err(err(
            "同源同集已有原岗位；先恢复原流程，重新创作须明确的新版本合同",
        ));
    }
    let expected = context_for(
        tx,
        &frozen.workflow_id,
        &frozen.context.episode,
        frozen.context.role,
    )?;
    if expected != frozen.context {
        return Err(err("岗位上下文历史不是已存岗位实际顺序"));
    }
    if order(frozen.context.role) == 1 {
        if frozen.previous_candidate.is_some() {
            return Err(err("初次主创不得伪造前岗候选"));
        }
    } else {
        let previous = read_role(
            tx,
            &frozen.workflow_id,
            &frozen.context.episode,
            role_at(order(frozen.context.role) - 1)?,
        )?
        .ok_or_else(|| err("前岗丢失"))?;
        let pin = previous
            .result
            .and_then(|v| v.candidate)
            .ok_or_else(|| err("前岗候选未保存"))?;
        if frozen.previous_candidate.as_ref() != Some(&pin)
            || frozen.base_sha256 != pin.native_sha256
            || frozen.common_inputs != previous.frozen.common_inputs
            || frozen.source_sha256 != previous.frozen.source_sha256
        {
            return Err(err("前岗基稿或共同供料版本不一致"));
        }
    }
    // A pre-existing model node without a controller reservation cannot be
    // retroactively called a controlled lite dispatch.
    if bound_job(tx, frozen, None)?.is_some() {
        return Err(err("模型任务早于controller冻结记录，禁止追认"));
    }
    let data = encode(frozen)?;
    tx.execute(
        "INSERT INTO series_lite_role_slots_v1 VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            id,
            frozen.workflow_id,
            frozen.context.episode,
            order(frozen.context.role),
            frozen.context.role.as_str(),
            data,
            digest(&data)
        ],
    )
    .map_err(|e| e.to_string())?;
    read(tx, &id)?.ok_or_else(|| err("冻结记录不可读取"))
}

/// Persist receipt/candidate BEFORE transport accept; either order can be
/// resumed idempotently. This records evidence, never validates creative quality.
/// Reviewer calls candidate=None: a valid negative review is still a received
/// review, and only the separate reviewer adapter can interpret its verdict.
pub(crate) fn record_candidate(
    tx: &Transaction<'_>,
    id: &str,
    raw: &VerifiedRawText,
    candidate: Option<&[u8]>,
) -> Result<RoleRecord> {
    atomic(tx, || {
        let slot = read(tx, id)?.ok_or_else(|| err("岗位尚未冻结"))?;
        let frozen = &slot.frozen;
        let (job, state, snapshot, envelope) =
            bound_job(tx, frozen, None)?.ok_or_else(|| err("没有原岗位任务"))?;
        let envelope = envelope.ok_or_else(|| err("岗位没有已接收原文本"))?;
        if !["received", "completed", "rejected"].contains(&state.as_str())
            || job != raw.job_id()
            || digest(&snapshot.to_string()) != raw.request_sha256()
            || frozen.identity.packet_file_sha256 != raw.packet_sha256()
            || envelope != *raw.transport_envelope()
            || digest(std::str::from_utf8(raw.bytes()).map_err(|_| err("原文本不是UTF8"))?)
                != raw.sha256()
        {
            return Err(err("VerifiedRawText不属于当前原槽/原job/原包"));
        }
        let review = frozen.context.role == Role::Reviewer;
        if review != candidate.is_none() {
            return Err(err("创作岗须存候选，独审岗不得另造创作候选"));
        }
        let author_attempt = if review {
            let prior = read_role(
                tx,
                &frozen.workflow_id,
                &frozen.context.episode,
                Role::Finisher,
            )?
            .ok_or_else(|| err("整编前岗丢失"))?;
            prior.result.ok_or_else(|| err("整编成果丢失"))?.job_id
        } else {
            job.clone()
        };
        let native = std::str::from_utf8(raw.bytes()).map_err(|_| err("原文本不是UTF8"))?;
        let mut payload = json!({"version":"lite-role-response/v1","encoding":"utf8","text":native,"nativeSha256":raw.sha256(),"slotId":id,"jobId":job,"requestSha256":raw.request_sha256(),"packetSha256":raw.packet_sha256(),"context":frozen.context,"actualContextIndependence":"NOT_VERIFIED","contentReview":"NOT_PROVEN","adoptionGranted":false});
        if review {
            let pin = frozen
                .previous_candidate
                .as_ref()
                .ok_or_else(|| err("独审缺当前候选"))?;
            payload["candidateSha256"] = json!(pin.payload_sha256);
            payload["candidateNativeSha256"] = json!(pin.native_sha256);
            payload["inputSha256"] = json!(digest(&encode(&frozen.common_inputs)?));
            payload["authorAttempt"] = json!(author_attempt);
            payload["reviewerAttempt"] = json!(job);
        }
        let raw_artifact = artifacts::put(
            tx,
            &Artifact {
                id: format!("{id}:raw"),
                identity: format!(
                    "lite-episode:{}:{}",
                    frozen.workflow_id, frozen.context.episode
                ),
                kind: if review { Kind::ReviewRaw } else { Kind::Raw },
                contract_version: VERSION.into(),
                source_sha256: frozen.source_sha256.clone(),
                inputs: frozen.common_inputs.clone(),
                producer: format!("{}:exact-receive", frozen.request.stage),
                author_attempt: job.clone(),
                parents: vec![],
                payload,
            },
        )?;
        let candidate_pin = if let Some(bytes) = candidate {
            let text = std::str::from_utf8(bytes).map_err(|_| err("候选不是UTF8"))?;
            crate::series_lite_edit::parse(bytes)?;
            let native_sha = digest(text);
            let artifact = artifacts::put(
                tx,
                &Artifact {
                    id: format!("{id}:candidate"),
                    identity: raw_artifact.artifact.identity.clone(),
                    kind: Kind::Candidate,
                    contract_version: VERSION.into(),
                    source_sha256: frozen.source_sha256.clone(),
                    inputs: frozen.common_inputs.clone(),
                    producer: "lite-controller-unvalidated-candidate/v1".into(),
                    author_attempt: job.clone(),
                    parents: vec![raw_artifact.artifact.id.clone()],
                    payload: json!({"version":"lite-candidate-utf8/v1","encoding":"utf8","text":text,"nativeSha256":native_sha,"baseSha256":frozen.base_sha256,"previousCandidate":frozen.previous_candidate,"validation":"NOT_PROVEN"}),
                },
            )?;
            Some(CandidatePin {
                artifact_id: artifact.artifact.id,
                payload_sha256: artifact.payload_sha256,
                native_sha256: native_sha,
            })
        } else {
            None
        };
        let result = RoleResult {
            slot_id: id.into(),
            job_id: job,
            native_response_sha256: raw.sha256().into(),
            request_sha256: raw.request_sha256().into(),
            raw_artifact_id: raw_artifact.artifact.id,
            raw_artifact_payload_sha256: raw_artifact.payload_sha256,
            candidate: candidate_pin,
        };
        if let Some(old) = slot.result {
            if old != result {
                return Err(err("岗位成果已存在且不能覆盖"));
            }
        } else {
            let data = encode(&result)?;
            tx.execute(
                "INSERT INTO series_lite_role_results_v1 VALUES(?1,?2,?3,?4)",
                params![id, result.job_id, data, digest(&data)],
            )
            .map_err(|e| e.to_string())?;
        }
        read(tx, id)?.ok_or_else(|| err("岗位成果保存后无法核对"))
    })
}

#[cfg(test)]
#[path = "series_lite_controller_store_tests.rs"]
mod tests;
