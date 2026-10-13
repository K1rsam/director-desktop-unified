use crate::store::{s, Result};
use serde_json::Value;

/// Manual text/material saves do not run the directing validators or rebuild
/// derived prompts. Existing structured shots must use refinement + adoption,
/// even when the caller also supplies replacement prompt fields.
pub(super) fn check_shots_unchanged(original: &Value, merged: &mut Value) -> Result<()> {
    if !s(original, "id").is_empty() && original.get("shots") != merged.get("shots") {
        let equal = original.get("shots").zip(merged.get("shots"))
            .is_some_and(|(before, after)| same_shot_value(before, after));
        if !equal {
            return Err("UNIT_SHOTS_EDIT_REQUIRES_REFINEMENT: 此保存入口用于文字和素材编辑；结构化镜头已改变，请通过导演精修后采用，确保镜头与提示词同步。".into());
        }
        // JS round trips render e.g. 3.0 as 3. Preserve the stored representation
        // so a text-only save cannot change shot bytes or create a false version.
        merged["shots"] = original["shots"].clone();
    }
    Ok(())
}

fn same_shot_value(before: &Value, after: &Value) -> bool {
    if before == after { return true; }
    match (before, after) {
        (Value::Array(a), Value::Array(b)) => a.len() == b.len()
            && a.iter().zip(b).all(|(a,b)| same_shot_value(a,b)),
        (Value::Object(a), Value::Object(b)) => a.len() == b.len()
            && a.iter().all(|(key,value)| b.get(key).is_some_and(|other| same_shot_value(value,other))),
        (Value::Number(a), Value::Number(b)) => same_integer_float(a,b) || same_integer_float(b,a),
        _ => false,
    }
}

fn same_integer_float(integer: &serde_json::Number, float: &serde_json::Number) -> bool {
    if !float.is_f64() { return false; }
    let Some(value) = float.as_f64() else { return false; };
    if value.fract() != 0.0 { return false; }
    // Do not collapse distinct large integers through a lossy f64 comparison.
    integer.as_i64().is_some_and(|number| value as i128 == number as i128)
        || integer.as_u64().is_some_and(|number| value >= 0.0 && value as u128 == number as u128)
}

#[cfg(test)]
#[path = "unit_save_guard_tests.rs"]
mod tests;
