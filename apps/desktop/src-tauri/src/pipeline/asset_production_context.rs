//! New Stage2 requests get scene/state context and narrowly proven prop design
//! candidates. These are production instructions, never ownership/visual facts.
use super::{asset_design_contract as contract, assets, digest, LockedSource};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub const LEGACY_PROFILE: &str = "asset-production-context/v1";
pub const PREVIOUS_PROFILE: &str = "asset-production-context/v2";
pub const PROFILE: &str = "asset-production-context/v3";
pub const PROFILE_FIELD: &str = "stage2ProductionContext";
pub const LEGACY_INSTRUCTIONS: &str = "本附加上下文只指导原C基础/D派生岗位的普通制作判断，不新增模型岗位或事实。applicableScenes是当前形象可复用的准确场次定位，factEvidenceScopes标明每条原facts的证据场次和背景行；原facts/claim/variantId与原已有evidence全部保留，不因属性词或出现次数删改。只有多场共用LOC在evidence额外提供最初适用场的完整原文，其它适用场仅为定位元数据，未声称提供整场文字。核对facts引用行及实际evidence：某一场有状态证据不等于它在所有适用场持续成立，区分身份/空间/物件的稳定设计与现场状态、时段、事件和后续变化。多场共用基础稿以可复用身份/结构为主，明确变化留给对应场次的表演/分镜；初始条件未说明时取最保守且不预支后续剧情的中性可用方案，不补造相反事实，取舍写入freeDesignScope。relatedProps只因已采用A15的SOURCE事实引用了同一非标题原文行而成为候选，不证明持有者、同一物件或长期佩戴；必须核对所给共享原行确认是否当前人物在当前形象使用同一件。只有acceptedDesign非空且关系有原文依据时，复用该道具已接受的形制、材质、颜色和标识，不再次定义同物件美术；这是文字一致性约束，不提升为SOURCE事实。没有有效设计或关系不明确时，只保留原文必要的持有/佩戴关系，不创造独立物件造型，也不推断同场其它主体的物品属于当前人物。候选道具不是parentDesign，不改变直接父形象继承、referenceVariantIds、图片引用或生成数量；差异与不确定性写入freeDesignScope，无需新增确认岗。";
pub const PREVIOUS_INSTRUCTIONS: &str = "本附加上下文只指导原C基础/D派生岗位的普通制作判断，不新增模型岗位或事实。applicableScenes是当前形象可复用的准确场次定位，factEvidenceScopes标明每条原facts的证据场次和背景行；原facts/claim/variantId与原已有evidence全部保留，不因属性词或出现次数删改。只有多场共用LOC在evidence额外提供最初适用场的完整原文，其它适用场仅为定位元数据，未声称提供整场文字。核对facts引用行及实际evidence：某一场有状态证据不等于它在所有适用场持续成立，区分身份/空间/物件的稳定设计与现场状态、时段、事件和后续变化。多场共用基础稿以可复用身份/结构为主，明确变化留给对应场次的表演/分镜；初始条件未说明时取最保守且不预支后续剧情的中性可用方案，不补造相反事实，取舍写入freeDesignScope。relatedProps只因已采用A15的SOURCE事实引用了同一非标题原文行而成为候选，不证明持有者、同一物件或长期佩戴；必须核对所给共享原行确认是否当前人物在当前形象使用同一件。只有acceptedDesign非空且关系有原文依据时，复用该道具已接受的形制、材质、颜色和标识，不再次定义同物件美术；这是文字一致性约束，不提升为SOURCE事实。没有有效设计或关系不明确时，只保留原文必要的持有/佩戴关系，不创造独立物件造型，也不推断同场其它主体的物品属于当前人物。候选道具不是parentDesign，不改变直接父形象继承、referenceVariantIds、图片引用或生成数量；差异与不确定性写入freeDesignScope，无需新增确认岗。DERIVED继承父稿的稳定身份、脸型、眉眼鼻唇关系、体态以及适用于当前形象且未被原文明示改变的稳定造型。parentDesign.freeDesignScope逐字来自已接受父稿，designChoices仅证明该美术取舍的来源和版本，不是SOURCE事实。保持未变穿戴只针对当前适用的穿戴，不把父稿限定场次的配饰、携带物、动作或临时状态自动扩展成常驻身份。先对照当前asset.scope、applicableScenes、factEvidenceScopes和实际evidence，判断STATE是否在当前场成立或有原文连续事实支持；父稿明确的场次/用途范围限制须按原范围处理。原文明确在当前场沿用或变更时服从原文并说明取舍；没有连续依据时，不以没有写下摘除/放下为由跨越父稿已明确的范围限制，也不凭换装名称武断删去原文明确仍适用的穿戴。全部原facts仍供核对，不把未适用STATE重写为相反事实。差异或冲突写入当前freeDesignScope，不重设计稳定身份，不扩展直接父引用或图片生成数量。";
pub const INSTRUCTIONS: &str = "本附加上下文只指导原C基础/D派生岗位的普通制作判断，不新增模型岗位或事实。applicableScenes是当前形象可复用的准确场次定位，factEvidenceScopes标明每条原facts的证据场次和背景行；原facts/claim/variantId与原已有evidence全部保留，不因属性词或出现次数删改。只有多场共用LOC在evidence额外提供最初适用场的完整原文，其它适用场仅为定位元数据，未声称提供整场文字。核对facts引用行及实际evidence：某一场有状态证据不等于它在所有适用场持续成立，区分身份/空间/物件的稳定设计与现场状态、时段、事件和后续变化。多场共用基础稿以可复用身份/结构为主，明确变化留给对应场次的表演/分镜；初始条件未说明时取最保守且不预支后续剧情的中性可用方案，不补造相反事实，取舍写入freeDesignScope。relatedProps只因已采用A15的SOURCE事实引用了同一非标题原文行而成为候选，不证明持有者、同一物件或长期佩戴；必须核对所给共享原行确认是否当前人物在当前形象使用同一件。只有acceptedDesign非空且关系有原文依据时，复用该道具已接受的形制、材质、颜色和标识，不再次定义同物件美术；这是文字一致性约束，不提升为SOURCE事实。没有有效设计或关系不明确时，只保留原文必要的持有/佩戴关系，不创造独立物件造型，也不推断同场其它主体的物品属于当前人物。候选道具不是parentDesign，不改变直接父形象继承、referenceVariantIds、图片引用或生成数量；差异与不确定性写入freeDesignScope，无需新增确认岗。DERIVED继承父稿的稳定身份、脸型、眉眼鼻唇关系、体态以及适用于当前形象且未被原文明示改变的稳定造型。parentDesign.freeDesignScope逐字来自已接受父稿，designChoices仅证明该美术取舍的来源和版本，不是SOURCE事实。保持未变穿戴只针对当前适用的穿戴，不把父稿限定场次的配饰、携带物、动作或临时状态自动扩展成常驻身份。先对照当前asset.scope、applicableScenes、factEvidenceScopes和实际evidence，判断STATE是否在当前场成立或有原文连续事实支持；父稿明确的场次/用途范围限制须按原范围处理。原文明确在当前场沿用或变更时服从原文并说明取舍；没有连续依据时，不以没有写下摘除/放下为由跨越父稿已明确的范围限制，也不凭换装名称武断删去原文明确仍适用的穿戴。全部原facts仍供核对，不把未适用STATE重写为相反事实。差异或冲突写入当前freeDesignScope，不重设计稳定身份，不扩展直接父引用或图片生成数量。LOC与独立PROP的同物件美术协同：relatedProps同样可由当前场景和道具已接受A15 SOURCE事实共享非标题原行产生候选；同场、共行、名称相近或空间包含词均不由程序判定同一物件。逐条核对sharedSourceRefs实际原文，确认当前场景确实容纳或呈现该件已登记道具后，才复用acceptedDesign中的形制、朝向、材质、颜色和标识，不把场景里的同件雕塑/陈设重新自由设计成另一造型。多个候选逐项判断，不把一个候选的造型套给同场其它物件；无有效acceptedDesign或同物关系不清时，只保留原文必要空间/陈设关系，不补造该独立物件的造型，差异与不确定性写入freeDesignScope。候选仅是文字设计一致性输入，不是父资产、SOURCE事实或自动图片参考；不增加referenceVariantIds、图片引用、父链或生成数量。";
const PARENT_CHOICES_USAGE: &str = "ACCEPTED_PARENT_DESIGN_CHOICES_NOT_SOURCE_FACT";

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

/// Only the engine's new-workflow boundary calls this. No legacy upgrade on replay.
pub fn freeze(input: &mut Value) {
    input[PROFILE_FIELD] = json!({"version":PROFILE,"instructions":INSTRUCTIONS,"instructionsSha256":digest(INSTRUCTIONS)});
    if let Some(explicit) = input["selection"].get("explicitAssetIds") {
        // Copied inside the already-restored profile so GUI dependency closure
        // cannot turn an implicit v1 source into an explicitly requested design.
        input[PROFILE_FIELD]["explicitAssetIds"] = explicit.clone();
    }
}

fn profile(input: &Value) -> Result<Option<&Value>, String> {
    let Some(value) = input.get(PROFILE_FIELD).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if !valid_profile(value) {
        return Err("第二阶段原制作上下文版本/指令摘要无效；未重建旧请求".into());
    }
    Ok(Some(value))
}

fn valid_profile(value: &Value) -> bool {
    let instructions = match value["version"].as_str() {
        Some(LEGACY_PROFILE) => LEGACY_INSTRUCTIONS,
        Some(PREVIOUS_PROFILE) => PREVIOUS_INSTRUCTIONS,
        Some(PROFILE) => INSTRUCTIONS,
        _ => return false,
    };
    let exact_instructions = value["instructions"] == instructions
        && value["instructionsSha256"] == digest(instructions);
    if modern_profile(text(value, "version")) {
        if let Some(explicit) = value.get("explicitAssetIds") {
            let Some(ids) = explicit.as_array() else { return false; };
            let mut seen = HashSet::new();
            if ids.iter().any(|id| id.as_str().is_none_or(|id| id.is_empty() || !seen.insert(id))) {
                return false;
            }
        }
    }
    exact_instructions
}

pub(crate) fn frozen_version(input: &Value) -> Result<Option<&str>, String> {
    profile(input).map(|value| value.map(|value| text(value, "version")))
}

pub(crate) fn explicit_design_asset_ids(input: &Value) -> Result<Option<&[Value]>, String> {
    let Some(frozen) = profile(input)? else { return Ok(None); };
    if !modern_profile(text(frozen, "version")) { return Ok(None); }
    Ok(frozen.get("explicitAssetIds").and_then(Value::as_array).map(Vec::as_slice))
}

fn modern_profile(version: &str) -> bool {
    matches!(version, PREVIOUS_PROFILE | PROFILE)
}

fn prop_context_subject(input: &Value, asset: &Value) -> Result<bool, String> {
    Ok(contract::is_character(asset)
        || frozen_version(input)? == Some(PROFILE)
            && matches!(text(asset, "kind"), "LOC" | "location" | "场景"))
}

/// New children may read implicit older accepted parents under their exact
/// original contract. Explicit targets keep the newly frozen instructions.
pub(super) fn accepted_parent_input(input: &Value, asset: &Value) -> Result<Option<Value>, String> {
    let explicit = explicit_design_asset_ids(input)?
        .or_else(|| input["selection"]["assetIds"].as_array().map(Vec::as_slice));
    let current = frozen_version(input)?;
    let previous = text(&asset["design"]["designReceipt"]["basis"]["assetProductionContext"], "version");
    let compatible = current == Some(PREVIOUS_PROFILE) && previous == LEGACY_PROFILE
        || current == Some(PROFILE) && matches!(previous, LEGACY_PROFILE | PREVIOUS_PROFILE);
    if !compatible
        || explicit.is_none_or(|ids| ids.contains(&asset["id"]))
    {
        return Ok(None);
    }
    let original = &asset["design"]["frozenContext"]["assetProductionContext"];
    let original_basis = &asset["design"]["designReceipt"]["basis"];
    if !valid_profile(original) || original["version"] != previous
        || binding(original) != original_basis["assetProductionContext"]
    {
        return Err("父稿原冻结制作合同或收据不匹配，未升级或重发".into());
    }
    let mut basis = input.clone();
    basis[PROFILE_FIELD] = json!({"version":original["version"],"instructions":original["instructions"],"instructionsSha256":original["instructionsSha256"]});
    if original_basis.get("identityProductionMaterialsSha256").is_none()
        && original_basis.get("assetScopeProductionMaterialsSha256").is_none()
    {
        for key in [contract::IDENTITY_MATERIAL_PROFILE_FIELD,contract::IDENTITY_MATERIALS_FIELD,contract::SCOPE_MATERIALS_FIELD] {
            basis.as_object_mut().unwrap().remove(key);
        }
    }
    Ok(Some(basis))
}

fn parent_design_choice_proof(asset: &Value, design: &Value) -> Option<Value> {
    if asset["promptOrigin"] == "MANUAL"
        || design["schemaVersion"] != "asset-design/v1"
        || design["assetId"] != asset["id"]
        || design["variantId"] != asset["variantId"]
        || text(asset, "prompt").trim().is_empty()
        || design["prompt"] != asset["prompt"]
        || text(design, "sourceSha256").is_empty()
        || text(&design["designReceipt"], "sha256").is_empty()
    {
        return None;
    }
    let scope = design["freeDesignScope"].as_str()?;
    Some(json!({
        "assetId":asset["id"],"identityId":contract::identity(asset),"variantId":asset["variantId"],
        "assetSemanticVersion":digest(&contract::asset_semantics(asset).to_string()),
        "sourceSha256":design["sourceSha256"],"designReceiptSha256":design["designReceipt"]["sha256"],
        "designVersion":digest(&json!({"prompt":asset["prompt"],"receipt":design["designReceipt"]}).to_string()),
        "freeDesignScopeSha256":digest(scope),"usage":PARENT_CHOICES_USAGE
    }))
}

/// Called only with an accepted/current parent design, never a manual or stale
/// candidate. v1 keeps its exact seven-field parent card.
pub(super) fn reference_design_choices(
    input: &Value,
    asset: &Value,
    design: Option<&Value>,
) -> Result<Option<Value>, String> {
    if !frozen_version(input)?.is_some_and(modern_profile) || asset["promptOrigin"] == "MANUAL" {
        return Ok(None);
    }
    design.map(|design| {
        let proof = parent_design_choice_proof(asset, design)
            .ok_or("已接受父设计的制作取舍证据不完整，原稿保留")?;
        if proof["sourceSha256"] != input["source"]["sha256"] {
            return Err("父设计制作取舍不属于当前锁定来源，原稿保留".into());
        }
        Ok(proof)
    }).transpose()
}

fn parent_design_choices(input: &Value, asset: &Value) -> Result<Option<Value>, String> {
    let catalogue = input["assets"].as_array().ok_or("制作上下文缺少资产目录")?;
    let Some(parent) = assets::resolve_asset_dependency(catalogue, asset)? else {
        return Ok(None);
    };
    let by_id = catalogue.iter().map(|row| (text(row,"id"), row)).collect();
    let parents = catalogue.iter().filter_map(|row| {
        assets::resolve_asset_dependency(catalogue, row).ok()
            .map(|parent|(text(row,"id").to_owned(),parent))
    }).collect();
    let target = assets::inline_reference_target(&parent, &by_id, &parents)?;
    let saved = catalogue.iter().find(|row| text(row,"id") == target)
        .ok_or("已接受父设计身份缺失")?;
    let original = accepted_parent_input(input, saved)?;
    if !contract::design_receipt_current(original.as_ref().unwrap_or(input), saved) {
        return Ok(None);
    }
    reference_design_choices(input, saved, saved.get("design"))
}

fn selected_facts(
    input: &Value,
    asset: &Value,
    source: &LockedSource,
) -> Result<Vec<Value>, String> {
    let catalogue = input["assets"].as_array().ok_or("制作上下文缺少资产目录")?;
    let by_id = catalogue.iter().map(|row| (text(row, "id"), row)).collect();
    assets::resolve_asset_dependency(catalogue, asset)?;
    let parents = catalogue
        .iter()
        .filter_map(|row| {
            assets::resolve_asset_dependency(catalogue, row)
                .ok()
                .map(|parent| (text(row, "id").to_owned(), parent))
        })
        .collect();
    let known = source.rows.iter().map(|row| row.id.as_str()).collect();
    assets::design_facts(input, asset, &by_id, &parents, &known).map(|(facts, _)| facts)
}

/// Explicit applicability can exclude a future appearance, but an intersecting
/// scene never establishes ownership or creates a candidate by itself.
fn explicit_scene_scope(
    asset: &Value,
    source: &LockedSource,
) -> Option<BTreeSet<(String, String)>> {
    if asset["scope"]["kind"] == "GLOBAL" {
        return None;
    }
    if asset["scope"]["kind"] == "SCENES" {
        let pairs = contract::scope_pairs(&asset["scope"])?;
        return pairs
            .iter()
            .all(|(episode, id)| {
                source
                    .scenes
                    .iter()
                    .any(|scene| scene.id == *id && scene.episode_id == *episode)
            })
            .then_some(pairs);
    }
    let ids = asset["applicability"]["scenes"].as_array()?;
    let pairs: BTreeSet<_> = ids
        .iter()
        .map(|id| {
            let id = id.as_str()?;
            let scene = source.scenes.iter().find(|scene| scene.id == id)?;
            Some((scene.episode_id.clone(), scene.id.clone()))
        })
        .collect::<Option<_>>()?;
    (!pairs.is_empty()).then_some(pairs)
}

/// A candidate requires actual accepted A15 proof for both facts. Scene titles,
/// CHARACTER_CLAIM, free-form relation notes and names cannot create an edge.
fn raw_candidates(
    input: &Value,
    asset: &Value,
    source: &LockedSource,
    accepted: &[Value],
    selected: &BTreeMap<String, Vec<Value>>,
) -> Result<Vec<Value>, String> {
    if !prop_context_subject(input, asset)? {
        return Ok(Vec::new());
    }
    let proof = |fact: &Value| accepted.iter().any(|known| known == fact);
    let non_title_rows: HashSet<_> = source
        .rows
        .iter()
        .filter(|row| row.kind != "format")
        .map(|row| row.id.as_str())
        .collect();
    let body_refs = |fact: &Value| -> BTreeSet<String> {
        fact["sourceRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| non_title_rows.contains(*id))
            .map(str::to_owned)
            .collect()
    };
    let character_facts: Vec<_> = selected
        .get(text(asset, "id"))
        .ok_or("当前形象事实尚未就绪")?
        .iter()
        .filter(|fact| fact["claim"] == "SOURCE" && proof(fact))
        .collect();
    let catalogue = input["assets"].as_array().ok_or("制作上下文缺少资产目录")?;
    let character_scope = explicit_scene_scope(asset, source);
    let mut result = Vec::new();
    for prop in catalogue.iter().filter(|prop| {
        matches!(text(prop, "kind"), "PROP" | "prop" | "道具")
            && !crate::store::asset_deleted(prop)
            && prop["preparation"]["mode"] != "INLINE"
    }) {
        if character_scope
            .as_ref()
            .zip(explicit_scene_scope(prop, source).as_ref())
            .is_some_and(|(character, prop)| character.is_disjoint(prop))
        {
            continue;
        }
        let Some(prop_facts) = selected.get(text(prop, "id")) else {
            continue;
        };
        let mut rows = BTreeSet::new();
        let mut character_ids = BTreeSet::new();
        let mut prop_ids = BTreeSet::new();
        for character in &character_facts {
            let left = body_refs(character);
            for fact in prop_facts
                .iter()
                .filter(|fact| fact["claim"] == "SOURCE" && proof(fact))
            {
                let right = body_refs(fact);
                let shared: Vec<_> = left.intersection(&right).cloned().collect();
                if !shared.is_empty() {
                    rows.extend(shared);
                    character_ids.insert(text(character, "id").to_owned());
                    prop_ids.insert(text(fact, "id").to_owned());
                }
            }
        }
        if !rows.is_empty() {
            let mut candidate = json!({"assetId":prop["id"],"identityId":contract::identity(prop),"variantId":prop["variantId"],"scope":prop["scope"],"applicability":prop["applicability"],"assetSemanticVersion":digest(&contract::asset_semantics(prop).to_string()),"sharedSourceRefs":rows,"characterFactIds":character_ids,"propFactIds":prop_ids,"relation":"SOURCE_ROW_CANDIDATE_NOT_OWNER_FACT"});
            if !contract::is_character(asset) {
                let facts = candidate.as_object_mut().unwrap().remove("characterFactIds").unwrap();
                candidate["locationFactIds"] = facts;
            }
            result.push(candidate);
        }
    }
    result.sort_by_key(|prop| text(prop, "assetId").to_owned());
    Ok(result)
}

fn reaches(graph: &BTreeMap<String, BTreeSet<String>>, origin: &str, target: &str) -> bool {
    let mut pending = vec![origin];
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if id == target {
            return true;
        }
        if !seen.insert(id) {
            continue;
        }
        if let Some(edges) = graph.get(id) {
            pending.extend(edges.iter().map(String::as_str));
        }
    }
    false
}

/// Separate text dependencies only. Parent edges always win over prop context;
/// an edge that would introduce a cycle is omitted, never added to the image DAG.
pub(crate) fn candidates(input: &Value) -> Result<BTreeMap<String, Vec<Value>>, String> {
    if profile(input)?.is_none() {
        return Ok(BTreeMap::new());
    }
    let source: LockedSource =
        serde_json::from_value(input["source"].clone()).map_err(|_| "制作上下文原文无效")?;
    let catalogue = input["assets"].as_array().ok_or("制作上下文缺少资产目录")?;
    let accepted = super::asset_visual_facts::accepted_source_facts(input, &source)?;
    let mut graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut parents = BTreeMap::new();
    let mut invalid = HashSet::new();
    for asset in catalogue {
        match assets::resolve_asset_dependency(catalogue, asset) {
            Ok(Some(parent)) => {
                parents.insert(text(asset, "id").to_owned(), Some(parent.clone()));
                graph
                    .entry(text(asset, "id").to_owned())
                    .or_default()
                    .insert(parent);
            }
            Ok(None) => {
                parents.insert(text(asset, "id").to_owned(), None);
            }
            Err(_) => {
                invalid.insert(text(asset, "id").to_owned());
            }
        }
    }
    let by_id = catalogue
        .iter()
        .map(|asset| (text(asset, "id"), asset))
        .collect();
    let known = source.rows.iter().map(|row| row.id.as_str()).collect();
    let selected: BTreeMap<_, _> = catalogue
        .iter()
        .filter(|asset| !invalid.contains(text(asset, "id")))
        .filter_map(|asset| {
            assets::design_facts(input, asset, &by_id, &parents, &known)
                .ok()
                .map(|(facts, _)| (text(asset, "id").to_owned(), facts))
        })
        .collect();
    let mut characters: Vec<_> = catalogue
        .iter()
        .filter(|asset| prop_context_subject(input, asset).unwrap_or(false) && !invalid.contains(text(asset, "id")))
        .collect();
    characters.sort_by_key(|asset| text(asset, "id"));
    let mut result = BTreeMap::new();
    for character in characters {
        let id = text(character, "id");
        let Ok(possible) = raw_candidates(input, character, &source, &accepted, &selected) else {
            continue; // Its unresolved fact/parent family is handled by C/D's existing gate.
        };
        let mut safe = Vec::new();
        for prop in possible {
            let prop_id = text(&prop, "assetId");
            if invalid.contains(prop_id) || reaches(&graph, prop_id, id) {
                continue;
            }
            graph
                .entry(id.to_owned())
                .or_default()
                .insert(prop_id.to_owned());
            safe.push(prop);
        }
        result.insert(id.to_owned(), safe);
    }
    Ok(result)
}

fn prop_design(input: &Value, prop: &Value, source: &LockedSource) -> Option<Value> {
    let design = &prop["design"];
    if prop["promptOrigin"] == "MANUAL"
        || design["schemaVersion"] != "asset-design/v1"
        || design["sourceSha256"] != source.sha256
        || design["assetId"] != prop["id"]
        || design["variantId"] != prop["variantId"]
        || text(prop, "prompt").trim().is_empty()
        || design["prompt"] != prop["prompt"]
    {
        return None;
    }
    // Evaluate an older accepted prop under its actual contract, not by silently
    // upgrading it to the new workflow profile. Current facts/source still bind.
    let mut basis = input.clone();
    let original_basis = &design["designReceipt"]["basis"];
    if original_basis.get("assetProductionContext").is_none() {
        basis.as_object_mut()?.remove(PROFILE_FIELD);
    } else if frozen_version(input).ok()?.is_some_and(modern_profile)
        && original_basis["assetProductionContext"]["version"].as_str() != frozen_version(input).ok()?
    {
        // New subjects may consume older props only after evaluating the
        // exact original frozen instructions, never a current-profile receipt.
        let original = &design["frozenContext"]["assetProductionContext"];
        let current = frozen_version(input).ok()??;
        let previous = text(original, "version");
        if !(current == PREVIOUS_PROFILE && previous == LEGACY_PROFILE
            || current == PROFILE && matches!(previous, LEGACY_PROFILE | PREVIOUS_PROFILE))
            || !valid_profile(original)
            || original["version"] != original_basis["assetProductionContext"]["version"]
        {
            return None;
        }
        basis[PROFILE_FIELD] = json!({"version":original["version"],"instructions":original["instructions"],"instructionsSha256":original["instructionsSha256"]});
    }
    if original_basis
        .get("identityProductionMaterialsSha256")
        .is_none()
        && original_basis
            .get("assetScopeProductionMaterialsSha256")
            .is_none()
    {
        for key in [
            contract::IDENTITY_MATERIAL_PROFILE_FIELD,
            contract::IDENTITY_MATERIALS_FIELD,
            contract::SCOPE_MATERIALS_FIELD,
        ] {
            basis.as_object_mut()?.remove(key);
        }
    }
    let known: HashSet<_> = source.rows.iter().map(|row| row.id.as_str()).collect();
    let facts = selected_facts(&basis, prop, source).ok()?;
    let mut evidence: HashSet<_> = prop["sourceRefs"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for fact in &facts {
        evidence.extend(
            fact["sourceRefs"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str),
        );
    }
    for row in assets::empty_design_scene_context(source, prop, &facts, &evidence) {
        evidence.insert(row.id.as_str());
    }
    let production_evidence = context(&basis, prop)
        .ok()?
        .as_ref()
        .map(|context| evidence_rows(context, source))
        .unwrap_or_default();
    evidence.extend(production_evidence.iter().map(String::as_str));
    let catalogue = basis["assets"].as_array()?;
    let parent = assets::resolve_asset_dependency(catalogue, prop).ok()?;
    assets::reusable_parent_design(
        &basis,
        prop,
        catalogue,
        &source.sha256,
        &known,
        &evidence,
        parent.as_deref(),
    )?;
    Some(
        json!({"prompt":prop["prompt"],"sourceRefs":design["sourceRefs"],"sourceSha256":source.sha256,"designReceiptSha256":design["designReceipt"]["sha256"],"designVersion":digest(&json!({"prompt":prop["prompt"],"receipt":design["designReceipt"]}).to_string()),"usage":"ACCEPTED_TEXT_DESIGN_NOT_SOURCE_FACT_OR_IMAGE_REFERENCE"}),
    )
}

pub(crate) fn context(input: &Value, asset: &Value) -> Result<Option<Value>, String> {
    let Some(frozen) = profile(input)? else {
        return Ok(None);
    };
    let source: LockedSource =
        serde_json::from_value(input["source"].clone()).map_err(|_| "制作上下文原文无效")?;
    let facts = selected_facts(input, asset, &source)?;
    let mut applicable = BTreeSet::new();
    if let Some(pairs) = contract::scope_pairs(&asset["scope"]) {
        applicable.extend(pairs.into_iter().map(|(_, scene)| scene));
    }
    applicable.extend(
        asset["applicability"]["scenes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    if applicable.is_empty() {
        let refs: HashSet<_> = asset["sourceRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        applicable.extend(
            source
                .rows
                .iter()
                .filter(|row| refs.contains(row.id.as_str()))
                .filter_map(|row| row.scene_id.clone()),
        );
    }
    let mut evidence_scenes = BTreeSet::new();
    let mut read_scenes = BTreeSet::new();
    if matches!(text(asset, "kind"), "LOC" | "location" | "场景") && applicable.len() > 1 {
        if let Some(first) = source
            .scenes
            .iter()
            .find(|scene| applicable.contains(&scene.id))
        {
            read_scenes.insert(first.id.clone());
        }
    }
    let mut fact_scopes = Vec::new();
    for fact in &facts {
        let refs: HashSet<_> = fact["sourceRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut scenes = BTreeSet::new();
        let mut background = Vec::new();
        for row in source
            .rows
            .iter()
            .filter(|row| refs.contains(row.id.as_str()))
        {
            if let Some(scene) = &row.scene_id {
                scenes.insert(scene.clone());
            } else {
                background.push(row.id.clone());
            }
        }
        evidence_scenes.extend(scenes.iter().cloned());
        fact_scopes.push(json!({"factId":fact["id"],"claim":fact["claim"],"scope":fact["scope"],"variantId":fact["variantId"],"sourceRefs":fact["sourceRefs"],"evidenceSceneIds":scenes,"backgroundRowIds":background}));
    }
    let scene_card = |scene: &super::source::SourceScene| json!({"id":scene.id,"episodeId":scene.episode_id,"title":scene.title});
    let mut related = if prop_context_subject(input, asset)? {
        candidates(input)?
            .remove(text(asset, "id"))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    for candidate in &mut related {
        let prop = input["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|prop| prop["id"] == candidate["assetId"]);
        let failed = input["stage2UnavailablePropAssetIds"]
            .as_array()
            .is_some_and(|ids| ids.contains(&candidate["assetId"]));
        candidate["acceptedDesign"] = if failed {
            Value::Null
        } else {
            prop.and_then(|prop| prop_design(input, prop, &source))
                .unwrap_or(Value::Null)
        };
        candidate["disposition"] = json!(if candidate["acceptedDesign"].is_null() {
            "RELATION_ONLY_UNTIL_VALID_DESIGN"
        } else {
            "VERIFY_SAME_OBJECT_IN_SOURCE_THEN_REUSE"
        });
    }
    let mut context = json!({"version":frozen["version"],"sourceSha256":source.sha256,"instructions":frozen["instructions"],"instructionsSha256":frozen["instructionsSha256"],"applicableScenes":source.scenes.iter().filter(|scene| applicable.contains(&scene.id)).map(scene_card).collect::<Vec<_>>(),"factEvidenceScenes":source.scenes.iter().filter(|scene| evidence_scenes.contains(&scene.id)).map(scene_card).collect::<Vec<_>>(),"factEvidenceScopes":fact_scopes,"readSceneIds":source.scenes.iter().filter(|scene|read_scenes.contains(&scene.id)).map(|scene|scene.id.clone()).collect::<Vec<_>>(),"relatedProps":related});
    if modern_profile(text(frozen, "version")) {
        if let Some(proof) = parent_design_choices(input, asset)? {
            context["parentDesignChoices"] = proof;
        }
    }
    Ok(Some(context))
}

/// Source text is still materialized once, in C/D evidence. Scene mapping is
/// context, not permission to adopt every row as a visual fact for this subject.
pub(crate) fn evidence_rows(context: &Value, source: &LockedSource) -> BTreeSet<String> {
    let scenes: HashSet<_> = context["readSceneIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    source
        .rows
        .iter()
        .filter(|row| {
            row.scene_id
                .as_deref()
                .is_some_and(|scene| scenes.contains(scene))
        })
        .map(|row| row.id.as_str())
        .chain(
            context["relatedProps"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|prop| {
                    prop["sharedSourceRefs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                }),
        )
        .map(str::to_owned)
        .collect()
}

pub(crate) fn binding(context: &Value) -> Value {
    let consumed: Vec<_> = context["relatedProps"].as_array().into_iter().flatten().filter(|prop| !prop["acceptedDesign"].is_null())
        .map(|prop| json!({"assetId":prop["assetId"],"identityId":prop["identityId"],"variantId":prop["variantId"],"assetSemanticVersion":prop["assetSemanticVersion"],"sourceSha256":context["sourceSha256"],"sharedSourceRefs":prop["sharedSourceRefs"],"designVersion":prop["acceptedDesign"]["designVersion"],"designReceiptSha256":prop["acceptedDesign"]["designReceiptSha256"]})).collect();
    let mut binding = json!({"version":context["version"],"instructionsSha256":context["instructionsSha256"],"sceneContextSha256":digest(&json!({"applicableScenes":context["applicableScenes"],"factEvidenceScenes":context["factEvidenceScenes"],"factEvidenceScopes":context["factEvidenceScopes"],"readSceneIds":context["readSceneIds"]}).to_string()),"relatedPropsSha256":digest(&context["relatedProps"].to_string()),"consumedPropDesigns":consumed});
    if modern_profile(text(context, "version")) {
        if let Some(proof) = context.get("parentDesignChoices") {
            binding["parentDesignChoices"] = proof.clone();
        }
    }
    binding
}

/// Atomic publication and resume guard for relevant props outside proposals.
/// Counters/media do not invalidate a design; a user edit to prose or semantic
/// identity does. Missing profile keeps the exact legacy publication contract.
pub fn validate_consumed_prop_designs(design: &Value, catalogue: &[Value]) -> Result<(), String> {
    validate_consumed_designs(design, catalogue, &mut HashSet::new())
}

fn validate_consumed_dependency(
    asset: &Value,
    catalogue: &[Value],
    visiting: &mut HashSet<String>,
) -> Result<(), String> {
    let id = text(asset,"id").to_owned();
    if !visiting.insert(id.clone()) || visiting.len() > catalogue.len() {
        return Err("已接受制作取舍依赖循环或超出冻结目录，原稿保留".into());
    }
    let result = validate_consumed_designs(&asset["design"], catalogue, visiting);
    visiting.remove(&id);
    result
}

fn validate_consumed_designs(
    design: &Value,
    catalogue: &[Value],
    visiting: &mut HashSet<String>,
) -> Result<(), String> {
    let Some(expected) = design["designReceipt"]["basis"].get("assetProductionContext") else {
        return Ok(());
    };
    let context = &design["frozenContext"]["assetProductionContext"];
    if !valid_profile(context)
        || *expected != binding(context)
    {
        return Err("已冻结制作上下文与设计收据不一致，原稿保留".into());
    }
    if let Some(proof) = expected.get("parentDesignChoices") {
        let matches: Vec<_> = catalogue.iter().filter(|parent| {
            parent["id"] == proof["assetId"] && !crate::store::asset_deleted(parent)
        }).collect();
        let [parent] = matches.as_slice() else {
            return Err("已使用的父设计制作取舍身份缺失或重复，原稿保留".into());
        };
        if !modern_profile(text(context, "version"))
            || proof["sourceSha256"] != context["sourceSha256"]
            || parent_design_choice_proof(parent, &parent["design"]).as_ref() != Some(proof)
        {
            return Err("已使用的父设计制作取舍/范围或设计版本已变化，原请求与稿件保留".into());
        }
        validate_consumed_dependency(parent, catalogue, visiting)?;
    }
    for proof in expected["consumedPropDesigns"]
        .as_array()
        .ok_or("道具设计绑定目录无效")?
    {
        let matches: Vec<_> = catalogue
            .iter()
            .filter(|prop| prop["id"] == proof["assetId"] && !crate::store::asset_deleted(prop))
            .collect();
        let [prop] = matches.as_slice() else {
            return Err("已使用的相关道具身份缺失或重复，未采用新人物稿".into());
        };
        if prop["variantId"] != proof["variantId"]
            || prop["promptOrigin"] == "MANUAL"
            || contract::identity(prop) != text(proof, "identityId")
            || digest(&contract::asset_semantics(prop).to_string()) != proof["assetSemanticVersion"]
            || prop["design"]["sourceSha256"] != proof["sourceSha256"]
            || prop["design"]["designReceipt"]["sha256"] != proof["designReceiptSha256"]
            || digest(
                &json!({"prompt":prop["prompt"],"receipt":prop["design"]["designReceipt"]})
                    .to_string(),
            ) != proof["designVersion"]
        {
            return Err("已使用的相关道具源/形象/设计版本已变化；原请求与稿件保留".into());
        }
        if prop["design"]["designReceipt"]["basis"]["sourceChain"]
            != json!(contract::source_chain(catalogue, prop)?)
        {
            return Err("已使用的相关道具祖先设计版本已变化；原请求与稿件保留".into());
        }
        if modern_profile(text(context, "version")) {
            validate_consumed_dependency(prop, catalogue, visiting)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_resume(
    original: &Value,
    result: &Value,
    current: &Value,
) -> Result<(), String> {
    if profile(original)?.is_none() {
        return Ok(());
    }
    let catalogue = current["assets"].as_array().ok_or("续跑缺少相关道具目录")?;
    for design in result["designs"].as_array().into_iter().flatten() {
        validate_consumed_prop_designs(design, catalogue)?;
    }
    // A stopped workflow may not have published any result yet. Protect valid
    // out-of-scope props from its original snapshot before replaying cached jobs.
    let wanted = original["selection"]["assetIds"].as_array();
    let original_catalogue = original["assets"].as_array().ok_or("续跑原资产目录缺失")?;
    let mut original_targets: HashSet<String> = original_catalogue
        .iter()
        .filter(|asset| wanted.is_none_or(|ids| ids.contains(&asset["id"])))
        .map(|asset| text(asset, "id").to_owned())
        .collect();
    for asset in original_catalogue
        .iter()
        .filter(|asset| wanted.is_none_or(|ids| ids.contains(&asset["id"])))
    {
        for ancestor in contract::source_chain(original_catalogue, asset)? {
            original_targets.insert(text(&ancestor, "assetId").to_owned());
        }
    }
    for asset in original["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| wanted.is_none_or(|ids| ids.contains(&asset["id"])))
    {
        // Original facts may still await their A15 variant repair. No C/D
        // context existed then; its stopped request is guarded by exact replay.
        if let Ok(Some(mut context)) = context(original, asset) {
            context["relatedProps"]
                .as_array_mut()
                .unwrap()
                .retain(|prop| !original_targets.contains(text(prop, "assetId")));
            if context.get("parentDesignChoices").is_some_and(|proof| {
                original_targets.contains(text(proof, "assetId"))
            }) {
                // Accepted in-flow parents are guarded by each frozen child
                // request/result; original preflight still contains old parents.
                context.as_object_mut().unwrap().remove("parentDesignChoices");
            }
            let design = json!({"designReceipt":{"basis":{"assetProductionContext":binding(&context)}},"frozenContext":{"assetProductionContext":context}});
            validate_consumed_prop_designs(&design, catalogue)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parent_design_choices_v1_instruction_and_binding_bytes_remain_exact() {
        assert_eq!(digest(LEGACY_INSTRUCTIONS), "918274f96c798388a74c1f022d34146a885ae2134fd469f12b8e1955c6c443e1");
        let frozen = json!({"version":LEGACY_PROFILE,"instructions":LEGACY_INSTRUCTIONS,"instructionsSha256":digest(LEGACY_INSTRUCTIONS)});
        let input = json!({PROFILE_FIELD:frozen});
        assert_eq!(frozen_version(&input).unwrap(),Some(LEGACY_PROFILE));
        let context = json!({"version":LEGACY_PROFILE,"sourceSha256":"source","instructions":LEGACY_INSTRUCTIONS,"instructionsSha256":digest(LEGACY_INSTRUCTIONS),"applicableScenes":[],"factEvidenceScenes":[],"factEvidenceScopes":[],"readSceneIds":[],"relatedProps":[]});
        assert_eq!(binding(&context),json!({"version":LEGACY_PROFILE,"instructionsSha256":digest(LEGACY_INSTRUCTIONS),"sceneContextSha256":digest(&json!({"applicableScenes":[],"factEvidenceScenes":[],"factEvidenceScopes":[],"readSceneIds":[]}).to_string()),"relatedPropsSha256":digest("[]"),"consumedPropDesigns":[]}));
        let design = json!({"designReceipt":{"basis":{"assetProductionContext":binding(&context)}},"frozenContext":{"assetProductionContext":context}});
        validate_consumed_prop_designs(&design,&[]).unwrap();
    }

    #[test]
    fn parent_design_choices_versions_and_exact_instruction_hash_fail_closed() {
        let mut input = json!({});
        freeze(&mut input);
        assert_eq!(frozen_version(&input).unwrap(),Some(PROFILE));
        for (version,instructions) in [(LEGACY_PROFILE,INSTRUCTIONS),(PROFILE,LEGACY_INSTRUCTIONS),("asset-production-context/v4",INSTRUCTIONS)] {
            let changed = json!({PROFILE_FIELD:{"version":version,"instructions":instructions,"instructionsSha256":digest(instructions)}});
            assert!(frozen_version(&changed).is_err());
        }
        input[PROFILE_FIELD]["instructionsSha256"] = json!(digest(LEGACY_INSTRUCTIONS));
        assert!(frozen_version(&input).is_err());
        for ids in [json!(null),json!(["child","child"]),json!([3]),json!([""])] {
            let changed = json!({PROFILE_FIELD:{"version":PROFILE,"instructions":INSTRUCTIONS,"instructionsSha256":digest(INSTRUCTIONS),"explicitAssetIds":ids}});
            assert!(frozen_version(&changed).is_err());
        }
    }

    #[test]
    fn parent_design_choices_missing_accepted_scope_rejects_v2_and_keeps_v1_card_contract() {
        let asset = json!({"id":"parent","baseAssetId":"subject","variantId":"base","prompt":"已接受的父稿","promptOrigin":"AI"});
        let design = json!({"schemaVersion":"asset-design/v1","assetId":"parent","variantId":"base","prompt":"已接受的父稿","sourceSha256":"source","designReceipt":{"sha256":"receipt"}});
        let mut input = json!({"source":{"sha256":"source"}});
        freeze(&mut input);
        assert!(reference_design_choices(&input,&asset,Some(&design)).is_err());
        input[PROFILE_FIELD] = json!({"version":LEGACY_PROFILE,"instructions":LEGACY_INSTRUCTIONS,"instructionsSha256":digest(LEGACY_INSTRUCTIONS)});
        assert_eq!(reference_design_choices(&input,&asset,Some(&design)).unwrap(),None);
    }

    #[test]
    fn unreceipted_shared_facts_never_create_prop_candidates() {
        let source =
            super::super::prepare_source("第一集\n场景：室内\n△甲拿着独立物件。\n").unwrap();
        let row = &source.rows[2].id;
        let person = json!({"id":"person-row","baseAssetId":"person","variantId":"base","kind":"CH","sourceRefs":[row]});
        let prop = json!({"id":"prop-row","baseAssetId":"object","variantId":"base","kind":"PROP","sourceRefs":[row]});
        let fact = |id: &str, owner: &str| json!({"id":id,"assetId":owner,"variantId":null,"attribute":"appearance","value":"已记录物件","scope":"STABLE","claim":"SOURCE","sourceRefs":[row],"sourceSha256":source.sha256});
        let mut input = json!({"source":source,"assets":[person,prop],"foundation":{"facts":[fact("person-fact","person"),fact("prop-fact","object")]}});
        freeze(&mut input);
        assert_eq!(
            candidates(&input).unwrap()["person-row"],
            Vec::<Value>::new()
        );
        let context = context(&input, &input["assets"][0]).unwrap().unwrap();
        assert_eq!(context["relatedProps"], json!([]));
        assert_eq!(
            context["factEvidenceScopes"].as_array().unwrap().len(),
            1,
            "unreceipted facts remain existing C/D facts; only candidate linking requires proof"
        );
    }

    #[test]
    fn legacy_profile_is_absent_and_frozen_instruction_tampering_fails_closed() {
        let mut input = json!({});
        assert!(profile(&input).unwrap().is_none());
        freeze(&mut input);
        assert!(profile(&input).unwrap().is_some());
        input[PROFILE_FIELD]["instructions"] = json!("changed instruction");
        input[PROFILE_FIELD]["instructionsSha256"] = json!(digest("changed instruction"));
        assert!(profile(&input).is_err());
        assert!(
            validate_consumed_prop_designs(&json!({"designReceipt":{"basis":{}}}), &[]).is_ok(),
            "no new gate for historical receipts"
        );
    }

    #[test]
    fn prop_consumption_ignores_save_and_media_counters_but_guards_semantics_and_design_bytes() {
        let prop = json!({"id":"prop","baseAssetId":"object","variantId":"base","kind":"PROP","description":"独立形制","sourceRefs":["source-row"],"prompt":"已采用物件稿","version":1,"design":{"sourceSha256":"source-sha","designReceipt":{"sha256":"receipt-sha","basis":{"sourceChain":[]}}}});
        let context = json!({"version":PROFILE,"sourceSha256":"source-sha","instructions":INSTRUCTIONS,"instructionsSha256":digest(INSTRUCTIONS),"relatedProps":[{"assetId":"prop","identityId":"object","variantId":"base","assetSemanticVersion":digest(&contract::asset_semantics(&prop).to_string()),"sharedSourceRefs":["source-row"],"acceptedDesign":{"designVersion":digest(&json!({"prompt":prop["prompt"],"receipt":prop["design"]["designReceipt"]}).to_string()),"designReceiptSha256":"receipt-sha"}}]});
        let design = json!({"designReceipt":{"basis":{"assetProductionContext":binding(&context)}},"frozenContext":{"assetProductionContext":context}});
        let mut saved = prop.clone();
        saved["version"] = json!(42);
        saved["adoptedMediaId"] = json!("media");
        validate_consumed_prop_designs(&design, &[saved.clone()]).unwrap();
        for field in ["description", "prompt"] {
            let mut edited = saved.clone();
            edited[field] = json!("user edit");
            assert!(
                validate_consumed_prop_designs(&design, &[edited]).is_err(),
                "{field}"
            );
        }
        assert!(validate_consumed_prop_designs(&design, &[]).is_err());
        assert!(validate_consumed_prop_designs(&design, &[saved.clone(), saved]).is_err());
    }
}
