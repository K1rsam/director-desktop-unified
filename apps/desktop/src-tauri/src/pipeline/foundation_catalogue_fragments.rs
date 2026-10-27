//! A lossless internal view of explicitly keyed, disjoint catalogue fragments.
//! Raw replies and array ordinals remain authoritative and are never rewritten.
use super::*;

const HELD: &str = "本批资产身份重复：同key片段不能安全归并";

fn parse(value: &Value) -> Result<AssetWire, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("F2目录结构无效：{e}"))
}

fn scene_pairs(value: &Value) -> Result<Vec<Value>, String> {
    let pairs = value["sceneRefs"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or(HELD)?;
    if value
        .get("scope")
        .is_some_and(|scope| scope["kind"] != "SCENES" || !scope.is_object())
        || pairs.iter().any(|pair| {
            pair.as_array().is_none_or(|v| {
                v.len() != 2 || v.iter().any(|s| s.as_str().is_none_or(str::is_empty))
            })
        })
    {
        return Err(HELD.into());
    }
    Ok(pairs.clone())
}

fn scalar_fields(value: &Value, omitted: &[&str]) -> Value {
    let mut result = value.as_object().cloned().unwrap_or_default();
    for key in omitted {
        result.remove(*key);
    }
    // An absent scope plus explicit sceneRefs is the existing SCENES contract.
    if !result.contains_key("scope") {
        result.insert("scope".into(), json!({"kind":"SCENES"}));
    }
    Value::Object(result)
}

fn union_array(target: &mut Value, incoming: &Value, field: &str) -> Result<(), String> {
    let Some(values) = incoming.get(field) else {
        return Ok(());
    };
    let values = values.as_array().ok_or(HELD)?;
    if target.get(field).is_none() {
        target[field] = json!([]);
    }
    let out = target[field].as_array_mut().ok_or(HELD)?;
    for value in values {
        if !out.contains(value) {
            out.push(value.clone());
        }
    }
    Ok(())
}

fn description_with_scene_labels(
    source: &LockedSource,
    fragments: &[&Value],
) -> Result<Option<String>, String> {
    let descriptions = fragments
        .iter()
        .map(|v| v.get("description"))
        .collect::<Vec<_>>();
    if descriptions.iter().all(|d| *d == descriptions[0]) {
        return Ok(descriptions[0].and_then(Value::as_str).map(str::to_owned));
    }
    let mut parts = Vec::new();
    for fragment in fragments {
        let Some(description) = fragment.get("description") else {
            continue;
        };
        let text = description.as_str().ok_or(HELD)?;
        let mut labels = Vec::new();
        for pair in scene_pairs(fragment)? {
            let scene = source
                .scenes
                .iter()
                .find(|s| {
                    s.id == pair[1].as_str().unwrap() && s.episode_id == pair[0].as_str().unwrap()
                })
                .ok_or(HELD)?;
            let episode = source
                .episodes
                .iter()
                .find(|e| e.id == scene.episode_id)
                .ok_or(HELD)?;
            labels.push(format!("{} · {}", episode.title, scene.title));
        }
        parts.push(format!("【{}】\n{}", labels.join("；"), text));
    }
    Ok(Some(parts.join("\n\n")))
}

fn validate_fragment(
    value: &Value,
    ordinal: usize,
    source: &LockedSource,
    batch: &Batch,
    prior: &FoundationProposal,
    allowed: &HashSet<String>,
    registered: &FoundationAsset,
) -> Result<(), String> {
    scene_pairs(value)?;
    let wire = parse(value)?;
    if wire.name.is_none()
        || wire.kind.is_none()
        || wire.episodes.is_none()
        || wire.preparation.is_none()
    {
        return Err(HELD.into());
    }
    if let Some(variants) = value.get("variants") {
        for variant in variants.as_array().ok_or(HELD)? {
            scene_pairs(variant)?;
            let key = variant["key"].as_str().ok_or(HELD)?;
            if !registered.variants.iter().any(|v| v.id == key)
                || variant["name"].as_str().is_none()
                || !variant["episodes"].is_array()
            {
                return Err(HELD.into());
            }
        }
    }
    // Validate each original independently, including its proposed parent graph.
    // An invalid fragment cannot borrow another fragment's valid references.
    let asset = build_asset(source, batch, &wire, prior, allowed, false, ordinal)?;
    let mut isolated = prior.clone();
    let index = isolated
        .assets
        .iter()
        .position(|a| a.id == registered.id)
        .ok_or(HELD)?;
    isolated.assets[index] = asset;
    validate_foundation(source, &isolated)?;
    Ok(())
}

fn merge_group(source: &LockedSource, fragments: &[&Value]) -> Result<Value, String> {
    const SETS: &[&str] = &[
        "aliases",
        "episodes",
        "sceneRefs",
        "sourceRefs",
        "variants",
        "description",
    ];
    let expected = scalar_fields(fragments[0], SETS);
    for (i, fragment) in fragments.iter().enumerate() {
        if scalar_fields(fragment, SETS) != expected {
            return Err(HELD.into());
        }
        let places = scene_pairs(fragment)?;
        for previous in &fragments[..i] {
            if fragment.get("description") != previous.get("description")
                && scene_pairs(previous)?.iter().any(|p| places.contains(p))
            {
                return Err(HELD.into());
            }
        }
    }
    let mut merged = (*fragments[0]).clone();
    for fragment in &fragments[1..] {
        for field in ["aliases", "episodes", "sceneRefs", "sourceRefs"] {
            union_array(&mut merged, fragment, field)?;
        }
        if let Some(variants) = fragment.get("variants") {
            if merged.get("variants").is_none() {
                merged["variants"] = json!([]);
            }
            for incoming in variants.as_array().ok_or(HELD)? {
                let key = incoming["key"].as_str().ok_or(HELD)?;
                let out = merged["variants"].as_array_mut().ok_or(HELD)?;
                if let Some(existing) = out.iter_mut().find(|v| v["key"] == key) {
                    let ignored = &["episodes", "sceneRefs", "sourceRefs"];
                    if scalar_fields(existing, ignored) != scalar_fields(incoming, ignored) {
                        return Err(HELD.into());
                    }
                    for field in ignored {
                        union_array(existing, incoming, field)?;
                    }
                } else {
                    out.push(incoming.clone());
                }
            }
        }
    }
    if let Some(description) = description_with_scene_labels(source, fragments)? {
        merged["description"] = json!(description);
    }
    Ok(merged)
}

/// Only repeated keys already registered in the supplied frozen basis qualify.
/// Nonduplicates remain byte-equivalent values with their original array index.
pub(super) fn catalogue_fragments(
    value: &Value,
    source: &LockedSource,
    batch: &Batch,
    prior: &FoundationProposal,
    allowed: &HashSet<String>,
) -> Result<Vec<(usize, AssetWire)>, String> {
    let assets = value["assets"].as_array().ok_or("F2目录assets须为数组")?;
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (ordinal, asset) in assets.iter().enumerate() {
        if let Some(key) = asset["key"].as_str() {
            groups.entry(key).or_default().push(ordinal);
        }
    }
    let mut out = Vec::with_capacity(assets.len());
    for (ordinal, asset) in assets.iter().enumerate() {
        let group = asset["key"]
            .as_str()
            .and_then(|key| groups.get(key))
            .filter(|g| g.len() > 1);
        let Some(group) = group else {
            out.push((ordinal, parse(asset)?));
            continue;
        };
        if group[0] != ordinal {
            continue;
        }
        let registered = prior
            .assets
            .iter()
            .find(|a| a.id == asset["key"].as_str().unwrap())
            .ok_or(HELD)?;
        for index in group {
            validate_fragment(
                &assets[*index],
                *index,
                source,
                batch,
                prior,
                allowed,
                registered,
            )?;
        }
        let fragments = group.iter().map(|i| &assets[*i]).collect::<Vec<_>>();
        let merged = merge_group(source, &fragments)?;
        validate_fragment(&merged, ordinal, source, batch, prior, allowed, registered)?;
        out.push((ordinal, parse(&merged)?));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        LockedSource,
        Batch,
        FoundationProposal,
        HashSet<String>,
        Value,
    ) {
        let source = prepare_source("第1集\n场景：大厅\n△门边有暗槽。\n第2集\n场景：宿舍\n△回忆大厅暗槽。\n第3集\n场景：教室\n△地面有粉笔。\n").unwrap();
        let scope = source
            .episodes
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let batch = Batch {
            pieces: whole_rows(&source, &scope),
            scope,
            fragment: None,
        };
        let prior = empty_foundation(&source);
        let allowed = source
            .rows
            .iter()
            .map(|r| r.id.clone())
            .collect::<HashSet<_>>();
        let pair = |n: usize| json!([source.scenes[n].episode_id, source.scenes[n].id]);
        let seed = json!({"name":"大厅", "type":"LOC", "description":"已有基线",
            "episodes":[batch.scope[0]], "sceneRefs":[pair(0)], "preparation":{"mode":"REFERENCE"},
            "variants":[{"name":"暗槽", "episodes":[batch.scope[0]], "sceneRefs":[pair(0)],
                "preparation":{"mode":"INLINE","note":"固定暗槽"}}]});
        let asset = build_asset(
            &source,
            &batch,
            &parse(&seed).unwrap(),
            &prior,
            &allowed,
            false,
            0,
        )
        .unwrap();
        let key = asset.id.clone();
        let variant = asset.variants[1].id.clone();
        let mut prior = prior;
        prior.assets.push(asset);
        let raw = json!({"assets":[
            {"key":key,"name":"大厅","type":"LOC","description":"实景原话。","aliases":["正厅"],
                "episodes":[batch.scope[0]],"sceneRefs":[pair(0)],"preparation":{"mode":"REFERENCE"}},
            {"key":key,"name":"大厅","type":"LOC","description":"回忆原话。","aliases":["旧大厅"],
                "episodes":[batch.scope[1]],"sceneRefs":[pair(1)],"preparation":{"mode":"REFERENCE"},
                "variants":[{"key":variant,"name":"暗槽","episodes":[batch.scope[1]],"sceneRefs":[pair(1)],
                    "preparation":{"mode":"INLINE","note":"固定暗槽"}}]},
            {"name":"粉笔","type":"PROP","episodes":[batch.scope[2]],"sceneRefs":[pair(2)],"preparation":{"mode":"REFERENCE"}}
        ]});
        (source, batch, prior, allowed, raw)
    }

    #[test]
    fn catalogue_fragments_merge_registered_disjoint_uses_without_loss_or_raw_mutation() {
        let (source, batch, prior, allowed, raw) = fixture();
        let frozen = raw.to_string();
        let out = catalogue_fragments(&raw, &source, &batch, &prior, &allowed).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, 0);
        assert_eq!(out[1].0, 2);
        let wire = &out[0].1;
        assert_eq!(wire.scene_refs.as_ref().unwrap().len(), 2);
        assert_eq!(wire.episodes.as_ref().unwrap(), &batch.scope[..2]);
        assert_eq!(wire.aliases.as_ref().unwrap(), &["正厅", "旧大厅"]);
        assert_eq!(
            wire.variants.as_ref().unwrap()[0].key.as_ref(),
            Some(&prior.assets[0].variants[1].id)
        );
        let description = wire.description.as_ref().unwrap();
        assert!(description.contains("实景原话。") && description.contains("回忆原话。"));
        assert!(
            description.contains(&source.scenes[0].title)
                && description.contains(&source.scenes[1].title)
        );
        let original_new = build_asset(
            &source,
            &batch,
            &parse(&raw["assets"][2]).unwrap(),
            &prior,
            &allowed,
            false,
            2,
        )
        .unwrap();
        let projected_new = build_asset(
            &source, &batch, &out[1].1, &prior, &allowed, false, out[1].0,
        )
        .unwrap();
        assert_eq!(original_new.id, projected_new.id);
        assert_eq!(raw.to_string(), frozen);
    }

    #[test]
    fn catalogue_fragments_fail_closed_on_identity_scope_graph_and_variant_conflicts() {
        for fault in [
            "name",
            "type",
            "preparation",
            "role",
            "relation",
            "scope_reason",
            "global",
            "missing_scenes",
            "overlap",
            "unknown_key",
            "new_variant",
            "bad_scene",
            "bad_source",
            "cycle",
        ] {
            let (source, batch, prior, allowed, mut raw) = fixture();
            match fault {
                "name" => raw["assets"][1]["name"] = json!("另一个大厅"),
                "type" => raw["assets"][1]["type"] = json!("PROP"),
                "preparation" => raw["assets"][1]["preparation"]["note"] = json!("不同准备"),
                "role" => raw["assets"][1]["role"] = json!("主角"),
                "relation" => raw["assets"][1]["relationNote"] = json!("不同关系"),
                "scope_reason" => {
                    raw["assets"][1]["scope"] = json!({"kind":"SCENES","reason":"另一个范围说明"})
                }
                "global" => {
                    raw["assets"][1]["scope"] = json!({"kind":"GLOBAL","reason":"不可广播"})
                }
                "missing_scenes" => {
                    raw["assets"][1]
                        .as_object_mut()
                        .unwrap()
                        .remove("sceneRefs");
                }
                "overlap" => {
                    raw["assets"][1]["sceneRefs"] = raw["assets"][0]["sceneRefs"].clone();
                    raw["assets"][1]["episodes"] = raw["assets"][0]["episodes"].clone();
                }
                "unknown_key" => {
                    raw["assets"][0]["key"] = json!("LOC_unknown");
                    raw["assets"][1]["key"] = json!("LOC_unknown");
                }
                "new_variant" => {
                    raw["assets"][1]["variants"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("key");
                }
                "bad_scene" => raw["assets"][1]["sceneRefs"][0][1] = json!("scene_missing"),
                "bad_source" => raw["assets"][1]["sourceRefs"] = json!(["row_missing"]),
                "cycle" => {
                    let id = raw["assets"][1]["variants"][0]["key"].clone();
                    raw["assets"][1]["variants"][0]["preparation"] =
                        json!({"mode":"DERIVE","fromKey":id});
                }
                _ => unreachable!(),
            }
            assert!(
                catalogue_fragments(&raw, &source, &batch, &prior, &allowed).is_err(),
                "{fault}"
            );
        }
    }

    #[test]
    fn catalogue_fragments_same_variant_only_unions_identical_semantics() {
        let (source, batch, prior, allowed, mut raw) = fixture();
        let mut first = raw["assets"][1]["variants"][0].clone();
        first["episodes"] = raw["assets"][0]["episodes"].clone();
        first["sceneRefs"] = raw["assets"][0]["sceneRefs"].clone();
        raw["assets"][0]["variants"] = json!([first]);
        let out = catalogue_fragments(&raw, &source, &batch, &prior, &allowed).unwrap();
        assert_eq!(out[0].1.variants.as_ref().unwrap().len(), 1);
        assert_eq!(
            out[0].1.variants.as_ref().unwrap()[0]
                .scene_refs
                .as_ref()
                .unwrap()
                .len(),
            2
        );
        raw["assets"][1]["variants"][0]["note"] = json!("不同的变体说明");
        assert!(catalogue_fragments(&raw, &source, &batch, &prior, &allowed).is_err());
    }

    #[test]
    fn catalogue_fragments_does_not_merge_names_or_unkeyed_new_assets() {
        let (source, batch, prior, allowed, mut raw) = fixture();
        raw["assets"][0].as_object_mut().unwrap().remove("key");
        raw["assets"][1].as_object_mut().unwrap().remove("key");
        let out = catalogue_fragments(&raw, &source, &batch, &prior, &allowed).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0, 1, 2]);
        raw["assets"][0]["key"] = json!("key_one");
        raw["assets"][1]["key"] = json!("key_two");
        assert_eq!(
            catalogue_fragments(&raw, &source, &batch, &prior, &allowed)
                .unwrap()
                .len(),
            3,
            "different keys remain untouched for the normal identity validator"
        );
    }
}
