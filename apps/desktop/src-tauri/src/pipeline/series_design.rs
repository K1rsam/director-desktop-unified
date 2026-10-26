//! Complete adopted text designs are independent of image prompts/media.
use super::*;

fn in_episode(asset: &Value, episode: &str) -> bool {
    if let Some(ids) = asset
        .pointer("/applicability/episodes")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        return ids.iter().any(|id| id == episode);
    }
    if let Some(scopes) = asset["applicability"].as_array().filter(|v| !v.is_empty()) {
        return scopes.iter().any(|s| s["episodeId"] == episode);
    }
    if let Some(scopes) = asset
        .pointer("/scope/scenes")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        return scopes
            .iter()
            .any(|s| s["episodeId"] == episode || s[0] == episode);
    }
    true
}
pub(super) fn adopted(asset: &Value, source: &LockedSource) -> bool {
    let d = &asset["adoptedDesign"];
    let has_full = d["full"].as_str().is_some_and(|s| !s.trim().is_empty())
        || d["full"].as_object().is_some_and(|m| !m.is_empty());
    has_full
        && !matches!(
            s(asset, "designStatus"),
            "needs_review" | "stale" | "rejected"
        )
        && d["assetId"] == asset["id"]
        && d["variantId"] == asset["variantId"]
        && d["sourceSha256"] == source.sha256
        && d["status"] == "ADOPTED"
        && d["provenance"] == "ADOPTED_DESIGN_NOT_SOURCE_FACT"
        && d["adoptionBasis"].is_object()
        && d["designRevision"].as_u64().is_some_and(|v| v > 0)
}
fn dependencies(asset: &Value) -> Result<Vec<Value>, String> {
    asset
        .pointer("/adoptedDesign/designDependencies")
        .or_else(|| asset.get("designDependencies"))
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_array()
                .cloned()
                .ok_or_else(|| "SERIES_DESIGN_DEPENDENCIES_NOT_ARRAY".into())
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}
fn selected(episode: &SourceEpisode, assets: &[Value]) -> Result<Vec<String>, String> {
    let mut ids: Vec<_> = assets
        .iter()
        .filter(|a| in_episode(a, &episode.id))
        .map(|a| s(a, "id").to_owned())
        .collect();
    let mut seen = HashSet::new();
    for id in &ids {
        if id.is_empty() || !seen.insert(id.clone()) {
            return Err("SERIES_DUPLICATE_ASSET_ID".into());
        }
    }
    let mut cursor = 0;
    while cursor < ids.len() {
        let a = assets
            .iter()
            .find(|a| a["id"] == ids[cursor])
            .ok_or("SERIES_ASSET_UNKNOWN")?;
        for d in dependencies(a)? {
            let id = s(&d, "assetId");
            if !assets.iter().any(|a| a["id"] == id) {
                return Err(format!(
                    "SERIES_DESIGN_PARENT_MISSING:{} -> {id}",
                    ids[cursor]
                ));
            }
            if seen.insert(id.into()) {
                ids.push(id.into());
            }
        }
        cursor += 1;
    }
    Ok(ids)
}
fn validate_dependencies(assets: &[Value], ids: &[String]) -> Result<(), String> {
    fn visit<'a>(
        id: &'a str,
        assets: &'a [Value],
        active: &mut Vec<String>,
        done: &mut HashSet<String>,
    ) -> Result<(), String> {
        if active.iter().any(|v| v == id) {
            active.push(id.into());
            return Err(format!(
                "SERIES_DESIGN_DEPENDENCY_CYCLE:{}",
                active.join(" -> ")
            ));
        }
        if done.contains(id) {
            return Ok(());
        }
        let a = assets
            .iter()
            .find(|a| a["id"] == id)
            .ok_or_else(|| format!("SERIES_DESIGN_PARENT_MISSING:{id}"))?;
        active.push(id.into());
        for dep in dependencies(a)? {
            let target = s(&dep, "assetId");
            let parent = assets
                .iter()
                .find(|a| a["id"] == target)
                .ok_or_else(|| format!("SERIES_DESIGN_PARENT_MISSING:{id} -> {target}"))?;
            if dep["designRevision"].as_u64().is_none()
                || dep["designRevision"] != parent["adoptedDesign"]["designRevision"]
            {
                return Err(format!("SERIES_DESIGN_PARENT_VERSION:{id} -> {target}"));
            }
            visit(target, assets, active, done)?;
        }
        active.pop();
        done.insert(id.into());
        Ok(())
    }
    let mut done = HashSet::new();
    for id in ids {
        visit(id, assets, &mut Vec::new(), &mut done)?;
    }
    Ok(())
}
pub(super) fn card(asset: &Value, include_design: bool) -> Value {
    let keys = [
        "id",
        "variantId",
        "identityId",
        "baseAssetId",
        "parentAssetId",
        "name",
        "kind",
        "type",
        "version",
        "designRevision",
        "designStatus",
        "aliases",
        "description",
        "applicability",
        "scope",
        "sourceRefs",
        "sourceFacts",
        "visualFacts",
        "preparation",
        "materialRole",
        "importName",
        "filename",
        "placeholderOrdinal",
        "sourceAssetId",
        "sourceVariantId",
    ];
    let mut out = Map::new();
    for key in keys {
        if let Some(value) = asset.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    if include_design {
        let mut design = Map::new();
        for key in [
            "version",
            "status",
            "assetId",
            "variantId",
            "designRevision",
            "sourceSha256",
            "sourceRefs",
            "sourceFacts",
            "productionChoices",
            "full",
            "brief",
            "designDependencies",
            "provenance",
            "adoptionBasis",
            "applicability",
        ] {
            if let Some(value) = asset["adoptedDesign"].get(key) {
                design.insert(key.into(), value.clone());
            }
        }
        out.insert("adoptedDesign".into(), Value::Object(design));
    }
    Value::Object(out)
}
pub(super) fn cards(
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
) -> Result<Vec<Value>, String> {
    let ids = selected(episode, assets)?;
    validate_dependencies(assets, &ids)?;
    ids.iter()
        .map(|id| {
            let a = assets.iter().find(|a| a["id"] == *id).unwrap();
            // Pure voice records are source identities; they need no visual design.
            let voice = export::is_voice_only(a);
            if !voice && !adopted(a, source) {
                return Err(format!("SERIES_FULL_DESIGN_NOT_ADOPTED:{id}"));
            }
            let mut out = card(a, !voice);
            if !in_episode(a, &episode.id) {
                out["materialRole"] = json!("MATERIAL_SOURCE_ONLY");
            }
            Ok(out)
        })
        .collect()
}
fn schema(targets: &[Value], rows: &[SourceRow], all: &[Value]) -> Value {
    let id =
        json!({"type":"string","enum":targets.iter().map(|a|a["id"].clone()).collect::<Vec<_>>()});
    let refs = super::super::refs(&rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>());
    let dependencies = array(obj(
        json!({"assetId":{"type":"string","enum":all.iter().map(|a|a["id"].clone()).collect::<Vec<_>>()},"designRevision":{"type":"integer","minimum":1}}),
    ));
    obj(
        json!({"schemaVersion":choices(&["series-full-design-candidate/v1"]),"designs":array(obj(json!({"assetId":id,"variantId":string(),"full":string(),"brief":string(),"sourceRefs":refs,"sourceFacts":array(obj(json!({"statement":string(),"sourceRefs":refs}))),"productionChoices":array(string()),"designDependencies":dependencies})))}),
    )
}
fn review_schema() -> Value {
    obj(
        json!({"schemaVersion":choices(&["series-design-review/v1"]),"candidateSha256":string(),"reviewedAssetIds":array(string()),"summary":string(),"issues":array(obj(json!({"assetId":string(),"problem":string(),"requestedChange":string()})))}),
    )
}
fn validate(raw: &Value, targets: &[Value], rows: &[SourceRow]) -> Result<Value, String> {
    if raw["schemaVersion"] != "series-full-design-candidate/v1" {
        return Err("SERIES_DESIGN_CANDIDATE_VERSION".into());
    }
    let designs = raw["designs"]
        .as_array()
        .ok_or("SERIES_DESIGN_CANDIDATES_MISSING")?;
    if designs.len() != targets.len() {
        return Err("SERIES_DESIGN_CANDIDATE_SCOPE".into());
    }
    let mut seen = HashSet::new();
    for d in designs {
        let asset = targets
            .iter()
            .find(|a| a["id"] == d["assetId"] && a["variantId"] == d["variantId"])
            .ok_or("SERIES_DESIGN_IDENTITY_MISMATCH")?;
        if !seen.insert(s(d, "assetId"))
            || s(d, "full").trim().is_empty()
            || s(d, "brief").trim().is_empty()
            || d["full"] == d["brief"]
        {
            return Err("SERIES_COMPLETE_DESIGN_REQUIRED".into());
        }
        let refs = string_list(&d["sourceRefs"])?;
        if refs.is_empty() || refs.iter().any(|id| !rows.iter().any(|r| r.id == *id)) {
            return Err(format!("SERIES_DESIGN_SOURCE_MISSING:{}", s(asset, "id")));
        }
        for fact in d["sourceFacts"]
            .as_array()
            .ok_or("SERIES_DESIGN_SOURCE_FACTS_REQUIRED")?
        {
            if s(fact, "statement").is_empty()
                || string_list(&fact["sourceRefs"])?
                    .iter()
                    .any(|id| !refs.contains(id))
            {
                return Err("SERIES_DESIGN_FACT_SOURCE_INVALID".into());
            }
        }
        string_list(&d["productionChoices"])?;
        if !d["designDependencies"].is_array() {
            return Err("SERIES_DESIGN_DEPENDENCIES_REQUIRED".into());
        }
    }
    Ok(raw.clone())
}
pub(super) async fn ensure(
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &mut Vec<Value>,
    entry: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let ids = selected(episode, assets)?;
    let targets: Vec<_> = assets
        .iter()
        .filter(|a| {
            ids.contains(&s(a, "id").to_owned()) && !export::is_voice_only(a) && !adopted(a, source)
        })
        .cloned()
        .collect();
    if targets.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let related: Vec<_> = assets
        .iter()
        .filter(|a| ids.contains(&s(a, "id").to_owned()))
        .map(|a| card(a, adopted(a, source)))
        .collect();
    let mut rows: Vec<_> = source
        .rows
        .iter()
        .filter(|r| {
            r.kind == "body"
                && (r.episode_id.as_deref() == Some(&episode.id)
                    || targets.iter().any(|a| {
                        a["sourceRefs"]
                            .as_array()
                            .is_some_and(|ids| ids.iter().any(|id| id == &r.id))
                    }))
        })
        .cloned()
        .collect();
    if input[design_source::FIELD] == design_source::VERSION {
        rows = design_source::rows(input, source, episode, &targets)?;
    }
    rows.sort_by_key(|r| r.range.start);
    let mut base = base_values(input, source, episode, &related, entry);
    let compact = input["seriesTransportVersion"] == COMPACT_DESIGN_INPUT;
    if compact {
        if let Some((_, value)) = base.iter_mut().find(|(kind, _)| *kind == "source") {
            value.as_object_mut().unwrap().remove("sourceRows");
            value["sourceRowsBinding"] = json!("designSourceRows");
        }
    }
    base.push(("designTargets",json!(targets.iter().map(|a|json!({"asset":if compact {json!({"id":a["id"],"variantId":a["variantId"]})} else {card(a,false)},"nextDesignRevision":a["designRevision"].as_u64().unwrap_or(0)+1})).collect::<Vec<_>>())));
    base.push(("designSourceRows", source_view(&rows)));
    let output_schema = schema(&targets, &rows, assets);
    let mut receipts = Vec::new();
    let mut previous = Value::Null;
    let mut issues = Value::Null;
    let mut last_issue_hash = String::new();
    for iteration in 0..=MAX_REPAIRS {
        let mut values = base.clone();
        if !previous.is_null() {
            values.push(("designCandidate", previous.clone()));
            values.push(("designIssues", issues.clone()));
        }
        let req = request(
            "design",
            "seriesDesign",
            &episode.id,
            &iteration.to_string(),
            input,
            values,
            output_schema.clone(),
            model,
            reasoning,
        )?;
        let (candidate, creation) =
            call(executor, req, |raw| validate(raw, &targets, &rows)).await?;
        receipts.push(creation.clone());
        let mut review_values = base.clone();
        review_values.push(("designCandidate", candidate.clone()));
        let mut req = request(
            "design-review",
            "seriesDesignReview",
            &episode.id,
            &iteration.to_string(),
            input,
            review_values,
            review_schema(),
            model,
            reasoning,
        )?;
        req.input["candidateSha256"] = json!(hash(&candidate));
        let expected: Vec<_> = targets.iter().map(|a| s(a, "id").to_owned()).collect();
        let (review, review_receipt) = call(executor, req, |raw| {
            if raw["schemaVersion"] != "series-design-review/v1"
                || raw["candidateSha256"] != hash(&candidate)
                || string_list(&raw["reviewedAssetIds"])? != expected
                || s(raw, "summary").trim().is_empty()
            {
                return Err("SERIES_DESIGN_REVIEW_UNBOUND".into());
            }
            for issue in raw["issues"]
                .as_array()
                .ok_or("SERIES_DESIGN_REVIEW_ISSUES")?
            {
                if !expected.iter().any(|id| issue["assetId"] == *id)
                    || s(issue, "problem").trim().is_empty()
                    || s(issue, "requestedChange").trim().is_empty()
                {
                    return Err("SERIES_DESIGN_REVIEW_ISSUE_INVALID".into());
                }
            }
            Ok(raw.clone())
        })
        .await?;
        receipts.push(review_receipt.clone());
        if review["issues"].as_array().unwrap().is_empty() {
            let mut upgraded = assets.clone();
            let mut adoptions = Vec::new();
            for d in candidate["designs"].as_array().unwrap() {
                let a = upgraded
                    .iter_mut()
                    .find(|a| a["id"] == d["assetId"] && a["variantId"] == d["variantId"])
                    .unwrap();
                let old = a["designRevision"].as_u64().unwrap_or(0);
                let revision = old + 1;
                let adopted = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":a["id"],"variantId":a["variantId"],"designRevision":revision,"sourceSha256":source.sha256,"sourceRefs":d["sourceRefs"],"sourceFacts":d["sourceFacts"],"productionChoices":d["productionChoices"],"full":d["full"],"brief":d["brief"],"designDependencies":d["designDependencies"],"provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT","adoptionBasis":{"kind":"delegated-text-review","designRequestSha256":creation["requestSha256"],"reviewRequestSha256":review_receipt["requestSha256"],"candidateSha256":hash(&candidate)}});
                a["adoptedDesign"] = adopted.clone();
                a["designRevision"] = json!(revision);
                a["designStatus"] = json!("adopted");
                adoptions.push(json!({"assetId":a["id"],"variantId":a["variantId"],"baseDesignRevision":old,"designRevision":revision,"adoptedDesign":adopted,"designReview":{"requestSha256":review_receipt["requestSha256"],"resultSha256":review_receipt["rawSha256"],"issues":[]}}));
            }
            validate_dependencies(&upgraded, &ids)?;
            *assets = upgraded;
            return Ok((adoptions, receipts));
        }
        let current = hash(&review["issues"]);
        if iteration == MAX_REPAIRS || current == last_issue_hash {
            return Err("SERIES_DESIGN_REVIEW_REPAIR_HELD".into());
        }
        last_issue_hash = current;
        previous = candidate;
        issues = review["issues"].clone();
    }
    Err("SERIES_DESIGN_REPAIR_LIMIT".into())
}

/// A design may have passed this episode's independent design review before
/// the episode is published. Prove that exact in-memory predecessor; do not
/// pretend it already exists in the asset database or relax the database CAS.
pub(crate) fn validate_unpublished_design_predecessor(
    input: &Value,
    delivery: &Value,
    before: &Value,
) -> Result<(), String> {
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| delivery["episodeId"] == e.id)
        .ok_or("SERIES_DESIGN_PREDECESSOR_EPISODE")?;
    let mut original = input["assets"]
        .as_array()
        .ok_or("SERIES_ASSETS_MISSING")?
        .clone();
    allocate_asset_identities(&mut original)?;
    let initial = original
        .iter()
        .find(|a| a["id"] == before["id"] && a["variantId"] == before["variantId"])
        .ok_or("SERIES_DESIGN_PREDECESSOR_INITIAL_IDENTITY")?;
    let initial_revision = initial["designRevision"].as_u64().unwrap_or(0);
    let revision = before["designRevision"]
        .as_u64()
        .ok_or("SERIES_DESIGN_PREDECESSOR_VERSION")?;
    if revision != initial_revision + 1
        || before["version"] != initial["version"]
        || !adopted(before, &source)
    {
        return Err("SERIES_DESIGN_PREDECESSOR_BASE_CHANGED".into());
    }
    let roles = delivery["adoptionReceipt"]["roleReceipts"]
        .as_array()
        .ok_or("SERIES_ADOPTION_ROLE_RECEIPTS_MISSING")?;
    let basis = &before["adoptedDesign"]["adoptionBasis"];
    let find = |sha: &Value| -> Result<&Value, String> {
        let found: Vec<_> = roles
            .iter()
            .filter(|r| r["requestSha256"] == *sha)
            .collect();
        if found.len() != 1 {
            return Err("SERIES_DESIGN_PREDECESSOR_RECEIPT_REQUIRED".into());
        }
        Ok(found[0])
    };
    let creation = find(&basis["designRequestSha256"])?;
    let review = find(&basis["reviewRequestSha256"])?;
    let created: TextRequest = serde_json::from_value(creation["request"].clone())
        .map_err(|_| "SERIES_DESIGN_PREDECESSOR_REQUEST")?;
    let reviewed: TextRequest = serde_json::from_value(review["request"].clone())
        .map_err(|_| "SERIES_DESIGN_PREDECESSOR_REQUEST")?;
    for (receipt, request) in [(creation, &created), (review, &reviewed)] {
        if role_receipt(request, &receipt["raw"])? != *receipt
            || request.input.get("seriesAssetSupplement").is_some()
            || request.input.get("seriesSupplyCorrection").is_some()
        {
            return Err("SERIES_DESIGN_PREDECESSOR_NOT_ORDINARY_DESIGN".into());
        }
        validate_role_packet(request, input, &source, episode)?;
    }
    let iteration = s(&created.input, "executionRoleId")
        .strip_prefix(&format!("{}/design/", episode.id))
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| *n <= MAX_REPAIRS)
        .ok_or("SERIES_DESIGN_PREDECESSOR_ROLE")?;
    if created.stage != "seriesDesign"
        || reviewed.stage != "seriesDesignReview"
        || reviewed.input["executionRoleId"] != format!("{}/design-review/{iteration}", episode.id)
        || packet_value(&reviewed.input["packet"], "designCandidate")? != &creation["raw"]
        || reviewed.input["candidateSha256"] != hash(&creation["raw"])
        || review["raw"]["candidateSha256"] != hash(&creation["raw"])
        || review["raw"]["schemaVersion"] != "series-design-review/v1"
        || review["raw"]["issues"] != json!([])
        || s(&review["raw"], "summary").trim().is_empty()
    {
        return Err("SERIES_DESIGN_PREDECESSOR_REVIEW_NOT_PASSED".into());
    }
    for kind in ["assets", "designTargets", "designSourceRows", "entry"] {
        if packet_value(&created.input["packet"], kind)?
            != packet_value(&reviewed.input["packet"], kind)?
        {
            return Err("SERIES_DESIGN_PREDECESSOR_REVIEW_INPUT_CHANGED".into());
        }
    }
    let targets = packet_value(&created.input["packet"], "designTargets")?
        .as_array()
        .ok_or("SERIES_DESIGN_PREDECESSOR_TARGETS")?;
    if !targets.iter().any(|t| {
        t["asset"]["id"] == before["id"]
            && t["asset"]["variantId"] == before["variantId"]
            && t["nextDesignRevision"] == revision
    }) || review["raw"]["reviewedAssetIds"]
        != json!(targets
            .iter()
            .map(|t| t["asset"]["id"].clone())
            .collect::<Vec<_>>())
    {
        return Err("SERIES_DESIGN_PREDECESSOR_TARGETS_CHANGED".into());
    }
    let prior_cards = packet_value(&created.input["packet"], "assets")?
        .as_array()
        .ok_or("SERIES_ASSETS_MISSING")?;
    let expected_card = card(initial, adopted(initial, &source));
    if !prior_cards.contains(&expected_card) {
        return Err("SERIES_DESIGN_PREDECESSOR_INITIAL_CARD_CHANGED".into());
    }
    let target_cards = targets
        .iter()
        .map(|t| {
            prior_cards
                .iter()
                .find(|a| a["id"] == t["asset"]["id"])
                .cloned()
                .ok_or_else(|| "SERIES_DESIGN_PREDECESSOR_TARGETS_CHANGED".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let row_values = packet_value(&created.input["packet"], "designSourceRows")?
        .as_array()
        .ok_or("SERIES_DESIGN_SOURCE_ROWS_MISSING")?;
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|r| row_values.iter().any(|v| v["id"] == r.id))
        .cloned()
        .collect();
    validate(&creation["raw"], &target_cards, &rows)?;
    let d = creation["raw"]["designs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["assetId"] == before["id"] && d["variantId"] == before["variantId"])
        .ok_or("SERIES_DESIGN_PREDECESSOR_DESIGN_MISSING")?;
    let expected = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":before["id"],"variantId":before["variantId"],"designRevision":revision,
        "sourceSha256":source.sha256,"sourceRefs":d["sourceRefs"],"sourceFacts":d["sourceFacts"],"productionChoices":d["productionChoices"],
        "full":d["full"],"brief":d["brief"],"designDependencies":d["designDependencies"],"provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT",
        "adoptionBasis":{"kind":"delegated-text-review","designRequestSha256":creation["requestSha256"],"reviewRequestSha256":review["requestSha256"],"candidateSha256":hash(&creation["raw"])}});
    if before["adoptedDesign"] != expected {
        return Err("SERIES_DESIGN_PREDECESSOR_RAW_CHANGED".into());
    }
    let expected_adoption = json!({"assetId":before["id"],"variantId":before["variantId"],"baseDesignRevision":initial_revision,"designRevision":revision,
        "adoptedDesign":expected,"designReview":{"requestSha256":review["requestSha256"],"resultSha256":review["rawSha256"],"issues":[]}});
    if delivery["designAdoptions"]
        .as_array()
        .ok_or("SERIES_DESIGN_ADOPTIONS_REQUIRED")?
        .iter()
        .filter(|a| a["assetId"] == before["id"] && a["designRevision"] == revision)
        .collect::<Vec<_>>()
        != vec![&expected_adoption]
    {
        return Err("SERIES_DESIGN_PREDECESSOR_ADOPTION_CHAIN_CHANGED".into());
    }
    Ok(())
}
