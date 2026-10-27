//! Desktop source-lock to lite standard-source adapter. Reads only; no task
//! dispatch, reclassification of the saved source, or semantic approval.
use crate::pipeline::{digest, slice_utf16, validate_source, LockedSource, SourceRange};
use serde_json::{json, Value};

pub(crate) const CONTRACT: &str = "desktop-lite-episode-source/v1";
type Result<T> = std::result::Result<T, String>;

pub(crate) fn prepare(
    locked: &LockedSource,
    episode_id: &str,
    known_speakers: &[String],
) -> Result<Value> {
    validate_source(locked)?;
    let episode = locked
        .episodes
        .iter()
        .find(|e| e.id == episode_id)
        .ok_or("LITE_DESKTOP_SOURCE_EPISODE_MISSING")?;
    let rows: Vec<_> = locked
        .rows
        .iter()
        .filter(|r| r.episode_id.as_deref() == Some(episode_id))
        .collect();
    if rows.is_empty()
        || rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
            != episode
                .row_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
    {
        return Err("LITE_DESKTOP_SOURCE_MEMBERSHIP_CHANGED".into());
    }
    if rows.windows(2).any(|r| r[0].range.end != r[1].range.start) {
        return Err("LITE_DESKTOP_SOURCE_NONCONTIGUOUS".into());
    }
    let range = SourceRange {
        start: rows[0].range.start,
        end: rows.last().unwrap().range.end,
    };
    let text = slice_utf16(&locked.text, &range)?;
    if rows.iter().map(|r| r.text.as_str()).collect::<String>() != text {
        return Err("LITE_DESKTOP_SOURCE_EXACT_TEXT_MISMATCH".into());
    }
    // The permanent Desktop episode ID supplies the key. An MD filename or a
    // recalculated asset hash cannot mint a second identity for this episode.
    let source = crate::series_lite_intake::prepare_source(
        &json!(text),
        &json!(episode_id),
        &json!(known_speakers),
    )
    .map_err(|e| format!("LITE_DESKTOP_INTAKE:{}", serde_json::to_string(&e).unwrap()))?;
    let locked_bytes = serde_json::to_string(locked).map_err(|e| e.to_string())?;
    let binding = json!({
        "version":CONTRACT,
        "sourceSha256":locked.sha256,
        "lockedSourceSha256":digest(&locked_bytes),
        "parserVersion":locked.parser_version,
        "episodeId":episode.id,
        "episodeNumber":episode.number,
        "sourceNumber":episode.source_number,
        "sourceRange":range,
        "episodeTextSha256":digest(&text),
        "coordinateSystem":"utf16-code-units",
        "rows":rows.iter().map(|r|json!({"desktopRowId":r.id,"sceneId":r.scene_id,
            "globalRange":r.range,"localRange":{"start":r.range.start-range.start,"end":r.range.end-range.start}})).collect::<Vec<_>>(),
        "standardSourceSha256":digest(&serde_json::to_string(&source).map_err(|e| e.to_string())?),
        "semanticReview":"NOT_RUN"
    });
    // Global background remains complete and distinct. It is supporting input,
    // never injected into the selected episode as new events or spoken lines.
    let background: Vec<_> = locked
        .rows
        .iter()
        .filter(|r| r.episode_id.is_none())
        .cloned()
        .collect();
    Ok(
        json!({"version":CONTRACT,"source":source,"desktopBinding":binding,"backgroundSourceRows":background}),
    )
}

/// Rebuild against the same frozen parser contract. A later recognizer, changed
/// human boundary decision, speaker list or source text must not re-sign this.
pub(crate) fn verify_saved(
    locked: &LockedSource,
    saved: &Value,
    known_speakers: &[String],
) -> Result<()> {
    if saved["version"] != CONTRACT {
        return Err("LITE_DESKTOP_SOURCE_CONTRACT_UNKNOWN".into());
    }
    let id = saved["desktopBinding"]["episodeId"]
        .as_str()
        .ok_or("LITE_DESKTOP_SOURCE_EPISODE_MISSING")?;
    if &prepare(locked, id, known_speakers)? != saved {
        return Err("LITE_DESKTOP_SOURCE_FROZEN_BINDING_CHANGED".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> LockedSource {
        crate::pipeline::prepare_source("【故事简介】\r\n甲要找回钥匙。\r\n第1集\r\n1-1 门口 日 外\r\n△甲抬头。\r\n甲：你好😀。\r\n第2集\r\n2-1 室内 夜 内\r\n乙：再见。\r\n").unwrap()
    }
    #[test]
    fn exact_episode_text_global_offsets_and_background_survive_reopen() {
        let locked = fixture();
        let before = serde_json::to_vec(&locked).unwrap();
        let names = vec!["甲".into(), "乙".into()];
        for episode in &locked.episodes {
            let prepared = prepare(&locked, &episode.id, &names).unwrap();
            let saved: Value =
                serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            verify_saved(&locked, &saved, &names).unwrap();
            let range: SourceRange =
                serde_json::from_value(saved["desktopBinding"]["sourceRange"].clone()).unwrap();
            assert_eq!(
                saved["source"]["intakeEvidence"]["sourceText"],
                slice_utf16(&locked.text, &range).unwrap()
            );
            assert!(!saved["backgroundSourceRows"].as_array().unwrap().is_empty());
            assert_eq!(saved["desktopBinding"]["semanticReview"], "NOT_RUN");
            assert_eq!(saved["source"]["key"], episode.id);
        }
        assert_eq!(serde_json::to_vec(&locked).unwrap(), before);
    }
    #[test]
    fn source_identity_coordinates_and_speaker_drift_cannot_reuse_saved_proof() {
        let locked = fixture();
        let names = vec!["甲".into(), "乙".into()];
        let saved = prepare(&locked, &locked.episodes[0].id, &names).unwrap();
        assert!(verify_saved(&locked, &saved, &["另一人".into()]).is_err());
        for field in [
            "episodeId",
            "sourceSha256",
            "lockedSourceSha256",
            "sourceRange",
        ] {
            let mut changed = saved.clone();
            changed["desktopBinding"][field] = json!("altered");
            assert!(verify_saved(&locked, &changed, &names).is_err());
        }
        let mut changed = locked.clone();
        changed.rows[0].text.push('改');
        assert!(verify_saved(&changed, &saved, &names).is_err());
        let mut changed = saved.clone();
        changed["backgroundSourceRows"] = json!([]);
        assert!(verify_saved(&locked, &changed, &names).is_err());
    }
}
