//! CLI process termination is not a remote terminal receipt. Keep historical
//! timeout recovery identities readable, while refusing to mint another attempt.
use super::*;

fn dispatch_observed(store: &Store, job: &str) -> Result<bool> {
    store.conn()?.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND ((type='codex_dispatch_started' AND json_extract(data,'$.version')='codex-dispatch-started/v1') OR (type='codex_heartbeat' AND json_extract(data,'$.stdinBytes')>0) OR (type='codex_event' AND json_extract(data,'$.type') IN ('thread.started','turn.started','item.started','item.updated','item.completed'))))",
        [job], |row| row.get(0),
    ).map_err(|error| error.to_string())
}

fn terminal_observed(store: &Store, job: &str) -> Result<bool> {
    store.conn()?.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type='codex_event' AND (json_extract(data,'$.type')='turn.completed' OR (json_extract(data,'$.type')='turn.failed' AND (json_extract(data,'$.error.message') LIKE 'You’ve hit your usage limit.%' OR json_extract(data,'$.error.message') LIKE 'You''ve hit your usage limit.%'))))",
        [job], |row| row.get(0),
    ).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::managed_installation_resume_tests::{stopped_text_fixture,save_quota_failure};

    #[tokio::test]
    async fn cli_unknown_timeout_proof_is_read_only_and_cannot_allocate_or_change_evidence() {
        let (_directory,executor,request,job,_,_)=stopped_text_fixture();
        let original=executor.store.job(&job).unwrap();
        let count=executor.store.jobs().unwrap().len();
        let retry=executor.prepare_transport_request(request.clone()).await.unwrap();
        executor.validate_transport_retry(&retry).unwrap();
        let snapshot=executor.request_snapshot(&retry).unwrap();
        assert!(executor.allocate_cli_text_job(&retry,&snapshot).err().unwrap().contains("CLI_TIMEOUT_RETRY_HELD"));
        assert!(check_cli_unknown_hold(&executor.store).unwrap_err().contains("CLI_SUBMISSION_UNKNOWN"));
        let mut forged=retry.clone();forged.input["textTransportRetry"]["quotaRejection"]=json!({"failureKind":"USAGE_LIMIT"});
        assert!(executor.validate_transport_retry_for_dispatch(&forged).unwrap_err().contains("UNBOUND"));
        assert_eq!(executor.store.job(&job).unwrap(),original);
        assert_eq!(executor.store.jobs().unwrap().len(),count);
        // Changing provider and dropping the workflow cannot bypass the hold.
        let mut settings=executor.settings.clone();settings["textProvider"]=json!("api");
        settings["textEndpoint"]=json!("http://127.0.0.1:1/never-called");
        let other=SavedTextExecutor{store:executor.store.clone(),settings,workflow_id:None};
        assert!(other.execute(request).await.unwrap_err().contains("CLI_SUBMISSION_UNKNOWN"));
        assert_eq!(executor.store.jobs().unwrap().len(),count);
    }

    #[test]
    fn cli_unknown_network_turn_failed_is_not_a_terminal_receipt_but_quota_is() {
        for message in ["stream disconnected before completion: tls handshake eof", "connection reset", "unclassified upstream error"] {
            let (_directory,executor,request,job,snapshot,_)=stopped_text_fixture();
            executor.store.event(&job,"codex_event",&json!({"type":"turn.started"})).unwrap();
            executor.store.event(&job,"codex_event",&json!({"type":"turn.failed","error":{"message":message}})).unwrap();
            let error=executor.record_text_execution_failure(&request,&job,&crate::pipeline::digest(&snapshot.to_string()),&format!("Codex执行失败: {message}")).unwrap();
            assert!(error.contains("CLI_SUBMISSION_UNKNOWN"));
            assert_eq!(executor.store.job(&job).unwrap()["status"],"submission_unknown");
            assert!(check_cli_unknown_hold(&executor.store).is_err());
        }
        let (_directory,executor,request,job,snapshot,_)=stopped_text_fixture();
        save_quota_failure(&executor,&request,&job,&snapshot);
        check_cli_unknown_hold(&executor.store).unwrap();
        assert!(terminal_observed(&executor.store,&job).unwrap());
    }

    #[test]
    fn cli_unknown_start_marker_closes_crash_gap_without_treating_unlaunched_preparation_as_dispatch() {
        let directory=tempfile::tempdir().unwrap();let store=Store::open(directory.path().join("project")).unwrap();
        store.create("synthetic","synthetic").unwrap();
        let job=store.add_job("text","prepared-only","codex","synthetic",&json!({}),"failed").unwrap();
        store.event(&job,"cli_dispatch_frozen",&json!({"preparedOnly":true})).unwrap();
        check_cli_unknown_hold(&store).unwrap();
        store.update_job(&job,"submitting",&json!({})).unwrap();
        store.event(&job,"codex_dispatch_started",&json!({"version":"codex-dispatch-started/v1","pid":123})).unwrap();
        store.recover_interrupted().unwrap();
        assert_eq!(store.job(&job).unwrap()["status"],"interrupted");
        assert!(check_cli_unknown_hold(&store).unwrap_err().contains("CLI_SUBMISSION_UNKNOWN"));
        let original=store.job(&job).unwrap();
        store.event(&job,"codex_event",&json!({"type":"turn.failed","error":{"message":""}})).unwrap();
        assert!(check_cli_unknown_hold(&store).is_err());
        assert_eq!(store.job(&job).unwrap(),original);
    }
}

/// Only an already accepted, fully bound historical recovery can retain its
/// prior readability. Merely having a retry UUID/result/proof is insufficient.
fn accepted_historical_recovery(store: &Store, origin: &str) -> Result<bool> {
    let connection = store.conn()?;
    let steps: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_steps')", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if !steps { return Ok(false); }
    let mut query = connection.prepare(
        "SELECT w.workflow_id,j.snapshot FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.state='accepted' AND j.kind='text' AND j.provider='codex' AND j.state='completed' AND (json_extract(j.snapshot,'$.input.textTransportRetry.originJobId')=?1 OR EXISTS(SELECT 1 FROM json_each(j.snapshot,'$.input.textTransportRetry.priorFailures') p WHERE json_extract(p.value,'$.jobId')=?1))",
    ).map_err(|error| error.to_string())?;
    let rows = query.query_map([origin], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?)))
        .map_err(|error| error.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    for (run, snapshot) in rows {
        let Ok(request) = serde_json::from_str::<TextRequest>(&snapshot) else { continue; };
        if accepted_transport_retry_proof(store, &run, &request)? { return Ok(true); }
    }
    Ok(false)
}

/// This is read-only, including for old failed/interrupted CLI records. Never
/// reset their state, raw response, snapshot, receipt, or original node budget.
pub(crate) fn cli_unknown_jobs(store: &Store) -> Result<Vec<(String, String)>> {
    let connection = store.conn()?;
    let mut query = connection.prepare(
        "SELECT id,state FROM jobs WHERE kind='text' AND provider='codex' AND state NOT IN ('completed','rejected') ORDER BY rowid",
    ).map_err(|error| error.to_string())?;
    let rows = query.query_map([], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?)))
        .map_err(|error| error.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    let mut held = Vec::new();
    for (job, state) in rows {
        let unknown = state == "submission_unknown"
            || (matches!(state.as_str(), "failed" | "interrupted" | "cancelled" | "stale")
                && dispatch_observed(store, &job)? && !terminal_observed(store, &job)?);
        if unknown && !accepted_historical_recovery(store, &job)? {
            let reason=format!("CLI_SUBMISSION_UNKNOWN: 原 CLI 任务 {job} 已开始但没有可信终态；本地退出不能证明云端失败，保留原请求、回包和预算，禁止通过重试、新工作流或切换渠道重发");
            held.push((job, reason));
        }
    }
    Ok(held)
}

pub(crate) fn check_cli_unknown_hold(store: &Store) -> Result<()> {
    if let Some((_, reason))=cli_unknown_jobs(store)?.into_iter().next() {return Err(reason);}
    Ok(())
}

impl SavedTextExecutor {
    pub(super) fn validate_transport_retry_for_dispatch(&self, request: &TextRequest) -> Result<()> {
        // validate_transport_retry remains the exact old read/replay contract.
        self.validate_transport_retry(request)?;
        if let Some(proof) = request.input.get("textTransportRetry") {
            if proof["quotaRejection"]["failureKind"] != "USAGE_LIMIT" {
                return Err("CLI_TIMEOUT_RETRY_HELD: 历史本地超时证据只能读取原已收到或已采用结果，不能证明远端失败，未分配或派发新任务".into());
            }
        }
        Ok(())
    }

    pub(super) fn record_text_execution_failure(&self, request: &TextRequest, job: &str, hash: &str, error: &str) -> Result<String> {
        let unknown = s(&self.settings,"textProvider") == "codex"
            && dispatch_observed(&self.store, job)? && !terminal_observed(&self.store, job)?;
        let state = if unknown { "submission_unknown" } else { "failed" };
        let error = if unknown {
            let cause = if error == "客户端正在关闭，未启动新的文本任务" {
                "客户端关闭时原 CLI 任务已开始"
            } else { error };
            format!("CLI_SUBMISSION_UNKNOWN: {cause}；已有派发但未收到可信终态，原任务保留，待核对，不自动重发")
        } else { error.to_string() };
        self.store.update_job(job, state, &json!({"error":error}))?;
        if let Some(run) = &self.workflow_id {
            self.store.checkpoint(run, &step_key(request), &request.stage, hash, state, Some(job), None, Some(&error))?;
        }
        Ok(error)
    }
}
