//! Live projection and media safety for unresolved foundation conflicts.
//! Nothing here rewrites frozen assets, requests, prompts or adopted images.
use crate::store::{s, Result, Store};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) const CODE: &str = "FOUNDATION_CONFLICT_REVIEW_REQUIRED";

fn pending<'a>(source: &Value, foundation: &'a Value) -> Vec<&'a Value> {
    if s(source, "sha256").is_empty() || foundation["sourceSha256"] != source["sha256"] {
        return vec![];
    }
    foundation["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|issue| {
            issue["details"]["kind"] == "FOUNDATION_CONFLICT_REVIEW"
                && issue["details"]["status"] == "PENDING"
        })
        .collect()
}

fn relevant(issue: &Value, asset: &Value) -> bool {
    let owner = issue["assetId"].as_str().filter(|s| !s.is_empty());
    let path = issue["details"]["path"].as_str().unwrap_or("");
    let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
    let identity_path = parts.first() == Some(&"assets");
    let left = owner.or_else(|| {
        if identity_path {
            parts.get(1).copied()
        } else {
            None
        }
    });
    let right = if identity_path && parts.get(2) == Some(&"identity-collision-with") {
        parts.get(3).copied()
    } else {
        None
    };
    let same = |id: &str| {
        ["id", "identityId", "baseAssetId"]
            .iter()
            .any(|key| asset[*key].as_str() == Some(id))
    };
    if !left.into_iter().chain(right).any(same) {
        return false;
    }
    // An identity collision protects both identities. A specific variant design
    // conflict protects that variant only; unrelated costumes remain available.
    if right.is_some() {
        return true;
    }
    let variant = issue["variantId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if identity_path && parts.get(2) == Some(&"variants") {
                parts.get(3).copied()
            } else {
                None
            }
        });
    variant.is_none_or(|id| asset["variantId"].as_str() == Some(id))
}

fn affected<'a>(source: &Value, foundation: &'a Value, asset: &Value) -> Vec<&'a Value> {
    pending(source, foundation)
        .into_iter()
        .filter(|i| relevant(i, asset))
        .collect()
}

pub(crate) fn require_clear(source: &Value, foundation: &Value, asset: &Value) -> Result<()> {
    let issues = affected(source, foundation, asset);
    if issues.is_empty() {
        return Ok(());
    }
    Err(format!("{CODE}: 资产「{}」有 {} 项身份或设计待核对，暂不制作或用作参考；已有提示词、图片和原记录保留，无关资产可继续。",s(asset,"name"),issues.len()))
}

/// Workspace-only overlays. Stored versions and existing frozen request hashes
/// do not change simply because a pending conflict appears or is resolved.
pub(crate) fn project(source: &Value, foundation: &Value, assets: &mut [Value]) -> Value {
    let all = pending(source, foundation);
    let mut blocked = BTreeSet::new();
    for asset in assets {
        let issues:Vec<_>=all.iter().filter(|issue|relevant(issue,asset)).map(|issue|json!({"coordinate":issue["coordinate"],"problem":issue["problem"],"assetId":issue["assetId"],"variantId":issue["variantId"],"path":issue["details"]["path"],"conflictId":issue["details"]["conflictId"]})).collect();
        if !issues.is_empty() {
            blocked.insert(s(asset, "id").to_owned());
        }
        if let Some(object) = asset.as_object_mut() {
            // Clear only this module's transient overlay, never other review flags.
            if issues.is_empty() {
                object.remove("foundationConflictReview");
            } else {
                object.insert(
                    "foundationConflictReview".into(),
                    json!({"pending":true,"pendingCount":issues.len(),"issues":issues,"code":CODE}),
                );
            }
        }
    }
    json!({"version":"foundation-conflict-media-gate/v1","pending":!all.is_empty(),"pendingCount":all.len(),"blockedAssetIds":blocked,"conflictReviewComplete":all.is_empty()})
}

/// Re-read current authoritative pending state immediately before dispatch,
/// including actual media donors and semantic reference bindings. Downloads and
/// reconciliation of already submitted requests do not call this submit gate.
pub(crate) fn validate_snapshot(store: &Store, snapshot: &Value) -> Result<()> {
    // Only the issue projection is needed. Avoid decoding a whole novel and
    // hundreds of asset definitions on every prepare/submit gate. One SELECT
    // observes both latest versions within the same SQLite read snapshot.
    let (source_hash,serialized):(Option<String>,Option<String>)=store.conn()?.query_row(
        "SELECT (SELECT sha256 FROM sources ORDER BY version DESC LIMIT 1),(SELECT json_object('sourceSha256',json_extract(data,'$.sourceSha256'),'issues',json_extract(data,'$.issues')) FROM foundations ORDER BY version DESC LIMIT 1)",[],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|error|error.to_string())?;
    let source = json!({"sha256":source_hash});
    let foundation = serialized
        .map(|text| serde_json::from_str::<Value>(&text).map_err(|error| error.to_string()))
        .transpose()?
        .unwrap_or(Value::Null);
    if pending(&source, &foundation).is_empty() {
        return Ok(());
    }
    let mut ids = BTreeSet::new();
    if snapshot["kind"] == "image" && !s(snapshot, "assetId").is_empty() {
        ids.insert(s(snapshot, "assetId").to_owned());
    }
    for field in ["references", "inlineReferences"] {
        for reference in snapshot[field].as_array().into_iter().flatten() {
            if !s(reference, "assetId").is_empty() {
                ids.insert(s(reference, "assetId").to_owned());
            }
            for semantic in reference["semanticBindings"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !s(semantic, "assetId").is_empty() {
                    ids.insert(s(semantic, "assetId").to_owned());
                }
            }
        }
    }
    for id in ids {
        require_clear(&source, &foundation, &store.active_asset(&id)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn issue(path: &str, owner: &str) -> Value {
        json!({"assetId":owner,"variantId":null,"coordinate":"conflict:1","problem":"原成果保留待核对","blocking":false,"details":{"kind":"FOUNDATION_CONFLICT_REVIEW","status":"PENDING","path":path,"conflictId":"c1"}})
    }
    fn fixture() -> (Value, Value, Vec<Value>) {
        (
            json!({"sha256":"source"}),
            json!({"sourceSha256":"source","issues":[issue("/assets/local/identity-collision-with/canonical","local")]}),
            vec![
                json!({"id":"local-row","baseAssetId":"local","variantId":"local:base","name":"重盾","version":3,"prompt":"已有提示词"}),
                json!({"id":"canonical-row","identityId":"canonical","variantId":"canonical:base","name":"重盾","version":4}),
                json!({"id":"other-row","baseAssetId":"other","variantId":"other:base","name":"重盾","version":5}),
            ],
        )
    }
    #[test]
    fn pending_identity_protects_both_exact_identities_not_shared_name() {
        let (source, foundation, mut assets) = fixture();
        let saved = assets.clone();
        let summary = project(&source, &foundation, &mut assets);
        assert_eq!(summary["pendingCount"], 1);
        assert_eq!(summary["conflictReviewComplete"], false);
        for i in 0..2 {
            assert_eq!(assets[i]["foundationConflictReview"]["pending"], true);
            assert!(require_clear(&source, &foundation, &assets[i])
                .unwrap_err()
                .contains(CODE));
            assert_eq!(assets[i]["version"], saved[i]["version"]);
            assert_eq!(assets[i]["prompt"], saved[i]["prompt"]);
        }
        assert!(require_clear(&source, &foundation, &assets[2]).is_ok());
        assert_eq!(assets[2], saved[2]);
    }
    #[test]
    fn variant_pending_is_local_and_resolved_projection_does_not_clear_other_reviews() {
        let (source, mut foundation, mut assets) = fixture();
        foundation["issues"] = json!([issue("/assets/local/variants/look-2/note", "local")]);
        assets.push(json!({"id":"second-look","baseAssetId":"local","variantId":"look-2","needsReview":true,"name":"换装"}));
        let summary = project(&source, &foundation, &mut assets);
        assert_eq!(summary["blockedAssetIds"], json!(["second-look"]));
        assert!(require_clear(&source, &foundation, &assets[0]).is_ok());
        foundation["issues"][0]["details"]["status"] = json!("RESOLVED");
        let summary = project(&source, &foundation, &mut assets);
        assert_eq!(summary["pendingCount"], 0);
        assert!(assets[3].get("foundationConflictReview").is_none());
        assert_eq!(assets[3]["needsReview"], true);
    }
    #[test]
    fn different_source_does_not_attach_pending_to_unrelated_import() {
        let (mut source, foundation, mut assets) = fixture();
        source["sha256"] = json!("different");
        assert_eq!(
            project(&source, &foundation, &mut assets)["pendingCount"],
            0
        );
        assert!(require_clear(&source, &foundation, &assets[0]).is_ok());
    }
    #[test]
    fn frozen_image_and_actual_reference_are_rechecked_without_writing_jobs() {
        let temporary = tempfile::TempDir::new().unwrap();
        let store = Store::open(temporary.path().join("project")).unwrap();
        store.create("isolated conflict gate", "p").unwrap();
        let source = store
            .save_source(json!(crate::pipeline::prepare_source(
                "第1集\n场景：大厅\n△人物持盾。\n"
            )
            .unwrap()))
            .unwrap();
        for asset in fixture().2 {
            store.save_asset(asset).unwrap();
        }
        let snapshot = json!({"kind":"image","assetId":"other-row","references":[]});
        let before = snapshot.clone();
        store.save_foundation(json!({"sourceSha256":source["sha256"],"issues":[issue("/assets/local/identity-collision-with/canonical","local")]})).unwrap();
        validate_snapshot(&store, &snapshot).unwrap();
        let mut direct = snapshot.clone();
        direct["assetId"] = json!("local-row");
        assert!(validate_snapshot(&store, &direct)
            .unwrap_err()
            .contains(CODE));
        let mut donor = snapshot.clone();
        donor["references"] = json!([{"assetId":"canonical-row"}]);
        assert!(validate_snapshot(&store, &donor).is_err());
        let mut semantic = snapshot.clone();
        semantic["references"] =
            json!([{"assetId":"other-row","semanticBindings":[{"assetId":"local-row"}]}]);
        assert!(validate_snapshot(&store, &semantic).is_err());
        assert_eq!(snapshot, before);
        assert!(store.job_summaries().unwrap().is_empty());
    }
}
