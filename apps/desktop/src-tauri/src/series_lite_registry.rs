//! Native validators for frozen lite 0.1.2 (6dea8d6c57dde38f04a4d71f05f4c039004dd4a9).
//! Validation is structural and reference-bound, not creative approval or publication.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
#[path = "series_lite_registry_season.rs"]
mod season;
#[path = "series_lite_registry_unicode.rs"]
mod unicode;
pub use season::{
    assert_season_use, check_season_bundle, hash_season_scope, validate_season_scope,
};
pub const SOURCE_COMMIT: &str = "6dea8d6c57dde38f04a4d71f05f4c039004dd4a9";
pub const SOURCE_ASSETS_SHA256: &str =
    "57c69b4d4c32cc46aeb24077d119c2f9ece4b2b4184299f353c8407700b22425";
pub const SOURCE_SEASON_SHA256: &str =
    "5c1d8e574ae1b9b3153f0af59eb92d140d3c7975228b541d8f3ac1bc6ef2be8b";
pub const SOURCE_IMPORT_NAMES_SHA256: &str =
    "aa81ed702ab63d8b5d97306eeffcc698393d5a7c71f1ce1fe96a01d6d52986cd";
pub const UNICODE_TABLE_SHA256: &str =
    "9737e8e60f7bb63d38c82c0febc81d464bbf624954093916c29e5dc6d5cd2a96";
type Result<T> = std::result::Result<T, String>;
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
fn need(ok: bool, code: &str, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("{code}: {message}"))
    }
}
fn js_space(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn text(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim_matches(js_space).is_empty() && !s.contains(['\0', '\r']))
}
fn season_text(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim_matches(js_space).is_empty() && !s.contains('\0'))
}
fn label(v: &Value) -> bool {
    season_text(v) && !v.as_str().unwrap().contains(['\r', '\n'])
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}
fn integer(v: &Value) -> Option<f64> {
    number(v).filter(|n| n.fract() == 0.0)
}
fn safe(v: &Value) -> Option<i64> {
    integer(v).filter(|n| n.abs() <= MAX_SAFE).map(|n| n as i64)
}
fn positive(v: &Value) -> Option<i64> {
    safe(v).filter(|n| *n > 0)
}
fn sha(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn arr<'a>(v: &'a Value, code: &str) -> Result<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| format!("{code}: expected array"))
}
fn strings(v: &Value, p: fn(&Value) -> bool) -> bool {
    v.as_array().is_some_and(|a| {
        let mut seen = HashSet::new();
        !a.is_empty() && a.iter().all(|v| p(v) && seen.insert(v.as_str().unwrap()))
    })
}
fn same(a: &Value, b: &Value) -> bool {
    canonical(a).ok() == canonical(b).ok()
}
pub fn canonical(v: &Value) -> Result<String> {
    Ok(match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
        Value::Number(_) => {
            let n = number(v).ok_or("SEASON_JSON: nonfinite number")?;
            ryu_js::Buffer::new().format(n).to_string()
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(canonical)
                .collect::<Result<Vec<_>>>()?
                .join(",")
        ),
        Value::Object(o) => {
            let mut keys: Vec<_> = o.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| Ok(format!(
                        "{}:{}",
                        serde_json::to_string(k).map_err(|e| e.to_string())?,
                        canonical(&o[k])?
                    )))
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            )
        }
    })
}
fn name(v: &Value, v7: bool) -> bool {
    text(v)
        && v != "无"
        && v.as_str().is_some_and(|s| {
            if v7 {
                !s.contains(['\n', '\u{2028}', '\u{2029}'])
            } else {
                !s.contains(['\n', '、', '[', ']', '@'])
            }
        })
}
fn adopted(a: &Value) -> bool {
    matches!(
        a["design"]["state"].as_str(),
        Some("TEXT_ADOPTED" | "REFERENCE_ADOPTED")
    )
}
fn scene_key(v: &Value) -> Result<String> {
    canonical(&json!([v["episode"], v["sceneId"]]))
}
fn unit_key(v: &Value) -> Result<String> {
    canonical(&json!([v["episode"], v["sceneId"], v["unitId"]]))
}

pub fn validate_import_mappings(assets: &Value) -> Result<()> {
    let assets = arr(assets, "ASSET_IMPORT_NAME")?;
    let mut names = HashSet::new();
    let mut ordinals = HashSet::new();
    for a in assets {
        let n = a["importName"]
            .as_str()
            .ok_or("ASSET_IMPORT_NAME: name missing")?;
        need(
            !n.is_empty() && n != "无" && n.chars().all(unicode::python_alnum),
            "ASSET_IMPORT_NAME",
            "unsafe import name",
        )?;
        let prefix = match a["type"].as_str() {
            Some("CH") => "角色",
            Some("SC") => "场景",
            Some("PR") => "道具",
            _ => return Err("ASSET_IMPORT_NAME: type invalid".into()),
        };
        let ordinal =
            safe(&a["placeholderOrdinal"]).ok_or("ASSET_PLACEHOLDER_ORDINAL: integer required")?;
        need(
            (0..=5700).contains(&ordinal),
            "ASSET_PLACEHOLDER_ORDINAL",
            "ordinal out of range",
        )?;
        need(
            format!("{prefix}-{n}.png").len() <= 255,
            "ASSET_IMPORT_NAME",
            "filename over 255 UTF-8 bytes",
        )?;
        let folded = unicode::casefold(&unicode::nfkc(n));
        need(
            names.insert((prefix, folded)),
            "ASSET_IMPORT_COLLISION",
            "normalized import name collision",
        )?;
        need(
            ordinals.insert(ordinal),
            "ASSET_PLACEHOLDER_ORDINAL",
            "duplicate ordinal",
        )?;
    }
    Ok(())
}
pub fn registry_scope(registry: &Value, require_locked: bool) -> Result<Option<&Value>> {
    let Some(scope) = registry.get("productionScope") else {
        need(
            registry.get("productionScopeHash").is_none(),
            "ASSET_SCOPE_BINDING",
            "orphan scope hash",
        )?;
        return Ok(None);
    };
    validate_season_scope(scope, Some(&registry["sourceHash"]), None, require_locked)?;
    need(
        registry["productionScopeHash"] == hash_season_scope(scope)?,
        "ASSET_SCOPE_BINDING",
        "scope hash mismatch",
    )?;
    Ok(Some(scope))
}

/// Complete validateRegistry equivalent. It allows historical design dependencies;
/// actual consumers must additionally call check_design_dependencies.
pub fn validate_registry(registry: &Value, require_locked: bool) -> Result<()> {
    need(
        registry["version"] == "series-assets-v1"
            && text(&registry["id"])
            && positive(&registry["revision"]).is_some()
            && sha(&registry["sourceHash"]),
        "ASSET_REGISTRY",
        "invalid registry header",
    )?;
    let assets = arr(&registry["assets"], "ASSET_REGISTRY")?;
    let requests = arr(&registry["requests"], "ASSET_REGISTRY")?;
    need(
        registry.get("importProfile").is_none() || registry["importProfile"] == "lingmo-v7",
        "ASSET_IMPORT_PROFILE",
        "unknown import profile",
    )?;
    let scope = registry_scope(registry, require_locked)?;
    let v7 = registry["importProfile"] == "lingmo-v7";
    if v7 {
        validate_import_mappings(&registry["assets"])?;
    }
    let mut ids = BTreeMap::new();
    let mut keys = HashSet::new();
    let mut aliases = BTreeMap::new();
    for a in assets {
        if let Some(scope) = scope {
            let uses = arr(&a["scopeUses"], "ASSET_SCOPE_USES")?;
            let mut seen = HashSet::new();
            need(!uses.is_empty(), "ASSET_SCOPE_USES", "empty scope uses")?;
            for u in uses {
                need(
                    integer(&u["episode"]).is_some()
                        && text(&u["sceneId"])
                        && seen.insert(scene_key(u)?),
                    "ASSET_SCOPE_USES",
                    "invalid/duplicate scope use",
                )?;
                assert_season_use(scope, &u["episode"], Some(&u["sceneId"]))?;
            }
        }
        let id = a["id"].as_str().ok_or("ASSET_ID: missing ID")?;
        need(
            id.len() >= 5
                && id.starts_with('A')
                && id[1..].bytes().all(|b| b.is_ascii_digit())
                && positive(&a["revision"]).is_some()
                && !ids.contains_key(id),
            "ASSET_ID",
            "invalid/duplicate asset ID or revision",
        )?;
        need(
            matches!(a["type"].as_str(), Some("SC" | "CH" | "PR"))
                && name(&a["name"], v7)
                && a["key"]
                    == format!(
                        "{}{}",
                        a["name"].as_str().unwrap_or(""),
                        a["type"].as_str().unwrap_or("")
                    )
                && text(&a["status"])
                && keys.insert(a["key"].as_str().unwrap_or("")),
            "ASSET_RECORD",
            "invalid name/type/status/key",
        )?;
        need(
            matches!(a["origin"]["kind"].as_str(), Some("source" | "design"))
                && strings(&a["origin"]["sourceRefs"], text)
                && text(&a["origin"]["basis"]),
            "ASSET_ORIGIN",
            "missing source/design evidence",
        )?;
        need(
            matches!(
                a["design"]["state"].as_str(),
                Some("UNADOPTED" | "TEXT_ADOPTED" | "REFERENCE_ADOPTED" | "STALE")
            ) && (!adopted(a) || text(&a["design"]["text"])),
            "ASSET_DESIGN",
            "invalid adopted design",
        )?;
        let names = arr(&a["aliases"], "ASSET_ALIASES")?;
        let mut seen = HashSet::new();
        need(
            names
                .iter()
                .all(|v| name(v, v7) && seen.insert(v.as_str().unwrap())),
            "ASSET_ALIASES",
            "invalid aliases",
        )?;
        need(
            a.get("originalName").is_none() || text(&a["originalName"]),
            "ASSET_ALIASES",
            "invalid original name",
        )?;
        for n in std::iter::once(&a["name"]).chain(names.iter()) {
            let normalized = unicode::nfkc(n.as_str().unwrap());
            let key = (
                a["type"].as_str().unwrap(),
                unicode::lowercase_en(normalized.trim_matches(js_space)),
            );
            if let Some(old) = aliases.insert(key, id) {
                need(
                    old == id,
                    "ASSET_ALIAS_CONFLICT",
                    "name alias belongs to another ID",
                )?;
            }
        }
        ids.insert(id, a);
    }
    for (id, a) in &ids {
        if let Some(deps) = a["design"].get("dependencies") {
            let mut seen = HashSet::new();
            for d in arr(deps, "ASSET_DESIGN_DEPENDENCY")? {
                let target = d["assetId"].as_str().and_then(|k| ids.get(k));
                need(
                    d.is_object()
                        && target.is_some()
                        && positive(&d["assetRevision"]).is_some()
                        && text(&d["purpose"]),
                    "ASSET_DESIGN_DEPENDENCY",
                    "invalid dependency",
                )?;
                need(
                    positive(&d["assetRevision"]) <= positive(&target.unwrap()["revision"])
                        && d["assetId"] != *id
                        && seen.insert(d["assetId"].as_str().unwrap()),
                    "ASSET_DESIGN_DEPENDENCY",
                    "future/self/duplicate dependency",
                )?;
            }
        }
    }
    let mut checked = HashSet::new();
    for id in ids.keys() {
        visit_dependencies(id, &ids, false, &mut vec![], &mut checked)?;
    }
    let mut request_ids = HashSet::new();
    for r in requests {
        let target = r["assetId"].as_str().and_then(|k| ids.get(k));
        need(
            text(&r["id"])
                && request_ids.insert(r["id"].as_str().unwrap())
                && target.is_some()
                && positive(&r["assetRevision"]).is_some()
                && positive(&r["assetRevision"]) <= positive(&target.unwrap()["revision"]),
            "ASSET_REQUEST",
            "invalid request ID/target/revision",
        )?;
        need(
            r.get("state").is_none() || matches!(r["state"].as_str(), Some("active" | "withdrawn")),
            "ASSET_REQUEST",
            "unknown request state",
        )?;
        let uses = arr(&r["uses"], "ASSET_REQUEST")?;
        let withdrawn = r["state"] == "withdrawn";
        let mut seen = HashSet::new();
        need(
            if withdrawn {
                uses.is_empty()
            } else {
                !uses.is_empty()
            },
            "ASSET_REQUEST",
            "request use count invalid",
        )?;
        for u in uses {
            need(
                integer(&u["episode"]).is_some()
                    && text(&u["sceneId"])
                    && text(&u["unitId"])
                    && seen.insert(unit_key(u)?),
                "ASSET_REQUEST",
                "invalid duplicate request coordinate",
            )?;
            if let Some(scope) = scope {
                assert_season_use(scope, &u["episode"], Some(&u["sceneId"]))?;
            }
        }
        if withdrawn {
            need(
                text(&r["resolution"]["reason"]) && text(&r["resolution"]["sourceReview"]),
                "ASSET_REQUEST",
                "withdrawn source review missing",
            )?;
        }
    }
    crate::series_lite_asset_uses::validate_usage_annotations(registry)
}
fn visit_dependencies(
    id: &str,
    ids: &BTreeMap<&str, &Value>,
    current: bool,
    active: &mut Vec<String>,
    checked: &mut HashSet<String>,
) -> Result<()> {
    if checked.contains(id) {
        return Ok(());
    }
    need(
        !active.iter().any(|s| s == id),
        "ASSET_DESIGN_DEPENDENCY_CYCLE",
        &format!("{} → {id}", active.join(" → ")),
    )?;
    active.push(id.into());
    let a = ids
        .get(id)
        .ok_or("ASSET_DESIGN_DEPENDENCY: unknown asset")?;
    if let Some(deps) = a["design"].get("dependencies") {
        for d in arr(deps, "ASSET_DESIGN_DEPENDENCY")? {
            let target_id = d["assetId"]
                .as_str()
                .ok_or("ASSET_DESIGN_DEPENDENCY: target missing")?;
            let target = ids
                .get(target_id)
                .ok_or("ASSET_DESIGN_DEPENDENCY: unknown target")?;
            if current {
                need(
                    same(&d["assetRevision"], &target["revision"]),
                    "ASSET_DESIGN_DEPENDENCY_STALE",
                    &format!("{} → {target_id}", active.join(" → ")),
                )?;
                need(
                    adopted(target),
                    "ASSET_DESIGN_DEPENDENCY_UNADOPTED",
                    "ancestor not adopted",
                )?;
            }
            visit_dependencies(target_id, ids, current, active, checked)?;
        }
    }
    active.pop();
    checked.insert(id.into());
    Ok(())
}
pub fn check_design_dependencies(registry: &Value, asset_id: &str) -> Result<()> {
    validate_registry(registry, false)?;
    let ids = arr(&registry["assets"], "ASSET_REGISTRY")?
        .iter()
        .map(|a| (a["id"].as_str().unwrap(), a))
        .collect();
    visit_dependencies(asset_id, &ids, true, &mut vec![], &mut HashSet::new())
}
/// Check only declared consumption against the adopted registry. This does not
/// establish source coverage or prose quality; missing narrative still needs review.
pub fn check_asset_bindings(bundle: &Value, registry: &Value) -> Result<Value> {
    validate_registry(registry, false)?;
    let scope = registry_scope(registry, true)?;
    if let Some(scope) = scope {
        check_season_bundle(bundle, Some(scope), false)?;
    } else {
        need(
            bundle.get("productionScope").is_none() && bundle.get("productionScopeHash").is_none(),
            "ASSET_SCOPE_BINDING",
            "season bundle cannot use legacy registry",
        )?;
    }
    let v7 = bundle["exportProfile"] == "lingmo-v7";
    need(
        !v7 || registry["importProfile"] == "lingmo-v7",
        "ASSET_IMPORT_PROFILE",
        "v7 bundle requires mapped registry",
    )?;
    let binding = &bundle["assetRegistry"];
    need(
        bundle["sourceHash"] == registry["sourceHash"]
            && binding["id"] == registry["id"]
            && positive(&binding["revision"]).is_some()
            && positive(&binding["revision"]) <= positive(&registry["revision"]),
        "ASSET_REGISTRY_BINDING",
        "source/registry identity or revision differs",
    )?;
    let ids: BTreeMap<_, _> = arr(&registry["assets"], "ASSET_REGISTRY")?
        .iter()
        .map(|a| (a["id"].as_str().unwrap(), a))
        .collect();
    let projection: BTreeMap<_, _> = arr(&bundle["assets"], "ASSET_BUNDLE")?
        .iter()
        .map(|a| (a["key"].as_str().unwrap_or(""), a))
        .collect();
    let mut used = vec![];
    let mut seen_used = HashSet::new();
    let mut uses = vec![];
    let mut episode_numbers = vec![];
    let mut dependency_checked = HashSet::new();
    for e in arr(&bundle["episodes"], "ASSET_BUNDLE")? {
        episode_numbers.push(&e["number"]);
        for u in arr(&e["draft"]["units"], "ASSET_BUNDLE")? {
            for key in arr(&u["bindings"], "ASSET_BUNDLE")? {
                let projected = key.as_str().and_then(|key| projection.get(key));
                let current = projected
                    .and_then(|a| a["assetId"].as_str())
                    .and_then(|id| ids.get(id));
                let current =
                    current.ok_or("ASSET_REGISTRY_MISSING: binding not present in registry")?;
                let a = projected.unwrap();
                need(
                    same(&a["assetRevision"], &current["revision"]),
                    "ASSET_REGISTRY_STALE",
                    "binding uses old design revision",
                )?;
                for field in ["key", "name", "type", "status"] {
                    need(
                        a[field] == current[field],
                        "ASSET_REGISTRY_MISMATCH",
                        &format!("projection {field} differs"),
                    )?;
                }
                if v7 {
                    for field in ["importName", "placeholderOrdinal"] {
                        need(
                            same(&a[field], &current[field]),
                            "ASSET_REGISTRY_MISMATCH",
                            &format!("projection {field} differs"),
                        )?;
                    }
                }
                need(
                    adopted(current),
                    "ASSET_DESIGN_UNADOPTED",
                    "consumed design is not adopted",
                )?;
                if let Some(scope) = scope {
                    assert_season_use(scope, &e["number"], Some(&u["sceneId"]))?;
                }
                let id = current["id"].as_str().unwrap();
                visit_dependencies(id, &ids, true, &mut vec![], &mut dependency_checked)?;
                if seen_used.insert(id) {
                    used.push(id);
                }
                uses.push(json!({"episode":e["number"],"sceneId":u["sceneId"],"unitId":u["id"],"assetId":current["id"],"assetRevision":current["revision"],"key":key}));
            }
        }
    }
    let mut closed = vec![];
    let mut pending = vec![];
    for r in registry["requests"].as_array().unwrap() {
        if r["state"] == "withdrawn" {
            continue;
        }
        let required = r["uses"].as_array().unwrap();
        let inside = |u: &Value| episode_numbers.iter().any(|n| same(n, &u["episode"]));
        for target in required.iter().filter(|u| inside(u)) {
            let key = unit_key(target)?;
            need(
                uses.iter().any(|u| {
                    unit_key(u).ok().as_deref() == Some(key.as_str())
                        && u["assetId"] == r["assetId"]
                }),
                "ASSET_REQUEST_UNCONSUMED",
                "requested asset not consumed at target coordinate",
            )?;
        }
        if required.iter().all(inside) {
            closed.push(r["id"].clone());
        } else {
            pending.push(r["id"].clone());
        }
    }
    Ok(
        json!({"id":registry["id"],"currentRevision":registry["revision"],"bundleRevision":binding["revision"],"assets":used,"uses":uses,"closedRequests":closed,"pendingOutsideScope":pending,"proof":"已声明的总库、文字卡、版本与使用坐标一致；正文漏项和来源语义仍须内容复核"}),
    )
}
/// Full native adapter for the record-uses pure transition. Source semantic claims
/// still require record_uses_bound and independent reading, not this structural gate.
pub struct NativeRegistryValidator;
impl crate::series_lite_asset_uses::RegistryValidator for NativeRegistryValidator {
    fn validate(&mut self, registry: &Value, require_locked: bool) -> Result<()> {
        validate_registry(registry, require_locked)
    }
}
#[cfg(test)]
#[path = "series_lite_registry_tests.rs"]
mod tests;
