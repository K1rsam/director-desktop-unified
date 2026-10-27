//! Reuse the independent visible v7 parser without imposing delivery-byte identity.
use serde_json::{json, Value};
type Result<T> = std::result::Result<T, String>;
const TICKS: i64 = 1_000_000_000;
fn container_seconds(ticks: i64) -> i64 {
    4.max((ticks + TICKS - 1) / TICKS)
}
fn need(ok: bool, code: &str, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("{code}: {message}"))
    }
}
#[path = "series_lite_bundle_parse.rs"]
mod reader;
pub(super) fn parse(markdown: &str) -> Result<Value> {
    reader::parse(markdown)
}
