//! Explicit three-lane trial. Old frozen inputs are replayed unchanged; only
//! new packets carry the trial identity. Candidates merge before publication.
use super::*;

pub(crate) const PROFILE: &str = "foundation-three-lane-trial/v1";
const REBASE: &str = "foundation-parallel-rebase/v1";

#[cfg(test)]
#[path = "foundation_saved_wave_tests.rs"]
mod saved_wave_tests;

fn value_sha<T: serde::Serialize>(value: &T) -> Result<String, String> {
    Ok(digest(
        &serde_json::to_string(value).map_err(|e| e.to_string())?,
    ))
}

/// A rebase edits a fixed saved result, not the whole catalogue. Select by exact
/// registered identities only, then close over every relationship of retained
/// assets (including their variants). Never infer identity from a name. Source,
/// evidence, original result and rules are deliberately outside this projection.
fn project_rebase_catalogues(input: &mut Value, assets: &[Value]) {
    fn strings(value: &Value, out: &mut HashSet<String>) {
        match value {
            Value::String(s) => {
                out.insert(s.clone());
            }
            Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
            Value::Object(object) => object.values().for_each(|v| strings(v, out)),
            _ => {}
        }
    }
    let mut owners = HashMap::new();
    for (index, asset) in assets.iter().enumerate() {
        if let Some(id) = asset["id"].as_str() {
            owners.insert(id.to_owned(), index);
        }
        for variant in asset["variants"].as_array().into_iter().flatten() {
            if let Some(id) = variant["id"].as_str() {
                owners.insert(id.to_owned(), index);
            }
        }
    }
    let mut roots = HashSet::new();
    for field in [
        "foundationParallelOriginalResult",
        "foundationParallelCurrentCatalogue",
        "automaticDecisions",
        "humanChoiceRegistry",
        "confirmedDecisions",
        "evidence",
    ] {
        strings(&input[field], &mut roots);
    }
    for path in input["foundationParallelRebase"]["conflicts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        roots.extend(path.split('/').map(str::to_owned));
    }
    let mut retained = HashSet::new();
    let mut pending: Vec<_> = roots
        .iter()
        .filter_map(|id| owners.get(id).copied())
        .collect();
    while let Some(index) = pending.pop() {
        if !retained.insert(index) {
            continue;
        }
        let mut references = HashSet::new();
        strings(&assets[index], &mut references);
        pending.extend(references.iter().filter_map(|id| owners.get(id).copied()));
    }
    for field in ["registry", "candidates", "materialCandidates"] {
        if let Some(entries) = input[field].as_array_mut() {
            entries.retain(|entry| {
                let key = entry
                    .get("baseKey")
                    .or_else(|| entry.get("key"))
                    .and_then(Value::as_str);
                // Unrecognised/legacy structures are kept, never guessed away.
                key.and_then(|key| owners.get(key))
                    .is_none_or(|index| retained.contains(index))
            });
        }
    }
}

fn rebase_capacity_input(
    stage: &str,
    input: Value,
    current: &FoundationProposal,
    schema: &Value,
) -> Result<Value, String> {
    let mut input = capacity_input(stage, input, schema)?;
    let before = request_chars(stage, &input, schema)?;
    if before <= MODEL_TRANSPORT_LIMIT {
        return Ok(input);
    }
    let assets = serde_json::to_value(&current.assets).map_err(|e| e.to_string())?;
    project_rebase_catalogues(&mut input, assets.as_array().ok_or("重基目录不是数组")?);
    let guide = input["transportInstructions"].as_str().unwrap_or("");
    input["transportInstructions"] = json!(format!("{guide} 本次为固定原稿受限重基，registry/candidates/materialCandidates仅提供原稿引用身份、明确冲突两端及其完整关系依赖闭包；未提供条目不表示不存在，不得重新发现、合并或改动批外身份。完整原稿、本批原文及历史证据均保留。"));
    let after = request_chars(stage, &input, schema)?;
    // Bounded numerical diagnostics: never log source, prompts or account data.
    eprintln!("FOUNDATION_PARALLEL_REBASE_CAPACITY stage={stage} beforeRequestChars={before} afterRequestChars={after} limit={MODEL_TRANSPORT_LIMIT}");
    Ok(input)
}

/// Called before capacity measurement/normal wire compilation. Never changes
/// source rows, schemas or rules; only historical retrieval is seed-restricted.
pub(super) fn attach_input(
    stage: &str,
    input: &mut Value,
    context: &Value,
    basis: &FoundationProposal,
) {
    let Some(config) = context
        .get("foundationParallelTrial")
        .filter(|c| c["version"] == PROFILE)
    else {
        return;
    };
    let history: HashSet<_> = config["historyEpisodeIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if let Some(rows) = input
        .pointer_mut("/evidence/historicalEvidenceCandidates")
        .and_then(Value::as_array_mut)
    {
        rows.retain(|row| row[1].as_str().is_some_and(|id| history.contains(id)));
    }
    let index = input["batchIndex"].as_u64().unwrap_or(0);
    let offset = if stage == "F2" {
        config["replayBatchCount"].as_u64().unwrap_or(0)
    } else {
        0
    };
    let mut marker = config.clone();
    marker["batchKey"] = input["batch"].clone();
    marker["batchIndex"] = json!(index);
    marker["laneId"] = json!(index.saturating_sub(offset) % 3);
    marker["waveIndex"] = json!(index.saturating_sub(offset) / 3);
    marker["packetId"] = json!(format!(
        "{}:{stage}:{index}",
        input["batch"].as_str().unwrap_or("")
    ));
    marker["basisSha256"] = json!(value_sha(basis).unwrap_or_default());
    input["foundationParallelTrial"] = marker;
    if stage == "Q1" {
        input["seriesDirectionPreparation"]["mode"] = json!("SIGNALS");
    }
    let guide = input["transportInstructions"].as_str().unwrap_or("");
    input["transportInstructions"] = json!(format!("{guide} 本批为三路并行候选，registry是共同波次基线。历史检索仅共同seed范围；不将未提供的兄弟批次视为已知事实。严格复用已登记身份；共享身份/外观/来源有冲突时交有据事项，不通过重命名或覆盖解决。Q1本批只交SIGNALS，禁止changes.overview（全剧概述），全剧概述与方向依据放入seriesDirection.signals，最终概述与方向另行统一收口。"));
}

fn validate_trial(context: &Value, selected: &[String]) -> Result<usize, String> {
    let c = &context["foundationParallelTrial"];
    if c["version"] != PROFILE
        || c["concurrency"] != 3
        || c["seedBatchCount"] != 1
        || c["authorizationSha256"]
            .as_str()
            .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("FOUNDATION_PARALLEL_TRIAL_INVALID: 缺少正式冻结的三路授权".into());
    }
    let count = c["replayBatchCount"]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|n| *n > 0)
        .ok_or("并行试验缺已接受前缀批数")?;
    let ids = c["historyEpisodeIds"]
        .as_array()
        .ok_or("并行试验缺共同历史范围")?;
    let unique: HashSet<_> = ids.iter().filter_map(Value::as_str).collect();
    if ids.is_empty()
        || unique.len() != ids.len()
        || unique.iter().any(|id| !selected.iter().any(|s| s == id))
    {
        return Err("并行试验共同历史范围无效".into());
    }
    Ok(count)
}

fn confirmed(context: &Value) -> Option<&str> {
    context
        .pointer("/settings/confirmedDirection")
        .and_then(|v| {
            v.as_str()
                .or_else(|| v.get("content").and_then(Value::as_str))
        })
        .filter(|s| !s.trim().is_empty())
}

fn apply(
    stage: &str,
    value: &Value,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    basis: &FoundationProposal,
    direction: Option<&str>,
    resolved: &HashSet<(String, String)>,
    receipt_time: &str,
) -> Result<(FoundationProposal, Vec<Value>), String> {
    if stage == "F2" {
        Ok((
            apply_catalogue(value, source, batch, input, basis, direction, resolved)?,
            vec![],
        ))
    } else {
        let times: HashMap<String, String> = input["automaticDecisions"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(value["appliedDecisions"].as_array().into_iter().flatten())
            .filter_map(|d| {
                d["id"]
                    .as_str()
                    .map(|id| (id.to_owned(), receipt_time.to_owned()))
            })
            .collect();
        apply_optimization_with_receipt_times(
            value,
            source,
            batch,
            input,
            basis,
            direction,
            Some(&times),
        )
    }
}

struct Executed {
    value: Value,
    receipt_time: String,
}

async fn execute(
    stage: &str,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    basis: &FoundationProposal,
    context: &Value,
    resolved: &HashSet<(String, String)>,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Executed, String> {
    let schema = delivery_schema(stage, input);
    if !fits(stage, input, &schema)? {
        return Err("FOUNDATION_PARALLEL_CAPACITY: 完整请求超预算，原候选保留且未提交".into());
    }
    let value=call_validated(executor,&format!("foundation:{stage}"),if stage=="F2"{F2_RULE}else{Q1_RULE},model,reasoning,input.clone(),schema,&[],|value| {
        restore_delivery_refs(value,input)?;
        if let Some(original)=input.get("foundationParallelOriginalResult") {
            validate_rebase_result(stage,original,value)?;
            validate_rebase_targets(stage,original,value,&input["foundationParallelRebase"]["allowedAssetOrdinals"])?;
            validate_rebase_fields(stage,original,value,&input["foundationParallelRebase"]["allowedAssetFields"])?;
        }
        if input.get("foundationParallelFinal").is_some() {
            if value["changes"].as_object().is_none_or(|m|m.keys().any(|k|k!="overview"))
                || ["issues","resolvedIssues","humanChoices","unresolved","notes","appliedDecisions"].iter().any(|key| value.get(*key).is_some_and(|v|!v.is_null() && v.as_array().is_none_or(|a|!a.is_empty())))
                || value.pointer("/seriesDirection/content").and_then(Value::as_str).is_none_or(|s|s.trim().is_empty()) {
                return Err("统一方向收口只能交overview、方向与已验证依据，changes不得含其他字段，不得修改目录/场次/原文".into());
            }
        }
        if stage=="Q1" && input.get("foundationParallelFinal").is_none() && value.pointer("/changes/overview").is_some_and(|v|!v.is_null()) {
            return Err("并行Q1不能改写全剧overview，请把整剧依据提交seriesDirection.signals，最终统一收口".into());
        }
        let (next,_)=apply(stage,value,source,batch,input,basis,confirmed(context),resolved,"VALIDATION_ONLY_NOT_PUBLISHED")?;
        if input.get("foundationParallelRebase").is_some() {
            let collisions=name_conflicts(basis,basis,&next);
            if !collisions.is_empty(){return Err(format!("FOUNDATION_PARALLEL_REBASE_UNRESOLVED: {}",collisions.join(";")));}
        }
        Ok(())
    }).await?;
    let receipt_time = if stage == "Q1" {
        executor.foundation_parallel_receipt_time(input).await?
    } else {
        String::new()
    };
    Ok(Executed {
        value,
        receipt_time,
    })
}

/// Structural three-way merge: no array-position identity or last-writer wins.
/// Known evidence/use arrays are sets. Unknown arrays and conflicting scalars
/// require one explicit model rebase against the current accepted basis.
fn array_identity(field: &str, value: &Value) -> Option<String> {
    if field == "issues" {
        if let Some(id) = value
            .pointer("/details/authorDecision/id")
            .and_then(Value::as_str)
        {
            return Some(format!("decision:{id}"));
        }
        let coordinate = value["coordinate"].as_str()?;
        return Some(format!(
            "{}:{}:{}:{}",
            coordinate,
            value
                .pointer("/details/kind")
                .and_then(Value::as_str)
                .unwrap_or(""),
            value["assetId"].as_str().unwrap_or(""),
            value["variantId"].as_str().unwrap_or("")
        ));
    }
    value[if field == "episodes" {
        "episodeId"
    } else {
        "id"
    }]
    .as_str()
    .map(str::to_owned)
}

fn merge_value(
    base: Option<&Value>,
    current: Option<&Value>,
    candidate: Option<&Value>,
    path: &str,
    conflicts: &mut Vec<String>,
) -> Option<Value> {
    merge_resolved_value(base, current, candidate, path, conflicts, &HashMap::new())
}

fn merge_resolved_value(
    base: Option<&Value>,
    current: Option<&Value>,
    candidate: Option<&Value>,
    path: &str,
    conflicts: &mut Vec<String>,
    resolutions: &HashMap<String, Option<Value>>,
) -> Option<Value> {
    if let Some(value) = resolutions.get(path) {
        return value.clone();
    }
    if let (Some(c), Some(n)) = (current, candidate) {
        if c["decisionPolicy"] == AUTO_INTERPRET_POLICY
            && n["decisionPolicy"] == AUTO_INTERPRET_POLICY
            && c["id"].is_string()
            && c["id"] == n["id"]
        {
            let mut cs = c.clone();
            let mut ns = n.clone();
            for field in ["appliedAt", "appliedNote", "receipts", "status"] {
                cs.as_object_mut().unwrap().remove(field);
                ns.as_object_mut().unwrap().remove(field);
            }
            if cs != ns {
                conflicts.push(format!("{path}/decision-semantics"));
                return current.cloned();
            }
            let mut merged = c.clone();
            let mut receipts = c["receipts"].as_array().cloned().unwrap_or_default();
            for receipt in n["receipts"].as_array().into_iter().flatten() {
                if !receipts.contains(receipt) {
                    receipts.push(receipt.clone());
                }
            }
            let coverage: HashSet<_> = receipts
                .iter()
                .flat_map(|r| r["coverage"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .collect();
            let complete = c["episodes"].as_array().is_some_and(|episodes| {
                episodes
                    .iter()
                    .all(|e| e.as_str().is_some_and(|e| coverage.contains(e)))
            });
            merged["status"] = json!(if complete {
                "APPLIED"
            } else {
                "PARTIALLY_APPLIED"
            });
            merged["receipts"] = json!(receipts);
            return Some(merged);
        }
    }
    let has_resolution_below = resolutions
        .keys()
        .any(|key| key.starts_with(&format!("{path}/")));
    if !has_resolution_below && candidate == base {
        return current.cloned();
    }
    if !has_resolution_below && (current == base || current == candidate) {
        return candidate.cloned();
    }
    if let (Some(Value::Object(c)), Some(Value::Object(n))) = (current, candidate) {
        let b = base.and_then(Value::as_object);
        if base.is_none() || b.is_some() {
            let keys: BTreeSet<_> = c
                .keys()
                .chain(n.keys())
                .chain(b.into_iter().flat_map(|b| b.keys()))
                .collect();
            let mut out = Map::new();
            for key in keys {
                if let Some(v) = merge_resolved_value(
                    b.and_then(|b| b.get(key)),
                    c.get(key),
                    n.get(key),
                    &format!("{path}/{key}"),
                    conflicts,
                    resolutions,
                ) {
                    out.insert(key.clone(), v);
                }
            }
            return Some(Value::Object(out));
        }
    }
    if let (Some(Value::Array(c)), Some(Value::Array(n))) = (current, candidate) {
        let b = base.and_then(Value::as_array);
        let field = path.rsplit('/').next().unwrap_or("");
        if base.is_none() || b.is_some() {
            if [
                "assets",
                "variants",
                "facts",
                "episodes",
                "issues",
                "authorDecisions",
            ]
            .contains(&field)
            {
                let valid = |a: &[Value]| {
                    let ids = a
                        .iter()
                        .filter_map(|v| array_identity(field, v))
                        .collect::<Vec<_>>();
                    ids.len() == a.len() && ids.iter().collect::<HashSet<_>>().len() == a.len()
                };
                if valid(c) && valid(n) && b.is_none_or(|a| valid(a)) {
                    let mut ids = vec![];
                    for v in c.iter().chain(n) {
                        let id = array_identity(field, v).unwrap();
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                    let mut out = vec![];
                    for id in ids {
                        let find = |a: &[Value]| {
                            a.iter().position(|v| {
                                array_identity(field, v).as_deref() == Some(id.as_str())
                            })
                        };
                        let bv = b.and_then(|a| find(a).map(|i| &a[i]));
                        let cv = find(c).map(|i| &c[i]);
                        let nv = find(n).map(|i| &n[i]);
                        if let Some(v) = merge_resolved_value(
                            bv,
                            cv,
                            nv,
                            &format!("{path}/{id}"),
                            conflicts,
                            resolutions,
                        ) {
                            out.push(v);
                        }
                    }
                    return Some(json!(out));
                }
            }
            if ["sourceRefs", "applicability", "scenes", "usageEpisodes"].contains(&field) {
                let empty = vec![];
                let b = b.unwrap_or(&empty);
                let mut out = vec![];
                for item in c.iter().chain(n) {
                    if b.contains(item) && (!c.contains(item) || !n.contains(item)) {
                        continue;
                    }
                    if !out.contains(item) {
                        out.push(item.clone());
                    }
                }
                return Some(json!(out));
            }
        }
    }
    conflicts.push(path.to_owned());
    current.cloned()
}

fn name_conflicts(
    base: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &FoundationProposal,
) -> Vec<String> {
    let known: HashSet<_> = base.assets.iter().map(|a| a.id.as_str()).collect();
    let mut conflicts = vec![];
    for asset in candidate
        .assets
        .iter()
        .filter(|a| !known.contains(a.id.as_str()))
    {
        for other in current
            .assets
            .iter()
            .filter(|a| a.id != asset.id && a.kind == asset.kind)
        {
            if asset.name == other.name
                || asset.aliases.contains(&other.name)
                || other.aliases.contains(&asset.name)
                || asset.aliases.iter().any(|a| other.aliases.contains(a))
            {
                conflicts.push(format!(
                    "/assets/{}/identity-collision-with/{}",
                    asset.id, other.id
                ));
            }
        }
    }
    conflicts
}
fn merge_candidate(
    base: &FoundationProposal,
    current: &FoundationProposal,
    candidate: &FoundationProposal,
    source: &LockedSource,
) -> Result<FoundationProposal, Vec<String>> {
    let (b, c, n) = (json!(base), json!(current), json!(candidate));
    let mut conflicts = name_conflicts(base, current, candidate);
    let merged =
        merge_value(Some(&b), Some(&c), Some(&n), "", &mut conflicts).unwrap_or(Value::Null);
    if !conflicts.is_empty() {
        return Err(conflicts);
    }
    finalize_merged(merged, source)
}

fn finalize_merged(
    merged: Value,
    source: &LockedSource,
) -> Result<FoundationProposal, Vec<String>> {
    let mut next: FoundationProposal =
        serde_json::from_value(merged).map_err(|e| vec![e.to_string()])?;
    let decisions = prior_auto_records(&next);
    retain_auto_interpretations(&mut next, &decisions);
    let selected = source
        .episodes
        .iter()
        .map(|e| e.id.clone())
        .collect::<Vec<_>>();
    let projected = source_projection(source, &next.issues, &selected).map_err(|e| vec![e])?;
    materialize_foundation(&projected, &mut next).map_err(|e| vec![e])?;
    validate_foundation(&projected, &next).map_err(|e| vec![e])?;
    Ok(next)
}

fn identity_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut value = value;
    let mut field = "";
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if let Some(items) = value.as_array() {
            value = items
                .iter()
                .find(|v| array_identity(field, v).as_deref() == Some(part))?;
        } else {
            value = value.get(part)?;
            field = part;
        }
    }
    Some(value)
}

fn merge_rebased(
    base: &FoundationProposal,
    current: &FoundationProposal,
    original_candidate: &FoundationProposal,
    applied: &FoundationProposal,
    paths: &[String],
    rebindings: &HashMap<String, String>,
    source: &LockedSource,
) -> Result<FoundationProposal, Vec<String>> {
    let (b, c, mut n, applied) = (
        json!(base),
        json!(current),
        json!(original_candidate),
        json!(applied),
    );
    let mut resolutions = HashMap::new();
    for path in paths {
        if let Some((prefix, _)) = path.split_once("/identity-collision-with/") {
            let old_id = prefix
                .strip_prefix("/assets/")
                .ok_or_else(|| vec![path.clone()])?;
            let target = rebindings
                .get(old_id)
                .ok_or_else(|| vec![format!("身份冲突没有明确复用当前key：{old_id}")])?;
            let target_path = format!("/assets/{target}");
            let target_asset = identity_path(&applied, &target_path)
                .cloned()
                .ok_or_else(|| vec![target_path.clone()])?;
            let entries = n["assets"].as_array_mut().unwrap();
            entries.retain(|a| a["id"] != old_id && a["id"] != *target);
            entries.push(target_asset.clone());
            resolutions.insert(target_path, Some(target_asset));
        } else {
            resolutions.insert(path.clone(), identity_path(&applied, path).cloned());
        }
    }
    let mut conflicts = vec![];
    let merged = merge_resolved_value(
        Some(&b),
        Some(&c),
        Some(&n),
        "",
        &mut conflicts,
        &resolutions,
    )
    .unwrap_or(Value::Null);
    if !conflicts.is_empty() {
        return Err(conflicts);
    }
    finalize_merged(merged, source)
}

fn rebase_identity_map(
    stage: &str,
    original: &Value,
    fixed: &Value,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    basis: &FoundationProposal,
) -> Result<HashMap<String, String>, String> {
    let pointer = if stage == "F2" {
        "/assets"
    } else {
        "/changes/assets"
    };
    let a = original
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let b = fixed
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut map = HashMap::new();
    for (index, (old, new)) in a.into_iter().zip(b).enumerate() {
        if old["key"] != new["key"] {
            let wire: AssetWire = serde_json::from_value(old).map_err(|e| e.to_string())?;
            let asset = build_asset(
                source,
                batch,
                &wire,
                basis,
                &allowed_refs(input),
                stage == "Q1",
                index,
            )?;
            map.insert(
                asset.id,
                new["key"].as_str().ok_or("身份重基缺正式key")?.to_owned(),
            );
        }
    }
    Ok(map)
}

/// Rebase cannot quietly change its episode coverage, introduce a different
/// scene edit, remove another asset, or rewrite non-conflicting batch fields.
fn validate_rebase_result(stage: &str, original: &Value, next: &Value) -> Result<(), String> {
    let fail = || "FOUNDATION_PARALLEL_REBASE_SCOPE: 重基改变了原批范围或非资产修订内容".to_owned();
    if original["format"] != next["format"]
        || original["sourceSha256"] != next["sourceSha256"]
        || original["coverage"] != next["coverage"]
    {
        return Err(fail());
    }
    let pointer = if stage == "F2" {
        "/assets"
    } else {
        "/changes/assets"
    };
    let old = original
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let new = next
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if old.len() != new.len() {
        return Err(fail());
    }
    for (a, b) in old.iter().zip(&new) {
        for key in ["name", "type", "episodes", "sceneRefs", "sourceRefs"] {
            if a[key] != b[key] {
                return Err(fail());
            }
        }
        let av = a["variants"].as_array().cloned().unwrap_or_default();
        let bv = b["variants"].as_array().cloned().unwrap_or_default();
        if av.len() != bv.len() {
            return Err(fail());
        }
        for (a, b) in av.iter().zip(&bv) {
            for key in ["name", "episodes", "sceneRefs", "sourceRefs"] {
                if a[key] != b[key] {
                    return Err(fail());
                }
            }
        }
    }
    let mut a = original.clone();
    let mut b = next.clone();
    if stage == "F2" {
        a.as_object_mut().ok_or_else(fail)?.remove("assets");
        b.as_object_mut().ok_or_else(fail)?.remove("assets");
    } else {
        if let Some(c) = a["changes"].as_object_mut() {
            c.remove("assets");
        }
        if let Some(c) = b["changes"].as_object_mut() {
            c.remove("assets");
        }
    }
    if a != b {
        return Err(fail());
    }
    Ok(())
}

fn validate_rebase_targets(
    stage: &str,
    original: &Value,
    next: &Value,
    allowed: &Value,
) -> Result<(), String> {
    let pointer = if stage == "F2" {
        "/assets"
    } else {
        "/changes/assets"
    };
    let a = original
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let b = next
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let allowed = allowed.as_array().ok_or("重基缺冲突资产边界")?;
    for (index, (a, b)) in a.iter().zip(&b).enumerate() {
        if a != b && !allowed.contains(&json!(index)) {
            return Err("重基修改了没有冲突的资产条目".into());
        }
    }
    Ok(())
}

fn validate_rebase_fields(
    stage: &str,
    original: &Value,
    next: &Value,
    policy: &Value,
) -> Result<(), String> {
    let pointer = if stage == "F2" {
        "/assets"
    } else {
        "/changes/assets"
    };
    let old = original
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let new = next
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, (a, b)) in old.iter().zip(&new).enumerate() {
        let allowed = policy[index.to_string()]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let keys: BTreeSet<_> = a
            .as_object()
            .into_iter()
            .flat_map(|m| m.keys())
            .chain(b.as_object().into_iter().flat_map(|m| m.keys()))
            .collect();
        for key in keys {
            if a.get(key) != b.get(key)
                && !allowed.contains(&json!("*"))
                && !allowed.contains(&json!(key))
            {
                return Err(format!("重基越过冲突字段：assets[{index}].{key}"));
            }
        }
    }
    Ok(())
}

fn rebase_field_policy(
    stage: &str,
    result: &Value,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    basis: &FoundationProposal,
    conflicts: &[String],
) -> Result<Value, String> {
    let pointer = if stage == "F2" {
        "/assets"
    } else {
        "/changes/assets"
    };
    let entries = result
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut policy = Map::new();
    for (index, value) in entries.into_iter().enumerate() {
        let wire: AssetWire = serde_json::from_value(value).map_err(|e| e.to_string())?;
        let asset = build_asset(
            source,
            batch,
            &wire,
            basis,
            &allowed_refs(input),
            stage == "Q1",
            index,
        )?;
        let prefix = format!("/assets/{}/", asset.id);
        let mut fields = BTreeSet::new();
        for path in conflicts.iter().filter_map(|p| p.strip_prefix(&prefix)) {
            let field = path.split('/').next().unwrap_or("");
            match field {
                "identity-collision-with" => {
                    fields.insert("*");
                }
                "variants" => {
                    fields.extend(["variants", "preparation", "scope"]);
                }
                "catalogName" => {
                    fields.insert("name");
                }
                _ => {
                    fields.insert(field);
                }
            }
        }
        policy.insert(index.to_string(), json!(fields));
    }
    Ok(Value::Object(policy))
}

fn rebase_targets(
    stage: &str,
    result: &Value,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    basis: &FoundationProposal,
    conflicts: &[String],
) -> Result<Vec<usize>, String> {
    if conflicts.iter().any(|c| !c.starts_with("/assets/")) {
        return Err(format!(
            "FOUNDATION_PARALLEL_REBASE_UNSUPPORTED: 非资产冲突不能冒充已解决，原候选保留：{}",
            conflicts.join(";")
        ));
    }
    let pointer = if stage == "F2" {
        "/assets"
    } else {
        "/changes/assets"
    };
    let entries = result
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut allowed = vec![];
    let mut covered = HashSet::new();
    for (index, value) in entries.into_iter().enumerate() {
        let wire: AssetWire = serde_json::from_value(value).map_err(|e| e.to_string())?;
        let asset = build_asset(
            source,
            batch,
            &wire,
            basis,
            &allowed_refs(input),
            stage == "Q1",
            index,
        )?;
        let prefix = format!("/assets/{}", asset.id);
        for conflict in conflicts {
            if conflict == &prefix || conflict.starts_with(&format!("{prefix}/")) {
                covered.insert(conflict);
                if !allowed.contains(&index) {
                    allowed.push(index);
                }
            }
        }
    }
    if covered.len() != conflicts.iter().collect::<HashSet<_>>().len() {
        return Err("FOUNDATION_PARALLEL_REBASE_UNSUPPORTED: 删除或归并冲突不属于原资产编辑条目，保留候选待专门收口".into());
    }
    Ok(allowed)
}

async fn optional_execute<'a>(
    slot: Option<&'a (Batch, Value)>,
    stage: &str,
    source: &LockedSource,
    basis: &FoundationProposal,
    context: &Value,
    resolved: &HashSet<(String, String)>,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Option<Result<Executed, String>> {
    match slot {
        Some((batch, input)) => Some(
            execute(
                stage, source, batch, input, basis, context, resolved, executor, model, reasoning,
            )
            .await,
        ),
        None => None,
    }
}

async fn wave(
    stage: &str,
    slots: &[(Batch, Value)],
    source: &LockedSource,
    root_source: &LockedSource,
    basis: &FoundationProposal,
    context: &Value,
    resolved: &HashSet<(String, String)>,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<(FoundationProposal, Vec<Value>), String> {
    // join! drains all allocated calls even if a sibling fails/turns UNKNOWN.
    // Results remain in index order regardless of network completion order.
    let (a, b, c) = tokio::join!(
        optional_execute(
            slots.first(),
            stage,
            source,
            basis,
            context,
            resolved,
            executor,
            model,
            reasoning
        ),
        optional_execute(
            slots.get(1),
            stage,
            source,
            basis,
            context,
            resolved,
            executor,
            model,
            reasoning
        ),
        optional_execute(
            slots.get(2),
            stage,
            source,
            basis,
            context,
            resolved,
            executor,
            model,
            reasoning
        )
    );
    let results = vec![a, b, c].into_iter().flatten().collect::<Vec<_>>();
    if let Some(error) = results.iter().find_map(|r| r.as_ref().err()) {
        return Err(error.clone());
    }
    let mut current = basis.clone();
    let mut signals = vec![];
    for ((batch, input), result) in slots.iter().zip(results) {
        let executed = result?;
        let result = executed.value;
        let (candidate, own_signals) = apply(
            stage,
            &result,
            source,
            batch,
            input,
            basis,
            confirmed(context),
            resolved,
            &executed.receipt_time,
        )?;
        match merge_candidate(basis, &current, &candidate, root_source) {
            Ok(next) => {
                current = next;
                signals.extend(own_signals);
            }
            Err(conflicts) => {
                let allowed =
                    rebase_targets(stage, &result, source, batch, input, basis, &conflicts)?;
                let mut revised = input_for(
                    stage,
                    source,
                    batch,
                    &current,
                    context,
                    input["batchIndex"].as_u64().unwrap_or(0) as usize,
                    input["delivery"]["batchCount"].as_u64().unwrap_or(1) as usize,
                    false,
                    &[],
                );
                revised["foundationParallelRebase"] = json!({"version":REBASE,"originalInputSha256":value_sha(input)?,"originalResultSha256":value_sha(&result)?,"currentBasisSha256":value_sha(&current)?,"attempt":1,"conflicts":conflicts});
                revised["foundationParallelRebase"]["allowedAssetOrdinals"] = json!(allowed);
                revised["foundationParallelRebase"]["allowedAssetFields"] =
                    rebase_field_policy(stage, &result, source, batch, input, basis, &conflicts)?;
                revised["foundationParallelOriginalResult"] = result.clone();
                revised["foundationParallelCurrentCatalogue"]=json!(current.assets.iter().filter(|a|conflicts.iter().any(|c|c.starts_with(&format!("/assets/{}/",a.id))||c==&format!("/assets/{}",a.id)||c.ends_with(&format!("/identity-collision-with/{}",a.id)))).map(|a|json!({"key":a.id,"name":a.name,"type":a.kind,"description":a.description,"aliases":a.aliases,"role":a.metadata.get("role"),"relationNote":a.metadata.get("relationNote"),"preparation":a.metadata.get("preparation"),"variants":a.variants.iter().map(|v|json!({"key":v.id,"name":v.name,"description":v.description,"fromKey":v.source_variant_id,"preparation":v.metadata.get("preparation")})).collect::<Vec<_>>()})).collect::<Vec<_>>());
                let guide = revised["transportInstructions"].as_str().unwrap_or("");
                revised["transportInstructions"]=json!(format!("{guide} 这是原批唯一一次受限重基。foundationParallelOriginalResult是已保存原稿；根据当前registry/candidates修复列出的共享身份、元数据及母图关系冲突，优先复用准确key。仅允许修改原稿assets条目中冲突字段及其身份key/父key，条目顺序与数量、名称、分类、集场、sourceRefs、形象名称及集场全部保留；其它字段逐值保持。不得增加/删除/归并批外身份、修改原文、扩大范围或推断已成功采用。"));
                revised = rebase_capacity_input(
                    stage,
                    revised,
                    &current,
                    &if stage == "F2" {
                        f2_schema()
                    } else {
                        q1_schema()
                    },
                )?;
                let fixed = execute(
                    stage, source, batch, &revised, &current, context, resolved, executor, model,
                    reasoning,
                )
                .await
                .map_err(|e| {
                    format!(
                        "FOUNDATION_PARALLEL_REBASE_FAILED: 原候选保留；本批一次修订未通过：{e}"
                    )
                })?;
                let (next, new_signals) = apply(
                    stage,
                    &fixed.value,
                    source,
                    batch,
                    &revised,
                    &current,
                    confirmed(context),
                    resolved,
                    &fixed.receipt_time,
                )?;
                let collisions = name_conflicts(&current, &current, &next);
                if !collisions.is_empty() {
                    return Err(format!(
                        "FOUNDATION_PARALLEL_REBASE_UNRESOLVED: {}",
                        collisions.join(";")
                    ));
                }
                let bindings =
                    rebase_identity_map(stage, &result, &fixed.value, source, batch, input, basis)?;
                current = merge_rebased(
                    basis,
                    &current,
                    &candidate,
                    &next,
                    &conflicts,
                    &bindings,
                    root_source,
                )
                .map_err(|remaining| {
                    format!(
                        "FOUNDATION_PARALLEL_REBASE_UNRESOLVED: {}",
                        remaining.join(";")
                    )
                })?;
                signals.extend(new_signals);
            }
        }
    }
    Ok((current, signals))
}

pub(super) async fn run(
    source: &LockedSource,
    context: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<FoundationProposal, String> {
    let source = upgrade_source_dialogues(source)?;
    validate_source(&source)?;
    let (selected, mut foundation, resolved) = delivery_basis(&source, context)?;
    let replay = validate_trial(context, &selected)?;
    let mut legacy = context.clone();
    legacy
        .as_object_mut()
        .ok_or("并行上下文必须为对象")?
        .remove("foundationParallelTrial");
    let mut pending = plan_batches(&source, &selected)?;
    let mut accepted = vec![];
    for index in 0..replay {
        if pending.is_empty() {
            return Err("并行授权前缀超出原批次".into());
        }
        let (batch, input) = prepare_capacity_batch(
            "F2",
            &source,
            &mut pending,
            &foundation,
            &legacy,
            index,
            &[],
        )?;
        if index == 0
            && json!(batch.scope) != context["foundationParallelTrial"]["historyEpisodeIds"]
        {
            return Err("共同seed必须精确等于原第一批范围".into());
        }
        let result = execute(
            "F2",
            &source,
            &batch,
            &input,
            &foundation,
            &legacy,
            &resolved,
            executor,
            model,
            reasoning,
        )
        .await?;
        foundation = apply_catalogue(
            &result.value,
            &source,
            &batch,
            &input,
            &foundation,
            confirmed(context),
            &resolved,
        )?;
        accepted.push(batch);
    }
    let mut index = replay;
    while !pending.is_empty() {
        let baseline = foundation.clone();
        let mut slots = vec![];
        for _ in 0..3 {
            if pending.is_empty() {
                break;
            }
            slots.push(prepare_capacity_batch(
                "F2",
                &source,
                &mut pending,
                &baseline,
                context,
                index,
                &[],
            )?);
            index += 1;
        }
        let (next, _) = wave(
            "F2", &slots, &source, &source, &baseline, context, &resolved, executor, model,
            reasoning,
        )
        .await?;
        foundation = next;
        accepted.extend(slots.into_iter().map(|(batch, _)| batch));
    }
    verify_coverage(&source, &selected, &accepted)?;
    let mut pending = editor_batches(accepted);
    let mut q_index = 0;
    let mut signals = vec![];
    let mut final_batch = None;
    while !pending.is_empty() {
        let projected = source_projection(&source, &foundation.issues, &selected)?;
        let baseline = foundation.clone();
        let mut slots = vec![];
        for _ in 0..3 {
            if pending.is_empty() {
                break;
            }
            slots.push(prepare_capacity_batch(
                "Q1",
                &projected,
                &mut pending,
                &baseline,
                context,
                q_index,
                &[],
            )?);
            q_index += 1;
        }
        final_batch = slots.last().map(|(batch, _)| batch.clone());
        let (next, new_signals) = wave(
            "Q1", &slots, &projected, &source, &baseline, context, &resolved, executor, model,
            reasoning,
        )
        .await?;
        foundation = next;
        signals.extend(new_signals);
    }
    let batch = final_batch.ok_or("并行核对没有最终范围")?;
    let projected = source_projection(&source, &foundation.issues, &selected)?;
    let mut input = input_for(
        "Q1",
        &projected,
        &batch,
        &foundation,
        context,
        q_index,
        q_index + 1,
        true,
        &signals,
    );
    input["seriesDirectionPreparation"]["mode"] = json!("FINAL");
    input["foundationParallelFinal"] = json!({"version":"foundation-parallel-final/v1","basisSha256":value_sha(&foundation)?,"signalsSha256":value_sha(&signals)?,"scope":selected});
    // Automatic interpretation receipts were already handled in the reviewed
    // packets; the direction-only final is not another decision application.
    for key in [
        "automaticDecisions",
        "automaticDecisionError",
        "foundationDecisionPolicy",
        "userReviewRequirements",
        "confirmedDecisions",
    ] {
        input.as_object_mut().unwrap().remove(key);
    }
    let guide = input["transportInstructions"].as_str().unwrap_or("");
    input["transportInstructions"]=json!(format!("{guide} 此次foundationParallelFinal为全剧方向统一收口，全部局部核对已保存。仅根据已验证priorSignals与priorDirection交seriesDirection.content；changes仅可包含overview（全剧概述，依据全部已核验signals），不得含其它字段；issues/resolvedIssues/humanChoices/unresolved/notes/appliedDecisions省略或空数组，不再编辑资产、集场、原文或采用决定。已确认方向原样保留。"));
    input = capacity_input("Q1", input, &q1_schema())?;
    let value = execute(
        "Q1",
        &projected,
        &batch,
        &input,
        &foundation,
        context,
        &resolved,
        executor,
        model,
        reasoning,
    )
    .await?;
    foundation = apply(
        "Q1",
        &value.value,
        &projected,
        &batch,
        &input,
        &foundation,
        confirmed(context),
        &resolved,
        &value.receipt_time,
    )?
    .0;
    foundation.updated_episode_ids = selected.clone();
    foundation.rule_snapshots = rule_snapshots()
        .into_iter()
        .filter(|r| ["screenplay-fidelity", F2_RULE, Q1_RULE].contains(&r.id.as_str()))
        .collect();
    let projected = source_projection(&source, &foundation.issues, &selected)?;
    materialize_foundation(&projected, &mut foundation)?;
    validate_foundation(&projected, &foundation)?;
    Ok(foundation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    };

    #[test]
    fn rebase_projection_keeps_exact_identity_closure_and_all_non_catalogue_bytes() {
        let mut assets: Vec<_> = (0..11).map(|i| json!({"id":format!("A{i}"),"variants":[{"id":format!("V{i}"),"preparation":{"fromKey":if i==0 {Value::Null} else {json!(format!("V{}",i-1))}}}]})).collect();
        assets.push(json!({"id":"unrelated","name":"same name","variants":[]}));
        assets.push(json!({"id":"collision","variants":[]}));
        let mut input = json!({
            "registry":assets.iter().map(|a|json!({"key":a["id"],"variants":a["variants"]})).collect::<Vec<_>>(),
            "candidates":[{"key":"A10","name":"same name","crossBatchVariants":[{"key":"V10","fromKey":"V9"}]}],
            "materialCandidates":[{"key":"V8","baseKey":"A8","kind":"VARIANT"},{"key":"unrelated"}],
            "source":{"rows":[["row","完整本批原文"]]},
            "evidence":{"historicalEvidenceCandidates":[["row0","完整历史证据"]]},
            "foundationParallelOriginalResult":{"assets":[{"key":"V10","name":"same name"}]},
            "foundationParallelRebase":{"attempt":1,"allowedAssetOrdinals":[0],"conflicts":["/assets/new/identity-collision-with/collision"]},
            "foundationParallelTrial":{"laneId":1,"basisSha256":"frozen"}
        });
        let before = input.clone();
        project_rebase_catalogues(&mut input, &assets);
        assert_eq!(input["registry"].as_array().unwrap().len(), 12);
        assert!(input["registry"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["key"] != "unrelated"));
        assert_eq!(input["candidates"], before["candidates"]);
        assert_eq!(
            input["materialCandidates"],
            json!([before["materialCandidates"][0]])
        );
        for (key, value) in before.as_object().unwrap() {
            if !["registry", "candidates", "materialCandidates"].contains(&key.as_str()) {
                assert_eq!(&input[key], value, "{key}");
            }
        }
        // A backwards relationship/cycle must terminate and keep all members.
        assets[0]["variants"][0]["preparation"]["fromKey"] = json!("V10");
        let once = input.clone();
        project_rebase_catalogues(&mut input, &assets);
        assert_eq!(input, once);
    }

    #[test]
    fn rebase_projection_measures_real_compiled_transport_for_f2_and_q1() {
        let source = prepare_source("第1集\n场景：大厅\n△大厅保持安静。\n").unwrap();
        let context = json!({"selection":{"episodeIds":[source.episodes[0].id]},"foundationModelInputProfile":super::super::super::foundation_wire::PROFILE});
        let (selected, foundation, _) = delivery_basis(&source, &context).unwrap();
        let batch = plan_batches(&source, &selected).unwrap().remove(0);
        let assets: Vec<_> = (0..100)
            .map(|i| json!({"id":format!("CH_{i:016x}"),"variants":[]}))
            .collect();
        for stage in ["F2", "Q1"] {
            let schema = if stage == "F2" {
                f2_schema()
            } else {
                q1_schema()
            };
            let mut input = input_for(
                stage,
                &source,
                &batch,
                &foundation,
                &context,
                0,
                1,
                false,
                &[],
            );
            input["registry"] = json!(assets.iter().map(|a|json!({"key":a["id"],"name":"registered asset","aliases":["完整但与固定重基无关的目录说明。".repeat(200)],"variants":[]})).collect::<Vec<_>>());
            input["foundationParallelOriginalResult"] =
                json!({"assets":[{"key":assets[0]["id"],"description":"完整原稿"}]});
            input["foundationParallelRebase"] = json!({"attempt":1,"conflicts":[format!("/assets/{}/description",assets[0]["id"].as_str().unwrap())]});
            let original = input.clone();
            let before = request_chars(stage, &input, &schema).unwrap();
            project_rebase_catalogues(&mut input, &assets);
            let after = request_chars(stage, &input, &schema).unwrap();
            assert!(before > MODEL_TRANSPORT_LIMIT, "{stage}: {before}");
            assert!(after < MODEL_TRANSPORT_LIMIT, "{stage}: {after}");
            assert_eq!(input["source"], original["source"]);
            assert_eq!(input["evidence"], original["evidence"]);
            assert_eq!(
                input["foundationParallelOriginalResult"],
                original["foundationParallelOriginalResult"]
            );
            assert_eq!(allowed_refs(&input), allowed_refs(&original));
            eprintln!("synthetic rebase {stage}: compiled request {before} -> {after}");
        }
    }
    struct Fake {
        cache: Mutex<HashMap<String, Value>>,
        requests: Mutex<Vec<TextRequest>>,
        active: AtomicUsize,
        peak: AtomicUsize,
        fail_once: AtomicBool,
    }
    impl Fake {
        fn new(fail: bool) -> Self {
            Self {
                cache: Mutex::new(HashMap::new()),
                requests: Mutex::new(vec![]),
                active: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                fail_once: AtomicBool::new(fail),
            }
        }
    }
    #[async_trait]
    impl TextExecutor for Fake {
        async fn execute(&self, r: TextRequest) -> Result<Value, String> {
            let key = value_sha(&r)?;
            if let Some(value) = self.cache.lock().unwrap().get(&key).cloned() {
                return Ok(value);
            }
            self.requests.lock().unwrap().push(r.clone());
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let index = r.input["batchIndex"].as_u64().unwrap_or(0);
            tokio::time::sleep(std::time::Duration::from_millis(if index % 3 == 0 {
                15
            } else {
                3
            }))
            .await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            if r.stage == "foundation:F2"
                && index == 4
                && self.fail_once.swap(false, Ordering::SeqCst)
            {
                return Err("synthetic one stopped call".into());
            }
            let mut value = if r.stage == "foundation:F2" {
                json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":r.input["sourceSha256"],"coverage":r.input["scope"],"assets":[]})
            } else {
                json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":r.input["sourceSha256"],"coverage":r.input["scope"],"changes":{}})
            };
            if r.input.get("foundationParallelFinal").is_some() {
                value["seriesDirection"] = json!({"content":"保持各集完整原文。","signals":[]});
            }
            self.cache.lock().unwrap().insert(key, value.clone());
            Ok(value)
        }
        async fn foundation_parallel_receipt_time(&self, _input: &Value) -> Result<String, String> {
            Ok("2026-10-27T12:00:00Z".into())
        }
    }
    fn large_fixture() -> (LockedSource, Value) {
        let source = prepare_source(
            &(1..=7)
                .map(|i| format!("第{i}集\n场景：大厅\n△{}\n", "空旷的大厅。".repeat(2500)))
                .collect::<String>(),
        )
        .unwrap();
        let ids: Vec<_> = source.episodes.iter().map(|e| e.id.clone()).collect();
        let seed = plan_batches(&source, &ids).unwrap()[0].scope.clone();
        let context = json!({"sourceVersion":1,"selection":{"episodeIds":ids},"foundationParallelTrial":{"version":PROFILE,"concurrency":3,"replayBatchCount":3,"seedBatchCount":1,"historyEpisodeIds":seed,"authorizationSha256":"a".repeat(64),"workflowId":"synthetic"}});
        (source, context)
    }
    #[tokio::test]
    async fn runner_replays_prefix_and_drains_three_calls_then_recovers_identical_requests() {
        let (source, context) = large_fixture();
        let executor = Fake::new(true);
        assert!(run(&source, &context, &executor, "test", "medium")
            .await
            .unwrap_err()
            .contains("synthetic one stopped call"));
        assert_eq!(executor.active.load(Ordering::SeqCst), 0);
        let before = executor.requests.lock().unwrap().clone();
        assert_eq!(
            before.len(),
            6,
            "three original + three fully drained siblings"
        );
        assert_eq!(executor.cache.lock().unwrap().len(), 5);
        let output = run(&source, &context, &executor, "test", "medium")
            .await
            .unwrap();
        assert_eq!(output.updated_episode_ids.len(), 7);
        assert_eq!(executor.peak.load(Ordering::SeqCst), 3);
        let requests = executor.requests.lock().unwrap();
        for index in 0..3 {
            assert!(requests[index]
                .input
                .get("foundationParallelTrial")
                .is_none());
        }
        for request in requests
            .iter()
            .filter(|r| r.input.get("foundationParallelTrial").is_some())
        {
            let marker = &request.input["foundationParallelTrial"];
            assert_eq!(
                marker["authorizationSha256"],
                context["foundationParallelTrial"]["authorizationSha256"]
            );
            assert!(marker["laneId"].as_u64().unwrap() < 3);
            assert_eq!(marker["basisSha256"].as_str().unwrap().len(), 64);
            if request.stage == "foundation:Q1" {
                assert_eq!(
                    request.input["seriesDirectionPreparation"]["mode"],
                    if request.input.get("foundationParallelFinal").is_some() {
                        "FINAL"
                    } else {
                        "SIGNALS"
                    }
                );
            }
        }
        for index in [0, 1, 2, 3, 5] {
            let hash = value_sha(&before[index]).unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| value_sha(r).unwrap() == hash)
                    .count(),
                1,
                "accepted request is recovered without a new call"
            );
        }
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.input.get("foundationParallelFinal").is_some())
                .count(),
            1
        );
    }
    #[test]
    fn unconflicted_asset_entry_cannot_be_rewritten_during_rebase() {
        let a = json!({"assets":[{"description":"conflict"},{"description":"keep"}]});
        let b = json!({"assets":[{"description":"fixed"},{"description":"keep"}]});
        assert!(validate_rebase_targets("F2", &a, &b, &json!([0])).is_ok());
        let b = json!({"assets":[{"description":"fixed"},{"description":"rewritten"}]});
        assert!(validate_rebase_targets("F2", &a, &b, &json!([0])).is_err());
    }
    #[tokio::test]
    #[ignore = "explicit read-only real-source capacity rehearsal; no provider calls"]
    async fn real_source_readonly_prefix_and_first_wave_preview() {
        let db = std::env::var("DIRECTOR_FOUNDATION_TRIAL_PREVIEW_DB")
            .expect("explicit readonly database required");
        let workflow = std::env::var("DIRECTOR_FOUNDATION_TRIAL_PREVIEW_RUN")
            .unwrap_or_else(|_| "dd6d6226-a1d5-42c0-9b5d-1460b5ca466b".into());
        let out = std::path::PathBuf::from(
            std::env::var("DIRECTOR_FOUNDATION_TRIAL_PREVIEW_REPORT_DIR")
                .expect("explicit existing .local output directory required"),
        )
        .canonicalize()
        .unwrap();
        let local = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.local")
            .canonicalize()
            .unwrap();
        assert!(
            out.starts_with(&local),
            "private rehearsal report must remain in repository .local"
        );
        let c = rusqlite::Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let raw: String = c
            .query_row(
                "SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow'",
                [&workflow],
                |r| r.get(0),
            )
            .unwrap();
        let snapshot: Value = serde_json::from_str(&raw).unwrap();
        let input = &snapshot["input"];
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let mut q=c.prepare("SELECT j.snapshot,w.result FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage='foundation:F2' AND w.state='accepted' AND j.state='completed' AND json_extract(j.snapshot,'$.input.foundationParallelTrial') IS NULL ORDER BY w.rowid LIMIT 3").unwrap();
        let originals: Vec<(TextRequest, Value)> = q
            .query_map([&workflow], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .unwrap()
            .map(|r| {
                let (request, result) = r.unwrap();
                (
                    serde_json::from_str(&request).unwrap(),
                    serde_json::from_str(&result).unwrap(),
                )
            })
            .collect();
        assert_eq!(originals.len(), 3);
        let mut context = json!({"foundation":input["foundation"],"assets":input["assets"],"selection":input["selection"],"foundationModelInputProfile":input["foundationModelInputProfile"],"foundationPhase":input["foundationPhase"],"confirmedDecisions":input["confirmedDecisions"],"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundationDecisionPolicy":input["foundationDecisionPolicy"],"automaticDecisions":input["automaticDecisions"],"userReviewRequirements":input["userReviewRequirements"],"settings":{"confirmedDirection":input["foundation"]["productionDirection"]}});
        crate::pipeline::asset_coverage::inherit(&mut context, input).unwrap();
        let covered: Vec<Value> = originals
            .iter()
            .flat_map(|(r, _)| r.input["scope"].as_array().unwrap().iter().cloned())
            .collect();
        assert_eq!(covered.len(), 49);
        assert_eq!(originals[0].0.input["scope"].as_array().unwrap().len(), 27);
        context["foundationParallelTrial"] =
            crate::foundation_trial_store::preview_contract(&c, &workflow, &snapshot).unwrap();
        struct ReadOnlyPreview {
            originals: Vec<(TextRequest, Value)>,
            replayed: Mutex<Vec<usize>>,
            new: Mutex<Vec<TextRequest>>,
        }
        #[async_trait]
        impl TextExecutor for ReadOnlyPreview {
            async fn execute(&self, r: TextRequest) -> Result<Value, String> {
                if r.input.get("foundationParallelTrial").is_some() {
                    self.new.lock().unwrap().push(r);
                    return Err("PREVIEW_ONLY_NO_MODEL".into());
                }
                let index = r.input["batchIndex"]
                    .as_u64()
                    .ok_or("missing old batch index")? as usize;
                let (expected, result) = self
                    .originals
                    .get(index)
                    .ok_or("unexpected legacy request")?;
                if r.input != expected.input
                    || r.schema != expected.schema
                    || r.stage != expected.stage
                    || r.rule_snapshot != expected.rule_snapshot
                    || r.model != expected.model
                    || r.reasoning != expected.reasoning
                {
                    return Err(format!(
                        "PREVIEW_LEGACY_MISMATCH_{index}: frozen input/schema/rules/model changed"
                    ));
                }
                self.replayed.lock().unwrap().push(index);
                Ok(result.clone())
            }
        }
        let model = originals[0].0.model.clone();
        let reasoning = originals[0].0.reasoning.clone();
        let exec = ReadOnlyPreview {
            originals,
            replayed: Mutex::new(vec![]),
            new: Mutex::new(vec![]),
        };
        let result = run(&source, &context, &exec, &model, &reasoning).await;
        let replayed = exec.replayed.lock().unwrap().clone();
        let requests = exec.new.lock().unwrap();
        let summary:Vec<Value>=requests.iter().map(|r|{
            let scope=r.input["scope"].as_array().unwrap();
            let numbers:Vec<_>=scope.iter().map(|id|source.episodes.iter().find(|e|Some(e.id.as_str())==id.as_str()).unwrap().number).collect();
            let history=expand_history(&r.input["evidence"]["historicalEvidenceCandidates"]).unwrap();
            let seed=&context["foundationParallelTrial"]["historyEpisodeIds"];
            let only_seed=history.as_array().unwrap().iter().all(|row|seed.as_array().unwrap().contains(&row[1]));
            json!({"batchIndex":r.input["batchIndex"],"laneId":r.input["foundationParallelTrial"]["laneId"],"episodeNumbers":numbers,"requestChars":request_chars("F2",&r.input,&r.schema).unwrap(),"fits":fits("F2",&r.input,&r.schema).unwrap(),"historyOnlySeed":only_seed,"inputSha256":value_sha(&r.input).unwrap(),"rows":r.input["source"]["rows"].as_array().unwrap().len()})
        }).collect();
        let error = result
            .err()
            .unwrap_or_else(|| "UNEXPECTED_COMPLETION".into());
        let report = json!({"kind":"READ_ONLY_CAPACITY_REHEARSAL_NOT_CLIENT_ACCEPTANCE","databaseAccess":"SQLITE_OPEN_READ_ONLY","modelCalls":0,"workflowId":workflow,"sourceSha256":source.sha256,"replayedBatchIndexes":replayed,"replayedEpisodeCount":covered.len(),"seedEpisodeCount":27,"firstWave":summary,"stop":error});
        std::fs::write(
            out.join("real-source-first-wave-preview.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        assert_eq!(error, "PREVIEW_ONLY_NO_MODEL", "{error}");
        assert_eq!(replayed, vec![0, 1, 2]);
        assert!(!requests.is_empty() && requests.len() <= 3);
        assert_eq!(summary[0]["episodeNumbers"][0], 50);
        assert!(summary
            .iter()
            .all(|s| s["fits"] == true && s["historyOnlySeed"] == true));
        let mut ordered = summary.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|s| s["batchIndex"].as_u64().unwrap());
        let ids: Vec<_> = ordered
            .iter()
            .flat_map(|s| s["episodeNumbers"].as_array().unwrap())
            .filter_map(Value::as_u64)
            .collect();
        assert!(ids.windows(2).all(|pair| pair[1] == pair[0] + 1));
    }
    #[test]
    fn automatic_q1_uses_actual_frozen_receipt_time_instead_of_now() {
        let source = prepare_source("第1集\n场景：大厅\n△空旷大厅。\n").unwrap();
        let ids = source
            .episodes
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let batch = plan_batches(&source, &ids).unwrap().remove(0);
        let initial = empty_foundation(&source);
        let context = json!({"sourceVersion":1,"projectId":"synthetic-time","selection":{"episodeIds":ids},"foundationDecisionPolicy":AUTO_INTERPRET_POLICY,"userReviewRequirements":[]});
        let f2 = input_for("F2", &source, &batch, &initial, &context, 0, 1, false, &[]);
        let raw = json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":source.sha256,"coverage":batch.scope,"assets":[],"humanChoices":[{"decisionKey":"empty-room","kind":"PRODUCTION_CHOICE","question":"大厅采用哪种制作解释？","reason":"不追加人物","options":["保留空厅","仅补声场"],"recommended":"保留空厅","recommendationReason":"保留原文动作","evidence":"已提供原文行","episodes":batch.scope,"sourceRefs":[source.rows[1].id,source.rows[2].id]}]});
        let basis =
            apply_catalogue(&raw, &source, &batch, &f2, &initial, None, &HashSet::new()).unwrap();
        let q1 = input_for("Q1", &source, &batch, &basis, &context, 0, 1, false, &[]);
        assert!(!q1["automaticDecisions"].as_array().unwrap().is_empty());
        let raw = json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":source.sha256,"coverage":batch.scope,"changes":{},"appliedDecisions":q1["automaticDecisions"].as_array().unwrap().iter().map(|d|json!({"id":d["id"],"answer":"保留空厅","note":"保留原文，解释独立记录","subjectAssetKeys":[]})).collect::<Vec<_>>()});
        let time = "2026-10-27T12:34:56.000Z";
        let first = apply(
            "Q1",
            &raw,
            &source,
            &batch,
            &q1,
            &basis,
            None,
            &HashSet::new(),
            time,
        )
        .unwrap()
        .0;
        let second = apply(
            "Q1",
            &raw,
            &source,
            &batch,
            &q1,
            &basis,
            None,
            &HashSet::new(),
            time,
        )
        .unwrap()
        .0;
        assert_eq!(value_sha(&first).unwrap(), value_sha(&second).unwrap());
        let receipts = prior_auto_records(&first);
        assert!(!receipts.is_empty());
        assert!(receipts.iter().all(|r| r["appliedAt"] == time));
    }
    #[test]
    fn rebase_cannot_change_nonconflicting_fields_on_same_asset() {
        let a = json!({"assets":[{"description":"old","role":"main"}]});
        let b = json!({"assets":[{"description":"new","role":"main"}]});
        assert!(validate_rebase_fields("F2", &a, &b, &json!({"0":["description"]})).is_ok());
        let b = json!({"assets":[{"description":"new","role":"changed"}]});
        assert!(validate_rebase_fields("F2", &a, &b, &json!({"0":["description"]})).is_err());
    }
    #[test]
    fn bound_rebase_preserves_sibling_nonconflicting_fields_and_original_intent() {
        let base =
            json!({"assets":[{"id":"a","description":"A","aliases":["old"],"role":"support"}]});
        let current =
            json!({"assets":[{"id":"a","description":"B","aliases":["brother"],"role":"support"}]});
        let original =
            json!({"assets":[{"id":"a","description":"A","aliases":["candidate"],"role":"lead"}]});
        // Applying the raw rebase directly to current would revert B to A.
        let applied =
            json!({"assets":[{"id":"a","description":"A","aliases":["resolved"],"role":"lead"}]});
        let resolutions = HashMap::from([(
            "/assets/a/aliases".into(),
            identity_path(&applied, "/assets/a/aliases").cloned(),
        )]);
        let mut conflicts = vec![];
        let merged = merge_resolved_value(
            Some(&base),
            Some(&current),
            Some(&original),
            "",
            &mut conflicts,
            &resolutions,
        )
        .unwrap();
        assert!(conflicts.is_empty());
        assert_eq!(merged["assets"][0]["description"], "B");
        assert_eq!(merged["assets"][0]["aliases"], json!(["resolved"]));
        assert_eq!(merged["assets"][0]["role"], "lead");
    }
    #[test]
    fn same_decision_identity_cannot_hide_opposite_answers_in_issue_union() {
        let record = |answer: &str, time: &str, episode: &str| json!({"id":"choice","decisionPolicy":AUTO_INTERPRET_POLICY,"answer":answer,"subjectAssetKeys":["a"],"episodes":["e1","e2"],"status":"PARTIALLY_APPLIED","appliedAt":time,"appliedNote":episode,"receipts":[{"coverage":[episode],"note":episode}]});
        let issue = |record: Value| json!({"coordinate":"interpretation:choice","problem":record["answer"],"details":{"kind":"APPLIED_INTERPRETATION","authorDecision":record}});
        let base = json!({"issues":[],"assets":[{"id":"a","authorDecisions":[]}]});
        let left = record("same", "time-one", "e1");
        let right = record("same", "time-two", "e2");
        let current =
            json!({"issues":[issue(left.clone())],"assets":[{"id":"a","authorDecisions":[left]}]});
        let candidate = json!({"issues":[issue(right.clone())],"assets":[{"id":"a","authorDecisions":[right]}]});
        let mut conflicts = vec![];
        let merged = merge_value(
            Some(&base),
            Some(&current),
            Some(&candidate),
            "",
            &mut conflicts,
        )
        .unwrap();
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(merged["issues"].as_array().unwrap().len(), 1);
        assert_eq!(
            merged["issues"][0]["details"]["authorDecision"]["appliedAt"],
            "time-one"
        );
        assert_eq!(
            merged["assets"][0]["authorDecisions"][0]["status"],
            "APPLIED"
        );
        let bad = record("opposite", "time-three", "e2");
        let candidate =
            json!({"issues":[issue(bad.clone())],"assets":[{"id":"a","authorDecisions":[bad]}]});
        conflicts.clear();
        merge_value(
            Some(&base),
            Some(&current),
            Some(&candidate),
            "",
            &mut conflicts,
        );
        assert!(conflicts
            .iter()
            .any(|p| p.starts_with("/issues/") && p.ends_with("decision-semantics")));
        assert!(conflicts
            .iter()
            .any(|p| p.starts_with("/assets/") && p.ends_with("decision-semantics")));
    }
    #[test]
    fn three_way_merge_preserves_disjoint_scope_and_never_last_writer_wins() {
        let b = json!({"assets":[{"id":"a","description":"old","sourceRefs":["r0"],"variants":[{"id":"v","sourceVariantId":null,"scope":{"scenes":[]}}]}]});
        let mut c = b.clone();
        c["assets"][0]["description"] = json!("first");
        c["assets"][0]["sourceRefs"] = json!(["r0", "r1"]);
        let mut n = b.clone();
        n["assets"][0]["sourceRefs"] = json!(["r0", "r2"]);
        let mut conflicts = vec![];
        let merged = merge_value(Some(&b), Some(&c), Some(&n), "", &mut conflicts).unwrap();
        assert!(conflicts.is_empty());
        assert_eq!(merged["assets"][0]["description"], "first");
        assert_eq!(merged["assets"][0]["sourceRefs"], json!(["r0", "r1", "r2"]));
        n["assets"][0]["description"] = json!("second");
        conflicts.clear();
        merge_value(Some(&b), Some(&c), Some(&n), "", &mut conflicts);
        assert_eq!(conflicts, vec!["/assets/a/description"]);
    }
    #[test]
    fn parallel_parent_and_remove_edit_conflicts_remain_explicit() {
        let b = json!({"variants":[{"id":"v","sourceVariantId":"root"}]});
        let c = json!({"variants":[{"id":"v","sourceVariantId":"p1"}]});
        let n = json!({"variants":[{"id":"v","sourceVariantId":"p2"}]});
        let mut conflicts = vec![];
        merge_value(Some(&b), Some(&c), Some(&n), "", &mut conflicts);
        assert_eq!(conflicts, vec!["/variants/v/sourceVariantId"]);
        conflicts.clear();
        merge_value(
            Some(&b),
            Some(&json!({"variants":[]})),
            Some(&n),
            "",
            &mut conflicts,
        );
        assert_eq!(conflicts, vec!["/variants/v"]);
    }
    #[test]
    fn rebase_keeps_batch_and_unrelated_changes_exact() {
        let old = json!({"format":"x","sourceSha256":"s","coverage":["e"],"assets":[{"name":"A","type":"CH","episodes":["e"],"sceneRefs":[],"preparation":{"mode":"REFERENCE"}}]});
        let mut next = old.clone();
        next["assets"][0]["key"] = json!("existing");
        assert!(validate_rebase_result("F2", &old, &next).is_ok());
        next["coverage"] = json!(["other"]);
        assert!(validate_rebase_result("F2", &old, &next).is_err());
        let old = json!({"format":"x","sourceSha256":"s","coverage":["e"],"changes":{"assets":[],"overview":"old"}});
        let mut next = old.clone();
        next["changes"]["overview"] = json!("new");
        assert!(validate_rebase_result("Q1", &old, &next).is_err());
    }
}
