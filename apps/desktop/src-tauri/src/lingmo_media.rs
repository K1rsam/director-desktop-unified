//! Local, receipt-bound folder backfill. No provider calls, generation, source
//! mutation, or fuzzy identity matching are allowed along this path.
use crate::{
    lingmo_import,
    pipeline::digest,
    store::{hash_file, now, parse, s, set_current_image, Result, Store},
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Instant,
};

const MAX_FILES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const PREVIEW_PREFIX: &str = "lingmo.media.preview:";
type CommitLocks = HashMap<PathBuf, Weak<Mutex<()>>>;
fn project_commit_lock(store: &Store) -> Result<Arc<Mutex<()>>> {
    static LOCKS: OnceLock<Mutex<CommitLocks>> = OnceLock::new();
    let key = store.root.canonicalize().map_err(|e| e.to_string())?;
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "本地素材写入锁不可用")?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

fn values(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}
fn metadata(store: &Store, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = store
        .conn()?
        .query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(parse).transpose()
}
fn put_metadata(store: &Store, key: &str, value: &Value) -> Result<()> {
    store.conn()?.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value.to_string()]).map_err(|e|e.to_string())?;
    Ok(())
}
fn assets(store: &Store, document: &str) -> Result<Vec<Value>> {
    let doc=lingmo_import::document(store, document)?;
    let prefix = format!("lingmo-{document}-asset-");
    let mut ids=HashSet::new();
    for scene in values(&doc,"scenes") {for id in values(&scene,"assetIds") {if let Some(id)=id.as_str(){ids.insert(id.to_owned());}}}
    if let Some(mapping)=metadata(store,&format!("lingmo.asset-identities:{document}"))? {
        if mapping["schemaVersion"]!="lingmo-asset-identities/v1" || mapping["documentSha256"]!=document {
            return Err("本分镜的资产接续映射身份不符，未猜测扫描范围".into());
        }
        for binding in values(&mapping,"bindings") {if !s(&binding,"assetId").is_empty(){ids.insert(s(&binding,"assetId").to_owned());}}
    }
    Ok(store
        .values("assets")?
        .into_iter()
        .filter(|a| {
            (s(a, "id").starts_with(&prefix) || ids.contains(s(a,"id")))
                && s(a, "origin") == "imported-storyboard"
                && s(a, "deletedAt").is_empty()
        })
        .collect())
}
fn asset_snapshot(a: &Value) -> Value {
    json!({"id":a["id"],"variantId":a["variantId"],"version":a["version"],"name":a["name"],"lingmoType":a["lingmoType"],"filename":a["filename"],"token":a["token"]})
}
fn media_kind(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "webp" => Some("image"),
        "wav" | "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "aif" | "aiff" => Some("audio"),
        _ => None,
    }
}
fn compatible(asset: &Value, kind: &str) -> bool {
    match s(asset, "lingmoType") {
        "SE" => kind == "audio",
        "OM" => ["image", "audio"].contains(&kind),
        "CH" | "PR" | "SC" => kind == "image",
        _ => false,
    }
}
fn linked(path: &Path) -> Result<bool> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return Ok(meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0);
    }
    #[cfg(not(windows))]
    {
        Ok(meta.file_type().is_symlink())
    }
}
fn no_links(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component);
        if cursor.exists() && linked(&cursor)? {
            return Err("路径包含链接或重解析点，未读取链接目标".into());
        }
    }
    Ok(())
}
fn collect(directory: &Path, output: &mut Vec<(PathBuf, PathBuf)>, root: &Path) -> Result<()> {
    let mut children = fs::read_dir(directory)
        .map_err(|e| format!("无法读取文件夹 {}: {e}", directory.display()))?
        .map(|e| e.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>>>()?;
    children.sort();
    for path in children {
        if output.len() >= MAX_FILES {
            return Err("文件夹文件超过20000项，请分批选择".into());
        }
        if linked(&path)? {
            output.push((path, root.to_path_buf()));
        } else if path.is_dir() {
            collect(&path, output, root)?;
        } else if path.is_file() {
            output.push((path, root.to_path_buf()));
        }
    }
    Ok(())
}
fn normalized_path(path: &str) -> String {
    path.replace('\\', "/")
}
fn typed_name(stem: &str) -> Option<(String, String)> {
    let inner = stem
        .strip_prefix("@[")
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(stem);
    let mut name = inner;
    let mut ty = None;
    for (prefix, kind) in [
        ("角色", "CH"),
        ("人物", "CH"),
        ("道具", "PR"),
        ("场景", "SC"),
        ("声音", "SE"),
        ("音频", "SE"),
        ("其他群演", "OM"),
        ("群演", "OM"),
        ("其他", "OM"),
    ] {
        if let Some(rest) = name
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix(['-', '_', '：', ':']))
        {
            name = rest;
            ty = Some(kind);
            break;
        }
    }
    for suffix in ["CH", "PR", "SC", "SE", "OM"] {
        if let Some(rest) = name.strip_suffix(suffix) {
            if ty.is_some_and(|kind| kind != suffix) {
                return None;
            }
            name = rest;
            ty = Some(suffix);
            break;
        }
    }
    let ty = ty?;
    (!name.is_empty() && name.trim() == name).then(|| (name.to_owned(), ty.to_owned()))
}
fn match_assets(path: &Path, relative: &str, kind: &str, assets: &[Value]) -> Vec<String> {
    let filename = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    let rel = normalized_path(relative);
    let exact: Vec<_> = assets
        .iter()
        .filter(|a| {
            compatible(a, kind) && !s(a, "filename").is_empty() && {
                let expected = normalized_path(s(a, "filename"));
                expected == rel || (!expected.contains('/') && expected == filename)
            }
        })
        .map(|a| s(a, "id").to_owned())
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    // Folder selection may start one level above/below the supplied catalog's
    // directory. A complete basename is still exact; multiple catalog identities
    // with that basename remain ambiguous instead of being guessed.
    let by_filename: Vec<_> = assets
        .iter()
        .filter(|a| {
            compatible(a, kind)
                && !s(a, "filename").is_empty()
                && normalized_path(s(a, "filename")).rsplit('/').next() == Some(filename)
        })
        .map(|a| s(a, "id").to_owned())
        .collect();
    if !by_filename.is_empty() {
        return by_filename;
    }
    let Some((name, ty)) = typed_name(path.file_stem().and_then(|v| v.to_str()).unwrap_or(""))
    else {
        return vec![];
    };
    assets
        .iter()
        .filter(|a| compatible(a, kind) && s(a, "name") == name && s(a, "lingmoType") == ty)
        .map(|a| s(a, "id").to_owned())
        .collect()
}

pub fn preview(store: &Store, request: &Value) -> Result<Value> {
    let start = Instant::now();
    let document = s(request, "documentId");
    let assets = assets(store, document)?;
    let mut inputs = Vec::new();
    if !s(request, "directory").is_empty() {
        let path = Path::new(s(request, "directory"));
        no_links(path)?;
        let root = path.canonicalize().map_err(|e| e.to_string())?;
        if !root.is_dir() {
            return Err("请选择本地素材文件夹".into());
        }
        collect(&root, &mut inputs, &root)?;
    } else {
        for value in values(request, "paths") {
            let path = PathBuf::from(value.as_str().ok_or("文件路径必须为字符串")?);
            if !path.is_absolute() {
                return Err("请选择本机完整文件路径".into());
            }
            let root = path.parent().ok_or("文件路径无效")?.to_path_buf();
            inputs.push((path, root));
        }
        if inputs.len() > MAX_FILES {
            return Err("单批文件不能超过20000项".into());
        }
    }
    if inputs.is_empty() {
        return Err("未找到本地素材文件".into());
    }
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for (path, root) in inputs {
        if !seen.insert(path.clone()) {
            continue;
        }
        let filename = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let mut row = json!({"id":digest(&path.to_string_lossy()),"path":path,"root":root,"relativePath":relative,"filename":filename,"kind":media_kind(&path).unwrap_or("image"),"status":"unsupported","candidateAssetIds":[],"size":0});
        let result = (|| -> Result<()> {
            no_links(&path)?;
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            let root = root.canonicalize().map_err(|e| e.to_string())?;
            if !canonical.starts_with(&root) || !canonical.is_file() {
                return Err("素材文件不在选择范围内".into());
            }
            row["path"] = json!(canonical);
            row["root"] = json!(root);
            row["size"] = json!(fs::metadata(&canonical).map_err(|e| e.to_string())?.len());
            let Some(kind) = media_kind(&canonical) else {
                row["message"] = json!("不支持的格式；支持 PNG/JPEG/WebP 及常用音频");
                return Ok(());
            };
            if row["size"].as_u64().unwrap_or(0) > MAX_FILE_BYTES {
                return Err("单个素材超过2GiB上限".into());
            }
            let (hash, size) = hash_file(&canonical)?;
            row["sha256"] = json!(hash);
            row["size"] = json!(size);
            let candidates = match_assets(&canonical, &relative, kind, &assets);
            row["candidateAssetIds"] = json!(candidates);
            row["status"] = json!(match candidates.len() {
                0 => "unmatched",
                1 => "matched",
                _ => "ambiguous",
            });
            if candidates.len() == 1 {
                let asset = assets.iter().find(|a| s(a, "id") == candidates[0]).unwrap();
                row["assetId"] = asset["id"].clone();
                row["variantId"] = asset["variantId"].clone();
                row["assetName"] = asset["name"].clone();
                row["expectedAdoptedMediaId"] =
                    json!(store.adopted(s(asset, "id"), s(asset, "variantId"))?);
            }
            Ok(())
        })();
        if let Err(message) = result {
            row["status"] = json!("unsupported");
            row["message"] = json!(message);
        }
        rows.push(row);
    }
    let mut targets = HashMap::<String, usize>::new();
    for row in &rows {
        if s(row, "status") == "matched" {
            *targets.entry(s(row, "assetId").to_owned()).or_default() += 1;
        }
    }
    for row in &mut rows {
        if targets
            .get(s(row, "assetId"))
            .is_some_and(|count| *count > 1)
        {
            row["status"] = json!("ambiguous");
            row["message"] = json!("同一资产匹配到多个文件，请明确选择其中一个");
            row.as_object_mut().unwrap().remove("assetId");
            row.as_object_mut().unwrap().remove("variantId");
        }
    }
    let token = uuid::Uuid::new_v4().to_string();
    let counts = json!({"files":rows.len(),"matched":rows.iter().filter(|r|s(r,"status")=="matched").count(),"unmatched":rows.iter().filter(|r|s(r,"status")=="unmatched").count(),"ambiguous":rows.iter().filter(|r|s(r,"status")=="ambiguous").count(),"unsupported":rows.iter().filter(|r|s(r,"status")=="unsupported").count()});
    let output = json!({"previewToken":token,"documentId":document,"rows":rows,"counts":counts,"milliseconds":start.elapsed().as_secs_f64()*1000.,"generationSubmissions":0});
    put_metadata(
        store,
        &format!("{PREVIEW_PREFIX}{token}"),
        &json!({"preview":output,"assets":assets.iter().map(asset_snapshot).collect::<Vec<_>>(),"createdAt":now()}),
    )?;
    Ok(output)
}

fn placeholder(store: &Store, media: &Value) -> Result<bool> {
    if media["placeholderSource"] == "image-pixels/v1" {
        if let Some(flag) = media["placeholder"].as_bool() {
            return Ok(flag);
        }
    }
    if s(media, "kind") != "image" {
        return Ok(false);
    }
    store.verify_media(media)?;
    let image = image::open(s(media, "path"))
        .map_err(|e| e.to_string())?
        .to_rgba8();
    Ok(image
        .pixels()
        .all(|p| p[3] == 0 || p.0[..3].iter().all(|c| *c >= 250)))
}

/// Restore the last scan without rescanning or adopting anything. File counts
/// and document asset counts are distinct; unmatched variants remain visible.
pub fn latest(store: &Store, request: &Value) -> Result<Value> {
    let document = s(request, "documentId");
    let current = assets(store, document)?;
    let conn = store.conn()?;
    let saved: Option<String> = conn.query_row(
        "SELECT value FROM metadata WHERE key LIKE 'lingmo.media.preview:%' AND json_extract(value,'$.preview.documentId')=?1 ORDER BY json_extract(value,'$.createdAt') DESC,rowid DESC LIMIT 1",
        [document], |r| r.get(0),
    ).optional().map_err(|e|e.to_string())?;
    let Some(saved) = saved else { return Ok(Value::Null) };
    let saved = parse(saved)?;
    let preview = &saved["preview"];
    let ids: HashSet<&str> = current.iter().map(|a| s(a,"id")).collect();
    let mut links = HashSet::<(String,String,String)>::new();
    let mut query = conn.prepare("SELECT m.sha256,ma.asset_id,ma.variant_id FROM media_assets ma JOIN media m ON m.id=ma.media_id").map_err(|e|e.to_string())?;
    let rows = query.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
    for row in rows { let row=row.map_err(|e|e.to_string())?; if ids.contains(row.1.as_str()) { links.insert(row); } }
    let mut completed = HashMap::<String,Vec<Value>>::new();
    let mut query = conn.prepare("SELECT value FROM metadata WHERE key LIKE 'lingmo.media.completed:%'").map_err(|e|e.to_string())?;
    for row in query.query_map([], |r|r.get::<_,String>(0)).map_err(|e|e.to_string())? {
        let done=parse(row.map_err(|e|e.to_string())?)?;
        completed.entry(s(&done,"rowId").to_owned()).or_default().push(done);
    }
    let mut pending = Vec::new();
    for row in values(preview,"rows") {
        let hash=s(&row,"sha256");
        let automatic = !hash.is_empty() && links.contains(&(hash.to_owned(),s(&row,"assetId").to_owned(),s(&row,"variantId").to_owned()));
        if automatic {continue;}
        let manual = completed.get(s(&row,"id")).is_some_and(|records| records.iter().any(|done| {
            !hash.is_empty() && links.contains(&(hash.to_owned(),s(done,"assetId").to_owned(),s(done,"variantId").to_owned()))
            && store.get("media",s(done,"mediaId")).is_ok_and(|media|s(&media,"sha256")==hash)
        }));
        if !manual {pending.push(row);}
    }
    let total=values(preview,"rows").len();
    Ok(json!({"preview":preview,"scannedAt":saved["createdAt"],"assetCount":current.len(),"fileCount":total,"importedFiles":total-pending.len(),"pendingFiles":pending.len(),"pendingRows":pending}))
}

pub fn commit(store: &Store, request: &Value) -> Result<Value> {
    let start = Instant::now();
    // Concurrent identical clicks must share the same once-only receipt and
    // immutable media rather than racing before Store's hash lookup/insert.
    let lock = project_commit_lock(store)?;
    let _guard = lock.lock().map_err(|_| "本地素材写入中断，请重试")?;
    let token = s(request, "previewToken");
    let receipt = metadata(store, &format!("{PREVIEW_PREFIX}{token}"))?
        .ok_or("预览已失效，请重新选择素材文件夹")?;
    let document = s(request, "documentId");
    if document != s(&receipt["preview"], "documentId") {
        return Err("预览不属于当前分镜来源".into());
    }
    let current = assets(store, document)?;
    let snapshots = values(&receipt, "assets");
    let rows = values(&receipt["preview"], "rows");
    let selections = values(request, "selections");
    let mut duplicate_rows = HashSet::new();
    let mut duplicate_assets = HashSet::new();
    let mut row_counts = HashMap::<String, usize>::new();
    let mut asset_counts = HashMap::<String, usize>::new();
    for selection in &selections {
        *row_counts
            .entry(s(selection, "rowId").to_owned())
            .or_default() += 1;
        *asset_counts
            .entry(s(selection, "assetId").to_owned())
            .or_default() += 1;
    }
    for (id, count) in row_counts {
        if count > 1 {
            duplicate_rows.insert(id);
        }
    }
    for (id, count) in asset_counts {
        if count > 1 {
            duplicate_assets.insert(id);
        }
    }
    let mut result_rows = Vec::new();
    let mut imported = 0;
    let mut adopted = 0;
    let mut placeholders = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut reused = 0;
    let mut media_cache = HashMap::<String, Value>::new();
    for selection in selections {
        let row_id = s(&selection, "rowId");
        let result = (|| -> Result<Value> {
            if duplicate_rows.contains(row_id)
                || duplicate_assets.contains(s(&selection, "assetId"))
            {
                return Err("同一批次不可重复选择文件或向同一资产提交多份素材".into());
            }
            let row = rows
                .iter()
                .find(|r| s(r, "id") == row_id)
                .ok_or("所选文件不属于本次预览")?;
            if s(row, "status") == "unsupported" {
                return Err("不支持的文件不可导入".into());
            }
            let asset = current
                .iter()
                .find(|a| s(a, "id") == s(&selection, "assetId"))
                .ok_or("目标资产不属于当前文档或已删除")?;
            let old = snapshots
                .iter()
                .find(|a| s(a, "id") == s(asset, "id"))
                .ok_or("目标资产不属于预览版本")?;
            if asset_snapshot(asset) != *old {
                return Err("预览后资产身份或版本已变化，请重新预览".into());
            }
            let variant = selection["variantId"]
                .as_str()
                .unwrap_or_else(|| s(asset, "variantId"));
            if variant != s(asset, "variantId") || !compatible(asset, s(row, "kind")) {
                return Err("目标形象或媒介类型不匹配".into());
            }
            if selection["adopt"] == true
                && s(row, "kind") == "image"
                && !selection
                    .get("expectedAdoptedMediaId")
                    .is_some_and(|v| v.is_null() || v.is_string())
            {
                return Err("采用图片需要提供预览时的当前图片，防止覆盖新选择".into());
            }
            let operation=digest(&json!({"token":token,"rowId":row_id,"assetId":asset["id"],"variantId":variant,"adopt":selection["adopt"],"expected":selection["expectedAdoptedMediaId"]}).to_string());
            let operation_key = format!("lingmo.media.completed:{operation}");
            if let Some(mut done) = metadata(store, &operation_key)? {
                store.verify_media(&store.get("media", s(&done, "mediaId"))?)?;
                done["status"] = json!("skipped");
                done["alreadyCompleted"] = json!(true);
                return Ok(done);
            }
            let path = Path::new(s(row, "path"));
            let root = Path::new(s(row, "root"));
            no_links(path)?;
            no_links(root)?;
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            if canonical != path
                || !canonical.starts_with(root.canonicalize().map_err(|e| e.to_string())?)
            {
                return Err("预览后素材路径已变化或越界".into());
            }
            let (hash, size) = hash_file(&canonical)?;
            if hash != s(row, "sha256") || Some(size) != row["size"].as_u64() {
                return Err("预览后文件内容已变化，请重新预览".into());
            }
            let cache_key = format!("{}:{hash}", s(row, "kind"));
            let before: bool = store
                .conn()?
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM media WHERE sha256=?1 AND kind=?2)",
                    params![hash, s(row, "kind")],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut media = if let Some(media) = media_cache.get(&cache_key) {
                store.verify_media(media)?;
                media.clone()
            } else {
                store.import_media(&canonical, s(row, "kind"), None, None)?
            };
            if s(&media, "sha256") != hash {
                return Err("导入期间文件内容已变化，未绑定资产".into());
            }
            let blank = placeholder(store, &media)?;
            media["placeholder"] = json!(blank);
            if s(row, "kind") == "image" {
                media["placeholderSource"] = json!("image-pixels/v1");
            }
            if s(row, "kind") == "audio" && media["metadataVerified"] != true {
                if !before {
                    return Err(format!(
                        "音频未通过本机媒体探测，原文件未改动：{}",
                        s(&media, "metadataError")
                    ));
                }
                // A previous attempt may have preserved the bytes before the
                // local ffprobe component was installed. Reuse those bytes and
                // recover their metadata instead of permanently caching failure.
                let metadata =
                    crate::media_probe::probe(Path::new(s(&media, "path")), &store.settings()?)
                        .map_err(|e| format!("音频未通过本机媒体探测，原文件未改动：{e}"))?;
                if !values(&metadata, "streams")
                    .iter()
                    .any(|stream| s(stream, "kind") == "audio")
                {
                    return Err("文件没有有效音频流，未绑定声音资产".into());
                }
                for (key, value) in metadata.as_object().ok_or("媒体探测未返回对象")? {
                    media[key] = value.clone();
                }
                media.as_object_mut().unwrap().remove("metadataError");
            }
            let media_id = s(&media, "id");
            let aid = s(asset, "id");
            let wants_adopt = selection["adopt"] == true && s(row, "kind") == "image" && !blank;
            let mut conn = store.conn()?;
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let live: String = tx
                .query_row("SELECT data FROM assets WHERE id=?1", [aid], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let live = parse(live)?;
            if !s(&live, "deletedAt").is_empty() || asset_snapshot(&live) != *old {
                return Err("导入期间资产版本已变化，未绑定原图".into());
            }
            tx.execute(
                "UPDATE media SET data=?1 WHERE id=?2",
                params![media.to_string(), media_id],
            )
            .map_err(|e| e.to_string())?;
            // The exclusion is link-specific: a shared file can be an ordinary
            // current image elsewhere while remaining an explicit candidate here.
            tx.execute(
                "INSERT INTO metadata(key,value) VALUES(?1,'true') ON CONFLICT(key) DO NOTHING",
                [format!("lingmo.media.candidate:{aid}:{variant}:{media_id}")],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT OR IGNORE INTO media_assets(media_id,asset_id,variant_id) VALUES(?1,?2,?3)",
                params![media_id, aid, variant],
            )
            .map_err(|e| e.to_string())?;
            let actual: Option<String> = tx
                .query_row(
                    "SELECT media_id FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
                    params![aid, variant],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let conflict =
                wants_adopt && actual.as_deref() != selection["expectedAdoptedMediaId"].as_str();
            let mut did_adopt = false;
            if wants_adopt && !conflict {
                set_current_image(
                    &tx,
                    aid,
                    variant,
                    media_id,
                    asset["version"].as_i64(),
                    false,
                )?;
                did_adopt = true;
            }
            let mut done = json!({"rowId":row_id,"status":if conflict {"adoption_conflict"}else if blank {"placeholder"}else {"imported"},"mediaId":media_id,"assetId":aid,"variantId":variant,"imported":true,"adopted":did_adopt,"placeholder":blank,"reused":before});
            if conflict {
                done["message"] =
                    json!("当前图片已被其他操作修改，原选择保留；本文件已作为候选入库");
            }
            if blank && selection["adopt"] == true {
                done["message"] = json!("白图仅作为占位候选，未采用为正式图片");
            }
            if !conflict {
                tx.execute(
                    "INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
                    params![operation_key, done.to_string()],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.execute("INSERT INTO metadata(key,value) VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            media_cache.insert(cache_key, media);
            Ok(done)
        })();
        let row = match result {
            Ok(v) => v,
            Err(message) => json!({"rowId":row_id,"status":"failed","message":message}),
        };
        if row["alreadyCompleted"] == true {
            skipped += 1;
        } else {
            if row["imported"] == true {
                imported += 1;
            }
            if row["adopted"] == true {
                adopted += 1;
            }
            if row["placeholder"] == true {
                placeholders += 1;
            }
            if row["reused"] == true {
                reused += 1;
            }
            if ["failed", "adoption_conflict"].contains(&s(&row, "status")) {
                failed += 1;
            }
        }
        result_rows.push(row);
    }
    Ok(
        json!({"documentId":document,"report":{"imported":imported,"adopted":adopted,"placeholders":placeholders,"skipped":skipped,"failed":failed,"reused":reused,"rows":result_rows,"milliseconds":start.elapsed().as_secs_f64()*1000.},"generationSubmissions":0}),
    )
}

#[cfg(test)]
#[path = "lingmo_media_tests.rs"]
mod tests;
