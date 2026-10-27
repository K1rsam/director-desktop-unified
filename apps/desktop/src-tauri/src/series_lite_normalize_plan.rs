use super::*;

fn emotion_ok(e: &Value) -> bool {
    [
        "subject",
        "contextTone",
        "audienceEffect",
        "change",
        "landing",
    ]
    .iter()
    .all(|k| has(&e[*k]))
        && e["retain"]
            .as_array()
            .is_some_and(|a| !a.is_empty() && a.iter().all(has))
}
fn required(row: &Value) -> bool {
    has(&row["text"])
        && !matches!(
            row["kind"].as_str(),
            Some("metadata" | "blank" | "punctuation")
        )
        && !row["text"]
            .as_str()
            .unwrap()
            .trim_matches(js_space)
            .chars()
            .all(|c| js_space(c) || "！!？?。，,.、…；;：:".contains(c))
}
fn ordered(refs: &mut [Value], rows: &[Value]) {
    refs.sort_by_key(|id| {
        rows.iter()
            .position(|r| r["id"] == *id)
            .unwrap_or(usize::MAX)
    })
}
fn source_id(source: &Value, reference: &Value) -> Result<Value> {
    resolve_source_ref(source, reference, false)
        .map(|r| r["id"].clone())
        .map_err(|e| {
            let code = if e.starts_with("SOURCE_REF_AMBIGUOUS") {
                "SOURCE_REF_AMBIGUOUS"
            } else {
                "SOURCE_REF_UNKNOWN"
            };
            fail(
                code,
                &format!(
                    "{}: {}",
                    if code == "SOURCE_REF_AMBIGUOUS" {
                        "源引用歧义"
                    } else {
                        "源引用不存在"
                    },
                    js(reference)
                ),
            )
        })
}

/// Equivalent to normalizeEventPlan on a pre-normalized source. This function
/// neither normalizes standard-episode-v1 itself nor validates the full plan.
pub fn normalize_event_plan(raw: &Value, source: &Value) -> Result<Value> {
    if raw.is_null() {
        return Err(type_error(
            "Cannot read properties of null (reading 'unitPackingPolicy')",
        ));
    }
    if raw.get("unitPackingPolicy").is_none() {
        return Ok(raw.clone());
    }
    if raw["unitPackingPolicy"] != "event-first-v1" {
        return Err(fail("PLAN_EVENT_POLICY", "未知事件单元策略"));
    }
    let mut events = raw["events"]
        .as_array()
        .filter(|e| !e.is_empty())
        .cloned()
        .ok_or_else(|| fail("PLAN_EVENTS", "完整事件与边界依据缺失"))?;
    let rows = source["sourceRows"]
        .as_array()
        .ok_or_else(|| type_error("source.sourceRows.map is not a function"))?;
    let dialogues = source["dialogue"]
        .as_array()
        .ok_or_else(|| type_error("source.dialogue is not iterable"))?;
    let production: Vec<_> = rows
        .iter()
        .filter(|r| required(r))
        .map(|r| r["id"].clone())
        .collect();
    let mut event_ids = vec![];
    let mut owners = vec![];
    let mut previous_end: Option<usize> = None;
    let mut previous_scene = Value::Null;
    for (i, event) in events.iter_mut().enumerate() {
        if !has(&event["id"])
            || matches!(
                event["id"].as_str(),
                Some("__proto__" | "constructor" | "prototype")
            )
            || event_ids.contains(&event["id"])
            || !has(&event["sceneKey"])
            || !emotion_ok(&event["emotionContract"])
            || !boundary_ok(
                &event["boundaryBefore"],
                &[
                    "episode_start",
                    "continuous",
                    "new_event",
                    "time_space_jump",
                ],
            )
        {
            return Err(fail(
                "PLAN_EVENT",
                "事件标识、场景、完整情绪契约或边界依据无效",
            ));
        }
        event_ids.push(event["id"].clone());
        if (i == 0) != (event["boundaryBefore"]["kind"] == "episode_start") {
            return Err(fail(
                "PLAN_EVENT_BOUNDARY",
                "仅首个事件使用episode_start；后续须说明连续、独立事件或时空跳转",
            ));
        }
        let refs = event["sourceRefs"]
            .as_array()
            .ok_or_else(|| fail("PLAN_EVENT_REFS", "事件须给稳定源引用"))?;
        let mut refs = uniq(
            refs.iter()
                .map(|r| source_id(source, r))
                .collect::<Result<Vec<_>>>()?,
        );
        for q in dialogues {
            let ids = q["sourceIds"]
                .as_array()
                .ok_or_else(|| type_error("q.sourceIds.some is not a function"))?;
            if ids.iter().any(|r| refs.contains(r)) {
                refs = uniq(refs.into_iter().chain(ids.iter().cloned()))
            }
        }
        ordered(&mut refs, rows);
        let indices: Vec<_> = refs
            .iter()
            .filter_map(|r| production.iter().position(|p| p == r))
            .collect();
        if indices.is_empty()
            || previous_end.is_some_and(|p| indices[0] <= p)
            || indices.last().unwrap() - indices[0] + 1 != indices.len()
        {
            return Err(fail(
                "PLAN_EVENT_CONTINUITY",
                "完整事件须按源顺序连续、非空且不交错",
            ));
        }
        previous_end = indices.last().copied();
        for index in indices {
            if owners.contains(&index) {
                return Err(fail("PLAN_EVENT_OVERLAP", "一个制作源只能归一个完整事件"));
            }
            owners.push(index)
        }
        if i > 0
            && event["boundaryBefore"]["kind"] == "continuous"
            && event["sceneKey"] != previous_scene
        {
            return Err(fail("PLAN_EVENT_BOUNDARY", "跨场景不可声明为同一连续事件"));
        }
        previous_scene = event["sceneKey"].clone();
        event["sourceRefs"] = json!(refs);
    }
    if (0..production.len()).any(|i| !owners.contains(&i)) {
        return Err(fail("PLAN_EVENT_COVERAGE", "实际剧情缺少完整事件归属"));
    }
    let original = events.clone();
    let mut groups: Vec<Value> = vec![];
    let mut aliases = HashMap::new();
    for event in events {
        if !groups.is_empty() && event["boundaryBefore"]["kind"] == "continuous" {
            let prior = groups.last_mut().unwrap();
            if prior.get("eventSegments").is_none_or(Value::is_null) {
                prior["eventSegments"] = json!([prior.clone()])
            }
            let segments = prior["eventSegments"]
                .as_array_mut()
                .ok_or_else(|| type_error("previous.eventSegments.push is not a function"))?;
            if let Some(others) = event.get("eventSegments").filter(|v| truth(v)) {
                segments.extend(
                    others
                        .as_array()
                        .ok_or_else(|| type_error("event.eventSegments is not iterable"))?
                        .iter()
                        .cloned(),
                )
            } else {
                segments.push(event.clone())
            }
            if segments
                .iter()
                .any(|s| s.get("emotionContract").is_none_or(Value::is_null))
            {
                return Err(type_error(
                    "Cannot read properties of null or undefined emotionContract",
                ));
            }
            let emotion = combine(
                &segments
                    .iter()
                    .map(|s| s["emotionContract"].clone())
                    .collect::<Vec<_>>(),
                &[
                    "subject",
                    "contextTone",
                    "audienceEffect",
                    "change",
                    "landing",
                ],
            );
            prior["sourceRefs"] = json!(uniq(
                list(&prior["sourceRefs"])
                    .iter()
                    .chain(list(&event["sourceRefs"]))
                    .cloned()
            ));
            prior["emotionContract"] = emotion;
            aliases.insert(js(&event["id"]), js(&prior["id"]));
        } else {
            aliases.insert(js(&event["id"]), js(&event["id"]));
            groups.push(event)
        }
    }
    let packages = raw["workPackages"]
        .as_array()
        .ok_or_else(|| type_error("plan.workPackages is not iterable"))?;
    let mut grouped: HashMap<String, Vec<Value>> = HashMap::new();
    for w in packages {
        let event = original.iter().find(|e| e["id"] == w["eventId"]);
        if event.is_none() {
            return Err(fail(
                "PLAN_EVENT_OWNER",
                &format!("工作包{}须唯一归属所引完整事件", field_js(w, "id")),
            ));
        }
        let refs = w["sourceRefs"]
            .as_array()
            .ok_or_else(|| type_error("w.sourceRefs.some is not a function"))?;
        if refs
            .iter()
            .any(|r| !list(&event.unwrap()["sourceRefs"]).contains(r))
        {
            return Err(fail(
                "PLAN_EVENT_OWNER",
                &format!("工作包{}须唯一归属所引完整事件", field_js(w, "id")),
            ));
        }
        grouped
            .entry(aliases[&js(&w["eventId"])].clone())
            .or_default()
            .push(w.clone());
    }
    let mut new_packages = vec![];
    let mut package_owners = HashMap::new();
    for event in &groups {
        let mut entries = grouped.remove(&js(&event["id"])).unwrap_or_default();
        if entries.is_empty() {
            return Err(fail(
                "PLAN_EVENT_OWNER",
                &format!("事件{}缺少主创工作包", field_js(event, "id")),
            ));
        }
        entries.sort_by_key(|w| {
            list(&w["sourceRefs"])
                .iter()
                .filter_map(|r| rows.iter().position(|row| row["id"] == *r))
                .min()
                .unwrap_or(usize::MAX)
        });
        let mut w = entries[0].clone();
        let mut refs = uniq(
            entries
                .iter()
                .flat_map(|w| list(&w["sourceRefs"]).iter().cloned()),
        );
        ordered(&mut refs, rows);
        w["eventId"] = event["id"].clone();
        w["sourceRefs"] = json!(refs);
        w["emotionContract"] = event["emotionContract"].clone();
        let mut beats = vec![];
        for x in &entries {
            if let Some(b) = x.get("beatContracts").filter(|v| truth(v)) {
                if let Some(b) = b.as_array() {
                    beats.extend(b.iter().cloned())
                } else {
                    beats.push(b.clone())
                }
            } else {
                let mut b = json!({"sourceRefs":x["sourceRefs"]});
                put_if(&mut b, "packageId", x.get("id"));
                put_if(&mut b, "emotionContract", x.get("emotionContract"));
                beats.push(b)
            }
        }
        w["beatContracts"] = json!(beats);
        for key in ["entryFactIds", "contextRefs", "dependsOn", "scenarioKeys"] {
            let mut merged = vec![];
            for x in &entries {
                if let Some(v) = x.get(key).filter(|v| truth(v)) {
                    if let Some(a) = v.as_array() {
                        merged.extend(a.iter().cloned())
                    } else {
                        merged.push(v.clone())
                    }
                }
            }
            w[key] = json!(uniq(merged));
        }
        for old in &entries {
            package_owners.insert(field_js(old, "id"), w.get("id").cloned());
        }
        new_packages.push(w);
    }
    for w in &mut new_packages {
        w["dependsOn"] = json!(uniq(
            list(&w["dependsOn"])
                .iter()
                .map(|id| package_owners
                    .get(&js(id))
                    .and_then(Clone::clone)
                    .filter(truth)
                    .unwrap_or_else(|| id.clone()))
                .filter(|id| *id != w["id"])
        ))
    }
    let mut plan = raw.clone();
    let provisional = plan["provisionalUnits"]
        .as_array_mut()
        .ok_or_else(|| type_error("plan.provisionalUnits is not iterable"))?;
    for u in provisional {
        if !u.is_object() {
            return Err(type_error("provisional unit is not an object"));
        }
        if let Some(owner) = package_owners
            .get(&field_js(u, "packageId"))
            .and_then(Clone::clone)
            .filter(truth)
        {
            u["packageId"] = owner;
        }
    }
    let mut packing = vec![];
    for w in &new_packages {
        if list(&w["beatContracts"]).iter().any(Value::is_null) {
            return Err(type_error(
                "Cannot read properties of null (reading 'packageId')",
            ));
        }
        let from = uniq(
            list(&w["beatContracts"])
                .iter()
                .map(|b| b["packageId"].clone()),
        );
        let merged = from.len() > 1;
        let mut p = json!({"eventId":w["eventId"],"from":from,"action":if merged{"merge"}else{"retain"},"reason":if merged{"complete_event_one_owner"}else{"single_event_owner"}});
        put_if(&mut p, "ownerId", w.get("id"));
        packing.push(p);
    }
    plan["events"] = json!(groups);
    plan["workPackages"] = json!(new_packages);
    plan["eventPacking"] = json!(packing);
    Ok(plan)
}
