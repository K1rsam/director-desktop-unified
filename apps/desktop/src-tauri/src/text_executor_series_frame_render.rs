//! Read-only selection of an exact legacy or new review-repair request.
//! A known/unknown existing node never gains a new request through this hook.
use super::*;
const HELD: &str = "SERIES_FRAME_RENDER_EXISTING_NODE_HELD";

impl SavedTextExecutor {
    fn frame_render_saved_domain(&self, snapshot: &Value) -> Result<Value> {
        if snapshot.get("sdkSchemaRejectionRetry").is_some()
            || snapshot.get("sdkSchemaContinuation").is_some()
        {
            let request: TextRequest =
                serde_json::from_value(snapshot.clone()).map_err(|_| HELD)?;
            let domain = self.request_domain_snapshot(&request)?;
            // Reuse the schema recovery's full ownership, refusal/anchor,
            // frozen-file and checkpoint proof. Merely removing its metadata
            // could turn a changed or orphaned node into a new request.
            if self.sdk_schema_saved_snapshot(&request, &domain)?.as_ref() != Some(snapshot) {
                return Err(HELD.into());
            }
            return Ok(domain);
        }
        if snapshot.get("sdkSupervisorHandoff").is_some() {
            return self
                .sdk_handoff_existing_domain(snapshot)?
                .ok_or_else(|| HELD.into());
        }
        let mut domain = snapshot.clone();
        if domain
            .as_object_mut()
            .ok_or(HELD)?
            .remove("executorIdentity")
            .is_none()
        {
            return Err(HELD.into());
        }
        Ok(domain)
    }
    fn frame_render_review_received(&self, run: &str, review: &Value) -> Result<()> {
        let request: TextRequest =
            serde_json::from_value(review["request"].clone()).map_err(|_| HELD)?;
        let c = self.store.conn()?;
        let row: (String,String,String,String,String,String,String,String,String) = c.query_row(
            "SELECT w.request_hash,w.state,w.result,j.snapshot,j.kind,j.provider,j.state,j.result,j.receipt FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
            params![run,step_key(&request)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(|_|HELD)?;
        let (hash, state, result, text, kind, provider, job_state, raw, receipt) = row;
        let snapshot: Value = serde_json::from_str(&text).map_err(|_| HELD)?;
        let receipt: Value = serde_json::from_str(&receipt).map_err(|_| HELD)?;
        let raw: Value = serde_json::from_str(&raw).map_err(|_| HELD)?;
        let result: Value = serde_json::from_str(&result).map_err(|_| HELD)?;
        if snapshot.to_string() != text
            || crate::pipeline::digest(&text) != hash
            || state != "accepted"
            || job_state != "completed"
            || kind != "text"
            || provider != s(&self.settings, "textProvider")
            || raw != review["raw"]
            || result != raw
            || receipt["contract"] != "desktop-runner/v1"
            || receipt["validation"] != "accepted"
            // Normal accept writes the original two-field runner receipt.
            // Its request binding is already proven by the canonical snapshot,
            // checkpoint and raw event below. A migrated/extended receipt must
            // still carry its exact accepted hash; absence is not a wildcard.
            || match receipt.get("acceptedRequestHash") {
                Some(accepted_hash) => accepted_hash != &json!(hash),
                None => {
                    receipt
                        != json!({"contract":"desktop-runner/v1","validation":"accepted"})
                }
            }
            || self.frame_render_saved_domain(&snapshot)?
                != self.request_domain_snapshot(&request)?
        {
            return Err(HELD.into());
        }
        let event:String=c.query_row("SELECT e.data FROM events e JOIN workflow_steps w ON w.text_job_id=e.job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND e.type='text_received_raw' ORDER BY e.id DESC LIMIT 1",params![run,step_key(&request)],|r|r.get(0)).map_err(|_|HELD)?;
        let event: Value = serde_json::from_str(&event).map_err(|_| HELD)?;
        if event["requestHash"] != hash || event["value"] != raw {
            return Err(HELD.into());
        }
        Ok(())
    }
    pub(super) fn saved_series_frame_render(
        &self,
        legacy: TextRequest,
        projected: TextRequest,
        review: &Value,
    ) -> Result<TextRequest> {
        if serde_json::to_value(&legacy).map_err(|_| HELD)?
            == serde_json::to_value(&projected).map_err(|_| HELD)?
        {
            return Ok(legacy);
        }
        let run = self.workflow_id.as_deref().ok_or(HELD)?;
        let c = self.store.conn()?;
        let parent:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|r.get(0)).map_err(|_|HELD)?;
        let parent: Value = serde_json::from_str(&parent).map_err(|_| HELD)?;
        if parent["input"]["settings"] != self.settings || step_key(&legacy) != step_key(&projected)
        {
            return Err(HELD.into());
        }
        crate::pipeline::series::validate_frame_render_projection(
            &legacy,
            &projected,
            &parent["input"],
            review,
        )?;
        let old_domain = self.request_domain_snapshot(&legacy)?;
        let new_domain = self.request_domain_snapshot(&projected)?;
        let saved:Option<(String,String,String,String,String)>=c.query_row("SELECT w.request_hash,j.snapshot,j.kind,j.provider,w.stage FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",params![run,step_key(&legacy)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|HELD)?;
        if let Some((hash, text, kind, provider, stage)) = saved {
            let snapshot: Value = serde_json::from_str(&text).map_err(|_| HELD)?;
            if text != snapshot.to_string()
                || hash != crate::pipeline::digest(&text)
                || kind != "text"
                || provider != s(&self.settings, "textProvider")
                || stage != legacy.stage
            {
                return Err(HELD.into());
            }
            let domain = self.frame_render_saved_domain(&snapshot)?;
            // State intentionally does not select a new request: UNKNOWN,
            // received, rejected and accepted all retain their frozen domain.
            return if domain == old_domain {
                Ok(legacy)
            } else if domain == new_domain {
                Ok(projected)
            } else {
                Err(HELD.into())
            };
        }
        let mut q=c.prepare("SELECT j.snapshot FROM jobs j WHERE j.kind='text' AND json_extract(j.snapshot,'$.stage')='seriesRepair' AND NOT EXISTS(SELECT 1 FROM workflow_steps w WHERE w.text_job_id=j.id)").map_err(|_|HELD)?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| HELD)?;
        for row in rows {
            let text = row.map_err(|_| HELD)?;
            let snapshot: Value = serde_json::from_str(&text).map_err(|_| HELD)?;
            let mut domain = snapshot.clone();
            domain
                .as_object_mut()
                .ok_or(HELD)?
                .remove("executorIdentity");
            if domain == old_domain
                || domain == new_domain
                || (snapshot["input"]["sourceSha256"] == legacy.input["sourceSha256"]
                    && snapshot["input"]["executionRoleId"] == legacy.input["executionRoleId"])
            {
                return Err(HELD.into());
            }
        }
        self.frame_render_review_received(run, review)?;
        Ok(projected)
    }
}

#[cfg(test)]
#[path = "text_executor_series_frame_render_tests.rs"]
mod tests;
