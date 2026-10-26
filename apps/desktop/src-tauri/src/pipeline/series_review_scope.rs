//! A reviewer may cite the supplied previous exit as read-only continuity
//! evidence. It does not gain write permission over another episode or asset.
use super::*;

const OUTSIDE: &str = "SERIES_REVIEW_ISSUE_OUTSIDE_SCOPE";
const HELD: &str = "SERIES_REVIEW_ENTRY_REVALIDATION_HELD";

pub(super) fn entry_source_refs(
    source: &LockedSource,
    episode: &SourceEpisode,
    entry: &Value,
) -> Result<HashSet<String>, String> {
    if entry.is_null() {
        return Ok(HashSet::new());
    }
    let position = source
        .episodes
        .iter()
        .position(|e| e.id == episode.id)
        .ok_or(OUTSIDE)?;
    let previous = position
        .checked_sub(1)
        .and_then(|i| source.episodes.get(i))
        .ok_or(OUTSIDE)?;
    if !entry.is_object()
        || entry["episodeId"] != previous.id
        || s(entry, "id").is_empty()
        || entry["continuityStale"] == true
        || entry["retired"] == true
    {
        return Err(OUTSIDE.into());
    }
    let refs = string_list(&entry["sourceRefs"])?;
    let mut unique = HashSet::new();
    if refs.is_empty()
        || refs.iter().any(|id| {
            !unique.insert(id.clone())
                || !source.rows.iter().any(|row| {
                    row.id == *id
                        && row.kind == "body"
                        && row.episode_id.as_deref() == Some(&previous.id)
                })
        })
    {
        return Err(OUTSIDE.into());
    }
    Ok(unique)
}

/// Revalidate the exact rejected raw against its own frozen candidate/entry.
/// This proof cannot create a request, modify a candidate or declare it passed.
pub(crate) fn revalidate_review_entry_scope_response(
    request: &TextRequest,
    raw: &Value,
    frozen_input: &Value,
) -> Result<Value, String> {
    validate_profile(frozen_input)?;
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| request.input["episodeId"] == e.id)
        .ok_or(HELD)?;
    if request.stage != "seriesReview"
        || request.skill != "series-reviewer"
        || request.input["role"] != "reviewer"
        || request.input["freshContext"] != true
        || !(0..=MAX_REPAIRS)
            .any(|n| request.input["executionRoleId"] == format!("{}/reviewer/{n}", episode.id))
        || request.input.get("validationRepair").is_some()
        || request.input.get("seriesAssetSupplement").is_some()
        || request.schema != review_schema()
        || !scope(frozen_input, &source)?.contains(&episode.id)
    {
        return Err(HELD.into());
    }
    validate_role_packet(request, frozen_input, &source, episode)?;
    let packet = &request.input["packet"];
    let entry = packet_value(packet, "entry")?;
    let assets = packet_value(packet, "assets")?.as_array().ok_or(HELD)?;
    let candidate = json!({"schemaVersion":"series-episode-candidate/v1",
        "plan":packet_value(packet,"plan")?,"selection":packet_value(packet,"selection")?,
        "draft":packet_value(packet,"draft")?});
    if request.input["candidateSha256"] != hash(&candidate)
        || validate_candidate(&candidate, &source, episode, assets, frozen_input)? != candidate
        || validate_review(raw, &candidate, &source, episode, &Value::Null)
            .err()
            .as_deref()
            != Some(OUTSIDE)
    {
        return Err(HELD.into());
    }
    let checked = validate_review(raw, &candidate, &source, episode, entry)?;
    if checked != *raw {
        return Err(HELD.into());
    }
    let mut allowed: Vec<_> = entry_source_refs(&source, episode, entry)?
        .into_iter()
        .collect();
    allowed.sort();
    Ok(
        json!({"version":"series-review-entry-scope-revalidation/v1",
        "sourceSha256":source.sha256,"episodeId":episode.id,
        "requestSha256":digest(&serde_json::to_string(request).map_err(|e|e.to_string())?),
        "rawSha256":hash(raw),"candidateSha256":hash(&candidate),"entrySha256":hash(entry),
        "allowedEntrySourceRefs":allowed,"conclusion":raw["conclusion"],"originalRawChanged":false}),
    )
}

#[cfg(test)]
#[path = "series_review_scope_tests.rs"]
mod tests;
