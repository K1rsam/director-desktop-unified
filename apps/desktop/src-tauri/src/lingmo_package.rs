//! Read-only, versioned external delivery evidence. A verified package is not
//! a fresh creative approval, an original project source or a generated image.
use crate::{
    pipeline::{digest, series, LockedSource, TextRequest},
    store::{parse, s, Result, Store},
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const PREFIX: &str = "lingmo.package:";
const MAX_FILE: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_TOTAL: u64 = 512 * 1024 * 1024;
fn hash(v: &Value) -> String {
    digest(&v.to_string())
}
fn bytes_hash(v: &[u8]) -> String {
    format!("{:x}", Sha256::digest(v))
}
fn rows<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v[key]
        .as_array()
        .ok_or_else(|| format!("完整接续包缺少数组：{key}"))
}
fn check(ok: bool, reason: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("完整接续包核验失败：{reason}"))
    }
}

fn no_link(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("无法读取接续包路径 {}：{e}", path.display()))?;
    check(!metadata.file_type().is_symlink(), "不接受符号链接")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        check(
            metadata.file_attributes() & 0x400 == 0,
            "不接受 junction/reparse 路径",
        )?;
    }
    Ok(())
}
pub(crate) fn root_path(path: &Path) -> Result<PathBuf> {
    check(path.is_absolute(), "包目录必须为绝对路径")?;
    let mut cursor = PathBuf::new();
    for part in path.components() {
        check(
            !matches!(part, Component::ParentDir | Component::CurDir),
            "包目录包含相对跳转",
        )?;
        cursor.push(part.as_os_str());
        if !matches!(part, Component::Prefix(_) | Component::RootDir) {
            no_link(&cursor)?;
        }
    }
    path.canonicalize().map_err(|e| e.to_string())
}
pub(crate) fn relative(root: &Path, name: &str) -> Result<PathBuf> {
    check(
        !name.is_empty() && !name.contains(['\\', ':']) && !name.ends_with('/'),
        "文件名必须是包内标准相对路径",
    )?;
    let path = Path::new(name);
    check(
        path.components().all(|c| matches!(c, Component::Normal(_)))
            && name
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "文件路径越出包目录",
    )?;
    let mut target = root.to_owned();
    for part in path.components() {
        let segment = part.as_os_str().to_str().ok_or("接续包文件名不是 UTF-8")?;
        check(!segment.ends_with([' ', '.']), "文件路径含不稳定尾缀")?;
        target.push(segment);
        no_link(&target)?;
    }
    check(target.is_file(), "清单项不是普通文件")?;
    check(
        target
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(root),
        "文件路径越出包目录",
    )?;
    Ok(target)
}
pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    f.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    check(bytes.len() as u64 <= MAX_FILE, "单文件超过 64 MiB")?;
    Ok(bytes)
}
pub(crate) struct VerifiedPackage {
    pub value: Value,
    pub(crate) files: BTreeMap<String, Vec<u8>>,
    pub(crate) manifest_bytes: Vec<u8>,
    pub(crate) manifest_name: String,
}
impl VerifiedPackage {
    pub fn proof(&self) -> Value {
        self.value["proof"].clone()
    }
    fn json(&self, name: &str) -> Result<Value> {
        serde_json::from_slice(
            self.files
                .get(name)
                .ok_or_else(|| format!("接续包清单缺少 {name}"))?,
        )
        .map_err(|e| format!("接续包 JSON {name}：{e}"))
    }
    fn text(&self, name: &str) -> Result<String> {
        String::from_utf8(
            self.files
                .get(name)
                .ok_or_else(|| format!("接续包清单缺少 {name}"))?
                .clone(),
        )
        .map_err(|e| e.to_string())
    }
}

/// An absent manifest remains MD/catalog evidence. A present, invalid manifest
/// fails closed unless the caller explicitly chose the lower catalog-only mode.
pub(crate) fn discover(
    request: &Value,
    text: &str,
    catalog: &Value,
) -> Result<Option<VerifiedPackage>> {
    let mode = s(request, "handoffMode");
    check(
        matches!(mode, "" | "auto" | "complete-package" | "catalog-only"),
        "未知导入证据模式",
    )?;
    if mode == "catalog-only" {
        return Ok(None);
    }
    let md = Path::new(s(request, "path"));
    if request.get("text").is_some() || s(request, "path").is_empty() {
        check(mode != "complete-package", "完整包必须选择原 MD 文件")?;
        return Ok(None);
    }
    let root = md.parent().ok_or("MD 文件目录缺失")?;
    if !s(request,"module22Root").is_empty() && request.get("module22Mapping").is_none() {
        let package_root=Path::new(s(request,"module22Root"));
        let package=if package_root.join("module22-map.json").is_file(){crate::module22_package::load(package_root)?}else{
            let mapping=crate::module22_discover::mapping(package_root,md,catalog,s(request,"module22DirectionPath"))?;
            crate::module22_package::load_mapped(package_root,&mapping)?
        };
        check(package.value["markdownText"]==text&&(catalog.is_null()||package.value["catalog"]==*catalog),"选定 MD/目录与原生资料文件夹不同版")?;
        return Ok(Some(package));
    }
    if let Some(mapping) = request.get("module22Mapping") {
        let package_root = request["module22Root"].as_str().map(Path::new).unwrap_or(root);
        let package = crate::module22_package::load_mapped(package_root, mapping)?;
        check(package.value["markdownText"] == text && (catalog.is_null() || package.value["catalog"] == *catalog), "选定 MD/目录与原生接续映射不同版")?;
        return Ok(Some(package));
    }
    if root.join("module22-map.json").is_file() {
        let package = crate::module22_package::load(root)?;
        check(package.value["markdownText"] == text && (catalog.is_null() || package.value["catalog"] == *catalog), "选定 MD/目录与原生接续包不同版")?;
        return Ok(Some(package));
    }
    if !root.join("manifest.json").exists() {
        check(
            mode != "complete-package",
            "缺少 manifest.json；当前只能作为 MD/目录导入",
        )?;
        return Ok(None);
    }
    let package = load(root)?;
    let manifest = &package.value["manifest"];
    check(
        md.file_name().and_then(|v| v.to_str()) == Some(s(manifest, "markdown")),
        "选定 MD 不属于同根 manifest",
    )?;
    check(
        package.text(s(manifest, "markdown"))? == text && package.value["catalog"] == *catalog,
        "MD/目录与完整包不一致",
    )?;
    Ok(Some(package))
}

fn packet_value<'a>(packet: &'a Value, kind: &str) -> Result<&'a Value> {
    let bindings: Vec<_> = rows(packet, "bindings")?
        .iter()
        .filter(|b| b["kind"] == kind)
        .collect();
    check(bindings.len() == 1, "岗位包材料绑定缺失或重复")?;
    packet["blobs"]
        .get(s(bindings[0], "blob"))
        .and_then(|b| b.get("value"))
        .ok_or_else(|| "岗位包材料缺失".into())
}
fn verify_packet(packet: &Value) -> Result<()> {
    let mut clean = packet.clone();
    clean
        .as_object_mut()
        .ok_or("岗位包格式错误")?
        .remove("inputFingerprint");
    check(packet["inputFingerprint"] == hash(&clean), "岗位包指纹不符")?;
    let mut kinds = HashSet::new();
    for binding in rows(packet, "bindings")? {
        let blob = &packet["blobs"][s(binding, "blob")];
        check(
            kinds.insert(s(binding, "kind"))
                && blob["format"] == "json"
                && s(binding, "blob") == hash(blob),
            "岗位包材料摘要或绑定重复",
        )?;
    }
    Ok(())
}
fn verify_delivery(
    delivery: &Value,
    manifest: &Value,
    direction: &Value,
    source: &LockedSource,
) -> Result<()> {
    let bundle = &delivery["bundle"];
    let receipt = &delivery["adoptionReceipt"];
    series::validate_visual_baseline_handoff(&direction["settings"], delivery)?;
    check(
        bundle["workflowProfile"] == series::PROFILE
            && bundle["sourceSha256"] == manifest["sourceSha256"]
            && bundle["episodeId"] == delivery["episodeId"]
            && bundle["productionScope"] == manifest["productionScope"]
            && receipt["version"] == "desktop-series-adoption/v1"
            && receipt["sourceSha256"] == manifest["sourceSha256"]
            && receipt["bundleSha256"] == hash(bundle)
            && receipt["markdownSha256"] == digest(s(delivery, "markdown"))
            && delivery["markdownSha256"] == receipt["markdownSha256"]
            && receipt["assetIndexSha256"] == hash(&delivery["assetIndex"])
            && receipt["rulePackSha256"] == manifest["rulePackSha256"],
        "采用稿或回执摘要/版本不符",
    )?;
    series::export::verify(
        s(delivery, "markdown"),
        &delivery["assetIndex"],
        &bundle["draft"],
    )?;
    let mut identities = HashSet::new();
    let mut roles = HashSet::new();
    let mut reviewer = None;
    for r in rows(receipt, "roleReceipts")? {
        let req: TextRequest = serde_json::from_value(r["request"].clone())
            .map_err(|e| format!("岗位请求格式错误：{e}"))?;
        let packet = &req.input["packet"];
        check(
            r["requestSha256"] == digest(&serde_json::to_string(&req).map_err(|e| e.to_string())?)
                && r["rawSha256"] == hash(&r["raw"])
                && r["role"] == req.input["role"]
                && r["stage"] == req.stage
                && r["executionRoleId"] == req.input["executionRoleId"]
                && !s(r, "executionRoleId").is_empty()
                && identities.insert(s(r, "executionRoleId"))
                && req.input["freshContext"] == true
                && req.input["workflowProfile"] == series::PROFILE
                && req.input["sourceSha256"] == manifest["sourceSha256"]
                && req.input["episodeId"] == delivery["episodeId"]
                && req.input["seriesRulePackSha256"] == manifest["rulePackSha256"]
                && r["packetSha256"] == hash(packet)
                && packet["sourceSha256"] == manifest["sourceSha256"]
                && packet["productionScope"] == manifest["productionScope"]
                && packet["rulePackSha256"] == manifest["rulePackSha256"]
                && packet["episodeId"] == delivery["episodeId"],
            "岗位原请求/原回包关系不符",
        )?;
        verify_packet(packet)?;
        check(
            packet_value(packet, "direction")? == &direction["adoptedDirection"],
            "采用方向与岗位材料不同版",
        )?;
        if matches!(s(r, "role"), "author" | "finisher" | "reviewer" | "repair") {
            check(
                packet_value(packet, "assets")? == &bundle["assets"]
                    && packet_value(packet, "entry")? == &bundle["entry"],
                "岗位未收到同版完整设计/入口",
            )?;
        }
        roles.insert(s(r, "role"));
        if r["role"] == "reviewer" {
            reviewer = Some(r);
        }
    }
    // Supplement author chains are retained verbatim. This importer checks the
    // current final independent review, not a fabricated replay of the writer.
    check(
        roles.contains("finisher")
            && (roles.contains("author") || receipt["assetSupplement"].is_object()),
        "缺少主创/整编来源",
    )?;
    let reviewer = reviewer.ok_or("完整接续包缺少独审回执")?;
    let candidate = json!({"schemaVersion":"series-episode-candidate/v1","plan":bundle["plan"],"selection":bundle["selection"],"draft":bundle["draft"]});
    let review = &reviewer["raw"];
    check(
        review["schemaVersion"] == "series-independent-review/v1"
            && review["candidateSha256"] == hash(&candidate)
            && review["conclusion"] == "pass"
            && delivery["review"] == *review
            && receipt["review"]["status"] == "passed"
            && receipt["review"]["candidateSha256"] == hash(&candidate)
            && receipt["review"]["requestSha256"] == reviewer["requestSha256"]
            && receipt["review"]["resultSha256"] == reviewer["rawSha256"],
        "独审没有绑定最终采用稿",
    )?;
    for key in ["plan", "selection", "draft"] {
        check(
            packet_value(&reviewer["request"]["input"]["packet"], key)? == &candidate[key],
            "独审收到的稿件不同版",
        )?;
    }
    let unit_ids: Vec<_> = rows(&bundle["draft"], "units")?
        .iter()
        .map(|u| u["id"].clone())
        .collect();
    check(
        review["reviewedUnitIds"] == json!(unit_ids)
            && !rows(review, "issues")?
                .iter()
                .any(|i| i["severity"] == "must_fix"),
        "独审单元不完整或仍有必修项",
    )?;
    let source_rows: Vec<_> = source
        .rows
        .iter()
        .filter(|r| r.kind == "body" && r.episode_id.as_deref() == delivery["episodeId"].as_str())
        .map(|r| json!(r.id))
        .collect();
    check(
        review["sourceRowIds"] == json!(source_rows),
        "独审原文行覆盖不符",
    )?;
    Ok(())
}

pub(crate) fn load(directory: &Path) -> Result<VerifiedPackage> {
    if directory.join("module22-map.json").is_file() { return crate::module22_package::load(directory); }
    let root = root_path(directory)?;
    let manifest_bytes = read(&relative(&root, "manifest.json")?)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    check(
        manifest["version"] == "desktop-series-delivery-package/v1",
        "尚不支持此完整包 schema（原生模块22包不能冒充本软件包）",
    )?;
    check(
        manifest["workflowProfile"] == series::PROFILE
            && manifest["status"] == "LOCAL_PACKAGE_READY"
            && manifest["mediaGenerated"] == false,
        "包工作流/状态不符",
    )?;
    let listed = rows(&manifest, "files")?;
    check(
        !listed.is_empty() && listed.len() <= 20000,
        "文件清单为空或超过上限",
    )?;
    let mut files = BTreeMap::new();
    let mut names = HashSet::new();
    let mut total = 0u64;
    for file in listed {
        let name = s(file, "path");
        check(
            name != "manifest.json" && names.insert(name.to_lowercase()),
            "清单路径重复",
        )?;
        let bytes = read(&relative(&root, name)?)?;
        total += bytes.len() as u64;
        check(total <= MAX_TOTAL, "包体超过 512 MiB")?;
        check(
            file["sha256"] == bytes_hash(&bytes)
                && file["bytes"].as_u64() == Some(bytes.len() as u64),
            &format!("文件摘要或长度不符：{name}"),
        )?;
        files.insert(name.to_owned(), bytes);
    }
    let mut package = VerifiedPackage {
        value: json!({"manifest":manifest}),
        files,
        manifest_bytes,
        manifest_name: "manifest.json".into(),
    };
    let manifest = package.value["manifest"].clone();
    let markdown = package.text(s(&manifest, "markdown"))?;
    let catalog = package.json(s(&manifest, "assetIndex"))?;
    let registry = package.json(s(&manifest, "currentRegistry"))?;
    let initial = package.json("资料/registry/initial.json")?;
    let queue = package.json(s(&manifest, "imageQueue"))?;
    let direction = package.json("资料/direction.json")?;
    let source_text = package.text("资料/source.txt")?;
    let source_lock = package.json("资料/source-lock.json")?;
    let source: LockedSource =
        serde_json::from_value(source_lock.clone()).map_err(|e| format!("源锁格式错误：{e}"))?;
    crate::pipeline::validate_source(&source)?;
    check(
        source.text == source_text
            && source.sha256 == s(&manifest, "sourceSha256")
            && !source.diagnostics.iter().any(|d| d.blocking),
        "源文/源锁不一致或范围未解锁",
    )?;
    let scope = &manifest["productionScope"];
    check(
        scope["sourceSha256"] == source.sha256 && matches!(s(scope, "status"), "locked" | "LOCKED"),
        "制作范围未锁定",
    )?;
    let scope_ids = rows(scope, "episodeIds")?;
    let sorted: Vec<_> = source
        .episodes
        .iter()
        .map(|e| json!(e.id))
        .filter(|id| scope_ids.contains(id))
        .collect();
    check(
        !scope_ids.is_empty() && sorted == *scope_ids,
        "范围集数重复、未知或顺序变化",
    )?;
    let episode_ids = rows(&manifest, "episodeIds")?;
    check(
        !episode_ids.is_empty()
            && scope_ids
                .iter()
                .filter(|id| episode_ids.contains(id))
                .cloned()
                .collect::<Vec<_>>()
                == *episode_ids,
        "交付集数越界或顺序变化",
    )?;
    for (v, version) in [
        (&registry, "desktop-series-asset-registry/v1"),
        (&initial, "desktop-series-asset-registry/v1"),
        (&direction, "desktop-series-direction/v1"),
        (&queue, "desktop-series-image-queue/v1"),
    ] {
        check(
            v["version"] == version && v["sourceSha256"] == source.sha256,
            "资料 schema/源版本不符",
        )?;
    }
    check(
        registry["productionScope"] == *scope
            && direction["productionScope"] == *scope
            && queue["productionScope"] == *scope
            && catalog["productionScope"] == *scope,
        "资料制作范围不同版",
    )?;
    check(
        catalog["schemaVersion"] == "lingmo-v7-assets-v1"
            && catalog["markdownSha256"] == digest(&markdown)
            && catalog["sourceSha256"] == source.sha256,
        "同版资产目录不符",
    )?;
    let mut clean = queue.clone();
    clean.as_object_mut().unwrap().remove("queueHash");
    check(
        queue["queueHash"] == hash(&clean)
            && queue["registrySha256"] == hash(&registry)
            && queue["assetIndexSha256"] == hash(&catalog)
            && queue["markdownSha256"] == catalog["markdownSha256"],
        "图片队列摘要不符",
    )?;
    let indexed = rows(&catalog, "assets")?;
    let used: HashSet<_> = indexed.iter().map(|a| s(a, "id").to_owned()).collect();
    check(used.len() == indexed.len(), "图片目录身份重复")?;
    let registry_assets = rows(&registry, "assets")?;
    let reg_ids: HashSet<_> = registry_assets.iter().map(|a| s(a, "id")).collect();
    check(
        reg_ids.len() == registry_assets.len() && !reg_ids.contains(""),
        "总库稳定身份重复或缺失",
    )?;
    let expected_index = series::export::index_assets(registry_assets, &used)?;
    check(
        expected_index.len() == indexed.len() && indexed.iter().all(|a| expected_index.contains(a)),
        "目录身份、名称、设计或版本与总库不符",
    )?;
    for original in rows(&initial, "assets")? {
        let current = registry_assets
            .iter()
            .find(|a| a["id"] == original["id"])
            .ok_or("初始总库身份在最终库中丢失")?;
        for key in [
            "variantId",
            "name",
            "kind",
            "importName",
            "filename",
            "placeholderOrdinal",
        ] {
            check(current[key] == original[key], "初始/最终总库稳定身份变化")?;
        }
        check(
            current["designRevision"].as_u64().unwrap_or(0)
                >= original["designRevision"].as_u64().unwrap_or(0),
            "总库设计版本倒退",
        )?;
    }
    let facts = package.json("资料/source-facts.json")?;
    check(
        facts["version"] == "desktop-series-source-facts/v1"
            && facts["sourceSha256"] == source.sha256,
        "源事实资料不同版",
    )?;
    for required in ["资料/资产总表.md", "说明.txt", "白图.zip"] {
        check(
            package.files.contains_key(required),
            "完整包缺少交付配套文件",
        )?;
    }
    let placeholders = package.json("资料/placeholder-verification.json")?;
    check(
        placeholders["version"] == "desktop-white-placeholder-check/v1"
            && rows(&placeholders, "files")?.len() == indexed.len(),
        "占位清单范围不符",
    )?;
    for asset in indexed {
        let relative = format!("白图/{}", s(asset, "filename"));
        let bytes = package
            .files
            .get(&relative)
            .ok_or("完整包缺少精确文件名的占位图")?;
        let matching: Vec<_> = rows(&placeholders, "files")?
            .iter()
            .filter(|p| p["assetId"] == asset["id"] && p["filename"] == asset["filename"])
            .collect();
        check(
            matching.len() == 1
                && matching[0]["sha256"] == bytes_hash(bytes)
                && matching[0]["productionImage"] == false,
            "白图占位来源不符；不能作为正式图采用",
        )?;
    }
    let expected_items:Vec<_>=indexed.iter().map(|a|{
        let uses:Vec<_>=catalog["units"].as_array().into_iter().flatten().filter(|u|u["imageTokens"].as_array().is_some_and(|tokens|tokens.contains(&a["token"]))).map(|u|json!({"episode":u["episode"],"sceneId":u["sceneId"],"unitId":u["unitId"],"purpose":"adopted-image-binding"})).collect();
        json!({"assetId":a["id"],"variantId":a["variantId"],"assetRevision":a["assetRevision"],"filename":a["filename"],"importName":a["importName"],"design":a["adoptedDesign"],"designHash":hash(&a["adoptedDesign"]),"uses":uses,"imageState":"not-generated"})
    }).collect();
    check(
        queue["items"] == json!(expected_items),
        "图片队列范围/用途/精确文件名不同版；未扩大到全库",
    )?;
    let mut episodes = Vec::new();
    let mut all_units = Vec::new();
    let mut union_assets = Vec::new();
    let mut unit_index = Vec::new();
    let mut combined = String::from("**【标准化剧本】**\n");
    for (position, id) in episode_ids.iter().enumerate() {
        let episode = package.json(&format!("资料/episodes/{:04}.json", position + 1))?;
        check(episode["episodeId"] == *id, "逐集资料身份或顺序不符")?;
        verify_delivery(&episode, &manifest, &direction, &source)?;
        let snapshot = package.json(&format!("资料/registry/after-{:04}.json", position + 1))?;
        check(
            snapshot["episodeId"] == *id && snapshot["assets"].is_array(),
            "逐集总库快照不符",
        )?;
        if position + 1 == episode_ids.len() {
            check(
                snapshot["assets"] == registry["assets"],
                "最终总库与最后采用快照不符",
            )?;
        }
        let body = s(&episode, "markdown")
            .strip_prefix("**【标准化剧本】**\n")
            .ok_or("逐集 MD 标准区缺失")?;
        combined.push_str(body);
        if !body.ends_with('\n') {
            combined.push('\n');
        }
        for a in rows(&episode["assetIndex"], "assets")? {
            if let Some(old) = union_assets
                .iter()
                .find(|old: &&Value| old["id"] == a["id"])
            {
                check(*old == *a, "跨集图片资产版本冲突")?;
            } else {
                union_assets.push(a.clone());
            }
        }
        unit_index.extend(rows(&episode["assetIndex"], "units")?.iter().cloned());
        all_units.extend(rows(&episode["bundle"]["draft"], "units")?.iter().cloned());
        episodes.push(episode);
    }
    check(
        markdown == combined
            && catalog["assets"] == json!(union_assets)
            && catalog["units"] == json!(unit_index),
        "总 MD/目录不是逐集采用稿按顺序合并",
    )?;
    series::export::verify(&markdown, &catalog, &json!({"units":all_units}))?;
    let proof = json!({"level":"desktop-complete-package","manifestSha256":bytes_hash(&package.manifest_bytes),
        "packageFingerprint":bytes_hash(&package.manifest_bytes),"sourceSha256":source.sha256,
        "registrySha256":hash(&registry),"queueHash":queue["queueHash"],"productionScopeHash":hash(scope),"directionSha256":hash(&direction),
        "explicitImageCount":indexed.len(),"registryAssetCount":registry_assets.len(),"upstreamModule22QueueVerified":false,
        "validation":"file hashes, source lock, exact image scope, versioned identities, final adopted draft and independent review bindings",
        "evidenceBoundary":"machine package verification; not a new semantic or visual review; placeholders are not production images"});
    package.value = json!({"version":"desktop-complete-handoff/v1","proof":proof,"manifest":manifest,"catalog":catalog,"registry":registry,"initialRegistry":initial,"direction":direction,"queue":queue,"sourceLock":source_lock,"sourceText":source_text,"episodes":episodes});
    Ok(package)
}

pub(crate) fn prepare_storage(store: &Store, package: &VerifiedPackage) -> Result<Value> {
    let sha = s(&package.value["proof"], "manifestSha256");
    let relative_root = format!("imports/complete-packages/{sha}");
    let root = store.root.join(&relative_root);
    if root.exists() {
        let saved = load(&root)?;
        check(saved.value == package.value, "已保存完整包内容冲突")?;
    } else {
        let parent = root.parent().unwrap();
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        root_path(parent)?;
        let staging = parent.join(format!(".pending-{sha}-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        // A partial copy never occupies the final content-addressed location.
        // Failures retain evidence; a retry uses a new exclusive staging path.
        let manifest_name = package.manifest_name.clone();
        for (name, bytes) in package
            .files
            .iter()
            .chain(std::iter::once((&manifest_name, &package.manifest_bytes)))
        {
            let target = staging.join(name);
            fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| e.to_string())?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        let saved = load(&staging)?;
        check(saved.value == package.value, "包副本写入后核验不符")?;
        if root.exists() {
            check(load(&root)?.value == package.value, "并发保存的完整包冲突")?;
        } else {
            fs::rename(&staging, &root).map_err(|e| format!("完整包原子保存失败：{e}"))?;
        }
    }
    Ok(
        json!({"version":"desktop-complete-handoff-pointer/v1","relativeDirectory":relative_root,"proof":package.proof(),"markdownSha256":package.value["catalog"]["markdownSha256"]}),
    )
}
pub(crate) fn save_pointer(tx: &Transaction<'_>, document_id: &str, pointer: &Value) -> Result<()> {
    let key = format!("{PREFIX}{document_id}");
    let old: Option<String> = tx
        .query_row("SELECT value FROM metadata WHERE key=?1", [&key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(old) = old {
        check(
            parse(old)? == *pointer,
            "此 MD 已有关联的不同完整包，原结果已保留",
        )?;
    } else {
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES(?1,?2)",
            params![key, pointer.to_string()],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub(crate) fn stored(store: &Store, document_id: &str) -> Result<Option<Value>> {
    let raw: Option<String> = store
        .conn()?
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("{PREFIX}{document_id}")],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(raw) = raw else { return Ok(None) };
    let pointer = parse(raw)?;
    check(
        pointer["version"] == "desktop-complete-handoff-pointer/v1"
            && pointer["markdownSha256"] == document_id,
        "保存的完整包指针不符",
    )?;
    let sha = s(&pointer["proof"], "manifestSha256");
    check(
        sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "包摘要格式错误",
    )?;
    let expected = format!("imports/complete-packages/{sha}");
    check(
        s(&pointer, "relativeDirectory") == expected,
        "包存储位置不符",
    )?;
    let package = load(&store.root.join(expected))?;
    check(
        package.proof() == pointer["proof"]
            && package.value["catalog"]["markdownSha256"] == document_id,
        "已保存包摘要或原 MD 关系不符",
    )?;
    Ok(Some(package.value))
}

#[cfg(test)]
#[path = "lingmo_package_tests.rs"]
mod tests;
