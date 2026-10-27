//! Desktop-only mechanical projection after lite creative composition/patching.
//! The controller must supply an independently verified, frozen scene supply pin.
//! This module neither grants design adoption nor proves semantic source coverage.
use crate::series_lite_edit::{canonical, parse, sha256, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const PROFILE: &str = "desktop-lite-scene-bindings-projection/v1";
pub(crate) const SUPPLY: &str = "desktop-lite-scene-supply/v1";
type Result<T> = std::result::Result<T, String>;
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("LITE_SCENE_PROJECTION:{code}"))
    }
}
fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A dependency pin, never a model's claim of approval. The supply loader must
/// verify original adoption receipts and current design dependencies BEFORE it
/// freezes this manifest in the controller ledger. Material-only parents are
/// intentionally absent from scenes' allowed_assets lists.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SceneSupply {
    pub version: String,
    pub source_hash: String,
    pub source_lock_hash: String,
    pub episode: i64,
    pub source_value_sha256: String,
    pub registry_snapshot_sha256: String,
    pub evidence_sha256: String,
    pub scenes: Vec<SuppliedScene>,
    pub assets: Vec<SuppliedAsset>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SuppliedScene {
    pub id: String,
    /// Hash of all original scene fields except bindings, including sourceRefs.
    pub protected_fields_sha256: String,
    pub allowed_assets: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SuppliedAsset {
    pub key: String,
    pub asset_id: String,
    pub asset_revision: u64,
    pub full_design_sha256: String,
    /// Reading a parent design does not authorize its on-screen appearance.
    pub material_source_only: bool,
    pub image_allowed: bool,
}
#[derive(Debug)]
pub(crate) struct Projection {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub changed: bool,
    pub receipt: Value,
}
fn strings(v: &Json) -> Result<Vec<String>> {
    v.array()?
        .iter()
        .map(|v| v.string().map(str::to_owned))
        .collect()
}
fn protected_scene(scene: &Json) -> Result<String> {
    let mut fields = scene.object()?.clone();
    fields.remove("bindings");
    Ok(sha256(&canonical(&Json::Object(fields))))
}

/// No file/database writes and no model calls. expected_* come from the
/// controller's immutable ledger, not from fields supplied by the model.
/// Normal bundle/source/registry gates must still run on the returned candidate.
pub(crate) fn project(
    candidate_raw: &[u8],
    expected_candidate_sha256: &str,
    supply_raw: &[u8],
    expected_supply_sha256: &str,
) -> Result<Projection> {
    need(
        hex(expected_candidate_sha256) && sha256(candidate_raw) == expected_candidate_sha256,
        "CANDIDATE_DRIFT",
    )?;
    need(
        hex(expected_supply_sha256) && sha256(supply_raw) == expected_supply_sha256,
        "SUPPLY_DRIFT",
    )?;
    // Parse first with the exact parser so duplicate keys cannot silently replace
    // a pinned permission before serde creates the typed control manifest.
    parse(supply_raw)?;
    let supply: SceneSupply =
        serde_json::from_slice(supply_raw).map_err(|_| "LITE_SCENE_PROJECTION:SUPPLY_SHAPE")?;
    need(
        supply.version == SUPPLY
            && [
                &supply.source_hash,
                &supply.source_lock_hash,
                &supply.source_value_sha256,
                &supply.registry_snapshot_sha256,
                &supply.evidence_sha256,
            ]
            .iter()
            .all(|s| hex(s)),
        "SUPPLY_IDENTITY",
    )?;
    let mut candidate = parse(candidate_raw)?;
    need(
        candidate.get("version")?.string()? == "series-storyboard-bundle-v1",
        "BUNDLE_VERSION",
    )?;
    need(
        candidate.get("sourceHash")?.string()? == supply.source_hash
            && candidate.get("sourceLockHash")?.string()? == supply.source_lock_hash,
        "SOURCE_LOCK_DRIFT",
    )?;
    let episodes = candidate.get("episodes")?.array()?;
    need(episodes.len() == 1, "SINGLE_EPISODE_REQUIRED")?;
    let episode = &episodes[0];
    need(
        episode.get("number")?.integer() == Some(supply.episode as i128),
        "EPISODE_IDENTITY",
    )?;
    need(
        sha256(&canonical(episode.get("source")?)) == supply.source_value_sha256,
        "SOURCE_VALUE_DRIFT",
    )?;

    let mut supplied = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for asset in &supply.assets {
        need(
            !asset.key.is_empty()
                && !asset.asset_id.is_empty()
                && asset.asset_revision > 0
                && hex(&asset.full_design_sha256),
            "DESIGN_PIN_MISSING",
        )?;
        need(
            supplied.insert(asset.key.as_str(), asset).is_none()
                && ids.insert(asset.asset_id.as_str()),
            "DUPLICATE_DESIGN_IDENTITY",
        )?;
    }
    let mut bundle_assets = BTreeMap::new();
    for asset in candidate.get("assets")?.array()? {
        let key = asset.get("key")?.string()?;
        need(
            bundle_assets.insert(key, asset).is_none(),
            "DUPLICATE_BUNDLE_ASSET",
        )?;
    }
    need(
        bundle_assets.keys().eq(supplied.keys()),
        "BUNDLE_SUPPLY_SCOPE_MISMATCH",
    )?;
    for (key, pin) in &supplied {
        let asset = bundle_assets
            .get(key)
            .ok_or("LITE_SCENE_PROJECTION:DESIGN_NOT_IN_BUNDLE")?;
        need(
            asset.get("assetId")?.string()? == pin.asset_id
                && asset.get("assetRevision")?.integer() == Some(pin.asset_revision as i128),
            "DESIGN_VERSION_DRIFT",
        )?;
    }
    let scenes = episode.get("scenes")?.array()?;
    need(
        !scenes.is_empty() && scenes.len() == supply.scenes.len(),
        "SCENE_SCOPE_CHANGED",
    )?;
    let mut allowed = BTreeMap::new();
    let mut unions = BTreeMap::<String, Vec<String>>::new();
    for (scene, pin) in scenes.iter().zip(&supply.scenes) {
        need(
            scene.get("id")?.string()? == pin.id
                && protected_scene(scene)? == pin.protected_fields_sha256,
            "PROTECTED_SCENE_CHANGED",
        )?;
        let keys: BTreeSet<_> = pin.allowed_assets.iter().map(String::as_str).collect();
        need(
            keys.len() == pin.allowed_assets.len()
                && keys
                    .iter()
                    .all(|key| supplied.get(key).is_some_and(|a| !a.material_source_only)),
            "SCENE_PERMISSION_INVALID",
        )?;
        need(
            allowed.insert(pin.id.as_str(), keys).is_none(),
            "DUPLICATE_SCENE",
        )?;
        unions.insert(pin.id.clone(), vec![]);
    }
    let mut unit_ids = BTreeSet::new();
    let mut used_scenes = BTreeSet::new();
    for unit in episode.get("draft")?.get("units")?.array()? {
        need(unit_ids.insert(unit.get("id")?.string()?), "DUPLICATE_UNIT")?;
        let scene_id = unit.get("sceneId")?.string()?;
        let permissions = allowed
            .get(scene_id)
            .ok_or("LITE_SCENE_PROJECTION:UNKNOWN_SCENE")?;
        used_scenes.insert(scene_id);
        let bindings = strings(unit.get("bindings")?)?;
        let unique: BTreeSet<_> = bindings.iter().map(String::as_str).collect();
        need(
            !bindings.is_empty() && unique.len() == bindings.len(),
            "UNIT_BINDINGS_INVALID",
        )?;
        for key in &bindings {
            need(
                permissions.contains(key.as_str()),
                "ASSET_REQUIRES_SCENE_SUPPLY",
            )?;
            let union = unions.get_mut(scene_id).unwrap();
            if !union.contains(key) {
                union.push(key.clone());
            }
        }
        for key in strings(unit.get("imageBindings")?)? {
            need(
                unique.contains(key.as_str())
                    && supplied
                        .get(key.as_str())
                        .is_some_and(|a| a.image_allowed && !a.material_source_only),
                "IMAGE_USE_NOT_AUTHORIZED",
            )?;
        }
    }
    need(
        used_scenes.len() == scenes.len(),
        "EMPTY_SCENE_REQUIRES_REVIEW",
    )?;
    let mut changes = vec![];
    let mut projected_scenes = scenes.to_vec();
    for scene in &mut projected_scenes {
        let id = scene.get("id")?.string()?.to_owned();
        let previous = strings(scene.get("bindings")?)?;
        let next = &unions[&id];
        if &previous != next {
            changes.push(json!({"sceneId":id,"before":previous,"after":next}));
            let Json::Object(fields) = scene else {
                unreachable!()
            };
            fields.insert(
                "bindings".into(),
                Json::Array(next.iter().cloned().map(Json::String).collect()),
            );
        }
    }
    let changed = !changes.is_empty();
    if changed {
        let Json::Object(root) = &mut candidate else {
            unreachable!()
        };
        let Json::Array(episodes) = root.get_mut("episodes").unwrap() else {
            unreachable!()
        };
        let Json::Object(episode) = &mut episodes[0] else {
            unreachable!()
        };
        episode.insert("scenes".into(), Json::Array(projected_scenes));
    }
    let bytes = if changed {
        canonical(&candidate)
    } else {
        candidate_raw.to_vec()
    };
    let hash = sha256(&bytes);
    let receipt = json!({"version":PROFILE,"inputSha256":expected_candidate_sha256,"outputSha256":hash,"supplySha256":expected_supply_sha256,
        "registrySnapshotSha256":supply.registry_snapshot_sha256,"supplyEvidenceSha256":supply.evidence_sha256,"changed":changed,"changes":changes,
        "mechanicalBundleValidation":"NOT_RUN","contentReview":"NOT_RUN","adoption":"NOT_GRANTED"});
    Ok(Projection {
        bytes,
        sha256: hash,
        changed,
        receipt,
    })
}

#[cfg(test)]
#[path = "series_lite_scene_projection_tests.rs"]
mod tests;
