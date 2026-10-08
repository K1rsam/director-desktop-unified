use crate::store::{atomic_json, s, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

/// One file lease covers read-modify-write in every process using this library.
pub fn lock(home: &Path) -> Result<fs::File> {
    fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join(".project-library.lock"))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if start.elapsed() < Duration::from_secs(5) => {
                thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(format!("项目列表正在更新，请稍后重试：{error}")),
        }
    }
}

fn read_rows(path: &Path) -> Result<Vec<Value>> {
    if !path.is_file() {
        return Ok(vec![]);
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let rows = value.as_array().ok_or("项目索引不是数组")?;
    let mut ids = HashSet::new();
    for row in rows {
        if !row.is_object() || s(row, "id").is_empty() || !ids.insert(s(row, "id")) {
            return Err("项目索引身份为空或重复".into());
        }
    }
    Ok(rows.clone())
}

/// Import only missing registrations. Existing lifecycle choices and databases remain authoritative.
/// Remember every consumed ID so a later user deletion cannot be undone by a legacy registry.
pub fn recover(home: &Path, legacy_registries: &[PathBuf]) -> Result<Value> {
    let _lease = lock(home)?;
    let registry_path = home.join("projects.json");
    let mut rows = read_rows(&registry_path)?;
    let mut ids: HashSet<_> = rows.iter().map(|r| s(r, "id").to_owned()).collect();
    let ledger_path = home.join("project-library-recovery.json");
    let mut ledger: Value = if ledger_path.is_file() {
        serde_json::from_slice(&fs::read(&ledger_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
    } else {
        json!({"version":1,"consumed":[]})
    };
    if ledger["version"] != 1
        || !ledger["consumed"].as_array().is_some_and(|rows| {
            rows.iter()
                .all(|r| r.as_str().is_some_and(|s| !s.is_empty()))
        })
    {
        return Err("项目目录接续记录无效；未修改原索引".into());
    }
    let mut consumed: HashSet<_> = ledger["consumed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut imported = vec![];
    let mut skipped = vec![];
    let mut ledger_changed = false;
    for registry in legacy_registries {
        let legacy_home = registry.parent().ok_or("旧索引路径没有父目录")?;
        let candidates = match read_rows(registry) {
            Ok(rows) => rows,
            Err(error) => {
                skipped.push(json!({"registry":registry,"reason":error}));
                continue;
            }
        };
        for mut row in candidates {
            let id = s(&row, "id").to_owned();
            let key = format!("{}|{id}", registry.to_string_lossy());
            if consumed.contains(&key) {
                continue;
            }
            if ids.contains(&id) {
                consumed.insert(key.clone());
                ledger["consumed"].as_array_mut().unwrap().push(json!(key));
                ledger_changed = true;
                continue;
            }
            let stored_root = PathBuf::from(s(&row, "rootPath"));
            let root = if let Ok(relative) = stored_root.strip_prefix(home) {
                let physical_root = legacy_home.join(relative);
                if physical_root.join("project.sqlite").is_file() {
                    physical_root
                } else {
                    stored_root
                }
            } else {
                stored_root
            };
            let verified = (|| -> Result<()> {
                if !root.is_absolute() || !root.join("project.sqlite").is_file() {
                    return Err("现有项目数据库不存在；未初始化空库".into());
                }
                let db = Connection::open_with_flags(
                    root.join("project.sqlite"),
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| e.to_string())?;
                let stored_id: String = db
                    .query_row("SELECT id FROM project LIMIT 1", [], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                if stored_id != id {
                    return Err("项目索引与数据库身份不一致；未替换".into());
                }
                Ok(())
            })();
            if let Err(reason) = verified {
                skipped.push(json!({"registry":registry,"projectId":id,"reason":reason}));
                continue;
            }
            row["rootPath"] = json!(root);
            imported
                .push(json!({"id":id,"title":row["title"],"rootPath":root,"registry":registry}));
            rows.push(row);
            ids.insert(id);
            consumed.insert(key.clone());
            ledger["consumed"].as_array_mut().unwrap().push(json!(key));
            ledger_changed = true;
        }
    }
    if !imported.is_empty() {
        atomic_json(&registry_path, &json!(rows))?;
    }
    if ledger_changed {
        atomic_json(&ledger_path, &ledger)?;
    }
    Ok(
        json!({"projectCount":rows.len(),"imported":imported,"skipped":skipped,"projectFilesChanged":false,"generationRequests":0}),
    )
}

/// The only automatic legacy locations are Windows packaged-host shadows of this app's registry.
pub fn recover_virtualized(home: &Path) -> Result<Value> {
    #[cfg(windows)]
    {
        crate::windows_runtime::verify_shared_directory(home)?;
        let roaming = home.parent().ok_or("缺少项目根目录")?;
        if !roaming
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("Roaming"))
        {
            return Ok(
                json!({"projectCount":read_rows(&home.join("projects.json"))?.len(),"imported":[],"skipped":[]}),
            );
        }
        let appdata = roaming.parent().ok_or("缺少用户应用数据目录")?;
        let packages = appdata.join("Local").join("Packages");
        let mut registries = vec![];
        if packages.is_dir() {
            for package in fs::read_dir(packages).map_err(|e| e.to_string())? {
                let package = package.map_err(|e| e.to_string())?;
                if !package.file_type().map_err(|e| e.to_string())?.is_dir() {
                    continue;
                }
                let path = package
                    .path()
                    .join("LocalCache")
                    .join("Roaming")
                    .join(home.file_name().ok_or("缺少应用标识")?)
                    .join("projects.json");
                if path.is_file() {
                    registries.push(path);
                }
            }
        }
        registries.sort();
        recover(home, &registries)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        Ok(json!({"imported":[],"skipped":[]}))
    }
}

#[cfg(test)]
#[path = "project_library_tests.rs"]
mod tests;
