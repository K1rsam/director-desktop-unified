//! Explicit persisted image-only runner. No Tauri, text workflow or global scheduler.
//! Standard desktop and this runner hold the same OS file lease at their entrypoints.
use super::*;
use serde::{Deserialize, Serialize};

const CONTRACT: &str = "background-chrome-images/v1";
const HELD: &str = "BACKGROUND_CHROME_HELD: 请求或原任务身份不一致；保留任务，不重发";

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
    atomic_json(&r.report_path,&json!({"schemaVersion":CONTRACT,"requestId":r.request_id,"requestSha256":r.hash()?,
        "projectId":r.project_id,"sourceSha256":r.source_sha256,"pid":std::process::id(),"status":status,
        "queuePhase":journal.phase,"operationJobId":operation,"updatedAt":now(),"jobs":jobs,
        "errorCode":match status {"held"=>Some("BACKGROUND_CHROME_HELD"),"stopped"=>Some("BACKGROUND_CHROME_STOPPED"),"timed_out"=>Some("BACKGROUND_CHROME_TIMEOUT"),_=>None},
        "scope":{"text":false,"video":false,"otherProviders":false,"otherProjects":false}}))
}

fn selected_step(engine:&Arc<Engine>,r:&Request,store:&Store,journal:&Journal) -> Result<bool> {
    r.validate_source_assets(store)?;
    let ids:HashSet<_>=journal.jobs.iter().map(|j|s(j,"jobId").to_owned()).collect();
    // Check the entire selected set before a dependency resolver can write.
    for identity in &journal.jobs {
        let job=store.job(s(identity,"jobId"))?;
        r.job_bound(&job)?;
        if job_identity(&job)!=*identity {return Err(HELD.into());}
        if job["status"]=="waiting_dependency" && job["snapshot"]["references"]!=json!([]) {return Err(HELD.into());}
    }
    engine.unblock_image_dependencies_selected(store,Some(&ids))?;
    let mut jobs=journal.jobs.iter().map(|j|store.job(s(j,"jobId"))).collect::<Result<Vec<_>>>()?;
    // Known originals precede new work; unknown tasks have no generation path.
    jobs.sort_by_key(|j|u8::from(s(j,"providerTaskId").is_empty()));
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
        let generate=!known && ["queued","ready"].contains(&state);
        if !generate && !(known && ["submitted","processing","succeeded","query_failed","download_failed"].contains(&state)) {continue;}
        if generate && !engine.chrome_image_queue_ready(store,key)? {continue;}
        let (config,account)=engine.generation_config(store,&job)?;
        if config.kind!="chrome" {return Err(HELD.into());}
        // Includes source/asset versions and exact adopted parent media hashes.
        binding::validate_generation_snapshot(store,&job["snapshot"])?;
        persist_report(r,journal,store,"running",Some(key))?;
        let account=match engine.checked_account(&config,&account) {
            Ok(account)=>account,
            Err(error)=>{Engine::account_preflight_failed(store,key,&error)?;return Ok(true);}
        };
        if generate {tasks::run_generation(store,key,&config,&account,engine.channels.as_ref())?;}
        else if ["succeeded","download_failed"].contains(&state) {
            tasks::download_saved_result(store,key,&config,&account,engine.channels.as_ref())?;
        } else {tasks::resume_job(store,key,&config,&account,engine.channels.as_ref(),true)?;}
        return Ok(true);
    }
    // Cleanup is restricted to the same exact completed jobs; original unknown pages remain.
    for key in tasks::due_browser_tab_cleanups(store)?.iter().filter(|key|ids.contains(key.as_str())) {
        let job=store.job(key)?;
        r.job_bound(&job)?;
        let (config,_)=engine.generation_config(store,&job)?;
        if config.kind!="chrome" {return Err(HELD.into());}
        tasks::run_browser_tab_cleanup(store,key,&config,engine.channels.as_ref())?;
        return Ok(true);
    }
    Ok(false)
}

fn run_owned(engine:Arc<Engine>,r:&Request) -> Result<i32> {
    r.validate()?;
    let store=engine.store(&r.project_id)?;
    if r.stop_path.try_exists().map_err(|e|e.to_string())? {
        let empty=Journal{version:CONTRACT.into(),request_sha256:r.hash()?,request_id:r.request_id.clone(),project_id:r.project_id.clone(),source_sha256:r.source_sha256.clone(),phase:"not_started".into(),jobs:vec![]};
        persist_report(r,&empty,&store,"stopped",None)?;return Ok(3);
    }
    let journal=prepare_queue(&engine,r,&store)?;
    let start=Instant::now();
    loop {
        let status=if r.stop_path.try_exists().map_err(|e|e.to_string())? {Some(("stopped",3))}
            else if start.elapsed()>=Duration::from_secs(r.max_runtime_seconds) {Some(("timed_out",3))}
            else {None};
        if let Some((status,code))=status {persist_report(r,&journal,&store,status,None)?;return Ok(code);}
        let progressed=match selected_step(&engine,r,&store,&journal) {
            Ok(value)=>value,
            Err(error)=>{persist_report(r,&journal,&store,"held",None)?;return Err(error);}
        };
        let jobs=journal.jobs.iter().map(|j|store.job(s(j,"jobId"))).collect::<Result<Vec<_>>>()?;
        let complete=journal.phase=="queued" && !jobs.is_empty() && jobs.iter().all(|j|j["status"]=="downloaded");
        if complete {
            for job in &jobs {
                let media=job["resultMediaIds"].as_array().filter(|ids|!ids.is_empty()).ok_or(HELD)?;
                for mid in media {store.verify_media(&store.get("media",mid.as_str().ok_or(HELD)?)?)?;}
                if store.adopted(s(job,"entityId"),s(&job["snapshot"],"variantId"))?.as_deref()!=media[0].as_str() {return Err(HELD.into());}
            }
            persist_report(r,&journal,&store,"completed",None)?;return Ok(0);
        }
        let runnable=jobs.iter().any(|j|["queued","ready","submitted","processing","succeeded","query_failed","download_failed"].contains(&s(j,"status")));
        let unknown=jobs.iter().any(|j|["submission_unknown","auth_required","recovery_paused"].contains(&s(j,"status")));
        let known_pending=jobs.iter().any(|j|!s(j,"providerTaskId").is_empty() && ["submitted","processing","succeeded","query_failed","download_failed"].contains(&s(j,"status")));
        if !progressed && (!runnable || unknown && !known_pending) {
            persist_report(r,&journal,&store,"held",None)?;return Ok(2);
        }
        persist_report(r,&journal,&store,"running",None)?;
        if !progressed {std::thread::sleep(Duration::from_millis(r.poll_interval_ms));}
    }
}

#[cfg(test)]
#[path="background_chrome_images_tests.rs"]
mod tests;
