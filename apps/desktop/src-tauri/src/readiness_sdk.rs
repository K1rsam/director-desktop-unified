//! Genuine official-package import and bounded, non-generating bridge health.
//! This module never uses the generation transport, a credential directory,
//! the Codex runtime, an account endpoint, or a caller-provided fixture.
use crate::text_sdk::{PROTOCOL, SDK_IMPLEMENTATION_SHA, SDK_VERSION};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

const SDK_PACKAGE_SHA: &str = "5ffdee6d6135684e502d2d6010bae3690c0e2acb44e95d1e27c14fc5bbfccf30";
const SDK_LICENSE_SHA: &str = "d17f227e4df5da1600391338865ce0f3055211760a36688f816941d58232d8dc";
const MAX_FILE_BYTES: usize = 2_097_152;
const MAX_OUTPUT_BYTES: usize = 32768;
const MAX_STDERR_BYTES: usize = 8192;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(8);
type CheckResult<T> = std::result::Result<T, &'static str>;

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn base_result() -> Value {
    json!({
        "schemaVersion":"provider-readiness/v1", "provider":"codex_sdk",
        "checkedAt":chrono::Utc::now().to_rfc3339(),
        "installation":{"state":"not_checked","version":null,"executable":null},
        "protocol":{"state":"not_checked","name":PROTOCOL},
        "authentication":{"state":"not_checked","identityVerified":false},
        "catalog":{"state":"not_checked","models":[]},
        "network":{"state":"not_tested","proof":[]},
        "generation":{"enabled":false,"testPerformed":false,"calls":0},
        "messages":[],
        "details":{"sdkPackage":"@openai/codex-sdk","sdkVersion":SDK_VERSION,
            "sdkInstalled":false,"nodeInstalled":false,"officialRuntimeChecked":false,
            "accountChecked":false,"modelTested":false}
    })
}

fn safe_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
}

fn resource_bytes(root: &Path, relative: &str) -> CheckResult<Vec<u8>> {
    let mut path = root.to_path_buf();
    for part in Path::new(relative).components() {
        if !matches!(part, Component::Normal(_)) {
            return Err("SDK_RESOURCE_PATH_INVALID");
        }
        path.push(part);
        if std::fs::symlink_metadata(&path)
            .map_err(|_| "SDK_RESOURCE_MISSING")?
            .is_symlink()
        {
            return Err("SDK_RESOURCE_SYMLINK_REJECTED");
        }
    }
    let file = std::fs::File::open(path).map_err(|_| "SDK_RESOURCE_MISSING")?;
    let metadata = file.metadata().map_err(|_| "SDK_RESOURCE_INVALID")?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("SDK_RESOURCE_INVALID");
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(file, MAX_FILE_BYTES as u64 + 1),
        &mut bytes,
    )
    .map_err(|_| "SDK_RESOURCE_READ_FAILED")?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("SDK_RESOURCE_LIMIT");
    }
    Ok(bytes)
}

fn verify_resources(root: &Path) -> CheckResult<()> {
    if !safe_absolute(root) {
        return Err("SDK_RESOURCE_PATH_INVALID");
    }
    let package = resource_bytes(root, "node_modules/@openai/codex-sdk/package.json")?;
    let implementation = resource_bytes(root, "node_modules/@openai/codex-sdk/dist/index.js")?;
    if sha256(&package) != SDK_PACKAGE_SHA || sha256(&implementation) != SDK_IMPLEMENTATION_SHA {
        return Err("SDK_PACKAGE_IDENTITY_MISMATCH");
    }
    let package: Value = serde_json::from_slice(&package).map_err(|_| "SDK_PACKAGE_INVALID")?;
    if package["name"] != "@openai/codex-sdk" || package["version"] != SDK_VERSION {
        return Err("SDK_PACKAGE_IDENTITY_MISMATCH");
    }
    for (name, expected) in [
        (
            "src/health.mjs",
            include_bytes!("../../../../sdk-bridge/src/health.mjs").as_slice(),
        ),
        (
            "src/bridge.mjs",
            include_bytes!("../../../../sdk-bridge/src/bridge.mjs").as_slice(),
        ),
        (
            "src/main.mjs",
            include_bytes!("../../../../sdk-bridge/src/main.mjs").as_slice(),
        ),
        (
            "src/native-runtime.mjs",
            include_bytes!("../../../../sdk-bridge/src/native-runtime.mjs").as_slice(),
        ),
        (
            "THIRD_PARTY_NOTICES.md",
            include_bytes!("../../../../sdk-bridge/THIRD_PARTY_NOTICES.md").as_slice(),
        ),
        (
            "package-lock.json",
            include_bytes!("../../../../sdk-bridge/package-lock.json").as_slice(),
        ),
    ] {
        if sha256(&resource_bytes(root, name)?) != sha256(expected) {
            return Err("SDK_BRIDGE_RESOURCE_IDENTITY_MISMATCH");
        }
    }
    let license = resource_bytes(root, "node_modules/@openai/codex-sdk/LICENSE")?;
    if sha256(&license) != SDK_LICENSE_SHA {
        return Err("SDK_LICENSE_INVALID");
    }
    Ok(())
}

fn resource_location(resource_root: Option<&Path>) -> CheckResult<(PathBuf, &'static str)> {
    if let Some(root) = resource_root {
        if !safe_absolute(root) {
            return Err("SDK_RESOURCE_PATH_INVALID");
        }
        let packaged = root.join("sdk-readiness");
        if packaged.is_dir() {
            return Ok((
                std::fs::canonicalize(packaged).map_err(|_| "SDK_RESOURCE_MISSING")?,
                "app_resources",
            ));
        }
    }
    // A development build can use its sibling bridge. Release builds only use
    // native app resources; no path originating in IPC is accepted.
    #[cfg(debug_assertions)]
    {
        let staged = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/sdk-readiness");
        if staged.join("manifest.json").is_file() {
            return Ok((
                std::fs::canonicalize(staged).map_err(|_| "SDK_RESOURCE_MISSING")?,
                "development_resources",
            ));
        }
        if resource_root.is_none() {
            return Ok((
                std::fs::canonicalize(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sdk-bridge"),
                )
                .map_err(|_| "SDK_RESOURCE_MISSING")?,
                "development_source",
            ));
        }
    }
    Err("SDK_BUNDLED_RESOURCES_MISSING")
}

fn node_candidates(settings: &Value) -> CheckResult<Vec<PathBuf>> {
    if let Some(value) = settings.get("textSdkNodePath") {
        let value = value.as_str().ok_or("SDK_NODE_PATH_INVALID")?;
        if !value.trim().is_empty() {
            let path = PathBuf::from(value);
            if !safe_absolute(&path) {
                return Err("SDK_NODE_PATH_INVALID");
            }
            return Ok(vec![path]);
        }
    }
    #[cfg(target_os = "macos")]
    let candidates = vec![
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ];
    #[cfg(windows)]
    let candidates = {
        let mut candidates = vec![PathBuf::from(r"C:\Program Files\nodejs\node.exe")];
        for name in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| safe_absolute(p))
            {
                candidates.push(root.join(if name == "LOCALAPPDATA" {
                    r"Programs\nodejs\node.exe"
                } else {
                    r"nodejs\node.exe"
                }));
            }
        }
        candidates
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let candidates = vec![
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ];
    Ok(candidates)
}

pub(crate) async fn native_components(
    settings: &Value,
    resource_root: &Path,
) -> crate::store::Result<(PathBuf, String, String, PathBuf)> {
    let (root, _) = resource_location(Some(resource_root)).map_err(str::to_owned)?;
    verify_resources(&root).map_err(str::to_owned)?;
    let paths = node_candidates(settings).map_err(str::to_owned)?;
    let node = paths
        .iter()
        .find(|p| p.exists())
        .ok_or("SDK_NODE_MISSING")?;
    let (node, sha) = verify_node(node).map_err(str::to_owned)?;
    let version = run_health(&node, &root, None)
        .await
        .map_err(str::to_owned)?;
    Ok((node, sha, version, root))
}

fn verify_node(path: &Path) -> CheckResult<(PathBuf, String)> {
    if !safe_absolute(path) {
        return Err("SDK_NODE_PATH_INVALID");
    }
    let path = std::fs::canonicalize(path).map_err(|_| "SDK_NODE_MISSING")?;
    let mut file = std::fs::File::open(&path).map_err(|_| "SDK_NODE_MISSING")?;
    let metadata = file.metadata().map_err(|_| "SDK_NODE_INVALID")?;
    if !metadata.is_file() || metadata.len() < 4 || metadata.len() > 536_870_912 {
        return Err("SDK_NODE_INVALID");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("SDK_NODE_NOT_EXECUTABLE");
        }
    }
    let mut header = [0; 4];
    std::io::Read::read_exact(&mut file, &mut header).map_err(|_| "SDK_NODE_INVALID")?;
    // Native binaries only. Never run a shell/batch wrapper selected by PATH.
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
        return Err("SDK_NODE_NATIVE_BINARY_REQUIRED");
    }
    std::io::Seek::rewind(&mut file).map_err(|_| "SDK_NODE_HASH_FAILED")?;
    let mut hasher = Sha256::new();
    let mut bounded = std::io::Read::take(file, 536_870_913);
    let mut buffer = [0; 65536];
    let mut total = 0usize;
    loop {
        let count =
            std::io::Read::read(&mut bounded, &mut buffer).map_err(|_| "SDK_NODE_HASH_FAILED")?;
        if count == 0 {
            break;
        }
        total += count;
        if total > 536_870_912 {
            return Err("SDK_NODE_INVALID");
        }
        hasher.update(&buffer[..count]);
    }
    let digest = format!("{:x}", hasher.finalize());
    Ok((path, digest))
}

async fn bounded_read<R: AsyncRead + Unpin>(reader: R, limit: usize) -> CheckResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "SDK_HEALTH_PIPE_FAILED")?;
    if bytes.len() > limit {
        return Err("SDK_HEALTH_OUTPUT_LIMIT");
    }
    Ok(bytes)
}

fn validate_output(bytes: &[u8]) -> CheckResult<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "SDK_HEALTH_INVALID_UTF8")?;
    if !text.ends_with('\n') {
        return Err("SDK_HEALTH_TRUNCATED_FRAME");
    }
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 3 {
        return Err("SDK_HEALTH_RESPONSE_COUNT");
    }
    let mut node_version = String::new();
    for (index, line) in lines.iter().enumerate() {
        let response: Value = serde_json::from_str(line).map_err(|_| "SDK_HEALTH_INVALID_JSON")?;
        if response["protocolVersion"] != PROTOCOL
            || response["type"] != "response"
            || response["requestId"] != format!("health_{}", index + 1)
            || response["attemptId"] != Value::Null
            || response["ok"] != true
        {
            return Err("SDK_HEALTH_PROTOCOL_MISMATCH");
        }
        let health = &response["health"];
        if health["sdkImported"] != true
            || health["sdkPackage"] != "@openai/codex-sdk"
            || health["sdkVersion"] != SDK_VERSION
            || health["executeCalls"] != 0
            || health["accountChecked"] != false
            || health["networkTested"] != false
            || health["generationEnabled"] != false
            || health["controlRequests"] != index + 1
            || health["allowedMethods"] != json!(["hello", "inspect", "shutdown"])
        {
            return Err("SDK_HEALTH_SCOPE_MISMATCH");
        }
        let version = health["nodeVersion"]
            .as_str()
            .ok_or("SDK_NODE_VERSION_INVALID")?;
        if version.len() > 64
            || !version.starts_with('v')
            || version[1..]
                .split('.')
                .next()
                .and_then(|v| v.parse::<u32>().ok())
                .is_none_or(|major| major < 18)
        {
            return Err("SDK_NODE_VERSION_UNSUPPORTED");
        }
        if index == 0 {
            node_version = version.to_string();
        }
        if version != node_version {
            return Err("SDK_NODE_VERSION_CHANGED");
        }
        let result = &response["result"];
        if index < 2 {
            if result["backend"] != "codex_sdk"
                || result["protocolVersion"] != PROTOCOL
                || result["sdk"]["version"] != SDK_VERSION
                || result["sdk"]["package"] != "@openai/codex-sdk"
                || result["liveCallsAllowed"] != false
                || result["offlineFixtureAvailable"] != false
                || result["available"] != false
                || result["capabilities"]["execute"] != "disabled"
            {
                return Err("SDK_HEALTH_BRIDGE_MISMATCH");
            }
        } else if result["shutdown"] != true {
            return Err("SDK_HEALTH_SHUTDOWN_MISSING");
        }
    }
    Ok(node_version)
}

async fn run_health(node: &Path, root: &Path, state_parent: Option<&Path>) -> CheckResult<String> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("director-sdk-readiness-");
    let temp = if let Some(parent) = state_parent {
        builder.tempdir_in(parent)
    } else {
        builder.tempdir()
    }
    .map_err(|_| "SDK_HEALTH_TEMP_FAILED")?;
    let home = temp.path().join("home");
    std::fs::create_dir(&home).map_err(|_| "SDK_HEALTH_TEMP_FAILED")?;
    let state = temp.path().join("worker");
    let mut command = tokio::process::Command::new(node);
    #[cfg(test)]
    {
        let preload = std::fs::canonicalize(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../sdk-bridge/test/health-no-effects.mjs"),
        )
        .map_err(|_| "SDK_HEALTH_PRELOAD_INVALID")?;
        command.arg("--import").arg(
            crate::text_sdk::node_import_specifier(&preload)
                .map_err(|_| "SDK_HEALTH_PRELOAD_INVALID")?,
        );
    }
    command
        .arg(
            crate::text_sdk::node_path_argument(&root.join("src/health.mjs"))
                .map_err(|_| "SDK_HEALTH_PATH_INVALID")?,
        )
        .arg("--state-dir")
        .arg(&state)
        .env_clear()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("CODEX_HOME", home.join("codex"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("TMPDIR", temp.path())
        .env("TEMP", temp.path())
        .env("TMP", temp.path())
        .current_dir(temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(test)]
    command.env("DIRECTOR_HEALTH_AUDIT", temp.path().join("audit.json"));
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
        let system_root =
            crate::text_sdk::node_system_root().map_err(|_| "SDK_HEALTH_WINDOWS_ENV_INVALID")?;
        command
            .env("SystemRoot", &system_root)
            .env("WINDIR", system_root);
    }
    let (mut child, _registration) = crate::text_executor::spawn_cli_child(&mut command)
        .map_err(|_| "SDK_HEALTH_START_FAILED")?;
    let mut stdin = child.stdin.take().ok_or("SDK_HEALTH_PIPE_FAILED")?;
    let stdout = child.stdout.take().ok_or("SDK_HEALTH_PIPE_FAILED")?;
    let stderr = child.stderr.take().ok_or("SDK_HEALTH_PIPE_FAILED")?;
    let outcome = tokio::time::timeout(HEALTH_TIMEOUT, async {
        for (index, method) in ["hello", "inspect", "shutdown"].iter().enumerate() {
            let request = json!({"protocolVersion":PROTOCOL,"requestId":format!("health_{}", index + 1),
                "attemptId":null,"method":method,"params":null});
            let mut bytes = serde_json::to_vec(&request).map_err(|_| "SDK_HEALTH_REQUEST_INVALID")?;
            bytes.push(b'\n');
            stdin.write_all(&bytes).await.map_err(|_| "SDK_HEALTH_WRITE_FAILED")?;
        }
        stdin.shutdown().await.map_err(|_| "SDK_HEALTH_WRITE_FAILED")?;
        drop(stdin);
        let (out, err, status) = tokio::try_join!(bounded_read(stdout, MAX_OUTPUT_BYTES), bounded_read(stderr, MAX_STDERR_BYTES),
            async { child.wait().await.map_err(|_| "SDK_HEALTH_WAIT_FAILED") })?;
        if !status.success() || !err.is_empty() { return Err("SDK_HEALTH_WORKER_FAILED"); }
        validate_output(&out)
    }).await.unwrap_or(Err("SDK_HEALTH_TIMEOUT"));
    if child.id().is_some() {
        let _ = child.start_kill();
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(_)) => (),
            _ => return Err("SDK_HEALTH_STOP_UNCONFIRMED"),
        }
    }
    #[cfg(test)]
    if outcome.is_ok() {
        let evidence: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("audit.json")).expect("required zero-effect audit"),
        )
        .expect("valid zero-effect audit");
        assert_eq!(evidence["forbiddenEffectAttempts"], 0);
        for key in evidence["environmentKeys"].as_array().unwrap() {
            assert!(
                [
                    "HOME",
                    "USERPROFILE",
                    "CODEX_HOME",
                    "XDG_CONFIG_HOME",
                    "XDG_CACHE_HOME",
                    "TMPDIR",
                    "TEMP",
                    "TMP",
                    "SystemRoot",
                    "WINDIR",
                    "DIRECTOR_HEALTH_AUDIT"
                ]
                .contains(&key.as_str().unwrap())
                    // CoreFoundation adds this locale key after Node starts on
                    // macOS, including when the parent supplies an empty env.
                    // Keep the no-effects audit strict for all other keys.
                    || (cfg!(target_os = "macos")
                        && key.as_str() == Some("__CF_USER_TEXT_ENCODING")),
                "unexpected inherited environment key: {key}"
            );
        }
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(&state).unwrap().count(), 0);
    }
    temp.close().map_err(|_| "SDK_HEALTH_TEMP_CLEANUP_FAILED")?;
    outcome
}

/// Caller must enforce an explicit native user action. `resource_root` comes
/// from Tauri's path resolver, never from an IPC payload or saved setting.
pub async fn check(settings: &Value, resource_root: Option<&Path>) -> Value {
    let mut value = base_result();
    // These historical fixture settings are test-only generation machinery and
    // must not influence a production readiness action, even in debug builds.
    if [
        "sdkOfflineFixture",
        "sdkFixtureDeadlineMs",
        "sdkBridgePath",
        "sdkResourceRoot",
        "sdkReadinessFixture",
    ]
    .iter()
    .any(|key| settings.get(key).is_some())
        || settings["textSdkIdentity"]["runtimeIdentity"] == "bundled-synthetic-fixture"
        || settings["textSdkIdentity"]["accountFingerprint"] == "synthetic-no-account"
    {
        value["installation"]["state"] = json!("invalid");
        value["messages"] = json!([
            "SDK_READINESS_FIXTURE_REJECTED：就绪检查不接受测试 fixture 或调用者提供的桥路径"
        ]);
        return value;
    }
    let (root, source) = match resource_location(resource_root) {
        Ok(location) => location,
        Err(error) => {
            value["installation"]["state"] = json!("missing");
            value["messages"] = json!([error]);
            return value;
        }
    };
    if let Err(error) = verify_resources(&root) {
        value["installation"]["state"] = json!(if error == "SDK_RESOURCE_MISSING" {
            "missing"
        } else {
            "invalid"
        });
        value["messages"] = json!([error]);
        return value;
    }
    value["installation"]["version"] = json!(SDK_VERSION);
    value["details"]["sdkInstalled"] = json!(true);
    value["details"]["resourceSource"] = json!(source);
    value["details"]["sdkImplementationSha256"] = json!(SDK_IMPLEMENTATION_SHA);
    let candidates = match node_candidates(settings) {
        Ok(paths) => paths,
        Err(error) => {
            value["installation"]["state"] = json!("invalid");
            value["messages"] = json!([error]);
            return value;
        }
    };
    let Some(candidate) = candidates.iter().find(|path| path.exists()) else {
        value["installation"]["state"] = json!("missing");
        value["messages"] = json!(["SDK_NODE_MISSING：官方 SDK 资源已安装；未找到本机 Node 18+，请选择本机 Node 可执行文件"]);
        return value;
    };
    let (node, node_sha) = match verify_node(candidate) {
        Ok(node) => node,
        Err(error) => {
            value["installation"]["state"] = json!("invalid");
            value["messages"] = json!([error]);
            return value;
        }
    };
    value["installation"]["state"] = json!("ready");
    value["installation"]["executable"] = json!(node.to_string_lossy());
    value["details"]["nodeInstalled"] = json!(true);
    value["details"]["nodeSha256"] = json!(node_sha);
    match run_health(&node, &root, None).await {
        Ok(version) => {
            value["protocol"]["state"] = json!("ready");
            value["details"]["nodeVersion"] = json!(version);
            value["details"]["checkedMethods"] = json!(["hello", "inspect", "shutdown"]);
            value["details"]["temporaryStateRemoved"] = json!(true);
            value["details"]["workerExited"] = json!(true);
            value["messages"] = json!([
                "官方 SDK 固定版本已导入，Node／桥协议已就绪",
                "未检查账号或模型，未测试网络；SDK 真实生成仍关闭，调用次数为 0"
            ]);
        }
        Err(error) => {
            value["protocol"]["state"] = json!("failed");
            value["messages"] = json!([error, "未检查账号或模型，SDK 真实生成仍关闭"]);
        }
    }
    if value["protocol"]["state"] == "ready" && crate::text_sdk::native_execution_enabled() {
        crate::text_sdk::add_native_readiness(settings, &mut value).await;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    fn genuine_source() -> PathBuf {
        std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sdk-bridge"))
            .unwrap()
    }
    fn genuine_node() -> PathBuf {
        let candidate = std::env::var_os("DIRECTOR_TEST_NODE").map(PathBuf::from).or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|p| p.join(if cfg!(windows) { "node.exe" } else { "node" })).find(|p| p.is_file())
        }).expect("readiness SDK integration requires installed Node and npm ci --ignore-scripts --omit=optional; never skipped");
        verify_node(&candidate).unwrap().0
    }
    fn genuine_packaged_resources() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("sdk-readiness");
        let source = genuine_source();
        for relative in [
            "src/health.mjs",
            "src/bridge.mjs",
            "src/main.mjs",
            "src/native-runtime.mjs",
            "THIRD_PARTY_NOTICES.md",
            "package-lock.json",
            "node_modules/@openai/codex-sdk/package.json",
            "node_modules/@openai/codex-sdk/dist/index.js",
            "node_modules/@openai/codex-sdk/LICENSE",
        ] {
            let file = bundle.join(relative);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, resource_bytes(&source, relative).unwrap()).unwrap();
        }
        temp
    }
    #[tokio::test]
    async fn sdk_readiness_imports_genuine_sdk_and_closes_worker_without_execution() {
        let root = genuine_source();
        verify_resources(&root).expect("pinned SDK resources are mandatory for integration tests");
        let isolated = tempfile::tempdir().unwrap();
        let version = run_health(&genuine_node(), &root, Some(isolated.path()))
            .await
            .unwrap();
        assert!(version.starts_with('v'));
        assert_eq!(std::fs::read_dir(isolated.path()).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn sdk_readiness_resolves_packaged_resources_and_rejects_tampered_sdk() {
        let temp = genuine_packaged_resources();
        let bundle = temp.path().join("sdk-readiness");
        let settings = json!({"textSdkNodePath":genuine_node()});
        let value = check(&settings, Some(temp.path())).await;
        assert_eq!(value["installation"]["state"], "ready", "{value}");
        assert_eq!(value["protocol"]["state"], "ready", "{value}");
        assert_eq!(value["details"]["resourceSource"], "app_resources");
        assert_eq!(
            value["generation"],
            json!({"enabled":false,"testPerformed":false,"calls":0})
        );
        assert_eq!(value["authentication"]["identityVerified"], false);
        std::fs::write(
            bundle.join("node_modules/@openai/codex-sdk/dist/index.js"),
            b"synthetic replacement",
        )
        .unwrap();
        let value = check(&settings, Some(temp.path())).await;
        assert_eq!(value["installation"]["state"], "invalid");
        assert_eq!(value["protocol"]["state"], "not_checked");
        assert_eq!(value["messages"][0], "SDK_PACKAGE_IDENTITY_MISMATCH");
    }
    #[tokio::test]
    async fn sdk_readiness_never_accepts_fixture_or_bridge_settings() {
        for key in [
            "sdkOfflineFixture",
            "sdkFixtureDeadlineMs",
            "sdkBridgePath",
            "sdkResourceRoot",
            "sdkReadinessFixture",
        ] {
            let value = check(&json!({key:"synthetic"}), None).await;
            assert_eq!(value["installation"]["state"], "invalid");
            assert_eq!(value["protocol"]["state"], "not_checked");
            assert_eq!(
                value["generation"],
                json!({"enabled":false,"testPerformed":false,"calls":0})
            );
        }
        let value = check(
            &json!({"textSdkIdentity":{"runtimeIdentity":"bundled-synthetic-fixture"}}),
            None,
        )
        .await;
        assert_eq!(value["installation"]["state"], "invalid");
    }
    #[tokio::test]
    async fn sdk_readiness_reports_missing_node_without_account_or_model_claims() {
        let temp = genuine_packaged_resources();
        let value = check(
            &json!({"textSdkNodePath":temp.path().join("missing-node")}),
            Some(temp.path()),
        )
        .await;
        assert_eq!(value["installation"]["state"], "missing");
        assert_eq!(value["details"]["sdkInstalled"], true);
        assert_eq!(value["authentication"]["state"], "not_checked");
        assert_eq!(value["catalog"]["models"], json!([]));
        assert_eq!(value["network"]["state"], "not_tested");
        #[cfg(not(debug_assertions))]
        {
            let missing_resources = check(&json!({}), None).await;
            assert_eq!(missing_resources["installation"]["state"], "missing");
            assert_eq!(missing_resources["details"]["sdkInstalled"], false);
            assert_eq!(
                missing_resources["messages"][0],
                "SDK_BUNDLED_RESOURCES_MISSING"
            );
        }
    }
    #[test]
    fn sdk_readiness_rejects_relative_paths_and_shell_wrappers() {
        assert!(node_candidates(&json!({"textSdkNodePath":"node"})).is_err());
        assert!(node_candidates(&json!({"textSdkNodePath":"/tmp/../node"})).is_err());
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("node");
        std::fs::write(&file, b"#!/bin/sh\necho v24.0.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(
            verify_node(&file).unwrap_err(),
            "SDK_NODE_NATIVE_BINARY_REQUIRED"
        );
    }
    #[tokio::test]
    async fn sdk_readiness_bounded_output_rejects_flood_and_truncated_protocol() {
        assert_eq!(
            bounded_read(&vec![b'x'; 65][..], 64).await.unwrap_err(),
            "SDK_HEALTH_OUTPUT_LIMIT"
        );
        assert_eq!(
            validate_output(b"{}").unwrap_err(),
            "SDK_HEALTH_TRUNCATED_FRAME"
        );
        assert_eq!(
            validate_output(b"{}\n").unwrap_err(),
            "SDK_HEALTH_RESPONSE_COUNT"
        );
    }
}
