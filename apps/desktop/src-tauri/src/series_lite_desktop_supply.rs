//! Read-only Desktop -> frozen lite supply inspection. No registry/adoption writes.
//! Native adapters live in series_lite_desktop_supply_native.rs. A verifier is
//! mandatory; this module has no trusting/default implementation.
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) const VERSION: &str = "desktop-lite-supply-evidence/v1";
pub(crate) const OLD_PROFILE: &str = "desktop-series672-assets112/v1";
type Result<T> = std::result::Result<T, String>;
fn hash(v: &Value) -> String {
    format!("{:x}", Sha256::digest(v.to_string().as_bytes()))
}
fn text(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.trim().is_empty())
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().ok_or("expected array".into())
}
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Gap {
    pub code: String,
    pub episode_id: Option<String>,
    pub asset_id: Option<String>,
    pub coordinates: Vec<String>,
    pub detail: String,
}
fn gap(
    code: &str,
    episode: Option<&str>,
    asset: Option<&str>,
    coordinates: Vec<String>,
    detail: impl Into<String>,
) -> Gap {
    Gap {
        code: code.into(),
        episode_id: episode.map(str::to_owned),
        asset_id: asset.map(str::to_owned),
        coordinates,
        detail: detail.into(),
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Inspection {
    pub version: String,
    pub ready: Vec<Value>,
    pub gaps: Vec<Gap>,
}
impl Inspection {
    fn new() -> Self {
        Self {
            version: VERSION.into(),
            ready: vec![],
            gaps: vec![],
        }
    }
}
pub(crate) trait SourceVerifier {
    /// Verify original Desktop parser contract AND exact prepared adapter output.
    fn verify(&self, locked: &Value, prepared: &Value, known_speakers: &[String]) -> Result<()>;
    /// Must use original source_number, never Desktop's ordinal `number`.
    fn original_episode_number(&self, source_number: &str) -> Option<u32>;
}
pub(crate) trait FrozenSeriesVerifier {
    /// Bind the original frozen rule/input validation and unique accepted jobs/raws.
    fn verify(
        &self,
        c: &Connection,
        input: &Value,
        delivery: &Value,
        workflow_id: &str,
    ) -> Result<()>;
}
#[path = "series_lite_desktop_supply_source.rs"]
mod source;
pub(crate) use source::inspect_sources;
#[path = "series_lite_desktop_supply_design.rs"]
mod design;
pub(crate) use design::inspect_designs;
#[cfg(test)]
#[path = "series_lite_desktop_supply_tests.rs"]
mod tests;
