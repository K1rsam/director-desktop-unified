//! Bounded append-only role history for explicitly versioned lite v2 runs.
//! Transport acceptance preserves evidence; it grants no creative approval.
use crate::pipeline::digest;
use crate::series_lite_controller_store::{
    self as v1, CandidatePin, FrozenRole, RoleRecord, RoleResult,
};
use crate::series_lite_review as review;
use crate::series_lite_transport::{Contributor, Role, RoleContext};
use crate::store::Result;
use crate::text_executor::VerifiedRawText;
use crate::workflow_artifacts::{self as artifacts, Artifact, Kind};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) const VERSION: &str = "series-lite-runtime/v2";
pub(crate) const MAX_SLOTS: u64 = 24;
const SLOTS: &str = "series_lite_role_slots_v2";
const RESULTS: &str = "series_lite_role_results_v2";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RevisionBinding {
    pub cause: String,
    pub ordinal: u64,
    pub attempt_key: String,
}
#[derive(Debug)]
pub(crate) struct Slot {
    pub record: RoleRecord,
    pub phase: String,
    pub prior_issues: Vec<review::Issue>,
    pub revision: Option<RevisionBinding>,
    pub invalid: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenSlot {
    frozen: FrozenRole,
    phase: String,
    prior_issues: Vec<review::Issue>,
    revision: Option<RevisionBinding>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedResult {
    result: RoleResult,
    invalid: Option<String>,
}

fn held(message: &str) -> String {
    format!("LITE_CONTROLLER_V2_HELD: {message}")
}
fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(held(message))
    }
}
fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}
fn installed(c: &Connection) -> Result<bool> {
    let n: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN (?1,?2)",
            params![SLOTS, RESULTS],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    match n {
        0 => Ok(false),
        2 => Ok(true),
        _ => Err(held("INCOMPLETE_INSTALL")),
    }
}
fn atomic<T>(tx: &Transaction<'_>, f: impl FnOnce() -> Result<T>) -> Result<T> {
    tx.execute_batch("SAVEPOINT lite_controller_v2_write")
        .map_err(|e| e.to_string())?;
    match f() {
        Ok(v) => {
            tx.execute_batch("RELEASE lite_controller_v2_write")
                .map_err(|e| e.to_string())?;
            Ok(v)
        }
        Err(e) => {
            tx.execute_batch(
                "ROLLBACK TO lite_controller_v2_write; RELEASE lite_controller_v2_write",
            )
            .map_err(|r| format!("{e}; {r}"))?;
            Err(e)
        }
    }
}
pub(crate) fn install(tx: &Transaction<'_>) -> Result<()> {
    atomic(tx, || {
        artifacts::install(tx)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS series_lite_role_slots_v2(
          id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL, episode TEXT NOT NULL,
          ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 24), role TEXT NOT NULL,
          data TEXT NOT NULL, data_sha256 TEXT NOT NULL, UNIQUE(workflow_id,episode,ordinal));
          CREATE TABLE IF NOT EXISTS series_lite_role_results_v2(
          slot_id TEXT PRIMARY KEY REFERENCES series_lite_role_slots_v2(id),job_id TEXT NOT NULL UNIQUE,data TEXT NOT NULL,data_sha256 TEXT NOT NULL);
          CREATE UNIQUE INDEX IF NOT EXISTS series_lite_role_revision_attempt_v2 ON series_lite_role_slots_v2(json_extract(data,'$.revision.attemptKey')) WHERE json_type(data,'$.revision')='object';
          CREATE TRIGGER IF NOT EXISTS series_lite_role_slots_v2_no_update BEFORE UPDATE ON series_lite_role_slots_v2 BEGIN SELECT RAISE(ABORT,'LITE_V2_FROZEN'); END;
          CREATE TRIGGER IF NOT EXISTS series_lite_role_slots_v2_no_delete BEFORE DELETE ON series_lite_role_slots_v2 BEGIN SELECT RAISE(ABORT,'LITE_V2_FROZEN'); END;
          CREATE TRIGGER IF NOT EXISTS series_lite_role_results_v2_no_update BEFORE UPDATE ON series_lite_role_results_v2 BEGIN SELECT RAISE(ABORT,'LITE_V2_RESULT_FROZEN'); END;
          CREATE TRIGGER IF NOT EXISTS series_lite_role_results_v2_no_delete BEFORE DELETE ON series_lite_role_results_v2 BEGIN SELECT RAISE(ABORT,'LITE_V2_RESULT_FROZEN'); END;") .map_err(|e|e.to_string())
    })
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
    let (kind, entity, body) = row.ok_or_else(|| held("PARENT_MISSING"))?;
    let snapshot: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let sha = snapshot["input"]["source"]["sha256"].as_str().unwrap_or("");
    need(
        kind == "workflow"
            && entity == "storyboard.run"
            && snapshot["input"]["liteControllerContract"] == v1::VERSION
            && snapshot["input"]["liteRuntimeContract"] == VERSION
            && sha.len() == 64
            && sha.bytes().all(|b| b.is_ascii_hexdigit())
            && source.is_none_or(|expected| expected == sha),
        "PARENT_NOT_V2_OR_SOURCE_CHANGED",
    )
}
fn slot_id(run: &str, episode: &str, ordinal: u64) -> String {
    format!(
        "lite-role-v2:{}",
        digest(&json!([VERSION, run, episode, ordinal]).to_string())
    )
}
fn planned(run: &str, episode: &str, ordinal: u64, role: Role, prior: &[Slot]) -> RoleContext {
    let id = slot_id(run, episode, ordinal);
    RoleContext {
        episode: episode.into(),
        role,
        iteration_id: format!("iteration:{id}"),
        context_id: format!("context:{id}"),
        prior_contributors: prior
            .iter()
            .filter(|s| s.record.frozen.context.role != Role::Reviewer)
            .map(|s| Contributor {
                role: s.record.frozen.context.role,
                context_id: s.record.frozen.context.context_id.clone(),
            })
            .collect(),
    }
}
fn issue_shape(issues: &[review::Issue]) -> Result<()> {
    let mut seen = BTreeSet::new();
    need(
        issues.len() <= 256
            && issues.iter().all(|i| {
                !i.id.trim().is_empty()
                    && seen.insert(i.id.as_str())
                    && !i.code.trim().is_empty()
                    && !i.detail.trim().is_empty()
                    && !i.impact.trim().is_empty()
            }),
        "PRIOR_ISSUES_INVALID",
    )
}
fn revision_valid(
    c: &Connection,
    frozen: &FrozenRole,
    binding: &RevisionBinding,
    fresh: bool,
) -> Result<()> {
    need(
        (1..=2).contains(&binding.ordinal) && !binding.attempt_key.is_empty(),
        "REVISION_BINDING_INVALID",
    )?;
    let row:Option<(String,String)>=c.query_row("SELECT request_json,state FROM workflow_issue_attempt_v1 WHERE cause=?1 AND ordinal=?2 AND attempt_key=?3",params![binding.cause,binding.ordinal,binding.attempt_key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|held("REVISION_BUDGET_NOT_RESERVED"))?;
    let (body, state) = row.ok_or_else(|| held("REVISION_BUDGET_NOT_RESERVED"))?;
    let attempt: crate::workflow_issues::Attempt =
        serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let ledger = crate::workflow_issues::read(c, &binding.cause)?
        .ok_or_else(|| held("REVISION_CAUSE_MISSING"))?;
    need(
        attempt.attempt_key == binding.attempt_key
            && attempt.workflow_id == frozen.workflow_id
            && attempt.base_sha256 == frozen.base_sha256
            && attempt.request_sha256 == digest(&encode(&frozen.request)?)
            && attempt.repair == crate::workflow_issues::Repair::AiRevision
            && ledger.issue["sourceSha256"] == frozen.source_sha256
            && ledger.attempts >= binding.ordinal
            && (!fresh || state == "reserved"),
        "REVISION_BUDGET_BINDING_OR_UNKNOWN",
    )
}
fn latest_pin(prior: &[Slot]) -> Option<&CandidatePin> {
    prior
        .iter()
        .rev()
        .find_map(|s| s.record.result.as_ref().and_then(|r| r.candidate.as_ref()))
}
fn settled(slot: &Slot) -> bool {
    slot.record.result.is_some()
        && match slot.invalid {
            Some(_) => slot.record.transport_state.as_deref() == Some("rejected"),
            None => slot.record.transport_state.as_deref() == Some("completed"),
        }
}
fn base_valid(
    prior: &[Slot],
    frozen: &FrozenRole,
    revision: Option<&RevisionBinding>,
) -> Result<()> {
    if let Some(first) = prior.first() {
        need(
            first.record.frozen.source_sha256 == frozen.source_sha256
                && first.record.frozen.common_inputs == frozen.common_inputs,
            "COMMON_INPUTS_CHANGED",
        )?;
    }
    if let Some(pin) = latest_pin(prior) {
        need(
            frozen.previous_candidate.as_ref() == Some(pin)
                && frozen.base_sha256 == pin.native_sha256,
            "LATEST_CANDIDATE_NOT_FROZEN_BASE",
        )
    } else if prior.is_empty() {
        need(
            frozen.context.role == Role::Author
                && frozen.previous_candidate.is_none()
                && revision.is_none(),
            "INITIAL_AUTHOR_ONLY",
        )
    } else {
        // The first author can return unusable JSON. A budgeted repair starts
        // from its original program-owned baseline, never invented output.
        need(
            revision.is_some()
                && frozen.context.role == Role::Author
                && frozen.previous_candidate.is_none()
                && frozen.base_utf8 == prior[0].record.frozen.base_utf8
                && prior
                    .iter()
                    .all(|s| s.invalid.is_some() && s.record.result.is_some()),
            "NO_CANDIDATE_REQUIRES_PROVEN_INITIAL_AUTHOR_REPAIR",
        )
    }
}
fn verify_artifacts(
    c: &Connection,
    slot: &RoleRecord,
    invalid: Option<&str>,
    envelope: &Value,
    phase: &str,
    revision: Option<&RevisionBinding>,
) -> Result<()> {
    let result = slot.result.as_ref().ok_or_else(|| held("RESULT_MISSING"))?;
    let frozen = &slot.frozen;
    let raw = artifacts::artifact(c, &result.raw_artifact_id)?
        .ok_or_else(|| held("RAW_ARTIFACT_MISSING"))?;
    let p = &raw.artifact.payload;
    let text = p["text"].as_str().ok_or_else(|| held("RAW_UTF8_MISSING"))?;
    need(
        raw.payload_sha256 == result.raw_artifact_payload_sha256
            && digest(text) == result.native_response_sha256
            && p["nativeSha256"] == result.native_response_sha256
            && envelope["rawText"] == text
            && p["slotId"] == slot.slot_id
            && p["jobId"] == result.job_id
            && p["requestSha256"] == result.request_sha256
            && p["packetSha256"] == frozen.identity.packet_file_sha256
            && p["context"] == serde_json::to_value(&frozen.context).map_err(|e| e.to_string())?
            && p["invalid"] == json!(invalid)
            && p["phase"] == phase
            && p["revision"] == json!(revision)
            && p["contentReview"] == "NOT_PROVEN"
            && p["adoptionGranted"] == false
            && raw.artifact.contract_version == VERSION
            && raw.artifact.source_sha256 == frozen.source_sha256
            && raw.artifact.inputs == frozen.common_inputs
            && raw.artifact.author_attempt == result.job_id
            && raw.artifact.kind
                == if frozen.context.role == Role::Reviewer {
                    Kind::ReviewRaw
                } else {
                    Kind::Raw
                },
        "RAW_ARTIFACT_BINDING_CHANGED",
    )?;
    if let Some(pin) = &result.candidate {
        let candidate = artifacts::artifact(c, &pin.artifact_id)?
            .ok_or_else(|| held("CANDIDATE_ARTIFACT_MISSING"))?;
        let p = &candidate.artifact.payload;
        let text = p["text"]
            .as_str()
            .ok_or_else(|| held("CANDIDATE_UTF8_MISSING"))?;
        crate::series_lite_edit::parse(text.as_bytes())?;
        need(
            candidate.payload_sha256 == pin.payload_sha256
                && digest(text) == pin.native_sha256
                && p["nativeSha256"] == pin.native_sha256
                && p["baseSha256"] == frozen.base_sha256
                && p["previousCandidate"] == json!(frozen.previous_candidate)
                && candidate.artifact.parents == vec![result.raw_artifact_id.clone()]
                && candidate.artifact.kind == Kind::Candidate
                && candidate.artifact.contract_version == VERSION
                && candidate.artifact.source_sha256 == frozen.source_sha256
                && candidate.artifact.inputs == frozen.common_inputs
                && candidate.artifact.author_attempt == result.job_id
                && p["validation"] == "NOT_PROVEN",
            "CANDIDATE_ARTIFACT_BINDING_CHANGED",
        )?;
    }
    need(
        if frozen.context.role == Role::Reviewer {
            result.candidate.is_none()
        } else {
            result.candidate.is_some() || invalid.is_some()
        },
        "CANDIDATE_OR_INVALID_REQUIRED",
    )
}
fn verify_rejection(c: &Connection, slot: &RoleRecord, invalid: Option<&str>) -> Result<()> {
    if slot.transport_state.as_deref() != Some("rejected") {
        return Ok(());
    }
    let result = slot
        .result
        .as_ref()
        .ok_or_else(|| held("REJECTED_RESULT_MISSING"))?;
    let (receipt,error,step_state,step_error,step_result):(String,String,String,String,String)=c.query_row(
        "SELECT j.receipt,j.error,w.state,w.error,w.result FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE j.id=?1 AND w.workflow_id=?2 AND w.stage=?3",
        params![result.job_id,slot.frozen.workflow_id,slot.frozen.request.stage],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|held("REJECTION_RECEIPT_MISSING"))?;
    let receipt: Value = serde_json::from_str(&receipt).map_err(|e| e.to_string())?;
    let step_result: Value = serde_json::from_str(&step_result).map_err(|e| e.to_string())?;
    need(
        invalid == Some(error.as_str())
            && step_error == error
            && step_state == "rejected"
            && receipt["contract"] == "desktop-runner/v1"
            && receipt["validation"] == "rejected"
            && step_result["nativeTextSha256"] == result.native_response_sha256,
        "REJECTION_RECEIPT_BINDING_CHANGED",
    )
}
/// Full source-bound read; old databases are observed without DDL.
pub(crate) fn history(c: &Connection, run: &str, episode: &str) -> Result<Vec<Slot>> {
    if !installed(c)? {
        return Ok(vec![]);
    }
    parent(c, run, None)?;
    let mut q=c.prepare("SELECT id,ordinal,role,data,data_sha256 FROM series_lite_role_slots_v2 WHERE workflow_id=?1 AND episode=?2 ORDER BY ordinal").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![run, episode], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    need(rows.len() <= MAX_SLOTS as usize, "ROLE_LIMIT_EXCEEDED")?;
    let mut out: Vec<Slot> = vec![];
    for (id, ordinal, role, body, hash) in rows {
        need(
            ordinal == out.len() as u64 + 1
                && id == slot_id(run, episode, ordinal)
                && digest(&body) == hash,
            "ROLE_SEQUENCE_OR_DIGEST_CHANGED",
        )?;
        let data: FrozenSlot = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let frozen = data.frozen;
        v1::frozen_valid(&frozen)?;
        parent(c, run, Some(&frozen.source_sha256))?;
        issue_shape(&data.prior_issues)?;
        need(
            frozen.workflow_id == run
                && frozen.context.episode == episode
                && frozen.context.role.as_str() == role
                && frozen.context == planned(run, episode, ordinal, frozen.context.role, &out)
                && !data.phase.trim().is_empty(),
            "FROZEN_ROLE_IDENTITY_CHANGED",
        )?;
        if let Some(last) = out.last() {
            need(settled(last), "PRIOR_ROLE_NOT_SETTLED")?;
        }
        base_valid(&out, &frozen, data.revision.as_ref())?;
        if let Some(binding) = &data.revision {
            revision_valid(c, &frozen, binding, false)?;
        }
        let saved: Option<(String, String, String)> = c
            .query_row(
                "SELECT job_id,data,data_sha256 FROM series_lite_role_results_v2 WHERE slot_id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (result, invalid) = if let Some((job, body, hash)) = saved {
            need(digest(&body) == hash, "RESULT_DIGEST_CHANGED")?;
            let saved: SavedResult = serde_json::from_str(&body).map_err(|e| e.to_string())?;
            need(
                saved.result.slot_id == id
                    && saved.result.job_id == job
                    && saved.invalid.as_ref().is_none_or(|v| !v.trim().is_empty()),
                "RESULT_IDENTITY_CHANGED",
            )?;
            (Some(saved.result), saved.invalid)
        } else {
            (None, None)
        };
        let bound = v1::bound_job(c, &frozen, result.as_ref())?;
        let state = bound.as_ref().map(|b| b.1.clone());
        let record = RoleRecord {
            slot_id: id,
            ordinal,
            frozen,
            result,
            transport_state: state,
        };
        if record.result.is_some() {
            let envelope = bound
                .as_ref()
                .and_then(|b| b.3.as_ref())
                .ok_or_else(|| held("ORIGINAL_ENVELOPE_MISSING"))?;
            verify_artifacts(
                c,
                &record,
                invalid.as_deref(),
                envelope,
                &data.phase,
                data.revision.as_ref(),
            )?;
            verify_rejection(c, &record, invalid.as_deref())?;
        }
        out.push(Slot {
            record,
            phase: data.phase,
            prior_issues: data.prior_issues,
            revision: data.revision,
            invalid,
        });
    }
    Ok(out)
}
pub(crate) fn context_for(
    c: &Connection,
    run: &str,
    episode: &str,
    role: Role,
) -> Result<RoleContext> {
    parent(c, run, None)?;
    let prior = history(c, run, episode)?;
    if let Some(last) = prior.last() {
        if !settled(last) {
            need(
                last.record.frozen.context.role == role,
                "PRIOR_RESULT_OR_UNKNOWN_RECONCILE_FIRST",
            )?;
            return Ok(last.record.frozen.context.clone());
        }
    } else {
        need(role == Role::Author, "INITIAL_AUTHOR_ONLY")?;
    }
    need(prior.len() < MAX_SLOTS as usize, "ROLE_LIMIT_EXCEEDED")?;
    Ok(planned(run, episode, prior.len() as u64 + 1, role, &prior))
}
pub(crate) fn reserve(
    tx: &Transaction<'_>,
    frozen: &FrozenRole,
    phase: &str,
    prior_issues: &[review::Issue],
    revision: Option<RevisionBinding>,
) -> Result<Slot> {
    atomic(tx, || {
        need(installed(tx)?, "EXPLICIT_INSTALL_REQUIRED")?;
        v1::frozen_valid(frozen)?;
        parent(tx, &frozen.workflow_id, Some(&frozen.source_sha256))?;
        need(
            !phase.trim().is_empty() && phase.len() <= 128 && !phase.contains('\0'),
            "PHASE_INVALID",
        )?;
        issue_shape(prior_issues)?;
        let rows = history(tx, &frozen.workflow_id, &frozen.context.episode)?;
        let data = FrozenSlot {
            frozen: frozen.clone(),
            phase: phase.into(),
            prior_issues: prior_issues.to_vec(),
            revision: revision.clone(),
        };
        let body = encode(&data)?;
        if let Some(old) = rows
            .iter()
            .find(|s| s.record.frozen.context.iteration_id == frozen.context.iteration_id)
        {
            let original = FrozenSlot {
                frozen: old.record.frozen.clone(),
                phase: old.phase.clone(),
                prior_issues: old.prior_issues.clone(),
                revision: old.revision.clone(),
            };
            need(encode(&original)? == body, "ORIGINAL_SLOT_INPUT_DRIFT")?;
            let ordinal = old.record.ordinal;
            return history(tx, &frozen.workflow_id, &frozen.context.episode)?
                .into_iter()
                .find(|s| s.record.ordinal == ordinal)
                .ok_or_else(|| held("ORIGINAL_SLOT_MISSING"));
        }
        need(rows.len() < MAX_SLOTS as usize, "ROLE_LIMIT_EXCEEDED")?;
        if let Some(last) = rows.last() {
            need(settled(last), "PRIOR_RESULT_OR_UNKNOWN_RECONCILE_FIRST")?;
        }
        let ordinal = rows.len() as u64 + 1;
        need(
            frozen.context
                == planned(
                    &frozen.workflow_id,
                    &frozen.context.episode,
                    ordinal,
                    frozen.context.role,
                    &rows,
                ),
            "CONTRIBUTOR_HISTORY_CHANGED",
        )?;
        base_valid(&rows, frozen, revision.as_ref())?;
        if frozen.context.role != Role::Reviewer
            && rows
                .iter()
                .any(|s| s.record.frozen.context.role == frozen.context.role)
        {
            need(
                revision.is_some(),
                "REPEATED_CREATIVE_ROLE_REQUIRES_REVISION_BUDGET",
            )?;
        }
        if let Some(binding) = &revision {
            revision_valid(tx, frozen, binding, true)?;
        }
        let prior:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM series_lite_role_slots_v2 WHERE workflow_id<>?1 AND episode=?2 AND json_extract(data,'$.frozen.sourceSha256')=?3)",params![frozen.workflow_id,frozen.context.episode,frozen.source_sha256],|r|r.get(0)).map_err(|e|e.to_string())?;
        need(!prior, "ORIGINAL_SAME_SOURCE_CHAIN_MUST_RESUME")?;
        need(
            v1::bound_job(tx, frozen, None)?.is_none(),
            "MODEL_JOB_PRECEDES_FROZEN_SLOT",
        )?;
        let id = slot_id(&frozen.workflow_id, &frozen.context.episode, ordinal);
        tx.execute(
            "INSERT INTO series_lite_role_slots_v2 VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                frozen.workflow_id,
                frozen.context.episode,
                ordinal,
                frozen.context.role.as_str(),
                body,
                digest(&body)
            ],
        )
        .map_err(|e| e.to_string())?;
        history(tx, &frozen.workflow_id, &frozen.context.episode)?
            .pop()
            .ok_or_else(|| held("SLOT_NOT_READABLE"))
    })
}
pub(crate) fn record_candidate(
    tx: &Transaction<'_>,
    slot_id: &str,
    raw: &VerifiedRawText,
    candidate: Option<&[u8]>,
    invalid: Option<&str>,
) -> Result<Slot> {
    atomic(tx, || {
        need(installed(tx)?, "EXPLICIT_INSTALL_REQUIRED")?;
        let (run, episode): (String, String) = tx
            .query_row(
                "SELECT workflow_id,episode FROM series_lite_role_slots_v2 WHERE id=?1",
                [slot_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| held("SLOT_NOT_RESERVED"))?;
        let slot = history(tx, &run, &episode)?
            .into_iter()
            .find(|s| s.record.slot_id == slot_id)
            .ok_or_else(|| held("SLOT_NOT_RESERVED"))?;
        let frozen = &slot.record.frozen;
        let (job, state, snapshot, envelope) =
            v1::bound_job(tx, frozen, None)?.ok_or_else(|| held("ORIGINAL_JOB_MISSING"))?;
        let envelope = envelope.ok_or_else(|| held("ORIGINAL_RAW_MISSING"))?;
        let native = std::str::from_utf8(raw.bytes()).map_err(|_| held("RAW_NOT_UTF8"))?;
        need(
            ["received", "completed", "rejected"].contains(&state.as_str())
                && job == raw.job_id()
                && digest(&snapshot.to_string()) == raw.request_sha256()
                && frozen.identity.packet_file_sha256 == raw.packet_sha256()
                && envelope == *raw.transport_envelope()
                && digest(native) == raw.sha256(),
            "RAW_NOT_BOUND_TO_ORIGINAL_SLOT",
        )?;
        let reviewer = frozen.context.role == Role::Reviewer;
        need(
            invalid.is_none_or(|v| !v.trim().is_empty())
                && (!reviewer || candidate.is_none())
                && (reviewer || candidate.is_some() || invalid.is_some()),
            "CANDIDATE_OR_INVALID_SHAPE",
        )?;
        let mut payload = json!({"version":"lite-role-response/v2","encoding":"utf8","text":native,"nativeSha256":raw.sha256(),"slotId":slot_id,"jobId":job,"requestSha256":raw.request_sha256(),"packetSha256":raw.packet_sha256(),"context":frozen.context,"phase":slot.phase,"revision":slot.revision,"invalid":invalid,"previousCandidate":frozen.previous_candidate,"actualContextIndependence":"NOT_VERIFIED","contentReview":"NOT_PROVEN","adoptionGranted":false});
        if reviewer {
            let pin = frozen
                .previous_candidate
                .as_ref()
                .ok_or_else(|| held("REVIEW_CANDIDATE_MISSING"))?;
            let prior = history(tx, &run, &episode)?;
            let author = prior
                .iter()
                .rev()
                .find_map(|s| {
                    s.record
                        .result
                        .as_ref()
                        .filter(|r| r.candidate.as_ref() == Some(pin))
                })
                .ok_or_else(|| held("REVIEW_CANDIDATE_PRODUCER_MISSING"))?;
            payload["candidateSha256"] = json!(pin.payload_sha256);
            payload["candidateNativeSha256"] = json!(pin.native_sha256);
            payload["inputSha256"] = json!(digest(&encode(&frozen.common_inputs)?));
            payload["authorAttempt"] = json!(author.job_id);
            payload["reviewerAttempt"] = json!(job);
        }
        let raw_artifact = artifacts::put(
            tx,
            &Artifact {
                id: format!("{slot_id}:raw"),
                identity: format!("lite-episode:{run}:{episode}"),
                kind: if reviewer { Kind::ReviewRaw } else { Kind::Raw },
                contract_version: VERSION.into(),
                source_sha256: frozen.source_sha256.clone(),
                inputs: frozen.common_inputs.clone(),
                producer: format!("{}:exact-receive", frozen.request.stage),
                author_attempt: job.clone(),
                parents: vec![],
                payload,
            },
        )?;
        let pin = if let Some(bytes) = candidate {
            let text = std::str::from_utf8(bytes).map_err(|_| held("CANDIDATE_NOT_UTF8"))?;
            crate::series_lite_edit::parse(bytes)?;
            let native_sha256 = digest(text);
            let item = artifacts::put(
                tx,
                &Artifact {
                    id: format!("{slot_id}:candidate"),
                    identity: raw_artifact.artifact.identity.clone(),
                    kind: Kind::Candidate,
                    contract_version: VERSION.into(),
                    source_sha256: frozen.source_sha256.clone(),
                    inputs: frozen.common_inputs.clone(),
                    producer: "lite-controller-unvalidated-candidate/v2".into(),
                    author_attempt: job.clone(),
                    parents: vec![raw_artifact.artifact.id.clone()],
                    payload: json!({"version":"lite-candidate-utf8/v2","encoding":"utf8","text":text,"nativeSha256":native_sha256,"baseSha256":frozen.base_sha256,"previousCandidate":frozen.previous_candidate,"validation":"NOT_PROVEN"}),
                },
            )?;
            Some(CandidatePin {
                artifact_id: item.artifact.id,
                payload_sha256: item.payload_sha256,
                native_sha256,
            })
        } else {
            None
        };
        let result = RoleResult {
            slot_id: slot_id.into(),
            job_id: job,
            request_sha256: raw.request_sha256().into(),
            native_response_sha256: raw.sha256().into(),
            raw_artifact_id: raw_artifact.artifact.id,
            raw_artifact_payload_sha256: raw_artifact.payload_sha256,
            candidate: pin,
        };
        if let Some(old) = slot.record.result {
            need(
                old == result && slot.invalid.as_deref() == invalid,
                "ORIGINAL_RESULT_CANNOT_BE_OVERWRITTEN",
            )?;
        } else {
            let data = encode(&SavedResult {
                result: result.clone(),
                invalid: invalid.map(str::to_owned),
            })?;
            tx.execute(
                "INSERT INTO series_lite_role_results_v2 VALUES(?1,?2,?3,?4)",
                params![slot_id, result.job_id, data, digest(&data)],
            )
            .map_err(|e| e.to_string())?;
        }
        history(tx, &run, &episode)?
            .into_iter()
            .find(|s| s.record.slot_id == slot_id)
            .ok_or_else(|| held("RESULT_NOT_READABLE"))
    })
}

#[cfg(test)]
#[path = "series_lite_controller_v2_tests.rs"]
pub(crate) mod tests;
