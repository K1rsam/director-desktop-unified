use crate::{
    binding,
    providers::{
        self, Capabilities, DownloadReceipt, GenerationRequest, PreparedRequest, ProviderConfig,
        ProviderError, ProviderResult, QueryResult, SubmitReceipt,
    },
    store::{id, now, s, Result, Store},
};
use serde_json::{json, Value};
use std::path::Path;

pub trait Channels: Send + Sync {
    fn capabilities(&self, c: &ProviderConfig) -> ProviderResult<Capabilities>;
    fn prepare(&self, c: &ProviderConfig, r: &GenerationRequest)
        -> ProviderResult<PreparedRequest>;
    fn submit(&self, c: &ProviderConfig, r: &PreparedRequest) -> ProviderResult<SubmitReceipt>;
    fn query(&self, c: &ProviderConfig, id: &str) -> ProviderResult<QueryResult>;
    fn query_for_kind(
        &self,
        c: &ProviderConfig,
        id: &str,
        kind: &str,
    ) -> ProviderResult<QueryResult> {
        let _ = kind;
        self.query(c, id)
    }
    fn query_prepared(
        &self,
        c: &ProviderConfig,
        p: &PreparedRequest,
        id: &str,
    ) -> ProviderResult<QueryResult> {
        self.query_for_kind(c, id, &p.request.kind)
    }
    fn download(
        &self,
        c: &ProviderConfig,
        url: &str,
        path: &Path,
    ) -> ProviderResult<DownloadReceipt>;
    fn reconcile(&self, c: &ProviderConfig, id: &str) -> ProviderResult<QueryResult>;
}
pub struct RealChannels;
impl Channels for RealChannels {
    fn capabilities(&self, c: &ProviderConfig) -> ProviderResult<Capabilities> {
        providers::capabilities(c)
    }
    fn prepare(
        &self,
        c: &ProviderConfig,
        r: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        providers::prepare(c, r)
    }
    fn submit(&self, c: &ProviderConfig, r: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        providers::submit(c, r)
    }
    fn query(&self, c: &ProviderConfig, id: &str) -> ProviderResult<QueryResult> {
        providers::query(c, id)
    }
    fn query_for_kind(
        &self,
        c: &ProviderConfig,
        id: &str,
        kind: &str,
    ) -> ProviderResult<QueryResult> {
        providers::query_for_kind(c, id, kind)
    }
    fn query_prepared(
        &self,
        c: &ProviderConfig,
        p: &PreparedRequest,
        id: &str,
    ) -> ProviderResult<QueryResult> {
        providers::query_prepared(c, p, id)
    }
    fn download(&self, c: &ProviderConfig, u: &str, p: &Path) -> ProviderResult<DownloadReceipt> {
        providers::download(c, u, p)
    }
    fn reconcile(&self, c: &ProviderConfig, id: &str) -> ProviderResult<QueryResult> {
        providers::reconcile_unknown(c, id)
    }
}
fn error_state(e: &ProviderError) -> &str {
    if e.submission_unknown {
        "submission_unknown"
    } else if e.auth_required {
        "auth_required"
    } else {
        "failed"
    }
}
pub fn generation_request(snapshot: &Value) -> Result<GenerationRequest> {
    serde_json::from_value(snapshot.clone()).map_err(|e| format!("请求契约无效: {e}"))
}

fn unsubmitted(job: &Value) -> bool {
    let receipt = &job["receipt"];
    (job["providerTaskId"].is_null() || job["providerTaskId"] == "")
        && job["submissionUnknown"].as_bool() != Some(true)
        && s(job, "status") != "submission_unknown"
        && (receipt.is_null() || receipt == &json!({}) || receipt.as_str() == Some(""))
}

/// Only an explicit review-only queued video may use the local preparation burst.
pub fn prepare_only_eligible(job: &Value) -> bool {
    s(job, "kind") == "video"
        && s(job, "status") == "queued"
        && job["snapshot"]["autoSubmit"].as_bool() == Some(false)
        && job["nextAttemptAt"].as_i64().unwrap_or(0) <= chrono::Utc::now().timestamp()
        && unsubmitted(job)
}

enum LocalTaskFailure {
    PreparedContract(serde_json::Error),
    PreparePersistence(String),
    SyncReceiptContract(serde_json::Error),
}

/// The error's originating operation, never its message text, decides the terminal policy.
fn fail_local_task(store: &Store, key: &str, from: &str, failure: LocalTaskFailure) -> Result<()> {
    let current = store.job(key)?;
    if s(&current, "status") != from {
        return Ok(());
    }
    let (code, message, needs_unsubmitted) = match failure {
        LocalTaskFailure::PreparedContract(error) => (
            "LOCAL_PREPARED_CONTRACT",
            format!("准备记录无法解析，未提交生成：{error}"),
            true,
        ),
        LocalTaskFailure::PreparePersistence(error) => (
            "LOCAL_PREPARE_PERSISTENCE",
            format!("本地素材准备结果无法保存，未提交生成：{error}"),
            true,
        ),
        LocalTaskFailure::SyncReceiptContract(error) => (
            "LOCAL_SYNC_RECEIPT_CONTRACT",
            format!("原同步图片回执无法解析；保留原任务身份，未重新生成：{error}"),
            false,
        ),
    };
    if needs_unsubmitted && !unsubmitted(&current) {
        return Ok(());
    }
    if !needs_unsubmitted && !s(&current, "providerTaskId").starts_with("sync:") {
        return Ok(());
    }
    // A concurrent edit/terminal transition owns the newer state. No identity,
    // original receipt, frozen request, or existing retry counter is rewritten.
    let persisted = store
        .claim_job_fields(
            key,
            from,
            "failed",
            &json!({"error":message,"nextAttemptAt":0}),
        )
        .map_err(|error| format!("{message}；终态也未能持久化：{error}"))?;
    if persisted {
        store.event(key, "local_error", &json!({"code":code,"message":message}))?;
    }
    Ok(())
}

/// Contains no submission route, even if another caller changes autoSubmit later.
pub fn prepare_only(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
) -> Result<()> {
    prepare_generation(store, key, cfg, account, channels, true).map(|_| ())
}

fn prepare_generation(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
    review_only: bool,
) -> Result<Option<Value>> {
    let mut j = store.job(key)?;
    if !unsubmitted(&j) || review_only && !prepare_only_eligible(&j) {
        return Ok(None);
    }
    let state = s(&j, "status").to_owned();
    if !["queued", "ready"].contains(&state.as_str()) {
        return Ok(None);
    }
    if s(&j, "accountId") != account {
        store.claim_job_fields(
            key,
            &state,
            "auth_required",
            &json!({"error":"账户身份已变化；请恢复原账户后继续"}),
        )?;
        return Ok(None);
    }
    if j["snapshot"]["problems"]
        .as_array()
        .map(|p| !p.is_empty())
        .unwrap_or(false)
    {
        store.claim_job_fields(
            key,
            &state,
            "blocked",
            &json!({"error":"素材包仍存在待处理问题"}),
        )?;
        return Ok(None);
    }
    if let Err(e) = binding::validate_generation_snapshot(store, &j["snapshot"]) {
        store.claim_job_fields(key, &state, "stale", &json!({"error":e}))?;
        return Ok(None);
    }
    if state == "queued" {
        let request = match generation_request(&j["snapshot"]) {
            Ok(r) => r,
            Err(e) => {
                store.claim_job_fields(key, "queued", "blocked", &json!({"error":e}))?;
                return Ok(None);
            }
        };
        if !store.claim_job(key, "queued", "preparing")? {
            return Ok(None);
        }
        match channels.prepare(cfg, &request) {
            Ok(p) => {
                // Uploads are a separate durable receipt class, never mistaken for generation IDs.
                let saved: Result<bool> = (|| {
                    let mut c = store.conn()?;
                    let tx = c.transaction().map_err(|e| e.to_string())?;
                    for upload in &p.upload_receipts {
                        tx.execute(
                            "INSERT INTO uploads VALUES(?1,?2,?3,?4)",
                            rusqlite::params![
                                id(),
                                key,
                                serde_json::to_string(upload).map_err(|e| e.to_string())?,
                                now()
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    tx.commit().map_err(|e| e.to_string())?;
                    // An edit/adoption during upload has already invalidated this preparation.
                    if let Err(e) = binding::validate_generation_snapshot(store, &j["snapshot"]) {
                        store.claim_job_fields(key, "preparing", "stale", &json!({"error":e}))?;
                        return Ok(false);
                    }
                    let current = store.job(key)?;
                    if !unsubmitted(&current) {
                        return Ok(false);
                    }
                    store.claim_job_fields(key, "preparing", "ready", &json!({"prepared":p}))
                })();
                match saved {
                    Ok(true) => j = store.job(key)?,
                    Ok(false) => return Ok(None),
                    Err(error) => {
                        fail_local_task(
                            store,
                            key,
                            "preparing",
                            LocalTaskFailure::PreparePersistence(error),
                        )?;
                        return Ok(None);
                    }
                }
            }
            Err(e) => {
                if unsubmitted(&store.job(key)?) {
                    store.claim_job_fields(
                        key,
                        "preparing",
                        error_state(&e),
                        &json!({"error":e.message}),
                    )?;
                }
                return Ok(None);
            }
        }
    }
    Ok(Some(j))
}

/// One invocation may prepare or submit once. Resume never enters this path for known/unknown submits.
pub fn run_generation(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
) -> Result<()> {
    let Some(j) = prepare_generation(store, key, cfg, account, channels, false)? else {
        return Ok(());
    };
    if s(&j, "status") == "ready"
        && j["snapshot"]["autoSubmit"].as_bool() == Some(true)
        && unsubmitted(&j)
    {
        if let Err(e) = binding::validate_generation_snapshot(store, &j["snapshot"]) {
            store.claim_job_fields(key, "ready", "stale", &json!({"error":e}))?;
            return Ok(());
        }
        let prepared: PreparedRequest = match serde_json::from_value(j["prepared"].clone()) {
            Ok(prepared) => prepared,
            Err(error) => {
                fail_local_task(
                    store,
                    key,
                    "ready",
                    LocalTaskFailure::PreparedContract(error),
                )?;
                return Ok(());
            }
        };
        // Persist intent BEFORE any billable network/CLI request.
        if !store.claim_job(key, "ready", "submitting")? {
            return Ok(());
        }
        match channels.submit(cfg, &prepared) {
            Ok(receipt) => {
                if receipt.task_id.is_empty() {
                    store.update_job(
                        key,
                        "submission_unknown",
                        &json!({"error":"渠道未返回可恢复的任务身份"}),
                    )?;
                    return Ok(());
                }
                let state = if receipt.status == "succeeded" {
                    "succeeded"
                } else {
                    "submitted"
                };
                store.update_job(key,state,&json!({"providerTaskId":receipt.task_id,"receipt":receipt,"result":{"resultUrls":receipt.result_urls}}))?;
            }
            Err(e) => {
                store.update_job(key, error_state(&e), &json!({"error":e.message}))?;
            }
        }
    }
    Ok(())
}

/// Query/download retries have no access to `submit`.
pub fn resume_job(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
    download: bool,
) -> Result<()> {
    let j = store.job(key)?;
    let task = s(&j, "providerTaskId");
    if task.is_empty() {
        return Err("没有已保存的渠道任务ID，不能自动恢复或重新提交".into());
    }
    if s(&j, "accountId") != account {
        store.update_job(
            key,
            "auth_required",
            &json!({"error":"账户身份已变化，请恢复原账户"}),
        )?;
        return Err("账户身份已变化，请恢复原账户".into());
    }
    let already_downloaded = !j["resultMediaIds"]
        .as_array()
        .map(|m| m.is_empty())
        .unwrap_or(true)
        && s(&j, "status") == "downloaded";
    let prepared = if !j["prepared"].is_null() {
        match serde_json::from_value::<PreparedRequest>(j["prepared"].clone()) {
            Ok(p) if p.request.kind == s(&j, "kind") => Some(p),
            Ok(_) => {
                record_query_error(
                    store,
                    key,
                    already_downloaded,
                    "准备记录的媒体类型与原任务不一致，未查询或采用结果；请核对原任务记录",
                )?;
                return Ok(());
            }
            Err(_) => {
                record_query_error(
                    store,
                    key,
                    already_downloaded,
                    "原任务准备记录无法解析，未查询或采用结果",
                )?;
                return Ok(());
            }
        }
    } else {
        None
    };
    if task.starts_with("sync:") {
        let receipt: SubmitReceipt = match serde_json::from_value(j["receipt"].clone()) {
            Ok(receipt) => receipt,
            Err(error) => {
                if already_downloaded {
                    store.event(
                        key,
                        "query_error",
                        &json!({"message":format!("原同步图片回执无法解析：{error}")}),
                    )?;
                } else {
                    fail_local_task(
                        store,
                        key,
                        s(&j, "status"),
                        LocalTaskFailure::SyncReceiptContract(error),
                    )?;
                }
                return Ok(());
            }
        };
        if download {
            return download_result(
                store,
                key,
                cfg,
                channels,
                &QueryResult {
                    task_id: task.into(),
                    status: "succeeded".into(),
                    result_urls: receipt.result_urls,
                    error: None,
                    actual_cost: None,
                    raw: receipt.raw,
                },
            );
        }
        return Err("同步图片任务没有云端查询ID；已有下载地址可重试，过期后需明确重新生成".into());
    }
    let queried = if let Some(p) = &prepared {
        channels.query_prepared(cfg, p, task)
    } else {
        channels.query_for_kind(cfg, task, s(&j, "kind"))
    };
    match queried {
        Ok(result) => {
            if result.task_id != task {
                record_query_error(
                    store,
                    key,
                    already_downloaded,
                    "渠道查询返回的任务身份与原任务不一致，未采用结果",
                )?;
                return Ok(());
            }
            let state = persist_query_result(store, key, &result, already_downloaded)?;
            if state == "succeeded" && download {
                download_result(store, key, cfg, channels, &result)?;
            }
        }
        Err(e) => {
            if already_downloaded {
                store.event(key, "query_error", &json!({"message":e.message}))?;
            } else if e.auth_required {
                store.update_job(key, "auth_required", &json!({"error":e.message}))?;
            } else {
                store.retry_job(key, "query", &e.message)?;
            }
        }
    }
    Ok(())
}

/// A failed follow-up inspection must not demote a completed local result.
fn record_query_error(
    store: &Store,
    key: &str,
    already_downloaded: bool,
    message: &str,
) -> Result<()> {
    if already_downloaded {
        store.event(key, "query_error", &json!({"message":message}))
    } else {
        store.retry_job(key, "query", message)
    }
}

/// A missing parsed result address is not proof of expiry. Keep the remote ID and raw
/// response so a later parser/query can recover the same task without another submission.
fn persist_query_result(
    store: &Store,
    key: &str,
    result: &QueryResult,
    already_downloaded: bool,
) -> Result<&'static str> {
    let (state, message) = if already_downloaded {
        ("downloaded", None)
    } else {
        match result.status.as_str() {
            "succeeded" if result.result_urls.is_empty() => (
                "query_failed",
                Some("生成完成，结果地址暂未解析或返回；保留原任务ID继续查询，不会重新提交生成"),
            ),
            "succeeded" => ("succeeded", None),
            "failed" => ("failed", None),
            "auth_required" => ("auth_required", None),
            "expired" | "result_expired" => (
                "result_expired",
                Some("渠道明确报告原任务结果已过期；已保留原任务记录，可核对原任务"),
            ),
            "processing" | "queued" | "pending" | "submitted" => ("processing", None),
            _ => (
                "query_failed",
                Some("渠道返回的原任务状态尚未识别；已保留查询响应，请恢复原任务查询"),
            ),
        }
    };
    let error = message.map(str::to_owned).or_else(|| result.error.clone());
    store.update_job(key, state, &json!({"result":result,"error":error}))?;
    if state == "query_failed" {
        store.retry_job(
            key,
            "query",
            error.as_deref().unwrap_or("原任务查询需要恢复"),
        )?;
    } else if state == "processing" {
        store
            .conn()?
            .execute(
                "UPDATE jobs SET query_attempts=0,next_attempt_at=?1 WHERE id=?2",
                rusqlite::params![chrono::Utc::now().timestamp() + 10, key],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(state)
}

/// Download a durable successful response without occupying a polling worker.
/// Old/incomplete packets are queried by original ID, never submitted again.
pub fn download_saved_result(store: &Store, key: &str, cfg: &ProviderConfig, account: &str, channels: &dyn Channels) -> Result<()> {
    let job = store.job(key)?;
    if !["succeeded", "download_failed"].contains(&s(&job, "status")) { return Ok(()); }
    if s(&job, "accountId") != account {
        store.update_job(key, "auth_required", &json!({"error":"账户身份已变化，请恢复原账户"}))?;
        return Ok(());
    }
    let task = s(&job, "providerTaskId");
    if task.is_empty() { return Err("没有已保存的渠道任务ID，不能下载或重新提交".into()); }
    let saved = serde_json::from_value::<QueryResult>(job["result"].clone()).ok()
        .filter(|result| result.task_id == task && result.status == "succeeded" && !result.result_urls.is_empty());
    let saved = saved.or_else(|| serde_json::from_value::<SubmitReceipt>(job["receipt"].clone()).ok()
        .filter(|receipt| receipt.task_id == task && receipt.status == "succeeded" && !receipt.result_urls.is_empty())
        .map(|receipt| QueryResult { task_id: receipt.task_id, status: "succeeded".into(), result_urls: receipt.result_urls, error: None, actual_cost: None, raw: receipt.raw }));
    match saved {
        Some(result) => download_result(store, key, cfg, channels, &result),
        None => resume_job(store, key, cfg, account, channels, false),
    }
}

pub fn download_result(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    channels: &dyn Channels,
    result: &QueryResult,
) -> Result<()> {
    let job = store.job(key)?;
    if s(&job, "status") == "downloaded"
        && job["resultMediaIds"]
            .as_array()
            .map(|ids| !ids.is_empty())
            .unwrap_or(false)
    {
        return Ok(());
    }
    if s(&job, "providerTaskId").is_empty() || s(&job, "providerTaskId") != result.task_id {
        store.retry_job(key, "query", "下载结果不属于原渠道任务，未导入文件")?;
        return Ok(());
    }
    if persist_query_result(store, key, result, false)? != "succeeded" {
        return Ok(());
    }
    store.update_job(key, "downloading", &json!({"result":result}))?;
    let mut media_ids: Vec<String> = Vec::new();
    let staging = store.root.join("downloads");
    if let Err(error) = std::fs::create_dir_all(&staging) {
        store.retry_job(key, "download", &format!("无法创建本地下载目录：{error}"))?;
        return Ok(());
    }
    for (i, url) in result.result_urls.iter().enumerate() {
        let extension = if s(&job, "kind") == "video" {
            "mp4"
        } else {
            "png"
        };
        let target = staging.join(format!("{key}-{i}-{}.{extension}", id()));
        match channels.download(cfg, url, &target) {
            Ok(_) => {
                let imported = store.import_media(&target, s(&job, "kind"), None, None);
                let m = match imported {
                    Ok(media) => media,
                    Err(error) => {
                        store.retry_job(
                            key,
                            "download",
                            &format!("下载结果尚未可靠导入：{error}"),
                        )?;
                        return Ok(());
                    }
                };
                if let Err(error) = store.attach_download_result(key, s(&m, "id")) {
                    store.retry_job(key, "download", &format!("下载结果尚未可靠关联：{error}"))?;
                    return Ok(());
                }
                media_ids.push(s(&m, "id").to_owned());
                let _ = std::fs::remove_file(&target);
            }
            Err(e) => {
                if e.auth_required {
                    store.update_job(
                        key,
                        "auth_required",
                        &json!({"error":e.message,"result":result}),
                    )?;
                } else {
                    store.retry_job(key, "download", &e.message)?;
                }
                return Ok(());
            }
        }
    }
    store.complete_download(key,&json!({"taskId":result.task_id,"resultUrls":result.result_urls,"mediaIds":media_ids,"actualCost":result.actual_cost,"raw":result.raw}),&media_ids)?;
    Ok(())
}
pub fn reconcile(
    store: &Store,
    key: &str,
    cfg: &ProviderConfig,
    account: &str,
    channels: &dyn Channels,
) -> Result<()> {
    let j = store.job(key)?;
    if s(&j, "status") != "submission_unknown" {
        return Err("此任务不是提交结果未知".into());
    }
    if s(&j, "accountId") != account {
        return Err("请恢复原账户后核对".into());
    }
    match channels.reconcile(cfg, s(&j["snapshot"], "requestId")) {
        Ok(r) => {
            if r.task_id.is_empty() || r.status == "unknown" {
                return Err("未能确定原任务，请在渠道记录中人工核对".into());
            }
            store.update_job(
                key,
                if r.status == "succeeded" {
                    "succeeded"
                } else {
                    "submitted"
                },
                &json!({"providerTaskId":r.task_id,"result":r}),
            )
        }
        Err(e) => Err(e.message),
    }
}
