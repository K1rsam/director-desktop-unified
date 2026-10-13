//! Opt-in C/D reading and ordinary art completion. Legacy requests retain their
//! original evidence bytes; source facts and preparation prose are never edited.
use super::*;
use std::collections::BTreeSet;

pub(crate) const DESIGN_EVIDENCE_PROFILE_FIELD: &str = "stage2DesignEvidence";
pub(crate) const DESIGN_EVIDENCE_PROFILE: &str = "asset-design-evidence/v1";
const INSTRUCTIONS: &str = "本合同补充C基础/D派生的取料和普通美术补足，不修改任何原文、facts、claim、资产身份或父链。evidence中标为SOURCE_CONTEXT的行是阅读材料，不自动成为当前资产的SOURCE事实；按当前asset.scope、准确集场坐标及variantId核对。LOC补入的整场原文只来自当前形象明确适用场次，GLOBAL只读原sourceRefs定位场次，不扩展全剧；此处readScenes说明本请求实际已提供的场次，优先于旧制作说明中的取料范围描述。逐场区分稳定空间结构/必要环境物件与事件造成的临时变化，物件存在与事件造成的破坏分别处理，不要求基础空间永远保留破坏状态；不能把同场其它主体、后来损坏、伤病或服装变化变成基线，也不能改写或删除原有事实。原文明示约束、当前形象有效状态、用户手写说明及已采用父稿中仍适用的设计均须遵守。preparation.note或identityProductionMaterial等制作资料中的‘原文未明确/无参考/不补造’若仅说明缺少原文依据，表示未知，不等于用户禁止普通美术设计；不取消其中的具体明示约束。对原文未限定的普通外貌、日常完整衣装、物件基本形制比例、基础材质和颜色等，按时代、身份与已确认画风提供保守且可直接制作的非剧情美术方案，写入freeDesignScope，绝不升级为SOURCE或新增剧情/能力/标识。人物全身设计不能因衣装未知而留空或要求用户另行确认；DERIVED保留直接父稿的稳定身份和未变设计，仅按当前有效变化调整。已限定使用场次的配饰/道具不得自动常驻。用户手写提示词始终保留，不据此合同自动改写。";

/// Called only by the new-workflow boundary, never while replaying a saved job.
pub(crate) fn freeze_design_evidence_profile(input: &mut Value) {
    input[DESIGN_EVIDENCE_PROFILE_FIELD] = json!({
        "version":DESIGN_EVIDENCE_PROFILE,"instructions":INSTRUCTIONS,
        "instructionsSha256":digest(INSTRUCTIONS)
    });
}

pub(super) fn enabled(input: &Value) -> Result<bool, String> {
    let Some(profile) = input
        .get(DESIGN_EVIDENCE_PROFILE_FIELD)
        .filter(|v| !v.is_null())
    else {
        return Ok(false);
    };
    if profile["version"] != DESIGN_EVIDENCE_PROFILE
        || profile["instructions"] != INSTRUCTIONS
        || profile["instructionsSha256"] != digest(INSTRUCTIONS)
    {
        return Err("第二阶段设计取料合同版本或指令摘要无效；保留冻结请求，不自动升级".into());
    }
    Ok(true)
}

pub(super) fn scene_context<'a>(
    input: &Value,
    source: &'a LockedSource,
    asset: &Value,
    facts: &[Value],
    evidence_ids: &HashSet<&str>,
) -> Result<Vec<&'a SourceRow>, String> {
    let legacy = empty_design_scene_context(source, asset, facts, evidence_ids);
    if !enabled(input)? {
        return Ok(legacy);
    }
    if !matches!(text(asset, "kind"), "LOC" | "location" | "场景")
        && !matches!(text(asset, "type"), "LOC" | "location" | "场景")
    {
        return Ok(legacy); // A many-scene character must not expand to the whole script.
    }
    if has_program_source_scope(asset) && asset["sourceScope"]["sourceSha256"] != source.sha256 {
        return Err("场景设计取料的来源摘要与锁定原文不一致".into());
    }
    let pairs = if asset["scope"]["kind"] == "SCENES" {
        let pairs = super::super::asset_design_contract::scope_pairs(&asset["scope"])
            .ok_or("场景设计的准确集场范围无效，未扩大取料")?;
        if pairs.iter().any(|(episode, scene)| {
            !source
                .scenes
                .iter()
                .any(|s| &s.id == scene && &s.episode_id == episode)
        }) {
            return Err("场景设计范围与锁定原文集场坐标不一致".into());
        }
        pairs
    } else {
        // GLOBAL is applicability, not permission to pass every scene. An old
        // catalogue without scope still has exact source reference anchors.
        let refs = exact_strings(asset, "sourceRefs")?;
        source
            .rows
            .iter()
            .filter(|row| refs.contains(&row.id))
            .filter_map(|row| Some((row.episode_id.clone()?, row.scene_id.clone()?)))
            .collect::<BTreeSet<_>>()
    };
    Ok(source
        .rows
        .iter()
        .filter(|row| {
            row.episode_id
                .as_ref()
                .zip(row.scene_id.as_ref())
                .is_some_and(|(ep, scene)| pairs.contains(&(ep.clone(), scene.clone())))
        })
        .collect())
}

/// The receipt binds the same ordered source payload that C/D consumes, not
/// merely the parser's claimed SHA or the sparse visual-fact references.
pub(crate) fn design_evidence_binding(
    input: &Value,
    asset: &Value,
) -> Result<Option<Value>, String> {
    if !enabled(input)? {
        return Ok(None);
    }
    let source: LockedSource = serde_json::from_value(input["source"].clone())
        .map_err(|_| "设计取料收据缺少有效锁定原文")?;
    let assets = input["assets"]
        .as_array()
        .ok_or("设计取料收据缺少资产目录")?;
    let by_id = assets.iter().map(|a| (text(a, "id"), a)).collect();
    let parents = assets
        .iter()
        .filter_map(|a| {
            resolve_asset_dependency(assets, a)
                .ok()
                .map(|parent| (text(a, "id").to_owned(), parent))
        })
        .collect();
    // A malformed unrelated branch does not block this design; the current
    // chain is still validated, without falling back to an invented base.
    super::super::asset_design_contract::source_chain(assets, asset)?;
    let known = source.rows.iter().map(|row| row.id.as_str()).collect();
    let (facts, _) = design_facts(input, asset, &by_id, &parents, &known)?;
    let refs = exact_strings(asset, "sourceRefs")?;
    let mut ids: HashSet<_> = refs.iter().map(String::as_str).collect();
    for fact in &facts {
        ids.extend(
            fact["sourceRefs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }
    let context = scene_context(input, &source, asset, &facts, &ids)?;
    ids.extend(context.iter().map(|row| row.id.as_str()));
    let production = super::super::asset_production_context::context(input, asset)?;
    let production_rows = production
        .as_ref()
        .map(|c| super::super::asset_production_context::evidence_rows(c, &source))
        .unwrap_or_default();
    ids.extend(production_rows.iter().map(String::as_str));
    let evidence: Vec<_> = source
        .rows
        .iter()
        .filter(|row| ids.contains(row.id.as_str()))
        .collect();
    let read_pairs: BTreeSet<_> = context
        .iter()
        .filter_map(|row| Some((row.episode_id.as_ref()?, row.scene_id.as_ref()?)))
        .collect();
    Ok(Some(json!({
        "version":DESIGN_EVIDENCE_PROFILE,"instructionsSha256":digest(INSTRUCTIONS),
        "sourceSha256":source.sha256,"evidenceSha256":digest(&json!(evidence).to_string()),
        "evidenceRowIds":evidence.iter().map(|row|&row.id).collect::<Vec<_>>(),
        "readScenes":read_pairs.into_iter().map(|(episode,scene)|json!({"episodeId":episode,"sceneId":scene})).collect::<Vec<_>>()
    })))
}

pub(super) fn request_context(input: &Value, binding: &Value) -> Value {
    let mut context = binding.clone();
    context["instructions"] = input[DESIGN_EVIDENCE_PROFILE_FIELD]["instructions"].clone();
    context
}

#[cfg(test)]
#[path = "assets_evidence_context_tests.rs"]
mod tests;
