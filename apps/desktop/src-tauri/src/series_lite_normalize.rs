//! Native equivalents of frozen lite 0.1.2 normalizeDraft and event packing.
//! Inputs are already-normalized sources. Normalization is NOT validation, adoption,
//! or a production gate; unchanged invalid fields must still pass the real gates.
//! Work on clones so a rejected normalization cannot partially mutate stored input.
use crate::series_lite_bundle_gate::{fantasy_timing, resolve_source_ref, valid_visual_subjects};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NormalizeError {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}
type Result<T> = std::result::Result<T, NormalizeError>;
fn fail(code: &str, message: &str) -> NormalizeError {
    NormalizeError {
        name: "Error".into(),
        code: Some(code.into()),
        message: message.into(),
    }
}
fn type_error(message: &str) -> NormalizeError {
    NormalizeError {
        name: "TypeError".into(),
        code: None,
        message: message.into(),
    }
}
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn has(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim_matches(js_space).is_empty())
}
fn js_space(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
        _ => true,
    }
}
fn js(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Array(a) => a
            .iter()
            .map(|v| if v.is_null() { String::new() } else { js(v) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
        _ => crate::series_lite_registry::canonical(v).unwrap_or_default(),
    }
}
fn field_js(v: &Value, k: &str) -> String {
    v.get(k).map(js).unwrap_or_else(|| "undefined".into())
}
fn uniq(v: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut out = vec![];
    for x in v {
        if !out.contains(&x) {
            out.push(x)
        }
    }
    out
}
fn put_if(out: &mut Value, key: &str, value: Option<&Value>) {
    if let Some(v) = value {
        out[key] = v.clone();
    }
}
fn canonical_row(row: &Value, source: &Value) -> Value {
    if truth(&row["id"]) {
        return row["id"].clone();
    }
    let suffix = if truth(&row["sourceId"]) {
        js(&row["sourceId"])
    } else {
        format!("L{:0>4}", field_js(row, "line"))
    };
    json!(format!("{}:{suffix}", field_js(source, "key")))
}
fn resolved(source: &Value, reference: &Value, context: bool) -> Option<Value> {
    resolve_source_ref(source, reference, context)
        .ok()
        .map(|r| canonical_row(r, source))
}
fn changed(
    obj: &mut Value,
    key: &str,
    to: Value,
    coordinate: Option<&Value>,
    changes: &mut Vec<Value>,
) {
    if obj.get(key) == Some(&to) {
        return;
    }
    let mut change = json!({"field":key,"to":to});
    put_if(&mut change, "coordinate", coordinate);
    put_if(&mut change, "from", obj.get(key));
    obj[key] = to;
    changes.push(change);
}
fn reference_change(coordinate: Option<&Value>, field: String, from: &Value, to: &Value) -> Value {
    let mut c = json!({"field":field,"from":from,"to":to});
    put_if(&mut c, "coordinate", coordinate);
    c
}

pub fn normalize_draft(raw: &Value, source: &Value, options: &Value) -> Result<Value> {
    if options.is_null() {
        return Err(type_error(
            "Cannot read properties of null (reading 'fantasyTimingVersion')",
        ));
    }
    let mut draft = raw.clone();
    let mut changes = vec![];
    if source
        .get("contextRows")
        .is_some_and(|v| truth(v) && !v.is_array())
    {
        return Err(type_error("contextRows.map is not a function"));
    }
    if list(&source["contextRows"]).iter().any(Value::is_null) {
        return Err(type_error("Cannot read properties of null (reading 'id')"));
    }
    let context_ids: Vec<_> = list(&source["contextRows"])
        .iter()
        .map(|r| canonical_row(r, source))
        .collect();
    if let Some(units) = draft.get_mut("units").and_then(Value::as_array_mut) {
        for u in units {
            if let Some(shots) = u.get_mut("shots").and_then(Value::as_array_mut) {
                for shot in shots {
                    if !shot.is_object() {
                        continue;
                    }
                    let coordinate = shot.get("id").cloned();
                    if let Some(refs) = shot.get("sourceRefs").and_then(Value::as_array).cloned() {
                        let mut kept = vec![];
                        for (i, r) in refs.iter().enumerate() {
                            let source_id = resolved(source, r, false).filter(truth);
                            let context_id = if source_id.is_none() {
                                resolved(source, r, true).filter(truth)
                            } else {
                                None
                            };
                            if let Some(cid) = context_id.filter(|id| {
                                context_ids.contains(id)
                                    && shot.get("contextRefs").is_none_or(Value::is_array)
                            }) {
                                if shot.get("contextRefs").is_none() {
                                    shot["contextRefs"] = json!([])
                                }
                                let context = shot["contextRefs"].as_array_mut().unwrap();
                                if !context.contains(&cid) {
                                    context.push(cid.clone())
                                }
                                let mut c = reference_change(
                                    coordinate.as_ref(),
                                    format!("sourceRefs[{i}]"),
                                    r,
                                    &cid,
                                );
                                c["target"] = json!("contextRefs");
                                c["action"] = json!("move");
                                changes.push(c);
                                continue;
                            }
                            if let Some(id) = source_id.filter(|id| id != r) {
                                changes.push(reference_change(
                                    coordinate.as_ref(),
                                    format!("sourceRefs[{i}]"),
                                    r,
                                    &id,
                                ));
                                kept.push(id)
                            } else {
                                kept.push(r.clone())
                            }
                        }
                        shot["sourceRefs"] = json!(kept);
                    }
                    if let Some(refs) = shot.get_mut("contextRefs").and_then(Value::as_array_mut) {
                        for (i, r) in refs.iter_mut().enumerate() {
                            if let Some(id) =
                                resolved(source, r, true).filter(|id| truth(id) && id != r)
                            {
                                changes.push(reference_change(
                                    coordinate.as_ref(),
                                    format!("contextRefs[{i}]"),
                                    r,
                                    &id,
                                ));
                                *r = id;
                            }
                        }
                    }
                    if let Some(parts) = shot.get_mut("dialogueParts").and_then(Value::as_array_mut)
                    {
                        for p in parts {
                            if p.is_object() {
                                let to = match p["presentation"].as_str() {
                                    Some("画外→画内" | "画外到画内" | "画外切画内") => {
                                        Some("画外转画内")
                                    }
                                    Some("画内→画外" | "画内到画外" | "画内切画外") => {
                                        Some("画内转画外")
                                    }
                                    _ => None,
                                };
                                if let Some(to) = to {
                                    changed(
                                        p,
                                        "presentation",
                                        json!(to),
                                        coordinate.as_ref(),
                                        &mut changes,
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(records) = draft
        .get_mut("fantasyRealization")
        .and_then(Value::as_array_mut)
    {
        for r in records {
            if r.is_object() && matches!(r["status"].as_str(), Some("已兑现" | "implemented")) {
                let c = r.get("candidateId").cloned();
                changed(r, "status", json!("realized"), c.as_ref(), &mut changes);
            }
        }
    }
    if options["fantasyTimingVersion"] == "fantasy-spans-v1" {
        let derived: Vec<_> = list(&draft["fantasyRealization"])
            .iter()
            .map(|r| {
                if r.get("screenSpans").is_some() {
                    fantasy_timing(r, &draft)
                } else {
                    None
                }
            })
            .collect();
        if let Some(records) = draft
            .get_mut("fantasyRealization")
            .and_then(Value::as_array_mut)
        {
            for (r, d) in records.iter_mut().zip(derived) {
                if let Some(d) = d {
                    for (key, to) in [
                        ("shotIds", d["shotIds"].clone()),
                        ("screenSeconds", d["screenSeconds"].clone()),
                        ("timingVersion", json!("fantasy-spans-v1")),
                    ] {
                        if r.get(key).is_some_and(|from| {
                            crate::series_lite_registry::canonical(from).ok()
                                == crate::series_lite_registry::canonical(&to).ok()
                        }) {
                            continue;
                        }
                        let mut c = json!({"field":key,"to":to,"action":"derive","reason":"verified_shot_ranges"});
                        put_if(&mut c, "coordinate", r.get("candidateId"));
                        put_if(&mut c, "from", r.get(key));
                        changes.push(c);
                        r[key] = to;
                    }
                }
            }
        }
    }
    changes.extend(pack_event_units_in_place(&mut draft, options)?);
    Ok(json!({"draft":draft,"changes":changes}))
}

fn combine(items: &[Value], keys: &[&str]) -> Value {
    let mut out = json!({});
    for key in keys {
        let values = uniq(items.iter().map(|x| x[*key].clone()));
        out[*key] = json!(values
            .iter()
            .map(|v| if v.is_null() { String::new() } else { js(v) })
            .collect::<Vec<_>>()
            .join("；"));
    }
    out["retain"] = json!(uniq(
        items
            .iter()
            .flat_map(|x| list(&x["retain"]).iter().cloned())
    ));
    out
}
fn boundary_ok(b: &Value, kinds: &[&str]) -> bool {
    b["kind"].as_str().is_some_and(|s| kinds.contains(&s)) && has(&b["reason"])
}
fn seconds(u: &Value) -> Option<f64> {
    let shots = u["shots"].as_array()?;
    if shots.is_empty() {
        return None;
    }
    shots.iter().try_fold(0.0, |sum, s| {
        s["duration"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0)
            .map(|n| sum + n)
    })
}
fn event_sources_match(prior: &Value, next: &Value, event: &Value) -> Result<bool> {
    for u in [prior, next] {
        for shot in list(&u["shots"]) {
            let Some(refs) = shot["sourceRefs"].as_array() else {
                return Ok(false);
            };
            for reference in refs {
                let contains = match &event["sourceRefs"] {
                    Value::Array(a) => a.contains(reference),
                    Value::String(s) => s.contains(&js(reference)),
                    _ => return Err(type_error("event.sourceRefs.includes is not a function")),
                };
                if !contains {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
fn parts(u: &Value) -> Result<Vec<Value>> {
    if let Some(s) = u.get("segments").filter(|v| truth(v)) {
        return s
            .as_array()
            .cloned()
            .ok_or_else(|| type_error("segments is not iterable"));
    }
    let mut part = u.clone();
    let obj = part
        .as_object_mut()
        .ok_or_else(|| type_error("unit is not an object"))?;
    obj.remove("shots");
    obj.remove("segments");
    obj.insert(
        "shotIds".into(),
        json!(list(&u["shots"])
            .iter()
            .map(|s| s["id"].clone())
            .collect::<Vec<_>>()),
    );
    Ok(vec![part])
}
fn span_label(p: &Value) -> Result<String> {
    let s = p["shotIds"]
        .as_array()
        .ok_or_else(|| type_error("segment shotIds is not an array"))?;
    Ok(format!(
        "{}—{}",
        s.first().map(js).unwrap_or_else(|| "undefined".into()),
        s.last().map(js).unwrap_or_else(|| "undefined".into())
    ))
}
fn fixed3(n: f64) -> f64 {
    // JS toFixed rounds the exact IEEE value, not a previously rounded n*1000.
    let bits = n.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let mut m = (bits & ((1u64 << 52) - 1)) as u128;
    let shift = if exp == 0 {
        -1074
    } else {
        m += 1u128 << 52;
        exp - 1023 - 52
    };
    let product = m * 1000;
    let rounded = if shift >= 0 {
        product.checked_shl(shift as u32).unwrap_or(u128::MAX)
    } else {
        let s = (-shift) as u32;
        if s >= 128 {
            0
        } else {
            let q = product >> s;
            q + u128::from(s > 0 && (product - (q << s)) >= (1u128 << (s - 1)))
        }
    };
    rounded as f64 / 1000.0
}

/// Standalone pack equivalent. Returned changes describe normalization only.
pub fn pack_event_units(raw: &Value, options: &Value) -> Result<Value> {
    let mut draft = raw.clone();
    let changes = pack_event_units_in_place(&mut draft, options)?;
    Ok(json!({"draft":draft,"changes":changes}))
}
fn pack_event_units_in_place(draft: &mut Value, options: &Value) -> Result<Vec<Value>> {
    if options.is_null() {
        return Err(type_error("Cannot destructure null options"));
    }
    let mut changes = vec![];
    let Some(policy) = options.get("unitPackingPolicy") else {
        return Ok(changes);
    };
    let plan = &options["plan"];
    if policy != "event-first-v1"
        || plan["unitPackingPolicy"] != "event-first-v1"
        || list(&plan["events"]).is_empty()
    {
        return Err(fail(
            "DRAFT_EVENT_POLICY",
            "新版整编必须携带明确完整事件计划",
        ));
    }
    let events = list(&plan["events"]);
    let units = list(&draft["units"]);
    if units.is_empty()
        || units.iter().any(|u| {
            !has(&u["eventId"])
                || !events.iter().any(|e| e["id"] == u["eventId"])
                || !boundary_ok(
                    &u["boundaryBefore"],
                    &[
                        "episode_start",
                        "continuous",
                        "capacity",
                        "new_event",
                        "time_space_jump",
                    ],
                )
        })
    {
        return Err(fail(
            "DRAFT_EVENT_METADATA",
            "新版单元必须有有效事件归属和明确边界，不能静默保留碎分",
        ));
    }
    if options.get("stage").is_some_and(|s| s != "finish") || units.len() < 2 {
        return Ok(changes);
    }
    let cap = options["settings"]
        .get("maxUnitSeconds")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(json!(30));
    if cap != 15 && cap != 30 {
        return Ok(changes);
    }
    let cap = cap.as_f64().unwrap();
    let units = units.to_vec();
    let mut packed: Vec<Value> = vec![];
    for next in &units {
        let Some(prior) = packed.last_mut() else {
            packed.push(next.clone());
            continue;
        };
        let event = events.iter().find(|e| e["id"] == prior["eventId"]);
        let reason = if event.is_none() || !events.iter().any(|e| e["id"] == next["eventId"]) {
            Some(("unknown_event", "事件归属未确认，保留边界"))
        } else if prior["eventId"] != next["eventId"] {
            Some(("different_event", "不同完整事件，保留边界"))
        } else if matches!(
            next["boundaryBefore"]["kind"].as_str(),
            Some("new_event" | "time_space_jump" | "episode_start")
        ) {
            Some(("explicit_boundary", "明确独立事件或时空跳转，保留边界"))
        } else if !boundary_ok(&next["boundaryBefore"], &["continuous", "capacity"]) {
            Some(("unconfirmed_boundary", "缺少明确连续依据，保留边界"))
        } else if !has(&prior["scene"]) || prior["scene"] != next["scene"] {
            Some(("scene_changed_or_unconfirmed", "场景头不一致，未猜测为同场"))
        } else if [&*prior, next].iter().any(|u| {
            seconds(u).is_none()
                || !u["intent"]["retain"].is_array()
                || ["props", "participants"].iter().any(|k| !u[*k].is_array())
                || ["lighting", "initialState", "exitState"]
                    .iter()
                    .any(|k| !has(&u[*k]))
                || u.get("visualSubjects")
                    .is_some_and(|v| !valid_visual_subjects(v))
        }) {
            Some(("invalid_unit", "现稿字段或时长未完整，交既有校验处理"))
        } else {
            let event = event.unwrap();
            if !event_sources_match(prior, next, event)? {
                Some(("event_source_mismatch", "镜头源引用与事件归属不符，未合并"))
            } else if seconds(prior).unwrap() + seconds(next).unwrap() > cap + 1e-6 {
                Some((
                    "capacity_exceeded",
                    "自然镜长合计超过所选容量，不压秒、不删表演",
                ))
            } else {
                None
            }
        };
        if let Some((reason, detail)) = reason {
            let coordinate = next
                .get("id")
                .filter(|v| truth(v))
                .cloned()
                .unwrap_or(json!("unit"));
            changes.push(json!({"coordinate":coordinate,"field":"unitBoundary","action":"retain","reason":reason,"detail":detail}));
            packed.push(next.clone());
            continue;
        }
        let mut segments = parts(prior)?;
        segments.extend(parts(next)?);
        if segments.is_empty()
            || segments
                .iter()
                .any(|s| !s.is_object() || !s["intent"].is_object())
        {
            return Err(type_error("invalid segment intent"));
        }
        let duration = seconds(prior).unwrap() + seconds(next).unwrap();
        let edge = prior["shots"].as_array_mut().unwrap().last_mut().unwrap();
        if let Some(t) = edge["transition"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned)
        {
            if ["下一单元", "下个单元"].iter().any(|s| t.contains(s))
                && !["外部", "另场", "次日", "后续事件"]
                    .iter()
                    .any(|s| t.contains(s))
                && !units.iter().any(|u| {
                    u["id"] != next["id"] && has(&u["id"]) && t.contains(u["id"].as_str().unwrap())
                })
            {
                let to = t
                    .replace("下一单元", "下一段")
                    .replace("下个单元", "下一段");
                let mut c = json!({"field":"transition","action":"normalize","from":t,"to":to,"reason":"merged_internal_boundary"});
                put_if(&mut c, "coordinate", edge.get("id"));
                changes.push(c);
                edge["transition"] = json!(to);
            }
        }
        let light = uniq(segments.iter().map(|s| s["lighting"].clone()));
        let lighting = if light.len() == 1 {
            light[0].clone()
        } else {
            json!(segments
                .iter()
                .map(|s| Ok(format!("【{}】{}", span_label(s)?, field_js(s, "lighting"))))
                .collect::<Result<Vec<_>>>()?
                .join("\n"))
        };
        let mut initial = vec![segments
            .first()
            .and_then(|s| s.get("initialState"))
            .filter(|v| !v.is_null())
            .map(js)
            .unwrap_or_default()];
        for s in segments.iter().skip(1) {
            let ids = s["shotIds"]
                .as_array()
                .ok_or_else(|| type_error("segment shotIds is not an array"))?;
            initial.push(format!(
                "【{}起承接】{}",
                ids.first().map(js).unwrap_or_else(|| "undefined".into()),
                field_js(s, "initialState")
            ))
        }
        let directed: Vec<_> = segments
            .iter()
            .filter(|s| has(&s["unitDirection"]))
            .collect();
        if !directed.is_empty() {
            prior["unitDirection"] = if directed.len() == segments.len()
                && uniq(directed.iter().map(|s| s["unitDirection"].clone())).len() == 1
            {
                directed[0]["unitDirection"].clone()
            } else {
                json!(directed
                    .iter()
                    .map(|s| Ok(format!(
                        "【{}】{}",
                        span_label(s)?,
                        field_js(s, "unitDirection")
                    )))
                    .collect::<Result<Vec<_>>>()?
                    .join("\n"))
            };
        }
        prior["title"] = json!(uniq(segments.iter().map(|s| s["title"].clone()))
            .iter()
            .map(|v| if v.is_null() { String::new() } else { js(v) })
            .collect::<Vec<_>>()
            .join(" / "));
        prior["intent"] = combine(
            &segments
                .iter()
                .map(|s| s["intent"].clone())
                .collect::<Vec<_>>(),
            &["subject", "change", "landing"],
        );
        for k in ["props", "participants"] {
            prior[k] = json!(uniq(
                segments.iter().flat_map(|s| list(&s[k]).iter().cloned())
            ))
        }
        if prior.get("visualSubjects").is_some() || next.get("visualSubjects").is_some() {
            let mut subjects = vec![];
            for s in list(&prior["visualSubjects"])
                .iter()
                .chain(list(&next["visualSubjects"]))
            {
                if !subjects
                    .iter()
                    .any(|p: &Value| p["name"] == s["name"] && p["domain"] == s["domain"])
                {
                    subjects.push(s.clone())
                }
            }
            prior["visualSubjects"] = json!(subjects)
        }
        prior["lighting"] = lighting;
        prior["initialState"] = json!(initial.join("\n"));
        prior["exitState"] = next["exitState"].clone();
        prior["shots"]
            .as_array_mut()
            .unwrap()
            .extend(list(&next["shots"]).iter().cloned());
        prior["segments"] = json!(segments);
        if let Some(records) = draft
            .get_mut("fantasyRealization")
            .and_then(Value::as_array_mut)
        {
            for f in records {
                if let Some(ids) = f.get("unitIds").and_then(Value::as_array) {
                    f["unitIds"] = json!(uniq(ids.iter().map(|id| if *id == next["id"] {
                        prior["id"].clone()
                    } else {
                        id.clone()
                    })));
                }
            }
        }
        let mut c = json!({"field":"units","action":"merge","from":[prior["id"],next["id"]],"seconds":fixed3(duration),"reason":"continuous_event_within_capacity","detail":"同一连续事件在所选容量内合并；画面、声画、镜长及台词未改，仅必要的内部单元转场用语另记归一；原分段头保存在segments"});
        put_if(&mut c, "coordinate", prior.get("id"));
        put_if(&mut c, "to", prior.get("id"));
        put_if(&mut c, "eventId", prior.get("eventId"));
        changes.push(c);
    }
    draft["units"] = json!(packed);
    Ok(changes)
}

#[path = "series_lite_normalize_plan.rs"]
mod plan;
pub use plan::normalize_event_plan;
#[cfg(test)]
#[path = "series_lite_normalize_tests.rs"]
mod tests;
