//! New production only. Historical workflows retain their original profile,
//! role requests, raw responses and recovery path.
use super::{
    array, choices, digest, obj, string, LockedSource, RuleSnapshot, SourceEpisode, SourceRow,
    TextExecutor, TextRequest,
};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
#[path = "series_design.rs"]
mod design;
pub(crate) use design::validate_unpublished_design_predecessor;
#[path = "series_design_source.rs"]
mod design_source;
#[path = "series_supply_correction.rs"]
mod supply_correction;
#[path = "series_supply_revision.rs"]
mod supply_revision;
pub(crate) use supply_correction::{
    revalidate_supply_correction_source_facts_response, validate_supply_correction_context,
    validate_supply_correction_transition,
};
#[path = "series_supplement.rs"]
mod supplement;
#[path = "series_supply.rs"]
mod supply;
pub(crate) use supplement::legacy_supplement_coverage_request;
pub(crate) use supplement::legacy_supplement_design_request;
pub(crate) use supplement::legacy_supplement_review_request;
pub(crate) use supplement::revalidate_supplement_design_response;
pub(crate) use supplement::revalidate_supplement_key_response;
pub(crate) use supplement::validate_supplement_transition;
pub(crate) use supplement::{
    supplement_identity_reservation, validate_supplement_identity_context,
};
pub(crate) use supply::{
    supplement_design_usable, supplement_supply_card, supplement_supply_catalogue,
    supplement_supply_state,
};
#[path = "series_export.rs"]
pub(crate) mod export;
#[path = "series_package.rs"]
mod package;
#[path = "series_repair.rs"]
mod repair;
#[path = "series_rules.rs"]
mod rules;
pub(crate) use repair::revalidate_review_repair_response;
#[path = "series_frame_render.rs"]
mod frame_render;
pub(crate) use frame_render::validate_projection as validate_frame_render_projection;
#[path = "series_review_scope.rs"]
mod review_scope;
pub(crate) use review_scope::revalidate_review_entry_scope_response;
#[cfg(test)]
#[path = "series_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) use tests::frame_render_tests::series_frame_render_fixture;
#[cfg(test)]
pub(crate) use tests::repair_plan_tests::series_repair_replay_fixture;
#[cfg(test)]
pub(crate) use tests::seed_engine_series_fixture;
#[cfg(test)]
pub(crate) use tests::series_review_entry_replay_fixture;
#[cfg(test)]
pub(crate) use tests::supplement_tests::correction_tests::series_normal_revision_fixture;
#[cfg(test)]
pub(crate) use tests::supplement_tests::correction_tests::series_supply_correction_fact_fixture;
#[cfg(test)]
pub(crate) use tests::supplement_tests::correction_tests::series_supply_correction_fixture;
#[cfg(test)]
pub(crate) use tests::supplement_tests::correction_tests::series_unpublished_revision_fixture;
#[cfg(test)]
pub(crate) use tests::supplement_tests::series_hyphen_supplement_fixture;
#[cfg(test)]
pub(crate) use tests::supplement_tests::series_supplement_fixture;
#[cfg(test)]
pub(crate) use tests::supplement_tests::series_supply_fixture;

pub const PROFILE: &str = "desktop-series672-assets112/v1";
pub const PROFILE_FIELD: &str = "workflowProfile";
pub const ROLE_PACKET: &str = "desktop-series-role-packet/v1";
const COMPACT_DESIGN_INPUT: &str = "series-compact-design-input/v1";
const ABSOLUTE_DIALOGUE_COORDINATES: &str = "source-absolute-utf16/v1";
const SOURCE_FACT_CONTRACT_FIELD: &str = "seriesSourceFactContract";
const SOURCE_FACT_CONTRACT: &str = "source-facts-summary-with-exact-evidence/v1";
const SOURCE_FACT_INSTRUCTIONS: &str = "输出字段语义：sourceFacts.statement是带明确sourceRefs的事实概括，可概括多条已供源行，不要求等于单行原文；不能把制作选择或无源推断写成源事实。refs只可引用当前冻结包实际提供的源行。evidence.quote才是逐字原文摘录，必须对应rowId且完全保留原词。跨集材料只作为事实与设计依据，不自动扩大本集使用范围；新增资产必须另有本集实际用途引文。scene_detail至少包含一个场景；仅可附带该场景完整设计明确依赖的本轮有效修订或已供有效道具，依赖必须绑定正确设计版本，不能任意添加ID或扩大用途。独立审查仍须核概括是否忠于原文。";
const MAX_REPAIRS: usize = 2;
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn hash(v: &Value) -> String {
    digest(&v.to_string())
}
fn delivery_supply_proof(delivery: &Value) -> &Value {
    let receipt = &delivery["adoptionReceipt"];
    if receipt["supplyContractCorrection"]["version"] == supply_correction::VERSION {
        &receipt["supplyContractCorrection"]
    } else {
        &receipt["assetSupplement"]
    }
}
fn visual_style(input: &Value) -> &str {
    super::asset_design_contract::project_visual_style(&input["settings"])
}
fn direction(input: &Value) -> Value {
    let confirmed = super::asset_design_contract::production_direction(input);
    let production = if !confirmed.is_null() {
        confirmed
    } else {
        input
            .pointer("/settings/confirmedDirection")
            .filter(|v| !v.is_null() && v.as_str() != Some(""))
            .cloned()
            .unwrap_or_else(|| input["foundation"]["direction"].clone())
    };
    json!({"productionDirection":production,"visualStyle":visual_style(input),
        "ratio":input.pointer("/settings/ratio").or_else(||input.pointer("/settings/aspectRatio")).cloned().unwrap_or(Value::Null),
        "systemPresentation":input["settings"]["systemPresentation"],
        "systemPresentations":input["settings"]["systemPresentations"]})
}
fn string_list(v: &Value) -> Result<Vec<String>, String> {
    v.as_array()
        .ok_or("SERIES_EXPECTED_ARRAY")?
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "SERIES_EXPECTED_STRING".into())
        })
        .collect()
}
pub fn uses_profile(input: &Value) -> Result<bool, String> {
    match input.get(PROFILE_FIELD) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::String(v)) if v == PROFILE => Ok(true),
        Some(Value::String(v)) if v.starts_with("desktop-series") => {
            Err("SERIES_WORKFLOW_PROFILE_UNKNOWN".into())
        }
        _ => Ok(false),
    }
}
pub fn rule_snapshots() -> Result<Vec<RuleSnapshot>, String> {
    rules::snapshots()
}
pub fn allocate_asset_identities(assets: &mut [Value]) -> Result<(), String> {
    export::allocate_asset_identities(assets)
}
pub(crate) fn allocate_asset_identities_from(
    assets: &mut [Value],
    floor: u64,
) -> Result<u64, String> {
    export::allocate_asset_identities_from(assets, floor)
}
pub(crate) fn selected_episode_ids(input: &Value) -> Result<Vec<String>, String> {
    validate_profile(input)?;
    scope(input, &source(input)?)
}
/// Compare the actual creative inputs, not the workflow ID or expanded season
/// scope. A reused delivery keeps its original roles and adoption provenance.
pub(crate) fn delivery_inputs_reusable(
    current: &Value,
    frozen: &Value,
    delivery: &Value,
    expected_entry: &Value,
) -> Result<bool, String> {
    validate_profile(current)?;
    if frozen[PROFILE_FIELD] != PROFILE
        || current["source"]["sha256"] != frozen["source"]["sha256"]
        || current["seriesRulePackSha256"] != frozen["seriesRulePackSha256"]
        || current["seriesSkillVersion"] != frozen["seriesSkillVersion"]
        || visual_style(current) != visual_style(frozen)
        || fantasy_mode(current)? != fantasy_mode(frozen)?
        || limit(current)? != limit(frozen)?
    {
        return Ok(false);
    }
    validate_delivery(frozen, delivery)?;
    let source = source(current)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| delivery["episodeId"] == e.id)
        .ok_or("SERIES_REUSE_EPISODE_UNKNOWN")?;
    let old_source = self::source(frozen)?;
    let Some(old_episode) = old_source.episodes.iter().find(|e| e.id == episode.id) else {
        return Ok(false);
    };
    let current_base = base_values(current, &source, episode, &[], &Value::Null);
    let old_base = base_values(frozen, &old_source, old_episode, &[], &Value::Null);
    if current_base != old_base {
        return Ok(false);
    }
    let entry = &delivery["bundle"]["entry"];
    if expected_entry["continuityStale"] == true
        || !value_is_projection(&boundary_content(entry), &boundary_content(expected_entry))
    {
        return Ok(false);
    }
    let assets = current["assets"]
        .as_array()
        .ok_or("SERIES_ASSETS_MISSING")?;
    // Missing/stale/changed ancestor design invalidates reuse. New unrelated
    // assets and image-only updates do not invalidate an adopted episode.
    let Ok(mut cards) = design::cards(&source, episode, assets) else {
        return Ok(false);
    };
    if let Some(supplied) = delivery_supply_proof(delivery)["suppliedExistingAssets"].as_array() {
        for old in supplied {
            let Some(asset) = assets.iter().find(|a| a["id"] == old["id"]) else {
                return Ok(false);
            };
            if !design::adopted(asset, &source) {
                return Ok(false);
            }
            let card = supplement_supply_card(asset);
            if let Some(position) = cards.iter().position(|a| a["id"] == old["id"]) {
                cards[position] = card;
            } else {
                cards.push(card);
            }
        }
    }
    let frozen_cards = delivery["bundle"]["assets"]
        .as_array()
        .ok_or("SERIES_ADOPTION_ASSETS_MISSING")?;
    if cards.len() != frozen_cards.len() {
        return Ok(false);
    }
    fn comparison_card(card: &Value) -> Value {
        let mut card = card.clone();
        if let Some(fields) = card.as_object_mut() {
            fields.remove("version");
            // The full adopted design and revision remain compared. This
            // status is redundant and may have been added by persistence.
            if fields
                .get("designStatus")
                .is_none_or(|v| v.is_null() || v == "adopted")
            {
                fields.remove("designStatus");
            }
        }
        card
    }
    Ok(frozen_cards.iter().all(|old| {
        cards
            .iter()
            .any(|new| new["id"] == old["id"] && comparison_card(new) == comparison_card(old))
    }))
}
pub(crate) fn value_is_projection(expected: &Value, actual: &Value) -> bool {
    match expected.as_object() {
        Some(fields) => fields
            .iter()
            .all(|(key, value)| actual.get(key) == Some(value)),
        None => expected == actual,
    }
}
fn boundary_content(value: &Value) -> Value {
    let mut result = value.clone();
    if let Some(fields) = result.as_object_mut() {
        for key in [
            "version",
            "expectedVersion",
            "retired",
            "continuityStale",
            "planningProfile",
            "sourceVersion",
            "foundationVersion",
        ] {
            fields.remove(key);
        }
    }
    result
}
pub fn export_delivery_package(
    input: &Value,
    deliveries: &[Value],
    directory: &std::path::Path,
) -> Result<Value, String> {
    package::write(input, deliveries, directory)
}
pub(crate) fn export_delivery_package_with_inputs(
    input: &Value,
    deliveries: &[Value],
    frozen_inputs: &[Value],
    directory: &std::path::Path,
) -> Result<Value, String> {
    package::write_with_inputs(input, deliveries, frozen_inputs, directory)
}
pub fn mark_input(input: &mut Value) -> Result<(), String> {
    if !input.is_object() {
        return Err("SERIES_INPUT_NOT_OBJECT".into());
    }
    if input
        .get(PROFILE_FIELD)
        .is_some_and(|v| !v.is_null() && v != PROFILE)
    {
        return Err("SERIES_CANNOT_REWRITE_FROZEN_PROFILE".into());
    }
    input[PROFILE_FIELD] = json!(PROFILE);
    input["seriesRulePackSha256"] = json!(rules::pack_hash()?);
    input["seriesSkillVersion"] = json!(rules::VERSION);
    input["seriesTransportVersion"] = json!(COMPACT_DESIGN_INPUT);
    input["seriesDialogueCoordinates"] = json!(ABSOLUTE_DIALOGUE_COORDINATES);
    input["seriesRuleMaterialization"] = json!(rules::CONTENT_ONCE);
    input[SOURCE_FACT_CONTRACT_FIELD] = json!(SOURCE_FACT_CONTRACT);
    input[design_source::FIELD] = json!(design_source::VERSION);
    input[supply_revision::INPUT_FIELD] = json!(supply_revision::VERSION);
    frame_render::mark_input(input)?;
    Ok(())
}
fn validate_profile(input: &Value) -> Result<(), String> {
    if !uses_profile(input)?
        || input["seriesRulePackSha256"] != rules::pack_hash()?
        || input["seriesSkillVersion"] != rules::VERSION
    {
        return Err("SERIES_FROZEN_RULES_MISMATCH".into());
    }
    if input
        .get("seriesTransportVersion")
        .is_some_and(|v| !v.is_null() && v != COMPACT_DESIGN_INPUT)
    {
        return Err("SERIES_TRANSPORT_VERSION_UNKNOWN".into());
    }
    if input
        .get("seriesDialogueCoordinates")
        .is_some_and(|v| !v.is_null() && v != ABSOLUTE_DIALOGUE_COORDINATES)
    {
        return Err("SERIES_DIALOGUE_COORDINATES_UNKNOWN".into());
    }
    if input
        .get(SOURCE_FACT_CONTRACT_FIELD)
        .is_some_and(|v| !v.is_null() && v != SOURCE_FACT_CONTRACT)
    {
        return Err("SERIES_SOURCE_FACT_CONTRACT_UNKNOWN".into());
    }
    validate_rule_materialization(input)?;
    frame_render::validate_profile(input)?;
    for key in [
        "storyboardRulePackProfile",
        "storyboardCreativeProfile",
        "storyboardCarrierProfile",
    ] {
        if input.get(key).is_some_and(|v| !v.is_null()) {
            return Err(format!("SERIES_MIXED_LEGACY_PROFILE:{key}"));
        }
    }
    Ok(())
}
fn limit(input: &Value) -> Result<u64, String> {
    let value = input
        .pointer("/settings/maxUnitSeconds")
        .or_else(|| input.pointer("/productionScope/maxUnitSeconds"))
        .or_else(|| input.pointer("/capabilities/maxDuration"))
        .and_then(Value::as_f64)
        .unwrap_or(30.0);
    if value == 15.0 {
        Ok(15)
    } else if value == 30.0 {
        Ok(30)
    } else {
        Err("SERIES_UNIT_LIMIT_MUST_BE_15_OR_30".into())
    }
}
fn source(input: &Value) -> Result<LockedSource, String> {
    let value: LockedSource = serde_json::from_value(input["source"].clone())
        .map_err(|e| format!("SERIES_SOURCE:{e}"))?;
    super::validate_source(&value)?;
    if value.diagnostics.iter().any(|d| d.blocking) {
        return Err("SERIES_SOURCE_BOUNDARY_UNRESOLVED".into());
    }
    Ok(value)
}
fn scope(input: &Value, source: &LockedSource) -> Result<Vec<String>, String> {
    let declared = &input["productionScope"];
    if !declared.is_object()
        || declared["sourceSha256"] != source.sha256
        || !matches!(s(declared, "status"), "locked" | "LOCKED")
    {
        return Err("SERIES_PRODUCTION_SCOPE_NOT_LOCKED".into());
    }
    let ids = string_list(&declared["episodeIds"])?;
    if ids.is_empty() {
        return Err("SERIES_PRODUCTION_SCOPE_EMPTY".into());
    }
    let known: Vec<_> = source
        .episodes
        .iter()
        .filter(|e| ids.contains(&e.id))
        .map(|e| e.id.clone())
        .collect();
    if known != ids {
        return Err("SERIES_SCOPE_UNKNOWN_DUPLICATE_OR_REORDERED".into());
    }
    let selected = match input
        .pointer("/selection/episodeIds")
        .filter(|v| !v.is_null())
    {
        Some(v) => string_list(v)?,
        None => ids.clone(),
    };
    if selected.is_empty() || selected.iter().any(|id| !ids.contains(id)) {
        return Err("SERIES_SELECTION_OUTSIDE_SCOPE".into());
    }
    let sorted: Vec<_> = ids.into_iter().filter(|id| selected.contains(id)).collect();
    if sorted != selected {
        return Err("SERIES_SELECTION_REORDERED".into());
    }
    Ok(selected)
}
fn fantasy_mode(input: &Value) -> Result<&str, String> {
    let mode = input
        .pointer("/settings/fantasyMode")
        .and_then(Value::as_str)
        .ok_or("SERIES_FANTASY_CHOICE_REQUIRED")?;
    if ["off", "recommend", "all"].contains(&mode) {
        Ok(mode)
    } else {
        Err("SERIES_FANTASY_CHOICE_REQUIRED".into())
    }
}
fn source_view(rows: &[SourceRow]) -> Value {
    json!(rows
        .iter()
        .map(|row| {
            let mut v = json!(row);
            if let Some(q) = v["dialogue"].as_object_mut() {
                q.remove("quote");
                for key in ["spokenSegments", "performanceSegments"] {
                    if let Some(parts) = q.get_mut(key).and_then(Value::as_array_mut) {
                        for p in parts {
                            if let Some(p) = p.as_object_mut() {
                                p.remove("text");
                            }
                        }
                    }
                }
            }
            v
        })
        .collect::<Vec<_>>())
}
/// Content-addressed values occur once; every semantic binding remains present.
fn packet(
    role: &str,
    episode: &str,
    input: &Value,
    values: Vec<(&str, Value)>,
) -> Result<Value, String> {
    let mut blobs = Map::new();
    let mut bindings = Vec::new();
    let mut kinds = HashSet::new();
    for (kind, value) in values {
        if !kinds.insert(kind) {
            return Err("SERIES_DUPLICATE_PACKET_KIND".into());
        }
        let body = json!({"format":"json","value":value});
        let key = hash(&body);
        blobs.entry(key.clone()).or_insert(body);
        bindings.push(json!({"kind":kind,"blob":key}));
    }
    for required in ["source", "direction", "state", "assets", "entry"] {
        if !kinds.contains(required) {
            return Err(format!("SERIES_PACKET_MISSING:{required}"));
        }
    }
    if matches!(role, "finisher" | "reviewer" | "repair") {
        for required in ["draft", "plan", "selection"] {
            if !kinds.contains(required) {
                return Err(format!("SERIES_PACKET_MISSING:{required}"));
            }
        }
    }
    let mut p = json!({"version":ROLE_PACKET,"workflowProfile":PROFILE,"role":role,"episodeId":episode,"sourceSha256":input["source"]["sha256"],"productionScope":input["productionScope"],"rulePackSha256":rules::pack_hash()?,"bindings":bindings,"blobs":blobs});
    p["inputFingerprint"] = json!(hash(&p));
    Ok(p)
}
fn task(role: &str) -> &'static str {
    match role {
        "author"=>"连续完成本集源入口、情绪统筹、已授权幻想选择及完整主创；同一回答交完整plan、selection、draft，不另设策划调用。实际阅读完整必要采用设计，短卡只定位。不得缺源、虚构身份或将纯声源当入画。",
        "finisher"=>"这是独立新上下文的整集整编。完整阅读全源、全plan/selection、全部候选、必要完整设计和实际入口，直接交改好的完整稿。保留有效表演，不为数字或凑整改稿。你参与改稿，不能冒称非作者独审。",
        "reviewer"=>"这是未参与本稿创作或整编的新上下文独审。完整双向读源和稿，核完整设计、时点知情、幻想许可、声窗、真实切点及逐段图片用途，只交具体问题和坐标，不能代写。特别审查不必要长镜，2–5秒是常见节奏而非硬限；长镜需要实际表演理由。程序通过不能代替内容判断。",
        "repair"=>"按给定问题定向修复完整候选，仅改问题单元及明确关联项。保留无关单元、原话、说者、声道、原顺序及采用设计。不得借修复重写全剧。",
        "design"=>"依据本集锁源、已确认方向和正式身份，完成缺失的完整文字设计与短卡：角色识别/服装结构、场景拓扑/通道尺度、道具操作条件及适用时点。不是图片提示词，不写七视图/白底或工具参数；源事实与制作补足分开。依赖只用正式ID和设计版本，未看像素如实说明。",
        "design-review"=>"作为独立设计复核岗位，对照完整源、方向和候选设计核身份/状态/时点/源约束/母设计依赖与可演出完整性。只交具体问题，不改原文，不以短卡或已有图片冒充完整设计。",
        _=>"未知岗位",
    }
}
fn request(
    role: &str,
    stage: &str,
    episode: &str,
    iteration: &str,
    input: &Value,
    values: Vec<(&str, Value)>,
    schema: Value,
    model: &str,
    reasoning: &str,
) -> Result<TextRequest, String> {
    let packet = packet(role, episode, input, values)?;
    let mut context = json!({"workflowProfile":PROFILE,"sourceSha256":input["source"]["sha256"],"episodeId":episode,"executionRoleId":format!("{episode}/{role}/{iteration}"),"freshContext":true,"role":role,"task":task(role),"packet":packet,"fantasyMode":fantasy_mode(input)?,"timing":{"internal":"exact decimal up to 9 places","maxUnitSeconds":limit(input)?,"outer":"max(4,ceil(actual))"},"transportContract":"sourceRows原文及原话区间是唯一来源；dialogueParts只选择lineId、quoteStart/quoteEnd与声窗，Rust物化文字/说者/声道。schema只约定运输字段，不限定艺术镜数、固定镜长、强制幻想比例。imageBindings必须显式列本单元实际使用的图片assetId，声音/仅提及不算图片。"});
    if input["seriesDialogueCoordinates"] == ABSOLUTE_DIALOGUE_COORDINATES {
        context["seriesDialogueCoordinates"] = json!(ABSOLUTE_DIALOGUE_COORDINATES);
        context["transportContract"] = json!(format!("{} quoteStart/quoteEnd是原文全局绝对UTF16半开坐标，直接使用该lineId的spokenSegments.range；不能从句首0开始。分段时在该绝对区间内选择连续子区间，遇到不发声演法须分开，不把演法念出。",s(&context,"transportContract")));
    }
    if input["seriesRuleMaterialization"] == rules::CONTENT_ONCE {
        context["seriesRuleMaterialization"] = json!(rules::CONTENT_ONCE);
    }
    if input[SOURCE_FACT_CONTRACT_FIELD] == SOURCE_FACT_CONTRACT {
        context[SOURCE_FACT_CONTRACT_FIELD] = json!(SOURCE_FACT_CONTRACT);
        context["sourceFactFieldSemantics"] = json!(SOURCE_FACT_INSTRUCTIONS);
    }
    if input[design_source::FIELD] == design_source::VERSION {
        context[design_source::FIELD] = json!(design_source::VERSION);
    }
    if let Some(version) = input.get(supply_revision::INPUT_FIELD) {
        context[supply_revision::INPUT_FIELD] = version.clone();
    }
    if input["seriesTransportVersion"] == COMPACT_DESIGN_INPUT
        && matches!(role, "design" | "design-review")
    {
        context["seriesTransportVersion"] = json!(COMPACT_DESIGN_INPUT);
        context["designReadContract"] = json!("完整设计供料只保存一次：source.sourceRowsBinding 指向本包 designSourceRows（含本集完整源及有据跨集设计行），按 episodeId 区分。designTargets.asset 仅定位，完整身份/范围/已有完整设计必须按 id+variantId 从 assets 读取，不能以定位项代替完整供料。");
    }
    context["seriesRulePackSha256"] = json!(rules::pack_hash()?);
    if let Some(version) = input.get(frame_render::PROFILE_FIELD) {
        context[frame_render::PROFILE_FIELD] = version.clone();
    }
    Ok(TextRequest {
        stage: stage.into(),
        skill: format!("series-{role}"),
        model: model.into(),
        reasoning: reasoning.into(),
        input: context,
        schema,
        rule_snapshot: rules::role(role, fantasy_mode(input)? != "off")?,
    })
}
fn role_receipt(request: &TextRequest, raw: &Value) -> Result<Value, String> {
    Ok(
        json!({"role":request.input["role"],"stage":request.stage,"executionRoleId":request.input["executionRoleId"],"requestSha256":digest(&serde_json::to_string(request).map_err(|e|e.to_string())?),"rawSha256":hash(raw),"packetSha256":hash(&request.input["packet"]),"contextIsolation":"fresh_context_required","request":request,"raw":raw}),
    )
}
pub(crate) fn repair_rules(origin_role: &str, fantasy: bool) -> Result<Value, String> {
    let mut result = rules::role("repair", fantasy)?;
    if origin_role != "repair" {
        result["originRules"] = rules::role(origin_role, fantasy)?;
    }
    Ok(result)
}
fn validate_rule_materialization(input: &Value) -> Result<(), String> {
    if input
        .get("seriesRuleMaterialization")
        .is_some_and(|v| !v.is_null() && v != rules::CONTENT_ONCE)
    {
        return Err("SERIES_RULE_MATERIALIZATION_VERSION".into());
    }
    Ok(())
}
/// Legacy snapshots keep their original byte shape; only a new frozen input
/// marker selects one physical body for repeated repair/origin rule content.
pub(crate) fn repair_rules_for(
    input: &Value,
    origin_role: &str,
    fantasy: bool,
) -> Result<Value, String> {
    validate_rule_materialization(input)?;
    let original = repair_rules(origin_role, fantasy)?;
    if input["seriesRuleMaterialization"] == rules::CONTENT_ONCE
        && original.get("originRules").is_some()
    {
        rules::materialize_repair(&original)
    } else {
        Ok(original)
    }
}
pub(crate) fn restore_repair_rule_snapshot(snapshot: &Value) -> Result<Value, String> {
    rules::restore_repair(snapshot)
}
fn packet_value<'a>(packet: &'a Value, kind: &str) -> Result<&'a Value, String> {
    let binding = packet["bindings"]
        .as_array()
        .ok_or("SERIES_PACKET_BINDINGS")?
        .iter()
        .find(|b| b["kind"] == kind)
        .ok_or_else(|| format!("SERIES_PACKET_MISSING:{kind}"))?;
    packet["blobs"]
        .get(s(binding, "blob"))
        .and_then(|v| v.get("value"))
        .ok_or_else(|| format!("SERIES_PACKET_BLOB_MISSING:{kind}"))
}
fn validate_role_packet(
    req: &TextRequest,
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
) -> Result<(), String> {
    let role = s(&req.input, "role");
    let p = &req.input["packet"];
    let allowed = match req.stage.as_str() {
        "seriesAuthor" => role == "author",
        "seriesFinisher" => role == "finisher",
        "seriesReview" => role == "reviewer",
        "seriesDesign" => role == "design",
        "seriesDesignReview" => role == "design-review",
        "seriesRepair" => matches!(role, "author" | "finisher" | "repair"),
        _ => false,
    };
    if !allowed
        || req.input["episodeId"] != episode.id
        || p["episodeId"] != episode.id
        || req.input["seriesRulePackSha256"] != rules::pack_hash()?
    {
        return Err("SERIES_ROLE_STAGE_IDENTITY".into());
    }
    if req.input["seriesDialogueCoordinates"] != input["seriesDialogueCoordinates"] {
        return Err("SERIES_DIALOGUE_COORDINATES_MISMATCH".into());
    }
    if req.input["seriesRuleMaterialization"] != input["seriesRuleMaterialization"] {
        return Err("SERIES_RULE_MATERIALIZATION_INPUT_MISMATCH".into());
    }
    if req.input[SOURCE_FACT_CONTRACT_FIELD] != input[SOURCE_FACT_CONTRACT_FIELD]
        || (input[SOURCE_FACT_CONTRACT_FIELD] == SOURCE_FACT_CONTRACT
            && req.input["sourceFactFieldSemantics"] != SOURCE_FACT_INSTRUCTIONS)
        || (input[SOURCE_FACT_CONTRACT_FIELD].is_null()
            && req.input.get("sourceFactFieldSemantics").is_some())
    {
        return Err("SERIES_SOURCE_FACT_CONTRACT_MISMATCH".into());
    }
    if req.input[frame_render::PROFILE_FIELD] != input[frame_render::PROFILE_FIELD] {
        return Err("SERIES_FRAME_RENDER_INPUT_MISMATCH".into());
    }
    if req.input[supply_revision::INPUT_FIELD] != input[supply_revision::INPUT_FIELD] {
        return Err("SERIES_SUPPLEMENT_VERSION_INPUT_MISMATCH".into());
    }
    let effective_role = if req.stage == "seriesRepair" {
        "repair"
    } else {
        role
    };
    let expected_rules = if req.stage == "seriesRepair" {
        repair_rules_for(input, role, fantasy_mode(input)? != "off")?
    } else {
        rules::role(effective_role, fantasy_mode(input)? != "off")?
    };
    if req.rule_snapshot != expected_rules {
        return Err("SERIES_ROLE_RULE_CONTENT_MISMATCH".into());
    }
    let mut values = Vec::new();
    for binding in p["bindings"].as_array().ok_or("SERIES_PACKET_BINDINGS")? {
        let key = s(binding, "blob");
        let body = p["blobs"].get(key).ok_or("SERIES_PACKET_BLOB_MISSING")?;
        if key != hash(body) || body["format"] != "json" {
            return Err("SERIES_PACKET_BLOB_HASH".into());
        }
        values.push((s(binding, "kind"), body["value"].clone()));
    }
    if packet(role, &episode.id, input, values)? != *p {
        return Err("SERIES_PACKET_FINGERPRINT_MISMATCH".into());
    }
    // Compare the source, confirmed direction and state with the frozen task,
    // independently of the hashes supplied in the candidate's own receipt.
    let compact_design = (input["seriesTransportVersion"] == COMPACT_DESIGN_INPUT
        || [supply::VERSION, supply_revision::VERSION]
            .iter()
            .any(|v| req.input["seriesAssetSupplement"]["version"] == *v)
        || req.input["seriesSupplyCorrection"]["version"] == supply_correction::VERSION)
        && matches!(role, "design" | "design-review");
    if compact_design && req.input["seriesTransportVersion"] != COMPACT_DESIGN_INPUT {
        return Err("SERIES_TRANSPORT_VERSION_MISMATCH".into());
    }
    if req.input.get("seriesSupplyCorrection").is_none()
        && req.input[design_source::FIELD] != input[design_source::FIELD]
    {
        return Err("SERIES_DESIGN_SOURCE_USE_VERSION_MISMATCH".into());
    }
    if input[design_source::FIELD] == design_source::VERSION
        && matches!(role, "design" | "design-review")
        && req.input.get("seriesAssetSupplement").is_none()
        && req.input.get("seriesSupplyCorrection").is_none()
    {
        let assets = packet_value(p, "assets")?
            .as_array()
            .ok_or("SERIES_ASSETS_MISSING")?;
        let targets = packet_value(p, "designTargets")?
            .as_array()
            .ok_or("SERIES_DESIGN_TARGETS_MISSING")?
            .iter()
            .map(|target| {
                assets
                    .iter()
                    .find(|a| {
                        a["id"] == target["asset"]["id"]
                            && a["variantId"] == target["asset"]["variantId"]
                    })
                    .cloned()
                    .ok_or_else(|| "SERIES_DESIGN_TARGET_UNKNOWN".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if packet_value(p, "designSourceRows")?
            != &source_view(&design_source::rows(input, source, episode, &targets)?)
        {
            return Err("SERIES_DESIGN_SOURCE_USES_CHANGED".into());
        }
    }
    for (kind, expected) in base_values(input, source, episode, &[], &Value::Null) {
        if !matches!(kind, "source" | "direction" | "state") {
            continue;
        }
        let mut actual = packet_value(p, kind)?.clone();
        if compact_design && kind == "source" {
            if actual["sourceRowsBinding"] != "designSourceRows"
                || actual.get("sourceRows").is_some()
            {
                return Err("SERIES_DESIGN_SOURCE_BINDING_INVALID".into());
            }
            let supplied = packet_value(p, "designSourceRows")?
                .as_array()
                .ok_or("SERIES_DESIGN_SOURCE_ROWS_MISSING")?;
            let rows: Vec<_> = source
                .rows
                .iter()
                .filter(|r| r.kind == "body" && supplied.iter().any(|v| v["id"] == r.id))
                .cloned()
                .collect();
            if source_view(&rows) != json!(supplied) {
                return Err("SERIES_DESIGN_SOURCE_ROWS_CHANGED".into());
            }
            let episode_rows: Vec<_> = rows
                .into_iter()
                .filter(|r| r.episode_id.as_deref() == Some(&episode.id))
                .collect();
            actual.as_object_mut().unwrap().remove("sourceRowsBinding");
            actual["sourceRows"] = source_view(&episode_rows);
        }
        if actual != expected {
            return Err(format!("SERIES_PACKET_FROZEN_INPUT_MISMATCH:{kind}"));
        }
    }
    Ok(())
}
async fn call<T>(
    executor: &dyn TextExecutor,
    request: TextRequest,
    validate: impl FnOnce(&Value) -> Result<T, String>,
) -> Result<(T, Value), String> {
    let request = executor.prepare_transport_request(request).await?;
    // UNKNOWN and transport failures deliberately leave this function: only the
    // saved executor may reconcile/recover the same attempt, never this domain.
    let raw = executor.execute(request.clone()).await?;
    match validate(&raw) {
        Ok(value) => {
            executor.accept(&request, &raw).await?;
            Ok((value, role_receipt(&request, &raw)?))
        }
        Err(error) => {
            executor.reject(&request, &raw, &error).await?;
            Err(error)
        }
    }
}
fn base_values(
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    entry: &Value,
) -> Vec<(&'static str, Value)> {
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|r| r.episode_id.as_deref() == Some(&episode.id) && r.kind == "body")
        .cloned()
        .collect();
    vec![
        (
            "source",
            json!({"episode":episode,"scenes":source.scenes.iter().filter(|s|s.episode_id==episode.id).collect::<Vec<_>>(),"sourceRows":source_view(&rows)}),
        ),
        ("direction", direction(input)),
        (
            "state",
            json!(super::storyboard::scoped_facts(input, &rows, &[])),
        ),
        ("assets", json!(assets)),
        ("entry", entry.clone()),
    ]
}
fn candidate_schema(
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    mode: &str,
) -> Value {
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|r| r.episode_id.as_deref() == Some(&episode.id) && r.kind == "body")
        .cloned()
        .collect();
    let scenes: Vec<_> = source
        .scenes
        .iter()
        .filter(|s| s.episode_id == episode.id)
        .cloned()
        .collect();
    let mut draft = super::storyboard::draft_schema(&rows, &[], assets, &[], &episode.id);
    draft["properties"]["units"]["items"]["properties"]["eventId"] = string();
    let image_ids: Vec<_> = assets
        .iter()
        .filter(|a| !export::is_voice_only(a) && a["materialRole"] != "MATERIAL_SOURCE_ONLY")
        .map(|a| a["id"].clone())
        .collect();
    draft["properties"]["units"]["items"]["properties"]["imageBindings"] = if image_ids.is_empty() {
        json!({"type":"array","items":{"type":"string"},"maxItems":0})
    } else {
        json!({"type":"array","items":{"type":"string","enum":image_ids},"maxItems":30,"uniqueItems":true})
    };
    draft["properties"]["units"]["items"]["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("imageBindings"));
    obj(
        json!({"schemaVersion":choices(&["series-episode-candidate/v1"]),"plan":super::storyboard::plan_schema(&rows,&[],&scenes,mode),"selection":array(string()),"draft":draft}),
    )
}
fn validate_candidate(
    raw: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    input: &Value,
) -> Result<Value, String> {
    validate_candidate_with_supply(raw, source, episode, assets, input, None)
}
fn validate_candidate_with_supply(
    raw: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    input: &Value,
    episode_supply: Option<&Value>,
) -> Result<Value, String> {
    if raw["schemaVersion"] != "series-episode-candidate/v1" {
        return Err("SERIES_CANDIDATE_VERSION".into());
    }
    let mut candidate = raw.clone();
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|r| r.episode_id.as_deref() == Some(&episode.id) && r.kind == "body")
        .cloned()
        .collect();
    let scenes: Vec<_> = source
        .scenes
        .iter()
        .filter(|s| s.episode_id == episode.id)
        .cloned()
        .collect();
    super::storyboard::materialize_validated_plan(&mut candidate["plan"], &rows, &[], &scenes)?;
    let choices = string_list(&candidate["selection"])?;
    let mut seen = HashSet::new();
    let proposals = candidate["plan"]["fantasyCandidates"]
        .as_array()
        .ok_or("SERIES_FANTASY_PLAN_MISSING")?;
    let selected: Vec<_> = choices
        .iter()
        .map(|id| {
            if !seen.insert(id) {
                return Err("SERIES_DUPLICATE_FANTASY_SELECTION".into());
            }
            proposals
                .iter()
                .find(|p| p["id"] == *id)
                .cloned()
                .ok_or_else(|| "SERIES_UNKNOWN_FANTASY_SELECTION".into())
        })
        .collect::<Result<_, String>>()?;
    let mode = fantasy_mode(input)?;
    if mode == "off" && (!selected.is_empty() || !proposals.is_empty()) {
        return Err("SERIES_FANTASY_NOT_AUTHORIZED".into());
    }
    if mode == "all" && selected.len() != proposals.len() {
        return Err("SERIES_AUTHORIZED_FANTASY_DROPPED".into());
    }
    if mode == "recommend"
        && (selected.iter().any(|p| p["recommended"] != true)
            || proposals
                .iter()
                .filter(|p| p["recommended"] == true)
                .count()
                != selected.len())
    {
        return Err("SERIES_RECOMMENDED_SELECTION_MISMATCH".into());
    }
    let cap = limit(input)?;
    export::timing(&mut candidate["draft"], cap)?;
    let refs: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
    let style = visual_style(input);
    let scoped_assets = supply::scoped_assets(source, episode, assets, episode_supply)?;
    super::storyboard::validate_authored(
        source,
        &mut candidate["draft"],
        &scoped_assets,
        &json!({"minDuration":0,"maxDuration":cap,"imageLimit":30}),
        &refs,
        style,
        &format!("{}:", episode.id),
    )?;
    // The old validator's float summation is not the new timing authority.
    export::timing(&mut candidate["draft"], cap)?;
    frame_render::materialize(&mut candidate["draft"], input)?;
    super::storyboard::validate_event_bindings(&candidate["draft"], &candidate["plan"])?;
    super::storyboard::validate_fantasy_profile(&candidate["draft"], &selected, false)?;
    export::validate_image_bindings(&candidate["draft"], assets)?;
    if candidate["draft"]["unresolved"]
        .as_array()
        .is_none_or(|v| !v.is_empty())
    {
        return Err("SERIES_CREATIVE_DEPENDENCY_UNRESOLVED".into());
    }
    Ok(candidate)
}
fn review_schema() -> Value {
    obj(
        json!({"schemaVersion":choices(&["series-independent-review/v1"]),"candidateSha256":string(),"reviewedUnitIds":array(string()),"sourceRowIds":array(string()),"conclusion":choices(&["pass","revise"]),"summary":string(),"issues":array(obj(json!({"id":string(),"severity":choices(&["must_fix","advisory"]),"unitIds":array(string()),"sourceRefs":array(string()),"problem":string(),"requestedChange":string()})))}),
    )
}
fn validate_review(
    raw: &Value,
    candidate: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    entry: &Value,
) -> Result<Value, String> {
    if raw["schemaVersion"] != "series-independent-review/v1"
        || raw["candidateSha256"] != hash(candidate)
        || s(raw, "summary").trim().is_empty()
    {
        return Err("SERIES_REVIEW_UNBOUND".into());
    }
    let units: Vec<_> = candidate["draft"]["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| s(u, "id").to_owned())
        .collect();
    if string_list(&raw["reviewedUnitIds"])? != units {
        return Err("SERIES_REVIEW_NOT_WHOLE_EPISODE".into());
    }
    let refs: Vec<_> = source
        .rows
        .iter()
        .filter(|r| r.kind == "body" && r.episode_id.as_deref() == Some(&episode.id))
        .map(|r| r.id.clone())
        .collect();
    if string_list(&raw["sourceRowIds"])? != refs {
        return Err("SERIES_REVIEW_NOT_WHOLE_SOURCE".into());
    }
    let mut ids = HashSet::new();
    let mut must_fix = 0;
    for issue in raw["issues"]
        .as_array()
        .ok_or("SERIES_REVIEW_ISSUES_MISSING")?
    {
        if s(issue, "id").is_empty()
            || !ids.insert(s(issue, "id"))
            || s(issue, "problem").trim().is_empty()
            || s(issue, "requestedChange").trim().is_empty()
        {
            return Err("SERIES_REVIEW_ISSUE_INVALID".into());
        }
        let affected = string_list(&issue["unitIds"])?;
        let issue_refs = string_list(&issue["sourceRefs"])?;
        if affected.is_empty() || affected.iter().any(|id| !units.contains(id)) {
            return Err("SERIES_REVIEW_ISSUE_OUTSIDE_SCOPE".into());
        }
        // Cross-episode continuity is reviewed against the actual frozen entry,
        // not arbitrary season rows. This is read-only evidence permission:
        // affected units and whole-source coverage remain the current episode.
        if issue_refs.iter().any(|r| !refs.contains(r)) {
            let entry_refs = review_scope::entry_source_refs(source, episode, entry)?;
            if issue_refs
                .iter()
                .any(|r| !refs.contains(r) && !entry_refs.contains(r))
            {
                return Err("SERIES_REVIEW_ISSUE_OUTSIDE_SCOPE".into());
            }
        }
        match s(issue, "severity") {
            "must_fix" => must_fix += 1,
            "advisory" => {}
            _ => return Err("SERIES_REVIEW_SEVERITY".into()),
        }
    }
    if (s(raw, "conclusion") == "pass") != (must_fix == 0)
        || !["pass", "revise"].contains(&s(raw, "conclusion"))
    {
        return Err("SERIES_REVIEW_CONCLUSION_MISMATCH".into());
    }
    Ok(raw.clone())
}
fn assert_local_repair(before: &Value, after: &Value, issues: &Value) -> Result<(), String> {
    let allowed: HashSet<String> = issues
        .as_array()
        .ok_or("SERIES_REPAIR_ISSUES")?
        .iter()
        .filter(|i| i["severity"] == "must_fix")
        .flat_map(|i| {
            i["unitIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    if before["plan"] != after["plan"] || before["selection"] != after["selection"] {
        return Err("SERIES_REPAIR_PLAN_CHANGED_WITHOUT_REVIEW".into());
    }
    for u in before["draft"]["units"].as_array().unwrap() {
        if !allowed.contains(s(u, "id"))
            && after["draft"]["units"]
                .as_array()
                .unwrap()
                .iter()
                .find(|next| next["id"] == u["id"])
                != Some(u)
        {
            return Err("SERIES_REPAIR_CHANGED_UNAFFECTED_UNIT".into());
        }
    }
    Ok(())
}
// Diagnostics quote only the canonical source's coordinates. They never shift
// the received draft, select a guessed interval, or authorize source mutation.
fn source_coordinate_diagnostics(
    source: &LockedSource,
    episode: &SourceEpisode,
    raw: &Value,
) -> Value {
    let used: HashSet<_> = raw["draft"]["units"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|unit| unit["shots"].as_array().into_iter().flatten())
        .flat_map(|shot| shot["dialogueParts"].as_array().into_iter().flatten())
        .filter_map(|part| part["lineId"].as_str())
        .collect();
    let ranges: Vec<_> = source.rows.iter()
        .filter(|row| row.episode_id.as_deref() == Some(&episode.id))
        .filter_map(|row| row.dialogue.as_ref())
        .filter(|quote| used.contains(quote.id.as_str()))
        .map(|quote| {
            let mut segments: Vec<_> = quote.spoken_segments.iter()
                .map(|segment| json!({"quoteStart":segment.range.start,"quoteEnd":segment.range.end}))
                .collect();
            if segments.is_empty() && quote.performance_segments.is_empty() && !quote.quote.is_empty() {
                segments.push(json!({"quoteStart":quote.range.start,"quoteEnd":quote.range.end}));
            }
            json!({"lineId":quote.id,"allowedAbsoluteRanges":segments})
        }).collect();
    json!({"version":"source-absolute-utf16-diagnostics/v1","sourceSha256":source.sha256,
        "intervals":"absolute UTF16 half-open; retain source order and cover every spoken segment once",
        "lines":ranges,"originalRawChanged":false})
}
async fn create_candidate(
    executor: &dyn TextExecutor,
    initial: TextRequest,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    input: &Value,
    receipts: &mut Vec<Value>,
    repair_scope: Option<(&Value, &Value)>,
    mut unadopted_author: Option<&mut Option<Value>>,
    episode_supply: Option<&Value>,
) -> Result<Value, String> {
    let origin_hash = digest(&serde_json::to_string(&initial).map_err(|e| e.to_string())?);
    let mut request = initial.clone();
    let mut last_problem = String::new();
    for attempt in 0..=MAX_REPAIRS {
        let prepared = executor.prepare_transport_request(request.clone()).await?;
        let raw = executor.execute(prepared.clone()).await?;
        let checked = repair::validate_scoped_candidate(
            &prepared,
            &raw,
            source,
            episode,
            assets,
            input,
            repair_scope,
            episode_supply,
        );
        match checked {
            Ok(value) => {
                executor.accept(&prepared, &raw).await?;
                receipts.push(role_receipt(&prepared, &raw)?);
                return Ok(value);
            }
            Err(problem) => {
                executor.reject(&prepared, &raw, &problem).await?;
                if attempt == 0 && supplement::eligible(&prepared, &raw) {
                    if let Some(target) = unadopted_author.as_deref_mut() {
                        let mut evidence = role_receipt(&prepared, &raw)?;
                        evidence["validation"] = json!("rejected");
                        evidence["reason"] = json!(problem);
                        *target = Some(evidence);
                        return Ok(raw);
                    }
                }
                if attempt == MAX_REPAIRS || problem == last_problem {
                    return Err(format!("SERIES_REPAIR_HELD:{problem}"));
                }
                last_problem = problem.clone();
                request = initial.clone();
                request.stage = "seriesRepair".into();
                request.skill = "series-repair".into();
                request.input["executionRoleId"] = json!(format!(
                    "{}/validation-repair/{}",
                    s(&initial.input, "executionRoleId"),
                    attempt + 1
                ));
                request.input["task"]=json!("修正这份原回包中程序已定位的具体领域错误，保留原文、原规则、有效设计与未受影响内容。不要重新开工或重做全剧。quoteStart/quoteEnd必须为sourceRows中spokenSegments.range的原文绝对UTF16半开坐标，不是句内从0计数；一个发声片段不得跨过不发声演法。可按sourceCoordinateDiagnostics核对。声窗为文字编排估算，不要求先生成配音；真实缺失的资产设计仍须如实保留待补，不能删除源动作或伪造资产ID。仍输出当前完整候选schema。");
                request.input["validationRepair"] = json!({"version":"series-domain-repair/v1","attempt":attempt+1,"maximumAttempts":MAX_REPAIRS,"originRequestSha256":origin_hash,"previousRequestSha256":digest(&serde_json::to_string(&prepared).map_err(|e|e.to_string())?),"previousRawSha256":hash(&raw),"problem":problem,"candidate":raw});
                request.input["validationRepair"]["sourceCoordinateDiagnostics"] =
                    source_coordinate_diagnostics(source, episode, &raw);
                request.rule_snapshot = repair_rules_for(
                    input,
                    s(&initial.input, "role"),
                    fantasy_mode(input)? != "off",
                )?;
            }
        }
    }
    Err("SERIES_REPAIR_LIMIT".into())
}
async fn episode(
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &mut Vec<Value>,
    entry: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    let (mut design_adoptions, design_receipts) = design::ensure(
        input, source, episode, assets, entry, executor, model, reasoning,
    )
    .await?;
    let mut cards = design::cards(source, episode, assets)?;
    let mut base = base_values(input, source, episode, &cards, entry);
    let mut schema = candidate_schema(source, episode, &cards, fantasy_mode(input)?);
    let mut receipts = design_receipts;
    let author = request(
        "author",
        "seriesAuthor",
        &episode.id,
        "0",
        input,
        base.clone(),
        schema.clone(),
        model,
        reasoning,
    )?;
    let mut unadopted_author = None;
    let authored = create_candidate(
        executor,
        author,
        source,
        episode,
        &cards,
        input,
        &mut receipts,
        None,
        Some(&mut unadopted_author),
        None,
    )
    .await?;
    let mut finish_values = base.clone();
    finish_values.extend([
        ("plan", authored["plan"].clone()),
        ("selection", authored["selection"].clone()),
        ("draft", authored["draft"].clone()),
    ]);
    let mut supplement_proof = Value::Null;
    let finisher = if let Some(origin) = unadopted_author {
        let completed = supplement::run(
            input, source, episode, assets, &origin, executor, model, reasoning,
        )
        .await?;
        cards = completed.cards;
        base = base_values(input, source, episode, &cards, entry);
        schema = candidate_schema(source, episode, &cards, fantasy_mode(input)?);
        design_adoptions.extend(completed.adoptions);
        receipts.extend(completed.receipts);
        supplement_proof = completed.proof;
        completed.finisher
    } else {
        request(
            "finisher",
            "seriesFinisher",
            &episode.id,
            "0",
            input,
            finish_values,
            schema.clone(),
            model,
            reasoning,
        )?
    };
    let mut candidate = create_candidate(
        executor,
        finisher,
        source,
        episode,
        &cards,
        input,
        &mut receipts,
        None,
        None,
        supplement_proof.get("episodeSupply"),
    )
    .await?;
    let mut review = Value::Null;
    let mut last_issues = String::new();
    for iteration in 0..=MAX_REPAIRS {
        let mut values = base.clone();
        values.extend([
            ("plan", candidate["plan"].clone()),
            ("selection", candidate["selection"].clone()),
            ("draft", candidate["draft"].clone()),
        ]);
        let mut req = request(
            "reviewer",
            "seriesReview",
            &episode.id,
            &iteration.to_string(),
            input,
            values,
            review_schema(),
            model,
            reasoning,
        )?;
        req.input["candidateSha256"] = json!(hash(&candidate));
        let (result, receipt) = call(executor, req, |raw| {
            validate_review(raw, &candidate, source, episode, entry)
        })
        .await?;
        receipts.push(receipt.clone());
        review = result;
        if review["conclusion"] == "pass" {
            break;
        }
        let issues_hash = hash(&review["issues"]);
        if iteration == MAX_REPAIRS || last_issues == issues_hash {
            return Err("SERIES_INDEPENDENT_REVIEW_REPAIR_HELD".into());
        }
        last_issues = issues_hash;
        let mut values = base.clone();
        values.extend([
            ("plan", candidate["plan"].clone()),
            ("selection", candidate["selection"].clone()),
            ("draft", candidate["draft"].clone()),
            ("issues", review["issues"].clone()),
        ]);
        let repair = request(
            "repair",
            "seriesRepair",
            &episode.id,
            &format!("review-{}", iteration + 1),
            input,
            values,
            schema.clone(),
            model,
            reasoning,
        )?;
        let projected = frame_render::project(&repair, input, &receipt)?;
        let repair = executor
            .prepare_series_frame_render(repair, projected, receipt)
            .await?;
        let next = create_candidate(
            executor,
            repair,
            source,
            episode,
            &cards,
            input,
            &mut receipts,
            Some((&candidate, &review["issues"])),
            None,
            supplement_proof.get("episodeSupply"),
        )
        .await?;
        candidate = next;
    }
    let style = visual_style(input);
    let delivery = export::export(source, episode, &candidate["draft"], &cards, style)?;
    let bundle = json!({"schemaVersion":"desktop-series-episode/v1","workflowProfile":PROFILE,"sourceSha256":source.sha256,"episodeId":episode.id,"productionScope":input["productionScope"],"plan":candidate["plan"],"selection":candidate["selection"],"draft":candidate["draft"],"assets":cards,"entry":entry});
    let review_receipt = receipts
        .iter()
        .rev()
        .find(|r| r["role"] == "reviewer")
        .cloned()
        .ok_or("SERIES_REVIEW_RECEIPT_MISSING")?;
    let used: Vec<_> = export::validate_image_bindings(&candidate["draft"], &cards)?
        .into_iter()
        .collect();
    let mut used = used;
    used.sort();
    let adoption = json!({"version":"desktop-series-adoption/v1","workflowProfile":PROFILE,"sourceSha256":source.sha256,"bundleSha256":hash(&bundle),"markdownSha256":delivery["markdownSha256"],"assetIndexSha256":hash(&delivery["assetIndex"]),"rulePackSha256":rules::pack_hash()?,"roleReceipts":receipts,"review":{"candidateSha256":hash(&candidate),"requestSha256":review_receipt["requestSha256"],"resultSha256":review_receipt["rawSha256"],"status":"passed"},"steps":{"sync":{"usedAssetIds":used,"designsSha256":hash(&json!(cards))},"check":{"sourceSha256":source.sha256,"candidateSha256":hash(&candidate)},"export":{"markdownSha256":delivery["markdownSha256"],"assetIndexSha256":hash(&delivery["assetIndex"])},"verify":{"parserVersion":crate::lingmo_parser::PARSER_VERSION,"status":"passed"}},"evidenceBoundary":"local contracts and independent model review; not platform import or media acceptance"});
    let mut adoption = adoption;
    if !supplement_proof.is_null() {
        let field = if supplement_proof["version"] == supply_correction::VERSION {
            "supplyContractCorrection"
        } else {
            "assetSupplement"
        };
        adoption[field] = supplement_proof;
    }
    let result = json!({"episodeId":episode.id,"markdown":delivery["markdown"],"markdownSha256":delivery["markdownSha256"],"assetIndex":delivery["assetIndex"],"bundle":bundle,"adoptionReceipt":adoption,"designAdoptions":design_adoptions,"review":review});
    validate_delivery(input, &result)?;
    Ok(result)
}
/// Used by Stage 4 only after reopening a durable publication. The returned
/// scope is an ephemeral projection backed by the complete adoption proof.
pub(crate) fn validated_supplied_scope_assets(
    input: &Value,
    delivery: &Value,
) -> Result<Vec<Value>, String> {
    validate_delivery(input, delivery)?;
    let receipt = &delivery["adoptionReceipt"];
    let proof = if receipt["supplyContractCorrection"]["version"] == supply_correction::VERSION {
        &receipt["supplyContractCorrection"]
    } else {
        &receipt["assetSupplement"]
    };
    if proof.get("episodeSupply").is_none() {
        return Ok(vec![]);
    }
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| delivery["episodeId"] == e.id)
        .ok_or("SERIES_SUPPLY_SCOPE_EPISODE_MISMATCH")?;
    let assets = proof["suppliedExistingAssets"]
        .as_array()
        .ok_or("SERIES_SUPPLY_PROOF_MISSING")?;
    supply::scoped_assets(&source, episode, assets, proof.get("episodeSupply"))
}
pub fn validate_delivery(input: &Value, delivery: &Value) -> Result<(), String> {
    validate_profile(input)?;
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| delivery["episodeId"] == e.id)
        .ok_or("SERIES_DELIVERY_EPISODE_UNKNOWN")?;
    if !scope(input, &source)?.contains(&episode.id) {
        return Err("SERIES_DELIVERY_OUTSIDE_SCOPE".into());
    }
    let bundle = &delivery["bundle"];
    let receipt = &delivery["adoptionReceipt"];
    if bundle["workflowProfile"] != PROFILE
        || bundle["sourceSha256"] != source.sha256
        || bundle["episodeId"] != episode.id
        || bundle["productionScope"] != input["productionScope"]
        || receipt["version"] != "desktop-series-adoption/v1"
        || receipt["sourceSha256"] != source.sha256
        || receipt["bundleSha256"] != hash(bundle)
        || receipt["rulePackSha256"] != rules::pack_hash()?
        || receipt["markdownSha256"] != digest(s(delivery, "markdown"))
        || delivery["markdownSha256"] != receipt["markdownSha256"]
        || receipt["assetIndexSha256"] != hash(&delivery["assetIndex"])
    {
        return Err("SERIES_ADOPTION_IDENTITY_MISMATCH".into());
    }
    let assets = bundle["assets"]
        .as_array()
        .ok_or("SERIES_ADOPTION_ASSETS_MISSING")?;
    let candidate = json!({"schemaVersion":"series-episode-candidate/v1","plan":bundle["plan"],"selection":bundle["selection"],"draft":bundle["draft"]});
    // Never trust a model-declared scope before its full supplement chain,
    // source evidence, independent review and immutable cards are verified.
    let corrected_author = supply_correction::validate_delivery(input, delivery)?;
    let supplemented_author = supplement::validate_delivery(input, delivery)? || corrected_author;
    let episode_supply = if corrected_author {
        receipt["supplyContractCorrection"].get("episodeSupply")
    } else {
        ([supply::VERSION, supply_revision::VERSION]
            .iter()
            .any(|v| receipt["assetSupplement"]["version"] == *v))
        .then(|| receipt["assetSupplement"].get("episodeSupply"))
        .flatten()
    };
    let checked = validate_candidate_with_supply(
        &candidate,
        &source,
        episode,
        assets,
        input,
        episode_supply,
    )?;
    if checked != candidate {
        return Err("SERIES_ADOPTION_NOT_CANONICAL".into());
    }
    export::verify(
        s(delivery, "markdown"),
        &delivery["assetIndex"],
        &bundle["draft"],
    )?;
    let expected_export = export::export(
        &source,
        episode,
        &bundle["draft"],
        assets,
        visual_style(input),
    )?;
    if expected_export["markdown"] != delivery["markdown"]
        || expected_export["assetIndex"] != delivery["assetIndex"]
    {
        return Err("SERIES_EXPORT_NOT_ADOPTED_BUNDLE".into());
    }
    let roles = receipt["roleReceipts"]
        .as_array()
        .ok_or("SERIES_ADOPTION_ROLE_RECEIPTS_MISSING")?;
    frame_render::validate_adoption(input, delivery)?;
    let mut identities = HashSet::new();
    let mut found = HashSet::new();
    let mut review = None;
    for r in roles {
        let req: TextRequest = serde_json::from_value(r["request"].clone())
            .map_err(|_| "SERIES_RECEIPT_REQUEST_INVALID")?;
        if r["requestSha256"] != digest(&serde_json::to_string(&req).map_err(|e| e.to_string())?)
            || r["rawSha256"] != hash(&r["raw"])
            || r["role"] != req.input["role"]
            || r["stage"] != req.stage
            || r["executionRoleId"] != req.input["executionRoleId"]
            || s(r, "executionRoleId").is_empty()
            || !identities.insert(s(r, "executionRoleId").to_owned())
            || req.input["workflowProfile"] != PROFILE
            || req.input["freshContext"] != true
            || req.input["sourceSha256"] != source.sha256
            || r["packetSha256"] != hash(&req.input["packet"])
        {
            return Err("SERIES_ROLE_RECEIPT_MISMATCH".into());
        }
        validate_role_packet(&req, input, &source, episode)?;
        if matches!(s(r, "role"), "author" | "finisher" | "reviewer" | "repair")
            && (packet_value(&req.input["packet"], "assets")? != &json!(assets)
                || packet_value(&req.input["packet"], "entry")? != &bundle["entry"])
        {
            return Err("SERIES_ROLE_ADOPTED_INPUT_MISMATCH".into());
        }
        found.insert(s(r, "role"));
        if r["role"] == "reviewer" {
            review = Some(r);
        }
    }
    if supplemented_author {
        if found.contains("author") {
            return Err("SERIES_UNADOPTED_AUTHOR_PROMOTED".into());
        }
        found.insert("author");
    }
    if !["author", "finisher", "reviewer"]
        .iter()
        .all(|r| found.contains(r))
    {
        return Err("SERIES_REQUIRED_ROLES_MISSING".into());
    }
    let review = review.ok_or("SERIES_REVIEW_RECEIPT_MISSING")?;
    let result = validate_review(
        &review["raw"],
        &candidate,
        &source,
        episode,
        &bundle["entry"],
    )?;
    if result["conclusion"] != "pass"
        || delivery["review"] != result
        || receipt["review"]["status"] != "passed"
        || receipt["review"]["candidateSha256"] != hash(&candidate)
        || receipt["review"]["requestSha256"] != review["requestSha256"]
        || receipt["review"]["resultSha256"] != review["rawSha256"]
    {
        return Err("SERIES_REVIEW_NOT_CURRENT_ADOPTED_DRAFT".into());
    }
    let review_request: TextRequest = serde_json::from_value(review["request"].clone())
        .map_err(|_| "SERIES_REVIEW_REQUEST_INVALID")?;
    for key in ["plan", "selection", "draft"] {
        if packet_value(&review_request.input["packet"], key)? != &candidate[key] {
            return Err("SERIES_REVIEW_DID_NOT_RECEIVE_ADOPTED_CANDIDATE".into());
        }
    }
    let mut used: Vec<_> = export::validate_image_bindings(&candidate["draft"], assets)?
        .into_iter()
        .collect();
    used.sort();
    let expected = json!({"sync":{"usedAssetIds":used,"designsSha256":hash(&json!(assets))},"check":{"sourceSha256":source.sha256,"candidateSha256":hash(&candidate)},"export":{"markdownSha256":delivery["markdownSha256"],"assetIndexSha256":hash(&delivery["assetIndex"])},"verify":{"parserVersion":crate::lingmo_parser::PARSER_VERSION,"status":"passed"}});
    if receipt["steps"] != expected {
        return Err("SERIES_ADOPTION_STEPS_NOT_SAME_VERSION".into());
    }
    Ok(())
}
pub fn validate_episode_delivery(input: &Value, output: &Value) -> Result<(), String> {
    let deliveries = output["seriesDeliveries"]
        .as_array()
        .filter(|v| v.len() == 1)
        .ok_or("SERIES_PUBLISH_REQUIRES_ONE_EPISODE")?;
    validate_delivery(input, &deliveries[0])?;
    if output["units"] != deliveries[0]["bundle"]["draft"]["units"]
        || output["episodeId"] != deliveries[0]["episodeId"]
    {
        return Err("SERIES_PUBLISHED_UNITS_MISMATCH".into());
    }
    Ok(())
}
pub async fn run(
    input: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    validate_profile(input)?;
    if model.trim().is_empty()
        || !["low", "medium", "high", "xhigh", "max", "ultra"].contains(&reasoning)
    {
        return Err("SERIES_MODEL_SETTINGS_INVALID".into());
    }
    let source = source(input)?;
    let selected = scope(input, &source)?;
    fantasy_mode(input)?;
    limit(input)?;
    if input
        .pointer("/selection/unitIds")
        .and_then(Value::as_array)
        .is_some_and(|v| !v.is_empty())
    {
        return Err("SERIES_PARTIAL_REVISION_REQUIRES_VERSIONED_ISSUE_SCOPE".into());
    }
    let mut assets = input["assets"]
        .as_array()
        .ok_or("SERIES_ASSETS_MISSING")?
        .clone();
    allocate_asset_identities(&mut assets)?;
    let mut deliveries = Vec::new();
    let mut units = Vec::new();
    let mut plans = Vec::new();
    let mut adoptions = Vec::new();
    let mut blocked = Vec::new();
    let mut previous = input
        .get("previousBoundary")
        .cloned()
        .unwrap_or(Value::Null);
    let mut unavailable: Option<String> = None;
    for episode in source.episodes.iter().filter(|e| selected.contains(&e.id)) {
        if let Some(prior) = &unavailable {
            blocked.push(json!({"episodeId":episode.id,"status":"waiting_dependency","dependsOn":prior,"reason":"前集真实尾态尚未完成，保留原任务与已完成结果。"}));
            continue;
        }
        match episode_run(
            input,
            &source,
            episode,
            &mut assets,
            &previous,
            executor,
            model,
            reasoning,
        )
        .await
        {
            Ok(delivery) => {
                let draft = &delivery["bundle"]["draft"];
                previous = draft["units"]
                    .as_array()
                    .and_then(|v| v.last())
                    .cloned()
                    .unwrap_or(Value::Null);
                let publication = json!({"schemaVersion":"complete-episode-delivery/v1","workflowProfile":PROFILE,"status":"running","scope":"episodes","sourceSha256":source.sha256,"episodeId":episode.id,"updatedEpisodeIds":[episode.id],"replacedUnitIds":[],"units":draft["units"],"plans":[{"episodeId":episode.id,"plan":delivery["bundle"]["plan"]}],"fantasyRealization":draft["fantasyRealization"],"revisionNotes":draft["revisionNotes"],"unresolved":[],"blocked":[],"seriesDeliveries":[delivery],"designAdoptions":delivery["designAdoptions"],"ruleSnapshots":rule_snapshots()?});
                validate_episode_delivery(input, &publication)?;
                executor.publish_episode(&publication, input).await?;
                units.extend(draft["units"].as_array().unwrap().iter().cloned());
                plans.push(json!({"episodeId":episode.id,"plan":delivery["bundle"]["plan"]}));
                adoptions.extend(
                    delivery["designAdoptions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
                deliveries.push(delivery);
            }
            Err(reason) => {
                blocked.push(json!({"episodeId":episode.id,"status":"blocked","reason":reason}));
                unavailable = Some(episode.id.clone());
            }
        }
    }
    let updated: Vec<_> = deliveries.iter().map(|d| d["episodeId"].clone()).collect();
    Ok(
        json!({"schemaVersion":"storyboard-delivery/v1","workflowProfile":PROFILE,"sourceSha256":source.sha256,"status":if blocked.is_empty(){"completed"}else{"partially_blocked"},"scope":"episodes","updatedEpisodeIds":updated,"replacedUnitIds":[],"units":units,"plans":plans,"waitingDecision":null,"waitingDecisions":[],"fantasyRealization":[],"revisionNotes":[],"unresolved":[],"blocked":blocked,"seriesDeliveries":deliveries,"designAdoptions":adoptions,"ruleSnapshots":rule_snapshots()?}),
    )
}
// Keep the noun `episode` free for the source object at call sites.
async fn episode_run(
    input: &Value,
    source: &LockedSource,
    ep: &SourceEpisode,
    assets: &mut Vec<Value>,
    entry: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    episode(input, source, ep, assets, entry, executor, model, reasoning).await
}
