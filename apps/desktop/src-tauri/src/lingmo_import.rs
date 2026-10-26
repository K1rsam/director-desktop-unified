//! External finished storyboards have their own immutable provenance. They are
//! never inserted into locked script sources or creative adoption receipts.
use crate::{
    lingmo_parser,
    pipeline::digest,
    store::{now, parse, s, save_asset_tx, save_unit_tx, Result, Store},
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Component, Path},
    time::Instant,
};

const PREFIX: &str = "lingmo.document:";
const MAX_BYTES: u64 = 64 * 1024 * 1024;
fn array(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}
fn read_text(path: &Path) -> Result<String> {
    let file = fs::File::open(path).map_err(|e| format!("无法读取 {}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("导入文本超过64MiB上限".into());
    }
    String::from_utf8(bytes).map_err(|e| format!("请使用UTF-8编码的MD/目录：{e}"))
}
fn read_input(
    request: &Value,
) -> Result<(
    Value,
    Value,
    String,
    Option<crate::lingmo_package::VerifiedPackage>,
)> {
    let text = if let Some(text) = request["text"].as_str() {
        if text.len() as u64 > MAX_BYTES {
            return Err("文本过大".into());
        }
        text.to_owned()
    } else {
        read_text(Path::new(s(request, "path")))?
    };
    let filename = Path::new(s(request, "path"))
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("粘贴分镜.md")
        .to_owned();
    let auto = format!("{}.assets.json", s(request, "path"));
    let path = if !s(request, "catalogPath").is_empty() {
        Some(s(request, "catalogPath"))
    } else if !s(request, "path").is_empty() && Path::new(&auto).is_file() {
        Some(auto.as_str())
    } else {
        None
    };
    let catalog = path
        .map(|path| read_text(Path::new(path)).and_then(parse))
        .transpose()?
        .unwrap_or(Value::Null);
    let package = crate::lingmo_package::discover(request, &text, &catalog)?;
    let catalog = if catalog.is_null() { package.as_ref().map(|p|p.value["catalog"].clone()).unwrap_or(catalog) } else { catalog };
    let mut parsed = lingmo_parser::parse(&text, (!catalog.is_null()).then_some(&catalog))?;
    parsed["handoff"] = if let Some(package) = &package {
        json!({"sourceKind":"external-package","completePackageVerified":true,"proof":package.proof()})
    } else {
        json!({"sourceKind":"imported-md","completePackageVerified":false,
            "level":if parsed["catalogMatched"]==true{"md-and-catalog"}else{"md-only"}})
    };
    if package.is_none() && (catalog.get("productionScope").is_some() || catalog.get("scopeHash").is_some() || request.get("module22Mapping").is_some() || !s(request,"module22Root").is_empty()) {
        parsed["handoff"]["scopeRestriction"] = json!("native-season-or-package-evidence-required");
    }
    Ok((parsed, catalog, filename, package))
}
fn preview_token(parsed: &Value, catalog: &Value) -> String {
    let base = format!("{}:{}", s(parsed, "markdownSha256"), catalog);
    if parsed["handoff"]["completePackageVerified"] == true {
        digest(&format!(
            "{base}:{}",
            s(&parsed["handoff"]["proof"], "packageFingerprint")
        ))
    } else {
        digest(&base)
    }
}
fn metadata(store: &Store, key: &str) -> Result<Value> {
    let raw: Option<String> = store
        .conn()?
        .query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(parse)
        .transpose()?
        .ok_or_else(|| "导入原件或索引不存在，请恢复完整项目包".into())
}
pub fn documents(store: &Store) -> Result<Vec<Value>> {
    let c = store.conn()?;
    let mut q = c
        .prepare("SELECT value FROM metadata WHERE key LIKE 'lingmo.document:%' ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.map(|r| parse(r.map_err(|e| e.to_string())?)).collect()
}
pub fn document(store: &Store, id: &str) -> Result<Value> {
    metadata(store, &format!("{PREFIX}{id}"))
}
/// Global entry point: no project, database or generation channel is needed.
pub fn preview_file(request: &Value) -> Result<Value> {
    if s(request, "path").is_empty() || request.get("text").is_some() {
        return Err("请选择本机分镜MD文件".into());
    }
    let start = Instant::now();
    let (parsed, catalog, filename, _) = read_input(request)?;
    let mut output = summary(&parsed, &filename);
    output["previewToken"] = json!(preview_token(&parsed, &catalog));
    output["parseMilliseconds"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    output["generationSubmissions"] = json!(0);
    Ok(output)
}

/// Validate the exact preview before creating any project directory. Import
/// repeats these checks at its own transaction boundary in case the file changes.
pub fn validate_project_input(request: &Value) -> Result<Value> {
    let preview = preview_file(request)?;
    if s(request, "expectedSha256") != s(&preview, "markdownSha256") {
        return Err("预览后MD内容已变化，请重新预览后导入".into());
    }
    if s(request, "previewToken").is_empty() || request["previewToken"] != preview["previewToken"] {
        return Err("缺少有效预览或资产目录已变化，请重新预览".into());
    }
    if preview["counts"]["fragments"].as_u64().unwrap_or(0) == 0 {
        return Err("未识别到有效片段，未创建项目".into());
    }
    if array(&preview, "diagnostics").iter().any(|d| {
        [
            "duplicate_number",
            "scene_without_episode",
            "scene_episode_mismatch",
            "unit_without_scene",
        ]
        .contains(&s(d, "code"))
    }) {
        return Err("MD存在集场/片段结构冲突，请按预览行号修正；未创建项目".into());
    }
    Ok(preview)
}
pub fn preview(store: &Store, request: &Value) -> Result<Value> {
    let start = Instant::now();
    let (parsed, catalog, filename, _) = read_input(request)?;
    let mut summary = summary(&parsed, &filename);
    summary["previewToken"] = json!(preview_token(&parsed, &catalog));
    summary["parseMilliseconds"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    let prior: Vec<_> = documents(store)?
        .into_iter()
        .filter(|d| s(d, "filename") == filename && d["markdownSha256"] != parsed["markdownSha256"])
        .collect();
    summary["revisionComparison"] = json!({"existingDocumentIds":prior.iter().map(|v|v["id"].clone()).collect::<Vec<_>>(),"previousCounts":prior.iter().map(|v|v["counts"].clone()).collect::<Vec<_>>(),"newCounts":parsed["counts"],"policy":"retain-existing-import-new-version"});
    let mut comparisons = Vec::new();
    for old in &prior {
        let old_sha = s(old, "id");
        let old_prefix = format!("lingmo-{old_sha}");
        let new_prefix = format!("lingmo-{}", s(&parsed, "markdownSha256"));
        let origins = metadata(store, &format!("lingmo.origins:{old_sha}"))?;
        let old_units: std::collections::HashMap<_, _> = origins
            .as_object()
            .into_iter()
            .flatten()
            .map(|(id, v)| (id.trim_start_matches(&old_prefix).to_owned(), (id, v)))
            .collect();
        let mut changes = Vec::new();
        let mut added = 0;
        let mut changed = 0;
        let mut edited = 0;
        let mut seen = std::collections::HashSet::new();
        for unit in array(&parsed, "units") {
            let key = s(&unit, "id").trim_start_matches(&new_prefix).to_owned();
            seen.insert(key.clone());
            match old_units.get(&key) {
                Some((id, origin)) if origin["promptSha256"] != digest(s(&unit, "prompt")) => {
                    changed += 1;
                    let current = store.get("units", id)?;
                    let has_edit = digest(s(&current, "prompt"))
                        != origin["promptSha256"].as_str().unwrap_or("");
                    if has_edit {
                        edited += 1;
                    }
                    changes.push(json!({"episode":unit["episodeNumber"],"scene":unit["sceneNumber"],"fragment":unit["fragmentNumber"],"kind":"changed","existingEdited":has_edit}));
                }
                None => {
                    added += 1;
                    changes.push(json!({"episode":unit["episodeNumber"],"scene":unit["sceneNumber"],"fragment":unit["fragmentNumber"],"kind":"added"}));
                }
                _ => {}
            }
        }
        let removed = old_units.keys().filter(|key| !seen.contains(*key)).count();
        comparisons.push(json!({"documentId":old_sha,"changedFragments":changed,"addedFragments":added,"removedFragments":removed,"editedExistingFragments":edited,"locations":changes}));
    }
    summary["revisionComparison"]["changes"] = json!(comparisons);
    Ok(summary)
}
fn summary(parsed: &Value, filename: &str) -> Value {
    let mut v =
        json!({"id":parsed["markdownSha256"],"filename":filename,"origin":"imported-storyboard"});
    for key in [
        "markdownSha256",
        "parserVersion",
        "counts",
        "catalogMatched",
        "diagnostics",
        "episodes",
        "scenes",
        "handoff",
    ] {
        v[key] = parsed[key].clone();
    }
    v
}
pub fn import(store: &Store, request: &Value) -> Result<Value> {
    let start = Instant::now();
    let (mut parsed, catalog, filename, package) = read_input(request)?;
    let parse_ms = start.elapsed().as_secs_f64() * 1000.0;
    let sha = s(&parsed, "markdownSha256").to_owned();
    if s(request, "expectedSha256") != sha {
        return Err("预览后MD内容已变化，请重新预览后导入".into());
    }
    if !s(request, "previewToken").is_empty()
        && s(request, "previewToken") != preview_token(&parsed, &catalog)
    {
        return Err("预览后资产目录或完整接续包已变化，请重新预览".into());
    }
    if array(&parsed, "units").is_empty() {
        return Err("未识别到有效片段，未写入项目".into());
    }
    if array(&parsed, "diagnostics").iter().any(|d| {
        [
            "duplicate_number",
            "scene_without_episode",
            "scene_episode_mismatch",
            "unit_without_scene",
        ]
        .contains(&s(d, "code"))
    }) {
        return Err("MD存在集场/片段结构冲突，请按预览行号修正；未写入项目".into());
    }
    let existing = documents(store)?;
    let duplicate = existing.iter().any(|d| s(d, "id") == sha);
    if !duplicate
        && request["allowNewRevision"] != true
        && existing.iter().any(|d| s(d, "filename") == filename)
    {
        return Err("同名MD存在不同版本；请核对预览差异，选择保留旧稿并新增修订版".into());
    }
    let mut doc = summary(&parsed, &filename);
    doc["importedAt"] = json!(now());
    let package_pointer = package
        .as_ref()
        .map(|p| crate::lingmo_package::prepare_storage(store, p))
        .transpose()?;
    if !duplicate {
        let mut connection = store.conn()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        // Check again inside the transaction. Never overwrite an existing import.
        if tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM metadata WHERE key=?1)",
                [format!("{PREFIX}{sha}")],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|e| e.to_string())?
        {
            return Err("该导入已由另一窗口保存，请刷新".into());
        }
        let preserved = crate::lingmo_continuation::continue_assets(&tx, &mut parsed, &catalog)?;
        if let Some(package) = &package {
            crate::lingmo_continuation::attach_package(
                &tx,
                &mut parsed,
                &catalog,
                &package.value,
                &preserved,
            )?;
        }
        doc = summary(&parsed, &filename);
        doc["importedAt"] = json!(now());
        for asset in array(&parsed, "assets") {
            if !preserved.contains(s(&asset, "id")) {
                save_asset_tx(&tx, asset)?;
            }
        }
        let mut origins = serde_json::Map::new();
        for mut unit in array(&parsed, "units") {
            unit["importSource"]["documentId"] = json!(sha);
            if package.is_some() {
                unit["importSource"]["sourceKind"] = json!("external-package");
                unit["importSource"]["packageFingerprint"] =
                    parsed["handoff"]["proof"]["packageFingerprint"].clone();
            }
            unit["sourceRefs"] = json!([]);
            origins.insert(s(&unit,"id").to_owned(),json!({"importSource":unit["importSource"],"episodeId":unit["episodeId"],"sceneId":unit["sceneId"],"fragmentNumber":unit["fragmentNumber"],"promptSha256":digest(s(&unit,"prompt"))}));
            save_unit_tx(&tx, unit)?;
        }
        for (key, value) in [
            (format!("{PREFIX}{sha}"), doc.clone()),
            (
                format!("lingmo.original:{sha}"),
                json!({"text":parsed["originalText"],"sha256":sha,"catalog":catalog}),
            ),
            (format!("lingmo.origins:{sha}"), Value::Object(origins)),
        ] {
            tx.execute(
                "INSERT INTO metadata(key,value) VALUES(?1,?2)",
                params![key, value.to_string()],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(pointer) = &package_pointer {
            crate::lingmo_package::save_pointer(&tx, &sha, pointer)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
    } else {
        doc = document(store, &sha)?;
        // Same MD with a changed sidecar must not silently remap existing edits.
        let old = metadata(store, &format!("lingmo.original:{sha}"))?;
        if old["catalog"] != catalog {
            return Err(
                "此MD已导入，但资产目录不同；原身份和编辑已保留，请使用原同版目录补图".into(),
            );
        }
        let mut connection = store.conn()?;
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        crate::lingmo_continuation::restore_assets(&tx, &mut parsed, &catalog)?;
        if let Some(pointer) = &package_pointer {
            crate::lingmo_package::save_pointer(&tx, &sha, pointer)?;
            // Package-level evidence can be added to an old exact MD/catalog
            // import without re-saving asset rows, units or frozen requests.
            doc["handoff"] = parsed["handoff"].clone();
            tx.execute(
                "UPDATE metadata SET value=?1 WHERE key=?2",
                params![doc.to_string(), format!("{PREFIX}{sha}")],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
    }
    // Text is committed as a single transaction. Optional media is independently
    // resumable; a bad/missing image cannot destroy an otherwise valid document.
    parsed["originalText"] = Value::Null;
    let text_ms = start.elapsed().as_secs_f64() * 1000.0;
    let media_report = import_images(store, request, &parsed).unwrap_or_else(
        |message| json!({"failures":[{"message":message}],"textImportPreserved":true}),
    );
    Ok(
        json!({"documentId":sha,"duplicate":duplicate,"counts":doc["counts"],"handoff":doc["handoff"],"media":media_report,"parseMilliseconds":parse_ms,"textImportMilliseconds":text_ms,"mediaImportMilliseconds":start.elapsed().as_secs_f64()*1000.0-text_ms,"milliseconds":start.elapsed().as_secs_f64()*1000.0,"generationSubmissions":0}),
    )
}
pub fn scene(store: &Store, document_id: &str, scene_id: &str) -> Result<Value> {
    let doc = document(store, document_id)?;
    let scene = array(&doc, "scenes")
        .into_iter()
        .find(|sc| s(sc, "id") == scene_id)
        .ok_or("场次不属于此导入文档")?;
    let units = array(&scene, "unitIds")
        .iter()
        .map(|id| store.get("units", id.as_str().unwrap_or("")))
        .collect::<Result<Vec<_>>>()?;
    let mut ids: Vec<String> = array(&scene, "assetIds")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    for unit in &units {
        for m in array(unit, "mentions") {
            let id = s(&m, "assetId");
            if !id.is_empty() && !ids.iter().any(|v| v == id) {
                ids.push(id.to_owned());
            }
        }
    }
    let assets = ids
        .iter()
        .filter_map(|id| store.get("assets", id).ok())
        .collect::<Vec<_>>();
    Ok(json!({"scene":scene,"units":units,"assets":assets}))
}
pub fn original(store: &Store, document_id: &str, unit_id: Option<&str>) -> Result<Value> {
    let original = metadata(store, &format!("lingmo.original:{document_id}"))?;
    let text = s(&original, "text");
    if digest(text) != document_id {
        return Err("导入原件摘要不符，未使用损坏原件".into());
    }
    if let Some(id) = unit_id.filter(|id| !id.is_empty()) {
        let origins = metadata(store, &format!("lingmo.origins:{document_id}"))?;
        let range = &origins[id]["importSource"];
        let start = range["byteStart"].as_u64().ok_or("片段原文范围缺失")? as usize;
        let end = range["byteEnd"].as_u64().ok_or("片段原文范围缺失")? as usize;
        let text = text.get(start..end).ok_or("片段原文范围越界")?;
        return Ok(json!({"text":text,"sha256":digest(text),"documentSha256":document_id}));
    }
    Ok(json!({"text":text,"sha256":document_id}))
}
pub fn validate_unit(store: &Store, unit: &Value) -> Result<()> {
    if s(unit, "origin") != "imported-storyboard" {
        return Err("非外部导入单元".into());
    }
    let sha = s(&unit["importSource"], "markdownSha256");
    let origins = metadata(store, &format!("lingmo.origins:{sha}"))?;
    let origin = &origins[s(unit, "id")];
    if origin.is_null()
        || origin["importSource"] != unit["importSource"]
        || origin["episodeId"] != unit["episodeId"]
        || origin["sceneId"] != unit["sceneId"]
        || origin["fragmentNumber"] != unit["fragmentNumber"]
    {
        return Err("外部成稿来源或集场归属不匹配，未绕过原文检查".into());
    }
    let original = original(store, sha, Some(s(unit, "id")))?;
    if original["sha256"] != origin["promptSha256"] {
        return Err("片段原文摘要不一致".into());
    }
    Ok(())
}
pub fn save_unit(store: &Store, original: &Value, mut unit: Value) -> Result<Value> {
    if original["origin"] != "imported-storyboard" {
        return Err("不能把普通单元改为外部导入来源".into());
    }
    validate_unit(store, &unit)?;
    for key in [
        "sourceVersion",
        "sourceRefs",
        "sourceSha256",
        "storyboardCreativeProfile",
    ] {
        if unit.get(key) != original.get(key) {
            return Err("外部成稿不能改写锁定原文或分镜来源".into());
        }
    }
    if unit["expectedVersion"].as_i64() != original["version"].as_i64() {
        return Err("单元版本已变化，请刷新后保存".into());
    }
    let old_refs = array(&unit, "assetRefs");
    let diagnostics = lingmo_parser::rebind_unit(&mut unit, &store.values("assets")?);
    // Keep the user's explicit ordering and media choice, only for identities
    // actually present in the edited text. New mentions append deterministically.
    let mut refs = array(&unit, "assetRefs");
    refs.sort_by_key(|r| {
        old_refs
            .iter()
            .position(|old| old["assetId"] == r["assetId"] && old["variantId"] == r["variantId"])
            .unwrap_or(usize::MAX)
    });
    for (i, r) in refs.iter_mut().enumerate() {
        r["order"] = json!(i + 1);
        if let Some(old) = old_refs
            .iter()
            .find(|old| old["assetId"] == r["assetId"] && old["variantId"] == r["variantId"])
        {
            if old.get("mediaId").is_some() {
                r["mediaId"] = old["mediaId"].clone();
            }
        }
    }
    unit["assetRefs"] = json!(refs);
    unit["importDiagnostics"] = json!(diagnostics);
    store.save_unit(unit)
}
pub fn compile_prompt(unit: &Value, assets: &[Value]) -> Result<String> {
    let mut checked = unit.clone();
    let diagnostics = lingmo_parser::rebind_unit(&mut checked, assets);
    if diagnostics.iter().any(|d| d["blocking"] == true) {
        return Err("正文中有无效或歧义资产标签，请先修正再准备制作".into());
    }
    if checked["mentions"] != unit["mentions"] {
        return Err("正文标签与保存的真实资产绑定不同，请先保存单元".into());
    }
    let mut prompt = s(unit, "prompt").to_owned();
    for mention in array(unit, "mentions").iter().rev() {
        let id = s(mention, "assetId");
        if id.is_empty() {
            return Err(format!("标签尚未绑定真实资产：{}", s(mention, "token")));
        }
        let start = mention["start"].as_u64().ok_or("标签位置缺失")? as usize;
        let end = mention["end"].as_u64().ok_or("标签位置缺失")? as usize;
        if prompt.get(start..end) != Some(s(mention, "token")) {
            return Err("标签位置与正文不符".into());
        }
        prompt.replace_range(
            start..end,
            &format!("{{{{asset:{id}:{}}}}}", s(mention, "variantId")),
        );
    }
    Ok(prompt)
}

fn import_images(store: &Store, request: &Value, parsed: &Value) -> Result<Value> {
    let mut report = json!({"imported":0,"missing":0,"placeholders":0,"failures":[]});
    if s(request, "imageZipPath").is_empty() && s(request, "imageDirectory").is_empty() {
        return Ok(report);
    }
    if parsed["catalogMatched"] != true {
        return Err("图片绑定要求同版资产目录；文本已保留，未猜测图片对应关系".into());
    }
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let directory = if !s(request, "imageZipPath").is_empty() {
        let mut zip = zip::ZipArchive::new(
            fs::File::open(s(request, "imageZipPath")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let rel = entry.enclosed_name().ok_or("图片包包含越界路径")?;
            if rel.components().any(|c| !matches!(c, Component::Normal(_)))
                || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
            {
                return Err("图片包包含不安全路径".into());
            }
            if entry.is_dir() {
                continue;
            }
            total = total.checked_add(entry.size()).ok_or("图片包过大")?;
            if total > 4 * 1024 * 1024 * 1024 {
                return Err("图片包超过4GiB上限".into());
            }
            let dst = temp.path().join(rel);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?
            }
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dst)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        }
        temp.path().to_path_buf()
    } else {
        Path::new(s(request, "imageDirectory"))
            .canonicalize()
            .map_err(|e| e.to_string())?
    };
    for asset in array(parsed, "assets") {
        let filename = s(&asset, "filename");
        if filename.is_empty() {
            continue;
        }
        let relative = Path::new(filename);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("目录图片文件名包含越界路径".into());
        }
        let path = directory.join(relative);
        if !path.is_file() {
            report["missing"] = json!(report["missing"].as_u64().unwrap_or(0) + 1);
            continue;
        }
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(directory.canonicalize().map_err(|e| e.to_string())?) {
            return Err("图片路径越界".into());
        }
        let result = (|| -> Result<()> {
            let mut media = store.import_media(
                &canonical,
                "image",
                Some(s(&asset, "id")),
                Some(s(&asset, "variantId")),
            )?;
            // White placeholder bytes remain non-production even if the package
            // label is absent; never auto-approve a blank image as finished art.
            let placeholder = match media["placeholderSource"]
                .as_str()
                .filter(|v| *v == "image-pixels/v1")
                .and(media["placeholder"].as_bool())
            {
                Some(value) => value,
                None => image::open(&canonical)
                    .map_err(|e| e.to_string())?
                    .to_rgba8()
                    .pixels()
                    .all(|p| p[3] == 0 || p.0[..3].iter().all(|v| *v >= 250)),
            };
            if placeholder {
                media["placeholder"] = json!(true);
                store
                    .conn()?
                    .execute(
                        "UPDATE media SET data=?1 WHERE id=?2",
                        params![media.to_string(), s(&media, "id")],
                    )
                    .map_err(|e| e.to_string())?;
                report["placeholders"] = json!(report["placeholders"].as_u64().unwrap_or(0) + 1);
            }
            // Import candidates only. Existing adoption or edited asset stays intact.
            report["imported"] = json!(report["imported"].as_u64().unwrap_or(0) + 1);
            Ok(())
        })();
        if let Err(message) = result {
            report["failures"]
                .as_array_mut()
                .unwrap()
                .push(json!({"filename":filename,"message":message}));
        }
    }
    Ok(report)
}

#[cfg(test)]
#[path = "lingmo_import_tests.rs"]
mod tests;
