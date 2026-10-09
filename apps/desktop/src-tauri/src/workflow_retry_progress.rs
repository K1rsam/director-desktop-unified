//! Read-only, content-free progress projection. Never drives execution or retries.
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::sync::OnceLock;

fn reconnect_counts(event: &Value) -> Option<(u64, u64)> {
    let object = event.as_object()?;
    if object.len() != 2 || event["type"] != "error" { return None; }
    let message = event["message"].as_str()?;
    if message.len() > 4096 { return None; }
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    // Match the exact bounded diagnostic accepted by sdk-bridge/src/bridge.mjs.
    let captures = PATTERN.get_or_init(|| regex::Regex::new(
        r"\AReconnecting\.\.\. ([1-9][0-9]?)/([1-9][0-9]?) \(stream disconnected before completion: [^\r\n]+\)\z"
    ).expect("constant reconnect expression")).captures(message)?;
    let attempt: u64 = captures[1].parse().ok()?;
    let limit: u64 = captures[2].parse().ok()?;
    (attempt <= limit && limit <= 10).then_some((attempt, limit))
}

pub(super) fn original_stream_reconnect(c: &Connection, job: &str, provider: &str) -> Option<Value> {
    let (kind, raw_path) = match provider {
        "codex" => ("codex_event", "$"),
        "codex_sdk" => ("sdk_event", "$.rawEvent"),
        _ => return None,
    };
    // Do not let a historical error overshadow any newer execution event.
    let query = format!("SELECT rowid, CASE WHEN length(CAST(data AS BLOB))<=8192 THEN json_extract(data,'{raw_path}') END FROM events WHERE job_id=?1 AND type=?2 ORDER BY rowid DESC LIMIT 1");
    let (latest, raw): (i64, Option<String>) = c.query_row(&query, params![job, kind], |r| Ok((r.get(0)?, r.get(1)?))).ok()?;
    let event: Value = serde_json::from_str(raw.as_deref()?).ok()?;
    let (attempt, limit) = reconnect_counts(&event)?;
    // Require the original stream's one thread and one active turn. Only bounded
    // reconnect notices may intervene; completion, failure and unknown errors
    // never become a recoverable state. The cap fails closed on excessive history.
    let query = format!("SELECT CASE WHEN length(CAST(data AS BLOB))<=8192 THEN json_extract(data,'{raw_path}') END FROM events WHERE job_id=?1 AND type=?2 AND rowid<=?3 AND json_extract(data,'{raw_path}.type') IN ('thread.started','turn.started','turn.completed','turn.failed','error') ORDER BY rowid LIMIT 65");
    let mut statement = c.prepare(&query).ok()?;
    let rows = statement.query_map(params![job, kind, latest], |r| r.get::<_, Option<String>>(0)).ok()?;
    let mut thread = false;
    let mut turn = false;
    let mut count = 0;
    for row in rows {
        count += 1;
        if count > 64 { return None; }
        let parsed: Value = serde_json::from_str(row.ok()?.as_deref()?).ok()?;
        match parsed["type"].as_str()? {
            "thread.started" if !thread && !turn && parsed["thread_id"].is_string() => thread = true,
            "turn.started" if thread && !turn => turn = true,
            "error" if thread && turn && reconnect_counts(&parsed).is_some() => {},
            _ => return None,
        }
    }
    (thread && turn).then(|| json!({"version":"stream-reconnect-progress/v1","attempt":attempt,"limit":limit}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(message: &str) -> Value { json!({"type":"error","message":message}) }
    fn diagnostic() -> Value { event("Reconnecting... 2/5 (stream disconnected before completion: tls handshake eof)") }
    fn database(kind: &str, values: &[Value]) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE events(job_id TEXT,type TEXT,data TEXT)").unwrap();
        for value in values {
            let data = if kind == "sdk_event" { json!({"rawEvent":value,"privatePath":"never disclose"}) } else { value.clone() };
            c.execute("INSERT INTO events VALUES('original',?1,?2)", params![kind, data.to_string()]).unwrap();
        }
        c
    }
    fn active() -> Vec<Value> { vec![json!({"type":"thread.started","thread_id":"original-thread"}),json!({"type":"turn.started"}),diagnostic()] }

    #[test]
    fn reconnect_projection_uses_exact_bridge_diagnostic_grammar() {
        assert_eq!(reconnect_counts(&diagnostic()), Some((2,5)));
        for message in ["Reconnecting... 0/5 (stream disconnected before completion: x)","Reconnecting... 6/5 (stream disconnected before completion: x)","Reconnecting... 1/11 (stream disconnected before completion: x)","Reconnecting... 01/5 (stream disconnected before completion: x)","Reconnecting... 1/5 (stream disconnected before completion: x)\n","tls handshake eof","Reconnecting... 1/5 (different error: x)"] {
            assert_eq!(reconnect_counts(&event(message)), None);
        }
        let mut changed=diagnostic(); changed["private"] = json!(true); assert_eq!(reconnect_counts(&changed),None);
        changed=diagnostic();changed["type"]=json!("turn.failed");assert_eq!(reconnect_counts(&changed),None);
        assert_eq!(reconnect_counts(&event(&format!("Reconnecting... 1/5 (stream disconnected before completion: {})","x".repeat(4096)))),None);
    }

    #[test]
    fn reconnect_projection_is_read_only_and_bound_to_one_active_original_stream() {
        for (provider,kind) in [("codex","codex_event"),("codex_sdk","sdk_event")] {
            let c=database(kind,&active()); c.execute_batch("PRAGMA query_only=ON").unwrap();
            let projected=original_stream_reconnect(&c,"original",provider).unwrap();
            assert_eq!(projected,json!({"version":"stream-reconnect-progress/v1","attempt":2,"limit":5}));
            assert!(original_stream_reconnect(&c,"unrelated",provider).is_none());
            let result=projected.to_string(); assert!(!result.contains("tls")&&!result.contains("private")&&!result.contains("original-thread"));
            assert_eq!(c.query_row("SELECT COUNT(*) FROM events",[],|r|r.get::<_,i64>(0)).unwrap(),3);
            for before in [vec![],vec![json!({"type":"thread.started","thread_id":"t"})],vec![json!({"type":"turn.started"})]] {
                let mut values=before;values.push(diagnostic());assert!(original_stream_reconnect(&database(kind,&values),"original",provider).is_none());
            }
            for end in [json!({"type":"turn.failed","error":{"message":"real failure"}}),event("real unknown error"),json!({"type":"turn.completed","usage":{}}),json!({"type":"item.completed","item":{"type":"agent_message","text":"private"}})] {
                let mut values=active();values.push(end);assert!(original_stream_reconnect(&database(kind,&values),"original",provider).is_none());
            }
            let mut values=active();values.push(event("real error"));values.push(diagnostic());assert!(original_stream_reconnect(&database(kind,&values),"original",provider).is_none());
            let mut values=active();values.push(json!({"type":"turn.failed"}));values.push(diagnostic());assert!(original_stream_reconnect(&database(kind,&values),"original",provider).is_none());
        }
    }
}
