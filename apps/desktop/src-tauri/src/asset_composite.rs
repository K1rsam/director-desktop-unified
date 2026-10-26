//! Deterministic identity-strip composition. Never calls a generation service.
//! Decoded mother pixels are copied one-for-one, without resizing or filtering.
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const PROFILE: &str = "asset-identity-pixel-composite/v1";
pub const LOWER_ADAPTIVE_PROFILE: &str = "asset-identity-pixel-composite/v2";
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_matches(value: &str, actual: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|c| c.is_ascii_hexdigit())
        && value.eq_ignore_ascii_case(actual)
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}
#[derive(Clone, Copy, Debug)]
struct Slot {
    source: Rect,
    x: u32,
    y: u32,
}
fn integer(value: &Value, key: &str) -> Result<u32, String> {
    value[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("布局 {key} 必须为非负整数像素"))
}
fn rect(value: &Value) -> Result<Rect, String> {
    Ok(Rect {
        x: integer(value, "x")?,
        y: integer(value, "y")?,
        width: integer(value, "width")?,
        height: integer(value, "height")?,
    })
}
fn in_bounds(r: Rect, width: u32, height: u32) -> bool {
    r.width > 0
        && r.height > 0
        && r.x.checked_add(r.width).is_some_and(|end| end <= width)
        && r.y.checked_add(r.height).is_some_and(|end| end <= height)
}
fn overlaps(a: Rect, b: Rect) -> bool {
    u64::from(a.x) < u64::from(b.x) + u64::from(b.width)
        && u64::from(b.x) < u64::from(a.x) + u64::from(a.width)
        && u64::from(a.y) < u64::from(b.y) + u64::from(b.height)
        && u64::from(b.y) < u64::from(a.y) + u64::from(a.height)
}
fn target(slot: Slot) -> Rect {
    Rect {
        x: slot.x,
        y: slot.y,
        width: slot.source.width,
        height: slot.source.height,
    }
}
fn slot_value(slot: Slot) -> Value {
    json!({"source":{"x":slot.source.x,"y":slot.source.y,"width":slot.source.width,"height":slot.source.height},"target":{"x":slot.x,"y":slot.y,"width":slot.source.width,"height":slot.source.height}})
}

fn layout(config: &Value, width: u32, height: u32) -> Result<Vec<Slot>, String> {
    let strip = config.get("identityStrip").filter(|v| !v.is_null());
    let slots = config.get("slots").filter(|v| !v.is_null());
    if strip.is_some() == slots.is_some() {
        return Err("必须明确选择上排 identityStrip 或三个 slots，不能缺失或同时提供".into());
    }
    if let Some(strip) = strip {
        let r = rect(strip)?;
        if !in_bounds(r, width, height)
            || r.x != 0
            || r.y != 0
            || r.width != width
            || r.height >= height
        {
            return Err("身份上排必须从左上角覆盖完整画布宽度且保留下排；不猜测裁切线".into());
        }
        return Ok(vec![Slot {
            source: r,
            x: r.x,
            y: r.y,
        }]);
    }
    let rows = slots
        .and_then(Value::as_array)
        .filter(|v| v.len() == 3)
        .ok_or("锁脸布局必须有明确的三个等像素槽位")?;
    let mut result: Vec<Slot> = Vec::new();
    for row in rows {
        let r = rect(&row["source"])?;
        let destination = &row["target"];
        if destination
            .get("width")
            .is_some_and(|v| v.as_u64() != Some(u64::from(r.width)))
            || destination
                .get("height")
                .is_some_and(|v| v.as_u64() != Some(u64::from(r.height)))
        {
            return Err("锁脸目标槽位不能改变母版裁片尺寸；禁止缩放/重采样".into());
        }
        let slot = Slot {
            source: r,
            x: integer(destination, "x")?,
            y: integer(destination, "y")?,
        };
        if !in_bounds(r, width, height) || !in_bounds(target(slot), width, height) {
            return Err("母版或目标锁脸裁片越出画布".into());
        }
        if result
            .iter()
            .any(|old| overlaps(old.source, r) || overlaps(target(*old), target(slot)))
        {
            return Err("三个母版裁片或目标槽位重叠，未执行合成".into());
        }
        result.push(slot);
    }
    Ok(result)
}

fn decode(bytes: &[u8], width: u32, height: u32, label: &str) -> Result<RgbaImage, String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let actual = reader
        .into_dimensions()
        .map_err(|e| format!("{label}无法读取尺寸：{e}"))?;
    if actual != (width, height) {
        return Err(format!(
            "{label}尺寸为{}×{}，冻结画布为{width}×{height}；不会自动缩放",
            actual.0, actual.1
        ));
    }
    let image = image::load_from_memory(bytes).map_err(|e| format!("{label}无法解码：{e}"))?;
    match image {
        DynamicImage::ImageRgba8(v) => Ok(v),
        DynamicImage::ImageRgb8(v) => Ok(DynamicImage::ImageRgb8(v).into_rgba8()),
        _ => Err(format!(
            "{label}不是8位RGB/RGBA图；避免量化改变身份原像素，需重新提供可核母版"
        )),
    }
}

/// A v2 request freezes both the inspected identity boundary and the only
/// permitted size adaptation. Never infer this contract for historical v1 jobs.
pub(crate) fn lower_adaptation_contract(config: &Value) -> Result<Value, String> {
    let width = integer(config, "width")?;
    let height = integer(config, "height")?;
    let slots = layout(config, width, height)?;
    if width == 0
        || width != height
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || config.get("identityStrip").is_none_or(Value::is_null)
        || slots.len() != 1
    {
        return Err("下排尺寸适配仅支持明确登记的1:1完整身份上排，不猜测三个裁片的服装区域".into());
    }
    let top = slots[0].source.height;
    Ok(
        json!({"version":"asset-costume-lower-adaptation/v1","inputCanvas":"square",
        "sourceTop":{"rule":"ceil-ratio","numerator":top,"denominator":height},
        "target":{"x":0,"y":top,"width":width,"height":height-top},
        "order":"crop-then-resize","filter":"Lanczos3","discardSourceTop":true,
        "identityResampling":false}),
    )
}

fn lower_mapping(config: &Value, source_width: u32, source_height: u32) -> Result<Value, String> {
    if source_width == 0
        || source_width != source_height
        || u64::from(source_width) * u64::from(source_height) > MAX_PIXELS
    {
        return Err("下排尺寸适配只接收64M像素以内的正方形原图；非正方形保留待复核".into());
    }
    let contract = lower_adaptation_contract(config)?;
    if config["lowerAdaptation"] != contract {
        return Err("缺少或不匹配冻结的下排尺寸适配合同，未自动缩放".into());
    }
    let height = integer(config, "height")?;
    let top = integer(&config["identityStrip"], "height")?;
    let source_top = ((u64::from(source_height) * u64::from(top) + u64::from(height) - 1)
        / u64::from(height)) as u32;
    if source_top >= source_height {
        return Err("渠道原图过小，按冻结比例向上取整后已无可用服装下排".into());
    }
    let resized =
        source_width != integer(config, "width")? || source_height - source_top != height - top;
    Ok(
        json!({"sourceWidth":source_width,"sourceHeight":source_height,
        "sourceCrop":{"x":0,"y":source_top,"width":source_width,"height":source_height-source_top},
        "target":contract["target"],"order":"crop-then-resize",
        "interpolation":if resized {"Lanczos3"} else {"none"},"resampled":resized,
        "sourceTopDiscarded":true}),
    )
}

fn lower_layer(
    config: &Value,
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<(RgbaImage, Option<Value>), String> {
    if config["version"] == PROFILE {
        return decode(bytes, width, height, "下排候选").map(|image| (image, None));
    }
    let (source_width, source_height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| format!("下排候选无法读取尺寸：{e}"))?;
    let mut proof = lower_mapping(config, source_width, source_height)?;
    let original = decode(bytes, source_width, source_height, "下排候选")?;
    let source = rect(&proof["sourceCrop"])?;
    let target = rect(&proof["target"])?;
    // Crop FIRST: filtering the whole image would sample discarded identity
    // pixels across the boundary and contaminate the new clothing region.
    let cropped =
        image::imageops::crop_imm(&original, source.x, source.y, source.width, source.height)
            .to_image();
    proof["sourceCropRgbaSha256"] = json!(hash(cropped.as_raw()));
    let lower = if proof["resampled"] == true {
        image::imageops::resize(
            &cropped,
            target.width,
            target.height,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        cropped
    };
    proof["outputRgbaSha256"] = json!(hash(lower.as_raw()));
    let mut canvas = RgbaImage::new(width, height);
    image::imageops::replace(
        &mut canvas,
        &lower,
        i64::from(target.x),
        i64::from(target.y),
    );
    Ok((canvas, Some(proof)))
}

pub fn requires_composition(snapshot: &Value) -> bool {
    snapshot
        .get("identityComposite")
        .is_some_and(|v| !v.is_null())
        || snapshot
            .pointer("/imagePresentation/layoutKind")
            .and_then(Value::as_str)
            == Some("costume")
        || snapshot
            .pointer("/imagePrompt/layoutKind")
            .and_then(Value::as_str)
            == Some("costume")
        || snapshot
            .pointer("/asset/imageTask/kind")
            .and_then(Value::as_str)
            == Some("costume")
}

struct FrozenMother<'a> {
    width: u32,
    height: u32,
    slots: Vec<Slot>,
    reference: &'a Value,
    path: PathBuf,
    sha256: String,
    image: RgbaImage,
}

fn frozen_mother(snapshot: &Value) -> Result<FrozenMother<'_>, String> {
    let config = &snapshot["identityComposite"];
    if config["version"] != PROFILE && config["version"] != LOWER_ADAPTIVE_PROFILE {
        return Err("缺少或不支持冻结的身份像素合成布局；下排候选保留待复核".into());
    }
    let width = integer(config, "width")?;
    let height = integer(config, "height")?;
    if width == 0
        || height == 0
        || width != height
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("身份板必须是已冻结的1:1有效画布，像素数不得超过64M".into());
    }
    let slots = layout(config, width, height)?;
    if config["version"] == LOWER_ADAPTIVE_PROFILE
        && config["lowerAdaptation"] != lower_adaptation_contract(config)?
    {
        return Err("缺少或不匹配冻结的下排尺寸适配合同，未自动缩放".into());
    }
    let media_id = s(config, "motherMediaId");
    if media_id.is_empty() {
        return Err("合成缺少已冻结的身份母版媒体ID".into());
    }
    let refs: Vec<_> = snapshot["references"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| s(r, "mediaId") == media_id)
        .collect();
    if refs.len() != 1 {
        return Err("身份母版不唯一或不在冻结的真实参考媒体中".into());
    }
    let reference = refs[0];
    if reference["kind"] != "image" {
        return Err("身份母版引用不是图片".into());
    }
    if let Some(expected) = config.get("motherMediaRevision").filter(|v| !v.is_null()) {
        let actual = reference
            .get("mediaRevision")
            .or_else(|| reference.get("version"));
        if actual != Some(expected) {
            return Err("合成母图媒体版本与冻结引用不匹配".into());
        }
    }
    if let Some(requirements) = snapshot
        .get("referenceRequirements")
        .filter(|v| !v.is_null())
    {
        let requirements = requirements.as_array().ok_or("合成参考需求不是有效列表")?;
        if !requirements.iter().any(|r| {
            let role = r["role"]
                .as_str()
                .or_else(|| r["kind"].as_str())
                .unwrap_or("");
            r["required"] != false
                && r["targetAssetId"] == reference["assetId"]
                && ["face", "identity", "identity-faces"].contains(&role)
                && r.get("targetVariantId")
                    .filter(|v| !v.is_null())
                    .is_none_or(|v| v == &reference["variantId"])
        }) {
            return Err("合成母图不属于当前明确登记的身份锁脸需求".into());
        }
    }
    let mother_path = Path::new(s(reference, "path"));
    if !mother_path.is_absolute() {
        return Err("合成必须使用已核绝对路径".into());
    }
    let path = mother_path
        .canonicalize()
        .map_err(|e| format!("身份母版文件不可读：{e}"))?;
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let sha256 = hash(&bytes);
    if !hash_matches(s(reference, "sha256"), &sha256)
        || !hash_matches(s(config, "motherSha256"), &sha256)
    {
        return Err("身份母版文件或冻结摘要已变化，未使用新图替代旧母版".into());
    }
    let image = decode(&bytes, width, height, "身份母版")?;
    Ok(FrozenMother {
        width,
        height,
        slots,
        reference,
        path,
        sha256,
        image,
    })
}

/// Validate before sending a paid lower-image request. This is deterministic
/// file/layout validation, not an image-quality review or adoption decision.
pub fn validate_snapshot(snapshot: &Value) -> Result<Value, String> {
    if !requires_composition(snapshot) {
        return Ok(json!({"status":"not_required","imageApproved":false}));
    }
    let frozen = frozen_mother(snapshot)?;
    Ok(
        json!({"status":"frozen_composition_valid","version":snapshot["identityComposite"]["version"],
        "motherMediaId":frozen.reference["mediaId"],"motherSha256":frozen.sha256,
        "motherMediaRevision":frozen.reference.get("mediaRevision").or_else(||frozen.reference.get("version")),
        "width":frozen.width,"height":frozen.height,"regions":frozen.slots.len(),
        "imageApproved":false,"visualReviewRequired":true}),
    )
}

/// The output must be a separate immutable candidate path. A failed or missing
/// composition contract retains the downloaded lower image for inspection.
/// A pixel proof is never a visual-quality approval or an adoption receipt.
pub fn compose_downloaded(
    snapshot: &Value,
    lower_path: &Path,
    output_path: &Path,
) -> Result<Value, String> {
    if !requires_composition(snapshot) {
        return Ok(
            json!({"status":"not_required","candidatePath":lower_path,"imageApproved":false}),
        );
    }
    match compose(snapshot, lower_path, output_path) {
        Ok(proof) => Ok(
            json!({"status":"composited_candidate","candidatePath":output_path,"lowerPath":lower_path,"proof":proof,"imageApproved":false,"visualReviewRequired":true}),
        ),
        Err(reason) => Ok(
            json!({"status":"needs_review","candidatePath":lower_path,"lowerPath":lower_path,"reason":reason,"lowerPreserved":lower_path.is_file(),"lowerModified":false,"imageApproved":false,"pixelEquality":false,"visualReviewRequired":true}),
        ),
    }
}

/// Recheck the immutable output and copied regions at adoption, not just download.
/// This proves identity pixels only; the caller still owns visual/user approval.
pub fn validate_adoption(
    snapshot: &Value,
    composition: &Value,
    candidate: &Path,
) -> Result<(), String> {
    if !requires_composition(snapshot) {
        return Ok(());
    }
    if composition["status"] != "composited_candidate"
        || composition["proof"]["pixelEquality"] != true
    {
        return Err("换装图片尚无有效原像素合成证明；下排候选保留，不能直接采用".into());
    }
    let frozen = frozen_mother(snapshot)?;
    let config = &snapshot["identityComposite"];
    let proof = &composition["proof"];
    if config["version"] == PROFILE && proof["resampling"] != false {
        return Err("旧版合成合同禁止重采样，未采用".into());
    }
    let bytes = fs::read(candidate).map_err(|e| e.to_string())?;
    if hash(&bytes) != s(&composition["proof"], "outputSha256") {
        return Err("合成候选与像素证明的文件不一致，未采用".into());
    }
    let output = decode(&bytes, frozen.width, frozen.height, "待采用合成图片")?;
    if config["version"] == LOWER_ADAPTIVE_PROFILE {
        let lower = &proof["lowerTransform"];
        let mapping = lower_mapping(
            config,
            integer(lower, "sourceWidth")?,
            integer(lower, "sourceHeight")?,
        )?;
        let matching_mapping = mapping
            .as_object()
            .unwrap()
            .iter()
            .all(|(key, value)| lower.get(key) == Some(value));
        let resampled = mapping["resampled"] == true;
        if proof["version"] != LOWER_ADAPTIVE_PROFILE
            || proof["layout"] != *config
            || proof["identityResampling"] != false
            || proof["colorTransform"] != false
            || proof["lowerResampling"] != resampled
            || proof["resampling"] != resampled
            || proof["outsideRegionsUnchanged"] != !resampled
            || proof["lowerMatchesFrozenTransform"] != true
            || proof["motherSha256"] != frozen.sha256
            || !matching_mapping
            || !hash_matches(s(proof, "lowerSha256"), s(proof, "lowerSha256"))
            || !hash_matches(
                s(lower, "sourceCropRgbaSha256"),
                s(lower, "sourceCropRgbaSha256"),
            )
        {
            return Err("换装下排适配证明与冻结合同不一致，未采用".into());
        }
        let target = rect(&mapping["target"])?;
        let actual =
            image::imageops::crop_imm(&output, target.x, target.y, target.width, target.height)
                .to_image();
        if !hash_matches(s(lower, "outputRgbaSha256"), &hash(actual.as_raw())) {
            return Err("采用前服装下排像素与适配证明不一致，未采用".into());
        }
    }
    for slot in frozen.slots {
        for y in 0..slot.source.height {
            for x in 0..slot.source.width {
                if frozen.image.get_pixel(slot.source.x + x, slot.source.y + y)
                    != output.get_pixel(slot.x + x, slot.y + y)
                {
                    return Err("采用前原像素核验失败，未采用".into());
                }
            }
        }
    }
    Ok(())
}

fn compose(snapshot: &Value, lower_path: &Path, output_path: &Path) -> Result<Value, String> {
    let config = &snapshot["identityComposite"];
    let FrozenMother {
        width,
        height,
        slots,
        reference,
        path: mother_canonical,
        sha256: mother_hash,
        image: mother,
    } = frozen_mother(snapshot)?;
    let media_id = s(reference, "mediaId");
    if !lower_path.is_absolute() || !output_path.is_absolute() {
        return Err("合成必须使用已核绝对路径".into());
    }
    let lower_canonical = lower_path
        .canonicalize()
        .map_err(|e| format!("下排候选不可读：{e}"))?;
    let parent = output_path
        .parent()
        .ok_or("合成输出缺少目录")?
        .canonicalize()
        .map_err(|e| format!("合成输出目录不可用：{e}"))?;
    let output = parent.join(output_path.file_name().ok_or("合成输出文件名无效")?);
    if output == lower_canonical
        || output == mother_canonical
        || output.exists()
            && output
                .canonicalize()
                .is_ok_and(|p| p == lower_canonical || p == mother_canonical)
    {
        return Err("合成输出不能覆盖母版或下载原件".into());
    }
    if output
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("png"))
    {
        return Err("合成候选必须使用独立PNG文件".into());
    }
    let lower_bytes = fs::read(&lower_canonical).map_err(|e| e.to_string())?;
    let lower_hash = hash(&lower_bytes);
    let (lower, lower_transform) = lower_layer(config, &lower_bytes, width, height)?;
    let mut candidate = lower.clone();
    for slot in &slots {
        for y in 0..slot.source.height {
            for x in 0..slot.source.width {
                candidate.put_pixel(
                    slot.x + x,
                    slot.y + y,
                    *mother.get_pixel(slot.source.x + x, slot.source.y + y),
                );
            }
        }
    }
    // Compare against the frozen lower mapping (v1: unchanged original).
    for y in 0..height {
        for x in 0..width {
            if let Some(slot) = slots.iter().find(|slot| {
                x >= slot.x
                    && y >= slot.y
                    && x - slot.x < slot.source.width
                    && y - slot.y < slot.source.height
            }) {
                if candidate.get_pixel(x, y)
                    != mother.get_pixel(slot.source.x + x - slot.x, slot.source.y + y - slot.y)
                {
                    return Err("身份裁片原像素核验失败".into());
                }
            } else if candidate.get_pixel(x, y) != lower.get_pixel(x, y) {
                return Err("合成意外改写已映射的下排像素".into());
            }
        }
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(candidate.clone())
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let bytes = encoded.into_inner();
    let roundtrip = decode(&bytes, width, height, "合成PNG")?;
    if roundtrip.as_raw() != candidate.as_raw() {
        return Err("合成PNG编码后的像素核验失败".into());
    }
    // Re-read both inputs immediately before publication. Never publish a
    // candidate against an input replaced while decoding or copying.
    if hash(&fs::read(&mother_canonical).map_err(|e| e.to_string())?) != mother_hash
        || hash(&fs::read(&lower_canonical).map_err(|e| e.to_string())?) != lower_hash
    {
        return Err("合成期间母版或下排文件发生变化，未发布输出".into());
    }
    let reused = if output.exists() {
        let old = fs::read(&output).map_err(|e| e.to_string())?;
        if old != bytes {
            return Err("合成输出位置已有不同候选，未覆盖历史图片".into());
        }
        true
    } else {
        let mut temporary = tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
        temporary.write_all(&bytes).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary
            .persist_noclobber(&output)
            .map_err(|e| format!("合成候选发布失败，未覆盖已有文件：{}", e.error))?;
        false
    };
    let output_hash = hash(&bytes);
    let regions:Vec<_>=slots.iter().map(|slot|{
        let mut region=Vec::with_capacity((u64::from(slot.source.width)*u64::from(slot.source.height)*4) as usize);
        let mut output_region=Vec::with_capacity(region.capacity());
        for y in 0..slot.source.height{for x in 0..slot.source.width{
            region.extend_from_slice(&mother.get_pixel(slot.source.x+x,slot.source.y+y).0);
            output_region.extend_from_slice(&roundtrip.get_pixel(slot.x+x,slot.y+y).0);
        }}
        json!({"mapping":slot_value(*slot),"motherRgbaSha256":hash(&region),"outputRgbaSha256":hash(&output_region),"pixelEquality":region==output_region})
    }).collect();
    let mut proof = json!({"version":config["version"],"motherMediaId":media_id,"motherMediaRevision":reference.get("mediaRevision").or_else(||reference.get("version")),"motherAssetId":reference["assetId"],"motherSha256":mother_hash,"lowerSha256":lower_hash,"outputSha256":output_hash,"width":width,"height":height,"layout":config,"regions":regions,"pixelEquality":true,"outsideRegionsUnchanged":true,"resampling":false,"colorTransform":false,"imageApproved":false,"visualReviewRequired":true,"reused":reused});
    if let Some(transform) = lower_transform {
        let resampled = transform["resampled"] == true;
        proof["identityResampling"] = json!(false);
        proof["lowerResampling"] = json!(resampled);
        proof["resampling"] = json!(resampled);
        proof["outsideRegionsUnchanged"] = json!(!resampled);
        proof["lowerMatchesFrozenTransform"] = json!(true);
        proof["lowerTransform"] = transform;
    }
    Ok(proof)
}

#[cfg(test)]
#[path = "asset_composite_v2_tests.rs"]
mod v2_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn png(path: &Path, seed: u8, size: u32) {
        let image = RgbaImage::from_fn(size, size, |x, y| {
            image::Rgba([
                seed.wrapping_add(x as u8),
                seed.wrapping_add((y * 3) as u8),
                x.wrapping_mul(y) as u8,
                255,
            ])
        });
        image.save(path).unwrap();
    }
    fn fixture() -> (
        tempfile::TempDir,
        Value,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mother = dir.path().join("mother.png");
        let lower = dir.path().join("lower.png");
        let output = dir.path().join("composite.png");
        png(&mother, 10, 12);
        png(&lower, 110, 12);
        let sha = hash(&fs::read(&mother).unwrap());
        let snapshot = json!({"imagePresentation":{"layoutKind":"costume"},"references":[{"mediaId":"mother-1","kind":"image","path":mother,"sha256":sha,"mediaRevision":3,"assetId":"person-1"}],"identityComposite":{"version":PROFILE,"width":12,"height":12,"motherMediaId":"mother-1","motherSha256":sha,"identityStrip":{"x":0,"y":0,"width":12,"height":4}}});
        (dir, snapshot, lower, output)
    }
    #[test]
    fn identity_strip_copies_exact_pixels_and_leaves_lower_and_sources_unchanged() {
        let (_dir, snapshot, lower, output) = fixture();
        let mother = Path::new(s(&snapshot["references"][0], "path"));
        let before = fs::read(&lower).unwrap();
        let mother_before = fs::read(mother).unwrap();
        let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
        assert_eq!(result["status"], "composited_candidate");
        assert_eq!(result["proof"]["pixelEquality"], true);
        assert_eq!(result["imageApproved"], false);
        validate_adoption(&snapshot, &result, &output).unwrap();
        assert!(validate_adoption(&snapshot, &result, &lower).is_err());
        assert!(validate_adoption(&snapshot, &json!({"status":"needs_review"}), &lower).is_err());
        let original = image::open(&lower).unwrap().to_rgba8();
        let identity = image::open(mother).unwrap().to_rgba8();
        let composed = image::open(&output).unwrap().to_rgba8();
        for y in 0..12 {
            for x in 0..12 {
                assert_eq!(
                    composed.get_pixel(x, y),
                    if y < 4 {
                        identity.get_pixel(x, y)
                    } else {
                        original.get_pixel(x, y)
                    }
                );
            }
        }
        assert_eq!(fs::read(&lower).unwrap(), before);
        assert_eq!(fs::read(mother).unwrap(), mother_before);
        let again = compose_downloaded(&snapshot, &lower, &output).unwrap();
        assert_eq!(again["proof"]["reused"], true);
        assert_eq!(
            again["proof"]["outputSha256"],
            result["proof"]["outputSha256"]
        );
    }
    #[test]
    fn preflight_checks_mother_before_any_lower_image_exists() {
        let (_dir, mut snapshot, lower, output) = fixture();
        fs::remove_file(&lower).unwrap();
        let result = validate_snapshot(&snapshot).unwrap();
        assert_eq!(result["status"], "frozen_composition_valid");
        assert_eq!(result["imageApproved"], false);
        assert!(!lower.exists());
        assert!(!output.exists());
        snapshot["identityComposite"]["motherSha256"] = json!("0".repeat(64));
        assert!(validate_snapshot(&snapshot).unwrap_err().contains("摘要"));
        assert!(!output.exists());
    }
    #[test]
    fn store_adoption_rejects_lower_only_but_accepts_the_proven_composite() {
        let (dir, mut snapshot, lower, output) = fixture();
        let store = crate::store::Store::open(dir.path().join("project")).unwrap();
        store.create("合成采用", "project").unwrap();
        store
            .save_asset(json!({"id":"child","variantId":"v1","kind":"CH","name":"换装"}))
            .unwrap();
        snapshot["assetId"] = json!("child");
        snapshot["variantId"] = json!("v1");
        let lower_media = store.import_media(&lower, "image", None, None).unwrap();
        let job = store
            .add_job(
                "image",
                "child",
                "offline",
                "offline",
                &snapshot,
                "downloaded",
            )
            .unwrap();
        store.attach_result(&job, s(&lower_media, "id")).unwrap();
        assert!(store
            .adopt("child", "v1", s(&lower_media, "id"))
            .unwrap_err()
            .contains("合成记录"));
        assert!(store.adopted("child", "v1").unwrap().is_none());
        let mut result = compose_downloaded(&snapshot, &lower, &output).unwrap();
        let media = store.import_media(&output, "image", None, None).unwrap();
        result["candidateMediaId"] = media["id"].clone();
        store.attach_result(&job, s(&media, "id")).unwrap();
        store
            .update_job(
                &job,
                "downloaded",
                &json!({"result":{"identityComposition":[result]}}),
            )
            .unwrap();
        store.adopt("child", "v1", s(&media, "id")).unwrap();
        assert_eq!(
            store.adopted("child", "v1").unwrap().as_deref(),
            media["id"].as_str()
        );
    }
    #[test]
    fn preflight_rejects_wrong_mother_version_or_a_non_identity_reference() {
        let (_dir, mut snapshot, _lower, _output) = fixture();
        snapshot["identityComposite"]["motherMediaRevision"] = json!(2);
        assert!(validate_snapshot(&snapshot).unwrap_err().contains("版本"));
        snapshot["identityComposite"]["motherMediaRevision"] = json!(3);
        snapshot["referenceRequirements"] = json!([{"targetAssetId":"person-1","role":"clothing"}]);
        assert!(validate_snapshot(&snapshot)
            .unwrap_err()
            .contains("身份锁脸"));
        snapshot["referenceRequirements"][0]["role"] = json!("identity-faces");
        assert!(validate_snapshot(&snapshot).is_ok());
        snapshot["referenceRequirements"][0]["targetAssetId"] = json!("different-person");
        assert!(validate_snapshot(&snapshot)
            .unwrap_err()
            .contains("身份锁脸"));
    }
    #[test]
    fn three_slots_move_only_equal_sized_original_rectangles() {
        let (_dir, mut snapshot, lower, output) = fixture();
        snapshot["identityComposite"]
            .as_object_mut()
            .unwrap()
            .remove("identityStrip");
        snapshot["identityComposite"]["slots"] = json!([
            {"source":{"x":0,"y":0,"width":2,"height":3},"target":{"x":1,"y":1}},
            {"source":{"x":4,"y":0,"width":2,"height":3},"target":{"x":5,"y":1}},
            {"source":{"x":8,"y":0,"width":2,"height":3},"target":{"x":9,"y":1}}
        ]);
        let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
        assert_eq!(result["status"], "composited_candidate");
        assert_eq!(result["proof"]["regions"].as_array().unwrap().len(), 3);
        let mother = image::open(s(&snapshot["references"][0], "path"))
            .unwrap()
            .to_rgba8();
        let actual = image::open(output).unwrap().to_rgba8();
        for x in [0, 4, 8] {
            for yy in 0..3 {
                for xx in 0..2 {
                    assert_eq!(
                        mother.get_pixel(x + xx, yy),
                        actual.get_pixel(x + 1 + xx, yy + 1)
                    );
                }
            }
        }
    }
    #[test]
    fn missing_layout_mother_drift_wrong_size_and_ambiguous_refs_keep_the_lower_candidate() {
        for case in [
            "missing",
            "version",
            "hash",
            "media",
            "duplicate",
            "size",
            "mother-size",
            "bounds",
            "full-height",
            "overlap",
            "scale",
            "depth",
        ] {
            let (_dir, mut snapshot, lower, output) = fixture();
            let before = fs::read(&lower).unwrap();
            match case {
                "missing" => {
                    snapshot
                        .as_object_mut()
                        .unwrap()
                        .remove("identityComposite");
                }
                "version" => snapshot["identityComposite"]["version"] = json!("future"),
                "hash" => snapshot["identityComposite"]["motherSha256"] = json!("0".repeat(64)),
                "media" => snapshot["identityComposite"]["motherMediaId"] = json!("another-mother"),
                "duplicate" => {
                    let reference = snapshot["references"][0].clone();
                    snapshot["references"]
                        .as_array_mut()
                        .unwrap()
                        .push(reference);
                }
                "size" => snapshot["identityComposite"]["width"] = json!(13),
                "mother-size" => {
                    let path = Path::new(s(&snapshot["references"][0], "path"));
                    png(path, 10, 10);
                    let h = hash(&fs::read(path).unwrap());
                    snapshot["references"][0]["sha256"] = json!(h);
                    snapshot["identityComposite"]["motherSha256"] = json!(h);
                }
                "bounds" => snapshot["identityComposite"]["identityStrip"]["width"] = json!(13),
                "full-height" => {
                    snapshot["identityComposite"]["identityStrip"]["height"] = json!(12)
                }
                "overlap" | "scale" => {
                    snapshot["identityComposite"]
                        .as_object_mut()
                        .unwrap()
                        .remove("identityStrip");
                    let mut slot =
                        json!({"source":{"x":0,"y":0,"width":2,"height":2},"target":{"x":0,"y":0}});
                    if case == "scale" {
                        slot["target"]["width"] = json!(3);
                    }
                    snapshot["identityComposite"]["slots"] = json!([slot, slot, slot]);
                }
                "depth" => {
                    let path = Path::new(s(&snapshot["references"][0], "path"));
                    DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                        12,
                        12,
                        image::Rgba([1000u16, 2000, 3000, 65535]),
                    ))
                    .save(path)
                    .unwrap();
                    let h = hash(&fs::read(path).unwrap());
                    snapshot["references"][0]["sha256"] = json!(h);
                    snapshot["identityComposite"]["motherSha256"] = json!(h);
                }
                _ => unreachable!(),
            }
            let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
            assert_eq!(result["status"], "needs_review", "{case}: {result}");
            assert_eq!(result["pixelEquality"], false);
            assert!(!output.exists(), "{case}");
            assert_eq!(fs::read(lower).unwrap(), before, "{case}");
        }
    }
    #[test]
    fn output_must_not_overwrite_a_source_or_existing_candidate() {
        let (_dir, snapshot, lower, output) = fixture();
        let before = fs::read(&lower).unwrap();
        let result = compose_downloaded(&snapshot, &lower, &lower).unwrap();
        assert_eq!(result["status"], "needs_review");
        assert_eq!(fs::read(&lower).unwrap(), before);
        png(&output, 77, 12);
        let prior = fs::read(&output).unwrap();
        let result = compose_downloaded(&snapshot, &lower, &output).unwrap();
        assert_eq!(result["status"], "needs_review");
        assert_eq!(fs::read(output).unwrap(), prior);
    }
    #[test]
    fn ordinary_images_do_not_gain_a_composition_or_approval_claim() {
        let (_dir, _snapshot, lower, output) = fixture();
        let result = compose_downloaded(
            &json!({"imagePresentation":{"layoutKind":"single-character"}}),
            &lower,
            &output,
        )
        .unwrap();
        assert_eq!(result["status"], "not_required");
        assert_eq!(result["imageApproved"], false);
        assert!(!output.exists());
    }
}
