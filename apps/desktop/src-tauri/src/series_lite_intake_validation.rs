//! Reuses the already-conformed lexical validators without changing their delivered modules.
//! Compiler errors are mapped back to the frozen compiler's public code vocabulary.
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
type Result<T> = std::result::Result<T, String>;
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(code.into())
    }
}
fn arr<'a>(v: &'a Value, c: &str) -> Result<&'a Vec<Value>> {
    v.as_array().ok_or_else(|| c.to_string())
}
fn st(v: &Value) -> &str {
    super::st(v)
}
fn n(v: &Value) -> Option<usize> {
    super::number(v)
}
fn ws(c: char) -> bool {
    super::ws(c)
}
fn trim(s: &str) -> &str {
    super::trim(s)
}
fn re(p: &str) -> Regex {
    super::re(p)
}
fn len(s: &str) -> usize {
    super::utf16_len(s)
}
fn slice(s: &str, a: usize, b: usize) -> Result<String> {
    super::slice(s, a, b).map_err(|e| e.to_string())
}
fn tail(s: &str, a: usize) -> String {
    slice(s, a, len(s)).unwrap_or_default()
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
#[allow(dead_code)]
#[path = "series_lite_bundle_standard_inline.rs"]
mod inline;
use inline::compact_performance_head_projection;
#[allow(dead_code)]
#[path = "series_lite_bundle_standard_headers.rs"]
mod headers;
fn mapped_error(error: String, path: &str) -> super::IntakeError {
    let code = error.split(':').next().unwrap_or(&error);
    let mapped = match code {
        "STANDARD_COMPACT_VOICE" => "COMPACT_VOICE_FIELDS",
        "STANDARD_POST_CUE_HEADER" => "POST_CUE_HEADER",
        "STANDARD_POST_CUE_CONTENT" => "POST_CUE_CONTENT",
        "STANDARD_POST_CUE_SPEAKER" => "POST_CUE_SPEAKER",
        "STANDARD_POST_CUE_CHANNEL" => "POST_CUE_CHANNEL",
        "STANDARD_POST_CUE_AMBIGUOUS" => "POST_CUE_AMBIGUOUS",
        "STANDARD_POST_CUE_FIELDS" => "POST_CUE_FIELDS",
        "STANDARD_GROUP_QUOTE" => "DIALOGUE_GROUP_QUOTE",
        "STANDARD_GROUP_ACTION" => "DIALOGUE_GROUP_ACTION",
        "STANDARD_GROUP_NESTED_HEAD" => "DIALOGUE_GROUP_NESTED_HEAD",
        "STANDARD_GROUP_HEADER" => "DIALOGUE_GROUP_HEADER",
        "STANDARD_GROUP_CHANNEL" => "DIALOGUE_GROUP_CHANNEL",
        "STANDARD_BODY_HEADER" => "BODY_GROUP_HEADER",
        "STANDARD_BODY_CHANNEL" => "BODY_GROUP_CHANNEL",
        _ => code,
    };
    super::err(mapped, path)
}
pub(super) fn compact(s: &str, c: &Value, v5: bool, path: &str) -> super::IntakeResult<()> {
    headers::compact_voice(s, c, v5)
        .map(|_| ())
        .map_err(|e| mapped_error(e, path))
}
pub(super) fn post(s: &str, c: &Value) -> bool {
    headers::post_fields(s, c)
}
pub(super) fn post_header(
    s: &str,
    a: usize,
    b: usize,
    c: &Value,
    path: &str,
) -> super::IntakeResult<()> {
    headers::post_header(s, a, b, c).map_err(|e| mapped_error(e, path))
}
pub(super) fn normal_header(
    s: &str,
    a: usize,
    b: usize,
    c: &Value,
    v5: bool,
    mixed: bool,
    path: &str,
) -> super::IntakeResult<()> {
    headers::normal_header(s, a, b, c, v5, mixed).map_err(|e| mapped_error(e, path))
}
pub(super) fn utterance(c: &Value, path: &str) -> super::IntakeResult<()> {
    headers::group_utterance(
        st(&c["text"]),
        st(&c["speaker"]["text"]),
        st(&c["cue"]["text"]),
    )
    .map_err(|e| mapped_error(e, path))
}
