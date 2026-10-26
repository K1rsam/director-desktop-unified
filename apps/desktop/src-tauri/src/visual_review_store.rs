use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

/// This is only a conservative veto from retained facts, never an approval or
/// a new quality verdict. Do not open receipt files or recurse into current
/// candidate_basis here: this code also runs during ordinary asset polling.
fn retained_negative_review<'a>(value: &'a Value, media_id: &str) -> Option<&'a Value> {
    let mut current = value;
    for _ in 0..4 {
        let same_media = s(current, "mediaId").is_empty() || current["mediaId"] == media_id;
        if same_media
            && (current["status"] == "needs_repair" || current["output"]["decision"] == "REPAIR")
        {
            return Some(current);
        }
        current = current.get("originalReview")?;
    }
    None
}
impl Store {
    /// One bounded database query for the current variants. No SDK, filesystem,
    /// model, candidate-basis or recursive parent validation occurs here.
    pub(crate) fn persisted_image_review_guards(
        &self,
        connection: &Connection,
        asset_id: Option<&str>,
    ) -> Result<HashMap<(String, String), Value>> {
        let mut query = connection.prepare(
            "SELECT d.asset_id,d.variant_id,d.media_id,m.value,json_extract(i.data,'$.visualReview') \
             FROM adoptions d JOIN assets a ON a.id=d.asset_id AND a.variant_id=d.variant_id \
             LEFT JOIN metadata m ON m.key='visual-review.media:'||d.media_id \
             LEFT JOIN media i ON i.id=d.media_id \
             WHERE (?1 IS NULL OR d.asset_id=?1) \
             AND (m.value IS NOT NULL OR json_type(i.data,'$.visualReview')='object')"
        ).map_err(|e|e.to_string())?;
        let rows = query
            .query_map([asset_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut guards = HashMap::new();
        for row in rows {
            let (aid, variant, mid, raw, media_raw) = row.map_err(|e| e.to_string())?;
            for text in [raw, media_raw].into_iter().flatten() {
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                if let Some(review) = retained_negative_review(&value, &mid) {
                    guards.insert((aid, variant), json!({
                        "kind":"retained-visual-review-needs-review", "mediaId":mid,
                        "reviewManifestSha256":review["manifestSha256"],
                        "reviewBasisSha256":review["basisSha256"],
                        "reason":"当前采用图片曾被判需返修；旧审核依据可能变化，保留选择并待复核，不视为已通过"
                    }));
                    break;
                }
            }
        }
        Ok(guards)
    }
    pub(crate) fn project_image_review_guard(
        asset: &mut Value,
        guards: &HashMap<(String, String), Value>,
    ) {
        if crate::asset_graph::image_adoption_blocked(asset) {
            return;
        }
        let key = (s(asset, "id").to_owned(), s(asset, "variantId").to_owned());
        if let Some(reason) = guards.get(&key) {
            asset["imageStatus"] = json!("needs_review");
            let mut reasons = asset["imageReviewReasons"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !reasons.contains(reason) {
                reasons.push(reason.clone());
            }
            asset["imageReviewReasons"] = json!(reasons);
        }
    }

    pub fn visual_review_delegation(&self) -> Result<Value> {
        let raw: Option<String> = self
            .conn()?
            .query_row(
                "SELECT value FROM metadata WHERE key='visual-review.delegation'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let mut value: Value = raw
            .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
            .unwrap_or_else(|| Ok(json!({"enabled":false})))?;
        if value["enabled"] == true
            && (value["projectRoot"] != json!(self.root)
                || value["projectId"] != self.project()?["id"])
        {
            value["enabled"] = json!(false);
            value["reason"] = json!("委托只适用于原本机项目；导入或移动后默认手动采用");
        }
        Ok(value)
    }
    pub fn set_visual_review_delegation(&self, enabled: bool, settings: &Value) -> Result<Value> {
        let cap = if enabled {
            verify_capability(self, settings)?
        } else {
            Value::Null
        };
        let value = json!({"version":"project-visual-delegation/v1","projectId":self.project()?["id"],"projectRoot":self.root,"enabled":enabled,"scopeSha256":cap["scopeSha256"],"recordedAt":now(),"basis":"explicit-project-opt-in","automaticRegeneration":false});
        let mut c = self.conn()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO metadata(key,value) VALUES('visual-review.delegation',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value.to_string()]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO settings(key,value) VALUES('imageAdoptionPolicy',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[json!(if enabled{"delegated-review"}else{"manual"}).to_string()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(value)
    }
    pub fn due_visual_reviews(&self) -> Result<Vec<Value>> {
        if self.visual_review_delegation()?["enabled"] != true {
            return Ok(vec![]);
        }
        let c = self.conn()?;
        let mut q=c.prepare("SELECT j.entity_id,jm.media_id FROM jobs j JOIN job_media jm ON jm.job_id=j.id WHERE j.kind='image' AND j.state='downloaded' AND json_extract(j.snapshot,'$.imageAdoptionPolicy')='delegated-review' AND NOT EXISTS(SELECT 1 FROM metadata m WHERE m.key='visual-review.media:'||jm.media_id) ORDER BY j.created_at LIMIT 1").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([], |r| {
                Ok(json!({"assetId":r.get::<_,String>(0)?,"mediaId":r.get::<_,String>(1)?}))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }
    pub(crate) fn interrupted_visual_reviews(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT value FROM metadata WHERE key LIKE 'visual-review.media:%' AND json_extract(value,'$.status')='reviewing' ORDER BY key LIMIT 1").map_err(|e|e.to_string())?;
        let raw = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        raw.into_iter()
            .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
            .collect()
    }
    pub fn record_visual_review(&self, media_id: &str, result: &Value) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        // A negative review does not remove the user's selection, but a current
        // adopted image must no longer count as a usable mother/final export.
        // Revalidate the real receipt and current basis before changing either
        // projection. Old candidates cannot invalidate a later selection.
        if result["status"] == "needs_repair" {
            let basis = self.verified_repair_review_basis(media_id, result)?;
            crate::asset_lifecycle::after_image_review_rejection(&tx, &basis, result)?;
        }
        // A stale/unknown replay is not a new quality decision. Keep the
        // complete old negative fact instead of replacing it with a status
        // summary that would silently re-enable the same selected picture.
        let previous: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key=?1",
                [format!("visual-review.media:{media_id}")],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let previous = previous
            .map(|v| serde_json::from_str::<Value>(&v).map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(Value::Null);
        let mut retained = result.clone();
        if retained_negative_review(&retained, media_id).is_none() {
            if let Some(negative) = retained_negative_review(&previous, media_id) {
                retained["originalReview"] = negative.clone();
            }
        }
        let result = &retained;
        tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("visual-review.media:{media_id}"),result.to_string()]).map_err(|e|e.to_string())?;
        let projection = json!({"status":result["status"],"reason":result["reason"],"checkedAt":result["checkedAt"],"adopted":result["adopted"],"output":result["output"],"attemptDirectory":result["attemptDirectory"],"evidenceUnavailable":result["evidenceUnavailable"],"originalReview":result["originalReview"]});
        tx.execute(
            "UPDATE media SET data=json_set(data,'$.visualReview',json(?1)) WHERE id=?2",
            params![projection.to_string(), media_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()
    }
    fn verified_repair_review_basis(&self, media_id: &str, result: &Value) -> Result<Value> {
        let attempt = PathBuf::from(s(result, "attemptDirectory"));
        let relative = attempt
            .strip_prefix(root(self))
            .map_err(|_| "VISUAL_REVIEW_SCOPE")?;
        if relative.components().count() != 1
            || !relative.to_string_lossy().starts_with("candidate-")
        {
            return Err("VISUAL_REVIEW_SCOPE".into());
        }
        let (plan, output) = terminal(&attempt)?;
        let basis = &plan["basis"];
        if plan["purpose"] != "candidate"
            || result["version"] != "asset-visual-review/v1"
            || result["status"] != "needs_repair"
            || result["assetId"] != basis["assetId"]
            || result["mediaId"] != media_id
            || basis["mediaId"] != media_id
            || result["scopeSha256"] != scope(&plan["baseIdentity"])
            || result["basisSha256"] != hash(basis)
            || result["basis"] != *basis
            || result["output"] != output
            || result["manifestSha256"] != hash_file(&attempt.join("manifest.json"))?.0
            || output["decision"] != "REPAIR"
            || review_passes(&output, plan["images"].as_array().map_or(0, Vec::len))
        {
            return Err("VISUAL_REVIEW_PROOF_REQUIRED".into());
        }
        if review_key_basis(&candidate_basis(self, s(basis, "assetId"), media_id)?)
            != review_key_basis(basis)
        {
            return Err("VISUAL_REVIEW_BINDING_CHANGED".into());
        }
        Ok(basis.clone())
    }
    pub fn adopt_visual_review(&self, result: &Value) -> Result<bool> {
        let attempt = PathBuf::from(s(result, "attemptDirectory"));
        let relative = attempt
            .strip_prefix(root(self))
            .map_err(|_| "VISUAL_REVIEW_SCOPE")?;
        if relative.components().count() != 1
            || !relative.to_string_lossy().starts_with("candidate-")
        {
            return Err("VISUAL_REVIEW_SCOPE".into());
        }
        let (plan, output) = terminal(&attempt)?;
        if plan["purpose"] != "candidate"
            || result["version"] != "asset-visual-review/v1"
            || result["status"] != "passed"
            || result["scopeSha256"] != scope(&plan["baseIdentity"])
            || result["basisSha256"] != hash(&plan["basis"])
            || result["basis"] != plan["basis"]
            || result["output"] != output
            || result["manifestSha256"] != hash_file(&attempt.join("manifest.json"))?.0
            || !review_passes(&output, plan["images"].as_array().map_or(0, Vec::len))
        {
            return Err("VISUAL_REVIEW_PROOF_REQUIRED".into());
        }
        let basis = &plan["basis"];
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        // Current source/asset/mother checks are reread while the IMMEDIATE
        // transaction prevents a competing local write before adoption.
        if review_key_basis(&candidate_basis(
            self,
            s(basis, "assetId"),
            s(basis, "mediaId"),
        )?) != review_key_basis(basis)
        {
            return Err("VISUAL_ADOPTION_BASIS_CHANGED".into());
        }
        let delegation: String = tx
            .query_row(
                "SELECT value FROM metadata WHERE key='visual-review.delegation'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "VISUAL_DELEGATION_REQUIRED")?;
        let delegation: Value = serde_json::from_str(&delegation).map_err(|e| e.to_string())?;
        if delegation["enabled"] != true
            || delegation["scopeSha256"] != result["scopeSha256"]
            || delegation["projectRoot"] != json!(self.root)
            || delegation["projectId"] != self.project()?["id"]
        {
            return Err("VISUAL_DELEGATION_CHANGED".into());
        }
        let asset_raw: String = tx
            .query_row(
                "SELECT data FROM assets WHERE id=?1",
                [s(basis, "assetId")],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let asset: Value = serde_json::from_str(&asset_raw).map_err(|e| e.to_string())?;
        if asset["version"] != basis["assetVersion"]
            || asset["variantId"] != basis["variantId"]
            || asset["adoptedDesign"] != basis["adoptedDesign"]
            || asset["imagePrompt"] != basis["imagePrompt"]
            || asset["deletedAt"].as_str().is_some_and(|v| !v.is_empty())
        {
            return Err("VISUAL_ADOPTION_BASIS_CHANGED".into());
        }
        let existing: Option<String> = tx
            .query_row(
                "SELECT media_id FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
                params![s(basis, "assetId"), s(basis, "variantId")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if existing.as_deref() == Some(s(basis, "mediaId")) {
            return Ok(
                asset["imageAdoptionBasis"]["kind"] == "delegated-visual-review"
                    && asset["imageAdoptionBasis"]["reviewManifestSha256"]
                        == result["manifestSha256"],
            );
        }
        if json!(existing) != basis["previousAdoptedMediaId"] {
            return Err("VISUAL_LATER_USER_SELECTION_PRESERVED".into());
        }
        for item in
            std::iter::once(basis).chain(basis["references"].as_array().into_iter().flatten())
        {
            let mid = s(item, "mediaId");
            let raw: String = tx
                .query_row("SELECT data FROM media WHERE id=?1", [mid], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let media: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            let expected = if item == basis {
                &basis["mediaSha256"]
            } else {
                &item["sha256"]
            };
            if hash_file(Path::new(s(&media, "path")))?.0 != expected.as_str().unwrap_or("") {
                return Err("VISUAL_MEDIA_CHANGED".into());
            }
            if item != basis {
                let raw: String = tx
                    .query_row(
                        "SELECT data FROM assets WHERE id=?1",
                        [s(item, "assetId")],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let parent: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
                if parent["version"] != item["assetVersion"]
                    || parent["variantId"] != item["variantId"]
                    || parent["adoptedDesign"] != item["design"]
                    || crate::asset_graph::image_adoption_blocked(&parent)
                    || crate::asset_graph::design_review_blocked(&parent)
                    || parent["deletedAt"].as_str().is_some_and(|v| !v.is_empty())
                {
                    return Err("VISUAL_PARENT_DESIGN_CHANGED".into());
                }
                let current: Option<String> = tx
                    .query_row(
                        "SELECT media_id FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
                        params![s(item, "assetId"), s(item, "variantId")],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if current.as_deref() != Some(mid) {
                    return Err("VISUAL_PARENT_CHANGED".into());
                }
            }
        }
        crate::store::set_current_image(
            &tx,
            s(basis, "assetId"),
            s(basis, "variantId"),
            s(basis, "mediaId"),
            basis["assetVersion"].as_i64(),
            false,
        )?;
        let raw: String = tx
            .query_row(
                "SELECT data FROM assets WHERE id=?1",
                [s(basis, "assetId")],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut current: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if current["imageStatus"] != "adopted" {
            return Err("VISUAL_DEPENDENCY_REVIEW_REQUIRED".into());
        }
        let adoption = json!({"version":"asset-image-adoption/v2","kind":"delegated-visual-review","mediaId":basis["mediaId"],"reviewManifestSha256":result["manifestSha256"],"reviewResultSha256":hash(&output),"reviewBasisSha256":hash(basis),"delegation":delegation,"qualityEvidence":"actual-sdk-visual-input-and-verified-review","adoptedAt":now(),"anchorSummary":output["anchorSummary"]});
        current["imageAdoptionBasis"] = adoption.clone();
        tx.execute(
            "UPDATE assets SET data=?1 WHERE id=?2",
            params![current.to_string(), s(basis, "assetId")],
        )
        .map_err(|e| e.to_string())?;
        let key = format!(
            "asset.lifecycle:{}",
            hash(&json!([basis["assetId"], basis["variantId"]]))
        );
        let raw: String = tx
            .query_row("SELECT value FROM metadata WHERE key=?1", [&key], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        let mut lifecycle: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        lifecycle["imageAdoptionBasis"] = adoption.clone();
        tx.execute(
            "UPDATE metadata SET value=?1 WHERE key=?2",
            params![lifecycle.to_string(), key],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'visual_review_adopted',?2,?3)",params![s(basis,"sourceJobId"),adoption.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(true)
    }
}
