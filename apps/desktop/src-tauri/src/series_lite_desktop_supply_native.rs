//! Native read-only bindings. Register alongside series_lite_desktop_supply.
//! This uses original validators; never re-signs old work with lite rules.
use crate::series_lite_desktop_supply::{
    self as supply, FrozenSeriesVerifier, Gap, Inspection, SourceVerifier,
};
use rusqlite::Connection;
use serde_json::{json, Value};
type Result<T> = std::result::Result<T, String>;
struct OriginalSource;
impl SourceVerifier for OriginalSource {
    fn verify(&self, locked: &Value, prepared: &Value, known_speakers: &[String]) -> Result<()> {
        let locked: crate::pipeline::LockedSource =
            serde_json::from_value(locked.clone()).map_err(|e| e.to_string())?;
        crate::series_lite_desktop_source::verify_saved(&locked, prepared, known_speakers)
    }
    fn original_episode_number(&self, label: &str) -> Option<u32> {
        // Empty/non-numeric original labels must not acquire a fabricated zero.
        if label.is_empty() {
            None
        } else {
            crate::pipeline::number_value(label)
        }
    }
}
struct OriginalAdoption;
impl FrozenSeriesVerifier for OriginalAdoption {
    fn verify(
        &self,
        c: &Connection,
        input: &Value,
        delivery: &Value,
        workflow_id: &str,
    ) -> Result<()> {
        crate::pipeline::series::validate_delivery(input, delivery)?;
        crate::workflow::verify_series_receipts(
            c,
            workflow_id,
            &json!({"seriesDeliveries":[delivery]}),
        )
    }
}
/// No scene assignment/renumbering fallback; individual source gaps remain local.
pub(crate) fn inspect_sources(
    locked: &crate::pipeline::LockedSource,
    episode_ids: &[String],
    known_speakers: &[String],
) -> Result<Inspection> {
    let mut prepared = Vec::new();
    let mut gaps = Vec::new();
    for id in episode_ids {
        match crate::series_lite_desktop_source::prepare(locked, id, known_speakers) {
            Ok(p) => prepared.push(p),
            Err(detail) => gaps.push(Gap {
                code: "LITE_SUPPLY_SOURCE_PREPARATION".into(),
                episode_id: Some(id.clone()),
                asset_id: None,
                coordinates: vec![],
                detail,
            }),
        }
    }
    let frozen = serde_json::to_value(locked).map_err(|e| e.to_string())?;
    let mut result = supply::inspect_sources(&OriginalSource, &frozen, &prepared, known_speakers);
    result.gaps.extend(gaps);
    Ok(result)
}
/// Uses one consistent read transaction; does not install/migrate any storage.
pub(crate) fn inspect_designs(
    c: &mut Connection,
    source_hash: &str,
    asset_ids: &[String],
) -> Result<Inspection> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|e| e.to_string())?;
    let result = supply::inspect_designs(&tx, source_hash, asset_ids, &OriginalAdoption);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

// These exercise the real Desktop parser/adapter and original numeral decoder.
// They are compiled only by the parent application's test build, not by the
// isolated loader harness with explicit verifier doubles.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_desktop_episode_label_survives_real_source_gate() {
        let locked =
            crate::pipeline::prepare_source("第28集\n28-1 门口 日 外\n△甲抬头。\n甲：你好😀。\n")
                .unwrap();
        assert_eq!(locked.episodes.len(), 1);
        assert_eq!(locked.episodes[0].number, 1);
        let result =
            inspect_sources(&locked, &[locked.episodes[0].id.clone()], &["甲".into()]).unwrap();
        assert_eq!(result.ready.len(), 1, "{result:?}");
        assert_eq!(result.ready[0]["episodeNumber"], 28);
        assert_eq!(result.ready[0]["originalEpisodeLabel"], "28");
        assert_eq!(result.ready[0]["desktopOrdinal"], 1);
    }
    #[test]
    fn original_decoder_requires_nonempty_original_label() {
        assert_eq!(OriginalSource.original_episode_number(""), None);
        assert_eq!(OriginalSource.original_episode_number("序章"), None);
        assert_eq!(OriginalSource.original_episode_number("二十八"), Some(28));
    }
}
