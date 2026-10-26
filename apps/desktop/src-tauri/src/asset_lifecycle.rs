//! Derived dependency state. It never rewrites adopted design/media or a sent
//! request, and never increments designRevision to represent an image decision.
use crate::{
    asset_graph,
    pipeline::digest,
    store::{now, parse, s, Result},
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};

fn rows(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}
fn key(asset: &Value) -> String {
    format!(
        "asset.lifecycle:{}",
        digest(&json!([asset["id"], asset["variantId"]]).to_string())
    )
}
fn asset(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    let raw: String = tx
        .query_row("SELECT data FROM assets WHERE id=?1", [id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    parse(raw)
}
fn all_assets(tx: &Transaction<'_>) -> Result<Vec<Value>> {
    let mut q = tx
        .prepare("SELECT data FROM assets ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let raw = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    raw.into_iter().map(parse).collect()
}
fn state(tx: &Transaction<'_>, a: &Value) -> Result<Value> {
    let raw: Option<String> = tx
        .query_row("SELECT value FROM metadata WHERE key=?1", [key(a)], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(parse)
        .transpose()
        .map(|v| v.unwrap_or_else(|| json!({"version":"asset-lifecycle/v1"})))
}
fn current_media(tx: &Transaction<'_>, id: &str, variant: &str) -> Result<Option<String>> {
    tx.query_row(
        "SELECT media_id FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
        params![id, variant],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}
fn save(tx: &Transaction<'_>, a: &Value, mut state: Value) -> Result<()> {
    let mut projection = a.clone();
    for field in [
        "imageStatus",
        "referenceBindings",
        "imageAdoptionBasis",
        "imageReviewReasons",
        "designStatus",
        "designReviewReasons",
        "promptStatus",
        "promptReviewReasons",
    ] {
        if let Some(value) = state.get(field) {
            projection[field] = value.clone();
        }
    }
    state["assetId"] = a["id"].clone();
    state["variantId"] = a["variantId"].clone();
    // These are projections of separate adoption/dependency facts. The immutable
    // design and asset version history are not invented anew for media state.
    tx.execute(
        "UPDATE assets SET data=?1 WHERE id=?2 AND variant_id=?3",
        params![projection.to_string(), s(a, "id"), s(a, "variantId")],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO metadata VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key(a), state.to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
fn push_reason(state: &mut Value, field: &str, reason: Value) {
    let mut reasons = rows(state, field);
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
    state[field] = json!(reasons);
}
fn revision(a: &Value) -> Value {
    [
        a["adoptedDesign"].get("designRevision"),
        a["adoptedDesign"].get("revision"),
        a.get("designRevision"),
    ]
    .into_iter()
    .flatten()
    .find(|v| !v.is_null())
    .cloned()
    .unwrap_or(Value::Null)
}
fn design_signature(a: &Value) -> Value {
    json!({"variantId":a["variantId"],"identityId":a["identityId"],"full":a["adoptedDesign"]["full"],
        "revision":revision(a),"sourceSha256":a["adoptedDesign"]["sourceSha256"],
        "designDependencies":a["adoptedDesign"].get("designDependencies").or_else(||a.get("designDependencies"))})
}

fn has_design(a: &Value) -> bool {
    !a["adoptedDesign"].is_null() || !revision(a).is_null()
}

/// Prove the old identity-only edit bug from the complete, contiguous version
/// history. Missing history or any past design is not permission to clear a gate.
fn md_only_gate_proof(tx: &Transaction<'_>, current: &Value, own: &Value) -> Result<Option<Value>> {
    let reasons = rows(own, "designReviewReasons");
    let expected =
        json!({"kind":"adopted-design-edited","assetId":current["id"],"currentRevision":null});
    if current["origin"] != "imported-storyboard"
        || has_design(current)
        || !current["design"].is_null()
        || own["designStatus"] != "needs_review"
        || reasons != vec![expected.clone()]
        || crate::store::asset_deleted(current)
    {
        return Ok(None);
    }
    let Some(version) = current["version"].as_i64().filter(|v| *v >= 2) else {
        return Ok(None);
    };
    let mut query = tx
        .prepare("SELECT version,data FROM asset_versions WHERE asset_id=?1 ORDER BY version")
        .map_err(|e| e.to_string())?;
    let history = query
        .query_map([s(current, "id")], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if history.len() as i64 != version {
        return Ok(None);
    }
    let mut previous: Option<Value> = None;
    let mut identity_changed = false;
    let mut hashes = Vec::new();
    for (index, (number, raw)) in history.iter().enumerate() {
        let value = parse(raw.clone())?;
        if *number != index as i64 + 1
            || value["version"] != *number
            || value["id"] != current["id"]
            || value["variantId"] != current["variantId"]
            || value["origin"] != "imported-storyboard"
            || has_design(&value)
            || !value["design"].is_null()
        {
            return Ok(None);
        }
        let old_reasons = rows(&value, "designReviewReasons");
        if (!old_reasons.is_empty() && old_reasons != vec![expected.clone()])
            || (asset_graph::design_review_blocked(&value)
                && (value["designStatus"] != "needs_review"
                    || old_reasons != vec![expected.clone()]))
        {
            return Ok(None);
        }
        if let Some(before) = &previous {
            let mut before_signature = design_signature(before);
            let mut next_signature = design_signature(&value);
            before_signature
                .as_object_mut()
                .unwrap()
                .remove("identityId");
            next_signature.as_object_mut().unwrap().remove("identityId");
            if before_signature != next_signature {
                return Ok(None);
            }
            identity_changed |= before["identityId"] != value["identityId"];
        }
        hashes.push(json!({"version":number,"sha256":digest(raw)}));
        previous = Some(value);
    }
    if !identity_changed
        || previous
            .as_ref()
            .is_none_or(|last| design_signature(last) != design_signature(current))
    {
        return Ok(None);
    }
    Ok(Some(
        json!({"version":"md-only-identity-gate-proof/v1","assetId":current["id"],"variantId":current["variantId"],
        "assetVersion":version,"history":hashes,"historyComplete":true,"adoptedDesignEverPresent":false,"identityOnlySignatureChanges":true,"originalReason":expected}),
    ))
}

fn repair_md_only_gate(tx: &Transaction<'_>, current: &Value, own: &mut Value) -> Result<bool> {
    let Some(proof) = md_only_gate_proof(tx, current, own)? else {
        return Ok(false);
    };
    let repair = json!({"version":"md-only-identity-gate-repair/v1","proof":proof,"originalDesignStatus":own["designStatus"],
        "originalDesignReviewReasons":own["designReviewReasons"],"repairedAt":now(),"modelCalls":0,"originalRequestsUnchanged":true});
    own["designStatus"] = json!("needsDesign");
    own["designReviewReasons"] = json!([]);
    own["mdOnlyDesignGateRepair"] = repair.clone();
    tx.execute(
        "INSERT INTO events(job_id,type,data,created_at) VALUES('',?1,?2,?3)",
        params![
            "md_only_identity_design_gate_repaired",
            repair.to_string(),
            now()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

impl crate::store::Store {
    /// Explicit images.design preflight only; this does not adopt a design,
    /// refresh words/images, touch original attempts, or widen the user scope.
    pub(crate) fn repair_md_only_identity_design_gates(
        &self,
        request: &Value,
    ) -> Result<Vec<String>> {
        let scope = self.image_production_scope()?;
        let selected = request
            .get("assetIds")
            .cloned()
            .unwrap_or_else(|| scope["assetIds"].clone());
        let ids = selected
            .as_array()
            .filter(|ids| !ids.is_empty())
            .ok_or("图片范围必须为非空资产 ID 数组")?;
        let mut seen = HashSet::new();
        for id in ids {
            let key = id
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("图片范围包含无效 ID")?;
            if !seen.insert(key)
                || (!rows(&scope, "assetIds").contains(id)
                    && !rows(&scope, "referenceOnlyIds").contains(id))
            {
                return Err("图片范围重复或不在当前正式图片及已登记参考范围".into());
            }
        }
        let mut connection = self.conn()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut repaired = Vec::new();
        for id in ids {
            let current = asset(&tx, id.as_str().unwrap())?;
            let mut own = state(&tx, &current)?;
            if repair_md_only_gate(&tx, &current, &mut own)? {
                save(&tx, &current, own)?;
                repaired.push(s(&current, "id").to_owned());
            }
        }
        if !repaired.is_empty() {
            crate::store::mark_changed(&tx)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(repaired)
    }
}

/// A retained image can be stale even when its bytes and adoption ID have not
/// changed. Follow only the media that each adopted consumer actually used;
/// family identity, planned dependencies and candidates never widen this set.
fn propagate_media_review(tx: &Transaction<'_>, seeds: Vec<Value>) -> Result<()> {
    let assets = all_assets(tx)?;
    let mut queue: VecDeque<Value> = seeds.into();
    let mut visited = HashSet::new();
    while let Some(parent) = queue.pop_front() {
        let parent_id = s(&parent, "id");
        let variant = s(&parent, "variantId");
        if !visited.insert((parent_id.to_owned(), variant.to_owned())) {
            continue;
        }
        let Some(mid) = current_media(tx, parent_id, variant)? else {
            continue;
        };
        for consumer in &assets {
            if consumer["id"] == parent_id
                || !asset_graph::modern(consumer)
                || crate::store::asset_deleted(consumer)
            {
                continue;
            }
            let mut affected = state(tx, consumer)?;
            let bindings = affected
                .get("referenceBindings")
                .unwrap_or(&consumer["referenceBindings"]);
            if !bindings.as_array().into_iter().flatten().any(|r| {
                r["assetId"] == parent_id
                    && r["mediaId"] == mid
                    && r["variantId"].as_str().is_none_or(|v| v == variant)
            }) {
                continue;
            }
            affected["imageStatus"] = json!("needs_review");
            push_reason(
                &mut affected,
                "imageReviewReasons",
                json!({
                    "kind":"reference-media-awaits-review","assetId":parent_id,
                    "variantId":variant,"frozenMediaId":mid,"currentMediaId":mid
                }),
            );
            save(tx, consumer, affected)?;
            queue.push_back(consumer.clone());
        }
    }
    Ok(())
}

/// The caller verifies the terminal visual receipt and full current basis while
/// holding this transaction. Preserve the selected image, source and versions;
/// only its usability and the actual media consumers' review state change.
pub(crate) fn after_image_review_rejection(
    tx: &Transaction<'_>,
    basis: &Value,
    review: &Value,
) -> Result<()> {
    let selected = asset(tx, s(basis, "assetId"))?;
    if selected["variantId"] != basis["variantId"]
        || selected["version"] != basis["assetVersion"]
        || selected["adoptedDesign"] != basis["adoptedDesign"]
        || selected["imagePrompt"] != basis["imagePrompt"]
    {
        return Err("VISUAL_REVIEW_BINDING_CHANGED".into());
    }
    if current_media(tx, s(&selected, "id"), s(&selected, "variantId"))?.as_deref()
        != Some(s(basis, "mediaId"))
    {
        return Ok(());
    }
    let mut own = state(tx, &selected)?;
    let reason = json!({"kind":"visual-review-needs-repair","assetId":basis["assetId"],
        "variantId":basis["variantId"],"mediaId":basis["mediaId"],"mediaSha256":basis["mediaSha256"],
        "reviewManifestSha256":review["manifestSha256"],"reviewResultSha256":digest(&review["output"].to_string()),
        "reviewBasisSha256":review["basisSha256"]});
    if own["imageStatus"] == "needs_repair" && rows(&own, "imageReviewReasons").contains(&reason) {
        return Ok(());
    }
    own["imageStatus"] = json!("needs_repair");
    push_reason(&mut own, "imageReviewReasons", reason.clone());
    save(tx, &selected, own)?;
    propagate_media_review(tx, vec![selected])?;
    tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'visual_review_current_image_needs_repair',?2,?3)",
        params![s(basis,"sourceJobId"),reason.to_string(),now()]).map_err(|e|e.to_string())?;
    Ok(())
}

/// Call after adoptions was updated in the same transaction. Bind the chosen
/// image to its own original job, never to today's mutable parent selection.
pub fn after_image_adoption(
    tx: &Transaction<'_>,
    asset_id: &str,
    variant: &str,
    media_id: &str,
    previous_media: Option<&str>,
) -> Result<()> {
    let selected = asset(tx, asset_id)?;
    if !asset_graph::modern(&selected) {
        return Ok(());
    }
    if s(&selected, "variantId") != variant {
        return Err("图片采用与当前形象身份不匹配".into());
    }
    if previous_media == Some(media_id) {
        let reasons = rows(&selected, "imageReviewReasons");
        if !matches!(s(&selected, "imageStatus"), "needs_review" | "stale")
            || reasons.is_empty()
            || reasons.iter().any(|reason| {
                !matches!(
                    s(reason, "kind"),
                    "reference-media-changed" | "reference-media-awaits-review"
                )
            })
        {
            // Re-selecting identical bytes is not a new quality decision and
            // cannot clear rejection, unknown work or an unrelated conflict.
            return Ok(());
        }
    }
    let mut q=tx.prepare("SELECT j.id,j.snapshot FROM job_media jm JOIN jobs j ON j.id=jm.job_id WHERE jm.media_id=?1 AND j.kind='image' ORDER BY j.rowid").map_err(|e|e.to_string())?;
    let jobs = q
        .query_map([media_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    let mut sources = Vec::new();
    for (id, raw) in jobs {
        let snapshot = parse(raw)?;
        if snapshot["assetId"] == asset_id && snapshot["variantId"] == variant {
            let refs = snapshot
                .get("referenceBindings")
                .filter(|v| v.is_array())
                .or_else(|| snapshot.get("references").filter(|v| v.is_array()))
                .cloned()
                .unwrap_or(json!([]));
            sources.push((id, refs, snapshot["assetVersion"].clone()));
        }
    }
    if sources.windows(2).any(|pair| pair[0].1 != pair[1].1) {
        return Err("所选图片存在不同原始参考链，未猜测采用依据".into());
    }
    let mut next = state(tx, &selected)?;
    let refs = sources.last().map(|v| v.1.clone()).unwrap_or(json!([]));
    let mut stale = Vec::new();
    if asset_graph::design_review_blocked(&selected) {
        stale.push(json!({"kind":"design-awaits-review","assetId":asset_id}));
    }
    for reference in refs.as_array().unwrap() {
        if s(reference, "assetId").is_empty()
            || s(reference, "mediaId").is_empty()
            || s(reference, "sha256").len() != 64
        {
            return Err("所选图片原参考媒体证据不完整".into());
        }
        let parent = asset(tx, s(reference, "assetId"))?;
        let adopted = current_media(tx, s(&parent, "id"), s(&parent, "variantId"))?;
        if adopted.as_deref() != Some(s(reference, "mediaId"))
            || asset_graph::image_adoption_blocked(&parent)
            || asset_graph::design_review_blocked(&parent)
        {
            stale.push(json!({"kind":"reference-media-changed","assetId":reference["assetId"],"frozenMediaId":reference["mediaId"],"currentMediaId":adopted}));
        }
    }
    next["referenceBindings"] = refs;
    next["imageStatus"] = json!(if stale.is_empty() {
        "adopted"
    } else {
        "needs_review"
    });
    next["imageReviewReasons"] = json!(stale);
    next["imageAdoptionBasis"] = json!({"version":"asset-image-adoption/v1","mediaId":media_id,
        "kind":if sources.is_empty(){"imported-current"}else{"user-selected-candidate"},
        "sourceJobId":sources.last().map(|v|v.0.as_str()),"sourceAssetVersion":sources.last().map(|v|&v.2),
        "qualityEvidence":"explicit-selection-not-system-visual-review","adoptedAt":now()});
    let mut affected_images = Vec::new();
    if next["imageStatus"] == "needs_review" {
        affected_images.push(selected.clone());
    }
    save(tx, &selected, next)?;
    if previous_media == Some(media_id) {
        return propagate_media_review(tx, affected_images);
    }
    for consumer in all_assets(tx)? {
        if consumer["id"] == asset_id
            || !asset_graph::modern(&consumer)
            || crate::store::asset_deleted(&consumer)
        {
            continue;
        }
        let mut current = state(tx, &consumer)?;
        let bindings = current
            .get("referenceBindings")
            .unwrap_or(&consumer["referenceBindings"]);
        let used: Vec<_> = bindings
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| {
                r["assetId"] == asset_id
                    && r["variantId"].as_str().is_none_or(|v| v == variant)
                    && r["mediaId"] != media_id
                    && previous_media.is_none_or(|old| r["mediaId"] == old)
            })
            .cloned()
            .collect();
        if used.is_empty() {
            continue;
        }
        for binding in used {
            push_reason(
                &mut current,
                "imageReviewReasons",
                json!({"kind":"reference-media-changed","assetId":asset_id,"variantId":variant,"frozenMediaId":binding["mediaId"],"currentMediaId":media_id}),
            );
        }
        current["imageStatus"] = json!("needs_review");
        save(tx, &consumer, current)?;
        affected_images.push(consumer);
    }
    propagate_media_review(tx, affected_images)
}

/// Call after an asset row is saved in the same transaction. Image changes are
/// handled above; only adopted design content/version invalidates design users.
pub fn after_asset_save(
    tx: &Transaction<'_>,
    previous: Option<&Value>,
    current: &Value,
) -> Result<()> {
    if !asset_graph::modern(current)
        && current["adoptedDesign"]["version"] != "series-full-design/v1"
    {
        return Ok(());
    }
    let changed = previous.is_some_and(|old| design_signature(old) != design_signature(current));
    let mut own = state(tx, current)?;
    repair_md_only_gate(tx, current, &mut own)?;
    if changed {
        let fresh_adoption = current["adoptedDesign"]["status"] == "ADOPTED"
            && current["adoptedDesign"]["adoptionBasis"].is_object()
            && previous.is_some_and(|old| {
                revision(current) != revision(old)
                    && current["adoptedDesign"]["adoptionBasis"]
                        != old["adoptedDesign"]["adoptionBasis"]
            });
        // MD-only assets can acquire an explicit identity/reference before any
        // adopted text design exists. That is new input for prompt authoring,
        // not an edit to a nonexistent adopted design. Keep invalidation for
        // any real prior/current design, including removal of a former design.
        if has_design(current) || previous.is_some_and(has_design) {
            own["designStatus"] = json!(if fresh_adoption {
                "adopted"
            } else {
                "needs_review"
            });
            own["designReviewReasons"] = if fresh_adoption {
                json!([])
            } else {
                json!([{"kind":"adopted-design-edited","assetId":current["id"],"currentRevision":revision(current)}])
            };
        }
        if current["imagePrompt"].is_object() {
            own["promptStatus"] = json!("needs_review");
        }
        if current_media(tx, s(current, "id"), s(current, "variantId"))?.is_some() {
            own["imageStatus"] = json!("needs_review");
            push_reason(
                &mut own,
                "imageReviewReasons",
                json!({"kind":"own-design-changed","assetId":current["id"],"currentRevision":revision(current)}),
            );
        }
    } else if current["imagePrompt"].is_object()
        && !crate::asset_graph::design_review_blocked(&own)
        && current["imagePrompt"]["basedOnDesignRevision"] == revision(current)
        && previous.is_some_and(|old| old["imagePrompt"] != current["imagePrompt"])
        && current["promptStatus"] == "ready"
        && crate::asset_workflow_store::validate_prompt_source_tx(tx, &current["imagePrompt"])
            .is_ok()
    {
        own["promptStatus"] = json!("ready");
        own["promptReviewReasons"] = json!([]);
    }
    if current["imagePrompt"]["sourceBindingContract"]
        == crate::pipeline::asset_workflow::SOURCE_BINDING_V1
        && crate::asset_workflow_store::validate_prompt_source_tx(tx, &current["imagePrompt"])
            .is_err()
    {
        own["promptStatus"] = json!("needs_review");
    }
    if current["promptStatus"] == "needs_review"
        && current["imagePrompt"]["status"] == "needs_review"
    {
        // Saving unverified manual words is a draft, even when the previous
        // lifecycle projection was ready. This is conservative, never approval.
        own["promptStatus"] = json!("needs_review");
    }
    save(tx, current, own)?;
    if !changed {
        return Ok(());
    }
    let assets = all_assets(tx)?;
    let mut design_queue = VecDeque::from([current.clone()]);
    let mut seen = HashSet::from([s(current, "id").to_owned()]);
    let mut affected_images = vec![current.clone()];
    while let Some(parent) = design_queue.pop_front() {
        for consumer in &assets {
            if seen.contains(s(consumer, "id")) || crate::store::asset_deleted(consumer) {
                continue;
            }
            let deps = consumer["adoptedDesign"]
                .get("designDependencies")
                .or_else(|| consumer.get("designDependencies"));
            if !deps
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|d| d["assetId"] == parent["id"] || d["targetAssetId"] == parent["id"])
            {
                continue;
            }
            let mut affected = state(tx, consumer)?;
            affected["designStatus"] = json!("needs_review");
            affected["promptStatus"] = json!("needs_review");
            let direct = parent["id"] == current["id"];
            push_reason(
                &mut affected,
                "designReviewReasons",
                json!({
                    "kind":if direct {"parent-design-changed"} else {"parent-design-awaits-review"},
                    "assetId":parent["id"],"rootAssetId":current["id"],
                    "previousRevision":if direct {previous.map(revision)} else {Some(revision(&parent))},
                    "currentRevision":revision(&parent)
                }),
            );
            if current_media(tx, s(consumer, "id"), s(consumer, "variantId"))?.is_some() {
                affected["imageStatus"] = json!("needs_review");
                affected_images.push(consumer.clone());
            }
            save(tx, consumer, affected)?;
            seen.insert(s(consumer, "id").to_owned());
            design_queue.push_back(consumer.clone());
        }
    }
    propagate_media_review(tx, affected_images)
}

/// Unit edits invalidate only prompts which actually froze that unit as input.
/// Keep manual words, adopted images, versions and all original jobs untouched.
pub(crate) fn after_unit_save(
    tx: &Transaction<'_>,
    previous: Option<&Value>,
    current: &Value,
) -> Result<()> {
    if previous.is_none() {
        return Ok(());
    }
    if previous.is_some_and(|old| {
        old["retired"] == current["retired"]
            && crate::asset_workflow_store::source_unit_binding(old)
                == crate::asset_workflow_store::source_unit_binding(current)
    }) {
        return Ok(());
    }
    let mut query=tx.prepare("SELECT data FROM assets WHERE json_extract(data,'$.imagePrompt.sourceBindingContract')=?1 AND EXISTS(SELECT 1 FROM json_each(assets.data,'$.imagePrompt.sourceUnitBindings') b WHERE json_extract(b.value,'$.unitId')=?2)").map_err(|e|e.to_string())?;
    let consumers = query
        .query_map(
            params![
                crate::pipeline::asset_workflow::SOURCE_BINDING_V1,
                s(current, "id")
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(query);
    for raw in consumers {
        let consumer = parse(raw)?;
        let prompt = &consumer["imagePrompt"];
        if crate::store::asset_deleted(&consumer)
            || prompt["sourceBindingContract"] != crate::pipeline::asset_workflow::SOURCE_BINDING_V1
            || !rows(prompt, "sourceUnitBindings")
                .iter()
                .any(|b| b["unitId"] == current["id"])
            || crate::asset_workflow_store::validate_prompt_source_tx(tx, prompt).is_ok()
        {
            continue;
        }
        let mut state = state(tx, &consumer)?;
        state["promptStatus"] = json!("needs_review");
        push_reason(
            &mut state,
            "promptReviewReasons",
            json!({"kind":"source-unit-changed","unitId":current["id"],
            "currentPromptSha256":digest(s(current,"prompt")),"originalPromptPreserved":true}),
        );
        save(tx, &consumer, state)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "asset_lifecycle_tests.rs"]
mod tests;
