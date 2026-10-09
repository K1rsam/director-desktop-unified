//! Explicit user-requested, non-generating readiness. No workflow/cache/credential writes.
use crate::{
    providers::{self, ProviderConfig},
    store::{now, s, Result},
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
pub const VERSION: &str = "provider-readiness/v1";
pub const PROVIDERS: &[&str] = &["codex_cli", "dreamina", "dreamina_canvas", "codex_sdk"];

pub fn capability(enabled: bool) -> Value {
    json!({"version":VERSION,"enabled":enabled,"providers":PROVIDERS,"generationAllowed":false})
}
pub fn request_settings(request: &Value, saved: &Value) -> Result<(String, Value)> {
    let provider = s(request, "provider");
    let key = match provider {
        "codex_cli" => "codexPath",
        "dreamina" => "dreaminaPath",
        "dreamina_canvas" => "dreaminaCanvasPath",
        "codex_sdk" => "textSdkNodePath",
        _ => {
            return Err(
                "READINESS_PROVIDER_UNSUPPORTED: 仅允许已适配的 CLI、即梦或官方 SDK 检查".into(),
            )
        }
    };
    let mut settings = json!({});
    for name in [key, "textModel", "reasoning"] {
        if let Some(value) = saved.get(name) {
            settings[name] = value.clone();
        }
    }
    if provider == "codex_sdk" {
        if let Some(path) = saved.get("codexPath") {
            settings["codexPath"] = path.clone();
        }
    }
    if let Some(overrides) = request.get("settings").filter(|v| !v.is_null()) {
        let overrides = overrides
            .as_object()
            .ok_or("READINESS_SETTINGS_INVALID: 检查设置必须为对象")?;
        if overrides.keys().any(|k| k != key && !(provider == "codex_sdk" && k == "codexPath")) {
            return Err("READINESS_SETTINGS_INVALID: 检查只接受本渠道的可执行文件路径，不接受执行、登录或凭据参数".into());
        }
        for (name, value) in overrides {
            let path = value
                .as_str()
                .ok_or("READINESS_PATH_INVALID: 路径必须为文本")?;
            if path.len() > 4096 || path.chars().any(char::is_control) {
                return Err("READINESS_PATH_INVALID: 路径无效".into());
            }
            settings[name] = json!(path.trim());
        }
    }
    Ok((provider.into(), settings))
}
pub fn empty(provider: &str) -> Value {
    json!({"schemaVersion":VERSION,"provider":provider,"checkedAt":now(),
        "installation":{"state":"not_checked","version":null,"executable":null},
        "protocol":{"state":"not_checked","name":null},
        "authentication":{"state":"not_checked","identityVerified":false},
        "catalog":{"state":"not_checked","models":[]},
        "network":{"state":"not_tested","proof":[]},
        "generation":{"enabled":false,"testPerformed":false,"calls":0},"messages":[],"details":{}})
}
fn bounded(value: &Value, limit: usize) -> Value {
    value
        .as_str()
        .filter(|s| !s.chars().any(char::is_control))
        .map(|s| json!(s.chars().take(limit).collect::<String>()))
        .unwrap_or(Value::Null)
}
pub fn codex_report(status: &Value, settings: &Value) -> Value {
    let mut result = empty("codex_cli");
    let installed = status["available"] == true;
    let inspected = installed
        && status["version"]
            .as_str()
            .is_some_and(|v| v.starts_with("codex-cli "));
    let rpc_ready = status["readinessRpcReady"].as_bool().unwrap_or(inspected);
    let logged_in = status["loggedIn"] == true;
    // The official read-only RPC checked the account in this invocation. Do not
    // confuse redacting its identity from the public report with not checking it.
    // A local version string alone is not account evidence.
    let account_checked = inspected && status["readinessRpcReady"] == true;
    let identity_verified = account_checked
        && logged_in
        && status["account"].as_object().is_some_and(|account| !account.is_empty());
    let models:Vec<Value>=status["models"].as_array().into_iter().flatten().take(1000).filter(|m|m["id"].as_str().is_some_and(|s|s.len()<=256&&!s.chars().any(char::is_control))).map(|m|json!({"id":m["id"],"name":bounded(&m["displayName"],256),"displayName":bounded(&m["displayName"],256),"supportedReasoningEfforts":m["supportedReasoningEfforts"],"defaultReasoningEffort":m["defaultReasoningEffort"],"isDefault":m["isDefault"]})).collect();
    result["installation"] = json!({"state":if inspected{"ready"}else if installed{"invalid"}else{"missing"},"version":bounded(&status["version"],128),"executable":bounded(&status["executable"],4096)});
    result["protocol"] = json!({"state":if rpc_ready{"ready"}else if installed{"failed"}else{"not_checked"},"name":"Codex app-server read-only RPC"});
    result["authentication"] = json!({"state":if logged_in{"authenticated"}else if rpc_ready{"not_logged_in"}else{"unknown"},"identityVerified":identity_verified});
    result["catalog"] = json!({"state":if models.is_empty(){"unavailable"}else{"available"},"models":models,"selectedModelAvailable":models.iter().any(|m|m["id"]==settings["textModel"])});
    let warnings: Vec<Value> = status["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .take(10)
        .map(|v| bounded(v, 512))
        .filter(|v| !v.is_null())
        .collect();
    result["messages"] = json!([
        "检查只读取 CLI 版本、现有登录、模型目录与额度，没有发起文本生成。",
        "账户核验仅确认 CLI 返回的现有登录；模型目录不是套餐权限或真实模型调用证明，公网及真实生成尚未单独验证。"
    ]);
    if let Some(error) = bounded(&status["error"], 512)
        .as_str()
        .filter(|e| !e.is_empty())
    {
        result["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!(format!("检查结果：{error}")));
    }
    for warning in &warnings {
        result["messages"]
            .as_array_mut()
            .unwrap()
            .push(warning.clone());
    }
    let mut safe = json!({"available":installed,"ready":status["ready"]==true,"loggedIn":logged_in,"version":result["installation"]["version"],"executable":result["installation"]["executable"],"models":models,"limits":status["limits"],"warnings":warnings,"error":bounded(&status["error"],512),"checkedAt":result["checkedAt"],"generationRequestsSent":0,"catalogIsEntitlementCheck":false,"globalConcurrency":4,"episodeConcurrency":2,"login":{"state":"idle"}});
    safe["account"] = json!({"type":bounded(&status["account"]["type"],64),"planType":bounded(&status["account"]["planType"],64)});
    result["details"] = json!({"cliStatus":safe,"accountChecked":account_checked,"accountIdentityNotExported":true,"permittedRpcMethods":["account/read(refreshToken=false)","model/list","account/rateLimits/read"]});
    result
}
pub async fn check(provider: &str, settings: &Value, resource_root: Option<&Path>) -> Value {
    match provider {
        "codex_cli" => codex_report(&crate::text_cli::readiness_status(settings).await, settings),
        "codex_sdk" => crate::readiness_sdk::check(settings, resource_root).await,
        "dreamina_canvas" => {
            let settings = settings.clone();
            tokio::task::spawn_blocking(move || crate::readiness_canvas::check(&settings)).await.unwrap_or_else(|_| {
                let mut report = empty("dreamina_canvas");
                report["messages"] = json!(["Canvas 只读检查未完成；没有发起生成。"]);
                report
            })
        },
        "dreamina" => {
            let path = discover_dreamina(settings, resource_root);
            let config: ProviderConfig = serde_json::from_value(
                json!({"kind":"dreamina","cliPath":path,"timeoutSeconds":15}),
            )
            .expect("fixed readiness config");
            // Existing provider inspection is synchronous and individually bounded.
            // It never enters DreaminaSessions, OAuth polling or job resumption.
            match tokio::task::spawn_blocking(move || providers::dreamina_readiness(&config)).await
            {
                Ok(value) => value,
                Err(_) => {
                    let mut r = empty("dreamina");
                    r["messages"] =
                        json!(["即梦只读检查进程未完成；没有发起生成，请检查本机 CLI。"]);
                    r
                }
            }
        }
        _ => {
            let mut r = empty(provider);
            r["messages"] = json!(["不支持的检查渠道"]);
            r
        }
    }
}
fn discover_dreamina(settings: &Value, resources: Option<&Path>) -> Option<PathBuf> {
    let selected = s(settings, "dreaminaPath").trim();
    if !selected.is_empty() {
        return Some(PathBuf::from(selected));
    }
    let name = if cfg!(windows) {
        "dreamina.exe"
    } else {
        "dreamina"
    };
    let mut candidates = Vec::new();
    if let Some(resources) = resources {
        candidates.push(resources.join("tools/dreamina/1.4.19").join(name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    candidates
        .into_iter()
        .find_map(|p| p.is_file().then(|| p.canonicalize().ok()).flatten())
}
