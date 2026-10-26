//! Local review repairs return only schema-writable fields. Program-owned
//! plan metadata and sound-source identities remain the exact frozen input.
use super::*;

pub(super) const PLAN_CHANGED: &str = "SERIES_REPAIR_PLAN_CHANGED_WITHOUT_REVIEW";
const SCOPE_HELD: &str = "SERIES_REPAIR_FROZEN_PLAN_SCOPE_MISMATCH";
const UNIT_CHANGED: &str = "SERIES_REPAIR_CHANGED_UNAFFECTED_UNIT";
const SOUND_SCOPE: &str = "SERIES_REPAIR_FROZEN_SOUND_SOURCE_SCOPE_MISMATCH";

/// Project the original input, never the received output. Comparing the whole
/// received plan with this value also rejects every undeclared output field.
pub(super) fn schema_projection(value: &Value, schema: &Value) -> Result<Value, String> {
    match schema["type"].as_str() {
        Some("object") => {
            let fields = value.as_object().ok_or(SCOPE_HELD)?;
            let properties = schema["properties"].as_object().ok_or(SCOPE_HELD)?;
            if schema["additionalProperties"] != false {
                return Err(SCOPE_HELD.into());
            }
            for key in schema["required"].as_array().ok_or(SCOPE_HELD)? {
                if !fields.contains_key(key.as_str().ok_or(SCOPE_HELD)?) {
                    return Err(SCOPE_HELD.into());
                }
            }
            let mut out = Map::new();
            for (key, property) in properties {
                if let Some(item) = fields.get(key) {
                    out.insert(key.clone(), schema_projection(item, property)?);
                }
            }
            Ok(Value::Object(out))
        }
        Some("array") => value
            .as_array()
            .ok_or(SCOPE_HELD)?
            .iter()
            .map(|item| schema_projection(item, &schema["items"]))
            .collect::<Result<Vec<_>, String>>()
            .map(Value::Array),
        Some("string") if value.is_string() => Ok(value.clone()),
        Some("number") if value.is_number() => Ok(value.clone()),
        Some("integer") if value.is_i64() || value.is_u64() => Ok(value.clone()),
        Some("boolean") if value.is_boolean() => Ok(value.clone()),
        Some("null") if value.is_null() => Ok(value.clone()),
        _ => Err(SCOPE_HELD.into()),
    }
}

pub(super) fn restore_frozen_plan(
    request: &TextRequest,
    raw: &Value,
    previous: &Value,
) -> Result<Value, String> {
    if request.stage != "seriesRepair"
        || request.skill != "series-repair"
        || request.input["role"] != "repair"
        || packet_value(&request.input["packet"], "plan")? != &previous["plan"]
        || packet_value(&request.input["packet"], "selection")? != &previous["selection"]
        || packet_value(&request.input["packet"], "draft")? != &previous["draft"]
    {
        return Err(SCOPE_HELD.into());
    }
    if raw["plan"] != schema_projection(&previous["plan"], &request.schema["properties"]["plan"])?
        || raw["selection"] != previous["selection"]
    {
        return Err(PLAN_CHANGED.into());
    }
    let mut restored = raw.clone();
    restored["plan"] = previous["plan"].clone();
    Ok(restored)
}

fn restore_frozen_units(
    request: &TextRequest,
    raw: &Value,
    restored: &mut Value,
    previous: &Value,
    issues: &Value,
) -> Result<(), String> {
    if packet_value(&request.input["packet"], "issues")? != issues
        || schema_projection(raw, &request.schema)? != *raw
    {
        return Err(SCOPE_HELD.into());
    }
    let mut allowed = HashSet::new();
    for issue in issues.as_array().ok_or(SCOPE_HELD)? {
        if issue["severity"] == "must_fix" {
            allowed.extend(string_list(&issue["unitIds"])?);
        }
    }
    let unit_schema = &request.schema["properties"]["draft"]["properties"]["units"]["items"];
    let received = raw["draft"]["units"].as_array().ok_or(SCOPE_HELD)?;
    let target = restored["draft"]["units"]
        .as_array_mut()
        .ok_or(SCOPE_HELD)?;
    let mut protected_position = None;
    for frozen in previous["draft"]["units"].as_array().ok_or(SCOPE_HELD)? {
        let positions: Vec<_> = received
            .iter()
            .enumerate()
            .filter_map(|(i, unit)| (unit["id"] == frozen["id"]).then_some(i))
            .collect();
        if !allowed.contains(s(frozen, "id")) {
            if positions.len() != 1
                || protected_position.is_some_and(|last| positions[0] <= last)
                || received[positions[0]] != schema_projection(frozen, unit_schema)?
            {
                return Err(UNIT_CHANGED.into());
            }
            protected_position = Some(positions[0]);
            // Only an exact writable projection may recover the full original
            // unit. Revalidation below still recomputes all derived fields.
            target[positions[0]] = frozen.clone();
        } else if frozen["soundSources"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        {
            // AUDIO references were removed from assetRefs during the original
            // materialization, and are not writable in this frozen schema.
            // Preserve their identity even in a repaired unit. The normal
            // validator checks the exact asset and current source scope; never
            // guess another unit if a repair has removed or split this ID.
            if positions.len() != 1 {
                return Err(SOUND_SCOPE.into());
            }
            target[positions[0]]["soundSources"] = frozen["soundSources"].clone();
        }
    }
    Ok(())
}

pub(super) fn validate_scoped_candidate(
    request: &TextRequest,
    raw: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    input: &Value,
    scope: Option<(&Value, &Value)>,
    episode_supply: Option<&Value>,
) -> Result<Value, String> {
    let mut candidate = if let Some((previous, issues)) = scope {
        let mut restored = restore_frozen_plan(request, raw, previous)?;
        restore_frozen_units(request, raw, &mut restored, previous, issues)?;
        restored
    } else {
        raw.clone()
    };
    super::frame_render::restore_for_request(request, &mut candidate, input, scope)?;
    let checked =
        validate_candidate_with_supply(&candidate, source, episode, assets, input, episode_supply)?;
    if let Some((previous, issues)) = scope {
        assert_local_repair(previous, &checked, issues)?;
        super::controller_review::assert_shots(request, previous, &checked)?;
    }
    Ok(checked)
}

/// Narrow historical SDK revalidation. The caller must prove the exact saved
/// repair and accepted independent-review receipts. This grants no retry and
/// never accepts a validation-repair successor or an unknown submission.
pub(crate) fn revalidate_review_repair_response(
    request: &TextRequest,
    raw: &Value,
    reviewer: &TextRequest,
    review_raw: &Value,
    frozen_input: &Value,
) -> Result<Value, String> {
    validate_profile(frozen_input)?;
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|episode| request.input["episodeId"] == episode.id)
        .ok_or(SCOPE_HELD)?;
    let role_id = s(&request.input, "executionRoleId");
    let iteration = (1..=MAX_REPAIRS)
        .find(|n| role_id == format!("{}/repair/review-{n}", episode.id))
        .ok_or(SCOPE_HELD)?;
    if request.input.get("validationRepair").is_some()
        || request.input.get("seriesAssetSupplement").is_some()
        || reviewer.stage != "seriesReview"
        || reviewer.skill != "series-reviewer"
        || reviewer.input["role"] != "reviewer"
        || reviewer.input["executionRoleId"] != format!("{}/reviewer/{}", episode.id, iteration - 1)
        || reviewer.model != request.model
        || reviewer.reasoning != request.reasoning
        || reviewer.schema != review_schema()
        || !scope(frozen_input, &source)?.contains(&episode.id)
    {
        return Err(SCOPE_HELD.into());
    }
    validate_role_packet(request, frozen_input, &source, episode)?;
    validate_role_packet(reviewer, frozen_input, &source, episode)?;
    for kind in [
        "source",
        "direction",
        "state",
        "assets",
        "entry",
        "plan",
        "selection",
        "draft",
    ] {
        if packet_value(&request.input["packet"], kind)?
            != packet_value(&reviewer.input["packet"], kind)?
        {
            return Err(SCOPE_HELD.into());
        }
    }
    let assets = packet_value(&request.input["packet"], "assets")?
        .as_array()
        .ok_or(SCOPE_HELD)?;
    if request.schema != candidate_schema(&source, episode, assets, fantasy_mode(frozen_input)?) {
        return Err(SCOPE_HELD.into());
    }
    let before = json!({"schemaVersion":"series-episode-candidate/v1",
        "plan":packet_value(&request.input["packet"], "plan")?,
        "selection":packet_value(&request.input["packet"], "selection")?,
        "draft":packet_value(&request.input["packet"], "draft")?});
    // Existing-supply migrations require their own complete evidence; never
    // infer permission from a card's presence if strict current scope rejects it.
    if validate_candidate(&before, &source, episode, assets, frozen_input)? != before
        || reviewer.input["candidateSha256"] != hash(&before)
        || validate_review(review_raw, &before, &source, episode,
            packet_value(&reviewer.input["packet"], "entry")?)?["conclusion"] != "revise"
        || packet_value(&request.input["packet"], "issues")? != &review_raw["issues"]
    {
        return Err(SCOPE_HELD.into());
    }
    let original_materialization = validate_candidate(raw, &source, episode, assets, frozen_input)?;
    if assert_local_repair(&before, &original_materialization, &review_raw["issues"])
        .err()
        .as_deref()
        != Some(PLAN_CHANGED)
    {
        return Err(SCOPE_HELD.into());
    }
    let checked = validate_scoped_candidate(
        request,
        raw,
        &source,
        episode,
        assets,
        frozen_input,
        Some((&before, &review_raw["issues"])),
        None,
    )?;
    Ok(
        json!({"version":"series-repair-frozen-plan-revalidation/v1",
        "sourceSha256":source.sha256,"episodeId":episode.id,
        "reviewRequestSha256":digest(&serde_json::to_string(reviewer).map_err(|e|e.to_string())?),
        "reviewRawSha256":hash(review_raw),"requestSha256":digest(&serde_json::to_string(request).map_err(|e|e.to_string())?),
        "rawSha256":hash(raw),"originalCandidateSha256":hash(&before),
        "originalPlanSha256":hash(&before["plan"]),"schemaWritablePlanSha256":hash(&raw["plan"]),
        "validatedCandidateSha256":hash(&checked),"originalRawChanged":false}),
    )
}
