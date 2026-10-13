//! Native product capabilities and a separate isolated verification boundary.
use crate::{providers::*, tasks::Channels};
use serde_json::{json, Value};
use std::path::Path;

pub const OFFLINE_ACTIONS: &[&str] = &[
    "getExecutionPolicy",
    "settings.global.get",
    "settings.global.save",
    "runtime.status",
    "chrome.status",
    "chrome.managed.status",
    "readiness.check",
    "textSdk.status",
    "images.previewPrompt",
    "projects.list",
    "projects.create",
    "projects.archive",
    "projects.restore",
    "projects.delete",
    "files.choose",
    "legacy.inspect",
    "legacy.import",
    "project.import",
    "project.export",
    "workspace.get",
    "units.get",
    "units.versions.list",
    "units.version.restore",
    "units.save",
    "production.direction.save",
    "foundation.choices.list",
    "foundation.choice.confirm",
    "assets.get",
    "assets.usage",
    "assets.save",
    "assets.delete",
    "assets.restore",
    "jobs.detail",
    "jobs.close_unknown",
    "jobs.archive",
    "jobs.restore",
    "source.import",
    "source.resolve",
    "source.reparse",
    "media.import",
    "media.adopt",
    "media.adopt_batch",
    "images.ensure_current",
    "videos.adopt",
    "settings.save",
    "backup.create",
    "backup.restore",
];
pub const CREDENTIAL_FIELDS: &[&str] = &["bhToken", "chromeToken", "textApiKey"];
pub const STANDARD_ACTIONS: &[&str] = &[
    "chrome.connect", "chrome.check", "chrome.open", "chrome.bind",
    "chrome.managed.start", "chrome.managed.login", "chrome.managed.recover",
    "runtime.prepare",
    "foundation.run", "foundation.choices.apply", "images.design", "images.produce", "images.run",
    "workflows.resume", "providers.refresh", "videos.preflight", "videos.prepare", "videos.submit",
    "jobs.query", "jobs.download", "jobs.reconcile", "textCli.status", "textCli.login",
    "textCli.loginStatus", "textCli.cancelLogin", "textCli.openLogin", "dreamina.status",
    "dreamina.login", "dreamina.logout", "dreamina.open_authorization",
];
pub const STAGE3_ACTIONS: &[&str] = &["storyboard.run", "storyboard.decision.resolve", "units.regenerate", "storyboard.refine", "storyboard.refinement.adopt"];

pub fn check_standard(request: &Value) -> Result<(), String> {
    let action = request["action"].as_str().unwrap_or("");
    if STAGE3_ACTIONS.contains(&action) {
        return if creative_runtime_ready() { Ok(()) }
        else { Err("STAGE3_ADAPTER_PENDING: 最新 3008 创作运行装配尚未接入；已有分镜和历史保留，未调用模型。".into()) };
    }
    if OFFLINE_ACTIONS.contains(&action) || STANDARD_ACTIONS.contains(&action) { return Ok(()); }
    Err(format!("STANDARD_ACTION_UNSUPPORTED: {action} 尚未实现"))
}
pub fn standard_report() -> Value {
    let mut actions = OFFLINE_ACTIONS.to_vec();
    actions.extend_from_slice(STANDARD_ACTIONS);
    if creative_runtime_ready() { actions.extend_from_slice(STAGE3_ACTIONS); }
    json!({"version":"execution-policy/v3","mode":"standard_native","liveExecutionAllowed":true,
        "stage3CreativePackStatus":if creative_runtime_ready(){"verified_active"}else{"inactive"},
        "allowedActions":actions,"credentialBackend":crate::platform::credential_backend_status(),
        "reasons":["第一阶段制作和第二阶段提示词设计使用所选真实文字渠道。",
            "图片及视频沿用已实现渠道的账户、模型、费用确认和未知提交保护；即梦画布生成桥尚未接入。",
            if creative_runtime_ready() { "第三阶段使用已验证接线的 3008 规则与 Rust 兼容适配；真实成稿质量以项目验收为准。" } else { "最新 3008 第三阶段尚待运行适配。" }],
        "textBackends":[{"id":"codex_cli","status":"enabled"},{"id":"codex_sdk","status":if crate::text_sdk::native_execution_enabled(){"explicit_selection_when_ready"}else{"offline_contract_only"}}],
        "localAcceptance":{"backend":"codex_cli","model":"gpt-6.1-sol","reasoning":"medium"}})
}
pub fn report() -> Value {
    json!({"version":"execution-policy/v1", "mode":"offline_prepare_only",
        "liveExecutionAllowed":false, "stage3CreativePackStatus":"inactive",
        "allowedActions":OFFLINE_ACTIONS,
        "reasons":["真实生成保持关闭；本机只读连接检查须由用户单独点击，云端与隔离探针不读取宿主账户。", "第三阶段原件已核对一致，运行适配仍待验证。"],
        "textBackends":[{"id":"codex_cli","status":"preserved_not_enabled"},{"id":"codex_sdk","status":"offline_contract_only"}],
        "localAcceptance":{"backend":"codex_cli","model":"gpt-6.1-sol","reasoning":"medium"}})
}
pub fn check(request: &Value) -> Result<(), String> {
    let action = request["action"].as_str().unwrap_or("");
    if !OFFLINE_ACTIONS.contains(&action) {
        return Err(format!(
            "OFFLINE_EXECUTION_BLOCKED: {action} 需要单独启用并验证的本地执行入口"
        ));
    }
    if ["settings.save", "settings.global.save"].contains(&action)
        && CREDENTIAL_FIELDS.iter().any(|field| {
            request["settings"]
                .get(field)
                .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
        })
    {
        return Err("OFFLINE_CREDENTIALS_BLOCKED: 离线工程不保存或读取账户凭据".into());
    }
    Ok(())
}
pub struct OfflineChannels;
fn blocked<T>() -> ProviderResult<T> {
    Err(ProviderError::new(
        "OFFLINE_EXECUTION_BLOCKED",
        "离线工程不连接渠道或启动生成。",
    ))
}
impl Channels for OfflineChannels {
    fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> {
        blocked()
    }
    fn prepare(
        &self,
        _: &ProviderConfig,
        _: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        blocked()
    }
    fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        blocked()
    }
    fn query(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        blocked()
    }
    fn download(&self, _: &ProviderConfig, _: &str, _: &Path) -> ProviderResult<DownloadReceipt> {
        blocked()
    }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        blocked()
    }
}

// Only the compiled compatibility adapter with verified frozen originals can open Stage3.
// User settings cannot activate another creative pack.
pub fn creative_runtime_ready() -> bool {
    crate::pipeline::stage3_pack::runtime_ready()
}
pub const NATIVE_TEXT_ACTIONS: &[&str] = &[
    "foundation.run",
    "images.design",
    "workflows.resume",
    "textCli.status",
];
pub fn check_native_action(request: &Value) -> Result<(), String> {
    let action = request["action"].as_str().unwrap_or("");
    if OFFLINE_ACTIONS.contains(&action) {
        return check(request);
    }
    if NATIVE_TEXT_ACTIONS.contains(&action) {
        return Ok(());
    }
    if ["storyboard.run", "storyboard.decision.resolve"].contains(&action)
        && creative_runtime_ready()
    {
        return Ok(());
    }
    Err(format!(
        "NATIVE_ACCEPTANCE_SCOPE_BLOCKED: {action} 未在本轮有界入口开放"
    ))
}
pub fn native_report(acceptance: &crate::native_acceptance::NativeAcceptance) -> Value {
    let mut allowed = OFFLINE_ACTIONS.to_vec();
    allowed.extend_from_slice(NATIVE_TEXT_ACTIONS);
    if creative_runtime_ready() {
        allowed.extend_from_slice(&["storyboard.run", "storyboard.decision.resolve"]);
    }
    json!({"version":"execution-policy/v2","mode":"native_cli_acceptance","liveExecutionAllowed":true,
        "stage3CreativePackStatus":if creative_runtime_ready(){"verified_active"}else{"inactive"},
        "allowedActions":allowed,"acceptance":acceptance.status(creative_runtime_ready()),
        "credentialBackend":crate::platform::credential_backend_status(),
        "reasons":["仅对显式授权文件哈希绑定的10集原文执行第一/二阶段文字工作。","CLI固定gpt-6.1-sol / medium；每阶段只有一个新工作流，恢复保留原任务。","Stage3适配未通过前保持关闭；SDK真实调用、未核验免费图片和真实视频均关闭。"]})
}
