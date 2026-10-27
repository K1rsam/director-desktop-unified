//! The adopted project baseline is frozen input, not model-authored metadata.
//! Legacy deliveries remain bound to their original rule pack and renderer.
use super::*;

pub(super) const VERSION: &str = "series-visual-baseline-v1";
const KEYS: [&str; 6] = ["form", "rendering", "materials", "colorLighting", "aspectRatio", "localExceptions"];
const HELD: &str = "SERIES_VISUAL_BASELINE_HELD:请在项目制作设置采用完整视觉基准，并确认画幅与六项原文依据一致";

pub(super) fn enabled(input: &Value) -> bool {
    input["seriesSkillVersion"] == rules::VERSION
}

pub(super) fn validate(input: &Value) -> Result<(), String> {
    if !enabled(input) { return Ok(()); }
    let settings = &input["settings"];
    let b = &settings["visualBaseline"];
    let fields = b.as_object().ok_or(HELD)?;
    let expected = ["version", "id", "revision", "status", "visualStyle", "coverage", "adoptionBasis"];
    if fields.len() != expected.len() || expected.iter().any(|key| !fields.contains_key(*key))
        || b["version"] != VERSION || b["status"] != "adopted"
        || b["revision"].as_u64().is_none_or(|n| n == 0 || n > 9_007_199_254_740_991)
        || s(b, "id").trim().is_empty() || s(b, "id").contains(['\r', '\n', '\0'])
        || s(b, "visualStyle").trim().is_empty() || s(b, "visualStyle").contains(['\r', '\0'])
        || s(b, "adoptionBasis").trim().is_empty()
        || settings["visualStyle"] != b["visualStyle"]
        || settings.get("projectVisualStyle").is_some_and(|v| v != &b["visualStyle"])
    { return Err(HELD.into()); }
    let coverage = b["coverage"].as_object().ok_or(HELD)?;
    if coverage.len() != KEYS.len() { return Err(HELD.into()); }
    let mut distinct = HashSet::new();
    for key in KEYS {
        let excerpt = coverage.get(key).and_then(Value::as_str).ok_or(HELD)?;
        if excerpt.trim().is_empty() || excerpt.contains(['\r', '\0']) || !s(b, "visualStyle").contains(excerpt) {
            return Err(HELD.into());
        }
        distinct.insert(excerpt);
    }
    let ratio = settings.get("ratio").or_else(|| settings.get("aspectRatio")).and_then(Value::as_str).ok_or(HELD)?;
    let valid_ratio = ratio.split_once(':').is_some_and(|(w,h)| {
        [w,h].iter().all(|v| !v.is_empty() && !v.starts_with('0') && v.bytes().all(|x| x.is_ascii_digit()) && v.parse::<u32>().is_ok())
    });
    let ratio_tokens: Vec<_> = s(&b["coverage"], "aspectRatio").split(|c: char| !c.is_ascii_digit() && c != ':').collect();
    if distinct.len() <= 1 || !valid_ratio || !ratio_tokens.contains(&ratio) { return Err(HELD.into()); }
    Ok(())
}

fn binding(input: &Value) -> Value {
    let b = &input["settings"]["visualBaseline"];
    json!({"version":VERSION,"id":b["id"],"revision":b["revision"],"sha256":hash(b),"visualStyle":b["visualStyle"]})
}

pub(super) fn materialize(draft: &mut Value, input: &Value) -> Result<(), String> {
    validate(input)?;
    if !enabled(input) { return Ok(()); }
    let expected = binding(input);
    let style = s(&input["settings"]["visualBaseline"], "visualStyle");
    for unit in draft["units"].as_array_mut().ok_or(HELD)? {
        if unit.get("visualBaselineBinding").is_some_and(|v| v != &expected)
            || !s(unit, "prompt").contains(style)
            || s(unit, "unitDirection").trim().is_empty()
        { return Err(HELD.into()); }
        unit["visualBaselineBinding"] = expected.clone();
    }
    Ok(())
}

pub(super) fn export_style(unit: &Value, style: &str) -> Result<(), String> {
    if let Some(b) = unit.get("visualBaselineBinding") {
        if b["version"] != VERSION || b["visualStyle"] != style || s(b, "sha256").len() != 64 {
            return Err(HELD.into());
        }
    }
    Ok(())
}

pub(super) fn bind_bundle(bundle: &mut Value, input: &Value) {
    if enabled(input) {
        bundle["settings"] = json!({"visualBaseline":input["settings"]["visualBaseline"],"visualStyle":input["settings"]["visualBaseline"]["visualStyle"]});
    }
}

pub(super) fn validate_delivery(input: &Value, delivery: &Value) -> Result<(), String> {
    validate(input)?;
    if !enabled(input) { return Ok(()); }
    let mut expected = json!({});
    bind_bundle(&mut expected, input);
    if delivery["bundle"]["settings"] != expected["settings"] { return Err(HELD.into()); }
    for unit in delivery["bundle"]["draft"]["units"].as_array().ok_or(HELD)? {
        if unit["visualBaselineBinding"] != binding(input) { return Err(HELD.into()); }
    }
    Ok(())
}

/// Independent handoff readback must not trust a rewritten direction manifest.
pub(crate) fn validate_handoff(settings: &Value, delivery: &Value) -> Result<(), String> {
    let roles = delivery["adoptionReceipt"]["roleReceipts"].as_array().ok_or(HELD)?;
    let has_baseline = delivery["bundle"]["settings"].get("visualBaseline").is_some()
        || roles.iter().any(|r| r["request"]["input"]["packet"]["bindings"].as_array().is_some_and(|bs| bs.iter().any(|b|b["kind"] == "visual-baseline")));
    if !has_baseline { return Ok(()); }
    let input = json!({"seriesSkillVersion":rules::VERSION,"settings":settings});
    validate_delivery(&input, delivery)?;
    for role in roles {
        if packet_value(&role["request"]["input"]["packet"], "visual-baseline")? != &settings["visualBaseline"] {
            return Err(HELD.into());
        }
    }
    Ok(())
}
