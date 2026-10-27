//! Exact frozen lite 0.1.2 deterministic intake tools. No model inference.
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
pub const SOURCE_COMMIT: &str = "6dea8d6c57dde38f04a4d71f05f4c039004dd4a9";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntakeError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_unit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Value>,
}
impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(m) = &self.message {
            write!(f, "{}: {}", self.code, m)
        } else {
            write!(
                f,
                "{}: {}",
                self.code,
                self.field_path.as_deref().unwrap_or("intake")
            )
        }
    }
}
impl std::error::Error for IntakeError {}
impl From<String> for IntakeError {
    fn from(s: String) -> Self {
        Self {
            message: Some(s.clone()),
            code: s.split(':').next().unwrap_or("INTAKE_ERROR").to_string(),
            field_path: None,
            source_unit_id: None,
            diagnostics: vec![],
        }
    }
}
pub type IntakeResult<T> = std::result::Result<T, IntakeError>;
fn err(code: &str, path: &str) -> IntakeError {
    IntakeError {
        code: code.into(),
        message: None,
        field_path: Some(path.into()),
        source_unit_id: None,
        diagnostics: vec![],
    }
}
fn need(ok: bool, code: &str, path: &str) -> IntakeResult<()> {
    if ok {
        Ok(())
    } else {
        Err(err(code, path))
    }
}
fn st(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn number(v: &Value) -> Option<usize> {
    v.as_f64()
        .filter(|x| x.is_finite() && *x >= 0. && x.fract() == 0. && *x <= 9_007_199_254_740_991.)
        .map(|x| x as usize)
}
fn ws(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn trim(s: &str) -> &str {
    s.trim_matches(ws)
}
fn re(p: &str) -> Regex {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Regex>>> = OnceLock::new();
    let mut c = CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap();
    c.entry(p.to_string()).or_insert_with(||Regex::new(&p.replace(r"\s",r"[\x09-\x0d\x20\u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]").replace(r"\S",r"[^\x09-\x0d\x20\u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]").replace(r"\d","[0-9]")).expect("frozen regex")).clone()
}
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}
fn slice(s: &str, a: usize, b: usize) -> IntakeResult<String> {
    let u: Vec<_> = s.encode_utf16().collect();
    need(a <= b && b <= u.len(), "INVALID_RANGE", "source.parts")?;
    String::from_utf16(&u[a..b]).map_err(|_| err("UNICODE_BOUNDARY", "source.parts"))
}
fn hash(v: &Value) -> IntakeResult<String> {
    crate::series_lite_bundle_standard::standard_fingerprint(v).map_err(Into::into)
}
fn arr<'a>(v: &'a Value, path: &str) -> IntakeResult<&'a Vec<Value>> {
    v.as_array().ok_or_else(|| err("INVALID_ARRAY", path))
}
fn nonempty(v: &Value, path: &str) -> IntakeResult<()> {
    need(
        v.as_str().is_some_and(|s| !trim(s).is_empty()),
        "REQUIRED_STRING",
        path,
    )
}
fn one_of(v: &Value, allowed: &[&str], path: &str) -> IntakeResult<()> {
    need(
        v.as_str().is_some_and(|s| allowed.contains(&s)),
        "UNKNOWN_KIND",
        path,
    )
}
fn shape(v: &Value, keys: &[&str], required: &[&str], path: &str) -> IntakeResult<()> {
    let o = v.as_object().ok_or_else(|| err("INVALID_OBJECT", path))?;
    for k in o.keys() {
        need(
            keys.contains(&k.as_str()),
            "UNKNOWN_FIELD",
            &format!("{path}.{k}"),
        )?;
    }
    for k in required {
        need(o.contains_key(*k), "MISSING_FIELD", &format!("{path}.{k}"))?;
    }
    Ok(())
}
/// Original parseScript contract. Decorated coordinates are decoded without replacing the source text.
/// This is the legacy adapter; it does NOT fabricate a standard package or source receipt.
pub fn parse_script(text: &Value, options: &Value) -> IntakeResult<Value> {
    let text = text.as_str().ok_or_else(|| err("SCRIPT_TEXT", "text"))?;
    let split = re(r"\r\n|\n|\r");
    let lines: Vec<_> = split.split(text).collect();
    let mut rows = vec![];
    let numeric = |s: &str| -> IntakeResult<Value> {
        let n = s.parse::<f64>().unwrap_or(f64::INFINITY);
        need(n.is_finite(), "SOURCE_ROW_COORDINATE", "line")?;
        Ok(json!(n))
    };
    for (i, line) in lines.iter().enumerate() {
        if i == lines.len() - 1 && line.is_empty() {
            continue;
        }
        let row = if let Some(c) =
            re(r"(?i)^\s*L(\d+)\s*\|\s*P(\d+)(?:\s*\|)?[ \t　]+([^\r\n\x{2028}\x{2029}]*)$")
                .captures(line)
        {
            json!({"sourceId":format!("P{}",&c[2]),"line":numeric(&c[2])?,"sourceFileLine":numeric(&c[1])?,"numericLineAlias":false,"text":&c[3]})
        } else if let Some(c) =
            re(r"(?i)^\s*P(\d+)(?:\s*\|)?[ \t　]+([^\r\n\x{2028}\x{2029}]*)$").captures(line)
        {
            json!({"sourceId":format!("P{}",&c[1]),"line":numeric(&c[1])?,"numericLineAlias":false,"text":&c[2]})
        } else if let Some(c) =
            re(r"(?i)^\s*L(\d+)\s*\|\s?([^\r\n\x{2028}\x{2029}]*)$").captures(line)
        {
            json!({"line":numeric(&c[1])?,"text":&c[2]})
        } else {
            json!({"line":i+1,"text":line})
        };
        rows.push(row);
    }
    let mut raw = json!({"title":options.get("title").cloned().unwrap_or(json!("")),"context":options.get("context").cloned().unwrap_or(json!("")),"contextRows":options.get("contextRows").cloned().unwrap_or(json!([])),"sourceRows":rows});
    if let Some(key) = options.get("key") {
        raw["key"] = key.clone();
    }
    crate::series_lite_bundle_gate::normalize_legacy_source(&raw).map_err(Into::into)
}
pub fn create_source_index(text: &Value, source_id: &Value) -> IntakeResult<Value> {
    let text = text
        .as_str()
        .ok_or_else(|| err("INVALID_SOURCE_TEXT", "source.text"))?;
    nonempty(source_id, "source.sourceId")?;
    let version = format!("src:{}", hash(&json!([source_id, text]))?);
    let mut units = vec![];
    let mut offset = 0;
    for m in re(r"[^\r\n]*(?:\r\n|\r|\n|$)")
        .find_iter(text)
        .filter(|m| !m.is_empty())
    {
        let end = offset + utf16_len(m.as_str());
        units.push(json!({"id":format!("{version}:u{}",units.len()+1),"start":offset,"end":end,"text":m.as_str()}));
        offset = end;
    }
    Ok(
        json!({"schemaVersion":"STANDARD_SCRIPT_SOURCE_INDEX_V1","indexerVersion":"physical-lines-v1","sourceId":source_id,"sourceVersion":version,"offsetUnit":"UTF16","text":text,"units":units}),
    )
}
#[path = "series_lite_intake_projection.rs"]
mod projection;
pub use projection::{
    derive_standard_script_lines, explicit_subtitles, standard_script_plan_version,
};
#[path = "series_lite_intake_compile.rs"]
mod compile;
#[cfg(test)]
#[path = "series_lite_intake_tests.rs"]
mod tests;
#[path = "series_lite_intake_validation.rs"]
mod validation;
pub use compile::compile_standard_script;
#[path = "series_lite_intake_plan.rs"]
mod plan;
pub use plan::prepare_standard_episode;
#[path = "series_lite_intake_prepare.rs"]
mod prepare;
pub use prepare::prepare_source;
