//! Mechanical validation of an already adopted upstream handoff. This does not
//! rerun its author/reviewer or claim a new desktop independent content review.
use crate::{
    module22::{check, rows, validate_scope},
    module22_json as js,
    pipeline::{
        digest,
        series::export::{decimal, ticks},
    },
    store::{s, Result},
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn fields(body: &str, label: &str) -> Vec<String> {
    let colon = format!("{label}：");
    let bracket = format!("[{label}] ");
    let mut out = Vec::new();
    let mut active: Option<String> = None;
    for line in body.lines() {
        if let Some(v) = line
            .strip_prefix(&colon)
            .or_else(|| line.strip_prefix(&bracket))
        {
            if let Some(v) = active.take() {
                out.push(v);
            }
            active = Some(v.into());
        } else if let Some(v) = line.strip_prefix("  ") {
            if let Some(a) = active.as_mut() {
                a.push('\n');
                a.push_str(v);
            }
        } else if let Some(v) = active.take() {
            out.push(v);
        }
    }
    if let Some(v) = active {
        out.push(v);
    }
    out
}
fn equal_field(body: &str, label: &str, wanted: &Value) -> Result<()> {
    check(
        fields(body, label) == vec![wanted.as_str().unwrap_or("").to_owned()],
        &format!("MD_FIELD:{label}"),
    )
}
pub(crate) fn verify(
    bundle: &Value,
    lock: &Value,
    registry: &Value,
    catalog: &Value,
    markdown: &str,
    queue: &Value,
    receipt: &Value,
) -> Result<Value> {
    let parsed = crate::lingmo_parser::parse(markdown, Some(catalog))?;
    check(
        parsed["catalogMatched"] == true
            && !rows(&parsed, "diagnostics")?
                .iter()
                .any(|d| d["blocking"] == true),
        "MD_INDEPENDENT_PARSE",
    )?;
    let current: HashMap<_, _> = rows(registry, "assets")?
        .iter()
        .map(|a| (s(a, "id"), a))
        .collect();
    check(
        current.len() == rows(registry, "assets")?.len(),
        "REGISTRY_DUPLICATE_ID",
    )?;
    let mut projected = HashMap::new();
    for a in rows(bundle, "assets")? {
        let full = current
            .get(s(a, "assetId"))
            .ok_or("MODULE22_BUNDLE_ASSET")?;
        check(
            full["revision"] == a["assetRevision"] && projected.insert(s(a, "key"), a).is_none(),
            "BUNDLE_ASSET_REVISION",
        )?;
        for key in [
            "name",
            "key",
            "type",
            "status",
            "importName",
            "placeholderOrdinal",
        ] {
            check(a[key] == full[key], "BUNDLE_ASSET_IDENTITY")?;
        }
    }
    let mut indexed = HashMap::new();
    for a in rows(catalog, "assets")? {
        let full = current.get(s(a, "id")).ok_or("MODULE22_CATALOG_ASSET")?;
        let prefix = match s(a, "type") {
            "CH" => "角色",
            "SC" => "场景",
            "PR" => "道具",
            _ => return Err("MODULE22_CATALOG_TYPE".into()),
        };
        check(
            indexed.insert(s(a, "id"), a).is_none()
                && a["assetRevision"] == full["revision"]
                && a["name"] == full["importName"]
                && a["type"] == full["type"]
                && a["placeholderOrdinal"] == full["placeholderOrdinal"]
                && a["token"] == format!("{}{}", s(full, "importName"), s(full, "type"))
                && a["filename"] == format!("{prefix}-{}.png", s(full, "importName")),
            "CATALOG_ASSET_IDENTITY",
        )?;
        check(
            matches!(
                s(&full["design"], "state"),
                "TEXT_ADOPTED" | "REFERENCE_ADOPTED"
            ) && full["design"]["full"].is_object()
                && !s(&full["design"], "text").is_empty(),
            "COMPLETE_ADOPTED_DESIGN_REQUIRED",
        )?;
    }
    check(
        indexed.len() == rows(queue, "items")?.len()
            && rows(queue, "items")?.iter().all(|i| {
                indexed.get(s(i, "assetId")).is_some_and(|a| {
                    a["assetRevision"] == i["assetRevision"] && a["filename"] == i["filename"]
                })
            }),
        "QUEUE_CATALOG_SCOPE",
    )?;
    let scope = registry.get("productionScope");
    if let Some(scope) = scope {
        validate_scope(scope, &registry["sourceHash"])?;
        check(
            bundle["productionScope"] == *scope
                && catalog["productionScope"] == *scope
                && catalog["scopeHash"] == js::hash(scope)?
                && receipt["productionScopeHash"] == js::hash(scope)?,
            "BUNDLE_SEASON_SCOPE",
        )?;
    } else {
        check(
            bundle.get("productionScope").is_none() && catalog.get("productionScope").is_none(),
            "BUNDLE_SEASON_DOWNGRADE",
        )?;
    }
    let episodes = rows(bundle, "episodes")?;
    check(
        episodes.len() == rows(bundle, "scope")?.len()
            && episodes.len() == rows(&parsed, "episodes")?.len(),
        "BUNDLE_EPISODE_ORDER",
    )?;
    let mut ordered_units = Vec::new();
    let mut expected_uses = Vec::new();
    let mut used_images = HashSet::new();
    let mut previous_lock = None;
    let mut total_ticks = 0u64;
    for (ei, e) in episodes.iter().enumerate() {
        check(
            e["number"] == bundle["scope"][ei]["number"]
                && e["number"] == parsed["episodes"][ei]["number"],
            "BUNDLE_EPISODE_ORDER",
        )?;
        let position = rows(lock, "episodes")?
            .iter()
            .position(|v| v["number"] == e["number"])
            .ok_or("MODULE22_SOURCE_LOCK_SCOPE")?;
        check(
            previous_lock.is_none_or(|old| position > old)
                && e["source"] == lock["episodes"][position]["source"],
            "SOURCE_LOCK_LITERAL",
        )?;
        previous_lock = Some(position);
        let scene_ids: Vec<_> = rows(e, "scenes")?.iter().map(|s| s["id"].clone()).collect();
        check(
            json!(scene_ids) == bundle["scope"][ei]["sceneIds"],
            "BUNDLE_SCENE_ORDER",
        )?;
        if let Some(scope) = scope {
            check(
                rows(scope, "episodes")?
                    .iter()
                    .any(|se| se["number"] == e["number"] && se["sceneIds"] == json!(scene_ids)),
                "BUNDLE_SEASON_USE",
            )?;
        }
        let units = rows(&e["draft"], "units")?;
        let mut seen = HashSet::new();
        for scene in rows(e, "scenes")? {
            let in_scene: Vec<_> = units
                .iter()
                .filter(|u| u["sceneId"] == scene["id"])
                .collect();
            check(!in_scene.is_empty(), "EMPTY_SCENE")?;
            for (fragment, u) in in_scene.into_iter().enumerate() {
                check(seen.insert(s(u, "id")), "DUPLICATE_UNIT")?;
                let mut tokens = Vec::new();
                for key in rows(u, "bindings")? {
                    let a = projected
                        .get(key.as_str().ok_or("MODULE22_BINDING")?)
                        .ok_or("MODULE22_BINDING")?;
                    expected_uses.push(json!({"episode":e["number"],"sceneId":u["sceneId"],"unitId":u["id"],"assetId":a["assetId"],"assetRevision":a["assetRevision"],"key":a["key"]}));
                }
                for key in rows(u, "imageBindings")? {
                    let a = projected
                        .get(key.as_str().ok_or("MODULE22_IMAGE_BINDING")?)
                        .ok_or("MODULE22_IMAGE_BINDING")?;
                    let c = indexed
                        .get(s(a, "assetId"))
                        .ok_or("MODULE22_IMAGE_BINDING")?;
                    used_images.insert(s(a, "assetId"));
                    tokens.push(c["token"].clone());
                    check(
                        rows(queue, "items")?.iter().any(|q| {
                            q["assetId"] == a["assetId"]
                                && rows(q, "uses").is_ok_and(|v| {
                                    v.iter().any(|x| {
                                        x["episode"] == e["number"] && x["sceneId"] == u["sceneId"]
                                    })
                                })
                        }),
                        "QUEUE_MISSING_USE",
                    )?;
                }
                let at = ordered_units.len();
                let item = catalog["units"].get(at).ok_or("MODULE22_CATALOG_UNIT")?;
                check(
                    item["episode"] == e["number"]
                        && item["sceneId"] == u["sceneId"]
                        && item["unitId"] == u["id"]
                        && item["fragment"] == fragment + 1
                        && item["imageTokens"] == json!(tokens),
                    "CATALOG_UNIT_BINDING",
                )?;
                ordered_units.push((u, e, item));
            }
        }
        check(seen.len() == units.len(), "UNIT_UNKNOWN_SCENE")?;
    }
    check(
        expected_uses == *rows(receipt, "uses")? && used_images.len() == indexed.len(),
        "CONSUMPTION_EXACT_USES",
    )?;
    check(
        ordered_units.len() == rows(catalog, "units")?.len()
            && ordered_units.len() == rows(&parsed, "units")?.len(),
        "MD_UNIT_COUNT",
    )?;
    let token_rx = regex::Regex::new(r"@\[[^\]]+\]").unwrap();
    for ((u, e, item), actual) in ordered_units.into_iter().zip(rows(&parsed, "units")?) {
        // The desktop parser deliberately retains the fragment and duration
        // header. Upstream renderUnitBody hashes only what follows those two
        // lines; verify their exact values before taking this read-only slice.
        // Never normalize, trim or rewrite the original saved Markdown.
        let prefix = format!(
            "片段{}\n时长：{}秒\n",
            item["fragment"], item["containerSeconds"]
        );
        let body = s(actual, "prompt")
            .strip_prefix(&prefix)
            .ok_or("MODULE22_MD_FRAGMENT_HEADER")?;
        check(
            item["bodySha256"] == digest(body)
                && item["bodyWithoutTokensSha256"] == digest(&token_rx.replace_all(body, "")),
            "MD_BODY_HASH",
        )?;
        check(
            actual["importSource"]["externalUnitId"] == u["id"],
            "MD_UNIT_ORDER",
        )?;
        let opening = u["segments"]
            .as_array()
            .and_then(|v| v.first())
            .unwrap_or(u);
        equal_field(body, "本单元光影", &opening["lighting"])?;
        equal_field(body, "人物状态与站位前提", &opening["initialState"])?;
        equal_field(body, "结束状态", &u["exitState"])?;
        let style = [
            s(&e["settings"], "visualStyle"),
            s(&e["settings"], "prohibitions"),
            s(u, "unitDirection"),
        ]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
        equal_field(body, "通用提示词", &json!(style))?;
        let shots = rows(u, "shots")?;
        check(shots.len() == rows(actual, "shots")?.len(), "MD_SHOT_COUNT")?;
        let mut cursor = 0u64;
        for (shot, got) in shots.iter().zip(rows(actual, "shots")?) {
            let duration = ticks(&shot["duration"])?;
            check(duration > 0, "MD_DURATION")?;
            let end = cursor.checked_add(duration).ok_or("MODULE22_MD_DURATION")?;
            check(
                ticks(&got["startSeconds"])? == cursor
                    && ticks(&got["endSeconds"])? == end
                    && ticks(&got["durationSeconds"])? == duration,
                "MD_EXACT_TIMELINE",
            )?;
            for (label, key) in [
                ("机位/运镜", "camera"),
                ("画面", "picture"),
                ("光影", "lighting"),
                ("声音", "sound"),
                ("转场", "transition"),
            ] {
                equal_field(s(got, "text"), label, &shot[key])?;
            }
            let parts = rows(shot, "dialogueParts")?;
            let actual_voices = fields(s(got, "text"), "台词");
            if parts.is_empty() {
                check(actual_voices == ["无"], "MD_VOICE")?;
            } else {
                check(actual_voices.len() == parts.len(), "MD_VOICE")?;
                for (p, line) in parts.iter().zip(actual_voices) {
                    let start = ticks(&p["voiceStart"])?;
                    let finish = ticks(&p["voiceEnd"])?;
                    check(finish > start && finish <= duration, "MD_VOICE_WINDOW")?;
                    check(
                        line.starts_with(&format!(
                            "{}–{}秒｜",
                            decimal(cursor + start),
                            decimal(cursor + finish)
                        )) && line.ends_with(&format!("）：{}", s(p, "text")))
                            && line.contains(&format!("｜{}｜", s(p, "presentation")))
                            && line.contains(&format!("演法：{}", s(p, "delivery"))),
                        "MD_VOICE_LITERAL",
                    )?;
                    if let Some(q) = e["source"]["dialogue"]
                        .as_array()
                        .and_then(|qs| qs.iter().find(|q| q["id"] == p["lineId"]))
                    {
                        check(
                            line.contains(&format!(
                                "秒｜{}（{}｜",
                                s(q, "speaker"),
                                s(q, "channel")
                            )),
                            "MD_VOICE_SPEAKER",
                        )?;
                    }
                }
            }
            cursor = end;
        }
        check(
            ticks(&item["actualSeconds"])? == cursor
                && item["containerSeconds"].as_u64()
                    == Some(((cursor + 999_999_999) / 1_000_000_000).max(4)),
            "MD_CONTAINER",
        )?;
        total_ticks = total_ticks
            .checked_add(cursor)
            .ok_or("MODULE22_TOTAL_DURATION")?;
    }
    if let Some(scope) = scope {
        if bundle["seasonDelivery"] == "final" {
            check(
                bundle["scope"] == scope["episodes"],
                "SEASON_FINAL_COVERAGE",
            )?;
            check(
                scope.get("durationException").is_some()
                    || (7_200_000_000_000..=9_000_000_000_000).contains(&total_ticks),
                "SEASON_FINAL_DURATION",
            )?;
        }
    }
    Ok(
        json!({"version":"module22-upstream-consumption-proof/v1","literalShotFields":true,"dialogueSegmentsAndWindows":true,"sourceLockLiteral":true,"existingConsumptionReceipt":true,"independentDesktopCreativeReview":false,"units":rows(catalog,"units")?.len(),"images":indexed.len()}),
    )
}
