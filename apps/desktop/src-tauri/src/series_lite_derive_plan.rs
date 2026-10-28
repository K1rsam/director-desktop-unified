//! Retrospective v0.4.1 plan indices, ported from the frozen lite_edit.py.
//! No inference, source edits, default planning step or adoption side effects.
use super::{parse, Json, Result};
use std::collections::{BTreeMap, BTreeSet};

const MAX_TICKS: u64 = 9_007_199_254_740_991;
fn need(ok: bool, detail: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("derive-plan: {detail}"))
    }
}
fn nonempty(v: Option<&Json>) -> bool {
    matches!(v, Some(Json::String(s)) if !s.trim().is_empty())
}
fn string(v: &Json, key: &str) -> Result<String> {
    Ok(v.get(key)?.string()?.to_owned())
}
fn text(v: impl Into<String>) -> Json {
    Json::String(v.into())
}
fn array(values: impl IntoIterator<Item = String>) -> Json {
    Json::Array(values.into_iter().map(text).collect())
}
fn object<const N: usize>(values: [(&str, Json); N]) -> Json {
    Json::Object(values.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn stable_ids(rows: &[Json], key: &str) -> Result<Vec<String>> {
    rows.iter()
        .map(|row| {
            let row = row.object()?;
            need(
                matches!(row.get("text"), Some(Json::String(_))),
                "invalid frozen source row",
            )?;
            let mut identity = row.get("id").cloned().unwrap_or(Json::Null);
            if identity == Json::Null && nonempty(row.get("sourceId")) {
                identity = text(format!("{key}:{}", row["sourceId"].string()?));
            }
            if identity == Json::Null {
                if let Some(Json::Number {
                    canonical,
                    integer: true,
                }) = row.get("line")
                {
                    if !canonical.starts_with('-') {
                        identity = text(format!("{key}:L{canonical:0>4}"));
                    }
                }
            }
            need(
                nonempty(Some(&identity)),
                "requires stable frozen source IDs",
            )?;
            Ok(identity.string()?.to_owned())
        })
        .collect()
}
fn refs(v: &Json, allowed: &BTreeSet<String>, required: bool, label: &str) -> Result<Vec<String>> {
    let values = v.array()?;
    let mut seen = BTreeSet::new();
    need(!required || !values.is_empty(), &format!("empty {label}"))?;
    values
        .iter()
        .map(|v| {
            let v = v.string()?;
            need(
                !v.trim().is_empty() && allowed.contains(v) && seen.insert(v.to_owned()),
                &format!("duplicate or unknown {label}"),
            )?;
            Ok(v.to_owned())
        })
        .collect()
}
/// Exact integer nanoseconds. No binary floating point or Decimal-context rounding.
fn ticks(v: &Json) -> Result<u64> {
    let Json::Number { canonical, .. } = v else {
        return Err("derive-plan: invalid shot duration".into());
    };
    need(!canonical.starts_with('-'), "invalid shot duration")?;
    let (mantissa, exponent) = canonical.split_once(['e', 'E']).unwrap_or((canonical, "0"));
    let exponent: i128 = exponent
        .parse()
        .map_err(|_| "derive-plan: invalid shot duration exponent")?;
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    need(!digits.is_empty(), "invalid shot duration")?;
    let scale = exponent - fraction.len() as i128 + 9;
    let (digits, zeros) = if scale < 0 {
        let cut = usize::try_from(-scale).map_err(|_| "derive-plan: duration precision")?;
        need(
            cut < digits.len()
                && digits[digits.len().saturating_sub(cut)..]
                    .bytes()
                    .all(|b| b == b'0'),
            "duration exceeds exact 9-decimal/safe tick precision",
        )?;
        (&digits[..digits.len() - cut], 0)
    } else {
        need(
            scale <= 16,
            "duration exceeds exact 9-decimal/safe tick precision",
        )?;
        (digits, scale as usize)
    };
    need(
        digits.len() + zeros <= 16,
        "duration exceeds exact 9-decimal/safe tick precision",
    )?;
    let n = digits
        .parse::<u64>()
        .map_err(|_| "derive-plan: invalid shot duration")?;
    let n = n
        .checked_mul(10_u64.pow(zeros as u32))
        .ok_or("derive-plan: duration overflow")?;
    need(
        n > 0 && n <= MAX_TICKS,
        "duration exceeds exact 9-decimal/safe tick precision",
    )?;
    Ok(n)
}
fn seconds(n: u64) -> Result<Json> {
    let value = format!("{}.{:09}", n / 1_000_000_000, n % 1_000_000_000);
    let value = value.trim_end_matches('0').trim_end_matches('.');
    parse(value.as_bytes())
}

pub(super) fn clear_indices(bundle: &mut Json) -> Result<()> {
    let Json::Object(root) = bundle else {
        return Err("derive-plan: bundle object required".into());
    };
    let Some(Json::Array(episodes)) = root.get_mut("episodes") else {
        return Err("derive-plan: episodes required".into());
    };
    need(episodes.len() == 1, "single episode required")?;
    let Json::Object(episode) = &mut episodes[0] else {
        return Err("derive-plan: episode object required".into());
    };
    let Some(Json::Object(plan)) = episode.get_mut("plan") else {
        return Err("derive-plan: plan object required".into());
    };
    plan.remove("workPackages");
    plan.remove("provisionalUnits");
    Ok(())
}

pub(super) fn derive(bundle: &mut Json) -> Result<bool> {
    let episode = &bundle.get("episodes")?.array()?[0];
    let plan = episode.get("plan")?;
    let draft = episode.get("draft")?;
    let supplied = ["workPackages", "provisionalUnits"]
        .map(|k| plan.object().map(|m| m.contains_key(k)))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    need(
        supplied[0] == supplied[1],
        "refuses mixed missing workPackages/provisionalUnits",
    )?;
    if supplied[0] {
        return Ok(false);
    }
    for (key, expected) in [
        ("schemaVersion", "performance-core-v4-plan"),
        ("unitPackingPolicy", "event-first-v1"),
    ] {
        if let Some(value) = plan.object()?.get(key) {
            need(value.string()? == expected, &format!("unknown plan {key}"))?;
        }
    }
    need(
        nonempty(plan.object()?.get("emotionArc")),
        "requires authored emotionArc",
    )?;
    let facts = plan.get("entryFacts")?.array()?;
    plan.get("fantasyCandidates")?.array()?;
    episode.get("selection")?.get("selectedIds")?.array()?;
    let source = episode.get("source")?;
    need(nonempty(source.object()?.get("key")), "requires source.key")?;
    let rows = source.get("sourceRows")?.array()?;
    need(!rows.is_empty(), "requires sourceRows")?;
    let empty = Vec::new();
    let context_rows = source
        .object()?
        .get("contextRows")
        .map(Json::array)
        .transpose()?
        .unwrap_or(&empty);
    let source_ids = stable_ids(rows, source.get("key")?.string()?)?;
    let context_ids = stable_ids(context_rows, source.get("key")?.string()?)?;
    let all_ids: Vec<_> = source_ids.iter().chain(&context_ids).cloned().collect();
    let allowed_context: BTreeSet<_> = all_ids.iter().cloned().collect();
    need(
        allowed_context.len() == all_ids.len(),
        "duplicate frozen source IDs",
    )?;
    let allowed_source: BTreeSet<_> = source_ids.iter().cloned().collect();
    let events = plan.get("events")?.array()?;
    need(!events.is_empty(), "requires authored events")?;
    let mut event_map = BTreeMap::new();
    for (i, event) in events.iter().enumerate() {
        need(
            nonempty(event.object()?.get("id")) && nonempty(event.object()?.get("sceneKey")),
            "invalid event ID/sceneKey",
        )?;
        let id = string(event, "id")?;
        need(
            !["__proto__", "constructor", "prototype"].contains(&id.as_str())
                && !event_map.contains_key(&id),
            "invalid or duplicate event ID",
        )?;
        let boundary = event.get("boundaryBefore")?;
        let kind = boundary.get("kind")?.string()?;
        need(
            [
                "episode_start",
                "continuous",
                "new_event",
                "time_space_jump",
            ]
            .contains(&kind)
                && nonempty(boundary.object()?.get("reason"))
                && ((i == 0) == (kind == "episode_start")),
            "requires authored event boundaryBefore",
        )?;
        let emotion = event.get("emotionContract")?;
        need(
            [
                "subject",
                "contextTone",
                "audienceEffect",
                "change",
                "landing",
            ]
            .iter()
            .all(|k| nonempty(emotion.object().ok().and_then(|o| o.get(*k)))),
            "requires complete authored emotionContract",
        )?;
        let retain = emotion.get("retain")?.array()?;
        need(
            !retain.is_empty() && retain.iter().all(|v| nonempty(Some(v))),
            "requires complete authored emotionContract retain",
        )?;
        refs(
            event.get("sourceRefs")?,
            &allowed_source,
            true,
            "event sourceRefs",
        )?;
        event_map.insert(id, event);
    }
    let mut fact_ids = BTreeSet::new();
    let mut fact_refs = Vec::new();
    for fact in facts {
        need(nonempty(fact.object()?.get("id")), "invalid entryFact ID")?;
        let id = string(fact, "id")?;
        need(fact_ids.insert(id.clone()), "duplicate entryFact ID")?;
        fact_refs.push((
            id,
            refs(
                fact.get("sourceRefs")?,
                &allowed_context,
                true,
                "entryFact sourceRefs",
            )?,
        ));
    }
    let units = draft.get("units")?.array()?;
    need(!units.is_empty(), "requires completed draft.units")?;
    let mut unit_ids = BTreeSet::new();
    let mut shot_ids = BTreeSet::new();
    let mut used_events = BTreeSet::new();
    let mut event_context: BTreeMap<_, BTreeSet<String>> = event_map
        .keys()
        .map(|id| (id.clone(), BTreeSet::new()))
        .collect();
    let mut provisional = Vec::new();
    for unit in units {
        need(
            nonempty(unit.object()?.get("id")) && unit_ids.insert(string(unit, "id")?),
            "invalid or duplicate unit ID",
        )?;
        let event_id = string(unit, "eventId")?;
        let event = event_map
            .get(&event_id)
            .ok_or("derive-plan: unit missing eventId or references unknown eventId")?;
        used_events.insert(event_id.clone());
        let event_refs: BTreeSet<_> = refs(
            event.get("sourceRefs")?,
            &allowed_source,
            true,
            "event sourceRefs",
        )?
        .into_iter()
        .collect();
        let shots = unit.get("shots")?.array()?;
        need(!shots.is_empty(), "requires completed unit shots")?;
        let mut total = 0_u64;
        let mut used_refs = BTreeSet::new();
        for shot in shots {
            need(
                nonempty(shot.object()?.get("id")) && shot_ids.insert(string(shot, "id")?),
                "invalid or duplicate shot ID",
            )?;
            let shot_refs = refs(
                shot.get("sourceRefs")?,
                &allowed_source,
                true,
                "shot sourceRefs",
            )?;
            need(
                shot_refs.iter().all(|r| event_refs.contains(r)),
                "shot sourceRefs outside its event",
            )?;
            used_refs.extend(shot_refs);
            if let Some(context) = shot.object()?.get("contextRefs") {
                event_context.get_mut(&event_id).unwrap().extend(refs(
                    context,
                    &allowed_context,
                    false,
                    "shot contextRefs",
                )?);
            }
            total = total
                .checked_add(ticks(shot.get("duration")?)?)
                .ok_or("derive-plan: unit duration overflow")?;
        }
        need(
            total <= MAX_TICKS,
            "unit duration exceeds safe tick precision",
        )?;
        let seconds = seconds(total)?;
        provisional.push(object([
            ("id", unit.get("id")?.clone()),
            ("packageId", text(event_id)),
            (
                "sourceRefs",
                array(
                    source_ids
                        .iter()
                        .filter(|id| used_refs.contains(*id))
                        .cloned(),
                ),
            ),
            ("secondsRange", Json::Array(vec![seconds.clone(), seconds])),
        ]));
    }
    need(
        used_events.len() == event_map.len(),
        "event has no completed unit",
    )?;
    let mut packages = Vec::new();
    for event in events {
        let id = string(event, "id")?;
        let context = &event_context[&id];
        let mut evidence: BTreeSet<_> = refs(
            event.get("sourceRefs")?,
            &allowed_source,
            true,
            "event sourceRefs",
        )?
        .into_iter()
        .collect();
        evidence.extend(context.iter().cloned());
        packages.push(object([
            ("id", text(id.clone())),
            ("eventId", text(id)),
            ("sourceRefs", event.get("sourceRefs")?.clone()),
            ("emotionContract", event.get("emotionContract")?.clone()),
            (
                "contextRefs",
                array(all_ids.iter().filter(|r| context.contains(*r)).cloned()),
            ),
            (
                "entryFactIds",
                array(
                    fact_refs
                        .iter()
                        .filter(|(_, refs)| refs.iter().all(|r| evidence.contains(r)))
                        .map(|(id, _)| id.clone()),
                ),
            ),
            ("dependsOn", Json::Array(vec![])),
            ("scenarioKeys", Json::Array(vec![])),
        ]));
    }
    let Json::Object(root) = bundle else {
        unreachable!()
    };
    let Json::Array(episodes) = root.get_mut("episodes").unwrap() else {
        unreachable!()
    };
    let Json::Object(episode) = &mut episodes[0] else {
        unreachable!()
    };
    let Json::Object(plan) = episode.get_mut("plan").unwrap() else {
        unreachable!()
    };
    plan.entry("schemaVersion".into())
        .or_insert_with(|| text("performance-core-v4-plan"));
    plan.entry("unitPackingPolicy".into())
        .or_insert_with(|| text("event-first-v1"));
    plan.insert("workPackages".into(), Json::Array(packages));
    plan.insert("provisionalUnits".into(), Json::Array(provisional));
    Ok(true)
}

#[cfg(test)]
#[path = "series_lite_derive_plan_tests.rs"]
mod tests;
