//! Same-identity design revisions. These are candidates until independent review.
use super::*;

pub(super) const CANDIDATE: &str = "series-supply-correction-candidate/v1";
pub(super) const REVIEW: &str = "series-supply-correction-review/v1";
pub(super) const VERSION: &str = "series-asset-supplement/v3";
pub(super) const INPUT_FIELD: &str = "seriesAssetSupplementVersion";
pub(super) const NORMAL_CANDIDATE: &str = "series-asset-supplement-candidate/v3";
pub(super) const NORMAL_REVIEW: &str = "series-asset-supplement-review/v3";

pub(super) fn enabled(context: &Value) -> bool {
    context["version"] == "series-supplement-identities/v3"
}
pub(super) fn dependency_version(asset: &Value, revisions: &[Value]) -> Option<u64> {
    asset["adoptedDesign"]["designRevision"]
        .as_u64()?
        .checked_add(u64::from(revisions.iter().any(|r| {
            r["assetId"] == asset["id"] && r["variantId"] == asset["variantId"]
        })))
}
pub(super) fn append_schema(mut schema: Value, ids: Vec<Value>) -> Value {
    let id = if ids.is_empty() {
        string()
    } else {
        json!({"type":"string","enum":ids})
    };
    schema["properties"]["revisions"] = array(obj(json!({
        "assetId":id,
        "variantId":string(),"baseAssetVersion":{"type":"integer","minimum":1},
        "baseDesignRevision":{"type":"integer","minimum":1},"baseDesignSha256":string(),
        "unresolvedIndexes":array(json!({"type":"integer","minimum":0})),
        "sourceRefs":array(string()),"evidence":array(obj(json!({"rowId":string(),"quote":string()}))),
        "rationale":string(),"changeSummary":string(),"preservedConstraints":string(),
        "full":string(),"brief":string(),
        "sourceFacts":array(obj(json!({"statement":string(),"sourceRefs":array(string())}))),
        "productionChoices":array(string()),
        "designDependencies":array(obj(json!({"assetId":string(),"designRevision":{"type":"integer","minimum":1}})))
    })));
    schema["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("revisions"));
    schema
}

pub(super) fn schema(identities: &Value, context: &Value) -> Value {
    let mut schema = supplement::effective_candidate_schema(identities);
    schema["properties"]["schemaVersion"] = choices(&[CANDIDATE]);
    let affected = context["authorization"]["affectedAssets"]
        .as_array()
        .unwrap();
    append_schema(
        schema,
        affected.iter().map(|a| a["assetId"].clone()).collect(),
    )
}

pub(super) fn legacy_candidate(candidate: &Value, identities: &Value) -> Value {
    let mut projected = candidate.clone();
    projected.as_object_mut().unwrap().remove("revisions");
    projected["schemaVersion"] = json!(if supply::enabled(identities) {
        "series-asset-supplement-candidate/v2"
    } else {
        "series-asset-supplement-candidate/v1"
    });
    projected
}

pub(super) fn validate(
    candidate: &Value,
    context: &Value,
    origin: &TextRequest,
    raw: &Value,
    known: &Value,
    identities: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    input: &Value,
) -> Result<Vec<Value>, String> {
    if candidate["schemaVersion"] != CANDIDATE {
        return Err("SERIES_SUPPLY_CORRECTION_CANDIDATE_VERSION".into());
    }
    let revisions = candidate["revisions"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("SERIES_SUPPLY_CORRECTION_REVISION_REQUIRED")?;
    // This is exactly the deterministic row set placed in the correction's
    // frozen designSourceRows packet, not the full project's available text.
    let authorized = context["authorization"]["affectedAssets"]
        .as_array()
        .unwrap();
    let before = supplement::before(origin)?;
    let targets: Vec<_> = before
        .iter()
        .filter(|a| authorized.iter().any(|b| b["assetId"] == a["id"]))
        .cloned()
        .collect();
    let rows = design_source::rows(input, source, episode, &targets)?;
    let added = supplement::validate_candidate_with_revisions(
        &legacy_candidate(candidate, identities),
        origin,
        raw,
        known,
        identities,
        source,
        episode,
        revisions,
        Some(&rows),
    )?;
    let allowed_keys = supply_correction::allowed_keys(context)?;
    if candidate["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| !allowed_keys.contains(&p["key"]))
    {
        return Err("SERIES_SUPPLY_CORRECTION_NEW_PROPOSAL_FORBIDDEN".into());
    }
    if candidate["revisions"].as_array().is_none_or(Vec::is_empty) {
        return Err("SERIES_SUPPLY_CORRECTION_REVISION_REQUIRED".into());
    }
    validate_revisions(candidate, before, &rows, source, Some(authorized), true)?;
    Ok(added)
}

/// Shared content checks. Normal v3 does not manufacture a correction context
/// or an authorization: same-identity changes consume its ordinary budget.
pub(super) fn validate_revisions(
    candidate: &Value,
    before: &[Value],
    rows: &[SourceRow],
    source: &LockedSource,
    authorized: Option<&Vec<Value>>,
    allow_fact_summary: bool,
) -> Result<(), String> {
    let revisions = candidate["revisions"]
        .as_array()
        .ok_or("SERIES_DESIGN_REVISIONS_REQUIRED")?;
    let mut seen = HashSet::new();
    for revision in revisions {
        let binding = authorized
            .map(|values| {
                values
                    .iter()
                    .find(|a| {
                        a["assetId"] == revision["assetId"]
                            && a["variantId"] == revision["variantId"]
                    })
                    .ok_or("SERIES_SUPPLY_CORRECTION_REVISION_UNAUTHORIZED")
            })
            .transpose()?;
        let old = before
            .iter()
            .find(|a| a["id"] == revision["assetId"] && a["variantId"] == revision["variantId"])
            .ok_or("SERIES_DESIGN_REVISION_NOT_SUPPLIED")?;
        if !seen.insert(s(revision, "assetId"))
            || !design::adopted(old, source)
            || export::is_voice_only(old)
            || revision["baseAssetVersion"] != old["version"]
            || revision["baseDesignRevision"] != old["designRevision"]
            || revision["baseDesignRevision"] != old["adoptedDesign"]["designRevision"]
            || revision["baseDesignSha256"] != hash(&old["adoptedDesign"])
            || old["designRevision"]
                .as_u64()
                .and_then(|v| v.checked_add(1))
                .is_none()
            || binding.is_some_and(|binding| {
                ["baseAssetVersion", "baseDesignRevision", "baseDesignSha256"]
                    .iter()
                    .any(|key| revision[*key] != binding[*key])
            })
        {
            return Err("SERIES_SUPPLY_CORRECTION_REVISION_BASE_CHANGED".into());
        }
        let indexes = revision["unresolvedIndexes"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("SERIES_SUPPLY_CORRECTION_REVISION_ISSUES_REQUIRED")?;
        let mut unique = HashSet::new();
        if indexes.iter().any(|v| {
            !unique.insert(v.to_string())
                || binding.is_some_and(|binding| {
                    !binding["unresolvedIndexes"].as_array().unwrap().contains(v)
                })
                || !candidate["resolutions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| {
                        r["unresolvedIndex"] == *v
                            && r["existingAssetIds"]
                                .as_array()
                                .is_some_and(|ids| ids.contains(&revision["assetId"]))
                    })
        }) {
            return Err("SERIES_SUPPLY_CORRECTION_REVISION_UNUSED".into());
        }
        for field in [
            "full",
            "brief",
            "rationale",
            "changeSummary",
            "preservedConstraints",
        ] {
            if s(revision, field).trim().is_empty() {
                return Err("SERIES_SUPPLY_CORRECTION_COMPLETE_DESIGN_REQUIRED".into());
            }
        }
        if revision["full"] == revision["brief"] || revision["full"] == old["adoptedDesign"]["full"]
        {
            return Err("SERIES_SUPPLY_CORRECTION_NO_DESIGN_CHANGE".into());
        }
        let refs = string_list(&revision["sourceRefs"])?;
        if refs.is_empty() || refs.iter().any(|id| !rows.iter().any(|r| r.id == *id)) {
            return Err("SERIES_SUPPLY_CORRECTION_SOURCE_SCOPE".into());
        }
        let evidence = revision["evidence"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("SERIES_SUPPLY_CORRECTION_SOURCE_EVIDENCE_REQUIRED")?;
        for e in evidence {
            if s(e, "quote").trim().is_empty()
                || !rows.iter().any(|r| {
                    r.id == s(e, "rowId") && refs.contains(&r.id) && r.text.contains(s(e, "quote"))
                })
            {
                return Err("SERIES_SUPPLY_CORRECTION_SOURCE_QUOTE_CHANGED".into());
            }
        }
        if binding.is_some_and(|binding| {
            !evidence.iter().any(|e| {
                binding["sourceRefs"]
                    .as_array()
                    .unwrap()
                    .contains(&e["rowId"])
            })
        }) {
            return Err("SERIES_SUPPLY_CORRECTION_AUTHORIZED_USE_MISSING".into());
        }
        for fact in revision["sourceFacts"]
            .as_array()
            .ok_or("SERIES_SUPPLY_CORRECTION_FACTS_REQUIRED")?
        {
            let historic = old["adoptedDesign"]["sourceFacts"]
                .as_array()
                .is_some_and(|f| f.contains(fact));
            if historic {
                continue;
            }
            let fact_refs = string_list(&fact["sourceRefs"])?;
            if s(fact, "statement").trim().is_empty()
                || fact_refs.is_empty()
                || fact_refs.iter().any(|id| !refs.contains(id))
            {
                return Err("SERIES_SUPPLY_CORRECTION_FACT_SOURCE_INVALID".into());
            }
            if !allow_fact_summary
                && !rows.iter().any(|row| {
                    fact_refs.contains(&row.id) && row.text.contains(s(fact, "statement"))
                })
            {
                return Err("SERIES_SUPPLY_CORRECTION_FACT_NOT_SOURCE_QUOTE".into());
            }
        }
        string_list(&revision["productionChoices"])?;
        for dep in revision["designDependencies"]
            .as_array()
            .ok_or("SERIES_SUPPLY_CORRECTION_DEPENDENCIES_REQUIRED")?
        {
            let target = before
                .iter()
                .find(|a| a["id"] == dep["assetId"])
                .ok_or("SERIES_SUPPLY_CORRECTION_DEPENDENCY_UNKNOWN")?;
            let revision_number = dependency_version(target, revisions)
                .ok_or("SERIES_SUPPLY_CORRECTION_DEPENDENCY_NOT_ADOPTED")?;
            if dep["assetId"] == revision["assetId"] || dep["designRevision"] != revision_number {
                return Err("SERIES_SUPPLY_CORRECTION_DEPENDENCY_VERSION".into());
            }
        }
    }
    Ok(())
}

pub(super) fn selected_rows(
    origin: &TextRequest,
    candidate: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
) -> Result<Vec<SourceRow>, String> {
    let revisions = candidate["revisions"]
        .as_array()
        .ok_or("SERIES_DESIGN_REVISIONS_REQUIRED")?;
    let before = supplement::before(origin)?;
    let targets = revisions
        .iter()
        .map(|r| {
            before
                .iter()
                .find(|a| a["id"] == r["assetId"] && a["variantId"] == r["variantId"])
                .cloned()
                .ok_or_else(|| "SERIES_DESIGN_REVISION_NOT_SUPPLIED".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    design_source::rows(
        &json!({"productionScope":origin.input["packet"]["productionScope"]}),
        source,
        episode,
        &targets,
    )
}
pub(super) fn add_source_rows(
    values: &mut [(&str, Value)],
    origin: &TextRequest,
    candidate: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
) -> Result<(), String> {
    let rows = selected_rows(origin, candidate, source, episode)?;
    values
        .iter_mut()
        .find(|(k, _)| *k == "designSourceRows")
        .ok_or("SERIES_DESIGN_SOURCE_ROWS_MISSING")?
        .1 = source_view(&rows);
    Ok(())
}
pub(super) fn revised_before(
    origin: &TextRequest,
    revised: &[Value],
) -> Result<Vec<Value>, String> {
    let mut before = supplement::before(origin)?.clone();
    for revision in revised {
        let card = before
            .iter_mut()
            .find(|a| a["id"] == revision["id"] && a["variantId"] == revision["variantId"])
            .ok_or("SERIES_DESIGN_REVISION_NOT_SUPPLIED")?;
        *card = revision.clone();
    }
    Ok(before)
}
pub(super) fn install(assets: &mut [Value], revised: &[Value]) -> Result<(), String> {
    for revision in revised {
        let card = assets
            .iter_mut()
            .find(|a| a["id"] == revision["id"] && a["variantId"] == revision["variantId"])
            .ok_or("SERIES_DESIGN_REVISION_NOT_SUPPLIED")?;
        for field in ["adoptedDesign", "designRevision", "designStatus"] {
            card[field] = revision[field].clone();
        }
    }
    Ok(())
}

pub(super) fn validate_adoptions(
    input: &Value,
    delivery: &Value,
    origin: &TextRequest,
    revised: &[Value],
    creation: &Value,
    review: &Value,
    identities: &Value,
) -> Result<(), String> {
    let adoptions = delivery["designAdoptions"]
        .as_array()
        .ok_or("SERIES_DESIGN_ADOPTIONS_REQUIRED")?;
    for card in revised {
        let before = supplement::before(origin)?
            .iter()
            .find(|a| a["id"] == card["id"] && a["variantId"] == card["variantId"])
            .ok_or("SERIES_DESIGN_REVISION_NOT_SUPPLIED")?;
        if enabled(identities) {
            let state = identities["assetStates"]
                .as_array()
                .ok_or("SERIES_SUPPLY_STATES_REQUIRED")?
                .iter()
                .find(|s| s["assetId"] == before["id"])
                .ok_or("SERIES_SUPPLY_STATE_MISSING")?;
            if state["adoptedDesignSha256"] != hash(&before["adoptedDesign"]) {
                validate_unpublished_design_predecessor(input, delivery, before)?;
            }
        }
        let revision = card["designRevision"]
            .as_u64()
            .ok_or("SERIES_DESIGN_REVISION_VERSION")?;
        let expected = json!({"assetId":card["id"],"variantId":card["variantId"],"baseDesignRevision":before["designRevision"],
            "designRevision":revision,"adoptedDesign":card["adoptedDesign"],"designReview":{"requestSha256":review["requestSha256"],"resultSha256":review["rawSha256"],"issues":[]}});
        if adoptions
            .iter()
            .filter(|a| a["assetId"] == card["id"] && a["designRevision"] == revision)
            .collect::<Vec<_>>()
            != vec![&expected]
            || card["adoptedDesign"]["adoptionBasis"]["designRequestSha256"]
                != creation["requestSha256"]
        {
            return Err("SERIES_DESIGN_REVISION_ADOPTION_CHAIN_CHANGED".into());
        }
    }
    Ok(())
}

pub(super) fn coverage(candidate: &Value, count: usize) -> Value {
    json!({"reviewedKeys":candidate["proposals"].as_array().unwrap().iter().map(|p|p["key"].clone()).collect::<Vec<_>>(),
        "reviewedIssueIndexes":(0..count).collect::<Vec<_>>(),
        "reviewedExistingAssetIds":candidate["existingSupply"].as_array().into_iter().flatten().map(|a|a["assetId"].clone()).collect::<Vec<_>>(),
        "reviewedRevisionAssetIds":candidate["revisions"].as_array().unwrap().iter().map(|a|a["assetId"].clone()).collect::<Vec<_>>()})
}
pub(super) fn review_schema(candidate: &Value, count: usize) -> Value {
    let nonblank = json!({"type":"string","pattern":r"\S"});
    let mut schema = obj(
        json!({"schemaVersion":choices(&[REVIEW]),"candidateSha256":{"type":"string","enum":[hash(candidate)]},
        "conclusion":choices(&["pass","revise"]),"summary":nonblank,
        "issues":array(obj(json!({"unresolvedIndex":{"type":"integer","minimum":0,"maximum":count.saturating_sub(1)},
            "proposalKey":string(),"assetId":string(),"problem":nonblank,"requestedChange":nonblank})))}),
    );
    for (field, values) in coverage(candidate, count).as_object().unwrap() {
        let items = values.as_array().unwrap();
        let mut item =
            json!({"type":if field == "reviewedIssueIndexes" {"integer"} else {"string"}});
        if !items.is_empty() {
            item["enum"] = values.clone();
        }
        schema["properties"][field] =
            json!({"type":"array","items":item,"minItems":items.len(),"maxItems":items.len()});
        schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!(field));
    }
    schema
}
pub(super) fn review(raw: &Value, candidate: &Value, count: usize) -> Result<(), String> {
    if raw["schemaVersion"] != REVIEW
        || raw["candidateSha256"] != hash(candidate)
        || s(raw, "summary").trim().is_empty()
    {
        return Err("SERIES_SUPPLY_CORRECTION_REVIEW_UNBOUND".into());
    }
    let expected = coverage(candidate, count);
    if expected
        .as_object()
        .unwrap()
        .iter()
        .any(|(k, v)| &raw[k] != v)
    {
        return Err("SERIES_SUPPLY_CORRECTION_REVIEW_COVERAGE".into());
    }
    let issues = raw["issues"]
        .as_array()
        .ok_or("SERIES_SUPPLY_CORRECTION_REVIEW_ISSUES")?;
    if !matches!(s(raw, "conclusion"), "pass" | "revise")
        || (raw["conclusion"] == "pass") != issues.is_empty()
    {
        return Err("SERIES_SUPPLY_CORRECTION_REVIEW_CONCLUSION".into());
    }
    for issue in issues {
        if issue["unresolvedIndex"]
            .as_u64()
            .is_none_or(|n| n >= count as u64)
            || s(issue, "problem").trim().is_empty()
            || s(issue, "requestedChange").trim().is_empty()
            || (!s(issue, "proposalKey").is_empty()
                && !expected["reviewedKeys"]
                    .as_array()
                    .unwrap()
                    .contains(&issue["proposalKey"]))
            || (!s(issue, "assetId").is_empty()
                && !expected["reviewedRevisionAssetIds"]
                    .as_array()
                    .unwrap()
                    .contains(&issue["assetId"])
                && !expected["reviewedExistingAssetIds"]
                    .as_array()
                    .unwrap()
                    .contains(&issue["assetId"]))
        {
            return Err("SERIES_SUPPLY_CORRECTION_REVIEW_ISSUE_INVALID".into());
        }
    }
    Ok(())
}
pub(super) fn adopted(
    candidate: &Value,
    origin: &TextRequest,
    source: &LockedSource,
    design_sha: &Value,
    review_sha: &Value,
) -> Result<Vec<Value>, String> {
    candidate["revisions"].as_array().ok_or("SERIES_SUPPLY_CORRECTION_REVISION_REQUIRED")?.iter().map(|r| {
        let mut card = supplement::before(origin)?.iter().find(|a| a["id"] == r["assetId"])
            .cloned().ok_or("SERIES_SUPPLY_CORRECTION_REVISION_UNAUTHORIZED")?;
        let next = r["baseDesignRevision"].as_u64().and_then(|v|v.checked_add(1)).ok_or("SERIES_SUPPLY_CORRECTION_BASE_VERSION")?;
        card["designRevision"] = json!(next);
        card["designStatus"] = json!("adopted");
        card["adoptedDesign"] = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":r["assetId"],"variantId":r["variantId"],
            "designRevision":next,"sourceSha256":source.sha256,"sourceRefs":r["sourceRefs"],"sourceFacts":r["sourceFacts"],
            "productionChoices":r["productionChoices"],"full":r["full"],"brief":r["brief"],"designDependencies":r["designDependencies"],
            "provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT","adoptionBasis":{"kind":"delegated-text-review",
                "correctionVersion":supply_correction::VERSION,"baseDesignSha256":r["baseDesignSha256"],
                "designRequestSha256":design_sha,"reviewRequestSha256":review_sha,"candidateSha256":hash(candidate)}});
        if candidate["schemaVersion"] == NORMAL_CANDIDATE {
            card["adoptedDesign"]["adoptionBasis"].as_object_mut().unwrap().remove("correctionVersion");
            card["adoptedDesign"]["adoptionBasis"]["supplementVersion"] = json!(VERSION);
        }
        Ok(card)
    }).collect()
}
