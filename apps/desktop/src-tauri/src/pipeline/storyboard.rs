use super::*;
use std::collections::{HashMap, HashSet};
#[path = "storyboard_assets.rs"]
pub(super) mod asset_cards;
#[path = "storyboard_creative.rs"]
pub(super) mod creative;
#[path = "storyboard_original.rs"]
pub(super) mod original;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn strings(value: &Value, key: &str) -> Result<Vec<String>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("缺少数组 {key}"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{key} 包含非字符串引用"))
        })
        .collect()
}
fn emotion_schema() -> Value {
    obj(
        json!({"subject":string(),"contextTone":string(),"audienceEffect":string(),"change":string(),"landing":string(),"retain":array(string())}),
    )
}
fn boundary_schema() -> Value {
    obj(
        json!({"kind":choices(&["episode_start","continuous","capacity","new_event","time_space_jump"]),"reason":string()}),
    )
}

fn plan_schema(
    rows: &[SourceRow],
    context: &[SourceRow],
    scenes: &[SourceScene],
    fantasy_mode: &str,
) -> Value {
    let source_ids: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
    let all_ids: Vec<_> = rows.iter().chain(context).map(|r| r.id.clone()).collect();
    let scene_ids: Vec<_> = scenes.iter().map(|s| s.id.clone()).collect();
    let fantasy = obj(
        json!({"id":string(),"sourceRefs":refs(&source_ids),"subject":string(),"emotionPurpose":string(),"recommended":{"type":"boolean"},"proposal":string(),"screenSecondsRange":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2},"extraSecondsRange":{"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2}}),
    );
    obj(
        json!({"schemaVersion":choices(&["storyboard-plan/v2"]),"unitPackingPolicy":choices(&["event-first-v1"]),"emotionArc":string(),
            "events":array(obj(json!({"id":string(),"sceneId":{"type":"string","enum":scene_ids},"sourceRefs":refs(&source_ids),"boundaryBefore":boundary_schema(),"emotionContract":emotion_schema()}))),
            "workPackages":array(obj(json!({"id":string(),"eventId":string(),"sourceRefs":refs(&source_ids),"contextRefs":refs(&all_ids),"entryFactIds":array(string()),"dependsOn":array(string()),"scenarioKeys":array(choices(original::SCENARIO_KEYS)),"emotionContract":emotion_schema()}))),
            "provisionalUnits":array(obj(json!({"id":string(),"packageId":string(),"sourceRefs":refs(&source_ids),"secondsRange":{"type":"array","items":{"type":"number","exclusiveMinimum":0},"minItems":2,"maxItems":2}}))),
            "entryFacts":array(obj(json!({"id":string(),"subject":string(),"statement":string(),"sourceRefs":refs(&all_ids),"evidenceMode":choices(&["source_rows"])}))),
            "fantasyCandidates":if fantasy_mode=="off"{json!({"type":"array","items":fantasy,"maxItems":0})}else{array(fantasy)}
        }),
    )
}

pub(super) fn draft_schema(
    rows: &[SourceRow],
    context: &[SourceRow],
    assets: &[Value],
    event_ids: &[String],
    episode_id: &str,
) -> Value {
    let source_ids: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
    let all_ids: Vec<_> = rows.iter().chain(context).map(|r| r.id.clone()).collect();
    let dialogue_ids: Vec<_> = rows
        .iter()
        .filter_map(|r| r.dialogue.as_ref().map(|q| q.id.clone()))
        .collect();
    let asset_ids: Vec<_> = assets
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let variant_ids: Vec<_> = assets
        .iter()
        .filter_map(|a| {
            a.get("variantId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let dialogue = obj(
        json!({"lineId":if dialogue_ids.is_empty(){string()}else{json!({"type":"string","enum":dialogue_ids})},"quoteStart":{"type":"integer","minimum":0},"quoteEnd":{"type":"integer","minimum":1},"presentation":choices(&["画内","画外","VO","OS","系统音","画外转画内","画内转画外"]),"delivery":string(),"voiceStart":{"type":"number"},"voiceEnd":{"type":"number"}}),
    );
    let shot = obj(
        json!({"id":string(),"duration":{"type":"number"},"camera":string(),"picture":string(),"lighting":string(),"sound":string(),"dialogueParts":if dialogue_ids.is_empty(){json!({"type":"array","items":dialogue,"maxItems":0})}else{array(dialogue)},"transition":string(),"sourceRefs":refs(&source_ids),"contextRefs":refs(&all_ids)}),
    );
    let asset_ref = obj(
        json!({"assetId":if asset_ids.is_empty(){string()}else{json!({"type":"string","enum":asset_ids})},"variantId":if variant_ids.is_empty(){string()}else{json!({"type":"string","enum":variant_ids})},"use":choices(&["character","scene","prop","style","audio","video"]),"order":{"type":"integer"},"sourceRefs":refs(&source_ids)}),
    );
    let unit = obj(
        json!({"id":string(),"episodeId":{"type":"string","enum":[episode_id]},"eventId":{"type":"string","enum":event_ids},"sceneId":string(),"boundaryBefore":boundary_schema(),"title":string(),"unitDirection":string(),"intent":obj(json!({"subject":string(),"change":string(),"landing":string(),"retain":array(string())})),"scene":string(),"participants":array(string()),"props":array(string()),"lighting":string(),"initialState":string(),"exitState":string(),"sourceRefs":refs(&source_ids),"assetRefs":if asset_ids.is_empty(){json!({"type":"array","items":asset_ref,"maxItems":0})}else{array(asset_ref)},"shots":array(shot)}),
    );
    obj(
        json!({"schemaVersion":choices(&["storyboard-draft/v2"]),"units":array(unit),"fantasyRealization":array(obj(json!({"candidateId":string(),"unitIds":array(string()),"shotIds":array(string()),"screenSeconds":{"type":"number"},"entry":string(),"exit":string(),"preservedFunction":string()}))),"revisionNotes":array(obj(json!({"coordinate":string(),"sourceRefs":refs(&source_ids),"change":string(),"preservedFunction":string()}))),"unresolved":array(obj(json!({"coordinate":string(),"problem":string()})))}),
    )
}

// The original coordinator finishes the related authored range, using a delta
// to avoid asking the editor to retype unchanged units. The application-wide
// UTF-16 transport budget is a desktop safeguard, not the old 5-unit/150-second
// author's soft target and not a new editorial duration limit. Unlike the
// foundation extraction's 24k output reservation, this limit measures the
// complete actual outbound envelope, including rules and strict output schema.
const FINISH_TRANSPORT_LIMIT: usize = STORYBOARD_TRANSPORT_LIMIT;

fn finish_transport_characters(request: &TextRequest) -> Result<usize, String> {
    let wire = compile_storyboard_dispatch_transport(request)?;
    let (input, schema) = wire
        .as_ref()
        .map(|wire| (&wire.input, &wire.schema))
        .unwrap_or((&request.input, &request.schema));
    Ok(crate::text_executor::transport_payload_characters(
        request, input, schema,
    ))
}

// New wire notation may repair an oversized indivisible request, but it cannot
// silently regroup the accepted editor requests of an old frozen workflow.
// Only new dependency-exit workflows opt into the latest greedy grouping.
fn finish_grouping_characters(
    request: &TextRequest,
    compact_dependency_exits: bool,
) -> Result<usize, String> {
    if compact_dependency_exits {
        // Keep the existing editor partitions. Only the indivisible request's
        // preflight uses the exact outgoing compiler; a smaller wire cannot
        // regroup already frozen adjacent author scopes.
        let wire = compile_storyboard_transport(request)?;
        let (input, schema) = wire
            .as_ref()
            .map(|wire| (&wire.input, &wire.schema))
            .unwrap_or((&request.input, &request.schema));
        return Ok(crate::text_executor::transport_payload_characters(
            request, input, schema,
        ));
    }
    let wire = compile_legacy_finish_grouping_transport(request)?;
    let (input, schema) = wire
        .as_ref()
        .map(|wire| (&wire.input, &wire.schema))
        .unwrap_or((&request.input, &request.schema));
    Ok(crate::text_executor::transport_payload_characters(
        request, input, schema,
    ))
}

#[derive(Clone)]
struct AcceptedAuthorScope {
    index: usize,
    batch: original::ExecutionBatch,
    input: Value,
    draft: Value,
}

pub(crate) fn finish_base_hash(base: &Value) -> String {
    digest(&json!({"schemaVersion":base["schemaVersion"],"units":base["units"],"fantasyRealization":base["fantasyRealization"]}).to_string())
}

pub(super) fn finish_delta_schema(full: &Value, base: &Value) -> Value {
    let ids: Vec<_> = base["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| unit["id"].clone())
        .collect();
    obj(json!({
        "schemaVersion":choices(&["storyboard-delta/v2"]),
        "baseDraftHash":{"type":"string","enum":[finish_base_hash(base)]},
        "retainedUnitIds":{"type":"array","items":{"type":"string","enum":ids}},
        "replacements":{"type":"array","items":obj(json!({
            "replaces":{"type":"array","items":{"type":"string","enum":ids},"minItems":1},
            "units":{"type":"array","items":full["properties"]["units"]["items"],"minItems":1},
            "fantasyRealization":full["properties"]["fantasyRealization"]
        }))},
        "revisionNotes":full["properties"]["revisionNotes"],
        "unresolved":full["properties"]["unresolved"]
    }))
}

/// Equivalent to the old applyDelta partition contract. Retained units and
/// their fantasies are copied by Rust; an edit must supply a complete group.
pub(super) fn apply_finish_delta(base: &Value, delta: &Value) -> Result<Value, String> {
    if text(delta, "schemaVersion") != "storyboard-delta/v2"
        || text(delta, "baseDraftHash") != finish_base_hash(base)
    {
        return Err("整编差量基稿与实际已校验作者稿不一致".into());
    }
    let old = base["units"].as_array().ok_or("整编基稿缺少单元")?;
    let ids: Vec<_> = old.iter().map(|unit| text(unit, "id")).collect();
    let old_set: HashSet<_> = ids.iter().copied().collect();
    if ids.is_empty() || ids.iter().any(|id| id.is_empty()) || old_set.len() != ids.len() {
        return Err("整编基稿单元身份为空或重复".into());
    }
    let retained = strings(delta, "retainedUnitIds")?;
    let groups = delta["replacements"]
        .as_array()
        .ok_or("整编差量缺少完整替换组")?;
    let mut claimed = HashSet::new();
    for id in &retained {
        if !old_set.contains(id.as_str()) || !claimed.insert(id.clone()) {
            return Err("旧单元须精确分区覆盖，不得重复或越界".into());
        }
    }
    let mut replaced = HashMap::new();
    let mut new_ids = HashSet::new();
    for (index, group) in groups.iter().enumerate() {
        let replaces = strings(group, "replaces")?;
        let units = group["units"]
            .as_array()
            .filter(|units| !units.is_empty())
            .ok_or("整编替换组必须交付完整新单元")?;
        let fantasies = group["fantasyRealization"]
            .as_array()
            .ok_or("整编替换组缺少幻想记录")?;
        if replaces.is_empty() {
            return Err("整编替换组旧单元不能为空".into());
        }
        let positions: Vec<_> = replaces
            .iter()
            .map(|id| ids.iter().position(|old| *old == id))
            .collect();
        if positions.iter().any(Option::is_none)
            || positions
                .windows(2)
                .any(|pair| pair[1].unwrap() != pair[0].unwrap() + 1)
        {
            return Err("替换组旧单元须按现稿顺序连续排列；跨越单元须纳入同组".into());
        }
        for id in &replaces {
            if !claimed.insert(id.clone()) {
                return Err("旧单元须精确分区覆盖，不得重复或越界".into());
            }
            replaced.insert(id.clone(), index);
        }
        let group_ids: HashSet<_> = units.iter().map(|unit| text(unit, "id")).collect();
        for unit in units {
            let id = text(unit, "id");
            if id.is_empty()
                || !new_ids.insert(id.to_string())
                || (old_set.contains(id) && !replaces.iter().any(|old| old == id))
            {
                return Err("整编新单元身份为空、重复或复用其他组旧身份".into());
            }
        }
        for fantasy in fantasies {
            if strings(fantasy, "unitIds")?
                .iter()
                .any(|id| !group_ids.contains(id.as_str()))
            {
                return Err("替换幻想只能引用本组完整新单元".into());
            }
        }
    }
    if claimed.len() != ids.len() || retained.iter().any(|id| new_ids.contains(id)) {
        return Err("旧单元不得遗漏，新单元不得撞保留单元".into());
    }
    let old_fantasies = base["fantasyRealization"]
        .as_array()
        .ok_or("整编基稿缺少幻想记录")?;
    for fantasy in old_fantasies {
        let refs = strings(fantasy, "unitIds")?;
        let touched: HashSet<_> = refs
            .iter()
            .filter_map(|id| replaced.get(id).copied())
            .collect();
        if touched.len() > 1
            || touched
                .iter()
                .next()
                .map(|group| refs.iter().any(|id| replaced.get(id) != Some(group)))
                .unwrap_or(false)
        {
            return Err("关联幻想必须随同一完整替换组处理，不得部分保留或拆散".into());
        }
    }
    let mut units = Vec::new();
    let mut emitted = HashSet::new();
    for unit in old {
        match replaced.get(text(unit, "id")) {
            None => units.push(unit.clone()),
            Some(index) => {
                if emitted.insert(*index) {
                    units.extend(groups[*index]["units"].as_array().unwrap().iter().cloned());
                }
            }
        }
    }
    let mut fantasy: Vec<_> = old_fantasies
        .iter()
        .filter(|record| {
            strings(record, "unitIds")
                .map(|ids| ids.iter().all(|id| retained.contains(id)))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    fantasy.extend(
        groups
            .iter()
            .flat_map(|group| group["fantasyRealization"].as_array().unwrap())
            .cloned(),
    );
    if !delta["revisionNotes"].is_array() || !delta["unresolved"].is_array() {
        return Err("整编差量缺少修订与未解决项数组".into());
    }
    Ok(
        json!({"schemaVersion":"storyboard-draft/v2","units":units,"fantasyRealization":fantasy,"revisionNotes":delta["revisionNotes"],"unresolved":delta["unresolved"]}),
    )
}

fn finish_request(
    scopes: &[AcceptedAuthorScope],
    plan: &Value,
    rows: &[SourceRow],
    context: &[SourceRow],
    assets: &[Value],
    selected: &[Value],
    episode_id: &str,
    model: &str,
    reasoning: &str,
    voice_reference: Option<&Value>,
    prior_finish: Option<&Value>,
    next: Option<&Value>,
) -> Result<(TextRequest, Value, Vec<String>), String> {
    let wanted: HashSet<_> = scopes
        .iter()
        .flat_map(|scope| scope.batch.source_refs.iter().cloned())
        .collect();
    let source_rows: Vec<_> = rows
        .iter()
        .filter(|row| wanted.contains(&row.id))
        .cloned()
        .collect();
    let refs: Vec<_> = source_rows.iter().map(|row| row.id.clone()).collect();
    let owner_ids: HashSet<_> = scopes
        .iter()
        .flat_map(|scope| scope.batch.package_ids.iter().cloned())
        .collect();
    let owners: Vec<_> = plan["workPackages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|owner| owner_ids.contains(text(owner, "id")))
        .cloned()
        .collect();
    let context_ids: HashSet<_> = scopes
        .iter()
        .flat_map(|scope| scope.input["contextRows"].as_array().into_iter().flatten())
        .filter_map(|row| row["id"].as_str())
        .collect();
    let context_rows: Vec<_> = rows
        .iter()
        .chain(context)
        .filter(|row| !wanted.contains(&row.id) && context_ids.contains(row.id.as_str()))
        .cloned()
        .collect();
    let candidates: Vec<_> = selected
        .iter()
        .filter(|candidate| {
            strings(candidate, "sourceRefs")
                .map(|ids| ids.iter().any(|id| wanted.contains(id)))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    let entry_ids: HashSet<_> = owners
        .iter()
        .flat_map(|owner| strings(owner, "entryFactIds").unwrap_or_default())
        .collect();
    let entry_facts: Vec<_> = plan["entryFacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| entry_ids.contains(text(fact, "id")))
        .cloned()
        .collect();
    let scenario_keys: Vec<_> = owners
        .iter()
        .flat_map(|owner| strings(owner, "scenarioKeys").unwrap_or_default())
        .collect();
    let methods = methods_for(
        &source_rows,
        &candidates,
        &scenario_keys,
        true,
        &entry_facts,
    )?;
    let event_ids: Vec<_> = plan["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| text(event, "id").to_string())
        .collect();
    let base = json!({"schemaVersion":"storyboard-draft/v2",
        "units":scopes.iter().flat_map(|scope|scope.draft["units"].as_array().unwrap()).cloned().collect::<Vec<_>>(),
        "fantasyRealization":scopes.iter().flat_map(|scope|scope.draft["fantasyRealization"].as_array().unwrap()).cloned().collect::<Vec<_>>(),
        "revisionNotes":[],"unresolved":[]});
    let identity=digest(&json!({"episodeId":episode_id,"authorBatchIds":scopes.iter().map(|scope|&scope.batch.id).collect::<Vec<_>>(),"sourceRefs":refs}).to_string());
    let mut input = scopes[0].input.clone();
    input.as_object_mut().unwrap().remove("frozenFinisherAssetContext");
    // A following editor is a new node. Its proof must bind this request, not
    // the already-dispatched editor whose unmodified input is held in `scope`.
    if let Some(proof) = input.get("finisherAssetInputProjection") {
        if proof["version"] != "finisher-asset-input-projection/v1"
            || input["storyboardAssetInputProfile"] != asset_cards::SCOPED_PROFILE
        {
            return Err("上一整编资产投影版本不成立；原请求保留，未派发新节点".into());
        }
        input.as_object_mut().unwrap().remove("finisherAssetInputProjection");
        input.as_object_mut().unwrap().remove("storyboardAssetInputProfile");
    }
    let mut author_revisions = Vec::new();
    for scope in scopes {
        if let Some(revision) = scope.input.get("authorChainRevision") {
            if !author_revisions.contains(revision) {
                author_revisions.push(revision.clone());
            }
        }
    }
    if author_revisions.len() == 1 {
        input["authorChainRevision"] = author_revisions[0].clone();
    } else if !author_revisions.is_empty() {
        input["authorChainRevision"] =
            json!({"version":"accepted-author-chain-change/v1","authorRevisions":author_revisions});
    }
    input.as_object_mut().unwrap().remove("workPackage");
    input.as_object_mut().unwrap().remove("dependencyOutputs");
    input["workPackages"] = json!(owners);
    input["sourceRows"] = json!(source_rows);
    input["contextRows"] = json!(context_rows);
    if super::stage3_pack::uses_profile(&input)? {
        let (cards, selection) = asset_cards::project_for_scope(
            &input,
            assets,
            episode_id,
            &source_rows,
            &context_rows,
        )?;
        input["assets"] = json!(cards);
        input["assetSelection"] = selection;
    }
    let mut decisions = Vec::new();
    for scope in scopes {
        for record in scope.input["scopedAuthorDecisions"]["records"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let key = author_decision_key(record);
            if let Some(previous) = decisions.iter().find(|r| author_decision_key(r) == key) {
                if previous != record {
                    return Err("关联整编的已采用解释回执冲突".into());
                }
            } else {
                decisions.push(record.clone());
            }
        }
    }
    if !decisions.is_empty() {
        let sha = text(&scopes[0].input["scopedAuthorDecisions"], "sourceSha256");
        // The first author may have no local interpretation; use the first package.
        let sha = if sha.is_empty() {
            scopes
                .iter()
                .find_map(|s| s.input["scopedAuthorDecisions"]["sourceSha256"].as_str())
                .unwrap_or("")
        } else {
            sha
        };
        attach_author_decisions(
            &mut input,
            author_decision_package(sha, &source_rows, decisions).as_ref(),
        );
    }
    input["entryFacts"] = json!(entry_facts);
    input["selectedFantasy"] = json!(candidates);
    let dependency_exits = input["plan"]["dependencyExits"].clone();
    input["plan"] = if creative::uses_profile(&input)? {
        creative::finish_plan_view(plan, &input, &dependency_exits)
    } else {
        plan.clone()
    };
    input["previousDraft"] = base.clone();
    input["baseDraftHash"] = json!(finish_base_hash(&base));
    input["batch"] = json!(format!("finish:{identity}"));
    input["finishMode"] = json!("delta");
    input["executionBatch"] = json!({"version":"related-range-finish/v1","id":format!("finish:{identity}"),"authorBatchIds":scopes.iter().map(|scope|&scope.batch.id).collect::<Vec<_>>(),"packageIds":scopes.iter().flat_map(|scope|&scope.batch.package_ids).collect::<Vec<_>>(),"sourceRefs":refs});
    if let Some(unit) = prior_finish {
        input["acceptedContinuity"] = json!({"version":"accepted-tail-v1","unit":unit,"sourceRefs":unit["sourceRefs"],"evidenceRole":"ACCEPTED_CREATIVE_STATE_ONLY"});
    }
    input["nextBoundary"] = next.cloned().unwrap_or(Value::Null);
    input["capacityBrief"] = original::capacity_brief(
        &input["capabilities"],
        Some(&base),
        voice_reference,
        &source_rows,
    );
    input["instruction"]=json!("所有本范围主创已形成已校验现稿，执行原整编岗的完整关联范围差量闭环。schema是运输形状：baseDraftHash逐字回填；未改单元仅列retainedUnitIds，由Rust原样拷贝。每个实际替换组列现稿顺序连续的replaces，交付完整新units与该组fantasyRealization；旧单元恰好分区覆盖一次，关联幻想必须整组处理，不得漏掉或拆散。无问题可全部保留且replacements=[]。只压缩未改正文运输，不削减情绪、表演、光影、声道、原话、必要动作或已选幻想。对白只选原lineId/quoteStart/quoteEnd，Rust物化；实际拆合才改ID并按新切点重算声窗。不自动取整、删图、截词、截提示词，也不为批次增加成品边界。");
    let schema = finish_delta_schema(
        &draft_schema(&source_rows, &context_rows, assets, &event_ids, episode_id),
        &base,
    );
    let request = creative::prepare_request(TextRequest {
        stage: "storyboardFinish".into(),
        skill: "episode-finishing-editor".into(),
        model: model.into(),
        reasoning: reasoning.into(),
        input,
        schema,
        rule_snapshot: snapshot("episode-finishing-editor", &methods)?,
    })?;
    Ok((request, base, refs))
}

async fn prepare_finish_assets(
    prepared: (TextRequest, Value, Vec<String>),
    frozen: &Value,
    assets: &[Value],
    executor: &dyn TextExecutor,
) -> Result<(TextRequest, Value, Vec<String>), String> {
    let (legacy, base, refs) = prepared;
    let legacy = creative::prepare_request(legacy)?;
    let mut projected = asset_cards::project_finish_request(&legacy, frozen, assets)?;
    if projected.input == legacy.input {
        return Ok((legacy, base, refs));
    }
    projected = creative::prepare_request(projected)?;
    projected.input["finisherAssetInputProjection"]["legacyDomainRequestSha256"] =
        json!(digest(&serde_json::to_string(&legacy).map_err(|e| e.to_string())?));
    let actual = executor
        .prepare_finish_asset_projection(legacy, projected)
        .await?;
    Ok((actual, base, refs))
}

// The live coordinator and the read-only capacity probe use this same builder.
// It binds the original rejected transport without creating a task or adopting it.
fn bind_scope_repair_request(
    prepared: (TextRequest, Value, Vec<String>),
    source_sha: &str,
    original_raw: &Value,
    origin: &TextRequest,
    scope: &AcceptedAuthorScope,
    issues: &[Value],
    attempt: usize,
) -> Result<(TextRequest, Value, Vec<String>), String> {
    let (mut request, base, refs) = prepared;
    let finish_origin = origin.stage == "storyboardFinish";
    if (!finish_origin && origin.stage != "storyboardCreate")
        || (finish_origin && origin.input["executionBatch"]["version"] != "related-range-finish/v1")
    {
        return Err("适用范围修订原岗或原执行范围不成立；保留原拒绝，未调用模型".into());
    }
    let descriptor_field = if finish_origin { "finishRepair" } else { "authorRepair" };
    let raw_field = if finish_origin { "originalFinishRawSha256" } else { "originalAuthorRawSha256" };
    let profile = if finish_origin { creative::FINISH_SCOPE_REPAIR_PROFILE } else { creative::AUTHOR_SCOPE_REPAIR_PROFILE };
    let execution_version = if finish_origin { creative::FINISH_SCOPE_REPAIR_EXECUTION_VERSION } else { creative::AUTHOR_SCOPE_REPAIR_EXECUTION_VERSION };
    let mut provenance = json!({"version":profile,"attempt":attempt,"maximumAttempts":2,
        "sourceSha256":source_sha,"originDomainRequestSha256":digest(&serde_json::to_string(origin).map_err(|e|e.to_string())?),
        "originExecutionBatch":origin.input["executionBatch"],"baseDraftHash":finish_base_hash(&base),
        "problemSignature":author_scope_problem_signature(issues)});
    provenance[raw_field] = json!(digest(&original_raw.to_string()));
    let identity = digest(&provenance.to_string());
    request.input[descriptor_field] = provenance;
    request.input["validationIssues"] = json!(issues);
    request.input["executionBatch"] = json!({"version":execution_version,
        "id":format!("{}:{identity}",if finish_origin{"finish-domain-repair"}else{"author-domain-repair"}),
        "sourceRefs":refs,"packageIds":scope.batch.package_ids,"originExecutionBatchId":scope.batch.id});
    Ok((creative::prepare_request(request)?, base, refs))
}

fn refs_valid(items: &[String], allowed: &HashSet<&str>, nonempty: bool) -> Result<(), String> {
    if nonempty && items.is_empty() {
        return Err("当前制作范围引用不能为空".into());
    }
    let mut seen = HashSet::new();
    if items
        .iter()
        .any(|id| !allowed.contains(id.as_str()) || !seen.insert(id))
    {
        return Err("引用未知、越界或重复".into());
    }
    Ok(())
}

fn normalize_coverage(source: &LockedSource, covered: &[String]) -> Result<Vec<String>, String> {
    // Port the original coordinator's evidence-set coverage. Citing a source
    // again does not replay it; actual speech and its order are checked below,
    // while Rust materializes unique locked body intervals independently.
    let cited: HashSet<_> = covered.iter().map(String::as_str).collect();
    Ok(source
        .rows
        .iter()
        .filter(|row| cited.contains(row.id.as_str()))
        .map(|row| row.id.clone())
        .collect())
}

fn validate_plan(
    plan: &Value,
    rows: &[SourceRow],
    context: &[SourceRow],
    scenes: &[SourceScene],
) -> Result<(), String> {
    if text(plan, "schemaVersion") != "storyboard-plan/v2" {
        return Err("情绪计划协议版本不匹配".into());
    }
    let allowed: HashSet<_> = rows.iter().map(|r| r.id.as_str()).collect();
    let all: HashSet<_> = rows.iter().chain(context).map(|r| r.id.as_str()).collect();
    let events = plan
        .get("events")
        .and_then(Value::as_array)
        .ok_or("情绪计划缺少 events")?;
    let packages = plan
        .get("workPackages")
        .and_then(Value::as_array)
        .ok_or("情绪计划缺少 workPackages")?;
    if events.is_empty() || packages.is_empty() {
        return Err("情绪计划遗漏当前制作范围".into());
    }
    let mut event_ids = HashSet::new();
    let mut event_coverage = Vec::new();
    for event in events {
        if text(event, "id").is_empty()
            || !event_ids.insert(text(event, "id"))
            || !scenes.iter().any(|s| s.id == text(event, "sceneId"))
        {
            return Err("事件身份或场景无效".into());
        }
        let rs = strings(event, "sourceRefs")?;
        refs_valid(&rs, &allowed, true)?;
        if rs.iter().any(|id| {
            rows.iter()
                .find(|r| &r.id == id)
                .and_then(|r| r.scene_id.as_deref())
                != Some(text(event, "sceneId"))
        }) {
            return Err("事件跨入另一个真实场景".into());
        }
        event_coverage.extend(rs);
        validate_emotion(&event["emotionContract"])?;
    }
    let expected: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
    if event_coverage != expected {
        return Err("事件必须按源顺序覆盖当前正文恰好一次".into());
    }
    let facts = plan
        .get("entryFacts")
        .and_then(Value::as_array)
        .ok_or("计划缺少 entryFacts")?;
    let mut fact_ids = HashSet::new();
    for fact in facts {
        if text(fact, "id").is_empty() || !fact_ids.insert(text(fact, "id")) {
            return Err("入口事实 id 重复".into());
        }
        refs_valid(&strings(fact, "sourceRefs")?, &all, true)?;
        if text(fact, "subject").trim().is_empty()
            || text(fact, "statement").trim().is_empty()
            || text(fact, "evidenceMode") != "source_rows"
        {
            return Err("入口事实须有主体、状态与程序源证据模式".into());
        }
    }
    let mut package_ids = HashSet::new();
    let mut package_coverage = Vec::new();
    for package in packages {
        let id = text(package, "id");
        let eid = text(package, "eventId");
        if id.is_empty() || !event_ids.contains(eid) || !package_ids.insert(id) {
            return Err("工作包身份或事件归属无效".into());
        }
        let source_refs = strings(package, "sourceRefs")?;
        refs_valid(&source_refs, &allowed, true)?;
        let event = events.iter().find(|e| text(e, "id") == eid).unwrap();
        let event_refs = strings(event, "sourceRefs")?;
        if source_refs.iter().any(|r| !event_refs.contains(r)) {
            return Err("工作包越出所属事件".into());
        }
        package_coverage.extend(source_refs);
        validate_emotion(&package["emotionContract"])?;
        refs_valid(&strings(package, "contextRefs")?, &all, false)?;
        if strings(package, "entryFactIds")?
            .iter()
            .any(|r| !fact_ids.contains(r.as_str()))
        {
            return Err("工作包引用未知入口事实".into());
        }
        if strings(package, "dependsOn")?
            .iter()
            .any(|r| r == id || !package_ids.contains(r.as_str()))
        {
            return Err("承接依赖必须引用已在源顺序前面的工作包".into());
        }
    }
    if package_coverage != expected {
        return Err("工作包必须依源顺序覆盖正文恰好一次".into());
    }
    let mut fantasy_ids = HashSet::new();
    for candidate in plan
        .get("fantasyCandidates")
        .and_then(Value::as_array)
        .ok_or("缺少幻想选择目录")?
    {
        if text(candidate, "id").is_empty() || !fantasy_ids.insert(text(candidate, "id")) {
            return Err("幻想候选身份空或重复".into());
        }
        refs_valid(&strings(candidate, "sourceRefs")?, &allowed, true)?;
        for field in ["screenSecondsRange", "extraSecondsRange"] {
            let values = candidate[field]
                .as_array()
                .filter(|v| v.len() == 2)
                .ok_or("幻想机会须给实际占时/净增时长区间")?;
            let low = values[0].as_f64().ok_or("幻想区间无效")?;
            let high = values[1].as_f64().ok_or("幻想区间无效")?;
            if !low.is_finite() || !high.is_finite() || low < 0.0 || high < low {
                return Err("幻想占时/净增区间无效".into());
            }
        }
    }
    Ok(())
}

fn validate_emotion(contract: &Value) -> Result<(), String> {
    if [
        "subject",
        "contextTone",
        "audienceEffect",
        "change",
        "landing",
    ]
    .iter()
    .any(|key| text(contract, key).trim().is_empty())
        || strings(contract, "retain")?.is_empty()
    {
        return Err("完整情绪契约缺少主体、当拍基调、观众回报、变化、落点或保留项".into());
    }
    Ok(())
}

fn capabilities(input: &Value) -> Result<(f64, f64, usize), String> {
    let caps = input.get("capabilities").ok_or("缺少冻结的分镜制作规格")?;
    let max = caps
        .get("maxDuration")
        .and_then(Value::as_f64)
        .ok_or("视频模型最大时长未知，不能生成假定容量的分镜")?;
    let min = caps
        .get("minDuration")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let images = caps
        .get("imageLimit")
        .or_else(|| caps.get("maxImages"))
        .and_then(Value::as_u64)
        .ok_or("视频模型参考图片容量未知")? as usize;
    if !max.is_finite() || !min.is_finite() || max <= 0.0 || min < 0.0 || min > max {
        return Err("视频模型时长能力无效".into());
    }
    Ok((min, max, images))
}

fn spoken_ranges(quote: &SourceDialogue) -> Vec<SourceRange> {
    // Canonical source parsing owns these segments. Legacy source without either
    // segment collection can still be read, but never trusted as model output.
    let ranges: Vec<SourceRange> = quote
        .spoken_segments
        .iter()
        .map(|segment| segment.range.clone())
        .collect();
    if ranges.is_empty() && quote.performance_segments.is_empty() && !quote.quote.is_empty() {
        vec![quote.range.clone()]
    } else {
        ranges
    }
}

fn expected_spoken(source: &LockedSource, quote: &SourceDialogue) -> Result<String, String> {
    spoken_ranges(quote)
        .iter()
        .map(|range| slice_utf16(&source.text, range))
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join(""))
}

/// Model output selects immutable absolute UTF-16 source intervals. Text,
/// speaker and channel are always filled by Rust before validation or caching.
fn spoken_position(quote: &SourceDialogue, offset: usize) -> Option<usize> {
    let mut consumed = 0;
    for range in spoken_ranges(quote) {
        let length = range.end - range.start;
        if offset >= consumed && offset < consumed + length {
            return Some(range.start + offset - consumed);
        }
        consumed += length;
    }
    None
}

fn materialize_dialogue(
    source: &LockedSource,
    draft: &mut Value,
    allow_legacy: bool,
) -> Result<(), String> {
    let quotes: HashMap<_, _> = source
        .rows
        .iter()
        .filter_map(|r| r.dialogue.as_ref().map(|q| (q.id.as_str(), q)))
        .collect();
    let mut cursors: HashMap<String, usize> = HashMap::new();
    for unit in draft
        .get_mut("units")
        .and_then(Value::as_array_mut)
        .ok_or("缺少单元数组")?
    {
        for shot in unit
            .get_mut("shots")
            .and_then(Value::as_array_mut)
            .ok_or("缺少镜表数组")?
        {
            for part in shot
                .get_mut("dialogueParts")
                .and_then(Value::as_array_mut)
                .ok_or("缺少对白数组")?
            {
                if !part.is_object() {
                    return Err("对白片段必须为结构对象".into());
                }
                let id = text(part, "lineId").to_string();
                let quote = quotes.get(id.as_str()).ok_or("对白引用未知源句")?;
                let ranges = spoken_ranges(quote);
                let selected =
                    match (part["quoteStart"].as_u64(), part["quoteEnd"].as_u64()) {
                        (Some(start), Some(end)) => SourceRange {
                            start: start as usize,
                            end: end as usize,
                        },
                        (None, None) if allow_legacy => {
                            let old = *cursors.get(&id).unwrap_or(&0);
                            let body = text(part, "text");
                            let count = body.encode_utf16().count();
                            let mut offset = 0;
                            let range = ranges
                                .iter()
                                .find_map(|range| {
                                    let length = range.end - range.start;
                                    let result = (old >= offset
                                        && old < offset + length
                                        && old + count <= offset + length)
                                        .then(|| SourceRange {
                                            start: range.start + old - offset,
                                            end: range.start + old - offset + count,
                                        });
                                    offset += length;
                                    result
                                })
                                .ok_or("旧对白片段跨演法区间或不是连续发声原话，请明确修订")?;
                            range
                        }
                        _ => return Err(
                            "对白必须选择 quoteStart/quoteEnd 原文UTF16半开区间；模型不能抄写台词"
                                .into(),
                        ),
                    };
                if selected.start >= selected.end
                    || !ranges
                        .iter()
                        .any(|range| selected.start >= range.start && selected.end <= range.end)
                {
                    return Err("发声区间越界或包含不发声演法；跨演法须分别选择连续原话区间".into());
                }
                let body = slice_utf16(&source.text, &selected)?;
                let old = *cursors.get(&id).unwrap_or(&0);
                if spoken_position(quote, old) != Some(selected.start) {
                    return Err("原话区间重复、跳过或乱序，不能靠相同文字掩盖来源错误".into());
                }
                if part.get("text").is_some() && text(part, "text") != body {
                    return Err("传入对白与原话区间不一致，禁止改词或把演法念出".into());
                }
                if part.get("speaker").is_some() && text(part, "speaker") != quote.speaker
                    || part.get("channel").is_some() && text(part, "channel") != quote.channel
                {
                    return Err("传入说者或声道不符合原话来源".into());
                }
                cursors.insert(id, old + body.encode_utf16().count());
                part["quoteStart"] = json!(selected.start);
                part["quoteEnd"] = json!(selected.end);
                part["text"] = json!(body);
                part["speaker"] = json!(quote.speaker);
                part["channel"] = json!(quote.channel);
            }
        }
    }
    Ok(())
}

fn creative_settings(settings: &Value) -> Value {
    let mut result = serde_json::Map::new();
    for key in [
        "visualStyle",
        "fantasyMode",
        "noSubtitles",
        "noBgm",
        "creativeDirection",
    ] {
        if let Some(value) = settings.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    Value::Object(result)
}

fn production_direction(input: &Value) -> Value {
    json!({"direction":input.pointer("/foundation/direction").and_then(Value::as_str).unwrap_or(""),
        "overview":input.pointer("/foundation/overview").and_then(Value::as_str).unwrap_or(""),
        "evidenceRule":"全剧方向统领表达，不改写源事实，不压平本拍情绪；本集原文决定事件及揭示时点。"})
}

const AUTHOR_DECISION_ROLE: &str = "APPLIED_CREATIVE_INTERPRETATION_ONLY";

fn author_decision_key(record: &Value) -> String {
    json!([record["id"], record["revision"], record["applicationScope"]]).to_string()
}

/// Adopted interpretation is neither a SOURCE fact nor permission to mutate it.
/// Subject keys and scopes are explicit identities; names never grant a match.
fn scoped_author_decisions(
    source: &LockedSource,
    input: &Value,
    rows: &[SourceRow],
) -> Result<Option<Value>, String> {
    let episode_ids: HashSet<_> = rows
        .iter()
        .filter_map(|r| r.episode_id.as_deref())
        .collect();
    let scene_ids: HashSet<_> = rows.iter().filter_map(|r| r.scene_id.as_deref()).collect();
    let identities: HashSet<_> = input["foundation"]["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| text(a, "id"))
        .chain(input["assets"].as_array().into_iter().flatten().map(|a| {
            ["identityId", "baseAssetId", "id"]
                .iter()
                .map(|key| text(a, key))
                .find(|id| !id.is_empty())
                .unwrap_or("")
        }))
        .filter(|id| !id.is_empty())
        .collect();
    let candidates = input["foundation"]["authorDecisions"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            input["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|a| a["authorDecisions"].as_array().into_iter().flatten()),
        );
    let mut records: Vec<Value> = Vec::new();
    for record in candidates {
        if record["status"] != "APPLIED" || record["sourceSha256"] != source.sha256 {
            continue;
        }
        let scope = record["applicationScope"]
            .as_object()
            .ok_or("已采用解释缺明确适用范围")?;
        if scope
            .keys()
            .any(|key| !["kind", "episodes", "scenes"].contains(&key.as_str()))
        {
            return Err("已采用解释包含未知适用范围字段".into());
        }
        let scope_episodes = strings(&record["applicationScope"], "episodes")?;
        if scope_episodes.is_empty()
            || scope_episodes
                .iter()
                .any(|id| !source.episodes.iter().any(|e| &e.id == id))
            || scope_episodes.iter().collect::<HashSet<_>>().len() != scope_episodes.len()
        {
            return Err("已采用解释的集范围已失效".into());
        }
        let matches = match text(&record["applicationScope"], "kind") {
            "GLOBAL" | "EPISODES" => scope_episodes
                .iter()
                .any(|id| episode_ids.contains(id.as_str())),
            "SCENES" => {
                let pairs = record["applicationScope"]["scenes"]
                    .as_array()
                    .filter(|p| !p.is_empty())
                    .ok_or("已采用解释的场范围为空")?;
                let mut seen = HashSet::new();
                let mut declared_episodes = HashSet::new();
                let mut matches = false;
                for pair in pairs {
                    let pair = pair
                        .as_array()
                        .filter(|p| p.len() == 2)
                        .ok_or("已采用解释场范围须为集场ID对")?;
                    let ep = pair[0].as_str().ok_or("已采用解释缺集ID")?;
                    let scene = pair[1].as_str().ok_or("已采用解释缺场ID")?;
                    if !source
                        .scenes
                        .iter()
                        .any(|s| s.episode_id == ep && s.id == scene)
                        || !seen.insert((ep, scene))
                    {
                        return Err("已采用解释的场范围已失效或重复".into());
                    }
                    declared_episodes.insert(ep);
                    matches |= episode_ids.contains(ep) && scene_ids.contains(scene);
                }
                if declared_episodes != scope_episodes.iter().map(String::as_str).collect() {
                    return Err("已采用解释的集场范围不一致".into());
                }
                matches
            }
            _ => return Err("已采用解释的范围类型无效".into()),
        };
        if !matches {
            continue;
        }
        if record.get("episodes").is_some()
            && strings(record, "episodes")?.iter().collect::<HashSet<_>>()
                != scope_episodes.iter().collect::<HashSet<_>>()
        {
            return Err("已采用解释声明的集范围不一致".into());
        }
        let id = text(record, "id");
        if id.is_empty()
            || text(record, "answer").trim().is_empty()
            || !(record["revision"].as_str().is_some_and(|s| !s.is_empty())
                || record["revision"].is_u64())
        {
            return Err("已采用解释缺冻结身份、版本或确认答案".into());
        }
        let refs = strings(record, "sourceRefs")?;
        if refs.is_empty()
            || refs.iter().collect::<HashSet<_>>().len() != refs.len()
            || refs
                .iter()
                .any(|id| !source.rows.iter().any(|r| &r.id == id))
        {
            return Err("已采用解释的原文来源已失效".into());
        }
        let subjects = strings(record, "subjectAssetKeys")?;
        if subjects.iter().any(|id| !identities.contains(id.as_str()))
            || subjects.iter().collect::<HashSet<_>>().len() != subjects.len()
        {
            return Err("已采用解释的主体身份未登记，不能按姓名推断".into());
        }
        let receipts = record["receipts"]
            .as_array()
            .filter(|r| !r.is_empty())
            .ok_or("已采用解释缺实际采用回执")?;
        let mut covered = HashSet::new();
        for receipt in receipts {
            if receipt["id"] != record["id"]
                || receipt["revision"] != record["revision"]
                || text(receipt, "note").trim().is_empty()
            {
                return Err("已采用解释的采用回执与冻结契约不一致".into());
            }
            let coverage = strings(receipt, "coverage")?;
            if coverage.is_empty() || coverage.iter().any(|ep| !scope_episodes.contains(ep)) {
                return Err("已采用解释的回执范围越界或为空".into());
            }
            covered.extend(coverage);
        }
        if scope_episodes.iter().any(|ep| !covered.contains(ep)) {
            return Err("已采用解释的采用回执未完整覆盖声明范围".into());
        }
        // This projection deliberately excludes changeTargets and legacy rights.
        let projected = Value::Object(
            [
                "id",
                "decisionKey",
                "revision",
                "status",
                "sourceSha256",
                "sourceVersion",
                "question",
                "reason",
                "answer",
                "evidence",
                "sourceRefs",
                "episodes",
                "applicationScope",
                "subjectAssetKeys",
                "interpretationOnly",
                "receipts",
                "appliedNote",
            ]
            .iter()
            .filter_map(|key| record.get(*key).map(|v| ((*key).into(), v.clone())))
            .collect(),
        );
        let key = author_decision_key(&projected);
        if let Some(previous) = records.iter().find(|r| author_decision_key(r) == key) {
            if previous != &projected {
                return Err("同一已采用创作解释回执存在冲突".into());
            }
        } else {
            records.push(projected);
        }
    }
    Ok(author_decision_package(&source.sha256, rows, records))
}

fn author_decision_package(sha: &str, rows: &[SourceRow], records: Vec<Value>) -> Option<Value> {
    if records.is_empty() {
        return None;
    }
    Some(
        json!({"version":"scoped-author-decisions/v1","sourceSha256":sha,
        "productionSourceRefs":rows.iter().map(|r| &r.id).collect::<Vec<_>>(),"records":records,
        "evidenceRole":AUTHOR_DECISION_ROLE,"sourceMutationAllowed":false,
        "rule":"已采用答案只作声明applicationScope内的制作解释；subjectAssetKeys仅按登记身份匹配。原文、说者、台词和动作不改；sourceRefs是准确来源，不升级SOURCE事实，不授予删除、归并、换名、改场或扩大全局适用权限。answer、question、reason、appliedNote及receipts.note是逐字历史说明，可能含旧调用临时运输引用eN/sN/rN/aN/vN；它们不是本次主键，N不能当本次序号。本次身份、集场和来源仅以已校验subjectAssetKeys、applicationScope、sourceRefs及当前assets/sourceRows为准；没有该旧请求的明确冻结映射时只按结构字段和原行证据理解，不猜短码对应，不从历史短码创造角色、资产、场号、可见道具文字或生成端控制符。不要把历史引用抄成生成正文；原文里本来出现的同形词和台词逐字保留，不替换或清洗历史自由说明、原话或源文。"}),
    )
}

fn decision_context(
    source: &LockedSource,
    rows: &[SourceRow],
    context: &[SourceRow],
    package: Option<&Value>,
) -> Vec<SourceRow> {
    let owned: HashSet<_> = rows.iter().map(|r| r.id.as_str()).collect();
    let mut ids: HashSet<&str> = context.iter().map(|r| r.id.as_str()).collect();
    if let Some(package) = package {
        ids.extend(
            package["records"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|r| r["sourceRefs"].as_array().into_iter().flatten())
                .filter_map(Value::as_str),
        );
    }
    source
        .rows
        .iter()
        .filter(|r| !owned.contains(r.id.as_str()) && ids.contains(r.id.as_str()))
        .cloned()
        .collect()
}

fn attach_author_decisions(input: &mut Value, package: Option<&Value>) {
    if let Some(package) = package {
        input["scopedAuthorDecisions"] = package.clone();
    }
}

/// Domain checks are deliberately independent of JSON-schema parsing and model self-assessment.
pub fn validate_storyboard(
    source: &LockedSource,
    draft: &Value,
    assets: &[Value],
    caps: &Value,
) -> Result<(), String> {
    let source = upgrade_source_dialogues(source)?;
    validate_storyboard_scope(&source, draft, assets, caps, None)
}

pub(super) fn validate_storyboard_scope(
    source: &LockedSource,
    draft: &Value,
    assets: &[Value],
    caps: &Value,
    expected_refs: Option<&[String]>,
) -> Result<(), String> {
    validate_storyboard_scope_with_issues(source, draft, assets, caps, expected_refs, None)
}

// The only deferred issue is a known asset's declared applicability. Every
// source/identity/quotation/timing check still runs before a repair may start.
// Publication and ordinary acceptance always use the strict wrapper above.
fn validate_storyboard_scope_with_issues(
    source: &LockedSource,
    draft: &Value,
    assets: &[Value],
    caps: &Value,
    expected_refs: Option<&[String]>,
    scope_issues: Option<&mut Vec<Value>>,
) -> Result<(), String> {
    validate_storyboard_repair_scope(
        source,
        draft,
        assets,
        caps,
        expected_refs,
        scope_issues,
        false,
        false,
    )
}

// A known same-episode scene coordinate may be reported only for an already
// rejected Finisher candidate. This never changes the strict adoption gate.
// Missing/foreign/non-production rows and ambiguous source scenes stay hard.
fn validate_storyboard_repair_scope(
    source: &LockedSource,
    draft: &Value,
    assets: &[Value],
    caps: &Value,
    expected_refs: Option<&[String]>,
    mut scope_issues: Option<&mut Vec<Value>>,
    defer_scene_coordinate: bool,
    defer_zero_duration: bool,
) -> Result<(), String> {
    validate_source(source)?;
    let (min, max, image_limit) = capabilities(&json!({"capabilities":caps}))?;
    let units = draft
        .get("units")
        .and_then(Value::as_array)
        .ok_or("分镜回包缺少 units")?;
    if units.is_empty() {
        return Err("分镜没有输出视频单元".into());
    }
    let mut unit_ids = HashSet::new();
    let mut shot_ids = HashSet::new();
    let mut covered = Vec::new();
    let all: HashSet<_> = source.rows.iter().map(|r| r.id.as_str()).collect();
    let quotes: HashMap<_, _> = source
        .rows
        .iter()
        .filter_map(|r| r.dialogue.as_ref().map(|q| (q.id.as_str(), q)))
        .collect();
    let source_dialogue_order: HashMap<_, _> = source
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.dialogue.as_ref().map(|q| (q.id.as_str(), i)))
        .collect();
    let mut spoken: HashMap<&str, String> = HashMap::new();
    let mut prior_dialogue_order = 0;
    for unit in units {
        let uid = text(unit, "id");
        if uid.is_empty() || !unit_ids.insert(uid) {
            return Err("分镜单元 id 为空或重复".into());
        }
        let eid = text(unit, "episodeId");
        let sid = text(unit, "sceneId");
        if !source
            .scenes
            .iter()
            .any(|s| s.id == sid && s.episode_id == eid)
        {
            return Err("单元场景/制作集身份不成立".into());
        }
        for field in [
            "title",
            "unitDirection",
            "scene",
            "lighting",
            "initialState",
            "exitState",
        ] {
            if text(unit, field).trim().is_empty() {
                return Err(format!("单元 {uid} 缺少独立可读的 {field}"));
            }
        }
        if ["同上", "承接上一镜", "承接上一个单元"]
            .iter()
            .any(|s| text(unit, "initialState").trim() == *s)
        {
            return Err("单元初态不能依赖生成端回查上一个单元".into());
        }
        let source_refs = strings(unit, "sourceRefs")?;
        refs_valid(&source_refs, &all, true)?;
        if source_refs.iter().any(|id| {
            source
                .rows
                .iter()
                .find(|r| &r.id == id)
                .map(|r| r.kind != "body" || r.episode_id.as_deref() != Some(eid))
                .unwrap_or(true)
        }) {
            return Err("单元来源跨集/跨场或包含非制作资料".into());
        }
        let source_scenes: HashSet<_> = source_refs
            .iter()
            .filter_map(|id| {
                source
                    .rows
                    .iter()
                    .find(|row| &row.id == id)
                    .and_then(|row| row.scene_id.as_deref())
            })
            .collect();
        if source_scenes.len() != 1 || !source_scenes.contains(sid) {
            if !defer_scene_coordinate || source_scenes.len() != 1 || scope_issues.is_none() {
                return Err("单元来源跨集/跨场或包含非制作资料".into());
            }
            let expected_scene = source_scenes.iter().next().unwrap();
            if !source
                .scenes
                .iter()
                .any(|scene| scene.id == *expected_scene && scene.episode_id == eid)
            {
                return Err("单元来源跨集/跨场或包含非制作资料".into());
            }
            scope_issues.as_deref_mut().unwrap().push(json!({"code":"UNIT_SCENE_COORDINATE","unitId":uid,"coordinate":format!("{uid}.sceneId"),"episodeId":eid,"sceneId":sid,"expectedSceneId":expected_scene,"sourceRefs":source_refs,"problem":"已知同集场坐标与本单元唯一制作来源不一致；原场与事件身份不变，须由原整编修订后重新完整校验"}));
        }
        covered.extend(source_refs.clone());
        let references = unit
            .get("assetRefs")
            .and_then(Value::as_array)
            .ok_or("单元缺少结构化资产引用")?;
        let mut orders = HashSet::new();
        let mut asset_identities = HashSet::new();
        let mut image_identities = HashSet::new();
        for reference in references {
            let aid = text(reference, "assetId");
            let vid = text(reference, "variantId");
            let asset = assets
                .iter()
                .find(|a| text(a, "id") == aid && text(a, "variantId") == vid)
                .ok_or("分镜引用未知资产/形象，不能按同名匹配")?;
            if !asset_identities.insert((aid, vid)) {
                return Err("单元重复引用同一资产形象".into());
            }
            let order = reference
                .get("order")
                .and_then(Value::as_u64)
                .filter(|o| *o > 0)
                .ok_or("素材顺序必须从1开始")?;
            if !orders.insert(order) {
                return Err("素材顺序重复".into());
            }
            let use_kind = text(reference, "use");
            if !text(reference, "imageAssetId").is_empty()
                || !text(reference, "imageVariantId").is_empty()
            {
                let pair =
                    image_identity(asset, assets).ok_or("未找到 INLINE 形象明确的父图身份")?;
                if pair
                    != (
                        text(reference, "imageAssetId").to_string(),
                        text(reference, "imageVariantId").to_string(),
                    )
                {
                    return Err("分镜图片映射与已登记来源关系不一致".into());
                }
            }
            if asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("AUDIO")
                && !["audio", "video"].contains(&use_kind)
            {
                return Err("声音来源不能要求人物/场景图片；纯 VO/OS 不证明角色入画".into());
            }
            if !["audio", "video"].contains(&use_kind) {
                image_identities.insert((
                    if text(reference, "imageAssetId").is_empty() {
                        aid
                    } else {
                        text(reference, "imageAssetId")
                    },
                    if text(reference, "imageVariantId").is_empty() {
                        vid
                    } else {
                        text(reference, "imageVariantId")
                    },
                ));
            }
            let scopes = asset.get("applicability");
            let outside_scope = if let Some(obj) = scopes.and_then(Value::as_object) {
                obj.get("episodes")
                    .and_then(Value::as_array)
                    .filter(|v| !v.is_empty())
                    .map(|v| !v.iter().any(|e| e.as_str() == Some(eid)))
                    .unwrap_or(false)
                    || obj
                        .get("scenes")
                        .and_then(Value::as_array)
                        .filter(|v| !v.is_empty())
                        .map(|v| !v.iter().any(|s| s.as_str() == Some(sid)))
                        .unwrap_or(false)
            } else if let Some(list) = scopes.and_then(Value::as_array).filter(|v| !v.is_empty()) {
                !list
                    .iter()
                    .any(|p| text(p, "episodeId") == eid && text(p, "sceneId") == sid)
            } else {
                false
            };
            if outside_scope {
                if let Some(issues) = scope_issues.as_deref_mut() {
                    issues.push(json!({"code":"ASSET_SCENE_SCOPE","unitId":uid,"coordinate":format!("{uid}.assetRefs[{}]",order-1),"assetId":aid,"variantId":vid,"episodeId":eid,"sceneId":sid,"use":use_kind,"sourceRefs":reference["sourceRefs"],"declaredApplicability":scopes,"problem":crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION}));
                } else {
                    return Err(crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION.into());
                }
            }
            let rs = strings(reference, "sourceRefs")?;
            refs_valid(&rs, &all, true)?;
            if rs.iter().any(|r| !source_refs.contains(r)) {
                return Err("单元资产引用越出本单元来源".into());
            }
        }
        if image_identities.len() > image_limit {
            return Err(format!("单元 {uid} 需要 {} 张图片，超出渠道 {image_limit} 张上限；须修订单元，不能自动删图",image_identities.len()));
        }
        if !orders.is_empty() && (1..=orders.len() as u64).any(|o| !orders.contains(&o)) {
            return Err("素材顺序存在空档".into());
        }
        let shots = unit
            .get("shots")
            .and_then(Value::as_array)
            .ok_or("单元缺少六栏镜表")?;
        if shots.is_empty() {
            return Err("单元镜表不能为空".into());
        }
        let mut duration = 0.0;
        for (shot_index, shot) in shots.iter().enumerate() {
            let id = text(shot, "id");
            if id.is_empty() || !shot_ids.insert(id) {
                return Err("镜号空或重复".into());
            }
            let seconds = match shot.get("duration").and_then(Value::as_f64) {
                Some(seconds) if seconds.is_finite() && seconds > 0.0 => seconds,
                Some(0.0)
                    if defer_zero_duration
                        && shot["dialogueParts"].as_array().is_some_and(Vec::is_empty) =>
                {
                    let issues = scope_issues
                        .as_deref_mut()
                        .ok_or(crate::text_executor::AUTHOR_SHOT_DURATION_REJECTION)?;
                    issues.push(json!({"code":"SHOT_DURATION","unitId":uid,"coordinate":format!("{uid}.shots[{shot_index}].duration"),"shotId":id,"originalDuration":0,"sourceRefs":shot["sourceRefs"],"problem":crate::text_executor::AUTHOR_SHOT_DURATION_REJECTION}));
                    // Only a rejected candidate can carry this issue. Do not
                    // invent seconds or alter the original shot/voice windows.
                    0.0
                }
                _ => return Err(crate::text_executor::AUTHOR_SHOT_DURATION_REJECTION.into()),
            };
            duration += seconds;
            for field in ["camera", "picture", "lighting", "sound", "transition"] {
                if text(shot, field).trim().is_empty() {
                    return Err(format!("镜 {id} 缺少 {field}"));
                }
            }
            let rs = strings(shot, "sourceRefs")?;
            refs_valid(&rs, &all, true)?;
            covered.extend(rs.iter().cloned());
            if rs.iter().any(|r| !source_refs.contains(r)) {
                return Err("镜头引用越出本单元来源".into());
            }
            refs_valid(&strings(shot, "contextRefs")?, &all, false)?;
            for part in shot
                .get("dialogueParts")
                .and_then(Value::as_array)
                .ok_or("镜表缺少对白/声窗字段")?
            {
                let line_id = text(part, "lineId");
                let quote = quotes.get(line_id).ok_or("对白引用未知源句")?;
                let body = text(part, "text");
                if let (Some(start), Some(end)) =
                    (part["quoteStart"].as_u64(), part["quoteEnd"].as_u64())
                {
                    let range = SourceRange {
                        start: start as usize,
                        end: end as usize,
                    };
                    if start >= end
                        || !spoken_ranges(quote)
                            .iter()
                            .any(|segment| range.start >= segment.start && range.end <= segment.end)
                        || slice_utf16(&source.text, &range)? != body
                    {
                        return Err("对白原话区间与物化文字不一致或包含演法".into());
                    }
                } else if part.get("quoteStart").is_some() || part.get("quoteEnd").is_some() {
                    return Err("原话区间必须为完整UTF16半开区间".into());
                }
                let start = part
                    .get("voiceStart")
                    .and_then(Value::as_f64)
                    .ok_or("缺少声窗开始")?;
                let end = part
                    .get("voiceEnd")
                    .and_then(Value::as_f64)
                    .ok_or("缺少声窗结束")?;
                if !start.is_finite()
                    || !end.is_finite()
                    || start < 0.0
                    || end <= start
                    || end > seconds + 0.000001
                {
                    return Err("声窗越出镜长或为空，禁止机械裁窗过关".into());
                }
                if text(part, "speaker") != quote.speaker || text(part, "channel") != quote.channel
                {
                    return Err("说者或声道被改写".into());
                }
                if !source_refs.iter().any(|r| {
                    source
                        .rows
                        .iter()
                        .find(|p| &p.id == r)
                        .and_then(|p| p.dialogue.as_ref())
                        .map(|q| q.id == quote.id)
                        .unwrap_or(false)
                }) {
                    return Err("对白越出单元来源".into());
                }
                let position = source_dialogue_order[line_id];
                if position < prior_dialogue_order {
                    return Err("源原话顺序被改写".into());
                }
                prior_dialogue_order = position;
                if body.is_empty() {
                    return Err("对白原片段不能为空".into());
                }
                let accumulated = spoken.entry(line_id).or_default();
                if let Some(start) = part["quoteStart"].as_u64() {
                    if spoken_position(quote, accumulated.encode_utf16().count())
                        != Some(start as usize)
                    {
                        return Err("原话区间重复、跳过或乱序".into());
                    }
                }
                accumulated.push_str(body);
                if !expected_spoken(source, quote)?.starts_with(accumulated.as_str()) {
                    return Err("对白新增、遗漏、重放、标点规范化或非连续片段".into());
                }
            }
        }
        if duration > max + 0.000001 {
            return Err(format!(
                "单元 {uid} 时长 {duration:.2} 秒超过渠道 {max:.2} 秒；须修订而非截断"
            ));
        }
        // A naturally short ending is legal editorial content; the channel may reject generation later.
        if duration < min && unit.get("allowShortEnding").and_then(Value::as_bool) == Some(false) {
            return Err("当前渠道最小时长未满足".into());
        }
        if let Some(recorded) = unit.get("duration").and_then(Value::as_f64) {
            if (recorded - duration).abs() > 0.001 {
                return Err("单元总长与镜长合计不一致".into());
            }
        }
    }
    let expected: Vec<_> = source
        .rows
        .iter()
        .filter(|r| {
            r.kind == "body"
                && expected_refs
                    .map(|refs| refs.contains(&r.id))
                    .unwrap_or(true)
                && r.episode_id
                    .as_ref()
                    .map(|id| units.iter().any(|u| text(u, "episodeId") == id))
                    .unwrap_or(false)
        })
        .map(|r| r.id.clone())
        .collect();
    let shot_covered: Vec<_> = units
        .iter()
        .flat_map(|unit| unit["shots"].as_array().into_iter().flatten())
        .flat_map(|shot| strings(shot, "sourceRefs").unwrap_or_default())
        .collect();
    if normalize_coverage(source, &shot_covered)? != expected {
        return Err(
            "单元须依源顺序完整覆盖制作正文；跨单元同句仅允许连续原片段且全句恰好一次".into(),
        );
    }
    for row in source.rows.iter().filter(|r| covered.contains(&r.id)) {
        if let Some(quote) = &row.dialogue {
            let expected = expected_spoken(source, quote)?;
            if !expected.is_empty()
                && spoken.get(quote.id.as_str()).map(String::as_str) != Some(expected.as_str())
            {
                return Err(format!("对白 {} 未完整出现一次", quote.id));
            }
        }
    }
    Ok(())
}

fn compile_prompt(unit: &Value, visual_style: &str) -> String {
    let mut parts = vec![];
    if !visual_style.trim().is_empty() {
        parts.push(format!("项目画风：{visual_style}"));
    }
    let mapping: Vec<_> = unit
        .get("assetRefs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| {
            format!(
                "{{{{asset:{}:{}}}}}：{}参考",
                text(r, "assetId"),
                text(r, "variantId"),
                text(r, "use")
            )
        })
        .collect();
    if !mapping.is_empty() {
        parts.push(format!("参考素材顺序：\n{}", mapping.join("\n")));
    }
    parts.push(format!(
        "演出总指令：{}\n场景：{}\n人物：{}\n道具：{}\n初态：{}\n光影：{}\n末态：{}",
        text(unit, "unitDirection"),
        text(unit, "scene"),
        unit.get("participants")
            .and_then(Value::as_array)
            .map(|v| v
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("、"))
            .unwrap_or_default(),
        unit.get("props")
            .and_then(Value::as_array)
            .map(|v| v
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("、"))
            .unwrap_or_default(),
        text(unit, "initialState"),
        text(unit, "lighting"),
        text(unit, "exitState")
    ));
    let mut offset = 0.0;
    for shot in unit
        .get("shots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let duration = shot.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
        let dialogue: Vec<_> = shot
            .get("dialogueParts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|p| {
                format!(
                    "{}{}（{}；{:.2}–{:.2}秒）：{}；演法：{}",
                    if text(p, "speaker").is_empty() {
                        "原声"
                    } else {
                        text(p, "speaker")
                    },
                    if text(p, "channel").is_empty() {
                        String::new()
                    } else {
                        format!("—{}", text(p, "channel"))
                    },
                    text(p, "presentation"),
                    offset + p.get("voiceStart").and_then(Value::as_f64).unwrap_or(0.0),
                    offset + p.get("voiceEnd").and_then(Value::as_f64).unwrap_or(0.0),
                    text(p, "text"),
                    text(p, "delivery")
                )
            })
            .collect();
        parts.push(format!(
            "镜头 {:.2}–{:.2}秒\n机位：{}\n画面：{}\n光影：{}\n声音：{}\n台词：{}\n转场：{}",
            offset,
            offset + duration,
            text(shot, "camera"),
            text(shot, "picture"),
            text(shot, "lighting"),
            text(shot, "sound"),
            if dialogue.is_empty() {
                "无对白".into()
            } else {
                dialogue.join("\n")
            },
            text(shot, "transition")
        ));
        offset += duration;
    }
    parts.join("\n\n")
}

/// Legacy prose is copied from adopted unit fields. Asset mapping is intentionally external/manual.
pub fn render_legacy_unit(unit: &Value) -> String {
    compile_prompt(unit, text(unit, "visualStyle"))
}

/// Add an ordered channel binding legend without rewriting the user's prompt.
/// Names are display values supplied from the exact current asset identities.
pub fn compile_reference_template(prompt: &str, bindings: &[Value]) -> Result<String, String> {
    creative::reference_template(prompt, bindings)
}

fn add_delivery_fields(draft: &mut Value, style: &str) -> Result<(), String> {
    for unit in draft
        .get_mut("units")
        .and_then(Value::as_array_mut)
        .ok_or("缺少单元")?
    {
        let duration: f64 = unit
            .get("shots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| s.get("duration").and_then(Value::as_f64))
            .sum();
        let prompt = compile_prompt(unit, style);
        let map = unit.as_object_mut().ok_or("单元不是对象")?;
        // Preserve the existing f64 JSON form for exact totals/cache inputs;
        // only a tiny accumulated fractional error changes the numeric value.
        let canonical_duration = crate::binding::normalized_integer_duration(duration)
            .map(|seconds| seconds as f64)
            .unwrap_or(duration);
        map.insert("duration".into(), json!(canonical_duration));
        map.insert("prompt".into(), json!(prompt));
        if let Some(refs) = map.get_mut("assetRefs").and_then(Value::as_array_mut) {
            for r in refs {
                let usage = text(r, "use").to_string();
                r["role"] = json!(usage);
            }
        }
    }
    Ok(())
}

/// Mechanical port of episode-boundary-seed's last-scene/complete-node tail.
/// The budget is soft; an oversized final source row remains whole. This is
/// source evidence, not proof that the current episode inherits a prior state.
fn previous_source_boundary(source: &LockedSource, episode: &SourceEpisode) -> Value {
    let Some(position) = source
        .episodes
        .iter()
        .position(|e| e.id == episode.id)
        .filter(|p| *p > 0)
    else {
        return Value::Null;
    };
    let previous = &source.episodes[position - 1];
    let scene = source
        .scenes
        .iter()
        .filter(|s| s.episode_id == previous.id)
        .last();
    let mut nodes: Vec<_> = source
        .rows
        .iter()
        .filter(|r| {
            r.episode_id.as_ref() == Some(&previous.id)
                && r.kind != "format"
                && scene.map(|s| s.row_ids.contains(&r.id)).unwrap_or(true)
        })
        .collect();
    let original_count = nodes.len();
    let required = nodes
        .iter()
        .rposition(|r| r.kind == "body")
        .map(|i| nodes[i].id.clone());
    while nodes.iter().filter(|r| !r.text.trim().is_empty()).count() > 1
        && nodes
            .last()
            .map(|r| r.range.end)
            .unwrap_or(0)
            .saturating_sub(nodes.first().map(|r| r.range.start).unwrap_or(0))
            > 2400
    {
        if nodes
            .first()
            .is_some_and(|r| required.as_deref() == Some(r.id.as_str()))
        {
            break;
        }
        nodes.remove(0);
    }
    let clipped = nodes.len() != original_count;
    let width = nodes
        .last()
        .map(|r| r.range.end)
        .unwrap_or(0)
        .saturating_sub(nodes.first().map(|r| r.range.start).unwrap_or(0));
    json!({"version":"source-boundary-nodes/v1","episodeId":previous.id,"sceneId":scene.map(|s|&s.id),
        "selectionKind":if clipped {"TAIL_NODES"} else if scene.is_some() {"SAVED_SCENE"} else {"WHOLE_EPISODE_FALLBACK"},
        "sourceRefs":nodes.iter().map(|r| &r.id).collect::<Vec<_>>(),"sourceSha256":source.sha256,
        "sourceRange":{"start":nodes.first().map(|r|r.range.start),"end":nodes.last().map(|r|r.range.end)},
        "clipped":clipped,"budgetExceeded":width>2400,"currentApplicability":"UNKNOWN",
        "policy":"末场或完整尾部源节点是有坐标的历史证据；不自动继承状态，不把选段冒充完整前场。"})
}

pub(crate) const CONTEXT_INPUT_PROFILE_FIELD: &str = "storyboardContextInputProfile";
pub(crate) const CONTEXT_INPUT_PROFILE: &str = "owner-evidence/v1";

fn uses_owner_context(input: &Value) -> Result<bool, String> {
    match input.get(CONTEXT_INPUT_PROFILE_FIELD) {
        None => Ok(false),
        Some(Value::String(value)) if value == CONTEXT_INPUT_PROFILE => Ok(true),
        _ => Err("分镜上下文输入版本未知；保留冻结流程，不改变已提交输入".into()),
    }
}

fn author_context(
    base: &[SourceRow],
    rows: &[SourceRow],
    owned: &[String],
    evidence: &HashSet<String>,
    scoped: bool,
) -> Vec<SourceRow> {
    let historical: HashSet<_> = base.iter().map(|row| row.id.as_str()).collect();
    base.iter()
        .chain(rows)
        .filter(|row| {
            !owned.contains(&row.id)
                && (row.kind == "context"
                    || evidence.contains(&row.id)
                    || (!scoped && historical.contains(row.id.as_str())))
        })
        .cloned()
        .collect()
}

fn relevant_context(
    source: &LockedSource,
    episode: &SourceEpisode,
    assets: &[Value],
    foundation: &Value,
) -> Vec<SourceRow> {
    let first = source
        .rows
        .iter()
        .filter(|r| r.episode_id.as_ref() == Some(&episode.id))
        .map(|r| r.range.start)
        .min()
        .unwrap_or(0);
    let identities: HashSet<_> = assets
        .iter()
        .map(|a| {
            if text(a, "baseAssetId").is_empty() {
                text(a, "id")
            } else {
                text(a, "baseAssetId")
            }
        })
        .collect();
    let mut selected = HashSet::new();
    let mut anonymous_utf16 = 0;
    let background_names: Vec<_> = assets
        .iter()
        .map(|a| text(a, "name").split('｜').next().unwrap_or(""))
        .filter(|n| !n.is_empty())
        .collect();
    for row in source.rows.iter().filter(|r| r.kind == "context") {
        let matched = background_names.iter().any(|name| row.text.contains(name));
        let explicit_global = ["世界规则", "世界观", "全局设定", "背景规则"]
            .iter()
            .any(|key| row.text.contains(key));
        let width = row.text.encode_utf16().count();
        if matched || explicit_global || anonymous_utf16 + width <= 12_000 {
            selected.insert(row.id.clone());
            if !matched && !explicit_global {
                anonymous_utf16 += width;
            }
        }
    }
    // Port of the old local historical retrieval: do not collapse distinct
    // claims to a guessed latest state. All relevant original evidence and its
    // same-episode neighbors are history, never current production coverage.
    let names: Vec<String> = assets
        .iter()
        .flat_map(|asset| {
            let mut names = vec![text(asset, "name")
                .split('｜')
                .next()
                .unwrap_or("")
                .to_string()];
            names.extend(
                asset["aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            );
            names
        })
        .filter(|name| !name.trim().is_empty())
        .collect();
    let mut historic = HashSet::new();
    for fact in foundation
        .get("facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| {
            identities.contains(text(f, "assetId"))
                || names.iter().any(|name| text(f, "subject") == name)
        })
    {
        for id in strings(fact, "sourceRefs").unwrap_or_default() {
            if source
                .rows
                .iter()
                .any(|r| r.id == id && r.range.end <= first && r.kind == "body")
            {
                historic.insert(id);
            }
        }
    }
    for row in source.rows.iter().filter(|r| {
        r.kind == "body" && r.range.end <= first && names.iter().any(|name| r.text.contains(name))
    }) {
        historic.insert(row.id.clone());
    }
    for (index, row) in source
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| historic.contains(&row.id))
    {
        for neighbor in source.rows
            [index.saturating_sub(1)..=(index + 1).min(source.rows.len() - 1)]
            .iter()
            .filter(|r| r.kind == "body" && r.episode_id == row.episode_id && r.range.end <= first)
        {
            selected.insert(neighbor.id.clone());
        }
    }
    selected.extend(
        strings(&previous_source_boundary(source, episode), "sourceRefs").unwrap_or_default(),
    );
    source
        .rows
        .iter()
        .filter(|r| selected.contains(&r.id))
        .cloned()
        .collect()
}

pub(super) fn validate_event_bindings(draft: &Value, plan: &Value) -> Result<(), String> {
    validate_event_bindings_for_repair(draft, plan, None)
}

fn validate_event_bindings_for_repair(
    draft: &Value,
    plan: &Value,
    issues: Option<&[Value]>,
) -> Result<(), String> {
    let events = plan["events"].as_array().ok_or("缺少事件目录")?;
    for unit in draft["units"].as_array().ok_or("缺少单元")? {
        let event = events
            .iter()
            .find(|e| text(e, "id") == text(unit, "eventId"))
            .ok_or("单元引用未知事件")?;
        // The source validator generated this issue from the unit's unique
        // locked source scene. Only the unit coordinate may be deferred; a
        // frozen event cannot follow the same incorrect unit coordinate.
        let coordinate_issue = issues.into_iter().flatten().find(|issue| {
            issue["code"] == "UNIT_SCENE_COORDINATE" && issue["unitId"] == unit["id"]
        });
        if let Some(issue) = coordinate_issue {
            if issue["sceneId"] != unit["sceneId"]
                || issue["sourceRefs"] != unit["sourceRefs"]
                || issue["expectedSceneId"] != event["sceneId"]
            {
                return Err("单元跨出事件所属场景".into());
            }
        }
        if text(unit, "sceneId") != text(event, "sceneId") {
            if coordinate_issue.is_none() {
                return Err("单元跨出事件所属场景".into());
            }
        }
        let allowed = strings(event, "sourceRefs")?;
        if strings(unit, "sourceRefs")?
            .iter()
            .any(|r| !allowed.contains(r))
        {
            return Err("单元来源越出事件范围".into());
        }
        let boundary = unit.get("boundaryBefore").ok_or("单元缺少真实边界")?;
        if ![
            "episode_start",
            "continuous",
            "capacity",
            "new_event",
            "time_space_jump",
        ]
        .contains(&text(boundary, "kind"))
            || text(boundary, "reason").trim().is_empty()
        {
            return Err("单元边界类型/依据无效".into());
        }
    }
    Ok(())
}

const STORYBOARD_METHOD: &str = "authors-then-editor/v2";

/// Engine supplies only this workflow's compact, source-bound checkpoints.
/// A method marker alone cannot override evidence that a later author may have
/// consumed an old per-batch finished tail. Without an exact compatible request
/// there is no safe way to reuse that author's cache, so stop that scope before
/// even asking the executor (which could otherwise send a new paid request).
fn incompatible_resume_scopes(
    input: &Value,
    source: &LockedSource,
) -> Result<HashMap<String, Value>, String> {
    let mut blocked = HashMap::new();
    let method = text(input, "storyboardMethod");
    if !method.is_empty() && ![STORYBOARD_METHOD, "author-batch-finish/v1"].contains(&method) {
        for episode in &source.episodes {
            blocked.insert(episode.id.clone(),json!({"episodeId":episode.id,"status":"blocked","code":"STORYBOARD_METHOD_INCOMPATIBLE","historicalMethod":method,"requiredMethod":STORYBOARD_METHOD,"reason":"流程冻结的第三阶段执行方法未知或不兼容；保留原成果和缓存，不自动重做主创。"}));
        }
        return Ok(blocked);
    }
    let history = match input.get("storyboardHistory") {
        None | Some(Value::Null) => return Ok(blocked),
        Some(Value::Array(history)) => history,
        _ => return Err(
            "第三阶段续跑历史必须为本流程的紧凑checkpoint数组；无法证明范围兼容，未派发模型请求"
                .into(),
        ),
    };
    for record in history {
        if !record.is_object() {
            return Err("第三阶段续跑checkpoint结构无效；原缓存与成果保留，未派发模型请求".into());
        }
        if !["storyboardFinish", "storyboard:finish", "finish"].contains(&text(record, "stage")) {
            continue;
        }
        let mode = text(record, "finishMode");
        let version = text(record, "executionBatchVersion");
        let schema = text(record, "outputSchemaVersion");
        let accepted = ["accepted", "completed"].contains(&text(record, "status"));
        if mode == "delta"
            && [
                "related-range-finish/v1",
                "carrier-repair/v1",
                creative::AUTHOR_SCOPE_REPAIR_EXECUTION_VERSION,
                creative::FINISH_SCOPE_REPAIR_EXECUTION_VERSION,
            ]
            .contains(&version)
            && (schema == "storyboard-delta/v2" || (!accepted && schema.is_empty()))
        {
            continue;
        }
        // Old full-draft batch finishes, incomplete evidence and contradictory
        // transport identities all fail closed. Never reinterpret their result
        // as an accepted delta or alter the immutable old workflow snapshot.
        let episode = text(record, "episodeId");
        let scopes: Vec<_> = if source.episodes.iter().any(|item| item.id == episode) {
            vec![episode.to_string()]
        } else {
            source.episodes.iter().map(|item| item.id.clone()).collect()
        };
        for episode in scopes {
            blocked.insert(episode.clone(),json!({"episodeId":episode,"status":"blocked","code":"STORYBOARD_METHOD_INCOMPATIBLE","historicalMethod":"author-batch-finish/v1","requiredMethod":STORYBOARD_METHOD,"history":{"stage":record["stage"],"status":record["status"],"requestHash":record["requestHash"],"finishMode":record["finishMode"],"executionBatchVersion":record["executionBatchVersion"],"outputSchemaVersion":record["outputSchemaVersion"]},"reason":"本范围已有旧逐批整编请求；后续作者可能消费旧整编末态，无法证明新请求与其缓存相同。保留已付回包、缓存与成稿，不无声重做主创；请单独核对历史范围后明确新建制作或局部修订。"}));
        }
    }
    Ok(blocked)
}

/// Frozen input: source/foundation/assets/settings/capabilities/selection.
/// Decisions live separately at decisions[episodeId].selectedFantasyIds, so a
/// review continuation reuses the identical accepted planning request.
/// selection.unitIds + existingUnits + revisionInstruction selects local rework.
pub async fn run_storyboard(
    input: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    // Runtime decisions/history are not the parent's frozen byte identity.
    // This complete packet is only a projection/proof argument, never a role input.
    let frozen_finish_assets = input.get("frozenFinisherAssetContext").unwrap_or(input);
    let creative_enabled = creative::uses_profile(input)?;
    let mac_pack = super::stage3_pack::uses_profile(input)?;
    if mac_pack {
        if !creative_enabled {
            return Err("Mac生产规则适配缺少锁定原话与导出合同，未派发请求".into());
        }
        let mut bound = json!({});
        super::stage3_pack::attach_profile(&mut bound, input)?;
    }
    let carrier_enabled = creative::uses_carrier_profile(input)?;
    if carrier_enabled && !creative_enabled {
        return Err("视频载体合同须与原三岗制作合同共同使用".into());
    }
    if carrier_enabled {
        capabilities(input)?;
        validate_carrier_capabilities(&input["capabilities"])?;
    }
    let revision_enabled = creative::uses_revision_profile(input)?;
    if revision_enabled
        && (!creative_enabled
            || !input
                .pointer("/selection/unitIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| !ids.is_empty()))
    {
        return Err("已采用规划局部重做须使用原创作合同并明确选择单元".into());
    }
    let creative_executor = creative::Executor(executor);
    let executor: &dyn TextExecutor = &creative_executor;
    asset_cards::validate_profile(input)?;
    let compact_dependency_exits = storyboard_dependencies::uses_profile(input)?;
    let scoped_author_context = uses_owner_context(input)?;
    let source: LockedSource =
        serde_json::from_value(input.get("source").cloned().ok_or("缺少锁定原文")?)
            .map_err(|e| format!("原文结构无效：{e}"))?;
    let source = upgrade_source_dialogues(&source)?;
    validate_source(&source)?;
    if source.diagnostics.iter().any(|d| d.blocking) {
        return Err("原文仍有未定集场范围，不能创作假定来源的分镜".into());
    }
    if input
        .pointer("/selection/unitIds")
        .and_then(Value::as_array)
        .map(|ids| !ids.is_empty())
        .unwrap_or(false)
    {
        return run_storyboard_revision(&source, input, executor, model, reasoning).await;
    }
    let incompatible_scopes = incompatible_resume_scopes(input, &source)?;
    let (minimum_duration, _, _) = capabilities(input)?;
    let caps = input.get("capabilities").unwrap();
    let assets = input
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("缺少资产身份目录")?;
    let raw_settings = input.get("settings").cloned().unwrap_or_else(|| json!({}));
    let mut settings = creative::effective_settings(&raw_settings, creative_enabled);
    if text(&settings, "fantasyMode").is_empty() {
        settings["fantasyMode"] = json!("recommend");
    }
    if creative_enabled {
        settings["fantasyEnabled"] = json!(text(&settings, "fantasyMode") != "off");
    }
    let fantasy_mode = text(&settings, "fantasyMode");
    if !["off", "recommend", "all", "review"].contains(&fantasy_mode) {
        return Err("幻想选择方式无效".into());
    }
    // This is an execution stop point, not a creative input. Keeping it out of
    // settings preserves accepted planning requests from unmarked review runs.
    let require_fantasy_review = raw_settings["requireFantasyReview"].as_bool() == Some(true);
    let style = text(&settings, "visualStyle");
    let direction = production_direction(input);
    let requested: Option<HashSet<&str>> = input
        .pointer("/selection/episodeIds")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect());
    let requested_one = input.get("episodeId").and_then(Value::as_str);
    if requested_one
        .map(|id| !source.episodes.iter().any(|e| e.id == id))
        .unwrap_or(false)
        || requested
            .as_ref()
            .map(|ids| {
                ids.iter()
                    .any(|id| !source.episodes.iter().any(|e| e.id == *id))
            })
            .unwrap_or(false)
    {
        return Err("指定制作集不存在".into());
    }
    let mut all_units = Vec::new();
    let mut plans = Vec::new();
    let mut blocked = Vec::new();
    let mut waiting = Vec::new();
    let mut unavailable = HashSet::new();
    let mut previous = input
        .get("previousBoundary")
        .cloned()
        .unwrap_or(Value::Null);
    let mut all_realizations = Vec::new();
    let mut all_revisions = Vec::new();
    let mut all_unresolved = Vec::new();
    let mut updated_episodes = Vec::new();
    let mut continuity_revision = Value::Null;
    for (position, episode) in source.episodes.iter().enumerate() {
        if requested_one.map(|id| id != episode.id).unwrap_or(false)
            || requested
                .as_ref()
                .map(|ids| !ids.contains(episode.id.as_str()))
                .unwrap_or(false)
        {
            continue;
        }
        if let Some(problem) = incompatible_scopes.get(&episode.id) {
            blocked.push(problem.clone());
            unavailable.insert(episode.id.clone());
            continue;
        }
        let prior_id = position
            .checked_sub(1)
            .map(|index| source.episodes[index].id.as_str());
        if prior_id.map(|id| unavailable.contains(id)).unwrap_or(false) {
            blocked.push(json!({"episodeId":episode.id,"status":"waiting_dependency","code":"CONTINUITY_PREDECESSOR_UNAVAILABLE","dependsOn":prior_id,"reason":"连续前集尚未完成，保留原末态，处理前置后继续。"}));
            unavailable.insert(episode.id.clone());
            continue;
        }
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.episode_id.as_ref() == Some(&episode.id) && r.kind == "body")
            .cloned()
            .collect();
        if rows.is_empty() {
            blocked.push(json!({"episodeId":episode.id,"reason":"当前集没有可制作正文"}));
            unavailable.insert(episode.id.clone());
            continue;
        }
        let scenes: Vec<_> = source
            .scenes
            .iter()
            .filter(|s| s.episode_id == episode.id)
            .cloned()
            .collect();
        let mut current_assets = episode_assets(assets, &episode.id);
        asset_cards::inherit_identity_aliases(input, &mut current_assets)?;
        let model_assets = asset_cards::project_for_episode(input, &current_assets, &episode.id)?;
        let scoped_decisions = match scoped_author_decisions(&source, input, &rows) {
            Ok(package) => package,
            Err(reason) => {
                blocked.push(json!({"episodeId":episode.id,"status":"blocked","code":"AUTHOR_DECISION_INVALID","reason":reason}));
                unavailable.insert(episode.id.clone());
                continue;
            }
        };
        let base_context = relevant_context(
            &source,
            episode,
            &current_assets,
            input.get("foundation").unwrap_or(&Value::Null),
        );
        let context = decision_context(&source, &rows, &base_context, scoped_decisions.as_ref());
        let foundation_episode = input
            .pointer("/foundation/episodes")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|e| text(e, "episodeId") == episode.id));
        if !previous.is_null() && prior_id != Some(text(&previous, "episodeId")) {
            previous = Value::Null;
            continuity_revision = Value::Null;
        }
        if !continuity_revision.is_null() {
            let mut evidence = continuity_revision.clone();
            evidence["episodeId"] = json!(episode.id);
            evidence["action"] = json!("recompute_affected_chain");
            evidence["previouslyAcceptedPlan"] = json!(input["storyboardHistory"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| r["stage"] == "storyboardPlan"
                    && r["status"] == "accepted"
                    && r["episodeId"] == episode.id));
            executor.continuity_changed(&evidence).await?;
        }
        let episode_facts = scoped_facts(input, &rows, &context);
        let decision = input
            .get("decisions")
            .and_then(|decisions| decisions.get(&episode.id))
            .and_then(|decision| decision.get("selectedFantasyIds"))
            .or_else(|| raw_settings.get("selectedFantasyIds"));
        let result: Result<Value, String> = async {
            let mut plan_methods=vec!["storyboard-common","storyboard-source-context","sound-rhythm"];
            let mut plan_input = json!({"episode":episode,"productionDirection":direction,"foundationEpisode":foundation_episode,"sourceRows":rows,"contextRows":context,"facts":episode_facts,"assets":model_assets,"settings":settings,"capabilities":caps,"previousBoundary":previous,"previousSourceBoundary":previous_source_boundary(&source,episode),"availableScenarioKeys":original::SCENARIO_KEYS,"capacityBrief":original::capacity_brief(caps,None,input.get("voiceReference"),&rows),"historyEvidenceRule":"contextRows仅为历史或公共背景证据，currentApplicability=UNKNOWN；相同姓名/邻文不证明同一身份、当拍知情或状态自动延续。当前原文优先。"});
            asset_cards::attach_catalog_scope(&mut plan_input, input, &episode.id);
            creative::mark_input(&mut plan_input, creative_enabled);
            creative::attach_carrier_profile(&mut plan_input, input)?;
            super::stage3_pack::attach_profile(&mut plan_input, input)?;
            if !continuity_revision.is_null(){plan_input["continuityRevision"]=continuity_revision.clone();}
            attach_author_decisions(&mut plan_input, scoped_decisions.as_ref());
            if fantasy_mode!="off" || methods_for(&rows,&[],&[],false,&episode_facts)?.contains(&"fantasy-expression") {plan_methods.push("fantasy-expression");}
            // The original coordinator returns malformed planning to the same
            // planner with its preserved candidate, rather than asking the user
            // to repair internal contracts or restarting the episode.
            let plan_schema_value=plan_schema(&rows,&context,&scenes,fantasy_mode);
            let base_plan_input=plan_input.clone();
            let origin_request=creative::prepare_request(TextRequest{stage:"storyboardPlan".into(),skill:"episode-emotion-planner".into(),model:model.into(),reasoning:reasoning.into(),input:plan_input.clone(),schema:plan_schema_value.clone(),rule_snapshot:snapshot("episode-emotion-planner",&plan_methods)?})?;
            let origin_hash=digest(&serde_json::to_string(&origin_request).map_err(|e|e.to_string())?);
            let mut original_raw_hash=String::new(); let mut last_problem=String::new();
            let mut attempt=0usize;
            let plan=loop {
                let mut rejected_raw=None;
                let request_hash=digest(&serde_json::to_string(&creative::prepare_request(TextRequest{input:plan_input.clone(),..origin_request.clone()})?).map_err(|e|e.to_string())?);
                let planned=call_validated(executor, "storyboardPlan", "episode-emotion-planner", model, reasoning,
                plan_input.clone(), plan_schema_value.clone(), &plan_methods, |value| {
                    let raw=value.clone();
                    validate_plan(value, &rows, &context, &scenes)?;
                    if let Err(reason)=original::normalize_event_plan(value,&rows) {
                        if crate::text_executor::planner_repairable_rejection(&reason) {rejected_raw=Some(raw);}
                        return Err(reason);
                    }
                    for fact in value["entryFacts"].as_array_mut().ok_or("缺少入口事实")? {
                        let evidence:Vec<_>=strings(fact,"sourceRefs")?.iter().map(|id|rows.iter().chain(&context).find(|r|&r.id==id).map(|r|r.text.clone()).ok_or("未知源证据")).collect::<Result<_,_>>()?;
                        fact["evidence"]=json!(evidence);fact["currentApplicability"]=json!("SOURCE_ANCHORED");
                    }
                    value["sourceFingerprints"]=json!(rows.iter().chain(&context).map(|r|(r.id.clone(),digest(&r.text))).collect::<HashMap<_,_>>());
                    if fantasy_mode == "off" && !value["fantasyCandidates"].as_array().unwrap().is_empty() { return Err("关闭额外幻想时不得新增幻想候选".into()); }
                    Ok(())
                }).await;
                match (planned,rejected_raw) {
                    (Ok(plan),_)=>break plan,
                    (Err(reason),Some(raw))=>{
                        if attempt>=2 || last_problem==reason {return Err(format!("PLAN_REPAIR_{}：原情绪统筹的定向修订未通过；已保存全部回包，不重复调用",if last_problem==reason{"NO_PROGRESS"}else{"LIMIT"}));}
                        if attempt==0 {original_raw_hash=digest(&raw.to_string());}
                        attempt+=1; last_problem=reason.clone();
                        plan_input=base_plan_input.clone();
                        plan_input["previousPlan"]=raw.clone();
                        plan_input["validationIssues"]=json!([{"code":"PLAN_EVENT_BOUNDARY","problem":reason}]);
                        plan_input["planRepair"]=json!({"version":"rejected-plan-repair/v1","attempt":attempt,"maximumAttempts":2,"sourceSha256":source.sha256,"originDomainRequestSha256":origin_hash,"originalPlanSha256":original_raw_hash,"previousRequestSha256":request_hash,"previousPlanSha256":digest(&raw.to_string())});
                    },
                    (Err(reason),None)=>return Err(reason),
                }
            };
            if fantasy_mode == "review" && require_fantasy_review && !plan["fantasyCandidates"].as_array().unwrap().is_empty() && decision.is_none() {
                return Ok(json!({"pendingPlan":plan}));
            }
            let selected_ids = match decision {
                Some(value) => Some(value.as_array().ok_or("幻想选择必须为id数组")?.iter().map(|id| id.as_str().ok_or("幻想选择id必须为字符串")).collect::<Result<HashSet<_>, _>>()?),
                None => None,
            };
            if selected_ids.as_ref().map(|ids| ids.iter().any(|id| !plan["fantasyCandidates"].as_array().unwrap().iter().any(|candidate| text(candidate, "id") == *id))).unwrap_or(false) {
                return Err("已选幻想机会不属于冻结计划，请重新核对选择".into());
            }
            let selected: Vec<_> = plan["fantasyCandidates"].as_array().unwrap().iter().filter(|candidate| if fantasy_mode!="off" && selected_ids.is_some() {selected_ids.as_ref().unwrap().contains(text(candidate,"id"))} else {match fantasy_mode {
                "all" => true,
                "recommend" | "review" => candidate["recommended"].as_bool() == Some(true),
                _ => false,
            }}).cloned().collect();
            let selected = creative::selected_candidates(selected, creative_enabled);
            let events=plan["events"].as_array().unwrap();
            let event_ids:Vec<_>=events.iter().map(|e|text(e,"id").to_string()).collect();
            let packages=plan["workPackages"].as_array().unwrap();
            let batches=original::author_batches(&plan,&rows,&selected)?;
            let mut created=Vec::new();let mut realizations=Vec::new();let mut revision_notes=Vec::new();let mut unresolved=Vec::new();let mut finish_provenance=Vec::new();
            let mut local_blocked=Vec::new();let mut failed_owners=HashSet::new();
            let mut accepted_boundary=previous.clone();let mut previous_batch:Option<String>=None;
            let mut package_outputs:Vec<(String,Value)>=Vec::new();
            let mut authored_scopes=Vec::new();
            let mut author_repair_cause = Value::Null;
            for (batch_index,batch) in batches.iter().enumerate() {
                let owners:Vec<_>=packages.iter().filter(|p|batch.package_ids.iter().any(|id|id==text(p,"id"))).collect();
                let dependencies:HashSet<_>=owners.iter().flat_map(|p|strings(p,"dependsOn").unwrap_or_default()).filter(|id|!batch.package_ids.contains(id)).collect();
                if batch.package_ids.iter().any(|id|failed_owners.contains(id)) || dependencies.iter().any(|id|failed_owners.contains(id)) {
                    local_blocked.push(json!({"episodeId":episode.id,"executionBatchId":batch.id,"packageIds":batch.package_ids,"status":"waiting_dependency","reason":"必要前稿尚未完成，保留已有步骤，处理关联前置后继续。"}));
                    failed_owners.extend(batch.package_ids.iter().cloned());accepted_boundary=Value::Null;continue;
                }
                let package_rows:Vec<_>=rows.iter().filter(|r|batch.source_refs.contains(&r.id)).cloned().collect();
                let candidates:Vec<_>=selected.iter().filter(|c|strings(c,"sourceRefs").map(|rs|rs.iter().any(|r|batch.source_refs.contains(r))).unwrap_or(false)).cloned().collect();
                let entry_ids:HashSet<_>=owners.iter().flat_map(|p|strings(p,"entryFactIds").unwrap_or_default()).collect();
                let entry_facts:Vec<_>=plan["entryFacts"].as_array().unwrap().iter().filter(|f|entry_ids.contains(text(f,"id"))).cloned().collect();
                let mut context_ids:HashSet<_>=owners.iter().flat_map(|p|strings(p,"contextRefs").unwrap_or_default()).collect();
                context_ids.extend(entry_facts.iter().flat_map(|f|strings(f,"sourceRefs").unwrap_or_default()));
                if let Some(first)=rows.iter().position(|r|batch.source_refs.contains(&r.id)) {context_ids.extend(rows[first.saturating_sub(3)..first].iter().map(|r|r.id.clone()));}
                // An execution split sees the remainder only as context, never as new production coverage.
                for owner in &owners {context_ids.extend(strings(owner,"sourceRefs")?.into_iter().filter(|r|!batch.source_refs.contains(r)));}
                let package_decisions=scoped_author_decisions(&source,input,&package_rows)?;
                let package_base_context=author_context(&base_context,&rows,&batch.source_refs,&context_ids,scoped_author_context);
                let package_context=decision_context(&source,&package_rows,&package_base_context,package_decisions.as_ref());
                let package_facts=if scoped_author_context {scoped_facts(input,&package_rows,&package_context)}else{episode_facts.clone()};
                let (package_assets,asset_selection)=asset_cards::project_for_scope(input,&current_assets,&episode.id,&package_rows,&package_context)?;
                let scenario_keys:Vec<_>=owners.iter().flat_map(|p|strings(p,"scenarioKeys").unwrap_or_default()).collect();
                let dependency_outputs:Vec<_>=package_outputs.iter().filter(|(id,_)|dependencies.contains(id)).map(|(_,output)|output).collect();
                let accepted_continuity=if accepted_boundary.is_null(){Value::Null}else{json!({"version":"accepted-tail-v1","executionBatchId":previous_batch,"unit":accepted_boundary,"sourceRefs":accepted_boundary["sourceRefs"],"boundaryBefore":events.iter().find(|e|text(e,"id")==text(owners[0],"eventId")).map(|e|&e["boundaryBefore"]),"evidenceRole":"ACCEPTED_CREATIVE_STATE_ONLY"})};
                let mut plan_view=json!({"emotionArc":plan["emotionArc"],"unitPackingPolicy":"event-first-v1","events":events.iter().filter(|e|owners.iter().any(|p|text(p,"eventId")==text(e,"id"))).collect::<Vec<_>>(),"workPackages":owners,"provisionalUnits":plan["provisionalUnits"].as_array().unwrap().iter().filter(|u|batch.provisional_unit_ids.iter().any(|id|id==text(u,"id"))).collect::<Vec<_>>(),"fantasyCandidates":candidates,"acceptedContinuity":accepted_continuity,"dependencyExits":dependency_outputs,"executionBatch":batch.value()});
                if compact_dependency_exits {
                    let source_order: Vec<_> = source.rows.iter().map(|row|row.id.clone()).collect();
                    plan_view["dependencyExits"] = storyboard_dependencies::project(&plan,&batch.package_ids,&source_order,&package_outputs)?;
                }
                let prefix=format!("{}:{}:",episode.id,batch.id);
                let mut author_input=json!({"episodeId":episode.id,"productionDirection":direction,"foundationEpisode":foundation_episode,"workPackage":owners.first(),"workPackages":owners,"plan":plan_view,"sourceRows":package_rows,"contextRows":package_context,"entryFacts":entry_facts,"facts":package_facts,"assets":package_assets,"selectedFantasy":candidates,"acceptedContinuity":accepted_continuity,"dependencyOutputs":dependency_outputs,"executionBatch":batch.value(),"settings":settings,"capabilities":caps,"capacityBrief":original::capacity_brief(caps,None,input.get("voiceReference"),&package_rows),"instruction":"作者范围不是视频单元数量。对白只选择lineId及spokenSegments内绝对UTF16 quoteStart/quoteEnd，演法不发声；Rust物化台词/说者/声道。本批正文完整一次；上下文和已采用前稿不重演，不自动继承前包幻想许可。"});
                if !asset_selection.is_null() { author_input["assetSelection"]=asset_selection; }
                asset_cards::attach_catalog_scope(&mut author_input, input, &episode.id);
                creative::mark_input(&mut author_input, creative_enabled);
                creative::attach_carrier_profile(&mut author_input, input)?;
            super::stage3_pack::attach_profile(&mut author_input, input)?;
                if !continuity_revision.is_null(){author_input["continuityRevision"]=continuity_revision.clone();}
                if creative_enabled { author_input["settings"]["fantasyEnabled"] = json!(!candidates.is_empty()); }
                if compact_dependency_exits {
                    author_input.as_object_mut().unwrap().remove("dependencyOutputs");
                }
                attach_author_decisions(&mut author_input, package_decisions.as_ref());
                let batch_result:Result<Value,String>=async {
                    let methods=methods_for(&package_rows,&candidates,&scenario_keys,false,&entry_facts)?;
                    let schema=draft_schema(&package_rows,&package_context,&current_assets,&event_ids,&episode.id);
                    {
                        let request = creative::prepare_request(TextRequest{stage:"storyboardCreate".into(),skill:"unit-performance-director".into(),model:model.into(),reasoning:reasoning.into(),input:author_input.clone(),schema:schema.clone(),rule_snapshot:snapshot("unit-performance-director",&methods)?})?;
                        if let Some(revision) = executor.author_chain_revision(&request,&author_repair_cause).await? {
                            author_input["authorChainRevision"] = revision;
                        }
                    }
                    let mut repair_candidate=None;
                    let result=call_validated(executor,"storyboardCreate","unit-performance-director",model,reasoning,
                        author_input.clone(),
                        schema.clone(),&methods,|value| {
                            let raw=value.clone();
                            if let Err(reason)=validate_authored(&source,value,&current_assets,caps,&batch.source_refs,style,&prefix) {
                                if creative_enabled && crate::text_executor::author_repairable_rejection(&reason) {
                                    let (candidate,issues)=author_validation_candidate(&source,&raw,&current_assets,caps,&batch.source_refs,style,&prefix,&plan,&candidates,&reason)?;
                                    if issues.is_empty(){return Err("主创待修原包没有可证实的适用范围问题；保留原拒绝，不重发".into());}
                                    repair_candidate=Some((raw,candidate,issues));
                                }
                                return Err(reason);
                            }
                            validate_event_bindings(value,&plan)?;validate_fantasy_profile(value,&candidates,creative_enabled)?;
                            creative::add_delivery(value,&source,style,&current_assets,creative_enabled)
                        }).await;
                    match (result,repair_candidate) {
                        (Err(reason),Some((raw,candidate,issues))) if crate::text_executor::author_repairable_rejection(&reason)=>{
                            let origin=creative::prepare_request(TextRequest{stage:"storyboardCreate".into(),skill:"unit-performance-director".into(),model:model.into(),reasoning:reasoning.into(),input:author_input.clone(),schema,rule_snapshot:snapshot("unit-performance-director",&methods)?})?;
                            let scope=AcceptedAuthorScope{index:batch_index,batch:batch.clone(),input:author_input.clone(),draft:candidate};
                            repair_author_scope(&source,&raw,&origin,&scope,issues,&plan,&rows,&context,&current_assets,&selected,&episode.id,model,reasoning,input.get("voiceReference"),caps,style,frozen_finish_assets,executor).await
                        }
                        (result,_)=>result,
                    }
                }.await;
                match batch_result {
                    Ok(authored)=>{
                        // The next author consumes the accepted author's actual
                        // tail, just as the original coordinator does. Finishing
                        // happens only after the related authors are complete.
                        accepted_boundary=authored["units"].as_array().unwrap().last().cloned().unwrap();previous_batch=Some(batch.id.clone());
                        for id in &batch.package_ids {package_outputs.push((id.clone(),authored.clone()));}
                        if !authored["authorRepairProvenance"].is_null(){
                            author_input["authorRepairProvenance"]=authored["authorRepairProvenance"].clone();
                            author_repair_cause=authored["authorRepairProvenance"].clone();
                        }
                        authored_scopes.push(AcceptedAuthorScope{index:batch_index,batch:batch.clone(),input:author_input,draft:authored});
                    }
                    Err(reason)=>{local_blocked.push(json!({"episodeId":episode.id,"executionBatchId":batch.id,"packageIds":batch.package_ids,"status":"blocked","reason":reason}));failed_owners.extend(batch.package_ids.iter().cloned());accepted_boundary=Value::Null;}
                }
            }
            let mut start=0;let mut failed_finish_owners=HashSet::new();let mut finished_refs=Vec::new();let mut completed_finishes=Vec::new();
            let mut prior_finish:Option<Value>=None;let mut prior_finish_index:Option<usize>=None;
            while start<authored_scopes.len() {
                let first=&authored_scopes[start];
                let dependencies:HashSet<_>=packages.iter().filter(|owner|first.batch.package_ids.iter().any(|id|id==text(owner,"id"))).flat_map(|owner|strings(owner,"dependsOn").unwrap_or_default()).collect();
                if first.batch.package_ids.iter().any(|id|failed_finish_owners.contains(id)) || dependencies.iter().any(|id|failed_finish_owners.contains(id)) {
                    local_blocked.push(json!({"episodeId":episode.id,"executionBatchId":first.batch.id,"packageIds":first.batch.package_ids,"status":"waiting_dependency","reason":"关联前稿整编尚未完成；保留已校验主创，不派发依赖整编。"}));
                    failed_finish_owners.extend(first.batch.package_ids.iter().cloned());start+=1;continue;
                }
                let prior=if prior_finish_index.map(|index|index+1==first.index).unwrap_or(false){prior_finish.as_ref()}else{None};
                let mut end=start+1;
                let mut prepared=finish_request(&authored_scopes[start..end],&plan,&rows,&context,&current_assets,&selected,&episode.id,model,reasoning,input.get("voiceReference"),prior,authored_scopes.get(end).and_then(|scope|scope.draft["units"].as_array().unwrap().first()))?;
                let measured=prepare_finish_assets(prepared.clone(),frozen_finish_assets,&current_assets,executor).await?;
                let actual_characters=executor.input_transport_characters(&measured.0)?;
                if actual_characters>FINISH_TRANSPORT_LIMIT {
                    local_blocked.push(json!({"episodeId":episode.id,"executionBatchId":first.batch.id,"packageIds":first.batch.package_ids,"status":"blocked","code":"FINISH_TRANSPORT_CAPACITY","actualCharacters":actual_characters,"limit":FINISH_TRANSPORT_LIMIT,"reason":"不可再分的完整作者范围超过桌面请求容量；已校验主创保留，不截断原文、现稿或规则。"}));
                    failed_finish_owners.extend(first.batch.package_ids.iter().cloned());start+=1;continue;
                }
                // Greedily retain the full adjacent range. Neither source video
                // seconds nor the author's 5/150 soft target splits the editor.
                while end<authored_scopes.len() && authored_scopes[end-1].index+1==authored_scopes[end].index {
                    let next_owners=&authored_scopes[end].batch.package_ids;
                    let next_dependencies:HashSet<_>=packages.iter().filter(|owner|next_owners.iter().any(|id|id==text(owner,"id"))).flat_map(|owner|strings(owner,"dependsOn").unwrap_or_default()).collect();
                    if next_owners.iter().any(|id|failed_finish_owners.contains(id)) || next_dependencies.iter().any(|id|failed_finish_owners.contains(id)) {break;}
                    let candidate=finish_request(&authored_scopes[start..end+1],&plan,&rows,&context,&current_assets,&selected,&episode.id,model,reasoning,input.get("voiceReference"),prior,authored_scopes.get(end+1).and_then(|scope|scope.draft["units"].as_array().unwrap().first()))?;
                    if finish_grouping_characters(&candidate.0,compact_dependency_exits)?>FINISH_TRANSPORT_LIMIT {break;}
                    prepared=candidate;end+=1;
                }
                let (request,base,scope_refs)=prepare_finish_assets(prepared,frozen_finish_assets,&current_assets,executor).await?;
                let actual_characters=executor.input_transport_characters(&request)?;
                if actual_characters>FINISH_TRANSPORT_LIMIT {
                    let owners:Vec<_>=authored_scopes[start..end].iter().flat_map(|scope|scope.batch.package_ids.iter().cloned()).collect();
                    local_blocked.push(json!({"episodeId":episode.id,"executionBatchId":request.input["executionBatch"]["id"],"packageIds":owners,"status":"blocked","code":"FINISH_TRANSPORT_CAPACITY","actualCharacters":actual_characters,"limit":FINISH_TRANSPORT_LIMIT,"reason":"完整关联范围超过实际发送容量；保留原主创、规则与已存在整编，不提交请求。"}));
                    failed_finish_owners.extend(owners);start=end;continue;
                }
                let finish_candidates=request.input["selectedFantasy"].as_array().unwrap().clone();
                let new_prefix=format!("{}:{}:",episode.id,text(&request.input["executionBatch"],"id"));
                let finish_result=async {
                    // Keep the accepted cache in its original delta shape. An
                    // internal materialization is recomputed on every replay;
                    // it cannot bypass the base hash or partition validation.
                    let mut response=executor.execute(request.clone()).await?;
                    let original_response=response.clone();
                    let validated=(|| {
                        let mut draft=apply_finish_delta(&base,&response)?;
                        let preserved_units:HashSet<_>=base["units"].as_array().unwrap().iter().map(|unit|text(unit,"id").to_string()).collect();
                        let preserved_shots:HashSet<_>=base["units"].as_array().unwrap().iter().flat_map(|unit|unit["shots"].as_array().unwrap()).map(|shot|text(shot,"id").to_string()).collect();
                        prefix_ids_preserving(&mut draft,&new_prefix,&preserved_units,&preserved_shots)?;
                        original::pack_event_units(&mut draft,&plan,caps["maxDuration"].as_f64().unwrap())?;
                        let validation_caps=carrier_validation_caps(&draft,caps,creative_enabled);
                        validate_authored(&source,&mut draft,&current_assets,&validation_caps,&scope_refs,style,&format!("{}:",episode.id))?;
                        validate_event_bindings(&draft,&plan)?;validate_fantasy_profile(&draft,&finish_candidates,creative_enabled)?;
                        creative::add_delivery(&mut draft,&source,style,&current_assets,creative_enabled)?;
                        mark_short_endings(&mut draft,minimum_duration);
                        response["materializedDraft"]=draft.clone();Ok::<Value,String>(draft)
                    })();
                    match validated {
                        Ok(draft)=>{executor.accept(&request,&response).await?;Ok(draft)},
                        Err(reason)=>{
                            executor.reject(&request,&original_response,&reason).await?;
                            if !creative_enabled || ![crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION,crate::text_executor::AUTHOR_SHOT_DURATION_REJECTION].contains(&reason.as_str()) {return Err(reason);}
                            // The editor can reintroduce a reference rejected in
                            // an author. The original coordinator routes either
                            // unadopted role back to the same bounded editor.
                            let mut candidate=apply_finish_delta(&base,&original_response)?;
                            let preserved_units:HashSet<_>=base["units"].as_array().unwrap().iter().map(|unit|text(unit,"id").to_string()).collect();
                            let preserved_shots:HashSet<_>=base["units"].as_array().unwrap().iter().flat_map(|unit|unit["shots"].as_array().unwrap()).map(|shot|text(shot,"id").to_string()).collect();
                            prefix_ids_preserving(&mut candidate,&new_prefix,&preserved_units,&preserved_shots)?;
                            original::pack_event_units(&mut candidate,&plan,caps["maxDuration"].as_f64().unwrap())?;
                            let validation_caps=carrier_validation_caps(&candidate,caps,true);
                            let (candidate,issues)=if reason==crate::text_executor::AUTHOR_SHOT_DURATION_REJECTION {
                                author_duration_candidate(&source,&candidate,&current_assets,&validation_caps,&scope_refs,style,&format!("{}:",episode.id),&plan,&finish_candidates)?
                            } else {
                                author_scope_candidate(&source,&candidate,&current_assets,&validation_caps,&scope_refs,style,&format!("{}:",episode.id),&plan,&finish_candidates,true)?
                            };
                            if issues.is_empty(){return Err("整编原拒绝没有可证实的适用范围问题；原回包保留，不重新整编".into());}
                            let mut batch=first.batch.clone();
                            batch.id=text(&request.input["executionBatch"],"id").to_string();
                            batch.package_ids=strings(&request.input["executionBatch"],"packageIds")?;
                            batch.source_refs=scope_refs.clone();
                            let scope=AcceptedAuthorScope{index:first.index,batch,input:request.input.clone(),draft:candidate};
                            repair_author_scope(&source,&original_response,&request,&scope,issues,&plan,&rows,&context,&current_assets,&selected,&episode.id,model,reasoning,input.get("voiceReference"),&validation_caps,style,frozen_finish_assets,executor).await
                        }
                    }
                }.await;
                match finish_result {
                    Ok(finished)=>{
                        completed_finishes.push((request.clone(),finished.clone(),scope_refs.clone()));
                        prior_finish=finished["units"].as_array().unwrap().last().cloned();prior_finish_index=Some(authored_scopes[end-1].index);
                        created.extend(finished["units"].as_array().unwrap().iter().cloned());realizations.extend(finished["fantasyRealization"].as_array().unwrap().iter().cloned());
                        revision_notes.extend(finished["revisionNotes"].as_array().unwrap().iter().cloned());unresolved.extend(finished["unresolved"].as_array().unwrap().iter().cloned());finished_refs.extend(scope_refs);
                    },
                    Err(reason)=>{let owners:Vec<_>=authored_scopes[start..end].iter().flat_map(|scope|scope.batch.package_ids.iter().cloned()).collect();local_blocked.push(json!({"episodeId":episode.id,"executionBatchId":request.input["executionBatch"]["id"],"packageIds":owners,"sourceRefs":scope_refs,"status":"blocked","reason":reason}));failed_finish_owners.extend(owners);prior_finish=None;prior_finish_index=None;}
                }
                start=end;
            }
            // First reproduce every original accepted Finish request and its
            // continuity byte-for-byte. Repairs are new nodes afterwards; an
            // altered tail cannot silently regenerate an already-paid editor.
            let original_episode_tail=created.last().cloned();
            if local_blocked.is_empty() && creative_enabled {
                created.clear();realizations.clear();revision_notes.clear();unresolved.clear();
                for (request,finished,refs) in &completed_finishes {
                    let repaired=repair_video_carrier(&source,finished.clone(),request,&plan,&current_assets,caps,refs,style,executor).await?;
                    created.extend(repaired["units"].as_array().unwrap().iter().cloned());
                    realizations.extend(repaired["fantasyRealization"].as_array().unwrap().iter().cloned());
                    revision_notes.extend(repaired["revisionNotes"].as_array().unwrap().iter().cloned());
                    unresolved.extend(repaired["unresolved"].as_array().unwrap().iter().cloned());
                    finish_provenance.push(json!({"version":"accepted-finish-domain/v1","executionBatchId":request.input["executionBatch"]["id"],"canonicalDomainRequestSha256":digest(&serde_json::to_string(request).map_err(|e|e.to_string())?),"draftSha256":finish_base_hash(&repaired),"carrierRepair":repaired["carrierRepair"]}));
                }
            } else {
                for (request,finished,_) in &completed_finishes {
                    finish_provenance.push(json!({"version":"accepted-finish-domain/v1","executionBatchId":request.input["executionBatch"]["id"],"canonicalDomainRequestSha256":digest(&serde_json::to_string(request).map_err(|e|e.to_string())?),"draftSha256":finish_base_hash(finished)}));
                }
            }
            if !created.is_empty() {
                validate_storyboard_scope(&source,&json!({"units":created}),&current_assets,caps,Some(&finished_refs))?;
            }
            Ok(json!({"draft":{"schemaVersion":"storyboard-draft/v2","units":created,"fantasyRealization":realizations,"revisionNotes":revision_notes,"unresolved":unresolved},"plan":plan,"localBlocked":local_blocked,"finishProvenance":finish_provenance,"originalEpisodeTail":original_episode_tail}))
        }.await;
        match result {
            Ok(done) if !done["pendingPlan"].is_null() => {
                plans.push(json!({"episodeId":episode.id,"plan":done["pendingPlan"]}));
                waiting.push(json!({"episodeId":episode.id,"selectionMode":"review","candidates":done["pendingPlan"]["fantasyCandidates"],"allowEmptySelection":true}));
                unavailable.insert(episode.id.clone());
            }
            Ok(done) => {
                let draft = &done["draft"];
                let complete = done["localBlocked"]
                    .as_array()
                    .is_some_and(|items| items.is_empty());
                if complete {
                    let delivery = json!({"schemaVersion":"complete-episode-delivery/v1","status":"running","scope":"episodes","sourceSha256":source.sha256,"episodeId":episode.id,"updatedEpisodeIds":[episode.id],"replacedUnitIds":[],"units":draft["units"],"plans":[{"episodeId":episode.id,"plan":done["plan"]}],"fantasyRealization":draft["fantasyRealization"],"revisionNotes":draft["revisionNotes"],"unresolved":draft["unresolved"],"finishProvenance":done["finishProvenance"],"blocked":[],"ruleSnapshots":storyboard_rules(input)?});
                    validate_episode_delivery(input, &delivery)?;
                    let mut publication_input = input.clone();
                    publication_input["previousBoundary"] = previous.clone();
                    // Publishing is program-owned and never enters a paid request/hash.
                    executor
                        .publish_episode(&delivery, &publication_input)
                        .await?;
                }
                if let Some(last) = draft["units"].as_array().unwrap().last() {
                    continuity_revision = if done["originalEpisodeTail"] != *last
                        || !continuity_revision.is_null()
                    {
                        let parent_revision = if continuity_revision.is_null() {
                            Value::Null
                        } else {
                            json!(digest(&continuity_revision.to_string()))
                        };
                        json!({"version":"accepted-tail-change/v1","reason":if done["originalEpisodeTail"]!=*last {"VIDEO_CARRIER_REPAIR"}else{"CONTINUITY_DEPENDENCY_UPDATED"},"sourceSha256":source.sha256,"previousEpisodeId":episode.id,"priorTailSha256":digest(&done["originalEpisodeTail"].to_string()),"currentTailSha256":digest(&last.to_string()),"parentRevisionSha256":parent_revision})
                    } else {
                        Value::Null
                    };
                    previous = last.clone();
                }
                all_units.extend(draft["units"].as_array().unwrap().iter().cloned());
                all_realizations.extend(
                    draft["fantasyRealization"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned(),
                );
                all_revisions.extend(draft["revisionNotes"].as_array().unwrap().iter().cloned());
                all_unresolved.extend(draft["unresolved"].as_array().unwrap().iter().cloned());
                if done["localBlocked"]
                    .as_array()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
                {
                    blocked.extend(done["localBlocked"].as_array().unwrap().iter().cloned());
                    unavailable.insert(episode.id.clone());
                } else {
                    updated_episodes.push(episode.id.clone());
                }
                plans.push(json!({"episodeId":episode.id,"plan":done["plan"]}));
            }
            Err(reason) => {
                blocked.push(json!({"episodeId":episode.id,"status":"blocked","reason":reason}));
                unavailable.insert(episode.id.clone());
            }
        }
    }
    Ok(
        json!({"schemaVersion":"storyboard-delivery/v1","sourceSha256":source.sha256,"status":if !waiting.is_empty(){"waiting_decision"}else if !blocked.is_empty(){"partially_blocked"}else{"completed"},"scope":"episodes","updatedEpisodeIds":updated_episodes,"replacedUnitIds":[],"units":all_units,"plans":plans,"waitingDecision":waiting.first(),"waitingDecisions":waiting,"fantasyRealization":all_realizations,"revisionNotes":all_revisions,"unresolved":all_unresolved,"blocked":blocked,"ruleSnapshots":storyboard_rules(input)?}),
    )
}

pub(super) fn episode_assets(assets: &[Value], episode_id: &str) -> Vec<Value> {
    let mut selected: Vec<Value> = assets
        .iter()
        .filter(|asset| {
            match asset
                .pointer("/applicability/episodes")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
            {
                Some(items) => items.iter().any(|item| item.as_str() == Some(episode_id)),
                None => asset
                    .get("applicability")
                    .and_then(Value::as_array)
                    .filter(|items| !items.is_empty())
                    .map(|items| {
                        items
                            .iter()
                            .any(|item| text(item, "episodeId") == episode_id)
                    })
                    .unwrap_or(true),
            }
        })
        .cloned()
        .collect();
    // Dependency references supply real media identity, not extra appearance.
    let mut cursor = 0;
    while cursor < selected.len() {
        let parent = resolve_asset_dependency(assets, &selected[cursor])
            .ok()
            .flatten();
        if let Some(parent) = parent {
            if !selected.iter().any(|a| text(a, "id") == parent) {
                if let Some(value) = assets.iter().find(|a| text(a, "id") == parent) {
                    let mut value = value.clone();
                    value["materialRole"] = json!("MATERIAL_SOURCE_ONLY");
                    selected.push(value);
                }
            }
        }
        cursor += 1;
    }
    // The validated scoped package is the sole interpretation transport. Raw
    // asset metadata must not reintroduce stale, unconfirmed or out-of-scope records.
    for asset in &mut selected {
        if let Some(map) = asset.as_object_mut() {
            map.remove("authorDecisions");
        }
    }
    selected
}

fn image_identity(asset: &Value, assets: &[Value]) -> Option<(String, String)> {
    if asset.pointer("/preparation/mode").and_then(Value::as_str) != Some("INLINE") {
        return None;
    }
    let mut current = asset;
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(text(current, "id")) {
            return None;
        }
        if current.pointer("/preparation/mode").and_then(Value::as_str) != Some("INLINE") {
            if current.pointer("/preparation/mode").and_then(Value::as_str) == Some("AUDIO") {
                return None;
            }
            return Some((
                text(current, "id").into(),
                text(current, "variantId").into(),
            ));
        }
        let explicit = resolve_asset_dependency(assets, current).ok().flatten();
        let from = current
            .pointer("/preparation/fromKey")
            .and_then(Value::as_str)
            .unwrap_or("");
        let candidates: Vec<_> = assets
            .iter()
            .filter(|other| {
                text(other, "id") != text(current, "id")
                    && (explicit.as_deref() == Some(text(other, "id"))
                        || !from.is_empty()
                            && [
                                text(other, "id"),
                                text(other, "key"),
                                text(other, "baseAssetId"),
                            ]
                            .contains(&from))
            })
            .collect();
        current = match candidates.as_slice() {
            [one] => one,
            [..] => {
                let baseline: Vec<_> = assets
                    .iter()
                    .filter(|other| {
                        text(other, "id") != text(current, "id")
                            && !text(current, "baseAssetId").is_empty()
                            && text(other, "baseAssetId") == text(current, "baseAssetId")
                            && other
                                .get("sourceVariantId")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .is_empty()
                            && other.pointer("/preparation/mode").and_then(Value::as_str)
                                != Some("INLINE")
                            && other.pointer("/preparation/mode").and_then(Value::as_str)
                                != Some("AUDIO")
                    })
                    .collect();
                if !candidates.is_empty() && candidates.len() != 1 {
                    return None;
                }
                match baseline.as_slice() {
                    [one] => one,
                    _ => return None,
                }
            }
        };
    }
}

fn materialize_asset_references(draft: &mut Value, assets: &[Value]) -> Result<(), String> {
    for unit in draft["units"].as_array_mut().ok_or("缺少分镜单元")? {
        let mut sound_sources = unit["soundSources"].as_array().cloned().unwrap_or_default();
        let owned = strings(unit, "sourceRefs")?;
        for sound in &sound_sources {
            if !assets.iter().any(|a| {
                text(a, "id") == text(sound, "assetId")
                    && text(a, "variantId") == text(sound, "variantId")
                    && a.pointer("/preparation/mode").and_then(Value::as_str) == Some("AUDIO")
            }) || strings(sound, "sourceRefs")?
                .iter()
                .any(|r| !owned.contains(r))
            {
                return Err("声音源身份或源范围失效".into());
            }
        }
        let mut references = Vec::new();
        for mut reference in unit["assetRefs"].as_array().ok_or("缺少资产引用")?.clone() {
            let asset = assets
                .iter()
                .find(|a| {
                    text(a, "id") == text(&reference, "assetId")
                        && text(a, "variantId") == text(&reference, "variantId")
                })
                .ok_or("分镜引用未知资产/形象")?;
            let mode = asset
                .pointer("/preparation/mode")
                .and_then(Value::as_str)
                .unwrap_or("");
            if mode == "AUDIO" {
                // A story voice identity is evidence, not an imported audio file.
                let refs = strings(&reference, "sourceRefs")?;
                if refs.is_empty() || refs.iter().any(|r| !owned.contains(r)) {
                    return Err("声音源引用越出本单元原文".into());
                }
                let sound = json!({"assetId":reference["assetId"],"variantId":reference["variantId"],"sourceRefs":reference["sourceRefs"],"materialStatus":"SOURCE_ONLY"});
                if !sound_sources.contains(&sound) {
                    sound_sources.push(sound);
                }
                continue;
            }
            let map = reference.as_object_mut().ok_or("资产引用不是对象")?;
            map.remove("imageAssetId");
            map.remove("imageVariantId");
            if mode == "INLINE" {
                if let Some((id, variant)) = image_identity(asset, assets) {
                    map.insert("imageAssetId".into(), json!(id));
                    map.insert("imageVariantId".into(), json!(variant));
                } else {
                    map.insert("imageBindingRequired".into(), json!(true));
                }
            }
            reference["order"] = json!(references.len() + 1);
            references.push(reference);
        }
        unit["assetRefs"] = json!(references);
        unit["soundSources"] = json!(sound_sources);
    }
    Ok(())
}

pub(super) fn scoped_facts(input: &Value, rows: &[SourceRow], context: &[SourceRow]) -> Vec<Value> {
    input
        .pointer("/foundation/facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|fact| {
            fact["sourceRefs"]
                .as_array()
                .map(|refs| {
                    refs.iter()
                        .filter_map(Value::as_str)
                        .any(|id| rows.iter().chain(context).any(|row| row.id == id))
                })
                .unwrap_or(false)
        })
        .map(|fact| {
            let mut fact = fact.clone();
            if let Some(map) = fact.as_object_mut() {
                map.remove("quotes");
            }
            fact
        })
        .collect()
}

pub(super) fn methods_for(
    rows: &[SourceRow],
    selected: &[Value],
    scenario_keys: &[String],
    finish: bool,
    entry_facts: &[Value],
) -> Result<Vec<&'static str>, String> {
    let mut methods = vec![
        "storyboard-common",
        "acting-blocking",
        "camera-emotion",
        "lighting-space",
        "sound-rhythm",
    ];
    let cue=regex::Regex::new(r"(?i)(?:Q|Ｑ)(?:版|化|形体)|梦境|梦中|梦里|主观(?:画面|镜头|空间)|(?:幻想|想象)(?:画面|空间)|小化身|(?:内心|心理|主观)小人|(?:二维|2D)[^。！？!?；;\n]{0,24}(?:小人|化身)").unwrap();
    if !selected.is_empty()
        || rows.iter().any(|row| cue.is_match(&row.text))
        || entry_facts
            .iter()
            .any(|fact| cue.is_match(text(fact, "statement")))
    {
        methods.push("fantasy-expression");
    }
    for key in scenario_keys {
        let name = original::scenario_rule(key).ok_or_else(|| format!("方法卡不存在：{key}"))?;
        if !methods.contains(&name) {
            methods.push(name);
        }
    }
    if finish {
        for key in ["editing-integrity", "camera-path-repair"] {
            let name = original::scenario_rule(key).unwrap();
            if !methods.contains(&name) {
                methods.push(name);
            }
        }
    }
    Ok(methods)
}

pub(super) fn validate_authored(
    source: &LockedSource,
    draft: &mut Value,
    assets: &[Value],
    caps: &Value,
    refs: &[String],
    style: &str,
    prefix: &str,
) -> Result<(), String> {
    if text(draft, "schemaVersion") != "storyboard-draft/v2" {
        return Err("新分镜创作须返回storyboard-draft/v2原话区间协议".into());
    }
    materialize_dialogue(source, draft, false)?;
    materialize_asset_references(draft, assets)?;
    prefix_ids(draft, prefix)?;
    add_delivery_fields(draft, style)?;
    validate_storyboard_scope(source, draft, assets, caps, Some(refs))?;
    materialize_unit_coverage(source, draft)
}

fn author_scope_candidate(
    source: &LockedSource,
    raw: &Value,
    assets: &[Value],
    caps: &Value,
    refs: &[String],
    style: &str,
    prefix: &str,
    plan: &Value,
    selected: &[Value],
    creative_enabled: bool,
) -> Result<(Value, Vec<Value>), String> {
    scope_repair_candidate(
        source,
        raw,
        assets,
        caps,
        refs,
        style,
        prefix,
        plan,
        selected,
        creative_enabled,
        false,
    )
}

/// A rejected model result is data for the original editor, never an accepted
/// author. Source coordinates remain locked; only an explicitly known local
/// validation defect may reach the existing two-attempt delta route.
fn author_validation_candidate(
    source: &LockedSource,
    raw: &Value,
    assets: &[Value],
    caps: &Value,
    refs: &[String],
    style: &str,
    prefix: &str,
    plan: &Value,
    selected: &[Value],
    reason: &str,
) -> Result<(Value, Vec<Value>), String> {
    if reason == crate::text_executor::AUTHOR_SHOT_DURATION_REJECTION {
        return author_duration_candidate(
            source, raw, assets, caps, refs, style, prefix, plan, selected,
        );
    }
    if reason == crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION {
        return author_scope_candidate(
            source, raw, assets, caps, refs, style, prefix, plan, selected, true,
        );
    }
    let missing_dialogue = crate::text_executor::author_missing_dialogue(reason);
    let code = match reason {
        crate::text_executor::AUTHOR_DIALOGUE_INTERVAL_REJECTION => "DIALOGUE_SOURCE_INTERVAL",
        crate::text_executor::AUTHOR_SOURCE_COVERAGE_REJECTION => "UNIT_SOURCE_COVERAGE",
        crate::text_executor::AUTHOR_ASSET_SOURCE_REJECTION => "ASSET_SOURCE_SCOPE",
        _ if missing_dialogue.is_some() => "DIALOGUE_SOURCE_COVERAGE",
        _ => return Err("该原主创错误没有明确的原整编修订入口；保留原回包".into()),
    };
    if raw["schemaVersion"] != "storyboard-draft/v2" {
        return Err("原主创修订缺少已知原话区间协议".into());
    }
    if missing_dialogue.is_some() {
        let mut checked = raw.clone();
        match validate_authored(source, &mut checked, assets, caps, refs, style, prefix) {
            Err(found) if found == reason => {}
            _ => return Err("原主创对白缺失与锁定源校验不一致；未创建修订".into()),
        }
    }
    let allowed: HashSet<_> = refs.iter().map(String::as_str).collect();
    let known: HashSet<_> = source.rows.iter().map(|row| row.id.as_str()).collect();
    let quotes: HashMap<_, _> = source
        .rows
        .iter()
        .filter_map(|row| {
            row.dialogue
                .as_ref()
                .map(|quote| (quote.id.as_str(), quote))
        })
        .collect();
    let missing_row = if let Some(line) = missing_dialogue {
        Some(
            source
                .rows
                .iter()
                .find(|row| {
                    row.dialogue.as_ref().is_some_and(|quote| quote.id == line)
                        && allowed.contains(row.id.as_str())
                })
                .ok_or("缺失对白不属于冻结主创正文；未扩大修订")?,
        )
    } else {
        None
    };
    let mut draft = raw.clone();
    materialize_asset_references(&mut draft, assets)?;
    prefix_ids(&mut draft, prefix)?;
    validate_event_bindings(&draft, plan)?;
    validate_fantasy_profile(&draft, selected, true)?;
    let mut issues = Vec::new();
    let units = draft["units"]
        .as_array()
        .filter(|units| !units.is_empty())
        .ok_or("原主创候选缺少完整单元")?;
    for unit in units {
        let unit_refs = strings(unit, "sourceRefs")?;
        refs_valid(&unit_refs, &allowed, true)?;
        let mut dialogue_issues = Vec::new();
        for (shot_index, shot) in unit["shots"]
            .as_array()
            .filter(|shots| !shots.is_empty())
            .ok_or("原主创候选缺少完整镜表")?
            .iter()
            .enumerate()
        {
            refs_valid(&strings(shot, "sourceRefs")?, &allowed, true)?;
            refs_valid(&strings(shot, "contextRefs")?, &known, false)?;
            let duration = shot["duration"]
                .as_f64()
                .filter(|number| number.is_finite() && *number > 0.0)
                .ok_or("原主创候选镜长无效")?;
            for (part_index, part) in shot["dialogueParts"]
                .as_array()
                .ok_or("原主创候选缺少对白范围")?
                .iter()
                .enumerate()
            {
                let line = text(part, "lineId");
                let quote = quotes.get(line).ok_or("对白引用未知源句")?;
                let owner = source
                    .rows
                    .iter()
                    .find(|row| row.dialogue.as_ref().is_some_and(|q| q.id == line))
                    .ok_or("对白引用未知源句")?;
                if !allowed.contains(owner.id.as_str()) {
                    return Err("对白引用越出冻结主创范围".into());
                }
                let start = part["quoteStart"]
                    .as_u64()
                    .ok_or("原主创修订缺少完整UTF16区间")? as usize;
                let end = part["quoteEnd"]
                    .as_u64()
                    .ok_or("原主创修订缺少完整UTF16区间")? as usize;
                if start >= end || end > source.text.encode_utf16().count() {
                    return Err("原主创区间越出锁定原文，未交给模型猜补".into());
                }
                let voice_start = part["voiceStart"]
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or("原主创声窗无效")?;
                let voice_end = part["voiceEnd"]
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or("原主创声窗无效")?;
                if voice_start < 0.0 || voice_end <= voice_start || voice_end > duration + 0.000001
                {
                    return Err("声窗越出镜长或为空，禁止机械裁窗过关".into());
                }
                if part.get("speaker").is_some() && text(part, "speaker") != quote.speaker
                    || part.get("channel").is_some() && text(part, "channel") != quote.channel
                {
                    return Err("传入说者或声道不符合原话来源".into());
                }
                if part.get("text").is_some()
                    && spoken_ranges(quote)
                        .iter()
                        .any(|range| start >= range.start && end <= range.end)
                    && text(part, "text") != slice_utf16(&source.text, &SourceRange { start, end })?
                {
                    return Err("传入对白与原话区间不一致，禁止改词或把演法念出".into());
                }
                if !spoken_ranges(quote)
                    .iter()
                    .any(|range| start >= range.start && end <= range.end)
                {
                    dialogue_issues.push(json!({"code":code,"unitId":unit["id"],"coordinate":format!("{}.shots[{shot_index}].dialogueParts[{part_index}]",text(unit,"id")),"lineId":line,"quoteStart":start,"quoteEnd":end,"allowedSpokenRanges":spoken_ranges(quote),"sourceRefs":[owner.id],"problem":reason}));
                }
            }
        }
        for reference in unit["assetRefs"]
            .as_array()
            .ok_or("原主创缺少资产引用数组")?
        {
            refs_valid(&strings(reference, "sourceRefs")?, &allowed, true)?;
        }
        if code == "DIALOGUE_SOURCE_INTERVAL" {
            issues.extend(dialogue_issues);
        } else if let Some(row) = missing_row {
            if unit_refs.contains(&row.id) {
                issues.push(json!({"code":code,"unitId":unit["id"],"coordinate":format!("{}.shots.dialogueParts",text(unit,"id")),"lineId":row.dialogue.as_ref().unwrap().id,"sourceRefs":[row.id],"allowedSpokenRanges":spoken_ranges(row.dialogue.as_ref().unwrap()),"problem":reason}));
            }
        } else if code == "UNIT_SOURCE_COVERAGE" {
            issues.push(json!({"code":code,"unitId":unit["id"],"coordinate":format!("{}.sourceRefs",text(unit,"id")),"sourceRefs":unit["sourceRefs"],"requiredSourceRefs":refs,"problem":reason}));
        } else if unit["assetRefs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| {
                strings(reference, "sourceRefs")
                    .unwrap()
                    .iter()
                    .any(|id| !unit_refs.contains(id))
            })
        {
            issues.push(json!({"code":code,"unitId":unit["id"],"coordinate":format!("{}.assetRefs",text(unit,"id")),"sourceRefs":unit["sourceRefs"],"problem":reason}));
        }
    }
    if issues.is_empty() {
        return Err("原主创拒绝没有可定位的本范围问题；不重新生成".into());
    }
    if code != "DIALOGUE_SOURCE_INTERVAL" {
        materialize_dialogue(source, &mut draft, false)?;
    }
    add_delivery_fields(&mut draft, style)?;
    Ok((draft, issues))
}

fn scope_repair_candidate(
    source: &LockedSource,
    raw: &Value,
    assets: &[Value],
    caps: &Value,
    refs: &[String],
    style: &str,
    prefix: &str,
    plan: &Value,
    selected: &[Value],
    creative_enabled: bool,
    defer_scene_coordinate: bool,
) -> Result<(Value, Vec<Value>), String> {
    if text(raw, "schemaVersion") != "storyboard-draft/v2" {
        return Err("新分镜创作须返回storyboard-draft/v2原话区间协议".into());
    }
    let mut draft = raw.clone();
    materialize_dialogue(source, &mut draft, false)?;
    materialize_asset_references(&mut draft, assets)?;
    prefix_ids(&mut draft, prefix)?;
    add_delivery_fields(&mut draft, style)?;
    let mut issues = Vec::new();
    validate_storyboard_repair_scope(
        source,
        &draft,
        assets,
        caps,
        Some(refs),
        Some(&mut issues),
        defer_scene_coordinate,
        false,
    )?;
    validate_event_bindings_for_repair(&draft, plan, Some(&issues))?;
    validate_fantasy_profile(&draft, selected, creative_enabled)?;
    materialize_unit_coverage(source, &mut draft)?;
    // These exports are data only. This candidate is never an adopted author.
    creative::add_delivery(&mut draft, source, style, assets, creative_enabled)?;
    Ok((draft, issues))
}

fn author_duration_candidate(
    source: &LockedSource,
    raw: &Value,
    assets: &[Value],
    caps: &Value,
    refs: &[String],
    style: &str,
    prefix: &str,
    plan: &Value,
    selected: &[Value],
) -> Result<(Value, Vec<Value>), String> {
    if raw["schemaVersion"] != "storyboard-draft/v2" {
        return Err("镜长修订缺少原话区间协议；未调用模型".into());
    }
    let mut draft = raw.clone();
    materialize_dialogue(source, &mut draft, false)?;
    materialize_asset_references(&mut draft, assets)?;
    prefix_ids(&mut draft, prefix)?;
    add_delivery_fields(&mut draft, style)?;
    let mut issues = Vec::new();
    validate_storyboard_repair_scope(
        source,
        &draft,
        assets,
        caps,
        Some(refs),
        Some(&mut issues),
        false,
        true,
    )?;
    if !issues.iter().any(|issue| issue["code"] == "SHOT_DURATION") {
        return Err("原镜长拒绝没有可定位的零秒无对白镜；未扩大修订".into());
    }
    validate_event_bindings(&draft, plan)?;
    validate_fantasy_profile(&draft, selected, true)?;
    materialize_unit_coverage(source, &mut draft)?;
    creative::add_delivery(&mut draft, source, style, assets, true)?;
    Ok((draft, issues))
}

fn author_scope_problem_signature(issues: &[Value]) -> String {
    let mut keys: Vec<_> = issues.iter().map(|issue| {
        let mut refs:Vec<_>=issue["sourceRefs"].as_array().into_iter().flatten().cloned().collect();
        refs.sort_by_key(Value::to_string);refs.dedup();
        let mut identity=json!({"code":issue["code"],"assetId":issue["assetId"],"variantId":issue["variantId"],"episodeId":issue["episodeId"],"sceneId":issue["sceneId"],"sourceRefs":refs});
        if issue["code"]=="UNIT_SCENE_COORDINATE" {identity["expectedSceneId"]=issue["expectedSceneId"].clone();}
        if ["DIALOGUE_SOURCE_INTERVAL","DIALOGUE_SOURCE_COVERAGE","UNIT_SOURCE_COVERAGE","ASSET_SOURCE_SCOPE","SHOT_DURATION"].contains(&text(issue,"code")) {
            for field in ["coordinate","lineId","quoteStart","quoteEnd","allowedSpokenRanges","requiredSourceRefs","shotId","originalDuration"] {identity[field]=issue[field].clone();}
        }
        identity.to_string()
    }).collect::<HashSet<_>>().into_iter().collect();
    keys.sort();
    digest(&json!(keys).to_string())
}

fn author_scope_repair_units(base: &Value, issues: &[Value]) -> Result<HashSet<String>, String> {
    let units = base["units"]
        .as_array()
        .ok_or("主创适用范围修订缺少原单元")?;
    let mut permitted: HashSet<_> = issues
        .iter()
        .map(|issue| text(issue, "unitId").to_string())
        .collect();
    if permitted.is_empty()
        || permitted
            .iter()
            .any(|id| !units.iter().any(|unit| text(unit, "id") == id))
    {
        return Err("主创适用范围修订没有可定位的原单元".into());
    }
    loop {
        let before = permitted.len();
        // A wrong scene coordinate can require repacking the same event's
        // adjacent units. They are a causal group, even when the original row
        // was split into disjoint intervals. Other events remain untouched.
        let coordinate_events: HashSet<_> = issues
            .iter()
            .filter(|issue| issue["code"] == "UNIT_SCENE_COORDINATE")
            .filter_map(|issue| units.iter().find(|unit| unit["id"] == issue["unitId"]))
            .map(|unit| text(unit, "eventId"))
            .filter(|id| !id.is_empty())
            .collect();
        for unit in units {
            if coordinate_events.contains(text(unit, "eventId")) {
                permitted.insert(text(unit, "id").into());
            }
        }
        let refs: HashSet<_> = units
            .iter()
            .filter(|unit| permitted.contains(text(unit, "id")))
            .flat_map(|unit| strings(unit, "sourceRefs").unwrap_or_default())
            .collect();
        for unit in units {
            if strings(unit, "sourceRefs")?
                .iter()
                .any(|id| refs.contains(id))
            {
                permitted.insert(text(unit, "id").into());
            }
        }
        for record in base["fantasyRealization"].as_array().into_iter().flatten() {
            let ids = strings(record, "unitIds")?;
            if ids.iter().any(|id| permitted.contains(id)) {
                let positions: Vec<_> = units
                    .iter()
                    .enumerate()
                    .filter(|(_, unit)| ids.iter().any(|id| id == text(unit, "id")))
                    .map(|(index, _)| index)
                    .collect();
                if let (Some(first), Some(last)) = (positions.first(), positions.last()) {
                    permitted.extend(
                        units[*first..=*last]
                            .iter()
                            .map(|unit| text(unit, "id").to_string()),
                    );
                }
            }
        }
        if permitted.len() == before {
            break;
        }
    }
    Ok(permitted)
}

/// Equivalent to the original coordinator's unresolved-author -> finish(delta)
/// route. The author remains rejected; only the fully validated Finisher result
/// becomes a usable scope and supplies the next author's real continuity.
async fn repair_author_scope(
    source: &LockedSource,
    original_raw: &Value,
    origin: &TextRequest,
    scope: &AcceptedAuthorScope,
    mut issues: Vec<Value>,
    plan: &Value,
    rows: &[SourceRow],
    context: &[SourceRow],
    assets: &[Value],
    selected: &[Value],
    episode_id: &str,
    model: &str,
    reasoning: &str,
    voice_reference: Option<&Value>,
    caps: &Value,
    style: &str,
    frozen_assets_input: &Value,
    executor: &dyn TextExecutor,
) -> Result<Value, String> {
    let finish_origin = origin.stage == "storyboardFinish";
    if (!finish_origin && origin.stage != "storyboardCreate")
        || (finish_origin && origin.input["executionBatch"]["version"] != "related-range-finish/v1")
    {
        return Err("适用范围修订原岗或原执行范围不成立；保留原拒绝，未调用模型".into());
    }
    let descriptor_field = if finish_origin {
        "finishRepair"
    } else {
        "authorRepair"
    };
    let raw_field = if finish_origin {
        "originalFinishRawSha256"
    } else {
        "originalAuthorRawSha256"
    };
    let error_prefix = if finish_origin {
        "FINISH_SCOPE_REPAIR"
    } else {
        "AUTHOR_SCOPE_REPAIR"
    };
    let origin_hash = digest(&serde_json::to_string(origin).map_err(|e| e.to_string())?);
    let raw_hash = digest(&original_raw.to_string());
    let mut current = scope.clone();
    let mut lineage = Vec::new();
    let source_repair = issues.iter().any(|issue| {
        [
            "DIALOGUE_SOURCE_INTERVAL",
            "DIALOGUE_SOURCE_COVERAGE",
            "UNIT_SOURCE_COVERAGE",
            "ASSET_SOURCE_SCOPE",
            "SHOT_DURATION",
        ]
        .contains(&text(issue, "code"))
    });
    for attempt in 1..=2 {
        let signature = author_scope_problem_signature(&issues);
        let prepared = finish_request(
            &[current.clone()],
            plan,
            rows,
            context,
            assets,
            selected,
            episode_id,
            model,
            reasoning,
            voice_reference,
            None,
            if finish_origin {
                origin
                    .input
                    .get("nextBoundary")
                    .filter(|value| !value.is_null())
            } else {
                None
            },
        )?;
        let prepared = bind_scope_repair_request(
            prepared, &source.sha256, original_raw, origin, scope, &issues, attempt,
        )?;
        let (request, base, refs) =
            prepare_finish_assets(prepared, frozen_assets_input, assets, executor).await?;
        let identity = digest(&request.input[descriptor_field].to_string());
        let actual_characters = executor.input_transport_characters(&request)?;
        if actual_characters > FINISH_TRANSPORT_LIMIT {
            return Err(format!("{error_prefix}_TRANSPORT_CAPACITY：actual={actual_characters} UTF16, limit={FINISH_TRANSPORT_LIMIT}；原回包、来源与方法卡完整保留，未提交新请求"));
        }
        let raw = executor.execute(request.clone()).await?;
        let permitted = author_scope_repair_units(&base, &issues)?;
        let validated: Result<(Value, Vec<Value>), String> = (|| {
            for group in raw["replacements"]
                .as_array()
                .ok_or("主创适用范围修订缺少差量替换组")?
            {
                if strings(group, "replaces")?
                    .iter()
                    .any(|id| !permitted.contains(id))
                {
                    return Err(
                        "主创适用范围修订越出问题单元及原文/幻想关联组；不扩大重做范围".into(),
                    );
                }
            }
            let mut draft = apply_finish_delta(&base, &raw)?;
            let preserved_units: HashSet<_> = base["units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|unit| text(unit, "id").to_string())
                .collect();
            let preserved_shots: HashSet<_> = base["units"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|unit| unit["shots"].as_array().unwrap())
                .map(|shot| text(shot, "id").to_string())
                .collect();
            prefix_ids_preserving(
                &mut draft,
                &format!(
                    "{episode_id}:{}-domain-repair:{identity}:",
                    if finish_origin { "finish" } else { "author" }
                ),
                &preserved_units,
                &preserved_shots,
            )?;
            let selected = request.input["selectedFantasy"]
                .as_array()
                .ok_or("主创修订缺少冻结幻想选择")?;
            let (next, remaining) = if source_repair {
                let mut checked = draft.clone();
                match validate_authored(
                    source,
                    &mut checked,
                    assets,
                    caps,
                    &refs,
                    style,
                    &format!("{episode_id}:"),
                ) {
                    Ok(()) => {
                        validate_event_bindings(&checked, plan)?;
                        validate_fantasy_profile(&checked, selected, true)?;
                        creative::add_delivery(&mut checked, source, style, assets, true)?;
                        (checked, Vec::new())
                    }
                    Err(reason) if crate::text_executor::author_repairable_rejection(&reason) => {
                        author_validation_candidate(
                            source,
                            &draft,
                            assets,
                            caps,
                            &refs,
                            style,
                            &format!("{episode_id}:"),
                            plan,
                            selected,
                            &reason,
                        )?
                    }
                    Err(reason) => return Err(reason),
                }
            } else {
                scope_repair_candidate(
                    source,
                    &draft,
                    assets,
                    caps,
                    &refs,
                    style,
                    &format!("{episode_id}:"),
                    plan,
                    selected,
                    true,
                    true,
                )?
            };
            if remaining.is_empty() {
                // A hard failure is rejected with this original raw too; it
                // cannot escape the bounded repair as an unvalidated receipt.
                validate_storyboard_scope(source, &next, assets, caps, Some(&refs))?;
                validate_event_bindings(&next, plan)?;
                validate_fantasy_profile(&next, selected, true)?;
            }
            Ok((next, remaining))
        })();
        match validated {
            Ok((mut next, remaining)) if remaining.is_empty() => {
                lineage.push(json!({"attempt":attempt,"canonicalDomainRequestSha256":digest(&serde_json::to_string(&request).map_err(|e|e.to_string())?),"baseDraftHash":finish_base_hash(&base),"draftSha256":finish_base_hash(&next)}));
                let mut proof = json!({"version":if finish_origin{"accepted-finish-scope-repair/v1"}else{"accepted-author-scope-repair/v1"},"sourceSha256":source.sha256,"originDomainRequestSha256":origin_hash,"nodes":lineage});
                proof[raw_field] = json!(raw_hash);
                proof[if finish_origin {
                    "originalFinishAccepted"
                } else {
                    "originalAuthorAccepted"
                }] = json!(false);
                next[if finish_origin {
                    "finishRepairProvenance"
                } else {
                    "authorRepairProvenance"
                }] = proof;
                let mut accepted = raw.clone();
                accepted["materializedDraft"] = next.clone();
                executor.accept(&request, &accepted).await?;
                return Ok(next);
            }
            Ok((next, remaining)) => {
                executor
                    .reject(
                        &request,
                        &raw,
                        if remaining
                            .iter()
                            .any(|issue| issue["code"] == "UNIT_SCENE_COORDINATE")
                        {
                            "单元来源跨集/跨场或包含非制作资料"
                        } else if source_repair {
                            remaining
                                .iter()
                                .find_map(|issue| {
                                    issue["problem"].as_str().filter(|reason| {
                                        crate::text_executor::author_repairable_rejection(reason)
                                    })
                                })
                                .ok_or("原文坐标修订缺少准确问题")?
                        } else {
                            crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION
                        },
                    )
                    .await?;
                if author_scope_problem_signature(&remaining) == signature {
                    return Err(format!("{error_prefix}_NO_PROGRESS：原整编修订后相同问题仍存在；保留原岗和修订回包，不重复提交"));
                }
                if attempt == 2 {
                    return Err(format!("{error_prefix}_LIMIT：原整编两次修订仍未通过领域校验；不重跑主创或放宽范围"));
                }
                lineage.push(json!({"attempt":attempt,"canonicalDomainRequestSha256":digest(&serde_json::to_string(&request).map_err(|e|e.to_string())?),"result":"rejected-scope","baseDraftHash":finish_base_hash(&base),"draftSha256":finish_base_hash(&next)}));
                current.draft = next;
                issues = remaining;
            }
            Err(reason) => {
                executor.reject(&request, &raw, &reason).await?;
                return Err(reason);
            }
        }
    }
    Err(format!(
        "{error_prefix}_LIMIT：保留原岗与修订回包，不追加调用"
    ))
}

/// Whole-episode publication proof, independently checked again at the store
/// boundary. Evidence from one author/transport fragment cannot satisfy it.
pub(crate) fn validate_episode_delivery(input: &Value, output: &Value) -> Result<(), String> {
    if output["schemaVersion"] != "complete-episode-delivery/v1"
        || output["scope"] != "episodes"
        || output["status"] != "running"
        || output["blocked"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
    {
        return Err("逐集发布只接受完整已校验整编成果".into());
    }
    let source: LockedSource =
        serde_json::from_value(input["source"].clone()).map_err(|e| e.to_string())?;
    let source = upgrade_source_dialogues(&source)?;
    validate_source(&source)?;
    let episode = text(output, "episodeId");
    if output["sourceSha256"] != source.sha256
        || !source.episodes.iter().any(|e| e.id == episode)
        || output["updatedEpisodeIds"] != json!([episode])
        || output["finishProvenance"]
            .as_array()
            .is_none_or(|entries| entries.is_empty())
    {
        return Err("完整单集来源或整编验收身份缺失".into());
    }
    let units = output["units"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or("完整集没有单元")?;
    if units.iter().any(|u| text(u, "episodeId") != episode) {
        return Err("逐集发布包含其他集或部分关联范围".into());
    }
    let refs: Vec<_> = source
        .rows
        .iter()
        .filter(|row| row.kind == "body" && row.episode_id.as_deref() == Some(episode))
        .map(|row| row.id.clone())
        .collect();
    let assets = input["assets"]
        .as_array()
        .ok_or("逐集发布缺少冻结资产目录")?;
    validate_storyboard_scope(&source, output, assets, &input["capabilities"], Some(&refs))?;
    if creative::uses_profile(input)?
        && !carrier_problems(output, &input["capabilities"]).is_empty()
    {
        return Err("完整集尚未通过视频载体修订，不发布为可生产成果".into());
    }
    Ok(())
}

pub(super) fn carrier_problems(draft: &Value, caps: &Value) -> Vec<Value> {
    let minimum = caps["minDuration"].as_f64().unwrap_or(0.0);
    let maximum = caps["maxDuration"].as_f64().unwrap_or(f64::INFINITY);
    draft["units"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|unit| {
            let seconds = unit["duration"].as_f64().unwrap_or(f64::NAN);
            let integer = crate::binding::normalized_integer_duration(seconds);
            let mut codes = vec![];
            if integer.is_none() {
                codes.push("DURATION_REVISION_REQUIRED");
            }
            if seconds + 1e-6 < minimum {
                codes.push("BELOW_CHANNEL_MINIMUM");
            }
            if seconds > maximum + 1e-6 {
                codes.push("ABOVE_CHANNEL_MAXIMUM");
            }
            if let Some(allowed) = caps["allowedDurations"]
                .as_array()
                .filter(|items| !items.is_empty())
            {
                if !allowed
                    .iter()
                    .filter_map(Value::as_f64)
                    .any(|value| (value - seconds).abs() <= 1e-6)
                {
                    codes.push("DURATION_NOT_ALLOWED");
                }
            }
            (!codes.is_empty()).then(
                || json!({"unitId":unit["id"],"actualDuration":unit["duration"],"codes":codes}),
            )
        })
        .collect()
}

fn validate_carrier_capabilities(caps: &Value) -> Result<(), String> {
    let minimum = caps["minDuration"].as_f64().unwrap_or(0.0);
    let maximum = caps["maxDuration"].as_f64().ok_or("载体最大时长未知")?;
    if let Some(allowed) = caps["allowedDurations"]
        .as_array()
        .filter(|items| !items.is_empty())
    {
        if allowed.iter().any(|value| {
            value
                .as_f64()
                .and_then(crate::binding::normalized_integer_duration)
                .is_none()
        }) || !allowed
            .iter()
            .filter_map(Value::as_f64)
            .any(|seconds| seconds + 1e-6 >= minimum && seconds <= maximum + 1e-6)
        {
            return Err("载体可用时长档位无效或没有当前范围内的正整秒档位，未调用模型".into());
        }
    } else if (minimum - 1e-6).ceil().max(1.0) > (maximum + 1e-6).floor() {
        return Err("渠道当前时长范围没有正整秒载体，未调用模型".into());
    }
    Ok(())
}

fn carrier_problem_signature(draft: &Value, problems: &[Value]) -> String {
    let values:Vec<_>=problems.iter().map(|problem| {
        let unit=draft["units"].as_array().into_iter().flatten().find(|u|u["id"]==problem["unitId"]).unwrap();
        json!({"sourceRefs":unit["sourceRefs"],"sceneId":unit["sceneId"],"actualDuration":problem["actualDuration"],"codes":problem["codes"]})
    }).collect();
    digest(&json!(values).to_string())
}

/// Separate source/creative validity from a reparable carrier limit. This is
/// used only before the same Finisher's bounded repair; publication still uses
/// the exact original channel limits. No source/quote/window checks are relaxed.
fn carrier_validation_caps(draft: &Value, caps: &Value, enabled: bool) -> Value {
    let mut value = caps.clone();
    if enabled {
        let highest = draft["units"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|unit| {
                unit["shots"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|shot| shot["duration"].as_f64())
                    .sum::<f64>()
            })
            .fold(caps["maxDuration"].as_f64().unwrap_or(0.0), f64::max);
        if highest.is_finite() {
            value["maxDuration"] = json!(highest);
        }
    }
    value
}

/// A new Finisher node repairs carrier problems in an accepted creative draft.
/// It never rewrites the accepted origin request or launches Planner/Author.
async fn repair_video_carrier(
    source: &LockedSource,
    mut draft: Value,
    origin: &TextRequest,
    plan: &Value,
    assets: &[Value],
    caps: &Value,
    refs: &[String],
    style: &str,
    executor: &dyn TextExecutor,
) -> Result<Value, String> {
    let initial = carrier_problems(&draft, caps);
    if initial.is_empty() {
        return Ok(draft);
    }
    let origin_hash = digest(&serde_json::to_string(origin).map_err(|e| e.to_string())?);
    let initial_hash = finish_base_hash(&draft);
    let mut seen = HashSet::new();
    let mut lineage = Vec::new();
    let mut previous_validation_error: Option<String> = None;
    for attempt in 1..=2 {
        let problems = carrier_problems(&draft, caps);
        if problems.is_empty() {
            break;
        }
        let signature = carrier_problem_signature(&draft, &problems);
        if previous_validation_error.is_none() && !seen.insert(signature.clone()) {
            return Err("VIDEO_CARRIER_REPAIR_NO_PROGRESS：原整编修订后相同载体问题仍存在；已保留原接受稿和修订回包，不重复提交".into());
        }
        let rows: Vec<SourceRow> = serde_json::from_value(origin.input["sourceRows"].clone())
            .map_err(|e| e.to_string())?;
        let context: Vec<SourceRow> = serde_json::from_value(origin.input["contextRows"].clone())
            .map_err(|e| e.to_string())?;
        let events: Vec<_> = plan["events"]
            .as_array()
            .ok_or("载体修订缺少原事件规划")?
            .iter()
            .map(|event| text(event, "id").to_string())
            .collect();
        let base = draft.clone();
        let base_hash = finish_base_hash(&base);
        let mut request = origin.clone();
        request.input[creative::CARRIER_PROFILE_FIELD] = json!(creative::CARRIER_PROFILE);
        request.input["previousDraft"] = base.clone();
        request.input["baseDraftHash"] = json!(base_hash);
        request.input["finishMode"] = json!("delta");
        request.input["carrierProblems"] = json!(problems);
        request.input["carrierRepair"] = json!({"version":"accepted-finisher-carrier-repair/v1","attempt":attempt,"maximumAttempts":2,"originDomainRequestSha256":origin_hash,"originExecutionBatch":origin.input["executionBatch"],"originalAcceptedDraftSha256":initial_hash,"baseDraftHash":base_hash,"problemSignature":signature});
        if let Some(error) = &previous_validation_error {
            request.input["carrierRepair"]["previousValidationError"] = json!(error);
        }
        let identity = digest(&request.input["carrierRepair"].to_string());
        request.input["executionBatch"] = json!({"version":"carrier-repair/v1","id":format!("carrier-repair:{identity}"),"sourceRefs":refs,"packageIds":origin.input["executionBatch"]["packageIds"],"originExecutionBatchId":origin.input["executionBatch"]["id"]});
        request.input["capacityBrief"] = original::capacity_brief(caps, Some(&base), None, &rows);
        request.schema = finish_delta_schema(
            &draft_schema(
                &rows,
                &context,
                assets,
                &events,
                text(&origin.input, "episodeId"),
            ),
            &base,
        );
        // prepare_request retains the original primary/method rules. The new
        // trusted carrier appendix is tied only to this explicit new node.
        request = creative::prepare_request(request)?;
        if executor.input_transport_characters(&request)? > FINISH_TRANSPORT_LIMIT {
            return Err("VIDEO_CARRIER_REPAIR_TRANSPORT_CAPACITY：原稿及方法卡完整保留，修订请求仍超过容量；未提交新请求".into());
        }
        let raw = executor.execute(request.clone()).await?;
        let validated = (|| {
            let affected: HashSet<_> = problems
                .iter()
                .filter_map(|p| p["unitId"].as_str())
                .collect();
            let old_units = base["units"].as_array().unwrap();
            let mut permitted: HashSet<String> =
                affected.iter().map(|id| (*id).to_string()).collect();
            // A selected fantasy is an indivisible edit group. Fill only its
            // real contiguous owner span, never a name-based guessed relation.
            loop {
                let before = permitted.len();
                for record in base["fantasyRealization"].as_array().into_iter().flatten() {
                    let ids = strings(record, "unitIds")?;
                    if ids.iter().any(|id| permitted.contains(id)) {
                        let positions: Vec<_> = old_units
                            .iter()
                            .enumerate()
                            .filter(|(_, u)| ids.iter().any(|id| id == text(u, "id")))
                            .map(|(i, _)| i)
                            .collect();
                        if let (Some(first), Some(last)) = (positions.first(), positions.last()) {
                            permitted.extend(
                                old_units[*first..=*last]
                                    .iter()
                                    .map(|u| text(u, "id").to_string()),
                            );
                        }
                    }
                }
                if permitted.len() == before {
                    break;
                }
            }
            for problem in &problems {
                if problem["codes"]
                    .as_array()
                    .is_some_and(|codes| codes.iter().any(|code| code == "BELOW_CHANNEL_MINIMUM"))
                {
                    let index = old_units
                        .iter()
                        .position(|u| u["id"] == problem["unitId"])
                        .unwrap();
                    for neighbour in [
                        index.checked_sub(1),
                        index.checked_add(1).filter(|i| *i < old_units.len()),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        permitted.insert(text(&old_units[neighbour], "id").to_string());
                    }
                }
            }
            for group in raw["replacements"]
                .as_array()
                .ok_or("载体修订缺少差量替换组")?
            {
                let replaces = strings(group, "replaces")?;
                if !replaces.iter().any(|id| affected.contains(id.as_str()))
                    || replaces.iter().any(|id| !permitted.contains(id))
                {
                    return Err(
                        "载体修订不得额外改写无问题的独立单元；必要相邻单元须纳入受影响替换组"
                            .into(),
                    );
                }
            }
            let mut next = apply_finish_delta(&base, &raw)?;
            if next["units"][0]["initialState"] != base["units"][0]["initialState"]
                || next["units"].as_array().unwrap().last().unwrap()["exitState"]
                    != base["units"].as_array().unwrap().last().unwrap()["exitState"]
            {
                return Err("载体修订不得改写本范围已确认初末态；前后稿与缓存保留".into());
            }
            let preserved_units: HashSet<_> = base["units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| text(u, "id").to_string())
                .collect();
            let preserved_shots: HashSet<_> = base["units"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|u| u["shots"].as_array().unwrap())
                .map(|s| text(s, "id").to_string())
                .collect();
            prefix_ids_preserving(
                &mut next,
                &format!(
                    "{}:carrier-repair:{identity}:",
                    text(&origin.input, "episodeId")
                ),
                &preserved_units,
                &preserved_shots,
            )?;
            let validation_caps = carrier_validation_caps(&next, caps, true);
            validate_authored(
                source,
                &mut next,
                assets,
                &validation_caps,
                refs,
                style,
                &format!("{}:", text(&origin.input, "episodeId")),
            )?;
            validate_event_bindings(&next, plan)?;
            validate_fantasy_profile(
                &next,
                origin.input["selectedFantasy"]
                    .as_array()
                    .ok_or("载体修订缺少原幻想选择")?,
                true,
            )?;
            creative::add_delivery(&mut next, source, style, assets, true)?;
            mark_short_endings(&mut next, caps["minDuration"].as_f64().unwrap_or(0.0));
            Ok::<Value, String>(next)
        })();
        match validated {
            Ok(next) => {
                let mut accepted = raw.clone();
                accepted["materializedDraft"] = next.clone();
                accepted["carrierRepair"] = request.input["carrierRepair"].clone();
                executor.accept(&request, &accepted).await?;
                lineage.push(json!({"canonicalDomainRequestSha256":digest(&serde_json::to_string(&request).map_err(|e|e.to_string())?),"baseDraftHash":base_hash,"resultDraftSha256":finish_base_hash(&next),"attempt":attempt}));
                draft = next;
            }
            Err(reason) => {
                executor.reject(&request, &raw, &reason).await?;
                if attempt == 2 {
                    return Err(format!(
                        "{}：{}",
                        if previous_validation_error.as_deref() == Some(reason.as_str()) {
                            "VIDEO_CARRIER_REPAIR_NO_PROGRESS"
                        } else {
                            "VIDEO_CARRIER_REPAIR_LIMIT"
                        },
                        reason
                    ));
                }
                previous_validation_error = Some(reason);
                continue;
            }
        }
        let remaining = carrier_problems(&draft, caps);
        if !remaining.is_empty() && seen.contains(&carrier_problem_signature(&draft, &remaining)) {
            return Err("VIDEO_CARRIER_REPAIR_NO_PROGRESS：同一载体问题没有进展；已保留原接受稿和修订回包，禁止自动重发".into());
        }
    }
    if !carrier_problems(&draft, caps).is_empty() {
        return Err(
            "VIDEO_CARRIER_REPAIR_LIMIT：原整编两次修订仍未满足载体，不自动增加岗位或重做主创"
                .into(),
        );
    }
    draft["carrierRepair"] = json!({"version":"accepted-finisher-carrier-repair/v1","originDomainRequestSha256":origin_hash,"originalAcceptedDraftSha256":initial_hash,"nodes":lineage});
    Ok(draft)
}

fn mark_short_endings(draft: &mut Value, minimum: f64) {
    for unit in draft["units"].as_array_mut().unwrap() {
        let mut issues: Vec<_> = unit["capacityIssues"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|issue| {
                !["BELOW_CHANNEL_MINIMUM", "DURATION_REVISION_REQUIRED"]
                    .contains(&text(issue, "code"))
            })
            .cloned()
            .collect();
        if unit["duration"].as_f64().unwrap() < minimum {
            issues.push(json!({"code":"BELOW_CHANNEL_MINIMUM","message":"源范围自然短收；当前渠道最小时长未满足，请调整渠道或合法单元组合，不新增空白或源外剧情。","minimum":minimum}));
        }
        if crate::binding::normalized_integer_duration(unit["duration"].as_f64().unwrap()).is_none()
        {
            issues.push(json!({"code":"DURATION_REVISION_REQUIRED","message":"实际镜长合计为非整秒；首版视频载体需要正整秒。原声窗与镜表保留，请明确修订，不能自动取整、填空或裁剪。","actualDuration":unit["duration"]}));
        }
        if !issues.is_empty() {
            unit["capacityIssues"] = json!(issues);
        } else {
            unit.as_object_mut().unwrap().remove("capacityIssues");
        }
    }
}

fn storyboard_rules(input: &Value) -> Result<Vec<RuleSnapshot>, String> {
    if super::stage3_pack::uses_profile(input)? {
        return super::stage3_pack::rule_snapshots();
    }
    Ok(rule_snapshots()
        .into_iter()
        .filter(|rule| {
            [
                "episode-emotion-planner",
                "unit-performance-director",
                "episode-finishing-editor",
                "acting-blocking",
                "camera-emotion",
                "lighting-space",
                "sound-rhythm",
                "fantasy-expression",
            ]
            .contains(&rule.id.as_str())
                || rule.id.starts_with("storyboard-")
        })
        .collect())
}

/// The engine binds this material to the exact adopted producer chain. The
/// pipeline independently checks source content, current unit versions and
/// canonical hashes; it never picks a latest plan or joins assets by name.
pub(super) fn accepted_revision_context(
    source: &LockedSource,
    input: &Value,
    episode_id: &str,
    episode_units: &[Value],
) -> Result<Value, String> {
    let contexts = input["revisionContexts"]
        .as_array()
        .ok_or("局部重做缺少已采用规划来源")?;
    let matching: Vec<_> = contexts
        .iter()
        .filter(|context| text(context, "episodeId") == episode_id)
        .collect();
    if matching.len() != 1 {
        return Err("局部重做的已采用规划来源缺失或不唯一；未重新调用Planner".into());
    }
    let context = matching[0];
    if text(context, "version") != "accepted-storyboard-revision/v1"
        || text(context, "sourceSha256") != source.sha256
        || ["originWorkflowId", "planJobId", "planRequestHash"]
            .iter()
            .any(|key| text(context, key).is_empty())
    {
        return Err("局部重做来源契约或原文身份已失效".into());
    }
    let bindings = context["unitBindings"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or("局部重做缺少已采用单元版本绑定")?;
    let mut bound_ids = HashSet::new();
    let mut bound_units = Vec::new();
    for binding in bindings {
        let unit_id = text(binding, "unitId");
        let unit = episode_units
            .iter()
            .find(|unit| text(unit, "id") == unit_id)
            .ok_or("局部重做的来源单元不在当前集成果中")?;
        if !bound_ids.insert(unit_id)
            || binding["unitVersion"].as_u64().is_none()
            || binding["unitVersion"] != unit["version"]
            || text(binding, "unitSha256") != digest(&unit.to_string())
        {
            return Err("局部重做来源单元的版本/正文发生变化或绑定重复".into());
        }
        bound_units.push(unit.clone());
    }
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|row| row.kind == "body" && row.episode_id.as_deref() == Some(episode_id))
        .cloned()
        .collect();
    let context_rows: Vec<_> = source
        .rows
        .iter()
        .filter(|row| !rows.iter().any(|owned| owned.id == row.id))
        .cloned()
        .collect();
    let scenes: Vec<_> = source
        .scenes
        .iter()
        .filter(|scene| scene.episode_id == episode_id)
        .cloned()
        .collect();
    let plan = &context["plan"];
    validate_plan(plan, &rows, &context_rows, &scenes)?;
    let fingerprints = plan["sourceFingerprints"]
        .as_object()
        .ok_or("已采用规划缺少冻结原行指纹")?;
    if rows
        .iter()
        .any(|row| fingerprints.get(&row.id) != Some(&json!(digest(&row.text))))
        || fingerprints.iter().any(|(id, hash)| {
            source
                .rows
                .iter()
                .find(|row| &row.id == id)
                .is_none_or(|row| hash != &json!(digest(&row.text)))
        })
    {
        return Err("局部重做的已采用规划原行指纹与锁定原文不一致".into());
    }
    let selected = context["selectedFantasy"]
        .as_array()
        .ok_or("局部重做缺少原已选幻想清单")?;
    let mut selected_ids = HashSet::new();
    for candidate in selected {
        let id = text(candidate, "id");
        let original = plan["fantasyCandidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| text(candidate, "id") == id)
            .ok_or("原已选幻想不属于该已采用规划")?;
        let production_card = creative::selected_candidates(vec![original.clone()], true)
            .pop()
            .unwrap();
        if !selected_ids.insert(id) || candidate != original && candidate != &production_card {
            return Err("局部重做的幻想身份/源锚/制作目的与原已选项不一致".into());
        }
    }
    let frozen_fantasy =
        json!({"units":bound_units,"fantasyRealization":context["fantasyRealization"]});
    if context["fantasyRealization"]
        .as_array()
        .ok_or("局部重做缺少已采用幻想记录")?
        .iter()
        .any(|record| record.get("status").is_some())
    {
        creative::validate_fantasy_records(&frozen_fantasy, selected)?;
    } else {
        validate_fantasy(&frozen_fantasy, selected)?;
    }
    Ok(context.clone())
}

fn revision_plan_scope(context: &Value, refs: &[String]) -> Result<Value, String> {
    let full = &context["plan"];
    let mut owners = Vec::new();
    for owner in full["workPackages"]
        .as_array()
        .ok_or("已采用规划缺少工作包")?
    {
        let owned = strings(owner, "sourceRefs")?;
        let current: Vec<_> = owned
            .iter()
            .filter(|id| refs.contains(id))
            .cloned()
            .collect();
        if current.is_empty() {
            continue;
        }
        let mut scoped = owner.clone();
        scoped["sourceRefs"] = json!(current);
        let mut context_refs = strings(owner, "contextRefs")?;
        for id in owned.iter().filter(|id| !refs.contains(id)) {
            if !context_refs.contains(id) {
                context_refs.push(id.clone());
            }
        }
        scoped["contextRefs"] = json!(context_refs);
        if let Some(beats) = owner["beatContracts"].as_array() {
            let mut relevant = Vec::new();
            for beat in beats {
                let beat_refs: Vec<_> = strings(beat, "sourceRefs")?
                    .into_iter()
                    .filter(|id| refs.contains(id))
                    .collect();
                if !beat_refs.is_empty() {
                    let mut beat = beat.clone();
                    beat["sourceRefs"] = json!(beat_refs);
                    relevant.push(beat);
                }
            }
            scoped["beatContracts"] = json!(relevant);
        }
        owners.push(scoped);
    }
    let event_ids: HashSet<_> = owners.iter().map(|owner| text(owner, "eventId")).collect();
    let owner_ids: HashSet<_> = owners.iter().map(|owner| text(owner, "id")).collect();
    let fact_ids: HashSet<_> = owners
        .iter()
        .flat_map(|owner| {
            owner["entryFactIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .collect();
    let mut events = Vec::new();
    for event in full["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event_ids.contains(text(event, "id")))
    {
        let mut event = event.clone();
        event["sourceRefs"] = json!(strings(&event, "sourceRefs")?
            .into_iter()
            .filter(|id| refs.contains(id))
            .collect::<Vec<_>>());
        events.push(event);
    }
    let projected = json!({"schemaVersion":"storyboard-plan/v2","emotionArc":full["emotionArc"],"unitPackingPolicy":full["unitPackingPolicy"],"events":events,"workPackages":owners,
        "entryFacts":full["entryFacts"].as_array().unwrap().iter().filter(|fact|fact_ids.contains(text(fact,"id"))).cloned().collect::<Vec<_>>(),
        "provisionalUnits":full["provisionalUnits"].as_array().into_iter().flatten().filter(|unit|owner_ids.contains(text(unit,"packageId"))&&unit["sourceRefs"].as_array().is_some_and(|ids|ids.iter().filter_map(Value::as_str).any(|id|refs.iter().any(|owned|owned==id)))).cloned().collect::<Vec<_>>()});
    if owners.is_empty() {
        return Err("局部重做未找到实际消费的原工作包".into());
    }
    Ok(projected)
}

async fn run_storyboard_revision(
    source: &LockedSource,
    input: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    let creative_enabled = creative::uses_profile(input)?;
    let mac_pack = super::stage3_pack::uses_profile(input)?;
    if mac_pack {
        if !creative_enabled {
            return Err("Mac生产规则适配缺少锁定原话与导出合同，未派发请求".into());
        }
        let mut bound = json!({});
        super::stage3_pack::attach_profile(&mut bound, input)?;
    }
    let revision_enabled = creative::uses_revision_profile(input)?;
    let selected = strings(&input["selection"], "unitIds")?;
    let selected_ids: HashSet<_> = selected.iter().map(String::as_str).collect();
    if selected_ids.len() != selected.len() {
        return Err("重做单元不能重复选择".into());
    }
    let existing = input["existingUnits"]
        .as_array()
        .ok_or("局部重做缺少冻结的原单元")?;
    if selected
        .iter()
        .any(|id| !existing.iter().any(|unit| text(unit, "id") == id))
    {
        return Err("所选重做单元不在冻结版本中".into());
    }
    let instruction = text(input, "revisionInstruction").trim();
    if instruction.is_empty() {
        return Err("请写明本次单元重做要求".into());
    }
    let caps = input.get("capabilities").ok_or("缺少冻结的分镜制作规格")?;
    let (minimum, _, _) = capabilities(input)?;
    let assets = input["assets"].as_array().ok_or("缺少资产身份目录")?;
    let settings = creative::effective_settings(&input["settings"], creative_enabled);
    let style = text(&settings, "visualStyle");
    let direction = production_direction(input);
    let mut units = Vec::new();
    let mut replaced = Vec::new();
    let mut updated = Vec::new();
    let mut blocked = Vec::new();
    let mut revisions = Vec::new();
    let mut unresolved = Vec::new();
    let mut realized = Vec::new();
    let mut impacts = Vec::new();
    let mut unavailable = HashSet::new();
    let mut accepted_boundaries: HashMap<String, Value> = HashMap::new();
    for (episode_position, episode) in source.episodes.iter().enumerate() {
        let mut episode_units: Vec<_> = existing
            .iter()
            .filter(|unit| text(unit, "episodeId") == episode.id)
            .cloned()
            .collect();
        episode_units.sort_by_key(|unit| unit_source_position(source, unit));
        let target_positions: Vec<_> = episode_units
            .iter()
            .enumerate()
            .filter(|(_, unit)| selected_ids.contains(text(unit, "id")))
            .map(|(index, _)| index)
            .collect();
        if target_positions.is_empty() {
            continue;
        }
        let accepted_context = if revision_enabled {
            match accepted_revision_context(source, input, &episode.id, &episode_units) {
                Ok(context) => Some(context),
                Err(reason) => {
                    blocked.push(json!({"episodeId":episode.id,"code":"REVISION_PROVENANCE_INVALID","reason":reason}));
                    unavailable.insert(episode.id.clone());
                    continue;
                }
            }
        } else {
            None
        };
        let prior_id = episode_position
            .checked_sub(1)
            .map(|index| source.episodes[index].id.as_str());
        if prior_id.map(|id| unavailable.contains(id)).unwrap_or(false) {
            blocked.push(json!({"episodeId":episode.id,"status":"waiting_dependency","dependsOn":prior_id,"reason":"连续前集重做尚未完成，先处理前置再继续。"}));
            unavailable.insert(episode.id.clone());
            continue;
        }
        // Include intervening units and all units sharing a split source line.
        // The published replacement set is explicit; no unrelated episode runs.
        let mut first = *target_positions.first().unwrap();
        let mut last = *target_positions.last().unwrap();
        loop {
            let refs: HashSet<_> = episode_units[first..=last]
                .iter()
                .flat_map(|unit| strings(unit, "sourceRefs").unwrap_or_default())
                .collect();
            let mut shared: Vec<_> = episode_units
                .iter()
                .enumerate()
                .filter(|(_, unit)| {
                    strings(unit, "sourceRefs")
                        .unwrap_or_default()
                        .iter()
                        .any(|id| refs.contains(id))
                })
                .map(|(index, _)| index)
                .collect();
            if let Some(context) = &accepted_context {
                let scope_ids: HashSet<_> = episode_units[first..=last]
                    .iter()
                    .map(|unit| text(unit, "id"))
                    .collect();
                for record in context["fantasyRealization"].as_array().unwrap() {
                    let related = strings(record, "unitIds")?;
                    if related.iter().any(|id| scope_ids.contains(id.as_str())) {
                        shared.extend(
                            episode_units
                                .iter()
                                .enumerate()
                                .filter(|(_, unit)| related.iter().any(|id| id == text(unit, "id")))
                                .map(|(index, _)| index),
                        );
                    }
                }
            }
            let next_first = shared.iter().copied().min().unwrap_or(first).min(first);
            let next_last = shared.iter().copied().max().unwrap_or(last).max(last);
            if (first, last) == (next_first, next_last) {
                break;
            }
            first = next_first;
            last = next_last;
        }
        let originals = &episode_units[first..=last];
        if accepted_context.as_ref().is_some_and(|context| {
            originals.iter().any(|unit| {
                !context["unitBindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|binding| binding["unitId"] == unit["id"])
            })
        }) {
            blocked.push(json!({"episodeId":episode.id,"code":"REVISION_PROVENANCE_INVALID","reason":"关联范围包含未绑定的当前采用稿；保留原稿，未调用Planner或主创。"}));
            continue;
        }
        let target_refs: HashSet<_> = originals
            .iter()
            .flat_map(|unit| strings(unit, "sourceRefs").unwrap_or_default())
            .collect();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| {
                row.kind == "body"
                    && row.episode_id.as_ref() == Some(&episode.id)
                    && target_refs.contains(&row.id)
            })
            .cloned()
            .collect();
        let refs: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
        let accepted_plan = accepted_context
            .as_ref()
            .map(|context| revision_plan_scope(context, &refs))
            .transpose()?;
        let mut current_assets = episode_assets(assets, &episode.id);
        asset_cards::inherit_identity_aliases(input, &mut current_assets)?;
        let model_assets = asset_cards::project_for_episode(input, &current_assets, &episode.id)?;
        let scoped_decisions = scoped_author_decisions(source, input, &rows)?;
        let base_context = relevant_context(
            source,
            episode,
            &current_assets,
            input.get("foundation").unwrap_or(&Value::Null),
        );
        let mut context = decision_context(source, &rows, &base_context, scoped_decisions.as_ref());
        if let Some(plan) = &accepted_plan {
            let mut ids: HashSet<_> = context.iter().map(|row| row.id.clone()).collect();
            for owner in plan["workPackages"].as_array().unwrap() {
                ids.extend(strings(owner, "contextRefs")?);
            }
            for fact in plan["entryFacts"].as_array().unwrap() {
                ids.extend(strings(fact, "sourceRefs")?);
            }
            context = source
                .rows
                .iter()
                .filter(|row| !refs.contains(&row.id) && ids.contains(&row.id))
                .cloned()
                .collect();
        }
        let facts = scoped_facts(input, &rows, &context);
        let before = first
            .checked_sub(1)
            .map(|index| &episode_units[index])
            .or_else(|| prior_id.and_then(|id| accepted_boundaries.get(id)))
            .or_else(|| input.get("previousBoundary"));
        let after = episode_units.get(last + 1);
        let fantasy_source = accepted_context
            .as_ref()
            .map(|context| &context["selectedFantasy"])
            .unwrap_or(&input["selectedFantasy"]);
        let candidates: Vec<_> =
            fantasy_source
                .as_array()
                .into_iter()
                .flatten()
                .filter(|candidate| {
                    strings(candidate, "sourceRefs")
                        .map(|ids| ids.iter().any(|id| target_refs.contains(id)))
                        .unwrap_or(false)
                        || accepted_context.as_ref().is_some_and(|context| {
                            context["fantasyRealization"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|record| {
                                    text(record, "candidateId") == text(candidate, "id")
                                        && record["unitIds"].as_array().into_iter().flatten().any(
                                            |id| originals.iter().any(|unit| unit["id"] == *id),
                                        )
                                })
                        })
                })
                .cloned()
                .collect();
        let candidates = if revision_enabled {
            candidates
                .into_iter()
                .map(|candidate| {
                    if candidate.get("suggestedProposal").is_some() {
                        candidate
                    } else {
                        creative::selected_candidates(vec![candidate], true)
                            .pop()
                            .unwrap()
                    }
                })
                .collect::<Vec<_>>()
        } else {
            candidates
        };
        let scenario_keys: Vec<_> = accepted_plan
            .as_ref()
            .map(|plan| {
                plan["workPackages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|owner| {
                        owner["scenarioKeys"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let methods = methods_for(&rows, &candidates, &scenario_keys, false, &facts)?;
        let mut events = Vec::new();
        for scene in source
            .scenes
            .iter()
            .filter(|scene| scene.episode_id == episode.id)
        {
            let scene_refs: Vec<_> = rows
                .iter()
                .filter(|row| row.scene_id.as_ref() == Some(&scene.id))
                .map(|row| row.id.clone())
                .collect();
            if !scene_refs.is_empty() {
                events.push(json!({"id":format!("revision:{}",scene.id),"sceneId":scene.id,"sourceRefs":scene_refs}));
            }
        }
        let plan = accepted_plan.unwrap_or_else(|| json!({"events":events}));
        let event_ids: Vec<_> = plan["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| text(event, "id").to_string())
            .collect();
        let identity = digest(&json!({"source":source.sha256,"originals":originals,"instruction":instruction,"caps":caps,"direction":direction,"settings":settings}).to_string());
        let prefix = format!("{}:revision:{}:", episode.id, &identity[..12]);
        let result: Result<Value,String> = async {
            if refs.is_empty() || target_refs.len()!=refs.len() { return Err("原单元来源已失效，请按当前原文重新创作整集".into()); }
            let mut author_input=json!({"mode":"revision","episodeId":episode.id,"productionDirection":direction,"sourceRows":rows,"contextRows":context,"facts":facts,"assets":model_assets,"events":plan["events"],"revisionTargetUnits":originals,"revisionInstruction":instruction,"acceptedContinuity":before,"nextBoundary":after,"selectedFantasy":candidates,"settings":settings,"capabilities":caps,"instruction":"仅重做本源范围，保留完整源事实、原话和前后连续状态。可拆合；对白仅选lineId与绝对UTF16原话区间，不发声演法。"});
            if mac_pack {
                let (cards, selection)=asset_cards::project_for_scope(input,&current_assets,&episode.id,&rows,&context)?;
                author_input["assets"]=json!(cards);
                author_input["assetSelection"]=selection;
            }
            asset_cards::attach_catalog_scope(&mut author_input, input, &episode.id);
            creative::mark_input(&mut author_input, creative_enabled);
            creative::attach_carrier_profile(&mut author_input, input)?;
            super::stage3_pack::attach_profile(&mut author_input, input)?;
            if let Some(context) = &accepted_context {
                author_input[creative::REVISION_PROFILE_FIELD]=json!(creative::REVISION_PROFILE);
                author_input["plan"]=plan.clone();
                author_input["workPackages"]=plan["workPackages"].clone();
                author_input["workPackage"]=plan["workPackages"][0].clone();
                author_input["entryFacts"]=plan["entryFacts"].clone();
                author_input["revisionProvenance"]=json!({"version":context["version"],"sourceSha256":context["sourceSha256"],"originWorkflowId":context["originWorkflowId"],"planJobId":context["planJobId"],"planRequestHash":context["planRequestHash"],"unitBindings":context["unitBindings"]});
                author_input["capacityBrief"]=original::capacity_brief(caps,Some(&json!({"units":originals})),input.get("voiceReference"),&rows);
            }
            let revision_provenance=author_input["revisionProvenance"].clone();
            attach_author_decisions(&mut author_input,scoped_decisions.as_ref());
            let authored = call_validated(executor,"storyboardCreate","unit-performance-director",model,reasoning,
                author_input,
                draft_schema(&rows,&context,&current_assets,&event_ids,&episode.id),&methods,|value| {
                    validate_authored(source,value,&current_assets,caps,&refs,style,&format!("{prefix}create:"))?;
                    validate_event_bindings(value,&plan)?;
                    validate_fantasy_profile(value,&candidates,creative_enabled)?;
                    creative::add_delivery(value,source,style,&current_assets,creative_enabled)
                }).await?;
            let finish_methods=methods_for(&rows,&candidates,&scenario_keys,true,&facts)?;
            let mut finish_input=json!({"mode":"revision","episodeId":episode.id,"productionDirection":direction,"sourceRows":rows,"contextRows":context,"facts":facts,"assets":model_assets,"plan":plan,"draft":authored,"revisionTargetUnits":originals,"revisionInstruction":instruction,"previousBoundary":before,"nextBoundary":after,"selectedFantasy":candidates,"settings":settings,"capabilities":caps,"instruction":"直接修好本关联范围；不改写无关单元。输出完整范围，按新切点选择原话区间并重算声窗。"});
            asset_cards::attach_catalog_scope(&mut finish_input, input, &episode.id);
            creative::mark_input(&mut finish_input, creative_enabled);
            creative::attach_carrier_profile(&mut finish_input, input)?;
            super::stage3_pack::attach_profile(&mut finish_input, input)?;
            if accepted_context.is_some() {
                finish_input[creative::REVISION_PROFILE_FIELD]=json!(creative::REVISION_PROFILE);
                finish_input["workPackages"]=plan["workPackages"].clone();
                finish_input["entryFacts"]=plan["entryFacts"].clone();
                finish_input["revisionProvenance"]=revision_provenance;
                finish_input["acceptedContinuity"]=json!(before);
                finish_input["capacityBrief"]=original::capacity_brief(caps,Some(&authored),input.get("voiceReference"),&rows);
            }
            attach_author_decisions(&mut finish_input,scoped_decisions.as_ref());
            let finish_schema=draft_schema(&rows,&context,&current_assets,&event_ids,&episode.id);
            let finish_request=creative::prepare_request(TextRequest{stage:"storyboardFinish".into(),skill:"episode-finishing-editor".into(),model:model.into(),reasoning:reasoning.into(),input:finish_input.clone(),schema:finish_schema.clone(),rule_snapshot:snapshot("episode-finishing-editor",&finish_methods)?})?;
            let finished = call_validated(executor,"storyboardFinish","episode-finishing-editor",model,reasoning,
                finish_input,
                finish_schema,&finish_methods,|value| {
                    original::pack_event_units(value,&plan,caps["maxDuration"].as_f64().unwrap())?;
                    let validation_caps=carrier_validation_caps(value,caps,creative_enabled);
                    validate_authored(source,value,&current_assets,&validation_caps,&refs,style,&prefix)?;
                    validate_event_bindings(value,&plan)?;
                    validate_fantasy_profile(value,&candidates,creative_enabled)?;
                    preserve_revision_ids(value,originals)?;
                    creative::add_delivery(value,source,style,&current_assets,creative_enabled)?;
                    let mut combined=episode_units[..first].to_vec();
                    combined.extend(value["units"].as_array().unwrap().iter().cloned());
                    combined.extend(episode_units[last+1..].iter().cloned());
                    validate_storyboard(source,&json!({"units":combined}),assets,&validation_caps)
                        .map_err(|reason|format!("局部重做与本集未改部分不一致：{reason}"))?;
                    mark_short_endings(value,minimum);
                    Ok(())
                }).await?;
            let finished=if creative_enabled {repair_video_carrier(source,finished,&finish_request,&plan,&current_assets,caps,&refs,style,executor).await?}else{finished};
            let mut combined=episode_units[..first].to_vec();combined.extend(finished["units"].as_array().unwrap().iter().cloned());combined.extend(episode_units[last+1..].iter().cloned());
            validate_storyboard(source,&json!({"units":combined}),assets,caps).map_err(|reason|format!("载体修订与未改单元不一致：{reason}"))?;
            Ok(finished)
        }.await;
        match result {
            Ok(draft) => {
                let new_last = draft["units"].as_array().unwrap().last().unwrap();
                accepted_boundaries.insert(
                    episode.id.clone(),
                    if last + 1 == episode_units.len() {
                        new_last.clone()
                    } else {
                        episode_units.last().unwrap().clone()
                    },
                );
                if let Some(next) = after {
                    if text(originals.last().unwrap(), "exitState") != text(new_last, "exitState") {
                        impacts.push(json!({"episodeId":episode.id,"fromUnitId":new_last["id"],"nextUnitId":next["id"],"reason":"重做范围末态变化，后继初态需重新核对。"}));
                    }
                }
                units.extend(draft["units"].as_array().unwrap().iter().cloned());
                replaced.extend(originals.iter().map(|unit| text(unit, "id").to_string()));
                updated.push(episode.id.clone());
                revisions.extend(draft["revisionNotes"].as_array().unwrap().iter().cloned());
                unresolved.extend(draft["unresolved"].as_array().unwrap().iter().cloned());
                realized.extend(
                    draft["fantasyRealization"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned(),
                );
            }
            Err(reason) => {
                unavailable.insert(episode.id.clone());
                blocked.push(json!({"episodeId":episode.id,"unitIds":originals.iter().map(|unit|text(unit,"id")).collect::<Vec<_>>(),"reason":reason}));
            }
        }
    }
    Ok(
        json!({"schemaVersion":"storyboard-delivery/v1","sourceSha256":source.sha256,"scope":"units","status":if blocked.is_empty(){"completed"}else{"partially_blocked"},"updatedEpisodeIds":updated,"replacedUnitIds":replaced,"units":units,"plans":[],"waitingDecision":null,"waitingDecisions":[],"continuityImpacts":impacts,"fantasyRealization":realized,"revisionNotes":revisions,"unresolved":unresolved,"blocked":blocked,"ruleSnapshots":storyboard_rules(input)?}),
    )
}

pub(super) fn unit_source_position(source: &LockedSource, unit: &Value) -> usize {
    unit["sourceCoverage"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|range| range["start"].as_u64().map(|start| start as usize))
        .min()
        .or_else(|| {
            strings(unit, "sourceRefs").ok().and_then(|refs| {
                source
                    .rows
                    .iter()
                    .filter(|row| refs.contains(&row.id))
                    .map(|row| row.range.start)
                    .min()
            })
        })
        .unwrap_or(usize::MAX)
}

fn preserve_revision_ids(draft: &mut Value, originals: &[Value]) -> Result<(), String> {
    let units = draft["units"].as_array_mut().ok_or("重做缺少单元")?;
    if units.len() != originals.len() {
        return Ok(());
    }
    let mut ids = HashMap::new();
    for (unit, original) in units.iter_mut().zip(originals) {
        ids.insert(
            text(unit, "id").to_string(),
            text(original, "id").to_string(),
        );
        unit["id"] = original["id"].clone();
    }
    for realization in draft["fantasyRealization"]
        .as_array_mut()
        .ok_or("缺少幻想记录")?
    {
        for id in realization["unitIds"]
            .as_array_mut()
            .ok_or("幻想单元引用无效")?
        {
            if let Some(next) = ids.get(id.as_str().ok_or("幻想单元id无效")?) {
                *id = json!(next);
            }
        }
    }
    Ok(())
}

fn materialize_unit_coverage(source: &LockedSource, draft: &mut Value) -> Result<(), String> {
    materialize_dialogue(source, draft, true)?;
    let units = draft["units"].as_array().ok_or("缺少单元")?;
    let cited: HashSet<_> = units
        .iter()
        .flat_map(|unit| unit["shots"].as_array().into_iter().flatten())
        .flat_map(|shot| strings(shot, "sourceRefs").unwrap_or_default())
        .collect();
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|row| cited.contains(&row.id))
        .collect();
    let mut spoken_owners: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for (index, unit) in units.iter().enumerate() {
        for shot in unit["shots"].as_array().ok_or("缺少镜表")? {
            for part in shot["dialogueParts"].as_array().ok_or("缺少对白")? {
                let end = part["quoteEnd"].as_u64().ok_or("对白覆盖区间缺失")? as usize;
                let owners = spoken_owners
                    .entry(text(part, "lineId").to_owned())
                    .or_default();
                if let Some(last) = owners.last_mut().filter(|(owner, _)| *owner == index) {
                    last.1 = end;
                } else {
                    owners.push((index, end));
                }
            }
        }
    }
    let mut coverage = vec![Vec::<Value>::new(); units.len()];
    let mut previous_owner = 0;
    for (position, row) in rows.iter().enumerate() {
        let spoken_quote = match row.dialogue.as_ref() {
            Some(quote) if !expected_spoken(source, quote)?.is_empty() => Some(quote),
            _ => None,
        };
        if let Some(quote) = spoken_quote {
            let owners = spoken_owners
                .get(&quote.id)
                .filter(|owners| !owners.is_empty())
                .ok_or("对白源行没有实际发声片段，不能以引用补足")?;
            let mut start = row.range.start;
            let spoken_end = spoken_ranges(quote)
                .last()
                .map(|range| range.end)
                .unwrap_or(quote.range.end);
            for &(owner, last) in owners {
                if owner < previous_owner {
                    return Err("正文覆盖归属与原话执行顺序不一致".into());
                }
                let end = if last == spoken_end {
                    row.range.end
                } else {
                    last
                };
                if start >= end {
                    return Err("正文覆盖区间空或重复".into());
                }
                coverage[owner]
                    .push(json!({"rowId":row.id,"start":start,"end":end,"dialogueId":quote.id}));
                start = end;
                previous_owner = owner;
            }
            if start != row.range.end {
                return Err("对白源行没有完整覆盖，不能以引用补足".into());
            }
        } else {
            let upper = rows[position + 1..]
                .iter()
                .filter_map(|next| next.dialogue.as_ref())
                .find_map(|quote| {
                    spoken_owners
                        .get(&quote.id)
                        .and_then(|owners| owners.first())
                        .map(|(owner, _)| *owner)
                })
                .unwrap_or(units.len().saturating_sub(1));
            let owner = units
                .iter()
                .enumerate()
                .find_map(|(index, unit)| {
                    (index >= previous_owner
                        && index <= upper
                        && unit["shots"].as_array().unwrap().iter().any(|shot| {
                            strings(shot, "sourceRefs")
                                .unwrap_or_default()
                                .iter()
                                .any(|id| id == &row.id)
                        }))
                    .then_some(index)
                })
                .ok_or("正文动作的实际镜头引用不能按原文顺序归属，需原整编处理")?;
            coverage[owner]
                .push(json!({"rowId":row.id,"start":row.range.start,"end":row.range.end}));
            previous_owner = owner;
        }
    }
    for (unit, ranges) in draft["units"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(coverage)
    {
        unit["sourceCoverage"] = json!(ranges);
    }
    Ok(())
}

fn prefix_ids(draft: &mut Value, prefix: &str) -> Result<(), String> {
    prefix_ids_preserving(draft, prefix, &HashSet::new(), &HashSet::new())
}

fn prefix_ids_preserving(
    draft: &mut Value,
    prefix: &str,
    preserved_units: &HashSet<String>,
    preserved_shots: &HashSet<String>,
) -> Result<(), String> {
    let mut units_map = HashMap::new();
    let mut unit_ids = HashSet::new();
    let mut shot_owners: HashMap<String, Vec<String>> = HashMap::new();
    for unit in draft
        .get("units")
        .and_then(Value::as_array)
        .ok_or("缺少单元")?
    {
        let old = text(unit, "id").to_string();
        let new = if old.starts_with(prefix) || preserved_units.contains(&old) {
            old.clone()
        } else {
            format!("{prefix}{old}")
        };
        if units_map.insert(old.clone(), new.clone()).is_some() {
            return Err("模型单元 id 重复".into());
        }
        if !unit_ids.insert(new) {
            return Err("规范化后单元 id 冲突；保留原回包".into());
        }
        let mut local_shots = HashSet::new();
        for shot in unit
            .get("shots")
            .and_then(Value::as_array)
            .ok_or("缺少镜表")?
        {
            let shot_id = text(shot, "id").to_string();
            if !local_shots.insert(shot_id.clone()) {
                return Err("模型镜号重复".into());
            }
            shot_owners.entry(shot_id).or_default().push(old.clone());
        }
    }
    let mut shots_map: HashMap<(String, String), String> = HashMap::new();
    let mut shot_ids = HashSet::new();
    for (old_shot, owners) in &shot_owners {
        // Keep the legacy mapping for globally unique IDs. If an accepted ID
        // is reused by a fresh unit, only its unambiguous accepted owner keeps
        // that ID; no adopted shot is silently renamed.
        let preserved_owner = if owners.len() > 1 && preserved_shots.contains(old_shot) {
            let found: Vec<_> = owners
                .iter()
                .filter(|owner| preserved_units.contains(*owner))
                .collect();
            if found.len() != 1 {
                return Err("重复镜号不能唯一绑定已采用单元；保留原回包".into());
            }
            found.into_iter().next()
        } else {
            None
        };
        for old_unit in owners {
            let new = if owners.len() == 1 || preserved_owner == Some(old_unit) {
                if old_shot.starts_with(prefix) || preserved_shots.contains(old_shot) {
                    old_shot.clone()
                } else {
                    format!("{prefix}{old_shot}")
                }
            } else {
                let owner = &units_map[old_unit];
                let namespace = if owner.starts_with(prefix) {
                    owner.clone()
                } else {
                    format!("{prefix}{owner}")
                };
                format!("{namespace}:shot:{old_shot}")
            };
            if !shot_ids.insert(new.clone()) {
                return Err("规范化后镜号冲突；保留原回包".into());
            }
            shots_map.insert((old_unit.clone(), old_shot.clone()), new);
        }
    }
    // A local identifier is meaningful only with its owner. Build the whole
    // mapping first and publish this clone only after every reference resolves.
    let mut normalized = draft.clone();
    // Event packing's segment headers refer to generated shot IDs. Keep those
    // references aligned when a finished response introduced fresh local IDs;
    // remap only program-created header tokens, never source dialogue text.
    for unit in normalized["units"].as_array_mut().unwrap() {
        let old_unit = text(unit, "id").to_string();
        unit["id"] = json!(units_map[&old_unit]);
        for shot in unit["shots"].as_array_mut().unwrap() {
            let old_shot = text(shot, "id").to_string();
            shot["id"] = json!(shots_map[&(old_unit.clone(), old_shot)]);
        }
        let mut headers = Vec::new();
        if let Some(segments) = unit.get_mut("segments").and_then(Value::as_array_mut) {
            for segment in segments {
                let ids = segment["shotIds"].as_array_mut().ok_or("联排段缺少镜号")?;
                let old: Vec<_> = ids
                    .iter()
                    .map(|id| id.as_str().map(str::to_string).ok_or("联排段镜号无效"))
                    .collect::<Result<_, _>>()?;
                for id in ids.iter_mut() {
                    *id = json!(shots_map
                        .get(&(old_unit.clone(), id.as_str().unwrap().to_string()))
                        .ok_or("联排段引用未知镜号")?);
                }
                if let (Some(a), Some(b), Some(c), Some(d)) = (
                    old.first(),
                    old.last(),
                    ids.first().and_then(Value::as_str),
                    ids.last().and_then(Value::as_str),
                ) {
                    headers.push((format!("【{a}—{b}】"), format!("【{c}—{d}】")));
                    headers.push((format!("【{a}起承接】"), format!("【{c}起承接】")));
                }
            }
        }
        for key in ["unitDirection", "lighting", "initialState"] {
            if let Some(value) = unit[key].as_str() {
                let mut updated = value.to_string();
                for (before, after) in &headers {
                    updated = updated.replace(before, after);
                }
                unit[key] = json!(updated);
            }
        }
    }
    for realization in normalized
        .get_mut("fantasyRealization")
        .and_then(Value::as_array_mut)
        .ok_or("缺少幻想兑现记录")?
    {
        let owners = strings(realization, "unitIds")?;
        for id in realization["unitIds"].as_array_mut().unwrap() {
            let old = id.as_str().ok_or("幻想记录 id 无效")?;
            *id = json!(units_map.get(old).ok_or("幻想记录引用不存在单元/镜号")?);
        }
        for id in realization
            .get_mut("shotIds")
            .and_then(Value::as_array_mut)
            .ok_or("幻想记录引用不是数组")?
        {
            let old_shot = id.as_str().ok_or("幻想记录 id 无效")?;
            let candidates: HashSet<_> = owners
                .iter()
                .filter_map(|owner| shots_map.get(&(owner.clone(), old_shot.to_string())))
                .collect();
            if candidates.len() != 1 {
                return Err("幻想记录镜号在声明单元中不存在或有歧义；保留原回包".into());
            }
            *id = json!(candidates.into_iter().next().unwrap());
        }
    }
    *draft = normalized;
    Ok(())
}

pub(super) fn validate_fantasy_profile(
    draft: &Value,
    selected: &[Value],
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        creative::validate_fantasy_records(draft, selected)
    } else {
        validate_fantasy(draft, selected)
    }
}

fn validate_fantasy(draft: &Value, selected: &[Value]) -> Result<(), String> {
    let realized = draft
        .get("fantasyRealization")
        .and_then(Value::as_array)
        .ok_or("缺少幻想兑现记录")?;
    let mut seen = HashSet::new();
    for r in realized {
        let id = text(r, "candidateId");
        if !selected.iter().any(|c| text(c, "id") == id) || !seen.insert(id) {
            return Err("未选幻想被执行或兑现记录重复".into());
        }
        let seconds = r
            .get("screenSeconds")
            .and_then(Value::as_f64)
            .filter(|s| s.is_finite() && *s > 0.0)
            .ok_or("幻想占时须与实际演出相符")?;
        let unit_ids = strings(r, "unitIds")?;
        let shot_ids = strings(r, "shotIds")?;
        if unit_ids.is_empty()
            || shot_ids.is_empty()
            || text(r, "entry").trim().is_empty()
            || text(r, "exit").trim().is_empty()
        {
            return Err("已选幻想缺少实际入出或镜引用".into());
        }
        let available: f64 = draft["units"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|u| unit_ids.iter().any(|id| id == text(u, "id")))
            .flat_map(|u| u["shots"].as_array().unwrap())
            .filter(|s| shot_ids.iter().any(|id| id == text(s, "id")))
            .filter_map(|s| s["duration"].as_f64())
            .sum();
        if seconds > available + 0.001 {
            return Err("幻想占时超出引用镜长".into());
        }
    }
    if selected.iter().any(|c| !seen.contains(text(c, "id"))) {
        return Err("已选幻想机会未实际兑现，不能静默删掉".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (LockedSource, Value, Vec<Value>, Value) {
        let source = prepare_source("第一集\n场景：门口\n小王OS：“先走，别回头😀。”\n").unwrap();
        let row = &source.rows[2];
        let q = row.dialogue.as_ref().unwrap();
        let eid = &source.episodes[0].id;
        let sid = &source.scenes[0].id;
        let assets = vec![
            json!({"id":"王:白衣","variantId":"白衣","applicability":{"episodes":[eid],"scenes":[sid]}}),
        ];
        let draft = json!({"schemaVersion":"storyboard-draft/v1","units":[{"id":"U1","episodeId":eid,"eventId":"E1","sceneId":sid,"title":"走","unitDirection":"她推门，他在画外说话。","scene":"门口","initialState":"小王站在门边。","exitState":"门打开。","lighting":"门外侧光","sourceRefs":[row.id],"assetRefs":[{"assetId":"王:白衣","variantId":"白衣","order":1,"use":"character","sourceRefs":[row.id]}],"shots":[{"id":"S1","duration":3.0,"camera":"近景","picture":"小王推门。","lighting":"侧光","sound":"门轴声","transition":"切到门外","sourceRefs":[row.id],"contextRefs":[],"dialogueParts":[{"lineId":q.id,"text":q.quote,"speaker":"小王","channel":"OS","presentation":"OS","voiceStart":0.0,"voiceEnd":2.9,"delivery":"压低声音"}]}]}],"fantasyRealization":[],"revisionNotes":[],"unresolved":[]});
        (
            source,
            draft,
            assets,
            json!({"minDuration":0,"maxDuration":15,"imageLimit":1}),
        )
    }
    #[test]
    fn exact_source_dialogue_passes_and_rephrasing_fails() {
        let (s, mut d, a, c) = fixture();
        validate_storyboard(&s, &d, &a, &c).unwrap();
        d["units"][0]["shots"][0]["dialogueParts"][0]["text"] = json!("先走，别回头。");
        assert!(validate_storyboard(&s, &d, &a, &c).is_err());
    }
    #[test]
    fn voice_windows_assets_and_capacity_cannot_be_clamped() {
        let (s, mut d, a, c) = fixture();
        d["units"][0]["shots"][0]["dialogueParts"][0]["voiceEnd"] = json!(4.0);
        assert!(validate_storyboard(&s, &d, &a, &c).is_err());
        let (_, mut d, _, _) = fixture();
        d["units"][0]["assetRefs"][0]["variantId"] = json!("战损");
        assert!(validate_storyboard(&s, &d, &a, &c).is_err());
        let (_, mut d, _, _) = fixture();
        d["units"][0]["shots"][0]["duration"] = json!(16.0);
        assert!(validate_storyboard(&s, &d, &a, &c).is_err());
    }

    #[test]
    fn delivery_canonicalizes_numeric_error_and_retains_real_shot_and_voice_times() {
        let (source, mut draft, assets, caps) = fixture();
        let original = draft["units"][0]["shots"][0].clone();
        for i in 0..10 {
            let mut shot = original.clone();
            shot["id"] = json!(format!("fractional{i}"));
            shot["duration"] = json!(0.1);
            shot["dialogueParts"] = json!([]);
            draft["units"][0]["shots"]
                .as_array_mut()
                .unwrap()
                .push(shot);
        }
        let timeline = draft["units"][0]["shots"].clone();
        let sum: f64 = timeline
            .as_array()
            .unwrap()
            .iter()
            .map(|shot| shot["duration"].as_f64().unwrap())
            .sum();
        assert_ne!(sum.fract(), 0.0);
        add_delivery_fields(&mut draft, "真人").unwrap();
        assert_eq!(draft["units"][0]["duration"], json!(4.0));
        assert_eq!(draft["units"][0]["shots"], timeline);
        validate_storyboard(&source, &draft, &assets, &caps).unwrap();
        draft["units"][0]["shots"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["duration"] = json!(0.6);
        let real_timeline = draft["units"][0]["shots"].clone();
        add_delivery_fields(&mut draft, "真人").unwrap();
        mark_short_endings(&mut draft, 0.0);
        assert!((draft["units"][0]["duration"].as_f64().unwrap() - 4.5).abs() < 1e-6);
        assert_eq!(draft["units"][0]["shots"], real_timeline);
        assert!(draft["units"][0]["capacityIssues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "DURATION_REVISION_REQUIRED"));
        assert_eq!(
            draft["units"][0]["shots"][0]["dialogueParts"],
            original["dialogueParts"]
        );
        validate_storyboard(&source, &draft, &assets, &caps).unwrap();
    }
    #[test]
    fn splitting_preserves_exact_quote_once() {
        let (s, mut d, a, c) = fixture();
        let q = s.rows[2].dialogue.as_ref().unwrap();
        let mut shot = d["units"][0]["shots"][0].clone();
        d["units"][0]["shots"][0]["dialogueParts"][0]["text"] = json!("“先走，");
        shot["id"] = json!("S2");
        shot["dialogueParts"][0]["text"] = json!("别回头😀。”");
        d["units"][0]["shots"].as_array_mut().unwrap().push(shot);
        validate_storyboard(&s, &d, &a, &c).unwrap();
        assert_eq!(q.quote, "“先走，别回头😀。”");
    }

    #[test]
    fn inline_variant_uses_exact_parent_image_without_losing_semantic_state() {
        let (source, mut draft, mut assets, caps) = fixture();
        assets[0]["baseAssetId"] = json!("王");
        assets[0]["preparation"] = json!({"mode":"GENERATE"});
        let mut child = assets[0].clone();
        child["id"] = json!("王:伤态");
        child["variantId"] = json!("hurt");
        child["sourceAssetId"] = assets[0]["id"].clone();
        child["sourceVariantId"] = assets[0]["variantId"].clone();
        child["preparation"] =
            json!({"mode":"INLINE","fromKey":"王:白衣","note":"伤态在当前画面表达"});
        assets.push(child);
        let mut reference = draft["units"][0]["assetRefs"][0].clone();
        reference["assetId"] = json!("王:伤态");
        reference["variantId"] = json!("hurt");
        reference["order"] = json!(2);
        draft["units"][0]["assetRefs"]
            .as_array_mut()
            .unwrap()
            .push(reference);
        materialize_asset_references(&mut draft, &assets).unwrap();
        assert_eq!(draft["units"][0]["assetRefs"][1]["assetId"], "王:伤态");
        assert_eq!(draft["units"][0]["assetRefs"][1]["imageAssetId"], "王:白衣");
        validate_storyboard(&source, &draft, &assets, &caps).unwrap();
        draft["units"][0]["assetRefs"][1]["imageAssetId"] = json!("冒名父图");
        assert!(validate_storyboard(&source, &draft, &assets, &caps)
            .unwrap_err()
            .contains("来源关系"));
    }

    #[test]
    fn audio_source_is_evidence_and_never_an_image_or_fabricated_audio_import() {
        let (source, mut draft, mut assets, _) = fixture();
        assets[0]["preparation"] = json!({"mode":"AUDIO"});
        materialize_asset_references(&mut draft, &assets).unwrap();
        assert_eq!(draft["units"][0]["assetRefs"], json!([]));
        assert_eq!(
            draft["units"][0]["soundSources"][0]["materialStatus"],
            "SOURCE_ONLY"
        );
        validate_storyboard(
            &source,
            &draft,
            &assets,
            &json!({"minDuration":0,"maxDuration":15,"imageLimit":0}),
        )
        .unwrap();
        let before = draft.clone();
        materialize_asset_references(&mut draft, &assets).unwrap();
        assert_eq!(draft, before);
        draft["units"][0]["soundSources"][0]["sourceRefs"] = json!(["foreign-source"]);
        assert!(materialize_asset_references(&mut draft, &assets).is_err());
    }
    struct Authored {
        plan: Value,
        draft: Value,
        stages: tokio::sync::Mutex<Vec<String>>,
    }
    #[async_trait]
    impl TextExecutor for Authored {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            self.stages.lock().await.push(request.stage.clone());
            assert_eq!(request.rule_snapshot["primary"]["id"], request.skill);
            if request.stage == "storyboardPlan" {
                Ok(self.plan.clone())
            } else if request.input["finishMode"] == "delta" {
                Ok(
                    json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":request.input["previousDraft"]["units"].as_array().unwrap().iter().map(|unit|unit["id"].clone()).collect::<Vec<_>>(),"replacements":[],"revisionNotes":[],"unresolved":[]}),
                )
            } else {
                let mut draft = self.draft.clone();
                draft["schemaVersion"] = json!("storyboard-draft/v2");
                let mut offsets: HashMap<String, usize> = HashMap::new();
                for unit in draft["units"].as_array_mut().unwrap() {
                    for shot in unit["shots"].as_array_mut().unwrap() {
                        for part in shot["dialogueParts"].as_array_mut().unwrap() {
                            let id = text(part, "lineId").to_string();
                            let quote = request.input["sourceRows"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .find_map(|row| {
                                    let quote = &row["dialogue"];
                                    (text(quote, "id") == id).then_some(quote)
                                })
                                .unwrap();
                            let old = *offsets.get(&id).unwrap_or(&0);
                            let count = text(part, "text").encode_utf16().count();
                            let start = quote["spokenSegments"][0]["range"]["start"]
                                .as_u64()
                                .unwrap() as usize
                                + old;
                            offsets.insert(id, old + count);
                            part["quoteStart"] = json!(start);
                            part["quoteEnd"] = json!(start + count);
                            let map = part.as_object_mut().unwrap();
                            map.remove("text");
                            map.remove("speaker");
                            map.remove("channel");
                        }
                    }
                }
                Ok(draft)
            }
        }
    }
    #[tokio::test]
    async fn three_roles_emit_bound_independently_usable_units() {
        let (s, mut draft, assets, caps) = fixture();
        let eid = &s.episodes[0].id;
        let sid = &s.scenes[0].id;
        let row = &s.rows[2].id;
        draft["units"][0]["participants"] = json!(["小王"]);
        draft["units"][0]["props"] = json!([]);
        draft["units"][0]["boundaryBefore"] =
            json!({"kind":"episode_start","reason":"当前集源开始"});
        let emotion = json!({"subject":"小王","contextTone":"急迫","audienceEffect":"离开决定可读","change":"被追赶后决定离开","landing":"推门出去","retain":["原话"]});
        let mut plan = json!({"schemaVersion":"storyboard-plan/v2","emotionArc":"离开","events":[{"id":"E1","sceneId":sid,"sourceRefs":[row],"boundaryBefore":{"kind":"episode_start","reason":"当前集源开始"},"emotionContract":emotion}],"workPackages":[{"id":"W1","eventId":"E1","sourceRefs":[row],"contextRefs":[],"entryFactIds":[],"dependsOn":[],"emotionContract":emotion}],"entryFacts":[],"fantasyCandidates":[]});
        plan["unitPackingPolicy"] = json!("event-first-v1");
        plan["provisionalUnits"] =
            json!([{"id":"P1","packageId":"W1","sourceRefs":[row],"secondsRange":[2,3]}]);
        plan["workPackages"][0]["scenarioKeys"] = json!([]);
        let exec = Authored {
            plan,
            draft,
            stages: tokio::sync::Mutex::new(vec![]),
        };
        let output=run_storyboard(&json!({"source":s,"assets":assets,"capabilities":caps,"settings":{"fantasyMode":"off","visualStyle":"16:9真人"},"episodeId":eid}),&exec,"gpt-6.1-sol","medium").await.unwrap();
        assert!(output["blocked"].as_array().unwrap().is_empty());
        assert_eq!(output["units"].as_array().unwrap().len(), 1);
        assert_eq!(
            *exec.stages.lock().await,
            vec!["storyboardPlan", "storyboardCreate", "storyboardFinish"]
        );
        assert_eq!(output["units"][0]["duration"], 3.0);
        let prompt = output["units"][0]["prompt"].as_str().unwrap();
        assert!(prompt.contains("{{asset:王:白衣:白衣}}"));
        assert!(prompt.contains("“先走，别回头😀。”"));
        assert!(prompt.contains("初态：小王站在门边。"));
    }
    #[test]
    fn cross_unit_quote_fragments_have_program_owned_nonoverlapping_coverage() {
        let (s, mut draft, assets, caps) = fixture();
        let mut second = draft["units"][0].clone();
        draft["units"][0]["shots"][0]["dialogueParts"][0]["text"] = json!("“先走，");
        second["id"] = json!("U2");
        second["shots"][0]["id"] = json!("S2");
        second["shots"][0]["dialogueParts"][0]["text"] = json!("别回头😀。”");
        draft["units"].as_array_mut().unwrap().push(second);
        validate_storyboard(&s, &draft, &assets, &caps).unwrap();
        materialize_unit_coverage(&s, &mut draft).unwrap();
        assert_eq!(
            draft["units"][0]["sourceCoverage"][0]["end"],
            draft["units"][1]["sourceCoverage"][0]["start"]
        );
        assert_eq!(
            draft["units"][0]["sourceCoverage"][0]["start"],
            s.rows[2].range.start
        );
        assert_eq!(
            draft["units"][1]["sourceCoverage"][0]["end"],
            s.rows[2].range.end
        );
    }
}

#[cfg(test)]
mod production_contract_tests {
    use super::*;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct Recorded {
        calls: Mutex<Vec<TextRequest>>,
        accepted: Mutex<HashMap<String, Value>>,
        rejected: Mutex<Vec<String>>,
        publications: Mutex<Vec<Value>>,
        continuity_updates: Mutex<Vec<Value>>,
        fractional_editor: bool,
        editor_duration: Option<f64>,
        carrier_response: Option<&'static str>,
        fantasy: bool,
        recommended_fantasy: bool,
        linked_fantasy: bool,
        fail_episode: String,
        invalid_interval: bool,
        rough_per_row: bool,
        large_picture: usize,
        fail_finish: bool,
        invalid_finish_delta: bool,
        plan_override: Option<Value>,
        planner_boundary_repair: Option<&'static str>,
        long_dialogue: bool,
        author_scope_response: Option<&'static str>,
        author_chain_branch: bool,
        author_source_error: Option<&'static str>,
        source_repair_no_progress: bool,
        finish_scope_response: Option<&'static str>,
        rejected_raw: Mutex<HashMap<String, Value>>,
        rejected_values: Mutex<Vec<(TextRequest, Value)>>,
    }

    fn mock_plan(request: &TextRequest, fantasy: bool) -> Value {
        let rows = request.input["sourceRows"].as_array().unwrap();
        let refs: Vec<_> = rows.iter().map(|row| row["id"].clone()).collect();
        let emotion = json!({"subject":"人物","contextTone":"当前源情境","audienceEffect":"真实回应可读","change":"依源事实回应","landing":"真实结果可见","retain":["源动作与原话"]});
        let mut value = json!({"schemaVersion":"storyboard-plan/v2","emotionArc":"因果回应",
            "events":[{"id":"E1","sceneId":rows[0]["sceneId"],"sourceRefs":refs,"boundaryBefore":{"kind":"episode_start","reason":"本集开始"},"emotionContract":emotion}],
            "workPackages":[{"id":"W1","eventId":"E1","sourceRefs":refs,"contextRefs":[],"entryFactIds":[],"dependsOn":[],"emotionContract":emotion}],"entryFacts":[],
            "fantasyCandidates":if fantasy {json!([{"id":"F1","sourceRefs":[rows[0]["id"]],"subject":"人物","emotionPurpose":"情绪重音","recommended":false,"proposal":"具象变化"}])}else{json!([])}});
        value["unitPackingPolicy"] = json!("event-first-v1");
        value["workPackages"][0]["scenarioKeys"] = json!([]);
        value["provisionalUnits"] = json!([{"id":"P1","packageId":"W1","sourceRefs":refs,"secondsRange":[3,rows.len() as f64*3.0]}]);
        for candidate in value["fantasyCandidates"].as_array_mut().unwrap() {
            candidate["screenSecondsRange"] = json!([1, 2]);
            candidate["extraSecondsRange"] = json!([0, 1]);
        }
        value
    }

    fn mock_draft(request: &TextRequest, invalid_interval: bool) -> Value {
        let event = request.input["workPackage"]["eventId"]
            .as_str()
            .map(|id| json!(id))
            .unwrap_or_else(|| {
                request.schema["properties"]["units"]["items"]["properties"]["eventId"]["enum"][0]
                    .clone()
            });
        let units: Vec<_>=request.input["sourceRows"].as_array().unwrap().iter().enumerate().map(|(index,row)| {
            let quote=&row["dialogue"];
            let parts: Vec<_>=quote["spokenSegments"].as_array().into_iter().flatten().enumerate().map(|(position,segment)|json!({
                "lineId":quote["id"],"quoteStart":if invalid_interval {quote["range"]["start"].clone()}else{segment["range"]["start"].clone()},"quoteEnd":segment["range"]["end"],"presentation":"画内","delivery":"自然说出","voiceStart":position as f64*0.5,"voiceEnd":position as f64*0.5+0.4
            })).collect();
            json!({"id":format!("U{index}"),"episodeId":row["episodeId"],"eventId":event,"sceneId":row["sceneId"],"title":"源动作回应","unitDirection":"人物完成源动作并回应。","intent":{"subject":"人物","change":"回应","landing":"结果","retain":["原话"]},"boundaryBefore":{"kind":"continuous","reason":"本场连续动作"},"scene":"门口","participants":["人物"],"props":[],"lighting":"门外侧光","initialState":"人物站在门边。","exitState":"人物保持原站位。","sourceRefs":[row["id"]],"assetRefs":[],"shots":[{"id":format!("S{index}"),"duration":3.0,"camera":"近景读回应","picture":"人物回应","lighting":"侧光","sound":"环境底声与原声","transition":"自然切出","sourceRefs":[row["id"]],"contextRefs":[],"dialogueParts":parts}]})
        }).collect();
        json!({"schemaVersion":"storyboard-draft/v2","units":units,"fantasyRealization":[],"revisionNotes":[],"unresolved":[]})
    }

    #[async_trait]
    impl TextExecutor for Recorded {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            let key = serde_json::to_string(&request).unwrap();
            if let Some(value) = self.accepted.lock().await.get(&key).cloned() {
                return Ok(value);
            }
            if let Some(value) = self.rejected_raw.lock().await.get(&key).cloned() {
                return Ok(value);
            }
            self.calls.lock().await.push(request.clone());
            if request.stage == "storyboardPlan" {
                let mut value = self
                    .plan_override
                    .clone()
                    .unwrap_or_else(|| mock_plan(&request, self.fantasy));
                if self.planner_boundary_repair == Some("FIX")
                    && request.input["planRepair"].is_object()
                {
                    value["events"][1]["boundaryBefore"]["kind"] = json!("time_space_jump");
                }
                if self.recommended_fantasy {
                    for candidate in value["fantasyCandidates"].as_array_mut().unwrap() {
                        candidate["recommended"] = json!(true);
                    }
                }
                if self.rough_per_row {
                    value["provisionalUnits"]=json!(request.input["sourceRows"].as_array().unwrap().iter().enumerate().map(|(i,row)|json!({"id":format!("P{i}"),"packageId":"W1","sourceRefs":[row["id"]],"secondsRange":[20,30]})).collect::<Vec<_>>());
                }
                if request.input["episode"]["id"].as_str() == Some(self.fail_episode.as_str()) {
                    value["events"] = json!([]);
                }
                Ok(value)
            } else if request.input["finishMode"] == "delta" {
                if self.fail_finish {
                    return Err("模拟整编失败；不自动重试".into());
                }
                let mut retained: Vec<_> = request.input["previousDraft"]["units"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|unit| unit["id"].clone())
                    .collect();
                if self.invalid_finish_delta {
                    retained.pop();
                }
                if [
                    creative::AUTHOR_SCOPE_REPAIR_EXECUTION_VERSION,
                    creative::FINISH_SCOPE_REPAIR_EXECUTION_VERSION,
                ]
                .contains(&text(&request.input["executionBatch"], "version"))
                {
                    let mode = if self.source_repair_no_progress {
                        "NO_PROGRESS"
                    } else if self.author_source_error.is_some() {
                        "SOURCE_FIX"
                    } else if request.input["finishRepair"].is_object() {
                        match self.finish_scope_response.unwrap() {
                            "TIMING_FIX" => "SOURCE_FIX",
                            "TIMING_NO_PROGRESS" => "NO_PROGRESS",
                            mode => mode,
                        }
                    } else {
                        self.author_scope_response.unwrap()
                    };
                    let attempt = request.input[if request.input["finishRepair"].is_object() {
                        "finishRepair"
                    } else {
                        "authorRepair"
                    }]["attempt"]
                        .clone();
                    if mode == "NO_PROGRESS" {
                        return Ok(
                            json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":[],"revisionNotes":[],"unresolved":[]}),
                        );
                    }
                    let affected: HashSet<_> = request.input["validationIssues"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|issue| issue["unitId"].as_str())
                        .collect();
                    let selected: Vec<_> = request.input["previousDraft"]["units"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|unit| {
                            affected.contains(text(unit, "id"))
                                || (mode == "FOREIGN" && !affected.contains(text(unit, "id")))
                        })
                        .collect();
                    let mut groups = Vec::new();
                    if mode == "SOURCE_FIX" {
                        let mut writer = request.clone();
                        writer.input["workPackage"] = request.input["workPackages"][0].clone();
                        if self.author_source_error == Some("COVERAGE") {
                            let repaired = mock_draft(&writer, false);
                            return Ok(
                                json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":[],"replacements":[{"replaces":retained,"units":repaired["units"],"fantasyRealization":[]}],"revisionNotes":[],"unresolved":[]}),
                            );
                        }
                        for (index, old) in selected.iter().enumerate() {
                            writer.input["sourceRows"] = json!(request.input["sourceRows"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .filter(|row| old["sourceRefs"]
                                    .as_array()
                                    .unwrap()
                                    .contains(&row["id"]))
                                .cloned()
                                .collect::<Vec<_>>());
                            let mut repaired = mock_draft(&writer, false);
                            for (position, unit) in repaired["units"]
                                .as_array_mut()
                                .unwrap()
                                .iter_mut()
                                .enumerate()
                            {
                                unit["id"] = json!(format!("source-fixed-{index}-{position}"));
                                unit["shots"][0]["id"] =
                                    json!(format!("source-shot-{index}-{position}"));
                            }
                            retained.retain(|id| *id != old["id"]);
                            groups.push(json!({"replaces":[old["id"]],"units":repaired["units"],"fantasyRealization":[]}));
                        }
                        return Ok(
                            json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":groups,"revisionNotes":[],"unresolved":[]}),
                        );
                    }
                    for old in selected {
                        let mut next = old.clone();
                        if ["PARTIAL", "LIMIT"].contains(&mode) && attempt == 1 {
                            next["assetRefs"].as_array_mut().unwrap().remove(0);
                        } else if mode == "LIMIT" {
                            next["assetRefs"][0]["assetId"] =
                                request.input["assets"][0]["id"].clone();
                        } else {
                            next["assetRefs"] = json!([]);
                        }
                        if mode == "SCENE_COORDINATE" {
                            next["sceneId"] = if attempt == 1 {
                                request.input["assets"][0]["applicability"]["scenes"][0].clone()
                            } else {
                                request.input["validationIssues"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .find(|issue| issue["code"] == "UNIT_SCENE_COORDINATE")
                                    .unwrap()["expectedSceneId"]
                                    .clone()
                            };
                        }
                        next["exitState"] = json!("原整编已验收真实末态。");
                        if mode == "BAD_QUOTE" {
                            next["shots"][0]["dialogueParts"][0]["text"] = json!("非法改写原话");
                        }
                        retained.retain(|id| id != &old["id"]);
                        groups.push(
                            json!({"replaces":[old["id"]],"units":[next],"fantasyRealization":[]}),
                        );
                    }
                    return Ok(
                        json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":groups,"revisionNotes":["按源实际声画修正参考资产需求"],"unresolved":[]}),
                    );
                }
                if request.input["executionBatch"]["version"] == "carrier-repair/v1" {
                    if self.carrier_response == Some("NO_PROGRESS") {
                        return Ok(
                            json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":[],"revisionNotes":[],"unresolved":[]}),
                        );
                    }
                    let affected: HashSet<_> = request.input["carrierProblems"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|p| p["unitId"].as_str())
                        .collect();
                    let mut groups = Vec::new();
                    retained.retain(|id| !affected.contains(id.as_str().unwrap()));
                    for unit in request.input["previousDraft"]["units"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|u| affected.contains(text(u, "id")))
                    {
                        let mut next = unit.clone();
                        next["shots"][0]["duration"] =
                            json!(if self.carrier_response == Some("MOVING") {
                                unit["shots"][0]["duration"].as_f64().unwrap() + 1.0
                            } else {
                                4.0
                            });
                        next["duration"] = next["shots"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|s| s["duration"].as_f64().unwrap())
                            .sum::<f64>()
                            .into();
                        if self.carrier_response == Some("BAD_QUOTE") {
                            next["shots"][0]["dialogueParts"][0]["text"] =
                                json!("模型不应改写原话。");
                        }
                        groups.push(
                            json!({"replaces":[unit["id"]],"units":[next],"fantasyRealization":[]}),
                        );
                    }
                    return Ok(
                        json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":groups,"revisionNotes":["原整编仅修载体时长"],"unresolved":[]}),
                    );
                }
                if self.fractional_editor {
                    let old = &request.input["previousDraft"]["units"][0];
                    let mut changed = old.clone();
                    let seconds = self.editor_duration.unwrap_or(3.5);
                    changed["shots"][0]["duration"] = json!(seconds);
                    changed["duration"] = json!(seconds);
                    retained.retain(|id| id != &old["id"]);
                    return Ok(
                        json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":[{"replaces":[old["id"]],"units":[changed],"fantasyRealization":[]}],"revisionNotes":[],"unresolved":[]}),
                    );
                }
                if self.finish_scope_response.is_some() {
                    let old = &request.input["previousDraft"]["units"][0];
                    let mut next = old.clone();
                    if matches!(
                        self.finish_scope_response,
                        Some("TIMING_FIX" | "TIMING_NO_PROGRESS")
                    ) {
                        let mut silent = next["shots"][0].clone();
                        silent["id"] = json!("finish-zero-duration");
                        silent["duration"] = json!(0);
                        silent["dialogueParts"] = json!([]);
                        next["shots"].as_array_mut().unwrap().push(silent);
                    } else {
                        next["assetRefs"]=json!(request.input["assets"].as_array().unwrap().iter().take(2).enumerate().map(|(index,asset)|json!({"assetId":asset["id"],"variantId":asset["variantId"],"order":index+1,"use":"character","sourceRefs":old["sourceRefs"]})).collect::<Vec<_>>());
                    }
                    retained.retain(|id| id != &old["id"]);
                    return Ok(
                        json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":[{"replaces":[old["id"]],"units":[next],"fantasyRealization":[]}],"revisionNotes":[],"unresolved":[]}),
                    );
                }
                Ok(
                    json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":[],"revisionNotes":[],"unresolved":[]}),
                )
            } else {
                let mut draft = mock_draft(&request, self.invalid_interval);
                if let Some(candidates) = request.input["selectedFantasy"].as_array() {
                    if !candidates.is_empty() {
                        draft["units"][0]["shots"][0]["picture"] = json!("人物的倒影从门面浮出，短暂放大凝住，再收回实际身影；人物仍完成源动作与原话。");
                        draft["fantasyRealization"] = json!(candidates.iter().map(|candidate|json!({"candidateId":candidate["id"],"unitIds":["U0"],"shotIds":["S0"],"screenSeconds":1.0,"entry":"倒影从门面浮出。","exit":"倒影收回实际身影。","preservedFunction":"保留源动作与原话，具象强化情绪回应。"})).collect::<Vec<_>>());
                        if request.input[creative::PROFILE_FIELD] == creative::PROFILE {
                            for record in draft["fantasyRealization"].as_array_mut().unwrap() {
                                record["domain"] = json!("人物主观边界；现实持物不变");
                                record["status"] = json!("realized");
                            }
                        }
                        if self.linked_fantasy && draft["units"].as_array().unwrap().len() > 1 {
                            draft["units"][1]["shots"][0]["picture"] = json!("主观倒影沿前一镜的目光落到门面，人物照原话开口，倒影随身体站稳收回真实轮廓。");
                            for record in draft["fantasyRealization"].as_array_mut().unwrap() {
                                record["unitIds"] = json!(["U0", "U1"]);
                                record["shotIds"] = json!(["S0", "S1"]);
                            }
                        }
                    }
                }
                if self.large_picture > 0 {
                    for unit in draft["units"].as_array_mut().unwrap() {
                        unit["shots"][0]["picture"] =
                            json!("源内有效画面".repeat(self.large_picture));
                    }
                }
                if self.long_dialogue {
                    for unit in draft["units"].as_array_mut().unwrap() {
                        let mut first = unit["shots"][0].clone();
                        let part = &first["dialogueParts"][0];
                        let start = part["quoteStart"].as_u64().unwrap();
                        let end = part["quoteEnd"].as_u64().unwrap();
                        let cut = start + (end - start) / 2;
                        first["duration"] = json!(12.0);
                        first["dialogueParts"][0]["quoteEnd"] = json!(cut);
                        first["dialogueParts"][0]["voiceStart"] = json!(0.5);
                        first["dialogueParts"][0]["voiceEnd"] = json!(11.8);
                        first["picture"]=json!("人物说话同时转体，抬手截住门板，反光中的主观轮廓随掌势放大，现实支点与持物不变。");
                        first["camera"] =
                            json!("沿手臂推到接触点，人物转体时侧移保持掌面与门缝同框。");
                        first["transition"] = json!("原句不断声，沿接触点切到听者。");
                        let mut second = first.clone();
                        second["id"] = json!(format!("{}-response", text(&first, "id")));
                        second["duration"] = json!(8.0);
                        second["dialogueParts"][0]["quoteStart"] = json!(cut);
                        second["dialogueParts"][0]["quoteEnd"] = json!(end);
                        second["dialogueParts"][0]["voiceStart"] = json!(0.0);
                        second["dialogueParts"][0]["voiceEnd"] = json!(7.5);
                        second["picture"]=json!("听者的视线落到门缝，手指抓紧再松开，人物持续说完原句，反光轮廓随松开的手势回归现实。");
                        second["camera"] =
                            json!("独立反打听者，沿落词缓慢拉出，展示门缝中的实际手势。");
                        unit["shots"] = json!([first, second]);
                    }
                }
                if let Some(mode) = self.author_scope_response {
                    if request.input["acceptedContinuity"].is_null() {
                        let last = draft["units"].as_array_mut().unwrap().last_mut().unwrap();
                        last["assetRefs"]=json!(request.input["assets"].as_array().unwrap().iter().take(2).enumerate().map(|(index,asset)|json!({"assetId":asset["id"],"variantId":asset["variantId"],"order":index+1,"use":"character","sourceRefs":last["sourceRefs"]})).collect::<Vec<_>>());
                        if mode == "HARD_AUTHOR_UNKNOWN" {
                            last["assetRefs"][0]["assetId"] = json!("unknown-identity");
                        }
                        if mode == "HARD_AUTHOR_QUOTE" {
                            last["shots"][0]["dialogueParts"][0]["text"] = json!("非法改写原话");
                        }
                    }
                }
                if let Some(mode) = self.author_source_error {
                    match mode {
                        "DIALOGUE" => {
                            for part in draft["units"][0]["shots"][0]["dialogueParts"]
                                .as_array_mut()
                                .unwrap()
                            {
                                part["quoteStart"] = request.input["sourceRows"][0]["dialogue"]
                                    ["range"]["start"]
                                    .clone();
                            }
                        }
                        "COVERAGE" => {
                            draft["units"].as_array_mut().unwrap().pop();
                        }
                        "ASSET_SOURCE" => {
                            draft["units"][0]["assetRefs"] = json!([{"assetId":request.input["assets"][0]["id"],"variantId":request.input["assets"][0]["variantId"],"use":"character","order":1,"sourceRefs":[request.input["sourceRows"][1]["id"]]}]);
                        }
                        "MISSING_DIALOGUE" => {
                            draft["units"][0]["shots"][0]["dialogueParts"] = json!([]);
                        }
                        "PARTIAL_DIALOGUE" => {
                            let part = &mut draft["units"][0]["shots"][0]["dialogueParts"][0];
                            part["quoteEnd"] = json!(part["quoteStart"].as_u64().unwrap() + 1);
                        }
                        "ZERO_DURATION" => {
                            let mut silent = draft["units"][0]["shots"][0].clone();
                            silent["id"] = json!("zero-duration-silent-shot");
                            silent["duration"] = json!(0);
                            silent["dialogueParts"] = json!([]);
                            draft["units"][0]["shots"]
                                .as_array_mut()
                                .unwrap()
                                .push(silent);
                        }
                        _ => panic!("unknown source-error fixture"),
                    }
                }
                Ok(draft)
            }
        }
        async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
            self.accepted
                .lock()
                .await
                .insert(serde_json::to_string(request).unwrap(), value.clone());
            Ok(())
        }
        async fn reject(
            &self,
            request: &TextRequest,
            value: &Value,
            reason: &str,
        ) -> Result<(), String> {
            if self.author_scope_response.is_some()
                || self.finish_scope_response.is_some()
                || self.author_source_error.is_some()
            {
                self.rejected_raw
                    .lock()
                    .await
                    .insert(serde_json::to_string(request).unwrap(), value.clone());
                self.rejected_values
                    .lock()
                    .await
                    .push((request.clone(), value.clone()));
            }
            self.rejected
                .lock()
                .await
                .push(format!("{}:{reason}", request.stage));
            Ok(())
        }
        async fn publish_episode(&self, output: &Value, _: &Value) -> Result<(), String> {
            let calls = self.calls.lock().await;
            self.publications.lock().await.push(json!({"output":output,"callsAtPublication":calls.iter().map(|request|json!({"stage":request.stage,"episodeId":request.input.get("episodeId").or_else(||request.input.pointer("/episode/id"))})).collect::<Vec<_>>() }));
            Ok(())
        }
        async fn continuity_changed(&self, evidence: &Value) -> Result<(), String> {
            self.continuity_updates.lock().await.push(evidence.clone());
            Ok(())
        }
        async fn author_chain_revision(
            &self,
            request: &TextRequest,
            repair: &Value,
        ) -> Result<Option<Value>, String> {
            Ok((self.author_chain_branch && !repair.is_null()).then(||json!({"version":"accepted-author-chain-change/v1","repairProofSha256":digest(&repair.to_string()),"executionBatchId":request.input["executionBatch"]["id"]})))
        }
    }

    fn input(source: &LockedSource) -> Value {
        json!({"source":source,"assets":[],"foundation":{"overview":"人物拒绝离开。","direction":"克制且保留局部爆发。","episodes":[],"facts":[]},"settings":{"fantasyMode":"off","visualStyle":"16:9真人","videoProvider":"private-channel","textApiToken":"test-placeholder","imageResolution":"4k"},"capabilities":{"minDuration":0,"maxDuration":15,"imageLimit":5},"selection":{"episodeIds":source.episodes.iter().map(|episode|episode.id.clone()).collect::<Vec<_>>()}})
    }

    #[tokio::test]
    async fn rejected_planning_uses_the_original_planner_and_stops_identical_errors() {
        let source =
            prepare_source("第一集\n场景：门口\n甲：走吧。\n场景：屋内\n甲：来了。").unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .collect();
        let emotion = json!({"subject":"甲","contextTone":"回应","audienceEffect":"可读","change":"依原文回应","landing":"到达","retain":["全部原话"]});
        let plan = json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"到达回应","entryFacts":[],"fantasyCandidates":[],
            "events":rows.iter().enumerate().map(|(i,row)|json!({"id":format!("E{i}"),"sceneId":row.scene_id,"sourceRefs":[row.id],"boundaryBefore":{"kind":if i==0{"episode_start"}else{"continuous"},"reason":"原文场景"},"emotionContract":emotion})).collect::<Vec<_>>(),
            "workPackages":rows.iter().enumerate().map(|(i,row)|json!({"id":format!("W{i}"),"eventId":format!("E{i}"),"sourceRefs":[row.id],"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":emotion})).collect::<Vec<_>>(),
            "provisionalUnits":rows.iter().enumerate().map(|(i,row)|json!({"id":format!("P{i}"),"packageId":format!("W{i}"),"sourceRefs":[row.id],"secondsRange":[3,4]})).collect::<Vec<_>>()});
        for mode in ["FIX", "NO_PROGRESS"] {
            let executor = Recorded {
                plan_override: Some(plan.clone()),
                planner_boundary_repair: Some(mode),
                ..Default::default()
            };
            let mut payload = input(&source);
            creative::mark_input(&mut payload, true);
            let output = run_storyboard(&payload, &executor, "offline", "medium")
                .await
                .unwrap();
            let calls = executor.calls.lock().await;
            let plans: Vec<_> = calls
                .iter()
                .filter(|request| request.stage == "storyboardPlan")
                .collect();
            assert_eq!(plans.len(), 2, "bounded original planner repair");
            assert_eq!(plans[0].skill, plans[1].skill);
            assert_eq!(plans[0].input["sourceRows"], plans[1].input["sourceRows"]);
            assert_eq!(plans[0].schema, plans[1].schema);
            assert_eq!(plans[1].input["previousPlan"], plan);
            assert_eq!(plans[1].input["planRepair"]["attempt"], 1);
            if mode == "FIX" {
                assert_eq!(output["status"], "completed");
            } else {
                assert_eq!(output["status"], "partially_blocked");
                assert!(output["blocked"]
                    .to_string()
                    .contains("PLAN_REPAIR_NO_PROGRESS"));
                assert_eq!(calls.len(), 2, "no author or endless planner retry");
            }
        }
    }

    #[test]
    fn scene_coordinate_repair_keeps_same_event_group_and_preserves_unrelated_events() {
        let base = json!({"units":[{"id":"u1","eventId":"e1","sourceRefs":["r1"]},{"id":"u2","eventId":"e1","sourceRefs":["r2"]},{"id":"u3","eventId":"e2","sourceRefs":["r3"]}],"fantasyRealization":[]});
        let coordinate = author_scope_repair_units(
            &base,
            &[json!({"code":"UNIT_SCENE_COORDINATE","unitId":"u2"})],
        )
        .unwrap();
        assert_eq!(coordinate, HashSet::from(["u1".into(), "u2".into()]));
        let asset =
            author_scope_repair_units(&base, &[json!({"code":"ASSET_SCENE_SCOPE","unitId":"u2"})])
                .unwrap();
        assert_eq!(asset, HashSet::from(["u2".into()]));
    }

    fn author_scope_input(lines: usize) -> (LockedSource, Value) {
        let text = format!(
            "第一集\n场景：门口\n{}",
            (0..lines)
                .map(|i| format!("甲OS：原话{i}完整保留。\n"))
                .collect::<String>()
        );
        let source = prepare_source(&text).unwrap();
        let mut request = input(&source);
        request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        request["assets"]=json!((0..2).map(|i|json!({"id":format!("voice-person-{i}:base"),"variantId":"base","name":format!("源声人物{i}"),"kind":"CHARACTER","applicability":{"episodes":[source.episodes[0].id],"scenes":["another-scene"]},"preparation":{"mode":"REFERENCE"}})).collect::<Vec<_>>());
        (source, request)
    }

    #[test]
    fn original_editor_repair_history_is_not_an_old_author_batch_finish() {
        let source = prepare_source("第一集\n场景：门口\n甲：原话。\n").unwrap();
        let mut request = input(&source);
        request["storyboardMethod"] = json!(STORYBOARD_METHOD);
        for version in [
            creative::AUTHOR_SCOPE_REPAIR_EXECUTION_VERSION,
            creative::FINISH_SCOPE_REPAIR_EXECUTION_VERSION,
        ] {
            request["storyboardHistory"] = json!([{"stage":"storyboardFinish","status":"accepted","episodeId":source.episodes[0].id,"finishMode":"delta","executionBatchVersion":version,"outputSchemaVersion":"storyboard-delta/v2","requestHash":"original-frozen-repair"}]);
            assert!(incompatible_resume_scopes(&request, &source)
                .unwrap()
                .is_empty());
            request["storyboardHistory"][0]["outputSchemaVersion"] = json!("storyboard-draft/v2");
            assert!(
                !incompatible_resume_scopes(&request, &source)
                    .unwrap()
                    .is_empty(),
                "a full draft cannot impersonate an accepted delta repair"
            );
            request["storyboardHistory"][0]["outputSchemaVersion"] = json!("storyboard-delta/v2");
            request["storyboardHistory"][0]["finishMode"] = json!("full");
            assert!(!incompatible_resume_scopes(&request, &source)
                .unwrap()
                .is_empty());
        }
    }

    #[tokio::test]
    async fn original_editor_repairs_source_coordinates_without_reauthoring_or_adopting_rejected_raw(
    ) {
        for mode in [
            "DIALOGUE",
            "COVERAGE",
            "ASSET_SOURCE",
            "MISSING_DIALOGUE",
            "PARTIAL_DIALOGUE",
            "ZERO_DURATION",
        ] {
            let source = prepare_source(
                "第一集\n场景：门口\n甲：（低声）原话😀完整保留。\n甲：下一句完整保留。\n",
            )
            .unwrap();
            let mut request = input(&source);
            request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
            request["assets"] = json!([{"id":"person-base","variantId":"base","name":"甲","kind":"CH","applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},"preparation":{"mode":"REFERENCE"}}]);
            let frozen = request.clone();
            let exec = Recorded {
                author_source_error: Some(mode),
                ..Default::default()
            };
            let output = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(
                output["status"], "completed",
                "{mode}: {}",
                output["blocked"]
            );
            validate_storyboard_scope(
                &source,
                &output,
                request["assets"].as_array().unwrap(),
                &request["capabilities"],
                None,
            )
            .unwrap();
            let calls = exec.calls.lock().await.clone();
            assert_eq!(
                calls.iter().filter(|r| r.stage == "storyboardPlan").count(),
                1
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|r| r.stage == "storyboardCreate")
                    .count(),
                1
            );
            let repair = calls.iter().find(|r| is_author_repair_request(r)).unwrap();
            assert_eq!(repair.skill, "episode-finishing-editor");
            assert_eq!(repair.input["authorRepair"]["maximumAttempts"], 2);
            assert!(repair.rule_snapshot["runtimeContract"]["content"]
                .as_str()
                .unwrap()
                .contains(if mode == "ZERO_DURATION" {
                    "SHOT_DURATION"
                } else {
                    "括号演法不发声"
                }));
            let rejected = exec.rejected_values.lock().await.clone();
            let original = rejected
                .iter()
                .find(|(r, _)| r.stage == "storyboardCreate")
                .unwrap();
            assert!(!exec
                .accepted
                .lock()
                .await
                .contains_key(&serde_json::to_string(&original.0).unwrap()));
            let count = calls.len();
            let again = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(again, output);
            assert_eq!(
                exec.calls.lock().await.len(),
                count,
                "cached repair resumes with zero new role calls"
            );
            assert_eq!(request, frozen);
            for row in source.rows.iter().filter(|row| row.dialogue.is_some()) {
                for segment in spoken_ranges(row.dialogue.as_ref().unwrap()) {
                    let quote = slice_utf16(&source.text, &segment).unwrap();
                    assert!(
                        output["units"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|unit| text(unit, "prompt").contains(&quote)),
                        "{mode}: locked speech must be retained"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn unchanged_source_coordinate_repair_stops_once_and_resume_does_not_reset_budget() {
        for mode in [
            "DIALOGUE",
            "COVERAGE",
            "ASSET_SOURCE",
            "MISSING_DIALOGUE",
            "PARTIAL_DIALOGUE",
            "ZERO_DURATION",
        ] {
            let source = prepare_source(
                "第一集\n场景：门口\n甲：（低声）原话😀完整保留。\n甲：下一句完整保留。\n",
            )
            .unwrap();
            let mut request = input(&source);
            request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
            request["assets"] = json!([{"id":"person-base","variantId":"base","name":"甲","kind":"CH","applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},"preparation":{"mode":"REFERENCE"}}]);
            let exec = Recorded {
                author_source_error: Some(mode),
                source_repair_no_progress: true,
                ..Default::default()
            };
            let output = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(output["status"], "partially_blocked");
            assert!(
                output["blocked"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| text(item, "reason").contains("NO_PROGRESS")),
                "{mode}: {}",
                output["blocked"]
            );
            assert!(output["units"].as_array().unwrap().is_empty());
            let calls = exec.calls.lock().await.clone();
            assert_eq!(
                calls.iter().filter(|r| is_author_repair_request(r)).count(),
                1
            );
            run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(
                exec.calls.lock().await.len(),
                calls.len(),
                "no unchanged repair or author dispatch on resume"
            );
        }
    }

    #[tokio::test]
    async fn ordinary_finisher_scope_repair_preserves_its_rejected_raw_and_reuses_all_paid_steps() {
        for mode in ["FIX", "TIMING_FIX"] {
            let (source, request) = author_scope_input(1);
            let exec = Recorded {
                finish_scope_response: Some(mode),
                ..Default::default()
            };
            let output = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(output["status"], "completed", "{}", output["blocked"]);
            validate_storyboard_scope(
                &source,
                &output,
                request["assets"].as_array().unwrap(),
                &request["capabilities"],
                None,
            )
            .unwrap();
            let calls = exec.calls.lock().await.clone();
            assert_eq!(
                calls.iter().filter(|r| r.stage == "storyboardPlan").count(),
                1
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|r| r.stage == "storyboardCreate")
                    .count(),
                1
            );
            let ordinary = calls
                .iter()
                .find(|r| r.input["executionBatch"]["version"] == "related-range-finish/v1")
                .unwrap();
            let repair = calls
                .iter()
                .find(|r| {
                    r.input["executionBatch"]["version"]
                        == creative::FINISH_SCOPE_REPAIR_EXECUTION_VERSION
                })
                .unwrap();
            assert_eq!(calls.len(), 4);
            assert_eq!(repair.input["finishRepair"]["attempt"], 1);
            assert_eq!(repair.input["finishRepair"]["maximumAttempts"], 2);
            assert_eq!(
                repair.input["finishRepair"]["originDomainRequestSha256"],
                digest(&serde_json::to_string(ordinary).unwrap())
            );
            let rejected = exec.rejected_values.lock().await.clone();
            let original = rejected
                .iter()
                .find(|(r, _)| r.input["executionBatch"]["version"] == "related-range-finish/v1")
                .unwrap();
            assert_eq!(
                repair.input["finishRepair"]["originalFinishRawSha256"],
                digest(&original.1.to_string())
            );
            let accepted = exec.accepted.lock().await;
            assert!(!accepted.contains_key(&serde_json::to_string(ordinary).unwrap()));
            let proof = &accepted[&serde_json::to_string(repair).unwrap()]["materializedDraft"]
                ["finishRepairProvenance"];
            assert_eq!(proof["originalFinishAccepted"], false);
            assert_eq!(proof["version"], "accepted-finish-scope-repair/v1");
            drop(accepted);
            let before = calls.len();
            assert_eq!(
                run_storyboard(&request, &exec, "offline", "medium")
                    .await
                    .unwrap()["units"],
                output["units"]
            );
            assert_eq!(exec.calls.lock().await.len(),before,"resume must replay the original rejected Finish and the accepted repair, without new model calls");
        }
    }

    #[tokio::test]
    async fn projected_finisher_and_its_bounded_repair_reopen_exactly_without_runtime_parent_payload() {
        let (source, mut frozen) = author_scope_input(1);
        frozen["assets"] = json!([{"id":"person:base","identityId":"person","variantId":"base","version":2,"name":"甲","kind":"CH",
            "sourceAssetId":null,"sourceVariantId":null,
            "applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},
            "preparation":{"mode":"REFERENCE"},"design":{"history":"PRIVATE_FULL_PARENT_ASSETS"},"prompt":"PRIVATE_FULL_PARENT_PROMPT"}]);
        frozen["storyboardAssetInputProfile"] = json!(asset_cards::SCOPED_PROFILE);
        super::super::stage3_pack::mark_input(&mut frozen).unwrap();
        let parent_bytes = frozen.to_string();
        let mut runtime = frozen.clone();
        runtime["decisions"] = json!([]);
        runtime["storyboardHistory"] = json!([]);
        runtime["frozenFinisherAssetContext"] = frozen.clone();
        let runtime_bytes = runtime.to_string();
        let exec = Recorded {finish_scope_response:Some("TIMING_FIX"),..Default::default()};
        let output = run_storyboard(&runtime,&exec,"offline","medium").await.unwrap();
        assert_eq!(output["status"],"completed","{}",output["blocked"]);
        let calls = exec.calls.lock().await.clone();
        assert_eq!(calls.len(),4,"planner, author, original editor, bounded editor repair");
        for call in &calls {
            assert!(call.input.get("frozenFinisherAssetContext").is_none(),"{}",call.stage);
            assert!(!call.input.to_string().contains("PRIVATE_FULL_PARENT"),"{}",call.stage);
            assert_eq!(serde_json::to_value(creative::prepare_request(call.clone()).unwrap()).unwrap(),serde_json::to_value(call).unwrap(),"final wrapper must preserve domain and proof bytes");
        }
        let author = calls.iter().find(|call|call.stage=="storyboardCreate").unwrap();
        assert!(author.input.get("storyboardAssetInputProfile").is_none());
        let ordinary = calls.iter().find(|call|call.input["executionBatch"]["version"]=="related-range-finish/v1").unwrap();
        let repair = calls.iter().find(|call|call.input["executionBatch"]["version"]==creative::FINISH_SCOPE_REPAIR_EXECUTION_VERSION).unwrap();
        for call in [ordinary,repair] {
            assert_eq!(call.input["finisherAssetInputProjection"]["frozenWorkflowInputSha256"],digest(&parent_bytes));
            assert_eq!(call.input["storyboardAssetInputProfile"],asset_cards::SCOPED_PROFILE);
            assert!(call.input["assets"][0].get("design").is_none());
        }
        assert_eq!(repair.input["finishRepair"]["originDomainRequestSha256"],digest(&serde_json::to_string(ordinary).unwrap()));
        assert_eq!(repair.input["finishRepair"]["attempt"],1);
        assert_eq!(repair.input["finishRepair"]["maximumAttempts"],2);
        assert_ne!(repair.input["finisherAssetInputProjection"],ordinary.input["finisherAssetInputProjection"]);
        for _ in 0..2 {
            assert_eq!(run_storyboard(&runtime,&exec,"offline","medium").await.unwrap()["units"],output["units"]);
            assert_eq!(exec.calls.lock().await.len(),4,"reopening must not refresh the paid editor or repair budget");
        }
        assert_eq!(frozen.to_string(),parent_bytes);
        assert_eq!(runtime.to_string(),runtime_bytes);
    }

    #[tokio::test]
    async fn ordinary_finisher_scope_repair_has_two_attempts_and_never_adopts_hard_or_unchanged_errors(
    ) {
        for (mode, maximum) in [
            ("NO_PROGRESS", 1),
            ("TIMING_NO_PROGRESS", 1),
            ("LIMIT", 2),
            ("BAD_QUOTE", 1),
            ("PARTIAL", 2),
        ] {
            let (_, request) = author_scope_input(1);
            let exec = Recorded {
                finish_scope_response: Some(mode),
                ..Default::default()
            };
            let result = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            let before = exec.calls.lock().await.clone();
            assert_eq!(
                before
                    .iter()
                    .filter(|r| r.input["executionBatch"]["version"]
                        == creative::FINISH_SCOPE_REPAIR_EXECUTION_VERSION)
                    .count(),
                maximum,
                "{mode}"
            );
            assert_eq!(
                before
                    .iter()
                    .filter(|r| r.stage == "storyboardCreate")
                    .count(),
                1
            );
            if mode == "PARTIAL" {
                assert_eq!(result["status"], "completed");
            } else {
                assert_eq!(result["status"], "partially_blocked", "{mode}");
                assert!(result["units"].as_array().unwrap().is_empty());
                assert!(exec.publications.lock().await.is_empty());
            }
            run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(
                exec.calls.lock().await.len(),
                before.len(),
                "{mode}: persistent replay must not reset the two attempts"
            );
        }
    }

    fn same_episode_coordinate_fixture() -> (
        LockedSource,
        TextRequest,
        AcceptedAuthorScope,
        Value,
        Value,
        Vec<Value>,
    ) {
        let source =
            prepare_source("第一集\n场景：门口\n甲OS：原话完整保留。\n场景：广场\n△人群站定。\n")
                .unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| {
                row.kind == "body" && row.scene_id.as_deref() == Some(source.scenes[0].id.as_str())
            })
            .cloned()
            .collect();
        let assets = vec![
            json!({"id":"known:base","variantId":"base","name":"已登记人物","kind":"CHARACTER","applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[1].id]},"preparation":{"mode":"REFERENCE"}}),
        ];
        let caps = json!({"minDuration":0,"maxDuration":15,"imageLimit":5});
        let mut request = TextRequest {
            stage: "storyboardCreate".into(),
            skill: "unit-performance-director".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"episodeId":source.episodes[0].id,"sourceRows":rows,"contextRows":[],"assets":assets,"capabilities":caps,"settings":{"fantasyMode":"off","visualStyle":"16:9真人"},creative::PROFILE_FIELD:creative::PROFILE}),
            schema: json!({}),
            rule_snapshot: snapshot("unit-performance-director", &[]).unwrap(),
        };
        let plan = mock_plan(&request, false);
        request.input["plan"] = plan.clone();
        request.input["workPackage"] = plan["workPackages"][0].clone();
        request.input["workPackages"] = plan["workPackages"].clone();
        request.input["selectedFantasy"] = json!([]);
        let refs: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
        let batch = original::ExecutionBatch {
            id: "known-source-batch".into(),
            package_ids: vec!["W1".into()],
            source_refs: refs.clone(),
            provisional_unit_ids: vec!["P1".into()],
            estimated_seconds: 3.0,
            exceeds_soft_target: false,
        };
        request.input["executionBatch"] = batch.value();
        request = creative::prepare_request(request).unwrap();
        let mut raw = mock_draft(&request, false);
        raw["units"][0]["assetRefs"] = json!([{"assetId":"known:base","variantId":"base","order":1,"use":"character","sourceRefs":refs}]);
        let (draft, issues) = author_scope_candidate(
            &source,
            &raw,
            &assets,
            &caps,
            &batch.source_refs,
            "16:9真人",
            &format!("{}:", source.episodes[0].id),
            &plan,
            &[],
            true,
        )
        .unwrap();
        let scope = AcceptedAuthorScope {
            index: 0,
            batch,
            input: request.input.clone(),
            draft,
        };
        (source, request, scope, raw, plan, issues)
    }

    #[tokio::test]
    async fn rejected_editor_same_episode_coordinate_uses_only_the_remaining_attempt_and_final_strict_gate(
    ) {
        let (source, origin, scope, raw, plan, issues) = same_episode_coordinate_fixture();
        let assets = origin.input["assets"].as_array().unwrap();
        let rows: Vec<SourceRow> =
            serde_json::from_value(origin.input["sourceRows"].clone()).unwrap();
        let exec = Recorded {
            author_scope_response: Some("SCENE_COORDINATE"),
            ..Default::default()
        };
        let output = repair_author_scope(
            &source,
            &raw,
            &origin,
            &scope,
            issues.clone(),
            &plan,
            &rows,
            &[],
            assets,
            &[],
            &source.episodes[0].id,
            "offline",
            "medium",
            None,
            &origin.input["capabilities"],
            "16:9真人",
            &origin.input,
            &exec,
        )
        .await
        .unwrap();
        validate_storyboard_scope(
            &source,
            &output,
            assets,
            &origin.input["capabilities"],
            Some(&scope.batch.source_refs),
        )
        .unwrap();
        let calls = exec.calls.lock().await.clone();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|r| r.stage == "storyboardFinish"));
        assert_eq!(calls[1].input["authorRepair"]["attempt"], 2);
        assert!(calls[1].input["validationIssues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "UNIT_SCENE_COORDINATE"
                && issue["expectedSceneId"] == source.scenes[0].id));
        assert!(calls[1].rule_snapshot["runtimeContract"]["content"]
            .as_str()
            .unwrap()
            .contains("不能仅换场号却保留不适用素材"));
        let rejected = exec.rejected_values.lock().await.clone();
        assert_eq!(rejected.len(), 1);
        let bad = &rejected[0].1;
        assert_eq!(
            bad["replacements"][0]["units"][0]["sceneId"],
            source.scenes[1].id
        );
        assert_eq!(output["units"][0]["sceneId"], source.scenes[0].id);
        assert_eq!(
            output["authorRepairProvenance"]["nodes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(exec
            .rejected
            .lock()
            .await
            .iter()
            .any(|error| error.contains("单元来源跨集/跨场或包含非制作资料")));
        assert_eq!(
            repair_author_scope(
                &source,
                &raw,
                &origin,
                &scope,
                issues,
                &plan,
                &rows,
                &[],
                assets,
                &[],
                &source.episodes[0].id,
                "offline",
                "medium",
                None,
                &origin.input["capabilities"],
                "16:9真人",
                &origin.input,
                &exec
            )
            .await
            .unwrap(),
            output
        );
        assert_eq!(
            exec.calls.lock().await.len(),
            2,
            "restoring the rejected first coordinate response cannot create a third request"
        );
    }

    #[test]
    fn editor_coordinate_candidate_does_not_weaken_unknown_cross_scene_quotation_or_adoption_gates()
    {
        let (source, origin, scope, raw, plan, _) = same_episode_coordinate_fixture();
        let assets = origin.input["assets"].as_array().unwrap();
        let caps = &origin.input["capabilities"];
        let prefix = format!("{}:", source.episodes[0].id);
        assert_eq!(plan["events"][0]["sceneId"], source.scenes[0].id);
        assert!(scope.batch.source_refs.iter().all(|id| source
            .rows
            .iter()
            .find(|row| &row.id == id)
            .unwrap()
            .scene_id
            .as_deref()
            == Some(source.scenes[0].id.as_str())));
        let mut wrong = raw.clone();
        wrong["units"][0]["assetRefs"] = json!([]);
        wrong["units"][0]["sceneId"] = json!(source.scenes[1].id);
        assert!(
            author_scope_candidate(
                &source,
                &wrong,
                assets,
                caps,
                &scope.batch.source_refs,
                "16:9真人",
                &prefix,
                &plan,
                &[],
                true
            )
            .is_err(),
            "an original Author cannot defer source coordinates"
        );
        let (candidate, issues) = scope_repair_candidate(
            &source,
            &wrong,
            assets,
            caps,
            &scope.batch.source_refs,
            "16:9真人",
            &prefix,
            &plan,
            &[],
            true,
            true,
        )
        .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["code"], "UNIT_SCENE_COORDINATE");
        assert!(
            validate_storyboard_scope(
                &source,
                &candidate,
                assets,
                caps,
                Some(&scope.batch.source_refs)
            )
            .is_err(),
            "the candidate is never an adopted draft"
        );
        for gate in [
            "unknownScene",
            "foreignEpisode",
            "unknownRow",
            "mixedScenes",
            "quote",
            "eventFollowsWrongUnit",
            "eventWrongWhileUnitCorrect",
        ] {
            let mut invalid = wrong.clone();
            let mut invalid_plan = plan.clone();
            match gate {
                "unknownScene" => invalid["units"][0]["sceneId"] = json!("not-a-known-scene"),
                "foreignEpisode" => invalid["units"][0]["episodeId"] = json!("foreign-episode"),
                "unknownRow" => invalid["units"][0]["sourceRefs"][0] = json!("not-a-source-row"),
                "mixedScenes" => invalid["units"][0]["sourceRefs"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(
                        source
                            .rows
                            .iter()
                            .find(|row| row.kind == "body"
                                && row.scene_id.as_deref() == Some(source.scenes[1].id.as_str()))
                            .unwrap()
                            .id
                    )),
                "quote" => {
                    invalid["units"][0]["shots"][0]["dialogueParts"][0]["text"] =
                        json!("非法更改台词")
                }
                "eventFollowsWrongUnit" => {
                    invalid_plan["events"][0]["sceneId"] = json!(source.scenes[1].id);
                    assert_eq!(
                        invalid["units"][0]["sceneId"],
                        invalid_plan["events"][0]["sceneId"]
                    );
                }
                _ => {
                    invalid["units"][0]["sceneId"] = json!(source.scenes[0].id);
                    invalid_plan["events"][0]["sceneId"] = json!(source.scenes[1].id);
                }
            }
            assert!(
                scope_repair_candidate(
                    &source,
                    &invalid,
                    assets,
                    caps,
                    &scope.batch.source_refs,
                    "16:9真人",
                    &prefix,
                    &invalid_plan,
                    &[],
                    true,
                    true
                )
                .is_err(),
                "{gate}"
            );
        }
    }

    #[test]
    #[ignore = "explicit private read-only rejected Finish fixtures; no model or production writes"]
    fn readonly_rejected_finisher_fixture_checks_scope_and_known_scene_coordinates_without_adoption(
    ) {
        let path = std::env::var("DIRECTOR_STAGE3_SCOPE_READONLY_DB")
            .expect("explicit DIRECTOR_STAGE3_SCOPE_READONLY_DB");
        let workflow = std::env::var("DIRECTOR_STAGE3_SCOPE_READONLY_WORKFLOW")
            .expect("explicit DIRECTOR_STAGE3_SCOPE_READONLY_WORKFLOW");
        let job_ids = std::env::var("DIRECTOR_STAGE3_FINISH_GATE_READONLY_JOBS")
            .expect("explicit comma separated DIRECTOR_STAGE3_FINISH_GATE_READONLY_JOBS");
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let snapshot:String=connection.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[&workflow],|r|r.get(0)).unwrap();
        let workflow_snapshot: Value = serde_json::from_str(&snapshot).unwrap();
        let source: LockedSource =
            serde_json::from_value(workflow_snapshot["input"]["source"].clone()).unwrap();
        let source = upgrade_source_dialogues(&source).unwrap();
        let assets = workflow_snapshot["input"]["assets"].as_array().unwrap();
        let mut coordinate_count = 0;
        let mut scope_count = 0;
        let mut checked = 0;
        for job in job_ids
            .split(',')
            .map(str::trim)
            .filter(|job| !job.is_empty())
        {
            let (snapshot,raw,hash,event):(String,String,String,String)=connection.query_row("SELECT j.snapshot,j.result,w.request_hash,e.data FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id JOIN events e ON e.id=(SELECT MAX(x.id) FROM events x WHERE x.job_id=j.id AND x.type='text_received_raw') WHERE j.id=?1 AND w.workflow_id=?2 AND w.stage='storyboardFinish' AND w.state='rejected' AND j.kind='text' AND j.state='rejected' AND j.error=w.error AND j.result=w.result AND json_extract(j.receipt,'$.validation')='rejected' AND json_extract(j.receipt,'$.contract')='desktop-runner/v1'",rusqlite::params![job,workflow],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(digest(&snapshot), hash);
            let request: TextRequest = serde_json::from_str(&snapshot).unwrap();
            let raw_value: Value = serde_json::from_str(&raw).unwrap();
            let event: Value = serde_json::from_str(&event).unwrap();
            assert_eq!(event["requestHash"], hash);
            assert!(event["value"] == raw_value);
            assert_eq!(raw_value["baseDraftHash"], request.input["baseDraftHash"]);
            let units: Vec<_> = raw_value["replacements"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| group["units"].as_array().unwrap())
                .cloned()
                .collect();
            assert!(!units.is_empty());
            let selected: HashSet<_> = units
                .iter()
                .flat_map(|unit| strings(unit, "sourceRefs").unwrap())
                .collect();
            let refs: Vec<_> = source
                .rows
                .iter()
                .filter(|row| selected.contains(&row.id))
                .map(|row| row.id.clone())
                .collect();
            let mut draft = json!({"schemaVersion":"storyboard-draft/v2","units":units,"fantasyRealization":[],"revisionNotes":[],"unresolved":[]});
            materialize_dialogue(&source, &mut draft, false).unwrap();
            materialize_asset_references(&mut draft, assets).unwrap();
            let mut issues = Vec::new();
            validate_storyboard_repair_scope(
                &source,
                &draft,
                assets,
                &request.input["capabilities"],
                Some(&refs),
                Some(&mut issues),
                true,
                false,
            )
            .unwrap();
            validate_event_bindings_for_repair(&draft, &request.input["plan"], Some(&issues))
                .unwrap();
            assert!(
                !issues.is_empty(),
                "the original rejected fixture must remain unadopted"
            );
            assert!(validate_storyboard_scope(
                &source,
                &draft,
                assets,
                &request.input["capabilities"],
                Some(&refs)
            )
            .is_err());
            coordinate_count += issues
                .iter()
                .filter(|issue| issue["code"] == "UNIT_SCENE_COORDINATE")
                .count();
            scope_count += issues
                .iter()
                .filter(|issue| issue["code"] == "ASSET_SCENE_SCOPE")
                .count();
            let after: (String, String) = connection
                .query_row("SELECT snapshot,result FROM jobs WHERE id=?1", [job], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert!(
                after.0 == snapshot && after.1 == raw,
                "read-only fixture must preserve the frozen request and received raw"
            );
            checked += 1;
        }
        assert!(checked > 0);
        assert!(coordinate_count > 0 || scope_count > 0);
        println!("readOnly=true checkedFixtures={checked} coordinateIssues={coordinate_count} applicabilityIssues={scope_count} newModelCalls=0 adopted=0");
    }

    #[tokio::test]
    async fn author_scope_repair_adopts_only_the_finisher_and_carries_its_real_tail_to_next_author()
    {
        let (source, request) = author_scope_input(6);
        let exec = Recorded {
            author_scope_response: Some("FIX"),
            rough_per_row: true,
            ..Default::default()
        };
        let output = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        validate_storyboard_scope(
            &source,
            &output,
            request["assets"].as_array().unwrap(),
            &request["capabilities"],
            None,
        )
        .unwrap();
        let calls = exec.calls.lock().await.clone();
        let authors: Vec<_> = calls
            .iter()
            .filter(|r| r.stage == "storyboardCreate")
            .collect();
        assert_eq!(authors.len(), 2);
        assert_eq!(
            authors[1].input["acceptedContinuity"]["unit"]["exitState"],
            "原整编已验收真实末态。"
        );
        assert!(authors[1].input["acceptedContinuity"]["unit"]["assetRefs"]
            .as_array()
            .unwrap()
            .is_empty());
        let repairs: Vec<_> = calls
            .iter()
            .filter(|r| is_author_repair_request(r))
            .collect();
        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0].input["authorRepair"]["maximumAttempts"], 2);
        assert_eq!(
            repairs[0].input["validationIssues"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(repairs[0].rule_snapshot["runtimeContract"]["content"]
            .as_str()
            .unwrap()
            .contains("尚未通过完整领域验收"));
        assert!(
            calls
                .iter()
                .any(|r| r.stage == "storyboardFinish" && !is_author_repair_request(r)),
            "the ordinary associated editor still runs"
        );
        let cache = exec.accepted.lock().await;
        assert!(
            !cache.contains_key(&serde_json::to_string(authors[0]).unwrap()),
            "invalid author must remain unadopted"
        );
        assert!(cache.contains_key(&serde_json::to_string(authors[1]).unwrap()));
        assert!(cache.contains_key(&serde_json::to_string(repairs[0]).unwrap()));
        drop(cache);
        let rejected = exec.rejected_values.lock().await.clone();
        let original = rejected
            .iter()
            .find(|(r, _)| r.stage == "storyboardCreate")
            .unwrap();
        assert_eq!(
            original.1["units"].as_array().unwrap().last().unwrap()["assetRefs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(exec.publications.lock().await.len(), 1);
        let before = calls.len();
        assert_eq!(
            run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap(),
            output
        );
        assert_eq!(
            exec.calls.lock().await.len(),
            before,
            "resume revalidates raw and reuses the repaired editor without a new author or planner"
        );
    }

    #[tokio::test]
    async fn repaired_author_dependency_branch_reaches_related_finisher_and_reuses_its_cache() {
        let (source, request) = author_scope_input(6);
        let executor = Recorded {
            author_scope_response: Some("FIX"),
            rough_per_row: true,
            author_chain_branch: true,
            ..Default::default()
        };
        let output = run_storyboard(&request, &executor, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        validate_storyboard_scope(
            &source,
            &output,
            request["assets"].as_array().unwrap(),
            &request["capabilities"],
            None,
        )
        .unwrap();
        let calls = executor.calls.lock().await.clone();
        let authors: Vec<_> = calls
            .iter()
            .filter(|r| r.stage == "storyboardCreate")
            .collect();
        assert!(authors.len() > 1);
        assert!(authors[0].input.get("authorChainRevision").is_none());
        assert_eq!(
            authors[1].input["authorChainRevision"]["version"],
            "accepted-author-chain-change/v1"
        );
        assert!(calls.iter().any(|r| r.stage == "storyboardFinish"
            && !is_author_repair_request(r)
            && r.input["authorChainRevision"]["version"] == "accepted-author-chain-change/v1"));
        assert_eq!(
            run_storyboard(&request, &executor, "offline", "medium")
                .await
                .unwrap(),
            output
        );
        assert_eq!(
            executor.calls.lock().await.len(),
            calls.len(),
            "resume must reuse the same repaired dependency branch"
        );
    }

    fn is_author_repair_request(request: &TextRequest) -> bool {
        request.input["executionBatch"]["version"]
            == creative::AUTHOR_SCOPE_REPAIR_EXECUTION_VERSION
    }

    #[tokio::test]
    async fn author_scope_repair_has_two_persistent_attempts_and_same_problem_stops_after_one() {
        let (_, request) = author_scope_input(2);
        for (mode, expected, status) in [
            ("NO_PROGRESS", 1, "partially_blocked"),
            ("PARTIAL", 2, "completed"),
            ("LIMIT", 2, "partially_blocked"),
        ] {
            let exec = Recorded {
                author_scope_response: Some(mode),
                ..Default::default()
            };
            let output = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(output["status"], status, "{}", output["blocked"]);
            let calls = exec.calls.lock().await.clone();
            let repairs: Vec<_> = calls
                .iter()
                .filter(|r| is_author_repair_request(r))
                .collect();
            assert_eq!(repairs.len(), expected);
            if mode == "NO_PROGRESS" {
                assert!(output["blocked"]
                    .to_string()
                    .contains("AUTHOR_SCOPE_REPAIR_NO_PROGRESS"));
                assert!(exec.publications.lock().await.is_empty());
            } else {
                assert_ne!(
                    repairs[0].input["authorRepair"]["problemSignature"],
                    repairs[1].input["authorRepair"]["problemSignature"]
                );
                assert_eq!(repairs[1].input["authorRepair"]["attempt"], 2);
            }
            if mode == "LIMIT" {
                assert!(output["blocked"]
                    .to_string()
                    .contains("AUTHOR_SCOPE_REPAIR_LIMIT"));
                assert!(exec.publications.lock().await.is_empty());
            }
            let before = calls.len();
            run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(exec.calls.lock().await.len(), before);
        }
    }

    #[tokio::test]
    async fn author_scope_issue_does_not_hide_hard_quote_or_identity_errors_or_allow_foreign_revision(
    ) {
        let (_, request) = author_scope_input(2);
        for mode in [
            "HARD_AUTHOR_UNKNOWN",
            "HARD_AUTHOR_QUOTE",
            "BAD_QUOTE",
            "FOREIGN",
        ] {
            let exec = Recorded {
                author_scope_response: Some(mode),
                ..Default::default()
            };
            let output = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(output["status"], "partially_blocked");
            assert!(output["units"].as_array().unwrap().is_empty());
            assert!(exec.publications.lock().await.is_empty());
            let calls = exec.calls.lock().await;
            assert_eq!(
                calls.iter().filter(|r| is_author_repair_request(r)).count(),
                if mode.starts_with("HARD_AUTHOR") {
                    0
                } else {
                    1
                }
            );
            assert!(exec.accepted.lock().await.keys().all(
                |key| serde_json::from_str::<TextRequest>(key).unwrap().stage == "storyboardPlan"
            ));
        }
    }

    #[test]
    fn author_scope_probe_defers_only_applicability_not_source_event_fantasy_or_quotation() {
        let (source, input) = author_scope_input(1);
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .cloned()
            .collect();
        let request = TextRequest {
            stage: "storyboardCreate".into(),
            skill: "unit-performance-director".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"sourceRows":rows,"workPackage":{"eventId":"E1"}}),
            schema: json!({}),
            rule_snapshot: json!({}),
        };
        let plan = mock_plan(&request, false);
        let mut raw = mock_draft(&request, false);
        raw["units"][0]["assetRefs"] = json!([{"assetId":input["assets"][0]["id"],"variantId":"base","order":1,"use":"character","sourceRefs":raw["units"][0]["sourceRefs"]}]);
        let refs: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
        let assets = input["assets"].as_array().unwrap();
        let original = raw.clone();
        let (_, issues) = author_scope_candidate(
            &source,
            &raw,
            assets,
            &input["capabilities"],
            &refs,
            "16:9真人",
            "ep:",
            &plan,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(raw, original);
        // Reference order is compiled by the existing program after AUDIO
        // extraction, not trusted from an author raw. This is not a relaxable
        // identity/scope error and cannot be used as the hard-gate fixture.
        let mut unordered = raw.clone();
        unordered["units"][0]["assetRefs"][0]["order"] = json!(0);
        let (ordered, scope_issues) = author_scope_candidate(
            &source,
            &unordered,
            assets,
            &input["capabilities"],
            &refs,
            "16:9真人",
            "ep:",
            &plan,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(ordered["units"][0]["assetRefs"][0]["order"], 1);
        assert_eq!(
            scope_issues.len(),
            1,
            "program numbering cannot clear the actual applicability error"
        );
        for problem in [
            "unknownAsset",
            "source",
            "event",
            "fantasy",
            "quote",
            "duplicateIdentity",
        ] {
            let mut bad = raw.clone();
            match problem {
                "unknownAsset" => bad["units"][0]["assetRefs"][0]["assetId"] = json!("unknown"),
                "source" => bad["units"][0]["shots"][0]["sourceRefs"] = json!(["unknown-row"]),
                "event" => bad["units"][0]["eventId"] = json!("unknown-event"),
                "fantasy" => {
                    bad["fantasyRealization"] = json!([{"candidateId":"not-selected","unitIds":["U0"],"shotIds":["S0"],"status":"realized","domain":"unsupported","entry":"入","exit":"出","screenSeconds":1,"preservedFunction":"原话"}])
                }
                "quote" => {
                    bad["units"][0]["shots"][0]["dialogueParts"][0]["text"] = json!("改写原话")
                }
                _ => {
                    let same_identity = bad["units"][0]["assetRefs"][0].clone();
                    bad["units"][0]["assetRefs"]
                        .as_array_mut()
                        .unwrap()
                        .push(same_identity);
                }
            }
            assert!(
                author_scope_candidate(
                    &source,
                    &bad,
                    assets,
                    &input["capabilities"],
                    &refs,
                    "16:9真人",
                    "ep:",
                    &plan,
                    &[],
                    true
                )
                .is_err(),
                "{problem} must remain a hard gate even when an earlier asset scope issue exists"
            );
        }
    }

    #[test]
    fn author_scope_no_progress_ignores_cosmetic_unit_names_orders_and_reference_order() {
        let first = json!({"code":"ASSET_SCENE_SCOPE","unitId":"old-unit","assetId":"stable-asset","variantId":"stable-variant","episodeId":"episode","sceneId":"scene","sourceRefs":["r1","r2"],"coordinate":"old-unit.assetRefs[0]"});
        let mut renamed = first.clone();
        renamed["unitId"] = json!("new-unit");
        renamed["coordinate"] = json!("new-unit.assetRefs[1]");
        renamed["sourceRefs"] = json!(["r2", "r1"]);
        assert_eq!(
            author_scope_problem_signature(&[first.clone()]),
            author_scope_problem_signature(&[renamed])
        );
        let mut changed = first.clone();
        changed["sceneId"] = json!("different-scene");
        assert_ne!(
            author_scope_problem_signature(&[first]),
            author_scope_problem_signature(&[changed])
        );
    }

    #[test]
    #[ignore = "explicit ENV-bound original rejected author; read-only, no model or writes"]
    fn saved_rejected_author_scope_raw_passes_only_the_hard_gate_without_adoption() {
        let path = std::env::var("DIRECTOR_AUTHOR_SCOPE_READONLY_DB")
            .expect("set DIRECTOR_AUTHOR_SCOPE_READONLY_DB");
        let workflow = std::env::var("DIRECTOR_AUTHOR_SCOPE_READONLY_WORKFLOW")
            .expect("set DIRECTOR_AUTHOR_SCOPE_READONLY_WORKFLOW");
        let job = std::env::var("DIRECTOR_AUTHOR_SCOPE_READONLY_JOB")
            .expect("set DIRECTOR_AUTHOR_SCOPE_READONLY_JOB");
        let c = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let (snapshot,raw,event,workflow_snapshot):(String,String,String,String)=c.query_row("SELECT j.snapshot,j.result,e.data,parent.snapshot FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id JOIN jobs parent ON parent.id=w.workflow_id JOIN events e ON e.id=(SELECT MAX(x.id) FROM events x WHERE x.job_id=j.id AND x.type='text_received_raw') WHERE j.id=?1 AND w.workflow_id=?2 AND j.kind='text' AND j.state='rejected' AND j.error=?3 AND w.state='rejected' AND w.stage='storyboardCreate' AND w.error=j.error AND w.result=j.result AND json_extract(j.receipt,'$.validation')='rejected' AND parent.kind='workflow' AND parent.entity_id='storyboard.run'",rusqlite::params![job,workflow,crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        let request: TextRequest = serde_json::from_str(&snapshot).unwrap();
        let raw: Value = serde_json::from_str(&raw).unwrap();
        let event: Value = serde_json::from_str(&event).unwrap();
        let run: Value = serde_json::from_str(&workflow_snapshot).unwrap();
        assert_eq!(event["requestHash"], digest(&snapshot));
        assert!(
            event["value"] == raw,
            "original receive evidence must equal the rejected raw without exposing its text"
        );
        assert_eq!(request.stage, "storyboardCreate");
        assert_eq!(request.skill, "unit-performance-director");
        assert_eq!(request.input[creative::PROFILE_FIELD], creative::PROFILE);
        let source: LockedSource = serde_json::from_value(run["input"]["source"].clone()).unwrap();
        let source = upgrade_source_dialogues(&source).unwrap();
        validate_source(&source).unwrap();
        let assets = episode_assets(
            run["input"]["assets"].as_array().unwrap(),
            text(&request.input, "episodeId"),
        );
        let refs = strings(&request.input["executionBatch"], "sourceRefs").unwrap();
        let prefix = format!(
            "{}:{}:",
            text(&request.input, "episodeId"),
            text(&request.input["executionBatch"], "id")
        );
        let selected = request.input["selectedFantasy"].as_array().unwrap();
        let original = raw.clone();
        let mut strict = raw.clone();
        assert_eq!(
            validate_authored(
                &source,
                &mut strict,
                &assets,
                &request.input["capabilities"],
                &refs,
                text(&request.input["settings"], "visualStyle"),
                &prefix
            )
            .unwrap_err(),
            crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION
        );
        let (candidate, issues) = author_scope_candidate(
            &source,
            &raw,
            &assets,
            &request.input["capabilities"],
            &refs,
            text(&request.input["settings"], "visualStyle"),
            &prefix,
            &request.input["plan"],
            selected,
            true,
        )
        .unwrap();
        assert!(!issues.is_empty());
        assert!(
            raw == original,
            "hard-gate replay cannot rewrite the original raw"
        );
        for issue in &issues {
            let asset = assets
                .iter()
                .find(|a| a["id"] == issue["assetId"] && a["variantId"] == issue["variantId"])
                .unwrap();
            assert_eq!(issue["declaredApplicability"], asset["applicability"]);
            assert!(strings(issue, "sourceRefs")
                .unwrap()
                .iter()
                .all(|id| refs.contains(id)));
            assert!(candidate["units"]
                .as_array()
                .unwrap()
                .iter()
                .any(|unit| unit["id"] == issue["unitId"] && unit["sceneId"] == issue["sceneId"]));
        }
        let evidence = json!({"version":"rejected-author-scope-hard-gate/v1","readOnly":true,"sourceSha256":source.sha256,"sourceRows":refs.len(),"units":candidate["units"].as_array().unwrap().len(),"scopeIssues":issues.len(),"knownCanonicalAssetVariants":true,"sourceDialogueIdentityOrderCoverageVerified":true,"originalRawUnchanged":raw==original,"originalAuthorAccepted":false,"modelCalls":0,"videoSubmissions":0});
        if let Ok(report) = std::env::var("DIRECTOR_AUTHOR_SCOPE_READONLY_REPORT") {
            std::fs::write(report, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
        println!("{}", evidence);
    }

    #[tokio::test]
    async fn complete_episode_publishes_before_the_next_plan_and_never_publishes_partial_authors() {
        let source = prepare_source(
            "第一集\n场景：门口\n甲：保留第一集。\n第二集\n场景：院内\n甲：保留第二集。\n",
        )
        .unwrap();
        let exec = Recorded {
            fail_episode: source.episodes[1].id.clone(),
            ..Default::default()
        };
        let output = run_storyboard(&input(&source), &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "partially_blocked");
        let publications = exec.publications.lock().await;
        assert_eq!(publications.len(), 1);
        let published = &publications[0]["output"];
        assert_eq!(published["episodeId"], source.episodes[0].id);
        validate_episode_delivery(&input(&source), published).unwrap();
        assert!(publications[0]["callsAtPublication"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["episodeId"] == source.episodes[0].id));
        assert_eq!(
            publications[0]["callsAtPublication"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["stage"],
            "storyboardFinish"
        );
        assert_eq!(output["updatedEpisodeIds"], json!([source.episodes[0].id]));
        drop(publications);
        let failed = Recorded {
            fail_finish: true,
            ..Default::default()
        };
        run_storyboard(&input(&source), &failed, "offline", "medium")
            .await
            .unwrap();
        assert!(
            failed.publications.lock().await.is_empty(),
            "accepted authors are never published as a complete episode"
        );
    }

    #[tokio::test]
    async fn accepted_fractional_finish_uses_only_one_new_delta_editor_then_replays_every_cache() {
        let source =
            prepare_source("第一集\n场景：门口\n甲：原话完整保留。\n甲：未改第二单元。\n").unwrap();
        let mut request = input(&source);
        request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        request["capabilities"]["maxDuration"] = json!(5);
        let exec = Recorded {
            fractional_editor: true,
            ..Default::default()
        };
        let output = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        assert!(carrier_problems(&output, &request["capabilities"]).is_empty());
        let calls = exec.calls.lock().await.clone();
        assert_eq!(
            calls.iter().filter(|r| r.stage == "storyboardPlan").count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|r| r.stage == "storyboardCreate")
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|r| r.stage == "storyboardFinish")
                .count(),
            2
        );
        let origin = calls
            .iter()
            .find(|r| {
                r.stage == "storyboardFinish"
                    && r.input["executionBatch"]["version"] != "carrier-repair/v1"
            })
            .unwrap();
        let repair = calls
            .iter()
            .find(|r| r.input["executionBatch"]["version"] == "carrier-repair/v1")
            .unwrap();
        assert!(
            origin.input.get(creative::CARRIER_PROFILE_FIELD).is_none(),
            "old accepted request remains unmarked"
        );
        assert_eq!(
            repair.input["carrierRepair"]["originDomainRequestSha256"],
            digest(&serde_json::to_string(origin).unwrap())
        );
        assert_eq!(repair.input["carrierProblems"].as_array().unwrap().len(), 1);
        assert!(text(&repair.rule_snapshot["runtimeContract"], "content").contains("正整秒"));
        let origin_result =
            exec.accepted.lock().await[&serde_json::to_string(origin).unwrap()].clone();
        assert_eq!(
            origin_result["materializedDraft"]["units"][0]["duration"], 3.5,
            "original paid result is retained unchanged"
        );
        let orig_units = origin_result["materializedDraft"]["units"]
            .as_array()
            .unwrap();
        assert_eq!(
            output["units"][1], orig_units[1],
            "unaffected unit stays byte-for-byte intact"
        );
        validate_episode_delivery(&request, &exec.publications.lock().await[0]["output"]).unwrap();
        let repeated = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(repeated, output);
        assert_eq!(
            exec.calls.lock().await.len(),
            calls.len(),
            "accepted Planner, Author, original Editor and repair all reuse exact caches"
        );
    }

    #[tokio::test]
    async fn carrier_repairs_stop_on_no_progress_or_two_attempts_and_never_publish_invalid_source()
    {
        let source = prepare_source("第一集\n场景：门口\n甲：原句不可裁。\n").unwrap();
        let mut request = input(&source);
        request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        for (mode, expected, count) in [
            ("NO_PROGRESS", "VIDEO_CARRIER_REPAIR_NO_PROGRESS", 1),
            ("MOVING", "VIDEO_CARRIER_REPAIR_LIMIT", 2),
            ("BAD_QUOTE", "传入对白与原话区间不一致", 2),
        ] {
            let exec = Recorded {
                fractional_editor: true,
                carrier_response: Some(mode),
                ..Default::default()
            };
            let result = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(result["status"], "partially_blocked");
            assert!(
                result["blocked"].to_string().contains(expected),
                "{}",
                result["blocked"]
            );
            assert_eq!(
                exec.calls
                    .lock()
                    .await
                    .iter()
                    .filter(|r| r.input["executionBatch"]["version"] == "carrier-repair/v1")
                    .count(),
                count
            );
            assert!(exec.publications.lock().await.is_empty());
            assert!(exec
                .accepted
                .lock()
                .await
                .values()
                .any(|v| v["materializedDraft"]["units"][0]["duration"] == 3.5));
        }
    }

    #[tokio::test]
    async fn carrier_over_maximum_uses_finisher_delta_not_clipping_and_keeps_the_accepted_origin() {
        let source = prepare_source("第一集\n场景：门口\n甲：全字全句保留。\n").unwrap();
        let mut request = input(&source);
        request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        let exec = Recorded {
            fractional_editor: true,
            editor_duration: Some(16.0),
            ..Default::default()
        };
        let output = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        assert_eq!(
            output["units"][0]["duration"], 4.0,
            "only the editor's actual replacement changes the timeline"
        );
        assert!(exec
            .accepted
            .lock()
            .await
            .values()
            .any(|v| v["materializedDraft"]["units"][0]["duration"] == 16.0));
        let calls = exec.calls.lock().await;
        let repair = calls
            .iter()
            .find(|r| r.input["executionBatch"]["version"] == "carrier-repair/v1")
            .unwrap();
        assert_eq!(
            repair.input["carrierProblems"][0]["codes"],
            json!(["ABOVE_CHANNEL_MAXIMUM"])
        );
        assert_eq!(
            repair.input["previousDraft"]["units"][0]["shots"][0]["duration"],
            16.0
        );
        assert_eq!(
            output["units"][0]["shots"][0]["dialogueParts"][0]["text"],
            "全字全句保留。"
        );
        let mut renamed = output.clone();
        renamed["units"][0]["id"] = json!("other-id");
        renamed["units"][0]["duration"] = json!(3.5);
        let mut same = renamed.clone();
        same["units"][0]["id"] = json!("another-id");
        assert_eq!(
            carrier_problem_signature(
                &renamed,
                &carrier_problems(&renamed, &request["capabilities"])
            ),
            carrier_problem_signature(&same, &carrier_problems(&same, &request["capabilities"]))
        );
    }

    #[tokio::test]
    async fn new_carrier_profile_frontloads_all_three_roles_without_altering_old_requests() {
        let source = prepare_source("第一集\n场景：门口\n甲：完整原话。\n").unwrap();
        let mut old = input(&source);
        old[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        let old_exec = Recorded::default();
        run_storyboard(&old, &old_exec, "offline", "medium")
            .await
            .unwrap();
        let mut new = old.clone();
        new[creative::CARRIER_PROFILE_FIELD] = json!(creative::CARRIER_PROFILE);
        let new_exec = Recorded::default();
        run_storyboard(&new, &new_exec, "offline", "medium")
            .await
            .unwrap();
        let originals = old_exec.calls.lock().await;
        let fresh = new_exec.calls.lock().await;
        assert_eq!(originals.len(), 3);
        assert_eq!(fresh.len(), 3);
        for (a, b) in originals.iter().zip(fresh.iter()) {
            assert!(a.input.get(creative::CARRIER_PROFILE_FIELD).is_none());
            assert_eq!(
                b.input[creative::CARRIER_PROFILE_FIELD],
                creative::CARRIER_PROFILE
            );
            assert!(text(&b.rule_snapshot["runtimeContract"], "content")
                .starts_with(text(&a.rule_snapshot["runtimeContract"], "content")));
            assert_eq!(a.rule_snapshot["primary"], b.rule_snapshot["primary"]);
            assert_eq!(a.rule_snapshot["methods"], b.rule_snapshot["methods"]);
            assert_eq!(a.input["sourceRows"], b.input["sourceRows"]);
            assert_eq!(a.schema, b.schema);
        }
        let mut invalid = new;
        invalid[creative::CARRIER_PROFILE_FIELD] = json!("unknown");
        let rejected = Recorded::default();
        assert!(run_storyboard(&invalid, &rejected, "offline", "medium")
            .await
            .is_err());
        assert!(rejected.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn changed_carrier_tail_transparently_recomputes_the_affected_chain_and_reuses_it_on_resume(
    ) {
        let source = prepare_source(
            "第一集\n场景：门口\n甲：完整第一句。\n第二集\n场景：院内\n甲：完整第二句。\n",
        )
        .unwrap();
        let mut request = input(&source);
        request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        request["storyboardHistory"] = json!([{"stage":"storyboardPlan","status":"accepted","episodeId":source.episodes[1].id}]);
        let exec = Recorded {
            fractional_editor: true,
            ..Default::default()
        };
        let result = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{}", result["blocked"]);
        assert_eq!(exec.publications.lock().await.len(), 2);
        let updates = exec.continuity_updates.lock().await;
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0]["previouslyAcceptedPlan"], true);
        assert_eq!(updates[0]["action"], "recompute_affected_chain");
        drop(updates);
        let calls = exec.calls.lock().await.clone();
        let first_tail = exec.publications.lock().await[0]["output"]["units"][0].clone();
        for call in calls.iter().filter(|call| {
            call.input["episodeId"] == source.episodes[1].id
                || call.input["episode"]["id"] == source.episodes[1].id
        }) {
            assert_eq!(
                call.input["continuityRevision"]["version"],
                "accepted-tail-change/v1"
            );
            assert_eq!(
                call.input["continuityRevision"]["currentTailSha256"],
                digest(&first_tail.to_string())
            );
            if call.stage == "storyboardPlan" {
                assert_eq!(call.input["previousBoundary"], first_tail);
            }
        }
        let repeated = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(repeated, result);
        assert_eq!(exec.calls.lock().await.len(), calls.len());
    }

    #[tokio::test]
    async fn scoped_catalog_handoff_preserves_direction_episode_context_and_cached_facts_across_three_roles(
    ) {
        let source = prepare_source(
            "第一集\n场景：门口\n甲：本场原话。\n第二集\n场景：院内\n甲：后集原话。\n",
        )
        .unwrap();
        let body = source.rows.iter().find(|row| row.kind == "body").unwrap();
        let eid = &source.episodes[0].id;
        let mut request = input(&source);
        request["selection"] = json!({"episodeIds":[eid]});
        request["storyboardAssetInputProfile"] = json!(asset_cards::SCOPED_PROFILE);
        let episode_foundation = json!({"episodeId":eid,"summary":"正式本集概要","context":"保留本集有源制作前情，不把它当新剧情。","sourceRefs":[body.id]});
        let fact = json!({"id":"visual","assetId":"person","variantId":"base","attribute":"服装","value":"源中已限定的服装","claim":"SOURCE","scope":"STABLE","sourceRefs":[body.id]});
        request["foundation"]["episodes"] = json!([episode_foundation.clone()]);
        request["foundation"]["facts"] = json!([fact.clone()]);
        request["foundation"]["assets"] = json!([{"id":"person","name":"正式甲","aliases":["旧称"],"variants":[{"id":"base","assetRowId":"person:base"}]}]);
        request["assets"] = json!([{"id":"person:base","identityId":"person","baseAssetId":"person","variantId":"base","version":3,"name":"正式甲｜基础","kind":"CH","sourceRefs":[body.id],"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceSha256":source.sha256},
            "scope":{"kind":"SCENES","reason":null,"scenes":[{"episodeId":eid,"sceneId":source.scenes[0].id},{"episodeId":source.episodes[1].id,"sceneId":source.scenes[1].id}]},
            "applicability":{"episodes":[eid,source.episodes[1].id],"scenes":[source.scenes[0].id,source.scenes[1].id]},"preparation":{"mode":"REFERENCE"},"prompt":"不能重做资产设计的旧图片提示词"}]);
        let frozen = request.to_string();
        let executor = Recorded::default();
        let first = run_storyboard(&request, &executor, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(first["status"], "completed", "{}", first["blocked"]);
        let calls = executor.calls.lock().await.clone();
        assert_eq!(
            calls
                .iter()
                .map(|call| call.stage.as_str())
                .collect::<Vec<_>>(),
            vec!["storyboardPlan", "storyboardCreate", "storyboardFinish"]
        );
        for call in calls {
            assert_eq!(
                call.input["productionDirection"],
                production_direction(&request)
            );
            assert_eq!(call.input["foundationEpisode"], episode_foundation);
            assert_eq!(call.input["facts"], json!([fact.clone()]));
            assert_eq!(call.input["assetCatalogScope"]["episodeId"], json!(eid));
            assert_eq!(
                call.input["assetCatalogScope"]["sourceSha256"],
                source.sha256
            );
            let asset = &call.input["assets"][0];
            assert_eq!(asset["aliases"], json!(["旧称"]));
            assert_eq!(asset["id"], "person:base");
            assert_eq!(
                asset["scope"]["scenes"],
                json!([{"episodeId":eid,"sceneId":source.scenes[0].id}])
            );
            assert!(asset.get("applicability").is_none());
            assert!(asset.get("prompt").is_none());
            assert_eq!(asset["sourceRefs"], json!([body.id]));
            assert_eq!(
                call.rule_snapshot["primary"],
                rule_snapshot(&call.skill).unwrap(),
                "old Skill body stays authoritative and unchanged"
            );
        }
        let count = executor.calls.lock().await.len();
        let second = run_storyboard(&request, &executor, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            executor.calls.lock().await.len(),
            count,
            "accepted calls replay without another asset/planner/author model request"
        );
        assert_eq!(
            request.to_string(),
            frozen,
            "the entire frozen source/foundation/asset envelope remains exact"
        );
    }

    fn continuous_action_fixture() -> (LockedSource, Value, Vec<String>) {
        let source =
            prepare_source("第一集\n场景：门口\n甲：走开。\n△甲说完，转身离开。\n乙：等等。\n")
                .unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .collect();
        assert_eq!(rows.len(), 3);
        assert!(rows[1].dialogue.is_none());
        let refs: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
        let request = TextRequest {
            stage: "storyboardCreate".into(),
            skill: "unit-performance-director".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"sourceRows":rows,"workPackage":{"eventId":"E1"}}),
            schema: json!({}),
            rule_snapshot: json!({}),
        };
        let mut draft = mock_draft(&request, false);
        let mut units = draft["units"].as_array().unwrap().clone();
        let last_quote_shot = units.pop().unwrap()["shots"][0].clone();
        let first_refs = json!([refs[0], refs[1]]);
        units[0]["sourceRefs"] = first_refs.clone();
        units[0]["shots"][0]["sourceRefs"] = first_refs;
        units[0]["shots"][0]["picture"] =
            json!("甲面向乙说完原话，末字后闭口收回目光，头和肩开始转向门外，脚还没有迈出。");
        units[0]["shots"][0]["transition"] = json!("原话结束后，沿刚开始的头肩转动切到髋部接续。");
        units[0]["shots"][0]["dialogueParts"][0]["voiceEnd"] = json!(2.8);
        units[0]["exitState"] = json!("甲已说完原话，头肩开始转向门外，髋部和第一步尚未跟上。");
        units[1]["sourceRefs"] = json!([refs[1], refs[2]]);
        units[1]["initialState"] = units[0]["exitState"].clone();
        units[1]["shots"][0]["duration"] = json!(2.0);
        units[1]["shots"][0]["picture"] =
            json!("甲的髋部接续已经开始的头肩转动，衣摆随之转过，第一步踩向门外，乙抬眼追随。");
        units[1]["shots"][0]["transition"] = json!("以第一步落地接到乙开口，甲的离开动作继续。");
        units[1]["shots"]
            .as_array_mut()
            .unwrap()
            .push(last_quote_shot);
        units[1]["exitState"] = json!("甲已迈向门外，乙说完原话并看向甲的背影。");
        draft["units"] = json!(units);
        (source, draft, refs)
    }

    #[test]
    fn continuous_action_evidence_across_units_keeps_creative_body_and_once_only_source_coverage() {
        let (source, mut draft, refs) = continuous_action_fixture();
        let frozen_source = serde_json::to_value(&source).unwrap();
        let creative_body = |draft: &Value| {
            json!(draft["units"].as_array().unwrap().iter().map(|unit| json!({
                "sourceRefs":unit["sourceRefs"],"initialState":unit["initialState"],"exitState":unit["exitState"],
                "shots":unit["shots"].as_array().unwrap().iter().map(|shot| json!({
                    "id":shot["id"],"duration":shot["duration"],"camera":shot["camera"],"picture":shot["picture"],
                    "lighting":shot["lighting"],"sound":shot["sound"],"transition":shot["transition"],"sourceRefs":shot["sourceRefs"]
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>())
        };
        let before = creative_body(&draft);
        validate_authored(
            &source,
            &mut draft,
            &[],
            &json!({"minDuration":1,"maxDuration":30,"imageLimit":5}),
            &refs,
            "16:9真人",
            "",
        )
        .unwrap();
        assert_eq!(
            creative_body(&draft),
            before,
            "evidence folding must not rewrite an action or its continuation"
        );
        assert_eq!(serde_json::to_value(&source).unwrap(), frozen_source);
        let coverage: Vec<_> = draft["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|unit| unit["sourceCoverage"].as_array().unwrap())
            .collect();
        assert_eq!(
            coverage
                .iter()
                .map(|range| text(range, "rowId"))
                .collect::<Vec<_>>(),
            refs.iter().map(String::as_str).collect::<Vec<_>>()
        );
        for (range, row) in coverage
            .iter()
            .zip(source.rows.iter().filter(|row| row.kind == "body"))
        {
            assert_eq!(range["start"], row.range.start);
            assert_eq!(range["end"], row.range.end);
        }
        assert_eq!(
            draft["units"][0]["sourceRefs"][1],
            draft["units"][1]["sourceRefs"][0]
        );
        let spoken: Vec<_> = draft["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|unit| unit["shots"].as_array().unwrap())
            .flat_map(|shot| shot["dialogueParts"].as_array().unwrap())
            .map(|part| (text(part, "speaker"), text(part, "text")))
            .collect();
        assert_eq!(spoken, vec![("甲", "走开。"), ("乙", "等等。")]);
    }

    #[test]
    fn continuous_action_evidence_does_not_relax_dialogue_scope_window_or_order_checks() {
        let (source, draft, refs) = continuous_action_fixture();
        let mut cases = Vec::new();
        let mut repeated_quote = draft.clone();
        let first_quote = repeated_quote["units"][0]["shots"][0]["dialogueParts"][0].clone();
        repeated_quote["units"][1]["sourceRefs"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!(refs[0]));
        repeated_quote["units"][1]["shots"][0]["sourceRefs"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!(refs[0]));
        repeated_quote["units"][1]["shots"][0]["dialogueParts"]
            .as_array_mut()
            .unwrap()
            .push(first_quote);
        cases.push(("repeated dialogue", repeated_quote, "原话区间重复"));
        let mut bad_window = draft.clone();
        bad_window["units"][0]["shots"][0]["dialogueParts"][0]["voiceEnd"] = json!(3.1);
        cases.push(("voice window", bad_window, "声窗越出"));
        let mut unknown = draft.clone();
        unknown["units"][1]["sourceRefs"][0] = json!("unknown-row");
        cases.push(("unknown reference", unknown, "引用未知"));
        let mut wrong_scene = draft.clone();
        wrong_scene["units"][1]["sceneId"] = json!("unknown-scene");
        cases.push(("scene identity", wrong_scene, "身份不成立"));
        let mut missing = draft.clone();
        for unit in missing["units"].as_array_mut().unwrap() {
            unit["sourceRefs"]
                .as_array_mut()
                .unwrap()
                .retain(|id| id.as_str() != Some(refs[1].as_str()));
            for shot in unit["shots"].as_array_mut().unwrap() {
                shot["sourceRefs"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|id| id.as_str() != Some(refs[1].as_str()));
            }
        }
        // Keep the action shot's required evidence valid while omitting the action row.
        missing["units"][1]["shots"][0]["sourceRefs"] = json!([refs[2]]);
        cases.push(("missing source row", missing, "完整覆盖"));
        let mut reordered = draft.clone();
        reordered["units"].as_array_mut().unwrap().swap(0, 1);
        cases.push(("reordered actual speech", reordered, "原话顺序"));
        for (name, mut invalid, expected_error) in cases {
            let error = validate_authored(
                &source,
                &mut invalid,
                &[],
                &json!({"minDuration":1,"maxDuration":30,"imageLimit":5}),
                &refs,
                "16:9真人",
                "",
            )
            .expect_err(name);
            assert!(error.contains(expected_error), "{name}: {error}");
        }
    }

    #[test]
    fn repeated_evidence_materializes_locked_source_once_without_replaying_speech() {
        let (source, mut draft, refs) = continuous_action_fixture();
        for unit in draft["units"].as_array_mut().unwrap() {
            unit["sourceRefs"] = json!(refs);
        }
        let frozen_shots: Vec<_> = draft["units"]
            .as_array()
            .unwrap()
            .iter()
            .map(|unit| unit["shots"].clone())
            .collect();
        validate_authored(
            &source,
            &mut draft,
            &[],
            &json!({"minDuration":1,"maxDuration":30,"imageLimit":5}),
            &refs,
            "16:9真人",
            "",
        )
        .unwrap();
        let ranges: Vec<_> = draft["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|unit| unit["sourceCoverage"].as_array().unwrap())
            .collect();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| refs.contains(&row.id))
            .collect();
        assert_eq!(ranges.len(), rows.len());
        for (range, row) in ranges.iter().zip(rows) {
            assert_eq!(range["rowId"], row.id);
            assert_eq!(range["start"], row.range.start);
            assert_eq!(range["end"], row.range.end);
        }
        for (index, unit) in draft["units"].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                unit["sourceRefs"],
                json!(refs),
                "evidence citations remain immutable"
            );
            assert_eq!(
                unit["shots"].as_array().unwrap().len(),
                frozen_shots[index].as_array().unwrap().len()
            );
            for (shot, old) in unit["shots"]
                .as_array()
                .unwrap()
                .iter()
                .zip(frozen_shots[index].as_array().unwrap())
            {
                for field in [
                    "duration",
                    "camera",
                    "picture",
                    "lighting",
                    "sound",
                    "transition",
                    "sourceRefs",
                    "contextRefs",
                ] {
                    assert_eq!(
                        shot[field], old[field],
                        "actual performance remains unchanged"
                    );
                }
            }
        }
        let spoken: String = draft["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|unit| unit["shots"].as_array().unwrap())
            .flat_map(|shot| shot["dialogueParts"].as_array().unwrap())
            .map(|part| text(part, "text"))
            .collect();
        assert_eq!(spoken, "走开。等等。");
    }

    #[test]
    #[ignore = "explicit private read-only rejected source evidence; zero model/provider calls"]
    fn readonly_source_evidence_revalidates_original_body_and_delta_without_writing() {
        let path = std::env::var("DIRECTOR_EVIDENCE_READONLY_DB").expect("explicit read-only DB");
        let run = std::env::var("DIRECTOR_EVIDENCE_READONLY_WORKFLOW")
            .expect("explicit original workflow");
        let jobs =
            std::env::var("DIRECTOR_EVIDENCE_READONLY_JOBS").expect("explicit original jobs");
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let parent:String=connection.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|row|row.get(0)).unwrap();
        let input: Value = serde_json::from_str(&parent).unwrap();
        let source: LockedSource =
            serde_json::from_value(input["input"]["source"].clone()).unwrap();
        let source = upgrade_source_dialogues(&source).unwrap();
        let assets = input["input"]["assets"].as_array().unwrap();
        let mut checked = 0;
        for id in jobs.split(',').map(str::trim).filter(|id| !id.is_empty()) {
            let (saved,raw):(String,String)=connection.query_row("SELECT snapshot,result FROM jobs WHERE id=?1 AND kind='text' AND state='rejected'",[id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
            let request: TextRequest = serde_json::from_str(&saved).unwrap();
            let result: Value = serde_json::from_str(&raw).unwrap();
            let mut draft = if request.stage == "storyboardFinish" {
                // The persisted model view omits program exports, while the
                // opaque base hash refers to the complete runtime base. Rebuild
                // that base from the sealed original author for this first
                // repair, rather than weakening the delta hash check.
                assert_eq!(
                    request.input["authorRepair"]["attempt"], 1,
                    "this fixture requires the first sealed editor repair"
                );
                let mut statement=connection.prepare("SELECT snapshot,result,error FROM jobs WHERE kind='text' AND state='rejected' AND json_extract(snapshot,'$.stage')='storyboardCreate'").unwrap();
                let original = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .unwrap()
                    .filter_map(|row| {
                        let (snapshot, value, error) = row.unwrap();
                        let origin: TextRequest = serde_json::from_str(&snapshot).unwrap();
                        (digest(&serde_json::to_string(&origin).unwrap())
                            == request.input["authorRepair"]["originDomainRequestSha256"])
                            .then_some((
                                origin,
                                serde_json::from_str::<Value>(&value).unwrap(),
                                error,
                            ))
                    })
                    .next()
                    .expect("sealed original author");
                assert_eq!(
                    digest(&original.1.to_string()),
                    request.input["authorRepair"]["originalAuthorRawSha256"]
                );
                let prefix = format!(
                    "{}:{}:",
                    text(&original.0.input, "episodeId"),
                    text(&original.0.input["executionBatch"], "id")
                );
                let (candidate, _) = author_validation_candidate(
                    &source,
                    &original.1,
                    assets,
                    &original.0.input["capabilities"],
                    &strings(&original.0.input["executionBatch"], "sourceRefs").unwrap(),
                    text(&original.0.input["settings"], "visualStyle"),
                    &prefix,
                    &original.0.input["plan"],
                    original.0.input["selectedFantasy"].as_array().unwrap(),
                    &original.2,
                )
                .unwrap();
                let base = json!({"schemaVersion":"storyboard-draft/v2","units":candidate["units"],"fantasyRealization":candidate["fantasyRealization"],"revisionNotes":[],"unresolved":[]});
                assert_eq!(finish_base_hash(&base), request.input["baseDraftHash"]);
                apply_finish_delta(&base, &result).unwrap()
            } else {
                result
            };
            let refs = strings(&request.input["executionBatch"], "sourceRefs").unwrap();
            validate_authored(
                &source,
                &mut draft,
                assets,
                &request.input["capabilities"],
                &refs,
                text(&request.input["settings"], "visualStyle"),
                &format!("{}:", text(&request.input, "episodeId")),
            )
            .unwrap();
            validate_event_bindings(&draft, &request.input["plan"]).unwrap();
            validate_fantasy_profile(
                &draft,
                request.input["selectedFantasy"].as_array().unwrap(),
                true,
            )
            .unwrap();
            let ranges: Vec<_> = draft["units"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|unit| unit["sourceCoverage"].as_array().unwrap())
                .collect();
            for row in source.rows.iter().filter(|row| refs.contains(&row.id)) {
                let selected: Vec<_> = ranges
                    .iter()
                    .filter(|range| range["rowId"] == row.id)
                    .collect();
                assert!(!selected.is_empty());
                let mut cursor = row.range.start;
                for range in selected {
                    assert_eq!(range["start"], cursor);
                    cursor = range["end"].as_u64().unwrap() as usize;
                }
                assert_eq!(cursor, row.range.end);
            }
            let after: (String, String) = connection
                .query_row(
                    "SELECT snapshot,result FROM jobs WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(after, (saved, raw));
            checked += 1;
        }
        println!("readOnly=true checked={checked} originalSourceCoverageExact=true newModelCalls=0 databaseWrites=0");
    }

    #[tokio::test]
    async fn original_production_profile_restores_three_role_contracts_and_v3_without_touching_legacy_cache(
    ) {
        let source = prepare_source(
            "第一集\n场景：门口\n小王：我先把整件事说清楚，你不要打断我，听完之后再作出决定。\n",
        )
        .unwrap();
        let exec = Recorded {
            long_dialogue: true,
            ..Default::default()
        };
        let mut request = input(&source);
        request["settings"] = json!({"fantasyMode":"recommend"});
        request["capabilities"]["maxDuration"] = json!(30);
        request[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        let before = request.clone();
        let result = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{}", result["blocked"]);
        assert_eq!(request, before);
        assert_eq!(exec.calls.lock().await.len(), 3);
        let calls = exec.calls.lock().await.clone();
        for r in &calls {
            assert_eq!(r.input[creative::PROFILE_FIELD], creative::PROFILE);
            assert!(
                text(&r.input["settings"], "visualStyle").contains("明确支点、重心转移与接触关系")
            );
            let contract = &r.rule_snapshot["runtimeContract"];
            assert_eq!(contract["sha256"], digest(text(contract, "content")));
            assert!(!text(contract, "content").contains(&source.text));
            assert_eq!(
                r.rule_snapshot["primary"]["sha256"],
                rule_snapshot(&r.skill).unwrap()["sha256"]
            );
        }
        let author = calls
            .iter()
            .find(|r| r.stage == "storyboardCreate")
            .unwrap();
        assert!(text(&author.rule_snapshot["runtimeContract"], "content")
            .contains("人物表演与选中幻想在同一声画过程内共同设计"));
        assert_eq!(
            author.schema["properties"]["units"]["items"]["properties"]["shots"]["items"]
                ["properties"]["duration"],
            json!({"type":"number"})
        );
        let units = result["units"].as_array().unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0]["shots"].as_array().unwrap().len(), 2);
        assert_eq!(units[0]["duration"], 20.0);
        let dialogue: String = units
            .iter()
            .flat_map(|u| u["shots"].as_array().unwrap())
            .flat_map(|s| s["dialogueParts"].as_array().unwrap())
            .map(|p| text(p, "text"))
            .collect();
        assert_eq!(
            dialogue,
            spoken_text(
                source
                    .rows
                    .iter()
                    .find_map(|r| r.dialogue.as_ref())
                    .unwrap()
            )
        );
        assert!(text(&units[0], "prompt").contains("分镜1｜0-12s｜12s"));
        assert!(text(&units[0], "prompt").contains("12–19.5秒｜小王"));
        assert_eq!(units[0]["seedanceExportVersion"], "seedance-execution-v3");
        run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(
            exec.calls.lock().await.len(),
            3,
            "same new profile replays its own accepted cache"
        );
        let mut unknown = request.clone();
        unknown[creative::PROFILE_FIELD] = json!("future");
        assert!(run_storyboard(&unknown, &exec, "offline", "medium")
            .await
            .is_err());
        assert_eq!(exec.calls.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn dependency_exit_profile_preserves_full_continuity_source_and_exact_legacy_replay() {
        let source = prepare_source("第一集\n场景：门口\n小王：第一句。\n小王：第二句。\n场景：院内\n小王：第三句。\n小王：第四句。\n").unwrap();
        let body: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .collect();
        assert_eq!(body.len(), 4);
        let emotion = json!({"subject":"人物","contextTone":"当前源情境","audienceEffect":"真实回应可读","change":"依源事实回应","landing":"真实结果可见","retain":["源动作与原话"]});
        let first_refs = json!([body[0].id, body[1].id]);
        let second_refs = json!([body[2].id, body[3].id]);
        let plan = json!({"schemaVersion":"storyboard-plan/v2","emotionArc":"因果回应","unitPackingPolicy":"event-first-v1",
            "events":[
                {"id":"E1","sceneId":body[0].scene_id,"sourceRefs":first_refs,"boundaryBefore":{"kind":"episode_start","reason":"本集开始"},"emotionContract":emotion},
                {"id":"E2","sceneId":body[2].scene_id,"sourceRefs":second_refs,"boundaryBefore":{"kind":"time_space_jump","reason":"原文明确转入院内"},"emotionContract":emotion}],
            "workPackages":[
                {"id":"W1","eventId":"E1","sourceRefs":first_refs,"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":emotion},
                {"id":"W2","eventId":"E2","sourceRefs":second_refs,"contextRefs":[],"entryFactIds":[],"dependsOn":["W1"],"scenarioKeys":[],"emotionContract":emotion}],
            "provisionalUnits":[{"id":"P1","packageId":"W1","sourceRefs":first_refs,"secondsRange":[6,8]},{"id":"P2","packageId":"W2","sourceRefs":second_refs,"secondsRange":[6,8]}],
            "entryFacts":[],"fantasyCandidates":[]});
        let legacy = input(&source);
        let frozen_bytes = serde_json::to_vec(&legacy).unwrap();
        let legacy_exec = Recorded {
            plan_override: Some(plan.clone()),
            ..Default::default()
        };
        let old = run_storyboard(&legacy, &legacy_exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(old["status"], "completed", "{}", old["blocked"]);
        let old_calls = legacy_exec.calls.lock().await.clone();
        let old_authors: Vec<_> = old_calls
            .iter()
            .filter(|call| call.stage == "storyboardCreate")
            .collect();
        assert_eq!(
            old_authors.len(),
            2,
            "two source-owned dependency calls must remain separate"
        );
        assert_eq!(old_authors[0].input["plan"]["dependencyExits"], json!([]));
        let old_deps = &old_authors[1].input["dependencyOutputs"];
        assert_eq!(old_deps.as_array().unwrap().len(), 1);
        assert_eq!(old_authors[1].input["plan"]["dependencyExits"], *old_deps);
        assert!(
            old_deps[0]["units"][0]["shots"].is_array(),
            "unmarked requests keep the full historical dependency draft"
        );
        let old_bytes: Vec<_> = old_calls
            .iter()
            .map(|call| serde_json::to_vec(call).unwrap())
            .collect();
        let old_hashes: Vec<_> = old_calls
            .iter()
            .map(|call| digest(&serde_json::to_string(call).unwrap()))
            .collect();
        let replay = run_storyboard(&legacy, &legacy_exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(replay["units"], old["units"]);
        assert_eq!(
            legacy_exec.calls.lock().await.len(),
            old_calls.len(),
            "legacy accepted requests restore with zero new executor calls"
        );
        assert_eq!(serde_json::to_vec(&legacy).unwrap(), frozen_bytes);
        let cached = legacy_exec.accepted.lock().await;
        for (index, call) in old_calls.iter().enumerate() {
            let exact = serde_json::to_string(call).unwrap();
            assert!(cached.contains_key(&exact));
            assert_eq!(exact.as_bytes(), old_bytes[index]);
            assert_eq!(digest(&exact), old_hashes[index]);
        }
        drop(cached);

        let mut compact = legacy.clone();
        compact[storyboard_dependencies::PROFILE_FIELD] = json!(storyboard_dependencies::PROFILE);
        let frozen_compact = compact.clone();
        let compact_exec = Recorded {
            plan_override: Some(plan),
            ..Default::default()
        };
        let new = run_storyboard(&compact, &compact_exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(new["status"], "completed", "{}", new["blocked"]);
        assert_eq!(new["sourceSha256"], source.sha256);
        assert_eq!(
            new["units"], old["units"],
            "only the dependency input is projected, never the authored source or results"
        );
        let calls = compact_exec.calls.lock().await.clone();
        let authors: Vec<_> = calls
            .iter()
            .filter(|call| call.stage == "storyboardCreate")
            .collect();
        assert_eq!(authors.len(), 2);
        assert_eq!(calls[0].stage, "storyboardPlan");
        assert_eq!(
            serde_json::to_vec(&calls[0]).unwrap(),
            serde_json::to_vec(&old_calls[0]).unwrap(),
            "planner canonical bytes/cache stay identical"
        );
        assert_eq!(authors[0].input["plan"]["dependencyExits"], json!({}));
        let exit = &authors[1].input["plan"]["dependencyExits"]["W1"];
        assert_eq!(exit.as_object().unwrap().len(), 6);
        let full_tail = &authors[1].input["acceptedContinuity"]["unit"];
        assert_eq!(exit["unitId"], full_tail["id"]);
        for field in ["scene", "lighting", "props", "participants", "exitState"] {
            assert_eq!(exit[field], full_tail[field]);
        }
        assert!(full_tail["shots"].is_array());
        assert!(full_tail["shots"][0]["dialogueParts"].is_array());
        assert!(!exit.to_string().contains("dialogueParts"));
        for (author, old_author) in authors.iter().zip(&old_authors) {
            assert!(author.input.get("dependencyOutputs").is_none());
            assert_eq!(
                author.input["acceptedContinuity"],
                old_author.input["acceptedContinuity"]
            );
            assert_eq!(
                author.input["plan"]["acceptedContinuity"],
                old_author.input["plan"]["acceptedContinuity"]
            );
            let mut restored = (*author).clone();
            restored.input["dependencyOutputs"] = old_author.input["dependencyOutputs"].clone();
            restored.input["plan"]["dependencyExits"] =
                old_author.input["plan"]["dependencyExits"].clone();
            assert_eq!(
                serde_json::to_vec(&restored).unwrap(),
                serde_json::to_vec(old_author).unwrap(),
                "all non-dependency fields, schema and Skills remain byte-identical"
            );
        }
        let repeated = run_storyboard(&compact, &compact_exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(repeated["units"], new["units"]);
        assert_eq!(
            compact_exec.calls.lock().await.len(),
            calls.len(),
            "new profile also restores all accepted roles with zero new executor calls"
        );
        assert_eq!(compact, frozen_compact);
        for row in body {
            let quote = row.dialogue.as_ref().unwrap();
            assert!(
                new["units"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|unit| text(unit, "prompt").contains(&quote.quote)),
                "source dialogue is preserved exactly"
            );
        }
    }

    #[tokio::test]
    async fn unsupported_dependency_input_profile_sends_no_plan_author_or_editor() {
        let source = prepare_source("第一集\n场景：门口\n小王：原话。\n").unwrap();
        for profile in [json!("dependency-exit/v999"), json!(7)] {
            let mut request = input(&source);
            request[storyboard_dependencies::PROFILE_FIELD] = profile;
            let exec = Recorded::default();
            assert!(run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap_err()
                .contains("依赖出口输入版本未知"));
            assert!(exec.calls.lock().await.is_empty());
        }
    }

    #[test]
    fn legacy_finish_grouping_cannot_expand_when_new_row_transport_repairs_capacity() {
        let original = format!(
            "第一集\n场景：门口\n{}",
            (0..120)
                .map(|index| format!("小王OS：第{index}句原话 e1 😀。\n"))
                .collect::<String>()
        );
        let source = prepare_source(&original).unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .collect();
        let fingerprints: HashMap<_, _> = rows
            .iter()
            .map(|row| (row.id.clone(), digest(&row.text)))
            .collect();
        let mut found = None;
        for padding in (20_000..60_000).step_by(500) {
            let request = TextRequest {
                stage: "storyboardFinish".into(),
                skill: "episode-finishing-editor".into(),
                model: "offline".into(),
                reasoning: "medium".into(),
                input: json!({"finishMode":"delta","sourceRows":rows,"contextRows":[],"plan":{"workPackages":[],"entryFacts":[],"fantasyCandidates":[],"sourceFingerprints":fingerprints},"workPackages":[],"entryFacts":[],"selectedFantasy":[],"previousDraft":{"units":[{"id":"frozen-unit","completeAction":"不可删完整现稿。".repeat(padding)}],"fantasyRealization":[]},"acceptedContinuity":{"unit":{"id":"frozen-tail","exitState":"握杖、停步、视线不变。"}},"baseDraftHash":"frozen-base"}),
                schema: json!({"type":"object","properties":{"baseDraftHash":{"type":"string","enum":["frozen-base"]}}}),
                rule_snapshot: json!({"primary":{"id":"episode-finishing-editor","content":"原整编岗位完整保留。"}}),
            };
            let old = finish_grouping_characters(&request, false).unwrap();
            let actual = finish_transport_characters(&request).unwrap();
            if old > FINISH_TRANSPORT_LIMIT && actual <= FINISH_TRANSPORT_LIMIT {
                found = Some((request, old, actual));
                break;
            }
        }
        let (request, old, actual) = found.expect(
            "fixture must expose an old oversized candidate that only the new row table can fit",
        );
        let before = serde_json::to_vec(&request).unwrap();
        assert!(
            old > FINISH_TRANSPORT_LIMIT,
            "an old workflow cannot greedily merge this extra adjacent scope"
        );
        let grouping = finish_grouping_characters(&request, true).unwrap();
        assert!(grouping <= FINISH_TRANSPORT_LIMIT);
        assert!(
            actual <= grouping,
            "dispatch can be smaller without changing existing marked grouping"
        );
        let wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(wire.input["previousDraft"], request.input["previousDraft"]);
        assert_eq!(
            crate::pipeline::storyboard_transport::expand_storyboard_input(&wire.input).unwrap(),
            request.input
        );
        assert_eq!(
            serde_json::to_vec(&request).unwrap(),
            before,
            "actual-wire repair never mutates canonical request/cache bytes"
        );
    }

    #[tokio::test]
    async fn production_cards_reach_three_roles_without_prompt_history_and_replay_exactly() {
        let source = prepare_source(
            "第一集\n场景：门口\n小王：旧场原话。\n第二集\n场景：院内\n小王：本场原话。\n",
        )
        .unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .collect();
        let mut request = input(&source);
        request["selection"] = json!({"episodeIds":[source.episodes[1].id]});
        request["storyboardAssetInputProfile"] = json!(asset_cards::PROFILE);
        request["assets"] = json!([
            {"id":"person:base","variantId":"base","baseAssetId":"person","identityId":"person","name":"同名角色","kind":"character","version":3,"applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},"sourceRefs":[rows[0].id],"preparation":{"mode":"REFERENCE"},"prompt":"PRIVATE_IMAGE_PROMPT","history":["PRIVATE_HISTORY"],"referenceImages":[{"url":"PRIVATE_URL","path":"PRIVATE_PATH"}]},
            {"id":"person:hurt","variantId":"hurt","baseAssetId":"person","identityId":"person","name":"同名角色","kind":"character","version":7,"applicability":{"episodes":[source.episodes[1].id],"scenes":[source.scenes[1].id]},"sourceRefs":[rows[1].id],"sourceVariantId":"base","sourceAssetId":"person:base","preparation":{"mode":"INLINE","fromKey":"person:base","note":"沿准确基础身份参考"},"prompt":"PRIVATE_IMAGE_PROMPT","design":{"prompt":"PRIVATE_IMAGE_PROMPT","inputFingerprint":"PRIVATE_HISTORY"},"adoptedDesign":{"version":5,"brief":"已采用的黑发白衣形象。","provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT"}}
        ]);
        let fact = json!({"id":"visual-fact","assetId":"person","variantId":"hurt","attribute":"衣服","value":"蓝衣","claim":"CHARACTER_CLAIM","scope":"STATE","sourceRefs":[rows[1].id]});
        request["foundation"]["facts"] = json!([fact.clone()]);
        let frozen = request.clone();
        let exec = Recorded::default();
        let first = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(first["status"], "completed", "{}", first["blocked"]);
        let calls = exec.calls.lock().await.clone();
        assert_eq!(
            calls
                .iter()
                .map(|call| call.stage.as_str())
                .collect::<Vec<_>>(),
            vec!["storyboardPlan", "storyboardCreate", "storyboardFinish"]
        );
        for call in &calls {
            let cards = call.input["assets"].as_array().unwrap();
            assert_eq!(
                cards.len(),
                2,
                "parent material identity remains available without expanding production scope"
            );
            let child = cards
                .iter()
                .find(|card| card["id"] == "person:hurt")
                .unwrap();
            let parent = cards
                .iter()
                .find(|card| card["id"] == "person:base")
                .unwrap();
            assert_eq!(child["variantId"], "hurt");
            assert_eq!(child["version"], 7);
            assert_eq!(
                child["applicability"],
                request["assets"][1]["applicability"]
            );
            assert_eq!(child["sourceRefs"], json!([rows[1].id]));
            assert_eq!(child["sourceAssetId"], "person:base");
            assert_eq!(child["sourceVariantId"], "base");
            assert_eq!(child["preparation"], request["assets"][1]["preparation"]);
            assert_eq!(
                child["adoptedDesign"]["provenance"],
                "ADOPTED_DESIGN_NOT_SOURCE_FACT"
            );
            assert_eq!(parent["materialRole"], "MATERIAL_SOURCE_ONLY");
            assert_eq!(
                parent["applicability"],
                request["assets"][0]["applicability"]
            );
            assert_eq!(call.input["facts"], json!([fact.clone()]));
            assert_eq!(call.input["capabilities"], request["capabilities"]);
            let encoded = call.input["assets"].to_string();
            for private in [
                "PRIVATE_IMAGE_PROMPT",
                "PRIVATE_HISTORY",
                "PRIVATE_URL",
                "PRIVATE_PATH",
            ] {
                assert!(!encoded.contains(private));
            }
        }
        assert!(text(&first["units"][0], "prompt").contains("本场原话。"));
        assert_eq!(first["sourceSha256"], source.sha256);
        assert_eq!(
            request, frozen,
            "projection is transport only; canonical input is immutable"
        );
        assert_eq!(
            run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap()["units"],
            first["units"]
        );
        assert_eq!(
            exec.calls.lock().await.len(),
            3,
            "accepted plan, author and delta editor remain reusable"
        );

        let mut legacy = request.clone();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("storyboardAssetInputProfile");
        let legacy_exec = Recorded::default();
        let old = run_storyboard(&legacy, &legacy_exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(old["status"], "completed");
        let legacy_calls = legacy_exec.calls.lock().await;
        let expected = serde_json::to_string(&episode_assets(
            request["assets"].as_array().unwrap(),
            &source.episodes[1].id,
        ))
        .unwrap();
        for call in legacy_calls.iter() {
            assert_eq!(
                serde_json::to_string(&call.input["assets"]).unwrap(),
                expected,
                "unmarked workflow must keep its exact old request assets"
            );
            assert!(call.input["assets"]
                .to_string()
                .contains("PRIVATE_IMAGE_PROMPT"));
        }
    }

    #[tokio::test]
    async fn unsupported_asset_input_profile_sends_no_plan_author_or_editor() {
        let source = prepare_source("第一集\n场景：门口\n小王：原话。\n").unwrap();
        let mut request = input(&source);
        request["storyboardAssetInputProfile"] = json!("unknown/v99");
        let exec = Recorded::default();
        assert!(run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap_err()
            .contains("资产输入运输版本未知"));
        assert!(exec.calls.lock().await.is_empty());
    }

    #[tokio::test]
    #[ignore = "explicit read-only saved production evidence; no CLI, channel or database writes"]
    async fn saved_stage3_finish_capacity_replays_exact_accepted_authors_without_model_calls() {
        use rusqlite::{Connection, OpenFlags};
        let path = std::env::var("DIRECTOR_STAGE3_READONLY_DB")
            .expect("explicit read-only evidence database path required");
        let workflow = std::env::var("DIRECTOR_STAGE3_READONLY_WORKFLOW")
            .expect("explicit read-only workflow identity required");
        let connection =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (raw, created): (String, String) = connection
            .query_row(
                "SELECT snapshot,created_at FROM jobs WHERE id=?1 AND kind='workflow'",
                [&workflow],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let snapshot: Value = serde_json::from_str(&raw).unwrap();
        let mut input = snapshot["input"].clone();
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let episode_count: usize = std::env::var("DIRECTOR_STAGE3_READONLY_EPISODE_COUNT")
            .ok()
            .map(|value| {
                value
                    .parse()
                    .expect("explicit episode count must be numeric")
            })
            .unwrap_or(1);
        assert!(
            [1, 2].contains(&episode_count),
            "this saved fixture covers exactly the first one or two completed-author episodes"
        );
        let episodes: Vec<_> = source
            .episodes
            .iter()
            .take(episode_count)
            .map(|episode| episode.id.clone())
            .collect();
        assert_eq!(episodes.len(), episode_count);
        input["selection"] = json!({"episodeIds":episodes});
        let mut statement = connection.prepare("SELECT snapshot,result FROM jobs WHERE kind='text' AND state='completed' AND json_extract(receipt,'$.validation')='accepted' AND created_at>=?1 AND coalesce(json_extract(snapshot,'$.input.episodeId'),json_extract(snapshot,'$.input.episode.id')) IN (SELECT value FROM json_each(?2)) ORDER BY rowid").unwrap();
        let saved: Vec<(TextRequest, Value)> = statement
            .query_map(
                rusqlite::params![created, serde_json::to_string(&episodes).unwrap()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap()
            .map(|row| {
                let (request, result) = row.unwrap();
                (
                    serde_json::from_str(&request).unwrap(),
                    serde_json::from_str(&result).unwrap(),
                )
            })
            .collect();
        drop(statement);
        drop(connection);
        assert_eq!(
            saved
                .iter()
                .filter(|(r, _)| r.stage == "storyboardPlan")
                .count(),
            episode_count
        );
        assert_eq!(
            saved
                .iter()
                .filter(|(r, _)| r.stage == "storyboardCreate")
                .count(),
            if episode_count == 1 { 3 } else { 5 }
        );
        let saved_finish_count = saved
            .iter()
            .filter(|(request, _)| request.stage == "storyboardFinish")
            .count();
        fn canonical_diff_paths(old: &Value, new: &Value, path: &str, out: &mut Vec<String>) {
            if old == new || out.len() >= 40 {
                return;
            }
            match (old, new) {
                (Value::Object(a), Value::Object(b)) => {
                    let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
                    for key in keys {
                        canonical_diff_paths(
                            a.get(key).unwrap_or(&Value::Null),
                            b.get(key).unwrap_or(&Value::Null),
                            &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                            out,
                        );
                    }
                }
                (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
                    for (index, (a, b)) in a.iter().zip(b).enumerate() {
                        canonical_diff_paths(a, b, &format!("{path}/{index}"), out);
                    }
                }
                _ => out.push(path.to_string()),
            }
        }
        struct Replay {
            accepted: tokio::sync::Mutex<Vec<(TextRequest, Value)>>,
            dispatched: tokio::sync::Mutex<Vec<TextRequest>>,
            reused: tokio::sync::Mutex<Vec<String>>,
            finishes: tokio::sync::Mutex<Vec<TextRequest>>,
        }
        #[async_trait]
        impl TextExecutor for Replay {
            async fn execute(&self, request: TextRequest) -> Result<Value, String> {
                let key = serde_json::to_string(&request).unwrap();
                if let Some((_, value)) = self
                    .accepted
                    .lock()
                    .await
                    .iter()
                    .find(|(old, _)| serde_json::to_string(old).unwrap() == key)
                {
                    if request.stage == "storyboardFinish" {
                        self.finishes.lock().await.push(request.clone());
                    }
                    self.reused.lock().await.push(request.stage);
                    return Ok(value.clone());
                }
                if request.stage != "storyboardFinish" {
                    return Err(format!(
                        "{} canonical request changed; forbidden model dispatch",
                        request.stage
                    ));
                }
                if let Some((old, _)) = self.accepted.lock().await.iter().find(|(old, _)| {
                    old.stage == request.stage
                        && old.input["executionBatch"]["id"]
                            == request.input["executionBatch"]["id"]
                }) {
                    let mut differences = Vec::new();
                    canonical_diff_paths(
                        &serde_json::to_value(old).unwrap(),
                        &serde_json::to_value(&request).unwrap(),
                        "",
                        &mut differences,
                    );
                    eprintln!(
                        "readonly accepted Finish canonical mismatch paths only: {}",
                        json!(differences)
                    );
                }
                self.dispatched.lock().await.push(request.clone());
                self.finishes.lock().await.push(request.clone());
                Ok(
                    json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":request.input["previousDraft"]["units"].as_array().unwrap().iter().map(|unit|unit["id"].clone()).collect::<Vec<_>>(),"replacements":[],"revisionNotes":[],"unresolved":[]}),
                )
            }
            async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
                let mut accepted = self.accepted.lock().await;
                let key = serde_json::to_string(request).unwrap();
                if let Some((_, old)) = accepted
                    .iter_mut()
                    .find(|(r, _)| serde_json::to_string(r).unwrap() == key)
                {
                    *old = value.clone();
                } else {
                    accepted.push((request.clone(), value.clone()));
                }
                Ok(())
            }
        }
        let model = saved[0].0.model.clone();
        let reasoning = saved[0].0.reasoning.clone();
        let frozen = input.clone();
        let executor = Replay {
            accepted: tokio::sync::Mutex::new(saved),
            dispatched: Default::default(),
            reused: Default::default(),
            finishes: Default::default(),
        };
        let output = run_storyboard(&input, &executor, &model, &reasoning)
            .await
            .unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        assert!(output["waitingDecisions"].as_array().unwrap().is_empty());
        assert_eq!(input, frozen, "original workflow material is immutable");
        assert_eq!(output["sourceSha256"], source.sha256);
        let reused = executor.reused.lock().await.clone();
        assert_eq!(
            reused
                .iter()
                .filter(|stage| *stage == "storyboardPlan")
                .count(),
            episode_count
        );
        assert_eq!(
            reused
                .iter()
                .filter(|stage| *stage == "storyboardCreate")
                .count(),
            if episode_count == 1 { 3 } else { 5 }
        );
        let dispatched = executor.dispatched.lock().await.clone();
        let finishes = executor.finishes.lock().await.clone();
        assert_eq!(
            finishes.len(),
            if episode_count == 1 { 3 } else { 5 },
            "every actual accepted-author range finishes without trimming or repeating an author"
        );
        if saved_finish_count == finishes.len() {
            assert!(dispatched.is_empty(),"all accepted editor requests must keep their exact canonical bytes/hash; no replacement mock/model call is allowed");
            assert_eq!(
                reused
                    .iter()
                    .filter(|stage| *stage == "storyboardFinish")
                    .count(),
                saved_finish_count
            );
        }
        let measurements:Vec<_>=finishes.iter().map(|request| {
            let canonical=crate::text_executor::transport_payload_characters(request,&request.input,&request.schema);
            let projected=finish_transport_characters(request).unwrap();
            assert!(projected<=FINISH_TRANSPORT_LIMIT,"{projected}>{FINISH_TRANSPORT_LIMIT}");
            assert!(canonical>=projected);
            if let Some(wire)=compile_storyboard_transport(request).unwrap() {
                assert_eq!(wire.input["previousDraft"],request.input["previousDraft"]);
                assert_eq!(crate::pipeline::storyboard_transport::expand_storyboard_input(&wire.input).unwrap(),request.input);
                assert_eq!(crate::pipeline::storyboard_transport::expand_storyboard_schema(&wire.schema).unwrap(),request.schema);
            }
            json!({"canonicalCharacters":canonical,"wireCharacters":projected,"sourceRows":request.input["sourceRows"].as_array().unwrap().len(),"units":request.input["previousDraft"]["units"].as_array().unwrap().len()})
        }).collect();
        let refs: Vec<_> = finishes
            .iter()
            .flat_map(|r| r.input["sourceRows"].as_array().unwrap())
            .map(|r| text(r, "id").to_string())
            .collect();
        let expected: Vec<_> = source
            .rows
            .iter()
            .filter(|r| {
                r.kind == "body"
                    && r.episode_id
                        .as_ref()
                        .is_some_and(|episode| episodes.contains(episode))
            })
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(
            refs, expected,
            "whole source coverage is exact once and ordered"
        );
        let repeated = run_storyboard(&input, &executor, &model, &reasoning)
            .await
            .unwrap();
        assert_eq!(repeated["units"], output["units"]);
        assert_eq!(
            executor.dispatched.lock().await.len(),
            dispatched.len(),
            "accepted editor replay dispatches zero additional requests"
        );
        println!(
            "{}",
            json!({"readOnlySavedEvidence":true,"episodes":episode_count,"plannerModelDispatches":0,"authorModelDispatches":0,"reusedPlannerRequests":episode_count,"reusedAuthorRequests":if episode_count==1 {3}else{5},"mockEditorRequests":dispatched.len(),"measurements":measurements,"units":output["units"].as_array().unwrap().len(),"scopeRows":expected.len(),"videoRequests":0})
        );
    }

    #[tokio::test]
    #[ignore = "explicit rejected author database COPY; exact Saved hook and CLI capacity, zero model calls or original writes"]
    async fn readonly_rejected_author_finish_asset_projection_measures_actual_cli_and_saved_parent() {
        use rusqlite::{Connection, OpenFlags};
        use std::path::Path;
        let path = std::env::var("DIRECTOR_FINISH_ASSETS_READONLY_DB").expect("explicit stable DB copy");
        let workflow = std::env::var("DIRECTOR_FINISH_ASSETS_READONLY_WORKFLOW").expect("explicit parent workflow");
        let job = std::env::var("DIRECTOR_FINISH_ASSETS_READONLY_JOB").expect("explicit rejected author job");
        let path = Path::new(&path).canonicalize().unwrap();
        let evidence_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.local/live-acceptance-20261008").canonicalize().unwrap();
        assert!(path.starts_with(&evidence_root),"the probe can only open an isolated local acceptance copy, never the live project");
        let before = crate::store::hash_file(&path).unwrap();
        let connection = Connection::open_with_flags(&path,OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        connection.execute_batch("PRAGMA query_only=ON; BEGIN;").unwrap();
        let (snapshot,raw,hash,event,parent_text):(String,String,String,String,String)=connection.query_row(
            "SELECT j.snapshot,j.result,w.request_hash,e.data,parent.snapshot FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id JOIN jobs parent ON parent.id=w.workflow_id JOIN events e ON e.id=(SELECT MAX(x.id) FROM events x WHERE x.job_id=j.id AND x.type='text_received_raw') WHERE j.id=?1 AND w.workflow_id=?2 AND j.kind='text' AND j.provider='codex' AND j.state='rejected' AND w.state='rejected' AND w.stage='storyboardCreate' AND j.error=w.error AND j.result=w.result AND json_extract(j.receipt,'$.validation')='rejected' AND json_extract(j.receipt,'$.contract')='desktop-runner/v1' AND parent.kind='workflow' AND parent.entity_id='storyboard.run'",
            rusqlite::params![job,workflow],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
        assert_eq!(digest(&snapshot),hash);
        let author:TextRequest=serde_json::from_str(&snapshot).unwrap();
        let raw_value:Value=serde_json::from_str(&raw).unwrap();
        let event:Value=serde_json::from_str(&event).unwrap();
        let parent:Value=serde_json::from_str(&parent_text).unwrap();
        assert_eq!(event["requestHash"],hash);
        assert_eq!(event["value"],raw_value);
        let frozen=&parent["input"];
        assert_eq!(frozen["storyboardAssetInputProfile"],asset_cards::SCOPED_PROFILE);
        assert!(author.input.get("storyboardAssetInputProfile").is_none(),"this exact saved role intentionally omits the internal parent marker");
        let source:LockedSource=serde_json::from_value(frozen["source"].clone()).unwrap();
        let source=upgrade_source_dialogues(&source).unwrap();
        validate_source(&source).unwrap();
        let episode_id=text(&author.input,"episodeId");
        let episode=source.episodes.iter().find(|episode|episode.id==episode_id).unwrap();
        let mut assets=episode_assets(frozen["assets"].as_array().unwrap(),episode_id);
        asset_cards::inherit_identity_aliases(frozen,&mut assets).unwrap();
        let rows:Vec<_>=source.rows.iter().filter(|row|row.kind=="body" && row.episode_id.as_deref()==Some(episode_id)).cloned().collect();
        let decisions=scoped_author_decisions(&source,frozen,&rows).unwrap();
        let context=decision_context(&source,&rows,&relevant_context(&source,episode,&assets,&frozen["foundation"]),decisions.as_ref());
        let refs=strings(&author.input["executionBatch"],"sourceRefs").unwrap();
        let style=text(&author.input["settings"],"visualStyle");
        let prefix=format!("{episode_id}:{}:",text(&author.input["executionBatch"],"id"));
        let mut checked=raw_value.clone();
        let reason=validate_authored(&source,&mut checked,&assets,&author.input["capabilities"],&refs,style,&prefix).unwrap_err();
        assert_eq!(reason,crate::text_executor::AUTHOR_ASSET_SOURCE_REJECTION,"revalidation must reach the exact known secondary domain defect without adopting the raw");
        let plan_text:String=connection.query_row("SELECT w.result FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage='storyboardPlan' AND w.state='accepted' AND j.state='completed' AND json_extract(j.snapshot,'$.input.episode.id')=?2",rusqlite::params![workflow,episode_id],|row|row.get(0)).unwrap();
        let plan:Value=serde_json::from_str(&plan_text).unwrap();
        let selected=author.input["selectedFantasy"].as_array().unwrap();
        let (draft,issues)=author_validation_candidate(&source,&raw_value,&assets,&author.input["capabilities"],&refs,style,&prefix,&plan,selected,&reason).unwrap();
        assert!(!issues.is_empty());
        let execution=&author.input["executionBatch"];
        let scope=AcceptedAuthorScope {index:0,batch:original::ExecutionBatch {
            id:text(execution,"id").into(),package_ids:strings(execution,"packageIds").unwrap(),source_refs:refs,
            provisional_unit_ids:strings(execution,"provisionalUnitIds").unwrap(),estimated_seconds:execution["estimatedSeconds"].as_f64().unwrap(),exceeds_soft_target:execution["exceedsSoftTarget"].as_bool().unwrap(),
        },input:author.input.clone(),draft};
        let prepared=bind_scope_repair_request(finish_request(&[scope.clone()],&plan,&rows,&context,&assets,selected,episode_id,&author.model,&author.reasoning,frozen.get("voiceReference"),None,None).unwrap(),&source.sha256,&raw_value,&author,&scope,&issues,1).unwrap();
        let legacy=prepared.0.clone();
        // The actual Saved selector runs only against this disposable copy.
        // All IDs/snapshots/settings remain the saved ones; no mock hash oracle.
        let isolated=tempfile::tempdir().unwrap();
        let copied=isolated.path().join("project.sqlite");
        std::fs::copy(&path,&copied).unwrap();
        let wal=path.with_file_name("project.sqlite-wal");
        if wal.exists() {std::fs::copy(&wal,isolated.path().join("project.sqlite-wal")).unwrap();}
        let saved=crate::text_executor::SavedTextExecutor {store:crate::store::Store {root:isolated.path().to_path_buf()},settings:frozen["settings"].clone(),workflow_id:Some(workflow.clone())};
        let counts=|c:&Connection|->(i64,i64) {c.query_row("SELECT (SELECT COUNT(*) FROM jobs),(SELECT COUNT(*) FROM events)",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap()};
        let isolated_counts=counts(&saved.store.conn().unwrap());
        let projected=prepare_finish_assets(prepared,frozen,&assets,&saved).await.unwrap().0;
        assert_eq!(counts(&saved.store.conn().unwrap()),isolated_counts,"the real selector cannot insert a task or spend an attempt");
        assert_eq!(projected.input["finisherAssetInputProjection"]["frozenWorkflowInputSha256"],digest(&frozen.to_string()));
        assert_eq!(projected.input["finisherAssetInputProjection"]["legacyDomainRequestSha256"],digest(&serde_json::to_string(&legacy).unwrap()));
        assert_eq!(serde_json::to_value(creative::prepare_request(projected.clone()).unwrap()).unwrap(),serde_json::to_value(&projected).unwrap());
        for field in ["previousDraft","sourceRows","contextRows","authorRepair","validationIssues","executionBatch","baseDraftHash"] {
            assert_eq!(legacy.input[field],projected.input[field],"{field} must remain complete and exact");
        }
        assert_eq!(legacy.rule_snapshot,projected.rule_snapshot,"all Mac primary/method/runtime texts are unchanged");
        assert_eq!(legacy.schema,projected.schema);
        assert!(projected.input.get("frozenFinisherAssetContext").is_none());
        let metric=|value:&Value| {let text=value.to_string();json!({"utf16":text.encode_utf16().count(),"bytes":text.len()})};
        let measure=|request:&TextRequest| {
            let compiled=crate::text_executor::compile_codex_input(request).unwrap();
            let prompt:Value=serde_json::from_str(&compiled.prompt).unwrap();
            let fields:serde_json::Map<String,Value>=prompt["input"].as_object().unwrap().iter().map(|(field,value)|(field.clone(),metric(value))).collect();
            let top:serde_json::Map<String,Value>=prompt.as_object().unwrap().iter().map(|(field,value)|(field.clone(),metric(value))).collect();
            json!({"actualUtf16":compiled.prompt.encode_utf16().count()+compiled.schema.encode_utf16().count(),"actualBytes":compiled.prompt.len()+compiled.schema.len(),
                "promptUtf16":compiled.prompt.encode_utf16().count(),"schemaUtf16":compiled.schema.encode_utf16().count(),"promptBytes":compiled.prompt.len(),"schemaBytes":compiled.schema.len(),"topLevelValues":top,"inputValues":fields,
                "canonicalAssets":metric(&request.input["assets"]),"assetCount":request.input["assets"].as_array().unwrap().len()})
        };
        let old=measure(&legacy); let new=measure(&projected);
        let report=json!({"readOnlyOriginal":true,"actualSavedHook":true,"newModelCalls":0,"newJobs":0,"newEvents":0,"adopted":0,"videoRequests":0,
            "sourceSha256":source.sha256,"workflowId":workflow,"authorJobId":job,"authorRequestHash":hash,"originalAuthorRawSha256":digest(&raw),"originalSnapshotSha256":digest(&snapshot),
            "limitUtf16":FINISH_TRANSPORT_LIMIT,"legacy":old,"projected":new,"issues":issues.iter().map(|issue|json!({"code":issue["code"],"coordinate":issue["coordinate"],"assetId":issue["assetId"],"sourceRefs":issue["sourceRefs"]})).collect::<Vec<_>>(),
            "unchanged":["sourceRows","contextRows","previousDraft","schema","ruleSnapshot","authorRepair","executionBatch","originalAuthorInput","originalRaw"],"frozenWorkflowInputSha256":digest(&frozen.to_string())});
        connection.execute_batch("ROLLBACK;").unwrap();
        let after:(String,String)=connection.query_row("SELECT snapshot,result FROM jobs WHERE id=?1",[&job],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(after,(snapshot,raw));
        drop(connection);
        assert_eq!(crate::store::hash_file(&path).unwrap(),before);
        println!("{}",report);
        if let Ok(report_path)=std::env::var("DIRECTOR_FINISH_ASSETS_READONLY_REPORT") {
            let report_path=Path::new(&report_path);
            assert!(report_path.parent().unwrap().canonicalize().unwrap().starts_with(&evidence_root));
            std::fs::write(report_path,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        assert!(old["actualUtf16"].as_u64().unwrap()>FINISH_TRANSPORT_LIMIT as u64,"saved legacy input must reproduce this actual capacity defect");
        assert!(new["actualUtf16"].as_u64().unwrap()<=FINISH_TRANSPORT_LIMIT as u64,"the complete source/method/editor packet must fit without truncation");
    }

    #[test]
    #[ignore = "explicit read-only accepted author evidence; zero model calls or database writes"]
    fn readonly_indivisible_author_finish_measures_actual_cli_after_lossless_projection() {
        let path = std::env::var("DIRECTOR_CAPACITY_READONLY_DB").unwrap();
        let job = std::env::var("DIRECTOR_CAPACITY_READONLY_JOB").unwrap();
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        connection.execute_batch("PRAGMA query_only=ON;").unwrap();
        let (snapshot,raw):(String,String)=connection.query_row("SELECT snapshot,result FROM jobs WHERE id=?1 AND kind='text' AND state='completed' AND json_extract(receipt,'$.validation')='accepted'",[&job],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let author: TextRequest = serde_json::from_str(&snapshot).unwrap();
        let draft: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(author.stage, "storyboardCreate");
        let workflow = std::env::var("DIRECTOR_CAPACITY_READONLY_WORKFLOW").unwrap();
        let plan:String=connection.query_row("SELECT j.result FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage='storyboardPlan' AND w.state='accepted' AND j.state='completed' AND json_extract(j.snapshot,'$.input.episode.id')=?2",rusqlite::params![workflow,text(&author.input,"episodeId")],|r|r.get(0)).unwrap();
        let plan: Value = serde_json::from_str(&plan).unwrap();
        let execution = &author.input["executionBatch"];
        let scope = AcceptedAuthorScope {
            index: 0,
            batch: original::ExecutionBatch {
                id: text(execution, "id").into(),
                package_ids: strings(execution, "packageIds").unwrap(),
                source_refs: strings(execution, "sourceRefs").unwrap(),
                provisional_unit_ids: strings(execution, "provisionalUnitIds").unwrap(),
                estimated_seconds: execution["estimatedSeconds"].as_f64().unwrap(),
                exceeds_soft_target: execution["exceedsSoftTarget"].as_bool().unwrap(),
            },
            input: author.input.clone(),
            draft,
        };
        let rows: Vec<SourceRow> =
            serde_json::from_value(author.input["sourceRows"].clone()).unwrap();
        let context: Vec<SourceRow> =
            serde_json::from_value(author.input["contextRows"].clone()).unwrap();
        let (request, _, _) = finish_request(
            &[scope],
            &plan,
            &rows,
            &context,
            author.input["assets"].as_array().unwrap(),
            author.input["selectedFantasy"].as_array().unwrap(),
            text(&author.input, "episodeId"),
            &author.model,
            &author.reasoning,
            None,
            None,
            None,
        )
        .unwrap();
        let baseline = compile_storyboard_transport(&request).unwrap();
        let old = baseline
            .as_ref()
            .map(|w| (&w.input, &w.schema))
            .unwrap_or((&request.input, &request.schema));
        let wire = compile_storyboard_dispatch_transport(&request)
            .unwrap()
            .unwrap();
        let current =
            crate::text_executor::codex_wire_characters(&request, &wire.input, &wire.schema);
        println!(
            "{}",
            json!({"readOnly":true,"newModelCalls":0,"databaseWrites":0,"baselineCliCharacters":crate::text_executor::codex_wire_characters(&request,old.0,old.1),"dispatchCliCharacters":current,"limit":FINISH_TRANSPORT_LIMIT})
        );
        assert_eq!(
            super::storyboard_transport::expand_storyboard_input(&wire.input).unwrap(),
            request.input
        );
        assert!(current <= FINISH_TRANSPORT_LIMIT);
        let after: (String, String) = connection
            .query_row(
                "SELECT snapshot,result FROM jobs WHERE id=?1",
                [&job],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(after, (snapshot, raw));
    }

    fn applied_interpretation(source: &LockedSource, scope: Value, refs: Vec<String>) -> Value {
        json!({"id":"author:explicit-interpretation","revision":"frozen-revision","status":"APPLIED",
            "sourceSha256":source.sha256,"sourceVersion":1,"answer":"说者和原话保留；制作身份按此局部解释。",
            "question":"制作身份如何表达？","evidence":"准确原行","sourceRefs":refs,"episodes":scope["episodes"],
            "applicationScope":scope,"subjectAssetKeys":[],"interpretationOnly":true,
            "changeTargets":{"source":"FORBIDDEN"},"legacyChangeTargets":{"speaker":"FORBIDDEN"},
            "receipts":[{"id":"author:explicit-interpretation","revision":"frozen-revision","note":"现有目录保留，已采用局部制作解释。","coverage":scope["episodes"]}],
            "appliedNote":"已采用解释，未改写原文。"})
    }

    #[tokio::test]
    async fn applied_interpretation_reaches_three_roles_with_exact_context_and_cached_replay() {
        let source = prepare_source(
            "第一集\n场景：旧房\n原名：原话必须保留。\n第二集\n场景：新房\n原名：现在出发 e3/s12/a17/r123。\n",
        )
        .unwrap();
        let original = json!(source);
        let historical = source.rows.iter().find(|r| r.kind == "body").unwrap();
        let scope = json!({"kind":"EPISODES","episodes":[source.episodes[1].id]});
        let mut record = applied_interpretation(&source, scope, vec![historical.id.clone()]);
        record["answer"] = json!("历史e3/s12/a17/r123仅是逐字制作解释，保留实际原话。");
        record["receipts"][0]["note"] = json!("旧调用e3/s12/a17/r123的解释已采用；不重新编号。");
        record["appliedNote"] = json!("逐字保留r123与a17说明，不作为本次身份。");
        let original_record = record.clone();
        let mut request = input(&source);
        request["selection"] = json!({"episodeIds":[source.episodes[1].id]});
        request["foundation"]["authorDecisions"] = json!([record.clone()]);
        request["assets"] = json!([{"id":"A:base","identityId":"A","baseAssetId":"A","variantId":"base","name":"完全不匹配原话姓名","kind":"character","applicability":{"episodes":[source.episodes[1].id]},"authorDecisions":[record.clone(),record]}]);
        let exec = Recorded::default();
        let first = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(first["status"], "completed", "{}", first["blocked"]);
        let calls = exec.calls.lock().await;
        assert_eq!(calls.len(), 3);
        for call in calls.iter() {
            let package = &call.input["scopedAuthorDecisions"];
            assert_eq!(package["evidenceRole"], AUTHOR_DECISION_ROLE);
            assert_eq!(package["sourceMutationAllowed"], false);
            assert_eq!(package["records"].as_array().unwrap().len(), 1);
            assert_eq!(
                package["records"][0]["applicationScope"]["kind"],
                "EPISODES"
            );
            assert_eq!(
                package["records"][0]["subjectAssetKeys"],
                json!([]),
                "same-name text must not assign an identity"
            );
            assert!(package["records"][0].get("changeTargets").is_none());
            assert!(package["records"][0].get("legacyChangeTargets").is_none());
            for field in [
                "answer",
                "appliedNote",
                "receipts",
                "sourceRefs",
                "applicationScope",
            ] {
                assert_eq!(
                    package["records"][0][field], original_record[field],
                    "historical literal prose and canonical bindings stay unchanged"
                );
            }
            assert_eq!(package["records"][0]["sourceRefs"], json!([historical.id]));
            assert_eq!(call.input["assets"][0]["id"], "A:base");
            let safety_rule = text(package, "rule");
            assert!(safety_rule.contains("eN/sN/rN/aN/vN"));
            assert!(safety_rule.contains("它们不是本次主键，N不能当本次序号"));
            assert!(safety_rule.contains(
                "已校验subjectAssetKeys、applicationScope、sourceRefs及当前assets/sourceRows"
            ));
            assert!(safety_rule.contains("不猜短码对应"));
            assert!(safety_rule.contains("原文里本来出现的同形词和台词逐字保留"));
            assert_eq!(call.input["facts"], json!([]));
            assert!(call.input["assets"][0].get("authorDecisions").is_none());
            let evidence = call.input["contextRows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == historical.id)
                .unwrap();
            assert_eq!(evidence["text"], historical.text);
            assert_eq!(evidence["range"], json!(historical.range));
            assert_eq!(evidence["dialogue"]["speaker"], "原名");
        }
        drop(calls);
        assert!(text(&first["units"][0], "prompt").contains("现在出发 e3/s12/a17/r123。"));
        assert!(
            !text(&first["units"][0], "prompt").contains("原话必须保留。"),
            "historical evidence is not production coverage"
        );
        assert_eq!(json!(source), original);
        assert_eq!(
            run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap()["units"],
            first["units"]
        );
        assert_eq!(
            exec.calls.lock().await.len(),
            3,
            "accepted model responses replay without another call"
        );
    }

    #[test]
    fn applied_interpretation_keeps_each_local_scope_and_never_inferrs_global_or_subject() {
        let source=prepare_source("第一集\n场景：门口\n小王：门口原话。\n场景：院内\n小王：院内原话。\n第二集\n场景：河边\n小王：河边原话。\n").unwrap();
        let body: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .cloned()
            .collect();
        let local = |scene: &SourceScene| json!({"kind":"SCENES","episodes":[scene.episode_id],"scenes":[[scene.episode_id,scene.id]]});
        let first =
            applied_interpretation(&source, local(&source.scenes[0]), vec![body[0].id.clone()]);
        let second =
            applied_interpretation(&source, local(&source.scenes[1]), vec![body[1].id.clone()]);
        let mut request = input(&source);
        request["foundation"]["authorDecisions"] = json!([first.clone(), second.clone(), first]);
        let package = scoped_author_decisions(&source, &request, &body[..2])
            .unwrap()
            .unwrap();
        assert_eq!(
            package["records"].as_array().unwrap().len(),
            2,
            "same receipt with distinct local scopes is not broadened or discarded"
        );
        assert_eq!(
            package["records"][0]["applicationScope"],
            local(&source.scenes[0])
        );
        assert_eq!(
            package["records"][1]["applicationScope"],
            local(&source.scenes[1])
        );
        let one = scoped_author_decisions(&source, &request, &body[..1])
            .unwrap()
            .unwrap();
        assert_eq!(one["records"].as_array().unwrap().len(), 1);
        assert!(
            scoped_author_decisions(&source, &request, &body[2..])
                .unwrap()
                .is_none(),
            "same name in another episode grants no applicability"
        );
        request["foundation"]["authorDecisions"] = json!([second]);
        request["foundation"]["authorDecisions"][0]["applicationScope"]["kind"] =
            json!("UNRESOLVED");
        assert!(scoped_author_decisions(&source, &request, &body).is_err());
    }

    #[tokio::test]
    async fn unconfirmed_stale_or_invalid_interpretation_never_becomes_source_authority() {
        let source = prepare_source("第一集\n场景：门口\n小王：别改原话。\n").unwrap();
        let row = source.rows.iter().find(|r| r.kind == "body").unwrap();
        let record = applied_interpretation(
            &source,
            json!({"kind":"EPISODES","episodes":[source.episodes[0].id]}),
            vec![row.id.clone()],
        );
        for (field, value) in [
            ("status", json!("CONFIRMED")),
            ("sourceSha256", json!("older-source")),
        ] {
            let mut changed = record.clone();
            changed[field] = value;
            let mut request = input(&source);
            request["foundation"]["authorDecisions"] = json!([changed]);
            let exec = Recorded::default();
            assert_eq!(
                run_storyboard(&request, &exec, "offline", "medium")
                    .await
                    .unwrap()["status"],
                "completed"
            );
            assert!(
                exec.calls
                    .lock()
                    .await
                    .iter()
                    .all(|c| c.input.get("scopedAuthorDecisions").is_none()),
                "unadopted/old records must leave previous request bytes unchanged"
            );
        }
        for (field, value) in [
            ("sourceRefs", json!(["unregistered-row"])),
            ("subjectAssetKeys", json!(["小王"])),
            ("receipts", json!([])),
            ("answer", json!("")),
        ] {
            let mut changed = record.clone();
            changed[field] = value;
            let mut request = input(&source);
            request["foundation"]["authorDecisions"] = json!([changed]);
            let exec = Recorded::default();
            let result = run_storyboard(&request, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(result["blocked"][0]["code"], "AUTHOR_DECISION_INVALID");
            assert!(result["units"].as_array().unwrap().is_empty());
            assert!(
                exec.calls.lock().await.is_empty(),
                "invalid current adoption must stop before any model call"
            );
        }
        let mut request = input(&source);
        request["foundation"]["authorDecisions"] = json!([record.clone(), record]);
        request["foundation"]["authorDecisions"][1]["answer"] = json!("相同冻结身份不同答案");
        assert!(scoped_author_decisions(&source, &request, &[row.clone()])
            .unwrap_err()
            .contains("冲突"));
    }

    #[tokio::test]
    async fn related_finish_unions_local_interpretations_and_revision_retains_the_same_package() {
        let source = prepare_source(
            &("第一集\n场景：门口\n".to_string()
                + &(0..8)
                    .map(|n| format!("小王：原句{n}。\n"))
                    .collect::<String>()),
        )
        .unwrap();
        let row = source.rows.iter().find(|r| r.kind == "body").unwrap();
        let record = applied_interpretation(
            &source,
            json!({"kind":"EPISODES","episodes":[source.episodes[0].id]}),
            vec![row.id.clone()],
        );
        let mut request = input(&source);
        request["foundation"]["authorDecisions"] = json!([record]);
        let exec = Recorded {
            rough_per_row: true,
            ..Default::default()
        };
        let first = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(first["status"], "completed");
        let calls = exec.calls.lock().await;
        assert_eq!(
            calls
                .iter()
                .filter(|c| c.stage == "storyboardCreate")
                .count(),
            2
        );
        let finish = calls
            .iter()
            .find(|c| c.stage == "storyboardFinish")
            .unwrap();
        assert_eq!(
            finish.input["scopedAuthorDecisions"]["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            finish.input["scopedAuthorDecisions"]["productionSourceRefs"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
        drop(calls);
        request["existingUnits"] = first["units"].clone();
        request["selection"] = json!({"unitIds":[first["units"][0]["id"]]});
        request["revisionInstruction"] = json!("保留原话和声窗，检查表演。");
        let revised = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(revised["status"], "completed", "{}", revised["blocked"]);
        let calls = exec.calls.lock().await;
        let revision: Vec<_> = calls
            .iter()
            .filter(|c| c.input["mode"] == "revision")
            .collect();
        assert_eq!(revision.len(), 2);
        assert!(revision
            .iter()
            .all(|c| c.input["scopedAuthorDecisions"]["records"]
                .as_array()
                .unwrap()
                .len()
                == 1));
    }

    #[tokio::test]
    async fn source_intervals_are_materialized_without_speaking_cues_or_copying_settings() {
        let source =
            prepare_source("第一集\n场景：门口\n小王（低声）：别动（抬手），回来😀。\n").unwrap();
        let exec = Recorded::default();
        let output = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "completed");
        let parts = output["units"][0]["shots"][0]["dialogueParts"]
            .as_array()
            .unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|part| text(part, "text"))
                .collect::<String>(),
            "别动，回来😀。"
        );
        assert!(parts
            .iter()
            .all(|part| text(part, "speaker") == "小王" && text(part, "channel") == "现场"));
        assert!(output["units"][0]["prompt"]
            .as_str()
            .unwrap()
            .contains("别动"));
        for request in exec.calls.lock().await.iter() {
            assert_eq!(
                request.input["productionDirection"]["direction"],
                "克制且保留局部爆发。"
            );
            assert!(request.input["settings"].get("textApiToken").is_none());
            assert!(request.input["settings"].get("videoProvider").is_none());
            if request.stage != "storyboardPlan" {
                let properties = &request.schema["properties"]["units"]["items"]["properties"]
                    ["shots"]["items"]["properties"]["dialogueParts"]["items"]["properties"];
                assert!(properties.get("text").is_none() && properties.get("speaker").is_none());
            }
        }
        assert_eq!(exec.accepted.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn long_owner_finishes_related_authored_range_once_and_passes_accepted_author_tail() {
        let source=prepare_source("第一集\n场景：门口\n小王：一。\n小王：二。\n小王：三。\n小王：四。\n小王：五。\n小王：六。\n小王：七。\n小王：八。\n").unwrap();
        let exec = Recorded {
            rough_per_row: true,
            ..Default::default()
        };
        let first = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(first["status"], "completed");
        assert_eq!(first["units"].as_array().unwrap().len(), 2);
        let calls = exec.calls.lock().await.clone();
        assert_eq!(
            calls.iter().map(|r| r.stage.as_str()).collect::<Vec<_>>(),
            vec![
                "storyboardPlan",
                "storyboardCreate",
                "storyboardCreate",
                "storyboardFinish"
            ]
        );
        assert_eq!(calls[1].input["sourceRows"].as_array().unwrap().len(), 5);
        assert_eq!(calls[2].input["sourceRows"].as_array().unwrap().len(), 3);
        assert_eq!(calls[3].input["sourceRows"].as_array().unwrap().len(), 8);
        assert_eq!(calls[3].input["finishMode"], "delta");
        assert_eq!(
            calls[2].input["acceptedContinuity"]["version"],
            "accepted-tail-v1"
        );
        assert_eq!(
            calls[2].input["acceptedContinuity"]["unit"]["id"],
            calls[3].input["previousDraft"]["units"][4]["id"]
        );
        assert_eq!(
            calls[2].input["acceptedContinuity"]["evidenceRole"],
            "ACCEPTED_CREATIVE_STATE_ONLY"
        );
        // The saved normalized plan and each accepted full scope are reusable;
        // resuming never invokes another model or shrinks/expands its scope.
        let repeated = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(first["units"], repeated["units"]);
        assert_eq!(exec.calls.lock().await.len(), 4);
        assert_eq!(first["sourceSha256"], source.sha256);
    }

    #[tokio::test]
    async fn failed_editor_resumes_from_accepted_authors_without_another_author_call() {
        let source=prepare_source("第一集\n场景：门口\n小王：一。\n小王：二。\n小王：三。\n小王：四。\n小王：五。\n小王：六。\n小王：七。\n小王：八。\n").unwrap();
        let mut exec = Recorded {
            rough_per_row: true,
            fail_finish: true,
            ..Default::default()
        };
        let failed = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(failed["status"], "partially_blocked");
        assert_eq!(failed["units"], json!([]));
        assert_eq!(
            exec.calls
                .lock()
                .await
                .iter()
                .filter(|r| r.stage == "storyboardCreate")
                .count(),
            2
        );
        exec.fail_finish = false;
        let completed = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(completed["status"], "completed");
        assert_eq!(
            exec.calls
                .lock()
                .await
                .iter()
                .filter(|r| r.stage == "storyboardCreate")
                .count(),
            2
        );
        assert_eq!(
            exec.calls
                .lock()
                .await
                .iter()
                .filter(|r| r.stage == "storyboardFinish")
                .count(),
            2
        );
        assert_eq!(exec.accepted.lock().await.len(), 4);
        let quoted: String = completed["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|u| u["shots"].as_array().unwrap())
            .flat_map(|shot| shot["dialogueParts"].as_array().unwrap())
            .map(|part| text(part, "text"))
            .collect();
        assert_eq!(quoted, "一。二。三。四。五。六。七。八。");
    }

    #[tokio::test]
    async fn related_editor_keeps_voice_reference_and_does_not_change_source_windows() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n").unwrap();
        let mut value = input(&source);
        value["voiceReference"] =
            json!({"rows":[{"id":source.rows[2].dialogue.as_ref().unwrap().id,"seconds":2.75}]});
        let exec = Recorded::default();
        let result = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(result["status"], "completed");
        let calls = exec.calls.lock().await;
        let editor = calls
            .iter()
            .find(|request| request.stage == "storyboardFinish")
            .unwrap();
        assert_eq!(
            editor.input["capacityBrief"]["voiceReference"]["seconds"],
            2.75
        );
        assert_eq!(
            editor.input["capacityBrief"]["voiceReference"]["isPerformedMinimum"],
            false
        );
        assert_eq!(
            result["units"][0]["shots"][0]["dialogueParts"][0]["voiceStart"],
            0.0
        );
        assert_eq!(
            result["units"][0]["shots"][0]["dialogueParts"][0]["voiceEnd"],
            0.4
        );
        assert_eq!(
            result["units"][0]["shots"][0]["dialogueParts"][0]["text"],
            "别动。"
        );
    }

    fn old_finish_history(episode: &str) -> Value {
        json!({"stage":"storyboardFinish","status":"accepted","episodeId":episode,"finishMode":null,"executionBatchVersion":"execution-batches-v1","outputSchemaVersion":"storyboard-draft/v2","requestHash":"immutable-old-request"})
    }

    #[tokio::test]
    async fn old_batch_editor_checkpoint_blocks_before_any_executor_call_and_preserves_cache() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n").unwrap();
        let exec = Recorded::default();
        exec.accepted.lock().await.insert(
            "old-paid-author".into(),
            json!({"raw":"原合法回包与旧末态保持"}),
        );
        let prior = exec.accepted.lock().await.clone();
        let mut value = input(&source);
        value["storyboardMethod"] = json!(STORYBOARD_METHOD);
        value["storyboardHistory"] = json!([old_finish_history(&source.episodes[0].id)]);
        let frozen = value.clone();
        let output = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "partially_blocked");
        assert_eq!(output["units"], json!([]));
        assert_eq!(output["blocked"][0]["episodeId"], source.episodes[0].id);
        assert_eq!(
            output["blocked"][0]["code"],
            "STORYBOARD_METHOD_INCOMPATIBLE"
        );
        assert!(exec.calls.lock().await.is_empty());
        assert_eq!(*exec.accepted.lock().await, prior);
        assert_eq!(value, frozen);
    }

    #[tokio::test]
    async fn current_editor_checkpoint_and_plan_only_history_reuse_accepted_requests() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n").unwrap();
        let exec = Recorded::default();
        let mut value = input(&source);
        value["storyboardMethod"] = json!(STORYBOARD_METHOD);
        let first = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(first["status"], "completed");
        assert_eq!(exec.calls.lock().await.len(), 3);
        value["storyboardHistory"] = json!([{"stage":"storyboardFinish","status":"accepted","episodeId":source.episodes[0].id,"finishMode":"delta","executionBatchVersion":"related-range-finish/v1","outputSchemaVersion":"storyboard-delta/v2","requestHash":"current-exact-request"}]);
        let repeated = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(repeated["units"], first["units"]);
        assert_eq!(exec.calls.lock().await.len(), 3);
        value["storyboardMethod"] = json!("author-batch-finish/v1");
        value["storyboardHistory"] = json!([{"stage":"storyboardPlan","status":"accepted","episodeId":source.episodes[0].id,"outputSchemaVersion":"storyboard-plan/v2"}]);
        let migrated = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(migrated["units"], first["units"]);
        assert_eq!(exec.calls.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn incompatible_history_is_scoped_and_transport_identity_cannot_be_overridden() {
        let source =
            prepare_source("第一集\n场景：门口\n小王：别动。\n第二集\n场景：屋内\n小王：回来。\n")
                .unwrap();
        let mut value = input(&source);
        value["selection"] = json!({"episodeIds":[source.episodes[0].id]});
        value["storyboardHistory"] = json!([old_finish_history(&source.episodes[1].id)]);
        let exec = Recorded::default();
        let result = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(result["status"], "completed");
        let mut contradictory = old_finish_history(&source.episodes[0].id);
        contradictory["finishMode"] = json!("delta");
        contradictory["outputSchemaVersion"] = json!("storyboard-delta/v2");
        value["storyboardMethod"] = json!(STORYBOARD_METHOD);
        value["storyboardHistory"] = json!([contradictory]);
        let exec = Recorded::default();
        let rejected = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(
            rejected["blocked"][0]["code"],
            "STORYBOARD_METHOD_INCOMPATIBLE"
        );
        assert!(exec.calls.lock().await.is_empty());
        value["storyboardHistory"] = json!([old_finish_history("missing-episode")]);
        let unknown = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(unknown["blocked"][0]["episodeId"], source.episodes[0].id);
        assert!(exec.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn rejected_delta_is_not_accepted_or_automatically_retried() {
        let source = prepare_source("第一集\n场景：门口\n小王：一。\n小王：二。\n").unwrap();
        let mut exec = Recorded {
            invalid_finish_delta: true,
            ..Default::default()
        };
        let failed = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(failed["status"], "partially_blocked");
        assert_eq!(failed["units"], json!([]));
        assert_eq!(exec.calls.lock().await.len(), 3);
        assert_eq!(exec.rejected.lock().await.len(), 1);
        assert_eq!(exec.accepted.lock().await.len(), 2);
        exec.invalid_finish_delta = false;
        let fixed = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(fixed["status"], "completed");
        assert_eq!(exec.calls.lock().await.len(), 4);
        assert_eq!(
            exec.calls
                .lock()
                .await
                .iter()
                .filter(|r| r.stage == "storyboardCreate")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn editor_batches_follow_actual_transport_budget_and_cover_source_once() {
        let mut screenplay = "第一集\n场景：门口\n".to_string();
        for i in 0..18 {
            screenplay.push_str(&format!("小王：第{i}句。\n"));
        }
        let source = prepare_source(&screenplay).unwrap();
        let exec = Recorded {
            rough_per_row: true,
            large_picture: 700,
            ..Default::default()
        };
        let result = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{}", result["blocked"]);
        let calls = exec.calls.lock().await.clone();
        let authors: Vec<_> = calls
            .iter()
            .filter(|r| r.stage == "storyboardCreate")
            .collect();
        let editors: Vec<_> = calls
            .iter()
            .filter(|r| r.stage == "storyboardFinish")
            .collect();
        assert_eq!(authors.len(), 4);
        assert!(
            editors.len() > 1 && editors.len() < authors.len(),
            "author={},finish={}",
            authors.len(),
            editors.len()
        );
        let last_author = calls
            .iter()
            .rposition(|r| r.stage == "storyboardCreate")
            .unwrap();
        assert!(calls
            .iter()
            .enumerate()
            .filter(|(_, r)| r.stage == "storyboardFinish")
            .all(|(index, _)| index > last_author));
        for editor in &editors {
            let actual = crate::text_executor::transport_payload_characters(
                editor,
                &editor.input,
                &editor.schema,
            );
            assert!(
                actual <= FINISH_TRANSPORT_LIMIT,
                "{actual}>{FINISH_TRANSPORT_LIMIT}"
            );
        }
        let actual_refs: Vec<_> = editors
            .iter()
            .flat_map(|r| r.input["sourceRows"].as_array().unwrap())
            .map(|row| text(row, "id").to_string())
            .collect();
        let expected_refs: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .map(|row| row.id.clone())
            .collect();
        assert_eq!(actual_refs, expected_refs);
        let quoted: String = result["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|u| u["shots"].as_array().unwrap())
            .flat_map(|shot| shot["dialogueParts"].as_array().unwrap())
            .map(|part| text(part, "text"))
            .collect();
        assert_eq!(
            quoted,
            (0..18).map(|i| format!("第{i}句。")).collect::<String>()
        );
    }

    fn delta_fixture() -> (Value, Value) {
        let base = json!({"schemaVersion":"storyboard-draft/v2","units":[{"id":"a","shots":[{"id":"sa","duration":2.75,"dialogueParts":[{"lineId":"locked","quoteStart":12,"quoteEnd":15,"voiceStart":0.25,"voiceEnd":2.4}]}]},{"id":"b","shots":[{"id":"sb"}]},{"id":"c","shots":[{"id":"sc"}]}],"fantasyRealization":[],"revisionNotes":[],"unresolved":[]});
        let delta = json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":finish_base_hash(&base),"retainedUnitIds":["a","b","c"],"replacements":[],"revisionNotes":[],"unresolved":[]});
        (base, delta)
    }

    #[test]
    fn delta_retains_exact_units_and_rejects_stale_missing_duplicate_or_noncontiguous_groups() {
        let (base, delta) = delta_fixture();
        assert_eq!(apply_finish_delta(&base, &delta).unwrap(), base);
        let mut bad = delta.clone();
        bad["baseDraftHash"] = json!("stale");
        assert!(apply_finish_delta(&base, &bad).is_err());
        let mut bad = delta.clone();
        bad["retainedUnitIds"] = json!(["a", "b"]);
        assert!(apply_finish_delta(&base, &bad).is_err());
        let mut bad = delta.clone();
        bad["retainedUnitIds"] = json!(["a", "a", "b", "c"]);
        assert!(apply_finish_delta(&base, &bad).is_err());
        let mut bad = delta.clone();
        bad["retainedUnitIds"] = json!(["b"]);
        bad["replacements"] =
            json!([{"replaces":["a","c"],"units":[{"id":"new"}],"fantasyRealization":[]}]);
        assert!(apply_finish_delta(&base, &bad).is_err());
        let mut changed = delta;
        changed["retainedUnitIds"] = json!(["a", "c"]);
        changed["replacements"] = json!([{"replaces":["b"],"units":[{"id":"new","shots":[{"id":"new-shot"}]}],"fantasyRealization":[]}]);
        let result = apply_finish_delta(&base, &changed).unwrap();
        assert_eq!(result["units"][0], base["units"][0]);
        assert_eq!(result["units"][2], base["units"][2]);
        assert_eq!(result["units"][1]["id"], "new");
        changed["replacements"][0]["units"][0]["id"] = json!("a");
        assert!(apply_finish_delta(&base, &changed).is_err());
    }

    #[test]
    fn delta_fantasy_requires_one_complete_group_and_cannot_be_silently_dropped() {
        let (mut base, mut delta) = delta_fixture();
        base["fantasyRealization"] = json!([{"candidateId":"f","unitIds":["a","b"],"shotIds":["sa","sb"],"screenSeconds":1.5,"entry":"进入","exit":"退出","preservedFunction":"关系变化"}]);
        delta["baseDraftHash"] = json!(finish_base_hash(&base));
        delta["retainedUnitIds"] = json!(["b", "c"]);
        delta["replacements"] =
            json!([{"replaces":["a"],"units":[{"id":"new"}],"fantasyRealization":[]}]);
        assert!(apply_finish_delta(&base, &delta).is_err());
        delta["retainedUnitIds"] = json!(["c"]);
        delta["replacements"] = json!([{"replaces":["a","b"],"units":[{"id":"new"}],"fantasyRealization":[{"candidateId":"f","unitIds":["outside"]}]}]);
        assert!(apply_finish_delta(&base, &delta).is_err());
        delta["replacements"][0]["fantasyRealization"][0]["unitIds"] = json!(["new"]);
        assert!(apply_finish_delta(&base, &delta).is_ok());
        // The production validator still requires the selected candidate's
        // complete, source-valid realization after application.
        delta["replacements"][0]["fantasyRealization"] = json!([]);
        let dropped = apply_finish_delta(&base, &delta).unwrap();
        assert!(validate_fantasy(&dropped, &[json!({"id":"f"})]).is_err());
    }

    #[test]
    fn method_cards_are_loaded_by_scope_and_source_expression_does_not_grant_fantasy() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n").unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .cloned()
            .collect();
        let base = methods_for(&rows, &[], &[], false, &[]).unwrap();
        assert!(base.contains(&"storyboard-common") && !base.contains(&"fantasy-expression"));
        let finish = methods_for(&rows, &[], &["long-voice".into()], true, &[]).unwrap();
        assert!(
            finish.contains(&"storyboard-card-long-voice")
                && finish.contains(&"storyboard-card-editing-integrity")
                && finish.contains(&"storyboard-card-camera-path-repair")
        );
        assert!(!finish.contains(&"storyboard-card-danger-rescue"));
        assert!(methods_for(&rows, &[], &["unknown-card".into()], false, &[]).is_err());
        let source = prepare_source("第一集\n场景：门口\nQ版小王OS：别动。\n").unwrap();
        assert!(methods_for(&source.rows, &[], &[], false, &[])
            .unwrap()
            .contains(&"fantasy-expression"));
    }

    #[test]
    fn prefixed_packed_segments_keep_shot_headers_and_source_dialogue_consistent() {
        let mut draft = json!({"units":[{"id":"U","unitDirection":"【S1—S2】完整表演。", "lighting":"【S1—S2】自然侧光。", "initialState":"原站位。\n【S1起承接】站位未变。",
            "segments":[{"shotIds":["S1","S2"]}],"shots":[{"id":"S1","dialogueParts":[{"text":"源文提到【S1—S2】，原话不许改。"}]},{"id":"S2"}]}],"fantasyRealization":[]});
        let quote = draft["units"][0]["shots"][0]["dialogueParts"].clone();
        prefix_ids(&mut draft, "episode:batch:").unwrap();
        assert_eq!(
            draft["units"][0]["segments"][0]["shotIds"],
            json!(["episode:batch:S1", "episode:batch:S2"])
        );
        assert_eq!(
            draft["units"][0]["unitDirection"],
            "【episode:batch:S1—episode:batch:S2】完整表演。"
        );
        assert!(text(&draft["units"][0], "initialState").contains("【episode:batch:S1起承接】"));
        assert_eq!(draft["units"][0]["shots"][0]["dialogueParts"], quote);
        let before = draft.clone();
        prefix_ids(&mut draft, "episode:batch:").unwrap();
        assert_eq!(draft, before);
    }

    #[test]
    fn scoped_shot_ids_resolve_only_cross_unit_reuse_and_preserve_source_performance() {
        let quote = json!([{"lineId":"locked","text":"原话含【S1—S2】，不可改。","quoteStart":4,"quoteEnd":20,"voiceStart":0.25,"voiceEnd":2.75}]);
        let mut draft = json!({"units":[
            {"id":"U1","unitDirection":"【S1—S2】起落。","lighting":"【S1—S2】侧光。","initialState":"【S1起承接】原站位。","segments":[{"shotIds":["S1","S2"]}],"shots":[{"id":"S1","duration":3.0,"dialogueParts":quote,"picture":"原动作一。"},{"id":"S2","duration":2.5,"dialogueParts":[],"picture":"原动作二。"}]},
            {"id":"U2","unitDirection":"【S1—S2】回应。","segments":[{"shotIds":["S1","S2"]}],"shots":[{"id":"S1","duration":4.0,"dialogueParts":[],"picture":"回应。"},{"id":"S2","duration":2.0,"dialogueParts":[],"picture":"落点。"}]}
        ],"fantasyRealization":[{"candidateId":"f1","unitIds":["U1"],"shotIds":["S1"],"entry":"U1/S1原入点。","exit":"原出点。","screenSeconds":1.5},{"candidateId":"f2","unitIds":["U2"],"shotIds":["S1","S2"],"entry":"U2/S1原入点。","exit":"原出点。","screenSeconds":2.5}]});
        let raw = draft.clone();
        prefix_ids(&mut draft, "episode:batch:").unwrap();
        assert_eq!(
            draft["units"][0]["shots"][0]["id"],
            "episode:batch:U1:shot:S1"
        );
        assert_eq!(
            draft["units"][1]["shots"][0]["id"],
            "episode:batch:U2:shot:S1"
        );
        assert_eq!(
            draft["units"][0]["segments"][0]["shotIds"],
            json!(["episode:batch:U1:shot:S1", "episode:batch:U1:shot:S2"])
        );
        assert_eq!(
            draft["units"][1]["segments"][0]["shotIds"],
            json!(["episode:batch:U2:shot:S1", "episode:batch:U2:shot:S2"])
        );
        assert_eq!(
            draft["fantasyRealization"][1]["unitIds"],
            json!(["episode:batch:U2"])
        );
        assert_eq!(
            draft["fantasyRealization"][1]["shotIds"],
            json!(["episode:batch:U2:shot:S1", "episode:batch:U2:shot:S2"])
        );
        for (before, after) in raw["units"]
            .as_array()
            .unwrap()
            .iter()
            .zip(draft["units"].as_array().unwrap())
        {
            for (before, after) in before["shots"]
                .as_array()
                .unwrap()
                .iter()
                .zip(after["shots"].as_array().unwrap())
            {
                let mut before = before.clone();
                let mut after = after.clone();
                before.as_object_mut().unwrap().remove("id");
                after.as_object_mut().unwrap().remove("id");
                assert_eq!(
                    before, after,
                    "source intervals, dialogue, voice windows and performance are untouched"
                );
            }
        }
        for i in 0..2 {
            assert_eq!(
                draft["fantasyRealization"][i]["entry"],
                raw["fantasyRealization"][i]["entry"]
            );
            assert_eq!(
                draft["fantasyRealization"][i]["exit"],
                raw["fantasyRealization"][i]["exit"]
            );
        }
        let once = draft.clone();
        prefix_ids(&mut draft, "episode:batch:").unwrap();
        assert_eq!(draft, once, "revalidation must not change normalized IDs");
    }

    #[test]
    fn scoped_shot_ids_keep_unique_legacy_and_adopted_ids_unchanged() {
        let mut draft = json!({"units":[{"id":"adopted-unit","shots":[{"id":"adopted-shot","duration":3.0,"dialogueParts":[]}]},{"id":"fresh-unit","shots":[{"id":"fresh-shot"},{"id":"episode:batch:already-prefixed"}]}],"fantasyRealization":[{"unitIds":["adopted-unit"],"shotIds":["adopted-shot"]}]});
        let adopted = draft["units"][0].clone();
        prefix_ids_preserving(
            &mut draft,
            "episode:batch:",
            &HashSet::from(["adopted-unit".into()]),
            &HashSet::from(["adopted-shot".into()]),
        )
        .unwrap();
        assert_eq!(draft["units"][0], adopted);
        assert_eq!(
            draft["units"][1]["shots"][0]["id"],
            "episode:batch:fresh-shot"
        );
        assert_eq!(
            draft["units"][1]["shots"][1]["id"],
            "episode:batch:already-prefixed"
        );
        assert_eq!(
            draft["fantasyRealization"][0]["shotIds"],
            json!(["adopted-shot"])
        );
    }

    #[test]
    fn scoped_shot_ids_do_not_rename_adopted_owner_when_fresh_unit_reuses_its_id() {
        let mut draft = json!({"units":[{"id":"adopted-unit","shots":[{"id":"adopted-shot","duration":3.0}]},{"id":"fresh-unit","shots":[{"id":"adopted-shot","duration":2.0}]}],"fantasyRealization":[{"unitIds":["adopted-unit"],"shotIds":["adopted-shot"]},{"unitIds":["fresh-unit"],"shotIds":["adopted-shot"]}]});
        let adopted = draft["units"][0].clone();
        prefix_ids_preserving(
            &mut draft,
            "episode:batch:",
            &HashSet::from(["adopted-unit".into()]),
            &HashSet::from(["adopted-shot".into()]),
        )
        .unwrap();
        assert_eq!(draft["units"][0], adopted);
        assert_eq!(
            draft["units"][1]["shots"][0]["id"],
            "episode:batch:fresh-unit:shot:adopted-shot"
        );
        assert_eq!(
            draft["fantasyRealization"][0]["shotIds"],
            json!(["adopted-shot"])
        );
        assert_eq!(
            draft["fantasyRealization"][1]["shotIds"],
            json!(["episode:batch:fresh-unit:shot:adopted-shot"])
        );
    }

    #[test]
    fn scoped_shot_ids_reject_same_unit_duplicates_and_ambiguous_fantasy_atomically() {
        let mut duplicate = json!({"units":[{"id":"U1","shots":[{"id":"S1"},{"id":"S1"}]}],"fantasyRealization":[]});
        let raw = duplicate.clone();
        assert_eq!(
            prefix_ids(&mut duplicate, "p:").unwrap_err(),
            "模型镜号重复"
        );
        assert_eq!(duplicate, raw);
        let mut ambiguous = json!({"units":[{"id":"U1","shots":[{"id":"S1"}]},{"id":"U2","shots":[{"id":"S1"}]}],"fantasyRealization":[{"unitIds":["U1","U2"],"shotIds":["S1"]}]});
        let raw = ambiguous.clone();
        assert!(prefix_ids(&mut ambiguous, "p:")
            .unwrap_err()
            .contains("有歧义"));
        assert_eq!(ambiguous, raw);
        let mut missing = raw.clone();
        missing["fantasyRealization"][0]["unitIds"] = json!(["outside-unit"]);
        let raw = missing.clone();
        assert!(prefix_ids(&mut missing, "p:").is_err());
        assert_eq!(missing, raw);
    }

    #[test]
    fn scoped_shot_ids_reject_cross_unit_segment_references_and_new_id_collisions() {
        let mut cross_unit = json!({"units":[{"id":"U1","segments":[{"shotIds":["S2"]}],"shots":[{"id":"S1"}]},{"id":"U2","shots":[{"id":"S2"}]}],"fantasyRealization":[]});
        let raw = cross_unit.clone();
        assert_eq!(
            prefix_ids(&mut cross_unit, "p:").unwrap_err(),
            "联排段引用未知镜号"
        );
        assert_eq!(cross_unit, raw);
        let mut collision = json!({"units":[{"id":"U1","shots":[{"id":"S1"}]},{"id":"U2","shots":[{"id":"S1"},{"id":"U1:shot:S1"}]}],"fantasyRealization":[]});
        let raw = collision.clone();
        assert!(prefix_ids(&mut collision, "p:")
            .unwrap_err()
            .contains("规范化后镜号冲突"));
        assert_eq!(collision, raw);
    }

    #[test]
    #[ignore = "explicit rejected author database COPY; read-only, zero model calls or adoption"]
    fn readonly_rejected_author_scoped_shot_ids_revalidates_all_author_gates() {
        let path = std::env::var("DIRECTOR_SHOT_ID_READONLY_DB")
            .expect("explicit read-only isolated DB copy");
        let workflow = std::env::var("DIRECTOR_SHOT_ID_READONLY_WORKFLOW")
            .expect("explicit original workflow");
        let job =
            std::env::var("DIRECTOR_SHOT_ID_READONLY_JOB").expect("explicit original author job");
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        connection
            .execute_batch("PRAGMA query_only=ON; BEGIN;")
            .unwrap();
        let (snapshot, raw, hash, event, workflow_snapshot): (String, String, String, String, String) = connection.query_row("SELECT j.snapshot,j.result,w.request_hash,e.data,parent.snapshot FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id JOIN jobs parent ON parent.id=w.workflow_id JOIN events e ON e.id=(SELECT MAX(x.id) FROM events x WHERE x.job_id=j.id AND x.type='text_received_raw') WHERE j.id=?1 AND w.workflow_id=?2 AND j.kind='text' AND j.state='rejected' AND j.error='模型镜号重复' AND w.state='rejected' AND w.stage='storyboardCreate' AND w.error=j.error AND w.result=j.result AND json_extract(j.receipt,'$.validation')='rejected' AND json_extract(j.receipt,'$.contract')='desktop-runner/v1' AND parent.kind='workflow' AND parent.entity_id='storyboard.run'", rusqlite::params![job, workflow], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
        connection.execute_batch("ROLLBACK;").unwrap();
        assert_eq!(digest(&snapshot), hash);
        let request: TextRequest = serde_json::from_str(&snapshot).unwrap();
        let raw: Value = serde_json::from_str(&raw).unwrap();
        let event: Value = serde_json::from_str(&event).unwrap();
        let run: Value = serde_json::from_str(&workflow_snapshot).unwrap();
        assert_eq!(event["requestHash"], hash);
        assert!(
            event["value"] == raw,
            "original receive event must equal rejected raw"
        );
        assert_eq!(request.stage, "storyboardCreate");
        assert_eq!(request.skill, "unit-performance-director");
        assert_eq!(request.input[creative::PROFILE_FIELD], creative::PROFILE);
        let source: LockedSource = serde_json::from_value(run["input"]["source"].clone()).unwrap();
        let source = upgrade_source_dialogues(&source).unwrap();
        validate_source(&source).unwrap();
        let assets = episode_assets(
            run["input"]["assets"].as_array().unwrap(),
            text(&request.input, "episodeId"),
        );
        let refs = strings(&request.input["executionBatch"], "sourceRefs").unwrap();
        let prefix = format!(
            "{}:{}:",
            text(&request.input, "episodeId"),
            text(&request.input["executionBatch"], "id")
        );
        let original = raw.clone();
        let mut normalized = raw.clone();
        prefix_ids(&mut normalized, &prefix).unwrap();
        let normalized_units = normalized["units"].as_array().unwrap();
        let unit_count = normalized_units.len();
        let shot_count: usize = normalized_units
            .iter()
            .map(|unit| unit["shots"].as_array().unwrap().len())
            .sum();
        let once = normalized.clone();
        prefix_ids(&mut normalized, &prefix).unwrap();
        assert!(normalized == once, "ID normalization is idempotent");
        let mut candidate = raw.clone();
        let mut gate = "authored_source_asset_voice_coverage";
        let result: Result<(), String> = (|| {
            validate_authored(
                &source,
                &mut candidate,
                &assets,
                &request.input["capabilities"],
                &refs,
                text(&request.input["settings"], "visualStyle"),
                &prefix,
            )?;
            gate = "event_bindings";
            validate_event_bindings(&candidate, &request.input["plan"])?;
            gate = "fantasy_profile";
            validate_fantasy_profile(
                &candidate,
                request.input["selectedFantasy"].as_array().unwrap(),
                true,
            )?;
            gate = "creative_delivery";
            creative::add_delivery(
                &mut candidate,
                &source,
                text(&request.input["settings"], "visualStyle"),
                &assets,
                true,
            )?;
            Ok(())
        })();
        assert!(
            raw == original,
            "revalidation cannot alter the original raw"
        );
        // This opt-in assertion describes an expected diagnostic from the full
        // validator. It does not skip a gate or accept the rejected candidate.
        let expected_rejection =
            std::env::var("DIRECTOR_SHOT_ID_READONLY_EXPECT_REJECTION").unwrap_or_default();
        let expected_known_rejection =
            !expected_rejection.is_empty() && result.as_ref().err() == Some(&expected_rejection);
        let diagnosis_matches = if expected_rejection.is_empty() {
            result.is_ok()
        } else {
            expected_known_rejection
        };
        let evidence = json!({"version":"scoped-shot-id-revalidation/v1","readOnly":true,"workflowId":workflow,"originalJobId":job,"requestHash":hash,"sourceSha256":source.sha256,"units":unit_count,"shots":shot_count,"normalizationIdempotent":normalized==once,"allAuthorGatesPassed":result.is_ok(),"expectedKnownRejection":expected_known_rejection,"expectedRejection":if expected_rejection.is_empty(){Value::Null}else{json!(expected_rejection)},"diagnosisMatchesExpectation":diagnosis_matches,"firstRejectedGate":if result.is_ok(){Value::Null}else{json!(gate)},"error":result.as_ref().err(),"originalRawUnchanged":raw==original,"accepted":false,"modelCalls":0,"videoSubmissions":0});
        if let Ok(report) = std::env::var("DIRECTOR_SHOT_ID_READONLY_REPORT") {
            std::fs::write(report, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
        println!("{evidence}");
        assert!(
            diagnosis_matches,
            "subsequent author gate {gate}: actual {:?}, expected {:?}",
            result.as_ref().err(),
            if expected_rejection.is_empty() {
                None
            } else {
                Some(expected_rejection.as_str())
            }
        );
    }

    #[test]
    fn previous_source_boundary_retains_complete_scene_or_whole_oversized_tail_node() {
        let mut text = String::from("第一集\n场景：旧场\n某人：旧场对白。\n场景：末场\n");
        for i in 0..10 {
            text.push_str(&format!("甲：第{i}句。\n"));
        }
        text.push_str("第二集\n场景：新场\n乙：回来。\n");
        let source = prepare_source(&text).unwrap();
        let evidence = previous_source_boundary(&source, &source.episodes[1]);
        assert_eq!(evidence["selectionKind"], "SAVED_SCENE");
        assert_eq!(evidence["clipped"], false);
        assert_eq!(evidence["currentApplicability"], "UNKNOWN");
        let refs = strings(&evidence, "sourceRefs").unwrap();
        assert_eq!(
            refs.iter()
                .filter(|id| source.rows.iter().any(|r| &r.id == *id && r.kind == "body"))
                .count(),
            10
        );
        assert!(!refs.iter().any(|id| source
            .rows
            .iter()
            .any(|r| &r.id == id && r.text.contains("旧场对白"))));
        let long = "字".repeat(2500);
        let source = prepare_source(&format!(
            "第一集\n场景：末场\n甲：短句。\n甲：{long}。\n第二集\n场景：新场\n乙：回来。\n"
        ))
        .unwrap();
        let evidence = previous_source_boundary(&source, &source.episodes[1]);
        assert_eq!(evidence["selectionKind"], "TAIL_NODES");
        assert_eq!(evidence["budgetExceeded"], true);
        let refs = strings(&evidence, "sourceRefs").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(
            source
                .rows
                .iter()
                .find(|r| r.id == refs[0])
                .unwrap()
                .dialogue
                .as_ref()
                .unwrap()
                .quote,
            format!("{long}。")
        );
        assert!(previous_source_boundary(&source, &source.episodes[0]).is_null());
    }

    #[test]
    fn history_keeps_relevant_different_claims_and_original_neighbors() {
        let source=prepare_source("第一集\n场景：门口\n甲：我已经把刀收好。\n乙：他还拿着刀。\n甲：我放下刀了。\n第二集\n场景：门口\n甲：走吧。\n").unwrap();
        let prior: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body" && r.episode_id.as_ref() == Some(&source.episodes[0].id))
            .collect();
        let assets = vec![json!({"id":"A:v","baseAssetId":"A","variantId":"v","name":"甲｜基础"})];
        let foundation = json!({"facts":[{"assetId":"A","attribute":"持物","claim":"CHARACTER_CLAIM","scope":"LOCAL","sourceRefs":[prior[0].id]},{"assetId":"A","attribute":"持物","claim":"CHARACTER_CLAIM","scope":"LOCAL","sourceRefs":[prior[2].id]}]});
        let context = relevant_context(&source, &source.episodes[1], &assets, &foundation);
        for row in prior {
            assert!(context.iter().any(|r| r.id == row.id && r.text == row.text));
        }
        assert!(!context
            .iter()
            .any(|r| r.episode_id.as_ref() == Some(&source.episodes[1].id)));
    }

    #[test]
    fn owner_context_keeps_declared_evidence_and_background_without_broadcasting_eighty_episodes() {
        let mut text = String::from("世界规则：刀不能穿过护盾。\n");
        for n in 1..=80 {
            text.push_str(&format!("第{n}集\n场景：门口\n甲：第{n}次检查，原话😀。\n"));
        }
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.episodes.len(), 80);
        let episode = source.episodes.last().unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body" && r.episode_id.as_ref() == Some(&episode.id))
            .cloned()
            .collect();
        let assets = vec![json!({"id":"A:v","baseAssetId":"A","name":"甲｜基础"})];
        let base = relevant_context(&source, episode, &assets, &Value::Null);
        let historical: Vec<_> = base.iter().filter(|r| r.kind == "body").collect();
        assert!(historical.len() >= 79);
        let chosen = &historical[0];
        let evidence = HashSet::from([chosen.id.clone()]);
        let owned = rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        let old = author_context(&base, &rows, &owned, &evidence, false);
        assert_eq!(
            json!(old),
            json!(base),
            "unmarked frozen workflows keep their exact old context"
        );
        let scoped = author_context(&base, &rows, &owned, &evidence, true);
        assert!(
            scoped.iter().any(|r| json!(r) == json!(chosen)),
            "explicit historical claims remain verbatim with coordinates"
        );
        for background in base.iter().filter(|r| r.kind == "context") {
            assert!(scoped.iter().any(|r| json!(r) == json!(background)));
        }
        assert_eq!(scoped.iter().filter(|r| r.kind == "body").count(), 1);
        let mut input = json!({"foundation":{"facts":source.rows.iter().map(|r|json!({"id":r.id,"sourceRefs":[r.id],"quotes":[r.text]})).collect::<Vec<_>>()}});
        let facts = scoped_facts(&input, &rows, &scoped);
        assert!(facts
            .iter()
            .all(|fact| rows.iter().chain(&scoped).any(|r| r.id == fact["id"])));
        assert!(facts.iter().all(|fact| fact.get("quotes").is_none()));
        assert!(!uses_owner_context(&input).unwrap());
        input[CONTEXT_INPUT_PROFILE_FIELD] = json!(CONTEXT_INPUT_PROFILE);
        assert!(uses_owner_context(&input).unwrap());
        input[CONTEXT_INPUT_PROFILE_FIELD] = json!("unknown");
        assert!(uses_owner_context(&input).is_err());
    }

    #[tokio::test]
    async fn invalid_spoken_range_is_rejected_and_never_accepted() {
        let source = prepare_source("第一集\n场景：门口\n小王：（低声）别动。\n").unwrap();
        let exec = Recorded {
            invalid_interval: true,
            ..Default::default()
        };
        let output = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(output["units"].as_array().unwrap().is_empty());
        assert_eq!(exec.accepted.lock().await.len(), 1);
        assert_eq!(exec.rejected.lock().await.len(), 1);
        assert!(output["blocked"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("演法"));
    }

    #[tokio::test]
    async fn review_waits_then_reuses_the_frozen_plan_with_an_empty_choice() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n").unwrap();
        let mut value = input(&source);
        value["settings"]["fantasyMode"] = json!("review");
        value["settings"]["requireFantasyReview"] = json!(true);
        let exec = Recorded {
            fantasy: true,
            ..Default::default()
        };
        let waiting = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(waiting["status"], "waiting_decision");
        assert_eq!(waiting["waitingDecision"]["selectionMode"], "review");
        assert!(waiting["blocked"].as_array().unwrap().is_empty());
        assert_eq!(exec.calls.lock().await.len(), 1);
        assert_eq!(exec.calls.lock().await[0].stage, "storyboardPlan");
        value["decisions"] = json!({source.episodes[0].id.clone():{"selectedFantasyIds":[]}});
        let done = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(done["status"], "completed");
        assert_eq!(
            exec.calls
                .lock()
                .await
                .iter()
                .filter(|request| request.stage == "storyboardPlan")
                .count(),
            1
        );
        let again = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(done["units"], again["units"]);
        assert_eq!(exec.calls.lock().await.len(), 3);
    }

    fn assert_recommended_fantasy_preserves_source(output: &Value, source: &LockedSource) {
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        assert_eq!(output["sourceSha256"], source.sha256);
        assert!(output["waitingDecisions"].as_array().unwrap().is_empty());
        assert_eq!(output["units"].as_array().unwrap().len(), 1);
        let unit = &output["units"][0];
        let shot = &unit["shots"][0];
        let row = source
            .rows
            .iter()
            .find(|row| row.dialogue.is_some())
            .unwrap();
        let dialogue = row.dialogue.as_ref().unwrap();
        assert_eq!(unit["sourceRefs"], json!([row.id]));
        assert_eq!(shot["dialogueParts"][0]["text"], dialogue.quote);
        assert_eq!(shot["dialogueParts"][0]["speaker"], dialogue.speaker);
        assert_eq!(shot["duration"], 3.0);
        assert_eq!(shot["dialogueParts"][0]["voiceStart"], 0.0);
        assert_eq!(shot["dialogueParts"][0]["voiceEnd"], 0.4);
        assert!(text(shot, "picture").contains("再收回实际身影"));
        assert_eq!(output["fantasyRealization"].as_array().unwrap().len(), 1);
        let realization = &output["fantasyRealization"][0];
        assert_eq!(realization["candidateId"], "F1");
        assert_eq!(realization["unitIds"], json!([unit["id"]]));
        assert_eq!(realization["shotIds"], json!([shot["id"]]));
        assert_eq!(realization["screenSeconds"], 1.0);
    }

    #[tokio::test]
    async fn default_fantasy_recommendation_completes_author_and_editor_without_review() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n").unwrap();
        let mut value = input(&source);
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("fantasyMode");
        let frozen_input = serde_json::to_string(&value).unwrap();
        let exec = Recorded {
            fantasy: true,
            recommended_fantasy: true,
            ..Default::default()
        };
        let done = run_storyboard(&value, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_recommended_fantasy_preserves_source(&done, &source);
        let calls = exec.calls.lock().await;
        assert_eq!(
            calls
                .iter()
                .map(|request| request.stage.as_str())
                .collect::<Vec<_>>(),
            vec!["storyboardPlan", "storyboardCreate", "storyboardFinish"]
        );
        assert!(calls
            .iter()
            .all(|request| request.input["settings"]["fantasyMode"] == "recommend"));
        assert!(calls.iter().all(|request| request.input["settings"]
            .get("requireFantasyReview")
            .is_none()));
        assert_eq!(calls[1].input["selectedFantasy"][0]["id"], "F1");
        assert_eq!(calls[2].input["selectedFantasy"][0]["id"], "F1");
        assert_eq!(serde_json::to_string(&value).unwrap(), frozen_input);
    }

    #[tokio::test]
    async fn unmarked_legacy_review_auto_recommends_with_the_identical_cached_plan() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n").unwrap();
        let mut legacy = input(&source);
        legacy["settings"]["fantasyMode"] = json!("review");
        let frozen_legacy = serde_json::to_string(&legacy).unwrap();
        let mut explicit_review = legacy.clone();
        explicit_review["settings"]["requireFantasyReview"] = json!(true);
        let exec = Recorded {
            fantasy: true,
            recommended_fantasy: true,
            ..Default::default()
        };
        // Obtain a real accepted plan through the same validation path. The
        // stop flag does not enter its request or replace its cached contents.
        let waiting = run_storyboard(&explicit_review, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(waiting["status"], "waiting_decision");
        assert!(waiting["units"].as_array().unwrap().is_empty());
        assert_eq!(exec.calls.lock().await.len(), 1);
        let cached_plan_request = exec.calls.lock().await[0].clone();
        let cached_plan_key = serde_json::to_string(&cached_plan_request).unwrap();
        let cached_plan = exec.accepted.lock().await[&cached_plan_key].clone();
        let done = run_storyboard(&legacy, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_recommended_fantasy_preserves_source(&done, &source);
        assert_eq!(exec.accepted.lock().await[&cached_plan_key], cached_plan);
        assert_eq!(exec.calls.lock().await.len(), 3);
        assert_eq!(
            exec.calls
                .lock()
                .await
                .iter()
                .filter(|request| request.stage == "storyboardPlan")
                .count(),
            1
        );
        assert_eq!(
            cached_plan_request.input["settings"]["fantasyMode"],
            "review"
        );
        assert!(cached_plan_request.input["settings"]
            .get("requireFantasyReview")
            .is_none());
        let again = run_storyboard(&legacy, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(done["units"], again["units"]);
        assert_eq!(done["fantasyRealization"], again["fantasyRealization"]);
        assert_eq!(
            exec.calls.lock().await.len(),
            3,
            "cache replay must not add a planner, author, or editor call"
        );
        assert_eq!(serde_json::to_string(&legacy).unwrap(), frozen_legacy);
        // An explicitly disabled review flag is also automatic, without a
        // changed planning request or another model call.
        legacy["settings"]["requireFantasyReview"] = json!(false);
        let disabled = run_storyboard(&legacy, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(done["units"], disabled["units"]);
        assert_eq!(exec.calls.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn failed_predecessor_blocks_dependent_episode_without_resetting_continuity() {
        let source =
            prepare_source("第一集\n场景：门口\n小王：别动。\n第二集\n场景：门口\n小王：回来。\n")
                .unwrap();
        let exec = Recorded {
            fail_episode: source.episodes[0].id.clone(),
            ..Default::default()
        };
        let output = run_storyboard(&input(&source), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(exec.calls.lock().await.len(), 1);
        assert_eq!(output["blocked"][1]["status"], "waiting_dependency");
        assert_eq!(output["blocked"][1]["dependsOn"], source.episodes[0].id);
    }

    #[tokio::test]
    async fn local_revision_runs_only_create_and_finish_and_preserves_other_units() {
        let source = prepare_source("第一集\n场景：门口\n小王：别动。\n小王：回来。\n").unwrap();
        let original = run_storyboard(
            &input(&source),
            &Recorded::default(),
            "gpt-6.1-sol",
            "medium",
        )
        .await
        .unwrap();
        let mut value = input(&source);
        value["selection"] = json!({"unitIds":[original["units"][0]["id"]]});
        value["existingUnits"] = original["units"].clone();
        value["revisionInstruction"] = json!("把本次回应拍得更清楚，保持原站位。");
        let exec = Recorded::default();
        let output = run_storyboard(&value, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "completed");
        assert_eq!(output["scope"], "units");
        assert_eq!(
            output["replacedUnitIds"],
            json!([original["units"][0]["id"]])
        );
        assert_eq!(output["units"][0]["id"], original["units"][0]["id"]);
        assert_eq!(output["units"].as_array().unwrap().len(), 1);
        assert_eq!(
            exec.calls
                .lock()
                .await
                .iter()
                .map(|request| request.stage.as_str())
                .collect::<Vec<_>>(),
            vec!["storyboardCreate", "storyboardFinish"]
        );
    }

    async fn accepted_revision_fixture() -> (LockedSource, Value, Value) {
        let source = prepare_source(
            "第一集\n场景：门口\n小王OS：我明白了。\n小王：我来。\n小王：你先走。\n",
        )
        .unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .collect();
        let plan_request = TextRequest {
            stage: "storyboardPlan".into(),
            skill: "episode-emotion-planner".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"sourceRows":rows}),
            schema: json!({}),
            rule_snapshot: json!({}),
        };
        let mut plan = mock_plan(&plan_request, true);
        plan["workPackages"][0]["scenarioKeys"] = json!(["long-voice"]);
        plan["entryFacts"] = json!([{"id":"known-entry","subject":"小王","statement":"小王在当前原行说出自己理解了信息。","sourceRefs":[rows[0].id],"evidenceMode":"source_rows"}]);
        plan["workPackages"][0]["entryFactIds"] = json!(["known-entry"]);
        plan["fantasyCandidates"][0]["recommended"] = json!(true);
        let first_exec = Recorded {
            plan_override: Some(plan),
            recommended_fantasy: true,
            linked_fantasy: true,
            ..Default::default()
        };
        let mut normal = input(&source);
        normal[creative::PROFILE_FIELD] = json!(creative::PROFILE);
        normal["settings"]["fantasyMode"] = json!("recommend");
        normal["capabilities"]["maxDuration"] = json!(4);
        let normal_bytes = serde_json::to_vec(&normal).unwrap();
        let original = run_storyboard(&normal, &first_exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(original["status"], "completed", "{}", original["blocked"]);
        assert_eq!(original["units"].as_array().unwrap().len(), 3);
        let calls = first_exec.calls.lock().await;
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.stage == "storyboardPlan")
                .count(),
            1
        );
        let planner = calls
            .iter()
            .find(|call| call.stage == "storyboardPlan")
            .unwrap();
        let author = calls
            .iter()
            .find(|call| call.stage == "storyboardCreate")
            .unwrap();
        let mut units = original["units"].clone();
        for unit in units.as_array_mut().unwrap() {
            unit["version"] = json!(1);
        }
        let context = json!({"version":"accepted-storyboard-revision/v1","sourceSha256":source.sha256,"episodeId":source.episodes[0].id,"originWorkflowId":"exact-adopted-origin","planJobId":"accepted-planner","planRequestHash":digest(&serde_json::to_string(planner).unwrap()),
            "unitBindings":units.as_array().unwrap().iter().map(|unit|json!({"unitId":unit["id"],"unitVersion":unit["version"],"unitSha256":digest(&unit.to_string())})).collect::<Vec<_>>(),
            "plan":original["plans"][0]["plan"],"selectedFantasy":author.input["selectedFantasy"],"fantasyRealization":original["fantasyRealization"]});
        assert_eq!(
            context["fantasyRealization"][0]["unitIds"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(serde_json::to_vec(&normal).unwrap(), normal_bytes);
        let mut revision = normal.clone();
        revision[creative::REVISION_PROFILE_FIELD] = json!(creative::REVISION_PROFILE);
        revision["existingUnits"] = units.clone();
        revision["selection"]["unitIds"] = json!([units[0]["id"]]);
        revision["revisionInstruction"]=json!("保留全部原话和既有光声持物，让OS中段的认识落实为源内观看变化；声音可连续跨镜，不为切镜另加吸气。");
        revision["revisionContexts"] = json!([context]);
        (source, revision, normal)
    }

    #[tokio::test]
    async fn accepted_plan_revision_preserves_original_emotion_cards_fantasy_group_and_two_role_scope(
    ) {
        let (source, request, normal) = accepted_revision_fixture().await;
        let frozen = serde_json::to_vec(&request).unwrap();
        let original_plan = &request["revisionContexts"][0]["plan"];
        let exec = Recorded::default();
        let output = run_storyboard(&request, &exec, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(output["status"], "completed", "{}", output["blocked"]);
        assert_eq!(output["scope"], "units");
        assert_eq!(
            output["replacedUnitIds"],
            json!([
                request["existingUnits"][0]["id"],
                request["existingUnits"][1]["id"]
            ])
        );
        assert_eq!(output["units"].as_array().unwrap().len(), 2);
        assert!(output["units"]
            .as_array()
            .unwrap()
            .iter()
            .all(|unit| unit["id"] != request["existingUnits"][2]["id"]));
        let calls = exec.calls.lock().await;
        assert_eq!(
            calls
                .iter()
                .map(|call| call.stage.as_str())
                .collect::<Vec<_>>(),
            vec!["storyboardCreate", "storyboardFinish"]
        );
        for call in calls.iter() {
            assert_eq!(
                call.input["revisionInstruction"],
                request["revisionInstruction"]
            );
            assert_eq!(
                call.input["workPackages"][0]["emotionContract"],
                original_plan["workPackages"][0]["emotionContract"]
            );
            assert_eq!(
                call.input["workPackages"][0]["id"],
                original_plan["workPackages"][0]["id"]
            );
            assert_eq!(
                call.input["plan"]["emotionArc"],
                original_plan["emotionArc"]
            );
            assert_eq!(
                call.input["plan"]["events"][0]["emotionContract"],
                original_plan["events"][0]["emotionContract"]
            );
            assert_eq!(call.input["entryFacts"], original_plan["entryFacts"]);
            assert_eq!(
                call.input["workPackages"][0]["beatContracts"][0]["emotionContract"],
                original_plan["workPackages"][0]["beatContracts"][0]["emotionContract"]
            );
            assert_eq!(
                call.input["workPackages"][0]["beatContracts"][0]["sourceRefs"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert!(call.rule_snapshot["methods"]
                .as_array()
                .unwrap()
                .iter()
                .any(|method| method["id"] == "storyboard-card-long-voice"));
            assert_eq!(
                call.rule_snapshot["primary"]["sha256"],
                rule_snapshot(&call.skill).unwrap()["sha256"]
            );
            assert_eq!(
                call.rule_snapshot["runtimeContract"]["id"],
                creative::REVISION_PROFILE
            );
            assert!(text(&call.rule_snapshot["runtimeContract"], "content")
                .contains("本可信规约授权两岗"));
            assert_eq!(
                call.input["selectedFantasy"],
                request["revisionContexts"][0]["selectedFantasy"]
            );
            assert_eq!(call.input["sourceRows"].as_array().unwrap().len(), 2);
            assert!(!call.input["sourceRows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == request["existingUnits"][2]["sourceRefs"][0]));
            assert!(call.input["contextRows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == request["existingUnits"][2]["sourceRefs"][0]));
            assert_eq!(
                call.input["revisionProvenance"]["planRequestHash"],
                request["revisionContexts"][0]["planRequestHash"]
            );
        }
        let spoken: String = output["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|unit| unit["shots"].as_array().unwrap())
            .flat_map(|shot| shot["dialogueParts"].as_array().unwrap())
            .map(|part| text(part, "text"))
            .collect();
        let expected: String = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .take(2)
            .filter_map(|row| row.dialogue.as_ref())
            .map(spoken_text)
            .collect();
        assert_eq!(spoken, expected);
        assert_eq!(
            output["fantasyRealization"][0]["candidateId"],
            request["revisionContexts"][0]["selectedFantasy"][0]["id"]
        );
        assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
        assert!(normal.get(creative::REVISION_PROFILE_FIELD).is_none());
    }

    #[tokio::test]
    async fn accepted_plan_revision_rejects_stale_or_fabricated_provenance_without_model_calls() {
        let (_, request, _) = accepted_revision_fixture().await;
        let mut invalids = Vec::new();
        let mut missing = request.clone();
        missing.as_object_mut().unwrap().remove("revisionContexts");
        invalids.push(missing);
        let mut stale = request.clone();
        stale["revisionContexts"][0]["unitBindings"][0]["unitSha256"] = json!("wrong");
        invalids.push(stale);
        let mut source = request.clone();
        source["revisionContexts"][0]["sourceSha256"] = json!("old-source");
        invalids.push(source);
        let mut fantasy = request.clone();
        fantasy["revisionContexts"][0]["selectedFantasy"][0]["id"] = json!("unselected");
        invalids.push(fantasy);
        let mut bad_row = request.clone();
        let row = bad_row["revisionContexts"][0]["plan"]["events"][0]["sourceRefs"][0]
            .as_str()
            .unwrap()
            .to_owned();
        bad_row["revisionContexts"][0]["plan"]["sourceFingerprints"][&row] =
            json!("not-current-hash");
        invalids.push(bad_row);
        for invalid in invalids {
            let exec = Recorded::default();
            let output = run_storyboard(&invalid, &exec, "offline", "medium")
                .await
                .unwrap();
            assert_eq!(output["status"], "partially_blocked");
            assert_eq!(output["blocked"][0]["code"], "REVISION_PROVENANCE_INVALID");
            assert!(exec.calls.lock().await.is_empty());
        }
    }

    #[test]
    fn utf16_selection_rejects_surrogate_splits_and_duplicate_source_intervals() {
        let source = prepare_source("第一集\n场景：门口\n小王：😀。\n").unwrap();
        let quote = source.rows[2].dialogue.as_ref().unwrap();
        let mut draft = json!({"units":[{"shots":[{"dialogueParts":[{"lineId":quote.id,"quoteStart":quote.range.start,"quoteEnd":quote.range.start+1}]}]}]});
        assert!(materialize_dialogue(&source, &mut draft, false).is_err());
        let part = draft["units"][0]["shots"][0]["dialogueParts"][0]
            .as_object_mut()
            .unwrap();
        part.remove("quoteStart");
        part.remove("quoteEnd");
        part.insert("text".into(), json!("改写原话"));
        assert!(materialize_dialogue(&source, &mut draft, false).is_err());
        let repeat_source = prepare_source("第一集\n场景：门口\n小王：哈哈\n").unwrap();
        let quote = repeat_source.rows[2].dialogue.as_ref().unwrap();
        let part = json!({"lineId":quote.id,"quoteStart":quote.range.start,"quoteEnd":quote.range.start+1});
        let mut repeated = json!({"units":[{"shots":[{"dialogueParts":[part.clone(),part]}]}]});
        assert!(materialize_dialogue(&repeat_source, &mut repeated, false).is_err());
    }
}
