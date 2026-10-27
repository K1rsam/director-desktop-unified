//! Additive usage evidence port from lite 0.1.2, commit 6dea8d6c57dde38f04a4d71f05f4c039004dd4a9.
//! Pure transition only: no filesystem publication, adoption, media or generation.
//! Full registry and locked-season validation is an explicit integration dependency.
//! This module does not substitute its local shape checks for that validator.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub const SOURCE_ASSET_USES_SHA256: &str =
    "a0e25b58ac19af5450cc7827ef26b618be61124509bce07f7815d83ac7ccf0ad";
pub const SOURCE_ASSETS_SHA256: &str =
    "57c69b4d4c32cc46aeb24077d119c2f9ece4b2b4184299f353c8407700b22425";
pub const SOURCE_SEASON_SCOPE_SHA256: &str =
    "5c1d8e574ae1b9b3153f0af59eb92d140d3c7975228b541d8f3ac1bc6ef2be8b";
type Result<T> = std::result::Result<T, String>;
const MAX_SAFE: i64 = 9_007_199_254_740_991;

/// Must perform the complete upstream registry validation, scope/source/hash binding,
/// and (when require_locked) locked-season gate. It must not mutate the registry.
/// There is intentionally no default implementation. Call evidence is not a PASS claim.
pub trait RegistryValidator {
    fn validate(&mut self, registry: &Value, require_locked: bool) -> Result<()>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageResult {
    pub registry: Value,
    pub changed: bool,
    pub asset_revisions_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<usize>,
    pub validation: String,
    pub validator_calls: Vec<String>,
}

fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn text(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim_matches(js_space).is_empty() && !s.contains(['\0', '\r']))
}
fn js_space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn integer(v: &Value) -> Option<i64> {
    let n = v.as_f64()?;
    (n.is_finite() && n.fract() == 0.0 && n.abs() <= MAX_SAFE as f64).then_some(n as i64)
}
fn positive(v: &Value) -> Option<i64> {
    integer(v).filter(|n| *n > 0)
}
fn fields(v: &Value, allowed: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|o| o.keys().all(|k| allowed.contains(&k.as_str())))
}
fn array<'a>(v: &'a Value, message: &str) -> Result<&'a Vec<Value>> {
    v.as_array().ok_or_else(|| message.into())
}
fn ids(registry: &Value) -> Result<BTreeMap<&str, &Value>> {
    let mut out = BTreeMap::new();
    for a in array(&registry["assets"], "ASSET_REGISTRY: assets missing")? {
        let id = a["id"].as_str().ok_or("ASSET_ID: missing ID")?;
        need(out.insert(id, a).is_none(), "ASSET_ID: duplicate ID")?;
    }
    Ok(out)
}
fn adopted(asset: &Value) -> bool {
    matches!(
        asset["design"]["state"].as_str(),
        Some("TEXT_ADOPTED" | "REFERENCE_ADOPTED")
    )
}
fn scene_key(u: &Value) -> Result<(i64, String)> {
    Ok((
        integer(&u["episode"]).ok_or("ASSET_USAGE: invalid episode")?,
        u["sceneId"]
            .as_str()
            .ok_or("ASSET_USAGE: missing sceneId")?
            .into(),
    ))
}

/// Local usage checks. Caller must also run the full registry/scope validator.
pub fn validate_usage_entries(entries: &Value, registry: &Value, current: bool) -> Result<()> {
    let assets = ids(registry)?;
    let entries = array(entries, "ASSET_USAGE: entries must be an array")?;
    need(!entries.is_empty(), "ASSET_USAGE: entries empty")?;
    let mut seen = HashSet::new();
    for e in entries {
        need(
            fields(e, &["assetId", "assetRevision", "uses"]),
            "ASSET_USAGE: forbidden entry field",
        )?;
        let id = e["assetId"]
            .as_str()
            .ok_or("ASSET_USAGE: missing asset ID")?;
        let a = assets.get(id).ok_or("ASSET_USAGE: unknown asset ID")?;
        let revision =
            positive(&e["assetRevision"]).ok_or("ASSET_USAGE: invalid asset revision")?;
        let actual =
            positive(&a["revision"]).ok_or("ASSET_USAGE: invalid current asset revision")?;
        need(
            seen.insert(id) && revision <= actual,
            "ASSET_USAGE: duplicate ID or future revision",
        )?;
        if current {
            need(
                revision == actual && adopted(a),
                "ASSET_USAGE: current adopted design required",
            )?;
        }
        let uses = array(&e["uses"], "ASSET_USAGE: uses must be an array")?;
        need(!uses.is_empty(), "ASSET_USAGE: uses empty")?;
        let mut scenes = HashSet::new();
        for u in uses {
            need(
                fields(u, &["episode", "sceneId", "purpose", "sourceRefs"])
                    && integer(&u["episode"]).is_some()
                    && text(&u["sceneId"])
                    && text(&u["purpose"]),
                "ASSET_USAGE: invalid use coordinate/purpose",
            )?;
            let refs = array(&u["sourceRefs"], "ASSET_USAGE: sourceRefs must be an array")?;
            let mut refs_seen = HashSet::new();
            need(
                !refs.is_empty()
                    && refs
                        .iter()
                        .all(|r| text(r) && refs_seen.insert(r.as_str().unwrap())),
                "ASSET_USAGE: missing or duplicate sourceRefs",
            )?;
            need(scenes.insert(scene_key(u)?), "ASSET_USAGE: duplicate scene")?;
            // Membership is local; full season shape, source binding and hash are the
            // explicit validator's responsibility, including malformed scope objects.
            if !registry["productionScope"].is_null() {
                let episodes = array(
                    &registry["productionScope"]["episodes"],
                    "SEASON_INPUT: episodes missing",
                )?;
                let member = episodes.iter().any(|e| {
                    integer(&e["number"]) == integer(&u["episode"])
                        && e["sceneIds"]
                            .as_array()
                            .is_some_and(|s| s.contains(&u["sceneId"]))
                });
                need(member, "SEASON_OUT_OF_SCOPE: usage outside selected season")?;
            }
        }
    }
    Ok(())
}

pub fn validate_usage_annotations(registry: &Value) -> Result<()> {
    let Some(rows) = registry.get("usageAnnotations") else {
        return Ok(());
    };
    let mut seen = HashSet::new();
    for row in array(rows, "ASSET_USAGE: usageAnnotations must be an array")? {
        need(
            fields(row, &["id", "reason", "recordedAtRevision", "entries"])
                && text(&row["id"])
                && text(&row["reason"]),
            "ASSET_USAGE: invalid annotation",
        )?;
        let revision = positive(&row["recordedAtRevision"])
            .ok_or("ASSET_USAGE: invalid annotation revision")?;
        need(
            revision
                <= positive(&registry["revision"])
                    .ok_or("ASSET_USAGE: invalid registry revision")?
                && seen.insert(row["id"].as_str().unwrap()),
            "ASSET_USAGE: duplicate annotation or future revision",
        )?;
        validate_usage_entries(&row["entries"], registry, false)?;
    }
    Ok(())
}

/// Annotations remain bound to their original design revision; scopeUses wins ties.
pub fn asset_scope_uses(registry: &Value, asset_id: &str) -> Result<Vec<Value>> {
    let assets = ids(registry)?;
    let asset = assets
        .get(asset_id)
        .ok_or("ASSET_USAGE: unknown asset ID")?;
    let mut uses = match asset.get("scopeUses") {
        Some(v) if !v.is_null() => array(v, "ASSET_USAGE: scopeUses invalid")?.clone(),
        _ => vec![],
    };
    if let Some(rows) = registry.get("usageAnnotations") {
        for row in array(rows, "ASSET_USAGE: annotations invalid")? {
            for e in array(&row["entries"], "ASSET_USAGE: entries invalid")? {
                if e["assetId"] == asset["id"]
                    && integer(&e["assetRevision"]) == integer(&asset["revision"])
                {
                    uses.extend(
                        array(&e["uses"], "ASSET_USAGE: uses invalid")?
                            .iter()
                            .cloned(),
                    );
                }
            }
        }
    }
    let mut seen = HashSet::new();
    let mut out = vec![];
    for u in uses {
        if seen.insert(scene_key(&u)?) {
            out.push(u);
        }
    }
    Ok(out)
}

fn dependencies(
    id: &str,
    assets: &BTreeMap<&str, &Value>,
    active: &mut Vec<String>,
    checked: &mut HashSet<String>,
) -> Result<()> {
    if checked.contains(id) {
        return Ok(());
    }
    need(
        !active.iter().any(|v| v == id),
        &format!(
            "ASSET_DESIGN_DEPENDENCY_CYCLE: {} → {id}",
            active.join(" → ")
        ),
    )?;
    active.push(id.into());
    let asset = assets
        .get(id)
        .ok_or("ASSET_DESIGN_DEPENDENCY: missing asset")?;
    if let Some(deps) = asset["design"].get("dependencies") {
        let mut seen = HashSet::new();
        for d in array(deps, "ASSET_DESIGN_DEPENDENCY: dependencies invalid")? {
            let target_id = d["assetId"]
                .as_str()
                .ok_or("ASSET_DESIGN_DEPENDENCY: missing target")?;
            let target = assets
                .get(target_id)
                .ok_or("ASSET_DESIGN_DEPENDENCY: unknown target")?;
            need(
                seen.insert(target_id)
                    && target_id != id
                    && text(&d["purpose"])
                    && positive(&d["assetRevision"]).is_some(),
                "ASSET_DESIGN_DEPENDENCY: invalid dependency",
            )?;
            need(
                integer(&d["assetRevision"]) == integer(&target["revision"]),
                &format!(
                    "ASSET_DESIGN_DEPENDENCY_STALE: {} → {target_id}",
                    active.join(" → ")
                ),
            )?;
            need(
                adopted(target),
                "ASSET_DESIGN_DEPENDENCY_UNADOPTED: ancestor design not adopted",
            )?;
            dependencies(target_id, assets, active, checked)?;
        }
    }
    active.pop();
    checked.insert(id.into());
    Ok(())
}

// This key only accepts validated usage schemas: every number is a JS-safe integer.
// It is not a general JS stable serializer and must not be used for snapshot hashes.
fn usage_key(asset_id: &Value, revision: &Value, u: &Value) -> Result<String> {
    Ok(serde_json::to_string(&json!([
        asset_id,
        integer(revision).ok_or("ASSET_USAGE: revision invalid")?,
        integer(&u["episode"]).ok_or("ASSET_USAGE: episode invalid")?,
        u["sceneId"],
        u["purpose"],
        u["sourceRefs"]
    ]))
    .map_err(|e| e.to_string())?)
}
fn entries_key(entries: &Value) -> Result<Vec<(String, Vec<String>)>> {
    array(entries, "ASSET_USAGE: entries invalid")?
        .iter()
        .map(|e| {
            Ok((
                format!(
                    "{}@{}",
                    e["assetId"].as_str().ok_or("ASSET_USAGE: ID invalid")?,
                    positive(&e["assetRevision"]).ok_or("ASSET_USAGE: revision invalid")?
                ),
                array(&e["uses"], "ASSET_USAGE: uses invalid")?
                    .iter()
                    .map(|u| usage_key(&e["assetId"], &e["assetRevision"], u))
                    .collect::<Result<Vec<_>>>()?,
            ))
        })
        .collect()
}

pub fn record_uses(
    registry: &Value,
    input: &Value,
    validator: &mut impl RegistryValidator,
) -> Result<UsageResult> {
    validator.validate(registry, true)?;
    validate_usage_annotations(registry)?;
    need(
        fields(
            input,
            &["version", "baseRevision", "requestId", "reason", "entries"],
        ) && input["version"] == "series-asset-uses-v1"
            && positive(&input["baseRevision"]).is_some()
            && text(&input["requestId"])
            && text(&input["reason"]),
        "ASSET_USAGE: invalid request or forbidden fields",
    )?;
    let empty = vec![];
    let rows = match registry.get("usageAnnotations") {
        Some(v) => array(v, "ASSET_USAGE: annotations invalid")?,
        None => &empty,
    };
    let previous = rows.iter().find(|r| r["id"] == input["requestId"]);
    if previous.is_none() {
        need(
            integer(&input["baseRevision"]) == integer(&registry["revision"]),
            "ASSET_BASE_REVISION: stale registry",
        )?;
        for key in ["changes", "requests"] {
            if let Some(values) = registry.get(key) {
                need(
                    !array(values, "ASSET_USAGE: request/change list invalid")?
                        .iter()
                        .any(|r| r["id"] == input["requestId"]),
                    "ASSET_USAGE: requestId already used",
                )?;
            }
        }
    }
    validate_usage_entries(&input["entries"], registry, true)?;
    if let Some(previous) = previous {
        need(
            previous["reason"] == input["reason"]
                && entries_key(&previous["entries"])? == entries_key(&input["entries"])?,
            "ASSET_USAGE: requestId content differs",
        )?;
    }
    let assets = ids(registry)?;
    for e in input["entries"].as_array().unwrap() {
        dependencies(
            e["assetId"].as_str().unwrap(),
            &assets,
            &mut vec![],
            &mut HashSet::new(),
        )?;
    }
    let mut result = UsageResult {
        registry: registry.clone(),
        changed: false,
        asset_revisions_changed: false,
        request_id: previous.map(|_| input["requestId"].as_str().unwrap().into()),
        entries: None,
        validation: "requires-full-registry-validation".into(),
        validator_calls: vec!["before:requireLocked=true".into()],
    };
    if previous.is_none() {
        let mut already = HashSet::new();
        for row in rows {
            for e in row["entries"].as_array().unwrap() {
                for u in e["uses"].as_array().unwrap() {
                    already.insert(usage_key(&e["assetId"], &e["assetRevision"], u)?);
                }
            }
        }
        let mut count = 0;
        for e in input["entries"].as_array().unwrap() {
            let mut fresh = false;
            for u in e["uses"].as_array().unwrap() {
                fresh |= !already.contains(&usage_key(&e["assetId"], &e["assetRevision"], u)?);
            }
            if fresh {
                count += 1;
            }
        }
        if count > 0 {
            let revision =
                positive(&registry["revision"]).ok_or("ASSET_USAGE: invalid registry revision")?;
            need(
                revision < MAX_SAFE,
                "ASSET_USAGE: registry revision exceeds JS safe integer",
            )?;
            result.registry["revision"] = json!(revision + 1);
            let mut rows = rows.clone();
            rows.push(json!({"id":input["requestId"],"reason":input["reason"],"recordedAtRevision":revision+1,"entries":input["entries"]}));
            result.registry["usageAnnotations"] = json!(rows);
            result.changed = true;
            result.request_id = Some(input["requestId"].as_str().unwrap().into());
            result.entries = Some(count);
        }
    }
    validate_usage_annotations(&result.registry)?;
    validator.validate(&result.registry, true)?;
    result
        .validator_calls
        .push("after:requireLocked=true".into());
    Ok(result)
}

/// Desktop extension, stricter than upstream 0.1.2: resolve each reference against
/// the exact frozen sourceHash and actual episode/scene. Never infer from ref spelling.
pub struct SourceUseIndex {
    pub source_hash: String,
    pub references: BTreeMap<String, (i64, String)>,
}
pub fn record_uses_bound(
    registry: &Value,
    input: &Value,
    source: &SourceUseIndex,
    validator: &mut impl RegistryValidator,
) -> Result<UsageResult> {
    need(
        registry["sourceHash"].as_str() == Some(source.source_hash.as_str())
            && source.source_hash.len() == 64
            && source
                .source_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "ASSET_USAGE_SOURCE_BINDING: source hash differs",
    )?;
    validate_usage_entries(&input["entries"], registry, true)?;
    for e in input["entries"].as_array().unwrap() {
        for u in e["uses"].as_array().unwrap() {
            let coordinate = scene_key(u)?;
            for r in u["sourceRefs"].as_array().unwrap() {
                need(source.references.get(r.as_str().unwrap()) == Some(&coordinate), "ASSET_USAGE_SOURCE_BINDING: source reference missing or belongs to another scene")?;
            }
        }
    }
    record_uses(registry, input, validator)
}

#[cfg(test)]
#[path = "series_lite_asset_uses_tests.rs"]
mod tests;
