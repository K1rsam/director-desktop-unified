//! Append-only missing-material resolution within the original frozen workflow.
//! An unadopted author is evidence, never silently promoted to accepted output.
use super::*;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const VERSION: &str = "series-asset-supplement/v1";
const CANDIDATE: &str = "series-asset-supplement-candidate/v1";
const REVIEW: &str = "series-asset-supplement-review/v1";
const REVIEW_DIGEST: &str = "series-supplement-review-digest/v1";
const REVIEW_COVERAGE: &str = "series-supplement-review-coverage/v1";
const REVIEW_RESPONSE_CONTRACT: &str = "完整独立审阅全部原问题、候选设计和所选既有资产后，把 expectedReviewCoverage 中各数组按给定原顺序原样回传到同名字段，不排序、不遗漏、不重复、不自行补入其他值；这些数组只声明覆盖范围，不预设审查结论。candidateSha256 直接复制输入同名值，不自行计算。summary 必须含非空白的实际复核结论；每个 issue 的 problem 和 requestedChange 必须含非空白的具体问题与修改要求，unresolvedIndex 对应原问题序号，proposalKey 为关联候选 key 或空字符串。只有 issues=[] 才能 conclusion=pass；存在任何问题必须 conclusion=revise 并列出 issues。";
const PROPOSAL_KEY_PATTERN: &str = r"^[a-z][a-z0-9_-]{0,63}$";
pub(super) fn request_sha(request: &TextRequest) -> Result<String, String> {
    Ok(digest(
        &serde_json::to_string(request).map_err(|e| e.to_string())?,
    ))
}
fn normalized(value: &str) -> String {
    value.nfkc().case_fold().collect()
}
/// Historical names/ordinals are reservations, not new creative source facts.
pub(crate) fn supplement_identity_reservation(asset: &Value) -> Option<Value> {
    let ordinal = asset["placeholderOrdinal"].as_u64()?;
    let kind = asset
        .get("kind")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .or_else(|| asset.get("type").and_then(Value::as_str))?;
    if ordinal > 5700
        || s(asset, "id").is_empty()
        || s(asset, "variantId").is_empty()
        || s(asset, "importName").is_empty()
        || s(asset, "filename").is_empty()
    {
        return None;
    }
    Some(
        json!({"id":asset["id"],"variantId":asset["variantId"],"kind":kind,
        "importName":asset["importName"],"filename":asset["filename"],"placeholderOrdinal":ordinal}),
    )
}
pub(crate) fn validate_supplement_identity_context(
    context: &Value,
    origin: &TextRequest,
    raw: &Value,
    known: &Value,
) -> Result<(), String> {
    let compact_known;
    let known = if supply::enabled(context) {
        compact_known = supplement_supply_catalogue(known);
        &compact_known
    } else {
        known
    };
    let reservations = context["reservedAssets"]
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_RESERVATIONS_REQUIRED")?;
    let floor = context["nextOrdinal"]
        .as_u64()
        .filter(|n| *n <= 5701)
        .ok_or("SERIES_SUPPLEMENT_HIGH_WATER_INVALID")?;
    let mut expected = json!({"version":"series-supplement-identities/v1","sourceSha256":origin.input["sourceSha256"],"episodeId":origin.input["episodeId"],
        "originRequestSha256":request_sha(origin)?,"originRawSha256":hash(raw),"knownCatalogSha256":hash(known),"nextOrdinal":floor,"reservedAssets":reservations});
    if supply::enabled(context) {
        supply::validate_states(context, known)?;
        expected["version"] = json!(if supply_revision::enabled(context) {
            "series-supplement-identities/v3"
        } else {
            "series-supplement-identities/v2"
        });
        expected["contractVersion"] = json!(supply::version(context));
        expected["assetStates"] = context["assetStates"].clone();
    }
    if supply_revision::enabled(context)
        && origin.input[supply_revision::INPUT_FIELD] != supply_revision::VERSION
    {
        return Err("SERIES_SUPPLEMENT_V3_NOT_FROZEN".into());
    }
    if *context != expected {
        return Err("SERIES_SUPPLEMENT_RESERVATIONS_UNBOUND".into());
    }
    let mut ids = HashSet::new();
    let mut ordinals = HashSet::new();
    let mut files = HashSet::new();
    let mut prior = "";
    for asset in reservations {
        if supplement_identity_reservation(asset).as_ref() != Some(asset)
            || !ids.insert(s(asset, "id"))
            || !ordinals.insert(asset["placeholderOrdinal"].as_u64().unwrap())
            || !files.insert(normalized(s(asset, "filename")))
            || s(asset, "id") < prior
            || asset["placeholderOrdinal"].as_u64().unwrap() >= floor
        {
            return Err("SERIES_SUPPLEMENT_RESERVATION_CONFLICT".into());
        }
        prior = s(asset, "id");
    }
    for asset in known
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_CATALOGUE_REQUIRED")?
    {
        if let Some(reservation) = supplement_identity_reservation(asset) {
            if !reservations.contains(&reservation) {
                return Err("SERIES_SUPPLEMENT_ACTIVE_IDENTITY_NOT_RESERVED".into());
            }
        } else if !export::is_voice_only(asset) {
            return Err("SERIES_SUPPLEMENT_ACTIVE_IDENTITY_INCOMPLETE".into());
        }
    }
    Ok(())
}
pub(super) fn eligible(request: &TextRequest, raw: &Value) -> bool {
    request.stage == "seriesAuthor"
        && request.input["role"] == "author"
        && request.input.get("seriesAssetSupplement").is_none()
        && raw["schemaVersion"] == "series-episode-candidate/v1"
        && raw["plan"].is_object()
        && raw["selection"].is_array()
        && raw["draft"]["units"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        && raw["draft"]["unresolved"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
}
fn evidence_schema() -> Value {
    array(obj(json!({"rowId":string(),"quote":string()})))
}
fn candidate_schema() -> Value {
    let evidence = evidence_schema();
    obj(json!({"schemaVersion":choices(&[CANDIDATE]),
        "resolutions":array(obj(json!({"unresolvedIndex":{"type":"integer","minimum":0},
            "disposition":choices(&["new_asset","existing_asset","scene_detail","no_visual_asset"]),
            "proposalKeys":array(string()),"existingAssetIds":array(string()),"evidence":evidence,
            "rationale":string()}))),
        "proposals":array(obj(json!({"key":{"type":"string","pattern":PROPOSAL_KEY_PATTERN,"minLength":1,"maxLength":64},"name":string(),"kind":choices(&["CH","SC","PR","BEAST","OM"]),
            "description":string(),"basis":choices(&["explicit_source","production_choice"]),"evidence":evidence_schema(),
            "sourceFacts":array(obj(json!({"statement":string(),"sourceRefs":array(string())}))),
            "productionChoices":array(string()),"full":string(),"brief":string(),
            "designDependencies":array(obj(json!({"assetId":string(),"designRevision":{"type":"integer","minimum":1}})))})))}))
}
pub(super) fn effective_candidate_schema(context: &Value) -> Value {
    if supply_revision::enabled(context) {
        let mut schema = supply::candidate_schema(candidate_schema());
        schema["properties"]["schemaVersion"] = choices(&[supply_revision::NORMAL_CANDIDATE]);
        supply_revision::append_schema(
            schema,
            context["assetStates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["assetId"].clone())
                .collect(),
        )
    } else if supply::enabled(context) {
        supply::candidate_schema(candidate_schema())
    } else {
        candidate_schema()
    }
}
fn legacy_candidate_schema(context: &Value) -> Value {
    let mut schema = effective_candidate_schema(context);
    schema["properties"]["proposals"]["items"]["properties"]["key"] = string();
    schema
}
/// Restore only the former unconstrained key schema. The executor still has to
/// match this complete request against an existing frozen job before using it.
pub(crate) fn legacy_supplement_design_request(
    request: &TextRequest,
) -> Result<Option<TextRequest>, String> {
    let context = &request.input["seriesAssetSupplement"];
    if request.stage != "seriesDesign"
        || context["phase"] != "design"
        || (context["version"] != VERSION && context["version"] != supply::VERSION)
    {
        return Ok(None);
    }
    let identities = packet_value(&request.input["packet"], "identityReservations")?;
    let legacy_schema = legacy_candidate_schema(identities);
    if request.schema == legacy_schema {
        return Ok(None);
    }
    if request.schema != effective_candidate_schema(identities) {
        return Err("SERIES_SUPPLEMENT_KEY_SCHEMA_UNBOUND".into());
    }
    let mut legacy = request.clone();
    legacy.schema = legacy_schema;
    Ok(Some(legacy))
}
fn review_schema() -> Value {
    obj(
        json!({"schemaVersion":choices(&[REVIEW]),"candidateSha256":string(),
        "reviewedKeys":array(string()),"reviewedIssueIndexes":array(json!({"type":"integer","minimum":0})),
        "conclusion":choices(&["pass","revise"]),"summary":string(),
        "issues":array(obj(json!({"unresolvedIndex":{"type":"integer","minimum":0},"proposalKey":string(),"problem":string(),"requestedChange":string()})))}),
    )
}
fn effective_review_schema(context: &Value, candidate_sha: Option<&str>) -> Value {
    let mut schema = if supply::enabled(context) {
        supply::review_schema(review_schema())
    } else {
        review_schema()
    };
    if supply_revision::enabled(context) {
        schema["properties"]["schemaVersion"] = choices(&[supply_revision::NORMAL_REVIEW]);
        schema["properties"]["reviewedRevisionAssetIds"] = array(string());
        schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("reviewedRevisionAssetIds"));
    }
    if let Some(sha) = candidate_sha {
        schema["properties"]["candidateSha256"] = json!({"type":"string","enum":[sha]});
    }
    schema
}
fn expected_review_coverage(candidate: &Value, issue_count: usize) -> Result<Value, String> {
    let keys: Vec<_> = candidate["proposals"]
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_PROPOSALS_REQUIRED")?
        .iter()
        .map(|p| p["key"].clone())
        .collect();
    let mut expected = json!({"reviewedKeys":keys,
        "reviewedIssueIndexes":(0..issue_count).collect::<Vec<_>>()});
    if candidate["schemaVersion"] == "series-asset-supplement-candidate/v2"
        || candidate["schemaVersion"] == supply_revision::NORMAL_CANDIDATE
    {
        expected["reviewedExistingAssetIds"] = json!(supply::ids(candidate)?);
    }
    if candidate["schemaVersion"] == supply_revision::NORMAL_CANDIDATE {
        expected["reviewedRevisionAssetIds"] = json!(candidate["revisions"]
            .as_array()
            .ok_or("SERIES_DESIGN_REVISIONS_REQUIRED")?
            .iter()
            .map(|a| a["assetId"].clone())
            .collect::<Vec<_>>());
    }
    Ok(expected)
}
fn coverage_array_schema(values: &Value, item_type: &str) -> Value {
    let count = values.as_array().unwrap().len();
    let mut item = json!({"type":item_type});
    // Keep an ordinary item schema for an empty list: an empty enum is invalid.
    if count > 0 {
        item["enum"] = values.clone();
    }
    json!({"type":"array","items":item,"minItems":count,"maxItems":count})
}
fn coverage_review_schema(context: &Value, candidate_sha: &str, expected: &Value) -> Value {
    let mut schema = effective_review_schema(context, Some(candidate_sha));
    for (field, item_type) in [
        ("reviewedKeys", "string"),
        ("reviewedIssueIndexes", "integer"),
        ("reviewedExistingAssetIds", "string"),
        ("reviewedRevisionAssetIds", "string"),
    ] {
        if let Some(values) = expected.get(field) {
            schema["properties"][field] = coverage_array_schema(values, item_type);
        }
    }
    let nonblank = json!({"type":"string","pattern":r"\S"});
    schema["properties"]["summary"] = nonblank.clone();
    let issue = &mut schema["properties"]["issues"]["items"]["properties"];
    issue["problem"] = nonblank.clone();
    issue["requestedChange"] = nonblank;
    if expected["reviewedIssueIndexes"]
        .as_array()
        .is_some_and(|values| !values.is_empty())
    {
        issue["unresolvedIndex"] =
            json!({"type":"integer","enum":expected["reviewedIssueIndexes"]});
    }
    let mut proposal_keys = vec![json!("")];
    proposal_keys.extend(expected["reviewedKeys"].as_array().unwrap().iter().cloned());
    issue["proposalKey"] = json!({"type":"string","enum":proposal_keys});
    schema
}
#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn supplement_review_coverage_keeps_original_order_and_rejects_false_pass_or_blank_issues() {
        let candidate = json!({"schemaVersion":"series-asset-supplement-candidate/v2",
            "proposals":[{"key":"z-key"},{"key":"a_key"}],
            "existingSupply":[{"assetId":"z-existing"},{"assetId":"a-existing"}]});
        let expected = expected_review_coverage(&candidate, 3).unwrap();
        assert_eq!(
            expected,
            json!({"reviewedKeys":["z-key","a_key"],
            "reviewedIssueIndexes":[0,1,2],"reviewedExistingAssetIds":["z-existing","a-existing"]})
        );
        let mut raw = expected;
        raw["schemaVersion"] = json!("series-asset-supplement-review/v2");
        raw["candidateSha256"] = json!(hash(&candidate));
        raw["conclusion"] = json!("pass");
        raw["summary"] = json!("全部范围已逐项独审");
        raw["issues"] = json!([]);
        validate_review(&raw, &candidate, 3).unwrap();
        for field in [
            "reviewedKeys",
            "reviewedIssueIndexes",
            "reviewedExistingAssetIds",
        ] {
            let mut changed = raw.clone();
            changed[field].as_array_mut().unwrap().reverse();
            assert!(
                validate_review(&changed, &candidate, 3).is_err(),
                "{field} order"
            );
            changed = raw.clone();
            changed[field].as_array_mut().unwrap().pop();
            assert!(
                validate_review(&changed, &candidate, 3).is_err(),
                "{field} omissions"
            );
        }
        let mut changed = raw.clone();
        changed["summary"] = json!(" \n\t ");
        assert!(validate_review(&changed, &candidate, 3).is_err());
        changed = raw.clone();
        changed["conclusion"] = json!("revise");
        assert!(validate_review(&changed, &candidate, 3).is_err());
        changed["issues"] = json!([{"unresolvedIndex":0,"proposalKey":"z-key", "problem":"空间相交", "requestedChange":"核实并修正高度"}]);
        validate_review(&changed, &candidate, 3).unwrap();
        let mut false_pass = changed.clone();
        false_pass["conclusion"] = json!("pass");
        assert!(validate_review(&false_pass, &candidate, 3).is_err());
        for field in ["problem", "requestedChange"] {
            let mut blank = changed.clone();
            blank["issues"][0][field] = json!(" \t");
            assert!(validate_review(&blank, &candidate, 3).is_err());
        }
    }

    #[test]
    fn supplement_review_coverage_schema_uses_scalar_enums_and_valid_empty_arrays() {
        let context =
            json!({"version":"series-supplement-identities/v2","contractVersion":supply::VERSION});
        let expected = json!({"reviewedKeys":[],"reviewedIssueIndexes":[0,1],"reviewedExistingAssetIds":["asset-z","asset-a"]});
        let schema = coverage_review_schema(&context, &"a".repeat(64), &expected);
        let properties = &schema["properties"];
        assert_eq!(
            properties["reviewedKeys"],
            json!({"type":"array","items":{"type":"string"},"minItems":0,"maxItems":0})
        );
        assert_eq!(
            properties["reviewedIssueIndexes"]["items"],
            json!({"type":"integer","enum":[0,1]})
        );
        assert_eq!(
            properties["reviewedExistingAssetIds"]["items"]["enum"],
            json!(["asset-z", "asset-a"])
        );
        assert_eq!(properties["summary"]["pattern"], json!(r"\S"));
        assert_eq!(
            properties["issues"]["items"]["properties"]["proposalKey"]["enum"],
            json!([""])
        );
        for field in [
            "reviewedKeys",
            "reviewedIssueIndexes",
            "reviewedExistingAssetIds",
        ] {
            assert!(properties[field].get("enum").is_none());
            assert!(properties[field].get("const").is_none());
        }
    }
}
/// Exact inverse of the new coverage guidance only. The durable executor must
/// still match the entire result to a real frozen snapshot before using it.
pub(crate) fn legacy_supplement_coverage_request(
    request: &TextRequest,
) -> Result<Option<TextRequest>, String> {
    if request.input["seriesAssetSupplement"]["version"] == supply_revision::VERSION {
        return Ok(None);
    }
    if request.input.get("reviewCoverageBinding").is_none() {
        if request.input.get("expectedReviewCoverage").is_some()
            || request.input.get("reviewResponseContract").is_some()
        {
            return Err("SERIES_SUPPLEMENT_REVIEW_COVERAGE_UNBOUND".into());
        }
        return Ok(None);
    }
    let contract = &request.input["seriesAssetSupplement"];
    if request.stage != "seriesDesignReview"
        || contract["phase"] != "design-review"
        || (contract["version"] != VERSION && contract["version"] != supply::VERSION)
    {
        return Err("SERIES_SUPPLEMENT_REVIEW_COVERAGE_UNBOUND".into());
    }
    let packet = &request.input["packet"];
    let identities = packet_value(packet, "identityReservations")?;
    let candidate = packet_value(packet, "supplementCandidate")?;
    let count = packet_value(packet, "missingMaterialIssues")?
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_REVIEW_ISSUES")?
        .len();
    let expected = expected_review_coverage(candidate, count)?;
    let sha = hash(candidate);
    if request.input["reviewCoverageBinding"] != REVIEW_COVERAGE
        || request.input["expectedReviewCoverage"] != expected
        || request.input["reviewResponseContract"] != REVIEW_RESPONSE_CONTRACT
        || request.input["reviewDigestBinding"] != REVIEW_DIGEST
        || request.input["candidateSha256"] != sha
        || contract["predecessorRawSha256"] != sha
        || request.schema != coverage_review_schema(identities, &sha, &expected)
    {
        return Err("SERIES_SUPPLEMENT_REVIEW_COVERAGE_UNBOUND".into());
    }
    let mut legacy = request.clone();
    for field in [
        "reviewCoverageBinding",
        "expectedReviewCoverage",
        "reviewResponseContract",
    ] {
        legacy.input.as_object_mut().unwrap().remove(field);
    }
    legacy.schema = effective_review_schema(identities, Some(&sha));
    Ok(Some(legacy))
}
fn bound_review_schema(candidate_sha: &str) -> Value {
    let mut schema = review_schema();
    schema["properties"]["candidateSha256"] = json!({"type":"string","enum":[candidate_sha]});
    schema
}
/// Exact transport-only inverse, so an already frozen v1 review is resumed
/// byte-for-byte rather than replaced when deterministic hash guidance arrives.
pub(crate) fn legacy_supplement_review_request(
    request: &TextRequest,
) -> Result<Option<TextRequest>, String> {
    if [supply::VERSION, supply_revision::VERSION]
        .iter()
        .any(|v| request.input["seriesAssetSupplement"]["version"] == *v)
    {
        return Ok(None);
    }
    let without_coverage = legacy_supplement_coverage_request(request)?;
    let request = without_coverage.as_ref().unwrap_or(request);
    if request.input.get("reviewDigestBinding").is_none() {
        return Ok(None);
    }
    let sha = s(
        &request.input["seriesAssetSupplement"],
        "predecessorRawSha256",
    );
    if request.stage != "seriesDesignReview"
        || request.input["seriesAssetSupplement"]["version"] != VERSION
        || request.input["seriesAssetSupplement"]["phase"] != "design-review"
        || request.input["reviewDigestBinding"] != REVIEW_DIGEST
        || request.input["candidateSha256"] != sha
        || sha.len() != 64
        || request.schema != bound_review_schema(sha)
    {
        return Err("SERIES_SUPPLEMENT_REVIEW_DIGEST_UNBOUND".into());
    }
    let mut legacy = request.clone();
    legacy
        .input
        .as_object_mut()
        .unwrap()
        .remove("reviewDigestBinding");
    legacy
        .input
        .as_object_mut()
        .unwrap()
        .remove("candidateSha256");
    legacy.schema = review_schema();
    Ok(Some(legacy))
}
pub(super) fn evidence_refs(
    value: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
) -> Result<Vec<String>, String> {
    evidence_refs_in_rows(value, source, episode, None)
}

/// Historical rows explain source facts, not additional episode uses. Every
/// proposed use still requires an exact quotation from the current episode.
fn evidence_refs_in_rows(
    value: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    supplied_rows: Option<&[SourceRow]>,
) -> Result<Vec<String>, String> {
    let evidence = value
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("SERIES_SUPPLEMENT_SOURCE_EVIDENCE_REQUIRED")?;
    let mut refs = Vec::new();
    for e in evidence {
        let row = source
            .rows
            .iter()
            .find(|r| {
                r.id == s(e, "rowId")
                    && r.kind == "body"
                    && supplied_rows.map_or_else(
                        || r.episode_id.as_deref() == Some(&episode.id),
                        |rows| {
                            rows.iter()
                                .any(|supplied| supplied.id == r.id && supplied.text == r.text)
                        },
                    )
            })
            .ok_or("SERIES_SUPPLEMENT_SOURCE_ROW_UNKNOWN")?;
        let quote = s(e, "quote");
        if quote.trim().is_empty() || !row.text.contains(quote) {
            return Err("SERIES_SUPPLEMENT_SOURCE_QUOTE_CHANGED".into());
        }
        if !refs.contains(&row.id) {
            refs.push(row.id.clone());
        }
    }
    if !refs.iter().any(|id| {
        source
            .rows
            .iter()
            .any(|row| row.id == *id && row.episode_id.as_deref() == Some(&episode.id))
    }) {
        return Err("SERIES_SUPPLEMENT_CURRENT_USE_EVIDENCE_REQUIRED".into());
    }
    Ok(refs)
}

#[cfg(test)]
mod source_evidence_tests {
    use super::*;

    #[test]
    fn supplement_fact_history_requires_supplied_rows_exact_quotes_and_current_use() {
        let source = crate::pipeline::prepare_source(
            "第一集\n场景：门口\n甲捡起铜钥匙。\n第二集\n场景：房间\n甲用铜钥匙打开箱子。\n",
        )
        .unwrap();
        let current = &source.episodes[1];
        let body: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .cloned()
            .collect();
        let old = body
            .iter()
            .find(|r| r.episode_id.as_deref() == Some(&source.episodes[0].id))
            .unwrap();
        let now = body
            .iter()
            .find(|r| r.episode_id.as_deref() == Some(&current.id))
            .unwrap();
        let evidence = json!([{"rowId":old.id,"quote":old.text},{"rowId":now.id,"quote":now.text}]);
        assert_eq!(
            evidence_refs_in_rows(&evidence, &source, current, Some(&body)).unwrap(),
            vec![old.id.clone(), now.id.clone()]
        );
        assert!(
            evidence_refs(&evidence, &source, current).is_err(),
            "legacy ordinary supply remains current-episode-only"
        );
        assert!(
            evidence_refs_in_rows(&evidence, &source, current, Some(&[now.clone()])).is_err(),
            "available project rows are not necessarily supplied rows"
        );
        assert_eq!(
            evidence_refs_in_rows(&json!([evidence[0]]), &source, current, Some(&body))
                .unwrap_err(),
            "SERIES_SUPPLEMENT_CURRENT_USE_EVIDENCE_REQUIRED"
        );
        let mut changed = evidence;
        changed[1]["quote"] = json!("甲用金钥匙打开箱子。");
        assert_eq!(
            evidence_refs_in_rows(&changed, &source, current, Some(&body)).unwrap_err(),
            "SERIES_SUPPLEMENT_SOURCE_QUOTE_CHANGED"
        );
        let mut stale = body.clone();
        stale
            .iter_mut()
            .find(|r| r.id == old.id)
            .unwrap()
            .text
            .push('改');
        assert!(evidence_refs_in_rows(
            &json!([{"rowId":old.id,"quote":old.text},{"rowId":now.id,"quote":now.text}]),
            &source,
            current,
            Some(&stale)
        )
        .is_err());
    }
}
pub(super) fn before<'a>(origin: &'a TextRequest) -> Result<&'a Vec<Value>, String> {
    packet_value(&origin.input["packet"], "assets")?
        .as_array()
        .ok_or_else(|| "SERIES_SUPPLEMENT_BEFORE_ASSETS".into())
}
fn catalogue(assets: &[Value]) -> Value {
    json!(assets
        .iter()
        .map(|a| design::card(a, false))
        .collect::<Vec<_>>())
}

/// A scene detail can use a prop explicitly pinned by that scene's complete
/// design. The attachment is not a new scene or an additional episode use.
pub(super) fn scene_detail_assets(
    ids: &[String],
    before: &[Value],
    revisions: &[Value],
    source: &LockedSource,
) -> bool {
    let scenes: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            before.iter().find(|a| {
                a["id"] == *id
                    && matches!(s(a, "kind"), "SC" | "LOC" | "scene" | "location" | "场景")
            })
        })
        .collect();
    if scenes.is_empty() {
        return false;
    }
    // Preserve the frozen scene-only contract, including an existingSupply
    // identity whose full design is loaded after selection. Only the newly
    // supported attached-prop path requires full design/dependency evidence.
    if ids
        .iter()
        .all(|id| scenes.iter().any(|scene| scene["id"] == *id))
    {
        return true;
    }
    ids.iter().all(|id| {
        let Some(asset) = before.iter().find(|a| a["id"] == *id) else {
            return false;
        };
        let revised = revisions.iter().find(|r| r["assetId"] == *id);
        if !design::adopted(asset, source) {
            return false;
        }
        if scenes.iter().any(|scene| scene["id"] == *id) {
            return true;
        }
        if !matches!(s(asset, "kind"), "PR" | "PROP" | "prop" | "道具") {
            return false;
        }
        let Some(version) = supply_revision::dependency_version(asset, revisions) else {
            return false;
        };
        if revised.is_some_and(|r| {
            r["baseDesignRevision"] != asset["designRevision"]
                || r["baseDesignSha256"] != hash(&asset["adoptedDesign"])
        }) {
            return false;
        }
        scenes.iter().any(|scene| {
            let design = revisions
                .iter()
                .find(|r| r["assetId"] == scene["id"])
                .unwrap_or(&scene["adoptedDesign"]);
            design["designDependencies"]
                .as_array()
                .is_some_and(|dependencies| {
                    dependencies.iter().any(|dependency| {
                        dependency["assetId"] == *id && dependency["designRevision"] == version
                    })
                })
        })
    })
}
pub(super) fn validate_candidate(
    raw: &Value,
    origin: &TextRequest,
    origin_raw: &Value,
    known: &Value,
    identity_context: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
) -> Result<Vec<Value>, String> {
    let revisions = if supply_revision::enabled(identity_context) {
        raw["revisions"]
            .as_array()
            .ok_or("SERIES_DESIGN_REVISIONS_REQUIRED")?
            .as_slice()
    } else {
        &[]
    };
    validate_candidate_with_revisions(
        raw,
        origin,
        origin_raw,
        known,
        identity_context,
        source,
        episode,
        revisions,
        None,
    )
}
/// Shared base validator for explicit v3 revisions and the separately
/// authorized correction. Historical v1/v2 callers always pass no revisions.
pub(super) fn validate_candidate_with_revisions(
    raw: &Value,
    origin: &TextRequest,
    origin_raw: &Value,
    known: &Value,
    identity_context: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    revisions: &[Value],
    supplied_rows: Option<&[SourceRow]>,
) -> Result<Vec<Value>, String> {
    validate_supplement_identity_context(identity_context, origin, origin_raw, known)?;
    let allow_fact_summary =
        supplied_rows.is_some() || origin.input[SOURCE_FACT_CONTRACT_FIELD] == SOURCE_FACT_CONTRACT;
    let v2 = supply::enabled(identity_context);
    if raw["schemaVersion"]
        != if supply_revision::enabled(identity_context) {
            supply_revision::NORMAL_CANDIDATE
        } else if v2 {
            "series-asset-supplement-candidate/v2"
        } else {
            CANDIDATE
        }
    {
        return Err("SERIES_SUPPLEMENT_CANDIDATE_VERSION".into());
    }
    let old = before(origin)?;
    if v2 {
        supply::validate_selection(
            raw,
            old,
            known,
            identity_context,
            source,
            episode,
            revisions,
            allow_fact_summary,
        )?;
    }
    let selected_existing = if v2 { supply::ids(raw)? } else { Vec::new() };
    let known = known
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_CATALOGUE_REQUIRED")?;
    for a in old {
        let expected = if v2 {
            supplement_supply_catalogue(&json!([design::card(a, false)]))[0].clone()
        } else {
            design::card(a, false)
        };
        if !known.iter().any(|k| k == &expected) {
            return Err("SERIES_SUPPLEMENT_CATALOGUE_CHANGED_EXISTING".into());
        }
    }
    let proposals = raw["proposals"]
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_PROPOSALS_REQUIRED")?;
    let resolutions = raw["resolutions"]
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_RESOLUTIONS_REQUIRED")?;
    let unresolved = origin_raw["draft"]["unresolved"]
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_ORIGIN_UNRESOLVED_REQUIRED")?;
    if resolutions.len() != unresolved.len() {
        return Err("SERIES_SUPPLEMENT_ISSUE_COVERAGE".into());
    }
    let mut keys = HashSet::new();
    let mut names = HashSet::new();
    // Keys are opaque, exact references. They enter an asset ID only through
    // SHA-256 below and are never interpreted as paths or normalized.
    let key_pattern = regex::Regex::new(PROPOSAL_KEY_PATTERN).unwrap();
    let mut added = Vec::new();
    for p in proposals {
        let key = s(p, "key");
        let name = s(p, "name");
        if !key_pattern.is_match(key)
            || !keys.insert(key)
            || name.trim().is_empty()
            || name != name.trim()
            || !names.insert(normalized(name))
            || !["CH", "SC", "PR", "BEAST", "OM"].contains(&s(p, "kind"))
        {
            return Err("SERIES_SUPPLEMENT_IDENTITY_INVALID".into());
        }
        if known.iter().any(|a| {
            normalized(s(a, "name")) == normalized(name)
                || a["aliases"].as_array().is_some_and(|vs| {
                    vs.iter().any(|v| {
                        v.as_str()
                            .is_some_and(|v| normalized(v) == normalized(name))
                    })
                })
        }) {
            return Err("SERIES_SUPPLEMENT_DUPLICATE_KNOWN_ASSET".into());
        }
        let mut refs = evidence_refs_in_rows(&p["evidence"], source, episode, supplied_rows)?;
        if !["explicit_source", "production_choice"].contains(&s(p, "basis"))
            || s(p, "description").trim().is_empty()
            || s(p, "full").trim().is_empty()
            || s(p, "brief").trim().is_empty()
            || p["full"] == p["brief"]
        {
            return Err("SERIES_SUPPLEMENT_COMPLETE_DESIGN_REQUIRED".into());
        }
        for fact in p["sourceFacts"]
            .as_array()
            .ok_or("SERIES_SUPPLEMENT_SOURCE_FACTS_REQUIRED")?
        {
            let fact_refs = string_list(&fact["sourceRefs"])?;
            if s(fact, "statement").trim().is_empty()
                || fact_refs.is_empty()
                || fact_refs.iter().any(|id| {
                    !source.rows.iter().any(|r| {
                        r.id == *id
                            && r.kind == "body"
                            && supplied_rows.map_or_else(
                                || r.episode_id.as_deref() == Some(&episode.id),
                                |rows| {
                                    rows.iter().any(|supplied| {
                                        supplied.id == r.id && supplied.text == r.text
                                    })
                                },
                            )
                    })
                })
            {
                return Err("SERIES_SUPPLEMENT_FACT_SOURCE_INVALID".into());
            }
            if !allow_fact_summary
                && !source
                    .rows
                    .iter()
                    .any(|r| fact_refs.contains(&r.id) && r.text.contains(s(fact, "statement")))
            {
                return Err("SERIES_SUPPLEMENT_FACT_NOT_SOURCE_QUOTE".into());
            }
            for id in fact_refs {
                if !refs.contains(&id) {
                    refs.push(id);
                }
            }
        }
        let choices = string_list(&p["productionChoices"])?;
        if p["basis"] == "production_choice" && choices.iter().all(|v| v.trim().is_empty()) {
            return Err("SERIES_SUPPLEMENT_PRODUCTION_CHOICE_UNDECLARED".into());
        }
        for dep in p["designDependencies"]
            .as_array()
            .ok_or("SERIES_SUPPLEMENT_DEPENDENCIES_REQUIRED")?
        {
            if !old.iter().any(|a| {
                a["id"] == dep["assetId"]
                    && design::adopted(a, source)
                    && supply_revision::dependency_version(a, revisions)
                        .is_some_and(|v| Some(v) == dep["designRevision"].as_u64())
            }) {
                return Err("SERIES_SUPPLEMENT_DEPENDENCY_NOT_ADOPTED".into());
            }
        }
        let identity = digest(&format!(
            "{}:{}:{}:{}",
            source.sha256,
            episode.id,
            request_sha(origin)?,
            key
        ));
        let scenes: Vec<_> = source
            .scenes
            .iter()
            .filter(|s| {
                s.episode_id == episode.id
                    && source
                        .rows
                        .iter()
                        .any(|r| refs.contains(&r.id) && r.scene_id.as_deref() == Some(&s.id))
            })
            .map(|s| s.id.clone())
            .collect();
        added.push(json!({"id":format!("series-material-{}",&identity[..24]),"variantId":"base","identityId":format!("series-material-{}",&identity[..24]),
            "name":name,"kind":p["kind"],"description":p["description"],"version":1,
            "sourceSha256":source.sha256,"sourceRefs":refs,"sourceFacts":p["sourceFacts"],
            "applicability":{"episodes":[episode.id],"scenes":scenes},"designRevision":1,
            "workflowProfile":PROFILE,"supplementKey":key}));
    }
    let mut resolved = HashSet::new();
    let mut used = HashSet::new();
    let mut scene_assets = old.clone();
    scene_assets.extend(
        known
            .iter()
            .filter(|a| selected_existing.contains(&a["id"]))
            .cloned(),
    );
    for r in resolutions {
        let index = r["unresolvedIndex"]
            .as_u64()
            .ok_or("SERIES_SUPPLEMENT_ISSUE_INDEX")? as usize;
        if index >= unresolved.len()
            || !resolved.insert(index)
            || s(r, "rationale").trim().is_empty()
        {
            return Err("SERIES_SUPPLEMENT_ISSUE_COVERAGE".into());
        }
        let proposed = string_list(&r["proposalKeys"])?;
        let existing = string_list(&r["existingAssetIds"])?;
        let boundary_only = r["disposition"] == "no_visual_asset"
            && proposed.is_empty()
            && existing.is_empty()
            && r["evidence"] == json!([]);
        if !boundary_only {
            evidence_refs(&r["evidence"], source, episode)?;
        }
        if proposed.iter().any(|k| !keys.contains(k.as_str()))
            || existing.iter().any(|id| {
                !old.iter().any(|a| a["id"] == *id) && !selected_existing.contains(&json!(id))
            })
        {
            return Err("SERIES_SUPPLEMENT_RESOLUTION_UNKNOWN_ASSET".into());
        }
        let valid = match s(r, "disposition") {
            "new_asset" => !proposed.is_empty(),
            "existing_asset" => proposed.is_empty() && !existing.is_empty(),
            "scene_detail" => {
                proposed.is_empty()
                    && !existing.is_empty()
                    && scene_detail_assets(&existing, &scene_assets, revisions, source)
            }
            "no_visual_asset" => proposed.is_empty(),
            _ => false,
        };
        if !valid {
            return Err("SERIES_SUPPLEMENT_RESOLUTION_DISPOSITION".into());
        }
        used.extend(proposed);
    }
    if used.len() != keys.len() {
        return Err("SERIES_SUPPLEMENT_UNUSED_PROPOSAL".into());
    }
    // Reserve all already-known names and ordinals; never rename existing cards.
    let mut all = known.clone();
    for reservation in identity_context["reservedAssets"].as_array().unwrap() {
        if !all.iter().any(|a| a["id"] == reservation["id"]) {
            all.push(reservation.clone());
        }
    }
    let initial = all.len();
    all.extend(added);
    allocate_asset_identities_from(&mut all, identity_context["nextOrdinal"].as_u64().unwrap())?;
    if &all[..known.len()] != known.as_slice() {
        return Err("SERIES_SUPPLEMENT_CHANGED_EXISTING_IDENTITY".into());
    }
    if supply_revision::enabled(identity_context) {
        let rows = supply_revision::selected_rows(origin, raw, source, episode)?;
        supply_revision::validate_revisions(raw, old, &rows, source, None, allow_fact_summary)?;
    }
    Ok(all.into_iter().skip(initial).collect())
}
pub(super) fn validate_review(
    raw: &Value,
    candidate: &Value,
    issue_count: usize,
) -> Result<(), String> {
    let keys: Vec<_> = candidate["proposals"]
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_PROPOSALS_REQUIRED")?
        .iter()
        .map(|p| p["key"].clone())
        .collect();
    let v3 = candidate["schemaVersion"] == supply_revision::NORMAL_CANDIDATE;
    let v2 = candidate["schemaVersion"] == "series-asset-supplement-candidate/v2" || v3;
    if v2 {
        supply::review(raw, candidate)?;
    }
    if raw["schemaVersion"]
        != if v3 {
            supply_revision::NORMAL_REVIEW
        } else if v2 {
            "series-asset-supplement-review/v2"
        } else {
            REVIEW
        }
        || raw["candidateSha256"] != hash(candidate)
        || raw["reviewedKeys"] != json!(keys)
        || raw["reviewedIssueIndexes"] != json!((0..issue_count).collect::<Vec<_>>())
        || s(raw, "summary").trim().is_empty()
    {
        return Err("SERIES_SUPPLEMENT_REVIEW_UNBOUND".into());
    }
    if v3
        && raw["reviewedRevisionAssetIds"]
            != expected_review_coverage(candidate, issue_count)?["reviewedRevisionAssetIds"]
    {
        return Err("SERIES_SUPPLEMENT_REVIEW_REVISION_COVERAGE".into());
    }
    let issues = raw["issues"]
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_REVIEW_ISSUES")?;
    if !matches!(s(raw, "conclusion"), "pass" | "revise")
        || (raw["conclusion"] == "pass") != issues.is_empty()
    {
        return Err("SERIES_SUPPLEMENT_REVIEW_CONCLUSION".into());
    }
    for issue in issues {
        if issue["unresolvedIndex"]
            .as_u64()
            .is_none_or(|n| n >= issue_count as u64)
            || (!s(issue, "proposalKey").is_empty() && !keys.contains(&issue["proposalKey"]))
            || s(issue, "problem").trim().is_empty()
            || s(issue, "requestedChange").trim().is_empty()
        {
            return Err("SERIES_SUPPLEMENT_REVIEW_ISSUE_INVALID".into());
        }
    }
    Ok(())
}
pub(super) fn adopted_added(
    mut added: Vec<Value>,
    candidate: &Value,
    source: &LockedSource,
    design_sha: &Value,
    review_sha: &Value,
) -> Vec<Value> {
    for (a, p) in added
        .iter_mut()
        .zip(candidate["proposals"].as_array().unwrap())
    {
        a["designStatus"] = json!("adopted");
        a["adoptedDesign"] = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":a["id"],"variantId":a["variantId"],
            "designRevision":1,"sourceSha256":source.sha256,"sourceRefs":a["sourceRefs"],"sourceFacts":p["sourceFacts"],"productionChoices":p["productionChoices"],
            "full":p["full"],"brief":p["brief"],"designDependencies":p["designDependencies"],"provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT",
            "adoptionBasis":{"kind":"delegated-text-review","supplementVersion":if candidate["schemaVersion"] == supply_revision::NORMAL_CANDIDATE { supply_revision::VERSION } else if candidate["schemaVersion"] == "series-asset-supplement-candidate/v2" { supply::VERSION } else { VERSION },"designRequestSha256":design_sha,"reviewRequestSha256":review_sha,"candidateSha256":hash(candidate)}});
    }
    added
}
fn contract(
    version: &str,
    origin: &TextRequest,
    origin_raw: &Value,
    phase: &str,
    attempt: usize,
    predecessor: &Value,
) -> Result<Value, String> {
    Ok(
        json!({"version":version,"phase":phase,"originExecutionRoleId":origin.input["executionRoleId"],"originRequestSha256":request_sha(origin)?,
        "originRawSha256":hash(origin_raw),"sourceSha256":origin.input["sourceSha256"],"episodeId":origin.input["episodeId"],
        "beforeAssetsSha256":hash(&json!(before(origin)?)),"round":0,"maximumRounds":1,"attempt":attempt,"maximumAttempts":MAX_REPAIRS,
        "predecessorExecutionRoleId":predecessor["executionRoleId"],"predecessorRequestSha256":predecessor["requestSha256"],"predecessorRawSha256":predecessor["rawSha256"]}),
    )
}
pub(super) fn design_values(
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    origin: &TextRequest,
    raw: &Value,
    known: &Value,
    identity_context: &Value,
) -> Result<Vec<(&'static str, Value)>, String> {
    let mut values = base_values(
        input,
        source,
        episode,
        before(origin)?,
        packet_value(&origin.input["packet"], "entry")?,
    );
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|r| r.kind == "body" && r.episode_id.as_deref() == Some(&episode.id))
        .cloned()
        .collect();
    if input["seriesTransportVersion"] == COMPACT_DESIGN_INPUT || supply::enabled(identity_context)
    {
        let value = &mut values
            .iter_mut()
            .find(|(kind, _)| *kind == "source")
            .unwrap()
            .1;
        value.as_object_mut().unwrap().remove("sourceRows");
        value["sourceRowsBinding"] = json!("designSourceRows");
    }
    values.push(("designSourceRows", source_view(&rows)));
    values.push(("knownAssetCatalog", known.clone()));
    values.push(("identityReservations", identity_context.clone()));
    values.push(("missingMaterialIssues", raw["draft"]["unresolved"].clone()));
    Ok(values)
}
fn add_contract(
    req: &mut TextRequest,
    origin: &TextRequest,
    raw: &Value,
    phase: &str,
    attempt: usize,
    predecessor: &Value,
) -> Result<(), String> {
    let identities = packet_value(&req.input["packet"], "identityReservations")?.clone();
    let v2 = supply::enabled(&identities);
    let v3 = supply_revision::enabled(&identities);
    req.input["seriesAssetSupplement"] = contract(
        if v2 {
            supply::version(&identities)
        } else {
            VERSION
        },
        origin,
        raw,
        phase,
        attempt,
        predecessor,
    )?;
    req.input["task"]=json!(match phase {
        "design"=>"同一冻结任务内部补料：逐条审查原作者未解决项，对照本集完整锁源与全资产目录自主决定是否必须独立资产；通用现场物件可作为已有场景细节，不为清空unresolved强造图片资产。仅必要缺项新增完整文字设计；不得改现有资产或完整设计，不生成图片提示词。sourceFacts必须用原文精确摘句与rowId，原文未明确的刀具/材质等制作选择单独列productionChoices。每项给有据处置与理由；无法成立应给独审具体问题，不伪造。新增ID/PNG名称由本机确定。",
        "design-review"=>"独立非作者补料复核：完整读锁源、已有目录、全部原问题、候选处置与完整设计；核独立图必要性、身份去重、源事实/制作选择分离、场内固定细节与图资产区分、依赖和适用时点。不得为清unresolved冒领设计；有问题给revise和具体位置；无问题才pass。你不写正文也不改稿。",
        _=>"同一冻结任务独立整编：原作者回包是未采用输入证据，保留其有效情绪、主创与演出结构；结合已独审的补料处置及全部完整资产设计完成整集。逐项处理缺项，不简单清空unresolved，不重造已采用设计。按sourceCoordinateDiagnostics把句内坐标修正为sourceRows.spokenSegments.range的绝对UTF16连续子区间，原raw不能改写。用新增正式资产ID和同版名称；普通场内细节按复核处置呈现。直接交完整plan/selection/draft，仍须严格程序核验与独立非作者审稿。"});
    req.input["task"]=json!(format!("{} 若原问题仅说明声窗尚无真实配音/动作演练验收，它属于媒体验收边界：以no_visual_asset处置并在rationale保留该边界，按完整原句和表演联排复核文字声窗即可继续文字交付；不得要求先生成媒体，不得宣称成片自然发声已验收。",s(&req.input,"task")));
    if v2 {
        req.input["task"]=json!(format!("{} 既有目录中的身份不能另建同名卡。若本集 assets 漏供但 knownAssetCatalog 已有该身份，以 existingSupply 明确 assetId、本集精确原文 evidence 与 rationale；identityReservations.assetStates.designUsable=true 必须选 reuse，full/brief 为空、sourceFacts/productionChoices/designDependencies 为空数组，本机随后读取原完整设计供独审和整编，禁止重写。designUsable=false 才选 complete，沿同一身份补齐完整设计并独审，不修改原适用范围。existing_asset/scene_detail 的 existingAssetIds 可指向已供卡或本轮 existingSupply。",s(&req.input,"task")));
        if matches!(phase, "design" | "design-review") {
            req.input["seriesTransportVersion"] = json!(COMPACT_DESIGN_INPUT);
            req.input["designReadContract"]=json!("完整锁源只保存一次：source.sourceRowsBinding 指向本包 designSourceRows。knownAssetCatalog 是全库轻目录，只用于身份选择，不代替完整设计；existingSupplySources 是本机按冻结哈希读取的所选既有资产完整设计，仅独审提供。全部规则与本集已供资产完整设计仍需完整读取。");
        }
        if phase == "finisher" {
            req.input["task"]=json!(format!("{} 既有卡的 applicability 保留历史范围；本轮经独审 existingSupply 的精确原文证据授权本集使用。adoptedDesign.sourceRefs 是设计事实来源，不等于每个分镜单元的用途来源；unit.assetRefs[*].sourceRefs 必须来自该单元 sourceRefs，不能把其他单元或其他集的设计来源抄入当前单元。",s(&req.input,"task")));
        }
    }
    if v3 {
        // Only a newly frozen v3 contract removes the obsolete prohibition.
        let task = s(&req.input, "task").replace(
            "不得改现有资产或完整设计",
            "不得改变现有资产身份；已有完整设计用途不足时沿同身份提交revisions新版",
        );
        req.input["task"] = json!(format!("{task} 正常v3允许add/revise/reuse：已供assets的设计结构有效不代表本集用途充分，可在revisions中沿相同assetId/variantId、baseAssetVersion/baseDesignRevision/baseDesignSha256提出完整新版；必须对应原unresolvedIndexes、有当前源文evidence、说明实际变化与保留约束，sourceFacts与制作选择分开。未变化填revisions=[]；不要为满足字段强改设计，不把同一身份换名新增。existingSupply仍只供漏入本集的既有身份，不能同时revise。独审会对实际revisions展开其明确适用场次的跨集原文；修订需保留既有有效使用、核真实消费者与依赖；新版本经本轮独立设计审查及最终独审后才采用。不能把原文未说明的全过程施工图或未做真实媒体测试变成文字硬门禁。"));
    }
    if req.input[SOURCE_FACT_CONTRACT_FIELD] == SOURCE_FACT_CONTRACT {
        req.input["task"] = json!(s(&req.input, "task").replace(
            "sourceFacts必须用原文精确摘句与rowId",
            "sourceFacts.statement可以概括已供原文事实并明确sourceRefs；evidence.quote才必须逐字摘录并绑定rowId",
        ));
    }
    Ok(())
}
/// Pure domain proof used by the durable SDK adapter before any fresh call.
/// Storage still has to prove exact same-workflow rejected origin, accepted
/// predecessor and no unknown submission; this function never grants retries.
pub(crate) fn validate_supplement_transition(
    request: &TextRequest,
    origin: &TextRequest,
    origin_raw: &Value,
    predecessor_request: Option<&TextRequest>,
    predecessor_raw: Option<&Value>,
    frozen_input: &Value,
) -> Result<(), String> {
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| origin.input["episodeId"] == e.id)
        .ok_or("SERIES_SUPPLEMENT_EPISODE_UNKNOWN")?;
    if !eligible(origin, origin_raw) {
        return Err("SERIES_SUPPLEMENT_ORIGIN_NOT_UNADOPTED_AUTHOR".into());
    }
    validate_role_packet(origin, frozen_input, &source, episode)?;
    validate_role_packet(request, frozen_input, &source, episode)?;
    if request.input["freshContext"] != true || request.input["workflowProfile"] != PROFILE {
        return Err("SERIES_SUPPLEMENT_CONTEXT_ISOLATION_REQUIRED".into());
    }
    let c = &request.input["seriesAssetSupplement"];
    let p = &request.input["packet"];
    let known = packet_value(p, "knownAssetCatalog")?;
    let identity_context = packet_value(p, "identityReservations")?;
    let v2 = supply::enabled(identity_context);
    let v3 = supply_revision::enabled(identity_context);
    validate_supplement_identity_context(identity_context, origin, origin_raw, known)?;
    let phase = s(c, "phase");
    let attempt = c["attempt"]
        .as_u64()
        .ok_or("SERIES_SUPPLEMENT_ATTEMPT_MISSING")? as usize;
    let (prior, prior_raw) = (
        predecessor_request.ok_or("SERIES_SUPPLEMENT_PREDECESSOR_MISSING")?,
        predecessor_raw.ok_or("SERIES_SUPPLEMENT_PREDECESSOR_RAW_MISSING")?,
    );
    if attempt > MAX_REPAIRS
        || *c
            != contract(
                if v2 {
                    supply::version(identity_context)
                } else {
                    VERSION
                },
                origin,
                origin_raw,
                phase,
                attempt,
                &role_receipt(prior, prior_raw)?,
            )?
        || request.model != origin.model
        || request.reasoning != origin.reasoning
    {
        return Err("SERIES_SUPPLEMENT_CONTRACT_UNBOUND".into());
    }
    let role = match phase {
        "design" => "design",
        "design-review" => "design-review",
        "finisher" => "finisher",
        _ => return Err("SERIES_SUPPLEMENT_PHASE_UNKNOWN".into()),
    };
    let stage = match phase {
        "design" => "seriesDesign",
        "design-review" => "seriesDesignReview",
        _ => "seriesFinisher",
    };
    let expected_id = format!(
        "{}/{role}/supplement-{}/{attempt}",
        episode.id,
        &request_sha(origin)?[..16]
    );
    if request.stage != stage
        || request.skill != format!("series-{role}")
        || request.input["executionRoleId"] != expected_id
    {
        return Err("SERIES_SUPPLEMENT_ROLE_IDENTITY".into());
    }
    if packet_value(p, "entry")? != packet_value(&origin.input["packet"], "entry")?
        || packet_value(p, "missingMaterialIssues")? != &origin_raw["draft"]["unresolved"]
    {
        return Err("SERIES_SUPPLEMENT_FROZEN_INPUT_CHANGED".into());
    }
    // Catalogue identities originate in the frozen input or in the exact
    // author's prior published cards. Mutable media/history are never input.
    let mut frozen_assets = frozen_input["assets"]
        .as_array()
        .ok_or("SERIES_ASSETS_MISSING")?
        .clone();
    allocate_asset_identities(&mut frozen_assets)?;
    for known_asset in known
        .as_array()
        .ok_or("SERIES_SUPPLEMENT_CATALOGUE_REQUIRED")?
    {
        let expected = before(origin)?
            .iter()
            .find(|a| a["id"] == known_asset["id"])
            .or_else(|| frozen_assets.iter().find(|a| a["id"] == known_asset["id"]));
        // V2 may supply an identity published by an earlier episode after the
        // original workflow input froze. Its state and reservation were frozen
        // by the durable executor, not inferred or generated by the model.
        if v2 {
            if !identity_context["reservedAssets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| supplement_identity_reservation(known_asset).as_ref() == Some(r))
                && !export::is_voice_only(known_asset)
            {
                return Err("SERIES_SUPPLEMENT_CATALOGUE_UNKNOWN".into());
            }
            if let Some(expected) = expected {
                let projected =
                    supplement_supply_catalogue(&json!([design::card(expected, false)]));
                // Active metadata and adopted revisions may reflect a prior
                // episode publication; permanent identity must stay exact.
                for key in [
                    "id",
                    "variantId",
                    "identityId",
                    "baseAssetId",
                    "parentAssetId",
                    "name",
                    "kind",
                    "type",
                    "importName",
                    "filename",
                    "placeholderOrdinal",
                ] {
                    if known_asset[key] != projected[0][key] {
                        return Err("SERIES_SUPPLEMENT_CATALOGUE_CHANGED_EXISTING".into());
                    }
                }
            }
        } else if *known_asset
            != design::card(
                expected.ok_or("SERIES_SUPPLEMENT_CATALOGUE_UNKNOWN")?,
                false,
            )
        {
            return Err("SERIES_SUPPLEMENT_CATALOGUE_CHANGED_EXISTING".into());
        }
    }
    if frozen_assets
        .iter()
        .any(|a| !known.as_array().unwrap().iter().any(|k| k["id"] == a["id"]))
    {
        return Err("SERIES_SUPPLEMENT_CATALOGUE_INCOMPLETE".into());
    }
    let count = origin_raw["draft"]["unresolved"].as_array().unwrap().len();
    if v3 && matches!(phase, "design" | "design-review") {
        let rows = if phase == "design" && attempt == 0 {
            source
                .rows
                .iter()
                .filter(|r| r.kind == "body" && r.episode_id.as_deref() == Some(&episode.id))
                .cloned()
                .collect()
        } else {
            let candidate = if phase == "design-review" {
                prior_raw
            } else {
                packet_value(&prior.input["packet"], "supplementCandidate")?
            };
            supply_revision::selected_rows(origin, candidate, &source, episode)?
        };
        if packet_value(p, "designSourceRows")? != &source_view(&rows) {
            return Err("SERIES_SUPPLEMENT_REVISION_SOURCE_USES_CHANGED".into());
        }
    }
    if phase != "design" || attempt != 0 {
        validate_role_packet(prior, frozen_input, &source, episode)?;
        for key in [
            "version",
            "originExecutionRoleId",
            "originRequestSha256",
            "originRawSha256",
            "sourceSha256",
            "episodeId",
            "beforeAssetsSha256",
            "round",
            "maximumRounds",
            "maximumAttempts",
        ] {
            if prior.input["seriesAssetSupplement"][key] != c[key] {
                return Err("SERIES_SUPPLEMENT_PREDECESSOR_ORIGIN_CHANGED".into());
            }
        }
        if packet_value(&prior.input["packet"], "knownAssetCatalog")? != known {
            return Err("SERIES_SUPPLEMENT_PREDECESSOR_CATALOGUE_CHANGED".into());
        }
        if packet_value(&prior.input["packet"], "identityReservations")? != identity_context {
            return Err("SERIES_SUPPLEMENT_RESERVATIONS_CHANGED".into());
        }
    }
    if phase == "design" {
        if (request.schema != effective_candidate_schema(identity_context)
            && (v3 || request.schema != legacy_candidate_schema(identity_context)))
            || packet_value(p, "assets")? != &json!(before(origin)?)
        {
            return Err("SERIES_SUPPLEMENT_DESIGN_INPUT_CHANGED".into());
        }
        if attempt == 0 {
            if request_sha(prior)? != request_sha(origin)? || prior_raw != origin_raw {
                return Err("SERIES_SUPPLEMENT_FIRST_PREDECESSOR".into());
            }
        } else {
            if prior.input["seriesAssetSupplement"]["phase"] != "design-review"
                || prior.input["seriesAssetSupplement"]["attempt"] != attempt - 1
                || prior_raw["conclusion"] != "revise"
            {
                return Err("SERIES_SUPPLEMENT_REPAIR_PREDECESSOR".into());
            }
            let candidate = packet_value(&prior.input["packet"], "supplementCandidate")?;
            validate_review(prior_raw, candidate, count)?;
            if packet_value(p, "supplementCandidate")? != candidate
                || packet_value(p, "supplementIssues")? != &prior_raw["issues"]
            {
                return Err("SERIES_SUPPLEMENT_REPAIR_INPUT_CHANGED".into());
            }
        }
    } else {
        validate_role_packet(prior, frozen_input, &source, episode)?;
        if prior.input["seriesAssetSupplement"]["originRequestSha256"] != c["originRequestSha256"]
            || prior.input["seriesAssetSupplement"]["attempt"] != c["attempt"]
            || packet_value(&prior.input["packet"], "knownAssetCatalog")? != known
        {
            return Err("SERIES_SUPPLEMENT_PREDECESSOR_CHANGED".into());
        }
        let candidate = packet_value(p, "supplementCandidate")?;
        let added = validate_candidate(
            candidate,
            origin,
            origin_raw,
            known,
            identity_context,
            &source,
            episode,
        )?;
        if phase == "design-review" {
            let without_coverage = legacy_supplement_coverage_request(request)?;
            let expected_schema = if v3 {
                let coverage = expected_review_coverage(candidate, count)?;
                if request.input["reviewDigestBinding"] != REVIEW_DIGEST
                    || request.input["reviewCoverageBinding"] != REVIEW_COVERAGE
                    || request.input["candidateSha256"] != hash(candidate)
                    || request.input["expectedReviewCoverage"] != coverage
                    || request.input["reviewResponseContract"] != REVIEW_RESPONSE_CONTRACT
                {
                    return Err("SERIES_SUPPLEMENT_REVIEW_COVERAGE_UNBOUND".into());
                }
                coverage_review_schema(identity_context, &hash(candidate), &coverage)
            } else if without_coverage.is_some() {
                coverage_review_schema(
                    identity_context,
                    &hash(candidate),
                    &expected_review_coverage(candidate, count)?,
                )
            } else if request.input.get("reviewDigestBinding").is_some() {
                if request.input["reviewDigestBinding"] != REVIEW_DIGEST
                    || request.input["candidateSha256"] != hash(candidate)
                {
                    return Err("SERIES_SUPPLEMENT_REVIEW_DIGEST_UNBOUND".into());
                }
                effective_review_schema(identity_context, Some(&hash(candidate)))
            } else {
                effective_review_schema(identity_context, None)
            };
            if prior.stage != "seriesDesign"
                || prior.input["seriesAssetSupplement"]["phase"] != "design"
                || candidate != prior_raw
                || request.schema != expected_schema
                || packet_value(p, "assets")? != &json!(before(origin)?)
            {
                return Err("SERIES_SUPPLEMENT_REVIEW_PREDECESSOR".into());
            }
            if v2 {
                supply::validate_source_cards(
                    candidate,
                    packet_value(p, "existingSupplySources")?,
                    identity_context,
                    &source,
                )?;
            }
        } else {
            if prior.stage != "seriesDesignReview"
                || prior.input["seriesAssetSupplement"]["phase"] != "design-review"
                || prior_raw["conclusion"] != "pass"
                || packet_value(&prior.input["packet"], "supplementCandidate")? != candidate
            {
                return Err("SERIES_SUPPLEMENT_FINISH_PREDECESSOR".into());
            }
            validate_review(prior_raw, candidate, count)?;
            let added = adopted_added(
                added,
                candidate,
                &source,
                &prior.input["seriesAssetSupplement"]["predecessorRequestSha256"],
                &json!(request_sha(prior)?),
            );
            let revised = if v3 {
                supply_revision::adopted(
                    candidate,
                    origin,
                    &source,
                    &prior.input["seriesAssetSupplement"]["predecessorRequestSha256"],
                    &json!(request_sha(prior)?),
                )?
            } else {
                vec![]
            };
            let mut expected = supply_revision::revised_before(origin, &revised)?;
            if v2 {
                expected.extend(supply::supplied(
                    candidate,
                    packet_value(&prior.input["packet"], "existingSupplySources")?,
                    identity_context,
                    &source,
                    &prior.input["seriesAssetSupplement"]["predecessorRequestSha256"],
                    &json!(request_sha(prior)?),
                )?);
            }
            expected.extend(added.iter().map(|a| design::card(a, true)));
            if packet_value(p, "assets")? != &json!(expected)
                || request.schema
                    != super::candidate_schema(
                        &source,
                        episode,
                        &expected,
                        fantasy_mode(frozen_input)?,
                    )
                || packet_value(p, "sourceCoordinateDiagnostics")?
                    != &source_coordinate_diagnostics(&source, episode, origin_raw)
                || packet_value(p, "supplementReview")? != prior_raw
            {
                return Err("SERIES_SUPPLEMENT_FINISH_INPUT_CHANGED".into());
            }
            for key in ["plan", "selection", "draft"] {
                if packet_value(p, key)? != &origin_raw[key] {
                    return Err("SERIES_SUPPLEMENT_ORIGINAL_AUTHOR_CHANGED".into());
                }
            }
        }
    }
    Ok(())
}

/// Recheck the exact v1 first design raw after the two source/boundary
/// false-positive validators were corrected. It grants no transport retry and
/// never edits raw. The executor must bind the precise historical rejection.
pub(crate) fn revalidate_supplement_design_response(
    request: &TextRequest,
    raw: &Value,
    origin: &TextRequest,
    origin_raw: &Value,
    frozen_input: &Value,
) -> Result<(), String> {
    if request.stage != "seriesDesign"
        || request.input["seriesAssetSupplement"]["version"] != VERSION
        || request.input["seriesAssetSupplement"]["phase"] != "design"
        || request.input["seriesAssetSupplement"]["attempt"] != 0
    {
        return Err("SERIES_SUPPLEMENT_REVALIDATION_SCOPE".into());
    }
    validate_supplement_transition(
        request,
        origin,
        origin_raw,
        Some(origin),
        Some(origin_raw),
        frozen_input,
    )?;
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| request.input["episodeId"] == e.id)
        .ok_or("SERIES_SUPPLEMENT_EPISODE_UNKNOWN")?;
    validate_candidate(
        raw,
        origin,
        origin_raw,
        packet_value(&request.input["packet"], "knownAssetCatalog")?,
        packet_value(&request.input["packet"], "identityReservations")?,
        &source,
        episode,
    )?;
    Ok(())
}

/// Recheck the exact first v2 design that the old hidden key restriction
/// rejected. The SDK executor must prove that precise old rejection and raw;
/// this validates only a candidate and does not authorize adoption or dispatch.
pub(crate) fn revalidate_supplement_key_response(
    request: &TextRequest,
    raw: &Value,
    origin: &TextRequest,
    origin_raw: &Value,
    frozen_input: &Value,
) -> Result<(), String> {
    let identities = packet_value(&request.input["packet"], "identityReservations")?;
    if request.stage != "seriesDesign"
        || request.input["seriesAssetSupplement"]["version"] != supply::VERSION
        || request.input["seriesAssetSupplement"]["phase"] != "design"
        || request.input["seriesAssetSupplement"]["attempt"] != 0
        || !supply::enabled(identities)
        || request.schema != legacy_candidate_schema(identities)
        || !raw["proposals"]
            .as_array()
            .is_some_and(|items| items.iter().any(|p| s(p, "key").contains('-')))
    {
        return Err("SERIES_SUPPLEMENT_KEY_REVALIDATION_SCOPE".into());
    }
    validate_supplement_transition(
        request,
        origin,
        origin_raw,
        Some(origin),
        Some(origin_raw),
        frozen_input,
    )?;
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| request.input["episodeId"] == e.id)
        .ok_or("SERIES_SUPPLEMENT_EPISODE_UNKNOWN")?;
    validate_candidate(
        raw,
        origin,
        origin_raw,
        packet_value(&request.input["packet"], "knownAssetCatalog")?,
        identities,
        &source,
        episode,
    )?;
    Ok(())
}

pub(super) struct Completed {
    pub proof: Value,
    pub cards: Vec<Value>,
    pub adoptions: Vec<Value>,
    pub receipts: Vec<Value>,
    pub finisher: TextRequest,
}
pub(super) async fn run(
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &mut Vec<Value>,
    origin_receipt: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Completed, String> {
    let origin: TextRequest = serde_json::from_value(origin_receipt["request"].clone())
        .map_err(|_| "SERIES_SUPPLEMENT_ORIGIN_REQUEST")?;
    let raw = &origin_receipt["raw"];
    if let Some(context) = executor
        .series_supply_correction_context(&origin, raw)
        .await?
    {
        return supply_correction::run(
            input,
            source,
            episode,
            assets,
            origin_receipt,
            &context,
            executor,
            model,
            reasoning,
        )
        .await;
    }
    let known_full = catalogue(assets);
    let desired = if input[supply_revision::INPUT_FIELD] == supply_revision::VERSION {
        supply_revision::VERSION
    } else {
        supply::VERSION
    };
    let identity_context = executor
        .series_supplement_identity_context(&origin, raw, &known_full, desired)
        .await?;
    let v2 = supply::enabled(&identity_context);
    let v3 = supply_revision::enabled(&identity_context);
    if desired == supply_revision::VERSION && !v3 {
        return Err("SERIES_SUPPLEMENT_V3_CONTEXT_REQUIRED".into());
    }
    let known = if v2 {
        supplement_supply_catalogue(&known_full)
    } else {
        known_full
    };
    validate_supplement_identity_context(&identity_context, &origin, raw, &known)?;
    let base = design_values(
        input,
        source,
        episode,
        &origin,
        raw,
        &known,
        &identity_context,
    )?;
    let mut receipts = Vec::new();
    let mut rounds = Vec::new();
    let mut previous = origin_receipt.clone();
    let mut last_problem = String::new();
    let mut previous_candidate = Value::Null;
    for attempt in 0..=MAX_REPAIRS {
        let iteration = format!("supplement-{}/{attempt}", &request_sha(&origin)?[..16]);
        let mut values = base.clone();
        if attempt > 0 {
            values.push(("supplementCandidate", previous_candidate.clone()));
            values.push(("supplementIssues", previous["raw"]["issues"].clone()));
            if v3 {
                supply_revision::add_source_rows(
                    &mut values,
                    &origin,
                    &previous_candidate,
                    source,
                    episode,
                )?;
            }
        }
        let mut req = request(
            "design",
            "seriesDesign",
            &episode.id,
            &iteration,
            input,
            values,
            effective_candidate_schema(&identity_context),
            model,
            reasoning,
        )?;
        add_contract(&mut req, &origin, raw, "design", attempt, &previous)?;
        let prior: TextRequest = serde_json::from_value(previous["request"].clone())
            .map_err(|_| "SERIES_SUPPLEMENT_PREDECESSOR_REQUEST")?;
        validate_supplement_transition(
            &req,
            &origin,
            raw,
            Some(&prior),
            Some(&previous["raw"]),
            input,
        )?;
        let (candidate, creation) = call(executor, req, |v| {
            validate_candidate(v, &origin, raw, &known, &identity_context, source, episode)?;
            if v2 {
                supply::source_cards(v, assets, &identity_context, source)?;
            }
            Ok(v.clone())
        })
        .await?;
        receipts.push(creation.clone());
        let mut values = base.clone();
        values.push(("supplementCandidate", candidate.clone()));
        if v3 {
            supply_revision::add_source_rows(&mut values, &origin, &candidate, source, episode)?;
        }
        let existing_sources = if v2 {
            supply::source_cards(&candidate, assets, &identity_context, source)?
        } else {
            Value::Null
        };
        if v2 {
            values.push(("existingSupplySources", existing_sources.clone()));
        }
        let mut req = request(
            "design-review",
            "seriesDesignReview",
            &episode.id,
            &iteration,
            input,
            values,
            effective_review_schema(&identity_context, None),
            model,
            reasoning,
        )?;
        add_contract(&mut req, &origin, raw, "design-review", attempt, &creation)?;
        req.input["reviewDigestBinding"] = json!(REVIEW_DIGEST);
        req.input["candidateSha256"] = json!(hash(&candidate));
        let coverage = expected_review_coverage(
            &candidate,
            raw["draft"]["unresolved"].as_array().unwrap().len(),
        )?;
        req.input["reviewCoverageBinding"] = json!(REVIEW_COVERAGE);
        req.input["expectedReviewCoverage"] = coverage.clone();
        req.input["reviewResponseContract"] = json!(REVIEW_RESPONSE_CONTRACT);
        req.schema = coverage_review_schema(&identity_context, &hash(&candidate), &coverage);
        let req = executor.prepare_transport_request(req).await?;
        let prior: TextRequest = serde_json::from_value(creation["request"].clone())
            .map_err(|_| "SERIES_SUPPLEMENT_PREDECESSOR_REQUEST")?;
        validate_supplement_transition(&req, &origin, raw, Some(&prior), Some(&candidate), input)?;
        let (review, review_receipt) = call(executor, req, |v| {
            validate_review(
                v,
                &candidate,
                raw["draft"]["unresolved"].as_array().unwrap().len(),
            )?;
            Ok(v.clone())
        })
        .await?;
        receipts.push(review_receipt.clone());
        rounds.push(json!({"designRequestSha256":creation["requestSha256"],"reviewRequestSha256":review_receipt["requestSha256"]}));
        if review["conclusion"] == "pass" {
            let added = adopted_added(
                validate_candidate(
                    &candidate,
                    &origin,
                    raw,
                    &known,
                    &identity_context,
                    source,
                    episode,
                )?,
                &candidate,
                source,
                &creation["requestSha256"],
                &review_receipt["requestSha256"],
            );
            let mut extended = assets.clone();
            let revised = if v3 {
                supply_revision::adopted(
                    &candidate,
                    &origin,
                    source,
                    &creation["requestSha256"],
                    &review_receipt["requestSha256"],
                )?
            } else {
                vec![]
            };
            supply_revision::install(&mut extended, &revised)?;
            let supplied = if v2 {
                supply::supplied(
                    &candidate,
                    &existing_sources,
                    &identity_context,
                    source,
                    &creation["requestSha256"],
                    &review_receipt["requestSha256"],
                )?
            } else {
                Vec::new()
            };
            if v2 {
                supply::install_in_memory(&mut extended, &supplied)?;
            }
            extended.extend(added.iter().cloned());
            let mut expected = supply_revision::revised_before(&origin, &revised)?;
            expected.extend(supplied.iter().cloned());
            expected.extend(added.iter().map(|a| design::card(a, true)));
            let cards = if v2 {
                expected.clone()
            } else {
                design::cards(source, episode, &extended)?
            };
            if cards != expected {
                return Err("SERIES_SUPPLEMENT_EXISTING_DESIGNS_CHANGED".into());
            }
            if v3 {
                let usage = supply::episode_use(&candidate, &episode.id)?;
                design::cards(
                    source,
                    episode,
                    &supply::scoped_assets(source, episode, &cards, Some(&usage))?,
                )?;
            }
            let mut values = base_values(
                input,
                source,
                episode,
                &cards,
                packet_value(&origin.input["packet"], "entry")?,
            );
            values.extend([
                ("plan", raw["plan"].clone()),
                ("selection", raw["selection"].clone()),
                ("draft", raw["draft"].clone()),
                ("knownAssetCatalog", known.clone()),
                ("identityReservations", identity_context.clone()),
                ("missingMaterialIssues", raw["draft"]["unresolved"].clone()),
                ("supplementCandidate", candidate.clone()),
                ("supplementReview", review.clone()),
                (
                    "sourceCoordinateDiagnostics",
                    source_coordinate_diagnostics(source, episode, raw),
                ),
            ]);
            let mut finisher = request(
                "finisher",
                "seriesFinisher",
                &episode.id,
                &iteration,
                input,
                values,
                super::candidate_schema(source, episode, &cards, fantasy_mode(input)?),
                model,
                reasoning,
            )?;
            add_contract(
                &mut finisher,
                &origin,
                raw,
                "finisher",
                attempt,
                &review_receipt,
            )?;
            let prior: TextRequest = serde_json::from_value(review_receipt["request"].clone())
                .map_err(|_| "SERIES_SUPPLEMENT_PREDECESSOR_REQUEST")?;
            validate_supplement_transition(
                &finisher,
                &origin,
                raw,
                Some(&prior),
                Some(&review),
                input,
            )?;
            let mut adoptions:Vec<_>=added.iter().map(|a|json!({"assetId":a["id"],"variantId":a["variantId"],"baseDesignRevision":0,"designRevision":1,
                "adoptedDesign":a["adoptedDesign"],"designReview":{"requestSha256":review_receipt["requestSha256"],"resultSha256":review_receipt["rawSha256"],"issues":[]}})).collect();
            if v2 {
                for (choice, a) in candidate["existingSupply"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(&supplied)
                {
                    if choice["designMode"] == "complete" {
                        adoptions.push(json!({"assetId":a["id"],"variantId":a["variantId"],"baseDesignRevision":a["designRevision"].as_u64().unwrap()-1,"designRevision":a["designRevision"],"adoptedDesign":a["adoptedDesign"],"designReview":{"requestSha256":review_receipt["requestSha256"],"resultSha256":review_receipt["rawSha256"],"issues":[]}}));
                    }
                }
            }
            for a in &revised {
                adoptions.push(json!({"assetId":a["id"],"variantId":a["variantId"],"baseDesignRevision":a["designRevision"].as_u64().unwrap()-1,
                    "designRevision":a["designRevision"],"adoptedDesign":a["adoptedDesign"],"designReview":{"requestSha256":review_receipt["requestSha256"],"resultSha256":review_receipt["rawSha256"],"issues":[]}}));
            }
            let mut proof = json!({"version":if v2{supply::version(&identity_context)}else{VERSION},"origin":origin_receipt,"beforeAssets":before(&origin)?,"knownAssetCatalog":known,"identityReservations":identity_context,
                "addedAssets":added,"rounds":rounds,"finisherExecutionRoleId":finisher.input["executionRoleId"],"finisherRequest":finisher});
            if v2 {
                proof["suppliedExistingAssets"] = json!(supplied);
                proof["episodeSupply"] = supply::episode_use(&candidate, &episode.id)?;
            }
            if v3 {
                proof["revisedAssets"] = json!(revised);
            }
            *assets = extended;
            return Ok(Completed {
                proof,
                cards,
                adoptions,
                receipts,
                finisher,
            });
        }
        let problem = hash(&review["issues"]);
        if attempt == MAX_REPAIRS || problem == last_problem {
            return Err("SERIES_SUPPLEMENT_REVIEW_REPAIR_HELD".into());
        }
        last_problem = problem;
        previous_candidate = candidate;
        previous = review_receipt;
    }
    Err("SERIES_SUPPLEMENT_REPAIR_LIMIT".into())
}

pub(super) fn validate_delivery(input: &Value, delivery: &Value) -> Result<bool, String> {
    let proof = &delivery["adoptionReceipt"]["assetSupplement"];
    if proof.is_null() {
        return Ok(false);
    }
    let v3 = proof["version"] == supply_revision::VERSION;
    let v2 = proof["version"] == supply::VERSION || v3;
    if !v2 && proof["version"] != VERSION {
        return Err("SERIES_SUPPLEMENT_PROOF_VERSION".into());
    }
    let origin_receipt = &proof["origin"];
    let origin: TextRequest = serde_json::from_value(origin_receipt["request"].clone())
        .map_err(|_| "SERIES_SUPPLEMENT_ORIGIN_REQUEST")?;
    let raw = &origin_receipt["raw"];
    let mut expected_receipt = role_receipt(&origin, raw)?;
    expected_receipt["validation"] = json!("rejected");
    expected_receipt["reason"] = origin_receipt["reason"].clone();
    if &expected_receipt != origin_receipt
        || s(origin_receipt, "reason").is_empty()
        || !eligible(&origin, raw)
        || proof["beforeAssets"] != json!(before(&origin)?)
    {
        return Err("SERIES_SUPPLEMENT_ORIGIN_EVIDENCE_CHANGED".into());
    }
    let roles = delivery["adoptionReceipt"]["roleReceipts"]
        .as_array()
        .ok_or("SERIES_ADOPTION_ROLE_RECEIPTS_MISSING")?;
    let rounds = proof["rounds"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= MAX_REPAIRS + 1)
        .ok_or("SERIES_SUPPLEMENT_ROUNDS_INVALID")?;
    let find = |sha: &Value| -> Result<&Value, String> {
        let values: Vec<_> = roles
            .iter()
            .filter(|r| r["requestSha256"] == *sha)
            .collect();
        if values.len() != 1 {
            return Err("SERIES_SUPPLEMENT_RECEIPT_NOT_UNIQUE".into());
        }
        Ok(values[0])
    };
    let mut previous = origin_receipt;
    let mut final_creation = &Value::Null;
    let mut last_problem = String::new();
    for (attempt, round) in rounds.iter().enumerate() {
        for (phase, sha) in [
            ("design", &round["designRequestSha256"]),
            ("design-review", &round["reviewRequestSha256"]),
        ] {
            let r = find(sha)?;
            let req: TextRequest = serde_json::from_value(r["request"].clone())
                .map_err(|_| "SERIES_SUPPLEMENT_REQUEST_INVALID")?;
            let prior: TextRequest = serde_json::from_value(previous["request"].clone())
                .map_err(|_| "SERIES_SUPPLEMENT_PREDECESSOR_REQUEST")?;
            if req.input["seriesAssetSupplement"]["phase"] != phase
                || req.input["seriesAssetSupplement"]["version"] != proof["version"]
                || req.input["seriesAssetSupplement"]["attempt"] != attempt
                || packet_value(&req.input["packet"], "knownAssetCatalog")?
                    != &proof["knownAssetCatalog"]
                || packet_value(&req.input["packet"], "identityReservations")?
                    != &proof["identityReservations"]
            {
                return Err("SERIES_SUPPLEMENT_ROUND_CHAIN_CHANGED".into());
            }
            validate_supplement_transition(
                &req,
                &origin,
                raw,
                Some(&prior),
                Some(&previous["raw"]),
                input,
            )?;
            if phase == "design" {
                final_creation = r;
            } else {
                validate_review(
                    &r["raw"],
                    &final_creation["raw"],
                    raw["draft"]["unresolved"].as_array().unwrap().len(),
                )?;
                if r["raw"]["conclusion"] == "revise" {
                    let problem = hash(&r["raw"]["issues"]);
                    if attempt + 1 == rounds.len() || problem == last_problem {
                        return Err("SERIES_SUPPLEMENT_REVIEW_NOT_PASSED".into());
                    }
                    last_problem = problem;
                } else if attempt + 1 != rounds.len() {
                    return Err("SERIES_SUPPLEMENT_REDUNDANT_ROUND".into());
                }
            }
            previous = r;
        }
    }
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| delivery["episodeId"] == e.id)
        .ok_or("SERIES_DELIVERY_EPISODE_UNKNOWN")?;
    let added = adopted_added(
        validate_candidate(
            &final_creation["raw"],
            &origin,
            raw,
            &proof["knownAssetCatalog"],
            &proof["identityReservations"],
            &source,
            episode,
        )?,
        &final_creation["raw"],
        &source,
        &final_creation["requestSha256"],
        &previous["requestSha256"],
    );
    let revised = if v3 {
        supply_revision::adopted(
            &final_creation["raw"],
            &origin,
            &source,
            &final_creation["requestSha256"],
            &previous["requestSha256"],
        )?
    } else {
        vec![]
    };
    if v3 && proof["revisedAssets"] != json!(revised) {
        return Err("SERIES_SUPPLEMENT_REVISED_ASSETS_CHANGED".into());
    }
    if v3 {
        supply_revision::validate_adoptions(
            input,
            delivery,
            &origin,
            &revised,
            final_creation,
            previous,
            &proof["identityReservations"],
        )?;
    }
    let mut expected = supply_revision::revised_before(&origin, &revised)?;
    if v2 {
        if proof["episodeSupply"] != supply::episode_use(&final_creation["raw"], &episode.id)? {
            return Err("SERIES_SUPPLY_EPISODE_USE_CHANGED".into());
        }
        let supplied = supply::supplied(
            &final_creation["raw"],
            packet_value(
                &previous["request"]["input"]["packet"],
                "existingSupplySources",
            )?,
            &proof["identityReservations"],
            &source,
            &final_creation["requestSha256"],
            &previous["requestSha256"],
        )?;
        if proof["suppliedExistingAssets"] != json!(supplied) {
            return Err("SERIES_SUPPLY_PROOF_CHANGED".into());
        }
        expected.extend(supplied);
    }
    expected.extend(added.iter().map(|a| design::card(a, true)));
    if proof["addedAssets"] != json!(added) || delivery["bundle"]["assets"] != json!(expected) {
        return Err("SERIES_SUPPLEMENT_NOT_APPEND_ONLY".into());
    }
    let finisher: TextRequest = serde_json::from_value(proof["finisherRequest"].clone())
        .map_err(|_| "SERIES_SUPPLEMENT_FINISH_REQUEST")?;
    if finisher.input["executionRoleId"] != proof["finisherExecutionRoleId"] {
        return Err("SERIES_SUPPLEMENT_FINISH_ID_CHANGED".into());
    }
    // A rejected finisher's bounded domain repair can carry the identical
    // supplement packet. Its original request is reconstructed from the repair
    // proof and still must match the frozen candidate/schema below.
    let finishers: Vec<_> = roles
        .iter()
        .filter(|r| {
            r["request"] == json!(finisher)
                || (r["stage"] == "seriesRepair"
                    && r["role"] == "finisher"
                    && r["request"]["input"]["validationRepair"]["originRequestSha256"]
                        == request_sha(&finisher).unwrap_or_default()
                    && r["request"]["input"]["packet"] == finisher.input["packet"]
                    && r["request"]["schema"] == finisher.schema)
        })
        .collect();
    if finishers.len() != 1 {
        return Err("SERIES_SUPPLEMENT_FINISH_RECEIPT_REQUIRED".into());
    }
    let prior: TextRequest = serde_json::from_value(previous["request"].clone())
        .map_err(|_| "SERIES_SUPPLEMENT_PREDECESSOR_REQUEST")?;
    validate_supplement_transition(
        &finisher,
        &origin,
        raw,
        Some(&prior),
        Some(&previous["raw"]),
        input,
    )?;
    Ok(true)
}
