//! Versioned image-prompt authoring. This module never generates or adopts media.
//! A storyboard, its adopted design and an image prompt are separate evidence.
use super::{digest, TextExecutor, TextRequest};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

pub const PROFILE: &str = "desktop-series672-assets112/v1";
pub const PROMPT_VERSION: &str = "asset-image-prompt/1.1.2";
pub const REQUEST_V2: &str = "asset-prompt-request/v2";
pub const SOURCE_BINDING_V1: &str = "asset-prompt-source-binding/v1";
pub const COSTUME_PROMPT_V1: &str = "asset-costume-prompt-layout/v1";
const MEDIA_PLAN_VERSION: &str = "asset-media-plan/v1";
const CONCURRENCY: usize = 4;
const COSTUME_PROMPT_CONTRACT: &str = "换装文字/派发职责合同 asset-costume-prompt-layout/v1：当前仅编写完整服装提示词与必要文字设计，不执行出图、合成或审图。costumeComposition 中的 registeredLayout 是已登记母图布局元数据，文件尺寸及摘要经过本地检查，不是本次模型读图证据。若 identityCopyMode=full-width-strip-original-pixels，整条 identityStrip 已覆盖三脸，禁止再要求三张脸各自槽位坐标。无母图、未采用、未登记布局、像素尺寸或裁切坐标缺失，只属于图片派发前的机械装配待办，不应写进本次 unresolved，也不阻断完整编词；仍须写清服装、材质、四视图、保留身份/发型/体型、上排预留与遮面规则。提示词中用“上排保留区及分界按派发时绑定的已确认母版布局”表达，不能编造尺寸或把当前像素值永久写死到服装提示词。实际尺寸由派发器对当前母版重新绑定，合成器先丢弃生成图上排、仅缩放下排，母图三脸原像素复制并验证；这些都不属于当前已完成成果。人物身份、服装目标、剧情事实或参考职责有实质歧义才进入 unresolved；不能为了完成写词清除或掩盖这些歧义。始终 imageEvidence=NO_PIXELS_READ。";
const MAIN: &str = include_str!("../../../../../rules/assets/1.1.2/SKILL.md");
const HANDOFF: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/22_storyboard_handoff.md");
const COMPILER: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/16_gpt_image25_prompt_compiler.md");
const REFERENCES: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/15_reference_router.md");
const BATCH: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/21_batch_production.md");
const CHARACTER: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/07_character_prompt_engine.md");
const COSTUME: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/08_costume_engine.md");
const GROUP: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/09_extra_cast_engine.md");
const SCENE: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/11_scene_prompt_engine.md");
const TOPOLOGY: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/12_scene_topology_lock.md");
const OPENING: &str =
    include_str!("../../../../../rules/assets/1.1.2/modules/14_opening_reference_engine.md");

const CONTRACT: &str = "桌面资产提示词岗位 desktop-series672-assets112/v1：只为当前显式图片项编写中文成品提示词，不调用图片服务，不改变已确认方向、资产身份、正式名、文件名、分镜叙事或 adoptedDesign。接续模式优先；不要重开三套画风，也不把电影感强改为真人。完整采用设计是约束，不是图片成品提示词；原文事实和普通美术补足分开。sourceKind=imported-md 时，证据只是外部分镜，不能伪称原始剧本。提供完整采用设计时 designSupplement 必须为空；否则依据本项精确片段补必要文字设计到 designSupplement，候选不冒称二阶段已采用设计。没有被证据锁死且不改变剧情/身份/拓扑的美术细节自主补足，实质无法裁决的歧义逐条写 unresolved，只阻塞本项。图片尚未读取，imageEvidence 必须为 NO_PIXELS_READ；即使目录有 mediaId，也不能声称目视质检、锁脸像素或正式采用已通过。referenceRequirements 是已登记的实际媒体需求，不能从家族/文字设计依赖或中文名字另猜父图、偷偷新增母图任务；确需未登记的参考而当前证据不足，说明具体缺口。只换衣服的 costume 任务独立生成下排四视图并保留母版上排槽位，原像素复制由本地后续合成完成；发型/年龄/形态实变不得机械贴旧脸。普通人物采用 1:1 七视图，非人主体用真实形态适配，不硬套人类，群演合板独立模板；标准场景 16:9 无人且拓扑清晰。imagePresentation 是资产板画幅，与视频画幅无关。按类型选择 layoutKind；任何已明确 imageTask.kind 不得擅自修改。提示词写实际目标/不可变特征/服装或空间结构/参考职责/光影材质构图/短避错，不混入评分、流程或整本证据。sourceRefs 只能使用本包 evidence.id；采用设计完整时可以不重复源文。已保存 raw 的恢复由原执行器处理，不创建新的付费尝试。";

const MEDIA_PLAN_CONTRACT: &str = "新增运输合同 asset-prompt-request/v2：mediaPlan 是第三阶段独立媒体生产计划，不能改 adoptedDesign、原文或显式图片队列。已有 referenceRequirements 时选 mode=preserve，输出空 referenceRequirements，程序完整复用已登记计划；没有明确计划时选 mode=plan，从 mediaCandidates 的真实 ID/variant 与 mediaEvidence 制定直接父图需求。每个新需求必须给出实际职责 role、mustPreserve/mayChange、逐字引用的 evidence（evidenceId/quote）；引用必须实际覆盖本项与目标，不可把同场出现本身当作要垫图的理由。相同 identityId/baseAssetId 只是可选关系，不自动成为媒体依赖；sourceAssetId 是旧关系线索，不足以证明同脸；designDependencies 只是文字设计依赖，不能直接转换。身份不明确或缺实际参考依据时写 unresolved，不凭名称相似猜锁脸。换装 imageTask.kind=costume 必须有明确同一身份的 face/identity/identity-faces 需求，发型年龄形态实变不是换装。场景状态/机位可声明 topology/state/viewpoint，引用同一场景母版；开场合成按实际首帧分别选 scene/appearance/prop。没有必要参考时明确给空计划和理由。imageTask.kind 与 layoutKind 必须相同；计划只声明未来需要的媒体，图片未读取、母图未采用或槽位未登记都不能伪称已通过。母图在队列外时只报告缺口，不生成或追加任务。";

fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn body(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub fn enabled(input: &Value) -> Result<bool, String> {
    match input.get("workflowProfile").filter(|v| !v.is_null()) {
        None => Ok(false),
        Some(value) if value == PROFILE => Ok(true),
        _ => Err("未知资产工作流版本；未按旧流程猜测执行，也未改写原冻结请求".into()),
    }
}

fn source_text(source: &Value) -> &str {
    if source["sourceKind"] == "original-script" {
        s(&source["lockedSource"], "text")
    } else {
        s(source, "text")
    }
}

pub fn validate_source(source: &Value) -> Result<(), String> {
    match s(source, "sourceKind") {
        "original-script" => {
            let locked: super::LockedSource =
                serde_json::from_value(source["lockedSource"].clone()).map_err(|_| {
                    "原剧本来源缺少有效 lockedSource；不得用分镜冒充原剧本".to_owned()
                })?;
            super::validate_source(&locked)?;
            if source["sha256"] != locked.sha256 {
                return Err("原剧本来源摘要不匹配".into());
            }
        }
        "imported-md" | "adopted-storyboard" | "external-package" => {
            if s(source, "text").trim().is_empty()
                || s(source, "sha256") != digest(s(source, "text"))
            {
                return Err("分镜来源正文缺失或摘要不匹配".into());
            }
            if source.get("lockedSource").is_some_and(|v| !v.is_null()) {
                return Err("分镜来源不能附加伪造的原剧本记录".into());
            }
            if source["sourceKind"] == "adopted-storyboard" {
                let manifest = &source["handoffManifest"];
                if !manifest.is_object()
                    || s(manifest, "assetRegistryId").is_empty()
                    || manifest["assetRegistryRevision"]
                        .as_u64()
                        .filter(|v| *v > 0)
                        .is_none()
                    || s(source, "handoffSha256") != digest(&manifest.to_string())
                {
                    return Err("采用分镜来源缺少已核同版交接清单及其摘要".into());
                }
            }
            if source["sourceKind"]=="external-package" {
                let packages=array(source,"completePackages");
                if packages.is_empty() || packages.len()!=array(source,"documents").len()
                    || packages.iter().any(|p|!matches!(s(&p["proof"],"level"),"desktop-complete-package"|"upstream-module22-package")
                        || ["packageFingerprint","manifestSha256","registrySha256","queueHash","directionSha256"].iter().any(|k|s(&p["proof"],k).len()!=64)
                        || !array(source,"documents").iter().any(|d|d["documentId"]==p["documentId"])) {
                    return Err("完整接续包来源缺少逐文档验证证明；不冒称上游模块22或季范围已验收".into());
                }
            }
        }
        _ => return Err("资产提示词来源类型无效".into()),
    }
    let mut ids = HashSet::new();
    for row in array(source, "evidence") {
        if s(row, "id").is_empty() || s(row, "text").trim().is_empty() || !ids.insert(s(row, "id"))
        {
            return Err("分镜取料包含空或重复证据身份".into());
        }
        let text = source_text(source);
        match (row["start"].as_u64(), row["end"].as_u64()) {
            (Some(start), Some(end))
                if start < end
                    && text.get(start as usize..end as usize) == Some(s(row, "text")) => {}
            (None, None) if text.contains(s(row, "text")) => {}
            _ => {
                return Err(format!(
                    "证据 {} 不属于所声明来源正文或位置已变化",
                    s(row, "id")
                ))
            }
        }
    }
    Ok(())
}

fn evidence_for(source: &Value, asset: &Value) -> Vec<Value> {
    let refs: HashSet<_> = array(asset, "sourceRefs")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let supplied = array(source, "evidence");
    if !supplied.is_empty() {
        return supplied
            .iter()
            .filter(|row| {
                refs.contains(s(row, "id"))
                    || array(row, "assetIds").iter().any(|id| id == &asset["id"])
            })
            .cloned()
            .collect();
    }
    if source["sourceKind"] == "original-script" {
        return array(&source["lockedSource"], "rows")
            .iter()
            .filter(|row| refs.contains(s(row, "id")))
            .cloned()
            .collect();
    }
    Vec::new()
}

fn design_revision(asset: &Value) -> Value {
    let design = &asset["adoptedDesign"];
    [
        design.get("designRevision"),
        design.get("revision"),
        asset.get("designRevision"),
        // A schema version such as series-full-design/v1 is not a revision.
        design.get("version").filter(|v| v.is_u64()),
    ]
    .into_iter()
    .flatten()
    .find(|v| !v.is_null())
    .cloned()
    .unwrap_or(Value::Null)
}

fn design_full(asset: &Value) -> Option<&Value> {
    asset["adoptedDesign"].get("full").filter(|v| match v {
        Value::String(text) => !text.trim().is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => false,
    })
}

fn validate_design(asset: &Value) -> Result<(), String> {
    if let Some(full) = design_full(asset) {
        let design = &asset["adoptedDesign"];
        let revision = design_revision(asset);
        let valid_revision = revision.as_u64().is_some_and(|v| v > 0)
            || revision.as_str().is_some_and(|v| !v.trim().is_empty());
        let adopted_state = |key: &str| {
            design.get(key).filter(|v| !v.is_null()).is_none_or(|v| {
                v.as_str().is_some_and(|v| {
                    v.eq_ignore_ascii_case("adopted") || v.eq_ignore_ascii_case("approved")
                })
            })
        };
        if !valid_revision
            || crate::asset_graph::design_review_blocked(asset)
            || !adopted_state("state")
            || !adopted_state("status")
            || design["adopted"] == false
            || (design["version"] == "series-full-design/v1" && design["status"] != "ADOPTED")
        {
            return Err(format!("{} 的完整设计缺版本或尚未采用", s(asset, "id")));
        }
        for (design_key, asset_key) in [("assetId", "id"), ("variantId", "variantId")] {
            if let Some(value) = design.get(design_key).filter(|v| !v.is_null()) {
                if value != &asset[asset_key] {
                    return Err(format!("{} 的完整采用设计身份不匹配", s(asset, "id")));
                }
            }
        }
        if let Some(hash) = design["fullSha256"].as_str() {
            if hash != digest(&body(full)) {
                return Err(format!("{} 的完整采用设计摘要不匹配", s(asset, "id")));
            }
        }
    }
    Ok(())
}

fn dependencies(asset: &Value) -> Result<&[Value], String> {
    [
        asset["adoptedDesign"].get("designDependencies"),
        asset.get("designDependencies"),
        asset["adoptedDesign"].get("dependencies"),
    ]
    .into_iter()
    .flatten()
    .find(|v| !v.is_null())
    .map(|v| {
        v.as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{} 的文字设计依赖不是有效列表", s(asset, "id")))
    })
    .unwrap_or(Ok(&[]))
}

fn dependency_id(dependency: &Value) -> &str {
    dependency["assetId"]
        .as_str()
        .or_else(|| dependency["targetAssetId"].as_str())
        .unwrap_or("")
}

fn supplied_package<'a>(input:&'a Value,asset:&Value)->Result<Option<&'a Value>,String> {
    let proof=&asset["handoffPackage"];
    if proof.is_null(){return Ok(None);}
    let package=array(input,"externalPackages").iter().find(|p|p["proof"]["packageFingerprint"]==proof["packageFingerprint"])
        .ok_or("完整包资产缺少对应的已验证资料；不能按名字拼接其它包")?;
    if package["version"]!="desktop-complete-handoff/v1" || !matches!(s(&package["proof"],"level"),"desktop-complete-package"|"upstream-module22-package")
        || package["proof"]["registrySha256"]!=digest(&package["registry"].to_string())
        || package["proof"]["directionSha256"]!=digest(&package["direction"].to_string())
        || package["proof"]["queueHash"]!=package["queue"]["queueHash"] {
        return Err("完整包方向、总库或范围不是原验证版本".into());
    }
    Ok(Some(package))
}

fn parent_designs(asset: &Value, assets: &[Value]) -> Result<Vec<Value>, String> {
    fn walk(
        asset: &Value,
        assets: &[Value],
        chain: &mut Vec<String>,
        seen: &mut HashSet<String>,
        result: &mut Vec<Value>,
    ) -> Result<(), String> {
        for dependency in dependencies(asset)? {
            let id = dependency_id(dependency);
            if chain.iter().any(|key| key == id) {
                chain.push(id.into());
                return Err(format!("文字设计依赖循环：{}", chain.join(" → ")));
            }
            let parent = assets
                .iter()
                .find(|a| s(a, "id") == id)
                .ok_or_else(|| format!("文字设计依赖缺项：{} → {id}", chain.join(" → ")))?;
            validate_design(parent)?;
            let full = design_full(parent).ok_or_else(|| {
                format!("{} 等待 {} 的完整采用设计；不要求母图", s(asset, "id"), id)
            })?;
            let revision = dependency
                .get("designRevision")
                .or_else(|| dependency.get("assetRevision"))
                .or_else(|| dependency.get("revision"))
                .filter(|v| !v.is_null())
                .ok_or_else(|| format!("文字设计依赖 {id} 缺少固定版本"))?;
            if revision != &design_revision(parent) {
                return Err(format!("文字设计依赖 {id} 的采用版本已变化"));
            }
            if !seen.insert(id.into()) {
                continue;
            }
            result.push(json!({"assetId":id,"designRevision":revision,"full":full,"fullSha256":digest(&body(full))}));
            chain.push(id.into());
            walk(parent, assets, chain, seen, result)?;
            chain.pop();
        }
        Ok(())
    }
    let mut result = Vec::new();
    walk(
        asset,
        assets,
        &mut vec![s(asset, "id").into()],
        &mut HashSet::new(),
        &mut result,
    )?;
    Ok(result)
}

fn references(asset: &Value, assets: &[Value]) -> Result<Vec<Value>, String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    if asset
        .get("referenceRequirements")
        .is_some_and(|v| !v.is_null() && !v.is_array())
    {
        return Err(format!("{} 的媒体需求不是有效列表", s(asset, "id")));
    }
    for reference in array(asset, "referenceRequirements") {
        if reference["required"] == false {
            continue;
        }
        let id = s(reference, "targetAssetId");
        let role = reference["role"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| reference["kind"].as_str().filter(|v| !v.trim().is_empty()))
            .unwrap_or("reference");
        if id.is_empty() || id == s(asset, "id") || !seen.insert((id, role)) {
            return Err(format!(
                "{} 的媒体需求缺身份、重复或引用自身",
                s(asset, "id")
            ));
        }
        if !assets.iter().any(|a| s(a, "id") == id) {
            return Err(format!("{} 的真实参考需求缺项：{id}", s(asset, "id")));
        }
        // `kind` is the handoff field; `role` is the desktop media-binding field.
        // A target-only relation remains a generic reference, never a guessed face lock.
        let mut normalized = reference.clone();
        normalized["role"] = json!(role);
        result.push(normalized);
    }
    Ok(result)
}

fn kind(asset: &Value) -> &str {
    asset["kind"]
        .as_str()
        .or_else(|| asset["type"].as_str())
        .unwrap_or("")
}
fn category(asset: &Value) -> &'static str {
    match kind(asset) {
        "CH" | "character" | "人物" => "character",
        "SC" | "LOC" | "scene" | "场景" => "scene",
        "PR" | "PROP" | "prop" | "道具" => "prop",
        "SE" | "audio" | "voice" | "声音" => "audio",
        _ => "other",
    }
}
fn explicit_layout(asset: &Value) -> &str {
    asset["imageTask"]["kind"]
        .as_str()
        .or_else(|| {
            if asset["imagePresentation"]["origin"] == "asset-skill-1.1.2" {
                None
            } else {
                asset["imagePresentation"]["layoutKind"].as_str()
            }
        })
        .unwrap_or("")
}
fn default_layout(asset: &Value) -> &str {
    let explicit = explicit_layout(asset);
    if !explicit.is_empty() {
        return explicit;
    }
    if asset["subjectForm"] == "nonhuman" {
        return "nonhuman";
    }
    if asset["isGroup"] == true {
        return "group";
    }
    match category(asset) {
        "character" => "single-character",
        "scene" => "scene",
        _ => "prop",
    }
}
fn presentation(layout: &str) -> Value {
    let (ratio,composition) = match layout {
        "single-character" => ("1:1","1:1正方形单人人物七视图，纯白画布、白棚柔光、留白自然分区，无额外标题编号文字边框。上排同尺寸中性正脸、向右纯侧脸、向右45°三分之四脸，仅头颈肩锁骨、无衣服饰品遮挡，保持正式发型。下排正面、向右侧面、背面全身，同身高同脚底基线，完整头脚、自然下垂双手不持物；右下放大正面上半身服装。正面全身、侧面全身和放大上半身的面部用纯白不透明完全遮五官，仅遮面部，保留头型耳朵发际线发型；背面保持自然后脑无白斑。"),
        "costume" => ("1:1","1:1正方形换装下排专用板：仅生成正面、向右侧面、背面完整全身和右下放大上半身四个服装视图；上排为已采用身份母版三脸保留同尺寸同坐标空槽，不生成或重画上排头部。具体画布尺寸和槽位由派发时真实母版布局固定。下排正面、侧面及放大上半身纯白不透明遮面，保留正式发型；背面自然无白斑。完整头脚、双手自然不持物、纯白背景。随后由本地原像素复制并逐像素验证三脸；本次提示词本身不代表合成或验证完成。"),
        "nonhuman" => ("1:1","1:1非人主体形态资料板，按实际物种/形态展示可辨识正侧背和关键结构，保持同一身份、比例、材质和状态；纯白中性背景、主体完整入画，不硬套人类七视图、白脸或人类服装版式，不添加无依据肢体与拟人特征。"),
        "group" => ("16:9","16:9群演合板，最多6–8人，一字排开正面完整全身、纯白背景，不加姓名编号文字；服装体系一致且每人可辨，保持自然年龄身高体型差，不用同脸换衣。"),
        "scene" => ("16:9","16:9标准场景资产，绝对无人（含剪影、远景人影和镜面倒影），空间结构清晰且基本全清晰，前中后景、门窗通道地标和表演区明确；尺度可用、垂直水平线稳定、边缘安全区，不强虚化、鱼眼或无依据遮挡。昼夜天气机位变化只改变允许层，保持已定拓扑。"),
        "opening" => ("16:9","16:9开场首帧合成参考，依据实际第一帧状态与明确参考职责，仅放已在场人物及对应服装，后进场人物不得提前出现；服从场景拓扑、进出口和动作起点，不加姓名编号箭头或站位线。"),
        "system-visual" => ("1:1","系统视觉资产遵守已采用可见载体、显示域和用途，保留明确要求的可见文字；不把纯系统声音做成图片，不改变原声。"),
        _ => ("1:1","独立道具资产完整清晰展示实际用途所需结构、材质、尺度和操作位置；保持源文明确文字与形态，不添加无依据人物、额外说明或水印。"),
    };
    json!({"version":"asset-image-presentation/1.1.2","layoutKind":layout,"ratio":ratio,"composition":composition,"origin":"asset-skill-1.1.2"})
}
pub fn image_presentation(asset: &Value) -> Value {
    let saved = asset["imagePrompt"]["layoutKind"]
        .as_str()
        .filter(|_| asset["imagePrompt"]["version"] == PROMPT_VERSION);
    presentation(saved.unwrap_or_else(|| default_layout(asset)))
}
pub fn prompt_text(asset: &Value) -> &str {
    asset["imagePrompt"]["text"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| s(asset, "prompt"))
}
pub fn compile_preview(asset: &Value) -> Result<Value, String> {
    if s(&asset["imagePrompt"], "text").trim().is_empty() {
        return Err("尚无新流程图片提示词；原手写提示词仍保留".into());
    }
    Ok(
        json!({"prompt":asset["imagePrompt"]["text"],"promptCompilerVersion":PROMPT_VERSION,"imagePresentation":image_presentation(asset),"assetVersion":asset["version"],"referenceRequirements":asset["referenceRequirements"],"imageEvidence":"NO_PIXELS_READ","requestFrozen":false}),
    )
}

/// Use exact, applicable main-file sections. The complete source hash remains
/// bound alongside the selected text; unrelated templates are not sent per item.
fn main_sections(numbers: &[u32]) -> String {
    let mut include = false;
    let mut result = String::new();
    for line in MAIN.split_inclusive('\n') {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(number) = heading
                .split_whitespace()
                .next()
                .and_then(|v| v.trim_end_matches('.').parse::<u32>().ok())
            {
                include = numbers.contains(&number);
            }
        }
        if include {
            result.push_str(line);
        }
    }
    result
}
fn rule(id: &str, content: &str) -> Value {
    json!({"id":id,"version":"1.1.2","sha256":digest(content),"content":content})
}
pub fn rule_snapshot(asset: &Value) -> Value {
    let mut sections = vec![1, 3, 10, 11, 12, 14, 15, 16, 17, 20];
    let mut methods = vec![
        rule("asset-handoff-22", HANDOFF),
        rule("asset-compiler-16", COMPILER),
        rule("asset-references-15", REFERENCES),
        rule("asset-batch-21", BATCH),
    ];
    match category(asset) {
        "character" => {
            sections.extend([4, 5]);
            methods.extend([
                rule("asset-character-07", CHARACTER),
                rule("asset-costume-08", COSTUME),
                rule("asset-group-09", GROUP),
            ]);
        }
        "scene" => {
            sections.extend([6, 7, 8, 9]);
            methods.extend([
                rule("asset-scene-11", SCENE),
                rule("asset-topology-12", TOPOLOGY),
                rule("asset-opening-14", OPENING),
            ]);
        }
        _ => {}
    }
    let selected = main_sections(&sections);
    let primary = rule(
        "gpt-image25-asset-workflow",
        &format!("{CONTRACT}\n\n{selected}"),
    );
    json!({"primary":primary,"methods":methods,"sourcePackage":{"version":"1.1.2","mainSha256":digest(MAIN),"handoffSha256":digest(HANDOFF),"mainSections":sections,"selectedMainSha256":digest(&selected)},"workflowProfile":PROFILE})
}

fn planning_input(asset: &Value) -> Value {
    let mut value = asset.clone();
    let plan = &asset["mediaPlan"];
    // Generated transport fields must not invalidate their own completed cache.
    // A user edit stops this projection and becomes a new explicit constraint.
    if plan["version"] == MEDIA_PLAN_VERSION
        && plan["origin"] == "asset-skill-1.1.2"
        && asset["referenceRequirements"] == plan["referenceRequirements"]
        && asset["imageTask"] == plan["imageTask"]
    {
        value["referenceRequirements"] = plan["inputReferenceRequirements"].clone();
        value["imageTask"] = plan["inputImageTask"].clone();
    }
    value
}

fn same_identity(a: &Value, b: &Value) -> bool {
    let key = |v: &Value| {
        v["identityId"]
            .as_str()
            .filter(|v| !v.is_empty())
            .or_else(|| v["baseAssetId"].as_str().filter(|v| !v.is_empty()))
            .map(str::to_owned)
    };
    if let (Some(a), Some(b)) = (key(a), key(b)) {
        return a == b;
    }
    a["baseAssetId"] == b["id"] || b["baseAssetId"] == a["id"]
}

/// Identity/design evidence only; image-prompt saves and later image adoption do
/// not change this fingerprint. A parent identity/design edit does.
pub(crate) fn media_identity_fingerprint(asset: &Value) -> String {
    digest(&json!({"id":asset["id"],"variantId":asset["variantId"],"kind":kind(asset),
        "identityId":asset["identityId"],"baseAssetId":asset["baseAssetId"],"parentAssetId":asset["parentAssetId"],
        "sourceAssetId":asset["sourceAssetId"],"sourceVariantId":asset["sourceVariantId"],
        "description":asset["description"],"sourceRefs":asset["sourceRefs"],"sourceFacts":asset["sourceFacts"],
        "adoptedDesign":asset["adoptedDesign"],"designStatus":asset["designStatus"],"deletedAt":asset["deletedAt"]}).to_string())
}

fn media_context(asset: &Value, assets: &[Value], evidence: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let related = |target: &Value| {
        target["id"] == asset["id"]
            || same_identity(asset, target)
            || ["sourceAssetId", "parentAssetId", "baseAssetId"]
                .iter()
                .any(|key| asset[*key] == target["id"])
            || array(asset, "referenceRequirements")
                .iter()
                .any(|r| r["targetAssetId"] == target["id"])
            || dependencies(asset)
                .unwrap_or(&[])
                .iter()
                .any(|d| dependency_id(d) == s(target, "id"))
            || evidence.iter().any(|e| {
                array(e, "assetIds").contains(&target["id"])
                    || array(target, "sourceRefs").contains(&e["id"])
            })
    };
    let candidates: Vec<_> = assets.iter().filter(|a| related(a) && s(a,"deletedAt").is_empty())
        .map(|a| json!({"id":a["id"],"variantId":a["variantId"],"name":a["name"],"kind":kind(a),
            "identityId":a["identityId"],"baseAssetId":a["baseAssetId"],"parentAssetId":a["parentAssetId"],
            "sourceAssetId":a["sourceAssetId"],"description":a["description"],"sourceFacts":a["sourceFacts"],
            "designRevision":design_revision(a),"evidenceFingerprint":media_identity_fingerprint(a)})).collect();
    let mut media_evidence = Vec::new();
    for row in evidence {
        let mut ids = array(row, "assetIds").to_vec();
        for a in assets
            .iter()
            .filter(|a| array(a, "sourceRefs").contains(&row["id"]))
        {
            if !ids.contains(&a["id"]) {
                ids.push(a["id"].clone());
            }
        }
        media_evidence.push(
            json!({"id":row["id"],"kind":"source-fragment","text":row["text"],"assetIds":ids}),
        );
    }
    // Full adopted designs can supply reference intent without resending an
    // unrelated screenplay. Dependency IDs limit attribution, not media choice.
    for a in assets.iter().filter(|a| a["id"] == asset["id"]) {
        if let Some(full) = design_full(a).filter(|_| validate_design(a).is_ok()) {
            let mut ids = vec![a["id"].clone()];
            for d in dependencies(a).unwrap_or(&[]) {
                ids.push(json!(dependency_id(d)));
            }
            for key in ["sourceAssetId", "parentAssetId", "baseAssetId"] {
                if assets.iter().any(|target| target["id"] == a[key]) {
                    ids.push(a[key].clone());
                }
            }
            media_evidence.push(json!({"id":format!("design:{}:{}",s(a,"id"),design_revision(a)),"kind":"adopted-design", "text":body(full),"assetIds":ids}));
        }
    }
    (candidates, media_evidence)
}

fn media_plan_schema(layouts: &[&str], candidates: &[Value], evidence: &[Value]) -> Value {
    let ids: Vec<_> = candidates.iter().map(|a| a["id"].clone()).collect();
    let evidence_ids: Vec<_> = evidence.iter().map(|e| e["id"].clone()).collect();
    let enum_or_string = |values: Vec<Value>| {
        if values.is_empty() {
            super::string()
        } else {
            json!({"type":"string","enum":values})
        }
    };
    super::obj(
        json!({"version":{"type":"string","enum":[MEDIA_PLAN_VERSION]},"mode":{"type":"string","enum":["preserve","plan"]},
        "imageTask":super::obj(json!({"kind":{"type":"string","enum":layouts}})),"rationale":super::string(),
        "referenceRequirements":super::array(super::obj(json!({"targetAssetId":enum_or_string(ids),"targetVariantId":super::string(),
            "role":{"type":"string","enum":["face","identity","identity-faces","clothing","appearance","scene","topology","state","viewpoint","opening","prop","reference"]},
            "mustPreserve":super::array(super::string()),"mayChange":super::array(super::string()),
            "evidence":super::array(super::obj(json!({"evidenceId":enum_or_string(evidence_ids),"quote":super::string()})))})))}),
    )
}

fn compile_media_plan(
    request: &TextRequest,
    asset: &Value,
    output: &Value,
) -> Result<Option<Value>, String> {
    if request.input["assetPromptRequestVersion"] != REQUEST_V2 {
        return Ok(None);
    }
    let plan = &output["mediaPlan"];
    if plan["version"] != MEDIA_PLAN_VERSION
        || plan["imageTask"]["kind"] != output["layoutKind"]
        || s(plan, "rationale").trim().is_empty()
        || !plan["referenceRequirements"].is_array()
    {
        return Err("媒体计划缺版本、明确理由或与图片版式不符".into());
    }
    let existing = array(&request.input, "referenceRequirements");
    let candidates = array(&request.input, "mediaCandidates");
    let mut requirements = Vec::new();
    match s(plan, "mode") {
        "preserve" if plan["referenceRequirements"].as_array().unwrap().is_empty() => {
            requirements = existing.to_vec()
        }
        "plan" if existing.is_empty() => {
            let mut seen = HashSet::new();
            for reference in array(plan, "referenceRequirements") {
                let target = candidates
                    .iter()
                    .find(|a| a["id"] == reference["targetAssetId"])
                    .ok_or("媒体计划目标不在冻结目录中")?;
                let role = s(reference, "role");
                if target["id"] == asset["id"]
                    || reference["targetVariantId"] != target["variantId"]
                    || !seen.insert((s(target, "id"), role))
                {
                    return Err("媒体计划引用自身、形象错配或重复".into());
                }
                let identity_role = ["face", "identity", "identity-faces"].contains(&role);
                if identity_role
                    && (category(asset) != "character"
                        || category(target) != "character"
                        || !same_identity(asset, target))
                {
                    return Err("身份母图缺少明确同一身份依据；未从名称或旧父资产字段猜锁脸".into());
                }
                if ["topology", "state", "viewpoint"].contains(&role)
                    && (category(asset) != "scene"
                        || category(target) != "scene"
                        || !(same_identity(asset, target)
                            || asset["sourceAssetId"] == target["id"]
                            || asset["parentAssetId"] == target["id"]))
                {
                    return Err("场景媒体计划缺少明确同一场景/拓扑依据".into());
                }
                if ![
                    "face",
                    "identity",
                    "identity-faces",
                    "clothing",
                    "appearance",
                    "scene",
                    "topology",
                    "state",
                    "viewpoint",
                    "opening",
                    "prop",
                    "reference",
                ]
                .contains(&role)
                    || array(reference, "mustPreserve").is_empty()
                    || !reference["mayChange"].is_array()
                    || ["mustPreserve", "mayChange"].iter().any(|key| {
                        array(reference, key)
                            .iter()
                            .any(|v| v.as_str().is_none_or(|v| v.trim().is_empty()))
                    })
                {
                    return Err("媒体计划缺少真实职责或保持/改变项".into());
                }
                let citations = array(reference, "evidence");
                if citations.is_empty() {
                    return Err("媒体计划缺少实际来源或完整设计证据".into());
                }
                for citation in citations {
                    let evidence = array(&request.input, "mediaEvidence")
                        .iter()
                        .find(|e| e["id"] == citation["evidenceId"])
                        .ok_or("媒体计划证据不属于冻结取料")?;
                    let quote = s(citation, "quote").trim();
                    if quote.is_empty()
                        || !s(evidence, "text").contains(quote)
                        || !array(evidence, "assetIds").contains(&asset["id"])
                        || !array(evidence, "assetIds").contains(&target["id"])
                    {
                        return Err("媒体计划引用不是当前与目标资产的精确共同证据".into());
                    }
                }
                let mut accepted = reference.clone();
                accepted["required"] = json!(true);
                requirements.push(accepted);
            }
        }
        _ => return Err("已有明确媒体计划只能完整复用，不得由写词静默改写".into()),
    }
    let mut parent_evidence = Vec::new();
    for reference in &requirements {
        let target = candidates
            .iter()
            .find(|a| a["id"] == reference["targetAssetId"])
            .ok_or("原媒体计划目标已缺失")?;
        if reference
            .get("targetVariantId")
            .filter(|v| !v.is_null())
            .is_some_and(|v| v != &target["variantId"])
        {
            return Err("原媒体计划形象版本已变化".into());
        }
        parent_evidence.push(json!({"assetId":target["id"],"variantId":target["variantId"],"fingerprint":target["evidenceFingerprint"]}));
    }
    let image_task = request.input["asset"]["imageTask"]
        .as_object()
        .filter(|_| request.input["asset"]["imageTask"]["kind"] == plan["imageTask"]["kind"])
        .map(|_| request.input["asset"]["imageTask"].clone())
        .unwrap_or_else(|| plan["imageTask"].clone());
    Ok(Some(
        json!({"version":MEDIA_PLAN_VERSION,"origin":"asset-skill-1.1.2","status":"ready",
        "mode":plan["mode"],"rationale":plan["rationale"],"imageTask":image_task,"referenceRequirements":requirements,
        "inputReferenceRequirements":request.input["referenceRequirements"],"inputImageTask":request.input["asset"]["imageTask"],
        "sourceKind":request.input["sourceKind"],"sourceSha256":request.input["sourceSha256"],"parentEvidence":parent_evidence,
        "requestFingerprint":request_fingerprint(request)?,"imageEvidence":"NO_PIXELS_READ","generationSubmissions":0}),
    ))
}

fn prompt_schema(asset: &Value, evidence: &[Value]) -> Value {
    let layouts: &[&str] = match category(asset) {
        "character" => &["single-character", "costume", "nonhuman", "group"],
        "scene" => &["scene", "opening"],
        _ => &["prop", "system-visual"],
    };
    let ids: Vec<_> = evidence.iter().map(|row| s(row, "id").to_owned()).collect();
    super::obj(
        json!({"assetId":{"type":"string","enum":[asset["id"]]},"variantId":{"type":"string","enum":[asset["variantId"]]},"promptText":super::string(),"designSupplement":super::string(),"layoutKind":{"type":"string","enum":layouts},"sourceRefs":super::refs(&ids),"imageEvidence":{"type":"string","enum":["NO_PIXELS_READ"]},"unresolved":super::array(super::string())}),
    )
}

fn costume_prompt_context(assets: &[Value], references: &[Value]) -> Value {
    let mothers: Vec<_> = references.iter().filter(|r| {
        ["identity", "face", "identity-faces"].contains(&s(r,"role"))
    }).map(|reference| {
        let mother = assets.iter().find(|a| a["id"] == reference["targetAssetId"]);
        let supplied = mother.map(|a| &a["identityLayoutSupply"]).unwrap_or(&Value::Null);
        let target_variant = reference.get("targetVariantId").filter(|v|!v.is_null());
        let registered = mother.is_some_and(|a| {
            supplied["version"] == "asset-inspected-layout-supply/v1" && supplied["status"] == "registered"
                && supplied["assetId"] == a["id"] && supplied["variantId"] == a["variantId"]
                && target_variant.is_none_or(|v|v == &a["variantId"])
                && supplied["mediaId"] == a["adoptedMediaId"] && !s(a,"adoptedMediaId").is_empty()
                && s(supplied,"sha256").len() == 64 && s(supplied,"sha256").bytes().all(|b|b.is_ascii_hexdigit())
                && supplied["fileBytesAndDimensionsVerified"] == true && supplied["imageEvidence"] == "NO_PIXELS_READ"
                && supplied["visualQualityApproved"] == false
                && supplied["identityCopyMode"] == "full-width-strip-original-pixels"
                && supplied["individualFaceSlotsRequired"] == false
                && crate::asset_composite::lower_adaptation_contract(supplied).is_ok()
        });
        json!({"targetAssetId":reference["targetAssetId"],"targetVariantId":reference["targetVariantId"],
            "registeredLayout":if registered{supplied.clone()}else{Value::Null},
            "dispatchReadiness":if registered{"registered-layout-recheck-before-send"}else{"awaiting-mother-or-inspected-layout"}})
    }).collect();
    json!({"version":COSTUME_PROMPT_V1,"phase":"prompt-authoring","promptMayProceedWithoutLayout":true,
        "layoutBindingStage":"image-dispatch","identityCopyMode":"full-width-strip-original-pixels",
        "individualFaceSlotsRequired":false,"motherReferences":mothers,
        "compositionRuntime":crate::asset_composite::LOWER_ADAPTIVE_PROFILE,
        "imageEvidence":"NO_PIXELS_READ","visualQualityApproved":false})
}

fn build_request(
    input: &Value,
    asset: &Value,
    item: &Value,
    model: &str,
    reasoning: &str,
) -> Result<TextRequest, String> {
    let source = &input["imageSource"];
    if input.get("costumePromptLayoutContract").filter(|v|!v.is_null()).is_some_and(|v|v!=COSTUME_PROMPT_V1) {
        return Err("未知换装编词/派发布局合同；保留原冻结请求".into());
    }
    if input.get("assetPromptSourceContract").filter(|v|!v.is_null()).is_some_and(|v|v!=SOURCE_BINDING_V1) {
        return Err("未知提示词来源合同；保留原冻结请求".into());
    }
    let assets = input["assets"].as_array().ok_or("缺少资产目录")?;
    let planning = input["assetPromptRequestVersion"] == REQUEST_V2;
    let projected = if planning {
        planning_input(asset)
    } else {
        asset.clone()
    };
    let asset = &projected;
    if input["assetPromptSourceContract"]==SOURCE_BINDING_V1
        && matches!(s(asset,"designStatus"),"needs_review"|"stale"|"rejected"|"invalid") {
        return Err(format!("{} 的采用设计仍待复核",s(asset,"id")));
    }
    validate_design(asset)?;
    let package=supplied_package(input,asset)?;
    let parents = if let Some(package)=package {
        let original=&asset["handoffRegistryAsset"];
        let registry=array(&package["registry"],"assets");
        if !registry.contains(original){return Err("资产原设计不属于该完整包的同版总库".into());}
        // A later explicitly adopted local design owns its own dependency
        // versions; a newly attached package must not roll that design back.
        let use_original=design_revision(asset)==design_revision(original) && design_full(asset)==design_full(original);
        let mut parents=if use_original {parent_designs(original,registry)?}else{parent_designs(asset,assets)?};
        for parent in &mut parents {parent["readOnly"]=json!(true);parent["sourcePackageFingerprint"]=package["proof"]["packageFingerprint"].clone();}
        parents
    } else {parent_designs(asset, assets)?};
    let references = references(asset, assets)?;
    let evidence = evidence_for(source, asset);
    if design_full(asset).is_none() && evidence.is_empty() {
        return Err("缺少该资产的完整采用设计或精确来源片段；仅名称不能作为设计依据".into());
    }
    let rule_snapshot = rule_snapshot(asset);
    let mut asset_card = json!({"id":asset["id"],"variantId":asset["variantId"],"name":asset["name"],"kind":kind(asset),"identityId":asset.get("identityId").or_else(||asset.get("baseAssetId")),"description":asset["description"],"sourceFacts":asset["sourceFacts"],"adoptedDesign":asset["adoptedDesign"],"designRevision":design_revision(asset),"imageTask":asset["imageTask"],"subjectForm":asset["subjectForm"],"uses":item["uses"],"filename":item["filename"]});
    let visual = package.map(|p|p["direction"]["adoptedDirection"].clone()).filter(|v|!v.is_null()).unwrap_or_else(||input
        .get("visualDirection")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| super::asset_design_contract::production_direction(input)));
    let style_settings=package.map(|p|&p["direction"]["settings"]).unwrap_or(&input["settings"]);
    if planning {
        for key in [
            "identityId",
            "baseAssetId",
            "parentAssetId",
            "sourceAssetId",
            "sourceVariantId",
        ] {
            asset_card[key] = asset[key].clone();
        }
    }
    let mut domain = json!({"workflowProfile":PROFILE,"sourceKind":source["sourceKind"],"sourceSha256":source["sha256"],"documentId":source["documentId"],"handoffSha256":source["handoffSha256"],"asset":asset_card,"evidence":evidence,"parentDesigns":parents,"referenceRequirements":references,"imagePresentation":presentation(default_layout(asset)),"visualDirection":visual,"visualStyle":super::asset_design_contract::project_visual_style(style_settings),"imageEvidence":"NO_PIXELS_READ","instruction":CONTRACT});
    if let Some(package)=package {domain["sourcePackageProof"]=package["proof"].clone();}
    let mut schema = prompt_schema(asset, &evidence);
    if planning {
        let (candidates, media_evidence) = media_context(asset, assets, &evidence);
        let layouts: Vec<_> = schema["properties"]["layoutKind"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let plan_schema = media_plan_schema(&layouts, &candidates, &media_evidence);
        schema["properties"]["mediaPlan"] = plan_schema;
        schema["required"]
            .as_array_mut()
            .ok_or("提示词schema缺少required")?
            .push(json!("mediaPlan"));
        domain["assetPromptRequestVersion"] = json!(REQUEST_V2);
        domain["mediaCandidates"] = json!(candidates);
        domain["mediaEvidence"] = json!(media_evidence);
        domain["instruction"] = json!(format!("{CONTRACT}\n\n{MEDIA_PLAN_CONTRACT}"));
    }
    if input["assetPromptSourceContract"]==SOURCE_BINDING_V1 {
        let served=array(&asset["referenceRegistration"],"serves");
        let used_units=array(item,"uses");
        let bindings:Vec<_>=array(input,"sourceUnitBindings").iter().filter(|binding| {
            array(binding,"assetIds").contains(&asset["id"])
                || served.iter().any(|v|array(binding,"assetIds").contains(&v["assetId"]))
                || used_units.iter().any(|v|v.get("unit").or_else(||v.get("unitId"))==Some(&binding["unitId"]))
        }).cloned().collect();
        domain["sourceBindingContract"]=json!(SOURCE_BINDING_V1);
        domain["sourceUnitBindings"]=json!(bindings);
    }
    if input["costumePromptLayoutContract"] == COSTUME_PROMPT_V1 && default_layout(asset) == "costume" {
        domain["costumePromptLayoutContract"] = json!(COSTUME_PROMPT_V1);
        domain["costumeComposition"] = costume_prompt_context(assets, &references);
        domain["instruction"] = json!(format!("{}\n\n{COSTUME_PROMPT_CONTRACT}",s(&domain,"instruction")));
    }
    Ok(TextRequest {
        stage: "assetDesign".into(),
        skill: "gpt-image25-asset-workflow".into(),
        model: model.into(),
        reasoning: reasoning.into(),
        schema,
        input: domain,
        rule_snapshot,
    })
}

fn request_fingerprint(request: &TextRequest) -> Result<String, String> {
    let mut domain = request.clone();
    // A proven stopped transport may add recovery metadata without changing
    // the authored domain. It must not invalidate the completed prompt cache.
    if let Some(input) = domain.input.as_object_mut() {
        input.remove("textTransportRetry");
        // Geometry is re-bound by the image dispatcher. Changing an adopted
        // mother/layout must not repurchase already complete clothing words.
        if input.get("costumePromptLayoutContract") == Some(&json!(COSTUME_PROMPT_V1)) {
            for mother in input.get_mut("costumeComposition").and_then(|v|v.get_mut("motherReferences"))
                .and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(fields)=mother.as_object_mut(){fields.remove("registeredLayout");fields.remove("dispatchReadiness");}
            }
        }
        if input.get("sourceBindingContract")==Some(&json!(SOURCE_BINDING_V1)) {
            // Whole-document provenance is retained in the actual request, but
            // unrelated unit edits must not repurchase unchanged asset words.
            input.remove("sourceSha256");
            input.remove("handoffSha256");
            for row in input.get_mut("evidence").and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(object)=row.as_object_mut(){object.remove("unitVersion");}
            }
        }
    }
    serde_json::to_string(&domain)
        .map(|value| digest(&value))
        .map_err(|e| e.to_string())
}

fn source_basis(request:&TextRequest)->Value {
    let input=&request.input;
    let mut evidence=input["evidence"].clone();
    for row in evidence.as_array_mut().into_iter().flatten() {
        if let Some(object)=row.as_object_mut(){object.remove("unitVersion");}
    }
    let basis=json!({"sourceKind":input["sourceKind"],"sourceUnitBindings":input["sourceUnitBindings"],
        "evidence":evidence,"asset":input["asset"],"parentDesigns":input["parentDesigns"],
        "visualDirection":input["visualDirection"],"visualStyle":input["visualStyle"],
        "referenceRequirements":input["referenceRequirements"]});
    json!({"sourceBindingContract":SOURCE_BINDING_V1,"sourceUnitBindings":input["sourceUnitBindings"],
        "sourceKind":input["sourceKind"],"sourceSha256":input["sourceSha256"],
        "sourceBasisSha256":digest(&basis.to_string())})
}

/// Local explicit editing/review freezes current evidence without a model call.
pub(crate) fn manual_prompt_basis(input:&Value,asset:&Value)->Result<Value,String> {
    let item=array(&input["imageQueue"],"items").iter().find(|i|i["assetId"]==asset["id"])
        .ok_or("该资产不在当前正式图片或已登记参考范围")?;
    let supplied=array(input,"assets").iter().find(|a|a["id"]==asset["id"]&&a["variantId"]==asset["variantId"])
        .ok_or("当前提示词资产供料缺失")?;
    if supplied["version"]!=asset["version"] {return Err("当前提示词资产版本已变化".into());}
    let request=build_request(input,supplied,item,"manual-edit","medium")?;
    if request.input["sourceBindingContract"]!=SOURCE_BINDING_V1 {return Err("手工复核需要当前来源合同".into());}
    let mut basis=source_basis(&request);
    basis["basedOnDesignRevision"]=design_revision(asset);
    basis["basedOnDesignHash"]=json!(design_full(asset).map(|v|digest(&body(v))));
    basis["visualDirectionVersion"]=request.input["visualDirection"]["version"].clone();
    Ok(basis)
}

fn finalize_output(request: &TextRequest, asset: &Value, output: &Value) -> Result<Value, String> {
    if output["assetId"] != asset["id"]
        || output["variantId"] != asset["variantId"]
        || output["imageEvidence"] != "NO_PIXELS_READ"
    {
        return Err("提示词回包身份或图片证据声明不匹配".into());
    }
    let unresolved = output["unresolved"]
        .as_array()
        .ok_or("提示词回包缺少待核列表")?;
    if unresolved
        .iter()
        .any(|v| v.as_str().is_none_or(|s| s.trim().is_empty()))
    {
        return Err("提示词待核原因必须为非空文字".into());
    }
    let known: HashSet<_> = array(&request.input, "evidence")
        .iter()
        .map(|v| s(v, "id"))
        .collect();
    let refs = output["sourceRefs"]
        .as_array()
        .ok_or("提示词回包缺少来源引用")?;
    let mut seen = HashSet::new();
    if refs.iter().any(|v| {
        v.as_str()
            .is_none_or(|id| !known.contains(id) || !seen.insert(id))
    }) {
        return Err("提示词引用越出当前来源或重复".into());
    }
    if !unresolved.is_empty() {
        return Ok(
            json!({"assetId":asset["id"],"variantId":asset["variantId"],"status":"blocked","reasons":unresolved}),
        );
    }
    let prompt = s(output, "promptText").trim();
    let supplement = s(output, "designSupplement").trim();
    if prompt.is_empty() || prompt.chars().count() > 14_000 || supplement.chars().count() > 20_000 {
        return Err("提示词或补料正文为空/过长".into());
    }
    if design_full(asset).is_none() && (supplement.is_empty() || refs.is_empty()) {
        return Err("无完整设计时必须给出有当前来源依据的必要设计补料".into());
    }
    if design_full(asset).is_some() && !supplement.is_empty() {
        return Err("已采用完整设计不得在第三阶段重新编写或覆盖".into());
    }
    let layout = s(output, "layoutKind");
    let media_plan = compile_media_plan(request, asset, output)?;
    let constraint_asset = if media_plan.is_some() {
        planning_input(asset)
    } else {
        asset.clone()
    };
    let allowed = &request.schema["properties"]["layoutKind"]["enum"];
    if !allowed
        .as_array()
        .is_some_and(|v| v.iter().any(|v| v == layout))
        || !explicit_layout(&constraint_asset).is_empty()
            && layout != explicit_layout(&constraint_asset)
    {
        return Err("提示词版式与资产类别或已确认任务冲突".into());
    }
    if layout == "costume"
        && !array(
            media_plan.as_ref().unwrap_or(&request.input),
            "referenceRequirements",
        )
        .iter()
        .any(|r| ["face", "identity", "identity-faces"].contains(&s(r, "role")))
    {
        return Err("换装合成缺少显式身份母图需求；未从中文名猜父图".into());
    }
    let presentation = presentation(layout);
    let style = s(&request.input, "visualStyle");
    let mut parts = Vec::new();
    if !style.is_empty() && !prompt.contains(style) {
        parts.push(style.to_owned());
    }
    parts.push(prompt.to_owned());
    let composition = s(&presentation, "composition");
    if !prompt.contains(composition) {
        parts.push(composition.to_owned());
    }
    let fingerprint = request_fingerprint(request)?;
    let mut result = asset.clone();
    result["workflowProfile"] = json!(PROFILE);
    result["imagePresentation"] = presentation;
    result["imagePrompt"] = json!({"version":PROMPT_VERSION,"text":parts.join("\n\n"),"origin":"AI","sourceKind":request.input["sourceKind"],"sourceSha256":request.input["sourceSha256"],"sourceRefs":refs,"basedOnDesignRevision":design_revision(asset),"basedOnDesignHash":design_full(asset).map(|v|digest(&body(v))),"visualDirectionVersion":request.input["visualDirection"].get("version"),"templateVersion":"1.1.2","layoutKind":layout,"inputFingerprint":fingerprint,"ruleSnapshotSha256":digest(&request.rule_snapshot.to_string()),"imageEvidence":"NO_PIXELS_READ","status":"ready"});
    if request.input["sourceBindingContract"]==SOURCE_BINDING_V1 {
        for (key,value) in source_basis(request).as_object().unwrap(){result["imagePrompt"][key]=value.clone();}
    }
    result["promptStatus"] = json!("ready");
    result["referenceRequirements"] = request.input["referenceRequirements"].clone();
    if let Some(plan) = media_plan {
        result["referenceRequirements"] = plan["referenceRequirements"].clone();
        result["imageTask"] = plan["imageTask"].clone();
        result["mediaPlan"] = plan;
    }
    if !supplement.is_empty() {
        result["designCandidate"] = json!({"version":"asset-design-supplement/v1","full":supplement,"sourceKind":request.input["sourceKind"],"sourceSha256":request.input["sourceSha256"],"sourceRefs":refs,"state":"candidate","adopted":false});
    }
    Ok(json!({"status":"ready","asset":result}))
}

/// Visual review consumes the same exact source/design selection as prompt
/// writing. It does not receive unrelated episodes or issue a text request.
pub(crate) fn visual_review_material(input:&Value,asset:&Value)->Result<Value,String>{
    let item=array(&input["imageQueue"],"items").iter().find(|i|i["assetId"]==asset["id"])
        .ok_or("视觉审核资产不在当前明确图片范围")?;
    let supplied=array(input,"assets").iter().find(|a|a["id"]==asset["id"]&&a["variantId"]==asset["variantId"])
        .ok_or("视觉审核的同版资产供料缺失")?;
    if supplied["version"]!=asset["version"]{return Err("视觉审核资产版本变化".into());}
    let request=build_request(input,supplied,item,"visual-review-material","medium")?;
    let mut evidence=request.input["evidence"].clone();
    for row in evidence.as_array_mut().into_iter().flatten(){if let Some(fields)=row.as_object_mut(){fields.remove("unitVersion");}}
    Ok(json!({"version":"asset-visual-source/v1","sourceKind":request.input["sourceKind"],"documentId":request.input["documentId"],
        "sourceUnitBindings":request.input["sourceUnitBindings"],"evidence":evidence,"asset":request.input["asset"],
        "parentDesigns":request.input["parentDesigns"],"visualDirection":request.input["visualDirection"],"visualStyle":request.input["visualStyle"],
        "imagePresentation":request.input["imagePresentation"],"referenceRequirements":request.input["referenceRequirements"]}))
}

async fn execute_prompt(
    executor: &dyn TextExecutor,
    request: TextRequest,
    asset: Value,
) -> Result<Value, String> {
    let mut request = executor.prepare_transport_request(request).await?;
    let output = match executor.execute(request.clone()).await {
        Ok(value) => value,
        Err(error) => {
            let next = executor.prepare_transport_request(request.clone()).await?;
            if serde_json::to_value(&next).map_err(|e| e.to_string())?
                == serde_json::to_value(&request).map_err(|e| e.to_string())?
            {
                return Err(error);
            }
            request = next;
            executor.execute(request.clone()).await?
        }
    };
    match finalize_output(&request, &asset, &output) {
        Ok(finalized) => {
            executor.accept(&request, &output).await?;
            Ok(finalized)
        }
        Err(reason) => {
            executor.reject(&request, &output, &reason).await?;
            Err(reason)
        }
    }
}

pub async fn run_asset_prompts(
    input: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    if !enabled(input)? {
        return Err("新资产提示词流程缺少明确工作流版本".into());
    }
    if input
        .get("assetPromptRequestVersion")
        .filter(|v| !v.is_null())
        .is_some_and(|v| v != REQUEST_V2)
    {
        return Err("未知写词请求版本；未改写旧冻结请求".into());
    }
    if model.trim().is_empty()
        || !["low", "medium", "high", "xhigh", "max", "ultra"].contains(&reasoning)
    {
        return Err("请选择有效文本模型与实际支持的 reasoning 参数".into());
    }
    validate_source(&input["imageSource"])?;
    let assets = input["assets"].as_array().ok_or("缺少资产目录")?;
    let mut by_id = HashMap::new();
    for asset in assets {
        if s(asset, "id").is_empty()
            || s(asset, "variantId").is_empty()
            || by_id.insert(s(asset, "id"), asset).is_some()
        {
            return Err("资产身份缺失或重复".into());
        }
    }
    let queue = &input["imageQueue"];
    if queue["version"] != "desktop-image-queue/v1" {
        return Err("新提示词流程只接受显式图片范围，不递归扩出祖先任务".into());
    }
    let items = queue["items"].as_array().ok_or("图片范围缺少 items")?;
    let mut item_ids = HashSet::new();
    for item in items {
        let id = s(item, "assetId");
        let asset = by_id
            .get(id)
            .ok_or_else(|| format!("图片范围资产不存在：{id}"))?;
        if !item_ids.insert(id)
            || item["variantId"] != asset["variantId"]
            || s(item, "filename").is_empty()
            || !item["uses"].is_array()
        {
            return Err(format!("图片范围身份、版本、文件名或用途无效：{id}"));
        }
    }
    let selected = input["selection"]["assetIds"].as_array();
    if let Some(ids) = selected {
        if ids
            .iter()
            .any(|id| id.as_str().is_none_or(|id| !item_ids.contains(id)))
        {
            return Err("所选资产不在本次显式图片范围，未自动扩项".into());
        }
    }
    let mut completed = Vec::new();
    let mut blocked = Vec::new();
    let mut skipped = Vec::new();
    let mut pending = VecDeque::new();
    for (index, item) in items.iter().enumerate() {
        if selected.is_some_and(|ids| !ids.contains(&item["assetId"])) {
            continue;
        }
        let asset = by_id[s(item, "assetId")];
        if category(asset) == "audio"
            || asset["imageRequired"] == false
            || matches!(s(asset, "purpose"), "voice-only" | "mention-only")
            || matches!(
                asset.pointer("/preparation/mode").and_then(Value::as_str),
                Some("AUDIO" | "INLINE")
            )
        {
            skipped.push(
                json!({"assetId":asset["id"],"reason":"非正式图片项，不创建图片提示词或生成任务"}),
            );
            continue;
        }
        match build_request(input, asset, item, model, reasoning) {
            Ok(request) => {
                let hash = request_fingerprint(&request)?;
                if asset["imagePrompt"]["version"] == PROMPT_VERSION
                    && asset["imagePrompt"]["inputFingerprint"] == hash
                    && !s(&asset["imagePrompt"], "text").trim().is_empty()
                {
                    completed.push((index, asset.clone()));
                } else if matches!(s(&asset["imagePrompt"], "origin"), "MANUAL" | "manual")
                    && !s(&asset["imagePrompt"], "text").trim().is_empty()
                    && input["overwriteManualPrompts"] != true
                {
                    let mut preserved = asset.clone();
                    let changed = if request.input["sourceBindingContract"]==SOURCE_BINDING_V1 {
                        asset["imagePrompt"]["sourceBindingContract"]!=SOURCE_BINDING_V1
                            || asset["imagePrompt"]["sourceBasisSha256"]!=source_basis(&request)["sourceBasisSha256"]
                    } else {
                        asset["imagePrompt"]["sourceSha256"]!=input["imageSource"]["sha256"]
                            || asset["imagePrompt"]["basedOnDesignRevision"]!=design_revision(asset)
                    };
                    if changed {
                        preserved["promptStatus"] = json!("needs_review");
                        blocked.push(json!({"assetId":asset["id"],"reason":"手工提示词保留；其来源或设计依据已变化，需核对当前版本"}));
                    } else {
                        preserved["promptStatus"] = json!("ready");
                    }
                    completed.push((index, preserved));
                } else {
                    pending.push_back((index, asset.clone(), request));
                }
            }
            Err(reason) => blocked.push(
                json!({"assetId":asset["id"],"variantId":asset["variantId"],"reason":reason}),
            ),
        }
    }
    type Work<'a> =
        Pin<Box<dyn Future<Output = (usize, String, Result<Value, String>)> + Send + 'a>>;
    let mut active: Vec<Work<'_>> = Vec::new();
    while !pending.is_empty() || !active.is_empty() {
        while active.len() < CONCURRENCY {
            let Some((index, asset, request)) = pending.pop_front() else {
                break;
            };
            let id = s(&asset, "id").to_owned();
            active.push(Box::pin(async move {
                (index, id, execute_prompt(executor, request, asset).await)
            }));
        }
        let (slot, (index, id, result)) = poll_fn(|cx| {
            for (slot, future) in active.iter_mut().enumerate() {
                if let Poll::Ready(value) = future.as_mut().poll(cx) {
                    return Poll::Ready((slot, value));
                }
            }
            Poll::Pending
        })
        .await;
        drop(active.remove(slot));
        match result{
            Ok(value) if value["status"]=="ready"=>completed.push((index,value["asset"].clone())),
            Ok(value)=>blocked.push(json!({"assetId":id,"reason":array(&value,"reasons").iter().filter_map(Value::as_str).collect::<Vec<_>>().join("；"),"unresolved":value["reasons"]})),
            Err(reason)=>blocked.push(json!({"assetId":id,"reason":reason})),
        }
    }
    completed.sort_by_key(|(index, _)| *index);
    let mut updated: Vec<_> = completed.into_iter().map(|(_, asset)| asset).collect();
    if input["assetPromptRequestVersion"] == REQUEST_V2 {
        let mut projected = assets.clone();
        for value in &updated {
            if let Some(a) = projected.iter_mut().find(|a| a["id"] == value["id"]) {
                *a = value.clone();
            }
        }
        let selection: Vec<_> = updated.iter().map(|a| a["id"].clone()).collect();
        let (_, invalid) = crate::asset_graph::explicit_order(&projected, &selection);
        for issue in invalid {
            if let Some(asset) = updated.iter_mut().find(|a| a["id"] == issue["assetId"]) {
                if asset["mediaPlan"].is_object() {
                    asset["mediaPlan"]["status"] = json!("needs_review");
                    asset["mediaPlan"]["issues"] = json!([issue["reason"]]);
                }
            }
            blocked.push(issue);
        }
    }
    let plans:Vec<_>=updated.iter().map(|asset|{
        let missing:Vec<_>=array(asset,"referenceRequirements").iter().filter(|r|r["required"]!=false&&by_id.get(s(r,"targetAssetId")).is_none_or(|a|s(a,"adoptedMediaId").is_empty()||crate::asset_graph::image_adoption_blocked(a)||crate::asset_graph::design_review_blocked(a))).map(|r|r["targetAssetId"].clone()).collect();
        let prompt_state=asset["promptStatus"].as_str().unwrap_or("ready");
        json!({"assetId":asset["id"],"variantId":asset["variantId"],"promptStatus":prompt_state,"status":if prompt_state!="ready"||asset["mediaPlan"]["status"]=="needs_review"{"needs_review"}else if missing.is_empty(){"ready"}else{"waiting_reference"},"missingReferences":missing,"prompt":prompt_text(asset),"imagePresentation":image_presentation(asset),"requestFrozen":false})
    }).collect();
    Ok(
        json!({"schemaVersion":"asset-prompt-batch/v1","workflowProfile":PROFILE,"sourceKind":input["imageSource"]["sourceKind"],"sourceSha256":input["imageSource"]["sha256"],"status":if blocked.is_empty(){"completed"}else{"partially_blocked"},"assets":updated,"blocked":blocked,"skipped":skipped,"imagePlan":plans,"promptCount":updated.len(),"imageGenerationSubmissions":0,"imageEvidence":"NO_PIXELS_READ"}),
    )
}

#[cfg(test)]
#[path = "asset_workflow_tests.rs"]
mod tests;
