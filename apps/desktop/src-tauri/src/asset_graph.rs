//! New workflow dependencies are explicit media requirements, never name guesses.
use crate::store::{s, Result, Store};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const PROFILE: &str = "desktop-series672-assets112/v1";
pub fn modern(asset: &Value) -> bool {
    asset["workflowProfile"] == PROFILE || asset["imagePrompt"].is_object()
}
/// State gate for a current adoption, shared by new image references, formal
/// exports and new video preparations. Callers still verify the adoption row
/// and media bytes. Missing legacy state remains compatible; historical
/// candidate/provenance records on an explicitly adopted media are not a veto.
pub(crate) fn image_adoption_blocked(asset: &Value) -> bool {
    matches!(
        s(asset, "imageStatus"),
        "needs_review" | "stale" | "rejected" | "invalid" | "needs_repair"
            | "unknown" | "submission_unknown" | "candidate" | "pending_review"
    )
}
pub(crate) fn design_review_blocked(asset: &Value) -> bool {
    matches!(s(asset, "designStatus"), "needs_review" | "stale" | "rejected" | "invalid")
}
pub fn image_required(asset: &Value) -> bool {
    !crate::store::asset_deleted(asset)
        && asset["imageRequired"] != false
        && !["audio", "voice", "SE"].contains(&s(asset, "kind"))
        && !["voice-only", "mention-only", "reference-only"].contains(&s(asset, "purpose"))
        && !matches!(
            asset.pointer("/preparation/mode").and_then(Value::as_str),
            Some("AUDIO" | "INLINE")
        )
}
pub fn reference_role(reference: &Value) -> &str {
    reference["role"]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| reference["kind"].as_str().filter(|v| !v.trim().is_empty()))
        .unwrap_or("reference")
}
fn identity(asset: &Value) -> Option<&str> {
    asset["identityId"]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            asset["baseAssetId"]
                .as_str()
                .filter(|v| !v.trim().is_empty())
        })
}
pub fn dependencies(assets: &[Value], asset: &Value) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    if asset
        .get("referenceRequirements")
        .is_some_and(|v| !v.is_null() && !v.is_array())
    {
        return Err(format!("{} 的参考需求不是有效列表", s(asset, "id")));
    }
    let requirements = asset.get("referenceRequirements").and_then(Value::as_array);
    for reference in requirements.into_iter().flatten() {
        if reference["required"] == false {
            continue;
        }
        let key = s(reference, "targetAssetId");
        if key.is_empty() {
            return Err("参考需求缺少稳定目标资产 ID".into());
        }
        if key == s(asset, "id") {
            return Err(format!("图片依赖自身：{key} → {key}"));
        }
        let parent = assets
            .iter()
            .find(|a| s(a, "id") == key && !crate::store::asset_deleted(a))
            .ok_or_else(|| format!("图片依赖不存在：{} → {key}", s(asset, "id")))?;
        if let Some(variant) = reference.get("targetVariantId").filter(|v| !v.is_null()) {
            if variant != &parent["variantId"] {
                return Err(format!("母版形象版本不匹配：{} → {key}", s(asset, "id")));
            }
        }
        if ["face", "identity", "identity-faces"].contains(&reference_role(reference))
            && identity(asset)
                .zip(identity(parent))
                .is_some_and(|(a, b)| a != b)
        {
            return Err(format!(
                "身份母版不属于同一角色：{} → {key}",
                s(asset, "id")
            ));
        }
        if !ids.contains(&key.to_owned()) {
            ids.push(key.to_owned());
        }
    }
    Ok(ids)
}

/// Resolve only the directly required, actually adopted media. Candidate files
/// never unlock children, and this function does not expand the image scope.
pub fn media_references(
    store: &Store,
    assets: &[Value],
    asset: &Value,
) -> Result<Option<Vec<Value>>> {
    let ids = dependencies(assets, asset)?;
    let mut refs = Vec::new();
    let mut missing = false;
    for id in ids {
        let parent = assets
            .iter()
            .find(|a| s(a, "id") == id)
            .ok_or("图片依赖不存在")?;
        let current = store.active_asset(&id)?;
        if current["variantId"] != parent["variantId"] || current["version"] != parent["version"] {
            return Err(format!("母版身份或资产版本在绑定期间已变化：{id}"));
        }
        let parent = &current;
        if image_adoption_blocked(parent) || design_review_blocked(parent) {
            missing = true;
            continue;
        }
        let Some(mid) = store.adopted(&id, s(parent, "variantId"))? else {
            missing = true;
            continue;
        };
        let media = store.get("media", &mid)?;
        if media["kind"] != "image" || media["placeholder"] == true {
            return Err(format!("{} 的当前母图不是有效正式图片", id));
        }
        let path = store.verify_media(&media)?;
        let requirements: Vec<_> = asset["referenceRequirements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["required"] != false && r["targetAssetId"] == id)
            .cloned()
            .collect();
        refs.push(json!({"mediaId":mid,"kind":"image","path":path,"relativePath":media["relativePath"],
            "sha256":media["sha256"],"contentHash":media["sha256"],
            "mediaRevision":media.get("version").filter(|v|v.as_u64().is_some_and(|n|n>0)).cloned().unwrap_or(json!(1)),
            "order":refs.len(),"usage":"source","assetId":parent["id"],"assetVersion":parent["version"],"variantId":parent["variantId"],
            "semanticBindings":[{"assetId":parent["id"],"variantId":parent["variantId"],"assetVersion":parent["version"]}],
            "requirements":requirements,"adoptionBasis":"current-adoption-record"}));
    }
    Ok((!missing).then_some(refs))
}

/// Validate all reachable edges, but never turn an unselected ancestor into work.
pub fn explicit_order(assets: &[Value], selection: &[Value]) -> (Vec<String>, Vec<Value>) {
    fn visit(
        key: &str,
        assets: &[Value],
        selected: &HashSet<String>,
        path: &mut Vec<String>,
        done: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if let Some(start) = path.iter().position(|p| p == key) {
            let mut chain = path[start..].to_vec();
            chain.push(key.into());
            return Err(format!("图片依赖循环：{}", chain.join(" → ")));
        }
        if done.contains(key) {
            return Ok(());
        }
        let asset = assets
            .iter()
            .find(|a| s(a, "id") == key && !crate::store::asset_deleted(a))
            .ok_or_else(|| format!("图片范围中的资产不存在：{key}"))?;
        path.push(key.into());
        for parent in dependencies(assets, asset)? {
            visit(&parent, assets, selected, path, done, order)?;
        }
        path.pop();
        done.insert(key.into());
        if selected.contains(key) {
            order.push(key.into());
        }
        Ok(())
    }
    let selected: HashSet<String> = selection
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut order = Vec::new();
    let mut done = HashSet::new();
    let mut blocked = Vec::new();
    for key in selection {
        let Some(key) = key.as_str().filter(|key| !key.is_empty()) else {
            blocked.push(json!({"assetId":key,"reason":"图片范围需要稳定资产 ID"}));
            continue;
        };
        let before = done.clone();
        let len = order.len();
        if let Err(error) = visit(
            key,
            assets,
            &selected,
            &mut Vec::new(),
            &mut done,
            &mut order,
        ) {
            done = before;
            order.truncate(len);
            blocked.push(json!({"assetId":key,"reason":error}));
        }
    }
    (order, blocked)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adoption_status_blocks_unusable_mother_without_erasing_old_adoption_or_candidates() {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::open(temporary.path().join("project")).unwrap();
        store.create("synthetic mother status", "p").unwrap();
        let mother = store.save_asset(json!({"id":"mother","variantId":"v1","kind":"character",
            "name":"合成母版","workflowProfile":PROFILE,"referenceRequirements":[]})).unwrap();
        let child = store.save_asset(json!({"id":"child","variantId":"v1","kind":"character",
            "name":"合成子项","workflowProfile":PROFILE,
            "referenceRequirements":[{"targetAssetId":"mother","targetVariantId":"v1","role":"reference"}]})).unwrap();
        let independent = store.save_asset(json!({"id":"independent","variantId":"v1","kind":"scene",
            "name":"合成独立场景","workflowProfile":PROFILE,"referenceRequirements":[]})).unwrap();
        let path = temporary.path().join("mother.png");
        image::RgbImage::from_fn(4, 4, |x, y| image::Rgb([45, x as u8 * 40, y as u8 * 40]))
            .save(&path).unwrap();
        let media = store.import_media(&path, "image", Some("mother"), Some("v1")).unwrap();
        assert!(media_references(&store, &store.values("assets").unwrap(), &child).unwrap().is_none(),
            "an unadopted candidate never unlocks a child");
        store.adopt("mother", "v1", s(&media, "id")).unwrap();
        // The selection remains in the candidate/history table after adoption.
        let bytes = std::fs::read(store.verify_media(&media).unwrap()).unwrap();
        for (status, blocked) in [
            (None, false), (Some(""), false), (Some("adopted"), false), (Some("approved"), false),
            (Some("needs_review"), true), (Some("stale"), true), (Some("rejected"), true),
            (Some("invalid"), true), (Some("needs_repair"), true), (Some("unknown"), true),
            (Some("submission_unknown"), true), (Some("candidate"), true), (Some("pending_review"), true),
        ] {
            let mut projection = store.active_asset("mother").unwrap();
            if let Some(status) = status { projection["imageStatus"] = json!(status); }
            else { projection.as_object_mut().unwrap().remove("imageStatus"); }
            store.conn().unwrap().execute("UPDATE assets SET data=?1 WHERE id='mother'", [projection.to_string()]).unwrap();
            let result = media_references(&store, &store.values("assets").unwrap(), &child).unwrap();
            assert_eq!(result.is_none(), blocked, "{status:?}");
            if let Some(references) = result { assert_eq!(references[0]["mediaId"], media["id"]); }
            assert_eq!(media_references(&store, &store.values("assets").unwrap(), &independent).unwrap(), Some(vec![]));
            assert_eq!(store.adopted("mother", "v1").unwrap().as_deref(), media["id"].as_str());
            assert_eq!(store.active_asset("mother").unwrap(), projection);
            assert_eq!(store.active_asset("mother").unwrap()["version"], mother["version"]);
            assert_eq!(std::fs::read(store.verify_media(&media).unwrap()).unwrap(), bytes);
        }
        assert!(store.job_summaries().unwrap().is_empty());
    }
    #[test]
    fn family_members_do_not_implicitly_require_images() {
        let a = json!({"id":"child","identityId":"p","sourceAssetId":"base","referenceRequirements":[]});
        assert!(dependencies(&[a.clone()], &a).unwrap().is_empty());
    }
    #[test]
    fn explicit_scope_never_silently_generates_ancestors() {
        let assets = vec![
            json!({"id":"a"}),
            json!({"id":"b","referenceRequirements":[{"targetAssetId":"a"}]}),
        ];
        let (order, blocked) = explicit_order(&assets, &[json!("b")]);
        assert_eq!(order, vec!["b"]);
        assert!(blocked.is_empty());
    }
    #[test]
    fn eleven_levels_and_independent_branches_are_stably_ordered() {
        let mut assets = vec![json!({"id":"a0"}), json!({"id":"independent"})];
        for i in 1..11 {
            assets.push(json!({"id":format!("a{i}"),"referenceRequirements":[{"targetAssetId":format!("a{}",i-1)}]}));
        }
        let mut selected: Vec<_> = (0..11).rev().map(|i| json!(format!("a{i}"))).collect();
        selected.push(json!("independent"));
        let (order, errors) = explicit_order(&assets, &selected);
        assert!(errors.is_empty());
        assert_eq!(order.len(), 12);
        for i in 0..11 {
            assert_eq!(order[i], format!("a{i}"));
        }
    }
    #[test]
    fn cycles_report_chain_and_leave_independent_work() {
        let assets = vec![
            json!({"id":"a","referenceRequirements":[{"targetAssetId":"b"}]}),
            json!({"id":"b","referenceRequirements":[{"targetAssetId":"a"}]}),
            json!({"id":"safe"}),
        ];
        let (order, errors) = explicit_order(&assets, &[json!("a"), json!("safe")]);
        assert_eq!(order, vec!["safe"]);
        assert!(s(&errors[0], "reason").contains("a → b → a"));
    }
    #[test]
    fn face_binding_cannot_cross_identity_or_variant() {
        let a = json!({"id":"a","identityId":"person1","referenceRequirements":[{"targetAssetId":"b","role":"face"}]});
        let b = json!({"id":"b","identityId":"person2","variantId":"base"});
        assert!(dependencies(&[a.clone(), b], &a)
            .unwrap_err()
            .contains("同一角色"));
    }
    #[test]
    fn handoff_kind_roles_and_explicit_base_identity_are_validated() {
        let child = json!({"id":"child","baseAssetId":"one","referenceRequirements":[{"targetAssetId":"mother","kind":"identity"}]});
        let mother = json!({"id":"mother","baseAssetId":"two","variantId":"original"});
        assert!(dependencies(&[child.clone(), mother.clone()], &child)
            .unwrap_err()
            .contains("同一角色"));
        let mut compatible = child;
        compatible["baseAssetId"] = json!("two");
        compatible["referenceRequirements"][0]["targetVariantId"] = json!("future");
        assert!(dependencies(&[compatible.clone(), mother], &compatible)
            .unwrap_err()
            .contains("版本不匹配"));
    }
    #[test]
    fn malformed_or_missing_dependency_never_becomes_an_independent_image() {
        let malformed = json!({"id":"a","referenceRequirements":{"targetAssetId":"b"}});
        assert!(dependencies(&[malformed.clone()], &malformed).is_err());
        let missing = json!({"id":"a","referenceRequirements":[{"targetAssetId":"b"}]});
        let safe = json!({"id":"safe"});
        let (order, blocked) = explicit_order(&[missing, safe], &[json!("a"), json!("safe")]);
        assert_eq!(order, vec!["safe"]);
        assert!(s(&blocked[0], "reason").contains("a → b"));
    }
    #[test]
    fn unresolved_visible_assets_stay_in_completion_scope() {
        assert!(image_required(
            &json!({"kind":"character","preparation":{"mode":"UNRESOLVED"}})
        ));
        assert!(!image_required(
            &json!({"kind":"audio","preparation":{"mode":"AUDIO"}})
        ));
        assert!(!image_required(
            &json!({"kind":"character","purpose":"mention-only"})
        ));
    }
}
