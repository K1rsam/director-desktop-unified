//! Channel adapters. No adapter retries a generation request or writes project state.
//! The caller persists its submit intent BEFORE `submit`, and the receipt immediately after.
mod bh;
mod chrome;
mod dreamina;
mod cli_observation;
mod wire;
#[cfg(test)]
mod wire_tests;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ACTIVE_CLI: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Call before desktop exit to stop locally owned CLI processes. Never cancels cloud tasks.
pub fn shutdown() {
    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::SeqCst);
}
/// After `shutdown`, wait until owned CLI children have been killed and joined.
/// Returns false on a bounded timeout; the caller can keep the window in its closing state.
pub fn await_local_shutdown(timeout_seconds: u64) -> bool {
    let start = std::time::Instant::now();
    while ACTIVE_CLI.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        if start.elapsed() > std::time::Duration::from_secs(timeout_seconds) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    true
}
struct CliGuard;
impl CliGuard {
    fn enter() -> Self {
        ACTIVE_CLI.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}
impl Drop for CliGuard {
    fn drop(&mut self) {
        ACTIVE_CLI.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
fn shutting_down() -> bool {
    SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst)
}

pub type ProviderResult<T> = Result<T, ProviderError>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub kind: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub credential_env: Option<String>,
    #[serde(default)]
    pub cli_path: Option<PathBuf>,
    #[serde(default)]
    pub bridge_token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome_binding: Option<crate::chrome_setup::BrowserBinding>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Reserved for a future reviewed registry. It does not bypass built-in wire adaptation.
    #[serde(default)]
    pub adapted_models: Vec<String>,
}
fn default_timeout() -> u64 {
    120
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaReference {
    pub media_id: String,
    pub kind: String,
    pub path: PathBuf,
    pub sha256: String,
    pub order: u32,
    #[serde(default)]
    pub usage: String,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationRequest {
    pub request_id: String,
    pub kind: String,
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub duration: Option<u32>,
    pub ratio: String,
    pub resolution: String,
    #[serde(default)]
    pub references: Vec<MediaReference>,
    #[serde(default = "empty_object")]
    pub options: Value,
}
fn empty_object() -> Value {
    json!({})
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapability {
    pub id: String,
    pub kind: String,
    pub adapted: bool,
    pub min_duration: Option<u32>,
    pub max_duration: Option<u32>,
    #[serde(default)]
    pub allowed_durations: Vec<u32>,
    #[serde(default)]
    pub ratios: Vec<String>,
    #[serde(default)]
    pub resolutions: Vec<String>,
    pub images: usize,
    pub videos: usize,
    pub audios: usize,
    pub total_references: Option<usize>,
    pub required_images: usize,
    pub price: Option<f64>,
    pub billing: Option<String>,
    pub currency: Option<String>,
    pub video_price_multiplier: Option<f64>,
    pub raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub provider: String,
    pub version: String,
    pub source: String,
    pub checked_at: String,
    pub models: Vec<ModelCapability>,
    pub can_cancel: bool,
    pub can_reconcile_unknown: bool,
    pub raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadReceipt {
    pub media_id: String,
    pub sha256: String,
    pub order: u32,
    pub kind: String,
    /// Includes all signature parameters. It must never be reconstructed from an object key.
    pub url: Option<String>,
    pub file_id: Option<String>,
    pub uploaded_at: String,
    pub upstream_read_confirmed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedRequest {
    pub provider: String,
    pub request: GenerationRequest,
    pub capability_version: String,
    pub upload_receipts: Vec<UploadReceipt>,
    pub wire: Value,
    pub estimated_cost: Option<f64>,
    pub prepared_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitReceipt {
    pub task_id: String,
    pub status: String,
    pub result_urls: Vec<String>,
    pub raw: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub task_id: String,
    /// processing, succeeded, failed, auth_required, or unknown.
    pub status: String,
    pub result_urls: Vec<String>,
    pub error: Option<String>,
    pub actual_cost: Option<f64>,
    pub raw: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReceipt {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub mime: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderError {
    pub code: String,
    pub submission_unknown: bool,
    pub auth_required: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}
impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ProviderError {}
impl ProviderError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            submission_unknown: false,
            auth_required: false,
            message: message.into(),
            context: None,
        }
    }
    pub fn uncertain(message: impl Into<String>) -> Self {
        let mut e = Self::new("SUBMISSION_UNKNOWN", message);
        e.submission_unknown = true;
        e
    }
    pub fn auth() -> Self {
        let mut e = Self::new(
            "AUTH_REQUIRED",
            "渠道登录或凭据已失效，请恢复同一账户后查询原任务。",
        );
        e.auth_required = true;
        e
    }
}
pub type PrepareObserver<'a> = dyn Fn(&Value) -> ProviderResult<()> + 'a;
pub fn prepare_observed(config:&ProviderConfig,request:&GenerationRequest,observer:&PrepareObserver<'_>) -> ProviderResult<PreparedRequest> {
    if config.kind=="chrome" {validate_request(request)?;chrome::prepare_observed(config,request,observer)}
    else {prepare(config,request)}
}
pub fn recover_chrome_prepare(config:&ProviderConfig,request:&GenerationRequest,proof:&Value,observer:&PrepareObserver<'_>) -> ProviderResult<PreparedRequest> {
    if config.kind!="chrome" || request.kind!="image" {return Err(ProviderError::new("CHROME_PREPARE_SCOPE","仅允许恢复原 Chrome 图片准备"));}
    validate_request(request)?;
    chrome::recover_prepare(config,request,proof,observer)
}
pub fn refresh_chrome_unsubmitted_prepared(config:&ProviderConfig,prepared:&PreparedRequest)->ProviderResult<PreparedRequest>{
    if config.kind!="chrome"||prepared.provider!="chrome"||prepared.request.kind!="image"{return Err(ProviderError::new("CHROME_PREPARE_SCOPE","仅允许核验原 Chrome 未提交图片准备"));}
    validate_request(&prepared.request)?;
    chrome::refresh_unsubmitted_prepared(config,prepared)
}

pub fn capabilities(config: &ProviderConfig) -> ProviderResult<Capabilities> {
    match config.kind.as_str() {
        "bh" | "binghuo" => bh::capabilities(config),
        "dreamina" => dreamina::capabilities(config),
        "chrome" => chrome::capabilities(config),
        _ => Err(ProviderError::new(
            "UNKNOWN_PROVIDER",
            "未配置此渠道适配器。",
        )),
    }
}
/// The task layer proves durable download completion before calling this adapter.
pub fn release_completed_tab(config: &ProviderConfig, prepared: &PreparedRequest, result: &QueryResult) -> ProviderResult<Value> {
    if config.kind == "chrome" { chrome::release_completed_tab(config, prepared, result) }
    else { Ok(json!({"status":"not_applicable"})) }
}
/// Current official CLI account identity and credit balance, with no raw UID or OAuth tokens.
pub fn dreamina_account(config: &ProviderConfig) -> ProviderResult<Value> {
    if config.kind != "dreamina" {
        return Err(ProviderError::new(
            "PROVIDER_MISMATCH",
            "此授权操作只适用于即梦官方 CLI。",
        ));
    }
    dreamina::account(config)
}
/// Explicit non-generating inspection, separate from OAuth and workflow recovery.
pub fn dreamina_readiness(config: &ProviderConfig) -> Value {
    dreamina::readiness(config)
}
/// Narrow reuse of the existing bounded, owned CLI supervisor. Caller cannot supply CLI args.
pub(crate) fn canvas_readiness_command(
    config: &ProviderConfig,
    command: crate::readiness_canvas::ReadCommand,
    deadline: std::time::Instant,
) -> (ProviderResult<crate::readiness_canvas::ProcessOutput>, Value) {
    dreamina::canvas_readiness_command(config, command, deadline)
}
/// User-triggered headless OAuth. Device material is for core memory only, never project state.
pub fn dreamina_begin_login(config: &ProviderConfig) -> ProviderResult<Value> {
    if config.kind != "dreamina" {
        return Err(ProviderError::new(
            "PROVIDER_MISMATCH",
            "此授权操作只适用于即梦官方 CLI。",
        ));
    }
    dreamina::begin_login(config)
}
/// Checks one device authorization attempt; completed login is independently identity-verified.
pub fn dreamina_check_login(config: &ProviderConfig, device_code: &str) -> ProviderResult<Value> {
    if config.kind != "dreamina" {
        return Err(ProviderError::new(
            "PROVIDER_MISMATCH",
            "此授权操作只适用于即梦官方 CLI。",
        ));
    }
    dreamina::check_login(config, device_code)
}
pub fn dreamina_logout(config: &ProviderConfig) -> ProviderResult<()> {
    if config.kind != "dreamina" {
        return Err(ProviderError::new(
            "PROVIDER_MISMATCH",
            "此授权操作只适用于即梦官方 CLI。",
        ));
    }
    dreamina::logout(config)
}
pub fn prepare(
    config: &ProviderConfig,
    request: &GenerationRequest,
) -> ProviderResult<PreparedRequest> {
    if shutting_down() {
        return Err(ProviderError::new(
            "CLIENT_CLOSING",
            "客户端正在关闭，未启动新的准备任务。",
        ));
    }
    validate_request(request)?;
    match config.kind.as_str() {
        "bh" | "binghuo" => bh::prepare(config, request),
        "dreamina" => dreamina::prepare(config, request),
        "chrome" => chrome::prepare(config, request),
        _ => Err(ProviderError::new(
            "UNKNOWN_PROVIDER",
            "未配置此渠道适配器。",
        )),
    }
}
pub fn submit(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
) -> ProviderResult<SubmitReceipt> {
    if shutting_down() {
        return Err(ProviderError::new(
            "CLIENT_CLOSING",
            "客户端正在关闭，未发送新的生成请求。",
        ));
    }
    if config.kind != prepared.provider && !(config.kind == "binghuo" && prepared.provider == "bh")
    {
        return Err(ProviderError::new(
            "PROVIDER_MISMATCH",
            "准备快照与提交渠道不一致。",
        ));
    }
    validate_request(&prepared.request)?;
    match config.kind.as_str() {
        "bh" | "binghuo" => bh::submit(config, prepared),
        "dreamina" => dreamina::submit(config, prepared),
        "chrome" => chrome::submit(config, prepared),
        _ => Err(ProviderError::new(
            "UNKNOWN_PROVIDER",
            "未配置此渠道适配器。",
        )),
    }
}
pub fn query(config: &ProviderConfig, task_id: &str) -> ProviderResult<QueryResult> {
    valid_id(task_id)?;
    match config.kind.as_str() {
        "bh" | "binghuo" => bh::query(config, task_id),
        "dreamina" => dreamina::query(config, task_id),
        "chrome" => chrome::query(config, task_id),
        _ => Err(ProviderError::new(
            "UNKNOWN_PROVIDER",
            "未配置此渠道适配器。",
        )),
    }
}
/// Queries an existing ID with its immutable image/video job kind. Never submits generation.
pub fn query_for_kind(
    config: &ProviderConfig,
    task_id: &str,
    kind: &str,
) -> ProviderResult<QueryResult> {
    valid_id(task_id)?;
    if !["image", "video"].contains(&kind) {
        return Err(ProviderError::new(
            "INVALID_RESULT_KIND",
            "原任务的媒体类型无效。",
        ));
    }
    if config.kind == "dreamina" {
        dreamina::query_kind(config, task_id, kind)
    } else {
        query(config, task_id)
    }
}
/// Uses persisted preparation identity for native Chrome provenance checks after restart.
pub fn query_prepared(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
    task_id: &str,
) -> ProviderResult<QueryResult> {
    valid_id(task_id)?;
    if config.kind == "chrome" {
        chrome::query_with_prepared(config, task_id, Some(prepared))
    } else if config.kind == "dreamina" {
        dreamina::query_prepared(config, prepared, task_id)
    } else {
        query(config, task_id)
    }
}
pub fn reconcile_unknown(config: &ProviderConfig, request_id: &str) -> ProviderResult<QueryResult> {
    // Chrome uses a caller-controlled request ID, so GET is safe even after an uncertain POST.
    if config.kind == "chrome" {
        return query(config, request_id);
    }
    Err(ProviderError::new(
        "MANUAL_RECONCILIATION_REQUIRED",
        "渠道没有按本地关联码核对未知提交的接口；请核对渠道记录，不能自动重发。",
    ))
}
pub fn download(
    config: &ProviderConfig,
    url: &str,
    target: &Path,
) -> ProviderResult<DownloadReceipt> {
    if config.kind == "chrome" {
        chrome::download(config, url, target)
    } else if config.kind == "dreamina" {
        wire::download_official_result(url, target)
    } else {
        wire::download_public(url, target)
    }
}

fn valid_id(id: &str) -> ProviderResult<()> {
    if id.is_empty()
        || id.len() > 200
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
    {
        return Err(ProviderError::new("INVALID_ID", "任务关联身份无效。"));
    }
    Ok(())
}
fn validate_request(request: &GenerationRequest) -> ProviderResult<()> {
    valid_id(&request.request_id)?;
    if !["image", "video"].contains(&request.kind.as_str())
        || request.prompt.trim().is_empty()
        || request.model.is_empty()
        || !request.options.is_object()
    {
        return Err(ProviderError::new(
            "INVALID_REQUEST",
            "生成类型、模型、提示词或参数无效。",
        ));
    }
    let mut orders = std::collections::HashSet::new();
    let mut media = std::collections::HashMap::new();
    for r in &request.references {
        if !["image", "video", "audio"].contains(&r.kind.as_str())
            || !orders.insert(r.order)
            || r.media_id.is_empty()
        {
            return Err(ProviderError::new(
                "INVALID_REFERENCES",
                "参考素材身份、类型或顺序重复。",
            ));
        }
        let identity = (&r.path, &r.sha256, &r.kind);
        if media
            .insert(&r.media_id, identity)
            .is_some_and(|old| old != identity)
        {
            return Err(ProviderError::new(
                "INVALID_REFERENCES",
                "同一媒体身份对应了不同文件或哈希。",
            ));
        }
        let meta = fs::symlink_metadata(&r.path)
            .map_err(|_| ProviderError::new("MEDIA_MISSING", "参考素材文件不存在。"))?;
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || !r.path.is_absolute()
            || meta.len() == 0
            || meta.len() > 200 * 1024 * 1024
        {
            return Err(ProviderError::new(
                "INVALID_MEDIA",
                "参考素材必须为非空绝对路径文件，单文件不超过 200 MiB。",
            ));
        }
        let bytes = fs::read(&r.path)
            .map_err(|_| ProviderError::new("MEDIA_READ_FAILED", "参考素材读取失败。"))?;
        if hex_hash(&bytes) != r.sha256 {
            return Err(ProviderError::new(
                "MEDIA_CHANGED",
                "参考素材与冻结快照的哈希不一致。",
            ));
        }
    }
    Ok(())
}
fn sorted_references(request: &GenerationRequest) -> Vec<MediaReference> {
    let mut refs = request.references.clone();
    refs.sort_by_key(|r| r.order);
    refs
}
fn hex_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn estimate(model: &ModelCapability, request: &GenerationRequest) -> Option<f64> {
    if model.currency.as_deref() != Some("CNY") {
        return None;
    }
    let mut total = match model.billing.as_deref()? {
        "per_second" => model.price? * request.duration? as f64,
        "per_call" | "per_image" => model.price?,
        _ => return None,
    };
    if request.references.iter().any(|r| r.kind == "video") {
        total *= model.video_price_multiplier?;
    }
    Some(total)
}
fn check_model(model: &ModelCapability, request: &GenerationRequest) -> ProviderResult<()> {
    if !model.adapted || model.kind != request.kind {
        return Err(ProviderError::new(
            "MODEL_NOT_ADAPTED",
            "此模型尚未完成参数适配。",
        ));
    }
    let count = |kind| request.references.iter().filter(|r| r.kind == kind).count();
    if count("image") > model.images
        || count("video") > model.videos
        || count("audio") > model.audios
        || count("image") < model.required_images
        || model
            .total_references
            .is_some_and(|max| request.references.len() > max)
    {
        return Err(ProviderError::new(
            "REFERENCE_CAPACITY",
            "素材数量超出模型能力，请修订关联单元；不会截断素材。",
        ));
    }
    if request.kind == "video" {
        let d = request
            .duration
            .ok_or_else(|| ProviderError::new("DURATION_REQUIRED", "视频需要指定时长。"))?;
        if model.min_duration.is_none()
            || model.max_duration.is_none()
            || d < model.min_duration.unwrap_or(0)
            || d > model.max_duration.unwrap_or(0)
            || (!model.allowed_durations.is_empty() && !model.allowed_durations.contains(&d))
        {
            return Err(ProviderError::new(
                "DURATION_CAPACITY",
                "时长超出已验证模型能力，请修订单元。",
            ));
        }
    }
    if model.ratios.is_empty()
        || !model.ratios.iter().any(|x| x == &request.ratio)
        || model.resolutions.is_empty()
        || !model
            .resolutions
            .iter()
            .any(|x| x.eq_ignore_ascii_case(&request.resolution))
    {
        return Err(ProviderError::new(
            "OUTPUT_CAPACITY",
            "比例或分辨率未被当前模型能力明确支持。",
        ));
    }
    Ok(())
}
