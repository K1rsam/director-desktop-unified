//! Frozen lite 0.1.2 standard-source proof verifier. No legacy fallback.
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
pub const SOURCE_COMMIT: &str = "6dea8d6c57dde38f04a4d71f05f4c039004dd4a9";
pub const SOURCE_STANDARD_SHA256: &str =
    "b56cb5be2d6cd3cf3035f801af9340089a2731aca299435edfe0617ea999e88e";
pub const SOURCE_INLINE_SHA256: &str =
    "45825afacb2317fc4fa9f4b116f4605bb72df493694a95f5aed19fb53538d4da";
type Result<T> = std::result::Result<T, String>;
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("{code}: frozen standard-source contract violated"))
    }
}
fn arr<'a>(v: &'a Value, code: &str) -> Result<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| format!("{code}: expected array"))
}
fn st(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn n(v: &Value) -> Option<usize> {
    v.as_f64()
        .filter(|x| x.is_finite() && *x >= 0. && *x <= 9_007_199_254_740_991. && x.fract() == 0.)
        .and_then(|x| usize::try_from(x as u64).ok())
}
fn ws(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn trim(s: &str) -> &str {
    s.trim_matches(ws)
}
fn re(p: &str) -> Regex {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Regex>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .expect("regex cache");
    cache.entry(p.to_string()).or_insert_with(||Regex::new(&p.replace(r"\s",r"[\x09-\x0d\x20\u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]").replace(r"\d","[0-9]")).expect("static port regex")).clone()
}
fn len(s: &str) -> usize {
    s.encode_utf16().count()
}
fn slice(s: &str, a: usize, b: usize) -> Result<String> {
    let u: Vec<_> = s.encode_utf16().collect();
    need(a <= b && b <= u.len(), "STANDARD_PARTS")?;
    String::from_utf16(&u[a..b]).map_err(|_| "STANDARD_PARTS: split UTF16 surrogate".into())
}
fn tail(s: &str, a: usize) -> String {
    slice(s, a, len(s)).unwrap_or_default()
}
fn canonical(v: &Value) -> Result<String> {
    Ok(match v {
        Value::Object(o) => {
            let mut keys: Vec<_> = o.keys().collect();
            keys.sort_by(|a, b| {
                let index = |s: &str| {
                    s.parse::<u32>()
                        .ok()
                        .filter(|i| *i < u32::MAX && i.to_string() == s)
                };
                match (index(a), index(b)) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => a.encode_utf16().cmp(b.encode_utf16()),
                }
            });
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| Ok(format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical(&o[k])?
                    )))
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            )
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(canonical)
                .collect::<Result<Vec<_>>>()?
                .join(",")
        ),
        _ => crate::series_lite_registry::canonical(v)?,
    })
}
pub fn standard_fingerprint(v: &Value) -> Result<String> {
    let bytes: Vec<_> = canonical(v)?
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn equal(a: &Value, b: &Value) -> bool {
    canonical(a).ok() == canonical(b).ok()
}
fn part_start(v: &Value) -> Option<usize> {
    v.as_array()?.first().and_then(|p| n(&p["start"]))
}
fn part_end(v: &Value) -> Option<usize> {
    v.as_array()?.last().and_then(|p| n(&p["end"]))
}
fn copy_parts(s: &str, v: &Value) -> Result<String> {
    arr(v, "STANDARD_PARTS")?
        .iter()
        .map(|p| {
            slice(
                s,
                n(&p["start"]).ok_or("STANDARD_PARTS")?,
                n(&p["end"]).ok_or("STANDARD_PARTS")?,
            )
        })
        .collect()
}
fn voice(channel: &str) -> Option<&'static str> {
    match channel {
        "SPEECH" => Some("现场对白"),
        "OS" => Some("OS"),
        "VO" | "NARRATION" => Some("VO"),
        "SYSTEM" => Some("系统音"),
        _ => None,
    }
}
#[path = "series_lite_bundle_standard_inline.rs"]
mod inline;
pub use inline::{compact_performance_head_projection, inline_performance_projection};
#[path = "series_lite_bundle_standard_headers.rs"]
mod headers;
#[path = "series_lite_bundle_standard_package.rs"]
mod package;
pub use package::standard_package_source;
#[path = "series_lite_bundle_standard_rows.rs"]
mod rows;
pub use rows::{
    normalize_standard_source, standard_dialogue, standard_dialogues, standard_onscreen_texts,
    standard_row_kind, verify_standard_source,
};
#[cfg(test)]
#[path = "series_lite_bundle_standard_tests.rs"]
mod tests;
/// Version router: a declared standard package is verified and never downgraded.
pub fn normalize_source(raw: &Value) -> Result<Value> {
    if raw["sourceProtocol"] == "standard-episode-v1" {
        normalize_standard_source(raw)
    } else {
        crate::series_lite_bundle_gate::normalize_legacy_source(raw)
    }
}
