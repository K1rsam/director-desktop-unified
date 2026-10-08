//! Narrow native boundary; source identity, workflows and business rules stay shared.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub const APPLICATION_ID: &str = "app.director.next";
pub const CREDENTIAL_SERVICE: &str = "director-desktop-next";
pub const HOME_MARKER: &str = "director-next-home.json";

/// A portable Windows EXE and its retained predecessor each use resources
/// bound to their own bytes. Tauri bundles keep their ordinary resource root.
pub fn native_resource_root(bundled: PathBuf) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        return client_resource_root(&bundled, &executable);
    }
    #[cfg(not(windows))]
    Ok(bundled)
}

#[cfg(windows)]
fn client_resource_root(bundled: &Path, executable: &Path) -> Result<PathBuf, String> {
    let (sha256, _) = crate::store::hash_file(executable)?;
    let directory = executable.parent().ok_or("客户端程序没有父目录")?;
    let mut roots = vec![directory.to_path_buf()];
    // Cargo's internal candidate executable lives in release/deps.
    if directory.file_name().is_some_and(|name| name == "deps") {
        if let Some(parent) = directory.parent() { roots.push(parent.to_path_buf()); }
    }
    for root in roots {
        let resources = root.join(".client-resources").join(&sha256);
        if !resources.join("sdk-readiness/manifest.json").is_file() { continue; }
        let physical_root = root.canonicalize().map_err(|e| e.to_string())?;
        let physical_resources = resources.canonicalize().map_err(|e| e.to_string())?;
        if !physical_resources.starts_with(physical_root.join(".client-resources")) {
            return Err("客户端SDK资源越出已验证的发布目录".into());
        }
        return Ok(physical_resources);
    }
    Ok(bundled.to_path_buf())
}

#[cfg(all(test, windows))]
mod client_resource_tests {
    use super::*;
    #[test]
    fn retained_executables_select_only_their_own_resource_hash() {
        let temp = tempfile::tempdir().unwrap();
        let release = temp.path().join("中文 目录/release");
        fs::create_dir_all(release.join("deps")).unwrap();
        let current = release.join("director-desktop-next.exe");
        let previous = release.join("director-desktop-next-previous.exe");
        let candidate = release.join("deps/director_desktop_next-candidate.exe");
        fs::write(&current, b"current synthetic executable").unwrap();
        fs::write(&previous, b"previous synthetic executable").unwrap();
        fs::write(&candidate, b"current synthetic executable").unwrap();
        let mut expected = vec![];
        for executable in [&current, &previous] {
            let (hash, _) = crate::store::hash_file(executable).unwrap();
            let root = release.join(".client-resources").join(hash);
            fs::create_dir_all(root.join("sdk-readiness")).unwrap();
            fs::write(root.join("sdk-readiness/manifest.json"), b"{}").unwrap();
            expected.push(root.canonicalize().unwrap());
        }
        assert_eq!(client_resource_root(&release, &current).unwrap(), expected[0]);
        assert_eq!(client_resource_root(&release, &previous).unwrap(), expected[1]);
        assert_eq!(client_resource_root(&release, &candidate).unwrap(), expected[0]);
        fs::write(&current, b"changed executable").unwrap();
        assert_eq!(client_resource_root(&release, &current).unwrap(), release);
    }
}

/// Never adopt an older app's registry merely because a caller passes its directory.
pub fn ensure_isolated_home(home: &Path) -> Result<(), String> {
    fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let marker = home.join(HOME_MARKER);
    if marker.exists() {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker).map_err(|e| e.to_string())?)
                .map_err(|e| format!("应用目录标识无效: {e}"))?;
        if value["applicationId"] != APPLICATION_ID {
            return Err("拒绝打开其他应用的数据目录".into());
        }
        return Ok(());
    }
    if home.join("projects.json").exists() || home.join("project.sqlite").exists() {
        return Err("拒绝自动接管已有工程目录；请使用新的 director-desktop-next 数据目录".into());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
        .map_err(|e| e.to_string())?;
    file.write_all(
        format!("{{\"applicationId\":\"{APPLICATION_ID}\",\"version\":1}}\n").as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

/// Only media directories belonging to this registered project enter the native asset scope.
/// Database, raw requests, logs, backups and credentials are not webview assets.
pub fn media_scopes(project: &Path) -> Result<Vec<PathBuf>, String> {
    let root = project.canonicalize().map_err(|e| e.to_string())?;
    let mut scopes = Vec::new();
    for name in ["media", "thumbnails"] {
        let path = project.join(name);
        if !path.exists() {
            continue;
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if !path.starts_with(&root) || path == root {
            return Err("媒体目录越出登记项目范围".into());
        }
        scopes.push(path);
    }
    Ok(scopes)
}

/// Native credentials are isolated by the new application's service name.
/// Unit tests use a process-local synthetic store, never the user's keychain.
/// Capability is a platform contract, not a claim that the user's keychain was accessed.
pub fn credential_backend_status() -> serde_json::Value {
    let supported = cfg!(any(target_os = "windows", target_os = "macos"));
    serde_json::json!({"platform":std::env::consts::OS,"supported":supported,
        "persistentBackendConfigured":supported,"nativeAccessVerified":false,
        "service":CREDENTIAL_SERVICE,"plaintextFallback":false})
}
#[cfg(not(test))]
fn require_persistent_credentials() -> Result<(), String> {
    use keyring::credential::CredentialPersistence;
    if !cfg!(any(target_os = "windows", target_os = "macos"))
        || !matches!(
            keyring::default::default_credential_builder().persistence(),
            CredentialPersistence::UntilDelete
        )
    {
        return Err("CREDENTIAL_BACKEND_UNSUPPORTED: 此平台没有获验证的持久原生凭据后端，不保存设置且不明文回退".into());
    }
    Ok(())
}
#[cfg(not(test))]
pub fn read_credential(account: &str) -> Result<String, String> {
    require_persistent_credentials()?;
    keyring::Entry::new(CREDENTIAL_SERVICE, account)
        .map_err(|e| e.to_string())?
        .get_password()
        .map_err(|e| e.to_string())
}
#[cfg(not(test))]
pub fn write_credential(account: &str, value: &str) -> Result<(), String> {
    require_persistent_credentials()?;
    keyring::Entry::new(CREDENTIAL_SERVICE, account)
        .map_err(|e| e.to_string())?
        .set_password(value)
        .map_err(|e| e.to_string())
}
#[cfg(test)]
fn synthetic_credentials() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static STORE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
        std::sync::OnceLock::new();
    STORE.get_or_init(Default::default)
}
#[cfg(test)]
pub fn read_credential(account: &str) -> Result<String, String> {
    synthetic_credentials()
        .lock()
        .map_err(|e| e.to_string())?
        .get(account)
        .cloned()
        .ok_or("No synthetic credential".into())
}
#[cfg(test)]
pub fn write_credential(account: &str, value: &str) -> Result<(), String> {
    synthetic_credentials()
        .lock()
        .map_err(|e| e.to_string())?
        .insert(account.into(), value.into());
    Ok(())
}
#[cfg(test)]
pub fn remove_synthetic_credential(account: &str) {
    if let Ok(mut store) = synthetic_credentials().lock() {
        store.remove(account);
    }
}
