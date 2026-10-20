//! Official TypeScript SDK transport. Production execution is deliberately gated.
//! Rust owns domain state; this module owns a bounded, explicitly located Node worker.
use crate::{
    pipeline::digest,
    store::{s, Result},
    text_transport::{Backend, FrozenAttempt, Observation, ObservationKind, TextTransport},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

pub const PROTOCOL: &str = "director-sdk-bridge/v1";
pub const SDK_VERSION: &str = "0.160.1";
pub const SDK_IMPLEMENTATION_SHA: &str =
    "d62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f";
pub const POLICY: &str = "sdk-text-only-offline/v1";
pub const NATIVE_POLICY: &str = "sdk-text-only-native/v1";
#[path = "sdk_native.rs"]
mod native;
#[path = "text_sdk_passive.rs"]
mod passive;
pub(crate) use passive::historical_receipt;
#[path = "text_sdk_local_recovery.rs"]
mod local_recovery;
pub(crate) use local_recovery::{local_preparation_evidence, local_recovery_identity_bound, recovered_receipt_bound, LOCAL_PATH_BRIDGE_SHA};
#[path = "text_sdk_schema_recovery.rs"]
mod schema_recovery;
pub(crate) use schema_recovery::{schema_preparation_evidence, schema_recovery_identity_bound, schema_recovered_receipt_bound, schema_recovery_record_bound};
#[path = "text_sdk_wrapper_repair.rs"]
mod wrapper_repair;
pub(crate) use wrapper_repair::{wrapper_preparation_evidence, repaired_receipt_bound, wrapper_repair_record_bound, WRAPPER_FAILED_BRIDGE_SHA, WRAPPER_FAILED_SUPERVISOR_SHA};
#[cfg(test)]
pub(crate) use wrapper_repair::test_append_wrapper_repair;
pub(crate) async fn prepare_local_recovery_handoff(settings: &Value) -> Result<SupervisorHandoff> {
    native::prepare_local_recovery_handoff(settings).await
}
pub(crate) fn verify_local_recovery_bytes(old: &Value, new: &Value, hashes: &Value) -> Result<()> {
    native::verify_local_recovery_bytes(old, new, hashes)
}
pub(crate) async fn prepare_schema_recovery_handoff(settings: &Value) -> Result<SupervisorHandoff> {
    native::prepare_schema_recovery_handoff(settings).await
}
pub(crate) fn verify_schema_recovery_bytes(old: &Value, new: &Value, hashes: &Value) -> Result<()> {
    native::verify_schema_recovery_bytes(old,new,hashes)
}
const SCHEMA_CONTROLLER: &str = include_str!("../../../../sdk-bridge/src/schema-preparation-controller.mjs");
pub(crate) fn schema_controller_sha256() -> String {digest(SCHEMA_CONTROLLER)}
pub use native::{enable_native_execution, native_execution_enabled, run_runtime_wrapper};
const FRAME_LIMIT: usize = 1_048_576;
const EVENT_LIMIT: usize = 2048;
const JOURNAL_LIMIT: usize = 16_777_216;
const STDERR_LIMIT: usize = 1_048_576;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);
pub type Observer = Arc<dyn Fn(&str, &Value) -> Result<()> + Send + Sync>;

/// Deliberately distinct from the historical CLI's strict three-field identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SdkIdentity {
    pub version: String,
    pub provider: String,
    pub sdk_package: String,
    pub sdk_version: String,
    pub sdk_implementation_sha256: String,
    pub bridge_version: String,
    pub bridge_sha256: String,
    pub protocol_version: String,
    pub node_executable: String,
    pub node_sha256: String,
    pub node_version: String,
    pub runtime_package_version: String,
    pub runtime_identity: String,
    pub permission_profile: String,
    pub account_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_home: Option<String>,
}
impl SdkIdentity {
    /// Historical evidence is not execution capability. Only the bridge content
    /// pin may differ; original identity bytes remain frozen and unmodified.
    pub(crate) fn validate_receipt(&self) -> Result<()> {
        if self.bridge_sha256.len() != 64
            || !self.bridge_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("SDK_IDENTITY_MISMATCH: 历史桥哈希无效".into());
        }
        let mut structural = self.clone();
        structural.bridge_sha256 = if self.version == "official-sdk-identity/v2" {
            compiled_native_bridge_sha()
        } else { compiled_bridge_sha() };
        structural.validate()
    }
    pub fn validate(&self) -> Result<()> {
        if !["official-sdk-identity/v1", "official-sdk-identity/v2"]
            .contains(&self.version.as_str())
            || self.provider != "codex_sdk"
            || self.sdk_package != "@openai/codex-sdk"
            || self.sdk_version != SDK_VERSION
            || self.sdk_implementation_sha256 != SDK_IMPLEMENTATION_SHA
            || self.bridge_version != "0.1.0"
            || self.protocol_version != PROTOCOL
            || !Path::new(&self.node_executable).is_absolute()
            || !self.node_version.starts_with('v')
            || self.node_version.len() < 3
            || self.node_sha256.len() != 64
            || !self.node_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(
                "SDK_IDENTITY_MISMATCH: 官方包、桥、权限或运行身份不匹配；禁止执行/回退".into(),
            );
        }
        if self.version == "official-sdk-identity/v1" {
            if self.runtime_package_version != SDK_VERSION
                || self.permission_profile != POLICY
                || self.bridge_sha256 != compiled_bridge_sha()
                || !["unverified-no-live", "bundled-synthetic-fixture"]
                    .contains(&self.runtime_identity.as_str())
                || !["not-inspected-no-account", "synthetic-no-account"]
                    .contains(&self.account_fingerprint.as_str())
                || [
                    &self.runtime_executable,
                    &self.runtime_sha256,
                    &self.supervisor_executable,
                    &self.supervisor_sha256,
                    &self.resource_directory,
                    &self.credential_home,
                    &self.codex_home,
                ]
                .iter()
                .any(|v| v.is_some())
            {
                return Err("SDK_IDENTITY_MISMATCH: 离线身份不得带入真实运行器".into());
            }
        } else {
            native::validate_identity(self)?;
        }
        Ok(())
    }
}
fn compiled_bridge_sha() -> String {
    digest(&format!(
        "{}{}{}{}",
        include_str!("../../../../sdk-bridge/src/main.mjs"),
        include_str!("../../../../sdk-bridge/src/bridge.mjs"),
        include_str!("../../../../sdk-bridge/test/fixture-runtime.mjs"),
        include_str!("../../../../sdk-bridge/test/fixtures/fake-codex.mjs")
    ))
}
pub fn identity(settings: &Value) -> Result<Value> {
    let value = settings
        .get("textSdkIdentity")
        .ok_or("SDK_IDENTITY_MISSING: 新工作流尚未冻结官方 SDK 身份")?;
    let identity: SdkIdentity =
        serde_json::from_value(value.clone()).map_err(|e| format!("SDK_IDENTITY_MISMATCH: {e}"))?;
    identity.validate()?;
    Ok(value.clone())
}
/// Snapshot/readback only. Config and every new dispatch still call identity().
pub(crate) fn receipt_identity(settings: &Value) -> Result<Value> {
    let value = settings.get("textSdkIdentity").ok_or("SDK_IDENTITY_MISSING")?;
    let identity: SdkIdentity = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    identity.validate_receipt()?;
    Ok(value.clone())
}
pub(crate) fn supervisor_handoff_receipt_identity_bound(old: &Value, new: &Value) -> bool {
    let (Ok(mut a), Ok(mut b)) = (serde_json::from_value::<SdkIdentity>(old.clone()),
        serde_json::from_value::<SdkIdentity>(new.clone())) else { return false; };
    if a.validate_receipt().is_err() || b.validate_receipt().is_err()
        || a.bridge_sha256 != b.bridge_sha256 { return false; }
    a.bridge_sha256 = compiled_native_bridge_sha();
    b.bridge_sha256 = compiled_native_bridge_sha();
    native::supervisor_handoff_identity_bound(&json!(a), &json!(b))
}
/// This DTO is safe even before Node has been installed. No runtime, account or model probe.
pub async fn status(settings: &Value) -> Value {
    let frozen = identity(settings).ok();
    let native = native_execution_enabled();
    json!({"backend":"codex_sdk","bridgeVersion":"0.1.0","protocolVersion":PROTOCOL,
        "sdk":{"package":"@openai/codex-sdk","version":SDK_VERSION,"implementationSha256":SDK_IMPLEMENTATION_SHA,"transport":"exec --experimental-json"},
        "available":native,"liveCallsAllowed":native,"sharedRustExecutor":true,
        "offlineContractImplemented":true,"nativeVerified":false,"identityFrozen":frozen.is_some(),
        "runtime":{"version":null,"verified":false},
        "capabilities":{"execute":if native {"explicit_selection_after_native_readiness"} else {"offline_fixture_only_in_tests"},"localReceiptObserve":true,"passiveRemoteObserve":false,"turnId":false,"cancel":if native {"AbortSignal_then_owned_windows_job"} else {"AbortSignal_best_effort"},"processTreeStopProven":false,"domainAcceptance":false,"rawRuntimeBytes":false,"maxActiveAttempts":1},
        "nativeExecutionRegistered":native,"explicitSelectionRequired":true,
        "blockers":if native {vec!["真实执行前必须通过官方账户/模型检查并冻结身份", "固定 SDK 不提供远端被动对账或 turn ID；未知提交不自动重发"]} else {vec!["SDK_LIVE_DISABLED：此入口没有标准 Windows 原生能力", "离线、headless 与未验证平台保持关闭"]},
        "limits":{"frameBytes":FRAME_LIMIT,"eventCount":EVENT_LIMIT,"journalBytes":JOURNAL_LIMIT,"stderrBytes":STDERR_LIMIT},
        "frozenIdentity":frozen})
}
pub async fn resolve_for_workflow(settings: &Value) -> Result<Value> {
    native::resolve(settings).await
}

/// A new, not-yet-created workflow node may bind today's supervisor while all
/// upstream SDK/runtime/account/model identities remain byte-for-byte frozen.
/// This never edits workflow settings or an existing attempt.
#[derive(Debug)]
pub(crate) struct SupervisorHandoff {
    pub identity: Value,
    pub resource_hashes: Value,
}
pub(crate) async fn prepare_supervisor_handoff(
    settings: &Value,
) -> Result<Option<SupervisorHandoff>> {
    native::prepare_supervisor_handoff(settings).await
}
pub(crate) fn supervisor_handoff_identity_bound(old: &Value, new: &Value) -> bool {
    native::supervisor_handoff_identity_bound(old, new)
}
pub(crate) fn verify_supervisor_handoff_bytes(old: &Value, new: &Value, hashes: &Value) -> Result<()> {
    native::verify_supervisor_handoff_bytes(old, new, hashes)
}

pub(crate) async fn add_native_readiness(settings: &Value, value: &mut Value) {
    native::add_readiness(settings, value).await;
}

fn compiled_native_bridge_sha() -> String {
    digest(&format!(
        "{}{}{}",
        include_str!("../../../../sdk-bridge/src/main.mjs"),
        include_str!("../../../../sdk-bridge/src/bridge.mjs"),
        include_str!("../../../../sdk-bridge/src/native-runtime.mjs")
    ))
}

/// Canonical paths remain the identity used for verification. Node's main-module
/// resolver cannot use Windows verbatim drive/UNC prefixes as script arguments.
/// Translate only those two filesystem spellings after the caller has verified
/// the path; never accept device namespaces or resolve relative input here.
pub(crate) fn node_path_argument(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("SDK_PATH_IDENTITY: Node 参数需要独立绝对路径".into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut parts = path.components();
        let mut regular = match parts.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
                Prefix::VerbatimUNC(server, share) => {
                    let mut value = std::ffi::OsString::from("\\\\");
                    value.push(server);
                    value.push("\\");
                    value.push(share);
                    value.push("\\");
                    PathBuf::from(value)
                }
                Prefix::Disk(_) | Prefix::UNC(_, _) => return Ok(path.to_path_buf()),
                _ => return Err("SDK_PATH_IDENTITY: Node 不接受设备命名空间".into()),
            },
            _ => return Err("SDK_PATH_IDENTITY: Node 路径缺少卷标".into()),
        };
        for part in parts {
            match part {
                Component::RootDir => (),
                Component::Normal(value) => regular.push(value),
                _ => return Err("SDK_PATH_IDENTITY: Node 路径不是规范绝对路径".into()),
            }
        }
        Ok(regular)
    }
    #[cfg(not(windows))]
    Ok(path.to_path_buf())
}

/// Node's script loader needs ordinary paths, while fs.mkdtemp in the pinned
/// SDK needs extended Windows data paths at MAX_PATH. Preserve the same physical
/// worker directory without canonicalizing through a caller-supplied symlink.
/// The unchanged bridge still owns its lock, attempt checks and frozen plan.
pub(crate) fn node_state_path_argument(path: &Path) -> Result<PathBuf> {
    let regular = node_path_argument(path)?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut parts = regular.components();
        let mut extended = match parts.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(drive) => PathBuf::from(format!(r"\\?\{}:\", char::from(drive))),
                Prefix::UNC(server, share) => {
                    let mut value = std::ffi::OsString::from(r"\\?\UNC\");
                    value.push(server);
                    value.push("\\");
                    value.push(share);
                    PathBuf::from(value)
                }
                _ => return Err("SDK_PATH_IDENTITY: worker 数据路径不是磁盘或UNC路径".into()),
            },
            _ => return Err("SDK_PATH_IDENTITY: worker 数据路径缺少卷标".into()),
        };
        for part in parts {
            if let Component::Normal(value) = part {
                extended.push(value);
            }
        }
        Ok(extended)
    }
    #[cfg(not(windows))]
    Ok(regular)
}

#[cfg(windows)]
pub(crate) fn node_system_root() -> Result<PathBuf> {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or("SDK_NODE_WINDOWS_ENV: Windows 系统目录缺失")?;
    let root = node_path_argument(&root)?;
    if !root.is_dir() {
        return Err("SDK_NODE_WINDOWS_ENV: Windows 系统目录无效".into());
    }
    Ok(root)
}

#[cfg(test)]
pub(crate) fn node_import_specifier(path: &Path) -> Result<String> {
    reqwest::Url::from_file_path(node_path_argument(path)?)
        .map(|url| url.into())
        .map_err(|_| "SDK_PATH_IDENTITY: Node 预加载模块需要 file URL".into())
}

#[derive(Clone)]
pub(crate) struct Config {
    node: PathBuf,
    bridge: PathBuf,
    state: PathBuf,
    fixture: Option<String>,
    deadline: Duration,
    identity: Value,
}
impl Config {
    pub(crate) fn from_frozen_identity(
        original_settings: &Value,
        frozen_identity: &Value,
        root: &Path,
    ) -> Result<Self> {
        // Only a local transport DTO is composed. The parent settings and saved
        // request are never refreshed or persisted through this method.
        let mut transport_settings = original_settings.clone();
        transport_settings["textSdkIdentity"] = frozen_identity.clone();
        Self::from_settings(&transport_settings, root)
    }
    pub(crate) fn from_settings(settings: &Value, root: &Path) -> Result<Self> {
        let identity = identity(settings)?;
        #[cfg(test)]
        if let Some(fixture) = settings.get("sdkOfflineFixture").and_then(Value::as_str) {
            let bridge =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sdk-bridge/src/main.mjs");
            let bridge = std::fs::canonicalize(bridge).map_err(|e| e.to_string())?;
            return Ok(Self {
                node: PathBuf::from(s(&identity, "nodeExecutable")),
                bridge,
                state: root.join("sdk-worker"),
                fixture: Some(fixture.into()),
                deadline: Duration::from_millis(
                    settings["sdkFixtureDeadlineMs"].as_u64().unwrap_or(10_000),
                ),
                identity,
            });
        }
        let _ = (root, identity);
        native::config(settings, root)
    }
    fn verify(&self) -> Result<()> {
        for p in [&self.node, &self.bridge, &self.state] {
            if !p.is_absolute()
                || p.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("SDK_PATH_IDENTITY: 需要独立绝对路径".into());
            }
        }
        let base = self
            .bridge
            .parent()
            .and_then(Path::parent)
            .ok_or("SDK_PATH_IDENTITY")?;
        let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| e.to_string());
        let bridge = if self.fixture.is_some() {
            digest(&format!(
                "{}{}{}{}",
                read(&base.join("src/main.mjs"))?,
                read(&base.join("src/bridge.mjs"))?,
                read(&base.join("test/fixture-runtime.mjs"))?,
                read(&base.join("test/fixtures/fake-codex.mjs"))?
            ))
        } else {
            digest(&format!(
                "{}{}{}",
                read(&base.join("src/main.mjs"))?,
                read(&base.join("src/bridge.mjs"))?,
                read(&base.join("src/native-runtime.mjs"))?
            ))
        };
        if bridge != s(&self.identity, "bridgeSha256")
            || crate::store::hash_file(&self.node)?.0 != s(&self.identity, "nodeSha256")
            || crate::store::hash_file(&base.join("node_modules/@openai/codex-sdk/dist/index.js"))?
                .0
                != SDK_IMPLEMENTATION_SHA
            || crate::store::hash_file(&base.join("node_modules/@openai/codex-sdk/package.json"))?.0
                != "5ffdee6d6135684e502d2d6010bae3690c0e2acb44e95d1e27c14fc5bbfccf30"
        {
            return Err("SDK_PACKAGE_IDENTITY: Node、桥或官方包文件与冻结身份不同".into());
        }
        if self.fixture.is_none() {
            native::verify_config(self)?;
        }
        Ok(())
    }
}

/// Bounded line decoder, including split UTF-8. A partial final line is not a receipt.
struct Frames<R> {
    reader: R,
    pending: Vec<u8>,
    bytes: usize,
    count: usize,
}
impl<R: AsyncRead + Unpin> Frames<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
            bytes: 0,
            count: 0,
        }
    }
    async fn next(&mut self) -> Result<Value> {
        loop {
            if let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                self.count += 1;
                if self.count > EVENT_LIMIT + 32 || end > FRAME_LIMIT {
                    return Err("SDK_EVENT_LIMIT".into());
                }
                std::str::from_utf8(&line).map_err(|_| "SDK_INVALID_UTF8".to_string())?;
                return serde_json::from_slice(&line).map_err(|_| "SDK_INVALID_NDJSON".into());
            }
            if self.pending.len() > FRAME_LIMIT {
                return Err("SDK_FRAME_LIMIT".into());
            }
            let mut bytes = [0u8; 8192];
            let n = self
                .reader
                .read(&mut bytes)
                .await
                .map_err(|e| format!("SDK_DISCONNECTED: {e}"))?;
            if n == 0 {
                return Err(if self.pending.is_empty() {
                    "SDK_DISCONNECTED"
                } else {
                    "SDK_TRUNCATED_FRAME"
                }
                .into());
            }
            self.bytes += n;
            if self.bytes > JOURNAL_LIMIT + 4 * FRAME_LIMIT {
                return Err("SDK_STREAM_LIMIT".into());
            }
            self.pending.extend_from_slice(&bytes[..n]);
        }
    }
}
#[derive(Default)]
struct StderrEvidence {
    bytes: usize,
    kept: Vec<u8>,
}
async fn drain_stderr<R: AsyncRead + Unpin>(
    mut reader: R,
    evidence: Arc<Mutex<StderrEvidence>>,
) -> Result<Vec<u8>> {
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).await.map_err(|e| e.to_string())?;
        let mut evidence = evidence.lock().map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(evidence.kept.clone());
        }
        evidence.bytes += n;
        let count = n.min(65536usize.saturating_sub(evidence.kept.len()));
        evidence.kept.extend_from_slice(&buf[..count]);
        if evidence.bytes > STDERR_LIMIT {
            return Err("SDK_STDERR_LIMIT".into());
        }
    }
}
struct Worker {
    child: tokio::process::Child,
    _registration: crate::text_executor::ChildRegistration,
    stdin: tokio::process::ChildStdin,
    frames: Frames<tokio::process::ChildStdout>,
    stderr: tokio::task::JoinHandle<Result<Vec<u8>>>,
    stderr_evidence: Arc<Mutex<StderrEvidence>>,
    next_id: u64,
    native_job: Option<native::ProcessJob>,
}
impl Worker {
    async fn spawn(config: &Config) -> Result<Self> {
        Self::spawn_entry(config,false).await
    }
    async fn spawn_entry(config: &Config, schema_recovery: bool) -> Result<Self> {
        config.verify()?;
        std::fs::create_dir_all(&config.state).map_err(|e| e.to_string())?;
        let entry = if schema_recovery {
            use std::io::Write;
            let metadata=std::fs::symlink_metadata(&config.state).map_err(|e|e.to_string())?;
            if !metadata.is_dir() || metadata.is_symlink() {return Err("SDK_SCHEMA_CONTROLLER_DIRECTORY".into());}
            let file=config.state.join(format!("schema-controller-{}.mjs",schema_controller_sha256()));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&file) {
                Ok(mut handle)=>{handle.write_all(SCHEMA_CONTROLLER.as_bytes()).map_err(|e|e.to_string())?;handle.sync_all().map_err(|e|e.to_string())?;},
                Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e.to_string()),
            }
            let m=std::fs::symlink_metadata(&file).map_err(|e|e.to_string())?;
            if !m.is_file() || m.is_symlink() || crate::store::hash_file(&file)?.0!=schema_controller_sha256() {return Err("SDK_SCHEMA_CONTROLLER_CHANGED".into());}
            file
        } else {config.bridge.clone()};
        // No inherited NODE_OPTIONS, HOME, credential, proxy, PATH or config variables.
        let mut command = tokio::process::Command::new(&config.node);
        command
            .arg("--max-old-space-size=192")
            .arg(node_path_argument(&entry)?)
            .arg("--state-dir")
            .arg(node_state_path_argument(&config.state)?)
            .env_clear()
            .current_dir(&config.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(fixture) = &config.fixture {
            command.arg("--offline-fixture").arg(fixture);
        } else {
            let file = config.state.join("native-config.json");
            crate::store::atomic_json(
                &file,
                &json!({"version":"sdk-native-config/v1","identity":config.identity}),
            )?;
            command
                .arg("--native-config")
                .arg(node_path_argument(&file)?);
        }
        #[cfg(windows)]
        {
            // Windows Node/OpenSSL requires this system-directory entry even
            // with no user environment. It grants no account or model access.
            let system_root = node_system_root()?;
            command
                .env("SystemRoot", &system_root)
                .env("WINDIR", system_root);
            command.creation_flags(if config.fixture.is_none() {
                0x08000004
            } else {
                0x08000000
            });
        }
        let (mut child, registration) = crate::text_executor::spawn_cli_child(&mut command)?;
        let native_job = if config.fixture.is_none() {
            // No execute request has been sent. Verified bridge import/hello do
            // not construct a runtime; containment is established before dispatch.
            match native::ProcessJob::attach_tokio(&child) {
                Ok(job) => {
                    if let Err(error) = job.start() {
                        let _ = child.wait().await;
                        return Err(error);
                    }
                    Some(job)
                }
                Err(error) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(error);
                }
            }
        } else {
            None
        };
        let stdin = child.stdin.take().ok_or("SDK_STDIN")?;
        let stdout = child.stdout.take().ok_or("SDK_STDOUT")?;
        let stderr = child.stderr.take().ok_or("SDK_STDERR")?;
        let stderr_evidence = Arc::new(Mutex::new(StderrEvidence::default()));
        let stderr = tokio::spawn(drain_stderr(stderr, stderr_evidence.clone()));
        Ok(Self {
            child,
            _registration: registration,
            stdin,
            frames: Frames::new(stdout),
            stderr,
            stderr_evidence,
            next_id: 0,
            native_job,
        })
    }
    async fn send(&mut self, method: &str, attempt: Option<&str>, params: Value) -> Result<String> {
        self.next_id += 1;
        let id = format!("rust_{}", self.next_id);
        let mut bytes=serde_json::to_vec(&json!({"protocolVersion":PROTOCOL,"requestId":id,"attemptId":attempt,"method":method,"params":params})).map_err(|e|e.to_string())?;
        if bytes.len() > FRAME_LIMIT {
            return Err("SDK_FRAME_LIMIT".into());
        }
        bytes.push(b'\n');
        tokio::time::timeout(CONTROL_TIMEOUT, self.stdin.write_all(&bytes))
            .await
            .map_err(|_| "SDK_INPUT_TIMEOUT")?
            .map_err(|e| format!("SDK_INPUT_CLOSED: {e}"))?;
        Ok(id)
    }
    async fn next(&mut self, deadline: tokio::time::Instant) -> Result<Value> {
        let value = tokio::select! {
            value=tokio::time::timeout_at(deadline,self.frames.next())=>value.map_err(|_|"SDK_DEADLINE")??,
            result=&mut self.stderr=>{return Err(match result {Ok(Err(e))=>e,Ok(Ok(_))=>"SDK_STDERR_CLOSED".into(),Err(e)=>e.to_string()});}
        };
        if value["protocolVersion"] != PROTOCOL {
            return Err("SDK_PROTOCOL_IDENTITY".into());
        }
        Ok(value)
    }
    async fn rpc(&mut self, method: &str, attempt: Option<&str>, params: Value) -> Result<Value> {
        let id = self.send(method, attempt, params).await?;
        let value = self
            .next(tokio::time::Instant::now() + CONTROL_TIMEOUT)
            .await?;
        if value["type"] != "response"
            || value["requestId"] != id
            || value["attemptId"].as_str() != attempt
        {
            return Err("SDK_RESPONSE_IDENTITY".into());
        }
        if value["ok"] != true {
            return Err(format!(
                "{}: {}",
                s(&value["error"], "code"),
                s(&value["error"], "message")
            ));
        }
        Ok(value["result"].clone())
    }
    async fn close(&mut self) -> Result<Option<Value>> {
        let _ = self.send("shutdown", None, Value::Null).await;
        if tokio::time::timeout(Duration::from_secs(2), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await;
        }
        self.stderr.abort();
        self.native_job.as_ref().map(|job| job.stop()).transpose()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stderr.abort();
        if let Some(job) = &self.native_job {
            let _ = job.stop();
        }
        let _ = self.child.start_kill();
    }
}

pub(crate) struct OfficialSdkTransport {
    config: Config,
    worker: tokio::sync::Mutex<Option<Worker>>,
    observer: Observer,
    active_attempt: Mutex<Option<String>>,
    cancel_requested: AtomicBool,
    cancel_notify: tokio::sync::Notify,
}
impl OfficialSdkTransport {
    pub(crate) fn new(config: Config, observer: Observer) -> Self {
        Self {
            config,
            worker: tokio::sync::Mutex::new(None),
            observer,
            active_attempt: Mutex::new(None),
            cancel_requested: AtomicBool::new(false),
            cancel_notify: tokio::sync::Notify::new(),
        }
    }
    pub(crate) fn set_observer(&mut self, observer: Observer) {
        self.observer = observer;
    }
    async fn connect(&self, worker: &mut Option<Worker>) -> Result<()> {
        if worker.is_none() {
            *worker = Some(Worker::spawn(&self.config).await?);
        }
        Ok(())
    }
    pub(crate) async fn close(&self) -> Result<()> {
        if let Some(mut worker) = self.worker.lock().await.take() {
            if let Some(proof) = worker.close().await? {
                (self.observer)("sdk_process_tree", &proof)?;
            }
            let evidence = worker.stderr_evidence.lock().map_err(|e| e.to_string())?;
            let hex = evidence
                .kept
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            (self.observer)(
                "sdk_stderr",
                &json!({"format":"sdk-sidecar-stderr/v1","bytes":evidence.bytes,"retainedBytes":evidence.kept.len(),"retainedHex":hex,"truncated":evidence.bytes>evidence.kept.len(),"scope":"Node sidecar stderr only; not SDK-internal child stderr"}),
            )?;
        }
        Ok(())
    }
    fn unknown(&self, attempt: &FrozenAttempt) -> Observation {
        Observation {
            attempt_id: attempt.attempt_id.clone(),
            sequence: attempt.last_sequence + 1,
            kind: ObservationKind::SubmissionUnknown,
            thread_id: attempt.thread_id.clone(),
            turn_id: None,
            raw_result: None,
        }
    }
    fn receipt(
        &self,
        attempt: &FrozenAttempt,
        receipt: &Value,
        last_sdk: u64,
    ) -> Result<Observation> {
        let compiled = crate::text_executor::compile_sdk_attempt_input(attempt)?;
        if receipt["semanticRequestHash"] != attempt.semantic_request_hash
            || receipt["executionFingerprint"] != attempt.execution_fingerprint
            || receipt["attemptEnvelopeHash"] != attempt.attempt_envelope_hash
            || receipt["compiledPromptSha256"] != digest(&compiled.prompt)
            || receipt["compiledSchemaSha256"] != digest(&compiled.schema)
        {
            return Err("SDK_RECEIPT_REQUEST_BINDING".into());
        }
        if receipt["attemptId"] != attempt.attempt_id
            || receipt["backend"] != "codex_sdk"
            || !receipt["turnId"].is_null()
            || receipt["domainAccepted"] != false
            || receipt["processTreeStopProven"] != false
            || receipt["lastSequence"].as_u64() != Some(last_sdk)
        {
            return Err("SDK_RECEIPT_IDENTITY".into());
        }
        // The SDK journal cursor is preserved separately; projection sequence is
        // even for SDK events, odd for terminal observations. Never reuse the last event.
        let sequence = (last_sdk
            .checked_mul(2)
            .and_then(|v| v.checked_add(1))
            .ok_or("SDK_SEQUENCE_LIMIT")?)
        .max(attempt.last_sequence + 1);
        Ok(Observation {
            attempt_id: attempt.attempt_id.clone(),
            sequence,
            kind: if receipt["state"] == "transport_completed" {
                ObservationKind::Completed
            } else {
                ObservationKind::SubmissionUnknown
            },
            thread_id: receipt["threadId"].as_str().map(str::to_owned),
            turn_id: None,
            raw_result: receipt.get("rawResult").filter(|v| !v.is_null()).cloned(),
        })
    }
}
impl OfficialSdkTransport {
    pub(crate) async fn recover_local_preparation(&self, attempt: &FrozenAttempt, proof: &Value) -> Result<Observation> {
        self.execute_method(attempt, Some(proof)).await
    }
    pub(crate) async fn recover_schema_preparation(&self, attempt: &FrozenAttempt, proof: &Value, evidence: &Value) -> Result<Observation> {
        self.execute_method(attempt,Some(&json!({"proof":proof,"evidence":evidence}))).await
    }
    async fn execute_method(&self, attempt: &FrozenAttempt, recovery: Option<&Value>) -> Result<Observation> {
        attempt.validate_identity()?;
        let schema_recovery=recovery.is_some_and(|r|r["proof"]["version"]==schema_recovery::SCHEMA_RECOVERY_VERSION);
        if attempt.backend != Backend::CodexSdk || if schema_recovery { !schema_recovery_identity_bound(&attempt.runtime_identity,&self.config.identity) }
            else if recovery.is_some() { !local_recovery_identity_bound(&attempt.runtime_identity, &self.config.identity) } else { attempt.runtime_identity != self.config.identity }
        {
            return Err("SDK_ATTEMPT_IDENTITY".into());
        }
        let compiled = crate::text_executor::compile_sdk_attempt_input(attempt)?;
        {
            let mut active = self.active_attempt.lock().map_err(|e| e.to_string())?;
            if active.is_some() {
                return Err("SDK_WORKER_OCCUPIED: 原执行仍占用 worker".into());
            }
            *active = Some(attempt.attempt_id.clone());
        }
        let mut worker = self.worker.lock().await;
        if schema_recovery {
            if worker.is_some(){return Err("SDK_SCHEMA_CONTROLLER_ALREADY_CONNECTED".into());}
            *worker=Some(Worker::spawn_entry(&self.config,true).await?);
        } else {self.connect(&mut worker).await?;}
        let worker = worker.as_mut().unwrap();
        let (method, params) = if schema_recovery {
            ("recover-schema-preparation",recovery.unwrap().clone())
        } else if let Some(proof) = recovery {
            (if proof["version"] == "sdk-wrapper-preparation-repair/v1" {"repair-wrapper-preparation"} else {"recover-local-preparation"}, json!({"proof":proof}))
        } else {
            ("execute", json!({"attempt":attempt,"compiledPrompt":compiled.prompt,"compiledSchema":compiled.schema,
                "compiledPromptSha256":digest(&compiled.prompt),"compiledSchemaSha256":digest(&compiled.schema)}))
        };
        let id=worker.send(method,Some(&attempt.attempt_id),params).await?;
        let deadline = tokio::time::Instant::now() + self.config.deadline;
        let mut last_sdk = 0;
        let mut ack = false;
        let result: Result<Observation> = async {
            loop {
                if self.cancel_requested.load(Ordering::SeqCst) {
                    return Err("SDK_CANCEL_REQUESTED".into());
                }
                let event = tokio::select! {
                    value=worker.next(deadline)=>value?,
                    _=self.cancel_notify.notified()=>return Err("SDK_CANCEL_REQUESTED".into()),
                };
                if event["requestId"] != id || event["attemptId"] != attempt.attempt_id {
                    return Err("SDK_EVENT_IDENTITY".into());
                }
                match s(&event, "type") {
                    "response" => {
                        if ack {
                            return Err("SDK_DUPLICATE_ACK".into());
                        }
                        if event["ok"] != true {
                            return Err(format!(
                                "{}: {}",
                                s(&event["error"], "code"),
                                s(&event["error"], "message")
                            ));
                        }
                        ack = true;
                        (self.observer)("sdk_dispatch_receipt", &event)?;
                    }
                    "sdk_event" => {
                        let sequence = event["sequence"].as_u64().ok_or("SDK_SEQUENCE")?;
                        if sequence != last_sdk + 1
                            || sequence > EVENT_LIMIT as u64
                            || !event["turnId"].is_null()
                        {
                            return Err("SDK_SEQUENCE".into());
                        }
                        last_sdk = sequence;
                        (self.observer)("sdk_event", &event)?;
                    }
                    "observation" => {
                        if !ack {
                            return Err("SDK_TERMINAL_BEFORE_ACK".into());
                        }
                        (self.observer)("sdk_observation", &event)?;
                        return self.receipt(attempt, &event, last_sdk);
                    }
                    _ => return Err("SDK_UNKNOWN_FRAME".into()),
                }
            }
        }
        .await;
        match result {
            Ok(observation) => Ok(observation),
            Err(error) => {
                // Best-effort cancellation does not clear task ownership or budget.
                let _ = worker
                    .send("cancel", Some(&attempt.attempt_id), Value::Null)
                    .await;
                (self.observer)(
                    "sdk_transport_unknown",
                    &json!({"attemptId":attempt.attempt_id,"lastSdkSequence":last_sdk,"error":error,"processTreeStopProven":false}),
                )?;
                Ok(self.unknown(attempt))
            }
        }
    }
}
#[async_trait::async_trait]
impl TextTransport for OfficialSdkTransport {
    async fn inspect(&self) -> Result<Value> {
        let mut worker = self.worker.lock().await;
        self.connect(&mut worker).await?;
        let value = worker
            .as_mut()
            .unwrap()
            .rpc("inspect", None, Value::Null)
            .await?;
        if value["sdk"]["package"] != "@openai/codex-sdk"
            || value["sdk"]["version"] != SDK_VERSION
            || value["protocolVersion"] != PROTOCOL
            || value["bridgeVersion"] != "0.1.0"
            || value["liveCallsAllowed"] != self.config.fixture.is_none()
            || value["capabilities"]["processTreeStopProven"] != false
        {
            return Err("SDK_INSPECT_IDENTITY".into());
        }
        Ok(value)
    }
    async fn execute(&self, attempt: &FrozenAttempt) -> Result<Observation> {
        self.execute_method(attempt, None).await
    }
    async fn reconcile(&self, attempt: &FrozenAttempt) -> Result<Observation> {
        attempt.validate_identity()?;
        let mut worker = self.worker.lock().await;
        if self.connect(&mut worker).await.is_err() {
            return Ok(self.unknown(attempt));
        }
        match worker
            .as_mut()
            .unwrap()
            .rpc("observe", Some(&attempt.attempt_id), Value::Null)
            .await
        {
            Ok(receipt) => {
                let sequence = receipt["lastSequence"].as_u64().ok_or("SDK_SEQUENCE")?;
                let observation = self.receipt(attempt, &receipt, sequence)?;
                let journal = self
                    .config
                    .state
                    .join(&attempt.attempt_id)
                    .join("events.ndjson");
                if journal.exists() {
                    if std::fs::metadata(&journal)
                        .map_err(|e| e.to_string())?
                        .len()
                        > JOURNAL_LIMIT as u64
                    {
                        return Err("SDK_JOURNAL_LIMIT".into());
                    }
                    let bytes = std::fs::read(&journal).map_err(|e| e.to_string())?;
                    let text = std::str::from_utf8(&bytes).map_err(|_| "SDK_INVALID_UTF8")?;
                    if !text.ends_with('\n') {
                        return Err("SDK_TRUNCATED_JOURNAL".into());
                    }
                    let mut count = 0;
                    for line in text.lines() {
                        count += 1;
                        if count > EVENT_LIMIT as u64 || line.len() > FRAME_LIMIT {
                            return Err("SDK_JOURNAL_LIMIT".into());
                        }
                        let event: Value =
                            serde_json::from_str(line).map_err(|_| "SDK_INVALID_NDJSON")?;
                        if event["attemptId"] != attempt.attempt_id
                            || event["sequence"].as_u64() != Some(count)
                            || event["protocolVersion"] != PROTOCOL
                        {
                            return Err("SDK_JOURNAL_IDENTITY".into());
                        }
                        (self.observer)("sdk_event", &event)?;
                    }
                    if count != sequence {
                        return Err("SDK_JOURNAL_SEQUENCE".into());
                    }
                } else if sequence != 0 {
                    return Err("SDK_JOURNAL_MISSING".into());
                }
                (self.observer)("sdk_reconciliation", &receipt)?;
                Ok(observation)
            }
            Err(error) => {
                (self.observer)(
                    "sdk_reconciliation_unknown",
                    &json!({"error":error,"newExecutions":0}),
                )?;
                Ok(self.unknown(attempt))
            }
        }
    }
    async fn cancel(&self, attempt: &FrozenAttempt) -> Result<Observation> {
        attempt.validate_identity()?;
        if self
            .active_attempt
            .lock()
            .map_err(|e| e.to_string())?
            .as_ref()
            .is_some_and(|id| id != &attempt.attempt_id)
        {
            return Err("SDK_CANCEL_IDENTITY".into());
        }
        // Never wait for execute's pipe ownership mutex: signal its select loop.
        self.cancel_requested.store(true, Ordering::SeqCst);
        self.cancel_notify.notify_one();
        (self.observer)(
            "sdk_cancel_requested",
            &json!({"attemptId":attempt.attempt_id,"processTreeStopProven":false}),
        )?;
        Ok(self.unknown(attempt))
    }
}

#[cfg(test)]
pub(crate) fn offline_settings(scenario: &str) -> Value {
    let node=std::env::var_os("DIRECTOR_TEST_NODE").map(PathBuf::from).or_else(||{
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|p|p.join(if cfg!(windows){"node.exe"}else{"node"})).find(|p|p.is_file())
    }).expect("Offline SDK integration requires an explicitly installed Node; run npm ci in sdk-bridge first");
    let node = std::fs::canonicalize(node).unwrap();
    let identity = SdkIdentity {
        version: "official-sdk-identity/v1".into(),
        provider: "codex_sdk".into(),
        sdk_package: "@openai/codex-sdk".into(),
        sdk_version: SDK_VERSION.into(),
        sdk_implementation_sha256: SDK_IMPLEMENTATION_SHA.into(),
        bridge_version: "0.1.0".into(),
        bridge_sha256: compiled_bridge_sha(),
        protocol_version: PROTOCOL.into(),
        node_executable: node.to_string_lossy().into(),
        node_sha256: crate::store::hash_file(&node).unwrap().0,
        node_version: String::from_utf8(
            std::process::Command::new(&node)
                .arg("--version")
                .env_clear()
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .into(),
        runtime_package_version: SDK_VERSION.into(),
        runtime_identity: "bundled-synthetic-fixture".into(),
        permission_profile: POLICY.into(),
        account_fingerprint: "synthetic-no-account".into(),
        runtime_executable: None,
        runtime_sha256: None,
        supervisor_executable: None,
        supervisor_sha256: None,
        resource_directory: None,
        credential_home: None,
        codex_home: None,
    };
    json!({"textProvider":"codex_sdk","textModel":"gpt-6.1-sol","reasoning":"medium","textSdkIdentity":identity,"sdkOfflineFixture":scenario})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn sdk_node_paths_keep_drive_unc_and_reject_devices_or_relative_paths() {
        for (verbatim, regular) in [
            (
                r"\\?\D:\SDK checks\三视图\health.mjs",
                r"D:\SDK checks\三视图\health.mjs",
            ),
            (
                r"\\?\UNC\server\share\SDK checks\三视图.mjs",
                r"\\server\share\SDK checks\三视图.mjs",
            ),
        ] {
            assert_eq!(
                node_path_argument(Path::new(verbatim)).unwrap(),
                Path::new(regular)
            );
            assert_eq!(
                node_path_argument(Path::new(regular)).unwrap(),
                Path::new(regular)
            );
            assert_eq!(node_state_path_argument(Path::new(regular)).unwrap(), Path::new(verbatim));
            assert_eq!(node_state_path_argument(Path::new(verbatim)).unwrap(), Path::new(verbatim));
            assert_eq!(node_state_path_argument(Path::new(&regular.replace('\\', "/"))).unwrap(), Path::new(verbatim));
        }
        for invalid in [
            r"relative.mjs",
            r"D:\..\health.mjs",
            r"\\.\pipe\sdk",
            r"\\?\GLOBALROOT\Device\sdk",
        ] {
            assert!(node_path_argument(Path::new(invalid)).is_err(), "{invalid}");
            assert!(node_state_path_argument(Path::new(invalid)).is_err(), "{invalid}");
        }
    }

    #[test]
    fn sdk_node_entry_and_import_support_canonical_non_ascii_paths() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("SDK 预加载 # %");
        std::fs::create_dir(&directory).unwrap();
        let preload = directory.join("preload.mjs");
        let main = directory.join("main.mjs");
        std::fs::write(
            &preload,
            "globalThis.__directorSyntheticPreload = 'fixture-loaded';",
        )
        .unwrap();
        std::fs::write(
            &main,
            "process.stdout.write(JSON.stringify({preload:globalThis.__directorSyntheticPreload || 'missing',environment:Object.keys(process.env)}));",
        )
        .unwrap();
        let preload = std::fs::canonicalize(preload).unwrap();
        let main = std::fs::canonicalize(main).unwrap();
        let specifier = node_import_specifier(&preload).unwrap();
        assert!(specifier.starts_with("file:"));
        assert!(
            specifier.contains("%20") && specifier.contains("%23") && specifier.contains("%25")
        );
        assert_eq!(
            reqwest::Url::parse(&specifier)
                .unwrap()
                .to_file_path()
                .unwrap(),
            node_path_argument(&preload).unwrap()
        );
        let settings = offline_settings("happy");
        let mut command =
            std::process::Command::new(s(&settings["textSdkIdentity"], "nodeExecutable"));
        command
            .arg("--import")
            .arg(specifier)
            .arg(node_path_argument(&main).unwrap())
            .env_clear()
            .current_dir(&directory)
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let system_root = node_system_root().unwrap();
            command
                .env("SystemRoot", &system_root)
                .env("WINDIR", system_root);
            command.creation_flags(0x08000000);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let evidence: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(evidence["preload"], "fixture-loaded");
        for key in evidence["environment"].as_array().unwrap() {
            assert!(
                (cfg!(windows) && ["SystemRoot", "WINDIR"].contains(&key.as_str().unwrap()))
                    || (cfg!(target_os = "macos")
                        && key.as_str() == Some("__CF_USER_TEXT_ENCODING")),
                "unexpected inherited Node environment key: {key}",
            );
        }
    }

    #[tokio::test]
    async fn sdk_frames_reassemble_split_utf8_and_reject_partial_invalid_and_large_frames() {
        let (mut tx, rx) = tokio::io::duplex(32);
        let writer = tokio::spawn(async move {
            for byte in "{\"text\":\"合成\"}\n".as_bytes() {
                tx.write_all(&[*byte]).await.unwrap();
            }
        });
        assert_eq!(
            Frames::new(rx).next().await.unwrap(),
            json!({"text":"合成"})
        );
        writer.await.unwrap();
        for (data, expected) in [
            (b"{\"x\":1}".to_vec(), "SDK_TRUNCATED_FRAME"),
            (vec![0xff, b'\n'], "SDK_INVALID_UTF8"),
            (vec![b'x'; FRAME_LIMIT + 1], "SDK_FRAME_LIMIT"),
        ] {
            let mut frames = Frames::new(data.as_slice());
            assert_eq!(frames.next().await.unwrap_err(), expected);
        }
    }
    #[tokio::test]
    async fn sdk_stderr_flood_retains_bounded_exact_prefix_and_fails_closed() {
        let bytes = vec![0xfe; STDERR_LIMIT + 1];
        let evidence = Arc::new(Mutex::new(StderrEvidence::default()));
        assert_eq!(
            drain_stderr(bytes.as_slice(), evidence.clone())
                .await
                .unwrap_err(),
            "SDK_STDERR_LIMIT"
        );
        let evidence = evidence.lock().unwrap();
        assert_eq!(evidence.kept.len(), 65536);
        assert!(evidence.kept.iter().all(|b| *b == 0xfe));
        assert_eq!(evidence.bytes, STDERR_LIMIT + 1);
    }
    #[tokio::test]
    async fn sdk_cancel_interrupts_active_pipe_owner_and_never_claims_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let settings = offline_settings("hang");
        let config = Config::from_settings(&settings, dir.path()).unwrap();
        let transport = OfficialSdkTransport::new(config, Arc::new(|_, _| Ok(())));
        let request = crate::pipeline::TextRequest {
            stage: "fixture".into(),
            skill: "fixture".into(),
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            input: json!({"synthetic":true}),
            schema: json!({"type":"object"}),
            rule_snapshot: json!({"synthetic":true}),
        };
        let attempt = FrozenAttempt::prepare(
            "cancel_original",
            "synthetic_workflow",
            "synthetic_node",
            Backend::CodexSdk,
            request.clone(),
            &settings["textSdkIdentity"],
        )
        .unwrap();
        let foreign = FrozenAttempt::prepare(
            "cancel_foreign",
            "synthetic_workflow",
            "synthetic_node",
            Backend::CodexSdk,
            request,
            &settings["textSdkIdentity"],
        )
        .unwrap();
        let cancel = async {
            let capture = dir
                .path()
                .join("sdk-worker/cancel_original/fixture-invocation.json");
            tokio::time::timeout(Duration::from_secs(4), async {
                while !capture.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert!(transport
                .cancel(&foreign)
                .await
                .unwrap_err()
                .contains("SDK_CANCEL_IDENTITY"));
            let outcome =
                tokio::time::timeout(Duration::from_millis(200), transport.cancel(&attempt))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(outcome.kind, ObservationKind::SubmissionUnknown);
        };
        let (result, ()) = tokio::join!(transport.execute(&attempt), cancel);
        assert_eq!(result.unwrap().kind, ObservationKind::SubmissionUnknown);
        transport.close().await.unwrap();
        let receipt: Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("sdk-worker/cancel_original/receipt.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["state"], "submission_unknown");
    }
    #[test]
    fn sdk_identity_rejects_cli_shape_unknown_fields_and_package_or_policy_drift() {
        let settings = offline_settings("happy");
        identity(&settings).unwrap();
        for field in [
            "provider",
            "sdkVersion",
            "sdkImplementationSha256",
            "bridgeSha256",
            "protocolVersion",
            "permissionProfile",
        ] {
            let mut changed = settings.clone();
            changed["textSdkIdentity"][field] = json!("changed");
            assert!(identity(&changed).is_err());
        }
        let mut extra = settings.clone();
        extra["textSdkIdentity"]["executable"] = json!("CLI");
        assert!(identity(&extra).is_err());
        assert!(identity(&json!({"textSdkIdentity":{"provider":"codex","executable":"/some/cli","version":"codex-cli 0.160.1"}})).is_err());
    }
    #[tokio::test]
    async fn sdk_input_format_legacy_and_new_success_replay_exact_bytes_without_another_execute() {
        sdk_input_format_restart_contract("happy", ObservationKind::Completed).await;
    }
    #[tokio::test]
    async fn sdk_input_format_legacy_and_new_unknown_remain_unknown_with_zero_restart_dispatch() {
        sdk_input_format_restart_contract("configuration-notice-only", ObservationKind::SubmissionUnknown).await;
    }
    async fn sdk_input_format_restart_contract(scenario: &str, expected: ObservationKind) {
        for rules_first in [false, true] {
            let temp = tempfile::tempdir().unwrap(); let settings = offline_settings(scenario);
            let config = Config::from_settings(&settings, temp.path()).unwrap();
            let transport = OfficialSdkTransport::new(config, Arc::new(|_, _| Ok(())));
            let request = crate::pipeline::TextRequest {
                stage: "fixture".into(), skill: "fixture".into(), model: "gpt-6.1-sol".into(), reasoning: "medium".into(),
                input: json!({"source":"合成原文完整保留\n😀"}), schema: json!({"type":"object"}),
                rule_snapshot: json!({"completeRule":"完整规则，不截断"}),
            };
            let mut attempt = if rules_first {
                FrozenAttempt::prepare_sdk_rules_first("format_fixture", "workflow_fixture", "node_fixture", request, &settings["textSdkIdentity"])
            } else {
                FrozenAttempt::prepare("format_fixture", "workflow_fixture", "node_fixture", Backend::CodexSdk, request, &settings["textSdkIdentity"])
            }.unwrap();
            let compiled = crate::text_executor::compile_sdk_attempt_input(&attempt).unwrap();
            let result = transport.execute(&attempt).await.unwrap();
            assert_eq!(result.kind, expected);
            transport.close().await.unwrap();
            attempt.begin_dispatch(Backend::CodexSdk).unwrap(); attempt.observe(result.clone()).unwrap();
            let frozen = temp.path().join("sdk-attempt.json"); attempt.persist(&frozen).unwrap();
            let directory = temp.path().join("sdk-worker/format_fixture");
            let invocation = std::fs::read(directory.join("fixture-invocation.json")).unwrap();
            let captured: Value = serde_json::from_slice(&invocation).unwrap();
            assert_eq!(captured["prompt"], compiled.prompt);
            assert!(compiled.prompt.starts_with(if rules_first {"{\"rules\":"} else {"{\"input\":"}));
            let events = std::fs::read(directory.join("events.ndjson")).unwrap();
            let frozen_bytes = std::fs::read(&frozen).unwrap();
            let config = Config::from_settings(&settings, temp.path()).unwrap();
            let resumed = OfficialSdkTransport::new(config, Arc::new(|_, _| Ok(())));
            let reopened = FrozenAttempt::load(&frozen).unwrap();
            let recovered = resumed.reconcile(&reopened).await.unwrap();
            resumed.close().await.unwrap();
            assert_eq!(recovered.kind, expected);
            assert_eq!(recovered.raw_result, result.raw_result);
            assert_eq!(std::fs::read(directory.join("fixture-invocation.json")).unwrap(), invocation);
            assert_eq!(std::fs::read(directory.join("events.ndjson")).unwrap(), events);
            assert_eq!(std::fs::read(&frozen).unwrap(), frozen_bytes);
            assert_eq!(std::fs::read_dir(temp.path().join("sdk-worker")).unwrap().flatten().filter(|item| item.path().join("fixture-invocation.json").is_file()).count(), 1);
        }
    }

}
