//! Portable, exclusive-create delivery package. Nothing here invokes a model,
//! image provider or video provider. A manifest is written only after read-back.
use super::*;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};

fn bytes_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn write_file(
    root: &Path,
    relative: &str,
    bytes: &[u8],
    files: &mut Vec<Value>,
) -> Result<(), String> {
    let target = root.join(relative);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| format!("SERIES_EXPORT_WRITE:{relative}:{e}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    let saved = fs::read(&target).map_err(|e| e.to_string())?;
    if saved != bytes {
        return Err(format!("SERIES_EXPORT_READBACK:{relative}"));
    }
    files.push(json!({"path":relative,"sha256":bytes_hash(bytes),"bytes":bytes.len()}));
    Ok(())
}
fn write_json(
    root: &Path,
    relative: &str,
    value: &Value,
    files: &mut Vec<Value>,
) -> Result<(), String> {
    write_file(
        root,
        relative,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
        files,
    )
}
fn table(assets: &[Value], used: &HashSet<String>) -> String {
    let mut text=String::from("# 当前资产总库\n\n此表由同版当前总库生成，仅供阅读；身份和设计修改须返回桌面端。\n\n| ID | 正式名 | 类别 | 设计版本 | 本次图片 | 精确文件名 |\n|---|---|---|---:|---|---|\n");
    let clean = |v: &Value| {
        v.as_str()
            .unwrap_or("")
            .replace('|', "\\|")
            .replace(['\r', '\n'], " ")
    };
    for a in assets {
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            clean(&a["id"]),
            clean(&a["name"]),
            clean(&a["kind"]),
            a["designRevision"].as_u64().unwrap_or(0),
            if used.contains(s(a, "id")) {
                "已引用，白图仅占位"
            } else if export::is_voice_only(a) {
                "纯声音，不生图"
            } else {
                "未列入本次图片范围"
            },
            clean(&a["filename"])
        ));
    }
    text
}
pub(super) fn write(
    input: &Value,
    deliveries: &[Value],
    directory: &Path,
) -> Result<Value, String> {
    let frozen: Vec<_> = deliveries.iter().map(|_| input).collect();
    write_contexts(input, deliveries, &frozen, directory)
}

pub(super) fn write_with_inputs(
    input: &Value,
    deliveries: &[Value],
    frozen_inputs: &[Value],
    directory: &Path,
) -> Result<Value, String> {
    let frozen: Vec<_> = frozen_inputs.iter().collect();
    write_contexts(input, deliveries, &frozen, directory)
}

/// Transport versions belong to each original workflow. Shared creative
/// authority must agree, but an older episode is never re-signed or validated
/// as though it had used a later request representation.
fn validate_common_context(input: &Value, frozen: &Value) -> Result<(), String> {
    validate_profile(frozen)?;
    if input["source"] != frozen["source"] {
        return Err("SERIES_EXPORT_FROZEN_SOURCE_MISMATCH".into());
    }
    if input["productionScope"] != frozen["productionScope"]
        || input["seriesRulePackSha256"] != frozen["seriesRulePackSha256"]
        || input["seriesSkillVersion"] != frozen["seriesSkillVersion"]
    {
        return Err("SERIES_EXPORT_FROZEN_SCOPE_OR_RULES_MISMATCH".into());
    }
    if super::direction(input) != super::direction(frozen)
        || fantasy_mode(input)? != fantasy_mode(frozen)?
        || limit(input)? != limit(frozen)?
    {
        return Err("SERIES_EXPORT_FROZEN_DIRECTION_MISMATCH".into());
    }
    Ok(())
}

fn write_contexts(
    input: &Value,
    deliveries: &[Value],
    frozen_inputs: &[&Value],
    directory: &Path,
) -> Result<Value, String> {
    validate_profile(input)?;
    let source = source(input)?;
    let selected = scope(input, &source)?;
    if deliveries.is_empty() {
        return Err("SERIES_EXPORT_NO_ADOPTED_EPISODES".into());
    }
    if frozen_inputs.len() != deliveries.len() {
        return Err("SERIES_EXPORT_FROZEN_INPUT_COUNT".into());
    }
    let mut seen = HashSet::new();
    let mut previous = None;
    let mut all_units = Vec::new();
    let mut image_assets = Vec::<Value>::new();
    let mut unit_index = Vec::new();
    let mut initial = input["assets"]
        .as_array()
        .ok_or("SERIES_EXPORT_REGISTRY_MISSING")?
        .clone();
    allocate_asset_identities(&mut initial)?;
    let mut current = initial.clone();
    let mut snapshots = Vec::new();
    let mut markdown = String::from("**【标准化剧本】**\n");
    for (delivery, frozen) in deliveries.iter().zip(frozen_inputs) {
        validate_common_context(input, frozen)?;
        validate_delivery(frozen, delivery)?;
        let id = s(delivery, "episodeId");
        if !selected.iter().any(|selected| selected == id) {
            return Err("SERIES_EXPORT_EPISODE_OUTSIDE_SELECTION".into());
        }
        let position = source
            .episodes
            .iter()
            .position(|e| e.id == id)
            .ok_or("SERIES_EXPORT_EPISODE_UNKNOWN")?;
        if !seen.insert(id.to_owned()) || previous.is_some_and(|p| position <= p) {
            return Err("SERIES_EXPORT_EPISODE_ORDER".into());
        }
        previous = Some(position);
        let body = s(delivery, "markdown")
            .strip_prefix("**【标准化剧本】**\n")
            .ok_or("SERIES_EXPORT_STANDARD_ZONE")?;
        markdown.push_str(body);
        if !body.ends_with('\n') {
            markdown.push('\n');
        }
        // The frozen starting registry predates any assets supplied during this
        // episode. Only the already validated adoption proof may extend it;
        // arbitrary bundle cards must never create identities during export.
        // Keep initial.json unchanged so the portable history remains truthful.
        if let Some(added) =
            delivery_supply_proof(delivery)["addedAssets"].as_array()
        {
            for asset in added {
                if let Some(existing) = current.iter().find(|old| old["id"] == asset["id"]) {
                    for key in [
                        "variantId",
                        "name",
                        "kind",
                        "importName",
                        "filename",
                        "placeholderOrdinal",
                    ] {
                        if existing[key] != asset[key] {
                            return Err(format!(
                                "SERIES_EXPORT_IDENTITY_CONFLICT:{}:{key}",
                                s(asset, "id")
                            ));
                        }
                    }
                } else {
                    current.push(asset.clone());
                }
            }
            // Check names, filenames and reserved ordinals against the entire
            // registry, including assets outside this episode's image scope.
            allocate_asset_identities(&mut current)?;
        }
        for a in delivery["bundle"]["assets"]
            .as_array()
            .ok_or("SERIES_EXPORT_ASSETS_MISSING")?
        {
            let target = current
                .iter_mut()
                .find(|old| old["id"] == a["id"] && old["variantId"] == a["variantId"])
                .ok_or("SERIES_EXPORT_REGISTRY_IDENTITY")?;
            for key in [
                "name",
                "kind",
                "importName",
                "filename",
                "placeholderOrdinal",
            ] {
                if target[key] != a[key] {
                    return Err(format!(
                        "SERIES_EXPORT_IDENTITY_CONFLICT:{}:{key}",
                        s(a, "id")
                    ));
                }
            }
            let old = target["designRevision"].as_u64().unwrap_or(0);
            let revision = a["designRevision"].as_u64().unwrap_or(0);
            if revision < old {
                return Err(format!("SERIES_EXPORT_DESIGN_REVERSED:{}", s(a, "id")));
            }
            if revision == old
                && !target["adoptedDesign"].is_null()
                && target["adoptedDesign"]["full"] != a["adoptedDesign"]["full"]
            {
                return Err(format!(
                    "SERIES_EXPORT_DESIGN_VERSION_COLLISION:{}",
                    s(a, "id")
                ));
            }
            if !a["adoptedDesign"].is_null() {
                target["adoptedDesign"] = a["adoptedDesign"].clone();
                target["designRevision"] = a["designRevision"].clone();
                if let Some(status) = a.get("designStatus") {
                    target["designStatus"] = status.clone();
                }
            }
        }
        snapshots.push(json!({"episodeId":id,"assets":current}));
        for a in delivery["assetIndex"]["assets"]
            .as_array()
            .ok_or("SERIES_EXPORT_INDEX_ASSETS")?
        {
            if let Some(old) = image_assets.iter().find(|old| old["id"] == a["id"]) {
                if old != a {
                    return Err(format!("SERIES_EXPORT_MIXED_ASSET_VERSIONS:{}", s(a, "id")));
                }
            } else {
                image_assets.push(a.clone());
            }
        }
        unit_index.extend(
            delivery["assetIndex"]["units"]
                .as_array()
                .ok_or("SERIES_EXPORT_INDEX_UNITS")?
                .iter()
                .cloned(),
        );
        all_units.extend(
            delivery["bundle"]["draft"]["units"]
                .as_array()
                .ok_or("SERIES_EXPORT_DRAFT_UNITS")?
                .iter()
                .cloned(),
        );
    }
    let used: HashSet<_> = image_assets.iter().map(|a| s(a, "id").to_owned()).collect();
    // Recheck union-level filenames/ordinals, not just each episode in isolation.
    let current_index = export::index_assets(&current, &used)?;
    for indexed in &image_assets {
        if current_index.iter().find(|a| a["id"] == indexed["id"]) != Some(indexed) {
            return Err("SERIES_EXPORT_CURRENT_REGISTRY_CONFLICT".into());
        }
    }
    let index = json!({"schemaVersion":"lingmo-v7-assets-v1","markdownSha256":digest(&markdown),"sourceSha256":source.sha256,"productionScope":input["productionScope"],"assets":image_assets,"units":unit_index});
    export::verify(&markdown, &index, &json!({"units":all_units}))?;
    let complete = selected.len() == seen.len() && selected.iter().all(|e| seen.contains(e));
    let initial_registry = json!({"version":"desktop-series-asset-registry/v1","sourceSha256":source.sha256,"workflowProfile":PROFILE,"assets":initial});
    let current_registry = json!({"version":"desktop-series-asset-registry/v1","sourceSha256":source.sha256,"workflowProfile":PROFILE,"productionScope":input["productionScope"],"assets":current});
    // This is a versioned desktop adapter, not a forged upstream season queue.
    // Its precise consumers and hashes are taken from the adopted bundles.
    let queue_items:Vec<_>=index["assets"].as_array().unwrap().iter().map(|a|{
        let uses:Vec<_>=index["units"].as_array().unwrap().iter().filter(|u|u["imageTokens"].as_array().is_some_and(|tokens|tokens.contains(&a["token"]))).map(|u|json!({"episode":u["episode"],"sceneId":u["sceneId"],"unitId":u["unitId"],"purpose":"adopted-image-binding"})).collect();
        json!({"assetId":a["id"],"variantId":a["variantId"],"assetRevision":a["assetRevision"],"filename":a["filename"],"importName":a["importName"],"design":a["adoptedDesign"],"designHash":hash(&a["adoptedDesign"]),"uses":uses,"imageState":"not-generated"})
    }).collect();
    let mut queue = json!({"version":"desktop-series-image-queue/v1","sourceSha256":source.sha256,"productionScope":input["productionScope"],"registrySha256":hash(&current_registry),"markdownSha256":index["markdownSha256"],"assetIndexSha256":hash(&index),"items":queue_items});
    queue["queueHash"] = json!(hash(&queue));
    if !directory.is_absolute() {
        return Err("SERIES_EXPORT_ABSOLUTE_DIRECTORY_REQUIRED".into());
    }
    let parent = directory
        .parent()
        .filter(|p| p.is_dir())
        .ok_or("SERIES_EXPORT_PARENT_MISSING")?;
    if directory.exists() {
        return Err("SERIES_EXPORT_DIRECTORY_EXISTS_NOT_OVERWRITTEN".into());
    }
    let canonical_parent = parent.canonicalize().map_err(|e| e.to_string())?;
    let name = directory
        .file_name()
        .ok_or("SERIES_EXPORT_DIRECTORY_NAME")?;
    let root = canonical_parent.join(name);
    fs::create_dir(&root).map_err(|e| format!("SERIES_EXPORT_EXCLUSIVE_CREATE:{e}"))?;
    for relative in ["资料", "资料/episodes", "资料/registry", "白图"] {
        fs::create_dir(root.join(relative)).map_err(|e| e.to_string())?;
    }
    let mut files = Vec::new();
    write_file(&root, "全剧_灵墨v7.md", markdown.as_bytes(), &mut files)?;
    write_json(&root, "全剧_灵墨v7.md.assets.json", &index, &mut files)?;
    write_file(&root, "资料/source.txt", source.text.as_bytes(), &mut files)?;
    write_json(&root, "资料/source-lock.json", &json!(source), &mut files)?;
    write_json(
        &root,
        "资料/registry/initial.json",
        &initial_registry,
        &mut files,
    )?;
    write_json(
        &root,
        "资料/registry/current.json",
        &current_registry,
        &mut files,
    )?;
    write_file(
        &root,
        "资料/资产总表.md",
        table(&current, &used).as_bytes(),
        &mut files,
    )?;
    write_json(&root, "资料/image-queue.json", &queue, &mut files)?;
    let mut direction = Map::new();
    for key in [
        "projectVisualStyle",
        "visualStyle",
        "productionDirection",
        "productionDirectionConfirmed",
        "confirmedDirection",
        "ratio",
        "aspectRatio",
        "fantasyMode",
        "maxUnitSeconds",
        "systemPresentation",
        "systemPresentations",
    ] {
        if let Some(value) = input["settings"].get(key) {
            direction.insert(key.into(), value.clone());
        }
    }
    write_json(
        &root,
        "资料/direction.json",
        &json!({"version":"desktop-series-direction/v1","sourceSha256":source.sha256,"settings":direction,"adoptedDirection":super::direction(input),"productionScope":input["productionScope"]}),
        &mut files,
    )?;
    write_json(
        &root,
        "资料/source-facts.json",
        &json!({"version":"desktop-series-source-facts/v1","sourceSha256":source.sha256,"facts":input["foundation"]["facts"]}),
        &mut files,
    )?;
    for (position, delivery) in deliveries.iter().enumerate() {
        write_json(
            &root,
            &format!("资料/episodes/{:04}.json", position + 1),
            delivery,
            &mut files,
        )?;
        write_json(
            &root,
            &format!("资料/registry/after-{:04}.json", position + 1),
            &snapshots[position],
            &mut files,
        )?;
    }
    let mut image_hashes = HashSet::new();
    let mut image_md5s = HashSet::new();
    let mut placeholder_proofs = Vec::new();
    let mut image_files = Vec::new();
    for a in index["assets"].as_array().unwrap() {
        let side = 300
            + a["placeholderOrdinal"]
                .as_u64()
                .ok_or("SERIES_PLACEHOLDER_ORDINAL_NOT_ALLOCATED")? as u32;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            side,
            side,
            image::Rgb([255, 255, 255]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
        let bytes = bytes.into_inner();
        let md5 = format!("{:x}", md5::Md5::digest(&bytes));
        let decoded = image::load_from_memory(&bytes)
            .map_err(|e| e.to_string())?
            .to_rgb8();
        if decoded.dimensions() != (side, side)
            || !decoded.pixels().all(|p| p.0 == [255, 255, 255])
            || !image_hashes.insert(bytes_hash(&bytes))
            || !image_md5s.insert(md5.clone())
        {
            return Err("SERIES_PLACEHOLDER_PIXEL_READBACK".into());
        }
        let filename = s(a, "filename");
        placeholder_proofs.push(json!({"assetId":a["id"],"filename":filename,"width":side,"height":side,"md5":md5,"sha256":bytes_hash(&bytes),"pixels":"opaque-white","productionImage":false}));
        let relative = format!("白图/{filename}");
        write_file(&root, &relative, &bytes, &mut files)?;
        image_files.push((filename.to_owned(), relative));
    }
    write_json(
        &root,
        "资料/placeholder-verification.json",
        &json!({"version":"desktop-white-placeholder-check/v1","scope":"exact-adopted-image-bindings","files":placeholder_proofs}),
        &mut files,
    )?;
    let mut zip_bytes = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut zip_bytes);
        for (name, relative) in &image_files {
            zip.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .map_err(|e| e.to_string())?;
            zip.write_all(&fs::read(root.join(relative)).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    write_file(&root, "白图.zip", zip_bytes.get_ref(), &mut files)?;
    let readme=format!("本包状态：本地上传包准备好（{}）。\n分镜 {} 集，图片占位 {} 张。白图不是正式图片，未提交图片或视频生成。\nMD 与 .assets.json 必须配套；纯 PNG 位于白图/，总库、完整设计、源锁及采用回执位于资料/。\n源 SHA-256：{}\n本地验证不表示灵墨平台已导入，也不表示图像或视频已验收。\n",if complete{"所选范围完整"}else{"逐集预览，范围尚未全部完成"},deliveries.len(),image_files.len(),source.sha256);
    write_file(&root, "说明.txt", readme.as_bytes(), &mut files)?;
    // Read the independently written package, never just the in-memory strings.
    let saved_md = fs::read_to_string(root.join("全剧_灵墨v7.md")).map_err(|e| e.to_string())?;
    let saved_index: Value = serde_json::from_slice(
        &fs::read(root.join("全剧_灵墨v7.md.assets.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    export::verify(&saved_md, &saved_index, &json!({"units":all_units}))?;
    let manifest = json!({"version":"desktop-series-delivery-package/v1","workflowProfile":PROFILE,"status":"LOCAL_PACKAGE_READY","scopeComplete":complete,"platformImportVerified":false,"mediaGenerated":false,"sourceSha256":source.sha256,"rulePackSha256":rules::pack_hash()?,"productionScope":input["productionScope"],"episodeIds":deliveries.iter().map(|d|d["episodeId"].clone()).collect::<Vec<_>>(),"markdown":"全剧_灵墨v7.md","assetIndex":"全剧_灵墨v7.md.assets.json","currentRegistry":"资料/registry/current.json","imageQueue":"资料/image-queue.json","placeholderDirectory":"白图","files":files});
    write_json(&root, "manifest.json", &manifest, &mut Vec::new())?;
    Ok(
        json!({"path":root.to_string_lossy(),"count":deliveries.len(),"manifestPath":root.join("manifest.json").to_string_lossy(),"imageCount":image_files.len(),"unitCount":all_units.len(),"scopeComplete":complete,"status":"LOCAL_PACKAGE_READY"}),
    )
}

#[cfg(test)]
#[path = "series_export_context_tests.rs"]
mod context_tests;
