//! A user-selected single-call director operation with immutable candidates.
//! The frozen skill is Mac's original; the schema/lifecycle below are the
//! explicit Rust compatibility implementation, not a claimed Mac source port.
use super::storyboard::{self, asset_cards, creative};
use super::{digest, obj, stage3_pack, LockedSource, SourceRow, TextExecutor, TextRequest};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const CANDIDATE_VERSION: &str = "storyboard-refinement-candidate/v1";
pub const FEEDBACK_VERSION: &str = "director-refinement-feedback/rust-v1";

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn ids(value: &Value, key: &str) -> Result<Vec<String>, String> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("精修缺少{key}数组"))?
        .iter()
        .map(|id| {
            id.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("精修{key}身份不是字符串"))
        })
        .collect()
}
fn unique_ids(value: &Value, key: &str) -> Result<HashSet<String>, String> {
    let values = ids(value, key)?;
    let unique: HashSet<_> = values.iter().cloned().collect();
    if values.len() != unique.len() || unique.iter().any(|id| id.is_empty()) {
        return Err(format!("精修{key}身份为空或重复"));
    }
    Ok(unique)
}

struct Basis {
    source: LockedSource,
    rows: Vec<SourceRow>,
    context_rows: Vec<SourceRow>,
    assets: Vec<Value>,
    context: Value,
    draft: Value,
    allowed: HashSet<String>,
    scope: Value,
    input_binding: String,
}

fn basis(input: &Value) -> Result<Basis, String> {
    if !stage3_pack::uses_profile(input)? {
        return Err("导演精修尚未绑定冻结Mac生产规则适配；未调用模型".into());
    }
    let mut checked = json!({});
    stage3_pack::attach_profile(&mut checked, input)?;
    let source: LockedSource =
        serde_json::from_value(input["source"].clone()).map_err(|e| e.to_string())?;
    let source = super::upgrade_source_dialogues(&source)?;
    super::validate_source(&source)?;
    if source.diagnostics.iter().any(|item| item.blocking) {
        return Err("精修原文仍有阻塞诊断".into());
    }
    let episode = text(input, "episodeId");
    if !source.episodes.iter().any(|item| item.id == episode) {
        return Err("精修集身份不存在".into());
    }
    let mut units: Vec<_> = input["existingUnits"]
        .as_array()
        .ok_or("精修缺少完整已采用集稿")?
        .iter()
        .filter(|unit| text(unit, "episodeId") == episode && unit["retired"] != true)
        .cloned()
        .collect();
    units.sort_by_key(|unit| storyboard::unit_source_position(&source, unit));
    if units.is_empty() {
        return Err("精修必须先有实际采用的完整集稿".into());
    }
    let context = storyboard::accepted_revision_context(&source, input, episode, &units)?;
    if context["unitBindings"].as_array().unwrap().len() != units.len() {
        return Err("精修来源只绑定部分集稿；必须读取完整已采用集稿，不扩大或猜测原稿".into());
    }
    let draft = json!({"schemaVersion":"storyboard-draft/v2","units":units,"fantasyRealization":context["fantasyRealization"],"revisionNotes":[],"unresolved":[]});
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|row| row.kind == "body" && row.episode_id.as_deref() == Some(episode))
        .cloned()
        .collect();
    let refs: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
    let assets = storyboard::episode_assets(
        input["assets"].as_array().ok_or("精修缺少冻结资产目录")?,
        episode,
    );
    storyboard::validate_storyboard_scope(
        &source,
        &draft,
        &assets,
        &input["capabilities"],
        Some(&refs),
    )?;
    let fingerprints = context["plan"]["sourceFingerprints"]
        .as_object()
        .ok_or("精修原规划缺少源指纹")?;
    let context_rows: Vec<_> = source
        .rows
        .iter()
        .filter(|row| !refs.contains(&row.id) && fingerprints.contains_key(&row.id))
        .cloned()
        .collect();
    let all_ids: HashSet<_> = units
        .iter()
        .map(|unit| text(unit, "id").to_string())
        .collect();
    let all_events: HashSet<_> = units
        .iter()
        .map(|unit| text(unit, "eventId").to_string())
        .collect();
    let requested = &input["refinementScope"];
    let chosen_units = if requested.get("unitIds").is_some() {
        Some(unique_ids(requested, "unitIds")?)
    } else {
        None
    };
    let chosen_events = if requested.get("eventIds").is_some() {
        Some(unique_ids(requested, "eventIds")?)
    } else {
        None
    };
    if chosen_units
        .as_ref()
        .is_some_and(|chosen| chosen.is_empty() || !chosen.is_subset(&all_ids))
        || chosen_events
            .as_ref()
            .is_some_and(|chosen| chosen.is_empty() || !chosen.is_subset(&all_events))
    {
        return Err("精修许可范围为空、未知或越过实际采用稿".into());
    }
    let allowed: HashSet<_> = units
        .iter()
        .filter(|unit| {
            chosen_units
                .as_ref()
                .is_none_or(|set| set.contains(text(unit, "id")))
                && chosen_events
                    .as_ref()
                    .is_none_or(|set| set.contains(text(unit, "eventId")))
        })
        .map(|unit| text(unit, "id").to_string())
        .collect();
    if allowed.is_empty() {
        return Err("精修许可事件与单元没有共同范围".into());
    }
    let ordered: Vec<_> = units
        .iter()
        .filter(|unit| allowed.contains(text(unit, "id")))
        .map(|unit| unit["id"].clone())
        .collect();
    let events: HashSet<_> = units
        .iter()
        .filter(|unit| allowed.contains(text(unit, "id")))
        .map(|unit| text(unit, "eventId").to_string())
        .collect();
    let mut events: Vec<_> = events.into_iter().collect();
    events.sort();
    let scope = json!({"version":"director-refinement-scope/v1","reviewScope":"complete_adopted_episode","unitIds":ordered,"eventIds":events});
    let mut binding = json!({"sourceSha256":source.sha256,"episodeId":episode,"baseDraftHash":storyboard::finish_base_hash(&draft),
        "acceptedContext":context,"assets":input["assets"],"settings":input["settings"],"capabilities":input["capabilities"],
        "productionDirection":input["productionDirection"],"refinementScope":scope,"reviewNotes":input["reviewNotes"],"preserveNotes":input["preserveNotes"],
        "rulePackSha256":stage3_pack::pack_sha256()?});
    if creative::uses_scene_profile(input)? {
        binding[creative::SCENE_PROFILE_FIELD] = input[creative::SCENE_PROFILE_FIELD].clone();
    }
    super::storyboard_sound_events::inherit(&mut binding, input)?;
    super::asset_use_binding::inherit(&mut binding, input)?;
    let input_binding = digest(&binding.to_string());
    Ok(Basis {
        source,
        rows,
        context_rows,
        assets,
        context,
        draft,
        allowed,
        scope,
        input_binding,
    })
}

fn refinement_schema(basis: &Basis) -> Value {
    let episode = text(&basis.context, "episodeId");
    let events: Vec<_> = basis.context["plan"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| text(event, "id").to_string())
        .collect();
    let full = storyboard::draft_schema(
        &basis.rows,
        &basis.context_rows,
        &basis.assets,
        &events,
        episode,
    );
    let mut schema = storyboard::finish_delta_schema(&full, &basis.draft);
    let old_ids: Vec<_> = basis.draft["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| unit["id"].clone())
        .collect();
    schema["properties"]["reviewDecisions"] = json!({"type":"array","items":obj(json!({
        "state":{"type":"string","enum":["changed","unresolved"]},"coordinate":{"type":"string"},
        "unitIds":{"type":"array","items":{"type":"string","enum":old_ids},"minItems":1},
        "reason":{"type":"string"},"preservedFunction":{"type":"string"}
    }))});
    schema["properties"]["productionFeedback"] = json!({"type":"array","items":obj(json!({
        "coordinate":{"type":"string"},"problem":{"type":"string"},"suggestedAction":{"type":"string"},
        "sourceRefs":{"type":"array","items":{"type":"string","enum":basis.rows.iter().map(|row|row.id.clone()).collect::<Vec<_>>()}}
    }))});
    for field in ["reviewDecisions", "productionFeedback"] {
        schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!(field));
    }
    schema
}

fn creative_body(units: &[Value]) -> Value {
    json!(units.iter().map(|unit|json!({"scene":unit["scene"],"participants":unit["participants"],"props":unit["props"],
        "lighting":unit["lighting"],"initialState":unit["initialState"],"exitState":unit["exitState"],
        "shots":unit["shots"].as_array().into_iter().flatten().map(|shot|json!({"duration":shot["duration"],"camera":shot["camera"],"picture":shot["picture"],"lighting":shot["lighting"],"sound":shot["sound"],"transition":shot["transition"],"dialogueParts":shot["dialogueParts"].as_array().into_iter().flatten().map(|part|json!({"lineId":part["lineId"],"quoteStart":part["quoteStart"],"quoteEnd":part["quoteEnd"],"presentation":part["presentation"],"delivery":part["delivery"],"voiceStart":part["voiceStart"],"voiceEnd":part["voiceEnd"]})).collect::<Vec<_>>()})).collect::<Vec<_>>() })).collect::<Vec<_>>())
}

fn validate_delta(input: &Value, basis: &Basis, raw: &Value) -> Result<(Value, Value), String> {
    let mut material = json!({"assets":basis.assets,"sourceRows":basis.rows});
    super::asset_use_binding::inherit(&mut material, input)?;
    super::storyboard_sound_events::inherit(&mut material, input)?;
    let projected = storyboard::materialize_role_result(&material, raw)?;
    let raw = &projected;
    let groups = raw["replacements"].as_array().ok_or("精修缺少差量替换组")?;
    let decisions = raw["reviewDecisions"]
        .as_array()
        .ok_or("精修丢失reviewDecisions")?;
    if !raw["productionFeedback"].is_array() {
        return Err("精修丢失productionFeedback".into());
    }
    let old = basis.draft["units"].as_array().unwrap();
    let mut delta = raw.clone();
    let mut changed = HashSet::new();
    for (position, group) in groups.iter().enumerate() {
        let replaces = unique_ids(group, "replaces")?;
        if replaces.is_empty() || !replaces.is_subset(&basis.allowed) {
            return Err("精修替换越过获准单元范围；候选未采用".into());
        }
        let originals: Vec<_> = old
            .iter()
            .filter(|unit| replaces.contains(text(unit, "id")))
            .cloned()
            .collect();
        let new = group["units"].as_array().ok_or("精修替换缺少完整单元")?;
        if creative_body(&originals) == creative_body(new) {
            return Err("精修替换只改身份、标题或说明，未形成实际声画修改".into());
        }
        let target_refs: HashSet<_> = originals
            .iter()
            .flat_map(|unit| ids(unit, "sourceRefs").unwrap_or_default())
            .collect();
        let refs: Vec<_> = basis
            .rows
            .iter()
            .filter(|row| target_refs.contains(&row.id))
            .map(|row| row.id.clone())
            .collect();
        let mut replacement = json!({"schemaVersion":"storyboard-draft/v2","units":new,"fantasyRealization":group["fantasyRealization"],"revisionNotes":raw["revisionNotes"],"unresolved":raw["unresolved"]});
        storyboard::validate_authored(
            &basis.source,
            &mut replacement,
            &basis.assets,
            &input["capabilities"],
            &refs,
            text(&input["settings"], "visualStyle"),
            &format!("{}:", text(input, "episodeId")),
        )?;
        creative::add_delivery(
            &mut replacement,
            &basis.source,
            text(&input["settings"], "visualStyle"),
            &basis.assets,
            true,
        )?;
        if !storyboard::carrier_problems(&replacement, &input["capabilities"]).is_empty() {
            return Err("精修候选未满足实际视频载体合同；未凑整或裁声窗".into());
        }
        delta["replacements"][position]["units"] = replacement["units"].clone();
        delta["replacements"][position]["fantasyRealization"] =
            replacement["fantasyRealization"].clone();
        changed.extend(replaces);
    }
    let mut reviewed = HashSet::new();
    for decision in decisions {
        let targets = unique_ids(decision, "unitIds")?;
        if targets.is_empty()
            || !targets.is_subset(&basis.allowed)
            || ["coordinate", "reason", "preservedFunction"]
                .iter()
                .any(|field| text(decision, field).trim().is_empty())
        {
            return Err("精修问题组缺少真实坐标、许可范围或保全说明".into());
        }
        match text(decision, "state") {
            "changed" if targets.is_subset(&changed) => reviewed.extend(targets),
            "unresolved"
                if raw["unresolved"].as_array().is_some_and(|items| {
                    items
                        .iter()
                        .any(|issue| issue["coordinate"] == decision["coordinate"])
                }) =>
            {
                ()
            }
            _ => return Err("精修决策未对应实际修改/未解决项；不以retained辩护代替精修".into()),
        }
    }
    if !changed.is_subset(&reviewed) {
        return Err("精修有实际替换却丢失对应reviewDecisions".into());
    }
    for feedback in raw["productionFeedback"].as_array().unwrap() {
        if ["coordinate", "problem", "suggestedAction"]
            .iter()
            .any(|field| text(feedback, field).trim().is_empty())
            || ids(feedback, "sourceRefs")?
                .iter()
                .any(|id| !basis.rows.iter().any(|row| &row.id == id))
        {
            return Err("精修productionFeedback坐标/源锚无效".into());
        }
    }
    let draft = storyboard::apply_finish_delta(&basis.draft, &delta)?;
    let refs: Vec<_> = basis.rows.iter().map(|row| row.id.clone()).collect();
    storyboard::validate_storyboard_scope(
        &basis.source,
        &draft,
        &basis.assets,
        &input["capabilities"],
        Some(&refs),
    )?;
    storyboard::validate_event_bindings(&draft, &basis.context["plan"])?;
    storyboard::validate_fantasy_profile(
        &draft,
        basis.context["selectedFantasy"].as_array().unwrap(),
        true,
    )?;
    Ok((delta, draft))
}

fn candidate_hash(candidate: &Value) -> String {
    let mut unsigned = candidate.clone();
    unsigned.as_object_mut().unwrap().remove("candidateSha256");
    digest(&unsigned.to_string())
}

pub async fn run_storyboard_refinement(
    input: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    if model.trim().is_empty()
        || !["low", "medium", "high", "xhigh", "max", "ultra"].contains(&reasoning)
    {
        return Err("精修模型/reasoning无效".into());
    }
    let basis = basis(input)?;
    let facts = storyboard::scoped_facts(input, &basis.rows, &basis.context_rows);
    let scenarios: Vec<_> = basis.context["plan"]["workPackages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|owner| {
            owner["scenarioKeys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let selected = basis.context["selectedFantasy"].as_array().unwrap();
    let methods = storyboard::methods_for(&basis.rows, selected, &scenarios, false, &facts)?;
    let (model_assets, asset_selection) = asset_cards::project_for_scope(
        input,
        &basis.assets,
        text(input, "episodeId"),
        &basis.rows,
        &basis.context_rows,
    )?;
    let mut model_draft = basis.draft.clone();
    for unit in model_draft["units"].as_array_mut().unwrap() {
        for field in [
            "prompt",
            "channelPromptTemplate",
            "referenceBindings",
            "visualStyle",
            "seedanceExportVersion",
        ] {
            unit.as_object_mut().unwrap().remove(field);
        }
    }
    let mut payload = json!({"episodeId":input["episodeId"],"sourceSha256":basis.source.sha256,"sourceRows":basis.rows,"contextRows":basis.context_rows,
        "facts":facts,"assets":model_assets,"plan":basis.context["plan"],"entryFacts":basis.context["plan"]["entryFacts"],"selectedFantasy":selected,
        "previousDraft":model_draft,"baseDraftHash":storyboard::finish_base_hash(&basis.draft),"refinementScope":basis.scope,
        "reviewNotes":input["reviewNotes"],"preserveNotes":input["preserveNotes"],"settings":input["settings"],"capabilities":input["capabilities"],
        "productionDirection":input["productionDirection"],"refinementFeedbackVersion":FEEDBACK_VERSION,
        "acceptedProvenance":basis.context["unitBindings"],"capacityBrief":storyboard::original::capacity_brief(&input["capabilities"],Some(&basis.draft),input.get("voiceReference"),&basis.rows)});
    stage3_pack::attach_profile(&mut payload, input)?;
    creative::attach_scene_profile(&mut payload, input)?;
    payload["assetSelection"] = asset_selection;
    let request = creative::prepare_request(TextRequest {
        stage: "storyboardRefine".into(),
        skill: "director-refinement".into(),
        model: model.into(),
        reasoning: reasoning.into(),
        input: payload,
        schema: refinement_schema(&basis),
        rule_snapshot: stage3_pack::snapshot("director-refinement", &methods)?,
    })?;
    let mut request = executor.prepare_transport_request(request).await?;
    if executor.input_transport_characters(&request)? > super::STORYBOARD_TRANSPORT_LIMIT {
        return Err("完整精修集稿超过运输容量；不截断原文、规则或现稿，未调用模型".into());
    }
    let mut raw = match executor.execute(request.clone()).await {
        Ok(raw) => raw,
        Err(error) => {
            let next = executor.prepare_transport_request(request.clone()).await?;
            if serde_json::to_value(&next).map_err(|e| e.to_string())?
                == serde_json::to_value(&request).map_err(|e| e.to_string())?
            {
                return Err(error);
            }
            request = next;
            if executor.input_transport_characters(&request)? > super::STORYBOARD_TRANSPORT_LIMIT {
                return Err("精修恢复请求超过容量；原任务保留".into());
            }
            executor.execute(request.clone()).await?
        }
    };
    // An accepted cache has the Rust validation projection beside the original
    // delta. Replaying it must recreate the same candidate, never promote the
    // projection into a model-authored field or change its fingerprint.
    if let Some(object) = raw.as_object_mut() {
        object.remove("materializedDraft");
    }
    let (delta, draft) = match validate_delta(input, &basis, &raw) {
        Ok(validated) => validated,
        Err(reason) => {
            executor.reject(&request, &raw, &reason).await?;
            return Err(reason);
        }
    };
    let no_change = delta["replacements"].as_array().unwrap().is_empty();
    let unresolved = delta["unresolved"]
        .as_array()
        .is_none_or(|items| !items.is_empty());
    let mut accepted = raw.clone();
    accepted["materializedDraft"] = draft.clone();
    executor.accept(&request, &accepted).await?;
    let mut candidate = json!({"schemaVersion":CANDIDATE_VERSION,"state":if unresolved{"blocked"}else if no_change{"no_improvement"}else{"candidate"},
        "sourceSha256":basis.source.sha256,"episodeId":input["episodeId"],"baseDraftHash":storyboard::finish_base_hash(&basis.draft),
        "baseUnitBindings":basis.context["unitBindings"],"acceptedContext":basis.context,"inputBindingSha256":basis.input_binding,"refinementScope":basis.scope,
        "domainRequestSha256":digest(&serde_json::to_string(&request).map_err(|e|e.to_string())?),"ruleReceipt":request.rule_snapshot["creativePack"],
        "model":model,"reasoning":reasoning,"rawDelta":raw,"validatedDelta":delta,"draft":draft,
        "reviewDecisions":accepted["reviewDecisions"],"productionFeedback":accepted["productionFeedback"],"feedbackVersion":FEEDBACK_VERSION,
        "engineeringValidated":true,"creativeQualityAccepted":false,"automaticAdoption":false});
    candidate["candidateSha256"] = json!(candidate_hash(&candidate));
    Ok(candidate)
}

/// Called again inside the store's publication transaction/guard. A candidate
/// cannot replace a newer adopted base or move to a different source/backend.
pub fn adopt_storyboard_refinement(
    current_input: &Value,
    candidate: &Value,
) -> Result<Value, String> {
    if candidate["schemaVersion"] != CANDIDATE_VERSION
        || candidate["state"] != "candidate"
        || candidate["candidateSha256"] != candidate_hash(candidate)
    {
        return Err("精修候选身份、状态或内容指纹无效；原采用稿保持".into());
    }
    let basis = basis(current_input)?;
    if candidate["sourceSha256"] != basis.source.sha256
        || candidate["episodeId"] != current_input["episodeId"]
        || candidate["baseDraftHash"] != storyboard::finish_base_hash(&basis.draft)
        || candidate["baseUnitBindings"] != basis.context["unitBindings"]
        || candidate["acceptedContext"] != basis.context
        || candidate["inputBindingSha256"] != basis.input_binding
        || candidate["refinementScope"] != basis.scope
        || candidate["ruleReceipt"]["packSha256"] != stage3_pack::pack_sha256()?
    {
        return Err(
            "精修候选所绑定原文、已采用集稿、许可或制作上下文已变化；不覆盖历史结果".into(),
        );
    }
    let (delta, draft) = validate_delta(current_input, &basis, &candidate["rawDelta"])?;
    if delta != candidate["validatedDelta"]
        || draft != candidate["draft"]
        || delta["unresolved"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
    {
        return Err("精修采用未重现同一领域验证或尚有未解决项".into());
    }
    let groups = delta["replacements"].as_array().unwrap();
    let replaced: Vec<_> = groups
        .iter()
        .flat_map(|group| group["replaces"].as_array().unwrap())
        .cloned()
        .collect();
    let units: Vec<_> = groups
        .iter()
        .flat_map(|group| group["units"].as_array().unwrap())
        .cloned()
        .collect();
    let fantasy: Vec<_> = groups
        .iter()
        .flat_map(|group| group["fantasyRealization"].as_array().unwrap())
        .cloned()
        .collect();
    let old = basis.draft["units"].as_array().unwrap();
    let mut impacts = Vec::new();
    for group in groups {
        let last = group["replaces"].as_array().unwrap().last().unwrap();
        let position = old.iter().position(|unit| unit["id"] == *last).unwrap();
        let new_last = group["units"].as_array().unwrap().last().unwrap();
        if old[position]["exitState"] != new_last["exitState"] {
            if let Some(next) = old.get(position + 1) {
                impacts.push(json!({"episodeId":current_input["episodeId"],"fromUnitId":new_last["id"],"nextUnitId":next["id"],"reason":"导演精修已采用末态变化，实际依赖后继须重新核对"}));
            } else if let Some(next) = basis
                .source
                .episodes
                .iter()
                .position(|episode| episode.id == text(current_input, "episodeId"))
                .and_then(|index| basis.source.episodes.get(index + 1))
            {
                let mut successors: Vec<_> = current_input["existingUnits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|unit| unit["episodeId"] == next.id && unit["retired"] != true)
                    .collect();
                successors
                    .sort_by_key(|unit| storyboard::unit_source_position(&basis.source, unit));
                if let Some(first) = successors.first() {
                    impacts.push(json!({"episodeId":next.id,"fromUnitId":new_last["id"],"nextUnitId":first["id"],"reason":"前集导演精修已采用尾态变化，后集实际入口须重新核对"}));
                }
            }
        }
    }
    Ok(
        json!({"schemaVersion":"storyboard-delivery/v1","sourceSha256":basis.source.sha256,"scope":"units","status":"completed",
        "updatedEpisodeIds":[current_input["episodeId"]],"replacedUnitIds":replaced,"units":units,"plans":[],"waitingDecision":null,"waitingDecisions":[],
        "fantasyRealization":fantasy,"revisionNotes":delta["revisionNotes"],"unresolved":[],"blocked":[],"continuityImpacts":impacts,
        "ruleSnapshots":stage3_pack::rule_snapshots()?,"directorRefinement":{"candidateSha256":candidate["candidateSha256"],"domainRequestSha256":candidate["domainRequestSha256"],
            "baseDraftHash":candidate["baseDraftHash"],"baseUnitBindings":candidate["baseUnitBindings"],"ruleReceipt":candidate["ruleReceipt"],
            "reviewDecisions":candidate["reviewDecisions"],"productionFeedback":candidate["productionFeedback"],"feedbackVersion":FEEDBACK_VERSION,
            "acceptedPlanContext":basis.context}}),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{store::Store, workflow::guard_from_input};
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct Model {
        pub(crate) calls: Mutex<Vec<TextRequest>>,
        cache: Mutex<HashMap<String, Value>>,
        pub(crate) rejected: Mutex<Vec<String>>,
        pub(crate) mode: &'static str,
        pub(crate) durable: Option<(Store, String)>,
    }
    fn plan(request: &TextRequest) -> Value {
        let rows = request.input["sourceRows"].as_array().unwrap();
        let refs: Vec<_> = rows.iter().map(|row| &row["id"]).collect();
        let emotion = json!({"subject":"小王","contextTone":"门口的回应","audienceEffect":"行动可读","change":"回应后离开","landing":"走入门内","retain":["源动作、原话、说者与OS"]});
        json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"回应后行动",
            "events":[{"id":"E1","sceneId":rows[0]["sceneId"],"sourceRefs":refs,"boundaryBefore":{"kind":"episode_start","reason":"本集开始"},"emotionContract":emotion}],
            "workPackages":[{"id":"W1","eventId":"E1","sourceRefs":refs,"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":emotion}],
            "provisionalUnits":[{"id":"P1","packageId":"W1","sourceRefs":refs,"secondsRange":[3,9]}],"entryFacts":[],"fantasyCandidates":[]})
    }
    fn authored(request: &TextRequest) -> Value {
        let units:Vec<_>=request.input["sourceRows"].as_array().unwrap().iter().enumerate().map(|(i,row)| {
            let parts:Vec<_>=row["dialogue"]["spokenSegments"].as_array().into_iter().flatten().map(|segment|json!({
                "lineId":row["dialogue"]["id"],"quoteStart":segment["range"]["start"],"quoteEnd":segment["range"]["end"],
                "presentation":if row["dialogue"]["channel"]=="OS" {"OS"} else {"画内"},"delivery":"自然连读原句","voiceStart":0.0,"voiceEnd":1.4})).collect();
            json!({"id":format!("U{i}"),"episodeId":row["episodeId"],"eventId":"E1","sceneId":row["sceneId"],"title":"门口回应","intent":{"subject":"小王","change":"依源回应","landing":"动作可见","retain":["原话"]},
                "boundaryBefore":{"kind":if i==0 {"episode_start"}else{"continuous"},"reason":"本场源连续"},"unitDirection":"小王在门口回应后继续源动作。","scene":"门口","participants":["小王"],"props":[],"lighting":"门外侧光","initialState":"小王站在门口。","exitState":"小王保持源内状态。","sourceRefs":[row["id"]],"assetRefs":[],
                "shots":[{"id":format!("S{i}"),"duration":3.0,"camera":"近景读回应","picture":"小王回应并完成原文动作。","lighting":"侧光","sound":"环境声与原声","transition":"源动作连续切出","sourceRefs":[row["id"]],"contextRefs":[],"dialogueParts":parts}]})
        }).collect();
        json!({"schemaVersion":"storyboard-draft/v2","units":units,"fantasyRealization":[],"revisionNotes":[],"unresolved":[]})
    }
    fn delta(request: &TextRequest, mode: &str) -> Value {
        let units = request.input["previousDraft"]["units"].as_array().unwrap();
        let mut retained: Vec<_> = units.iter().map(|unit| unit["id"].clone()).collect();
        let mut groups = Vec::new();
        let mut decisions = Vec::new();
        if !["no_change", "unresolved"].contains(&mode) {
            let scope = request.input["refinementScope"]["unitIds"]
                .as_array()
                .unwrap();
            let targets = if mode == "outside" {
                vec![units.last().unwrap()]
            } else if mode == "scattered" {
                vec![&units[0], units.last().unwrap()]
            } else {
                vec![units
                    .iter()
                    .find(|unit| scope.contains(&unit["id"]))
                    .unwrap()]
            };
            for old in targets {
                let mut next = old.clone();
                next["id"] = json!(format!("refined-{}", text(old, "id")));
                next["title"] = json!("更清晰的动作回应");
                if mode != "title_only" {
                    next["shots"][0]["picture"] = json!(if text(&old["shots"][0], "picture")
                        .contains("同时抬眼")
                    {
                        "小王原句发声与推门共时，动作撞点处镜内近脸读清，再接住原文落点。"
                    } else {
                        "小王原句发声同时抬眼，门边动作起效时近脸及时读到反应，再接住源动作结果。"
                    });
                }
                if mode == "bad_quote" {
                    next["shots"][0]["dialogueParts"][0]["text"] = json!("擅改原话");
                }
                retained.retain(|id| *id != old["id"]);
                groups.push(json!({"replaces":[old["id"]],"units":[next],"fantasyRealization":[]}));
                decisions.push(json!({"state":"changed","coordinate":format!("{}/镜1",text(old,"id")),"unitIds":[old["id"]],"reason":"原反应到达迟缓→声画同时起效并在近脸读清","preservedFunction":"完整原话、源动作结果、说者声道与邻镜保持"}));
            }
        }
        let unresolved = if mode == "unresolved" {
            decisions.push(json!({"state":"unresolved","coordinate":"整集/末段","unitIds":[units[0]["id"]],"reason":"缺少可确定的图像依据","preservedFunction":"不猜造型，不越出许可"}));
            json!([{"coordinate":"整集/末段","problem":"缺少可确定的图像依据"}])
        } else {
            json!([])
        };
        json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,
            "replacements":groups,"revisionNotes":[],"reviewDecisions":decisions,"productionFeedback":[],"unresolved":unresolved})
    }
    pub(crate) fn request_key(request: &TextRequest) -> String {
        digest(&serde_json::to_string(request).unwrap())
    }
    fn wire_key(request: &TextRequest) -> String {
        digest(&serde_json::to_value(request).unwrap().to_string())
    }
    #[async_trait::async_trait]
    impl TextExecutor for Model {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            let key = request_key(&request);
            if let Some((store, run)) = &self.durable {
                if let Some(value) = store.accepted_step(run, &wire_key(&request))? {
                    return Ok(value);
                }
            }
            if let Some(value) = self.cache.lock().await.get(&key).cloned() {
                return Ok(value);
            }
            self.calls.lock().await.push(request.clone());
            let value = match request.stage.as_str() {
                "storyboardPlan" => plan(&request),
                "storyboardCreate" => authored(&request),
                "storyboardFinish" => {
                    json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":request.input["previousDraft"]["units"].as_array().unwrap().iter().map(|unit|&unit["id"]).collect::<Vec<_>>(),"replacements":[],"revisionNotes":[],"unresolved":[]})
                }
                "storyboardRefine" => delta(&request, self.mode),
                _ => return Err("合成测试不支持其他阶段，更不调用真实CLI".into()),
            };
            if let Some((store, run)) = &self.durable {
                let snapshot = serde_json::to_value(&request).unwrap();
                let key = wire_key(&request);
                let job = store.add_job(
                    "text",
                    &request.stage,
                    "codex",
                    "offline-fixture",
                    &snapshot,
                    "received",
                )?;
                store.event(
                    &job,
                    "text_received_raw",
                    &json!({"requestHash":key,"value":value}),
                )?;
                store.checkpoint(
                    run,
                    &key,
                    &request.stage,
                    &key,
                    "received",
                    Some(&job),
                    Some(&value),
                    None,
                )?;
            }
            Ok(value)
        }
        async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
            let key = request_key(request);
            self.cache.lock().await.insert(key.clone(), value.clone());
            if let Some((store, run)) = &self.durable {
                let key = wire_key(request);
                let job:String=store.conn()?.query_row("SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND request_hash=?2",rusqlite::params![run,key],|row|row.get(0)).map_err(|e|e.to_string())?;
                store.update_job(&job,"completed",&json!({"result":value,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}}))?;
                store.checkpoint(
                    run,
                    &key,
                    &request.stage,
                    &key,
                    "accepted",
                    Some(&job),
                    Some(value),
                    None,
                )?;
            }
            Ok(())
        }
        async fn reject(&self, _: &TextRequest, _: &Value, reason: &str) -> Result<(), String> {
            self.rejected.lock().await.push(reason.into());
            Ok(())
        }
    }

    pub(crate) async fn fixture() -> (tempfile::TempDir, Store, Value, String) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("isolated-source")).unwrap();
        store
            .create("第三阶段离线合成验收", "stage3-offline")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        let source = store
            .save_source(json!(super::super::prepare_source(
                "第一集\n场景：门口\n小王OS：“先走，别回头😀。”\n△小王推门。\n△小王走入。\n"
            )
            .unwrap()))
            .unwrap();
        let mut input = json!({"source":source,"assets":[],"foundation":{"version":0,"facts":[]},"settings":{"visualStyle":"16:9真人","fantasyMode":"off"},"capabilities":{"minDuration":0,"maxDuration":3,"imageLimit":0},"existingUnits":[],"selection":{"episodeIds":[source["episodes"][0]["id"]]}});
        stage3_pack::mark_input(&mut input).unwrap();
        let snapshot =
            json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input});
        let origin = store
            .add_job(
                "workflow",
                "storyboard.run",
                "local",
                "offline-fixture",
                &snapshot,
                "running",
            )
            .unwrap();
        let executor = Model {
            durable: Some((store.clone(), origin.clone())),
            ..Default::default()
        };
        let output =
            storyboard::run_storyboard(&input, &executor, "offline-fixture-model", "medium")
                .await
                .unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        store
            .publish_storyboard(&guard_from_input(&input), &output, &input, None)
            .unwrap();
        store
            .update_job(&origin, "completed", &json!({"result":output}))
            .unwrap();
        let units: Vec<_> = output["units"]
            .as_array()
            .unwrap()
            .iter()
            .map(|unit| store.get("units", text(unit, "id")).unwrap())
            .collect();
        assert_eq!(units.len(), 3);
        let calls = executor.calls.lock().await;
        let planner = calls
            .iter()
            .find(|request| request.stage == "storyboardPlan")
            .unwrap();
        let planner_job:String=store.conn().unwrap().query_row("SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND stage='storyboardPlan'",[&origin],|row|row.get(0)).unwrap();
        input["existingUnits"] = json!(units);
        input["episodeId"] = source["episodes"][0]["id"].clone();
        input["reviewNotes"] = json!("按整集找真实声画缺口，不改原话");
        input["preserveNotes"] = json!("保住源动作及OS");
        input["refinementScope"] = json!({});
        input["revisionContexts"] = json!([{"version":"accepted-storyboard-revision/v1","sourceSha256":source["sha256"],"episodeId":input["episodeId"],"originWorkflowId":origin,"planJobId":planner_job,"planRequestHash":wire_key(planner),"plan":output["plans"][0]["plan"],"selectedFantasy":[],"fantasyRealization":[],
            "unitBindings":units.iter().map(|unit|json!({"unitId":unit["id"],"unitVersion":unit["version"],"unitSha256":digest(&unit.to_string())})).collect::<Vec<_>>()}]);
        (directory, store, input, origin)
    }
    pub(crate) async fn saved_candidate(
        store: &Store,
        input: &Value,
        mode: &'static str,
    ) -> (String, Value) {
        let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.refine","input":input,"savedSettingsSha256":digest(&store.settings().unwrap().to_string())});
        let run = store
            .add_job(
                "workflow",
                "storyboard.refine",
                "local",
                "offline-fixture",
                &snapshot,
                "running",
            )
            .unwrap();
        let executor = Model {
            mode,
            durable: Some((store.clone(), run.clone())),
            ..Default::default()
        };
        let candidate =
            run_storyboard_refinement(input, &executor, "offline-fixture-model", "medium")
                .await
                .unwrap();
        store
            .update_job(&run, "completed", &json!({"result":candidate}))
            .unwrap();
        (run, candidate)
    }

    #[tokio::test]
    async fn asset_use_binding_new_three_roles_and_refinement_reopen_keep_raw_and_retained() {
        use super::super::asset_use_binding as binding;
        let source=super::super::prepare_source("第一集\n场景：门口\n△小王推门。\n").unwrap();
        let mut fresh=json!({"source":source,"assets":[],"foundation":{"facts":[]},"settings":{"fantasyMode":"off","visualStyle":"16:9真人"},"capabilities":{"minDuration":0,"maxDuration":15,"imageLimit":0},"selection":{"episodeIds":[source.episodes[0].id]}});
        stage3_pack::mark_input(&mut fresh).unwrap();binding::freeze_new(&mut fresh).unwrap();
        let model=Model::default();let output=storyboard::run_storyboard(&fresh,&model,"offline","medium").await.unwrap();
        assert_eq!(output["status"],"completed","{}",output["blocked"]);
        let calls=model.calls.lock().await;assert_eq!(calls.len(),3);
        for request in calls.iter(){assert_eq!(request.input[binding::FIELD],binding::PROFILE);assert_eq!(request.rule_snapshot["runtimeContract"][binding::FIELD],binding::PROFILE);}
        assert!(output["units"].as_array().unwrap().iter().all(|u|u[binding::FIELD]==binding::PROFILE));
        drop(calls);

        let (directory,store,mut input,_origin)=fixture().await;
        binding::freeze_new(&mut input).unwrap();
        let retained=input["existingUnits"][1].clone();let (run,candidate)=saved_candidate(&store,&input,"scattered").await;
        assert!(candidate["rawDelta"]["replacements"][0]["units"][0].get(binding::FIELD).is_none());
        assert_eq!(candidate["validatedDelta"]["replacements"][0]["units"][0][binding::FIELD],binding::PROFILE);
        let raw=candidate["rawDelta"].clone();let original=store.job(&run).unwrap()["result"].clone();
        let reopened=Store::open(directory.path().join("isolated-source")).unwrap();
        reopened.publish_storyboard_refinement(&run,text(&candidate,"candidateSha256"),&input).unwrap();
        assert_eq!(reopened.job(&run).unwrap()["result"],original);
        assert_eq!(reopened.get("units",text(&retained,"id")).unwrap(),retained);
        assert_eq!(reopened.job(&run).unwrap()["result"]["rawDelta"],raw);
        reopened.publish_storyboard_refinement(&run,text(&candidate,"candidateSha256"),&input).unwrap();
    }

    #[tokio::test]
    async fn sound_event_three_roles_keep_old_replay_and_prepared_requests_exact() {
        use super::super::storyboard_sound_events as sound;
        let source = super::super::prepare_source("第一集\n场景：门口\n小王：原话完整保留。\n").unwrap();
        let mut old = json!({"source":source,"assets":[],"foundation":{"facts":[]},"settings":{"fantasyMode":"off","visualStyle":"16:9真人"},"capabilities":{"minDuration":0,"maxDuration":15,"imageLimit":0},"selection":{"episodeIds":[source.episodes[0].id]}});
        stage3_pack::mark_input(&mut old).unwrap();
        let executor = Model::default();
        let before = storyboard::run_storyboard(&old, &executor, "offline", "medium").await.unwrap();
        let old_calls = executor.calls.lock().await.clone();
        assert_eq!(old_calls.len(), 3);
        let mut fresh = old.clone();
        sound::freeze_new(&mut fresh).unwrap();
        let output = storyboard::run_storyboard(&fresh, &executor, "offline", "medium").await.unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        let calls = executor.calls.lock().await.clone();
        assert_eq!(calls.len(), 6);
        for request in &calls[3..] {
            assert_eq!(request.input[sound::PROFILE_FIELD], sound::PROFILE);
            assert_eq!(request.rule_snapshot["runtimeContract"][sound::PROFILE_FIELD], sound::PROFILE);
            for prepared in [creative::prepare_request(request.clone()).unwrap(), stage3_pack::prepare_request(request.clone()).unwrap()] {
                assert_eq!(serde_json::to_vec(&prepared).unwrap(), serde_json::to_vec(request).unwrap(), "{} keeps frozen identity", request.stage);
            }
        }
        assert!(output["units"].as_array().unwrap().iter().all(|unit| unit[sound::PROFILE_FIELD] == sound::PROFILE));
        assert_eq!(storyboard::run_storyboard(&fresh, &executor, "offline", "medium").await.unwrap(), output);
        assert_eq!(storyboard::run_storyboard(&old, &executor, "offline", "medium").await.unwrap(), before);
        assert_eq!(executor.calls.lock().await.len(), 6, "both frozen versions reuse accepted results");
        assert_eq!(serde_json::to_vec(&executor.calls.lock().await[..3]).unwrap(), serde_json::to_vec(&old_calls).unwrap());
        for request in &old_calls {
            assert!(request.input.get(sound::PROFILE_FIELD).is_none());
            assert!(request.rule_snapshot["runtimeContract"].get(sound::PROFILE_FIELD).is_none());
        }
        fresh[sound::PROFILE_FIELD] = json!("unknown");
        assert!(storyboard::run_storyboard(&fresh, &executor, "offline", "medium").await.is_err());
        assert_eq!(executor.calls.lock().await.len(), 6, "unknown contracts fail before dispatch");
    }

    #[tokio::test]
    async fn sound_event_refinement_reopens_without_retyping_raw_or_retained_units() {
        use super::super::storyboard_sound_events as sound;
        let (directory, store, old, _origin) = fixture().await;
        let mut current = old.clone();
        sound::freeze_new(&mut current).unwrap();
        assert_ne!(basis(&old).unwrap().input_binding, basis(&current).unwrap().input_binding);
        let executor = Model { mode: "scattered", ..Default::default() };
        let old_candidate = run_storyboard_refinement(&old, &executor, "offline", "medium").await.unwrap();
        let new_candidate = run_storyboard_refinement(&current, &executor, "offline", "medium").await.unwrap();
        assert!(adopt_storyboard_refinement(&old, &new_candidate).is_err());
        assert!(adopt_storyboard_refinement(&current, &old_candidate).is_err());
        let calls = executor.calls.lock().await.clone();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].input[sound::PROFILE_FIELD], sound::PROFILE);
        assert_eq!(serde_json::to_vec(&creative::prepare_request(calls[1].clone()).unwrap()).unwrap(), serde_json::to_vec(&calls[1]).unwrap());
        assert_eq!(calls[1].input["previousDraft"], calls[0].input["previousDraft"]);
        run_storyboard_refinement(&old, &executor, "offline", "medium").await.unwrap();
        assert_eq!(executor.calls.lock().await.len(), 2);
        let retained = current["existingUnits"][1].clone();
        let (run, candidate) = saved_candidate(&store, &current, "scattered").await;
        let raw = candidate["rawDelta"].clone();
        let original = store.job(&run).unwrap()["result"].clone();
        assert!(raw["replacements"][0]["units"][0].get(sound::PROFILE_FIELD).is_none());
        assert_eq!(candidate["validatedDelta"]["replacements"][0]["units"][0][sound::PROFILE_FIELD], sound::PROFILE);
        let reopened = Store::open(directory.path().join("isolated-source")).unwrap();
        reopened.publish_storyboard_refinement(&run, text(&candidate, "candidateSha256"), &current).unwrap();
        assert_eq!(reopened.job(&run).unwrap()["result"], original);
        assert_eq!(reopened.get("units", text(&retained, "id")).unwrap(), retained);
        assert_eq!(reopened.job(&run).unwrap()["result"]["rawDelta"], raw);
        reopened.publish_storyboard_refinement(&run, text(&candidate, "candidateSha256"), &current).unwrap();
    }

    #[tokio::test]
    async fn actual_mac_rules_cover_three_roles_and_exact_legacy_replay_stays_old() {
        let source = super::super::prepare_source("第一集\n场景：门口\n△小王推门。\n").unwrap();
        let legacy = json!({"source":source,"assets":[],"foundation":{"facts":[]},"settings":{"fantasyMode":"off","visualStyle":"16:9真人"},"capabilities":{"minDuration":0,"maxDuration":15,"imageLimit":0},"selection":{"episodeIds":[source.episodes[0].id]}});
        let executor = Model::default();
        let before = storyboard::run_storyboard(&legacy, &executor, "offline", "medium")
            .await
            .unwrap();
        let old_calls = executor.calls.lock().await.clone();
        assert_eq!(old_calls.len(), 3);
        for request in &old_calls {
            assert!(request.input["capacityBrief"].get("rhythm").is_none());
        }
        let mut current = legacy.clone();
        stage3_pack::mark_input(&mut current).unwrap();
        let fresh = storyboard::run_storyboard(&current, &executor, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(fresh["status"], "completed");
        let calls = executor.calls.lock().await.clone();
        assert_eq!(calls.len(), 6);
        for request in &calls[3..] {
            assert_eq!(
                request.input[stage3_pack::RHYTHM_PROFILE_FIELD],
                stage3_pack::RHYTHM_PROFILE
            );
            assert_eq!(
                request.input["capacityBrief"]["preferredRange"],
                Value::Null
            );
            assert_eq!(
                request.input["capacityBrief"]["rhythm"]["reference"]["commonShotSecondsRange"],
                json!([2, 5])
            );
            assert_eq!(
                request.input["capacityBrief"]["rhythm"]["audit"]["available"],
                request.stage == "storyboardFinish"
            );
            assert_eq!(
                request.rule_snapshot["creativePack"]["packId"],
                stage3_pack::PACK_ID
            );
            assert_eq!(
                request.rule_snapshot["creativePack"]["dialogueProfile"],
                stage3_pack::DIALOGUE_PROFILE
            );
            assert_eq!(
                request.rule_snapshot["primary"]["sha256"],
                stage3_pack::rule_snapshots()
                    .unwrap()
                    .iter()
                    .find(|rule| rule.id == request.skill)
                    .unwrap()
                    .sha256
            );
            assert!(!text(&request.rule_snapshot["runtimeContract"], "content")
                .contains("每镜尽量2–5秒"));
        }
        let again = storyboard::run_storyboard(&current, &executor, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(again, fresh);
        assert_eq!(
            storyboard::run_storyboard(&legacy, &executor, "offline", "medium")
                .await
                .unwrap(),
            before
        );
        assert_eq!(executor.calls.lock().await.len(), 6);
        assert_eq!(
            serde_json::to_value(&executor.calls.lock().await[..3]).unwrap(),
            serde_json::to_value(&old_calls).unwrap()
        );
        let mut unknown = current;
        unknown[stage3_pack::PROFILE_FIELD] = json!("unknown");
        assert!(
            storyboard::run_storyboard(&unknown, &executor, "offline", "medium")
                .await
                .is_err()
        );
        assert_eq!(executor.calls.lock().await.len(), 6);
    }

    #[tokio::test]
    async fn refinement_scene_profile_is_frozen_and_candidate_adoption_binding_cannot_drift() {
        let (_directory,_store,old,_origin) = fixture().await;
        let executor = Model {mode:"changed",..Default::default()};
        let old_basis = basis(&old).unwrap();
        let old_candidate = run_storyboard_refinement(&old,&executor,"offline","medium").await.unwrap();
        let first = executor.calls.lock().await[0].clone();
        assert!(first.input.get(creative::SCENE_PROFILE_FIELD).is_none());
        let mut current = old.clone();
        storyboard::freeze_scene_input_profile(&mut current).unwrap();
        let new_basis = basis(&current).unwrap();
        assert_ne!(old_basis.input_binding,new_basis.input_binding,"candidate freeze includes new scene guidance");
        let new_candidate = run_storyboard_refinement(&current,&executor,"offline","medium").await.unwrap();
        assert!(adopt_storyboard_refinement(&current,&old_candidate).is_err(),"a candidate cannot silently switch profile");
        assert!(adopt_storyboard_refinement(&old,&new_candidate).is_err());
        adopt_storyboard_refinement(&current,&new_candidate).unwrap();
        let calls = executor.calls.lock().await.clone();
        assert_eq!(calls.len(),2);
        assert_eq!(calls[1].input[creative::SCENE_PROFILE_FIELD],creative::SCENE_PROFILE);
        assert_eq!(calls[1].rule_snapshot["runtimeContract"]["sceneProfile"],creative::SCENE_PROFILE);
        assert_eq!(calls[1].input["previousDraft"],first.input["previousDraft"],"full adopted draft stays identical model material");
        run_storyboard_refinement(&old,&executor,"offline","medium").await.unwrap();
        assert_eq!(executor.calls.lock().await.len(),2,"old frozen refinement remains cache-compatible");
        current[creative::SCENE_PROFILE_FIELD] = json!("unknown");
        assert!(run_storyboard_refinement(&current,&executor,"offline","medium").await.is_err());
        assert_eq!(executor.calls.lock().await.len(),2);
    }

    #[tokio::test]
    async fn refinement_feeds_the_full_adopted_episode_and_changes_only_permitted_group() {
        let (_directory, _store, mut input, _origin) = fixture().await;
        let first = input["existingUnits"][0]["id"].clone();
        input["refinementScope"] = json!({"unitIds":[first]});
        let frozen = input.clone();
        let executor = Model {
            mode: "changed",
            ..Default::default()
        };
        let candidate = run_storyboard_refinement(&input, &executor, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(candidate["state"], "candidate");
        assert_eq!(input, frozen);
        assert_eq!(candidate["automaticAdoption"], false);
        assert_eq!(candidate["creativeQualityAccepted"], false);
        let calls = executor.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].stage, "storyboardRefine");
        assert_eq!(calls[0].skill, "director-refinement");
        assert_eq!(
            calls[0].input["capacityBrief"]["rhythm"]["audit"]["available"],
            true
        );
        assert_eq!(
            calls[0].input["capacityBrief"]["rhythm"]["audit"]["providedUnitCount"],
            3
        );
        assert_eq!(
            calls[0].input["capacityBrief"]["rhythm"]["reference"]["hardShotDurationBounds"],
            false
        );
        assert_eq!(
            calls[0].input["previousDraft"]["units"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(calls[0].input["sourceRows"].as_array().unwrap().len(), 3);
        assert_eq!(calls[0].input["refinementScope"]["unitIds"], json!([first]));
        assert!(!calls[0].rule_snapshot["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule["id"] == "episode-finishing-editor"));
        drop(calls);
        assert_eq!(candidate["draft"]["units"][1], input["existingUnits"][1]);
        assert_eq!(candidate["draft"]["units"][2], input["existingUnits"][2]);
        assert_eq!(
            candidate["draft"]["units"][0]["shots"][0]["dialogueParts"],
            input["existingUnits"][0]["shots"][0]["dialogueParts"]
        );
        let output = adopt_storyboard_refinement(&input, &candidate).unwrap();
        assert_eq!(output["replacedUnitIds"], json!([first]));
        assert_eq!(output["units"].as_array().unwrap().len(), 1);
        assert_eq!(
            output["directorRefinement"]["acceptedPlanContext"],
            input["revisionContexts"][0]
        );
        assert_eq!(
            run_storyboard_refinement(&input, &executor, "offline", "medium")
                .await
                .unwrap(),
            candidate
        );
        assert_eq!(
            executor.calls.lock().await.len(),
            1,
            "exact accepted replay creates no model call"
        );
        let mut stale = input.clone();
        stale["settings"]["visualStyle"] = json!("其他风格");
        assert!(adopt_storyboard_refinement(&stale, &candidate).is_err());
        let mut forged = candidate;
        forged["draft"]["units"][0]["title"] = json!("改坏候选");
        assert!(adopt_storyboard_refinement(&input, &forged).is_err());
    }

    #[tokio::test]
    async fn non_improvements_source_changes_scope_changes_and_unresolved_are_never_adopted() {
        let (_directory, _store, mut input, _origin) = fixture().await;
        input["refinementScope"] = json!({"unitIds":[input["existingUnits"][0]["id"]]});
        for mode in ["title_only", "bad_quote", "outside"] {
            let executor = Model {
                mode,
                ..Default::default()
            };
            assert!(
                run_storyboard_refinement(&input, &executor, "offline", "medium")
                    .await
                    .is_err(),
                "{mode}"
            );
            assert_eq!(executor.calls.lock().await.len(), 1);
            assert_eq!(executor.rejected.lock().await.len(), 1);
        }
        for (mode, state) in [("no_change", "no_improvement"), ("unresolved", "blocked")] {
            let executor = Model {
                mode,
                ..Default::default()
            };
            let candidate = run_storyboard_refinement(&input, &executor, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(candidate["state"], state);
            assert!(adopt_storyboard_refinement(&input, &candidate).is_err());
        }
        let mut unbound = input;
        unbound["revisionContexts"][0]["unitBindings"]
            .as_array_mut()
            .unwrap()
            .pop();
        let executor = Model::default();
        assert!(
            run_storyboard_refinement(&unbound, &executor, "offline", "medium")
                .await
                .is_err()
        );
        assert!(executor.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn saved_candidates_reopen_and_adopt_atomically_without_overwriting_raw_or_retained_units(
    ) {
        let (directory, store, input, _origin) = fixture().await;
        let (run, candidate) = saved_candidate(&store, &input, "scattered").await;
        let original = store.job(&run).unwrap()["result"].clone();
        let unchanged = input["existingUnits"][1].clone();
        let reopened = Store::open(directory.path().join("isolated-source")).unwrap();
        reopened
            .publish_storyboard_refinement(&run, text(&candidate, "candidateSha256"), &input)
            .unwrap();
        assert_eq!(
            reopened.job(&run).unwrap()["result"],
            original,
            "candidate raw result stays immutable"
        );
        assert_eq!(
            reopened.get("units", text(&unchanged, "id")).unwrap(),
            unchanged
        );
        let adopted = reopened.job(&run).unwrap();
        let receipt = &adopted["prepared"]["refinementAdoption"];
        assert_eq!(receipt["adoptedUnits"].as_array().unwrap().len(), 3);
        assert_eq!(receipt["candidateSha256"], candidate["candidateSha256"]);
        let left = text(
            &candidate["validatedDelta"]["replacements"][0]["units"][0],
            "id",
        );
        let right = text(
            &candidate["validatedDelta"]["replacements"][1]["units"][0],
            "id",
        );
        let links: Vec<(String, String)> = reopened
            .conn()
            .unwrap()
            .prepare("SELECT unit_id,dependency_id FROM unit_dependencies")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(links.contains(&(text(&unchanged, "id").into(), left.into())));
        assert!(links.contains(&(right.into(), text(&unchanged, "id").into())));
        let count_before: usize = reopened
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type='storyboard_refinement_adopted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        reopened
            .publish_storyboard_refinement(&run, text(&candidate, "candidateSha256"), &input)
            .unwrap();
        assert_eq!(reopened.job(&run).unwrap(), adopted);
        assert_eq!(count_before, 1);
    }

    #[tokio::test]
    async fn native_saved_executor_reopens_the_exact_refinement_checkpoint_without_dispatch() {
        let (directory, store, input, _origin) = fixture().await;
        let model = Model {
            mode: "changed",
            ..Default::default()
        };
        let candidate =
            run_storyboard_refinement(&input, &model, "offline-fixture-model", "medium")
                .await
                .unwrap();
        let request = model.calls.lock().await[0].clone();
        let run=store.add_job("workflow","storyboard.refine","local","offline-fixture",
            &json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.refine","input":input,"savedSettingsSha256":digest(&store.settings().unwrap().to_string())}),"completed").unwrap();
        store
            .update_job(&run, "completed", &json!({"result":candidate}))
            .unwrap();
        // The native saved executor must read this domain-accepted synthetic
        // checkpoint after reopening. Fallback is loopback, never an external API.
        let settings = json!({"textProvider":"api","textEndpoint":"http://127.0.0.1:9","textApiToken":"offline-fixture-placeholder"});
        let mut snapshot = serde_json::to_value(&request).unwrap();
        snapshot["executorPolicy"] = json!("codex-isolated/v1");
        snapshot["executorIdentity"] = json!({"provider":"api","endpoint":"http://127.0.0.1:9"});
        let hash = digest(&snapshot.to_string());
        let mut accepted = candidate["rawDelta"].clone();
        accepted["materializedDraft"] = candidate["draft"].clone();
        let job = store
            .add_job(
                "text",
                "storyboardRefine",
                "api",
                "offline-fixture",
                &snapshot,
                "completed",
            )
            .unwrap();
        store.update_job(&job,"completed",&json!({"result":accepted,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
        store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":hash,"value":candidate["rawDelta"]}),
            )
            .unwrap();
        store
            .checkpoint(
                &run,
                &format!("storyboardRefine:{}:", text(&input, "episodeId")),
                "storyboardRefine",
                &hash,
                "accepted",
                Some(&job),
                Some(&accepted),
                None,
            )
            .unwrap();
        let reopened = Store::open(directory.path().join("isolated-source")).unwrap();
        let before_jobs = reopened.jobs().unwrap().len();
        let original_result: String = reopened
            .conn()
            .unwrap()
            .query_row("SELECT result FROM jobs WHERE id=?1", [&run], |row| {
                row.get(0)
            })
            .unwrap();
        let saved = crate::text_executor::SavedTextExecutor {
            store: reopened.clone(),
            settings,
            workflow_id: Some(run.clone()),
        };
        let replay = run_storyboard_refinement(&input, &saved, "offline-fixture-model", "medium")
            .await
            .unwrap();
        assert!(
            replay == candidate,
            "native replay changed the frozen candidate"
        );
        assert_eq!(
            reopened.jobs().unwrap().len(),
            before_jobs,
            "no new text task or generation dispatch"
        );
        reopened
            .publish_storyboard_refinement(&run, text(&candidate, "candidateSha256"), &input)
            .unwrap();
        let result_after: String = reopened
            .conn()
            .unwrap()
            .query_row("SELECT result FROM jobs WHERE id=?1", [&run], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            result_after, original_result,
            "adoption must preserve original candidate bytes"
        );
        let mut presented_candidate = candidate.clone();
        // Store::job adds its independently computed mediaIds for presentation.
        // Verify that projection separately from the immutable original result.
        presented_candidate["mediaIds"] = json!([]);
        assert!(reopened.job(&run).unwrap()["result"] == presented_candidate);
    }

    #[tokio::test]
    async fn adopted_preimage_settings_and_raw_checkpoint_conflicts_roll_back_the_whole_publication(
    ) {
        for conflict in ["unit", "settings", "raw"] {
            let (_directory, store, input, _origin) = fixture().await;
            let (run, candidate) = saved_candidate(&store, &input, "changed").await;
            let first = text(&input["existingUnits"][0], "id");
            match conflict {
                "unit" => {
                    let mut unit = store.get("units", first).unwrap();
                    unit["shots"][0]["picture"] = json!("用户手改");
                    store.save_unit(unit).unwrap();
                }
                "settings" => store.save_settings(&json!({"ratio":"9:16"})).unwrap(),
                "raw" => {
                    store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE type='text_received_raw' AND job_id IN (SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1)",[&run]).unwrap();
                }
                _ => unreachable!(),
            }
            let before = store.get("units", first).unwrap();
            let job = store.job(&run).unwrap();
            assert!(
                store
                    .publish_storyboard_refinement(
                        &run,
                        text(&candidate, "candidateSha256"),
                        &input
                    )
                    .is_err(),
                "{conflict}"
            );
            assert_eq!(store.get("units", first).unwrap(), before);
            assert_eq!(store.job(&run).unwrap(), job);
            assert!(store
                .get(
                    "units",
                    text(
                        &candidate["validatedDelta"]["replacements"][0]["units"][0],
                        "id"
                    )
                )
                .is_err());
        }
    }
}
