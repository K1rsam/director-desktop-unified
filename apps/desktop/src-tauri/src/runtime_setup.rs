//! Explicit connection setup. Discover native programs, then install verified,
//! pinned official releases in the application home only when genuinely absent.
//! No shell, npm scripts, account operations, model requests, or settings writes.
use crate::store::{atomic_json, s, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_UNPACKED: u64 = 512 * 1024 * 1024;
const MAX_FILES: usize = 128;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(240);
const CODEX_EXE: &str = "package/vendor/x86_64-pc-windows-msvc/bin/codex.exe";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Program {
    Node,
    Codex,
    Dreamina,
}

struct Release<'a> {
    program: Program,
    directory: &'a str,
    url: &'a str,
    digest: &'a str,
    executable: &'a str,
    max_download: u64,
    tar_gzip: bool,
}

fn release(program: Program) -> Release<'static> {
    match program {
        Program::Node => Release {
            program, directory: "node-24.12.0-windows-x64",
            url: "https://nodejs.org/dist/v24.12.0/win-x64/node.exe",
            digest: "sha256:2ffe3acc0458fdde999f50d11809bbe7c9b7ef204dcf17094e325d26ace101d8",
            executable: "node.exe", max_download: 128 * 1024 * 1024, tar_gzip: false,
        },
        Program::Codex => Release {
            program, directory: "codex-0.160.1-windows-x64",
            url: "https://registry.npmjs.org/@openai/codex/-/codex-0.160.1-win32-x64.tgz",
            digest: "sha512:yyqykHtHNhm0ZViofGozz9a4ffUyXDsiyRu9Q2hPNg/cu4uzSsovNr5kDmzBXPFWAilmlZuNkwpw1W7uae2nlg==",
            executable: CODEX_EXE, max_download: 256 * 1024 * 1024, tar_gzip: true,
        },
        Program::Dreamina => Release {
            program, directory: "dreamina-1.4.19-windows-x64",
            url: "https://lf3-static.bytednsdoc.com/obj/eden-cn/psj_hupthlyk/ljhwZthlaukjlkulzlp/dreamina_cli_beta/dreamina_cli_windows_amd64.exe",
            digest: "sha256:bc027b25299f80886ce8d719ed28c345a39c950404a5957a228c8c683abaab81",
            executable: "dreamina.exe", max_download: 64 * 1024 * 1024, tar_gzip: false,
        },
    }
}

fn key(program: Program) -> &'static str {
    match program {
        Program::Node => "textSdkNodePath",
        Program::Codex => "codexPath",
        Program::Dreamina => "dreaminaPath",
    }
}
fn label(program: Program) -> &'static str {
    match program {
        Program::Node => "文本运行环境",
        Program::Codex => "Codex",
        Program::Dreamina => "即梦生成 CLI",
    }
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn safe_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        && !path.to_string_lossy().chars().any(char::is_control)
        && path.ancestors().all(|p| match fs::symlink_metadata(p) {
            Ok(m) => !is_reparse(&m),
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        })
}

fn native_file(path: &Path) -> bool {
    if !safe_absolute(path) {
        return false;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if !meta.is_file() || !(4..=MAX_UNPACKED).contains(&meta.len()) {
        return false;
    }
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    #[cfg(windows)]
    {
        magic.starts_with(b"MZ")
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
            && [
                b"\x7fELF".as_slice(),
                b"\xcf\xfa\xed\xfe",
                b"\xfe\xed\xfa\xcf",
                b"\xca\xfe\xba\xbe",
                b"\xbe\xba\xfe\xca",
                b"\xca\xfe\xba\xbf",
                b"\xbf\xba\xfe\xca",
            ]
            .contains(&magic.as_slice())
    }
}

fn configured(program: Program, settings: &Value) -> Option<PathBuf> {
    if program == Program::Codex {
        let path = crate::text_cli::resolve_executable(settings).ok()?;
        return native_file(&path).then_some(path);
    }
    let value = s(settings, key(program)).trim().trim_matches('"');
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    native_file(&path).then_some(path)
}

fn candidates(program: Program, resources: Option<&Path>) -> Vec<PathBuf> {
    let name = match program {
        Program::Node => {
            if cfg!(windows) {
                "node.exe"
            } else {
                "node"
            }
        }
        Program::Codex => {
            if cfg!(windows) {
                "codex.exe"
            } else {
                "codex"
            }
        }
        Program::Dreamina => {
            if cfg!(windows) {
                "dreamina.exe"
            } else {
                "dreamina"
            }
        }
    };
    let mut result = vec![];
    if let Some(paths) = std::env::var_os("PATH") {
        result.extend(
            std::env::split_paths(&paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name)),
        );
    }
    let user = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    if let Some(user) = user {
        result.push(user.join(".local/bin").join(name));
        if program == Program::Node {
            result.push(user.join("AppData/Local/Programs/nodejs/node.exe"));
        }
        if program == Program::Dreamina {
            result.push(user.join(".dreamina_cli/bin").join(name));
        }
    }
    if program == Program::Node {
        for env in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(env).map(PathBuf::from) {
                result.push(
                    root.join(if env == "LOCALAPPDATA" {
                        "Programs/nodejs"
                    } else {
                        "nodejs"
                    })
                    .join(name),
                );
            }
        }
        result.extend(
            [
                "C:/Program Files/nodejs/node.exe",
                "/opt/homebrew/bin/node",
                "/usr/local/bin/node",
                "/usr/bin/node",
            ]
            .iter()
            .map(PathBuf::from),
        );
    }
    if program == Program::Dreamina {
        let mut roots = vec![];
        if let Some(root) = resources {
            roots.push(root.to_path_buf());
        }
        if let Some(parent) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            roots.extend([parent.clone(), parent.join("resources")]);
        }
        if let Some(parent) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
            roots.push(parent.to_path_buf());
        }
        result.extend(
            roots
                .into_iter()
                .map(|root| root.join("tools/dreamina/1.4.19").join(name)),
        );
    }
    result
}

fn digest_file(path: &Path, max: u64, sha512: bool) -> Result<String> {
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut sha256 = Sha256::new();
    let mut sha512_hash = Sha512::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > max {
            return Err("RUNTIME_SIZE_LIMIT: 下载或安装文件超过大小限制。".into());
        }
        if sha512 {
            sha512_hash.update(&buffer[..read]);
        } else {
            sha256.update(&buffer[..read]);
        }
    }
    if sha512 {
        Ok(format!(
            "sha512:{}",
            STANDARD.encode(sha512_hash.finalize())
        ))
    } else {
        Ok(format!("sha256:{:x}", sha256.finalize()))
    }
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':'])
        && !value.chars().any(char::is_control)
        && !value.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.ends_with([' ', '.'])
        })
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn installed(root: &Path, spec: &Release<'_>) -> Result<Option<PathBuf>> {
    let directory = root.join(spec.directory);
    if !directory.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    if !safe_absolute(&directory) {
        return Err("RUNTIME_INSTALL_PATH_UNSAFE: 本机运行目录包含外部链接，未修改。".into());
    }
    let receipt_path = directory.join("receipt.json");
    if !safe_absolute(&receipt_path) {
        return Err("RUNTIME_INSTALL_PATH_UNSAFE: 运行环境记录包含外部链接，未修改。".into());
    }
    let file = File::open(&receipt_path).map_err(|_| {
        "RUNTIME_INSTALL_INCOMPLETE: 本机运行环境记录不完整，已保留文件；请更新或修复客户端。"
    })?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 256 * 1024 {
        return Err("RUNTIME_INSTALL_RECEIPT_INVALID: 安装记录异常。".into());
    }
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("RUNTIME_INSTALL_RECEIPT_INVALID: 安装记录异常。".into());
    }
    let receipt: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "RUNTIME_INSTALL_RECEIPT_INVALID: 安装记录损坏。")?;
    let files = receipt["files"]
        .as_array()
        .ok_or("RUNTIME_INSTALL_RECEIPT_INVALID: 缺少文件校验。")?;
    if receipt["version"] != 1
        || receipt["archiveDigest"] != spec.digest
        || receipt["source"] != spec.url
        || files.is_empty()
        || files.len() > MAX_FILES
    {
        return Err("RUNTIME_INSTALL_RECEIPT_INVALID: 安装记录不匹配。".into());
    }
    let mut includes_exe = false;
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for item in files {
        let relative = s(item, "path");
        if !safe_relative(relative) || !seen.insert(relative.to_ascii_lowercase()) {
            return Err("RUNTIME_INSTALL_RECEIPT_INVALID: 文件路径无效。".into());
        }
        let file = directory.join(relative);
        if !safe_absolute(&file) {
            return Err("RUNTIME_INSTALL_CHANGED: 运行环境文件包含外部链接。".into());
        }
        let size = fs::metadata(&file)
            .map_err(|_| "RUNTIME_INSTALL_CHANGED: 运行环境文件缺失。")?
            .len();
        total = total.checked_add(size).ok_or("RUNTIME_SIZE_LIMIT")?;
        if total > MAX_UNPACKED
            || item["bytes"].as_u64() != Some(size)
            || item["sha256"] != digest_file(&file, MAX_UNPACKED, false)?
        {
            return Err("RUNTIME_INSTALL_CHANGED: 运行环境文件校验未通过，未采用。".into());
        }
        includes_exe |= relative == spec.executable;
    }
    let executable = directory.join(spec.executable);
    if !includes_exe || !native_file(&executable) {
        return Err("RUNTIME_INSTALL_EXECUTABLE_INVALID: 运行程序无效。".into());
    }
    Ok(Some(executable))
}

fn verify_discovered(path: PathBuf, root: &Path, spec: &Release<'_>) -> Result<PathBuf> {
    let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    // A read-only inventory must work before the application has ever installed
    // anything. An existing executable cannot live inside a missing root.
    let canonical_root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
        Err(error) => return Err(error.to_string()),
    };
    #[cfg(windows)]
    let managed = canonical
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with(&format!(
            "{}\\",
            canonical_root.to_string_lossy().to_ascii_lowercase()
        ));
    #[cfg(not(windows))]
    let managed = canonical.starts_with(&canonical_root);
    if !managed {
        return Ok(path);
    }
    // Saving a private path must never turn it into an unverified external one.
    // All companion files remain covered on every later automatic configuration.
    let verified = installed(root, spec)?.ok_or(
        "RUNTIME_INSTALL_INCOMPLETE: 本机运行环境记录不完整，已保留文件；请更新或修复客户端。",
    )?;
    if fs::canonicalize(&verified).map_err(|e| e.to_string())? != canonical {
        return Err("RUNTIME_INSTALL_VERSION_UNVERIFIED: 本机运行环境版本尚未核验，已保留文件；请更新或修复客户端。".into());
    }
    Ok(path)
}

fn programs(provider: &str) -> Result<&'static [Program]> {
    match provider {
        "codex_cli" => Ok(&[Program::Codex]),
        "codex_sdk" => Ok(&[Program::Node, Program::Codex]),
        "dreamina" => Ok(&[Program::Dreamina]),
        _ => Err("RUNTIME_PROVIDER_UNSUPPORTED: 此渠道不需要本机组件。".into()),
    }
}

fn discovered(
    program: Program,
    settings: &Value,
    defaults: &Value,
    resources: Option<&Path>,
    root: &Path,
) -> Result<Option<PathBuf>> {
    let spec = release(program);
    let found = configured(program, settings)
        .or_else(|| configured(program, defaults))
        .or_else(|| candidates(program, resources).into_iter().find(|p| native_file(p)));
    match found {
        Some(path) => verify_discovered(path, root, &spec).map(Some),
        None => installed(root, &spec),
    }
}

fn component_status(program: Program, path: Result<Option<PathBuf>>, root: &Path) -> Value {
    let id = match program { Program::Node => "node", Program::Codex => "codex", Program::Dreamina => "dreamina" };
    let mut component = json!({"id":id,"name":label(program),"state":"missing","version":null});
    match path {
        Ok(Some(path)) => {
            component["state"] = json!("installed");
            // Only our hash-checked release has a known version. Merely finding
            // an external executable is not proof of its version or account.
            let spec = release(program);
            if fs::canonicalize(root.join(spec.directory).join(spec.executable)).ok()
                .is_some_and(|managed| fs::canonicalize(&path).ok().as_ref() == Some(&managed)) {
                component["version"] = json!(match program { Program::Node => "24.12.0", Program::Codex => "0.160.1", Program::Dreamina => "1.4.19" });
            }
            component["path"] = json!(path);
        }
        Ok(None) => {}
        Err(error) => { component["state"] = json!("invalid"); component["message"] = json!(error); }
    }
    component
}

/// Local inventory only: no mkdir, leases, downloads, child processes or auth.
/// Private installs remain subject to receipt and companion-file hash checks.
pub fn status(request: &Value, saved: &Value, home: &Path, resources: Option<&Path>) -> Result<Value> {
    let (provider, settings) = crate::readiness::request_settings(request, saved)?;
    let selected = programs(&provider)?;
    let root = home.join("runtimes");
    if !safe_absolute(home) || !safe_absolute(&root) {
        return Err("RUNTIME_INSTALL_PATH_UNSAFE: 本机运行目录无效，未修改。".into());
    }
    let defaults = crate::executable_paths::load(home)?;
    let mut patch = Map::new();
    let mut components = Vec::new();
    for &program in selected {
        let component = component_status(program, discovered(program, &settings, &defaults, resources, &root), &root);
        if let Some(path) = component.get("path") { patch.insert(key(program).into(), path.clone()); }
        components.push(component);
    }
    let ready = components.iter().all(|item| item["state"] == "installed");
    let invalid = components.iter().any(|item| item["state"] == "invalid");
    Ok(json!({"schemaVersion":"runtime-status/v1","provider":provider,"installed":ready,
        "installSupported":cfg!(all(windows,target_arch="x86_64")) && !invalid,
        "components":components,"settings":patch,"checkedAt":chrono::Utc::now().to_rfc3339()}))
}

fn download(spec: &Release<'_>, target: &Path, deadline: Instant) -> Result<()> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or("RUNTIME_SETUP_TIMEOUT: 自动配置超时，可稍后重试。")?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(remaining.min(Duration::from_secs(150)))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("director-desktop-runtime/1")
        .build()
        .map_err(|e| e.to_string())?;
    // All three version-pinned URLs currently serve directly. An unreviewed CDN
    // redirect fails closed; no arbitrary URL ever comes from the IPC request.
    let mut response = client.get(spec.url).send().map_err(|_| {
        format!(
            "RUNTIME_DOWNLOAD_FAILED: {}下载未完成，请检查网络后重试。",
            label(spec.program)
        )
    })?;
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|n| n == 0 || n > spec.max_download)
    {
        return Err("RUNTIME_DOWNLOAD_INVALID: 官方下载响应无效或超过限制，未安装。".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        if Instant::now() >= deadline {
            return Err("RUNTIME_SETUP_TIMEOUT: 自动配置超时，可稍后重试。".into());
        }
        let count = response
            .read(&mut buffer)
            .map_err(|_| "RUNTIME_DOWNLOAD_INTERRUPTED: 下载中断，可稍后重试。")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > spec.max_download {
            return Err("RUNTIME_SIZE_LIMIT: 下载文件超过限制，未安装。".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
    }
    output.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

fn unpack(archive: &Path, directory: &Path, deadline: Instant) -> Result<Vec<String>> {
    let decoder = flate2::read::GzDecoder::new(File::open(archive).map_err(|e| e.to_string())?);
    let mut tar = tar::Archive::new(decoder);
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for entry in tar.entries().map_err(|e| e.to_string())? {
        if Instant::now() >= deadline {
            return Err("RUNTIME_SETUP_TIMEOUT: 自动配置超时，可稍后重试。".into());
        }
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        // The pinned npm archive contains regular files only. Never unpack links,
        // device files, duplicate case-insensitive paths, or escaped destinations.
        if !entry.header().entry_type().is_file()
            || !safe_relative(&path)
            || !path.starts_with("package/")
            || !seen.insert(path.to_ascii_lowercase())
        {
            return Err("RUNTIME_ARCHIVE_PATH_INVALID: 官方程序包文件结构异常，未安装。".into());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("RUNTIME_SIZE_LIMIT")?;
        if total > MAX_UNPACKED || paths.len() >= MAX_FILES {
            return Err("RUNTIME_SIZE_LIMIT: 解压文件超过限制。".into());
        }
        let destination = directory.join(&path);
        fs::create_dir_all(destination.parent().ok_or("RUNTIME_ARCHIVE_PATH_INVALID")?)
            .map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|e| e.to_string())?;
        let bytes = std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        if bytes != entry.size() {
            return Err("RUNTIME_ARCHIVE_TRUNCATED: 程序包不完整。".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if entry.header().mode().unwrap_or(0) & 0o111 != 0 {
                0o700
            } else {
                0o600
            };
            file.set_permissions(fs::Permissions::from_mode(mode))
                .map_err(|e| e.to_string())?;
        }
        file.sync_all().map_err(|e| e.to_string())?;
        paths.push(path);
    }
    Ok(paths)
}

fn install_with(
    spec: &Release<'_>,
    root: &Path,
    deadline: Instant,
    fetch: impl FnOnce(&Release<'_>, &Path, Instant) -> Result<()>,
) -> Result<PathBuf> {
    if let Some(path) = installed(root, spec)? {
        return Ok(path);
    }
    let temporary = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let archive = temporary.path().join("download");
    fetch(spec, &archive, deadline)?;
    if digest_file(
        &archive,
        spec.max_download,
        spec.digest.starts_with("sha512:"),
    )? != spec.digest
    {
        return Err("RUNTIME_DOWNLOAD_HASH_MISMATCH: 官方下载版本已变化或文件不完整；未安装，请更新客户端后重试。".into());
    }
    let payload = temporary.path().join("payload");
    fs::create_dir(&payload).map_err(|e| e.to_string())?;
    let paths = if spec.tar_gzip {
        unpack(&archive, &payload, deadline)?
    } else {
        fs::rename(&archive, payload.join(spec.executable)).map_err(|e| e.to_string())?;
        vec![spec.executable.to_owned()]
    };
    if !native_file(&payload.join(spec.executable)) {
        return Err("RUNTIME_INSTALL_EXECUTABLE_INVALID: 下载不是可用的原生程序，未安装。".into());
    }
    if spec.program == Program::Codex {
        for dependency in [
            "package/vendor/x86_64-pc-windows-msvc/codex-package.json",
            "package/vendor/x86_64-pc-windows-msvc/codex-resources/codex-command-runner.exe",
            "package/vendor/x86_64-pc-windows-msvc/codex-resources/codex-windows-sandbox-setup.exe",
            "package/vendor/x86_64-pc-windows-msvc/bin/codex-code-mode-host.exe",
            "package/vendor/x86_64-pc-windows-msvc/codex-path/rg.exe",
        ] {
            if !paths.iter().any(|p| p == dependency) {
                return Err(
                    "RUNTIME_CODEX_PACKAGE_INCOMPLETE: 官方运行依赖不完整，未安装。".into(),
                );
            }
        }
    }
    let files: Result<Vec<Value>> = paths.iter().map(|p| {
        let file = payload.join(p);
        Ok(json!({"path":p,"bytes":fs::metadata(&file).map_err(|e|e.to_string())?.len(),"sha256":digest_file(&file, MAX_UNPACKED, false)?}))
    }).collect();
    atomic_json(
        &payload.join("receipt.json"),
        &json!({"version":1,"source":spec.url,"archiveDigest":spec.digest,"files":files?}),
    )?;
    if Instant::now() >= deadline {
        return Err("RUNTIME_SETUP_TIMEOUT: 自动配置超时，可稍后重试。".into());
    }
    // Publish only the complete verified directory. Never replace an existing
    // installation, user-selected binary, or a program possibly in use.
    fs::rename(&payload, root.join(spec.directory))
        .map_err(|_| "RUNTIME_INSTALL_BUSY: 运行环境正在更新，请重试。")?;
    Ok(root.join(spec.directory).join(spec.executable))
}

pub fn prepare(
    request: &Value,
    saved: &Value,
    home: &Path,
    resources: Option<&Path>,
) -> Result<Value> {
    let (provider, settings) = crate::readiness::request_settings(request, saved)?;
    let programs = programs(&provider)?;
    if !safe_absolute(home) {
        return Err("RUNTIME_INSTALL_PATH_UNSAFE: 本机运行目录无效。".into());
    }
    let defaults = crate::executable_paths::load(home)?;
    let root = home.join("runtimes");
    if !safe_absolute(&root) {
        return Err("RUNTIME_INSTALL_PATH_UNSAFE: 本机运行目录包含外部链接。".into());
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let lock_path = root.join(".prepare.lock");
    if !safe_absolute(&lock_path) {
        return Err("RUNTIME_INSTALL_PATH_UNSAFE: 运行环境锁包含外部链接。".into());
    }
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| e.to_string())?;
    lease
        .try_lock()
        .map_err(|_| "RUNTIME_SETUP_BUSY: 另一个连接正在自动配置，请稍后重试。")?;
    let deadline = Instant::now() + TOTAL_TIMEOUT;
    let mut patch = Map::new();
    let mut added = Vec::<String>::new();
    let mut messages = Vec::new();
    for &program in programs {
        let spec = release(program);
        let path = if let Some(path) = discovered(program, &settings, &defaults, resources, &root)? {
            path
        } else {
            if !cfg!(all(windows, target_arch = "x86_64")) {
                return Err("RUNTIME_INSTALL_PLATFORM_UNSUPPORTED: 当前平台未提供自动安装包；已安装程序仍可自动识别。".into());
            }
            let path = install_with(&spec, &root, deadline, download)?;
            added.push(key(program).into());
            messages.push(format!("已自动安装{}。", label(program)));
            path
        };
        patch.insert(key(program).into(), json!(path));
    }
    if messages.is_empty() {
        messages.push("已找到本机程序，可以继续检查连接。".into());
    }
    Ok(
        json!({"schemaVersion":"runtime-prepare/v1","provider":provider,"settings":patch,"installed":added,"messages":messages}),
    )
}

#[cfg(test)]
mod tests;
