use super::*;

/// Each prepared source is independently verified; one ambiguous episode never
/// prevents a different, explicitly mapped episode from returning its evidence.
pub(crate) fn inspect_sources(
    verifier: &dyn SourceVerifier,
    locked: &Value,
    prepared_sources: &[Value],
    known_speakers: &[String],
) -> Inspection {
    let mut out = Inspection::new();
    let episodes = match array(&locked["episodes"]) {
        Ok(v) => v,
        Err(e) => {
            out.gaps
                .push(gap("LITE_SUPPLY_SOURCE_SHAPE", None, None, vec![], e));
            return out;
        }
    };
    let mut numbers: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for e in episodes {
        if let Some(n) = verifier.original_episode_number(s(e, "sourceNumber")) {
            numbers.entry(n).or_default().push(s(e, "id"));
        }
    }
    let mut seen = BTreeSet::new();
    for prepared in prepared_sources {
        let id = s(&prepared["desktopBinding"], "episodeId");
        if !seen.insert(id.to_owned()) {
            out.gaps.push(gap(
                "LITE_SUPPLY_EPISODE_DUPLICATE",
                Some(id),
                None,
                vec![],
                "same episode supplied twice",
            ));
            continue;
        }
        if let Err(e) = verifier.verify(locked, prepared, known_speakers) {
            out.gaps
                .push(gap("LITE_SUPPLY_SOURCE_PROOF", Some(id), None, vec![], e));
            continue;
        }
        let Some(ep) = episodes.iter().find(|e| e["id"] == id) else {
            out.gaps.push(gap(
                "LITE_SUPPLY_EPISODE_UNKNOWN",
                Some(id),
                None,
                vec![],
                "episode absent from locked source",
            ));
            continue;
        };
        let Some(number) = verifier.original_episode_number(s(ep, "sourceNumber")) else {
            out.gaps.push(gap(
                "LITE_SUPPLY_ORIGINAL_NUMBER_UNRESOLVED",
                Some(id),
                None,
                vec![],
                format!(
                    "original label {:?}; internal ordinal {} is not used",
                    ep["sourceNumber"], ep["number"]
                ),
            ));
            continue;
        };
        if numbers.get(&number).is_none_or(|ids| ids.len() != 1) {
            out.gaps.push(gap(
                "LITE_SUPPLY_ORIGINAL_NUMBER_COLLISION",
                Some(id),
                None,
                numbers
                    .get(&number)
                    .into_iter()
                    .flatten()
                    .map(|s| s.to_string())
                    .collect(),
                "original episode integer is not unique",
            ));
            continue;
        }
        match map_episode(locked, prepared, ep, number) {
            Ok(v) => {
                let unassigned: Vec<String> = v["unassignedRows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["standardRowId"].as_str().map(str::to_owned))
                    .collect();
                if !unassigned.is_empty() {
                    out.gaps.push(gap("LITE_SUPPLY_SCENE_ASSIGNMENT_REQUIRED",Some(id),None,unassigned,"mapping evidence is available, but unassigned original rows are not silently attached to a scene"));
                }
                out.ready.push(v)
            }
            Err((code, coords, detail)) => {
                out.gaps.push(gap(&code, Some(id), None, coords, detail))
            }
        }
    }
    out
}
type MapError = (String, Vec<String>, String);
fn failure(code: &str, refs: Vec<String>, message: &str) -> MapError {
    (code.into(), refs, message.into())
}
fn map_episode(
    locked: &Value,
    prepared: &Value,
    episode: &Value,
    number: u32,
) -> std::result::Result<Value, MapError> {
    let source = &prepared["source"];
    crate::series_lite_bundle_standard::verify_standard_source(source)
        .map_err(|e| failure("LITE_SUPPLY_STANDARD_PROOF", vec![], &e))?;
    let binding = &prepared["desktopBinding"];
    let origin = binding["sourceRange"]["start"]
        .as_u64()
        .ok_or_else(|| failure("LITE_SUPPLY_RANGE", vec![], "original UTF16 offset missing"))?;
    let rows = array(&binding["rows"]).map_err(|e| failure("LITE_SUPPLY_RANGE", vec![], &e))?;
    let pkg = &source["standardPackage"];
    let evidence =
        array(&pkg["evidence"]).map_err(|e| failure("LITE_SUPPLY_STANDARD_PROOF", vec![], &e))?;
    let mut entries = array(&pkg["script"]["entries"])
        .map_err(|e| failure("LITE_SUPPLY_STANDARD_PROOF", vec![], &e))?
        .clone();
    entries.extend(pkg["annotations"].as_array().into_iter().flatten().cloned());
    for exclusion in pkg["exclusions"].as_array().into_iter().flatten() {
        let mut e = exclusion.clone();
        e["target"] = json!("EXCLUDE");
        e["sourceUnitIds"] = e["unitIds"].clone();
        entries.push(e);
    }
    let standard_rows: Vec<_> = source["sourceRows"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(source["contextRows"].as_array().into_iter().flatten())
        .collect();
    let scene_ids =
        array(&episode["sceneIds"]).map_err(|e| failure("LITE_SUPPLY_SCENE_MAP", vec![], &e))?;
    for id in scene_ids {
        if !locked["scenes"]
            .as_array()
            .is_some_and(|a| a.iter().filter(|s| s["id"] == *id).count() == 1)
        {
            return Err(failure(
                "LITE_SUPPLY_SCENE_UNKNOWN",
                vec![id.to_string()],
                "episode scene lacks unique original scene object",
            ));
        }
    }
    let mut mapped = Vec::new();
    let mut gaps = Vec::new();
    for e in entries {
        let key = format!("{}:{}", s(source, "key"), s(&e, "id"));
        if !standard_rows.iter().any(|r| r["id"] == key) {
            return Err(failure(
                "LITE_SUPPLY_STANDARD_ROW_MISSING",
                vec![key],
                "compiled entry has no normalized source row",
            ));
        }
        let ids = array(&e["sourceUnitIds"])
            .map_err(|msg| failure("LITE_SUPPLY_STANDARD_PROOF", vec![key.clone()], &msg))?;
        let mut spans = Vec::new();
        for id in ids {
            let u = evidence.iter().find(|u| u["id"] == *id).ok_or_else(|| {
                failure(
                    "LITE_SUPPLY_STANDARD_UNIT_MISSING",
                    vec![key.clone()],
                    "entry unit absent from evidence",
                )
            })?;
            spans.push((
                u["start"].as_u64().ok_or_else(|| {
                    failure(
                        "LITE_SUPPLY_RANGE",
                        vec![key.clone()],
                        "invalid evidence start",
                    )
                })?,
                u["end"].as_u64().ok_or_else(|| {
                    failure(
                        "LITE_SUPPLY_RANGE",
                        vec![key.clone()],
                        "invalid evidence end",
                    )
                })?,
            ));
        }
        if spans.is_empty() || spans.windows(2).any(|p| p[0].1 != p[1].0) {
            return Err(failure(
                "LITE_SUPPLY_STANDARD_SPAN",
                vec![key],
                "entry evidence is not contiguous",
            ));
        }
        let start = spans[0].0;
        let end = spans.last().unwrap().1;
        let a = origin
            .checked_add(start)
            .ok_or_else(|| failure("LITE_SUPPLY_RANGE", vec![key.clone()], "UTF16 overflow"))?;
        let b = origin
            .checked_add(end)
            .ok_or_else(|| failure("LITE_SUPPLY_RANGE", vec![key.clone()], "UTF16 overflow"))?;
        let relevant: Vec<_> = rows
            .iter()
            .filter(|r| {
                r["globalRange"]["start"].as_u64().is_some_and(|s| s < b)
                    && r["globalRange"]["end"].as_u64().is_some_and(|n| n > a)
            })
            .collect();
        let mut cursor = a;
        let mut desktop_rows = Vec::new();
        let mut scenes = BTreeSet::new();
        let mut unassigned = false;
        for row in relevant {
            let left = row["globalRange"]["start"].as_u64().unwrap().max(a);
            let right = row["globalRange"]["end"].as_u64().unwrap().min(b);
            if left != cursor {
                return Err(failure(
                    "LITE_SUPPLY_ROW_COVERAGE",
                    vec![key],
                    "desktop rows do not exactly cover standard entry",
                ));
            }
            cursor = right;
            desktop_rows.push(json!({"desktopRowId":row["desktopRowId"],"sceneId":row["sceneId"],"globalRange":{"start":left,"end":right},"localRange":{"start":left-origin,"end":right-origin}}));
            if let Some(scene) = row["sceneId"].as_str() {
                scenes.insert(scene.to_owned());
            } else {
                unassigned = true;
            }
        }
        if cursor != b {
            return Err(failure(
                "LITE_SUPPLY_ROW_COVERAGE",
                vec![key],
                "standard entry extends outside desktop rows",
            ));
        }
        if scenes.len() > 1 || unassigned && !scenes.is_empty() {
            return Err(failure(
                "LITE_SUPPLY_CROSS_SCENE_ENTRY",
                vec![key],
                "standard grouping crosses locked scene boundary; no guessed split",
            ));
        }
        let scene = scenes.iter().next().cloned();
        if scene
            .as_ref()
            .is_some_and(|id| !scene_ids.contains(&json!(id)))
        {
            return Err(failure(
                "LITE_SUPPLY_SCENE_UNKNOWN",
                vec![key],
                "row scene absent from locked episode",
            ));
        }
        if scene.is_none() {
            gaps.push(json!({"standardRowId":key,"target":e["target"],"kind":e["kind"],"reason":"original Desktop row has no scene; retain unassigned, no auto-first-scene assignment"}));
        }
        mapped.push(json!({"standardRowId":key,"target":e["target"],"kind":e["kind"],"sceneId":scene,"localRange":{"start":start,"end":end},"globalRange":{"start":a,"end":b},"desktopRows":desktop_rows}));
    }
    mapped.sort_by_key(|r| r["localRange"]["start"].as_u64());
    let scene_mappings:Vec<_>=scene_ids.iter().map(|id|json!({"desktopSceneId":id,"originalScene":locked["scenes"].as_array().and_then(|a|a.iter().find(|s|s["id"]==*id)).cloned(),"standardRowIds":mapped.iter().filter(|r|r["sceneId"]==*id).map(|r|r["standardRowId"].clone()).collect::<Vec<_>>()})).collect();
    Ok(
        json!({"version":VERSION,"kind":"source-map","sourceSha256":binding["sourceSha256"],"lockedSourceSha256":binding["lockedSourceSha256"],"desktopEpisodeId":episode["id"],"desktopOrdinal":episode["number"],"originalEpisodeLabel":episode["sourceNumber"],"episodeNumber":number,"source":source,"desktopBinding":binding,"backgroundSourceRows":prepared["backgroundSourceRows"],"rowMappings":mapped,"sceneMappings":scene_mappings,"unassignedRows":gaps,"readyForSceneBaseline":gaps.is_empty(),"semanticReview":"NOT_RUN"}),
    )
}
