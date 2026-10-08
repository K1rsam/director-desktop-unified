//! Read-only CLI discovery, account/catalog inspection and browser login.
//! No function here starts a thread or a model turn.
use crate::store::{s, Result};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};

const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
const CACHE_AGE: Duration = Duration::from_secs(45);
static CACHE: OnceLock<Mutex<Option<(PathBuf, Instant, Value)>>> = OnceLock::new();
static LOGIN: OnceLock<Mutex<Option<Arc<LoginSession>>>> = OnceLock::new();
static LOGIN_START: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static CONTROL_RUNTIME: OnceLock<Result<tokio::runtime::Runtime>> = OnceLock::new();

/// Settings RPCs and login callbacks share a persistent runtime. A temporary
/// block_on runtime would cancel the pending login as soon as dispatch returns.
pub fn control_runtime() -> Result<&'static tokio::runtime::Runtime> {
    CONTROL_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        path.extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("exe"))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|v| v.permissions().mode() & 0o111 != 0)
    }
}

fn package_native_paths(base: &Path) -> Vec<PathBuf> {
    #[cfg(windows)]
    let mut platforms = [
        ("win32-x64", "x86_64-pc-windows-msvc", "codex.exe"),
        ("win32-arm64", "aarch64-pc-windows-msvc", "codex.exe"),
    ];
    #[cfg(target_os = "macos")]
    let mut platforms = [
        ("darwin-arm64", "aarch64-apple-darwin", "codex"),
        ("darwin-x64", "x86_64-apple-darwin", "codex"),
    ];
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut platforms = [
        ("linux-x64", "x86_64-unknown-linux-musl", "codex"),
        ("linux-arm64", "aarch64-unknown-linux-musl", "codex"),
    ];
    if cfg!(target_arch = "aarch64") && !cfg!(target_os = "macos")
        || cfg!(target_arch = "x86_64") && cfg!(target_os = "macos")
    {
        platforms.reverse();
    }
    let mut paths = vec![];
    for (platform, triple, name) in platforms {
        for suffix in ["bin", "codex"] {
            paths.push(base.join(format!("node_modules/@openai/codex/node_modules/@openai/codex-{platform}/vendor/{triple}/{suffix}/{name}")));
            paths.push(base.join(format!(
                "node_modules/@openai/codex/vendor/{triple}/{suffix}/{name}"
            )));
        }
    }
    paths
}

fn preferred_executable(path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).ok();
    // npm's Unix executable is usually a symlink to bin/codex.js. Use the
    // platform binary inside that package so GUI startup does not need Node PATH.
    if let Some(target) = canonical
        .as_ref()
        .filter(|p| p.file_name().is_some_and(|name| name == "codex.js"))
    {
        if let Some(base) = target.ancestors().nth(5) {
            if let Some(native) = package_native_paths(base)
                .into_iter()
                .find(|p| is_executable(p))
            {
                return Some(native);
            }
        }
    }
    if is_executable(path) {
        return Some(path.to_owned());
    }
    if path.is_file()
        && path.file_name().is_some_and(|name| {
            ["codex.cmd", "codex.bat", "codex.ps1"]
                .iter()
                .any(|expected| name.eq_ignore_ascii_case(expected))
        })
    {
        if let Some(base) = path.parent() {
            return package_native_paths(base)
                .into_iter()
                .find(|p| is_executable(p));
        }
    }
    None
}

/// Only the application's native, versioned Windows installation is relocatable.
/// An arbitrary explicit path (including npm wrappers) remains authoritative.
pub(crate) fn managed_codex_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData/Local"))
            })
            .map(|p| p.join("OpenAI/Codex/bin"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn path_equal(a: &Path, b: &Path) -> bool {
    let normalized = |p: &Path| p.components().collect::<PathBuf>();
    #[cfg(windows)]
    {
        normalized(a)
            .to_string_lossy()
            .eq_ignore_ascii_case(&normalized(b).to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        normalized(a) == normalized(b)
    }
}

pub(crate) fn managed_codex_path_at(path: &Path, root: &Path) -> bool {
    let path = path.components().collect::<PathBuf>();
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || !path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
    {
        return false;
    }
    let Some(version) = path.parent() else {
        return false;
    };
    let Some(name) = version.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.len() == 16
        && name.bytes().all(|byte| byte.is_ascii_hexdigit())
        && version
            .parent()
            .is_some_and(|parent| path_equal(parent, root))
}

fn no_reparse_ancestors(path: &Path) -> bool {
    path.ancestors().all(|part| {
        let Ok(metadata) = std::fs::symlink_metadata(part) else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return false;
            }
        }
        true
    })
}

/// Filesystem identity for a native executable, never a wrapper or a reparse
/// route. Separator spelling is transport data, not executable identity.
pub(crate) fn verified_native_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || !is_executable(path)
        || !no_reparse_ancestors(path)
    {
        return Err("Codex CLI 原生文件身份无法核实；原任务保留，未执行请求".into());
    }
    let native = std::fs::canonicalize(path)
        .map_err(|_| "Codex CLI 原生文件身份无法核实；原任务保留，未执行请求".to_string())?;
    #[cfg(windows)]
    let native = {
        let value = native.to_string_lossy();
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{rest}"))
        } else if let Some(rest) = value.strip_prefix(r"\\?\") {
            PathBuf::from(rest)
        } else {
            native
        }
    };
    Ok(native)
}

fn managed_native_at(root: &Path) -> Option<PathBuf> {
    if !no_reparse_ancestors(root) {
        return None;
    }
    let mut candidates: Vec<_> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path().join("codex.exe");
            (managed_codex_path_at(&path, root)
                && is_executable(&path)
                && no_reparse_ancestors(&path))
            .then(|| {
                (
                    entry
                        .metadata()
                        .and_then(|metadata| metadata.modified())
                        .ok(),
                    path,
                )
            })
        })
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

fn relocate_deleted_managed_at(path: &Path, root: &Path) -> Option<PathBuf> {
    if path.try_exists().ok() != Some(false) || !managed_codex_path_at(path, root) {
        return None;
    }
    managed_native_at(root)
}

pub(crate) fn configured_managed_path(settings: &Value) -> Option<PathBuf> {
    let path = PathBuf::from(s(settings, "codexPath").trim().trim_matches('"'));
    let root = managed_codex_root()?;
    managed_codex_path_at(&path, &root).then_some(path)
}

/// A configured path is authoritative. Wrapper paths are mapped to their native
/// package binary instead of invoking a shell/Node wrapper from a GUI process.
pub fn resolve_executable(settings: &Value) -> Result<PathBuf> {
    let configured = s(settings, "codexPath").trim().trim_matches('"');
    let environment = std::env::var("CODEX_CLI_PATH").unwrap_or_default();
    let configured = if configured.is_empty() {
        environment.trim().trim_matches('"')
    } else {
        configured
    };
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if let Some(native) = preferred_executable(&path) {
            return Ok(native);
        }
        if let Some(native) =
            managed_codex_root().and_then(|root| relocate_deleted_managed_at(&path, &root))
        {
            return Ok(native);
        }
        return Err("指定的 Codex CLI 不可用，请选择原生可执行文件或检查安装位置".into());
    }
    #[cfg(windows)]
    let name = "codex.exe";
    #[cfg(not(windows))]
    let name = "codex";
    if let Some(paths) = std::env::var_os("PATH") {
        let directories: Vec<_> = std::env::split_paths(&paths).collect();
        // Prefer an actual binary on Windows; npm command wrappers are not executables.
        for dir in &directories {
            let path = dir.join(name);
            if let Some(native) = preferred_executable(&path) {
                return Ok(native);
            }
        }
        for dir in &directories {
            if let Some(path) = package_native_paths(dir)
                .into_iter()
                .find(|p| is_executable(p))
            {
                return Ok(path);
            }
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join("AppData/Local")));
        if let Some(local) = local {
            let root = local.join("OpenAI/Codex/bin");
            if let Some(path) = managed_native_at(&root) {
                return Ok(path);
            }
        }
        let roaming = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join("AppData/Roaming")));
        if let Some(roaming) = roaming {
            if let Some(path) = package_native_paths(&roaming.join("npm"))
                .into_iter()
                .find(|p| is_executable(p))
            {
                return Ok(path);
            }
        }
    }
    let mut directories = vec![];
    if let Some(home) = home {
        directories.extend([
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join(".npm-global/bin"),
        ]);
        for package_base in [home.join(".npm-global/lib"), home.join(".local/lib")] {
            if let Some(path) = package_native_paths(&package_base)
                .into_iter()
                .find(|p| is_executable(p))
            {
                return Ok(path);
            }
        }
    }
    #[cfg(not(windows))]
    {
        for base in ["/opt/homebrew/lib", "/usr/local/lib"] {
            if let Some(path) = package_native_paths(Path::new(base))
                .into_iter()
                .find(|p| is_executable(p))
            {
                return Ok(path);
            }
        }
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ]);
    }
    directories
        .into_iter()
        .find_map(|p| preferred_executable(&p.join(name)))
        .ok_or_else(|| {
            "未找到 Codex CLI，请在渠道与设置中选择本机 CLI；Windows 使用 codex.exe".into()
        })
}

fn command(path: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(path);
    command.current_dir(std::env::temp_dir()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

const INSPECTION_STDOUT_LIMIT:usize=262_144;
const INSPECTION_STDERR_LIMIT:usize=65_536;
// Native cold launch can exceed three seconds under concurrent compilation on
// macOS. This only bounds local version/help inspection, never a model request.
const INSPECTION_TIMEOUT:Duration=Duration::from_secs(10);
const RPC_LINE_LIMIT:usize=1_048_576;
const RPC_TOTAL_LIMIT:usize=16_777_216;
const RPC_FRAME_LIMIT:usize=2048;
async fn bounded_inspection_read(mut reader:impl AsyncRead+Unpin,limit:usize)->Result<Vec<u8>> {
    let mut bytes=Vec::new();
    (&mut reader).take(limit as u64+1).read_to_end(&mut bytes).await.map_err(|_|"CLI 检查输出读取失败".to_string())?;
    if bytes.len()>limit {return Err("CLI_INSPECTION_OUTPUT_LIMIT: CLI 检查输出超过限制，未发起生成".into());}
    Ok(bytes)
}
async fn bounded_rpc_line(reader:&mut (impl AsyncBufRead+Unpin))->Result<Option<Vec<u8>>> {
    let mut line=Vec::new();
    loop {
        let buffer=reader.fill_buf().await.map_err(|_|"无法读取 CLI 状态".to_string())?;
        if buffer.is_empty() {return if line.is_empty(){Ok(None)}else{Ok(Some(line))};}
        let end=buffer.iter().position(|b|*b==b'\n').map(|i|i+1);
        let length=end.unwrap_or(buffer.len());
        if line.len()+length>RPC_LINE_LIMIT {return Err("CLI_RPC_LINE_LIMIT: CLI 状态行超过安全读取上限".into());}
        line.extend_from_slice(&buffer[..length]);reader.consume(length);
        if end.is_some() {return Ok(Some(line));}
    }
}
async fn inspect_command(path: &Path, arguments: &[&str]) -> Result<String> {
    let mut cmd=command(path);
    cmd.args(arguments).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let (mut child,_registration)=crate::text_executor::spawn_cli_child(&mut cmd)?;
    let stdout=child.stdout.take().ok_or("CLI 检查输出不可用")?;
    let stderr=child.stderr.take().ok_or("CLI 检查错误输出不可用")?;
    let result=tokio::time::timeout(INSPECTION_TIMEOUT,async {
        let (stdout,stderr,status)=tokio::try_join!(
            bounded_inspection_read(stdout,INSPECTION_STDOUT_LIMIT),
            bounded_inspection_read(stderr,INSPECTION_STDERR_LIMIT),
            async {child.wait().await.map_err(|_|"CLI 检查进程状态不可用".to_string())})?;
        if !status.success() {return Err("Codex CLI 本机检查失败，请检查所选可执行文件".into());}
        Ok(format!("{}\n{}",String::from_utf8_lossy(&stdout),String::from_utf8_lossy(&stderr)))
    }).await.unwrap_or_else(|_|Err("Codex CLI 本机检查超时；未发送生成请求".into()));
    if child.id().is_some() {
        let _=child.start_kill();
        if !matches!(tokio::time::timeout(Duration::from_secs(2),child.wait()).await,Ok(Ok(_))) {
            return Err("CLI_INSPECTION_STOP_UNKNOWN: 检查进程退出尚未确认".into());
        }
    }
    result
}

struct RpcClient {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
    received_bytes: usize,
    received_frames: usize,
    _registration: crate::text_executor::ChildRegistration,
    login_notification: Option<Value>,
}
impl RpcClient {
    async fn open(path: &Path) -> Result<Self> {
        let mut cmd = command(path);
        // Inspection must refer to the same official provider used by exec with
        // --ignore-user-config, rather than a host's custom provider/profile.
        cmd.args([
            "app-server",
            "--stdio",
            "-c",
            "model_provider=\"openai\"",
            "-c",
            "chatgpt_base_url=\"https://chatgpt.com/backend-api\"",
            "-c",
            "features.plugins=false",
            "-c",
            "features.apps=false",
            "-c",
            "features.hooks=false",
            "-c",
            "features.memories=false",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        let (mut child, registration) = crate::text_executor::spawn_cli_child(&mut cmd)?;
        let stdin = child.stdin.take().ok_or("无法打开 CLI 状态输入")?;
        let stdout = child.stdout.take().ok_or("无法读取 CLI 状态")?;
        let mut rpc = Self {
            child,
            stdin,
            reader: BufReader::new(stdout),
            received_bytes: 0,
            received_frames: 0,
            _registration: registration,
            login_notification: None,
        };
        rpc.send(&json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"director-workbench","title":"Director Workbench","version":"0.1.0"},"capabilities":{"experimentalApi":false}}})).await?;
        rpc.response(1).await?;
        rpc.send(&json!({"method":"initialized"})).await?;
        Ok(rpc)
    }
    async fn send(&mut self, value: &Value) -> Result<()> {
        self.stdin
            .write_all(format!("{value}\n").as_bytes())
            .await
            .map_err(|_| "无法发送 CLI 状态请求".to_string())
    }
    async fn next(&mut self) -> Result<Value> {
        let result=self.next_bounded().await;
        if result.is_err() {self.close().await?;}
        result
    }
    async fn next_bounded(&mut self) -> Result<Value> {
        loop {
            let bytes=bounded_rpc_line(&mut self.reader).await?.ok_or("Codex CLI 状态进程已退出")?;
            self.received_bytes+=bytes.len();self.received_frames+=1;
            if self.received_bytes>RPC_TOTAL_LIMIT || self.received_frames>RPC_FRAME_LIMIT {
                return Err("CLI_RPC_OUTPUT_LIMIT: CLI 状态累计输出超过安全上限".into());
            }
            let line=String::from_utf8(bytes).map_err(|_|"CLI 状态包含无效 UTF-8".to_string())?;
            if let Ok(value) = serde_json::from_str(&line) {
                return Ok(value);
            }
        }
    }
    async fn response(&mut self, expected: u64) -> Result<Value> {
        loop {
            let value = self.next().await?;
            if value["id"].as_u64() == Some(expected) {
                if !value["error"].is_null() {
                    return Err("Codex CLI 暂时无法读取此项，请刷新或检查登录".into());
                }
                return Ok(value["result"].clone());
            }
            if value["method"] == "account/login/completed" {
                self.login_notification = Some(value["params"].clone());
            }
        }
    }
    async fn call(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        self.send(&json!({"id":id,"method":method,"params":params}))
            .await?;
        self.response(id).await
    }
    async fn close(&mut self)->Result<()> {
        if self.child.id().is_some() {let _=self.child.start_kill();}
        match tokio::time::timeout(Duration::from_secs(2),self.child.wait()).await {
            Ok(Ok(_))=>Ok(()),
            _=>Err("CLI_RPC_STOP_UNKNOWN: CLI 状态进程退出尚未确认".into()),
        }
    }
}

fn normalize_models(payload: &Value) -> Vec<Value> {
    payload["data"].as_array().into_iter().flatten().filter(|model| model["hidden"].as_bool() != Some(true)).filter_map(|model| {
        let id = model["model"].as_str().or_else(|| model["id"].as_str())?;
        let efforts: Vec<_> = model["supportedReasoningEfforts"].as_array().into_iter().flatten().filter_map(|value| value.as_str().or_else(|| value["reasoningEffort"].as_str())).collect();
        Some(json!({"id":id,"displayName":model["displayName"].as_str().unwrap_or(id),"supportedReasoningEfforts":efforts,"defaultReasoningEffort":model["defaultReasoningEffort"],"isDefault":model["isDefault"].as_bool().unwrap_or(false)}))
    }).collect()
}
fn window(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    let used = value["usedPercent"].as_f64();
    json!({"usedPercent":used,"remainingPercent":used.map(|used| (100.0-used).clamp(0.0,100.0)),"windowDurationMins":value["windowDurationMins"],"resetsAt":value["resetsAt"]})
}
fn normalize_limits(payload: &Value) -> Vec<Value> {
    let values: Vec<_> = if let Some(map) = payload["rateLimitsByLimitId"].as_object() {
        map.values().collect()
    } else if payload["rateLimits"].is_object() {
        vec![&payload["rateLimits"]]
    } else {
        vec![]
    };
    values.into_iter().map(|v| json!({"id":v["limitId"].as_str().unwrap_or("codex"),"name":v["limitName"],"planType":v["planType"],"primary":window(&v["primary"]),"secondary":window(&v["secondary"]),"credits":if v["credits"].is_object(){json!({"balance":v["credits"]["balance"],"hasCredits":v["credits"]["hasCredits"].as_bool().unwrap_or(false),"unlimited":v["credits"]["unlimited"].as_bool().unwrap_or(false)})}else{Value::Null}})).collect()
}

async fn read_status(path: &Path) -> Result<Value> {
    let (version, help) = tokio::try_join!(
        inspect_command(path, &["--version"]),
        inspect_command(path, &["exec", "--help"])
    )?;
    let version = version
        .lines()
        .find(|line| line.starts_with("codex-cli "))
        .ok_or("所选程序不是 Codex CLI")?
        .trim()
        .to_string();
    let compatible = [
        "--json",
        "--output-schema",
        "--output-last-message",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
    ]
    .iter()
    .all(|flag| help.contains(flag));
    let mut rpc = RpcClient::open(path).await?;
    let account = rpc
        .call(2, "account/read", json!({"refreshToken":false}))
        .await?;
    let account = account["account"].clone();
    let logged_in = account.is_object();
    let mut warnings = vec![];
    if !compatible {
        warnings.push("当前 CLI 缺少文本隔离或结构化输出参数，请升级 CLI".to_string());
    }
    let mut models = vec![];
    let mut cursor = Value::Null;
    for page in 0..10 {
        match rpc
            .call(
                10 + page,
                "model/list",
                json!({"limit":100,"cursor":cursor,"includeHidden":false}),
            )
            .await
        {
            Ok(payload) => {
                models.extend(normalize_models(&payload));
                cursor = payload["nextCursor"].clone();
                if cursor.is_null() {
                    break;
                }
            }
            Err(_) => {
                warnings.push("模型目录暂时无法读取，请刷新；不会自动替换模型".into());
                break;
            }
        }
    }
    let limits = if logged_in {
        match tokio::time::timeout(
            Duration::from_secs(5),
            rpc.call(30, "account/rateLimits/read", Value::Null),
        )
        .await
        {
            Ok(Ok(payload)) => normalize_limits(&payload),
            _ => {
                warnings.push("额度暂时无法读取，不表示剩余额度为零".into());
                vec![]
            }
        }
    } else {
        vec![]
    };
    rpc.close().await?;
    let account_summary = if logged_in {
        json!({"type":account["type"],"email":account["email"],"planType":account["planType"]})
    } else {
        Value::Null
    };
    Ok(
        json!({"available":true,"ready":logged_in && compatible && !models.is_empty(),"executable":path.to_string_lossy(),"version":version,"loggedIn":logged_in,"account":account_summary,"models":models,"limits":limits,"checkedAt":crate::store::now(),"warnings":warnings,"error":if logged_in{Value::Null}else{json!("Codex CLI 尚未登录，请在此完成浏览器登录")},"generationRequestsSent":0,"catalogIsEntitlementCheck":false,"globalConcurrency":4,"episodeConcurrency":2,"login":login_snapshot()}),
    )
}

/// Forced inspection is used by Settings; execution preflight reuses a short,
/// path-bound cache and never takes the workflow submission lock.
pub async fn status(settings: &Value, force: bool) -> Value {
    let path = match resolve_executable(settings) {
        Ok(path) => path,
        Err(error) => {
            return json!({"available":false,"ready":false,"loggedIn":false,"executable":Value::Null,"version":Value::Null,"account":Value::Null,"models":[],"limits":[],"checkedAt":crate::store::now(),"warnings":[],"error":error,"login":login_snapshot(),"generationRequestsSent":0})
        }
    };
    if !force {
        if let Ok(cache) = CACHE.get_or_init(|| Mutex::new(None)).lock() {
            if let Some((identity, checked, value)) = cache.as_ref() {
                if identity == &path && checked.elapsed() < CACHE_AGE {
                    let mut value = value.clone();
                    value["login"] = login_snapshot();
                    return value;
                }
            }
        }
    }
    let result = tokio::time::timeout(STATUS_TIMEOUT, read_status(&path)).await;
    let value = match result {
        Ok(Ok(value)) => value,
        failure => {
            json!({"available":true,"ready":false,"loggedIn":false,"executable":path.to_string_lossy(),"version":Value::Null,"account":Value::Null,"models":[],"limits":[],"checkedAt":crate::store::now(),"warnings":[],"error":match failure{Ok(Err(error))=>error,_=>"Codex CLI 状态检查超时，请稍后刷新；未发送生成请求".into()},"login":login_snapshot(),"generationRequestsSent":0})
        }
    };
    if let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(None)).lock() {
        *cache = Some((path, Instant::now(), value.clone()));
    }
    value
}

/// User-triggered readiness preserves local installation evidence when the
/// separate account/catalog RPC fails. It does not log in, refresh a token or exec.
pub async fn readiness_status(settings:&Value)->Value {
    let path=match resolve_executable(settings) {Ok(path)=>path,Err(_)=>return status(settings,true).await};
    let local=match inspect_command(&path,&["--version"]).await {
        Ok(text)=>match text.lines().find(|line|line.starts_with("codex-cli ")) {
            Some(version)=>version.trim().to_owned(),
            None=>return json!({"available":true,"ready":false,"loggedIn":false,"readinessRpcReady":false,"version":null,"executable":path,"models":[],"limits":[],"warnings":[],"error":"所选程序未返回 Codex CLI 版本，未检查账户或模型","generationRequestsSent":0}),
        },
        Err(error)=>return json!({"available":true,"ready":false,"loggedIn":false,"readinessRpcReady":false,"version":null,"executable":path,"models":[],"limits":[],"warnings":[],"error":error,"generationRequestsSent":0}),
    };
    let mut result=status(settings,true).await;
    result["readinessRpcReady"]=json!(result["version"].as_str().is_some_and(|v|v.starts_with("codex-cli ")));
    result["version"]=json!(local);
    result
}

pub fn validate_selection(status: &Value, model: &str, reasoning: &str) -> Result<()> {
    if status["ready"].as_bool() != Some(true) {
        return Err(status["error"]
            .as_str()
            .unwrap_or("Codex CLI 尚未准备好，请在渠道与设置中检查登录及版本")
            .into());
    }
    let selected = status["models"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["id"] == model)
        .ok_or("所选文本模型不在 CLI 模型目录中，请刷新并选择模型")?;
    if !selected["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|effort| effort == reasoning)
    {
        return Err("所选文本模型不支持此推理强度，请调整后再生成".into());
    }
    for limit in status["limits"].as_array().into_iter().flatten().filter(|v| v["id"] == "codex") {
        if limit.pointer("/credits/unlimited").and_then(Value::as_bool) == Some(true)
            || limit.pointer("/credits/hasCredits").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        for window in [&limit["primary"], &limit["secondary"]] {
            if window["usedPercent"].as_f64().is_some_and(|used| used.is_finite() && used >= 100.0) {
                return Err(format!("CODEX_USAGE_LIMIT：Codex 当前额度已用完；恢复时间 {}，已完成步骤保留，暂不提交模型请求", window["resetsAt"]));
            }
        }
    }
    Ok(())
}
pub async fn ensure_ready(settings: &Value, model: &str, reasoning: &str) -> Result<()> {
    if s(settings, "textProvider") == "api" {
        return Ok(());
    }
    validate_selection(&status(settings, false).await, model, reasoning)
}

/// Resolve only an explicitly empty "CLI default" selection. Existing named
/// models and efforts are validated, never replaced by a silent fallback.
/// The caller persists this returned settings object in its workflow snapshot.
pub async fn resolve_settings(settings: &Value) -> Result<Value> {
    if s(settings, "textProvider") == "api" {
        return Ok(settings.clone());
    }
    let inspected = status(settings, false).await;
    let mut resolved = settings.clone();
    if s(settings, "textModel").trim().is_empty() {
        let default = inspected["models"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|model| model["isDefault"].as_bool() == Some(true))
            .ok_or("CLI 尚未返回默认模型，请刷新并选择一个文本模型")?;
        resolved["textModel"] = default["id"].clone();
        if s(settings, "reasoning").trim().is_empty() {
            resolved["reasoning"] = default["defaultReasoningEffort"].clone();
        }
    }
    validate_selection(
        &inspected,
        s(&resolved, "textModel"),
        s(&resolved, "reasoning"),
    )?;
    let native = verified_native_path(Path::new(s(&inspected, "executable")))?;
    resolved["textCliVersion"] = inspected["version"].clone();
    resolved["textCliExecutable"] = json!(native);
    resolved["codexPath"] = json!(native);
    Ok(resolved)
}

struct LoginSession {
    state: Mutex<Value>,
    cancel: AtomicBool,
}
fn login_snapshot() -> Value {
    LOGIN
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|session| {
            session
                .as_ref()
                .and_then(|v| v.state.lock().ok().map(|v| v.clone()))
        })
        .unwrap_or_else(|| json!({"state":"idle"}))
}
pub fn login_status() -> Value {
    json!({"login":login_snapshot()})
}
fn clear_cache() {
    if let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(None)).lock() {
        *cache = None;
    }
}
pub fn cancel_login() -> Value {
    if let Ok(session) = LOGIN.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(session) = session.as_ref() {
            session.cancel.store(true, Ordering::SeqCst);
            if let Ok(mut state) = session.state.lock() {
                if state["state"] == "pending" {
                    *state = json!({"state":"cancelled"});
                }
            }
        }
    }
    login_status()
}
/// Only the pending login owned by this workbench is cancelled. The desktop's
/// text executor registry separately kills/drains its own app-server children.
pub fn shutdown() {
    cancel_login();
}

fn validated_auth_url(url: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(url).map_err(|_| "登录地址无效")?;
    if url.scheme() != "https" || url.host_str() != Some("auth.openai.com") {
        return Err("登录地址不是官方授权地址".into());
    }
    Ok(url)
}
async fn open_auth_url(url: &str) -> Result<()> {
    let url = validated_auth_url(url)?;
    #[cfg(windows)]
    let mut command = {
        let mut c = tokio::process::Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url.as_str()]);
        c.creation_flags(0x08000000);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = tokio::process::Command::new("/usr/bin/open");
        c.arg(url.as_str());
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = tokio::process::Command::new("xdg-open");
        c.arg(url.as_str());
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = tokio::time::timeout(Duration::from_secs(5), command.status())
        .await
        .map_err(|_| "打开浏览器超时，请使用登录链接")?
        .map_err(|_| "无法打开系统浏览器，请使用登录链接")?;
    if !status.success() {
        return Err("无法打开系统浏览器，请使用登录链接".into());
    }
    Ok(())
}
pub async fn open_login_url() -> Result<Value> {
    let state = login_snapshot();
    if state["state"] != "pending" {
        return Err("当前没有等待中的登录，请重新点击登录".into());
    }
    open_auth_url(state["authUrl"].as_str().ok_or("登录链接尚未取得")?).await?;
    Ok(json!({"login":state}))
}
pub async fn start_login(settings: &Value) -> Result<Value> {
    let response = begin_login(settings).await?;
    if let Some(url) = response["login"]["authUrl"].as_str() {
        if let Err(error) = open_auth_url(url).await {
            if let Ok(sessions) = LOGIN.get_or_init(|| Mutex::new(None)).lock() {
                if let Some(session) = sessions.as_ref() {
                    if let Ok(mut state) = session.state.lock() {
                        state["error"] = json!(error);
                    }
                }
            }
        }
    }
    Ok(login_status())
}
async fn begin_login(settings: &Value) -> Result<Value> {
    let _start_gate = LOGIN_START
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let prior = login_snapshot();
    if prior["state"] == "pending" {
        return Ok(json!({"login":prior}));
    }
    let path = resolve_executable(settings)?;
    let mut rpc = tokio::time::timeout(STATUS_TIMEOUT, RpcClient::open(&path))
        .await
        .map_err(|_| "启动 CLI 登录超时")??;
    let result = tokio::time::timeout(
        STATUS_TIMEOUT,
        rpc.call(2, "account/login/start", json!({"type":"chatgpt"})),
    )
    .await
    .map_err(|_| "取得 CLI 登录链接超时")??;
    let login_id = result["loginId"]
        .as_str()
        .ok_or("CLI 未返回登录标识")?
        .to_string();
    let url = result["authUrl"]
        .as_str()
        .ok_or("CLI 未返回官方登录链接")?
        .to_string();
    validated_auth_url(&url)?;
    let session = Arc::new(LoginSession {
        state: Mutex::new(json!({"state":"pending","loginId":login_id,"authUrl":url})),
        cancel: AtomicBool::new(false),
    });
    *LOGIN
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|e| e.to_string())? = Some(session.clone());
    clear_cache();
    control_runtime()?.spawn(async move {
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if session.cancel.load(Ordering::SeqCst) {
                let _=rpc.send(&json!({"id":3,"method":"account/login/cancel","params":{"loginId":login_id}})).await;
                break;
            }
            if Instant::now() > deadline {
                if let Ok(mut state) = session.state.lock() {
                    *state = json!({"state":"failed","error":"登录等待超时，请重新登录"});
                }
                break;
            }
            let message = if let Some(params) = rpc.login_notification.take() {
                Ok(json!({"method":"account/login/completed","params":params}))
            } else {
                match tokio::time::timeout(Duration::from_millis(250), rpc.next()).await {
                    Ok(value) => value,
                    Err(_) => continue,
                }
            };
            match message {
                Ok(value)
                    if value["method"] == "account/login/completed"
                        && value["params"]["loginId"] == login_id =>
                {
                    if let Ok(mut state) = session.state.lock() {
                        if session.cancel.load(Ordering::SeqCst) || state["state"]!="pending" { break; }
                        *state = if value["params"]["success"].as_bool() == Some(true) {
                            json!({"state":"completed"})
                        } else {
                            json!({"state":"failed","error":"官方登录未完成，请重新登录"})
                        };
                    }
                    clear_cache();
                    break;
                }
                Err(_) => {
                    if let Ok(mut state) = session.state.lock() {
                        if state["state"] == "pending" {
                            *state =
                                json!({"state":"failed","error":"CLI 登录连接已结束，请重新登录"});
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
        let _ = rpc.close().await;
    });
    Ok(login_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_quota_exhaustion_blocks_dispatch_but_unknown_limits_are_not_zero() {
        let base = json!({"ready":true,"models":[{"id":"one","supportedReasoningEfforts":["medium"]}]});
        assert!(validate_selection(&base,"one","medium").is_ok());
        for key in ["primary", "secondary"] {
            let mut status = base.clone();
            status["limits"] = json!([{"id":"codex",key:{"usedPercent":100.0,"resetsAt":12345}}]);
            assert!(validate_selection(&status,"one","medium").unwrap_err().contains("CODEX_USAGE_LIMIT"));
            status["limits"][0][key]["usedPercent"] = Value::Null;
            assert!(validate_selection(&status,"one","medium").is_ok());
            status["limits"][0][key]["usedPercent"] = json!(99.9);
            assert!(validate_selection(&status,"one","medium").is_ok());
            status["limits"][0][key]["usedPercent"] = json!(100.0);
            status["limits"][0]["credits"] = json!({"hasCredits":true});
            assert!(validate_selection(&status,"one","medium").is_ok());
        }
    }
    #[test]
    fn model_capabilities_preserve_each_models_efforts_and_hide_internal_entries() {
        let models = normalize_models(
            &json!({"data":[{"model":"one","displayName":"One","supportedReasoningEfforts":[{"reasoningEffort":"medium"},{"reasoningEffort":"max"}],"defaultReasoningEffort":"medium","isDefault":true},{"model":"two","supportedReasoningEfforts":[{"reasoningEffort":"low"}],"hidden":false},{"model":"private","hidden":true}]}),
        );
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0]["supportedReasoningEfforts"],
            json!(["medium", "max"])
        );
        let status = json!({"ready":true,"models":models});
        assert!(validate_selection(&status, "one", "max").is_ok());
        assert!(validate_selection(&status, "two", "max").is_err());
        assert!(validate_selection(&status, "missing", "medium").is_err());
        assert!(
            validate_selection(&json!({"ready":false,"error":"尚未登录"}), "one", "medium")
                .unwrap_err()
                .contains("尚未登录")
        );
    }
    #[test]
    fn quota_summary_contains_display_fields_but_no_account_identity_or_unknown_secrets() {
        let limits = normalize_limits(
            &json!({"accountId":"private-id","accessToken":"secret","rateLimitsByLimitId":{"codex":{"limitId":"codex","primary":{"usedPercent":27,"windowDurationMins":10080,"resetsAt":123},"secondary":null,"credits":{"balance":"20","hasCredits":true,"unlimited":false,"token":"secret"}}}}),
        );
        assert_eq!(limits[0]["primary"]["remainingPercent"], 73.0);
        assert!(!serde_json::to_string(&limits).unwrap().contains("secret"));
        assert!(!serde_json::to_string(&limits)
            .unwrap()
            .contains("private-id"));
        assert!(normalize_limits(&json!({})).is_empty());
    }
    #[test]
    fn invalid_explicit_path_does_not_silently_fallback_to_a_different_cli() {
        let status =
            resolve_executable(&json!({"codexPath":"Z:/definitely-not-installed/codex.exe"}));
        assert!(status.is_err());
    }
    #[test]
    fn managed_installation_relocation_only_uses_complete_native_versions() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("OpenAI/Codex/bin");
        std::fs::create_dir_all(root.join("0000000000000001")).unwrap();
        std::fs::create_dir_all(root.join("0000000000000002")).unwrap();
        std::fs::create_dir_all(root.join("not-an-official-version")).unwrap();
        let native = root.join("0000000000000001/codex.exe");
        std::fs::write(&native, b"offline fixture; never executed").unwrap();
        std::fs::write(root.join("not-an-official-version/codex.exe"), b"excluded").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let removed = root.join("0000000000000000/codex.exe");
        assert_eq!(
            relocate_deleted_managed_at(&removed, &root),
            Some(native.clone())
        );
        assert!(
            relocate_deleted_managed_at(&native, &root).is_none(),
            "existing explicit binaries stay pinned"
        );
        for foreign in [
            directory.path().join("codex.exe"),
            root.join("0000000000000000/nested/codex.exe"),
            root.join("not-an-official-version/codex.exe"),
            root.join("0000000000000000/../0000000000000002/codex.exe"),
        ] {
            assert!(relocate_deleted_managed_at(&foreign, &root).is_none());
        }
        std::fs::remove_file(&native).unwrap();
        assert!(
            relocate_deleted_managed_at(&removed, &root).is_none(),
            "incomplete updates cannot become execution targets"
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_installation_rejects_reparse_root_and_binary() {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("bin");
        let version = root.join("0000000000000001");
        std::fs::create_dir_all(&version).unwrap();
        let foreign = directory.path().join("foreign.exe");
        std::fs::write(&foreign, b"never executed").unwrap();
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&foreign, version.join("codex.exe")).unwrap();
        assert!(managed_native_at(&root).is_none());
        let link = directory.path().join("linked-bin");
        symlink(&root, &link).unwrap();
        assert!(managed_native_at(&link).is_none());
    }
    #[tokio::test]
    async fn untrusted_login_url_is_rejected_before_starting_browser() {
        assert!(open_auth_url("https://evil.example/auth").await.is_err());
        assert!(open_auth_url("file:///C:/private").await.is_err());
    }
    #[test]
    fn owned_login_continues_after_dispatch_returns_and_cancel_is_final() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("login-fixture.rs");
        let executable = directory.path().join(if cfg!(windows) {
            "login-fixture.exe"
        } else {
            "login-fixture"
        });
        std::fs::write(&source,r#"
use std::io::{BufRead,Write};
fn main(){
 for line in std::io::stdin().lock().lines(){let line=line.unwrap();
  if line.contains("\"initialize\""){println!("{{\"id\":1,\"result\":{{}}}}");}
  if line.contains("account/login/start"){
   println!("{{\"id\":2,\"result\":{{\"loginId\":\"offline-login\",\"authUrl\":\"https://auth.openai.com/oauth/authorize?offline=true\"}}}}");
   std::thread::spawn(||{std::thread::sleep(std::time::Duration::from_millis(200));println!("{{\"method\":\"account/login/completed\",\"params\":{{\"loginId\":\"offline-login\",\"success\":true}}}}");std::io::stdout().flush().unwrap();});
  }
  std::io::stdout().flush().unwrap();
 }
}
"#).unwrap();
        let compiler = std::env::var_os("RUSTC")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                            .map(|p| PathBuf::from(p).join(".cargo"))
                    })
                    .map(|p| {
                        p.join("bin")
                            .join(if cfg!(windows) { "rustc.exe" } else { "rustc" })
                    })
                    .unwrap_or_else(|| PathBuf::from("rustc"))
            });
        let mut compile = std::process::Command::new(compiler);
        compile
            .args(["--edition=2021", "--crate-name", "login_fixture"])
            .arg(&source)
            .arg("-o")
            .arg(&executable);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            compile.creation_flags(0x08000000);
        }
        let output = compile.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let settings = json!({"codexPath":executable});
        // The real login core is exercised, with only browser opening kept out of
        // this offline fixture. No credentials or external servers are involved.
        let pending = control_runtime()
            .unwrap()
            .block_on(begin_login(&settings))
            .unwrap();
        assert_eq!(pending["login"]["state"], "pending");
        let deadline = Instant::now() + Duration::from_secs(2);
        while login_status()["login"]["state"] == "pending" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            login_status()["login"]["state"],
            "completed",
            "callback must survive after block_on returns"
        );
        assert!(crate::text_executor::await_local_shutdown(2));
        let pending = control_runtime()
            .unwrap()
            .block_on(begin_login(&settings))
            .unwrap();
        assert_eq!(pending["login"]["state"], "pending");
        assert_eq!(cancel_login()["login"]["state"], "cancelled");
        assert!(crate::text_executor::await_local_shutdown(2));
        assert_eq!(
            login_status()["login"]["state"],
            "cancelled",
            "late callback cannot override a cancelled login"
        );
    }
    #[tokio::test]
    #[ignore = "reads native local CLI version/login/catalog/limits; sends no model generation"]
    async fn local_status_is_read_only_and_bounded() {
        let status = status(&json!({}), true).await;
        assert_eq!(status["available"], true);
        assert_eq!(status["ready"], true, "{}", status["error"]);
        assert_eq!(status["generationRequestsSent"], 0);
        assert!(status["models"].as_array().is_some_and(|m| !m.is_empty()));
        println!(
            "{}",
            json!({"available":status["available"],"ready":status["ready"],"version":status["version"],"modelCount":status["models"].as_array().map(Vec::len),"limitCount":status["limits"].as_array().map(Vec::len),"generationRequestsSent":0})
        );
    }
}
