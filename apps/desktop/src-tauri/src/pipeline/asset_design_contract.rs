//! Stage2 design validity is deliberately independent of visual-fact receipts.
use super::{digest, resolve_asset_dependency, rule_snapshot};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};

pub const DESIGN_PROFILE: &str = "asset-design-context/v2";
pub const COMPARISON_PROFILE_FIELD: &str = "stage2ComparisonProfile";
pub const COMPARISON_PROFILE: &str = "asset-accepted-comparisons/v1";
pub(crate) const FROZEN_COMPARISON_REQUESTS: &str = "frozenAssetComparisonRequests";

pub(crate) fn incremental_comparisons(input: &Value) -> Result<bool, String> {
    match input.get(COMPARISON_PROFILE_FIELD).filter(|v| !v.is_null()) {
        None => Ok(false),
        Some(value) if value == COMPARISON_PROFILE => Ok(true),
        _ => Err("未知人物设计对照策略，保留原请求且不派发".into()),
    }
}

/// Read only requests bound to this workflow (or its explicitly bound image
/// resume ancestors). The executor remains authoritative for replay/acceptance.
pub(crate) fn frozen_comparison_requests(store: &crate::store::Store, workflow: &str) -> Result<Value, String> {
    let mut result = serde_json::Map::new();
    let mut cursor = workflow.to_owned();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(cursor.clone()) { return Err("图片续跑来源循环".into()); }
        let parent = store.job(&cursor)?;
        if parent["kind"] != "workflow" || !incremental_comparisons(&parent["snapshot"]["stage2Context"])? {
            return Err("人物对照恢复缺少原工作流策略".into());
        }
        let connection = store.conn()?;
        let mut query = connection.prepare("SELECT w.node_key,w.request_hash,j.snapshot,j.kind FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage='assetDesign' ORDER BY w.rowid").map_err(|e|e.to_string())?;
        let rows = query.query_map([&cursor], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?))).map_err(|e|e.to_string())?;
        for row in rows {
            let (node, hash, raw, kind) = row.map_err(|e|e.to_string())?;
            let raw = raw.ok_or("人物对照恢复缺少绑定请求")?;
            let saved: Value = serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            let asset = &saved["input"]["asset"];
            let expected_node = format!("assetDesign:asset:{}",digest(&json!({"id":asset["id"],"variantId":asset["variantId"]}).to_string()));
            if kind.as_deref() != Some("text") || raw != saved.to_string() || hash != digest(&raw) || node != expected_node
                || saved["stage"] != "assetDesign" || saved["skill"] != "asset-art-director"
                || saved["input"]["sourceSha256"] != parent["snapshot"]["stage2Context"]["source"]["sha256"]
                || !incremental_comparisons(&saved["input"])? || !saved["input"]["comparisonDesigns"].is_array()
                || s(asset,"id").is_empty()
            { return Err("人物对照恢复的请求绑定或摘要不匹配".into()); }
            let key = s(asset,"id").to_owned();
            if let Some(previous) = result.get(&key) {
                // Native execution identity may differ across an image resume;
                // the immutable domain input/schema/rules must still agree.
                if comparison_domain_input(&previous["input"]) != comparison_domain_input(&saved["input"]) {
                    return Err("同一人物存在冲突的已派发设计请求".into());
                }
                for field in ["stage","skill","model","reasoning","schema","ruleSnapshot"] {
                    if previous[field] != saved[field] { return Err("同一人物存在冲突的已派发设计请求".into()); }
                }
            } else { result.insert(key,saved); }
        }
        let origin = &parent["snapshot"]["request"]["imageResumeOrigin"];
        let Some(previous) = origin["workflowId"].as_str() else { break; };
        let original = store.job(previous)?;
        if origin["snapshotSha256"] != digest(&original["snapshot"].to_string()) {
            return Err("图片续跑来源快照变化，不能重建人物对照".into());
        }
        cursor = previous.to_owned();
    }
    Ok(Value::Object(result))
}

/// Transport retry proofs are revalidated/restored by TextExecutor immediately
/// before dispatch. Compare the creative domain without rewriting saved bytes.
pub(crate) fn comparison_domain_input(input: &Value) -> Value {
    let mut input=input.clone();
    if let Some(object)=input.as_object_mut() { object.remove("textTransportRetry"); }
    input
}
pub(crate) use super::assets::{freeze_inline_delta_profile, INLINE_DELTA_PROFILE_FIELD};
pub const IDENTITY_MATERIAL_PROFILE: &str = "asset-identity-production-material/v1";
pub const IDENTITY_MATERIAL_PROFILE_FIELD: &str = "stage2IdentityMaterialProfile";
pub const IDENTITY_MATERIALS_FIELD: &str = "identityProductionMaterials";
pub const SCOPE_MATERIALS_FIELD: &str = "assetScopeProductionMaterials";
pub const SCOPE_MATERIAL_PROFILE: &str = "asset-scope-production-material/v1";
pub const IDENTITY_MATERIAL_POLICY: &str = "identityProductionMaterial/identityProductionMaterials 是已采用制作基础的主体身份说明，scopeProductionMaterial/assetScopeProductionMaterials 是准确形象适用范围的制作理由。两者仅供定位身份与理解制作意图，不是 SOURCE 视觉事实，不能写入 facts/deltaFacts 或代替原文证据。当前 asset.description、形象范围及 preparation 仍独立保留；说明不覆盖用户编辑、子形象说明或当前状态。涉及外观、年龄、物种和未来变化必须核对本包锁定原文、准确主体及适用版本；说明与原文或当前形象冲突时保留差异，不移植为外观事实。";
pub const PRESENTATION_PROFILE: &str = "asset-image-presentation/v1";
pub const CH_PRESENTATION: &str = "左侧大幅肖像/近景，右侧正面、侧面、背面全身；各区保持同一身份、发型、衣装和当前状态，全身头脚完整，肖像保留头顶发型，各分区清晰，不以浅景深虚化其它视图，干净中性背景，无资料栏、标签。";
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
fn first<'a>(values: impl IntoIterator<Item = &'a str>) -> &'a str {
    values
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
}
pub fn is_character(asset: &Value) -> bool {
    matches!(
        first([s(asset, "kind"), s(asset, "type")]),
        "CH" | "character" | "人物"
    )
}
pub fn identity(asset: &Value) -> &str {
    first([
        s(asset, "identityId"),
        s(asset, "baseAssetId"),
        s(asset, "id"),
    ])
}
/// Freeze this addendum only at a new Stage2 workflow boundary. Existing flat
/// rows keep their user/variant description; old workflow inputs remain legacy.
pub fn freeze_identity_production_materials(input: &mut Value) -> Result<(), String> {
    let source_sha = s(&input["source"], "sha256");
    let foundation_sha = s(&input["foundation"], "sourceSha256");
    if !foundation_sha.is_empty() && foundation_sha != source_sha {
        return Err("身份制作资料不属于当前锁定原文，未冻结新设计输入".into());
    }
    let mut registered = HashSet::new();
    for asset in input["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| !crate::store::asset_deleted(asset))
    {
        if !s(asset, "identityId").is_empty()
            && !s(asset, "baseAssetId").is_empty()
            && s(asset, "identityId") != s(asset, "baseAssetId")
        {
            return Err("资产的主体身份字段冲突，不能选择制作说明".into());
        }
        registered.insert(identity(asset));
    }
    let mut materials = serde_json::Map::new();
    let mut seen = HashSet::new();
    for subject in input["foundation"]["assets"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let id = s(subject, "id");
        if id.is_empty() || !registered.contains(id) {
            continue;
        }
        if !seen.insert(id) {
            return Err("制作基础主体身份重复，不能选择身份说明".into());
        }
        let description = s(subject, "description");
        if description.trim().is_empty() {
            continue;
        }
        materials.insert(id.to_owned(), json!({"version":IDENTITY_MATERIAL_PROFILE,"identityId":id,"description":description,"sourceSha256":source_sha,"provenance":"FOUNDATION_IDENTITY_DESCRIPTION_NOT_VISUAL_FACT"}));
    }
    let scope_materials = projected_scope_materials(input)?;
    input[IDENTITY_MATERIAL_PROFILE_FIELD] = json!(IDENTITY_MATERIAL_PROFILE);
    input[IDENTITY_MATERIALS_FIELD] = Value::Object(materials);
    input[SCOPE_MATERIALS_FIELD] = Value::Object(scope_materials);
    Ok(())
}

pub(crate) fn scope_pairs(scope: &Value) -> Option<BTreeSet<(String, String)>> {
    if scope["kind"] != "SCENES" {
        return None;
    }
    let mut pairs = BTreeSet::new();
    for place in scope["scenes"].as_array()? {
        let (episode, scene) = if let Some(pair) = place.as_array().filter(|pair| pair.len() == 2) {
            (pair[0].as_str()?, pair[1].as_str()?)
        } else {
            (place["episodeId"].as_str()?, place["sceneId"].as_str()?)
        };
        if episode.is_empty()
            || scene.is_empty()
            || !pairs.insert((episode.to_owned(), scene.to_owned()))
        {
            return None;
        }
    }
    (!pairs.is_empty()).then_some(pairs)
}

/// The legacy flat row and the current foundation must describe exactly the
/// same scoped version. Nonempty user reasons are authoritative and untouched.
fn projected_scope_materials(input: &Value) -> Result<serde_json::Map<String, Value>, String> {
    let mut materials = serde_json::Map::new();
    if input["foundation"]["sourceSha256"] != input["source"]["sha256"]
        || input["foundation"]["sourceVersion"] != input["source"]["version"]
        || !input["foundation"]["version"]
            .as_u64()
            .is_some_and(|v| v > 0)
    {
        return Ok(materials);
    }
    for asset in input["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| !crate::store::asset_deleted(asset))
    {
        if !asset["version"].as_u64().is_some_and(|v| v > 0) {
            continue;
        }
        // Current saved/user reasons are production context in their own right;
        // the foundation is only a fallback for the old missing-reason shape.
        let own_reason = s(&asset["scope"], "reason");
        let Some(pairs) = scope_pairs(&asset["scope"]) else {
            continue;
        };
        if !pairs.iter().all(|(episode, scene)| {
            input["source"]["scenes"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|known| {
                    known["id"] == scene.as_str() && known["episodeId"] == episode.as_str()
                })
        }) {
            continue;
        }
        let ids = |key: &str| {
            asset["applicability"][key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
        };
        if ids("episodes") != pairs.iter().map(|(episode, _)| episode.clone()).collect()
            || ids("scenes") != pairs.iter().map(|(_, scene)| scene.clone()).collect()
        {
            continue;
        }
        let (reason, provenance) = if own_reason.trim().is_empty() {
            let owners: Vec<_> = input["foundation"]["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|owner| owner["id"] == identity(asset))
                .collect();
            let [owner] = owners.as_slice() else {
                continue;
            };
            let variants: Vec<_> = owner["variants"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|variant| variant["id"] == asset["variantId"])
                .collect();
            let [variant] = variants.as_slice() else {
                continue;
            };
            let reason = s(&variant["scope"], "reason");
            if reason.trim().is_empty()
                || variant
                    .get("assetRowId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id != s(asset, "id"))
                || variant
                    .get("assetVersion")
                    .is_some_and(|version| version != &asset["version"])
                || variant["sourceRefs"] != asset["sourceRefs"]
                || scope_pairs(&variant["scope"]).as_ref() != Some(&pairs)
            {
                continue;
            }
            (reason.to_owned(), "FOUNDATION_SCOPE_REASON_NOT_VISUAL_FACT")
        } else {
            (
                own_reason.to_owned(),
                "CURRENT_ASSET_SCOPE_REASON_NOT_VISUAL_FACT",
            )
        };
        let mut scope = asset["scope"].clone();
        scope["reason"] = json!(reason);
        let material = json!({"version":SCOPE_MATERIAL_PROFILE,"assetId":asset["id"],"identityId":identity(asset),"variantId":asset["variantId"],"assetVersion":asset["version"],"foundationVersion":input["foundation"]["version"],"sourceVersion":input["source"]["version"],"sourceSha256":input["source"]["sha256"],"scope":scope,"originalScopeSha256":digest(&asset["scope"].to_string()),"sourceRefsSha256":digest(&asset["sourceRefs"].to_string()),"provenance":provenance});
        if materials
            .insert(s(asset, "id").to_owned(), material)
            .is_some()
        {
            return Err("形象范围制作资料的资产行身份重复".into());
        }
    }
    Ok(materials)
}

pub fn scope_production_material(input: &Value, asset: &Value) -> Result<Option<Value>, String> {
    if input[IDENTITY_MATERIAL_PROFILE_FIELD].as_str() != Some(IDENTITY_MATERIAL_PROFILE) {
        return Ok(None);
    }
    let Some(material) = input[SCOPE_MATERIALS_FIELD].get(s(asset, "id")) else {
        return Ok(None);
    };
    if material["version"] != SCOPE_MATERIAL_PROFILE
        || material["assetId"] != asset["id"]
        || material["identityId"] != identity(asset)
        || material["variantId"] != asset["variantId"]
        || material["sourceSha256"] != input["source"]["sha256"]
        || material["sourceVersion"] != input["source"]["version"]
        || !matches!(
            s(material, "provenance"),
            "FOUNDATION_SCOPE_REASON_NOT_VISUAL_FACT"
                | "CURRENT_ASSET_SCOPE_REASON_NOT_VISUAL_FACT"
        )
        || material["provenance"] == "CURRENT_ASSET_SCOPE_REASON_NOT_VISUAL_FACT"
            && material["scope"] != asset["scope"]
        || !material["assetVersion"]
            .as_u64()
            .is_some_and(|version| version > 0)
        || material["originalScopeSha256"] != digest(&asset["scope"].to_string())
        || material["sourceRefsSha256"] != digest(&asset["sourceRefs"].to_string())
        || scope_pairs(&material["scope"]) != scope_pairs(&asset["scope"])
        || scope_pairs(&material["scope"]).is_none()
        || s(&material["scope"], "reason").trim().is_empty()
    {
        return Err("形象范围制作说明的主体/版本/集场或来源绑定无效".into());
    }
    Ok(Some(material.clone()))
}

/// Stage3 publishes units, never assets. Enrich only its new input clone. Stage2
/// instead applies this same reason in model cards, retaining raw asset hashes.
pub(crate) fn apply_scope_reasons_to_new_asset_context(input: &mut Value) -> Result<(), String> {
    let materials = projected_scope_materials(input)?;
    for asset in input["assets"].as_array_mut().into_iter().flatten() {
        if let Some(material) = materials.get(s(asset, "id")) {
            asset["scope"]["reason"] = material["scope"]["reason"].clone();
        }
    }
    input["foundationScopeReasonBindings"] = Value::Object(materials);
    Ok(())
}

fn scope_material_semantics(material: &Value) -> Value {
    let mut result = material.clone();
    if let Some(object) = result.as_object_mut() {
        for counter in ["assetVersion", "foundationVersion", "sourceVersion"] {
            object.remove(counter);
        }
    }
    result
}

fn consumed_scope_materials(
    input: &Value,
    asset_ids: &HashSet<&str>,
) -> Result<Vec<Value>, String> {
    let mut materials = Vec::new();
    for asset in input["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| asset_ids.contains(s(asset, "id")))
    {
        if let Some(material) = scope_production_material(input, asset)? {
            materials.push(scope_material_semantics(&material));
        }
    }
    materials.sort_by_key(Value::to_string);
    Ok(materials)
}

pub fn identity_production_material(input: &Value, asset: &Value) -> Result<Option<Value>, String> {
    match input.get(IDENTITY_MATERIAL_PROFILE_FIELD) {
        None | Some(Value::Null) => return Ok(None), // Never enrich an old frozen request.
        Some(value) if value.as_str() == Some(IDENTITY_MATERIAL_PROFILE) => {}
        _ => return Err("未知的第二阶段身份制作资料版本，未创建请求".into()),
    }
    let materials = input[IDENTITY_MATERIALS_FIELD]
        .as_object()
        .ok_or("冻结身份制作资料目录无效")?;
    let Some(material) = materials.get(identity(asset)) else {
        return Ok(None); // No name/alias/first-item fallback across identities.
    };
    if material["version"] != IDENTITY_MATERIAL_PROFILE
        || material["identityId"] != identity(asset)
        || material["sourceSha256"] != input["source"]["sha256"]
        || material["provenance"] != "FOUNDATION_IDENTITY_DESCRIPTION_NOT_VISUAL_FACT"
        || s(material, "description").trim().is_empty()
    {
        return Err("身份制作资料的源版本或准确主体绑定无效".into());
    }
    Ok(Some(material.clone()))
}

fn consumed_identity_materials(
    input: &Value,
    subjects: &HashSet<&str>,
) -> Result<Vec<Value>, String> {
    let mut ids: Vec<_> = subjects.iter().copied().collect();
    ids.sort_unstable();
    ids.into_iter()
        .filter_map(|id| identity_production_material(input, &json!({"identityId":id})).transpose())
        .collect()
}
pub fn project_visual_style(settings: &Value) -> &str {
    let style = first([
        s(settings, "projectVisualStyle"),
        s(settings, "visualStyle"),
    ])
    .trim();
    if style == "未选择" {
        ""
    } else {
        style
    }
}
pub fn production_direction(input: &Value) -> Value {
    if input["settings"]["productionDirectionConfirmed"] == true {
        input["settings"]["productionDirection"].clone()
    } else if input["foundation"]["directionConfirmed"] == true {
        input["foundation"]["productionDirection"].clone()
    } else {
        Value::Null
    }
}
/// Explicit per-asset composition wins; the project CH override never leaks to other kinds.
pub fn image_presentation(settings: &Value, asset: &Value) -> Value {
    let own = &asset["imagePresentation"];
    let project = &settings["imagePresentation"];
    let explicit = first([
        own.as_str().unwrap_or(""),
        s(own, "composition"),
        s(asset, "composition"),
    ]);
    let project_ch = first([project.as_str().unwrap_or(""), s(project, "composition")]);
    let composition = if !explicit.is_empty() {
        explicit
    } else if is_character(asset) {
        if project_ch.is_empty() {
            CH_PRESENTATION
        } else {
            project_ch
        }
    } else {
        ""
    };
    let ratio = first([
        s(own, "ratio"),
        s(asset, "ratio"),
        s(settings, "ratio"),
        if is_character(asset) { "16:9" } else { "" },
    ]);
    json!({"version":PRESENTATION_PROFILE,"composition":composition,"ratio":ratio,"origin":if !explicit.is_empty(){"asset"}else if is_character(asset)&&!project_ch.is_empty(){"project"}else{"default"}})
}
/// Never sanitize source words (for example a sign reading 第十集). Exact existing
/// components in legacy/manual prose are not duplicated, and saved prose is untouched.
pub fn compile_image_prompt(body: &str, style: &str, presentation: &Value) -> String {
    let body = body.trim();
    let style = style.trim();
    let layout = s(presentation, "composition").trim();
    let mut parts = Vec::new();
    if !style.is_empty() && !body.starts_with(style) {
        parts.push(style);
    }
    parts.push(body);
    if !layout.is_empty() && !body.contains(layout) && style != layout {
        parts.push(layout);
    }
    parts.join("\n\n")
}
/// A semantic version ignores save counters, adopted pictures and generated output.
pub fn asset_semantics(asset: &Value) -> Value {
    let mut result = json!({});
    for key in [
        "id",
        "identityId",
        "baseAssetId",
        "variantId",
        "name",
        "kind",
        "type",
        "description",
        "sourceRefs",
        "sourceRefsPurpose",
        "sourceScope",
        "scope",
        "applicability",
        "preparation",
        "imagePresentation",
        "composition",
        "ratio",
    ] {
        result[key] = asset[key].clone();
    }
    result
}
pub fn source_chain(assets: &[Value], asset: &Value) -> Result<Vec<Value>, String> {
    let mut chain = Vec::new();
    let mut current = asset;
    while let Some(id) = resolve_asset_dependency(assets, current)? {
        current = assets
            .iter()
            .find(|a| s(a, "id") == id)
            .ok_or("来源设计已不存在")?;
        chain.push(json!({"assetId":id,"identityId":identity(current),"semanticVersion":digest(&asset_semantics(current).to_string()),"designVersion":digest(&json!({"prompt":current["prompt"],"receipt":current["design"]["designReceipt"]}).to_string())}));
    }
    Ok(chain)
}
fn fact_semantics(fact: &Value) -> Value {
    json!({"id":fact["id"],"assetId":fact["assetId"],"variantId":fact["variantId"],"attribute":fact["attribute"],"value":fact["value"],"scope":fact["scope"],"claim":fact["claim"],"sourceRefs":fact["sourceRefs"]})
}
/// Resume may reuse the original workflow's published facts, never silently read
/// later edits into a request already bound to a design/source version.
pub fn validate_resume_facts(
    original: &Value,
    result: &Value,
    current: &Value,
) -> Result<(), String> {
    if original["source"]["sha256"] != current["source"]["sha256"] {
        return Err("图片续跑的锁定来源已变化".into());
    }
    if let Some(rule) = original["designRuleSha256"].as_str() {
        if rule_snapshot("asset-art-director")?["sha256"] != rule {
            return Err("图片续跑美术规则版本已变化；原请求保留，请发起新设计".into());
        }
    }
    let assets = current["assets"].as_array().ok_or("图片续跑缺少资产目录")?;
    let selection = current["selection"]["assetIds"].as_array();
    let mut subjects = HashSet::new();
    let mut related_asset_ids = HashSet::new();
    for asset in assets
        .iter()
        .filter(|a| selection.is_none_or(|ids| ids.contains(&a["id"])))
    {
        subjects.insert(identity(asset).to_owned());
        related_asset_ids.insert(s(asset, "id").to_owned());
        for source in source_chain(assets, asset)? {
            subjects.insert(s(&source, "identityId").to_owned());
            related_asset_ids.insert(s(&source, "assetId").to_owned());
        }
    }
    let expected = result
        .get("facts")
        .filter(|f| f.is_array())
        .unwrap_or(&original["foundation"]["facts"]);
    let normalized = |facts: &Value| {
        let mut facts: Vec<_> = facts
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| subjects.contains(s(f, "assetId")))
            .map(fact_semantics)
            .collect();
        facts.sort_by_key(Value::to_string);
        facts
    };
    if normalized(expected) != normalized(&current["foundation"]["facts"]) {
        return Err("图片续跑的相关视觉事实语义版本已变化；原包与预算保留，请发起新制作".into());
    }
    if original
        .get(IDENTITY_MATERIAL_PROFILE_FIELD)
        .is_some_and(|value| !value.is_null())
    {
        let mut current_materials = current.clone();
        freeze_identity_production_materials(&mut current_materials)?;
        let selected: HashSet<_> = subjects.iter().map(String::as_str).collect();
        if consumed_identity_materials(original, &selected)?
            != consumed_identity_materials(&current_materials, &selected)?
        {
            return Err("图片续跑的相关主体制作说明已变化；原请求与预算保留，请发起新设计".into());
        }
        let related: HashSet<_> = related_asset_ids.iter().map(String::as_str).collect();
        let expected_scope = consumed_scope_materials(original, &related)?;
        let mut frozen_scope = current.clone();
        frozen_scope[IDENTITY_MATERIAL_PROFILE_FIELD] =
            original[IDENTITY_MATERIAL_PROFILE_FIELD].clone();
        frozen_scope[SCOPE_MATERIALS_FIELD] = original[SCOPE_MATERIALS_FIELD].clone();
        if expected_scope != consumed_scope_materials(&frozen_scope, &related)? {
            return Err("图片续跑的准确形象制作范围已变化，原请求未重建".into());
        }
        for material in expected_scope {
            if material["provenance"] == "CURRENT_ASSET_SCOPE_REASON_NOT_VISUAL_FACT" {
                continue; // Its saved row hash above is authoritative, not the foundation reason.
            }
            let variants: Vec<_> = current["foundation"]["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|owner| owner["id"] == material["identityId"])
                .flat_map(|owner| owner["variants"].as_array().into_iter().flatten())
                .filter(|variant| variant["id"] == material["variantId"])
                .collect();
            let [variant] = variants.as_slice() else {
                return Err("图片续跑的范围制作说明主体/形象不存在或身份重复".into());
            };
            let current_reason = s(&variant["scope"], "reason");
            if scope_pairs(&variant["scope"]) != scope_pairs(&material["scope"])
                || material["sourceRefsSha256"] != digest(&variant["sourceRefs"].to_string())
                || !current_reason.trim().is_empty()
                    && current_reason != s(&material["scope"], "reason")
            {
                return Err("图片续跑的已冻结形象范围制作理由已变化，未重新投影或重发".into());
            }
        }
    }
    super::asset_production_context::validate_resume(original, result, current)?;
    Ok(())
}
pub fn design_receipt(input: &Value, asset: &Value) -> Result<Value, String> {
    let assets = input["assets"].as_array().ok_or("缺少资产目录")?;
    let chain = source_chain(assets, asset)?;
    let subjects: HashSet<_> = std::iter::once(identity(asset))
        .chain(chain.iter().filter_map(|v| v["identityId"].as_str()))
        .collect();
    let mut facts: Vec<_> = input["foundation"]["facts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| subjects.contains(s(f, "assetId")))
        .map(fact_semantics)
        .collect();
    facts.sort_by_key(Value::to_string);
    let mut basis = json!({"version":DESIGN_PROFILE,"ruleSha256":rule_snapshot("asset-art-director")?["sha256"],"sourceSha256":input["source"]["sha256"],"assetSemanticVersion":digest(&asset_semantics(asset).to_string()),"sourceChain":chain,"factsSha256":digest(&json!(facts).to_string()),"projectVisualStyle":project_visual_style(&input["settings"]),"productionDirection":production_direction(input),"imagePresentation":image_presentation(&input["settings"],asset)});
    if let Some(context) = super::assets::inline_delta_context(input, asset)? {
        basis["inlineDeltaContext"] = context;
    }
    let materials = consumed_identity_materials(input, &subjects)?;
    if !materials.is_empty() {
        basis["identityProductionMaterialsSha256"] = json!(digest(&json!(materials).to_string()));
    }
    let scoped_ids: HashSet<_> = std::iter::once(s(asset, "id"))
        .chain(
            chain
                .iter()
                .filter_map(|ancestor| ancestor["assetId"].as_str()),
        )
        .collect();
    let scope_materials = consumed_scope_materials(input, &scoped_ids)?;
    if !scope_materials.is_empty() {
        basis["assetScopeProductionMaterialsSha256"] =
            json!(digest(&json!(scope_materials).to_string()));
    }
    if let Some(context) = super::asset_production_context::context(input, asset)? {
        basis["assetProductionContext"] = super::asset_production_context::binding(&context);
    }
    Ok(json!({"version":DESIGN_PROFILE,"sha256":digest(&basis.to_string()),"basis":basis}))
}
pub fn design_receipt_current(input: &Value, asset: &Value) -> bool {
    design_receipt(input, asset).is_ok_and(|expected| asset["design"]["designReceipt"] == expected)
}
/// Bounded aesthetic context only: never a source fact, cache dependency or image reference.
pub fn comparison_designs(input: &Value, asset: &Value) -> Vec<Value> {
    if !is_character(asset) {
        return Vec::new();
    }
    let Some(assets) = input["assets"].as_array() else {
        return Vec::new();
    };
    let Ok(chain) = source_chain(assets, asset) else {
        return Vec::new();
    };
    let mut seen: HashSet<String> = std::iter::once(identity(asset).to_string())
        .chain(chain.iter().map(|a| s(a, "identityId").to_string()))
        .collect();
    let mut candidates: Vec<_> = assets
        .iter()
        .filter(|a| {
            is_character(a) && !crate::store::asset_deleted(a) && !s(a, "prompt").trim().is_empty()
        })
        .collect();
    candidates.sort_by_key(|a| (identity(a), s(a, "id")));
    let mut remaining: usize = 6400 - 2; // Include array brackets, separators and metadata in the cap.
    let mut result = Vec::new();
    for other in candidates {
        let ai = s(other, "promptOrigin") == "AI"
            || s(&other["design"], "schemaVersion") == "asset-design/v1";
        if ai
            && s(other, "promptOrigin") != "MANUAL"
            && super::assets::automatic_design_stale(input, other)
        {
            continue;
        }
        if !seen.insert(identity(other).to_string()) {
            continue;
        }
        let chars: Vec<_> = s(other, "prompt").chars().take(1600).collect();
        let mut item = json!({"assetId":other["id"],"identityId":identity(other),"prompt":"","usage":"AESTHETIC_COMPARISON_ONLY_NOT_SOURCE_OR_IMAGE_DEPENDENCY"});
        let separator = usize::from(!result.is_empty());
        let budget = remaining.saturating_sub(separator);
        let mut low = 0;
        let mut high = chars.len();
        while low < high {
            let mid = (low + high + 1) / 2;
            item["prompt"] = json!(chars[..mid].iter().collect::<String>());
            if item.to_string().chars().count() <= budget {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        if low == 0 {
            continue;
        }
        item["prompt"] = json!(chars[..low].iter().collect::<String>());
        remaining -= item.to_string().chars().count() + separator;
        result.push(item);
        if result.len() == 8 || remaining == 0 {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incremental_comparison_bound_request_recovery_is_read_only_and_rejects_drift() {
        let temp=tempfile::tempdir().unwrap();
        let store=crate::store::Store::open(temp.path().join("project")).unwrap();
        store.ensure_workflow_storage().unwrap();
        let mut context=json!({"source":{"sha256":"source-sha"}});
        context[COMPARISON_PROFILE_FIELD]=json!(COMPARISON_PROFILE);
        let parent=store.add_job("workflow","images.design","local","local",&json!({"stage":"images.design","stage2Context":context}),"running").unwrap();
        let mut input=json!({"sourceSha256":"source-sha","asset":{"id":"a","variantId":"base"},"comparisonDesigns":[]});
        input[COMPARISON_PROFILE_FIELD]=json!(COMPARISON_PROFILE);
        let saved=json!({"stage":"assetDesign","skill":"asset-art-director","input":input,"model":"offline","reasoning":"medium","schema":{},"ruleSnapshot":{}});
        let job=store.add_job("text","design","codex","offline",&saved,"received").unwrap();
        let node=format!("assetDesign:asset:{}",digest(&json!({"id":"a","variantId":"base"}).to_string()));
        store.checkpoint(&parent,&node,"assetDesign",&digest(&saved.to_string()),"received",Some(&job),None,None).unwrap();
        let before=store.job(&job).unwrap();
        assert_eq!(frozen_comparison_requests(&store,&parent).unwrap()["a"],saved);
        assert_eq!(store.job(&job).unwrap(),before);
        let origin=store.job(&parent).unwrap();
        let resume=store.add_job("workflow","images.produce","local","local",&json!({"stage":"images.produce","stage2Context":context,"request":{"imageResumeOrigin":{"workflowId":parent,"snapshotSha256":digest(&origin["snapshot"].to_string())}}}),"queued").unwrap();
        assert_eq!(frozen_comparison_requests(&store,&resume).unwrap()["a"],saved);
        store.conn().unwrap().execute("UPDATE workflow_steps SET request_hash='changed' WHERE workflow_id=?1",[&parent]).unwrap();
        assert!(frozen_comparison_requests(&store,&resume).unwrap_err().contains("摘要不匹配"));
        assert_eq!(store.job(&job).unwrap(),before);
        assert!(!incremental_comparisons(&json!({})).unwrap());
        assert!(incremental_comparisons(&json!({COMPARISON_PROFILE_FIELD:"unknown"})).is_err());
    }
    #[test]
    fn incremental_comparison_cannot_upgrade_a_bound_legacy_resume_ancestor() {
        let temp=tempfile::tempdir().unwrap();
        let store=crate::store::Store::open(temp.path().join("project")).unwrap();
        store.ensure_workflow_storage().unwrap();
        // Covers both pre-context records and older frozen comparison maps.
        for (index,context) in [Value::Null,json!({"comparisonDesignsByAssetId":{"a":[]}})].into_iter().enumerate() {
            let original=store.add_job("workflow","images.produce","local","local",
                &json!({"stage":"images.produce","stage2Context":context}),"completed").unwrap();
            let before=store.job(&original).unwrap();
            let resume=store.add_job("workflow",&format!("images.produce:{index}"),"local","local",
                &json!({"stage":"images.produce","stage2Context":{COMPARISON_PROFILE_FIELD:COMPARISON_PROFILE},
                    "request":{"imageResumeOrigin":{"workflowId":original,"snapshotSha256":digest(&before["snapshot"].to_string())}}}),"queued").unwrap();
            assert!(frozen_comparison_requests(&store,&resume).unwrap_err().contains("缺少原工作流策略"),
                "an accidental policy upgrade must fail closed, not resample or bypass the ancestor");
            assert_eq!(store.job(&original).unwrap(),before);
        }
    }
    fn basis() -> Value {
        json!({"source":{"sha256":"locked"},"foundation":{"facts":[]},"settings":{"projectVisualStyle":"水墨国漫","ratio":"9:16"},"assets":[{"id":"a","identityId":"a","variantId":"base","kind":"CH","prompt":"成熟深肤色人物","promptOrigin":"MANUAL"}]})
    }
    #[test]
    fn presentation_defaults_only_characters_and_preserves_explicit_ratio_and_layout() {
        for kind in ["CH", "character"] {
            for variant in ["base", "hurt"] {
                let asset = json!({"kind":kind,"variantId":variant});
                assert_eq!(
                    image_presentation(&json!({}), &asset)["composition"],
                    CH_PRESENTATION
                );
                assert_eq!(image_presentation(&json!({}), &asset)["ratio"], "16:9");
                assert_eq!(
                    image_presentation(&json!({"ratio":"9:16"}), &asset)["ratio"],
                    "9:16"
                );
            }
        }
        for kind in ["GROUP", "BEAST", "LOC", "PROP", "scene", "prop"] {
            assert_eq!(
                image_presentation(
                    &json!({"imagePresentation":"人物四视图"}),
                    &json!({"kind":kind})
                )["composition"],
                ""
            );
        }
        let explicit =
            json!({"kind":"CH","imagePresentation":{"composition":"单幅正面半身","ratio":"3:2"}});
        assert_eq!(
            image_presentation(
                &json!({"ratio":"16:9","imagePresentation":"项目构图"}),
                &explicit
            )["composition"],
            "单幅正面半身"
        );
        assert_eq!(
            image_presentation(&json!({"ratio":"16:9"}), &explicit)["ratio"],
            "3:2"
        );
    }
    #[test]
    fn compiler_preserves_visible_words_and_adds_components_once_without_rewriting_body() {
        let p = image_presentation(&json!({}), &json!({"kind":"CH"}));
        let body = "六十岁深肤色人物，招牌写“第十集”。";
        let prompt = compile_image_prompt(body, "水墨国漫", &p);
        assert_eq!(prompt.matches("水墨国漫").count(), 1);
        assert_eq!(prompt.matches(CH_PRESENTATION).count(), 1);
        assert!(prompt.contains(body));
        assert_eq!(compile_image_prompt(&prompt, "水墨国漫", &p), prompt);
    }
    #[test]
    fn design_cache_tracks_rule_style_presentation_asset_source_but_not_other_designs_or_fact_rule()
    {
        let mut input = basis();
        input["assets"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"b","kind":"CH","prompt":"另一人","promptOrigin":"MANUAL"}));
        let receipt = design_receipt(&input, &input["assets"][0]).unwrap();
        input["assets"][0]["design"] = json!({"designReceipt":receipt});
        input["assets"][1]["prompt"] = json!("另一人新设计");
        input["foundation"]["visualFactBatches"] = json!([{"ruleSha256":"changed"}]);
        input["assets"][0]["version"] = json!(88);
        assert!(design_receipt_current(&input, &input["assets"][0]));
        for (field, value) in [
            ("projectVisualStyle", json!("CG")),
            ("imagePresentation", json!("正面")),
            ("ratio", json!("3:2")),
            ("productionDirection", json!("朴素布料")),
        ] {
            let mut changed = input.clone();
            changed["settings"][field] = value;
            if field == "productionDirection" {
                changed["settings"]["productionDirectionConfirmed"] = json!(true);
            }
            assert!(
                !design_receipt_current(&changed, &changed["assets"][0]),
                "{field}"
            );
        }
        input["assets"][0]["description"] = json!("年龄与体态变更");
        assert!(!design_receipt_current(&input, &input["assets"][0]));
    }
    #[test]
    fn comparison_identity_exclusions_and_bounds_are_stable_and_not_dependencies() {
        let mut input = basis();
        let current = json!({"id":"child","identityId":"a","kind":"CH","sourceAssetId":"a"});
        input["assets"]
            .as_array_mut()
            .unwrap()
            .push(current.clone());
        for (id, kind, origin, extra) in [
            ("self-alias", "CH", "MANUAL", json!({"identityId":"a"})),
            ("deleted", "CH", "MANUAL", json!({"deletedAt":"now"})),
            ("beast", "BEAST", "MANUAL", json!({})),
            ("stale", "CH", "AI", json!({})),
        ] {
            let mut a = json!({"id":id,"kind":kind,"promptOrigin":origin,"prompt":"不该出现"});
            for (k, v) in extra.as_object().unwrap() {
                a[k] = v.clone();
            }
            input["assets"].as_array_mut().unwrap().push(a);
        }
        for n in 0..12 {
            input["assets"].as_array_mut().unwrap().push(json!({"id":format!("p{n:02}"),"identityId":format!("person{}",n/2),"kind":"CH","promptOrigin":"MANUAL","prompt":"样".repeat(2000)}));
        }
        let context = comparison_designs(&input, &current);
        assert_eq!(context.len(), 4);
        assert!(context
            .iter()
            .all(|a| s(a, "prompt").chars().count() <= 1600));
        assert!(json!(context).to_string().chars().count() <= 6400);
        assert!(context.iter().all(|a| s(a, "assetId").starts_with('p')));
        input["assets"].as_array_mut().unwrap().reverse();
        assert_eq!(comparison_designs(&input, &current), context);
        assert_eq!(
            source_chain(input["assets"].as_array().unwrap(), &current)
                .unwrap()
                .len(),
            1
        );
        for a in input["assets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .filter(|a| s(a, "id").starts_with('p'))
        {
            a["prompt"] = json!("短设计");
            a["identityId"] = a["id"].clone();
        }
        assert_eq!(comparison_designs(&input, &current).len(), 8);
    }
    #[test]
    fn direct_source_design_semantics_invalidate_descendant_but_counters_do_not() {
        let mut input = basis();
        input["assets"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"child","kind":"CH","sourceAssetId":"a"}));
        let receipt = design_receipt(&input, &input["assets"][1]).unwrap();
        input["assets"][1]["design"] = json!({"designReceipt":receipt});
        input["assets"][0]["version"] = json!(20);
        assert!(design_receipt_current(&input, &input["assets"][1]));
        input["assets"][0]["prompt"] = json!("原脸保持，新衣装");
        assert!(!design_receipt_current(&input, &input["assets"][1]));
    }
    #[test]
    fn resume_preserves_published_facts_but_rejects_relevant_later_edits_and_rule_change() {
        let original = basis();
        let fact = json!({"id":"f","assetId":"a","value":"原肤色"});
        let result = json!({"facts":[fact]});
        let mut current = original.clone();
        current["foundation"]["facts"] = json!([fact,{"assetId":"unrelated","value":"新衣装"}]);
        assert!(validate_resume_facts(&original, &result, &current).is_ok());
        current["foundation"]["facts"][0]["quotes"] = json!(["程序物化引文"]);
        assert!(validate_resume_facts(&original, &result, &current).is_ok());
        current["foundation"]["facts"][0]["value"] = json!("后来修改肤色");
        assert!(validate_resume_facts(&original, &result, &current).is_err());
        let mut old_rule = original.clone();
        old_rule["designRuleSha256"] = json!("old-art-rule");
        assert!(validate_resume_facts(&old_rule, &json!({}), &original).is_err());
    }
}
