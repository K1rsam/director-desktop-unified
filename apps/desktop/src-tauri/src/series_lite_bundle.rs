//! Native deterministic Lingmo v7 delivery for frozen lite 0.1.2.
//! Complete author/source/fantasy quality checks remain a mandatory explicit gate.
//! Neither mechanical checks nor a matching MD imply independent semantic approval.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
#[path = "series_lite_bundle_json.rs"]
mod ordered_json;
#[path = "series_lite_bundle_parse.rs"]
mod parse;
#[path = "series_lite_bundle_project.rs"]
mod project;
use ordered_json::Ordered;
type Result<T> = std::result::Result<T, String>;
const TICKS: i64 = 1_000_000_000;
const MAX_SAFE: i64 = 9_007_199_254_740_991;
fn need(ok: bool, code: &str, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("{code}: {message}"))
    }
}
fn hash(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
fn text(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim().is_empty() && !s.contains(['\0', '\r']))
}
fn array<'a>(v: &'a Value, code: &str) -> Result<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| format!("{code}: expected array"))
}
fn string<'a>(v: &'a Value, code: &str) -> Result<&'a str> {
    v.as_str().ok_or_else(|| format!("{code}: expected text"))
}
fn same(a: &Value, b: &Value) -> bool {
    crate::series_lite_registry::canonical(a).ok() == crate::series_lite_registry::canonical(b).ok()
}
fn empty_array() -> Value {
    json!([])
}
fn optional_array<'a>(v: &'a Value, key: &str, empty: &'a Value) -> Result<&'a Vec<Value>> {
    array(v.get(key).unwrap_or(empty), "BUNDLE_SHAPE")
}
pub fn time_ticks(v: &Value, positive: bool, code: &str) -> Result<i64> {
    let n = v
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("{code}: time must be finite"))?;
    need(
        if positive { n > 0.0 } else { n >= 0.0 },
        code,
        "invalid time sign",
    )?;
    let scaled = n * TICKS as f64;
    let rounded = scaled.round();
    need(
        rounded.is_finite()
            && rounded.abs() <= MAX_SAFE as f64
            && !(n > 0.0 && rounded == 0.0)
            && (scaled - rounded).abs() <= f64::EPSILON * scaled.abs().max(1.0) * 4.0,
        "TIME_PRECISION",
        "time exceeds nine decimal precision",
    )?;
    Ok(rounded as i64)
}
pub fn time_text_ticks(ticks: i64) -> String {
    let whole = ticks / TICKS;
    let frac = ticks % TICKS;
    if frac == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{:09}", frac).trim_end_matches('0').into()
    }
}
fn container_seconds(ticks: i64) -> i64 {
    4.max((ticks + TICKS - 1) / TICKS)
}

#[derive(Clone, Debug)]
pub struct GateEvidence {
    pub bundle_sha256: String,
    /// In exact bundle episode order, each value is normalizeSource(e.source).
    pub normalized_sources: Vec<Value>,
    pub report: Value,
    pub validator_id: String,
}
/// No default/no-op implementation. The integration must supply complete frozen
/// normalizeSource + checkBundle + validateDraft semantics and source-lock checks.
pub trait BundleGate {
    fn check(&mut self, bundle_raw: &[u8]) -> Result<GateEvidence>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub bundle_sha256: String,
    pub markdown: String,
    pub markdown_sha256: String,
    pub manifest_json: String,
    pub manifest_sha256: String,
    pub projection: Value,
    pub gate_report: Value,
    pub gate_validator: String,
    pub validation: String,
}
/// Render one already verified projection with the exact same delivery writer.
pub(crate) fn render_projected_unit(unit: &Value) -> Result<String> {
    project::render_unit(unit)
}
pub fn export_lingmo_v7(bundle_raw: &[u8], gate: &mut impl BundleGate) -> Result<Delivery> {
    let ordered: Ordered =
        serde_json::from_slice(bundle_raw).map_err(|e| format!("BUNDLE_JSON: {e}"))?;
    let bundle = ordered.value()?;
    let evidence = gate.check(bundle_raw)?;
    need(
        evidence.bundle_sha256 == hash(bundle_raw) && !evidence.validator_id.trim().is_empty(),
        "BUNDLE_GATE_BINDING",
        "gate receipt does not bind exact input bytes",
    )?;
    check_mechanical(&bundle, &evidence.normalized_sources)?;
    let projection = project::project(&bundle, &evidence.normalized_sources)?;
    let markdown = project::render(&projection)?;
    let parsed = parse::parse(&markdown)?;
    need(
        same(&parsed, &projection),
        "VERIFY_MISMATCH",
        "independent parsed projection differs",
    )?;
    let bundle_sha256 = hash(bundle_raw);
    let manifest = project::catalog(&bundle, &ordered, &projection, &markdown, &bundle_sha256)?;
    let manifest_json = manifest.stringify(true)? + "\n";
    Ok(Delivery {
        bundle_sha256,
        markdown_sha256: hash(markdown.as_bytes()),
        markdown,
        manifest_sha256: hash(manifest_json.as_bytes()),
        manifest_json,
        projection,
        gate_report: evidence.report,
        gate_validator: evidence.validator_id,
        validation:
            "mechanical-delivery-only; complete-bundle-gate-required; semantic-review-not-proven"
                .into(),
    })
}
pub fn verify_lingmo_v7(
    bundle_raw: &[u8],
    markdown: &str,
    manifest_json: &str,
    gate: &mut impl BundleGate,
) -> Result<Value> {
    let expected = export_lingmo_v7(bundle_raw, gate)?;
    let actual = parse::parse(markdown)?;
    need(
        same(&actual, &expected.projection),
        "VERIFY_MISMATCH",
        "visible fields differ from adopted bundle",
    )?;
    let manifest: Value =
        serde_json::from_str(manifest_json).map_err(|e| format!("VERIFY_ASSET_INDEX: {e}"))?;
    let expected_manifest: Value = serde_json::from_str(&expected.manifest_json).unwrap();
    need(
        same(&manifest, &expected_manifest),
        "VERIFY_ASSET_INDEX",
        "manifest filenames/revisions/bodies differ",
    )?;
    // Parser is independent; this additional byte check binds the exact delivery
    // hash so equivalent-looking unapproved formatting is not silently accepted.
    need(
        markdown == expected.markdown,
        "VERIFY_MISMATCH",
        "MD exact bytes differ",
    )?;
    Ok(
        json!({"ok":true,"bundleSha256":expected.bundle_sha256,"markdownSha256":hash(markdown),"manifestSha256":hash(manifest_json),"proof":"独立反解可见字段与绑定目录一致；不代表语义独审、审图或平台验收"}),
    )
}
pub fn sync_asset_consumption(
    bundle_raw: &[u8],
    registry: &Value,
    registry_snapshot: &str,
    registry_raw: &[u8],
    gate: &mut impl BundleGate,
) -> Result<Value> {
    let bundle: Value = serde_json::from_slice(bundle_raw).map_err(|e| e.to_string())?;
    let evidence = gate.check(bundle_raw)?;
    need(
        evidence.bundle_sha256 == hash(bundle_raw),
        "BUNDLE_GATE_BINDING",
        "wrong input receipt",
    )?;
    check_mechanical(&bundle, &evidence.normalized_sources)?;
    let registry_parsed: Value = serde_json::from_slice(registry_raw).map_err(|e| e.to_string())?;
    need(
        same(registry, &registry_parsed),
        "ASSET_REGISTRY_BINDING",
        "registry bytes and value differ",
    )?;
    let report = crate::series_lite_registry::check_asset_bindings(&bundle, registry)?;
    let mut receipt = json!({"version":"series-asset-consumption-v1","sourceHash":bundle["sourceHash"],"bundleSha256":hash(bundle_raw),"registrySnapshot":registry_snapshot,"registrySha256":hash(registry_raw)});
    receipt
        .as_object_mut()
        .unwrap()
        .extend(report.as_object().unwrap().clone());
    if let Some(scope) = registry.get("productionScope") {
        receipt["productionScopeHash"] =
            json!(crate::series_lite_registry::hash_season_scope(scope)?);
    }
    Ok(receipt)
}

pub fn source_lock_hash(lock_raw: &[u8]) -> Result<String> {
    let ordered: Ordered =
        serde_json::from_slice(lock_raw).map_err(|e| format!("SOURCE_LOCK_HASH: {e}"))?;
    Ok(hash(ordered.stringify(false)?))
}
/// Inputs must be produced by the complete frozen normalizer gate. This binds its
/// normalized results to a separately frozen lock identity and original episode order;
/// it does not pretend that comparing raw strings alone implements normalizeSource.
pub fn verify_source_lock(
    bundle_raw: &[u8],
    lock_raw: &[u8],
    bundle_sources: &[Value],
    lock_sources: &[Value],
) -> Result<Value> {
    let bundle: Value =
        serde_json::from_slice(bundle_raw).map_err(|e| format!("SOURCE_LOCK_HASH: {e}"))?;
    let lock: Value =
        serde_json::from_slice(lock_raw).map_err(|e| format!("SOURCE_LOCK_HASH: {e}"))?;
    need(
        lock["version"] == "series-source-lock-v1",
        "SOURCE_LOCK_REQUIRED",
        "invalid lock version",
    )?;
    let expected = source_lock_hash(lock_raw)?;
    need(
        bundle["sourceLockHash"] == expected,
        "SOURCE_LOCK_HASH",
        "lock identity differs",
    )?;
    let source_hash = string(&lock["sourceHash"], "SOURCE_LOCK_HASH")?;
    need(
        source_hash.len() == 64
            && source_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && bundle
                .get("sourceHash")
                .is_none_or(|v| v.is_null() || *v == source_hash),
        "SOURCE_LOCK_HASH",
        "original source differs",
    )?;
    let locked = array(&lock["episodes"], "SOURCE_LOCK_SCOPE")?;
    let episodes = array(&bundle["episodes"], "SOURCE_LOCK_SCOPE")?;
    let mut seen = HashSet::new();
    need(
        lock_sources.len() == locked.len() && bundle_sources.len() == episodes.len(),
        "SOURCE_LOCK_SCOPE",
        "normalized source counts differ",
    )?;
    for e in locked {
        need(
            e["number"]
                .as_f64()
                .is_some_and(|n| n.is_finite() && n.fract() == 0.0)
                && seen.insert(crate::series_lite_registry::canonical(&e["number"])?),
            "SOURCE_LOCK_SCOPE",
            "duplicate/invalid lock episode",
        )?;
    }
    let mut previous = None;
    for (i, e) in episodes.iter().enumerate() {
        let pos = locked
            .iter()
            .position(|l| same(&l["number"], &e["number"]))
            .ok_or("SOURCE_LOCK_SCOPE: episode absent from frozen scope")?;
        need(
            previous.is_none_or(|p| pos > p),
            "SOURCE_LOCK_SCOPE",
            "locked source order differs",
        )?;
        previous = Some(pos);
        need(
            same(&bundle_sources[i], &lock_sources[pos]),
            "SOURCE_LOCK_SOURCE",
            "normalized source differs from pre-author frozen source",
        )?;
    }
    Ok(
        json!({"mode":"locked-source-v1","sourceLockHash":expected,"proof":"分镜对照创作前冻结的接入source；接入文本与原件仍须核源"}),
    )
}

pub fn validate_visual_baseline(b: &Value) -> Result<()> {
    let required = [
        "version",
        "id",
        "revision",
        "status",
        "visualStyle",
        "coverage",
    ];
    let optional = ["adoptionBasis"];
    need(
        b.as_object().is_some_and(|o| {
            required.iter().all(|k| o.contains_key(*k))
                && o.keys()
                    .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
        }),
        "VISUAL_BASELINE",
        "unknown/missing baseline fields",
    )?;
    need(
        b["version"] == "series-visual-baseline-v1"
            && text(&b["id"])
            && !b["id"].as_str().unwrap().contains('\n')
            && b["revision"]
                .as_f64()
                .is_some_and(|n| n > 0.0 && n <= MAX_SAFE as f64 && n.fract() == 0.0)
            && b["status"] == "adopted"
            && text(&b["visualStyle"])
            && text(&b["adoptionBasis"]),
        "VISUAL_BASELINE",
        "invalid adopted baseline",
    )?;
    let keys = [
        "form",
        "rendering",
        "materials",
        "colorLighting",
        "aspectRatio",
        "localExceptions",
    ];
    let coverage = b["coverage"]
        .as_object()
        .ok_or("VISUAL_BASELINE: coverage missing")?;
    need(
        coverage.len() == keys.len()
            && keys.iter().all(|k| {
                coverage.get(*k).is_some_and(|v| {
                    text(v)
                        && b["visualStyle"]
                            .as_str()
                            .unwrap()
                            .contains(v.as_str().unwrap())
                })
            }),
        "VISUAL_BASELINE",
        "coverage must quote full actual baseline",
    )?;
    need(
        coverage
            .values()
            .map(|v| v.as_str().unwrap())
            .collect::<HashSet<_>>()
            .len()
            > 1,
        "VISUAL_BASELINE",
        "coverage cannot repeat one tag for everything",
    )
}
pub fn compose_general_prompt(settings: &Value, direction: &Value) -> Result<String> {
    if let Some(b) = settings.get("visualBaseline") {
        validate_visual_baseline(b)?;
        need(
            settings["visualStyle"] == b["visualStyle"],
            "VISUAL_BASELINE",
            "style differs from full adopted baseline",
        )?;
    }
    Ok([
        &settings["visualStyle"],
        &settings["prohibitions"],
        direction,
    ]
    .into_iter()
    .filter(|v| text(v))
    .map(|v| v.as_str().unwrap())
    .collect::<Vec<_>>()
    .join("\n"))
}
fn check_visual_baselines(bundle: &Value) -> Result<()> {
    let episodes = array(&bundle["episodes"], "VISUAL_BASELINE")?;
    need(!episodes.is_empty(), "VISUAL_BASELINE", "empty episodes")?;
    let present: Vec<_> = episodes
        .iter()
        .filter_map(|e| e["settings"].get("visualBaseline"))
        .collect();
    if present.is_empty() {
        return Ok(());
    }
    need(
        present.len() == episodes.len(),
        "VISUAL_BASELINE",
        "mixed baseline presence",
    )?;
    for e in episodes {
        let b = &e["settings"]["visualBaseline"];
        validate_visual_baseline(b)?;
        need(
            same(b, present[0]) && e["settings"]["visualStyle"] == b["visualStyle"],
            "VISUAL_BASELINE",
            "baseline/style differs across episodes",
        )?;
    }
    Ok(())
}

/// Native safety subset before projection. Complete narrative/fantasy/source
/// normalization checks are deliberately not claimed by this function.
pub fn check_mechanical(bundle: &Value, sources: &[Value]) -> Result<()> {
    need(
        bundle["version"] == "series-storyboard-bundle-v1",
        "BUNDLE_VERSION",
        "unknown version",
    )?;
    need(
        bundle["exportProfile"] == "lingmo-v7",
        "EXPORT_PROFILE",
        "this native delivery module only implements Lingmo v7",
    )?;
    need(
        bundle["testFixture"].is_boolean() && text(&bundle["title"]),
        "BUNDLE",
        "title/testFixture missing",
    )?;
    need(
        array(&bundle["unresolved"], "UNRESOLVED")?.is_empty(),
        "UNRESOLVED",
        "unresolved findings",
    )?;
    crate::series_lite_registry::validate_import_mappings(&bundle["assets"])?;
    check_visual_baselines(bundle)?;
    for a in array(&bundle["assets"], "ASSETS")? {
        need(
            text(&a["key"])
                && text(&a["name"])
                && text(&a["status"])
                && a["key"]
                    == format!(
                        "{}{}",
                        a["name"].as_str().unwrap_or(""),
                        a["type"].as_str().unwrap_or("")
                    ),
            "ASSET",
            "invalid asset key/name/status",
        )?;
    }
    let assets: BTreeMap<_, _> = array(&bundle["assets"], "ASSETS")?
        .iter()
        .map(|a| (a["key"].as_str().unwrap(), a))
        .collect();
    need(
        assets.len() == bundle["assets"].as_array().unwrap().len(),
        "ASSET_DUPLICATE",
        "duplicate key",
    )?;
    let episodes = array(&bundle["episodes"], "SCOPE")?;
    let scope = array(&bundle["scope"], "SCOPE")?;
    need(
        !scope.is_empty() && scope.len() == episodes.len() && sources.len() == episodes.len(),
        "EPISODE_COVERAGE",
        "scope/source count differs",
    )?;
    let mut episode_seen = HashSet::new();
    for (i, e) in episodes.iter().enumerate() {
        need(
            e["number"].as_f64().is_some_and(|n| n.fract() == 0.0)
                && same(&e["number"], &scope[i]["number"])
                && episode_seen.insert(crate::series_lite_registry::canonical(&e["number"])?),
            "EPISODE_ORDER",
            "episode order mismatch",
        )?;
        let scenes = array(&e["scenes"], "SCENE_COVERAGE")?;
        let scene_ids: Vec<_> = scenes.iter().map(|s| s["id"].clone()).collect();
        need(
            same(&json!(scene_ids), &scope[i]["sceneIds"]),
            "SCENE_COVERAGE",
            "scene coverage differs",
        )?;
        let source = &sources[i];
        let rows = array(&source["sourceRows"], "SOURCE_BINDING")?;
        let quotes = array(&source["dialogue"], "SOURCE_BINDING")?;
        let mut ownership = BTreeMap::new();
        let mut previous_row = None;
        for scene in scenes {
            for r in array(&scene["sourceRefs"], "SCENE_SOURCE")? {
                let pos = rows
                    .iter()
                    .position(|v| v["id"] == *r)
                    .ok_or("SCENE_SOURCE: unknown ref")?;
                need(
                    previous_row.is_none_or(|p| pos > p)
                        && ownership
                            .insert(
                                string(r, "SCENE_SOURCE")?.to_string(),
                                string(&scene["id"], "SCENE_SOURCE")?.to_string(),
                            )
                            .is_none(),
                    "SCENE_SOURCE",
                    "duplicate/reordered source ownership",
                )?;
                previous_row = Some(pos);
            }
        }
        need(
            ownership.len() == rows.len(),
            "SCENE_SOURCE",
            "missing source row ownership",
        )?;
        let mut spoken: BTreeMap<String, String> = BTreeMap::new();
        let mut previous_scene = None;
        for u in array(&e["draft"]["units"], "BUNDLE_SHAPE")? {
            let scene_pos = scenes
                .iter()
                .position(|s| s["id"] == u["sceneId"])
                .ok_or("UNIT_SCENE: missing scene")?;
            need(
                previous_scene.is_none_or(|p| scene_pos >= p),
                "SCENE_ORDER",
                "unit scene backwards",
            )?;
            previous_scene = Some(scene_pos);
            let bindings = array(&u["bindings"], "UNIT_BINDINGS")?;
            let images = array(&u["imageBindings"], "IMAGE_BINDINGS")?;
            need(
                bindings
                    .iter()
                    .all(|k| k.as_str().is_some_and(|s| assets.contains_key(s)))
                    && bindings.iter().collect::<HashSet<_>>().len() == bindings.len(),
                "UNIT_BINDINGS",
                "unknown/duplicate binding",
            )?;
            need(
                images.len() <= 30
                    && images.iter().all(|k| bindings.contains(k))
                    && images.iter().collect::<HashSet<_>>().len() == images.len(),
                "IMAGE_BINDINGS",
                "invalid image references",
            )?;
            let mut total = 0i64;
            let mut shot_ids = HashSet::new();
            for shot in array(&u["shots"], "BUNDLE_SHAPE")? {
                need(
                    text(&shot["id"]) && shot_ids.insert(shot["id"].as_str().unwrap()),
                    "SHOT_ID",
                    "invalid/duplicate shot",
                )?;
                let ticks = time_ticks(&shot["duration"], true, "DURATION")?;
                total = total
                    .checked_add(ticks)
                    .ok_or("UNIT_RANGE: duration overflow")?;
                let mut previous_end = 0;
                for r in array(&shot["sourceRefs"], "SOURCE_BINDING")? {
                    need(
                        ownership
                            .get(string(r, "SOURCE_BINDING")?)
                            .is_some_and(|s| u["sceneId"] == *s),
                        "UNIT_SOURCE_SCENE",
                        "shot source crosses scene",
                    )?;
                }
                for part in array(&shot["dialogueParts"], "BUNDLE_SHAPE")? {
                    let start = time_ticks(&part["voiceStart"], false, "VOICE_WINDOW")?;
                    let end = time_ticks(&part["voiceEnd"], false, "VOICE_WINDOW")?;
                    need(
                        end > start && end <= ticks && start >= previous_end,
                        "VOICE_WINDOW",
                        "overlap/outside voice window",
                    )?;
                    previous_end = end;
                    let q = quotes
                        .iter()
                        .find(|q| q["id"] == part["lineId"])
                        .ok_or("DIALOGUE_UNKNOWN: missing original line")?;
                    for r in array(&q["sourceIds"], "SOURCE_BINDING")? {
                        need(
                            ownership
                                .get(string(r, "SOURCE_BINDING")?)
                                .is_some_and(|s| u["sceneId"] == *s),
                            "DIALOGUE_SOURCE_SCENE",
                            "dialogue crosses source scene",
                        )?;
                    }
                    let id = string(&q["id"], "SOURCE_BINDING")?;
                    spoken
                        .entry(id.into())
                        .or_default()
                        .push_str(string(&part["text"], "DIALOGUE_TEXT")?);
                }
                for field in ["camera", "picture", "lighting", "sound", "transition"] {
                    need(text(&shot[field]), "SHOT_FIELDS", field)?;
                }
            }
            let cap = match e["settings"]["maxUnitSeconds"].as_f64() {
                Some(15.0) => 15,
                _ => 30,
            };
            need(
                total > 0 && total <= cap * TICKS,
                "UNIT_RANGE",
                "actual unit duration exceeds project cap",
            )?;
        }
        for q in quotes {
            need(
                spoken
                    .get(string(&q["id"], "SOURCE_BINDING")?)
                    .map(String::as_str)
                    == q["quote"].as_str(),
                "DIALOGUE_COVERAGE",
                "spoken text does not exactly reconstruct source quote",
            )?;
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "series_lite_bundle_tests.rs"]
mod tests;
