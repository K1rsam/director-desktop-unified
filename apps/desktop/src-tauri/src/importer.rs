use crate::store::{s, Result, Store};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub fn read_source(path: &Path) -> Result<String> {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.eq_ignore_ascii_case("docx"))
        .unwrap_or(false)
    {
        let mut zip = zip::ZipArchive::new(fs::File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut xml = String::new();
        zip.by_name("word/document.xml")
            .map_err(|e| e.to_string())?
            .take(64 * 1024 * 1024)
            .read_to_string(&mut xml)
            .map_err(|e| e.to_string())?;
        let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
        let mut out = String::new();
        for p in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "p")
        {
            for n in p.descendants().filter(|n| n.is_element()) {
                match n.tag_name().name() {
                    "t" => out.push_str(n.text().unwrap_or("")),
                    "tab" => out.push('\t'),
                    "br" | "cr" => out.push('\n'),
                    _ => {}
                }
            }
            out.push('\n');
        }
        Ok(out)
    } else {
        String::from_utf8(fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|_| "源文件必须为UTF-8文本或DOCX".into())
    }
}

/// File-name evidence applies only to an actual file import. Pasted text never
/// inherits a caller-supplied path or an unrelated document's title evidence.
pub fn prepare_source_import(text: Option<&str>, path: &Path) -> Result<crate::pipeline::LockedSource> {
    if let Some(text) = text {
        return crate::pipeline::prepare_source(text);
    }
    let text = read_source(path)?;
    let source = crate::pipeline::prepare_source(&text)?;
    let Some(stem) = path.file_stem().and_then(|name| name.to_str()).filter(|name| !name.is_empty()) else {
        return Ok(source);
    };
    let mut nonempty = source.rows.iter().filter(|row| !row.text.trim().is_empty());
    let Some(title) = nonempty.next() else { return Ok(source); };
    let Some(next) = nonempty.next() else { return Ok(source); };
    // Remove line terminators only: no whitespace, punctuation, spelling or
    // case normalization can turn an unrelated first line into a title.
    if title.text.trim_end_matches(['\r', '\n']) != stem || title.kind != "body"
        || title.episode_id.is_some() || title.scene_id.is_some() {
        return Ok(source);
    }
    let Some(first_episode) = source.episodes.first() else { return Ok(source); };
    let is_first = first_episode.source_number == "一"
        || first_episode.source_number.parse::<u32>() == Ok(1);
    let direct_first = next.kind == "format" && first_episode.row_ids.first() == Some(&next.id);
    // Existing parsing already treats episode zero as auxiliary context. This
    // exception changes only the cover row, never the zero-episode material.
    let zero_context = next.kind == "context" && next.episode_id.is_none()
        && regex::Regex::new(r"^(?:#{1,6}\s*)?第\s*(?:0+|零|〇)\s*集(?:\s|[:：｜|—-]|$)").unwrap().is_match(next.text.trim());
    if !is_first || !(direct_first || zero_context) { return Ok(source); }
    let evidence = json!({"version":"document-title/file-stem-exact/v1",
        "fileName":path.file_name().and_then(|name|name.to_str()),"sourceSha256":source.sha256,
        "titleRowId":title.id,"nextRowId":next.id,"start":title.range.start,"end":title.range.end,
        "nextBoundary":if direct_first {"first-production-episode"} else {"existing-zero-episode-context"},
        "firstProductionEpisodeId":first_episode.id});
    crate::pipeline::apply_boundary_decisions(&source, &[crate::pipeline::BoundaryDecision {
        start:title.range.start,end:title.range.end,kind:"context".into(),episode_number:None,scene_title:None,
        reason:format!("文件导入自动识别文档标题；{evidence}"),decision_version:None,confirmation:None,
    }])
}

#[cfg(test)]
#[path = "importer_source_title_tests.rs"]
mod source_title_tests;

fn legacy_paths(path: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let root = if path.is_dir() {
        path.to_path_buf()
    } else {
        let parent = path.parent().ok_or("无效旧项目路径")?;
        if parent.file_name().and_then(|n| n.to_str()) == Some("prisma") {
            parent.parent().unwrap_or(parent).to_path_buf()
        } else {
            parent.to_path_buf()
        }
    };
    let mut db = if path.is_file() {
        path.to_path_buf()
    } else {
        root.join("prisma/dev.db")
    };
    let mut refs = root.join("outputs/asset-references");
    let mut storyboard = root.join(".storyboard-stage3-v2");
    // Read only these path settings. No credentials are imported or exposed.
    for name in [".env.local", ".env"] {
        if let Ok(text) = fs::read_to_string(root.join(name)) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let value = v.trim().trim_matches(['\'', '"']);
                    match k.trim() {
                        "DATABASE_URL" => {
                            if let Some(p) = value.strip_prefix("file:") {
                                let p = PathBuf::from(p);
                                db = if p.is_absolute() {
                                    p
                                } else {
                                    root.join("prisma").join(p)
                                }
                            }
                        }
                        "DIRECTOR_ASSET_REFERENCE_DIR" => {
                            let p = PathBuf::from(value);
                            refs = if p.is_absolute() { p } else { root.join(p) }
                        }
                        "DIRECTOR_STORYBOARD_DATA_DIR" => {
                            let p = PathBuf::from(value);
                            storyboard = if p.is_absolute() { p } else { root.join(p) }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if !db.is_file() {
        for p in [
            root.join("prisma/dev.db"),
            root.join("prisma/director.db"),
            root.join("dev.db"),
        ] {
            if p.is_file() {
                db = p;
                break;
            }
        }
    }
    Ok((db, refs, storyboard))
}
pub fn inspect_legacy(path: &Path) -> Result<Value> {
    let (db, refs, storyboard) = legacy_paths(path)?;
    let c = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let mut q = c
        .prepare("SELECT id,title FROM Project ORDER BY createdAt")
        .map_err(|e| e.to_string())?;
    let projects = q
        .query_map([], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?}))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"dbPath":db,"referenceDirectory":refs,"storyboardDirectory":storyboard,"projects":projects}),
    )
}

pub fn import_legacy(store: &Store, path: &Path, options: &Value) -> Result<Value> {
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        return import_json(store, path);
    }
    let (db, refs, storyboard) = legacy_paths(path)?;
    let c = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("无法只读打开旧数据库: {e}"))?;
    let projects = inspect_legacy(path)?;
    let p = if !s(options, "legacyProjectId").is_empty() {
        projects["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| s(p, "id") == s(options, "legacyProjectId"))
            .cloned()
            .ok_or("旧项目不存在")?
    } else {
        let list = projects["projects"].as_array().unwrap();
        if list.len() != 1 {
            return Err(format!(
                "旧数据库含{}个项目，请先选择要导入的旧项目",
                list.len()
            ));
        }
        list[0].clone()
    };
    let pid = s(&p, "id");
    let mut ep = c
        .prepare("SELECT id,number,title,content FROM Episode WHERE projectId=?1 ORDER BY number")
        .map_err(|e| e.to_string())?;
    let episodes = ep
        .query_map([pid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut source = String::new();
    for (_, n, title, text) in &episodes {
        source.push_str(&format!("第{n}集 {}\n", title.as_deref().unwrap_or("")));
        source.push_str(text);
        source.push('\n');
    }
    if !source.is_empty() {
        let mut v = serde_json::to_value(crate::pipeline::prepare_source(&source)?)
            .map_err(|e| e.to_string())?;
        v["legacyProvenance"] = json!({"projectId":pid,"episodeBodies":episodes.iter().map(|(id,n,_,t)|json!({"id":id,"number":n,"sha256":crate::pipeline::digest(t)})).collect::<Vec<_>>(),"boundaryWrapperAdded":true});
        store.save_source(v)?;
    }
    let mut q=c.prepare("SELECT sceneKey,title,content FROM SceneSnapshot WHERE projectId=?1 ORDER BY createdAt,id").map_err(|e|e.to_string())?;
    let snapshots = q
        .query_map([pid], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut prompts = HashMap::<String, Value>::new();
    let mut imagejobs = Vec::new();
    let mut imageoptions = HashMap::new();
    let mut pointers = HashMap::new();
    let mut histories = HashMap::<String, Vec<Value>>::new();
    for (key, title, raw) in snapshots {
        let Some(key) = key else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        match key.as_str() {
            "ASSET_IMAGE_JOB_V1" => imagejobs.push(v),
            "ASSET_IMAGE_OPTIONS_V1" => {
                if let Some(key) = title {
                    imageoptions.insert(key, v);
                }
            }
            "ASSET_DESIGN_CURRENT_V1" => {
                pointers.insert(s(&v, "assetKey").to_string(), v);
            }
            "ASSET_DESIGN_ADOPTION_V1" => histories
                .entry(s(&v, "assetKey").to_string())
                .or_default()
                .push(v),
            _ => {
                if key.contains("ASSET") && !s(&v, "key").is_empty() && !s(&v, "body").is_empty() {
                    prompts.insert(s(&v, "key").to_string(), v);
                }
            }
        }
    }
    let mut diagnostics = Vec::new();
    let mut asset_count = 0;
    let mut media_ids = BTreeSet::new();
    let mut adopted_images = 0;
    let keys: BTreeSet<String> = imagejobs
        .iter()
        .map(|j| s(j, "key").to_owned())
        .chain(imageoptions.keys().cloned())
        .chain(pointers.keys().cloned())
        .chain(prompts.keys().cloned())
        .filter(|k| !k.is_empty() && !k.starts_with('$'))
        .collect();
    for key in keys {
        let aid = format!("legacy:{pid}:{key}");
        let current =
            pointers
                .get(&key)
                .and_then(|pointer| pointer["currentVersion"].as_i64())
                .and_then(|version| {
                    histories.get(&key)?.iter().rev().find(|v| {
                        v["version"].as_i64() == Some(version) && s(v, "action") == "ADOPT"
                    })
                });
        let jobs: Vec<&Value> = imagejobs.iter().filter(|j| s(j, "key") == key).collect();
        if store.get("assets", &aid).is_err() {
            let prompt = prompts
                .get(&key)
                .map(|p| s(p, "body"))
                .unwrap_or_else(|| jobs.last().map(|j| s(j, "prompt")).unwrap_or(""));
            let name = jobs
                .iter()
                .rev()
                .find_map(|j| j["name"].as_str())
                .unwrap_or(&key);
            store.save_asset(json!({"id":aid,"baseAssetId":key,"variantId":"legacy","name":name,"kind":"UNCLASSIFIED","prompt":prompt,"description":"从旧系统导入，身份类型、适用集场与派生来源待核对","legacyKey":key,"legacyProjectId":pid,"legacyAdoptionVersion":current.map(|v|v["version"].clone()),"applicability":{"episodes":[],"scenes":[]},"needsReview":true}))?;
            asset_count += 1;
        }
        // Manual references are stored in image options and adoption records, independently of image jobs.
        let mut images = Vec::new();
        for job in &jobs {
            if let Some(v) = job["images"].as_array() {
                images.extend(v.iter().cloned());
            }
        }
        if let Some(v) = imageoptions
            .get(&key)
            .and_then(|v| v["references"].as_array())
        {
            images.extend(v.iter().cloned());
        }
        if let Some(v) = current.and_then(|v| v["references"].as_array()) {
            images.extend(v.iter().cloned());
        }
        if let Some(ids) = current.and_then(|v| v["referenceIds"].as_array()) {
            for id in ids {
                if let Some(id) = id.as_str() {
                    if !images.iter().any(|v| s(v, "id") == id) {
                        images.push(json!({"id":id}));
                    }
                }
            }
        }
        let mut by_id = BTreeMap::<String, Vec<Value>>::new();
        for image in images {
            by_id
                .entry(s(&image, "id").to_owned())
                .or_default()
                .push(image);
        }
        let mut mapped = HashMap::new();
        for (image_id, records) in by_id {
            if image_id.is_empty()
                || image_id.len() > 128
                || !image_id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                diagnostics.push(
                    json!({"code":"LEGACY_MEDIA_ID_INVALID","imageId":image_id,"assetId":aid}),
                );
                continue;
            }
            let file = refs.join(&image_id);
            let Ok(metadata) = fs::symlink_metadata(&file) else {
                diagnostics
                    .push(json!({"code":"LEGACY_MEDIA_MISSING","imageId":image_id,"assetId":aid}));
                continue;
            };
            if !metadata.file_type().is_file() {
                diagnostics.push(json!({"code":"LEGACY_MEDIA_NOT_REGULAR","imageId":image_id}));
                continue;
            }
            let (hash, size) = crate::store::hash_file(&file)?;
            if records.iter().any(|v| {
                !s(v, "sha256").is_empty() && !s(v, "sha256").eq_ignore_ascii_case(&hash)
                    || v["size"].as_u64().is_some_and(|expected| expected != size)
            }) {
                diagnostics.push(
                    json!({"code":"LEGACY_HASH_OR_SIZE_MISMATCH","imageId":image_id,"assetId":aid}),
                );
                continue;
            }
            match store.import_media(&file,"image",Some(&aid),Some("legacy")){Ok(media)=>{let mid=s(&media,"id").to_owned();mapped.insert(image_id.clone(),mid.clone());media_ids.insert(mid);},Err(error)=>diagnostics.push(json!({"code":"LEGACY_MEDIA_IMPORT_FAILED","imageId":image_id,"assetId":aid,"message":error}))}
        }
        if let Some(current) = current {
            let ids = current["referenceIds"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if ids.len() == 1 {
                if let Some(mid) = ids[0].as_str().and_then(|i| mapped.get(i)) {
                    store.adopt(&aid, "legacy", mid)?;
                    adopted_images += 1;
                } else {
                    diagnostics.push(json!({"code":"LEGACY_ADOPTED_MEDIA_UNAVAILABLE","assetKey":key,"referenceIds":ids}));
                }
            } else if ids.len() > 1 {
                diagnostics.push(json!({"code":"LEGACY_MULTIPLE_ADOPTED","assetKey":key,"referenceIds":ids,"message":"已导入所有可用候选，但旧采用含多图，需明确正式参考图的用途与顺序"}));
            }
        }
    }
    let mut documents = Vec::new();
    let directory = storyboard.join("documents");
    if directory.is_dir() {
        let mut files = fs::read_dir(directory)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        files.sort();
        for file in files {
            match read_legacy_document(&file) {
                Ok(document) => {
                    if document.project_id == pid {
                        documents.push(document);
                    }
                }
                Err(error) => diagnostics
                    .push(json!({"code":"LEGACY_DOCUMENT_UNREADABLE","file":file,"message":error})),
            }
        }
    }
    let valid_episodes: BTreeSet<String> =
        episodes.iter().map(|(id, _, _, _)| id.clone()).collect();
    let (unit_count, group_count) = import_documents(
        store,
        pid,
        documents,
        Some(&valid_episodes),
        &mut diagnostics,
    )?;
    save_report(
        store,
        json!({"projectId":pid,"assets":asset_count,"media":media_ids.len(),"adoptedImages":adopted_images,"units":unit_count,"adoptedGroups":group_count,"diagnostics":diagnostics,"credentialsImported":false,"oldFilesChanged":false,"note":"已采用旧分镜仍需确认原文引用及assetId、variantId；图片身份类型和适用范围待核对，导入不猜测映射"}),
    )
}
struct LegacyDocument {
    file: PathBuf,
    value: Value,
    project_id: String,
    episode_id: String,
    document_id: String,
    adopted_at: i64,
    ordering_source: String,
}
fn timestamp(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis())
}
fn adopted_groups(value: &Value) -> Vec<&Value> {
    value["adopted"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter(|g| {
                    g.pointer("/draft/units")
                        .and_then(Value::as_array)
                        .is_some_and(|units| !units.is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}
fn read_legacy_document(file: &Path) -> Result<LegacyDocument> {
    let metadata = fs::symlink_metadata(file).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("旧分镜文档不是普通文件或超过64MiB".into());
    }
    let value: Value = serde_json::from_slice(&fs::read(file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let project_id = value
        .pointer("/web/binding/projectId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let episode_id = value
        .pointer("/web/binding/episodeId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let document_id = value["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| {
            file.file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("document")
        })
        .to_owned();
    // Current groups refer to candidates; only their own settled/recovered operations determine adoption age.
    let mut dates = Vec::new();
    for group in adopted_groups(&value) {
        if let Some(t) = timestamp(&group["adoptedAt"]) {
            dates.push(t)
        }
        let ids = std::iter::once(group["candidateId"].as_str())
            .chain(
                group["candidateIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(Value::as_str),
            )
            .flatten()
            .collect::<BTreeSet<_>>();
        if let Some(candidates) = value["candidates"].as_array() {
            for candidate in candidates.iter().filter(|c| ids.contains(s(c, "id"))) {
                if let Some(t) = timestamp(&candidate["recoveredAt"]) {
                    dates.push(t)
                }
                if let Some(operations) = value["operations"].as_array() {
                    for op in operations
                        .iter()
                        .filter(|op| s(op, "attemptId") == s(candidate, "attemptId"))
                    {
                        if let Some(t) = timestamp(&op["localSettledAt"])
                            .or_else(|| timestamp(&op["completedAt"]))
                            .or_else(|| timestamp(&op["createdAt"]))
                        {
                            dates.push(t)
                        }
                    }
                }
            }
        }
    }
    let (adopted_at, ordering_source) = if let Some(t) = dates.into_iter().max() {
        (t, "adopted-operation".to_owned())
    } else if let Some(t) = timestamp(&value["web"]["createdAt"]) {
        (t, "document-createdAt-fallback".to_owned())
    } else {
        (
            metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|v| v.as_millis().min(i64::MAX as u128) as i64)
                .unwrap_or(0),
            "file-mtime-fallback".to_owned(),
        )
    };
    Ok(LegacyDocument {
        file: file.to_path_buf(),
        value,
        project_id,
        episode_id,
        document_id,
        adopted_at,
        ordering_source,
    })
}
fn product_summary(value: &Value) -> Value {
    let count = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0)
    };
    let current_ids: BTreeSet<&str> = adopted_groups(value)
        .iter()
        .flat_map(|g| {
            std::iter::once(g["candidateId"].as_str())
                .chain(
                    g["candidateIds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(Value::as_str),
                )
                .flatten()
        })
        .collect();
    json!({"resultUnits":count("/result/units"),"outputUnits":count("/output/units"),"finishedUnits":count("/finished/units"),"assemblyUnits":count("/assembly/draft/units"),"unusedCandidates":value["candidates"].as_array().map(|items|items.iter().filter(|c|!current_ids.contains(s(c,"id"))).map(|c|json!({"id":c["id"],"status":c["status"],"stage":c["stage"]})).collect::<Vec<_>>()).unwrap_or_default()})
}
fn import_documents(
    store: &Store,
    pid: &str,
    documents: Vec<LegacyDocument>,
    valid_episodes: Option<&BTreeSet<String>>,
    diagnostics: &mut Vec<Value>,
) -> Result<(usize, usize)> {
    let mut by_episode = BTreeMap::<String, Vec<LegacyDocument>>::new();
    for document in documents {
        if document.project_id != pid {
            continue;
        }
        let groups = adopted_groups(&document.value);
        if groups.is_empty() {
            diagnostics.push(json!({"code":"LEGACY_UNADOPTED_DOCUMENT","documentId":document.document_id,"file":document.file,"artifacts":product_summary(&document.value),"message":"未找到当前明确已采用的draft.units；成果仅记入报告，不作为正式分镜"}));
            continue;
        }
        if document.episode_id.is_empty()
            || valid_episodes.is_some_and(|ids| !ids.contains(&document.episode_id))
        {
            diagnostics.push(json!({"code":"LEGACY_EPISODE_UNBOUND","documentId":document.document_id,"file":document.file,"legacyEpisodeId":document.episode_id}));
            continue;
        }
        by_episode
            .entry(document.episode_id.clone())
            .or_default()
            .push(document);
    }
    let source = store.source()?;
    let mut imported = 0;
    let mut group_count = 0;
    for (legacy_episode, mut documents) in by_episode {
        documents.sort_by(|a, b| {
            a.adopted_at
                .cmp(&b.adopted_at)
                .then_with(|| a.file.cmp(&b.file))
        });
        let selected = documents.pop().unwrap();
        for older in documents {
            diagnostics.push(json!({"code":if older.adopted_at==selected.adopted_at{"LEGACY_DOCUMENT_ORDER_AMBIGUOUS"}else{"LEGACY_SUPERSEDED_DOCUMENT"},"legacyEpisodeId":legacy_episode,"documentId":older.document_id,"file":older.file,"selectedDocumentId":selected.document_id,"selectedFile":selected.file,"adoptedGroups":adopted_groups(&older.value).len(),"artifacts":product_summary(&older.value),"message":"同集仅以最新已采用文档为导入基准；其它文档保留原文件与报告"}));
        }
        if selected.ordering_source != "adopted-operation" {
            diagnostics.push(json!({"code":"LEGACY_ADOPTION_TIME_INFERRED","documentId":selected.document_id,"file":selected.file,"orderingSource":selected.ordering_source,"message":"旧稿未保存可关联采用时间，排序使用文档时间证据，需复核同集基准"}));
        }
        let episode_number = selected
            .value
            .pointer("/web/binding/episodeNumber")
            .and_then(Value::as_u64);
        let matches: Vec<&Value> = source["episodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| {
                episode_number.is_some()
                    && e["sourceNumber"]
                        .as_str()
                        .and_then(|n| n.parse::<u64>().ok())
                        == episode_number
            })
            .collect();
        let episode_id = if matches.len() == 1 {
            s(matches[0], "id").to_owned()
        } else {
            format!("legacy:{pid}:episode:{legacy_episode}")
        };
        let doc_namespace = crate::pipeline::digest(
            &serde_json::to_string(&json!([
                pid,
                selected.document_id,
                selected.file.file_name()
            ]))
            .unwrap(),
        );
        let groups = adopted_groups(&selected.value);
        let mut unit_contents = BTreeMap::<String, BTreeSet<String>>::new();
        for group in &groups {
            for unit in group["draft"]["units"].as_array().unwrap() {
                if !s(unit, "id").is_empty() {
                    unit_contents
                        .entry(s(unit, "id").to_owned())
                        .or_default()
                        .insert(crate::pipeline::digest(&unit.to_string()));
                }
            }
        }
        let conflicts: BTreeSet<String> = unit_contents
            .into_iter()
            .filter(|(_, contents)| contents.len() > 1)
            .map(|(id, _)| id)
            .collect();
        for id in &conflicts {
            diagnostics.push(json!({"code":"LEGACY_UNIT_ID_CONFLICT","legacyUnitId":id,"documentId":selected.document_id,"file":selected.file,"message":"不同采用组包含同ID且内容不同的单元；均按组命名空间保留，需人工复核"}));
        }
        for (group_index, group) in groups.iter().enumerate() {
            group_count += 1;
            let group_namespace = crate::pipeline::digest(
                &json!([group["packageIds"], group["candidateId"], group_index]).to_string(),
            );
            for (unit_index, original) in group["draft"]["units"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                if !original.is_object() {
                    diagnostics.push(json!({"code":"LEGACY_UNIT_INVALID","documentId":selected.document_id,"groupIndex":group_index,"unitIndex":unit_index}));
                    continue;
                }
                let legacy_id = original["id"].clone();
                let fingerprint = crate::pipeline::digest(&original.to_string());
                let unit_namespace =
                    crate::pipeline::digest(&json!([legacy_id, unit_index]).to_string());
                let id = format!(
                    "legacy:{pid}:document:{}:group:{}:unit:{}",
                    &doc_namespace[..16],
                    &group_namespace[..16],
                    &unit_namespace[..16]
                );
                if store
                    .get("units", &id)
                    .ok()
                    .is_some_and(|existing| s(&existing, "legacyUnitFingerprint") == fingerprint)
                {
                    imported += 1;
                    continue;
                }
                let mut unit = original.clone();
                unit["id"] = json!(id);
                unit["episodeId"] = json!(episode_id);
                unit["legacyUnitId"] = legacy_id;
                unit["legacyProjectId"] = json!(pid);
                unit["legacyEpisodeId"] = json!(legacy_episode);
                unit["legacyDocumentId"] = json!(selected.document_id);
                unit["legacyGroupIndex"] = json!(group_index);
                unit["legacyPackageIds"] = group["packageIds"].clone();
                unit["legacyFile"] = json!(selected.file);
                unit["legacyUnitFingerprint"] = json!(fingerprint);
                unit["legacyIdConflict"] = json!(conflicts.contains(s(original, "id")));
                if s(&unit, "prompt").is_empty() {
                    unit["prompt"] = unit
                        .get("body")
                        .or(unit.get("text"))
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| json!(s))
                        .unwrap_or_else(|| json!(crate::pipeline::render_legacy_unit(original)));
                }
                if unit["duration"].as_f64().is_none() {
                    if let Some(shots) = unit["shots"].as_array() {
                        let durations: Option<Vec<f64>> = shots
                            .iter()
                            .map(|shot| {
                                shot["duration"]
                                    .as_f64()
                                    .filter(|v| v.is_finite() && *v > 0.0)
                            })
                            .collect();
                        if let Some(durations) = durations {
                            let duration: f64 = durations.iter().sum();
                            if duration.is_finite() && duration > 0.0 {
                                unit["duration"] = json!(duration);
                            }
                        }
                    }
                }
                unit["legacySourceRefs"] = unit["sourceRefs"].clone();
                unit["legacyAssetRefs"] = unit["assetRefs"].clone();
                unit["legacySceneId"] = unit["sceneId"].clone();
                unit.as_object_mut().unwrap().remove("sceneId");
                unit.as_object_mut().unwrap().remove("preparedPackage");
                unit["sourceRefs"] = json!([]);
                unit["assetRefs"] = json!([]);
                unit["needsSourceMapping"] = json!(true);
                unit["needsAssetMapping"] = json!(true);
                store.save_unit(unit)?;
                imported += 1;
            }
        }
        diagnostics.push(json!({"code":"LEGACY_SELECTED_DOCUMENT","documentId":selected.document_id,"file":selected.file,"legacyEpisodeId":legacy_episode,"adoptedGroups":groups.len(),"ignoredAlternativeArtifacts":product_summary(&selected.value),"needsSourceMapping":true,"needsAssetMapping":true}));
    }
    Ok((imported, group_count))
}
fn save_report(store: &Store, mut report: Value) -> Result<Value> {
    let id = crate::store::id();
    report["reportId"] = json!(id);
    store
        .conn()?
        .execute(
            "INSERT INTO metadata(key,value) VALUES(?1,?2)",
            [format!("legacy_import_report:{id}"), report.to_string()],
        )
        .map_err(|e| e.to_string())?;
    Ok(report)
}
fn import_json(store: &Store, path: &Path) -> Result<Value> {
    let v: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if let Some(pid) = v.pointer("/web/binding/projectId").and_then(Value::as_str) {
        let mut diagnostics = Vec::new();
        let (units, groups) = import_documents(
            store,
            pid,
            vec![read_legacy_document(path)?],
            None,
            &mut diagnostics,
        )?;
        return save_report(
            store,
            json!({"projectId":pid,"assets":0,"media":0,"units":units,"adoptedGroups":groups,"diagnostics":diagnostics,"oldFilesChanged":false,"credentialsImported":false}),
        );
    }
    if let Some(text) = v["source"].as_str().or(v["source"]["text"].as_str()) {
        store.save_source(
            serde_json::to_value(crate::pipeline::prepare_source(text)?)
                .map_err(|e| e.to_string())?,
        )?;
    }
    for a in v["assets"].as_array().unwrap_or(&Vec::new()) {
        store.save_asset(a.clone())?;
    }
    for u in v["units"].as_array().unwrap_or(&Vec::new()) {
        store.save_unit(u.clone())?;
    }
    Ok(
        json!({"assets":v["assets"].as_array().map(Vec::len),"units":v["units"].as_array().map(Vec::len)}),
    )
}
