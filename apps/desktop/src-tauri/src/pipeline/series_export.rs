//! Deterministic v7 delivery: exact nanosecond windows, same-version identity,
//! independent local reader verification. No provider or filesystem side effects.
use super::super::{digest, LockedSource, SourceEpisode, SourceScene};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

pub const SECOND: u64 = 1_000_000_000;
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
pub fn ticks(value: &Value) -> Result<u64, String> {
    if !value.is_number() {
        return Err("SERIES_TIME_NOT_NUMBER".into());
    }
    let raw = value.to_string();
    if raw.starts_with('-') {
        return Err("SERIES_TIME_NEGATIVE".into());
    }
    let (mantissa, exp) = if let Some((m, e)) = raw.split_once(['e', 'E']) {
        (m, e.parse::<i32>().map_err(|_| "SERIES_TIME_EXPONENT")?)
    } else {
        (raw.as_str(), 0)
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let coefficient = digits.parse::<u64>().map_err(|_| "SERIES_TIME_RANGE")?;
    let power = 9 + exp - i32::try_from(fraction.len()).map_err(|_| "SERIES_TIME_RANGE")?;
    if !(-19..=19).contains(&power) {
        return Err("SERIES_TIME_PRECISION".into());
    }
    if power >= 0 {
        coefficient
            .checked_mul(10u64.checked_pow(power as u32).ok_or("SERIES_TIME_RANGE")?)
            .ok_or_else(|| "SERIES_TIME_RANGE".into())
    } else {
        let factor = 10u64
            .checked_pow((-power) as u32)
            .ok_or("SERIES_TIME_RANGE")?;
        if coefficient % factor != 0 {
            return Err("SERIES_TIME_PRECISION_MAX_9".into());
        }
        Ok(coefficient / factor)
    }
}
pub fn decimal(ticks: u64) -> String {
    if ticks % SECOND == 0 {
        return (ticks / SECOND).to_string();
    }
    format!("{}.{:09}", ticks / SECOND, ticks % SECOND)
        .trim_end_matches('0')
        .to_owned()
}
fn number(t: u64) -> Result<Value, String> {
    serde_json::from_str(&decimal(t)).map_err(|e| e.to_string())
}
pub fn timing(draft: &mut Value, cap: u64) -> Result<(), String> {
    if ![15, 30].contains(&cap) {
        return Err("SERIES_UNIT_LIMIT_MUST_BE_15_OR_30".into());
    }
    for unit in draft["units"]
        .as_array_mut()
        .ok_or("SERIES_UNITS_MISSING")?
    {
        let mut cursor = 0u64;
        for shot in unit["shots"].as_array_mut().ok_or("SERIES_SHOTS_MISSING")? {
            let duration = ticks(&shot["duration"])?;
            if duration == 0 {
                return Err("SERIES_ZERO_SHOT_DURATION".into());
            }
            for part in shot["dialogueParts"]
                .as_array()
                .ok_or("SERIES_DIALOGUE_MISSING")?
            {
                let start = ticks(&part["voiceStart"])?;
                let end = ticks(&part["voiceEnd"])?;
                if start >= end || end > duration {
                    return Err("SERIES_VOICE_WINDOW_OUTSIDE_SHOT".into());
                }
            }
            shot["startSeconds"] = number(cursor)?;
            cursor = cursor.checked_add(duration).ok_or("SERIES_TIME_OVERFLOW")?;
            shot["endSeconds"] = number(cursor)?;
        }
        let outer = 4u64.max(cursor.div_ceil(SECOND));
        if cursor == 0 || cursor > cap * SECOND || outer > cap {
            return Err(format!(
                "SERIES_UNIT_DURATION_LIMIT:{}:{}",
                s(unit, "id"),
                decimal(cursor)
            ));
        }
        unit["actualDurationSeconds"] = number(cursor)?;
        unit["duration"] = number(cursor)?;
        unit["containerSeconds"] = json!(outer);
        unit["timingProfile"] = json!("lingmo-v7-nanoseconds/v1");
    }
    Ok(())
}
fn source_kind(asset: &Value) -> &str {
    asset
        .get("kind")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .or_else(|| asset.get("type").and_then(Value::as_str))
        .unwrap_or("")
}
pub(super) fn is_voice_only(asset: &Value) -> bool {
    matches!(source_kind(asset), "SE" | "audio" | "voice" | "声音")
        || asset.pointer("/preparation/mode").and_then(Value::as_str) == Some("AUDIO")
}
fn ty(asset: &Value) -> Result<&'static str, String> {
    if is_voice_only(asset) {
        return Err(format!(
            "SERIES_VOICE_ONLY_IMAGE_FORBIDDEN:{}",
            s(asset, "id")
        ));
    }
    // This is the import image category only. The original source kind remains
    // intact, so a BEAST never acquires a human design template by conversion.
    match source_kind(asset) {
        "CH" | "character" | "人物" | "BEAST" => Ok("CH"),
        "SC" | "LOC" | "scene" | "location" | "场景" => Ok("SC"),
        "PR" | "PROP" | "prop" | "道具" => Ok("PR"),
        "OM" | "GROUP" | "other" | "群演" => Ok("OM"),
        _ => Err(format!("SERIES_NOT_IMAGE_ASSET:{}", s(asset, "id"))),
    }
}
fn collision_key(name: &str) -> String {
    name.nfkc().case_fold().collect()
}
fn allowed_name(name: &str) -> bool {
    !name.is_empty()
        && name != "无"
        && regex::Regex::new(r"^[\p{L}\p{N}]+$")
            .unwrap()
            .is_match(name)
}
fn generated_name(asset: &Value) -> String {
    let allowed = regex::Regex::new(r"^[\p{L}\p{N}]$").unwrap();
    asset["name"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| allowed.is_match(&c.to_string()))
        .collect()
}
fn filename_fits(filename: &str) -> bool {
    filename.len() <= 255 && filename.encode_utf16().count() <= 255
}
fn image_prefix(kind: &str) -> &'static str {
    match kind {
        "CH" => "角色",
        "SC" => "场景",
        "PR" => "道具",
        _ => "群演",
    }
}
struct NamePlan {
    index: usize,
    id: String,
    kind: &'static str,
    base: String,
    frozen: bool,
    assigned: Option<(String, String)>,
}
fn available_name(
    names: &HashSet<String>,
    files: &HashSet<String>,
    kind: &str,
    name: &str,
) -> bool {
    let filename = format!("{}-{name}.png", image_prefix(kind));
    !names.contains(&format!("{kind}:{}", collision_key(name)))
        && !files.contains(&collision_key(&filename))
}
fn reserve_name(
    plan: &mut NamePlan,
    name: String,
    names: &mut HashSet<String>,
    files: &mut HashSet<String>,
) {
    let filename = format!("{}-{name}.png", image_prefix(plan.kind));
    names.insert(format!("{}:{}", plan.kind, collision_key(&name)));
    files.insert(collision_key(&filename));
    plan.assigned = Some((name, filename));
}
fn disambiguated_name(plan: &NamePlan, attempt: usize) -> String {
    // Only the immutable formal identity determines this suffix. Formal names,
    // variants, source fields and image-parent relations are never rewritten.
    let identity = digest(&plan.id);
    let suffix = if attempt == 0 {
        format!("区分{}", &identity[..12])
    } else {
        format!("区分{}序{attempt}", &identity[..12])
    };
    let prefix = image_prefix(plan.kind);
    let mut name = String::new();
    for c in plan.base.chars() {
        let candidate = format!("{name}{c}");
        if !filename_fits(&format!("{prefix}-{candidate}{suffix}.png")) {
            break;
        }
        name.push(c);
    }
    format!("{name}{suffix}")
}
/// Allocate against the whole stable registry, never a per-episode subset.
/// Existing ordinals are immutable and new ones append after the high-water mark.
pub fn allocate_asset_identities(assets: &mut [Value]) -> Result<(), String> {
    allocate_asset_identities_from(assets, 0).map(|_| ())
}
pub fn allocate_asset_identities_from(assets: &mut [Value], floor: u64) -> Result<u64, String> {
    if floor > 5701 {
        return Err("SERIES_PLACEHOLDER_HIGH_WATER_RANGE".into());
    }
    let mut ids = HashSet::new();
    let mut ordinals = HashSet::new();
    let mut next = floor;
    for asset in assets.iter() {
        if s(asset, "id").is_empty() || !ids.insert(s(asset, "id").to_owned()) {
            return Err("SERIES_DUPLICATE_ASSET_ID".into());
        }
        if let Some(ordinal) = asset.get("placeholderOrdinal").filter(|v| !v.is_null()) {
            let ordinal = ordinal
                .as_u64()
                .filter(|v| *v <= 5700)
                .ok_or("SERIES_PLACEHOLDER_ORDINAL_RANGE")?;
            if !ordinals.insert(ordinal) {
                return Err("SERIES_PLACEHOLDER_ORDINAL_COLLISION".into());
            }
            next = next.max(ordinal + 1);
        }
    }
    let mut plans = Vec::new();
    for (index, asset) in assets.iter().enumerate() {
        if is_voice_only(asset) {
            continue;
        }
        let kind = ty(asset)?;
        let prefix = image_prefix(kind);
        let frozen = !s(asset, "importName").is_empty() || !s(asset, "filename").is_empty();
        let name = if !s(asset, "importName").is_empty() {
            s(asset, "importName").to_owned()
        } else if !s(asset, "filename").is_empty() {
            s(asset, "filename")
                .strip_prefix(&format!("{prefix}-"))
                .and_then(|v| v.strip_suffix(".png"))
                .ok_or_else(|| format!("SERIES_IMAGE_FILENAME_MAPPING_CHANGED:{}", s(asset, "id")))?
                .to_owned()
        } else {
            generated_name(asset)
        };
        if !allowed_name(&name) {
            return Err(format!("SERIES_IMPORT_NAME_INVALID:{}", s(asset, "id")));
        }
        let filename = format!("{prefix}-{name}.png");
        if frozen && !filename_fits(&filename) {
            return Err(format!("SERIES_IMAGE_FILENAME_TOO_LONG:{}", s(asset, "id")));
        }
        if !s(asset, "filename").is_empty() && asset["filename"] != filename {
            return Err(format!(
                "SERIES_IMAGE_FILENAME_MAPPING_CHANGED:{}",
                s(asset, "id")
            ));
        }
        plans.push(NamePlan {
            index,
            id: s(asset, "id").to_owned(),
            kind,
            base: name,
            frozen,
            assigned: None,
        });
    }
    // Sort the allocation plan, not the registry. First allocation is stable
    // even if an unrelated database query presents rows in a different order.
    plans.sort_by(|a, b| a.id.cmp(&b.id));
    let mut names = HashSet::new();
    let mut filenames = HashSet::new();
    let mut counts = HashMap::<String, usize>::new();
    for plan in &mut plans {
        if plan.frozen {
            if !available_name(&names, &filenames, plan.kind, &plan.base) {
                return Err(format!(
                    "SERIES_IMPORT_NAME_COLLISION:{}:{}",
                    plan.id, plan.base
                ));
            }
            let name = plan.base.clone();
            reserve_name(plan, name, &mut names, &mut filenames);
        } else {
            *counts
                .entry(format!("{}:{}", plan.kind, collision_key(&plan.base)))
                .or_default() += 1;
        }
    }
    // Preserve all available readable names before assigning exceptional
    // suffixes, so a generated suffix cannot steal another asset's real name.
    for plan in &mut plans {
        if !plan.frozen
            && counts.get(&format!("{}:{}", plan.kind, collision_key(&plan.base))) == Some(&1)
            && filename_fits(&format!("{}-{}.png", image_prefix(plan.kind), plan.base))
            && available_name(&names, &filenames, plan.kind, &plan.base)
        {
            let name = plan.base.clone();
            reserve_name(plan, name, &mut names, &mut filenames);
        }
    }
    let bound = plans.len();
    for plan in &mut plans {
        if plan.assigned.is_none() {
            for attempt in 0..=bound {
                let name = disambiguated_name(plan, attempt);
                if available_name(&names, &filenames, plan.kind, &name) {
                    reserve_name(plan, name, &mut names, &mut filenames);
                    break;
                }
            }
            if plan.assigned.is_none() {
                return Err("SERIES_IMPORT_NAME_RESERVATION_EXHAUSTED".into());
            }
        }
    }
    let needed = plans
        .iter()
        .filter(|p| {
            assets[p.index]
                .get("placeholderOrdinal")
                .is_none_or(Value::is_null)
        })
        .count() as u64;
    if next.checked_add(needed).is_none_or(|end| end > 5701) {
        return Err("SERIES_PLACEHOLDER_ORDINAL_RANGE".into());
    }
    // Commit only after the whole plan is valid; an error must not leave a
    // partially assigned caller-owned registry.
    for plan in plans {
        let asset = &mut assets[plan.index];
        if asset.get("placeholderOrdinal").is_none_or(Value::is_null) {
            asset["placeholderOrdinal"] = json!(next);
            next += 1;
        }
        let (name, filename) = plan.assigned.unwrap();
        asset["importName"] = json!(name);
        asset["filename"] = json!(filename);
    }
    Ok(next)
}
pub fn index_assets(assets: &[Value], used: &HashSet<String>) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    let mut names = HashSet::new();
    let mut filenames = HashSet::new();
    let mut ordinals = HashSet::new();
    for asset in assets {
        if !used.contains(s(asset, "id")) {
            continue;
        }
        let kind = ty(asset)?;
        let imported = s(asset, "importName");
        let name = if imported.is_empty() {
            s(asset, "name")
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        } else {
            imported.into()
        };
        if !allowed_name(&name) {
            return Err(format!("SERIES_IMPORT_NAME_INVALID:{}", s(asset, "id")));
        }
        let prefix = match kind {
            "CH" => "角色",
            "SC" => "场景",
            "PR" => "道具",
            _ => "群演",
        };
        let filename = if s(asset, "filename").is_empty() {
            format!("{prefix}-{name}.png")
        } else {
            s(asset, "filename").into()
        };
        if filename.len() > 255
            || filename.encode_utf16().count() > 255
            || filename != format!("{prefix}-{name}.png")
        {
            return Err("SERIES_IMAGE_FILENAME_INVALID".into());
        }
        if !names.insert(format!("{kind}:{}", collision_key(&name)))
            || !filenames.insert(collision_key(&filename))
        {
            return Err(format!("SERIES_IMPORT_NAME_COLLISION:{}", name));
        }
        let ordinal = asset["placeholderOrdinal"]
            .as_u64()
            .ok_or("SERIES_PLACEHOLDER_ORDINAL_NOT_ALLOCATED")?;
        if ordinal > 5700 || !ordinals.insert(ordinal) {
            return Err("SERIES_PLACEHOLDER_ORDINAL_COLLISION".into());
        }
        rows.push(json!({"id":asset["id"],"variantId":asset["variantId"],"name":name,"formalName":asset["name"],"importName":name,"type":kind,"token":format!("{name}{kind}"),"filename":filename,"placeholderOrdinal":ordinal,"assetRevision":asset.get("designRevision").or_else(||asset.get("version")).cloned().unwrap_or(json!(1)),"adoptedDesign":asset["adoptedDesign"]}));
    }
    if rows.len() != used.len() {
        return Err("SERIES_IMAGE_BINDING_UNKNOWN_ASSET".into());
    }
    Ok(rows)
}
fn strings(v: &Value, k: &str) -> Result<Vec<String>, String> {
    v[k].as_array()
        .ok_or_else(|| format!("SERIES_MISSING_ARRAY:{k}"))?
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("SERIES_BAD_STRING:{k}"))
        })
        .collect()
}
pub fn validate_image_bindings(draft: &Value, assets: &[Value]) -> Result<HashSet<String>, String> {
    let mut used = HashSet::new();
    for u in draft["units"].as_array().ok_or("SERIES_UNITS_MISSING")? {
        if strings(u, "imageBindings")?.len() > 30 {
            return Err("SERIES_IMAGE_BINDINGS_LIMIT_30".into());
        }
        let mut unit_seen = HashSet::new();
        for id in strings(u, "imageBindings")? {
            if !unit_seen.insert(id.clone()) {
                return Err("SERIES_DUPLICATE_IMAGE_BINDING".into());
            }
            let a = assets
                .iter()
                .find(|a| a["id"] == id)
                .ok_or("SERIES_IMAGE_BINDING_UNKNOWN_ASSET")?;
            ty(a)?;
            if a["materialRole"] == "MATERIAL_SOURCE_ONLY" {
                return Err("SERIES_READONLY_DESIGN_ANCESTOR_NOT_IMAGE_USE".into());
            }
            if is_voice_only(a) {
                return Err("SERIES_VOICE_ONLY_IMAGE_FORBIDDEN".into());
            }
            if !u["assetRefs"].as_array().into_iter().flatten().any(|r| {
                r["assetId"] == id
                    && r["variantId"] == a["variantId"]
                    && !matches!(s(r, "use"), "audio" | "video")
            }) {
                return Err("SERIES_IMAGE_WITHOUT_VISUAL_REFERENCE".into());
            }
            used.insert(id);
        }
    }
    Ok(used)
}
fn field(out: &mut String, label: &str, value: &str) {
    out.push('[');
    out.push_str(label);
    out.push_str("] ");
    let mut lines = value.split('\n');
    out.push_str(lines.next().unwrap_or(""));
    out.push('\n');
    for line in lines {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
}
fn names_for(unit: &Value, rows: &HashMap<String, &Value>, kind: &str) -> Result<String, String> {
    let mut items = Vec::new();
    for id in strings(unit, "imageBindings")? {
        let row = rows.get(&id).ok_or("SERIES_BINDING_INDEX_MISSING")?;
        if row["type"] == kind || (kind == "CH" && row["type"] == "OM") {
            items.push(format!("{}@[{}]", s(row, "formalName"), s(row, "token")));
        }
    }
    Ok(if items.is_empty() {
        "无".into()
    } else {
        items.join("、")
    })
}
fn render_unit(
    unit: &Value,
    style: &str,
    rows: &HashMap<String, &Value>,
) -> Result<String, String> {
    let mut out = String::new();
    field(&mut out, "通用提示词", &super::frame_render::export_style(unit, style)?);
    field(&mut out, "单元标题", s(unit, "title"));
    field(&mut out, "单元演出", s(unit, "unitDirection"));
    field(&mut out, "场景", &names_for(unit, rows, "SC")?);
    field(&mut out, "本单元道具", &names_for(unit, rows, "PR")?);
    field(&mut out, "本单元出场人物", &names_for(unit, rows, "CH")?);
    field(&mut out, "场景状态", s(unit, "scene"));
    field(&mut out, "本单元光影", s(unit, "lighting"));
    field(&mut out, "人物状态与站位前提", s(unit, "initialState"));
    for (i, shot) in unit["shots"]
        .as_array()
        .ok_or("SERIES_SHOTS_MISSING")?
        .iter()
        .enumerate()
    {
        out.push_str(&format!(
            "分镜{}｜{}-{}s｜{}s\n",
            i + 1,
            decimal(ticks(&shot["startSeconds"])?),
            decimal(ticks(&shot["endSeconds"])?),
            decimal(ticks(&shot["duration"])?)
        ));
        for (label, key) in [
            ("机位与运镜", "camera"),
            ("画面", "picture"),
            ("光影", "lighting"),
            ("声音", "sound"),
        ] {
            field(&mut out, label, s(shot, key));
        }
        for part in shot["dialogueParts"]
            .as_array()
            .ok_or("SERIES_DIALOGUE_MISSING")?
        {
            let start = ticks(&shot["startSeconds"])? + ticks(&part["voiceStart"])?;
            let end = ticks(&shot["startSeconds"])? + ticks(&part["voiceEnd"])?;
            field(
                &mut out,
                "人声",
                &format!(
                    "{}-{}秒｜{}｜{}｜{}｜{}",
                    decimal(start),
                    decimal(end),
                    s(part, "speaker"),
                    s(part, "channel"),
                    s(part, "presentation"),
                    s(part, "delivery")
                ),
            );
            field(&mut out, "原话", s(part, "text"));
        }
        field(&mut out, "转场", s(shot, "transition"));
    }
    field(&mut out, "结束状态", s(unit, "exitState"));
    Ok(out)
}
fn scene_number(scene: &SourceScene, episode: &SourceEpisode, position: usize) -> String {
    let re = regex::Regex::new(r"^\s*(\d+\s*-\s*\d+)(?:\s|$|[：:])").unwrap();
    re.captures(&scene.title)
        .map(|c| c[1].replace(' ', ""))
        .unwrap_or_else(|| format!("{}-{}", episode.number, position + 1))
}
fn escape_name(s: &str) -> String {
    s.replace('\\', "\\\\").replace('、', "\\、")
}
pub fn export(
    source: &LockedSource,
    episode: &SourceEpisode,
    draft: &Value,
    assets: &[Value],
    style: &str,
) -> Result<Value, String> {
    let used = validate_image_bindings(draft, assets)?;
    let index = index_assets(assets, &used)?;
    let rows: HashMap<String, &Value> = index.iter().map(|v| (s(v, "id").into(), v)).collect();
    let units = draft["units"].as_array().ok_or("SERIES_UNITS_MISSING")?;
    let mut text = format!("**【标准化剧本】**\n第{}集\n", episode.number);
    let mut unit_index = Vec::new();
    let mut scenes_seen = HashSet::new();
    let mut emitted = 0;
    for (position, scene) in source
        .scenes
        .iter()
        .filter(|s| s.episode_id == episode.id)
        .enumerate()
    {
        let local: Vec<_> = units.iter().filter(|u| u["sceneId"] == scene.id).collect();
        if local.is_empty() {
            return Err(format!("SERIES_SCENE_MISSING:{}", scene.id));
        }
        let number = scene_number(scene, episode, position);
        if !scenes_seen.insert(number.clone()) {
            return Err("SERIES_SCENE_NUMBER_COLLISION".into());
        }
        text.push_str(&number);
        text.push('\n');
        for (label, kind) in [("场景", "SC"), ("人物", "CH"), ("道具", "PR")] {
            let mut seen = HashSet::new();
            let mut names = Vec::new();
            for unit in &local {
                for id in strings(unit, "imageBindings")? {
                    let row = rows[&id];
                    if (row["type"] == kind || (kind == "CH" && row["type"] == "OM"))
                        && seen.insert(id)
                    {
                        names.push(escape_name(s(row, "name")));
                    }
                }
            }
            text.push_str(&format!(
                "{label}：{}\n",
                if names.is_empty() {
                    "无".into()
                } else {
                    names.join("、")
                }
            ));
        }
        for (fragment, unit) in local.iter().enumerate() {
            let body = render_unit(unit, style, &rows)?;
            text.push_str(&format!(
                "片段{}\n时长：{}秒\n",
                fragment + 1,
                unit["containerSeconds"]
            ));
            text.push_str(&body);
            let tokens: Vec<_> = strings(unit, "imageBindings")?
                .iter()
                .map(|id| rows[id]["token"].clone())
                .collect();
            unit_index.push(json!({"unitId":unit["id"],"episode":episode.number,"sceneId":number,"sourceSceneId":scene.id,"sourceSceneTitle":scene.title,"fragment":fragment+1,"imageTokens":tokens,"actualSeconds":unit["actualDurationSeconds"],"containerSeconds":unit["containerSeconds"],"bodySha256":digest(&body)}));
            emitted += 1;
        }
    }
    if emitted != units.len() {
        return Err("SERIES_EXPORT_UNIT_SCOPE".into());
    }
    let sha = digest(&text);
    let catalog = json!({"schemaVersion":"lingmo-v7-assets-v1","markdownSha256":sha,"sourceSha256":source.sha256,"assets":index,"units":unit_index});
    verify(&text, &catalog, draft)?;
    Ok(json!({"markdown":text,"markdownSha256":sha,"assetIndex":catalog}))
}
pub fn verify(markdown: &str, catalog: &Value, draft: &Value) -> Result<(), String> {
    if catalog["markdownSha256"] != digest(markdown) {
        return Err("SERIES_MD_HASH_MISMATCH".into());
    }
    let parsed = crate::lingmo_parser::parse(markdown, Some(catalog))?;
    if parsed["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|d| d["blocking"] == true)
    {
        return Err(format!(
            "SERIES_MD_INDEPENDENT_PARSE:{}",
            parsed["diagnostics"]
        ));
    }
    let actual = parsed["units"].as_array().ok_or("SERIES_MD_NO_UNITS")?;
    let expected = draft["units"].as_array().ok_or("SERIES_DRAFT_NO_UNITS")?;
    if actual.len() != expected.len() {
        return Err("SERIES_MD_UNIT_COUNT".into());
    }
    let catalog_assets = catalog["assets"]
        .as_array()
        .ok_or("SERIES_MD_INDEX_ASSETS")?;
    let rows: HashMap<String, &Value> = catalog_assets
        .iter()
        .map(|a| (s(a, "id").to_owned(), a))
        .collect();
    let token_rx = regex::Regex::new(r"@\[[^\]\r\n]+\]").unwrap();
    for (unit, original) in actual.iter().zip(expected) {
        if unit["importSource"]["externalUnitId"] != original["id"] {
            return Err("SERIES_MD_UNIT_ORDER".into());
        }
        let body = s(unit, "prompt");
        let mut actual_tokens: Vec<_> = token_rx
            .find_iter(body)
            .map(|m| m.as_str().to_owned())
            .collect();
        let mut expected_tokens: Vec<_> = strings(original, "imageBindings")?
            .iter()
            .map(|id| {
                rows.get(id)
                    .map(|a| format!("@[{}]", s(a, "token")))
                    .ok_or("SERIES_MD_IMAGE_INDEX_MISSING")
            })
            .collect::<Result<_, _>>()?;
        actual_tokens.sort();
        expected_tokens.sort();
        if actual_tokens != expected_tokens {
            return Err("SERIES_MD_EXACT_IMAGE_TOKEN_SET".into());
        }
        for (label, kind) in [
            ("场景", "SC"),
            ("本单元道具", "PR"),
            ("本单元出场人物", "CH"),
        ] {
            if read_fields(body, label) != vec![names_for(original, &rows, kind)?] {
                return Err(format!("SERIES_MD_IMAGE_LABEL_MISMATCH:{label}"));
            }
        }
        let shots = unit["shots"].as_array().ok_or("SERIES_MD_NO_SHOTS")?;
        let wanted = original["shots"]
            .as_array()
            .ok_or("SERIES_DRAFT_NO_SHOTS")?;
        if shots.len() != wanted.len() {
            return Err("SERIES_MD_SHOT_COUNT".into());
        }
        for (read, source) in shots.iter().zip(wanted) {
            for (a, b) in [
                ("startSeconds", "startSeconds"),
                ("endSeconds", "endSeconds"),
                ("durationSeconds", "duration"),
            ] {
                if ticks(&read[a])? != ticks(&source[b])? {
                    return Err("SERIES_MD_EXACT_TIMELINE".into());
                }
            }
            // Full literal fields are checked independently against the parsed shot body.
            let body = s(read, "text");
            for (label, key) in [
                ("机位与运镜", "camera"),
                ("画面", "picture"),
                ("光影", "lighting"),
                ("声音", "sound"),
                ("转场", "transition"),
            ] {
                if read_fields(body, label) != vec![s(source, key).to_owned()] {
                    return Err(format!("SERIES_MD_FIELD_MISMATCH:{label}"));
                }
            }
            let voices: Vec<_> = source["dialogueParts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| s(v, "text").to_owned())
                .collect();
            if read_fields(body, "原话") != voices {
                return Err("SERIES_MD_QUOTE_MISMATCH".into());
            }
        }
    }
    Ok(())
}
fn read_fields(body: &str, label: &str) -> Vec<String> {
    let prefix = format!("[{label}] ");
    let mut values = Vec::new();
    let mut current: Option<String> = None;
    for line in body.lines() {
        if let Some(value) = line.strip_prefix(&prefix) {
            if let Some(old) = current.take() {
                values.push(old);
            }
            current = Some(value.into());
        } else if let Some(value) = line.strip_prefix("  ") {
            if let Some(current) = current.as_mut() {
                current.push('\n');
                current.push_str(value);
            }
        } else if let Some(old) = current.take() {
            values.push(old);
        }
    }
    if let Some(old) = current {
        values.push(old);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readonly_design_ancestor_does_not_become_an_image_consumer() {
        let assets = vec![
            json!({"id":"parent","variantId":"base","kind":"character","materialRole":"MATERIAL_SOURCE_ONLY"}),
        ];
        let draft = json!({"units":[{"imageBindings":["parent"],"assetRefs":[{"assetId":"parent","variantId":"base","use":"character"}]}]});
        assert_eq!(
            validate_image_bindings(&draft, &assets).unwrap_err(),
            "SERIES_READONLY_DESIGN_ANCESTOR_NOT_IMAGE_USE"
        );
    }
    #[test]
    fn nanoseconds_and_container_are_separate() {
        let mut draft =
            json!({"units":[{"id":"u","shots":[{"duration":10.25,"dialogueParts":[]}]}]});
        timing(&mut draft, 15).unwrap();
        assert_eq!(draft["units"][0]["duration"], 10.25);
        assert_eq!(draft["units"][0]["containerSeconds"], 11);
        draft["units"][0]["shots"][0]["duration"] = json!(3.2);
        timing(&mut draft, 15).unwrap();
        assert_eq!(draft["units"][0]["containerSeconds"], 4);
        draft["units"][0]["shots"][0]["duration"] = json!(15.000000001f64);
        assert!(timing(&mut draft, 15).is_err());
        assert_eq!(ticks(&json!(0.000000001f64)).unwrap(), 1);
        assert!(ticks(&json!(0.0000000001f64)).is_err());
    }
    #[test]
    fn original_fields_roundtrip_newlines_without_word_changes() {
        let mut out = String::new();
        field(&mut out, "原话", "甲：你好。\n  原空格");
        field(&mut out, "转场", "切");
        assert_eq!(read_fields(&out, "原话"), vec!["甲：你好。\n  原空格"]);
    }
}
