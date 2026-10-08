//! Rust port of storyboard-v4's event-unit-packing, execution-batches,
//! author-call-groups, accepted-continuity and creative-brief contracts.
//! The original role and method bodies live in rules; this module performs
//! mechanical ownership, transport and timing work, never creative judgement.
use super::*;

pub(super) const SCENARIO_KEYS: &[&str] = &[
    "camera-path-repair",
    "comic-contrast",
    "danger-rescue",
    "editing-integrity",
    "expressive-emphasis",
    "joy-acceptance",
    "long-voice",
    "quiet-nonhuman",
    "scale-silence",
    "spatial-contact",
    "status-conflict",
];

pub(super) fn scenario_rule(key: &str) -> Option<&'static str> {
    Some(match key {
        "camera-path-repair" => "storyboard-card-camera-path-repair",
        "comic-contrast" => "storyboard-card-comic-contrast",
        "danger-rescue" => "storyboard-card-danger-rescue",
        "editing-integrity" => "storyboard-card-editing-integrity",
        "expressive-emphasis" => "storyboard-card-expressive-emphasis",
        "joy-acceptance" => "storyboard-card-joy-acceptance",
        "long-voice" => "storyboard-card-long-voice",
        "quiet-nonhuman" => "storyboard-card-quiet-nonhuman",
        "scale-silence" => "storyboard-card-scale-silence",
        "spatial-contact" => "storyboard-card-spatial-contact",
        "status-conflict" => "storyboard-card-status-conflict",
        _ => return None,
    })
}

fn unique(values: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut result = Vec::new();
    for value in values {
        if !result.contains(&value) {
            result.push(value);
        }
    }
    result
}

fn combine_emotion(items: &[Value], fields: &[&str]) -> Value {
    let mut combined = serde_json::Map::new();
    for key in fields {
        let values = unique(items.iter().map(|item| item[*key].clone()));
        combined.insert(
            (*key).into(),
            json!(values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("；")),
        );
    }
    combined.insert(
        "retain".into(),
        json!(unique(items.iter().flat_map(|item| {
            item["retain"].as_array().cloned().unwrap_or_default()
        }))),
    );
    Value::Object(combined)
}

/// Mirrors normalizeEventPlan: continuous beats in the same real scene become
/// one event/author owner; beat contracts and source anchors remain intact.
pub(super) fn normalize_event_plan(plan: &mut Value, rows: &[SourceRow]) -> Result<(), String> {
    if text(plan, "schemaVersion") != "storyboard-plan/v2"
        || text(plan, "unitPackingPolicy") != "event-first-v1"
    {
        return Err(
            "新版计划须使用 storyboard-plan/v2 与 event-first-v1，不静默退回旧碎分协议".into(),
        );
    }
    let events = plan["events"].as_array().ok_or("缺少完整事件")?.clone();
    let packages = plan["workPackages"].as_array().ok_or("缺少工作包")?.clone();
    let provisional = plan["provisionalUnits"]
        .as_array()
        .ok_or("缺少完整范围联排 provisionalUnits")?
        .clone();
    if provisional.is_empty() {
        return Err("初排必须实际承载完整源范围，不把联排留给下游".into());
    }
    let order: HashMap<_, _> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| (row.id.as_str(), i))
        .collect();
    let mut provisional_ids = HashSet::new();
    for unit in &provisional {
        if text(unit, "id").is_empty() || !provisional_ids.insert(text(unit, "id")) {
            return Err("初排单元 id 空或重复".into());
        }
        let owner = packages
            .iter()
            .find(|w| text(w, "id") == text(unit, "packageId"))
            .ok_or("初排单元缺少工作包")?;
        let refs = strings(unit, "sourceRefs")?;
        let owner_refs = strings(owner, "sourceRefs")?;
        if refs.is_empty() || refs.iter().any(|r| !owner_refs.contains(r)) {
            return Err("初排单元越出工作包源范围".into());
        }
        let seconds = unit["secondsRange"]
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("初排须给两个有效软容量值")?;
        let low = seconds[0].as_f64().ok_or("初排时长无效")?;
        let high = seconds[1].as_f64().ok_or("初排时长无效")?;
        if !low.is_finite() || !high.is_finite() || low <= 0.0 || high < low {
            return Err("初排软容量区间无效".into());
        }
    }
    for package in &packages {
        for key in strings(package, "scenarioKeys")? {
            if scenario_rule(&key).is_none() {
                return Err(format!("未提供的方法卡不可假装加载：{key}"));
            }
        }
        let refs = strings(package, "sourceRefs")?;
        if refs.iter().any(|r| {
            !provisional.iter().any(|u| {
                text(u, "packageId") == text(package, "id")
                    && u["sourceRefs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v.as_str() == Some(r))
            })
        }) {
            return Err(format!("初排遗漏工作包 {} 的正文", text(package, "id")));
        }
    }
    let mut groups: Vec<Value> = Vec::new();
    let mut aliases: HashMap<String, String> = HashMap::new();
    for (index, event) in events.iter().enumerate() {
        let kind = text(&event["boundaryBefore"], "kind");
        if (index == 0) != (kind == "episode_start") || kind == "capacity" {
            return Err("仅首事件使用 episode_start，事件不能用视频容量当剧情边界".into());
        }
        if kind == "continuous" {
            let prior = groups.last_mut().ok_or("首事件不能声明前场连续")?;
            if text(prior, "sceneId") != text(event, "sceneId") {
                return Err("真实跨场景不能声明 continuous".into());
            }
            let mut segments = prior["eventSegments"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![prior.clone()]);
            segments.push(event.clone());
            prior["sourceRefs"] =
                json!(unique(segments.iter().flat_map(|e| {
                    e["sourceRefs"].as_array().cloned().unwrap_or_default()
                })));
            prior["emotionContract"] = combine_emotion(
                &segments
                    .iter()
                    .map(|e| e["emotionContract"].clone())
                    .collect::<Vec<_>>(),
                &[
                    "subject",
                    "contextTone",
                    "audienceEffect",
                    "change",
                    "landing",
                ],
            );
            prior["eventSegments"] = json!(segments);
            aliases.insert(text(event, "id").into(), text(prior, "id").into());
        } else {
            aliases.insert(text(event, "id").into(), text(event, "id").into());
            groups.push(event.clone());
        }
    }
    let mut owners: HashMap<String, String> = HashMap::new();
    let mut merged = Vec::new();
    for event in &groups {
        let mut list: Vec<_> = packages
            .iter()
            .filter(|w| {
                aliases.get(text(w, "eventId")).map(String::as_str) == Some(text(event, "id"))
            })
            .cloned()
            .collect();
        list.sort_by_key(|w| {
            w["sourceRefs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|id| order.get(id))
                .copied()
                .min()
                .unwrap_or(usize::MAX)
        });
        if list.is_empty() {
            return Err(format!("事件 {} 缺少主创工作包", text(event, "id")));
        }
        let mut owner = list[0].clone();
        owner["eventId"] = event["id"].clone();
        owner["sourceRefs"] = event["sourceRefs"].clone();
        owner["emotionContract"] = event["emotionContract"].clone();
        owner["beatContracts"] = json!(list.iter().flat_map(|w| w["beatContracts"].as_array().cloned().unwrap_or_else(||vec![json!({"packageId":w["id"],"sourceRefs":w["sourceRefs"],"emotionContract":w["emotionContract"]})])).collect::<Vec<_>>());
        for key in ["entryFactIds", "contextRefs", "dependsOn", "scenarioKeys"] {
            owner[key] = json!(unique(
                list.iter()
                    .flat_map(|w| w[key].as_array().cloned().unwrap_or_default())
            ));
        }
        for old in &list {
            owners.insert(text(old, "id").into(), text(&owner, "id").into());
        }
        merged.push(owner);
    }
    for owner in &mut merged {
        owner["dependsOn"] = json!(unique(
            strings(owner, "dependsOn")?
                .iter()
                .map(|id| json!(owners.get(id).unwrap_or(id)))
                .filter(|id| id.as_str() != Some(text(owner, "id")))
        ));
    }
    for unit in plan["provisionalUnits"].as_array_mut().unwrap() {
        if let Some(owner) = owners.get(text(unit, "packageId")) {
            unit["packageId"] = json!(owner);
        }
    }
    for candidate in plan["fantasyCandidates"].as_array().ok_or("缺少幻想目录")? {
        let anchors = strings(candidate, "sourceRefs")?;
        let ids: HashSet<_> = merged
            .iter()
            .filter(|w| {
                w["sourceRefs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| anchors.iter().any(|a| r.as_str() == Some(a)))
            })
            .map(|w| text(w, "id"))
            .collect();
        if ids.len() != 1 {
            return Err("同一次幻想须由同一完整作者范围负责；不能自动截断跨包幻想".into());
        }
    }
    plan["events"] = json!(groups);
    plan["workPackages"] = json!(merged);
    plan["eventPacking"] = json!(owners);
    Ok(())
}

#[derive(Debug, Clone)]
pub(super) struct ExecutionBatch {
    pub id: String,
    pub package_ids: Vec<String>,
    pub source_refs: Vec<String>,
    pub provisional_unit_ids: Vec<String>,
    pub estimated_seconds: f64,
    pub exceeds_soft_target: bool,
}
impl ExecutionBatch {
    pub fn value(&self) -> Value {
        json!({"version":"execution-batches-v1","id":self.id,"packageIds":self.package_ids,"sourceRefs":self.source_refs,"provisionalUnitIds":self.provisional_unit_ids,"estimatedSeconds":self.estimated_seconds,"exceedsSoftTarget":self.exceeds_soft_target,"softTargetNote":"5个初排单元/150秒仅为运输目标；不可拆的源句、初排跨度与已选幻想保留完整。"})
    }
}

/// Port of buildExecutionBatches: protect provisional spans, whole utterances
/// and selected fantasies before grouping atoms by 5 units / 150 seconds.
pub(super) fn execution_batches(
    package: &Value,
    plan: &Value,
    rows: &[SourceRow],
    selected: &[Value],
) -> Result<Vec<ExecutionBatch>, String> {
    let package_refs = strings(package, "sourceRefs")?;
    let refs: Vec<_> = rows
        .iter()
        .filter(|r| package_refs.contains(&r.id))
        .map(|r| r.id.clone())
        .collect();
    let rank: HashMap<_, _> = refs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.as_str(), i))
        .collect();
    let rough: Vec<_> = plan["provisionalUnits"]
        .as_array()
        .ok_or("缺少初排")?
        .iter()
        .filter(|u| text(u, "packageId") == text(package, "id"))
        .collect();
    let mut blocked = HashSet::new();
    let mut protect = |span: Vec<String>| {
        let positions: Vec<_> = span
            .iter()
            .filter_map(|id| rank.get(id.as_str()))
            .copied()
            .collect();
        if let (Some(first), Some(last)) = (positions.iter().min(), positions.iter().max()) {
            for edge in *first..*last {
                blocked.insert(edge);
            }
        }
    };
    for unit in &rough {
        protect(strings(unit, "sourceRefs")?);
    }
    // Locked source parser owns one complete utterance per SourceRow.
    for row in rows.iter().filter(|r| r.dialogue.is_some()) {
        protect(vec![row.id.clone()]);
    }
    for candidate in selected {
        protect(strings(candidate, "sourceRefs")?);
    }
    let mut atoms = Vec::new();
    let mut start = 0;
    for index in 0..refs.len() {
        if index + 1 == refs.len() || !blocked.contains(&index) {
            let source_refs = refs[start..=index].to_vec();
            let units: Vec<_> = rough
                .iter()
                .filter(|u| {
                    u["sourceRefs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| source_refs.iter().any(|id| r.as_str() == Some(id)))
                })
                .collect();
            atoms.push((
                source_refs,
                units
                    .iter()
                    .map(|u| text(u, "id").to_string())
                    .collect::<Vec<_>>(),
                units
                    .iter()
                    .filter_map(|u| u["secondsRange"][1].as_f64())
                    .sum::<f64>(),
            ));
            start = index + 1;
        }
    }
    let mut batches: Vec<ExecutionBatch> = Vec::new();
    for (source_refs, unit_ids, seconds) in atoms {
        let append = batches
            .last()
            .map(|b| {
                b.provisional_unit_ids.len() + unit_ids.len() <= 5
                    && b.estimated_seconds + seconds <= 150.0
            })
            .unwrap_or(false);
        if append {
            let batch = batches.last_mut().unwrap();
            batch.source_refs.extend(source_refs);
            batch.provisional_unit_ids.extend(unit_ids);
            batch.estimated_seconds += seconds;
        } else {
            batches.push(ExecutionBatch {
                id: format!("{}-B{:02}", text(package, "id"), batches.len() + 1),
                package_ids: vec![text(package, "id").into()],
                source_refs,
                provisional_unit_ids: unit_ids,
                estimated_seconds: seconds,
                exceeds_soft_target: false,
            });
        }
    }
    for batch in &mut batches {
        batch.exceeds_soft_target =
            batch.provisional_unit_ids.len() > 5 || batch.estimated_seconds > 150.0;
    }
    Ok(batches)
}

/// The only cross-owner author transport is the old bounded adjacent short tail.
pub(super) fn author_batches(
    plan: &Value,
    rows: &[SourceRow],
    selected: &[Value],
) -> Result<Vec<ExecutionBatch>, String> {
    let packages = plan["workPackages"].as_array().ok_or("缺少工作包")?;
    let mut result = Vec::new();
    let mut index = 0;
    while index < packages.len() {
        let mut parent = execution_batches(&packages[index], plan, rows, selected)?;
        if parent.len() == 1 && !parent[0].exceeds_soft_target && index + 1 < packages.len() {
            let child = execution_batches(&packages[index + 1], plan, rows, selected)?;
            let dependencies = strings(&packages[index + 1], "dependsOn")?;
            if child.len() == 1
                && !child[0].exceeds_soft_target
                && child[0].provisional_unit_ids.len() == 1
                && child[0].estimated_seconds <= 6.0
                && dependencies.contains(&text(&packages[index], "id").to_string())
                && parent[0].provisional_unit_ids.len() + 1 <= 5
                && parent[0].estimated_seconds + child[0].estimated_seconds <= 150.0
            {
                parent[0].id = format!("{}+{}", parent[0].id, child[0].id);
                parent[0].package_ids.extend(child[0].package_ids.clone());
                parent[0].source_refs.extend(child[0].source_refs.clone());
                parent[0]
                    .provisional_unit_ids
                    .extend(child[0].provisional_unit_ids.clone());
                parent[0].estimated_seconds += child[0].estimated_seconds;
                index += 1;
            }
        }
        result.extend(parent);
        index += 1;
    }
    Ok(result)
}

pub(in crate::pipeline) fn capacity_brief(
    caps: &Value,
    draft: Option<&Value>,
    voice: Option<&Value>,
    rows: &[SourceRow],
) -> Value {
    let maximum = caps["maxDuration"].as_f64().unwrap_or(30.0);
    let timings:Vec<_>=draft.and_then(|d|d["units"].as_array()).into_iter().flatten().map(|unit|{
        let mut seconds=0.0;let mut speech=0.0;let mut valid=true;
        for shot in unit["shots"].as_array().into_iter().flatten() {
            let duration=shot["duration"].as_f64().unwrap_or(f64::NAN);seconds+=duration;
            let mut windows:Vec<_>=shot["dialogueParts"].as_array().into_iter().flatten().filter_map(|p|Some((p["voiceStart"].as_f64()?,p["voiceEnd"].as_f64()?))).collect();
            windows.sort_by(|a,b|a.0.total_cmp(&b.0));let mut end:f64=0.0;
            for (a,b) in windows {if a<0.0 || b<=a || b>duration {valid=false;}speech+=(b-a.max(end)).max(0.0);end=end.max(b);}
        }
        json!({"id":unit["id"],"eventId":unit["eventId"],"seconds":seconds,"speechWindowSeconds":if valid{json!(speech)}else{Value::Null},"withoutSpeechWindowSeconds":if valid{json!(seconds-speech)}else{Value::Null}})
    }).collect();
    let ids: HashSet<_> = rows
        .iter()
        .filter_map(|r| r.dialogue.as_ref().map(|q| q.id.as_str()))
        .collect();
    let voices: Vec<_> = voice
        .and_then(|v| v["rows"].as_array())
        .into_iter()
        .flatten()
        .filter(|r| {
            ids.contains(text(r, "id")) && r["seconds"].as_f64().map(|s| s > 0.0).unwrap_or(false)
        })
        .collect();
    json!({"maxUnitSeconds":maximum,"preferredRange":if maximum==30.0{json!([26,30])}else if maximum==15.0{json!([13,15])}else{Value::Null},"hardMinimum":false,
      "timingCaveat":"导演声窗与平读都不是演员实配或不可压缩下限；声窗外可能承载动作、反应或幻想。先重排可共时声画与合法切点，不催读、不删有效表演、不加空白凑档。",
      "currentDraft":if draft.is_some(){json!({"unitCount":timings.len(),"totalSeconds":timings.iter().filter_map(|u|u["seconds"].as_f64()).sum::<f64>(),"units":timings})}else{Value::Null},
      "voiceReference":if voice.is_some(){json!({"coveredLines":voices.len(),"totalLines":ids.len(),"seconds":voices.iter().filter_map(|r|r["seconds"].as_f64()).sum::<f64>(),"isPerformedMinimum":false})}else{Value::Null}})
}

/// Mirror packEventUnits. Only an explicit same event + same scene + legal
/// continuous/capacity boundary may merge. No shot duration, voice interval or
/// creative paragraph is shortened; old segment heads remain source of display.
pub(super) fn pack_event_units(
    draft: &mut Value,
    plan: &Value,
    maximum: f64,
) -> Result<(), String> {
    let original = draft["units"].as_array().ok_or("缺少单元")?.clone();
    let events = plan["events"].as_array().ok_or("缺少事件")?;
    let mut packed: Vec<Value> = Vec::new();
    let mut changed = Vec::new();
    let mut remap = HashMap::new();
    for next in original {
        let can_merge = packed
            .last()
            .map(|prior| {
                let event = events
                    .iter()
                    .find(|e| text(e, "id") == text(prior, "eventId"));
                let duration = |u: &Value| {
                    u["shots"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|s| s["duration"].as_f64().unwrap_or(f64::INFINITY))
                        .sum::<f64>()
                };
                text(prior, "eventId") == text(&next, "eventId")
                    && ["continuous", "capacity"].contains(&text(&next["boundaryBefore"], "kind"))
                    && text(prior, "scene") == text(&next, "scene")
                    && text(prior, "sceneId") == text(&next, "sceneId")
                    && duration(prior) + duration(&next) <= maximum + 1e-6
                    && event
                        .map(|e| {
                            [prior, &next].iter().all(|u| {
                                u["shots"].as_array().into_iter().flatten().all(|s| {
                                    s["sourceRefs"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .all(|r| e["sourceRefs"].as_array().unwrap().contains(r))
                                })
                            })
                        })
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        if !can_merge {
            packed.push(next);
            continue;
        }
        let prior = packed.last_mut().unwrap();
        let segment = |u: &Value| {
            let mut part = u.clone();
            let ids = part["shots"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| s["id"].clone())
                .collect::<Vec<_>>();
            part.as_object_mut().unwrap().remove("shots");
            part.as_object_mut().unwrap().remove("segments");
            part["shotIds"] = json!(ids);
            part
        };
        let mut segments = prior["segments"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![segment(prior)]);
        segments.extend(
            next["segments"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![segment(&next)]),
        );
        let joined = |key: &str| {
            segments
                .iter()
                .map(|s| {
                    format!(
                        "【{}—{}】{}",
                        s["shotIds"][0].as_str().unwrap_or(""),
                        s["shotIds"]
                            .as_array()
                            .unwrap()
                            .last()
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                        text(s, key)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        prior["title"] = json!(unique(segments.iter().map(|s| s["title"].clone()))
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" / "));
        prior["unitDirection"] = json!(joined("unitDirection"));
        prior["lighting"] = json!(joined("lighting"));
        prior["intent"] = combine_emotion(
            &segments
                .iter()
                .map(|s| s["intent"].clone())
                .collect::<Vec<_>>(),
            &["subject", "change", "landing"],
        );
        for key in ["props", "participants", "sourceRefs"] {
            prior[key] = json!(unique(
                segments
                    .iter()
                    .flat_map(|s| s[key].as_array().cloned().unwrap_or_default())
            ));
        }
        // Preserve the first independent initial state and all subsequent state
        // boundaries; changing display cannot silently flatten light or props.
        prior["initialState"] = json!([
            text(&segments[0], "initialState").to_string(),
            segments
                .iter()
                .skip(1)
                .map(|s| format!(
                    "【{}起承接】{}",
                    s["shotIds"][0].as_str().unwrap_or(""),
                    text(s, "initialState")
                ))
                .collect::<Vec<_>>()
                .join("\n")
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n"));
        prior["exitState"] = next["exitState"].clone();
        let mut references = prior["assetRefs"].as_array().cloned().unwrap_or_default();
        for reference in next["assetRefs"].as_array().into_iter().flatten() {
            if let Some(old) = references.iter_mut().find(|r| {
                text(r, "assetId") == text(reference, "assetId")
                    && text(r, "variantId") == text(reference, "variantId")
            }) {
                if text(old, "use") != text(reference, "use") {
                    return Err("合并同一资产形象时素材用途冲突，须实际联修".into());
                }
                old["sourceRefs"] = json!(unique(
                    old["sourceRefs"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .chain(
                            reference["sourceRefs"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default()
                        )
                ));
            } else {
                references.push(reference.clone());
            }
        }
        for (index, reference) in references.iter_mut().enumerate() {
            reference["order"] = json!(index + 1);
        }
        prior["assetRefs"] = json!(references);
        prior["shots"]
            .as_array_mut()
            .unwrap()
            .extend(next["shots"].as_array().unwrap().iter().cloned());
        prior["segments"] = json!(segments);
        remap.insert(text(&next, "id").to_string(), text(prior, "id").to_string());
        changed.push(json!({"coordinate":prior["id"],"sourceRefs":prior["sourceRefs"],"change":format!("连续完整事件在容量内合并 {}；原镜长、原话区间与声窗均保留",text(&next,"id")),"preservedFunction":"原有情绪段、造型、光影、动作、原声与真实边界完整保留。"}));
    }
    for realization in draft["fantasyRealization"]
        .as_array_mut()
        .ok_or("缺少幻想记录")?
    {
        realization["unitIds"] = json!(unique(
            strings(realization, "unitIds")?
                .iter()
                .map(|id| json!(remap.get(id).unwrap_or(id)))
        ));
    }
    draft["units"] = json!(packed);
    draft["revisionNotes"]
        .as_array_mut()
        .ok_or("缺少修订记录")?
        .extend(changed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_fixture() -> (Vec<SourceRow>, Value) {
        let source =
            prepare_source("第一集\n场景：门口\n甲：走。\n乙：别走。\n甲：回来。\n").unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .cloned()
            .collect();
        let emotion = json!({"subject":"甲乙","contextTone":"分别","audienceEffect":"不舍可读","change":"催促与挽留","landing":"关系回应","retain":["源声与行为"]});
        let events:Vec<_>=rows.iter().enumerate().map(|(i,row)|json!({"id":format!("E{i}"),"sceneId":row.scene_id,"sourceRefs":[row.id],"boundaryBefore":{"kind":if i==0{"episode_start"}else{"continuous"},"reason":"同场持续回应"},"emotionContract":emotion})).collect();
        let packages:Vec<_>=rows.iter().enumerate().map(|(i,row)|json!({"id":format!("W{i}"),"eventId":format!("E{i}"),"sourceRefs":[row.id],"contextRefs":[],"entryFactIds":[],"dependsOn":if i==0{vec![]}else{vec![format!("W{}",i-1)]},"scenarioKeys":if i==1{vec!["long-voice"]}else{vec![]},"emotionContract":emotion})).collect();
        let units:Vec<_>=rows.iter().enumerate().map(|(i,row)|json!({"id":format!("P{i}"),"packageId":format!("W{i}"),"sourceRefs":[row.id],"secondsRange":[2,3]})).collect();
        (
            rows,
            json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"挽留","events":events,"workPackages":packages,"provisionalUnits":units,"entryFacts":[],"fantasyCandidates":[]}),
        )
    }

    #[test]
    fn continuous_beats_share_one_author_and_keep_their_source_contracts() {
        let (rows, mut plan) = plan_fixture();
        normalize_event_plan(&mut plan, &rows).unwrap();
        assert_eq!(plan["events"].as_array().unwrap().len(), 1);
        assert_eq!(plan["workPackages"].as_array().unwrap().len(), 1);
        assert_eq!(
            plan["workPackages"][0]["beatContracts"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            plan["workPackages"][0]["sourceRefs"],
            json!(rows.iter().map(|r| &r.id).collect::<Vec<_>>())
        );
        assert_eq!(
            plan["workPackages"][0]["scenarioKeys"],
            json!(["long-voice"])
        );
        assert_eq!(plan["workPackages"][0]["dependsOn"], json!([]));
        assert!(plan["provisionalUnits"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["packageId"] == "W0"));
        // Refinement is idempotent: accepted cached work remains identical.
        let stable = plan.clone();
        normalize_event_plan(&mut plan, &rows).unwrap();
        assert_eq!(stable["workPackages"], plan["workPackages"]);
    }

    #[test]
    fn a_real_time_space_boundary_and_unknown_method_are_not_merged() {
        let (rows, mut plan) = plan_fixture();
        plan["events"][1]["boundaryBefore"]["kind"] = json!("time_space_jump");
        plan["events"][2]["boundaryBefore"]["kind"] = json!("new_event");
        normalize_event_plan(&mut plan, &rows).unwrap();
        assert_eq!(plan["events"].as_array().unwrap().len(), 3);
        plan["workPackages"][0]["scenarioKeys"] = json!(["invented-role"]);
        let error = normalize_event_plan(&mut plan, &rows).unwrap_err();
        assert!(error.contains("不存在") || error.contains("不可假装"));
    }

    #[test]
    fn indivisible_fantasy_is_not_cut_to_satisfy_transport_targets() {
        let source = prepare_source(
            "第一集\n场景：门口\n甲：一。\n甲：二。\n甲：三。\n甲：四。\n甲：五。\n甲：六。\n",
        )
        .unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .cloned()
            .collect();
        let refs: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
        let owner = json!({"id":"W","sourceRefs":refs});
        let plan = json!({"provisionalUnits":rows.iter().enumerate().map(|(i,r)|json!({"id":format!("P{i}"),"packageId":"W","sourceRefs":[r.id],"secondsRange":[20,30]})).collect::<Vec<_>>()});
        let normal = execution_batches(&owner, &plan, &rows, &[]).unwrap();
        assert_eq!(normal.len(), 2);
        assert_eq!(normal[0].provisional_unit_ids.len(), 5);
        let selected = vec![json!({"id":"F","sourceRefs":refs})];
        let intact = execution_batches(&owner, &plan, &rows, &selected).unwrap();
        assert_eq!(intact.len(), 1);
        assert_eq!(intact[0].source_refs, refs);
        assert!(intact[0].exceeds_soft_target);
    }

    #[test]
    fn short_tail_group_is_transport_only_and_requires_real_dependency() {
        let (rows, mut plan) = plan_fixture();
        plan["events"][1]["boundaryBefore"]["kind"] = json!("new_event");
        plan["events"][2]["boundaryBefore"]["kind"] = json!("new_event");
        normalize_event_plan(&mut plan, &rows).unwrap();
        let batches = author_batches(&plan, &rows, &[]).unwrap();
        assert_eq!(batches[0].package_ids, vec!["W0", "W1"]);
        assert_eq!(plan["events"].as_array().unwrap().len(), 3);
        plan["workPackages"][1]["dependsOn"] = json!([]);
        let batches = author_batches(&plan, &rows, &[]).unwrap();
        assert_eq!(batches[0].package_ids, vec!["W0"]);
    }

    fn units_fixture() -> (Value, Value) {
        let unit = |id: &str, shot: &str, seconds: f64, boundary: &str| json!({"id":id,"eventId":"E","sceneId":"S","title":id,"unitDirection":"人物作用清晰","intent":{"subject":"甲","change":"应对","landing":"结果","retain":["动作"]},"boundaryBefore":{"kind":boundary,"reason":"同一源事件"},"scene":"门口","props":[],"participants":["甲"],"lighting":"窗侧光","initialState":"甲持物","exitState":"甲松手","sourceRefs":[shot],"assetRefs":[],"shots":[{"id":shot,"duration":seconds,"sourceRefs":[shot],"dialogueParts":[{"lineId":shot,"quoteStart":1,"quoteEnd":2,"voiceStart":0,"voiceEnd":2}]}]});
        (
            json!({"units":[unit("U1","R1",10.0,"episode_start"),unit("U2","R2",8.0,"capacity")],"fantasyRealization":[{"candidateId":"F","unitIds":["U1","U2"],"shotIds":["R1","R2"]}],"revisionNotes":[]}),
            json!({"events":[{"id":"E","sourceRefs":["R1","R2"]}]}),
        )
    }

    #[test]
    fn legal_event_merge_preserves_raw_shots_voice_and_fantasy() {
        let (mut draft, plan) = units_fixture();
        let shots: Vec<_> = draft["units"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|u| u["shots"].as_array().unwrap().clone())
            .collect();
        pack_event_units(&mut draft, &plan, 30.0).unwrap();
        assert_eq!(draft["units"].as_array().unwrap().len(), 1);
        assert_eq!(draft["units"][0]["shots"], json!(shots));
        assert_eq!(draft["fantasyRealization"][0]["unitIds"], json!(["U1"]));
        assert_eq!(draft["units"][0]["segments"].as_array().unwrap().len(), 2);
        let stable = draft.clone();
        pack_event_units(&mut draft, &plan, 30.0).unwrap();
        assert_eq!(stable, draft);
    }

    #[test]
    fn pack_preserves_cross_scene_explicit_boundary_and_over_capacity() {
        for case in 0..3 {
            let (mut draft, plan) = units_fixture();
            match case {
                0 => draft["units"][1]["sceneId"] = json!("other"),
                1 => draft["units"][1]["boundaryBefore"]["kind"] = json!("new_event"),
                _ => draft["units"][1]["shots"][0]["duration"] = json!(25),
            };
            let before = draft.clone();
            pack_event_units(&mut draft, &plan, 30.0).unwrap();
            assert_eq!(before, draft);
        }
    }

    #[test]
    fn overlapping_voice_windows_are_counted_once_and_never_a_hard_minimum() {
        let caps = json!({"maxDuration":30});
        let draft = json!({"units":[{"id":"U","shots":[{"duration":10,"dialogueParts":[{"voiceStart":0,"voiceEnd":4},{"voiceStart":2,"voiceEnd":5}]}]}]});
        let brief = capacity_brief(&caps, Some(&draft), None, &[]);
        assert_eq!(
            brief["currentDraft"]["units"][0]["speechWindowSeconds"],
            5.0
        );
        assert_eq!(
            brief["currentDraft"]["units"][0]["withoutSpeechWindowSeconds"],
            5.0
        );
        assert_eq!(brief["hardMinimum"], false);
    }
}
