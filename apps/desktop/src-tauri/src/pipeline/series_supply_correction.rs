//! One explicitly authorized correction of the legacy immutable-design supply contract.
//! The original attempts, rejected author and raws remain independent evidence.
use super::*;
pub(super) const VERSION: &str = "series-supply-contract-correction/v1";
const AUTHORIZATION: &str = "controller-verified-supply-contract-gap/v1";
const MAXIMUM_ATTEMPTS: usize = 1;

fn req(receipt: &Value) -> Result<TextRequest, String> {
    serde_json::from_value(receipt["request"].clone())
        .map_err(|_| "SERIES_SUPPLY_CORRECTION_RECEIPT_REQUEST".into())
}
fn sha(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
}
fn legacy(context: &Value) -> Result<&Vec<Value>, String> {
    context["legacyReceipts"]
        .as_array()
        .ok_or_else(|| "SERIES_SUPPLY_CORRECTION_LEGACY_REQUIRED".into())
}
fn origins(context: &Value) -> Result<(&Value, &Value), String> {
    let packet = &legacy(context)?
        .first()
        .ok_or("SERIES_SUPPLY_CORRECTION_LEGACY_REQUIRED")?["request"]["input"]["packet"];
    Ok((
        packet_value(packet, "knownAssetCatalog")?,
        packet_value(packet, "identityReservations")?,
    ))
}
pub(super) fn allowed_keys(context: &Value) -> Result<Vec<Value>, String> {
    let mut keys = Vec::new();
    for receipt in legacy(context)?.iter().filter(|r| r["role"] == "design") {
        for proposal in receipt["raw"]["proposals"]
            .as_array()
            .ok_or("SERIES_SUPPLY_CORRECTION_LEGACY_PROPOSALS")?
        {
            if !keys.contains(&proposal["key"]) {
                keys.push(proposal["key"].clone());
            }
        }
    }
    Ok(keys)
}

/// Pure proof only. The durable executor separately proves workflow ownership,
/// the authorization ledger, stopped state, exact job raws and no UNKNOWN.
pub(crate) fn validate_supply_correction_context(
    context: &Value,
    origin: &TextRequest,
    origin_raw: &Value,
    frozen_input: &Value,
) -> Result<(), String> {
    if !supplement::eligible(origin, origin_raw)
        || origin.input.get("seriesSupplyCorrection").is_some()
    {
        return Err("SERIES_SUPPLY_CORRECTION_ORIGIN_INVALID".into());
    }
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| origin.input["episodeId"] == e.id)
        .ok_or("SERIES_SUPPLY_CORRECTION_EPISODE_UNKNOWN")?;
    validate_role_packet(origin, frozen_input, &source, episode)?;
    let receipts = legacy(context)?;
    let authorization = &context["authorization"];
    let expected = json!({"version":VERSION,"workflowId":context["workflowId"],
        "originRequestSha256":supplement::request_sha(origin)?,"originRawSha256":hash(origin_raw),
        "legacyFailureCode":"SERIES_SUPPLEMENT_REVIEW_REPAIR_HELD","legacyReceipts":receipts,
        "legacyReceiptsSha256":hash(&json!(receipts)),"authorization":authorization,
        "maximumAttempts":MAXIMUM_ATTEMPTS,"maximumCorrections":1});
    if context != &expected
        || s(context, "workflowId").trim().is_empty()
        || receipts.len() != (MAX_REPAIRS + 1) * 2
        || authorization["kind"] != AUTHORIZATION
        || authorization["classification"] != "existing_design_revision_unsupported"
        || !sha(&authorization["auditSha256"])
    {
        return Err("SERIES_SUPPLY_CORRECTION_AUTHORIZATION_INVALID".into());
    }
    let mut previous = role_receipt(origin, origin_raw)?;
    let mut previous_problem = String::new();
    for (index, receipt) in receipts.iter().enumerate() {
        let request = req(receipt)?;
        let prior = req(&previous)?;
        let phase = if index % 2 == 0 {
            "design"
        } else {
            "design-review"
        };
        if role_receipt(&request, &receipt["raw"])? != *receipt
            || request.input["seriesAssetSupplement"]["phase"] != phase
            || request.input["seriesAssetSupplement"]["attempt"] != index / 2
            || request.schema["properties"].get("revisions").is_some()
            || request
                .schema
                .pointer("/properties/existingSupply/items/properties/designMode/enum")
                .and_then(Value::as_array)
                .is_some_and(|v| v.contains(&json!("revise")))
        {
            return Err("SERIES_SUPPLY_CORRECTION_LEGACY_CHAIN_CHANGED".into());
        }
        supplement::validate_supplement_transition(
            &request,
            origin,
            origin_raw,
            Some(&prior),
            Some(&previous["raw"]),
            frozen_input,
        )?;
        if phase == "design-review" {
            supplement::validate_review(
                &receipt["raw"],
                &previous["raw"],
                origin_raw["draft"]["unresolved"].as_array().unwrap().len(),
            )?;
            let problem = hash(&receipt["raw"]["issues"]);
            if receipt["raw"]["conclusion"] != "revise" || problem == previous_problem {
                return Err("SERIES_SUPPLY_CORRECTION_LEGACY_NOT_EXHAUSTED".into());
            }
            previous_problem = problem;
        }
        previous = receipt.clone();
    }
    let affected = authorization["affectedAssets"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("SERIES_SUPPLY_CORRECTION_AFFECTED_REQUIRED")?;
    if authorization
        != &json!({"kind":AUTHORIZATION,"classification":"existing_design_revision_unsupported",
        "auditSha256":authorization["auditSha256"],"affectedAssets":affected})
    {
        return Err("SERIES_SUPPLY_CORRECTION_AUTHORIZATION_INVALID".into());
    }
    let before = supplement::before(origin)?;
    let targets: Vec<_> = before
        .iter()
        .filter(|a| affected.iter().any(|v| v["assetId"] == a["id"]))
        .cloned()
        .collect();
    let use_rows = design_source::rows(frozen_input, &source, episode, &targets)?;
    let mut ids = HashSet::new();
    for affected in affected {
        let asset = before
            .iter()
            .find(|a| a["id"] == affected["assetId"] && a["variantId"] == affected["variantId"])
            .ok_or("SERIES_SUPPLY_CORRECTION_AFFECTED_NOT_SUPPLIED")?;
        if !ids.insert(s(affected, "assetId"))
            || !design::adopted(asset, &source)
            || affected["baseAssetVersion"] != asset["version"]
            || asset["version"].as_u64().is_none_or(|v| v == 0)
            || affected["baseDesignRevision"] != asset["designRevision"]
            || affected["baseDesignRevision"] != asset["adoptedDesign"]["designRevision"]
            || affected["baseDesignSha256"] != hash(&asset["adoptedDesign"])
        {
            return Err("SERIES_SUPPLY_CORRECTION_AFFECTED_BASE_CHANGED".into());
        }
        let refs = string_list(&affected["sourceRefs"])?;
        let indexes = affected["unresolvedIndexes"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("SERIES_SUPPLY_CORRECTION_AFFECTED_ISSUES")?;
        let mut seen = HashSet::new();
        if refs.is_empty()
            || refs.iter().any(|id| !use_rows.iter().any(|r| r.id == *id))
            || !refs.iter().any(|id| {
                use_rows
                    .iter()
                    .any(|r| r.id == *id && r.episode_id.as_deref() == Some(&episode.id))
            })
            || indexes.iter().any(|i| {
                i.as_u64().is_none_or(|n| {
                    n >= origin_raw["draft"]["unresolved"].as_array().unwrap().len() as u64
                }) || !seen.insert(i.to_string())
            })
        {
            return Err("SERIES_SUPPLY_CORRECTION_AFFECTED_SOURCE_UNBOUND".into());
        }
        let basis = receipts
            .iter()
            .find(|r| {
                r["requestSha256"] == affected["basisReviewRequestSha256"]
                    && r["role"] == "design-review"
            })
            .ok_or("SERIES_SUPPLY_CORRECTION_REVIEW_BASIS_MISSING")?;
        let issue = affected["basisReviewIssueIndex"]
            .as_u64()
            .and_then(|i| basis["raw"]["issues"].get(i as usize))
            .ok_or("SERIES_SUPPLY_CORRECTION_REVIEW_BASIS_MISSING")?;
        if !indexes.contains(&issue["unresolvedIndex"]) {
            return Err("SERIES_SUPPLY_CORRECTION_REVIEW_BASIS_UNBOUND".into());
        }
        if affected
            != &json!({"assetId":asset["id"],"variantId":asset["variantId"],"baseAssetVersion":asset["version"],
            "baseDesignRevision":asset["designRevision"],"baseDesignSha256":hash(&asset["adoptedDesign"]),
            "unresolvedIndexes":indexes,"sourceRefs":affected["sourceRefs"],
            "basisReviewRequestSha256":basis["requestSha256"],"basisReviewIssueIndex":affected["basisReviewIssueIndex"]})
        {
            return Err("SERIES_SUPPLY_CORRECTION_AFFECTED_FIELDS".into());
        }
    }
    Ok(())
}

fn contract(
    context: &Value,
    origin: &TextRequest,
    raw: &Value,
    phase: &str,
    attempt: usize,
    previous: &Value,
) -> Result<Value, String> {
    Ok(
        json!({"version":VERSION,"contextSha256":hash(context),"originExecutionRoleId":origin.input["executionRoleId"],
        "originRequestSha256":supplement::request_sha(origin)?,"originRawSha256":hash(raw),"phase":phase,"attempt":attempt,
        "maximumAttempts":MAXIMUM_ATTEMPTS,"maximumCorrections":1,
        "predecessorExecutionRoleId":previous["executionRoleId"],"predecessorRequestSha256":previous["requestSha256"],"predecessorRawSha256":previous["rawSha256"]}),
    )
}
fn materials(
    candidate: &Value,
    existing_sources: &Value,
    context: &Value,
    origin: &TextRequest,
    raw: &Value,
    input: &Value,
    design_sha: &Value,
    review_sha: &Value,
) -> Result<(Vec<Value>, Vec<Value>, Vec<Value>, Vec<Value>), String> {
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| origin.input["episodeId"] == e.id)
        .unwrap();
    let (known, identities) = origins(context)?;
    let added = supply_revision::validate(
        candidate, context, origin, raw, known, identities, &source, episode, input,
    )?;
    let mut added = supplement::adopted_added(added, candidate, &source, design_sha, review_sha);
    for a in &mut added {
        a["adoptedDesign"]["adoptionBasis"]
            .as_object_mut()
            .unwrap()
            .remove("supplementVersion");
        a["adoptedDesign"]["adoptionBasis"]["correctionVersion"] = json!(VERSION);
    }
    let mut supplied = if supply::enabled(identities) {
        supply::supplied(
            candidate,
            existing_sources,
            identities,
            &source,
            design_sha,
            review_sha,
        )?
    } else {
        vec![]
    };
    for a in &mut supplied {
        if candidate["existingSupply"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["assetId"] == a["id"] && c["designMode"] == "complete")
        {
            a["adoptedDesign"]["adoptionBasis"]
                .as_object_mut()
                .unwrap()
                .remove("supplementVersion");
            a["adoptedDesign"]["adoptionBasis"]["correctionVersion"] = json!(VERSION);
        }
    }
    let revised = supply_revision::adopted(candidate, origin, &source, design_sha, review_sha)?;
    let mut cards = supplement::before(origin)?.clone();
    for revised in &revised {
        *cards.iter_mut().find(|a| a["id"] == revised["id"]).unwrap() = revised.clone();
    }
    cards.extend(supplied.iter().cloned());
    cards.extend(added.iter().map(|a| design::card(a, true)));
    // Includes cycle checks and pinning every dependency to the revised graph.
    // Use the projected episode use only for selected existing supply.
    let usage = if supply::enabled(identities) {
        Some(supply::episode_use(candidate, &episode.id)?)
    } else {
        None
    };
    let scoped = supply::scoped_assets(&source, episode, &cards, usage.as_ref())?;
    design::cards(&source, episode, &scoped)?;
    Ok((cards, revised, supplied, added))
}

fn request_for(
    context: &Value,
    origin: &TextRequest,
    raw: &Value,
    input: &Value,
    phase: &str,
    attempt: usize,
    previous: &Value,
    existing_sources: &Value,
) -> Result<TextRequest, String> {
    let source = source(input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| origin.input["episodeId"] == e.id)
        .ok_or("SERIES_SUPPLY_CORRECTION_EPISODE_UNKNOWN")?;
    let (known, identities) = origins(context)?;
    let affected = context["authorization"]["affectedAssets"]
        .as_array()
        .unwrap();
    let targets: Vec<_> = supplement::before(origin)?
        .iter()
        .filter(|a| affected.iter().any(|v| v["assetId"] == a["id"]))
        .cloned()
        .collect();
    let use_rows = design_source::rows(input, &source, episode, &targets)?;
    let mut values =
        supplement::design_values(input, &source, episode, origin, raw, known, identities)?;
    values
        .iter_mut()
        .find(|(k, _)| *k == "designSourceRows")
        .unwrap()
        .1 = source_view(&use_rows);
    let source_binding = &mut values.iter_mut().find(|(k, _)| *k == "source").unwrap().1;
    source_binding.as_object_mut().unwrap().remove("sourceRows");
    source_binding["sourceRowsBinding"] = json!("designSourceRows");
    let basis: Vec<_> = affected.iter().map(|a| {
        let receipt = legacy(context).unwrap().iter().find(|r| r["requestSha256"] == a["basisReviewRequestSha256"]).unwrap();
        json!({"assetId":a["assetId"],"reviewRequestSha256":receipt["requestSha256"],
            "issue":receipt["raw"]["issues"][a["basisReviewIssueIndex"].as_u64().unwrap() as usize]})
    }).collect();
    let authorization = json!({"version":VERSION,"contextSha256":hash(context),"legacyReceiptsSha256":context["legacyReceiptsSha256"],
        "authorization":context["authorization"],"reviewBasis":basis,"allowedProposalKeys":allowed_keys(context)?});
    values.push(("supplyCorrectionAuthorization", authorization.clone()));
    let count = raw["draft"]["unresolved"].as_array().unwrap().len();
    let (stage, schema) = match phase {
        "design" => {
            if attempt > 0 {
                values.push((
                    "supplementCandidate",
                    packet_value(
                        &previous["request"]["input"]["packet"],
                        "supplementCandidate",
                    )?
                    .clone(),
                ));
                values.push(("supplementIssues", previous["raw"]["issues"].clone()));
            }
            ("seriesDesign", supply_revision::schema(identities, context))
        }
        "design-review" => {
            values.push(("supplementCandidate", previous["raw"].clone()));
            values.push(("existingSupplySources", existing_sources.clone()));
            (
                "seriesDesignReview",
                supply_revision::review_schema(&previous["raw"], count),
            )
        }
        "finisher" => {
            let candidate = packet_value(
                &previous["request"]["input"]["packet"],
                "supplementCandidate",
            )?;
            let (cards, _, _, _) = materials(
                candidate,
                existing_sources,
                context,
                origin,
                raw,
                input,
                &previous["request"]["input"]["seriesSupplyCorrection"]["predecessorRequestSha256"],
                &previous["requestSha256"],
            )?;
            values = base_values(
                input,
                &source,
                episode,
                &cards,
                packet_value(&origin.input["packet"], "entry")?,
            );
            values.extend([
                ("knownAssetCatalog", known.clone()),
                ("identityReservations", identities.clone()),
                ("missingMaterialIssues", raw["draft"]["unresolved"].clone()),
                ("supplyCorrectionAuthorization", authorization),
                ("supplementCandidate", candidate.clone()),
                ("supplementReview", previous["raw"].clone()),
                ("existingSupplySources", existing_sources.clone()),
                ("plan", raw["plan"].clone()),
                ("selection", raw["selection"].clone()),
                ("draft", raw["draft"].clone()),
                (
                    "sourceCoordinateDiagnostics",
                    source_coordinate_diagnostics(&source, episode, raw),
                ),
            ]);
            (
                "seriesFinisher",
                super::candidate_schema(&source, episode, &cards, fantasy_mode(input)?),
            )
        }
        _ => return Err("SERIES_SUPPLY_CORRECTION_PHASE_INVALID".into()),
    };
    let iteration = format!(
        "supply-correction-{}/{attempt}",
        &supplement::request_sha(origin)?[..16]
    );
    let mut request = request(
        phase,
        stage,
        &episode.id,
        &iteration,
        input,
        values,
        schema,
        &origin.model,
        &origin.reasoning,
    )?;
    request.input["seriesSupplyCorrection"] =
        contract(context, origin, raw, phase, attempt, previous)?;
    request.input[design_source::FIELD] = json!(design_source::VERSION);
    request.input["task"] = json!(match phase {
        "design" => "纠正旧供料合同对同资产revise的遗漏：在授权affectedAssets内沿同一assetId/variantId提出完整新版设计，绑定原版本和哈希；保持源事实、身份和已用有效约束，解决本集用途缺口，不把结构有效误当作用途充分。完整阅读本集原文和designSourceRows中显式相关跨集动作，区分源事实、制作选择和历史防止凭空增能的限定。允许局部合理补足，不为消除unresolved强造复杂设备、施工图或未入镜全过程。revisions列实际变化、保留约束、有据用途及对应原问题；无须保留未采用的旧失败候选。proposals只可复用allowedProposalKeys，不扩新资产范围；已有身份不能换名另建。既有完整设计以原全文为基准，不以brief替代。图像提示词与媒体不在本次工作范围。",
        "design-review" => "独立非作者复核供料合同纠正：逐条核原问题、同资产旧完整设计与revisions新版、原版本哈希、完整本集及显式跨集用途；严格分开源事实和制作选择，保留已用约束，核功能/空间/依赖可用性与影响，不能因程序允许revise就默认通过。不要把原文未说明的全过程机械施工或未完成的真实媒体验收变为文字硬门禁。完整独审后逐项复制expectedReviewCoverage原序数组、candidateSha256；summary及issue字段须有非空具体内容。有问题conclusion=revise且issues非空，无问题才pass且issues=[]。",
        _ => "同一冻结任务独立整编：原作者回包仍为未采用证据；保留有效情绪、完整主创与演出结构，采用独审通过的同身份新版完整设计、补料处置及正式资产ID完成整集。逐项解决缺项，不简单清空unresolved。adoptedDesign.sourceRefs是设计事实来源，不等于当前镜头用途；unit.assetRefs.sourceRefs仅取本单元原文。使用sourceCoordinateDiagnostics修正绝对UTF16原话坐标，原raw不可改写。保持原词/说者/声道和出场顺序；文字交付仍需程序检查与最终独立非作者审稿，不宣称真实图片/配音/视频验收。"
    });
    if matches!(phase, "design" | "design-review") {
        request.input["seriesTransportVersion"] = json!(COMPACT_DESIGN_INPUT);
        request.input["designReadContract"] = json!("source.sourceRowsBinding指向designSourceRows，本集完整body与受授权资产显式相关跨集原文均已按源序提供。assets保留旧完整设计；designUsable只表示结构有效，不表示当前用途充分。授权和旧独审问题为工程证据，不是新剧情事实；不读取或改写旧原回包。");
    }
    if phase == "design-review" {
        request.input["candidateSha256"] = json!(hash(&previous["raw"]));
        request.input["expectedReviewCoverage"] =
            supply_revision::coverage(&previous["raw"], count);
    }
    Ok(request)
}

// A short-lived proof over immutably borrowed complete values. It cannot be
// constructed outside this module, outlive a run/delivery validation, or cache
// any mutable database, UNKNOWN, authorization-ledger or CAS state.
struct VerifiedContext<'a> {
    context: &'a Value,
    origin: &'a TextRequest,
    origin_raw: &'a Value,
    frozen_input: &'a Value,
}
impl<'a> VerifiedContext<'a> {
    fn new(
        context: &'a Value,
        origin: &'a TextRequest,
        origin_raw: &'a Value,
        frozen_input: &'a Value,
    ) -> Result<Self, String> {
        validate_supply_correction_context(context, origin, origin_raw, frozen_input)?;
        Ok(Self {
            context,
            origin,
            origin_raw,
            frozen_input,
        })
    }
}

pub(crate) fn validate_supply_correction_transition(
    request: &TextRequest,
    context: &Value,
    origin: &TextRequest,
    origin_raw: &Value,
    predecessor_request: Option<&TextRequest>,
    predecessor_raw: Option<&Value>,
    frozen_input: &Value,
) -> Result<(), String> {
    let verified = VerifiedContext::new(context, origin, origin_raw, frozen_input)?;
    validate_verified_transition(request, &verified, predecessor_request, predecessor_raw)
}

fn validate_verified_transition(
    request: &TextRequest,
    verified: &VerifiedContext<'_>,
    predecessor_request: Option<&TextRequest>,
    predecessor_raw: Option<&Value>,
) -> Result<(), String> {
    let VerifiedContext {
        context,
        origin,
        origin_raw,
        frozen_input,
    } = *verified;
    let prior = predecessor_request.ok_or("SERIES_SUPPLY_CORRECTION_PREDECESSOR_REQUIRED")?;
    let prior_raw = predecessor_raw.ok_or("SERIES_SUPPLY_CORRECTION_PREDECESSOR_RAW_REQUIRED")?;
    let previous = role_receipt(prior, prior_raw)?;
    let c = &request.input["seriesSupplyCorrection"];
    let phase = s(c, "phase");
    let attempt = c["attempt"]
        .as_u64()
        .filter(|n| *n <= MAXIMUM_ATTEMPTS as u64)
        .ok_or("SERIES_SUPPLY_CORRECTION_ATTEMPT_LIMIT")? as usize;
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| origin.input["episodeId"] == e.id)
        .unwrap();
    validate_role_packet(request, frozen_input, &source, episode)?;
    validate_role_packet(prior, frozen_input, &source, episode)?;
    let count = origin_raw["draft"]["unresolved"].as_array().unwrap().len();
    let (known, identities) = origins(context)?;
    let existing_sources = if phase == "design-review" {
        packet_value(&request.input["packet"], "existingSupplySources")?.clone()
    } else if phase == "finisher" {
        packet_value(&prior.input["packet"], "existingSupplySources")?.clone()
    } else {
        Value::Null
    };
    if phase == "design" && attempt == 0 {
        if legacy(context)?.last() != Some(&previous) {
            return Err("SERIES_SUPPLY_CORRECTION_FIRST_PREDECESSOR".into());
        }
    } else {
        let pc = &prior.input["seriesSupplyCorrection"];
        let expected_phase = if phase == "design-review" {
            "design"
        } else {
            "design-review"
        };
        let expected_attempt = if phase == "design" {
            attempt - 1
        } else {
            attempt
        };
        if pc["version"] != VERSION
            || pc["contextSha256"] != hash(context)
            || pc["originRequestSha256"] != supplement::request_sha(origin)?
            || pc["phase"] != expected_phase
            || pc["attempt"] != expected_attempt
        {
            return Err("SERIES_SUPPLY_CORRECTION_PREDECESSOR_CHANGED".into());
        }
        let candidate = if phase == "design-review" {
            prior_raw
        } else {
            packet_value(&prior.input["packet"], "supplementCandidate")?
        };
        supply_revision::validate(
            candidate,
            context,
            origin,
            origin_raw,
            known,
            identities,
            &source,
            episode,
            frozen_input,
        )?;
        if phase == "design-review" && supply::enabled(identities) {
            supply::validate_source_cards(candidate, &existing_sources, identities, &source)?;
        }
        if phase != "design-review" {
            supply_revision::review(prior_raw, candidate, count)?;
            if prior_raw["conclusion"] != if phase == "design" { "revise" } else { "pass" } {
                return Err("SERIES_SUPPLY_CORRECTION_PREDECESSOR_CONCLUSION".into());
            }
        }
    }
    let expected = request_for(
        context,
        origin,
        origin_raw,
        frozen_input,
        phase,
        attempt,
        &previous,
        &existing_sources,
    )?;
    if serde_json::to_value(request).map_err(|e| e.to_string())? != json!(expected) {
        return Err("SERIES_SUPPLY_CORRECTION_REQUEST_CHANGED".into());
    }
    Ok(())
}

/// A summary was rejected by an application-only substring check, even though
/// the frozen field/rules allowed a source-bound fact summary. This is candidate
/// validation only: the original independent review and budgets still apply.
pub(crate) fn revalidate_supply_correction_source_facts_response(
    request: &TextRequest,
    candidate: &Value,
    context: &Value,
    origin: &TextRequest,
    origin_raw: &Value,
    prior: &TextRequest,
    prior_raw: &Value,
    frozen_input: &Value,
) -> Result<(), String> {
    if request.stage != "seriesDesign"
        || request.skill != "series-design"
        || request.input["seriesSupplyCorrection"]["version"] != VERSION
        || request.input["seriesSupplyCorrection"]["phase"] != "design"
        || request.input["seriesSupplyCorrection"]["attempt"] != 0
        || request.input.get("validationRepair").is_some()
        || request.input.get("seriesAssetSupplement").is_some()
    {
        return Err("SERIES_SUPPLY_FACT_REVALIDATION_SCOPE".into());
    }
    validate_supply_correction_transition(
        request,
        context,
        origin,
        origin_raw,
        Some(prior),
        Some(prior_raw),
        frozen_input,
    )?;
    let source = source(frozen_input)?;
    let episode = source
        .episodes
        .iter()
        .find(|e| request.input["episodeId"] == e.id)
        .ok_or("SERIES_SUPPLY_CORRECTION_EPISODE_UNKNOWN")?;
    let (known, identities) = origins(context)?;
    supply_revision::validate(
        candidate,
        context,
        origin,
        origin_raw,
        known,
        identities,
        &source,
        episode,
        frozen_input,
    )?;
    // This policy does not turn an unrelated historical rejection into a pass.
    let has_summary = ["proposals", "revisions"].iter().any(|field| {
        candidate[*field]
            .as_array()
            .into_iter()
            .flatten()
            .any(|item| {
                item["sourceFacts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|fact| {
                        let historic = *field == "revisions"
                            && supplement::before(origin).is_ok_and(|before| {
                                before.iter().any(|asset| {
                                    asset["id"] == item["assetId"]
                                        && asset["adoptedDesign"]["sourceFacts"]
                                            .as_array()
                                            .is_some_and(|facts| facts.contains(fact))
                                })
                            });
                        !historic
                            && fact["sourceRefs"]
                                .as_array()
                                .is_some_and(|refs| !refs.is_empty())
                            && !source.rows.iter().any(|row| {
                                fact["sourceRefs"]
                                    .as_array()
                                    .unwrap()
                                    .contains(&json!(row.id))
                                    && row.text.contains(s(fact, "statement"))
                            })
                    })
            })
    });
    if !has_summary {
        return Err("SERIES_SUPPLY_FACT_REVALIDATION_NOT_APPLICABLE".into());
    }
    Ok(())
}

pub(super) async fn run(
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &mut Vec<Value>,
    origin_receipt: &Value,
    context: &Value,
    executor: &dyn TextExecutor,
    _model: &str,
    _reasoning: &str,
) -> Result<supplement::Completed, String> {
    let origin = req(origin_receipt)?;
    let raw = &origin_receipt["raw"];
    let verified = VerifiedContext::new(context, &origin, raw, input)?;
    let (known, identities) = origins(context)?;
    let mut previous = legacy(context)?.last().unwrap().clone();
    let mut receipts = Vec::new();
    let mut rounds = Vec::new();
    let mut last_problem = String::new();
    for attempt in 0..=MAXIMUM_ATTEMPTS {
        let creation_request = request_for(
            context,
            &origin,
            raw,
            input,
            "design",
            attempt,
            &previous,
            &Value::Null,
        )?;
        validate_verified_transition(
            &creation_request,
            &verified,
            Some(&req(&previous)?),
            Some(&previous["raw"]),
        )?;
        let (candidate, creation) = call(executor, creation_request, |candidate| {
            supply_revision::validate(
                candidate, context, &origin, raw, known, identities, source, episode, input,
            )?;
            if supply::enabled(identities) {
                supply::source_cards(candidate, assets, identities, source)?;
            }
            Ok(candidate.clone())
        })
        .await?;
        let existing_sources = if supply::enabled(identities) {
            supply::source_cards(&candidate, assets, identities, source)?
        } else {
            Value::Null
        };
        receipts.push(creation.clone());
        let review_request = request_for(
            context,
            &origin,
            raw,
            input,
            "design-review",
            attempt,
            &creation,
            &existing_sources,
        )?;
        validate_verified_transition(
            &review_request,
            &verified,
            Some(&req(&creation)?),
            Some(&candidate),
        )?;
        let (review, review_receipt) = call(executor, review_request, |review| {
            supply_revision::review(
                review,
                &candidate,
                raw["draft"]["unresolved"].as_array().unwrap().len(),
            )?;
            Ok(review.clone())
        })
        .await?;
        receipts.push(review_receipt.clone());
        rounds.push(json!({"designRequestSha256":creation["requestSha256"],"reviewRequestSha256":review_receipt["requestSha256"]}));
        if review["conclusion"] == "pass" {
            let (cards, revised, supplied, added) = materials(
                &candidate,
                &existing_sources,
                context,
                &origin,
                raw,
                input,
                &creation["requestSha256"],
                &review_receipt["requestSha256"],
            )?;
            let finisher = request_for(
                context,
                &origin,
                raw,
                input,
                "finisher",
                attempt,
                &review_receipt,
                &existing_sources,
            )?;
            validate_verified_transition(
                &finisher,
                &verified,
                Some(&req(&review_receipt)?),
                Some(&review),
            )?;
            let mut adoptions = Vec::new();
            for a in revised
                .iter()
                .chain(&added)
                .chain(supplied.iter().filter(|a| {
                    candidate["existingSupply"].as_array().is_some_and(|items| {
                        items
                            .iter()
                            .any(|c| c["assetId"] == a["id"] && c["designMode"] == "complete")
                    })
                }))
            {
                let revision = a["designRevision"].as_u64().unwrap();
                adoptions.push(json!({"assetId":a["id"],"variantId":a["variantId"],"baseDesignRevision":revision-1,
                    "designRevision":revision,"adoptedDesign":a["adoptedDesign"],"designReview":{"requestSha256":review_receipt["requestSha256"],"resultSha256":review_receipt["rawSha256"],"issues":[]}}));
            }
            let mut proof = json!({"version":VERSION,"context":context,"origin":origin_receipt,"beforeAssets":supplement::before(&origin)?,
                "knownAssetCatalog":known,"identityReservations":identities,"revisedAssets":revised,"suppliedExistingAssets":supplied,
                "addedAssets":added,"rounds":rounds,"roleReceipts":receipts,
                "finisherExecutionRoleId":finisher.input["executionRoleId"],"finisherRequest":finisher});
            if supply::enabled(identities) {
                proof["episodeSupply"] = supply::episode_use(&candidate, &episode.id)?;
            }
            for a in &revised {
                let current = assets
                    .iter_mut()
                    .find(|v| v["id"] == a["id"])
                    .ok_or("SERIES_SUPPLY_CORRECTION_ASSET_MISSING")?;
                for field in ["adoptedDesign", "designRevision", "designStatus"] {
                    current[field] = a[field].clone();
                }
            }
            supply::install_in_memory(assets, &supplied)?;
            assets.extend(added);
            return Ok(supplement::Completed {
                proof,
                cards,
                adoptions,
                receipts,
                finisher,
            });
        }
        let problem = hash(&review["issues"]);
        if attempt == MAXIMUM_ATTEMPTS || problem == last_problem {
            return Err("SERIES_SUPPLY_CORRECTION_REPAIR_HELD".into());
        }
        last_problem = problem;
        previous = review_receipt;
    }
    Err("SERIES_SUPPLY_CORRECTION_ATTEMPT_LIMIT".into())
}

pub(super) fn validate_delivery(input: &Value, delivery: &Value) -> Result<bool, String> {
    let proof = &delivery["adoptionReceipt"]["supplyContractCorrection"];
    if proof.is_null() {
        return Ok(false);
    }
    if proof["version"] != VERSION || !delivery["adoptionReceipt"]["assetSupplement"].is_null() {
        return Err("SERIES_SUPPLY_CORRECTION_PROOF_VERSION".into());
    }
    let context = &proof["context"];
    let origin_receipt = &proof["origin"];
    let origin = req(origin_receipt)?;
    let raw = &origin_receipt["raw"];
    let mut expected_origin = role_receipt(&origin, raw)?;
    expected_origin["validation"] = json!("rejected");
    expected_origin["reason"] = origin_receipt["reason"].clone();
    if expected_origin != *origin_receipt || s(origin_receipt, "reason").is_empty() {
        return Err("SERIES_SUPPLY_CORRECTION_ORIGIN_EVIDENCE_CHANGED".into());
    }
    let verified = VerifiedContext::new(context, &origin, raw, input)?;
    let (known, identities) = origins(context)?;
    if proof["beforeAssets"] != json!(supplement::before(&origin)?)
        || &proof["knownAssetCatalog"] != known
        || &proof["identityReservations"] != identities
    {
        return Err("SERIES_SUPPLY_CORRECTION_PROOF_INPUT_CHANGED".into());
    }
    let roles = delivery["adoptionReceipt"]["roleReceipts"]
        .as_array()
        .ok_or("SERIES_ADOPTION_ROLE_RECEIPTS_MISSING")?;
    let find = |sha: &Value| -> Result<&Value, String> {
        let matches: Vec<_> = roles
            .iter()
            .filter(|r| r["requestSha256"] == *sha)
            .collect();
        if matches.len() != 1 {
            return Err("SERIES_SUPPLY_CORRECTION_RECEIPT_NOT_UNIQUE".into());
        }
        Ok(matches[0])
    };
    let rounds = proof["rounds"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= MAXIMUM_ATTEMPTS + 1)
        .ok_or("SERIES_SUPPLY_CORRECTION_ROUNDS_INVALID")?;
    let mut previous = legacy(context)?.last().unwrap();
    let mut final_creation = &Value::Null;
    let mut correction_receipts = Vec::new();
    for (attempt, round) in rounds.iter().enumerate() {
        for (phase, sha) in [
            ("design", &round["designRequestSha256"]),
            ("design-review", &round["reviewRequestSha256"]),
        ] {
            let receipt = find(sha)?;
            let request = req(receipt)?;
            if request.input["seriesSupplyCorrection"]["phase"] != phase
                || request.input["seriesSupplyCorrection"]["attempt"] != attempt
                || role_receipt(&request, &receipt["raw"])? != *receipt
            {
                return Err("SERIES_SUPPLY_CORRECTION_ROUND_CHAIN_CHANGED".into());
            }
            validate_verified_transition(
                &request,
                &verified,
                Some(&req(previous)?),
                Some(&previous["raw"]),
            )?;
            if phase == "design" {
                final_creation = receipt;
            } else {
                supply_revision::review(
                    &receipt["raw"],
                    &final_creation["raw"],
                    raw["draft"]["unresolved"].as_array().unwrap().len(),
                )?;
                if (receipt["raw"]["conclusion"] == "pass") != (attempt + 1 == rounds.len()) {
                    return Err("SERIES_SUPPLY_CORRECTION_REVIEW_NOT_PASSED".into());
                }
            }
            correction_receipts.push(receipt.clone());
            previous = receipt;
        }
    }
    if proof["roleReceipts"] != json!(correction_receipts) {
        return Err("SERIES_SUPPLY_CORRECTION_ROLE_EVIDENCE_CHANGED".into());
    }
    let (cards, revised, supplied, added) = materials(
        &final_creation["raw"],
        packet_value(
            &previous["request"]["input"]["packet"],
            "existingSupplySources",
        )?,
        context,
        &origin,
        raw,
        input,
        &final_creation["requestSha256"],
        &previous["requestSha256"],
    )?;
    supply_revision::validate_adoptions(
        input,
        delivery,
        &origin,
        &revised,
        final_creation,
        previous,
        identities,
    )?;
    if delivery["bundle"]["assets"] != json!(cards)
        || proof["revisedAssets"] != json!(revised)
        || proof["suppliedExistingAssets"] != json!(supplied)
        || proof["addedAssets"] != json!(added)
    {
        return Err("SERIES_SUPPLY_CORRECTION_ADOPTION_CHANGED".into());
    }
    if supply::enabled(identities)
        && proof["episodeSupply"]
            != supply::episode_use(&final_creation["raw"], s(&origin.input, "episodeId"))?
    {
        return Err("SERIES_SUPPLY_CORRECTION_EPISODE_USE_CHANGED".into());
    }
    let finisher: TextRequest = serde_json::from_value(proof["finisherRequest"].clone())
        .map_err(|_| "SERIES_SUPPLY_CORRECTION_FINISH_REQUEST")?;
    let finishers: Vec<_> = roles
        .iter()
        .filter(|r| {
            r["request"] == json!(finisher)
                || (r["stage"] == "seriesRepair"
                    && r["role"] == "finisher"
                    && r["request"]["input"]["validationRepair"]["originRequestSha256"]
                        == supplement::request_sha(&finisher).unwrap_or_default()
                    && r["request"]["input"]["packet"] == finisher.input["packet"]
                    && r["request"]["schema"] == finisher.schema)
        })
        .collect();
    if finishers.len() != 1 || proof["finisherExecutionRoleId"] != finisher.input["executionRoleId"]
    {
        return Err("SERIES_SUPPLY_CORRECTION_FINISH_RECEIPT_REQUIRED".into());
    }
    validate_verified_transition(
        &finisher,
        &verified,
        Some(&req(previous)?),
        Some(&previous["raw"]),
    )?;
    Ok(true)
}
