//! Explicit local record visibility maintenance. No accounts, scheduler or recovery.
use super::*;
use serde::{Deserialize, Serialize};

const CONTRACT: &str = "task-record-maintenance/v1";
const HELD: &str = "TASK_RECORD_MAINTENANCE_HELD: 请求或原任务身份不一致，未修改任务记录";

#[derive(Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct JobBinding { job_id: String, snapshot_sha256: String }

#[derive(Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct Request {
    schema_version: String,
    action: String,
    home: PathBuf,
    project_id: String,
    source_sha256: String,
    source_version: i64,
    jobs: Vec<JobBinding>,
    report_path: PathBuf,
}
impl Request {
    fn hash(&self) -> Result<String> { Ok(pipeline::digest(&serde_json::to_value(self).map_err(|_|HELD)?.to_string())) }
    fn validate(&self, input: &Path) -> Result<()> {
        let sha=|v:&str|v.len()==64 && v.bytes().all(|c|c.is_ascii_hexdigit());
        let mut ids=HashSet::new();
        if self.schema_version!=CONTRACT || !["jobs.archive","jobs.restore"].contains(&self.action.as_str())
            || !self.home.is_absolute() || !self.report_path.is_absolute() || self.report_path==input
            || self.report_path.extension().and_then(|v|v.to_str())!=Some("json")
            || uuid::Uuid::parse_str(&self.project_id).is_err() || self.source_version<=0 || !sha(&self.source_sha256)
            || self.jobs.is_empty() || self.jobs.len()>100
            || self.jobs.iter().any(|j|uuid::Uuid::parse_str(&j.job_id).is_err()||!sha(&j.snapshot_sha256)||!ids.insert(&j.job_id)) {
            return Err(HELD.into());
        }
        Ok(())
    }
    fn check_report(&self, engine: &Engine) -> Result<()> {
        let parent=self.report_path.parent().ok_or(HELD)?.canonicalize().map_err(|_|HELD)?;
        let home=self.home.canonicalize().map_err(|_|HELD)?;
        if parent.starts_with(&home) { return Err(HELD.into()); }
        let registry=engine.registry()?;
        for project in registry.as_array().ok_or(HELD)? {
            let path=Path::new(s(project,"rootPath"));
            let root=match path.canonicalize() {Ok(root)=>root,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>continue,Err(_)=>return Err(HELD.into())};
            if parent.starts_with(root) { return Err(HELD.into()); }
        }
        if self.report_path.try_exists().map_err(|_|HELD)? {
            let metadata=fs::symlink_metadata(&self.report_path).map_err(|_|HELD)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len()>262_144 {return Err(HELD.into());}
            let old:Value=serde_json::from_slice(&fs::read(&self.report_path).map_err(|_|HELD)?).map_err(|_|HELD)?;
            if old["schemaVersion"]!=CONTRACT || old["requestSha256"]!=self.hash()? {return Err(HELD.into());}
        }
        Ok(())
    }
}

/// Called only by the explicit --maintain-task-records entrypoint.
pub fn run_file(input: &Path) -> Result<i32> {
    if !input.is_absolute() || fs::metadata(input).map_err(|_|HELD)?.len()>262_144 {return Err(HELD.into());}
    let request:Request=serde_json::from_slice(&fs::read(input).map_err(|_|HELD)?).map_err(|_|HELD)?;
    request.validate(input)?;
    crate::windows_runtime::verify_shared_directory(&request.home)?;
    let _lease=background_chrome_images::HomeExecutionLease::acquire(&request.home)?;
    crate::platform::ensure_isolated_home(&request.home)?;
    let engine=Engine::build_with_recovery(request.home.clone(),Arc::new(crate::execution_policy::OfflineChannels),false,true,false)?;
    request.check_report(&engine)?;
    let result=execute(&engine,&request);
    let report=match &result {
        Ok(jobs)=>json!({"schemaVersion":CONTRACT,"requestSha256":request.hash()?,"projectId":request.project_id,
            "action":request.action,"status":"completed","jobs":jobs,"generationRequestsSent":0,"originalEvidencePreserved":true}),
        Err(_)=>json!({"schemaVersion":CONTRACT,"requestSha256":request.hash()?,"projectId":request.project_id,
            "action":request.action,"status":"held","errorCode":"TASK_RECORD_MAINTENANCE_HELD","generationRequestsSent":0}),
    };
    atomic_json(&request.report_path,&report)?;
    result.map(|_|0)
}

fn execute(engine:&Arc<Engine>, request:&Request) -> Result<Vec<Value>> {
    let store=engine.store(&request.project_id)?;
    let source=store.source()?;
    if source["sha256"]!=request.source_sha256 || source["version"]!=request.source_version {return Err(HELD.into());}
    // Validate the entire selection before the first visibility mutation.
    for binding in &request.jobs {
        let job=store.job(&binding.job_id)?;
        if pipeline::digest(&job["snapshot"].to_string())!=binding.snapshot_sha256
            || !store.task_record_is_terminal_failure(&binding.job_id)? {return Err(HELD.into());}
    }
    let mut reports=Vec::new();
    for binding in &request.jobs {
        engine.dispatch(json!({"action":request.action,"projectId":request.project_id,"jobId":binding.job_id,
            "expectedSnapshotSha256":binding.snapshot_sha256}))?;
        let archived=store.archived_task_record_ids()?.contains(&binding.job_id);
        if archived != (request.action=="jobs.archive") {return Err(HELD.into());}
        reports.push(json!({"jobId":binding.job_id,"taskRecordArchived":archived}));
    }
    Ok(reports)
}
