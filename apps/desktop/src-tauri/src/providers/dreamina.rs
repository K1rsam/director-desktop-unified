use super::*;
use super::cli_observation::CliObservation;
use std::{
    collections::HashMap,
    io::Read,
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

fn cli_path(config: &ProviderConfig) -> ProviderResult<&Path> {
    let path = config.cli_path.as_deref().ok_or_else(|| {
        ProviderError::new(
            "CLI_NOT_CONFIGURED",
            "请选择已安装的官方 dreamina 可执行文件。",
        )
    })?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ProviderError::new("CLI_NOT_FOUND", "官方 CLI 路径不存在。"))?;
    if !path.is_absolute() || !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ProviderError::new(
            "CLI_PATH",
            "官方 CLI 必须为明确配置的绝对文件路径。",
        ));
    }
    Ok(path)
}

#[derive(Debug)]
struct CliOutput {
    success: bool,
    exit_code: Option<i32>,
    signal: Option<i32>,
    launch: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

// The CLI uses Go's proxy environment. A desktop launched by Explorer/Sky can
// have no proxy environment even though this user's static Windows proxy is on.
// Read that configuration for this child only; never edit the process/system env.
#[derive(Clone, Default)]
struct ProxyEnvironment {
    http: Option<String>,
    https: Option<String>,
    all: Option<String>,
    bypass: Option<String>,
}
#[derive(Default)]
struct WindowsProxySettings {
    enabled: bool,
    server: String,
    bypass: String,
    // PAC/WPAD are deliberately not resolved or used as a proxy address.
    pac_present: bool,
}
#[derive(Default)]
struct StaticProxies {
    http: Option<String>,
    https: Option<String>,
}

fn inherited_proxy_environment() -> ProxyEnvironment {
    let read = |upper: &str, lower: &str| {
        std::env::var_os(upper)
            .or_else(|| std::env::var_os(lower))
            .map(|v| v.to_string_lossy().into_owned())
    };
    ProxyEnvironment {
        http: read("HTTP_PROXY", "http_proxy"),
        https: read("HTTPS_PROXY", "https_proxy"),
        all: read("ALL_PROXY", "all_proxy"),
        bypass: read("NO_PROXY", "no_proxy"),
    }
}

fn static_proxy_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
        || value.contains('@')
    {
        return None;
    }
    let value = if value.contains("://") {
        value.to_owned()
    } else {
        format!("http://{value}")
    };
    let url = reqwest::Url::parse(&value).ok()?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return None;
    }
    Some(url.to_string())
}

fn parse_static_proxies(value: &str) -> Option<StaticProxies> {
    if value.len() > 16 * 1024 || value.chars().any(char::is_control) {
        return None;
    }
    let entries: Vec<_> = value
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if entries.is_empty() {
        return None;
    }
    if entries.len() == 1 && !entries[0].contains('=') {
        let url = static_proxy_url(entries[0])?;
        return Some(StaticProxies {
            http: Some(url.clone()),
            https: Some(url),
        });
    }
    let mut proxies = StaticProxies::default();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let (protocol, address) = entry.split_once('=')?;
        let protocol = protocol.trim().to_ascii_lowercase();
        if !seen.insert(protocol.clone()) {
            return None;
        }
        match protocol.as_str() {
            "http" => proxies.http = Some(static_proxy_url(address)?),
            "https" => proxies.https = Some(static_proxy_url(address)?),
            // Do not turn a SOCKS/FTP entry into an HTTP proxy.
            "socks" | "ftp" => {}
            _ => return None,
        }
    }
    Some(proxies)
}

fn converted_windows_bypass(value: &str) -> Option<String> {
    if value.len() > 16 * 1024 || value.chars().any(char::is_control) {
        return None;
    }
    let mut supported = Vec::new();
    for value in value.split(';').map(str::trim).filter(|v| !v.is_empty()) {
        // WinINET <local>, arbitrary wildcard patterns, URL/port patterns and
        // CIDR semantics are not equivalent to Go/reqwest NO_PROXY. Skip them.
        let suffix = value.strip_prefix("*.");
        let candidate = suffix
            .map(|domain| format!(".{domain}"))
            .unwrap_or_else(|| value.to_owned());
        let ip = candidate.parse::<std::net::IpAddr>().is_ok();
        let host = candidate.trim_start_matches('.');
        let domain = !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            });
        // A bare DNS hostname would also bypass its subdomains in Go, unlike
        // WinINET's exact pattern. Only exact IPs and explicit *.suffix agree.
        if !ip && !(suffix.is_some() && domain) {
            continue;
        }
        let candidate = candidate.to_ascii_lowercase();
        if !supported.contains(&candidate) {
            supported.push(candidate);
        }
    }
    (!supported.is_empty()).then(|| supported.join(","))
}

fn effective_proxy_environment(
    inherited: ProxyEnvironment,
    windows: Option<&WindowsProxySettings>,
) -> ProxyEnvironment {
    let mut result = inherited;
    // An explicit ALL_PROXY (even an empty deliberate override) wins over the
    // system configuration. Map it to missing scheme variables for the Go CLI.
    if let Some(all) = &result.all {
        if result.http.is_none() {
            result.http = Some(all.clone());
        }
        if result.https.is_none() {
            result.https = Some(all.clone());
        }
        return result;
    }
    if let Some(settings) =
        windows.filter(|s| s.enabled && (!s.pac_present || !s.server.trim().is_empty()))
    {
        if let Some(proxies) = parse_static_proxies(&settings.server) {
            let used = (result.http.is_none() && proxies.http.is_some())
                || (result.https.is_none() && proxies.https.is_some());
            if result.http.is_none() {
                result.http = proxies.http;
            }
            if result.https.is_none() {
                result.https = proxies.https;
            }
            if used && result.bypass.is_none() {
                result.bypass = converted_windows_bypass(&settings.bypass);
            }
        }
    }
    result
}

#[cfg(windows)]
fn windows_proxy_settings() -> Option<WindowsProxySettings> {
    use std::ffi::c_void;
    #[repr(C)]
    struct IeProxyConfig {
        auto_detect: i32,
        auto_config: *mut u16,
        proxy: *mut u16,
        bypass: *mut u16,
    }
    #[link(name = "winhttp")]
    extern "system" {
        fn WinHttpGetIEProxyConfigForCurrentUser(config: *mut IeProxyConfig) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }
    unsafe fn copy_wide(pointer: *const u16) -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        for length in 0..32768 {
            if *pointer.add(length) == 0 {
                return String::from_utf16(std::slice::from_raw_parts(pointer, length)).ok();
            }
        }
        None
    }
    let mut config = IeProxyConfig {
        auto_detect: 0,
        auto_config: std::ptr::null_mut(),
        proxy: std::ptr::null_mut(),
        bypass: std::ptr::null_mut(),
    };
    // The API reports lpszProxy only when "use a proxy server" is enabled for
    // the active connection. Every API-owned string is freed even on failure.
    let ok = unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) != 0 };
    let proxy = unsafe { copy_wide(config.proxy) };
    let bypass = unsafe { copy_wide(config.bypass) };
    let pac_present = !config.auto_config.is_null();
    unsafe {
        for pointer in [config.auto_config, config.proxy, config.bypass] {
            if !pointer.is_null() {
                GlobalFree(pointer.cast());
            }
        }
    }
    ok.then(|| WindowsProxySettings {
        enabled: proxy.as_ref().is_some_and(|p| !p.trim().is_empty()),
        server: proxy.unwrap_or_default(),
        bypass: bypass.unwrap_or_default(),
        pac_present,
    })
}
#[cfg(not(windows))]
fn windows_proxy_settings() -> Option<WindowsProxySettings> {
    None
}

fn child_proxy_environment() -> ProxyEnvironment {
    effective_proxy_environment(
        inherited_proxy_environment(),
        windows_proxy_settings().as_ref(),
    )
}

fn apply_child_proxy_environment(command: &mut Command, proxy: &ProxyEnvironment) {
    for (key, value) in [
        ("HTTP_PROXY", &proxy.http),
        ("HTTPS_PROXY", &proxy.https),
        ("ALL_PROXY", &proxy.all),
        ("NO_PROXY", &proxy.bypass),
    ] {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
}

// Official image/video result GETs share the child CLI's deterministic scheme
// precedence. This is local client configuration, not a global env mutation.
pub(super) fn apply_result_proxies(
    builder: reqwest::blocking::ClientBuilder,
) -> ProviderResult<reqwest::blocking::ClientBuilder> {
    result_proxies_from_environment(builder, &child_proxy_environment())
}
fn result_proxies_from_environment(
    mut builder: reqwest::blocking::ClientBuilder,
    environment: &ProxyEnvironment,
) -> ProviderResult<reqwest::blocking::ClientBuilder> {
    builder = builder.no_proxy();
    let bypass = environment
        .bypass
        .as_deref()
        .and_then(reqwest::NoProxy::from_string);
    for (https, address) in [(false, &environment.http), (true, &environment.https)] {
        if let Some(address) = address.as_deref().filter(|s| !s.is_empty()) {
            let proxy = if https {
                reqwest::Proxy::https(address)
            } else {
                reqwest::Proxy::http(address)
            }
            .map_err(|_| {
                ProviderError::new(
                    "PROXY_CONFIGURATION",
                    "即梦结果下载的显式代理配置无效；原任务保持可恢复。",
                )
            })?
            .no_proxy(bypass.clone());
            builder = builder.proxy(proxy);
        }
    }
    Ok(builder)
}

fn execute(
    config: &ProviderConfig,
    args: &[String],
    submitting: bool,
) -> ProviderResult<CliOutput> {
    execute_until(config, args, submitting, None)
}

fn capability_timeout() -> ProviderError {
    ProviderError::new(
        "CLI_CAPABILITY_TIMEOUT",
        "即梦模型能力检查超时，请稍后重试；已有目录与生成任务保留。",
    )
}

// Only version/help inspection supplies this deadline. Generation and original-task
// queries continue to use their configured timeout through `execute` above.
fn execute_until(
    config: &ProviderConfig,
    args: &[String],
    submitting: bool,
    deadline: Option<Instant>,
) -> ProviderResult<CliOutput> {
    execute_until_in_directory(config, args, submitting, deadline, None)
}

fn execute_until_in_directory(
    config: &ProviderConfig,
    args: &[String],
    submitting: bool,
    deadline: Option<Instant>,
    directory: Option<&Path>,
) -> ProviderResult<CliOutput> {
    execute_until_in_directory_observed(config,args,submitting,deadline,directory,None)
}
fn execute_until_in_directory_observed(
    config: &ProviderConfig, args: &[String], submitting: bool, deadline: Option<Instant>,
    directory: Option<&Path>, observation: Option<CliObservation>,
) -> ProviderResult<CliOutput> {
    let _active = CliGuard::enter();
    if shutting_down() {
        return Err(ProviderError::new(
            "CLIENT_CLOSING",
            "客户端正在关闭，未启动新的本地任务。",
        ));
    }
    if deadline.is_some_and(|end| Instant::now() >= end) {
        return Err(capability_timeout());
    }
    // A child-specific working directory must not change how the executable is
    // resolved, or the parent process/account environment.
    let mut command = cli_command_in_directory(config, directory)?;
    let proxy_environment = child_proxy_environment();
    apply_child_proxy_environment(&mut command, &proxy_environment);
    let launch = safe_launch_fingerprint_in_directory(&proxy_environment, directory);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let policy = CliProcessPolicy {
        run: Duration::from_secs(config.timeout_seconds.clamp(1, 900)),
        drain: Duration::from_secs(5),
        cleanup: Duration::from_secs(5),
        inspection_deadline: deadline,
    };
    supervise_command_observed(command, submitting, launch, policy, Arc::new(shutting_down), observation)
}

#[derive(Clone, Copy)]
struct CliProcessPolicy {
    run: Duration,
    drain: Duration,
    cleanup: Duration,
    inspection_deadline: Option<Instant>,
}

struct CliPipeOutput {
    bytes: Vec<u8>,
    exceeded: bool,
}

async fn drain_cli_pipe<R: tokio::io::AsyncRead + Unpin>(
    mut stream: R,
    observation: Option<CliObservation>, stream_name: &'static str,
) -> std::io::Result<CliPipeOutput> {
    use tokio::io::AsyncReadExt;
    let mut chunk = [0; 8192];
    let mut bytes = Vec::new();
    let mut exceeded = false;
    loop {
        let count = match stream.read(&mut chunk).await {
            Ok(count) => count,
            Err(error) => {if let Some(o)=&observation{o.pipe(stream_name,0,false,true);} return Err(error);}
        };
        if let Some(o)=&observation{o.pipe(stream_name,count,count==0,false);}
        if count == 0 {
            return Ok(CliPipeOutput { bytes, exceeded });
        }
        let keep = ((wire::MAX_JSON + 1) as usize - bytes.len()).min(count);
        bytes.extend_from_slice(&chunk[..keep]);
        exceeded |= bytes.len() as u64 > wire::MAX_JSON || keep < count;
        // Keep consuming both pipes after the retention limit. Closing a pipe here
        // can block/change the CLI before it has returned its submission identity.
        tokio::task::yield_now().await;
    }
}

fn cli_execution_error(submitting: bool, code: &str, message: &str) -> ProviderError {
    if submitting {
        ProviderError::uncertain(format!("{message} 请核对原任务，不能自动重发。"))
    } else {
        ProviderError::new(code, message)
    }
}

// The adapter is synchronous. A scoped owner runs async process I/O on its own
// runtime, including when called from an existing Tokio runtime. There are no
// reader threads or detached tasks: cancellation drops both owned pipe futures.
fn supervise_command(
    command: Command,
    submitting: bool,
    launch: String,
    policy: CliProcessPolicy,
    stopped: Arc<dyn Fn() -> bool + Send + Sync>,
) -> ProviderResult<CliOutput> {
    supervise_command_observed(command,submitting,launch,policy,stopped,None)
}
fn supervise_command_observed(
    command: Command, submitting: bool, launch: String, policy: CliProcessPolicy,
    stopped: Arc<dyn Fn() -> bool + Send + Sync>, observation: Option<CliObservation>,
) -> ProviderResult<CliOutput> {
    let spawned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let child_started = spawned.clone();
    let worker_observation=observation.clone();
    let result=std::thread::scope(|scope| {
        scope
            .spawn(move || {
                if let Some(o)=&worker_observation{o.mark("thread_started");}
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| {
                        ProviderError::new("CLI_RUNTIME", "无法建立本地 CLI 监督；尚未提交生成。")
                    })?;
                if let Some(o)=&worker_observation{o.mark("runtime_ready");}
                let result=runtime.block_on(supervise_command_async(
                    command,
                    submitting,
                    launch,
                    policy,
                    stopped,
                    child_started,
                    worker_observation.clone(),
                ));
                if let Some(o)=&worker_observation{o.mark("runtime_finished");}
                drop(runtime);
                if let Some(o)=&worker_observation{o.mark("runtime_dropped");}
                result
            })
            .join()
            .map_err(|_| {
                cli_execution_error(
                    submitting && spawned.load(std::sync::atomic::Ordering::SeqCst),
                    "CLI_RUNTIME",
                    "本地 CLI 监督异常；执行记录保持原状。",
                )
            })?
    });
    if let Some(o)=&observation{o.mark("thread_joined");}
    result
}

async fn supervise_command_async(
    command: Command,
    submitting: bool,
    launch: String,
    policy: CliProcessPolicy,
    stopped: Arc<dyn Fn() -> bool + Send + Sync>,
    spawned: Arc<std::sync::atomic::AtomicBool>,
    observation: Option<CliObservation>,
) -> ProviderResult<CliOutput> {
    if stopped() {
        return Err(ProviderError::new(
            "CLIENT_CLOSING",
            "客户端正在关闭，未启动新的本地任务。",
        ));
    }
    let budget = policy
        .inspection_deadline
        .map(|end| {
            end.saturating_duration_since(Instant::now())
                .min(policy.run)
        })
        .unwrap_or(policy.run);
    if budget.is_zero() {
        return Err(capability_timeout());
    }
    let deadline = tokio::time::Instant::now() + budget;
    if let Some(o)=&observation{o.budget(budget.as_millis().min(u64::MAX as u128) as u64);}
    let mut command = command;
    // Tokio's Windows anonymous ChildStdout uses a blocking ReadFile worker.
    // An inherited writer would keep that worker (and Runtime::drop) alive after
    // cancelling a future. Use private OVERLAPPED server handles for both reads.
    #[cfg(windows)]
    let (stdout, stderr) = {
        let (stdout, writer) = owned_cli_output_pipe().await.map_err(|_| {
            ProviderError::new(
                "CLI_PIPE_START",
                "无法建立本地 CLI 输出管道；尚未提交生成。",
            )
        })?;
        command.stdout(Stdio::from(writer));
        let (stderr, writer) = owned_cli_output_pipe().await.map_err(|_| {
            ProviderError::new(
                "CLI_PIPE_START",
                "无法建立本地 CLI 错误管道；尚未提交生成。",
            )
        })?;
        command.stderr(Stdio::from(writer));
        (stdout, stderr)
    };
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    if let Some(o)=&observation{o.mark("spawn_start");}
    let mut child = command
        .spawn()
        .map_err(|_| ProviderError::new("CLI_START", "无法启动官方 CLI；尚未提交生成。"))?;
    spawned.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(o)=&observation{o.spawned(child.id());o.mark("observing");}
    // Close the parent's copies of the child-side writers immediately. The
    // command is single-use; retaining it would itself prevent pipe EOF.
    drop(command);
    let execution = async {
        #[cfg(not(windows))]
        let stdout = child.stdout.take().ok_or_else(|| cli_execution_error(submitting, "CLI_OUTPUT", "无法读取 CLI 输出。"))?;
        #[cfg(not(windows))]
        let stderr = child.stderr.take().ok_or_else(|| cli_execution_error(submitting, "CLI_OUTPUT", "无法读取 CLI 错误输出。"))?;
        let output = drain_cli_pipe(stdout,observation.clone(),"stdout");
        let errors = drain_cli_pipe(stderr,observation.clone(),"stderr");
        tokio::pin!(output, errors);
        let mut bytes = None;
        let mut error = None;
        let mut status = None;
        let mut drain_deadline = deadline;
        let mut control = tokio::time::interval(Duration::from_millis(20));
        loop {
            if status.is_some() && bytes.is_some() && error.is_some() {
                break Ok((status.unwrap(), bytes.unwrap(), error.unwrap()));
            }
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => {
                    if let Some(o)=&observation{o.timed_out(if policy.inspection_deadline.is_some_and(|end| Instant::now()>=end){"overall"}else{"run"},tokio::time::Instant::now().saturating_duration_since(deadline).as_millis().min(u64::MAX as u128) as u64,status.is_some());}
                    break Err(if !submitting && policy.inspection_deadline.is_some_and(|end| Instant::now() >= end) {
                        capability_timeout()
                    } else {
                        cli_execution_error(submitting, "CLI_TIMEOUT", "CLI 请求超过运行期限；原任务保持可恢复。")
                    });
                }
                _ = tokio::time::sleep_until(drain_deadline), if status.is_some() => {
                    if let Some(o)=&observation{o.timed_out("drain",tokio::time::Instant::now().saturating_duration_since(drain_deadline).as_millis().min(u64::MAX as u128) as u64,true);}
                    break Err(cli_execution_error(submitting, "CLI_OUTPUT_TIMEOUT", "CLI 已退出，但输出管道未在5秒排空期限内结束。"));
                }
                _ = control.tick() => {
                    if let Some(o)=&observation{o.tick();}
                    if stopped() {
                        break Err(cli_execution_error(submitting, "CLIENT_CLOSING", "客户端已停止本地 CLI 执行。"));
                    }
                }
                result = &mut output, if bytes.is_none() => {
                    bytes = Some(result.map_err(|_| cli_execution_error(submitting, "CLI_OUTPUT", "CLI 输出读取失败。"))?);
                }
                result = &mut errors, if error.is_none() => {
                    error = Some(result.map_err(|_| cli_execution_error(submitting, "CLI_OUTPUT", "CLI 错误输出读取失败。"))?);
                }
                result = child.wait(), if status.is_none() => {
                    status = Some(result.map_err(|_| cli_execution_error(submitting, "CLI_WAIT", "CLI 进程状态未确认。"))?);
                    if let Some(o)=&observation{o.exited(status.unwrap());}
                    drain_deadline = (tokio::time::Instant::now() + policy.drain).min(deadline);
                }
            }
        }
    }.await;
    // Both pipe futures are dropped before stop/reap. A descendant retaining a
    // standard handle cannot extend the request deadline or delay TempDir cleanup.
    let (status, bytes, error) = match execution {
        Ok(result) => result,
        Err(mut error) => {
            if !stop_cli_process_observed(&mut child, policy.cleanup, observation.clone()).await {
                error
                    .message
                    .push_str(" 本地进程退出尚未确认；不会自动重发。");
            }
            return Err(error);
        }
    };
    if bytes.exceeded || error.exceeded {
        return Err(cli_execution_error(
            submitting,
            "CLI_OUTPUT_LIMIT",
            "CLI 输出超出大小限制。",
        ));
    }
    if let Some(o)=&observation{o.mark("completed");}
    Ok(CliOutput {
        success: status.success(),
        exit_code: status.code(),
        signal: {
            #[cfg(unix)] { use std::os::unix::process::ExitStatusExt; status.signal() }
            #[cfg(not(unix))] { None }
        },
        launch,
        stdout: bytes.bytes,
        stderr: error.bytes,
    })
}

#[cfg(windows)]
async fn owned_cli_output_pipe(
) -> std::io::Result<(tokio::net::windows::named_pipe::NamedPipeServer, fs::File)> {
    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};
    let name = format!("\\\\.\\pipe\\director-dreamina-{}", uuid::Uuid::new_v4());
    let server = ServerOptions::new()
        .pipe_mode(PipeMode::Byte)
        .access_inbound(true)
        .access_outbound(false)
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&name)?;
    let writer = fs::OpenOptions::new().write(true).open(&name)?;
    server.connect().await?;
    Ok((server, writer))
}

async fn stop_cli_process(child: &mut tokio::process::Child, limit: Duration) -> bool {
    stop_cli_process_observed(child,limit,None).await
}
async fn stop_cli_process_observed(child: &mut tokio::process::Child, limit: Duration, observation: Option<CliObservation>) -> bool {
    if let Some(o)=&observation{o.mark("cleanup_start");}
    // Preserve the existing single pre-kill query and never act on an exited PID.
    let before=child.try_wait();
    if let Some(o)=&observation{o.pre_cleanup(&before);}
    if before.ok().flatten().is_some() {
        if let Some(o)=&observation{o.cleanup_done(true);}
        return true;
    }
    if let Some(o)=&observation{o.kill_requested();}
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let mut command = tokio::process::Command::new("taskkill.exe");
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(0x08000000);
        if let Ok(mut helper) = command.spawn() {
            if tokio::time::timeout(limit, helper.wait()).await.is_err() {
                let _ = helper.start_kill();
                let _ = tokio::time::timeout(Duration::from_millis(500), helper.wait()).await;
            }
        }
    }
    let _ = child.start_kill();
    let confirmed=match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(status))=>{if let Some(o)=&observation{o.exited(status);} true},
        _=>false,
    };
    if let Some(o)=&observation{o.cleanup_done(confirmed);}
    confirmed
}

fn cli_command_in_directory(
    config: &ProviderConfig,
    directory: Option<&Path>,
) -> ProviderResult<Command> {
    let executable = if directory.is_some() {
        fs::canonicalize(cli_path(config)?)
            .map_err(|_| ProviderError::new("CLI_START", "无法定位官方 CLI；尚未提交生成。"))?
    } else {
        cli_path(config)?.to_path_buf()
    };
    let mut command = Command::new(executable);
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    Ok(command)
}

fn safe_launch_fingerprint_for(proxy: &ProxyEnvironment) -> String {
    safe_launch_fingerprint_in_directory(proxy, None)
}
fn safe_launch_fingerprint_in_directory(
    proxy: &ProxyEnvironment,
    directory: Option<&Path>,
) -> String {
    // Fingerprint the effective child proxy values, not the parent's values.
    // Only hashes escape; neither proxy URLs nor OAuth variables are logged.
    let cwd = directory
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .map(|p| p.to_string_lossy().into_owned());
    let identities: Vec<_> = ["USERPROFILE", "HOME", "DREAMINA_HOME"]
        .into_iter()
        .map(|key| (key, std::env::var(key).ok()))
        .collect();
    launch_fingerprint_from(cwd, proxy, &identities)
}
fn launch_fingerprint_from(
    cwd: Option<String>,
    proxy: &ProxyEnvironment,
    identities: &[(&str, Option<String>)],
) -> String {
    let mut context = vec![(
        "cwd",
        cwd.map(|p| if cfg!(windows) { p.to_lowercase() } else { p }),
    )];
    for (key, value) in [
        ("HTTP_PROXY", &proxy.http),
        ("HTTPS_PROXY", &proxy.https),
        ("ALL_PROXY", &proxy.all),
        ("NO_PROXY", &proxy.bypass),
    ] {
        context.push((key, value.clone()));
    }
    for (key, value) in identities {
        context.push((*key, value.clone()));
    }
    hex_hash(&serde_json::to_vec(&context).unwrap_or_default())[..12].to_owned()
}
fn failure_redactions(
    config: Option<&ProviderConfig>,
    args: &[String],
    directory: Option<&Path>,
) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(config) = config {
        values.extend(config.endpoint.iter().cloned());
        values.extend(
            config
                .cli_path
                .iter()
                .map(|p| p.to_string_lossy().into_owned()),
        );
        if let Some(key) = &config.credential_env {
            if let Ok(value) = std::env::var(key) {
                values.push(value);
            }
        }
    }
    values.extend(directory.map(|p| p.to_string_lossy().into_owned()));
    values.extend(
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
    );
    for key in [
        "USERPROFILE",
        "HOME",
        "DREAMINA_HOME",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ] {
        values.extend(std::env::var(key).ok());
    }
    let mut previous_sensitive = false;
    for arg in args {
        let (name, value) = arg.split_once('=').unwrap_or((arg, ""));
        let sensitive = name == "--prompt"
            || name.contains("token")
            || name.contains("secret")
            || name.contains("credential")
            || name.contains("authorization")
            || name.contains("device_code")
            || name.contains("user_code");
        if previous_sensitive {
            values.push(arg.clone());
        }
        if sensitive && !value.is_empty() {
            values.push(value.to_owned());
        }
        previous_sensitive = sensitive && value.is_empty();
    }
    values.retain(|value| !value.is_empty());
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

/// Redact the complete retained stderr BEFORE limiting display length. The
/// supervisor already bounds retained bytes; a cut raw prefix could expose a
/// fragment of a long credential or prompt. Ambiguous lines expose only class.
fn safe_failure_stderr(stderr: &[u8], redactions: &[String]) -> Option<String> {
    if stderr.is_empty() {
        return None;
    }
    let Ok(raw) = std::str::from_utf8(stderr) else {
        return Some("[stderr 内容未安全披露]".into());
    };
    let mut cleaned = raw.to_owned();
    // Do not replace a private directory prefix before recognizing its whole
    // path: replacing C:\\Users\\name alone could leave a project suffix visible.
    let path_literal = |value: &str| {
        value.starts_with('/') || value.starts_with('\\') || value.as_bytes().get(1) == Some(&b':')
    };
    for value in redactions.iter().filter(|value| !path_literal(value)) {
        cleaned = cleaned.replace(value, "[redacted]");
        if let Ok(escaped) = serde_json::to_string(value) {
            cleaned = cleaned.replace(&escaped[1..escaped.len() - 1], "[redacted]");
        }
    }
    // Handle JSON-escaped URLs without decoding arbitrary escapes into content.
    cleaned = cleaned.replace(r"\/", "/");
    for (pattern, replacement) in [
        (r"\x1b\[[0-?]*[ -/]*[@-~]", ""),
        (
            r#"(?i)\b[a-z][a-z0-9+.-]{1,20}://[^\s<>"']+"#,
            "[media-url]",
        ),
        (
            r#"(?i)(?:\\\\\?\\)?[a-z]:[\\/][^\r\n"']*"#,
            "[private-path]",
        ),
        (r#"\\\\[^\\\s]+\\[^\r\n"']*"#, "[private-path]"),
        (r#"(?:^|[\s"'=:(])/[^\r\n"']*"#, " [private-path]"),
        (r#"(?:\./|\.\\)[^\s"'<>]+"#, "[reference-file]"),
        (
            r#"(?i)\b[^\s"'<>:/\\=]+\.(?:png|jpe?g|webp|gif|bmp|mp4|mov|wav|mp3|sqlite|db|exe)\b"#,
            "[reference-file]",
        ),
        (
            r"\b[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b",
            "[credential]",
        ),
        (
            r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            "[id]",
        ),
        (r"(?i)\b[0-9a-f]{32,}\b", "[id]"),
        (r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d+)?\b", "[network-address]"),
        (
            r"(?i)\b(?:[a-z0-9_-]+\.)+[a-z]{2,}(?::\d+)?\b",
            "[network-address]",
        ),
    ] {
        cleaned = regex::Regex::new(pattern)
            .unwrap()
            .replace_all(&cleaned, replacement)
            .into_owned();
    }
    for value in redactions.iter().filter(|value| path_literal(value)) {
        cleaned = cleaned.replace(value, "[private-path]");
        if let Ok(escaped) = serde_json::to_string(value) {
            cleaned = cleaned.replace(&escaped[1..escaped.len() - 1], "[private-path]");
        }
    }
    // Concatenated/underscore debug keys are also sensitive (accessToken,
    // PRIVATE_TOKEN). Do not rely on word boundaries to recognize such keys.
    let sensitive = regex::Regex::new(r"(?i)(?:access[_-]?token|refresh[_-]?token|id[_-]?token|token|authorization|credential|secret|password|passwd|api[_-]?key|access[_-]?key|cookie|signature|signed|x-amz-|x-tos-|device[_-]?code|user[_-]?code|\b(?:pwd|bearer|basic)\b)").unwrap();
    let escaped = regex::Regex::new(r"(?i)%[0-9a-f]{2}|\\u[0-9a-f]{4}|\\x[0-9a-f]{2}").unwrap();
    let quoted_flag = regex::Regex::new(r#"["'](--[a-zA-Z0-9_-]+)["']"#).unwrap();
    let quoted = regex::Regex::new(r#""[^"\r\n]*"|'[^'\r\n]*'"#).unwrap();
    let prompt_value = regex::Regex::new(r"(?i)(?:--prompt\s*=|\bprompt\s*[:=]).*").unwrap();
    let opaque = regex::Regex::new(r"\b[A-Za-z0-9_+/=-]{24,}\b").unwrap();
    let identifiers = regex::Regex::new(r"\b\d{8,}\b").unwrap();
    let useful = regex::Regex::new(r"(?i)(?:\berror\b|\bfailed\b|\bfailure\b|\binvalid\b|\bunsupported\b|\brequired\b|\bcannot\b|\bunable\b|\bmust\b|\bnot found\b|\bnot exist\b|\bdenied\b|\btimeout\b|\bdeadline\b|错误|失败|无法|不支持|无效|不存在|超时|必须)").unwrap();
    let sanitized: Vec<String> = cleaned
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            if sensitive.is_match(line)
                || escaped.is_match(line)
                || line.chars().any(|c| c.is_control() && c != '\t')
                || line
                    .chars()
                    .any(|c| matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
                || line.contains('{')
                || line.contains('}')
                || line.contains('?')
                || line.contains('&')
            {
                return Some("[stderr 此行未安全披露]".into());
            }
            let line = prompt_value.replace_all(line, "prompt=[redacted]");
            let line = quoted_flag.replace_all(&line, "$1");
            let line = quoted.replace_all(&line, "[value]");
            let line = opaque.replace_all(&line, "[value]");
            let line = identifiers.replace_all(&line, "[id]");
            if !useful.is_match(&line) {
                Some("[stderr 此行未安全披露]".into())
            } else {
                Some(line.into_owned())
            }
        })
        .collect();
    if sanitized.is_empty() {
        return None;
    }
    let mut summary = sanitized.join(" | ");
    if redactions.iter().any(|value| summary.contains(value)) {
        return Some("[stderr 内容未安全披露]".into());
    }
    // This is a display bound over fully sanitized Unicode, never raw stderr.
    if summary.chars().count() > 1000 {
        summary = summary.chars().take(1000).collect();
        summary.push('…');
    }
    Some(summary)
}

fn output_error(output: &CliOutput, submitting: bool) -> ProviderError {
    output_error_with_redactions(output, submitting, &failure_redactions(None, &[], None))
}

fn output_error_for_invocation(
    output: &CliOutput,
    submitting: bool,
    config: &ProviderConfig,
    args: &[String],
    directory: Option<&Path>,
) -> ProviderError {
    output_error_with_redactions(
        output,
        submitting,
        &failure_redactions(Some(config), args, directory),
    )
}

fn output_error_with_redactions(
    output: &CliOutput,
    submitting: bool,
    redactions: &[String],
) -> ProviderError {
    let mut error = classified_output_error(output, submitting);
    if !output.success {
        if let Some(summary) = safe_failure_stderr(&output.stderr, redactions) {
            error.message.push_str(" stderr摘要：");
            error.message.push_str(&summary);
        }
    }
    error
}

fn classified_output_error(output: &CliOutput, submitting: bool) -> ProviderError {
    // Inspect both streams, but never copy raw OAuth/CLI content into public errors.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    let hint = [
        ("DNS", "no such host"),
        ("DNS", "name resolution"),
        ("CONNECTION", "dial tcp"),
        ("CONNECTION", "connection refused"),
        ("CONNECTION", "connection reset"),
        ("TLS", "tls"),
        ("CERTIFICATE", "x509"),
        ("CERTIFICATE", "certificate"),
        ("PROXY", "proxyconnect"),
        ("TIMEOUT", "timeout"),
        ("TIMEOUT", "timed out"),
        ("EOF", "unexpected eof"),
        ("CLI_ARGUMENT", "unknown command"),
        ("CLI_ARGUMENT", "unknown flag"),
        ("OS_PERMISSION", "access is denied"),
        ("OS_PERMISSION", "permission denied"),
        ("RESPONSE_FORMAT", "invalid character"),
        ("RESPONSE_FORMAT", "unmarshal"),
        ("HTTP_401", "http 401"),
        ("HTTP_401", "status code: 401"),
        ("HTTP_403", "http 403"),
        ("HTTP_429", "http 429"),
    ]
    .iter()
    .find(|(_, marker)| text.contains(marker))
    .map(|(hint, _)| *hint)
    .unwrap_or("UNCLASSIFIED");
    // Keep enough safe process evidence to diagnose the app's actual launch.
    // Raw stderr/stdout can contain OAuth secrets and signed media URLs.
    let diagnostic = format!(
        " [CLI退出码={}；类别={}；stdout={}B；stderr={}B；launch={}]",
        output
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "未知".into()),
        hint,
        output.stdout.len(),
        output.stderr.len(),
        output.launch
    );
    if text.contains("aigccomplianceconfirmationrequired") {
        return ProviderError::new(
            "WEB_CONFIRMATION_REQUIRED",
            format!("此模型要求先在即梦网页完成首次使用确认。{diagnostic}"),
        );
    }
    // SDK errors can mention login while reporting a transport failure. Such
    // text is not evidence that the stored identity or credential has expired.
    let timeout = [
        "timeout",
        "timed out",
        "deadline exceeded",
        "context deadline",
    ]
    .iter()
    .any(|marker| text.contains(marker));
    let transport = timeout
        || [
            "connection reset",
            "connection refused",
            "no such host",
            "name resolution",
            "unexpected eof",
            "tls handshake",
            "x509",
            "proxyconnect",
            "network is unreachable",
            "failed to connect",
            "dial tcp",
            "certificate verify failed",
        ]
        .iter()
        .any(|marker| text.contains(marker));
    if transport {
        return if submitting {
            ProviderError::uncertain(format!(
                "CLI 提交遇到网络错误且回执未确认；请核对原任务，不能自动重发。{diagnostic}"
            ))
        } else {
            ProviderError::new(
                if timeout {
                    "CLI_QUERY_TIMEOUT"
                } else {
                    "CLI_NETWORK"
                },
                format!(
                    "即梦 CLI 只读请求遇到网络错误；保留原任务，可稍后查询原ID重试。{diagnostic}"
                ),
            )
        };
    }
    if text.contains("unauthorized")
        || text.contains("unauthenticated")
        || text.contains("not authenticated")
        || text.contains("not logged")
        || text.contains("token expired")
        || text.contains("expired token")
        || text.contains("invalid token")
        || text.contains("login required")
        || text.contains("please login")
        || text.contains("please log in")
        || text.contains("please run dreamina login")
        || text.contains("please use dreamina login")
        || text.contains("need to login")
        || text.contains("login denied")
        || text.contains("authentication failed")
        || text.contains("http 401")
        || text.contains("status code: 401")
    {
        let mut error = ProviderError::auth();
        error.message.push_str(&diagnostic);
        return error;
    }
    if submitting {
        ProviderError::uncertain(format!(
            "CLI 未返回可确认的提交结果；请核对渠道记录。{diagnostic}"
        ))
    } else {
        ProviderError::new(
            "CLI_QUERY_FAILED",
            format!("即梦 CLI 只读命令未成功；保留原任务，可检查连接后重试。{diagnostic}"),
        )
    }
}

fn run(config: &ProviderConfig, args: &[String], submitting: bool) -> ProviderResult<Vec<u8>> {
    let output = execute(config, args, submitting)?;
    if !output.success {
        return Err(output_error_for_invocation(
            &output, submitting, config, args, None,
        ));
    }
    Ok(output.stdout)
}

fn run_in_directory(
    config: &ProviderConfig,
    args: &[String],
    directory: Option<&Path>,
) -> ProviderResult<Vec<u8>> {
    let output = execute_until_in_directory(config, args, true, None, directory)?;
    if !output.success {
        return Err(output_error_for_invocation(
            &output, true, config, args, directory,
        ));
    }
    Ok(output.stdout)
}

fn auth_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let ansi = regex::Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").unwrap();
    ansi.replace_all(&text, "").into_owned()
}

fn auth_json(bytes: &[u8]) -> ProviderResult<Value> {
    // Some CLI builds prefix their JSON with a human-readable line or ANSI escape.
    let cleaned = auth_text(bytes);
    if let Ok(value) = serde_json::from_str::<Value>(cleaned.trim()) {
        return Ok(value);
    }
    for (position, ch) in cleaned.char_indices().filter(|(_, ch)| *ch == '{') {
        let mut stream =
            serde_json::Deserializer::from_str(&cleaned[position..]).into_iter::<Value>();
        if let Some(Ok(value)) = stream.next() {
            if value.is_object() {
                return Ok(value);
            }
        }
        let _ = ch;
    }
    Err(ProviderError::new(
        "CLI_AUTH_JSON",
        "官方 CLI 授权响应无法识别；未保存授权材料。",
    ))
}

fn oauth_material_body(bytes: &[u8]) -> Value {
    if let Ok(body) = auth_json(bytes) {
        return body;
    }
    // Official SDKs can print label:value material instead of a JSON object.
    // This mirrors the source-verified SHUO label parser, restricted to OAuth fields.
    let text = auth_text(bytes);
    let mut fields = serde_json::Map::new();
    for key in [
        "verification_uri_complete",
        "verification_url",
        "verification_uri",
        "user_code",
        "userCode",
        "device_code",
        "deviceCode",
        "expires_in",
        "interval",
    ] {
        let pattern = format!(
            r#"(?i)(?:^|[\s,{{]){}\s*[:=]\s*['"]?([^'"\s,;}}]+)"#,
            regex::escape(key)
        );
        if let Some(value) = regex::Regex::new(&pattern)
            .unwrap()
            .captures(&text)
            .and_then(|captures| captures.get(1))
        {
            fields.insert(
                key.into(),
                if ["expires_in", "interval"].contains(&key) {
                    value
                        .as_str()
                        .parse::<u64>()
                        .ok()
                        .map(Value::from)
                        .unwrap_or(Value::Null)
                } else {
                    json!(value.as_str())
                },
            );
        }
    }
    Value::Object(fields)
}

fn account_snapshot(body: &Value) -> Value {
    let data = task_object(body);
    let identity = match data.get("user_id") {
        Some(Value::String(id))
            if !id.trim().is_empty()
                && id.trim() != "0"
                && id.trim().len() <= 256
                && !id.trim().chars().any(char::is_whitespace)
                && !id.chars().any(char::is_control) =>
        {
            Some(id.trim().to_string())
        }
        Some(Value::Number(id)) => id.as_u64().filter(|id| *id > 0).map(|id| id.to_string()),
        _ => None,
    };
    let credit = |value: Option<&Value>| -> Option<f64> {
        value
            .and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|x| x.parse::<f64>().ok()))
            })
            .filter(|x| x.is_finite() && *x >= 0.)
    };
    // The official CLI returns this explicit anonymous account envelope after
    // logout. An absent ID alone remains unknown; a real account with zero
    // credits is still authenticated.
    let logged_out = identity.is_none()
        && matches!(data.get("user_id"), Some(Value::Number(id)) if id.as_u64() == Some(0))
        && data.get("user_name").and_then(Value::as_str) == Some("")
        && credit(data.get("total_credit")) == Some(0.);
    let mut result = json!({
        "status":if identity.is_some(){"authenticated"}else if logged_out{"logged_out"}else{"unknown"},
        "ready":identity.is_some(),
        "identityVerified":identity.is_some(),
        "message":if identity.is_some(){"即梦已登录，账户身份与积分已核对。"}else if logged_out{"即梦 CLI 尚未登录或授权已失效；请到渠道与设置完成官方 OAuth 登录，队列已暂停。"}else{"CLI 返回了积分信息，但没有可验证的账户身份；暂不能提交或恢复生成。"},
        "checkedAt":now(),
        "credits":{"total":credit(data.get("total_credit")),"member":credit(data.get("vip_credit")),"gift":credit(data.get("gift_credit")),"purchase":credit(data.get("purchase_credit"))}
    });
    if let Some(identity) = identity {
        result["accountKey"] = json!(format!(
            "dreamina:official:user:{}",
            hex_hash(identity.as_bytes())
        ));
    }
    if let Some(vip) = data
        .get("vip_level")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control))
    {
        result["vipLevel"] = json!(vip);
    }
    result
}

pub(super) fn account(config: &ProviderConfig) -> ProviderResult<Value> {
    let output = execute(config, &["user_credit".into()], false)?;
    if !output.success {
        return Err(output_error(&output, false));
    }
    Ok(account_snapshot(&auth_json(&output.stdout)?))
}

fn readiness_log_write_restricted(output:&CliOutput)->bool {
    let message=String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    message.contains("initialize logs failed") && message.contains("initialize file logger")
        && (message.contains("operation not permitted") || message.contains("permission denied"))
}
fn legacy_readiness_help_supported(command: Option<&str>, bytes: &[u8]) -> bool {
    let Ok(help) = std::str::from_utf8(bytes) else { return false; };
    if help.len() > 1024 * 1024 || !help.contains("Usage:") || serde_json::from_str::<Value>(help).is_ok()
        || help.to_ascii_lowercase().contains("dreamina-canvas") { return false; }
    let usage = help.lines().any(|line| {
        let mut parts = line.split_whitespace();
        if parts.next() != Some("dreamina") { return false; }
        match (command, parts.next()) {
            (Some(expected), Some(actual)) => actual == expected,
            (None, Some("[command]" | "[flags]")) => true,
            _ => false,
        }
    });
    if !usage { return false; }
    let flags: &[&str] = match command {
        None => return ["login", "user_credit", "query_result", "text2image", "image2image", "text2video", "multimodal2video"]
            .iter().all(|name| has_token(help, name))
            && (has_token(help, "--version") || has_token(help, "version")),
        Some("text2image" | "image2image") => &["--model_version", "--ratio", "--resolution_type", "--prompt"],
        Some("text2video") => &["--model_version", "--ratio", "--video_resolution", "--duration", "--prompt"],
        Some("multimodal2video") => &["--model_version", "--ratio", "--video_resolution", "--duration", "--image", "--video", "--audio"],
        _ => return false,
    };
    flags.iter().all(|flag| has_token(help, flag))
}
fn legacy_protocol_unsupported() -> ProviderError {
    ProviderError::new("DREAMINA_PROTOCOL_UNSUPPORTED", "此文件未通过已适配的旧版即梦生成 CLI 版本与命令协议核验。")
}
/// No DreaminaSessions polling, account mutation, generation or job-resume hooks.
pub(super) fn readiness(config: &ProviderConfig) -> Value {
    let mut result=crate::readiness::empty("dreamina");
    let identity=match cli_identity(config) {
        Ok(identity)=>identity,
        Err(error)=>{result["installation"]["state"]=json!("missing");result["messages"]=json!([error.message]);return result;}
    };
    result["installation"]=json!({"state":"ready","version":null,"executable":identity["path"]});
    result["details"]=json!({"binarySha256":identity["sha256"],"freeImageEligibility":"unverified","accountIdentityNotExported":true});
    let mut verified_legacy_version = None;
    let inspected=capabilities_with_runner(config,Instant::now,|config,args,deadline| {
        let read = |arguments: &[String]| -> ProviderResult<Vec<u8>> {
            let output=execute_until(config,arguments,false,Some(deadline))?;
            if readiness_log_write_restricted(&output) {return Err(ProviderError::new("CLI_LOCAL_LOG_WRITE_RESTRICTED","官方 CLI 本地日志初始化受限；未继续账户检查。"));}
            if !output.success {return Err(output_error(&output,false));}
            Ok(output.stdout)
        };
        let bytes=read(args)?;
        if args == ["version"] {
            verified_legacy_version = crate::readiness_canvas::legacy_version_label(&bytes);
            if verified_legacy_version.is_none() {
                return Err(legacy_protocol_unsupported());
            }
            if !legacy_readiness_help_supported(None, &read(&["--help".into()])?) { return Err(legacy_protocol_unsupported()); }
        } else if args.len() != 2 || args[1] != "-h" || !legacy_readiness_help_supported(Some(&args[0]), &bytes) {
            return Err(legacy_protocol_unsupported());
        }
        Ok(bytes)
    });
    result["installation"]["version"] = json!(verified_legacy_version);
    let mut messages=vec!["只读取官方 CLI 版本/帮助目录与账户积分；没有图片或视频生成。".to_string(),"积分余额和模型目录不能证明当前账户免费资格；免费出图尚未核验。".to_string()];
    match inspected {
        Ok(caps)=>{
            result["protocol"]=json!({"state":"ready","name":"Official Dreamina CLI help + user_credit"});
            let models:Vec<_>=caps.models.iter().map(|m|json!({"id":m.id,"name":m.id,"kind":m.kind})).collect();
            result["catalog"]=json!({"state":if models.is_empty(){"unavailable"}else{"available"},"models":models});
        },
        Err(error)=>{
            result["protocol"]["state"]=json!("failed");result["catalog"]["state"]=json!("unavailable");
            if error.code=="DREAMINA_PROTOCOL_UNSUPPORTED" {
                result["protocol"]=json!({"state":"unsupported","name":"Legacy Dreamina generation CLI"});
                result["details"]["detectedProtocol"]=json!("unsupported_for_legacy");
                messages.push("所选程序未通过旧版即梦生成 CLI 的版本与命令协议核验，已停止账户检查；dreamina-canvas 请使用独立 Canvas CLI 路径与检查按钮。旧任务和账户记录保持原样。".into());
                result["messages"]=json!(messages);return result;
            }
            if error.code=="CLI_LOCAL_LOG_WRITE_RESTRICTED" {
                result["details"]["localEnvironmentIssue"]=json!(error.code);
                messages.push("官方 CLI 本地日志写入受限，已停止后续账户检查；这不代表账号失效，未尝试改目录或安全权限。".into());
                result["messages"]=json!(messages);return result;
            }
            messages.push(format!("CLI 帮助/目录检查失败（{}），未推断可生成模型。",error.code));
        }
    }
    if verified_legacy_version.is_none() {
        messages.push("未能核验旧版 CLI 协议，未继续 user_credit 账户检查；没有发起登录或生成。".into());
        result["messages"] = json!(messages); return result;
    }
    let account_result=(|| {
        let output=execute(config,&["user_credit".into()],false)?;
        if readiness_log_write_restricted(&output) {return Err(ProviderError::new("CLI_LOCAL_LOG_WRITE_RESTRICTED","官方 CLI 本地日志初始化受限。"));}
        if !output.success {return Err(output_error(&output,false));}
        Ok(account_snapshot(&auth_json(&output.stdout)?))
    })();
    match account_result {
        Ok(account)=>{
            result["authentication"]=json!({"state":if account["identityVerified"]==true{"authenticated"}else if account["status"]=="logged_out"{"not_logged_in"}else{"unknown"},"identityVerified":account["identityVerified"]==true});
            result["details"]["credits"]=account["credits"].clone();
            result["details"]["accountQueryCompleted"]=json!(true);
            messages.push("user_credit 已返回；未进行独立网络探测或真实模型请求。".into());
        },
        Err(error)=>{
            result["authentication"]["state"]=json!(if error.auth_required{"not_logged_in"}else{"unknown"});
            result["details"]["accountQueryCompleted"]=json!(false);
            if error.code=="CLI_LOCAL_LOG_WRITE_RESTRICTED" {result["details"]["localEnvironmentIssue"]=json!(error.code);}

            messages.push(format!("账户只读检查未完成（{}）；检查不会发起登录、退出或重试生成。",error.code));
        }
    }
    result["messages"]=json!(messages);result
}

/// Shares owned pipes, deadlines, shutdown accounting and child cleanup with legacy readiness.
/// Static Canvas read commands only; no caller-controlled flags, login or node mutations.
pub(super) fn canvas_readiness_command(
    config: &ProviderConfig,
    command: crate::readiness_canvas::ReadCommand,
    deadline: Instant,
) -> (ProviderResult<crate::readiness_canvas::ProcessOutput>, Value) {
    let args = command.args().iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let observation=CliObservation::new();
    let result=execute_until_in_directory_observed(config,&args,false,Some(deadline),None,Some(observation.clone())).map(|output| {
    // Keep both bounded streams in memory until the Canvas-only classifier has
    // checked failure envelopes. Do not route through the legacy text classifier.
    crate::readiness_canvas::ProcessOutput {
        success: output.success, exit_code: output.exit_code, signal: output.signal,
        local_log_write_restricted: readiness_log_write_restricted(&output),
        stdout: output.stdout, stderr: output.stderr,
    }
    });
    (result,observation.snapshot())
}

fn login_material(body: &Value) -> ProviderResult<Value> {
    let data = task_object(body);
    let field = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|key| {
            data.get(*key)
                .and_then(Value::as_str)
                .filter(|s| {
                    !s.trim().is_empty() && s.len() <= 8192 && !s.chars().any(char::is_control)
                })
                .map(|s| s.trim().to_string())
        })
    };
    let device = field(&["device_code", "deviceCode"]).ok_or_else(|| {
        ProviderError::new(
            "CLI_AUTH_MATERIAL",
            "官方 CLI 没有返回设备授权码；请刷新登录状态。",
        )
    })?;
    let uri = field(&[
        "verification_uri_complete",
        "verification_url",
        "verification_uri",
    ])
    .ok_or_else(|| ProviderError::new("CLI_AUTH_MATERIAL", "官方 CLI 没有返回授权网页地址。"))?;
    let mut result = json!({"device_code":device,"verification_uri":uri});
    if let Some(code) = field(&["user_code", "userCode"]) {
        result["user_code"] = json!(code);
    }
    for key in ["expires_in", "interval"] {
        if let Some(value) = data
            .get(key)
            .and_then(Value::as_u64)
            .filter(|n| *n > 0 && *n <= 86400)
        {
            result[key] = json!(value);
        }
    }
    // This allowlist is returned only to the core's in-memory OAuth flow. Never store raw JSON.
    Ok(result)
}

pub(super) fn begin_login(config: &ProviderConfig) -> ProviderResult<Value> {
    begin_login_with(
        || account(config),
        |fresh| execute(config, &[if fresh { "relogin" } else { "login" }.into(), "--headless".into()], false),
    )
}

fn begin_login_with(
    mut read_account: impl FnMut() -> ProviderResult<Value>,
    mut start_login: impl FnMut(bool) -> ProviderResult<CliOutput>,
) -> ProviderResult<Value> {
    // This entry point is reached only by an explicit OAuth button click.
    // Like SHUO's relogin action, request a fresh official challenge directly:
    // a failed user_credit probe or a stale locally-valid token must not block
    // the very action the user needs to repair their connection. Startup and
    // queue polling use account(), never this credential-changing command.
    let output = start_login(true)?;
    if !output.success {
        return Err(output_error(&output, false));
    }
    let body = oauth_material_body(&output.stdout);
    if task_object(&body).get("device_code").is_none()
        && task_object(&body).get("deviceCode").is_none()
    {
        // Login can reuse an existing session; only user_credit proves which account it is.
        let account = read_account()?;
        if account["ready"] == true && account["identityVerified"] == true {
            return Ok(account);
        }
        return Err(ProviderError::new(
            "CLI_AUTH_CHALLENGE_MISSING",
            "即梦 CLI 未返回有效授权链接，请点击 OAuth 登录重试。",
        ));
    }
    login_material(&body)
}

fn login_result(output: &CliOutput) -> ProviderResult<Option<Value>> {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    if text.contains("access_denied")
        || text.contains("authorization denied")
        || text.contains("authsdk: login denied")
    {
        return Err(ProviderError::new(
            "CLI_AUTH_DENIED",
            "即梦授权被拒绝，尚未连接账户。",
        ));
    }
    if text.contains("expired_token") || text.contains("device_code expired") {
        return Err(ProviderError::new(
            "CLI_AUTH_EXPIRED",
            "即梦授权已过期，请重新发起登录。",
        ));
    }
    if text.contains("authorization_pending") || text.contains("slow_down") {
        return Ok(Some(
            json!({"status":"pending","ready":false,"identityVerified":false,"slowDown":text.contains("slow_down"),"message":"等待在即梦授权网页完成登录确认。","checkedAt":now()}),
        ));
    }
    if !output.success {
        return Err(output_error(output, false));
    }
    // A successful process exit is only permission to query identity, never proof of login.
    Ok(None)
}

pub(super) fn check_login(config: &ProviderConfig, device_code: &str) -> ProviderResult<Value> {
    if device_code.is_empty()
        || device_code.len() > 4096
        || device_code.chars().any(char::is_whitespace)
        || device_code.chars().any(char::is_control)
    {
        return Err(ProviderError::new(
            "CLI_AUTH_DEVICE",
            "设备授权码无效；请重新发起登录。",
        ));
    }
    let output = execute(
        config,
        &[
            "login".into(),
            "checklogin".into(),
            format!("--device_code={device_code}"),
            // Return control to the desktop between bounded authorization checks.
            "--poll=1".into(),
        ],
        false,
    )?;
    if let Some(pending) = login_result(&output)? {
        return Ok(pending);
    }
    account(config)
}

pub(super) fn logout(config: &ProviderConfig) -> ProviderResult<()> {
    let output = execute(config, &["logout".into()], false)?;
    if !output.success {
        return Err(output_error(&output, false));
    }
    Ok(())
}
fn has_token(text: &str, token: &str) -> bool {
    let expression = format!(
        r"(^|[^a-zA-Z0-9_.]){}([^a-zA-Z0-9_.]|$)",
        regex::escape(token)
    );
    regex::Regex::new(&expression).unwrap().is_match(text)
}

fn declared_resolutions(help: &str, model: &str) -> Vec<String> {
    let expected: &[&str] = match model {
        "3.0" | "3.1" => &["1k", "2k"],
        "5.0Pro" => &["1.5k", "2k", "4k"],
        _ => &["2k", "4k"],
    };
    // Match a model-specific declaration rather than a resolution occurring elsewhere.
    let declaration = help.lines().find(|line| {
        let Some((names, settings)) = line.split_once("->") else {
            return false;
        };
        has_token(names, model) && settings.contains("resolution_type")
    });
    expected
        .iter()
        .filter(|resolution| declaration.is_some_and(|line| has_token(line, resolution)))
        .map(|x| (*x).into())
        .collect()
}

fn custom_1080_verified(help: &str) -> bool {
    has_token(help, "--width")
        && has_token(help, "--height")
        && help.contains("mutually exclusive with --ratio")
        && help.contains("2k: each side 768-3072; total pixels <= 4194304")
}

fn video_declaration<'a>(help: &'a str, model: &str, field: &str) -> Option<&'a str> {
    // The help lists model-specific rules first, then a rule for all remaining models.
    // Reading the whole help would accidentally lend another model its 4K or 1080P.
    let declarations: Vec<_> = help
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .filter_map(|line| line.split_once("->"))
        .filter(|(_, settings)| settings.contains(field))
        .collect();
    declarations
        .iter()
        .find(|(names, _)| has_token(names, model))
        .or_else(|| {
            declarations.iter().find(|(names, _)| {
                names.trim() == "all other models"
                    || (model.starts_with("seedance2.0")
                        && names.trim() == "all other seedance2.0 models")
            })
        })
        .map(|(_, settings)| *settings)
}

fn video_resolutions(help: &str, model: &str) -> Vec<String> {
    let Some(declaration) = video_declaration(help, model, "video_resolution") else {
        return vec![];
    };
    let field = declaration
        .split_once("video_resolution")
        .map(|(_, settings)| settings.split(';').next().unwrap_or(""))
        .unwrap_or("");
    ["480p", "720p", "1080p", "4k"]
        .iter()
        .filter(|resolution| has_token(field, resolution))
        .map(|resolution| (*resolution).into())
        .collect()
}

fn cli_identity(config: &ProviderConfig) -> ProviderResult<Value> {
    let path = cli_path(config)?;
    let mut file = fs::File::open(path)
        .map_err(|_| ProviderError::new("CLI_READ", "无法核对官方 CLI 文件。"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut bytes = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| ProviderError::new("CLI_READ", "官方 CLI 文件读取失败。"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok(json!({"path":path,"sha256":format!("{:x}",hash.finalize()),"bytes":bytes}))
}

fn image_1080_dimensions(ratio: &str) -> ProviderResult<(u32, u32)> {
    // The 1080p preset means a short edge of 1080 px, not an upstream enum.
    let dimensions = match ratio {
        "21:9" => (2520, 1080),
        "16:9" => (1920, 1080),
        "3:2" => (1620, 1080),
        "4:3" => (1440, 1080),
        "1:1" => (1080, 1080),
        "3:4" => (1080, 1440),
        "2:3" => (1080, 1620),
        "9:16" => (1080, 1920),
        _ => {
            return Err(ProviderError::new(
                "OUTPUT_CAPACITY",
                "1080P 图片预设只支持当前官方 CLI 的常用比例。",
            ))
        }
    };
    let (width, height) = dimensions;
    if !(768..=3072).contains(&width)
        || !(768..=3072).contains(&height)
        || u64::from(width) * u64::from(height) > 4_194_304
    {
        return Err(ProviderError::new(
            "OUTPUT_CAPACITY",
            "1080P 图片预设超出已验证像素容量。",
        ));
    }
    Ok(dimensions)
}

const CAPABILITY_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CAPABILITY_CACHE_MAX_KEYS: usize = 16;

#[derive(Clone, Eq, PartialEq, Hash)]
struct CapabilityCacheKey {
    path: PathBuf,
    sha256: String,
    bytes: u64,
    modified: Option<SystemTime>,
}
struct CachedCapabilities {
    value: Capabilities,
    checked: Instant,
}
type CapabilityCacheSlot = Arc<Mutex<Option<CachedCapabilities>>>;
#[derive(Default)]
struct CapabilityCache {
    entries: Mutex<HashMap<CapabilityCacheKey, CapabilityCacheSlot>>,
}
static CAPABILITY_CACHE: OnceLock<CapabilityCache> = OnceLock::new();

fn capability_error(mut error: ProviderError, step: &str) -> ProviderError {
    // Project task errors currently persist this message. Keep the typed code
    // and the fixed local probe name, without copying CLI output or paths.
    error.message = format!("[{}; capabilityProbe={step}] {}", error.code, error.message);
    error
}

fn capability_cache_key(
    config: &ProviderConfig,
    identity: &Value,
) -> ProviderResult<CapabilityCacheKey> {
    let path = fs::canonicalize(cli_path(config)?)
        .map_err(|_| ProviderError::new("CLI_PATH", "无法核对官方 CLI 的实际文件路径。"))?;
    let metadata = fs::metadata(&path)
        .map_err(|_| ProviderError::new("CLI_READ", "无法核对官方 CLI 文件身份。"))?;
    Ok(CapabilityCacheKey {
        path,
        sha256: identity["sha256"]
            .as_str()
            .ok_or_else(|| ProviderError::new("CLI_IDENTITY", "官方 CLI 文件身份无效。"))?
            .to_owned(),
        bytes: identity["bytes"]
            .as_u64()
            .ok_or_else(|| ProviderError::new("CLI_IDENTITY", "官方 CLI 文件身份无效。"))?,
        modified: metadata.modified().ok(),
    })
}

impl CapabilityCache {
    fn get_or_inspect(
        &self,
        config: &ProviderConfig,
        mut clock: impl FnMut() -> Instant,
        inspect: impl FnOnce(&ProviderConfig) -> ProviderResult<Capabilities>,
    ) -> ProviderResult<Capabilities> {
        let identity =
            cli_identity(config).map_err(|error| capability_error(error, "cli_identity"))?;
        let key = capability_cache_key(config, &identity)
            .map_err(|error| capability_error(error, "cli_identity"))?;
        let slot = {
            let mut entries = self.entries.lock().map_err(|_| {
                capability_error(
                    ProviderError::new("CLI_CAPABILITY_CACHE", "官方 CLI 能力缓存暂不可用。"),
                    "cache",
                )
            })?;
            if !entries.contains_key(&key) && entries.len() >= CAPABILITY_CACHE_MAX_KEYS {
                // Eviction only causes a new bounded read; retained in-flight
                // slots remain valid and no account/receipt is held here.
                entries.clear();
            }
            entries.entry(key.clone()).or_default().clone()
        };
        // Single-flight for this exact executable. The process-wide map lock
        // is released before the existing bounded read-only inspection.
        let mut cached = slot.lock().map_err(|_| {
            capability_error(
                ProviderError::new("CLI_CAPABILITY_CACHE", "官方 CLI 能力缓存暂不可用。"),
                "cache",
            )
        })?;
        if let Some(entry) = cached.as_ref() {
            if clock()
                .checked_duration_since(entry.checked)
                .is_some_and(|age| age < CAPABILITY_CACHE_TTL)
            {
                // checked_at deliberately remains the actual help-read time.
                // Authentication is never cached by this helper.
                return Ok(entry.value.clone());
            }
        }
        *cached = None;
        let value = inspect(config)?;
        let after =
            cli_identity(config).map_err(|error| capability_error(error, "cli_identity"))?;
        if after != identity
            || capability_cache_key(config, &after)
                .map_err(|error| capability_error(error, "cli_identity"))?
                != key
        {
            return Err(capability_error(
                ProviderError::new(
                    "CLI_CHANGED",
                    "官方 CLI 在能力检查期间变化，请重新核实安装文件。",
                ),
                "cli_identity",
            ));
        }
        *cached = Some(CachedCapabilities {
            value: value.clone(),
            checked: clock(),
        });
        Ok(value)
    }
}

pub(super) fn capabilities(config: &ProviderConfig) -> ProviderResult<Capabilities> {
    CAPABILITY_CACHE
        .get_or_init(CapabilityCache::default)
        .get_or_inspect(config, Instant::now, |config| {
            capabilities_with_runner(config, Instant::now, |config, args, deadline| {
                let output = execute_until(config, args, false, Some(deadline))?;
                if !output.success {
                    return Err(output_error(&output, false));
                }
                Ok(output.stdout)
            })
        })
}

fn capabilities_with_runner(
    config: &ProviderConfig,
    mut clock: impl FnMut() -> Instant,
    mut runner: impl FnMut(&ProviderConfig, &[String], Instant) -> ProviderResult<Vec<u8>>,
) -> ProviderResult<Capabilities> {
    let overall_deadline = clock() + Duration::from_secs(30);
    let command_budget = Duration::from_secs(config.timeout_seconds.clamp(1, 15));
    let mut inspect = |args: &[String]| -> ProviderResult<Vec<u8>> {
        let step = args.first().map(String::as_str).unwrap_or("help");
        let started = clock();
        if started >= overall_deadline {
            return Err(capability_error(capability_timeout(), step));
        }
        let deadline = (started + command_budget).min(overall_deadline);
        let bytes =
            runner(config, args, deadline).map_err(|error| capability_error(error, step))?;
        if clock() >= deadline {
            return Err(capability_error(capability_timeout(), step));
        }
        Ok(bytes)
    };
    let version = inspect(&["version".into()])?;
    let mut inspect_help = |command: &str| -> ProviderResult<String> {
        let bytes = inspect(&[command.into(), "-h".into()])?;
        String::from_utf8(bytes).map_err(|_| {
            capability_error(
                ProviderError::new("CLI_HELP", "CLI 帮助编码无效。"),
                command,
            )
        })
    };
    let video_help = inspect_help("multimodal2video")?;
    let text_video_help = inspect_help("text2video")?;
    let image_help = inspect_help("text2image")?;
    let edit_help = inspect_help("image2image")?;
    capabilities_from_help(
        &version,
        &video_help,
        &text_video_help,
        &image_help,
        &edit_help,
    )
    .map_err(|error| capability_error(error, "help_contract"))
}

fn capabilities_from_help(
    version: &[u8],
    video_help: &str,
    text_video_help: &str,
    image_help: &str,
    edit_help: &str,
) -> ProviderResult<Capabilities> {
    let mut models = Vec::new();
    let ratios: Vec<String> = ["1:1", "3:4", "16:9", "4:3", "9:16", "21:9"]
        .iter()
        .filter(|r| video_help.contains(**r) && text_video_help.contains(**r))
        .map(|r| r.to_string())
        .collect();
    for model in [
        "seedance2.0",
        "seedance2.0fast",
        "seedance2.0_vip",
        "seedance2.0fast_vip",
        "seedance2.0mini",
        "seedance2.5",
    ] {
        if !has_token(&video_help, model) || !has_token(&text_video_help, model) {
            continue;
        }
        let v25 = model == "seedance2.5";
        // These mappings are enabled only when the current CLI explicitly advertises them.
        let counts_verified = if v25 {
            video_help.contains("image<=30")
                && video_help.contains("video<=10")
                && video_help.contains("audio<=10")
                && video_help.contains("total inputs<=50")
                && video_help.contains("4-30")
        } else {
            video_help.contains("image<=9")
                && video_help.contains("video<=3")
                && video_help.contains("audio<=3")
                && video_help.contains("total inputs<=12")
                && video_help.contains("4-15")
        };
        let text_resolutions = video_resolutions(text_video_help, model);
        let resolutions: Vec<_> = video_resolutions(video_help, model)
            .into_iter()
            .filter(|resolution| text_resolutions.contains(resolution))
            .collect();
        let resolution_mode = if resolutions.len() == 1 {
            "fixed"
        } else {
            "choice"
        };
        models.push(ModelCapability{id:model.into(),kind:"video".into(),adapted:counts_verified && !resolutions.is_empty() && !ratios.is_empty(),min_duration:Some(4),max_duration:Some(if v25{30}else{15}),allowed_durations:vec![],ratios:ratios.clone(),resolutions,images:if v25{30}else{9},videos:if v25{10}else{3},audios:if v25{10}else{3},total_references:Some(if v25{50}else{12}),required_images:0,price:None,billing:None,currency:None,video_price_multiplier:None,raw:json!({"verifiedFrom":"CLI help","resolutionMode":resolution_mode,"referenceDurationMin":2,"referenceDurationMax":if v25{30}else{15}})});
    }
    let image_ratios: Vec<String> = ["21:9", "16:9", "3:2", "4:3", "1:1", "3:4", "2:3", "9:16"]
        .iter()
        .filter(|r| image_help.contains(**r))
        .map(|r| r.to_string())
        .collect();
    for model in [
        "3.0", "3.1", "4.0", "4.1", "4.5", "4.6", "4.7", "5.0", "5.0Pro",
    ] {
        if !has_token(&image_help, model) {
            continue;
        }
        let edit_supported = has_token(edit_help, model);
        let mut resolutions = declared_resolutions(image_help, model);
        if edit_supported {
            let editing = declared_resolutions(edit_help, model);
            resolutions.retain(|r| editing.contains(r));
        }
        let custom_1080 = resolutions.iter().any(|r| r == "2k")
            && custom_1080_verified(image_help)
            && (!edit_supported || custom_1080_verified(edit_help));
        if custom_1080 {
            resolutions.insert(0, "1080p".into());
        }
        models.push(ModelCapability {
            id: model.into(),
            kind: "image".into(),
            adapted: !resolutions.is_empty(),
            min_duration: None,
            max_duration: None,
            allowed_durations: vec![],
            ratios: image_ratios.iter().filter(|ratio| !edit_supported || edit_help.contains(ratio.as_str())).cloned().collect(),
            resolutions,
            images: if edit_supported { 10 } else { 0 },
            videos: 0,
            audios: 0,
            total_references: Some(10),
            required_images: 0,
            price: None,
            billing: None,
            currency: None,
            video_price_multiplier: None,
            raw: json!({"verifiedFrom":"CLI help","presetMap":if custom_1080 {json!({"1080p":{"customPixelPreset":true,"shortEdge":1080,"resolutionType":"2k","dimensionMode":"width_height","ratioArgument":false,"minimumSide":768,"maximumSide":3072,"maximumPixels":4194304}})}else{json!({})}}),
        });
    }
    let material = format!(
        "dreamina-adapter/preflight-v2\n{}\n{video_help}\n{text_video_help}\n{image_help}\n{edit_help}",
        String::from_utf8_lossy(version)
    );
    Ok(Capabilities {
        provider: "dreamina".into(),
        version: hex_hash(material.as_bytes()),
        source: "cli_help".into(),
        checked_at: now(),
        models,
        can_cancel: false,
        can_reconcile_unknown: false,
        raw: json!({"adapterContract":"dreamina-adapter/preflight-v2","build":serde_json::from_slice::<Value>(version).unwrap_or_else(|_|json!({"unparsed":true})),"help":{"multimodal2video":video_help,"text2video":text_video_help,"text2image":image_help,"image2image":edit_help}}),
    })
}

pub(super) fn prepare(
    config: &ProviderConfig,
    request: &GenerationRequest,
) -> ProviderResult<PreparedRequest> {
    prepare_with_capability_resolver(config, request, capabilities)
}

fn prepare_with_capability_resolver(
    config: &ProviderConfig,
    request: &GenerationRequest,
    resolver: impl FnOnce(&ProviderConfig) -> ProviderResult<Capabilities>,
) -> ProviderResult<PreparedRequest> {
    let identity = cli_identity(config).map_err(|error| capability_error(error, "cli_identity"))?;
    let caps = resolver(config)?;
    if cli_identity(config).map_err(|error| capability_error(error, "cli_identity"))? != identity {
        return Err(ProviderError::new(
            "CLI_CHANGED",
            "官方 CLI 在预检期间变化，请重新准备关联单元。",
        ));
    }
    let mut prepared = prepare_with_capabilities(request, &caps)?;
    prepared.wire["cliIdentity"] = identity;
    Ok(prepared)
}

fn validate_cli_options(request: &GenerationRequest) -> ProviderResult<()> {
    if !request.options.as_object().is_some_and(|m| {
        m.iter().all(|(key, value)| {
            request.kind == "image" && key == "count" && value.as_u64() == Some(1)
        })
    }) {
        return Err(ProviderError::new(
            "UNSUPPORTED_OPTION",
            "官方 CLI 当前只使用已验证的基础参数，不会忽略额外生成设置。",
        ));
    }
    if request.references.iter().any(|r| {
        ["first_frame", "start_frame", "last_frame", "end_frame"].contains(&r.usage.as_str())
    }) {
        return Err(ProviderError::new(
            "FRAME_MODE_NOT_ADAPTED",
            "当前官方 CLI 适配使用全能参考，首尾帧用途需要单独适配后使用。",
        ));
    }
    Ok(())
}

fn prepare_with_capabilities(
    request: &GenerationRequest,
    caps: &Capabilities,
) -> ProviderResult<PreparedRequest> {
    validate_cli_options(request)?;
    let model = caps
        .models
        .iter()
        .find(|m| m.id == request.model && m.kind == request.kind)
        .ok_or_else(|| {
            ProviderError::new("MODEL_UNAVAILABLE", "当前官方 CLI 没有明确支持此模型。")
        })?;
    check_model(model, request)?;
    let refs = sorted_references(request);
    let has_refs = !refs.is_empty();
    let command = match (request.kind.as_str(), has_refs) {
        ("image", true) => "image2image",
        ("image", false) => "text2image",
        ("video", true) => "multimodal2video",
        ("video", false) => "text2video",
        _ => unreachable!(),
    };
    let mut args = vec![
        command.to_string(),
        format!("--model_version={}", request.model),
        format!("--prompt={}", request.prompt),
        "--poll=0".into(),
    ];
    if request.kind == "video" {
        args.push(format!("--ratio={}", request.ratio));
        if has_refs
            && request.model != "seedance2.5"
            && !refs.iter().any(|r| r.kind == "image" || r.kind == "video")
        {
            return Err(ProviderError::new(
                "REFERENCE_REQUIRED",
                "此模型不能只使用音频参考。",
            ));
        }
        args.push(format!("--duration={}", request.duration.unwrap()));
        args.push(format!(
            "--video_resolution={}",
            request.resolution.to_ascii_lowercase()
        ));
        let max_duration = if request.model == "seedance2.5" {
            30.
        } else {
            15.
        };
        for kind in ["video", "audio"] {
            let items: Vec<_> = refs.iter().filter(|r| r.kind == kind).collect();
            let mut total = 0.;
            for item in items {
                let duration = item.duration_seconds.ok_or_else(|| {
                    ProviderError::new("MEDIA_DURATION_REQUIRED", "视频及音频参考需要已验证时长。")
                })?;
                if !duration.is_finite() || duration < 2. || duration > max_duration {
                    return Err(ProviderError::new(
                        "MEDIA_DURATION",
                        "参考音视频超出官方 CLI 时长能力。",
                    ));
                }
                total += duration;
            }
            if total > max_duration {
                return Err(ProviderError::new(
                    "MEDIA_DURATION",
                    "同类参考音视频总时长超出模型能力。",
                ));
            }
        }
        for r in &refs {
            args.push(format!(
                "--{}={}",
                r.kind,
                r.path
                    .to_str()
                    .ok_or_else(|| ProviderError::new("MEDIA_PATH", "素材路径无法编码。"))?
            ));
        }
    } else {
        let resolution = if request.resolution.eq_ignore_ascii_case("1080p") {
            let (width, height) = image_1080_dimensions(&request.ratio)?;
            args.push(format!("--width={width}"));
            args.push(format!("--height={height}"));
            "2k".into()
        } else {
            args.push(format!("--ratio={}", request.ratio));
            request.resolution.to_ascii_lowercase()
        };
        args.push(format!("--resolution_type={}", resolution));
        args.push("--generate_num=1".into());
        if has_refs {
            let paths: ProviderResult<Vec<_>> =
                refs.iter()
                    .map(|r| {
                        let s = r.path.to_str().ok_or_else(|| {
                            ProviderError::new("MEDIA_PATH", "素材路径无法编码。")
                        })?;
                        if s.contains([',', '"', '\n', '\r']) {
                            return Err(ProviderError::new(
                                "MEDIA_PATH",
                                "官方图片 CLI 不支持路径中的逗号、引号或换行。",
                            ));
                        }
                        Ok(s)
                    })
                    .collect();
            args.push(format!("--images={}", paths?.join(",")));
        }
    }
    let mut frozen = request.clone();
    frozen.references = refs;
    let mut wire = json!({"args":args,"uploadMode":"cli_internal","uploadReceiptsExposed":false,"preflight":{"capabilityVersion":caps.version,"model":model}});
    if has_refs {
        wire["localMediaTransport"] = json!(LOCAL_MEDIA_TRANSPORT);
    }
    Ok(PreparedRequest {
        provider: "dreamina".into(),
        request: frozen,
        capability_version: caps.version.clone(),
        upload_receipts: vec![],
        wire,
        estimated_cost: None,
        prepared_at: now(),
    })
}

fn parse_output(bytes: &[u8], submitting: bool) -> ProviderResult<Value> {
    serde_json::from_slice::<Value>(bytes)
        .map(wire::sanitize)
        .map_err(|_| {
            if submitting {
                ProviderError::uncertain("官方 CLI 的提交回执无法解析；不能自动重发。")
            } else {
                ProviderError::new("CLI_INVALID_JSON", "官方 CLI 查询响应不是可识别 JSON。")
            }
        })
}
fn task_object(value: &Value) -> &Value {
    value.get("data").filter(|v| v.is_object()).unwrap_or(value)
}
fn result_urls(value: &Value, expected_kind: Option<&str>) -> Vec<String> {
    fn add(value: &Value, out: &mut Vec<String>) -> bool {
        let Some(value) = value.as_str() else {
            return false;
        };
        let valid = !value.chars().any(|c| c.is_control() || c.is_whitespace())
            && reqwest::Url::parse(value).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            });
        if valid && !out.iter().any(|url| url == value) {
            // Keep signed result URLs byte-for-byte, including every query parameter.
            out.push(value.into());
        }
        valid
    }
    fn collect(
        v: &Value,
        out: &mut Vec<String>,
        depth: u32,
        record: bool,
        kind: Option<&str>,
        expected_kind: Option<&str>,
    ) {
        if depth > 6 {
            return;
        }
        match v {
            Value::Array(xs) => {
                for x in xs {
                    collect(x, out, depth + 1, record, kind, expected_kind)
                }
            }
            Value::String(_) if record => {
                add(v, out);
            }
            Value::Object(map) => {
                let keys: &[&str] = match expected_kind.or(kind) {
                    Some("image") => &["image_url", "result_url", "download_url"],
                    Some("video") => &["video_url", "result_url", "download_url"],
                    _ => &["image_url", "video_url", "result_url", "download_url"],
                };
                // A generic `url` is accepted only inside an output container and only
                // as fallback. Queue/help/reference/thumbnail dictionaries are never walked.
                let opposite_record = kind.is_none()
                    && match expected_kind {
                        Some("image") => {
                            map.contains_key("video_url") && !map.contains_key("image_url")
                        }
                        Some("video") => {
                            map.contains_key("image_url") && !map.contains_key("video_url")
                        }
                        _ => false,
                    };
                let typed_result = !opposite_record
                    && keys
                        .iter()
                        .any(|key| map.get(*key).is_some_and(|url| add(url, out)));
                if record && !typed_result && !opposite_record {
                    if let Some(url) = map.get("url") {
                        add(url, out);
                    }
                }
                for key in [
                    "data",
                    "result",
                    "results",
                    "result_json",
                    "result_data",
                    "outputs",
                    "images",
                    "videos",
                    "result_list",
                    "result_urls",
                    "image_urls",
                    "video_urls",
                ] {
                    if let Some(nested) = map.get(key) {
                        if (key == "result_json" && !nested.is_object())
                            || (["result_urls", "image_urls", "video_urls"].contains(&key)
                                && !nested.is_array())
                        {
                            continue;
                        }
                        let kind = match key {
                            "images" | "image_urls" => Some("image"),
                            "videos" | "video_urls" => Some("video"),
                            _ => kind,
                        };
                        if expected_kind.is_some() && kind.is_some() && expected_kind != kind {
                            continue;
                        }
                        collect(nested, out, depth + 1, key != "data", kind, expected_kind);
                    }
                }
            }
            _ => {}
        }
    }
    let mut urls = Vec::new();
    collect(value, &mut urls, 0, false, None, expected_kind);
    urls
}
#[cfg(test)]
fn normalized(body: Value, expected: Option<&str>) -> ProviderResult<QueryResult> {
    normalized_for_kind(body, expected, None)
}
fn normalized_for_kind(
    body: Value,
    expected: Option<&str>,
    kind: Option<&str>,
) -> ProviderResult<QueryResult> {
    let task = task_object(&body);
    let mut explicit_id = None;
    for envelope in [&body, task] {
        for alias in ["submit_id", "submitId", "task_id", "taskId"] {
            if let Some(value) = envelope.get(alias) {
                let id = value.as_str().ok_or_else(|| {
                    ProviderError::new("TASK_ID_INVALID", "CLI 返回了无法识别的任务身份类型。")
                })?;
                valid_id(id).map_err(|_| {
                    ProviderError::new("TASK_ID_INVALID", "CLI 返回的任务身份无效。")
                })?;
                if explicit_id.is_some_and(|previous| previous != id) {
                    return Err(ProviderError::new(
                        "TASK_ID_CONFLICT",
                        "CLI 返回的多个任务身份不一致。",
                    ));
                }
                explicit_id = Some(id);
            }
        }
    }
    let id = explicit_id.or(expected).unwrap_or("");
    if expected.is_some_and(|x| x != id) {
        return Err(ProviderError::new(
            "TASK_MISMATCH",
            "CLI 返回了其他任务的结果。",
        ));
    }
    let status = match task.get("gen_status").and_then(Value::as_str) {
        Some("success") => "succeeded",
        Some("fail" | "failed") => "failed",
        Some("querying" | "queued" | "pending") => "processing",
        _ => "unknown",
    };
    let urls = if status == "succeeded" {
        result_urls(task, kind)
    } else {
        Vec::new()
    };
    let error = task
        .get("fail_reason")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| wire::safe_message(task));
    Ok(QueryResult {
        task_id: id.into(),
        status: status.into(),
        result_urls: urls,
        error,
        actual_cost: None,
        raw: body,
    })
}
const LOCAL_MEDIA_TRANSPORT: &str = "relative-files/v1";

struct CliMediaInvocation {
    args: Vec<String>,
    media_directory: Option<tempfile::TempDir>,
}
impl CliMediaInvocation {
    fn directory(&self) -> Option<&Path> {
        self.media_directory.as_ref().map(tempfile::TempDir::path)
    }
}

fn local_media_name(index: usize, path: &Path) -> ProviderResult<String> {
    let extension = path.extension().and_then(|value| value.to_str());
    if extension.is_some_and(|value| {
        value.is_empty() || value.len() > 16 || !value.bytes().all(|b| b.is_ascii_alphanumeric())
    }) {
        return Err(ProviderError::new(
            "MEDIA_PATH",
            "素材扩展名无法安全保留为官方 CLI 的本地文件；尚未提交生成。",
        ));
    }
    Ok(match extension {
        Some(extension) => format!("./reference-{:03}.{extension}", index + 1),
        None => format!("./reference-{:03}", index + 1),
    })
}

fn materialize_cli_media(
    request: &GenerationRequest,
    mut args: Vec<String>,
) -> ProviderResult<CliMediaInvocation> {
    let references = sorted_references(request);
    if references.is_empty() {
        return Ok(CliMediaInvocation {
            args,
            media_directory: None,
        });
    }
    // No global cwd change, URL conversion or shared fixed files. Each invocation
    // owns byte-identical inputs until its CLI process and output readers finish.
    // A temporary copy also makes source mutation after validation fail closed.
    let directory = tempfile::Builder::new()
        .prefix("director-dreamina-media-")
        .tempdir()
        .map_err(|_| {
            ProviderError::new(
                "MEDIA_TRANSPORT",
                "无法创建本地素材运输目录；尚未提交生成。",
            )
        })?;
    let mut relative = Vec::with_capacity(references.len());
    for (index, reference) in references.iter().enumerate() {
        let name = local_media_name(index, &reference.path)?;
        let destination = directory.path().join(name.strip_prefix("./").unwrap());
        fs::copy(&reference.path, &destination).map_err(|_| {
            ProviderError::new("MEDIA_TRANSPORT", "本地素材物化失败；尚未提交生成。")
        })?;
        let mut source = fs::File::open(&destination).map_err(|_| {
            ProviderError::new("MEDIA_TRANSPORT", "物化素材无法复核；尚未提交生成。")
        })?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let size = source.read(&mut buffer).map_err(|_| {
                ProviderError::new("MEDIA_TRANSPORT", "物化素材读取失败；尚未提交生成。")
            })?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        if format!("{:x}", hash.finalize()) != reference.sha256 {
            return Err(ProviderError::new(
                "MEDIA_CHANGED",
                "本地素材物化字节与冻结哈希不一致；尚未提交生成。",
            ));
        }
        relative.push(name);
    }
    if request.kind == "image" {
        let expected = format!(
            "--images={}",
            references
                .iter()
                .map(|reference| reference.path.to_str().unwrap())
                .collect::<Vec<_>>()
                .join(",")
        );
        let positions: Vec<_> = args
            .iter()
            .enumerate()
            .filter_map(|(index, arg)| arg.starts_with("--images=").then_some(index))
            .collect();
        if positions.len() != 1 || args[positions[0]] != expected {
            return Err(ProviderError::new(
                "PREPARED_INVALID",
                "图片素材参数与冻结引用不一致；尚未提交生成。",
            ));
        }
        args[positions[0]] = format!("--images={}", relative.join(","));
    } else {
        let positions: Vec<_> = args
            .iter()
            .enumerate()
            .filter_map(|(index, arg)| {
                ["--image=", "--video=", "--audio="]
                    .iter()
                    .any(|prefix| arg.starts_with(prefix))
                    .then_some(index)
            })
            .collect();
        if positions.len() != references.len() {
            return Err(ProviderError::new(
                "PREPARED_INVALID",
                "视频素材参数数量与冻结引用不一致；尚未提交生成。",
            ));
        }
        for ((position, reference), name) in positions.into_iter().zip(&references).zip(relative) {
            let expected = format!("--{}={}", reference.kind, reference.path.to_str().unwrap());
            if args[position] != expected {
                return Err(ProviderError::new(
                    "PREPARED_INVALID",
                    "视频素材顺序与冻结引用不一致；尚未提交生成。",
                ));
            }
            args[position] = format!("--{}={name}", reference.kind);
        }
    }
    Ok(CliMediaInvocation {
        args,
        media_directory: Some(directory),
    })
}

pub(super) fn submit(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
) -> ProviderResult<SubmitReceipt> {
    submit_with_media_runner(config, prepared, |args, directory| {
        run_in_directory(config, args, directory)
    })
}

fn validated_submit_args(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
) -> ProviderResult<Vec<String>> {
    // The caller has already persisted its submit intent. All current help/version
    // commands belong to prepare; submit performs only local integrity checks before
    // starting the actual generation process. A replaced CLI invalidates the preflight.
    validate_request(&prepared.request)?;
    let identity = prepared.wire.get("cliIdentity").ok_or_else(|| {
        ProviderError::new(
            "CLI_PREFLIGHT_MISSING",
            "此准备结果缺少官方 CLI 文件证明，请重新准备后生成。",
        )
    })?;
    if &cli_identity(config)? != identity {
        return Err(ProviderError::new(
            "CLI_CHANGED",
            "官方 CLI 已更换，请重新准备关联单元。",
        ));
    }
    let preflight = prepared
        .wire
        .get("preflight")
        .ok_or_else(|| ProviderError::new("PREPARED_INVALID", "CLI 准备能力快照缺失。"))?;
    if preflight.get("capabilityVersion").and_then(Value::as_str)
        != Some(prepared.capability_version.as_str())
    {
        return Err(ProviderError::new(
            "PREPARED_INVALID",
            "CLI 准备能力身份不一致。",
        ));
    }
    let model: ModelCapability = serde_json::from_value(preflight["model"].clone())
        .map_err(|_| ProviderError::new("PREPARED_INVALID", "CLI 准备型号能力无效。"))?;
    let caps = Capabilities {
        provider: "dreamina".into(),
        version: prepared.capability_version.clone(),
        source: "frozen_preflight".into(),
        checked_at: prepared.prepared_at.clone(),
        models: vec![model],
        can_cancel: false,
        can_reconcile_unknown: false,
        raw: Value::Null,
    };
    let mut current = prepare_with_capabilities(&prepared.request, &caps)?;
    current.wire["cliIdentity"] = identity.clone();
    match prepared.wire.get("localMediaTransport") {
        None => {
            // Old immutable preparations remain verifiable. Existing cloud IDs
            // never enter submit; explicit unsubmitted work uses safe local paths.
            current
                .wire
                .as_object_mut()
                .unwrap()
                .remove("localMediaTransport");
        }
        Some(Value::String(profile)) if profile == LOCAL_MEDIA_TRANSPORT => {}
        Some(_) => {
            return Err(ProviderError::new(
                "UNSUPPORTED_MEDIA_TRANSPORT",
                "官方 CLI 本地素材运输规约未知；尚未提交生成。",
            ));
        }
    }
    if current.wire != prepared.wire || !prepared.upload_receipts.is_empty() {
        return Err(ProviderError::new(
            "PREPARED_INVALID",
            "CLI 参数与冻结请求不一致。",
        ));
    }
    current
        .wire
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::new("PREPARED_INVALID", "CLI 准备快照无效。"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| ProviderError::new("PREPARED_INVALID", "CLI 参数无效。"))
        })
        .collect::<ProviderResult<_>>()
}

#[cfg(test)]
fn submit_with_runner(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
    runner: impl FnOnce(&[String]) -> ProviderResult<Vec<u8>>,
) -> ProviderResult<SubmitReceipt> {
    submit_with_media_runner(config, prepared, |args, _| runner(args))
}

fn submit_with_media_runner(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
    runner: impl FnOnce(&[String], Option<&Path>) -> ProviderResult<Vec<u8>>,
) -> ProviderResult<SubmitReceipt> {
    let args = validated_submit_args(config, prepared)?;
    let invocation = materialize_cli_media(&prepared.request, args)?;
    let body = parse_output(&runner(&invocation.args, invocation.directory())?, true)?;
    let result =
        normalized_for_kind(body, None, Some(&prepared.request.kind)).map_err(|error| {
            if ["TASK_ID_INVALID", "TASK_ID_CONFLICT"].contains(&error.code.as_str()) {
                ProviderError::uncertain(
                    "官方 CLI 已运行生成但提交回执任务身份无效；请核对原任务，不能自动重发。",
                )
            } else {
                error
            }
        })?;
    if result.task_id.is_empty() {
        return Err(ProviderError::uncertain(
            "官方 CLI 未返回 submit_id；请核对任务历史。",
        ));
    }
    Ok(SubmitReceipt {
        task_id: result.task_id,
        status: result.status,
        result_urls: result.result_urls,
        raw: result.raw,
    })
}
pub(super) fn query(config: &ProviderConfig, task_id: &str) -> ProviderResult<QueryResult> {
    query_with_runner(task_id, None, || {
        run(
            config,
            &["query_result".into(), format!("--submit_id={task_id}")],
            false,
        )
    })
}
pub(super) fn query_kind(
    config: &ProviderConfig,
    task_id: &str,
    kind: &str,
) -> ProviderResult<QueryResult> {
    query_with_runner(task_id, Some(kind), || {
        run(
            config,
            &["query_result".into(), format!("--submit_id={task_id}")],
            false,
        )
    })
}
pub(super) fn query_prepared(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
    task_id: &str,
) -> ProviderResult<QueryResult> {
    query_prepared_with_runner(prepared, task_id, || {
        run(
            config,
            &["query_result".into(), format!("--submit_id={task_id}")],
            false,
        )
    })
}
fn query_prepared_with_runner(
    prepared: &PreparedRequest,
    task_id: &str,
    runner: impl FnOnce() -> ProviderResult<Vec<u8>>,
) -> ProviderResult<QueryResult> {
    if prepared.provider != "dreamina" {
        return Err(ProviderError::new(
            "PROVIDER_MISMATCH",
            "原任务准备身份与即梦渠道不一致。",
        ));
    }
    query_with_runner(task_id, Some(&prepared.request.kind), runner)
}
fn query_with_runner(
    task_id: &str,
    kind: Option<&str>,
    runner: impl FnOnce() -> ProviderResult<Vec<u8>>,
) -> ProviderResult<QueryResult> {
    valid_id(task_id)?;
    if kind.is_some_and(|kind| !["image", "video"].contains(&kind)) {
        return Err(ProviderError::new(
            "INVALID_RESULT_KIND",
            "原任务的媒体类型无效。",
        ));
    }
    let body = parse_output(&runner()?, false)?;
    normalized_for_kind(body, Some(task_id), kind)
}

#[cfg(test)]
mod bounded_process_tests {
    use super::*;
    include!("cli_observation_tests.rs");

    struct Fixture {
        _directory: tempfile::TempDir,
        executable: PathBuf,
    }
    fn fixture() -> &'static Fixture {
        static FIXTURE: OnceLock<Fixture> = OnceLock::new();
        FIXTURE.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("fixture.rs");
            let executable = directory.path().join(if cfg!(windows) { "fixture.exe" } else { "fixture" });
            fs::write(&source, r#"
use std::{io::Write,process::{Command,Stdio},time::{Duration,Instant}};
fn main() {
    let args:Vec<String>=std::env::args().collect();
    let mode=&args[1];
    if mode=="hold" { std::thread::sleep(Duration::from_millis(1500)); return; }
    let counter=std::path::Path::new(&args[2]);
    std::fs::OpenOptions::new().create(true).append(true).open(counter).unwrap().write_all(b"1").unwrap();
    std::fs::write(counter.with_extension("pid"),std::process::id().to_string()).unwrap();
    if mode=="observation-gated" {
        let gate=counter.with_extension("release");
        let deadline=Instant::now()+Duration::from_secs(10);
        while !gate.exists() {
            assert!(Instant::now()<deadline,"supervisor never released offline observation fixture");
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(180));
    }
    if mode=="inherited" {
        let mut c=Command::new(std::env::current_exe().unwrap());
        c.arg("hold").stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
        #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); }
        c.spawn().unwrap();
    }
    if mode=="slow" { std::thread::sleep(Duration::from_secs(10)); }
    if mode=="quiet" { std::thread::sleep(Duration::from_millis(180)); }
    if mode=="flood" {
        std::io::stderr().write_all(&vec![b'e';9*1024*1024]).unwrap();
        std::io::stdout().write_all(&vec![b'o';9*1024*1024]).unwrap();
    }
    if mode=="media" {
        assert_eq!(std::fs::read("reference-001.png").unwrap(),b"immutable fixture reference");
        std::thread::sleep(Duration::from_millis(180));
        assert_eq!(std::fs::read("reference-001.png").unwrap(),b"immutable fixture reference");
    }
    print!("{{\"submit_id\":\"offline-known\",\"gen_status\":\"processing\"}}");
    std::io::stdout().flush().unwrap();
}
"#).unwrap();
            let compiler = std::env::var_os("RUSTC").map(PathBuf::from).unwrap_or_else(|| {
                std::env::var_os("CARGO_HOME").map(PathBuf::from)
                    .or_else(|| std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|p| PathBuf::from(p).join(".cargo")))
                    .map(|p| p.join("bin").join(if cfg!(windows) { "rustc.exe" } else { "rustc" }))
                    .unwrap_or_else(|| PathBuf::from("rustc"))
            });
            let mut compile = Command::new(compiler);
            compile.args(["--edition=2021", "--crate-name", "dreamina_offline_fixture"]).arg(source).arg("-o").arg(&executable);
            #[cfg(windows)] {
                use std::os::windows::process::CommandExt;
                compile.creation_flags(0x08000000);
            }
            let output = compile.output().unwrap();
            assert!(output.status.success(), "fixture compile failed: {}", String::from_utf8_lossy(&output.stderr));
            Fixture { _directory: directory, executable }
        })
    }
    fn command(mode: &str, counter: &Path) -> Command {
        let mut command = Command::new(&fixture().executable);
        command
            .arg(mode)
            .arg(counter)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
    }
    fn policy() -> CliProcessPolicy {
        CliProcessPolicy {
            run: Duration::from_secs(4),
            drain: Duration::from_millis(80),
            cleanup: Duration::from_millis(500),
            inspection_deadline: None,
        }
    }
    fn run(
        command: Command,
        submitting: bool,
        policy: CliProcessPolicy,
    ) -> ProviderResult<CliOutput> {
        supervise_command(
            command,
            submitting,
            "offline-no-credential".into(),
            policy,
            Arc::new(|| false),
        )
    }
    #[cfg(windows)]
    fn fixture_process_exited(counter: &Path) -> bool {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        let pid = fs::read_to_string(counter.with_extension("pid"))
            .unwrap()
            .parse()
            .unwrap();
        unsafe {
            let handle = OpenProcess(0x00100000, 0, pid);
            if handle.is_null() {
                return true;
            }
            let exited = WaitForSingleObject(handle, 0) == 0;
            CloseHandle(handle);
            exited
        }
    }
    #[test]
    fn quiet_cli_finishes_once_without_false_hang_or_reexecution() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let output = run(command("quiet", &counter), true, policy()).unwrap();
        assert!(output.success);
        assert_eq!(fs::read(counter).unwrap(), b"1");
        assert_eq!(
            parse_output(&output.stdout, true).unwrap()["submit_id"],
            "offline-known"
        );
    }
    #[test]
    fn inherited_pipe_has_a_bound_including_runtime_drop() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let _ = fixture();
        let start = Instant::now();
        let error = run(command("inherited", &counter), false, policy()).unwrap_err();
        assert_eq!(error.code, "CLI_OUTPUT_TIMEOUT");
        assert!(!error.submission_unknown);
        assert!(!error.auth_required);
        assert!(
            start.elapsed() < Duration::from_millis(1000),
            "entire synchronous call must return before 1.5s inherited writer EOF"
        );
        assert_eq!(fs::read(counter).unwrap(), b"1");
    }
    #[test]
    fn output_retention_limit_still_drains_both_pipes_until_cli_exit() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let error = run(command("flood", &counter), false, policy()).unwrap_err();
        assert_eq!(error.code, "CLI_OUTPUT_LIMIT");
        assert!(!error.submission_unknown);
        assert_eq!(fs::read(&counter).unwrap(), b"1");
        #[cfg(windows)]
        assert!(fixture_process_exited(&counter));
    }
    #[test]
    fn timeout_kills_reaps_and_preserves_readonly_error_type() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let _ = fixture();
        let mut p = policy();
        p.run = Duration::from_millis(100);
        let start = Instant::now();
        let error = run(command("slow", &counter), false, p).unwrap_err();
        assert_eq!(error.code, "CLI_TIMEOUT");
        assert!(!error.submission_unknown);
        assert!(!error.auth_required);
        assert_eq!(fs::read(&counter).unwrap(), b"1");
        assert!(start.elapsed() < Duration::from_secs(2));
        #[cfg(windows)]
        assert!(fixture_process_exited(&counter));
    }
    #[test]
    fn submitting_timeout_is_unknown_and_never_starts_second_exec() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let mut p = policy();
        p.run = Duration::from_millis(100);
        let error = run(command("slow", &counter), true, p).unwrap_err();
        assert!(error.submission_unknown);
        assert!(!error.auth_required);
        assert_eq!(fs::read(&counter).unwrap(), b"1");
        #[cfg(windows)]
        assert!(fixture_process_exited(&counter));
    }
    #[test]
    fn close_during_execution_is_bounded_and_before_spawn_is_not_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let _ = fixture();
        let start = Instant::now();
        let error = supervise_command(
            command("slow", &counter),
            false,
            "offline".into(),
            policy(),
            Arc::new(move || start.elapsed() > Duration::from_millis(100)),
        )
        .unwrap_err();
        assert_eq!(error.code, "CLIENT_CLOSING");
        assert!(!error.auth_required);
        assert!(!error.submission_unknown);
        assert!(start.elapsed() < Duration::from_secs(2));
        #[cfg(windows)]
        assert!(fixture_process_exited(&counter));
        let never = dir.path().join("never");
        let error = supervise_command(
            command("slow", &never),
            true,
            "offline".into(),
            policy(),
            Arc::new(|| true),
        )
        .unwrap_err();
        assert_eq!(error.code, "CLIENT_CLOSING");
        assert!(!error.submission_unknown);
        assert!(!never.exists());
    }
    #[test]
    fn missing_executable_is_proven_unsubmitted() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Command::new(dir.path().join("absent-cli"));
        c.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let error = run(c, true, policy()).unwrap_err();
        assert_eq!(error.code, "CLI_START");
        assert!(!error.submission_unknown);
    }
    #[test]
    fn reference_directory_survives_complete_process_collection() {
        // Bytes verify transport identity only; this fixture is not image-quality evidence.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.png");
        fs::write(&path, b"immutable fixture reference").unwrap();
        let request = GenerationRequest {
            request_id: "offline".into(),
            kind: "image".into(),
            model: "dreamina-image-5.0".into(),
            prompt: "offline".into(),
            duration: None,
            ratio: "16:9".into(),
            resolution: "2k".into(),
            references: vec![MediaReference {
                media_id: "media".into(),
                kind: "image".into(),
                path: path.clone(),
                sha256: hex_hash(&fs::read(&path).unwrap()),
                order: 1,
                usage: "identity".into(),
                duration_seconds: None,
            }],
            options: json!({}),
        };
        let before = serde_json::to_value(&request).unwrap();
        let invocation = materialize_cli_media(
            &request,
            vec![format!("--images={}", path.to_str().unwrap())],
        )
        .unwrap();
        let materialized = invocation.directory().unwrap().to_path_buf();
        let counter = dir.path().join("calls");
        let mut c = command("media", &counter);
        c.current_dir(&materialized);
        let output = run(c, true, policy()).unwrap();
        assert!(output.success);
        assert!(materialized.join("reference-001.png").is_file());
        assert_eq!(before, serde_json::to_value(&request).unwrap());
        drop(invocation);
        assert!(!materialized.exists());
        assert_eq!(fs::read(counter).unwrap(), b"1");
    }
    #[tokio::test]
    async fn synchronous_adapter_is_safe_inside_existing_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        let output = run(command("quiet", &counter), false, policy()).unwrap();
        assert!(output.success);
        assert_eq!(fs::read(counter).unwrap(), b"1");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VIDEO_HELP: &str =
        include_str!("../../../tools/dreamina/1.4.19/multimodal2video-help.txt");
    const TEXT_VIDEO_HELP: &str =
        include_str!("../../../tools/dreamina/1.4.19/text2video-help.txt");
    const IMAGE_HELP: &str = "model_version: 3.0, 3.1, 4.0, 4.1, 4.5, 4.6, 4.7, 5.0, 5.0Pro\nratio: 21:9, 16:9, 3:2, 4:3, 1:1, 3:4, 2:3, 9:16\n- 3.0/3.1 -> resolution_type 1k or 2k; custom width/height requires resolution_type 2k\n- 4.0/4.1/4.5/4.6/4.7/5.0 -> resolution_type 2k or 4k\n- 5.0Pro -> resolution_type 1.5k, 2k, or 4k\n--width and --height are mutually exclusive with --ratio\n- 2k: each side 768-3072; total pixels <= 4194304";
    const EDIT_HELP: &str = "model_version: 4.0, 4.1, 4.5, 4.6, 4.7, 5.0, 5.0Pro\nratio: 21:9, 16:9, 3:2, 4:3, 1:1, 3:4, 2:3, 9:16\n- 4.0/4.1/4.5/4.6/4.7/5.0 -> resolution_type 2k or 4k\n- 5.0Pro -> resolution_type 1.5k, 2k, or 4k\n--width and --height are mutually exclusive with --ratio\n- 2k: each side 768-3072; total pixels <= 4194304";
    #[test]
    fn legacy_readiness_help_requires_root_commands_and_each_legacy_flag_contract() {
        let actual_root = include_str!("../../../tools/dreamina/1.4.19/root-help.txt");
        assert!(legacy_readiness_help_supported(None, actual_root.as_bytes()));
        for (command, help) in [
            ("text2image", include_str!("../../../tools/dreamina/1.4.19/text2image-help.txt")),
            ("image2image", include_str!("../../../tools/dreamina/1.4.19/image2image-help.txt")),
        ] {
            assert!(legacy_readiness_help_supported(Some(command), help.as_bytes()));
        }
        let root = "Usage:\n  dreamina [command]\nAvailable Commands:\n login\n user_credit\n query_result\n text2image\n image2image\n text2video\n multimodal2video\nFlags:\n --version";
        assert!(legacy_readiness_help_supported(None, root.as_bytes()));
        assert!(!legacy_readiness_help_supported(None, root.replace("user_credit", "account").as_bytes()));
        assert!(!legacy_readiness_help_supported(None, root.replace("dreamina [command]", "dreamina-canvas [command]").as_bytes()));
        for (command, help) in [("multimodal2video", VIDEO_HELP), ("text2video", TEXT_VIDEO_HELP)] {
            assert!(legacy_readiness_help_supported(Some(command), help.as_bytes()));
            assert!(!legacy_readiness_help_supported(Some(command), help.replace("--model_version", "--model").as_bytes()));
        }
        for command in ["text2image", "image2image"] {
            let help = format!("Usage:\n  dreamina {command} [flags]\nFlags:\n --model_version 5.0 --ratio 16:9 --resolution_type 2k --prompt text");
            assert!(legacy_readiness_help_supported(Some(command), help.as_bytes()));
            assert!(!legacy_readiness_help_supported(Some(command), help.replace("--resolution_type", "--resolution").as_bytes()));
            assert!(!legacy_readiness_help_supported(Some("text2video"), help.as_bytes()));
        }
        assert!(!legacy_readiness_help_supported(None, br#"{"schemaVersion":"1","ok":true,"data":{"help":"dreamina [command]"}}"#));
        assert!(!legacy_readiness_help_supported(Some("text2image"), b"\xff"));
    }
    fn offline_help(args: &[String]) -> Vec<u8> {
        match args[0].as_str() {
            "version" => b"{}".to_vec(),
            "multimodal2video" => VIDEO_HELP.as_bytes().to_vec(),
            "text2video" => TEXT_VIDEO_HELP.as_bytes().to_vec(),
            "text2image" => IMAGE_HELP.as_bytes().to_vec(),
            "image2image" => EDIT_HELP.as_bytes().to_vec(),
            _ => panic!("unexpected capability probe"),
        }
    }
    #[test]
    fn capability_inspection_shares_a_total_deadline_and_does_not_change_generation_config() {
        let config: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","timeoutSeconds":120})).unwrap();
        let base = Instant::now();
        let time = std::cell::Cell::new(base);
        let mut calls = Vec::new();
        let result = capabilities_with_runner(
            &config,
            || time.get(),
            |seen, args, deadline| {
                assert_eq!(seen.timeout_seconds, 120);
                assert_eq!(deadline - time.get(), Duration::from_secs(15));
                assert!(deadline <= base + Duration::from_secs(30));
                calls.push(args.to_vec());
                time.set(time.get() + Duration::from_secs(3));
                Ok(offline_help(args))
            },
        )
        .unwrap();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0], ["version"]);
        assert!(calls[1..]
            .iter()
            .all(|args| args.len() == 2 && args[1] == "-h"));
        assert_eq!(result.models.len(), 15);
        assert_eq!(config.timeout_seconds, 120);
        assert!(
            config.cli_path.is_none(),
            "injected inspection never starts a CLI"
        );
    }
    #[test]
    fn exhausted_capability_budget_stops_before_the_remaining_image_probes() {
        let config: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","timeoutSeconds":120})).unwrap();
        let base = Instant::now();
        let time = std::cell::Cell::new(base);
        let mut calls = 0;
        let error = capabilities_with_runner(
            &config,
            || time.get(),
            |_, args, deadline| {
                calls += 1;
                if calls <= 2 {
                    assert_eq!(deadline - time.get(), Duration::from_secs(15));
                    time.set(time.get() + Duration::from_secs(14));
                } else {
                    assert_eq!(deadline - time.get(), Duration::from_secs(2));
                    assert_eq!(deadline, base + Duration::from_secs(30));
                    time.set(deadline);
                }
                Ok(offline_help(args))
            },
        )
        .unwrap_err();
        assert_eq!(calls, 3);
        assert_eq!(error.code, "CLI_CAPABILITY_TIMEOUT");
        assert!(!error.submission_unknown);
        assert!(!error.auth_required);
    }
    #[test]
    fn capability_inspection_respects_a_shorter_user_timeout() {
        let config: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","timeoutSeconds":1})).unwrap();
        let time = std::cell::Cell::new(Instant::now());
        let mut calls = 0;
        let error = capabilities_with_runner(
            &config,
            || time.get(),
            |_, _, deadline| {
                calls += 1;
                assert_eq!(deadline - time.get(), Duration::from_secs(1));
                time.set(deadline);
                Ok(b"{}".to_vec())
            },
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error.code, "CLI_CAPABILITY_TIMEOUT");
    }
    #[test]
    fn expired_capability_budget_does_not_start_a_command() {
        let config: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","timeoutSeconds":120})).unwrap();
        let base = Instant::now();
        let mut clock_calls = 0;
        let error = capabilities_with_runner(
            &config,
            || {
                clock_calls += 1;
                if clock_calls == 1 {
                    base
                } else {
                    base + Duration::from_secs(30)
                }
            },
            |_, _, _| panic!("expired inspection must never start a command"),
        )
        .unwrap_err();
        assert_eq!(error.code, "CLI_CAPABILITY_TIMEOUT");
    }
    fn image_caps() -> Capabilities {
        capabilities_from_help(b"{}", "", "", IMAGE_HELP, EDIT_HELP).unwrap()
    }
    fn image_request(model: &str, ratio: &str, resolution: &str) -> GenerationRequest {
        GenerationRequest {
            request_id: "offline-image".into(),
            kind: "image".into(),
            model: model.into(),
            prompt: "offline reference character".into(),
            duration: None,
            ratio: ratio.into(),
            resolution: resolution.into(),
            references: vec![],
            options: json!({}),
        }
    }

    fn transport_reference(directory: &Path, name: &str, kind: &str, order: u32) -> MediaReference {
        let path = directory.join(name);
        let bytes = format!("offline frozen bytes for {name}").into_bytes();
        fs::write(&path, &bytes).unwrap();
        MediaReference {
            media_id: format!("media:{kind}:{order}"),
            kind: kind.into(),
            path,
            sha256: hex_hash(&bytes),
            order,
            usage: "reference".into(),
            duration_seconds: (kind != "image").then_some(3.),
        }
    }
    fn transport_prepared(
        config: &ProviderConfig,
        references: Vec<MediaReference>,
        video: bool,
    ) -> PreparedRequest {
        let mut request = image_request(
            if video { "seedance2.0" } else { "5.0" },
            "16:9",
            if video { "720p" } else { "2k" },
        );
        if video {
            request.kind = "video".into();
            request.duration = Some(5);
        }
        request.references = references;
        let caps =
            capabilities_from_help(b"{}", VIDEO_HELP, TEXT_VIDEO_HELP, IMAGE_HELP, EDIT_HELP)
                .unwrap();
        let mut prepared = prepare_with_capabilities(&request, &caps).unwrap();
        prepared.wire["cliIdentity"] = cli_identity(config).unwrap();
        prepared
    }

    #[test]
    fn local_media_transport_image_uses_ordered_relative_bytes_and_keeps_canonical_snapshot() {
        let (_cli, config, _) = fake_cli_prepared();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let refs = vec![
            transport_reference(first.path(), "中文 first.png", "image", 1),
            transport_reference(second.path(), "second image.PNG", "image", 0),
        ];
        let mut prepared = transport_prepared(&config, refs, false);
        prepared
            .request
            .prompt
            .push_str(" Literal C:\\not-a-reference.png must stay prompt text.");
        let caps = image_caps();
        let mut new = prepare_with_capabilities(&prepared.request, &caps).unwrap();
        new.wire["cliIdentity"] = cli_identity(&config).unwrap();
        prepared = new;
        let snapshot = serde_json::to_string(&prepared).unwrap();
        let parent_cwd = std::env::current_dir().unwrap();
        let mut owned = None;
        let mut calls = 0;
        let receipt = submit_with_media_runner(&config, &prepared, |args, cwd| {
            calls += 1;
            let cwd = cwd.unwrap();
            owned = Some(cwd.to_path_buf());
            assert_eq!(args[0], "image2image");
            let images = args
                .iter()
                .find(|value| value.starts_with("--images="))
                .unwrap()
                .strip_prefix("--images=")
                .unwrap();
            assert_eq!(images, "./reference-001.PNG,./reference-002.png");
            for (name, reference) in images.split(',').zip(&prepared.request.references) {
                assert!(name.starts_with("./") && !name.contains([':', '\\', '"']));
                let bytes = fs::read(cwd.join(name)).unwrap();
                assert_eq!(hex_hash(&bytes), reference.sha256);
                assert_eq!(bytes, fs::read(&reference.path).unwrap());
            }
            assert!(args
                .iter()
                .any(|value| value == &format!("--prompt={}", prepared.request.prompt)));
            assert_eq!(std::env::current_dir().unwrap(), parent_cwd);
            Ok(br#"{"submit_id":"relative-image","gen_status":"querying"}"#.to_vec())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(receipt.task_id, "relative-image");
        assert_eq!(serde_json::to_string(&prepared).unwrap(), snapshot);
        assert!(
            !owned.unwrap().exists(),
            "temporary copies live through the runner and are then removed"
        );
    }

    #[test]
    fn local_media_transport_video_preserves_mixed_order_and_each_frozen_file() {
        let (_cli, config, _) = fake_cli_prepared();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let third = tempfile::tempdir().unwrap();
        let refs = vec![
            transport_reference(first.path(), "movie clip.mp4", "video", 2),
            transport_reference(second.path(), "角色.png", "image", 1),
            transport_reference(third.path(), "voice reference.wav", "audio", 0),
        ];
        // Bytes test only the transport trust boundary, not real containers or visual quality.
        let prepared = transport_prepared(&config, refs, true);
        let before = serde_json::to_string(&prepared).unwrap();
        submit_with_media_runner(&config, &prepared, |args, cwd| {
            assert_eq!(args[0], "multimodal2video");
            let paths: Vec<_> = args
                .iter()
                .filter(|arg| {
                    ["--image=", "--video=", "--audio="]
                        .iter()
                        .any(|prefix| arg.starts_with(prefix))
                })
                .collect();
            assert_eq!(
                paths,
                vec![
                    "--audio=./reference-001.wav",
                    "--image=./reference-002.png",
                    "--video=./reference-003.mp4"
                ]
            );
            for (arg, reference) in paths.into_iter().zip(&prepared.request.references) {
                let (_, name) = arg.split_once('=').unwrap();
                assert_eq!(
                    hex_hash(&fs::read(cwd.unwrap().join(name)).unwrap()),
                    reference.sha256
                );
            }
            Ok(br#"{"submit_id":"relative-video","gen_status":"querying"}"#.to_vec())
        })
        .unwrap();
        assert_eq!(serde_json::to_string(&prepared).unwrap(), before);
    }

    #[test]
    fn local_media_transport_changed_source_or_unknown_profile_never_starts_generation() {
        let (_cli, config, _) = fake_cli_prepared();
        let media = tempfile::tempdir().unwrap();
        let reference = transport_reference(media.path(), "source.png", "image", 0);
        let mut prepared = transport_prepared(&config, vec![reference.clone()], false);
        prepared.wire["localMediaTransport"] = json!("unknown-profile");
        let mut calls = 0;
        let error = submit_with_media_runner(&config, &prepared, |_, _| {
            calls += 1;
            Ok(vec![])
        })
        .unwrap_err();
        assert_eq!(error.code, "UNSUPPORTED_MEDIA_TRANSPORT");
        assert_eq!(calls, 0);
        prepared.wire["localMediaTransport"] = json!(LOCAL_MEDIA_TRANSPORT);
        fs::write(&reference.path, b"changed source").unwrap();
        let error = submit_with_media_runner(&config, &prepared, |_, _| {
            calls += 1;
            Ok(vec![])
        })
        .unwrap_err();
        assert_eq!(error.code, "MEDIA_CHANGED");
        assert_eq!(calls, 0);
        assert!(!error.submission_unknown);
    }

    #[test]
    fn local_media_transport_copy_rechecks_bytes_even_after_original_preflight() {
        let media = tempfile::tempdir().unwrap();
        let reference = transport_reference(media.path(), "source.png", "image", 0);
        let request = GenerationRequest {
            references: vec![reference.clone()],
            ..image_request("5.0", "16:9", "2k")
        };
        let args = prepare_with_capabilities(&request, &image_caps())
            .unwrap()
            .wire["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        fs::write(&reference.path, b"changed after validation").unwrap();
        let error = materialize_cli_media(&request, args).err().unwrap();
        assert_eq!(error.code, "MEDIA_CHANGED");
        assert!(!error.submission_unknown);
    }

    #[test]
    fn local_media_transport_unknown_submit_is_once_only_and_cleans_owned_files() {
        let (_cli, config, _) = fake_cli_prepared();
        let media = tempfile::tempdir().unwrap();
        let prepared = transport_prepared(
            &config,
            vec![transport_reference(media.path(), "source.png", "image", 0)],
            false,
        );
        let mut owned = None;
        let mut calls = 0;
        let error = submit_with_media_runner(&config, &prepared, |_, cwd| {
            calls += 1;
            owned = Some(cwd.unwrap().to_path_buf());
            Ok(b"not a receipt".to_vec())
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(error.submission_unknown);
        assert!(!owned.unwrap().exists());
    }

    #[test]
    fn local_media_transport_legacy_preparation_stays_exact_and_original_id_query_needs_no_file() {
        let (_cli, config, _) = fake_cli_prepared();
        let media = tempfile::tempdir().unwrap();
        let reference = transport_reference(media.path(), "old input.png", "image", 0);
        let mut prepared = transport_prepared(&config, vec![reference.clone()], false);
        prepared
            .wire
            .as_object_mut()
            .unwrap()
            .remove("localMediaTransport");
        let old = serde_json::to_string(&prepared).unwrap();
        submit_with_media_runner(&config, &prepared, |args, cwd| {
            assert!(args.iter().any(|arg| arg == "--images=./reference-001.png"));
            assert!(cwd.unwrap().join("reference-001.png").exists());
            Ok(
                br#"{"submit_id":"explicit-unsubmitted-operation","gen_status":"querying"}"#
                    .to_vec(),
            )
        })
        .unwrap();
        assert_eq!(serde_json::to_string(&prepared).unwrap(), old);
        fs::remove_file(&reference.path).unwrap();
        let mut query_calls = 0;
        let result = query_prepared_with_runner(&prepared, "old-durable-id", || {
            query_calls += 1;
            Ok(br#"{"submit_id":"old-durable-id","gen_status":"querying"}"#.to_vec())
        })
        .unwrap();
        assert_eq!(result.task_id, "old-durable-id");
        assert_eq!(query_calls, 1);
        assert_eq!(serde_json::to_string(&prepared).unwrap(), old);
    }

    #[test]
    fn local_media_transport_simultaneous_invocations_have_private_directories() {
        let (_cli, config, _) = fake_cli_prepared();
        let media = tempfile::tempdir().unwrap();
        let prepared = transport_prepared(
            &config,
            vec![transport_reference(media.path(), "source.png", "image", 0)],
            false,
        );
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let config = config.clone();
                let prepared = prepared.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut own = None;
                    submit_with_media_runner(&config, &prepared, |_, cwd| {
                        own = Some(cwd.unwrap().to_path_buf());
                        barrier.wait();
                        assert!(own.as_ref().unwrap().join("reference-001.png").exists());
                        Ok(br#"{"submit_id":"fixture-once","gen_status":"querying"}"#.to_vec())
                    })
                    .unwrap();
                    own.unwrap()
                })
            })
            .collect();
        let paths: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_ne!(paths[0], paths[1]);
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn local_media_transport_child_command_keeps_verified_absolute_cli_and_rejects_relative_config()
    {
        let directory = tempfile::Builder::new()
            .prefix("director-relative-cli-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let executable = directory.path().join("offline-cli.exe");
        fs::write(&executable, b"non-executable fixture").unwrap();
        let cwd = std::env::current_dir().unwrap();
        let relative = executable.strip_prefix(&cwd).unwrap().to_path_buf();
        let (_cli, mut config, _) = fake_cli_prepared();
        config.cli_path = Some(relative);
        let child = tempfile::tempdir().unwrap();
        assert_eq!(
            cli_command_in_directory(&config, Some(child.path()))
                .unwrap_err()
                .code,
            "CLI_PATH",
            "the configured CLI remains an explicitly selected absolute file"
        );
        config.cli_path = Some(executable.clone());
        let command = cli_command_in_directory(&config, Some(child.path())).unwrap();
        assert_eq!(
            Path::new(command.get_program()),
            fs::canonicalize(executable).unwrap()
        );
        assert_eq!(command.get_current_dir(), Some(child.path()));
        assert_eq!(std::env::current_dir().unwrap(), cwd);
        let proxy = ProxyEnvironment::default();
        assert_ne!(
            safe_launch_fingerprint_for(&proxy),
            safe_launch_fingerprint_in_directory(&proxy, Some(child.path()))
        );
    }

    #[cfg(windows)]
    #[test]
    fn local_media_transport_windows_references_on_available_different_drives_are_relative() {
        let (_cli, config, _) = fake_cli_prepared();
        let system = tempfile::tempdir().unwrap();
        let project = tempfile::Builder::new()
            .prefix("director-cross-drive-")
            .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap();
        let refs = vec![
            transport_reference(system.path(), "C input.png", "image", 0),
            transport_reference(project.path(), "D input.png", "image", 1),
        ];
        let prepared = transport_prepared(&config, refs, false);
        let distinct_drives = prepared.request.references[0].path.components().next()
            != prepared.request.references[1].path.components().next();
        submit_with_media_runner(&config, &prepared, |args, cwd| {
            assert!(args
                .iter()
                .any(|arg| arg == "--images=./reference-001.png,./reference-002.png"));
            assert!(cwd.unwrap().join("reference-001.png").exists());
            assert!(cwd.unwrap().join("reference-002.png").exists());
            Ok(br#"{"submit_id":"drive-fixture","gen_status":"querying"}"#.to_vec())
        })
        .unwrap();
        eprintln!(
            "offline Windows media transport tested distinct available drives: {distinct_drives}"
        );
    }
    fn fake_cli_prepared() -> (tempfile::TempDir, ProviderConfig, PreparedRequest) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dreamina-local-proof.exe");
        // A deliberately non-executable file proves the injected submission path
        // never tries to execute version/help during its local preflight validation.
        fs::write(&path, b"offline CLI identity only").unwrap();
        let config = ProviderConfig {
            kind: "dreamina".into(),
            endpoint: None,
            credential_env: None,
            cli_path: Some(path),
            bridge_token_env: None,
            chrome_binding: None,browser_transport:None,
            timeout_seconds: 1,
            adapted_models: vec![],
        };
        let mut prepared =
            prepare_with_capabilities(&image_request("5.0", "16:9", "1080p"), &image_caps())
                .unwrap();
        prepared.wire["cliIdentity"] = cli_identity(&config).unwrap();
        (directory, config, prepared)
    }
    #[test]
    fn capability_cache_one_help_round_serves_startup_and_fifty_four_preflights() {
        let (_directory, config, _) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        let clock = std::cell::Cell::new(Instant::now());
        let probes = std::cell::Cell::new(0);
        let startup = cache
            .get_or_inspect(
                &config,
                || clock.get(),
                |cfg| {
                    capabilities_with_runner(
                        cfg,
                        || clock.get(),
                        |_, args, _| {
                            probes.set(probes.get() + 1);
                            Ok(offline_help(args))
                        },
                    )
                },
            )
            .unwrap();
        assert_eq!(probes.get(), 5, "one version and four help commands");
        for item in 0..54 {
            let mut request = image_request("5.0", "16:9", "2k");
            request.request_id = format!("offline-cached-item-{item}");
            let prepared = prepare_with_capability_resolver(&config, &request, |cfg| {
                cache.get_or_inspect(
                    cfg,
                    || clock.get(),
                    |_| panic!("a successful unchanged CLI inspection must be reused"),
                )
            })
            .unwrap();
            assert_eq!(prepared.capability_version, startup.version);
            assert_eq!(prepared.request.request_id, request.request_id);
            assert_eq!(
                validated_submit_args(&config, &prepared).unwrap()[0],
                "text2image"
            );
        }
        let reread = cache
            .get_or_inspect(
                &config,
                || clock.get(),
                |_| panic!("unexpected reinspection"),
            )
            .unwrap();
        assert_eq!(reread.checked_at, startup.checked_at);
        assert_eq!(probes.get(), 5);
        assert_eq!(config.timeout_seconds, 1, "generation timeout is unchanged");
    }
    #[test]
    fn capability_cache_single_flight_reuses_the_exact_binary_for_concurrent_reads() {
        let (_directory, config, _) = fake_cli_prepared();
        let cache = Arc::new(CapabilityCache::default());
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let config = config.clone();
                let barrier = barrier.clone();
                let reads = reads.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    cache
                        .get_or_inspect(&config, Instant::now, |_| {
                            reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(10));
                            Ok(image_caps())
                        })
                        .unwrap()
                })
            })
            .collect();
        let caps: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(caps
            .iter()
            .all(|value| value.checked_at == caps[0].checked_at));
    }
    #[test]
    fn capability_cache_different_paths_do_not_share_same_content_capabilities() {
        let (directory, config, _) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        let reads = std::cell::Cell::new(0);
        let inspect = |_: &ProviderConfig| {
            reads.set(reads.get() + 1);
            Ok(image_caps())
        };
        cache
            .get_or_inspect(&config, Instant::now, inspect)
            .unwrap();
        let second = directory.path().join("other-dreamina.exe");
        fs::copy(config.cli_path.as_ref().unwrap(), &second).unwrap();
        let mut changed = config.clone();
        changed.cli_path = Some(second);
        cache
            .get_or_inspect(&changed, Instant::now, inspect)
            .unwrap();
        assert_eq!(reads.get(), 2);
    }
    #[test]
    fn capability_cache_same_length_content_replacement_invalidates_and_blocks_old_submit() {
        let (_directory, config, old_prepared) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        let reads = std::cell::Cell::new(0);
        let inspect = |_: &ProviderConfig| {
            reads.set(reads.get() + 1);
            Ok(image_caps())
        };
        cache
            .get_or_inspect(&config, Instant::now, inspect)
            .unwrap();
        let path = config.cli_path.as_ref().unwrap();
        let original = fs::read(path).unwrap();
        let replacement = original.to_ascii_uppercase();
        assert_eq!(replacement.len(), original.len());
        fs::write(path, replacement).unwrap();
        cache
            .get_or_inspect(&config, Instant::now, inspect)
            .unwrap();
        assert_eq!(
            reads.get(),
            2,
            "content hash is checked, not only file size"
        );
        let submitted = std::cell::Cell::new(0);
        let error = submit_with_runner(&config, &old_prepared, |_| {
            submitted.set(submitted.get() + 1);
            panic!("a replaced binary cannot execute a frozen preparation")
        })
        .unwrap_err();
        assert_eq!(error.code, "CLI_CHANGED");
        assert_eq!(submitted.get(), 0);
    }
    #[test]
    fn capability_cache_file_metadata_change_also_invalidates_same_bytes() {
        let (_directory, config, _) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        let reads = std::cell::Cell::new(0);
        let inspect = |_: &ProviderConfig| {
            reads.set(reads.get() + 1);
            Ok(image_caps())
        };
        cache
            .get_or_inspect(&config, Instant::now, inspect)
            .unwrap();
        let path = config.cli_path.as_ref().unwrap();
        let bytes = fs::read(path).unwrap();
        let previous = fs::metadata(path).unwrap().modified().unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(946684800))
            .unwrap();
        assert_ne!(fs::metadata(path).unwrap().modified().unwrap(), previous);
        assert_eq!(fs::read(path).unwrap(), bytes);
        cache
            .get_or_inspect(&config, Instant::now, inspect)
            .unwrap();
        assert_eq!(reads.get(), 2);
    }
    #[test]
    fn capability_cache_ttl_is_short_and_hits_do_not_extend_the_actual_read_time() {
        let (_directory, config, _) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        let base = Instant::now();
        let clock = std::cell::Cell::new(base);
        let reads = std::cell::Cell::new(0);
        let inspect = |_: &ProviderConfig| {
            reads.set(reads.get() + 1);
            let mut value = image_caps();
            value.checked_at = format!("offline-real-read-{}", reads.get());
            Ok(value)
        };
        let first = cache
            .get_or_inspect(&config, || clock.get(), inspect)
            .unwrap();
        clock.set(base + CAPABILITY_CACHE_TTL - Duration::from_secs(1));
        let hit = cache
            .get_or_inspect(&config, || clock.get(), inspect)
            .unwrap();
        assert_eq!(hit.checked_at, first.checked_at);
        assert_eq!(reads.get(), 1);
        clock.set(base + CAPABILITY_CACHE_TTL);
        let refreshed = cache
            .get_or_inspect(&config, || clock.get(), inspect)
            .unwrap();
        assert_eq!(reads.get(), 2);
        assert_ne!(refreshed.checked_at, first.checked_at);
        assert_eq!(CAPABILITY_CACHE_TTL, Duration::from_secs(300));
    }
    #[test]
    fn capability_cache_failures_keep_typed_probe_error_and_never_prepare_or_submit() {
        let (_directory, config, _) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        let probes = std::cell::Cell::new(0);
        let submits = std::cell::Cell::new(0);
        for _ in 0..2 {
            let result = prepare_with_capability_resolver(
                &config,
                &image_request("5.0", "16:9", "2k"),
                |cfg| {
                    cache.get_or_inspect(cfg, Instant::now, |cfg| {
                        capabilities_with_runner(cfg, Instant::now, |_, _, _| {
                            probes.set(probes.get() + 1);
                            Err(ProviderError::new(
                                "CLI_CAPABILITY_TIMEOUT",
                                "offline bounded inspection timeout",
                            ))
                        })
                    })
                },
            );
            match result {
                Err(error) => {
                    assert_eq!(error.code, "CLI_CAPABILITY_TIMEOUT");
                    assert!(error
                        .message
                        .contains("[CLI_CAPABILITY_TIMEOUT; capabilityProbe=version]"));
                    assert!(!error.auth_required);
                    assert!(!error.submission_unknown);
                }
                Ok(prepared) => {
                    let _ = submit_with_runner(&config, &prepared, |_| {
                        submits.set(submits.get() + 1);
                        panic!("a failed inspection cannot reach submit")
                    });
                    panic!("a failed inspection cannot prepare");
                }
            }
        }
        assert_eq!(
            probes.get(),
            2,
            "failures are not reused as a cached result"
        );
        assert_eq!(submits.get(), 0);
        cache
            .get_or_inspect(&config, Instant::now, |_| Ok(image_caps()))
            .unwrap();
        cache
            .get_or_inspect(&config, Instant::now, |_| {
                panic!("success should be cached")
            })
            .unwrap();
    }
    #[test]
    fn capability_cache_does_not_grant_unknown_models_or_relax_per_item_capacity() {
        let (_directory, config, _) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        cache
            .get_or_inspect(&config, Instant::now, |_| {
                capabilities_from_help(b"{}", VIDEO_HELP, TEXT_VIDEO_HELP, IMAGE_HELP, EDIT_HELP)
            })
            .unwrap();
        let resolve = |cfg: &ProviderConfig| {
            cache.get_or_inspect(cfg, Instant::now, |_| panic!("cache is fresh"))
        };
        let error = prepare_with_capability_resolver(
            &config,
            &image_request("unadapted-new-model", "16:9", "2k"),
            resolve,
        )
        .unwrap_err();
        assert_eq!(error.code, "MODEL_UNAVAILABLE");
        assert!(prepare_with_capability_resolver(
            &config,
            &image_request("5.0", "16:9", "1.5k"),
            resolve
        )
        .is_err());
        let mut video = image_request("seedance2.0", "16:9", "720p");
        video.kind = "video".into();
        video.duration = Some(30);
        assert!(prepare_with_capability_resolver(&config, &video, resolve).is_err());
        // No account or token is part of the cached local help contract.
        let cached = resolve(&config).unwrap();
        assert!(cached.raw.get("accountKey").is_none());
        assert!(cached.raw.get("credits").is_none());
        assert_eq!(config.credential_env, None);
    }
    #[test]
    fn capability_cache_changed_binary_during_inspection_is_not_saved() {
        let (_directory, config, _) = fake_cli_prepared();
        let cache = CapabilityCache::default();
        let error = cache
            .get_or_inspect(&config, Instant::now, |cfg| {
                fs::write(
                    cfg.cli_path.as_ref().unwrap(),
                    b"changed during local help read",
                )
                .unwrap();
                Ok(image_caps())
            })
            .unwrap_err();
        assert_eq!(error.code, "CLI_CHANGED");
        assert!(!error.auth_required);
        assert!(!error.submission_unknown);
        let reads = std::cell::Cell::new(0);
        cache
            .get_or_inspect(&config, Instant::now, |_| {
                reads.set(reads.get() + 1);
                Ok(image_caps())
            })
            .unwrap();
        assert_eq!(reads.get(), 1);
    }
    #[test]
    fn video_model_sizes_are_specific_and_fixed_choices_are_not_empty() {
        let caps = capabilities_from_help(b"{}", VIDEO_HELP, TEXT_VIDEO_HELP, "", "").unwrap();
        let expected = [
            ("seedance2.0", vec!["720p"]),
            ("seedance2.0fast", vec!["720p"]),
            ("seedance2.0_vip", vec!["720p", "1080p", "4k"]),
            ("seedance2.0fast_vip", vec!["720p"]),
            ("seedance2.0mini", vec!["720p"]),
            ("seedance2.5", vec!["480p", "720p", "1080p"]),
        ];
        for (id, resolutions) in expected {
            let model = caps.models.iter().find(|m| m.id == id).unwrap();
            assert!(model.adapted, "{id} must remain selectable");
            assert_eq!(model.resolutions, resolutions, "{id}");
            assert_eq!(model.ratios.len(), 6, "{id}");
            assert_eq!(
                model.raw["resolutionMode"],
                if resolutions.len() == 1 {
                    "fixed"
                } else {
                    "choice"
                }
            );
            let mut request = image_request(id, "16:9", resolutions[0]);
            request.kind = "video".into();
            request.duration = Some(5);
            assert!(prepare_with_capabilities(&request, &caps).is_ok(), "{id}");
        }
    }
    #[test]
    fn video_sizes_do_not_leak_from_other_models_or_flags() {
        let text_without_other_rule = TEXT_VIDEO_HELP.replace(
            "- all other models -> video_resolution 720p; duration 4-15s",
            "- all other models -> duration 4-15s",
        );
        let caps =
            capabilities_from_help(b"{}", VIDEO_HELP, &text_without_other_rule, "", "").unwrap();
        let fast = caps
            .models
            .iter()
            .find(|m| m.id == "seedance2.0fast")
            .unwrap();
        assert!(fast.resolutions.is_empty());
        assert!(!fast.adapted);
        let vip = caps
            .models
            .iter()
            .find(|m| m.id == "seedance2.0_vip")
            .unwrap();
        assert_eq!(vip.resolutions, vec!["720p", "1080p", "4k"]);
        let inconsistent = TEXT_VIDEO_HELP.replace(
            "- seedance2.0_vip -> video_resolution 720p, 1080p, or 4k; duration 4-15s",
            "- seedance2.0_vip -> video_resolution 720p; duration 4-15s",
        );
        let caps = capabilities_from_help(b"{}", VIDEO_HELP, &inconsistent, "", "").unwrap();
        assert_eq!(
            caps.models
                .iter()
                .find(|m| m.id == "seedance2.0_vip")
                .unwrap()
                .resolutions,
            vec!["720p"]
        );
    }
    #[test]
    fn frozen_submit_runs_only_the_generation_command_once() {
        let (_directory, config, prepared) = fake_cli_prepared();
        let mut calls = 0;
        let receipt = submit_with_runner(&config, &prepared, |args| {
            calls += 1;
            assert_eq!(args[0], "text2image");
            assert!(args.iter().any(|arg| arg == "--poll=0"));
            assert!(args.iter().any(|arg| arg == "--width=1920"));
            assert!(args.iter().any(|arg| arg == "--resolution_type=2k"));
            assert!(!args
                .iter()
                .any(|arg| ["version", "-h", "--help"].contains(&arg.as_str())));
            Ok(br#"{"submit_id":"offline-task","gen_status":"querying"}"#.to_vec())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(receipt.task_id, "offline-task");
        assert_eq!(receipt.status, "processing");
    }
    #[test]
    fn changed_cli_or_frozen_wire_is_rejected_before_generation() {
        let (_directory, config, mut prepared) = fake_cli_prepared();
        prepared.wire["args"][1] = json!("--model_version=unverified");
        let mut called = false;
        let error = submit_with_runner(&config, &prepared, |_| {
            called = true;
            Ok(vec![])
        })
        .unwrap_err();
        assert_eq!(error.code, "PREPARED_INVALID");
        assert!(!called);
        let (_directory, config, prepared) = fake_cli_prepared();
        fs::write(
            config.cli_path.as_ref().unwrap(),
            b"CLI changed after preflight",
        )
        .unwrap();
        let error = submit_with_runner(&config, &prepared, |_| {
            called = true;
            Ok(vec![])
        })
        .unwrap_err();
        assert_eq!(error.code, "CLI_CHANGED");
        assert!(!called);
    }
    #[test]
    fn local_submit_keeps_options_and_media_constraints() {
        let (_directory, config, mut prepared) = fake_cli_prepared();
        prepared.request.options = json!({"ignored_option": true});
        assert_eq!(
            validated_submit_args(&config, &prepared).unwrap_err().code,
            "UNSUPPORTED_OPTION"
        );
        let (directory, config, mut prepared) = fake_cli_prepared();
        let reference = directory.path().join("reference.png");
        fs::write(&reference, b"original immutable reference").unwrap();
        prepared.request.references.push(MediaReference {
            media_id: "reference".into(),
            kind: "image".into(),
            path: reference.clone(),
            sha256: hex_hash(b"original immutable reference"),
            order: 1,
            usage: "reference".into(),
            duration_seconds: None,
        });
        fs::write(&reference, b"modified reference").unwrap();
        assert_eq!(
            validated_submit_args(&config, &prepared).unwrap_err().code,
            "MEDIA_CHANGED"
        );
    }
    fn auth_output(success: bool, stdout: Value, stderr: &str) -> CliOutput {
        CliOutput {
            success,
            signal: None,
            exit_code: Some(if success { 0 } else { 1 }),
            launch: "offline-launch".into(),
            stdout: serde_json::to_vec(&stdout).unwrap(),
            stderr: stderr.as_bytes().into(),
        }
    }
    fn windows_static(server: &str, bypass: &str) -> WindowsProxySettings {
        WindowsProxySettings {
            enabled: true,
            server: server.into(),
            bypass: bypass.into(),
            pac_present: false,
        }
    }
    #[test]
    fn proxy_parser_accepts_static_default_per_protocol_and_ipv6() {
        let default = parse_static_proxies("127.0.0.1:17890").unwrap();
        assert_eq!(default.http.as_deref(), Some("http://127.0.0.1:17890/"));
        assert_eq!(default.http, default.https);
        let mapped = parse_static_proxies("http=127.0.0.1:17891; https=https://proxy.example:18443; socks=ignored:18080; ftp=ignored:18081").unwrap();
        assert_eq!(mapped.http.as_deref(), Some("http://127.0.0.1:17891/"));
        assert_eq!(
            mapped.https.as_deref(),
            Some("https://proxy.example:18443/")
        );
        assert_eq!(
            parse_static_proxies("[::1]:17890")
                .unwrap()
                .https
                .as_deref(),
            Some("http://[::1]:17890/")
        );
        assert!(parse_static_proxies("socks=127.0.0.1:17890")
            .unwrap()
            .https
            .is_none());
    }
    #[test]
    fn proxy_parser_rejects_ambiguous_servers_credentials_urls_and_invalid_ports() {
        for invalid in [
            "",
            "http=a:1;HTTP=b:2",
            "a:1;b:2",
            "a:1;http=b:2",
            "unknown=a:1",
            "http=",
            "http://user:PRIVATE@proxy.example:8080",
            "http://@proxy.example:8080",
            "http://a.example:8080/path",
            "http://a.example:8080/?query=PRIVATE",
            "http://a.example:8080/#fragment",
            "file:///tmp/proxy",
            "socks5://proxy.example:8080",
            "http://proxy.example:0",
            "http://proxy.example:99999",
            "http://proxy.example:8080\n",
            "http://proxy.example:8080\0",
        ] {
            assert!(
                parse_static_proxies(invalid).is_none(),
                "malformed static configuration must not route requests"
            );
        }
    }
    #[test]
    fn proxy_environment_precedence_is_scheme_specific_and_preserves_explicit_bypass() {
        let explicit = ProxyEnvironment {
            http: Some("http://explicit.example:18080".into()),
            bypass: Some("exact-explicit-bypass".into()),
            ..Default::default()
        };
        let settings = windows_static(
            "http=system.example:18081;https=system.example:18082",
            "*.system.example;127.0.0.1",
        );
        let effective = effective_proxy_environment(explicit, Some(&settings));
        assert_eq!(
            effective.http.as_deref(),
            Some("http://explicit.example:18080")
        );
        assert_eq!(
            effective.https.as_deref(),
            Some("http://system.example:18082/")
        );
        assert_eq!(effective.bypass.as_deref(), Some("exact-explicit-bypass"));
    }
    #[test]
    fn explicit_all_proxy_wins_and_is_mapped_only_to_missing_cli_schemes() {
        let settings = windows_static("system.example:18082", "*.system.example");
        let explicit = ProxyEnvironment {
            http: Some("http://http-explicit.example:18080".into()),
            all: Some("http://all-explicit.example:18081".into()),
            ..Default::default()
        };
        let effective = effective_proxy_environment(explicit, Some(&settings));
        assert_eq!(
            effective.http.as_deref(),
            Some("http://http-explicit.example:18080")
        );
        assert_eq!(
            effective.https.as_deref(),
            Some("http://all-explicit.example:18081")
        );
        assert!(
            effective.bypass.is_none(),
            "system bypass cannot alter an entirely explicit route"
        );
        let empty = effective_proxy_environment(
            ProxyEnvironment {
                all: Some(String::new()),
                ..Default::default()
            },
            Some(&settings),
        );
        assert_eq!(empty.http.as_deref(), Some(""));
        assert_eq!(empty.https.as_deref(), Some(""));
    }
    #[test]
    fn explicit_empty_scheme_never_falls_back_to_system_proxy() {
        let settings = windows_static("system.example:18082", "*.system.example");
        let effective = effective_proxy_environment(
            ProxyEnvironment {
                http: Some(String::new()),
                https: Some(String::new()),
                bypass: Some(String::new()),
                ..Default::default()
            },
            Some(&settings),
        );
        assert_eq!(effective.http.as_deref(), Some(""));
        assert_eq!(effective.https.as_deref(), Some(""));
        assert_eq!(effective.bypass.as_deref(), Some(""));
    }
    #[test]
    fn disabled_pac_only_and_invalid_static_proxy_never_inject_guessed_routes() {
        for settings in [
            WindowsProxySettings {
                enabled: false,
                server: "stale.example:18080".into(),
                ..Default::default()
            },
            WindowsProxySettings {
                enabled: false,
                pac_present: true,
                ..Default::default()
            },
            WindowsProxySettings {
                enabled: true,
                pac_present: true,
                ..Default::default()
            },
            windows_static("http=bad.example:99999", "*"),
        ] {
            let effective =
                effective_proxy_environment(ProxyEnvironment::default(), Some(&settings));
            assert!(
                effective.http.is_none() && effective.https.is_none() && effective.bypass.is_none()
            );
        }
        let static_with_pac = WindowsProxySettings {
            pac_present: true,
            ..windows_static("127.0.0.1:17890", "")
        };
        assert_eq!(
            effective_proxy_environment(ProxyEnvironment::default(), Some(&static_with_pac))
                .https
                .as_deref(),
            Some("http://127.0.0.1:17890/")
        );
    }
    #[test]
    fn bypass_conversion_keeps_only_equivalent_ips_and_explicit_domain_suffixes() {
        assert_eq!(converted_windows_bypass("<local>;*.EXAMPLE.test;127.0.0.1;::1;*.EXAMPLE.test;exact.example.test;localhost;*;10.0.0.0/8;abc*;http://example.test;example.test:80").as_deref(), Some(".example.test,127.0.0.1,::1"));
        assert!(converted_windows_bypass("<local>;exact.example.test;*;10.0.0.0/8").is_none());
        assert!(converted_windows_bypass("*.example.test\n").is_none());
    }
    #[test]
    fn child_command_receives_effective_proxy_without_mutating_parent_environment() {
        let before: Vec<_> = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"]
            .into_iter()
            .map(|k| (k, std::env::var_os(k)))
            .collect();
        let environment = effective_proxy_environment(
            ProxyEnvironment::default(),
            Some(&windows_static("127.0.0.1:17890", "*.example.test")),
        );
        let mut command = Command::new("offline-not-executed");
        apply_child_proxy_environment(&mut command, &environment);
        let vars: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(
            vars.get("HTTP_PROXY").and_then(|v| v.as_deref()),
            Some("http://127.0.0.1:17890/")
        );
        assert_eq!(
            vars.get("HTTPS_PROXY").and_then(|v| v.as_deref()),
            Some("http://127.0.0.1:17890/")
        );
        assert_eq!(
            vars.get("NO_PROXY").and_then(|v| v.as_deref()),
            Some(".example.test")
        );
        for (key, value) in before {
            assert_eq!(std::env::var_os(key), value);
        }
    }
    #[test]
    fn launch_fingerprint_tracks_effective_child_proxy_and_cwd_without_leaking_values() {
        let direct = ProxyEnvironment::default();
        let proxy = effective_proxy_environment(
            direct.clone(),
            Some(&windows_static("proxy.example:17890", "*.example.test")),
        );
        let identities = [
            ("USERPROFILE", Some("private-user-directory".into())),
            ("HOME", None),
            ("DREAMINA_HOME", None),
        ];
        let original = launch_fingerprint_from(Some("C:\\launch".into()), &direct, &identities);
        let effective = launch_fingerprint_from(Some("C:\\launch".into()), &proxy, &identities);
        assert_ne!(original, effective);
        assert_eq!(effective.len(), 12);
        assert!(effective.bytes().all(|v| v.is_ascii_hexdigit()));
        assert_ne!(
            effective,
            launch_fingerprint_from(Some("C:\\other".into()), &proxy, &identities)
        );
        #[cfg(windows)]
        assert_eq!(
            effective,
            launch_fingerprint_from(Some("c:\\LAUNCH".into()), &proxy, &identities)
        );
        let mut output = auth_output(
            false,
            json!({}),
            "connection refused https://PRIVATE.example/token=PRIVATE",
        );
        output.launch = effective.clone();
        let error = output_error(&output, false);
        assert!(error.message.contains(&format!("launch={effective}")));
        for hidden in ["proxy.example", "private-user-directory", "PRIVATE"] {
            assert!(!error.message.contains(hidden));
        }
    }
    #[test]
    fn official_result_proxy_configuration_errors_are_sanitized() {
        let invalid = ProxyEnvironment {
            https: Some("http://PRIVATE@".into()),
            ..Default::default()
        };
        let error = result_proxies_from_environment(reqwest::blocking::Client::builder(), &invalid)
            .err()
            .unwrap();
        assert_eq!(error.code, "PROXY_CONFIGURATION");
        assert!(!error.message.contains("PRIVATE"));
        let configured = effective_proxy_environment(
            ProxyEnvironment::default(),
            Some(&windows_static("127.0.0.1:17890", "*.example.test")),
        );
        assert!(
            result_proxies_from_environment(reqwest::blocking::Client::builder(), &configured)
                .unwrap()
                .build()
                .is_ok()
        );
    }
    #[test]
    fn official_result_client_uses_the_selected_http_proxy_on_loopback_fixture() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            let mut socket = loop {
                match listener.accept() {
                    Ok((s, peer)) => {
                        assert!(peer.ip().is_loopback());
                        break s;
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => return false,
                }
            };
            // Winsock inherits the listener's nonblocking mode on accept. A
            // read timeout does not clear it, and TCP may split the headers.
            socket.set_nonblocking(false).unwrap();
            let read_deadline = Instant::now() + Duration::from_secs(2);
            let mut bytes = Vec::new();
            let mut chunk = [0; 512];
            loop {
                let remaining = read_deadline
                    .checked_duration_since(Instant::now())
                    .filter(|duration| !duration.is_zero())
                    .expect("loopback proxy request headers exceeded the existing two-second limit");
                socket.set_read_timeout(Some(remaining)).unwrap();
                let capacity = (8192 - bytes.len()).min(chunk.len());
                assert!(capacity > 0, "loopback proxy request headers exceeded 8 KiB");
                let count = match socket.read(&mut chunk[..capacity]) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    result => result.expect("read the bounded loopback proxy request headers"),
                };
                assert!(count > 0, "loopback proxy request headers ended before CRLFCRLF");
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|ending| ending == b"\r\n\r\n") {
                    break;
                }
            }
            let requested = String::from_utf8_lossy(&bytes)
                .starts_with("GET http://fixture.invalid/result ");
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
            requested
        });
        let env = effective_proxy_environment(
            ProxyEnvironment::default(),
            Some(&windows_static(&address.to_string(), "")),
        );
        let client = result_proxies_from_environment(
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(3))
                // A broken proxy selection must still stay on loopback. The
                // absolute request URI above distinguishes proxy from direct.
                .resolve("fixture.invalid", address),
            &env,
        )
        .unwrap()
        .build()
        .unwrap();
        let response = client.get("http://fixture.invalid/result").send();
        let requested = worker.join().unwrap();
        assert!(
            requested,
            "the loopback proxy must receive the fixture request; no public DNS is used"
        );
        assert_eq!(response.unwrap().text().unwrap(), "ok");
    }
    #[test]
    fn network_login_diagnostics_are_not_auth_failures_and_never_leak_raw_streams() {
        for (body, expected) in [
            ("authsdk login: dial tcp secret.invalid:443: connection refused token=PRIVATE https://secret.invalid/signed", "CLI_NETWORK"),
            ("login check: context deadline exceeded authorization=PRIVATE", "CLI_QUERY_TIMEOUT"),
            ("authsdk: TLS handshake timeout access_token=PRIVATE", "CLI_QUERY_TIMEOUT"),
        ] {
            let output = auth_output(false, json!({"private":"PRIVATE"}), body);
            let read = output_error(&output, false);
            assert_eq!(read.code, expected);
            assert!(!read.auth_required);
            assert!(!read.submission_unknown);
            assert!(read.message.contains("CLI退出码=1"));
            assert!(!read.message.contains("PRIVATE"));
            assert!(!read.message.contains("secret.invalid"));
            assert!(!read.message.contains("https://"));
            let submit = output_error(&output, true);
            assert!(submit.submission_unknown, "a failed transport cannot confirm a submit was rejected");
            assert!(!submit.auth_required);
        }
    }
    #[test]
    fn safe_cli_failure_keeps_flag_and_model_semantics_and_original_submit_policy() {
        let output = auth_output(
            false,
            json!({}),
            "Error: invalid --resolution_type=2k for model_version=5.0; required image input",
        );
        for submitting in [false, true] {
            let original = classified_output_error(&output, submitting);
            let actual = output_error_with_redactions(&output, submitting, &[]);
            assert_eq!(actual.code, original.code);
            assert_eq!(actual.auth_required, original.auth_required);
            assert_eq!(actual.submission_unknown, original.submission_unknown);
            assert!(actual.message.starts_with(&original.message));
            assert!(actual.message.contains("invalid --resolution_type=2k"));
            assert!(actual.message.contains("model_version=5.0"));
            assert!(actual.message.contains("required image input"));
        }
        for body in [
            "unknown flag: --unsupported",
            "context deadline exceeded",
            "HTTP 401 unauthorized",
        ] {
            let output = auth_output(false, json!({}), body);
            let before = classified_output_error(&output, true);
            let after = output_error_with_redactions(&output, true, &[]);
            assert_eq!(
                (after.code, after.auth_required, after.submission_unknown),
                (before.code, before.auth_required, before.submission_unknown)
            );
        }
    }

    #[test]
    fn safe_cli_failure_redacts_full_configured_secrets_and_prompt_before_truncation() {
        let secret = format!("ConfiguredPrivate{}EndPrivate", "LongValue".repeat(800));
        let prompt = "PRIVATE 原完整提示词含文字和 C:\\PrivateProject\\reference.png。";
        let raw = format!(
            "Error: rejected value {secret}\nError: input creation failed {prompt}\n{}",
            "Error: unsupported value\n".repeat(200)
        );
        let summary =
            safe_failure_stderr(raw.as_bytes(), &[secret.clone(), prompt.into()]).unwrap();
        for private in [
            &secret[..40],
            "EndPrivate",
            prompt,
            "PrivateProject",
            "PRIVATE",
        ] {
            assert!(
                !summary.contains(private),
                "a raw truncation or fragmented literal must never be published"
            );
        }
        assert!(summary.contains("Error: rejected value [redacted]"));
        assert!(summary.chars().count() <= 1001);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn safe_cli_failure_removes_urls_jwts_headers_and_escaped_private_values() {
        let raw=concat!(
            "Error: upload failed https://private.invalid/path?signature=PRIVATE_SIG&token=PRIVATE_TOKEN\n",
            "Error: request failed https:\\/\\/private.invalid/path?X-Amz-Signature=PRIVATE_SIG\n",
            "Error: request failed abcdefghijklmnop.abcdefghijklmnop.abcdefghijklmnop\n",
            "Authorization: Bearer PRIVATE_TOKEN\nPRIVATE_CONTINUATION_TOKEN\n",
            "Error: token=PRIVATE_TOKEN cookie=PRIVATE_COOKIE\n",
            "Error: rejected prompt=PRIVATE_PROMPT_SNIPPET\n",
            "Error: invalid escaped value %50%52%49%56%41%54%45\n",
            "Error: invalid escaped value \\u0050\\u0052\\u0049\\u0056\n"
        );
        let summary = safe_failure_stderr(raw.as_bytes(), &[]).unwrap();
        for private in [
            "private.invalid",
            "signature=",
            "PRIVATE",
            "abcdefghijklmnop",
            "https:",
            "%50",
            "\\u0050",
        ] {
            assert!(!summary.contains(private));
        }
        assert!(summary.contains("upload failed [media-url]"));
        assert!(summary.contains("此行未安全披露"));
    }

    #[test]
    fn safe_cli_failure_hides_complete_private_paths_without_leaking_root_suffixes() {
        let raw = concat!(
            "Error: open C:\\Users\\PrivateUser\\AppData\\project-private\\reference.png failed\n",
            "Error: open \\\\private-server\\private-share\\reference.png failed\n",
            "Error: open /Users/PrivateUser/project-private/reference.png failed\n",
            "Error: open ./reference-001.png failed\n",
            "Error: image upload failed private.invalid:443\n"
        );
        let summary = safe_failure_stderr(
            raw.as_bytes(),
            &["C:\\Users\\PrivateUser".into(), "/Users/PrivateUser".into()],
        )
        .unwrap();
        for private in [
            "PrivateUser",
            "AppData",
            "project-private",
            "private-server",
            "private-share",
            "reference-001.png",
            "private.invalid",
        ] {
            assert!(!summary.contains(private));
        }
        assert!(summary.contains("[private-path]"));
        assert!(summary.contains("[reference-file]"));
        assert!(summary.contains("[network-address]"));
    }

    #[test]
    fn safe_cli_failure_never_copies_stdout_or_untrusted_binary_control_lines() {
        let output = auth_output(
            false,
            json!({"access_token":"PRIVATE_STDOUT"}),
            "Error: invalid input",
        );
        let error = output_error_with_redactions(&output, true, &[]);
        assert!(error.message.contains("Error: invalid input"));
        assert!(!error.message.contains("PRIVATE_STDOUT"));
        let empty = auth_output(false, json!({"private":"PRIVATE_STDOUT"}), "");
        assert!(!output_error_with_redactions(&empty, true, &[])
            .message
            .contains("stderr摘要"));
        assert_eq!(
            safe_failure_stderr(&[0xff, 0xfe], &[]).unwrap(),
            "[stderr 内容未安全披露]"
        );
        for raw in [
            "Error: invalid \u{1b}UNHANDLED_PRIVATE",
            "Error: invalid \u{202e}BIDI_PRIVATE",
            "{\"error\":\"PRIVATE_JSON\"}",
        ] {
            let summary = safe_failure_stderr(raw.as_bytes(), &[]).unwrap();
            assert!(!summary.contains("PRIVATE"));
            assert!(summary.contains("未安全披露"));
        }
    }

    #[test]
    fn safe_cli_invocation_redactions_include_configured_values_and_original_arguments() {
        let config = ProviderConfig {
            kind: "dreamina".into(),
            endpoint: Some("https://private.invalid/PRIVATE_ENDPOINT".into()),
            credential_env: Some("PATH".into()),
            cli_path: Some(PathBuf::from("C:\\PrivateProject\\dreamina.exe")),
            bridge_token_env: None,
            chrome_binding: None,browser_transport:None,
            timeout_seconds: 60,
            adapted_models: vec![],
        };
        let args = vec![
            "image2image".into(),
            "--prompt=PRIVATE_EXACT_PROMPT".into(),
            "--device_code".into(),
            "PRIVATE_DEVICE".into(),
        ];
        let values = failure_redactions(
            Some(&config),
            &args,
            Some(Path::new("C:\\PrivateTransport")),
        );
        for value in [
            "PRIVATE_EXACT_PROMPT",
            "PRIVATE_DEVICE",
            "https://private.invalid/PRIVATE_ENDPOINT",
            "C:\\PrivateTransport",
        ] {
            assert!(values.iter().any(|v| v == value));
        }
        if let Ok(value) = std::env::var("PATH") {
            assert!(values.contains(&value));
        }
        let output = auth_output(false, json!({}), "Error: rejected PRIVATE_EXACT_PROMPT");
        let error = output_error_for_invocation(
            &output,
            true,
            &config,
            &args,
            Some(Path::new("C:\\PrivateTransport")),
        );
        assert!(!error.message.contains("PRIVATE_EXACT_PROMPT"));
        assert!(error.message.contains("rejected [redacted]"));
        assert!(error.submission_unknown);
    }
    #[test]
    fn explicit_authentication_failure_pauses_but_generic_login_advice_does_not() {
        for message in [
            "HTTP 401 unauthorized",
            "not logged in",
            "token expired",
            "please run dreamina login",
        ] {
            assert!(output_error(&auth_output(false, json!({}), message), false).auth_required);
        }
        let generic = output_error(
            &auth_output(
                false,
                json!({}),
                "command failed; see login documentation token=PRIVATE",
            ),
            false,
        );
        assert_eq!(generic.code, "CLI_QUERY_FAILED");
        assert!(!generic.auth_required);
        assert!(!generic.message.contains("PRIVATE"));
        assert!(
            output_error(&auth_output(false, json!({}), "unknown CLI failure"), true)
                .submission_unknown
        );
    }
    #[test]
    fn account_identity_hash_and_credit_allowlist_are_stable() {
        let body = json!({"user_id":"123456789","total_credit":14414,"vip_level":"maestro","access_token":"must-not-escape","refresh_token":"also-secret","user_name":"private-name"});
        let first = account_snapshot(&body);
        let second = account_snapshot(
            &json!({"data":{"user_id":123456789,"total_credit":0,"access_token":"rotated"}}),
        );
        assert_eq!(first["accountKey"], second["accountKey"]);
        assert_eq!(first["credits"]["total"], 14414.);
        assert_eq!(second["credits"]["total"], 0.);
        assert_eq!(first["ready"], true);
        let output = first.to_string();
        for secret in [
            "123456789",
            "must-not-escape",
            "also-secret",
            "private-name",
            "access_token",
            "refresh_token",
        ] {
            assert!(!output.contains(secret));
        }
        assert!(first["credits"]["member"].is_null());
    }
    #[test]
    fn credit_without_identity_does_not_enable_submission() {
        for body in [
            json!({"total_credit":14414}),
            json!({"user_id":"","total_credit":14414}),
            json!({"user_id":0}),
            json!({"user_id":"0"}),
        ] {
            let snapshot = account_snapshot(&body);
            assert_eq!(snapshot["ready"], false);
            assert_eq!(snapshot["identityVerified"], false);
            assert!(snapshot.get("accountKey").is_none());
        }
    }
    #[test]
    fn official_anonymous_account_is_logged_out_without_mistaking_zero_credit_accounts() {
        let anonymous = account_snapshot(&json!({"data":{"user_id":0,"user_name":"","total_credit":0,"vip_level":""}}));
        assert_eq!(anonymous["status"], "logged_out");
        assert_eq!(anonymous["ready"], false);
        assert!(anonymous.get("accountKey").is_none());
        let real = account_snapshot(&json!({"user_id":123,"user_name":"","total_credit":0}));
        assert_eq!(real["status"], "authenticated");
        assert_eq!(real["ready"], true);
        assert_eq!(account_snapshot(&json!({"total_credit":0}))["status"], "unknown");
    }
    #[test]
    fn explicit_anonymous_login_requests_fresh_oauth_without_generation() {
        let mut account_reads = 0;
        let mut login_starts = 0;
        let result = begin_login_with(
            || {
                account_reads += 1;
                Ok(account_snapshot(&json!({"user_id":0,"user_name":"","total_credit":0})))
            },
            |fresh| {
                assert!(fresh, "anonymous account must not reuse a stale local token");
                login_starts += 1;
                Ok(auth_output(true, json!({"device_code":"memory-only","verification_uri":"https://example.test/oauth","user_code":"ABCD","expires_in":600,"access_token":"must-not-escape"}), ""))
            },
        ).unwrap();
        assert_eq!(account_reads, 0);
        assert_eq!(login_starts, 1);
        assert_eq!(result["device_code"], "memory-only");
        assert_eq!(result["verification_uri"], "https://example.test/oauth");
        assert!(!result.to_string().contains("must-not-escape"));
    }
    #[test]
    fn explicit_oauth_is_independent_of_stale_or_unreachable_account_status() {
        let result = begin_login_with(
            || panic!("explicit OAuth must not depend on an account preflight"),
            |fresh| {
                assert!(fresh);
                Ok(auth_output(true, json!({"device_code":"new-challenge","verification_uri":"https://jimeng.jianying.com/ai-tool/cli-auth"}), ""))
            },
        ).unwrap();
        assert_eq!(result["device_code"], "new-challenge");
    }
    #[test]
    fn login_without_challenge_rechecks_identity_instead_of_trusting_exit_success() {
        let mut reads = 0;
        let result = begin_login_with(
            || {
                reads += 1;
                Ok(account_snapshot(&json!({"user_id":123,"total_credit":10})))
            },
            |_| Ok(auth_output(true, json!({"access_token":"must-not-escape"}), "")),
        ).unwrap();
        assert_eq!(reads, 1);
        assert_eq!(result["ready"], true);
        assert_eq!(result["credits"]["total"], 10.);
        assert!(!result.to_string().contains("must-not-escape"));
        let error = begin_login_with(
            || Ok(account_snapshot(&json!({"user_id":0,"total_credit":0}))),
            |_| Ok(auth_output(true, json!({}), "cached token reused")),
        ).unwrap_err();
        assert_eq!(error.code, "CLI_AUTH_CHALLENGE_MISSING");
    }
    #[test]
    fn oauth_material_is_strictly_allowlisted() {
        let value=login_material(&json!({"data":{"device_code":"device-only-memory","user_code":"ABCD","verification_uri":"https://example.test/login","verification_uri_complete":"https://example.test/complete?user_code=ABCD","expires_in":600,"interval":5,"access_token":"hidden","refresh_token":"hidden2","user_id":"not-allowed"}})).unwrap();
        assert_eq!(
            value["verification_uri"],
            "https://example.test/complete?user_code=ABCD"
        );
        assert_eq!(value["device_code"], "device-only-memory");
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert!(!value.to_string().contains("token"));
        let camel = login_material(
            &json!({"deviceCode":"abc","userCode":"XYZ","verification_url":"https://example.test"}),
        )
        .unwrap();
        assert_eq!(camel["user_code"], "XYZ");
    }
    #[test]
    fn present_credit_components_preserve_zero_and_missing_stay_unknown() {
        let snapshot = account_snapshot(
            &json!({"user_id":"credit-user","total_credit":100,"vip_credit":0,"gift_credit":"10","purchase_credit":null}),
        );
        assert_eq!(snapshot["credits"]["member"], 0.);
        assert_eq!(snapshot["credits"]["gift"], 10.);
        assert!(snapshot["credits"]["purchase"].is_null());
    }
    #[test]
    fn oauth_pending_refusal_and_expiry_are_distinct() {
        let pending = login_result(&auth_output(
            false,
            json!({"error":"authorization_pending","access_token":"never-return"}),
            "wait",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(pending["status"], "pending");
        assert_eq!(pending["ready"], false);
        assert!(!pending.to_string().contains("never-return"));
        assert_eq!(
            login_result(&auth_output(false, json!({"error":"access_denied"}), ""))
                .unwrap_err()
                .code,
            "CLI_AUTH_DENIED"
        );
        assert_eq!(
            login_result(&auth_output(false, json!({"error":"expired_token"}), ""))
                .unwrap_err()
                .code,
            "CLI_AUTH_EXPIRED"
        );
        assert!(
            login_result(&auth_output(true, json!({"access_token":"discarded"}), ""))
                .unwrap()
                .is_none()
        );
        assert!(login_result(&auth_output(false, json!({"error":"unrecognized"}), "")).is_err());
    }
    #[test]
    fn prefixed_ansi_oauth_json_is_read_without_returning_prefix() {
        let json=auth_json(b"\x1b[32mOAuth ready\x1b[0m\n{\"device_code\":\"abc\",\"verification_uri\":\"https://example.test\"}\n").unwrap();
        assert_eq!(login_material(&json).unwrap()["device_code"], "abc");
    }
    #[test]
    fn official_plain_label_oauth_output_is_supported_and_allowlisted() {
        let output=b"\x1b[32mverification_uri: https://example.test/oauth\x1b[0m\nuser_code: ABCD\ndevice_code: internal-only\nexpires_in=600\ninterval: 5\naccess_token: must-not-return\n";
        let body = oauth_material_body(output);
        let material = login_material(&body).unwrap();
        assert_eq!(material["verification_uri"], "https://example.test/oauth");
        assert_eq!(material["user_code"], "ABCD");
        assert_eq!(material["device_code"], "internal-only");
        assert_eq!(material["expires_in"], 600);
        assert!(!material.to_string().contains("must-not-return"));
        assert_eq!(
            login_result(&auth_output(false, json!({}), "authsdk: login denied"))
                .unwrap_err()
                .code,
            "CLI_AUTH_DENIED"
        );
    }
    #[test]
    fn custom_1080_image_wire_has_dimensions_and_no_ratio_or_1080_enum() {
        let caps = image_caps();
        let expected = [
            ("21:9", 2520, 1080),
            ("16:9", 1920, 1080),
            ("3:2", 1620, 1080),
            ("4:3", 1440, 1080),
            ("1:1", 1080, 1080),
            ("3:4", 1080, 1440),
            ("2:3", 1080, 1620),
            ("9:16", 1080, 1920),
        ];
        for (ratio, width, height) in expected {
            let prepared =
                prepare_with_capabilities(&image_request("5.0", ratio, "1080p"), &caps).unwrap();
            let args = prepared.wire["args"].as_array().unwrap();
            assert!(args.contains(&json!(format!("--width={width}"))));
            assert!(args.contains(&json!(format!("--height={height}"))));
            assert!(args.contains(&json!("--resolution_type=2k")));
            assert!(!args
                .iter()
                .any(|x| x.as_str().unwrap().starts_with("--ratio=")));
            assert!(!args.contains(&json!("--resolution_type=1080p")));
            assert_eq!(prepared.request.resolution, "1080p");
        }
        assert!(prepare_with_capabilities(&image_request("5.0", "7:3", "1080p"), &caps).is_err());
    }
    #[test]
    fn image_native_sizes_are_model_specific() {
        let caps = image_caps();
        assert!(prepare_with_capabilities(&image_request("3.0", "16:9", "4k"), &caps).is_err());
        assert!(prepare_with_capabilities(&image_request("5.0", "16:9", "1.5k"), &caps).is_err());
        let native =
            prepare_with_capabilities(&image_request("5.0Pro", "9:16", "4k"), &caps).unwrap();
        let args = native.wire["args"].as_array().unwrap();
        assert!(args.contains(&json!("--ratio=9:16")));
        assert!(args.contains(&json!("--resolution_type=4k")));
        assert!(!args
            .iter()
            .any(|x| x.as_str().unwrap().starts_with("--width=")));
        let insufficient = IMAGE_HELP.replace(
            "2k: each side 768-3072; total pixels <= 4194304",
            "custom limits unknown",
        );
        let unsupported = capabilities_from_help(b"{}", "", "", &insufficient, EDIT_HELP).unwrap();
        assert!(unsupported
            .models
            .iter()
            .all(|model| !model.resolutions.contains(&"1080p".into())));
    }
    #[test]
    fn accepted_is_not_finished() {
        let q = normalized(
            json!({"submit_id":"known","gen_status":"querying","result_url":"https://ignored"}),
            Some("known"),
        )
        .unwrap();
        assert_eq!(q.status, "processing");
        assert!(q.result_urls.is_empty());
    }
    #[test]
    fn official_image_result_json_keeps_original_urls_order_and_dimensions() {
        // Structure confirmed from an existing task; all identities/URLs here are synthetic.
        let body = json!({
            "submit_id":"image-task", "gen_status":"success", "queue_info":{"queue_idx":0},
            "result_json":{"images":[
                {"image_url":"https://example.test/first.png?signature=A%2FB&expires=123","width":2560,"height":1440},
                {"image_url":"https://example.test/second.png","width":2560,"height":1440},
                {"image_url":"https://example.test/first.png?signature=A%2FB&expires=123"}
            ],"videos":[]}
        });
        let query = normalized(body, Some("image-task")).unwrap();
        assert_eq!(query.status, "succeeded");
        assert_eq!(
            query.result_urls,
            vec![
                "https://example.test/first.png?signature=A%2FB&expires=123",
                "https://example.test/second.png"
            ]
        );
        assert_eq!(query.raw["result_json"]["images"][0]["width"], 2560);
        assert_eq!(query.raw["result_json"]["images"][0]["height"], 1440);
    }
    #[test]
    fn video_result_json_uses_video_url_then_record_url_without_covers() {
        let query = normalized(json!({
            "data":{"submit_id":"video-task","gen_status":"success","result_json":{"videos":[
                {"video_url":"https://example.test/original1.mp4","url":"https://example.test/ambiguous-preview.mp4","image_url":"https://example.test/cover.png","thumbnail_url":"https://example.test/thumb.png"},
                {"url":"https://example.test/original2.mp4","cover_url":"https://example.test/cover2.png"}
            ]},"help":{"url":"https://example.test/help"},"reference":{"video_url":"https://example.test/reference.mp4"}}
        }), Some("video-task")).unwrap();
        assert_eq!(query.task_id, "video-task");
        assert_eq!(query.status, "succeeded");
        assert_eq!(
            query.result_urls,
            vec![
                "https://example.test/original1.mp4",
                "https://example.test/original2.mp4"
            ]
        );
    }
    #[test]
    fn result_string_arrays_are_allowed_only_in_output_containers() {
        let query = normalized(json!({
            "submit_id":"array-task","gen_status":"success",
            "result_json":{"images":["https://example.test/first.png"],"image_urls":["https://example.test/second.png"],"video_urls":["https://example.test/third.mp4"]},
            "result_urls":["https://example.test/fourth.mp4"],
            "input":{"images":["https://example.test/reference.png"]},
            "help_urls":["https://example.test/help"]
        }), Some("array-task")).unwrap();
        assert_eq!(
            query.result_urls,
            vec![
                "https://example.test/first.png",
                "https://example.test/second.png",
                "https://example.test/third.mp4",
                "https://example.test/fourth.mp4"
            ]
        );
    }
    #[test]
    fn pending_failed_and_unknown_tasks_never_collect_temporary_output() {
        for (status, normalized_status) in [
            ("querying", "processing"),
            ("fail", "failed"),
            ("new-upstream-state", "unknown"),
        ] {
            let result = normalized(json!({
                "submit_id":"original-task","gen_status":status,
                "result_json":{"images":[{"image_url":"https://example.test/temporary.png"}],"videos":[{"video_url":"https://example.test/temporary.mp4"}]}
            }), Some("original-task")).unwrap();
            assert_eq!(result.task_id, "original-task");
            assert_eq!(result.status, normalized_status);
            assert!(result.result_urls.is_empty());
        }
    }
    #[test]
    fn success_without_output_is_not_inferred_expired_and_preview_fields_are_ignored() {
        let result = normalized(json!({
            "submit_id":"known","gen_status":"success","url":"https://example.test/help",
            "queue_info":{"url":"https://example.test/queue"},
            "result_json":{"images":[{"thumbnail_url":"https://example.test/thumb.png","cover_url":"https://example.test/cover.png"}],"videos":[],"help":{"url":"https://example.test/help"},"request":{"images":[{"image_url":"https://example.test/reference.png"}]}},
            "input":{"images":["https://example.test/reference2.png"]}
        }), Some("known")).unwrap();
        assert_eq!(result.status, "succeeded");
        assert!(result.result_urls.is_empty());
        assert_eq!(result.raw["gen_status"], "success");
    }
    #[test]
    fn malformed_insecure_or_credential_urls_are_not_results() {
        let result = normalized(
            json!({
                "submit_id":"known","gen_status":"success","result_json":{"images":[
                    {"image_url":"http://example.test/output.png"},
                    {"image_url":"https://user:password@example.test/output.png"},
                    {"image_url":"https://"},
                    {"image_url":"https://example.test/out\nput.png"},
                    {"image_url":"not a URL"}
                ]}
            }),
            Some("known"),
        )
        .unwrap();
        assert!(result.result_urls.is_empty());
        let malformed_containers = normalized(
            json!({"submit_id":"known","gen_status":"success",
                "result_json":"https://example.test/not-an-output-object",
                "image_urls":"https://example.test/not-an-output-array"}),
            Some("known"),
        )
        .unwrap();
        assert!(malformed_containers.result_urls.is_empty());
    }
    #[test]
    fn malformed_explicit_query_identity_never_falls_back_to_expected_id() {
        for id in [
            json!(123),
            json!(""),
            json!("invalid task id"),
            json!(["known"]),
        ] {
            let result = normalized(
                json!({"submit_id":id,"gen_status":"success"}),
                Some("known"),
            );
            assert_eq!(result.unwrap_err().code, "TASK_ID_INVALID");
        }
    }
    #[test]
    fn image_and_video_submit_receipts_use_the_same_strict_result_contract() {
        let (_directory, config, prepared) = fake_cli_prepared();
        let image = submit_with_runner(&config, &prepared, |args| {
            assert_eq!(args[0], "text2image");
            Ok(serde_json::to_vec(&json!({"submit_id":"image-id","gen_status":"success","result_json":{"images":[{"image_url":"https://example.test/original.png"}]}})).unwrap())
        }).unwrap();
        assert_eq!(image.status, "succeeded");
        assert_eq!(image.result_urls, vec!["https://example.test/original.png"]);
        let mut request = image_request("seedance2.5", "16:9", "720p");
        request.kind = "video".into();
        request.duration = Some(5);
        let caps = capabilities_from_help(b"{}", VIDEO_HELP, TEXT_VIDEO_HELP, "", "").unwrap();
        let mut prepared = prepare_with_capabilities(&request, &caps).unwrap();
        prepared.wire["cliIdentity"] = cli_identity(&config).unwrap();
        let video = submit_with_runner(&config, &prepared, |args| {
            assert_eq!(args[0], "text2video");
            Ok(serde_json::to_vec(&json!({"submit_id":"video-id","gen_status":"success","result_json":{"videos":[{"video_url":"https://example.test/original.mp4"}]}})).unwrap())
        }).unwrap();
        assert_eq!(video.status, "succeeded");
        assert_eq!(video.result_urls, vec!["https://example.test/original.mp4"]);
        let unknown = submit_with_runner(&config, &prepared, |_| {
            Ok(br#"{"submit_id":123,"gen_status":"querying"}"#.to_vec())
        })
        .unwrap_err();
        assert!(unknown.submission_unknown);
    }
    fn mixed_output_fixture() -> Value {
        // Deliberately misleading suffixes: selection follows typed containers,
        // never the URL extension, which may also be absent on signed CDN URLs.
        json!({"submit_id":"mixed-task","gen_status":"success","result_json":{
            "images":[{"image_url":"https://example.test/image-as.mp4"},{"url":"https://example.test/second-image"}],
            "videos":[{"video_url":"https://example.test/video-as.png"},{"url":"https://example.test/second-video"}]
        }})
    }
    #[test]
    fn typed_results_filter_mixed_containers_without_guessing_url_extensions() {
        let image =
            normalized_for_kind(mixed_output_fixture(), Some("mixed-task"), Some("image")).unwrap();
        let video =
            normalized_for_kind(mixed_output_fixture(), Some("mixed-task"), Some("video")).unwrap();
        assert_eq!(
            image.result_urls,
            vec![
                "https://example.test/image-as.mp4",
                "https://example.test/second-image"
            ]
        );
        assert_eq!(
            video.result_urls,
            vec![
                "https://example.test/video-as.png",
                "https://example.test/second-video"
            ]
        );
        let typed_fields = json!({"submit_id":"mixed-task","gen_status":"success","results":[
            {"image_url":"https://example.test/image", "result_url":"https://example.test/image-alias", "url":"https://example.test/image-preview"},
            {"video_url":"https://example.test/video", "download_url":"https://example.test/video-alias", "url":"https://example.test/video-preview"}
        ]});
        assert_eq!(
            normalized_for_kind(typed_fields.clone(), Some("mixed-task"), Some("image"))
                .unwrap()
                .result_urls,
            vec!["https://example.test/image"]
        );
        assert_eq!(
            normalized_for_kind(typed_fields, Some("mixed-task"), Some("video"))
                .unwrap()
                .result_urls,
            vec!["https://example.test/video"]
        );
    }
    #[test]
    fn prepared_query_and_submit_apply_the_frozen_media_kind() {
        let (_directory, config, mut prepared) = fake_cli_prepared();
        let query = query_prepared_with_runner(&prepared, "mixed-task", || {
            Ok(serde_json::to_vec(&mixed_output_fixture()).unwrap())
        })
        .unwrap();
        let submit = submit_with_runner(&config, &prepared, |_| {
            Ok(serde_json::to_vec(&mixed_output_fixture()).unwrap())
        })
        .unwrap();
        let images = vec![
            "https://example.test/image-as.mp4",
            "https://example.test/second-image",
        ];
        assert_eq!(query.result_urls, images);
        assert_eq!(submit.result_urls, images);
        let caps = capabilities_from_help(b"{}", VIDEO_HELP, TEXT_VIDEO_HELP, "", "").unwrap();
        let mut request = image_request("seedance2.5", "16:9", "720p");
        request.kind = "video".into();
        request.duration = Some(5);
        prepared = prepare_with_capabilities(&request, &caps).unwrap();
        prepared.wire["cliIdentity"] = cli_identity(&config).unwrap();
        let query = query_prepared_with_runner(&prepared, "mixed-task", || {
            Ok(serde_json::to_vec(&mixed_output_fixture()).unwrap())
        })
        .unwrap();
        let submit = submit_with_runner(&config, &prepared, |_| {
            Ok(serde_json::to_vec(&mixed_output_fixture()).unwrap())
        })
        .unwrap();
        let videos = vec![
            "https://example.test/video-as.png",
            "https://example.test/second-video",
        ];
        assert_eq!(query.result_urls, videos);
        assert_eq!(submit.result_urls, videos);
        prepared.request.kind = "audio".into();
        let mut called = false;
        assert_eq!(
            query_prepared_with_runner(&prepared, "mixed-task", || {
                called = true;
                Ok(vec![])
            })
            .unwrap_err()
            .code,
            "INVALID_RESULT_KIND"
        );
        assert!(!called);
    }
    #[test]
    fn task_identity_aliases_must_match_each_other_and_the_original_task() {
        for alias in ["submit_id", "submitId", "task_id", "taskId"] {
            let mut body = json!({"gen_status":"querying"});
            body[alias] = json!("other");
            assert_eq!(
                normalized(body.clone(), Some("original")).unwrap_err().code,
                "TASK_MISMATCH"
            );
            body[alias] = json!("original");
            assert_eq!(
                normalized(body, Some("original")).unwrap().task_id,
                "original"
            );
        }
        let matching = json!({"submit_id":"original","submitId":"original","task_id":"original","taskId":"original","data":{"submit_id":"original","gen_status":"querying"}});
        assert_eq!(
            normalized(matching, Some("original")).unwrap().task_id,
            "original"
        );
        for conflict in [
            json!({"submit_id":"original","task_id":"other","gen_status":"success"}),
            json!({"submit_id":"original","data":{"taskId":"other","gen_status":"success"}}),
        ] {
            assert_eq!(
                normalized(conflict, Some("original")).unwrap_err().code,
                "TASK_ID_CONFLICT"
            );
        }
        assert_eq!(
            normalized(
                json!({"submit_id":"original","task_id":123}),
                Some("original")
            )
            .unwrap_err()
            .code,
            "TASK_ID_INVALID"
        );
    }
    #[test]
    fn conflicting_submit_aliases_remain_unknown_and_queries_never_submit() {
        let (_directory, config, prepared) = fake_cli_prepared();
        let error = submit_with_runner(&config, &prepared, |_| {
            Ok(br#"{"submit_id":"original","task_id":"other","gen_status":"success"}"#.to_vec())
        })
        .unwrap_err();
        assert!(error.submission_unknown);
        let mut calls = 0;
        let result = query_with_runner("mixed-task", Some("video"), || {
            calls += 1;
            Ok(serde_json::to_vec(&mixed_output_fixture()).unwrap())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(result.task_id, "mixed-task");
        assert_eq!(
            result.result_urls,
            vec![
                "https://example.test/video-as.png",
                "https://example.test/second-video"
            ]
        );
    }
    #[test]
    fn success_extracts_only_result_fields() {
        let q=normalized(json!({"submit_id":"known","gen_status":"success","result_data":[{"video_url":"https://example.test/out.mp4"}],"request":{"images":["https://example.test/reference.png"]}}),Some("known")).unwrap();
        assert_eq!(q.result_urls, vec!["https://example.test/out.mp4"]);
    }
    #[test]
    fn other_task_fails() {
        assert!(normalized(
            json!({"submit_id":"other","gen_status":"success"}),
            Some("known")
        )
        .is_err());
    }
    #[test]
    fn exact_model_tokens() {
        assert!(!has_token("seedance2.0fast_vip", "seedance2.0"));
        assert!(has_token("seedance2.0, seedance2.5", "seedance2.0"));
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires independently downloaded official CLI; help-only inspection"]
    fn installed_official_cli_help_contract() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tools/dreamina/1.4.19/dreamina.exe");
        let config = ProviderConfig {
            kind: "dreamina".into(),
            endpoint: None,
            credential_env: None,
            cli_path: Some(path),
            bridge_token_env: None,
            chrome_binding: None,browser_transport:None,
            timeout_seconds: 20,
            adapted_models: vec![],
        };
        let caps = capabilities(&config).unwrap();
        let video = caps.models.iter().find(|m| m.id == "seedance2.5").unwrap();
        assert!(video.adapted);
        assert_eq!(
            (video.images, video.videos, video.audios, video.max_duration),
            (30, 10, 10, Some(30))
        );
        assert!(caps
            .models
            .iter()
            .any(|m| m.id == "5.0" && m.kind == "image" && m.adapted));
        let image = caps.models.iter().find(|m| m.id == "5.0").unwrap();
        assert_eq!(image.ratios.len(), 8);
        assert_eq!(image.resolutions, vec!["1080p", "2k", "4k"]);
        let fast = caps
            .models
            .iter()
            .find(|m| m.id == "seedance2.0fast")
            .unwrap();
        assert_eq!(fast.resolutions, vec!["720p"]);
        let vip = caps
            .models
            .iter()
            .find(|m| m.id == "seedance2.0_vip")
            .unwrap();
        assert_eq!(vip.resolutions, vec!["720p", "1080p", "4k"]);
        for id in [
            "seedance2.0",
            "seedance2.0fast",
            "seedance2.0_vip",
            "seedance2.0fast_vip",
            "seedance2.0mini",
            "seedance2.5",
        ] {
            let model = caps.models.iter().find(|m| m.id == id).unwrap();
            assert!(model.adapted, "{id}");
            assert_eq!(model.ratios.len(), 6, "{id}");
            let expected: Vec<&str> = match id {
                "seedance2.5" => vec!["480p", "720p", "1080p"],
                "seedance2.0_vip" => vec!["720p", "1080p", "4k"],
                _ => vec!["720p"],
            };
            assert_eq!(model.resolutions, expected, "{id}");
        }
        let mut prepared =
            prepare_with_capabilities(&image_request("5.0", "16:9", "1080p"), &caps).unwrap();
        prepared.wire["cliIdentity"] = cli_identity(&config).unwrap();
        assert!(validated_submit_args(&config, &prepared).is_ok());
        assert!(prepared.wire["args"]
            .as_array()
            .unwrap()
            .contains(&json!("--width=1920")));
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "read-only current official CLI account check; requires an existing authorized session"]
    fn installed_official_cli_account_readonly_contract() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tools/dreamina/1.4.19/dreamina.exe");
        let config = ProviderConfig {
            kind: "dreamina".into(),
            endpoint: None,
            credential_env: None,
            cli_path: Some(path),
            bridge_token_env: None,
            chrome_binding: None,browser_transport:None,
            timeout_seconds: 20,
            adapted_models: vec![],
        };
        let result = account(&config).unwrap();
        assert_eq!(result["ready"], true);
        assert_eq!(result["identityVerified"], true);
        assert!(result["credits"]["total"].as_f64().is_some());
        assert!(result["accountKey"]
            .as_str()
            .unwrap()
            .starts_with("dreamina:official:user:"));
        assert!(result.get("user_id").is_none());
        assert!(result.get("access_token").is_none());
    }
}
