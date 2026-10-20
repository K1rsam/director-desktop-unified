use super::*;
use reqwest::blocking::multipart::{Form, Part};
use std::io::Read;

const UPLOAD_PROOF_VERSION: &str = "bh-local-upload-readback/v1";

fn upload_url(config: &ProviderConfig, value: &str) -> ProviderResult<(reqwest::Url, i64)> {
    let fail = || {
        ProviderError::new(
            "UPLOAD_URL_INVALID",
            "BH 未返回可核验的完整签名 OSS 地址，已阻止生成。",
        )
    };
    let url = reqwest::Url::parse(value).map_err(|_| fail())?;
    let base = reqwest::Url::parse(&wire::base(config, false)?).map_err(|_| fail())?;
    // Only a deliberately configured loopback fixture may read from its exact
    // origin. A production upload cannot redirect this read to an arbitrary URL.
    let local = matches!(
        base.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    ) && base.scheme() == "http"
        && url.origin() == base.origin();
    let oss = url.scheme() == "https"
        && url.port().is_none_or(|port| port == 443)
        && url.host_str().is_some_and(|host| {
            regex::Regex::new(r"^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]\.oss-[a-z0-9-]+\.aliyuncs\.com$")
                .unwrap()
                .is_match(host)
        });
    if (!local && !oss)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(fail());
    }
    let mut query = std::collections::HashMap::new();
    for (key, value) in url.query_pairs() {
        if query.insert(key.to_string(), value.to_string()).is_some() {
            return Err(fail());
        }
    }
    let expiry = if query.get("x-oss-signature").is_some_and(|v| !v.is_empty()) {
        let stamp = query.get("x-oss-date").ok_or_else(fail)?;
        let start = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ")
            .map_err(|_| fail())?
            .and_utc()
            .timestamp();
        let seconds = query
            .get("x-oss-expires")
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0)
            .ok_or_else(fail)?;
        start.checked_add(seconds).ok_or_else(fail)?
    } else if query.get("Signature").is_some_and(|v| !v.is_empty()) {
        query
            .get("Expires")
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(fail)?
    } else {
        return Err(fail());
    };
    if expiry.saturating_sub(chrono::Utc::now().timestamp()) <= 120 {
        return Err(ProviderError::new(
            "UPLOAD_URL_EXPIRED",
            "BH 参考素材地址已过期或临近过期；请重新准备，尚未提交生成。",
        ));
    }
    Ok((url, expiry))
}

fn verify_uploaded_media(
    config: &ProviderConfig,
    value: &str,
    reference: &MediaReference,
    bytes: u64,
    model: &str,
) -> ProviderResult<Value> {
    let (url, expiry) = upload_url(config, value)?;
    // No provider credential, cookies or referer are attached. Redirects are
    // disabled, so a signed OSS response cannot bounce into an internal service.
    let mut response = wire::client(130)?.get(url).send().map_err(|_| {
        ProviderError::new(
            "UPLOAD_READBACK_FAILED",
            "BH 素材读回暂时失败；尚未提交生成。",
        )
    })?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length != bytes)
    {
        return Err(ProviderError::new(
            "UPLOAD_READBACK_FAILED",
            "BH 素材读回状态或字节数不一致，尚未提交生成。",
        ));
    }
    let mut actual = Vec::new();
    response
        .by_ref()
        .take(bytes + 1)
        .read_to_end(&mut actual)
        .map_err(|_| {
            ProviderError::new(
                "UPLOAD_READBACK_FAILED",
                "BH 素材读回不完整；尚未提交生成。",
            )
        })?;
    if actual.len() as u64 != bytes || hex_hash(&actual) != reference.sha256 {
        return Err(ProviderError::new(
            "UPLOAD_READBACK_MISMATCH",
            "BH 素材读回哈希或字节数不一致，尚未提交生成。",
        ));
    }
    upload_url(config, value)?;
    Ok(
        json!({"version":UPLOAD_PROOF_VERSION,"model":model,"kind":reference.kind,"sha256":reference.sha256,
        "bytes":bytes,"urlSha256":hex_hash(value.as_bytes()),"expiresAt":expiry,"verifiedAt":now(),
        "localReadbackVerified":true,"upstreamReadConfirmed":false}),
    )
}

fn validate_upload_proofs(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
) -> ProviderResult<()> {
    let fail = || {
        ProviderError::new(
            "BH_UPLOAD_PREPARATION_REQUIRED",
            "BH 原准备记录缺少可靠素材读回证明；请重新准备，尚未提交生成。",
        )
    };
    let references = sorted_references(&prepared.request);
    if references.len() != prepared.upload_receipts.len() {
        return Err(fail());
    }
    for (reference, receipt) in references.iter().zip(&prepared.upload_receipts) {
        let url = receipt.url.as_deref().ok_or_else(fail)?;
        let (_, expiry) = upload_url(config, url)?;
        let proof = &receipt.raw;
        if receipt.media_id != reference.media_id
            || receipt.kind != reference.kind
            || receipt.sha256 != reference.sha256
            || receipt.order != reference.order
            || proof["version"] != UPLOAD_PROOF_VERSION
            || proof["model"] != prepared.request.model
            || proof["kind"] != reference.kind
            || proof["sha256"] != reference.sha256
            || proof["urlSha256"] != hex_hash(url.as_bytes())
            || proof["bytes"].as_u64() != fs::metadata(&reference.path).ok().map(|m| m.len())
            || proof["localReadbackVerified"] != true
            || proof["expiresAt"].as_i64() != Some(expiry)
        {
            return Err(fail());
        }
    }
    Ok(())
}

const ADAPTED: &[&str] = &[
    "bh2.0-mini-480p",
    "bh2.0-mini-720p",
    "bh2.0-480p",
    "bh2.0-720p",
    "bh2.0-1080p",
    "bh2.0-fast-480p",
    "bh2.0-fast-720p",
    "bh2.04K",
    "rd2.5-720p",
    "SD2.5-480p",
    "SD2.5-720p",
    "SD2.5-1080p",
    "sd2.5-720p-ch2",
    "sd2.5-720p-ch3",
    "quanneng2.0",
    "quanneng2.0-9tu",
    "sd2.0-cf-1080p",
    "seedance-2.0-480p",
    "seedance-2.0-720p",
    "gemini-3-pro-image-preview",
    "gemini-3.1-flash-image-preview",
    "image2k4k",
    "image4k",
    "image2-high",
    "gpt-image-2",
];

// Explicit image execution manifests from the SHUO BH catalogue template.
// A model's name alone must not opt an unknown protocol into this mapping.
#[derive(Clone, Copy, PartialEq)]
enum ImageProtocol {
    Resolution,
    HighResolution,
    QualityLevel,
}
fn image_protocol(id: &str) -> Option<ImageProtocol> {
    match id {
        "gemini-3-pro-image-preview"
        | "gemini-3.1-flash-image-preview"
        | "image2k4k"
        | "image4k" => Some(ImageProtocol::Resolution),
        "image2-high" => Some(ImageProtocol::HighResolution),
        "gpt-image-2" => Some(ImageProtocol::QualityLevel),
        _ => None,
    }
}
const IMAGE_RATIOS: &[&str] = &[
    "1:1", "5:4", "4:5", "4:3", "3:4", "3:2", "2:3", "16:9", "9:16", "21:9",
];
const IMAGE_QUALITY_LEVELS: &[&str] = &["low", "medium", "high"];
fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|xs| {
            xs.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
fn number(v: Option<&Value>) -> Option<u32> {
    v.and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

pub(super) fn parse_catalog(_config: &ProviderConfig, raw: Value) -> ProviderResult<Capabilities> {
    let version = raw
        .get("catalog_version")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProviderError::new("INVALID_CATALOG", "模型目录缺少版本。"))?;
    let rows = raw
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::new("INVALID_CATALOG", "模型目录缺少列表。"))?;
    if raw.get("ok").and_then(Value::as_bool) != Some(true)
        || rows.len() > 2000
        || raw
            .get("count")
            .and_then(Value::as_u64)
            .is_some_and(|n| n != rows.len() as u64)
    {
        return Err(ProviderError::new(
            "INVALID_CATALOG",
            "模型目录结构或数量无效。",
        ));
    }
    let mut flat = Vec::new();
    for row in rows {
        if let Some(variants) = row.get("variants").and_then(Value::as_array) {
            if variants.is_empty()
                || variants.len() > 2000
                || row.get("id").is_some()
                || row.get("price").is_some()
                || row.get("billing").and_then(Value::as_str) != Some("per_second")
                || !row
                    .get("family")
                    .and_then(Value::as_str)
                    .is_some_and(|x| !x.is_empty())
            {
                return Err(ProviderError::new("INVALID_CATALOG", "分组模型目录无效。"));
            }
            for variant in variants {
                let mut value = row
                    .as_object()
                    .ok_or_else(|| ProviderError::new("INVALID_CATALOG", "目录格式无效。"))?
                    .clone();
                value.remove("variants");
                value.remove("family");
                let values = variant
                    .as_object()
                    .ok_or_else(|| ProviderError::new("INVALID_CATALOG", "变体格式无效。"))?;
                if [
                    "price",
                    "billing",
                    "unit",
                    "kind",
                    "category",
                    "displayName",
                    "family",
                ]
                .iter()
                .any(|key| values.contains_key(*key))
                {
                    return Err(ProviderError::new(
                        "INVALID_CATALOG",
                        "分组变体覆盖了计费或模型分类，目录需重新适配。",
                    ));
                }
                value.extend(values.clone());
                if let Some(price) = variant.get("per_second") {
                    value.insert("price".into(), json!({"per_second":price}));
                }
                if let Some(resolution) = variant.get("resolution") {
                    value.insert("resolutions".into(), json!([resolution]));
                }
                flat.push(Value::Object(value));
            }
        } else {
            flat.push(row.clone());
        }
    }
    let mut ids = std::collections::HashSet::new();
    let mut models = Vec::new();
    for row in flat {
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.trim() == *id)
            .ok_or_else(|| ProviderError::new("INVALID_CATALOG", "模型 ID 无效。"))?;
        if !ids.insert(id.to_string()) {
            return Err(ProviderError::new("INVALID_CATALOG", "模型 ID 重复。"));
        }
        if !matches!(
            row.get("status").and_then(Value::as_str),
            Some("active" | "new")
        ) {
            continue;
        }
        let kind = row
            .get("kind")
            .or_else(|| row.get("category"))
            .and_then(Value::as_str)
            .unwrap_or(if id.contains("image") || id.starts_with("gemini") {
                "image"
            } else {
                "video"
            });
        if !matches!(kind, "image" | "video") {
            continue;
        }
        let reference = row.get("reference");
        let refs = |name| {
            reference
                .and_then(|r| r.get(name))
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize
        };
        let durations = row.get("durations");
        let duration_list: Vec<u32> = durations
            .and_then(Value::as_array)
            .map(|xs| xs.iter().filter_map(|v| number(Some(v))).collect())
            .unwrap_or_else(|| {
                row.get("allowed_durations")
                    .and_then(Value::as_array)
                    .map(|xs| xs.iter().filter_map(|v| number(Some(v))).collect())
                    .unwrap_or_default()
            });
        let min = number(durations.and_then(|v| v.get("min")))
            .or_else(|| duration_list.iter().min().copied());
        let max = number(durations.and_then(|v| v.get("max")))
            .or_else(|| duration_list.iter().max().copied());
        let billing = row
            .get("billing")
            .and_then(Value::as_str)
            .map(str::to_string);
        let unknown_pricing = row.as_object().is_some_and(|map| {
            map.keys().any(|key| {
                let name = key.to_ascii_lowercase();
                ["price", "billing", "multiplier", "surcharge", "tier"]
                    .iter()
                    .any(|part| name.contains(part))
                    && !["price", "billing", "video_price_multiplier", "per_second"]
                        .contains(&name.as_str())
            })
        }) || row
            .get("price")
            .and_then(Value::as_object)
            .is_some_and(|map| map.len() != 1);
        let price = if unknown_pricing {
            None
        } else {
            row.get("price")
                .and_then(|p| p.get("amount").or_else(|| p.get("per_second")))
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite() && *x >= 0.)
        };
        let image_protocol = (kind == "image").then(|| image_protocol(id)).flatten();
        let declared_ratios = row
            .get("allowed_sizes")
            .or_else(|| row.get("ratios"))
            .or_else(|| row.get("aspect_ratios"));
        let ratios = if image_protocol.is_some() && declared_ratios.is_none() {
            // BH's live image rows omit ratios; the six known execution
            // manifests declare these options. Explicit server limits win.
            IMAGE_RATIOS.iter().map(|value| (*value).into()).collect()
        } else {
            strings(declared_ratios)
        };
        // GPT Image 2 has no pixel-resolution control in its BH manifest.
        // Project its actual quality selector through the common output-choice
        // field, retaining the original catalogue row and labelling the mapping.
        let resolutions = if image_protocol == Some(ImageProtocol::QualityLevel) {
            IMAGE_QUALITY_LEVELS
                .iter()
                .map(|value| (*value).into())
                .collect()
        } else {
            strings(row.get("resolutions"))
        };
        // A directory ID or configuration allowlist cannot establish parameter compatibility.
        // New protocols require an explicit code mapping and wire tests before enabling.
        let protocol_known = if kind == "image" {
            image_protocol.is_some()
        } else {
            ADAPTED.contains(&id) && self::image_protocol(id).is_none()
        };
        let complete = !ratios.is_empty()
            && !resolutions.is_empty()
            && reference.is_some()
            && (image_protocol != Some(ImageProtocol::QualityLevel)
                || row
                    .get("resolutions")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty))
            && (kind == "image" || min.is_some() && max.is_some());
        let multiplier = row
            .get("video_price_multiplier")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && *x > 0. && *x <= 100.)
            .or_else(|| {
                if row.get("video_price_multiplier").is_some() {
                    None
                } else if id.starts_with("bh2.0-") || id == "bh2.04K" {
                    Some(1.8)
                } else if id == "seedance-2.0-480p" {
                    Some(1.)
                } else {
                    None
                }
            });
        let mut projected_raw = row.clone();
        if let Some(protocol) = image_protocol {
            projected_raw["desktopParameterContract"] = json!({
                "version":"bh-image-shuo/v1",
                "source":"shuo-bh-execution-manifest",
                "outputParameter":if protocol == ImageProtocol::QualityLevel {"quality_level"} else {"resolution"},
                "defaultOutput":if protocol == ImageProtocol::QualityLevel {json!("medium")} else {Value::Null},
                "ratiosSource":if declared_ratios.is_none() {"execution-manifest"} else {"live_catalog"}
            });
        }
        models.push(ModelCapability {
            id: id.into(),
            kind: kind.into(),
            adapted: protocol_known && complete,
            min_duration: min,
            max_duration: max,
            allowed_durations: duration_list,
            ratios,
            resolutions,
            images: refs("images"),
            videos: refs("videos"),
            audios: refs("audios"),
            total_references: number(reference.and_then(|r| r.get("total"))).map(|n| n as usize),
            required_images: number(row.get("required_images")).unwrap_or(0) as usize,
            price,
            billing,
            currency: row.get("unit").and_then(Value::as_str).map(str::to_string),
            video_price_multiplier: multiplier,
            raw: projected_raw,
        });
    }
    if models.len() > 2000 {
        return Err(ProviderError::new("INVALID_CATALOG", "模型目录超出容量。"));
    }
    Ok(Capabilities {
        provider: "bh".into(),
        version: version.into(),
        source: "live_catalog".into(),
        checked_at: now(),
        models,
        can_cancel: false,
        can_reconcile_unknown: false,
        raw,
    })
}

pub(super) fn capabilities(config: &ProviderConfig) -> ProviderResult<Capabilities> {
    let response = wire::client(30)?
        .get(format!("{}/models/catalog", wire::base(config, false)?))
        .bearer_auth(wire::credential(config, false)?)
        .send()
        .map_err(|_| ProviderError::new("CATALOG_OFFLINE", "模型目录暂时不可用。"))?;
    parse_catalog(config, wire::read_json(response, false)?)
}

pub(super) fn prepare(
    config: &ProviderConfig,
    request: &GenerationRequest,
) -> ProviderResult<PreparedRequest> {
    let caps = capabilities(config)?;
    let model = caps
        .models
        .iter()
        .find(|m| m.id == request.model)
        .ok_or_else(|| ProviderError::new("MODEL_UNAVAILABLE", "当前目录没有此模型。"))?;
    check_model(model, request)?;
    for (key, value) in request.options.as_object().unwrap() {
        if !(key == "generateAudio" && request.kind == "video" && value.is_boolean())
            && !(key == "count" && request.kind == "image" && value.as_u64() == Some(1))
        {
            return Err(ProviderError::new(
                "UNSUPPORTED_OPTION",
                format!("BH 未适配参数 {key}，未提交生成。"),
            ));
        }
    }
    let frame_count = |names: &[&str]| {
        request
            .references
            .iter()
            .filter(|r| names.contains(&r.usage.as_str()))
            .count()
    };
    let first_count = frame_count(&["start_frame", "first_frame"]);
    let last_count = frame_count(&["end_frame", "last_frame"]);
    if first_count > 1
        || last_count > 1
        || last_count > 0 && first_count == 0
        || first_count + last_count > 0
            && request.references.iter().any(|r| {
                r.kind != "image"
                    || !["start_frame", "first_frame", "end_frame", "last_frame"]
                        .contains(&r.usage.as_str())
            })
    {
        return Err(ProviderError::new(
            "FRAME_MODE",
            "首尾帧不能与普通图片、音频或视频参考混用。",
        ));
    }
    if first_count + last_count > 0
        && model
            .raw
            .get("supports_first_last_frame")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(ProviderError::new(
            "FRAME_CAPABILITY",
            "当前目录未明确此模型首尾帧能力，尚未上传素材。",
        ));
    }
    let client = wire::client(config.timeout_seconds)?;
    let base = wire::base(config, false)?;
    let key = wire::credential(config, false)?;
    let mut receipts = Vec::new();
    let mut images = Vec::new();
    let mut videos = Vec::new();
    let mut audios = Vec::new();
    let mut first = Vec::new();
    let mut last = Vec::new();
    for reference in sorted_references(request) {
        let bytes = fs::read(&reference.path)
            .map_err(|_| ProviderError::new("MEDIA_READ_FAILED", "素材读取失败。"))?;
        if hex_hash(&bytes) != reference.sha256 {
            return Err(ProviderError::new(
                "MEDIA_CHANGED",
                "上传前素材哈希已变化。",
            ));
        }
        let ext = reference
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let (limit, mime) = match (reference.kind.as_str(), ext.as_str()) {
            ("image", "jpg" | "jpeg") => (10 * 1024 * 1024, "image/jpeg"),
            ("image", "png") => (10 * 1024 * 1024, "image/png"),
            ("image", "webp") => (10 * 1024 * 1024, "image/webp"),
            ("audio", "mp3") => (50 * 1024 * 1024, "audio/mpeg"),
            ("audio", "wav") => (50 * 1024 * 1024, "audio/wav"),
            ("video", "mp4") => (60 * 1024 * 1024, "video/mp4"),
            _ => {
                return Err(ProviderError::new(
                    "MEDIA_FORMAT",
                    "BH 不支持此参考素材格式。",
                ))
            }
        };
        if bytes.len() > limit {
            return Err(ProviderError::new(
                "MEDIA_SIZE",
                "参考素材超过 BH 单文件大小限制。",
            ));
        }
        let bytes_len = bytes.len() as u64;
        let part = Part::bytes(bytes)
            .file_name(format!("{}.{}", reference.sha256, ext))
            .mime_str(mime)
            .map_err(|_| ProviderError::new("MEDIA_FORMAT", "素材格式无效。"))?;
        let response = client
            .post(format!("{base}/assets/uploads"))
            .bearer_auth(&key)
            .multipart(
                Form::new()
                    .part("file", part)
                    .text("model", request.model.clone()),
            )
            .send()
            .map_err(|_| {
                ProviderError::new("UPLOAD_FAILED", "素材上传结果未确认；尚未提交生成。")
            })?;
        let body = wire::read_json(response, false)?;
        let url = body
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::new("UPLOAD_NO_URL", "上传回执未返回完整 URL。"))?;
        if body
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != reference.kind)
            || body
                .get("bytes")
                .is_some_and(|value| value.as_u64() != Some(bytes_len))
        {
            return Err(ProviderError::new(
                "UPLOAD_URL_INVALID",
                "BH 未返回匹配素材的完整签名 OSS 地址，已阻止生成。",
            ));
        }
        let proof = verify_uploaded_media(config, url, &reference, bytes_len, &request.model)?;
        match reference.kind.as_str() {
            "image" => match reference.usage.as_str() {
                "start_frame" | "first_frame" => first.push(url.to_string()),
                "end_frame" | "last_frame" => last.push(url.to_string()),
                _ => images.push(url.to_string()),
            },
            "video" => videos.push(url.to_string()),
            "audio" => audios.push(url.to_string()),
            _ => unreachable!(),
        }
        receipts.push(UploadReceipt {
            media_id: reference.media_id,
            sha256: reference.sha256,
            order: reference.order,
            kind: reference.kind,
            url: Some(url.into()),
            file_id: None,
            uploaded_at: now(),
            upstream_read_confirmed: false,
            raw: proof,
        });
    }
    if first.len() > 1
        || last.len() > 1
        || (!last.is_empty() && first.is_empty())
        || ((!first.is_empty() || !last.is_empty())
            && (!images.is_empty() || !videos.is_empty() || !audios.is_empty()))
    {
        return Err(ProviderError::new(
            "FRAME_MODE",
            "首尾帧不能与普通图片、音频或视频参考混用。",
        ));
    }
    let resolution = model
        .resolutions
        .iter()
        .find(|value| value.eq_ignore_ascii_case(&request.resolution))
        .unwrap();
    let mut body = json!({"model":request.model,"prompt":request.prompt,"n":1});
    let obj = body.as_object_mut().unwrap();
    if request.kind == "video" {
        obj.insert("resolution".into(), json!(resolution));
        obj.insert("duration".into(), json!(request.duration));
        obj.insert("ratio".into(), json!(request.ratio));
        obj.insert("reference_videos".into(), json!(videos));
        obj.insert("reference_audios".into(), json!(audios));
        obj.insert(
            "generate_audio".into(),
            json!(request
                .options
                .get("generateAudio")
                .and_then(Value::as_bool)
                .unwrap_or(true)),
        );
        if !first.is_empty() {
            obj.insert("start_frame".into(), json!(first));
        }
        if !last.is_empty() {
            obj.insert("end_frame".into(), json!(last));
        }
    } else {
        obj.insert("size".into(), json!(request.ratio));
        obj.insert("response_format".into(), json!("url"));
        match image_protocol(&request.model) {
            Some(ImageProtocol::QualityLevel) => {
                obj.insert("quality_level".into(), json!(resolution));
            }
            Some(protocol) => {
                obj.insert("resolution".into(), json!(resolution));
                obj.insert("quality".into(), json!(resolution));
                if protocol == ImageProtocol::HighResolution {
                    obj.insert("quality_level".into(), json!("high"));
                }
            }
            None => unreachable!("check_model rejects unknown image protocols"),
        }
    }
    if request.kind == "video" || !images.is_empty() {
        obj.insert("images".into(), json!(images));
    }
    let mut frozen = request.clone();
    frozen.references = sorted_references(request);
    Ok(PreparedRequest {
        provider: "bh".into(),
        request: frozen,
        capability_version: caps.version,
        upload_receipts: receipts,
        wire: body,
        estimated_cost: estimate(model, request),
        prepared_at: now(),
    })
}

pub(super) fn submit(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
) -> ProviderResult<SubmitReceipt> {
    let caps = capabilities(config)?;
    if caps.version != prepared.capability_version {
        return Err(ProviderError::new(
            "CAPABILITY_CHANGED",
            "BH 模型目录已变化，请重新准备关联单元。",
        ));
    }
    let model = caps
        .models
        .iter()
        .find(|m| m.id == prepared.request.model)
        .ok_or_else(|| ProviderError::new("MODEL_UNAVAILABLE", "当前目录已不提供此模型。"))?;
    check_model(model, &prepared.request)?;
    validate_upload_proofs(config, prepared)?;
    if shutting_down() {
        return Err(ProviderError::new(
            "CLIENT_CLOSING",
            "客户端正在关闭，未发送生成请求。",
        ));
    }
    let endpoint = if prepared.request.kind == "video" {
        "video/generations"
    } else {
        "images/generations"
    };
    let timeout = if prepared.request.kind == "image" {
        config.timeout_seconds.max(900)
    } else {
        config.timeout_seconds
    };
    let response = wire::client(timeout)?
        .post(format!("{}/{endpoint}", wire::base(config, false)?))
        .bearer_auth(wire::credential(config, false)?)
        .json(&prepared.wire)
        .send()
        .map_err(|_| ProviderError::uncertain("生成提交回执未确认；不能自动重新生成。"))?;
    let body = wire::read_json(response, true)?;
    if prepared.request.kind == "image" {
        let urls: Vec<String> = body
            .get("data")
            .and_then(Value::as_array)
            .map(|xs| {
                xs.iter()
                    .filter_map(|x| x.get("url").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if urls.len() != 1 {
            return Err(ProviderError::uncertain(
                "图片结果数量或格式未确认；请核对渠道记录。",
            ));
        }
        return Ok(SubmitReceipt {
            task_id: format!("sync:{}", prepared.request.request_id),
            status: "succeeded".into(),
            result_urls: urls,
            raw: body,
        });
    }
    let id = body
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .ok_or_else(|| ProviderError::uncertain("渠道受理回执缺少任务 ID；不能自动重发。"))?
        .to_string();
    Ok(SubmitReceipt {
        task_id: id,
        status: "processing".into(),
        result_urls: Vec::new(),
        raw: body,
    })
}
pub(super) fn query(config: &ProviderConfig, task_id: &str) -> ProviderResult<QueryResult> {
    if task_id.starts_with("sync:") {
        return Err(ProviderError::new(
            "SYNCHRONOUS_RESULT",
            "同步图片结果请使用已持久化回执中的下载地址。",
        ));
    }
    let response = wire::client(30)?
        .get(format!(
            "{}/video/generations/{task_id}",
            wire::base(config, false)?
        ))
        .bearer_auth(wire::credential(config, false)?)
        .send()
        .map_err(|_| ProviderError::new("QUERY_OFFLINE", "原任务查询暂时失败。"))?;
    let body = wire::read_json(response, false)?;
    if body
        .get("task_id")
        .and_then(Value::as_str)
        .is_some_and(|x| x != task_id)
    {
        return Err(ProviderError::new(
            "TASK_MISMATCH",
            "渠道返回了其他任务的结果。",
        ));
    }
    let status = body
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let status = if status == "success" {
        "succeeded"
    } else {
        status
    };
    let urls = if status == "succeeded" {
        body.get("result_url")
            .or_else(|| body.get("url"))
            .and_then(Value::as_str)
            .map(|x| vec![x.into()])
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(QueryResult {
        task_id: task_id.into(),
        status: if ["processing", "succeeded", "failed"].contains(&status) {
            status.into()
        } else {
            "unknown".into()
        },
        result_urls: urls,
        error: wire::safe_message(&body),
        actual_cost: body.get("actual_cost").and_then(Value::as_f64),
        raw: body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ProviderConfig {
        ProviderConfig {
            kind: "bh".into(),
            endpoint: None,
            credential_env: None,
            cli_path: None,
            bridge_token_env: None,
            chrome_binding: None,
            browser_transport: None,
            timeout_seconds: 10,
            adapted_models: Vec::new(),
        }
    }
    #[test]
    fn grouped_catalog_and_unknown_models_are_explicit() {
        let raw = json!({"ok":true,"catalog_version":"v1","count":2,"models":[{"family":"test","status":"active","kind":"video","billing":"per_second","unit":"CNY","durations":{"min":4,"max":15},"allowed_sizes":["16:9"],"reference":{"images":9,"videos":3,"audios":3},"variants":[{"id":"bh2.0-720p","resolution":"720P","per_second":0.5}]},{"id":"new-model","status":"active","kind":"video","resolutions":["720P"],"allowed_sizes":["16:9"],"durations":{"min":4,"max":15},"reference":{"images":9}}]});
        let caps = parse_catalog(&config(), raw).unwrap();
        assert_eq!(caps.models[0].price, Some(0.5));
        assert!(caps.models[0].adapted);
        assert!(!caps.models[1].adapted);
    }
    #[test]
    fn duplicate_models_fail_closed() {
        let raw = json!({"ok":true,"catalog_version":"v1","models":[{"id":"same"},{"id":"same"}]});
        assert!(parse_catalog(&config(), raw).is_err());
    }
}

#[cfg(test)]
#[path = "bh_image_contract_tests.rs"]
mod image_contract_tests;
