//! Lossless, local Lingmo storyboard reader. No filesystem or provider calls.
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const PARSER_VERSION: &str = "lingmo-rust/1.0.0";
const TYPES: [&str; 5] = ["SC", "CH", "PR", "SE", "OM"];
fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn diagnostic(code: &str, message: impl Into<String>, line: usize, blocking: bool) -> Value {
    json!({"code":code,"message":message.into(),"line":line,"blocking":blocking})
}
fn rx(pattern: &'static str) -> Regex {
    Regex::new(pattern).expect("constant Lingmo regex")
}
struct Patterns {
    episode: Regex,
    scene: Regex,
    unit: Regex,
    shot: Regex,
    duration: Regex,
    declaration: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        episode: rx(r"^第\s*([0-9]+)\s*集(?:\s+.*)?$"),
        scene: rx(r"^([0-9]+)-([0-9]+)$"),
        unit: rx(r"^片段\s*([0-9]+)(?:\s+.*)?$"),
        shot: rx(r"^分镜\s*([0-9]+)\s*[｜|]\s*([0-9]+(?:\.[0-9]+)?)\s*[-–—]\s*([0-9]+(?:\.[0-9]+)?)\s*s\s*[｜|]\s*([0-9]+(?:\.[0-9]+)?)\s*s\s*$"),
        duration: rx(r"^时长\s*[:：]\s*([0-9]+(?:\.[0-9]+)?)\s*秒\s*$"),
        declaration: rx(r"^(场景|人物|道具|声音|其他群演|群演)\s*[:：]\s*(.*)$"),
    })
}
struct Line<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}
fn lines(text: &str) -> Vec<Line<'_>> {
    let mut offset = 0;
    text.split_inclusive('\n')
        .map(|s| {
            let line = Line {
                text: s.trim_end_matches(['\r', '\n']),
                start: offset,
                end: offset + s.len(),
            };
            offset += s.len();
            line
        })
        .collect()
}
fn token_parts(token: &str) -> Option<(&str, &str)> {
    let inner = token.strip_prefix("@[")?.strip_suffix(']')?;
    if inner.contains(['[', ']', '\r', '\n', '@']) {
        return None;
    }
    TYPES.iter().find_map(|kind| {
        let name = inner.strip_suffix(kind)?;
        if name.is_empty() || name.trim() != name {
            None
        } else {
            Some((name, *kind))
        }
    })
}
fn kind(ty: &str) -> &'static str {
    match ty {
        "CH" => "character",
        "SC" => "scene",
        "PR" => "prop",
        "SE" => "audio",
        _ => "other",
    }
}
fn role(ty: &str) -> &'static str {
    match ty {
        "CH" => "character",
        "SC" => "scene",
        "PR" => "prop",
        "SE" => "audio",
        _ => "other",
    }
}
fn make_asset(namespace: &str, name: &str, ty: &str, source: Option<&Value>) -> Value {
    let identity = source
        .and_then(|s| s["id"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| hash(&format!("{ty}\0{name}")));
    let revision = source
        .and_then(|s| s.get("assetRevision"))
        .cloned()
        .unwrap_or(json!(1));
    // Hashing external IDs also prevents separator ambiguity in downstream {{asset:...}} tokens.
    let id = format!("{namespace}-asset-{}", hash(&identity));
    let variant_id = format!(
        "{namespace}-variant-{}",
        hash(&format!("{identity}\0{revision}"))
    );
    json!({"id":id,"variantId":variant_id,"name":name,"kind":kind(ty),"lingmoType":ty,
        "token":format!("@[{name}{ty}]"),"externalId":source.and_then(|s|s.get("id")),
        "assetRevision":revision,"filename":source.and_then(|s|s.get("filename")),
        "placeholder":source.and_then(|s|s.get("placeholderOrdinal")).is_some(),
        "origin":"imported-storyboard"})
}
fn asset_index(assets: &[Value]) -> HashMap<String, Vec<usize>> {
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, a) in assets.iter().enumerate() {
        if let Some(token) = a["token"].as_str() {
            index.entry(token.into()).or_default().push(i);
        }
    }
    index
}
/// Returns all closed valid tokens, and diagnoses malformed markers without altering text.
fn scan_mentions(
    text: &str,
    first_line: usize,
    assets: &[Value],
    index: &HashMap<String, Vec<usize>>,
    diagnostics: &mut Vec<Value>,
) -> Vec<Value> {
    let mut mentions = Vec::new();
    for (line_index, line) in lines(text).iter().enumerate() {
        let mut pos = 0;
        while let Some(relative) = line.text[pos..].find("@[") {
            let start = pos + relative;
            let Some(close) = line.text[start + 2..].find(']') else {
                diagnostics.push(diagnostic(
                    "invalid_token",
                    "资产标签缺少右括号",
                    first_line + line_index,
                    true,
                ));
                break;
            };
            let end = start + 2 + close + 1;
            let token = &line.text[start..end];
            if let Some((name, ty)) = token_parts(token) {
                let candidates = index.get(token).map(Vec::as_slice).unwrap_or(&[]);
                let asset = if candidates.len() == 1 {
                    Some(&assets[candidates[0]])
                } else {
                    None
                };
                if candidates.len() != 1 {
                    diagnostics.push(diagnostic(
                        if candidates.is_empty() {
                            "missing_asset"
                        } else {
                            "ambiguous_asset"
                        },
                        format!(
                            "标签 {token} 匹配到 {} 个资产，未自动选择",
                            candidates.len()
                        ),
                        first_line + line_index,
                        true,
                    ));
                }
                mentions.push(json!({"token":token,"name":name,"type":ty,"start":line.start+start,"end":line.start+end,
                    "line":first_line+line_index,"assetId":asset.map(|a|&a["id"]),"variantId":asset.map(|a|&a["variantId"])}));
            } else {
                diagnostics.push(diagnostic(
                    "invalid_token",
                    format!("非法资产标签 {token}，需要名称与 SC/CH/PR/SE/OM 后缀"),
                    first_line + line_index,
                    true,
                ));
            }
            pos = end;
        }
    }
    mentions
}
fn populate_unit(
    unit: &mut Value,
    assets: &[Value],
    index: &HashMap<String, Vec<usize>>,
) -> Vec<Value> {
    let mut diagnostics = Vec::new();
    let text = unit["prompt"].as_str().unwrap_or("").to_owned();
    let first_line = unit["importSource"]["lineStart"].as_u64().unwrap_or(1) as usize;
    let mentions = scan_mentions(&text, first_line, assets, index, &mut diagnostics);
    let mut refs = Vec::new();
    let mut audio = Vec::new();
    let mut seen = HashSet::new();
    for m in &mentions {
        if m["assetId"].is_null() || !seen.insert(format!("{}|{}", m["assetId"], m["variantId"])) {
            continue;
        }
        let target = if m["type"] == "SE" {
            &mut audio
        } else {
            &mut refs
        };
        target.push(json!({"assetId":m["assetId"],"variantId":m["variantId"],"role":role(m["type"].as_str().unwrap_or("")),"order":target.len()+1}));
    }
    let ls = lines(&text);
    let p = patterns();
    let mut shots = Vec::new();
    let mut actual = 0.0_f64;
    let mut duration = None;
    let mut previous_end = 0.0;
    let mut numbers = HashSet::new();
    for (i, l) in ls.iter().enumerate() {
        if let Some(c) = p.duration.captures(l.text.trim()) {
            duration = c[1].parse::<f64>().ok();
        }
        if let Some(c) = p.shot.captures(l.text.trim()) {
            if let Some(last) = shots.last_mut() {
                close_shot(last, &text, l.start, first_line + i - 1);
            }
            let start = c[2].parse::<f64>().unwrap_or(0.0);
            let end = c[3].parse::<f64>().unwrap_or(0.0);
            let d = c[4].parse::<f64>().unwrap_or(0.0);
            if end < start
                || (end - start - d).abs() > 0.011
                || (start - previous_end).abs() > 0.011
                || !numbers.insert(c[1].to_string())
            {
                diagnostics.push(diagnostic(
                    "shot_timeline",
                    "分镜编号或时间线不连续/时长不一致，保留原始值",
                    first_line + i,
                    true,
                ));
            }
            previous_end = end;
            actual = actual.max(end);
            shots.push(json!({"number":c[1].parse::<u64>().unwrap_or(0),"originalNumber":&c[1],"startSeconds":start,"endSeconds":end,"durationSeconds":d,
                "lineStart":first_line+i,"lineEnd":first_line+i,"byteStart":l.start,"text":""}));
        } else if l.text.trim().starts_with("分镜") {
            diagnostics.push(diagnostic(
                "invalid_shot_header",
                "分镜头格式无法识别，原文仍完整保留",
                first_line + i,
                true,
            ));
        }
    }
    if let Some(last) = shots.last_mut() {
        close_shot(
            last,
            &text,
            text.len(),
            first_line + ls.len().saturating_sub(1),
        );
    }
    if shots.is_empty() {
        diagnostics.push(diagnostic(
            "missing_shots",
            "片段没有可识别分镜",
            first_line,
            true,
        ));
    }
    if duration.is_none() {
        diagnostics.push(diagnostic(
            "missing_duration",
            "片段缺少外层时长",
            first_line,
            true,
        ));
    }
    if duration.is_some_and(|d| actual > d + 0.011) {
        diagnostics.push(diagnostic(
            "container_too_short",
            "镜头时间线超出外层时长，未拉伸原文",
            first_line,
            true,
        ));
    }
    unit["mentions"] = json!(mentions);
    unit["assetRefs"] = json!(refs);
    unit["audioAssetRefs"] = json!(audio);
    unit["shots"] = json!(shots);
    unit["duration"] = json!(duration);
    unit["durationSeconds"] = json!(duration);
    unit["actualDurationSeconds"] = json!(actual);
    diagnostics
}
fn close_shot(shot: &mut Value, text: &str, end: usize, line_end: usize) {
    let start = shot["byteStart"].as_u64().unwrap_or(0) as usize;
    shot["text"] = json!(&text[start..end]);
    shot["lineEnd"] = json!(line_end);
    shot["byteEnd"] = json!(end);
}
/// Recompute edited-text references and timing; immutable importSource remains unchanged.
pub fn rebind_unit(unit: &mut Value, assets: &[Value]) -> Vec<Value> {
    let mut index = asset_index(assets);
    // A later document revision can legitimately contain the same printed token.
    // Existing verified identities win; never reinterpret an old unit by catalog order.
    let mut previous: HashMap<String, HashSet<(String, String)>> = HashMap::new();
    if let Some(mentions) = unit["mentions"].as_array() {
        for m in mentions {
            if let (Some(token), Some(id), Some(variant)) = (
                m["token"].as_str(),
                m["assetId"].as_str(),
                m["variantId"].as_str(),
            ) {
                previous
                    .entry(token.into())
                    .or_default()
                    .insert((id.into(), variant.into()));
            }
        }
    }
    let namespace = unit["importSource"]["markdownSha256"]
        .as_str()
        .map(|sha| format!("lingmo-{sha}-asset-"));
    for (token, candidates) in &mut index {
        let Some((name, ty)) = token_parts(token) else {
            candidates.clear();
            continue;
        };
        candidates.retain(|i| {
            assets[*i]["name"].as_str() == Some(name)
                && assets[*i]["lingmoType"].as_str() == Some(ty)
                && assets[*i]["id"].as_str().is_some()
                && assets[*i]["variantId"].as_str().is_some()
        });
        if let Some(bindings) = previous.get(token).filter(|b| b.len() == 1) {
            let (id, variant) = bindings.iter().next().unwrap();
            let verified: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|i| {
                    assets[*i]["id"].as_str() == Some(id.as_str())
                        && assets[*i]["variantId"].as_str() == Some(variant.as_str())
                })
                .collect();
            if verified.len() == 1 {
                *candidates = verified;
                continue;
            }
        }
        if let Some(prefix) = namespace.as_deref() {
            let local: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|i| {
                    assets[*i]["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with(prefix))
                })
                .collect();
            if !local.is_empty() {
                *candidates = local;
            }
        }
    }
    populate_unit(unit, assets, &index)
}

pub fn parse(text: &str, catalog: Option<&Value>) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Err("分镜 MD 为空".into());
    }
    let sha = hash(text);
    let namespace = format!("lingmo-{sha}");
    let p = patterns();
    let ls = lines(text);
    let mut diagnostics = Vec::new();
    let catalog_matched =
        catalog.is_some_and(|c| c["markdownSha256"].as_str() == Some(sha.as_str()));
    if catalog.is_some() && !catalog_matched {
        diagnostics.push(diagnostic(
            "catalog_hash_mismatch",
            "资产目录不属于当前 MD；保留纯文本结构，未采用目录身份/版本/文件绑定",
            1,
            true,
        ));
    }
    let valid_catalog = if catalog_matched { catalog } else { None };
    let mut assets = Vec::new();
    if let Some(c) = valid_catalog {
        if let Some(entries) = c["assets"].as_array() {
            for a in entries {
                let name = a["name"].as_str().unwrap_or("");
                let ty = a["type"].as_str().unwrap_or("");
                let expected = format!("{name}{ty}");
                let token = a["token"].as_str().unwrap_or("");
                let valid = !name.is_empty()
                    && TYPES.contains(&ty)
                    && (token == expected || token == format!("@[{expected}]"))
                    && a["id"].as_str().is_some();
                if !valid {
                    diagnostics.push(diagnostic(
                        "invalid_catalog_asset",
                        "目录资产 name/type/token/id 不一致，未采用该项",
                        1,
                        true,
                    ));
                    continue;
                }
                assets.push(make_asset(&namespace, name, ty, Some(a)));
            }
        } else {
            diagnostics.push(diagnostic(
                "invalid_catalog_assets",
                "目录 assets 必须是数组",
                1,
                true,
            ));
        }
    }
    // Without a trusted sidecar, each exact name+type is its own local identity.
    if !catalog_matched {
        let mut seen = HashSet::new();
        for l in &ls {
            let mut pos = 0;
            while let Some(r) = l.text[pos..].find("@[") {
                let start = pos + r;
                let Some(close) = l.text[start + 2..].find(']') else {
                    break;
                };
                let end = start + 2 + close + 1;
                let token = &l.text[start..end];
                if let Some((name, ty)) = token_parts(token) {
                    if seen.insert(token.to_owned()) {
                        assets.push(make_asset(&namespace, name, ty, None));
                    }
                }
                pos = end;
            }
        }
    }
    let mut episodes = Vec::<Value>::new();
    let mut scenes = Vec::<Value>::new();
    let mut units = Vec::<Value>::new();
    let mut current_episode: Option<usize> = None;
    let mut current_scene: Option<usize> = None;
    let mut open_unit: Option<(usize, usize)> = None;
    let mut identifiers = HashSet::new();
    for (i, l) in ls.iter().enumerate() {
        let trimmed = l.text.trim().trim_start_matches('\u{feff}');
        let ep = p.episode.captures(trimmed);
        let scene = p.scene.captures(trimmed);
        let fragment = p.unit.captures(trimmed);
        if ep.is_some() || scene.is_some() || fragment.is_some() {
            if let Some((u, start)) = open_unit.take() {
                units[u]["prompt"] = json!(&text[start..l.start]);
                units[u]["importSource"]["byteEnd"] = json!(l.start);
                units[u]["importSource"]["lineEnd"] = json!(i);
            }
        }
        if let Some(c) = ep {
            if let Some(s) = current_scene.take() {
                scenes[s]["lineEnd"] = json!(i);
            }
            let raw = &c[1];
            let id = unique_id(
                format!("{namespace}-episode-{raw}"),
                i + 1,
                &mut identifiers,
                &mut diagnostics,
            );
            episodes.push(json!({"id":id,"number":raw.parse::<u64>().unwrap_or(0),"originalNumber":raw,"title":trimmed,"sceneIds":[]}));
            current_episode = Some(episodes.len() - 1);
            continue;
        }
        if let Some(c) = scene {
            if let Some(s) = current_scene.take() {
                scenes[s]["lineEnd"] = json!(i);
            }
            let Some(e) = current_episode else {
                diagnostics.push(diagnostic(
                    "scene_without_episode",
                    "场次缺少所属集",
                    i + 1,
                    true,
                ));
                continue;
            };
            if c[1].parse::<u64>().ok() != episodes[e]["number"].as_u64() {
                diagnostics.push(diagnostic(
                    "scene_episode_mismatch",
                    "场次编号与所在集不一致，未自动改号",
                    i + 1,
                    true,
                ));
            }
            let id = unique_id(
                format!("{namespace}-scene-{trimmed}"),
                i + 1,
                &mut identifiers,
                &mut diagnostics,
            );
            episodes[e]["sceneIds"]
                .as_array_mut()
                .unwrap()
                .push(json!(id));
            scenes.push(json!({"id":id,"episodeId":episodes[e]["id"],"number":trimmed,"title":trimmed,"declaredAssets":{"SC":[],"CH":[],"PR":[],"SE":[],"OM":[]},"unitIds":[],"assetIds":[],"lineStart":i+1,"lineEnd":i+1}));
            current_scene = Some(scenes.len() - 1);
            continue;
        }
        if let Some(c) = fragment {
            let (Some(e), Some(s)) = (current_episode, current_scene) else {
                diagnostics.push(diagnostic(
                    "unit_without_scene",
                    "片段缺少所属集/场次",
                    i + 1,
                    true,
                ));
                continue;
            };
            let raw = &c[1];
            let id = unique_id(
                format!(
                    "{namespace}-unit-{}-{}-{raw}",
                    episodes[e]["originalNumber"].as_str().unwrap_or(""),
                    scenes[s]["number"].as_str().unwrap_or("")
                ),
                i + 1,
                &mut identifiers,
                &mut diagnostics,
            );
            scenes[s]["unitIds"].as_array_mut().unwrap().push(json!(id));
            units.push(json!({"id":id,"episodeId":episodes[e]["id"],"episodeNumber":episodes[e]["number"],"sceneId":scenes[s]["id"],"sceneNumber":scenes[s]["number"],"fragmentNumber":raw.parse::<u64>().unwrap_or(0),"originalFragmentNumber":raw,"title":trimmed,"prompt":"","origin":"imported-storyboard","importSource":{"markdownSha256":sha,"lineStart":i+1,"lineEnd":i+1,"byteStart":l.start,"byteEnd":l.end,"externalUnitId":null}}));
            open_unit = Some((units.len() - 1, l.start));
            continue;
        }
        if open_unit.is_none() {
            if let (Some(s), Some(c)) = (current_scene, p.declaration.captures(trimmed)) {
                let ty = match &c[1] {
                    "场景" => "SC",
                    "人物" => "CH",
                    "道具" => "PR",
                    "声音" => "SE",
                    _ => "OM",
                };
                for part in c[2].split(['、', ',', '，', ';', '；']) {
                    let part = part.trim();
                    if part.is_empty() || matches!(part, "无" | "暂无" | "无。") {
                        continue;
                    }
                    let name = if let Some(at) = part.find("@[") {
                        if let Some((name, token_ty)) = token_parts(&part[at..]) {
                            if token_ty != ty {
                                diagnostics.push(diagnostic(
                                    "declaration_type_mismatch",
                                    format!("清单类型 {ty} 与标签 {token_ty} 不同"),
                                    i + 1,
                                    true,
                                ));
                            }
                            name
                        } else {
                            part
                        }
                    } else {
                        part
                    };
                    let arr = scenes[s]["declaredAssets"][ty].as_array_mut().unwrap();
                    if !arr.contains(&json!(name)) {
                        arr.push(json!(name));
                    }
                }
            }
        }
    }
    if let Some((u, start)) = open_unit.take() {
        units[u]["prompt"] = json!(&text[start..]);
        units[u]["importSource"]["byteEnd"] = json!(text.len());
        units[u]["importSource"]["lineEnd"] = json!(ls.len());
    }
    if let Some(s) = current_scene {
        scenes[s]["lineEnd"] = json!(ls.len());
    }
    if units.is_empty() {
        return Err("未识别到“第 N 集 → N-M → 片段 N”的完整结构".into());
    }
    if !catalog_matched {
        let mut known: HashSet<String> = assets
            .iter()
            .filter_map(|a| a["token"].as_str().map(str::to_owned))
            .collect();
        for s in &scenes {
            for ty in TYPES {
                for name in s["declaredAssets"][ty].as_array().unwrap() {
                    if let Some(name) = name.as_str() {
                        let token = format!("@[{name}{ty}]");
                        if token_parts(&token).is_some() && known.insert(token) {
                            assets.push(make_asset(&namespace, name, ty, None));
                        }
                    }
                }
            }
        }
    }
    let index = asset_index(&assets);
    let mut catalog_units: HashMap<String, Vec<&Value>> = HashMap::new();
    if let Some(c) = valid_catalog {
        if let Some(us) = c["units"].as_array() {
            for u in us {
                catalog_units
                    .entry(format!(
                        "{}|{}|{}",
                        u["episode"],
                        u["sceneId"].as_str().unwrap_or(""),
                        u["fragment"]
                    ))
                    .or_default()
                    .push(u);
            }
        }
    }
    for unit in &mut units {
        diagnostics.extend(populate_unit(unit, &assets, &index));
        if catalog_matched {
            let key = format!(
                "{}|{}|{}",
                unit["episodeNumber"],
                unit["sceneNumber"].as_str().unwrap_or(""),
                unit["fragmentNumber"]
            );
            let matches = catalog_units.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            let line = unit["importSource"]["lineStart"].as_u64().unwrap_or(1) as usize;
            if matches.len() == 1 {
                let cu = matches[0];
                unit["importSource"]["externalUnitId"] = cu["unitId"].clone();
                let mut actual: Vec<String> = unit["mentions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["type"] != "SE")
                    .filter_map(|m| {
                        m["token"]
                            .as_str()
                            .map(|s| s.trim_start_matches("@[").trim_end_matches(']').to_owned())
                    })
                    .collect();
                actual.sort();
                actual.dedup();
                let mut expected: Vec<String> = cu["imageTokens"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .map(|s| s.trim_start_matches("@[").trim_end_matches(']').to_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                expected.sort();
                expected.dedup();
                if actual != expected {
                    diagnostics.push(diagnostic(
                        "catalog_unit_tokens_mismatch",
                        "目录单元 imageTokens 与正文不同，使用正文引用并报告差异",
                        line,
                        true,
                    ));
                }
                for (external, internal) in [
                    ("actualSeconds", "actualDurationSeconds"),
                    ("containerSeconds", "durationSeconds"),
                ] {
                    if let (Some(a), Some(b)) = (cu[external].as_f64(), unit[internal].as_f64()) {
                        if (a - b).abs() > 0.011 {
                            diagnostics.push(diagnostic(
                                "catalog_unit_duration_mismatch",
                                format!("目录 {external} 与正文不一致"),
                                line,
                                true,
                            ));
                        }
                    }
                }
            } else {
                diagnostics.push(diagnostic(
                    "catalog_unit_mismatch",
                    format!(
                        "目录集/场/片段匹配到 {} 条记录，未采用外部单元身份",
                        matches.len()
                    ),
                    line,
                    true,
                ));
            }
        }
    }
    for scene in &mut scenes {
        let mut ids = Vec::<Value>::new();
        let line = scene["lineStart"].as_u64().unwrap_or(1) as usize;
        for ty in TYPES {
            for name in scene["declaredAssets"][ty].as_array().unwrap() {
                let token = format!("@[{}{ty}]", name.as_str().unwrap_or(""));
                match index.get(&token).map(Vec::as_slice).unwrap_or(&[]) {
                    [one] => {
                        let id = assets[*one]["id"].clone();
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                    matches => diagnostics.push(diagnostic(
                        "scene_asset_unresolved",
                        format!("场清单 {token} 匹配到 {} 个资产", matches.len()),
                        line,
                        false,
                    )),
                }
            }
        }
        for u in units.iter().filter(|u| u["sceneId"] == scene["id"]) {
            for m in u["mentions"].as_array().unwrap() {
                if !m["assetId"].is_null() && !ids.contains(&m["assetId"]) {
                    ids.push(m["assetId"].clone());
                }
                let ty = m["type"].as_str().unwrap_or("OM");
                let declared = scene["declaredAssets"][ty].as_array().unwrap();
                if !declared.is_empty() && !declared.contains(&m["name"]) {
                    diagnostics.push(diagnostic(
                        "scene_asset_not_declared",
                        format!(
                            "片段引用 {} 未列入本场 {ty} 清单",
                            m["token"].as_str().unwrap_or("")
                        ),
                        m["line"].as_u64().unwrap_or(1) as usize,
                        false,
                    ));
                }
            }
        }
        scene["assetIds"] = json!(ids);
    }
    // Includes tokens in scene declarations as well as unit bodies; avoid duplicating unit diagnostics.
    let mut doc_diagnostics = Vec::new();
    let all_mentions = scan_mentions(text, 1, &assets, &index, &mut doc_diagnostics);
    for d in doc_diagnostics {
        if !diagnostics.contains(&d) {
            diagnostics.push(d);
        }
    }
    let shot_count: usize = units
        .iter()
        .map(|u| u["shots"].as_array().unwrap().len())
        .sum();
    for d in &mut diagnostics {
        let line = d["line"].as_u64().unwrap_or(0);
        if let Some(s) = scenes.iter().find(|s| {
            s["lineStart"].as_u64().unwrap_or(0) <= line
                && s["lineEnd"].as_u64().unwrap_or(0) >= line
        }) {
            d["sceneId"] = s["id"].clone();
            d["sceneNumber"] = s["number"].clone();
            d["episodeId"] = s["episodeId"].clone();
        }
        if let Some(u) = units.iter().find(|u| {
            u["importSource"]["lineStart"].as_u64().unwrap_or(0) <= line
                && u["importSource"]["lineEnd"].as_u64().unwrap_or(0) >= line
        }) {
            d["unitId"] = u["id"].clone();
            d["fragmentNumber"] = u["fragmentNumber"].clone();
        }
    }
    let counts = json!({"episodes":episodes.len(),"scenes":scenes.len(),"fragments":units.len(),"shots":shot_count,"mentionOccurrences":all_mentions.len(),"assets":assets.len()});
    Ok(
        json!({"schemaVersion":"lingmo-import/v1","parserVersion":PARSER_VERSION,"markdownSha256":sha,"originalText":text,"episodes":episodes,"scenes":scenes,"units":units,"assets":assets,"diagnostics":diagnostics,"counts":counts,"catalogMatched":catalog_matched}),
    )
}
fn unique_id(
    id: String,
    line: usize,
    seen: &mut HashSet<String>,
    diagnostics: &mut Vec<Value>,
) -> String {
    if seen.insert(id.clone()) {
        id
    } else {
        diagnostics.push(diagnostic(
            "duplicate_number",
            "同层编号重复，保留原号并隔离内部身份",
            line,
            true,
        ));
        format!("{id}-line-{line}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE:&str="**【标准化剧本】**\r\n第1集\r\n1-1\r\n场景：阳台、餐厅\r\n人物：甲\r\n片段1\r\n时长：4秒\r\n[场景] @[阳台SC]\r\n[人物] @[甲CH] @[甲PR] @[甲SE] @[路人OM]\r\n分镜1｜0-2.7s｜2.7s\r\n画面：甲走入。\r\n[结束状态] 保留。\r\n片段2\r\n时长：3秒\r\n[场景] @[餐厅SC]\r\n分镜1｜0-2.1s｜2.1s\r\n画面：结束。";
    #[test]
    fn lossless_hierarchy_types_and_offsets() {
        let d = parse(SAMPLE, None).unwrap();
        assert_eq!(d["originalText"], SAMPLE);
        assert_eq!(d["counts"]["scenes"], 1);
        assert_eq!(d["counts"]["fragments"], 2);
        assert_eq!(d["counts"]["shots"], 2);
        assert_eq!(d["counts"]["assets"], 6);
        let u = &d["units"][0];
        assert_eq!(u["durationSeconds"], 4.0);
        assert_eq!(u["actualDurationSeconds"], 2.7);
        assert_eq!(u["assetRefs"].as_array().unwrap().len(), 4);
        assert_eq!(u["audioAssetRefs"].as_array().unwrap().len(), 1);
        for u in d["units"].as_array().unwrap() {
            let start = u["importSource"]["byteStart"].as_u64().unwrap() as usize;
            let end = u["importSource"]["byteEnd"].as_u64().unwrap() as usize;
            assert_eq!(u["prompt"], &SAMPLE[start..end]);
            for m in u["mentions"].as_array().unwrap() {
                assert_eq!(
                    &u["prompt"].as_str().unwrap()[m["start"].as_u64().unwrap() as usize
                        ..m["end"].as_u64().unwrap() as usize],
                    m["token"].as_str().unwrap()
                );
            }
        }
        assert_eq!(d["units"][0]["assetRefs"][1]["role"], "character");
        assert_ne!(
            d["units"][0]["mentions"][1]["assetId"],
            d["units"][0]["mentions"][2]["assetId"]
        );
    }
    #[test]
    fn plain_scene_catalog_creates_all_five_types_without_inflating_unit_refs() {
        let text = "第1集\n1-1\n场景：阳台、餐厅\n人物：甲、乙\n道具：钥匙\n声音：风声\n其他群演：路人\n片段1\n时长：3秒\n@[阳台SC] @[甲CH]\n分镜1｜0-2s｜2s\n画面：甲站在阳台。\n片段2\n时长：3秒\n@[餐厅SC]\n分镜1｜0-2s｜2s\n画面：餐厅。";
        let d = parse(text, None).unwrap();
        assert_eq!(d["counts"]["scenes"], 1);
        assert_eq!(d["counts"]["assets"], 7);
        assert_eq!(d["scenes"][0]["assetIds"].as_array().unwrap().len(), 7);
        let assets = d["assets"].as_array().unwrap();
        for (name, ty) in [
            ("阳台", "SC"),
            ("餐厅", "SC"),
            ("甲", "CH"),
            ("乙", "CH"),
            ("钥匙", "PR"),
            ("风声", "SE"),
            ("路人", "OM"),
        ] {
            assert_eq!(
                assets
                    .iter()
                    .filter(|a| a["name"] == name && a["lingmoType"] == ty)
                    .count(),
                1
            );
        }
        assert_eq!(d["units"][0]["assetRefs"].as_array().unwrap().len(), 2);
        assert_eq!(d["units"][1]["assetRefs"].as_array().unwrap().len(), 1);
        assert!(d["units"][0]["audioAssetRefs"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(d["units"][0]["assetRefs"][0]["order"], 1);
        assert_eq!(d["units"][0]["assetRefs"][1]["order"], 2);
        assert_eq!(d["units"][0]["sceneId"], d["units"][1]["sceneId"]);
        assert!(d["diagnostics"].as_array().unwrap().is_empty());
    }
    #[test]
    fn trusted_sidecar_does_not_invent_unlisted_scene_asset_identity() {
        let text = "第1集\n1-1\n人物：甲、未入目录的人\n片段1\n时长：3秒\n@[甲CH]\n分镜1｜0-2s｜2s\n画面：甲。";
        let catalog = json!({"markdownSha256":hash(text),"assets":[{"id":"CH1","name":"甲","type":"CH","token":"甲CH","assetRevision":2}],"units":[{"episode":1,"sceneId":"1-1","fragment":1,"unitId":"U1","imageTokens":["甲CH"],"actualSeconds":2.0,"containerSeconds":3.0}]});
        let d = parse(text, Some(&catalog)).unwrap();
        assert_eq!(d["counts"]["assets"], 1);
        assert_eq!(d["assets"][0]["externalId"], "CH1");
        assert_eq!(d["assets"][0]["assetRevision"], 2);
        assert_eq!(d["scenes"][0]["assetIds"].as_array().unwrap().len(), 1);
        assert!(d["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "scene_asset_unresolved"));
    }
    #[test]
    fn catalog_hash_mismatch_does_not_use_external_identity() {
        let c = json!({"markdownSha256":"wrong","assets":[{"id":"secret","name":"甲","type":"CH","token":"甲CH","assetRevision":5}]});
        let d = parse(SAMPLE, Some(&c)).unwrap();
        assert_eq!(d["catalogMatched"], false);
        assert!(d["assets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["externalId"].is_null()));
        assert!(d["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "catalog_hash_mismatch"));
    }
    #[test]
    fn ambiguous_variants_never_choose_first() {
        let c = json!({"markdownSha256":hash(SAMPLE),"assets":[{"id":"a","name":"甲","type":"CH","token":"甲CH","assetRevision":1},{"id":"a","name":"甲","type":"CH","token":"甲CH","assetRevision":2}],"units":[]});
        let d = parse(SAMPLE, Some(&c)).unwrap();
        let m = &d["units"][0]["mentions"][1];
        assert!(m["assetId"].is_null());
        assert_ne!(d["assets"][0]["variantId"], d["assets"][1]["variantId"]);
        assert!(d["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "ambiguous_asset"));
    }
    #[test]
    fn malformed_tokens_preserved_and_diagnosed() {
        let text = SAMPLE.replace("@[甲CH]", "@[甲XX] @[CH] @[甲CH\n");
        let d = parse(&text, None).unwrap();
        assert_eq!(d["originalText"], text);
        assert!(
            d["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| d["code"] == "invalid_token")
                .count()
                >= 3
        );
    }
    #[test]
    fn edited_references_are_rebound_without_mutating_origin() {
        let d = parse(SAMPLE, None).unwrap();
        let mut u = d["units"][0].clone();
        let origin = u["importSource"].clone();
        u["prompt"] = json!("片段1\n时长：3秒\n@[餐厅SC] @[甲SE]\n分镜1｜0-2s｜2s\n画面：保留。");
        let ds = rebind_unit(&mut u, d["assets"].as_array().unwrap());
        assert!(ds.is_empty());
        assert_eq!(u["importSource"], origin);
        assert_eq!(u["assetRefs"].as_array().unwrap().len(), 1);
        assert_eq!(u["audioAssetRefs"].as_array().unwrap().len(), 1);
        assert_eq!(u["actualDurationSeconds"], 2.0);
    }
    #[test]
    fn duplicate_numbers_preserve_both() {
        let text = format!("{SAMPLE}\n片段2\n时长：3秒\n分镜1｜0-3s｜3s\n画面：原文");
        let d = parse(&text, None).unwrap();
        assert_eq!(d["units"].as_array().unwrap().len(), 3);
        assert_ne!(d["units"][1]["id"], d["units"][2]["id"]);
        assert_eq!(
            d["units"][1]["fragmentNumber"],
            d["units"][2]["fragmentNumber"]
        );
    }
    #[test]
    fn rebind_prefers_verified_existing_then_document_namespace() {
        let original = parse(SAMPLE, None).unwrap();
        let revised = parse(&SAMPLE.replace("画面：结束。", "画面：结束！"), None).unwrap();
        let mut assets = revised["assets"].as_array().unwrap().clone();
        assets.extend(original["assets"].as_array().unwrap().iter().cloned());
        let mut old_unit = original["units"][0].clone();
        let old_refs = old_unit["assetRefs"].clone();
        assert!(rebind_unit(&mut old_unit, &assets).is_empty());
        assert_eq!(old_unit["assetRefs"], old_refs);
        // New token in an edit binds within the old document, despite reversed global order.
        old_unit["prompt"] = json!("片段1\n时长：3秒\n@[餐厅SC]\n分镜1｜0-2s｜2s\n画面：原文");
        assert!(rebind_unit(&mut old_unit, &assets).is_empty());
        assert_eq!(
            old_unit["assetRefs"][0]["assetId"],
            original["units"][1]["assetRefs"][0]["assetId"]
        );
        // A user can explicitly bind another document's precise identity; preserve it.
        old_unit["mentions"] = revised["units"][1]["mentions"].clone();
        assert!(rebind_unit(&mut old_unit, &assets).is_empty());
        assert_eq!(
            old_unit["assetRefs"][0]["assetId"],
            revised["units"][1]["assetRefs"][0]["assetId"]
        );
    }
    #[test]
    fn rebind_rejects_forged_asset_metadata() {
        let parsed = parse(SAMPLE, None).unwrap();
        let mut assets = parsed["assets"].as_array().unwrap().clone();
        let target = assets.iter_mut().find(|a| a["token"] == "@[甲CH]").unwrap();
        target["name"] = json!("另一人");
        let mut unit = parsed["units"][0].clone();
        let diagnostics = rebind_unit(&mut unit, &assets);
        assert!(diagnostics.iter().any(|d| d["code"] == "missing_asset"));
        assert!(unit["mentions"][1]["assetId"].is_null());
    }
    /// Opt-in local acceptance: no private body or absolute personal path is in this fixture.
    #[test]
    #[ignore = "requires LINGMO_ACCEPTANCE_MD local private fixture"]
    fn complete_private_sample() {
        let path = std::env::var("LINGMO_ACCEPTANCE_MD").expect("fixture path");
        let text = std::fs::read_to_string(&path).unwrap();
        let c: Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{path}.assets.json")).unwrap())
                .unwrap();
        let start = std::time::Instant::now();
        let d = parse(&text, Some(&c)).unwrap();
        assert_eq!(
            d["counts"],
            json!({"episodes":150,"scenes":371,"fragments":883,"shots":5407,"mentionOccurrences":4736,"assets":518})
        );
        assert_eq!(d["catalogMatched"], true);
        assert_eq!(d["originalText"], text);
        let us = d["units"].as_array().unwrap();
        assert_eq!(us[0]["durationSeconds"], 24.0);
        assert_eq!(us[0]["actualDurationSeconds"], 23.8);
        assert_eq!(us[882]["durationSeconds"], 4.0);
        assert_eq!(us[882]["actualDurationSeconds"], 2.7);
        assert_eq!(
            d["scenes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["number"] == "150-1")
                .count(),
            1
        );
        assert!(us
            .iter()
            .flat_map(|u| u["mentions"].as_array().unwrap())
            .all(|m| !m["assetId"].is_null()));
        for u in us {
            assert!(!u["importSource"]["externalUnitId"].is_null());
            let a = u["importSource"]["byteStart"].as_u64().unwrap() as usize;
            let b = u["importSource"]["byteEnd"].as_u64().unwrap() as usize;
            assert_eq!(u["prompt"], &text[a..b]);
        }
        let blocking: Vec<_> = d["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["blocking"] == true)
            .collect();
        assert!(blocking.is_empty(), "blocking diagnostics: {blocking:?}");
        println!(
            "Lingmo full sample: {:?}; counts {}; diagnostics {}",
            start.elapsed(),
            d["counts"],
            d["diagnostics"].as_array().unwrap().len()
        );
    }
}
