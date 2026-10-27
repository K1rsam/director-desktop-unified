use super::*;

pub fn hash_season_scope(scope: &Value) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(canonical(scope)?.as_bytes())
    ))
}
fn catalog(value: &Value, code: &str, allow_empty: bool) -> Result<Vec<Value>> {
    let values = arr(value, code)?;
    need(
        allow_empty || !values.is_empty(),
        code,
        "empty episode catalog",
    )?;
    let mut seen = HashSet::new();
    let mut out = vec![];
    for e in values {
        let number =
            safe(&e["number"]).ok_or_else(|| format!("{code}: episode number not safe integer"))?;
        need(
            e.is_object() && seen.insert(number) && strings(&e["sceneIds"], label),
            code,
            "invalid/duplicate episode or scene labels",
        )?;
        out.push(json!({"number":number,"sceneIds":e["sceneIds"]}));
    }
    Ok(out)
}
fn confirmation(value: &Value, code: &str) -> Result<()> {
    need(
        value.is_object()
            && matches!(value["kind"].as_str(), Some("user" | "delegated"))
            && season_text(&value["basis"]),
        code,
        "confirmation kind/basis missing",
    )
}
pub fn validate_season_scope(
    scope: &Value,
    source_hash: Option<&Value>,
    index: Option<&Value>,
    require_locked: bool,
) -> Result<()> {
    need(
        scope.is_object() && scope["version"] == "series-season-scope-v1",
        "SEASON_SCOPE",
        "invalid scope version",
    )?;
    need(
        label(&scope["id"])
            && positive(&scope["revision"]).is_some()
            && positive(&scope["seasonNumber"]).is_some(),
        "SEASON_SCOPE",
        "invalid scope identity/revision",
    )?;
    need(
        matches!(scope["status"].as_str(), Some("proposed" | "locked")),
        "SEASON_SCOPE",
        "invalid status",
    )?;
    need(
        !require_locked || scope["status"] == "locked",
        "SEASON_NOT_LOCKED",
        "scope is not locked",
    )?;
    need(
        sha(&scope["sourceHash"]) && source_hash.is_none_or(|h| *h == scope["sourceHash"]),
        "SEASON_SOURCE_HASH",
        "source hash mismatch",
    )?;
    let reading = catalog(&scope["readingScope"], "SEASON_READING_SCOPE", false)?;
    let selected = catalog(&scope["episodes"], "SEASON_EPISODES", false)?;
    let start = reading
        .iter()
        .position(|e| e["number"] == selected[0]["number"])
        .ok_or("SEASON_CONTINUITY: start outside reading scope")?;
    need(
        start + selected.len() <= reading.len(),
        "SEASON_CONTINUITY",
        "selection beyond reading scope",
    )?;
    for (i, e) in selected.iter().enumerate() {
        need(
            e["number"] == reading[start + i]["number"],
            "SEASON_CONTINUITY",
            "nonconsecutive source order",
        )?;
        need(
            e["sceneIds"] == reading[start + i]["sceneIds"],
            "SEASON_WHOLE_EPISODE",
            "partial/reordered episode",
        )?;
    }
    if positive(&scope["seasonNumber"]) == Some(1) {
        need(
            start == 0,
            "SEASON_CONTINUITY",
            "first season must start at first episode",
        )?;
    } else {
        let c = &scope["continuation"];
        need(
            c.is_object()
                && sha(&c["previousScopeHash"])
                && label(&c["previousSeasonId"])
                && c["previousSeasonId"] != scope["id"]
                && same(&c["nextEpisode"], &selected[0]["number"]),
            "SEASON_CONTINUATION",
            "continuation identity or next episode mismatch",
        )?;
    }
    let t = &scope["target"];
    need(
        t.is_object()
            && number(&t["minSeconds"]) == Some(7200.0)
            && number(&t["maxSeconds"]) == Some(9000.0)
            && number(&t["planningSeconds"]) == Some(8100.0),
        "SEASON_TARGET",
        "target must remain 7200/9000/8100",
    )?;
    let last = selected.last().unwrap();
    let ending = &scope["ending"];
    need(
        ending.is_object()
            && same(&ending["episode"], &last["number"])
            && last["sceneIds"].as_array().unwrap().last() == Some(&ending["sceneId"])
            && season_text(&ending["sourceRef"])
            && season_text(&ending["closure"])
            && season_text(&ending["hook"]),
        "SEASON_ENDING",
        "ending must bind last selected scene with source/closure/hook",
    )?;
    let min = number(&ending["estimate"]["minSeconds"]);
    let max = number(&ending["estimate"]["maxSeconds"]);
    need(
        ending["estimate"].is_object()
            && min.is_some_and(|n| n > 0.0)
            && max.is_some()
            && max >= min,
        "SEASON_ESTIMATE",
        "invalid estimate interval",
    )?;
    if let Some(refs) = ending.get("sourceRefs") {
        need(
            strings(refs, season_text),
            "SEASON_ENDING",
            "invalid ending refs",
        )?;
    }
    if scope["status"] == "locked" || scope.get("confirmation").is_some() {
        confirmation(&scope["confirmation"], "SEASON_CONFIRMATION")?;
    }
    if let Some(exception) = scope.get("durationException") {
        need(
            exception.is_object() && season_text(&exception["basis"]),
            "SEASON_DURATION_EXCEPTION",
            "missing exception basis",
        )?;
        confirmation(&exception["confirmation"], "SEASON_DURATION_EXCEPTION")?;
    }
    if scope["status"] == "locked" {
        need(
            scope.get("durationException").is_some()
                || (max.unwrap() >= 7200.0 && min.unwrap() <= 9000.0),
            "SEASON_ESTIMATE_OUTSIDE_TARGET",
            "estimate misses target without explicit exception",
        )?;
    }
    if let Some(index) = index {
        need(
            index.is_object() && index["version"] == "series-source-v1",
            "SEASON_INDEX",
            "source index version invalid",
        )?;
        need(
            index["sourceHash"] == scope["sourceHash"],
            "SEASON_SOURCE_HASH",
            "index source differs",
        )?;
        let indexed:Vec<Value>=arr(&index["episodes"],"SEASON_INDEX")?.iter().map(|e|json!({"number":e["number"],"sceneIds":e["scenes"].as_array().map(|s|s.iter().map(|v|v["id"].clone()).collect::<Vec<_>>())})).collect();
        let indexed = catalog(&json!(indexed), "SEASON_READING_SCOPE", false)?;
        if index.get("readingScope").is_some() || index.get("scopeHash").is_some() {
            need(
                catalog(&index["readingScope"], "SEASON_READING_SCOPE", false)? == reading,
                "SEASON_READING_SCOPE",
                "projected reading scope differs",
            )?;
            let h = hash_season_scope(scope)?;
            need(
                index["productionScope"].is_object()
                    && hash_season_scope(&index["productionScope"])? == h
                    && index["scopeHash"] == h,
                "SEASON_SCOPE_HASH",
                "projected scope hash differs",
            )?;
            need(
                indexed == selected,
                "SEASON_READING_SCOPE",
                "projected selected scenes differ",
            )?;
        } else {
            need(
                indexed == reading,
                "SEASON_READING_SCOPE",
                "full source catalog differs",
            )?;
            if let Some(embedded) = index.get("productionScope") {
                need(
                    embedded.is_object(),
                    "SEASON_SCOPE_HASH",
                    "invalid embedded scope",
                )?;
                let h = hash_season_scope(embedded)?;
                if h != hash_season_scope(scope)? {
                    need(
                        positive(&scope["seasonNumber"]).is_some_and(|n| n > 1)
                            && scope["continuation"]["previousScopeHash"] == h
                            && embedded["id"] == scope["continuation"]["previousSeasonId"],
                        "SEASON_SCOPE_HASH",
                        "embedded scope not current/previous",
                    )?;
                    let full = json!({"version":index["version"],"sourceHash":index["sourceHash"],"episodes":index["episodes"]});
                    validate_season_scope(embedded, Some(&scope["sourceHash"]), Some(&full), true)?;
                }
            }
        }
    }
    canonical(scope)?;
    Ok(())
}
pub fn assert_season_use(scope: &Value, episode: &Value, scene_id: Option<&Value>) -> Result<()> {
    validate_season_scope(scope, None, None, true)?;
    let selected = scope["episodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| same(&e["number"], episode));
    need(
        selected.is_some_and(|e| {
            scene_id.is_none_or(|id| e["sceneIds"].as_array().unwrap().contains(id))
        }),
        "SEASON_OUT_OF_SCOPE",
        "episode/scene outside selected scope",
    )
}
fn duration_ticks(v: &Value) -> Result<i64> {
    let n = number(v).ok_or("SEASON_DURATION: nonfinite duration")?;
    need(n > 0.0, "SEASON_DURATION", "duration must be positive")?;
    let scaled = n * 1e9;
    let ticks = scaled.round();
    need(
        ticks.is_finite()
            && ticks > 0.0
            && ticks <= MAX_SAFE
            && (scaled - ticks).abs() <= f64::EPSILON * scaled.abs().max(1.0) * 4.0,
        "SEASON_DURATION",
        "duration exceeds nine-decimal precision",
    )?;
    Ok(ticks as i64)
}
pub fn check_season_bundle(
    bundle: &Value,
    scope: Option<&Value>,
    require_complete: bool,
) -> Result<Value> {
    need(bundle.is_object(), "SEASON_BUNDLE", "bundle must be object")?;
    let Some(embedded) = bundle.get("productionScope") else {
        need(
            scope.is_none()
                && bundle.get("scopeHash").is_none()
                && bundle.get("seasonDelivery").is_none(),
            "SEASON_SCOPE_REQUIRED",
            "declared season cannot downgrade to legacy",
        )?;
        return Ok(json!({"legacy":true,"scopeHash":null,"coverage":null,"duration":null}));
    };
    validate_season_scope(embedded, Some(&bundle["sourceHash"]), None, true)?;
    let active = scope.unwrap_or(embedded);
    validate_season_scope(active, Some(&bundle["sourceHash"]), None, true)?;
    let hash = hash_season_scope(active)?;
    need(
        hash_season_scope(embedded)? == hash && bundle.get("scopeHash").is_none_or(|h| *h == hash),
        "SEASON_SCOPE_HASH",
        "bundle scope/hash differs",
    )?;
    if let Some(reading) = bundle.get("readingScope") {
        need(
            catalog(reading, "SEASON_READING_SCOPE", false)?
                == catalog(&active["readingScope"], "SEASON_READING_SCOPE", false)?,
            "SEASON_READING_SCOPE",
            "bundle reading scope drift",
        )?;
    }
    need(
        bundle.get("seasonDelivery").is_none()
            || matches!(bundle["seasonDelivery"].as_str(), Some("preview" | "final")),
        "SEASON_DELIVERY",
        "invalid delivery mode",
    )?;
    let selected = active["episodes"].as_array().unwrap();
    let mut covered = HashSet::new();
    let mut seen_episodes = HashSet::new();
    let mut previous_episode = None;
    let mut ticks: i64 = 0;
    let mut local = vec![];
    for e in arr(&bundle["episodes"], "SEASON_BUNDLE")? {
        let pos = selected
            .iter()
            .position(|s| same(&s["number"], &e["number"]))
            .ok_or("SEASON_OUT_OF_SCOPE: bundle episode outside scope")?;
        need(
            seen_episodes.insert(pos) && previous_episode.is_none_or(|p| pos > p),
            "SEASON_BUNDLE_ORDER",
            "episode order/duplicate",
        )?;
        previous_episode = Some(pos);
        let scene_ids = selected[pos]["sceneIds"].as_array().unwrap();
        let scenes = arr(&e["scenes"], "SEASON_BUNDLE")?;
        let mut previous_scene = None;
        let mut local_scenes = HashSet::new();
        for s in scenes {
            let p = scene_ids
                .iter()
                .position(|id| *id == s["id"])
                .ok_or("SEASON_OUT_OF_SCOPE: bundle scene outside scope")?;
            need(
                local_scenes.insert(p) && previous_scene.is_none_or(|q| p > q),
                "SEASON_BUNDLE_ORDER",
                "scene order/duplicate",
            )?;
            previous_scene = Some(p);
        }
        local.push(json!({"number":e["number"],"sceneIds":scenes.iter().map(|s|s["id"].clone()).collect::<Vec<_>>()}));
        let empty = json!([]);
        let units = e["draft"]
            .get("units")
            .filter(|u| !u.is_null())
            .unwrap_or(&empty);
        let mut previous_unit_scene = None;
        for u in arr(units, "SEASON_BUNDLE")? {
            let p = scene_ids
                .iter()
                .position(|id| *id == u["sceneId"])
                .ok_or("SEASON_OUT_OF_SCOPE: unit scene outside scope")?;
            need(
                u.is_object() && local_scenes.contains(&p),
                "SEASON_OUT_OF_SCOPE",
                "unit scene absent from local catalog",
            )?;
            need(
                previous_unit_scene.is_none_or(|q| p >= q),
                "SEASON_BUNDLE_ORDER",
                "unit scene order",
            )?;
            previous_unit_scene = Some(p);
            for shot in arr(&u["shots"], "SEASON_BUNDLE")? {
                ticks = ticks
                    .checked_add(duration_ticks(&shot["duration"])?)
                    .ok_or("SEASON_DURATION: total overflow")?;
                need(
                    ticks as f64 <= MAX_SAFE,
                    "SEASON_DURATION",
                    "unsafe total duration",
                )?;
                covered.insert((pos, p));
            }
        }
    }
    if let Some(local_scope) = bundle.get("scope") {
        need(
            same(
                &json!(catalog(local_scope, "SEASON_BUNDLE_SCOPE", true)?),
                &json!(local),
            ),
            "SEASON_BUNDLE_SCOPE",
            "bundle scope and actual local catalog differ",
        )?;
    }
    let mut selected_scenes = vec![];
    let mut covered_scenes = vec![];
    let mut missing_scenes = vec![];
    let mut covered_episodes = vec![];
    let mut missing_episodes = vec![];
    for (i, e) in selected.iter().enumerate() {
        let ids = e["sceneIds"].as_array().unwrap();
        let complete = ids
            .iter()
            .enumerate()
            .all(|(j, _)| covered.contains(&(i, j)));
        if complete {
            covered_episodes.push(e["number"].clone());
        } else {
            missing_episodes.push(e["number"].clone());
        }
        for (j, id) in ids.iter().enumerate() {
            let entry = json!({"episode":e["number"],"sceneId":id});
            selected_scenes.push(entry.clone());
            if covered.contains(&(i, j)) {
                covered_scenes.push(entry);
            } else {
                missing_scenes.push(entry);
            }
        }
    }
    let complete = missing_scenes.is_empty();
    let within = (7_200_000_000_000..=9_000_000_000_000).contains(&ticks);
    let exception = active.get("durationException");
    let mut duration = json!({"actualSeconds":ticks as f64/1e9,"target":active["target"],"status":if within{"within-target"}else if exception.is_some(){"exception-authorized"}else{"outside-target"}});
    if !within {
        if let Some(e) = exception {
            duration["exception"] = e.clone();
        }
    }
    if require_complete || bundle["seasonDelivery"] == "final" {
        need(
            complete,
            "SEASON_INCOMPLETE",
            "final delivery lacks actual scene shots",
        )?;
        need(
            within || exception.is_some(),
            "SEASON_DURATION_TARGET",
            "actual duration outside target without exception",
        )?;
    }
    Ok(
        json!({"legacy":false,"scopeHash":hash,"coverage":{"selectedEpisodes":selected.iter().map(|e|e["number"].clone()).collect::<Vec<_>>(),"coveredEpisodes":covered_episodes,"missingEpisodes":missing_episodes,"selectedScenes":selected_scenes,"coveredScenes":covered_scenes,"missingScenes":missing_scenes,"complete":complete},"duration":duration}),
    )
}
