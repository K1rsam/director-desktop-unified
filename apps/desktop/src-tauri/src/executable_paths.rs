//! Machine-local executable defaults. This file is not part of a project export.
//! Only explicit path saves update it; opening or importing projects never seeds it.
use crate::store::{atomic_json, Result, Store};
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

pub const PATH_KEYS: [&str; 4] = [
    "codexPath",
    "dreaminaPath",
    "dreaminaCanvasPath",
    "textSdkNodePath",
];
pub const FILE_NAME: &str = "local-executable-paths.json";

fn local_path(value: &Value) -> Option<String> {
    let value = value.as_str()?.trim().trim_matches('"');
    if value.is_empty()
        || value.len() > 32768
        || value.chars().any(char::is_control)
        || !Path::new(value).is_absolute()
    {
        return None;
    }
    Some(value.to_owned())
}

fn path_patch(settings: &Value) -> Map<String, Value> {
    PATH_KEYS
        .iter()
        .filter_map(|key| {
            local_path(settings.get(*key)?).map(|path| ((*key).to_owned(), json!(path)))
        })
        .collect()
}

/// An atomic reader never needs a file lease. Missing defaults are an empty patch.
pub fn load(home: &Path) -> Result<Value> {
    let file = home.join(FILE_NAME);
    let bytes = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(error) => return Err(format!("无法读取本机程序路径默认值：{error}")),
    };
    if bytes.len() > 256 * 1024 {
        return Err("本机程序路径默认值文件过大，未采用。".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "本机程序路径默认值无效，未采用。".to_owned())?;
    let document = value
        .as_object()
        .ok_or("本机程序路径默认值不是对象，未采用。")?;
    let paths = value["paths"]
        .as_object()
        .ok_or("本机程序路径默认值缺少路径，未采用。")?;
    if value["version"] != 1
        || document.len() != 2
        || !document.contains_key("version")
        || paths.keys().any(|key| !PATH_KEYS.contains(&key.as_str()))
        || paths.values().any(|value| local_path(value).is_none())
    {
        return Err("本机程序路径默认值包含不支持的字段或路径，未采用。".into());
    }
    Ok(Value::Object(path_patch(&value["paths"])))
}

fn lock(home: &Path) -> Result<fs::File> {
    fs::create_dir_all(home).map_err(|error| error.to_string())?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join(".local-executable-paths.lock"))
        .map_err(|error| error.to_string())?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if start.elapsed() < Duration::from_secs(5) => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(format!("本机程序路径默认值正在更新，请稍后重试：{error}")),
        }
    }
}

/// Remember only nonempty absolute paths supplied in this explicit save. An empty
/// project field does not erase a previous machine default or switch other projects.
pub fn remember(home: &Path, settings: &Value) -> Result<()> {
    let patch = path_patch(settings);
    if patch.is_empty() {
        return Ok(());
    }
    let _lease = lock(home)?;
    let mut paths = load(home)?;
    paths
        .as_object_mut()
        .ok_or("本机程序路径默认值无效")?
        .extend(patch);
    atomic_json(&home.join(FILE_NAME), &json!({"version":1,"paths":paths}))
        .map_err(|error| format!("本机程序路径默认值未保存：{error}"))
}

/// Only the new-project/import hooks call this. A project's own nonempty path is
/// authoritative; this does not inspect accounts, models, or frozen job settings.
pub fn apply(home: &Path, store: &Store) -> Result<()> {
    let defaults = load(home)?;
    let current = store.settings()?;
    let patch: Map<_, _> = defaults
        .as_object()
        .ok_or("本机程序路径默认值无效")?
        .iter()
        .filter(|(key, _)| {
            current.get(key.as_str()).is_none_or(|value| {
                value.is_null() || value.as_str().is_some_and(|path| path.trim().is_empty())
            })
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !patch.is_empty() {
        store.save_settings(&Value::Object(patch))?;
    }
    Ok(())
}

/// Remove only current configuration from a portable DB. Frozen snapshots and
/// receipts intentionally retain their exact historical paths and identities.
pub fn remove_project_settings(connection: &rusqlite::Connection) -> Result<()> {
    connection
        .execute(
            "DELETE FROM settings WHERE key IN (?1,?2,?3,?4)",
            rusqlite::params![PATH_KEYS[0], PATH_KEYS[1], PATH_KEYS[2], PATH_KEYS[3]],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
