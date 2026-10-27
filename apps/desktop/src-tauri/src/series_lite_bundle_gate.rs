//! Native frozen lite 0.1.2 author/source validation components.
//! Separate from the already delivered rendering module while this port is integrated.
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
type Result<T> = std::result::Result<T, String>;
fn need(ok: bool, code: &str, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("{code}: {message}"))
    }
}
fn same(a: &Value, b: &Value) -> bool {
    crate::series_lite_registry::canonical(a).ok() == crate::series_lite_registry::canonical(b).ok()
}
#[path = "series_lite_bundle_draft.rs"]
mod draft;
pub use draft::{
    checked_events, fantasy_timing, resolve_source_ref, valid_visual_subjects, validate_draft,
};
#[path = "series_lite_bundle_source.rs"]
mod source;
pub use source::normalize_legacy_source;
#[path = "series_lite_bundle_gate_check.rs"]
mod check;
pub use check::NativeLegacyBundleGate;
#[cfg(test)]
#[path = "series_lite_bundle_gate_tests.rs"]
mod tests;
