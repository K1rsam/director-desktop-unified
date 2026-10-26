//! Snapshot export of the current adopted images, never a media generator or
//! adoption action. The flat PNG folder contains no references or candidates.
use crate::store::{hash_file, id, now, parse, s, Result, Store};
use image::{GenericImageView, ImageFormat};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{BufWriter, Cursor, Write},
    path::{Path, PathBuf},
};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const VERSION: &str = "desktop-current-image-export/v1";
const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 100_000_000;
const IMAGE_DIR: &str = "图片";

#[cfg(test)]
#[path = "asset_image_export_tests.rs"]
mod tests;

fn rows(value: &Value, key: &str) -> Vec<Value> {
    value[key].as_array().cloned().unwrap_or_default()
}
fn collision_key(name: &str) -> String {
    name.nfkc().case_fold().nfkc().collect()
}
fn valid_filename(name: &str) -> Result<()> {
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("");
    let normalized = collision_key(stem);
    let device = normalized
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches([' ', '.']);
    let reserved = matches!(device, "con" | "prn" | "aux" | "nul" | "conin$" | "conout$")
        || (device.len() == 4
            && (device.starts_with("com") || device.starts_with("lpt"))
            && matches!(device.as_bytes()[3], b'1'..=b'9'));
    if stem.is_empty()
        || name == ".png"
        || !name.to_ascii_lowercase().ends_with(".png")
        || name.len() > 255
        || name.encode_utf16().count() > 255
        || name.trim() != name
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
        || path.components().count() != 1
        || !matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
        || reserved
    {
        return Err(format!(
            "IMAGE_EXPORT_FILENAME_INVALID:{name}:需要跨平台有效的同版纯PNG文件名，未自动改名"
        ));
    }
    Ok(())
}
fn filename(asset: &Value, deliveries: &[Value]) -> Result<(Option<String>, String)> {
    let mut declared = Vec::<(String, String)>::new();
    if !s(asset, "filename").is_empty() {
        declared.push((s(asset, "filename").into(), "asset-filename".into()));
    }
    for delivery in deliveries {
        for mapped in rows(&delivery["assetIndex"], "assets") {
            if mapped["id"] == asset["id"]
                && (mapped["variantId"].is_null() || mapped["variantId"] == asset["variantId"])
                && !s(&mapped, "filename").is_empty()
            {
                declared.push((s(&mapped, "filename").into(), "adopted-asset-index".into()));
            }
        }
    }
    if declared.windows(2).any(|p| p[0].0 != p[1].0) {
        return Err(format!(
            "IMAGE_EXPORT_FILENAME_MAPPING_CONFLICT:{}:资产与采用目录的精确文件名不一致",
            s(asset, "id")
        ));
    }
    if let Some((name, basis)) = declared.into_iter().next() {
        valid_filename(&name)?;
        return Ok((Some(name), basis));
    }
    // MD-only uses the same exact token contract as the existing image queue.
    // This never manufactures a filename from a mutable display name.
    if asset["origin"] == "imported-storyboard" {
        if let Some(token) = s(asset, "token")
            .strip_prefix("@[")
            .and_then(|v| v.strip_suffix(']'))
        {
            let ty = s(asset, "lingmoType");
            if ["CH", "SC", "PR", "OM"].contains(&ty)
                && token.strip_suffix(ty).is_some_and(|v| !v.is_empty())
            {
                let name = format!("{token}.png");
                valid_filename(&name)?;
                return Ok((Some(name), "source-token-contract".into()));
            }
        }
    }
    Ok((None, "missing-exact-filename".into()))
}
fn reference_only(asset: &Value, scope: &Value) -> bool {
    rows(scope, "referenceOnlyIds").contains(&asset["id"])
        || asset["referenceOnly"] == true
        || asset["materialRole"] == "MATERIAL_SOURCE_ONLY"
        || ["reference-only", "referenceOnly"].contains(&s(asset, "purpose"))
        || ["reference-only", "referenceOnly"].contains(&s(asset, "productionPurpose"))
}
fn freeze_locked(store: &Store, tx: &Transaction<'_>) -> Result<Value> {
    // The caller holds an IMMEDIATE transaction. Other connections used by the
    // existing scope API may read, while no adoption or unit edit can commit.
    let scope = store.image_production_scope()?;
    let ids = scope["assetIds"]
        .as_array()
        .ok_or("IMAGE_EXPORT_SCOPE_INVALID")?;
    let mut seen = HashSet::new();
    let deliveries = store.series_deliveries()?;
    let mut names = HashMap::<String, String>::new();
    let mut items = Vec::new();
    let review_guards = store.persisted_image_review_guards(tx, None)?;
    for asset_id in ids {
        let aid = asset_id
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("IMAGE_EXPORT_SCOPE_INVALID")?;
        if !seen.insert(aid) {
            return Err("IMAGE_EXPORT_SCOPE_DUPLICATE".into());
        }
        let raw: String = tx
            .query_row("SELECT data FROM assets WHERE id=?1", [aid], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut asset = parse(raw)?;
        Store::project_image_review_guard(&mut asset, &review_guards);
        let excluded =
            reference_only(&asset, &scope) || !crate::asset_graph::image_required(&asset);
        let (name, basis) = if excluded {
            (None, "excluded-reference".into())
        } else {
            filename(&asset, &deliveries)?
        };
        if let Some(name) = &name {
            if let Some(owner) = names.insert(collision_key(name), aid.to_owned()) {
                return Err(format!("IMAGE_EXPORT_FILENAME_COLLISION:{owner}:{aid}:{name}:跨平台文件名碰撞，未覆盖或自行改名"));
            }
        }
        let adoption: Option<(String, String)> = tx
            .query_row(
                "SELECT media_id,created_at FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
                params![aid, s(&asset, "variantId")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (adopted, media) = match adoption {
            Some((mid, time)) => {
                let raw: Option<String> = tx
                    .query_row("SELECT data FROM media WHERE id=?1", [&mid], |r| r.get(0))
                    .optional()
                    .map_err(|e| e.to_string())?;
                (
                    json!({"mediaId":mid,"adoptedAt":time}),
                    raw.map(parse).transpose()?.unwrap_or(Value::Null),
                )
            }
            None => (Value::Null, Value::Null),
        };
        items.push(json!({"asset":asset,"filename":name,"filenameBasis":basis,"excluded":excluded,"adoption":adopted,"media":media}));
    }
    let project: (String, String) = tx
        .query_row("SELECT id,title FROM project LIMIT 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| e.to_string())?;
    Ok(json!({"projectId":project.0,"projectTitle":project.1,"scope":scope,"items":items}))
}
fn freeze(store: &Store) -> Result<Value> {
    let mut connection = store.conn()?;
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    freeze_locked(store, &tx)
}
fn no_links(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err("IMAGE_EXPORT_PATH_NOT_ABSOLUTE".into());
    }
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component);
        // Rust treats a bare VerbatimDisk prefix (\\?\C:) as absolute, but
        // Windows rejects metadata on it with ERROR_INVALID_FUNCTION. Query
        // the root after RootDir, not an incomplete prefix. UNC paths also
        // expose a RootDir component, so their share root is still checked.
        if matches!(component, std::path::Component::Prefix(_)) || !cursor.is_absolute() {
            continue;
        }
        let metadata = fs::symlink_metadata(&cursor).map_err(|e| e.to_string())?;
        let mut linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            linked |= metadata.file_attributes() & 0x400 != 0;
        }
        if linked {
            return Err("IMAGE_EXPORT_LINK_PATH:导出路径含链接或重解析点".into());
        }
    }
    Ok(())
}
struct OwnedDirectory {
    root: PathBuf,
    files: Vec<PathBuf>,
    image_dir_created: bool,
    published: bool,
}
impl OwnedDirectory {
    fn create(parent: &Path) -> Result<Self> {
        let root = parent.join(format!(
            "正式图片-{}-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            &id()[..8]
        ));
        fs::create_dir(&root).map_err(|e| format!("新导出目录创建失败，未覆盖已有目录：{e}"))?;
        let mut owned = Self {
            root,
            files: Vec::new(),
            image_dir_created: false,
            published: false,
        };
        fs::create_dir(owned.root.join(IMAGE_DIR)).map_err(|e| e.to_string())?;
        owned.image_dir_created = true;
        owned.write(
            ".incomplete",
            b"Unpublished current-image snapshot; do not import yet.\n",
        )?;
        Ok(owned)
    }
    fn create_file(&mut self, relative: &str) -> Result<fs::File> {
        let path = self.root.join(relative);
        no_links(path.parent().ok_or("IMAGE_EXPORT_PARENT_MISSING")?)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("导出文件不能新建，未覆盖：{e}"))?;
        self.files.push(path);
        Ok(file)
    }
    fn write(&mut self, relative: &str, bytes: &[u8]) -> Result<()> {
        let mut file = self.create_file(relative)?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())
    }
}
impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        if self.published
            || no_links(&self.root).is_err()
            || (self.image_dir_created && no_links(&self.root.join(IMAGE_DIR)).is_err())
        {
            return;
        }
        // Delete only files this instance created, with no recursive deletion.
        for path in self.files.iter().rev() {
            let _ = fs::remove_file(path);
        }
        if self.image_dir_created {
            let _ = fs::remove_dir(self.root.join(IMAGE_DIR));
        }
        let _ = fs::remove_dir(&self.root);
    }
}
fn item_record(item: &Value) -> Value {
    let asset = &item["asset"];
    json!({"assetId":asset["id"],"variantId":asset["variantId"],"assetVersion":asset["version"],
        "assetRevision":asset["assetRevision"],"origin":asset["origin"],"importSource":asset["importSource"],"handoffRegistry":asset["handoffRegistry"],
        "designRevision":asset.get("designRevision").or_else(||asset.get("assetRevision")).cloned().unwrap_or(Value::Null),
        "externalId":asset["externalId"],"name":asset["name"],"importName":asset["importName"],"token":asset["token"],
        "filename":item["filename"],"filenameBasis":item["filenameBasis"],"adoption":item["adoption"],"imageAdoptionBasis":asset["imageAdoptionBasis"]})
}
fn missing(item: &Value, reason: &str, message: &str) -> Value {
    let mut row = item_record(item);
    row["reason"] = json!(reason);
    row["message"] = json!(message);
    row
}
fn write_image(store: &Store, item: &Value, directory: &mut OwnedDirectory) -> Result<Value> {
    let media = &item["media"];
    let path = store.absolute_media(media)?;
    no_links(&path)?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
        return Err("图片为空或超出本地无损导出大小上限".into());
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let original_sha = format!("{:x}", Sha256::digest(&bytes));
    if original_sha != s(media, "sha256")
        || media["size"]
            .as_u64()
            .is_some_and(|size| size != bytes.len() as u64)
    {
        return Err("当前原图哈希或大小不一致，未把损坏文件导出为正式图片".into());
    }
    let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("当前格式不支持保持原像素转PNG".into());
    }
    let dimensions = image::ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if u64::from(dimensions.0) * u64::from(dimensions.1) > MAX_IMAGE_PIXELS {
        return Err("当前图片超出1亿像素本地导出上限".into());
    }
    if format == ImageFormat::WebP
        && image::codecs::webp::WebPDecoder::new(Cursor::new(&bytes))
            .map_err(|e| e.to_string())?
            .has_animation()
    {
        return Err("动态WebP不能静默丢帧转单张PNG，原文件保留".into());
    }
    let decoded = image::load_from_memory_with_format(&bytes, format).map_err(|e| e.to_string())?;
    let (width, height) = decoded.dimensions();
    if width == 0
        || height == 0
        || media["width"]
            .as_u64()
            .is_some_and(|v| v != u64::from(width))
        || media["height"]
            .as_u64()
            .is_some_and(|v| v != u64::from(height))
    {
        return Err("原图尺寸与已核验媒体记录不一致".into());
    }
    if decoded
        .to_rgba8()
        .pixels()
        .all(|p| p[3] == 0 || p.0[..3].iter().all(|v| *v >= 250))
    {
        return Err("IMAGE_EXPORT_PLACEHOLDER_PIXELS:白图或全透明占位未导出为正式成图".into());
    }
    let relative = format!("{IMAGE_DIR}/{}", s(item, "filename"));
    if format == ImageFormat::Png {
        directory.write(&relative, &bytes)?;
    } else {
        let file = directory.create_file(&relative)?;
        let mut writer = BufWriter::new(file);
        decoded
            .write_to(&mut writer, ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        writer.get_ref().sync_all().map_err(|e| e.to_string())?;
    }
    let (output_sha, output_size) = hash_file(&directory.root.join(&relative))?;
    let mut row = item_record(item);
    row["mediaId"] = media["id"].clone();
    row["sourceRelativePath"] = media["relativePath"].clone();
    row["sourceSha256"] = json!(original_sha);
    row["outputSha256"] = json!(output_sha);
    row["outputSize"] = json!(output_size);
    row["width"] = json!(width);
    row["height"] = json!(height);
    row["outputPath"] = json!(relative);
    row["conversion"] = json!({"sourceFormat":format.extensions_str()[0],"outputFormat":"png","converted":format!=ImageFormat::Png,"resampled":false,"pixelDimensionsPreserved":true});
    Ok(row)
}
struct Prepared {
    snapshot: Value,
    directory: OwnedDirectory,
    exported: Vec<Value>,
    missing: Vec<Value>,
    excluded: Vec<Value>,
}
fn prepare(store: &Store, request: &Value) -> Result<Prepared> {
    let requested = Path::new(s(request, "path"));
    if !requested.is_absolute() || !requested.is_dir() {
        return Err("请选择已有的本地图片导出文件夹".into());
    }
    no_links(requested)?;
    let parent = requested.canonicalize().map_err(|e| e.to_string())?;
    let snapshot = freeze(store)?;
    if rows(&snapshot["scope"], "assetIds").is_empty() {
        return Err("当前正式图片范围为空".into());
    }
    let mut directory = OwnedDirectory::create(&parent)?;
    let mut exported = Vec::new();
    let mut missing_rows = Vec::new();
    let mut excluded = Vec::new();
    for item in rows(&snapshot, "items") {
        let asset = &item["asset"];
        let media = &item["media"];
        if item["excluded"] == true {
            excluded.push(missing(
                &item,
                "reference_only",
                "仅参考母版或非正式图片，不混入正式目录",
            ));
            continue;
        }
        if item["filename"].is_null() {
            missing_rows.push(missing(
                &item,
                "filename_missing",
                "缺少同版精确PNG文件名，未根据显示名称猜写",
            ));
            continue;
        }
        if crate::asset_graph::image_adoption_blocked(asset)
            || crate::asset_graph::design_review_blocked(asset) {
            missing_rows.push(missing(
                &item,
                "needs_review",
                "当前图片或设计尚需复核，保留历史采用但不作为最终成图导出",
            ));
            continue;
        }
        if item["adoption"].is_null() {
            missing_rows.push(missing(
                &item,
                "not_adopted",
                "尚无当前采用图片；候选或历史图不自动替代",
            ));
            continue;
        }
        if media.is_null() || media["kind"] != "image" {
            missing_rows.push(missing(
                &item,
                "media_missing",
                "当前采用媒体缺失或不是图片",
            ));
            continue;
        }
        if media["placeholder"] == true {
            missing_rows.push(missing(
                &item,
                "placeholder",
                "当前采用记录指向白图占位，未冒充正式成图",
            ));
            continue;
        }
        match write_image(store, &item, &mut directory) {
            Ok(row) => exported.push(row),
            Err(error) => {
                // A conversion failure may have created a partial file. It is
                // absent from the successful flat folder and retained in report.
                let output = directory.root.join(IMAGE_DIR).join(s(&item, "filename"));
                if directory.files.contains(&output) && output.is_file() {
                    no_links(&directory.root.join(IMAGE_DIR))?;
                    fs::remove_file(&output).map_err(|e| e.to_string())?;
                }
                let code = if error.starts_with("IMAGE_EXPORT_PLACEHOLDER_PIXELS") {
                    "placeholder"
                } else {
                    "media_unavailable"
                };
                missing_rows.push(missing(&item, code, &error));
            }
        }
    }
    Ok(Prepared {
        snapshot,
        directory,
        exported,
        missing: missing_rows,
        excluded,
    })
}
fn publish(store: &Store, mut prepared: Prepared) -> Result<Value> {
    let mut connection = store.conn()?;
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if freeze_locked(store, &tx)? != prepared.snapshot {
        return Err("IMAGE_EXPORT_SNAPSHOT_CHANGED:导出期间采用、范围或资产版本已变化，未发布旧映射；请重试".into());
    }
    no_links(&prepared.directory.root)?;
    no_links(&prepared.directory.root.join(IMAGE_DIR))?;
    let items = rows(&prepared.snapshot, "items");
    let by_id: HashMap<_, _> = items.iter().map(|i| (s(&i["asset"], "id"), i)).collect();
    for row in &prepared.exported {
        let item = by_id
            .get(s(row, "assetId"))
            .ok_or("IMAGE_EXPORT_SNAPSHOT_MISSING")?;
        let source = store.absolute_media(&item["media"])?;
        no_links(&source)?;
        store.verify_media(&item["media"])?;
        let output = prepared.directory.root.join(s(row, "outputPath"));
        no_links(&output)?;
        if hash_file(&output)?.0 != s(row, "outputSha256") {
            return Err("IMAGE_EXPORT_OUTPUT_CHANGED:导出文件在发布前发生变化".into());
        }
    }
    let count = prepared.exported.len();
    let missing_count = prepared.missing.len();
    let excluded_count = prepared.excluded.len();
    let status = if missing_count == 0 && excluded_count == 0 {
        "complete"
    } else if count > 0 {
        "partial"
    } else {
        "no_ready_images"
    };
    let manifest = json!({"version":VERSION,"createdAt":now(),"status":status,"projectId":prepared.snapshot["projectId"],"projectTitle":prepared.snapshot["projectTitle"],
        "imageDirectory":IMAGE_DIR,"scope":prepared.snapshot["scope"],"scopeCount":rows(&prepared.snapshot["scope"],"assetIds").len(),"count":count,"missingCount":missing_count,"excludedCount":excluded_count,
        "snapshotSha256":crate::pipeline::digest(&prepared.snapshot.to_string()),"exported":prepared.exported,"missing":prepared.missing,"excluded":prepared.excluded,
        "adoptionEvidence":"existing-current-adoption-records; no new approval or visual review claimed","generationSubmissions":0,"videoSubmissions":0});
    prepared.directory.write(
        "manifest.json",
        &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )?;
    fs::remove_file(prepared.directory.root.join(".incomplete")).map_err(|e| e.to_string())?;
    prepared.directory.published = true;
    Ok(
        json!({"version":VERSION,"path":prepared.directory.root,"imageDirectory":prepared.directory.root.join(IMAGE_DIR),"manifestPath":prepared.directory.root.join("manifest.json"),
        "status":status,"scopeCount":manifest["scopeCount"],"count":count,"missingCount":missing_count,"excludedCount":excluded_count,"missing":manifest["missing"],"excluded":manifest["excluded"],"generationSubmissions":0,"videoSubmissions":0}),
    )
}
impl Store {
    /// `path` is an existing absolute parent directory. Every call exclusively
    /// creates one new package; request-side asset IDs never broaden its scope.
    pub fn export_current_images(&self, request: &Value) -> Result<Value> {
        publish(self, prepare(self, request)?)
    }
}
