//! Versioned program rendering. The model never edits derived prompt fields.
//! Historical author/finisher requests retain their old byte-for-byte renderer;
//! an unallocated review repair can freeze a narrow rendering amendment.
use super::*;

pub(super) const VERSION: &str = "frame-render/v1";
pub(super) const PROFILE_FIELD: &str = "seriesFrameRenderVersion";
const REPAIR_FIELD: &str = "seriesFrameRenderRepair";
const UNIT_FIELD: &str = "promptRender";
const HELD: &str = "SERIES_FRAME_RENDER_BINDING_HELD";
const CONTRACT: &str = "prompt与promptRender是Rust派生字段，不在模型输出schema中。继续仅修复schema可写分镜；Rust校验后为明确问题范围的单元补充已确认视频画幅，再交下一独审。原问题、完整源、原话、镜表及无关单元不变。资产板画幅不受此合同影响。";

fn ratio(input: &Value) -> Result<Option<String>, String> {
    let direction = direction(input);
    let value = &direction["ratio"];
    if value.is_null() || value.as_str() == Some("") {
        return Ok(None);
    }
    let text = value.as_str().ok_or(HELD)?;
    let Some((width, height)) = text.split_once(':') else {
        return Err(HELD.into());
    };
    if [width, height].iter().any(|v| {
        v.is_empty()
            || v.len() > 4
            || !v.bytes().all(|b| b.is_ascii_digit())
            || v.parse::<u32>().ok().is_none_or(|n| n == 0)
    }) {
        return Err(HELD.into());
    }
    Ok(Some(text.into()))
}
fn metadata(ratio: &str) -> Value {
    json!({"version":VERSION,"ratio":ratio})
}
pub(super) fn mark_input(input: &mut Value) -> Result<(), String> {
    validate_profile(input)?;
    if ratio(input)?.is_some() {
        input[PROFILE_FIELD] = json!(VERSION);
    }
    Ok(())
}
pub(super) fn validate_profile(input: &Value) -> Result<(), String> {
    if input.get(PROFILE_FIELD).is_some_and(|v| v != VERSION)
        || (input[PROFILE_FIELD] == VERSION && ratio(input)?.is_none())
    {
        return Err(HELD.into());
    }
    Ok(())
}
/// Called after the unchanged legacy prompt compiler, making replay idempotent.
pub(super) fn materialize(draft: &mut Value, input: &Value) -> Result<(), String> {
    validate_profile(input)?;
    for unit in draft["units"].as_array_mut().ok_or(HELD)? {
        if input[PROFILE_FIELD] != VERSION && unit.get(UNIT_FIELD).is_none() {
            continue;
        }
        let ratio = ratio(input)?.ok_or(HELD)?;
        let expected = metadata(&ratio);
        if unit.get(UNIT_FIELD).is_some_and(|v| v != &expected) {
            return Err(HELD.into());
        }
        unit[UNIT_FIELD] = expected;
        unit["prompt"] = json!(format!("画幅：{ratio}\n\n{}", s(unit, "prompt")));
    }
    Ok(())
}
pub(super) fn export_style(unit: &Value, style: &str) -> Result<String, String> {
    let Some(render) = unit.get(UNIT_FIELD) else {
        return Ok(style.into());
    };
    let ratio = s(render, "ratio");
    if ratio.is_empty() || render != &metadata(ratio) {
        return Err(HELD.into());
    }
    Ok(if style.is_empty() {
        format!("画幅：{ratio}")
    } else {
        format!("{style}；画幅：{ratio}")
    })
}
fn candidate(request: &TextRequest) -> Result<Value, String> {
    let packet = &request.input["packet"];
    Ok(json!({"schemaVersion":"series-episode-candidate/v1",
        "plan":packet_value(packet,"plan")?,"selection":packet_value(packet,"selection")?,
        "draft":packet_value(packet,"draft")?}))
}
fn affected(request: &TextRequest) -> Result<Vec<String>, String> {
    let issues = packet_value(&request.input["packet"], "issues")?
        .as_array()
        .ok_or(HELD)?;
    let all: HashSet<_> = issues
        .iter()
        .filter(|i| i["severity"] == "must_fix")
        .map(|i| string_list(&i["unitIds"]))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let before = candidate(request)?;
    let units = before["draft"]["units"].as_array().ok_or(HELD)?;
    let ordered: Vec<_> = units
        .iter()
        .filter(|u| all.contains(s(u, "id")))
        .map(|u| s(u, "id").to_owned())
        .collect();
    if ordered.is_empty() || ordered.len() != all.len() {
        return Err(HELD.into());
    }
    Ok(ordered)
}
fn proof(
    legacy: &TextRequest,
    input: &Value,
    review_request_hash: &Value,
    review_raw_hash: &Value,
) -> Result<Value, String> {
    let before = candidate(legacy)?;
    let direction = packet_value(&legacy.input["packet"], "direction")?;
    if legacy.stage != "seriesRepair"
        || legacy.skill != "series-repair"
        || legacy.input["role"] != "repair"
        || legacy.input.get(REPAIR_FIELD).is_some()
        || input[PROFILE_FIELD] == VERSION
        || direction != &super::direction(input)
        || legacy.input["workflowProfile"] != PROFILE
        || legacy.input["freshContext"] != true
    {
        return Err(HELD.into());
    }
    let role_id = s(&legacy.input, "executionRoleId");
    let episode = s(&legacy.input, "episodeId");
    if !(1..=MAX_REPAIRS).any(|n| role_id == format!("{episode}/repair/review-{n}")) {
        return Err(HELD.into());
    }
    Ok(
        json!({"version":VERSION,"mode":"review-repair","sourceSha256":legacy.input["sourceSha256"],
        "episodeId":episode,"executionRoleId":role_id,"ratio":ratio(input)?.ok_or(HELD)?,
        "directionSha256":hash(direction),"candidateSha256":hash(&before),
        "issuesSha256":hash(packet_value(&legacy.input["packet"],"issues")?),
        "legacyRequestSha256":digest(&serde_json::to_string(legacy).map_err(|e|e.to_string())?),
        "reviewRequestSha256":review_request_hash,"reviewRawSha256":review_raw_hash,
        "affectedUnitIds":affected(legacy)?,"executorContract":CONTRACT}),
    )
}
pub(super) fn project(
    legacy: &TextRequest,
    input: &Value,
    receipt: &Value,
) -> Result<TextRequest, String> {
    if input[PROFILE_FIELD] == VERSION || ratio(input)?.is_none() {
        return Ok(legacy.clone());
    }
    let before = candidate(legacy)?;
    let affected = affected(legacy)?;
    if before["draft"]["units"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .filter(|u| affected.iter().any(|id| u["id"] == *id))
        .all(|u| u.get(UNIT_FIELD).is_some())
    {
        return Ok(legacy.clone());
    }
    let review: TextRequest =
        serde_json::from_value(receipt["request"].clone()).map_err(|_| HELD)?;
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| legacy.input["episodeId"] == e.id)
        .ok_or(HELD)?;
    validate_role_packet(legacy, input, &source, episode)?;
    validate_role_packet(&review, input, &source, episode)?;
    let iteration = (1..=MAX_REPAIRS)
        .find(|n| legacy.input["executionRoleId"] == format!("{}/repair/review-{n}", episode.id))
        .ok_or(HELD)?;
    if receipt != &role_receipt(&review, &receipt["raw"])?
        || review.stage != "seriesReview"
        || review.skill != "series-reviewer"
        || review.schema != controller_review::schema(controller_review::marker(&review))
        || review.model != legacy.model
        || review.reasoning != legacy.reasoning
        || review.input["executionRoleId"] != format!("{}/reviewer/{}", episode.id, iteration - 1)
        || review.input["candidateSha256"] != hash(&before)
        || candidate(&review)? != before
        || packet_value(&legacy.input["packet"], "issues")? != &controller_review::effective_issues(legacy, receipt, input)?
        || controller_review::validate_review_context(
            &receipt["raw"],
            &before,
            &source,
            episode,
            packet_value(&review.input["packet"], "entry")?,
            controller_review::marker(&review),
        )?["conclusion"]
            != "revise"
    {
        return Err(HELD.into());
    }
    for kind in ["source", "direction", "state", "assets", "entry"] {
        if packet_value(&legacy.input["packet"], kind)?
            != packet_value(&review.input["packet"], kind)?
        {
            return Err(HELD.into());
        }
    }
    let mut projected = legacy.clone();
    projected.input[REPAIR_FIELD] = proof(
        legacy,
        input,
        &receipt["requestSha256"],
        &receipt["rawSha256"],
    )?;
    Ok(projected)
}
pub(crate) fn validate_projection(
    legacy: &TextRequest,
    projected: &TextRequest,
    input: &Value,
    receipt: &Value,
) -> Result<(), String> {
    if serde_json::to_value(projected).map_err(|_| HELD)?
        != serde_json::to_value(project(legacy, input, receipt)?).map_err(|_| HELD)?
    {
        return Err(HELD.into());
    }
    Ok(())
}
fn original_request(request: &TextRequest) -> Result<TextRequest, String> {
    let mut original = request.clone();
    original
        .input
        .as_object_mut()
        .ok_or(HELD)?
        .remove("textTransportRetry");
    if let Some(repair) = original
        .input
        .as_object_mut()
        .ok_or(HELD)?
        .remove("validationRepair")
    {
        let role_id = s(&request.input[REPAIR_FIELD], "executionRoleId");
        if repair["version"] != "series-domain-repair/v1"
            || repair["maximumAttempts"] != MAX_REPAIRS
            || !(1..=MAX_REPAIRS).any(|n| {
                repair["attempt"] == n
                    && request.input["executionRoleId"]
                        == format!("{role_id}/validation-repair/{n}")
            })
        {
            return Err(HELD.into());
        }
        original.input["executionRoleId"] = json!(role_id);
        original.input["task"] = json!(task("repair"));
        if repair["originRequestSha256"]
            != digest(&serde_json::to_string(&original).map_err(|_| HELD)?)
        {
            return Err(HELD.into());
        }
    }
    Ok(original)
}
/// Restore only program-owned fields already present in the frozen candidate,
/// or newly authorized by this exact repair's rendering contract.
pub(super) fn restore_for_request(
    request: &TextRequest,
    candidate: &mut Value,
    input: &Value,
    scope: Option<(&Value, &Value)>,
) -> Result<(), String> {
    let prior = scope
        .map(|(previous, _)| previous["draft"].clone())
        .or_else(|| {
            packet_value(&request.input["packet"], "draft")
                .ok()
                .cloned()
        })
        .unwrap_or(Value::Null);
    let prior_units = prior["units"].as_array();
    let mut rendered = HashSet::new();
    let mut rendered_refs = HashSet::new();
    // A later local repair may split a previously rendered unit. Inherit its
    // program metadata by exact source coverage even when the model changes
    // the unit ID; a second rendering upgrade is neither needed nor allowed.
    for unit in prior_units.into_iter().flatten() {
        if let Some(saved) = unit.get(UNIT_FIELD) {
            if saved != &metadata(&ratio(input)?.ok_or(HELD)?) {
                return Err(HELD.into());
            }
            rendered_refs.extend(string_list(&unit["sourceRefs"])?);
        }
    }
    if request.input.get(REPAIR_FIELD).is_some() {
        if scope.is_none() {
            return Err(HELD.into());
        }
        let original = original_request(request)?;
        let contract = &original.input[REPAIR_FIELD];
        let mut legacy = original.clone();
        legacy
            .input
            .as_object_mut()
            .ok_or(HELD)?
            .remove(REPAIR_FIELD);
        if contract
            != &proof(
                &legacy,
                input,
                &contract["reviewRequestSha256"],
                &contract["reviewRawSha256"],
            )?
        {
            return Err(HELD.into());
        }
        rendered.extend(string_list(&contract["affectedUnitIds"])?);
        for unit in prior_units
            .into_iter()
            .flatten()
            .filter(|u| rendered.contains(s(u, "id")))
        {
            rendered_refs.extend(string_list(&unit["sourceRefs"])?);
        }
    }
    for unit in candidate["draft"]["units"].as_array_mut().ok_or(HELD)? {
        let old = prior_units
            .into_iter()
            .flatten()
            .find(|u| u["id"] == unit["id"]);
        let trusted = old.and_then(|u| u.get(UNIT_FIELD));
        if input[PROFILE_FIELD] != VERSION
            && unit.get(UNIT_FIELD).is_some_and(|v| Some(v) != trusted)
        {
            return Err(HELD.into());
        }
        unit.as_object_mut().ok_or(HELD)?.remove(UNIT_FIELD);
        if let Some(value) = trusted {
            unit[UNIT_FIELD] = value.clone();
        }
        let new_affected = if old.is_none() && !rendered_refs.is_empty() {
            let refs = string_list(&unit["sourceRefs"])?;
            !refs.is_empty() && refs.iter().all(|r| rendered_refs.contains(r))
        } else {
            false
        };
        if rendered.contains(s(unit, "id")) || new_affected {
            unit[UNIT_FIELD] = metadata(&ratio(input)?.ok_or(HELD)?);
        }
    }
    Ok(())
}
pub(super) fn validate_adoption(input: &Value, delivery: &Value) -> Result<(), String> {
    let units = delivery["bundle"]["draft"]["units"]
        .as_array()
        .ok_or(HELD)?;
    if input[PROFILE_FIELD] != VERSION && units.iter().all(|u| u.get(UNIT_FIELD).is_none()) {
        return Ok(());
    }
    let roles = delivery["adoptionReceipt"]["roleReceipts"]
        .as_array()
        .ok_or(HELD)?;
    for (position, role) in roles.iter().enumerate() {
        if role["request"]["input"].get(REPAIR_FIELD).is_none() {
            continue;
        }
        let request: TextRequest =
            serde_json::from_value(role["request"].clone()).map_err(|_| HELD)?;
        let original = original_request(&request)?;
        let mut legacy = original.clone();
        let contract = legacy
            .input
            .as_object_mut()
            .ok_or(HELD)?
            .remove(REPAIR_FIELD)
            .ok_or(HELD)?;
        let review = roles[..position]
            .iter()
            .rev()
            .find(|r| {
                r["role"] == "reviewer"
                    && r["requestSha256"] == contract["reviewRequestSha256"]
                    && r["rawSha256"] == contract["reviewRawSha256"]
            })
            .ok_or(HELD)?;
        validate_projection(&legacy, &original, input, review)?;
    }
    let last = roles
        .iter()
        .rev()
        .find(|r| matches!(s(r, "role"), "author" | "finisher" | "repair"))
        .ok_or(HELD)?;
    let request: TextRequest = serde_json::from_value(last["request"].clone()).map_err(|_| HELD)?;
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| delivery["episodeId"] == e.id)
        .ok_or(HELD)?;
    let assets = delivery["bundle"]["assets"].as_array().ok_or(HELD)?;
    let before = if request.input["role"] == "repair" {
        Some(candidate(&request)?)
    } else {
        None
    };
    let issues = if before.is_some() {
        Some(packet_value(&request.input["packet"], "issues")?)
    } else {
        None
    };
    let scoped = before.as_ref().zip(issues);
    let checked = repair::validate_scoped_candidate(
        &request,
        &last["raw"],
        &source,
        episode,
        assets,
        input,
        scoped,
        delivery_supply_proof(delivery).get("episodeSupply"),
    )?;
    if checked["draft"] != delivery["bundle"]["draft"]
        || checked["plan"] != delivery["bundle"]["plan"]
        || checked["selection"] != delivery["bundle"]["selection"]
    {
        return Err(HELD.into());
    }
    Ok(())
}
