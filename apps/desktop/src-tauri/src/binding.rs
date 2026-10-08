use crate::store::{hash_file, id, s, Result, Store};
use serde_json::{json, Value};

/// A positive whole-second carrier, allowing only the same 1e-6 numerical
/// tolerance as the original storyboard capacity checker. This does not ceil
/// a real fractional duration, change shots/voice windows or add screen time.
pub(crate) fn normalized_integer_duration(seconds: f64) -> Option<u32> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let integer = seconds.round();
    (integer >= 1.0 && integer <= u32::MAX as f64 && (seconds - integer).abs() <= 1e-6)
        .then_some(integer as u32)
}

/// Names are display text only. References are resolved by immutable identity and scope.
pub fn bind(store: &Store, unit: &Value, provider: &str, model: &str) -> Result<Value> {
    if s(unit, "prompt").trim().is_empty() {
        return Err("单元提示词为空".into());
    }
    let mut refs = unit["assetRefs"]
        .as_array()
        .ok_or("分镜没有结构化assetRefs，请先完成旧分镜资产映射")?
        .clone();
    refs.sort_by_key(|r| r.get("order").and_then(Value::as_u64).unwrap_or(0));
    let mut media = Vec::new();
    let mut inline_references = Vec::new();
    let mut problems = Vec::new();
    if unit["continuityStale"] == true {
        problems.push(json!({"code":"CONTINUITY_REVISION_REQUIRED","message":"前置单元已修改，请先更新本单元的连续状态与提示词"}));
    }
    let duration = unit["duration"].as_f64().unwrap_or(0.0);
    let integer_duration = normalized_integer_duration(duration);
    if integer_duration.is_none() {
        problems.push(json!({"code":"DURATION_REVISION_REQUIRED","message":"首版视频渠道要求正整秒载体；请修订单元时长，不自动取整或填空"}));
    }
    let source = store.source()?;
    if source.is_null() {
        problems.push(
            json!({"code":"MISSING_SOURCE","message":"请先导入源文档，为单元保存明确来源版本"}),
        );
    } else {
        if unit["sourceVersion"].as_i64().is_some_and(|version|Some(version)!=source["version"].as_i64()) {
            problems.push(json!({"code":"SOURCE_VERSION_MISMATCH","message":"单元仍绑定旧原文版本，须通过当前原文完整校验后重新准备。"}));
        }
        let episode = s(unit, "episodeId");
        let scene = s(unit, "sceneId");
        if !source["episodes"]
            .as_array()
            .map(|es| es.iter().any(|e| s(e, "id") == episode))
            .unwrap_or(false)
        {
            problems
                .push(json!({"code":"INVALID_EPISODE","message":"单元所属集没有映射到当前原文"}));
        }
        if !source["scenes"]
            .as_array()
            .map(|sc| {
                sc.iter()
                    .any(|sc| s(sc, "id") == scene && s(sc, "episodeId") == episode)
            })
            .unwrap_or(false)
        {
            problems.push(json!({"code":"INVALID_SCENE","message":"单元所属场没有映射到当前原文"}));
        }
        let source_refs = unit["sourceRefs"].as_array();
        if source_refs.map(|r| r.is_empty()).unwrap_or(true) {
            problems
                .push(json!({"code":"SOURCE_REFS_REQUIRED","message":"单元缺少当前正文来源引用"}));
        } else if source_refs.unwrap().iter().any(|id| {
            !source["rows"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .any(|row| {
                    row["id"] == *id
                        && s(row, "episodeId") == episode
                        && s(row, "sceneId") == scene
                        && s(row, "kind") == "body"
                })
        }) {
            problems.push(
                json!({"code":"INVALID_SOURCE_REF","message":"单元来源已失效或越出所属集场"}),
            );
        }
    }
    if unit["needsAssetMapping"].as_bool() == Some(true) {
        problems.push(json!({"code":"LEGACY_ASSET_MAPPING","message":"旧分镜需要明确确认assetId、variantId、用途与顺序"}));
    }
    if let Some(issues) = unit["capacityIssues"].as_array() {
        if !issues.is_empty() {
            problems.push(json!({"code":"CAPACITY_REVISION_REQUIRED","message":"该单元仍有时长或素材容量问题，请修订关联单元","details":issues}));
        }
    }
    for (order, r) in refs.iter().enumerate() {
        let aid = s(r, "assetId");
        if aid.is_empty() {
            problems.push(json!({"code":"MISSING_ASSET_ID","order":order,"message":"引用缺少assetId，不能按姓名自动配图"}));
            continue;
        }
        let a = match store.get("assets", aid) {
            Ok(a) => a,
            Err(e) => {
                problems.push(json!({"code":"MISSING_ASSET","assetId":aid,"message":e}));
                continue;
            }
        };
        if crate::store::asset_deleted(&a) {
            problems.push(json!({"code":"ASSET_DELETED","assetId":aid,"message":"此资产已删除，请在单元配图中替换，或到图片资产恢复"}));
            continue;
        }
        if a["scope"]["kind"] == "UNRESOLVED" {
            problems.push(json!({"code":"ASSET_SCOPE_UNRESOLVED","assetId":aid,"message":"此形象的适用集场尚未明确，请先处理制作基础中的相关问题"}));
            continue;
        }
        if a["scope"]["kind"] == "SCENES"
            && !a["scope"]["scenes"].as_array().is_some_and(|scenes| {
                scenes.iter().any(|sc| {
                    sc["episodeId"] == unit["episodeId"] && sc["sceneId"] == unit["sceneId"]
                })
            })
        {
            problems.push(
                json!({"code":"OUT_OF_SCOPE","assetId":aid,"message":"形象不适用于本单元的集场"}),
            );
            continue;
        }
        if a["needsReview"].as_bool() == Some(true) {
            problems.push(json!({"code":"ASSET_REVIEW_REQUIRED","assetId":aid,"message":"导入形象身份及适用范围尚未核对"}));
            continue;
        }
        let variant = if s(r, "variantId").is_empty() {
            s(&a, "variantId")
        } else {
            s(r, "variantId")
        };
        if variant != s(&a, "variantId") {
            problems.push(json!({"code":"VARIANT_MISMATCH","assetId":aid,"variantId":variant,"message":"指定形象版本不存在"}));
            continue;
        }
        if let Some(scope) = a["applicability"]["episodes"].as_array() {
            if !scope.is_empty() && !scope.iter().any(|e| e == &unit["episodeId"]) {
                problems.push(
                    json!({"code":"OUT_OF_SCOPE","assetId":aid,"message":"形象不适用于本集"}),
                );
                continue;
            }
        }
        if let Some(scope) = a["applicability"]["scenes"].as_array() {
            if !scope.is_empty() && !scope.iter().any(|e| e == &unit["sceneId"]) {
                problems.push(
                    json!({"code":"OUT_OF_SCOPE","assetId":aid,"message":"形象不适用于本场"}),
                );
                continue;
            }
        }
        // INLINE is a semantic appearance, not a request for a new image. Resolve
        // its actual material ancestor in Rust while retaining its scope/version.
        if a.pointer("/preparation/mode").and_then(Value::as_str) == Some("AUDIO") {
            continue;
        }
        // Source-owned text interfaces are INLINE props without an image
        // ancestor. They remain explicit semantic references in the snapshot;
        // their complete instructions are compiled as text, not a phantom image.
        if inline_text_material(store, &a)? {
            if ["mediaId", "imageAssetId", "imageVariantId"]
                .iter()
                .any(|field| !s(r, field).is_empty())
            {
                problems.push(json!({"code":"MATERIAL_SOURCE_MISMATCH","assetId":aid,"message":"文字素材不使用图片槽，请更新已指定的配图"}));
                continue;
            }
            inline_references.push(json!({"assetId":aid,"variantId":variant,"assetVersion":a["version"],"kind":"inline-text","name":a["name"],"description":a["description"],"preparation":a["preparation"],"sourceRefs":a["sourceRefs"],"usage":r.get("use").or(r.get("usage")).cloned().unwrap_or(json!("prop"))}));
            continue;
        }
        let image_asset = match image_material_asset(store, &a) {
            Ok(value) => value,
            Err(message) => {
                problems.push(
                    json!({"code":"MATERIAL_SOURCE_UNRESOLVED","assetId":aid,"message":message}),
                );
                continue;
            }
        };
        let image_aid = s(&image_asset, "id");
        let image_variant = s(&image_asset, "variantId");
        if (!s(r, "imageAssetId").is_empty() && s(r, "imageAssetId") != image_aid)
            || (!s(r, "imageVariantId").is_empty() && s(r, "imageVariantId") != image_variant)
        {
            problems.push(json!({"code":"MATERIAL_SOURCE_MISMATCH","assetId":aid,"message":"参考图身份与当前素材来源不一致，请更新配图"}));
            continue;
        }
        let adopted = store.adopted(image_aid, image_variant)?;
        let mid = if !s(r, "mediaId").is_empty() {
            if adopted.as_deref() != Some(s(r, "mediaId")) {
                problems.push(json!({"code":"STALE_ADOPTION","assetId":aid,"message":"显式图片引用已不是当前采用版本"}));
                continue;
            }
            s(r, "mediaId").to_string()
        } else {
            match adopted {
                Some(m) => m,
                None => {
                    problems.push(json!({"code":"MISSING_IMAGE","assetId":aid,"variantId":variant,"message":"没有已采用图片"}));
                    continue;
                }
            }
        };
        let m = store.get("media", &mid)?;
        let p = match store.verify_media(&m) {
            Ok(p) => p,
            Err(e) => {
                problems.push(json!({"code":"INVALID_MEDIA","mediaId":mid,"message":e}));
                continue;
            }
        };
        let semantic = json!({"assetId":aid,"variantId":variant,"assetVersion":a["version"]});
        if let Some(existing) = media
            .iter_mut()
            .find(|item: &&mut Value| item["mediaId"] == mid && item["kind"] == "image")
        {
            existing["semanticBindings"]
                .as_array_mut()
                .ok_or("素材语义绑定缺失")?
                .push(semantic);
        } else {
            media.push(json!({"mediaId":mid,"assetId":image_aid,"assetVersion":image_asset["version"],"variantId":image_variant,"semanticBindings":[semantic],"kind":"image","path":p,"relativePath":m["relativePath"],"sha256":m["sha256"],"order":media.len(),"usage":r.get("role").or(r.get("usage")).unwrap_or(&json!("reference")),"assetName":a["name"]}));
        }
    }
    if let Some(extras) = unit["mediaRefs"].as_array() {
        let mut ordered = extras.clone();
        ordered.sort_by_key(|r| r["order"].as_u64().unwrap_or(0));
        for r in &ordered {
            let m = store.get("media", s(r, "mediaId"))?;
            let p = store.verify_media(&m)?;
            if ["audio", "video"].contains(&s(&m, "kind"))
                && m["metadataVerified"].as_bool() != Some(true)
            {
                problems.push(json!({"code":"MEDIA_METADATA_REQUIRED","mediaId":m["id"],"message":"音视频时长尚未可靠读取，请检查媒体探测工具或重新导入"}));
            }
            media.push(json!({"mediaId":m["id"],"kind":m["kind"],"path":p,"relativePath":m["relativePath"],"sha256":m["sha256"],"order":media.len(),"usage":r.get("usage").or(r.get("role")).unwrap_or(&json!("reference")),"durationSeconds":m.get("durationSeconds")}));
        }
    }
    let prompt = compile_bound_prompt(unit, &store.values("assets")?, &media, &inline_references)?;
    if prompt.contains("{{asset:") {
        problems.push(json!({"code":"UNBOUND_PLACEHOLDER","message":"提示词含未绑定资产占位符"}));
    }
    if provider != "bh" && unit.get("generateAudio").and_then(Value::as_bool) == Some(false) {
        problems.push(json!({"code":"AUDIO_OPTION_UNSUPPORTED","message":"所选渠道尚未核实静音生成参数，请调整单元设置或渠道"}));
    }
    // A source @图片N must agree with the compiled order. We do not silently renumber it.
    let image_order = media.iter().filter(|r| s(r, "kind") == "image").count();
    let markers = regex::Regex::new(r"@图片(\d+)").unwrap();
    if markers
        .captures_iter(&prompt)
        .any(|c| c[1].parse::<usize>().unwrap_or(usize::MAX) > image_order)
    {
        problems.push(json!({"code":"REFERENCE_ORDER","message":"提示词图片编号超出实际绑定顺序"}));
    }
    let options = if provider == "bh" {
        json!({"generateAudio":unit.get("generateAudio").cloned().unwrap_or(json!(true))})
    } else {
        json!({})
    };
    let wire_duration = integer_duration
        .map(|duration| json!(duration))
        .unwrap_or_else(|| unit["duration"].clone());
    Ok(
        json!({"requestId":id(),"kind":"video","provider":provider,"model":model,"unitId":unit["id"],"unitVersion":unit["version"],"episodeId":unit["episodeId"],"sourceVersion":store.source()?["version"],"sourcePrompt":unit["prompt"],"sourcePromptSha256":crate::pipeline::digest(s(unit,"prompt")),"prompt":prompt,"duration":wire_duration,"ratio":store.settings()?["ratio"],"resolution":store.settings()?.get("videoResolution").cloned().unwrap_or(json!("720p")),"references":media,"inlineReferences":inline_references,"problems":problems,"ruleSnapshot":crate::pipeline::rule_snapshot("unit-video-production")?,"options":options}),
    )
}

/// The same pure compiler serves preparation and metadata-only readiness proofs.
/// It does not open media files, resolve a name to an identity, or alter source text.
pub(crate) fn compile_bound_prompt(
    unit: &Value,
    assets: &[Value],
    references: &[Value],
    inline_references: &[Value],
) -> Result<String> {
    let mut refs = unit["assetRefs"]
        .as_array()
        .ok_or("单元资产引用缺失")?
        .clone();
    refs.sort_by_key(|r| r.get("order").and_then(Value::as_u64).unwrap_or(0));
    let mut prompt = if s(unit, "storyboardCreativeProfile") == "original-production-contract/v1" {
        let mut bindings = Vec::new();
        for reference in &refs {
            let Some(asset) = assets
                .iter()
                .find(|a| s(a, "id") == s(reference, "assetId") && !crate::store::asset_deleted(a))
            else {
                continue;
            };
            if asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("AUDIO") {
                continue;
            }
            bindings.push(json!({"assetId":asset["id"],"variantId":asset["variantId"],"name":asset["name"],
                "use":reference.get("use").or(reference.get("usage")).or(reference.get("role")).cloned().unwrap_or(json!("reference"))}));
        }
        crate::pipeline::compile_reference_template(s(unit, "prompt"), &bindings)?
    } else {
        s(unit, "prompt").to_owned()
    };
    for reference in inline_references {
        let placeholder = format!(
            "{{{{asset:{}:{}}}}}",
            s(reference, "assetId"),
            s(reference, "variantId")
        );
        let mut text = format!("【文字素材：{}", s(reference, "name"));
        for detail in [
            s(reference, "description"),
            s(&reference["preparation"], "note"),
        ] {
            if !detail.trim().is_empty() {
                text.push_str("；");
                text.push_str(detail);
            }
        }
        text.push('】');
        prompt = prompt.replace(&placeholder, &text);
    }
    // Compile explicit identity placeholders, never infer a name-to-image alias.
    let mut image_order = 0;
    let mut video_order = 0;
    let mut audio_order = 0;
    for r in references {
        let marker = match s(r, "kind") {
            "image" => {
                image_order += 1;
                format!("@图片{image_order}")
            }
            "video" => {
                video_order += 1;
                format!("@视频{video_order}")
            }
            _ => {
                audio_order += 1;
                format!("@音频{audio_order}")
            }
        };
        let placeholder = format!("{{{{asset:{}:{}}}}}", s(r, "assetId"), s(r, "variantId"));
        prompt = prompt.replace(&placeholder, &marker);
        for semantic in r["semanticBindings"].as_array().into_iter().flatten() {
            let placeholder = format!(
                "{{{{asset:{}:{}}}}}",
                s(semantic, "assetId"),
                s(semantic, "variantId")
            );
            prompt = prompt.replace(&placeholder, &marker);
        }
    }
    Ok(prompt)
}

pub fn validate_snapshot(store: &Store, snapshot: &Value) -> Result<()> {
    let unit = store.get("units", s(snapshot, "unitId"))?;
    if unit["version"] != snapshot["unitVersion"] {
        return Err("分镜版本已变化，请重新准备".into());
    }
    if store.source()?["version"] != snapshot["sourceVersion"] {
        return Err("原文版本已变化，请重新准备".into());
    }
    if snapshot.get("sourcePrompt").is_some() || snapshot.get("sourcePromptSha256").is_some() {
        if snapshot["sourcePrompt"].as_str() != unit["prompt"].as_str()
            || s(snapshot, "sourcePromptSha256") != crate::pipeline::digest(s(&unit, "prompt"))
        {
            return Err("单元原始提示词或来源哈希已变化，请重新准备".into());
        }
        let compiled = compile_bound_prompt(
            &unit,
            &store.values("assets")?,
            snapshot["references"].as_array().ok_or("素材快照缺失")?,
            snapshot
                .get("inlineReferences")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        )?;
        if compiled != s(snapshot, "prompt") {
            return Err("素材包提示词与冻结原始提示词不一致，请重新准备".into());
        }
    }
    if let Some(inline) = snapshot.get("inlineReferences") {
        let references = inline.as_array().ok_or("文字素材快照无效，请重新准备")?;
        let mut identities = std::collections::HashSet::new();
        for reference in references {
            let asset = store.active_asset(s(reference, "assetId"))?;
            if s(reference, "kind") != "inline-text"
                || !identities.insert((s(reference, "assetId"), s(reference, "variantId")))
                || asset["version"] != reference["assetVersion"]
                || s(&asset, "variantId") != s(reference, "variantId")
                || !inline_text_material(store, &asset)?
                || ["name", "description", "preparation", "sourceRefs"]
                    .iter()
                    .any(|field| asset[*field] != reference[*field])
            {
                return Err("文字素材身份、版本或制作方式已变化，请重新准备".into());
            }
        }
    }
    for r in snapshot["references"].as_array().ok_or("素材快照缺失")? {
        for semantic in r["semanticBindings"].as_array().into_iter().flatten() {
            let asset = store.active_asset(s(semantic, "assetId"))?;
            if asset["version"] != semantic["assetVersion"]
                || s(&asset, "variantId") != s(semantic, "variantId")
            {
                return Err("单元语义形象版本已变化，请重新准备".into());
            }
            let material = image_material_asset(store, &asset)?;
            if s(&material, "id") != s(r, "assetId")
                || s(&material, "variantId") != s(r, "variantId")
            {
                return Err("单元素材来源已变化，请重新准备".into());
            }
        }
        let m = store.get("media", s(r, "mediaId"))?;
        let p = store.verify_media(&m)?;
        if !same_path(&p, s(r, "path")) {
            return Err("素材执行路径已经变化，请重新准备".into());
        }
        if hash_file(&p)?.0 != s(r, "sha256") {
            return Err("素材快照哈希已失效".into());
        }
        if !s(r, "assetId").is_empty() {
            let a = store.active_asset(s(r, "assetId"))?;
            if a["version"] != r["assetVersion"]
                || store
                    .adopted(s(r, "assetId"), s(r, "variantId"))?
                    .as_deref()
                    != Some(s(r, "mediaId"))
            {
                return Err("采用图片或形象版本已变化，请重新准备".into());
            }
        }
    }
    Ok(())
}

fn inline_text_material(store: &Store, asset: &Value) -> Result<bool> {
    // The production catalogue uses PROP; legacy/manual assets use prop.
    // Only this domain kind can be a source-owned, parentless text interface.
    Ok(matches!(s(asset, "kind"), "PROP" | "prop")
        && asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("INLINE")
        && s(asset, "sourceAssetId").is_empty()
        && s(asset, "sourceVariantId").is_empty()
        && s(&asset["preparation"], "fromKey").is_empty()
        && store
            .adopted(s(asset, "id"), s(asset, "variantId"))?
            .is_none())
}

fn image_material_asset(store: &Store, asset: &Value) -> Result<Value> {
    let assets = store.values("assets")?;
    let mut current = asset.clone();
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert(s(&current, "id").to_owned()) {
            return Err("沿用图片的素材来源出现循环".into());
        }
        if crate::store::asset_deleted(&current) {
            return Err("参考图片的来源资产已删除".into());
        }
        match current.pointer("/preparation/mode").and_then(Value::as_str) {
            Some("UNRESOLVED") => return Err("素材制作方式尚未明确".into()),
            Some("AUDIO") => return Err("纯声音资产不能用作图片素材来源".into()),
            Some("INLINE") => {
                if store
                    .adopted(s(&current, "id"), s(&current, "variantId"))?
                    .is_some()
                {
                    return Ok(current);
                }
                let parent = crate::pipeline::resolve_asset_dependency(&assets, &current)?
                    .ok_or("沿用图片的形象没有明确来源，请指定基础形象")?;
                current = store.active_asset(&parent)?;
            }
            _ => return Ok(current),
        }
    }
}

/// Image-production references freeze every semantic appearance on the actual
/// donor path. A current explicitly adopted INLINE image is a real donor itself.
fn validate_image_semantics(store: &Store, reference: &Value) -> Result<()> {
    let Some(bindings) = reference.get("semanticBindings") else {
        // Older direct-image snapshots have no separate semantic appearance.
        return Ok(());
    };
    let bindings = bindings
        .as_array()
        .filter(|b| !b.is_empty())
        .ok_or("图片语义依赖快照为空或无效，请重新准备")?;
    let assets = store.values("assets")?;
    let mut identities = std::collections::HashSet::new();
    for semantic in bindings {
        if !identities.insert((s(semantic, "assetId"), s(semantic, "variantId"))) {
            return Err("图片语义依赖快照含重复身份，请重新准备".into());
        }
        let mut current = store.active_asset(s(semantic, "assetId"))?;
        if s(&current, "variantId") != s(semantic, "variantId")
            || current["version"] != semantic["assetVersion"]
        {
            return Err("图片语义形象身份或版本已变化，请重新准备".into());
        }
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(s(&current, "id").to_owned()) {
                return Err("图片语义素材来源出现循环，请修订来源关系".into());
            }
            if crate::store::asset_deleted(&current) {
                return Err("图片语义素材来源已删除，请重新准备".into());
            }
            let frozen = bindings
                .iter()
                .find(|b| {
                    s(b, "assetId") == s(&current, "id")
                        && s(b, "variantId") == s(&current, "variantId")
                })
                .ok_or("图片语义来源链缺少冻结形象，请重新准备")?;
            if frozen["assetVersion"] != current["version"] {
                return Err("图片语义形象版本已变化，请重新准备".into());
            }
            match current.pointer("/preparation/mode").and_then(Value::as_str) {
                Some("AUDIO") => return Err("纯声音形象不能作为图片依赖".into()),
                Some("UNRESOLVED") => return Err("图片语义素材制作方式尚未明确".into()),
                _ => {}
            }
            // Respect the first real adopted donor, including a manually chosen
            // INLINE image. Its unused ancestor does not define this material.
            if store
                .adopted(s(&current, "id"), s(&current, "variantId"))?
                .is_some()
                || current.pointer("/preparation/mode").and_then(Value::as_str) != Some("INLINE")
            {
                if current["id"] != reference["assetId"]
                    || current["variantId"] != reference["variantId"]
                    || current["version"] != reference["assetVersion"]
                {
                    return Err("图片语义形象的实际素材来源已变化，请重新准备".into());
                }
                break;
            }
            let parent = crate::pipeline::resolve_asset_dependency(&assets, &current)?
                .ok_or("图片语义形象没有明确的图片来源")?;
            current = store.active_asset(&parent)?;
        }
    }
    Ok(())
}

/// Images and videos share the same immutable-reference validity boundary.
pub fn validate_generation_snapshot(store: &Store, snapshot: &Value) -> Result<()> {
    if s(snapshot, "kind") == "video" {
        validate_snapshot(store, snapshot)?;
    } else if s(snapshot, "kind") == "image" {
        let asset = store.active_asset(s(snapshot, "assetId"))?;
        if asset["version"] != snapshot["assetVersion"] {
            return Err("图片设计版本已变化，请重新准备".into());
        }
        let assets = store.values("assets")?;
        if snapshot["sourceChain"].is_array() && json!(crate::pipeline::asset_design_contract::source_chain(&assets,&asset)?) != snapshot["sourceChain"] {
            return Err("冻结图片来源设计语义版本已变化，请重新准备".into());
        }
        let gate = json!({"source":store.source()?,"foundation":store.foundation()?,"assets":assets,"selection":{"assetIds":[asset["id"]]}});
        crate::pipeline::validate_asset_visual_gate(&gate,&asset)?;
        if crate::pipeline::asset_visual_facts::cached_visual_check_required(&gate)? {
            return Err("视觉状态待核对，已有提示词不能绕过未归属或不适用裁决".into());
        }
        if !snapshot["sourceVersion"].is_null()
            && store.source()?["version"] != snapshot["sourceVersion"]
        {
            return Err("原文版本已变化，请重新准备".into());
        }
        for r in snapshot["references"]
            .as_array()
            .ok_or("图片依赖快照缺失")?
        {
            let a = store.active_asset(s(r, "assetId"))?;
            let variant = if s(r, "variantId").is_empty() {
                s(&a, "variantId")
            } else {
                s(r, "variantId")
            };
            if s(&a, "variantId") != variant
                || a["version"] != r["assetVersion"]
                || store.adopted(s(r, "assetId"), variant)?.as_deref() != Some(s(r, "mediaId"))
            {
                return Err("图片派生参考已经替换，请重新准备".into());
            }
            validate_image_semantics(store, r)?;
            let m = store.get("media", s(r, "mediaId"))?;
            let p = store.verify_media(&m)?;
            if s(r, "sha256") != s(&m, "sha256") || !same_path(&p, s(r, "path")) {
                return Err("图片素材路径或哈希已变化，请重新准备".into());
            }
        }
    } else {
        return Err("未知生成类型".into());
    }
    Ok(())
}

fn same_path(path: &std::path::Path, other: &str) -> bool {
    !other.is_empty() && std::fs::canonicalize(path).ok() == std::fs::canonicalize(other).ok()
}

#[cfg(test)]
mod image_semantic_tests {
    use super::*;
    use tempfile::TempDir;

    struct Fixture {
        temporary: TempDir,
        store: Store,
    }
    impl Fixture {
        fn new() -> Self {
            let temporary = TempDir::new().unwrap();
            let store = Store::open(temporary.path().join("project")).unwrap();
            store
                .create("offline semantic image references", "p")
                .unwrap();
            for (id, variant, mode, parent) in [
                ("base", "base", "REFERENCE", None),
                ("middle", "base", "INLINE", Some("base")),
                ("inline", "coverage", "INLINE", Some("middle")),
                ("child", "base", "DERIVE", Some("inline")),
            ] {
                store
                    .save_asset(json!({"id":id,"baseAssetId":"identity","variantId":variant,
                    "name":"同名显示不决定身份","prompt":"offline","sourceAssetId":parent,
                    "preparation":{"mode":mode}}))
                    .unwrap();
            }
            let fixture = Self { temporary, store };
            fixture.adopt("base", 20);
            fixture
        }
        fn adopt(&self, asset: &str, tone: u8) -> Value {
            let a = self.store.active_asset(asset).unwrap();
            let path = self.temporary.path().join(format!("{asset}-{tone}.png"));
            image::RgbImage::from_pixel(2, 2, image::Rgb([tone, 10, 30]))
                .save(&path)
                .unwrap();
            let media = self
                .store
                .import_media(&path, "image", Some(asset), Some(s(&a, "variantId")))
                .unwrap();
            self.store
                .adopt(asset, s(&a, "variantId"), s(&media, "id"))
                .unwrap();
            media
        }
        fn snapshot(&self, donor: &str, semantics: &[&str]) -> Value {
            let child = self.store.active_asset("child").unwrap();
            let asset = self.store.active_asset(donor).unwrap();
            let mid = self
                .store
                .adopted(donor, s(&asset, "variantId"))
                .unwrap()
                .unwrap();
            let media = self.store.get("media", &mid).unwrap();
            json!({"kind":"image","assetId":"child","assetVersion":child["version"],"variantId":"base",
                "references":[{"kind":"image","assetId":donor,"assetVersion":asset["version"],"variantId":asset["variantId"],
                    "mediaId":mid,"sha256":media["sha256"],"path":self.store.verify_media(&media).unwrap(),
                    "semanticBindings":semantics.iter().map(|id|{let a=self.store.active_asset(id).unwrap();
                        json!({"assetId":id,"variantId":a["variantId"],"assetVersion":a["version"]})}).collect::<Vec<_>>() }]})
        }
    }

    #[test]
    fn every_inline_identity_and_intermediate_version_is_frozen_beside_the_real_parent_image() {
        for changed in ["inline", "middle"] {
            let fixture = Fixture::new();
            let snapshot = fixture.snapshot("base", &["inline", "middle", "base"]);
            validate_generation_snapshot(&fixture.store, &snapshot).unwrap();
            let physical = fixture.store.active_asset("base").unwrap();
            let media = s(&snapshot["references"][0], "mediaId").to_owned();
            let mut asset = fixture.store.active_asset(changed).unwrap();
            asset["expectedVersion"] = asset["version"].clone();
            asset["description"] = json!("语义状态已更新");
            fixture.store.save_asset(asset).unwrap();
            assert!(validate_generation_snapshot(&fixture.store, &snapshot)
                .unwrap_err()
                .contains("语义"));
            assert_eq!(
                fixture.store.active_asset("base").unwrap()["version"],
                physical["version"]
            );
            assert_eq!(fixture.store.adopted("base", "base").unwrap(), Some(media));
        }
    }

    #[test]
    fn inline_manual_image_is_the_material_and_its_unused_parent_is_not_inferred() {
        let fixture = Fixture::new();
        let inherited = fixture.snapshot("base", &["inline", "middle", "base"]);
        let manual = fixture.adopt("inline", 70);
        assert!(validate_generation_snapshot(&fixture.store, &inherited)
            .unwrap_err()
            .contains("素材来源"));
        let direct = fixture.snapshot("inline", &["inline"]);
        validate_generation_snapshot(&fixture.store, &direct).unwrap();
        assert_eq!(direct["references"][0]["mediaId"], manual["id"]);
        let mut asset = fixture.store.active_asset("inline").unwrap();
        asset["sourceAssetId"] = json!("missing-unused-parent");
        asset["expectedVersion"] = asset["version"].clone();
        fixture.store.save_asset(asset).unwrap();
        let direct = fixture.snapshot("inline", &["inline"]);
        validate_generation_snapshot(&fixture.store, &direct).unwrap();
    }

    #[test]
    fn intermediate_manual_image_stops_the_semantic_path_at_the_exact_adopted_identity() {
        let fixture = Fixture::new();
        fixture.adopt("middle", 80);
        let exact = fixture.snapshot("middle", &["inline", "middle"]);
        validate_generation_snapshot(&fixture.store, &exact).unwrap();
        let obsolete = fixture.snapshot("base", &["inline", "middle", "base"]);
        assert!(validate_generation_snapshot(&fixture.store, &obsolete).is_err());
    }

    #[test]
    fn semantic_chain_cannot_omit_intermediate_identity_or_hide_a_bad_variant() {
        let fixture = Fixture::new();
        let missing = fixture.snapshot("base", &["inline", "base"]);
        assert!(validate_generation_snapshot(&fixture.store, &missing)
            .unwrap_err()
            .contains("来源链"));
        let original = fixture.snapshot("base", &["inline", "middle", "base"]);
        for bad in [Value::Null, json!([]), json!({"assetId":"inline"})] {
            let mut snapshot = original.clone();
            snapshot["references"][0]["semanticBindings"] = bad;
            assert!(validate_generation_snapshot(&fixture.store, &snapshot).is_err());
        }
        let mut wrong = original.clone();
        let mut bad = wrong["references"][0]["semanticBindings"][0].clone();
        bad["variantId"] = json!("different-appearance");
        wrong["references"][0]["semanticBindings"]
            .as_array_mut()
            .unwrap()
            .push(bad);
        assert!(
            validate_generation_snapshot(&fixture.store, &wrong).is_err(),
            "a valid sibling entry must not conceal a false semantic variant"
        );
        let mut duplicate = original.clone();
        duplicate["references"][0]["semanticBindings"]
            .as_array_mut()
            .unwrap()
            .push(original["references"][0]["semanticBindings"][0].clone());
        assert!(validate_generation_snapshot(&fixture.store, &duplicate).is_err());
    }

    #[test]
    fn cyclic_missing_audio_and_unresolved_material_paths_fail_before_submission() {
        for failure in ["cycle", "missing", "audio", "unresolved"] {
            let fixture = Fixture::new();
            let mut asset = fixture.store.active_asset("middle").unwrap();
            asset["expectedVersion"] = asset["version"].clone();
            match failure {
                "cycle" => asset["sourceAssetId"] = json!("inline"),
                "missing" => asset["sourceAssetId"] = json!("unregistered-parent"),
                "audio" => asset["preparation"]["mode"] = json!("AUDIO"),
                _ => asset["preparation"]["mode"] = json!("UNRESOLVED"),
            }
            fixture.store.save_asset(asset).unwrap();
            let snapshot = fixture.snapshot("base", &["inline", "middle", "base"]);
            assert!(
                validate_generation_snapshot(&fixture.store, &snapshot).is_err(),
                "{failure}"
            );
        }
    }

    #[test]
    fn legacy_direct_reference_remains_valid_and_actual_image_replacement_never_does() {
        let fixture = Fixture::new();
        let mut snapshot = fixture.snapshot("base", &["base"]);
        snapshot["references"][0]
            .as_object_mut()
            .unwrap()
            .remove("semanticBindings");
        validate_generation_snapshot(&fixture.store, &snapshot).unwrap();
        fixture.adopt("base", 110);
        assert!(validate_generation_snapshot(&fixture.store, &snapshot).is_err());
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;

    #[test]
    fn whole_second_carrier_accepts_only_small_binary_error_not_real_fractional_timing() {
        let sum: f64 = (0..10).fold(4.0, |value, _| value + 0.1);
        assert_ne!(sum.fract(), 0.0);
        assert_eq!(normalized_integer_duration(sum), Some(5));
        assert_eq!(normalized_integer_duration(15.0 + 0.5e-6), Some(15));
        assert_eq!(normalized_integer_duration(15.0 - 0.5e-6), Some(15));
        assert_eq!(normalized_integer_duration(15.0 + 2e-6), None);
        assert_eq!(normalized_integer_duration(14.5), None);
        assert_eq!(normalized_integer_duration(0.0), None);
        assert_eq!(normalized_integer_duration(0.5e-6), None);
        assert_eq!(normalized_integer_duration(f64::NAN), None);
        assert_eq!(normalized_integer_duration(f64::INFINITY), None);
        assert_eq!(normalized_integer_duration(u32::MAX as f64 + 1.0), None);
    }

    #[test]
    fn binding_normalizes_wire_only_and_never_rewrites_the_saved_unit() {
        let temporary = tempfile::TempDir::new().unwrap();
        let store = Store::open(temporary.path().join("project")).unwrap();
        store.create("isolated numeric test", "p").unwrap();
        let source = crate::pipeline::prepare_source("第一集\n场景：门口\n△小王推门。\n").unwrap();
        let episode = source.episodes[0].id.clone();
        let scene = source.scenes[0].id.clone();
        let row = source.rows[2].id.clone();
        store.save_source(json!(source)).unwrap();
        for (duration, expected, blocked) in
            [(15.0 + 0.5e-6, json!(15), false), (14.5, json!(14.5), true)]
        {
            let current=store.save_unit(json!({"id":"u","episodeId":episode,"sceneId":scene,"sourceRefs":[row],"prompt":"人物推门，原动作完整。","duration":duration,"assetRefs":[],"shots":[{"duration":duration,"picture":"原动作","dialogueParts":[]}]})).unwrap();
            let snapshot = bind(&store, &current, "mock", "video").unwrap();
            assert_eq!(snapshot["duration"], expected);
            assert_eq!(
                snapshot["problems"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|problem| problem["code"] == "DURATION_REVISION_REQUIRED"),
                blocked
            );
            assert_eq!(store.get("units", "u").unwrap(), current);
            assert_eq!(current["shots"][0]["duration"], json!(duration));
        }
    }
}
