//! Bounded observation of an owned OpenCLI click. This module cannot prepare or submit.
use super::*;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const VERSION: &str = "opencli-unknown-read-recovery/v1";
const DELAYS: [i64; 5] = [15, 30, 60, 120, 240];
const HELD: &str = "原网页请求身份无法核验；保留未知提交，不重新发送";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Budget {
    version: String,
    binding_sha256: String,
    request_id: String,
    attempts: usize,
    status: String,
    next_attempt_at: i64,
    last_code: String,
}
fn budget_key(key: &str) -> String {
    format!("opencli_unknown_read:{key}")
}
fn hash(v: &Value) -> String {
    crate::pipeline::digest(&v.to_string())
}
fn binding(job: &Value) -> String {
    hash(
        &json!({"snapshot":job["snapshot"],"prepared":job["prepared"],"accountId":job["accountId"],
        "provider":job["provider"],"kind":job["kind"],"jobId":job["id"]}),
    )
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn sha(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn stable_url(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|s| s.strip_prefix("https://chatgpt.com/c/"))
        .is_some_and(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
fn identity_error() -> ProviderError {
    ProviderError::new("OPENCLI_UNKNOWN_IDENTITY", HELD)
}

pub(crate) fn opencli_unknown_candidate(job: &Value) -> bool {
    job["kind"] == "image"
        && job["provider"] == "chrome"
        && job["status"] == "submission_unknown"
        && job["snapshot"]["providerConfig"]["browserTransport"]["transport"] == "opencli/v1"
}

fn saved_budget(store: &Store, key: &str) -> Result<(Option<String>, Option<Budget>)> {
    let raw: Option<String> = store
        .conn()?
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [budget_key(key)],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let budget = raw
        .as_ref()
        .map(|s| serde_json::from_str::<Budget>(s))
        .transpose()
        .map_err(|_| HELD)?;
    if budget.as_ref().is_some_and(|b| {
        b.version != VERSION
            || b.attempts > DELAYS.len()
            || !["waiting", "stopped", "exhausted", "completed"].contains(&b.status.as_str())
    }) {
        return Err(HELD.into());
    }
    Ok((raw, budget))
}

/// Pending includes a future backoff so the dedicated runner does not exit early.
pub(crate) fn opencli_unknown_pending(store: &Store, job: &Value) -> Result<bool> {
    if !opencli_unknown_candidate(job) {
        return Ok(false);
    }
    Ok(saved_budget(store, s(job, "id"))?
        .1
        .is_none_or(|b| b.status == "waiting" && b.attempts < DELAYS.len()))
}
pub(crate) fn opencli_unknown_due(store: &Store, job: &Value) -> Result<bool> {
    due_at(store, job, chrono::Utc::now().timestamp())
}
fn due_at(store: &Store, job: &Value, at: i64) -> Result<bool> {
    if !opencli_unknown_candidate(job) {
        return Ok(false);
    }
    Ok(saved_budget(store, s(job, "id"))?.1.is_none_or(|b| {
        b.status == "waiting" && b.attempts < DELAYS.len() && b.next_attempt_at <= at
    }))
}
pub(crate) fn opencli_unknown_jobs(store: &Store) -> Result<Vec<Value>> {
    let c = store.conn()?;
    let mut q=c.prepare("SELECT id FROM jobs WHERE kind='image' AND provider='chrome' AND state='submission_unknown' AND json_extract(snapshot,'$.providerConfig.browserTransport.transport')='opencli/v1' ORDER BY updated_at,rowid").map_err(|e|e.to_string())?;
    let ids = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut jobs = vec![];
    for id in ids {
        let job = store.job(&id)?;
        if opencli_unknown_due(store, &job)? {
            jobs.push(job);
        }
    }
    Ok(jobs)
}

fn save_budget(
    store: &Store,
    key: &str,
    old: &Option<String>,
    next: &Budget,
    event: &str,
) -> Result<bool> {
    let mut c = store.conn()?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let current: Option<String> = tx
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [budget_key(key)],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if current != *old {
        return Ok(false);
    }
    let value = serde_json::to_string(next).map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![budget_key(key),value]).map_err(|e|e.to_string())?;
    tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",rusqlite::params![key,event,
        json!({"version":VERSION,"requestId":next.request_id,"bindingSha256":next.binding_sha256,"attempt":next.attempts,
            "status":next.status,"nextAttemptAt":next.next_attempt_at,"code":next.last_code,"readOnly":true,
            "prepareCalled":false,"submitCalled":false,"retryBudgetsReset":false}).to_string(),now()]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(true)
}

fn original_packet(
    job: &Value,
    cfg: &ProviderConfig,
    account: &str,
) -> ProviderResult<(PreparedRequest, Value)> {
    let request = generation_request(&job["snapshot"]).map_err(|_| identity_error())?;
    let prepared: PreparedRequest =
        serde_json::from_value(job["prepared"].clone()).map_err(|_| identity_error())?;
    let frozen: ProviderConfig = serde_json::from_value(job["snapshot"]["providerConfig"].clone())
        .map_err(|_| identity_error())?;
    let transport = cfg.browser_transport.as_ref().ok_or_else(identity_error)?;
    let id = &request.request_id;
    if !opencli_unknown_candidate(job)
        || cfg.kind != "chrome"
        || json!(frozen) != json!(cfg)
        || prepared.provider != "chrome"
        || prepared.wire["transport"] != "opencli/v1"
        || json!(prepared.request) != json!(request)
        || request.kind != "image"
        || !valid_id(id)
        || !sha(&prepared.wire["requestHash"])
        || !sha(&transport["runtimeSha256"])
        || s(&prepared.wire, "profile") != s(transport, "profile")
        || s(&prepared.wire, "profile").is_empty()
        || s(&prepared.wire, "accountKey") != s(transport, "accountKey")
        || s(&prepared.wire, "accountKey").is_empty()
        || (!s(job, "providerTaskId").is_empty() && s(job, "providerTaskId") != id)
    {
        return Err(identity_error());
    }
    if s(job, "accountId") != account {
        return Err(ProviderError::new(
            "ACCOUNT_CHANGED",
            "账户身份已变化；保留未知提交",
        ));
    }
    let root = PathBuf::from(s(transport, "stateRoot"));
    if !root.is_absolute() {
        return Err(identity_error());
    }
    let canonical_root = fs::canonicalize(&root).map_err(|_| identity_error())?;
    let file = root.join(id).join("request.json");
    let meta = fs::symlink_metadata(&file).map_err(|_| identity_error())?;
    let canonical = fs::canonicalize(&file).map_err(|_| identity_error())?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() > 1_048_576
        || !canonical.starts_with(&canonical_root)
    {
        return Err(identity_error());
    }
    let saved: Value = serde_json::from_slice(&fs::read(file).map_err(|_| identity_error())?)
        .map_err(|_| identity_error())?;
    let original: GenerationRequest =
        serde_json::from_value(saved["request"].clone()).map_err(|_| identity_error())?;
    if saved["version"] != "opencli-task/v1"
        || saved["requestId"] != *id
        || json!(original) != json!(request)
        || saved["requestHash"] != prepared.wire["requestHash"]
        || saved["profile"] != transport["profile"]
        || saved["accountKey"] != transport["accountKey"]
        || !valid_id(s(&saved, "page"))
    {
        return Err(identity_error());
    }
    Ok((prepared, saved))
}

fn owned_click(prepared: &PreparedRequest, saved: &Value) -> bool {
    let id = &prepared.request.request_id;
    let intent = &saved["sendIntent"];
    let owned = &saved["ownedSend"];
    prepared.wire["sendProtocol"] == "owned-composer-send/v2"
        && sha(&prepared.wire["composerAdapterSha256"])
        && owned["version"] == prepared.wire["sendProtocol"]
        && owned["token"] == *id
        && owned["adapterSha256"] == prepared.wire["composerAdapterSha256"]
        && chrono::DateTime::parse_from_rfc3339(s(saved, "sentAt")).is_ok()
        && intent["version"] == "owned-click-intent/v1"
        && intent["token"] == *id
        && intent["requestId"] == *id
        && intent["requestHash"] == prepared.wire["requestHash"]
        && !s(saved, "prompt").is_empty()
        && intent["promptSha256"] == crate::pipeline::digest(s(saved, "prompt"))
        && (intent["page"] == saved["page"] || stable_url(&saved["conversationUrl"]))
        && valid_id(s(intent, "page"))
}

/// Manual reconciliation shares the frozen full-request check, including old protocols.
/// It has no access to the automatic budget and cannot reset exhausted attempts.
pub(super) fn query_bound_original(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
) -> ProviderResult<QueryResult> {
    let job = store.job(key).map_err(|_| identity_error())?;
    let (prepared, _) = original_packet(&job, cfg, account)?;
    let result = channels.query_prepared(cfg, &prepared, &prepared.request.request_id)?;
    if result.task_id != prepared.request.request_id
        || store.job(key).map_err(|_| identity_error())? != job
    {
        return Err(identity_error());
    }
    original_packet(&job, cfg, account)?;
    Ok(result)
}

fn safe_code(code: &str) -> String {
    if !code.is_empty()
        && code.len() <= 80
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        code.into()
    } else {
        "ORIGINAL_READ_FAILED".into()
    }
}
fn terminal_error(e: &ProviderError) -> bool {
    e.auth_required
        || [
            "ACCOUNT_CHANGED",
            "AUTH_REQUIRED",
            "BROWSER_CHECK_REQUIRED",
            "IMAGE_PROVENANCE",
            "PROVIDER_MISMATCH",
            "REQUEST_IDENTITY_CHANGED",
            "CONVERSATION_CHANGED",
            "ORIGINAL_SEND_INTENT_UNCONFIRMED",
            "ORIGINAL_REQUEST_REQUIRED",
            "OPENCLI_UNKNOWN_IDENTITY",
            "ORIGINAL_JOURNAL_CHANGED",
            "ORIGINAL_ARCHIVE_CHANGED",
            "OPENCLI_COMPONENT_MISMATCH",
        ]
        .contains(&e.code.as_str())
}

pub(super) fn preserve_original_outcome(store: &Store, key: &str, job: &Value) -> Result<()> {
    store.event(key,"opencli_unknown_original_outcome",&json!({"version":VERSION,
        "requestId":job["snapshot"]["requestId"],"bindingSha256":binding(job),"originalResult":job["result"],
        "originalReceipt":job["receipt"],"originalError":job["error"],"originalState":job["status"],
        "queryAttempts":job["queryAttempts"],"downloadAttempts":job["downloadAttempts"],"readOnly":true,"resubmitted":false}))
}

pub(crate) fn run_opencli_unknown_read(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
) -> Result<bool> {
    run_at(
        store,
        key,
        cfg,
        account,
        channels,
        chrono::Utc::now().timestamp(),
    )
}
/// Local configuration/identity failures must terminate the read lane too;
/// otherwise the dedicated runner would keep waiting for a job it cannot inspect.
pub(crate) fn stop_opencli_unknown_read(store: &Store, key: &str, code: &str) -> Result<bool> {
    let job = store.job(key)?;
    if !opencli_unknown_candidate(&job) {
        return Ok(false);
    }
    let (old, previous) = saved_budget(store, key)?;
    let mut budget = previous.unwrap_or(Budget {
        version: VERSION.into(),
        binding_sha256: binding(&job),
        request_id: s(&job["snapshot"], "requestId").into(),
        attempts: 0,
        status: "waiting".into(),
        next_attempt_at: 0,
        last_code: String::new(),
    });
    if budget.status != "waiting" {
        return Ok(false);
    }
    budget.status = "stopped".into();
    budget.next_attempt_at = 0;
    budget.last_code = safe_code(code);
    save_budget(store, key, &old, &budget, "opencli_unknown_read_stopped")
}
fn run_at(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
    at: i64,
) -> Result<bool> {
    let job = store.job(key)?;
    if !opencli_unknown_candidate(&job) {
        return Ok(false);
    }
    let (old, previous) = saved_budget(store, key)?;
    let fresh = previous.is_none();
    let mut budget = previous.unwrap_or(Budget {
        version: VERSION.into(),
        binding_sha256: binding(&job),
        request_id: s(&job["snapshot"], "requestId").into(),
        attempts: 0,
        status: "waiting".into(),
        next_attempt_at: at + DELAYS[0],
        last_code: "ORIGINAL_READ_SCHEDULED".into(),
    });
    if budget.status != "waiting" || budget.attempts >= DELAYS.len() {
        return Ok(false);
    }
    if !fresh && budget.next_attempt_at > at {
        return Ok(false);
    }
    let packet = original_packet(&job, cfg, account).and_then(|(p, j)| {
        if budget.binding_sha256 != binding(&job)
            || budget.request_id != p.request.request_id
            || !owned_click(&p, &j)
        {
            Err(identity_error())
        } else {
            Ok((p, j))
        }
    });
    let (prepared, _) = match packet {
        Ok(packet) => packet,
        Err(e) => {
            budget.status = "stopped".into();
            budget.next_attempt_at = 0;
            budget.last_code = safe_code(&e.code);
            return save_budget(store, key, &old, &budget, "opencli_unknown_read_stopped");
        }
    };
    if fresh {
        return save_budget(store, key, &old, &budget, "opencli_unknown_read_scheduled");
    }
    // Reserve and consume the attempt before reading. A crash cannot reset it.
    budget.attempts += 1;
    budget.next_attempt_at = DELAYS.get(budget.attempts).map_or(0, |delay| at + delay);
    budget.status = if budget.attempts == DELAYS.len() {
        "exhausted"
    } else {
        "waiting"
    }
    .into();
    budget.last_code = "ORIGINAL_READ_STARTED".into();
    if !save_budget(store, key, &old, &budget, "opencli_unknown_read_started")? {
        return Ok(false);
    }
    let claimed = Some(serde_json::to_string(&budget).map_err(|e| e.to_string())?);
    let outcome = channels.query_prepared(cfg, &prepared, &prepared.request.request_id);
    let current = store.job(key)?;
    if current != job {
        budget.status = "stopped".into();
        budget.next_attempt_at = 0;
        budget.last_code = "ORIGINAL_JOB_CHANGED".into();
        save_budget(
            store,
            key,
            &claimed,
            &budget,
            "opencli_unknown_read_stopped",
        )?;
        return Ok(true);
    }
    let outcome = outcome.and_then(|result| {
        let (_, saved) = original_packet(&job, cfg, account)?;
        if !owned_click(&prepared, &saved) || result.task_id != prepared.request.request_id {
            return Err(identity_error());
        }
        // A page still loading can return processing without a DOM acknowledgement.
        // That is not permission to turn UNKNOWN into a confirmed remote task.
        let acknowledged = saved["sendConfirmed"] == true && stable_url(&saved["conversationUrl"]);
        let processing = ["processing", "queued", "pending", "submitted"]
            .contains(&result.status.as_str())
            && acknowledged
            && result.raw["conversationUrl"] == saved["conversationUrl"];
        let completed = result.status == "succeeded"
            && acknowledged
            && saved["status"] == "downloaded"
            && result.raw["originalIdentityVerified"] == true
            && result.raw["requestHash"] == prepared.wire["requestHash"];
        if !processing && !completed {
            return Err(ProviderError::new(
                if result.status == "auth_required" {
                    "AUTH_REQUIRED"
                } else {
                    "ORIGINAL_RESULT_UNCONFIRMED"
                },
                "尚未核实原任务回执",
            ));
        }
        Ok(result)
    });
    match outcome {
        Ok(result) => {
            preserve_original_outcome(store, key, &job)?;
            super::persist_query_result_with_budget(store, key, &result, false, true)?;
            budget.status = "completed".into();
            budget.next_attempt_at = 0;
            budget.last_code = "ORIGINAL_READ_CONFIRMED".into();
            save_budget(
                store,
                key,
                &claimed,
                &budget,
                "opencli_unknown_read_confirmed",
            )?;
        }
        Err(error) => {
            budget.last_code = safe_code(&error.code);
            if terminal_error(&error) {
                budget.status = "stopped".into();
                budget.next_attempt_at = 0;
            }
            save_budget(
                store,
                key,
                &claimed,
                &budget,
                if budget.status == "waiting" {
                    "opencli_unknown_read_waiting"
                } else {
                    "opencli_unknown_read_stopped"
                },
            )?;
        }
    }
    Ok(true)
}

#[cfg(test)]
#[path = "tasks_opencli_unknown_tests.rs"]
mod tests;
