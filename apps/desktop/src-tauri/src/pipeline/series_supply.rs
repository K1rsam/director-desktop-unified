//! V2 adds source-proven use of an existing identity. It never duplicates a
//! known asset merely because that asset was omitted from episode applicability.
use super::*;
pub(super) const VERSION: &str = "series-asset-supplement/v2";
pub(super) fn enabled(context: &Value) -> bool {
    context["version"] == "series-supplement-identities/v2" || supply_revision::enabled(context)
}
pub(super) fn version(context: &Value) -> &'static str {
    if supply_revision::enabled(context) {
        supply_revision::VERSION
    } else {
        VERSION
    }
}
pub(crate) fn supplement_supply_catalogue(assets: &Value) -> Value {
    let keys = [
        "id",
        "variantId",
        "identityId",
        "baseAssetId",
        "parentAssetId",
        "name",
        "aliases",
        "kind",
        "type",
        "version",
        "applicability",
        "scope",
        "designRevision",
        "designStatus",
        "importName",
        "filename",
        "placeholderOrdinal",
        "materialRole",
    ];
    json!(assets
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let mut card = Map::new();
            for key in keys {
                if let Some(value) = a.get(key) {
                    card.insert(key.into(), value.clone());
                }
            }
            let mut projected = Value::Object(card);
            // PROP + preparation.mode=AUDIO is a source voice identity, not
            // an image whose PNG mapping is missing. Preserve its actual mode
            // when compacting would erase the only voice-only discriminator.
            // Limit this addition to previously unfreezable unmapped records:
            // successful historical v2 catalogue hashes must stay unchanged.
            if !export::is_voice_only(&projected)
                && export::is_voice_only(a)
                && supplement_identity_reservation(a).is_none()
            {
                projected["preparation"] = json!({"mode":a["preparation"]["mode"]});
            }
            projected
        })
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn series_supply_audio_prop_projection_keeps_voice_mode_and_forbids_image_use() {
        let audio = json!({"id":"system-voice","variantId":"base","kind":"PROP","name":"系统声音",
            "preparation":{"mode":"AUDIO","note":"源有系统音，不生成实体图"},"version":2});
        let compact = supplement_supply_catalogue(&json!([audio]));
        assert_eq!(compact[0]["kind"], "PROP", "do not rewrite source category");
        assert_eq!(compact[0]["preparation"], json!({"mode":"AUDIO"}));
        assert!(export::is_voice_only(&compact[0]));
        assert!(supplement_identity_reservation(&compact[0]).is_none());
        assert_eq!(
            supplement_supply_catalogue(&compact),
            compact,
            "projection is stable when validating a frozen packet"
        );
        let draft = json!({"units":[{"imageBindings":["system-voice"],"assetRefs":[{"assetId":"system-voice","variantId":"base","use":"prop"}]}]});
        assert!(
            export::validate_image_bindings(&draft, compact.as_array().unwrap())
                .unwrap_err()
                .starts_with("SERIES_VOICE_ONLY_IMAGE_FORBIDDEN")
        );
        assert!(compact[0].get("filename").is_none());
        assert!(compact[0].get("placeholderOrdinal").is_none());
    }

    #[test]
    fn series_supply_voice_mode_fix_leaves_previously_freezable_catalogue_bytes_unchanged() {
        let prior = json!([
            {"id":"visible","variantId":"base","kind":"PROP","name":"镜子","version":1,"importName":"镜子","filename":"PR-镜子.png","placeholderOrdinal":3},
            {"id":"declared-voice","variantId":"base","kind":"SE","name":"系统音","version":1},
            {"id":"historic-mapped","variantId":"base","kind":"PROP","name":"历史映射","version":1,"importName":"历史映射","filename":"PR-历史映射.png","placeholderOrdinal":4}
        ]);
        let mut full = prior.clone();
        full[0]["preparation"] = json!({"mode":"IMAGE"});
        full[1]["preparation"] = json!({"mode":"AUDIO"});
        full[2]["preparation"] = json!({"mode":"AUDIO"});
        assert_eq!(supplement_supply_catalogue(&full), prior);
        assert_eq!(hash(&supplement_supply_catalogue(&full)), hash(&prior));
        let visible_without_mapping = json!([{"id":"visual","variantId":"base","kind":"PROP","name":"声音也可能是名称","preparation":{"mode":"IMAGE"}}]);
        let projected = supplement_supply_catalogue(&visible_without_mapping);
        assert!(
            !export::is_voice_only(&projected[0]),
            "never infer voice-only from a name or absent image fields"
        );
    }
}
pub(crate) fn supplement_supply_card(asset: &Value) -> Value {
    let mut result = design::card(asset, false);
    if let Some(value) = asset.get("adoptedDesign") {
        result["adoptedDesign"] = value.clone();
    }
    result
}
pub(crate) fn supplement_design_usable(asset: &Value, source_sha: &str) -> bool {
    let d = &asset["adoptedDesign"];
    (d["full"].as_str().is_some_and(|v| !v.trim().is_empty())
        || d["full"].as_object().is_some_and(|v| !v.is_empty()))
        && !matches!(
            s(asset, "designStatus"),
            "needs_review" | "stale" | "rejected"
        )
        && d["assetId"] == asset["id"]
        && d["variantId"] == asset["variantId"]
        && d["sourceSha256"] == source_sha
        && d["status"] == "ADOPTED"
        && d["provenance"] == "ADOPTED_DESIGN_NOT_SOURCE_FACT"
        && d["adoptionBasis"].is_object()
        && d["designRevision"].as_u64().is_some_and(|v| v > 0)
}
pub(crate) fn supplement_supply_state(asset: &Value, source_sha: &str) -> Value {
    json!({"assetId":asset["id"],"assetVersion":asset["version"],"designRevision":asset["designRevision"].as_u64().unwrap_or(0),
        "adoptedDesignSha256":hash(&asset["adoptedDesign"]),"supplyCardSha256":hash(&supplement_supply_card(asset)),"designUsable":supplement_design_usable(asset,source_sha)})
}
pub(super) fn validate_states(context: &Value, known: &Value) -> Result<(), String> {
    if context["contractVersion"] != version(context) {
        return Err("SERIES_SUPPLY_CONTEXT_VERSION".into());
    }
    let states = context["assetStates"]
        .as_array()
        .ok_or("SERIES_SUPPLY_STATES_REQUIRED")?;
    let known = known
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_CATALOGUE_REQUIRED")?;
    if states.len() != known.len() {
        return Err("SERIES_SUPPLY_STATE_SCOPE".into());
    }
    let mut ids = HashSet::new();
    let mut prior = "";
    for state in states {
        let id = s(state, "assetId");
        if id.is_empty()
            || id < prior
            || !ids.insert(id)
            || !known.iter().any(|a| a["id"] == id)
            || state["assetVersion"].as_u64().is_none_or(|v| v == 0)
            || state["designRevision"].as_u64().is_none()
            || state["designUsable"].as_bool().is_none()
            || s(state, "adoptedDesignSha256").len() != 64
            || s(state, "supplyCardSha256").len() != 64
        {
            return Err("SERIES_SUPPLY_STATE_INVALID".into());
        }
        prior = id;
    }
    Ok(())
}
pub(super) fn candidate_schema(mut base: Value) -> Value {
    base["properties"]["schemaVersion"] = choices(&["series-asset-supplement-candidate/v2"]);
    base["properties"]["existingSupply"] = array(obj(
        json!({"assetId":string(),"designMode":choices(&["reuse","complete"]),
        "evidence":array(obj(json!({"rowId":string(),"quote":string()}))),"rationale":string(),"full":string(),"brief":string(),
        "sourceFacts":array(obj(json!({"statement":string(),"sourceRefs":array(string())}))),"productionChoices":array(string()),
        "designDependencies":array(obj(json!({"assetId":string(),"designRevision":{"type":"integer","minimum":1}})))}),
    ));
    base["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("existingSupply"));
    base
}
pub(super) fn review_schema(mut base: Value) -> Value {
    base["properties"]["schemaVersion"] = choices(&["series-asset-supplement-review/v2"]);
    base["properties"]["reviewedExistingAssetIds"] = array(string());
    base["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("reviewedExistingAssetIds"));
    base
}
fn selections(candidate: &Value) -> Result<&Vec<Value>, String> {
    candidate["existingSupply"]
        .as_array()
        .ok_or_else(|| "SERIES_SUPPLY_SELECTION_REQUIRED".into())
}
fn state<'a>(context: &'a Value, id: &str) -> Result<&'a Value, String> {
    context["assetStates"]
        .as_array()
        .ok_or("SERIES_SUPPLY_STATES_REQUIRED")?
        .iter()
        .find(|a| a["assetId"] == id)
        .ok_or_else(|| "SERIES_SUPPLY_STATE_MISSING".into())
}
pub(super) fn ids(candidate: &Value) -> Result<Vec<Value>, String> {
    Ok(selections(candidate)?
        .iter()
        .map(|a| a["assetId"].clone())
        .collect())
}
pub(super) fn episode_use(candidate: &Value, episode: &str) -> Result<Value, String> {
    Ok(
        json!({"version":"series-existing-episode-supply/v1","episodeId":episode,"items":selections(candidate)?.iter().map(|c|json!({"assetId":c["assetId"],"designMode":c["designMode"],"evidence":c["evidence"],"rationale":c["rationale"]})).collect::<Vec<_>>()}),
    )
}
/// A validation-only view of already reviewed episode supply. Do not persist
/// this scope onto the asset: its original applicability is historical data.
/// Callers must validate the supply transition or whole delivery first.
pub(super) fn scoped_assets(
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    usage: Option<&Value>,
) -> Result<Vec<Value>, String> {
    let mut projected = assets.to_vec();
    let Some(usage) = usage.filter(|v| !v.is_null()) else {
        return Ok(projected);
    };
    if usage["version"] != "series-existing-episode-supply/v1" || usage["episodeId"] != episode.id {
        return Err("SERIES_SUPPLY_SCOPE_EPISODE_MISMATCH".into());
    }
    let mut seen = HashSet::new();
    for item in usage["items"]
        .as_array()
        .ok_or("SERIES_SUPPLY_SCOPE_ITEMS")?
    {
        let id = s(item, "assetId");
        if id.is_empty() || !seen.insert(id) {
            return Err("SERIES_SUPPLY_SCOPE_IDENTITY".into());
        }
        let refs = super::supplement::evidence_refs(&item["evidence"], source, episode)?;
        let mut scenes = Vec::new();
        for row_id in refs {
            let row = source.rows.iter().find(|r| r.id == row_id).unwrap();
            let scene = row
                .scene_id
                .as_deref()
                .ok_or("SERIES_SUPPLY_SCOPE_SCENE_REQUIRED")?;
            if !source
                .scenes
                .iter()
                .any(|s| s.id == scene && s.episode_id == episode.id)
            {
                return Err("SERIES_SUPPLY_SCOPE_SCENE_UNKNOWN".into());
            }
            if !scenes.iter().any(|s: &String| s == scene) {
                scenes.push(scene.to_owned());
            }
        }
        let card = projected
            .iter_mut()
            .find(|a| a["id"] == id)
            .ok_or("SERIES_SUPPLY_SCOPE_ASSET_MISSING")?;
        card["applicability"] = json!({"episodes":[episode.id],"scenes":scenes});
        card["scope"] = json!({"kind":"SCENES","scenes":scenes.iter().map(|scene|json!({"episodeId":episode.id,"sceneId":scene})).collect::<Vec<_>>()});
    }
    Ok(projected)
}
pub(super) fn validate_selection(
    candidate: &Value,
    before: &[Value],
    known: &Value,
    context: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    revisions: &[Value],
    allow_fact_summary: bool,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for choice in selections(candidate)? {
        let id = s(choice, "assetId");
        let state = state(context, id)?;
        let asset = known
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .ok_or("SERIES_SUPPLY_UNKNOWN_ID")?;
        if !seen.insert(id)
            || before.iter().any(|a| a["id"] == id)
            || export::is_voice_only(asset)
            || asset["materialRole"] == "MATERIAL_SOURCE_ONLY"
            || s(choice, "rationale").trim().is_empty()
        {
            return Err("SERIES_SUPPLY_SCOPE_INVALID".into());
        }
        super::supplement::evidence_refs(&choice["evidence"], source, episode)?;
        if !candidate["resolutions"]
            .as_array()
            .ok_or("SERIES_SUPPLEMENT_RESOLUTIONS_REQUIRED")?
            .iter()
            .any(|r| {
                r["existingAssetIds"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|v| v == id))
            })
        {
            return Err("SERIES_SUPPLY_UNUSED_IDENTITY".into());
        }
        match s(choice, "designMode") {
            "reuse" => {
                if state["designUsable"] != true
                    || !s(choice, "full").is_empty()
                    || !s(choice, "brief").is_empty()
                    || choice["sourceFacts"] != json!([])
                    || choice["productionChoices"] != json!([])
                    || choice["designDependencies"] != json!([])
                {
                    return Err("SERIES_SUPPLY_REUSE_MUST_NOT_REWRITE_DESIGN".into());
                }
            }
            "complete" => {
                if state["designUsable"] != false
                    || s(choice, "full").trim().is_empty()
                    || s(choice, "brief").trim().is_empty()
                    || choice["full"] == choice["brief"]
                {
                    return Err("SERIES_SUPPLY_COMPLETE_DESIGN_REQUIRED".into());
                }
                for fact in choice["sourceFacts"]
                    .as_array()
                    .ok_or("SERIES_SUPPLY_FACTS_REQUIRED")?
                {
                    let fact_refs = string_list(&fact["sourceRefs"])?;
                    if fact_refs.is_empty()
                        || fact_refs.iter().any(|id| {
                            !source.rows.iter().any(|r| {
                                r.id == *id
                                    && r.kind == "body"
                                    && r.episode_id.as_deref() == Some(&episode.id)
                            })
                        })
                        || s(fact, "statement").trim().is_empty()
                        || (!allow_fact_summary
                            && !source.rows.iter().any(|r| {
                                fact_refs.contains(&r.id) && r.text.contains(s(fact, "statement"))
                            }))
                    {
                        return Err("SERIES_SUPPLY_FACT_NOT_SOURCE_QUOTE".into());
                    }
                }
                string_list(&choice["productionChoices"])?;
                for dep in choice["designDependencies"]
                    .as_array()
                    .ok_or("SERIES_SUPPLY_DEPENDENCIES_REQUIRED")?
                {
                    if !before.iter().any(|a| {
                        a["id"] == dep["assetId"]
                            && design::adopted(a, source)
                            && supply_revision::dependency_version(a, revisions)
                                .is_some_and(|v| Some(v) == dep["designRevision"].as_u64())
                    }) {
                        return Err("SERIES_SUPPLY_DEPENDENCY_NOT_ADOPTED".into());
                    }
                }
            }
            _ => return Err("SERIES_SUPPLY_DESIGN_MODE".into()),
        }
    }
    Ok(())
}
pub(super) fn source_cards(
    candidate: &Value,
    assets: &[Value],
    context: &Value,
    source: &LockedSource,
) -> Result<Value, String> {
    let mut cards = Vec::new();
    for choice in selections(candidate)? {
        let id = s(choice, "assetId");
        let state = state(context, id)?;
        let a = assets
            .iter()
            .find(|a| a["id"] == id)
            .ok_or("SERIES_SUPPLY_SOURCE_MISSING")?;
        let mut card = supplement_supply_card(a);
        card["version"] = state["assetVersion"].clone();
        validate_source_card(&card, context, source)?;
        cards.push(card);
    }
    Ok(json!(cards))
}
fn validate_source_card(
    card: &Value,
    context: &Value,
    source: &LockedSource,
) -> Result<(), String> {
    let state = state(context, s(card, "id"))?;
    if hash(card) != state["supplyCardSha256"]
        || hash(&card["adoptedDesign"]) != state["adoptedDesignSha256"]
        || card["version"] != state["assetVersion"]
        || card["designRevision"].as_u64().unwrap_or(0) != state["designRevision"].as_u64().unwrap()
        || supplement_design_usable(card, &source.sha256)
            != state["designUsable"].as_bool().unwrap()
    {
        return Err("SERIES_SUPPLY_SOURCE_CARD_CHANGED".into());
    }
    Ok(())
}
pub(super) fn validate_source_cards(
    candidate: &Value,
    sources: &Value,
    context: &Value,
    source: &LockedSource,
) -> Result<(), String> {
    let selections = selections(candidate)?;
    let sources = sources.as_array().ok_or("SERIES_SUPPLY_SOURCES_REQUIRED")?;
    if selections.len() != sources.len() {
        return Err("SERIES_SUPPLY_SOURCE_SCOPE".into());
    }
    for (choice, card) in selections.iter().zip(sources) {
        if choice["assetId"] != card["id"] {
            return Err("SERIES_SUPPLY_SOURCE_ORDER".into());
        }
        validate_source_card(card, context, source)?;
    }
    Ok(())
}
pub(super) fn supplied(
    candidate: &Value,
    sources: &Value,
    context: &Value,
    source: &LockedSource,
    design_sha: &Value,
    review_sha: &Value,
) -> Result<Vec<Value>, String> {
    let selections = selections(candidate)?;
    let sources = sources.as_array().ok_or("SERIES_SUPPLY_SOURCES_REQUIRED")?;
    if selections.len() != sources.len() {
        return Err("SERIES_SUPPLY_SOURCE_SCOPE".into());
    }
    let mut result = Vec::new();
    for (choice, card) in selections.iter().zip(sources) {
        if choice["assetId"] != card["id"] {
            return Err("SERIES_SUPPLY_SOURCE_ORDER".into());
        }
        validate_source_card(card, context, source)?;
        let mut next = card.clone();
        if choice["designMode"] == "complete" {
            let revision = state(context, s(card, "id"))?["designRevision"]
                .as_u64()
                .unwrap()
                + 1;
            let mut refs: Vec<_> = choice["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["rowId"].clone())
                .collect();
            for fact in choice["sourceFacts"].as_array().unwrap() {
                for id in fact["sourceRefs"].as_array().unwrap() {
                    if !refs.contains(id) {
                        refs.push(id.clone());
                    }
                }
            }
            next["designRevision"] = json!(revision);
            next["designStatus"] = json!("adopted");
            next["adoptedDesign"] = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":card["id"],"variantId":card["variantId"],"designRevision":revision,
                "sourceSha256":source.sha256,"sourceRefs":refs,"sourceFacts":choice["sourceFacts"],"productionChoices":choice["productionChoices"],"full":choice["full"],"brief":choice["brief"],"designDependencies":choice["designDependencies"],
                "provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT","adoptionBasis":{"kind":"delegated-text-review","supplementVersion":version(context),"designRequestSha256":design_sha,"reviewRequestSha256":review_sha,"candidateSha256":hash(candidate)}});
        }
        if !design::adopted(&next, source) {
            return Err("SERIES_SUPPLY_FULL_DESIGN_NOT_ADOPTED".into());
        }
        result.push(next);
    }
    Ok(result)
}
pub(super) fn review(raw: &Value, candidate: &Value) -> Result<(), String> {
    if raw["reviewedExistingAssetIds"] != json!(ids(candidate)?) {
        return Err("SERIES_SUPPLY_REVIEW_SCOPE".into());
    }
    Ok(())
}
pub(super) fn install_in_memory(assets: &mut [Value], supplied: &[Value]) -> Result<(), String> {
    for card in supplied {
        let original = assets
            .iter_mut()
            .find(|a| a["id"] == card["id"])
            .ok_or("SERIES_SUPPLY_SOURCE_MISSING")?;
        for field in ["adoptedDesign", "designRevision", "designStatus", "version"] {
            if let Some(value) = card.get(field) {
                original[field] = value.clone();
            }
        }
    }
    Ok(())
}
