//! Native consumer of the frozen series-image-queue-v1 contract. Hash/identity
//! validation is not authorization, semantic review, or image adoption.
use crate::{
    module22_json as js,
    store::{s, Result},
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(crate) const FORMAT: &str = "series-module22-native/v1";
#[path = "module22_runtime.rs"]
mod runtime;
pub(crate) use runtime::{
    freeze_image_batch, freeze_image_batch_with_context, validate_image_snapshot, ImageBatchContext,
};
pub(crate) fn check(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("MODULE22_{code}"))
    }
}
pub(crate) fn rows<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    v[k].as_array().ok_or_else(|| format!("MODULE22_ARRAY:{k}"))
}
fn text(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|v| !v.trim().is_empty() && !v.contains('\0'))
}
fn positive(v: &Value) -> bool {
    v.as_u64()
        .is_some_and(|v| v > 0 && v <= 9_007_199_254_740_991)
}
fn sha(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn adopted(asset: &Value) -> bool {
    matches!(
        s(&asset["design"], "state"),
        "TEXT_ADOPTED" | "REFERENCE_ADOPTED"
    )
}
fn same(a: &Value, b: &Value) -> Result<bool> {
    Ok(js::stable(a)? == js::stable(b)?)
}
fn scene_same(a: &Value, b: &Value) -> bool {
    a["episode"] == b["episode"] && a["sceneId"] == b["sceneId"]
}
fn confirmation(v: &Value) -> bool {
    matches!(s(v, "kind"), "user" | "delegated") && text(&v["basis"])
}

pub(crate) fn validate_scope(scope: &Value, source: &Value) -> Result<()> {
    check(
        scope["version"] == "series-season-scope-v1"
            && text(&scope["id"])
            && positive(&scope["revision"])
            && positive(&scope["seasonNumber"]),
        "SEASON_SCOPE",
    )?;
    check(scope["status"] == "locked", "SEASON_NOT_LOCKED")?;
    check(
        sha(&scope["sourceHash"]) && scope["sourceHash"] == *source,
        "SEASON_SOURCE_HASH",
    )?;
    let read = rows(scope, "readingScope")?;
    let selected = rows(scope, "episodes")?;
    for catalog in [read, selected] {
        let mut numbers = HashSet::new();
        check(!catalog.is_empty(), "SEASON_EPISODES")?;
        for e in catalog {
            check(
                e["number"].as_i64().is_some() && numbers.insert(e["number"].to_string()),
                "SEASON_EPISODES",
            )?;
            let scenes = rows(e, "sceneIds")?;
            let mut seen = HashSet::new();
            check(
                !scenes.is_empty()
                    && scenes.iter().all(|v| {
                        text(v)
                            && !v.as_str().unwrap().contains(['\r', '\n'])
                            && seen.insert(v.to_string())
                    }),
                "SEASON_SCENES",
            )?;
        }
    }
    let start = read
        .iter()
        .position(|e| e["number"] == selected[0]["number"])
        .ok_or("MODULE22_SEASON_CONTINUITY")?;
    check(start + selected.len() <= read.len(), "SEASON_CONTINUITY")?;
    for (i, e) in selected.iter().enumerate() {
        check(
            e["number"] == read[start + i]["number"]
                && e["sceneIds"] == read[start + i]["sceneIds"],
            "SEASON_WHOLE_EPISODE",
        )?;
    }
    if scope["seasonNumber"] == 1 {
        check(start == 0, "SEASON_CONTINUITY")?;
    } else {
        let c = &scope["continuation"];
        check(
            sha(&c["previousScopeHash"])
                && text(&c["previousSeasonId"])
                && c["previousSeasonId"] != scope["id"]
                && c["nextEpisode"] == selected[0]["number"],
            "SEASON_CONTINUATION",
        )?;
    }
    check(
        scope["target"]["minSeconds"] == 7200
            && scope["target"]["maxSeconds"] == 9000
            && scope["target"]["planningSeconds"] == 8100,
        "SEASON_TARGET",
    )?;
    let end = &scope["ending"];
    let last = selected.last().unwrap();
    check(
        end["episode"] == last["number"]
            && Some(&end["sceneId"]) == rows(last, "sceneIds")?.last()
            && ["sourceRef", "closure", "hook"]
                .iter()
                .all(|k| text(&end[*k])),
        "SEASON_ENDING",
    )?;
    let min = end["estimate"]["minSeconds"].as_f64().unwrap_or(0.0);
    let max = end["estimate"]["maxSeconds"].as_f64().unwrap_or(0.0);
    check(
        min > 0.0 && max >= min && confirmation(&scope["confirmation"]),
        "SEASON_CONFIRMATION",
    )?;
    if let Some(ex) = scope.get("durationException") {
        check(
            text(&ex["basis"]) && confirmation(&ex["confirmation"]),
            "SEASON_DURATION_EXCEPTION",
        )?;
    } else {
        check(
            max >= 7200.0 && min <= 9000.0,
            "SEASON_ESTIMATE_OUTSIDE_TARGET",
        )?;
    }
    if let Some(refs) = end.get("sourceRefs") {
        let refs = refs.as_array().ok_or("MODULE22_SEASON_ENDING")?;
        let mut seen = HashSet::new();
        check(
            !refs.is_empty() && refs.iter().all(|v| text(v) && seen.insert(v.to_string())),
            "SEASON_ENDING",
        )?;
    }
    Ok(())
}
fn assert_use(scope: &Value, u: &Value) -> Result<()> {
    check(
        rows(scope, "episodes")?.iter().any(|e| {
            e["number"] == u["episode"]
                && e["sceneIds"]
                    .as_array()
                    .is_some_and(|a| a.contains(&u["sceneId"]))
        }),
        "SEASON_OUT_OF_SCOPE",
    )
}
fn valid_use(u: &Value) -> bool {
    u["episode"].as_i64().is_some() && text(&u["sceneId"]) && text(&u["purpose"])
}
fn filename(asset: &Value, registry: &Value) -> Result<String> {
    let prefix = match s(asset, "type") {
        "CH" => "角色",
        "SC" => "场景",
        "PR" => "道具",
        _ => return Err("MODULE22_IMAGE_FILENAME".into()),
    };
    let name = if registry["importProfile"] == "lingmo-v7" {
        s(asset, "importName")
    } else {
        s(asset, "name")
    };
    let file = format!("{prefix}-{name}.png");
    check(
        !name.is_empty()
            && file.len() <= 255
            && !file.chars().any(|c| c < ' ' || "\\/:*?\"<>|".contains(c)),
        "IMAGE_FILENAME",
    )?;
    Ok(file)
}
fn collect_dependencies(
    asset: &Value,
    ids: &HashMap<&str, &Value>,
    out: &mut Vec<Value>,
    visited: &mut HashSet<String>,
    active: &mut HashSet<String>,
) -> Result<()> {
    let id = s(asset, "id").to_owned();
    check(active.insert(id.clone()), "IMAGE_DESIGN_DEPENDENCY")?;
    if let Some(deps) = asset["design"].get("dependencies") {
        for d in deps.as_array().ok_or("MODULE22_IMAGE_DESIGN_DEPENDENCY")? {
            let parent = ids
                .get(s(d, "assetId"))
                .ok_or("MODULE22_IMAGE_DESIGN_DEPENDENCY")?;
            check(
                parent["revision"] == d["assetRevision"]
                    && adopted(parent)
                    && !active.contains(s(parent, "id")),
                "IMAGE_DESIGN_DEPENDENCY",
            )?;
            if !visited.insert(s(parent, "id").to_owned()) {
                continue;
            }
            let mut item = d.clone();
            check(item.is_object(), "IMAGE_DESIGN_DEPENDENCY")?;
            item["designHash"] = json!(js::hash(&parent["design"])?);
            item["designState"] = parent["design"]["state"].clone();
            out.push(item);
            collect_dependencies(parent, ids, out, visited, active)?;
        }
    }
    active.remove(&id);
    Ok(())
}
pub(crate) fn validate_batch(
    queue: &Value,
    batch: &Value,
    registry: &Value,
    snapshot_sha: &str,
    source_sha: &str,
) -> Result<Value> {
    let items = rows(queue, "items")?;
    check(
        queue["version"] == "series-image-queue-v1" && !items.is_empty(),
        "IMAGE_QUEUE",
    )?;
    let mut payload = queue.clone();
    payload
        .as_object_mut()
        .ok_or("MODULE22_IMAGE_QUEUE")?
        .remove("queueHash");
    check(
        sha(&queue["queueHash"]) && queue["queueHash"] == js::hash(&payload)?,
        "IMAGE_QUEUE_HASH",
    )?;
    check(
        registry["version"] == "series-assets-v1"
            && registry["id"] == queue["assetRegistry"]["id"]
            && registry["sourceHash"] == queue["sourceHash"]
            && positive(&queue["assetRegistry"]["revision"])
            && registry["revision"].as_u64() >= queue["assetRegistry"]["revision"].as_u64(),
        "IMAGE_REGISTRY",
    )?;
    check(
        queue["assetRegistry"]["sha256"] == snapshot_sha,
        "IMAGE_REGISTRY_HASH",
    )?;
    // Source bytes are always available in a complete package; do not weaken
    // nonseason packages merely because the legacy validator only needs them in season mode.
    check(queue["sourceHash"] == source_sha, "IMAGE_SOURCE_HASH")?;
    let scoped = [registry, queue]
        .iter()
        .any(|v| v.get("productionScope").is_some() || v.get("productionScopeHash").is_some());
    if scoped {
        for v in [registry, queue] {
            validate_scope(&v["productionScope"], &v["sourceHash"])?;
            check(
                v["productionScopeHash"] == js::hash(&v["productionScope"])?,
                "IMAGE_SCOPE_HASH",
            )?;
        }
        check(
            registry["productionScopeHash"] == queue["productionScopeHash"],
            "IMAGE_SCOPE_HASH",
        )?;
    }
    let tasks = rows(batch, "tasks")?;
    check(
        batch["version"] == "series-image-batch-v1"
            && batch["queueHash"] == queue["queueHash"]
            && batch["sourceHash"] == queue["sourceHash"]
            && !tasks.is_empty(),
        "IMAGE_BATCH",
    )?;
    check(
        if scoped {
            batch["productionScopeHash"] == queue["productionScopeHash"]
        } else {
            batch.get("productionScopeHash").is_none()
        },
        "IMAGE_SCOPE_HASH",
    )?;
    let mut ids = HashMap::new();
    for a in rows(registry, "assets")? {
        check(
            ids.insert(s(a, "id"), a).is_none() && !s(a, "id").is_empty(),
            "IMAGE_REGISTRY",
        )?;
    }
    let mut indexed = HashMap::new();
    let mut task_ids = HashSet::new();
    for item in items {
        let id = s(item, "assetId");
        check(
            id.starts_with('A')
                && id.len() >= 5
                && id[1..].bytes().all(|b| b.is_ascii_digit())
                && positive(&item["assetRevision"])
                && s(item, "itemId") == format!("{id}@{}", item["assetRevision"])
                && indexed.insert(s(item, "itemId"), item).is_none(),
            "IMAGE_ITEM",
        )?;
        check(
            item["approval"] == "design-and-uses-approved"
                && item["imageState"] == "not-generated"
                && adopted(item)
                && item["designHash"] == js::hash(&item["design"])?,
            "IMAGE_DESIGN_HASH",
        )?;
        let uses = rows(item, "uses")?;
        check(!uses.is_empty() && uses.iter().all(valid_use), "IMAGE_USES")?;
        if scoped {
            for u in uses {
                assert_use(&queue["productionScope"], u)?;
            }
        }
        let task = format!(
            "IMG-{}",
            &js::hash(&json!([
                registry["id"],
                queue["productionScopeHash"],
                item["itemId"],
                item["filename"],
                item["designHash"]
            ]))?[..24]
        );
        check(
            item["taskId"] == task && task_ids.insert(task),
            "IMAGE_TASK_ID",
        )?;
    }
    let mut seen = HashSet::new();
    for task in tasks {
        let item = indexed
            .get(s(task, "itemId"))
            .ok_or("MODULE22_IMAGE_TASK_ID")?;
        check(seen.insert(s(task, "itemId")), "IMAGE_TASK_ID")?;
        for key in [
            "taskId",
            "assetId",
            "assetRevision",
            "designHash",
            "filename",
        ] {
            check(task[key] == item[key], "IMAGE_TASK_BINDING")?;
        }
        let uses = rows(task, "uses")?;
        check(!uses.is_empty(), "IMAGE_TASK_USES")?;
        for u in uses {
            check(
                rows(item, "uses")?
                    .iter()
                    .any(|v| same(v, u).unwrap_or(false)),
                "IMAGE_TASK_USES",
            )?;
            if scoped {
                assert_use(&queue["productionScope"], u)?;
            }
        }
        let asset = ids
            .get(s(item, "assetId"))
            .ok_or("MODULE22_IMAGE_DESIGN_STALE")?;
        check(
            asset["revision"] == item["assetRevision"]
                && adopted(asset)
                && js::hash(&asset["design"])? == item["designHash"],
            "IMAGE_DESIGN_STALE",
        )?;
        check(asset["imageRequired"] != false, "IMAGE_NOT_REQUIRED")?;
        check(
            item["filename"] == filename(asset, registry)?,
            "IMAGE_FILENAME",
        )?;
        for key in ["key", "name", "type", "origin"] {
            check(same(&item[key], &asset[key])?, "IMAGE_IDENTITY")?;
        }
        if scoped {
            for u in rows(item, "uses")? {
                check(
                    rows(asset, "scopeUses")?.iter().any(|v| scene_same(v, u)),
                    "IMAGE_SOURCE_USES",
                )?;
            }
        }
        let mut deps = Vec::new();
        collect_dependencies(
            asset,
            &ids,
            &mut deps,
            &mut HashSet::new(),
            &mut HashSet::new(),
        )?;
        check(
            same(&json!(deps), &item["dependencies"])?,
            "IMAGE_DESIGN_DEPENDENCY",
        )?;
    }
    Ok(
        json!({"ok":true,"legacy":!scoped,"queueHash":queue["queueHash"],"productionScopeHash":queue["productionScopeHash"],"tasks":tasks.len(),"imageApproval":false,"validator":"rust-module22/v1","proof":"source bytes, explicit tasks, scope, immutable snapshot, current designs and dependencies; no generation or adoption authorization"}),
    )
}
pub(crate) fn batch(queue: &Value, items: &[Value]) -> Value {
    let mut v = json!({"version":"series-image-batch-v1","queueHash":queue["queueHash"],"sourceHash":queue["sourceHash"],"tasks":items.iter().map(|a|json!({"itemId":a["itemId"],"taskId":a["taskId"],"assetId":a["assetId"],"assetRevision":a["assetRevision"],"designHash":a["designHash"],"filename":a["filename"],"uses":a["uses"]})).collect::<Vec<_>>()});
    if let Some(scope) = queue.get("productionScopeHash") {
        v["productionScopeHash"] = scope.clone();
    }
    v
}

#[cfg(test)]
#[path = "module22_tests.rs"]
mod tests;
