//! Old A15_A: one bounded source pass, program validation, accepted checkpoints.
use super::*;
use std::collections::{BTreeMap, HashSet};

const CAPACITY: usize = 24_000;
const WIRE_PROFILE: &str = "asset-visual-facts-compact/v1";
const WIRE_RESULT: &str = "asset-visual-facts-wire/v1";
pub(crate) const VISUAL_FACT_REPAIR_PROFILE: &str = "asset-visual-fact-repair/v1";
pub(crate) const VISUAL_FACT_UNBOUND_ERROR: &str = "局部视觉状态未绑定明确版本";
pub(crate) const VISUAL_FACT_REPAIR_RULE: &str = "沿用原A15_A视觉特征提取方法，仅修订程序指出的未绑定STATE/TRANSITION事实。所有锁定原文与主体/已有版本目录仍为本包资料；不新增身份或版本，不修改原事实的属性、值、来源层次及原行出处，不把后续变化强塞母形象。每个factIndex必须恰好返回一项局部运输修订：BIND_EXISTING_VERSION仅绑定该主体已登记且原文支持的版本；NOT_ASSET_VISUAL仅用于原方法明确不登记的战斗过程、瞬时效果或非设定图变化，原事实仍保留审计，不能用于绕过真实未确定的衣装/伤态；UNRESOLVED保留无法定位的视觉状态，阻塞仅该主体及其必要派生。reason须说明原文与版本依据，不得以数量唯一作为时间适用证据。无需回抄其它已有效事实，不审查或重建目录。";
const PRODUCTION_CHOICE_RULE: &str = "acceptedProductionConstraints是已经采用的当前制作取舍，精确绑定源版本、形象ID、版本及适用集场；preparation.note不是SOURCE视觉事实，不能改写或覆盖原文。packetApplicability仅把其既有集场ID对应到本包只读短引用。若原文明确记载未来视觉变化，但当前正式制作取舍与适用范围明确尚未采用此状态，可以用CURRENT_SCOPE_NOT_APPLICABLE；必须逐形象列出notApplicableAssetIds并引用对应productionConstraintRefs，说明本批不适用依据。原未来事实的值、属性、claim、出处和未绑定状态必须完整保留，不绑定基础形象、不冒称NOT_ASSET_VISUAL、不新增形象。没有足够正式范围依据仍返回UNRESOLVED，不能把仅有一个版本当时间证据。对其它操作productionConstraintRefs/notApplicableAssetIds返回空数组。该裁决仅允许这些准确形象在当前冻结制作取舍下继续，未覆盖形象仍局部阻塞。";

fn repair_rule(constraints: &[Value]) -> String {
    if constraints.is_empty() {
        VISUAL_FACT_REPAIR_RULE.into()
    } else {
        format!("{VISUAL_FACT_REPAIR_RULE}\n{PRODUCTION_CHOICE_RULE}")
    }
}

pub(crate) fn is_visual_fact_repair(request: &TextRequest) -> bool {
    request.stage == "assetVisualFacts"
        && request.skill == "asset-visual-facts-extractor"
        && request.input["visualFactRepair"]["version"] == VISUAL_FACT_REPAIR_PROFILE
}

pub(crate) fn validate_visual_fact_repair_request(request: &TextRequest) -> Result<(), String> {
    if request.input.get("visualFactRepair").is_none() {
        return Ok(());
    }
    let descriptor = &request.input["visualFactRepair"];
    let constraints = descriptor
        .get("acceptedProductionConstraints")
        .map(|value| value.as_array().ok_or("正式制作取舍目录类型错误"))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    validate_constraints(&constraints, text(descriptor, "sourceSha256"))?;
    for constraint in &constraints {
        let identity = request.input["identities"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|identity| identity["assetId"] == constraint["subjectAssetId"])
            .ok_or("正式制作取舍主体越出原视觉包")?;
        let version = identity["versions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|version| {
                version["assetRowId"] == constraint["assetId"]
                    && version["variantId"] == constraint["variantId"]
            })
            .ok_or("正式制作取舍形象越出原视觉包")?;
        if version["name"] != constraint["name"]
            || ["episodes", "scenes"].iter().any(|key| {
                strings(&version["applicability"], key)
                    != strings(&constraint["packetApplicability"], key)
            })
        {
            return Err("正式制作取舍的形象显示名或本包集场映射改变".into());
        }
    }
    let content = repair_rule(&constraints);
    if !is_visual_fact_repair(request)
        || descriptor["maxAttempts"] != 2
        || !descriptor["attempt"]
            .as_u64()
            .is_some_and(|attempt| (1..=2).contains(&attempt))
        || descriptor["sourceSha256"] != request.input["sourceSha256"]
        || descriptor["packetFingerprint"] != request.input["coveredPacketRef"]
        || descriptor["originalRawSha256"]
            != digest(&request.input["rejectedVisualFacts"].to_string())
        || request.rule_snapshot["runtimeContract"]["version"] != VISUAL_FACT_REPAIR_PROFILE
        || request.rule_snapshot["runtimeContract"]["content"] != content
        || request.rule_snapshot["runtimeContract"]["sha256"] != digest(&content)
    {
        return Err("视觉事实局部修订的原包/原回包/两次预算或原岗位合同无效，未发送请求".into());
    }
    if !constraints.is_empty()
        && descriptor["productionConstraintsSha256"] != digest(&json!(constraints).to_string())
        || descriptor
            .get("legacyProductionConstraintUpgrade")
            .is_some()
            && (descriptor["legacyProductionConstraintUpgrade"] != true
                || descriptor["attempt"] != 2
                || constraints.is_empty())
    {
        return Err("正式制作取舍摘要或旧局部修订续接边界无效".into());
    }
    for field in [
        "sourceSha256",
        "packetFingerprint",
        "originalDomainRequestSha256",
        "originalRawSha256",
        "baseFactsSha256",
    ] {
        let hash = text(descriptor, field);
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("视觉事实局部修订缺少完整源绑定哈希，未发送请求".into());
        }
    }
    Ok(())
}

fn validate_constraints(constraints: &[Value], source_sha: &str) -> Result<(), String> {
    let mut ids = HashSet::new();
    for constraint in constraints {
        let mut canonical = constraint.clone();
        canonical
            .as_object_mut()
            .ok_or("正式制作取舍不是对象")?
            .remove("constraintSha256");
        if text(constraint, "assetId").is_empty()
            || !ids.insert(text(constraint, "assetId"))
            || text(constraint, "subjectAssetId").is_empty()
            || text(constraint, "variantId").is_empty()
            || !constraint["assetVersion"]
                .as_u64()
                .is_some_and(|version| version > 0)
            || constraint["sourceSha256"] != source_sha
            || text(&constraint["preparation"], "note").trim().is_empty()
            || constraint["constraintSha256"] != digest(&canonical.to_string())
            || text(constraint, "sourceRefsSha256").len() != 64
            || !text(constraint, "sourceRefsSha256")
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("正式制作取舍缺少准确形象/源/版本或摘要绑定".into());
        }
        let mut has_scope = false;
        for key in ["episodes", "scenes"] {
            let values = constraint["applicability"][key]
                .as_array()
                .ok_or("正式制作取舍范围缺失")?;
            let mut unique = HashSet::new();
            for value in values {
                let id = value
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("正式制作取舍范围ID无效")?;
                if !unique.insert(id) {
                    return Err("正式制作取舍范围重复".into());
                }
                has_scope = true;
            }
        }
        if !has_scope {
            return Err("正式制作取舍不能以空范围冒充全局适用".into());
        }
    }
    Ok(())
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn subject(asset: &Value) -> &str {
    if text(asset, "baseAssetId").is_empty() {
        text(asset, "id")
    } else {
        text(asset, "baseAssetId")
    }
}
fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// The cache identity is the original domain packet, never its transport.
fn canonical_packet(source: &LockedSource, identities: &[Value], rows: &[&SourceRow]) -> Value {
    json!({"sourceSha256":source.sha256,"identities":identities,"rows":rows.iter().map(|row|json!({"id":row.id,"number":row.number,"text":row.text,"kind":row.kind,"episodeId":row.episode_id,"sceneId":row.scene_id})).collect::<Vec<_>>()})
}

fn ordered_subset(requested: &[String], original: &[String]) -> bool {
    let mut cursor = 0;
    for id in requested {
        let Some(position) = original[cursor..]
            .iter()
            .position(|candidate| candidate == id)
        else {
            return false;
        };
        cursor += position + 1;
    }
    true
}

/// An adopted packet may cover a later smaller reading scope. Its original
/// identity is rebuilt from the locked source, never relabeled as the new packet.
fn validate_cached_packet<'a>(
    cache: &'a Value,
    source: &LockedSource,
    identities: &[Value],
    requested_ids: &[String],
    current_rule_sha: &str,
) -> Result<&'a Vec<Value>, String> {
    if cache["schemaVersion"] != "asset-visual-facts-cache/v1"
        || cache["status"] != "accepted"
        || cache["sourceSha256"] != source.sha256
        || text(cache, "ruleSha256") != current_rule_sha
    {
        return Err("视觉事实缓存的已采用回执、原文或规则身份变化".into());
    }
    let cached_ids: Vec<String> = cache["rowIds"]
        .as_array()
        .ok_or("视觉事实缓存缺少原行目录")?
        .iter()
        .map(|id| {
            id.as_str()
                .map(str::to_string)
                .ok_or("视觉事实缓存原行ID无效")
        })
        .collect::<Result<_, _>>()?;
    let all_ids: Vec<_> = source.rows.iter().map(|row| row.id.clone()).collect();
    let unique: HashSet<_> = cached_ids.iter().collect();
    if cached_ids.is_empty()
        || unique.len() != cached_ids.len()
        || !ordered_subset(&cached_ids, &all_ids)
        || !ordered_subset(requested_ids, &cached_ids)
    {
        return Err("视觉事实缓存原行缺失、重复、乱序或不覆盖当前范围".into());
    }
    let by_id: BTreeMap<_, _> = source
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    let cached_rows: Vec<_> = cached_ids.iter().map(|id| by_id[id.as_str()]).collect();
    if digest(&canonical_packet(source, identities, &cached_rows).to_string())
        != text(cache, "inputFingerprint")
    {
        return Err("视觉事实缓存原包哈希与当前锁定来源/身份目录不一致".into());
    }
    let known: HashSet<_> = cached_ids.iter().map(String::as_str).collect();
    let facts = cache["facts"].as_array().ok_or("视觉事实缓存缺少facts")?;
    for fact in facts {
        if fact["sourceSha256"] != source.sha256 {
            return Err("视觉事实缓存事实源版本变化".into());
        }
        validate_fact(fact, identities, &known)?;
    }
    for (field, disposition) in [
        ("unassignedFacts", "UNRESOLVED"),
        ("excludedFacts", "NOT_ASSET_VISUAL"),
        ("deferredFacts", "CURRENT_SCOPE_NOT_APPLICABLE"),
    ] {
        if let Some(records) = cache.get(field) {
            let records = records.as_array().ok_or("视觉事实缓存的局部分区无效")?;
            for record in records {
                let fact = &record["fact"];
                validate_fact_checked(fact, identities, &known, false)?;
                if fact["sourceSha256"] != source.sha256
                    || record["disposition"] != disposition
                    || !fact["variantId"].is_null()
                    || !["STATE", "TRANSITION"].contains(&text(fact, "scope"))
                {
                    return Err("视觉事实缓存的局部原回包审计不匹配".into());
                }
            }
        }
    }
    Ok(facts)
}

struct PacketWire {
    input: Value,
    row_ids: Vec<String>,
    row_aliases: Vec<String>,
    rows_by_alias: BTreeMap<String, String>,
    packet_hash: String,
    episode_aliases: BTreeMap<String, String>,
    scene_aliases: BTreeMap<String, String>,
}

fn aliases<'a>(
    prefix: char,
    ids: impl IntoIterator<Item = &'a str>,
    occupied: &mut HashSet<String>,
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let mut number = 0;
    for id in ids {
        if result.contains_key(id) {
            continue;
        }
        let alias = loop {
            number += 1;
            let candidate = format!("{prefix}{number}");
            if occupied.insert(candidate.clone()) {
                break candidate;
            }
        };
        result.insert(id.to_string(), alias);
    }
    result
}

impl PacketWire {
    fn compile(source: &LockedSource, identities: &[Value], rows: &[&SourceRow]) -> Self {
        let packet = canonical_packet(source, identities, rows);
        let packet_hash = digest(&packet.to_string());
        // An alias cannot be confused with any real source/subject identity,
        // including a legacy identity that already happens to be named r1.
        let mut occupied: HashSet<String> = source
            .rows
            .iter()
            .map(|r| r.id.clone())
            .chain(source.episodes.iter().map(|e| e.id.clone()))
            .chain(source.scenes.iter().map(|s| s.id.clone()))
            .collect();
        let mut episode_ids: HashSet<String> =
            rows.iter().filter_map(|r| r.episode_id.clone()).collect();
        let mut scene_ids: HashSet<String> =
            rows.iter().filter_map(|r| r.scene_id.clone()).collect();
        for identity in identities {
            occupied.insert(text(identity, "assetId").into());
            for version in identity["versions"].as_array().into_iter().flatten() {
                occupied.insert(text(version, "assetRowId").into());
                occupied.insert(text(version, "variantId").into());
                episode_ids.extend(strings(&version["applicability"], "episodes"));
                scene_ids.extend(strings(&version["applicability"], "scenes"));
            }
        }
        episode_ids.extend(
            source
                .scenes
                .iter()
                .filter(|s| scene_ids.contains(&s.id))
                .map(|s| s.episode_id.clone()),
        );
        // Source order determines aliases; neither names nor text are rewritten.
        let episode_aliases = aliases(
            'e',
            source
                .episodes
                .iter()
                .filter(|e| episode_ids.contains(&e.id))
                .map(|e| e.id.as_str()),
            &mut occupied,
        );
        let scene_aliases = aliases(
            's',
            source
                .scenes
                .iter()
                .filter(|s| scene_ids.contains(&s.id))
                .map(|s| s.id.as_str()),
            &mut occupied,
        );
        let row_map = aliases('r', rows.iter().map(|r| r.id.as_str()), &mut occupied);
        let row_ids: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
        let row_aliases: Vec<_> = row_ids.iter().map(|id| row_map[id].clone()).collect();
        let rows_by_alias = row_map
            .iter()
            .map(|(id, alias)| (alias.clone(), id.clone()))
            .collect();
        let mut compact_identities = identities.to_vec();
        for identity in &mut compact_identities {
            for version in identity["versions"].as_array_mut().into_iter().flatten() {
                for (key, map) in [("episodes", &episode_aliases), ("scenes", &scene_aliases)] {
                    if let Some(ids) = version["applicability"]
                        .get_mut(key)
                        .and_then(Value::as_array_mut)
                    {
                        for id in ids {
                            if let Some(alias) = id.as_str().and_then(|id| map.get(id)) {
                                *id = json!(alias);
                            }
                        }
                    }
                }
            }
        }
        let episodes: Vec<_> = source.episodes.iter().filter_map(|episode| {
            let alias = episode_aliases.get(&episode.id)?;
            Some(json!({"id":alias,"number":episode.number,"sourceNumber":episode.source_number,"title":episode.title}))
        }).collect();
        let scenes: Vec<_> = source
            .scenes
            .iter()
            .filter_map(|scene| {
                let alias = scene_aliases.get(&scene.id)?;
                let mut value = serde_json::to_value(scene).expect("source scene is serializable");
                value.as_object_mut().unwrap().remove("rowIds");
                value["id"] = json!(alias);
                value["episodeId"] = json!(episode_aliases
                    .get(&scene.episode_id)
                    .unwrap_or(&scene.episode_id));
                Some(value)
            })
            .collect();
        let input = json!({"sourceSha256":source.sha256,"referenceEncoding":WIRE_PROFILE,"domain":if rows.iter().all(|r|r.episode_id.is_none()){"AUTHOR"}else{"EPISODE"},"coveredPacketRef":packet_hash,"identities":compact_identities,"episodes":episodes,"scenes":scenes,"rows":rows.iter().map(|row|json!({"id":row_map[&row.id],"number":row.number,"text":row.text,"kind":row.kind,"episodeId":row.episode_id.as_ref().map(|id|episode_aliases.get(id).unwrap_or(id)),"sceneId":row.scene_id.as_ref().map(|id|scene_aliases.get(id).unwrap_or(id))})).collect::<Vec<_>>(),"transportInstructions":"原版A15_A岗位方法不变，仅替换Skill末尾的桌面运输外壳。rows的text和number是锁定原文与真实行号，id为本包短行引用；episodes/scenes提供只读集场名称、原集号和权威信息，identities仍仅为主体及已有版本定位，不能充当外观证据。AUTHOR与EPISODE按本包domain分别处理，不提升角色陈述为客观事实。输出严格asset-visual-facts-wire/v1：schemaVersion、coveredPacketRef、facts、issues；coveredPacketRef原样返回输入值，确认处理本包全部rows一次。facts仍为assetId/variantId/attribute/value/scope/claim/sourceRefs，sourceRefs只用本包row.id短引用，至少一项且不重复；Rust还原到锁定原文。issues为空；无明确事实允许facts为空。不要回抄coveredRowIds、原文、行坐标或集场坐标，不创造范围外证据。"});
        Self {
            input,
            row_ids,
            row_aliases,
            rows_by_alias,
            packet_hash,
            episode_aliases,
            scene_aliases,
        }
    }

    fn schema(&self, identities: &[Value]) -> Value {
        let owner_ids: Vec<_> = identities.iter().map(|i| text(i, "assetId")).collect();
        let variants: Vec<_> = identities
            .iter()
            .flat_map(|i| {
                i["versions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| json!(text(v, "variantId")))
            })
            .chain(std::iter::once(Value::Null))
            .collect();
        let mut schema = obj(
            json!({"schemaVersion":choices(&[WIRE_RESULT]),"coveredPacketRef":{"type":"string","enum":[self.packet_hash]},"facts":array(obj(json!({"assetId":{"type":"string","enum":owner_ids},"variantId":{"type":["string","null"],"enum":variants},"attribute":string(),"value":string(),"scope":choices(&["STABLE","STATE","TRANSITION"]),"claim":choices(&["SOURCE","CHARACTER_CLAIM"]),"sourceRefs":{"type":"array","items":{"$ref":"#/$defs/sourceRow"}}}))),"issues":{"type":"array","maxItems":0,"items":{"type":"string"}}}),
        );
        // A single enum belongs to sparse facts, not to a copied coverage table.
        schema["$defs"] = json!({"sourceRow":{"type":"string","enum":self.row_aliases}});
        schema
    }

    /// Legacy canonical replies/checkpoints remain valid. Wire replies must bind
    /// this exact domain packet before short references can be interpreted.
    /// Restore and validation are atomic; reject never changes the raw reply.
    fn materialize(
        &self,
        output: &mut Value,
        identities: &[Value],
        source_sha: &str,
    ) -> Result<(), String> {
        self.materialize_checked(output, identities, source_sha, true)
    }

    /// Old A15 accepts a source-bound unassigned state as a local ownership
    /// issue, not a malformed whole packet. This probe never accepts the raw
    /// extraction; it only supplies evidence to the same role's bounded repair.
    fn materialize_checked(
        &self,
        output: &mut Value,
        identities: &[Value],
        source_sha: &str,
        require_variant: bool,
    ) -> Result<(), String> {
        let compact = match text(output, "schemaVersion") {
            WIRE_RESULT => {
                if text(output, "coveredPacketRef") != self.packet_hash
                    || output.get("coveredRowIds").is_some()
                {
                    return Err("视觉事实包运输回执不匹配".into());
                }
                true
            }
            "asset-visual-facts/v1" => {
                if output["coveredRowIds"] != json!(self.row_ids)
                    || output.get("coveredPacketRef").is_some()
                {
                    return Err("旧视觉事实包范围或回执不完整".into());
                }
                false
            }
            _ => return Err("视觉事实包运输版本未知".into()),
        };
        if output["issues"]
            .as_array()
            .map(|v| !v.is_empty())
            .unwrap_or(true)
        {
            return Err("视觉事实包范围或回执不完整".into());
        }
        let mut facts = output["facts"]
            .as_array()
            .ok_or("视觉事实包缺少facts")?
            .clone();
        let known: HashSet<_> = self.row_ids.iter().map(String::as_str).collect();
        for fact in &mut facts {
            if compact {
                let refs = fact
                    .get_mut("sourceRefs")
                    .and_then(Value::as_array_mut)
                    .ok_or("视觉事实缺少源行引用")?;
                for reference in refs {
                    let id = reference
                        .as_str()
                        .and_then(|alias| self.rows_by_alias.get(alias))
                        .ok_or("视觉事实短行引用未知、越包或类型错误")?;
                    *reference = json!(id);
                }
            }
            validate_fact_checked(fact, identities, &known, require_variant)?;
            *fact = json!({"assetId":fact["assetId"],"variantId":fact["variantId"],"attribute":fact["attribute"],"value":fact["value"],"scope":fact["scope"],"claim":fact["claim"],"sourceRefs":fact["sourceRefs"]});
            let id = digest(&fact.to_string());
            fact["id"] = json!(format!("visual-{id}"));
            fact["sourceSha256"] = json!(source_sha);
        }
        *output = json!({"schemaVersion":"asset-visual-facts/v1","coveredRowIds":self.row_ids,"facts":facts,"issues":[]});
        Ok(())
    }
}

fn identity_cards(input: &Value, assets: &[Value]) -> Vec<Value> {
    let mut grouped: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for asset in assets {
        grouped
            .entry(subject(asset).into())
            .or_default()
            .push(asset);
    }
    grouped.into_iter().map(|(id,versions)|{
        let foundation=input.pointer("/foundation/assets").and_then(Value::as_array).into_iter().flatten().find(|a|text(a,"id")==id);
        json!({"assetId":id,"name":foundation.and_then(|a|a.get("name")).or_else(||versions[0].get("name")),"aliases":foundation.and_then(|a|a.get("aliases")),"kind":versions[0].get("kind").or_else(||versions[0].get("type")),"versions":versions.iter().map(|a|json!({"assetRowId":a["id"],"variantId":a["variantId"],"name":a["name"],"applicability":a["applicability"]})).collect::<Vec<_>>()})
    }).collect()
}

/// Only accepted registry choices enter this addendum. They are not evidence
/// facts and do not change the original extraction packet or its cache key.
fn production_constraints(
    source: &LockedSource,
    wire: &PacketWire,
    identities: &[Value],
    registry: &[Value],
) -> Result<Vec<Value>, String> {
    let registered: BTreeMap<_, _> = registry
        .iter()
        .map(|asset| (text(asset, "id"), asset))
        .collect();
    if registered.len() != registry.len() {
        return Err("正式制作目录形象ID重复".into());
    }
    let known: HashSet<_> = source.rows.iter().map(|row| row.id.as_str()).collect();
    let mut constraints = Vec::new();
    for identity in identities {
        for version in identity["versions"].as_array().into_iter().flatten() {
            let Some(asset) = registered.get(text(version, "assetRowId")) else {
                continue;
            };
            let note = text(&asset["preparation"], "note");
            if note.trim().is_empty() {
                continue;
            }
            if subject(asset) != text(identity, "assetId")
                || asset["variantId"] != version["variantId"]
                || asset["applicability"] != version["applicability"]
            {
                return Err("正式制作取舍与原包形象身份/范围不一致".into());
            }
            let refs = asset["sourceRefs"]
                .as_array()
                .ok_or("正式制作取舍缺少形象来源")?;
            let mut unique = HashSet::new();
            if refs.is_empty()
                || refs.iter().any(|id| {
                    id.as_str()
                        .is_none_or(|id| !known.contains(id) || !unique.insert(id))
                })
            {
                return Err("正式制作取舍形象来源越界、重复或为空".into());
            }
            let scope = json!({"episodes":strings(&asset["applicability"], "episodes"),"scenes":strings(&asset["applicability"], "scenes")});
            let packet_scope = json!({"episodes":strings(&scope,"episodes").iter().map(|id|wire.episode_aliases.get(id).cloned().ok_or("正式制作集范围不在原包目录")).collect::<Result<Vec<_>,_>>()?,"scenes":strings(&scope,"scenes").iter().map(|id|wire.scene_aliases.get(id).cloned().ok_or("正式制作场范围不在原包目录")).collect::<Result<Vec<_>,_>>()?});
            let mut constraint = json!({"assetId":asset["id"],"subjectAssetId":identity["assetId"],"variantId":asset["variantId"],"assetVersion":asset["version"],"name":asset["name"],"description":asset["description"],"sourceSha256":source.sha256,"preparation":{"mode":asset["preparation"]["mode"],"note":note},"applicability":scope,"packetApplicability":packet_scope,"sourceAssetId":asset["sourceAssetId"],"sourceVariantId":asset["sourceVariantId"],"sourceRefsSha256":digest(&json!(refs).to_string())});
            constraint["constraintSha256"] = json!(digest(&constraint.to_string()));
            constraints.push(constraint);
        }
    }
    validate_constraints(&constraints, &source.sha256)?;
    Ok(constraints)
}

fn affected(
    source: &LockedSource,
    assets: &[&Value],
    rows: &[&SourceRow],
    reason: &str,
) -> Vec<Value> {
    assets.iter().filter(|asset|{
        let explicit:HashSet<_>=strings(asset,"sourceRefs").into_iter().collect();
        let episodes:HashSet<_>=strings(&asset["applicability"],"episodes").into_iter().chain(source.rows.iter().filter(|row|explicit.contains(&row.id)).filter_map(|row|row.episode_id.clone())).collect();
        rows.iter().any(|row|explicit.contains(&row.id)||row.kind!="format"&&row.episode_id.as_ref().map(|id|episodes.contains(id)).unwrap_or(true))
    }).map(|asset|json!({"assetId":asset["id"],"variantId":asset["variantId"],"reason":format!("视觉事实批次需要处理：{reason}")})).collect()
}

fn validate_fact(
    fact: &Value,
    identities: &[Value],
    row_ids: &HashSet<&str>,
) -> Result<(), String> {
    validate_fact_checked(fact, identities, row_ids, true)
}

fn validate_fact_checked(
    fact: &Value,
    identities: &[Value],
    row_ids: &HashSet<&str>,
    require_variant: bool,
) -> Result<(), String> {
    let identity = identities
        .iter()
        .find(|identity| text(identity, "assetId") == text(fact, "assetId"))
        .ok_or("视觉事实主体不属于本次明确目录")?;
    let variant = fact.get("variantId").and_then(Value::as_str);
    if variant
        .map(|variant| {
            !identity["versions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|v| text(v, "variantId") == variant)
        })
        .unwrap_or(false)
    {
        return Err("视觉事实版本不属于其主体".into());
    }
    if !["STABLE", "STATE", "TRANSITION"].contains(&text(fact, "scope")) {
        return Err("视觉事实属性或适用范围无效".into());
    }
    if require_variant && text(fact, "scope") != "STABLE" && variant.is_none() {
        return Err(VISUAL_FACT_UNBOUND_ERROR.into());
    }
    if !["SOURCE", "CHARACTER_CLAIM"].contains(&text(fact, "claim"))
        || text(fact, "attribute").trim().is_empty()
        || text(fact, "value").trim().is_empty()
    {
        return Err("视觉事实缺少有效属性、值或来源层次".into());
    }
    let refs = fact
        .get("sourceRefs")
        .and_then(Value::as_array)
        .ok_or("视觉事实缺少源行引用")?;
    let mut seen = HashSet::new();
    if refs.is_empty()
        || refs.iter().any(|value| {
            value
                .as_str()
                .map(|row| !row_ids.contains(row) || !seen.insert(row))
                .unwrap_or(true)
        })
    {
        return Err("视觉事实引用越界、重复或为空".into());
    }
    Ok(())
}

fn unbound_indices(facts: &[Value]) -> Vec<usize> {
    facts
        .iter()
        .enumerate()
        .filter(|(_, fact)| {
            ["STATE", "TRANSITION"].contains(&text(fact, "scope"))
                && fact.get("variantId").is_none_or(Value::is_null)
        })
        .map(|(index, _)| index)
        .collect()
}

fn blocked_subjects(assets: &[&Value], unassigned: &[Value], reason: &str) -> Vec<Value> {
    let subjects: HashSet<_> = unassigned
        .iter()
        .map(|fact| text(fact, "assetId"))
        .collect();
    assets
        .iter()
        .filter(|asset| subjects.contains(subject(asset)))
        .map(|asset| json!({"assetId":asset["id"],"variantId":asset["variantId"],"reason":reason}))
        .collect()
}

fn deferred_blocked_subjects(assets: &[&Value], cache: &Value) -> Vec<Value> {
    let mut blocked = Vec::new();
    for record in cache["deferredFacts"].as_array().into_iter().flatten() {
        for asset in assets
            .iter()
            .filter(|asset| subject(asset) == text(&record["fact"], "assetId"))
        {
            if !strings(record, "notApplicableAssetIds")
                .iter()
                .any(|id| id == text(asset, "id"))
            {
                blocked.push(json!({"assetId":asset["id"],"variantId":asset["variantId"],"reason":"未来视觉事实尚未获得该准确形象的当前制作范围裁决；只阻塞未覆盖形象"}));
            }
        }
    }
    blocked
}

fn registry_identity_subset(
    input: &Value,
    registry: &[Value],
    previous: &[Value],
) -> Result<Vec<Value>, String> {
    let registered = identity_cards(input, registry);
    let mut identities = Vec::new();
    let mut seen = HashSet::new();
    for original in previous {
        let id = text(original, "assetId");
        if !seen.insert(id) {
            return Err("旧视觉包主体目录重复".into());
        }
        let mut current = registered
            .iter()
            .find(|identity| text(identity, "assetId") == id)
            .ok_or("旧视觉包主体不在当前正式目录")?
            .clone();
        let mut versions = Vec::new();
        let mut slots = HashSet::new();
        for original_version in original["versions"]
            .as_array()
            .ok_or("旧视觉包形象目录缺失")?
        {
            let row_id = text(original_version, "assetRowId");
            if !slots.insert(row_id) {
                return Err("旧视觉包形象目录重复".into());
            }
            let version = current["versions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|version| {
                    text(version, "assetRowId") == row_id
                        && version["variantId"] == original_version["variantId"]
                })
                .ok_or("旧视觉包形象身份已变化")?;
            versions.push(version.clone());
        }
        current["versions"] = json!(versions);
        identities.push(current);
    }
    Ok(identities)
}

/// Early receipts retained the transport identities with short episode/scene
/// references. Recompile their exact packet before comparing to registry IDs;
/// short references are never accepted as domain identities on their own.
fn cached_identity_subset(
    input: &Value,
    registry: &[Value],
    previous: &[Value],
    source: &LockedSource,
    cache: &Value,
) -> Result<Vec<Value>, String> {
    let rebuilt = registry_identity_subset(input, registry, previous)?;
    if rebuilt == previous {
        return Ok(rebuilt);
    }
    let rows = cached_rows(source, cache)?;
    let wire = PacketWire::compile(source, &rebuilt, &rows);
    if wire.packet_hash == text(cache, "inputFingerprint")
        && wire.input["identities"] == json!(previous)
    {
        return Ok(rebuilt);
    }
    Err("旧视觉包身份无法按原锁定资料完整还原".into())
}

fn identity_selection_is_covered(selected: &[Value], packet: &[Value]) -> bool {
    selected.iter().all(|identity| {
        let Some(original) = packet
            .iter()
            .find(|original| original["assetId"] == identity["assetId"])
        else {
            return false;
        };
        ["name", "aliases", "kind"]
            .iter()
            .all(|key| original[*key] == identity[*key])
            && identity["versions"].as_array().is_some_and(|versions| {
                versions.iter().all(|version| {
                    original["versions"]
                        .as_array()
                        .is_some_and(|originals| originals.contains(version))
                })
            })
    })
}

fn cached_rows<'a>(source: &'a LockedSource, cache: &Value) -> Result<Vec<&'a SourceRow>, String> {
    let ids = strings(cache, "rowIds");
    let all: Vec<_> = source.rows.iter().map(|row| row.id.clone()).collect();
    if ids.is_empty()
        || ids.len() != cache["rowIds"].as_array().map(Vec::len).unwrap_or(0)
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
        || !ordered_subset(&ids, &all)
    {
        return Err("旧视觉包原行无效或顺序改变".into());
    }
    let by_id: BTreeMap<_, _> = source
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    Ok(ids.iter().map(|id| by_id[id.as_str()]).collect())
}

/// Rechecking unchanged production semantics may account for a prompt-only
/// asset version increment. The old decision/hash/receipt stays untouched.
fn validate_deferred_cache(
    cache: &Value,
    source: &LockedSource,
    wire: &PacketWire,
    identities: &[Value],
    registry: &[Value],
) -> Result<Vec<Value>, String> {
    let Some(records) = cache.get("deferredFacts") else {
        return Ok(Vec::new());
    };
    let records = records.as_array().ok_or("视觉事实不适用审计类型错误")?;
    let provenance = &cache["repairProvenance"];
    let constraints = provenance["acceptedProductionConstraints"]
        .as_array()
        .ok_or("视觉事实不适用审计缺少原正式制作取舍")?;
    validate_constraints(constraints, &source.sha256)?;
    if provenance["originalAccepted"] != false
        || provenance["version"] != VISUAL_FACT_REPAIR_PROFILE
        || provenance["productionConstraintsSha256"] != digest(&json!(constraints).to_string())
        || provenance["nodes"]
            .as_array()
            .is_none_or(|nodes| nodes.is_empty() || nodes.len() > 2)
    {
        return Err("视觉事实不适用裁决缺少原岗位正式采用证据".into());
    }
    let relevant: Vec<_> = identities
        .iter()
        .filter(|identity| {
            constraints
                .iter()
                .any(|constraint| constraint["subjectAssetId"] == identity["assetId"])
        })
        .cloned()
        .collect();
    let current = production_constraints(source, wire, &relevant, registry)?;
    let row_ids: HashSet<_> = wire.row_ids.iter().map(String::as_str).collect();
    let mut proofs = Vec::new();
    for record in records {
        let fact = &record["fact"];
        validate_fact_checked(fact, identities, &row_ids, false)?;
        if fact["sourceSha256"] != source.sha256
            || !fact["variantId"].is_null()
            || !["STATE", "TRANSITION"].contains(&text(fact, "scope"))
            || text(record, "reason").trim().is_empty()
        {
            return Err("本批不适用审计原事实或原范围发生变化".into());
        }
        let correction = json!({"variantId":null,"reason":record["reason"],"productionConstraintRefs":record["productionConstraintRefs"],"notApplicableAssetIds":record["notApplicableAssetIds"]});
        let index = record["factIndex"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("本批不适用原索引错误")?;
        if deferred_record(&correction, fact, index, constraints)? != *record {
            return Err("本批不适用审计与原裁决不一致".into());
        }
        for reference in strings(record, "productionConstraintRefs") {
            let original = constraints
                .iter()
                .find(|constraint| text(constraint, "constraintSha256") == reference)
                .unwrap();
            let now = current
                .iter()
                .find(|constraint| constraint["assetId"] == original["assetId"])
                .ok_or("本批不适用的正式制作取舍已缺失")?;
            let mut left = original.clone();
            let mut right = now.clone();
            for field in ["assetVersion", "constraintSha256"] {
                left.as_object_mut().unwrap().remove(field);
                right.as_object_mut().unwrap().remove(field);
            }
            if left != right {
                return Err("本批不适用的源/身份/正式制作取舍/范围已变化，原两次预算不重置".into());
            }
            if original["assetVersion"] != now["assetVersion"] {
                proofs.push(json!({"assetId":original["assetId"],"variantId":original["variantId"],"sourceSha256":source.sha256,"originalConstraintSha256":reference,"originalAssetVersion":original["assetVersion"],"currentAssetVersion":now["assetVersion"],"productionSemanticsEqual":true}));
            }
        }
    }
    Ok(proofs)
}

/// Queue preflight is independent of whether an image prompt already exists.
/// Cloud task ownership is deliberately not consulted: this only controls new
/// preparation; known tasks continue querying their original IDs elsewhere.
pub fn cached_visual_check_required(input: &Value) -> Result<bool, String> {
    Ok(!cached_visual_check_asset_ids(input)?.is_empty())
}

pub(crate) fn cached_visual_check_asset_ids(input: &Value) -> Result<Vec<String>, String> {
    let Some(assets) = input["assets"].as_array() else {
        return Ok(Vec::new());
    };
    let wanted = input
        .pointer("/selection/assetIds")
        .and_then(Value::as_array);
    let required: Vec<_> = assets
        .iter()
        .filter(|asset| {
            !["INLINE", "AUDIO"].contains(&text(&asset["preparation"], "mode"))
                && !["AUDIO", "audio"].contains(&text(asset, "kind"))
                && wanted
                    .is_none_or(|ids| ids.iter().any(|id| id.as_str() == Some(text(asset, "id"))))
        })
        .collect();
    let caches: Vec<_> = input
        .pointer("/foundation/visualFactBatches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|cache| {
            cache["unassignedFacts"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(cache["deferredFacts"].as_array().into_iter().flatten())
                .any(|record| {
                    required
                        .iter()
                        .any(|asset| subject(asset) == text(&record["fact"], "assetId"))
                })
        })
        .collect();
    if caches.is_empty() {
        return Ok(Vec::new());
    }
    let source: LockedSource = serde_json::from_value(input["source"].clone())
        .map_err(|_| "待核视觉状态缺少当前锁定原文")?;
    validate_source(&source)?;
    let registry = input["assetRegistry"].as_array().unwrap_or(assets);
    let current_rule = rule_snapshot("asset-visual-facts-extractor")?["sha256"]
        .as_str()
        .ok_or("视觉事实规则摘要缺失")?
        .to_string();
    let mut needed = HashSet::new();
    for cache in caches {
        let pending: Vec<_> = cache["unassignedFacts"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let deferred: Vec<_> = cache["deferredFacts"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        for asset in &required {
            if pending
                .iter()
                .any(|record| subject(asset) == text(&record["fact"], "assetId"))
            {
                needed.insert(text(asset, "id").to_string());
            }
        }
        if deferred.is_empty() {
            continue;
        }
        let validation = (|| -> Result<(), String> {
            let previous = cache
                .pointer("/repairProvenance/identityCards")
                .and_then(Value::as_array)
                .ok_or("不适用裁决缺少冻结身份目录")?;
            let identities = registry_identity_subset(input, registry, previous)?;
            if identities != *previous {
                return Err("不适用裁决身份或范围已变化".into());
            }
            let rows = cached_rows(&source, cache)?;
            let wire = PacketWire::compile(&source, &identities, &rows);
            validate_cached_packet(cache, &source, &identities, &wire.row_ids, &current_rule)?;
            validate_deferred_cache(cache, &source, &wire, &identities, registry)?;
            Ok(())
        })();
        for asset in &required {
            if deferred.iter().any(|record| {
                subject(asset) == text(&record["fact"], "assetId")
                    && (validation.is_err()
                        || !strings(record, "notApplicableAssetIds")
                            .iter()
                            .any(|id| id == text(asset, "id")))
            }) {
                needed.insert(text(asset, "id").to_string());
            }
        }
    }
    Ok(required
        .iter()
        .filter(|asset| needed.contains(text(asset, "id")))
        .map(|asset| text(asset, "id").to_string())
        .collect())
}

fn repaired_fact(fact: &Value, variant: &str, source_sha: &str) -> Value {
    let mut normalized = json!({"assetId":fact["assetId"],"variantId":variant,"attribute":fact["attribute"],"value":fact["value"],"scope":fact["scope"],"claim":fact["claim"],"sourceRefs":fact["sourceRefs"]});
    normalized["id"] = json!(format!("visual-{}", digest(&normalized.to_string())));
    normalized["sourceSha256"] = json!(source_sha);
    normalized
}

/// Delta application changes only the declared version slot. Non-visual and
/// still-unassigned records remain immutable evidence in their own partitions.
fn apply_visual_fact_corrections(
    output: &mut Value,
    original_hash: &str,
    base: &[Value],
    indices: &[usize],
    identities: &[Value],
    source_sha: &str,
    row_ids: &HashSet<&str>,
) -> Result<(), String> {
    apply_visual_fact_corrections_with_constraints(
        output,
        original_hash,
        base,
        indices,
        identities,
        source_sha,
        row_ids,
        &[],
    )
}

fn deferred_record(
    correction: &Value,
    fact: &Value,
    index: usize,
    constraints: &[Value],
) -> Result<Value, String> {
    let refs = strings(correction, "productionConstraintRefs");
    let ids = strings(correction, "notApplicableAssetIds");
    if correction["variantId"] != Value::Null
        || refs.is_empty()
        || ids.is_empty()
        || refs.len()
            != correction["productionConstraintRefs"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0)
        || ids.len()
            != correction["notApplicableAssetIds"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0)
        || refs.iter().collect::<HashSet<_>>().len() != refs.len()
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
    {
        return Err("本批不适用裁决缺少准确形象/制作取舍引用或隐式绑定版本".into());
    }
    let selected: Vec<_> = refs
        .iter()
        .map(|reference| {
            constraints
                .iter()
                .find(|constraint| text(constraint, "constraintSha256") == reference)
                .ok_or("本批不适用裁决引用未知制作取舍")
        })
        .collect::<Result<_, _>>()?;
    if selected.iter().any(|constraint| {
        text(constraint, "subjectAssetId") != text(fact, "assetId")
            || !ids.iter().any(|id| id == text(constraint, "assetId"))
    }) || ids.iter().any(|id| {
        !selected
            .iter()
            .any(|constraint| text(constraint, "assetId") == id)
    }) {
        return Err("本批不适用裁决越出原事实主体或未经正式取舍覆盖的形象".into());
    }
    Ok(
        json!({"fact":fact,"factIndex":index,"reason":correction["reason"],"disposition":"CURRENT_SCOPE_NOT_APPLICABLE","productionConstraintRefs":refs,"notApplicableAssetIds":ids,"productionConstraintsSha256":digest(&json!(constraints).to_string())}),
    )
}

fn apply_visual_fact_corrections_with_constraints(
    output: &mut Value,
    original_hash: &str,
    base: &[Value],
    indices: &[usize],
    identities: &[Value],
    source_sha: &str,
    row_ids: &HashSet<&str>,
    constraints: &[Value],
) -> Result<(), String> {
    validate_constraints(constraints, source_sha)?;
    if text(output, "schemaVersion") != VISUAL_FACT_REPAIR_PROFILE
        || text(output, "originalRawSha256") != original_hash
    {
        return Err("视觉事实局部修订运输回执不匹配".into());
    }
    let corrections = output["corrections"]
        .as_array()
        .ok_or("视觉事实局部修订缺少corrections")?
        .clone();
    let mut changed = BTreeMap::new();
    let mut excluded = Vec::new();
    let mut unassigned = Vec::new();
    let mut deferred = Vec::new();
    for correction in &corrections {
        let index = correction["factIndex"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|index| indices.contains(index))
            .ok_or("视觉事实局部修订越出原未绑定索引")?;
        if changed.contains_key(&index) || text(correction, "reason").trim().is_empty() {
            return Err("视觉事实局部修订索引重复或缺少原文依据".into());
        }
        let fact = &base[index];
        // Repeat all source and subject gates before applying a delta. A
        // correction cannot legitimize a polluted original extraction.
        validate_fact_checked(fact, identities, row_ids, false)?;
        let action = text(correction, "action");
        if !constraints.is_empty()
            && action != "CURRENT_SCOPE_NOT_APPLICABLE"
            && (correction["productionConstraintRefs"] != json!([])
                || correction["notApplicableAssetIds"] != json!([]))
        {
            return Err("其它视觉修订不能夹带本批不适用授权".into());
        }
        match action {
            "BIND_EXISTING_VERSION" => {
                let variant = correction["variantId"]
                    .as_str()
                    .filter(|variant| !variant.is_empty())
                    .ok_or("视觉事实局部绑定缺少已有版本")?;
                let updated = repaired_fact(fact, variant, source_sha);
                validate_fact(&updated, identities, row_ids)?;
                changed.insert(index, Some(updated));
            }
            "NOT_ASSET_VISUAL" | "UNRESOLVED" => {
                if correction.get("variantId") != Some(&Value::Null) {
                    return Err("未绑定视觉事实不能隐式设置版本".into());
                }
                let record = json!({"fact":fact,"factIndex":index,"reason":correction["reason"],"disposition":action});
                if action == "NOT_ASSET_VISUAL" {
                    excluded.push(record);
                } else {
                    unassigned.push(record);
                }
                changed.insert(index, None);
            }
            "CURRENT_SCOPE_NOT_APPLICABLE" if !constraints.is_empty() => {
                deferred.push(deferred_record(correction, fact, index, constraints)?);
                changed.insert(index, None);
            }
            _ => return Err("视觉事实局部修订操作未知".into()),
        }
    }
    if changed.len() != indices.len() {
        return Err("视觉事实局部修订未完整覆盖指定未绑定项".into());
    }
    let mut facts = Vec::new();
    for (index, fact) in base.iter().enumerate() {
        if let Some(replacement) = changed.get(&index) {
            if let Some(replacement) = replacement {
                facts.push(replacement.clone());
            }
        } else {
            validate_fact(fact, identities, row_ids)?;
            facts.push(fact.clone());
        }
    }
    let mut normalized = json!({"schemaVersion":VISUAL_FACT_REPAIR_PROFILE,"originalRawSha256":original_hash,"corrections":corrections,"facts":facts,"unassignedFacts":unassigned,"excludedFacts":excluded});
    if !constraints.is_empty() {
        normalized["deferredFacts"] = json!(deferred);
    }
    // Accepted checkpoints carry the derived partitions; never trust them when
    // replaying. Recompute and require equality before returning them again.
    for field in ["facts", "unassignedFacts", "excludedFacts", "deferredFacts"] {
        if output.get(field).is_some() && output[field] != normalized[field] {
            return Err("视觉事实局部修订checkpoint与原始delta不一致".into());
        }
    }
    *output = normalized;
    Ok(())
}

fn build_repair_request(
    original: &TextRequest,
    wire: &PacketWire,
    base: &[Value],
    identities: &[Value],
    constraints: &[Value],
    original_raw: &Value,
    attempt: u64,
    legacy_upgrade: bool,
) -> TextRequest {
    let indices = unbound_indices(base);
    let original_hash = digest(&original_raw.to_string());
    let variants: Vec<_> = identities
        .iter()
        .flat_map(|identity| identity["versions"].as_array().into_iter().flatten())
        .map(|version| version["variantId"].clone())
        .chain(std::iter::once(Value::Null))
        .collect();
    let mut input = wire.input.clone();
    input["visualFactRepair"] = json!({"version":VISUAL_FACT_REPAIR_PROFILE,"sourceSha256":wire.input["sourceSha256"],"packetFingerprint":wire.packet_hash,"originalDomainRequestSha256":digest(&serde_json::to_string(original).unwrap()),"originalRawSha256":original_hash,"baseFactsSha256":digest(&json!(base).to_string()),"attempt":attempt,"maxAttempts":2});
    if !constraints.is_empty() {
        input["visualFactRepair"]["acceptedProductionConstraints"] = json!(constraints);
        input["visualFactRepair"]["productionConstraintsSha256"] =
            json!(digest(&json!(constraints).to_string()));
    }
    if legacy_upgrade {
        input["visualFactRepair"]["legacyProductionConstraintUpgrade"] = json!(true);
    }
    input["rejectedVisualFacts"] = original_raw.clone();
    input["unassignedVisualFacts"] = json!(indices
        .iter()
        .map(|index| json!({"factIndex":index,"fact":base[*index]}))
        .collect::<Vec<_>>());
    let mut correction = json!({"factIndex":{"type":"integer","enum":indices},"action":choices(&["BIND_EXISTING_VERSION","NOT_ASSET_VISUAL","UNRESOLVED"]),"variantId":{"type":["string","null"],"enum":variants},"reason":string()});
    if !constraints.is_empty() {
        correction["action"] = choices(&[
            "BIND_EXISTING_VERSION",
            "NOT_ASSET_VISUAL",
            "UNRESOLVED",
            "CURRENT_SCOPE_NOT_APPLICABLE",
        ]);
        correction["productionConstraintRefs"] = array(
            json!({"type":"string","enum":constraints.iter().map(|constraint|constraint["constraintSha256"].clone()).collect::<Vec<_>>()}),
        );
        correction["notApplicableAssetIds"] = array(
            json!({"type":"string","enum":constraints.iter().map(|constraint|constraint["assetId"].clone()).collect::<Vec<_>>()}),
        );
    }
    let schema = obj(
        json!({"schemaVersion":choices(&[VISUAL_FACT_REPAIR_PROFILE]),"originalRawSha256":{"type":"string","enum":[original_hash]},"corrections":array(obj(correction))}),
    );
    let content = repair_rule(constraints);
    let mut rule_snapshot = original.rule_snapshot.clone();
    rule_snapshot["runtimeContract"] =
        json!({"version":VISUAL_FACT_REPAIR_PROFILE,"content":content,"sha256":digest(&content)});
    TextRequest {
        stage: original.stage.clone(),
        skill: original.skill.clone(),
        model: original.model.clone(),
        reasoning: original.reasoning.clone(),
        input,
        schema,
        rule_snapshot,
    }
}

async fn repair_unbound_packet(
    wire: &PacketWire,
    original_raw: &Value,
    probe: &Value,
    identities: &[Value],
    source: &LockedSource,
    registry: &[Value],
    continuation: Option<(Value, Vec<Value>)>,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    let source_sha = source.sha256.as_str();
    let original_hash = digest(&original_raw.to_string());
    let original_request = TextRequest {
        stage: "assetVisualFacts".into(),
        skill: "asset-visual-facts-extractor".into(),
        model: model.into(),
        reasoning: reasoning.into(),
        input: wire.input.clone(),
        schema: wire.schema(identities),
        rule_snapshot: snapshot("asset-visual-facts-extractor", &[])?,
    };
    let domain_hash = digest(&serde_json::to_string(&original_request).unwrap());
    let row_ids: HashSet<_> = wire.row_ids.iter().map(String::as_str).collect();
    let mut base = probe["facts"]
        .as_array()
        .ok_or("视觉事实局部修订缺少合法源包")?
        .clone();
    let mut nodes = Vec::new();
    let mut excluded = Vec::new();
    let mut unresolved = Vec::new();
    let mut deferred = Vec::new();
    let relevant: Vec<_> = identities
        .iter()
        .filter(|identity| {
            base.iter().any(|fact| {
                fact["assetId"] == identity["assetId"]
                    && ["STATE", "TRANSITION"].contains(&text(fact, "scope"))
                    && fact["variantId"].is_null()
            })
        })
        .cloned()
        .collect();
    let constraints = production_constraints(source, wire, &relevant, registry)?;
    let legacy_upgrade = continuation.is_some();
    if let Some((accepted, previous_nodes)) = continuation {
        if constraints.is_empty() {
            return Err("原局部修订仍未获得新的正式制作取舍，不重发".into());
        }
        base = accepted["facts"]
            .as_array()
            .ok_or("旧修订缺少已验证事实")?
            .clone();
        unresolved = accepted["unassignedFacts"]
            .as_array()
            .ok_or("旧修订缺少未绑定审计")?
            .clone();
        base.extend(unresolved.iter().map(|record| record["fact"].clone()));
        excluded = accepted["excludedFacts"]
            .as_array()
            .ok_or("旧修订缺少排除审计")?
            .clone();
        nodes = previous_nodes;
    }
    for attempt in if legacy_upgrade { 2..=2 } else { 1..=2 } {
        let indices = unbound_indices(&base);
        if indices.is_empty() {
            break;
        }
        let request = build_repair_request(
            &original_request,
            wire,
            &base,
            identities,
            &constraints,
            original_raw,
            attempt,
            legacy_upgrade,
        );
        let mut output = executor.execute(request.clone()).await?;
        let raw = output.clone();
        validate_visual_fact_repair_request(&request)?;
        if let Err(reason) = apply_visual_fact_corrections_with_constraints(
            &mut output,
            &original_hash,
            &base,
            &indices,
            identities,
            source_sha,
            &row_ids,
            &constraints,
        ) {
            executor.reject(&request, &raw, &reason).await?;
            return Err(reason);
        }
        executor.accept(&request, &output).await?;
        let delta = json!({"schemaVersion":raw["schemaVersion"],"originalRawSha256":raw["originalRawSha256"],"corrections":raw["corrections"]});
        nodes.push(json!({"attempt":attempt,"domainRequestSha256":digest(&serde_json::to_string(&request).unwrap()),"rawDeltaSha256":digest(&delta.to_string())}));
        excluded.extend(output["excludedFacts"].as_array().unwrap().iter().cloned());
        deferred.extend(
            output["deferredFacts"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
        unresolved = output["unassignedFacts"].as_array().unwrap().clone();
        let remaining = unresolved.len();
        let resolved = indices.len() - remaining;
        base = output["facts"].as_array().unwrap().clone();
        // Only real progress permits a second same-role call. Its input and
        // attempt identity are deterministic and cannot reset on recovery.
        if remaining == 0 || resolved == 0 || attempt == 2 {
            break;
        }
        base.extend(unresolved.iter().map(|record| record["fact"].clone()));
    }
    let mut repaired = json!({"facts":base,"unassignedFacts":unresolved,"excludedFacts":excluded,"repairProvenance":{"version":VISUAL_FACT_REPAIR_PROFILE,"originalRawSha256":original_hash,"originalDomainRequestSha256":domain_hash,"originalAccepted":false,"nodes":nodes}});
    if !constraints.is_empty() {
        repaired["deferredFacts"] = json!(deferred);
        repaired["repairProvenance"]["acceptedProductionConstraints"] = json!(constraints);
        repaired["repairProvenance"]["productionConstraintsSha256"] =
            json!(digest(&json!(constraints).to_string()));
        repaired["repairProvenance"]["identityCards"] = json!(identities);
    }
    Ok(repaired)
}

/// A previously accepted unresolved correction may consume its second and last
/// attempt only when it demonstrably lacked the now-present adopted choice.
async fn continue_cached_production_choice(
    input: &Value,
    source: &LockedSource,
    cache: &Value,
    registry: &[Value],
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Option<Value>, String> {
    let provenance = &cache["repairProvenance"];
    let pending = cache["unassignedFacts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let Some(nodes) = provenance["nodes"]
        .as_array()
        .filter(|nodes| nodes.len() == 1 && nodes[0]["attempt"] == 1)
    else {
        return Ok(None);
    };
    if pending.is_empty()
        || provenance.get("acceptedProductionConstraints").is_some()
        || provenance["version"] != VISUAL_FACT_REPAIR_PROFILE
        || provenance["originalAccepted"] != false
    {
        return Ok(None);
    }
    if !input["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|asset| {
            pending
                .iter()
                .any(|record| record["fact"]["assetId"] == subject(asset))
                && (text(asset, "prompt").trim().is_empty()
                    || input
                        .pointer("/selection/assetIds")
                        .and_then(Value::as_array)
                        .is_none_or(|ids| {
                            ids.iter().any(|id| id.as_str() == Some(text(asset, "id")))
                        }))
        })
    {
        return Ok(None);
    }
    let Some(context) = executor
        .visual_fact_repair_context(
            &source.sha256,
            text(cache, "inputFingerprint"),
            text(provenance, "originalDomainRequestSha256"),
            text(provenance, "originalRawSha256"),
        )
        .await?
    else {
        return Ok(None);
    };
    let original: TextRequest = serde_json::from_value(context["originalRequest"].clone())
        .map_err(|_| "旧视觉修订缺少完整原请求")?;
    let accepted_request: TextRequest =
        serde_json::from_value(context["acceptedRepairRequest"].clone())
            .map_err(|_| "旧视觉修订缺少完整已采用局部请求")?;
    let original_raw = &context["originalRaw"];
    if original.stage != "assetVisualFacts"
        || original.skill != "asset-visual-facts-extractor"
        || original.model != model
        || original.reasoning != reasoning
        || original.rule_snapshot != snapshot("asset-visual-facts-extractor", &[])?
        || digest(&serde_json::to_string(&original).unwrap())
            != text(provenance, "originalDomainRequestSha256")
        || digest(&original_raw.to_string()) != text(provenance, "originalRawSha256")
    {
        return Err("旧视觉修订原源/规则/模型请求或原回包绑定改变，未续发".into());
    }
    let identities = registry_identity_subset(
        input,
        registry,
        original.input["identities"]
            .as_array()
            .ok_or("旧视觉修订缺少原主体目录")?,
    )?;
    let rows = cached_rows(source, cache)?;
    let wire = PacketWire::compile(source, &identities, &rows);
    if wire.input != original.input
        || wire.schema(&identities) != original.schema
        || wire.packet_hash != text(cache, "inputFingerprint")
    {
        return Err("旧视觉修订原包无法由当前锁定来源/正式身份完整还原，未续发".into());
    }
    let row_ids = wire.row_ids.clone();
    let rule_sha = rule_snapshot("asset-visual-facts-extractor")?["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    validate_cached_packet(cache, source, &identities, &row_ids, &rule_sha)?;
    let mut probe = original_raw.clone();
    wire.materialize_checked(&mut probe, &identities, &source.sha256, false)?;
    let base = probe["facts"].as_array().unwrap();
    let expected = build_repair_request(
        &original,
        &wire,
        base,
        &identities,
        &[],
        original_raw,
        1,
        false,
    );
    if serde_json::to_value(&expected).unwrap() != serde_json::to_value(&accepted_request).unwrap()
        || accepted_request.input["visualFactRepair"]
            .get("acceptedProductionConstraints")
            .is_some()
    {
        return Err("旧局部修订请求并非相同原包的缺失制作上下文首轮，未续发".into());
    }
    validate_visual_fact_repair_request(&accepted_request)?;
    let mut accepted = context["acceptedRepair"].clone();
    let untouched = accepted.clone();
    let known = wire.row_ids.iter().map(String::as_str).collect();
    apply_visual_fact_corrections(
        &mut accepted,
        text(provenance, "originalRawSha256"),
        base,
        &unbound_indices(base),
        &identities,
        &source.sha256,
        &known,
    )?;
    let delta = json!({"schemaVersion":accepted["schemaVersion"],"originalRawSha256":accepted["originalRawSha256"],"corrections":accepted["corrections"]});
    if accepted != untouched
        || ["facts", "unassignedFacts", "excludedFacts"]
            .iter()
            .any(|key| accepted[*key] != cache[*key])
        || accepted["unassignedFacts"]
            .as_array()
            .is_none_or(Vec::is_empty)
        || nodes[0]["domainRequestSha256"]
            != digest(&serde_json::to_string(&accepted_request).unwrap())
        || nodes[0]["rawDeltaSha256"] != digest(&delta.to_string())
    {
        return Err("旧已采用修订无法从原delta完整再验，未续发".into());
    }
    let relevant: Vec<_> = identities
        .iter()
        .filter(|identity| {
            pending
                .iter()
                .any(|record| record["fact"]["assetId"] == identity["assetId"])
        })
        .cloned()
        .collect();
    let constraints = production_constraints(source, &wire, &relevant, registry)?;
    if !constraints.iter().any(|constraint| {
        pending
            .iter()
            .any(|record| record["fact"]["assetId"] == constraint["subjectAssetId"])
    }) {
        return Ok(None);
    }
    let repaired = repair_unbound_packet(
        &wire,
        original_raw,
        &probe,
        &identities,
        source,
        registry,
        Some((accepted, nodes.clone())),
        executor,
        model,
        reasoning,
    )
    .await?;
    let mut updated = cache.clone();
    for field in [
        "facts",
        "unassignedFacts",
        "excludedFacts",
        "deferredFacts",
        "repairProvenance",
    ] {
        if let Some(value) = repaired.get(field) {
            updated[field] = value.clone();
        }
    }
    updated["repairProvenance"]["productionConstraintUpgrade"] = json!({"originInputFingerprint":cache["inputFingerprint"],"originalAcceptedCacheSha256":digest(&cache.to_string()),"originJobId":context["originJobId"],"acceptedFirstRepairJobId":context["acceptedRepairJobId"],"originalPacketRebuilt":true,"originalRawFullyValidated":true,"acceptedFirstDeltaRevalidated":true,"additionalAttempt":2,"maxAttempts":2});
    Ok(Some(updated))
}

/// Recover only the source binding dropped by the old typed persistence step.
/// The original adopted packet must still verify against current source, rules
/// and asset identities, and every other fact field must match exactly.
async fn restore_persisted_fact_source_bindings(
    input: &Value,
    source: &LockedSource,
    registry: &[Value],
    executor: &dyn TextExecutor,
    facts: &mut [Value],
) -> Result<(), String> {
    if input.pointer("/foundation/sourceSha256") != Some(&json!(source.sha256))
        || !facts.iter().any(|fact| fact.get("sourceSha256").is_none()) {
        return Ok(());
    }
    let rule = rule_snapshot("asset-visual-facts-extractor")?["sha256"]
        .as_str().ok_or("视觉事实规则缺少SHA")?.to_owned();
    for cache in input.pointer("/foundation/visualFactBatches")
        .and_then(Value::as_array).into_iter().flatten() {
        let previous = if let Some(cards) = cache.get("identityCards")
            .or_else(|| cache.pointer("/repairProvenance/identityCards")).and_then(Value::as_array) {
            Some(cards.clone())
        } else {
            executor.visual_fact_packet_identities(&source.sha256, text(cache, "inputFingerprint")).await?
        };
        let Some(previous) = previous else { continue; };
        let Ok(identities) = cached_identity_subset(input, registry, &previous, source, cache) else { continue; };
        let Ok(rows) = cached_rows(source, cache) else { continue; };
        let ids: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
        let Ok(bound) = validate_cached_packet(cache, source, &identities, &ids, &rule) else { continue; };
        for fact in facts.iter_mut().filter(|fact| fact.get("sourceSha256").is_none()) {
            let mut persisted = fact.clone();
            if let Some(object) = persisted.as_object_mut() { object.remove("quotes"); }
            if bound.iter().any(|original| {
                let mut candidate = original.clone();
                if let Some(object) = candidate.as_object_mut() {
                    object.remove("quotes");
                    object.remove("sourceSha256");
                }
                candidate == persisted
            }) {
                fact["sourceSha256"] = json!(source.sha256);
            }
        }
    }
    Ok(())
}

pub async fn run_asset_visual_facts(
    input: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<Value, String> {
    let source: LockedSource =
        serde_json::from_value(input.get("source").cloned().ok_or("视觉事实缺少锁定原文")?)
            .map_err(|e| e.to_string())?;
    validate_source(&source)?;
    let assets = input["assets"]
        .as_array()
        .ok_or("视觉事实缺少明确资产范围")?;
    let identities = identity_cards(input, assets);
    let registry = input["assetRegistry"].as_array().unwrap_or(assets);
    let known: HashSet<_> = source.rows.iter().map(|row| row.id.as_str()).collect();
    let mut facts = input
        .pointer("/foundation/facts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    restore_persisted_fact_source_bindings(input, &source, registry, executor, &mut facts).await?;
    let mut blocked = Vec::new();
    let mut missing = Vec::new();
    for asset in assets {
        let explicit = input
            .pointer("/selection/assetIds")
            .and_then(Value::as_array)
            .map(|ids| ids.iter().any(|id| id.as_str() == Some(text(asset, "id"))))
            .unwrap_or(true);
        if !explicit && !text(asset, "prompt").trim().is_empty() {
            continue;
        }
        let relevant: Vec<_> = facts
            .iter()
            .filter(|fact| {
                text(fact, "assetId") == subject(asset)
                    && (text(fact, "variantId").is_empty()
                        || text(fact, "variantId") == text(asset, "variantId"))
            })
            .collect();
        if let Some(reason) = relevant
            .iter()
            .find_map(|fact| validate_fact(fact, &identities, &known).err())
        {
            blocked.push(
                json!({"assetId":asset["id"],"variantId":asset["variantId"],"reason":reason}),
            );
            continue;
        }
        let derived =
            !text(asset, "sourceAssetId").is_empty() || !text(asset, "sourceVariantId").is_empty();
        let current_fact = relevant.iter().any(|fact| {
            text(fact, "variantId") == text(asset, "variantId")
                || !derived && text(fact, "scope") == "STABLE" && text(fact, "variantId").is_empty()
        });
        if input
            .pointer("/foundation/sourceSha256")
            .and_then(Value::as_str)
            == Some(&source.sha256)
            && current_fact
        {
            continue;
        }
        missing.push(asset);
    }
    // Stable appearance evidence cannot erase an unresolved future-state audit.
    // Revisit only the actually requested shapes of that exact identity.
    for cache in input
        .pointer("/foundation/visualFactBatches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for record in cache["unassignedFacts"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(cache["deferredFacts"].as_array().into_iter().flatten())
        {
            for asset in assets
                .iter()
                .filter(|asset| subject(asset) == text(&record["fact"], "assetId"))
            {
                let selected = input
                    .pointer("/selection/assetIds")
                    .and_then(Value::as_array)
                    .is_none_or(|ids| ids.iter().any(|id| id.as_str() == Some(text(asset, "id"))));
                if (selected || text(asset, "prompt").trim().is_empty())
                    && !missing
                        .iter()
                        .any(|candidate| candidate["id"] == asset["id"])
                {
                    missing.push(asset);
                }
            }
        }
    }
    if missing.is_empty() {
        return Ok(json!({"facts":facts,"blocked":blocked,"visualFactBatches":[]}));
    }
    let explicit_rows: HashSet<_> = missing
        .iter()
        .flat_map(|asset| strings(asset, "sourceRefs"))
        .collect();
    let episodes: HashSet<String> = missing
        .iter()
        .flat_map(|asset| {
            asset
                .pointer("/applicability/episodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
        })
        .chain(
            source
                .rows
                .iter()
                .filter(|row| explicit_rows.contains(&row.id))
                .filter_map(|row| row.episode_id.clone()),
        )
        .collect();
    let rows: Vec<_> = source
        .rows
        .iter()
        .filter(|row| {
            explicit_rows.contains(&row.id)
                || row
                    .episode_id
                    .as_ref()
                    .map(|id| episodes.contains(id))
                    .unwrap_or(row.kind == "context")
        })
        .collect();
    if rows.is_empty() {
        return Ok(json!({"facts":facts,"blocked":blocked,"visualFactBatches":[]}));
    }
    let mut groups: Vec<Vec<&SourceRow>> = Vec::new();
    let mut current = Vec::new();
    let mut chars = 0;
    let mut author = false;
    for row in rows {
        let size = row.text.encode_utf16().count() + 1;
        let is_author = row.episode_id.is_none();
        if !current.is_empty() && (chars + size > CAPACITY || author != is_author) {
            groups.push(std::mem::take(&mut current));
            chars = 0;
        }
        author = is_author;
        chars += size;
        current.push(row);
    }
    if !current.is_empty() {
        groups.push(current)
    }
    let mut receipts = Vec::new();
    let mut cache_reuses = Vec::new();
    let current_rule_sha = rule_snapshot("asset-visual-facts-extractor")?["sha256"]
        .as_str()
        .ok_or("视觉事实规则缺少SHA")?
        .to_string();
    for (index, rows) in groups.iter().enumerate() {
        let ids: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
        let packet = canonical_packet(&source, &identities, rows);
        let packet_hash = digest(&serde_json::to_string(&packet).unwrap());
        let caches = input
            .pointer("/foundation/visualFactBatches")
            .and_then(Value::as_array);
        let exact = caches
            .into_iter()
            .flatten()
            .find(|cache| text(cache, "inputFingerprint") == packet_hash);
        let mut supersets:Vec<_> = caches
            .into_iter()
            .flatten()
            .filter(|cache| {
                let Some(row_ids) = cache["rowIds"].as_array().and_then(|rows| {
                    rows.iter()
                        .map(|row| row.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                }) else {
                    return false;
                };
                row_ids.len() >= ids.len() && ordered_subset(&ids, &row_ids)
            })
            .collect();
        supersets.sort_by_key(|cache| {
                cache["rowIds"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(usize::MAX)
            });
        let mut superset=None;
        for candidate in supersets {
            // A matching reading range is not a matching asset selection. A
            // proven packet for different subjects must not block new assets.
            let previous=if let Some(cards)=candidate.get("identityCards").or_else(||candidate.pointer("/repairProvenance/identityCards")).and_then(Value::as_array){Some(cards.clone())}else{executor.visual_fact_packet_identities(&source.sha256,text(candidate,"inputFingerprint")).await?};
            if let Some(previous)=previous {
                if let Ok(rebuilt)=cached_identity_subset(input,registry,&previous,&source,candidate) {
                    if validate_cached_packet(candidate,&source,&rebuilt,&ids,&current_rule_sha).is_ok()
                        && !identity_selection_is_covered(&identities,&rebuilt) {continue;}
                }
            }
            superset=Some(candidate);break;
        }
        if let Some(cache) = exact.or(superset) {
            let mut adopted = cache.clone();
            let mut continuation_error = None;
            match continue_cached_production_choice(
                input, &source, cache, registry, executor, model, reasoning,
            )
            .await
            {
                Ok(Some(updated)) => adopted = updated,
                Ok(None) => {}
                Err(reason) => continuation_error = Some(reason),
            }
            let cache = &adopted;
            let full_registry = identity_cards(input, registry);
            let mut packet_identities = identities.clone();
            let mut validation =
                validate_cached_packet(cache, &source, &packet_identities, &ids, &current_rule_sha)
                    .map(|_| ());
            if validation.is_err()
                && identity_selection_is_covered(&identities, &full_registry)
                && validate_cached_packet(cache, &source, &full_registry, &ids, &current_rule_sha)
                    .is_ok()
            {
                packet_identities = full_registry;
                validation = Ok(());
            }
            if validation.is_err() {
                let previous=if let Some(cards)=cache.get("identityCards").or_else(||cache.pointer("/repairProvenance/identityCards")).and_then(Value::as_array){Some(cards.clone())}else{executor.visual_fact_packet_identities(&source.sha256,text(cache,"inputFingerprint")).await?};
                if let Some(previous) = previous.as_ref() {
                    if let Ok(rebuilt) = cached_identity_subset(input, registry, previous, &source, cache) {
                        if identity_selection_is_covered(&identities, &rebuilt)
                            && validate_cached_packet(
                                cache,
                                &source,
                                &rebuilt,
                                &ids,
                                &current_rule_sha,
                            )
                            .is_ok()
                        {
                            packet_identities = rebuilt;
                            validation = Ok(());
                        }
                    }
                }
            }
            let deferred_proofs = if validation.is_ok() {
                let old_rows = cached_rows(&source, cache)?;
                let old_wire = PacketWire::compile(&source, &packet_identities, &old_rows);
                validate_deferred_cache(cache, &source, &old_wire, &packet_identities, registry)
            } else {
                Ok(Vec::new())
            };
            match validation {
                Ok(()) => {
                    let cached = cache["facts"].as_array().unwrap();
                    facts.extend(cached.iter().cloned());
                    let pending: Vec<_> = cache["unassignedFacts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|record| record["fact"].clone())
                        .collect();
                    blocked.extend(blocked_subjects(
                        &missing,
                        &pending,
                        continuation_error.as_deref().unwrap_or(
                            "视觉状态仍无可证的已登记适用形象；原事实保留，仅阻塞对应主体与派生",
                        ),
                    ));
                    match deferred_proofs {
                        Ok(proofs) => {
                            blocked.extend(deferred_blocked_subjects(&missing, cache));
                            for proof in proofs {
                                cache_reuses.push(json!({"kind":"production-choice-version-revalidation","proof":proof,"originalInputFingerprint":cache["inputFingerprint"]}));
                            }
                        }
                        Err(reason) => {
                            let pending = cache["deferredFacts"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|record| record["fact"].clone())
                                .collect::<Vec<_>>();
                            blocked.extend(blocked_subjects(&missing, &pending, &reason));
                        }
                    }
                    // Preserve the original accepted packet and receipt. This
                    // separate program proof is not a model profile or new adoption.
                    if text(cache, "inputFingerprint") != packet_hash {
                        cache_reuses.push(json!({"originalInputFingerprint":cache["inputFingerprint"],"requestedInputFingerprint":packet_hash,"sourceSha256":source.sha256,"ruleSha256":current_rule_sha,"identitySha256":digest(&json!(packet_identities).to_string()),"originalRowCount":cache["rowIds"].as_array().unwrap().len(),"requestedRowIds":ids,"requestedRowCount":ids.len(),"orderedSubsetVerified":true,"originalPacketRebuilt":true,"factsFullyValidated":true}));
                    }
                    receipts.push(cache.clone());
                }
                Err(reason) => blocked.extend(affected(
                    &source,
                    &missing,
                    rows,
                    &format!("旧缓存无效：{reason}，未重发提取"),
                )),
            }
            continue;
        }
        let wire = PacketWire::compile(&source, &identities, rows);
        debug_assert_eq!(wire.packet_hash, packet_hash);
        let schema = wire.schema(&identities);
        let mut rejected_raw = None;
        let result = call_validated(
            executor,
            "assetVisualFacts",
            "asset-visual-facts-extractor",
            model,
            reasoning,
            wire.input.clone(),
            schema,
            &[],
            |output| {
                let original = output.clone();
                let result = wire.materialize(output, &identities, &source.sha256);
                if result.is_err() {
                    rejected_raw = Some(original);
                }
                result
            },
        )
        .await;
        match result {
            Ok(result) => {
                let found = result["facts"].as_array().unwrap();
                facts.extend(found.iter().cloned());
                receipts.push(json!({"schemaVersion":"asset-visual-facts-cache/v1","status":"accepted","sourceSha256":source.sha256,"inputFingerprint":packet_hash,"ruleSha256":rule_snapshot("asset-visual-facts-extractor")?["sha256"],"rowIds":ids,"identityCards":identities,"facts":found,"batchIndex":index,"referenceEncoding":WIRE_PROFILE,"coveredPacketRef":packet_hash}));
            }
            Err(reason) if reason == VISUAL_FACT_UNBOUND_ERROR && rejected_raw.is_some() => {
                let original = rejected_raw.unwrap();
                let mut probe = original.clone();
                if let Err(hard_error) =
                    wire.materialize_checked(&mut probe, &identities, &source.sha256, false)
                {
                    blocked.extend(affected(&source, &missing, rows, &hard_error));
                    continue;
                }
                let unassigned: Vec<_> = probe["facts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|fact| {
                        ["STATE", "TRANSITION"].contains(&text(fact, "scope"))
                            && fact["variantId"].is_null()
                    })
                    .cloned()
                    .collect();
                match repair_unbound_packet(
                    &wire,
                    &original,
                    &probe,
                    &identities,
                    &source,
                    registry,
                    None,
                    executor,
                    model,
                    reasoning,
                )
                .await
                {
                    Ok(repaired) => {
                        let found = repaired["facts"].as_array().unwrap();
                        let pending: Vec<_> = repaired["unassignedFacts"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|record| record["fact"].clone())
                            .collect();
                        blocked.extend(blocked_subjects(
                            &missing,
                            &pending,
                            "视觉状态仍无可证的已登记适用形象；原事实保留，仅阻塞对应主体与派生",
                        ));
                        blocked.extend(deferred_blocked_subjects(&missing, &repaired));
                        facts.extend(found.iter().cloned());
                        let mut cache = json!({"schemaVersion":"asset-visual-facts-cache/v1","status":"accepted","sourceSha256":source.sha256,"inputFingerprint":packet_hash,"ruleSha256":rule_snapshot("asset-visual-facts-extractor")?["sha256"],"rowIds":ids,"facts":found,"unassignedFacts":repaired["unassignedFacts"],"excludedFacts":repaired["excludedFacts"],"repairProvenance":repaired["repairProvenance"],"batchIndex":index,"referenceEncoding":WIRE_PROFILE,"coveredPacketRef":packet_hash});
                        if let Some(deferred) = repaired.get("deferredFacts") {
                            cache["deferredFacts"] = deferred.clone();
                        }
                        receipts.push(cache);
                    }
                    Err(repair_error) => {
                        // All other facts passed the full source/identity gates.
                        // Preserve them without declaring the rejected packet
                        // accepted; failure is routed by exact subject, not by
                        // the broad AUTHOR reading range.
                        facts.extend(
                            probe["facts"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .filter(|fact| {
                                    !unassigned.iter().any(|pending| pending["id"] == fact["id"])
                                })
                                .cloned(),
                        );
                        blocked.extend(blocked_subjects(
                            &missing,
                            &unassigned,
                            &format!("视觉事实局部修订未完成：{repair_error}"),
                        ));
                    }
                }
            }
            Err(reason) => {
                blocked.extend(affected(&source, &missing, rows, &reason));
            }
        }
    }
    let mut signatures = HashSet::new();
    facts.retain(|fact| signatures.insert(digest(&serde_json::to_string(&json!({"assetId":fact["assetId"],"variantId":fact["variantId"],"attribute":fact["attribute"],"value":fact["value"],"scope":fact["scope"],"claim":fact["claim"],"sourceRefs":fact["sourceRefs"]})).unwrap())));
    Ok(
        json!({"facts":facts,"blocked":blocked,"visualFactBatches":receipts,"visualFactCacheReuses":cache_reuses}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Extractor {
        mode: &'static str,
        calls: Mutex<Vec<Value>>,
        accepted: Mutex<Vec<Value>>,
        rejected: Mutex<Vec<String>>,
        rejected_values: Mutex<Vec<Value>>,
        requests: Mutex<Vec<TextRequest>>,
    }
    impl Extractor {
        fn new(mode: &'static str) -> Self {
            Self {
                mode,
                calls: Mutex::new(Vec::new()),
                accepted: Mutex::new(Vec::new()),
                rejected: Mutex::new(Vec::new()),
                rejected_values: Mutex::new(Vec::new()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }
    #[async_trait]
    impl TextExecutor for Extractor {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            assert_eq!(request.stage, "assetVisualFacts");
            assert_eq!(request.skill, "asset-visual-facts-extractor");
            self.calls.lock().unwrap().push(request.input.clone());
            self.requests.lock().unwrap().push(request.clone());
            if is_visual_fact_repair(&request) {
                assert!(request.rule_snapshot["runtimeContract"]["content"]
                    .as_str()
                    .unwrap()
                    .contains("不把后续变化强塞母形象"));
                assert_eq!(request.input["visualFactRepair"]["maxAttempts"], 2);
                let pending = request.input["unassignedVisualFacts"].as_array().unwrap();
                let corrections:Vec<_>=pending.iter().map(|item| {
                    if self.mode=="repair-defer" || self.mode=="repair-defer-foreign" {
                        return choice_correction(&request,item,self.mode=="repair-defer-foreign");
                    }
                    let identity=request.input["identities"].as_array().unwrap().iter().find(|identity|identity["assetId"]==item["fact"]["assetId"]).unwrap();
                    let action=match self.mode {"repair-unresolved"=>"UNRESOLVED","repair-nonvisual"=>"NOT_ASSET_VISUAL","repair-progress" if request.input["visualFactRepair"]["attempt"]==1 && item["factIndex"]!=0=>"UNRESOLVED",_=>"BIND_EXISTING_VERSION"};
                    json!({"factIndex":if self.mode=="repair-foreign-index"{json!(999)}else{item["factIndex"].clone()},"action":action,"variantId":if action!="BIND_EXISTING_VERSION" {Value::Null} else if self.mode=="repair-wrong-version" {json!("not-owned")} else {identity["versions"][0]["variantId"].clone()},"reason":"依据本包原行按原A15方法判断；不创建新身份。"})
                }).collect();
                return Ok(
                    json!({"schemaVersion":VISUAL_FACT_REPAIR_PROFILE,"originalRawSha256":request.input["visualFactRepair"]["originalRawSha256"],"corrections":corrections}),
                );
            }
            let rows = request.input["rows"].as_array().unwrap();
            if self.mode == "partial"
                && rows
                    .iter()
                    .any(|row| text(row, "text").contains("失败标记"))
            {
                return Err("模拟首包失败".into());
            }
            let identity = &request.input["identities"][0];
            let row = rows
                .iter()
                .find(|row| text(row, "kind") != "format")
                .unwrap_or(&rows[0]);
            let fact = json!({"assetId":if self.mode=="owner"{"unknown"}else{text(identity,"assetId")},"variantId":if self.mode=="variant"{json!("not-owned")}else{Value::Null},"attribute":"hair.color","value":"黑发","scope":"STABLE","claim":"SOURCE","sourceRefs":if self.mode=="source"{json!(["foreign-row"])}else{json!([row["id"]])},"quotes":["模型多余重抄不得保留"]});
            if self.mode.starts_with("repair-") {
                let other = &request.input["identities"][1];
                let mut fact = fact;
                if self.mode == "repair-progress" {
                    fact["scope"] = json!("STATE");
                }
                let mut unbound = json!({"assetId":other["assetId"],"variantId":null,"attribute":"hair.style","value":"本行局部发型变化","scope":"TRANSITION","claim":"SOURCE","sourceRefs":[row["id"]]});
                if text(row, "text").contains("后期剪清爽短发") {
                    unbound["value"] = json!("后期剪清爽短发");
                }
                if self.mode == "repair-hard-source" {
                    unbound["sourceRefs"] = json!(["foreign-row"]);
                }
                return Ok(
                    json!({"schemaVersion":WIRE_RESULT,"coveredPacketRef":request.input["coveredPacketRef"],"facts":[fact,unbound],"issues":[]}),
                );
            }
            Ok(
                json!({"schemaVersion":WIRE_RESULT,"coveredPacketRef":request.input["coveredPacketRef"],"facts":if self.mode=="empty"||self.mode=="partial"{json!([])}else{json!([fact])},"issues":[]}),
            )
        }
        async fn accept(&self, _request: &TextRequest, value: &Value) -> Result<(), String> {
            self.accepted.lock().unwrap().push(value.clone());
            Ok(())
        }
        async fn reject(
            &self,
            _request: &TextRequest,
            value: &Value,
            reason: &str,
        ) -> Result<(), String> {
            self.rejected.lock().unwrap().push(reason.into());
            self.rejected_values.lock().unwrap().push(value.clone());
            Ok(())
        }
    }
    fn choice_correction(request: &TextRequest, item: &Value, foreign: bool) -> Value {
        let choices = request.input["visualFactRepair"]["acceptedProductionConstraints"]
            .as_array()
            .unwrap();
        let selected: Vec<_> = choices
            .iter()
            .filter(|choice| foreign || choice["subjectAssetId"] == item["fact"]["assetId"])
            .collect();
        let mut ids: Vec<_> = selected
            .iter()
            .map(|choice| choice["assetId"].clone())
            .collect();
        if foreign {
            ids.push(json!("李:base"));
        }
        json!({"factIndex":item["factIndex"],"action":"CURRENT_SCOPE_NOT_APPLICABLE","variantId":null,"reason":"保留原文明示未来短发；当前已采用取舍与本批集场明确使用前期造型。","productionConstraintRefs":selected.iter().map(|choice|choice["constraintSha256"].clone()).collect::<Vec<_>>(),"notApplicableAssetIds":ids})
    }

    fn production_choice_basis() -> Value {
        let mut input = basis();
        let source = prepare_source(
            "第一集\n场景：门口\n△小王和小李出现，小王前期留长发，后期剪清爽短发。\n",
        )
        .unwrap();
        input["source"] = json!(source);
        input["foundation"]["sourceSha256"] = json!(source.sha256);
        let episode = input["source"]["episodes"][0]["id"].clone();
        let scene = input["source"]["scenes"][0]["id"].clone();
        for asset in input["assets"].as_array_mut().unwrap() {
            asset["sourceRefs"] = json!([source.rows[2].id]);
            asset["version"] = json!(3);
            asset["description"] = json!("正式前期造型");
            asset["applicability"] = json!({"episodes":[episode],"scenes":[scene]});
            asset["preparation"] =
                json!({"mode":"REFERENCE","note":"当前批次锁定前期长发，不提前采用后期短发。"});
        }
        input["assetRegistry"] = input["assets"].clone();
        input
    }

    #[test]
    fn current_scope_decision_covers_only_the_named_shape_and_keeps_a_future_sibling_blocked() {
        let mut input = production_choice_basis();
        let mut sibling = input["assets"][0].clone();
        sibling["id"] = json!("王:future");
        sibling["variantId"] = json!("future");
        sibling["name"] = json!("王未来形象");
        sibling["preparation"]["note"] = json!("未来短发另列，未采用本批基础造型取舍。");
        input["assets"].as_array_mut().unwrap().push(sibling);
        input["assetRegistry"] = input["assets"].clone();
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let identities = identity_cards(&input, input["assets"].as_array().unwrap());
        let rows: Vec<_> = source.rows.iter().collect();
        let wire = PacketWire::compile(&source, &identities, &rows);
        let constraints = production_constraints(
            &source,
            &wire,
            &identities,
            input["assetRegistry"].as_array().unwrap(),
        )
        .unwrap();
        let base = constraints
            .iter()
            .find(|constraint| constraint["assetId"] == "王:base")
            .unwrap();
        let fact = json!({"assetId":"王","variantId":null,"attribute":"hair.style","value":"后期剪清爽短发","scope":"TRANSITION","claim":"SOURCE","sourceRefs":[source.rows[2].id],"sourceSha256":source.sha256,"id":"original-future-fact"});
        let mut delta = json!({"schemaVersion":VISUAL_FACT_REPAIR_PROFILE,"originalRawSha256":"original-raw","corrections":[{"factIndex":0,"action":"CURRENT_SCOPE_NOT_APPLICABLE","variantId":null,"reason":"仅此本批形象明确锁定前期长发；未来形象不承借该取舍。","productionConstraintRefs":[base["constraintSha256"]],"notApplicableAssetIds":["王:base"]}]});
        let row_ids = wire.row_ids.iter().map(String::as_str).collect();
        apply_visual_fact_corrections_with_constraints(
            &mut delta,
            "original-raw",
            &[fact.clone()],
            &[0],
            &identities,
            &source.sha256,
            &row_ids,
            &constraints,
        )
        .unwrap();
        assert_eq!(delta["deferredFacts"][0]["fact"], fact);
        assert!(delta["facts"].as_array().unwrap().is_empty());
        assert!(delta["excludedFacts"].as_array().unwrap().is_empty());
        let needed: Vec<_> = input["assets"].as_array().unwrap().iter().collect();
        let blocked = deferred_blocked_subjects(&needed, &delta);
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0]["assetId"], "王:future");
    }

    #[tokio::test]
    async fn adopted_current_production_choice_defers_future_fact_without_rebinding_or_losing_source(
    ) {
        let mut input = production_choice_basis();
        let exec = Extractor::new("repair-defer");
        let output = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(exec.calls.lock().unwrap().len(), 2);
        assert_eq!(exec.accepted.lock().unwrap().len(), 1);
        assert_eq!(
            exec.rejected.lock().unwrap().as_slice(),
            [VISUAL_FACT_UNBOUND_ERROR]
        );
        assert!(output["blocked"].as_array().unwrap().is_empty());
        assert_eq!(output["facts"].as_array().unwrap().len(), 1);
        let cache = &output["visualFactBatches"][0];
        assert!(cache["excludedFacts"].as_array().unwrap().is_empty());
        let deferred = &cache["deferredFacts"][0];
        assert_eq!(deferred["fact"]["scope"], "TRANSITION");
        assert!(deferred["fact"]["variantId"].is_null());
        assert_eq!(
            deferred["fact"]["sourceRefs"],
            json!([input["source"]["rows"][2]["id"]])
        );
        let requests = exec.requests.lock().unwrap();
        assert!(requests[0].input.get("visualFactRepair").is_none());
        assert!(requests[0].rule_snapshot.get("runtimeContract").is_none());
        assert_eq!(requests[1].input["visualFactRepair"]["attempt"], 1);
        assert_eq!(
            requests[1].input["visualFactRepair"]["acceptedProductionConstraints"][0]
                ["assetVersion"],
            3
        );
        validate_visual_fact_repair_request(&requests[1]).unwrap();
        drop(requests);
        input["foundation"]["visualFactBatches"] = output["visualFactBatches"].clone();
        assert_eq!(
            run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
                .await
                .unwrap(),
            output
        );
        // Image prompt/design changes advance the row version, not its adopted
        // production choice. The old hashes and original fact remain untouched.
        for key in ["assets", "assetRegistry"] {
            for asset in input[key].as_array_mut().unwrap() {
                asset["version"] = json!(4);
                asset["prompt"] = json!("新图片提示词");
            }
        }
        let replay = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(replay["blocked"].as_array().unwrap().is_empty());
        assert_eq!(replay["visualFactBatches"], output["visualFactBatches"]);
        assert!(!replay["visualFactCacheReuses"]
            .as_array()
            .unwrap()
            .is_empty());
        for field in ["name", "description"] {
            let mut changed = input.clone();
            changed["assets"][0][field] = json!("不同正式语义");
            changed["assetRegistry"][0][field] = json!("不同正式语义");
            assert!(
                !run_asset_visual_facts(&changed, &Never, "gpt-6.1-sol", "medium")
                    .await
                    .unwrap()["blocked"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        let mut changed = input.clone();
        changed["assets"][0]["preparation"]["note"] = json!("改为本批短发");
        changed["assetRegistry"][0]["preparation"]["note"] = json!("改为本批短发");
        assert!(
            !run_asset_visual_facts(&changed, &Never, "gpt-6.1-sol", "medium")
                .await
                .unwrap()["blocked"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn production_choice_cannot_authorize_other_identity_or_polluted_source_or_constraint_hash(
    ) {
        let input = production_choice_basis();
        let exec = Extractor::new("repair-defer-foreign");
        let output = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(exec.calls.lock().unwrap().len(), 2);
        assert!(exec.accepted.lock().unwrap().is_empty());
        assert_eq!(output["blocked"].as_array().unwrap().len(), 1);
        let request = exec.requests.lock().unwrap()[1].clone();
        for field in ["sourceSha256", "constraintSha256", "sourceRefsSha256"] {
            let mut changed = request.clone();
            changed.input["visualFactRepair"]["acceptedProductionConstraints"][0][field] =
                json!("f".repeat(64));
            assert!(validate_visual_fact_repair_request(&changed).is_err());
        }
    }

    struct ChoiceResume {
        context: Value,
        requests: Mutex<Vec<TextRequest>>,
        accepted: Mutex<Vec<Value>>,
    }
    #[async_trait]
    impl TextExecutor for ChoiceResume {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            assert_eq!(request.input["visualFactRepair"]["attempt"], 2);
            assert_eq!(
                request.input["visualFactRepair"]["legacyProductionConstraintUpgrade"],
                true
            );
            validate_visual_fact_repair_request(&request)?;
            self.requests.lock().unwrap().push(request.clone());
            Ok(
                json!({"schemaVersion":VISUAL_FACT_REPAIR_PROFILE,"originalRawSha256":request.input["visualFactRepair"]["originalRawSha256"],"corrections":request.input["unassignedVisualFacts"].as_array().unwrap().iter().map(|item|choice_correction(&request,item,false)).collect::<Vec<_>>()}),
            )
        }
        async fn accept(&self, _request: &TextRequest, value: &Value) -> Result<(), String> {
            self.accepted.lock().unwrap().push(value.clone());
            Ok(())
        }
        async fn visual_fact_repair_context(
            &self,
            _source_sha: &str,
            _packet_hash: &str,
            _domain_hash: &str,
            _raw_hash: &str,
        ) -> Result<Option<Value>, String> {
            Ok(Some(self.context.clone()))
        }
    }

    #[tokio::test]
    async fn legacy_unresolved_accepted_first_delta_uses_only_one_context_bound_second_a15_attempt()
    {
        let mut original_input = production_choice_basis();
        for key in ["assets", "assetRegistry"] {
            for asset in original_input[key].as_array_mut().unwrap() {
                asset["preparation"] = Value::Null;
            }
        }
        let old = Extractor::new("repair-unresolved");
        let original = run_asset_visual_facts(&original_input, &old, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(original["blocked"].as_array().unwrap().len(), 1);
        let requests = old.requests.lock().unwrap();
        let context = json!({"originalRequest":requests[0],"originalRaw":old.rejected_values.lock().unwrap()[0],"acceptedRepairRequest":requests[1],"acceptedRepair":old.accepted.lock().unwrap()[0],"originJobId":"original-job","acceptedRepairJobId":"first-repair-job"});
        drop(requests);
        let exec = ChoiceResume {
            context: context.clone(),
            requests: Mutex::new(Vec::new()),
            accepted: Mutex::new(Vec::new()),
        };
        let mut current = production_choice_basis();
        current["foundation"]["facts"] = original["facts"].clone();
        current["foundation"]["visualFactBatches"] = original["visualFactBatches"].clone();
        let repaired = run_asset_visual_facts(&current, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(
            exec.requests.lock().unwrap().len(),
            1,
            "no new extraction or first repair"
        );
        assert_eq!(exec.accepted.lock().unwrap().len(), 1);
        assert!(repaired["blocked"].as_array().unwrap().is_empty());
        assert_eq!(repaired["facts"], original["facts"]);
        let nodes = &repaired["visualFactBatches"][0]["repairProvenance"]["nodes"];
        assert_eq!(nodes.as_array().unwrap().len(), 2);
        assert_eq!(
            nodes[0],
            original["visualFactBatches"][0]["repairProvenance"]["nodes"][0]
        );
        assert_eq!(nodes[1]["attempt"], 2);
        assert_eq!(
            context["originalRaw"],
            old.rejected_values.lock().unwrap()[0]
        );
        current["foundation"]["visualFactBatches"] = repaired["visualFactBatches"].clone();
        let replay = run_asset_visual_facts(&current, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(replay["blocked"].as_array().unwrap().is_empty());
        assert_eq!(replay["visualFactBatches"], repaired["visualFactBatches"]);
        for field in ["originalRaw", "acceptedRepair"] {
            let mut polluted = context.clone();
            polluted[field]["facts"][0]["value"] = json!("污染事实");
            let bad = ChoiceResume {
                context: polluted,
                requests: Mutex::new(Vec::new()),
                accepted: Mutex::new(Vec::new()),
            };
            current["foundation"]["visualFactBatches"] = original["visualFactBatches"].clone();
            let result = run_asset_visual_facts(&current, &bad, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert_eq!(bad.requests.lock().unwrap().len(), 0);
            assert!(!result["blocked"].as_array().unwrap().is_empty());
        }
    }
    struct Never;
    #[async_trait]
    impl TextExecutor for Never {
        async fn execute(&self, _request: TextRequest) -> Result<Value, String> {
            panic!("有效或空结果缓存不应重复调用")
        }
    }
    fn basis() -> Value {
        let source = prepare_source("第一集\n场景：门口\n△小王和小李黑发出现。\n").unwrap();
        let row = source.rows[2].id.clone();
        json!({"source":source,"assets":[{"id":"王:base","baseAssetId":"王","variantId":"base","name":"王","sourceRefs":[row],"prompt":"旧历史不应发给提取岗"},{"id":"李:base","baseAssetId":"李","variantId":"base","name":"李","sourceRefs":[row]}],"foundation":{"sourceSha256":source.sha256,"facts":[]}})
    }

    #[tokio::test]
    async fn asset_fact_source_binding_survives_publish_round_trip_and_rejects_foreign_sha() {
        let input = basis();
        let output = run_asset_visual_facts(&input, &Extractor::new("valid"), "offline", "medium").await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(directory.path().to_path_buf()).unwrap();
        store.create("offline facts round trip", "fixture").unwrap();
        let source = store.save_source(input["source"].clone()).unwrap();
        let mut published = output.clone();
        published["assets"] = json!([]);
        store.publish_asset_design(source["version"].as_i64().unwrap(), 0, &published).unwrap();
        let saved = store.foundation().unwrap();
        for (actual, expected) in saved["facts"].as_array().unwrap().iter().zip(output["facts"].as_array().unwrap()) {
            assert_eq!(actual["sourceSha256"], expected["sourceSha256"]);
            let mut actual = actual.clone();
            actual.as_object_mut().unwrap().remove("quotes");
            assert_eq!(actual, *expected);
        }
        let version = saved["version"].as_i64().unwrap();
        published["facts"][0]["sourceSha256"] = json!("foreign-source");
        assert!(store.publish_asset_design(source["version"].as_i64().unwrap(), version, &published).unwrap_err().contains("源摘要"));
        assert_eq!(store.foundation().unwrap(), saved);
    }

    #[tokio::test]
    async fn persisted_fact_source_binding_restores_only_exact_validated_cache_facts() {
        let mut input = basis();
        let output = run_asset_visual_facts(&input, &Extractor::new("valid"), "offline", "medium").await.unwrap();
        input["foundation"]["facts"] = output["facts"].clone();
        input["foundation"]["visualFactBatches"] = output["visualFactBatches"].clone();
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        for change in ["none", "missingCache", "rule", "packet", "cacheSource", "factValue", "foundationSource", "explicitWrongSha"] {
            let mut current = input.clone();
            let mut facts = output["facts"].as_array().unwrap().clone();
            for fact in &mut facts { fact.as_object_mut().unwrap().remove("sourceSha256"); }
            match change {
                "none" => {}
                "missingCache" => current["foundation"]["visualFactBatches"] = json!([]),
                "rule" => current["foundation"]["visualFactBatches"][0]["ruleSha256"] = json!("changed"),
                "packet" => current["foundation"]["visualFactBatches"][0]["inputFingerprint"] = json!("changed"),
                "cacheSource" => current["foundation"]["visualFactBatches"][0]["sourceSha256"] = json!("changed"),
                "factValue" => facts[0]["value"] = json!("changed"),
                "foundationSource" => current["foundation"]["sourceSha256"] = json!("changed"),
                "explicitWrongSha" => facts[0]["sourceSha256"] = json!("changed"),
                _ => unreachable!(),
            }
            let before = facts.clone();
            restore_persisted_fact_source_bindings(&current, &source, current["assets"].as_array().unwrap(), &Never, &mut facts).await.unwrap();
            if change == "none" { assert_eq!(facts, *output["facts"].as_array().unwrap()); }
            else { assert_eq!(facts, before, "{change}: cannot invent an old source binding"); }
        }
    }

    #[tokio::test]
    #[ignore = "requires DIRECTOR_ASSET_PROMPT_SNAPSHOT_ROOT; reads the provided incident snapshot with SQLite read-only flags, no CLI"]
    async fn actual_15_asset_design_packages_restore_persisted_fact_source_bindings_without_cli() {
        let root = std::path::PathBuf::from(std::env::var_os("DIRECTOR_ASSET_PROMPT_SNAPSHOT_ROOT").expect("incident snapshot required"));
        let conn = rusqlite::Connection::open_with_flags(root.join("project.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let source: Value = serde_json::from_str(&conn.query_row("SELECT data FROM sources ORDER BY version DESC LIMIT 1", [], |r| r.get::<_,String>(0)).unwrap()).unwrap();
        let foundation: Value = serde_json::from_str(&conn.query_row("SELECT data FROM foundations ORDER BY version DESC LIMIT 1", [], |r| r.get::<_,String>(0)).unwrap()).unwrap();
        let mut q = conn.prepare("SELECT data FROM assets ORDER BY id").unwrap();
        let assets: Vec<Value> = q.query_map([], |r|r.get::<_,String>(0)).unwrap().map(|r|serde_json::from_str(&r.unwrap()).unwrap()).collect();
        drop(q);
        let input = json!({"source":source,"foundation":foundation,"assets":assets});
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        validate_source(&source).unwrap();
        let mut restored = input["foundation"]["facts"].as_array().unwrap().clone();
        restore_persisted_fact_source_bindings(&input, &source, &assets, &Never, &mut restored).await.unwrap();
        let mut q = conn.prepare("SELECT snapshot FROM jobs WHERE kind='text' AND json_extract(snapshot,'$.stage')='assetDesign' AND json_extract(snapshot,'$.executorIdentity.version') IS NULL").unwrap();
        let snapshots: Vec<Value> = q.query_map([], |r|r.get::<_,String>(0)).unwrap().map(|r|serde_json::from_str(&r.unwrap()).unwrap()).collect();
        assert_eq!(snapshots.len(), 23);
        let mut affected = 0;
        let mut checked = 0;
        for snapshot in snapshots {
            let old = snapshot["input"]["facts"].as_array().unwrap();
            if !old.is_empty() { affected += 1; }
            for fact in old {
                let mut actual = restored.iter().find(|current|current["id"]==fact["id"]).unwrap().clone();
                actual.as_object_mut().unwrap().remove("quotes");
                assert_eq!(actual, *fact, "only the original validated source binding may be restored");
                checked += 1;
            }
        }
        assert_eq!((affected, checked), (15,36));
        eprintln!("15 asset-design packages / 36 original facts matched after proven source binding restoration; zero CLI executions and database writes");
    }
    fn episode_subset_basis() -> Value {
        let source = prepare_source(
            "第一集\n场景：门口\n△甲黑发出现。\n第二集\n场景：院子\n△乙走入院子。\n",
        )
        .unwrap();
        json!({"source":source,"foundation":{"sourceSha256":source.sha256,"facts":[]},"assets":[
            {"id":"A:base","baseAssetId":"A","variantId":"A:base","name":"甲","sourceRefs":[source.rows[2].id],"applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]}},
            {"id":"B:base","baseAssetId":"B","variantId":"B:base","name":"乙","sourceRefs":[source.rows[5].id],"applicability":{"episodes":[source.episodes[1].id],"scenes":[source.scenes[1].id]}}
        ]})
    }

    #[tokio::test]
    async fn accepted_superset_survives_fact_coverage_shrinking_episode_scope_without_a_model_call()
    {
        let mut input = episode_subset_basis();
        let executor = Extractor::new("valid");
        let first = run_asset_visual_facts(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(executor.calls.lock().unwrap().len(), 1);
        assert_eq!(
            first["visualFactBatches"][0]["rowIds"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
        input["foundation"]["facts"] = first["facts"].clone();
        input["foundation"]["visualFactBatches"] = first["visualFactBatches"].clone();
        let frozen = input["foundation"]["visualFactBatches"].clone();
        // Changes to production output metadata never change an A15 source packet.
        input["assets"][0]["version"] = json!(12);
        input["assets"][0]["prompt"] = json!("新的图片生产提示词");
        input["assets"][0]["design"] = json!({"history":"not A15 material"});
        let replay = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(replay["blocked"].as_array().unwrap().is_empty());
        assert_eq!(replay["facts"], first["facts"]);
        assert_eq!(
            replay["visualFactBatches"], frozen,
            "retain the original adopted packet, never forge a smaller packet receipt"
        );
        let proof = &replay["visualFactCacheReuses"][0];
        assert_eq!(proof["originalRowCount"], 6);
        assert_eq!(proof["requestedRowCount"], 3);
        assert_eq!(
            proof["originalInputFingerprint"],
            frozen[0]["inputFingerprint"]
        );
        assert_ne!(
            proof["requestedInputFingerprint"],
            proof["originalInputFingerprint"]
        );
        assert_eq!(proof["originalPacketRebuilt"], true);
        assert_eq!(proof["factsFullyValidated"], true);
        assert_eq!(input["foundation"]["visualFactBatches"], frozen);
    }

    #[tokio::test]
    async fn an_exact_accepted_packet_is_preferred_to_an_equally_valid_superset() {
        let mut input = episode_subset_basis();
        let executor = Extractor::new("valid");
        let first = run_asset_visual_facts(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        input["foundation"]["facts"] = first["facts"].clone();
        input["foundation"]["visualFactBatches"] = first["visualFactBatches"].clone();
        let subset = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        let mut exact = first["visualFactBatches"][0].clone();
        exact["inputFingerprint"] =
            subset["visualFactCacheReuses"][0]["requestedInputFingerprint"].clone();
        exact["coveredPacketRef"] = exact["inputFingerprint"].clone();
        exact["rowIds"] = subset["visualFactCacheReuses"][0]["requestedRowIds"].clone();
        exact["facts"] = json!([]);
        input["foundation"]["visualFactBatches"]
            .as_array_mut()
            .unwrap()
            .push(exact.clone());
        let replay = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(replay["visualFactBatches"], json!([exact]));
        assert!(replay["visualFactCacheReuses"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(replay["blocked"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn changing_selected_subjects_in_the_same_episode_extracts_only_the_new_packet() {
        let mut input=episode_subset_basis();
        let mut registry=input["assets"].as_array().unwrap().clone();
        registry[1]["sourceRefs"]=registry[0]["sourceRefs"].clone();
        registry[1]["applicability"]=registry[0]["applicability"].clone();
        input["assetRegistry"]=json!(registry);
        input["assets"]=json!([registry[0]]);
        let first_executor=Extractor::new("valid");
        let first=run_asset_visual_facts(&input,&first_executor,"gpt-6.1-sol","medium").await.unwrap();
        input["foundation"]["facts"]=first["facts"].clone();
        input["foundation"]["visualFactBatches"]=first["visualFactBatches"].clone();
        let original=input["foundation"]["visualFactBatches"].clone();
        input["assets"]=json!([registry[1]]);
        let next_executor=Extractor::new("valid");
        let next=run_asset_visual_facts(&input,&next_executor,"gpt-6.1-sol","medium").await.unwrap();
        assert!(next["blocked"].as_array().unwrap().is_empty());
        assert_eq!(next_executor.calls.lock().unwrap().len(),1);
        assert_eq!(next["visualFactBatches"][0]["rowIds"],original[0]["rowIds"]);
        assert_ne!(next["visualFactBatches"][0]["inputFingerprint"],original[0]["inputFingerprint"]);
        assert_eq!(next["visualFactBatches"][0]["identityCards"][0]["assetId"],"B");
        assert_eq!(input["foundation"]["visualFactBatches"],original);
        // A same-selection corrupted cache remains a hard failure, not a retry.
        input["foundation"]["visualFactBatches"]=next["visualFactBatches"].clone();
        input["foundation"]["visualFactBatches"][0]["inputFingerprint"]=json!("corrupt");
        let held=run_asset_visual_facts(&input,&Never,"gpt-6.1-sol","medium").await.unwrap();
        assert!(!held["blocked"].as_array().unwrap().is_empty());
    }

    #[test]
    fn legacy_compact_identity_requires_exact_source_packet_and_registry() {
        let input = episode_subset_basis();
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let registry = input["assets"].as_array().unwrap();
        let identities = identity_cards(&input, registry);
        let rows: Vec<_> = source.rows.iter().collect();
        let wire = PacketWire::compile(&source, &identities, &rows);
        let previous = wire.input["identities"].as_array().unwrap();
        assert_ne!(*previous, identities);
        let cache = json!({"inputFingerprint":wire.packet_hash,"rowIds":wire.row_ids});
        assert_eq!(cached_identity_subset(&input,registry,previous,&source,&cache).unwrap(),identities);
        for field in ["hash","alias","registry"] {
            let mut altered_cache=cache.clone(); let mut altered=previous.clone(); let mut changed=registry.clone();
            match field {
                "hash"=>altered_cache["inputFingerprint"]=json!("wrong"),
                "alias"=>altered[0]["versions"][0]["applicability"]["episodes"][0]=json!("e999"),
                _=>changed[0]["name"]=json!("changed name"),
            }
            assert!(cached_identity_subset(&input,&changed,&altered,&source,&altered_cache).is_err(),"{field}");
        }
    }

    #[tokio::test]
    #[ignore = "Explicit private current A15 packet; no CLI or production writes"]
    async fn actual_legacy_compact_packet_selects_new_subjects_without_cli() {
        use crate::{store::Store,text_executor::SavedTextExecutor};
        let root=std::path::PathBuf::from(std::env::var("DIRECTOR_VISUAL_SELECTION_REGRESSION_ROOT").expect("private copy required")).canonicalize().unwrap();
        let allowed=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("reports/desktop-real-e2e-20261005").canonicalize().unwrap();
        assert!(root.starts_with(&allowed)&&root.file_name().unwrap().to_string_lossy().starts_with("boundary-regression-heart-"));
        let store=Store::open(root).unwrap();
        let workflow=store.job("01a8bcc6-94f0-42f4-a4c3-c208de0d764d").unwrap();
        let all=store.values("assets").unwrap();
        let wanted=workflow["snapshot"]["request"]["assetIds"].as_array().unwrap();
        let selected:Vec<_>=all.iter().filter(|a|wanted.contains(&a["id"])).cloned().collect();
        assert_eq!(selected.len(),16);
        let input=json!({"source":store.source().unwrap(),"foundation":store.foundation().unwrap(),"assets":selected,"assetRegistry":all,"selection":{"assetIds":wanted}});
        struct NoCli { saved:SavedTextExecutor,calls:Mutex<Vec<TextRequest>> }
        #[async_trait]
        impl TextExecutor for NoCli {
            async fn visual_fact_packet_identities(&self,source:&str,packet:&str)->Result<Option<Vec<Value>>,String>{self.saved.visual_fact_packet_identities(source,packet).await}
            async fn execute(&self,request:TextRequest)->Result<Value,String>{self.calls.lock().unwrap().push(request);Err("NO_CLI_DIAGNOSTIC_NEW_PACKET".into())}
        }
        let executor=NoCli{saved:SavedTextExecutor{store:store.clone(),settings:json!({"textProvider":"codex"}),workflow_id:None},calls:Mutex::new(Vec::new())};
        let count:i64=store.conn().unwrap().query_row("SELECT count(*) FROM jobs",[],|r|r.get(0)).unwrap();
        let output=run_asset_visual_facts(&input,&executor,"gpt-6.1-sol","medium").await.unwrap();
        assert!(!executor.calls.lock().unwrap().is_empty(),"old packet must not block new subjects: {}",output["blocked"]);
        assert!(output["blocked"].as_array().unwrap().iter().all(|b|!text(b,"reason").contains("旧缓存无效")));
        assert_eq!(store.conn().unwrap().query_row("SELECT count(*) FROM jobs",[],|r|r.get::<_,i64>(0)).unwrap(),count);
        println!("{{\"legacyAliasesRecompiled\":true,\"newSubjectsNotBlocked\":true,\"modelExecutions\":0,\"productionWrites\":0}}");
    }

    #[test]
    fn superset_packet_rebuild_rejects_wrong_source_rule_identity_order_hash_or_fact_binding() {
        let source = prepare_source(
            "第一集\n场景：门口\n△苏远黑发出现。\n第二集\n场景：院子\n△苏远走入院子。\n",
        )
        .unwrap();
        let identities = identities(&source);
        let rows: Vec<_> = source.rows.iter().collect();
        let wire = PacketWire::compile(&source, &identities, &rows);
        let mut response = wire_reply(&wire, &wire.row_aliases[2]);
        wire.materialize(&mut response, &identities, &source.sha256)
            .unwrap();
        let rule_sha = rule_snapshot("asset-visual-facts-extractor").unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string();
        let cache = json!({"schemaVersion":"asset-visual-facts-cache/v1","status":"accepted","sourceSha256":source.sha256,"inputFingerprint":wire.packet_hash,"rowIds":wire.row_ids,"ruleSha256":rule_sha,"facts":response["facts"]});
        let requested = source.rows[3..]
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            validate_cached_packet(&cache, &source, &identities, &requested, &rule_sha).unwrap(),
            response["facts"].as_array().unwrap()
        );
        let frozen = cache.clone();
        for (pointer, value) in [
            ("/sourceSha256", json!("wrong-source")),
            ("/ruleSha256", json!("wrong-rule")),
            ("/inputFingerprint", json!("polluted-hash")),
            ("/status", json!("rejected")),
            ("/facts/0/sourceRefs", json!(["unknown-row"])),
            ("/facts/0/assetId", json!("unknown-subject")),
            ("/facts/0/variantId", json!("foreign-version")),
            ("/facts/0/sourceSha256", json!("wrong-fact-source")),
        ] {
            let mut polluted = cache.clone();
            *polluted.pointer_mut(pointer).unwrap() = value;
            assert!(
                validate_cached_packet(&polluted, &source, &identities, &requested, &rule_sha)
                    .is_err(),
                "{pointer} must fail closed"
            );
        }
        let mut changed_identity = identities.clone();
        changed_identity[0]["name"] = json!("different catalog identity description");
        assert!(
            validate_cached_packet(&cache, &source, &changed_identity, &requested, &rule_sha)
                .is_err()
        );
        let mut wrong_order = requested.clone();
        wrong_order.reverse();
        assert!(
            validate_cached_packet(&cache, &source, &identities, &wrong_order, &rule_sha).is_err()
        );
        let mut duplicate = cache.clone();
        duplicate["rowIds"][1] = duplicate["rowIds"][0].clone();
        assert!(
            validate_cached_packet(&duplicate, &source, &identities, &requested, &rule_sha)
                .is_err()
        );
        assert_eq!(
            cache, frozen,
            "checking never repairs, relabels or mutates the old adopted receipt"
        );
    }
    #[tokio::test]
    async fn only_real_partial_progress_allows_a_second_a15_delta_and_cached_resume_calls_none() {
        let mut input = basis();
        let executor = Extractor::new("repair-progress");
        let output = run_asset_visual_facts(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(output["blocked"].as_array().unwrap().is_empty());
        assert_eq!(output["facts"].as_array().unwrap().len(), 2);
        assert_eq!(
            executor.calls.lock().unwrap().len(),
            3,
            "one original extraction and at most two original-role delta repairs"
        );
        assert_eq!(
            executor.accepted.lock().unwrap().len(),
            2,
            "the original unbound batch is never accepted"
        );
        let calls = executor.calls.lock().unwrap();
        assert_eq!(calls[1]["visualFactRepair"]["attempt"], 1);
        assert_eq!(calls[2]["visualFactRepair"]["attempt"], 2);
        assert_eq!(
            calls[2]["unassignedVisualFacts"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            calls[1]["rejectedVisualFacts"],
            calls[2]["rejectedVisualFacts"]
        );
        drop(calls);
        input["foundation"]["visualFactBatches"] = output["visualFactBatches"].clone();
        let replay = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(replay, output);
    }
    #[tokio::test]
    async fn one_packet_reads_source_once_for_all_subjects_and_program_attaches_fact_identity() {
        let input = basis();
        let exec = Extractor::new("valid");
        let output = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(output["blocked"].as_array().unwrap().is_empty());
        assert_eq!(exec.calls.lock().unwrap().len(), 1);
        assert_eq!(exec.accepted.lock().unwrap().len(), 1);
        let calls = exec.calls.lock().unwrap();
        assert_eq!(calls[0]["identities"].as_array().unwrap().len(), 2);
        assert!(!serde_json::to_string(&calls[0]).unwrap().contains("旧历史"));
        let rows = calls[0]["rows"].as_array().unwrap();
        assert_eq!(
            rows.len(),
            input["source"]["rows"].as_array().unwrap().len()
        );
        assert!(rows.iter().all(|row| row.get("dialogue").is_none()));
        assert!(output["facts"][0].get("quotes").is_none());
        assert!(text(&output["facts"][0], "id").starts_with("visual-"));
        assert_eq!(
            output["facts"][0]["sourceSha256"],
            input["source"]["sha256"]
        );
    }
    #[tokio::test]
    async fn invalid_owner_version_or_source_is_rejected_without_success_cache() {
        for mode in ["owner", "variant", "source"] {
            let exec = Extractor::new(mode);
            let output = run_asset_visual_facts(&basis(), &exec, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert_eq!(exec.accepted.lock().unwrap().len(), 0);
            assert_eq!(exec.rejected.lock().unwrap().len(), 1);
            assert!(output["facts"].as_array().unwrap().is_empty());
            assert!(output["visualFactBatches"].as_array().unwrap().is_empty());
            assert_eq!(output["blocked"].as_array().unwrap().len(), 2);
        }
    }
    #[tokio::test]
    async fn unassigned_transition_uses_only_original_a15_and_preserves_other_facts_and_cached_calls(
    ) {
        let mut input = basis();
        let exec = Extractor::new("repair-bind");
        let output = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(exec.calls.lock().unwrap().len(), 2);
        assert_eq!(
            exec.accepted.lock().unwrap().len(),
            1,
            "original unbound extraction stays rejected"
        );
        assert_eq!(
            exec.rejected.lock().unwrap().as_slice(),
            [VISUAL_FACT_UNBOUND_ERROR]
        );
        assert!(output["blocked"].as_array().unwrap().is_empty());
        assert_eq!(output["facts"].as_array().unwrap().len(), 2);
        assert_eq!(output["facts"][0]["scope"], "STABLE");
        assert!(output["facts"][0]["variantId"].is_null());
        assert_eq!(output["facts"][1]["variantId"], "base");
        let original = exec.rejected_values.lock().unwrap()[0].clone();
        assert!(original["facts"][1]["variantId"].is_null());
        for field in ["assetId", "attribute", "value", "scope", "claim"] {
            assert_eq!(output["facts"][1][field], original["facts"][1][field]);
        }
        let cache = &output["visualFactBatches"][0];
        assert_eq!(cache["repairProvenance"]["originalAccepted"], false);
        assert_eq!(
            cache["repairProvenance"]["originalRawSha256"],
            digest(&original.to_string())
        );
        input["foundation"]["visualFactBatches"] = output["visualFactBatches"].clone();
        let replay = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(
            replay, output,
            "cached repaired packet does not repeat the extractor or local correction"
        );
    }
    #[tokio::test]
    async fn unresolved_visual_transition_blocks_only_its_identity_and_keeps_the_original_record() {
        for mode in [
            "repair-unresolved",
            "repair-nonvisual",
            "repair-wrong-version",
            "repair-foreign-index",
        ] {
            let mut input = basis();
            let exec = Extractor::new(mode);
            let output = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
            assert_eq!(
                exec.calls.lock().unwrap().len(),
                2,
                "{mode}: unchanged issue cannot use a second call"
            );
            assert_eq!(output["facts"].as_array().unwrap().len(), 1);
            assert_eq!(output["facts"][0]["assetId"], "李");
            let blocked = output["blocked"].as_array().unwrap();
            if mode == "repair-nonvisual" {
                assert!(blocked.is_empty());
                assert_eq!(
                    output["visualFactBatches"][0]["excludedFacts"][0]["fact"]["scope"],
                    "TRANSITION"
                );
            } else {
                assert_eq!(blocked.len(), 1, "{mode}");
                assert_eq!(blocked[0]["assetId"], "王:base");
            }
            if mode == "repair-unresolved" || mode == "repair-nonvisual" {
                input["foundation"]["visualFactBatches"] = output["visualFactBatches"].clone();
                assert_eq!(
                    run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
                        .await
                        .unwrap(),
                    output
                );
            }
            assert!(exec.rejected_values.lock().unwrap()[0]["facts"][1]["variantId"].is_null());
        }
    }
    #[tokio::test]
    async fn unbound_state_cannot_hide_an_invalid_source_or_repair_an_unknown_identity() {
        let exec = Extractor::new("repair-hard-source");
        let output = run_asset_visual_facts(&basis(), &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(exec.calls.lock().unwrap().len(), 1);
        assert!(exec.accepted.lock().unwrap().is_empty());
        assert!(output["facts"].as_array().unwrap().is_empty());
        assert_eq!(output["blocked"].as_array().unwrap().len(), 2);
        assert!(output["visualFactBatches"].as_array().unwrap().is_empty());
    }
    #[tokio::test]
    async fn successful_empty_packet_is_reused_without_another_extraction() {
        let mut input = basis();
        let exec = Extractor::new("empty");
        let first = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(first["visualFactBatches"].as_array().unwrap().len(), 1);
        input["foundation"]["visualFactBatches"] = first["visualFactBatches"].clone();
        let second = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(second["blocked"].as_array().unwrap().is_empty());
        assert!(second["facts"].as_array().unwrap().is_empty());
        assert_eq!(second["visualFactBatches"], first["visualFactBatches"]);
    }
    struct Replay(Value);
    #[async_trait]
    impl TextExecutor for Replay {
        async fn execute(&self, _request: TextRequest) -> Result<Value, String> {
            Ok(self.0.clone())
        }
    }
    #[tokio::test]
    async fn checkpoint_replay_does_not_change_program_fact_ids_or_keep_model_quotes() {
        let input = basis();
        let exec = Extractor::new("valid");
        let first = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        let replay = Replay(exec.accepted.lock().unwrap()[0].clone());
        let second = run_asset_visual_facts(&input, &replay, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(second["facts"], first["facts"]);
        assert!(second["facts"][0].get("quotes").is_none());
    }
    #[tokio::test]
    async fn valid_existing_current_facts_are_preserved_without_reading_source_again() {
        let mut input = basis();
        let row = input["source"]["rows"][2]["id"].clone();
        input["foundation"]["facts"] = json!([{"id":"王-fact","assetId":"王","variantId":null,"attribute":"hair.color","value":"黑发","scope":"STABLE","claim":"SOURCE","sourceRefs":[row]},{"id":"李-fact","assetId":"李","variantId":null,"attribute":"hair.color","value":"黑发","scope":"STABLE","claim":"SOURCE","sourceRefs":[row]}]);
        let output = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(output["facts"], input["foundation"]["facts"]);
        assert!(output["blocked"].as_array().unwrap().is_empty());
    }
    #[tokio::test]
    async fn tampered_cached_fact_blocks_locally_without_automatic_reextraction() {
        let mut input = basis();
        let exec = Extractor::new("empty");
        let first = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        input["foundation"]["visualFactBatches"] = first["visualFactBatches"].clone();
        input["foundation"]["visualFactBatches"][0]["facts"] = json!([{"assetId":"王","variantId":null,"attribute":"hair.color","value":"黑发","scope":"STABLE","claim":"SOURCE","sourceRefs":["foreign-row"]}]);
        let output = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(output["blocked"].as_array().unwrap().len(), 2);
        assert!(output["facts"].as_array().unwrap().is_empty());
    }
    #[tokio::test]
    async fn capacity_packs_keep_successful_results_and_only_block_affected_subjects() {
        let text = format!(
            "第一集\n场景：门口\n△王：失败标记{}\n第二集\n场景：河边\n△李：{}\n",
            "甲".repeat(18_000),
            "乙".repeat(18_000)
        );
        let source = prepare_source(&text).unwrap();
        let a = source.rows[2].id.clone();
        let b = source.rows[5].id.clone();
        let input = json!({"source":source,"assets":[{"id":"a","variantId":"base","sourceRefs":[a]},{"id":"b","variantId":"base","sourceRefs":[b]}]});
        let exec = Extractor::new("partial");
        let output = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(exec.calls.lock().unwrap().len(), 2);
        assert_eq!(output["visualFactBatches"].as_array().unwrap().len(), 1);
        assert_eq!(output["blocked"].as_array().unwrap().len(), 1);
        assert_eq!(output["blocked"][0]["assetId"], "a");
    }

    fn identities(source: &LockedSource) -> Vec<Value> {
        vec![
            json!({"assetId":"person","name":"e1 r1 苏远","aliases":["s1"],"versions":[{"assetRowId":"person:base","variantId":"base","name":"r1 默认","applicability":{"scope":"SCENES","episodes":source.episodes.iter().map(|e|&e.id).collect::<Vec<_>>(),"scenes":source.scenes.iter().map(|s|&s.id).collect::<Vec<_>>()}}]}),
        ]
    }
    fn wire_reply(wire: &PacketWire, reference: &str) -> Value {
        json!({"schemaVersion":WIRE_RESULT,"coveredPacketRef":wire.packet_hash,"facts":[{"assetId":"person","variantId":null,"attribute":"hair.color","value":"原文黑发；r1 e1是原样说明","scope":"STABLE","claim":"SOURCE","sourceRefs":[reference]}],"issues":[]})
    }

    #[test]
    fn compact_transport_preserves_source_text_order_and_authority_without_identity_leak() {
        let mut source = prepare_source(
            "第一集\n场景：e1门口\n△r1和e1黑发出现。\n第二集\n场景：河边\n△苏远白衣。\n",
        )
        .unwrap();
        source.scenes[0].metadata.insert(
            "authority".into(),
            json!({"level":"SOURCE","note":"r1 e1原样"}),
        );
        let identities = identities(&source);
        let rows: Vec<_> = source.rows.iter().collect();
        let wire = PacketWire::compile(&source, &identities, &rows);
        let packet = canonical_packet(&source, &identities, &rows);
        assert_eq!(wire.packet_hash, digest(&packet.to_string()));
        assert_eq!(wire.input["domain"], "EPISODE");
        for (original, compact) in rows.iter().zip(wire.input["rows"].as_array().unwrap()) {
            for key in ["number", "text", "kind"] {
                assert_eq!(compact[key], packet["rows"][original.number - 1][key]);
            }
            assert_eq!(wire.rows_by_alias[text(compact, "id")], original.id);
            assert!(compact.get("range").is_none());
        }
        assert_eq!(wire.input["identities"][0]["name"], "e1 r1 苏远");
        assert_eq!(
            wire.input["identities"][0]["versions"][0]["name"],
            "r1 默认"
        );
        assert_eq!(
            wire.input["episodes"][0]["sourceNumber"],
            source.episodes[0].source_number
        );
        assert_eq!(wire.input["scenes"][0]["title"], source.scenes[0].title);
        assert_eq!(
            wire.input["scenes"][0]["authority"],
            source.scenes[0].metadata["authority"]
        );
        assert_eq!(
            wire.input["identities"][0]["versions"][0]["applicability"]["episodes"][0],
            wire.input["episodes"][0]["id"]
        );
        assert_eq!(
            wire.input["identities"][0]["versions"][0]["applicability"]["scenes"][0],
            wire.input["scenes"][0]["id"]
        );
        let mut raw = wire_reply(&wire, &wire.row_aliases[2]);
        let original = raw.clone();
        wire.materialize(&mut raw, &identities, &source.sha256)
            .unwrap();
        assert_eq!(
            raw["coveredRowIds"],
            json!(rows.iter().map(|r| &r.id).collect::<Vec<_>>())
        );
        assert_eq!(raw["facts"][0]["sourceRefs"], json!([source.rows[2].id]));
        assert_eq!(raw["facts"][0]["value"], original["facts"][0]["value"]);
        assert_eq!(
            original["facts"][0]["sourceRefs"],
            json!([wire.row_aliases[2]]),
            "raw wire was not rewritten"
        );
        let accepted = raw.clone();
        wire.materialize(&mut raw, &identities, &source.sha256)
            .unwrap();
        assert_eq!(
            raw, accepted,
            "accepted canonical checkpoint replay is idempotent"
        );
    }

    #[test]
    fn packet_receipt_and_sparse_references_are_strict_and_restore_is_atomic() {
        let source = prepare_source("作者：苏远黑发。\n第一集\n场景：门口\n△苏远白衣。\n").unwrap();
        let identities = identities(&source);
        let author: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.episode_id.is_none())
            .collect();
        let body: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.episode_id.is_some())
            .collect();
        assert!(!author.is_empty() && !body.is_empty());
        let a = PacketWire::compile(&source, &identities, &author);
        let b = PacketWire::compile(&source, &identities, &body);
        assert_ne!(a.packet_hash, b.packet_hash);
        assert_eq!(
            a.row_aliases[0], b.row_aliases[0],
            "short IDs are task local; receipt prevents cross-packet reuse"
        );
        let valid = wire_reply(&b, &b.row_aliases[0]);
        let mut cases = Vec::new();
        let mut wrong = valid.clone();
        wrong["coveredPacketRef"] = json!(a.packet_hash);
        cases.push(wrong);
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("coveredPacketRef");
        cases.push(missing);
        let mut duplicate = valid.clone();
        duplicate["facts"][0]["sourceRefs"] = json!([b.row_aliases[0], b.row_aliases[0]]);
        cases.push(duplicate);
        for reference in ["r99999", "e1", &author[0].id, &body[0].id] {
            let mut unknown = valid.clone();
            unknown["facts"][0]["sourceRefs"] = json!([reference]);
            cases.push(unknown);
        }
        let mut coords = valid.clone();
        coords["coveredRowIds"] = json!([b.row_aliases[0]]);
        cases.push(coords);
        let mut nonstring = valid.clone();
        nonstring["facts"][0]["sourceRefs"] = json!([7]);
        cases.push(nonstring);
        let mut empty = valid.clone();
        empty["facts"][0]["sourceRefs"] = json!([]);
        cases.push(empty);
        let mut issue = valid.clone();
        issue["issues"] = json!(["新增审查"]);
        cases.push(issue);
        for mut reply in cases {
            let original = reply.clone();
            assert!(
                b.materialize(&mut reply, &identities, &source.sha256)
                    .is_err(),
                "{original}"
            );
            assert_eq!(reply, original, "rejected raw stays byte-equivalent");
        }
        let mut partial = valid.clone();
        partial["facts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"assetId":"foreign"}));
        let original = partial.clone();
        assert!(b
            .materialize(&mut partial, &identities, &source.sha256)
            .is_err());
        assert_eq!(partial, original);
    }

    #[test]
    fn legacy_raw_is_still_source_bound_and_produces_the_same_fact_identity() {
        let source = prepare_source("第一集\n场景：门口\n△苏远黑发。\n").unwrap();
        let identities = identities(&source);
        let rows: Vec<_> = source.rows.iter().collect();
        let wire = PacketWire::compile(&source, &identities, &rows);
        let mut compact = wire_reply(&wire, &wire.row_aliases[2]);
        let mut legacy = json!({"schemaVersion":"asset-visual-facts/v1","coveredRowIds":wire.row_ids,"facts":compact["facts"],"issues":[]});
        legacy["facts"][0]["sourceRefs"] = json!([source.rows[2].id]);
        legacy["facts"][0]["quotes"] = json!(["模型引文不采用"]);
        wire.materialize(&mut compact, &identities, &source.sha256)
            .unwrap();
        let old = legacy.clone();
        wire.materialize(&mut legacy, &identities, &source.sha256)
            .unwrap();
        assert_eq!(legacy, compact);
        for mode in ["partial", "duplicate", "nonstring", "foreign", "alias"] {
            let mut invalid = old.clone();
            match mode {
                "partial" => {
                    invalid["coveredRowIds"].as_array_mut().unwrap().pop();
                }
                "duplicate" => invalid["coveredRowIds"][1] = invalid["coveredRowIds"][0].clone(),
                "nonstring" => invalid["coveredRowIds"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(1)),
                "foreign" => invalid["facts"][0]["sourceRefs"] = json!(["outside"]),
                "alias" => invalid["facts"][0]["sourceRefs"] = json!([wire.row_aliases[2]]),
                _ => unreachable!(),
            }
            let original = invalid.clone();
            assert!(
                wire.materialize(&mut invalid, &identities, &source.sha256)
                    .is_err(),
                "{mode}"
            );
            assert_eq!(invalid, original);
        }
    }

    #[tokio::test]
    async fn accepted_precompact_cache_uses_unchanged_packet_and_rule_hash_without_a_call() {
        let mut input = basis();
        let exec = Extractor::new("valid");
        let first = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let rows: Vec<_> = source.rows.iter().collect();
        let calls = exec.calls.lock().unwrap();
        // This fixture has no applicability aliases, so its subjects are exactly
        // the pre-transport identity table used to calculate the old hash.
        let packet = canonical_packet(&source, calls[0]["identities"].as_array().unwrap(), &rows);
        assert_eq!(
            first["visualFactBatches"][0]["inputFingerprint"],
            digest(&packet.to_string())
        );
        let mut old = first["visualFactBatches"].clone();
        old[0].as_object_mut().unwrap().remove("referenceEncoding");
        old[0].as_object_mut().unwrap().remove("coveredPacketRef");
        input["foundation"]["visualFactBatches"] = old.clone();
        let replay = run_asset_visual_facts(&input, &Never, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(replay["visualFactBatches"], old);
        assert_eq!(replay["facts"], first["facts"]);
        assert!(replay["blocked"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn author_and_episode_domains_keep_two_requests_and_exact_disjoint_materialized_coverage()
    {
        let source = prepare_source(
            "作者：苏远黑发。\n第一集\n场景：门口\n△苏远白衣。\n第二集\n场景：河边\n△苏远出现。\n",
        )
        .unwrap();
        let explicit: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .map(|r| &r.id)
            .collect();
        let input = json!({"source":source,"assets":[{"id":"a","baseAssetId":"person","variantId":"base","sourceRefs":explicit}],"foundation":{"facts":[]}});
        let exec = Extractor::new("empty");
        let output = run_asset_visual_facts(&input, &exec, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        let calls = exec.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["domain"], "AUTHOR");
        assert_eq!(calls[1]["domain"], "EPISODE");
        let covered: Vec<_> = output["visualFactBatches"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|b| strings(b, "rowIds"))
            .collect();
        assert_eq!(covered.iter().collect::<HashSet<_>>().len(), covered.len());
        assert_eq!(
            covered,
            source.rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            calls
                .iter()
                .map(|c| c["rows"].as_array().unwrap().len())
                .sum::<usize>(),
            covered.len()
        );
    }

    #[test]
    fn three_hundred_sixty_six_rows_have_one_receipt_and_one_short_reference_schema() {
        let text = format!(
            "第一集\n场景：门口\n{}",
            (1..=364)
                .map(|i| format!("△苏远第{i}行，e1 r1。\n"))
                .collect::<String>()
        );
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.rows.len(), 366);
        let identities = identities(&source);
        let rows: Vec<_> = source.rows.iter().collect();
        let wire = PacketWire::compile(&source, &identities, &rows);
        let schema = wire.schema(&identities);
        assert!(schema["properties"].get("coveredRowIds").is_none());
        assert_eq!(
            schema["$defs"]["sourceRow"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            366
        );
        assert_eq!(
            schema["properties"]["coveredPacketRef"]["enum"],
            json!([wire.packet_hash])
        );
        let packet = canonical_packet(&source, &identities, &rows);
        let old_ref_bytes = json!(wire.row_ids).to_string().len();
        let ack_bytes = json!({"coveredPacketRef":wire.packet_hash})
            .to_string()
            .len();
        assert!(
            ack_bytes * 50 < old_ref_bytes,
            "{ack_bytes} vs {old_ref_bytes}"
        );
        assert!(
            wire.input.to_string().len() < packet.to_string().len(),
            "names and instructions still outweigh aliases on short fixtures only"
        );
        assert!(
            schema.to_string().len() < old_ref_bytes,
            "schema no longer repeats hundreds of canonical IDs"
        );
        let mut empty = json!({"schemaVersion":WIRE_RESULT,"coveredPacketRef":wire.packet_hash,"facts":[],"issues":[]});
        wire.materialize(&mut empty, &identities, &source.sha256)
            .unwrap();
        assert_eq!(empty["coveredRowIds"].as_array().unwrap().len(), 366);
        println!("A15 366 rows: canonical_input_bytes={} compact_input_bytes={} old_coverage_bytes={} packet_ack_bytes={} compact_schema_bytes={}",packet.to_string().len(),wire.input.to_string().len(),old_ref_bytes,ack_bytes,schema.to_string().len());
    }
}
