//! Port of the 3008 light delivery: program fidelity -> packed F2 -> differential Q1.
//! The frozen single-episode implementation remains in the parent module.
use super::*;
use serde_json::Map;
use std::collections::BTreeSet;

pub const METHOD_VERSION: &str = "foundation-delivery/v2";
// Original defaults: project-foundation-contracts.ts, foundation-fragments.ts,
// and project-foundation.ts. These are application UTF-16 budgets, not model tokens.
const SOURCE_BUDGET: usize = 24_000;
const FRAGMENT_BUDGET: usize = 18_000;
const REQUEST_BUDGET: usize = 200_000;
const OUTPUT_RESERVE: usize = 24_000;
const BACKGROUND_CANDIDATES_BUDGET: usize = 12_000;
const F2_RULE: &str = "foundation-asset-catalog";
const Q1_RULE: &str = "foundation-finishing-editor";
pub(crate) const AUTHOR_CHOICES_VERSION: &str = "foundation-author-choices/v1";
pub(crate) const AUTO_INTERPRET_POLICY: &str = "q1-auto-interpret/v1";
pub(crate) const AUTO_INTERPRET_EVIDENCE_REJECTION: &str = "自动解释证据超出冻结来源或准确集场";
const CHOICE_ASSET_FIELDS: &[&str] = &[
    "name",
    "type",
    "role",
    "relationNote",
    "aliases",
    "sceneRefs",
    "preparation",
    "variants",
];
const CHOICE_SCENE_FIELDS: &[&str] = &[
    "location",
    "timeOfDay",
    "interiorExterior",
    "timeLayer",
    "weather",
    "characters",
];

fn choice_strings(value: &Value, at: &str) -> Result<Vec<String>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| format!("{at}须为文本数组"))?;
    let mut out = Vec::new();
    for item in items {
        let text = item
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("{at}含无效文本"))?;
        if !out.iter().any(|old| old == text) {
            out.push(text.to_owned());
        }
    }
    out.sort();
    Ok(out)
}
fn choice_targets(value: &Value, foundation: &FoundationProposal) -> Result<Value, String> {
    let fields = value.as_object().ok_or("选择修改权限须为明确对象")?;
    if fields
        .keys()
        .any(|k| !["assetKeys", "assetFields", "sceneFields"].contains(&k.as_str()))
    {
        return Err("选择不允许删除、归并、拆场或未知权限".into());
    }
    let keys = choice_strings(&value["assetKeys"], "assetKeys")?;
    let assets = choice_strings(&value["assetFields"], "assetFields")?;
    let scenes = choice_strings(&value["sceneFields"], "sceneFields")?;
    if keys
        .iter()
        .any(|k| !foundation.assets.iter().any(|a| &a.id == k))
        || assets
            .iter()
            .any(|f| !CHOICE_ASSET_FIELDS.contains(&f.as_str()))
        || scenes
            .iter()
            .any(|f| !CHOICE_SCENE_FIELDS.contains(&f.as_str()))
        || keys.is_empty() != assets.is_empty()
        || assets.is_empty() && scenes.is_empty()
    {
        return Err("旧选择权限缺失或不合法；仅可作为制作解释，不得改写原文或扩大权限".into());
    }
    Ok(json!({"assetKeys":keys,"assetFields":assets,"sceneFields":scenes}))
}
fn choice_scope(
    choice: &Value,
    fallback: &[String],
    source: &LockedSource,
) -> Result<Value, String> {
    let scope = &choice["applicationScope"];
    let kind = scope["kind"].as_str().unwrap_or("EPISODES");
    match kind {
        "GLOBAL" => Ok(
            json!({"kind":"GLOBAL","episodes":source.episodes.iter().map(|e|e.id.clone()).collect::<Vec<_>>()}),
        ),
        "EPISODES" => {
            let episodes = if scope["episodes"].is_array() {
                choice_strings(&scope["episodes"], "applicationScope.episodes")?
            } else if choice["episodes"].is_array()
                && !choice["episodes"].as_array().unwrap().is_empty()
            {
                choice_strings(&choice["episodes"], "episodes")?
            } else {
                fallback.to_vec()
            };
            if episodes.is_empty()
                || episodes
                    .iter()
                    .any(|id| !source.episodes.iter().any(|e| &e.id == id))
            {
                return Err("制作选择集范围不在当前原文".into());
            }
            Ok(json!({"kind":"EPISODES","episodes":episodes}))
        }
        "SCENES" => {
            let pairs = scope["scenes"].as_array().ok_or("制作选择场范围无效")?;
            let mut scenes = vec![];
            let mut episodes = BTreeSet::new();
            for pair in pairs {
                let pair = pair
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .ok_or("场范围须为[episodeId,sceneId]")?;
                let ep = pair[0].as_str().ok_or("场范围缺集ID")?;
                let sc = pair[1].as_str().ok_or("场范围缺场ID")?;
                if !source
                    .scenes
                    .iter()
                    .any(|s| s.episode_id == ep && s.id == sc)
                {
                    return Err("制作选择场范围不在当前原文".into());
                }
                let p = json!([ep, sc]);
                if !scenes.contains(&p) {
                    scenes.push(p);
                }
                episodes.insert(ep.to_owned());
            }
            if scenes.is_empty() {
                return Err("制作选择场范围不能为空".into());
            }
            scenes.sort_by_key(Value::to_string);
            Ok(json!({"kind":"SCENES","episodes":episodes,"scenes":scenes}))
        }
        _ => Err("制作选择范围类型无效".into()),
    }
}
fn choice_key(key: &str) -> String {
    key.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .split('/')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("/")
}

fn same_choice_evidence(record: &Value, contract: &Value) -> bool {
    [
        "id",
        "sourceVersion",
        "sourceSha256",
        "applicationScope",
        "sourceRefs",
        "members",
    ]
    .iter()
    .all(|key| record[*key] == contract[*key])
}

/// A source-bound projection, not a permission inferred from a model's recommendation.
/// Malformed desktop legacy rights are retained verbatim and grant explanation only.
/// Valid old rights remain distinct; missing/invalid rights never inherit a sibling's rights.
pub(crate) fn choice_contracts(
    project: &str,
    source_value: &Value,
    foundation_value: &Value,
    saved: &[Value],
) -> Result<Value, String> {
    let source: LockedSource = serde_json::from_value(source_value.clone())
        .map_err(|e| format!("选择缺当前锁定原文：{e}"))?;
    let foundation: FoundationProposal = serde_json::from_value(foundation_value.clone())
        .map_err(|e| format!("选择缺制作基础：{e}"))?;
    if foundation.source_sha256 != source.sha256
        || foundation_value["sourceVersion"] != source_value["version"]
    {
        return Err("制作基础不属于当前原文，请先完成制作基础".into());
    }
    let mut contracts: Vec<Value> = vec![];
    for issue in &foundation.issues {
        let Some(details) = issue
            .details
            .as_ref()
            .filter(|d| d["owner"] == "AUTHOR" && d["kind"] == "HUMAN_CHOICE")
        else {
            continue;
        };
        let choice = &details["choice"];
        let key = choice["decisionKey"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("历史选择缺decisionKey")?;
        let options = choice["options"]
            .as_array()
            .filter(|a| (2..=3).contains(&a.len()))
            .ok_or("历史选择选项无效")?;
        if options
            .iter()
            .any(|v| v.as_str().is_none_or(|s| s.trim().is_empty()))
            || options.iter().collect::<HashSet<_>>().len() != options.len()
        {
            return Err("历史选择须有2–3个不同文本选项".into());
        }
        let scope = choice_scope(choice, &issue.episode_ids, &source)?;
        let targets = choice_targets(&choice["changeTargets"], &foundation).ok();
        let refs = choice_strings(&choice["sourceRefs"], "选择sourceRefs")?;
        if refs.is_empty()
            || refs
                .iter()
                .any(|id| !source.rows.iter().any(|r| &r.id == id))
        {
            return Err("历史选择原文证据已失效".into());
        }
        let identity = json!([
            project,
            source_value["version"],
            source.sha256,
            choice_key(key),
            scope,
            targets,
            options
        ]);
        let id = format!("author:{}", digest(&identity.to_string()));
        let member = json!({"coordinate":issue.coordinate,"choiceSha256":digest(&choice.to_string()),"choice":choice});
        if let Some(existing) = contracts.iter_mut().find(|c| c["id"] == id) {
            if !existing["members"].as_array().unwrap().contains(&member) {
                existing["members"].as_array_mut().unwrap().push(member);
            }
            let old = existing["sourceRefs"].as_array_mut().unwrap();
            for r in refs {
                let r = json!(r);
                if !old.contains(&r) {
                    old.push(r);
                }
            }
            old.sort_by_key(Value::to_string);
            let subjects = existing["subjectAssetKeys"].as_array_mut().unwrap();
            for key in choice["changeTargets"]["assetKeys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|id| {
                    foundation
                        .assets
                        .iter()
                        .any(|a| Some(a.id.as_str()) == id.as_str())
                })
            {
                if !subjects.contains(key) {
                    subjects.push(key.clone());
                }
            }
            subjects.sort_by_key(Value::to_string);
        } else {
            contracts.push(json!({"id":id,"decisionKey":key,"status":"PENDING","sourceVersion":source_value["version"],"sourceSha256":source.sha256,"foundationVersion":foundation_value["version"],"question":choice["question"],"reason":choice["reason"],"options":options,"recommended":choice["recommended"],"recommendationReason":choice["recommendationReason"],"evidence":choice["evidence"],"sourceRefs":refs,"episodes":scope["episodes"],"applicationScope":scope,"changeTargets":targets,"legacyChangeTargets":if targets.is_none(){choice["changeTargets"].clone()}else{Value::Null},"interpretationOnly":targets.is_none(),"members":[member],"subjectAssetKeys":choice["changeTargets"]["assetKeys"].as_array().into_iter().flatten().filter(|id|foundation.assets.iter().any(|a|Some(a.id.as_str())==id.as_str())).collect::<Vec<_>>() }));
        }
    }
    for contract in &mut contracts {
        contract["revision"] = json!(digest(
            &json!([
                contract["id"],
                foundation_value["version"],
                contract["members"],
                contract["sourceRefs"]
            ])
            .to_string()
        ));
        if let Some(record) = saved.iter().find(|r| {
            r["id"] == contract["id"]
                && r["revision"] == contract["revision"]
                && r["status"] == "CONFIRMED"
        }) {
            *contract = record.clone();
        }
    }
    // Published applied receipts are authoritative after a crash between publication
    // and UI refresh. They also preserve historical legal choice bodies and evidence.
    for record in foundation_value["authorDecisions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            r["status"] == "APPLIED"
                && r["sourceSha256"] == source.sha256
                && r["sourceVersion"] == source_value["version"]
        })
    {
        if let Some(at) = contracts.iter().position(|c| c["id"] == record["id"]) {
            // A same-named choice with revised evidence is new model work. A
            // prior receipt cannot silently close the newly frozen issue.
            if same_choice_evidence(record, &contracts[at]) {
                contracts[at] = record.clone();
            }
        } else {
            contracts.push(record.clone());
        }
    }
    Ok(
        json!({"projectId":project,"sourceVersion":source_value["version"],"sourceSha256":source.sha256,"foundationVersion":foundation_value["version"],"contracts":contracts}),
    )
}

/// Only frozen, user-authored requirements request a human stop. A model's
/// AUTHOR label or recommendation is not an instruction to ask the user.
pub(crate) fn choice_requires_user_review(contract: &Value, requirements: &Value) -> bool {
    requirements
        .as_array()
        .into_iter()
        .flatten()
        .any(|requirement| {
            if let Some(key) = requirement.as_str() {
                return contract["id"].as_str() == Some(key)
                    || contract["decisionKey"]
                        .as_str()
                        .is_some_and(|k| choice_key(k) == choice_key(key));
            }
            if requirement["explicitReviewRequired"] != true
                && requirement["needsUserConfirmation"] != true
            {
                return false;
            }
            let identity = (!requirement["id"].is_null() && requirement["id"] == contract["id"])
                || requirement["decisionKey"].as_str().is_some_and(|key| {
                    contract["decisionKey"]
                        .as_str()
                        .is_some_and(|k| choice_key(k) == choice_key(key))
                });
            identity
                && ["sourceSha256", "sourceVersion", "applicationScope"]
                    .iter()
                    .all(|k| requirement.get(*k).is_none_or(|v| v == &contract[*k]))
        })
}

/// Plain AUTHOR choices are model work within Q1. Old manually entered answers
/// remain advisory evidence, without pretending the user required that answer.
pub(crate) fn automatic_decisions(
    view: &Value,
    requirements: &Value,
) -> Result<Vec<Value>, String> {
    if !requirements.is_null() && !requirements.is_array() {
        return Err("用户预先确认要求须为明确列表".into());
    }
    Ok(view["contracts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["status"] != "APPLIED" && !choice_requires_user_review(c, requirements))
        .map(|c| {
            let mut c = c.clone();
            c.as_object_mut().unwrap().remove("requiresUserReview");
            if let Some(answer) = c
                .get("answer")
                .cloned()
                .filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
            {
                c["priorAnswer"] = answer;
            }
            c.as_object_mut().unwrap().remove("answer");
            c["status"] = json!("MODEL_PENDING");
            // Automatic interpretation never grants source/identity mutation,
            // even when an old contract happened to name a legal metadata field.
            if !c["changeTargets"].is_null() {
                c["legacyChangeTargets"] = c["changeTargets"].clone();
            }
            c["changeTargets"] = Value::Null;
            c["interpretationOnly"] = json!(true);
            c["needsUserConfirmation"] = json!(false);
            c
        })
        .collect())
}

fn auto_view(
    source: &LockedSource,
    foundation: &FoundationProposal,
    context: &Value,
) -> Result<Value, String> {
    let mut source_value = json!(source);
    source_value["version"] = context.get("sourceVersion").cloned().unwrap_or(json!(1));
    let mut foundation_value = json!(foundation);
    foundation_value["sourceVersion"] = source_value["version"].clone();
    foundation_value["version"] = context
        .pointer("/foundation/version")
        .cloned()
        .unwrap_or(json!(0));
    foundation_value["authorDecisions"] = json!(foundation_auto_applied_decisions(foundation));
    choice_contracts(
        context["projectId"].as_str().unwrap_or("local"),
        &source_value,
        &foundation_value,
        &[],
    )
}

/// Nonblocking interpretation receipts are audit records, not SOURCE facts or
/// another review job. Subject metadata carries them into stages two and three.
pub(crate) fn foundation_auto_applied_decisions(foundation: &FoundationProposal) -> Vec<Value> {
    let mut records = vec![];
    for record in foundation
        .assets
        .iter()
        .flat_map(|a| {
            a.metadata
                .get("authorDecisions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .chain(foundation.issues.iter().filter_map(|i| {
            i.details
                .as_ref()
                .filter(|d| d["kind"] == "APPLIED_INTERPRETATION")
                .and_then(|d| d.get("authorDecision"))
        }))
    {
        if record["status"] == "APPLIED"
            && record["sourceSha256"] == foundation.source_sha256
            && !records.iter().any(|old: &Value| old["id"] == record["id"])
        {
            records.push(record.clone());
        }
    }
    records.sort_by_key(|r| r["id"].as_str().unwrap_or("").to_owned());
    records
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}
fn optional_obj(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn refs_schema() -> Value {
    array(string())
}
fn preparation_schema() -> Value {
    optional_obj(
        json!({"mode":choices(&["REFERENCE","DERIVE","INLINE","AUDIO","UNRESOLVED"]),"fromKey":string(),"note":string()}),
        &["mode"],
    )
}
fn asset_schema(patch: bool) -> Value {
    // Original light catalog: models declare identity and actual use. Rust binds
    // the source scope; a row-by-row evidence inventory is not F2/Q1's job.
    let variant = optional_obj(
        json!({"key":string(),"localKey":string(),"name":string(),"episodes":refs_schema(),"sceneRefs":array(array(string())),"note":string(),"preparation":preparation_schema(),"scope":optional_obj(json!({"kind":choices(&["SCENES","GLOBAL","UNRESOLVED"]),"reason":string()}), &["kind"])}),
        if patch {
            &[][..]
        } else {
            &["name", "episodes", "sceneRefs"][..]
        },
    );
    optional_obj(
        json!({"key":string(),"name":string(),"type":choices(&["CH","GROUP","BEAST","LOC","PROP"]),"aliases":refs_schema(),"episodes":refs_schema(),"sceneRefs":array(array(string())),"role":string(),"relationNote":string(),"description":string(),"preparation":preparation_schema(),"variants":array(variant),"scope":optional_obj(json!({"kind":choices(&["SCENES","GLOBAL","UNRESOLVED"]),"reason":string()}), &["kind"])}),
        if patch {
            &[][..]
        } else {
            &["name", "type", "episodes", "sceneRefs", "preparation"][..]
        },
    )
}
fn issue_schema() -> Value {
    optional_obj(
        json!({"coordinate":string(),"problem":string(),"sourceRefs":refs_schema(),"blocking":{"type":"boolean"},"assetId":string(),"variantId":string(),"episodeIds":refs_schema()}),
        &["coordinate", "problem", "sourceRefs", "blocking"],
    )
}
fn choice_schema() -> Value {
    optional_obj(
        json!({"decisionKey":string(),"kind":choices(&["SOURCE_CONFLICT","PRODUCTION_CHOICE"]),"question":string(),"reason":string(),"options":refs_schema(),"recommended":string(),"recommendationReason":string(),"evidence":string(),"episodes":refs_schema(),"sourceRefs":refs_schema(),"changeTargets":obj(json!({"assetKeys":refs_schema(),"assetFields":refs_schema(),"sceneFields":refs_schema()})),"applicationScope":optional_obj(json!({"kind":choices(&["GLOBAL","EPISODES","SCENES"]),"episodes":refs_schema(),"scenes":array(array(string()))}), &["kind"]),"evidenceRefs":array(obj(json!({"rowId":string()}))),"receivingUses":refs_schema()}),
        &[
            "decisionKey",
            "kind",
            "question",
            "reason",
            "options",
            "recommended",
            "recommendationReason",
            "evidence",
            "episodes",
            "sourceRefs",
        ],
    )
}
fn pending_schema() -> Value {
    optional_obj(
        json!({"owner":choices(&["Q1","PROGRAM","STAGE2"]),"key":string(),"episodes":refs_schema(),"note":string(),"sourceRefs":refs_schema()}),
        &["owner", "episodes", "note", "sourceRefs"],
    )
}
fn f2_schema() -> Value {
    optional_obj(
        json!({"format":choices(&["ASSET_CATALOG_DESKTOP_V2"]),"sourceSha256":string(),"coverage":refs_schema(),"assets":array(asset_schema(false)),"issues":array(issue_schema()),"humanChoices":array(choice_schema()),"unresolved":array(pending_schema()),"notes":refs_schema()}),
        &["format", "sourceSha256", "coverage", "assets"],
    )
}
fn q1_schema() -> Value {
    let scene = optional_obj(
        json!({"episodeId":string(),"sceneId":string(),"location":string(),"timeOfDay":string(),"interiorExterior":string(),"timeLayer":string(),"weather":string(),"characters":refs_schema()}),
        &["episodeId", "sceneId"],
    );
    let split = obj(
        json!({"episodeId":string(),"sceneId":string(),"cuts":array(obj(json!({"rowId":string(),"anchor":string()})))}),
    );
    let episode = optional_obj(
        json!({"episodeId":string(),"summary":string(),"context":string()}),
        &["episodeId"],
    );
    let merge = obj(
        json!({"fromKey":string(),"intoKey":string(),"sourceRefs":refs_schema(),"reason":string()}),
    );
    let signal =
        obj(json!({"episodeId":string(),"rowId":string(),"quote":string(),"reading":string()}));
    optional_obj(
        json!({"format":choices(&["ASSET_OPTIMIZATION_DESKTOP_V2"]),"sourceSha256":string(),"coverage":refs_schema(),"changes":optional_obj(json!({"assets":array(asset_schema(true)),"removeAssetKeys":refs_schema(),"mergeAssets":array(merge),"episodes":array(episode),"overview":string(),"scenes":array(scene),"sceneSplits":array(split)}), &[]),"issues":array(issue_schema()),"resolvedIssues":refs_schema(),"seriesDirection":optional_obj(json!({"signals":array(signal),"content":string()}), &[]),"humanChoices":array(choice_schema()),"unresolved":array(pending_schema()),"notes":refs_schema()}),
        &["format", "sourceSha256", "coverage", "changes"],
    )
}

#[derive(Clone, Debug)]
struct Piece {
    row_id: String,
    start: usize,
    end: usize,
    text: String,
}
#[derive(Clone, Debug)]
struct Batch {
    scope: Vec<String>,
    pieces: Vec<Piece>,
    fragment: Option<Value>,
}
impl Batch {
    fn row_ids(&self) -> HashSet<&str> {
        self.pieces.iter().map(|p| p.row_id.as_str()).collect()
    }
    fn key(&self) -> String {
        digest(
            &json!([
                self.scope,
                self.pieces
                    .iter()
                    .map(|p| (&p.row_id, p.start, p.end))
                    .collect::<Vec<_>>()
            ])
            .to_string(),
        )
    }
}
fn whole_rows(source: &LockedSource, scope: &[String]) -> Vec<Piece> {
    source
        .rows
        .iter()
        .filter(|r| r.episode_id.as_ref().is_some_and(|e| scope.contains(e)))
        .map(|r| Piece {
            row_id: r.id.clone(),
            start: r.range.start,
            end: r.range.end,
            text: r.text.clone(),
        })
        .collect()
}
fn split_episode(source: &LockedSource, episode: &str) -> Result<Vec<Batch>, String> {
    let rows = whole_rows(source, &[episode.into()]);
    let text: String = rows.iter().map(|r| r.text.as_str()).collect();
    let mut spans = vec![];
    let mut from = 0;
    let length = units(&text);
    while from < length {
        let target = (from + FRAGMENT_BUDGET).min(length);
        let mut end = from;
        let mut cursor = 0;
        let mut newline = None;
        for ch in text.chars() {
            let next = cursor + ch.len_utf16();
            if next > target {
                break;
            }
            if next > from {
                end = next;
            }
            if ch == '\n' && next >= from + FRAGMENT_BUDGET / 2 {
                newline = Some(next);
            }
            cursor = next;
        }
        if target < length {
            if let Some(at) = newline {
                end = at;
            } else if end > from
                && slice_utf16(
                    &text,
                    &SourceRange {
                        start: end - 1,
                        end,
                    },
                )
                .ok()
                .as_deref()
                    == Some("\r")
            {
                end -= 1;
            }
        }
        if end <= from {
            return Err("长集分片无法取得完整 Unicode 边界".into());
        }
        spans.push((from, end));
        from = end;
    }
    let mut result = vec![];
    for (index, (start, end)) in spans.iter().copied().enumerate() {
        let mut offset = 0;
        let mut pieces = vec![];
        for row in &rows {
            let limit = offset + units(&row.text);
            let a = start.max(offset);
            let b = end.min(limit);
            if b > a {
                pieces.push(Piece {
                    row_id: row.row_id.clone(),
                    start: row.start + a - offset,
                    end: row.start + b - offset,
                    text: slice_utf16(
                        &row.text,
                        &SourceRange {
                            start: a - offset,
                            end: b - offset,
                        },
                    )?,
                });
            }
            offset = limit;
        }
        let fragment = json!({"episodeId":episode,"index":index,"count":spans.len(),"start":start,"end":end,"sourceSha256":source.sha256});
        result.push(Batch {
            scope: vec![episode.into()],
            pieces,
            fragment: Some(fragment),
        });
    }
    Ok(result)
}
fn plan_batches(source: &LockedSource, selected: &[String]) -> Result<Vec<Batch>, String> {
    let mut result = vec![];
    let mut pending = vec![];
    let mut size = 0;
    for episode in source.episodes.iter().filter(|e| selected.contains(&e.id)) {
        let rows = whole_rows(source, &[episode.id.clone()]);
        let count = rows.iter().map(|p| units(&p.text)).sum::<usize>();
        if count > SOURCE_BUDGET {
            if !pending.is_empty() {
                result.push(Batch {
                    pieces: whole_rows(source, &pending),
                    scope: std::mem::take(&mut pending),
                    fragment: None,
                });
                size = 0;
            }
            result.extend(split_episode(source, &episode.id)?);
        } else {
            if !pending.is_empty() && size + count > SOURCE_BUDGET {
                result.push(Batch {
                    pieces: whole_rows(source, &pending),
                    scope: std::mem::take(&mut pending),
                    fragment: None,
                });
                size = 0;
            }
            pending.push(episode.id.clone());
            size += count;
        }
    }
    if !pending.is_empty() {
        result.push(Batch {
            pieces: whole_rows(source, &pending),
            scope: pending,
            fragment: None,
        });
    }
    verify_coverage(source, selected, &result)?;
    Ok(result)
}
fn verify_coverage(
    source: &LockedSource,
    selected: &[String],
    batches: &[Batch],
) -> Result<(), String> {
    for row in source
        .rows
        .iter()
        .filter(|r| r.episode_id.as_ref().is_some_and(|e| selected.contains(e)))
    {
        let mut pieces = batches
            .iter()
            .flat_map(|b| &b.pieces)
            .filter(|p| p.row_id == row.id)
            .collect::<Vec<_>>();
        pieces.sort_by_key(|p| p.start);
        let mut end = row.range.start;
        let mut restored = String::new();
        for part in pieces {
            if part.start != end || part.end <= part.start {
                return Err("来源分批存在遗漏、重叠或乱序".into());
            }
            end = part.end;
            restored.push_str(&part.text);
        }
        if end != row.range.end || restored != row.text {
            return Err("来源分批未完整逐字覆盖锁定行".into());
        }
    }
    Ok(())
}

/// Names/aliases only retrieve evidence. They never determine canonical identity or current state.
fn evidence(source: &LockedSource, batch: &Batch, foundation: &FoundationProposal) -> Value {
    let current: String = batch.pieces.iter().map(|p| p.text.as_str()).collect();
    let mut names: BTreeSet<String> = source
        .rows
        .iter()
        .filter_map(|r| r.dialogue.as_ref())
        .flat_map(|d| dialogue_speaker_names(d).into_iter().map(str::to_owned))
        .collect();
    for asset in &foundation.assets {
        names.insert(asset.name.clone());
        names.extend(asset.aliases.iter().cloned());
        if let Some((name, _)) = asset.name.split_once(['|', '｜']) {
            names.insert(name.into());
        }
    }
    names.retain(|name| !name.is_empty() && current.contains(name));
    for asset in &foundation.assets {
        if names.contains(&asset.name) || asset.aliases.iter().any(|a| names.contains(a)) {
            names.insert(asset.name.clone());
            names.extend(asset.aliases.iter().cloned());
        }
    }
    let relevant = |text: &str| names.iter().any(|name| text.contains(name));
    let earliest = source
        .episodes
        .iter()
        .filter(|e| batch.scope.contains(&e.id))
        .map(|e| e.number)
        .min()
        .unwrap_or(0);
    let history: Vec<_> = source
        .rows
        .iter()
        .filter(|r| {
            r.episode_id.as_ref().is_some_and(|id| {
                source
                    .episodes
                    .iter()
                    .any(|e| &e.id == id && e.number < earliest)
            }) && r.kind != "format"
        })
        .collect();
    let anchors: HashSet<_> = history
        .iter()
        .filter(|r| relevant(&r.text))
        .map(|r| r.id.as_str())
        .collect();
    let mut candidates = vec![];
    for (at, row) in history.iter().enumerate() {
        let neighbours = [
            at.checked_sub(1).and_then(|i| history.get(i)),
            history.get(at + 1),
        ]
        .into_iter()
        .flatten()
        .filter(|r| r.episode_id == row.episode_id && anchors.contains(r.id.as_str()))
        .map(|r| r.id.as_str())
        .collect::<Vec<_>>();
        if anchors.contains(row.id.as_str()) || !neighbours.is_empty() {
            candidates.push(json!([
                row.id,
                row.episode_id,
                row.scene_id,
                row.text,
                if row.dialogue.is_some() {
                    "CHARACTER_CLAIM"
                } else {
                    "SOURCE"
                },
                if anchors.contains(row.id.as_str()) {
                    "SUBJECT_MENTION"
                } else {
                    "SOURCE_NEIGHBOUR"
                },
                neighbours
            ]));
        }
    }
    let mut background = vec![];
    let mut unlinked = vec![];
    let mut omitted = vec![];
    let mut count = 0;
    for row in source.rows.iter().filter(|r| r.kind == "context") {
        let explicit = row.dialogue.is_none()
            && [
                "规则：",
                "规则:",
                "世界规则：",
                "世界观规则：",
                "世界观设定：",
                "世界设定：",
            ]
            .iter()
            .any(|prefix| row.text.trim_start().starts_with(prefix));
        let entry = json!([
            row.id,
            row.text,
            if row.dialogue.is_some() {
                "CHARACTER_CLAIM"
            } else {
                "SOURCE"
            },
            if explicit {
                "EXPLICIT_GLOBAL_DECLARATION"
            } else {
                "UNRESOLVED"
            }
        ]);
        if explicit || relevant(&row.text) {
            background.push(entry);
        } else if count + units(&row.text) <= BACKGROUND_CANDIDATES_BUDGET {
            count += units(&row.text);
            unlinked.push(entry);
        } else {
            omitted.push(json!([row.id, row.number, row.range.start, row.range.end]));
        }
    }
    json!({"schema":"FOUNDATION_EVIDENCE_DESKTOP_V2","backgroundColumns":["rowId","text","claim","applicability"],"background":background,"backgroundCandidates":unlinked,"backgroundCandidatesOmitted":omitted,"historicalColumns":["rowId","episodeId","sceneId","text","claim","retrievalMode","anchorRowIds"],"historicalEvidenceCandidates":candidates,"pendingIssues":foundation.issues.iter().filter(|issue|issue.episode_ids.is_empty()||issue.episode_ids.iter().any(|id|batch.scope.contains(id))).collect::<Vec<_>>(),"candidateUse":"HISTORICAL_EVIDENCE_ONLY","currentApplicability":"UNKNOWN","rule":"姓名/别名/邻句仅检索；不证明同身份、知情、时序或状态永久延续。台词只是CHARACTER_CLAIM。匿名资料不是GLOBAL；未来剧情不提前应用。pendingIssues仅本批关联待办，Q1实际局部修正才回resolvedIssues坐标；PROGRAM事项不能由模型仅回执关闭。"})
}
fn registry(foundation: &FoundationProposal) -> Value {
    json!(foundation.assets.iter().map(|a|json!({"key":a.id,"name":a.metadata.get("catalogName").cloned().unwrap_or(json!(a.name)),"type":a.kind,"aliases":a.aliases,"variants":a.variants.iter().map(|v|json!({"key":v.id,"name":v.name,"fromKey":v.source_variant_id})).collect::<Vec<_>>()})).collect::<Vec<_>>())
}
fn catalogue(foundation: &FoundationProposal, batch: &Batch) -> Value {
    let row_ids = batch.row_ids();
    json!(foundation.assets.iter().filter(|a|a.variants.iter().any(|v|v.scope.kind=="GLOBAL"||v.scope.scenes.iter().any(|s|batch.scope.contains(&s.episode_id)))||a.source_refs.iter().any(|id|row_ids.contains(id.as_str()))).map(|a| {
        let mut value=json!({"key":a.id,"name":a.metadata.get("catalogName").cloned().unwrap_or(json!(a.name)),"type":a.kind,"aliases":a.aliases,"sourceRefs":a.source_refs,"variants":a.variants.iter().map(|v|{
            let mut item=json!({"key":v.id,"name":v.name,"sourceRefs":v.source_refs,"sceneRefs":v.scope.scenes.iter().filter(|s|batch.scope.contains(&s.episode_id)).map(|s|json!([s.episode_id,s.scene_id])).collect::<Vec<_>>(),"scope":{"kind":v.scope.kind,"reason":v.scope.reason},"fromKey":v.source_variant_id});
            if let Some(map)=item.as_object_mut(){for (key,val) in &v.metadata {map.insert(key.clone(),val.clone());}} item
        }).collect::<Vec<_>>()});
        if let Some(map)=value.as_object_mut(){for (key,val) in &a.metadata {map.insert(key.clone(),val.clone());}} value
    }).collect::<Vec<_>>())
}
fn input_for_legacy(
    stage: &str,
    source: &LockedSource,
    batch: &Batch,
    foundation: &FoundationProposal,
    context: &Value,
    index: usize,
    total: usize,
    final_batch: bool,
    direction_signals: &[Value],
) -> Value {
    let ids = batch.row_ids();
    let source_rows: HashMap<_, _> = source
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    let rows: Vec<_> = batch
        .pieces
        .iter()
        .map(|piece| {
            let row = source_rows[piece.row_id.as_str()];
            json!([
                piece.row_id,
                row.number,
                row.episode_id,
                row.scene_id,
                row.kind,
                row.dialogue.as_ref().map(|d| d.speaker.as_str()),
                row.dialogue.as_ref().map(|d| d.channel.as_str()),
                piece.start,
                piece.end,
                piece.text
            ])
        })
        .collect();
    json!({"method":METHOD_VERSION,"catalogueContract":"light-catalogue/v1","sourceReferenceEncoding":"row-coordinates/v1","sourceSha256":source.sha256,"scope":batch.scope,"batch":batch.key(),"batchIndex":index,"delivery":{"method":METHOD_VERSION,"stage":stage,"phase":if stage=="F2"{"GENERATE_F2"}else{"EDIT"},"scope":batch.scope,"batchIndex":index,"batchCount":total,"fragment":batch.fragment},"source":{"columns":["rowId","line","episodeId","sceneId","kind","speaker","channel","start","end","text"],"coordinateSystem":"UTF-16 half-open","rows":rows},"scenes":source.scenes.iter().filter(|s|s.row_ids.iter().any(|id|ids.contains(id.as_str()))).map(|s|json!([s.episode_id,s.id,s.title])).collect::<Vec<_>>(),"registry":registry(foundation),"candidates":if stage=="Q1"{catalogue(foundation,batch)}else{json!(null)},"evidence":evidence(source,batch,foundation),"humanChoiceRegistry":foundation.issues.iter().filter_map(|issue|issue.details.as_ref().and_then(|details|details.get("choice"))).collect::<Vec<_>>(),"transportInstructions":"schema使用桌面固定format；episodes/coverage为输入episodeId，sceneRefs为[episodeId,sceneId]。资产/形象及普通集场修订沿旧轻量目录，不填写sourceRefs；Rust绑定实际sourceSha和声明的集场范围，范围资料不是外观事实。真正事项证据、方向与拆场rowId只选schema枚举的r:start:end，与输入第1列的row:源前缀:start:end逐个对应；程序绑定当前sourceSha还原，不重算或拼接start/end。humanChoices保留旧完整问题/方案/推荐/权限及必要sourceRefs；推荐不是已确认，技术错误不得包装为取舍。程序保存choice到issues.details，普通notes不阻塞；仅有据源矛盾阻塞关联范围。unresolved使用owner/episodes/note/sourceRefs。Q1省略/null保留字段，只交差异，不把PROGRAM计划声称已执行。","projectConditions":context.get("settings").cloned().unwrap_or(json!({})),"seriesDirectionPreparation":if stage=="Q1"{json!({"mode":if final_batch{"FINAL"}else{"SIGNALS"},"priorSignals":direction_signals,"priorDirection":foundation.direction,"confirmedDirection":context.pointer("/settings/confirmedDirection"),"scope":context.pointer("/selection/episodeIds")})}else{json!(null)}})
}
// A model projection, never the saved catalogue. Like the original production
// contract, usage is local and unlocated definitions remain separate candidates.
fn light_catalogue(foundation: &FoundationProposal, batch: &Batch) -> (Value, Value) {
    fn program_scope(meta: &Map<String, Value>, sha: &str) -> bool {
        meta.get("sourceRefsPurpose").and_then(Value::as_str) == Some("SOURCE_SCOPE")
            && meta.get("sourceScope").is_some_and(|scope| {
                scope["kind"] == "PROGRAM_SCOPE" && scope["sourceSha256"] == sha
            })
    }
    fn metadata(meta: &Map<String, Value>, refs: &[String], sha: &str) -> Map<String, Value> {
        let mut out = meta.clone();
        for key in [
            "catalogName",
            "usageEpisodes",
            "key",
            "name",
            "type",
            "aliases",
            "variants",
            "sceneRefs",
            "scope",
            "fromKey",
        ] {
            out.remove(key);
        }
        if program_scope(meta, sha) {
            out.remove("sourceScope");
            out.remove("sourceRefsPurpose");
            out.remove("sourceRefs");
        } else if !refs.is_empty() {
            out.insert("sourceRefs".into(), json!(refs));
        }
        out
    }
    fn variant(v: &FoundationVariant, batch: &Batch, sha: &str) -> Value {
        let mut value = Value::Object(metadata(&v.metadata, &v.source_refs, sha));
        value["key"] = json!(v.id);
        value["name"] = json!(v.name);
        value["fromKey"] = json!(v.source_variant_id);
        value["scope"] = json!({"kind":v.scope.kind,"reason":v.scope.reason});
        let scenes: Vec<_> = v
            .scope
            .scenes
            .iter()
            .filter(|s| batch.scope.contains(&s.episode_id))
            .collect();
        value["sceneRefs"] = json!(scenes
            .iter()
            .map(|s| json!([s.episode_id, s.scene_id]))
            .collect::<Vec<_>>());
        value["episodes"] = json!(batch
            .scope
            .iter()
            .filter(|ep| v.scope.kind == "GLOBAL" || scenes.iter().any(|s| &s.episode_id == *ep))
            .collect::<Vec<_>>());
        if !program_scope(&v.metadata, sha) && !v.description.is_empty() {
            value["description"] = json!(v.description);
        }
        value
    }
    let mut candidates = vec![];
    let mut unlocated = vec![];
    for a in &foundation.assets {
        let local = |v: &FoundationVariant| {
            v.scope.kind == "GLOBAL"
                || v.scope
                    .scenes
                    .iter()
                    .any(|s| batch.scope.contains(&s.episode_id))
        };
        let located = a
            .variants
            .iter()
            .any(|v| v.scope.kind == "GLOBAL" || !v.scope.scenes.is_empty());
        let mut value = Value::Object(metadata(
            &a.metadata,
            &a.source_refs,
            &foundation.source_sha256,
        ));
        value["key"] = json!(a.id);
        value["name"] = a
            .metadata
            .get("catalogName")
            .cloned()
            .unwrap_or(json!(a.name));
        value["type"] = json!(a.kind);
        value["aliases"] = json!(a.aliases);
        value["episodes"] =
            json!(batch
                .scope
                .iter()
                .filter(|ep| a.variants.iter().any(|v| v.scope.kind == "GLOBAL"
                    || v.scope.scenes.iter().any(|s| &s.episode_id == *ep)))
                .collect::<Vec<_>>());
        if !program_scope(&a.metadata, &foundation.source_sha256) && !a.description.is_empty() {
            value["description"] = json!(a.description);
        }
        value["variants"] = json!(a
            .variants
            .iter()
            .filter(|v| local(v) || v.metadata.get("isBase") == Some(&json!(true)))
            .map(|v| variant(v, batch, &foundation.source_sha256))
            .collect::<Vec<_>>());
        value["crossBatchVariants"] = json!(a.variants.iter().filter(|v|!local(v)&&v.metadata.get("isBase")!=Some(&json!(true))).map(|v|json!({"key":v.id,"name":v.name,"fromKey":v.source_variant_id,"preparation":v.metadata.get("preparation")})).collect::<Vec<_>>());
        if a.variants.iter().any(local) {
            candidates.push(value.clone());
        }
        if !located {
            // The entire unlocated definition remains available, including
            // child design notes; it still grants no usage in this batch.
            value["variants"] = json!(a
                .variants
                .iter()
                .map(|v| variant(v, batch, &foundation.source_sha256))
                .collect::<Vec<_>>());
            value["crossBatchVariants"] = json!([]);
            value["baseKey"] = json!(a.id);
            value["kind"] = json!("BASE");
            value["use"] = json!("CANDIDATE_ONLY");
            value["reason"] = json!("UNLOCATED_USAGE");
            unlocated.push(value);
        } else {
            for v in a
                .variants
                .iter()
                .filter(|v| v.scope.kind != "GLOBAL" && v.scope.scenes.is_empty())
            {
                let mut item = variant(v, batch, &foundation.source_sha256);
                item["baseKey"] = json!(a.id);
                item["baseName"] = json!(a.name);
                item["type"] = json!(a.kind);
                item["kind"] = json!("VARIANT");
                item["use"] = json!("CANDIDATE_ONLY");
                item["reason"] = json!("UNLOCATED_USAGE");
                unlocated.push(item);
            }
        }
    }
    (json!(candidates), json!(unlocated))
}

fn input_for(
    stage: &str,
    source: &LockedSource,
    batch: &Batch,
    foundation: &FoundationProposal,
    context: &Value,
    index: usize,
    total: usize,
    final_batch: bool,
    direction_signals: &[Value],
) -> Value {
    let mut input = input_for_legacy(
        stage,
        source,
        batch,
        foundation,
        context,
        index,
        total,
        final_batch,
        direction_signals,
    );
    if stage == "Q1"
        && context["foundationDecisionPolicy"] == AUTO_INTERPRET_POLICY
        && context["foundationPhase"] != "APPLY_DECISION"
    {
        let requirements = context
            .get("userReviewRequirements")
            .cloned()
            .unwrap_or(json!([]));
        let contracts = if context["foundationPhase"] == "AUTO_INTERPRET" {
            context["automaticDecisions"]
                .as_array()
                .cloned()
                .ok_or_else(|| "自动制作解释缺冻结契约".to_owned())
        } else {
            auto_view(source, foundation, context)
                .and_then(|view| automatic_decisions(&view, &requirements))
        };
        match contracts {
            Ok(decisions) => {
                input["automaticDecisions"] = json!(decisions
                    .into_iter()
                    .filter(|d| d["episodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|e| batch.scope.iter().any(|id| Some(id.as_str()) == e.as_str())))
                    .map(|d| {
                        let mut projected = Value::Object(
                            [
                                "id",
                                "decisionKey",
                                "revision",
                                "status",
                                "sourceSha256",
                                "sourceVersion",
                                "question",
                                "reason",
                                "options",
                                "recommended",
                                "recommendationReason",
                                "priorAnswer",
                                "evidence",
                                "sourceRefs",
                                "episodes",
                                "applicationScope",
                                "interpretationOnly",
                                "subjectAssetKeys",
                            ]
                            .into_iter()
                            .filter_map(|k| d.get(k).map(|v| (k.to_owned(), v.clone())))
                            .collect(),
                        );
                        if let Some(old) = prior_auto_records(foundation)
                            .iter()
                            .find(|r| same_choice_evidence(r, &d))
                        {
                            projected["previousAppliedAnswer"] = old["answer"].clone();
                            projected["previousSubjectAssetKeys"] = old["subjectAssetKeys"].clone();
                        }
                        projected
                    })
                    .collect::<Vec<_>>())
            }
            Err(error) => input["automaticDecisionError"] = json!(error),
        }
        input["foundationDecisionPolicy"] = json!(AUTO_INTERPRET_POLICY);
        input["userReviewRequirements"] = requirements;
        input["projectId"] = context.get("projectId").cloned().unwrap_or(json!("local"));
        input["sourceVersion"] = context.get("sourceVersion").cloned().unwrap_or(json!(1));
        input["foundationVersion"] = context
            .pointer("/foundation/version")
            .cloned()
            .unwrap_or(json!(0));
        // Decision support may legitimately come from another scene/episode.
        // Send only missing frozen evidence; existing request bytes stay exact
        // when the source/history already includes every support row.
        let provided = allowed_refs(&input);
        let missing: HashSet<_> = input["automaticDecisions"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|decision| decision["sourceRefs"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .filter(|id| !provided.contains(*id))
            .map(str::to_owned)
            .collect();
        if !missing.is_empty() {
            input["evidence"]["automaticDecisionSupport"] = json!({"columns":["rowId","line","episodeId","sceneId","kind","speaker","channel","start","end","text","claim"],"use":"DECISION_SUPPORT_ONLY","currentApplicability":"UNKNOWN","rows":source.rows.iter().filter(|row|missing.contains(row.id.as_str())).map(|row|json!([row.id,row.number,row.episode_id,row.scene_id,row.kind,row.dialogue.as_ref().map(|d|&d.speaker),row.dialogue.as_ref().map(|d|&d.channel),row.range.start,row.range.end,row.text,if row.dialogue.is_some(){"CHARACTER_CLAIM"}else{"SOURCE"}])).collect::<Vec<_>>()});
        }
        if let Ok(view) = auto_view(source, foundation, context) {
            input["humanChoiceRegistry"] = json!(view["contracts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| d["status"] == "APPLIED"
                    || choice_requires_user_review(d, &input["userReviewRequirements"]))
                .map(|d| Value::Object(
                    [
                        "id",
                        "decisionKey",
                        "status",
                        "question",
                        "answer",
                        "evidence",
                        "sourceRefs",
                        "episodes",
                        "applicationScope",
                        "subjectAssetKeys"
                    ]
                    .into_iter()
                    .filter_map(|key| d.get(key).map(|v| (key.to_owned(), v.clone())))
                    .collect()
                ))
                .collect::<Vec<_>>());
        }
        if context["foundationPhase"] == "AUTO_INTERPRET" {
            input["delivery"]["phase"] = json!("AUTO_INTERPRET");
            input["seriesDirectionPreparation"] = Value::Null;
        }
        let guide = input["transportInstructions"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        // Automatic interpretation is an authority policy, not a replacement
        // for the original editor. Fresh EDIT still completes the catalogue in
        // this same Q1 reply; only a completed-foundation AUTO_INTERPRET run is
        // restricted to adopting an interpretation without catalogue changes.
        let phase_rule = if context["foundationPhase"] == "AUTO_INTERPRET" {
            "AUTO_INTERPRET只采用解释及关闭同证据同范围Q1待办，changes必须为空，不重跑F2、不修改目录/方向/源事实。"
        } else {
            "本次phase为EDIT，沿原Q1直接终编本批目录：自动取舍和实际修正必须在同一回答完成，不另设解释岗。制作解释若影响现有资产的名称、描述、preparation或形象，按旧Q1授权交changes.assets沿现有key的真实差异；UNRESOLVED制作方式须依已选解释与原文依据落实，不能只记appliedDecisions而保留待定目录后关闭事项。原文矛盾的声明/对白仍逐字保留，目录中的制作解释不升级为源事实，不凭此改说者、台词或动作。只有对应事项实际改好或现稿已满足，才交resolvedIssues；PROGRAM事项不能凭回执关闭。"
        };
        input["transportInstructions"]=json!(format!("{guide} 当前用户授权普通AUTHOR制作取舍由本次原Q1自动完成，不逐条提问、不新增岗位。automaticDecisions不是用户已确认答案；priorAnswer仅是旧人工输入参考，按锁定原文自行选择最优且保守的制作解释，每项appliedDecisions交准确id、非空answer和note实际采用依据；previousAppliedAnswer/previousSubjectAssetKeys是前容量批已采用解释和主体，后批逐字沿用并补本批应用依据。humanChoiceRegistry中APPLIED为同源已采用解释，不重复提问或当作SOURCE事实。新发现普通取舍仍用原humanChoices完整证据/范围登记，同答appliedDecisions用其decisionKey并交answer/note。只在userReviewRequirements预先明确指定的事项上保留用户确认；模型自称需要用户确认无效。制作解释不是原文事实，不改变原句/说者/对白/动作，不创造人物、身份或无据别名；颜色/数量/称呼矛盾可明确区分原文声明和镜头制作解释。解释必须对应当前资产主体和精确集场，缺主体时appliedDecisions.subjectAssetKeys只选输入现有key，不按名字合并。{phase_rule}"));
    }
    if stage == "Q1" && context["foundationPhase"] == "APPLY_DECISION" {
        let decisions: Vec<_> = context["confirmedDecisions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| {
                d["episodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|e| batch.scope.iter().any(|id| Some(id.as_str()) == e.as_str()))
            })
            .map(|d| {
                Value::Object(
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
                        "changeTargets",
                        "interpretationOnly",
                        "subjectAssetKeys",
                    ]
                    .into_iter()
                    .filter_map(|k| d.get(k).map(|v| (k.to_owned(), v.clone())))
                    .collect(),
                )
            })
            .collect();
        input["delivery"]["phase"] = json!("APPLY_DECISION");
        input["confirmedDecisions"] = json!(decisions);
        input["humanChoiceRegistry"] = json!(decisions);
        if let Some(issues) = input
            .pointer_mut("/evidence/pendingIssues")
            .and_then(Value::as_array_mut)
        {
            issues.retain(|issue| {
                issue.pointer("/details/owner").and_then(Value::as_str) != Some("AUTHOR")
            });
        }
        input["decisionMutationRule"]=json!("同时服从applicationScope和changeTargets，精确使用confirmedDecisions.id，不修改answer。interpretationOnly没有任何资产/场次修改权限，只把已确认解释用于后续制作并交采用回执；原文、说者标签、台词、动作逐字不变。共享对象顶层字段只有全部用途在授权范围时才能改；局部状态须获variants权限且保留无关形象。只输出实际差异。禁止删除、归并、拆场、重编分集或改全剧方向。实际采用或现稿已满足后交appliedDecisions准确id和非空依据，收到不等于采用。必要Q1待办只有同一解释的原文证据和范围明确覆盖时才可交resolvedIssues；PROGRAM事项不能关闭。 ");
        let guide = input["transportInstructions"].as_str().unwrap_or("");
        input["transportInstructions"]=json!(format!("{guide} APPLY_DECISION沿原Q1采用已确认制作解释，appliedDecisions须含本批每个已提供决定的准确id/note；不重复询问已确认问题，不再提取F2。"));
        input["seriesDirectionPreparation"] = json!(null);
    }
    super::asset_coverage::foundation_input(stage, &mut input, context);
    // New source metadata only; legacy sources have no joint members and their
    // complete frozen input remains byte-identical.
    let allowed = allowed_refs(&input);
    let groups: Vec<_> = source.rows.iter().filter(|row| allowed.contains(&row.id))
        .filter_map(|row| row.dialogue.as_ref().filter(|q| !q.speakers.is_empty())
            .map(|q| json!({"rowId":row.id,"lineId":q.id,"speaker":q.speaker,"speakers":q.speakers}))).collect();
    if !groups.is_empty() {
        input["jointSpeakerProfile"] = json!("source-dialogue-members/v1");
        input["jointSpeakers"] = json!(groups);
        let guide = input["transportInstructions"].as_str().unwrap_or("").to_owned();
        input["transportInstructions"] = json!(format!("{guide} jointSpeakers是程序根据原文明确人物名单识别的共同说者：speaker完整保留原标签，speakers各text与range是分别登记的原文成员，不是一个新人物、群像资产或互为别名。按每个成员分别识别并沿用正式身份、适用形象及历史资料，不按连接标签创建或合并资产；共同的一句原话保持一次，不按成员人数复制。"));
    }
    if context["foundationModelInputProfile"] != super::foundation_wire::PROFILE {
        return input; // Frozen tasks and accepted cache retain their original wire.
    }
    input["modelInputProfile"] = json!(super::foundation_wire::PROFILE);
    input["registry"]=json!(foundation.assets.iter().map(|a|json!({"key":a.id,"name":a.metadata.get("catalogName").cloned().unwrap_or(json!(a.name)),"type":a.kind,"aliases":a.aliases,"variants":a.variants.iter().map(|v|json!({"key":v.id,"name":v.name,"fromKey":v.source_variant_id,"preparation":v.metadata.get("preparation")})).collect::<Vec<_>>()})).collect::<Vec<_>>());
    if stage == "Q1" {
        let (candidates, unlocated) = light_catalogue(foundation, batch);
        let local_keys: HashSet<_> = candidates
            .as_array()
            .unwrap()
            .iter()
            .chain(unlocated.as_array().unwrap())
            .filter(|a| a["kind"] != "VARIANT")
            .filter_map(|a| a.get("baseKey").or_else(|| a.get("key")))
            .filter_map(Value::as_str)
            .collect();
        input["registry"]
            .as_array_mut()
            .unwrap()
            .retain(|a| !local_keys.contains(a["key"].as_str().unwrap_or("")));
        input["candidates"] = candidates;
        input["materialCandidates"] = unlocated;
    }
    let guide = input["transportInstructions"].as_str().unwrap().to_owned();
    input["transportInstructions"]=json!(format!("{guide} 本批candidates仅本批适用形象及独立base形象；crossBatchVariants仅用于身份/制作来源识别，不表示本批出场。materialCandidates未定位，不能当成本批出场或GLOBAL。SOURCE_SCOPE程序范围引用从轻目录省略，真正视觉证据保留。F2重整已登记key时沿用registry中的preparation/fromKey；需要改变既有制作来源应提出关联问题，不重新推断覆盖来源。Q1仍按旧规约交明确差异修订。"));
    input
}

pub(crate) const MODEL_TRANSPORT_LIMIT: usize = REQUEST_BUDGET - OUTPUT_RESERVE;
fn request_chars(stage: &str, input: &Value, schema: &Value) -> Result<usize, String> {
    // Unlike the old schema:null CLI request, desktop output-schema is actual input.
    // Account for the exact strict output-schema wire without changing the domain
    // schema saved in a TextRequest or any previously accepted cache identity.
    let exact_schema = if schema.get("$defs").is_some() {
        schema.clone()
    } else {
        delivery_schema(stage, input)
    };
    if input["modelInputProfile"] == super::foundation_wire::PROFILE {
        let compiled =
            super::foundation_wire::compile(&format!("foundation:{stage}"), input, &exact_schema)?;
        let skill = if stage == "F2" { F2_RULE } else { Q1_RULE };
        let request = TextRequest {
            stage: format!("foundation:{stage}"),
            skill: skill.into(),
            model: "m".repeat(128),
            reasoning: "ultra".into(),
            input: input.clone(),
            schema: exact_schema,
            rule_snapshot: snapshot(skill, &[])?,
        };
        return Ok(crate::text_executor::transport_payload_characters(
            &request,
            &compiled.input,
            &compiled.schema,
        ));
    }
    // Frozen legacy requests retain their original planning and cache identity.
    let wire_schema = crate::text_executor::structured_schema_for_budget(&exact_schema);
    let request = json!({"stage":format!("foundation:{stage}"),"skill":if stage=="F2"{F2_RULE}else{Q1_RULE},"rules":snapshot(if stage=="F2"{F2_RULE}else{Q1_RULE},&[])?,"input":input,"schema":wire_schema});
    Ok(units(&request.to_string()))
}
fn fits(stage: &str, input: &Value, schema: &Value) -> Result<bool, String> {
    Ok(request_chars(stage, input, schema)? + OUTPUT_RESERVE <= REQUEST_BUDGET)
}

const HISTORY_DICTIONARY_FORMAT: &str = "HISTORICAL_EVIDENCE_TABLE_DESKTOP_V1";
const HISTORY_DICTIONARY_GUIDE: &str = "columns逐行对应rows；dictionaries按字段名给出值表，行内整数为该值表零起始索引。每行仍独立，rowId原样保留，不合并相同原句或人物事件。anchorRowIds的整数为本表rows零起始行号，恢复为那一行原rowId，顺序不变。text逐字保留；null仅为原显式空值。所有候选仅HISTORICAL_EVIDENCE_ONLY、当前适用性UNKNOWN；CHARACTER_CLAIM不升级为事实。";
const HISTORY_COLUMNS: [&str; 7] = [
    "rowId",
    "episodeId",
    "sceneId",
    "text",
    "claim",
    "retrievalMode",
    "anchorRowIds",
];

/// Equivalent to 3008 foundation-model-input.ts's exact dictionaries and
/// anchorRows projection. Desktop source row IDs are already stable and stay
/// explicit; no source hashes are recomputed or semantic candidates merged.
fn compact_history(value: &Value) -> Result<Value, String> {
    let Some(rows) = value.as_array().filter(|rows| rows.len() >= 2) else {
        return Ok(value.clone());
    };
    let mut positions = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(fields) = row.as_array().filter(|fields| fields.len() == 7) else {
            return Ok(value.clone());
        };
        let Some(id) = fields[0].as_str().filter(|id| !id.is_empty()) else {
            return Ok(value.clone());
        };
        if positions.insert(id.to_owned(), index).is_some()
            || !fields[1].is_string()
            || !(fields[2].is_string() || fields[2].is_null())
            || !fields[3].is_string()
            || !matches!(fields[4].as_str(), Some("SOURCE" | "CHARACTER_CLAIM"))
            || !matches!(
                fields[5].as_str(),
                Some("SUBJECT_MENTION" | "SOURCE_NEIGHBOUR")
            )
            || !fields[6].is_array()
        {
            return Ok(value.clone());
        }
    }
    if rows.iter().any(|row| {
        row[6]
            .as_array()
            .unwrap()
            .iter()
            .any(|anchor| anchor.as_str().is_none_or(|id| !positions.contains_key(id)))
    }) {
        return Ok(value.clone()); // Unknown/legacy links stay addressable and unchanged.
    }
    let mut encoded = rows.clone();
    let mut dictionaries = Map::new();
    for column in 1..=5 {
        let mut dictionary = vec![];
        let mut indexes = HashMap::new();
        let mut projected = vec![];
        for row in rows {
            let key = row[column].to_string();
            let index = *indexes.entry(key).or_insert_with(|| {
                dictionary.push(row[column].clone());
                dictionary.len() - 1
            });
            projected.push(json!(index));
        }
        let original: Vec<_> = rows.iter().map(|row| row[column].clone()).collect();
        if units(&json!({HISTORY_COLUMNS[column]:dictionary}).to_string())
            + units(&json!(projected).to_string())
            < units(&json!(original).to_string())
        {
            dictionaries.insert(HISTORY_COLUMNS[column].into(), json!(dictionary));
            for (row, index) in encoded.iter_mut().zip(projected) {
                row[column] = index;
            }
        }
    }
    for row in &mut encoded {
        row[6] = json!(row[6]
            .as_array()
            .unwrap()
            .iter()
            .map(|anchor| positions[anchor.as_str().unwrap()])
            .collect::<Vec<_>>());
    }
    let table = json!({"format":HISTORY_DICTIONARY_FORMAT,"columns":HISTORY_COLUMNS,"dictionaries":dictionaries,"anchorEncoding":"ROW_INDEX_V1","rowCount":rows.len(),"interpretation":HISTORY_DICTIONARY_GUIDE,"rows":encoded});
    if units(&table.to_string()) >= units(&value.to_string()) {
        return Ok(value.clone());
    }
    if expand_history(&table)? != *value {
        return Err("历史证据字典无法完整恢复，不提交模型".into());
    }
    Ok(table)
}

fn expand_history(value: &Value) -> Result<Value, String> {
    if value.get("format").and_then(Value::as_str) != Some(HISTORY_DICTIONARY_FORMAT) {
        return Ok(value.clone());
    }
    if value.as_object().is_none_or(|object| {
        object.keys().any(|key| {
            ![
                "format",
                "columns",
                "dictionaries",
                "anchorEncoding",
                "rowCount",
                "interpretation",
                "rows",
            ]
            .contains(&key.as_str())
        })
    }) || value["columns"] != json!(HISTORY_COLUMNS)
        || value["anchorEncoding"] != "ROW_INDEX_V1"
        || value["interpretation"] != HISTORY_DICTIONARY_GUIDE
    {
        return Err("历史证据字典声明不合法".into());
    }
    let rows = value["rows"].as_array().ok_or("历史证据字典缺rows")?;
    if value["rowCount"].as_u64() != Some(rows.len() as u64) {
        return Err("历史证据字典行数不一致".into());
    }
    let dictionaries = value["dictionaries"]
        .as_object()
        .ok_or("历史证据字典缺值表")?;
    if dictionaries
        .iter()
        .any(|(key, values)| !HISTORY_COLUMNS[1..=5].contains(&key.as_str()) || !values.is_array())
    {
        return Err("历史证据字典含未知列".into());
    }
    let ids = rows
        .iter()
        .map(|row| {
            row.as_array()
                .filter(|fields| fields.len() == 7)
                .and_then(|fields| fields[0].as_str())
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or("历史证据字典来源行不合法")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err("历史证据字典来源行重复".into());
    }
    let mut expanded = rows.clone();
    for row in &mut expanded {
        for column in 1..=5 {
            if let Some(dictionary) = dictionaries.get(HISTORY_COLUMNS[column]) {
                let index = row[column].as_u64().ok_or("历史证据字典索引不合法")? as usize;
                row[column] = dictionary
                    .as_array()
                    .unwrap()
                    .get(index)
                    .cloned()
                    .ok_or("历史证据字典索引越界")?;
            }
        }
        let anchors = row[6]
            .as_array()
            .ok_or("历史证据字典缺邻句来源")?
            .iter()
            .map(|anchor| {
                anchor
                    .as_u64()
                    .and_then(|index| ids.get(index as usize))
                    .cloned()
                    .ok_or("历史证据字典邻句索引越界")
            })
            .collect::<Result<Vec<_>, _>>()?;
        row[6] = json!(anchors);
    }
    Ok(json!(expanded))
}

fn capacity_input(stage: &str, mut input: Value, schema: &Value) -> Result<Value, String> {
    if let Some(error) = input["automaticDecisionError"].as_str() {
        return Err(error.to_owned());
    }
    // Apply model-only factoring only when the original request is too large.
    // Previously accepted small request bytes and their cache identity are stable.
    if input["modelInputProfile"] == super::foundation_wire::PROFILE
        || !fits(stage, &input, schema)?
    {
        if let Some(history) = input.pointer("/evidence/historicalEvidenceCandidates") {
            input["evidence"]["historicalEvidenceCandidates"] = compact_history(history)?;
        }
    }
    Ok(input)
}

fn prepare_capacity_batch(
    stage: &str,
    source: &LockedSource,
    pending: &mut Vec<Batch>,
    foundation: &FoundationProposal,
    context: &Value,
    index: usize,
    signals: &[Value],
) -> Result<(Batch, Value), String> {
    super::asset_coverage::enabled(context)?;
    if pending.is_empty() {
        return Err("容量规划没有待处理批次".into());
    }
    let batch = pending.remove(0);
    let schema = if stage == "F2" {
        f2_schema()
    } else {
        q1_schema()
    };
    let input = capacity_input(
        stage,
        input_for(
            stage,
            source,
            &batch,
            foundation,
            context,
            index,
            index + pending.len() + 1,
            stage == "Q1" && pending.is_empty(),
            signals,
        ),
        &schema,
    )?;
    if fits(stage, &input, &schema)? {
        return Ok((batch, input));
    }
    let overflow = || {
        if stage == "F2" {
            "F2完整请求超过200k应用预算；原文与已接受响应保留，未截断或提交"
        } else {
            "Q1完整请求超过200k应用预算；未截断资料或自动购买重试"
        }
        .to_owned()
    };
    if batch.fragment.is_some() || batch.scope.len() == 1 {
        return Err(overflow());
    }
    // Port foundation-request-plan.ts: exact projected request, largest fitting
    // ordered prefix, and one intact remainder. Never turn the tail into a queue
    // of singleton model calls. Long fragments remain indivisible boundaries.
    let mut lower = 1;
    let mut upper = batch.scope.len() - 1;
    let mut best = None;
    while lower <= upper {
        let count = (lower + upper) / 2;
        let prefix = Batch {
            scope: batch.scope[..count].to_vec(),
            pieces: whole_rows(source, &batch.scope[..count]),
            fragment: None,
        };
        let candidate = capacity_input(
            stage,
            input_for(
                stage,
                source,
                &prefix,
                foundation,
                context,
                index,
                index + pending.len() + 2,
                false,
                signals,
            ),
            &schema,
        )?;
        if fits(stage, &candidate, &schema)? {
            best = Some((count, prefix, candidate));
            lower = count + 1;
        } else {
            upper = count - 1;
        }
    }
    let Some((mut count, mut prefix, _)) = best else {
        return Err(overflow());
    };
    // If the final tail would contain just one short episode, move one episode
    // across the boundary when *both* requests fit. Keep order and call count.
    if pending.is_empty() && count > 1 && batch.scope.len() - count == 1 {
        let balanced = Batch {
            scope: batch.scope[..count - 1].to_vec(),
            pieces: whole_rows(source, &batch.scope[..count - 1]),
            fragment: None,
        };
        let tail = Batch {
            scope: batch.scope[count - 1..].to_vec(),
            pieces: whole_rows(source, &batch.scope[count - 1..]),
            fragment: None,
        };
        let left = capacity_input(
            stage,
            input_for(
                stage,
                source,
                &balanced,
                foundation,
                context,
                index,
                index + 2,
                false,
                signals,
            ),
            &schema,
        )?;
        let right = capacity_input(
            stage,
            input_for(
                stage,
                source,
                &tail,
                foundation,
                context,
                index + 1,
                index + 2,
                stage == "Q1",
                signals,
            ),
            &schema,
        )?;
        if fits(stage, &left, &schema)? && fits(stage, &right, &schema)? {
            count -= 1;
            prefix = balanced;
        }
    }
    let mut remainder = Batch {
        scope: batch.scope[count..].to_vec(),
        pieces: whole_rows(source, &batch.scope[count..]),
        fragment: None,
    };
    if stage == "F2" {
        // The old delivery recalculates the next 24k source prefix from pending
        // episodes, so an overflow tail can share a batch with its next short
        // source block without crossing a fragment or reordering selection.
        while pending.first().is_some_and(|next| {
            next.fragment.is_none()
                && remainder
                    .pieces
                    .iter()
                    .chain(&next.pieces)
                    .map(|piece| units(&piece.text))
                    .sum::<usize>()
                    <= SOURCE_BUDGET
        }) {
            let next = pending.remove(0);
            remainder.scope.extend(next.scope);
            remainder.pieces.extend(next.pieces);
        }
    }
    pending.insert(0, remainder);
    let input = capacity_input(
        stage,
        input_for(
            stage,
            source,
            &prefix,
            foundation,
            context,
            index,
            index + pending.len() + 1,
            false,
            signals,
        ),
        &schema,
    )?;
    if !fits(stage, &input, &schema)? {
        return Err(overflow());
    }
    Ok((prefix, input))
}

fn editor_batches(batches: Vec<Batch>) -> Vec<Batch> {
    let mut editor: Vec<Batch> = vec![];
    for batch in batches {
        if batch.fragment.is_none() && editor.last().is_some_and(|prior| prior.fragment.is_none()) {
            let prior = editor.last_mut().unwrap();
            prior.scope.extend(batch.scope);
            prior.pieces.extend(batch.pieces);
        } else {
            editor.push(batch);
        }
    }
    editor
}

/// Read-only capacity planning. The compatibility q1MinimumCalls field is a
/// current-registry plan, not a mathematical minimum or generated-content proof.
/// Execution recomputes both stages with their actual registry/data.
pub fn preview_foundation_delivery(
    source: &LockedSource,
    context: &Value,
) -> Result<Value, String> {
    let source = upgrade_source_dialogues(source)?;
    validate_source(&source)?;
    if context
        .pointer("/selection/episodeIds")
        .is_some_and(|value| !value.is_array())
    {
        return Err("制作集选择必须是非空ID数组，不能静默扩大到全剧".into());
    }
    let selected: Vec<String> = context
        .pointer("/selection/episodeIds")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or("制作集ID无效".into())
                })
                .collect::<Result<_, String>>()
        })
        .transpose()?
        .unwrap_or_else(|| source.episodes.iter().map(|e| e.id.clone()).collect());
    if selected.is_empty()
        || selected.iter().collect::<HashSet<_>>().len() != selected.len()
        || selected
            .iter()
            .any(|id| !source.episodes.iter().any(|e| &e.id == id))
    {
        return Err("制作集规划范围无效".into());
    }
    let mut pending = plan_batches(&source, &selected)?;
    let foundation = existing_foundation(&source, context)?;
    let mut plans = vec![];
    let mut batches = vec![];
    while !pending.is_empty() {
        let (batch, input) = prepare_capacity_batch(
            "F2",
            &source,
            &mut pending,
            &foundation,
            context,
            plans.len(),
            &[],
        )?;
        let schema = f2_schema();
        let request_chars = request_chars("F2", &input, &schema)?;
        let wire_schema = crate::text_executor::structured_schema_for_budget(&schema);
        plans.push(json!({"stage":"F2","scope":batch.scope,"fragment":batch.fragment,"sourceChars":batch.pieces.iter().map(|p|units(&p.text)).sum::<usize>(),"inputChars":units(&input.to_string()),"schemaChars":units(&wire_schema.to_string()),"requestChars":request_chars,"withinBudget":request_chars+OUTPUT_RESERVE<=REQUEST_BUDGET,"historicalEncoding":input.pointer("/evidence/historicalEvidenceCandidates/format")}));
        batches.push(batch);
    }
    verify_coverage(&source, &selected, &batches)?;
    let mut q_pending = editor_batches(batches);
    let mut q1_groups = 0;
    while !q_pending.is_empty() {
        prepare_capacity_batch(
            "Q1",
            &source,
            &mut q_pending,
            &foundation,
            context,
            q1_groups,
            &[],
        )?;
        q1_groups += 1;
    }
    Ok(
        json!({"method":METHOD_VERSION,"sourceSha256":source.sha256,"sourceCoverageVerified":true,"selectedEpisodeIds":selected,"f1ModelCalls":0,"f2PlannedCalls":plans.len(),"q1MinimumCalls":q1_groups,"q1PlanBasis":"PRE_MODEL_CURRENT_REGISTRY","f2Batches":plans,"budgets":{"sourceChars":SOURCE_BUDGET,"fragmentChars":FRAGMENT_BUDGET,"completeRequestChars":REQUEST_BUDGET,"outputReserveChars":OUTPUT_RESERVE,"coordinateSystem":"UTF-16"},"note":"F2预览与执行共用完整请求预检和无损历史字典；当前registry随后增大仍可再拆批。q1MinimumCalls兼容字段是完整源和当前registry的容量计划数，非数学最少调用数；最终范围取决于F2实际目录/方向信号。本报告未执行模型。"}),
    )
}
pub(crate) fn allowed_refs(input: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    for pointer in [
        "/source/rows",
        "/evidence/background",
        "/evidence/backgroundCandidates",
        "/evidence/historicalEvidenceCandidates",
        "/evidence/automaticDecisionSupport/rows",
    ] {
        let original = input.pointer(pointer).cloned().unwrap_or(json!([]));
        let expanded = expand_history(&original).unwrap_or(json!([]));
        for row in expanded.as_array().into_iter().flatten() {
            if let Some(id) = row.get(0).and_then(Value::as_str) {
                ids.insert(id.into());
            }
        }
    }
    ids
}

// One shared enum, rather than repeating hundreds of IDs in every field. These
// are genuine evidence/cut references; ordinary catalog entries have no refs.
fn delivery_schema(stage: &str, input: &Value) -> Value {
    fn constrain(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(properties) = map.get_mut("properties").and_then(Value::as_object_mut) {
                    if let Some(refs) = properties.get_mut("sourceRefs") {
                        refs["items"] = json!({"$ref":"#/$defs/sourceRow"});
                    }
                    if let Some(row) = properties.get_mut("rowId") {
                        *row = json!({"$ref":"#/$defs/sourceRow"});
                    }
                }
                for value in map.values_mut() {
                    constrain(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    constrain(value);
                }
            }
            _ => {}
        }
    }
    let mut schema = if stage == "F2" {
        f2_schema()
    } else {
        q1_schema()
    };
    if stage == "Q1" && input["foundationDecisionPolicy"] == AUTO_INTERPRET_POLICY {
        schema["properties"]["appliedDecisions"] = array(optional_obj(
            json!({"id":string(),"decisionKey":string(),"answer":string(),"note":string(),"subjectAssetKeys":refs_schema()}),
            &["answer", "note"],
        ));
        schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("appliedDecisions"));
    }
    if stage == "Q1"
        && input.pointer("/delivery/phase").and_then(Value::as_str) == Some("APPLY_DECISION")
    {
        schema["properties"]["appliedDecisions"] =
            array(obj(json!({"id":string(),"note":string()})));
        schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("appliedDecisions"));
    }
    constrain(&mut schema);
    let prefix = format!(
        "row:{}:",
        input["sourceSha256"]
            .as_str()
            .and_then(|s| s.get(..12))
            .unwrap_or("")
    );
    let ids: BTreeSet<_> = allowed_refs(input)
        .into_iter()
        .map(|id| {
            id.strip_prefix(&prefix)
                .map(|s| format!("r:{s}"))
                .unwrap_or(id)
        })
        .collect();
    schema["$defs"] = json!({"sourceRow":{"type":"string","enum":ids}});
    schema
}

fn restore_delivery_refs(value: &mut Value, input: &Value) -> Result<(), String> {
    let allowed = allowed_refs(input);
    let prefix = format!(
        "row:{}:",
        input["sourceSha256"]
            .as_str()
            .and_then(|s| s.get(..12))
            .ok_or("缺锁定源版本")?
    );
    fn walk(value: &mut Value, allowed: &HashSet<String>, prefix: &str) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                if let Some(refs) = map.get_mut("sourceRefs").and_then(Value::as_array_mut) {
                    for id in refs {
                        if let Some(suffix) = id.as_str().and_then(|s| s.strip_prefix("r:")) {
                            let canonical = format!("{prefix}{suffix}");
                            if !allowed.contains(&canonical) {
                                return Err(format!("原文证据编号未在本次提供：{suffix}"));
                            }
                            *id = json!(canonical);
                        }
                    }
                }
                if let Some(id) = map.get_mut("rowId") {
                    if let Some(suffix) = id.as_str().and_then(|s| s.strip_prefix("r:")) {
                        let canonical = format!("{prefix}{suffix}");
                        if !allowed.contains(&canonical) {
                            return Err(format!("原文切点/证据编号未在本次提供：{suffix}"));
                        }
                        *id = json!(canonical);
                    }
                }
                for v in map.values_mut() {
                    walk(v, allowed, prefix)?;
                }
            }
            Value::Array(values) => {
                for v in values {
                    walk(v, allowed, prefix)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, &allowed, &prefix)?;
    if let Some(signals) = value
        .pointer_mut("/seriesDirection/signals")
        .and_then(Value::as_array_mut)
    {
        for signal in signals {
            if let Some(prior) = input
                .pointer("/seriesDirectionPreparation/priorSignals")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|prior| {
                    prior["rowId"] == signal["rowId"] && prior["episodeId"] == signal["episodeId"]
                })
            {
                signal["quote"] = prior["quote"].clone();
                continue;
            }
            let rows = input
                .pointer("/source/rows")
                .and_then(Value::as_array)
                .ok_or("方向依据缺本批原行")?;
            let row = rows
                .iter()
                .find(|row| row[0] == signal["rowId"] && row[2] == signal["episodeId"])
                .ok_or("方向依据行不属于本批所指集数")?;
            // A model judges the direction; it does not transcribe the source.
            // Materialize the exact provided locked row, including punctuation.
            signal["quote"] = json!(row[9]
                .as_str()
                .ok_or("方向原行必须为文本")?
                .trim_end_matches(['\r', '\n']));
        }
    }
    Ok(())
}

/// Acceptance may expand frozen source coordinates and materialize direction
/// quotes. Permit exactly that original normalization, never arbitrary raw /
/// domain differences or references outside the frozen request.
pub(crate) fn accepted_scope_reason_raw_matches(
    input: &Value,
    raw: &Value,
    accepted: &Value,
) -> bool {
    let mut normalized = raw.clone();
    restore_delivery_refs(&mut normalized, input).is_ok() && normalized == *accepted
}

/// Narrow migration for a *saved* rejected F2. Group IDs may be expanded only
/// into complete, contiguous, currently provided rows of one episode/scene.
/// Never round a substring, fill a gap, borrow unprovided history, or guess text.
pub(crate) fn normalize_legacy_catalogue(value: &mut Value, input: &Value) -> Result<(), String> {
    let allowed = allowed_refs(input);
    let prefix = format!(
        "row:{}:",
        input["sourceSha256"]
            .as_str()
            .and_then(|s| s.get(..12))
            .ok_or("旧目录缺锁定源版本")?
    );
    let parsed = |id: &str| -> Option<(usize, usize)> {
        let suffix = id.strip_prefix(&prefix)?;
        let (start, end) = suffix.split_once(':')?;
        Some((start.parse().ok()?, end.parse().ok()?))
    };
    let mut rows: Vec<_> = allowed
        .iter()
        .filter_map(|id| parsed(id).map(|(s, e)| (s, e, id.clone())))
        .collect();
    rows.sort_by_key(|r| r.0);
    let owner = |id: &str| -> (Value, Value) {
        for pointer in ["/source/rows", "/evidence/historicalEvidenceCandidates"] {
            let table =
                expand_history(input.pointer(pointer).unwrap_or(&Value::Null)).unwrap_or(json!([]));
            if let Some(row) = table
                .as_array()
                .into_iter()
                .flatten()
                .find(|r| r[0].as_str() == Some(id))
            {
                return if pointer == "/source/rows" {
                    (row[2].clone(), row[3].clone())
                } else {
                    (row[1].clone(), row[2].clone())
                };
            }
        }
        (Value::Null, Value::Null)
    };
    let expand = |id: &str| -> Result<Vec<String>, String> {
        if allowed.contains(id) {
            return Ok(vec![id.into()]);
        }
        let (start, end) = parsed(id)
            .filter(|(s, e)| s < e)
            .ok_or_else(|| format!("旧目录引用不是本次完整原行：{id}"))?;
        let selected: Vec<_> = rows.iter().filter(|r| r.0 >= start && r.1 <= end).collect();
        if selected.len() < 2
            || selected.first().is_none_or(|r| r.0 != start)
            || selected.last().is_none_or(|r| r.1 != end)
            || selected.windows(2).any(|r| r[0].1 != r[1].0)
            || selected
                .iter()
                .any(|r| owner(&r.2) != owner(&selected[0].2))
        {
            return Err(format!("旧目录引用缺行、越界或跨集场，不能自动恢复：{id}"));
        }
        Ok(selected.into_iter().map(|r| r.2.clone()).collect())
    };
    fn walk(
        value: &mut Value,
        expand: &impl Fn(&str) -> Result<Vec<String>, String>,
    ) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                if let Some(refs) = map.get_mut("sourceRefs") {
                    let raw: Vec<String> = serde_json::from_value(refs.clone())
                        .map_err(|_| "旧目录引用须为文本数组")?;
                    let mut restored = vec![];
                    for id in raw {
                        restored.extend(expand(&id)?);
                    }
                    if restored.iter().collect::<HashSet<_>>().len() != restored.len() {
                        return Err("旧目录展开后原文引用重复".into());
                    }
                    *refs = json!(restored);
                }
                if let Some(refs) = map.get_mut("evidenceRefs").and_then(Value::as_array_mut) {
                    let mut restored = vec![];
                    for raw in std::mem::take(refs) {
                        let id = raw["rowId"].as_str().ok_or("旧目录证据对象缺rowId")?;
                        for id in expand(id)? {
                            let mut item = raw.clone();
                            item["rowId"] = json!(id);
                            restored.push(item);
                        }
                    }
                    *refs = restored;
                }
                for v in map.values_mut() {
                    walk(v, expand)?;
                }
            }
            Value::Array(values) => {
                for v in values {
                    walk(v, expand)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut candidate = value.clone();
    walk(&mut candidate, &expand)?;
    *value = candidate;
    Ok(())
}

// Attach scope anchors, not visual claims. Models still decide actual usage.
// An empty usage remains an unlocated catalog candidate; no implicit appearance.
fn bind_catalogue_scope(
    source: &LockedSource,
    batch: &Batch,
    allowed: &HashSet<String>,
    supplied: Option<&Vec<String>>,
    episodes: Option<&Vec<String>>,
    pairs: Option<&Vec<Vec<String>>>,
) -> Result<Vec<String>, String> {
    let mut refs = supplied.cloned().unwrap_or_default();
    if supplied.is_some() {
        provided_refs(&refs, allowed)?;
    }
    if supplied.is_some()
        && pairs.is_some_and(|p| !p.is_empty())
        && !refs.iter().any(|id| batch.row_ids().contains(id.as_str()))
    {
        return Err("实际本场使用不能仅用前批引用补足；范围由本次原文绑定".into());
    }
    let places = places(source, batch, &refs, episodes, pairs)?;
    for place in &places {
        let row = source
            .rows
            .iter()
            .find(|r| {
                r.episode_id.as_ref() == Some(&place.episode_id)
                    && r.scene_id.as_ref() == Some(&place.scene_id)
                    && batch.row_ids().contains(r.id.as_str())
            })
            .ok_or("本批没有实际提供该场原文")?;
        if !refs.contains(&row.id) {
            refs.push(row.id.clone());
        }
    }
    if places.is_empty() {
        for episode in episodes.into_iter().flatten() {
            let row = source
                .rows
                .iter()
                .find(|r| {
                    r.episode_id.as_ref() == Some(episode)
                        && batch.row_ids().contains(r.id.as_str())
                })
                .ok_or("本批没有实际提供该集原文")?;
            if !refs.contains(&row.id) {
                refs.push(row.id.clone());
            }
        }
    }
    Ok(refs)
}

fn mark_scope(metadata: &mut Map<String, Value>, source: &LockedSource) {
    metadata.insert("sourceRefsPurpose".into(), json!("SOURCE_SCOPE"));
    metadata.insert(
        "sourceScope".into(),
        json!({"sourceSha256":source.sha256,"kind":"PROGRAM_SCOPE"}),
    );
}

fn bind_explicit_global_context(
    source: &LockedSource,
    allowed: &HashSet<String>,
    scope: Option<&WireScope>,
    refs: &mut Vec<String>,
) {
    // Only an explicit GLOBAL declaration receives actually supplied author
    // context. Empty/unlocated candidates and one-off dialogue stay unlocated.
    if scope.is_some_and(|scope| scope.kind == "GLOBAL") {
        for row in source.rows.iter().filter(|row| {
            row.kind == "context"
                && row.dialogue.is_none()
                && allowed.contains(&row.id)
                && [
                    "规则：",
                    "规则:",
                    "世界规则：",
                    "世界观规则：",
                    "世界观设定：",
                    "世界设定：",
                ]
                .iter()
                .any(|prefix| row.text.trim_start().starts_with(prefix))
        }) {
            if !refs.contains(&row.id) {
                refs.push(row.id.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preparation {
    mode: String,
    #[serde(default)]
    from_key: Option<String>,
    #[serde(default)]
    note: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireScope {
    kind: String,
    #[serde(default)]
    reason: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VariantWire {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    local_key: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    episodes: Option<Vec<String>>,
    #[serde(default)]
    scene_refs: Option<Vec<Vec<String>>>,
    #[serde(default)]
    source_refs: Option<Vec<String>>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    preparation: Option<Preparation>,
    #[serde(default)]
    scope: Option<WireScope>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssetWire {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    aliases: Option<Vec<String>>,
    #[serde(default)]
    episodes: Option<Vec<String>>,
    #[serde(default)]
    scene_refs: Option<Vec<Vec<String>>>,
    #[serde(default)]
    source_refs: Option<Vec<String>>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    relation_note: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    preparation: Option<Preparation>,
    #[serde(default)]
    variants: Option<Vec<VariantWire>>,
    #[serde(default)]
    scope: Option<WireScope>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogueWire {
    format: String,
    source_sha256: String,
    coverage: Vec<String>,
    assets: Vec<AssetWire>,
    #[serde(default)]
    issues: Vec<FoundationIssue>,
    #[serde(default)]
    human_choices: Vec<Value>,
    #[serde(default)]
    unresolved: Vec<Value>,
    #[serde(default)]
    notes: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EpisodePatch {
    episode_id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    source_refs: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MergePatch {
    from_key: String,
    into_key: String,
    source_refs: Vec<String>,
    reason: String,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Changes {
    #[serde(default)]
    assets: Vec<AssetWire>,
    #[serde(default)]
    remove_asset_keys: Vec<String>,
    #[serde(default)]
    merge_assets: Vec<MergePatch>,
    #[serde(default)]
    episodes: Vec<EpisodePatch>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    scenes: Vec<Value>,
    #[serde(default)]
    scene_splits: Vec<Value>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OptimizationWire {
    format: String,
    source_sha256: String,
    coverage: Vec<String>,
    changes: Changes,
    #[serde(default)]
    issues: Vec<FoundationIssue>,
    #[serde(default)]
    resolved_issues: Vec<String>,
    #[serde(default)]
    applied_decisions: Vec<Value>,
    #[serde(default)]
    series_direction: Option<Value>,
    #[serde(default)]
    human_choices: Vec<Value>,
    #[serde(default)]
    unresolved: Vec<Value>,
    #[serde(default)]
    notes: Vec<String>,
}

fn supplemental_issues(
    choices: &[Value],
    pending: &[Value],
    notes: &[String],
    batch: &Batch,
    allowed: &HashSet<String>,
) -> Result<Vec<FoundationIssue>, String> {
    let mut result = vec![];
    for choice in choices {
        let map = choice.as_object().ok_or("必要制作取舍须为对象")?;
        for field in [
            "decisionKey",
            "kind",
            "question",
            "reason",
            "recommended",
            "recommendationReason",
            "evidence",
        ] {
            if map
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err(format!("制作取舍缺少 {field}"));
            }
        }
        let kind = choice["kind"].as_str().unwrap();
        if !["SOURCE_CONFLICT", "PRODUCTION_CHOICE"].contains(&kind) {
            return Err("技术错误不能包装成制作取舍".into());
        }
        let options: Vec<String> =
            serde_json::from_value(choice["options"].clone()).map_err(|_| "制作取舍选项无效")?;
        if !(2..=3).contains(&options.len())
            || options.iter().collect::<HashSet<_>>().len() != options.len()
            || !options
                .iter()
                .any(|option| Some(option.as_str()) == choice["recommended"].as_str())
        {
            return Err("制作取舍须有2–3个不同选项和其中一个推荐，推荐不是确认".into());
        }
        let refs: Vec<String> = serde_json::from_value(choice["sourceRefs"].clone())
            .map_err(|_| "制作取舍缺少锁定sourceRefs")?;
        provided_refs(&refs, allowed)?;
        let episodes: Vec<String> = serde_json::from_value(choice["episodes"].clone())
            .map_err(|_| "制作取舍影响范围无效")?;
        if episodes.iter().any(|id| !batch.scope.contains(id)) {
            return Err("制作取舍不得扩大当前授权范围".into());
        }
        let blocking = kind == "SOURCE_CONFLICT" && refs.len() >= 2;
        result.push(FoundationIssue{coordinate:format!("choice:{}",choice["decisionKey"].as_str().unwrap()),problem:choice["question"].as_str().unwrap().into(),source_refs:refs,blocking,asset_id:None,variant_id:None,episode_ids:if episodes.is_empty(){batch.scope.clone()}else{episodes},details:Some(json!({"owner":"AUTHOR","kind":"HUMAN_CHOICE","choice":choice,"recommendedIsAdopted":false}))});
    }
    for item in pending {
        let owner = item["owner"]
            .as_str()
            .filter(|owner| ["Q1", "PROGRAM", "STAGE2"].contains(owner))
            .ok_or("未决事项owner无效")?;
        let note = item["note"]
            .as_str()
            .filter(|note| !note.trim().is_empty())
            .ok_or("未决事项缺原因")?;
        let episodes: Vec<String> =
            serde_json::from_value(item["episodes"].clone()).map_err(|_| "未决事项范围无效")?;
        if episodes.iter().any(|id| !batch.scope.contains(id)) {
            return Err("未决事项越出本批范围".into());
        }
        let refs: Vec<String> =
            serde_json::from_value(item["sourceRefs"].clone()).map_err(|_| "未决事项来源无效")?;
        if !refs.is_empty() {
            provided_refs(&refs, allowed)?;
        }
        result.push(FoundationIssue {
            coordinate: format!("pending:{}", &digest(&item.to_string())[..20]),
            problem: note.into(),
            source_refs: refs.clone(),
            blocking: owner == "Q1" && !refs.is_empty(),
            asset_id: None,
            variant_id: None,
            episode_ids: if episodes.is_empty() {
                batch.scope.clone()
            } else {
                episodes
            },
            details: Some(json!({"owner":owner,"pending":item})),
        });
    }
    for note in notes {
        if !note.trim().is_empty() {
            result.push(FoundationIssue {
                coordinate: format!("note:{}", &digest(note)[..20]),
                problem: note.clone(),
                source_refs: vec![],
                blocking: false,
                asset_id: None,
                variant_id: None,
                episode_ids: batch.scope.clone(),
                details: Some(json!({"owner":"STAGE2","kind":"PRODUCTION_NOTE"})),
            });
        }
    }
    Ok(result)
}

fn check_envelope(
    format: &str,
    expected: &str,
    hash: &str,
    source: &LockedSource,
    coverage: &[String],
    batch: &Batch,
) -> Result<(), String> {
    if format != expected || hash != source.sha256 {
        return Err("资料交付格式或锁定源版本不一致".into());
    }
    if coverage.len() != batch.scope.len()
        || coverage.iter().collect::<HashSet<_>>().len() != coverage.len()
        || coverage.iter().any(|id| !batch.scope.contains(id))
    {
        return Err("资料批次coverage未精确覆盖本次提供范围".into());
    }
    Ok(())
}
fn provided_refs(refs: &[String], allowed: &HashSet<String>) -> Result<(), String> {
    if refs.is_empty() {
        return Err("原文证据为空；请引用本次提供的原行".into());
    }
    let mut seen = HashSet::new();
    for id in refs {
        if !seen.insert(id) {
            return Err(format!("原文证据重复：{id}"));
        }
        if !allowed.contains(id) {
            return Err(format!("原文证据编号未在本次提供：{id}"));
        }
    }
    Ok(())
}
fn places(
    source: &LockedSource,
    batch: &Batch,
    _refs: &[String],
    episode_ids: Option<&Vec<String>>,
    pairs: Option<&Vec<Vec<String>>>,
) -> Result<Vec<Applicability>, String> {
    let mut result = vec![];
    for pair in pairs.into_iter().flatten() {
        if pair.len() != 2
            || !batch.scope.contains(&pair[0])
            || !source
                .scenes
                .iter()
                .any(|s| s.episode_id == pair[0] && s.id == pair[1])
            || !batch.pieces.iter().any(|piece| {
                source.rows.iter().any(|r| {
                    r.id == piece.row_id
                        && r.episode_id.as_ref() == Some(&pair[0])
                        && r.scene_id.as_ref() == Some(&pair[1])
                })
            })
        {
            return Err("形象本批适用场缺少本场锁定来源或越出提供范围".into());
        }
        let place = Applicability {
            episode_id: pair[0].clone(),
            scene_id: pair[1].clone(),
        };
        if result.contains(&place) {
            return Err("形象集场引用重复".into());
        }
        result.push(place);
    }
    if let Some(episodes) = episode_ids {
        if episodes.iter().collect::<HashSet<_>>().len() != episodes.len()
            || episodes.iter().any(|id| !batch.scope.contains(id))
            || result.iter().any(|p| !episodes.contains(&p.episode_id))
        {
            return Err("资产用途集号与本次场引用不一致".into());
        }
    }
    Ok(result)
}

/// Nonempty, proved scene usage is authoritative. Episode IDs are redundant
/// transport metadata, so derive them without editing the saved model reply.
/// Catalogue/reference-only entries still retain their episode-only contract.
fn projected_usage_episodes(
    source: &LockedSource,
    batch: &Batch,
    episode_ids: Option<Vec<String>>,
    pairs: Option<&Vec<Vec<String>>>,
) -> Result<Option<Vec<String>>, String> {
    if pairs.is_none_or(|pairs| pairs.is_empty()) {
        return Ok(episode_ids);
    }
    // Validate exact source, batch and supplied scene rows before projecting;
    // a listed episode never creates or broadens a scene reference.
    let proved = places(source, batch, &[], None, pairs)?;
    Ok(Some(
        batch
            .scope
            .iter()
            .filter(|episode| proved.iter().any(|place| &place.episode_id == *episode))
            .cloned()
            .collect(),
    ))
}

fn scope_value(
    raw: Option<&WireScope>,
    places: Vec<Applicability>,
    refs: &[String],
    source: &LockedSource,
) -> Result<FoundationScope, String> {
    let kind = raw
        .map(|s| s.kind.as_str())
        .unwrap_or(if places.is_empty() {
            "UNRESOLVED"
        } else {
            "SCENES"
        });
    let reason = raw.and_then(|s| s.reason.clone());
    if kind == "GLOBAL"
        && (reason.as_ref().is_none_or(|r| r.trim().is_empty())
            || !places.is_empty()
            || !refs.iter().any(|id| {
                source
                    .rows
                    .iter()
                    .any(|r| &r.id == id && r.kind == "context" && r.dialogue.is_none())
            }))
    {
        return Err("GLOBAL必须有明确资料来源和原因，不能由一次出场推广".into());
    }
    if !["GLOBAL", "SCENES", "UNRESOLVED"].contains(&kind)
        || kind == "SCENES" && places.is_empty()
        || kind == "UNRESOLVED" && !places.is_empty()
    {
        return Err("形象范围声明不合法".into());
    }
    Ok(FoundationScope {
        kind: kind.into(),
        scenes: places,
        reason,
    })
}

// Scope reasons describe where/how an appearance may be used. They are not
// visual SOURCE facts. Keep the earlier wording first, then exact new lines;
// never paraphrase, infer an omitted reason, or repeat a previously saved line.
fn merge_scope_reasons(previous: Option<&str>, supplied: Option<&str>) -> Option<String> {
    let mut lines = Vec::new();
    for reason in [previous, supplied].into_iter().flatten() {
        for line in reason.lines().filter(|line| !line.trim().is_empty()) {
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn scope_for_patch(
    raw: Option<&WireScope>,
    places: Vec<Applicability>,
    refs: &[String],
    source: &LockedSource,
    previous: Option<&FoundationVariant>,
    batch: &Batch,
    patch: bool,
) -> Result<FoundationScope, String> {
    // A scoped difference may remove this batch's use while retaining genuine
    // uses outside the batch. Validate the final range, not just its empty delta.
    if patch
        && places.is_empty()
        && raw.is_some_and(|scope| scope.kind == "SCENES")
        && previous.is_some_and(|v| {
            v.scope.kind == "SCENES"
                && v.scope
                    .scenes
                    .iter()
                    .any(|p| !batch.scope.contains(&p.episode_id))
        })
    {
        return Ok(FoundationScope {
            kind: "SCENES".into(),
            scenes: places,
            reason: raw.and_then(|s| s.reason.clone()),
        });
    }
    scope_value(raw, places, refs, source)
}
fn metadata(
    preparation: Option<&Preparation>,
    role: Option<&String>,
    relation: Option<&String>,
    note: Option<&String>,
) -> Result<Map<String, Value>, String> {
    let mut result = Map::new();
    if let Some(p) = preparation {
        if !["REFERENCE", "DERIVE", "INLINE", "AUDIO", "UNRESOLVED"].contains(&p.mode.as_str())
            || !["DERIVE", "INLINE"].contains(&p.mode.as_str()) && p.from_key.is_some()
        {
            return Err("素材制作方式或派生来源字段不合法；剧情关联不是素材来源".into());
        }
        let mut value = json!({"mode":p.mode});
        if let Some(key) = &p.from_key {
            value["fromKey"] = json!(key);
        }
        if let Some(note) = &p.note {
            value["note"] = json!(note);
        }
        result.insert("preparation".into(), value);
    }
    if let Some(role) = role {
        if !role.is_empty()
            && !["男主", "女主", "主角", "配角", "反派", "路人"].contains(&role.as_str())
        {
            return Err("人物定位字段不受支持".into());
        }
        result.insert("role".into(), json!(role));
    }
    if let Some(relation) = relation {
        result.insert("relationNote".into(), json!(relation));
    }
    if let Some(note) = note {
        result.insert("note".into(), json!(note));
    }
    Ok(result)
}
fn base_id(asset: &FoundationAsset) -> Result<String, String> {
    asset
        .variants
        .iter()
        .find(|v| v.metadata.get("isBase") == Some(&json!(true)))
        .or_else(|| asset.variants.first())
        .map(|v| v.id.clone())
        .ok_or("已登记身份缺少基础形象".into())
}
fn update_usage(
    metadata: &mut Map<String, Value>,
    incoming: &[String],
    batch: &Batch,
    patch: bool,
) {
    let mut usage: Vec<String> = metadata
        .get("usageEpisodes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if patch && batch.fragment.is_none() {
        usage.retain(|id| !batch.scope.contains(id));
    }
    for id in incoming {
        if !usage.contains(id) {
            usage.push(id.clone());
        }
    }
    metadata.insert("usageEpisodes".into(), json!(usage));
}
fn source_parent(
    preparation: Option<&Preparation>,
    owner: &str,
    own_id: &str,
    assets: &[FoundationAsset],
    default_parent: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(p) = preparation else {
        return Ok(default_parent.map(str::to_string));
    };
    if !["DERIVE", "INLINE"].contains(&p.mode.as_str()) {
        return Ok(None);
    }
    if p.mode == "INLINE" && p.from_key.is_none() && default_parent.is_none() {
        // A standalone inline item has no proved carrying image. Binding must
        // keep it unresolved; the program cannot guess a scene/character image.
        return Ok(None);
    }
    let requested = p
        .from_key
        .as_deref()
        .or(default_parent)
        .ok_or("派生素材缺少真实来源身份/形象键")?;
    let parent = if let Some(asset) = assets.iter().find(|a| a.id == requested) {
        base_id(asset)?
    } else {
        resolve_foundation_variant(assets, owner, requested)?
            .1
            .id
            .clone()
    };
    if parent == own_id {
        return Err("派生素材不能引用自身".into());
    }
    Ok(Some(parent))
}
fn build_asset(
    source: &LockedSource,
    batch: &Batch,
    wire: &AssetWire,
    prior: &FoundationProposal,
    allowed: &HashSet<String>,
    patch: bool,
    new_ordinal: usize,
) -> Result<FoundationAsset, String> {
    let old = wire
        .key
        .as_ref()
        .and_then(|id| prior.assets.iter().find(|a| &a.id == id));
    if wire.key.is_some() && old.is_none() {
        return Err("未知资产key不可由模型猜测；新对象须省略key由程序编号".into());
    }
    let supplied_episodes = projected_usage_episodes(
        source,
        batch,
        wire.episodes.clone(),
        wire.scene_refs.as_ref(),
    )?;
    let scope_episodes = supplied_episodes.clone().or_else(|| {
        old.and_then(|a| a.metadata.get("usageEpisodes"))
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .filter(|e| batch.scope.iter().any(|id| id == e))
                    .map(str::to_string)
                    .collect()
            })
    });
    let mut refs_value = bind_catalogue_scope(
        source,
        batch,
        allowed,
        wire.source_refs.as_ref(),
        scope_episodes.as_ref(),
        wire.scene_refs.as_ref(),
    )?;
    bind_explicit_global_context(source, allowed, wire.scope.as_ref(), &mut refs_value);
    let refs = &refs_value;
    let catalog_name = wire
        .name
        .clone()
        .or_else(|| {
            old.map(|a| {
                a.metadata
                    .get("catalogName")
                    .and_then(Value::as_str)
                    .unwrap_or(&a.name)
                    .to_string()
            })
        })
        .filter(|s| !s.trim().is_empty())
        .ok_or("新资产缺少基础名称")?;
    let kind = wire
        .kind
        .clone()
        .or_else(|| old.map(|a| a.kind.clone()))
        .filter(|k| ["CH", "GROUP", "BEAST", "LOC", "PROP"].contains(&k.as_str()))
        .ok_or("新资产分类无效")?;
    // Keep the old display contract while separating canonical identity and its
    // baseline appearance. Neither half becomes a lookup key.
    let (name, baseline_name) = if kind == "CH" {
        catalog_name
            .split_once(['|', '｜'])
            .map(|(identity, look)| (identity.trim().to_string(), look.trim().to_string()))
            .unwrap_or((catalog_name.clone(), "默认".into()))
    } else {
        (catalog_name.clone(), catalog_name.clone())
    };
    if name.is_empty() || baseline_name.is_empty() {
        return Err("人物身份或基础服装名称为空".into());
    }
    if !patch && old.is_some_and(|a| a.name != name || a.kind != kind) {
        return Err("资产身份key不能通过本批显示名或分类修改替换；请明确身份修复".into());
    }
    if old.is_some_and(|a| {
        a.kind == "CH"
            && kind == "CH"
            && wire.name.is_some()
            && a.variants.first().is_some_and(|v| v.name != baseline_name)
            && (!patch
                || !wire.variants.as_ref().is_some_and(|variants| {
                    variants.iter().any(|v| {
                        v.key.as_deref() == a.variants.first().map(|base| base.id.as_str())
                            && v.name.as_deref() == Some(baseline_name.as_str())
                    })
                }))
    }) {
        return Err("已登记人物基线服装不能由主资产名称重写；有据换装应声明子形象".into());
    }
    let id = old.map(|a| a.id.clone()).unwrap_or_else(|| {
        let identity = if wire.source_refs.is_some() {
            // Preserve the exact identity of already saved evidence-inventory
            // requests, so their subsequent frozen editor can be replayed.
            json!([source.sha256, kind, catalog_name, refs.first()])
        } else {
            // Scope anchors and display names are not unique identity keys.
            json!([source.sha256, kind, catalog_name, batch.key(), new_ordinal])
        };
        format!("{}_{}", kind, &digest(&identity.to_string())[..16])
    });
    let bid = if let Some(old) = old {
        base_id(old)?
    } else {
        format!("{id}:base")
    };
    let mut asset = old.cloned().unwrap_or(FoundationAsset {
        id: id.clone(),
        name: name.clone(),
        kind: kind.clone(),
        aliases: vec![],
        description: String::new(),
        source_refs: vec![],
        variants: vec![],
        metadata: Map::new(),
    });
    if patch {
        // Like the old Q1 contract, omitted aliases remain unchanged even
        // during an explicit display-name edit. Name history is not a new
        // source-backed alias declaration or a replacement identity key.
        asset.name = name.clone();
        asset.kind = kind.clone();
        if !matches!(kind.as_str(), "CH" | "BEAST") {
            asset.metadata.remove("role");
        }
    }
    if let Some(aliases) = &wire.aliases {
        if patch {
            asset.aliases = aliases.clone();
        } else {
            for alias in aliases {
                if !asset.aliases.contains(alias) {
                    asset.aliases.push(alias.clone());
                }
            }
        }
    }
    if let Some(description) = &wire.description {
        asset.description = description.clone();
    }
    for reference in refs {
        if !asset.source_refs.contains(reference) {
            asset.source_refs.push(reference.clone());
        }
    }
    asset.metadata.extend(metadata(
        wire.preparation.as_ref(),
        // Narrative importance and image-template classification are separate:
        // a non-human individual can be the protagonist without becoming CH.
        // Empty/non-applicable roles are transport noise, not user decisions.
        wire.role.as_ref().filter(|role| matches!(kind.as_str(), "CH" | "BEAST") && !role.trim().is_empty()),
        wire.relation_note.as_ref(),
        None,
    )?);
    mark_scope(&mut asset.metadata, source);
    asset
        .metadata
        .insert("catalogName".into(), json!(catalog_name));
    if let Some(episodes) = &supplied_episodes {
        update_usage(&mut asset.metadata, episodes, batch, patch);
    }
    if !matches!(kind.as_str(), "CH" | "BEAST") {
        asset.metadata.remove("role");
    }
    let base_changed = old.is_none()
        || patch && kind != "CH" && wire.name.is_some()
        || wire.scene_refs.is_some()
        || wire.episodes.is_some()
        || wire.preparation.is_some()
        || wire.scope.is_some();
    if base_changed {
        let previous = asset.variants.iter().find(|v| v.id == bid).cloned();
        let place = if wire.scene_refs.is_some() {
            places(
                source,
                batch,
                refs,
                supplied_episodes.as_ref(),
                wire.scene_refs.as_ref(),
            )?
        } else {
            vec![]
        };
        let scope = if wire.scene_refs.is_some() || wire.scope.is_some() || old.is_none() {
            scope_for_patch(
                wire.scope.as_ref(),
                place,
                refs,
                source,
                previous.as_ref(),
                batch,
                patch,
            )?
        } else {
            previous
                .as_ref()
                .map(|v| v.scope.clone())
                .unwrap_or_default()
        };
        let mut base = previous.unwrap_or(FoundationVariant {
            id: bid.clone(),
            name: baseline_name.clone(),
            description: String::new(),
            source_variant_id: None,
            source_refs: vec![],
            applicability: vec![],
            scope: FoundationScope::default(),
            asset_row_id: None,
            metadata: Map::new(),
        });
        // Non-character bases are a desktop projection of the original
        // asset display name, not another independently named entity.
        if patch && kind != "CH" && wire.name.is_some() {
            base.name = baseline_name.clone();
        }
        if patch && batch.fragment.is_none() && wire.scene_refs.is_some() {
            base.scope
                .scenes
                .retain(|p| !batch.scope.contains(&p.episode_id));
        }
        if scope.kind == "GLOBAL" {
            base.scope = scope;
        } else {
            base.scope.reason =
                merge_scope_reasons(base.scope.reason.as_deref(), scope.reason.as_deref());
            for p in scope.scenes {
                if !base.scope.scenes.contains(&p) {
                    base.scope.scenes.push(p);
                }
            }
            base.scope.kind = if base.scope.scenes.is_empty() {
                "UNRESOLVED"
            } else {
                "SCENES"
            }
            .into();
        }
        base.applicability = base.scope.scenes.clone();
        for r in refs {
            if !base.source_refs.contains(r) {
                base.source_refs.push(r.clone());
            }
        }
        base.source_variant_id = source_parent(
            wire.preparation.as_ref(),
            &id,
            &bid,
            &prior.assets,
            base.source_variant_id.as_deref(),
        )?;
        base.metadata
            .extend(metadata(wire.preparation.as_ref(), None, None, None)?);
        base.metadata.insert("isBase".into(), json!(true));
        mark_scope(&mut base.metadata, source);
        if let Some(usage) = asset.metadata.get("usageEpisodes") {
            base.metadata.insert("usageEpisodes".into(), usage.clone());
        }
        if let Some(at) = asset.variants.iter().position(|v| v.id == bid) {
            asset.variants[at] = base;
        } else {
            asset.variants.insert(0, base);
        }
    }
    if let Some(variants) = &wire.variants {
        // Allocate all stable IDs before resolving edges. localKey is an exact,
        // owner-local transport alias, never a persisted identity or name lookup.
        let mut local_keys = HashMap::new();
        let mut planned = Vec::new();
        let mut planned_ids = HashSet::new();
        for v in variants {
            let existing = v
                .key
                .as_ref()
                .and_then(|key| asset.variants.iter().find(|v| &v.id == key));
            if v.key.is_some() && existing.is_none() {
                return Err("未知形象key不可猜测，新形象须省略key".into());
            }
            let name = v
                .name
                .clone()
                .or_else(|| existing.map(|v| v.name.clone()))
                .filter(|s| !s.trim().is_empty())
                .ok_or("形象名称缺失")?;
            let vid = existing
                .map(|v| v.id.clone())
                .unwrap_or_else(|| format!("{id}_V_{}", &digest(&name)[..16]));
            if !planned_ids.insert(vid.clone()) {
                return Err("子形象重复或再次声明基础形象".into());
            }
            if let Some(alias) = &v.local_key {
                if alias.trim().is_empty() || alias.trim() != alias {
                    return Err("形象localKey不能为空或含首尾空白".into());
                }
                if alias.split_at_checked(1).is_some_and(|(head, tail)|
                    ["e", "s", "r", "a", "v"].contains(&head)
                        && !tail.is_empty() && tail.bytes().all(|c| c.is_ascii_digit()))
                {
                    return Err("形象localKey不能占用运输短键eN/sN/rN/aN/vN".into());
                }
                if local_keys.insert(alias.clone(), vid.clone()).is_some() {
                    return Err("同一资产的形象localKey重复".into());
                }
            }
            planned.push((vid, name));
        }
        for alias in local_keys.keys() {
            if alias == &id || alias == &bid || planned_ids.contains(alias)
                || prior.assets.iter().any(|a| &a.id == alias || a.variants.iter().any(|v| &v.id == alias))
            {
                return Err("形象localKey与已登记或本包稳定键冲突，来源不明确".into());
            }
        }
        let mut reference_owner = asset.clone();
        for (vid, name) in &planned {
            if !reference_owner.variants.iter().any(|v| &v.id == vid) {
                reference_owner.variants.push(FoundationVariant {
                    id: vid.clone(), name: name.clone(), description: String::new(),
                    source_variant_id: None, source_refs: vec![], applicability: vec![],
                    scope: FoundationScope::default(), asset_row_id: None, metadata: Map::new(),
                });
            }
        }
        let mut all = prior.assets.clone();
        if let Some(at) = all.iter().position(|a| a.id == id) {
            all[at] = reference_owner;
        } else {
            all.push(reference_owner);
        }
        let mut seen = HashSet::new();
        for (v, (vid, name)) in variants.iter().zip(planned) {
            let existing = v.key.as_ref().and_then(|key| asset.variants.iter().find(|v| &v.id == key));
            // The registry exposes the base as an existing keyed appearance.
            // F2 may describe its uses alongside the other appearances; that
            // updates the same record, rather than creating a duplicate child.
            if !seen.insert(vid.clone())
                || (vid == bid && !patch && !old.is_some_and(|a| a.variants.iter().any(|v| v.id == bid))) {
                return Err("子形象重复或再次声明基础形象".into());
            }
            if vid == bid && !patch && name != existing.unwrap().name {
                return Err("基础形象身份不能在形象差异中隐式改名".into());
            }
            let supplied_episodes = projected_usage_episodes(
                source,
                batch,
                v.episodes.clone(),
                v.scene_refs.as_ref(),
            )?;
            let scope_episodes = supplied_episodes.clone().or_else(|| {
                existing
                    .and_then(|v| v.metadata.get("usageEpisodes"))
                    .and_then(Value::as_array)
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .filter(|e| batch.scope.iter().any(|id| id == e))
                            .map(str::to_string)
                            .collect()
                    })
            });
            let mut refs_value = bind_catalogue_scope(
                source,
                batch,
                allowed,
                v.source_refs.as_ref(),
                scope_episodes.as_ref(),
                v.scene_refs.as_ref(),
            )?;
            bind_explicit_global_context(source, allowed, v.scope.as_ref(), &mut refs_value);
            let refs = &refs_value;
            let mut variant = existing.cloned().unwrap_or(FoundationVariant {
                id: vid.clone(),
                name: name.clone(),
                description: String::new(),
                source_variant_id: Some(bid.clone()),
                source_refs: vec![],
                applicability: vec![],
                scope: FoundationScope::default(),
                asset_row_id: None,
                metadata: Map::new(),
            });
            // The supplied key has already resolved to this owner's existing
            // variant. Q1 may explicitly edit its label without re-keying it
            // or implicitly redirecting its production source or uses.
            if patch && v.name.is_some() {
                variant.name = name;
            }
            if v.scene_refs.is_some() || existing.is_none() {
                let current = scope_for_patch(
                    v.scope.as_ref(),
                    places(
                        source,
                        batch,
                        refs,
                        supplied_episodes.as_ref(),
                        v.scene_refs.as_ref(),
                    )?,
                    refs,
                    source,
                    existing,
                    batch,
                    patch,
                )?;
                if (patch || vid == bid) && batch.fragment.is_none() {
                    variant
                        .scope
                        .scenes
                        .retain(|p| !batch.scope.contains(&p.episode_id));
                }
                if current.kind == "GLOBAL" {
                    variant.scope = current;
                } else {
                    variant.scope.reason = merge_scope_reasons(
                        variant.scope.reason.as_deref(),
                        current.reason.as_deref(),
                    );
                    for p in current.scenes {
                        if !variant.scope.scenes.contains(&p) {
                            variant.scope.scenes.push(p);
                        }
                    }
                    variant.scope.kind = if variant.scope.scenes.is_empty() {
                        "UNRESOLVED"
                    } else {
                        "SCENES"
                    }
                    .into();
                }
            }
            for r in refs {
                if !variant.source_refs.contains(r) {
                    variant.source_refs.push(r.clone());
                }
            }
            variant.applicability = variant.scope.scenes.clone();
            let mut preparation = v.preparation.clone();
            if let Some(p) = &mut preparation {
                if let Some(stable) = p.from_key.as_ref().and_then(|key| local_keys.get(key)) {
                    p.from_key = Some(stable.clone());
                }
            }
            variant.source_variant_id = source_parent(
                preparation.as_ref(),
                &id,
                &vid,
                &all,
                variant.source_variant_id.as_deref(),
            )?;
            // Persist only the canonical immediate parent, including asset-key
            // references which resolve to an existing base appearance.
            if let Some(p) = &mut preparation {
                if p.from_key.is_some() && ["DERIVE", "INLINE"].contains(&p.mode.as_str()) {
                    p.from_key = variant.source_variant_id.clone();
                }
            }
            variant.metadata.extend(metadata(
                preparation.as_ref(),
                None,
                None,
                v.note.as_ref(),
            )?);
            mark_scope(&mut variant.metadata, source);
            if let Some(episodes) = &supplied_episodes {
                update_usage(&mut variant.metadata, episodes, batch, patch || vid == bid && batch.fragment.is_none());
            }
            if existing.is_none() && v.preparation.is_none() {
                variant
                    .metadata
                    .insert("preparation".into(), json!({"mode":"DERIVE","fromKey":bid}));
            }
            if let Some(note) = &v.note {
                variant.description = note.clone();
            }
            if let Some(at) = asset.variants.iter().position(|v| v.id == vid) {
                asset.variants[at] = variant;
            } else {
                asset.variants.push(variant);
            }
        }
        // A supplied child set replaces only this batch's child usage. Omitted
        // children remain canonical records and retain every batch-external use.
        // A long fragment cannot clear usage belonging to an unread sibling slice.
        if patch && batch.fragment.is_none() && variants.iter().any(|v| v.key.is_none()) {
            for variant in asset
                .variants
                .iter_mut()
                .filter(|v| v.id != bid && !seen.contains(&v.id))
            {
                variant
                    .scope
                    .scenes
                    .retain(|s| !batch.scope.contains(&s.episode_id));
                variant.applicability = variant.scope.scenes.clone();
                if variant.scope.scenes.is_empty() && variant.scope.kind == "SCENES" {
                    variant.scope.kind = "UNRESOLVED".into();
                }
            }
        }
    }
    Ok(asset)
}

fn check_issues(
    issues: &[FoundationIssue],
    batch: &Batch,
    allowed: &HashSet<String>,
) -> Result<(), String> {
    for issue in issues {
        if !issue.source_refs.is_empty() {
            provided_refs(&issue.source_refs, allowed)?;
        }
        if issue.episode_ids.iter().any(|id| !batch.scope.contains(id)) {
            return Err("局部事项越出实际提供范围".into());
        }
    }
    Ok(())
}

// F2 proposes production methods; it does not silently replace a registered
// material dependency. Keep its valid scope/evidence delta and give Q1 both
// alternatives through the existing issue channel instead of rejecting a batch.
fn retain_f2_preparation(
    asset: &mut FoundationAsset,
    prior: &FoundationProposal,
    source: &LockedSource,
    batch: &Batch,
) -> Result<Vec<FoundationIssue>, String> {
    let Some(old) = prior.assets.iter().find(|a| a.id == asset.id) else {
        return Ok(vec![]);
    };
    let old_base = base_id(old)?;
    let current_rows = batch.row_ids();
    let mut issues = vec![];
    for variant in &mut asset.variants {
        let Some(previous) = old.variants.iter().find(|v| v.id == variant.id) else {
            continue;
        };
        let old_preparation = previous.metadata.get("preparation");
        let proposed_preparation = variant.metadata.get("preparation").cloned();
        let old_mode = old_preparation.and_then(|p| p.get("mode"));
        let proposed_mode = proposed_preparation.as_ref().and_then(|p| p.get("mode"));
        if previous.source_variant_id == variant.source_variant_id && old_mode == proposed_mode {
            continue;
        }
        let previous_basis =
            json!({"preparation":old_preparation,"sourceVariantId":previous.source_variant_id});
        let proposed_basis =
            json!({"preparation":proposed_preparation,"sourceVariantId":variant.source_variant_id});
        let source_refs: Vec<String> = variant
            .source_refs
            .iter()
            .filter(|id| current_rows.contains(id.as_str()))
            .cloned()
            .collect();
        let current_scenes: Vec<_> = variant
            .scope
            .scenes
            .iter()
            .filter(|place| {
                source.rows.iter().any(|row| {
                    source_refs.contains(&row.id)
                        && row.episode_id.as_ref() == Some(&place.episode_id)
                        && row.scene_id.as_ref() == Some(&place.scene_id)
                })
            })
            .collect();
        issues.push(FoundationIssue {
            coordinate: format!("preparation:{}", &digest(&json!([batch.key(), asset.id, variant.id, previous_basis, proposed_basis]).to_string())[..20]),
            problem: format!("形象「{}｜{}」的制作来源建议与既有登记不同；已保留原登记，Q1可明确修订，或在resolvedIssues中确认沿用原登记。", asset.name, variant.name),
            source_refs,
            blocking: false,
            asset_id: Some(asset.id.clone()),
            variant_id: Some(variant.id.clone()),
            episode_ids: batch.scope.clone(),
            details: Some(json!({"owner":"Q1","kind":"PREPARATION_CONFLICT","method":METHOD_VERSION,
                "previous":previous_basis,"proposed":proposed_basis,"sourceRefsPurpose":"SOURCE_SCOPE",
                "currentSceneRefs":current_scenes,"previousSceneRefs":previous.scope.scenes,
                "proposedSceneRefs":variant.scope.scenes})),
        });
        variant.source_variant_id = previous.source_variant_id.clone();
        if let Some(preparation) = old_preparation {
            variant
                .metadata
                .insert("preparation".into(), preparation.clone());
        } else {
            variant.metadata.remove("preparation");
        }
        if variant.id == old_base {
            if let Some(preparation) = old.metadata.get("preparation") {
                asset
                    .metadata
                    .insert("preparation".into(), preparation.clone());
            } else {
                asset.metadata.remove("preparation");
            }
        }
    }
    Ok(issues)
}

fn retains_registered_preparation(
    issue: &FoundationIssue,
    foundation: &FoundationProposal,
) -> bool {
    let Some(details) = issue.details.as_ref() else {
        return false;
    };
    if issue.blocking
        || details["owner"] != "Q1"
        || details["kind"] != "PREPARATION_CONFLICT"
        || details["method"] != METHOD_VERSION
    {
        return false;
    }
    let Some(variant) = foundation
        .assets
        .iter()
        .find(|a| Some(&a.id) == issue.asset_id.as_ref())
        .and_then(|a| {
            a.variants
                .iter()
                .find(|v| Some(&v.id) == issue.variant_id.as_ref())
        })
    else {
        return false;
    };
    details["previous"]
        == json!({"preparation":variant.metadata.get("preparation"),"sourceVariantId":variant.source_variant_id})
}

fn apply_catalogue(
    value: &Value,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    prior: &FoundationProposal,
    confirmed: Option<&str>,
    resolved: &HashSet<(String, String)>,
) -> Result<FoundationProposal, String> {
    let wire: CatalogueWire =
        serde_json::from_value(value.clone()).map_err(|e| format!("F2目录结构无效：{e}"))?;
    check_envelope(
        &wire.format,
        "ASSET_CATALOG_DESKTOP_V2",
        &wire.source_sha256,
        source,
        &wire.coverage,
        batch,
    )?;
    let allowed = allowed_refs(input);
    check_issues(&wire.issues, batch, &allowed)?;
    if wire.assets.iter().any(|asset| {
        asset.name.is_none()
            || asset.kind.is_none()
            || asset.episodes.is_none()
            || asset.scene_refs.is_none()
            || asset.preparation.is_none()
            || asset.variants.as_ref().is_some_and(|variants| {
                variants
                    .iter()
                    .any(|v| v.name.is_none() || v.episodes.is_none() || v.scene_refs.is_none())
            })
    }) {
        return Err("F2必要目录字段缺失；不以桌面默认值补造模型未交付内容".into());
    }
    let mut candidate = empty_foundation(source);
    let mut staged = prior.clone();
    let mut seen = HashSet::new();
    let mut explicit_base_scopes = HashSet::new();
    for (ordinal, wire) in wire.assets.iter().enumerate() {
        let asset = build_asset(source, batch, wire, &staged, &allowed, false, ordinal)?;
        let bid = base_id(&asset)?;
        if wire.variants.as_ref().is_some_and(|variants| {
            variants.iter().any(|v| v.key.as_deref() == Some(bid.as_str()))
        }) {
            explicit_base_scopes.insert(bid);
        }
        if !seen.insert(asset.id.clone()) {
            return Err("本批资产身份重复".into());
        }
        if let Some(at) = staged.assets.iter().position(|a| a.id == asset.id) {
            staged.assets[at] = asset.clone();
        } else {
            staged.assets.push(asset.clone());
        }
        candidate.assets.push(asset);
    }
    // Validate the complete proposed graph BEFORE restoring old dependencies.
    // Unknown parents, self-links, cycles and invalid identities/scenes remain
    // hard errors even when some of the offending proposals target old keys.
    validate_foundation(source, &staged)?;
    candidate.issues = wire.issues;
    candidate.issues.extend(supplemental_issues(
        &wire.human_choices,
        &wire.unresolved,
        &wire.notes,
        batch,
        &allowed,
    )?);
    for asset in &mut candidate.assets {
        candidate
            .issues
            .extend(retain_f2_preparation(asset, prior, source, batch)?);
    }
    let mut next = prior.clone();
    // Reuse canonical identity/parent merge checks. Metadata is orthogonal to identity.
    merge_foundation(&mut next, candidate.clone(), confirmed, resolved)?;
    for asset in candidate.assets {
        if let Some(target) = next.assets.iter_mut().find(|a| a.id == asset.id) {
            target.metadata = asset.metadata;
            target.description = asset.description;
            for v in asset.variants {
                if let Some(prior) = target.variants.iter_mut().find(|p| p.id == v.id) {
                    prior.metadata = v.metadata;
                    // The shared identity merge unions scene coordinates, but
                    // does not merge their usage explanations. Preserve the
                    // accepted reason here for both new and existing variants.
                    if prior.scope.kind != "GLOBAL" && v.scope.kind != "GLOBAL" {
                        prior.scope.reason = merge_scope_reasons(
                            prior.scope.reason.as_deref(),
                            v.scope.reason.as_deref(),
                        );
                    }
                    // build_asset already retains uses outside this batch (and
                    // earlier fragments). Do not union stale in-batch base uses
                    // back in after the model explicitly supplied its scope.
                    if explicit_base_scopes.contains(&v.id) {
                        prior.scope = v.scope;
                        prior.applicability = prior.scope.scenes.clone();
                    }
                }
            }
        }
    }
    materialize_foundation(source, &mut next)?;
    validate_foundation(source, &next)?;
    // Require each explicitly named co-speaker in this delivered source batch
    // to resolve separately. Do not guess aliases or silently invent assets.
    let batch_rows = batch.row_ids();
    for row in source.rows.iter().filter(|row| batch_rows.contains(row.id.as_str())) {
        for q in row.dialogue.iter().filter(|q| !q.speakers.is_empty()) {
            for member in &q.speakers {
                let matches = next.assets.iter().filter(|asset| ["CH", "GROUP", "BEAST"].contains(&asset.kind.as_str()) &&
                    (asset.name.split(['|', '｜']).next().unwrap_or("").trim() == member.text || asset.aliases.contains(&member.text))).count();
                if matches != 1 {
                    return Err(format!("共同说者成员「{}」须分别对应唯一正式资产；当前匹配 {} 项，请依据原文名单修正目录身份或别名", member.text, matches));
                }
            }
        }
    }
    Ok(next)
}
fn scene_todos(
    changes: &Changes,
    source: &LockedSource,
    batch: &Batch,
    allowed: &HashSet<String>,
) -> Result<Vec<FoundationIssue>, String> {
    let mut result = vec![];
    for (kind, list) in [
        ("sceneMetadata", &changes.scenes),
        ("sceneSplit", &changes.scene_splits),
    ] {
        for value in list {
            let raw = value.as_object().ok_or("场次程序计划须为对象")?;
            let episode = raw
                .get("episodeId")
                .and_then(Value::as_str)
                .ok_or("场次计划缺episodeId")?;
            let scene = raw
                .get("sceneId")
                .and_then(Value::as_str)
                .ok_or("场次计划缺sceneId")?;
            if !batch.scope.iter().any(|id| id == episode)
                || !source
                    .scenes
                    .iter()
                    .any(|s| s.id == scene && s.episode_id == episode)
            {
                return Err("场次程序计划越出来源范围".into());
            }
            let mut refs = vec![];
            if kind == "sceneMetadata" {
                if raw.keys().any(|key| {
                    ![
                        "episodeId",
                        "sceneId",
                        "sourceRefs",
                        "location",
                        "timeOfDay",
                        "interiorExterior",
                        "timeLayer",
                        "weather",
                        "characters",
                    ]
                    .contains(&key.as_str())
                }) {
                    return Err("场次元数据修改含未知字段".into());
                }
                refs = if let Some(raw_refs) = raw.get("sourceRefs") {
                    let refs: Vec<String> = serde_json::from_value(raw_refs.clone())
                        .map_err(|_| "场修改来源结构无效")?;
                    provided_refs(&refs, allowed)?;
                    refs
                } else {
                    source
                        .rows
                        .iter()
                        .filter(|r| {
                            r.scene_id.as_deref() == Some(scene)
                                && batch.row_ids().contains(r.id.as_str())
                        })
                        .map(|r| r.id.clone())
                        .collect()
                };
                if !refs.iter().any(|id| {
                    batch.row_ids().contains(id.as_str())
                        && source
                            .rows
                            .iter()
                            .any(|r| &r.id == id && r.scene_id.as_deref() == Some(scene))
                }) {
                    return Err("场次元数据修改缺少本场实际提供的来源".into());
                }
                for field in [
                    "location",
                    "timeOfDay",
                    "interiorExterior",
                    "timeLayer",
                    "weather",
                ] {
                    if raw.get(field).is_some_and(|value| !value.is_string()) {
                        return Err("场次元数据文本字段类型无效".into());
                    }
                }
                if raw.get("characters").is_some_and(|value| {
                    value
                        .as_array()
                        .is_none_or(|items| items.iter().any(|item| !item.is_string()))
                }) {
                    return Err("场次人物清单类型无效".into());
                }
                if raw
                    .keys()
                    .all(|key| ["episodeId", "sceneId", "sourceRefs"].contains(&key.as_str()))
                {
                    return Err("场次计划没有实际修改字段".into());
                }
            } else {
                if raw
                    .keys()
                    .any(|key| !["episodeId", "sceneId", "cuts"].contains(&key.as_str()))
                {
                    return Err("拆场计划含未知字段".into());
                }
                let cuts = raw
                    .get("cuts")
                    .and_then(Value::as_array)
                    .filter(|a| !a.is_empty())
                    .ok_or("拆场须有内部边界")?;
                let mut seen = HashSet::new();
                let mut last = 0;
                for cut in cuts {
                    if cut.as_object().is_none_or(|raw| {
                        raw.keys()
                            .any(|key| !["rowId", "anchor"].contains(&key.as_str()))
                    }) {
                        return Err("拆场切点包含未知字段".into());
                    }
                    let id = cut["rowId"].as_str().ok_or("拆场缺rowId")?;
                    let anchor = cut["anchor"].as_str().ok_or("拆场缺anchor")?;
                    let row = source
                        .rows
                        .iter()
                        .find(|r| r.id == id)
                        .ok_or("拆场源行不存在")?;
                    let first = source
                        .scenes
                        .iter()
                        .find(|s| s.id == scene)
                        .and_then(|s| s.row_ids.first());
                    if !batch.row_ids().contains(id)
                        || row.scene_id.as_deref() != Some(scene)
                        || first.map(String::as_str) == Some(id)
                        || row.text.trim_end_matches(['\r', '\n']) != anchor
                        || !seen.insert(id)
                        || row.range.start <= last
                    {
                        return Err("拆场锚点非内部有序逐字原文边界".into());
                    }
                    last = row.range.start;
                    refs.push(id.into());
                }
            }
            let mut plan = value.clone();
            if kind == "sceneMetadata" {
                plan["sourceRefs"] = json!(refs);
            }
            result.push(FoundationIssue {
                coordinate: format!("program:{kind}:{scene}"),
                problem: "已保存有据场次程序计划，尚未发布到锁定来源；由程序处理，不重试模型"
                    .into(),
                source_refs: refs,
                blocking: false,
                asset_id: None,
                variant_id: None,
                episode_ids: vec![episode.into()],
                details: Some(
                    json!({"owner":"PROGRAM","method":METHOD_VERSION,"planKind":kind,"plan":plan,"requiresProgramExecution":true}),
                ),
            });
        }
    }
    Ok(result)
}
fn decision_contains_place(decision: &Value, ep: &str, scene: Option<&str>) -> bool {
    match decision["applicationScope"]["kind"].as_str().unwrap_or("") {
        "GLOBAL" => true,
        "EPISODES" => decision["episodes"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| e.as_str() == Some(ep)),
        "SCENES" => scene.is_some_and(|sc| {
            decision["applicationScope"]["scenes"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p[0].as_str() == Some(ep) && p[1].as_str() == Some(sc))
        }),
        _ => false,
    }
}
fn decision_covers_asset(decision: &Value, asset: &FoundationAsset) -> bool {
    asset.variants.iter().all(|v| {
        if v.scope.kind == "GLOBAL" {
            decision["applicationScope"]["kind"] == "GLOBAL"
        } else {
            !v.scope.scenes.is_empty()
                && v.scope
                    .scenes
                    .iter()
                    .all(|p| decision_contains_place(decision, &p.episode_id, Some(&p.scene_id)))
        }
    })
}
fn decision_receipts(value: &Value, input: &Value) -> Result<Vec<Value>, String> {
    let decisions = input["confirmedDecisions"]
        .as_array()
        .ok_or("采用决定缺冻结契约")?;
    let replies = value["appliedDecisions"]
        .as_array()
        .ok_or("Q1未交appliedDecisions采用回执")?;
    let mut receipts = vec![];
    for reply in replies {
        let map = reply.as_object().ok_or("采用回执须为对象")?;
        if map.keys().any(|k| !["id", "note"].contains(&k.as_str())) {
            return Err("采用回执不能修改答案或契约".into());
        }
        let id = reply["id"].as_str().ok_or("采用回执缺准确id")?;
        let note = reply["note"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("采用回执缺实际应用依据")?;
        let decision = decisions
            .iter()
            .find(|d| d["id"].as_str() == Some(id))
            .ok_or("Q1回执未提供的已确认决定")?;
        if decision["status"] != "CONFIRMED"
            || decision["sourceSha256"] != input["sourceSha256"]
            || decision["answer"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
        {
            return Err("采用决定尚未确认或来源失效".into());
        }
        if receipts.iter().any(|r: &Value| r["id"] == id) {
            return Err("Q1重复采用同一决定".into());
        }
        receipts.push(json!({"id":id,"revision":decision["revision"],"note":note,"coverage":input["scope"].as_array().into_iter().flatten().filter(|ep|decision["episodes"].as_array().into_iter().flatten().any(|d|d==*ep)).collect::<Vec<_>>()}));
    }
    if receipts.len() != decisions.len() {
        return Err("Q1遗漏本批已确认决定，未声称采用".into());
    }
    Ok(receipts)
}
fn validate_decision_changes(
    value: &Value,
    source: &LockedSource,
    batch: &Batch,
    prior: &FoundationProposal,
    input: &Value,
) -> Result<(), String> {
    decision_receipts(value, input)?;
    let decisions = input["confirmedDecisions"]
        .as_array()
        .ok_or("采用决定缺冻结契约")?;
    let changes = value["changes"].as_object().ok_or("Q1采用决定缺差异对象")?;
    for field in ["removeAssetKeys", "mergeAssets", "sceneSplits", "episodes"] {
        if changes
            .get(field)
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return Err("采用决定不允许删除、归并、拆场或重编分集".into());
        }
    }
    if changes.get("overview").is_some_and(|v| !v.is_null())
        || value.get("seriesDirection").is_some_and(|v| !v.is_null())
    {
        return Err("局部制作决定不得改全剧方向或概览".into());
    }
    for patch in changes
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = patch["key"].as_str().ok_or("决定修订须有原登记资产key")?;
        let asset = prior
            .assets
            .iter()
            .find(|a| a.id == key)
            .ok_or("决定不能创建未授权身份")?;
        let permitted = |field: &str, variant: Option<&FoundationVariant>| {
            decisions.iter().any(|d| {
                d["interpretationOnly"] != true
                    && d["changeTargets"]["assetKeys"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|k| k.as_str() == Some(key))
                    && d["changeTargets"]["assetFields"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|f| f.as_str() == Some(field))
                    && variant
                        .map(|v| {
                            if v.scope.kind == "GLOBAL" {
                                d["applicationScope"]["kind"] == "GLOBAL"
                            } else {
                                !v.scope.scenes.is_empty()
                                    && v.scope.scenes.iter().all(|p| {
                                        decision_contains_place(d, &p.episode_id, Some(&p.scene_id))
                                    })
                            }
                        })
                        .unwrap_or_else(|| decision_covers_asset(d, asset))
            })
        };
        for (field, v) in patch.as_object().ok_or("资产差异须为对象")? {
            if v.is_null() || field == "key" {
                continue;
            }
            if field == "variants" {
                for variant in v.as_array().ok_or("形象差异须为数组")? {
                    let id = variant["key"].as_str().ok_or("决定形象修订须有准确key")?;
                    let old = asset.variants.iter().find(|old| old.id == id);
                    if !permitted("variants", old) {
                        return Err("决定未获该形象/范围的variants修改权限".into());
                    }
                    for pair in variant["sceneRefs"].as_array().into_iter().flatten() {
                        let ep = pair[0].as_str().ok_or("形象差异缺集范围")?;
                        let sc = pair[1].as_str().ok_or("形象差异缺场范围")?;
                        if !batch.scope.iter().any(|e| e == ep)
                            || !source
                                .scenes
                                .iter()
                                .any(|s| s.episode_id == ep && s.id == sc)
                            || !decisions.iter().any(|d| {
                                d["changeTargets"]["assetKeys"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|k| k.as_str() == Some(key))
                                    && d["changeTargets"]["assetFields"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .any(|f| f == "variants")
                                    && decision_contains_place(d, ep, Some(sc))
                            })
                        {
                            return Err("决定形象修订越出授权集场".into());
                        }
                    }
                    if old.is_none() && variant["sceneRefs"].as_array().is_none_or(|a| a.is_empty())
                    {
                        return Err("新局部形象须有明确授权场引用".into());
                    }
                    if variant["scope"]["kind"] == "GLOBAL"
                        && !decisions.iter().any(|d| {
                            d["applicationScope"]["kind"] == "GLOBAL"
                                && d["changeTargets"]["assetKeys"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|k| k.as_str() == Some(key))
                        })
                    {
                        return Err("局部选择不得扩大为GLOBAL形象".into());
                    }
                }
            } else if !CHOICE_ASSET_FIELDS.contains(&field.as_str()) || !permitted(field, None) {
                return Err(format!("决定没有完整用途范围内的资产 {field} 修改权限"));
            }
        }
    }
    for patch in changes
        .get("scenes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ep = patch["episodeId"].as_str().ok_or("场次修订缺集ID")?;
        let scene = patch["sceneId"].as_str().ok_or("场次修订缺场ID")?;
        if !batch.scope.iter().any(|e| e == ep)
            || !source
                .scenes
                .iter()
                .any(|s| s.episode_id == ep && s.id == scene)
        {
            return Err("决定场次修订越出本批来源".into());
        }
        for (field, v) in patch.as_object().ok_or("场次差异须为对象")? {
            if v.is_null() || ["episodeId", "sceneId"].contains(&field.as_str()) {
                continue;
            }
            if !CHOICE_SCENE_FIELDS.contains(&field.as_str())
                || !decisions.iter().any(|d| {
                    d["interpretationOnly"] != true
                        && decision_contains_place(d, ep, Some(scene))
                        && d["changeTargets"]["sceneFields"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|f| f.as_str() == Some(field))
                })
            {
                return Err(format!(
                    "决定无场次 {field} 修改权限；原文说者、台词和动作不可改"
                ));
            }
        }
    }
    if value["humanChoices"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        return Err("采用已确认决定不能重新创建AUTHOR问题".into());
    }
    Ok(())
}
fn decision_resolves_pending(issue: &FoundationIssue, input: &Value) -> bool {
    issue.details.as_ref().is_some_and(|d| d["owner"] == "Q1")
        && !issue.source_refs.is_empty()
        && input["confirmedDecisions"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|d| {
                issue.episode_ids.iter().all(|ep| {
                    d["episodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|e| e.as_str() == Some(ep))
                }) && issue.source_refs.iter().any(|id| {
                    d["sourceRefs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|r| r.as_str() == Some(id))
                })
            })
}

fn auto_receipt_closes_author(issue: &FoundationIssue, records: &[Value]) -> bool {
    let Some(details) = issue.details.as_ref().filter(|d| d["owner"] == "AUTHOR") else {
        return false;
    };
    let Some(choice) = details.get("choice") else {
        return false;
    };
    records.iter().any(|record| {
        record["status"] == "APPLIED"
            && record["members"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|member| {
                    member["coordinate"] == issue.coordinate
                        && member["choiceSha256"] == digest(&choice.to_string())
                })
    })
}

fn auto_receipt_resolves_pending(
    issue: &FoundationIssue,
    records: &[Value],
    source: &LockedSource,
) -> bool {
    issue.details.as_ref().is_some_and(|d| {
        d["owner"] == "Q1"
            && d.pointer("/pending/owner")
                .is_some_and(|owner| owner == "Q1")
    }) && !issue.source_refs.is_empty()
        && !issue.episode_ids.is_empty()
        && records.iter().any(|record| {
            record["status"] == "APPLIED"
                && record["sourceSha256"] == source.sha256
                && issue.episode_ids.iter().all(|ep| {
                    record["episodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|e| e.as_str() == Some(ep))
                })
                && issue.source_refs.iter().all(|id| {
                    record["sourceRefs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|r| r.as_str() == Some(id))
                })
                && issue.asset_id.as_ref().is_none_or(|key| {
                    record["subjectAssetKeys"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|subject| subject.as_str() == Some(key))
                })
                && issue.variant_id.is_none()
                && issue
                    .source_refs
                    .iter()
                    .filter_map(|id| source.rows.iter().find(|row| &row.id == id))
                    .any(|row| {
                        row.episode_id.as_ref().is_some_and(|ep| {
                            decision_contains_place(record, ep, row.scene_id.as_deref())
                        })
                    })
        })
}

fn prior_auto_records(foundation: &FoundationProposal) -> Vec<Value> {
    let mut records = vec![];
    for record in foundation
        .assets
        .iter()
        .flat_map(|a| {
            a.metadata
                .get("authorDecisions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .chain(foundation.issues.iter().filter_map(|i| {
            i.details
                .as_ref()
                .filter(|d| d["kind"] == "APPLIED_INTERPRETATION")
                .and_then(|d| d.get("authorDecision"))
        }))
    {
        if record["decisionPolicy"] == AUTO_INTERPRET_POLICY
            && record["sourceSha256"] == foundation.source_sha256
            && !records.iter().any(|old: &Value| old["id"] == record["id"])
        {
            records.push(record.clone());
        }
    }
    records
}

fn sent_decision_evidence(input: &Value, original: &SourceRow) -> bool {
    let mut found = false;
    for (columns, rows) in [
        (
            input
                .pointer("/source/columns")
                .cloned()
                .unwrap_or(Value::Null),
            input
                .pointer("/source/rows")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            input
                .pointer("/evidence/backgroundColumns")
                .cloned()
                .unwrap_or(Value::Null),
            input
                .pointer("/evidence/background")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            input
                .pointer("/evidence/backgroundColumns")
                .cloned()
                .unwrap_or(Value::Null),
            input
                .pointer("/evidence/backgroundCandidates")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            input
                .pointer("/evidence/historicalColumns")
                .cloned()
                .unwrap_or(Value::Null),
            input
                .pointer("/evidence/historicalEvidenceCandidates")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            input
                .pointer("/evidence/automaticDecisionSupport/columns")
                .cloned()
                .unwrap_or(Value::Null),
            input
                .pointer("/evidence/automaticDecisionSupport/rows")
                .cloned()
                .unwrap_or(Value::Null),
        ),
    ] {
        let Some(columns) = columns.as_array() else {
            continue;
        };
        let Ok(rows) = expand_history(&rows) else {
            return false;
        };
        let Some(id_at) = columns.iter().position(|column| column == "rowId") else {
            continue;
        };
        let Some(text_at) = columns.iter().position(|column| column == "text") else {
            continue;
        };
        for row in rows
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| row.get(id_at).and_then(Value::as_str) == Some(original.id.as_str()))
        {
            if row.get(text_at).and_then(Value::as_str) != Some(original.text.as_str()) {
                return false;
            }
            for (column, expected) in [
                ("episodeId", json!(original.episode_id)),
                ("sceneId", json!(original.scene_id)),
                (
                    "claim",
                    json!(if original.dialogue.is_some() {
                        "CHARACTER_CLAIM"
                    } else {
                        "SOURCE"
                    }),
                ),
            ] {
                if let Some(at) = columns.iter().position(|field| field == column) {
                    if row.get(at) != Some(&expected) {
                        return false;
                    }
                }
            }
            found = true;
        }
    }
    found
}

fn automatic_receipts(
    value: &Value,
    source: &LockedSource,
    batch: &Batch,
    prior: &FoundationProposal,
    input: &Value,
    frozen_receipt_times: Option<&HashMap<String, String>>,
) -> Result<Vec<Value>, String> {
    let offered = input["automaticDecisions"]
        .as_array()
        .ok_or("自动解释缺本批准确契约")?;
    let replies = value["appliedDecisions"]
        .as_array()
        .ok_or("原Q1未交自动制作解释采用回执")?;
    let mut combined = prior.clone();
    let new_choices = value["humanChoices"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    combined.issues.extend(supplemental_issues(
        &new_choices,
        &[],
        &[],
        batch,
        &allowed_refs(input),
    )?);
    let context = json!({"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundation":{"version":input["foundationVersion"]}});
    let view = auto_view(source, &combined, &context)?;
    let contracts = automatic_decisions(&view, &input["userReviewRequirements"])?;
    let expected: Vec<_> = contracts
        .iter()
        .filter(|c| {
            c["episodes"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|ep| batch.scope.iter().any(|b| Some(b.as_str()) == ep.as_str()))
        })
        .filter(|c| {
            offered.iter().any(|d| d["id"] == c["id"])
                || new_choices
                    .iter()
                    .any(|d| d["decisionKey"] == c["decisionKey"])
        })
        .collect();
    if offered.iter().any(|d| {
        d["sourceSha256"] != source.sha256
            || d["status"] != "MODEL_PENDING"
            || !expected.iter().any(|c| {
                c["id"] == d["id"]
                    && c["revision"] == d["revision"]
                    && c["sourceRefs"] == d["sourceRefs"]
                    && c["applicationScope"] == d["applicationScope"]
                    && c["subjectAssetKeys"] == d["subjectAssetKeys"]
            })
    }) {
        return Err("自动解释契约的来源、版本、主体或范围已变化".into());
    }
    let previous = prior_auto_records(prior);
    let mut records = vec![];
    for reply in replies {
        let fields = reply.as_object().ok_or("自动解释采用回执须为对象")?;
        if fields.keys().any(|k| {
            !["id", "decisionKey", "answer", "note", "subjectAssetKeys"].contains(&k.as_str())
        }) {
            return Err("自动解释不能修改源版本、范围、证据或权限".into());
        }
        let candidates: Vec<_> = expected
            .iter()
            .copied()
            .filter(|c| {
                if let Some(id) = reply["id"].as_str() {
                    offered.iter().any(|d| d["id"] == id) && c["id"] == id
                } else {
                    reply["decisionKey"].as_str().is_some_and(|key| {
                        c["decisionKey"]
                            .as_str()
                            .is_some_and(|k| choice_key(k) == choice_key(key))
                    }) && !offered.iter().any(|d| d["id"] == c["id"])
                }
            })
            .collect();
        if candidates.len() != 1 {
            return Err("自动解释回执缺准确已提供id，或新decisionKey不存在/有歧义".into());
        }
        let contract = candidates[0];
        if reply
            .get("decisionKey")
            .is_some_and(|key| key != &contract["decisionKey"])
            || records.iter().any(|r: &Value| r["id"] == contract["id"])
        {
            return Err("自动解释回执身份错配或重复".into());
        }
        let answer = reply["answer"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("自动解释缺模型实际选用的制作解释")?;
        let note = reply["note"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("自动解释缺实际采用依据")?;
        let refs = choice_strings(&contract["sourceRefs"], "自动解释原文证据")?;
        // Supporting evidence is not an application permission. A father's
        // earlier dialogue may support a later scene's interpretation without
        // authorizing an edit or applying that interpretation to the old scene.
        // Still require real, exactly supplied source text and a local anchor.
        if refs.is_empty()
            || refs.iter().any(|id| {
                source
                    .rows
                    .iter()
                    .find(|row| &row.id == id)
                    .is_none_or(|row| !sent_decision_evidence(input, row))
            })
            || !refs
                .iter()
                .filter_map(|id| source.rows.iter().find(|row| &row.id == id))
                .any(|row| {
                    row.episode_id.as_ref().is_some_and(|ep| {
                        decision_contains_place(contract, ep, row.scene_id.as_deref())
                    }) || contract["applicationScope"]["kind"] == "GLOBAL"
                        && row.episode_id.is_none()
                })
        {
            return Err(AUTO_INTERPRET_EVIDENCE_REJECTION.into());
        }
        let subjects = if let Some(subjects) = reply.get("subjectAssetKeys") {
            choice_strings(subjects, "自动解释主体")?
        } else {
            choice_strings(&contract["subjectAssetKeys"], "自动解释主体")?
        };
        let original = choice_strings(&contract["subjectAssetKeys"], "自动解释主体")?;
        if !original.is_empty() && subjects != original
            || subjects
                .iter()
                .any(|key| !prior.assets.iter().any(|a| &a.id == key))
        {
            return Err("自动解释不能猜测或替换现有资产主体".into());
        }
        let old = previous.iter().find(|r| same_choice_evidence(r, contract));
        if old.is_some_and(|r| r["answer"] != answer || r["subjectAssetKeys"] != json!(subjects)) {
            return Err("后续容量批次不能改写已采用的同一制作解释或资产主体".into());
        }
        let mut receipts = old
            .and_then(|r| r["receipts"].as_array())
            .cloned()
            .unwrap_or_default();
        let receipt = json!({"id":contract["id"],"revision":contract["revision"],"note":note,"coverage":batch.scope.iter().filter(|ep|contract["episodes"].as_array().into_iter().flatten().any(|p|p.as_str()==Some(ep))).collect::<Vec<_>>()});
        if !receipts.iter().any(|r| r == &receipt) {
            receipts.push(receipt);
        }
        let coverage: HashSet<_> = receipts
            .iter()
            .flat_map(|r| r["coverage"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .collect();
        let complete = contract["episodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|ep| ep.as_str().is_some_and(|ep| coverage.contains(ep)));
        let mut record = contract.clone();
        record["status"] = json!(if complete {
            "APPLIED"
        } else {
            "PARTIALLY_APPLIED"
        });
        record["answer"] = json!(answer);
        record["subjectAssetKeys"] = json!(subjects);
        record["decisionPolicy"] = json!(AUTO_INTERPRET_POLICY);
        record["authority"] = json!("MODEL_PRODUCTION_INTERPRETATION");
        record["receipts"] = json!(receipts);
        record["appliedNote"] = json!(note);
        record["appliedAt"] = if let Some(times) = frozen_receipt_times {
            let id = contract["id"]
                .as_str()
                .ok_or("理由再投影自动解释缺原回执ID")?;
            json!(times
                .get(id)
                .filter(|time| !time.trim().is_empty())
                .ok_or("理由再投影缺原自动解释采用时间，不制造新历史回执")?)
        } else {
            json!(crate::store::now())
        };
        records.push(record);
    }
    if records.len() != expected.len() {
        return Err("原Q1遗漏本批普通AUTHOR的自动解释及采用回执".into());
    }
    Ok(records)
}

fn retain_auto_interpretations(next: &mut FoundationProposal, records: &[Value]) {
    for record in records {
        if record["status"] == "APPLIED" {
            next.issues.retain(|issue| {
                !record["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|member| {
                        issue.coordinate == member["coordinate"]
                            && issue
                                .details
                                .as_ref()
                                .and_then(|d| d.get("choice"))
                                .is_some_and(|c| digest(&c.to_string()) == member["choiceSha256"])
                    })
            });
        }
        next.issues.retain(|issue| {
            issue
                .details
                .as_ref()
                .and_then(|d| d.get("authorDecision"))
                .is_none_or(|r| r["id"] != record["id"])
        });
        next.issues.push(FoundationIssue {
            coordinate: format!("interpretation:{}", record["id"].as_str().unwrap()),
            problem: record["answer"].as_str().unwrap().to_owned(),
            source_refs: record["sourceRefs"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            blocking: false,
            asset_id: record["subjectAssetKeys"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .map(str::to_owned),
            variant_id: None,
            episode_ids: record["episodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            details: Some(
                json!({"owner":"AUTHOR","kind":"APPLIED_INTERPRETATION","authorDecision":record}),
            ),
        });
        for asset in &mut next.assets {
            if record["subjectAssetKeys"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|key| key.as_str() == Some(&asset.id))
            {
                let mut history = asset
                    .metadata
                    .get("authorDecisions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                history.retain(|r| r["id"] != record["id"]);
                history.push(record.clone());
                asset
                    .metadata
                    .insert("authorDecisions".into(), json!(history));
            }
        }
    }
}
fn apply_optimization(
    value: &Value,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    prior: &FoundationProposal,
    confirmed: Option<&str>,
) -> Result<(FoundationProposal, Vec<Value>), String> {
    apply_optimization_with_receipt_times(value, source, batch, input, prior, confirmed, None)
}

fn apply_optimization_with_receipt_times(
    value: &Value,
    source: &LockedSource,
    batch: &Batch,
    input: &Value,
    prior: &FoundationProposal,
    confirmed: Option<&str>,
    frozen_receipt_times: Option<&HashMap<String, String>>,
) -> Result<(FoundationProposal, Vec<Value>), String> {
    let applying =
        input.pointer("/delivery/phase").and_then(Value::as_str) == Some("APPLY_DECISION");
    let automatic = input["foundationDecisionPolicy"] == AUTO_INTERPRET_POLICY;
    let auto_records = if automatic {
        automatic_receipts(value, source, batch, prior, input, frozen_receipt_times)?
    } else {
        vec![]
    };
    if automatic
        && input.pointer("/delivery/phase").and_then(Value::as_str) == Some("AUTO_INTERPRET")
        && value["changes"].as_object().is_none_or(|m| !m.is_empty())
    {
        return Err("自动制作解释不允许修改已验收目录、身份或源事实".into());
    }
    if applying {
        validate_decision_changes(value, source, batch, prior, input)?;
    }
    let wire: OptimizationWire =
        serde_json::from_value(value.clone()).map_err(|e| format!("Q1差异结构无效：{e}"))?;
    check_envelope(
        &wire.format,
        "ASSET_OPTIMIZATION_DESKTOP_V2",
        &wire.source_sha256,
        source,
        &wire.coverage,
        batch,
    )?;
    let allowed = allowed_refs(input);
    check_issues(&wire.issues, batch, &allowed)?;
    // Match the old editor: execute exact split declarations in memory before
    // validating metadata and asset usage of the resulting scene identities.
    // Publication remains a later, atomic program action.
    let split_changes = Changes {
        scene_splits: wire.changes.scene_splits.clone(),
        ..Changes::default()
    };
    let mut program_todos = scene_todos(&split_changes, source, batch, &allowed)?;
    let split_source = source_projection(source, &program_todos, &batch.scope)?;
    let metadata_changes = Changes {
        scenes: wire.changes.scenes.clone(),
        ..Changes::default()
    };
    program_todos.extend(scene_todos(
        &metadata_changes,
        &split_source,
        batch,
        &allowed,
    )?);
    let projected_source = source_projection(source, &program_todos, &batch.scope)?;
    let source = &projected_source;
    let mut next = prior.clone();
    let mut seen = HashSet::new();
    for (ordinal, patch) in wire.changes.assets.iter().enumerate() {
        let asset = build_asset(source, batch, patch, &next, &allowed, true, ordinal)?;
        if !seen.insert(asset.id.clone()) {
            return Err("Q1重复修改同一身份".into());
        }
        if let Some(at) = next.assets.iter().position(|a| a.id == asset.id) {
            next.assets[at] = asset;
        } else {
            next.assets.push(asset);
        }
    }
    if batch.fragment.is_some()
        && (!wire.changes.remove_asset_keys.is_empty() || !wire.changes.merge_assets.is_empty())
    {
        return Err("长集分片不能删除或归并整集身份，应登记局部程序事项".into());
    }
    for key in &wire.changes.remove_asset_keys {
        let asset = next
            .assets
            .iter_mut()
            .find(|a| &a.id == key)
            .ok_or("Q1移出未知身份")?;
        for variant in &mut asset.variants {
            if variant.scope.kind == "GLOBAL" {
                return Err("局部移出不能删除GLOBAL身份的批外用途".into());
            }
            variant
                .scope
                .scenes
                .retain(|s| !batch.scope.contains(&s.episode_id));
            variant.applicability = variant.scope.scenes.clone();
            if variant.scope.scenes.is_empty() {
                variant.scope.kind = "UNRESOLVED".into();
            }
        }
        asset
            .metadata
            .insert("removedFromEpisodes".into(), json!(batch.scope));
    }
    for patch in &wire.changes.merge_assets {
        provided_refs(&patch.source_refs, &allowed)?;
        if patch.from_key == patch.into_key || patch.reason.trim().is_empty() {
            return Err("身份归并须明确不同键及有据原因".into());
        }
        let from = next
            .assets
            .iter()
            .find(|a| a.id == patch.from_key)
            .cloned()
            .ok_or("身份归并来源不存在")?;
        let into = next
            .assets
            .iter()
            .find(|a| a.id == patch.into_key)
            .cloned()
            .ok_or("身份归并目标不存在")?;
        if from.kind != into.kind
            || [&from, &into].iter().any(|a| {
                a.variants.iter().any(|v| {
                    v.scope.kind == "GLOBAL"
                        || v.scope
                            .scenes
                            .iter()
                            .any(|s| !batch.scope.contains(&s.episode_id))
                })
            })
        {
            return Err("本批身份归并影响批外范围或不同分类，必须留明确事项".into());
        }
        if next.assets.iter().flat_map(|a| &a.variants).any(|v| {
            v.source_variant_id
                .as_ref()
                .is_some_and(|id| from.variants.iter().any(|p| &p.id == id))
        }) {
            return Err("身份归并涉及已使用的素材父关系，须先明确重定位".into());
        }
        let target = next
            .assets
            .iter_mut()
            .find(|a| a.id == patch.into_key)
            .unwrap();
        for r in from.source_refs {
            if !target.source_refs.contains(&r) {
                target.source_refs.push(r);
            }
        }
        for v in from.variants {
            if target.variants.iter().any(|p| p.id == v.id) {
                return Err("归并形象键冲突".into());
            }
            target.variants.push(v);
        }
        if !target.aliases.contains(&from.name) {
            target.aliases.push(from.name);
        }
        for alias in from.aliases {
            if !target.aliases.contains(&alias) {
                target.aliases.push(alias);
            }
        }
        next.assets.retain(|a| a.id != patch.from_key);
        for fact in &mut next.facts {
            if fact.asset_id == patch.from_key {
                fact.asset_id = patch.into_key.clone();
            }
        }
        for issue in &mut next.issues {
            if issue.asset_id.as_deref() == Some(&patch.from_key) {
                issue.asset_id = Some(patch.into_key.clone());
            }
        }
    }
    for patch in &wire.changes.episodes {
        if !batch.scope.contains(&patch.episode_id) {
            return Err("分集导航修订越出提供范围".into());
        }
        let refs = if patch.source_refs.is_empty() {
            source
                .rows
                .iter()
                .filter(|r| {
                    r.episode_id.as_ref() == Some(&patch.episode_id)
                        && batch.row_ids().contains(r.id.as_str())
                })
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        } else {
            patch.source_refs.clone()
        };
        provided_refs(&refs, &allowed)?;
        if refs.iter().any(|id| {
            source.rows.iter().any(|r| {
                &r.id == id
                    && r.episode_id
                        .as_ref()
                        .is_some_and(|owner| owner != &patch.episode_id)
            })
        }) {
            return Err("分集导航不能借另一集原文".into());
        }
        let episode = next
            .episodes
            .iter_mut()
            .find(|e| e.episode_id == patch.episode_id)
            .ok_or("所改分集导航未登记")?;
        if let Some(summary) = &patch.summary {
            episode.summary = summary.clone();
        }
        if let Some(context) = &patch.context {
            episode.context = context.clone();
        }
        episode.source_refs = refs;
    }
    if let Some(overview) = &wire.changes.overview {
        next.overview = overview.clone();
    }
    for coordinate in &wire.resolved_issues {
        // An adopted AUTO_INTERPRET receipt closes only its exact AUTHOR
        // members. The later retain step checks the member SHA again and keeps
        // the immutable choice evidence in the applied interpretation audit.
        let matches: Vec<_> = next
            .issues
            .iter()
            .filter(|issue| &issue.coordinate == coordinate)
            .collect();
        if automatic
            && !matches.is_empty()
            && matches
                .iter()
                .all(|issue| auto_receipt_closes_author(issue, &auto_records))
        {
            continue;
        }
        let issue = next
            .issues
            .iter()
            .find(|i| &i.coordinate == coordinate)
            .ok_or("Q1回执未提供的事项")?;
        if issue
            .details
            .as_ref()
            .is_some_and(|d| d["owner"] == "AUTHOR")
            || issue
                .details
                .as_ref()
                .and_then(|d| d.get("owner"))
                .and_then(Value::as_str)
                == Some("PROGRAM")
            || issue.episode_ids.iter().any(|id| !batch.scope.contains(id))
            || wire.changes.assets.is_empty()
                && !retains_registered_preparation(issue, &next)
                && !(applying && decision_resolves_pending(issue, input))
                && !(automatic
                    && matches
                        .iter()
                        .all(|issue| auto_receipt_resolves_pending(issue, &auto_records, source)))
        {
            return Err("Q1不能仅回执关闭程序事项或未实改的事项".into());
        }
        next.issues.retain(|i| &i.coordinate != coordinate);
    }
    if !applying && !automatic && !wire.applied_decisions.is_empty() {
        return Err("普通Q1不能假称采用未提供的确认决定".into());
    }
    next.issues.extend(wire.issues);
    next.issues.extend(supplemental_issues(
        &wire.human_choices,
        &wire.unresolved,
        &wire.notes,
        batch,
        &allowed,
    )?);
    next.issues.extend(program_todos);
    let mut signals = vec![];
    if let Some(direction) = wire.series_direction {
        let raw = direction.as_object().ok_or("方向候选须为对象")?;
        if raw
            .keys()
            .any(|k| !["signals", "content"].contains(&k.as_str()))
        {
            return Err("方向候选含未知字段".into());
        }
        for signal in raw
            .get("signals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let episode = signal["episodeId"].as_str().ok_or("方向依据缺episodeId")?;
            let id = signal["rowId"].as_str().ok_or("方向依据缺rowId")?;
            let quote = signal["quote"]
                .as_str()
                .filter(|q| !q.is_empty())
                .ok_or("方向依据缺逐字引文")?;
            if input
                .pointer("/seriesDirectionPreparation/mode")
                .and_then(Value::as_str)
                == Some("FINAL")
                && input
                    .pointer("/seriesDirectionPreparation/priorSignals")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|prior| {
                        prior["rowId"] == *id
                            && prior["episodeId"] == *episode
                            && prior["quote"] == *quote
                    })
            {
                // Previously verified evidence is already carried in the input.
                // Do not reject or re-add it just because FINAL cites the series.
                continue;
            }
            if !batch.scope.iter().any(|e| e == episode)
                || !batch
                    .pieces
                    .iter()
                    .any(|p| p.row_id == id && p.text.contains(quote))
                || !source
                    .rows
                    .iter()
                    .any(|r| r.id == id && r.episode_id.as_deref() == Some(episode))
            {
                return Err("方向依据不属于本次逐字原文".into());
            }
            if signal["reading"].as_str().is_none() {
                return Err("方向依据解释无效".into());
            }
            signals.push(signal.clone());
        }
        if let Some(content) = raw.get("content") {
            let content = content.as_str().ok_or("方向内容须为文本")?;
            if input
                .pointer("/seriesDirectionPreparation/mode")
                .and_then(Value::as_str)
                != Some("FINAL")
            {
                return Err("仅最后提供批次可汇总方向".into());
            }
            if units(content) > 600 {
                return Err("方向brief超过旧600字符上限".into());
            }
            if confirmed.is_none() {
                next.direction = content.into();
            }
        }
    }
    if let Some(confirmed) = confirmed {
        next.direction = confirmed.into();
    }
    if automatic {
        retain_auto_interpretations(&mut next, &auto_records);
        let review = auto_view(
            source,
            &next,
            &json!({"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundation":{"version":input["foundationVersion"]}}),
        )?;
        let reused: Vec<_> = review["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|record| {
                record["status"] == "APPLIED" && record["decisionPolicy"] == AUTO_INTERPRET_POLICY
            })
            .cloned()
            .collect();
        retain_auto_interpretations(&mut next, &reused);
        for issue in &mut next.issues {
            if review["contracts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| choice_requires_user_review(c, &input["userReviewRequirements"]))
                .any(|c| {
                    c["members"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|m| m["coordinate"] == issue.coordinate)
                })
            {
                issue.blocking = true;
            }
        }
    }
    materialize_foundation(source, &mut next)?;
    validate_foundation(source, &next)?;
    Ok((next, signals))
}

fn source_projection(
    source: &LockedSource,
    issues: &[FoundationIssue],
    selected: &[String],
) -> Result<LockedSource, String> {
    let edits: Vec<_> = issues.iter().filter_map(|issue| {
        let details = issue.details.as_ref()?;
        (details["owner"] == "PROGRAM" && ["sceneMetadata", "sceneSplit"].contains(&details["planKind"].as_str().unwrap_or("")))
            .then(|| json!({"kind":details["planKind"],"sourceSha256":source.sha256,"plan":details["plan"]}))
    }).collect();
    if edits.is_empty() {
        return Ok(source.clone());
    }
    super::scene_edits::preview_foundation_scene_edits(source, &edits, selected)
}

/// Adopt unfinished ordinary AUTHOR interpretations using only original Q1.
/// The full frozen contract is retained by the workflow; model input is scoped.
pub(crate) async fn run_foundation_auto_interpretation(
    source: &LockedSource,
    context: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<(FoundationProposal, Vec<Value>), String> {
    let source = upgrade_source_dialogues(source)?;
    validate_source(&source)?;
    if context["foundationPhase"] != "AUTO_INTERPRET"
        || context["foundationDecisionPolicy"] != AUTO_INTERPRET_POLICY
    {
        return Err("自动制作解释流程未明确授权".into());
    }
    let decisions = context["automaticDecisions"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("没有尚未采用的普通AUTHOR解释")?;
    let mut foundation = existing_foundation(&source, context)?;
    let view = auto_view(&source, &foundation, context)?;
    let expected = automatic_decisions(&view, &context["userReviewRequirements"])?;
    for decision in decisions {
        if decision["status"] != "MODEL_PENDING"
            || !expected.iter().any(|c| {
                [
                    "id",
                    "revision",
                    "sourceSha256",
                    "sourceVersion",
                    "sourceRefs",
                    "applicationScope",
                    "subjectAssetKeys",
                    "members",
                ]
                .iter()
                .all(|k| c[*k] == decision[*k])
            })
        {
            return Err("自动解释冻结契约已失效或包含用户明确要求确认的事项".into());
        }
    }
    if decisions
        .iter()
        .map(|d| &d["id"])
        .collect::<HashSet<_>>()
        .len()
        != decisions.len()
    {
        return Err("自动解释冻结契约重复".into());
    }
    let selected: Vec<_> = source
        .episodes
        .iter()
        .filter(|e| {
            decisions.iter().any(|d| {
                d["episodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|ep| ep.as_str() == Some(&e.id))
            })
        })
        .map(|e| e.id.clone())
        .collect();
    let batches = plan_batches(&source, &selected)?;
    verify_coverage(&source, &selected, &batches)?;
    let mut pending = editor_batches(batches);
    let mut index = 0;
    while !pending.is_empty() {
        let (batch, input) = prepare_capacity_batch(
            "Q1",
            &source,
            &mut pending,
            &foundation,
            context,
            index,
            &[],
        )?;
        let result = call_validated(
            executor,
            "foundation:Q1",
            Q1_RULE,
            model,
            reasoning,
            input.clone(),
            delivery_schema("Q1", &input),
            &[],
            |value| {
                restore_delivery_refs(value, &input)?;
                apply_optimization(value, &source, &batch, &input, &foundation, None).map(|_| ())
            },
        )
        .await?;
        foundation = apply_optimization(&result, &source, &batch, &input, &foundation, None)?.0;
        index += 1;
    }
    let applied = foundation_auto_applied_decisions(&foundation);
    if decisions
        .iter()
        .any(|d| !applied.iter().any(|a| a["id"] == d["id"]))
    {
        return Err("自动制作解释尚未完整覆盖全部冻结范围".into());
    }
    foundation.updated_episode_ids = selected;
    materialize_foundation(&source, &mut foundation)?;
    validate_foundation(&source, &foundation)?;
    Ok((foundation, applied))
}

/// Confirmed AUTHOR decisions continue through the original Q1, never F2.
/// The frozen foundation is the input catalogue; accepted Q1 checkpoints survive
/// interruption and publication waits until every decision's affected scope is covered.
pub(crate) async fn run_foundation_choice_application(
    source: &LockedSource,
    context: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<(FoundationProposal, Vec<Value>), String> {
    let source = upgrade_source_dialogues(source)?;
    validate_source(&source)?;
    if context["foundationPhase"] != "APPLY_DECISION" {
        return Err("制作决定采用阶段不正确".into());
    }
    let decisions = context["confirmedDecisions"]
        .as_array()
        .filter(|d| !d.is_empty())
        .ok_or("没有待采用的已确认制作决定")?;
    let selected: Vec<_> = source
        .episodes
        .iter()
        .filter(|e| {
            decisions.iter().any(|d| {
                d["episodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|id| id.as_str() == Some(&e.id))
            })
        })
        .map(|e| e.id.clone())
        .collect();
    let mut foundation = existing_foundation(&source, context)?;
    for decision in decisions {
        if decision["status"] != "CONFIRMED"
            || decision["sourceSha256"] != source.sha256
            || decision["answer"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
            || decision["episodes"].as_array().is_none_or(|es| {
                es.is_empty()
                    || es.iter().any(|e| {
                        e.as_str()
                            .is_none_or(|id| !selected.iter().any(|e| e == id))
                    })
            })
        {
            return Err("确认决定来源、答案或范围失效".into());
        }
        if decision["interpretationOnly"] != true {
            choice_targets(&decision["changeTargets"], &foundation)?;
        }
        if !decision["members"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|member| {
                foundation.issues.iter().any(|i| {
                    i.coordinate == member["coordinate"]
                        && i.details
                            .as_ref()
                            .and_then(|d| d.get("choice"))
                            .is_some_and(|c| digest(&c.to_string()) == member["choiceSha256"])
                })
            })
        {
            return Err("选择契约已更新，旧确认不能应用到不同问题".into());
        }
    }
    if selected.is_empty() {
        return Err("制作决定没有当前制作范围".into());
    }
    let confirmed = context
        .pointer("/settings/confirmedDirection")
        .and_then(|v| {
            v.as_str()
                .or_else(|| v.get("content").and_then(Value::as_str))
        })
        .filter(|v| !v.trim().is_empty());
    let batches = plan_batches(&source, &selected)?;
    verify_coverage(&source, &selected, &batches)?;
    let mut pending = editor_batches(batches);
    let mut receipts = vec![];
    let mut index = 0;
    while !pending.is_empty() {
        let (batch, input) = prepare_capacity_batch(
            "Q1",
            &source,
            &mut pending,
            &foundation,
            context,
            index,
            &[],
        )?;
        let result = call_validated(
            executor,
            "foundation:Q1",
            Q1_RULE,
            model,
            reasoning,
            input.clone(),
            delivery_schema("Q1", &input),
            &[],
            |value| {
                restore_delivery_refs(value, &input)?;
                apply_optimization(value, &source, &batch, &input, &foundation, confirmed)
                    .map(|_| ())
            },
        )
        .await?;
        let (next, _) =
            apply_optimization(&result, &source, &batch, &input, &foundation, confirmed)?;
        foundation = next;
        receipts.extend(decision_receipts(&result, &input)?);
        index += 1;
    }
    let mut applied = vec![];
    for decision in decisions {
        let own: Vec<_> = receipts
            .iter()
            .filter(|r| r["id"] == decision["id"])
            .cloned()
            .collect();
        let coverage: HashSet<_> = own
            .iter()
            .flat_map(|r| r["coverage"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .collect();
        if decision["episodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|ep| ep.as_str().is_none_or(|ep| !coverage.contains(ep)))
        {
            return Err("采用回执未完整覆盖确认决定范围".into());
        }
        foundation.issues.retain(|issue| {
            !decision["members"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|member| {
                    issue.coordinate == member["coordinate"]
                        && issue
                            .details
                            .as_ref()
                            .and_then(|d| d.get("choice"))
                            .is_some_and(|c| digest(&c.to_string()) == member["choiceSha256"])
                })
        });
        let mut record = decision.clone();
        record["status"] = json!("APPLIED");
        record["receipts"] = json!(own);
        record["appliedNote"] = json!(own
            .iter()
            .filter_map(|r| r["note"].as_str())
            .collect::<Vec<_>>()
            .join("\n"));
        record["appliedAt"] = json!(crate::store::now());
        // Explanations are clearly typed, source-bound metadata, never SOURCE facts.
        // Preserve scope here so later asset/image/unit prompts cannot treat a local
        // interpretation as a global identity or silently rewrite quoted dialogue.
        for asset in &mut foundation.assets {
            if record["subjectAssetKeys"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|id| id.as_str() == Some(&asset.id))
            {
                let mut history = asset
                    .metadata
                    .get("authorDecisions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                history.retain(|d| d["id"] != record["id"]);
                history.push(record.clone());
                asset
                    .metadata
                    .insert("authorDecisions".into(), json!(history));
            }
        }
        applied.push(record);
    }
    foundation.updated_episode_ids = selected;
    materialize_foundation(&source, &mut foundation)?;
    validate_foundation(&source, &foundation)?;
    Ok((foundation, applied))
}

fn delivery_basis(
    source: &LockedSource,
    context: &Value,
) -> Result<(Vec<String>, FoundationProposal, HashSet<(String, String)>), String> {
    if context
        .pointer("/selection/episodeIds")
        .is_some_and(|value| !value.is_array())
    {
        return Err("制作集选择必须是非空ID数组，不能静默扩大到全剧".into());
    }
    let requested = context
        .pointer("/selection/episodeIds")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .map(|id| id.as_str().map(str::to_string).ok_or("制作集ID无效".into()))
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_else(|| source.episodes.iter().map(|e| e.id.clone()).collect());
    if requested.is_empty()
        || requested.iter().collect::<HashSet<_>>().len() != requested.len()
        || requested
            .iter()
            .any(|id| !source.episodes.iter().any(|e| &e.id == id))
    {
        return Err("请选择当前来源内非空且不重复的制作集范围".into());
    }
    let selected: Vec<_> = source
        .episodes
        .iter()
        .filter(|e| requested.contains(&e.id))
        .map(|e| e.id.clone())
        .collect();
    if source.diagnostics.iter().any(|d| {
        d.blocking
            && (d.row_ids.is_empty()
                || d.row_ids.iter().any(|id| {
                    source
                        .rows
                        .iter()
                        .find(|r| &r.id == id)
                        .is_some_and(|r| r.episode_id.as_ref().is_none_or(|e| selected.contains(e)))
                }))
    }) {
        return Err("所选来源仍有集场/对白待确认；程序保真不能猜测".into());
    }
    let mut foundation = existing_foundation(source, context)?;
    let resolved: HashSet<_> = foundation
        .assets
        .iter()
        .flat_map(|a| {
            a.variants
                .iter()
                .filter(|v| effective_scope(v).kind != "UNRESOLVED")
                .map(|v| (a.id.clone(), v.id.clone()))
        })
        .collect();
    foundation
        .episodes
        .retain(|e| !selected.contains(&e.episode_id));
    for asset in &mut foundation.assets {
        for v in &mut asset.variants {
            if v.scope.kind == "SCENES" {
                v.scope.scenes.retain(|s| !selected.contains(&s.episode_id));
                v.applicability = v.scope.scenes.clone();
                if v.scope.scenes.is_empty() {
                    v.scope.kind = "UNRESOLVED".into();
                }
            }
        }
    }
    foundation.facts.retain_mut(|f| {
        f.source_refs.retain(|id| {
            source
                .rows
                .iter()
                .find(|r| &r.id == id)
                .is_none_or(|r| r.episode_id.as_ref().is_none_or(|e| !selected.contains(e)))
        });
        !f.source_refs.is_empty()
    });
    foundation
        .issues
        .retain(|i| !i.episode_ids.iter().any(|e| selected.contains(e)));
    for episode in source.episodes.iter().filter(|e| selected.contains(&e.id)) {
        let row = source
            .rows
            .iter()
            .find(|r| r.episode_id.as_ref() == Some(&episode.id))
            .ok_or("制作集没有锁定原行")?;
        foundation.episodes.push(FoundationEpisode {
            episode_id: episode.id.clone(),
            summary: episode.title.clone(),
            context: String::new(),
            source_refs: vec![row.id.clone()],
        });
    }
    Ok((selected, foundation, resolved))
}

/// One accepted DOMAIN result and its original frozen DOMAIN input. The caller
/// must bind these to completed workflow/text-job/raw/receipt/request and frozen
/// rule hashes before calling this pure projector. A raw transport response is
/// not an accepted domain result, and this function never invokes an executor.
pub(crate) struct AcceptedFoundationReasonStep<'a> {
    pub stage: &'a str,
    pub input: &'a Value,
    pub result: &'a Value,
}

#[derive(Debug, Clone)]
pub(crate) struct FoundationReasonProjection {
    pub foundation: FoundationProposal,
    pub restored_variant_ids: Vec<String>,
    pub projected_source: LockedSource,
}

#[cfg(test)]
pub(crate) fn reason_projection_test_fixture() -> (
    LockedSource,
    Value,
    FoundationProposal,
    Vec<(String, Value, Value)>,
) {
    let (source, context, current, f2_input, f2_raw, q1_input, q1_raw) =
        tests::scope_reasons_saved_chain();
    (
        source,
        context,
        current,
        vec![
            ("foundation:F2".into(), f2_input, f2_raw),
            ("foundation:Q1".into(), q1_input, q1_raw),
        ],
    )
}

fn reason_replay_batch(
    source: &LockedSource,
    selected: &[String],
    stage: &str,
    index: usize,
    input: &Value,
) -> Result<Batch, String> {
    let scope: Vec<String> =
        serde_json::from_value(input["scope"].clone()).map_err(|_| "理由再投影缺原冻结集范围")?;
    let ordered: Vec<_> = source
        .episodes
        .iter()
        .filter(|episode| scope.contains(&episode.id))
        .map(|episode| episode.id.clone())
        .collect();
    if scope.is_empty()
        || scope != ordered
        || scope.iter().any(|episode| !selected.contains(episode))
        || input["method"] != METHOD_VERSION
        || input["sourceSha256"] != source.sha256
        || input.pointer("/delivery/method").and_then(Value::as_str) != Some(METHOD_VERSION)
        || input.pointer("/delivery/stage").and_then(Value::as_str) != Some(stage)
        || input["batchIndex"].as_u64() != Some(index as u64)
        || input
            .pointer("/delivery/batchIndex")
            .and_then(Value::as_u64)
            != Some(index as u64)
        || input.pointer("/delivery/scope") != Some(&input["scope"])
        || input.pointer("/source/columns")
            != Some(&json!([
                "rowId",
                "line",
                "episodeId",
                "sceneId",
                "kind",
                "speaker",
                "channel",
                "start",
                "end",
                "text"
            ]))
        || input
            .pointer("/source/coordinateSystem")
            .and_then(Value::as_str)
            != Some("UTF-16 half-open")
        || input.pointer("/delivery/phase").and_then(Value::as_str)
            != Some(if stage == "F2" { "GENERATE_F2" } else { "EDIT" })
    {
        return Err("理由再投影冻结来源、阶段或批次身份不一致".into());
    }
    let rows = input
        .pointer("/source/rows")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or("理由再投影缺原冻结来源行")?;
    let mut pieces = Vec::new();
    let mut last_position = None;
    for value in rows {
        let fields = value
            .as_array()
            .filter(|fields| fields.len() == 10)
            .ok_or("理由再投影来源行坐标结构无效")?;
        let (position, row) = source
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| fields[0].as_str() == Some(row.id.as_str()))
            .ok_or("理由再投影引用未知原行")?;
        let start = fields[7]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("理由再投影行起点无效")?;
        let end = fields[8]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("理由再投影行终点无效")?;
        if last_position.is_some_and(|last| position <= last)
            || row
                .episode_id
                .as_ref()
                .is_none_or(|episode| !scope.contains(episode))
            || start < row.range.start
            || end > row.range.end
            || start >= end
            || fields[..7]
                != json!([
                    row.id,
                    row.number,
                    row.episode_id,
                    row.scene_id,
                    row.kind,
                    row.dialogue.as_ref().map(|dialogue| &dialogue.speaker),
                    row.dialogue.as_ref().map(|dialogue| &dialogue.channel)
                ])
                .as_array()
                .unwrap()[..]
        {
            return Err("理由再投影原行身份、集场、顺序或区间不一致".into());
        }
        let text = slice_utf16(&source.text, &SourceRange { start, end })?;
        if fields[9].as_str() != Some(text.as_str()) {
            return Err("理由再投影原冻结区间文本不一致".into());
        }
        pieces.push(Piece {
            row_id: row.id.clone(),
            start,
            end,
            text,
        });
        last_position = Some(position);
    }
    let fragment = input
        .pointer("/delivery/fragment")
        .filter(|value| !value.is_null())
        .cloned();
    let batch = Batch {
        scope,
        pieces,
        fragment,
    };
    if input["batch"].as_str() != Some(batch.key().as_str()) {
        return Err("理由再投影冻结批次摘要不一致".into());
    }
    if let Some(fragment) = &batch.fragment {
        let episode = fragment["episodeId"]
            .as_str()
            .ok_or("理由再投影长集片段无效")?;
        let start = fragment["start"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or("理由再投影片段起点无效")?;
        let end = fragment["end"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or("理由再投影片段终点无效")?;
        let all = whole_rows(source, &batch.scope);
        if batch.scope != vec![episode.to_owned()]
            || fragment["sourceSha256"] != source.sha256
            || fragment["index"].as_u64().is_none()
            || fragment["count"]
                .as_u64()
                .is_none_or(|count| fragment["index"].as_u64().unwrap() >= count)
            || start >= end
            || end > all.iter().map(|piece| units(&piece.text)).sum::<usize>()
        {
            return Err("理由再投影片段身份或范围无效".into());
        }
        let mut cursor = 0;
        let mut expected = Vec::new();
        for piece in all {
            let limit = cursor + units(&piece.text);
            let a = start.max(cursor);
            let b = end.min(limit);
            if b > a {
                expected.push(json!([
                    piece.row_id,
                    piece.start + a - cursor,
                    piece.start + b - cursor
                ]));
            }
            cursor = limit;
        }
        if expected
            != batch
                .pieces
                .iter()
                .map(|p| json!([p.row_id, p.start, p.end]))
                .collect::<Vec<_>>()
        {
            return Err("理由再投影片段未精确包含原冻结区间".into());
        }
    } else if batch
        .pieces
        .iter()
        .map(|p| json!([p.row_id, p.start, p.end]))
        .collect::<Vec<_>>()
        != whole_rows(source, &batch.scope)
            .iter()
            .map(|p| json!([p.row_id, p.start, p.end]))
            .collect::<Vec<_>>()
    {
        return Err("理由再投影完整批次遗漏原行".into());
    }
    let ids = batch.row_ids();
    let scenes = source
        .scenes
        .iter()
        .filter(|scene| scene.row_ids.iter().any(|id| ids.contains(id.as_str())))
        .map(|scene| json!([scene.episode_id, scene.id, scene.title]))
        .collect::<Vec<_>>();
    if input["scenes"] != json!(scenes) {
        return Err("理由再投影原冻结集场身份不一致".into());
    }
    Ok(batch)
}

fn catalogue_without_scope_reasons(foundation: &FoundationProposal) -> Value {
    let mut assets = json!(foundation.assets);
    for asset in assets.as_array_mut().unwrap() {
        for variant in asset["variants"].as_array_mut().unwrap() {
            variant["scope"]["reason"] = Value::Null;
        }
    }
    assets
}

/// Prepare a reason-only projection for a NEW workflow. No database, snapshot,
/// historic result or source is mutated. Missing/ambiguous evidence and any
/// catalogue change apart from reasons fail closed; callers retain `current`.
pub(crate) fn project_missing_scope_reasons(
    source: &LockedSource,
    original_workflow_input: &Value,
    current: &FoundationProposal,
    steps: &[AcceptedFoundationReasonStep<'_>],
) -> Result<FoundationReasonProjection, String> {
    let source = upgrade_source_dialogues(source)?;
    validate_source(&source)?;
    if current.source_sha256 != source.sha256 || steps.is_empty() {
        return Err("理由再投影缺同源已验收目录或原步骤".into());
    }
    let (selected, mut replayed, resolved) = delivery_basis(&source, original_workflow_input)?;
    // Scope projection must not mint a new history timestamp. All other receipt
    // fields are recalculated from the original accepted result and still take
    // part in the exact catalogue comparison below.
    let receipt_times: HashMap<String, String> = prior_auto_records(current)
        .iter()
        .filter_map(|record| {
            Some((
                record["id"].as_str()?.to_owned(),
                record["appliedAt"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let confirmed = original_workflow_input
        .pointer("/settings/confirmedDirection")
        .and_then(|v| {
            v.as_str()
                .or_else(|| v.get("content").and_then(Value::as_str))
        })
        .filter(|s| !s.trim().is_empty());
    let mut f2_batches = Vec::new();
    let mut q1_batches = Vec::new();
    for step in steps {
        let mut result = step.result.clone();
        restore_delivery_refs(&mut result, step.input)?;
        match step.stage {
            "foundation:F2" if q1_batches.is_empty() => {
                let batch =
                    reason_replay_batch(&source, &selected, "F2", f2_batches.len(), step.input)?;
                replayed = apply_catalogue(
                    &result, &source, &batch, step.input, &replayed, confirmed, &resolved,
                )?;
                f2_batches.push(batch);
            }
            "foundation:Q1" if !f2_batches.is_empty() => {
                verify_coverage(&source, &selected, &f2_batches)?;
                let projected = source_projection(&source, &replayed.issues, &selected)?;
                let batch =
                    reason_replay_batch(&projected, &selected, "Q1", q1_batches.len(), step.input)?;
                (replayed, _) = apply_optimization_with_receipt_times(
                    &result,
                    &projected,
                    &batch,
                    step.input,
                    &replayed,
                    confirmed,
                    Some(&receipt_times),
                )?;
                q1_batches.push(batch);
            }
            _ => return Err("理由再投影只支持完整有序的已接受 F2→Q1 目录步骤".into()),
        }
    }
    if f2_batches.is_empty() || q1_batches.is_empty() {
        return Err("理由再投影缺完整已接受 F2／Q1 链".into());
    }
    verify_coverage(&source, &selected, &f2_batches)?;
    verify_coverage(&source, &selected, &q1_batches)?;
    let projected_source = source_projection(&source, &replayed.issues, &selected)?;
    materialize_foundation(&projected_source, &mut replayed)?;
    validate_foundation(&projected_source, &replayed)?;
    if catalogue_without_scope_reasons(current) != catalogue_without_scope_reasons(&replayed) {
        return Err("当前目录与原已接受链不再精确一致；不自动恢复或覆盖人工修改".into());
    }
    let mut foundation = current.clone();
    let mut restored_variant_ids = Vec::new();
    for asset in &mut foundation.assets {
        let original = replayed.assets.iter().find(|a| a.id == asset.id).unwrap();
        for variant in &mut asset.variants {
            let original = original
                .variants
                .iter()
                .find(|v| v.id == variant.id)
                .unwrap();
            if variant.scope.kind == "SCENES"
                && variant
                    .scope
                    .reason
                    .as_deref()
                    .is_none_or(|reason| reason.trim().is_empty())
                && original
                    .scope
                    .reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty())
            {
                variant.scope.reason = original.scope.reason.clone();
                restored_variant_ids.push(variant.id.clone());
            }
        }
    }
    Ok(FoundationReasonProjection {
        foundation,
        restored_variant_ids,
        projected_source,
    })
}

/// No F1 model call: the trusted source contract and explicit decisions already
/// establish source fidelity. F2 is packed across episodes; Q1 edits differences.
pub async fn run_foundation_delivery(
    source: &LockedSource,
    context: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<FoundationProposal, String> {
    let source = upgrade_source_dialogues(source)?;
    validate_source(&source)?;
    let (selected, mut foundation, resolved) = delivery_basis(&source, context)?;
    let confirmed = context
        .pointer("/settings/confirmedDirection")
        .and_then(|v| {
            v.as_str()
                .or_else(|| v.get("content").and_then(Value::as_str))
        })
        .filter(|s| !s.trim().is_empty());
    let mut pending = plan_batches(&source, &selected)?;
    let mut accepted_batches = vec![];
    let mut call_index = 0;
    while !pending.is_empty() {
        let (batch, input) = prepare_capacity_batch(
            "F2",
            &source,
            &mut pending,
            &foundation,
            context,
            call_index,
            &[],
        )?;
        let result = call_validated(
            executor,
            "foundation:F2",
            F2_RULE,
            model,
            reasoning,
            input.clone(),
            delivery_schema("F2", &input),
            &[],
            |value| {
                restore_delivery_refs(value, &input)?;
                apply_catalogue(
                    value,
                    &source,
                    &batch,
                    &input,
                    &foundation,
                    confirmed,
                    &resolved,
                )
                .map(|_| ())
            },
        )
        .await?;
        foundation = apply_catalogue(
            &result,
            &source,
            &batch,
            &input,
            &foundation,
            confirmed,
            &resolved,
        )?;
        accepted_batches.push(batch);
        call_index += 1;
    }
    verify_coverage(&source, &selected, &accepted_batches)?;
    // Q1 packs every contiguous short-episode group by its *actual* request budget,
    // rather than inheriting F2's 24k source-only limit. Long fragments stay ordered.
    let mut q_pending = editor_batches(accepted_batches);
    let mut signals = vec![];
    let mut q_index = 0;
    while !q_pending.is_empty() {
        let q_source = source_projection(&source, &foundation.issues, &selected)?;
        let (batch, input) = prepare_capacity_batch(
            "Q1",
            &q_source,
            &mut q_pending,
            &foundation,
            context,
            q_index,
            &signals,
        )?;
        let result = call_validated(
            executor,
            "foundation:Q1",
            Q1_RULE,
            model,
            reasoning,
            input.clone(),
            delivery_schema("Q1", &input),
            &[],
            |value| {
                restore_delivery_refs(value, &input)?;
                apply_optimization(value, &q_source, &batch, &input, &foundation, confirmed)
                    .map(|_| ())
            },
        )
        .await?;
        let (next, new_signals) =
            apply_optimization(&result, &q_source, &batch, &input, &foundation, confirmed)?;
        foundation = next;
        signals.extend(new_signals);
        q_index += 1;
    }
    foundation.updated_episode_ids = selected.clone();
    foundation.rule_snapshots = rule_snapshots()
        .into_iter()
        .filter(|r| ["screenplay-fidelity", F2_RULE, Q1_RULE].contains(&r.id.as_str()))
        .collect();
    let projected_source = source_projection(&source, &foundation.issues, &selected)?;
    materialize_foundation(&projected_source, &mut foundation)?;
    validate_foundation(&projected_source, &foundation)?;
    Ok(foundation)
}

#[cfg(test)]
pub(crate) mod synthetic {
    include!("../../../fixtures/synthetic/foundation.rs");
}

#[cfg(test)]
mod asset_coverage_tests {
    use super::*;

    #[test]
    fn asset_coverage_f2_q1_frozen_profile_reaches_full_source_and_compiled_wire_only_on_new_runs() {
        let source=prepare_source("第一集\n场景：摊位\n△顾客甲接过包裹，再递还卖主。\n顾客甲：先让我看看。\n△卖主放下手电，拿起铲子。\n").unwrap();
        let selected:Vec<_>=source.episodes.iter().map(|episode|episode.id.clone()).collect();
        let batch=plan_batches(&source,&selected).unwrap().remove(0);
        let foundation=empty_foundation(&source);
        let old_context=json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        let mut frozen=old_context.clone();
        super::super::asset_coverage::freeze_new(&mut frozen).unwrap();
        let frozen:Value=serde_json::from_slice(&serde_json::to_vec(&frozen).unwrap()).unwrap();
        let mut context=old_context.clone();
        super::super::asset_coverage::inherit(&mut context,&frozen).unwrap();
        for stage in ["F2","Q1"] {
            let old=input_for(stage,&source,&batch,&foundation,&old_context,0,1,true,&[]);
            let mut new=input_for(stage,&source,&batch,&foundation,&context,0,1,true,&[]);
            assert_eq!(new[super::super::asset_coverage::FIELD],super::super::asset_coverage::PROFILE);
            let schema=delivery_schema(stage,&new);
            let compiled=super::super::foundation_wire::compile(&format!("foundation:{stage}"),&new,&schema).unwrap();
            assert_eq!(compiled.input[super::super::asset_coverage::FIELD],super::super::asset_coverage::PROFILE);
            assert!(compiled.input["transportInstructions"].as_str().unwrap().contains("必要素材闭环核对"));
            assert_ne!(digest(&old.to_string()),digest(&new.to_string()),"new contract participates in request identity");
            new.as_object_mut().unwrap().remove(super::super::asset_coverage::FIELD);
            new["transportInstructions"]=old["transportInstructions"].clone();
            assert_eq!(new,old,"all source, registered identities and scope remain exactly unchanged");
            assert!(old.get(super::super::asset_coverage::FIELD).is_none());
        }
    }

    #[test]
    fn asset_coverage_unknown_foundation_profile_fails_before_consuming_pending_work() {
        let source=prepare_source("第一集\n场景：大厅\n小王：开始。\n").unwrap();
        let selected:Vec<_>=source.episodes.iter().map(|episode|episode.id.clone()).collect();
        let mut pending=plan_batches(&source,&selected).unwrap();
        let count=pending.len();
        let context=json!({super::super::asset_coverage::FIELD:"unknown"});
        assert!(prepare_capacity_batch("F2",&source,&mut pending,&empty_foundation(&source),&context,0,&[]).is_err());
        assert_eq!(pending.len(),count);
    }

    #[test]
    fn asset_coverage_restricted_editor_phases_do_not_gain_catalogue_edit_authority() {
        let context=json!({super::super::asset_coverage::FIELD:super::super::asset_coverage::PROFILE});
        for phase in ["AUTO_INTERPRET","APPLY_DECISION","REPAIR"] {
            let mut input=json!({"delivery":{"phase":phase},"transportInstructions":"original bounded authority"});
            super::super::asset_coverage::foundation_input("Q1",&mut input,&context);
            assert_eq!(input["transportInstructions"],"original bounded authority");
            assert_eq!(input[super::super::asset_coverage::FIELD],super::super::asset_coverage::PROFILE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_speaker_f1_retrieves_members_keeps_two_assets_and_roundtrips_wire() {
        let source = prepare_source("第一集\n场景：码头\n△林川右手受伤。\n△周航穿着雨衣。\n第二集\n场景：门口\n人物：林川、周航\n林川&周航：（同时回头）真的来了！\n").unwrap();
        let scope = vec![source.episodes[1].id.clone()];
        let batch = Batch { pieces:whole_rows(&source,&scope),scope,fragment:None };
        let prior = empty_foundation(&source);
        let input = input_for("F2",&source,&batch,&prior,&json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE}),0,1,false,&[]);
        assert_eq!(input["jointSpeakerProfile"],"source-dialogue-members/v1");
        assert_eq!(input["jointSpeakers"][0]["speakers"][0]["text"],"林川");
        assert_eq!(input["jointSpeakers"][0]["speakers"][1]["text"],"周航");
        let evidence = expand_history(&input["evidence"]["historicalEvidenceCandidates"]).unwrap();
        assert!(evidence.to_string().contains("林川右手受伤"));
        assert!(evidence.to_string().contains("周航穿着雨衣"));
        let wire = super::super::foundation_wire::compile("foundation:F2",&input,&delivery_schema("F2",&input)).unwrap();
        let mut restored = wire.input.clone(); wire.restore(&mut restored).unwrap();
        assert_eq!(restored["jointSpeakers"], input["jointSpeakers"]);
        let scene=&source.scenes[1];
        let mut raw=empty_catalogue(&source,&batch);
        raw["assets"]=json!(["林川","周航"].iter().map(|name|json!({"name":name,"type":"CH","episodes":batch.scope,"sceneRefs":[[scene.episode_id,scene.id]],"preparation":{"mode":"REFERENCE"},"variants":[]})).collect::<Vec<_>>());
        let raw_before=raw.to_string();
        let next=apply_catalogue(&raw,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap();
        assert_eq!(next.assets.len(),2);
        assert_ne!(next.assets[0].id,next.assets[1].id);
        assert_eq!(raw.to_string(),raw_before);
        validate_foundation(&source,&next).unwrap();
        let mut missing=raw.clone();missing["assets"].as_array_mut().unwrap().pop();
        assert!(apply_catalogue(&missing,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap_err().contains("共同说者成员"));
        let mut combined=raw.clone(); combined["assets"][0]["name"]=json!("林川&周航");
        assert!(apply_catalogue(&combined,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap_err().contains("共同说者"));
        let mut merged=next.clone(); merged.assets[0].aliases=vec!["林川".into(),"周航".into()];
        assert!(validate_foundation(&source,&merged).unwrap_err().contains("共同说者"));
    }

    use std::sync::Mutex;

    struct CopiedProjectLiveExecutor {
        inner: crate::text_executor::SavedTextExecutor,
        frozen_f2: Value,
        frozen_q1: Option<Value>,
    }
    #[async_trait]
    impl TextExecutor for CopiedProjectLiveExecutor {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            if request.stage == "foundation:Q1" {
                let mut captured = serde_json::to_value(&request).map_err(|e| e.to_string())?;
                captured["structuredOutputTransport"] = json!("strict-optionals/v1");
                captured["executorPolicy"] = json!("codex-isolated/v1");
                captured["executorIdentity"] = json!({"provider":"codex","executable":crate::text_cli::resolve_executable(&self.inner.settings)?.to_string_lossy()});
                let path = std::path::PathBuf::from(
                    std::env::var("DIRECTOR_TEST_FOUNDATION_REPORT_DIR")
                        .expect("explicit report directory required"),
                )
                .join(format!(
                    "current-q1-{}.json",
                    request.input["scope"].as_array().unwrap().len()
                ));
                std::fs::write(path, serde_json::to_vec_pretty(&captured).unwrap())
                    .map_err(|e| e.to_string())?;
            }
            if request.stage == "foundation:F2" {
                let mut current = serde_json::to_value(&request).map_err(|e| e.to_string())?;
                current["structuredOutputTransport"] = json!("strict-optionals/v1");
                current["executorPolicy"] = json!("codex-isolated/v1");
                current["executorIdentity"] = json!({"provider":"codex","executable":crate::text_cli::resolve_executable(&self.inner.settings)?.to_string_lossy()});
                if !crate::text_executor::legacy_f2_snapshot_compatible(&self.frozen_f2, &current) {
                    return Err(
                        "真实副本F2请求与冻结请求不满足精确迁移条件；测试禁止重新生成F2".into(),
                    );
                }
                self.inner.ensure_saved_replay_available(&request)?;
            }
            if request.stage == "foundation:Q1"
                && self
                    .frozen_q1
                    .as_ref()
                    .is_some_and(|f| f["input"]["scope"] == request.input["scope"])
            {
                let frozen = self.frozen_q1.as_ref().unwrap();
                let mut current = serde_json::to_value(&request).map_err(|e| e.to_string())?;
                current["structuredOutputTransport"] = json!("strict-optionals/v1");
                current["executorPolicy"] = json!("codex-isolated/v1");
                current["executorIdentity"] = json!({"provider":"codex","executable":crate::text_cli::resolve_executable(&self.inner.settings)?.to_string_lossy()});
                if &current != frozen {
                    return Err("真实副本首批Q1请求变化；测试禁止重新生成已收到的整编".into());
                }
                self.inner.ensure_saved_replay_available(&request)?;
            }
            self.inner.execute(request).await
        }
        async fn accept(&self, r: &TextRequest, v: &Value) -> Result<(), String> {
            self.inner.accept(r, v).await
        }
        async fn reject(&self, r: &TextRequest, v: &Value, e: &str) -> Result<(), String> {
            self.inner.reject(r, v, e).await
        }
    }

    #[tokio::test]
    #[ignore = "requires explicit isolated copy and DIRECTOR_TEST_REAL_FOUNDATION_WORKFLOW_ID/F2_JOB_ID/Q1_JOB_ID; bounded live Q1 continuation only on the isolated copy"]
    async fn real_script_copy_reuses_original_f2_and_completes_live_q1_handoff() {
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| panic!("required explicit environment variable: {name}"))
        };
        let root = std::path::PathBuf::from(
            std::env::var("DIRECTOR_TEST_REAL_FOUNDATION_COPY")
                .expect("explicit isolated copy required"),
        );
        let resolved = std::fs::canonicalize(&root).unwrap();
        assert!(
            resolved.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
                && root.file_name().and_then(|s| s.to_str()) == Some("isolated-project")
        );
        let store = crate::store::Store::open(root.clone()).unwrap();
        let run_value = required("DIRECTOR_TEST_REAL_FOUNDATION_WORKFLOW_ID");
        let run = run_value.as_str();
        let rejected_value = required("DIRECTOR_TEST_REAL_FOUNDATION_F2_JOB_ID");
        let rejected = rejected_value.as_str();
        let parent = store.job(run).unwrap();
        let original = store.job(rejected).unwrap();
        let input = &parent["snapshot"]["input"];
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let context = json!({"foundation":input["foundation"],"assets":input["assets"],"selection":input["selection"],"settings":{"confirmedDirection":input["foundation"]["productionDirection"]}});
        let q1_value = required("DIRECTOR_TEST_REAL_FOUNDATION_Q1_JOB_ID");
        let saved_q1 = store.job(&q1_value).ok();
        let executor = CopiedProjectLiveExecutor {
            inner: crate::text_executor::SavedTextExecutor {
                store: store.clone(),
                settings: input["settings"].clone(),
                workflow_id: Some(run.into()),
            },
            frozen_f2: original["snapshot"].clone(),
            frozen_q1: saved_q1.as_ref().map(|j| j["snapshot"].clone()),
        };
        let counts = || -> usize {
            store
                .jobs()
                .unwrap()
                .iter()
                .filter(|j| j["kind"] == "text")
                .count()
        };
        let before = counts();
        let before_f2 = store
            .jobs()
            .unwrap()
            .iter()
            .filter(|j| j["kind"] == "text" && j["snapshot"]["stage"] == "foundation:F2")
            .count();
        let started = std::time::Instant::now();
        let mut report = json!({"ok":false,"fixtureProject":root,"productionDatabaseWrites":false,"sourceSha256":source.sha256,"episodes":source.episodes.len(),"oldF2RawPreserved":false,"newF2Requests":0,"imageVideoRequests":0});
        let outcome:Result<(),String>=async {
            let mut proposal=run_foundation_delivery(&source,&context,&executor,"gpt-6.1-sol","medium").await?;
            report["firstRunMilliseconds"]=json!(started.elapsed().as_millis());
            report["newTextRequests"]=json!(counts()-before);
            let jobs=store.jobs()?;
            assert_eq!(jobs.iter().filter(|j|j["kind"]=="text"&&j["snapshot"]["stage"]=="foundation:F2").count(),before_f2,"replay must not generate F2");
            assert!(counts()-before<=1,"only the remaining Q1 capacity batch may call the model");
            let replay=run_foundation_delivery(&source,&context,&executor,"gpt-6.1-sol","medium").await?;
            assert_eq!(counts(),jobs.iter().filter(|j|j["kind"]=="text").count(),"same-source replay must not add requests");
            assert_eq!(serde_json::to_value(&proposal).unwrap(),serde_json::to_value(replay).unwrap());
            let original_after=store.job(rejected)?;
            assert_eq!(original_after["snapshot"],original["snapshot"]);assert_eq!(original_after["result"],original["result"]);
            if let Some(saved)=&saved_q1 {let after=store.job(saved["id"].as_str().unwrap())?;assert_eq!(after["snapshot"],saved["snapshot"]);assert_eq!(after["result"],saved["result"]);report["oldQ1RawPreserved"]=json!(true);}
            report["totalQ1Calls"]=json!(store.jobs()?.iter().filter(|j|j["kind"]=="text"&&j["snapshot"]["stage"]=="foundation:Q1").count());
            report["oldF2RawPreserved"]=json!(true);report["cacheReplayNewRequests"]=json!(0);
            let revised=super::super::execute_foundation_scene_edits(&source,&mut proposal)?.map(serde_json::to_value).transpose().map_err(|e|e.to_string())?;
            report["assetIdentities"]=json!(proposal.assets.len());
            let assets=flatten_foundation_assets(&proposal);report["stage2AssetRows"]=json!(assets.len());
            report["issues"]=json!(proposal.issues.iter().map(|i|json!({"coordinate":i.coordinate,"blocking":i.blocking,"episodes":i.episode_ids})).collect::<Vec<_>>());
            let guard=parent["prepared"].get("publicationGuard").unwrap_or(&parent["snapshot"]["guard"]);
            store.publish_foundation_with_source(guard,serde_json::to_value(&proposal).map_err(|e|e.to_string())?,assets,Some(run),revised)?;
            assert_eq!(store.source()?["text"],source.text);assert_eq!(store.source()?["sha256"],source.sha256);
            report["sourcePreserved"]=json!(true);report["publishedFoundationVersion"]=store.foundation()?["version"].clone();
            Ok(())
        }.await;
        report["ok"] = json!(outcome.is_ok());
        report["totalMilliseconds"] = json!(started.elapsed().as_millis());
        if let Err(e) = &outcome {
            report["error"] = json!(e);
        }
        let output = std::path::PathBuf::from(
            std::env::var("DIRECTOR_TEST_FOUNDATION_REPORT_DIR")
                .expect("explicit report directory required"),
        )
        .join("live-copy.json");
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        outcome.unwrap();
    }

    #[tokio::test]
    #[ignore = "explicit fresh Temp project: two real Codex text requests for the saved 10-episode script, no production writes or media"]
    async fn compact_ten_episode_live_timing_and_stage2_handoff() {
        let root = std::path::PathBuf::from(
            std::env::var("DIRECTOR_SPEED_PROJECT")
                .expect("explicit new isolated project required"),
        );
        let report_path = std::path::PathBuf::from(
            std::env::var("DIRECTOR_SPEED_REPORT").expect("explicit report required"),
        );
        let resolved = std::fs::canonicalize(&root).unwrap();
        assert!(
            resolved.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap())
                && root.file_name().and_then(|s| s.to_str()) == Some("isolated-project")
        );
        assert!(
            !root.join("project.sqlite").exists(),
            "fresh test only; retained results must not silently regenerate"
        );
        let source: LockedSource = serde_json::from_slice(
            &std::fs::read(
                std::env::var("DIRECTOR_TEST_FOUNDATION_SOURCE")
                    .expect("explicit private source fixture path required"),
            )
            .unwrap(),
        )
        .unwrap();
        let store = crate::store::Store::open(root.clone()).unwrap();
        store
            .create("10集轻输入真实验收", &uuid::Uuid::new_v4().to_string())
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        store.save_source(json!(source)).unwrap();
        let settings = crate::text_cli::resolve_settings(
            &json!({"textProvider":"codex","textModel":"gpt-6.1-sol","reasoning":"medium"}),
        )
        .await
        .unwrap();
        let context = json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        let run=store.add_job("workflow","foundation.run","local","",&json!({"foundationMethod":METHOD_VERSION,"sourceSha256":source.sha256,"foundationModelInputProfile":super::super::foundation_wire::PROFILE}),"running").unwrap();
        let executor = crate::text_executor::SavedTextExecutor {
            store: store.clone(),
            settings,
            workflow_id: Some(run.clone()),
        };
        let started = std::time::Instant::now();
        let mut report = json!({"ok":false,"profile":super::super::foundation_wire::PROFILE,"model":"gpt-6.1-sol","reasoning":"medium","fixtureProject":root,"episodes":source.episodes.len(),"sourceSha256":source.sha256,"productionDatabaseWrites":false,"mediaRequests":0});
        let result: Result<(), String> = async {
            let mut proposal =
                run_foundation_delivery(&source, &context, &executor, "gpt-6.1-sol", "medium")
                    .await?;
            report["generationMilliseconds"] = json!(started.elapsed().as_millis());
            let jobs = store.jobs()?;
            assert_eq!(
                jobs.iter().filter(|j| j["kind"] == "text").count(),
                2,
                "actual 10-episode data must fit one F2 and one Q1"
            );
            let replay =
                run_foundation_delivery(&source, &context, &executor, "gpt-6.1-sol", "medium")
                    .await?;
            assert_eq!(store.jobs()?.len(), jobs.len());
            assert_eq!(json!(replay), json!(proposal));
            report["cacheAdditionalRequests"] = json!(0);
            let revised = super::super::execute_foundation_scene_edits(&source, &mut proposal)?
                .unwrap_or(source.clone());
            assert_eq!(revised.text, source.text);
            assert_eq!(revised.sha256, source.sha256);
            validate_source(&revised)?;
            validate_foundation(&revised, &proposal)?;
            store.save_source(json!(revised))?;
            store.save_foundation(json!(proposal))?;
            let assets = flatten_foundation_assets(&proposal);
            assert!(!assets.is_empty());
            for asset in &assets {
                store.save_asset(asset.clone())?;
            }
            assert_eq!(store.values("assets")?.len(), assets.len());
            report["assetIdentities"] = json!(proposal.assets.len());
            report["stage2AssetRows"] = json!(assets.len());
            report["sourcePreserved"] = json!(true);
            report["blockingIssues"] = json!(proposal.issues.iter().filter(|i| i.blocking).count());
            report["notes"] = json!(proposal
                .issues
                .iter()
                .map(|i| json!({"coordinate":i.coordinate,"blocking":i.blocking}))
                .collect::<Vec<_>>());
            Ok(())
        }
        .await;
        report["ok"] = json!(result.is_ok());
        report["totalMilliseconds"] = json!(started.elapsed().as_millis());
        if let Err(error) = &result {
            report["error"] = json!(error);
        }
        report["jobs"]=json!(store.jobs().unwrap().iter().filter(|j|j["kind"]=="text").map(|j|{
            let wire=std::fs::read(root.join("text-runs").join(j["id"].as_str().unwrap()).join("wire-request.json")).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok());
            let raw=std::fs::read_to_string(root.join("text-runs").join(j["id"].as_str().unwrap()).join("result.json")).ok();
            json!({"id":j["id"],"stage":j["snapshot"]["stage"],"status":j["status"],"createdAt":j["createdAt"],"updatedAt":j["updatedAt"],"receipt":j["receipt"],"error":j["error"],"canonicalInputChars":units(&j["snapshot"]["input"].to_string()),"wireInputChars":wire.as_ref().map(|v|units(&v["prompt"]["input"].to_string())),"rawOutputChars":raw.as_ref().map(|v|units(v))})
        }).collect::<Vec<_>>());
        crate::store::atomic_json(&report_path, &report).unwrap();
        println!(
            "{}",
            json!({"ok":report["ok"],"report":report_path,"milliseconds":report["generationMilliseconds"],"error":report["error"]})
        );
        result.unwrap();
    }
    #[tokio::test]
    #[ignore = "explicit retained speed fixture: read saved F2/Q1 only, zero model calls or production writes"]
    async fn final_compact_contract_replays_real_speed_results_without_generation() {
        struct Replay {
            jobs: Vec<Value>,
            root: std::path::PathBuf,
            calls: Mutex<Vec<Value>>,
        }
        #[async_trait]
        impl TextExecutor for Replay {
            async fn execute(&self, r: TextRequest) -> Result<Value, String> {
                let saved = self
                    .jobs
                    .iter()
                    .find(|j| j["snapshot"]["stage"] == r.stage)
                    .ok_or("replay has no exact stage")?;
                assert_eq!(saved["snapshot"]["sourceSha256"], Value::Null);
                assert_eq!(saved["snapshot"]["schema"], r.schema);
                assert_eq!(saved["snapshot"]["ruleSnapshot"], r.rule_snapshot);
                let old_wire: Value = serde_json::from_slice(
                    &std::fs::read(
                        self.root
                            .join("text-runs")
                            .join(saved["id"].as_str().unwrap())
                            .join("wire-request.json"),
                    )
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let compiled =
                    super::super::foundation_wire::compile(&r.stage, &r.input, &r.schema)?;
                assert_eq!(
                    compiled.input, old_wire["prompt"]["input"],
                    "current final transport input differs from the actually tested model input"
                );
                assert!(
                    request_chars(r.stage.rsplit(':').next().unwrap(), &r.input, &r.schema)?
                        <= MODEL_TRANSPORT_LIMIT
                );
                let mut result = saved["result"].clone();
                compiled.restore(&mut result)?;
                self.calls.lock().unwrap().push(
                    json!({"stage":r.stage,"scope":r.input["scope"],"actualTestedWireReused":true}),
                );
                Ok(result)
            }
        }
        let root = std::path::PathBuf::from(
            std::env::var("DIRECTOR_SPEED_PROJECT").expect("explicit retained fixture"),
        );
        let canonical = std::fs::canonicalize(&root).unwrap();
        assert!(canonical.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
        let store = crate::store::Store::open(root.clone()).unwrap();
        // Store::jobs is a UI projection and adds result.mediaIds; the saved
        // executor reads the original SQL value, which is the contract evidence.
        let connection = store.conn().unwrap();
        let mut statement = connection
            .prepare("SELECT id,snapshot,result FROM jobs WHERE kind='text' ORDER BY created_at")
            .unwrap();
        let jobs=statement.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).unwrap().map(|row|{let(id,snapshot,result)=row.unwrap();json!({"id":id,"snapshot":serde_json::from_str::<Value>(&snapshot).unwrap(),"result":serde_json::from_str::<Value>(&result).unwrap()})}).collect::<Vec<_>>();
        assert_eq!(jobs.len(), 2);
        let source: LockedSource = serde_json::from_slice(
            &std::fs::read(
                std::env::var("DIRECTOR_TEST_FOUNDATION_SOURCE")
                    .expect("explicit private source fixture path required"),
            )
            .unwrap(),
        )
        .unwrap();
        let executor = Replay {
            jobs,
            root,
            calls: Mutex::new(vec![]),
        };
        let context = json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        let mut proposal =
            run_foundation_delivery(&source, &context, &executor, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
        let revised = super::super::execute_foundation_scene_edits(&source, &mut proposal)
            .unwrap()
            .unwrap_or(source.clone());
        validate_source(&revised).unwrap();
        validate_foundation(&revised, &proposal).unwrap();
        assert_eq!(source.text, revised.text);
        assert_eq!(source.sha256, revised.sha256);
        assert!(proposal.issues.iter().all(|i| !regex::Regex::new(r"^r\d+$")
            .unwrap()
            .is_match(&i.coordinate)));
        let calls = executor.calls.into_inner().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(flatten_foundation_assets(&proposal).len(), 51);
        let report = json!({"ok":true,"realModelCalls":0,"finalWireMatchesTestedInput":true,"sourcePreserved":true,"assetRows":51,"sourceIssueCoordinatesCanonical":true,"calls":calls});
        let path = std::path::PathBuf::from(
            std::env::var("DIRECTOR_TEST_FOUNDATION_REPORT_DIR")
                .expect("explicit report directory required"),
        )
        .join("final-replay.json");
        crate::store::atomic_json(&path, &report).unwrap();
    }
    fn source(count: usize) -> LockedSource {
        prepare_source(
            &(1..=count)
                .map(|i| format!("第{i}集\n场景：大厅\n小王：开始。\n△小王穿白衣进入。\n"))
                .collect::<String>(),
        )
        .unwrap()
    }
    fn empty_catalogue(source: &LockedSource, batch: &Batch) -> Value {
        json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":source.sha256,"coverage":batch.scope,"assets":[]})
    }
    fn empty_edit(source: &LockedSource, batch: &Batch) -> Value {
        json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":source.sha256,"coverage":batch.scope,"changes":{}})
    }

    fn episode_usage_projection_fixture() -> (LockedSource, Batch, FoundationProposal, Value, Value) {
        let source = prepare_source(concat!(
            "第一集\n场景：室内\n△甲穿靴。\n",
            "第二集\n场景：门外\n△甲继续穿靴。\n",
            "第三集\n场景：码头\n△甲仍穿靴。\n",
        )).unwrap();
        let scope: Vec<_> = source.episodes.iter().map(|episode| episode.id.clone()).collect();
        let batch = Batch { pieces: whole_rows(&source, &scope), scope, fragment: None };
        let prior = empty_foundation(&source);
        let pairs: Vec<_> = source.scenes.iter().map(|scene| json!([scene.episode_id,scene.id])).collect();
        let mut raw = empty_catalogue(&source, &batch);
        raw["assets"] = json!([{
            "name":"甲｜常服","type":"CH","episodes":[batch.scope[0],batch.scope[0]],
            "sceneRefs":[pairs[0],pairs[2]],"preparation":{"mode":"REFERENCE"},
            "variants":[{"name":"穿靴","episodes":[batch.scope[0],batch.scope[1]],
                "sceneRefs":[pairs[1],pairs[2]],"preparation":{"mode":"DERIVE"}}]
        }]);
        let input = input_for("F2", &source, &batch, &prior, &json!({}), 0, 1, false, &[]);
        (source,batch,prior,input,raw)
    }

    #[test]
    fn scene_usage_episodes_project_base_and_variant_without_changing_raw() {
        let (source,batch,prior,input,raw) = episode_usage_projection_fixture();
        let frozen = raw.to_string();
        let next = apply_catalogue(&raw,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap();
        let asset = &next.assets[0];
        let base = &asset.variants[0];
        let child = &asset.variants[1];
        assert_eq!(asset.metadata["usageEpisodes"],json!([batch.scope[0],batch.scope[2]]));
        assert_eq!(base.metadata["usageEpisodes"],asset.metadata["usageEpisodes"]);
        assert_eq!(child.metadata["usageEpisodes"],json!([batch.scope[1],batch.scope[2]]));
        assert_eq!(base.scope.scenes.len(),2);
        assert_eq!(child.scope.scenes.len(),2);
        assert_eq!(child.source_variant_id.as_deref(),Some(base.id.as_str()));
        assert!(next.facts.is_empty(),"usage projection never invents appearance facts");
        assert_eq!(raw.to_string(),frozen,"original model reply stays byte-identical");
        let projected = super::super::flatten_foundation_assets(&next);
        let child_row = projected.iter().find(|row|row["variantId"]==child.id).unwrap();
        assert_eq!(child_row["applicability"]["episodes"],json!([batch.scope[1],batch.scope[2]]));
        assert_eq!(child_row["sourceVariantId"],base.id);
    }

    #[test]
    fn scene_usage_episode_projection_cannot_bypass_source_batch_or_reference_guards() {
        let (source,batch,prior,input,raw) = episode_usage_projection_fixture();
        for failure in ["unknown_scene","wrong_episode","duplicate_scene","unprovided_scene","invalid_source_ref"] {
            let mut damaged = raw.clone();
            let mut current_batch = batch.clone();
            match failure {
                "unknown_scene" => damaged["assets"][0]["sceneRefs"][0][1] = json!("scene:unknown"),
                "wrong_episode" => damaged["assets"][0]["sceneRefs"][0][0] = json!(batch.scope[1]),
                "duplicate_scene" => damaged["assets"][0]["sceneRefs"][1] = damaged["assets"][0]["sceneRefs"][0].clone(),
                "unprovided_scene" => current_batch.pieces.retain(|piece| !source.rows.iter().any(|row|row.id==piece.row_id && row.scene_id.as_deref()==Some(source.scenes[2].id.as_str()))),
                "invalid_source_ref" => damaged["assets"][0]["sourceRefs"] = json!(["row:unprovided"]),
                _ => unreachable!(),
            }
            assert!(apply_catalogue(&damaged,&source,&current_batch,&input,&prior,None,&HashSet::new()).is_err(),"{failure}");
        }
        let mut narrow_batch = batch.clone();
        narrow_batch.scope = batch.scope[..2].to_vec();
        narrow_batch.pieces = whole_rows(&source,&narrow_batch.scope);
        let mut outside = raw;
        outside["coverage"] = json!(narrow_batch.scope);
        let narrow_input = input_for("F2",&source,&narrow_batch,&prior,&json!({}),0,1,false,&[]);
        assert!(apply_catalogue(&outside,&source,&narrow_batch,&narrow_input,&prior,None,&HashSet::new()).is_err());
    }

    #[test]
    fn episode_only_reference_catalogue_preserves_usage_without_inventing_scenes() {
        let (source,batch,prior,input,mut raw) = episode_usage_projection_fixture();
        raw["assets"][0]["variants"] = json!([]);
        raw["assets"][0]["episodes"] = json!([batch.scope[0],batch.scope[2]]);
        raw["assets"][0]["sceneRefs"] = json!([]);
        let frozen = raw.to_string();
        let next = apply_catalogue(&raw,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap();
        let asset = &next.assets[0];
        assert_eq!(asset.metadata["usageEpisodes"],raw["assets"][0]["episodes"]);
        assert_eq!(asset.variants[0].scope.kind,"UNRESOLVED");
        assert!(asset.variants[0].applicability.is_empty());
        assert!(asset.variants[0].scope.scenes.is_empty());
        assert_eq!(raw.to_string(),frozen);
        raw["assets"][0]["episodes"] = json!(["episode:unprovided"]);
        assert!(apply_catalogue(&raw,&source,&batch,&input,&prior,None,&HashSet::new()).is_err());
    }

    #[test]
    fn scope_reasons_raw_matches_only_original_frozen_ref_and_quote_normalization() {
        let source = source(1);
        let selected = source
            .episodes
            .iter()
            .map(|episode| episode.id.clone())
            .collect::<Vec<_>>();
        let batch = plan_batches(&source, &selected).unwrap().remove(0);
        let input = input_for(
            "Q1",
            &source,
            &batch,
            &empty_foundation(&source),
            &json!({}),
            0,
            1,
            true,
            &[],
        );
        let row = source.rows.iter().find(|row| row.kind == "body").unwrap();
        let prefix = format!("row:{}:", &source.sha256[..12]);
        let short = format!("r:{}", row.id.strip_prefix(&prefix).unwrap());
        let raw = json!({"sourceRefs":[short],"seriesDirection":{"signals":[{
            "rowId":short,"episodeId":row.episode_id,"reading":"只读取冻结原行"
        }]}});
        let mut accepted = raw.clone();
        accepted["sourceRefs"] = json!([row.id]);
        accepted["seriesDirection"]["signals"][0]["rowId"] = json!(row.id);
        accepted["seriesDirection"]["signals"][0]["quote"] =
            json!(row.text.trim_end_matches(['\r', '\n']));
        assert!(accepted_scope_reason_raw_matches(&input, &raw, &accepted));
        let frozen = raw.clone();
        let mut extra = raw.clone();
        extra["transportOnly"] = json!("不能删除额外内容以伪称一致");
        assert!(!accepted_scope_reason_raw_matches(
            &input, &extra, &accepted
        ));
        let mut outside = raw.clone();
        outside["sourceRefs"] = json!(["r:999999:1000000"]);
        let mut outside_accepted = accepted.clone();
        outside_accepted["sourceRefs"] = json!([format!("{prefix}999999:1000000")]);
        assert!(!accepted_scope_reason_raw_matches(
            &input,
            &outside,
            &outside_accepted
        ));
        let mut changed_quote = accepted.clone();
        changed_quote["seriesDirection"]["signals"][0]["quote"] = json!("篡改锁定原句");
        assert!(!accepted_scope_reason_raw_matches(
            &input,
            &raw,
            &changed_quote
        ));
        assert_eq!(raw, frozen);
    }

    fn scope_reasons_fixture() -> (LockedSource, Value, Batch, FoundationProposal, Value, Value) {
        let source = prepare_source(
            "第1集\n场景：客厅\n刘富贵（电话声）：先打个电话。\n△苏阳接听电话。\n第2集\n场景：木船\n△木船返航，水泵和相连水管随船运载。\n",
        ).unwrap();
        let scope = source
            .episodes
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let context = json!({"selection":{"episodeIds":scope}});
        let (_, basis, _) = delivery_basis(&source, &context).unwrap();
        let batch = plan_batches(&source, &scope).unwrap().remove(0);
        let input = input_for("F2", &source, &batch, &basis, &context, 0, 1, false, &[]);
        let phone = &source.scenes[0];
        let returning = &source.scenes[1];
        let mut raw = empty_catalogue(&source, &batch);
        raw["assets"] = json!([
            {"name":"刘富贵｜默认","type":"CH","description":"剧本登记的刘富贵。",
             "episodes":[phone.episode_id],"sceneRefs":[[phone.episode_id,phone.id]],
             "scope":{"kind":"SCENES","reason":"电话声音，不计实体入镜。"},
             "preparation":{"mode":"REFERENCE"}},
            {"name":"水泵与相连水管","type":"PROP","description":"水泵及相连水管。",
             "episodes":[returning.episode_id],"sceneRefs":[[returning.episode_id,returning.id]],
             "scope":{"kind":"SCENES","reason":"返航运载配套。\n不新增抽水行为。"},
             "preparation":{"mode":"REFERENCE"},
             "variants":[{"name":"返航运输状态","episodes":[returning.episode_id],
                "sceneRefs":[[returning.episode_id,returning.id]],
                "scope":{"kind":"SCENES","reason":"返航运输状态。\n不新增抽水行为。"},
                "note":"保持相连器具的运输状态。","preparation":{"mode":"DERIVE"}}]}
        ]);
        (source, context, batch, basis, input, raw)
    }

    #[test]
    fn scope_reasons_f2_keeps_voice_only_and_returning_pump_without_visual_facts() {
        let (source, _, batch, basis, input, raw) = scope_reasons_fixture();
        let frozen = raw.clone();
        let next =
            apply_catalogue(&raw, &source, &batch, &input, &basis, None, &HashSet::new()).unwrap();
        assert_eq!(
            next.assets[0].variants[0].scope.reason.as_deref(),
            Some("电话声音，不计实体入镜。")
        );
        assert_eq!(
            next.assets[1].variants[0].scope.reason.as_deref(),
            Some("返航运载配套。\n不新增抽水行为。")
        );
        assert_eq!(
            next.assets[1].variants[1].scope.reason.as_deref(),
            Some("返航运输状态。\n不新增抽水行为。")
        );
        assert_eq!(
            next.assets[1].variants[1].source_variant_id.as_deref(),
            Some(next.assets[1].variants[0].id.as_str())
        );
        assert_eq!(next.assets[1].description, "水泵及相连水管。");
        assert!(next.facts.is_empty());
        let flat = flatten_foundation_assets(&next);
        assert_eq!(
            flat[0]["scope"]["reason"],
            raw["assets"][0]["scope"]["reason"]
        );
        assert_eq!(raw, frozen, "the accepted original reply remains literal");
    }

    #[test]
    fn scope_reasons_f2_existing_base_and_child_merge_in_order_without_duplicates() {
        let (source, context, batch, basis, input, raw) = scope_reasons_fixture();
        let prior =
            apply_catalogue(&raw, &source, &batch, &input, &basis, None, &HashSet::new()).unwrap();
        let input = input_for("F2", &source, &batch, &prior, &context, 0, 1, false, &[]);
        let mut update = raw.clone();
        update["assets"].as_array_mut().unwrap().remove(0);
        update["assets"][0]["key"] = json!(prior.assets[1].id);
        update["assets"][0]["scope"]["reason"] =
            json!("不新增抽水行为。\n仅甲板运载。\n仅甲板运载。");
        update["assets"][0]["variants"][0]["key"] = json!(prior.assets[1].variants[1].id);
        update["assets"][0]["variants"][0]["scope"]["reason"] =
            json!("不新增抽水行为。\n仅甲板运载。");
        let next = apply_catalogue(
            &update,
            &source,
            &batch,
            &input,
            &prior,
            None,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            next.assets[1].variants[0].scope.reason.as_deref(),
            Some("返航运载配套。\n不新增抽水行为。\n仅甲板运载。")
        );
        assert_eq!(
            next.assets[1].variants[1].scope.reason.as_deref(),
            Some("返航运输状态。\n不新增抽水行为。\n仅甲板运载。")
        );
        let repeated = apply_catalogue(
            &update,
            &source,
            &batch,
            &input,
            &next,
            None,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(json!(next.assets), json!(repeated.assets));
        assert_eq!(
            next.assets[0].variants[0].scope.reason,
            prior.assets[0].variants[0].scope.reason
        );
    }

    #[test]
    fn scope_reasons_q1_retains_prior_and_new_base_and_child_explanations() {
        let (source, context, batch, basis, input, raw) = scope_reasons_fixture();
        let prior =
            apply_catalogue(&raw, &source, &batch, &input, &basis, None, &HashSet::new()).unwrap();
        let input = input_for("Q1", &source, &batch, &prior, &context, 0, 1, true, &[]);
        let returning = &source.scenes[1];
        let mut edit = empty_edit(&source, &batch);
        edit["changes"]["assets"] = json!([{"key":prior.assets[1].id,
            "episodes":[returning.episode_id],"sceneRefs":[[returning.episode_id,returning.id]],
            "scope":{"kind":"SCENES","reason":"不新增抽水行为。\n运输连续性保留。"},
            "variants":[{"key":prior.assets[1].variants[1].id,"episodes":[returning.episode_id],
                "sceneRefs":[[returning.episode_id,returning.id]],
                "scope":{"kind":"SCENES","reason":"不新增抽水行为。\n运输连续性保留。"}}]}]);
        let (next, _) = apply_optimization(&edit, &source, &batch, &input, &prior, None).unwrap();
        assert_eq!(
            next.assets[1].variants[0].scope.reason.as_deref(),
            Some("返航运载配套。\n不新增抽水行为。\n运输连续性保留。")
        );
        assert_eq!(
            next.assets[1].variants[1].scope.reason.as_deref(),
            Some("返航运输状态。\n不新增抽水行为。\n运输连续性保留。")
        );
        assert!(next.facts.is_empty());
        assert_eq!(next.assets[1].description, prior.assets[1].description);
    }

    pub(super) fn scope_reasons_saved_chain() -> (
        LockedSource,
        Value,
        FoundationProposal,
        Value,
        Value,
        Value,
        Value,
    ) {
        let (source, context, batch, basis, f2_input, f2_raw) = scope_reasons_fixture();
        let accepted = apply_catalogue(
            &f2_raw,
            &source,
            &batch,
            &f2_input,
            &basis,
            None,
            &HashSet::new(),
        )
        .unwrap();
        let mut old_projection = accepted.clone();
        for variant in old_projection
            .assets
            .iter_mut()
            .flat_map(|asset| &mut asset.variants)
        {
            variant.scope.reason = None;
        }
        // Freeze Q1 with the old lossy candidate, exactly as a saved workflow
        // would have done. Do not regenerate its input from enriched metadata.
        let q1_input = input_for(
            "Q1",
            &source,
            &batch,
            &old_projection,
            &context,
            0,
            1,
            true,
            &[],
        );
        let q1_raw = empty_edit(&source, &batch);
        let (current, _) =
            apply_optimization(&q1_raw, &source, &batch, &q1_input, &old_projection, None).unwrap();
        (source, context, current, f2_input, f2_raw, q1_input, q1_raw)
    }

    #[test]
    fn scope_reasons_projection_replays_frozen_chain_and_only_fills_missing_reasons() {
        let (source, context, mut current, f2_input, f2_raw, q1_input, q1_raw) =
            scope_reasons_saved_chain();
        current.assets[0].variants[0].scope.reason = Some("已由用户确认的电话范围解释。".into());
        current.direction = "保留现有制作方向".into();
        let original = json!(current);
        let frozen = json!([f2_input, f2_raw, q1_input, q1_raw]);
        let steps = [
            AcceptedFoundationReasonStep {
                stage: "foundation:F2",
                input: &f2_input,
                result: &f2_raw,
            },
            AcceptedFoundationReasonStep {
                stage: "foundation:Q1",
                input: &q1_input,
                result: &q1_raw,
            },
        ];
        let projected = project_missing_scope_reasons(&source, &context, &current, &steps).unwrap();
        assert_eq!(
            projected.restored_variant_ids,
            current.assets[1]
                .variants
                .iter()
                .map(|v| v.id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            projected.foundation.assets[0].variants[0].scope.reason,
            current.assets[0].variants[0].scope.reason
        );
        assert_eq!(
            projected.foundation.assets[1].variants[0]
                .scope
                .reason
                .as_deref(),
            Some("返航运载配套。\n不新增抽水行为。")
        );
        assert_eq!(
            projected.foundation.assets[1].variants[1]
                .scope
                .reason
                .as_deref(),
            Some("返航运输状态。\n不新增抽水行为。")
        );
        assert_eq!(projected.foundation.direction, current.direction);
        assert_eq!(json!(projected.projected_source), json!(source));
        assert_eq!(json!(current), original);
        assert_eq!(json!([f2_input, f2_raw, q1_input, q1_raw]), frozen);
        assert_eq!(
            catalogue_without_scope_reasons(&projected.foundation),
            catalogue_without_scope_reasons(&current)
        );
    }

    #[test]
    fn scope_reasons_projection_rejects_manual_catalogue_or_broadened_scope() {
        let (source, context, current, f2_input, f2_raw, q1_input, q1_raw) =
            scope_reasons_saved_chain();
        let steps = [
            AcceptedFoundationReasonStep {
                stage: "foundation:F2",
                input: &f2_input,
                result: &f2_raw,
            },
            AcceptedFoundationReasonStep {
                stage: "foundation:Q1",
                input: &q1_input,
                result: &q1_raw,
            },
        ];
        for mutation in [
            "name",
            "description",
            "parent",
            "metadata",
            "scope",
            "missing",
        ] {
            let mut changed = current.clone();
            match mutation {
                "name" => changed.assets[0].name.push_str("人工修订"),
                "description" => changed.assets[0].description.push_str("人工修订"),
                "parent" => changed.assets[1].variants[1].source_variant_id = None,
                "metadata" => {
                    changed.assets[0]
                        .metadata
                        .insert("manualRevision".into(), json!(true));
                }
                "scope" => {
                    let additional = Applicability {
                        episode_id: source.scenes[1].episode_id.clone(),
                        scene_id: source.scenes[1].id.clone(),
                    };
                    changed.assets[0].variants[0]
                        .scope
                        .scenes
                        .push(additional.clone());
                    changed.assets[0].variants[0].applicability.push(additional);
                }
                "missing" => {
                    changed.assets.remove(0);
                }
                _ => unreachable!(),
            }
            let frozen = json!(changed);
            let error = project_missing_scope_reasons(&source, &context, &changed, &steps)
                .err()
                .unwrap();
            assert!(error.contains("不再精确一致"), "{mutation}: {error}");
            assert_eq!(json!(changed), frozen);
        }
    }

    #[test]
    fn scope_reasons_projection_rejects_missing_chain_and_changed_frozen_source() {
        let (source, context, current, f2_input, f2_raw, q1_input, q1_raw) =
            scope_reasons_saved_chain();
        let f2_only = [AcceptedFoundationReasonStep {
            stage: "foundation:F2",
            input: &f2_input,
            result: &f2_raw,
        }];
        assert!(
            project_missing_scope_reasons(&source, &context, &current, &f2_only)
                .err()
                .unwrap()
                .contains("完整已接受")
        );
        for mutation in ["text", "scene", "hash", "index", "phase", "interval"] {
            let mut altered = f2_input.clone();
            match mutation {
                "text" => altered["source"]["rows"][0][9] = json!("篡改原文"),
                "scene" => altered["scenes"][0][1] = json!("伪造场号"),
                "hash" => altered["batch"] = json!("unknown"),
                "index" => altered["batchIndex"] = json!(1),
                "phase" => altered["delivery"]["phase"] = json!("AUTO_INTERPRET"),
                "interval" => altered["source"]["rows"][0][7] = json!(source.rows[0].range.end),
                _ => unreachable!(),
            }
            let steps = [
                AcceptedFoundationReasonStep {
                    stage: "foundation:F2",
                    input: &altered,
                    result: &f2_raw,
                },
                AcceptedFoundationReasonStep {
                    stage: "foundation:Q1",
                    input: &q1_input,
                    result: &q1_raw,
                },
            ];
            assert!(
                project_missing_scope_reasons(&source, &context, &current, &steps).is_err(),
                "{mutation}"
            );
        }
    }

    #[test]
    fn scope_reasons_projection_preserves_original_auto_receipt_time_and_content() {
        let (source, mut context, batch, basis, f2_input, mut f2_raw) = scope_reasons_fixture();
        context["projectId"] = json!("synthetic-scope-reasons");
        context["sourceVersion"] = json!(1);
        context["foundationDecisionPolicy"] = json!(AUTO_INTERPRET_POLICY);
        context["userReviewRequirements"] = json!([]);
        let refs = json!([source.rows[0].id, source.rows[1].id]);
        f2_raw["humanChoices"] = json!([{
            "decisionKey":"scope-reason-test","kind":"PRODUCTION_CHOICE",
            "question":"电话声采用哪种制作解释？","reason":"保留电话场范围说明",
            "options":["仅作为声音","制作解释分离"],"recommended":"仅作为声音",
            "recommendationReason":"不追加原文未写的实体入镜","evidence":"已提供锁定原行",
            "episodes":[source.episodes[0].id],"sourceRefs":refs,
            "changeTargets":{"assetKeys":[],"assetFields":["description"],"sceneFields":[]}
        }]);
        let mut current = apply_catalogue(
            &f2_raw,
            &source,
            &batch,
            &f2_input,
            &basis,
            None,
            &HashSet::new(),
        )
        .unwrap();
        for variant in current
            .assets
            .iter_mut()
            .flat_map(|asset| &mut asset.variants)
        {
            variant.scope.reason = None;
        }
        let q1_input = input_for("Q1", &source, &batch, &current, &context, 0, 1, true, &[]);
        let decision = &q1_input["automaticDecisions"][0];
        let mut q1_raw = empty_edit(&source, &batch);
        q1_raw["appliedDecisions"] = json!([{
            "id":decision["id"],"answer":"仅作为声音","note":"解释单独留存，原文及目录不改写",
            "subjectAssetKeys":[current.assets[0].id]
        }]);
        let original_time = "2026-10-08T16:05:25.000Z";
        let times = HashMap::from([(
            decision["id"].as_str().unwrap().to_owned(),
            original_time.to_owned(),
        )]);
        let q1_basis = current.clone();
        (current, _) = apply_optimization_with_receipt_times(
            &q1_raw,
            &source,
            &batch,
            &q1_input,
            &current,
            None,
            Some(&times),
        )
        .unwrap();
        let accepted_receipts = prior_auto_records(&current);
        assert_eq!(accepted_receipts[0]["appliedAt"], original_time);
        let steps = [
            AcceptedFoundationReasonStep {
                stage: "foundation:F2",
                input: &f2_input,
                result: &f2_raw,
            },
            AcceptedFoundationReasonStep {
                stage: "foundation:Q1",
                input: &q1_input,
                result: &q1_raw,
            },
        ];
        let projected = project_missing_scope_reasons(&source, &context, &current, &steps).unwrap();
        assert_eq!(prior_auto_records(&projected.foundation), accepted_receipts);
        assert_eq!(projected.restored_variant_ids.len(), 3);
        let empty_times = HashMap::new();
        assert!(
            apply_optimization_with_receipt_times(
                &q1_raw,
                &source,
                &batch,
                &q1_input,
                &q1_basis,
                None,
                Some(&empty_times),
            )
            .unwrap_err()
            .contains("不制造新历史回执"),
            "pure replay must not fall back to the current clock"
        );
    }

    fn author_fixture() -> (LockedSource, FoundationProposal, Value) {
        let source = source(2);
        let scope = source
            .episodes
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let batch = plan_batches(&source, &scope).unwrap().remove(0);
        let mut basis = empty_foundation(&source);
        basis.episodes = source
            .episodes
            .iter()
            .map(|e| FoundationEpisode {
                episode_id: e.id.clone(),
                summary: e.title.clone(),
                context: String::new(),
                source_refs: vec![source
                    .rows
                    .iter()
                    .find(|r| r.episode_id.as_ref() == Some(&e.id))
                    .unwrap()
                    .id
                    .clone()],
            })
            .collect();
        let input = input_for("F2", &source, &batch, &basis, &json!({}), 0, 1, false, &[]);
        let mut catalogue = empty_catalogue(&source, &batch);
        catalogue["assets"] = json!([{"name":"小王","type":"CH","episodes":scope,"sceneRefs":source.scenes.iter().map(|s|json!([s.episode_id,s.id])).collect::<Vec<_>>(),"preparation":{"mode":"REFERENCE"}}]);
        basis = apply_catalogue(
            &catalogue,
            &source,
            &batch,
            &input,
            &basis,
            None,
            &HashSet::new(),
        )
        .unwrap();
        basis.assets[0].id = "CH_test".into();
        let refs = source
            .rows
            .iter()
            .filter(|r| r.episode_id.as_ref() == Some(&source.episodes[0].id))
            .take(2)
            .map(|r| r.id.clone())
            .collect::<Vec<_>>();
        let choice = json!({"decisionKey":"Name / Source conflict","kind":"SOURCE_CONFLICT","question":"制作身份采用哪种解释？","reason":"两处声明不同","options":["保留标注","制作解释分离"],"recommended":"制作解释分离","recommendationReason":"保留原文","evidence":"锁定原句","episodes":[source.episodes[0].id],"sourceRefs":refs,"changeTargets":{"assetKeys":["CH_test"],"assetFields":["description"],"sceneFields":["speaker"]}});
        basis.issues =
            supplemental_issues(&[choice], &[], &[], &batch, &allowed_refs(&input)).unwrap();
        let mut source_value = json!(source);
        source_value["version"] = json!(1);
        let mut foundation = json!(basis);
        foundation["version"] = json!(1);
        foundation["sourceVersion"] = json!(1);
        let contract =
            choice_contracts("p", &source_value, &foundation, &[]).unwrap()["contracts"][0].clone();
        (source, basis, contract)
    }
    #[test]
    fn author_contracts_dedupe_legacy_explanations_without_inheriting_permissions() {
        let (s, mut basis, _) = author_fixture();
        let mut duplicate = basis.issues[0].clone();
        duplicate.details.as_mut().unwrap()["choice"]["changeTargets"]["assetKeys"] = json!([]);
        basis.issues.push(duplicate);
        let mut sv = json!(s);
        sv["version"] = json!(1);
        let mut f = json!(basis);
        f["version"] = json!(1);
        f["sourceVersion"] = json!(1);
        let view = choice_contracts("p", &sv, &f, &[]).unwrap();
        assert_eq!(view["contracts"].as_array().unwrap().len(), 1);
        assert_eq!(view["contracts"][0]["members"].as_array().unwrap().len(), 2);
        assert_eq!(view["contracts"][0]["interpretationOnly"], true);
        assert!(view["contracts"][0]["changeTargets"].is_null());
        assert_eq!(view["contracts"][0]["subjectAssetKeys"], json!(["CH_test"]));
        let mut legal = basis.issues[0].clone();
        legal.details.as_mut().unwrap()["choice"]["changeTargets"] =
            json!({"assetKeys":["CH_test"],"assetFields":["relationNote"],"sceneFields":[]});
        basis.issues.push(legal);
        f = json!(basis);
        f["version"] = json!(1);
        f["sourceVersion"] = json!(1);
        assert_eq!(
            choice_contracts("p", &sv, &f, &[]).unwrap()["contracts"]
                .as_array()
                .unwrap()
                .len(),
            2,
            "valid rights are not merged into a missing/invalid consent"
        );
    }
    struct AuthorQ1 {
        calls: Mutex<Vec<TextRequest>>,
    }
    #[async_trait]
    impl TextExecutor for AuthorQ1 {
        async fn execute(&self, r: TextRequest) -> Result<Value, String> {
            assert_eq!(r.stage, "foundation:Q1");
            assert_eq!(r.input["delivery"]["phase"], "APPLY_DECISION");
            let answer = json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":r.input["sourceSha256"],"coverage":r.input["scope"],"changes":{},"appliedDecisions":r.input["confirmedDecisions"].as_array().unwrap().iter().map(|d|json!({"id":d["id"],"note":"原文标注完整保留，后续制作单独遵循已确认身份解释；现有目录无需扩大或更名。"})).collect::<Vec<_>>()});
            self.calls.lock().unwrap().push(r);
            Ok(answer)
        }
    }
    #[tokio::test]
    async fn author_apply_uses_only_original_q1_and_preserves_source_and_f2_catalogue() {
        let (s, basis, mut contract) = author_fixture();
        contract["status"] = json!("CONFIRMED");
        contract["answer"] = json!("保留说者、台词和动作；制作身份按上下文解释");
        contract["decidedAt"] = json!("confirmed");
        let context = json!({"foundation":basis,"foundationPhase":"APPLY_DECISION","confirmedDecisions":[contract],"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        let original = json!(s);
        let executor = AuthorQ1 {
            calls: Mutex::new(vec![]),
        };
        let (next, applied) =
            run_foundation_choice_application(&s, &context, &executor, "test", "medium")
                .await
                .unwrap();
        assert_eq!(json!(s), original);
        assert_eq!(next.assets[0].name, basis.assets[0].name);
        assert_eq!(
            json!(next.assets[0].variants),
            json!(basis.assets[0].variants)
        );
        assert_eq!(json!(next.facts), json!(basis.facts));
        assert!(next.issues.is_empty());
        assert_eq!(applied[0]["status"], "APPLIED");
        assert_eq!(executor.calls.lock().unwrap().len(), 1);
        assert!(next.assets[0].metadata["authorDecisions"][0]["sourceRefs"].is_array());
        let mut output = json!(next);
        output["version"] = json!(2);
        output["sourceVersion"] = json!(1);
        output["authorDecisions"] = json!(applied);
        let mut sv = json!(s);
        sv["version"] = json!(1);
        assert_eq!(
            choice_contracts("p", &sv, &output, &[]).unwrap()["contracts"][0]["status"],
            "APPLIED"
        );
    }
    #[test]
    fn author_receipts_and_permissions_fail_closed_without_source_edits() {
        let (s, basis, mut contract) = author_fixture();
        contract["status"] = json!("CONFIRMED");
        contract["answer"] = json!("保留原文");
        let batch = plan_batches(&s, &[s.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let input = input_for(
            "Q1",
            &s,
            &batch,
            &basis,
            &json!({"foundationPhase":"APPLY_DECISION","confirmedDecisions":[contract]}),
            0,
            1,
            true,
            &[],
        );
        let mut reply = empty_edit(&s, &batch);
        reply["appliedDecisions"] =
            json!([{"id":contract["id"],"note":"解释已用于后续制作，原文不动"}]);
        assert!(apply_optimization(&reply, &s, &batch, &input, &basis, None).is_ok());
        for bad in [
            json!([]),
            json!([{"id":"unknown","note":"应用"}]),
            json!([{"id":contract["id"],"note":""}]),
            json!([{"id":contract["id"],"note":"采用","answer":"改答案"}]),
        ] {
            let mut bad_reply = reply.clone();
            bad_reply["appliedDecisions"] = bad;
            assert!(apply_optimization(&bad_reply, &s, &batch, &input, &basis, None).is_err());
        }
        reply["changes"] = json!({"scenes":[{"episodeId":s.episodes[0].id,"sceneId":s.scenes[0].id,"speaker":"改名"}]});
        assert!(apply_optimization(&reply, &s, &batch, &input, &basis, None)
            .unwrap_err()
            .contains("speaker"));
        reply["changes"] = json!({"assets":[{"key":"CH_test","aliases":["伪造别名"]}]});
        assert!(apply_optimization(&reply, &s, &batch, &input, &basis, None).is_err());
        let mut legal = contract.clone();
        legal["interpretationOnly"] = json!(false);
        legal["changeTargets"] =
            json!({"assetKeys":["CH_test"],"assetFields":["relationNote"],"sceneFields":[]});
        let input = input_for(
            "Q1",
            &s,
            &batch,
            &basis,
            &json!({"foundationPhase":"APPLY_DECISION","confirmedDecisions":[legal]}),
            0,
            1,
            true,
            &[],
        );
        reply["changes"] = json!({"assets":[{"key":"CH_test","relationNote":"改变共享身份解释"}]});
        assert!(
            apply_optimization(&reply, &s, &batch, &input, &basis, None).is_err(),
            "one-episode decision cannot mutate the shared identity used by two episodes"
        );
    }
    fn auto_context(basis: &FoundationProposal, contract: Value) -> Value {
        json!({"projectId":"p","sourceVersion":1,"foundation":{"version":1,"sourceVersion":1,"sourceSha256":basis.source_sha256,"schemaVersion":basis.schema_version,"overview":basis.overview,"direction":basis.direction,"episodes":basis.episodes,"assets":basis.assets,"facts":basis.facts,"issues":basis.issues},"foundationPhase":"AUTO_INTERPRET","foundationDecisionPolicy":AUTO_INTERPRET_POLICY,"automaticDecisions":[contract],"userReviewRequirements":[],"foundationModelInputProfile":super::super::foundation_wire::PROFILE})
    }
    struct AutoAuthorQ1 {
        calls: Mutex<Vec<TextRequest>>,
    }
    #[async_trait]
    impl TextExecutor for AutoAuthorQ1 {
        async fn execute(&self, r: TextRequest) -> Result<Value, String> {
            assert_eq!(r.stage, "foundation:Q1");
            assert_eq!(r.input["foundationDecisionPolicy"], AUTO_INTERPRET_POLICY);
            let reply = json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":r.input["sourceSha256"],"coverage":r.input["scope"],"changes":{},"appliedDecisions":r.input["automaticDecisions"].as_array().unwrap().iter().map(|d|json!({"id":d["id"],"answer":d.get("previousAppliedAnswer").cloned().unwrap_or(json!("制作解释独立记录；源说者标注和全部台词、动作逐字保留。")),"note":"对照已提供原句完成最保守解释，不创建或更名任何人物身份。"})).collect::<Vec<_>>()});
            self.calls.lock().unwrap().push(r);
            Ok(reply)
        }
    }
    #[test]
    fn auto_author_requires_only_explicit_user_requirement_not_model_label() {
        let (_, _, mut contract) = author_fixture();
        contract["status"] = json!("CONFIRMED");
        contract["answer"] = json!("旧人工填写");
        contract["explicitReviewRequired"] = json!(true);
        let view = json!({"contracts":[contract]});
        let decisions = automatic_decisions(&view, &json!([])).unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0]["priorAnswer"], "旧人工填写");
        assert!(decisions[0]["answer"].is_null());
        assert_eq!(decisions[0]["status"], "MODEL_PENDING");
        assert!(automatic_decisions(&view,&json!([{"decisionKey":contract["decisionKey"],"explicitReviewRequired":true,"sourceSha256":contract["sourceSha256"],"applicationScope":contract["applicationScope"]}])).unwrap().is_empty());
        assert_eq!(
            automatic_decisions(
                &view,
                &json!([{"decisionKey":contract["decisionKey"],"explicitReviewRequired":false}])
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(automatic_decisions(&view,&json!([{"decisionKey":contract["decisionKey"],"explicitReviewRequired":true,"sourceSha256":"stale"}])).unwrap().len(),1);
    }
    #[tokio::test]
    async fn auto_author_completed_foundation_uses_one_q1_without_confirm_or_f2() {
        let (source, basis, mut contract) = author_fixture();
        contract["status"] = json!("CONFIRMED");
        contract["answer"] = json!("旧人工填写不作为模型的固定答案");
        let auto = automatic_decisions(&json!({"contracts":[contract]}), &json!([]))
            .unwrap()
            .remove(0);
        let context = auto_context(&basis, auto);
        let executor = AutoAuthorQ1 {
            calls: Mutex::new(vec![]),
        };
        let before = json!(source);
        let (next, records) =
            run_foundation_auto_interpretation(&source, &context, &executor, "test", "medium")
                .await
                .unwrap();
        assert_eq!(executor.calls.lock().unwrap().len(), 1);
        assert_eq!(json!(source), before);
        assert_eq!(next.assets[0].name, basis.assets[0].name);
        assert_eq!(next.assets[0].aliases, basis.assets[0].aliases);
        assert_eq!(json!(next.facts), json!(basis.facts));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["status"], "APPLIED");
        assert_eq!(records[0]["authority"], "MODEL_PRODUCTION_INTERPRETATION");
        assert_ne!(records[0]["answer"], contract["answer"]);
        assert_eq!(records[0]["applicationScope"], contract["applicationScope"]);
        assert_eq!(records[0]["sourceRefs"], contract["sourceRefs"]);
        assert!(next.issues.iter().all(|i| !i.blocking));
        assert_eq!(
            next.assets[0].metadata["authorDecisions"][0]["id"],
            records[0]["id"]
        );
    }
    #[test]
    fn auto_author_rejects_scope_identity_receipt_and_source_mutation() {
        let (source, basis, contract) = author_fixture();
        let auto = automatic_decisions(&json!({"contracts":[contract]}), &json!([]))
            .unwrap()
            .remove(0);
        let context = auto_context(&basis, auto.clone());
        let batch = plan_batches(&source, &[source.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let input = input_for("Q1", &source, &batch, &basis, &context, 0, 1, true, &[]);
        let reply = json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":source.sha256,"coverage":batch.scope,"changes":{},"appliedDecisions":[{"id":auto["id"],"answer":"源文本完整保留，制作解释单独记录","note":"已采用保守解释"}]});
        assert!(apply_optimization(&reply, &source, &batch, &input, &basis, None).is_ok());
        for invalid in [
            json!([]),
            json!([{"id":"invented","answer":"解释","note":"采用"}]),
            json!([{"id":auto["id"],"answer":"解释","note":"采用","subjectAssetKeys":["new-person"]}]),
            json!([{"id":auto["id"],"answer":"解释","note":"采用","sourceSha256":"fake"}]),
        ] {
            let mut changed = reply.clone();
            changed["appliedDecisions"] = invalid;
            assert!(apply_optimization(&changed, &source, &batch, &input, &basis, None).is_err());
        }
        for changes in [
            json!({"assets":[{"key":"CH_test","aliases":["无据别名"]}]}),
            json!({"scenes":[{"episodeId":source.episodes[0].id,"sceneId":source.scenes[0].id,"speaker":"改说者"}]}),
        ] {
            let mut changed = reply.clone();
            changed["changes"] = changes;
            assert!(apply_optimization(&changed, &source, &batch, &input, &basis, None).is_err());
        }
        let mut changed = input.clone();
        changed["automaticDecisions"][0]["applicationScope"] = json!({"kind":"GLOBAL","episodes":source.episodes.iter().map(|e|e.id.clone()).collect::<Vec<_>>()});
        assert!(apply_optimization(&reply, &source, &batch, &changed, &basis, None).is_err());
    }
    struct FreshAutoFlow {
        calls: Mutex<Vec<String>>,
        new_choice: bool,
    }
    #[async_trait]
    impl TextExecutor for FreshAutoFlow {
        async fn execute(&self, r: TextRequest) -> Result<Value, String> {
            self.calls.lock().unwrap().push(r.stage.clone());
            let first = &r.input["source"]["rows"][0];
            let choice = json!({"decisionKey":"source/color","kind":"SOURCE_CONFLICT","question":"颜色如何制作解释？","reason":"保留源文本矛盾","options":["保留原句并分离制作解释","重写原句"],"recommended":"保留原句并分离制作解释","recommendationReason":"源文不改","evidence":"源原句","episodes":r.input["scope"],"sourceRefs":[first[0]],"changeTargets":{"assetKeys":[],"assetFields":["description"],"sceneFields":["speaker"]}});
            if r.stage == "foundation:F2" {
                return Ok(
                    json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":r.input["sourceSha256"],"coverage":r.input["scope"],"assets":[{"name":"小王","type":"CH","episodes":r.input["scope"],"sceneRefs":r.input["scenes"].as_array().unwrap().iter().map(|s|json!([s[0],s[1]])).collect::<Vec<_>>(),"preparation":{"mode":"REFERENCE"}}],"humanChoices":if self.new_choice{vec![]}else{vec![choice]}}),
                );
            }
            let subjects = json!([r.input["candidates"][0]["key"]]);
            let receipts = if self.new_choice {
                vec![
                    json!({"decisionKey":choice["decisionKey"],"answer":"保留源句并按镜头光照解释颜色，不登记为源事实。","note":"在本次Q1完成解释采用","subjectAssetKeys":subjects}),
                ]
            } else {
                r.input["automaticDecisions"].as_array().unwrap().iter().map(|d|json!({"id":d["id"],"answer":"保留源句并按镜头光照解释颜色，不登记为源事实。","note":"在原Q1完成解释采用","subjectAssetKeys":subjects})).collect()
            };
            Ok(
                json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":r.input["sourceSha256"],"coverage":r.input["scope"],"changes":{},"humanChoices":if self.new_choice{vec![choice]}else{vec![]},"appliedDecisions":receipts}),
            )
        }
    }
    #[tokio::test]
    async fn auto_author_fresh_f2_and_new_q1_choice_adopt_within_same_original_q1() {
        for new_choice in [false, true] {
            let source = source(1);
            let executor = FreshAutoFlow {
                calls: Mutex::new(vec![]),
                new_choice,
            };
            let context = json!({"projectId":"p","sourceVersion":1,"foundationDecisionPolicy":AUTO_INTERPRET_POLICY,"userReviewRequirements":[],"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
            let next = run_foundation_delivery(&source, &context, &executor, "test", "medium")
                .await
                .unwrap();
            assert_eq!(
                *executor.calls.lock().unwrap(),
                vec!["foundation:F2", "foundation:Q1"]
            );
            let records = foundation_auto_applied_decisions(&next);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0]["sourceSha256"], source.sha256);
            assert_eq!(records[0]["subjectAssetKeys"], json!([next.assets[0].id]));
            assert!(next.issues.iter().all(|i| !i.blocking));
        }
    }
    #[test]
    fn auto_author_edit_keeps_original_catalogue_editor_authority() {
        let (source, basis, contract) = author_fixture();
        let batch = plan_batches(&source, &[source.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let mut context = auto_context(&basis, contract);
        let auto_input = input_for("Q1", &source, &batch, &basis, &context, 0, 1, true, &[]);
        let auto_guide = auto_input["transportInstructions"].as_str().unwrap();
        assert!(auto_guide.contains("AUTO_INTERPRET只采用解释"));
        assert!(auto_guide.contains("changes必须为空"));

        context["foundationPhase"] = Value::Null;
        let edit_input = input_for("Q1", &source, &batch, &basis, &context, 0, 1, true, &[]);
        let edit_guide = edit_input["transportInstructions"].as_str().unwrap();
        assert_eq!(edit_input["delivery"]["phase"], "EDIT");
        assert!(edit_guide.contains("自动取舍和实际修正必须在同一回答完成"));
        assert!(edit_guide.contains("UNRESOLVED制作方式须依已选解释与原文依据落实"));
        assert!(!edit_guide.contains("changes必须为空"));
        assert!(!edit_guide.contains("不修改目录/方向/源事实"));
        let compiled = super::super::foundation_wire::compile(
            "Q1",
            &edit_input,
            &delivery_schema("Q1", &edit_input),
        )
        .unwrap();
        assert!(compiled.input["transportInstructions"]
            .as_str()
            .unwrap()
            .contains("自动取舍和实际修正必须在同一回答完成"));
    }
    struct FreshAutoCatalogueEditor {
        calls: Mutex<Vec<String>>,
        edit_catalogue: bool,
    }
    #[async_trait]
    impl TextExecutor for FreshAutoCatalogueEditor {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            self.calls.lock().unwrap().push(request.stage.clone());
            let scope = &request.input["scope"];
            let rows = request.input["source"]["rows"].as_array().unwrap();
            let refs = json!([rows[0][0], rows[1][0]]);
            if request.stage == "foundation:F2" {
                return Ok(json!({
                    "format":"ASSET_CATALOG_DESKTOP_V2", "sourceSha256":request.input["sourceSha256"], "coverage":scope,
                    "assets":[{"name":"小王", "type":"CH", "episodes":scope, "sceneRefs":request.input["scenes"].as_array().unwrap().iter().map(|s|json!([s[0],s[1]])).collect::<Vec<_>>(), "preparation":{"mode":"UNRESOLVED", "note":"待本次Q1完成造型制作解释"}}],
                    "issues":[{"coordinate":"catalogue:production-look", "problem":"造型制作方式仍未落实", "blocking":true, "sourceRefs":refs, "episodeIds":scope}],
                    "humanChoices":[{"decisionKey":"production-look", "kind":"PRODUCTION_CHOICE", "question":"采用哪种制作解释？", "reason":"需要稳定人物基线", "options":["按原文白衣制作", "保持待定"], "recommended":"按原文白衣制作", "recommendationReason":"原文实际动作明确服装", "evidence":"已提供原行", "episodes":scope, "sourceRefs":refs, "changeTargets":{"assetKeys":[], "assetFields":["description","preparation"], "sceneFields":[]}}]
                }));
            }
            assert_eq!(request.stage, "foundation:Q1");
            assert_eq!(request.input["delivery"]["phase"], "EDIT");
            assert!(!request.input["transportInstructions"]
                .as_str()
                .unwrap()
                .contains("changes必须为空"));
            let key = &request.input["candidates"][0]["key"];
            Ok(json!({
                "format":"ASSET_OPTIMIZATION_DESKTOP_V2", "sourceSha256":request.input["sourceSha256"], "coverage":scope,
                "changes":if self.edit_catalogue {json!({"assets":[{"key":key, "description":"白衣按源文实际动作制作；本次解释不改原文", "preparation":{"mode":"REFERENCE", "note":"使用原文白衣作为稳定基线"}}]})} else {json!({})},
                "appliedDecisions":request.input["automaticDecisions"].as_array().unwrap().iter().map(|d|json!({"id":d["id"], "answer":"按原文白衣制作", "note":"仅落实稳定造型，不改源说者、对白或动作", "subjectAssetKeys":[key]})).collect::<Vec<_>>(),
                "resolvedIssues":["catalogue:production-look", "choice:production-look"]
            }))
        }
    }
    #[tokio::test]
    async fn auto_author_edit_resolves_production_basis_in_same_q1_not_receipt_only() {
        let source = source(1);
        let frozen_source = json!(source);
        let context = json!({"projectId":"p", "sourceVersion":1, "foundationDecisionPolicy":AUTO_INTERPRET_POLICY, "userReviewRequirements":[], "foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        for edit_catalogue in [false, true] {
            let executor = FreshAutoCatalogueEditor {
                calls: Mutex::new(vec![]),
                edit_catalogue,
            };
            let result =
                run_foundation_delivery(&source, &context, &executor, "test", "medium").await;
            assert_eq!(
                *executor.calls.lock().unwrap(),
                vec!["foundation:F2", "foundation:Q1"]
            );
            assert_eq!(json!(source), frozen_source);
            if edit_catalogue {
                let next = result.unwrap();
                assert_eq!(next.assets[0].metadata["preparation"]["mode"], "REFERENCE");
                assert_eq!(
                    next.assets[0].variants[0].metadata["preparation"]["mode"],
                    "REFERENCE"
                );
                assert!(next.issues.iter().all(|issue| !issue.blocking));
                assert_eq!(foundation_auto_applied_decisions(&next).len(), 1);
            } else {
                assert!(result.unwrap_err().contains("不能仅回执"));
            }
        }
    }
    #[test]
    fn auto_author_explicit_user_stop_keeps_original_evidence() {
        let (source, basis, contract) = author_fixture();
        let mut context = auto_context(&basis, contract.clone());
        context["foundationPhase"] = Value::Null;
        context["userReviewRequirements"] =
            json!([{"id":contract["id"],"needsUserConfirmation":true}]);
        let batch = plan_batches(&source, &[source.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let input = input_for("Q1", &source, &batch, &basis, &context, 0, 1, true, &[]);
        assert!(input["automaticDecisions"].as_array().unwrap().is_empty());
        let mut reply = empty_edit(&source, &batch);
        reply["appliedDecisions"] = json!([]);
        let next = apply_optimization(&reply, &source, &batch, &input, &basis, None)
            .unwrap()
            .0;
        assert_eq!(next.issues[0].source_refs, basis.issues[0].source_refs);
        assert!(next.issues[0].blocking);
        assert!(foundation_auto_applied_decisions(&next).is_empty());
    }
    #[test]
    fn auto_author_capacity_batches_keep_answer_subject_and_complete_coverage() {
        let (source, mut basis, _) = author_fixture();
        basis.issues[0].details.as_mut().unwrap()["choice"]["episodes"] = json!(source
            .episodes
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>());
        let view = auto_view(
            &source,
            &basis,
            &json!({"projectId":"p","sourceVersion":1,"foundation":{"version":1}}),
        )
        .unwrap();
        let automatic = automatic_decisions(&view, &json!([])).unwrap().remove(0);
        let context = auto_context(&basis, automatic.clone());
        let first = plan_batches(&source, &[source.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let input = input_for("Q1", &source, &first, &basis, &context, 0, 2, false, &[]);
        let mut reply = empty_edit(&source, &first);
        reply["appliedDecisions"] = json!([{"id":automatic["id"],"answer":"同一保守制作解释","note":"第一容量批已采用","subjectAssetKeys":["CH_test"]}]);
        let partial = apply_optimization(&reply, &source, &first, &input, &basis, None)
            .unwrap()
            .0;
        assert!(foundation_auto_applied_decisions(&partial).is_empty());
        assert_eq!(
            prior_auto_records(&partial)[0]["status"],
            "PARTIALLY_APPLIED"
        );
        let second = plan_batches(&source, &[source.episodes[1].id.clone()])
            .unwrap()
            .remove(0);
        let input = input_for("Q1", &source, &second, &partial, &context, 1, 2, true, &[]);
        assert_eq!(
            input["automaticDecisions"][0]["previousAppliedAnswer"],
            "同一保守制作解释"
        );
        assert_eq!(
            input["automaticDecisions"][0]["previousSubjectAssetKeys"],
            json!(["CH_test"])
        );
        let mut reply = empty_edit(&source, &second);
        reply["appliedDecisions"] = json!([{"id":automatic["id"],"answer":"同一保守制作解释","note":"第二容量批已采用","subjectAssetKeys":["CH_test"]}]);
        let mut changed = reply.clone();
        changed["appliedDecisions"][0]["answer"] = json!("另一解释");
        assert!(apply_optimization(&changed, &source, &second, &input, &partial, None).is_err());
        let complete = apply_optimization(&reply, &source, &second, &input, &partial, None)
            .unwrap()
            .0;
        let record = &foundation_auto_applied_decisions(&complete)[0];
        assert_eq!(record["status"], "APPLIED");
        assert_eq!(record["receipts"].as_array().unwrap().len(), 2);
        assert!(complete.issues.iter().all(|issue| !issue.blocking));
    }
    #[test]
    fn auto_author_applied_receipt_reuses_only_exact_source_evidence() {
        let (source, basis, contract) = author_fixture();
        let automatic = automatic_decisions(&json!({"contracts":[contract]}), &json!([]))
            .unwrap()
            .remove(0);
        let context = auto_context(&basis, automatic.clone());
        let batch = plan_batches(
            &source,
            &source
                .episodes
                .iter()
                .map(|e| e.id.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .remove(0);
        let input = input_for("Q1", &source, &batch, &basis, &context, 0, 1, true, &[]);
        let mut reply = empty_edit(&source, &batch);
        reply["appliedDecisions"] =
            json!([{"id":automatic["id"],"answer":"制作解释不改源文","note":"完整采用"}]);
        let mut applied = apply_optimization(&reply, &source, &batch, &input, &basis, None)
            .unwrap()
            .0;
        let issue = basis
            .issues
            .iter()
            .find(|issue| {
                issue
                    .details
                    .as_ref()
                    .is_some_and(|d| d["kind"] == "HUMAN_CHOICE")
            })
            .unwrap()
            .clone();
        applied.issues.push(issue.clone());
        let mut fresh = context.clone();
        fresh["foundationPhase"] = Value::Null;
        let input = input_for("Q1", &source, &batch, &applied, &fresh, 0, 1, true, &[]);
        assert!(input["automaticDecisions"].as_array().unwrap().is_empty());
        assert_eq!(input["humanChoiceRegistry"][0]["status"], "APPLIED");
        let mut reply = empty_edit(&source, &batch);
        reply["appliedDecisions"] = json!([]);
        let reused = apply_optimization(&reply, &source, &batch, &input, &applied, None)
            .unwrap()
            .0;
        assert!(reused.issues.iter().all(|issue| !issue.blocking));
        applied.issues.last_mut().unwrap().details.as_mut().unwrap()["choice"]["evidence"] =
            json!("证据已变");
        let changed = input_for("Q1", &source, &batch, &applied, &fresh, 0, 1, true, &[]);
        assert_eq!(changed["automaticDecisions"].as_array().unwrap().len(), 1);
        assert!(changed["automaticDecisions"][0]["previousAppliedAnswer"].is_null());
    }
    #[test]
    fn auto_author_support_evidence_does_not_expand_application_scope_or_mutate_source() {
        let (source, mut basis, _) = author_fixture();
        let support = source
            .rows
            .iter()
            .find(|row| {
                row.episode_id.as_ref() == Some(&source.episodes[1].id) && row.dialogue.is_some()
            })
            .unwrap();
        basis.issues[0].details.as_mut().unwrap()["choice"]["sourceRefs"]
            .as_array_mut()
            .unwrap()
            .push(json!(support.id));
        let view = auto_view(
            &source,
            &basis,
            &json!({"projectId":"p","sourceVersion":1,"foundation":{"version":1}}),
        )
        .unwrap();
        let contract = automatic_decisions(&view, &json!([])).unwrap().remove(0);
        let context = auto_context(&basis, contract.clone());
        let batch = plan_batches(&source, &[source.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let input = input_for("Q1", &source, &batch, &basis, &context, 0, 1, true, &[]);
        assert!(sent_decision_evidence(&input, support));
        let mut reply = empty_edit(&source, &batch);
        reply["appliedDecisions"] = json!([{"id":contract["id"],"answer":"先前声明仅用于本场制作解释，不改原文或身份","note":"引用已实际提供的另一集支持证据","subjectAssetKeys":["CH_test"]}]);
        let next = apply_optimization(&reply, &source, &batch, &input, &basis, None)
            .unwrap()
            .0;
        let record = &foundation_auto_applied_decisions(&next)[0];
        assert_eq!(record["applicationScope"], contract["applicationScope"]);
        assert_eq!(record["episodes"], json!([source.episodes[0].id]));
        assert_eq!(record["sourceRefs"], contract["sourceRefs"]);
        assert_eq!(json!(next.facts), json!(basis.facts));
        assert_eq!(next.assets[0].aliases, basis.assets[0].aliases);
        let mut unsent = input.clone();
        unsent["evidence"]["historicalEvidenceCandidates"] = json!([]);
        unsent["evidence"]["automaticDecisionSupport"] = Value::Null;
        assert_eq!(
            apply_optimization(&reply, &source, &batch, &unsent, &basis, None).unwrap_err(),
            AUTO_INTERPRET_EVIDENCE_REJECTION
        );
        let mut altered = input.clone();
        altered["evidence"]["automaticDecisionSupport"] =
            json!({"columns":["rowId","text"],"rows":[[support.id,"伪造支持原文"]]});
        assert_eq!(
            apply_optimization(&reply, &source, &batch, &altered, &basis, None).unwrap_err(),
            AUTO_INTERPRET_EVIDENCE_REJECTION
        );
        let mut source_change = reply.clone();
        source_change["sourceSha256"] = json!("other-source");
        assert!(apply_optimization(&source_change, &source, &batch, &input, &basis, None).is_err());
        let mut no_anchor = basis.clone();
        no_anchor.issues[0].details.as_mut().unwrap()["choice"]["sourceRefs"] = json!([support.id]);
        let view = auto_view(
            &source,
            &no_anchor,
            &json!({"projectId":"p","sourceVersion":1,"foundation":{"version":1}}),
        )
        .unwrap();
        let contract = automatic_decisions(&view, &json!([])).unwrap().remove(0);
        let input = input_for(
            "Q1",
            &source,
            &batch,
            &no_anchor,
            &auto_context(&no_anchor, contract.clone()),
            0,
            1,
            true,
            &[],
        );
        reply["appliedDecisions"][0]["id"] = contract["id"].clone();
        assert_eq!(
            apply_optimization(&reply, &source, &batch, &input, &no_anchor, None).unwrap_err(),
            AUTO_INTERPRET_EVIDENCE_REJECTION
        );
    }
    #[test]
    fn auto_author_exact_adoption_closes_only_matching_author_and_q1_todos() {
        let (source, mut basis, contract) = author_fixture();
        let contract = automatic_decisions(&json!({"contracts":[contract]}), &json!([]))
            .unwrap()
            .remove(0);
        let mut pending = basis.issues[0].clone();
        pending.coordinate = "pending:auto-exact".into();
        pending.asset_id = Some("CH_test".into());
        pending.details = Some(json!({"owner":"Q1","pending":{"owner":"Q1"}}));
        basis.issues.push(pending.clone());
        let batch = plan_batches(&source, &[source.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let make_input = |basis: &FoundationProposal| {
            input_for(
                "Q1",
                &source,
                &batch,
                basis,
                &auto_context(basis, contract.clone()),
                0,
                1,
                true,
                &[],
            )
        };
        let input = make_input(&basis);
        let mut reply = empty_edit(&source, &batch);
        reply["appliedDecisions"] = json!([{"id":contract["id"],"answer":"仅采用本集制作解释，原文保持逐字不变","note":"实际采用现有主体且精确覆盖待办证据","subjectAssetKeys":["CH_test"]}]);
        reply["resolvedIssues"] = json!([basis.issues[0].coordinate, pending.coordinate]);
        let next = apply_optimization(&reply, &source, &batch, &input, &basis, None)
            .unwrap()
            .0;
        assert!(next.issues.iter().all(|issue| !issue.blocking));
        assert_eq!(foundation_auto_applied_decisions(&next).len(), 1);
        assert_eq!(json!(next.facts), json!(basis.facts));
        for owner in ["PROGRAM", "SOURCE"] {
            let mut changed = basis.clone();
            changed.issues[1].details = Some(json!({"owner":owner,"pending":{"owner":owner}}));
            assert!(apply_optimization(
                &reply,
                &source,
                &batch,
                &make_input(&changed),
                &changed,
                None
            )
            .unwrap_err()
            .contains("不能仅回执"));
        }
        let mut unrelated = basis.clone();
        let other = source
            .rows
            .iter()
            .find(|row| {
                row.episode_id.as_ref() == Some(&source.episodes[0].id)
                    && !pending.source_refs.contains(&row.id)
            })
            .unwrap();
        unrelated.issues[1].source_refs.push(other.id.clone());
        assert!(apply_optimization(
            &reply,
            &source,
            &batch,
            &make_input(&unrelated),
            &unrelated,
            None
        )
        .unwrap_err()
        .contains("不能仅回执"));
        let mut colliding = basis.clone();
        let mut program = pending;
        program.coordinate = colliding.issues[0].coordinate.clone();
        program.details = Some(json!({"owner":"PROGRAM"}));
        colliding.issues.push(program);
        assert!(apply_optimization(
            &reply,
            &source,
            &batch,
            &make_input(&colliding),
            &colliding,
            None
        )
        .unwrap_err()
        .contains("不能仅回执"));
    }
    #[test]
    #[ignore = "requires DIRECTOR_AUTO_Q1_REAL_DB, DIRECTOR_AUTO_Q1_WORKFLOW_ID and DIRECTOR_AUTO_Q1_TEXT_JOB_ID; readonly frozen reply, zero CLI or production writes"]
    fn saved_actual_auto_q1_cross_scope_reply_revalidates_without_generation() {
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| panic!("required explicit environment variable: {name}"))
        };
        let path = std::path::PathBuf::from(required("DIRECTOR_AUTO_Q1_REAL_DB"));
        let workflow_id = required("DIRECTOR_AUTO_Q1_WORKFLOW_ID");
        let text_job_id = required("DIRECTOR_AUTO_Q1_TEXT_JOB_ID");
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let (snapshot, raw): (String, String) = connection
            .query_row(
                "SELECT j.snapshot,j.result FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE j.id=?1 AND w.workflow_id=?2",
                rusqlite::params![text_job_id,workflow_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let frozen: Value = serde_json::from_str(&snapshot).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        let workflow: String = connection
            .query_row(
                "SELECT snapshot FROM jobs WHERE id=?1",
                [&workflow_id],
                |row| row.get(0),
            )
            .unwrap();
        let workflow: Value = serde_json::from_str(&workflow).unwrap();
        let input = &workflow["input"];
        let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
        let context = json!({"foundation":input["foundation"],"assets":input["assets"],"selection":input["selection"],"foundationModelInputProfile":input["foundationModelInputProfile"],"foundationPhase":input["foundationPhase"],"confirmedDecisions":input["confirmedDecisions"],"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundationDecisionPolicy":input["foundationDecisionPolicy"],"automaticDecisions":input["automaticDecisions"],"userReviewRequirements":input["userReviewRequirements"],"settings":{"confirmedDirection":input["foundation"]["productionDirection"]}});
        let prior = existing_foundation(&source, &context).unwrap();
        let selected = choice_strings(&frozen["input"]["scope"], "scope").unwrap();
        let mut pending = editor_batches(plan_batches(&source, &selected).unwrap());
        let (batch, rebuilt) =
            prepare_capacity_batch("Q1", &source, &mut pending, &prior, &context, 0, &[]).unwrap();
        assert!(
            pending.is_empty(),
            "the frozen automatic interpretation was one exact Q1 call"
        );
        let changed_keys: Vec<_> = rebuilt
            .as_object()
            .unwrap()
            .keys()
            .chain(frozen["input"].as_object().unwrap().keys())
            .filter(|key| rebuilt[*key] != frozen["input"][*key])
            .collect();
        assert!(
            changed_keys.is_empty(),
            "rebuilt frozen input changed fields: {changed_keys:?}"
        );
        assert_eq!(delivery_schema("Q1", &rebuilt), frozen["schema"]);
        let next = apply_optimization(&value, &source, &batch, &frozen["input"], &prior, None)
            .unwrap()
            .0;
        let records = foundation_auto_applied_decisions(&next);
        assert_eq!(records.len(), 4);
        assert!(records.iter().all(|record| record["status"] == "APPLIED"));
        assert!(next.issues.iter().all(|issue| !issue.blocking));
        assert_eq!(json!(next.facts), json!(prior.facts));
        assert_eq!(
            next.assets
                .iter()
                .map(|asset| (&asset.id, &asset.name, &asset.aliases))
                .collect::<Vec<_>>(),
            prior
                .assets
                .iter()
                .map(|asset| (&asset.id, &asset.name, &asset.aliases))
                .collect::<Vec<_>>()
        );
        validate_source(&source).unwrap();
        validate_foundation(&source, &next).unwrap();
    }
    struct Counter {
        requests: Mutex<Vec<TextRequest>>,
    }
    #[async_trait]
    impl TextExecutor for Counter {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            let scope = request.input["scope"].clone();
            let hash = request.input["sourceSha256"].clone();
            let stage = request.stage.clone();
            self.requests.lock().unwrap().push(request);
            Ok(if stage == "foundation:F2" {
                json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":hash,"coverage":scope,"assets":[]})
            } else {
                json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":hash,"coverage":scope,"changes":{}})
            })
        }
    }
    #[tokio::test]
    async fn ten_short_episodes_are_one_catalogue_and_one_editor_call() {
        let source = source(10);
        let executor = Counter {
            requests: Mutex::new(vec![]),
        };
        let result =
            run_foundation_delivery(&source, &json!({}), &executor, "gpt-6.1-sol", "medium")
                .await
                .unwrap();
        let calls = executor.requests.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].input["scope"].as_array().unwrap().len(), 10);
        assert_eq!(result.episodes.len(), 10);
        assert_eq!(result.updated_episode_ids.len(), 10);
        assert!(calls[0].input["source"]["rows"][0].is_array());
        assert!(calls[0].schema["properties"].get("episodes").is_none());
        assert_eq!(
            calls[1].schema["properties"]["changes"]["required"],
            json!([])
        );
    }
    #[test]
    fn long_utf16_fragments_cover_crlf_surrogates_and_every_locked_row() {
        let text = format!(
            "第1集\r\n场景：大厅\r\n△{}\r\n小王：结束。\r\n第2集\r\n场景：门口\r\n小王：走。\r\n",
            "😀甲".repeat(9000)
        );
        let s = prepare_source(&text).unwrap();
        let selected = s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        let batches = plan_batches(&s, &selected).unwrap();
        assert!(batches.iter().any(|b| b.fragment.is_some()));
        verify_coverage(&s, &selected, &batches).unwrap();
        for batch in batches.iter().filter(|b| b.fragment.is_some()) {
            assert!(batch.pieces.iter().map(|p| units(&p.text)).sum::<usize>() <= FRAGMENT_BUDGET);
            for p in &batch.pieces {
                assert_eq!(
                    slice_utf16(
                        &s.text,
                        &SourceRange {
                            start: p.start,
                            end: p.end
                        }
                    )
                    .unwrap(),
                    p.text
                );
            }
        }
    }
    #[test]
    fn empty_editor_changes_preserve_every_canonical_value() {
        let s = source(2);
        let batch = plan_batches(
            &s,
            &s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        )
        .unwrap()
        .remove(0);
        let p = empty_foundation(&s);
        let input = input_for("Q1", &s, &batch, &p, &json!({}), 0, 1, true, &[]);
        let (next, _) =
            apply_optimization(&empty_edit(&s, &batch), &s, &batch, &input, &p, None).unwrap();
        assert_eq!(
            serde_json::to_value(next).unwrap(),
            serde_json::to_value(p).unwrap()
        );
    }
    #[test]
    fn q1_explicit_display_names_keep_keys_sources_and_aliases() {
        for kind in ["PROP", "LOC"] {
            let source = source(2);
            let selected = source
                .episodes
                .iter()
                .map(|e| e.id.clone())
                .collect::<Vec<_>>();
            let batch = plan_batches(&source, &selected).unwrap().remove(0);
            let initial = empty_foundation(&source);
            let f2_input = input_for(
                "F2",
                &source,
                &batch,
                &initial,
                &json!({}),
                0,
                1,
                false,
                &[],
            );
            let mut raw = empty_catalogue(&source, &batch);
            raw["assets"] = json!([{"name":"稳定对象｜待定", "type":kind, "aliases":["已有别名"], "episodes":selected, "sceneRefs":source.scenes.iter().map(|s|json!([s.episode_id,s.id])).collect::<Vec<_>>(), "preparation":{"mode":"REFERENCE"}, "variants":[{"name":"持续状态", "episodes":selected, "sceneRefs":source.scenes.iter().map(|s|json!([s.episode_id,s.id])).collect::<Vec<_>>(), "preparation":{"mode":"DERIVE"}}]}]);
            let basis = apply_catalogue(
                &raw,
                &source,
                &batch,
                &f2_input,
                &initial,
                None,
                &HashSet::new(),
            )
            .unwrap();
            let original = &basis.assets[0];
            let input = input_for("Q1", &source, &batch, &basis, &json!({}), 0, 1, true, &[]);
            let mut reply = empty_edit(&source, &batch);
            reply["changes"] = json!({"assets":[{"key":original.id, "name":"稳定对象｜明确制作解释", "variants":[{"key":original.variants[0].id, "name":"稳定对象｜明确制作解释"}, {"key":original.variants[1].id, "name":"持续状态｜明确显示名"}]}]});
            let next = apply_optimization(&reply, &source, &batch, &input, &basis, None)
                .unwrap()
                .0;
            let mut unchanged_labels = reply.clone();
            unchanged_labels["changes"]["assets"][0]["name"] = json!(original.name);
            for (index, variant) in original.variants.iter().enumerate() {
                unchanged_labels["changes"]["assets"][0]["variants"][index]["name"] =
                    json!(variant.name);
            }
            let control =
                apply_optimization(&unchanged_labels, &source, &batch, &input, &basis, None)
                    .unwrap()
                    .0;
            let renamed = &next.assets[0];
            assert_eq!(renamed.id, original.id);
            assert_eq!(renamed.name, "稳定对象｜明确制作解释");
            assert_eq!(renamed.aliases, original.aliases);
            assert_eq!(renamed.variants[0].name, renamed.name);
            assert_eq!(renamed.variants[1].name, "持续状态｜明确显示名");
            for ((old, current), unchanged) in original
                .variants
                .iter()
                .zip(&renamed.variants)
                .zip(&control.assets[0].variants)
            {
                assert_eq!(old.id, current.id);
                assert_eq!(old.source_variant_id, current.source_variant_id);
                // Omitted uses receive the existing program scope anchors in
                // either delta. Editing a display label adds no different refs.
                assert_eq!(unchanged.source_refs, current.source_refs);
                assert!(old
                    .source_refs
                    .iter()
                    .all(|r| current.source_refs.contains(r)));
                assert_eq!(current.metadata["sourceRefsPurpose"], "SOURCE_SCOPE");
                assert_eq!(old.scope.scenes, current.scope.scenes);
                assert_eq!(old.metadata["preparation"], current.metadata["preparation"]);
            }
            assert_eq!(json!(next.facts), json!(basis.facts));
            let mut implicit = reply.clone();
            implicit["changes"]["assets"][0]["variants"][0]["key"] = json!("another-owner:base");
            assert!(
                apply_optimization(&implicit, &source, &batch, &input, &basis, None)
                    .unwrap_err()
                    .contains("未知形象key")
            );
            let mut unknown_asset = reply.clone();
            unknown_asset["changes"]["assets"][0]["key"] = json!("unprovided-identity");
            assert!(
                apply_optimization(&unknown_asset, &source, &batch, &input, &basis, None)
                    .unwrap_err()
                    .contains("未知资产key")
            );
        }
    }
    #[test]
    fn q1_character_base_display_edit_must_be_explicit_not_top_name_side_effect() {
        let (source, basis) = two_episode_catalogue();
        let selected = source
            .episodes
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let batch = plan_batches(&source, &selected).unwrap().remove(0);
        let input = input_for("Q1", &source, &batch, &basis, &json!({}), 0, 1, true, &[]);
        let original = &basis.assets[0];
        let mut reply = empty_edit(&source, &batch);
        reply["changes"] = json!({"assets":[{"key":original.id, "name":"小王｜白衣（日常）"}]});
        assert!(
            apply_optimization(&reply, &source, &batch, &input, &basis, None)
                .unwrap_err()
                .contains("基线服装")
        );
        reply["changes"]["assets"][0]["variants"] =
            json!([{"key":original.variants[0].id, "name":"白衣（日常）"}]);
        let next = apply_optimization(&reply, &source, &batch, &input, &basis, None)
            .unwrap()
            .0;
        let mut unchanged_labels = reply.clone();
        unchanged_labels["changes"]["assets"][0]["name"] = original.metadata["catalogName"].clone();
        unchanged_labels["changes"]["assets"][0]["variants"][0]["name"] =
            json!(original.variants[0].name);
        let control = apply_optimization(&unchanged_labels, &source, &batch, &input, &basis, None)
            .unwrap()
            .0;
        assert_eq!(next.assets[0].id, original.id);
        assert_eq!(next.assets[0].variants[0].id, original.variants[0].id);
        assert_eq!(next.assets[0].variants[0].name, "白衣（日常）");
        assert_eq!(
            next.assets[0].variants[0].source_variant_id,
            original.variants[0].source_variant_id
        );
        assert_eq!(
            next.assets[0].variants[0].source_refs,
            control.assets[0].variants[0].source_refs
        );
        assert!(original.variants[0]
            .source_refs
            .iter()
            .all(|r| next.assets[0].variants[0].source_refs.contains(r)));
        assert_eq!(
            next.assets[0].variants[0].metadata["sourceRefsPurpose"],
            "SOURCE_SCOPE"
        );
        assert_eq!(
            next.assets[0].variants[0].scope.scenes,
            original.variants[0].scope.scenes
        );
        assert_eq!(next.assets[0].aliases, original.aliases);
        let mut f2 = empty_catalogue(&source, &batch);
        f2["assets"] = json!([{"key":original.id, "name":"小王｜白衣（日常）", "type":"CH", "episodes":selected, "sceneRefs":source.scenes.iter().map(|s|json!([s.episode_id,s.id])).collect::<Vec<_>>(), "preparation":{"mode":"REFERENCE"}, "variants":[{"key":original.variants[0].id,"name":"白衣（日常）","episodes":selected,"sceneRefs":source.scenes.iter().map(|s|json!([s.episode_id,s.id])).collect::<Vec<_>>()}]}]);
        let f2_input = input_for("F2", &source, &batch, &basis, &json!({}), 0, 1, false, &[]);
        assert!(apply_catalogue(
            &f2,
            &source,
            &batch,
            &f2_input,
            &basis,
            None,
            &HashSet::new()
        )
        .unwrap_err()
        .contains("基线服装"));
    }
    #[tokio::test]
    #[ignore = "requires DIRECTOR_FOUNDATION_EDIT_REAL_DB, DIRECTOR_FOUNDATION_EDIT_WORKFLOW_ID and DIRECTOR_FOUNDATION_EDIT_TEXT_JOB_ID; readonly frozen F2/Q1 replies, zero live calls or production writes"]
    async fn saved_actual_q1_explicit_base_display_edit_revalidates_without_generation() {
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| panic!("required explicit environment variable: {name}"))
        };
        let connection = rusqlite::Connection::open_with_flags(
            required("DIRECTOR_FOUNDATION_EDIT_REAL_DB"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let workflow_id = required("DIRECTOR_FOUNDATION_EDIT_WORKFLOW_ID");
        let text_job_id = required("DIRECTOR_FOUNDATION_EDIT_TEXT_JOB_ID");
        let workflow: String = connection
            .query_row(
                "SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow'",
                [&workflow_id],
                |r| r.get(0),
            )
            .unwrap();
        let (q1_snapshot, q1_raw): (String, String) = connection.query_row("SELECT j.snapshot,j.result FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE j.id=?1 AND w.workflow_id=?2 AND w.stage='foundation:Q1'", rusqlite::params![text_job_id,workflow_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let f2_raw: String = connection.query_row("SELECT j.result FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE w.workflow_id=?1 AND w.stage='foundation:F2' AND w.state='accepted'", [&workflow_id], |r| r.get(0)).unwrap();
        let workflow: Value = serde_json::from_str(&workflow).unwrap();
        let q1_snapshot: Value = serde_json::from_str(&q1_snapshot).unwrap();
        let q1_reply: Value = serde_json::from_str(&q1_raw).unwrap();
        let source: LockedSource =
            serde_json::from_value(workflow["input"]["source"].clone()).unwrap();
        let original_source = json!(source);
        struct FrozenEditor {
            calls: Mutex<Vec<String>>,
            f2: Value,
            q1: Value,
            q1_input: Value,
            q1_schema: Value,
        }
        #[async_trait]
        impl TextExecutor for FrozenEditor {
            async fn execute(&self, request: TextRequest) -> Result<Value, String> {
                self.calls.lock().unwrap().push(request.stage.clone());
                match request.stage.as_str() {
                    "foundation:F2" => Ok(self.f2.clone()),
                    "foundation:Q1" => {
                        fn first_difference(left: &Value, right: &Value, path: &str) -> String {
                            match (left, right) {
                                (Value::Object(left), Value::Object(right)) => {
                                    for key in left.keys().chain(right.keys()) {
                                        if left.get(key) != right.get(key) {
                                            let next = format!("{path}/{key}");
                                            return match (left.get(key), right.get(key)) {
                                                (Some(l), Some(r)) => first_difference(l, r, &next),
                                                _ => next,
                                            };
                                        }
                                    }
                                }
                                (Value::Array(left), Value::Array(right)) => {
                                    for index in 0..left.len().max(right.len()) {
                                        if left.get(index) != right.get(index) {
                                            let next = format!("{path}/{index}");
                                            return match (left.get(index), right.get(index)) {
                                                (Some(l), Some(r)) => first_difference(l, r, &next),
                                                _ => next,
                                            };
                                        }
                                    }
                                }
                                _ => {}
                            }
                            path.into()
                        }
                        assert!(
                            request.input == self.q1_input,
                            "existing paid Q1 canonical input must remain exact; first difference {} (new sha {}, frozen sha {})",
                            first_difference(&request.input, &self.q1_input, ""),
                            digest(&request.input.to_string()),
                            digest(&self.q1_input.to_string())
                        );
                        assert!(
                            request.schema == self.q1_schema,
                            "existing paid Q1 canonical schema must remain exact; first difference {} (new sha {}, frozen sha {})",
                            first_difference(&request.schema, &self.q1_schema, ""),
                            digest(&request.schema.to_string()),
                            digest(&self.q1_schema.to_string())
                        );
                        Ok(self.q1.clone())
                    }
                    _ => panic!("unexpected role; this fixture cannot call a live executor"),
                }
            }
        }
        let executor = FrozenEditor {
            calls: Mutex::new(vec![]),
            f2: serde_json::from_str(&f2_raw).unwrap(),
            q1: q1_reply.clone(),
            q1_input: q1_snapshot["input"].clone(),
            q1_schema: q1_snapshot["schema"].clone(),
        };
        // Reproduce Engine's foundation.run context, which deliberately exposes
        // production direction rather than machine/provider execution settings.
        let input = &workflow["input"];
        let context = json!({"foundation":input["foundation"],"assets":input["assets"],"selection":input["selection"],"foundationModelInputProfile":input["foundationModelInputProfile"],"foundationPhase":input["foundationPhase"],"confirmedDecisions":input["confirmedDecisions"],"projectId":input["projectId"],"sourceVersion":input["sourceVersion"],"foundationDecisionPolicy":input["foundationDecisionPolicy"],"automaticDecisions":input["automaticDecisions"],"userReviewRequirements":input["userReviewRequirements"],"settings":{"confirmedDirection":input["foundation"]["productionDirection"]}});
        let next = run_foundation_delivery(
            &source,
            &context,
            &executor,
            q1_snapshot["model"].as_str().unwrap(),
            q1_snapshot["reasoning"].as_str().unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            *executor.calls.lock().unwrap(),
            vec!["foundation:F2", "foundation:Q1"]
        );
        assert_eq!(json!(source), original_source);
        assert!(next.issues.iter().all(|issue| !issue.blocking));
        let patch = &q1_reply["changes"]["assets"][0];
        let asset = next
            .assets
            .iter()
            .find(|a| Some(a.id.as_str()) == patch["key"].as_str())
            .unwrap();
        assert_eq!(json!(asset.name), patch["name"]);
        assert_eq!(asset.metadata["preparation"]["mode"], "REFERENCE");
        for patch in patch["variants"].as_array().unwrap() {
            let variant = asset
                .variants
                .iter()
                .find(|v| Some(v.id.as_str()) == patch["key"].as_str())
                .unwrap();
            assert_eq!(json!(variant.name), patch["name"]);
            assert_eq!(variant.metadata["preparation"]["mode"], "REFERENCE");
        }
        let adopted = foundation_auto_applied_decisions(&next);
        assert!(!adopted.is_empty());
        assert!(adopted
            .iter()
            .all(|r| r["sourceSha256"] == source.sha256 && r["status"] == "APPLIED"));
        validate_source(&source).unwrap();
        validate_foundation(&source, &next).unwrap();
    }
    #[test]
    fn malformed_coverage_and_unprovided_evidence_are_rejected() {
        let s = source(2);
        let batches = plan_batches(
            &s,
            &s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
        let batch = &batches[0];
        let p = empty_foundation(&s);
        let input = input_for("F2", &s, batch, &p, &json!({}), 0, 1, false, &[]);
        let mut wire = empty_catalogue(&s, batch);
        wire["coverage"] = json!([batch.scope[0]]);
        assert!(apply_catalogue(&wire, &s, batch, &input, &p, None, &HashSet::new()).is_err());
        assert!(provided_refs(&["not-supplied".into()], &allowed_refs(&input)).is_err());
    }
    #[test]
    fn historical_neighbours_and_character_claims_are_evidence_only() {
        let s=prepare_source("第1集\n场景：大厅\n△小王受伤。\n△他随后痊愈。\n第2集\n场景：大厅\n小王：我没有受过伤。\n").unwrap();
        let batch = Batch {
            scope: vec![s.episodes[1].id.clone()],
            pieces: whole_rows(&s, &[s.episodes[1].id.clone()]),
            fragment: None,
        };
        let evidence = evidence(&s, &batch, &empty_foundation(&s));
        assert_eq!(evidence["currentApplicability"], "UNKNOWN");
        assert!(evidence["historicalEvidenceCandidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[3].as_str().unwrap().contains("痊愈") && r[5] == "SOURCE_NEIGHBOUR"));
    }
    #[test]
    fn same_names_do_not_merge_new_source_anchored_identities() {
        let s = source(2);
        let batch = plan_batches(
            &s,
            &s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        )
        .unwrap()
        .remove(0);
        let p = empty_foundation(&s);
        let input = input_for("F2", &s, &batch, &p, &json!({}), 0, 1, false, &[]);
        let rows = s
            .rows
            .iter()
            .filter(|r| r.kind == "body" && r.text.contains("白衣"))
            .collect::<Vec<_>>();
        let mut wire = empty_catalogue(&s, &batch);
        wire["assets"]=json!(rows.iter().map(|r|json!({"name":"小王｜白衣","type":"CH","episodes":[r.episode_id],"sceneRefs":[[r.episode_id,r.scene_id]],"sourceRefs":[r.id],"preparation":{"mode":"REFERENCE"}})).collect::<Vec<_>>());
        let next = apply_catalogue(&wire, &s, &batch, &input, &p, None, &HashSet::new()).unwrap();
        assert_eq!(next.assets.len(), 2);
        assert_ne!(next.assets[0].id, next.assets[1].id);
        assert_eq!(
            next.assets[0].variants[0].metadata["preparation"]["mode"],
            "REFERENCE"
        );
    }

    #[test]
    fn catalogue_narrative_role_is_independent_of_human_image_template() {
        let source = source(1);
        let scope = source.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        let batch = plan_batches(&source, &scope).unwrap().remove(0);
        let prior = empty_foundation(&source);
        let input = input_for("F2", &source, &batch, &prior, &json!({}), 0, 1, false, &[]);
        for kind in ["CH", "BEAST", "GROUP", "LOC", "PROP"] {
            for role in ["主角", "", "  "] {
                let mut raw = empty_catalogue(&source, &batch);
                raw["assets"] = json!([{"name":"测试对象", "type":kind, "episodes":scope,
                    "sceneRefs":source.scenes.iter().map(|s|json!([s.episode_id,s.id])).collect::<Vec<_>>(),
                    "role":role,"preparation":{"mode":"REFERENCE"}}]);
                let next = apply_catalogue(&raw, &source, &batch, &input, &prior, None, &HashSet::new()).unwrap();
                assert_eq!(next.assets[0].kind, kind);
                let expected = if matches!(kind, "CH" | "BEAST") && !role.trim().is_empty() { Some(json!(role)) } else { None };
                assert_eq!(next.assets[0].metadata.get("role"), expected.as_ref());
                assert!(!flatten_foundation_assets(&next).is_empty());
                if matches!(kind, "CH" | "BEAST") {
                    raw["assets"][0]["role"] = json!("invalid-role");
                    assert!(apply_catalogue(&raw, &source, &batch, &input, &prior, None, &HashSet::new()).is_err());
                }
            }
        }
    }

    #[test]
    fn f2_existing_base_is_updated_once_with_its_own_scope() {
        let (source, prior) = two_episode_catalogue();
        let all = source.episodes.iter().map(|e|e.id.clone()).collect::<Vec<_>>();
        let batch = plan_batches(&source, &all).unwrap().remove(0);
        let input = input_for("F2", &source, &batch, &prior, &json!({}), 0, 1, false, &[]);
        let base = &prior.assets[0].variants[0];
        let first = &source.scenes[0];
        let mut raw = empty_catalogue(&source, &batch);
        raw["assets"] = json!([{"key":prior.assets[0].id,"name":"小王｜白衣","type":"CH","episodes":all,
            "sceneRefs":source.scenes.iter().map(|s|json!([s.episode_id,s.id])).collect::<Vec<_>>(),"preparation":{"mode":"REFERENCE"},
            "variants":[{"key":base.id,"name":base.name,"episodes":[first.episode_id],"sceneRefs":[[first.episode_id,first.id]],"preparation":{"mode":"REFERENCE"}}]}]);
        let next = apply_catalogue(&raw,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap();
        let retained = &next.assets[0].variants[0];
        assert_eq!(next.assets[0].variants.len(), prior.assets[0].variants.len());
        assert_eq!(retained.id, base.id);
        assert_eq!(retained.name, base.name);
        assert_eq!(retained.scope.scenes.len(), 1);
        assert_eq!(retained.scope.scenes[0].episode_id, first.episode_id);
        assert_eq!(retained.metadata["usageEpisodes"], json!([first.episode_id]));
        raw["assets"][0]["variants"][0]["name"] = json!("隐式改名");
        assert!(apply_catalogue(&raw,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap_err().contains("隐式改名"));
        raw["assets"][0]["variants"][0]["name"] = json!(base.name);
        let duplicate = raw["assets"][0]["variants"][0].clone();
        raw["assets"][0]["variants"].as_array_mut().unwrap().push(duplicate);
        assert!(apply_catalogue(&raw,&source,&batch,&input,&prior,None,&HashSet::new()).unwrap_err().contains("子形象重复"));
    }

    #[test]
    #[ignore = "explicit readonly DB/workflow/text job via DIRECTOR_ROLE_REAL_DB, DIRECTOR_ROLE_WORKFLOW, DIRECTOR_ROLE_TEXT; no CLI or project writes"]
    fn saved_actual_nonhuman_protagonist_catalogue_revalidates_without_generation() {
        let env = |key| std::env::var(key).expect(key);
        let connection = rusqlite::Connection::open_with_flags(env("DIRECTOR_ROLE_REAL_DB"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let workflow = env("DIRECTOR_ROLE_WORKFLOW");
        let job = env("DIRECTOR_ROLE_TEXT");
        let parent: String = connection.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow'", [&workflow], |r| r.get(0)).unwrap();
        let (snapshot, raw): (String, String) = connection.query_row("SELECT j.snapshot,j.result FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE j.id=?1 AND w.workflow_id=?2 AND w.stage='foundation:F2'", rusqlite::params![job,workflow], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        let parent: Value = serde_json::from_str(&parent).unwrap();
        let source: LockedSource = serde_json::from_value(parent["input"]["source"].clone()).unwrap();
        let snapshot: Value = serde_json::from_str(&snapshot).unwrap();
        let raw: Value = serde_json::from_str(&raw).unwrap();
        let input = &snapshot["input"];
        let scope: Vec<String> = serde_json::from_value(input["scope"].clone()).unwrap();
        let batch = Batch { pieces: whole_rows(&source, &scope), scope, fragment: None };
        let prior = existing_foundation(&source, &parent["input"]).unwrap();
        let next = apply_catalogue(&raw, &source, &batch, input, &prior, None, &HashSet::new()).unwrap();
        assert!(next.assets.len() >= raw["assets"].as_array().unwrap().len());
        assert!(next.assets.iter().any(|a| a.kind == "BEAST" && a.metadata.get("role") == Some(&json!("主角"))));
        println!("Saved catalogue accepted: {} assets, {} stage-two rows, zero model calls", next.assets.len(), flatten_foundation_assets(&next).len());
    }

    fn two_episode_catalogue() -> (LockedSource, FoundationProposal) {
        let s = source(2);
        let selected = s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        let batch = plan_batches(&s, &selected).unwrap().remove(0);
        let prior = empty_foundation(&s);
        let refs = s
            .rows
            .iter()
            .filter(|r| r.kind == "body" && r.text.contains("白衣"))
            .map(|r| r.id.clone())
            .collect::<Vec<_>>();
        let pairs = s
            .rows
            .iter()
            .filter(|r| refs.contains(&r.id))
            .map(|r| json!([r.episode_id, r.scene_id]))
            .collect::<Vec<_>>();
        let wire = json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":s.sha256,"coverage":selected,"assets":[{"name":"小王｜白衣","type":"CH","episodes":selected,"sceneRefs":pairs,"sourceRefs":refs,"role":"主角","relationNote":"独立人物，不因同名合并","preparation":{"mode":"REFERENCE"},"variants":[{"name":"伤态","episodes":selected,"sceneRefs":pairs,"sourceRefs":refs,"note":"必要持续状态","preparation":{"mode":"INLINE"}},{"name":"旧伤态","episodes":selected,"sceneRefs":pairs,"sourceRefs":refs,"preparation":{"mode":"DERIVE"}}]}]});
        let input = input_for("F2", &s, &batch, &prior, &json!({}), 0, 1, false, &[]);
        let next =
            apply_catalogue(&wire, &s, &batch, &input, &prior, None, &HashSet::new()).unwrap();
        (s, next)
    }

    fn local_parent_catalogue() -> (LockedSource, Batch, FoundationProposal, Value, Value) {
        let s = source(2);
        let scope: Vec<_> = s.episodes.iter().map(|e| e.id.clone()).collect();
        let batch = Batch { pieces: whole_rows(&s, &scope), scope, fragment: None };
        let prior = empty_foundation(&s);
        let pairs: Vec<_> = s.scenes.iter().map(|scene| json!([scene.episode_id, scene.id])).collect();
        let mut raw = empty_catalogue(&s, &batch);
        raw["assets"] = json!([{"name":"小王｜默认","type":"CH","episodes":batch.scope,"sceneRefs":pairs,
            "preparation":{"mode":"REFERENCE"},"variants":[
                {"localKey":"injured","name":"西装受伤","episodes":batch.scope,"sceneRefs":pairs,"preparation":{"mode":"DERIVE","fromKey":"suit"}},
                {"localKey":"suit","name":"西装","episodes":batch.scope,"sceneRefs":pairs,"preparation":{"mode":"DERIVE"}}
            ]}]);
        let input = input_for("F2", &s, &batch, &prior, &json!({}), 0, 1, false, &[]);
        (s, batch, prior, input, raw)
    }

    #[test]
    fn f2_local_parent_forward_references_keep_immediate_parent_and_persist_only_stable_keys() {
        let (s, batch, prior, input, raw) = local_parent_catalogue();
        assert_eq!(asset_schema(false)["properties"]["variants"]["items"]["properties"]["localKey"]["type"], "string");
        let transport_context = json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        let transport_input = input_for("F2", &s, &batch, &prior, &transport_context, 0, 1, false, &[]);
        let transport = super::super::foundation_wire::compile("foundation:F2", &transport_input, &delivery_schema("F2", &transport_input)).unwrap();
        let mut restored = raw.clone();
        transport.restore(&mut restored).unwrap();
        assert_eq!(restored, raw, "local aliases must survive transport without becoming persistent IDs");
        let next = apply_catalogue(&raw, &s, &batch, &input, &prior, None, &HashSet::new()).unwrap();
        let asset = &next.assets[0];
        let suit = asset.variants.iter().find(|v| v.name == "西装").unwrap();
        let injured = asset.variants.iter().find(|v| v.name == "西装受伤").unwrap();
        assert_eq!(suit.source_variant_id.as_deref(), Some(base_id(asset).unwrap().as_str()));
        assert_eq!(injured.source_variant_id.as_deref(), Some(suit.id.as_str()));
        assert_eq!(injured.metadata["preparation"]["fromKey"], suit.id);
        let persisted = serde_json::to_value(&next).unwrap();
        assert!(!persisted.to_string().contains("localKey"));
        let round_trip: FoundationProposal = serde_json::from_value(persisted).unwrap();
        assert_eq!(serde_json::to_value(&round_trip.assets[0].variants).unwrap(), serde_json::to_value(&next.assets[0].variants).unwrap());
        let flat = flatten_foundation_assets(&round_trip);
        assert_eq!(flat.iter().find(|a| a["variantId"] == injured.id).unwrap()["sourceVariantId"], suit.id);
    }

    #[test]
    fn f2_local_parent_eleven_level_chain_is_order_independent() {
        let (s, batch, prior, input, mut raw) = local_parent_catalogue();
        let template = raw["assets"][0]["variants"][0].clone();
        raw["assets"][0]["variants"] = json!((1..=10).rev().map(|n| {
            let mut v = template.clone();
            v["localKey"] = json!(format!("form{n}"));
            v["name"] = json!(format!("形态{n}"));
            v["preparation"] = if n == 1 { json!({"mode":"DERIVE"}) }
                else { json!({"mode":"DERIVE","fromKey":format!("form{}",n-1)}) };
            v
        }).collect::<Vec<_>>());
        let next = apply_catalogue(&raw, &s, &batch, &input, &prior, None, &HashSet::new()).unwrap();
        let asset = &next.assets[0];
        assert_eq!(asset.variants.len(), 11);
        for n in 1..=10 {
            let v = asset.variants.iter().find(|v| v.name == format!("形态{n}")).unwrap();
            let expected = if n == 1 { base_id(asset).unwrap() } else {
                asset.variants.iter().find(|v| v.name == format!("形态{}", n-1)).unwrap().id.clone()
            };
            assert_eq!(v.source_variant_id.as_deref(), Some(expected.as_str()));
        }
    }

    #[test]
    fn f2_local_parent_nonhuman_evolution_keeps_each_previous_form_and_an_independent_peer() {
        let s = prepare_source(concat!(
            "第1集\n场景：山洞\n△青鳞幼龙爬出洞口。\n",
            "第2集\n场景：山洞\n△同一条青鳞幼龙展开新生翼膜，青色鳞片仍在。\n",
            "第3集\n场景：山洞\n△刚才那条龙再长出赤角，翼膜仍在。\n",
            "△另一条同物种幼龙经过，不是此前进化的那一条。\n"
        ))
        .unwrap();
        let scope: Vec<_> = s.episodes.iter().map(|e| e.id.clone()).collect();
        let scenes: Vec<_> = s.scenes.iter().map(|v| json!([v.episode_id, v.id])).collect();
        let batch = Batch { pieces: whole_rows(&s, &scope), scope, fragment: None };
        let prior = empty_foundation(&s);
        let mut raw = empty_catalogue(&s, &batch);
        raw["assets"] = json!([
            {"name": "青鳞龙", "type": "BEAST", "role": "主角",
             "episodes": [batch.scope[0]], "sceneRefs": [scenes[0]],
             "preparation": {"mode": "REFERENCE"}, "variants": [
                {"localKey": "horned", "name": "赤角翼膜形态",
                 "episodes": [batch.scope[2]], "sceneRefs": [scenes[2]],
                 "preparation": {"mode": "DERIVE", "fromKey": "winged"}},
                {"localKey": "winged", "name": "翼膜形态",
                 "episodes": [batch.scope[1]], "sceneRefs": [scenes[1]],
                 "preparation": {"mode": "DERIVE"}}
             ]},
            {"name": "另一条青鳞幼龙", "type": "BEAST",
             "episodes": [batch.scope[2]], "sceneRefs": [scenes[2]],
             "preparation": {"mode": "REFERENCE"}}
        ]);
        let context = json!({"foundationModelInputProfile": super::super::foundation_wire::PROFILE});
        let input = input_for("F2", &s, &batch, &prior, &context, 0, 1, false, &[]);
        let transport = super::super::foundation_wire::compile(
            "foundation:F2", &input, &delivery_schema("F2", &input)
        ).unwrap();
        transport.restore(&mut raw).unwrap();
        let next = apply_catalogue(
            &raw, &s, &batch, &input, &prior, None, &HashSet::new()
        ).unwrap();
        let dragon = next.assets.iter().find(|a| a.name == "青鳞龙").unwrap();
        let winged = dragon.variants.iter().find(|v| v.name == "翼膜形态").unwrap();
        let horned = dragon.variants.iter().find(|v| v.name == "赤角翼膜形态").unwrap();
        assert_eq!(dragon.kind, "BEAST");
        assert_eq!(dragon.metadata["role"], "主角");
        assert_eq!(dragon.variants.len(), 3);
        assert_eq!(winged.source_variant_id, Some(base_id(dragon).unwrap()));
        assert_eq!(horned.source_variant_id, Some(winged.id.clone()));
        assert_ne!(horned.source_variant_id, Some(base_id(dragon).unwrap()));
        let flattened = super::super::flatten_foundation_assets(&next);
        let horned_row = flattened.iter().find(|v| v["variantId"] == horned.id).unwrap();
        let winged_row = flattened.iter().find(|v| v["variantId"] == winged.id).unwrap();
        assert_eq!(
            super::super::assets::resolve_asset_dependency(&flattened, horned_row).unwrap(),
            Some(winged_row["id"].as_str().unwrap().to_owned())
        );
        let peer = next.assets.iter().find(|a| a.name == "另一条青鳞幼龙").unwrap();
        assert_eq!(peer.kind, "BEAST");
        assert_eq!(peer.variants[0].source_variant_id, None);
        let peer_row = flattened.iter().find(|v| v["identityId"] == peer.id).unwrap();
        assert_eq!(
            super::super::assets::resolve_asset_dependency(&flattened, peer_row).unwrap(),
            None
        );
    }

    #[test]
    fn f2_local_parent_independent_previous_body_survives_transport_and_flattening() {
        let (s, batch, prior, _, mut raw) = local_parent_catalogue();
        let mut independent = raw["assets"][0]["variants"][0].clone();
        independent["localKey"] = json!("previous-body");
        independent["name"] = json!("前世异体");
        independent["preparation"] = json!({
            "mode": "REFERENCE",
            "note": "原文明示为不同实际身体，保留独立制作基线"
        });
        raw["assets"][0]["variants"]
            .as_array_mut()
            .unwrap()
            .push(independent);
        let context = json!({
            "foundationModelInputProfile": super::super::foundation_wire::PROFILE
        });
        let input = input_for("F2", &s, &batch, &prior, &context, 0, 1, false, &[]);
        let transport = super::super::foundation_wire::compile(
            "foundation:F2",
            &input,
            &delivery_schema("F2", &input),
        )
        .unwrap();
        let mut restored = raw.clone();
        transport.restore(&mut restored).unwrap();
        assert_eq!(restored, raw);
        let next = apply_catalogue(
            &restored,
            &s,
            &batch,
            &input,
            &prior,
            None,
            &HashSet::new(),
        )
        .unwrap();
        let asset = &next.assets[0];
        let past = asset.variants.iter().find(|v| v.name == "前世异体").unwrap();
        assert_eq!(past.source_variant_id, None);
        assert_eq!(past.metadata["preparation"]["mode"], "REFERENCE");
        let flattened = super::super::flatten_foundation_assets(&next);
        let past_row = flattened.iter().find(|v| v["variantId"] == past.id).unwrap();
        assert_eq!(past_row["sourceVariantId"], Value::Null);
        assert_eq!(
            super::super::assets::resolve_asset_dependency(&flattened, past_row).unwrap(),
            None
        );
        for child_name in ["西装", "西装受伤"] {
            let child = asset.variants.iter().find(|v| v.name == child_name).unwrap();
            let row = flattened.iter().find(|v| v["variantId"] == child.id).unwrap();
            let parent = flattened
                .iter()
                .find(|v| Some(v["variantId"].as_str().unwrap()) == child.source_variant_id.as_deref())
                .unwrap();
            assert_eq!(
                super::super::assets::resolve_asset_dependency(&flattened, row).unwrap(),
                Some(parent["id"].as_str().unwrap().to_owned())
            );
        }
    }

    #[test]
    fn f2_local_parent_missing_source_never_guesses_from_identity_or_another_asset() {
        let (s, batch, prior, input, raw) = local_parent_catalogue();
        let mut no_base_source = raw.clone();
        no_base_source["assets"][0]["preparation"] = json!({"mode": "DERIVE"});
        assert!(apply_catalogue(
            &no_base_source, &s, &batch, &input, &prior, None, &HashSet::new()
        )
        .unwrap_err()
        .contains("缺少真实来源"));

        let mut independent = raw;
        independent["assets"][0]["variants"][0]["preparation"] = json!({"mode": "REFERENCE"});
        let registered = apply_catalogue(
            &independent, &s, &batch, &input, &prior, None, &HashSet::new()
        )
        .unwrap();
        let asset = &registered.assets[0];
        let parentless = asset.variants.iter().find(|v| v.name == "西装受伤").unwrap();
        assert_eq!(parentless.source_variant_id, None);
        let mut edit = empty_edit(&s, &batch);
        edit["changes"] = json!({"assets": [{
            "key": asset.id,
            "variants": [{"key": parentless.id, "preparation": {"mode": "DERIVE"}}]
        }]});
        let q1 = input_for("Q1", &s, &batch, &registered, &json!({}), 0, 1, true, &[]);
        assert!(apply_optimization(&edit, &s, &batch, &q1, &registered, None)
            .unwrap_err()
            .contains("缺少真实来源"));
        edit["changes"]["assets"][0]["variants"][0]["preparation"]["fromKey"] =
            json!(base_id(asset).unwrap());
        let repaired = apply_optimization(&edit, &s, &batch, &q1, &registered, None).unwrap().0;
        assert_eq!(
            repaired.assets[0].variants.iter().find(|v| v.id == parentless.id).unwrap().source_variant_id,
            Some(base_id(asset).unwrap())
        );
    }

    #[test]
    fn f2_local_parent_rejects_duplicate_unknown_self_cycle_and_registered_key_collision() {
        let (s, batch, prior, input, raw) = local_parent_catalogue();
        let run = |raw: &Value| apply_catalogue(raw, &s, &batch, &input, &prior, None, &HashSet::new());
        let mut duplicate = raw.clone();
        duplicate["assets"][0]["variants"][0]["localKey"] = json!("suit");
        assert!(run(&duplicate).unwrap_err().contains("localKey重复"));
        let mut unknown = raw.clone();
        unknown["assets"][0]["variants"][0]["preparation"]["fromKey"] = json!("missing");
        assert!(run(&unknown).is_err());
        let mut own = raw.clone();
        own["assets"][0]["variants"][0]["preparation"]["fromKey"] = json!("injured");
        assert!(run(&own).unwrap_err().contains("自身"));
        let mut cycle = raw.clone();
        cycle["assets"][0]["variants"][1]["preparation"]["fromKey"] = json!("injured");
        assert!(run(&cycle).unwrap_err().contains("循环"));
        let mut invalid_mode = raw.clone();
        invalid_mode["assets"][0]["variants"][0]["preparation"]["mode"] = json!("REFERENCE");
        assert!(run(&invalid_mode).is_err());
        let mut reserved = raw.clone();
        reserved["assets"][0]["variants"][0]["localKey"] = json!("v1");
        assert!(run(&reserved).unwrap_err().contains("运输短键"));
        let existing = run(&raw).unwrap();
        let mut collision = raw;
        collision["assets"][0]["variants"][0]["localKey"] = json!(existing.assets[0].variants[0].id);
        assert!(apply_catalogue(&collision, &s, &batch, &input, &existing, None, &HashSet::new()).unwrap_err().contains("稳定键冲突"));
    }

    #[test]
    fn f2_local_parent_aliases_do_not_escape_the_owner_or_return_package() {
        let (s, batch, prior, input, mut raw) = local_parent_catalogue();
        let mut other = raw["assets"][0].clone();
        other["name"] = json!("小李｜默认");
        other["variants"].as_array_mut().unwrap().pop();
        raw["assets"].as_array_mut().unwrap().push(other);
        assert!(apply_catalogue(&raw, &s, &batch, &input, &prior, None, &HashSet::new()).is_err());
        raw["assets"].as_array_mut().unwrap().pop();
        let registered = apply_catalogue(&raw, &s, &batch, &input, &prior, None, &HashSet::new()).unwrap();
        let mut next = raw.clone();
        next["assets"][0]["key"] = json!(registered.assets[0].id);
        next["assets"][0]["variants"].as_array_mut().unwrap().pop();
        next["assets"][0]["variants"][0]["name"] = json!("新的伤态");
        assert!(apply_catalogue(&next, &s, &batch, &input, &registered, None, &HashSet::new()).is_err());
    }

    fn preparation_conflict_fixture() -> (LockedSource, FoundationProposal, Batch) {
        let s = source(2);
        let selected = s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        let all = Batch {
            scope: selected.clone(),
            pieces: whole_rows(&s, &selected),
            fragment: None,
        };
        let prior = empty_foundation(&s);
        let pairs = s
            .scenes
            .iter()
            .map(|scene| json!([scene.episode_id, scene.id]))
            .collect::<Vec<_>>();
        let asset = json!({"name":"小王｜默认","type":"CH","episodes":selected,"sceneRefs":pairs,
        "preparation":{"mode":"REFERENCE","note":"已登记人物基线"},"variants":[
            {"name":"小王｜Q版","episodes":selected,"sceneRefs":pairs,"preparation":{"mode":"REFERENCE","note":"已登记独立制作"}},
            {"name":"小王｜白衣","episodes":selected,"sceneRefs":pairs,"preparation":{"mode":"DERIVE","note":"已登记基线派生"}}
        ]});
        let mut raw = empty_catalogue(&s, &all);
        raw["assets"] = json!([asset, asset]); // Equal display names are distinct identities.
        let input = input_for("F2", &s, &all, &prior, &json!({}), 0, 1, false, &[]);
        let p = apply_catalogue(&raw, &s, &all, &input, &prior, None, &HashSet::new()).unwrap();
        let first = vec![s.episodes[0].id.clone()];
        let batch = Batch {
            scope: first.clone(),
            pieces: whole_rows(&s, &first),
            fragment: None,
        };
        (s, p, batch)
    }

    fn preparation_conflict_catalogue(
        s: &LockedSource,
        p: &FoundationProposal,
        batch: &Batch,
        prep: Value,
    ) -> Value {
        let asset = &p.assets[0];
        let pairs = s
            .scenes
            .iter()
            .filter(|scene| batch.scope.contains(&scene.episode_id))
            .map(|scene| json!([scene.episode_id, scene.id]))
            .collect::<Vec<_>>();
        let mut wire = empty_catalogue(s, batch);
        wire["assets"] = json!([{"key":asset.id,"name":"小王｜默认","type":"CH","episodes":batch.scope,
        "sceneRefs":pairs,"preparation":asset.metadata["preparation"],"variants":[{
            "key":asset.variants[1].id,"name":asset.variants[1].name,"episodes":batch.scope,
            "sceneRefs":pairs,"preparation":prep
        }]}]);
        wire
    }

    #[test]
    fn f2_preparation_conflict_retains_reference_and_every_external_use_for_both_profiles() {
        let (s, p, batch) = preparation_conflict_fixture();
        for context in [
            json!({}),
            json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE}),
        ] {
            let input = input_for("F2", &s, &batch, &p, &context, 0, 1, false, &[]);
            let wire = preparation_conflict_catalogue(
                &s,
                &p,
                &batch,
                json!({"mode":"DERIVE","fromKey":p.assets[0].variants[0].id,"note":"本批建议基线派生"}),
            );
            let next =
                apply_catalogue(&wire, &s, &batch, &input, &p, None, &HashSet::new()).unwrap();
            let old = &p.assets[0].variants[1];
            let new = &next.assets[0].variants[1];
            assert_eq!(new.source_variant_id, None);
            assert_eq!(new.metadata["preparation"], old.metadata["preparation"]);
            assert_eq!(new.scope.scenes, old.scope.scenes);
            assert_eq!(new.source_refs, old.source_refs);
            assert_eq!(new.metadata["usageEpisodes"], old.metadata["usageEpisodes"]);
            assert_eq!(
                serde_json::to_value(&next.assets[1]).unwrap(),
                serde_json::to_value(&p.assets[1]).unwrap()
            );
            let issue = next
                .issues
                .iter()
                .find(|i| {
                    i.details
                        .as_ref()
                        .is_some_and(|d| d["kind"] == "PREPARATION_CONFLICT")
                })
                .unwrap();
            assert!(!issue.blocking);
            assert!(issue.coordinate.starts_with("preparation:"));
            assert_eq!(issue.episode_ids, batch.scope);
            assert!(issue
                .source_refs
                .iter()
                .all(|id| batch.row_ids().contains(id.as_str())));
            let details = issue.details.as_ref().unwrap();
            assert_eq!(
                details["previous"]["preparation"],
                old.metadata["preparation"]
            );
            assert_eq!(details["previous"]["sourceVariantId"], Value::Null);
            assert_eq!(
                details["proposed"]["sourceVariantId"],
                p.assets[0].variants[0].id
            );
            assert_eq!(details["proposed"]["preparation"]["mode"], "DERIVE");
            assert_eq!(
                details["previousSceneRefs"],
                serde_json::to_value(&old.scope.scenes).unwrap()
            );
            assert_eq!(details["currentSceneRefs"].as_array().unwrap().len(), 1);
        }
    }

    #[test]
    fn f2_preparation_conflict_q1_can_keep_or_explicitly_change_the_registered_basis() {
        let (s, p, batch) = preparation_conflict_fixture();
        let f2 = input_for("F2", &s, &batch, &p, &json!({}), 0, 1, false, &[]);
        let raw = preparation_conflict_catalogue(
            &s,
            &p,
            &batch,
            json!({"mode":"DERIVE","fromKey":p.assets[0].variants[0].id}),
        );
        let staged = apply_catalogue(&raw, &s, &batch, &f2, &p, None, &HashSet::new()).unwrap();
        let issue = staged
            .issues
            .iter()
            .find(|i| i.coordinate.starts_with("preparation:"))
            .unwrap();
        let q1 = input_for("Q1", &s, &batch, &staged, &json!({}), 0, 1, true, &[]);
        let mut keep = empty_edit(&s, &batch);
        keep["resolvedIssues"] = json!([issue.coordinate]);
        let (retained, _) = apply_optimization(&keep, &s, &batch, &q1, &staged, None).unwrap();
        assert_eq!(retained.assets[0].variants[1].source_variant_id, None);
        assert_eq!(
            retained.assets[0].variants[1].metadata["preparation"],
            p.assets[0].variants[1].metadata["preparation"]
        );
        assert!(retained
            .issues
            .iter()
            .all(|i| i.coordinate != issue.coordinate));
        let mut explicit_keep = keep.clone();
        explicit_keep["changes"] = json!({"assets":[{"key":p.assets[0].id,"variants":[{"key":p.assets[0].variants[1].id,"preparation":p.assets[0].variants[1].metadata["preparation"]}]}]});
        assert!(
            apply_optimization(&explicit_keep, &s, &batch, &q1, &staged, None)
                .unwrap()
                .0
                .issues
                .iter()
                .all(|i| i.coordinate != issue.coordinate)
        );
        let mut change = keep;
        change["changes"] = json!({"assets":[{"key":p.assets[0].id,"variants":[{"key":p.assets[0].variants[1].id,"preparation":{"mode":"DERIVE","fromKey":p.assets[0].variants[0].id,"note":"Q1明确采纳制作建议"}}]}]});
        let (changed, _) = apply_optimization(&change, &s, &batch, &q1, &staged, None).unwrap();
        assert_eq!(
            changed.assets[0].variants[1].source_variant_id,
            Some(p.assets[0].variants[0].id.clone())
        );
        assert!(changed
            .issues
            .iter()
            .all(|i| i.coordinate != issue.coordinate));
        assert_eq!(
            changed.assets[0].variants[1].scope.scenes,
            p.assets[0].variants[1].scope.scenes
        );
    }

    #[test]
    fn f2_preparation_conflict_swap_parent_and_mode_only_changes_are_q1_suggestions() {
        let (s, p, batch) = preparation_conflict_fixture();
        let input = input_for("F2", &s, &batch, &p, &json!({}), 0, 1, false, &[]);
        let mut raw = preparation_conflict_catalogue(
            &s,
            &p,
            &batch,
            json!({"mode":"DERIVE","fromKey":p.assets[0].variants[1].id}),
        );
        raw["assets"][0]["variants"][0]["key"] = json!(p.assets[0].variants[2].id);
        raw["assets"][0]["variants"][0]["name"] = json!(p.assets[0].variants[2].name);
        let next = apply_catalogue(&raw, &s, &batch, &input, &p, None, &HashSet::new()).unwrap();
        assert_eq!(
            next.assets[0].variants[2].source_variant_id,
            p.assets[0].variants[2].source_variant_id
        );
        assert_eq!(
            next.assets[0].variants[2].metadata["preparation"],
            p.assets[0].variants[2].metadata["preparation"]
        );
        let issue = next
            .issues
            .iter()
            .find(|i| i.coordinate.starts_with("preparation:"))
            .unwrap();
        assert_eq!(
            issue.details.as_ref().unwrap()["proposed"]["sourceVariantId"],
            p.assets[0].variants[1].id
        );
        let mut mode_only = preparation_conflict_catalogue(&s, &p, &batch, json!({"mode":"AUDIO"}));
        let next =
            apply_catalogue(&mode_only, &s, &batch, &input, &p, None, &HashSet::new()).unwrap();
        assert_eq!(
            next.assets[0].variants[1].metadata["preparation"],
            p.assets[0].variants[1].metadata["preparation"]
        );
        mode_only["assets"][0]["preparation"] = json!({"mode":"AUDIO"});
        let next =
            apply_catalogue(&mode_only, &s, &batch, &input, &p, None, &HashSet::new()).unwrap();
        assert_eq!(
            next.assets[0].metadata["preparation"],
            p.assets[0].metadata["preparation"]
        );
        assert_eq!(
            next.assets[0].variants[0].metadata["preparation"],
            p.assets[0].variants[0].metadata["preparation"]
        );
    }

    #[test]
    fn f2_preparation_conflict_does_not_hide_invalid_graph_identity_or_required_fields() {
        let (s, p, batch) = preparation_conflict_fixture();
        let input = input_for("F2", &s, &batch, &p, &json!({}), 0, 1, false, &[]);
        let run = |raw: &Value| apply_catalogue(raw, &s, &batch, &input, &p, None, &HashSet::new());
        let valid = preparation_conflict_catalogue(
            &s,
            &p,
            &batch,
            json!({"mode":"DERIVE","fromKey":p.assets[0].variants[0].id}),
        );
        let mut unknown = valid.clone();
        unknown["assets"][0]["variants"][0]["preparation"]["fromKey"] = json!("missing-parent");
        assert!(run(&unknown).is_err());
        let mut self_link = valid.clone();
        self_link["assets"][0]["variants"][0]["preparation"]["fromKey"] =
            json!(p.assets[0].variants[1].id);
        assert!(run(&self_link).unwrap_err().contains("自身"));
        let mut cycle = valid.clone();
        cycle["assets"][0]["variants"][0]["preparation"]["fromKey"] =
            json!(p.assets[0].variants[2].id);
        let mut second = cycle["assets"][0]["variants"][0].clone();
        second["key"] = json!(p.assets[0].variants[2].id);
        second["name"] = json!(p.assets[0].variants[2].name);
        second["preparation"]["fromKey"] = json!(p.assets[0].variants[1].id);
        cycle["assets"][0]["variants"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(run(&cycle).unwrap_err().contains("循环"));
        let mut wrong_identity = valid.clone();
        wrong_identity["assets"][0]["name"] = json!("另一人｜默认");
        assert!(run(&wrong_identity).unwrap_err().contains("身份"));
        let mut wrong_type = valid.clone();
        wrong_type["assets"][0]["type"] = json!("PROP");
        assert!(run(&wrong_type).unwrap_err().contains("身份"));
        let mut missing = valid.clone();
        missing["assets"][0]["variants"][0]
            .as_object_mut()
            .unwrap()
            .remove("sceneRefs");
        assert!(run(&missing).unwrap_err().contains("必要目录字段"));
        let mut bad_scene = valid;
        bad_scene["assets"][0]["variants"][0]["sceneRefs"] =
            json!([[batch.scope[0], "unknown-scene"]]);
        assert!(run(&bad_scene).is_err());
    }

    #[test]
    fn f2_preparation_conflict_new_legal_variant_and_equivalent_parent_are_not_conflicts() {
        let (s, p, batch) = preparation_conflict_fixture();
        let input = input_for("F2", &s, &batch, &p, &json!({}), 0, 1, false, &[]);
        let mut new_child = preparation_conflict_catalogue(
            &s,
            &p,
            &batch,
            json!({"mode":"DERIVE","fromKey":p.assets[0].variants[0].id}),
        );
        new_child["assets"][0]["variants"][0]
            .as_object_mut()
            .unwrap()
            .remove("key");
        new_child["assets"][0]["variants"][0]["name"] = json!("新伤态");
        let next =
            apply_catalogue(&new_child, &s, &batch, &input, &p, None, &HashSet::new()).unwrap();
        let child = next.assets[0]
            .variants
            .iter()
            .find(|v| v.name == "新伤态")
            .unwrap();
        assert_eq!(
            child.source_variant_id,
            Some(p.assets[0].variants[0].id.clone())
        );
        assert!(next
            .issues
            .iter()
            .all(|i| !i.coordinate.starts_with("preparation:")));
        let mut equivalent = preparation_conflict_catalogue(
            &s,
            &p,
            &batch,
            json!({"mode":"DERIVE","fromKey":p.assets[0].id,"note":"同一父的另一种有效键"}),
        );
        equivalent["assets"][0]["variants"][0]["key"] = json!(p.assets[0].variants[2].id);
        equivalent["assets"][0]["variants"][0]["name"] = json!(p.assets[0].variants[2].name);
        let next =
            apply_catalogue(&equivalent, &s, &batch, &input, &p, None, &HashSet::new()).unwrap();
        assert!(next
            .issues
            .iter()
            .all(|i| !i.coordinate.starts_with("preparation:")));
    }

    #[test]
    fn f2_preparation_conflict_resolution_exception_does_not_close_real_or_program_issues() {
        let (s, p, batch) = preparation_conflict_fixture();
        let f2 = input_for("F2", &s, &batch, &p, &json!({}), 0, 1, false, &[]);
        let raw = preparation_conflict_catalogue(
            &s,
            &p,
            &batch,
            json!({"mode":"DERIVE","fromKey":p.assets[0].variants[0].id}),
        );
        let staged = apply_catalogue(&raw, &s, &batch, &f2, &p, None, &HashSet::new()).unwrap();
        let issue_at = staged
            .issues
            .iter()
            .position(|i| i.coordinate.starts_with("preparation:"))
            .unwrap();
        let mut keep = empty_edit(&s, &batch);
        keep["resolvedIssues"] = json!([staged.issues[issue_at].coordinate]);
        for kind in [
            "blocking",
            "program",
            "ordinary",
            "basis-differs",
            "outside-scope",
        ] {
            let mut p = staged.clone();
            match kind {
                "blocking" => p.issues[issue_at].blocking = true,
                "program" => {
                    p.issues[issue_at].details.as_mut().unwrap()["owner"] = json!("PROGRAM")
                }
                "ordinary" => {
                    p.issues[issue_at].details.as_mut().unwrap()["kind"] = json!("SOURCE_CONFLICT")
                }
                "basis-differs" => {
                    p.issues[issue_at].details.as_mut().unwrap()["previous"]["sourceVariantId"] =
                        json!(p.assets[0].variants[0].id)
                }
                _ => p.issues[issue_at]
                    .episode_ids
                    .push(s.episodes[1].id.clone()),
            }
            let input = input_for("Q1", &s, &batch, &p, &json!({}), 0, 1, true, &[]);
            assert!(
                apply_optimization(&keep, &s, &batch, &input, &p, None).is_err(),
                "{kind}"
            );
        }
    }
    #[test]
    fn inline_uses_strong_baseline_and_editor_keeps_external_appearance_scope() {
        let (s, p) = two_episode_catalogue();
        let asset = &p.assets[0];
        let baseline = &asset.variants[0];
        let inline = &asset.variants[1];
        let untouched = &asset.variants[2];
        assert_eq!(asset.name, "小王");
        assert_eq!(baseline.name, "白衣");
        assert_eq!(
            inline.source_variant_id.as_deref(),
            Some(baseline.id.as_str())
        );
        assert_eq!(inline.metadata["preparation"]["mode"], "INLINE");
        let batch = Batch {
            scope: vec![s.episodes[1].id.clone()],
            pieces: whole_rows(&s, &[s.episodes[1].id.clone()]),
            fragment: None,
        };
        let reference = s
            .rows
            .iter()
            .find(|r| r.episode_id.as_ref() == Some(&s.episodes[1].id) && r.text.contains("白衣"))
            .unwrap()
            .id
            .clone();
        let input = input_for("Q1", &s, &batch, &p, &json!({}), 0, 1, true, &[]);
        let mut wire = empty_edit(&s, &batch);
        wire["changes"] = json!({"assets":[{"key":asset.id,"role":"男主","sourceRefs":[reference],"variants":[{"key":inline.id,"sourceRefs":[reference],"note":"本批有据延续"}]}]});
        let (next, _) = apply_optimization(&wire, &s, &batch, &input, &p, None).unwrap();
        let asset = &next.assets[0];
        assert_eq!(asset.metadata["role"], "男主");
        assert_eq!(
            asset.metadata["relationNote"],
            p.assets[0].metadata["relationNote"]
        );
        assert_eq!(
            asset.variants[0].source_variant_id,
            baseline.source_variant_id
        );
        assert_eq!(
            asset
                .variants
                .iter()
                .find(|v| v.id == inline.id)
                .unwrap()
                .scope
                .scenes
                .len(),
            2
        );
        let shape = asset
            .variants
            .iter()
            .find(|v| v.id == untouched.id)
            .unwrap();
        // A keyed patch changes only its named variant. An omitted sibling is
        // not a request to remove its current-batch or external appearances.
        assert_eq!(
            serde_json::to_value(shape).unwrap(),
            serde_json::to_value(untouched).unwrap()
        );
        assert_eq!(shape.scope.scenes.len(), 2);
        assert!(shape
            .scope
            .scenes
            .iter()
            .any(|place| place.episode_id == s.episodes[0].id));
        assert!(shape
            .scope
            .scenes
            .iter()
            .any(|place| place.episode_id == s.episodes[1].id));
    }
    #[test]
    fn editor_parent_cycle_and_prior_source_borrowing_are_rejected() {
        let (s, p) = two_episode_catalogue();
        let asset = &p.assets[0];
        let child = &asset.variants[1];
        let batch = Batch {
            scope: vec![s.episodes[1].id.clone()],
            pieces: whole_rows(&s, &[s.episodes[1].id.clone()]),
            fragment: None,
        };
        let current = s
            .rows
            .iter()
            .find(|r| r.episode_id.as_ref() == Some(&s.episodes[1].id) && r.text.contains("白衣"))
            .unwrap();
        let previous = s
            .rows
            .iter()
            .find(|r| r.episode_id.as_ref() == Some(&s.episodes[0].id) && r.text.contains("白衣"))
            .unwrap();
        let input = input_for("Q1", &s, &batch, &p, &json!({}), 0, 1, true, &[]);
        let mut cycle = empty_edit(&s, &batch);
        cycle["changes"] = json!({"assets":[{"key":asset.id,"sourceRefs":[current.id],"preparation":{"mode":"DERIVE","fromKey":child.id}}]});
        assert!(apply_optimization(&cycle, &s, &batch, &input, &p, None)
            .unwrap_err()
            .contains("循环"));
        let mut borrowed = empty_edit(&s, &batch);
        borrowed["changes"] = json!({"assets":[{"key":asset.id,"sourceRefs":[previous.id],"episodes":[current.episode_id],"sceneRefs":[[current.episode_id,current.scene_id]]}]});
        assert!(apply_optimization(&borrowed, &s, &batch, &input, &p, None)
            .unwrap_err()
            .contains("本场"));
    }
    #[test]
    fn light_catalogue_changes_bind_current_scope_without_model_reference_inventories() {
        let schema = q1_schema();
        let asset_schema = &schema["properties"]["changes"]["properties"]["assets"]["items"];
        let variant_schema = &asset_schema["properties"]["variants"]["items"];
        for item in [asset_schema, variant_schema] {
            assert_eq!(item["required"], json!([]));
            assert!(item["properties"].get("sourceRefs").is_none());
        }
        assert_eq!(schema["properties"]["changes"]["required"], json!([]));
        let f2 = f2_schema();
        assert!(f2["properties"]["assets"]["items"]["properties"]
            .get("sourceRefs")
            .is_none());
        assert!(
            f2["properties"]["assets"]["items"]["properties"]["variants"]["items"]["properties"]
                .get("sourceRefs")
                .is_none()
        );
        let (source, prior) = two_episode_catalogue();
        let asset = &prior.assets[0];
        let child = &asset.variants[1];
        let batch = Batch {
            scope: vec![source.episodes[1].id.clone()],
            pieces: whole_rows(&source, &[source.episodes[1].id.clone()]),
            fragment: None,
        };
        let reference = source
            .rows
            .iter()
            .find(|row| {
                row.episode_id.as_ref() == Some(&source.episodes[1].id) && row.text.contains("白衣")
            })
            .unwrap();
        let input = input_for("Q1", &source, &batch, &prior, &json!({}), 0, 1, true, &[]);
        let mut edit = empty_edit(&source, &batch);
        edit["changes"] = json!({"assets":[{"key":asset.id,"variants":[{"key":child.id,"preparation":{"mode":"DERIVE","fromKey":asset.variants[0].id}}]}]});
        let (scope_bound, _) =
            apply_optimization(&edit, &source, &batch, &input, &prior, None).unwrap();
        assert_eq!(
            scope_bound.assets[0].metadata["sourceRefsPurpose"],
            "SOURCE_SCOPE"
        );
        assert!(
            scope_bound.facts.is_empty(),
            "source scope must not create visual claims"
        );
        edit["changes"]["assets"][0]["sourceRefs"] = json!([reference.id]);
        edit["changes"]["assets"][0]["variants"][0]["sourceRefs"] = json!([]);
        assert!(apply_optimization(&edit, &source, &batch, &input, &prior, None).is_err());
        edit["changes"]["assets"][0]["variants"][0]["sourceRefs"] = json!([reference.id]);
        let (next, _) = apply_optimization(&edit, &source, &batch, &input, &prior, None).unwrap();
        assert_eq!(next.assets[0].id, asset.id);
        assert_eq!(next.assets[0].variants[1].id, child.id);
        assert_eq!(
            next.assets[0].variants[1].metadata["preparation"]["mode"],
            "DERIVE"
        );
        assert_eq!(next.assets[0].variants[0].name, asset.variants[0].name);
        assert_eq!(next.assets[0].variants[1].scope.scenes, child.scope.scenes);
    }
    #[test]
    fn historical_dictionary_roundtrips_every_claim_neighbor_and_distinct_source_row() {
        let source = prepare_source(
            &(1..=6)
                .map(|number| {
                    format!(
                        "第{number}集\n场景：大厅\n{}小王：我没有受伤。\n",
                        "小王：我受伤了。\n△门边地面留着雨水。\n△小王右臂受伤。\n".repeat(24)
                    )
                })
                .collect::<String>(),
        )
        .unwrap();
        let episode = source.episodes.last().unwrap().id.clone();
        let batch = Batch {
            scope: vec![episode.clone()],
            pieces: whole_rows(&source, &[episode]),
            fragment: None,
        };
        let mut input = input_for(
            "F2",
            &source,
            &batch,
            &empty_foundation(&source),
            &json!({}),
            0,
            1,
            false,
            &[],
        );
        let original = input["evidence"]["historicalEvidenceCandidates"].clone();
        let table = compact_history(&original).unwrap();
        assert_eq!(table["format"], HISTORY_DICTIONARY_FORMAT);
        assert!(units(&table.to_string()) < units(&original.to_string()));
        assert_eq!(expand_history(&table).unwrap(), original);
        let candidates = original.as_array().unwrap();
        assert_eq!(table["rowCount"], candidates.len());
        assert_eq!(
            candidates
                .iter()
                .map(|row| row[0].as_str().unwrap())
                .collect::<HashSet<_>>()
                .len(),
            candidates.len()
        );
        assert!(candidates.iter().any(
            |row| row[3].as_str().unwrap().contains("没有受伤") && row[4] == "CHARACTER_CLAIM"
        ));
        assert!(candidates
            .iter()
            .any(|row| row[5] == "SOURCE_NEIGHBOUR" && !row[6].as_array().unwrap().is_empty()));
        let before = allowed_refs(&input);
        input["evidence"]["historicalEvidenceCandidates"] = table.clone();
        assert_eq!(allowed_refs(&input), before);
        assert_eq!(
            input["evidence"]["candidateUse"],
            "HISTORICAL_EVIDENCE_ONLY"
        );
        assert_eq!(input["evidence"]["currentApplicability"], "UNKNOWN");
        let mut invalid = table.clone();
        invalid["rows"][0][6] = json!([candidates.len() + 1]);
        assert!(expand_history(&invalid).is_err());
        let mut unknown = original.clone();
        unknown[0]
            .as_array_mut()
            .unwrap()
            .push(json!("unknown metadata"));
        assert_eq!(compact_history(&unknown).unwrap(), unknown);
    }
    #[test]
    fn capacity_projection_preserves_small_request_bytes_and_rejects_indivisible_overflow() {
        let source = source(2);
        let batch = plan_batches(
            &source,
            &source
                .episodes
                .iter()
                .map(|episode| episode.id.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .remove(0);
        let foundation = empty_foundation(&source);
        for stage in ["F2", "Q1"] {
            let input = input_for(
                stage,
                &source,
                &batch,
                &foundation,
                &json!({}),
                0,
                1,
                stage == "Q1",
                &[],
            );
            let schema = if stage == "F2" {
                f2_schema()
            } else {
                q1_schema()
            };
            let before = input.to_string();
            assert!(fits(stage, &input, &schema).unwrap());
            assert_eq!(
                capacity_input(stage, input, &schema).unwrap().to_string(),
                before
            );
        }
        let context = json!({"settings":{"direction":"字".repeat(REQUEST_BUDGET)}});
        assert!(preview_foundation_delivery(&source, &context)
            .unwrap_err()
            .contains("超过200k"));
    }
    #[test]
    fn light_transport_is_versioned_and_does_not_alias_the_old_evidence_inventory_request() {
        // Golden captured from the already accepted synthetic live fixture. This
        // test reads no fixture/database and performs no model call.
        let source=prepare_source("第1集\n场景：旧仓库\n△梁晨穿深蓝工作服，持有刻有‘L-17’的银色钥匙站在木门前。陈岚穿灰色风衣，站在梁晨身旁。\n梁晨：这把钥匙只能交给你。\n△梁晨把银色钥匙交给陈岚，陈岚接过收好。\n第2集\n场景：旧仓库\n△紧接上场。梁晨仍穿深蓝工作服，陈岚仍穿灰色风衣，从口袋取出同一把刻有‘L-17’的银色钥匙，打开木门。\n陈岚：钥匙没有离开我。\n△梁晨左臂受伤，袖口留下持续可见血迹；他与陈岚仍在门前等待。\n梁晨：先别走。\n").unwrap();
        let source = upgrade_source_dialogues(&source).unwrap();
        let context = json!({"settings":{"ratio":"16:9","medium":"真人","confirmedDirection":"双人交接关键钥匙的连续行动，保留原文人物、对白、服装与伤态，不补造设定。"}});
        let scope = source
            .episodes
            .iter()
            .map(|episode| episode.id.clone())
            .collect::<Vec<_>>();
        let mut pending = plan_batches(&source, &scope).unwrap();
        let (_, input) = prepare_capacity_batch(
            "F2",
            &source,
            &mut pending,
            &empty_foundation(&source),
            &context,
            0,
            &[],
        )
        .unwrap();
        let request = TextRequest {
            stage: "foundation:F2".into(),
            skill: F2_RULE.into(),
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            input,
            schema: f2_schema(),
            rule_snapshot: snapshot(F2_RULE, &[]).unwrap(),
        };
        assert_eq!(request.input["catalogueContract"], "light-catalogue/v1");
        assert_ne!(
            digest(&serde_json::to_value(request).unwrap().to_string()),
            "6198cfa2fc3aee8634dfd1fce7e84f70e53b71ad19fabca0d8c2907a405839cf"
        );
    }

    #[test]
    fn synthetic_ten_episode_catalogue_recovers_all_47_assets_without_guessing_source() {
        let fixture = synthetic::fixture("rejected-task.json");
        let source = synthetic::source();
        let input = &fixture["snapshot"]["input"];
        let batch = Batch {
            scope: serde_json::from_value(input["scope"].clone()).unwrap(),
            pieces: whole_rows(
                &source,
                &serde_json::from_value::<Vec<String>>(input["scope"].clone()).unwrap(),
            ),
            fragment: None,
        };
        let prior = empty_foundation(&source);
        assert!(apply_catalogue(
            &fixture["result"],
            &source,
            &batch,
            input,
            &prior,
            None,
            &HashSet::new()
        )
        .unwrap_err()
        .contains("未在本次提供"));
        let mut restored = fixture["result"].clone();
        normalize_legacy_catalogue(&mut restored, input).unwrap();
        let accepted = apply_catalogue(
            &restored,
            &source,
            &batch,
            input,
            &prior,
            None,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(accepted.assets.len(), 47);
        assert_eq!(source.sha256, fixture["result"]["sourceSha256"]);
        assert!(accepted.facts.is_empty());
        assert!(accepted
            .assets
            .iter()
            .flat_map(|a| &a.variants)
            .all(|v| v.metadata["sourceRefsPurpose"] == "SOURCE_SCOPE"));
        assert!(flatten_foundation_assets(&accepted).len() >= 47);
        let raw = fixture["result"].clone();
        for invalid in [
            format!(
                "row:{}:{}:{}",
                &source.sha256[..12],
                source.rows[2].range.start,
                source.rows[3].range.end - 1
            ),
            format!(
                "row:{}:{}:{}",
                &source.sha256[..12],
                source.rows[2].range.start + 1,
                source.rows[3].range.end
            ),
            format!(
                "row:000000000000:{}:{}",
                source.rows[2].range.start, source.rows[3].range.end
            ),
        ] {
            let mut damaged = raw.clone();
            damaged["assets"][0]["sourceRefs"] = json!([invalid]);
            assert!(
                normalize_legacy_catalogue(&mut damaged, input).is_err(),
                "invalid boundaries must not round to real rows"
            );
            assert_eq!(
                damaged["assets"][0]["sourceRefs"],
                json!([invalid]),
                "failed normalization is atomic"
            );
        }
        let second = synthetic::fixture("rejected-second-f2.json");
        assert_eq!(second["snapshot"], fixture["snapshot"]);
        let mut restored = second["result"].clone();
        normalize_legacy_catalogue(&mut restored, input).unwrap();
        let accepted = apply_catalogue(
            &restored,
            &source,
            &batch,
            input,
            &prior,
            None,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(accepted.assets.len(), 47);
    }

    #[test]
    fn compact_unlocated_catalogue_stays_unlocated_and_real_evidence_is_exactly_enumerated() {
        let s = source(2);
        let scope = s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        let b = plan_batches(&s, &scope).unwrap().remove(0);
        let p = empty_foundation(&s);
        let input = input_for("F2", &s, &b, &p, &json!({}), 0, 1, false, &[]);
        let mut wire = empty_catalogue(&s, &b);
        wire["assets"] = json!([{"name":"小王｜默认","type":"CH","episodes":[],"sceneRefs":[],"preparation":{"mode":"REFERENCE"}}]);
        let next = apply_catalogue(&wire, &s, &b, &input, &p, None, &HashSet::new()).unwrap();
        assert!(next.assets[0].source_refs.is_empty());
        assert!(next.assets[0].variants[0].scope.scenes.is_empty());
        assert_eq!(next.assets[0].variants[0].scope.kind, "UNRESOLVED");
        let schema = delivery_schema("F2", &input);
        let ids = schema["$defs"]["sourceRow"]["enum"].as_array().unwrap();
        assert_eq!(ids.len(), allowed_refs(&input).len());
        assert!(ids.iter().all(|v| v.as_str().unwrap().starts_with("r:")));
        let row = &s.rows[2];
        let suffix = row.id.splitn(3, ':').nth(2).unwrap();
        let mut reference = json!({"sourceRefs":[format!("r:{suffix}")]});
        restore_delivery_refs(&mut reference, &input).unwrap();
        assert_eq!(reference["sourceRefs"], json!([row.id]));
        assert!(restore_delivery_refs(&mut json!({"sourceRefs":["r:0:999999"]}), &input).is_err());
    }
    #[test]
    fn compact_profile_fits_synthetic_ten_episode_catalogue_in_one_editor_request() {
        let fixture = synthetic::fixture("rejected-task.json");
        let source = synthetic::source();
        let frozen = &fixture["snapshot"]["input"];
        let scope: Vec<String> = serde_json::from_value(frozen["scope"].clone()).unwrap();
        let batch = Batch {
            scope: scope.clone(),
            pieces: whole_rows(&source, &scope),
            fragment: None,
        };
        let empty = empty_foundation(&source);
        let mut raw = fixture["result"].clone();
        normalize_legacy_catalogue(&mut raw, frozen).unwrap();
        let prior =
            apply_catalogue(&raw, &source, &batch, frozen, &empty, None, &HashSet::new()).unwrap();
        let context = json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        let legacy = input_for("Q1", &source, &batch, &prior, &json!({}), 0, 1, true, &[]);
        assert_eq!(
            legacy,
            input_for_legacy("Q1", &source, &batch, &prior, &json!({}), 0, 1, true, &[])
        );
        let compact = capacity_input(
            "Q1",
            input_for("Q1", &source, &batch, &prior, &context, 0, 1, true, &[]),
            &q1_schema(),
        )
        .unwrap();
        assert_eq!(compact["scope"], json!(scope));
        assert_eq!(compact["source"], legacy["source"]);
        assert!(fits("Q1", &compact, &q1_schema()).unwrap());
        let (next, _) =
            prepare_capacity_batch("Q1", &source, &mut vec![batch], &prior, &context, 0, &[])
                .unwrap();
        assert_eq!(next.scope.len(), 10);
        assert_eq!(compact["registry"], json!([]));
        let aliases = super::super::foundation_wire::compile(
            "Q1",
            &compact,
            &delivery_schema("Q1", &compact),
        )
        .unwrap();
        let mut restored = aliases.input.clone();
        aliases.restore(&mut restored).unwrap();
        assert_eq!(restored["source"], compact["source"]);
        println!(
            "10 episodes: legacy Q1={} compact exact Q1={} candidate {} -> {}, wire input={}",
            request_chars("Q1", &legacy, &q1_schema()).unwrap(),
            request_chars("Q1", &compact, &q1_schema()).unwrap(),
            units(&legacy["candidates"].to_string()),
            units(&compact["candidates"].to_string()),
            units(&aliases.input.to_string())
        );
    }
    #[test]
    fn compact_catalogue_preserves_real_evidence_and_unlocated_scope_and_existing_sources() {
        let source = source(2);
        let scope = source
            .episodes
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let batch = Batch {
            scope: scope.clone(),
            pieces: whole_rows(&source, &scope),
            fragment: None,
        };
        let input = input_for(
            "F2",
            &source,
            &batch,
            &empty_foundation(&source),
            &json!({}),
            0,
            1,
            false,
            &[],
        );
        let mut raw = empty_catalogue(&source, &batch);
        raw["assets"] = json!([{"name":"小王｜默认","type":"CH","episodes":[scope[0]],"sceneRefs":[[scope[0],source.scenes[0].id]],"preparation":{"mode":"REFERENCE"},"variants":[{"name":"小王｜Q版","episodes":[],"sceneRefs":[],"preparation":{"mode":"REFERENCE","note":"既有独立制作"}}]},{"name":"未知道具","type":"PROP","episodes":[],"sceneRefs":[],"preparation":{"mode":"REFERENCE"}}]);
        let mut prior = apply_catalogue(
            &raw,
            &source,
            &batch,
            &input,
            &empty_foundation(&source),
            None,
            &HashSet::new(),
        )
        .unwrap();
        let context = json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE});
        let f2 = input_for("F2", &source, &batch, &prior, &context, 0, 1, false, &[]);
        assert_eq!(
            f2["registry"][0]["variants"][1]["preparation"]["mode"],
            "REFERENCE"
        );
        assert!(f2["registry"][0]["variants"][1]["fromKey"].is_null());
        let (catalogue, unlocated) = light_catalogue(&prior, &batch);
        assert!(catalogue[0].get("sourceRefs").is_none());
        assert_eq!(unlocated[0]["kind"], "VARIANT");
        assert_eq!(unlocated[0]["episodes"], json!([]));
        assert_eq!(unlocated[0]["use"], "CANDIDATE_ONLY");
        assert_eq!(unlocated[1]["kind"], "BASE");
        let a = &mut prior.assets[0];
        a.metadata.remove("sourceScope");
        a.metadata
            .insert("sourceRefsPurpose".into(), json!("VISUAL_FACT"));
        a.description = "白衣的可见事实".into();
        let genuine_refs = a.source_refs.clone();
        let (catalogue, _) = light_catalogue(&prior, &batch);
        assert_eq!(catalogue[0]["sourceRefs"], json!(genuine_refs));
        assert_eq!(catalogue[0]["description"], "白衣的可见事实");
        assert_eq!(catalogue[0]["sourceRefsPurpose"], "VISUAL_FACT");
    }
    #[test]
    fn synthetic_editor_patches_base_and_exact_split_scenes_without_a_new_model_call() {
        let fixture = synthetic::fixture("rejected-task.json");
        let editor = synthetic::fixture("rejected-q1.json");
        let source = synthetic::source();
        let input = &fixture["snapshot"]["input"];
        let scope: Vec<String> = serde_json::from_value(input["scope"].clone()).unwrap();
        let batch = Batch {
            scope: scope.clone(),
            pieces: whole_rows(&source, &scope),
            fragment: None,
        };
        let mut catalogue = fixture["result"].clone();
        normalize_legacy_catalogue(&mut catalogue, input).unwrap();
        let prior = apply_catalogue(
            &catalogue,
            &source,
            &batch,
            input,
            &empty_foundation(&source),
            None,
            &HashSet::new(),
        )
        .unwrap();
        let mut value = editor["result"].clone();
        restore_delivery_refs(&mut value, &editor["snapshot"]["input"]).unwrap();
        let q_scope: Vec<String> =
            serde_json::from_value(editor["snapshot"]["input"]["scope"].clone()).unwrap();
        let q_batch = Batch {
            scope: q_scope.clone(),
            pieces: whole_rows(&source, &q_scope),
            fragment: None,
        };
        let (mut accepted, _) = apply_optimization(
            &value,
            &source,
            &q_batch,
            &editor["snapshot"]["input"],
            &prior,
            None,
        )
        .unwrap();
        let final_editor = synthetic::fixture("rejected-q1-final.json");
        let final_scope: Vec<String> =
            serde_json::from_value(final_editor["snapshot"]["input"]["scope"].clone()).unwrap();
        let final_batch = Batch {
            scope: final_scope.clone(),
            pieces: whole_rows(&source, &final_scope),
            fragment: None,
        };
        let projected = source_projection(&source, &accepted.issues, &scope).unwrap();
        let before = accepted.clone();
        let mut final_value = final_editor["result"].clone();
        restore_delivery_refs(&mut final_value, &final_editor["snapshot"]["input"]).unwrap();
        accepted = apply_optimization(
            &final_value,
            &projected,
            &final_batch,
            &final_editor["snapshot"]["input"],
            &accepted,
            None,
        )
        .unwrap()
        .0;
        for patch in final_value["changes"]["assets"].as_array().unwrap() {
            let old = before.assets.iter().find(|a| a.id == patch["key"]).unwrap();
            let new = accepted.assets.iter().find(|a| a.id == old.id).unwrap();
            assert_eq!(old.variants.len(), new.variants.len());
            for variant in old
                .variants
                .iter()
                .filter(|v| v.metadata.get("isBase") != Some(&json!(true)))
            {
                assert_eq!(
                    serde_json::to_value(variant).unwrap(),
                    serde_json::to_value(new.variants.iter().find(|v| v.id == variant.id).unwrap())
                        .unwrap(),
                    "keyed base diff must preserve all omitted child variants"
                );
            }
            assert!(new.variants[0]
                .scope
                .scenes
                .iter()
                .all(|p| !final_scope.contains(&p.episode_id)));
            assert_eq!(new.variants[0].scope.kind, "SCENES");
        }
        accepted.updated_episode_ids = scope;
        let revised = super::super::execute_foundation_scene_edits(&source, &mut accepted)
            .unwrap()
            .unwrap();
        assert_eq!(revised.text, source.text);
        assert_eq!(revised.sha256, source.sha256);
        assert_eq!(revised.scenes.len(), source.scenes.len() + 6);
        assert_eq!(accepted.assets.len(), 48);
        assert!(flatten_foundation_assets(&accepted).len() >= 48);
        assert!(accepted.assets.iter().flat_map(|a| &a.variants).any(|v| v
            .scope
            .scenes
            .iter()
            .any(|p| p.scene_id.ends_with("__L44"))));
        validate_foundation(&revised, &accepted).unwrap();
        // Without its actual split plan, the same guessed scene IDs fail closed.
        value["changes"]["sceneSplits"] = json!([]);
        assert!(apply_optimization(
            &value,
            &source,
            &q_batch,
            &editor["snapshot"]["input"],
            &prior,
            None
        )
        .is_err());
    }
    #[test]
    fn new_scope_bound_identities_are_distinct_even_when_names_and_scope_match() {
        let s = source(1);
        let b = plan_batches(&s, &[s.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let p = empty_foundation(&s);
        let input = input_for("F2", &s, &b, &p, &json!({}), 0, 1, false, &[]);
        let mut wire = empty_catalogue(&s, &b);
        let asset = json!({"name":"小王｜默认","type":"CH","episodes":b.scope,"sceneRefs":[[s.episodes[0].id,s.scenes[0].id]],"preparation":{"mode":"REFERENCE"}});
        wire["assets"] = json!([asset, asset]);
        let next = apply_catalogue(&wire, &s, &b, &input, &p, None, &HashSet::new()).unwrap();
        assert_ne!(next.assets[0].id, next.assets[1].id);
        wire["assets"][0]["episodes"] = json!([]);
        wire["assets"][0]["sceneRefs"] = json!([]);
        wire["assets"][1] = wire["assets"][0].clone();
        let next = apply_catalogue(&wire, &s, &b, &input, &p, None, &HashSet::new()).unwrap();
        assert_ne!(next.assets[0].id, next.assets[1].id);
        assert!(next.assets.iter().all(|a| a.source_refs.is_empty()));
    }
    #[test]
    fn explicit_global_catalogue_requires_actually_supplied_author_rule_and_reason() {
        let make = |s: &LockedSource| {
            let b = plan_batches(s, &[s.episodes[0].id.clone()])
                .unwrap()
                .remove(0);
            let p = empty_foundation(s);
            let input = input_for("F2", s, &b, &p, &json!({}), 0, 1, false, &[]);
            let mut wire = empty_catalogue(s, &b);
            wire["assets"] = json!([{"name":"校徽","type":"PROP","episodes":[],"sceneRefs":[],"preparation":{"mode":"REFERENCE"},"scope":{"kind":"GLOBAL","reason":"作者明确规定全校统一"}}]);
            (b, p, input, wire)
        };
        let s=prepare_source("【世界观设定】\n世界规则：全校校徽为同一形制。\n第1集\n场景：教室\n小王：我见过校徽。\n").unwrap();
        let (b, p, input, wire) = make(&s);
        let next = apply_catalogue(&wire, &s, &b, &input, &p, None, &HashSet::new()).unwrap();
        assert_eq!(next.assets[0].variants[0].scope.kind, "GLOBAL");
        assert!(next.assets[0]
            .source_refs
            .iter()
            .all(|id| s.rows.iter().any(|r| &r.id == id && r.kind == "context")));
        assert!(next.facts.is_empty());
        let mut missing = wire.clone();
        missing["assets"][0]["scope"]["reason"] = json!("");
        assert!(apply_catalogue(&missing, &s, &b, &input, &p, None, &HashSet::new()).is_err());
        let no_rule = prepare_source(
            "【人物小传】\n人物简介：小王见过校徽。\n第1集\n场景：教室\n小王：我见过校徽。\n",
        )
        .unwrap();
        let (b, p, input, wire) = make(&no_rule);
        assert!(apply_catalogue(&wire, &no_rule, &b, &input, &p, None, &HashSet::new()).is_err());
    }
    #[tokio::test]
    async fn eighty_episode_mock_execution_keeps_capacity_tails_packed_and_exactly_once() {
        eighty_episode_mock(false).await;
    }
    #[tokio::test]
    async fn compact_eighty_episode_mock_preserves_original_dependency_and_source_boundaries() {
        eighty_episode_mock(true).await;
    }
    async fn eighty_episode_mock(compact: bool) {
        let started = std::time::Instant::now();
        let context = if compact {
            json!({"foundationModelInputProfile":super::super::foundation_wire::PROFILE})
        } else {
            json!({})
        };
        let source = prepare_source(
            &(1..=80)
                .map(|number| {
                    format!(
                        "第{number}集\n场景：旧仓库\n{}\n梁晨：钥匙在这里。\n",
                        "△梁晨穿深色夹克站在仓库门口，陈岚检查门锁，钥匙的齿纹保持一致。\n"
                            .repeat(24)
                    )
                })
                .collect::<String>(),
        )
        .unwrap();
        let counter = Counter {
            requests: Mutex::new(vec![]),
        };
        let output = run_foundation_delivery(&source, &context, &counter, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(output.updated_episode_ids.len(), 80);
        let requests = counter.requests.lock().unwrap();
        let all = source
            .episodes
            .iter()
            .map(|episode| json!(episode.id))
            .collect::<Vec<_>>();
        for stage in ["foundation:F2", "foundation:Q1"] {
            let calls = requests
                .iter()
                .filter(|request| request.stage == stage)
                .collect::<Vec<_>>();
            assert!(
                !calls.is_empty() && calls.len() < 10,
                "capacity tails were not packed: {stage} {} calls",
                calls.len()
            );
            assert!(
                calls
                    .iter()
                    .all(|request| request.input["scope"].as_array().unwrap().len() > 1),
                "a singleton tail must join the next fitting capacity batch"
            );
            let scopes = calls
                .iter()
                .flat_map(|request| request.input["scope"].as_array().unwrap())
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                scopes, all,
                "source episode coverage/order must stay exactly once for {stage}"
            );
            let raw_rows = calls
                .iter()
                .flat_map(|request| request.input["source"]["rows"].as_array().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                raw_rows.len(),
                source
                    .rows
                    .iter()
                    .filter(|row| row.episode_id.is_some())
                    .count()
            );
            assert_eq!(
                raw_rows
                    .iter()
                    .map(|row| row[0].as_str().unwrap())
                    .collect::<HashSet<_>>()
                    .len(),
                raw_rows.len()
            );
            for request in calls {
                assert!(fits(
                    stage.rsplit(':').next().unwrap(),
                    &request.input,
                    &request.schema
                )
                .unwrap());
                assert!(request.input["delivery"]["fragment"].is_null());
                for row in request.input["source"]["rows"].as_array().unwrap() {
                    let original = source
                        .rows
                        .iter()
                        .find(|original| original.id == row[0])
                        .unwrap();
                    assert_eq!(row[9], original.text);
                }
            }
        }
        let preview = preview_foundation_delivery(&source, &context).unwrap();
        assert_eq!(
            preview["f2PlannedCalls"],
            requests
                .iter()
                .filter(|request| request.stage == "foundation:F2")
                .count()
        );
        assert_eq!(
            preview["q1MinimumCalls"],
            requests
                .iter()
                .filter(|request| request.stage == "foundation:Q1")
                .count()
        );
        if let Ok(path) = std::env::var("DIRECTOR_TEST_80_REPORT") {
            let path = std::path::PathBuf::from(path);
            assert!(path.is_absolute());
            let report = json!({"ok":true,"mock":true,"realModelCalls":0,"episodes":80,"sourceCharacters":units(&source.text),"elapsedMilliseconds":started.elapsed().as_millis(),"profile":if cfg!(debug_assertions){"debug"}else{"release"},"modelInputProfile":context["foundationModelInputProfile"],"calls":requests.iter().map(|r|json!({"stage":r.stage,"episodeCount":r.input["scope"].as_array().unwrap().len(),"scope":r.input["scope"],"requestCharacters":request_chars(r.stage.rsplit(':').next().unwrap(),&r.input,&r.schema).unwrap(),"withinBudget":fits(r.stage.rsplit(':').next().unwrap(),&r.input,&r.schema).unwrap()})).collect::<Vec<_>>(),"preview":preview,"sourceExactlyOnce":true});
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
    }
    #[test]
    fn scoped_program_split_cannot_be_marked_as_already_published() {
        let s = prepare_source("第1集\n场景：大厅\n小王：开始。\n△小王走到门外。\n小王：走。\n")
            .unwrap();
        let batch = plan_batches(&s, &[s.episodes[0].id.clone()])
            .unwrap()
            .remove(0);
        let p = empty_foundation(&s);
        let row = s.rows.iter().find(|r| r.text.contains("走到门外")).unwrap();
        let input = input_for("Q1", &s, &batch, &p, &json!({}), 0, 1, true, &[]);
        let mut wire = empty_edit(&s, &batch);
        wire["changes"] = json!({"sceneSplits":[{"episodeId":s.episodes[0].id,"sceneId":s.scenes[0].id,"cuts":[{"rowId":row.id,"anchor":row.text.trim_end_matches(['\r','\n'])}]}]});
        let (next, _) = apply_optimization(&wire, &s, &batch, &input, &p, None).unwrap();
        assert_eq!(next.issues[0].details.as_ref().unwrap()["owner"], "PROGRAM");
        assert!(!next.issues[0].blocking);
        assert_eq!(s.scenes.len(), 1);
        let mut receipt = empty_edit(&s, &batch);
        receipt["resolvedIssues"] = json!([next.issues[0].coordinate]);
        assert!(apply_optimization(&receipt, &s, &batch, &input, &next, None).is_err());
    }
    struct AcceptedCache {
        calls: Mutex<usize>,
        accepted: Mutex<HashMap<String, Value>>,
    }
    #[async_trait]
    impl TextExecutor for AcceptedCache {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            let key = digest(&serde_json::to_string(&request).unwrap());
            if let Some(value) = self.accepted.lock().unwrap().get(&key) {
                return Ok(value.clone());
            }
            *self.calls.lock().unwrap() += 1;
            Ok(if request.stage == "foundation:F2" {
                let row = request.input["source"]["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row[9].as_str().unwrap().contains("白衣"))
                    .unwrap();
                json!({"format":"ASSET_CATALOG_DESKTOP_V2","sourceSha256":request.input["sourceSha256"],"coverage":request.input["scope"],"assets":[{"name":"小王｜白衣","type":"CH","episodes":[row[2]],"sceneRefs":[[row[2],row[3]]],"sourceRefs":[row[0]],"role":"主角","preparation":{"mode":"REFERENCE"}}]})
            } else {
                json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":request.input["sourceSha256"],"coverage":request.input["scope"],"changes":{}})
            })
        }
        async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
            self.accepted.lock().unwrap().insert(
                digest(&serde_json::to_string(request).unwrap()),
                value.clone(),
            );
            Ok(())
        }
    }
    #[tokio::test]
    async fn exact_accepted_requests_replay_without_another_generation() {
        let s = source(10);
        let cache = AcceptedCache {
            calls: Mutex::new(0),
            accepted: Mutex::new(HashMap::new()),
        };
        let a = run_foundation_delivery(&s, &json!({}), &cache, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        let b = run_foundation_delivery(&s, &json!({}), &cache, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(*cache.calls.lock().unwrap(), 2);
        assert_eq!(a.assets.len(), 1);
        assert_eq!(
            serde_json::to_value(a).unwrap(),
            serde_json::to_value(b).unwrap()
        );
        let preview = preview_foundation_delivery(&s, &json!({})).unwrap();
        assert_eq!(preview["f2PlannedCalls"], 1);
        assert_eq!(preview["q1MinimumCalls"], 1);
        assert_eq!(preview["sourceCoverageVerified"], true);
    }
    #[test]
    fn complete_choice_is_preserved_but_production_note_never_blocks() {
        let s = source(2);
        let batch = plan_batches(
            &s,
            &s.episodes.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        )
        .unwrap()
        .remove(0);
        let p = empty_foundation(&s);
        let input = input_for("Q1", &s, &batch, &p, &json!({}), 0, 1, true, &[]);
        let refs = s
            .rows
            .iter()
            .filter(|r| r.text.contains("白衣"))
            .map(|r| r.id.clone())
            .collect::<Vec<_>>();
        let choice = json!({"decisionKey":"costume/conflict","kind":"SOURCE_CONFLICT","question":"冲突服装采用哪一份？","reason":"两处原文声明冲突","options":["保留前稿","保留后稿"],"recommended":"保留前稿","recommendationReason":"保留前稿来源","evidence":"两份锁定原句","episodes":batch.scope,"sourceRefs":refs,"changeTargets":{"assetKeys":[],"assetFields":[],"sceneFields":["characters"]}});
        let mut wire = empty_edit(&s, &batch);
        wire["humanChoices"] = json!([choice.clone()]);
        wire["notes"] = json!(["动作与表演交第三阶段处理"]);
        let (next, _) = apply_optimization(&wire, &s, &batch, &input, &p, None).unwrap();
        let issue = next
            .issues
            .iter()
            .find(|issue| issue.coordinate == "choice:costume/conflict")
            .unwrap();
        assert!(issue.blocking);
        assert_eq!(issue.details.as_ref().unwrap()["choice"], choice);
        assert_eq!(
            issue.details.as_ref().unwrap()["recommendedIsAdopted"],
            false
        );
        assert!(next
            .issues
            .iter()
            .filter(|issue| issue.coordinate.starts_with("note:"))
            .all(|issue| !issue.blocking));
    }
}
