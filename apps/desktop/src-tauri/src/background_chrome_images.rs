//! Explicit persisted image-only runner. No Tauri, text workflow or global scheduler.
//! Standard desktop and this runner hold the same OS file lease at their entrypoints.
use super::*;
use serde::{Deserialize, Serialize};
use rusqlite::OptionalExtension;

const CONTRACT: &str = "background-chrome-images/v1";
const HELD: &str = "BACKGROUND_CHROME_HELD: 请求或原任务身份不一致；保留任务，不重发";
const CLEANUP_FINISH_TIMEOUT: Duration = Duration::from_secs(90);

/// OS ownership, not a PID marker. Crash/exit releases the lock automatically.
pub struct HomeExecutionLease { _file: fs::File }
impl HomeExecutionLease {
    pub fn acquire(home: &Path) -> Result<Self> {
        if !home.is_absolute() { return Err("执行目录必须是绝对路径".into()); }
        fs::create_dir_all(home).map_err(|e|e.to_string())?;
        let home = fs::canonicalize(home).map_err(|e|e.to_string())?;
        let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .open(home.join(".execution-owner.lock")).map_err(|e|e.to_string())?;
        file.try_lock().map_err(|_|"HOME_EXECUTION_BUSY: 当前桌面端或后台图片队列已拥有执行目录；未启动第二个执行器")?;
        Ok(Self { _file: file })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub struct AssetBinding {
    pub asset_id: String,
    pub variant_id: String,
    pub asset_version: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub struct Request {
    pub schema_version: String,
    pub request_id: String,
    pub home: PathBuf,
    pub project_id: String,
    pub source_sha256: String,
    pub source_version: i64,
    pub assets: Vec<AssetBinding>,
    pub selected_asset_ids: Vec<String>,
    pub model: String,
    pub image_production_policy: String,
    pub report_path: PathBuf,
    pub stop_path: PathBuf,
    pub max_runtime_seconds: u64,
    pub poll_interval_ms: u64,
    #[serde(default)]
    pub resume_job_ids: Vec<String>,
}
impl Request {
    pub fn from_file(path: &Path) -> Result<Self> {
        if !path.is_absolute() || fs::metadata(path).map_err(|e|e.to_string())?.len()>262_144 { return Err(HELD.into()); }
        let request: Self = serde_json::from_slice(&fs::read(path).map_err(|e|e.to_string())?).map_err(|_|HELD)?;
        request.validate()?;
        if request.report_path == path || request.stop_path == path { return Err(HELD.into()); }
        Ok(request)
    }
    fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        let selected: HashSet<_> = self.selected_asset_ids.iter().collect();
        if self.schema_version != CONTRACT || uuid::Uuid::parse_str(&self.request_id).is_err()
            || self.project_id.trim().is_empty() || self.source_version<=0
            || self.source_sha256.len()!=64 || !self.source_sha256.bytes().all(|c|c.is_ascii_hexdigit())
            || !self.home.is_absolute() || !self.report_path.is_absolute() || !self.stop_path.is_absolute()
            || self.report_path == self.stop_path || self.model != "chatgpt-image"
            || self.image_production_policy != "automatic" || self.assets.is_empty() || self.assets.len()>1000
            || selected.is_empty() || selected.len()!=self.selected_asset_ids.len()
            || !(1..=86_400).contains(&self.max_runtime_seconds) || !(250..=5000).contains(&self.poll_interval_ms)
            || self.assets.iter().any(|a|a.asset_id.trim().is_empty() || a.variant_id.trim().is_empty() || a.asset_version<=0 || !ids.insert(a.asset_id.clone()))
            || selected.iter().any(|id|!ids.contains(id.as_str()))
            || self.resume_job_ids.iter().any(|id|id.trim().is_empty())
            || self.resume_job_ids.iter().collect::<HashSet<_>>().len()!=self.resume_job_ids.len()
        { return Err(HELD.into()); }
        Ok(())
    }
    fn hash(&self) -> Result<String> { Ok(pipeline::digest(&serde_json::to_value(self).map_err(|e|e.to_string())?.to_string())) }
    fn binding(&self) -> Result<Value> {
        Ok(json!({"version":CONTRACT,"requestId":self.request_id,"requestSha256":self.hash()?,
            "projectId":self.project_id,"sourceSha256":self.source_sha256,"sourceVersion":self.source_version}))
    }
    fn validate_source_assets(&self, store: &Store) -> Result<()> {
        let source=store.source()?;
        if store.project()?["id"] != self.project_id || source["sha256"]!=self.source_sha256 || source["version"]!=self.source_version { return Err(HELD.into()); }
        for binding in &self.assets {
            let a=store.active_asset(&binding.asset_id)?;
            if a["variantId"]!=binding.variant_id || a["version"]!=binding.asset_version { return Err(HELD.into()); }
        }
        let assets=store.values("assets")?;
        let selection=json!(self.selected_asset_ids);
        let (closure, issues)=image_closure(store,&assets,selection.as_array());
        if !issues.is_empty() || closure.iter().any(|id|!self.assets.iter().any(|a| &a.asset_id==id)) { return Err(HELD.into()); }
        Ok(())
    }
    fn job_bound(&self, job: &Value) -> Result<()> {
        let a=self.assets.iter().find(|a|job["entityId"]==a.asset_id).ok_or(HELD)?;
        if job["kind"]!="image" || job["provider"]!="chrome" || job["snapshot"]["kind"]!="image"
            || job["snapshot"]["assetId"]!=a.asset_id || job["snapshot"]["variantId"]!=a.variant_id
            || job["snapshot"]["assetVersion"]!=a.asset_version || job["snapshot"]["sourceVersion"]!=self.source_version
            || job["snapshot"]["model"]!=self.model || s(&job["snapshot"],"requestId").is_empty()
            || (job["snapshot"]["backgroundChromeBinding"]!=self.binding()? && !self.resume_job_ids.iter().any(|id|job["id"]==*id))
        { return Err(HELD.into()); }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Journal {
    version: String,
    request_sha256: String,
    request_id: String,
    project_id: String,
    source_sha256: String,
    phase: String,
    jobs: Vec<Value>,
}
fn job_identity(job: &Value) -> Value {
    // Only dependency reference resolution may populate this field later.
    // Prompt, source chain, settings, model and every other frozen byte stay bound.
    let mut invariant=job["snapshot"].clone();
    if let Some(obj)=invariant.as_object_mut() {obj.remove("references");}
    json!({"jobId":job["id"],"assetId":job["entityId"],"variantId":job["snapshot"]["variantId"],
        "assetVersion":job["snapshot"]["assetVersion"],"requestId":job["snapshot"]["requestId"],
        "frozenSnapshotSha256":pipeline::digest(&invariant.to_string())})
}
fn journal_file(r: &Request) -> PathBuf { r.home.join("background-chrome-images").join(format!("{}.json",r.request_id)) }

/// Public executable entry: deliberately no generic RPC/action/provider field.
pub fn run_file(path: &Path) -> Result<i32> {
    let request=Request::from_file(path)?;
    let result=run_request(&request);
    if let Err(error)=&result {
        // GUI-subsystem executables have no reliable stderr. Persist only a fixed
        // error class, never raw upstream bodies, prompts or credential-bearing URLs.
        let code=if error.starts_with("HOME_EXECUTION_BUSY") {"HOME_EXECUTION_BUSY"}
            else if error.starts_with("BACKGROUND_CHROME_HELD") {"BACKGROUND_CHROME_HELD"}
            else {"BACKGROUND_CHROME_FAILED"};
        let report_exists=request.report_path.try_exists().unwrap_or(true);
        let old=fs::metadata(&request.report_path).ok().filter(|m|m.len()<=1_048_576)
            .and_then(|_|fs::read(&request.report_path).ok()).and_then(|b|serde_json::from_slice::<Value>(&b).ok());
        if !report_exists || old.as_ref().is_some_and(|v|v["schemaVersion"]==CONTRACT && v["requestId"]==request.request_id && v["requestSha256"]==request.hash().unwrap_or_default()) {
            let mut report=old.unwrap_or_else(||json!({"schemaVersion":CONTRACT,"requestId":request.request_id,"requestSha256":request.hash().unwrap_or_default(),"projectId":request.project_id,"sourceSha256":request.source_sha256,"jobs":[]}));
            report["status"]=json!("held");report["errorCode"]=json!(code);report["updatedAt"]=json!(now());
            let _=atomic_json(&request.report_path,&report);
        }
    }
    result
}
fn make_engine(home:PathBuf,channels:Arc<dyn Channels>,live_accounts:bool) -> Result<Arc<Engine>> {
    let mut engine=Engine::build_with_recovery(home,channels,live_accounts,false,false)?;
    Arc::get_mut(&mut engine).ok_or(HELD)?.standard_native=true;
    Ok(engine)
}
fn run_request(request:&Request) -> Result<i32> {
    crate::windows_runtime::verify_shared_directory(&request.home)?;
    let _lease=HomeExecutionLease::acquire(&request.home)?;
    crate::platform::ensure_isolated_home(&request.home)?;
    // Do not read a saved text acceptance grant or reconcile any other project.
    let engine=make_engine(request.home.clone(),Arc::new(crate::tasks::RealChannels),true)?;
    run_owned(engine,request)
}

fn prepare_queue(engine: &Engine, r: &Request, store: &Store) -> Result<Journal> {
    r.validate_source_assets(store)?;
    let path=journal_file(r);
    let hash=r.hash()?;
    let existing=path.try_exists().map_err(|e|e.to_string())?;
    let mut journal=if existing {
        if fs::metadata(&path).map_err(|e|e.to_string())?.len()>1_048_576 {return Err(HELD.into());}
        let journal:Journal=serde_json::from_slice(&fs::read(&path).map_err(|e|e.to_string())?).map_err(|_|HELD)?;
        if journal.version!=CONTRACT || journal.request_sha256!=hash || journal.request_id!=r.request_id
            || journal.project_id!=r.project_id || journal.source_sha256!=r.source_sha256 { return Err(HELD.into()); }
        journal
    } else {
        let journal=Journal {version:CONTRACT.into(),request_sha256:hash,request_id:r.request_id.clone(),
            project_id:r.project_id.clone(),source_sha256:r.source_sha256.clone(),phase:"queue_intent".into(),jobs:vec![]};
        // Before queue allocation. An interrupted intent is never replayed as a fresh batch.
        fs::create_dir_all(path.parent().unwrap()).map_err(|e|e.to_string())?;
        atomic_json(&path,&serde_json::to_value(&journal).map_err(|e|e.to_string())?)?;
        journal
    };
    if !existing {
        let all=store.jobs()?;
        for job in &all {
            if job["kind"]=="image" && retains_image_generation(job) && r.assets.iter().any(|a|job["entityId"]==a.asset_id) {
                r.job_bound(job)?; // Cannot reuse a different provider, version or unapproved original job.
            }
        }
        let request=json!({"provider":"chrome","model":r.model,"assetIds":r.selected_asset_ids,
            "imageProductionPolicy":"automatic","backgroundChromeBinding":r.binding()?});
        let report=engine.queue_images(store,&request)?;
        for item in report["queued"].as_array().into_iter().flatten().chain(report["reused"].as_array().into_iter().flatten()) {
            let job=store.job(s(item,"jobId"))?;
            r.job_bound(&job)?;
            journal.jobs.push(job_identity(&job));
        }
        journal.phase=if report["blockedCount"].as_u64().unwrap_or(0)>0 {"partially_queued"} else {"queued"}.into();
        atomic_json(&path,&serde_json::to_value(&journal).map_err(|e|e.to_string())?)?;
    } else if journal.phase=="queue_intent" {
        // Discover only this exact frozen intent after a crash; no full-batch replay.
        let binding=r.binding()?;
        for job in store.jobs()?.iter().filter(|j|j["snapshot"]["backgroundChromeBinding"]==binding) {
            r.job_bound(job)?;
            journal.jobs.push(job_identity(job));
        }
        journal.phase="interrupted_queue_intent".into();
        atomic_json(&path,&serde_json::to_value(&journal).map_err(|e|e.to_string())?)?;
    }
    let mut ids=HashSet::new();
    for identity in &journal.jobs {
        if !ids.insert(s(identity,"jobId")) {return Err(HELD.into());}
        let job=store.job(s(identity,"jobId"))?;
        r.job_bound(&job)?;
        if job_identity(&job)!=*identity {return Err(HELD.into());}
        // Only our selected image jobs. No broad recover_interrupted call.
        let state=s(&job,"status");
        let to=match state {"preparing"=>Some("recovery_paused"),"submitting"=>Some(if s(&job,"providerTaskId").is_empty(){"submission_unknown"}else{"submitted"}),"downloading"=>Some("download_failed"),_=>None};
        if let Some(to)=to {store.claim_job_fields(s(identity,"jobId"),state,to,&json!({"error":"后台图片执行中断，保留原请求和结果；不重新提交未知任务"}))?;}
    }
    Ok(journal)
}

fn persist_report(r:&Request,journal:&Journal,store:&Store,status:&str,operation:Option<&str>) -> Result<()> {
    if r.report_path.try_exists().map_err(|e|e.to_string())? {
        let old:Value=serde_json::from_slice(&fs::read(&r.report_path).map_err(|e|e.to_string())?).map_err(|_|HELD)?;
        if old["schemaVersion"]!=CONTRACT || old["requestId"]!=r.request_id || old["requestSha256"]!=r.hash()? {return Err(HELD.into());}
    }
    let mut jobs=vec![];
    for binding in &journal.jobs {
        let job=store.job(s(binding,"jobId"))?;
        let adopted=store.adopted(s(&job,"entityId"),s(&job["snapshot"],"variantId"))?;
        jobs.push(json!({"jobId":job["id"],"requestId":job["snapshot"]["requestId"],"assetId":job["entityId"],
            "status":job["status"],"providerTaskId":job["providerTaskId"],"nextAttemptAt":job["nextAttemptAt"],
            "mediaIds":job["resultMediaIds"],"adoptedMediaId":adopted}));
    }
    let images_complete=journal.phase=="queued" && !jobs.is_empty() && jobs.iter().all(|job|job["status"]=="downloaded");
    let cleanup=cleanup_summary(store,journal)?;
    atomic_json(&r.report_path,&json!({"schemaVersion":CONTRACT,"requestId":r.request_id,"requestSha256":r.hash()?,
        "projectId":r.project_id,"sourceSha256":r.source_sha256,"pid":std::process::id(),"status":status,
        "queuePhase":journal.phase,"operationJobId":operation,"updatedAt":now(),"jobs":jobs,
        "imagesComplete":images_complete,"cleanup":cleanup,
        "cleanupFinishTimeoutSeconds":CLEANUP_FINISH_TIMEOUT.as_secs(),
        "errorCode":match status {"held"=>Some("BACKGROUND_CHROME_HELD"),"stopped"=>Some("BACKGROUND_CHROME_STOPPED"),"timed_out"=>Some("BACKGROUND_CHROME_TIMEOUT"),"completed_cleanup_pending"=>Some("BACKGROUND_CHROME_CLEANUP_PENDING"),_=>None},
        "scope":{"text":false,"video":false,"otherProviders":false,"otherProjects":false}}))
}

/// Read only the exact journal IDs. No all-project legacy backfill or media calls.
fn cleanup_summary(store:&Store,journal:&Journal) -> Result<Value> {
    let mut rows=vec![];
    let mut done=0;let mut retained=0;let mut pending=0;let mut not_ready=0;
    for binding in &journal.jobs {
        let key=s(binding,"jobId");
        let raw:Option<String>=store.conn()?.query_row("SELECT value FROM metadata WHERE key=?1",[format!("browser_tab_cleanup:{key}")],|row|row.get(0))
            .optional().map_err(|e|e.to_string())?;
        let value:Value=raw.as_deref().map(serde_json::from_str).transpose().map_err(|_|HELD)?.unwrap_or(Value::Null);
        let status=match s(&value,"status") {"done"=>{done+=1;"done"},"retained"=>{retained+=1;"retained"},"pending"=>{pending+=1;"pending"},"" if value.is_null()=>{not_ready+=1;"not_ready"},_=>return Err(HELD.into())};
        let reason=s(&value["outcome"],"reason");
        let disposition=if status=="done" {"closed"} else if value["outcome"]["status"]=="retained" {"retained"} else {"unconfirmed"};
        rows.push(json!({"jobId":key,"status":status,"pageDisposition":disposition,"reason":reason,
            "outcomeStatus":value["outcome"]["status"],"nextAttemptAt":value["nextAttemptAt"],"attempts":value["attempts"]}));
    }
    Ok(json!({"done":done,"pending":pending,"retained":retained,"notReady":not_ready,"jobs":rows}))
}

fn selected_cleanup_step(engine:&Arc<Engine>,r:&Request,store:&Store,journal:&Journal) -> Result<bool> {
    // Narrow backfill: the helper is passed only a proven selected downloaded ID.
    for saved in &journal.jobs {
        let job=store.job(s(saved,"jobId"))?;
        r.job_bound(&job)?;
        if job_identity(&job)!=*saved {return Err(HELD.into());}
        if job["status"]=="downloaded" {tasks::queue_browser_tab_cleanup(store,s(saved,"jobId"))?;}
    }
    for row in cleanup_summary(store,journal)?["jobs"].as_array().into_iter().flatten() {
        if row["status"]!="pending" || row["nextAttemptAt"].as_i64().unwrap_or(0)>chrono::Utc::now().timestamp() {continue;}
        let key=s(row,"jobId");let job=store.job(key)?;
        if job["status"]!="downloaded" {continue;}
        let (config,_)=engine.generation_config(store,&job)?;
        if config.kind!="chrome" {return Err(HELD.into());}
        // Original helper proves owned page, completed media and idle state. It
        // persists pending/retained/done; this runner never infers closure itself.
        tasks::run_browser_tab_cleanup(store,key,&config,engine.channels.as_ref())?;
        return Ok(true);
    }
    Ok(false)
}

fn recover_selected_dependency_blocks(r:&Request,store:&Store,journal:&Journal)->Result<bool> {
    let assets=store.values("assets")?;
    let mut changed=false;
    // A legacy parent prepare failure is recoverable without replacing either ID.
    // Only the exact historical dependency failure is eligible; unknown submits,
    // edited source chains and unrelated blocked errors remain untouched.
    for saved in &journal.jobs {
        let job=store.job(s(saved,"jobId"))?;
        if job["status"]!="blocked" || job["error"]!="指定来源图片任务未完成，请处理该来源后重新制作关联图片" {continue;}
        r.job_bound(&job)?;
        if job_identity(&job)!=*saved {return Err(HELD.into());}
        if !job["prepared"].is_null() || !job["result"].is_null() || !s(&job,"providerTaskId").is_empty()
            || !(job["receipt"].is_null() || job["receipt"]==json!({}) || job["receipt"]=="")
            || job["snapshot"]["references"]!=json!([]) {continue;}
        binding::validate_generation_snapshot(store,&job["snapshot"])?;
        let asset=store.active_asset(s(&job,"entityId"))?;
        if !job["snapshot"]["sourceChain"].is_array()
            || json!(pipeline::asset_design_contract::source_chain(&assets,&asset)?)!=job["snapshot"]["sourceChain"] {return Err(HELD.into());}
        let deps=job["snapshot"]["dependencyJobIds"].as_array().filter(|a|!a.is_empty()).ok_or(HELD)?;
        let expected=image_dependency_ids(&assets,&asset)?;
        let mut parent_entities=HashSet::new();let mut all_available=true;let mut restored_parent=false;
        for dep in deps {
            let id=dep.as_str().ok_or(HELD)?;
            let parent_saved=journal.jobs.iter().find(|j|j["jobId"]==id).ok_or(HELD)?;
            let parent=store.job(id)?;r.job_bound(&parent)?;
            if job_identity(&parent)!=*parent_saved || !parent_entities.insert(s(&parent,"entityId").to_owned()) {return Err(HELD.into());}
            let event=if parent["status"]=="waiting_dependency" {"chrome_dependency_recovery"} else {"chrome_prepare_recovery_started"};
            let restored:bool=store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type=?2)",rusqlite::params![id,event],|row|row.get(0)).map_err(|e|e.to_string())?;
            restored_parent|=restored && (parent["status"]=="waiting_dependency" || !parent["prepared"].is_null());
            if !["waiting_dependency","queued","ready","submitted","processing","succeeded","downloaded","query_failed","download_failed"].contains(&s(&parent,"status")) {all_available=false;}
        }
        // The frozen dependencyJobIds may omit references already adopted when
        // this job was queued. Every saved dependency still has to be a current
        // declared reference; the journal binds the original complete ID list.
        if parent_entities.iter().any(|id|!expected.contains(id)) {return Err(HELD.into());}
        if !all_available || !restored_parent {continue;}
        let mut c=store.conn()?;
        let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let mut q=tx.prepare("SELECT type,data FROM events WHERE job_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
        let history=q.query_map([s(&job,"id")],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).map_err(|e|e.to_string())?
            .collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        drop(q);
        let mut blocked_proven=false;let mut submitted=false;
        for (kind,raw) in &history {
            let value:Value=serde_json::from_str(raw).map_err(|_|HELD)?;
            if kind=="state" && value["state"]=="blocked" && value["error"]==job["error"] {blocked_proven=true;}
            submitted|=kind.contains("submit") || kind.contains("query") || kind.contains("download") || kind.contains("reconcile")
                || ["ready","preparing","submitting","submitted","processing","submission_unknown","succeeded","downloading","downloaded"].iter().any(|state|value["state"]==*state || value["from"]==*state);
        }
        if !blocked_proven || submitted {continue;}
        let count=tx.execute("UPDATE jobs SET state='waiting_dependency',next_attempt_at=0,updated_at=?1 WHERE id=?2 AND state='blocked' AND snapshot=?3 AND error=?4 AND prepared IS NULL AND result IS NULL AND (task_id IS NULL OR task_id='') AND (receipt IS NULL OR receipt IN ('','null','{}'))",
            rusqlite::params![now(),s(&job,"id"),job["snapshot"].to_string(),s(&job,"error")]).map_err(|e|e.to_string())?;
        if count==1 {
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_dependency_recovery',?2,?3)",rusqlite::params![s(&job,"id"),
                json!({"version":"chrome-dependency-recovery/v1","previousState":"blocked","previousError":job["error"],"snapshotSha256":pipeline::digest(&job["snapshot"].to_string()),"eventHistorySha256":pipeline::digest(&json!(history).to_string()),"dependencyJobIds":deps,"sameRequestId":true,"generationAllowed":false}).to_string(),now()]).map_err(|e|e.to_string())?;
            changed=true;
        }
        tx.commit().map_err(|e|e.to_string())?;
    }
    if changed {store.dirty()?;}
    Ok(changed)
}

#[cfg(test)]
fn selected_step(engine:&Arc<Engine>,r:&Request,store:&Store,journal:&Journal) -> Result<bool> {
    selected_step_with_paused_queries(engine,r,store,journal,&mut HashSet::new())
}
fn selected_step_with_paused_queries(engine:&Arc<Engine>,r:&Request,store:&Store,journal:&Journal,paused_queries:&mut HashSet<String>) -> Result<bool> {
    r.validate_source_assets(store)?;
    let ids:HashSet<_>=journal.jobs.iter().map(|j|s(j,"jobId").to_owned()).collect();
    let mut prepare_budgets_retired=false;
    // Check the entire selected set before a dependency resolver can write.
    for identity in &journal.jobs {
        let job=store.job(s(identity,"jobId"))?;
        r.job_bound(&job)?;
        if job_identity(&job)!=*identity {return Err(HELD.into());}
        if job["status"]=="waiting_dependency" && job["snapshot"]["references"]!=json!([]) {return Err(HELD.into());}
    }
    for identity in &journal.jobs {
        let job=store.job(s(identity,"jobId"))?;
        if ["failed","recovery_paused","preparing"].contains(&s(&job,"status")) {
            let (cfg,_)=engine.generation_config(store,&job)?;
            prepare_budgets_retired|=tasks::retire_exhausted_chrome_prepare(store,&job,&cfg)?;
        }
    }
    let dependencies_restored=recover_selected_dependency_blocks(r,store,journal)?;
    engine.unblock_image_dependencies_selected(store,Some(&ids))?;
    let mut jobs=journal.jobs.iter().map(|j|store.job(s(j,"jobId"))).collect::<Result<Vec<_>>>()?;
    // Known originals precede new work; unknown tasks have no generation path.
    jobs.sort_by_key(|j|(u8::from(s(j,"providerTaskId").is_empty()),j["nextAttemptAt"].as_i64().unwrap_or(0)));
    for job in jobs {
        r.job_bound(&job)?;
        let state=s(&job,"status");
        let key=s(&job,"id");
        if journal.jobs.iter().find(|saved|saved["jobId"]==job["id"]) != Some(&job_identity(&job)) {return Err(HELD.into());}
        if !job["prepared"].is_null() {
            let p:crate::providers::PreparedRequest=serde_json::from_value(job["prepared"].clone()).map_err(|_|HELD)?;
            if p.provider!="chrome" || serde_json::to_value(&p.request).map_err(|e|e.to_string())?
                != serde_json::to_value(tasks::generation_request(&job["snapshot"])?).map_err(|e|e.to_string())? {return Err(HELD.into());}
        }
        if job["nextAttemptAt"].as_i64().unwrap_or(0)>chrono::Utc::now().timestamp() {continue;}
        let known=!s(&job,"providerTaskId").is_empty();
        let (config,account)=engine.generation_config(store,&job)?;
        let recover_prepare=!known && ["failed","recovery_paused","preparing"].contains(&state)
            && tasks::chrome_prepare_recovery_eligible(store,&job,&config)?;
        let generate=!known && ["queued","ready"].contains(&state);
        let refresh_marker=format!("prepared:{key}");
        let refresh_prepared=!known&&!paused_queries.contains(&refresh_marker)&&tasks::prepared_runtime_refresh_eligible(store,&job,&config)?;
        // An explicit batch continuation may read each exhausted KNOWN original
        // once. It never resets retry budgets or calls prepare/submit; a failed
        // recovery remains held for the rest of this invocation.
        let recover_paused_query=known && state=="recovery_paused" && !paused_queries.contains(key);
        if !generate && !recover_prepare && !refresh_prepared && !recover_paused_query && !(known && ["submitted","processing","succeeded","query_failed","download_failed"].contains(&state)) {continue;}
        // Refresh only reads the original page and updates its transport proof.
        // The generation queue accepts queued/ready jobs, so a failed original
        // must first be restored; its eventual first submit still passes that gate.
        if generate && !engine.chrome_image_queue_ready(store,key)? {continue;}
        if refresh_prepared && engine.chrome_image_queue_blocker(store,key)?.is_some() {continue;}
        if config.kind!="chrome" {return Err(HELD.into());}
        // Includes source/asset versions and exact adopted parent media hashes.
        binding::validate_generation_snapshot(store,&job["snapshot"])?;
        persist_report(r,journal,store,"running",Some(key))?;
        let account=match engine.checked_account(&config,&account) {
            Ok(account)=>account,
            Err(error)=>{Engine::account_preflight_failed(store,key,&error)?;return Ok(true);}
        };
        if recover_paused_query {
            paused_queries.insert(key.to_owned());
            store.event(key,"chrome_paused_original_query_requested",&json!({"providerTaskId":job["providerTaskId"],"requestId":job["snapshot"]["requestId"],"queryAttemptsPreserved":job["queryAttempts"],"prepareCalled":false,"submitCalled":false,"boundedPerBatchContinuation":true}))?;
        }
        if refresh_prepared {
            paused_queries.insert(refresh_marker.clone());
            let refreshed=tasks::refresh_prepared_runtime(store,key,&config,&account,engine.channels.as_ref())?;
            if !refreshed&&store.job(key)?["nextAttemptAt"].as_i64().is_some_and(|at|at>chrono::Utc::now().timestamp()){
                paused_queries.remove(&refresh_marker);return Ok(true);
            }
            return Ok(refreshed);
        }
        if recover_prepare {tasks::resume_chrome_prepare(store,key,&config,&account,engine.channels.as_ref())?;}
        else if generate {tasks::run_generation(store,key,&config,&account,engine.channels.as_ref())?;}
        else if ["succeeded","download_failed"].contains(&state) {
            tasks::download_saved_result(store,key,&config,&account,engine.channels.as_ref())?;
        } else {tasks::resume_job(store,key,&config,&account,engine.channels.as_ref(),true)?;}
        return Ok(true);
    }
    // New generation/recovery operations above retain priority over cleanup.
    Ok(selected_cleanup_step(engine,r,store,journal)? || dependencies_restored || prepare_budgets_retired)
}

fn run_owned(engine:Arc<Engine>,r:&Request) -> Result<i32> {
    run_owned_with_cleanup_budget(engine,r,CLEANUP_FINISH_TIMEOUT)
}
fn run_owned_with_cleanup_budget(engine:Arc<Engine>,r:&Request,cleanup_budget:Duration) -> Result<i32> {
    r.validate()?;
    let store=engine.store(&r.project_id)?;
    if r.stop_path.try_exists().map_err(|e|e.to_string())? {
        let empty=Journal{version:CONTRACT.into(),request_sha256:r.hash()?,request_id:r.request_id.clone(),project_id:r.project_id.clone(),source_sha256:r.source_sha256.clone(),phase:"not_started".into(),jobs:vec![]};
        persist_report(r,&empty,&store,"stopped",None)?;return Ok(3);
    }
    let journal=prepare_queue(&engine,r,&store)?;
    let start=Instant::now();
    let mut cleanup_started:Option<Instant>=None;
    let mut completion_media_verified=false;
    let mut paused_queries=HashSet::new();
    loop {
        let status=if r.stop_path.try_exists().map_err(|e|e.to_string())? {Some(("stopped",3))}
            else if start.elapsed()>=Duration::from_secs(r.max_runtime_seconds) {
                Some(if cleanup_started.is_some() {("completed_cleanup_pending",0)} else {("timed_out",3)})
            }
            else {None};
        if let Some((status,code))=status {persist_report(r,&journal,&store,status,None)?;return Ok(code);}
        let progressed=match selected_step_with_paused_queries(&engine,r,&store,&journal,&mut paused_queries) {
            Ok(value)=>value,
            Err(error)=>{persist_report(r,&journal,&store,"held",None)?;return Err(error);}
        };
        let jobs=journal.jobs.iter().map(|j|store.job(s(j,"jobId"))).collect::<Result<Vec<_>>>()?;
        let complete=journal.phase=="queued" && !jobs.is_empty() && jobs.iter().all(|j|j["status"]=="downloaded");
        if complete {
            if !completion_media_verified {for job in &jobs {
                let media=job["resultMediaIds"].as_array().filter(|ids|!ids.is_empty()).ok_or(HELD)?;
                for mid in media {store.verify_media(&store.get("media",mid.as_str().ok_or(HELD)?)?)?;}
                if store.adopted(s(job,"entityId"),s(&job["snapshot"],"variantId"))?.as_deref()!=media[0].as_str() {return Err(HELD.into());}
            } completion_media_verified=true;}
            let cleanup=cleanup_summary(&store,&journal)?;
            if cleanup["pending"].as_u64().unwrap_or(0)==0 && cleanup["notReady"].as_u64().unwrap_or(0)==0 {
                let status=if cleanup["retained"].as_u64().unwrap_or(0)>0 {"completed_with_retained_pages"} else {"completed"};
                persist_report(r,&journal,&store,status,None)?;return Ok(0);
            }
            let began=cleanup_started.get_or_insert_with(Instant::now);
            if began.elapsed()>=cleanup_budget {
                persist_report(r,&journal,&store,"completed_cleanup_pending",None)?;return Ok(0);
            }
            persist_report(r,&journal,&store,"finishing_cleanup",None)?;
            if !progressed {std::thread::sleep(Duration::from_millis(r.poll_interval_ms));}
            continue;
        }
        let recoverable=jobs.iter().filter(|j|["failed","recovery_paused","preparing"].contains(&s(j,"status")))
            .map(|j|engine.generation_config(&store,j).and_then(|(cfg,_)|tasks::chrome_prepare_recovery_eligible(&store,j,&cfg)))
            .collect::<Result<Vec<_>>>()?.into_iter().any(|eligible|eligible);
        let refreshable=jobs.iter().filter(|j|!paused_queries.contains(&format!("prepared:{}",s(j,"id"))))
            .map(|j|engine.generation_config(&store,j).and_then(|(cfg,_)|tasks::prepared_runtime_refresh_eligible(&store,j,&cfg)))
            .collect::<Result<Vec<_>>>()?.into_iter().any(|eligible|eligible);
        let runnable=recoverable || refreshable || jobs.iter().any(|j|["queued","ready","submitted","processing","succeeded","query_failed","download_failed"].contains(&s(j,"status")));
        let unknown=jobs.iter().any(|j|["submission_unknown","auth_required","recovery_paused"].contains(&s(j,"status")));
        let known_pending=jobs.iter().any(|j|!s(j,"providerTaskId").is_empty() && (["submitted","processing","succeeded","query_failed","download_failed"].contains(&s(j,"status"))
            || j["status"]=="recovery_paused" && !paused_queries.contains(s(j,"id"))));
        if !progressed && (!runnable || unknown && !known_pending && !recoverable) {
            persist_report(r,&journal,&store,"held",None)?;return Ok(2);
        }
        persist_report(r,&journal,&store,"running",None)?;
        if !progressed {std::thread::sleep(Duration::from_millis(r.poll_interval_ms));}
    }
}

#[cfg(test)]
#[path="background_chrome_images_tests.rs"]
mod tests;
