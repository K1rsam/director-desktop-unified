//! Reversible task-list visibility. Provider history and scheduler inputs stay intact.
use super::*;

const PREFIX: &str = "task_record_visibility:";
const VERSION: &str = "task-record-visibility/v1";

fn terminal_failure(job: &Value) -> bool {
    if job["kind"] != "image" || job["status"] != "failed" { return false; }
    if job["provider"] == "chrome" { return crate::chrome_terminal::settled_job(job); }
    let task = s(job, "providerTaskId");
    ["dreamina", "bh"].contains(&s(job, "provider")) && !task.is_empty()
        && job["result"]["taskId"] == task && job["result"]["status"] == "failed"
        && job["result"]["resultUrls"].as_array().is_some_and(Vec::is_empty)
}

fn archived_metadata(raw: Option<String>, key: &str) -> Result<bool> {
    let Some(raw) = raw else { return Ok(false); };
    let record: Value = serde_json::from_str(&raw).map_err(|_| "任务列表记录损坏，未覆盖历史")?;
    if record["schemaVersion"] != VERSION || record["jobId"] != key || !record["archived"].is_boolean() {
        return Err("任务列表记录身份不一致，未覆盖历史".into());
    }
    Ok(record["archived"] == true)
}

impl Store {
    pub(crate) fn task_record_is_terminal_failure(&self, key: &str) -> Result<bool> {
        Ok(terminal_failure(&self.job(key)?))
    }
    /// Only a proved terminal image failure can be hidden or restored. No task
    /// columns, provider calls, retry budgets, snapshots, or receipts are changed.
    pub fn set_task_record_archived(&self, key: &str, archived: bool, expected_snapshot: Option<&str>) -> Result<bool> {
        let mut connection = self.conn()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        let query = format!("SELECT {JOB_COLUMNS} FROM jobs AS j WHERE j.id=?1");
        let raw = tx.query_row(&query, [key], read_job).optional().map_err(|e| e.to_string())?.ok_or("任务不存在")?;
        let job = finish_job(raw)?;
        if let Some(expected) = expected_snapshot {
            if expected != crate::pipeline::digest(&job["snapshot"].to_string()) {
                return Err("任务冻结材料已变化，请刷新后再处理记录".into());
            }
        }
        if !terminal_failure(&job) {
            return Err("只能移除或恢复已证明结束的失败图片记录；进行中或结果未知的任务保留".into());
        }
        let metadata_key = format!("{PREFIX}{key}");
        let original: Option<String> = tx.query_row("SELECT value FROM metadata WHERE key=?1", [&metadata_key], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
        if archived_metadata(original, key)? == archived { return Ok(false); }
        let changed_at = now();
        let record = json!({"schemaVersion":VERSION,"jobId":key,"archived":archived,"updatedAt":changed_at});
        tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![metadata_key,record.to_string()]).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)", params![key,
            if archived { "task_record_archived" } else { "task_record_restored" },
            json!({"schemaVersion":VERSION,"archived":archived,"recordOnly":true,"originalEvidencePreserved":true}).to_string(), changed_at]).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO metadata(key,value) VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [changed_at]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// This projection is deliberately separate from job_summaries(), jobs(),
    /// pending_jobs(), and all channel ownership or retry queries.
    pub fn archived_task_record_ids(&self) -> Result<HashSet<String>> {
        let connection = self.conn()?;
        let query = format!("SELECT {JOB_COLUMNS},m.value AS task_record_visibility FROM jobs AS j JOIN metadata AS m ON m.key=?1||j.id WHERE j.kind='image' AND j.state='failed'");
        let mut statement = connection.prepare(&query).map_err(|e| e.to_string())?;
        let rows = statement.query_map([PREFIX], |row| Ok((read_job(row)?,row.get::<_,String>("task_record_visibility")?))).map_err(|e| e.to_string())?;
        let mut ids = HashSet::new();
        for row in rows {
            let (raw, metadata) = row.map_err(|e| e.to_string())?;
            let job = finish_job(raw)?;
            if archived_metadata(Some(metadata), s(&job,"id"))? && terminal_failure(&job) { ids.insert(s(&job,"id").to_owned()); }
        }
        Ok(ids)
    }
}
