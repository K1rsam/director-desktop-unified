//! Inactive exact-response seam. This reuses the existing job scheduler and
//! UNKNOWN/account/CAS contracts; it is not a second dispatcher or adoption gate.
use super::*;
use crate::series_lite_transport::{RoleContext, TransportIdentity};

pub(super) const PROFILE: &str = "series-lite-exact-text/v1";
const ENVELOPE: &str = "model-response-utf8/v1";
const LIMIT: usize = 1_048_576;

/// Proven SDK final-response text / CLI result-file bytes. Not HTTP/runtime
/// stream bytes, valid domain JSON, independent review, or approval.
#[derive(Clone, Debug)]
pub struct VerifiedRawText {
    bytes: Vec<u8>,
    sha256: String,
    job_id: String,
    request_sha256: String,
    packet_sha256: String,
    snapshot: String,
    envelope: Value,
}
impl VerifiedRawText {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    pub fn request_sha256(&self) -> &str {
        &self.request_sha256
    }
    pub fn packet_sha256(&self) -> &str {
        &self.packet_sha256
    }
    /// Transport callback value only. Save composed candidate bytes separately;
    /// never replace this envelope with a parsed/normalized candidate.
    pub fn transport_envelope(&self) -> &Value {
        &self.envelope
    }
}
fn held() -> String {
    "LITE_EXACT_EVIDENCE_HELD: 原岗位包、响应文本或冻结证据不一致，未采用或重发".into()
}
fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn enabled(request: &TextRequest) -> bool {
    request.input["textExact"]["version"] == PROFILE
}

pub(super) fn validate_request(request: &TextRequest) -> Result<()> {
    let marked = request.input.get("textExact");
    let stage = request.stage.starts_with("series-lite:");
    if marked.is_none() && !stage {
        return Ok(());
    }
    if !enabled(request) {
        return Err(held());
    }
    let input = request.input.as_object().ok_or_else(held)?;
    if input.len() != 6
        || ![
            "transportProfile",
            "packetUtf8",
            "identity",
            "roleContext",
            "transportRule",
            "textExact",
        ]
        .iter()
        .all(|k| input.contains_key(*k))
    {
        return Err(held());
    }
    let marker = input["textExact"].as_object().ok_or_else(held)?;
    if marker.len() != 2
        || !marker.contains_key("version")
        || !marker.contains_key("baseSha256")
        || !marker["baseSha256"].as_str().is_some_and(hex)
    {
        return Err(held());
    }
    let identity: TransportIdentity =
        serde_json::from_value(input["identity"].clone()).map_err(|_| held())?;
    let context: RoleContext =
        serde_json::from_value(input["roleContext"].clone()).map_err(|_| held())?;
    if request.stage != identity.role.stage()
        || context.role != identity.role
        || context.episode != identity.episode
        || context.iteration_id.trim().is_empty()
        || input["transportProfile"] != crate::series_lite_transport::PROFILE
        || input["transportRule"] != crate::series_lite_transport::MODEL_RULE
        || !input["packetUtf8"].as_str().is_some_and(|raw| {
            raw.len() == identity.packet_bytes
                && crate::pipeline::digest(raw) == identity.packet_file_sha256
        })
        || request.input.get("modelInputProfile").is_some()
    {
        return Err(held());
    }
    Ok(())
}
fn bind(request: &TextRequest, expected: &TransportIdentity, context: &RoleContext) -> Result<()> {
    validate_request(request)?;
    if !enabled(request) {
        return Err(held());
    }
    let mut carrier = request.input.clone();
    carrier
        .as_object_mut()
        .ok_or_else(held)?
        .remove("textExact");
    // expected/context are supplied by the immutable controller ledger, never
    // derived from carrier.identity/roleContext to manufacture a matching pin.
    crate::series_lite_transport::restore(&carrier, expected, context)?;
    Ok(())
}
pub(super) fn wrap(bytes: &[u8]) -> Result<Value> {
    if bytes.is_empty() || bytes.len() > LIMIT {
        return Err(held());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| held())?;
    Ok(
        json!({"version":ENVELOPE,"rawText":text,"nativeTextSha256":crate::pipeline::digest(text),"byteLength":bytes.len()}),
    )
}
fn unwrap(envelope: &Value) -> Result<Vec<u8>> {
    let text = envelope["rawText"].as_str().ok_or_else(held)?;
    if wrap(text.as_bytes())? != *envelope {
        return Err(held());
    }
    Ok(text.as_bytes().to_vec())
}
pub(super) fn receive_evidence(envelope: &Value, request_hash: &str) -> Value {
    json!({"version":PROFILE,"requestHash":request_hash,"nativeTextSha256":envelope["nativeTextSha256"],"byteLength":envelope["byteLength"]})
}
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| held())?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || reparse
        || metadata.len() > 4_194_304
    {
        return Err(held());
    }
    let bytes = std::fs::read(path).map_err(|_| held())?;
    if bytes.len() > 4_194_304 {
        return Err(held());
    }
    Ok(bytes)
}
impl SavedTextExecutor {
    pub(super) async fn lite_execute_exact(
        &self,
        request: TextRequest,
        expected: &TransportIdentity,
        context: &RoleContext,
    ) -> Result<VerifiedRawText> {
        bind(&request, expected, context)?;
        if self.workflow_id.is_none()
            || !matches!(s(&self.settings, "textProvider"), "codex" | "codex_sdk")
        {
            return Err("LITE_EXACT_BACKEND_UNSUPPORTED: 需要持久工作流及 CLI/SDK".into());
        }
        // Completed historical text can be read under its original identity;
        // switching the current account cannot re-sign it or turn it into work.
        if let Some(raw) = self.lite_saved_exact(&request)? {
            return Ok(raw);
        }
        let envelope = self.execute(request.clone()).await?;
        let raw = self.lite_saved_exact(&request)?.ok_or_else(held)?;
        if raw.envelope != envelope {
            return Err(held());
        }
        Ok(raw)
    }
    pub(super) fn lite_exact_callback_bound(
        &self,
        request: &TextRequest,
        value: &Value,
    ) -> Result<()> {
        if !enabled(request) {
            return Ok(());
        }
        let raw = self.lite_saved_exact(request)?.ok_or_else(held)?;
        if raw.envelope != *value {
            return Err(held());
        }
        Ok(())
    }
    pub(super) fn lite_exact_cli_commit(
        &self,
        request: &TextRequest,
        value: &Value,
        reason: Option<&str>,
    ) -> Result<()> {
        let raw = self.lite_saved_exact(request)?.ok_or_else(held)?;
        if raw.envelope != *value {
            return Err(held());
        }
        let run = self.workflow_id.as_deref().ok_or_else(held)?;
        let mut c = self.store.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (state,receipt,error):(String,Option<String>,Option<String>)=tx.query_row("SELECT state,receipt,error FROM jobs WHERE id=?1 AND kind='text' AND provider='codex' AND result=?2 AND snapshot=?3",params![raw.job_id,value.to_string(),raw.snapshot],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|held())?;
        let next = if reason.is_some() {
            "rejected"
        } else {
            "completed"
        };
        let step_next = if reason.is_some() {
            "rejected"
        } else {
            "accepted"
        };
        let next_receipt = json!({"contract":"desktop-runner/v1","validation":if reason.is_some(){"rejected"}else{"accepted"}});
        if state == next
            && receipt
                .as_deref()
                .and_then(|v| serde_json::from_str::<Value>(v).ok())
                == Some(next_receipt.clone())
            && error.as_deref() == reason
        {
            return Ok(());
        }
        if state != "received" || receipt.is_some() || error.is_some() {
            return Err(held());
        }
        let event = json!({"requestHash":raw.request_sha256,"value":value}).to_string();
        let native = receive_evidence(value, &raw.request_sha256).to_string();
        let timestamp = now();
        let changed=tx.execute("UPDATE jobs SET state=?1,receipt=?2,error=?3,updated_at=?4 WHERE id=?5 AND state='received' AND result=?6 AND receipt IS NULL AND error IS NULL AND EXISTS(SELECT 1 FROM events WHERE id=(SELECT MAX(id) FROM events WHERE job_id=?5 AND type='text_received_raw') AND data=?7) AND EXISTS(SELECT 1 FROM events WHERE id=(SELECT MAX(id) FROM events WHERE job_id=?5 AND type='lite_exact_received') AND data=?8) AND snapshot=?9",params![next,next_receipt.to_string(),reason,timestamp,raw.job_id,value.to_string(),event,native,raw.snapshot]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err(held());
        }
        let changed=tx.execute("UPDATE workflow_steps SET state=?1,result=?2,error=?3,updated_at=?4 WHERE workflow_id=?5 AND node_key=?6 AND stage=?7 AND text_job_id=?8 AND request_hash=?9 AND state IN ('running','received')",params![step_next,value.to_string(),reason,timestamp,run,step_key(request),request.stage,raw.job_id,raw.request_sha256]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err(held());
        }
        tx.commit().map_err(|e| e.to_string())
    }
    fn lite_saved_exact(&self, request: &TextRequest) -> Result<Option<VerifiedRawText>> {
        validate_request(request)?;
        let run = self.workflow_id.as_deref().ok_or_else(held)?;
        let node = step_key(request);
        let record:Option<(String,String,String,String)>=self.store.conn()?.query_row(
            "SELECT w.text_job_id,w.request_hash,w.state,j.snapshot FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND w.stage=?3 AND j.kind='text'",
            params![run,node,request.stage],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
        let Some((job_id, hash, step_state, snapshot_text)) = record else {
            return Ok(None);
        };
        if uuid::Uuid::parse_str(&job_id).is_err() {
            return Err(held());
        }
        let snapshot: Value = serde_json::from_str(&snapshot_text).map_err(|_| held())?;
        let original: TextRequest = serde_json::from_value(snapshot.clone()).map_err(|_| held())?;
        if serde_json::to_value(&original).map_err(|_| held())?
            != serde_json::to_value(request).map_err(|_| held())?
            || crate::pipeline::digest(&snapshot_text) != hash
        {
            return Err(held());
        }
        let job = self.sdk_job(&job_id)?; // Strip Store's UI-only mediaIds projection.
        let provider = s(&job, "provider");
        if !matches!(provider, "codex" | "codex_sdk")
            || provider != s(&self.settings, "textProvider")
        {
            return Err(held());
        }
        if !["received", "completed", "rejected"].contains(&s(&job, "status")) {
            // UNKNOWN stays in the existing scheduler's observe-only branch.
            if !job["result"].is_null() {
                return Err(held());
            }
            return Ok(None);
        }
        if !["running", "received", "accepted", "rejected", "interrupted"]
            .contains(&step_state.as_str())
        {
            return Err(held());
        }
        let dir = self.store.root.join("text-runs").join(&job_id);
        let bytes = read_bounded(&dir.join("result.json"))?;
        let envelope = wrap(&bytes)?;
        let c = self.store.conn()?;
        let raw_event:String=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",[&job_id],|r|r.get(0)).map_err(|_|held())?;
        let native_event:String=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type='lite_exact_received' ORDER BY id DESC LIMIT 1",[&job_id],|r|r.get(0)).map_err(|_|held())?;
        if serde_json::from_str::<Value>(&raw_event).map_err(|_| held())?
            != json!({"requestHash":hash,"value":envelope})
            || serde_json::from_str::<Value>(&native_event).map_err(|_| held())?
                != receive_evidence(&envelope, &hash)
            || job["result"] != envelope
        {
            return Err(held());
        }
        if provider == "codex_sdk" {
            if self.sdk_raw(&job, &snapshot)? != envelope {
                return Err(held());
            }
        } else {
            self.lite_cli_original_dispatch(request, &job, &dir, &hash, run, &node)?;
        }
        // Re-read database evidence to reject a concurrent receive/accept rewrite.
        let after = self.sdk_job(&job_id)?;
        if after != job {
            return Err(held());
        }
        let still_bound:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2 AND stage=?3 AND text_job_id=?4 AND request_hash=?5 AND state=?6) AND ?7=(SELECT data FROM events WHERE job_id=?4 AND type='text_received_raw' ORDER BY id DESC LIMIT 1) AND ?8=(SELECT data FROM events WHERE job_id=?4 AND type='lite_exact_received' ORDER BY id DESC LIMIT 1)",params![run,node,request.stage,job_id,hash,step_state,raw_event,native_event],|r|r.get(0)).map_err(|_|held())?;
        if !still_bound {
            return Err(held());
        }
        Ok(Some(VerifiedRawText {
            sha256: envelope["nativeTextSha256"]
                .as_str()
                .ok_or_else(held)?
                .into(),
            bytes,
            job_id,
            request_sha256: hash,
            packet_sha256: s(&request.input["identity"], "packetFileSha256").into(),
            snapshot: snapshot_text,
            envelope,
        }))
    }
    fn lite_cli_original_dispatch(
        &self,
        request: &TextRequest,
        job: &Value,
        dir: &Path,
        hash: &str,
        run: &str,
        node: &str,
    ) -> Result<()> {
        let metadata_bytes = read_bounded(&dir.join("cli-dispatch.json"))?;
        let metadata: Value = serde_json::from_slice(&metadata_bytes).map_err(|_| held())?;
        let compiled = compile_codex_input(request)?;
        let prompt = match metadata["format"].as_str() {
            Some(CLI_RULES_FIRST_FORMAT) => rules_first_cli_prompt(
                &serde_json::from_str(&compiled.prompt).map_err(|_| held())?,
            )?,
            Some(CLI_LEGACY_FORMAT) => compiled.prompt,
            _ => return Err(held()),
        };
        let allocation = CliDispatchAllocation {
            job: s(job, "id").into(),
            request_hash: hash.into(),
            workflow: Some(run.into()),
            node: node.into(),
            retry_origin: None,
        };
        // Initial exact seam deliberately has no automatic transport retry.
        if metadata
            != cli_dispatch_metadata(
                &allocation,
                s(&metadata, "format"),
                &prompt,
                &compiled.schema,
            )
            || metadata_bytes != metadata.to_string().as_bytes()
            || read_bounded(&dir.join("cli-prompt.txt"))? != prompt.as_bytes()
            || read_bounded(&dir.join("schema.json"))? != compiled.schema.as_bytes()
        {
            return Err(held());
        }
        let event:String=self.store.conn()?.query_row("SELECT data FROM events WHERE job_id=?1 AND type='cli_dispatch_frozen' ORDER BY id DESC LIMIT 1",[s(job,"id")],|r|r.get(0)).map_err(|_|held())?;
        if event != metadata.to_string() {
            return Err(held());
        }
        match s(job, "status") {
            "received" if job["receipt"].is_null() && job["error"].is_null() => {}
            "completed"
                if job["receipt"]["contract"] == "desktop-runner/v1"
                    && job["receipt"]["validation"] == "accepted"
                    && job["receipt"]["migrationPolicy"].is_null()
                    && job["error"].is_null() => {}
            "rejected"
                if job["receipt"]["contract"] == "desktop-runner/v1"
                    && job["receipt"]["validation"] == "rejected"
                    && job["error"].as_str().is_some() => {}
            _ => return Err(held()),
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "text_executor_lite_exact_tests.rs"]
mod tests;
