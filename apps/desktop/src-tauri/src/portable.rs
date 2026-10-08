use crate::store::{id, now, s, Result, Store};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

fn zip_file(writer: &mut ZipWriter<fs::File>, name: &str, path: &Path) -> Result<()> {
    writer
        .start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .map_err(|e| e.to_string())?;
    let mut source = fs::File::open(path).map_err(|e| e.to_string())?;
    std::io::copy(&mut source, writer).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn export(store: &Store, path: &Path) -> Result<Value> {
    let temp = store.root.join(format!(".export-{}.sqlite", id()));
    store.backup_db(&temp)?;
    // Export consistent DB first; never copy a live WAL database by filesystem copy.
    let snap = rusqlite::Connection::open(&temp).map_err(|e| e.to_string())?;
    let schema_version = validate_schema(&snap)?;
    snap.execute(
        "DELETE FROM settings WHERE lower(key) LIKE '%token%' OR lower(key) LIKE '%key%'",
        [],
    )
    .map_err(|e| e.to_string())?;
    crate::executable_paths::remove_project_settings(&snap)?;
    let mut q = snap
        .prepare("SELECT data FROM media")
        .map_err(|e| e.to_string())?;
    let media: Vec<Value> = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .map(|r| crate::store::parse(r.map_err(|e| e.to_string())?))
        .collect::<Result<_>>()?;
    drop(q);
    drop(snap);
    let partial = path.with_extension("director.partial");
    let result = (|| {
        let file = fs::File::create(&partial).map_err(|e| e.to_string())?;
        let mut zip = ZipWriter::new(file);
        zip_file(&mut zip, "project.sqlite", &temp)?;
        zip.start_file("manifest.json", SimpleFileOptions::default())
            .map_err(|e| e.to_string())?;
        let manifest = json!({"format":"director-project-v1","schemaVersion":schema_version,"project":store.project()?,"exportedAt":now(),"mediaCount":media.len(),"credentialsIncluded":false});
        zip.write_all(manifest.to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        let mut included = std::collections::HashMap::new();
        for m in &media {
            let p = store.verify_media(m)?;
            let rel = s(m, "relativePath");
            if let Some(hash) = included.get(rel) {
                if hash != s(m, "sha256") {
                    return Err("相同媒体路径存在冲突摘要，未导出".into());
                }
                continue;
            }
            zip_file(&mut zip, rel, &p)?;
            included.insert(rel.to_string(), s(m, "sha256").to_string());
        }
        zip.finish()
            .map_err(|e| e.to_string())?
            .sync_all()
            .map_err(|e| e.to_string())?;
        fs::rename(&partial, path).map_err(|e| e.to_string())?;
        Ok(json!({"path":path,"mediaCount":media.len()}))
    })();
    let _ = fs::remove_file(temp);
    if result.is_err() {
        let _ = fs::remove_file(partial);
    }
    result
}
pub fn import(path: &Path, target: &Path) -> Result<Store> {
    if target.exists() {
        return Err("导入目标必须为空的新目录".into());
    }
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut zip = ZipArchive::new(fs::File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let rel = entry
                .enclosed_name()
                .ok_or("导入包包含越界路径")?
                .to_path_buf();
            if rel
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("非法导入路径".into());
            }
            let name = rel.to_string_lossy().replace('\\', "/");
            if name != "manifest.json" && name != "project.sqlite" && !name.starts_with("media/") {
                return Err("导入包包含非项目内容".into());
            }
            if entry
                .unix_mode()
                .map(|m| m & 0o170000 == 0o120000)
                .unwrap_or(false)
            {
                return Err("不允许导入符号链接".into());
            }
            total = total.checked_add(entry.size()).ok_or("导入包过大")?;
            if total > 256 * 1024 * 1024 * 1024 {
                return Err("导入包超过256GiB保护阈值".into());
            }
            let dst = target.join(rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if entry.is_dir() {
                continue;
            }
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dst)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        }
        let manifest_path = target.join("manifest.json");
        if fs::metadata(&manifest_path)
            .map_err(|e| e.to_string())?
            .len()
            > 1024 * 1024
        {
            return Err("项目包manifest超过1MiB".into());
        }
        let manifest: Value =
            serde_json::from_slice(&fs::read(manifest_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let manifest_version = manifest["schemaVersion"]
            .as_u64()
            .filter(|version| matches!(version, 1 | 2));
        if s(&manifest, "format") != "director-project-v1" || manifest_version.is_none() {
            return Err(
                "不支持此项目包schemaVersion；需要明确的director-project-v1及已知数据库版本1或2"
                    .into(),
            );
        }
        // Validate before Store::open can initialize or migrate a database.
        let raw = rusqlite::Connection::open_with_flags(
            target.join("project.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        let database_version = validate_schema(&raw)?;
        if Some(database_version) != manifest_version {
            return Err("manifest与数据库schema_version不一致".into());
        }
        drop(raw);
        let store = Store::open(target.to_path_buf())?;
        let integrity: String = store
            .conn()?
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if integrity != "ok" {
            return Err("项目数据库完整性校验失败".into());
        }
        // Packages from older versions may contain another machine's active paths.
        // Do not adopt them as local defaults or rewrite historical task snapshots.
        crate::executable_paths::remove_project_settings(&store.conn()?)?;
        for m in store.values("media")? {
            store.verify_media(&m)?;
        }
        if manifest["mediaCount"].as_u64() != Some(store.values("media")?.len() as u64) {
            return Err("项目包媒体数量与数据库不一致".into());
        }
        relocate(&store)?;
        store.recover_interrupted()?;
        Ok(store)
    })();
    if result.is_err() {
        let failed = target.with_extension(format!("failed-{}", id()));
        let _ = fs::rename(target, failed);
    }
    result
}

pub const BACKUP_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
pub fn backup(store: &Store, checkpoint: bool) -> Result<Value> {
    let dir = store.root.join("backups");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let prefix = if checkpoint { "checkpoint" } else { "snapshot" };
    let path = dir.join(format!(
        "{prefix}-{}-{}.sqlite",
        chrono::Utc::now().format("%Y%m%dT%H%M%S"),
        id()
    ));
    store.backup_db(&path)?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > BACKUP_LIMIT {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
        return Err("单份数据库快照超过2GiB；自动备份无法在容量内保存，请手动导出项目".into());
    }
    prune(&dir, BACKUP_LIMIT)?;
    store.conn()?.execute("INSERT INTO metadata VALUES('backed_up_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
    Ok(json!({"path":path,"kind":prefix}))
}
pub fn prune(dir: &Path, limit: u64) -> Result<()> {
    let mut files: Vec<(PathBuf, u64)> = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            if (n.starts_with("snapshot-") || n.starts_with("checkpoint-"))
                && n.ends_with(".sqlite")
            {
                Some((e.path(), e.metadata().ok()?.len()))
            } else {
                None
            }
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut checkpoints = 0;
    let mut snapshots = 0;
    let mut kept = Vec::new();
    for (p, size) in files {
        let cp = p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("checkpoint-");
        let counter = if cp { &mut checkpoints } else { &mut snapshots };
        *counter += 1;
        if *counter > if cp { 2 } else { 48 } {
            fs::remove_file(&p).map_err(|e| e.to_string())?;
        } else {
            kept.push((p, size));
        }
    }
    let mut total: u64 = kept.iter().map(|(_, n)| n).sum();
    while total > limit && kept.len() > 1 {
        let (p, size) = kept.pop().unwrap();
        fs::remove_file(p).map_err(|e| e.to_string())?;
        total -= size;
    }
    if total > limit {
        return Err("备份容量限制无法满足".into());
    }
    Ok(())
}
pub fn maybe_backup(store: &Store) -> Result<()> {
    let c = store.conn()?;
    let read = |key: &str| {
        c.query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    };
    let changed = read("changed_at");
    let backed = read("backed_up_at");
    if changed.is_some() && changed > backed {
        let elapsed = backed
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds())
            .unwrap_or(901);
        if elapsed >= 900 {
            backup(store, false)?;
        }
    }
    Ok(())
}
fn validate_schema(connection: &rusqlite::Connection) -> Result<u64> {
    let version: String = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "项目数据库缺少明确schema_version，不能自动猜测兼容性".to_owned())?;
    match version.as_str() {
        "1" => Ok(1),
        "2" => Ok(2),
        _ => Err(format!("不支持项目数据库schema_version={version}")),
    }
}

/// Used by project import and Engine's backup-copy restoration after media has been verified/copied.
/// Submitted and unknown submission snapshots remain byte-identical; only local unsent work is invalidated.
pub fn relocate(store: &Store) -> Result<usize> {
    validate_schema(&store.conn()?)?;
    let connection = store.conn()?;
    let mut query=connection.prepare("SELECT id,snapshot,state FROM jobs WHERE kind IN ('image','video') AND COALESCE(task_id,'')='' AND (receipt IS NULL OR trim(receipt) IN ('','null','{}')) AND state IN ('queued','ready','prepared','preparing','waiting_dependency','stale')").map_err(|e|e.to_string())?;
    let pending = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(query);
    drop(connection);
    store.refresh_media_paths()?;
    store.invalidate_unsubmitted("项目目录已重定位，旧素材准备已过期，请重新物化后再生成")?;
    let mut connection = store.conn()?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let mut count = 0;
    for (id, snapshot, previous_state) in pending {
        // Recheck the current state/receipt inside this transaction before touching an execution snapshot.
        let raw:Option<String>=transaction.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND state='stale' AND COALESCE(task_id,'')='' AND (receipt IS NULL OR trim(receipt) IN ('','null','{}'))",[&id],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
        if raw.as_deref() != Some(snapshot.as_str()) {
            continue;
        }
        let mut value: Value =
            serde_json::from_str(&snapshot).map_err(|e| format!("未提交任务快照无效: {e}"))?;
        remove_execution_paths(&mut value);
        transaction
            .execute(
                "UPDATE jobs SET snapshot=?1,prepared=NULL WHERE id=?2",
                rusqlite::params![value.to_string(), id],
            )
            .map_err(|e| e.to_string())?;
        if previous_state != "stale" {
            count += 1;
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(count)
}
fn remove_execution_paths(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for key in [
            "preparedPackage",
            "path",
            "projectRoot",
            "rootPath",
            "workingDirectory",
            "cwd",
        ] {
            object.remove(key);
        }
        for field in ["references", "mediaRefs"] {
            if let Some(references) = object.get_mut(field).and_then(Value::as_array_mut) {
                for reference in references {
                    if let Some(item) = reference.as_object_mut() {
                        for key in ["path", "thumbnailPath", "localPath"] {
                            item.remove(key);
                        }
                    }
                }
            }
        }
        if let Some(request) = object.get_mut("request") {
            remove_execution_paths(request);
        }
    }
}
