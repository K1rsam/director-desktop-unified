//! Frozen season-handoff verify-next, restricted explicitly to supported v7 exports.
use super::*;
#[path = "series_lite_workspace_visible.rs"]
mod visible;
use crate::series_lite_registry::{check_season_bundle, hash_season_scope, validate_season_scope};

fn file(reads: &mut Reads, root: &Path, reference: &Value) -> Result<Vec<u8>> {
    fields(reference, &["path", "sha256"])?;
    let name = string(&reference["path"])?;
    need(
        text(&reference["path"])
            && !name.contains(['\\', ':'])
            && !Path::new(name).is_absolute()
            && name
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".."),
        "handoff path must be package-relative",
    )?;
    let path = reads.path(&root.join(name))?;
    need(
        path.starts_with(root) && path != root,
        "handoff file escapes package",
    )?;
    let raw = reads.read(&root.join(name))?;
    need(
        reference["sha256"] == hash(&raw),
        "handoff reference hash missing or changed",
    )?;
    Ok(raw)
}
fn indexed(index: &Value) -> Result<Value> {
    Ok(json!(array(&index["episodes"] )?.iter().map(|e| {
        let scenes=e["scenes"].as_array().ok_or("WORKSPACE: index scenes missing")?;
        Ok(json!({"number":e["number"],"sceneIds":scenes.iter().map(|s|s["id"].clone()).collect::<Vec<_>>()}))
    }).collect::<Result<Vec<_>>>()?))
}
fn strings(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.iter().all(text))
}
fn source_refs(v: &Value, refs: &[Value]) -> bool {
    v.as_array().is_some_and(|a| {
        !a.is_empty()
            && a.iter()
                .enumerate()
                .all(|(i, r)| text(r) && refs.contains(r) && !a[..i].contains(r))
    })
}
fn continuity(
    value: &Value,
    scope: &Value,
    tail: &Value,
    bundle: &Value,
    registry: &Value,
) -> Result<()> {
    need(
        value["version"] == "series-season-continuity-v1"
            && value["scopeHash"] == hash_season_scope(scope)?,
        "continuity scope differs",
    )?;
    let expected = json!({"episode":tail["episode"],"sceneId":tail["sceneId"],"unitId":tail["unitId"],"shotId":tail["shotId"]});
    need(
        same(&value["tail"], &expected),
        "continuity tail coordinates differ",
    )?;
    record(&value["review"])?;
    need(
        value["unresolved"].as_array().is_some_and(Vec::is_empty),
        "continuity unresolved issues remain",
    )?;
    need(
        strings(&value["deferred"])
            && value["states"].is_array()
            && value["openHooks"].as_array().is_some_and(|a| !a.is_empty()),
        "continuity state/hooks/deferred missing",
    )?;
    let mut refs = Vec::new();
    for e in array(&bundle["episodes"])? {
        for r in array(&e["source"]["sourceRows"])? {
            refs.push(r["id"].clone());
        }
    }
    let assets = array(&registry["assets"])?;
    let mut seen = Vec::new();
    for state in array(&value["states"])? {
        let asset = assets
            .iter()
            .find(|a| a["id"] == state["assetId"])
            .ok_or("WORKSPACE: unknown continuity asset")?;
        need(
            asset["type"] == "CH"
                && asset["revision"] == state["assetRevision"]
                && !seen.contains(&asset["id"])
                && text(&state["position"])
                && text(&state["emotion"])
                && source_refs(&state["sourceRefs"], &refs),
            "continuity character binding invalid",
        )?;
        seen.push(asset["id"].clone());
        need(
            ["knowledge", "heldProps", "injuries"]
                .iter()
                .all(|k| strings(&state[*k])),
            "continuity character details missing",
        )?;
        need(
            array(&state["heldProps"])?
                .iter()
                .all(|id| assets.iter().any(|a| a["id"] == *id && a["type"] == "PR")),
            "held prop not registered PR",
        )?;
    }
    need(
        !array(&value["states"])?.is_empty() || text(&value["stateBasis"]),
        "empty character states need basis",
    )?;
    for hook in array(&value["openHooks"])? {
        need(
            text(&hook["text"]) && source_refs(&hook["sourceRefs"], &refs),
            "hook lacks actual source anchor",
        )?;
    }
    Ok(())
}
pub(super) fn previous(reads: &mut Reads, state: &Value) -> Result<Value> {
    let scope = &state["productionScope"];
    let prior = &state["continuationEvidence"];
    let path_value = if prior.is_object() {
        &prior["path"]
    } else {
        &Value::Null
    };
    if scope.is_null() || scope["seasonNumber"].as_f64() == Some(1.0) {
        need(
            path_value.is_null(),
            "only continuation accepts previous handoff",
        )?;
        return Ok(Value::Null);
    }
    let requested = reads.root.join(string(path_value)?);
    let path = reads.path(&requested)?;
    let raw = reads.read(&requested)?;
    if !prior["sha256"].is_null() {
        need(
            prior["sha256"] == hash(&raw),
            "previous handoff bytes changed",
        )?;
    }
    let root = path
        .parent()
        .ok_or("WORKSPACE: handoff directory missing")?;
    let previous = parse(&raw)?;
    let report = verify_closed(reads, root, &previous)?;
    need(
        !report["resume"]["nextEpisode"].is_null(),
        "source ended; append-source transition unsupported",
    )?;
    let old_scope = parse(&file(reads, root, &previous["scope"])?)?;
    let old_index = parse(&file(reads, root, &previous["source"]["index"])?)?;
    need(
        scope["sourceHash"] == old_scope["sourceHash"],
        "cross-source continuation unsupported",
    )?;
    let source_view = json!({"version":old_index["version"],"sourceHash":old_index["sourceHash"],"episodes":old_index["episodes"]});
    validate_season_scope(
        scope,
        Some(&old_scope["sourceHash"]),
        Some(&source_view),
        true,
    )?;
    need(
        scope["id"] != old_scope["id"]
            && scope["seasonNumber"].as_f64()
                == old_scope["seasonNumber"].as_f64().map(|n| n + 1.0)
            && same(&scope["continuation"], &report["resume"]),
        "continuation does not match actual previous cursor",
    )?;
    let relative = path
        .strip_prefix(&reads.root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(
        json!({"path":relative,"sha256":hash(&raw),"previousScopeHash":report["scopeHash"],"nextEpisode":report["resume"]["nextEpisode"]}),
    )
}
fn verify_closed(reads: &mut Reads, root: &Path, input: &Value) -> Result<Value> {
    fields(
        input,
        &[
            "version",
            "kind",
            "source",
            "scope",
            "closed",
            "scopeHash",
            "resume",
            "actualTail",
            "reusableAssets",
        ],
    )?;
    need(
        input["version"] == "series-season-handoff-v1" && input["kind"] == "closed",
        "continuation requires actual closed handoff, not planned",
    )?;
    fields(&input["source"], &["original", "body", "index"])?;
    let original = file(reads, root, &input["source"]["original"])?;
    file(reads, root, &input["source"]["body"])?;
    let index = parse(&file(reads, root, &input["source"]["index"])?)?;
    let scope = parse(&file(reads, root, &input["scope"])?)?;
    need(
        index.is_object()
            && index.get("readingScope").is_none()
            && index.get("scopeHash").is_none(),
        "handoff requires full original source index",
    )?;
    validate_season_scope(&scope, Some(&json!(hash(&original))), Some(&index), true)?;
    need(
        same(&indexed(&index)?, &scope["readingScope"]),
        "handoff full source order differs",
    )?;
    let scope_hash = hash_season_scope(&scope)?;
    let last = array(&scope["episodes"])?
        .last()
        .ok_or("WORKSPACE: empty scope")?;
    let reading = array(&scope["readingScope"])?;
    let cursor = reading
        .iter()
        .position(|e| e["number"] == last["number"])
        .ok_or("WORKSPACE: ending outside reading scope")?;
    let resume = json!({"previousSeasonId":scope["id"],"previousScopeHash":scope_hash,"nextEpisode":reading.get(cursor+1).map(|e|e["number"].clone())});
    let closed = &input["closed"];
    fields(
        closed,
        &[
            "bundle",
            "sourceLock",
            "registry",
            "adoptedStoryboard",
            "continuity",
            "adoption",
            "images",
        ],
    )?;
    record(&closed["adoption"])?;
    let bundle_raw = file(reads, root, &closed["bundle"])?;
    let bundle = parse(&bundle_raw)?;
    need(
        bundle["testFixture"] == false,
        "synthetic marker cannot close production season",
    )?;
    if bundle["exportProfile"] != "lingmo-v7" {
        return Err("WORKSPACE_UNSUPPORTED_HANDOFF_PROFILE: legacy single-zone closed handoff is not ported; no downgrade".into());
    }
    let lock = file(reads, root, &closed["sourceLock"])?;
    let registry = parse(&file(reads, root, &closed["registry"])?)?;
    let markdown = file(reads, root, &closed["adoptedStoryboard"])?;
    let continuation = parse(&file(reads, root, &closed["continuity"])?)?;
    check_season_bundle(&bundle, Some(&scope), true)?;
    let mut gate = crate::series_lite_bundle_gate::NativeLegacyBundleGate {
        registry: Some(&registry),
        source_lock: Some(&lock),
        season_scope: Some(&scope),
    };
    let delivery = crate::series_lite_bundle::export_lingmo_v7(&bundle_raw, &mut gate)?;
    let visible = visible::parse(std::str::from_utf8(&markdown).map_err(|e| e.to_string())?)?;
    need(
        same(&visible, &delivery.projection),
        "adopted markdown differs from closed bundle",
    )?;
    let episode = array(&bundle["episodes"])?
        .last()
        .ok_or("WORKSPACE: no final episode")?;
    let unit = array(&episode["draft"]["units"])?
        .last()
        .ok_or("WORKSPACE: no final unit")?;
    let shot = array(&unit["shots"])?
        .last()
        .ok_or("WORKSPACE: no final shot")?;
    let tail = json!({"episode":episode["number"],"sceneId":unit["sceneId"],"unitId":unit["id"],"shotId":shot["id"],"exitState":unit["exitState"],"lastPicture":shot["picture"],"lastSound":shot["sound"]});
    continuity(&continuation, &scope, &tail, &bundle, &registry)?;
    let assets = array(&bundle["assets"])?;
    let mut used = Vec::new();
    for e in array(&bundle["episodes"])? {
        for u in array(&e["draft"]["units"])? {
            for key in array(&u["bindings"])? {
                if let Some(a) = assets.iter().find(|a| a["key"] == *key) {
                    used.push(a["assetId"].clone());
                }
            }
        }
    }
    let reusable:Vec<_>=array(&registry["assets"] )?.iter().filter(|a|used.contains(&a["id"])).map(|a|json!({"assetId":a["id"],"assetRevision":a["revision"],"key":a["key"],"designState":a["design"]["state"]})).collect();
    let mut image_keys = Vec::new();
    for image in array(&closed["images"])? {
        fields(
            image,
            &["assetId", "assetRevision", "state", "file", "adoption"],
        )?;
        need(
            reusable.iter().any(|a| {
                a["assetId"] == image["assetId"] && a["assetRevision"] == image["assetRevision"]
            }) && matches!(
                image["state"].as_str(),
                Some("candidate_pending_user" | "adopted")
            ) && !image_keys.contains(&image["assetId"]),
            "image state/revision binding invalid or duplicated",
        )?;
        image_keys.push(image["assetId"].clone());
        if image["state"] == "adopted" {
            record(&image["adoption"])?;
        } else {
            need(
                image.get("adoption").is_none(),
                "candidate cannot carry adoption",
            )?;
        }
        file(reads, root, &image["file"])?;
    }
    need(
        input["scopeHash"] == scope_hash
            && same(&input["resume"], &resume)
            && same(&input["actualTail"], &tail)
            && same(&input["reusableAssets"], &json!(reusable)),
        "handoff derived fields differ from actual files",
    )?;
    Ok(json!({"scopeHash":scope_hash,"resume":resume}))
}
