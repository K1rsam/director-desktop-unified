//! Explicit, source-bound interpretation of an adopted identity after art-only edits.
//! Never changes the stored draft or relaxes the shared asset-use validator.
use super::{asset_use_binding, digest};
use serde_json::{json, Value};
use std::collections::HashSet;
pub(crate) const FIELD: &str = "refinementIdentityRebinding";
pub(crate) const RECOVERY: &str = "refinementIdentityRebindingRecovery";
const VERSION: &str = "accepted-refinement-identity-rebinding/v1";
const RECOVERY_VERSION: &str = "undispatched-refinement-identity-recovery/v1";
const HELD: &str = "精修素材重绑定证据或原采用稿已变化；保留原请求，未重新派发";
fn sha(v: &Value) -> String {
    digest(&v.to_string())
}
fn sealed(mut v: Value) -> Value {
    v["sha256"] = json!(sha(&v));
    v
}
fn valid_seal(v: &Value) -> bool {
    let mut u = v.clone();
    u.as_object_mut().map(|m| m.remove("sha256"));
    v["sha256"] == sha(&u)
}
fn hash(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
fn evidence(a: &Value) -> Value {
    json!({"purpose":"REGISTERED_IDENTITY_NOT_ACTION","assetId":a["id"],"variantId":a["variantId"],"assetVersion":a["version"],"sourceRefsSha256":sha(&a["sourceRefs"]),"sourceRefsPurpose":a["sourceRefsPurpose"]})
}
fn units(input: &Value) -> Result<Vec<&Value>, String> {
    Ok(input["existingUnits"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .filter(|u| u["episodeId"] == input["episodeId"] && u["retired"] != true)
        .collect())
}
fn bindings(input: &Value) -> Result<Value, String> {
    let mut out = units(input)?
        .iter()
        .map(|u| json!({"unitId":u["id"],"unitVersion":u["version"],"unitSha256":sha(u)}))
        .collect::<Vec<_>>();
    out.sort_by_key(|u| u["unitId"].as_str().unwrap_or("").to_owned());
    Ok(json!(out))
}
fn registration(a: &Value) -> Value {
    let mut v = a.clone();
    if let Some(m) = v.as_object_mut() {
        for k in ["version", "design", "prompt"] {
            m.remove(k);
        }
    }
    v
}
fn entry(
    input: &Value,
    unit: &Value,
    kind: &str,
    index: usize,
    old: &Value,
    origin: &str,
    origin_sha: &str,
) -> Result<Value, String> {
    let reference = &unit[kind][index];
    let matches = input["assets"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .filter(|a| a["id"] == reference["assetId"] && a["variantId"] == reference["variantId"])
        .collect::<Vec<_>>();
    let [current] = matches.as_slice() else {
        return Err(HELD.into());
    };
    if old["id"] != current["id"]
        || old["variantId"] != current["variantId"]
        || registration(old) != registration(current)
        || old["version"]
            .as_u64()
            .is_none_or(|v| v == 0 || current["version"].as_u64().is_none_or(|n| n <= v))
        || reference["identityEvidence"] != evidence(old)
    {
        return Err(HELD.into());
    }
    let context = input["revisionContexts"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .find(|c| {
            c["episodeId"] == input["episodeId"]
                && c["originWorkflowId"] == origin
                && c["sourceSha256"] == input["source"]["sha256"]
        })
        .ok_or(HELD)?;
    if context["unitBindings"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .filter(|b| {
            b["unitId"] == unit["id"]
                && b["unitVersion"] == unit["version"]
                && b["unitSha256"] == sha(unit)
        })
        .count()
        != 1
    {
        return Err(HELD.into());
    }
    Ok(
        json!({"unitId":unit["id"],"unitVersion":unit["version"],"unitSha256":sha(unit),"referenceKind":kind,"referenceIndex":index,
 "originWorkflowId":origin,"originSnapshotSha256":origin_sha,"assetBefore":old,"currentAssetSha256":sha(current),"priorIdentityEvidence":reference["identityEvidence"],"currentIdentityEvidence":evidence(current)}),
    )
}
/// Origins are fetched from authoritative accepted workflow snapshots by Engine.
pub(crate) fn build(input: &Value, origins: &[(String, Value)]) -> Result<Option<Value>, String> {
    let assets = input["assets"].as_array().ok_or(HELD)?;
    let mut entries = Vec::new();
    for unit in units(input)? {
        if !asset_use_binding::enabled(unit)? {
            continue;
        }
        for kind in ["assetRefs", "soundSources"] {
            for (index, r) in unit[kind].as_array().into_iter().flatten().enumerate() {
                let found = assets
                    .iter()
                    .filter(|a| a["id"] == r["assetId"] && a["variantId"] == r["variantId"])
                    .collect::<Vec<_>>();
                let [current] = found.as_slice() else {
                    return Err(HELD.into());
                };
                if r["identityEvidence"] == evidence(current) {
                    continue;
                }
                let mut choices = Vec::new();
                for (id, snapshot) in origins {
                    if snapshot["input"]["source"]["sha256"] != input["source"]["sha256"] {
                        continue;
                    }
                    for old in snapshot["input"]["assets"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|a| evidence(a) == r["identityEvidence"])
                    {
                        if let Ok(value) = entry(input, unit, kind, index, old, id, &sha(snapshot))
                        {
                            choices.push(value);
                        }
                    }
                }
                if choices.len() != 1 {
                    return Err(HELD.into());
                }
                entries.push(choices.remove(0));
            }
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    let packet = sealed(
        json!({"version":VERSION,"sourceSha256":input["source"]["sha256"],"episodeId":input["episodeId"],"baseUnitBindings":bindings(input)?,"entries":entries}),
    );
    let mut checked = input.clone();
    checked[FIELD] = packet.clone();
    let draft = json!({"units":units(input)?});
    validation_view(&checked, &draft)?;
    Ok(Some(packet))
}
pub(crate) fn inherit(target: &mut Value, frozen: &Value) -> Result<(), String> {
    match frozen.get(FIELD) {
        None => {
            if target.get(FIELD).is_some() {
                return Err(HELD.into());
            }
        }
        Some(packet) => {
            if target.get(FIELD).is_some_and(|p| p != packet) {
                return Err(HELD.into());
            }
            target[FIELD] = packet.clone();
            validate(target)?;
        }
    }
    Ok(())
}
pub(crate) fn validate(input: &Value) -> Result<(), String> {
    let Some(packet) = input.get(FIELD) else {
        return Ok(());
    };
    if packet.as_object().is_none_or(|m| m.len() != 6)
        || packet["version"] != VERSION
        || !valid_seal(packet)
        || !hash(&packet["sourceSha256"])
        || packet["sourceSha256"] != input["source"]["sha256"]
        || packet["episodeId"] != input["episodeId"]
        || packet["baseUnitBindings"] != bindings(input)?
    {
        return Err(HELD.into());
    }
    let mut seen = HashSet::new();
    for e in packet["entries"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or(HELD)?
    {
        let unit = units(input)?
            .into_iter()
            .find(|u| u["id"] == e["unitId"])
            .ok_or(HELD)?;
        let kind = e["referenceKind"]
            .as_str()
            .filter(|s| ["assetRefs", "soundSources"].contains(s))
            .ok_or(HELD)?;
        let index = e["referenceIndex"].as_u64().ok_or(HELD)? as usize;
        if !hash(&e["originSnapshotSha256"])
            || unit[kind].as_array().is_none_or(|a| index >= a.len())
            || !seen.insert((e["unitId"].to_string(), kind.to_owned(), index))
            || entry(
                input,
                unit,
                kind,
                index,
                &e["assetBefore"],
                e["originWorkflowId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(HELD)?,
                e["originSnapshotSha256"].as_str().ok_or(HELD)?,
            )? != *e
        {
            return Err(HELD.into());
        }
    }
    Ok(())
}
/// Only unchanged original units can use the historical evidence. New model
/// replacements must have freshly materialized current evidence and pass strict validation.
pub(crate) fn validation_view(input: &Value, draft: &Value) -> Result<Value, String> {
    validate(input)?;
    let mut view = draft.clone();
    let assets = input["assets"].as_array().ok_or(HELD)?;
    for unit in view["units"].as_array_mut().ok_or(HELD)? {
        let original_sha = sha(unit);
        let unit_id = unit["id"].clone();
        if let Some(packet) = input.get(FIELD) {
            for e in packet["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["unitId"] == unit_id)
            {
                if original_sha != e["unitSha256"] {
                    continue;
                }
                let kind = e["referenceKind"].as_str().unwrap();
                let index = e["referenceIndex"].as_u64().unwrap() as usize;
                unit[kind][index]["identityEvidence"] = e["currentIdentityEvidence"].clone();
            }
        }
        asset_use_binding::validate_unit(unit, assets)?;
    }
    Ok(view)
}
pub(crate) fn attach_recovery(
    input: &mut Value,
    snapshot: &Value,
    prepared: &Value,
) -> Result<(), String> {
    if let Some(recovery) = prepared.get(RECOVERY) {
        if recovery.as_object().is_none_or(|m| m.len() != 4)
            || recovery["version"] != RECOVERY_VERSION
            || recovery["snapshotSha256"] != sha(snapshot)
            || !valid_seal(recovery)
            || snapshot["input"].get(FIELD).is_some()
        {
            return Err(HELD.into());
        }
        let mut frozen = snapshot["input"].clone();
        frozen[FIELD] = recovery["packet"].clone();
        inherit(input, &frozen)?;
    }
    Ok(())
}
pub(crate) fn recovery(snapshot: &Value, packet: &Value) -> Value {
    sealed(json!({"version":RECOVERY_VERSION,"snapshotSha256":sha(snapshot),"packet":packet}))
}

#[cfg(test)]
#[path = "storyboard_refinement_identity_tests.rs"]
mod tests;
