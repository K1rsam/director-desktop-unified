//! Explicit asset uses, not names or keyword guesses, select cross-episode source.
use super::*;

pub(super) const FIELD: &str = "seriesDesignSourceUses";
pub(super) const VERSION: &str = "series-design-source-uses/v1";

pub(super) fn rows(
    input: &Value,
    source: &LockedSource,
    episode: &SourceEpisode,
    targets: &[Value],
) -> Result<Vec<SourceRow>, String> {
    // Validate the scope including order, but use the locked production scope,
    // not the current episode selection, for explicitly declared asset uses.
    scope(input, source)?;
    let allowed = string_list(&input["productionScope"]["episodeIds"])?;
    let mut scenes = HashSet::new();
    let mut direct = HashSet::new();
    for asset in targets {
        for values in [
            asset.pointer("/scope/scenes"),
            asset.pointer("/applicability/scenes"),
        ] {
            for reference in values.and_then(Value::as_array).into_iter().flatten() {
                let (scene, declared_episode) = if let Some(id) = reference.as_str() {
                    (id, None)
                } else if let Some(pair) = reference.as_array() {
                    (
                        pair.get(1).and_then(Value::as_str).unwrap_or(""),
                        pair.first().and_then(Value::as_str),
                    )
                } else {
                    (s(reference, "sceneId"), reference["episodeId"].as_str())
                };
                let matched = source
                    .scenes
                    .iter()
                    .find(|candidate| {
                        candidate.id == scene
                            && declared_episode.is_none_or(|id| id == candidate.episode_id)
                    })
                    .ok_or("SERIES_DESIGN_USE_SCENE_UNKNOWN")?;
                if allowed.contains(&matched.episode_id) {
                    scenes.insert(matched.id.clone());
                }
            }
        }
        for id in asset["sourceRefs"].as_array().into_iter().flatten() {
            let row = source
                .rows
                .iter()
                .find(|r| id == &r.id)
                .ok_or("SERIES_DESIGN_USE_SOURCE_UNKNOWN")?;
            if !row
                .episode_id
                .as_ref()
                .is_some_and(|id| allowed.contains(id))
            {
                continue;
            }
            if row.kind == "body" {
                direct.insert(row.id.clone());
            } else if let Some(scene) = &row.scene_id {
                // A scene heading is a pointer to that numbered scene, not a
                // body row. Expanding it supplies the previously omitted use.
                scenes.insert(scene.clone());
            }
        }
    }
    Ok(source
        .rows
        .iter()
        .filter(|r| {
            r.kind == "body"
                && (r.episode_id.as_deref() == Some(&episode.id)
                    || (r.episode_id.as_ref().is_some_and(|id| allowed.contains(id))
                        && (direct.contains(&r.id)
                            || r.scene_id.as_ref().is_some_and(|id| scenes.contains(id)))))
        })
        .cloned()
        .collect())
}
