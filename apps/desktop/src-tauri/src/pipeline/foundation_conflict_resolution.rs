//! Versioned, source-bound conflict decisions. Model output never replaces the
//! saved catalogue/raw result: Rust applies explicit operations to frozen state.
use super::*;

pub(super) const VERSION: &str = "foundation-conflict-resolution/v1";
const RECORDS: &str = "foundationIdentityDecisions";

#[derive(Clone)]
pub(super) struct Manifest {
    pub input: Value,
    pub schema: Value,
    pub digest: String,
    items: Vec<Item>,
    batch: Batch,
}

#[derive(Clone)]
struct Item {
    id: String,
    path: String,
    source_id: String,
    target_id: Option<String>,
    current: Option<Value>,
    candidate: Option<Value>,
    expected: String,
}

pub(super) struct ResolutionOutcome {
    pub proposal: FoundationProposal,
    pub review_items: Vec<FoundationIssue>,
    pub distinct_pairs: Vec<(String, String)>,
}

fn fault(message: &str) -> String {
    format!("FOUNDATION_CONFLICT_RESOLUTION: {message}")
}

fn present_hash(value: &Option<Value>) -> Result<String, String> {
    value_sha(&json!({"present":value.is_some(),"value":value}))
}

fn is_identity(path: &str) -> Option<(&str, &str)> {
    let (left, right) = path
        .strip_prefix("/assets/")?
        .split_once("/identity-collision-with/")?;
    (!left.is_empty() && !right.is_empty() && !left.contains('/') && !right.contains('/'))
        .then_some((left, right))
}

fn field_allowed(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.first() != Some(&"assets") || parts.len() < 3 {
        return false;
    }
    let leaf = parts.last().copied().unwrap_or("");
    // Identity, source coordinates, scope, arrays and arbitrary metadata are
    // not writable through a text/design decision.
    matches!(leaf, "description" | "note" | "relationNote" | "role")
        || (parts.get(parts.len().saturating_sub(2)) == Some(&"preparation")
            && matches!(leaf, "mode" | "fromKey"))
        || leaf == "sourceVariantId"
}

pub(super) fn build_manifest(
    source: &LockedSource,
    batch: &Batch,
    basis: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &FoundationProposal,
    original_input: &Value,
    original_result: &Value,
    conflicts: &[String],
) -> Result<Manifest, String> {
    if [basis, current, candidate]
        .iter()
        .any(|p| p.source_sha256 != source.sha256)
    {
        return Err(fault("基稿来源不一致"));
    }
    let mut paths = conflicts.to_vec();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(fault("冲突清单为空"));
    }
    let cv = json!(current);
    let nv = json!(candidate);
    let mut items = vec![];
    for path in &paths {
        let (source_id, target_id, old, next) = if let Some((left, right)) = is_identity(path) {
            let a = candidate
                .assets
                .iter()
                .find(|a| a.id == left)
                .ok_or_else(|| fault("候选身份不存在"))?;
            let b = current
                .assets
                .iter()
                .find(|a| a.id == right)
                .ok_or_else(|| fault("目标身份不存在"))?;
            if a.kind != b.kind
                || basis.assets.iter().any(|a| a.id == left)
                || current.assets.iter().any(|a| a.id == left)
            {
                return Err(fault("身份冲突不是新增同类身份"));
            }
            (
                left.to_owned(),
                Some(right.to_owned()),
                Some(json!(b)),
                Some(json!(a)),
            )
        } else {
            let id = path
                .strip_prefix("/assets/")
                .and_then(|s| s.split('/').next())
                .ok_or_else(|| fault("不支持非资产冲突"))?;
            if !current
                .assets
                .iter()
                .chain(&candidate.assets)
                .any(|a| a.id == id)
            {
                return Err(fault("冲突对象不存在"));
            }
            (
                id.to_owned(),
                None,
                identity_path(&cv, path).cloned(),
                identity_path(&nv, path).cloned(),
            )
        };
        let expected = present_hash(&old)?;
        let id = format!(
            "conflict:{}",
            &value_sha(&json!([source.sha256, batch.scope, path, old, next]))?[..24]
        );
        items.push(Item {
            id,
            path: path.clone(),
            source_id,
            target_id,
            current: old,
            candidate: next,
            expected,
        });
    }
    let stage = if original_result["format"] == "ASSET_OPTIMIZATION_DESKTOP_V2" {
        "Q1"
    } else {
        "F2"
    };
    let original_hash = value_sha(original_result)?;
    let cause = value_sha(
        &json!({"sourceSha256":source.sha256,"stage":stage,"scope":batch.scope,"originalResultSha256":original_hash,"conflicts":paths}),
    )?;
    let mut input = json!({"version":VERSION,"sourceSha256":source.sha256,"stage":stage,"scope":batch.scope,
        "originalInputSha256":value_sha(original_input)?,"originalResultSha256":original_hash,
        "basisSha256":value_sha(basis)?,"currentBasisSha256":value_sha(current)?,"candidateSha256":value_sha(candidate)?,
        "issueSetSha256":value_sha(&paths)?,"causeSha256":cause,"conflicts":paths,
        "items":items.iter().map(|i|json!({"conflictId":i.id,"path":i.path,"sourceAssetId":i.source_id,"targetAssetId":i.target_id,"expectedOldHash":i.expected,"currentPresent":i.current.is_some(),"current":i.current,"candidatePresent":i.candidate.is_some(),"candidate":i.candidate,"fieldWritable":field_allowed(&i.path)})).collect::<Vec<_>>(),
        "contract":"仅提交decisions，不重交资产目录。名称/别名重合只是候选，不证明同一身份。keep_distinct须有来源证据确认两个不同身份；无法判定时defer_identity保留独立且标待核对；bind_asset复用目标基础形象；bind_variant/record_use仅登记目标已有形象的本批用途，不能改写目标设计。observed名称与canonical名称分开保留。字段采用只选current/candidate；design_change仅允许指定文本叶字段。不得改原文、集场、用途范围或未知身份。证据只引用实际提供的本批rowId。无法证明同一身份时保留独立待核对，不强行归并。"});
    let digest = value_sha(&input)?;
    input["manifestSha256"] = json!(digest);
    let row_ids: Vec<_> = batch
        .pieces
        .iter()
        .map(|p| p.row_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let choices: Vec<_>=items.iter().map(|item| {
        let identity=item.target_id.is_some();
        let actions=if identity {vec!["keep_distinct","defer_identity","bind_asset","bind_variant","record_use"]} else if field_allowed(&item.path) {vec!["adopt_current","adopt_candidate","design_change"]} else {vec!["adopt_current"]};
        let variants:Vec<_>=item.target_id.as_ref().and_then(|id|current.assets.iter().find(|a|&a.id==id)).into_iter().flat_map(|a|&a.variants).map(|v|json!(v.id)).chain(std::iter::once(Value::Null)).collect();
        json!({"type":"object","additionalProperties":false,"required":["conflictId","expectedOldHash","action","targetAssetId","targetVariantId","take","replacementText","reason","evidenceRows"],"properties":{
            "conflictId":{"type":"string","enum":[item.id]},"expectedOldHash":{"type":"string","enum":[item.expected]},"action":{"type":"string","enum":actions},
            "targetAssetId":{"type":["string","null"],"enum":[item.target_id]},"targetVariantId":{"type":["string","null"],"enum":variants},
            "take":{"type":["string","null"],"enum":["current","candidate",null]},"replacementText":{"type":["string","null"]},"reason":{"type":"string","minLength":1},"evidenceRows":{"type":"array","uniqueItems":true,"items":{"$ref":"#/$defs/sourceRow"}}}})
    }).collect();
    let schema = json!({"type":"object","$defs":{"sourceRow":{"type":"string","enum":row_ids}},"additionalProperties":false,"required":["version","manifestSha256","decisions"],"properties":{"version":{"type":"string","enum":[VERSION]},"manifestSha256":{"type":"string","enum":[digest]},"decisions":{"type":"array","minItems":items.len(),"maxItems":items.len(),"items":{"anyOf":choices}}}});
    Ok(Manifest {
        input,
        schema,
        digest,
        items,
        batch: batch.clone(),
    })
}

fn verify_manifest(
    m: &Manifest,
    source: &LockedSource,
    basis: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &FoundationProposal,
) -> Result<(), String> {
    let mut packet = m.input.clone();
    packet
        .as_object_mut()
        .ok_or_else(|| fault("清单不是对象"))?
        .remove("manifestSha256");
    if value_sha(&packet)? != m.digest
        || m.input["sourceSha256"] != source.sha256
        || m.input["basisSha256"] != value_sha(basis)?
        || m.input["currentBasisSha256"] != value_sha(current)?
        || m.input["candidateSha256"] != value_sha(candidate)?
    {
        return Err(fault("清单或基稿已改变，禁止应用旧决议"));
    }
    Ok(())
}

fn review(m: &Manifest, item: &Item, reason: &str) -> FoundationIssue {
    FoundationIssue {
        coordinate: format!("conflict-review:{}", item.id),
        problem: format!("资产冲突待核对：{reason}"),
        source_refs: vec![],
        blocking: false,
        asset_id: Some(item.source_id.clone()),
        variant_id: None,
        episode_ids: m.batch.scope.clone(),
        details: Some(
            json!({"owner":"PROGRAM","kind":"FOUNDATION_CONFLICT_REVIEW","status":"PENDING","conflictId":item.id,"path":item.path,"causeSha256":m.input["causeSha256"],"manifestSha256":m.digest,"scope":m.batch.scope,"reason":reason}),
        ),
    }
}

fn pair_record(m: &Manifest, item: &Item, status: &str, reason: &str) -> Value {
    let mut pair = vec![
        item.source_id.clone(),
        item.target_id.clone().unwrap_or_default(),
    ];
    pair.sort();
    json!({"id":format!("identity-decision:{}",item.id),"version":VERSION,"sourceSha256":m.input["sourceSha256"],"assetIds":pair,"status":status,"manifestSha256":m.digest,"conflictId":item.id,"reason":reason})
}

pub(super) fn distinct_pair_recorded(
    proposal: &FoundationProposal,
    left: &str,
    right: &str,
) -> bool {
    let mut pair = vec![left, right];
    pair.sort();
    proposal.assets.iter().any(|a| {
        a.metadata
            .get(RECORDS)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|r| {
                r["version"] == VERSION
                    && r["sourceSha256"] == proposal.source_sha256
                    && r["status"] == "DISTINCT"
                    && r["assetIds"] == json!(pair)
            })
    })
}

/// Pending pairs are intentionally separate from adjudicated DISTINCT pairs.
/// They suppress repeated reconciliation only while their visible issue exists.
pub(super) fn deferred_pair_recorded(
    proposal: &FoundationProposal,
    left: &str,
    right: &str,
) -> bool {
    let mut pair = vec![left, right];
    pair.sort();
    proposal.assets.iter().any(|a| {
        a.metadata
            .get(RECORDS)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|r| {
                r["version"] == VERSION
                    && r["sourceSha256"] == proposal.source_sha256
                    && r["status"] == "PENDING"
                    && r["assetIds"] == json!(pair)
                    && proposal.issues.iter().any(|i| {
                        i.details.as_ref().is_some_and(|d| {
                            d["kind"] == "FOUNDATION_CONFLICT_REVIEW"
                                && d["status"] == "PENDING"
                                && d["conflictId"] == r["conflictId"]
                        })
                    })
            })
    })
}

fn append_record(asset: &mut FoundationAsset, key: &str, value: Value) -> Result<(), String> {
    let values = asset
        .metadata
        .entry(key)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| fault("历史决议元数据类型不正确"))?;
    if !values.contains(&value) {
        values.push(value);
    }
    Ok(())
}

fn remap_references(value: &mut Value, mapping: &HashMap<String, String>, field: &str) {
    match value {
        Value::String(s)
            if [
                "assetId",
                "variantId",
                "sourceVariantId",
                "fromKey",
                "baseKey",
                "parentAssetId",
                "parentVariantId",
                "subjectAssetKeys",
            ]
            .contains(&field) =>
        {
            if let Some(target) = mapping.get(s) {
                *s = target.clone();
            }
        }
        Value::Array(items) => {
            for v in items {
                remap_references(v, mapping, field)
            }
        }
        Value::Object(object) => {
            for (k, v) in object {
                remap_references(v, mapping, k)
            }
        }
        _ => {}
    }
}
fn has_old_reference(value: &Value, mapping: &HashMap<String, String>) -> bool {
    match value {
        Value::String(s) => mapping.contains_key(s),
        Value::Array(a) => a.iter().any(|v| has_old_reference(v, mapping)),
        Value::Object(o) => o.values().any(|v| has_old_reference(v, mapping)),
        _ => false,
    }
}

/// Preserve target design exactly; only explicit source-bound use is extended.
/// Unsafe dependency maps are deferred instead of guessing a replacement.
fn bind_identity(
    m: &Manifest,
    item: &Item,
    decision: &Value,
    source: &LockedSource,
    basis: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &mut Value,
    resolutions: &mut HashMap<String, Option<Value>>,
) -> Result<(), String> {
    let source_asset: FoundationAsset =
        serde_json::from_value(item.candidate.clone().ok_or_else(|| fault("缺候选身份"))?)
            .map_err(|e| e.to_string())?;
    let target_id = item
        .target_id
        .as_deref()
        .ok_or_else(|| fault("缺目标身份"))?;
    let target = current
        .assets
        .iter()
        .find(|a| a.id == target_id)
        .ok_or_else(|| fault("目标身份不存在"))?;
    if m.items.iter().any(|other| {
        other.id != item.id && (other.source_id == item.source_id || other.source_id == target.id)
    }) {
        return Err(fault(
            "同一身份存在多目标或设计冲突，无法局部证明绑定；保留独立待核对",
        ));
    }
    if let Some(other) = candidate["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["id"] == target.id)
    {
        let original = basis
            .assets
            .iter()
            .find(|a| a.id == target.id)
            .map(|a| json!(a));
        if Some(other) != original.as_ref() && *other != json!(target) {
            return Err(fault("目标同时具有候选设计修改，不能以身份绑定覆盖"));
        }
    }
    if decision["targetAssetId"] != target_id {
        return Err(fault("目标身份越界"));
    }
    let requested = decision["targetVariantId"].as_str();
    let target_variant = if decision["action"] == "bind_asset" {
        if requested.is_some() {
            return Err(fault("bind_asset不能隐含选择变体"));
        }
        target
            .variants
            .iter()
            .find(|v| v.metadata.get("isBase") == Some(&json!(true)))
            .ok_or_else(|| fault("目标没有明确基础形象"))?
    } else {
        target
            .variants
            .iter()
            .find(|v| Some(v.id.as_str()) == requested)
            .ok_or_else(|| fault("目标变体不属于目标身份"))?
    };
    let bases: Vec<_> = source_asset
        .variants
        .iter()
        .filter(|v| v.metadata.get("isBase") == Some(&json!(true)))
        .collect();
    if bases.len() != 1 || source_asset.variants.len() != 1 {
        return Err(fault("候选含多个形象，无法证明完整变体映射；保留独立身份"));
    }
    let base = bases[0];
    if base.scope.kind != "SCENES" || base.scope.scenes.is_empty() {
        return Err(fault("候选用途不是明确的本批场次"));
    }
    let evidence: Vec<_> = decision["evidenceRows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if evidence.is_empty() {
        return Err(fault("身份绑定缺本批原文证据"));
    }
    for place in &base.scope.scenes {
        if !m.batch.scope.contains(&place.episode_id)
            || !source
                .scenes
                .iter()
                .any(|s| s.id == place.scene_id && s.episode_id == place.episode_id)
            || !m.batch.pieces.iter().any(|p| {
                source.rows.iter().any(|r| {
                    r.id == p.row_id
                        && r.episode_id.as_ref() == Some(&place.episode_id)
                        && r.scene_id.as_ref() == Some(&place.scene_id)
                })
            })
        {
            return Err(fault("绑定用途超出原批原文"));
        }
    }
    if !evidence.iter().any(|id| {
        source.rows.iter().any(|r| {
            r.id == *id
                && base.scope.scenes.iter().any(|p| {
                    r.episode_id.as_ref() == Some(&p.episode_id)
                        && r.scene_id.as_ref() == Some(&p.scene_id)
                })
        })
    }) {
        return Err(fault("绑定证据没有覆盖候选用途"));
    }
    let mapping = HashMap::from([
        (source_asset.id.clone(), target.id.clone()),
        (base.id.clone(), target_variant.id.clone()),
    ]);
    let mut changed = candidate.clone();
    changed["assets"]
        .as_array_mut()
        .ok_or_else(|| fault("候选目录损坏"))?
        .retain(|a| a["id"] != source_asset.id);
    remap_references(&mut changed, &mapping, "");
    if has_old_reference(&changed, &mapping) {
        return Err(fault("仍有未支持的身份依赖字段，不能删除候选身份"));
    }
    let path = format!("/assets/{}", target.id);
    let mut adopted: FoundationAsset = match resolutions.get(&path).cloned().flatten() {
        Some(v) => serde_json::from_value(v).map_err(|e| e.to_string())?,
        None => target.clone(),
    };
    let variant = adopted
        .variants
        .iter_mut()
        .find(|v| v.id == target_variant.id)
        .ok_or_else(|| fault("目标形象失效"))?;
    for place in base
        .scope
        .scenes
        .iter()
        .filter(|_| target_variant.scope.kind != "GLOBAL")
    {
        if !variant
            .scope
            .scenes
            .iter()
            .any(|p| p.episode_id == place.episode_id && p.scene_id == place.scene_id)
        {
            variant.scope.scenes.push(place.clone());
        }
        if !variant
            .applicability
            .iter()
            .any(|p| p.episode_id == place.episode_id && p.scene_id == place.scene_id)
        {
            variant.applicability.push(place.clone());
        }
    }
    if variant.scope.kind != "GLOBAL" {
        variant.scope.kind = "SCENES".into();
    }
    for id in base
        .source_refs
        .iter()
        .chain(&source_asset.source_refs)
        .filter(|id| m.batch.pieces.iter().any(|p| &p.row_id == *id))
    {
        if !variant.source_refs.contains(id) {
            variant.source_refs.push(id.clone());
        }
        if !adopted.source_refs.contains(id) {
            adopted.source_refs.push(id.clone());
        }
    }
    let uses = adopted
        .metadata
        .entry("usageEpisodes")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| fault("目标用途目录类型不正确"))?;
    for place in &base.scope.scenes {
        let episode = json!(place.episode_id);
        if !uses.contains(&episode) {
            uses.push(episode);
        }
    }
    append_record(
        &mut adopted,
        "foundationObservedUses",
        json!({"id":format!("observed-use:{}",item.id),"sourceSha256":source.sha256,"manifestSha256":m.digest,"conflictId":item.id,"observedName":source_asset.name,"canonicalName":target.name,"targetAssetId":target.id,"targetVariantId":target_variant.id,"sourceAssetId":source_asset.id,"sourceVariantId":base.id,"scenes":base.scope.scenes,"evidenceRows":evidence,"reason":decision["reason"]}),
    )?;
    resolutions.insert(path, Some(json!(adopted)));
    *candidate = changed;
    Ok(())
}

fn finish(
    m: &Manifest,
    source: &LockedSource,
    basis: &FoundationProposal,
    current: &FoundationProposal,
    candidate: Value,
    resolutions: HashMap<String, Option<Value>>,
    reviews: Vec<FoundationIssue>,
    records: Vec<(String, Value)>,
    distinct_pairs: Vec<(String, String)>,
) -> Result<ResolutionOutcome, String> {
    let mut remaining = vec![];
    let merged = merge_resolved_value(
        Some(&json!(basis)),
        Some(&json!(current)),
        Some(&candidate),
        "",
        &mut remaining,
        &resolutions,
    )
    .ok_or_else(|| fault("合并结果为空"))?;
    if !remaining.is_empty() {
        return Err(fault(&format!(
            "存在未列出的新冲突：{}",
            remaining.join(";")
        )));
    }
    let mut proposal: FoundationProposal =
        serde_json::from_value(merged).map_err(|e| e.to_string())?;
    for (id, record) in records {
        let asset = proposal
            .assets
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| fault("保留身份不存在"))?;
        append_record(asset, RECORDS, record)?;
    }
    for issue in &reviews {
        if !proposal
            .issues
            .iter()
            .any(|i| i.coordinate == issue.coordinate)
        {
            proposal.issues.push(issue.clone());
        }
    }
    // Full source lock, dependency, usage and catalogue validation stays mandatory.
    let proposal = finalize_merged(json!(proposal), source).map_err(|e| fault(&e.join(";")))?;
    if proposal.source_sha256 != m.input["sourceSha256"].as_str().unwrap_or("") {
        return Err(fault("合并来源漂移"));
    }
    Ok(ResolutionOutcome {
        proposal,
        review_items: reviews,
        distinct_pairs,
    })
}

pub(super) fn apply(
    m: &Manifest,
    response: &Value,
    source: &LockedSource,
    basis: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &FoundationProposal,
) -> Result<ResolutionOutcome, String> {
    verify_manifest(m, source, basis, current, candidate)?;
    let object = response.as_object().ok_or_else(|| fault("决议不是对象"))?;
    if object.len() != 3 || response["version"] != VERSION || response["manifestSha256"] != m.digest
    {
        return Err(fault("决议版本或基稿摘要不匹配"));
    }
    let decisions = response["decisions"]
        .as_array()
        .ok_or_else(|| fault("缺decisions"))?;
    if decisions.len() != m.items.len() {
        return Err(fault("决议数量不等于冲突数量"));
    }
    let mut seen = HashSet::new();
    let mut resolutions = HashMap::new();
    let mut changed = json!(candidate);
    let mut reviews = vec![];
    let mut records = vec![];
    let mut distinct = vec![];
    for decision in decisions {
        let fields = decision
            .as_object()
            .ok_or_else(|| fault("决议条目不是对象"))?;
        let required = [
            "conflictId",
            "expectedOldHash",
            "action",
            "targetAssetId",
            "targetVariantId",
            "take",
            "replacementText",
            "reason",
            "evidenceRows",
        ];
        if fields.len() != required.len() || required.iter().any(|k| !fields.contains_key(*k)) {
            return Err(fault("决议字段不符合固定合同"));
        }
        let id = decision["conflictId"]
            .as_str()
            .ok_or_else(|| fault("缺conflictId"))?;
        if !seen.insert(id) {
            return Err(fault("冲突决议重复"));
        }
        let item = m
            .items
            .iter()
            .find(|i| i.id == id)
            .ok_or_else(|| fault("未知冲突ID"))?;
        if decision["expectedOldHash"] != item.expected
            || decision["targetAssetId"] != json!(item.target_id)
        {
            return Err(fault("目标或预期旧值不匹配"));
        }
        let reason = decision["reason"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| fault("决议缺依据说明"))?;
        let evidence = decision["evidenceRows"]
            .as_array()
            .ok_or_else(|| fault("证据不是数组"))?;
        let mut evidence_seen = HashSet::new();
        for id in evidence {
            let id = id.as_str().ok_or_else(|| fault("证据ID不是字符串"))?;
            if !evidence_seen.insert(id) || !m.batch.pieces.iter().any(|p| p.row_id == id) {
                return Err(fault("决议证据越过本批或重复"));
            }
        }
        let action = decision["action"].as_str().unwrap_or("");
        if item.target_id.is_some() {
            if !decision["take"].is_null() || !decision["replacementText"].is_null() {
                return Err(fault("身份决议不能附带设计重写"));
            }
            if action == "keep_distinct" || action == "defer_identity" {
                if !decision["targetVariantId"].is_null() {
                    return Err(fault("不同身份不能绑定变体"));
                }
                if action == "defer_identity" || evidence.is_empty() {
                    reviews.push(review(m, item, reason));
                    records.push((item.source_id.clone(), pair_record(m, item, "PENDING", reason)));
                } else {
                    records.push((item.source_id.clone(), pair_record(m, item, "DISTINCT", reason)));
                    distinct.push((item.source_id.clone(), item.target_id.clone().unwrap()));
                }
            } else if ["bind_asset", "bind_variant", "record_use"].contains(&action) {
                if let Err(error) = bind_identity(
                    m,
                    item,
                    decision,
                    source,
                    basis,
                    current,
                    &mut changed,
                    &mut resolutions,
                ) {
                    reviews.push(review(m, item, &error));
                    records.push((
                        item.source_id.clone(),
                        pair_record(m, item, "PENDING", &error),
                    ));
                }
            } else {
                return Err(fault("身份冲突动作无效"));
            }
        } else {
            if !decision["targetVariantId"].is_null() {
                return Err(fault("字段决议不能绑定变体"));
            }
            let value = match action {
                "adopt_current"
                    if decision["take"] == "current" && decision["replacementText"].is_null() =>
                {
                    item.current.clone()
                }
                "adopt_candidate"
                    if field_allowed(&item.path)
                        && decision["take"] == "candidate"
                        && decision["replacementText"].is_null() =>
                {
                    item.candidate.clone()
                }
                "design_change"
                    if field_allowed(&item.path)
                        && decision["take"].is_null()
                        && ["description", "note", "relationNote"]
                            .contains(&item.path.rsplit('/').next().unwrap_or(""))
                        && !evidence.is_empty() =>
                {
                    Some(json!(decision["replacementText"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| fault("设计修订缺文本"))?))
                }
                _ => return Err(fault("字段决议动作或取值越界")),
            };
            if !field_allowed(&item.path) {
                reviews.push(review(
                    m,
                    item,
                    "非设计字段冲突保留当前值，待人工/专用流程核对",
                ));
            }
            resolutions.insert(item.path.clone(), value);
        }
    }
    finish(
        m,
        source,
        basis,
        current,
        changed,
        resolutions,
        reviews,
        records,
        distinct,
    )
}

pub(super) fn fallback(
    m: &Manifest,
    source: &LockedSource,
    basis: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &FoundationProposal,
    reason: &str,
) -> Result<ResolutionOutcome, String> {
    verify_manifest(m, source, basis, current, candidate)?;
    let mut resolutions = HashMap::new();
    let mut reviews = vec![];
    let mut records = vec![];
    for item in &m.items {
        reviews.push(review(m, item, reason));
        if item.target_id.is_some() {
            records.push((
                item.source_id.clone(),
                pair_record(m, item, "PENDING", reason),
            ));
        } else {
            resolutions.insert(item.path.clone(), item.current.clone());
        }
    }
    finish(
        m,
        source,
        basis,
        current,
        json!(candidate),
        resolutions,
        reviews,
        records,
        vec![],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        source: LockedSource,
        batch: Batch,
        basis: FoundationProposal,
        current: FoundationProposal,
        candidate: FoundationProposal,
        raw: Value,
    }
    fn asset(source: &LockedSource, id: &str, name: &str, episode: usize) -> FoundationAsset {
        let scene = source
            .scenes
            .iter()
            .find(|s| s.episode_id == source.episodes[episode].id)
            .unwrap();
        let row = source
            .rows
            .iter()
            .find(|r| r.scene_id.as_deref() == Some(scene.id.as_str()))
            .unwrap();
        let place = Applicability {
            episode_id: scene.episode_id.clone(),
            scene_id: scene.id.clone(),
        };
        FoundationAsset {
            id: id.into(),
            name: name.into(),
            kind: "PROP".into(),
            aliases: vec!["重盾".into()],
            description: "原设计保持".into(),
            source_refs: vec![row.id.clone()],
            variants: vec![FoundationVariant {
                id: format!("{id}:base"),
                name: name.into(),
                description: "母版原设计".into(),
                source_variant_id: None,
                source_refs: vec![row.id.clone()],
                applicability: vec![place.clone()],
                scope: FoundationScope {
                    kind: "SCENES".into(),
                    scenes: vec![place],
                    reason: None,
                },
                asset_row_id: None,
                metadata: serde_json::Map::from_iter([("isBase".into(), json!(true))]),
            }],
            metadata: Map::new(),
        }
    }
    fn fixture() -> Fixture {
        let source=prepare_source("第1集\n场景：整备库\n△司空狄把钻头装到钛盾上。\n第2集\n场景：竞技场\n△司空狄拖行钻头重盾，钻头犁地。\n").unwrap();
        let all: Vec<_> = source.episodes.iter().map(|e| e.id.clone()).collect();
        let (_, basis, _) =
            delivery_basis(&source, &json!({"selection":{"episodeIds":all}})).unwrap();
        let scope = vec![source.episodes[1].id.clone()];
        let batch = Batch {
            pieces: whole_rows(&source, &scope),
            scope,
            fragment: None,
        };
        let mut target = asset(&source, "shield", "资格赛钛盾", 0);
        let mut drill = target.variants[0].clone();
        drill.id = "shield:drill".into();
        drill.name = "废钻头高频反震盾".into();
        drill.description = "焊接倒钩钻头，固定高频震荡器".into();
        drill.source_variant_id = Some("shield:base".into());
        drill.metadata.insert("isBase".into(), json!(false));
        target.variants.push(drill);
        let mut current = basis.clone();
        current.assets.push(target);
        let mut candidate = basis.clone();
        candidate
            .assets
            .push(asset(&source, "observed", "实体钻头重盾", 1));
        let raw = json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":source.sha256,"coverage":batch.scope,"assets":[{"name":"实体钻头重盾","type":"PROP","episodes":batch.scope}]});
        Fixture {
            source,
            batch,
            basis,
            current,
            candidate,
            raw,
        }
    }
    fn manifest(f: &Fixture) -> Manifest {
        build_manifest(
            &f.source,
            &f.batch,
            &f.basis,
            &f.current,
            &f.candidate,
            &json!({"source":"frozen"}),
            &f.raw,
            &["/assets/observed/identity-collision-with/shield".into()],
        )
        .unwrap()
    }
    fn decision(m: &Manifest, f: &Fixture, action: &str) -> Value {
        json!({"version":VERSION,"manifestSha256":m.digest,"decisions":[{"conflictId":m.items[0].id,"expectedOldHash":m.items[0].expected,"action":action,"targetAssetId":"shield","targetVariantId":if action=="bind_variant"||action=="record_use" {json!("shield:drill")} else {Value::Null},"take":null,"replacementText":null,"reason":"本批明确钻头重盾连续使用；源定义与用途分开","evidenceRows":[f.candidate.assets[0].source_refs[0]]}]})
    }
    #[test]
    fn shield_existing_variant_binding_preserves_canonical_design_and_observed_name() {
        let f = fixture();
        let m = manifest(&f);
        let before = json!(f.current);
        let raw = f.raw.clone();
        let out = apply(
            &m,
            &decision(&m, &f, "bind_variant"),
            &f.source,
            &f.basis,
            &f.current,
            &f.candidate,
        )
        .unwrap();
        assert!(out.review_items.is_empty());
        assert_eq!(out.proposal.assets.len(), 1);
        let shield = &out.proposal.assets[0];
        assert_eq!(shield.name, "资格赛钛盾");
        assert_eq!(
            shield.variants[1].description,
            f.current.assets[0].variants[1].description
        );
        assert_eq!(
            shield.variants[1].source_variant_id,
            f.current.assets[0].variants[1].source_variant_id
        );
        assert_eq!(shield.variants[1].scope.scenes.len(), 2);
        assert_eq!(shield.variants[0].scope.scenes.len(), 1);
        assert_eq!(
            shield.metadata["foundationObservedUses"][0]["observedName"],
            "实体钻头重盾"
        );
        assert_eq!(json!(f.current), before);
        assert_eq!(f.raw, raw);
        validate_foundation(&f.source, &out.proposal).unwrap();
    }
    #[test]
    fn binding_remaps_incoming_parent_and_fact_references_without_dangling_ids() {
        let mut f = fixture();
        let mut child = asset(&f.source, "child", "后继盾附件", 1);
        child.aliases.clear();
        child.variants[0].source_variant_id = Some("observed:base".into());
        child.variants[0].metadata.insert(
            "preparation".into(),
            json!({"mode":"DERIVE","fromKey":"observed:base"}),
        );
        f.candidate.assets.push(child);
        f.candidate.facts.push(FoundationFact {
            id: "fact".into(),
            asset_id: "observed".into(),
            variant_id: Some("observed:base".into()),
            attribute: "状态".into(),
            value: "钻头犁地".into(),
            scope: "STATE".into(),
            claim: "SOURCE".into(),
            source_refs: f.candidate.assets[0].source_refs.clone(),
            quotes: vec![],
        });
        let m = manifest(&f);
        let out = apply(
            &m,
            &decision(&m, &f, "bind_variant"),
            &f.source,
            &f.basis,
            &f.current,
            &f.candidate,
        )
        .unwrap();
        assert!(out.review_items.is_empty());
        let child = out
            .proposal
            .assets
            .iter()
            .find(|a| a.id == "child")
            .unwrap();
        assert_eq!(
            child.variants[0].source_variant_id.as_deref(),
            Some("shield:drill")
        );
        assert_eq!(
            child.variants[0].metadata["preparation"]["fromKey"],
            "shield:drill"
        );
        assert_eq!(out.proposal.facts[0].asset_id, "shield");
        assert_eq!(
            out.proposal.facts[0].variant_id.as_deref(),
            Some("shield:drill")
        );
        validate_foundation(&f.source, &out.proposal).unwrap();
    }
    #[test]
    fn unsupported_dependency_is_preserved_as_visible_pending_not_silently_dropped() {
        let mut f = fixture();
        let mut child = asset(&f.source, "child", "附件", 1);
        child
            .metadata
            .insert("unknownFutureBinding".into(), json!("observed:base"));
        f.candidate.assets.push(child);
        let m = manifest(&f);
        let out = apply(
            &m,
            &decision(&m, &f, "bind_variant"),
            &f.source,
            &f.basis,
            &f.current,
            &f.candidate,
        )
        .unwrap();
        assert_eq!(out.review_items.len(), 1);
        assert!(!out.review_items[0].blocking);
        assert!(out.proposal.assets.iter().any(|a| a.id == "observed"));
        assert!(deferred_pair_recorded(&out.proposal, "observed", "shield"));
        assert!(!distinct_pair_recorded(&out.proposal, "observed", "shield"));
    }
    #[test]
    fn same_alias_does_not_bind_without_explicit_identity_decision_and_source_proof() {
        let f = fixture();
        let m = manifest(&f);
        let out = apply(
            &m,
            &decision(&m, &f, "keep_distinct"),
            &f.source,
            &f.basis,
            &f.current,
            &f.candidate,
        )
        .unwrap();
        assert_eq!(out.proposal.assets.len(), 2);
        assert!(distinct_pair_recorded(&out.proposal, "shield", "observed"));
        assert!(!distinct_pair_recorded(&out.proposal, "other", "observed"));
        let mut changed = out.proposal.clone();
        changed.source_sha256 = "different".into();
        assert!(!distinct_pair_recorded(&changed, "shield", "observed"));
        let mut response = decision(&m, &f, "bind_variant");
        response["decisions"][0]["evidenceRows"] = json!([]);
        let pending = apply(&m, &response, &f.source, &f.basis, &f.current, &f.candidate).unwrap();
        assert_eq!(pending.review_items.len(), 1);
        assert_eq!(pending.proposal.assets.len(), 2);
    }
    #[test]
    fn uncertain_identity_is_not_an_adjudicated_distinct_pair() {
        let f = fixture();
        let m = manifest(&f);
        for action in ["defer_identity", "keep_distinct"] {
            let mut raw = decision(&m, &f, action);
            raw["decisions"][0]["evidenceRows"] = json!([]);
            let out = apply(&m, &raw, &f.source, &f.basis, &f.current, &f.candidate).unwrap();
            assert_eq!(out.proposal.assets.len(), 2);
            assert_eq!(out.review_items.len(), 1);
            assert!(!distinct_pair_recorded(&out.proposal, "shield", "observed"));
            assert!(deferred_pair_recorded(&out.proposal, "shield", "observed"));
            assert!(out.distinct_pairs.is_empty());
        }
    }
    #[test]
    fn stale_manifest_wrong_old_hash_duplicate_and_out_of_scope_evidence_fail_closed() {
        let f = fixture();
        let m = manifest(&f);
        let good = decision(&m, &f, "keep_distinct");
        for mut bad in [good.clone(), good.clone(), good.clone()]
            .into_iter()
            .enumerate()
        {
            match bad.0 {
                0 => bad.1["decisions"][0]["expectedOldHash"] = json!("wrong"),
                1 => {
                    bad.1["decisions"][0]["evidenceRows"] =
                        json!([f.current.assets[0].source_refs[0]])
                }
                _ => bad.1["decisions"]
                    .as_array_mut()
                    .unwrap()
                    .push(good["decisions"][0].clone()),
            };
            assert!(apply(&m, &bad.1, &f.source, &f.basis, &f.current, &f.candidate).is_err());
        }
        let mut current = f.current.clone();
        current.assets[0].description.push('变');
        assert!(apply(&m, &good, &f.source, &f.basis, &current, &f.candidate).is_err());
    }
    #[test]
    fn bounded_fallback_retains_independent_candidates_and_persistent_review() {
        let f = fixture();
        let m = manifest(&f);
        let out = fallback(
            &m,
            &f.source,
            &f.basis,
            &f.current,
            &f.candidate,
            "两次局部修复未通过",
        )
        .unwrap();
        assert_eq!(out.proposal.assets.len(), 2);
        assert_eq!(out.review_items.len(), 1);
        assert!(out.distinct_pairs.is_empty());
        assert!(out.proposal.issues.iter().any(|i| i
            .details
            .as_ref()
            .is_some_and(|d| d["kind"] == "FOUNDATION_CONFLICT_REVIEW")));
        validate_foundation(&f.source, &out.proposal).unwrap();
    }
    #[test]
    fn precise_field_adoption_preserves_other_fields_and_allows_grounded_text_repair() {
        let mut f = fixture();
        f.basis = f.current.clone();
        f.candidate = f.basis.clone();
        f.current.assets[0].description = "当前说明".into();
        f.candidate.assets[0].description = "候选说明".into();
        let m = build_manifest(
            &f.source,
            &f.batch,
            &f.basis,
            &f.current,
            &f.candidate,
            &json!({}),
            &f.raw,
            &["/assets/shield/description".into()],
        )
        .unwrap();
        let response = json!({"version":VERSION,"manifestSha256":m.digest,"decisions":[{"conflictId":m.items[0].id,"expectedOldHash":m.items[0].expected,"action":"design_change","targetAssetId":null,"targetVariantId":null,"take":null,"replacementText":"两段原文说明合并，保留盾体及钻头状态","reason":"整备与竞技场源用途均保留","evidenceRows":[f.batch.pieces[0].row_id]}]});
        let out = apply(&m, &response, &f.source, &f.basis, &f.current, &f.candidate).unwrap();
        assert_eq!(
            out.proposal.assets[0].description,
            "两段原文说明合并，保留盾体及钻头状态"
        );
        assert_eq!(out.proposal.assets[0].name, f.current.assets[0].name);
        assert_eq!(
            json!(out.proposal.assets[0].variants),
            json!(f.current.assets[0].variants)
        );
    }
}
