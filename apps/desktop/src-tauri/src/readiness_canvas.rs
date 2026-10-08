//! Explicit read-only Canvas integration. It is deliberately not a generation provider.
//! No legacy account/history migration, OAuth, remote writes, catalog caching or retries.
use crate::{
    providers::{self, ProviderConfig, ProviderError, ProviderResult},
    store::s,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

#[path = "readiness_canvas_diagnostics.rs"]
mod diagnostics;
pub(crate) use diagnostics::ProcessOutput;

const PROVIDER: &str = "dreamina_canvas";
const OVERALL_TIMEOUT: Duration = Duration::from_secs(45);

/// All subprocess arguments are compiled in. IPC accepts only the executable path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadCommand {
    Version,
    Schema,
    AuthStatus,
    AuthAccount,
    ImageModels,
}
impl ReadCommand {
    pub(crate) fn args(self) -> &'static [&'static str] {
        match self {
            Self::Version => &["--format", "json", "version"],
            Self::Schema => &["--format", "json", "schema"],
            Self::AuthStatus => &[
                "--format",
                "json",
                "--profile",
                "default",
                "--region",
                "cn",
                "--non-interactive",
                "auth",
                "status",
            ],
            Self::AuthAccount => &[
                "--format",
                "json",
                "--profile",
                "default",
                "--region",
                "cn",
                "--non-interactive",
                "auth",
                "account",
            ],
            Self::ImageModels => &[
                "--format",
                "json",
                "--profile",
                "default",
                "--region",
                "cn",
                "--non-interactive",
                "model",
                "list",
                "--type",
                "image",
            ],
        }
    }
    pub(crate) fn output_limit(self) -> usize {
        if matches!(self, Self::Schema | Self::ImageModels) {
            1024 * 1024
        } else {
            64 * 1024
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Schema => "schema",
            Self::AuthStatus => "auth status",
            Self::AuthAccount => "auth account",
            Self::ImageModels => "model list --type image",
        }
    }
}
fn failure(code: &'static str) -> ProviderError {
    ProviderError::new(code, "Canvas 只读检查未完成。")
}
fn identifier(v: &Value, max: usize) -> Option<&str> {
    v.as_str().filter(|s| {
        !s.is_empty()
            && s.len() <= max
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
    })
}
fn text(v: &Value, max: usize) -> Option<&str> {
    v.as_str()
        .filter(|s| s.len() <= max && !s.chars().any(char::is_control))
}
fn envelope(bytes: &[u8]) -> ProviderResult<Value> {
    let body: Value = serde_json::from_slice(bytes).map_err(|_| failure("CANVAS_INVALID_JSON"))?;
    if body["schemaVersion"] != "1" {
        return Err(failure("CANVAS_ENVELOPE_UNSUPPORTED"));
    }
    if body["ok"] == false {
        // Never export error messages, credential-storage directories, request payloads or tokens.
        return Err(failure(match body["error"]["reason"].as_str() {
            Some("credential_store_permission_denied") => {
                "CANVAS_CREDENTIAL_STORE_PERMISSION_DENIED"
            }
            Some("credential_store_unavailable") => "CANVAS_CREDENTIAL_STORE_UNAVAILABLE",
            Some("credential_store_corrupt") => "CANVAS_CREDENTIAL_STORE_CORRUPT",
            Some("dns_resolution_failed") => "CANVAS_DNS_FAILED",
            Some("network_permission_denied") => "CANVAS_NETWORK_PERMISSION_DENIED",
            Some("network_timeout") => "CANVAS_NETWORK_TIMEOUT",
            _ => "CANVAS_READ_FAILED",
        }));
    }
    if body["ok"] != true || !body["data"].is_object() {
        return Err(failure("CANVAS_ENVELOPE_INVALID"));
    }
    Ok(body["data"].clone())
}

fn optional_bool(source: &Value, out: &mut Map<String, Value>, key: &str) -> ProviderResult<()> {
    if let Some(value) = source.get(key) {
        if !value.is_boolean() {
            return Err(failure("CANVAS_CATALOG_INVALID"));
        }
        out.insert(key.into(), value.clone());
    }
    Ok(())
}
fn optional_number(source: &Value, out: &mut Map<String, Value>, key: &str) -> ProviderResult<()> {
    if let Some(value) = source.get(key) {
        if !value.as_f64().is_some_and(|v| v.is_finite()) {
            return Err(failure("CANVAS_CATALOG_INVALID"));
        }
        out.insert(key.into(), value.clone());
    }
    Ok(())
}
fn optional_strings(source: &Value, out: &mut Map<String, Value>, key: &str) -> ProviderResult<()> {
    if let Some(value) = source.get(key) {
        let values = value
            .as_array()
            .filter(|v| v.len() <= 128)
            .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
        if !values.iter().all(|v| text(v, 256).is_some()) {
            return Err(failure("CANVAS_CATALOG_INVALID"));
        }
        out.insert(key.into(), value.clone());
    }
    Ok(())
}
fn reference_spec(value: &Value) -> ProviderResult<Value> {
    if !value.is_object() {
        return Err(failure("CANVAS_CATALOG_INVALID"));
    }
    let mut out = Map::new();
    for key in ["min", "max"] {
        if value[key].as_u64().is_none() {
            return Err(failure("CANVAS_CATALOG_INVALID"));
        }
        out.insert(key.into(), value[key].clone());
    }
    if !value["types"].is_array() {
        return Err(failure("CANVAS_CATALOG_INVALID"));
    }
    optional_strings(value, &mut out, "requireAnyOfTypes")?;
    if let Some(types) = value.get("types") {
        let types = types
            .as_array()
            .filter(|v| v.len() <= 16)
            .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
        let mut result = vec![];
        for kind in types {
            let mut item = Map::new();
            let name = identifier(&kind["mediaType"], 64)
                .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
            item.insert("mediaType".into(), json!(name));
            for key in ["min", "max"] {
                if kind[key].as_u64().is_none() {
                    return Err(failure("CANVAS_CATALOG_INVALID"));
                }
                item.insert(key.into(), kind[key].clone());
            }
            result.push(Value::Object(item));
        }
        out.insert("types".into(), json!(result));
    }
    Ok(Value::Object(out))
}
fn mode_spec(value: &Value) -> ProviderResult<Value> {
    let mut out = Map::new();
    let name = identifier(&value["name"], 64).ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
    out.insert("name".into(), json!(name));
    optional_bool(value, &mut out, "vipRequired")?;
    optional_strings(value, &mut out, "vipResolutions")?;
    if let Some(references) = value.get("references") {
        out.insert("references".into(), reference_spec(references)?);
    }
    if let Some(flags) = value.get("flags") {
        let flags = flags
            .as_array()
            .filter(|v| v.len() <= 64)
            .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
        let mut result = vec![];
        for flag in flags {
            let mut item = Map::new();
            let name = text(&flag["flag"], 64)
                .filter(|name| {
                    name.starts_with("--")
                        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                })
                .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
            item.insert("flag".into(), json!(name));
            if !flag["required"].is_boolean() {
                return Err(failure("CANVAS_CATALOG_INVALID"));
            }
            item.insert("required".into(), flag["required"].clone());
            if let Some(values) = flag.get("values") {
                let values = values
                    .as_array()
                    .filter(|v| v.len() <= 128)
                    .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
                if !values.iter().all(|v| text(v, 256).is_some()) {
                    return Err(failure("CANVAS_CATALOG_INVALID"));
                }
                item.insert("values".into(), json!(values));
            }
            for key in ["min", "max", "step"] {
                optional_number(flag, &mut item, key)?;
            }
            for key in ["minLength", "maxLength"] {
                if let Some(value) = flag.get(key) {
                    if value.as_u64().is_none() {
                        return Err(failure("CANVAS_CATALOG_INVALID"));
                    }
                    item.insert(key.into(), value.clone());
                }
            }
            result.push(Value::Object(item));
        }
        out.insert("flags".into(), json!(result));
    }
    // Absence is kept absent: no synthesized resolution, references or membership defaults.
    Ok(Value::Object(out))
}
fn catalog(data: &Value) -> ProviderResult<Value> {
    let items = data["items"]
        .as_array()
        .filter(|v| v.len() <= 256)
        .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
    let mut models = vec![];
    let mut ids = std::collections::HashSet::new();
    for item in items {
        let model =
            identifier(&item["model"], 256).ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
        if item["type"] != "image" || !ids.insert(model) {
            return Err(failure("CANVAS_CATALOG_INVALID"));
        }
        let mut spec = Map::new();
        spec.insert("model".into(), json!(model));
        optional_strings(item, &mut spec, "aliases")?;
        optional_bool(item, &mut spec, "vipRequired")?;
        optional_strings(item, &mut spec, "vipResolutions")?;
        if !item["modes"].is_array() {
            return Err(failure("CANVAS_CATALOG_INVALID"));
        }
        if let Some(modes) = item.get("modes") {
            let modes = modes
                .as_array()
                .filter(|v| v.len() <= 32)
                .ok_or_else(|| failure("CANVAS_CATALOG_INVALID"))?;
            spec.insert(
                "modes".into(),
                json!(modes
                    .iter()
                    .map(mode_spec)
                    .collect::<ProviderResult<Vec<_>>>()?),
            );
        }
        models.push(json!({"id":model,"name":model,"kind":"image","specification":spec}));
    }
    Ok(json!(models))
}

pub(crate) fn executable(path: &Path) -> ProviderResult<Value> {
    if !path.is_absolute() {
        return Err(failure("CANVAS_PATH_INVALID"));
    }
    let meta = fs::symlink_metadata(path).map_err(|_| failure("CANVAS_NOT_FOUND"))?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() < 4
        || meta.len() > 256 * 1024 * 1024
    {
        return Err(failure("CANVAS_BINARY_INVALID"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(failure("CANVAS_NOT_EXECUTABLE"));
        }
    }
    let mut file = fs::File::open(path).map_err(|_| failure("CANVAS_BINARY_UNREADABLE"))?;
    let mut header = [0; 4];
    file.read_exact(&mut header)
        .map_err(|_| failure("CANVAS_BINARY_INVALID"))?;
    if ![
        b"\x7fELF".as_slice(),
        b"\xcf\xfa\xed\xfe",
        b"\xfe\xed\xfa\xcf",
        b"\xca\xfe\xba\xbe",
        b"\xbe\xba\xfe\xca",
        b"\xca\xfe\xba\xbf",
        b"\xbf\xba\xfe\xca",
    ]
    .contains(&header.as_slice())
        && !header.starts_with(b"MZ")
    {
        return Err(failure("CANVAS_NATIVE_BINARY_REQUIRED"));
    }
    std::io::Seek::rewind(&mut file).map_err(|_| failure("CANVAS_BINARY_UNREADABLE"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    let mut bytes = 0;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| failure("CANVAS_BINARY_UNREADABLE"))?;
        if count == 0 {
            break;
        }
        bytes += count;
        if bytes > 256 * 1024 * 1024 {
            return Err(failure("CANVAS_BINARY_INVALID"));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(json!({"path":path,"sha256":format!("{:x}",hasher.finalize())}))
}
fn add_error(report: &mut Value, command: ReadCommand, error: &ProviderError) {
    report["details"]["checks"][command.label()] = json!({"state":"failed","code":error.code});
    report["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!(format!(
            "{} 检查未完成（{}）；已保留其他独立检查结果。",
            command.label(),
            error.code
        )));
}

pub(crate) fn legacy_version_label(bytes: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(bytes).ok()?;
    if value.len() > 4096 {
        return None;
    }
    if let Ok(build) = serde_json::from_str::<Value>(value) {
        // The legacy Windows CLI prints a bare Git build object. Canvas uses
        // a schema/ok/data envelope, which must never enter this adapter.
        let object = build.as_object()?;
        if object.len() != 3 { return None; }
        let version = object.get("version")?.as_str()?;
        let commit = object.get("commit")?.as_str()?;
        let built_at = object.get("build_time")?.as_str()?;
        if !(7..=40).contains(&commit.len()) || !commit.bytes().all(|c| c.is_ascii_hexdigit())
            || !version.strip_suffix("-dirty").unwrap_or(version).eq_ignore_ascii_case(commit)
            || chrono::DateTime::parse_from_rfc3339(built_at).is_err() {
            return None;
        }
        // This is build-format evidence only. The readiness adapter separately
        // checks the old root commands and every adapted command's help flags.
        return Some(version.to_string());
    }
    if value.trim_start().starts_with(['{', '[']) || value.to_ascii_lowercase().contains("dreamina-canvas") { return None; }
    value.split_whitespace().find_map(|part| {
        if part == "1d0bd17-dirty" {
            return Some(part.to_string());
        }
        let part = part.strip_prefix('v').unwrap_or(part);
        part.strip_prefix("1.4.")
            .filter(|patch| !patch.is_empty() && patch.bytes().all(|c| c.is_ascii_digit()))
            .map(|_| part.to_string())
    })
}
#[cfg(test)]
fn legacy_version_supported(bytes: &[u8]) -> bool {
    legacy_version_label(bytes).is_some()
}

fn protocol_error_state(error: &ProviderError) -> &'static str {
    if error.code.ends_with("_UNSUPPORTED") {
        "unsupported"
    } else {
        "failed"
    }
}

fn base_report() -> Value {
    let mut report = crate::readiness::empty(PROVIDER);
    report["protocol"]["name"] = json!("Dreamina Canvas JSON schema 1 · read-only");
    report["details"] = json!({"checks":{},"detectedProtocol":"unknown","profile":"default","region":"cn",
        "accountQuery":{"state":"not_checked"},"quota":{"state":"unsupported","remaining":null},
        "freeImageEligibility":"unverified","accountIdentityNotExported":true,"modelSpecifications":[],
        "generationBridge":"not_implemented","catalogIsEntitlementCheck":false});
    report["messages"] = json!([
        "仅检查 Canvas 版本、协议、已有登录、账户与图片模型目录，没有生成或登录操作。",
        "此版本没有已适配的只读积分查询；余额未知。会员状态与模型目录均不能证明可免费生成。",
        "本次目录仅供查看；未修改旧版 Dreamina、已保存生成设置、任务或历史记录。"
    ]);
    report
}

pub fn check(settings: &Value) -> Value {
    let mut report = base_report();
    let selected = s(settings, "dreaminaCanvasPath").trim();
    if selected.is_empty() {
        report["installation"]["state"] = json!("missing");
        report["messages"].as_array_mut().unwrap().push(json!(
            "请选择已安装的 dreamina-canvas 可执行文件；不会借用旧版 Dreamina 路径。"
        ));
        return report;
    }
    let identity = match executable(Path::new(selected)) {
        Ok(identity) => identity,
        Err(error) => {
            report["installation"]["state"] = json!(if error.code == "CANVAS_NOT_FOUND" {
                "missing"
            } else {
                "invalid"
            });
            add_error(&mut report, ReadCommand::Version, &error);
            return report;
        }
    };
    report["installation"] = json!({"state":"ready","version":null,"executable":identity["path"]});
    report["details"]["binarySha256"] = identity["sha256"].clone();
    let config: ProviderConfig = serde_json::from_value(
        json!({"kind":"dreamina_canvas","cliPath":selected,"timeoutSeconds":10}),
    )
    .expect("fixed Canvas readiness config");
    let deadline = Instant::now() + OVERALL_TIMEOUT;
    let mut process_diagnostics = Map::new();
    let mut report = inspect(report, |command| {
        let (outcome,supervisor) = providers::canvas_readiness_command(&config, command, deadline);
        let (mut diagnostic,result)=match outcome {
            Ok(output)=>diagnostics::classify(command,output),
            Err(error)=>(json!({"termination":"unknown","exitCode":null,"signal":null,"stdoutBytes":null,"stderrBytes":null,"stdoutShape":"not_parsed","stderrShape":"not_parsed","structuredErrorSource":"none","errors":{}}),Err(error)),
        };
        diagnostic["supervisor"]=supervisor;
        process_diagnostics.insert(command.label().into(),diagnostic);
        result
    });
    for (command, diagnostic) in &process_diagnostics {
        if report["details"]["checks"][command]["state"] == "failed" {
            let termination = if let Some(signal) = diagnostic["signal"].as_i64() {
                format!("信号 {signal}")
            } else if let Some(code) = diagnostic["exitCode"].as_i64() {
                format!("退出码 {code}")
            } else {
                "退出状态未知".into()
            };
            report["messages"].as_array_mut().unwrap().push(json!(format!(
                "{command}：{termination}，标准输出 {} 字节，错误输出 {} 字节；只保留结构化安全诊断，未推断原因或自动重试。",
                diagnostic["stdoutBytes"].as_u64().map(|n|n.to_string()).unwrap_or_else(||"未知".into()), diagnostic["stderrBytes"].as_u64().map(|n|n.to_string()).unwrap_or_else(||"未知".into())
            )));
            if let Some(source) = diagnostic["structuredErrorSource"]
                .as_str()
                .filter(|source| ["stdout", "stderr"].contains(source))
            {
                let error = &diagnostic["errors"][source];
                let fields: Vec<_> = ["code", "class", "reason", "requiredAction"]
                    .iter()
                    .filter_map(|key| error[*key].as_str().map(|value| format!("{key}={value}")))
                    .collect();
                if !fields.is_empty() {
                    report["messages"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(format!(
                            "{command} 结构化错误：{}",
                            fields.join("；")
                        )));
                }
            }
            if report["details"]["checks"][command]["code"] == "CANVAS_OAUTH_SCOPE_INSUFFICIENT" {
                report["messages"].as_array_mut().unwrap().push(json!("Canvas 官方账户接口报告授权范围不足，需要通过官方流程补充授权；本次检查没有启动登录或修改凭据。"));
            }
        }
    }
    report["details"]["processDiagnostics"] = Value::Object(process_diagnostics);
    report
}

fn inspect(
    mut report: Value,
    mut run: impl FnMut(ReadCommand) -> ProviderResult<Vec<u8>>,
) -> Value {
    let mut read = |command| run(command).and_then(|bytes| envelope(&bytes));
    let version = match read(ReadCommand::Version) {
        Ok(data) => data,
        Err(error) => {
            report["protocol"]["state"] = json!(protocol_error_state(&error));
            add_error(&mut report, ReadCommand::Version, &error);
            return report;
        }
    };
    if let Some(version) = identifier(&version["version"], 64) {
        report["installation"]["version"] = json!(version);
    }
    if let Err(error) = supported_version(&version) {
        report["protocol"]["state"] = json!("unsupported");
        add_error(&mut report, ReadCommand::Version, &error);
        return report;
    }
    report["details"]["detectedProtocol"] = json!("dreamina_canvas");
    report["details"]["checks"]["version"] = json!({"state":"ready"});
    let schema = match read(ReadCommand::Schema).and_then(|data| supported_schema(&data)) {
        Ok(schema) => schema,
        Err(error) => {
            report["protocol"]["state"] = json!(protocol_error_state(&error));
            add_error(&mut report, ReadCommand::Schema, &error);
            return report;
        }
    };
    report["protocol"]["state"] = json!("ready");
    report["details"]["checks"]["schema"] = json!({"state":"ready"});
    let auth = match read(ReadCommand::AuthStatus) {
        Ok(data) => {
            if data["profile"] != "default"
                || data["region"] != "cn"
                || data["distribution"] != "cn"
                || data["edition"] != "public"
                || data["version"] != version["version"]
                || !data["loggedIn"].is_boolean()
            {
                report["authentication"]["state"] = json!("unknown");
                add_error(
                    &mut report,
                    ReadCommand::AuthStatus,
                    &failure("CANVAS_AUTH_CONTEXT_MISMATCH"),
                );
                return report;
            }
            let logged_in = data["loggedIn"] == true;
            report["authentication"]["state"] = json!(if logged_in {
                "authenticated"
            } else {
                "not_logged_in"
            });
            report["details"]["checks"]["auth status"] = json!({"state":"ready"});
            logged_in
        }
        Err(error) => {
            report["authentication"]["state"] = json!("unknown");
            add_error(&mut report, ReadCommand::AuthStatus, &error);
            return report;
        }
    };
    if auth && schema.account {
        match read(ReadCommand::AuthAccount) {
            Ok(data)
                if data["userId"].as_u64().is_some_and(|id| id > 0)
                    && data.get("isVip").is_none_or(Value::is_boolean)
                    && data.get("vipLevel").is_none_or(|v| text(v, 64).is_some()) =>
            {
                let mut account = json!({"state":"available"});
                for key in ["isVip", "vipLevel"] {
                    if let Some(value) = data.get(key) {
                        account[key] = value.clone();
                    }
                }
                report["details"]["accountQuery"] = account;
                report["authentication"]["identityVerified"] = json!(true);
                report["details"]["checks"]["auth account"] = json!({"state":"ready"});
                report["network"] = json!({"state":"verified_read_only","proof":["auth account"]});
            }
            Ok(_) => {
                report["details"]["accountQuery"]["state"] = json!("unavailable");
                add_error(
                    &mut report,
                    ReadCommand::AuthAccount,
                    &failure("CANVAS_ACCOUNT_INVALID"),
                );
            }
            Err(error) => {
                report["details"]["accountQuery"]["state"] = json!("unavailable");
                add_error(&mut report, ReadCommand::AuthAccount, &error);
            }
        }
    } else if !schema.account {
        report["details"]["accountQuery"]["state"] = json!("unsupported");
    }
    // No account call when status says logged out; catalog is independently discovered when authenticated.
    if !auth {
        report["messages"].as_array_mut().unwrap().push(json!(
            "当前未登录，未继续账户或远端模型查询；此检查不会自动登录。"
        ));
        return report;
    }
    if schema.models {
        match read(ReadCommand::ImageModels).and_then(|data| catalog(&data)) {
            Ok(models) => {
                report["details"]["modelSpecifications"] = json!(models
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|model| model["specification"].clone())
                    .collect::<Vec<_>>());
                let display: Vec<_> = models
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(
                        |model| json!({"id":model["id"],"name":model["name"],"kind":model["kind"]}),
                    )
                    .collect();
                report["catalog"] = json!({"state":"available","models":display});
                report["details"]["checks"]["model list --type image"] = json!({"state":"ready"});
                // The schema establishes a read-only catalog command, not that
                // its result required a network request. Only server-authenticated
                // `auth account` success provides network evidence here.
            }
            Err(error) => {
                report["catalog"]["state"] = json!("unavailable");
                add_error(&mut report, ReadCommand::ImageModels, &error);
            }
        }
    } else {
        report["catalog"]["state"] = json!("unavailable");
        report["details"]["checks"]["model list --type image"] = json!({"state":"unsupported"});
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    const MODELS: &[u8] = include_bytes!("../../fixtures/synthetic/canvas-readiness/models.json");
    #[test]
    fn compact_catalog_keeps_canonical_ids_and_absent_capabilities_unknown() {
        let data = envelope(MODELS).unwrap();
        let models = catalog(&data).unwrap();
        assert_eq!(models[0]["id"], "seedream_5.0_lite");
        assert_ne!(models[0]["id"], "5.0lite");
        assert_eq!(models[0]["specification"]["modes"][0]["vipRequired"], false);
        assert!(models[0]["specification"]["modes"][1]
            .get("flags")
            .is_none());
        assert!(models[1]["specification"]["modes"][0]
            .get("vipRequired")
            .is_none());
        assert!(models[1]["specification"]["modes"][0]
            .get("references")
            .is_none());
        assert_eq!(
            models[1]["specification"]["modes"][1]["references"]["types"],
            json!([])
        );
        assert!(models[2]["specification"]["modes"][0]
            .get("flags")
            .is_none());
    }
    #[test]
    fn model_details_shape_and_invalid_catalog_fail_closed() {
        for data in [
            json!({"query":"5.0", "matches":[{"generation":{}}]}),
            json!({"items":[{"type":"image","model":"id","modes":[{"name":"t2i","vipRequired":"false"}]}]}),
            json!({"items":[{"type":"video","model":"id"}]}),
            json!({"items":[{"type":"image","model":"same"},{"type":"image","model":"same"}]}),
        ] {
            assert!(catalog(&data).is_err());
        }
        assert_eq!(catalog(&json!({"items":[]})).unwrap(), json!([]));
    }
    #[test]
    fn numeric_flag_bounds_accept_negative_but_integer_counts_reject_fractions() {
        let mode =
            json!({"name":"t2i","flags":[{"flag":"--seed","required":false,"min":-1,"max":100}]});
        assert_eq!(mode_spec(&mode).unwrap()["flags"][0]["min"], -1);
        assert!(
            mode_spec(&json!({"name":"i2i","references":{"min":0,"max":1.5,"types":[]}})).is_err()
        );
        assert!(mode_spec(
            &json!({"name":"t2i","flags":[{"flag":"--prompt","required":true,"maxLength":1.5}]})
        )
        .is_err());
    }
    #[test]
    fn envelopes_fail_closed_and_never_retain_private_errors() {
        for bytes in [
            b"not JSON".as_slice(),
            br#"{"schemaVersion":"2","ok":true,"data":{}}"#,
            br#"{"schemaVersion":"1","ok":true,"data":[]}"#,
        ] {
            assert!(envelope(bytes).is_err());
        }
        let error = envelope(br#"{"schemaVersion":"1","ok":false,"error":{"reason":"credential_store_permission_denied","message":"PRIVATE_SENTINEL","localStorage":{"directory":"PRIVATE_SENTINEL"}}}"#).unwrap_err();
        assert_eq!(error.code, "CANVAS_CREDENTIAL_STORE_PERMISSION_DENIED");
        assert!(!error.message.contains("PRIVATE"));
    }
    #[test]
    fn legacy_discriminator_never_treats_canvas_as_legacy() {
        assert!(legacy_version_supported(b"1d0bd17-dirty\n"));
        assert!(legacy_version_supported(b"1.4.19\n"));
        assert!(legacy_version_supported(b"dreamina version v1.4.19"));
        for version in [
            b"1.0.2".as_slice(),
            b"dreamina-canvas 1.0.2",
            b"dreamina-canvas 1.4.19",
            br#"{"schemaVersion":"1","ok":true,"data":{"version":"1.4.19"}}"#,
            b"unknown",
        ] {
            assert!(!legacy_version_supported(version));
        }
    }
    #[test]
    fn legacy_bare_git_build_json_keeps_version_and_requires_consistent_public_metadata() {
        for (version, commit) in [("1d0bd17-dirty", "1d0bd17"), ("abcdef0", "abcdef0"),
            ("0123456789abcdef0123456789abcdef01234567-dirty", "0123456789abcdef0123456789abcdef01234567")] {
            let build = json!({"version":version,"commit":commit,"build_time":"2026-10-03T13:51:54Z"});
            assert_eq!(legacy_version_label(build.to_string().as_bytes()), Some(version.to_string()));
        }
        for build in [
            json!({"version":"1d0bd17-dirty","commit":"deadbee","build_time":"2026-10-03T13:51:54Z"}),
            json!({"version":"abcdef-dirty","commit":"abcdef","build_time":"2026-10-03T13:51:54Z"}),
            json!({"version":"123456g-dirty","commit":"123456g","build_time":"2026-10-03T13:51:54Z"}),
            json!({"version":"1d0bd17-dirty","commit":"1d0bd17","build_time":"unknown"}),
            json!({"version":"1d0bd17-dirty","commit":"1d0bd17","build_time":123}),
            json!({"version":"1d0bd17-dirty","commit":"1d0bd17"}),
            json!({"schemaVersion":"1","ok":true,"data":{"version":"1d0bd17-dirty","commit":"1d0bd17","build_time":"2026-10-03T13:51:54Z"}}),
            json!({"version":"1d0bd17-dirty","commit":"1d0bd17","build_time":"2026-10-03T13:51:54Z","schemaVersion":"1"}),
        ] {
            assert!(legacy_version_label(build.to_string().as_bytes()).is_none(), "{build}");
        }
        assert!(legacy_version_label(b"{\"version\": 1.4.19").is_none());
    }
    #[test]
    fn allowlist_contains_no_mutating_or_generation_commands() {
        for command in [
            ReadCommand::Version,
            ReadCommand::Schema,
            ReadCommand::AuthStatus,
            ReadCommand::AuthAccount,
            ReadCommand::ImageModels,
        ] {
            let args = command.args();
            assert!(!args.iter().any(|arg| [
                "--yes",
                "login",
                "logout",
                "create",
                "edit",
                "run",
                "quote",
                "user_credit",
                "submit",
                "upload"
            ]
            .contains(arg)));
            assert_eq!(&args[..2], &["--format", "json"]);
        }
    }
}

fn supported_version(data: &Value) -> ProviderResult<()> {
    if data["version"] != "1.0.2"
        || data["commit"] != "09e50ac"
        || data["edition"] != "public"
        || data["distribution"] != "cn"
    {
        return Err(failure("CANVAS_VERSION_UNSUPPORTED"));
    }
    Ok(())
}
struct SchemaCapabilities {
    account: bool,
    models: bool,
}
fn named<'a>(items: &'a Value, name: &str) -> Option<&'a Value> {
    let items = items.as_array().filter(|items| items.len() <= 256)?;
    let mut matches = items.iter().filter(|item| item["name"] == name);
    let item = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(item)
}
fn read_leaf(value: &Value) -> bool {
    value["reads"] == true
        && value["writes"] == false
        && value["confirmation"] == false
        && value["formats"]
            .as_array()
            .is_some_and(|formats| formats.contains(&json!("json")))
        && value.get("arguments").is_none_or(|args| {
            args.as_array()
                .is_some_and(|args| args.iter().all(|arg| arg["required"] == false))
        })
}
fn field(data: &Value, name: &str, kind: &str) -> bool {
    data["type"] == "object"
        && named(&data["fields"], name).is_some_and(|field| field["type"] == kind)
}
fn no_required_flags(node: &Value) -> bool {
    node["flags"]
        .as_array()
        .is_some_and(|flags| flags.iter().all(|flag| flag["required"] == false))
}
fn supported_schema(data: &Value) -> ProviderResult<SchemaCapabilities> {
    if data["schemaVersion"] != "1"
        || data["command"] != "dreamina-canvas"
        || data["edition"] != "public"
        || data["distribution"] != "cn"
    {
        return Err(failure("CANVAS_SCHEMA_UNSUPPORTED"));
    }
    let flags = &data["globalFlags"];
    for (name, kind) in [
        ("format", "string"),
        ("non-interactive", "boolean"),
        ("profile", "string"),
        ("region", "string"),
    ] {
        if !named(flags, name).is_some_and(|flag| flag["type"] == kind && flag["required"] == false)
        {
            return Err(failure("CANVAS_FLAGS_UNSUPPORTED"));
        }
    }
    if !flags
        .as_array()
        .is_some_and(|flags| flags.iter().all(|flag| flag["required"] == false))
        || named(flags, "region").unwrap()["constraints"] != "cn"
        || !named(flags, "format").unwrap()["constraints"]
            .as_str()
            .is_some_and(|s| s.split('|').any(|v| v == "json"))
    {
        return Err(failure("CANVAS_FLAGS_UNSUPPORTED"));
    }
    for name in ["version", "schema"] {
        if !named(&data["subcommands"], name)
            .is_some_and(|node| read_leaf(node) && no_required_flags(node))
        {
            return Err(failure("CANVAS_SCHEMA_UNSUPPORTED"));
        }
    }
    let auth = named(&data["subcommands"], "auth")
        .ok_or_else(|| failure("CANVAS_AUTH_STATUS_UNSUPPORTED"))?;
    // The auth parent legitimately writes. Validate only the exact called leaf.
    if !named(&auth["subcommands"], "status").is_some_and(|node| {
        read_leaf(node)
            && no_required_flags(node)
            && [
                ("loggedIn", "boolean"),
                ("profile", "string"),
                ("region", "string"),
                ("version", "string"),
                ("edition", "string"),
                ("distribution", "string"),
            ]
            .iter()
            .all(|(name, kind)| field(&node["successData"], name, kind))
    }) {
        return Err(failure("CANVAS_AUTH_STATUS_UNSUPPORTED"));
    }
    let account = named(&auth["subcommands"], "account").is_some_and(|node| {
        read_leaf(node)
            && no_required_flags(node)
            && field(&node["successData"], "userId", "integer")
    });
    let models = named(&data["subcommands"], "model")
        .and_then(|node| named(&node["subcommands"], "list"))
        .is_some_and(|node| {
            read_leaf(node)
                && node["flags"].as_array().is_some_and(|flags| {
                    flags
                        .iter()
                        .all(|flag| flag["name"] == "type" || flag["required"] == false)
                })
                && named(&node["flags"], "type").is_some_and(|flag| {
                    flag["type"] == "string"
                        && flag["required"] == true
                        && flag["constraints"]
                            .as_str()
                            .is_some_and(|v| v.split('|').any(|s| s == "image"))
                })
                && field(&node["successData"], "items", "array<object>")
                && named(&node["successData"]["fields"], "items").is_some_and(|items| {
                    [
                        ("type", "string"),
                        ("model", "string"),
                        ("modes", "array<object>"),
                    ]
                    .iter()
                    .all(|(name, kind)| {
                        named(&items["fields"], name).is_some_and(|field| field["type"] == *kind)
                    })
                })
        });
    Ok(SchemaCapabilities { account, models })
}

#[cfg(test)]
mod report_tests {
    use super::*;
    pub(super) fn fixture(command: ReadCommand) -> Vec<u8> {
        match command {
            ReadCommand::Version => {
                include_bytes!("../../fixtures/synthetic/canvas-readiness/version.json").to_vec()
            }
            ReadCommand::Schema => {
                include_bytes!("../../fixtures/synthetic/canvas-readiness/schema.json").to_vec()
            }
            ReadCommand::AuthStatus => {
                include_bytes!("../../fixtures/synthetic/canvas-readiness/auth-status.json")
                    .to_vec()
            }
            ReadCommand::AuthAccount => {
                include_bytes!("../../fixtures/synthetic/canvas-readiness/account.json").to_vec()
            }
            ReadCommand::ImageModels => {
                include_bytes!("../../fixtures/synthetic/canvas-readiness/models.json").to_vec()
            }
        }
    }
    #[test]
    fn exact_schema_leaf_permission_preserves_readonly_auth_under_write_parent() {
        let mut schema = envelope(&fixture(ReadCommand::Schema)).unwrap();
        let caps = supported_schema(&schema).unwrap();
        assert!(caps.account && caps.models);
        schema["subcommands"][1]["subcommands"][1]["writes"] = json!(true);
        assert_eq!(
            supported_schema(&schema).err().unwrap().code,
            "CANVAS_AUTH_STATUS_UNSUPPORTED"
        );
        let mut schema = envelope(&fixture(ReadCommand::Schema)).unwrap();
        schema["subcommands"][1]["subcommands"][0]["writes"] = json!(true);
        let caps = supported_schema(&schema).unwrap();
        assert!(!caps.account && caps.models);
    }
    #[test]
    fn schema_shape_and_unknown_global_flags_fail_closed() {
        let schema = envelope(&fixture(ReadCommand::Schema)).unwrap();
        for field in [
            "subcommands",
            "globalFlags",
            "command",
            "edition",
            "distribution",
            "schemaVersion",
        ] {
            let mut changed = schema.clone();
            changed.as_object_mut().unwrap().remove(field);
            assert!(supported_schema(&changed).is_err(), "{field}");
        }
        let mut changed = schema.clone();
        changed["globalFlags"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"unknown","required":true}));
        assert!(supported_schema(&changed).is_err());
        let mut changed = schema.clone();
        changed["globalFlags"][1]["type"] = json!("string");
        assert!(supported_schema(&changed).is_err());
        let mut changed = schema.clone();
        changed["subcommands"][0]["subcommands"][0]["flags"][0]["name"] = json!("flag");
        assert!(!supported_schema(&changed).unwrap().models);
    }
    #[test]
    fn complete_report_is_readonly_private_and_dynamic_with_no_quota_claim() {
        let mut commands = vec![];
        let report = inspect(base_report(), |command| {
            commands.push(command);
            Ok(fixture(command))
        });
        assert_eq!(
            commands,
            [
                ReadCommand::Version,
                ReadCommand::Schema,
                ReadCommand::AuthStatus,
                ReadCommand::AuthAccount,
                ReadCommand::ImageModels
            ]
        );
        assert_eq!(report["protocol"]["state"], "ready");
        assert_eq!(report["authentication"]["state"], "authenticated");
        assert_eq!(report["authentication"]["identityVerified"], true);
        assert_eq!(
            report["network"],
            json!({"state":"verified_read_only","proof":["auth account"]})
        );
        assert_eq!(report["catalog"]["models"][0]["id"], "seedream_5.0_lite");
        assert_eq!(report["details"]["accountQuery"]["isVip"], false);
        assert_eq!(report["details"]["quota"]["state"], "unsupported");
        assert!(report["details"]["quota"]["remaining"].is_null());
        assert_eq!(report["details"]["freeImageEligibility"], "unverified");
        assert_eq!(
            report["generation"],
            json!({"enabled":false,"testPerformed":false,"calls":0})
        );
        for secret in [
            "123456789",
            "PRIVATE_EXPIRY_SENTINEL",
            "PRIVATE_CREDENTIAL_SENTINEL",
            "userId",
            "expiresAt",
            "credentialStorage",
        ] {
            assert!(!report.to_string().contains(secret), "{secret}");
        }
    }
    #[test]
    fn failed_account_does_not_erase_local_login_or_independent_catalog() {
        let report = inspect(base_report(), |command| {
            if command == ReadCommand::AuthAccount {
                Err(failure("CANVAS_NETWORK_TIMEOUT"))
            } else {
                Ok(fixture(command))
            }
        });
        assert_eq!(report["authentication"]["state"], "authenticated");
        assert_eq!(report["authentication"]["identityVerified"], false);
        assert_eq!(report["details"]["accountQuery"]["state"], "unavailable");
        assert_eq!(report["catalog"]["state"], "available");
        assert_eq!(report["network"], json!({"state":"not_tested","proof":[]}));
    }
    #[test]
    fn catalog_failure_preserves_account_and_empty_catalog_is_valid() {
        let failed = inspect(base_report(), |command| {
            if command == ReadCommand::ImageModels {
                Err(failure("CANVAS_NETWORK_TIMEOUT"))
            } else {
                Ok(fixture(command))
            }
        });
        assert_eq!(failed["authentication"]["identityVerified"], true);
        assert_eq!(failed["catalog"]["state"], "unavailable");
        assert_eq!(
            failed["network"],
            json!({"state":"verified_read_only","proof":["auth account"]})
        );
        let empty = inspect(base_report(), |command| {
            if command == ReadCommand::ImageModels {
                Ok(br#"{"schemaVersion":"1","ok":true,"data":{"items":[]}}"#.to_vec())
            } else {
                Ok(fixture(command))
            }
        });
        assert_eq!(empty["catalog"], json!({"state":"available","models":[]}));
    }
    #[test]
    fn protocol_or_auth_uncertainty_stops_without_remote_account_queries() {
        for changed in [
            ReadCommand::Version,
            ReadCommand::Schema,
            ReadCommand::AuthStatus,
        ] {
            let mut calls = vec![];
            let report = inspect(base_report(), |command| {
                calls.push(command);
                if command == changed {
                    Err(failure("CANVAS_INVALID_JSON"))
                } else {
                    Ok(fixture(command))
                }
            });
            assert!(!calls.contains(&ReadCommand::AuthAccount));
            assert!(!calls.contains(&ReadCommand::ImageModels));
            assert_eq!(report["generation"]["calls"], 0);
            assert_ne!(report["authentication"]["state"], "not_logged_in");
            if changed != ReadCommand::Version {
                assert_eq!(report["installation"]["version"], "1.0.2");
            }
        }
    }
    #[test]
    fn unsupported_account_or_catalog_leaf_skips_only_that_command() {
        for (parent, child, skipped) in [
            (1, 0, ReadCommand::AuthAccount),
            (0, 0, ReadCommand::ImageModels),
        ] {
            let mut calls = vec![];
            let report = inspect(base_report(), |command| {
                calls.push(command);
                let mut data: Value = serde_json::from_slice(&fixture(command)).unwrap();
                if command == ReadCommand::Schema {
                    data["data"]["subcommands"][parent]["subcommands"][child]["writes"] =
                        json!(true);
                }
                Ok(serde_json::to_vec(&data).unwrap())
            });
            assert!(!calls.contains(&skipped));
            assert_eq!(report["protocol"]["state"], "ready");
            assert_eq!(report["authentication"]["state"], "authenticated");
            if skipped == ReadCommand::AuthAccount {
                assert_eq!(report["catalog"]["state"], "available");
                assert_eq!(report["network"], json!({"state":"not_tested","proof":[]}));
            }
        }
    }
    #[test]
    fn explicit_logged_out_and_context_mismatch_never_start_login() {
        for mismatch in [false, true] {
            let mut calls = vec![];
            let report = inspect(base_report(), |command| {
                calls.push(command);
                let mut data: Value = serde_json::from_slice(&fixture(command)).unwrap();
                if command == ReadCommand::AuthStatus {
                    if mismatch {
                        data["data"]["region"] = json!("unknown");
                    } else {
                        data["data"]["loggedIn"] = json!(false);
                    }
                }
                Ok(serde_json::to_vec(&data).unwrap())
            });
            assert_eq!(calls.len(), 3);
            assert_eq!(
                report["authentication"]["state"],
                if mismatch { "unknown" } else { "not_logged_in" }
            );
            assert_eq!(report["catalog"]["state"], "not_checked");
        }
    }
    #[test]
    fn unsupported_versions_stop_before_schema_or_account() {
        for changed in ["version", "edition", "distribution", "commit"] {
            let mut calls = vec![];
            let report = inspect(base_report(), |command| {
                calls.push(command);
                let mut data: Value = serde_json::from_slice(&fixture(command)).unwrap();
                data["data"][changed] = json!("unknown");
                Ok(serde_json::to_vec(&data).unwrap())
            });
            assert_eq!(calls, [ReadCommand::Version]);
            assert_eq!(report["protocol"]["state"], "unsupported");
        }
    }
}
