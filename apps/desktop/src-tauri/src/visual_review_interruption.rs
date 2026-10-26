//! Explain the bound SDK interruption before a missing runtime receipt hides it.
//! This module never settles UNKNOWN and cannot dispatch or accept a result.
use super::*;
use sha2::Digest;

fn diagnostic(event: &Value) -> bool {
    let item = &event["item"];
    event["type"] == "item.completed"
        && event.as_object().is_some_and(|v| v.len() == 2)
        && item.as_object().is_some_and(|v| v.len() == 3)
        && item["type"] == "error"
        && item["id"].as_str().is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        })
        && item["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty() && m.len() <= 4096)
}
pub(super) fn reason(
    plan: &Value,
    result: &Value,
    manifest_sha: &str,
    bytes: &[u8],
) -> Result<String> {
    let sha = format!("{:x}", sha2::Sha256::digest(bytes));
    if bytes.len() > 8 * 1024 * 1024
        || result["version"] != "visual-sdk-receipt/v1"
        || result["state"] != "submission_unknown"
        || result["attemptId"] != plan["attemptId"]
        || result["manifestSha256"] != manifest_sha
        || result["eventsSha256"] != sha
    {
        return Err("VISUAL_INTERRUPTION_EVIDENCE_CHANGED: 原中断回执与事件不匹配".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "VISUAL_EVENT_ENCODING")?;
    let events: Vec<Value> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| "VISUAL_EVENT_ENCODING")?;
    if result["eventCount"] != events.len() {
        return Err("VISUAL_INTERRUPTION_EVENT_COUNT".into());
    }
    if result["error"] == "VISUAL_TOOL_BLOCKED"
        && events.len() == 2
        && events[0]["type"] == "thread.started"
        && events[0]["thread_id"] == result["threadId"]
        && diagnostic(&events[1])
    {
        return Ok("旧视觉组件将运行器启动提示误判为禁用工具事件（VISUAL_TOOL_BLOCKED），导致原 SDK 流中断。尚未取得完整结果；原记录保留，不重复提交。".into());
    }
    let code = s(result, "error").chars().take(256).collect::<String>();
    Ok(format!(
        "原 SDK 流中断（{}）；尚未取得完整结果，保留原记录，不重复提交。",
        if code.is_empty() {
            "原因未完整返回"
        } else {
            &code
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, Value, Vec<u8>) {
        let plan = json!({"attemptId":"attempt"});
        let events = [
            json!({"type":"thread.started","thread_id":"thread"}),
            json!({"type":"item.completed","item":{"id":"item_0","type":"error","message":"Under-development features enabled: skip_host_skill_discovery."}}),
        ];
        let raw = events
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>()
            .into_bytes();
        let result = json!({"version":"visual-sdk-receipt/v1","state":"submission_unknown","attemptId":"attempt","manifestSha256":"manifest","threadId":"thread","eventCount":2,"eventsSha256":format!("{:x}",sha2::Sha256::digest(&raw)),"error":"VISUAL_TOOL_BLOCKED"});
        (plan, result, raw)
    }
    #[test]
    fn interrupted_startup_diagnostic_has_real_reason_without_requiring_missing_runtime_receipt() {
        let (plan, result, raw) = fixture();
        let message = reason(&plan, &result, "manifest", &raw).unwrap();
        assert!(message.contains("启动提示误判"));
        assert!(message.contains("不重复提交"));
        let mut tls = result.clone();
        tls["error"] = json!("tls handshake eof");
        let message = reason(&plan, &tls, "manifest", &raw).unwrap();
        assert!(message.contains("tls handshake eof"));
        assert!(!message.contains("启动提示误判"));
        for field in ["eventsSha256", "attemptId", "manifestSha256", "version"] {
            let mut changed = result.clone();
            changed[field] = json!("changed");
            assert!(reason(&plan, &changed, "manifest", &raw).is_err());
        }
    }
    #[test]
    fn public_old_runner_hash_preserves_preparation_proofs_without_permitting_unknown_code() {
        assert!(preparation_runner_known(&json!(
            LEGACY_DIAGNOSTIC_RUNNER_SHA
        )));
        assert!(preparation_runner_known(&json!(digest(RUNNER))));
        assert!(!preparation_runner_known(&json!("0".repeat(64))));
    }
}
