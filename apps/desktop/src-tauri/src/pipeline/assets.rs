use super::*;
use std::{
    collections::{BTreeMap, HashSet},
    future::{poll_fn, Future},
    pin::Pin,
    task::Poll,
};

const ASSET_DESIGN_CONCURRENCY: usize = 4;

#[path = "assets_evidence_context.rs"]
mod design_evidence;
pub(crate) use design_evidence::{freeze_design_evidence_profile, design_evidence_binding, DESIGN_EVIDENCE_PROFILE_FIELD};

pub(crate) const INLINE_DELTA_PROFILE_FIELD: &str = "stage2InlineDeltaProfile";
pub(crate) const INLINE_DELTA_PROFILE: &str = "asset-inline-effective-delta/v1";

pub(crate) fn freeze_inline_delta_profile(input: &mut Value) {
    input[INLINE_DELTA_PROFILE_FIELD] = json!(INLINE_DELTA_PROFILE);
}

fn inline_delta_enabled(input: &Value) -> Result<bool, String> {
    match input.get(INLINE_DELTA_PROFILE_FIELD) {
        None | Some(Value::Null) => Ok(false),
        Some(value) if value == INLINE_DELTA_PROFILE => Ok(true),
        _ => Err("INLINE 派生变化合同版本无效；保留原冻结请求，不自动升级".into()),
    }
}

/// Only fresh opted-in requests change delta semantics. The actual donor is
/// authoritative; a normal designed parent and its older ancestors stay out.
pub(super) fn inline_delta_context(input: &Value, asset: &Value) -> Result<Option<Value>, String> {
    if !inline_delta_enabled(input)? { return Ok(None); }
    let assets = input["assets"].as_array().ok_or("缺少资产目录")?;
    let Some(direct) = resolve_asset_dependency(assets, asset)? else { return Ok(None); };
    let by_id: BTreeMap<_, _> = assets.iter().map(|a| (text(a,"id"), a)).collect();
    let parents: BTreeMap<_, _> = assets.iter().filter_map(|a| resolve_asset_dependency(assets,a).ok().map(|p|(text(a,"id").to_owned(),p))).collect();
    let donor = inline_reference_target(&direct, &by_id, &parents)?;
    if donor == direct { return Ok(None); }
    let mut skipped = Vec::new();
    let mut cursor = direct.as_str();
    while cursor != donor {
        let ancestor = by_id.get(cursor).ok_or("INLINE 来源形象不存在")?;
        // A different subject can be an explicit image reference, never a fact donor.
        if asset_identity(ancestor) == asset_identity(asset) {
            skipped.push(cursor.to_owned());
        }
        cursor = parents.get(cursor).and_then(|p| p.as_deref()).ok_or("INLINE 指定来源不存在")?;
    }
    Ok(Some(json!({"version":INLINE_DELTA_PROFILE,"directAssetId":direct,"reusedDesignAssetId":donor,"effectiveInlineAssetIds":skipped})))
}

/// A program scope marker locates text; it is not the text itself. If A15 found
/// no appearance facts, give C/D the exact named scene instead of only its title.
/// Existing fact-backed requests stay byte-for-byte stable for recovery.
pub(super) fn empty_design_scene_context<'a>(
    source: &'a LockedSource,
    asset: &Value,
    facts: &[Value],
    evidence_ids: &HashSet<&str>,
) -> Vec<&'a SourceRow> {
    if !facts.is_empty()
        || !has_program_source_scope(asset)
        || source
            .rows
            .iter()
            .any(|row| evidence_ids.contains(row.id.as_str()) && row.kind != "format")
    {
        return Vec::new();
    }
    let scenes: HashSet<_> = source
        .rows
        .iter()
        .filter(|row| evidence_ids.contains(row.id.as_str()))
        .filter_map(|row| row.scene_id.as_deref())
        .collect();
    source
        .rows
        .iter()
        .filter(|row| row.scene_id.as_deref().is_some_and(|id| scenes.contains(id)))
        .collect()
}

fn inline_asset(asset: &Value) -> bool {
    asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("INLINE")
}

fn adopted_reference_available(assets: &[Value], parent: &str) -> bool {
    let mut current = parent;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(current) { return false; }
        let Ok(asset) = dependency_asset(assets, current) else { return false; };
        if !text(asset,"adoptedMediaId").is_empty() { return true; }
        if !inline_asset(asset) { return false; }
        let Ok(Some(next)) = resolve_asset_dependency(assets,asset) else { return false; };
        // Reborrow from the immutable catalogue rather than the temporary ID.
        let Ok(next_asset) = dependency_asset(assets,&next) else { return false; };
        current = text(next_asset,"id");
    }
}

pub(super) fn inline_reference_target(
    parent: &str,
    by_id: &BTreeMap<&str, &Value>,
    parents: &BTreeMap<String, Option<String>>,
) -> Result<String, String> {
    let mut cursor = parent;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(cursor) {
            return Err("INLINE 来源依赖循环，需修订来源关系".into());
        }
        let asset = by_id.get(cursor).ok_or("INLINE 指定来源不存在")?;
        if !inline_asset(asset)
            || !text(asset, "prompt").trim().is_empty()
            || !text(asset, "adoptedMediaId").is_empty()
        {
            return Ok(cursor.to_owned());
        }
        cursor = parents
            .get(cursor)
            .and_then(|parent| parent.as_deref())
            .ok_or("INLINE 文字表现没有可复用形象，不能作为派生图片来源")?;
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Resolve the direct art source, independently of the image preparation mode.
/// Names and catalogue order never identify a base appearance.
pub fn resolve_asset_dependency(assets: &[Value], asset: &Value) -> Result<Option<String>, String> {
    let direct = direct_asset_dependency(assets, asset)?;
    let mut seen = HashSet::from([text(asset, "id").to_owned()]);
    let mut cursor = direct.clone();
    while let Some(id) = cursor {
        if !seen.insert(id.clone()) {
            return Err("形象来源依赖循环，需修订来源关系".into());
        }
        let source = dependency_asset(assets, &id)?;
        cursor = direct_asset_dependency(assets, source)?;
    }
    Ok(direct)
}

fn dependency_asset<'a>(assets: &'a [Value], id: &str) -> Result<&'a Value, String> {
    let matches: Vec<_> = assets.iter().filter(|a| text(a, "id") == id).collect();
    match matches.as_slice() {
        [one] if crate::store::asset_deleted(one) => Err(format!("来源资产已删除：{id}")),
        [one] => Ok(one),
        [] => Err(format!("明确来源资产不存在：{id}")),
        _ => Err(format!("来源资产身份重复：{id}")),
    }
}

fn explicit_asset_identity(asset: &Value) -> &str {
    ["baseAssetId", "identityId"]
        .into_iter()
        .map(|key| text(asset, key))
        .find(|identity| !identity.is_empty())
        .unwrap_or("")
}

fn base_appearance(asset: &Value, identity: &str) -> bool {
    asset["isBase"].as_bool() == Some(true)
        || text(asset, "variantId") == "base"
        || (!identity.is_empty() && text(asset, "variantId") == format!("{identity}:base"))
}

fn direct_asset_dependency(assets: &[Value], asset: &Value) -> Result<Option<String>, String> {
    if let Some(id) = asset
        .get("sourceAssetId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        return dependency_asset(assets, id).map(|a| Some(text(a, "id").to_owned()));
    }
    let parent = asset
        .get("sourceVariantId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    if let Some(parent) = parent {
        let id = resolve_reference(assets, asset, parent)?;
        return dependency_asset(assets, &id).map(|a| Some(text(a, "id").to_owned()));
    }

    // An explicit null in a current catalogue declares an independent form.
    // Only legacy rows that omitted it may inherit the registered base.
    if asset.get("sourceVariantId") == Some(&Value::Null)
        && asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("REFERENCE")
    {
        return Ok(None);
    }

    let identity = explicit_asset_identity(asset);
    if asset.pointer("/preparation/mode").and_then(Value::as_str) != Some("REFERENCE")
        || identity.is_empty()
        || text(asset, "variantId").is_empty()
        || base_appearance(asset, identity)
    {
        return Ok(None);
    }
    // F2 registers the baseline as identity:base. Legacy imports can carry a
    // literal base or an explicit isBase marker; never guess from their names.
    let bases: Vec<_> = assets
        .iter()
        .filter(|candidate| {
            explicit_asset_identity(candidate) == identity && base_appearance(candidate, identity)
        })
        .collect();
    match bases.as_slice() {
        [] if text(asset, "variantId").starts_with(&format!("{identity}_V_")) => {
            Err("已登记子形象的基础来源缺失或已删除，请恢复或明确指定来源".into())
        }
        [] => Ok(None), // A legacy/standalone identity need not have a registered base.
        [one] => {
            dependency_asset(assets, text(one, "id")).map(|base| Some(text(base, "id").to_owned()))
        }
        _ => Err("同一主体有多个基础形象，请明确指定来源资产".into()),
    }
}
fn resolve_reference(assets: &[Value], asset: &Value, parent: &str) -> Result<String, String> {
    if let Some(exact) = assets.iter().find(|a| text(a, "id") == parent) {
        return Ok(text(exact, "id").to_string());
    }
    let candidates: Vec<_> = assets
        .iter()
        .filter(|a| text(a, "variantId") == parent)
        .collect();
    let identity = text(asset, "baseAssetId");
    let same_subject: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|a| !identity.is_empty() && text(a, "baseAssetId") == identity)
        .collect();
    let matches = if same_subject.is_empty() {
        candidates
    } else {
        same_subject
    };
    match matches.as_slice() {
        [one] => Ok(text(one, "id").to_string()),
        [] => Err(format!("来源形象不存在：{parent}")),
        _ => Err(format!(
            "来源形象 {parent} 有多个身份，请指定 sourceAssetId"
        )),
    }
}
fn exact_strings(value: &Value, key: &str) -> Result<Vec<String>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("缺少 {key}"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{key} 含非字符串引用"))
        })
        .collect()
}

fn asset_identity(asset: &Value) -> &str {
    if text(asset, "baseAssetId").is_empty() {
        text(asset, "id")
    } else {
        text(asset, "baseAssetId")
    }
}

/// The old C/D input contract inherits only this subject's explicit version facts.
/// A child replaces the same attribute, not another subject or a later state.
pub(super) fn design_facts(
    input: &Value,
    asset: &Value,
    by_id: &BTreeMap<&str, &Value>,
    parents: &BTreeMap<String, Option<String>>,
    known_rows: &HashSet<&str>,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let identity = asset_identity(asset);
    let inline_context = inline_delta_context(input, asset)?;
    let mut delta_variants = HashSet::from([text(asset, "variantId")]);
    if let Some(context) = &inline_context {
        for id in context["effectiveInlineAssetIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            delta_variants.insert(text(by_id.get(id).ok_or("INLINE 来源形象不存在")?, "variantId"));
        }
    }
    let owned: Vec<_> = input
        .pointer("/foundation/facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|fact| text(fact, "assetId") == identity)
        .collect();
    if let Some(fact) = owned.iter().find(|fact| {
        text(fact, "variantId").is_empty() && ["STATE", "TRANSITION"].contains(&text(fact, "scope"))
    }) {
        return Err(format!(
            "{} 的适用形象尚未确定，请先修正制作基础",
            text(fact, "attribute")
        ));
    }
    let mut chain = Vec::new();
    let mut cursor = Some(text(asset, "id"));
    let mut seen = HashSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return Err("形象依赖循环，需修订来源关系".into());
        }
        chain.push(*by_id.get(id).ok_or("来源形象不存在")?);
        cursor = parents.get(id).and_then(|parent| parent.as_deref());
    }
    chain.reverse();
    let mut selected: Vec<&Value> = owned
        .iter()
        .copied()
        .filter(|fact| text(fact, "variantId").is_empty() && text(fact, "scope") == "STABLE")
        .collect();
    for version in chain {
        if asset_identity(version) != identity {
            continue;
        }
        let explicit: Vec<_> = owned
            .iter()
            .copied()
            .filter(|fact| text(fact, "variantId") == text(version, "variantId"))
            .collect();
        let attributes: HashSet<_> = explicit
            .iter()
            .map(|fact| text(fact, "attribute"))
            .collect();
        selected.retain(|fact| !attributes.contains(text(fact, "attribute")));
        selected.extend(explicit);
    }
    let mut ids = HashSet::new();
    let mut facts = Vec::new();
    let mut delta = Vec::new();
    for fact in selected {
        let refs = exact_strings(fact, "sourceRefs")?;
        if refs.is_empty() || refs.iter().any(|row| !known_rows.contains(row.as_str())) {
            return Err("视觉事实引用不属于当前锁定原文".into());
        }
        if text(fact, "id").is_empty() || text(fact, "attribute").is_empty() {
            return Err("视觉事实缺少稳定身份或属性".into());
        }
        if !ids.insert(text(fact, "id")) {
            continue;
        }
        let mut fact = fact.clone();
        fact.as_object_mut()
            .ok_or("视觉事实必须为对象")?
            .remove("quotes");
        if delta_variants.contains(text(&fact, "variantId")) {
            delta.push(fact.clone());
        }
        facts.push(fact);
    }
    Ok((facts, delta))
}

fn has_program_source_scope(asset: &Value) -> bool {
    text(asset, "sourceRefsPurpose") == "SOURCE_SCOPE"
        && asset.pointer("/sourceScope/kind").and_then(Value::as_str) == Some("PROGRAM_SCOPE")
}

const SOURCE_SCOPE_POLICY: &str = "SOURCE_SCOPE/PROGRAM_SCOPE引用仅用于定位原文阅读范围，不是这些行已证明属于该资产的外观事实。facts/deltaFacts才是已按主体和版本登记的视觉提取记录，仍须对照其引用原文；claim保留原层次，CHARACTER_CLAIM不得升级为SOURCE。目录名称、description和preparation.note是身份/制作资料，不自动成为视觉事实；原文中的其它主体和后续状态不得套入当前形象。";

fn asset_design_card(input: &Value, asset: &Value) -> Result<Value, String> {
    let mut card = json!({
        "id":asset["id"],"baseAssetId":asset["baseAssetId"],"variantId":asset["variantId"],
        "name":asset["name"],"kind":asset.get("kind").or_else(||asset.get("type")),
        "description":asset["description"],"sourceRefs":asset["sourceRefs"],
        "scope":asset["scope"],"applicability":asset["applicability"],"preparation":asset["preparation"]
    });
    if has_program_source_scope(asset) {
        card["sourceRefsPurpose"] = asset["sourceRefsPurpose"].clone();
        card["sourceScope"] = asset["sourceScope"].clone();
    }
    if let Some(material) = super::asset_design_contract::identity_production_material(input, asset)? {
        card["identityProductionMaterial"] = material;
    }
    if let Some(material) = super::asset_design_contract::scope_production_material(input, asset)? {
        card["scope"]["reason"] = material["scope"]["reason"].clone();
        card["scopeProductionMaterial"] = material;
    }
    Ok(card)
}

fn reference_design_card(input: &Value, asset: &Value, design: Option<&Value>) -> Result<Value, String> {
    let mut card = json!({
        "assetId":asset["id"],"variantId":asset["variantId"],"assetVersion":asset["version"],
        "prompt":asset["prompt"],
        "origin":if text(asset,"promptOrigin")=="MANUAL" {"MANUAL"} else if design.is_some() {"AI"} else {"UNVERIFIED"},
        "sourceRefs":design.and_then(|value|value.get("sourceRefs")),
        "evidencePolicy":"已保存来源设计是外观一致性约束，不是新的原文事实；未提供图片像素。"
    });
    if let Some(proof) = super::asset_production_context::reference_design_choices(input, asset, design)? {
        card["freeDesignScope"] = design.unwrap()["freeDesignScope"].clone();
        card["designChoices"] = proof;
    }
    Ok(card)
}

pub(super) fn reusable_parent_design<'a>(
    input: &Value,
    asset: &'a Value,
    assets: &[Value],
    source_sha: &str,
    known_rows: &HashSet<&str>,
    evidence_ids: &HashSet<&str>,
    parent: Option<&str>,
) -> Option<&'a Value> {
    let design = asset.get("design")?;
    let original = super::asset_production_context::accepted_parent_input(input, asset).ok()?;
    if !super::asset_design_contract::design_receipt_current(original.as_ref().unwrap_or(input), asset)
        || text(design, "schemaVersion") != "asset-design/v1"
        || text(design, "sourceSha256") != source_sha
        || text(design, "assetId") != text(asset, "id")
        || text(design, "variantId") != text(asset, "variantId")
        || text(design, "prompt").trim().is_empty()
        || text(design, "prompt") != text(asset, "prompt")
    {
        return None;
    }
    let source_refs = exact_strings(design, "sourceRefs").ok()?;
    if source_refs.is_empty()
        || source_refs
            .iter()
            .any(|r| !known_rows.contains(r.as_str()) || !evidence_ids.contains(r.as_str()))
    {
        return None;
    }
    let references = exact_strings(design, "referenceAssetIds").ok()?;
    let resolved = exact_strings(design, "referenceVariantIds")
        .ok()?
        .iter()
        .map(|r| resolve_reference(assets, asset, r))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let mut seen = HashSet::new();
    if references != resolved
        || references
            .iter()
            .any(|r| r == text(asset, "id") || !seen.insert(r))
        || parent
            .map(|p| !references.iter().any(|r| r == p))
            .unwrap_or(false)
    {
        return None;
    }
    Some(design)
}

/// Pure preflight shared by direct image submission and one-click. A prompt is
/// never proof that unresolved source state has become applicable.
pub fn validate_asset_visual_gate(input: &Value, asset: &Value) -> Result<(), String> {
    let assets = input["assets"].as_array().ok_or("缺少资产目录")?;
    let parents = assets.iter().filter_map(|a|resolve_asset_dependency(assets,a).ok().map(|p|(text(a,"id").to_owned(),p))).collect();
    let by_id = assets.iter().map(|a|(text(a,"id"),a)).collect();
    let known = input["source"]["rows"].as_array().into_iter().flatten().filter_map(|r|r["id"].as_str()).collect();
    design_facts(input,asset,&by_id,&parents,&known).map(|_|())
}

pub fn image_production_policy(settings: &Value) -> &'static str {
    if settings["requireImageTrialReview"].as_bool() == Some(true) {
        "trialReview"
    } else {
        "automatic"
    }
}

/// One-click refreshes known AI designs only. Saved/manual/legacy prose is not
/// retrospectively relabeled as AI, and a missing source never invents evidence.
pub fn automatic_design_required(input: &Value, asset: &Value) -> bool {
    if input["activeImageAssetIds"]
        .as_array()
        .is_some_and(|ids| ids.contains(&asset["id"]))
    {
        return false;
    }
    automatic_design_stale(input, asset)
}

pub(super) fn automatic_design_stale(input: &Value, asset: &Value) -> bool {
    if text(asset, "prompt").trim().is_empty() {
        return true;
    }
    if text(asset, "promptOrigin") == "MANUAL"
        || (text(asset, "promptOrigin") != "AI"
            && text(&asset["design"], "schemaVersion") != "asset-design/v1")
    {
        return false;
    }
    match super::asset_production_context::frozen_version(input) {
        Ok(Some(version)) if asset["design"]["designReceipt"]["basis"]["assetProductionContext"]["version"] != version => {
            if super::asset_production_context::accepted_parent_input(input,asset).ok().flatten().is_none() {
                return true; // Only an implicit, exactly revalidated v1 source can cross profiles.
            }
        }
        Err(_) => return true,
        _ => {}
    }
    let Some(assets) = input["assets"].as_array() else {
        return true;
    };
    let known: HashSet<_> = input["source"]["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["id"].as_str())
        .collect();
    let source_sha = text(&input["source"], "sha256");
    let Ok(source_refs) = exact_strings(asset, "sourceRefs") else {
        return true;
    };
    if source_sha.is_empty()
        || source_refs.is_empty()
        || source_refs.iter().any(|r| !known.contains(r.as_str()))
    {
        return true;
    }
    let by_id: BTreeMap<_, _> = assets.iter().map(|a| (text(a, "id"), a)).collect();
    let Ok(parent) = resolve_asset_dependency(assets, asset) else {
        return true;
    };
    let parents: BTreeMap<_, _> = assets
        .iter()
        .filter_map(|a| {
            resolve_asset_dependency(assets, a)
                .ok()
                .map(|p| (text(a, "id").to_owned(), p))
        })
        .collect();
    let Ok((facts, _)) = design_facts(input, asset, &by_id, &parents, &known) else {
        return true;
    };
    let mut evidence: HashSet<_> = source_refs.iter().map(String::as_str).collect();
    for fact in &facts {
        evidence.extend(
            fact["sourceRefs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }
    let scope_source = serde_json::from_value::<LockedSource>(input["source"].clone()).ok();
    if let Some(source) = &scope_source {
        let Ok(context) = design_evidence::scene_context(input, source, asset, &facts, &evidence) else { return true; };
        for row in context {
            evidence.insert(row.id.as_str());
        }
    }
    let production_evidence = match super::asset_production_context::context(input, asset) {
        Ok(Some(context)) => {
            let Some(source) = &scope_source else {return true;};
            super::asset_production_context::evidence_rows(&context,source)
        },
        Ok(None) => Default::default(),
        Err(_) => return true,
    };
    evidence.extend(production_evidence.iter().map(String::as_str));
    reusable_parent_design(
        input,
        asset,
        assets,
        source_sha,
        &known,
        &evidence,
        parent.as_deref(),
    )
    .is_none()
}

pub async fn run_asset_design(
    input: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    inline_delta_enabled(input)?;
    let design_evidence_enabled = design_evidence::enabled(input)?;
    let incremental_comparisons = super::asset_design_contract::incremental_comparisons(input)?;
    let assets = input
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("资产设计输入缺少 assets")?;
    if assets.is_empty() {
        return Err("没有可设计的资产形象".into());
    }
    let source: LockedSource =
        serde_json::from_value(input.get("source").cloned().ok_or("缺少锁定原文")?)
            .map_err(|e| format!("原文结构无效：{e}"))?;
    validate_source(&source)?;
    let automatic = input["imageProductionPolicy"] == "automatic"
        || input["automaticDesign"].as_bool() == Some(true);
    let known_rows: HashSet<_> = source.rows.iter().map(|r| r.id.as_str()).collect();
    let mut by_id = BTreeMap::new();
    for asset in assets {
        let id = text(asset, "id");
        if id.is_empty() || text(asset, "variantId").is_empty() || by_id.insert(id, asset).is_some()
        {
            return Err("资产形象行 id 为空或重复，不能按姓名推断".into());
        }
    }
    let selected_assets = input
        .pointer("/selection/assetIds")
        .and_then(Value::as_array);
    let selected_variants = input
        .pointer("/selection/variantIds")
        .and_then(Value::as_array);
    let mut wanted: HashSet<String> = assets
        .iter()
        .filter(|a| {
            selected_assets
                .map(|ids| ids.iter().any(|id| id.as_str() == Some(text(a, "id"))))
                .unwrap_or_else(|| {
                    selected_variants
                        .map(|ids| {
                            ids.iter()
                                .any(|id| id.as_str() == Some(text(a, "variantId")))
                        })
                        .unwrap_or(true)
                })
        })
        .map(|a| text(a, "id").to_string())
        .collect();
    let explicitly_selected = if let Some(explicit) = super::asset_production_context::explicit_design_asset_ids(input)? {
        wanted.iter().filter(|id| explicit.iter().any(|value| value.as_str() == Some(id.as_str()))).cloned().collect()
    } else { wanted.clone() };
    let mut parents = BTreeMap::new();
    let mut parent_errors = BTreeMap::new();
    for asset in assets {
        match resolve_asset_dependency(assets, asset) {
            Ok(parent) => {
                parents.insert(text(asset, "id").to_string(), parent);
            }
            Err(reason) => {
                parent_errors.insert(text(asset, "id").to_string(), reason);
            }
        }
    }
    let mut next = wanted.clone();
    while !next.is_empty() {
        let mut added = HashSet::new();
        for id in next {
            if automatic
                && by_id.get(id.as_str()).is_some_and(|asset| {
                    inline_asset(asset) && !text(asset, "adoptedMediaId").is_empty()
                })
            {
                continue;
            }
            if let Some(Some(parent)) = parents.get(&id) {
                if wanted.insert(parent.clone()) {
                    added.insert(parent.clone());
                }
            }
        }
        next = added;
    }
    // INLINE is a performance/coverage state, not an independent image target.
    // Keep manually imported media and prompts untouched; this report describes
    // only the automatic design plan and does not create an adoption.
    let skipped: Vec<_> = assets.iter()
        .filter(|asset| wanted.contains(text(asset, "id")) && inline_asset(asset))
        .map(|asset| {
            let id = text(asset, "id");
            let parent = parents.get(id).and_then(|parent| parent.as_deref());
            let own_media = asset.get("adoptedMediaId").and_then(Value::as_str)
                .filter(|media| !media.is_empty());
            let reference_error = parent_errors.get(id).cloned().or_else(|| {
                parent.and_then(|parent| inline_reference_target(parent, &by_id, &parents).err())
            });
            let parent_media = parent.and_then(|parent| inline_reference_target(parent, &by_id, &parents).ok())
                .and_then(|parent| by_id.get(parent.as_str()))
                .and_then(|asset| asset.get("adoptedMediaId"))
                .and_then(Value::as_str).filter(|media| !media.is_empty());
            let status = if own_media.is_some() { "existingImageRetained" }
                else if reference_error.is_some() { "needsReference" }
                else if parent.is_some() { "reuseParentImage" } else { "textOnly" };
            let reason = if own_media.is_some() {
                "INLINE 已有采用图片保留，自动流程不另行设计或生成图片。".into()
            } else { reference_error.unwrap_or_else(|| {
                if parent.is_some() {
                    "INLINE 仅为表演或覆盖态，复用指定来源图片，不独立设计或生成图片。".into()
                } else {
                    "INLINE 无独立来源图片，系统窗口或规则素材由分镜文字表现，不独立设计或生成图片。".into()
                }
            }) };
            json!({"assetId":id,"variantId":asset["variantId"],"mode":"INLINE","status":status,"sourceAssetId":parent,"adoptedMediaId":own_media.or(parent_media),"reason":reason})
        }).collect();
    let cached_visual_ids = super::asset_visual_facts::cached_visual_check_asset_ids(input)?;
    let visual_assets: Vec<_> = assets
        .iter()
        .filter(|asset| {
            if !wanted.contains(text(asset, "id")) || inline_asset(asset) {
                return false;
            }
            if automatic
                && !automatic_design_required(input, asset)
                && !cached_visual_ids.iter().any(|id| id == text(asset, "id"))
            {
                return false;
            }
            let mut seen = HashSet::new();
            let mut cursor = Some(text(asset, "id"));
            while let Some(id) = cursor {
                if !seen.insert(id) || parent_errors.contains_key(id) {
                    return false;
                }
                cursor = parents.get(id).and_then(|p| p.as_deref());
            }
            exact_strings(asset, "sourceRefs")
                .map(|refs| {
                    !refs.is_empty() && refs.iter().all(|row| known_rows.contains(row.as_str()))
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    let visual = if visual_assets.is_empty() {
        json!({"facts":input.pointer("/foundation/facts").cloned().unwrap_or(json!([])),"blocked":[],"visualFactBatches":[]})
    } else {
        let mut visual_input = input.clone();
        visual_input["assetRegistry"] = input["assets"].clone();
        visual_input["assets"] = json!(visual_assets);
        super::asset_visual_facts::run_asset_visual_facts(&visual_input, executor, model, reasoning)
            .await?
    };
    let mut design_basis = input.clone();
    if !design_basis["foundation"].is_object() {
        design_basis["foundation"] = json!({});
    }
    design_basis["foundation"]["facts"] = visual["facts"].clone();
    if input.get(super::asset_production_context::PROFILE_FIELD).is_some_and(|value| !value.is_null()) {
        // Only the new local design basis receives fresh validated receipts.
        // Original workflow/source bytes and A15 repair budgets stay untouched.
        let mut receipts = input["foundation"]["visualFactBatches"].as_array().cloned().unwrap_or_default();
        for receipt in visual["visualFactBatches"].as_array().into_iter().flatten() {
            receipts.retain(|old| old["inputFingerprint"] != receipt["inputFingerprint"]);
            receipts.push(receipt.clone());
        }
        design_basis["foundation"]["visualFactBatches"] = json!(receipts);
    }
    let prop_dependencies = super::asset_production_context::candidates(&design_basis)?;
    let mut pending: BTreeMap<_, _> = by_id
        .iter()
        .filter(|(key, asset)| wanted.contains(**key) && !inline_asset(asset))
        .map(|(k, v)| ((*k).to_string(), *v))
        .collect();
    let mut blocked = visual["blocked"].as_array().cloned().unwrap_or_default();
    let mut failed: HashSet<String> = blocked
        .iter()
        .map(|item| text(item, "assetId").to_string())
        .collect();
    pending.retain(|id, _| !failed.contains(id));
    // Freeze a deterministic topological character lane, independent of which
    // unrelated non-character future happens to finish first. Other kinds still
    // use all available slots. Existing dependency errors retain their handling.
    let mut comparison_order = Vec::new();
    if incremental_comparisons {
        let mut remaining: BTreeMap<String, HashSet<String>> = pending.iter().map(|(id, _)| {
            let mut dependencies = HashSet::new();
            if let Some(Some(parent)) = parents.get(id) {
                if let Ok(parent) = inline_reference_target(parent, &by_id, &parents) {
                    if pending.contains_key(&parent) { dependencies.insert(parent); }
                }
            }
            for prop in prop_dependencies.get(id).into_iter().flatten() {
                if let Some(prop) = prop["assetId"].as_str().filter(|p| pending.contains_key(*p)) {
                    dependencies.insert(prop.to_owned());
                }
            }
            (id.clone(), dependencies)
        }).collect();
        while !remaining.is_empty() {
            let Some(next) = remaining.iter().find(|(_, dependencies)| dependencies.is_empty()).map(|(id,_)|id.clone()) else {
                comparison_order.extend(remaining.keys().filter(|id| super::asset_design_contract::is_character(pending[*id])).cloned());
                break;
            };
            remaining.remove(&next);
            for dependencies in remaining.values_mut() { dependencies.remove(&next); }
            if super::asset_design_contract::is_character(pending[&next]) { comparison_order.push(next); }
        }
    }
    let mut designs: Vec<Value> = Vec::new();
    let mut available_designs: BTreeMap<String, Value> = BTreeMap::new();
    for asset in assets
        .iter()
        .filter(|asset| wanted.contains(text(asset, "id")) && inline_asset(asset))
    {
        if !text(asset, "prompt").trim().is_empty()
            && (text(asset, "promptOrigin") == "MANUAL"
                || asset.get("design").map(Value::is_null).unwrap_or(true))
        {
            available_designs.insert(
                text(asset, "id").to_owned(),
                reference_design_card(&design_basis, asset, None)?,
            );
        } else if !text(asset, "prompt").trim().is_empty() {
            // Historic INLINE AI designs are valid donors only under their
            // exact accepted receipt. Never downgrade failed AI proof to prose.
            let parent = parents.get(text(asset,"id")).and_then(|p|p.as_deref());
            let mut evidence: HashSet<_> = asset["sourceRefs"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if let Ok((facts, _)) = design_facts(&design_basis, asset, &by_id, &parents, &known_rows) {
                for fact in &facts {
                    evidence.extend(fact["sourceRefs"].as_array().into_iter().flatten().filter_map(Value::as_str));
                }
                let Ok(scope_context) = design_evidence::scene_context(&design_basis, &source, asset, &facts, &evidence) else { continue; };
                evidence.extend(scope_context.iter().map(|row|row.id.as_str()));
                let Ok(production_context) = super::asset_production_context::context(&design_basis,asset) else { continue; };
                let production_rows = production_context.as_ref().map(|context|super::asset_production_context::evidence_rows(context,&source)).unwrap_or_default();
                evidence.extend(production_rows.iter().map(String::as_str));
                if let Some(design) = reusable_parent_design(&design_basis, asset, assets, &source.sha256, &known_rows, &evidence, parent) {
                    available_designs.insert(text(asset,"id").to_owned(), reference_design_card(&design_basis,asset,Some(design))?);
                }
            }
        }
    }
    let mut in_flight: Vec<(
        String,
        Pin<Box<dyn Future<Output = Result<Value, String>> + Send + '_>>,
    )> = Vec::new();
    while !pending.is_empty() {
        let mut progress = false;
        let keys: Vec<_> = pending.keys().cloned().collect();
        for id in keys {
            if in_flight.len() >= ASSET_DESIGN_CONCURRENCY {
                break;
            }
            if in_flight.iter().any(|(running, _)| running == &id) {
                continue;
            }
            let asset = pending[&id];
            if incremental_comparisons && super::asset_design_contract::is_character(asset)
                && comparison_order.iter().find(|candidate| pending.contains_key(*candidate)) != Some(&id)
            { continue; }
            let variant = text(asset, "variantId");
            let parent = parents.get(&id).and_then(|p| p.as_deref());
            if automatic
                && input["activeImageAssetIds"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&asset["id"]))
            {
                if automatic_design_stale(input, asset) {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":"已有图片任务正在处理或等待未知提交核对；不改写其设计，关联项等待该来源版本有效"}));
                    failed.insert(id.clone());
                } else {
                    available_designs.insert(
                        id.clone(),
                        reference_design_card(
                            &design_basis,
                            asset,
                            asset.get("design").filter(|design| {
                                text(design, "schemaVersion") == "asset-design/v1"
                            }),
                        )?,
                    );
                }
                pending.remove(&id);
                progress = true;
                continue;
            }
            if let Some(reason) = parent_errors.get(&id) {
                blocked.push(json!({"assetId":id,"variantId":variant,"reason":reason}));
                failed.insert(id.clone());
                pending.remove(&id);
                progress = true;
                continue;
            }
            let reference_parent = match parent
                .map(|parent| inline_reference_target(parent, &by_id, &parents))
                .transpose()
            {
                Ok(parent) => parent,
                Err(reason) => {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":reason}));
                    failed.insert(id.clone());
                    pending.remove(&id);
                    progress = true;
                    continue;
                }
            };
            if let Some(reference_parent) = reference_parent.as_deref() {
                if failed.contains(reference_parent) {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":"来源形象设计失败","sourceAssetId":parent}));
                    failed.insert(id.clone());
                    pending.remove(&id);
                    progress = true;
                    continue;
                }
                if pending.contains_key(reference_parent) {
                    continue;
                }
                if !available_designs.contains_key(reference_parent) {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":"指定来源没有有效提示词；已保存文字保留，请核对来源设计","sourceAssetId":parent}));
                    failed.insert(id.clone());
                    pending.remove(&id);
                    progress = true;
                    continue;
                }
            }
            if let Err(reason) = validate_asset_visual_gate(&design_basis,asset) {
                blocked.push(json!({"assetId":id,"variantId":variant,"reason":reason}));
                failed.insert(id.clone()); pending.remove(&id); progress = true; continue;
            }
            if (automatic || !explicitly_selected.contains(&id))
                && !text(asset, "prompt").trim().is_empty()
            {
                let manual = text(asset, "promptOrigin") == "MANUAL";
                let no_ai_design = if automatic {
                    text(asset, "promptOrigin") != "AI"
                        && text(&asset["design"], "schemaVersion") != "asset-design/v1"
                } else {
                    asset.get("design").map(Value::is_null).unwrap_or(true)
                };
                if manual || no_ai_design {
                    // A direct saved reference is usable without pretending it is
                    // an accepted AI result. Never rewrite an implicit dependency.
                    available_designs.insert(id.clone(), reference_design_card(&design_basis, asset, None)?);
                    pending.remove(&id);
                    progress = true;
                    continue;
                }
            }
            let source_refs = match exact_strings(asset, "sourceRefs") {
                Ok(refs)
                    if !refs.is_empty() && refs.iter().all(|r| known_rows.contains(r.as_str())) =>
                {
                    refs
                }
                _ => {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":"形象缺少当前锁定来源引用"}));
                    failed.insert(id.clone());
                    pending.remove(&id);
                    progress = true;
                    continue;
                }
            };
            // Shared-source prop candidates are text consistency inputs only.
            // Wait for related original targets; never add targets or images.
            // Manual/reused implicit drafts above keep their original handling.
            if prop_dependencies.get(&id).is_some_and(|props| props.iter().any(|prop| {
                prop["assetId"].as_str().is_some_and(|prop_id| pending.contains_key(prop_id) && !failed.contains(prop_id))
            })) {
                continue;
            }
            let (facts, delta_facts) =
                match design_facts(&design_basis, asset, &by_id, &parents, &known_rows) {
                    Ok(facts) => facts,
                    Err(reason) => {
                        blocked.push(json!({"assetId":id,"variantId":variant,"reason":reason}));
                        failed.insert(id.clone());
                        pending.remove(&id);
                        progress = true;
                        continue;
                    }
                };
            let mut evidence_ids: HashSet<&str> = source_refs.iter().map(String::as_str).collect();
            for fact in &facts {
                if let Some(rs) = fact.get("sourceRefs").and_then(Value::as_array) {
                    for r in rs.iter().filter_map(Value::as_str) {
                        evidence_ids.insert(r);
                    }
                }
            }
            let scope_context = match design_evidence::scene_context(&design_basis, &source, asset, &facts, &evidence_ids) {
                Ok(context) => context,
                Err(reason) => {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":reason}));
                    failed.insert(id.clone()); pending.remove(&id); progress = true; continue;
                }
            };
            for row in &scope_context {
                evidence_ids.insert(row.id.as_str());
            }
            if design_basis.get(super::asset_production_context::PROFILE_FIELD).is_some() {
                let mut unavailable: Vec<_> = failed.iter().cloned().collect();
                unavailable.sort();
                design_basis["stage2UnavailablePropAssetIds"] = json!(unavailable);
            }
            let production_context = super::asset_production_context::context(&design_basis, asset)?;
            let production_evidence = production_context.as_ref().map(|context|super::asset_production_context::evidence_rows(context,&source)).unwrap_or_default();
            for row in &production_evidence { evidence_ids.insert(row.as_str()); }
            if automatic || !explicitly_selected.contains(&id) {
                let parent_redesigned = reference_parent.as_deref().is_some_and(|parent| {
                    designs
                        .iter()
                        .any(|design| text(design, "assetId") == parent)
                });
                if let Some(design) = reusable_parent_design(
                    &design_basis,
                    asset,
                    assets,
                    &source.sha256,
                    &known_rows,
                    &evidence_ids,
                    parent,
                )
                .filter(|_| !automatic || !parent_redesigned)
                {
                    available_designs
                        .insert(id.clone(), reference_design_card(&design_basis, asset, Some(design))?);
                    pending.remove(&id);
                    progress = true;
                    continue;
                }
                if !automatic && !text(asset, "prompt").trim().is_empty() {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":"来源形象的旧自动设计已失效，请先核对或明确重新设计；当前提示词已保留"}));
                    failed.insert(id.clone());
                    pending.remove(&id);
                    progress = true;
                    continue;
                }
            }
            let evidence: Vec<_> = source
                .rows
                .iter()
                .filter(|r| evidence_ids.contains(r.id.as_str()))
                .collect();
            let parent_design = reference_parent
                .as_deref()
                .and_then(|p| available_designs.get(p));
            let mode = if parent.is_some() { "DERIVED" } else { "BASE" };
            let production_direction = super::asset_design_contract::production_direction(input);
            let design_receipt = match super::asset_design_contract::design_receipt(&design_basis, asset) {
                Ok(receipt) => receipt,
                Err(reason) if design_evidence_enabled => {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":reason}));
                    failed.insert(id.clone()); pending.remove(&id); progress = true; continue;
                }
                Err(reason) => return Err(reason),
            };
            let frozen_request = incremental_comparisons.then(||input[super::asset_design_contract::FROZEN_COMPARISON_REQUESTS].get(&id)).flatten();
            let comparison_designs = if let Some(saved) = frozen_request {
                saved["input"]["comparisonDesigns"].as_array().cloned().ok_or("已派发人物设计缺少冻结对照")?
            } else if incremental_comparisons {
                // Future selected characters must not leak into an earlier
                // request when a resumed image workflow already published them.
                let mut comparison_basis = design_basis.clone();
                if let Some(rows) = comparison_basis["assets"].as_array_mut() {
                    for row in rows {
                        if pending.contains_key(text(row,"id")) || failed.contains(text(row,"id")) {
                            row["prompt"] = json!("");
                        }
                    }
                }
                super::asset_design_contract::comparison_designs(&comparison_basis, asset)
            } else {
                input["comparisonDesignsByAssetId"].get(&id).and_then(Value::as_array).cloned().unwrap_or_else(||super::asset_design_contract::comparison_designs(input, asset))
            };
            let allowed_references: Vec<String> = parent.into_iter().map(str::to_string).collect();
            let output_schema = obj(
                json!({"schemaVersion":choices(&["asset-design/v1"]),"assetId":{"type":"string","enum":[id]},"variantId":{"type":"string","enum":[variant]},"prompt":string(),"freeDesignScope":string(),"sourceRefs":refs(&evidence.iter().map(|r|r.id.clone()).collect::<Vec<_>>()),"referenceVariantIds":refs(&allowed_references)}),
            );
            let asset_card = asset_design_card(&design_basis, asset)?;
            let parent_card = parent.and_then(|p|by_id.get(p)).map(|asset|asset_design_card(&design_basis, asset)).transpose()?;
            let mut design_input = json!({"sourceSha256":source.sha256,"designMode":mode,"asset":asset_card,"facts":facts,"deltaFacts":delta_facts,"evidence":evidence,"parentDesign":parent_design,"parentAsset":parent_card,"referenceCatalog":parent_card.into_iter().collect::<Vec<_>>(),"productionDirection":production_direction,"imageEvidence":"NO_PIXELS_READ","instruction":"执行designMode确定的C基础/D派生岗位。referenceVariantIds只使用指定直接来源资产行id；基础形象为空。名称只作显示，不推断身份。原文仅在evidence物化一次；facts与deltaFacts不重抄原句。未提供当前旧提示词与历史候选，不以历史代替证据。"});
            if design_input["asset"].get("identityProductionMaterial").is_some()
                || design_input["parentAsset"].get("identityProductionMaterial").is_some()
                || design_input["asset"].get("scopeProductionMaterial").is_some()
                || design_input["parentAsset"].get("scopeProductionMaterial").is_some()
            {
                design_input["identityProductionMaterialPolicy"] = json!(super::asset_design_contract::IDENTITY_MATERIAL_POLICY);
            }
            design_input["projectVisualStyle"] = json!(super::asset_design_contract::project_visual_style(&input["settings"]));
            design_input["imagePresentation"] = super::asset_design_contract::image_presentation(&input["settings"], asset);
            design_input["comparisonDesigns"] = json!(comparison_designs);
            if incremental_comparisons {
                design_input[super::asset_design_contract::COMPARISON_PROFILE_FIELD] = json!(super::asset_design_contract::COMPARISON_PROFILE);
            }
            if !scope_context.is_empty() && !design_evidence_enabled {
                design_input["sourceScopeContext"] = json!({"version":"asset-scene-context/v1","instruction":"evidence包含SOURCE_SCOPE标记所定位场次的锁定原文，不再只提供场次标题。先在原文中确认当前主体及其物种/空间/物件身份；空facts仅表示尚无已登记外观约束，不表示原文没有该主体。仍按原C/D岗位区分原文依据与普通非剧情美术补足，补足写入freeDesignScope；不得把同场其它主体或后期状态移植给当前形象，也不得把整场原文标成该资产的外观事实。"});
            }
            let mut frozen_context = json!({"version":super::asset_design_contract::DESIGN_PROFILE,"projectVisualStyle":design_input["projectVisualStyle"],"productionDirection":design_input["productionDirection"],"imagePresentation":design_input["imagePresentation"],"comparisonDesigns":design_input["comparisonDesigns"],"model":model,"reasoning":reasoning,"textProvider":input["settings"]["textProvider"],"textBackend":input["settings"]["textBackend"]});
            if let Some(binding) = design_receipt["basis"].get("designEvidenceContext") {
                design_input["designEvidenceContext"] = design_evidence::request_context(input, binding);
                frozen_context["designEvidenceContext"] = binding.clone();
            }
            if incremental_comparisons {
                frozen_context[super::asset_design_contract::COMPARISON_PROFILE_FIELD] = json!(super::asset_design_contract::COMPARISON_PROFILE);
            }
            if let Some(context) = production_context {
                design_input["assetProductionContext"] = context.clone();
                frozen_context["assetProductionContext"] = context;
            }
            for (card, key) in [("asset", "assetIdentityProductionMaterial"), ("parentAsset", "parentIdentityProductionMaterial")] {
                if let Some(material) = design_input[card].get("identityProductionMaterial") {
                    frozen_context[key] = material.clone();
                }
            }
            for (card, key) in [("asset", "assetScopeProductionMaterial"), ("parentAsset", "parentScopeProductionMaterial")] {
                if let Some(material) = design_input[card].get("scopeProductionMaterial") { frozen_context[key] = material.clone(); }
            }
            if reference_parent.as_deref() != parent {
                design_input["inlineReference"] = json!({"directAssetId":parent,"reusedDesignAssetId":reference_parent,"instruction":"直接来源为INLINE覆盖态，parentDesign复用其明确来源形象；referenceVariantIds仍按指定直接来源id输出，不新增独立INLINE图片。"});
                if let Some(context) = inline_delta_context(&design_basis, asset)? {
                    design_input["inlineReference"] = context.clone();
                    design_input["inlineReference"]["instruction"] = json!("parentDesign是实际复用祖先的已接受设计；deltaFacts已包含该设计之后被跳过INLINE层与当前子形象的全部当前有效变化，同属性以较近当前形象为准。按deltaFacts修改祖先稿，保留未变身份；referenceVariantIds仍只输出指定直接来源id，不新增INLINE图片或事实。");
                    frozen_context["inlineDeltaContext"] = context;
                }
            }
            if has_program_source_scope(asset)
                || parent
                    .and_then(|id| by_id.get(id))
                    .is_some_and(|asset| has_program_source_scope(asset))
                || design_evidence_enabled
            {
                design_input["sourceEvidencePolicy"] = json!(SOURCE_SCOPE_POLICY);
                let scope_ids: HashSet<_> = if has_program_source_scope(asset) {
                    source_refs.iter().map(String::as_str).collect()
                } else {
                    HashSet::new()
                };
                let fact_ids: HashSet<_> = design_input["facts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|fact| {
                        fact["sourceRefs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                    })
                    .map(str::to_string)
                    .collect();
                for row in design_input["evidence"]
                    .as_array_mut()
                    .into_iter()
                    .flatten()
                {
                    let id = text(row, "id");
                    let mut uses = Vec::new();
                    if scope_ids.contains(id) {
                        uses.push("SOURCE_SCOPE");
                    }
                    if fact_ids.contains(id) {
                        uses.push("VISUAL_FACT_REFERENCE");
                    }
                    if scope_context.iter().any(|row| row.id == id) || production_evidence.contains(id) {
                        uses.push("SOURCE_CONTEXT");
                    }
                    row["evidenceUses"] = json!(uses);
                }
            }
            if let Some(saved) = frozen_request {
                if super::asset_design_contract::comparison_domain_input(&saved["input"]) != design_input || saved["schema"] != output_schema
                    || saved["model"] != model || saved["reasoning"] != reasoning
                    || saved["ruleSnapshot"] != snapshot("asset-art-director", &[])?
                { return Err("已派发人物设计的冻结领域请求已变化，保留原请求且不重发".into()); }
            }
            let fingerprint = digest(&serde_json::to_string(&design_input).unwrap());
            // Own the validation inputs so futures do not borrow temporary facts
            // or the mutable ready/completed maps. A parent stays pending until
            // call_validated has also saved its accepted result.
            let validation_id = id.clone();
            let validation_variant = variant.to_owned();
            let validation_evidence: HashSet<String> =
                evidence_ids.iter().map(|id| (*id).to_owned()).collect();
            let source_sha = source.sha256.clone();
            let response = Box::pin(async move {
                call_validated(
                    executor,
                    "assetDesign",
                    "asset-art-director",
                    model,
                    reasoning,
                    design_input,
                    output_schema,
                    &[],
                    |output| {
                        if text(output, "schemaVersion") != "asset-design/v1"
                            || text(output, "assetId") != validation_id
                            || text(output, "variantId") != validation_variant
                            || text(output, "prompt").trim().is_empty()
                            || text(output, "prompt").chars().count() > 14_000
                            || output
                                .get("freeDesignScope")
                                .and_then(Value::as_str)
                                .is_none()
                        {
                            return Err("模型回包身份不匹配或缺少有效提示词".into());
                        }
                        let rs = exact_strings(output, "sourceRefs")?;
                        let mut seen_refs = HashSet::new();
                        if rs.is_empty()
                            || rs.iter().any(|r| {
                                !validation_evidence.contains(r.as_str()) || !seen_refs.insert(r)
                            })
                        {
                            return Err("资产设计引用越出当前提供源证据或重复".into());
                        }
                        let resolved = exact_strings(output, "referenceVariantIds")?
                            .iter()
                            .map(|r| resolve_reference(assets, asset, r))
                            .collect::<Result<Vec<_>, _>>()?;
                        if resolved != allowed_references {
                            return Err("图片依赖必须与指定直接来源一致，不能扩大或省略".into());
                        }
                        let map = output.as_object_mut().ok_or("资产设计回包必须为对象")?;
                        map.insert("referenceAssetIds".into(), json!(resolved));
                        map.insert("sourceSha256".into(), json!(source_sha));
                        map.insert("status".into(), json!("candidate"));
                        map.insert("designMode".into(), json!(mode));
                        map.insert("inputFingerprint".into(), json!(fingerprint));
                        map.insert("designReceipt".into(), design_receipt.clone());
                        map.insert("frozenContext".into(), frozen_context.clone());
                        Ok(())
                    },
                )
                .await
            });
            in_flight.push((id, response));
            progress = true;
        }
        if !in_flight.is_empty() {
            let (completed, response) = poll_fn(|context| {
                for (index, (_, future)) in in_flight.iter_mut().enumerate() {
                    if let Poll::Ready(response) = future.as_mut().poll(context) {
                        return Poll::Ready((index, response));
                    }
                }
                Poll::Pending
            })
            .await;
            let (id, _) = in_flight.remove(completed);
            let asset = by_id[id.as_str()];
            let variant = text(asset, "variantId");
            match response {
                Ok(output) => {
                    let mut updated_asset = (*asset).clone();
                    updated_asset["prompt"] = output["prompt"].clone();
                    updated_asset["promptOrigin"] = json!("AI");
                    updated_asset["design"] = output.clone();
                    if let Some(rows) = design_basis["assets"].as_array_mut() {
                        if let Some(saved) = rows.iter_mut().find(|a| text(a,"id")==id) { *saved = updated_asset.clone(); }
                    }
                    available_designs.insert(
                        id.clone(),
                        reference_design_card(&design_basis, &updated_asset, Some(&output))?,
                    );
                    designs.push(output);
                }
                Err(reason) => {
                    blocked.push(json!({"assetId":id,"variantId":variant,"reason":reason}));
                    failed.insert(id.clone());
                }
            }
            pending.remove(&id);
            continue;
        }
        if !progress {
            for (id, asset) in &pending {
                blocked.push(json!({"assetId":id,"variantId":text(asset,"variantId"),"reason":"形象依赖循环，需修订来源关系"}));
            }
            break;
        }
    }
    designs.sort_by(|a, b| text(a, "assetId").cmp(text(b, "assetId")));
    let trial_approved = input
        .pointer("/settings/trialApproved")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let policy = input["imageProductionPolicy"]
        .as_str()
        .filter(|p| ["automatic", "trialReview"].contains(p))
        .unwrap_or_else(|| image_production_policy(&input["settings"]));
    let needs_trial = policy == "trialReview" && !trial_approved;
    let image_plan:Vec<_>=designs.iter().map(|design|{let id=text(design,"assetId");let dependencies=design["referenceAssetIds"].clone();let missing:Vec<_>=dependencies.as_array().unwrap().iter().filter_map(Value::as_str).filter(|parent|!adopted_reference_available(assets,parent)).collect();json!({"assetId":id,"variantId":design["variantId"],"dependencies":dependencies,"prompt":design["prompt"],"imageProductionPolicy":policy,"status":if needs_trial{"awaitingTrial"}else if !missing.is_empty(){"waitingAdoption"}else{"ready"},"missingAdoption":missing,"candidateCount":if needs_trial{1}else{input.pointer("/settings/imageCount").and_then(Value::as_u64).unwrap_or(1)}})}).collect();
    let updated: Vec<_> = designs
        .iter()
        .map(|design| {
            let mut asset = (*by_id[text(design, "assetId")]).clone();
            asset["prompt"] = design["prompt"].clone();
            asset["promptOrigin"] = json!("AI");
            asset["designStatus"] = json!("candidate");
            asset["design"] = design.clone();
            if let Some(Some(parent)) = parents.get(text(design, "assetId")) {
                asset["sourceAssetId"] = json!(parent);
            }
            asset
        })
        .collect();
    Ok(
        json!({"schemaVersion":"asset-design-batch/v1","sourceSha256":source.sha256,"assets":updated,"designs":designs,"imagePlan":image_plan,"imageProductionPolicy":policy,"facts":visual["facts"],"visualFactBatches":visual["visualFactBatches"],"visualFactCacheReuses":visual["visualFactCacheReuses"].as_array().cloned().unwrap_or_default(),"blocked":blocked,"skipped":skipped,"trialApproved":trial_approved,"ruleSnapshots":[rule_snapshot("asset-visual-facts-extractor")?,rule_snapshot("asset-art-director")?,rule_snapshot("asset-image-production")?]}),
    )
}

#[cfg(test)]
#[path = "assets_inline_delta_tests.rs"]
mod inline_delta_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_context_only_extends_empty_program_scope_packets() {
        let source = prepare_source("第一集\n场景：湖边\n△一条变异鲟鱼游过。\n第二集\n场景：屋内\n△小王穿着红衣。\n").unwrap();
        let marker = source.rows.iter().find(|row| row.kind == "format" && row.scene_id.is_some()).unwrap();
        let body = source.rows.iter().find(|row| row.text.contains("鲟鱼")).unwrap();
        let asset = json!({"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceSha256":source.sha256}});
        let marker_only = HashSet::from([marker.id.as_str()]);
        let rows = empty_design_scene_context(&source, &asset, &[], &marker_only);
        assert!(rows.iter().any(|row| row.id == body.id));
        assert!(rows.iter().all(|row| row.scene_id == marker.scene_id));
        assert!(empty_design_scene_context(&source, &asset, &[json!({"value":"已登记事实"})], &marker_only).is_empty(), "fact-backed original requests must not gain new evidence");
        assert!(empty_design_scene_context(&source, &asset, &[], &HashSet::from([body.id.as_str()])).is_empty(), "a packet already carrying locked body text remains unchanged");
        assert!(empty_design_scene_context(&source, &json!({}), &[], &marker_only).is_empty(), "ordinary references never expand implicitly");
    }
    fn empty_visual(request: &TextRequest) -> Value {
        json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":[],"issues":[]})
    }
    #[derive(Default)]
    struct ConcurrentDesigner {
        active: std::sync::atomic::AtomicUsize,
        peak: std::sync::atomic::AtomicUsize,
        facts_calls: std::sync::atomic::AtomicUsize,
        visual_inputs: std::sync::Mutex<Vec<Value>>,
        requests: std::sync::Mutex<Vec<Value>>,
        accepted: std::sync::Mutex<HashSet<String>>,
        events: std::sync::Mutex<Vec<String>>,
        slow_released: std::sync::atomic::AtomicBool,
        release_slow: tokio::sync::Notify,
    }
    #[async_trait]
    impl TextExecutor for ConcurrentDesigner {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            use std::sync::atomic::Ordering;
            if request.stage == "assetVisualFacts" {
                self.facts_calls.fetch_add(1, Ordering::SeqCst);
                self.visual_inputs
                    .lock()
                    .unwrap()
                    .push(request.input.clone());
                return Ok(empty_visual(&request));
            }
            assert_eq!(request.stage, "assetDesign");
            assert_eq!(request.skill, "asset-art-director");
            let asset = &request.input["asset"];
            let id = text(asset, "id");
            if let Some(parent) = request.input["parentDesign"]["assetId"].as_str() {
                if request.input["parentDesign"]["origin"] == "AI" {
                    assert!(
                        self.accepted.lock().unwrap().contains(parent),
                        "derived design started before parent acceptance: {parent} -> {id}"
                    );
                }
                assert!(!text(&request.input["parentDesign"], "prompt")
                    .trim()
                    .is_empty());
            }
            self.requests.lock().unwrap().push(request.input.clone());
            self.events.lock().unwrap().push(format!("start:{id}"));
            if id == "good-child" {
                self.slow_released.store(true, Ordering::SeqCst);
                self.release_slow.notify_waiters();
            }
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            if id.starts_with("z-slow") {
                let notified = self.release_slow.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if !self.slow_released.load(Ordering::SeqCst) {
                    tokio::time::timeout(std::time::Duration::from_secs(2), notified)
                        .await
                        .expect("ready child was blocked behind unrelated pending work");
                }
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.events.lock().unwrap().push(format!("finish:{id}"));
            if id == "b-parent" {
                return Err("offline local parent failure".into());
            }
            let refs = request.input["parentAsset"]["id"]
                .as_str()
                .map(|id| vec![id])
                .unwrap_or_default();
            Ok(
                json!({"schemaVersion":"asset-design/v1","assetId":id,"variantId":asset["variantId"],"prompt":format!("{id}源证据约束下完整形象"),"freeDesignScope":"背景灰度","sourceRefs":asset["sourceRefs"],"referenceVariantIds":refs}),
            )
        }
        async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
            if request.stage == "assetDesign" {
                let id = text(value, "assetId");
                if id == "a-parent" {
                    // Saving/accepting a valid parent is part of the dependency gate.
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                self.accepted.lock().unwrap().insert(id.into());
                self.events.lock().unwrap().push(format!("accept:{id}"));
            }
            Ok(())
        }
    }
    fn design_asset(id: &str, row: &str, parent: Option<&str>) -> Value {
        let mut asset = json!({"id":id,"baseAssetId":id,"variantId":"base","name":id,"kind":"CH","sourceRefs":[row],"preparation":{"mode":if parent.is_some(){"DERIVE"}else{"REFERENCE"}}});
        if let Some(parent) = parent {
            asset["sourceAssetId"] = json!(parent);
        }
        asset
    }
    #[tokio::test]
    async fn fifty_four_independent_designs_use_four_slots_and_one_shared_a15_without_inline_targets(
    ) {
        use std::sync::atomic::Ordering;
        let source = prepare_source("第一集\n场景：门口\n△小王出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let mut assets: Vec<_> = (0..54)
            .map(|n| design_asset(&format!("asset-{n:02}"), &row, None))
            .collect();
        assets[0]["adoptedMediaId"] = json!("existing-parent-media");
        for n in 0..4 {
            let mut inline = design_asset(
                &format!("inline-{n}"),
                &row,
                if n < 2 { Some("asset-00") } else { None },
            );
            inline["preparation"] = json!({"mode":"INLINE"});
            assets.push(inline);
        }
        let input = json!({"source":source,"assets":assets});
        let before = input.clone();
        let executor = ConcurrentDesigner::default();
        let output = run_asset_design(&input, &executor, "offline-test-model", "medium")
            .await
            .unwrap();
        assert!(output["blocked"].as_array().unwrap().is_empty());
        assert_eq!(executor.peak.load(Ordering::SeqCst), 4);
        assert_eq!(executor.facts_calls.load(Ordering::SeqCst), 1);
        for field in ["assets", "designs", "imagePlan"] {
            assert_eq!(output[field].as_array().unwrap().len(), 54);
            assert!(output[field].as_array().unwrap().iter().all(|item| {
                !text(item, if field == "assets" { "id" } else { "assetId" }).starts_with("inline-")
            }));
        }
        assert_eq!(output["skipped"].as_array().unwrap().len(), 4);
        assert_eq!(output["skipped"][0]["status"], "reuseParentImage");
        assert_eq!(
            output["skipped"][0]["adoptedMediaId"],
            "existing-parent-media"
        );
        assert_eq!(output["skipped"][2]["status"], "textOnly");
        let requests = executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 54);
        let ids: HashSet<_> = requests
            .iter()
            .map(|request| text(&request["asset"], "id"))
            .collect();
        assert_eq!(
            ids.len(),
            54,
            "successful independent designs must not be resubmitted"
        );
        assert!(
            !serde_json::to_string(&executor.visual_inputs.lock().unwrap()[0])
                .unwrap()
                .contains("inline-")
        );
        assert_eq!(
            input, before,
            "manual media/input records are never mutated by design planning"
        );
    }
    #[tokio::test]
    async fn ready_children_start_after_accepted_parent_while_failed_branches_stay_local() {
        let source = prepare_source("第一集\n场景：门口\n△小王出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let assets = vec![
            design_asset("good-child", &row, Some("a-parent")),
            design_asset("bad-child", &row, Some("b-parent")),
            design_asset("a-parent", &row, None),
            design_asset("b-parent", &row, None),
            design_asset("z-slow-1", &row, None),
            design_asset("z-slow-2", &row, None),
            design_asset("z-slow-3", &row, None),
        ];
        let executor = ConcurrentDesigner::default();
        let output = run_asset_design(
            &json!({"source":source,"assets":assets}),
            &executor,
            "offline-test-model",
            "medium",
        )
        .await
        .unwrap();
        let blocked: HashSet<_> = output["blocked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| text(item, "assetId"))
            .collect();
        assert_eq!(blocked, HashSet::from(["b-parent", "bad-child"]));
        assert_eq!(output["designs"].as_array().unwrap().len(), 5);
        let requests = executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 6);
        assert!(!requests
            .iter()
            .any(|request| request["asset"]["id"] == "bad-child"));
        let events = executor.events.lock().unwrap();
        let position = |event: &str| events.iter().position(|value| value == event).unwrap();
        assert!(position("accept:a-parent") < position("start:good-child"));
        assert!(
            position("start:good-child") < position("finish:z-slow-1"),
            "a ready child should not wait for an unrelated full batch to finish"
        );
        assert!(executor.peak.load(std::sync::atomic::Ordering::SeqCst) <= 4);
    }
    #[tokio::test]
    async fn inline_only_plans_keep_manual_media_and_prompts_without_any_model_call() {
        let source = prepare_source("第一集\n场景：门口\n△小王出现。\n").unwrap();
        let input = json!({"source":source,"selection":{"assetIds":["inline-cover","inline-window","inline-manual"]},"assets":[
            {"id":"manual-parent","variantId":"base","prompt":"手写完整来源","promptOrigin":"MANUAL","adoptedMediaId":"manual-parent-image"},
            {"id":"inline-cover","variantId":"state","sourceAssetId":"manual-parent","preparation":{"mode":"INLINE"}},
            {"id":"inline-window","variantId":"window","preparation":{"mode":"INLINE"}},
            {"id":"inline-manual","variantId":"manual","prompt":"保留手写提示词","promptOrigin":"MANUAL","adoptedMediaId":"manual-import-image","preparation":{"mode":"INLINE"}},
        ]});
        let before = input.clone();
        let output = run_asset_design(&input, &Never, "offline-test-model", "medium")
            .await
            .unwrap();
        for field in [
            "assets",
            "designs",
            "imagePlan",
            "blocked",
            "visualFactBatches",
        ] {
            assert!(output[field].as_array().unwrap().is_empty());
        }
        assert_eq!(
            output["skipped"][0]["adoptedMediaId"],
            "manual-parent-image"
        );
        assert_eq!(output["skipped"][1]["status"], "textOnly");
        assert_eq!(output["skipped"][2]["status"], "existingImageRetained");
        assert_eq!(
            output["skipped"][2]["adoptedMediaId"],
            "manual-import-image"
        );
        assert_eq!(input, before);
    }
    #[tokio::test]
    async fn derived_child_through_inline_reuses_a_valid_parent_without_changing_direct_reference_id(
    ) {
        let source = prepare_source("第一集\n场景：门口\n△小王出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let mut inline = design_asset("inline-cover", &row, Some("a-parent"));
        inline["preparation"] = json!({"mode":"INLINE"});
        let assets = vec![
            design_asset("child", &row, Some("inline-cover")),
            inline,
            design_asset("a-parent", &row, None),
        ];
        let executor = ConcurrentDesigner::default();
        let output = run_asset_design(
            &json!({"source":source,"assets":assets,"selection":{"assetIds":["child"]}}),
            &executor,
            "offline-test-model",
            "medium",
        )
        .await
        .unwrap();
        assert!(output["blocked"].as_array().unwrap().is_empty());
        assert_eq!(executor.requests.lock().unwrap().len(), 2);
        let requests = executor.requests.lock().unwrap();
        let child = requests
            .iter()
            .find(|request| request["asset"]["id"] == "child")
            .unwrap();
        assert_eq!(child["parentDesign"]["assetId"], "a-parent");
        assert_eq!(child["inlineReference"]["directAssetId"], "inline-cover");
        let child = output["designs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|design| design["assetId"] == "child")
            .unwrap();
        assert_eq!(child["referenceAssetIds"], json!(["inline-cover"]));
        assert_eq!(output["skipped"].as_array().unwrap().len(), 1);
    }
    struct Never;
    #[async_trait]
    impl TextExecutor for Never {
        async fn execute(&self, _request: TextRequest) -> Result<Value, String> {
            panic!("循环/缺失来源不得调用模型")
        }
    }
    #[tokio::test]
    async fn cyclic_or_missing_dependency_blocks_locally_without_model_call() {
        let s = prepare_source("第一集\n场景：门口\n△小王出现。\n").unwrap();
        let row = &s.rows[2].id;
        let result=run_asset_design(&json!({"source":s,"assets":[{"id":"a:1","variantId":"a1","sourceVariantId":"a2","sourceRefs":[row]},{"id":"a:2","variantId":"a2","sourceVariantId":"a1","sourceRefs":[row]},{"id":"b:1","variantId":"b1","sourceVariantId":"missing","sourceRefs":[row]}]}),&Never,"gpt-6.1-sol","medium").await.unwrap();
        assert_eq!(result["blocked"].as_array().unwrap().len(), 3);
        assert!(result["designs"].as_array().unwrap().is_empty());
    }
    #[test]
    fn repeated_base_variant_resolves_by_subject_or_explicit_identity() {
        let assets = vec![
            json!({"id":"王:base","baseAssetId":"王","variantId":"base"}),
            json!({"id":"李:base","baseAssetId":"李","variantId":"base"}),
            json!({"id":"王:伤态","baseAssetId":"王","variantId":"hurt","sourceVariantId":"base"}),
        ];
        assert_eq!(
            resolve_asset_dependency(&assets, &assets[2]).unwrap(),
            Some("王:base".into())
        );
        assert!(resolve_asset_dependency(
            &assets,
            &json!({"id":"new","variantId":"legacy","sourceVariantId":"base"})
        )
        .is_err());
        assert_eq!(
            resolve_asset_dependency(
                &assets,
                &json!({"id":"new","variantId":"legacy","sourceAssetId":"李:base"})
            )
            .unwrap(),
            Some("李:base".into())
        );
    }

    #[test]
    fn explicitly_parentless_reference_is_not_rebased_to_a_later_registered_form() {
        let assets = vec![
            json!({"id":"later","baseAssetId":"subject","variantId":"subject:base","isBase":true}),
            json!({"id":"initial","baseAssetId":"subject","variantId":"subject_V_initial","sourceVariantId":null,"preparation":{"mode":"REFERENCE"}}),
            json!({"id":"next","baseAssetId":"subject","variantId":"next","sourceVariantId":"subject_V_initial","preparation":{"mode":"DERIVE"}}),
        ];
        assert_eq!(resolve_asset_dependency(&assets, &assets[1]).unwrap(), None);
        assert_eq!(resolve_asset_dependency(&assets, &assets[2]).unwrap(), Some("initial".into()));
        let legacy = json!({"id":"legacy","baseAssetId":"subject","variantId":"subject_V_legacy","preparation":{"mode":"REFERENCE"}});
        assert_eq!(resolve_asset_dependency(&assets, &legacy).unwrap(), Some("later".into()));
    }

    #[test]
    fn reference_children_inherit_the_unique_identity_base_without_name_or_row_id_guessing() {
        for kind in ["CH", "LOC"] {
            let assets = vec![
                json!({"id":"unrelated-row","baseAssetId":"other","variantId":"base","name":"同名","kind":kind}),
                json!({"id":"opaque-source-row","baseAssetId":"subject","variantId":"subject:base","name":"不同显示名","kind":kind,"preparation":{"mode":"REFERENCE"}}),
                json!({"id":"opaque-child-row","baseAssetId":"subject","variantId":"clothes-or-crater","name":"同名","kind":kind,"preparation":{"mode":"REFERENCE"}}),
            ];
            let before = assets.clone();
            assert_eq!(
                resolve_asset_dependency(&assets, &assets[2]).unwrap(),
                Some("opaque-source-row".into())
            );
            assert_eq!(resolve_asset_dependency(&assets, &assets[1]).unwrap(), None);
            assert_eq!(
                assets, before,
                "resolving a source never rewrites the catalogue or preparation"
            );
        }
        let assets = vec![
            json!({"id":"legacy-base-row","identityId":"legacy-owner","variantId":"opaque-base-version","isBase":true}),
            json!({"id":"legacy-child-row","identityId":"legacy-owner","variantId":"blue","preparation":{"mode":"REFERENCE"}}),
        ];
        assert_eq!(
            resolve_asset_dependency(&assets, &assets[1]).unwrap(),
            Some("legacy-base-row".into())
        );
    }

    #[test]
    fn reference_fallback_preserves_legacy_and_non_reference_assets_and_rejects_bad_bases() {
        let base = json!({"id":"base-row","baseAssetId":"subject","variantId":"base"});
        let child = json!({"id":"child-row","baseAssetId":"subject","variantId":"blue","preparation":{"mode":"REFERENCE"}});
        assert_eq!(
            resolve_asset_dependency(&[child.clone()], &child).unwrap(),
            None
        );
        let mut registered_child = child.clone();
        registered_child["variantId"] = json!("subject_V_registered");
        assert!(
            resolve_asset_dependency(&[registered_child.clone()], &registered_child)
                .unwrap_err()
                .contains("基础来源缺失")
        );
        let no_identity =
            json!({"id":"standalone","variantId":"blue","preparation":{"mode":"REFERENCE"}});
        assert_eq!(
            resolve_asset_dependency(&[base.clone()], &no_identity).unwrap(),
            None
        );
        for mode in ["INLINE", "AUDIO", "DERIVE", "UNRESOLVED"] {
            let mut non_reference = child.clone();
            non_reference["preparation"]["mode"] = json!(mode);
            assert_eq!(
                resolve_asset_dependency(&[base.clone(), non_reference.clone()], &non_reference)
                    .unwrap(),
                None
            );
        }
        let mut deleted = base.clone();
        deleted["deletedAt"] = json!("2026-01-01T00:00:00Z");
        assert!(resolve_asset_dependency(&[deleted, child.clone()], &child)
            .unwrap_err()
            .contains("已删除"));
        let second = json!({"id":"another-base-row","baseAssetId":"subject","variantId":"opaque-version","isBase":true});
        assert!(
            resolve_asset_dependency(&[base, second, child.clone()], &child)
                .unwrap_err()
                .contains("多个基础形象")
        );
    }

    #[test]
    fn explicit_sources_take_priority_over_the_reference_baseline_and_never_silently_fallback() {
        let assets = vec![
            json!({"id":"base-row","baseAssetId":"subject","variantId":"base"}),
            json!({"id":"white-row","baseAssetId":"subject","variantId":"white","sourceAssetId":"base-row"}),
            json!({"id":"injury-row","baseAssetId":"subject","variantId":"injury","sourceAssetId":"white-row","sourceVariantId":"base","preparation":{"mode":"REFERENCE"}}),
        ];
        assert_eq!(
            resolve_asset_dependency(&assets, &assets[2]).unwrap(),
            Some("white-row".into())
        );
        let mut variant_only = assets[2].clone();
        variant_only
            .as_object_mut()
            .unwrap()
            .remove("sourceAssetId");
        variant_only["sourceVariantId"] = json!("white");
        assert_eq!(
            resolve_asset_dependency(&assets, &variant_only).unwrap(),
            Some("white-row".into())
        );
        let mut missing = assets[2].clone();
        missing["sourceAssetId"] = json!("removed-source");
        assert!(resolve_asset_dependency(&assets, &missing).is_err());
        let mut deleted_assets = assets.clone();
        deleted_assets[1]["deletedAt"] = json!("2026-01-01T00:00:00Z");
        assert!(
            resolve_asset_dependency(&deleted_assets, &deleted_assets[2])
                .unwrap_err()
                .contains("已删除")
        );
    }

    #[test]
    fn implicit_and_explicit_source_cycles_are_rejected_before_dispatch() {
        let assets = vec![
            json!({"id":"base-row","baseAssetId":"subject","variantId":"base","sourceAssetId":"child-row"}),
            json!({"id":"child-row","baseAssetId":"subject","variantId":"blue","preparation":{"mode":"REFERENCE"}}),
        ];
        for asset in &assets {
            assert!(resolve_asset_dependency(&assets, asset)
                .unwrap_err()
                .contains("循环"));
        }
        let mut self_reference = assets[0].clone();
        self_reference["sourceAssetId"] = json!("base-row");
        assert!(
            resolve_asset_dependency(&[self_reference.clone()], &self_reference)
                .unwrap_err()
                .contains("循环")
        );
    }

    struct Designer;
    #[derive(Default)]
    struct ComparisonDesigner {
        requests: std::sync::Mutex<Vec<TextRequest>>,
        accepted: std::sync::Mutex<HashSet<String>>,
        cache: std::sync::Mutex<BTreeMap<String,Value>>,
        fail_b: std::sync::atomic::AtomicBool,
        character_active: std::sync::atomic::AtomicUsize,
        character_peak: std::sync::atomic::AtomicUsize,
        active: std::sync::atomic::AtomicUsize,
        peak: std::sync::atomic::AtomicUsize,
        generated: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl TextExecutor for ComparisonDesigner {
        async fn execute(&self, request: TextRequest) -> Result<Value,String> {
            use std::sync::atomic::Ordering::SeqCst;
            if request.stage == "assetVisualFacts" { return Ok(empty_visual(&request)); }
            let id = text(&request.input["asset"],"id").to_owned();
            let hash = digest(&serde_json::to_string(&request).unwrap());
            self.requests.lock().unwrap().push(request.clone());
            if let Some(value)=self.cache.lock().unwrap().get(&hash).cloned() { return Ok(value); }
            for comparison in request.input["comparisonDesigns"].as_array().unwrap() {
                assert!(self.accepted.lock().unwrap().contains(text(comparison,"assetId")),"comparison is not yet accepted");
            }
            let character = super::super::asset_design_contract::is_character(&request.input["asset"]);
            if character { let active=self.character_active.fetch_add(1,SeqCst)+1; self.character_peak.fetch_max(active,SeqCst); }
            let active=self.active.fetch_add(1,SeqCst)+1; self.peak.fetch_max(active,SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(3)).await;
            self.active.fetch_sub(1,SeqCst);
            if character { self.character_active.fetch_sub(1,SeqCst); }
            if id=="b" && self.fail_b.load(SeqCst) { return Err("synthetic first attempt failure".into()); }
            self.generated.fetch_add(1,SeqCst);
            let refs=request.input["parentAsset"]["id"].as_str().map(|id|vec![id]).unwrap_or_default();
            let value=json!({"schemaVersion":"asset-design/v1","assetId":id,"variantId":request.input["asset"]["variantId"],"prompt":format!("{id}独立脸部比例与衣装轮廓"),"freeDesignScope":"未限定外貌","sourceRefs":request.input["asset"]["sourceRefs"],"referenceVariantIds":refs});
            self.cache.lock().unwrap().insert(hash,value.clone());
            Ok(value)
        }
        async fn accept(&self,request:&TextRequest,_value:&Value)->Result<(),String> {
            if request.stage=="assetDesign" { self.accepted.lock().unwrap().insert(text(&request.input["asset"],"id").into()); }
            Ok(())
        }
    }
    fn comparison_fixture() -> Value {
        let source=prepare_source("第一集\n场景：门口\n△甲乙丙站在门口。\n").unwrap();
        let row=source.rows[2].id.clone();
        let mut assets:Vec<_>=["c","b","a"].iter().map(|id|design_asset(id,&row,None)).collect();
        for n in 0..4 { let mut prop=design_asset(&format!("prop{n}"),&row,None);prop["kind"]=json!("PROP");assets.push(prop); }
        let mut input=json!({"source":source,"assets":assets,"comparisonDesignsByAssetId":{"a":[],"b":[],"c":[]}});
        input[super::super::asset_design_contract::COMPARISON_PROFILE_FIELD]=json!(super::super::asset_design_contract::COMPARISON_PROFILE);
        input
    }
    #[tokio::test]
    async fn incremental_comparison_new_characters_receive_accepted_predecessors_and_props_stay_parallel() {
        use std::sync::atomic::Ordering::SeqCst;
        let input=comparison_fixture();let exec=ComparisonDesigner::default();
        let result=run_asset_design(&input,&exec,"offline","medium").await.unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty());
        assert_eq!(exec.generated.load(SeqCst),7);
        assert_eq!(exec.character_peak.load(SeqCst),1);
        assert_eq!(exec.peak.load(SeqCst),4);
        let requests=exec.requests.lock().unwrap();
        for (id,expected) in [("a",vec![]),("b",vec!["a"]),("c",vec!["a","b"])] {
            let request=requests.iter().find(|r|r.input["asset"]["id"]==id).unwrap();
            assert_eq!(request.input["comparisonDesigns"].as_array().unwrap().iter().map(|r|text(r,"assetId")).collect::<Vec<_>>(),expected);
            assert!(request.input["referenceCatalog"].as_array().unwrap().is_empty());
        }
    }
    #[tokio::test]
    async fn incremental_comparison_resume_preserves_dispatched_domain_hash_after_failed_predecessor_recovers() {
        use std::sync::atomic::Ordering::SeqCst;
        let mut input=comparison_fixture();let exec=ComparisonDesigner::default();exec.fail_b.store(true,SeqCst);
        let first=run_asset_design(&input,&exec,"offline","medium").await.unwrap();
        assert_eq!(first["blocked"].as_array().unwrap().len(),1);
        let original=exec.requests.lock().unwrap().clone();
        let mut frozen=json!({});
        for r in &original { frozen[text(&r.input["asset"],"id")]=serde_json::to_value(r).unwrap(); }
        input[super::super::asset_design_contract::FROZEN_COMPARISON_REQUESTS]=frozen;
        exec.fail_b.store(false,SeqCst);exec.requests.lock().unwrap().clear();
        let second=run_asset_design(&input,&exec,"offline","medium").await.unwrap();
        assert!(second["blocked"].as_array().unwrap().is_empty());
        assert_eq!(exec.generated.load(SeqCst),7,"accepted predecessors and later requests are cache replays");
        for replay in exec.requests.lock().unwrap().iter() {
            let old=original.iter().find(|r|r.input["asset"]["id"]==replay.input["asset"]["id"]).unwrap();
            assert_eq!(serde_json::to_value(old).unwrap(),serde_json::to_value(replay).unwrap());
        }
        let dispatched=exec.requests.lock().unwrap().len();
        input["settings"]["projectVisualStyle"]=json!("changed");
        assert!(run_asset_design(&input,&exec,"offline","medium").await.unwrap_err().contains("冻结领域请求"));
        assert_eq!(exec.requests.lock().unwrap().len(),dispatched,"domain drift cannot dispatch");
    }
    #[tokio::test]
    async fn incremental_comparison_legacy_frozen_empty_map_and_parallelism_are_unchanged() {
        use std::sync::atomic::Ordering::SeqCst;
        let mut input=comparison_fixture();input.as_object_mut().unwrap().remove(super::super::asset_design_contract::COMPARISON_PROFILE_FIELD);
        let exec=ComparisonDesigner::default();run_asset_design(&input,&exec,"offline","medium").await.unwrap();
        assert!(exec.character_peak.load(SeqCst)>1);
        for request in exec.requests.lock().unwrap().iter() {
            assert_eq!(request.input["comparisonDesigns"],json!([]));
            assert!(request.input.get(super::super::asset_design_contract::COMPARISON_PROFILE_FIELD).is_none());
        }
    }
    #[tokio::test]
    async fn incremental_comparison_legacy_nonempty_map_replays_original_request_after_partial_failure() {
        use std::sync::atomic::Ordering::SeqCst;
        let mut input=comparison_fixture();
        input.as_object_mut().unwrap().remove(super::super::asset_design_contract::COMPARISON_PROFILE_FIELD);
        let original_comparison=json!([{"assetId":"previously-accepted","prompt":"原已采用角色的宽脸短发对照"}]);
        input["comparisonDesignsByAssetId"]["a"]=original_comparison.clone();
        let exec=ComparisonDesigner::default();
        exec.accepted.lock().unwrap().insert("previously-accepted".into());
        exec.fail_b.store(true,SeqCst);
        let first=run_asset_design(&input,&exec,"offline","medium").await.unwrap();
        assert_eq!(first["blocked"].as_array().unwrap().len(),1);
        let originals=exec.requests.lock().unwrap().clone();
        assert_eq!(originals.iter().find(|r|r.input["asset"]["id"]=="a").unwrap().input["comparisonDesigns"],original_comparison);
        exec.fail_b.store(false,SeqCst);
        exec.requests.lock().unwrap().clear();
        let restored=run_asset_design(&input,&exec,"offline","medium").await.unwrap();
        assert!(restored["blocked"].as_array().unwrap().is_empty());
        assert_eq!(exec.generated.load(SeqCst),7,"previously accepted legacy requests must replay rather than regenerate");
        for request in exec.requests.lock().unwrap().iter() {
            let original=originals.iter().find(|r|r.input["asset"]["id"]==request.input["asset"]["id"]).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(),serde_json::to_value(original).unwrap(),
                "the complete legacy request, including its frozen comparison map, must remain unchanged");
            assert!(request.input.get(super::super::asset_design_contract::COMPARISON_PROFILE_FIELD).is_none());
        }
    }
    #[tokio::test]
    async fn incremental_comparison_character_lane_respects_parent_topology_before_lexical_order() {
        let mut input=comparison_fixture();
        input["assets"][2]["sourceAssetId"]=json!("c");
        input["assets"][2]["preparation"]["mode"]=json!("DERIVE");
        let exec=ComparisonDesigner::default();
        let result=run_asset_design(&input,&exec,"offline","medium").await.unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty());
        let requests=exec.requests.lock().unwrap();
        let chars:Vec<_>=requests.iter().filter(|r|super::super::asset_design_contract::is_character(&r.input["asset"])).map(|r|text(&r.input["asset"],"id")).collect();
        assert_eq!(chars,vec!["b","c","a"]);
    }
    #[async_trait]
    impl TextExecutor for Designer {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            if request.stage == "assetVisualFacts" {
                return Ok(empty_visual(&request));
            }
            let asset = &request.input["asset"];
            Ok(
                json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":format!("{}全身设定图，白衣，纯色背景。",text(asset,"name")),"freeDesignScope":"背景灰度","sourceRefs":asset["sourceRefs"],"referenceVariantIds":[]}),
            )
        }
    }
    #[tokio::test]
    async fn multiple_legacy_or_base_variants_are_designed_as_distinct_assets() {
        let s = prepare_source("第一集\n场景：门口\n△小王和小李穿白衣出现。\n").unwrap();
        let row = &s.rows[2].id;
        let result=run_asset_design(&json!({"source":s,"assets":[{"id":"a","variantId":"base","name":"小王","sourceRefs":[row]},{"id":"b","variantId":"base","name":"小李","sourceRefs":[row]}]}),&Designer,"gpt-6.1-sol","medium").await.unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty());
        assert_eq!(result["designs"].as_array().unwrap().len(), 2);
        assert_ne!(result["assets"][0]["id"], result["assets"][1]["id"]);
        assert!(result["imagePlan"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["status"] == "ready" && p["imageProductionPolicy"] == "automatic"));
    }
    struct CountingDesigner {
        requests: std::sync::Mutex<Vec<Value>>,
    }
    #[async_trait]
    impl TextExecutor for CountingDesigner {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            if request.stage == "assetVisualFacts" {
                return Ok(empty_visual(&request));
            }
            self.requests.lock().unwrap().push(request.input.clone());
            let asset = &request.input["asset"];
            let references = request.input["parentAsset"]["id"]
                .as_str()
                .map(|id| vec![id])
                .unwrap_or_default();
            Ok(
                json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":"新设计完整提示词","freeDesignScope":"背景灰度","sourceRefs":asset["sourceRefs"],"referenceVariantIds":references}),
            )
        }
    }

    #[tokio::test]
    async fn reference_clothes_and_scene_designs_use_old_derived_contract_and_parent_bound_hashes()
    {
        let source =
            prepare_source("第一集\n场景：庭院\n△甲换上蓝衣，庭院地面留下深坑。\n").unwrap();
        let row = source.rows[2].id.clone();
        for kind in ["CH", "LOC"] {
            let parent = json!({"id":"opaque-base-row","baseAssetId":"subject","variantId":"subject:base","kind":kind,"prompt":"已确认基础面部或场景布局","promptOrigin":"MANUAL","sourceRefs":[row]});
            let child = json!({"id":"opaque-child-row","baseAssetId":"subject","variantId":"new-appearance","kind":kind,"prompt":"","sourceRefs":[row],"preparation":{"mode":"REFERENCE"},"adoptedMediaId":"existing-current-image","candidates":["old-history-image"]});
            let mut input = json!({"source":source,"assets":[parent,child],"selection":{"assetIds":["opaque-child-row"]}});
            let before = input.clone();
            let executor = CountingDesigner {
                requests: std::sync::Mutex::new(vec![]),
            };
            let first = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert!(first["blocked"].as_array().unwrap().is_empty());
            assert_eq!(first["designs"].as_array().unwrap().len(), 1);
            assert_eq!(first["designs"][0]["designMode"], "DERIVED");
            assert_eq!(
                first["designs"][0]["referenceAssetIds"],
                json!(["opaque-base-row"])
            );
            assert_eq!(
                first["imagePlan"][0]["dependencies"],
                json!(["opaque-base-row"])
            );
            assert_eq!(first["assets"][0]["preparation"]["mode"], "REFERENCE");
            assert_eq!(
                first["assets"][0]["adoptedMediaId"],
                "existing-current-image"
            );
            assert_eq!(
                first["assets"][0]["candidates"],
                json!(["old-history-image"])
            );
            assert_eq!(
                input, before,
                "source/catalogue/current image/history are immutable inputs"
            );
            let requests = executor.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0]["designMode"], "DERIVED");
            assert_eq!(requests[0]["parentDesign"]["prompt"], parent["prompt"]);
            assert_eq!(requests[0]["parentAsset"]["id"], "opaque-base-row");
            drop(requests);
            input["assets"][0]["prompt"] = json!("更新后的基础面部或场景布局");
            let second = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert_ne!(
                first["designs"][0]["inputFingerprint"], second["designs"][0]["inputFingerprint"],
                "a parent design change cannot reuse the previous request hash"
            );

            let mut old_independent = before.clone();
            old_independent["assets"][1]["prompt"] = json!("迁移时误用的独立设计");
            old_independent["assets"][1]["promptOrigin"] = json!("AI");
            old_independent["assets"][1]["design"] = json!({"schemaVersion":"asset-design/v1","assetId":"opaque-child-row","variantId":"new-appearance","sourceSha256":source.sha256,"prompt":"迁移时误用的独立设计","sourceRefs":[row],"referenceAssetIds":[],"referenceVariantIds":[]});
            assert!(automatic_design_required(&old_independent, &old_independent["assets"][1]), "a BASE result without the now-resolved parent cannot be reused as a derived design");
            assert_eq!(
                old_independent["assets"][1]["adoptedMediaId"],
                "existing-current-image"
            );
        }
    }

    #[tokio::test]
    async fn designing_only_a_missing_child_reuses_the_valid_parent_without_returning_a_parent_update(
    ) {
        let source =
            prepare_source("第一集\n场景：门口\n△小王穿白衣出现，随后衣服破损。\n").unwrap();
        let row = source.rows[2].id.clone();
        let design = json!({"schemaVersion":"asset-design/v1","assetId":"parent","variantId":"base","sourceSha256":source.sha256,"prompt":"已确认白衣完整设定","sourceRefs":[row],"referenceVariantIds":[],"referenceAssetIds":[]});
        let parent = json!({"id":"parent","baseAssetId":"person","variantId":"base","version":7,"prompt":"已确认白衣完整设定","sourceRefs":[row],"design":design});
        let child = json!({"id":"child","baseAssetId":"person","variantId":"hurt","prompt":"","sourceAssetId":"parent","sourceRefs":[row]});
        let mut input =
            json!({"source":source,"assets":[parent,child],"selection":{"assetIds":["child"]}});
        input["assets"][0]["design"]["designReceipt"] = super::super::asset_design_contract::design_receipt(&input,&input["assets"][0]).unwrap();
        let parent = input["assets"][0].clone();
        let executor = CountingDesigner {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let result = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty());
        assert_eq!(executor.requests.lock().unwrap().len(), 1);
        assert_eq!(
            executor.requests.lock().unwrap()[0]["parentDesign"]["prompt"],
            design["prompt"]
        );
        assert_eq!(
            executor.requests.lock().unwrap()[0]["parentDesign"]["origin"],
            "AI"
        );
        assert_eq!(
            executor.requests.lock().unwrap()[0]["parentDesign"]["assetId"],
            "parent"
        );
        assert_eq!(input["assets"][0], parent);
        for key in ["assets", "designs", "imagePlan"] {
            assert_eq!(result[key].as_array().unwrap().len(), 1);
        }
        assert_eq!(result["assets"][0]["id"], "child");
        assert_eq!(result["designs"][0]["referenceAssetIds"], json!(["parent"]));
        input["selection"]["assetIds"] = json!(["parent"]);
        let explicit = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(executor.requests.lock().unwrap().len(), 2);
        assert_eq!(explicit["assets"][0]["id"], "parent");
    }

    fn visual_fact(
        id: &str,
        subject: &str,
        variant: Option<&str>,
        attribute: &str,
        value: &str,
        scope: &str,
        row: &str,
    ) -> Value {
        json!({"id":id,"assetId":subject,"variantId":variant,"attribute":attribute,"value":value,"scope":scope,"claim":"SOURCE","sourceRefs":[row],"quotes":["不能发回模型的重复原文"]})
    }

    #[test]
    fn legacy_exact_reference_cards_keep_their_existing_wire_shape() {
        let legacy = json!({"id":"row-a","baseAssetId":"person","variantId":"base","name":"小王","kind":"CH","sourceRefs":["exact-row"]});
        assert_eq!(
            asset_design_card(&json!({}), &legacy).unwrap(),
            json!({
                "id":"row-a","baseAssetId":"person","variantId":"base","name":"小王","kind":"CH",
                "description":null,"sourceRefs":["exact-row"],"scope":null,"applicability":null,"preparation":null
            })
        );
        assert!(!has_program_source_scope(&legacy));
    }

    struct ScopeDesigner {
        requests: std::sync::Mutex<Vec<Value>>,
    }
    #[async_trait]
    impl TextExecutor for ScopeDesigner {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            if request.stage != "assetDesign" {
                return Err("已有有效事实，本测试不得新增视觉提取或其它模型调用".into());
            }
            self.requests.lock().unwrap().push(request.input.clone());
            let asset = &request.input["asset"];
            let references = request.input["parentAsset"]["id"]
                .as_str()
                .map(|id| vec![id])
                .unwrap_or_default();
            Ok(
                json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":"原文约束与可编辑设计","freeDesignScope":"未限定的背景灰度","sourceRefs":asset["sourceRefs"],"referenceVariantIds":references}),
            )
        }
    }

    #[tokio::test]
    async fn program_scope_is_labeled_in_base_and_derived_inputs_without_becoming_facts() {
        let source = prepare_source(
            "第一集\n场景：门口\n△小王黑发白衣，小李穿红衣。\n小李：小王的右手受伤了。\n",
        )
        .unwrap();
        let appearance = source
            .rows
            .iter()
            .find(|row| row.text.contains("小王黑发"))
            .unwrap();
        let claim = source
            .rows
            .iter()
            .find(|row| row.text.contains("右手受伤"))
            .unwrap();
        let scope_refs: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.scene_id == appearance.scene_id)
            .map(|row| row.id.clone())
            .collect();
        let scope = json!({"sourceSha256":source.sha256,"kind":"PROGRAM_SCOPE"});
        let mut hurt = visual_fact(
            "hurt",
            "person",
            Some("hurt"),
            "右手",
            "伤口",
            "STATE",
            &claim.id,
        );
        hurt["claim"] = json!("CHARACTER_CLAIM");
        let facts = json!([
            visual_fact(
                "hair",
                "person",
                Some("base"),
                "头发",
                "黑发",
                "STABLE",
                &appearance.id
            ),
            hurt,
            visual_fact(
                "other",
                "other-person",
                Some("base"),
                "衣服",
                "红衣",
                "STATE",
                &appearance.id
            )
        ]);
        let assets = json!([
            {"id":"base-row","baseAssetId":"person","variantId":"base","name":"小王基础形象","prompt":"已保存基础设计","promptOrigin":"MANUAL","sourceRefs":scope_refs,"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":scope},
            {"id":"hurt-row","baseAssetId":"person","variantId":"hurt","name":"小王伤态","sourceAssetId":"base-row","sourceRefs":scope_refs,"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":scope}
        ]);
        let executor = ScopeDesigner {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        for (selected, mode, expected_ids) in [
            ("base-row", "BASE", vec!["hair"]),
            ("hurt-row", "DERIVED", vec!["hair", "hurt"]),
        ] {
            let input = json!({"source":source,"assets":assets,"selection":{"assetIds":[selected]},"foundation":{"sourceSha256":source.sha256,"facts":facts}});
            let before = executor.requests.lock().unwrap().len();
            let result = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert!(result["blocked"].as_array().unwrap().is_empty(), "{result}");
            assert_eq!(result["facts"], facts);
            let requests = executor.requests.lock().unwrap();
            assert_eq!(requests.len(), before + 1);
            let request = requests.last().unwrap();
            assert_eq!(request["designMode"], mode);
            assert_eq!(request["asset"]["sourceRefsPurpose"], "SOURCE_SCOPE");
            assert_eq!(request["asset"]["sourceScope"], scope);
            assert!(request["sourceEvidencePolicy"]
                .as_str()
                .unwrap()
                .contains("不是这些行已证明属于该资产的外观事实"));
            let ids: Vec<_> = request["facts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|fact| text(fact, "id"))
                .collect();
            assert_eq!(ids, expected_ids);
            assert!(request["facts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|fact| text(fact, "assetId") == "person"));
            assert!(request["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["evidenceUses"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("SOURCE_SCOPE"))));
            let referenced = request["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == appearance.id)
                .unwrap();
            assert!(referenced["evidenceUses"]
                .as_array()
                .unwrap()
                .contains(&json!("VISUAL_FACT_REFERENCE")));
            if mode == "DERIVED" {
                assert_eq!(request["parentAsset"]["sourceScope"], scope);
                assert_eq!(
                    request["referenceCatalog"][0]["sourceRefsPurpose"],
                    "SOURCE_SCOPE"
                );
                assert_eq!(request["deltaFacts"][0]["variantId"], "hurt");
                assert_eq!(request["deltaFacts"][0]["claim"], "CHARACTER_CLAIM");
            } else {
                assert!(request["facts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|fact| fact["variantId"] == "base"));
            }
        }
        assert_eq!(executor.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn derived_input_inherits_only_its_ancestors_and_replaces_explicit_attributes() {
        let source = prepare_source("第一集\n场景：门口\n△小王黑发白衣，换蓝衣后右手受伤。\n第二集\n场景：河边\n△小王穿红衣出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let future_row = source.rows[5].id.clone();
        let assets = json!([
            {"id":"base","baseAssetId":"person","variantId":"base","prompt":"手写黑发白衣","promptOrigin":"MANUAL","sourceRefs":[row]},
            {"id":"blue","baseAssetId":"person","variantId":"blue","prompt":"手写蓝衣","sourceAssetId":"base","sourceRefs":[row]},
            {"id":"hurt","baseAssetId":"person","variantId":"hurt","prompt":"不能当新事实的旧提示词","sourceAssetId":"blue","sourceRefs":[row],"design":{"oldHistory":"不要发送"},"candidates":[{"private":"不要发送"}]}
        ]);
        let input = json!({"source":source,"assets":assets,"selection":{"assetIds":["hurt"]},"foundation":{"directionConfirmed":true,"productionDirection":"3D漫剧的克制写实表达","facts":[
            visual_fact("hair","person",None,"头发","黑发","STABLE",&row),
            visual_fact("white","person",Some("base"),"衣服","白衣","STABLE",&row),
            visual_fact("blue","person",Some("blue"),"衣服","蓝衣","STATE",&row),
            visual_fact("hurt","person",Some("hurt"),"右手","伤口","STATE",&row),
            visual_fact("future","person",Some("future"),"衣服","红衣","STATE",&future_row),
            visual_fact("other","other-person",Some("hurt"),"头发","红发","STABLE",&row)
        ]}});
        let executor = CountingDesigner {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let result = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty());
        let requests = executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request["designMode"], "DERIVED");
        assert_eq!(request["parentDesign"]["assetId"], "blue");
        assert_eq!(request["parentDesign"]["prompt"], "手写蓝衣");
        assert_eq!(request["parentDesign"]["origin"], "UNVERIFIED");
        assert_eq!(request["productionDirection"], "3D漫剧的克制写实表达");
        let ids: Vec<_> = request["facts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|fact| text(fact, "id"))
            .collect();
        assert_eq!(ids, vec!["hair", "blue", "hurt"]);
        assert_eq!(request["deltaFacts"][0]["id"], "hurt");
        assert_eq!(request["deltaFacts"].as_array().unwrap().len(), 1);
        assert!(request["facts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|fact| fact.get("quotes").is_none()));
        assert_eq!(request["evidence"].as_array().unwrap().len(), 1);
        assert!(request["asset"].get("prompt").is_none());
        assert!(request["asset"].get("design").is_none());
        assert!(request["asset"].get("candidates").is_none());
        assert_eq!(request["referenceCatalog"].as_array().unwrap().len(), 1);
        assert_eq!(result["assets"][0]["promptOrigin"], "AI");
    }

    #[tokio::test]
    async fn manual_parent_without_ai_design_or_source_facts_stays_a_direct_reference() {
        let source = prepare_source("第一集\n场景：门口\n△小王衣服破损。\n").unwrap();
        let row = source.rows[2].id.clone();
        let parent = json!({"id":"parent","variantId":"base","prompt":"用户确认手写服装设计","promptOrigin":"MANUAL"});
        let input = json!({"source":source,"assets":[parent,{"id":"child","variantId":"hurt","sourceAssetId":"parent","sourceRefs":[row]}],"selection":{"assetIds":["child"]},"foundation":{"directionConfirmed":false,"productionDirection":"尚未确认方向"}});
        let executor = CountingDesigner {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let result = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty());
        assert_eq!(executor.requests.lock().unwrap().len(), 1);
        assert_eq!(
            executor.requests.lock().unwrap()[0]["parentDesign"]["origin"],
            "MANUAL"
        );
        assert!(executor.requests.lock().unwrap()[0]["productionDirection"].is_null());
        assert_eq!(result["assets"].as_array().unwrap().len(), 1);
        assert_eq!(result["assets"][0]["id"], "child");
        assert_eq!(input["assets"][0], parent);
    }

    #[tokio::test]
    async fn stale_implicit_ai_parent_is_preserved_and_only_its_dependents_block() {
        let source = prepare_source("第一集\n场景：门口\n△小王与小李出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let parent = json!({"id":"parent","variantId":"base","prompt":"旧自动设计不得隐式覆盖","sourceRefs":[row],"design":{"sourceSha256":"stale","prompt":"旧自动设计不得隐式覆盖"}});
        let input = json!({"source":source,"assets":[parent,{"id":"child","variantId":"hurt","sourceAssetId":"parent","sourceRefs":[row]},{"id":"unrelated","variantId":"base","sourceRefs":[row]}],"selection":{"assetIds":["child","unrelated"]}});
        let executor = CountingDesigner {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let result = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(result["blocked"].as_array().unwrap().len(), 2);
        assert_eq!(result["designs"].as_array().unwrap().len(), 1);
        assert_eq!(result["designs"][0]["assetId"], "unrelated");
        assert_eq!(executor.requests.lock().unwrap().len(), 1);
        assert_eq!(input["assets"][0], parent);
    }

    #[tokio::test]
    async fn unresolved_state_and_invalid_fact_rows_do_not_call_the_model() {
        let source = prepare_source("第一集\n场景：门口\n△小王出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        for fact in [
            visual_fact("unresolved", "person", None, "衣服", "伤态", "STATE", &row),
            visual_fact(
                "bad-source",
                "person",
                Some("base"),
                "衣服",
                "蓝衣",
                "STABLE",
                "unknown-row",
            ),
        ] {
            let input = json!({"source":source,"assets":[{"id":"a","baseAssetId":"person","variantId":"base","sourceRefs":[row]}],"foundation":{"facts":[fact]}});
            let result = run_asset_design(&input, &Never, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert_eq!(result["blocked"].as_array().unwrap().len(), 1);
            assert!(result["designs"].as_array().unwrap().is_empty());
        }
    }

    struct ValidatedDesigner {
        invalid: &'static str,
        accepted: std::sync::Mutex<Vec<Value>>,
        rejected: std::sync::Mutex<Vec<String>>,
    }
    #[async_trait]
    impl TextExecutor for ValidatedDesigner {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            if request.stage == "assetVisualFacts" {
                return Ok(empty_visual(&request));
            }
            assert!(request.rule_snapshot["primary"]["content"]
                .as_str()
                .unwrap()
                .contains("BASE 人物设计"));
            assert!(request.rule_snapshot["primary"]["content"]
                .as_str()
                .unwrap()
                .contains("DERIVED："));
            let asset = &request.input["asset"];
            Ok(
                json!({"schemaVersion":"asset-design/v1","assetId":if self.invalid=="identity"{"wrong"}else{text(asset,"id")},"variantId":asset["variantId"],"prompt":"完整资产外观","freeDesignScope":"非剧情背景补足","sourceRefs":if self.invalid=="source"{json!(["unknown-row"])}else{asset["sourceRefs"].clone()},"referenceVariantIds":if self.invalid=="reference"{json!([text(asset,"id")])}else{json!([])}}),
            )
        }
        async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
            if request.stage == "assetDesign" {
                self.accepted.lock().unwrap().push(value.clone());
            }
            Ok(())
        }
        async fn reject(
            &self,
            _request: &TextRequest,
            _value: &Value,
            reason: &str,
        ) -> Result<(), String> {
            self.rejected.lock().unwrap().push(reason.into());
            Ok(())
        }
    }
    #[tokio::test]
    async fn output_is_accepted_only_after_identity_source_and_dependency_validation() {
        let source = prepare_source("第一集\n场景：门口\n△小王出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let input =
            json!({"source":source,"assets":[{"id":"a","variantId":"base","sourceRefs":[row]}]});
        for invalid in ["identity", "source", "reference", ""] {
            let executor = ValidatedDesigner {
                invalid,
                accepted: std::sync::Mutex::new(Vec::new()),
                rejected: std::sync::Mutex::new(Vec::new()),
            };
            let result = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert_eq!(
                executor.accepted.lock().unwrap().len(),
                usize::from(invalid.is_empty())
            );
            assert_eq!(
                executor.rejected.lock().unwrap().len(),
                usize::from(!invalid.is_empty())
            );
            assert_eq!(
                result["blocked"].as_array().unwrap().len(),
                usize::from(!invalid.is_empty())
            );
            if invalid.is_empty() {
                let accepted = executor.accepted.lock().unwrap();
                assert_eq!(accepted[0]["sourceSha256"], source.sha256);
                assert_eq!(accepted[0]["designMode"], "BASE");
                assert_eq!(accepted[0]["referenceAssetIds"], json!([]));
                assert!(!text(&accepted[0], "inputFingerprint").is_empty());
            }
        }
        let missing_parent = json!({"source":source,"assets":[{"id":"parent","variantId":"base","prompt":"手写来源"},{"id":"child","variantId":"hurt","sourceAssetId":"parent","sourceRefs":[row]}],"selection":{"assetIds":["child"]}});
        let executor = ValidatedDesigner {
            invalid: "",
            accepted: std::sync::Mutex::new(Vec::new()),
            rejected: std::sync::Mutex::new(Vec::new()),
        };
        let result = run_asset_design(&missing_parent, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(executor.accepted.lock().unwrap().len(), 0);
        assert_eq!(executor.rejected.lock().unwrap().len(), 1);
        assert_eq!(result["blocked"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn automatic_design_repairs_stale_ai_and_only_its_ai_descendants_keeps_valid_and_manual()
    {
        let source = prepare_source("第一集\n场景：门口\n△甲白衣破损，乙黑发出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let valid_design = |id: &str, prompt: &str, refs: Value| json!({"schemaVersion":"asset-design/v1","assetId":id,"variantId":"base","sourceSha256":source.sha256,"prompt":prompt,"sourceRefs":[row],"referenceAssetIds":refs,"referenceVariantIds":refs});
        let parent = json!({"id":"parent","baseAssetId":"person","variantId":"base","version":3,"prompt":"旧AI父形象","promptOrigin":"AI","sourceRefs":[row],"design":{"schemaVersion":"asset-design/v1","sourceSha256":"old","assetId":"parent","variantId":"base","prompt":"旧AI父形象","sourceRefs":[row],"referenceAssetIds":[],"referenceVariantIds":[]}});
        let child = json!({"id":"child","baseAssetId":"person","variantId":"base","sourceAssetId":"parent","prompt":"旧AI子形象","promptOrigin":"AI","sourceRefs":[row],"design":valid_design("child","旧AI子形象",json!(["parent"]))});
        let current = json!({"id":"current","variantId":"base","prompt":"有效AI形象","promptOrigin":"AI","sourceRefs":[row],"design":valid_design("current","有效AI形象",json!([]))});
        let manual = json!({"id":"manual","variantId":"base","prompt":"手写设计无需强制重做","promptOrigin":"MANUAL","sourceRefs":[]});
        let mut input = json!({"source":source,"assets":[parent,child,current,manual],"settings":{"trialApproved":false},"imageProductionPolicy":"automatic","selection":{"assetIds":["parent","child","current","manual"]}});
        input["assets"][2]["design"]["designReceipt"] = super::super::asset_design_contract::design_receipt(&input, &input["assets"][2]).unwrap();
        assert!(automatic_design_required(&input, &input["assets"][0]));
        assert!(!automatic_design_required(&input, &input["assets"][2]));
        assert!(!automatic_design_required(&input, &input["assets"][3]));
        let before = input.clone();
        let exec = CountingDesigner {
            requests: std::sync::Mutex::new(vec![]),
        };
        let result = run_asset_design(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty(), "{result}");
        let requests = exec.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|r| text(&r["asset"], "id"))
                .collect::<Vec<_>>(),
            vec!["parent", "child"]
        );
        assert_eq!(requests[1]["parentDesign"]["prompt"], "新设计完整提示词");
        assert_eq!(result["assets"].as_array().unwrap().len(), 2);
        assert_eq!(
            result["trialApproved"], false,
            "automatic execution never claims manual style acceptance"
        );
        assert!(result["imagePlan"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["imageProductionPolicy"] == "automatic" && p["status"] != "awaitingTrial"));
        assert_eq!(input, before);
    }

    #[tokio::test]
    async fn only_explicit_trial_review_setting_keeps_the_manual_gate() {
        assert_eq!(image_production_policy(&json!({})), "automatic");
        assert_eq!(
            image_production_policy(&json!({"trialApproved":false})),
            "automatic"
        );
        assert_eq!(
            image_production_policy(&json!({"requireImageTrialReview":true})),
            "trialReview"
        );
        let source = prepare_source("第一集\n场景：门口\n△甲出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let input = json!({"source":source,"assets":[{"id":"a","variantId":"base","sourceRefs":[row]}],"settings":{"requireImageTrialReview":true,"trialApproved":false},"automaticDesign":true});
        let result = run_asset_design(&input, &Designer, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(result["imageProductionPolicy"], "trialReview");
        assert_eq!(result["imagePlan"][0]["status"], "awaitingTrial");
        assert_eq!(result["trialApproved"], false);
    }

    #[test]
    fn ai_with_missing_receipt_requires_design_without_relabeling_unknown_or_manual_prose() {
        let input = json!({"source":{"sha256":"sha","rows":[{"id":"row"}]},"assets":[]});
        assert!(automatic_design_required(
            &input,
            &json!({"prompt":"AI旧文字","promptOrigin":"AI","sourceRefs":["row"]})
        ));
        assert!(!automatic_design_required(
            &input,
            &json!({"prompt":"历史未认证文字","sourceRefs":[]})
        ));
        assert!(!automatic_design_required(
            &input,
            &json!({"prompt":"手写","promptOrigin":"MANUAL","design":{"sourceSha256":"old"}})
        ));
    }

    #[tokio::test]
    async fn automatic_design_does_not_rewrite_an_active_stale_image_source() {
        let source = prepare_source("第一集\n场景：门口\n△甲与乙出现。\n").unwrap();
        let row = &source.rows[2].id;
        let parent = json!({"id":"parent","variantId":"base","prompt":"正在执行的旧 AI 提示词","promptOrigin":"AI","sourceRefs":[row],"design":{"schemaVersion":"asset-design/v1","sourceSha256":"old"}});
        let input = json!({"source":source,"assets":[parent,{"id":"child","variantId":"base","sourceAssetId":"parent","sourceRefs":[row]},{"id":"independent","variantId":"base","sourceRefs":[row]}],"selection":{"assetIds":["child","independent"]},"imageProductionPolicy":"automatic","activeImageAssetIds":["parent"]});
        assert!(!automatic_design_required(&input, &input["assets"][0]));
        let executor = CountingDesigner {
            requests: std::sync::Mutex::new(vec![]),
        };
        let result = run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(
            executor
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|r| text(&r["asset"], "id"))
                .collect::<Vec<_>>(),
            vec!["independent"]
        );
        assert_eq!(result["designs"].as_array().unwrap().len(), 1);
        assert_eq!(result["designs"][0]["assetId"], "independent");
        let blocked: HashSet<_> = result["blocked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| text(b, "assetId"))
            .collect();
        assert_eq!(blocked, HashSet::from(["parent", "child"]));
        assert_eq!(input["assets"][0], parent);
    }

    #[tokio::test]
    async fn automatic_derived_design_uses_the_manual_inline_donor_without_designing_its_parent() {
        let source = prepare_source("第一集\n场景：门口\n△甲穿着破损蓝衣。\n").unwrap();
        let row = &source.rows[2].id;
        let input = json!({"source":source,"imageProductionPolicy":"automatic","selection":{"assetIds":["child"]},"assets":[
            {"id":"unused-parent","variantId":"base","sourceRefs":[row]},
            {"id":"inline","variantId":"coverage","sourceAssetId":"unused-parent","sourceRefs":[row],"preparation":{"mode":"INLINE"},"prompt":"手写蓝衣覆盖态","promptOrigin":"MANUAL","adoptedMediaId":"explicit-manual-material"},
            {"id":"child","variantId":"hurt","sourceAssetId":"inline","sourceRefs":[row]}
        ]});
        let executor = CountingDesigner {
            requests: std::sync::Mutex::new(vec![]),
        };
        let output = run_asset_design(&input, &executor, "offline", "medium")
            .await
            .unwrap();
        assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
        let requests = executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["asset"]["id"], "child");
        assert_eq!(requests[0]["parentDesign"]["assetId"], "inline");
        assert_eq!(requests[0]["parentDesign"]["origin"], "MANUAL");
        assert_eq!(output["assets"].as_array().unwrap().len(), 1);
        assert_eq!(output["assets"][0]["id"], "child");
    }

    struct UnassignedVisual;
    #[async_trait]
    impl TextExecutor for UnassignedVisual {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            assert_eq!(request.stage, "assetVisualFacts");
            if request.input.get("visualFactRepair").is_some() {
                return Ok(
                    json!({"schemaVersion":"asset-visual-fact-repair/v1","originalRawSha256":request.input["visualFactRepair"]["originalRawSha256"],"corrections":request.input["unassignedVisualFacts"].as_array().unwrap().iter().map(|item|json!({"factIndex":item["factIndex"],"action":"UNRESOLVED","variantId":null,"reason":"原文未定位未来外观到当前形象，保留原事实。"})).collect::<Vec<_>>()}),
                );
            }
            let identity = &request.input["identities"][0];
            let row = &request.input["rows"][0];
            Ok(
                json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":[{"assetId":identity["assetId"],"variantId":null,"attribute":"hair.style","value":"后期短发","scope":"TRANSITION","claim":"SOURCE","sourceRefs":[row["id"]]}],"issues":[]}),
            )
        }
    }

    #[tokio::test]
    async fn existing_prompt_cannot_bypass_cached_unassigned_state_or_redesign_unrelated_assets() {
        let source =
            prepare_source("第一集\n场景：门口\n△甲当前长发，后期短发。乙出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        let assets = json!([
            {"id":"subject-base","baseAssetId":"subject","variantId":"base","name":"甲","kind":"CH","sourceRefs":[row],"prompt":"用户已有设计","promptOrigin":"MANUAL","preparation":{"mode":"REFERENCE"}},
            {"id":"unrelated","baseAssetId":"other","variantId":"base","name":"乙","kind":"CH","sourceRefs":[row],"prompt":"其它已有设计","promptOrigin":"MANUAL","preparation":{"mode":"REFERENCE"}}
        ]);
        let mut input = json!({"source":source,"assets":assets,"automaticDesign":true,"imageProductionPolicy":"automatic","selection":{"assetIds":["subject-base","unrelated"]},"foundation":{"sourceSha256":source.sha256,"facts":[]}});
        let mut seed = input.clone();
        seed["assets"] = json!([assets[0].clone()]);
        let visual = super::super::asset_visual_facts::run_asset_visual_facts(
            &seed,
            &UnassignedVisual,
            "offline",
            "medium",
        )
        .await
        .unwrap();
        assert_eq!(
            visual["visualFactBatches"][0]["unassignedFacts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        input["foundation"]["visualFactBatches"] = visual["visualFactBatches"].clone();
        assert!(super::super::asset_visual_facts::cached_visual_check_required(&input).unwrap());
        let before = input.clone();
        let output = run_asset_design(&input, &Never, "offline", "medium")
            .await
            .unwrap();
        assert!(
            output["assets"].as_array().unwrap().is_empty(),
            "existing prompts must never be designed again"
        );
        assert!(output["blocked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["assetId"] == "subject-base"));
        assert!(output["blocked"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["assetId"] != "unrelated"));
        assert_eq!(input, before);
    }
}
