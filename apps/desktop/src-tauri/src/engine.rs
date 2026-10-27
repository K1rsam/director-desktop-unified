use crate::{
    binding,
    dreamina_session::{authorization_url, DreaminaSessions},
    pipeline, portable,
    providers::{Capabilities, ModelCapability, ProviderConfig, ProviderError, ProviderResult},
    store::{atomic_json, id, now, s, Result, Store},
    tasks::{self, Channels},
    text_executor::SavedTextExecutor,
};
#[cfg(test)]
use crate::tasks::RealChannels;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
#[path = "app_settings_engine.rs"]
mod app_settings_engine;
#[path = "background_chrome_images.rs"]
pub mod background_chrome_images;
#[path = "task_record_maintenance.rs"]
pub mod task_record_maintenance;
#[path = "unit_save_guard.rs"]
mod unit_save_guard;
#[path = "lingmo_project.rs"]
mod lingmo_project;
#[path = "engine_refinement_identity.rs"]
mod refinement_identity;
#[path = "engine_asset_workflow.rs"]
mod asset_workflow_engine;
#[path = "engine_source_analysis.rs"]
mod source_analysis_engine;
#[path = "engine_visual_review.rs"]
mod visual_review_engine;
#[cfg(test)]
#[path = "engine_refinement_continuity_tests.rs"]
mod refinement_continuity_tests;
#[cfg(test)]
#[path = "engine_backup_restore_tests.rs"]
mod backup_restore_tests;

// Bound short-lived local CLI/process and file I/O work, not outstanding cloud tasks.
const IMAGE_SUBMIT_WORKERS: usize = 8;
const IMAGE_QUERY_WORKERS: usize = 8;
const IMAGE_DOWNLOAD_WORKERS: usize = 4;

fn image_stage_completion_state(output: &Value) -> &'static str {
    if output["blocked"]
        .as_array()
        .is_some_and(|blocked| !blocked.is_empty())
    {
        "partially_blocked"
    } else {
        "completed"
    }
}

fn image_production_resume_eligible(job: &Value) -> bool {
    s(job, "kind") == "workflow"
        && s(job, "entityId").starts_with("images.produce:")
        && ["completed", "partially_blocked", "failed", "interrupted"].contains(&s(job, "status"))
        && job["snapshot"]["stage"] == "images.produce"
        && job["snapshot"]["request"]["action"] == "images.produce"
        && job["snapshot"]["settings"].is_object()
        && job["snapshot"]["sourceVersion"]
            .as_i64()
            .is_some_and(|version| version > 0)
        && job["result"]["blocked"]
            .as_array()
            .is_some_and(|blocked| !blocked.is_empty())
        && ["partiallyQueued", "partially_blocked"].contains(&s(&job["result"], "status"))
}

/// Project the frozen workflow into automatic design without upgrading or
/// dropping a saved contract. This same path is used for new runs and resumes.
fn image_production_design_input(
    context: &Value,
    assets: &[Value],
    settings: &Value,
    policy: &str,
    active: &[Value],
    wanted: &[String],
) -> Value {
    let mut input = json!({"source":context["source"],"foundation":context["foundation"],"assets":assets,"settings":settings,"comparisonDesignsByAssetId":context["comparisonDesignsByAssetId"],"imageProductionPolicy":policy,"automaticDesign":true,"activeImageAssetIds":active,"selection":{"assetIds":wanted}});
    for key in [
        pipeline::asset_design_contract::COMPARISON_PROFILE_FIELD,
        pipeline::asset_design_contract::FROZEN_COMPARISON_REQUESTS,
        pipeline::asset_design_contract::IDENTITY_MATERIAL_PROFILE_FIELD,
        pipeline::asset_design_contract::IDENTITY_MATERIALS_FIELD,
        pipeline::asset_design_contract::SCOPE_MATERIALS_FIELD,
        pipeline::asset_production_context::PROFILE_FIELD,
        pipeline::asset_design_contract::INLINE_DELTA_PROFILE_FIELD,
        pipeline::DESIGN_EVIDENCE_PROFILE_FIELD,
        pipeline::asset_visual_source_reparse::FIELD,
    ] {
        if let Some(value) = context.get(key) { input[key] = value.clone(); }
    }
    input
}

pub struct Engine {
    pub home: PathBuf,
    pub channels: Arc<dyn Channels>,
    active: Mutex<HashSet<String>>,
    // One shared channel slot alternates generation with original-ID recovery.
    // Restart defaults to recovery first; no durable submission state is changed.
    media_query_next: Mutex<HashMap<String, bool>>,
    image_wake: (Mutex<bool>, Condvar),
    registry_mutex: Mutex<()>,
    // Serializes registry lifecycle changes with synchronous dispatch and worker reservations.
    lifecycle_mutex: Mutex<()>,
    dreamina_sessions: Arc<DreaminaSessions>,
    canvas_sessions: crate::canvas_oauth::CanvasSessions,
    live_accounts: bool,
    offline_only: bool,
    standard_native: bool,
    native_acceptance: Option<Arc<crate::native_acceptance::NativeAcceptance>>,
    readiness_enabled: bool,
    readiness_resources: Option<PathBuf>,
    pub stop: AtomicBool,
}
struct VideoChannel {
    account: String,
    fingerprint: String,
    capabilities: Capabilities,
    report: Value,
}

/// Only prompt compilation metadata is read. Neither full historical drafts,
/// rule bodies nor original media bytes belong in this listing query.
fn ready_prompt_evidence(
    connection: &rusqlite::Connection,
) -> Result<HashMap<String, (Value, Value)>> {
    let mut query = connection.prepare(r#"
        SELECT j.id,
            json_object('unitId',json_extract(j.snapshot,'$.unitId'),
                'unitVersion',json_extract(j.snapshot,'$.unitVersion'),
                'sourceVersion',json_extract(j.snapshot,'$.sourceVersion'),
                'sourcePrompt',json_extract(j.snapshot,'$.sourcePrompt'),
                'sourcePromptSha256',json_extract(j.snapshot,'$.sourcePromptSha256'),
                'origin',json_extract(j.snapshot,'$.origin'),
                'importSource',json_extract(j.snapshot,'$.importSource'),
                'mentions',json_extract(j.snapshot,'$.mentions'),
                'audioAssetRefs',json_extract(j.snapshot,'$.audioAssetRefs'),
                'prompt',json_extract(j.snapshot,'$.prompt'),
                'duration',json_extract(j.snapshot,'$.duration'),
                'ratio',json_extract(j.snapshot,'$.ratio'),
                'resolution',json_extract(j.snapshot,'$.resolution'),
                'references',json_extract(j.snapshot,'$.references'),
                'inlineReferences',COALESCE(json_extract(j.snapshot,'$.inlineReferences'),json('[]'))),
            json_object('id',json_extract(v.data,'$.id'),
                'version',json_extract(v.data,'$.version'),
                'sourceVersion',json_extract(v.data,'$.sourceVersion'),
                'prompt',json_extract(v.data,'$.prompt'),
                'duration',json_extract(v.data,'$.duration'),
                'assetRefs',json_extract(v.data,'$.assetRefs'),
                'origin',json_extract(v.data,'$.origin'),
                'importSource',json_extract(v.data,'$.importSource'),
                'mentions',json_extract(v.data,'$.mentions'),
                'audioAssetRefs',json_extract(v.data,'$.audioAssetRefs'),
                'mediaRefs',json_extract(v.data,'$.mediaRefs'),
                'storyboardCreativeProfile',json_extract(v.data,'$.storyboardCreativeProfile'))
        FROM jobs j LEFT JOIN unit_versions v
            ON v.unit_id=j.entity_id AND v.version=json_extract(j.snapshot,'$.unitVersion')
        WHERE j.kind='video' AND j.state='ready'
    "#).map_err(|e|e.to_string())?;
    let mut evidence = HashMap::new();
    for row in query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
    {
        let (job, snapshot, unit) = row.map_err(|e| e.to_string())?;
        evidence.insert(
            job,
            (crate::store::parse(snapshot)?, crate::store::parse(unit)?),
        );
    }
    Ok(evidence)
}

/// This proves only source-to-compiled-prompt provenance. Actual submission
/// still calls binding::validate_snapshot, including original-media hashing.
fn prepared_source_prompt_provenance(
    job_id: &str,
    current_unit: &Value,
    frozen_unit: &Value,
    snapshot: &Value,
    assets: &[Value],
    source_version: i64,
    settings: &Value,
) -> Result<Value> {
    let imported = s(frozen_unit, "origin") == "imported-storyboard";
    if s(frozen_unit, "id").is_empty()
        || snapshot["unitId"] != frozen_unit["id"]
        || snapshot["unitVersion"]
            .as_i64()
            .filter(|v| *v > 0)
            .is_none()
        || snapshot["unitVersion"] != frozen_unit["version"]
        || current_unit["id"] != frozen_unit["id"]
        || current_unit["version"] != frozen_unit["version"]
        || current_unit["assetRefs"] != frozen_unit["assetRefs"]
        || current_unit["storyboardCreativeProfile"] != frozen_unit["storyboardCreativeProfile"]
        || current_unit["continuityStale"] == true
        || s(frozen_unit, "prompt").trim().is_empty()
    {
        return Err("素材包缺少一致的不可变单元原文版本".into());
    }
    if imported {
        // Imported workspace summaries omit the large prompt and mentions.
        // Prove against the actual frozen unit-version row instead. bind and
        // validate_snapshot separately verify the persisted original Markdown.
        if s(current_unit, "origin") != "imported-storyboard"
            || s(snapshot, "origin") != "imported-storyboard"
            || !snapshot["sourceVersion"].is_null()
            || !frozen_unit["importSource"].is_object()
            || s(&frozen_unit["importSource"], "markdownSha256").is_empty()
            || current_unit["importSource"] != frozen_unit["importSource"]
            || !frozen_unit["mentions"].is_array()
            || ["origin", "importSource", "mentions", "audioAssetRefs"].iter()
                .any(|field| snapshot[*field] != frozen_unit[*field])
            || ["audioAssetRefs", "mediaRefs"].iter()
                .any(|field| current_unit[*field] != frozen_unit[*field])
            || (!s(current_unit, "prompt").is_empty() && current_unit["prompt"] != frozen_unit["prompt"])
            || snapshot["sourcePrompt"] != frozen_unit["prompt"]
            || s(snapshot, "sourcePromptSha256") != pipeline::digest(s(frozen_unit, "prompt"))
        {
            return Err("导入成稿素材包缺少一致的来源、标签或冻结原文证明".into());
        }
    } else if snapshot["sourceVersion"].as_i64() != Some(source_version)
        || source_version <= 0
        || frozen_unit["sourceVersion"] != snapshot["sourceVersion"]
        || current_unit["sourceVersion"] != snapshot["sourceVersion"]
        || current_unit["prompt"] != frozen_unit["prompt"]
        || s(current_unit, "origin") == "imported-storyboard"
        || s(snapshot, "origin") == "imported-storyboard"
    {
        return Err("素材包缺少一致的不可变单元原文版本".into());
    }
    if !snapshot["sourcePrompt"].is_null() || !snapshot["sourcePromptSha256"].is_null() {
        if snapshot["sourcePrompt"] != frozen_unit["prompt"]
            || s(snapshot, "sourcePromptSha256") != pipeline::digest(s(frozen_unit, "prompt"))
        {
            return Err("冻结原提示词或其哈希与单元历史不一致".into());
        }
    }
    let duration =
        binding::normalized_integer_duration(current_unit["duration"].as_f64().unwrap_or(0.0));
    if duration.map(|v| json!(v)).as_ref() != Some(&snapshot["duration"])
        || current_unit["duration"] != frozen_unit["duration"]
        || snapshot["ratio"] != settings["ratio"]
        || snapshot["resolution"]
            != settings
                .get("videoResolution")
                .cloned()
                .unwrap_or(json!("720p"))
    {
        return Err("素材包时长或模型载体参数已变化".into());
    }
    let reference_assets = |reference: &Value| -> Result<&Value> {
        let asset = assets
            .iter()
            .find(|a| s(a, "id") == s(reference, "assetId") && !crate::store::asset_deleted(a))
            .ok_or("素材包资产身份缺失")?;
        if asset["version"] != reference["assetVersion"]
            || asset["variantId"] != reference["variantId"]
        {
            return Err("素材包语义形象版本已变化".into());
        }
        Ok(asset)
    };
    let references = snapshot["references"].as_array().ok_or("素材包引用缺失")?;
    let inline = snapshot["inlineReferences"]
        .as_array()
        .ok_or("素材包文字引用无效")?;
    for (order, reference) in references.iter().enumerate() {
        if reference["order"].as_u64() != Some(order as u64) {
            return Err("素材包物理引用顺序无效".into());
        }
        if !s(reference, "assetId").is_empty() {
            let physical = reference_assets(reference)?;
            if imported && s(reference, "bindingSource") == "unit-media-ref" {
                if !matches!(s(reference, "kind"), "audio" | "video")
                    || !["assetRefs", "audioAssetRefs", "mediaRefs"].iter().any(|field| {
                        frozen_unit[*field].as_array().into_iter().flatten().any(|r|
                            r["assetId"] == reference["assetId"]
                            && r["variantId"] == reference["variantId"]
                            && r["mediaId"] == reference["mediaId"])
                    })
                {
                    return Err("导入成稿音视频引用与冻结关联不同".into());
                }
            } else if physical["adoptedMediaId"] != reference["mediaId"] {
                return Err("素材包采用图已变化".into());
            }
        }
        for semantic in reference["semanticBindings"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut asset = reference_assets(semantic)?;
            let mut visited = HashSet::new();
            while asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("INLINE")
                && asset["adoptedMediaId"].is_null()
            {
                if !visited.insert(s(asset, "id")) {
                    return Err("素材包沿用来源循环".into());
                }
                let parent =
                    pipeline::resolve_asset_dependency(assets, asset)?.ok_or("沿用形象没有来源")?;
                asset = assets
                    .iter()
                    .find(|a| s(a, "id") == parent && !crate::store::asset_deleted(a))
                    .ok_or("沿用来源不存在")?;
            }
            if asset["id"] != reference["assetId"] || asset["variantId"] != reference["variantId"] {
                return Err("语义形象与物理素材来源不一致".into());
            }
        }
    }
    for reference in inline {
        let asset = reference_assets(reference)?;
        if s(reference, "kind") != "inline-text"
            || !matches!(s(asset, "kind"), "PROP" | "prop")
            || ["name", "description", "preparation", "sourceRefs"]
                .iter()
                .any(|key| asset[*key] != reference[*key])
            || asset.pointer("/preparation/mode").and_then(Value::as_str) != Some("INLINE")
            || !asset["adoptedMediaId"].is_null()
            || [
                s(asset, "sourceAssetId"),
                s(asset, "sourceVariantId"),
                s(&asset["preparation"], "fromKey"),
            ]
            .iter()
            .any(|s| !s.is_empty())
        {
            return Err("冻结文字素材身份或正文不一致".into());
        }
    }
    for reference in frozen_unit["assetRefs"]
        .as_array()
        .ok_or("单元资产引用缺失")?
    {
        let asset = assets
            .iter()
            .find(|a| s(a, "id") == s(reference, "assetId") && !crate::store::asset_deleted(a))
            .ok_or("单元资产身份不存在")?;
        if !s(reference, "variantId").is_empty() && reference["variantId"] != asset["variantId"] {
            return Err("单元形象身份与素材包不同".into());
        }
        if asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("AUDIO") {
            continue;
        }
        let matches = |value: &Value| {
            value["assetId"] == asset["id"] && value["variantId"] == asset["variantId"]
        };
        if !inline.iter().any(matches)
            && !references.iter().any(|r| {
                matches(r)
                    || r["semanticBindings"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(matches)
            })
        {
            return Err("素材包缺少单元的明确语义引用".into());
        }
    }
    if binding::compile_bound_prompt(frozen_unit, assets, references, inline)?
        != s(snapshot, "prompt")
    {
        return Err("原提示词按冻结引用编译后与素材包不一致".into());
    }
    let mut proof =
        json!({"version":"unit-source-prompt/v1","origin":"unit-version-and-reference-compilation","jobId":job_id,
        "unitId":frozen_unit["id"],"unitVersion":frozen_unit["version"],"sourceVersion":snapshot["sourceVersion"],
        "sourcePromptSha256":pipeline::digest(s(frozen_unit,"prompt")),"compiledPrompt":snapshot["prompt"],"compiledPromptVerified":true});
    if imported {
        proof["origin"] = json!("imported-unit-version-and-reference-compilation");
        proof["importSource"] = frozen_unit["importSource"].clone();
    }
    Ok(proof)
}
impl Engine {
    /// Normal installed desktop. Isolated probes and the one-shot diagnostic
    /// core keep their explicit offline constructor; no UI switch changes this.
    pub fn new_standard(home: PathBuf) -> Result<Arc<Self>> {
        crate::platform::ensure_isolated_home(&home)?;
        if !cfg!(any(target_os="macos",target_os="windows")) {
            return Self::new_offline_verification(home);
        }
        let saved_grant = home.join("native-cli-acceptance.json");
        if saved_grant.try_exists().map_err(|e| e.to_string())? {
            // Preserve an existing source-bound test reservation and its budget.
            return Self::with_native_acceptance(home, crate::native_acceptance::AcceptanceGrant::from_file(&saved_grant)?);
        }
        let mut engine=Self::build(home, Arc::new(crate::tasks::RealChannels), true, false)?;
        let inner=Arc::get_mut(&mut engine).ok_or("初始化所有权无效")?;
        inner.standard_native=true;
        inner.readiness_enabled=true;
        Ok(engine)
    }
    pub fn new(home: PathBuf) -> Result<Arc<Self>> {
        crate::platform::ensure_isolated_home(&home)?;
        let saved_grant = home.join("native-cli-acceptance.json");
        if saved_grant.try_exists().map_err(|e| e.to_string())? {
            return Self::with_native_acceptance(home, crate::native_acceptance::AcceptanceGrant::from_file(&saved_grant)?);
        }
        let mut engine=Self::build(home, Arc::new(crate::execution_policy::OfflineChannels), false, true)?;
        Arc::get_mut(&mut engine).ok_or("初始化所有权无效")?.readiness_enabled=cfg!(any(target_os="macos",target_os="windows"));
        Ok(engine)
    }
    /// Isolated startup acceptance must never read host accounts, even on a native OS.
    pub fn new_offline_verification(home:PathBuf)->Result<Arc<Self>> {
        crate::platform::ensure_isolated_home(&home)?;
        Self::build(home,Arc::new(crate::execution_policy::OfflineChannels),false,true)
    }
    /// Resource paths come from Tauri, never from an IPC request or project settings.
    pub fn with_native_resource_root(mut self:Arc<Self>,root:PathBuf)->Result<Arc<Self>> {
        if !root.is_absolute() {return Err("桌面资源路径必须是绝对路径".into());}
        Arc::get_mut(&mut self).ok_or("资源路径只能在启动前设置")?.readiness_resources=Some(root);
        Ok(self)
    }
    pub fn with_native_acceptance(home:PathBuf, grant:crate::native_acceptance::AcceptanceGrant)->Result<Arc<Self>> {
        let acceptance=crate::native_acceptance::NativeAcceptance::open(&home,grant)?;
        // Media channels and their automatic scheduler remain disabled. Only bounded
        // explicit CLI text operations can cross this separate acceptance boundary.
        let mut engine=Self::build(home,Arc::new(crate::execution_policy::OfflineChannels),true,true)?;
        Arc::get_mut(&mut engine).ok_or("验收引擎初始化所有权无效")?.native_acceptance=Some(Arc::new(acceptance));
        Arc::get_mut(&mut engine).ok_or("验收引擎初始化所有权无效")?.readiness_enabled=cfg!(any(target_os="macos",target_os="windows"));
        for project in engine.project_list()? {
            if let Ok(store)=engine.store(s(&project,"id")) {
                if let Err(error)=engine.native_acceptance.as_ref().unwrap().reconcile_unbound(&store) {
                    store.event("","operation_error",&json!({"code":"NATIVE_ACCEPTANCE_RECOVERY_HELD","message":error}))?;
                }
            }
        }
        Ok(engine)
    }
    fn reserve_native_workflow(&self,request:&Value,source:&Value,settings:&Value)->Result<Value> {
        if let Some(acceptance)=&self.native_acceptance {
            let reservation=acceptance.reserve(request,source,settings,crate::execution_policy::creative_runtime_ready())?;
            Ok(acceptance.receipt(&reservation))
        } else {Ok(Value::Null)}
    }
    fn bind_native_workflow(&self,store:&Store,receipt:&Value,job_id:&str)->Result<()> {
        if let Some(acceptance)=&self.native_acceptance {
            let reservation=serde_json::from_value::<crate::native_acceptance::Reservation>(receipt.clone()).map_err(|_|"验收工作流缺少原预约标记")?;
            acceptance.bind_workflow(&reservation,&store.job(job_id)?)?;
        }
        Ok(())
    }
    pub fn with_channels(home: PathBuf, channels: Arc<dyn Channels>) -> Result<Arc<Self>> {
        Self::build(home, channels, false, false)
    }
    fn build(home: PathBuf, channels: Arc<dyn Channels>, live_accounts: bool, offline_only: bool) -> Result<Arc<Self>> {
        Self::build_with_recovery(home, channels, live_accounts, offline_only, true)
    }
    fn build_with_recovery(home: PathBuf, channels: Arc<dyn Channels>, live_accounts: bool, offline_only: bool, recover_projects: bool) -> Result<Arc<Self>> {
        fs::create_dir_all(home.join("projects")).map_err(|e| e.to_string())?;
        let engine = Arc::new(Self {
            home,
            channels,
            active: Mutex::new(HashSet::new()),
            media_query_next: Mutex::new(HashMap::new()),
            image_wake: (Mutex::new(false), Condvar::new()),
            registry_mutex: Mutex::new(()),
            lifecycle_mutex: Mutex::new(()),
            dreamina_sessions: Arc::new(DreaminaSessions::default()),
            canvas_sessions: crate::canvas_oauth::CanvasSessions::default(),
            live_accounts,
            offline_only,
            standard_native: false,
            native_acceptance: None,
            readiness_enabled: false,
            readiness_resources: None,
            stop: AtomicBool::new(false),
        });
        if recover_projects { for p in engine.project_list()? {
            if project_archived(&p)
                || !Path::new(s(&p, "rootPath"))
                    .join("project.sqlite")
                    .is_file()
            {
                continue;
            }
            let store = registered_store(&p)?;
            store.recover_interrupted()?;
            store.ensure_current_images()?;
        } }
        Ok(engine)
    }
    pub fn registry(&self) -> Result<Value> {
        let p = self.home.join("projects.json");
        if !p.exists() {
            return Ok(json!([]));
        }
        serde_json::from_slice(&fs::read(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
    fn register(&self, store: &Store) -> Result<()> {
        let _guard = self.registry_mutex.lock().map_err(|e| e.to_string())?;
        let _file_lease = crate::project_library::lock(&self.home)?;
        let mut list = self.registry()?;
        let rows = list.as_array_mut().ok_or("项目索引损坏")?;
        let mut p = store.project()?;
        p["archivedAt"] = Value::Null;
        p["lifecycleVersion"] = json!(0);
        if rows.iter().any(|v| v["id"] == p["id"]) {
            return Err("此项目已经导入；为保持任务身份不能重复注册".into());
        }
        rows.push(p);
        atomic_json(&self.home.join("projects.json"), &list)
    }
    pub fn project_list(&self) -> Result<Vec<Value>> {
        let mut rows = self.registry()?.as_array().ok_or("项目索引损坏")?.clone();
        for p in &mut rows {
            p["lifecycleVersion"] = json!(lifecycle_version(p)?);
            if p.get("archivedAt").is_none() {
                p["archivedAt"] = Value::Null;
            }
            // A missing/archived registry entry must never initialize a fresh database.
            if project_archived(p) {
                continue;
            }
            if let Ok(store) = registered_store(p) {
                let c = store.conn()?;
                let count = |table: &str| {
                    // Retired revisions remain stored, but are absent from the active workspace.
                    let filter = if table == "units" {
                        " WHERE COALESCE(json_extract(data,'$.retired'),0)<>1"
                    } else {
                        ""
                    };
                    c.query_row(&format!("SELECT COUNT(*) FROM {table}{filter}"), [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap_or(0)
                };
                p["counts"] = json!({"assets":count("assets"),"media":count("media"),"units":count("units"),"jobs":count("jobs")});
                p["sourceVersion"] = store.source()?["version"].clone();
            }
        }
        Ok(rows)
    }
    pub fn store(&self, key: &str) -> Result<Store> {
        let p = self
            .registry()?
            .as_array()
            .ok_or("项目索引损坏")?
            .iter()
            .find(|p| s(p, "id") == key)
            .cloned()
            .ok_or("项目不存在")?;
        if project_archived(&p) {
            return Err("项目已归档，请先恢复项目".into());
        }
        registered_store(&p)
    }
    fn project_lifecycle(&self, request: &Value) -> Result<Value> {
        let key = s(request, "projectId");
        let expected = request["expectedVersion"]
            .as_u64()
            .ok_or("缺少项目当前版本，请刷新后再操作")?;
        let _registry = self.registry_mutex.lock().map_err(|e| e.to_string())?;
        let _file_lease = crate::project_library::lock(&self.home)?;
        let mut registry = self.registry()?;
        let rows = registry.as_array_mut().ok_or("项目索引损坏")?;
        let index = rows
            .iter()
            .position(|p| s(p, "id") == key)
            .ok_or("项目不存在")?;
        let current = &rows[index];
        let version = lifecycle_version(current)?;
        if version != expected {
            return Err("项目状态已变化，请刷新后再操作".into());
        }
        let prefix = format!("project:{key}:");
        if self
            .active
            .lock()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|k| k.starts_with(&prefix))
        {
            return Err("本项目仍有本地任务正在执行，请等待任务结束后再归档或删除".into());
        }
        let action = s(request, "action");
        let archived = project_archived(current);
        if action == "projects.delete" && !archived {
            return Err("请先归档项目，再删除项目记录".into());
        }
        if action == "projects.archive" && archived {
            return Err("项目已经归档，请刷新列表".into());
        }
        if action == "projects.restore" && !archived {
            return Err("项目当前未归档，请刷新列表".into());
        }
        if action == "projects.restore" {
            let store = registered_store(current)?;
            store.recover_interrupted()?;
            store.ensure_current_images()?;
        }
        let next = version.checked_add(1).ok_or("项目版本已超出范围")?;
        let mut result = current.clone();
        if action == "projects.archive" {
            if let Some(summary) = self.project_list()?.into_iter().find(|p| s(p, "id") == key) {
                result["counts"] = summary["counts"].clone();
                result["sourceVersion"] = summary["sourceVersion"].clone();
            }
        }
        result["projectId"] = json!(key);
        result["lifecycleVersion"] = json!(next);
        result["filesRetained"] = json!(true);
        if action == "projects.delete" {
            // Finish a possibly interrupted global-import receipt before
            // removing its registration. A later retry must respect deletion.
            if key.starts_with("lingmo-") {
                let receipt_path=Path::new(s(current,"rootPath")).join(".lingmo-create.json");
                if receipt_path.is_file() {
                    let mut receipt:Value=serde_json::from_slice(&fs::read(&receipt_path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                    if format!("lingmo-{}",s(&receipt,"identity"))==key {
                        receipt["state"]=json!("deleted");
                        atomic_json(&receipt_path,&receipt)?;
                    }
                }
            }
            result["deleted"] = json!(true);
            rows.remove(index);
        } else {
            result["archivedAt"] = if action == "projects.archive" {
                json!(now())
            } else {
                Value::Null
            };
            result["updatedAt"] = json!(now());
            let mut saved = result.clone();
            saved
                .as_object_mut()
                .ok_or("项目索引损坏")?
                .remove("projectId");
            saved
                .as_object_mut()
                .ok_or("项目索引损坏")?
                .remove("filesRetained");
            rows[index] = saved;
        }
        atomic_json(&self.home.join("projects.json"), &registry)?;
        Ok(result)
    }
    pub fn config(&self, store: &Store, provider: &str) -> Result<(ProviderConfig, String)> {
        self.config_with_settings(store, provider, &self.effective_settings(store)?)
    }
    fn config_with_settings(&self, store: &Store, provider: &str, settings: &Value) -> Result<(ProviderConfig, String)> {
        self.config_for_identity(s(&store.project()?,"id"),provider,settings)
    }
    fn config_for_identity(&self, pid:&str, provider: &str, settings: &Value) -> Result<(ProviderConfig, String)> {
        if provider=="chrome" && settings["chromeTransport"]=="opencli" {return crate::providers::opencli::config(&self.home,settings).map_err(|e|e.message);}
        if provider == "dreamina" && self.live_accounts {
            let config = self.dreamina_config_with_settings(settings)?;
            let account = self.dreamina_sessions.snapshot(&config)?;
            let identity = s(&account, "accountKey");
            return Ok((
                config,
                if account["ready"] == true
                    && account["identityVerified"] == true
                    && !identity.is_empty()
                {
                    identity.to_owned()
                } else {
                    "dreamina:official:unverified".into()
                },
            ));
        }
        let mut env = format!(
            "DIRECTOR_NEW_{}_{}",
            pid.replace('-', "_"),
            provider.to_uppercase()
        );
        if settings[crate::app_settings::SCOPE].is_object() {
            if let Ok(reference)=crate::app_settings::reference(settings,pid,provider) {
                env=format!("DIRECTOR_GLOBAL_{}_{}",provider.to_uppercase(),&pipeline::digest(&reference)[..24]);
            }
        }
        if self.live_accounts && ["bh", "chrome"].contains(&provider) {
            let bound_secret = crate::app_settings::credential(settings,pid,provider);
            if settings[crate::app_settings::SCOPE]["credentialHashes"].is_object() && bound_secret.is_err(){return Err(bound_secret.unwrap_err());}
            let secret = bound_secret.ok();
            if let Some(secret) = secret {
                std::env::set_var(&env, secret);
            } else {
                std::env::remove_var(&env);
            }
        }
        let cli = if s(&settings, "dreaminaPath").is_empty() {
            bundled_dreamina().map(|p| json!(p)).unwrap_or(Value::Null)
        } else {
            settings["dreaminaPath"].clone()
        };
        let cfg:ProviderConfig=serde_json::from_value(json!({"kind":provider,"endpoint":match provider{"bh"=>settings.get("bhEndpoint"),"chrome"=>settings.get("chromeEndpoint"),_=>None},"credentialEnv":if provider=="bh"{Some(env.clone())}else{None},"bridgeTokenEnv":if provider=="chrome"{Some(env.clone())}else{None},"chromeBinding":if provider=="chrome"{crate::chrome_setup::browser_binding(settings)?}else{None},"cliPath":cli,"timeoutSeconds":settings.get("providerTimeoutSeconds").cloned().unwrap_or(json!(120)),"adaptedModels":settings.get("adaptedModels").cloned().unwrap_or(json!([]))})).map_err(|e|e.to_string())?;
        let account_key = format!("{provider}AccountId");
        let explicit = s(&settings, &account_key);
        let endpoint = cfg.endpoint.as_deref().unwrap_or("official-cli");
        let account = if !explicit.is_empty() {
            format!("{provider}:{endpoint}:account:{explicit}")
        } else if let Ok(secret) = std::env::var(&env) {
            use sha2::{Digest, Sha256};
            format!(
                "{provider}:{endpoint}:credential:{:x}",
                Sha256::digest(secret.as_bytes())
            )
        } else {
            format!("{provider}:{endpoint}:unidentified")
        };
        Ok((cfg, account))
    }
    fn generation_config(&self, store: &Store, summary: &Value) -> Result<(ProviderConfig,String)> {
        // pending_jobs deliberately returns a small projection, never mistake it
        // for the original submission snapshot.
        let full = store.job(s(summary,"id"))?;
        let job=&full;
        let provider=s(job,"provider");
        if !job["snapshot"]["frozenSettings"].is_object() {return self.config_with_settings(store,provider,&store.settings()?);}
        let mut settings=job["snapshot"]["frozenSettings"].clone();
        // Channel parameters are frozen, but current identity must still be
        // verified. Never label changed credentials as the old saved account.
        if (!self.live_accounts || provider!="dreamina") && !settings[crate::app_settings::SCOPE].is_object() {
            let account_key=format!("{provider}AccountId");
            settings[&account_key]=store.settings()?[&account_key].clone();
        }
        if let Some(path)=job["snapshot"]["providerConfig"]["cliPath"].as_str() {settings["dreaminaPath"]=json!(path);}
        let (config,account)=self.config_with_settings(store,provider,&settings)?;
        let mut config: ProviderConfig = if job["snapshot"]["providerConfig"].is_object() {
            serde_json::from_value(job["snapshot"]["providerConfig"].clone()).map_err(|e|format!("冻结图片渠道配置无效：{e}"))?
        } else {config};
        if provider=="chrome" && s(job,"kind")=="image" && config.chrome_binding.is_none() {
            config.chrome_binding=self.legacy_chrome_execution_binding(store,job,&settings,&config,&account)?;
        }
        Ok((config,account))
    }
    /// Only untouched legacy queue entries may acquire an execution identity.
    /// The request and its old channel config remain byte-for-byte frozen.
    fn legacy_chrome_execution_binding(&self,store:&Store,job:&Value,frozen:&Value,config:&ProviderConfig,account:&str)->Result<Option<crate::chrome_setup::BrowserBinding>> {
        use rusqlite::OptionalExtension;
        let key=format!("legacy_chrome_execution_binding:{}",s(job,"id"));
        // The existing parent-image resolver fills only references after the
        // direct parent lands. All immutable request/channel identity stays bound.
        let mut identity=job["snapshot"].clone();
        if let Some(fields)=identity.as_object_mut(){fields.remove("references");}
        let snapshot_hash=pipeline::digest(&identity.to_string());
        let config_hash=pipeline::digest(&serde_json::to_string(config).map_err(|e|e.to_string())?);
        let decode=|raw:String|->Result<Option<crate::chrome_setup::BrowserBinding>> {
            let saved:Value=serde_json::from_str(&raw).map_err(|_|"旧Chrome队列执行绑定记录损坏，未重新绑定。")?;
            if saved["version"]!="legacy-chrome-execution-binding/v1" || saved["snapshotHash"]!=snapshot_hash
                || saved["configHash"]!=config_hash || saved["accountId"]!=account || saved["accountId"]!=job["accountId"] {
                return Err("旧Chrome队列的原请求或渠道身份已变化，未重新绑定。".into());
            }
            let binding:crate::chrome_setup::BrowserBinding=serde_json::from_value(saved["binding"].clone()).map_err(|_|"旧Chrome队列执行绑定无效。")?;
            crate::chrome_setup::browser_binding(&crate::chrome_setup::binding_settings(config.endpoint.as_deref().unwrap_or(""),Some(&binding)))
        };
        let c=store.conn()?;
        if let Some(raw)=c.query_row("SELECT value FROM metadata WHERE key=?1",[&key],|r|r.get::<_,String>(0)).optional().map_err(|e|e.to_string())? {return decode(raw);}
        if !["queued","waiting_dependency","failed","auth_required"].contains(&s(job,"status"))
            || !s(job,"providerTaskId").is_empty() || !job["prepared"].is_null()
            || !(job["receipt"].is_null() || job["receipt"]==json!({}) || job["receipt"]=="")
            || job["submissionUnknown"]==true || job["result"]["dispatchStarted"]==true || job["result"]["submissionUnknown"]==true {
            return Ok(None);
        }
        let prior_dispatch:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND (type LIKE '%dispatch%started%' OR json_extract(data,'$.state') IN ('submitting','submitted','submission_unknown','processing','succeeded','downloaded') OR json_extract(data,'$.from') IN ('submitting','submitted','submission_unknown','processing','succeeded','downloaded')))",[s(job,"id")],|r|r.get(0)).map_err(|e|e.to_string())?;
        if prior_dispatch {return Ok(None);}
        let current=self.effective_settings(store)?;
        let Some(binding)=crate::chrome_setup::browser_binding(&current)? else {return Ok(None);};
        let pid=s(&store.project()?,"id").to_owned();
        let (frozen_config,_)=self.config_with_settings(store,"chrome",frozen)?;
        let (current_config,current_account)=self.config_with_settings(store,"chrome",&current)?;
        if config.endpoint!=current_config.endpoint || config.endpoint.as_deref()!=Some(binding.endpoint.as_str())
            || config.bridge_token_env!=frozen_config.bridge_token_env || account!=current_account || job["accountId"]!=account
            || crate::app_settings::reference(frozen,&pid,"chrome")?!=crate::app_settings::reference(&current,&pid,"chrome")? {
            return Err("旧Chrome队列与当前全局渠道的地址或原凭据身份不一致，未迁移或提交。".into());
        }
        let saved=json!({"version":"legacy-chrome-execution-binding/v1","snapshotHash":snapshot_hash,"configHash":config_hash,
            "accountId":account,"binding":binding,"fixedAt":now()});
        // CAS against the complete original request and empty remote/preparation
        // evidence. A racing submit or a previous migration always wins.
        c.execute("INSERT OR IGNORE INTO metadata(key,value) SELECT ?1,?2 FROM jobs j WHERE j.id=?3 AND j.snapshot=?4 AND j.account_id=?5 AND j.provider='chrome' AND j.kind='image' AND j.state IN ('queued','waiting_dependency','failed','auth_required') AND COALESCE(j.task_id,'')='' AND j.prepared IS NULL AND (j.receipt IS NULL OR j.receipt IN ('','null','{}')) AND NOT EXISTS(SELECT 1 FROM events e WHERE e.job_id=j.id AND (e.type LIKE '%dispatch%started%' OR json_extract(e.data,'$.state') IN ('submitting','submitted','submission_unknown','processing','succeeded','downloaded') OR json_extract(e.data,'$.from') IN ('submitting','submitted','submission_unknown','processing','succeeded','downloaded')))",
            rusqlite::params![key,saved.to_string(),s(job,"id"),job["snapshot"].to_string(),account]).map_err(|e|e.to_string())?;
        let raw:Option<String>=c.query_row("SELECT value FROM metadata WHERE key=?1",[&key],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        raw.map(decode).transpose().map(Option::flatten)
    }
    fn dreamina_config(&self, store: &Store) -> Result<ProviderConfig> {
        self.dreamina_config_with_settings(&self.effective_settings(store)?)
    }
    fn dreamina_config_with_settings(&self, settings: &Value) -> Result<ProviderConfig> {
        let cli = if s(&settings, "dreaminaPath").is_empty() {
            bundled_dreamina().map(|p| json!(p)).unwrap_or(Value::Null)
        } else {
            settings["dreaminaPath"].clone()
        };
        serde_json::from_value(json!({"kind":"dreamina","cliPath":cli,"timeoutSeconds":settings.get("providerTimeoutSeconds").cloned().unwrap_or(json!(120))})).map_err(|e| e.to_string())
    }
    fn require_account(&self, store: &Store, provider: &str) -> Result<()> {
        if provider == "dreamina" && self.live_accounts {
            let account = self
                .dreamina_sessions
                .snapshot(&self.dreamina_config(store)?)?;
            if account["ready"] != true
                || account["identityVerified"] != true
                || s(&account, "accountKey").is_empty()
            {
                return Err("请在渠道设置中连接即梦官方 CLI，或刷新现有登录状态。".into());
            }
            return Ok(());
        }
        let settings=self.effective_settings(store)?;
        if provider=="chrome" && settings["chromeTransport"]=="opencli" {return Ok(());}
        if settings[crate::app_settings::SCOPE].is_object() && ["bh","chrome"].contains(&provider) {
            crate::app_settings::credential(&settings,s(&store.project()?,"id"),provider).map(|_|())
        } else {require_account(store, provider)}
    }
    fn checked_account(&self, config: &ProviderConfig, fallback: &str) -> ProviderResult<String> {
        if self.live_accounts && config.kind == "dreamina" {
            self.dreamina_sessions.execution_identity(config)
        } else {
            Ok(fallback.to_owned())
        }
    }
    fn pause_for_auth(store: &Store, key: &str, message: &str) -> Result<()> {
        let job = store.job(key)?;
        if message == "客户端已停止本地 CLI 查询。"
            && !s(&job, "providerTaskId").is_empty()
            && ["image", "video"].contains(&s(&job, "kind"))
        {
            return store.update_job(
                key,
                "submitted",
                &json!({"error":"本地查询已中断；重新打开后继续原任务。"}),
            );
        }
        // Authentication failure must never release an uncertain submit for regeneration.
        let state = if s(&job, "status") == "submission_unknown" {
            "submission_unknown"
        } else {
            "auth_required"
        };
        store.update_job(key, state, &json!({"error":message}))
    }
    fn account_preflight_failed(store: &Store, key: &str, error: &ProviderError) -> Result<()> {
        let job = store.job(key)?;
        let has_task = !s(&job, "providerTaskId").is_empty();
        store.event(
            key,
            "account_preflight_failed",
            &json!({
                "code":error.code,"authRequired":error.auth_required,
                "message":error.message,"queryOnly":has_task
            }),
        )?;
        if s(&job, "status") == "downloaded"
            && job["resultMediaIds"]
                .as_array()
                .is_some_and(|ids| !ids.is_empty())
        {
            // A failed inspection cannot invalidate an already imported result.
            return Ok(());
        }
        if s(&job, "status") == "submission_unknown" {
            // No identity failure may release an uncertain submit for generation.
            return store.update_job(key, "submission_unknown", &json!({"error":error.message}));
        }
        if error.auth_required {
            return Self::pause_for_auth(store, key, &error.message);
        }
        if error.code == "CLIENT_CLOSING" {
            return store.update_job(
                key,
                if has_task {
                    "submitted"
                } else {
                    s(&job, "status")
                },
                &json!({"error":"本地账户查询已中断；重新打开后继续原任务。"}),
            );
        }
        if has_task {
            // This bounded retry only queries the original durable ID; it never submits.
            return store.retry_job(key, "query", &error.message);
        }
        store.update_job(key, "failed", &json!({
            "error":format!("{} 账户预检未完成，尚未调用生成；请检查连接后明确重试。",error.message)
        }))
    }
    /// Authentication recovery only re-enables queries for the original persisted IDs.
    /// A pre-submission failure requires a new explicit generation operation.
    pub fn resume_authenticated_dreamina(&self, store: &Store, account: &Value) -> Result<()> {
        let _lifecycle = self.lifecycle_mutex.lock().map_err(|e| e.to_string())?;
        let pid = s(&store.project()?, "id").to_owned();
        if self
            .registry()?
            .as_array()
            .ok_or("项目索引损坏")?
            .iter()
            .any(|p| s(p, "id") == pid && project_archived(p))
        {
            return Ok(());
        }
        self.resume_authenticated_dreamina_locked(store, account)
    }
    fn resume_authenticated_dreamina_locked(&self, store: &Store, account: &Value) -> Result<()> {
        if account["ready"] != true
            || account["identityVerified"] != true
            || s(account, "accountKey").is_empty()
        {
            return Ok(());
        }
        let identity = s(account, "accountKey");
        let mut conn = store.conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let resumed = tx.execute("INSERT INTO events(job_id,type,data,created_at) SELECT id,'authentication_recovered','{\"queryOnly\":true,\"state\":\"submitted\"}',?1 FROM jobs WHERE provider='dreamina' AND state='auth_required' AND account_id=?2 AND task_id IS NOT NULL AND task_id<>''",rusqlite::params![now(),identity]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE jobs SET state='submitted',error=NULL,next_attempt_at=0,updated_at=?1 WHERE provider='dreamina' AND state='auth_required' AND account_id=?2 AND task_id IS NOT NULL AND task_id<>''",rusqlite::params![now(),identity]).map_err(|e|e.to_string())?;
        let expired = tx.execute("UPDATE jobs SET state='stale',error='登录已恢复；此项没有生成任务回执，请明确重新发起生成',updated_at=?1 WHERE provider='dreamina' AND state='auth_required' AND account_id=?2 AND (task_id IS NULL OR task_id='')",rusqlite::params![now(),identity]).map_err(|e|e.to_string())?;
        if resumed + expired > 0 {
            tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn workspace(&self, store: &Store, episode: Option<&str>) -> Result<Value> {
        let mut assets = store.values("assets")?;
        let mut deleted_assets = store.deleted_assets()?;
        let media = store.values("media")?;
        let mut units = store.unit_summaries()?;
        let jobs = store.job_summaries()?;
        let c = store.conn()?;
        let mut aq = c
            .prepare("SELECT asset_id,variant_id,media_id FROM adoptions")
            .map_err(|e| e.to_string())?;
        let adopted = aq
            .query_map([], |r| {
                Ok((
                    (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(|e| e.to_string())?;
        let mut mq = c
            .prepare("SELECT asset_id,variant_id,media_id FROM media_assets ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        let mut candidates: HashMap<(String, String), Vec<String>> = HashMap::new();
        for row in mq
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
        {
            let (a, v, m) = row.map_err(|e| e.to_string())?;
            candidates.entry((a, v)).or_default().push(m);
        }
        let mut vq=c.prepare("SELECT a.unit_id,a.media_id,COALESCE((SELECT json_extract(value,'$.unitVersion') FROM metadata WHERE key='video_adoption:'||a.unit_id AND json_extract(value,'$.mediaId')=a.media_id),(SELECT json_extract(j.snapshot,'$.unitVersion') FROM jobs j JOIN job_media jm ON jm.job_id=j.id WHERE jm.media_id=a.media_id AND j.entity_id=a.unit_id AND j.kind='video' AND j.created_at<=a.created_at ORDER BY j.created_at DESC LIMIT 1)) FROM video_adoptions a").map_err(|e|e.to_string())?;
        let video_adoptions = vq
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?),
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(|e| e.to_string())?;
        for a in assets.iter_mut().chain(deleted_assets.iter_mut()) {
            let key = (s(a, "id").to_string(), s(a, "variantId").to_string());
            a["adoptedMediaId"] = json!(adopted.get(&key));
            a["candidates"] = json!(candidates.get(&key).cloned().unwrap_or_default());
        }
        if let Some(ep) = episode {
            units.retain(|u| s(u, "episodeId") == ep)
        }
        let settings = store.settings()?;
        let source_version: i64 = c
            .query_row("SELECT COALESCE(MAX(version),0) FROM sources", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        // One metadata-only join, not an original-media read/hash for every unit.
        // Historical text is selected by the ready job's frozen unit version.
        let ready_prompt_evidence = ready_prompt_evidence(&c)?;
        for u in &mut units {
            if let Some(j) = jobs
                .iter()
                .find(|j| s(j, "entityId") == s(u, "id") && s(j, "kind") == "video")
            {
                u["status"] = j["status"].clone();
                u["problems"] = j["snapshot"]["problems"].clone();
                if s(j, "status") == "ready" {
                    u["preparedPackage"] = json!({"provider":j["provider"],"model":j["snapshot"]["model"],"unitVersion":j["snapshot"]["unitVersion"],"references":j["snapshot"]["references"],"problems":j["snapshot"]["problems"],"estimatedCost":j["prepared"]["estimatedCost"],"jobId":j["id"],"ratio":j["snapshot"]["ratio"],"resolution":j["snapshot"]["resolution"],"duration":j["snapshot"]["duration"],"prompt":j["snapshot"]["prompt"]});
                    if let Some((snapshot, frozen_unit)) = ready_prompt_evidence.get(s(j, "id")) {
                        u["preparedPackage"]["inlineReferences"] =
                            snapshot["inlineReferences"].clone();
                        match prepared_source_prompt_provenance(
                            s(j, "id"),
                            u,
                            frozen_unit,
                            snapshot,
                            &assets,
                            source_version,
                            &settings,
                        ) {
                            Ok(proof) => {
                                u["preparedPackage"]["sourcePrompt"] =
                                    frozen_unit["prompt"].clone();
                                u["preparedPackage"]["sourcePromptProvenance"] = proof;
                            }
                            Err(_) => {
                                u["preparedPackage"]["sourcePromptProvenance"] = json!({"version":"unit-source-prompt/v1","jobId":j["id"],"compiledPromptVerified":false});
                            }
                        }
                    } else {
                        u["preparedPackage"]["sourcePromptProvenance"] = json!({"version":"unit-source-prompt/v1","jobId":j["id"],"compiledPromptVerified":false});
                    }
                }
                u["resultMediaIds"] = j["resultMediaIds"].clone();
            }
            if let Some((mid, version)) = video_adoptions.get(s(u, "id")) {
                u["adoptedMediaId"] = json!(mid);
                u["adoptedVideoVersion"] = json!(version);
            }
            if let Some(obj) = u.as_object_mut() {
                for key in [
                    "shots",
                    "plan",
                    "emotionPlan",
                    "performance",
                    "ruleSnapshots",
                ] {
                    obj.remove(key);
                }
            }
        }
        let mut summaries = jobs.clone();
        let archived_task_records = store.archived_task_record_ids()?;
        let mut auto_query = c.prepare("SELECT id,json_extract(snapshot,'$.autoSubmit') FROM jobs WHERE kind IN ('image','video')").map_err(|e|e.to_string())?;
        let auto_submit = auto_query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<bool>>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(|e| e.to_string())?;
        for j in &mut summaries {
            *j = workspace_job_summary(j.clone(),auto_submit.get(s(j,"id")).copied().flatten());
            j["taskRecordArchived"] = json!(archived_task_records.contains(s(j,"id")));
        }
        let mut providers = Vec::new();
        let mut q = c
            .prepare("SELECT data FROM catalogs")
            .map_err(|e| e.to_string())?;
        for v in q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
        {
            providers.push(crate::store::parse(v.map_err(|e| e.to_string())?)?);
        }
        let source = store.source()?;
        let mut diagnostics = source.get("diagnostics").cloned().unwrap_or(json!([]));
        let last_import: Option<String> = store
            .conn()?
            .query_row(
                "SELECT value FROM metadata WHERE key='last_import_report'",
                [],
                |r| r.get(0),
            )
            .ok();
        let report = last_import
            .and_then(|v| serde_json::from_str::<Value>(&v).ok())
            .unwrap_or(Value::Null);
        if let Some(items) = report["diagnostics"].as_array() {
            if let Some(all) = diagnostics.as_array_mut() {
                all.extend(items.iter().cloned());
            }
        }
        let mut dq=c.prepare("SELECT type,data,created_at FROM events WHERE type IN ('batch_blocked','operation_error','local_error','catalog_refresh_failed') ORDER BY id DESC LIMIT 30").map_err(|e|e.to_string())?;
        let notices = dq
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for notice in notices {
            let (code, data, time) = notice.map_err(|e| e.to_string())?;
            let data: Value = serde_json::from_str(&data).unwrap_or(json!(data));
            let message = if let Some(items) = data.as_array() {
                items
                    .iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect::<Vec<_>>()
                    .join("；")
            } else {
                s(&data, "message").to_string()
            };
            if let Some(items) = diagnostics.as_array_mut() {
                items.push(json!({"code":code,"message":message,"createdAt":time,"details":data}));
            }
        }
        let rules: Vec<_> = pipeline::rule_snapshots()
            .into_iter()
            .map(|r| json!({"id":r.id,"version":r.version,"sha256":r.sha256}))
            .collect();
        let mut display_settings=self.effective_settings(store)?;
        if display_settings["imageAdoptionPolicy"]=="delegated-review"&&store.visual_review_delegation()?["enabled"]!=true{
            display_settings["imageAdoptionPolicy"]=json!("manual");
        }
        // Match the execution gate without exposing credential references or
        // hashes. Global Chrome/BH identities need no user-entered account label.
        let credential_status: serde_json::Map<String, Value> = ["bh", "chrome"]
            .into_iter()
            .map(|provider| (provider.into(), json!({"configured": self.require_account(store, provider).is_ok()})))
            .collect();
        if let Some(object)=display_settings.as_object_mut(){object.remove(crate::app_settings::SCOPE);}
        if self.standard_native {if let Some(global)=crate::app_settings::load(&self.home)? {for (_,entry) in global["catalogs"].as_object().into_iter().flatten(){let cap=entry["capabilities"].clone();providers.retain(|p|p["provider"]!=cap["provider"]);providers.push(cap);}}}
        Ok(
            json!({"project":store.project()?,"source":source,"foundation":store.foundation()?,"assets":assets,"deletedAssets":deleted_assets,"media":media,"units":units,"jobs":summaries,"workflowProgress":store.workflow_progress()?,"settings":display_settings,"credentialStatus":credential_status,"providers":providers,"dreaminaAccount":self.dreamina_sessions.snapshot(&self.dreamina_config(store)?)?,"rules":rules,"diagnostics":diagnostics,"lastImportReport":report,"lingmoDocuments":crate::lingmo_import::documents(store)?,"imageProductionScope":store.image_production_scope()?,"imageReviewCapability":self.visual_review_capability(store,&display_settings)?,"imageReviewDelegation":store.visual_review_delegation()?}),
        )
    }
    pub fn cancel_canvas_oauth(&self) { self.canvas_sessions.shutdown(); }
    pub fn canvas_oauth_stopped(&self) -> bool { self.canvas_sessions.stopped() }
    pub fn dispatch(self: &Arc<Self>, request: Value) -> Result<Value> {
        if request.get("backgroundChromeBinding").is_some() {
            return Err("后台队列绑定只能由专用执行入口创建".into());
        }
        let action = s(&request, "action");
        if action == "getExecutionPolicy" {
            let mut policy=match &self.native_acceptance {Some(a)=>crate::execution_policy::native_report(a),None if self.standard_native=>crate::execution_policy::standard_report(),None=>crate::execution_policy::report()};
            policy["readiness"]=crate::readiness::capability(self.readiness_enabled);
            policy["canvasOAuth"]=json!({"version":"canvas-oauth/v1","enabled":self.readiness_enabled});
            return Ok(policy);
        }
        if crate::canvas_oauth::ACTIONS.contains(&action) {
            if !self.readiness_enabled { return Err("CANVAS_AUTH_NATIVE_ONLY: 当前环境未开放本机即梦授权。".into()); }
            let _lifecycle=self.lifecycle_mutex.lock().map_err(|e|e.to_string())?;
            let project=s(&request,"projectId");
            let store=self.store(project)?;
            let (_,settings)=crate::readiness::request_settings(&json!({"provider":"dreamina_canvas","settings":request["settings"]}),&store.settings()?)?;
            let path=crate::canvas_oauth::resolve(&settings,self.readiness_resources.as_deref(),&self.home)?;
            return match action {
                "canvas.login" => {
                    if request.get("force").is_some_and(|v|!v.is_boolean()) {return Err("重新授权参数无效。".into());}
                    let force=request["force"]==true;
                    if force && request["confirmedForce"]!=true {return Err("重新授权会清除本配置登录态，请先确认。".into());}
                    store.save_settings(&json!({"dreaminaCanvasPath":path}))?;
                    atomic_json(&self.home.join("canvas-cli-path.json"),&json!({"path":path}))?;
                    crate::executable_paths::remember(&self.home,&json!({"dreaminaCanvasPath":path}))?;
                    self.canvas_sessions.begin(project,path,force)
                },
                "canvas.logout" => {
                    if request["confirmedLogout"]!=true {return Err("退出会清除当前 Canvas 配置的登录状态，请先确认。".into());}
                    self.canvas_sessions.logout(project,path)
                },
                "canvas.loginStatus" => self.canvas_sessions.status(project,&path),
                "canvas.cancelLogin" => self.canvas_sessions.cancel(project,&path),
                _ => {
                    let state=self.canvas_sessions.status(project,&path)?;
                    if state["state"]!="authorizing" {return Err("当前没有等待确认的即梦授权。".into());}
                    crate::canvas_oauth::open_authorization(s(&state,"verificationUrl"))?;
                    Ok(state)
                }
            };
        }
        if action=="readiness.check" && !self.readiness_enabled {
            return Err("READINESS_NATIVE_ONLY: 只读连接检查仅由本机桌面显式触发；云端和隔离启动探针不读取宿主账户".into());
        }
        if action == "runtime.prepare" && (!self.standard_native || !self.readiness_enabled || !self.live_accounts) {
            return Err("RUNTIME_SETUP_NATIVE_ONLY: 自动配置仅由标准桌面端连接操作触发；离线验收不会下载或安装程序。".into());
        }
        if self.native_acceptance.is_some() { crate::execution_policy::check_native_action(&request)?; }
        else if self.standard_native { crate::execution_policy::check_standard(&request)?; }
        else if self.offline_only { crate::execution_policy::check(&request)?; }
        if action.starts_with("settings.global.") || request["scope"]=="global" {
            return self.dispatch_global_settings(&request);
        }
        // File pickers and library reads must not hold a project lifecycle reservation.
        match action {
            "projects.list" => return Ok(json!(self.project_list()?)),
            "lingmo.preview_file" => return crate::lingmo_import::preview_file(&request),
            "files.choose" => return choose_files(&request),
            "legacy.inspect" => {
                return crate::importer::inspect_legacy(Path::new(s(&request, "path")))
            }
            _ => {}
        }
        let lifecycle = self.lifecycle_mutex.lock().map_err(|e| e.to_string())?;
        match action {
            "lingmo.create_project" => return self.create_lingmo_project(&request),
            "projects.archive" | "projects.restore" | "projects.delete" => {
                return self.project_lifecycle(&request)
            }
            "projects.create" => {
                let key = id();
                let root = if s(&request, "rootPath").is_empty() {
                    self.home.join("projects").join(&key)
                } else {
                    PathBuf::from(s(&request, "rootPath")).join(&key)
                };
                let store = Store::open(root)?;
                store.create(s(&request, "title"), &key)?;
                crate::executable_paths::apply(&self.home, &store)?;
                self.register(&store)?;
                return store.project();
            }
            "project.import" => {
                let target = self.home.join("projects").join(id());
                let store = portable::import(Path::new(s(&request, "path")), &target)?;
                crate::executable_paths::apply(&self.home, &store)?;
                self.register(&store)?;
                store.ensure_current_images()?;
                return self.workspace(&store, None);
            }
            _ => {}
        }
        let store = self.store(s(&request, "projectId"))?;
        // The short lock makes validation + reservation atomic with archive/delete. The
        // marker outlives synchronous work, and spawned workers acquire their own marker.
        let _operation = self.acquire_keys(project_worker_keys(&store, vec![])?)?;
        drop(lifecycle);
        let request = if let Some(acceptance)=&self.native_acceptance {
            let scoped=request.clone();
            if ["foundation.run","images.design","storyboard.run"].contains(&action) {
                acceptance.grant.check_start(&request,&store.source()?,&store.settings()?,crate::execution_policy::creative_runtime_ready())?;
            } else if ["workflows.resume","storyboard.decision.resolve"].contains(&action) {
                let job=store.job(s(&request,"workflowId"))?;
                acceptance.check_resume(&request,&job,&store.source()?)?;
            } else if action=="source.import" {
                if let Some(text)=request["text"].as_str() {
                    if pipeline::digest(text)!=acceptance.grant.locked_source_sha256 { return Err("验收导入文本不是获准原文，未覆盖项目".into()); }
                } else {
                    acceptance.grant.verify_document(&fs::read(s(&request,"path")).map_err(|e|e.to_string())?)?;
                }
            }
            scoped
        } else {request.clone()};
        match action {
            "workspace.get" => {
                store.ensure_current_images()?;
                return self.workspace(&store, request.get("episodeId").and_then(Value::as_str));
            }
            "lingmo.preview" => return crate::lingmo_import::preview(&store, &request),
            "lingmo.document" => return crate::lingmo_import::document(&store, s(&request,"documentId")),
            "lingmo.scene" => return crate::lingmo_import::scene(&store, s(&request,"documentId"), s(&request,"sceneId")),
            "lingmo.original" => return crate::lingmo_import::original(&store, s(&request,"documentId"), request["unitId"].as_str()),
            "lingmo.media.preview" => return crate::lingmo_media::preview(&store, &request),
            "lingmo.media.latest" => return crate::lingmo_media::latest(&store, &request),
            "lingmo.media.commit" => {
                let report=crate::lingmo_media::commit(&store,&request)?;
                let mut workspace=self.workspace(&store,None)?;
                workspace["lingmoMediaImportReport"]=report["report"].clone();
                return Ok(workspace);
            }
            "lingmo.import" => {
                let report=crate::lingmo_import::import(&store,&request)?;
                let mut workspace=self.workspace(&store,None)?;
                workspace["lingmoImportReport"]=report;
                return Ok(workspace);
            }
            "units.get" => return store.get("units", s(&request, "unitId")),
            "units.versions.list" => return store.unit_versions(s(&request, "unitId")),
            "units.version.restore" => {
                store.restore_unit_version(
                    s(&request, "unitId"),
                    request["version"].as_i64().ok_or("请选择历史版本")?,
                    request["expectedVersion"]
                        .as_i64()
                        .ok_or("缺少当前版本，请刷新")?,
                )?;
            }
            "production.direction.save" => {
                store.save_production_direction(
                    s(&request, "direction"),
                    request["expectedSourceVersion"]
                        .as_i64()
                        .ok_or("请先导入原文")?,
                    request["expectedFoundationVersion"].as_i64(),
                )?;
            }
            "foundation.choices.list" => return store.foundation_choices(),
            "foundation.choice.confirm" => return store.confirm_foundation_choice(&request),
            "foundation.choices.apply" => {
                self.spawn_foundation_choices(store.clone(), &request)?;
            }
            "workflows.resume" | "storyboard.decision.resolve" => {
                self.resume_production_checked(store.clone(), &request,
                    crate::execution_policy::creative_runtime_ready())?;
            }
            "units.regenerate" => {
                let unit = store.get("units", s(&request, "unitId"))?;
                if request.get("expectedVersion").is_some()
                    && request["expectedVersion"] != unit["version"]
                {
                    return Err("单元已更新，请刷新后重做".into());
                }
                if s(&request, "instruction").trim().is_empty() {
                    return Err("请填写此单元的修改要求".into());
                }
                if unit["timingProfile"]=="lingmo-v7-nanoseconds/v1" {
                    return Err("新版分镜的整集精修入口尚未开放；可编辑文字，原采用稿保留，不调用旧版规则重写".into());
                }
                let run = json!({"action":"storyboard.run","projectId":request["projectId"],"episodeIds":[unit["episodeId"]],"unitIds":[unit["id"]],"revisionInstruction":request["instruction"]});
                self.spawn_production(store.clone(), run)?;
            }
            "images.previewPrompt" => {
                let asset = store.active_asset(s(&request,"assetId"))?;
                if crate::asset_graph::modern(&asset) { return pipeline::asset_workflow::compile_preview(&asset); }
                let settings = store.settings()?;
                let presentation = pipeline::asset_design_contract::image_presentation(&settings,&asset);
                return Ok(json!({"prompt":pipeline::asset_design_contract::compile_image_prompt(s(&asset,"prompt"),pipeline::asset_design_contract::project_visual_style(&settings),&presentation),"promptCompilerVersion":pipeline::asset_design_contract::IMAGE_PROMPT_COMPILER_VERSION,"promptBody":asset["prompt"],"imagePresentation":presentation,"assetVersion":asset["version"]}));
            }
            "images.exportPrompts" => return store.export_image_prompts(&request),
            "images.exportCurrent" => return store.export_current_images(&request),
            "storyboard.exportDelivery" => return store.export_series_delivery(&request),
            "images.registerReference" => return store.register_image_reference(&request),
            "assets.identityLayout.save" => {store.save_identity_layout(&request)?;},
            "assets.get" => return store.get("assets", s(&request, "assetId")),
            "assets.usage" => return store.asset_usage(s(&request, "assetId")),
            "jobs.detail" => {
                let mut j = store.job(s(&request, "jobId"))?;
                j["taskRecordArchived"] = json!(store.archived_task_record_ids()?.contains(s(&request,"jobId")));
                let c = store.conn()?;
                let mut q=c.prepare("SELECT type,data,created_at FROM events WHERE job_id=?1 ORDER BY id DESC LIMIT 100").map_err(|e|e.to_string())?;
                let events=q.query_map([s(&request,"jobId")],|r|Ok(json!({"type":r.get::<_,String>(0)?,"data":r.get::<_,String>(1)?,"createdAt":r.get::<_,String>(2)?}))).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
                j["events"] = json!(events);
                return Ok(j);
            }
            "source.analyze" => {
                self.spawn_source_analysis(store.clone(), &request)?;
            }
            "source.import" => {
                let source = crate::importer::prepare_source_import(
                    request["text"].as_str(), Path::new(s(&request, "path")),
                )?;
                store.save_source(
                    serde_json::to_value(source)
                        .map_err(|e| e.to_string())?,
                )?;
            }
            "source.resolve" | "source.reparse" => {
                let expected = request["expectedSourceVersion"]
                    .as_i64()
                    .ok_or("缺少当前原文版本，请刷新后再确认")?;
                let raw = store.source()?;
                if raw["version"].as_i64() != Some(expected) {
                    return Err("原文范围已更新，请刷新后再确认".into());
                }
                let source = serde_json::from_value(raw).map_err(|e| e.to_string())?;
                let resolved = if action == "source.reparse" {
                    pipeline::reparse_source(&source)?
                } else {
                    let decisions: Vec<pipeline::BoundaryDecision> =
                        serde_json::from_value(request["decisions"].clone())
                            .map_err(|e| e.to_string())?;
                    if decisions.iter().any(|d| d.kind == "independent_episode" && d.confirmation.as_ref().is_none_or(|proof| proof.source_version != expected)) {
                        return Err("独立同号集确认版本已过期，请刷新后再确认".into());
                    }
                    pipeline::apply_boundary_decisions(&source, &decisions)?
                };
                let resolved = serde_json::to_value(resolved).map_err(|e| e.to_string())?;
                if action == "source.reparse" {
                    if resolved["parserVersion"] == "source-parser/v10" {
                        // Own the writer through the protected source/foundation/unit
                        // transaction; a live or unknown node cannot be superseded.
                        let _lease = self.acquire_pipeline(&store)?;
                        store.save_v10_reparsed_source_if_current(resolved, expected)?;
                    } else {
                        store.save_reparsed_source_if_current(resolved, expected)?;
                    }
                } else {
                    store.save_source_if_current(resolved, expected)?;
                }
            }
            "legacy.import" => {
                let report = crate::importer::import_legacy(
                    &store,
                    Path::new(s(&request, "path")),
                    &request,
                )?;
                store.event("", "legacy_import", &report)?;
                store.conn()?.execute("INSERT INTO metadata VALUES('last_import_report',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[report.to_string()]).map_err(|e|e.to_string())?;
            }
            "assets.imagePrompt.save" => {
                store.save_image_prompt(&request,&self.effective_settings(&store)?)?;
            }
            "assets.save" => {
                let original = store
                    .get("assets", s(&request["asset"], "id"))
                    .unwrap_or(json!({}));
                let mut asset = merge(original.clone(), &request["asset"]);
                if request["asset"].get("prompt").is_some() && original["prompt"] != asset["prompt"]
                {
                    asset["promptOrigin"] = json!("MANUAL");
                }
                store.save_asset_user(asset)?;
            }
            "assets.delete" | "assets.restore" => {
                store.set_asset_deleted(
                    s(&request, "assetId"),
                    request["expectedVersion"]
                        .as_i64()
                        .ok_or("请刷新资产后再操作")?,
                    action == "assets.delete",
                )?;
            }
            "units.save" => {
                let original = store
                    .get("units", s(&request["unit"], "id"))
                    .unwrap_or(json!({}));
                let mut merged = merge(original.clone(), &request["unit"]);
                if original["origin"]=="imported-storyboard" || merged["origin"]=="imported-storyboard" {
                    crate::lingmo_import::save_unit(&store,&original,merged)?;
                    return self.workspace(&store,None);
                }
                unit_save_guard::check_shots_unchanged(&original, &mut merged)?;
                let value = store.save_unit(merged)?;
                if original["exitState"] != value["exitState"]
                    || original["prompt"] != value["prompt"]
                {
                    store.invalidate_unit_dependents(s(&value, "id"))?;
                }
            }
            "media.import" => {
                if let Some(paths) = request["paths"].as_array() {
                    let mut imported = Vec::new();
                    let mut failures = Vec::new();
                    for path in paths {
                        match import_user_media(
                            &store,
                            &request,
                            Path::new(path.as_str().unwrap_or("")),
                        ) {
                            Ok(media) => imported.push(media["id"].clone()),
                            Err(message) => failures.push(json!({"path":path,"message":message})),
                        }
                    }
                    let mut workspace = self.workspace(&store, None)?;
                    workspace["mediaImportReport"] = json!({"importedCount":imported.len(),"mediaIds":imported,"failures":failures,"order":"input","currentImage":"lastSuccessfulImport"});
                    return Ok(workspace);
                }
                import_user_media(&store, &request, Path::new(s(&request, "path")))?;
            }
            "images.ensure_current" => {
                let report = store.ensure_current_images()?;
                let mut workspace = self.workspace(&store, None)?;
                workspace["currentImageInitialization"] = report;
                return Ok(workspace);
            }
            "media.adopt" => store.adopt(
                s(&request, "assetId"),
                s(&request, "variantId"),
                s(&request, "mediaId"),
            )?,
            "media.adopt_batch" => store.adopt_batch(
                request["selections"]
                    .as_array()
                    .ok_or("请选择要采用的候选")?,
            )?,
            "videos.adopt" => adopt_video(&store, s(&request, "unitId"), s(&request, "mediaId"))?,
            "settings.save" => {
                let data = &request["settings"];
                if data.get(crate::app_settings::SCOPE).is_some(){return Err("全局执行身份只能由程序冻结，不能写入项目设置。".into());}
                let old = store.settings()?;
                let pid = s(&request, "projectId");
                for (k, provider) in [
                    ("bhToken", "bh"),
                    ("chromeToken", "chrome"),
                    ("textApiKey", "text"),
                ] {
                    if let Some(secret) = data[k].as_str().filter(|s| !s.is_empty()) {
                        crate::platform::write_credential(&format!("{pid}:{provider}"), secret)
                            .map_err(|e| format!("无法安全保存凭据: {e}"))?;
                    }
                }
                store.save_settings(data)?;
                if self.live_accounts && data.get("dreaminaPath").is_some() && data["dreaminaPath"] != old["dreaminaPath"]
                {
                    self.dreamina_sessions
                        .forget(&self.dreamina_config(&store)?)?;
                }
                if [
                    "ratio",
                    "videoResolution",
                    "imageResolution",
                    "videoProvider",
                    "videoModel",
                    "imageProvider",
                    "imageModel",
                    "bhEndpoint",
                    "dreaminaPath",
                ]
                .iter()
                .any(|k| data.get(*k).is_some() && data[*k] != old[*k])
                {
                    store.invalidate_unsubmitted_settings("生成设置已变化，请重新准备")?;
                }
                crate::executable_paths::remember(&self.home, data)?;
            }
            "runtime.prepare" => {
                let _setup = self.acquire_keys(vec!["runtime:prepare".into()])?;
                return crate::runtime_setup::prepare(&request, &store.settings()?, &self.home, self.readiness_resources.as_deref());
            }
            "readiness.check" => {
                let (provider,mut settings)=crate::readiness::request_settings(&request,&store.settings()?)?;
                if provider=="dreamina_canvas" {
                    if let Ok(path)=crate::canvas_oauth::resolve(&settings,self.readiness_resources.as_deref(),&self.home) {settings["dreaminaCanvasPath"]=json!(path);}
                }
                let _readiness=self.acquire_keys(vec![format!("readiness:{provider}")])?;
                return Ok(crate::text_cli::control_runtime()?.block_on(crate::readiness::check(&provider,&settings,self.readiness_resources.as_deref())));
            }
            "textSdk.status" => {
                return Ok(crate::text_cli::control_runtime()?.block_on(crate::text_sdk::status(&store.settings()?)));
            }
            "textCli.status"
            | "textCli.login"
            | "textCli.loginStatus"
            | "textCli.cancelLogin"
            | "textCli.openLogin" => {
                if !self.live_accounts {
                    if action == "textCli.status" {
                        return Ok(
                            json!({"available":false,"ready":false,"loggedIn":false,"models":[],"limits":[],"warnings":["离线启动验收不读取宿主 CLI 账户"],"login":{"state":"idle"},"generationRequestsSent":0,"diagnostic":true}),
                        );
                    }
                    return Err("离线验收模式不启动账户登录".into());
                }
                let mut settings = store.settings()?;
                if action == "textCli.status" {
                    if let Some(path) = request["settings"]["codexPath"].as_str() {
                        settings["codexPath"] = json!(path.trim());
                    }
                }
                let runtime = crate::text_cli::control_runtime()?;
                return match action {
                    "textCli.status" => {
                        Ok(runtime.block_on(crate::text_cli::status(&settings, true)))
                    }
                    "textCli.login" => runtime.block_on(crate::text_cli::start_login(&settings)),
                    "textCli.loginStatus" => Ok(crate::text_cli::login_status()),
                    "textCli.cancelLogin" => Ok(crate::text_cli::cancel_login()),
                    _ => runtime.block_on(crate::text_cli::open_login_url()),
                };
            }
            "dreamina.status" | "dreamina.login" | "dreamina.logout" => {
                let _guard = self.acquire_keys(vec!["channel:dreamina".into()])?;
                let config = self.dreamina_config(&store)?;
                let account = match action {
                    "dreamina.login" => {
                        let account = self.dreamina_sessions.begin(&config)?;
                        if s(&account, "status") == "authorizing" {
                            if let Err(error) =
                                open_authorization(s(&account["authorization"], "verificationUrl"))
                            {
                                store.event("", "operation_error", &json!({"message":error}))?;
                                return Err(error);
                            }
                        }
                        account
                    }
                    "dreamina.logout" => self.dreamina_sessions.logout(&config)?,
                    _ => self.dreamina_sessions.poll_or_refresh(&config)?,
                };
                self.resume_authenticated_dreamina_locked(&store, &account)?;
            }
            "dreamina.open_authorization" => {
                let account = self
                    .dreamina_sessions
                    .snapshot(&self.dreamina_config(&store)?)?;
                if s(&account, "status") != "authorizing" {
                    return Err("没有正在进行的即梦授权，请先连接。".into());
                }
                open_authorization(s(&account["authorization"], "verificationUrl"))?;
            }
            "providers.refresh" => {
                let provider = s(&request, "provider");
                let _guard = self.acquire_keys(vec![format!("channel:{provider}")])?;
                let (c, _) = self.config(&store, provider)?;
                if provider == "dreamina" && self.live_accounts {
                    // snapshot/due are scoped to this configured CLI, not another project's
                    // cached account. Model inspection need not repeat a fresh credit query.
                    let account = model_refresh_account(
                        self.dreamina_sessions.snapshot(&c)?,
                        self.dreamina_sessions.due(&c)?,
                        || self.dreamina_sessions.poll_or_refresh(&c),
                    )?;
                    self.resume_authenticated_dreamina_locked(&store, &account)?;
                }
                let (c, account) = self.config(&store, provider)?;
                let fingerprint = capability_fingerprint(&c, &account);
                let cap = match self.channels.capabilities(&c) {
                    Ok(cap) => cap,
                    Err(e) => {
                        if e.auth_required {
                            store
                                .conn()?
                                .execute("DELETE FROM catalogs WHERE provider=?1", [provider])
                                .map_err(|e| e.to_string())?;
                        }
                        return Err(e.message);
                    }
                };
                let (current, current_account) = self.config(&store, provider)?;
                if capability_fingerprint(&current, &current_account) != fingerprint {
                    return Err("渠道或账户配置在核实时发生变化，请重试".into());
                }
                save_capability_cache(&store, &c, &account, &fingerprint, &cap)?;
                return Ok(json!(cap));
            }
            "videos.preflight" => {
                let provider = video_provider(&store, &request)?;
                let _guard = self.acquire_keys(vec![format!("channel:{provider}")])?;
                let verified = self.video_channel(&store, &provider, &request)?;
                let mut workspace = self.workspace(&store, None)?;
                workspace["videoPreflight"] = verified.report;
                return Ok(workspace);
            }
            "videos.prepare" => {
                let provider = video_provider(&store, &request)?;
                let report = if provider == "dreamina" {
                    self.prepare_dreamina_videos(&store, &request, Duration::from_secs(30))?
                } else {
                    let _guard = self.acquire_keys(vec![format!("channel:{provider}")])?;
                    self.queue_videos(&store, &request, false)?
                };
                let mut workspace = self.workspace(&store, None)?;
                workspace["videoQueueReport"] = report;
                return Ok(workspace);
            }
            "videos.submit" => {
                let provider = video_provider(&store, &request)?;
                let _guard = self.acquire_keys(vec![format!("channel:{provider}")])?;
                let report = self.queue_videos(&store, &request, true)?;
                let mut workspace = self.workspace(&store, None)?;
                workspace["videoQueueReport"] = report;
                return Ok(workspace);
            }
            "images.reviewCapability" | "images.reviewCapability.restartUnknown" | "images.reviewCandidate" => {
                self.spawn_visual_review(store.clone(),request.clone())?;
            }
            "images.reviewDelegation.set" => {
                let enabled=request["enabled"].as_bool().ok_or("请明确开启或关闭委托审图")?;
                store.set_visual_review_delegation(enabled,&self.effective_settings(&store)?)?;
            }
            "images.reviewReconcile" => {
                let _guard=self.acquire_keys(project_worker_keys(&store,vec!["channel:sdk-visual-review".into(),format!("pipeline:{}",s(&store.project()?,"id"))])?)?;
                let result=crate::visual_review::reconcile(&store,&self.effective_settings(&store)?,&request)?;
                let mut workspace=self.workspace(&store,None)?;
                workspace["imageReviewReconciliation"]=result;
                return Ok(workspace);
            }
            "images.run" => {
                if request.get("backgroundChromeBinding").is_some() {
                    return Err("后台队列绑定只能由专用执行入口创建".into());
                }
                let report = self.queue_images(&store, &request)?;
                let mut workspace = self.workspace(&store, None)?;
                workspace["imageQueueReport"] = report;
                return Ok(workspace);
            }
            "storyboard.refinement.continuity.preview" => {
                let plan=store.preview_refinement_continuity(s(&request,"workflowId"),s(&request,"candidateSha256"))?;
                let mut workspace=self.workspace(&store,None)?;
                workspace["refinementContinuityReviewPreview"]=plan;
                return Ok(workspace);
            }
            "storyboard.refinement.adopt" => {
                let _pipeline_lease = self.acquire_pipeline(&store)?;
                let run = s(&request, "workflowId");
                if let Some(review)=request.get("continuityReview") {
                    store.adopt_refinement_continuity(run,s(&request,"candidateSha256"),review)?;
                    return self.workspace(&store,None);
                }
                let old = store.job(run)?;
                if old["entityId"] != "storyboard.refine" { return Err("请选择原导演精修记录".into()); }
                let original = &old["snapshot"]["input"];
                if old["snapshot"]["savedSettingsSha256"] != pipeline::digest(&store.settings()?.to_string()) { return Err("项目参数已变化，精修结果已保留，请核对后重新精修".into()); }
                let mut current = refinement_input_with_capabilities(&store, &json!({"episodeId":original["episodeId"],"refinementScope":original["refinementScope"],"reviewNotes":original["reviewNotes"],"preserveNotes":original["preserveNotes"]}),original.get("capabilities"))?;
                let mut effective_original=original.clone();
                pipeline::storyboard_refinement_identity::attach_recovery(&mut effective_original,&old["snapshot"],&old["prepared"])?;
                pipeline::storyboard_refinement::inherit_frozen_profiles(&mut current, &effective_original)?;
                // Execution identity is frozen by preflight, not editable project state.
                current["settings"] = original["settings"].clone();
                store.publish_storyboard_refinement(run, s(&request,"candidateSha256"), &current)?;
            }
            "foundation.run" | "images.design" | "images.produce" | "storyboard.run" | "storyboard.refine" => {
                self.spawn_stage(store.clone(), request.clone())?;
            }
            "jobs.archive" | "jobs.restore" => {
                let key = s(&request, "jobId");
                let _guard = self.acquire_keys(vec![format!("job:{key}")])?;
                let expected = request.get("expectedSnapshotSha256").map(|v| v.as_str().ok_or("冻结材料摘要参数无效")).transpose()?;
                store.set_task_record_archived(key, action == "jobs.archive", expected)?;
            }
            "jobs.close_unknown" => {
                let key = s(&request, "jobId");
                let _guard = self.acquire_keys(vec![format!("job:{key}")])?;
                if store.job(key)?["provider"] == "codex_sdk" {
                    return Err("SDK_UNRESOLVED_HELD: 缺少远端被动对账及进程树停止证明，保留 SDK 原执行，不通过关闭记录重发".into());
                }
                store.close_unknown_job(
                    key,
                    request["confirmedNotAccepted"].as_bool() == Some(true),
                    s(&request, "reason"),
                )?;
            }
            "jobs.query" | "jobs.download" | "jobs.reconcile" => {
                let key = s(&request, "jobId").to_owned();
                let job = store.job(&key)?;
                let (cfg, account) = self.generation_config(&store, &job)?;
                let engine = self.clone();
                let st = store.clone();
                let act = action.to_owned();
                let remote = s(&request, "providerTaskId").to_owned();
                let locks = project_worker_keys(
                    &store,
                    vec![
                        format!("job:{key}"),
                        format!("channel:{}", s(&job, "provider")),
                    ],
                )?;
                self.spawn_exclusive_keys(locks, move || {
                    let account = match engine.checked_account(&cfg, &account) {
                        Ok(account) => account,
                        Err(error) => {
                            Self::account_preflight_failed(&st, &key, &error)?;
                            return Ok(());
                        }
                    };
                    let result = if act == "jobs.reconcile" {
                        if remote.is_empty() {
                            tasks::reconcile(&st, &key, &cfg, &account, engine.channels.as_ref())
                        } else {
                            if s(&job, "status") != "submission_unknown"
                                || s(&job, "accountId") != account
                            {
                                return Err("仅原账户的未知提交可人工关联任务ID".into());
                            }
                            let query = engine
                                .channels
                                .query(&cfg, &remote)
                                .map_err(|e| e.message)?;
                            st.event(
                                &key,
                                "manual_task_identification",
                                &json!({"providerTaskId":remote,"userIdentified":true}),
                            )?;
                            st.update_job(
                                &key,
                                if query.status == "succeeded" {
                                    "succeeded"
                                } else {
                                    "submitted"
                                },
                                &json!({"providerTaskId":remote,"result":query}),
                            )
                        }
                    } else {
                        tasks::resume_job(
                            &st,
                            &key,
                            &cfg,
                            &account,
                            engine.channels.as_ref(),
                            act == "jobs.download",
                        )
                    };
                    if let Err(e) = &result {
                        st.event(&key, "operation_error", &json!({"message":e}))?;
                    }
                    result
                })?;
            }
            "project.export" => return portable::export(&store, Path::new(s(&request, "path"))),
            "backup.create" => return portable::backup(&store, true),
            "backup.restore" => return self.restore_backup(&store, Path::new(s(&request, "path"))),
            _ => return Err(format!("未知操作: {action}")),
        }
        self.workspace(&store, None)
    }
    fn spawn_exclusive<F>(self: &Arc<Self>, key: String, work: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        let mut keys = vec![key.clone()];
        if let Some(project_id) = key.strip_prefix("pipeline:") {
            keys.push(format!("project:{project_id}:{}", id()));
        }
        self.spawn_exclusive_keys(keys, work)
    }
    fn spawn_exclusive_keys<F>(self: &Arc<Self>, keys: Vec<String>, work: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        let guard = self.acquire_keys(keys)?;
        Self::spawn_owned(guard, work)
    }
    fn acquire_pipeline(self: &Arc<Self>, store: &Store) -> Result<ActiveGuard> {
        self.acquire_keys(project_worker_keys(store, vec![format!("pipeline:{}", s(&store.project()?, "id"))])?)
    }
    fn spawn_owned<F>(guard: ActiveGuard, work: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        // A lease may span a slow preflight. Keep the old post-preflight exit
        // gate before transferring ownership to a new worker.
        if guard.engine.stop.load(Ordering::SeqCst) {
            return Err("客户端正在退出".into());
        }
        std::thread::spawn(move || {
            let _guard = guard;
            let _ = work();
        });
        Ok(())
    }
    fn acquire_keys(self: &Arc<Self>, keys: Vec<String>) -> Result<ActiveGuard> {
        if self.stop.load(Ordering::SeqCst) {
            return Err("客户端正在退出".into());
        }
        let mut active = self.active.lock().map_err(|e| e.to_string())?;
        // Close the race with shutdown between the optimistic check and taking this lock.
        if self.stop.load(Ordering::SeqCst) {
            return Err("客户端正在退出".into());
        }
        if let Some(key) = keys.iter().find(|key| worker_key_conflicts(&active, key)) {
            let message = if let Some(provider) = key.strip_prefix("channel:") {
                let name = match provider {
                    "dreamina" => "即梦",
                    "bh" => "BH",
                    "chrome" => "Chrome 图片",
                    _ => provider,
                };
                format!("{name}渠道正在检查或处理已有任务，请稍后再试；这不影响原文和分镜阅读。")
            } else if key.starts_with("pipeline:") {
                "本项目的文字制作流程正在执行，请在任务记录查看进度。".to_owned()
            } else {
                "这项操作正在执行，请稍后刷新记录。".to_owned()
            };
            return Err(message);
        }
        for key in &keys {
            active.insert(key.clone());
        }
        drop(active);
        Ok(ActiveGuard {
            engine: self.clone(),
            keys,
        })
    }
    /// After stopping new work, drain only channel/job workers through their final receipt writes.
    /// Pipeline/text work is deliberately outside this wait. Repeated bounded calls are safe.
    pub fn await_channel_shutdown(&self, timeout_seconds: u64) -> bool {
        let started = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_seconds.min(60));
        loop {
            match self.active.lock() {
                Ok(active)
                    if !active
                        .iter()
                        .any(|key| key.starts_with("job:") || key.starts_with("channel:")) =>
                {
                    return true
                }
                Err(_) => return false,
                _ => {}
            }
            if started.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    /// Drain every owned worker, including final pipeline/text database writes after cancellation.
    pub fn await_worker_shutdown(&self, timeout_seconds: u64) -> bool {
        let started = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_seconds.min(60));
        loop {
            match self.active.lock() {
                Ok(active) if active.is_empty() => return true,
                Err(_) => return false,
                _ => {}
            }
            if started.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn start(self: &Arc<Self>) {
        if self.offline_only { return; }
        let engine = self.clone();
        std::thread::spawn(move || {
            while !engine.stop.load(Ordering::SeqCst) {
                let _ = engine.tick();
                let (pending, signal) = &engine.image_wake;
                let guard = pending.lock().unwrap_or_else(|e| e.into_inner());
                let (mut guard, _) = signal.wait_timeout_while(guard, Duration::from_secs(5), |wake| !*wake)
                    .unwrap_or_else(|e| e.into_inner());
                *guard = false;
            }
        });
    }
    pub fn tick(self: &Arc<Self>) -> Result<()> {
        if self.offline_only { return Ok(()); }
        for p in self.project_list()? {
            let lifecycle = self.lifecycle_mutex.lock().map_err(|e| e.to_string())?;
            if project_archived(&p) {
                continue;
            }
            // Re-check after taking the same lock used by archive/delete and dispatch.
            let store = match self.store(s(&p, "id")) {
                Ok(store) => store,
                Err(_) => continue,
            };
            let _operation = self.acquire_keys(project_worker_keys(&store, vec![])?)?;
            drop(lifecycle);
            let _ = portable::maybe_backup(&store);
            self.refresh_due_dreamina(&store)?;
            self.refresh_due_catalogs(&store)?;
            self.unblock_image_dependencies(&store)?;
            self.schedule_opencli_unknown_reads(&store)?;
            let mut jobs = store.pending_jobs()?;
            let query_next = self
                .media_query_next
                .lock()
                .map_err(|e| e.to_string())?
                .clone();
            jobs.sort_by(|a, b| {
                let rank = |job: &Value| {
                    let known = !s(job, "providerTaskId").is_empty();
                    let prefer_query = query_next.get(s(job, "provider")).copied().unwrap_or(true);
                    u8::from(known != prefer_query)
                };
                rank(a)
                    .cmp(&rank(b))
                    .then_with(|| {
                        a["nextAttemptAt"]
                            .as_i64()
                            .unwrap_or(0)
                            .cmp(&b["nextAttemptAt"].as_i64().unwrap_or(0))
                    })
                    .then_with(|| s(a, "updatedAt").cmp(s(b, "updatedAt")))
            });
            // Review-only preparations share one bounded channel reservation.
            // Paid submissions and original-ID recovery retain their existing slot/fairness rules.
            let mut prepare_candidates: HashMap<String, Vec<String>> = HashMap::new();
            for job in jobs.iter().filter(|job| tasks::prepare_only_eligible(job)) {
                prepare_candidates
                    .entry(s(job, "provider").to_owned())
                    .or_default()
                    .push(s(job, "id").to_owned());
            }
            let mut served = HashSet::new();
            let mut chrome_new_work_waiting = false;
            for j in jobs {
                let state = s(&j, "status");
                let provider = s(&j, "provider");
                if !["video", "image"].contains(&s(&j, "kind")) {
                    continue;
                }
                if ![
                    "queued",
                    "ready",
                    "submitted",
                    "processing",
                    "succeeded",
                    "query_failed",
                    "download_failed",
                ]
                .contains(&state)
                {
                    continue;
                }
                // Image operations own independent, bounded local worker lanes.
                // A cloud task waiting to finish consumes no local worker slot.
                if s(&j, "kind") == "image" && provider == "dreamina" {
                    self.schedule_image_operation(&store, &j)?;
                    continue;
                }
                if j["nextAttemptAt"].as_i64().unwrap_or(0) > chrono::Utc::now().timestamp()
                    || served.contains(provider)
                {
                    continue;
                }
                if state == "ready"
                    && s(&j, "providerTaskId").is_empty()
                    && j["snapshot"]["autoSubmit"].as_bool() != Some(true)
                {
                    continue;
                }
                let key = s(&j, "id").to_owned();
                let chrome_generation = provider == "chrome" && s(&j, "kind") == "image"
                    && ["queued", "ready"].contains(&state) && s(&j, "providerTaskId").is_empty();
                if chrome_generation {
                    if chrome_new_work_waiting { continue; }
                    if !self.chrome_image_queue_ready(&store, &key)? {
                        // One cross-project scan per queue pass, not per waiting image.
                        // Do not mark the provider served: original queries still run.
                        chrome_new_work_waiting = true;
                        continue;
                    }
                }
                let (cfg, account) = self.generation_config(&store,&j)?;
                let st = store.clone();
                let channels = self.channels.clone();
                // A durable ID is always query/download-only, even if an old
                // local state was incorrectly left queued/ready.
                let is_gen =
                    ["queued", "ready"].contains(&state) && s(&j, "providerTaskId").is_empty();
                let review_only = tasks::prepare_only_eligible(&j);
                let prepare_keys = if review_only {
                    prepare_candidates
                        .get(provider)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let locks = project_worker_keys(
                    &store,
                    vec![format!("channel:{provider}"), format!("job:{key}")],
                )?;
                let engine = self.clone();
                let spawned = self.spawn_exclusive_keys(locks, move || {
                    // The channel reservation alone ends after submit returns.
                    // Recheck durable ownership inside it to close cross-project
                    // races before account preflight or browser preparation.
                    if chrome_generation && !engine.chrome_image_queue_ready(&st, &key)? {
                        return Ok(());
                    }
                    let account = match engine.checked_account(&cfg, &account) {
                        Ok(account) => account,
                        Err(error) => {
                            Self::account_preflight_failed(&st, &key, &error)?;
                            return Ok(());
                        }
                    };
                    if review_only {
                        let started = Instant::now();
                        let fingerprint = capability_fingerprint(&cfg, &account);
                        for (attempt, candidate) in prepare_keys.iter().enumerate() {
                            // This is a between-items soft budget. One upstream upload may take longer.
                            if attempt >= 64 || started.elapsed() >= Duration::from_secs(2) || engine.stop.load(Ordering::SeqCst) {
                                break;
                            }
                            let _job_guard = if candidate == &key {
                                None // The original reservation already owns this job.
                            } else {
                                match engine.acquire_keys(vec![format!("job:{candidate}")]) {
                                    Ok(guard) => Some(guard),
                                    Err(_) => continue,
                                }
                            };
                            // Configuration edits must not make a burst silently use another account.
                            let (current_config, current_account) = match st.job(candidate).and_then(|job|engine.generation_config(&st, &job)) {
                                Ok(config) => config,
                                Err(error) => {
                                    if let Err(persist_error) = st.event(candidate, "local_error", &json!({"message":error})) {
                                        eprintln!("本地素材准备配置错误未能写入任务记录：{error}；{persist_error}");
                                    }
                                    break;
                                }
                            };
                            if capability_fingerprint(&current_config, &current_account) != fingerprint {
                                break;
                            }
                            if let Err(error) = tasks::prepare_only(&st, candidate, &cfg, &account, channels.as_ref()) {
                                if let Err(persist_error) = st.event(candidate, "local_error", &json!({"message":error})) {
                                    eprintln!("本地素材准备错误未能写入任务记录：{error}；{persist_error}");
                                }
                            }
                        }
                        return Ok(());
                    }
                    let r = if is_gen {
                        tasks::run_generation(&st, &key, &cfg, &account, channels.as_ref())
                    } else {
                        tasks::resume_job(&st, &key, &cfg, &account, channels.as_ref(), true)
                    };
                    if let Err(e) = r {
                        if let Err(persist_error) = st.event(&key, "local_error", &json!({"message":e})) {
                            eprintln!("本地任务错误未能写入任务记录：{e}；{persist_error}");
                        }
                    }
                    drop(engine);
                    Ok(())
                });
                if spawned.is_ok() {
                    served.insert(provider.to_string());
                    self.media_query_next
                        .lock()
                        .map_err(|e| e.to_string())?
                        .insert(provider.to_string(), is_gen);
                }
                // One due operation per channel. Each selected class yields the
                // next slot to its sibling class; absent/future work never idles
                // the other class. Existing due/backoff checks remain above.
            }
            self.schedule_browser_tab_cleanup(&store)?;
            self.schedule_visual_reviews(&store)?;
        }
        Ok(())
    }
    fn chrome_image_queue_ready(&self, store: &Store, key: &str) -> Result<bool> {
        let state = s(&store.job(key)?, "status").to_owned();
        if !["queued", "ready"].contains(&state.as_str()) { return Ok(false); }
        let result = self.chrome_image_queue_blocker(store, key);
        match result {
            Ok(None) => {store.clear_browser_queue_wait(key)?;Ok(true)},
            Ok(Some(reason)) => {
                store.defer_browser_queue(key, &state, &reason)?;
                Ok(false)
            }
            Err(error) => {
                store.claim_job_fields(key, &state, &state,
                    &json!({"error":format!("Chrome 队列等待核对原任务：{error}"),
                        "nextAttemptAt":chrono::Utc::now().timestamp()+10}))?;
                Ok(false)
            }
        }
    }
    // Read-only ownership check also used before refreshing a failed prepare.
    // An active original must finish before another task spends its proof attempt.
    fn chrome_image_queue_blocker(&self, store: &Store, key: &str) -> Result<Option<String>> {
        let current_project = s(&store.project()?, "id").to_owned();
        let current=store.job(key)?;
        let opencli=current["snapshot"]["providerConfig"]["browserTransport"]["transport"]=="opencli/v1";
            let registry = self.registry()?;
            for project in registry.as_array().ok_or("项目索引损坏")? {
                let except = if s(project, "id") == current_project { key } else { "" };
                if let Some(blocker) = Store::chrome_image_blocker_scoped(
                    Path::new(s(project, "rootPath")), s(project, "id"), except,opencli,
                )? { return Ok(Some(format!("等待项目「{}」中的原图片任务 {} 完成或恢复；当前图片尚未提交。", s(project,"title"), blocker))); }
            }
            Ok(None)
    }
    fn wake_images(&self) {
        let (pending, signal) = &self.image_wake;
        if let Ok(mut pending) = pending.lock() {
            *pending = true;
            signal.notify_one();
        }
    }
    fn image_preflight_failed(store: &Store, key: &str, error: &ProviderError) -> Result<()> {
        let job = store.job(key)?;
        let attempts = job["queryAttempts"].as_u64().unwrap_or(0);
        if !error.auth_required && error.code != "CLIENT_CLOSING"
            && s(&job, "providerTaskId").is_empty()
            && ["queued", "ready"].contains(&s(&job, "status")) && attempts < 2 {
            // Only a read-only preflight failed: generation has not been entered.
            // Two durable, delayed retries; unknown/accepted submissions never enter here.
            store.event(key, "account_preflight_retry", &json!({"code":error.code,"attempt":attempts+1,"generationCalled":false}))?;
            return store.update_job(key, s(&job, "status"), &json!({"error":error.message,"queryAttempts":attempts+1,"nextAttemptAt":chrono::Utc::now().timestamp()+10*(attempts as i64+1)}));
        }
        Self::account_preflight_failed(store, key, error)
    }
    fn schedule_browser_tab_cleanup(self: &Arc<Self>, store: &Store) -> Result<()> {
        if self.active.lock().map_err(|e|e.to_string())?.contains("channel:chrome") {return Ok(());}
        for key in tasks::due_browser_tab_cleanups(store)? {
            let job=store.job(&key)?;
            let (config,_)=match self.generation_config(store,&job) {Ok(config)=>config,Err(_)=>continue};
            let st=store.clone();
            let channels=self.channels.clone();
            let locks=project_worker_keys(store,vec!["channel:chrome".into(),format!("job:{key}")])?;
            if self.spawn_exclusive_keys(locks,move || tasks::run_browser_tab_cleanup(&st,&key,&config,channels.as_ref())).is_ok() {
                // One cleanup occupies the existing channel reservation. Remaining
                // durable entries are available on the next scheduler pass.
                break;
            }
        }
        Ok(())
    }
    fn schedule_opencli_unknown_reads(self: &Arc<Self>, store: &Store) -> Result<()> {
        if self.active.lock().map_err(|e|e.to_string())?.contains("channel:chrome") {return Ok(());}
        for job in tasks::opencli_unknown_jobs(store)? {
            let key=s(&job,"id").to_owned();
            let (config,account)=match self.generation_config(store,&job) {
                Ok(value)=>value,
                Err(_)=>{tasks::stop_opencli_unknown_read(store,&key,"FROZEN_CONFIG_UNAVAILABLE")?;continue;}
            };
            let st=store.clone();let channels=self.channels.clone();
            let locks=project_worker_keys(store,vec!["channel:chrome".into(),format!("job:{key}")])?;
            if self.spawn_exclusive_keys(locks,move || {
                // The original-page query itself verifies the real logged-in account.
                // No status probe, preparation, upload or submit is reachable here.
                tasks::run_opencli_unknown_read(&st,&key,&config,&account,channels.as_ref()).map(|_|())
            }).is_ok() {break;}
        }
        Ok(())
    }
    fn schedule_image_operation(self: &Arc<Self>, store: &Store, job: &Value) -> Result<()> {
        let state = s(job, "status");
        if job["nextAttemptAt"].as_i64().unwrap_or(0) > chrono::Utc::now().timestamp() {
            return Ok(());
        }
        let known = !s(job, "providerTaskId").is_empty();
        let (lane, slots) = if !known && ["queued", "ready"].contains(&state) {
            if job["snapshot"]["autoSubmit"].as_bool() != Some(true) { return Ok(()); }
            ("submit", IMAGE_SUBMIT_WORKERS)
        } else if known && state == "succeeded" {
            ("download", IMAGE_DOWNLOAD_WORKERS)
        } else if known {
            ("query", IMAGE_QUERY_WORKERS)
        } else { return Ok(()); };
        let key = s(job, "id").to_owned();
        let provider = s(job, "provider");
        // Compact pending projections omit M2's frozen provider/settings fields.
        // Reload the original job for every lane; current defaults never replace it.
        let (config, fallback) = match self.generation_config(store, job) {
            Ok(config) => config,
            Err(error) => {
                store.event(&key, "local_error", &json!({"message":error,"generationCalled":false}))?;
                store.claim_job_fields(&key, state, state, &json!({"error":error,"nextAttemptAt":chrono::Utc::now().timestamp()+10}))?;
                return Ok(());
            }
        };
        let mut reservation = None;
        for slot in 0..slots {
            let keys = project_worker_keys(store, vec![format!("image:{provider}:{lane}:{slot}"), format!("job:{key}")])?;
            if let Ok(guard) = self.acquire_keys(keys) { reservation = Some(guard); break; }
        }
        let Some(guard) = reservation else { return Ok(()); };
        let engine = self.clone();
        let store = store.clone();
        let expected_state = state.to_owned();
        std::thread::spawn(move || {
            let _guard = guard; // Completion wakes the scheduler after all DB writes and unlocks.
            let work = || -> Result<()> {
                let current = store.job(&key)?;
                if s(&current, "status") != expected_state { return Ok(()); }
                let identity = if engine.live_accounts {
                    engine.dreamina_sessions.image_execution_identity(&config)
                } else { Ok(fallback.clone()) };
                let account = match identity {
                    Ok(account) => account,
                    Err(error) => { Self::image_preflight_failed(&store, &key, &error)?; return Ok(()); }
                };
                match lane {
                    "submit" => tasks::run_generation(&store, &key, &config, &account, engine.channels.as_ref()),
                    "query" => tasks::resume_job(&store, &key, &config, &account, engine.channels.as_ref(), false),
                    _ => tasks::download_saved_result(&store, &key, &config, &account, engine.channels.as_ref()),
                }
            };
            if let Err(error) = work() {
                let _ = store.event(&key, "local_error", &json!({"message":error}));
                // Bound unexpected local failures instead of waking a tight retry loop.
                let _ = store.conn().and_then(|c| c.execute("UPDATE jobs SET next_attempt_at=?1 WHERE id=?2",rusqlite::params![chrono::Utc::now().timestamp()+10,key]).map(|_|()).map_err(|e|e.to_string()));
            }
        });
        Ok(())
    }
    fn refresh_due_dreamina(self: &Arc<Self>, store: &Store) -> Result<()> {
        if !self.live_accounts {
            return Ok(());
        }
        let config = self.dreamina_config(store)?;
        if !config.cli_path.as_ref().is_some_and(|p| p.is_file()) {
            return Ok(());
        }
        if !self.dreamina_sessions.due(&config)? {
            return Ok(());
        }
        let engine = self.clone();
        let st = store.clone();
        let locks = project_worker_keys(store, vec!["channel:dreamina".into()])?;
        let _ = self.spawn_exclusive_keys(locks, move || {
            let account = engine.dreamina_sessions.poll_or_refresh(&config)?;
            // Shared official CLI identity applies to every local project on this computer.
            for project in engine.project_list()? {
                let _lifecycle = engine.lifecycle_mutex.lock().map_err(|e| e.to_string())?;
                let target = match engine.store(s(&project, "id")) {
                    Ok(target) => target,
                    Err(_) => continue,
                };
                if crate::dreamina_session::config_key(&engine.dreamina_config(&target)?)
                    == crate::dreamina_session::config_key(&config)
                {
                    engine.resume_authenticated_dreamina_locked(&target, &account)?;
                }
            }
            let _ = st;
            Ok(())
        });
        Ok(())
    }
    fn refresh_due_catalogs(self: &Arc<Self>, store: &Store) -> Result<()> {
        let settings = store.settings()?;
        for provider in ["dreamina", "bh", "chrome"] {
            if provider == "dreamina"
                && s(&settings, "dreaminaPath").is_empty()
                && bundled_dreamina().is_none()
            {
                continue;
            }
            if provider == "chrome" && s(&settings, "chromeEndpoint").is_empty() {
                continue;
            }
            let key = format!("catalog_attempt:{provider}");
            let checked: Option<String> = store
                .conn()?
                .query_row("SELECT value FROM metadata WHERE key=?1", [&key], |r| {
                    r.get(0)
                })
                .ok();
            if checked
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds() < 900)
                .unwrap_or(false)
            {
                continue;
            }
            let (cfg, account) = self.config(store, provider)?;
            if provider == "bh"
                && cfg
                    .credential_env
                    .as_deref()
                    .and_then(|e| std::env::var(e).ok())
                    .is_none()
            {
                continue;
            }
            let channels = self.channels.clone();
            let engine = self.clone();
            let st = store.clone();
            let provider = provider.to_owned();
            let locks = project_worker_keys(
                store,
                vec![format!("catalog:{provider}"), format!("channel:{provider}")],
            )?;
            let _ = self.spawn_exclusive_keys(locks, move || {
                // A busy startup account check is a deferred attempt, not a failed refresh.
                // Only an acquired worker may start the 15-minute refresh interval.
                st.conn()?.execute("INSERT INTO metadata VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,now()]).map_err(|e|e.to_string())?;
                let fingerprint = capability_fingerprint(&cfg, &account);
                let (current, current_account) = engine.config(&st, &provider)?;
                if capability_fingerprint(&current, &current_account) != fingerprint {
                    return Err("渠道或账户配置在核实时发生变化，请重试".into());
                }
                match channels.capabilities(&cfg) {
                    Ok(cap) => {
                        let (current, current_account) = engine.config(&st, &provider)?;
                        if capability_fingerprint(&current, &current_account) != fingerprint {
                            return Err("渠道或账户配置在核实时发生变化，请重试".into());
                        }
                        save_capability_cache(&st, &cfg, &account, &fingerprint, &cap)?;
                    }
                    Err(e) => {
                        st.event(
                            "",
                            "catalog_refresh_failed",
                            &json!({"provider":provider,"message":e.message}),
                        )?;
                        if e.auth_required {
                            st.conn()?
                                .execute("DELETE FROM catalogs WHERE provider=?1", [provider])
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
                Ok(())
            });
        }
        Ok(())
    }
    fn unblock_image_dependencies(&self, store: &Store) -> Result<()> {
        self.unblock_image_dependencies_selected(store, None)
    }
    fn unblock_image_dependencies_selected(&self, store: &Store, selected: Option<&HashSet<String>>) -> Result<()> {
        let assets = store.values("assets")?;
        for summary in store
            .pending_jobs()?
            .iter()
            .filter(|j| s(j, "status") == "waiting_dependency")
            .filter(|j| selected.is_none_or(|ids| ids.contains(s(j, "id")) && j["kind"] == "image" && j["provider"] == "chrome"))
        {
            let full_job = store.job(s(summary, "id"))?;
            let j = &full_job;
            let a = match store.active_asset(s(j, "entityId")) {
                Ok(asset) => asset,
                Err(error) => {
                    store.update_job(s(j, "id"), "stale", &json!({"error":error}))?;
                    continue;
                }
            };
            if a["version"] != j["snapshot"]["assetVersion"] {
                store.update_job(s(j, "id"), "stale", &json!({"error":"派生设计已变化"}))?;
                continue;
            }
            let dependency_check: Result<bool> = (|| {
                if j["snapshot"]["workflowProfile"]==crate::asset_graph::PROFILE {
                    if j["snapshot"]["referenceRequirements"]!=a["referenceRequirements"] {return Err("等待期间参考需求已变化，原请求保留".into());}
                    return Ok(image_references(store,&assets,&a)?.is_some());
                }
                if j["snapshot"]["sourceChain"].is_array() && json!(pipeline::asset_design_contract::source_chain(&assets,&a)?) != j["snapshot"]["sourceChain"] {
                    return Err("等待期间指定来源设计已变化，保留冻结请求，请重新制作关联图片".into());
                }
                for key in j["snapshot"]["dependencyJobIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    let parent = store.job(key.as_str().ok_or("图片依赖任务ID无效")?)?;
                    let state = s(&parent, "status");
                    if [
                        "failed",
                        "blocked",
                        "stale",
                        "closed_by_user",
                        "result_expired",
                    ]
                    .contains(&state)
                    {
                        return Err("指定来源图片任务未完成，请处理该来源后重新制作关联图片".into());
                    }
                    if state != "downloaded" {
                        return Ok(false);
                    }
                    let source = store.active_asset(s(&parent, "entityId"))?;
                    if source["version"] != parent["snapshot"]["assetVersion"]
                        || source["variantId"] != parent["snapshot"]["variantId"]
                    {
                        return Err("来源图片任务的形象版本已变化，请重新准备关联图片".into());
                    }
                    let current = store.adopted(s(&source, "id"), s(&source, "variantId"))?;
                    if current.as_deref() != parent["result"]["mediaIds"][0].as_str()
                        || current.is_none()
                    {
                        return Err("指定来源任务的图片尚未成为当前素材，请重新准备关联图片".into());
                    }
                }
                Ok(true)
            })();
            match dependency_check {
                Ok(false) => continue,
                Err(error) => {
                    store.update_job(s(j, "id"), "blocked", &json!({"error":error}))?;
                    continue;
                }
                Ok(true) => {}
            }
            match image_references(store, &assets, &a) {
                Ok(Some(refs)) => {
                    let mut snapshot = j["snapshot"].clone();
                    snapshot["references"] = json!(refs);
                    if snapshot["workflowProfile"]==crate::asset_graph::PROFILE {snapshot["referenceBindings"]=snapshot["references"].clone();}
                    if let Err(error)=crate::asset_identity_layout::freeze(store,&mut snapshot) {
                        store.update_job(s(j,"id"),"blocked",&json!({"error":error}))?;
                        continue;
                    }
                    store.conn()?.execute("UPDATE jobs SET snapshot=?1 WHERE id=?2 AND state='waiting_dependency'",rusqlite::params![snapshot.to_string(),s(j,"id")]).map_err(|e|e.to_string())?;
                    store.claim_job(s(j, "id"), "waiting_dependency", "queued")?;
                }
                Ok(None) => {}
                Err(e) => {
                    store.update_job(s(j, "id"), "blocked", &json!({"error":e}))?;
                }
            }
        }
        Ok(())
    }
    fn video_channel(
        &self,
        store: &Store,
        provider: &str,
        request: &Value,
    ) -> Result<VideoChannel> {
        if self.live_accounts && !["dreamina", "bh"].contains(&provider) {
            return Err("所选渠道尚未适配视频生成".into());
        }
        let (config, mut account) = self.config(store, provider)?;
        if provider == "dreamina" && self.live_accounts {
            let mut status = self.dreamina_sessions.snapshot(&config)?;
            if self.dreamina_sessions.due(&config)? || status["ready"] != true {
                status = self.dreamina_sessions.poll_or_refresh(&config)?;
                self.resume_authenticated_dreamina(store, &status)?;
            }
            if status["ready"] != true
                || status["identityVerified"] != true
                || s(&status, "accountKey").is_empty()
            {
                return Err(if s(&status, "message").is_empty() {
                    "请先连接即梦官方账户".into()
                } else {
                    s(&status, "message").into()
                });
            }
            account = s(&status, "accountKey").to_owned();
        } else {
            self.require_account(store, provider)?;
            if self.live_accounts
                && provider == "bh"
                && config
                    .credential_env
                    .as_deref()
                    .and_then(|name| std::env::var(name).ok())
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err("请先在渠道设置中配置 BH 凭据".into());
            }
        }
        if request
            .get("expectedAccountKey")
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty())
            .is_some_and(|expected| expected != account)
        {
            return Err("账户身份已变化，请核对当前账户后重新准备".into());
        }
        let fingerprint = capability_fingerprint(&config, &account);
        let usable = self.global_capability_cache(provider,&fingerprint)?.or(cached_video_capabilities(store, provider, &fingerprint)?);
        let (capabilities, source) = if let Some(cap) = usable {
            (cap, "cache")
        } else {
            let cap = match self.channels.capabilities(&config) {
                Ok(cap) => cap,
                Err(error) => {
                    store.event(
                        "",
                        "video_preflight_failed",
                        &json!({"provider":provider,"message":error.message}),
                    )?;
                    if error.auth_required {
                        store
                            .conn()?
                            .execute("DELETE FROM catalogs WHERE provider=?1", [provider])
                            .map_err(|e| e.to_string())?;
                    }
                    return Err(error.message);
                }
            };
            if cap.provider != provider
                || cap.version.is_empty()
                || !timestamp_fresh(&cap.checked_at, 900)
            {
                return Err("渠道没有返回当前、完整且身份一致的能力目录".into());
            }
            let (current, current_account) = self.config(store, provider)?;
            if capability_fingerprint(&current, &current_account) != fingerprint {
                return Err("渠道或账户配置在核实时发生变化，请重试准备".into());
            }
            save_capability_cache(store, &config, &account, &fingerprint, &cap)?;
            (cap, "refreshed")
        };
        let report = json!({"provider":provider,"capabilityVersion":capabilities.version,"source":source,"checkedAt":capabilities.checked_at});
        Ok(VideoChannel {
            account,
            fingerprint,
            capabilities,
            report,
        })
    }
    /// The official adapter's prepare compiles local paths/arguments only. This
    /// verified cache path never acquires the paid/query channel reservation.
    fn cached_dreamina_video_channel(
        &self,
        store: &Store,
        r: &Value,
    ) -> Result<Option<VideoChannel>> {
        if !self.live_accounts {
            return Ok(None);
        }
        let (config, account) = self.config(store, "dreamina")?;
        let status = self.dreamina_sessions.snapshot(&config)?;
        if !verified_dreamina_session(&status, self.dreamina_sessions.due(&config)?) {
            return Ok(None);
        }
        if r.get("expectedAccountKey")
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty())
            .is_some_and(|expected| expected != account)
        {
            return Err("账户身份已变化，请核对当前账户后重新准备".into());
        }
        let fingerprint = capability_fingerprint(&config, &account);
        Ok(self.global_capability_cache("dreamina",&fingerprint)?.or(cached_video_capabilities(store, "dreamina", &fingerprint)?).map(|capabilities| {
            let report = json!({"provider":"dreamina","capabilityVersion":capabilities.version,"source":"cache","checkedAt":capabilities.checked_at});
            VideoChannel { account, fingerprint, capabilities, report }
        }))
    }
    fn video_prepare_context(&self, store: &Store, r: &Value) -> Result<Value> {
        if self.stop.load(Ordering::SeqCst) {
            return Err("客户端正在退出，未排入新的准备任务".into());
        }
        let project = store.project()?;
        let registry = self.registry()?;
        let registered = registry
            .as_array()
            .ok_or("项目索引损坏")?
            .iter()
            .find(|p| p["id"] == project["id"])
            .ok_or("项目登记已变化，未排入新的准备任务")?;
        if project_archived(registered) {
            return Err("项目已归档，未排入新的准备任务".into());
        }
        let (config, account) = self.config(store, "dreamina")?;
        let settings = self.effective_settings(store)?;
        let connection = store.conn()?;
        let ids = r["unitIds"].to_string();
        let source: i64 = connection
            .query_row("SELECT COALESCE(MAX(version),0) FROM sources", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        let units: String = connection.query_row("SELECT json_group_array(json_object('id',id,'version',version,'retired',retired)) FROM (SELECT id,version,COALESCE(json_extract(data,'$.retired'),0) AS retired FROM units WHERE id IN (SELECT value FROM json_each(?1)) ORDER BY id)", [&ids], |row| row.get(0)).map_err(|e| e.to_string())?;
        let references: String = connection.query_row("SELECT json_group_array(json_object('unit',unit_id,'order',position,'asset',asset_id,'variant',variant_id,'version',version,'media',media_id)) FROM (SELECT r.unit_id,r.position,r.asset_id,r.variant_id,a.version,d.media_id FROM unit_refs r LEFT JOIN assets a ON a.id=r.asset_id LEFT JOIN adoptions d ON d.asset_id=r.asset_id AND d.variant_id=r.variant_id WHERE r.unit_id IN (SELECT value FROM json_each(?1)) ORDER BY r.unit_id,r.position)", [&ids], |row| row.get(0)).map_err(|e| e.to_string())?;
        Ok(
            json!({"project":registered,"config":capability_fingerprint(&config,&account),"sourceVersion":source,"units":units,"references":references,"parameters":{"videoProvider":settings["videoProvider"],"videoModel":settings["videoModel"],"ratio":settings["ratio"],"videoResolution":settings["videoResolution"]}}),
        )
    }
    fn prepare_dreamina_videos(
        self: &Arc<Self>,
        store: &Store,
        r: &Value,
        timeout: Duration,
    ) -> Result<Value> {
        let started = Instant::now();
        let timeout = timeout.min(Duration::from_secs(30));
        let pid = s(&store.project()?, "id").to_owned();
        let original = self.video_prepare_context(store, r)?;
        // A duplicate preparation click is excluded without consuming the
        // channel slot used by unrelated images or known-ID queries.
        let _operation = self.acquire_keys(vec![format!("video-prepare:{pid}:dreamina")])?;
        loop {
            if self.video_prepare_context(store, r)? != original {
                return Err(
                    "项目、渠道、账户或所选分镜在等待核实时已变化；未排入新的准备任务，请刷新核对"
                        .into(),
                );
            }
            if let Some(verified) = self.cached_dreamina_video_channel(store, r)? {
                if self.video_prepare_context(store, r)? != original {
                    return Err("准备依据已变化，未排入新的准备任务".into());
                }
                return self.queue_videos_verified(store, r, false, Some(verified));
            }
            if started.elapsed() >= timeout {
                return Err("即梦渠道核实等待已达上限（最多30秒）；未排入新的准备任务，已有生成和原任务查询继续保留".into());
            }
            let guard = {
                let mut active = self.active.lock().map_err(|e| e.to_string())?;
                if self.stop.load(Ordering::SeqCst) {
                    return Err("客户端正在退出，未排入新的准备任务".into());
                }
                let key = "channel:dreamina".to_owned();
                if worker_key_conflicts(&active, &key) {
                    None
                } else {
                    active.insert(key.clone());
                    Some(ActiveGuard {
                        engine: self.clone(),
                        keys: vec![key],
                    })
                }
            };
            if let Some(_guard) = guard {
                let verified = self.video_channel(store, "dreamina", r)?;
                if self.video_prepare_context(store, r)? != original {
                    return Err("准备核实期间依据已变化，未排入新的准备任务".into());
                }
                return self.queue_videos_verified(store, r, false, Some(verified));
            }
            std::thread::sleep(
                Duration::from_millis(20).min(timeout.saturating_sub(started.elapsed())),
            );
        }
    }
    fn queue_videos(&self, store: &Store, r: &Value, submit: bool) -> Result<Value> {
        self.queue_videos_verified(store, r, submit, None)
    }
    fn queue_videos_verified(
        &self,
        store: &Store,
        r: &Value,
        submit: bool,
        verified: Option<VideoChannel>,
    ) -> Result<Value> {
        let ids = r["unitIds"]
            .as_array()
            .filter(|ids| !ids.is_empty())
            .ok_or("请选择分镜单元")?;
        let mut seen = HashSet::new();
        for id in ids {
            let key = id
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .ok_or("无效unitId")?;
            if !seen.insert(key) {
                return Err("分镜单元选择不能重复".into());
            }
        }
        let mut settings = self.effective_settings(store)?;
        crate::app_settings::freeze_credentials(&mut settings);
        let provider = video_provider(store, r)?;
        let model = r
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(s(&settings, "videoModel"))
            .trim();
        if model.is_empty() {
            return Err("请选择视频模型".into());
        }
        let verified = match verified {
            Some(value) => value,
            None => self.video_channel(store, &provider, r)?,
        };
        let capability = verified
            .capabilities
            .models
            .iter()
            .find(|m| m.id == model && m.kind == "video")
            .ok_or("所选模型不是此渠道已核实的视频模型")?;
        if !capability.adapted
            || capability.min_duration.is_none_or(|min| min == 0)
            || capability
                .max_duration
                .is_none_or(|max| Some(max) < capability.min_duration)
            || capability.ratios.is_empty()
            || capability.resolutions.is_empty()
        {
            return Err("所选视频模型的参数能力尚未完整适配".into());
        }
        let ratio = r
            .get("ratio")
            .and_then(Value::as_str)
            .unwrap_or(s(&settings, "ratio"))
            .trim();
        let resolution = r
            .get("resolution")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                settings
                    .get("videoResolution")
                    .and_then(Value::as_str)
                    .unwrap_or("720p")
            })
            .trim();
        let ratio = capability
            .ratios
            .iter()
            .find(|v| v.as_str() == ratio)
            .ok_or("所选比例不在此视频模型能力中")?
            .clone();
        let resolution = capability
            .resolutions
            .iter()
            .find(|v| v.eq_ignore_ascii_case(resolution))
            .ok_or("所选尺寸不在此视频模型能力中")?
            .clone();
        let (current, current_account) = self.config(store, &provider)?;
        if capability_fingerprint(&current, &current_account) != verified.fingerprint {
            return Err("渠道或账户配置在核实时发生变化，请重试准备".into());
        }
        let parameters = json!({"videoProvider":provider,"videoModel":model,"ratio":ratio,"videoResolution":resolution});
        if !submit {
            save_video_parameters_and_invalidate(
                store,
                &parameters,
                &verified.capabilities.version,
            )?;
        } else if ["videoProvider", "videoModel", "ratio", "videoResolution"]
            .iter()
            .any(|key| settings[*key] != parameters[*key])
        {
            return Err("当前参数与保存的准备设置不一致，请先重新准备并核对素材".into());
        }
        let source_projection = if !submit {
            store.rebind_verified_source_projection(ids,&json!({"minDuration":capability.min_duration,"maxDuration":capability.max_duration,"imageLimit":capability.images,"allowedDurations":capability.allowed_durations}))?
        } else { Value::Null };
        let jobs = store.job_summaries()?;
        let mut queued = Vec::new();
        let mut blocked = Vec::new();
        let mut reused = 0usize;
        for uid in ids {
            let key = uid.as_str().unwrap();
            let result: Result<Value> = (|| {
                if submit {
                    let summary = jobs
                        .iter()
                        .find(|j| {
                            s(j, "entityId") == key
                                && s(j, "kind") == "video"
                                && s(j, "status") == "ready"
                                && s(j, "provider") == provider
                        })
                        .ok_or("请先准备并查看冻结素材、价格估算")?;
                    let job = store.job(s(summary, "id"))?;
                    if s(&job, "accountId") != verified.account {
                        return Err("准备包属于其他账户，请恢复原账户或重新准备".into());
                    }
                    let snapshot = &job["snapshot"];
                    if s(snapshot, "model") != model
                        || snapshot["ratio"] != ratio
                        || snapshot["resolution"] != resolution
                        || s(snapshot, "capabilityVersion") != verified.capabilities.version
                        || s(&job["prepared"], "capabilityVersion") != verified.capabilities.version
                    {
                        store.conn()?.execute("UPDATE jobs SET state='stale',error='冻结参数或能力版本已变化，请重新准备素材包',next_attempt_at=0,updated_at=?1 WHERE id=?2 AND state='ready' AND (task_id IS NULL OR task_id='') AND (receipt IS NULL OR receipt IN ('','null','{}'))",rusqlite::params![now(),s(&job,"id")]).map_err(|e|e.to_string())?;
                        return Err("冻结参数或能力版本已变化，请重新准备素材包".into());
                    }
                    let prepared: crate::providers::PreparedRequest =
                        serde_json::from_value(job["prepared"].clone())
                            .map_err(|_| "冻结的渠道准备回执不完整")?;
                    let request = tasks::generation_request(snapshot)?;
                    if serde_json::to_value(&prepared.request).map_err(|e| e.to_string())?
                        != serde_json::to_value(&request).map_err(|e| e.to_string())?
                    {
                        return Err("准备回执与冻结素材包不一致，请重新准备".into());
                    }
                    if snapshot["problems"]
                        .as_array()
                        .is_none_or(|p| !p.is_empty())
                    {
                        return Err("冻结素材包仍有待处理问题".into());
                    }
                    binding::validate_snapshot(store, snapshot)?;
                    validate_video_capacity(capability, snapshot)?;
                    let mut snapshot = snapshot.clone();
                    snapshot["autoSubmit"] = json!(true);
                    let changed=store.conn()?.execute("UPDATE jobs SET snapshot=?1 WHERE id=?2 AND state='ready' AND (task_id IS NULL OR task_id='') AND account_id=?3",rusqlite::params![snapshot.to_string(),s(&job,"id"),verified.account]).map_err(|e|e.to_string())?;
                    if changed != 1 {
                        return Err("准备状态已变化，请刷新并重新核对".into());
                    }
                    return Ok(
                        json!({"unitId":key,"jobId":job["id"],"status":"ready","submissionEnabled":true}),
                    );
                }
                if let Some(summary) = jobs.iter().find(|j| {
                    s(j, "entityId") == key
                        && s(j, "kind") == "video"
                        && is_active_generation(s(j, "status"))
                }) {
                    let job = store.job(s(summary, "id"))?;
                    if s(&job, "status") == "ready"
                        && s(&job, "accountId") == verified.account
                        && s(&job["snapshot"], "model") == model
                        && job["snapshot"]["ratio"] == ratio
                        && job["snapshot"]["resolution"] == resolution
                        && s(&job["snapshot"], "capabilityVersion") == verified.capabilities.version
                        && s(&job["prepared"], "capabilityVersion") == verified.capabilities.version
                        && binding::validate_snapshot(store, &job["snapshot"]).is_ok()
                    {
                        reused += 1;
                        return Ok(
                            json!({"unitId":key,"jobId":job["id"],"status":"ready","reused":true}),
                        );
                    }
                    return Err("已有原任务或准备包，请恢复原任务或处理未知提交后再生成".into());
                }
                let unit = store.get("units", key)?;
                let mut snapshot = binding::bind(store, &unit, &provider, model)?;
                snapshot["frozenSettings"]=settings.clone();
                snapshot["providerConfig"]=serde_json::to_value(self.config_with_settings(store,&provider,&settings)?.0).map_err(|e|e.to_string())?;
                snapshot["autoSubmit"] = json!(false);
                snapshot["capabilityVersion"] = json!(verified.capabilities.version);
                snapshot["capabilitySnapshot"] = json!({"version":verified.capabilities.version,"checkedAt":verified.capabilities.checked_at,"model":capability});
                if let Err(message) = validate_video_capacity(capability, &snapshot) {
                    snapshot["problems"]
                        .as_array_mut()
                        .ok_or("素材问题列表无效")?
                        .push(json!({"code":"VIDEO_CAPACITY_REVISION_REQUIRED","message":message}));
                }
                let has_problems = snapshot["problems"]
                    .as_array()
                    .is_none_or(|p| !p.is_empty());
                let job = store.add_job(
                    "video",
                    key,
                    &provider,
                    &verified.account,
                    &snapshot,
                    if has_problems { "blocked" } else { "queued" },
                )?;
                if has_problems {
                    return Err(format!(
                        "素材包有待处理问题，请修订此单元后重新准备（任务 {job}）"
                    ));
                }
                Ok(json!({"unitId":key,"jobId":job,"status":"queued"}))
            })();
            match result {
                Ok(item) => queued.push(item),
                Err(reason) => blocked.push(json!({"unitId":key,"reason":reason})),
            }
        }
        if !blocked.is_empty() {
            store.event("", "batch_blocked", &json!(blocked))?;
        }
        if queued.is_empty() {
            return Err(blocked
                .iter()
                .map(|v| format!("{}: {}", s(v, "unitId"), s(v, "reason")))
                .collect::<Vec<_>>()
                .join("；"));
        }
        let mut preflight = verified.report;
        preflight["model"] = json!(model);
        preflight["ratio"] = json!(ratio);
        preflight["resolution"] = json!(resolution);
        preflight["sourceProjection"] = source_projection;
        Ok(
            json!({"queuedCount":queued.len()-reused,"reusedCount":reused,"blockedCount":blocked.len(),"queued":queued,"blocked":blocked,"preflight":preflight}),
        )
    }
    fn queue_images(&self, store: &Store, r: &Value) -> Result<Value> {
        let result = self.queue_images_with_account_check(store, r, |config, account| {
            self.checked_account(config, account)
        });
        self.wake_images();
        result
    }
    fn queue_images_with_account_check(
        &self,
        store: &Store,
        r: &Value,
        check: impl FnOnce(&ProviderConfig, &str) -> ProviderResult<String>,
    ) -> Result<Value> {
        self.queue_images_frozen(store, r, &self.effective_settings(store)?, |config,account| {
            let checked = check(config,account)?;
            if self.live_accounts && config.kind == "dreamina" {
                let current = self.dreamina_config(store).map_err(|e|ProviderError::new("CONFIG_CHANGED",e))?;
                if serde_json::to_value(current).ok() != serde_json::to_value(config).ok() {
                    return Err(ProviderError::new("CONFIG_CHANGED","即梦 CLI 配置在账户核实时已变化，请重试图片制作"));
                }
            }
            Ok(checked)
        })
    }
    fn queue_images_frozen(
        &self, store: &Store, r: &Value, settings: &Value,
        check: impl FnOnce(&ProviderConfig, &str) -> ProviderResult<String>,
    ) -> Result<Value> {
        let mut frozen_settings=settings.clone();
        crate::app_settings::freeze_credentials(&mut frozen_settings);
        let settings=&frozen_settings;
        let provider = s(r, "provider");
        let model = s(r, "model");
        if model.is_empty() {
            return Err("请选择图片模型".into());
        }
        let (config, cached_account) = self.config_with_settings(store, provider, settings)?;
        let account = if provider == "dreamina" && self.live_accounts {
            // Long asset design may outlive a failed background identity check.
            // Inspect the official account now; a cached failure is not logout proof.
            let account = match check(&config, &cached_account) {
                Ok(account) => account,
                Err(error) => {
                    store.event(
                        "",
                        "image_account_preflight_failed",
                        &json!({"provider":provider,"error":error,"newGenerationQueued":false}),
                    )?;
                    return Err(format!(
                        "即梦账户核实未完成（{}）：{}；尚未排入新的图片生成任务。",
                        error.code, error.message
                    ));
                }
            };
            if account.trim().is_empty() {
                return Err("即梦未返回可核实的账户身份，未排入新的图片生成任务".into());
            }
            let current = self.dreamina_config_with_settings(settings)?;
            if serde_json::to_value(&current).map_err(|e| e.to_string())?
                != serde_json::to_value(&config).map_err(|e| e.to_string())?
            {
                return Err("即梦 CLI 配置在账户核实时已变化，请重试图片制作".into());
            }
            account
        } else {
            let key=format!("{provider}AccountId");
            if !settings[crate::app_settings::SCOPE].is_object() && ["dreamina","bh","chrome"].contains(&provider) && store.settings()?[&key] != settings[&key] {
                return Err("图片制作期间账户身份已变化，请恢复原账户或发起新制作".into());
            }
            if provider=="chrome" && settings["chromeTransport"]=="opencli" {
                if s(settings,"opencliProfileId").is_empty()||s(settings,"opencliAccountKey").is_empty(){return Err("请在软件设置连接 OpenCLI 网页出图".into());}
            } else if settings[crate::app_settings::SCOPE].is_object() && ["bh","chrome"].contains(&provider) {
                crate::app_settings::credential(settings,s(&store.project()?,"id"),provider)?;
            } else if ["dreamina", "bh", "chrome"].contains(&provider) && s(settings, &format!("{provider}AccountId")).trim().is_empty() {
                return Err("冻结图片设置缺少稳定账户标签，请在新制作请求中选择账户".into());
            }
            cached_account
        };
        let new_workflow=r["workflowProfile"]==crate::asset_graph::PROFILE;
        let image_scope=store.image_production_scope()?;
        let selected = r["assetIds"].as_array().or_else(||if new_workflow {image_scope["assetIds"].as_array()}else{None});
        if selected.is_some_and(|ids| {
            ids.iter()
                .any(|id| id.as_str().is_none_or(|id| id.trim().is_empty()))
        }) {
            return Err("图片选择必须使用有效资产ID".into());
        }
        let assets = store.values("assets")?;
        let gate_input = json!({"source":store.source()?,"foundation":store.foundation()?,"assets":assets,"selection":{"assetIds":r["assetIds"]}});
        let visual_blocked = if new_workflow {Vec::new()}else{pipeline::asset_visual_facts::cached_visual_check_asset_ids(&gate_input)?};
        let policy = r["imageProductionPolicy"]
            .as_str()
            .filter(|p| ["automatic", "trialReview"].contains(p))
            .unwrap_or_else(|| pipeline::image_production_policy(&settings));
        let review = policy == "trialReview";
        let approved = settings["trialApproved"].as_bool() == Some(true);
        if review && !approved && r["trial"].as_bool() != Some(true) {
            return Err("请先生成少量试稿并确认画风，再启动批量生成".into());
        }
        let (order, closure_errors) = if new_workflow {
            let selection=selected.cloned().unwrap_or_default();
            for id in &selection {
                if !image_scope["assetIds"].as_array().is_some_and(|ids|ids.contains(id))
                    && !image_scope["referenceOnlyIds"].as_array().is_some_and(|ids|ids.contains(id)) {
                    return Err("所选图片超出正式范围；必要参考母版须先显式登记".into());
                }
            }
            crate::asset_graph::explicit_order(&assets,&selection)
        }else{image_closure(store, &assets, selected)};
        let mut selected_count = 0;
        for asset in assets.iter().filter(|a| order.contains(&s(a, "id").to_owned())) {
            if selected.map(|ids| ids.contains(&asset["id"])).unwrap_or(true)
                || store.adopted(s(asset, "id"), s(asset, "variantId"))?.is_none() {
                selected_count += 1;
            }
        }
        if review && !approved && selected_count > 3 {
            return Err("未确认画风时试稿最多选择3个资产".into());
        }
        let mut jobs = store.job_summaries()?;
        // UI summaries intentionally omit frozen payloads and native proofs.
        // Only full original evidence may release a known failed Chrome attempt.
        if provider=="chrome" {
            for job in jobs.iter_mut().filter(|j|j["provider"]=="chrome"&&j["kind"]=="image"&&j["status"]=="failed"&&!s(j,"providerTaskId").is_empty()) {
                *job=store.job(s(job,"id"))?;
            }
        }
        let mut errors = Vec::new();
        let mut blocked = Vec::new();
        let mut queued = Vec::new();
        let mut reused = Vec::new();
        let mut module22_context = crate::module22::ImageBatchContext::new(store);
        for issue in closure_errors {
            errors.push(format!("{}: {}", s(&issue, "assetId"), s(&issue, "reason")));
            blocked.push(issue);
        }
        let mut dependency_jobs: std::collections::HashMap<String, String> = jobs
            .iter()
            .filter(|j| s(j, "kind") == "image" && retains_image_generation(j))
            .map(|j| (s(j, "entityId").to_owned(), s(j, "id").to_owned()))
            .collect();
        for a in order
            .iter()
            .filter_map(|id| assets.iter().find(|a| s(a, "id") == id))
        {
            // Selecting a descendant also queues its missing ancestors. Existing
            // ancestor images are reused unless that ancestor was explicitly selected.
            if selected.map(|l| !l.contains(&a["id"])).unwrap_or(false)
                && store.adopted(s(a, "id"), s(a, "variantId"))?.is_some() {
                continue;
            }
            if !independent_image(a) {
                if a.pointer("/preparation/mode").and_then(Value::as_str) == Some("UNRESOLVED") {
                    blocked.push(
                        json!({"assetId":a["id"],"reason":"素材制作方式尚未明确，请调整资产"}),
                    );
                }
                continue;
            }
            if let Some(job) = jobs.iter().find(|j| {
                s(j, "entityId") == s(a, "id")
                    && s(j, "kind") == "image"
                    && retains_image_generation(j)
            }) {
                if s(job, "status") == "submission_unknown" {
                    blocked.push(json!({"assetId":a["id"],"jobId":job["id"],"status":job["status"],"reason":"原图片任务提交结果未知，保留原记录等待核对，不会再次提交生成"}));
                }
                reused.push(json!({"assetId":a["id"],"jobId":job["id"],"status":job["status"],"reused":true}));
                continue;
            }
            let result: Result<String> = (|| {
                if !independent_image(a) {
                    return Err("此形象沿用来源素材或为纯声音，无需独立出图".into());
                }
                if visual_blocked.iter().any(|id|id==s(a,"id")) {
                    return Err("视觉状态尚未归属或本批不适用裁决已失效，请先重新核对设计依据；已有提示词不能绕过".into());
                }
                if !new_workflow {pipeline::validate_asset_visual_gate(&gate_input, a)?;}
                if new_workflow && !a["referenceRequirements"].is_array() && !s(a,"sourceAssetId").is_empty() {
                    return Err("旧派生资产的媒体依赖尚未核对，请先编写新流程提示词；不会凭父标签直接派图".into());
                }
                let prompt_body=pipeline::asset_workflow::prompt_text(a);
                if new_workflow && (a["promptStatus"]=="needs_review" || a["imagePrompt"]["status"]=="needs_review") {
                    return Err("提示词依据已变化，需复核后生成；未提交图片请求".into());
                }
                if new_workflow {crate::asset_workflow_store::validate_prompt_source(store,&a["imagePrompt"])?;}
                if prompt_body.is_empty() {
                    return Err("资产缺少图片设计提示词".into());
                }
                let (mut refs, mut state) = match image_references(store, &assets, a) {
                    Ok(Some(refs)) => (refs, "queued"),
                    Ok(None) => (vec![], "waiting_dependency"),
                    Err(e) => return Err(e),
                };
                let mut dependency_job_ids = Vec::new();
                for dependency in image_dependency_ids(&assets, a)? {
                    let source = assets
                        .iter()
                        .find(|p| s(p, "id") == dependency)
                        .ok_or("图片依赖不存在")?;
                    if store
                        .adopted(&dependency, s(source, "variantId"))?
                        .is_some()
                    {
                        if let Some(job) = dependency_jobs.get(&dependency).filter(|_|!new_workflow) {
                            if !dependency_job_ids.contains(job) {
                                dependency_job_ids.push(job.clone());
                            }
                        }
                        continue;
                    }
                    let chain = image_reference_chain(&assets, source)?;
                    let mut has_source = false;
                    for dependency in chain {
                        if let Some(job) = dependency_jobs.get(s(dependency, "id")) {
                            has_source = true;
                            if !dependency_job_ids.contains(job) {
                                dependency_job_ids.push(job.clone());
                            }
                        }
                        if store
                            .adopted(s(dependency, "id"), s(dependency, "variantId"))?
                            .is_some()
                        {
                            has_source = true;
                            break;
                        }
                    }
                    if !has_source && !new_workflow {
                        return Err(format!("来源形象「{}」没有当前图片且未能排入生成队列；请查看该形象的失败原因", s(source, "name")));
                    }
                }
                if !dependency_job_ids.is_empty() {
                    refs.clear();
                    state = "waiting_dependency";
                }
                if new_workflow && r["onlyReady"] == true && state == "waiting_dependency" {
                    return Err("等待真实参考母图采用；本次只生成就绪图片，未创建生成任务".into());
                }
                let style = pipeline::asset_design_contract::project_visual_style(settings);
                let presentation = if crate::asset_graph::modern(a){pipeline::asset_workflow::image_presentation(a)}else{pipeline::asset_design_contract::image_presentation(settings, a)};
                // The web channel returns website-native files; a saved CLI/API
                // size is not a pixel-size option for this provider. Resolve only
                // new snapshots, keeping the saved settings and old jobs intact.
                let resolution = if provider == "chrome" { json!("native") }
                    else { settings.get("imageResolution").cloned().unwrap_or(json!("2K")) };
                let prompt = if crate::asset_graph::modern(a){prompt_body.to_owned()}else{pipeline::asset_design_contract::compile_image_prompt(prompt_body, style, &presentation)};
                let source_chain=if new_workflow{Value::Null}else{json!(pipeline::asset_design_contract::source_chain(&assets,a)?)};
                let mut snapshot = json!({"requestId":id(),"kind":"image","provider":provider,"model":model,"prompt":prompt,"promptBody":a["prompt"],"visualStyle":style,"imagePresentation":presentation,"frozenSettings":settings,"providerConfig":config,"designReceipt":a["design"]["designReceipt"],"designContext":a["design"]["frozenContext"],"sourceChain":source_chain,"imageProductionPolicy":policy,"assetId":a["id"],"assetVersion":a["version"],"variantId":a["variantId"],"sourceVersion":store.source()?["version"],"sourceVariantId":a["sourceVariantId"],"ratio":presentation["ratio"],"resolution":resolution,"references":refs,"dependencyJobIds":dependency_job_ids,"options":{},"autoSubmit":true,"ruleSnapshot":pipeline::rule_snapshot("asset-image-production")?});
                snapshot["promptCompilerVersion"] = json!(pipeline::asset_design_contract::IMAGE_PROMPT_COMPILER_VERSION);
                if new_workflow {
                    snapshot["workflowProfile"]=json!(crate::asset_graph::PROFILE);
                    snapshot["promptBody"]=json!(prompt_body);
                    snapshot["sourceChain"]=Value::Null;
                    snapshot["referenceRequirements"]=a["referenceRequirements"].clone();
                    snapshot["referenceBindings"]=json!(refs);
                    let visual_review=crate::visual_review::frozen_adoption_policy(store,settings)?;
                    snapshot["imageAdoptionPolicy"]=visual_review["policy"].clone();
                    snapshot["visualReviewDelegation"]=visual_review;
                    snapshot["ruleSnapshot"]=pipeline::asset_workflow::rule_snapshot(a);
                    snapshot["promptCompilerVersion"]=json!(pipeline::asset_workflow::PROMPT_VERSION);
                    snapshot["identityComposite"]=a["identityComposite"].clone();
                    if a["imagePrompt"].get("sourceBindingContract").is_some() {
                        snapshot["promptSource"]=json!({"sourceBindingContract":a["imagePrompt"]["sourceBindingContract"],
                            "sourceUnitBindings":a["imagePrompt"]["sourceUnitBindings"],"sourceKind":a["imagePrompt"]["sourceKind"],
                            "sourceSha256":a["imagePrompt"]["sourceSha256"],"sourceBasisSha256":a["imagePrompt"]["sourceBasisSha256"]});
                    }
                    if state=="queued" {crate::asset_identity_layout::freeze(store,&mut snapshot)?;}
                }
                if let Some(binding) = r.get("backgroundChromeBinding") {
                    if provider != "chrome" { return Err("后台图片入口仅允许 Chrome".into()); }
                    snapshot["backgroundChromeBinding"] = binding.clone();
                }
                if let Some(previous)=jobs.iter().find(|j|j["entityId"]==a["id"]&&crate::chrome_terminal::settled_job(j)) {
                    snapshot["retryOfJobId"]=previous["id"].clone();
                    snapshot["retryOfRequestId"]=previous["providerTaskId"].clone();
                    if provider=="chrome" && snapshot["providerConfig"]["browserTransport"]["transport"]=="opencli/v1"
                        && previous["result"]["raw"]["opencliNotSubmitted"]["clickCalls"]==0 {
                        // New explicit retry only. The old frozen request and
                        // its proven no-send journal remain unchanged.
                        snapshot["options"]["retryOfRequestId"]=previous["providerTaskId"].clone();
                    }
                }
                crate::module22::freeze_image_batch_with_context(store, a, &mut snapshot, &mut module22_context)?;
                store.add_job("image", s(a, "id"), provider, &account, &snapshot, state)
            })();
            match result {
                Ok(key) => {
                    dependency_jobs.insert(s(a, "id").to_owned(), key.clone());
                    queued.push(
                        json!({"assetId":a["id"],"jobId":key,"status":store.job(&key)?["status"]}),
                    );
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", s(a, "id")));
                    blocked.push(json!({"assetId":a["id"],"reason":e}));
                }
            }
        }
        if !errors.is_empty() {
            store.event("", "batch_blocked", &json!(errors))?;
        }
        if !new_workflow && queued.is_empty() && reused.is_empty() && !blocked.is_empty() {
            return Err(errors.join("；"));
        }
        // One-click Stage2 also enters this frozen-settings path directly.
        self.wake_images();
        Ok(
            json!({"queued":queued,"reused":reused,"blocked":blocked,"queuedCount":queued.len(),"reusedCount":reused.len(),"blockedCount":blocked.len(),"imageProductionPolicy":policy}),
        )
    }
    fn spawn_foundation_choices(self: &Arc<Self>, store: Store, request: &Value) -> Result<()> {
        let pipeline_lease = self.acquire_pipeline(&store)?;
        crate::text_executor::check_cli_unknown_hold(&store)?;
        let view = store.foundation_choices()?;
        if request["expectedSourceVersion"] != view["sourceVersion"]
            || request["expectedFoundationVersion"] != view["foundationVersion"]
        {
            return Err("原文或制作基础已变化，请刷新制作选择后再采用".into());
        }
        let decisions: Vec<_> = view["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["status"] == "CONFIRMED")
            .cloned()
            .collect();
        if decisions.is_empty() {
            return Err("没有待采用的已确认制作选择".into());
        }
        let selected: HashSet<_> = decisions
            .iter()
            .flat_map(|d| d["episodes"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .collect();
        let source = store.source()?;
        let episode_ids: Vec<_> = source["episodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| selected.contains(s(e, "id")))
            .map(|e| e["id"].clone())
            .collect();
        let mut input = production_input(
            &store,
            &json!({"action":"foundation.run","episodeIds":episode_ids}),
        )?;
        let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        input["settings"]=self.effective_settings(&store)?;
        crate::app_settings::freeze_credentials(&mut input["settings"]);
        input["settings"] =
            runtime.block_on(resolve_text_execution_settings(&input["settings"]))?;
        input["foundationPhase"] = json!("APPLY_DECISION");
        input["confirmedDecisions"] = json!(decisions);
        input["foundationModelInputProfile"] = json!(pipeline::foundation_wire::PROFILE);
        let guard = crate::workflow::guard_from_input(&input);
        let snapshot = json!({"runnerVersion":"desktop-runner/v1","foundationMethod":pipeline::foundation_delivery::METHOD_VERSION,"foundationPhase":"APPLY_DECISION","stage":"foundation.run","sourceVersion":input["source"]["version"],"selection":input["selection"],"input":input,"guard":guard,"rules":production_rules("foundation.run")});
        store.ensure_workflow_storage()?;
        let job = store.reserve_foundation_choice_workflow(&snapshot)?;
        self.execute_production_owned(store, &job, snapshot, pipeline_lease)
    }
    fn spawn_production(self: &Arc<Self>, store: Store, request: Value) -> Result<()> {
        let stage = s(&request, "action");
        // New refinement must own the project before reading its adopted basis,
        // inspecting an account, or reserving any workflow/budget. Move this
        // same lease into the worker; a check followed by reacquisition races.
        let refinement_lease = if stage == "storyboard.refine" {
            Some(self.acquire_pipeline(&store)?)
        } else { None };
        // New production cannot hide a dispatched but unacknowledged CLI turn
        // behind a different workflow, episode selection, or text provider.
        // Original resume remains a separate path able to read saved results.
        crate::text_executor::check_cli_unknown_hold(&store)?;
        // Serialize each foundation/Stage3 start before a workflow or preflight
        // is reserved. The existing pipeline key still owns actual execution.
        let _production_start = if ["foundation.run","storyboard.run"].contains(&stage) {
            let key = if stage == "foundation.run" { "foundation-start" } else { "storyboard-start" };
            Some(self.acquire_keys(vec![format!("{key}:{}", s(&store.project()?, "id"))])?)
        } else { None };
        let planning=if matches!(stage,"storyboard.run"|"storyboard.refine"){self.global_planning_capabilities(&store)?}else{None};
        let mut input = if stage == "storyboard.refine" { refinement_input_with_capabilities(&store, &request,planning.as_ref())? } else { production_input_for_new_workflow_with_capabilities(&store, &request,planning.as_ref())? };
        if stage == "storyboard.refine" { pipeline::asset_use_binding::freeze_new(&mut input)?; }
        if stage == "foundation.run" {
            match existing_foundation_start(&store, &input)? {
                FoundationStartBoundary::New => {},
                FoundationStartBoundary::Resume(run) => {
                    let analysis_parent=s(&request,"sourceAnalysisWorkflowId");
                    if !analysis_parent.is_empty() {store.link_existing_source_analysis_successor(analysis_parent,&run)?;}
                    return self.resume_production_checked(store, &json!({"action":"workflows.resume","workflowId":run}),crate::execution_policy::creative_runtime_ready());
                },
                FoundationStartBoundary::Hold(reason) => return Err(reason),
            }
        }
        if stage == "storyboard.run" {
            match existing_storyboard_start(&store, &input)? {
                StoryboardStartBoundary::New => {},
                StoryboardStartBoundary::Resume(run) => {
                    return self.resume_production_checked(store, &json!({"action":"workflows.resume","workflowId":run}),crate::execution_policy::creative_runtime_ready());
                },
                StoryboardStartBoundary::Hold(reason) => return Err(reason),
            }
        }
        // Existing workflows return through their original resume path above.
        // Every genuinely new production holds ownership through reservation.
        let pipeline_lease = match refinement_lease {
            Some(lease) => lease,
            None => self.acquire_pipeline(&store)?,
        };
        crate::text_executor::check_cli_unknown_hold(&store)?;
        // Only a genuinely new writer/refinement gets the scene contract. The
        // resume returns above and saved-snapshot recovery keep their exact input.
        if stage=="storyboard.run" && pipeline::series::uses_profile(&input)? {
            // Reserve stable identities only after ruling out frozen recovery
            // and taking the project lease. Refresh versions before CAS.
            store.ensure_series_asset_identities()?;
            input=production_input_for_new_workflow_with_capabilities(&store,&request,planning.as_ref())?;
            if prepare_series_continuation(&store,&mut input)? {return Ok(());}
        }
        if matches!(stage, "storyboard.run" | "storyboard.refine") && !pipeline::series::uses_profile(&input)? {
            pipeline::freeze_scene_input_profile(&mut input)?;
            pipeline::storyboard_sound_events::freeze_new(&mut input)?;
            pipeline::asset_coverage::freeze_new(&mut input)?;
        }
        let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
        let saved_settings_sha256 = freeze_production_execution_settings(&store, stage, &mut input,
            |settings| {let mut effective=if self.standard_native {crate::app_settings::effective(&self.home,settings)?} else {settings.clone()};crate::app_settings::freeze_credentials(&mut effective);runtime.block_on(resolve_text_execution_settings(&effective))})?;
        if stage == "foundation.run" {
            // Freeze the new transport on new runs only. Resuming an existing
            // workflow keeps its original input and accepted request identities.
            input["foundationModelInputProfile"] = json!(pipeline::foundation_wire::PROFILE);
            pipeline::asset_coverage::freeze_new(&mut input)?;
            input["projectId"] = store.project()?["id"].clone();
            input["sourceVersion"] = input["source"]["version"].clone();
            input["foundationDecisionPolicy"] =
                json!(pipeline::foundation_delivery::AUTO_INTERPRET_POLICY);
            input["userReviewRequirements"] = input["settings"]
                .get("userReviewRequirements")
                .cloned()
                .unwrap_or(json!([]));
            if input["foundation"]["sourceSha256"] == input["source"]["sha256"] {
                let decisions =
                    store.foundation_automatic_decisions(&input["userReviewRequirements"])?;
                if !decisions.is_empty() {
                    input["foundationPhase"] = json!("AUTO_INTERPRET");
                    input["automaticDecisions"] = json!(decisions);
                }
            }
        }
        if stage == "storyboard.run" && pipeline::series::uses_profile(&input)? {
            input["storyboardMethod"] = json!("series-full-episode/v1");
        }
        let rules = if pipeline::series::uses_profile(&input)? { json!(pipeline::series::rule_snapshots_for(&input)?) } else if pipeline::stage3_pack::uses_profile(&input)? { json!(pipeline::stage3_pack::rule_snapshots()?) } else { production_rules(stage) };
        let guard = crate::workflow::guard_from_input(&input);
        store.ensure_workflow_storage()?;
        {
            let mut c = store.conn()?;
            let tx = c.transaction().map_err(|e| e.to_string())?;
            crate::workflow::verify_guard(&tx, &guard)?;
        }
        let native_receipt=self.reserve_native_workflow(&request,&input["source"],&input["settings"])?;
        let mut snapshot = json!({"runnerVersion":"desktop-runner/v1","foundationMethod":if stage=="foundation.run" {json!("foundation-delivery/v2")}else{Value::Null},"stage":stage,"savedSettingsSha256":saved_settings_sha256,"sourceVersion":input["source"]["version"],"selection":input["selection"],"planningProfile":input["capabilities"],"input":input,"guard":guard,"rules":rules,"nativeAcceptance":native_receipt});
        let source_analysis_parent = s(&request, "sourceAnalysisWorkflowId");
        if !source_analysis_parent.is_empty() {
            if stage != "foundation.run" { return Err("格式识别只能衔接制作基础".into()); }
            snapshot["sourceAnalysisContinuation"] = json!({"version":"source-analysis-continuation/v1","workflowId":source_analysis_parent});
        }
        let job = if !source_analysis_parent.is_empty() {
            store.reserve_foundation_after_source_analysis(source_analysis_parent, &snapshot)?
        } else if stage == "storyboard.refine" {
            store.reserve_storyboard_refinement_workflow(&snapshot)?
        } else if input["foundationPhase"] == "AUTO_INTERPRET" {
            store.reserve_foundation_auto_workflow(&snapshot)?
        } else {
            store.add_job("workflow", stage, "local", "local", &snapshot, "queued")?
        };
        self.bind_native_workflow(&store,&native_receipt,&job)?;
        self.execute_production_owned(store, &job, snapshot, pipeline_lease)
    }
    fn resume_production_checked(self: &Arc<Self>, store: Store, request: &Value, creative_ready: bool) -> Result<()> {
        if self.standard_native {
            check_standard_workflow_resume(creative_ready, &store.job(s(request,"workflowId"))?)?;
        }
        self.resume_production(store, request)
    }
    fn resume_production(self: &Arc<Self>, store: Store, request: &Value) -> Result<()> {
        let id = s(request, "workflowId");
        let job = store.job(id)?;
        if job["entityId"] == "source.analyze" {
            return self.resume_source_analysis(store, request, &job);
        }
        if request.get("supplyContractCorrection").is_some()
            && (s(request,"action")!="workflows.resume" || job["entityId"]!="storyboard.run"
                || job["snapshot"]["input"]["workflowProfile"]!=crate::asset_graph::PROFILE) {
            return Err("供料合同纠正仅适用于同一条新分镜原流程，未新建任务".into());
        }
        if request.get("controllerReviewSupplement").is_some()
            && (s(request,"action")!="workflows.resume" || job["entityId"]!="storyboard.run"
                || job["snapshot"]["input"]["workflowProfile"]!=crate::asset_graph::PROFILE) {
            return Err("主控审稿补充仅适用于同一条新版分镜原流程，未新建任务".into());
        }
        if job["snapshot"]["stage2Context"]["workflowProfile"]==crate::asset_graph::PROFILE {
            if s(&job,"kind")!="workflow" || !["failed","interrupted","partially_blocked","blocked"].contains(&s(&job,"status")){return Err("请选择中断的原提示词流程".into());}
            let lease=self.acquire_pipeline(&store)?;
            return self.execute_image_workflow_v1(store,id.to_owned(),job["snapshot"].clone(),lease);
        }
        if job["snapshot"]["stage"]=="images.design" {
            if s(request,"action")!="workflows.resume" || s(&job,"kind")!="workflow" || !["failed","interrupted","partially_blocked","blocked"].contains(&s(&job,"status")) {
                return Err("请选择中断的原资产文字设计流程".into());
            }
            let pipeline_lease = self.acquire_pipeline(&store)?;
            let snapshot=job["snapshot"].clone();
            let original=&snapshot["stage2Context"];
            if !original.is_object() || original["designRuleSha256"].as_str().is_none() || snapshot["sourceVersion"]!=store.source()?["version"] {
                return Err("原资产设计缺少完整冻结上下文或源版本已变化，保留原稿且不重发".into());
            }
            let current_assets=store.values("assets")?;
            let current=json!({"source":store.source()?,"foundation":store.foundation()?,"assets":current_assets,"selection":original["selection"],"settings":snapshot["settings"]});
            let prior_facts=if job["result"]["publication"]["status"]=="held_partial" {&Value::Null} else {&job["result"]};
            pipeline::asset_design_contract::validate_resume_facts(original,prior_facts,&current)?;
            for asset in original["assets"].as_array().ok_or("缺少原冻结资产")?.iter().filter(|a|original["selection"]["assetIds"].as_array().is_none_or(|ids|ids.contains(&a["id"]))) {
                let updated=current_assets.iter().find(|a|a["id"]==asset["id"]).ok_or("原设计目标已删除，未重发")?;
                if pipeline::asset_design_contract::asset_semantics(asset)!=pipeline::asset_design_contract::asset_semantics(updated) {
                    return Err("原设计目标语义已变化，原回包和预算保留".into());
                }
                let chain=|assets:&[Value],asset:&Value|->Result<Value>{Ok(json!(pipeline::asset_design_contract::source_chain(assets,asset)?.iter().map(|a|json!({"assetId":a["assetId"],"semanticVersion":a["semanticVersion"]})).collect::<Vec<_>>()))};
                if chain(original["assets"].as_array().unwrap(),asset)?!=chain(&current_assets,updated)? {return Err("原设计直接来源链已变化，未重发".into());}
            }
            store.event(id,"asset_design_resume_requested",&json!({"sameWorkflow":true,"snapshotSha256":pipeline::digest(&snapshot.to_string()),"newWorkflowCreated":false}))?;
            return self.execute_asset_stage_owned(store,id.to_string(),snapshot,pipeline_lease);
        }
        // Early Stage2 records incorrectly called a wholly/partly blocked
        // image-production run completed. Resume is an explicit continuation
        // using its saved selection/settings; never rewrite that old receipt.
        if image_production_resume_eligible(&job) {
            if s(request, "action") != "workflows.resume" {
                return Err("图片制作流程不能采用分镜创作选择".into());
            }
            let pipeline_lease = self.acquire_pipeline(&store)?;
            if job["snapshot"]["sourceVersion"] != store.source()?["version"] {
                return Err("图片流程原文版本已变化，请从当前原文明确重新启动图片阶段".into());
            }
            let mut resumed = job["snapshot"]["request"].clone();
            resumed["action"] = json!("images.produce");
            resumed["imageResumeOrigin"] = json!({"version":"image-production-resume/v1","workflowId":id,"snapshotSha256":pipeline::digest(&job["snapshot"].to_string())});
            store.event(id,"image_production_resume_requested",&json!({"version":"image-production-resume/v1","sourceVersion":job["snapshot"]["sourceVersion"],"originalStatus":job["status"],"snapshotSha256":pipeline::digest(&job["snapshot"].to_string()),"preserveOriginalRecord":true}))?;
            return self.reserve_asset_stage_owned(store, resumed, pipeline_lease);
        }
        if s(&job, "kind") != "workflow"
            || !matches!(
                s(&job, "status"),
                "failed" | "interrupted" | "waiting_decision" | "partially_blocked" | "blocked"
            )
        {
            return Err("请选择已中断或等待处理的制作流程".into());
        }
        let snapshot = job["snapshot"].clone();
        if snapshot["runnerVersion"] != "desktop-runner/v1" {
            return Err("此记录来自旧执行方式，请保留历史并重新启动阶段".into());
        }
        // A busy project is not a failed attempt. Acquire before resume events
        // or decisions change, retaining the same frozen workflow and budget.
        let pipeline_lease = self.acquire_pipeline(&store)?;
        refinement_identity::prepare_resume(&store,id)?;
        let rule_migration =
            workflow_resume_rule_audit(&job, &production_rules_for_snapshot(&snapshot)?)?;
        if let Some(authorization)=request.get("supplyContractCorrection") {
            SavedTextExecutor{store:store.clone(),settings:snapshot["input"]["settings"].clone(),workflow_id:Some(id.to_owned())}
                .authorize_series_supply_correction(authorization)?;
        }
        if let Some(authorization)=request.get("controllerReviewSupplement") {
            SavedTextExecutor{store:store.clone(),settings:snapshot["input"]["settings"].clone(),workflow_id:Some(id.to_owned())}
                .authorize_series_controller_review(authorization)?;
        } else if s(request,"action")=="workflows.resume" && snapshot["input"]["workflowProfile"]==crate::asset_graph::PROFILE {
            SavedTextExecutor{store:store.clone(),settings:snapshot["input"]["settings"].clone(),workflow_id:Some(id.to_owned())}
                .continue_completed_controller_review()?;
        }
        if s(request, "action") == "storyboard.decision.resolve" {
            let waiting = &job["result"]["waitingDecision"];
            let episode = s(waiting, "episodeId");
            if episode.is_empty() {
                return Err("当前流程没有等待选择的方案".into());
            }
            let selected = request["selectedIds"]
                .as_array()
                .ok_or("请选择方案，或明确使用空选择")?;
            let allowed: HashSet<_> = waiting["candidates"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| s(v, "id"))
                .collect();
            if selected
                .iter()
                .any(|id| id.as_str().is_none_or(|id| !allowed.contains(id)))
            {
                return Err("选择包含未提供的创作方案".into());
            }
            let mut decisions = job["prepared"]["decisions"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            decisions.insert(episode.to_owned(), json!({"selectedFantasyIds":selected}));
            let mut prepared = job["prepared"].as_object().cloned().unwrap_or_default();
            prepared.insert("decisions".into(), json!(decisions));
            store.update_job(id, s(&job, "status"), &json!({"prepared":prepared}))?;
        }
        if let Some(audit) = rule_migration {
            // A transport correction never rewrites the frozen workflow, source,
            // or accepted checkpoints. Current requests still load current rules.
            store.event(id, "foundation_transport_rule_migration", &audit)?;
        }
        if s(request, "action") == "workflows.resume" {
            store.event(id, "text_workflow_resume_requested", &json!({
                "version":"text-workflow-resume/v1",
                "snapshotSha256":pipeline::digest(&snapshot.to_string()),
                "previousStatus":job["status"],
                "maximumQuotaAttempts":1
            }))?;
        }
        self.execute_production_owned(store, id, snapshot, pipeline_lease)
    }
    fn execute_production_owned(
        self: &Arc<Self>,
        store: Store,
        run: &str,
        snapshot: Value,
        lease: ActiveGuard,
    ) -> Result<()> {
        let run = run.to_owned();
        Self::spawn_owned(lease,move||{
            let runtime=tokio::runtime::Runtime::new().map_err(|e|e.to_string())?;
            let result:Result<Value>=runtime.block_on(async {
                let stage=s(&snapshot,"stage");let mut input=snapshot["input"].clone();
                store.refresh_storyboard_audit_guard(&run,&snapshot)?;
                let record=store.job(&run)?;
                pipeline::storyboard_refinement_identity::attach_recovery(&mut input,&snapshot,&record["prepared"])?;
                refinement_identity::verify(&store,&input)?;
                let guard=record["prepared"].get("publicationGuard").unwrap_or(&snapshot["guard"]).clone();
                {let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;crate::workflow::verify_guard(&tx,&guard)?;}
                input["decisions"]=record["prepared"]["decisions"].clone();
                store.update_job(&run,"running",&json!({}))?;
                let settings=input["settings"].clone();
                let exec=SavedTextExecutor{store:store.clone(),settings:settings.clone(),workflow_id:Some(run.clone())};
                let model=s(&settings,"textModel");let reasoning=s(&settings,"reasoning");
                if stage=="foundation.run" {
                    let source:pipeline::LockedSource=serde_json::from_value(input["source"].clone()).map_err(|e|e.to_string())?;
                    let mut context=json!({"foundation":input["foundation"],"assets":input["assets"],"selection":input["selection"],"foundationModelInputProfile":input["foundationModelInputProfile"],"foundationPhase":input["foundationPhase"],"confirmedDecisions":input["confirmedDecisions"],"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundationDecisionPolicy":input["foundationDecisionPolicy"],"automaticDecisions":input["automaticDecisions"],"userReviewRequirements":input["userReviewRequirements"],"settings":{"confirmedDirection":input["foundation"]["productionDirection"]}});
                    pipeline::asset_coverage::inherit(&mut context, &input)?;
                    let (mut proposal,applied)=if input["foundationPhase"]=="APPLY_DECISION" {
                        if snapshot["foundationMethod"]!=pipeline::foundation_delivery::METHOD_VERSION {return Err("制作选择采用流程版本无效，未重新提取F2".into())}
                        pipeline::foundation_delivery::run_foundation_choice_application(&source,&context,&exec,model,reasoning).await?
                    }else if input["foundationPhase"]=="AUTO_INTERPRET" {
                        pipeline::foundation_delivery::run_foundation_auto_interpretation(&source,&context,&exec,model,reasoning).await?
                    }else{let proposal=match snapshot["foundationMethod"].as_str() {
                        Some("foundation-delivery/v2")=>pipeline::foundation_delivery::run_foundation_delivery(&source,&context,&exec,model,reasoning).await?,
                        None=>pipeline::run_foundation(&source,&context,&exec,model,reasoning).await?,
                        _=>return Err("制作基础执行方法版本不受支持，已保留原记录".into())
                    };let applied=pipeline::foundation_delivery::foundation_auto_applied_decisions(&proposal);(proposal,applied)};
                    let revised=if snapshot["foundationMethod"]=="foundation-delivery/v2" {
                        pipeline::execute_foundation_scene_edits(&source,&mut proposal)?.map(serde_json::to_value).transpose().map_err(|e|e.to_string())?
                    } else {None};
                    let mut assets=pipeline::flatten_foundation_assets(&proposal);
                    let mut output=serde_json::to_value(proposal).map_err(|e|e.to_string())?;
                    let mut decisions=input["foundation"]["authorDecisions"].as_array().into_iter().flatten().filter(|d|d["sourceSha256"]==output["sourceSha256"]).cloned().collect::<Vec<_>>();
                    for decision in applied { decisions.retain(|d|d["id"]!=decision["id"]); decisions.push(decision); }
                    if !decisions.is_empty() {
                        output["authorDecisions"]=json!(decisions);
                        for asset in &mut assets {
                            let local:Vec<_>=decisions.iter().filter(|d|d["subjectAssetKeys"].as_array().into_iter().flatten().any(|id|id==&asset["identityId"])).cloned().collect();
                            if !local.is_empty() {asset["authorDecisions"]=json!(local);}
                        }
                    }
                    store.publish_foundation_with_source(&guard,output,assets,Some(&run),revised)
                }else if stage=="storyboard.refine" {
                    pipeline::run_storyboard_refinement(&input,&exec,model,reasoning).await
                }else{
                    let history={
                        let connection=store.conn()?;
                        let mut statement=connection.prepare("SELECT w.stage,w.state,json_extract(j.snapshot,'$.input.episodeId'),json_extract(j.snapshot,'$.input.finishMode'),json_extract(j.snapshot,'$.input.executionBatch.version'),json_extract(w.result,'$.schemaVersion'),w.request_hash FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 ORDER BY w.rowid").map_err(|e|e.to_string())?;
                        let rows=statement.query_map([&run],|row|Ok(json!({"stage":row.get::<_,String>(0)?,"status":row.get::<_,String>(1)?,"episodeId":row.get::<_,Option<String>>(2)?,"finishMode":row.get::<_,Option<String>>(3)?,"executionBatchVersion":row.get::<_,Option<String>>(4)?,"outputSchemaVersion":row.get::<_,Option<String>>(5)?,"requestHash":row.get::<_,String>(6)?}))).map_err(|e|e.to_string())?;
                        rows.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?
                    };
                    input["storyboardHistory"]=json!(history);
                    // Runtime proof context is the original frozen parent input,
                    // not the input augmented with decisions or resume history.
                    input["frozenFinisherAssetContext"]=record["snapshot"]["input"].clone();
                    let mut output=pipeline::run_storyboard(&input,&exec,model,reasoning).await?;
                    output["planningProfile"]=input["capabilities"].clone();output["selection"]=input["selection"].clone();
                    if !pipeline::series::uses_profile(&input)? && output["units"].as_array().is_some_and(|u|!u.is_empty()) {
                        let latest=store.job(&run)?;
                        let publication_guard=latest["prepared"].get("publicationGuard").unwrap_or(&snapshot["guard"]).clone();
                        store.publish_storyboard(&publication_guard,&output,&input,Some(&run))?;
                    }
                    Ok(output)
                }
            });
            match result {
                Ok(value)=>{let status=match s(&value,"status"){"waiting_decision"=>"waiting_decision","partially_blocked"=>"partially_blocked",_=>"completed"};store.update_job(&run,status,&json!({"result":value}))},
                Err(error)=>store.update_job(&run,"failed",&json!({"error":error}))
            }
        })
    }
    fn spawn_stage(self: &Arc<Self>, store: Store, r: Value) -> Result<()> {
        let stage = s(&r, "action").to_owned();
        if stage == "foundation.run" || stage == "storyboard.run" || stage == "storyboard.refine" {
            return self.spawn_production(store, r);
        }
        let pipeline_lease = self.acquire_pipeline(&store)?;
        self.reserve_asset_stage_owned(store, r, pipeline_lease)
    }
    fn reserve_asset_stage_owned(self: &Arc<Self>, store: Store, mut r: Value, pipeline_lease: ActiveGuard) -> Result<()> {
        crate::text_executor::check_cli_unknown_hold(&store)?;
        if r.get("imageResumeOrigin").is_none() {
            return self.reserve_image_workflow_v1(store,r,pipeline_lease);
        }
        let stage = s(&r, "action").to_owned();
        let all_assets = store.values("assets")?;
        let (wanted, _) = image_closure(&store, &all_assets, r["assetIds"].as_array());
        let (saved_settings, original_context) = if let Some(origin) = r.get("imageResumeOrigin") {
            let old = store.job(s(origin, "workflowId"))?;
            if origin["version"] != "image-production-resume/v1"
                || !image_production_resume_eligible(&old)
                || origin["snapshotSha256"] != pipeline::digest(&old["snapshot"].to_string())
                || old["snapshot"]["sourceVersion"] != store.source()?["version"]
            {
                return Err("图片续跑原流程/范围绑定已变化，未重新提取或生成".into());
            }
            let mut expected = old["snapshot"]["request"].clone();
            let mut actual = r.clone();
            actual
                .as_object_mut()
                .ok_or("图片续跑请求结构无效")?
                .remove("imageResumeOrigin");
            expected["action"] = json!("images.produce");
            if actual != expected {
                return Err("图片续跑不能修改冻结选集、资产或渠道参数".into());
            }
            (old["snapshot"]["settings"].clone(),old["snapshot"]["stage2Context"].clone())
        } else {
            (self.effective_settings(&store)?,Value::Null)
        };
        let mut saved_settings=saved_settings;
        crate::app_settings::freeze_credentials(&mut saved_settings);
        let policy = pipeline::image_production_policy(&saved_settings);
        r["imageProductionPolicy"] = json!(policy);
        let active_image_ids: Vec<_> = store
            .job_summaries()?
            .iter()
            .filter(|j| s(j, "kind") == "image" && is_active_generation(s(j, "status")))
            .map(|j| j["entityId"].clone())
            .collect();
        let mut design_input = json!({"source":store.source()?,"foundation":store.foundation()?,"assets":all_assets,"settings":saved_settings,"activeImageAssetIds":active_image_ids,"selection":{"assetIds":wanted}});
        if !original_context.is_object() {
            let projection = store.foundation_context_for_new_workflow()?;
            design_input["foundation"] = projection.foundation;
            design_input["foundationReasonProjection"] = projection.diagnostic;
            if r.get("imageResumeOrigin").is_none() {
                store.freeze_visual_fact_source_reparse(&mut design_input)?;
            }
        }
        design_input["designRuleSha256"] = if original_context.is_object() {
            original_context["designRuleSha256"].clone()
        } else {
            pipeline::rule_snapshot("asset-art-director")?["sha256"].clone()
        };
        if original_context.is_object() {
            let old_job=store.job(s(&r["imageResumeOrigin"],"workflowId"))?;
            pipeline::asset_design_contract::validate_resume_facts(&original_context,&old_job["result"],&design_input)?;
            for key in &wanted {
                let old = original_context["assets"].as_array().into_iter().flatten().find(|a|s(a,"id")==key).ok_or("续跑原冻结资产不存在")?;
                let current = all_assets.iter().find(|a|s(a,"id")==key).ok_or("续跑资产不存在")?;
                if pipeline::asset_design_contract::asset_semantics(old) != pipeline::asset_design_contract::asset_semantics(current) {
                    return Err("图片续跑的资产语义版本已变化，请发起新制作；原请求未改写".into());
                }
                let original_assets = original_context["assets"].as_array().ok_or("续跑原冻结资产目录缺失")?;
                let topology = |chain: Vec<Value>| chain.iter().map(|a|json!({"assetId":a["assetId"],"semanticVersion":a["semanticVersion"]})).collect::<Vec<_>>();
                if topology(pipeline::asset_design_contract::source_chain(original_assets,old)?) != topology(pipeline::asset_design_contract::source_chain(&all_assets,current)?) {
                    return Err("图片续跑的指定来源语义版本已变化，请发起新制作".into());
                }
            }
            design_input["comparisonDesignsByAssetId"] = original_context["comparisonDesignsByAssetId"].clone();
            if let Some(profile) = original_context.get(pipeline::asset_design_contract::COMPARISON_PROFILE_FIELD) {
                design_input[pipeline::asset_design_contract::COMPARISON_PROFILE_FIELD] = profile.clone();
            }
            // The current immutable fact receipts carry repair budgets. Direction and
            // comparisons remain those captured by the original workflow.
            design_input["foundation"]["directionConfirmed"] = original_context["foundation"]["directionConfirmed"].clone();
            design_input["foundation"]["productionDirection"] = original_context["foundation"]["productionDirection"].clone();
            for key in [pipeline::asset_design_contract::IDENTITY_MATERIAL_PROFILE_FIELD, pipeline::asset_design_contract::IDENTITY_MATERIALS_FIELD, pipeline::asset_design_contract::SCOPE_MATERIALS_FIELD, pipeline::asset_production_context::PROFILE_FIELD, pipeline::asset_design_contract::INLINE_DELTA_PROFILE_FIELD, pipeline::DESIGN_EVIDENCE_PROFILE_FIELD] {
                if let Some(value) = original_context.get(key) { design_input[key] = value.clone(); }
            }
            if let Some(value) = original_context.get(pipeline::asset_visual_source_reparse::FIELD) {
                design_input[pipeline::asset_visual_source_reparse::FIELD] = value.clone();
            }
        } else {
            design_input["selection"]["explicitAssetIds"] = json!(r["assetIds"].as_array().cloned()
                .unwrap_or_else(|| all_assets.iter().filter(|asset| independent_image(asset)).map(|asset| asset["id"].clone()).collect()));
            pipeline::asset_design_contract::freeze_identity_production_materials(&mut design_input)?;
            pipeline::asset_production_context::freeze(&mut design_input);
            pipeline::asset_design_contract::freeze_inline_delta_profile(&mut design_input);
            let mut comparisons = json!({});
            for asset in &all_assets { comparisons[s(asset,"id")] = json!(pipeline::asset_design_contract::comparison_designs(&design_input,asset)); }
            design_input["comparisonDesignsByAssetId"] = comparisons;
            // A missing legacy stage2Context needs reconstruction, but does not
            // make its explicit resume a new workflow policy. Only genuinely
            // new productions opt in; resumes retain their original comparison
            // contract (including absence of a profile).
            if r.get("imageResumeOrigin").is_none() {
                pipeline::freeze_design_evidence_profile(&mut design_input);
                design_input[pipeline::asset_design_contract::COMPARISON_PROFILE_FIELD] = json!(pipeline::asset_design_contract::COMPARISON_PROFILE);
            }
        }
        let needs_visual_check =
            pipeline::asset_visual_facts::cached_visual_check_required(&design_input)?;
        let needs_design = stage == "images.design"
            || needs_visual_check
            || all_assets.iter().any(|asset| {
                wanted.iter().any(|id| id == s(asset, "id"))
                    && pipeline::automatic_design_required(&design_input, asset)
            });
        if needs_design {
            image_design_source(&store)?;
        }
        let execution_settings = if needs_design {
            let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
            runtime.block_on(resolve_text_execution_settings(&saved_settings))?
        } else {
            saved_settings
        };
        let version = store.source()?["version"].clone();
        design_input["settings"] = execution_settings.clone();
        let native_receipt=self.reserve_native_workflow(&r,&design_input["source"],&execution_settings)?;
        let stage2_context = design_input;
        let snapshot = json!({"runnerVersion":"stage2-runner/v1","stage":stage,"sourceVersion":version,"settings":execution_settings,"request":r,"imageProductionPolicy":policy,"stage2Context":stage2_context,"nativeAcceptance":native_receipt});
        let job = store.add_job(
            "workflow",
            &format!("{stage}:{}", s(&r, "episodeId")),
            "local",
            "local",
            &snapshot,
            "queued",
        )?;
        self.bind_native_workflow(&store,&native_receipt,&job)?;
        self.execute_asset_stage_owned(store,job,snapshot,pipeline_lease)
    }
    fn execute_asset_stage_owned(self:&Arc<Self>,store:Store,job:String,snapshot:Value,lease:ActiveGuard)->Result<()> {
        if snapshot["stage2Context"]["workflowProfile"]==crate::asset_graph::PROFILE {
            return self.execute_image_workflow_v1(store,job,snapshot,lease);
        }
        let stage=s(&snapshot,"stage").to_string();
        let r=snapshot["request"].clone();
        let execution_settings=snapshot["settings"].clone();
        let mut stage2_context=snapshot["stage2Context"].clone();
        let version=snapshot["sourceVersion"].clone();
        let engine=self.clone();
        store.ensure_workflow_storage()?;
        if pipeline::asset_design_contract::incremental_comparisons(&stage2_context)? {
            stage2_context[pipeline::asset_design_contract::FROZEN_COMPARISON_REQUESTS] =
                pipeline::asset_design_contract::frozen_comparison_requests(&store,&job)?;
        }
        Self::spawn_owned(lease,move||{
            let runtime=tokio::runtime::Runtime::new().map_err(|e|e.to_string())?;
            let result:Result<Value>=runtime.block_on(async{
                store.update_job(&job,"running",&json!({}))?;let settings=execution_settings;let exec=SavedTextExecutor{store:store.clone(),settings:settings.clone(),workflow_id:Some(job.clone())};
                let model=s(&settings,"textModel");let reasoning=s(&settings,"reasoning");
                match stage.as_str(){
                    "images.design"=>{image_design_source(&store)?;let basis_version=stage2_context["foundation"]["version"].as_i64().unwrap_or(0);let input=stage2_context.clone();let output=pipeline::run_asset_design(&input,&exec,model,reasoning).await?;store.finish_asset_design_workflow(version.as_i64().ok_or("原文版本缺失")?,basis_version,&output,&job)},
                    "images.produce"=>{
                        let mut all=stage2_context["assets"].as_array().cloned().ok_or("缺少冻结资产上下文")?;
                        for asset in &mut all {
                            if asset.pointer("/preparation/mode").and_then(Value::as_str)==Some("INLINE") {asset["adoptedMediaId"]=json!(store.adopted(s(asset,"id"),s(asset,"variantId"))?);}
                        }
                        let (wanted,dependency_errors)=image_closure(&store,&all,r["assetIds"].as_array());
                        let basis=stage2_context["foundation"].clone();
                        let basis_version=basis["version"].as_i64().unwrap_or(0);
                        let policy=r["imageProductionPolicy"].as_str().filter(|p|["automatic","trialReview"].contains(p)).unwrap_or_else(||pipeline::image_production_policy(&settings));
                        let active_image_ids:Vec<_>=store.job_summaries()?.iter().filter(|j|s(j,"kind")=="image"&&is_active_generation(s(j,"status"))).map(|j|j["entityId"].clone()).collect();
                        let input=image_production_design_input(&stage2_context,&all,&settings,policy,&active_image_ids,&wanted);
                        let needs_design=all.iter().any(|a|wanted.iter().any(|id|id==s(a,"id"))&&pipeline::automatic_design_required(&input,a));
                        let needs_visual_check=pipeline::asset_visual_facts::cached_visual_check_required(&input)?;
                        let needs_asset_step=needs_design||needs_visual_check;
                        let mut output=if needs_asset_step {
                            image_design_source(&store)?;
                            pipeline::run_asset_design(&input,&exec,model,reasoning).await?
                        } else {json!({"assets":[],"blocked":[],"imagePlan":[],"imageProductionPolicy":policy})};
                        if let Some(blocked)=output["blocked"].as_array_mut(){blocked.extend(dependency_errors);}
                        if store.source()?["version"]!=version{return Err("原文已更新；设计结果保留，不覆盖当前资产".into())}
                        if needs_asset_step{store.publish_asset_design(version.as_i64().ok_or("原文版本缺失")?,basis_version,&output)?;}
                        let approved=settings["trialApproved"].as_bool()==Some(true);let trial=r["trial"].as_bool()==Some(true);
                        if policy=="trialReview"&&!approved&&!trial{output["status"]=json!("awaitingTrial");return Ok(output)}
                        let mut generation=r.clone();generation["action"]=json!("images.run");generation["imageProductionPolicy"]=json!(policy);
                        let rows=store.values("assets")?;
                        let (wanted,errs)=image_closure(&store,&rows,r["assetIds"].as_array());
                        if let Some(blocked)=output["blocked"].as_array_mut(){blocked.extend(errs);}
                        let failed:HashSet<String>=output["blocked"].as_array().into_iter().flatten().filter_map(|a|a["assetId"].as_str().map(str::to_owned)).collect();
                        let mut refreshed:HashSet<String>=output["assets"].as_array().into_iter().flatten().filter_map(|a|a["id"].as_str().map(str::to_owned)).collect();
                        for key in &wanted {
                            let a=rows.iter().find(|a|s(a,"id")==key).ok_or("图片闭包中的资产已经不存在")?;
                            let inherited=image_dependency_ids(&rows,a)?.iter().any(|id|rows.iter().find(|a|s(a,"id")==id).and_then(|a|image_reference_chain(&rows,a).ok()).is_some_and(|chain|chain.iter().any(|a|refreshed.contains(s(a,"id")))));
                            if inherited {refreshed.insert(key.clone());}
                        }
                        let mut ids=Vec::new();
                        for key in &wanted {
                            let already_active=active_image_ids.iter().any(|id|id.as_str()==Some(key));
                            if failed.contains(key.as_str())&&!already_active {continue;}
                            let a=rows.iter().find(|a|s(a,"id")==key).ok_or("图片资产已经不存在")?;
                            if already_active||refreshed.contains(key)||store.adopted(key,s(a,"variantId"))?.is_none(){ids.push(json!(key));}
                        }
                        generation["assetIds"]=json!(ids);
                        let report=engine.queue_images_frozen(&store,&generation,&settings,|config,account|engine.checked_account(config,account))?;
                        if let (Some(blocked),Some(queue_blocked))=(output["blocked"].as_array_mut(),report["blocked"].as_array()){blocked.extend(queue_blocked.iter().cloned());}
                        output["status"]=json!(if output["blocked"].as_array().is_some_and(|blocked|!blocked.is_empty()){"partially_blocked"}else if report["queuedCount"].as_u64().unwrap_or(0)>0{"queuedForGeneration"}else if report["reusedCount"].as_u64().unwrap_or(0)>0{"processingExisting"}else{"noGenerationNeeded"});
                        output["imageProductionPolicy"]=json!(policy);
                        output["imagePlan"]=json!(wanted.iter().filter_map(|key|rows.iter().find(|a|s(a,"id")==key)).map(|a|{
                            let queued=report["queued"].as_array().into_iter().flatten().chain(report["reused"].as_array().into_iter().flatten()).find(|q|q["assetId"]==a["id"]);
                            let status=if failed.contains(s(a,"id")){"blocked"}else{queued.map(|q|s(q,"status")).unwrap_or("ready")};
                            json!({"assetId":a["id"],"variantId":a["variantId"],"dependencies":image_dependency_ids(&rows,a).unwrap_or_default(),"imageProductionPolicy":policy,"jobId":queued.and_then(|q|q.get("jobId")),"status":if status=="waiting_dependency"{"waitingAdoption"}else{status},"prompt":a["prompt"],"candidateCount":1})
                        }).collect::<Vec<_>>());
                        output["imageQueueReport"]=report;
                        Ok(output)
                    },
                    _=>Err("未知阶段".into())
                }
            });match result{Ok(v)=>if stage=="images.design"{Ok(())}else{store.update_job(&job,image_stage_completion_state(&v),&json!({"result":v}))},Err(e)=>store.update_job(&job,"failed",&json!({"error":e}))}
        })
    }
    fn restore_backup(&self, store: &Store, path: &Path) -> Result<Value> {
        let source = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let backup_dir = fs::canonicalize(store.root.join("backups")).map_err(|e| e.to_string())?;
        if !source.starts_with(&backup_dir) {
            return Err("请选择本项目的状态快照".into());
        }
        let new_id = id();
        let root = self.home.join("projects").join(&new_id);
        fs::create_dir_all(root.join("media")).map_err(|e| e.to_string())?;
        fs::copy(&source, root.join("project.sqlite")).map_err(|e| e.to_string())?;
        let restored = Store::open(root)?;
        // UI media projection creates thumbnails and reads the original file.
        // A fresh restore has neither yet: enumerate immutable snapshot metadata,
        // copy and verify every original, then relocate/project the completed copy.
        let media = {
            let c = restored.conn()?;
            let mut query = c.prepare("SELECT data FROM media ORDER BY rowid").map_err(|e| e.to_string())?;
            let values = query.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?
                .map(|row| crate::store::parse(row.map_err(|e| e.to_string())?))
                .collect::<Result<Vec<_>>>()?;
            values
        };
        for m in media {
            let src = store.verify_media(&m)?;
            let dst = restored.absolute_media(&m)?;
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            fs::copy(src, dst).map_err(|e| e.to_string())?;
            restored.verify_media(&m)?;
        }
        restored
            .conn()?
            .execute(
                "UPDATE project SET id=?1,title=title||'（恢复副本）'",
                [new_id],
            )
            .map_err(|e| e.to_string())?;
        portable::relocate(&restored)?;
        restored.recover_interrupted()?;
        self.register(&restored)?;
        self.workspace(&restored, None)
    }
}
/// Preserve the compact workflow identity used by Stage1/Stage3 start hints;
/// never expose frozen bodies, rule texts, settings or transport payloads here.
fn workspace_job_summary(mut job: Value, auto_submit: Option<bool>) -> Value {
    let snap = job["snapshot"].clone();
    let prepared = job["prepared"].clone();
    job["snapshot"] = if job["kind"] == "workflow" {
        json!({"runnerVersion":snap["runnerVersion"],"stage":snap["stage"],
            "sourceVersion":snap["sourceVersion"],"selection":snap["selection"],
            "input":{"source":{"version":snap["input"]["source"]["version"],"sha256":snap["input"]["source"]["sha256"]}}})
    } else {
        json!({"model":snap["model"],"unitVersion":snap["unitVersion"],"problems":snap["problems"],"autoSubmit":auto_submit})
    };
    job["prepared"] = json!({"estimatedCost":prepared["estimatedCost"]});
    if let Some(object) = job.as_object_mut() { object.remove("receipt"); }
    job
}

#[cfg(test)]
mod workspace_job_summary_tests {
    use super::*;

    #[test]
    fn actual_store_workflow_summaries_keep_start_hint_binding_through_workspace_projection() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store.create("offline workspace summary","synthetic").unwrap();
        let source = store.save_source(json!(pipeline::prepare_source("第一集\n场景：门口\n甲：开门。\n第二集\n场景：院内\n乙：回头。\n").unwrap())).unwrap();
        let selection = json!({"episodeIds":source["episodes"].as_array().unwrap().iter().map(|episode|&episode["id"]).collect::<Vec<_>>()});
        for stage in ["foundation.run","storyboard.run"] {
            let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":stage,"sourceVersion":source["version"],"selection":selection,
                "input":{"source":source,"selection":selection,"settings":{"textApiKey":"SYNTHETIC_PRIVATE_SETTING"}},
                "rules":[{"content":"SYNTHETIC_PRIVATE_RULE"}]});
            let key = store.add_job("workflow",stage,"local","local",&snapshot,"failed").unwrap();
            let stored = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==key).unwrap();
            let summary = workspace_job_summary(stored,None);
            assert_eq!(summary["snapshot"],json!({"runnerVersion":"desktop-runner/v1","stage":stage,
                "sourceVersion":source["version"],"selection":selection,
                "input":{"source":{"version":source["version"],"sha256":source["sha256"]}}}));
            assert_eq!(summary["status"],"failed");
            assert!(!summary.to_string().contains("SYNTHETIC_PRIVATE_"));
            assert!(summary["snapshot"]["input"]["source"].get("text").is_none());
            assert!(summary["snapshot"]["input"]["source"].get("rows").is_none());
            assert!(summary["snapshot"].get("rules").is_none());
            assert!(summary["snapshot"]["input"].get("settings").is_none());
            assert_eq!(store.job(&key).unwrap()["snapshot"],snapshot);
        }
    }

    #[test]
    fn actual_child_unknown_survives_workspace_projection_while_known_terminal_children_do_not() {
        let temp = tempfile::tempdir().unwrap();let store = Store::open(temp.path().join("project")).unwrap();store.create("offline child unknown summary","synthetic").unwrap();store.ensure_workflow_storage().unwrap();
        for stage in ["foundation.run","storyboard.run"] {
            let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":stage,"sourceVersion":1,"selection":{"episodeIds":["e1"]},
                "input":{"source":{"version":1,"sha256":"a".repeat(64),"text":"SYNTHETIC_PRIVATE_SOURCE"},"settings":{"textApiKey":"SYNTHETIC_PRIVATE_SETTING"}}});
            let parent = store.add_job("workflow",stage,"local","local",&snapshot,"completed").unwrap();
            let child = store.add_job("text","frozen-node","codex_sdk","local",&json!({}),"submission_unknown").unwrap();
            store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,updated_at) VALUES(?1,'frozen-node',?2,'synthetic','failed',?3,?4)",rusqlite::params![parent,stage,child,now()]).unwrap();
            let before = store.job(&parent).unwrap();
            let stored = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==parent).unwrap();
            let summary = workspace_job_summary(stored,None);
            assert_eq!(summary["status"],"completed");assert_eq!(summary["submissionUnknown"],true);
            assert!(!summary.to_string().contains("SYNTHETIC_PRIVATE_"));assert_eq!(store.job(&parent).unwrap(),before);
            for state in ["failed","rejected","completed"] {
                store.update_job(&child,state,&json!({})).unwrap();
                let stored = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==parent).unwrap();
                assert_eq!(workspace_job_summary(stored,None)["submissionUnknown"],false,"known terminal state {state} does not inherit an unknown flag");
                assert_eq!(store.job(&parent).unwrap(),before);
            }
        }
    }

    #[test]
    fn non_workflow_summary_keeps_media_metadata_and_omits_transport_and_private_bodies() {
        let temp = tempfile::tempdir().unwrap();let store = Store::open(temp.path().join("project")).unwrap();store.create("offline media summary","synthetic").unwrap();
        let key = store.add_job("video","u1","mock","local",&json!({"model":"model","unitVersion":3,"problems":[],"autoSubmit":true,
            "runnerVersion":"SYNTHETIC_PRIVATE_RUNNER","input":{"source":{"text":"SYNTHETIC_PRIVATE_SOURCE"}},"prompt":"SYNTHETIC_PRIVATE_PROMPT"}),"ready").unwrap();
        store.update_job(&key,"ready",&json!({"prepared":{"estimatedCost":2,"wire":{"payload":"SYNTHETIC_PRIVATE_WIRE"}},"receipt":{"private":"SYNTHETIC_PRIVATE_RECEIPT"}})).unwrap();
        let stored = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==key).unwrap();
        let summary = workspace_job_summary(stored,Some(true));
        assert_eq!(summary["snapshot"],json!({"model":"model","unitVersion":3,"problems":[],"autoSubmit":true}));
        assert_eq!(summary["prepared"],json!({"estimatedCost":2}));
        assert!(summary.get("receipt").is_none());
        assert!(!summary.to_string().contains("SYNTHETIC_PRIVATE_"));
    }
}

struct ActiveGuard {
    engine: Arc<Engine>,
    keys: Vec<String>,
}
fn project_archived(project: &Value) -> bool {
    !s(project, "archivedAt").is_empty()
}
fn lifecycle_version(project: &Value) -> Result<u64> {
    match project.get("lifecycleVersion") {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| "项目生命周期版本损坏".into()),
    }
}
fn registered_store(project: &Value) -> Result<Store> {
    let root = PathBuf::from(s(project, "rootPath"));
    let database = root.join("project.sqlite");
    if s(project, "rootPath").is_empty() || !database.is_file() {
        return Err("项目文件不存在；请保留项目记录并核对项目目录".into());
    }
    // Validate identity read-only before Store::open can initialize/migrate anything.
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| e.to_string())?;
    let actual_id: String = connection
        .query_row("SELECT id FROM project LIMIT 1", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if actual_id != s(project, "id") {
        return Err("项目目录与登记身份不一致，未打开或改写项目".into());
    }
    drop(connection);
    Store::open(root)
}
fn project_worker_keys(store: &Store, mut keys: Vec<String>) -> Result<Vec<String>> {
    keys.push(format!("project:{}:{}", s(&store.project()?, "id"), id()));
    Ok(keys)
}
fn image_design_source(store: &Store) -> Result<pipeline::LockedSource> {
    let raw = store.source()?;
    if raw.is_null() || s(&raw, "text").is_empty() {
        return Err("本项目尚未导入剧本原文。请先到第一阶段导入原文，再设计资产提示词；已有手写提示词的资产可以直接生成图片。".into());
    }
    let source = serde_json::from_value(raw)
        .map_err(|e| format!("剧本原文记录不完整，请重新导入或核对：{e}"))?;
    pipeline::validate_source(&source)?;
    Ok(source)
}
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.engine.active.lock() {
            for key in &self.keys {
                active.remove(key);
            }
        }
        if self.keys.iter().any(|key| key.starts_with("image:")) {
            self.engine.wake_images();
        }
    }
}
// Account mutation and legacy exclusive operations still exclude all image
// workers. Different image lanes can run concurrently; a job can have one owner.
fn worker_key_conflicts(active: &HashSet<String>, key: &str) -> bool {
    active.contains(key)
        || key.strip_prefix("channel:").is_some_and(|provider| active.iter().any(|k| k.starts_with(&format!("image:{provider}:"))))
        || key.strip_prefix("image:").and_then(|rest| rest.split(':').next()).is_some_and(|provider| active.contains(&format!("channel:{provider}")))
}
fn is_active_generation(state: &str) -> bool {
    [
        "queued",
        "preparing",
        "ready",
        "submitting",
        "submitted",
        "processing",
        "downloading",
        "auth_required",
        "submission_unknown",
        "query_failed",
        "download_failed",
        "waiting_dependency",
        "recovery_paused",
        "succeeded",
    ]
    .contains(&state)
}
fn retains_image_generation(job: &Value) -> bool {
    is_active_generation(s(job, "status")) || s(job, "provider") == "chrome"
        && s(job, "status") == "failed" && !s(job, "providerTaskId").is_empty() && !crate::chrome_terminal::settled_job(job)
}
fn require_account(store: &Store, provider: &str) -> Result<()> {
    if ["dreamina", "bh", "chrome"].contains(&provider)
        && s(&store.settings()?, &format!("{provider}AccountId"))
            .trim()
            .is_empty()
    {
        return Err(
            "请为渠道配置稳定账户标签；更换账户时必须更改标签，不能把登录状态当作账户身份".into(),
        );
    }
    Ok(())
}
fn video_provider(store: &Store, request: &Value) -> Result<String> {
    let settings = store.settings()?;
    let provider = request
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or(s(&settings, "videoProvider"))
        .trim();
    if provider.is_empty() {
        Err("请选择视频渠道".into())
    } else {
        Ok(provider.to_owned())
    }
}
fn timestamp_fresh(value: &str, seconds: i64) -> bool {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .is_some_and(|time| {
            let age = chrono::Utc::now().signed_duration_since(time).num_seconds();
            (0..seconds).contains(&age)
        })
}
fn model_refresh_account(
    snapshot: Value,
    refresh_due: bool,
    refresh: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    if verified_dreamina_session(&snapshot, refresh_due) {
        Ok(snapshot)
    } else {
        refresh()
    }
}
fn verified_dreamina_session(snapshot: &Value, refresh_due: bool) -> bool {
    !refresh_due
        && s(snapshot, "status") == "authenticated"
        && snapshot["ready"] == true
        && snapshot["identityVerified"] == true
        && !s(snapshot, "accountKey").is_empty()
        && timestamp_fresh(s(snapshot, "checkedAt"), 60)
}
// Both ordinary preflight and the local-only entry use this same durable
// fingerprint/version/time predicate. No second capability cache is created.
#[derive(Debug, PartialEq)]
enum StoryboardStartBoundary {
    New,
    Resume(String),
    Hold(String),
}

fn storyboard_source_identity(value: &Value) -> Result<String> {
    let source: pipeline::LockedSource = serde_json::from_value(value.clone())
        .map_err(|_| "制作流程缺少可核验的锁定原文")?;
    let source = pipeline::upgrade_source_dialogues(&source)?;
    pipeline::validate_source(&source)?;
    // LockedSource excludes storage version/id. Equal text alone is not enough:
    // confirmed episode/scene boundaries and source rows must also agree.
    Ok(pipeline::digest(&serde_json::to_string(&source).map_err(|e| e.to_string())?))
}

fn storyboard_scope_ids(selection: &Value, field: &str) -> Result<HashSet<String>> {
    let Some(value) = selection.get(field).filter(|value| !value.is_null()) else {
        return Ok(HashSet::new());
    };
    let values = value.as_array().ok_or("制作流程的冻结范围不是数组")?;
    let mut ids = HashSet::new();
    for value in values {
        let id = value.as_str().filter(|id| !id.is_empty()).ok_or("制作流程范围缺少正式ID")?;
        if !ids.insert(id.to_owned()) { return Err("制作流程范围ID重复".into()) }
    }
    Ok(ids)
}

/// Pure start policy. Workflows arrive newest first; completed newer deliveries
/// may supersede stopped historical branches, never an unresolved submission.
fn storyboard_start_boundary(workflows: &[Value], input: &Value) -> Result<StoryboardStartBoundary> {
    let identity = storyboard_source_identity(&input["source"])?;
    let episodes = storyboard_scope_ids(&input["selection"], "episodeIds")?;
    let units = storyboard_scope_ids(&input["selection"], "unitIds")?;
    if episodes.is_empty() { return Err("请明确分镜制作集范围".into()) }
    let mut completed_episodes = HashSet::new();
    let mut completed_units = HashSet::new();
    let mut matches = Vec::new();
    for job in workflows {
        if job["kind"] != "workflow" || job["entityId"] != "storyboard.run" { continue; }
        let snapshot = &job["snapshot"];
        let original = &snapshot["input"];
        if original["source"]["sha256"] != input["source"]["sha256"] { continue; }
        let selection = original.get("selection").filter(|value|value.is_object()).unwrap_or(&snapshot["selection"]);
        let old_episodes = storyboard_scope_ids(selection, "episodeIds")?;
        let old_units = storyboard_scope_ids(selection, "unitIds")?;
        if old_episodes.is_empty() { return Err("原分镜流程的冻结集范围缺失，不能通过新流程绕过".into()) }
        if episodes.is_disjoint(&old_episodes) { continue; }
        // A parser/metadata upgrade does not settle an upstream submission.
        // Same raw source and overlapping frozen scope still belong to the old
        // attempt; decide this before the stronger interpretation identity.
        let unknown = job["submissionUnknown"].as_bool() == Some(true) || s(job,"status") == "submission_unknown";
        if unknown && (old_units.is_empty() || units.is_empty() || !units.is_disjoint(&old_units)) {
            return Ok(StoryboardStartBoundary::Hold(format!("原分镜流程 {} 存在未知提交；先核对原执行，不另建重叠范围或重发", s(job,"id"))));
        }
        let same_source = match storyboard_source_identity(&original["source"]) {
            Ok(old) => old == identity,
            Err(_) => {
                if original["source"]["sha256"] == input["source"]["sha256"] {
                    return Ok(StoryboardStartBoundary::Hold(format!("原分镜流程 {} 的源绑定不能核验，保留记录且不另建相同来源任务", s(job,"id"))));
                }
                false
            }
        };
        if !same_source { continue; }
        if s(job,"status") == "completed" && !unknown {
            if old_units.is_empty() { completed_episodes.extend(old_episodes); }
            else { completed_units.extend(old_units); }
            continue;
        }
        if ["cancelled", "closed_by_user"].contains(&s(job,"status")) && !unknown { continue; }
        let published = &job["prepared"]["publishedEpisodes"];
        let mut pending_episodes = old_episodes.clone();
        // A published projection proves content availability, not the remote
        // outcome of any unresolved text node. Unknown owns its original scope.
        if old_units.is_empty() && !unknown {
            pending_episodes.retain(|id| {
                let proof = &published[id];
                !(proof["version"] == "complete-episode-publication/v1"
                    && proof["episodeId"] == id.as_str()
                    && proof["sourceSha256"] == input["source"]["sha256"]
                    && proof["episodeDeliverySha256"].as_str().is_some_and(|sha| sha.len()==64)
                    && proof["unitIds"].as_array().is_some_and(|ids| !ids.is_empty()))
            });
        }
        if !unknown { pending_episodes.retain(|id| !completed_episodes.contains(id)); }
        let mut pending_units = old_units.clone();
        if !unknown { pending_units.retain(|id| !completed_units.contains(id)); }
        if !old_units.is_empty() && pending_units.is_empty() { continue; }
        let overlaps = !episodes.is_disjoint(&pending_episodes)
            && (old_units.is_empty() || units.is_empty() || !units.is_disjoint(&pending_units));
        if !overlaps { continue; }
        let id = s(job,"id");
        if unknown {
            return Ok(StoryboardStartBoundary::Hold(format!("原分镜流程 {id} 存在未知提交；先核对原执行，不另建重叠范围或重发")));
        }
        if old_episodes != episodes || old_units != units {
            return Ok(StoryboardStartBoundary::Hold(format!("所选范围与未完成分镜流程 {id} 重叠；请继续原流程，不通过缩小或扩大范围重发")));
        }
        if !units.is_empty() && original["revisionInstruction"] != input["revisionInstruction"] {
            return Ok(StoryboardStartBoundary::Hold(format!("单元范围已有未完成分镜流程 {id}，修改要求与原任务不同；先处理原流程，未忽略新要求或重发")));
        }
        let original_version=original["source"].get("version").filter(|value|!value.is_null()).unwrap_or(&snapshot["sourceVersion"]);
        if *original_version != input["source"]["version"] {
            return Ok(StoryboardStartBoundary::Hold(format!("原分镜流程 {id} 的原文内容与范围相同但保存版本已变化；保留原请求，先核对来源版本绑定")));
        }
        if snapshot["runnerVersion"] != "desktop-runner/v1"
            || !["failed","interrupted","waiting_decision","partially_blocked","blocked"].contains(&s(job,"status")) {
            return Ok(StoryboardStartBoundary::Hold(format!("原分镜流程 {id} 仍在执行或缺少可恢复身份，不另建同范围任务")));
        }
        matches.push(id.to_owned());
    }
    match matches.len() {
        0 => Ok(StoryboardStartBoundary::New),
        1 => Ok(StoryboardStartBoundary::Resume(matches.remove(0))),
        _ => Ok(StoryboardStartBoundary::Hold("相同来源与范围有多个未完成分镜流程，请从原记录选择继续；未新建或重发".into())),
    }
}

fn existing_storyboard_start(store: &Store, input: &Value) -> Result<StoryboardStartBoundary> {
    store.ensure_workflow_storage()?;
    let connection = store.conn()?;
    let mut query = connection.prepare(r#"SELECT j.id,j.state,
        json_object('runnerVersion',json_extract(j.snapshot,'$.runnerVersion'),
            'sourceVersion',json_extract(j.snapshot,'$.sourceVersion'),
            'selection',json_extract(j.snapshot,'$.selection'),
            'input',json_object('source',json_extract(j.snapshot,'$.input.source'),
                'selection',json_extract(j.snapshot,'$.input.selection'),
                'revisionInstruction',json_extract(j.snapshot,'$.input.revisionInstruction'))),
        COALESCE(json_extract(j.prepared,'$.publishedEpisodes'),'{}'),
        EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs t ON t.id=w.text_job_id
            WHERE w.workflow_id=j.id AND t.state='submission_unknown') OR j.state='submission_unknown'
        FROM jobs j WHERE j.kind='workflow' AND j.entity_id='storyboard.run'
            AND json_extract(j.snapshot,'$.input.source.sha256')=?1
        ORDER BY j.created_at DESC,j.rowid DESC"#).map_err(|e|e.to_string())?;
    let records = query.query_map([s(&input["source"],"sha256")],|row| {
        Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,
            row.get::<_,String>(3)?,row.get::<_,bool>(4)?))
    }).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let workflows: Vec<Value> = records.into_iter().map(|(id,status,snapshot,published,unknown)| {
        Ok(json!({"id":id,"kind":"workflow","entityId":"storyboard.run","status":status,
            "snapshot":serde_json::from_str::<Value>(&snapshot).map_err(|e|e.to_string())?,
            "prepared":{"publishedEpisodes":serde_json::from_str::<Value>(&published).map_err(|e|e.to_string())?},
            "submissionUnknown":unknown}))
    }).collect::<Result<_>>()?;
    storyboard_start_boundary(&workflows,input)
}


#[derive(Debug, PartialEq)]
enum FoundationStartBoundary {
    New,
    Resume(String),
    Hold(String),
}

/// Foundation has no per-episode publication proof. Only a newer completed
/// workflow with the exact frozen scope supersedes an older stopped branch.
/// Unknown submissions keep their original scope even after such a completion.
fn foundation_start_boundary(workflows: &[Value], input: &Value) -> Result<FoundationStartBoundary> {
    let identity = storyboard_source_identity(&input["source"])?;
    let episodes = storyboard_scope_ids(&input["selection"], "episodeIds")?;
    if episodes.is_empty() { return Err("请明确制作基础的集范围".into()) }
    let mut completed_scopes = Vec::new();
    let mut matches = Vec::new();
    for job in workflows {
        if job["kind"] != "workflow" || job["entityId"] != "foundation.run" { continue; }
        let snapshot = &job["snapshot"];
        let original = &snapshot["input"];
        if original["source"]["sha256"] != input["source"]["sha256"] { continue; }
        let selection = original.get("selection").filter(|value| value.is_object()).unwrap_or(&snapshot["selection"]);
        let old_episodes = storyboard_scope_ids(selection, "episodeIds")?;
        if old_episodes.is_empty() {
            return Ok(FoundationStartBoundary::Hold(format!("原制作基础流程 {} 缺少冻结集范围，保留记录且不另建相同来源任务", s(job,"id"))));
        }
        if episodes.is_disjoint(&old_episodes) { continue; }
        // Re-recognizing identical text must not bypass a pending upstream result.
        let unknown = job["submissionUnknown"].as_bool() == Some(true) || s(job,"status") == "submission_unknown";
        if unknown {
            return Ok(FoundationStartBoundary::Hold(format!("原制作基础流程 {} 存在未知提交；先核对原执行，不另建重叠范围或重发", s(job,"id"))));
        }
        let same_source = match storyboard_source_identity(&original["source"]) {
            Ok(old) => old == identity,
            Err(_) => return Ok(FoundationStartBoundary::Hold(format!("原制作基础流程 {} 的源绑定不能核验，保留记录且不另建相同来源任务", s(job,"id")))),
        };
        if !same_source { continue; }
        if s(job,"status") == "completed" && !unknown {
            completed_scopes.push(old_episodes);
            continue;
        }
        if ["cancelled", "closed_by_user"].contains(&s(job,"status")) && !unknown { continue; }
        if !unknown && completed_scopes.iter().any(|scope| scope == &old_episodes) { continue; }
        let id = s(job,"id");
        if unknown {
            return Ok(FoundationStartBoundary::Hold(format!("原制作基础流程 {id} 存在未知提交；先核对原执行，不另建重叠范围或重发")));
        }
        if old_episodes != episodes {
            return Ok(FoundationStartBoundary::Hold(format!("所选范围与未完成制作基础流程 {id} 重叠；请继续原流程，不通过缩小或扩大范围重发")));
        }
        let original_version = original["source"].get("version").filter(|value| !value.is_null()).unwrap_or(&snapshot["sourceVersion"]);
        if *original_version != input["source"]["version"] {
            return Ok(FoundationStartBoundary::Hold(format!("原制作基础流程 {id} 的原文内容与范围相同但保存版本已变化；保留原请求，先核对来源版本绑定")));
        }
        if snapshot["runnerVersion"] != "desktop-runner/v1" || snapshot["stage"] != "foundation.run"
            || !["failed","interrupted","waiting_decision","partially_blocked","blocked"].contains(&s(job,"status")) {
            return Ok(FoundationStartBoundary::Hold(format!("原制作基础流程 {id} 仍在执行或缺少可恢复身份，不另建同范围任务")));
        }
        matches.push(id.to_owned());
    }
    match matches.len() {
        0 => Ok(FoundationStartBoundary::New),
        1 => Ok(FoundationStartBoundary::Resume(matches.remove(0))),
        _ => Ok(FoundationStartBoundary::Hold("相同来源与范围有多个未完成制作基础流程，请从原记录选择继续；未新建或重发".into())),
    }
}

fn existing_foundation_start(store: &Store, input: &Value) -> Result<FoundationStartBoundary> {
    store.ensure_workflow_storage()?;
    let connection = store.conn()?;
    let mut query = connection.prepare(r#"SELECT j.id,j.state,
        json_object('runnerVersion',json_extract(j.snapshot,'$.runnerVersion'),
            'stage',json_extract(j.snapshot,'$.stage'),
            'sourceVersion',json_extract(j.snapshot,'$.sourceVersion'),
            'selection',json_extract(j.snapshot,'$.selection'),
            'input',json_object('source',json_extract(j.snapshot,'$.input.source'),
                'selection',json_extract(j.snapshot,'$.input.selection'))),
        EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs t ON t.id=w.text_job_id
            WHERE w.workflow_id=j.id AND t.state='submission_unknown') OR j.state='submission_unknown'
        FROM jobs j WHERE j.kind='workflow' AND j.entity_id='foundation.run'
            AND json_extract(j.snapshot,'$.input.source.sha256')=?1
        ORDER BY j.created_at DESC,j.rowid DESC"#).map_err(|e|e.to_string())?;
    let records = query.query_map([s(&input["source"],"sha256")],|row| {
        Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,bool>(3)?))
    }).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let workflows: Vec<Value> = records.into_iter().map(|(id,status,snapshot,unknown)| {
        Ok(json!({"id":id,"kind":"workflow","entityId":"foundation.run","status":status,
            "snapshot":serde_json::from_str::<Value>(&snapshot).map_err(|e|e.to_string())?,"submissionUnknown":unknown}))
    }).collect::<Result<_>>()?;
    foundation_start_boundary(&workflows,input)
}

#[cfg(test)]
mod foundation_start_boundary_tests {
    use super::*;

    fn input() -> Value {
        let mut source = json!(pipeline::prepare_source("第一集\n场景：门口\n甲：开门。\n第二集\n场景：院内\n乙：回头。\n第三集\n场景：小路\n甲：走吧。\n").unwrap());
        source["version"] = json!(1);
        json!({"selection":{"episodeIds":source["episodes"].as_array().unwrap().iter().map(|ep|&ep["id"]).collect::<Vec<_>>()},"source":source})
    }
    fn job(input: &Value, status: &str) -> Value {
        json!({"id":"original","kind":"workflow","entityId":"foundation.run","status":status,
            "snapshot":{"runnerVersion":"desktop-runner/v1","stage":"foundation.run","sourceVersion":input["source"]["version"],"selection":input["selection"],"input":input},"submissionUnknown":false})
    }
    fn episode(input: &Value, index: usize) -> Value {
        let mut next = input.clone();next["selection"]["episodeIds"] = json!([input["source"]["episodes"][index]["id"]]);next
    }

    #[test]
    fn same_failed_scope_continues_frozen_original_instead_of_creating_a_new_workflow() {
        let current = input();
        for status in ["failed","interrupted","partially_blocked","blocked","waiting_decision"] {
            let original = job(&current,status);let before = original.to_string();
            assert_eq!(foundation_start_boundary(&[original.clone()],&current).unwrap(),FoundationStartBoundary::Resume("original".into()));
            assert_eq!(original.to_string(),before);
        }
    }

    #[test]
    fn unknown_keeps_its_whole_scope_after_newer_completion_without_storyboard_proofs() {
        let current = input();let mut unknown = job(&current,"partially_blocked");unknown["submissionUnknown"] = json!(true);
        assert!(matches!(foundation_start_boundary(&[job(&current,"completed"),unknown.clone()],&current).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("未知提交")));
        assert!(matches!(foundation_start_boundary(&[unknown.clone()],&episode(&current,1)).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("未知提交")));
        unknown["status"] = json!("submission_unknown");unknown["submissionUnknown"] = json!(false);
        assert!(matches!(foundation_start_boundary(&[unknown],&current).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("未知提交")));
    }

    #[test]
    fn overlapping_rescope_is_held_but_disjoint_episode_scope_is_distinct() {
        let current = input();let first = episode(&current,0);
        assert!(matches!(foundation_start_boundary(&[job(&current,"failed")],&first).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("重叠")));
        assert!(matches!(foundation_start_boundary(&[job(&first,"failed")],&current).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("重叠")));
        assert_eq!(foundation_start_boundary(&[job(&first,"failed")],&episode(&current,2)).unwrap(),FoundationStartBoundary::New);
    }

    #[test]
    fn real_source_change_is_new_but_identical_text_and_catalogue_version_change_is_held() {
        let current = input();let original = job(&current,"failed");
        let mut changed = current.clone();changed["source"] = json!(pipeline::prepare_source("第一集\n场景：门口\n甲：不同的原话。\n").unwrap());changed["source"]["version"] = json!(2);changed["selection"]["episodeIds"] = json!([changed["source"]["episodes"][0]["id"]]);
        assert_eq!(foundation_start_boundary(&[original.clone()],&changed).unwrap(),FoundationStartBoundary::New);
        let mut version_only = current.clone();version_only["source"]["version"] = json!(2);
        assert!(matches!(foundation_start_boundary(&[original.clone()],&version_only).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("版本")));
        let source: pipeline::LockedSource = serde_json::from_value(current["source"].clone()).unwrap();let range = &source.rows.iter().find(|row|row.kind=="body").unwrap().range;
        let decision: pipeline::BoundaryDecision = serde_json::from_value(json!({"start":range.start,"end":range.end,"kind":"body","episodeNumber":1,"sceneTitle":"正式确认的新场地","reason":"合成测试确认范围"})).unwrap();
        changed = current.clone();changed["source"] = json!(pipeline::apply_boundary_decisions(&source,&[decision]).unwrap());changed["source"]["version"] = json!(2);
        assert_eq!(changed["source"]["sha256"],current["source"]["sha256"]);
        assert_eq!(foundation_start_boundary(&[original],&changed).unwrap(),FoundationStartBoundary::New);
    }

    #[test]
    fn completion_allows_explicit_regeneration_and_only_supersedes_exact_frozen_scope() {
        let current = input();
        assert_eq!(foundation_start_boundary(&[job(&current,"completed")],&current).unwrap(),FoundationStartBoundary::New);
        assert_eq!(foundation_start_boundary(&[job(&current,"completed"),job(&current,"failed")],&current).unwrap(),FoundationStartBoundary::New);
        assert_eq!(foundation_start_boundary(&[job(&current,"failed"),job(&current,"completed")],&current).unwrap(),FoundationStartBoundary::Resume("original".into()));
        assert_eq!(foundation_start_boundary(&[job(&episode(&current,0),"completed"),job(&current,"failed")],&current).unwrap(),FoundationStartBoundary::Resume("original".into()));
    }

    #[test]
    fn stage_identity_ambiguity_and_running_records_cannot_be_reused_as_foundation() {
        let current = input();let mut other = job(&current,"failed");other["entityId"] = json!("storyboard.run");
        assert_eq!(foundation_start_boundary(&[other],&current).unwrap(),FoundationStartBoundary::New);
        let mut second = job(&current,"failed");second["id"] = json!("second");
        assert!(matches!(foundation_start_boundary(&[second,job(&current,"interrupted")],&current).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("多个")));
        assert!(matches!(foundation_start_boundary(&[job(&current,"running")],&current).unwrap(),FoundationStartBoundary::Hold(_)));
        let mut wrong_stage = job(&current,"failed");wrong_stage["snapshot"]["stage"] = json!("storyboard.run");
        assert!(matches!(foundation_start_boundary(&[wrong_stage],&current).unwrap(),FoundationStartBoundary::Hold(_)));
    }

    #[test]
    fn sqlite_lookup_uses_only_foundation_history_and_keeps_sdk_unknown_before_new_preflight() {
        let temp = tempfile::tempdir().unwrap();let store = Store::open(temp.path().join("project")).unwrap();store.create("offline foundation start boundary","synthetic").unwrap();store.ensure_workflow_storage().unwrap();
        let current = input();let snapshot = job(&current,"failed")["snapshot"].clone();
        let run = store.add_job("workflow","foundation.run","local","local",&snapshot,"partially_blocked").unwrap();
        for _ in 0..105 {store.add_job("workflow","storyboard.run","local","local",&snapshot,"completed").unwrap();}
        let child = store.add_job("text","foundation:F2","codex_sdk","local",&json!({}),"rejected").unwrap();
        store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,updated_at) VALUES(?1,'original-f2','foundation:F2','synthetic','rejected',?2,?3)",rusqlite::params![run,child,now()]).unwrap();
        assert_eq!(existing_foundation_start(&store,&current).unwrap(),FoundationStartBoundary::Resume(run.clone()));
        let before = store.job(&run).unwrap()["snapshot"].to_string();
        store.update_job(&child,"submission_unknown",&json!({})).unwrap();
        let count: i64 = store.conn().unwrap().query_row("SELECT COUNT(*) FROM jobs",[],|row|row.get(0)).unwrap();
        assert!(matches!(existing_foundation_start(&store,&current).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("未知提交")));
        assert_eq!(store.job(&run).unwrap()["snapshot"].to_string(),before);
        assert_eq!(store.job(&child).unwrap()["status"],"submission_unknown");
        assert_eq!(store.conn().unwrap().query_row("SELECT COUNT(*) FROM jobs",[],|row|row.get::<_,i64>(0)).unwrap(),count);
    }
}

#[cfg(test)]
mod storyboard_start_boundary_tests {
    use super::*;

    fn input() -> Value {
        let mut source = json!(pipeline::prepare_source("第一集\n场景：門口\n甲：开门。\n第二集\n场景：院内\n乙：回头。\n第三集\n场景：小路\n甲：走吧。\n").unwrap());
        source["version"] = json!(1);
        json!({"selection":{"episodeIds":source["episodes"].as_array().unwrap().iter().map(|ep|&ep["id"]).collect::<Vec<_>>()},"source":source})
    }
    fn job(input: &Value, status: &str) -> Value {
        json!({"id":"original","kind":"workflow","entityId":"storyboard.run","status":status,
            "snapshot":{"runnerVersion":"desktop-runner/v1","sourceVersion":input["source"]["version"],"selection":input["selection"],"input":input},
            "prepared":{"publishedEpisodes":{}},"submissionUnknown":false})
    }
    fn episode(input: &Value, index: usize) -> Value {
        let mut next = input.clone();
        next["selection"]["episodeIds"] = json!([input["source"]["episodes"][index]["id"]]);
        next
    }

    #[test]
    fn unfinished_exact_scope_continues_original_without_mutating_frozen_inputs() {
        let current = input();
        for status in ["failed","interrupted","partially_blocked","blocked","waiting_decision"] {
            let original = job(&current,status);
            let before = original.to_string();
            assert_eq!(storyboard_start_boundary(&[original.clone()],&current).unwrap(),StoryboardStartBoundary::Resume("original".into()));
            assert_eq!(original.to_string(),before);
        }
    }

    #[test]
    fn unknown_is_held_even_when_a_newer_completed_branch_exists() {
        let current = input();
        let mut unknown = job(&current,"partially_blocked");
        unknown["submissionUnknown"] = json!(true);
        assert!(matches!(storyboard_start_boundary(&[job(&current,"completed"),unknown],&current).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("未知提交")));
    }

    #[test]
    fn published_all_episode_proofs_cannot_disarm_an_unknown_original_scope() {
        let current = input();
        let mut unknown = job(&current,"partially_blocked");
        unknown["submissionUnknown"] = json!(true);
        for episode in current["source"]["episodes"].as_array().unwrap() {
            let id=s(episode,"id");
            unknown["prepared"]["publishedEpisodes"][id]=json!({"version":"complete-episode-publication/v1","episodeId":id,"sourceSha256":current["source"]["sha256"],"episodeDeliverySha256":"a".repeat(64),"unitIds":[format!("{id}-accepted")]});
        }
        for state in ["partially_blocked","completed"] {
            unknown["status"]=json!(state);
            for selected in [&current,&episode(&current,0)] {
                assert!(matches!(storyboard_start_boundary(&[job(&current,"completed"),unknown.clone()],selected).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("未知提交")));
            }
        }
        let mut changed=current.clone();changed["source"]=json!(pipeline::prepare_source("第一集\n场景：门口\n甲：真实不同的原文。\n").unwrap());changed["source"]["version"]=json!(2);changed["selection"]["episodeIds"]=json!([changed["source"]["episodes"][0]["id"]]);
        assert_eq!(storyboard_start_boundary(&[unknown],&changed).unwrap(),StoryboardStartBoundary::New);
    }

    #[test]
    fn partial_publication_allows_completed_and_disjoint_ranges_but_holds_pending_overlap() {
        let current = input();
        let mut original = job(&current,"partially_blocked");
        let completed = s(&current["source"]["episodes"][0],"id");
        original["prepared"]["publishedEpisodes"][completed] = json!({"version":"complete-episode-publication/v1","episodeId":completed,"sourceSha256":current["source"]["sha256"],"episodeDeliverySha256":"a".repeat(64),"unitIds":["accepted-unit"]});
        assert_eq!(storyboard_start_boundary(&[original.clone()],&episode(&current,0)).unwrap(),StoryboardStartBoundary::New);
        assert!(matches!(storyboard_start_boundary(&[original.clone()],&episode(&current,1)).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("重叠")));
        assert_eq!(storyboard_start_boundary(&[original],&current).unwrap(),StoryboardStartBoundary::Resume("original".into()));
        let isolated = job(&episode(&current,0),"failed");
        assert_eq!(storyboard_start_boundary(&[isolated],&episode(&current,2)).unwrap(),StoryboardStartBoundary::New);
    }

    #[test]
    fn actual_source_changes_are_distinct_but_version_only_change_cannot_reset_a_task() {
        let current = input();
        let original = job(&current,"failed");
        let mut changed_text = current.clone();
        changed_text["source"] = json!(pipeline::prepare_source("第一集\n场景：门口\n甲：另一句。\n").unwrap());
        changed_text["source"]["version"] = json!(2);
        changed_text["selection"]["episodeIds"] = json!([changed_text["source"]["episodes"][0]["id"]]);
        assert_eq!(storyboard_start_boundary(&[original.clone()],&changed_text).unwrap(),StoryboardStartBoundary::New);
        let mut version_only = current.clone();version_only["source"]["version"] = json!(2);
        assert!(matches!(storyboard_start_boundary(&[original.clone()],&version_only).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("版本")));
        let parsed: pipeline::LockedSource=serde_json::from_value(current["source"].clone()).unwrap();
        let range=&parsed.rows.iter().find(|row|row.kind=="body").unwrap().range;
        let decision: pipeline::BoundaryDecision=serde_json::from_value(json!({"start":range.start,"end":range.end,"kind":"body","episodeNumber":1,"sceneTitle":"已确认另一场地","reason":"合成用户明确调整场范围"})).unwrap();
        let mut changed_catalogue = current.clone();changed_catalogue["source"]=json!(pipeline::apply_boundary_decisions(&parsed,&[decision]).unwrap());changed_catalogue["source"]["version"]=json!(2);
        assert_eq!(changed_catalogue["source"]["sha256"],current["source"]["sha256"]);
        assert_eq!(storyboard_start_boundary(&[original],&changed_catalogue).unwrap(),StoryboardStartBoundary::New);
    }

    #[test]
    fn unit_scopes_are_not_conflated_and_changed_revision_instruction_is_not_ignored() {
        let mut current = episode(&input(),0);
        current["selection"]["unitIds"] = json!(["u1"]);current["revisionInstruction"] = json!("保留全部原话，改善动作。");
        let original = job(&current,"interrupted");
        assert_eq!(storyboard_start_boundary(&[original.clone()],&current).unwrap(),StoryboardStartBoundary::Resume("original".into()));
        let mut different = current.clone();different["selection"]["unitIds"] = json!(["u2"]);
        assert_eq!(storyboard_start_boundary(&[original.clone()],&different).unwrap(),StoryboardStartBoundary::New);
        different=current.clone();different["revisionInstruction"] = json!("采用不同的具体演法。");
        assert!(matches!(storyboard_start_boundary(&[original],&different).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("要求")));
    }

    #[test]
    fn completed_regeneration_is_allowed_but_ambiguous_or_running_origins_are_held() {
        let current = input();
        assert_eq!(storyboard_start_boundary(&[job(&current,"completed"),job(&current,"failed")],&current).unwrap(),StoryboardStartBoundary::New);
        let mut second = job(&current,"failed");second["id"] = json!("second");
        assert!(matches!(storyboard_start_boundary(&[second,job(&current,"interrupted")],&current).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("多个")));
        assert!(matches!(storyboard_start_boundary(&[job(&current,"running")],&current).unwrap(),StoryboardStartBoundary::Hold(_)));
        assert_eq!(storyboard_start_boundary(&[],&current).unwrap(),StoryboardStartBoundary::New);
    }

    #[test]
    fn sqlite_lookup_keeps_unknown_and_history_beyond_workspace_summary_limit() {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("offline start boundary","synthetic").unwrap();
        store.ensure_workflow_storage().unwrap();
        let current=input();let snapshot=job(&current,"failed")["snapshot"].clone();
        let run=store.add_job("workflow","storyboard.run","local","local",&snapshot,"partially_blocked").unwrap();
        for _ in 0..105 {store.add_job("workflow","foundation.run","local","local",&json!({}),"completed").unwrap();}
        assert_eq!(existing_storyboard_start(&store,&current).unwrap(),StoryboardStartBoundary::Resume(run.clone()));
        let child=store.add_job("text","storyboardCreate","codex","local",&json!({}),"submission_unknown").unwrap();
        store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,updated_at) VALUES(?1,'original-author','storyboardCreate','synthetic','failed',?2,?3)",rusqlite::params![run,child,now()]).unwrap();
        let before=store.job(&run).unwrap()["snapshot"].to_string();
        assert!(matches!(existing_storyboard_start(&store,&current).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("未知提交")));
        assert_eq!(store.job(&run).unwrap()["snapshot"].to_string(),before);
        assert_eq!(store.job(&child).unwrap()["status"],"submission_unknown");
    }
}

fn cached_video_capabilities(
    store: &Store,
    provider: &str,
    fingerprint: &str,
) -> Result<Option<Capabilities>> {
    use rusqlite::OptionalExtension;
    let connection = store.conn()?;
    let cached: Option<(String, String)> = connection
        .query_row(
            "SELECT data,fetched_at FROM catalogs WHERE provider=?1",
            [provider],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let marker: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("capability_binding:{provider}")],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let marker = marker.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    Ok(cached.and_then(|(raw, at)| {
        let cap: Capabilities = serde_json::from_str(&raw).ok()?;
        let bound = marker
            .as_ref()
            .is_some_and(|m| s(m, "fingerprint") == fingerprint && s(m, "version") == cap.version);
        (bound
            && timestamp_fresh(&at, 900)
            && timestamp_fresh(&cap.checked_at, 900)
            && cap.provider == provider
            && !cap.version.is_empty())
        .then_some(cap)
    }))
}
fn capability_fingerprint(config: &ProviderConfig, account: &str) -> String {
    use sha2::{Digest, Sha256};
    let credential_hash = config
        .credential_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .map(|secret| format!("{:x}", Sha256::digest(secret.as_bytes())));
    let tool=config.cli_path.as_ref().and_then(|path|std::fs::metadata(path).ok()).map(|m|json!({"bytes":m.len(),"modified":m.modified().ok().and_then(|v|v.duration_since(std::time::UNIX_EPOCH).ok()).map(|d|d.as_nanos().to_string())}));
    format!("{:x}",Sha256::digest(serde_json::to_vec(&json!({"config":config,"account":account,"credentialHash":credential_hash,"tool":tool})).unwrap_or_default()))
}
fn save_capability_cache(
    store: &Store,
    config: &ProviderConfig,
    account: &str,
    fingerprint: &str,
    cap: &Capabilities,
) -> Result<()> {
    if cap.provider != config.kind
        || cap.version.is_empty()
        || !timestamp_fresh(&cap.checked_at, 900)
    {
        return Err("能力目录的渠道身份或版本无效".into());
    }
    if capability_fingerprint(config, account) != fingerprint {
        return Err("渠道或账户配置在核实时发生变化，请重试".into());
    }
    let mut conn = store.conn()?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO catalogs VALUES(?1,?2,?3) ON CONFLICT(provider) DO UPDATE SET data=excluded.data,fetched_at=excluded.fetched_at",rusqlite::params![config.kind,serde_json::to_string(cap).map_err(|e|e.to_string())?,now()]).map_err(|e|e.to_string())?;
    tx.execute(
        "INSERT INTO metadata VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![
            format!("capability_binding:{}", config.kind),
            json!({"fingerprint":fingerprint,"version":cap.version}).to_string()
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
fn save_video_parameters_and_invalidate(
    store: &Store,
    parameters: &Value,
    capability_version: &str,
) -> Result<()> {
    let mut conn = store.conn()?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    for key in ["videoProvider", "videoModel", "ratio", "videoResolution"] {
        tx.execute("INSERT INTO settings VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,parameters[key].to_string()]).map_err(|e|e.to_string())?;
    }
    let changed_at = now();
    // Shared ratio affects image requests; video-only settings never invalidate image work.
    // The same transaction closes the settings/claim race on other channel workers.
    tx.execute("UPDATE jobs SET state='stale',error='视频参数或能力版本已变化，请重新准备',next_attempt_at=0,updated_at=?1 WHERE (task_id IS NULL OR task_id='') AND (receipt IS NULL OR receipt IN ('','null','{}')) AND state IN ('queued','preparing','ready','waiting_dependency') AND ((kind='video' AND (provider!=?2 OR COALESCE(json_extract(snapshot,'$.model'),'')!=?3 OR COALESCE(json_extract(snapshot,'$.ratio'),'')!=?4 OR COALESCE(json_extract(snapshot,'$.resolution'),'')!=?5 OR COALESCE(json_extract(snapshot,'$.capabilityVersion'),'')!=?6)) OR (kind='image' AND json_type(snapshot,'$.frozenSettings') IS NULL AND COALESCE(json_extract(snapshot,'$.ratio'),?4)!=?4))",rusqlite::params![changed_at,s(parameters,"videoProvider"),s(parameters,"videoModel"),s(parameters,"ratio"),s(parameters,"videoResolution"),capability_version]).map_err(|e|e.to_string())?;
    tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[changed_at]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
fn validate_video_capacity(model: &ModelCapability, snapshot: &Value) -> Result<()> {
    let duration = snapshot["duration"]
        .as_u64()
        .ok_or("视频时长需要已核实的正整秒")?;
    if duration < model.min_duration.unwrap_or(0) as u64
        || duration > model.max_duration.unwrap_or(0) as u64
        || (!model.allowed_durations.is_empty()
            && !model
                .allowed_durations
                .iter()
                .any(|d| *d as u64 == duration))
    {
        return Err("单元时长超出所选模型能力，请修订此单元，不会自动删减".into());
    }
    let refs = snapshot["references"]
        .as_array()
        .ok_or("冻结素材引用列表缺失")?;
    let count = |kind: &str| refs.iter().filter(|r| s(r, "kind") == kind).count();
    if refs
        .iter()
        .any(|r| !["image", "video", "audio"].contains(&s(r, "kind")))
        || count("image") > model.images
        || count("image") < model.required_images
        || count("video") > model.videos
        || count("audio") > model.audios
        || model
            .total_references
            .is_some_and(|limit| refs.len() > limit)
    {
        return Err("单元素材数量超出所选模型能力，请修订单元，不会自动删除素材".into());
    }
    Ok(())
}
fn open_authorization(value: &str) -> Result<()> {
    let url = authorization_url(value)?;
    #[cfg(target_os = "windows")]
    {
        open_authorization_windows(&url)?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|_| "无法打开即梦授权页".to_string())?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|_| "无法打开即梦授权页".to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn shell_open_result(result: isize) -> Result<()> {
    // ShellExecute returns an HINSTANCE-shaped status, not a process handle.
    if result > 32 {
        Ok(())
    } else {
        Err(format!("无法调用默认浏览器（Windows 错误 {result}）。请复制授权链接到浏览器打开。"))
    }
}

#[cfg(all(test, target_os = "windows"))]
#[test]
fn authorization_browser_errors_never_claim_a_page_opened() {
    for code in [0, 2, 3, 5, 8, 26, 27, 28, 29, 30, 31, 32] {
        let error = shell_open_result(code).unwrap_err();
        assert!(error.contains("复制授权链接"));
    }
    assert!(shell_open_result(33).is_ok());
    for target in ["C:/Users/Public/Documents", "file:///C:/Users", "https://evil.test/", "https://jimeng.jianying.com/?device_code=secret"] {
        assert!(open_authorization(target).is_err());
    }
}

#[cfg(target_os = "windows")]
fn open_authorization_windows(url: &str) -> Result<()> {
    // A URL is the shell target, never an Explorer argument or shell command.
    // A dedicated STA thread also works when invoked from a Tokio/MTA worker.
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    std::thread::spawn(move || {
        use std::{ffi::c_void, ptr};
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(window: *mut c_void, operation: *const u16, file: *const u16, parameters: *const u16, directory: *const u16, show: i32) -> *mut c_void;
        }
        #[link(name = "ole32")]
        unsafe extern "system" {
            fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
            fn CoUninitialize();
        }
        let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        // COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE.
        let initialized = unsafe { CoInitializeEx(ptr::null_mut(), 0x2 | 0x4) };
        if initialized < 0 {
            return Err("无法初始化默认浏览器调用，请复制授权链接到浏览器打开。".into());
        }
        // Both strings remain alive and nul terminated for the synchronous call.
        let result = unsafe { ShellExecuteW(ptr::null_mut(), verb.as_ptr(), url.as_ptr(), ptr::null(), ptr::null(), 1) } as isize;
        unsafe { CoUninitialize(); }
        shell_open_result(result)
    }).join().map_err(|_| "默认浏览器调用中断，请复制授权链接到浏览器打开。".to_string())?
}
fn import_user_media(store: &Store, request: &Value, path: &Path) -> Result<Value> {
    if s(request, "kind") == "image" {
        if let Some(asset) = request["assetId"].as_str() {
            return store.import_image_as_current(path, asset, request["variantId"].as_str());
        }
    }
    store.import_media(
        path,
        s(request, "kind"),
        request["assetId"].as_str(),
        request["variantId"].as_str(),
    )
}
fn bundled_dreamina() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "dreamina.exe"
    } else {
        "dreamina"
    };
    let rel = PathBuf::from("tools/dreamina/1.4.19").join(name);
    let mut roots = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
            roots.push(parent.join("resources"));
            #[cfg(target_os = "macos")]
            if let Some(contents) = parent.parent() {
                roots.push(contents.join("Resources"));
            }
        }
    }
    if let Some(project) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        roots.push(project.to_path_buf());
    }
    roots
        .into_iter()
        .map(|r| r.join(&rel))
        .find(|p| p.is_file())
        .and_then(|p| fs::canonicalize(p).ok())
}

fn image_dependency_ids(assets: &[Value], asset: &Value) -> Result<Vec<String>> {
    if crate::asset_graph::modern(asset) {return crate::asset_graph::dependencies(assets,asset);}
    let mut ids = Vec::new();
    if let Some(refs) = asset["design"]["referenceAssetIds"].as_array() {
        for r in refs {
            let key = r.as_str().ok_or("图片参考必须是稳定资产ID")?;
            if ids.iter().any(|id| id == key) {
                return Err("图片设计有重复参考".into());
            }
            ids.push(key.to_string());
        }
    }
    if let Some(parent) = pipeline::resolve_asset_dependency(assets, asset)? {
        if !ids.contains(&parent) {
            ids.insert(0, parent);
        }
    }
    for key in &ids {
        if key == s(asset, "id") {
            return Err("图片参考引用了自身".into());
        }
        if !assets.iter().any(|a| s(a, "id") == key) {
            return Err(format!("图片参考资产不存在：{key}"));
        }
    }
    Ok(ids)
}
fn image_references(store: &Store, assets: &[Value], asset: &Value) -> Result<Option<Vec<Value>>> {
    if crate::asset_graph::modern(asset) {return crate::asset_graph::media_references(store,assets,asset);}
    let ids = image_dependency_ids(assets, asset)?;
    let mut refs = Vec::new();
    let mut missing = false;
    for key in &ids {
        let a = assets
            .iter()
            .find(|a| s(a, "id") == key)
            .ok_or("图片依赖不存在")?;
        // A manually chosen INLINE image remains an explicit real material.
        // Otherwise INLINE reuses its registered ancestor, never a fabricated image.
        let chain = if crate::asset_graph::modern(asset) || store.adopted(key, s(a, "variantId"))?.is_some() {
            vec![a]
        } else {
            image_reference_chain(assets, a)?
        };
        let mut semantic = Vec::new();
        let mut material = None;
        for candidate in chain {
            semantic.push(json!({"assetId":candidate["id"],"variantId":candidate["variantId"],"assetVersion":candidate["version"]}));
            if let Some(mid) = store.adopted(s(candidate, "id"), s(candidate, "variantId"))? {
                material = Some((candidate, mid));
                break;
            }
        }
        if let Some((material, mid)) = material {
            let m = store.get("media", &mid)?;
            let path = store.verify_media(&m)?;
            if let Some(existing) = refs
                .iter_mut()
                .find(|r: &&mut Value| r["mediaId"] == mid && r["assetId"] == material["id"])
            {
                for binding in semantic {
                    if !existing["semanticBindings"]
                        .as_array()
                        .unwrap()
                        .contains(&binding)
                    {
                        existing["semanticBindings"]
                            .as_array_mut()
                            .unwrap()
                            .push(binding);
                    }
                }
            } else {
                refs.push(json!({"mediaId":mid,"kind":"image","path":path,"relativePath":m["relativePath"],"sha256":m["sha256"],"contentHash":m["sha256"],"mediaRevision":m.get("version").cloned().unwrap_or(json!(1)),"order":refs.len(),"usage":"source","assetId":material["id"],"assetVersion":material["version"],"variantId":material["variantId"],"semanticBindings":semantic,"requirements":asset["referenceRequirements"].as_array().into_iter().flatten().filter(|r|r["targetAssetId"]==material["id"]).cloned().collect::<Vec<_>>(),"adoptionBasis":"current-adoption-record"}));
            }
        } else {
            missing = true
        }
    }
    Ok(if missing { None } else { Some(refs) })
}
fn image_reference_chain<'a>(assets: &'a [Value], asset: &'a Value) -> Result<Vec<&'a Value>> {
    let mut current = asset;
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(s(current, "id")) {
            return Err("沿用图片的素材来源出现循环".into());
        }
        if crate::store::asset_deleted(current) {
            return Err("参考图片的来源资产已删除".into());
        }
        chain.push(current);
        match current.pointer("/preparation/mode").and_then(Value::as_str) {
            Some("UNRESOLVED") => return Err("素材制作方式尚未明确".into()),
            Some("AUDIO") => return Err("纯声音资产不能用作图片来源".into()),
            Some("INLINE") => {
                let parent = pipeline::resolve_asset_dependency(assets, current)?
                    .ok_or("INLINE文字表现没有明确图片来源")?;
                current = assets
                    .iter()
                    .find(|a| s(a, "id") == parent)
                    .ok_or("INLINE指定来源不存在")?;
            }
            _ => return Ok(chain),
        }
    }
}
fn image_closure(
    store: &Store,
    assets: &[Value],
    selection: Option<&Vec<Value>>,
) -> (Vec<String>, Vec<Value>) {
    fn visit(
        key: &str,
        store: &Store,
        assets: &[Value],
        visiting: &mut HashSet<String>,
        done: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if done.contains(key) {
            return Ok(());
        }
        if !visiting.insert(key.to_string()) {
            return Err(format!("图片依赖循环：{key}"));
        }
        let a = assets
            .iter()
            .find(|a| s(a, "id") == key)
            .ok_or_else(|| format!("图片资产不存在：{key}"))?;
        if a.pointer("/preparation/mode").and_then(Value::as_str) == Some("UNRESOLVED") {
            return Err(format!("形象「{}」的素材制作方式尚未明确", s(a, "name")));
        }
        if a.pointer("/preparation/mode").and_then(Value::as_str) == Some("INLINE")
            && store.adopted(key, s(a, "variantId"))?.is_some()
        {
            visiting.remove(key);
            done.insert(key.to_string());
            return Ok(());
        }
        for parent in image_dependency_ids(assets, a)? {
            visit(&parent, store, assets, visiting, done, order)?;
        }
        visiting.remove(key);
        done.insert(key.to_string());
        if independent_image(a) {
            order.push(key.to_string());
        }
        Ok(())
    }
    let selected: Vec<_> = selection
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| assets.iter().map(|a| s(a, "id").to_string()).collect());
    let mut done = HashSet::new();
    let mut order = Vec::new();
    let mut errors = Vec::new();
    for key in selected {
        let completed_before = done.clone();
        let order_before = order.len();
        if let Err(e) = visit(
            &key,
            store,
            assets,
            &mut HashSet::new(),
            &mut done,
            &mut order,
        ) {
            // An invalid selected branch must not leak its partial ancestors
            // into billable work; ancestors of other valid selections remain.
            done = completed_before;
            order.truncate(order_before);
            errors.push(json!({"assetId":key,"reason":e}));
        }
    }
    (order, errors)
}
fn independent_image(asset: &Value) -> bool {
    !matches!(
        asset.pointer("/preparation/mode").and_then(Value::as_str),
        Some("INLINE" | "AUDIO" | "UNRESOLVED")
    )
}
fn self_capabilities(store: &Store) -> Result<Value> {
    let settings = store.settings()?;
    let provider = s(&settings, "videoProvider");
    let model = s(&settings, "videoModel");
    if model.trim().is_empty() {
        // An explicit versioned writing specification, not a claim of live channel availability.
        return Ok(
            json!({"profileId":"storyboard-planning/seedance2.5","version":"storyboard-planning/seedance2.5/v1","planningOnly":true,"source":"versioned_planning_profile","model":"seedance2.5","provider":"planning","minDuration":4,"maxDuration":30,"imageLimit":30,"videoLimit":10,"audioLimit":10,"totalReferenceLimit":50,"maxTotalReferences":50,"basis":"official_cli_1.4.19_help_verified_2026-10-04","requiresLivePreflightAtStage4":true}),
        );
    }
    let raw: String = store
        .conn()?
        .query_row(
            "SELECT data FROM catalogs WHERE provider=?1",
            [provider],
            |r| r.get(0),
        )
        .map_err(|_| "请先刷新所选视频渠道能力，再生成分镜")?;
    let caps: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    planning_capabilities_from_catalog(&settings,&caps)
}
fn planning_capabilities_from_catalog(settings:&Value,caps:&Value)->Result<Value>{
    let provider=s(settings,"videoProvider");
    let model=s(settings,"videoModel");
    let m = caps["models"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|m| s(m, "id") == model && s(m, "kind") == "video")
        })
        .ok_or("所选视频模型没有已核实能力")?;
    if m["adapted"].as_bool() != Some(true) {
        return Err("模型尚未完成参数适配".into());
    }
    let min = m["minDuration"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("所选视频模型没有已核实最小时长")?;
    let max = m["maxDuration"]
        .as_u64()
        .filter(|n| *n >= min)
        .ok_or("所选视频模型没有已核实最大时长")?;
    if s(&caps, "version").is_empty()
        || ["images", "videos", "audios"]
            .iter()
            .any(|key| m[*key].as_u64().is_none())
    {
        return Err("所选视频模型能力版本或素材容量尚未核实".into());
    }
    Ok(
        json!({"minDuration":min,"maxDuration":max,"imageLimit":m["images"],"videoLimit":m["videos"],"audioLimit":m["audios"],"totalReferenceLimit":m["totalReferences"],"maxTotalReferences":m["totalReferences"],"allowedDurations":m["allowedDurations"],"model":model,"provider":provider,"version":caps["version"],"planningOnly":false,"requiresLivePreflightAtStage4":true}),
    )
}

/// Validate a frozen rule set before resuming. The sole compatibility exception
/// is the desktop transport footer of restored foundation F2/Q1 rules; their
/// original methods and every other rule remain byte-for-byte immutable.
pub fn workflow_resume_rule_audit(job: &Value, current_rules: &Value) -> Result<Option<Value>> {
    const CHANGED: &str = "制作规则已更新，旧流程与成果已保留；请按新规则启动阶段";
    let snapshot = &job["snapshot"];
    let frozen = &snapshot["rules"];
    if frozen == current_rules {
        return Ok(None);
    }
    if s(job, "kind") != "workflow"
        || s(job, "entityId") != "foundation.run"
        || s(snapshot, "stage") != "foundation.run"
        || snapshot["runnerVersion"] != "desktop-runner/v1"
        || snapshot["foundationMethod"] != "foundation-delivery/v2"
    {
        return Err(CHANGED.into());
    }
    let old = frozen.as_array().ok_or(CHANGED)?;
    let current = current_rules.as_array().ok_or(CHANGED)?;
    if old.is_empty() || old.len() != current.len() {
        return Err(CHANGED.into());
    }
    let mut ids = HashSet::new();
    let mut changed_rules = Vec::new();
    let mut old_digests = Vec::new();
    let mut current_digests = Vec::new();
    for (before, after) in old.iter().zip(current) {
        let id = s(before, "id");
        let version = s(before, "version");
        if id.is_empty()
            || version.is_empty()
            || !ids.insert(id)
            || before["id"] != after["id"]
            || before["version"] != after["version"]
        {
            return Err(CHANGED.into());
        }
        let old_content = before["content"].as_str().ok_or(CHANGED)?;
        let new_content = after["content"].as_str().ok_or(CHANGED)?;
        if s(before, "sha256") != pipeline::digest(old_content)
            || s(after, "sha256") != pipeline::digest(new_content)
        {
            return Err(CHANGED.into());
        }
        old_digests.push(json!({"id":id,"version":version,"sha256":before["sha256"]}));
        current_digests.push(json!({"id":id,"version":version,"sha256":after["sha256"]}));
        if before == after {
            continue;
        }
        if !matches!(
            id,
            "foundation-asset-catalog" | "foundation-finishing-editor"
        ) {
            return Err(CHANGED.into());
        }
        // Only content and its digest may differ, including any future metadata.
        let mut old_identity = before.as_object().ok_or(CHANGED)?.clone();
        let mut new_identity = after.as_object().ok_or(CHANGED)?.clone();
        for key in ["content", "sha256"] {
            old_identity.remove(key);
            new_identity.remove(key);
        }
        if old_identity != new_identity {
            return Err(CHANGED.into());
        }
        let old_body = foundation_method_before_transport(old_content).ok_or(CHANGED)?;
        let new_body = foundation_method_before_transport(new_content).ok_or(CHANGED)?;
        if old_body != new_body {
            return Err(CHANGED.into());
        }
        changed_rules.push(json!({"id":id,"version":version,"oldSha256":before["sha256"],"currentSha256":after["sha256"],"originalBodySha256":pipeline::digest(old_body)}));
    }
    if changed_rules.is_empty() {
        return Err(CHANGED.into());
    }
    Ok(Some(json!({
        "policy":"foundation-transport-footer/v1",
        "stage":"foundation.run",
        "foundationMethod":"foundation-delivery/v2",
        "frozenSnapshotSha256":pipeline::digest(&snapshot.to_string()),
        "sourceVersion":snapshot["sourceVersion"],
        "sourceSha256":snapshot["input"]["source"]["sha256"],
        "oldRules":old_digests,
        "currentRules":current_digests,
        "changedRules":changed_rules,
        "originalSnapshotPreserved":true
    })))
}

// An exact, standalone heading distinguishes the appendix from a phrase inside
// the original method. Ambiguous duplicate appendix headings fail closed.
fn foundation_method_before_transport(content: &str) -> Option<&str> {
    const HEADING: &str = "## 桌面运输契约";
    let mut headings = content.match_indices(HEADING).filter(|(offset, _)| {
        let suffix = &content[offset + HEADING.len()..];
        (*offset == 0 || content.as_bytes()[offset - 1] == b'\n')
            && (suffix.is_empty() || suffix.starts_with('\n') || suffix.starts_with("\r\n"))
    });
    let (offset, _) = headings.next()?;
    if headings.next().is_some() {
        return None;
    }
    Some(&content[..offset])
}

fn production_rules(stage: &str) -> Value {
    let ids: Vec<&str> = if stage == "foundation.run" {
        vec![
            "screenplay-fidelity",
            "foundation-asset-catalog",
            "foundation-finishing-editor",
        ]
    } else {
        vec![
            "screenplay-fidelity",
            "episode-emotion-planner",
            "unit-performance-director",
            "episode-finishing-editor",
            "acting-blocking",
            "camera-emotion",
            "lighting-space",
            "sound-rhythm",
            "fantasy-expression",
            "storyboard-common",
            "storyboard-source-context",
            "storyboard-card-camera-path-repair",
            "storyboard-card-comic-contrast",
            "storyboard-card-danger-rescue",
            "storyboard-card-editing-integrity",
            "storyboard-card-expressive-emphasis",
            "storyboard-card-joy-acceptance",
            "storyboard-card-long-voice",
            "storyboard-card-quiet-nonhuman",
            "storyboard-card-scale-silence",
            "storyboard-card-spatial-contact",
            "storyboard-card-status-conflict",
        ]
    };
    json!(pipeline::rule_snapshots()
        .into_iter()
        .filter(|r| ids.contains(&r.id.as_str()))
        .collect::<Vec<_>>())
}
// A previously started run keeps its actual rules and requests. It may finish or
// resume its accepted steps; a new run uses the restored F2 → Q1 method.
fn production_rules_for_snapshot(snapshot: &Value) -> Result<Value> {
    if pipeline::series::uses_profile(&snapshot["input"])? {
        return Ok(json!(pipeline::series::rule_snapshots_for(&snapshot["input"])?));
    }
    if pipeline::stage3_pack::uses_profile(&snapshot["input"])? {
        let mut checked = json!({});
        pipeline::stage3_pack::attach_profile(&mut checked, &snapshot["input"])?;
        return Ok(json!(pipeline::stage3_pack::rule_snapshots()?));
    }
    if s(snapshot, "stage") != "foundation.run" {
        return Ok(production_rules(s(snapshot, "stage")));
    }
    match snapshot["foundationMethod"].as_str() {
        Some("foundation-delivery/v2") => Ok(production_rules("foundation.run")),
        None => Ok(json!(pipeline::rule_snapshots()
            .into_iter()
            .filter(|r| ["screenplay-fidelity", "story-foundation"].contains(&r.id.as_str()))
            .collect::<Vec<_>>())),
        _ => Err("制作基础执行方法版本不受支持，已保留原记录".into()),
    }
}
const STORYBOARD_REVISION_PROFILE: &str = "accepted-plan-revision/v1";
const STORYBOARD_REVISION_CONTEXT: &str = "accepted-storyboard-revision/v1";
fn revision_origin_error(detail: &str) -> String {
    format!(
        "局部重做无法证明当前采用稿的原规划与幻想合同：{detail}；请按当前原文生成整集，未调用模型"
    )
}
fn adopted_unit_content(unit: &Value) -> Value {
    let mut value = unit.clone();
    if let Some(object) = value.as_object_mut() {
        for field in [
            "continuityStale",
            "foundationVersion",
            "planningProfile",
            "retired",
            "sourceVersion",
            "version",
        ] {
            object.remove(field);
        }
    }
    value
}
// This is only the documented creative/v1 transport projection between an
// accepted Author and the Finisher's actual input. It is never used to compare
// a current adopted unit with a workflow's formal published result.
fn consumed_author_unit(unit: &Value, creative: bool) -> Value {
    let mut value = adopted_unit_content(unit);
    if creative {
        if let Some(object) = value.as_object_mut() {
            for field in [
                "prompt",
                "channelPromptTemplate",
                "referenceBindings",
                "visualStyle",
                "seedanceExportVersion",
                "storyboardCreativeProfile",
            ] {
                object.remove(field);
            }
        }
    }
    value
}
fn selected_revision_candidate_matches(planned: &Value, selected: &Value) -> bool {
    if planned == selected {
        return true;
    }
    let mut card = json!({});
    for field in [
        "id",
        "sourceRefs",
        "subject",
        "emotionPurpose",
        "screenSecondsRange",
        "extraSecondsRange",
    ] {
        if let Some(value) = planned.get(field) {
            card[field] = value.clone();
        }
    }
    if !s(planned, "proposal").trim().is_empty() {
        card["suggestedProposal"] = planned["proposal"].clone();
    }
    card == *selected
}
struct RevisionAcceptedStep {
    job_id: String,
    hash: String,
    stage: String,
    snapshot: Value,
    result: Value,
    raw_result: Value,
}
fn revision_accepted_steps(
    connection: &rusqlite::Connection,
    run: &str,
    episode: &str,
) -> Result<Vec<RevisionAcceptedStep>> {
    let mut query = connection.prepare("SELECT j.id,w.request_hash,w.stage,j.snapshot,w.result,j.receipt FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.state='accepted' AND j.kind='text' AND j.state='completed' AND w.stage IN ('storyboardPlan','storyboardCreate','storyboardFinish','storyboardRefine') AND COALESCE(json_extract(j.snapshot,'$.input.episodeId'),json_extract(j.snapshot,'$.input.episode.id'))=?2 AND NOT EXISTS(SELECT 1 FROM workflow_steps newer WHERE newer.workflow_id=w.workflow_id AND newer.stage=w.stage AND newer.request_hash=w.request_hash AND (newer.updated_at>w.updated_at OR (newer.updated_at=w.updated_at AND newer.rowid>w.rowid)))").map_err(|e| e.to_string())?;
    let rows = query
        .query_map(rusqlite::params![run, episode], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut steps = Vec::new();
    for row in rows {
        let (job_id, hash, stage, raw_snapshot, raw_result, raw_receipt) =
            row.map_err(|e| e.to_string())?;
        let snapshot: Value = serde_json::from_str(&raw_snapshot)
            .map_err(|_| revision_origin_error("原执行快照无效"))?;
        let result: Value = serde_json::from_str(&raw_result)
            .map_err(|_| revision_origin_error("已采用节点结果无效"))?;
        let receipt: Value = serde_json::from_str(&raw_receipt)
            .map_err(|_| revision_origin_error("节点校验回执无效"))?;
        let skill = match stage.as_str() {
            "storyboardPlan" => "episode-emotion-planner",
            "storyboardCreate" => "unit-performance-director",
            "storyboardRefine" => "director-refinement",
            _ => "episode-finishing-editor",
        };
        if pipeline::digest(&raw_snapshot) != hash
            || snapshot["stage"] != stage
            || snapshot["skill"] != skill
            || receipt["validation"] != "accepted"
            || receipt["contract"] != "desktop-runner/v1"
            || receipt
                .get("acceptedRequestHash")
                .is_some_and(|value| value != &json!(hash))
        {
            return Err(revision_origin_error(
                "原节点身份、请求哈希或校验回执发生变化",
            ));
        }
        let projected = if ["storyboardFinish", "storyboardRefine"].contains(&stage.as_str())
            && result["materializedDraft"].is_object()
        { result["materializedDraft"].clone() } else { result.clone() };
        steps.push(RevisionAcceptedStep {
            job_id,
            hash,
            stage,
            snapshot,
            result: projected,
            raw_result: result,
        });
    }
    Ok(steps)
}
fn revision_plan_provenance(
    connection: &rusqlite::Connection,
    run: &str,
    snapshot: &Value,
    output: &Value,
    episode: &str,
    source_sha: &str,
    steps: &[RevisionAcceptedStep],
) -> Result<Value> {
    if snapshot["input"]["storyboardRevisionProfile"] == STORYBOARD_REVISION_PROFILE {
        let matches: Vec<_> = snapshot["input"]["revisionContexts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|context| context["episodeId"] == episode)
            .collect();
        if matches.len() != 1 {
            return Err(revision_origin_error("上一局部采用稿缺少唯一原规划来源"));
        }
        let inherited = matches[0];
        if inherited["version"] != STORYBOARD_REVISION_CONTEXT
            || inherited["sourceSha256"] != source_sha
            || s(inherited, "originWorkflowId").is_empty()
            || s(inherited, "planJobId").is_empty()
            || s(inherited, "planRequestHash").is_empty()
        {
            return Err(revision_origin_error(
                "上一局部采用稿的原规划来源合同不完整",
            ));
        }
        let origin = s(inherited, "originWorkflowId");
        let original_snapshot: String = connection.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'", [origin], |row| row.get(0)).map_err(|_| revision_origin_error("继承的原规划流程不存在"))?;
        let original: Value = serde_json::from_str(&original_snapshot)
            .map_err(|_| revision_origin_error("继承的原规划流程快照无效"))?;
        if original["input"]["source"]["sha256"] != source_sha {
            return Err(revision_origin_error("继承规划不属于当前锁定原文"));
        }
        let original_steps = revision_accepted_steps(connection, origin, episode)?;
        if !original_steps.iter().any(|step| {
            step.stage == "storyboardPlan"
                && step.job_id == s(inherited, "planJobId")
                && step.hash == s(inherited, "planRequestHash")
                && step.result == inherited["plan"]
        }) {
            return Err(revision_origin_error(
                "继承的Planner不是原流程精确已采用节点",
            ));
        }
        let bindings = inherited["unitBindings"]
            .as_array()
            .filter(|values| !values.is_empty())
            .ok_or_else(|| revision_origin_error("上一局部稿缺少采用单元绑定"))?;
        let previous = snapshot["input"]["existingUnits"]
            .as_array()
            .ok_or_else(|| revision_origin_error("上一局部稿缺少冻结单元"))?;
        let mut seen = HashSet::new();
        for binding in bindings {
            let unit_id = s(binding, "unitId");
            let unit = previous
                .iter()
                .find(|unit| unit["id"] == unit_id)
                .ok_or_else(|| revision_origin_error("继承绑定的原单元不在冻结稿中"))?;
            if unit_id.is_empty()
                || !seen.insert(unit_id)
                || unit["episodeId"] != episode
                || binding["unitVersion"] != unit["version"]
                || s(binding, "unitSha256") != pipeline::digest(&unit.to_string())
            {
                return Err(revision_origin_error(
                    "继承单元的身份、版本或完整稿哈希不一致",
                ));
            }
        }
        return Ok(
            json!({"originWorkflowId":origin,"planJobId":inherited["planJobId"],"planRequestHash":inherited["planRequestHash"],"plan":inherited["plan"]}),
        );
    }
    if snapshot["input"].get("storyboardRevisionProfile").is_some()
        || !snapshot["input"]["selection"]["unitIds"].is_null()
    {
        return Err(revision_origin_error("旧局部稿没有可继承的正式Planner来源"));
    }
    let plans: Vec<_> = output["plans"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["episodeId"] == episode)
        .collect();
    if plans.len() != 1 {
        return Err(revision_origin_error("正式采用稿没有唯一整集规划"));
    }
    let accepted: Vec<_> = steps
        .iter()
        .filter(|step| step.stage == "storyboardPlan" && step.result == plans[0]["plan"])
        .collect();
    if accepted.len() != 1 {
        return Err(revision_origin_error("正式规划没有唯一精确Planner验收节点"));
    }
    Ok(
        json!({"originWorkflowId":run,"planJobId":accepted[0].job_id,"planRequestHash":accepted[0].hash,"plan":accepted[0].result}),
    )
}
fn revision_context_for_origin(
    connection: &rusqlite::Connection,
    run: &str,
    snapshot: &Value,
    output: &Value,
    episode: &str,
    source_sha: &str,
    existing: &[Value],
) -> Result<Value> {
    if snapshot["stage"] == "storyboard.refine" {
        return revision_context_from_refinement(connection,run,snapshot,output,episode,source_sha,existing);
    }
    let steps = revision_accepted_steps(connection, run, episode)?;
    let provenance = revision_plan_provenance(
        connection, run, snapshot, output, episode, source_sha, &steps,
    )?;
    let adopted = output["units"]
        .as_array()
        .ok_or_else(|| revision_origin_error("原流程缺少正式单元稿"))?;
    let mut bindings = Vec::new();
    let mut finishes = HashSet::new();
    for unit in existing
        .iter()
        .filter(|unit| unit["episodeId"] == episode && unit["retired"] != true)
    {
        let matches: Vec<_> = adopted
            .iter()
            .filter(|original| {
                original["id"] == unit["id"]
                    && adopted_unit_content(original) == adopted_unit_content(unit)
            })
            .collect();
        if matches.is_empty() {
            continue;
        }
        if matches.len() != 1 || unit["version"].as_u64().unwrap_or(0) == 0 {
            return Err(revision_origin_error("采用单元身份或版本不唯一"));
        }
        let owners: Vec<_> = steps
            .iter()
            .filter(|step| {
                step.stage == "storyboardFinish"
                    && step.result["units"].as_array().is_some_and(|units| {
                        units.iter().any(|original| {
                            original["id"] == unit["id"]
                                && adopted_unit_content(original) == adopted_unit_content(unit)
                        })
                    })
            })
            .collect();
        if owners.len() != 1 {
            return Err(revision_origin_error("正式单元没有唯一整编验收来源"));
        }
        let finish = owners[0];
        let draft = finish.snapshot["input"]
            .get("previousDraft")
            .or_else(|| finish.snapshot["input"].get("draft"))
            .and_then(|draft| draft["units"].as_array())
            .filter(|units| !units.is_empty())
            .ok_or_else(|| revision_origin_error("整编稿缺少实际已采用主创输入"))?;
        let creative = finish.snapshot["input"]["storyboardCreativeProfile"]
            == "original-production-contract/v1";
        for original in draft {
            if !steps.iter().any(|step| {
                step.stage == "storyboardCreate"
                    && step.result["units"].as_array().is_some_and(|units| {
                        units.iter().any(|unit| {
                            consumed_author_unit(unit, creative) == adopted_unit_content(original)
                        })
                    })
            }) {
                return Err(revision_origin_error("整编输入无法回绑已采用主创稿"));
            }
        }
        finishes.insert(finish.job_id.as_str());
        bindings.push(json!({"unitId":unit["id"],"unitVersion":unit["version"],"unitSha256":pipeline::digest(&unit.to_string())}));
    }
    bindings.sort_by(|left, right| s(left, "unitId").cmp(s(right, "unitId")));
    let bound: HashSet<_> = bindings
        .iter()
        .map(|binding| s(binding, "unitId"))
        .collect();
    let formal = output["fantasyRealization"]
        .as_array()
        .ok_or_else(|| revision_origin_error("正式采用稿缺少幻想兑现记录"))?;
    let mut realized = Vec::new();
    let mut candidates = Vec::new();
    for record in formal {
        let units = record["unitIds"]
            .as_array()
            .filter(|units| !units.is_empty())
            .ok_or_else(|| revision_origin_error("正式幻想记录缺少单元身份"))?;
        if !units
            .iter()
            .any(|unit| unit.as_str().is_some_and(|id| bound.contains(id)))
        {
            continue;
        }
        if units
            .iter()
            .any(|unit| unit.as_str().is_none_or(|id| !bound.contains(id)))
        {
            return Err(revision_origin_error(
                "幻想关联组的部分单元已不属于同一正式采用稿",
            ));
        }
        let owners: Vec<_> = steps
            .iter()
            .filter(|step| {
                finishes.contains(step.job_id.as_str())
                    && step.result["fantasyRealization"]
                        .as_array()
                        .is_some_and(|records| records.iter().any(|item| item == record))
            })
            .collect();
        if owners.len() != 1 {
            return Err(revision_origin_error("正式幻想兑现缺少唯一采用整编记录"));
        }
        let selected: Vec<_> = owners[0].snapshot["input"]["selectedFantasy"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|candidate| candidate["id"] == record["candidateId"])
            .collect();
        if selected.len() != 1
            || !provenance["plan"]["fantasyCandidates"]
                .as_array()
                .is_some_and(|planned| {
                    planned.iter().any(|candidate| {
                        selected_revision_candidate_matches(candidate, selected[0])
                    })
                })
        {
            return Err(revision_origin_error(
                "幻想方案不是原Planner方案与实际已选对象",
            ));
        }
        let draft = owners[0].snapshot["input"]
            .get("previousDraft")
            .or_else(|| owners[0].snapshot["input"].get("draft"))
            .and_then(|draft| draft["units"].as_array())
            .unwrap();
        let creative = owners[0].snapshot["input"]["storyboardCreativeProfile"]
            == "original-production-contract/v1";
        if !steps.iter().any(|step| {
            step.stage == "storyboardCreate"
                && step.snapshot["input"]["selectedFantasy"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|value| value == selected[0]))
                && step.result["units"].as_array().is_some_and(|units| {
                    units.iter().any(|unit| {
                        draft.iter().any(|original| {
                            consumed_author_unit(unit, creative) == adopted_unit_content(original)
                        })
                    })
                })
        }) {
            return Err(revision_origin_error(
                "已选幻想未实际进入同一采用链的主创输入",
            ));
        }
        if !candidates.iter().any(|candidate| candidate == selected[0]) {
            candidates.push(selected[0].clone());
        }
        if !realized.iter().any(|item| item == record) {
            realized.push(record.clone());
        }
    }
    Ok(
        json!({"version":STORYBOARD_REVISION_CONTEXT,"sourceSha256":source_sha,"episodeId":episode,"originWorkflowId":provenance["originWorkflowId"],"planJobId":provenance["planJobId"],"planRequestHash":provenance["planRequestHash"],"unitBindings":bindings,"plan":provenance["plan"],"selectedFantasy":candidates,"fantasyRealization":realized}),
    )
}
fn revision_context_from_refinement(
    connection: &rusqlite::Connection,
    run: &str,
    snapshot: &Value,
    projection: &Value,
    episode: &str,
    source_sha: &str,
    existing: &[Value],
) -> Result<Value> {
    use rusqlite::OptionalExtension;
    let candidate=&projection["refinementCandidate"];
    let receipt=&projection["refinementAdoption"];
    if receipt["version"]!="storyboard-refinement-adoption/v1" || receipt["workflowId"]!=run
        || receipt["candidateSha256"]!=candidate["candidateSha256"] || receipt["sourceSha256"]!=source_sha
        || receipt["episodeId"]!=episode || receipt["acceptedPlanContext"]!=candidate["acceptedContext"]
        || receipt["domainRequestSha256"]!=candidate["domainRequestSha256"] || receipt["ruleReceipt"]!=candidate["ruleReceipt"]
    {return Err(revision_origin_error("精修采用回执没有回绑原候选与规划"));}
    let mut frozen=snapshot["input"].clone();
    pipeline::storyboard_refinement_identity::attach_recovery(&mut frozen,snapshot,projection)?;
    if receipt.get(pipeline::storyboard_refinement_identity::FIELD)!=candidate.get(pipeline::storyboard_refinement_identity::FIELD)
        || (candidate.get(pipeline::storyboard_refinement_identity::FIELD).is_some()
            && refinement_identity::packet(connection,&frozen)?.as_ref()!=candidate.get(pipeline::storyboard_refinement_identity::FIELD)) {
        return Err(revision_origin_error("精修身份重绑定未回绑原登记来源"));
    }
    let expected=pipeline::adopt_storyboard_refinement(&frozen,candidate)
        .map_err(|_|revision_origin_error("精修候选不能重现原领域验证"))?;
    if receipt["deliverySha256"]!=pipeline::digest(&expected.to_string()) {
        return Err(revision_origin_error("精修采用作品与原差量不一致"));
    }
    let steps=revision_accepted_steps(connection,run,episode)?;
    let accepted:Vec<_>=steps.iter().filter(|step|{
        let request=serde_json::from_value::<pipeline::TextRequest>(step.snapshot.clone());
        let mut raw=step.raw_result.clone();
        if let Some(object)=raw.as_object_mut(){object.remove("materializedDraft");}
        step.stage=="storyboardRefine" && raw==candidate["rawDelta"]
            && step.snapshot["ruleSnapshot"]["creativePack"]==candidate["ruleReceipt"]
            && request.is_ok_and(|request|pipeline::digest(&serde_json::to_string(&request).unwrap())==candidate["domainRequestSha256"])
    }).collect();
    if accepted.len()!=1 {return Err(revision_origin_error("精修没有唯一原文绑定的领域验收节点"));}
    let original_event:Option<String>=connection.query_row("SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY rowid DESC LIMIT 1",
        [&accepted[0].job_id],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
    let original_event:Value=original_event.as_deref().map(serde_json::from_str).transpose()
        .map_err(|_|revision_origin_error("精修原回包事件无效"))?.unwrap_or(Value::Null);
    if original_event["requestHash"]!=accepted[0].hash || original_event["value"]!=candidate["rawDelta"] {
        return Err(revision_origin_error("精修采用丢失精确原回包事件"));
    }
    let mut inherited=snapshot.clone();
    inherited["input"]["storyboardRevisionProfile"]=json!(STORYBOARD_REVISION_PROFILE);
    inherited["input"]["revisionContexts"]=json!([receipt["acceptedPlanContext"]]);
    let provenance=revision_plan_provenance(connection,run,&inherited,&expected,episode,source_sha,&steps)?;
    let adopted=receipt["adoptedUnits"].as_array().ok_or_else(||revision_origin_error("精修采用缺少实际单位后像"))?;
    let post_bindings=receipt["unitBindings"].as_array().ok_or_else(||revision_origin_error("精修采用缺少版本后像"))?;
    let candidate_units=candidate["draft"]["units"].as_array().ok_or_else(||revision_origin_error("精修候选缺少全稿"))?;
    let mut seen=HashSet::new();
    if adopted.len()!=post_bindings.len() || adopted.len()!=candidate_units.len()
        || adopted.iter().any(|unit|{
            !seen.insert(s(unit,"id")) || unit["episodeId"]!=episode
                || post_bindings.iter().filter(|binding|binding["unitId"]==unit["id"] && binding["unitVersion"]==unit["version"]
                    && binding["unitSha256"]==pipeline::digest(&unit.to_string())).count()!=1
                || candidate_units.iter().filter(|original|original["id"]==unit["id"]
                    && adopted_unit_content(original)==adopted_unit_content(unit)).count()!=1
        }) {return Err(revision_origin_error("精修采用单位、作品或版本后像无效"));}
    let mut bindings=Vec::new();
    for unit in existing.iter().filter(|unit|unit["episodeId"]==episode && unit["retired"]!=true) {
        if adopted.iter().any(|original|original["id"]==unit["id"] && adopted_unit_content(original)==adopted_unit_content(unit)) {
            bindings.push(json!({"unitId":unit["id"],"unitVersion":unit["version"],"unitSha256":pipeline::digest(&unit.to_string())}));
        }
    }
    let bound:HashSet<_>=bindings.iter().map(|binding|s(binding,"unitId")).collect();
    let formal=receipt["fantasyRealization"].as_array().ok_or_else(||revision_origin_error("精修采用缺少完整幻想兑现"))?;
    if receipt["fantasyRealization"]!=candidate["draft"]["fantasyRealization"] {return Err(revision_origin_error("精修采用幻想不属于原验收作品"));}
    let selected=receipt["acceptedPlanContext"]["selectedFantasy"].as_array().ok_or_else(||revision_origin_error("精修采用丢失原已选幻想"))?;
    let mut realized=Vec::new();let mut candidates=Vec::new();
    for record in formal {
        let ids=record["unitIds"].as_array().filter(|ids|!ids.is_empty()).ok_or_else(||revision_origin_error("精修幻想缺少实际单位身份"))?;
        if !ids.iter().any(|id|id.as_str().is_some_and(|id|bound.contains(id))) {continue;}
        if ids.iter().any(|id|id.as_str().is_none_or(|id|!bound.contains(id))) {return Err(revision_origin_error("精修幻想关联组已不完整属于同一采用稿"));}
        let matching:Vec<_>=selected.iter().filter(|candidate|candidate["id"]==record["candidateId"]).collect();
        if matching.len()!=1 {return Err(revision_origin_error("精修幻想不属于原已选合同"));}
        realized.push(record.clone());
        if !candidates.iter().any(|candidate|candidate==matching[0]) {candidates.push(matching[0].clone());}
    }
    Ok(json!({"version":STORYBOARD_REVISION_CONTEXT,"sourceSha256":source_sha,"episodeId":episode,
        "originWorkflowId":provenance["originWorkflowId"],"planJobId":provenance["planJobId"],"planRequestHash":provenance["planRequestHash"],
        "unitBindings":bindings,"plan":provenance["plan"],"selectedFantasy":candidates,"fantasyRealization":realized}))
}
fn accepted_storyboard_revision_contexts(
    store: &Store,
    input: &Value,
    ids: &[Value],
) -> Result<Vec<Value>> {
    let source_sha = s(&input["source"], "sha256");
    let existing = input["existingUnits"]
        .as_array()
        .ok_or_else(|| revision_origin_error("缺少当前采用单元"))?;
    let mut selected = HashSet::new();
    for id in ids {
        let unit_id = id
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| revision_origin_error("单元ID无效"))?;
        if !selected.insert(unit_id) {
            return Err(revision_origin_error("单元选择重复"));
        }
    }
    let connection = store.conn()?;
    crate::workflow::verify_refinement_continuity_history(&connection,input)?;
    let has_steps: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_steps')", [], |row|row.get(0)).map_err(|e|e.to_string())?;
    if !has_steps {
        return Err(revision_origin_error("项目没有正式采用流程节点"));
    }
    let mut query = connection.prepare("SELECT id,snapshot,result,prepared,entity_id FROM jobs WHERE kind='workflow' AND entity_id IN ('storyboard.run','storyboard.refine') AND result IS NOT NULL AND json_extract(result,'$.sourceSha256')=?1 AND (entity_id='storyboard.run' OR (state='completed' AND json_extract(prepared,'$.refinementAdoption.version')='storyboard-refinement-adoption/v1')) ORDER BY COALESCE((SELECT MAX(e.id) FROM events e WHERE e.job_id=jobs.id AND e.type='storyboard_refinement_adopted'),0) DESC,jobs.rowid DESC").map_err(|e|e.to_string())?;
    let rows = query
        .query_map([source_sha], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut origins = Vec::new();
    for row in rows {
        let (id, snapshot, result, prepared, stage) = row.map_err(|e| e.to_string())?;
        let snapshot: Value =
            serde_json::from_str(&snapshot).map_err(|_| revision_origin_error("原流程快照无效"))?;
        let mut result: Value =
            serde_json::from_str(&result).map_err(|_| revision_origin_error("原流程采用稿无效"))?;
        if stage=="storyboard.refine" {
            let prepared:Value=prepared.as_deref().map(serde_json::from_str).transpose()
                .map_err(|_|revision_origin_error("精修采用回执无效"))?.unwrap_or(Value::Null);
            let receipt=&prepared["refinementAdoption"];
            result=json!({"sourceSha256":result["sourceSha256"],"scope":"units","units":receipt["adoptedUnits"],
                "fantasyRealization":receipt["fantasyRealization"],"refinementCandidate":result,"refinementAdoption":receipt});
            if let Some(recovery)=prepared.get(pipeline::storyboard_refinement_identity::RECOVERY){result[pipeline::storyboard_refinement_identity::RECOVERY]=recovery.clone();}
        }
        if snapshot["runnerVersion"] == "desktop-runner/v1"
            && snapshot["input"]["source"]["sha256"] == source_sha
        {
            origins.push((id, snapshot, result));
        }
    }
    let mut wanted = HashSet::new();
    for id in &selected {
        let current = existing
            .iter()
            .find(|unit| s(unit, "id") == *id && unit["retired"] != true)
            .ok_or_else(|| revision_origin_error("所选单元不是当前正式稿"))?;
        if !input["selection"]["episodeIds"]
            .as_array()
            .is_some_and(|episodes| {
                episodes
                    .iter()
                    .any(|episode| episode == &current["episodeId"])
            })
        {
            return Err(revision_origin_error("所选单元不属于本次冻结集范围"));
        }
        let matches: Vec<_> = origins
            .iter()
            .filter(|(_, _, result)| {
                let episode = s(current, "episodeId");
                let published = result["scope"] == "units"
                    || result["updatedEpisodeIds"]
                        .as_array()
                        .is_some_and(|episodes| episodes.iter().any(|id| id == episode));
                published
                    && result["units"].as_array().is_some_and(|units| {
                        units.iter().any(|unit| {
                            unit["id"] == current["id"]
                                && adopted_unit_content(unit) == adopted_unit_content(current)
                        })
                    })
            })
            .collect();
        // A refinement adoption receipt freezes the full adopted episode,
        // including unchanged units. Its newest actual adoption owns the
        // continued context; older equal retained bodies remain historical.
        let refinement=matches.iter().find(|(_,snapshot,_)|snapshot["stage"]=="storyboard.refine");
        if matches.len() != 1 && refinement.is_none() {
            return Err(revision_origin_error(
                "当前单元没有唯一逐值相等的正式采用流程",
            ));
        }
        let origin=refinement.map(|entry|*entry).unwrap_or(matches[0]);
        wanted.insert((origin.0.clone(), s(current, "episodeId").to_owned()));
    }
    let mut wanted: Vec<_> = wanted.into_iter().collect();
    wanted.sort();
    let mut contexts: Vec<Value> = Vec::new();
    for (run, episode) in wanted {
        let (_, snapshot, output) = origins.iter().find(|origin| origin.0 == run).unwrap();
        let context = revision_context_for_origin(
            &connection,
            &run,
            snapshot,
            output,
            &episode,
            source_sha,
            existing,
        )?;
        if let Some(previous) = contexts
            .iter_mut()
            .find(|context| context["episodeId"] == episode)
        {
            if ["originWorkflowId", "planJobId", "planRequestHash", "plan"]
                .iter()
                .any(|field| previous[*field] != context[*field])
            {
                return Err(revision_origin_error("同集所选稿来自不同原规划合同"));
            }
            for field in ["unitBindings", "selectedFantasy", "fantasyRealization"] {
                let target = previous[field].as_array_mut().unwrap();
                for value in context[field].as_array().unwrap() {
                    if !target.contains(value) {
                        target.push(value.clone());
                    }
                }
            }
        } else {
            contexts.push(context);
        }
    }
    for context in &mut contexts {
        context["unitBindings"]
            .as_array_mut()
            .unwrap()
            .sort_by(|left, right| s(left, "unitId").cmp(s(right, "unitId")));
    }
    contexts.sort_by(|left, right| s(left, "episodeId").cmp(s(right, "episodeId")));
    for id in selected {
        if !contexts.iter().any(|context| {
            context["unitBindings"]
                .as_array()
                .is_some_and(|bindings| bindings.iter().any(|binding| binding["unitId"] == id))
        }) {
            return Err(revision_origin_error("所选单元未回绑完整采用版本"));
        }
    }
    Ok(contexts)
}
#[cfg(test)]
mod refinement_lineage_tests {
    use super::*;
    use crate::pipeline::storyboard_refinement::tests::{fixture,saved_candidate,Model};

    #[test]
    #[ignore = "explicit isolated database copy; no production writes, model or channel calls"]
    fn refinement_dispatch_isolated_saved_candidate_adopts_without_changing_original_evidence() {
        let root=PathBuf::from(std::env::var("DIRECTOR_REFINEMENT_ISOLATED_ADOPTION_ROOT").expect("isolated copy root required"));
        let marker:Value=serde_json::from_slice(&fs::read(root.join("isolated-refinement-adoption.json")).unwrap()).unwrap();
        assert_eq!(marker["purpose"],"isolated-refinement-adoption/v1");
        assert_eq!(fs::canonicalize(&root).unwrap(),fs::canonicalize(s(&marker,"isolatedRoot")).unwrap());
        assert_ne!(fs::canonicalize(&root).unwrap(),fs::canonicalize(s(&marker,"sourceRoot")).unwrap());
        let store=Store::open(root.clone()).unwrap();let run=s(&marker,"workflowId");let saved=store.job(run).unwrap();
        assert_eq!(saved["result"]["candidateSha256"],marker["candidateSha256"]);assert!(saved["prepared"].get("refinementAdoption").is_none());
        let frozen=&saved["snapshot"]["input"];let request=json!({"episodeId":frozen["episodeId"],"refinementScope":frozen["refinementScope"],"reviewNotes":frozen["reviewNotes"],"preserveNotes":frozen["preserveNotes"]});
        let mut old_input=refinement_input_with_capabilities(&store,&request,frozen.get("capabilities")).unwrap();pipeline::asset_use_binding::inherit(&mut old_input,frozen).unwrap();old_input["settings"]=frozen["settings"].clone();
        assert!(pipeline::adopt_storyboard_refinement(&old_input,&saved["result"]).is_err(),"old dispatch must reproduce the saved candidate rejection");
        let source=store.source().unwrap();let assets=store.values("assets").unwrap();let settings=store.settings().unwrap();
        let count_jobs=||store.conn().unwrap().query_row("SELECT COUNT(*) FROM jobs",[],|r|r.get::<_,i64>(0)).unwrap();let jobs=count_jobs();
        let engine=Engine::with_channels(root.join("isolated-engine-home"),Arc::new(crate::execution_policy::OfflineChannels)).unwrap();engine.register(&store).unwrap();
        let adoption=json!({"action":"storyboard.refinement.adopt","projectId":store.project().unwrap()["id"],"workflowId":run,"candidateSha256":marker["candidateSha256"]});
        engine.dispatch(adoption.clone()).unwrap();let adopted=store.values("units").unwrap();engine.dispatch(adoption).unwrap();
        assert_eq!(store.values("units").unwrap(),adopted);assert_eq!(store.source().unwrap(),source);assert_eq!(store.values("assets").unwrap(),assets);assert_eq!(store.settings().unwrap(),settings);assert_eq!(count_jobs(),jobs);
        let after=store.job(run).unwrap();assert_eq!(after["snapshot"],saved["snapshot"]);assert_eq!(after["result"],saved["result"]);assert_eq!(after["prepared"]["refinementAdoption"]["candidateSha256"],marker["candidateSha256"]);
        println!("isolated saved candidate adopted and replayed: newModelJobs=0 originalSnapshot=true rawCandidate=true source=true assets=true settings=true productionDatabaseWrites=0");
    }

    #[tokio::test]
    async fn refinement_dispatch_adoption_inherits_all_frozen_profiles_without_upgrading_legacy() {
        for marked in [false,true] {
            let (_directory,store,base,_origin)=fixture().await;
            let request=json!({"episodeId":base["episodeId"],"refinementScope":{},"reviewNotes":"审阅本集","preserveNotes":"保留原话"});
            let mut input=refinement_input_with_capabilities(&store,&request,Some(&base["capabilities"])).unwrap();
            if marked {
                input["storyboardSceneInputProfile"]=json!("unit-local-scene/v1");
                pipeline::storyboard_sound_events::freeze_new(&mut input).unwrap();
                pipeline::asset_use_binding::freeze_new(&mut input).unwrap();
            }
            let (run,candidate)=saved_candidate(&store,&input,"scattered").await;
            let source=store.source().unwrap();let settings=store.settings().unwrap();let raw=store.job(&run).unwrap()["result"].clone();
            let retained=input["existingUnits"][1].clone();
            let mut prior=refinement_input_with_capabilities(&store,&request,Some(&base["capabilities"])).unwrap();
            pipeline::asset_use_binding::inherit(&mut prior,&input).unwrap();prior["settings"]=input["settings"].clone();
            if marked {assert!(pipeline::adopt_storyboard_refinement(&prior,&candidate).unwrap_err().contains("上下文已变化"),"reproduce old dispatch omission");}
            let engine=Engine::with_channels(store.root.join("isolated-adopt-engine"),Arc::new(crate::execution_policy::OfflineChannels)).unwrap();engine.register(&store).unwrap();
            let result=engine.dispatch(json!({"action":"storyboard.refinement.adopt","projectId":store.project().unwrap()["id"],"workflowId":run,"candidateSha256":candidate["candidateSha256"]})).unwrap();
            assert!(result.is_object());
            let saved=store.job(&run).unwrap();assert_eq!(saved["result"],raw);
            assert_eq!(saved["prepared"]["refinementAdoption"]["candidateSha256"],candidate["candidateSha256"]);
            assert_eq!(store.get("units",s(&retained,"id")).unwrap(),retained);
            assert_eq!(store.source().unwrap(),source);assert_eq!(store.settings().unwrap(),settings);
            for field in ["storyboardSceneInputProfile","storyboardSoundEventProfile","assetUseBindingProfile"] {
                assert_eq!(saved["snapshot"]["input"].get(field),input.get(field));
                if !marked {assert!(saved["snapshot"]["input"].get(field).is_none());}
            }
        }
    }

    #[tokio::test]
    async fn delta_finisher_and_refinement_adoption_keep_the_original_plan_for_further_work() {
        let (_directory,store,mut input,origin)=fixture().await;
        let selected:Vec<_>=input["existingUnits"].as_array().unwrap().iter().map(|unit|unit["id"].clone()).collect();
        let original=accepted_storyboard_revision_contexts(&store,&input,&selected).unwrap();
        assert_eq!(original.len(),1);assert_eq!(original[0]["originWorkflowId"],origin);
        assert_eq!(original[0]["unitBindings"].as_array().unwrap().len(),3,"materialized delta finisher remains a proven adopted draft");
        input["revisionContexts"]=json!(original);
        let (run,candidate)=saved_candidate(&store,&input,"changed").await;
        store.publish_storyboard_refinement(&run,s(&candidate,"candidateSha256"),&input).unwrap();
        let receipt=store.job(&run).unwrap()["prepared"]["refinementAdoption"].clone();
        input["existingUnits"]=receipt["adoptedUnits"].clone();
        let ids:Vec<_>=input["existingUnits"].as_array().unwrap().iter().map(|unit|unit["id"].clone()).collect();
        let inherited=accepted_storyboard_revision_contexts(&store,&input,&ids).unwrap();
        assert_eq!(inherited.len(),1);assert_eq!(inherited[0]["originWorkflowId"],origin);
        assert_eq!(inherited[0]["planJobId"],candidate["acceptedContext"]["planJobId"]);
        assert_eq!(inherited[0]["planRequestHash"],candidate["acceptedContext"]["planRequestHash"]);
        assert_eq!(inherited[0]["unitBindings"].as_array().unwrap().len(),3);
        input["revisionContexts"]=json!(inherited);
        let mut executor=Model::default();executor.mode="changed";
        let next=pipeline::run_storyboard_refinement(&input,&executor,"offline-fixture-model","medium").await.unwrap();
        assert_eq!(next["state"],"candidate","second refinement uses the new adopted base with the original Planner");
        let local=accepted_storyboard_revision_contexts(&store,&input,&[ids[0].clone()]).unwrap();
        assert_eq!(local[0]["originWorkflowId"],origin,"local remake also inherits the same accepted planning contract");
        store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE type='text_received_raw' AND job_id IN (SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1)",[&run]).unwrap();
        assert!(accepted_storyboard_revision_contexts(&store,&input,&ids).is_err(),"a missing original raw does not fall back to an older equal retained body");
    }
}
fn check_standard_workflow_resume(creative_ready: bool, job: &Value) -> Result<()> {
    if !creative_ready && matches!(s(job, "entityId"), "storyboard.run" | "storyboard.refine") {
        return Err("STAGE3_ADAPTER_PENDING: 最新 3008 运行装配尚未接入；原工作流和预算保留，未重发。".into());
    }
    Ok(())
}

fn freeze_production_execution_settings(
    store: &Store,
    stage: &str,
    input: &mut Value,
    resolve: impl FnOnce(&Value) -> Result<Value>,
) -> Result<String> {
    // Bind the project settings that actually supplied this input, before a
    // potentially blocking preflight adds the observed execution identity.
    let saved_settings_sha256 = pipeline::digest(&input["settings"].to_string());
    let execution_settings = resolve(&input["settings"])?;
    if stage == "storyboard.refine"
        && saved_settings_sha256 != pipeline::digest(&store.settings()?.to_string())
    {
        return Err("项目保存设置在精修预检期间变化；未创建任务或派发请求，请刷新后重新开始".into());
    }
    input["settings"] = execution_settings;
    Ok(saved_settings_sha256)
}

#[cfg(test)]
fn refinement_input(store: &Store, request: &Value) -> Result<Value> {
    refinement_input_with_capabilities(store,request,None)
}
fn refinement_input_with_capabilities(store: &Store, request: &Value,capabilities:Option<&Value>) -> Result<Value> {
    let episode = s(request, "episodeId");
    if episode.is_empty() { return Err("请先选择要精修的单集".into()); }
    if store.series_deliveries()?.iter().any(|d|d["episodeId"]==episode) {
        return Err("新版分镜的整集精修入口尚未开放；保留原采用稿，不调用旧版规则".into());
    }
    let mut input = production_input_with_capabilities(store, &json!({"action":"storyboard.run","episodeIds":[episode]}),capabilities)?;
    let units: Vec<Value> = input["existingUnits"].as_array().into_iter().flatten()
        .filter(|unit| s(unit,"episodeId")==episode && unit["retired"]!=true).map(|unit|unit["id"].clone()).collect();
    if units.is_empty() { return Err("此集尚无已采用分镜，请先生成完整集稿".into()); }
    input["revisionContexts"] = json!(accepted_storyboard_revision_contexts(store,&input,&units)?);
    input["episodeId"] = json!(episode);
    input["refinementScope"] = request.get("refinementScope").cloned().unwrap_or(json!({}));
    input["reviewNotes"] = json!(s(request,"reviewNotes"));
    input["preserveNotes"] = json!(s(request,"preserveNotes"));
    input["productionDirection"] = input["foundation"]["productionDirection"].clone();
    refinement_identity::freeze(store,&mut input)?;
    Ok(input)
}
fn production_input(store: &Store, request: &Value) -> Result<Value> {
    production_input_with_capabilities(store,request,None)
}
fn production_input_with_capabilities(store: &Store, request: &Value,capabilities:Option<&Value>) -> Result<Value> {
    let mut request = request.clone();
    if let Some(selection) = request["options"]["selection"].as_object().cloned() {
        for (k, v) in selection {
            request[k] = v;
        }
    }
    if s(&request, "action") == "foundation.run" && request.get("episodeIds").is_none() {
        request["episodeIds"] = json!(store.source()?["episodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| e["id"].clone())
            .collect::<Vec<_>>());
    }
    let mut input =
        storyboard_input_inner_with_capabilities(store, &request, s(&request, "action") != "foundation.run",capabilities)?;
    if let Some(ids) = request.get("unitIds") {
        let ids = ids.as_array().ok_or("单元选择必须为数组")?;
        if ids.is_empty()
            || ids.iter().any(|id| {
                !input["existingUnits"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|u| u["id"] == *id)
            })
        {
            return Err("所选单元不存在于当前成果中".into());
        }
        input["selection"]["unitIds"] = json!(ids);
        input["revisionInstruction"] = request["revisionInstruction"].clone();
    }
    // Only freshly constructed whole-episode/partial-revision runs opt into the
    // compact transport. resume_production reads its saved snapshot directly.
    if s(&request, "action") == "storyboard.run" {
        pipeline::stage3_pack::mark_input(&mut input)?;
        input["storyboardCarrierProfile"] = json!("integer-video-units/v1");
        input["storyboardAssetInputProfile"] = json!("asset-production-card/v2");
        input[pipeline::REPETITION_TRANSPORT_PROFILE_FIELD] = json!(pipeline::REPETITION_TRANSPORT_PROFILE);
        input[pipeline::CONTEXT_INPUT_PROFILE_FIELD] = json!(pipeline::CONTEXT_INPUT_PROFILE);
        input[pipeline::storyboard_dependencies::PROFILE_FIELD] = json!(pipeline::storyboard_dependencies::PROFILE);
        if let Some(ids) = request["unitIds"].as_array() {
            input["revisionContexts"] =
                json!(accepted_storyboard_revision_contexts(store, &input, ids)?);
            input["storyboardRevisionProfile"] = json!(STORYBOARD_REVISION_PROFILE);
        }
    }
    Ok(input)
}
/// Reuse adopted episodes without copying role receipts or reserving new work.
/// Called under the production lease, after stable identity allocation.
fn prepare_series_continuation(store: &Store, input: &mut Value) -> Result<bool> {
    let reused=store.reusable_series_episode_ids(input)?;
    if reused.is_empty() {return Ok(false);}
    let pending:Vec<_>=input["selection"]["episodeIds"].as_array().ok_or("缺少制作集数")?.iter().filter(|id|!reused.iter().any(|e|id.as_str()==Some(e))).cloned().collect();
    store.event("","series_adopted_episodes_reused",&json!({"episodeIds":reused,"newEpisodeIds":pending,"newRoleReceiptsCreated":false}))?;
    if pending.is_empty(){return Ok(true);}
    let deliveries=store.series_deliveries()?;
    let prior=deliveries.iter().find(|d|d["episodeId"].as_str()==reused.last().map(String::as_str)).ok_or("复用集交付记录缺失")?;
    let tail=prior["bundle"]["draft"]["units"].as_array().and_then(|u|u.last()).ok_or("复用集缺少真实尾态")?;
    // Reuse already verified the adopted body. The next CAS guard also needs
    // the current persisted version; the immutable bundle has no DB version.
    input["previousBoundary"]=store.get("units",s(tail,"id"))?;
    input["selection"]["episodeIds"]=json!(pending);
    input["reusedEpisodeIds"]=json!(reused);
    Ok(false)
}

/// Only an actual new Stage3 writer receives this read-only projection. The
/// refinement reader and every saved recovery snapshot use production_input.
#[cfg(test)]
fn production_input_for_new_workflow(store: &Store, request: &Value) -> Result<Value> {
    production_input_for_new_workflow_with_capabilities(store,request,None)
}
fn production_input_for_new_workflow_with_capabilities(store: &Store, request: &Value,capabilities:Option<&Value>) -> Result<Value> {
    if request["unitIds"].as_array().is_some_and(|ids|!ids.is_empty()) {
        for id in request["unitIds"].as_array().unwrap() {
            if store.get("units",id.as_str().unwrap_or(""))?["timingProfile"]=="lingmo-v7-nanoseconds/v1" {return Err("新版单元必须通过绑定整集采用稿的精修入口，未调用旧版规则".into());}
        }
        return production_input_with_capabilities(store,request,capabilities);
    }
    let mut input = production_input_with_capabilities(store, request,capabilities)?;
    if s(request, "action") == "storyboard.run" {
        pipeline::asset_use_binding::freeze_new(&mut input)?;
        let projection = store.foundation_context_for_new_workflow()?;
        input["foundation"] = projection.foundation;
        input["foundationReasonProjection"] = projection.diagnostic;
        pipeline::asset_design_contract::apply_scope_reasons_to_new_asset_context(&mut input)?;
        // Only this new-work reservation changes the creative contract. The
        // shared input builder still supports original refinement/recovery.
        for key in ["storyboardRulePackProfile","storyboardCreativeProfile","storyboardCarrierProfile"] {
            input.as_object_mut().unwrap().remove(key);
        }
        input["productionScope"]=json!({"version":"desktop-production-scope/v1","status":"locked","sourceSha256":input["source"]["sha256"],"episodeIds":input["source"]["episodes"].as_array().into_iter().flatten().map(|e|e["id"].clone()).collect::<Vec<_>>(),"basis":"locked-desktop-source-scope;selection-is-execution-subset"});
        input["settings"]["confirmedDirection"]=input["foundation"]["productionDirection"].clone();
        if input["settings"]["fantasyMode"].is_null() {input["settings"]["fantasyMode"]=json!("off");}
        pipeline::series::mark_input(&mut input)?;
    }
    Ok(input)
}
#[cfg(test)]
fn storyboard_input(store: &Store, request: &Value) -> Result<Value> {
    storyboard_input_inner(store, request, true)
}
#[cfg(test)]
fn storyboard_input_inner(store: &Store, request: &Value, writing: bool) -> Result<Value> {
    storyboard_input_inner_with_capabilities(store,request,writing,None)
}
fn storyboard_input_inner_with_capabilities(store: &Store, request: &Value, writing: bool,capabilities:Option<&Value>) -> Result<Value> {
    let raw_source = store.source()?;
    let source: pipeline::LockedSource =
        serde_json::from_value(raw_source.clone()).map_err(|_| "请先导入并锁定剧本原文")?;
    let source = pipeline::upgrade_source_dialogues(&source)?;
    pipeline::validate_source(&source)?;
    if source.episodes.is_empty() {
        return Err("锁定原文尚未包含可制作的集数".into());
    }
    let selection = if let Some(ids) = request.get("episodeIds") {
        ids.as_array()
            .ok_or("分镜选择episodeIds必须为数组")?
            .clone()
    } else if let Some(id) = request
        .get("episodeId")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
    {
        vec![json!(id)]
    } else {
        return Err("请从锁定原文中选择要生成分镜的集数".into());
    };
    if selection.is_empty() {
        return Err("请至少选择一集生成分镜".into());
    }
    let mut selected = HashSet::new();
    for episode in &selection {
        let id = episode
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or("分镜选择必须使用有效集数ID")?;
        if !source.episodes.iter().any(|episode| episode.id == id) {
            return Err(format!("所选集数不在当前锁定原文中：{id}"));
        }
        if !selected.insert(id) {
            return Err("所选集数不能重复".into());
        }
    }
    // Source order, never checkbox click order, governs the continuity chain.
    let episodes: Vec<Value> = source
        .episodes
        .iter()
        .filter(|episode| selected.contains(episode.id.as_str()))
        .map(|episode| json!(episode.id))
        .collect();
    let first = source
        .episodes
        .iter()
        .position(|episode| selected.contains(episode.id.as_str()))
        .ok_or("制作集数为空")?;
    let preceding: HashSet<&str> = source.episodes[..first]
        .iter()
        .map(|episode| episode.id.as_str())
        .collect();
    let previous = store
        .values("units")?
        .into_iter()
        .filter(|unit| preceding.contains(s(unit, "episodeId")))
        .max_by_key(|unit| {
            unit["sourceRefs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|id| {
                    source
                        .rows
                        .iter()
                        .find(|row| row.id == id)
                        .map(|row| row.range.end)
                })
                .max()
                .unwrap_or(0)
        });
    let mut source_json = serde_json::to_value(source).map_err(|e| e.to_string())?;
    source_json["version"] = raw_source["version"].clone();
    Ok(
        json!({"source":source_json,"foundation":store.foundation()?,"assets":store.values("assets")?,"existingUnits":store.values("units")?,"settings":store.settings()?,"selection":{"episodeIds":episodes},"capabilities":if writing{match capabilities{Some(cap)=>cap.clone(),None=>self_capabilities(store)?}}else{Value::Null},"previousBoundary":previous}),
    )
}
pub fn merge(mut base: Value, changes: &Value) -> Value {
    if let (Some(b), Some(c)) = (base.as_object_mut(), changes.as_object()) {
        for (k, v) in c {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}
fn adopt_video(store: &Store, unit_id: &str, media_id: &str) -> Result<()> {
    let media = store.get("media", media_id)?;
    if s(&media, "kind") != "video" {
        return Err("只能采用视频".into());
    }
    if media["metadataVerified"].as_bool() != Some(true)
        || !media["streams"]
            .as_array()
            .map(|streams| {
                streams.iter().any(|stream| {
                    s(stream, "kind") == "video"
                        && stream["attachedPicture"].as_bool() != Some(true)
                })
            })
            .unwrap_or(false)
    {
        return Err("视频容器尚未可靠核验；请检查下载文件与媒体探测工具后再采用".into());
    }
    store.verify_media(&media)?;
    let mut c = store.conn()?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let version: i64 = tx
        .query_row("SELECT version FROM units WHERE id=?1", [unit_id], |r| {
            r.get(0)
        })
        .map_err(|_| "分镜单元不存在")?;
    let job_id:String=tx.query_row("SELECT j.id FROM job_media jm JOIN jobs j ON j.id=jm.job_id WHERE jm.media_id=?1 AND j.entity_id=?2 AND j.kind='video' AND json_extract(j.snapshot,'$.unitVersion')=?3 ORDER BY j.created_at DESC LIMIT 1",rusqlite::params![media_id,unit_id,version],|r|r.get(0)).map_err(|_|"视频结果不属于当前分镜版本；请查看对应历史任务")?;
    let adopted_at = now();
    tx.execute("INSERT INTO video_adoptions VALUES(?1,?2,?3) ON CONFLICT(unit_id) DO UPDATE SET media_id=excluded.media_id,created_at=excluded.created_at",rusqlite::params![unit_id,media_id,adopted_at]).map_err(|e|e.to_string())?;
    let proof = json!({"unitId":unit_id,"unitVersion":version,"mediaId":media_id,"jobId":job_id,"adoptedAt":adopted_at,"humanAdopted":true});
    tx.execute(
        "INSERT INTO metadata VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![format!("video_adoption:{unit_id}"), proof.to_string()],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    store.dirty()?;
    Ok(())
}
fn choose_files(r: &Value) -> Result<Value> {
    #[cfg(feature = "desktop")]
    {
        let dialog = rfd::FileDialog::new();
        let paths = if s(r, "kind") == "directory" {
            dialog.pick_folder().into_iter().collect::<Vec<_>>()
        } else if r["multiple"].as_bool() == Some(true) {
            dialog.pick_files().unwrap_or_default()
        } else {
            dialog.pick_file().into_iter().collect()
        };
        Ok(json!(paths))
    }
    #[cfg(not(feature = "desktop"))]
    {
        let _ = r;
        Err("文件选择需要桌面客户端；核心CLI请传绝对路径".into())
    }
}

#[cfg(test)]
mod local_video_prepare_tests {
    use super::*;
    use crate::providers::{
        DownloadReceipt, GenerationRequest, PreparedRequest, QueryResult, SubmitReceipt,
    };
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct LocalChannels {
        caps: AtomicUsize,
        prepares: AtomicUsize,
        submits: AtomicUsize,
        queries: AtomicUsize,
        capability_error: Mutex<Option<ProviderError>>,
    }
    impl LocalChannels {
        fn capability() -> Capabilities {
            serde_json::from_value(json!({"provider":"dreamina","version":"offline-local/v1","source":"offline fixture","checkedAt":now(),"models":[{"id":"offline-video","kind":"video","adapted":true,"minDuration":4,"maxDuration":15,"allowedDurations":[],"ratios":["16:9"],"resolutions":["720p"],"images":9,"videos":3,"audios":3,"totalReferences":12,"requiredImages":0,"raw":{}}],"canCancel":false,"canReconcileUnknown":false,"raw":{}})).unwrap()
        }
        fn assert_no_cloud_call(&self) {
            assert_eq!(self.submits.load(Ordering::SeqCst), 0);
            assert_eq!(self.queries.load(Ordering::SeqCst), 0);
        }
    }
    impl Channels for LocalChannels {
        fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> {
            self.caps.fetch_add(1, Ordering::SeqCst);
            match self.capability_error.lock().unwrap().clone() {
                Some(error) => Err(error),
                None => Ok(Self::capability()),
            }
        }
        fn prepare(
            &self,
            _: &ProviderConfig,
            r: &GenerationRequest,
        ) -> ProviderResult<PreparedRequest> {
            self.prepares.fetch_add(1, Ordering::SeqCst);
            Ok(PreparedRequest {
                provider: "dreamina".into(),
                request: r.clone(),
                capability_version: "offline-local/v1".into(),
                upload_receipts: vec![],
                wire: json!({"offlineLocal":true}),
                estimated_cost: None,
                prepared_at: now(),
            })
        }
        fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
            self.submits.fetch_add(1, Ordering::SeqCst);
            panic!("local preparation must never submit")
        }
        fn query(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            panic!("local preparation must never query or reconcile an existing task")
        }
        fn download(
            &self,
            _: &ProviderConfig,
            _: &str,
            _: &Path,
        ) -> ProviderResult<DownloadReceipt> {
            panic!("no download")
        }
        fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
            panic!("unknown submissions stay untouched")
        }
    }
    struct Fixture {
        _temp: tempfile::TempDir,
        engine: Arc<Engine>,
        store: Store,
        pid: String,
        channels: Arc<LocalChannels>,
        ids: Vec<String>,
    }
    impl Fixture {
        fn new(count: usize) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let channels = Arc::new(LocalChannels::default());
            let engine = Engine::build(temp.path().join("home"), channels.clone(), true, false).unwrap();
            let p = engine
                .dispatch(json!({"action":"projects.create","title":"离线本地准备回归"}))
                .unwrap();
            let pid = s(&p, "id").to_owned();
            let store = engine.store(&pid).unwrap();
            let cli = temp.path().join("offline-cli");
            fs::write(&cli, b"non-executable offline identity fixture").unwrap();
            store.save_settings(&json!({"dreaminaPath":cli,"videoProvider":"dreamina","videoModel":"offline-video","ratio":"16:9","videoResolution":"720p"})).unwrap();
            let source = serde_json::to_value(
                pipeline::prepare_source("第1集\n场景：门口\n甲：走吧。\n").unwrap(),
            )
            .unwrap();
            store.save_source(source.clone()).unwrap();
            let row = source["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| s(r, "kind") == "body")
                .unwrap();
            store.save_asset(json!({"id":"reference","variantId":"base","name":"甲","kind":"CH","prompt":"离线形象","sourceRefs":[row["id"]]})).unwrap();
            let image = temp.path().join("reference.png");
            image::RgbImage::from_pixel(4, 3, image::Rgb([10, 20, 30]))
                .save(&image)
                .unwrap();
            let media = store
                .import_media(&image, "image", Some("reference"), Some("base"))
                .unwrap();
            store.adopt("reference", "base", s(&media, "id")).unwrap();
            let ids = (0..count).map(|i| format!("u{i:02}")).collect::<Vec<_>>();
            for uid in &ids {
                store.save_unit(json!({"id":uid,"episodeId":source["episodes"][0]["id"],"sceneId":source["scenes"][0]["id"],"sourceRefs":[row["id"]],"duration":5,"prompt":"{{asset:reference:base}} 甲在门口说走吧。","assetRefs":[{"assetId":"reference","variantId":"base","order":0}]})).unwrap();
            }
            let config = engine.dreamina_config(&store).unwrap();
            engine.dreamina_sessions.seed_for_test(&config,json!({"status":"authenticated","ready":true,"identityVerified":true,"accountKey":"offline-official-account","checkedAt":now()})).unwrap();
            save_capability_cache(
                &store,
                &config,
                "offline-official-account",
                &capability_fingerprint(&config, "offline-official-account"),
                &LocalChannels::capability(),
            )
            .unwrap();
            Self {
                _temp: temp,
                engine,
                store,
                pid,
                channels,
                ids,
            }
        }
        fn request(&self) -> Value {
            json!({"action":"videos.prepare","projectId":self.pid,"provider":"dreamina","model":"offline-video","unitIds":self.ids,"ratio":"16:9","resolution":"720p"})
        }
        fn expire_capabilities(&self) {
            self.store
                .conn()
                .unwrap()
                .execute("UPDATE catalogs SET fetched_at='2000-01-01T00:00:00Z'", [])
                .unwrap();
        }
        fn wait_reserved(&self) {
            let start = Instant::now();
            let key = format!("video-prepare:{}:dreamina", self.pid);
            while !self.engine.active.lock().unwrap().contains(&key) {
                assert!(
                    start.elapsed() < Duration::from_secs(3),
                    "request must reserve the preparation intent"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        fn spawn(&self) -> std::thread::JoinHandle<Result<Value>> {
            let engine = self.engine.clone();
            let r = self.request();
            std::thread::spawn(move || engine.dispatch(r))
        }
    }
    #[test]
    fn verified_cache_prepares_57_while_image_channel_is_held_reusing_37_without_submission() {
        let f = Fixture::new(57);
        let mut initial = f.request();
        initial["unitIds"] = json!(f.ids[..37]);
        let first = f.engine.dispatch(initial).unwrap();
        let (config, account) = f.engine.config(&f.store, "dreamina").unwrap();
        let mut frozen = Vec::new();
        for entry in first["videoQueueReport"]["queued"].as_array().unwrap() {
            let key = s(entry, "jobId");
            tasks::prepare_only(&f.store, key, &config, &account, f.channels.as_ref()).unwrap();
            let job = f.store.job(key).unwrap();
            assert_eq!(job["status"], "ready");
            frozen.push(job);
        }
        let _image = f
            .engine
            .acquire_keys(vec![
                "channel:dreamina".into(),
                "job:offline-image-worker".into(),
            ])
            .unwrap();
        let result = f.engine.dispatch(f.request()).unwrap();
        let report = &result["videoQueueReport"];
        assert_eq!(report["reusedCount"], 37);
        assert_eq!(report["queuedCount"], 20);
        assert_eq!(report["blockedCount"], 0);
        assert_eq!(f.channels.caps.load(Ordering::SeqCst), 0);
        assert_eq!(f.channels.prepares.load(Ordering::SeqCst), 37);
        for job in frozen {
            assert_eq!(
                f.store.job(s(&job, "id")).unwrap(),
                job,
                "existing reviewed package remains immutable"
            );
        }
        let jobs = f.store.jobs().unwrap();
        assert_eq!(jobs.len(), 57);
        for job in &jobs {
            assert_eq!(job["snapshot"]["autoSubmit"], false);
            assert!(s(job, "providerTaskId").is_empty());
        }
        // The separately owned local worker may compile new packages later; even
        // a direct local compile contains no submit path while images hold the slot.
        for job in jobs.iter().filter(|job| job["status"] == "queued") {
            tasks::prepare_only(
                &f.store,
                s(job, "id"),
                &config,
                &account,
                f.channels.as_ref(),
            )
            .unwrap();
        }
        assert!(f
            .store
            .jobs()
            .unwrap()
            .iter()
            .all(|job| job["status"] == "ready"));
        assert_eq!(f.channels.prepares.load(Ordering::SeqCst), 57);
        f.channels.assert_no_cloud_call();
        let mut submit = f.request();
        submit["action"] = json!("videos.submit");
        assert!(f
            .engine
            .dispatch(submit)
            .unwrap_err()
            .contains("渠道正在检查"));
        assert!(f
            .store
            .jobs()
            .unwrap()
            .iter()
            .all(|job| job["snapshot"]["autoSubmit"] == false));
    }
    #[test]
    fn expired_capability_waits_same_click_then_refreshes_once_and_excludes_duplicate_click() {
        let f = Fixture::new(2);
        f.expire_capabilities();
        let held = f
            .engine
            .acquire_keys(vec!["channel:dreamina".into()])
            .unwrap();
        let pending = f.spawn();
        f.wait_reserved();
        assert!(f.store.jobs().unwrap().is_empty());
        assert_eq!(f.channels.caps.load(Ordering::SeqCst), 0);
        assert!(f
            .engine
            .dispatch(f.request())
            .unwrap_err()
            .contains("这项操作正在执行"));
        drop(held);
        let result = pending.join().unwrap().unwrap();
        assert_eq!(result["videoQueueReport"]["queuedCount"], 2);
        assert_eq!(
            result["videoQueueReport"]["preflight"]["source"],
            "refreshed"
        );
        assert_eq!(f.channels.caps.load(Ordering::SeqCst), 1);
        f.channels.assert_no_cloud_call();
    }
    #[test]
    fn cached_local_entry_requires_every_official_identity_and_capability_proof() {
        let f = Fixture::new(1);
        let config = f.engine.dreamina_config(&f.store).unwrap();
        let good = f.engine.dreamina_sessions.snapshot(&config).unwrap();
        for (key, value) in [
            ("status", json!("error")),
            ("ready", json!(false)),
            ("identityVerified", json!(false)),
            ("accountKey", json!("")),
            ("checkedAt", json!("2000-01-01T00:00:00Z")),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            f.engine
                .dreamina_sessions
                .seed_for_test(&config, bad)
                .unwrap();
            assert!(
                f.engine
                    .cached_dreamina_video_channel(&f.store, &f.request())
                    .unwrap()
                    .is_none(),
                "invalid {key} must not enter local fast path"
            );
        }
        f.engine
            .dreamina_sessions
            .seed_for_test(&config, good)
            .unwrap();
        let mut wrong = f.request();
        wrong["expectedAccountKey"] = json!("another-account");
        assert!(f
            .engine
            .cached_dreamina_video_channel(&f.store, &wrong)
            .is_err());
        f.store
            .conn()
            .unwrap()
            .execute(
                "UPDATE metadata SET value='{}' WHERE key='capability_binding:dreamina'",
                [],
            )
            .unwrap();
        assert!(f
            .engine
            .cached_dreamina_video_channel(&f.store, &f.request())
            .unwrap()
            .is_none());
        assert!(f.store.jobs().unwrap().is_empty());
        f.channels.assert_no_cloud_call();
    }
    #[test]
    fn held_channel_timeout_and_shutdown_create_no_jobs_or_preflight_calls() {
        let f = Fixture::new(1);
        f.expire_capabilities();
        let _held = f
            .engine
            .acquire_keys(vec!["channel:dreamina".into()])
            .unwrap();
        let start = Instant::now();
        let error = f
            .engine
            .prepare_dreamina_videos(&f.store, &f.request(), Duration::from_millis(80))
            .unwrap_err();
        assert!(error.contains("等待已达上限"));
        assert!(start.elapsed() < Duration::from_secs(3));
        let pending = f.spawn();
        f.wait_reserved();
        f.engine.stop.store(true, Ordering::SeqCst);
        assert!(pending.join().unwrap().unwrap_err().contains("正在退出"));
        assert!(f.store.jobs().unwrap().is_empty());
        assert_eq!(f.channels.caps.load(Ordering::SeqCst), 0);
        f.channels.assert_no_cloud_call();
    }
    #[test]
    fn held_channel_rejects_configuration_unit_reference_and_archive_drift_without_jobs() {
        for drift in [
            "config",
            "source",
            "unit",
            "reference",
            "archive",
            "account",
        ] {
            let f = Fixture::new(1);
            f.expire_capabilities();
            let _held = f
                .engine
                .acquire_keys(vec!["channel:dreamina".into()])
                .unwrap();
            let pending = f.spawn();
            f.wait_reserved();
            match drift {
                "config" => {
                    f.store
                        .save_settings(&json!({"providerTimeoutSeconds":60}))
                        .unwrap();
                }
                "source" => {
                    f.store
                        .save_source(
                            serde_json::to_value(
                                pipeline::prepare_source(
                                    "第1集\n场景：门口\n甲：走吧。\n乙：等等。\n",
                                )
                                .unwrap(),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                }
                "unit" => {
                    let mut unit = f.store.get("units", &f.ids[0]).unwrap();
                    unit["prompt"] = json!("更新后的离线提示词");
                    f.store.save_unit(unit).unwrap();
                }
                "reference" => {
                    let mut asset = f.store.get("assets", "reference").unwrap();
                    asset["prompt"] = json!("更新后的离线形象");
                    f.store.save_asset(asset).unwrap();
                }
                "archive" => {
                    let mut registry = f.engine.registry().unwrap();
                    registry[0]["archivedAt"] = json!(now());
                    atomic_json(&f.engine.home.join("projects.json"), &registry).unwrap();
                }
                "account" => {
                    let config = f.engine.dreamina_config(&f.store).unwrap();
                    f.engine.dreamina_sessions.seed_for_test(&config,json!({"status":"authenticated","ready":true,"identityVerified":true,"accountKey":"changed-offline-account","checkedAt":now()})).unwrap();
                }
                _ => unreachable!(),
            }
            let error = pending.join().unwrap().unwrap_err();
            assert!(
                error.contains("变化") || error.contains("归档"),
                "{drift}: {error}"
            );
            assert!(f.store.jobs().unwrap().is_empty(), "{drift}");
            assert_eq!(f.channels.caps.load(Ordering::SeqCst), 0);
            f.channels.assert_no_cloud_call();
        }
    }
    #[test]
    fn failed_readonly_refresh_preserves_original_known_and_unknown_jobs_without_submission() {
        for auth in [false, true] {
            let f = Fixture::new(1);
            f.expire_capabilities();
            let mut original = Vec::new();
            for (entity, state, remote) in [
                ("old-known", "processing", Some("original-cloud-id")),
                ("old-unknown", "submission_unknown", None),
            ] {
                let key = f
                    .store
                    .add_job(
                        "video",
                        entity,
                        "dreamina",
                        "offline-official-account",
                        &json!({"prompt":"original frozen prompt","autoSubmit":true}),
                        state,
                    )
                    .unwrap();
                let mut fields = json!({"receipt":{"original":true}});
                if let Some(remote) = remote {
                    fields["providerTaskId"] = json!(remote);
                }
                f.store.update_job(&key, state, &fields).unwrap();
                original.push(f.store.job(&key).unwrap());
            }
            let expected = if auth {
                ProviderError::auth()
            } else {
                ProviderError::new("OFFLINE_NETWORK", "readonly fixture transport failure")
            };
            *f.channels.capability_error.lock().unwrap() = Some(expected.clone());
            let error = f.engine.dispatch(f.request()).unwrap_err();
            assert_eq!(error, expected.message);
            assert_eq!(f.channels.caps.load(Ordering::SeqCst), 1);
            assert_eq!(f.store.jobs().unwrap().len(), 2);
            for job in original {
                assert_eq!(f.store.job(s(&job, "id")).unwrap(), job);
            }
            f.channels.assert_no_cloud_call();
        }
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;
    use crate::providers::{
        DownloadReceipt, GenerationRequest, PreparedRequest, QueryResult, SubmitReceipt,
    };
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct ImageQueueChannels {
        submits: AtomicUsize,
        queries: Mutex<Vec<String>>,
    }
    impl Channels for ImageQueueChannels {
        fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> {
            panic!("identity-only queue preflight must not inspect a model or generate")
        }
        fn prepare(
            &self,
            _: &ProviderConfig,
            _: &GenerationRequest,
        ) -> ProviderResult<PreparedRequest> {
            panic!("queue preflight must not upload or prepare cloud materials")
        }
        fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
            self.submits.fetch_add(1, Ordering::SeqCst);
            Err(ProviderError::new(
                "OFFLINE_ONLY",
                "no generation permitted",
            ))
        }
        fn query(&self, _: &ProviderConfig, task: &str) -> ProviderResult<QueryResult> {
            self.queries.lock().unwrap().push(task.to_owned());
            Ok(QueryResult {
                task_id: task.to_owned(),
                status: "processing".into(),
                result_urls: vec![],
                error: None,
                actual_cost: None,
                raw: json!({}),
            })
        }
        fn download(
            &self,
            _: &ProviderConfig,
            _: &str,
            _: &Path,
        ) -> ProviderResult<DownloadReceipt> {
            panic!("identity checks cannot download")
        }
        fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
            panic!("unknown submission must remain untouched")
        }
    }
    fn image_queue_fixture() -> (
        tempfile::TempDir,
        Store,
        Arc<Engine>,
        Arc<ImageQueueChannels>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store
            .create("offline image identity", "image-identity")
            .unwrap();
        store
            .save_settings(
                &json!({"dreaminaPath":"offline-original-cli","imageProductionPolicy":"automatic"}),
            )
            .unwrap();
        store.save_asset(json!({"id":"fresh-asset","variantId":"base","name":"离线形象","kind":"character","prompt":"离线形象提示词","preparation":{"mode":"IMAGE"}})).unwrap();
        let channels = Arc::new(ImageQueueChannels::default());
        let engine = Engine::build(temp.path().join("home"), channels.clone(), true, false).unwrap();
        (temp, store, engine, channels)
    }
    fn image_queue_request(assets: Value) -> Value {
        json!({"provider":"dreamina","model":"offline-image-model","assetIds":assets,"imageProductionPolicy":"automatic"})
    }
    #[test]
    fn images_queue_fresh_identity_succeeds_without_a_ready_cached_projection() {
        let (_temp, store, engine, channels) = image_queue_fixture();
        let report = engine
            .queue_images_with_account_check(
                &store,
                &image_queue_request(json!(["fresh-asset"])),
                |config, cached| {
                    assert_eq!(config.kind, "dreamina");
                    assert_eq!(cached, "dreamina:official:unverified");
                    Ok("verified-current-account".into())
                },
            )
            .unwrap();
        assert_eq!(report["queuedCount"], 1);
        let jobs = store.jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["accountId"], "verified-current-account");
        assert_eq!(jobs[0]["status"], "queued");
        assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
        assert!(channels.queries.lock().unwrap().is_empty());
    }
    fn assert_image_queue_identity_error(error: ProviderError) {
        let (_temp, store, engine, channels) = image_queue_fixture();
        let actual = engine
            .queue_images_with_account_check(
                &store,
                &image_queue_request(json!(["fresh-asset"])),
                |_, _| Err(error.clone()),
            )
            .unwrap_err();
        assert!(actual.contains(&error.code));
        assert!(actual.contains(&error.message));
        assert!(!actual.contains("刷新现有登录状态"));
        let raw: String = store.conn().unwrap().query_row("SELECT data FROM events WHERE type='image_account_preflight_failed' ORDER BY rowid DESC LIMIT 1", [], |row| row.get(0)).unwrap();
        let event: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(event["error"], serde_json::to_value(error).unwrap());
        assert_eq!(event["newGenerationQueued"], false);
        assert!(store.jobs().unwrap().is_empty());
        assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn images_queue_network_identity_failure_keeps_its_type_and_does_not_claim_logout() {
        assert_image_queue_identity_error(ProviderError::new(
            "CLI_NETWORK",
            "temporary offline network failure",
        ));
    }
    #[test]
    fn images_queue_real_authentication_failure_keeps_the_authentication_flag() {
        assert_image_queue_identity_error(ProviderError::auth());
    }
    #[test]
    fn images_queue_rejects_cli_configuration_changes_during_identity_inspection() {
        let (_temp, store, engine, channels) = image_queue_fixture();
        let error = engine
            .queue_images_with_account_check(
                &store,
                &image_queue_request(json!(["fresh-asset"])),
                |_, _| {
                    store
                        .save_settings(&json!({"dreaminaPath":"offline-replaced-cli"}))
                        .unwrap();
                    Ok("verified-current-account".into())
                },
            )
            .unwrap_err();
        assert!(error.contains("配置在账户核实时已变化"));
        assert!(store.jobs().unwrap().is_empty());
        assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn images_queue_changed_account_binds_new_jobs_but_preserves_known_and_unknown_identity() {
        let (_temp, store, engine, channels) = image_queue_fixture();
        let mut original = Vec::new();
        for (asset_id, state, remote) in [
            ("known-asset", "processing", Some("original-cloud-task")),
            ("unknown-asset", "submission_unknown", None),
        ] {
            store.save_asset(json!({"id":asset_id,"variantId":"base","name":asset_id,"prompt":"offline existing prompt"})).unwrap();
            let key = store
                .add_job(
                    "image",
                    asset_id,
                    "dreamina",
                    "original-account",
                    &json!({"requestId":format!("frozen-{asset_id}"),"prompt":"unchanged"}),
                    state,
                )
                .unwrap();
            let mut fields = json!({"receipt":{"original":asset_id},"result":{"preserved":true}});
            if let Some(remote) = remote {
                fields["providerTaskId"] = json!(remote);
            }
            store.update_job(&key, state, &fields).unwrap();
            original.push((key.clone(), store.job(&key).unwrap()));
        }
        let report = engine
            .queue_images_with_account_check(
                &store,
                &image_queue_request(json!(["fresh-asset", "known-asset", "unknown-asset"])),
                |_, _| Ok("newly-verified-account".into()),
            )
            .unwrap();
        assert_eq!(report["queuedCount"], 1);
        assert_eq!(report["reusedCount"], 2);
        assert_eq!(report["blockedCount"], 1);
        for (key, before) in &original {
            assert_eq!(
                store.job(key).unwrap(),
                *before,
                "existing task and receipt must not be rewritten"
            );
        }
        let jobs = store.jobs().unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(
            jobs.iter()
                .find(|job| job["entityId"] == "fresh-asset")
                .unwrap()["accountId"],
            "newly-verified-account"
        );
        let config = engine.dreamina_config(&store).unwrap();
        assert!(tasks::resume_job(
            &store,
            &original[0].0,
            &config,
            "newly-verified-account",
            channels.as_ref(),
            false
        )
        .is_err());
        assert!(channels.queries.lock().unwrap().is_empty());
        tasks::resume_job(
            &store,
            &original[0].0,
            &config,
            "original-account",
            channels.as_ref(),
            false,
        )
        .unwrap();
        assert_eq!(
            *channels.queries.lock().unwrap(),
            vec!["original-cloud-task"]
        );
        assert_eq!(
            store.job(&original[0].0).unwrap()["receipt"],
            original[0].1["receipt"]
        );
        assert_eq!(store.job(&original[1].0).unwrap(), original[1].1);
        assert_eq!(channels.submits.load(Ordering::SeqCst), 0);
    }
    fn known_identity_fixture() -> (tempfile::TempDir, Store, String) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store
            .create("offline identity recovery", "identity")
            .unwrap();
        let key=store.add_job("image","asset","dreamina","original-account",&json!({"requestId":"frozen-request","prompt":"unchanged","references":[{"mediaId":"frozen-media"}]}),"processing").unwrap();
        store.update_job(&key,"processing",&json!({"providerTaskId":"original-cloud-task","prepared":{"original":true},"receipt":{"taskId":"original-cloud-task"},"result":{"preserved":true}})).unwrap();
        (temp, store, key)
    }
    #[test]
    fn transient_identity_checks_retry_only_the_known_id_with_a_five_attempt_bound() {
        let (_temp, store, key) = known_identity_fixture();
        let before = store.job(&key).unwrap();
        for attempt in 1..=5 {
            Engine::account_preflight_failed(
                &store,
                &key,
                &ProviderError::new("CLI_NETWORK", "safe network failure"),
            )
            .unwrap();
            let job = store.job(&key).unwrap();
            assert_eq!(
                job["status"],
                if attempt == 5 {
                    "recovery_paused"
                } else {
                    "query_failed"
                }
            );
            assert_eq!(job["queryAttempts"], attempt);
            assert_eq!(job["providerTaskId"], "original-cloud-task");
            for field in [
                "snapshot",
                "prepared",
                "receipt",
                "result",
                "accountId",
                "downloadAttempts",
            ] {
                assert_eq!(
                    job[field], before[field],
                    "identity inspection must preserve {field}"
                );
            }
            assert_eq!(
                store.jobs().unwrap().len(),
                1,
                "identity checks cannot create a generation job"
            );
        }
        assert!(!store
            .pending_jobs()
            .unwrap()
            .iter()
            .any(|job| job["id"] == key));
    }
    #[test]
    fn genuine_identity_authentication_failure_pauses_without_changing_cloud_materials() {
        let (_temp, store, key) = known_identity_fixture();
        let before = store.job(&key).unwrap();
        Engine::account_preflight_failed(&store, &key, &ProviderError::auth()).unwrap();
        let job = store.job(&key).unwrap();
        assert_eq!(job["status"], "auth_required");
        assert_eq!(job["queryAttempts"], 0);
        for field in [
            "snapshot",
            "prepared",
            "receipt",
            "result",
            "accountId",
            "providerTaskId",
        ] {
            assert_eq!(job[field], before[field]);
        }
    }
    #[test]
    fn identity_errors_never_release_unknown_submissions_or_auto_retry_generation() {
        let (_temp, store, _known) = known_identity_fixture();
        let key = store
            .add_job(
                "image",
                "unknown",
                "dreamina",
                "original-account",
                &json!({"requestId":"uncertain-original"}),
                "submission_unknown",
            )
            .unwrap();
        store.update_job(&key,"submission_unknown",&json!({"prepared":{"upload":"original"},"receipt":{"opaque":"original"},"result":{"raw":"original"}})).unwrap();
        let before = store.job(&key).unwrap();
        for error in [
            ProviderError::new("CLI_TIMEOUT", "temporary"),
            ProviderError::auth(),
            ProviderError::new("CLIENT_CLOSING", "closing"),
        ] {
            Engine::account_preflight_failed(&store, &key, &error).unwrap();
            let job = store.job(&key).unwrap();
            assert_eq!(job["status"], "submission_unknown");
            assert_eq!(job["queryAttempts"], 0);
            for field in [
                "snapshot",
                "prepared",
                "receipt",
                "result",
                "accountId",
                "providerTaskId",
            ] {
                assert_eq!(job[field], before[field]);
            }
        }
        let queued = store
            .add_job(
                "image",
                "unsubmitted",
                "dreamina",
                "original-account",
                &json!({"requestId":"not-submitted"}),
                "queued",
            )
            .unwrap();
        Engine::account_preflight_failed(
            &store,
            &queued,
            &ProviderError::new("CLI_NETWORK", "temporary"),
        )
        .unwrap();
        let job = store.job(&queued).unwrap();
        assert_eq!(job["status"], "failed");
        assert!(job["providerTaskId"].is_null());
        assert!(job["error"].as_str().unwrap().contains("尚未调用生成"));
        assert!(!store
            .pending_jobs()
            .unwrap()
            .iter()
            .any(|job| job["id"] == key || job["id"] == queued));
        assert_eq!(store.jobs().unwrap().len(), 3);
    }
    #[test]
    fn closing_identity_inspection_preserves_known_id_and_downloaded_media_results() {
        let (temp, store, key) = known_identity_fixture();
        Engine::account_preflight_failed(
            &store,
            &key,
            &ProviderError::new("CLIENT_CLOSING", "closing"),
        )
        .unwrap();
        let job = store.job(&key).unwrap();
        assert_eq!(job["status"], "submitted");
        assert_eq!(job["providerTaskId"], "original-cloud-task");
        assert_eq!(job["queryAttempts"], 0);
        let path = temp.path().join("real.png");
        image::RgbaImage::new(2, 2).save(&path).unwrap();
        let media = store.import_media(&path, "image", None, None).unwrap();
        store.attach_result(&key, s(&media, "id")).unwrap();
        store
            .update_job(
                &key,
                "downloaded",
                &json!({"result":{"mediaIds":[media["id"]]}}),
            )
            .unwrap();
        let before = store.job(&key).unwrap();
        for error in [
            ProviderError::new("CLI_QUERY_FAILED", "temporary"),
            ProviderError::auth(),
        ] {
            Engine::account_preflight_failed(&store, &key, &error).unwrap();
            assert_eq!(
                store.job(&key).unwrap(),
                before,
                "a failed identity recheck cannot demote a local result"
            );
        }
    }
    fn authenticated_snapshot() -> Value {
        json!({"status":"authenticated","ready":true,"identityVerified":true,"accountKey":"offline-account","checkedAt":now(),"credits":{"total":10}})
    }
    #[test]
    fn model_refresh_reuses_a_fresh_verified_account_without_rechecking_credits() {
        let snapshot = authenticated_snapshot();
        let account = model_refresh_account(snapshot.clone(), false, || {
            panic!("fresh model inspection must not run another account/credit query")
        })
        .unwrap();
        assert_eq!(account, snapshot);
    }
    #[test]
    fn model_refresh_checks_stale_unverified_or_different_config_accounts() {
        let fresh = authenticated_snapshot();
        let mut expired = fresh.clone();
        expired["checkedAt"] =
            json!((chrono::Utc::now() - chrono::Duration::seconds(61)).to_rfc3339());
        let mut future = fresh.clone();
        future["checkedAt"] =
            json!((chrono::Utc::now() + chrono::Duration::seconds(61)).to_rfc3339());
        let mut unverified = fresh.clone();
        unverified["identityVerified"] = json!(false);
        let mut missing_identity = fresh.clone();
        missing_identity["accountKey"] = json!("");
        // A different configured CLI has no matching entry; snapshot lookup is
        // intentionally performed by providers.refresh before this selection.
        let sessions = DreaminaSessions::default();
        let other_config: ProviderConfig = serde_json::from_value(
            json!({"kind":"dreamina","cliPath":"offline-different-cli","timeoutSeconds":120}),
        )
        .unwrap();
        let other_snapshot = sessions.snapshot(&other_config).unwrap();
        let mut calls = 0;
        for (snapshot, due) in [
            (fresh, true),
            (expired, false),
            (future, false),
            (unverified, false),
            (missing_identity, false),
            (other_snapshot, false),
        ] {
            let result = model_refresh_account(snapshot, due, || {
                calls += 1;
                Ok(json!({"status":"checked-now"}))
            })
            .unwrap();
            assert_eq!(result["status"], "checked-now");
        }
        assert_eq!(calls, 6);
    }
    #[test]
    fn model_refresh_preserves_the_existing_authorization_poll_path() {
        let pending = json!({"status":"authorizing","ready":false,"identityVerified":false,"checkedAt":now()});
        let result = model_refresh_account(pending.clone(), false, || Ok(pending.clone())).unwrap();
        assert_eq!(result, pending);
    }
    #[test]
    fn auth_preflight_failure_preserves_unknown_submit_until_explicit_reconciliation() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store.create("验证", "project").unwrap();
        let unknown = store
            .add_job(
                "image",
                "asset",
                "dreamina",
                "original",
                &json!({"requestId":"unknown-request"}),
                "submission_unknown",
            )
            .unwrap();
        Engine::pause_for_auth(&store, &unknown, "登录已过期").unwrap();
        let job = store.job(&unknown).unwrap();
        assert_eq!(job["status"], "submission_unknown");
        assert_eq!(job["snapshot"]["requestId"], "unknown-request");
        let engine =
            Engine::with_channels(temp.path().join("home"), Arc::new(RealChannels)).unwrap();
        engine
            .resume_authenticated_dreamina(
                &store,
                &json!({"ready":true,"identityVerified":true,"accountKey":"original"}),
            )
            .unwrap();
        assert_eq!(store.job(&unknown).unwrap()["status"], "submission_unknown");
        assert!(store.pending_jobs().unwrap().is_empty());
    }
}

#[cfg(test)]
#[path = "engine_repair_tests.rs"]
mod repair_tests;

#[cfg(test)]
#[path = "image_scheduler_tests.rs"]
mod image_scheduler_tests;
#[cfg(test)]
#[path = "chrome_queue_scheduler_tests.rs"]
mod chrome_queue_scheduler_tests;
#[cfg(test)]
#[path = "chrome_legacy_binding_tests.rs"]
mod chrome_legacy_binding_tests;
#[cfg(test)]
#[path = "chrome_cleanup_scheduler_tests.rs"]
mod chrome_cleanup_scheduler_tests;

#[cfg(test)]
#[path = "project_lifecycle_tests.rs"]
mod project_lifecycle_tests;
#[cfg(test)]
#[path = "task_record_tests.rs"]
mod task_record_tests;

#[cfg(test)]
#[path = "engine_refinement_cas_tests.rs"]
mod refinement_cas_tests;
#[cfg(test)]
#[path = "engine_series_start_tests.rs"]
mod series_start_tests;
#[cfg(test)]
#[path = "engine_cli_unknown_tests.rs"]
mod cli_unknown_tests;

#[cfg(test)]
mod production_input_transport_tests {
    use super::*;

    #[test]
    fn new_workflow_reason_projection_reaches_flat_stage3_cards_without_mutating_old_contexts() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store.create("synthetic projection", "fixture").unwrap();
        let source = store.save_source(json!(pipeline::prepare_source("第一集\n场景：门口\n甲：开门。\n").unwrap())).unwrap();
        let place = json!({"episodeId":source["episodes"][0]["id"],"sceneId":source["scenes"][0]["id"]});
        store.save_asset(json!({"id":"p-base","baseAssetId":"p","identityId":"p","variantId":"base","name":"甲","kind":"CH","sourceRefs":[source["rows"][2]["id"]],"scope":{"kind":"SCENES","scenes":[place],"reason":null},"applicability":{"episodes":[source["episodes"][0]["id"]],"scenes":[source["scenes"][0]["id"]]}})).unwrap();
        store.save_foundation(json!({"schemaVersion":"foundation/v1","sourceSha256":source["sha256"],"overview":"","direction":"","episodes":[],"facts":[],"issues":[],"assets":[{"id":"p","name":"甲","kind":"CH","description":"主体制作说明","aliases":[],"sourceRefs":[source["rows"][2]["id"]],"variants":[{"id":"base","assetRowId":"p-base","name":"基础","description":"","sourceVariantId":null,"sourceRefs":[source["rows"][2]["id"]],"applicability":[place],"scope":{"kind":"SCENES","scenes":[place],"reason":"本场明确出场。"}}]}]})).unwrap();
        store.ensure_series_asset_identities().unwrap();
        let raw_assets = store.values("assets").unwrap();
        let raw_foundation = store.foundation().unwrap();
        let request = json!({"action":"storyboard.run","episodeId":source["episodes"][0]["id"]});
        let old = production_input(&store, &request).unwrap();
        let bytes = old.to_string();
        let job = store.add_job("workflow","storyboard.run","local","local",&json!({"input":old}),"interrupted").unwrap();
        let new = production_input_for_new_workflow(&store, &request).unwrap();
        assert_eq!(new["assets"][0]["scope"]["reason"], "本场明确出场。");
        assert!(new["foundationReasonProjection"].is_object());
        assert_eq!(new["foundationScopeReasonBindings"]["p-base"]["variantId"], "base");
        assert_eq!(store.values("assets").unwrap(), raw_assets);
        assert_eq!(store.foundation().unwrap(), raw_foundation);
        assert_eq!(store.job(&job).unwrap()["snapshot"]["input"].to_string(), bytes);
        let foundation = production_input_for_new_workflow(&store,&json!({"action":"foundation.run","episodeId":source["episodes"][0]["id"]})).unwrap();
        assert!(foundation.get("foundationReasonProjection").is_none());
        assert!(foundation["assets"][0]["scope"]["reason"].is_null());
    }
    #[test]
    fn fresh_storyboard_whole_and_partial_runs_opt_in_without_rewriting_legacy_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store.create("offline transport marker", "project").unwrap();
        let source = pipeline::prepare_source("第1集\n场景：门口\n甲：走吧。\n").unwrap();
        store.save_source(json!(source)).unwrap();
        let episode = &source.episodes[0].id;
        let legacy = storyboard_input(&store, &json!({"episodeId":episode})).unwrap();
        assert!(legacy.get("storyboardCreativeProfile").is_none());
        assert!(legacy.get("storyboardAssetInputProfile").is_none());
        assert!(legacy.get(pipeline::REPETITION_TRANSPORT_PROFILE_FIELD).is_none());
        assert!(legacy
            .get(pipeline::storyboard_dependencies::PROFILE_FIELD)
            .is_none());
        let original_bytes = serde_json::to_vec(&legacy).unwrap();
        let key=store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":legacy}),"interrupted").unwrap();
        let whole = production_input(
            &store,
            &json!({"action":"storyboard.run","episodeId":episode}),
        )
        .unwrap();
        assert_eq!(
            whole["storyboardAssetInputProfile"],
            "asset-production-card/v2"
        );
        assert_eq!(
            whole["storyboardCreativeProfile"],
            "original-production-contract/v1"
        );
        assert_eq!(whole["storyboardCarrierProfile"], "integer-video-units/v1");
        assert_eq!(whole[pipeline::REPETITION_TRANSPORT_PROFILE_FIELD], pipeline::REPETITION_TRANSPORT_PROFILE);
        assert_eq!(
            whole[pipeline::storyboard_dependencies::PROFILE_FIELD],
            pipeline::storyboard_dependencies::PROFILE
        );
        store.save_unit(json!({"id":"existing-unit","episodeId":episode,"prompt":"offline","assetRefs":[],"sourceRefs":[]})).unwrap();
        let jobs_before: i64 = store
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        let missing_origin=production_input(&store,&json!({"action":"storyboard.run","episodeId":episode,"unitIds":["existing-unit"],"revisionInstruction":"离线局部修订"})).unwrap_err();
        assert!(
            missing_origin.contains("项目没有正式采用流程节点")
                && missing_origin.contains("未调用模型"),
            "a manually seeded unit has no accepted Planner provenance"
        );
        let jobs_after: i64 = store
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            jobs_after, jobs_before,
            "missing provenance cannot create work"
        );
        let retained = store.job(&key).unwrap()["snapshot"]["input"].clone();
        assert_eq!(
            serde_json::to_vec(&retained).unwrap(),
            original_bytes,
            "new construction cannot alter a saved legacy workflow or its text cache identity"
        );
        let foundation = production_input(
            &store,
            &json!({"action":"foundation.run","episodeId":episode}),
        )
        .unwrap();
        assert!(foundation.get("storyboardAssetInputProfile").is_none());
        assert!(foundation.get(pipeline::REPETITION_TRANSPORT_PROFILE_FIELD).is_none());
        assert!(foundation
            .get(pipeline::storyboard_dependencies::PROFILE_FIELD)
            .is_none());
    }
}

/// Freeze the requested backend before creating workflow jobs; no implicit fallback.
async fn resolve_text_execution_settings(settings:&Value)->Result<Value> {
    match s(settings,"textProvider") {
        "codex_sdk" => crate::text_sdk::resolve_for_workflow(settings).await,
        "" | "codex" | "api" => crate::text_cli::resolve_settings(settings).await,
        _ => Err("UNKNOWN_TEXT_BACKEND: 未知文本入口，不自动改用 CLI 或 API".into()),
    }
}

#[cfg(test)]
#[path = "readiness_engine_tests.rs"]
mod readiness_engine_tests;

#[cfg(test)]
#[path = "stage2_frozen_input_tests.rs"]
mod stage2_frozen_input_tests;

#[cfg(test)]
mod joint_unknown_boundary_tests {
    use super::*;

    fn sources() -> (Value, Value) {
        let next = pipeline::prepare_source("第一集\n场景：门口\n人物：林川、周航\n林川&周航：同声。\n第二集\n场景：院内\n周航：走吧。\n").unwrap();
        assert_eq!(next.parser_version,"source-parser/v8");
        let mut old = json!(next); old["parserVersion"] = json!("source-parser/v7");
        for row in old["rows"].as_array_mut().unwrap() {
            if row["dialogue"]["speakers"].is_array() { row["dialogue"] = Value::Null; }
        }
        pipeline::validate_source(&serde_json::from_value(old.clone()).unwrap()).unwrap();
        let reparsed = pipeline::reparse_source(&serde_json::from_value(old.clone()).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&reparsed).unwrap(),json!(next));
        let mut new=json!(reparsed); old["version"]=json!(1);new["version"]=json!(2);
        (old,new)
    }
    fn input(source: &Value, episode: usize) -> Value {
        let mut value=json!({"source":source,"selection":{"episodeIds":[source["episodes"][episode]["id"]]}});
        pipeline::asset_coverage::freeze_new(&mut value).unwrap();value
    }
    fn job(stage: &str, input: &Value, state: &str, unknown: bool) -> Value {
        json!({"id":"original","kind":"workflow","entityId":stage,"status":state,"submissionUnknown":unknown,
            "snapshot":{"runnerVersion":"desktop-runner/v1","stage":stage,"sourceVersion":input["source"]["version"],"input":input},"prepared":{"publishedEpisodes":{}}})
    }
    #[test]
    fn joint_unknown_parser_upgrade_and_coverage_cannot_cross_frozen_source_scope() {
        let (old,new)=sources();let original=input(&old,0);let current=input(&new,0);
        assert_eq!(old["sha256"],new["sha256"]);assert_ne!(storyboard_source_identity(&old).unwrap(),storyboard_source_identity(&new).unwrap());
        for stage in ["foundation.run","storyboard.run"] {
            for (state,unknown) in [("partially_blocked",true),("completed",true),("submission_unknown",false),("closed_by_user",true)] {
                let record=job(stage,&original,state,unknown);let before=record.to_string();
                if stage=="foundation.run" {assert!(matches!(foundation_start_boundary(&[record.clone()],&current).unwrap(),FoundationStartBoundary::Hold(reason) if reason.contains("未知提交")));}
                else {assert!(matches!(storyboard_start_boundary(&[record.clone()],&current).unwrap(),StoryboardStartBoundary::Hold(reason) if reason.contains("未知提交")));}
                assert_eq!(record.to_string(),before);
            }
            for state in ["completed","failed"] {
                let record=job(stage,&original,state,false);
                if stage=="foundation.run" {assert_eq!(foundation_start_boundary(&[record],&current).unwrap(),FoundationStartBoundary::New);}
                else {assert_eq!(storyboard_start_boundary(&[record],&current).unwrap(),StoryboardStartBoundary::New);}
            }
            let unrelated=input(&new,1);let record=job(stage,&original,"partially_blocked",true);
            if stage=="foundation.run" {assert_eq!(foundation_start_boundary(&[record],&unrelated).unwrap(),FoundationStartBoundary::New);}
            else {assert_eq!(storyboard_start_boundary(&[record],&unrelated).unwrap(),StoryboardStartBoundary::New);}
        }
        let mut old_unit=original.clone();old_unit["selection"]["unitIds"]=json!(["u1"]);
        let mut other_unit=current.clone();other_unit["selection"]["unitIds"]=json!(["u2"]);
        assert_eq!(storyboard_start_boundary(&[job("storyboard.run",&old_unit,"partially_blocked",true)],&other_unit).unwrap(),StoryboardStartBoundary::New);
        other_unit["selection"]["unitIds"]=json!(["u1"]);
        assert!(matches!(storyboard_start_boundary(&[job("storyboard.run",&old_unit,"partially_blocked",true)],&other_unit).unwrap(),StoryboardStartBoundary::Hold(_)));
        let changed=json!(pipeline::prepare_source("第一集\n场景：别处\n甲：真正不同的原文。\n").unwrap());let changed=input(&changed,0);
        assert_eq!(storyboard_start_boundary(&[job("storyboard.run",&original,"partially_blocked",true)],&changed).unwrap(),StoryboardStartBoundary::New);
        assert_eq!(foundation_start_boundary(&[job("foundation.run",&original,"partially_blocked",true)],&changed).unwrap(),FoundationStartBoundary::New);
    }
    #[test]
    fn joint_unknown_real_start_keeps_jobs_receipts_budgets_and_zero_reservation_after_reparse() {
        for stage in ["foundation.run","storyboard.run"] {
            let temp=tempfile::tempdir().unwrap();let engine=Engine::new_offline_verification(temp.path().join("isolated-app")).unwrap();
            let store=Store::open(temp.path().join("project")).unwrap();store.create("synthetic unknown parser migration","synthetic").unwrap();store.ensure_workflow_storage().unwrap();
            let (old,new)=sources();let old=store.save_source(old).unwrap();
            let original=input(&old,0);let mut snapshot=job(stage,&original,"partially_blocked",true)["snapshot"].clone();
            snapshot["input"].as_object_mut().unwrap().remove(pipeline::asset_coverage::FIELD);
            let run=store.add_job("workflow",stage,"local","local",&snapshot,"partially_blocked").unwrap();
            let child=store.add_job("text","original-node","codex_sdk","synthetic",&json!({"originalFrozenRequest":true}),"submission_unknown").unwrap();
            store.update_job(&child,"submission_unknown",&json!({"receipt":{"attemptId":"original-attempt","state":"submission_unknown"},"result":{"retainedRaw":"synthetic original fragment"}})).unwrap();
            store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,result,error,updated_at) VALUES(?1,'original-node',?2,'original-hash','blocked',?3,'{\"revisionBudgetUsed\":1}','original unknown',?4)",rusqlite::params![run,stage,child,now()]).unwrap();
            let current=store.save_reparsed_source_if_current(new,old["version"].as_i64().unwrap()).unwrap();
            let counts=||store.conn().unwrap().query_row("SELECT (SELECT COUNT(*) FROM jobs),(SELECT COUNT(*) FROM events),(SELECT COUNT(*) FROM workflow_steps)",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap();
            let before=counts();let parent=store.job(&run).unwrap();let original_child=store.job(&child).unwrap();
            let step:String=store.conn().unwrap().query_row("SELECT result FROM workflow_steps WHERE workflow_id=?1",[&run],|r|r.get(0)).unwrap();
            let error=engine.spawn_production(store.clone(),json!({"action":stage,"episodeIds":[current["episodes"][0]["id"]]})).unwrap_err();
            assert!(error.contains("未知提交"),"{stage}: {error}");
            assert_eq!(counts(),before,"the real engine must hold before reserving a workflow, preflight or model node");
            assert_eq!(store.job(&run).unwrap(),parent);assert_eq!(store.job(&child).unwrap(),original_child);
            assert_eq!(store.conn().unwrap().query_row("SELECT result FROM workflow_steps WHERE workflow_id=?1",[&run],|r|r.get::<_,String>(0)).unwrap(),step);
        }
    }
}
