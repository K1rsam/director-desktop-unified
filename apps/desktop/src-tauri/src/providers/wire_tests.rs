//! Real HTTP client against a local fixture, with no upstream credentials or paid requests.
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

pub(super) struct Fixture {
    pub(super) endpoint: String,
    pub(super) requests: Arc<Mutex<Vec<(String, String)>>>,
    pub(super) thread: thread::JoinHandle<()>,
}
pub(super) fn fixture(replies: Vec<Value>) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let saved = requests.clone();
    let thread = thread::spawn(move || {
        let mut recovery_alias=String::new();
        for mut reply in replies {
            // A changed request sequence must fail the fixture instead of
            // blocking thread.join forever waiting for an unused response.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline,
                            "fixture expected another request; received {:?}", saved.lock().unwrap());
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            // Winsock may inherit the listener's nonblocking mode.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            let (header_end, length) = loop {
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_string();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while bytes.len() < header_end + length {
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let line = headers.lines().next().unwrap().to_string();
            let body = String::from_utf8_lossy(&bytes[header_end..header_end + length]).to_string();
            if line.starts_with("POST /browser/tabs/open "){
                recovery_alias=serde_json::from_str::<Value>(&body).ok().and_then(|v|v["launchToken"].as_str().map(String::from)).unwrap_or_default();
            }
            saved.lock().unwrap().push((line, body));
            if reply["__echoRecoveryAlias"]==true{
                reply.as_object_mut().unwrap().remove("__echoRecoveryAlias");
                reply=serde_json::from_str(&reply.to_string().replace("__RECOVERY_ALIAS__",&recovery_alias)).unwrap();
            }
            let status=reply.get("__httpStatus").and_then(Value::as_u64).unwrap_or(200);
            if let Some(object)=reply.as_object_mut() {object.remove("__httpStatus");}
            let body = if reply == json!("invalid-json") {
                b"not json".to_vec()
            } else {
                serde_json::to_vec(&reply).unwrap()
            };
            write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        }
    });
    Fixture {
        endpoint,
        requests,
        thread,
    }
}
fn catalog() -> Value {
    json!({"ok":true,"catalog_version":"fixture-v1","count":1,"models":[{"id":"bh2.0-720p","status":"active","kind":"video","billing":"per_second","unit":"CNY","price":{"amount":0.5},"durations":{"min":4,"max":15},"allowed_sizes":["16:9"],"resolutions":["720P"],"reference":{"images":9,"videos":3,"audios":3}}]})
}
fn config(endpoint: String) -> ProviderConfig {
    let env = format!("DIRECTOR_FIXTURE_{}", uuid::Uuid::new_v4().simple());
    std::env::set_var(&env, "test-only-not-a-real-key");
    ProviderConfig {
        kind: "bh".into(),
        endpoint: Some(endpoint),
        credential_env: Some(env),
        cli_path: None,
        bridge_token_env: None,
        chrome_binding: None,
        timeout_seconds: 5,
        adapted_models: vec![],
    }
}
fn request() -> GenerationRequest {
    GenerationRequest {
        request_id: "local-1".into(),
        kind: "video".into(),
        model: "bh2.0-720p".into(),
        prompt: "人物挥手，图片1保持角色外形".into(),
        duration: Some(5),
        ratio: "16:9".into(),
        resolution: "720P".into(),
        references: vec![],
        options: json!({}),
    }
}

#[test]
fn chrome_wire_detail_and_safe_code_survive_500_without_exposing_credentials_or_objects() {
    for (body,expected) in [(json!({"detail":"Client has unreleased standalone command","code":"STANDALONE_COMMAND_BUSY"}),Some("Client has unreleased standalone command")),
        (json!({"detail":"Bearer synthetic-private-token"}),None),(json!({"detail":"error https://fixture.invalid/?token=hidden"}),None),
        (json!({"detail":{"token":"hidden"}}),None),(json!({"detail":"authorization=hidden"}),None)] {
        assert_eq!(wire::safe_message(&body).as_deref(),expected);
    }
    assert_eq!(wire::safe_code(&json!({"code":"STANDALONE_COMMAND_BUSY"})).as_deref(),Some("STANDALONE_COMMAND_BUSY"));
    assert!(wire::safe_code(&json!({"code":"Bearer private"})).is_none());
    let f=fixture(vec![json!({"__httpStatus":500,"detail":"Browser handshake timed out","code":"TAB_HANDSHAKE_TIMEOUT"})]);
    let response=wire::client(5).unwrap().get(f.endpoint.clone()).send().unwrap();
    let error=wire::read_json(response,false).unwrap_err();f.thread.join().unwrap();
    assert_eq!(error.code,"TAB_HANDSHAKE_TIMEOUT");assert_eq!(error.message,"Browser handshake timed out");assert_eq!(error.context.unwrap()["httpStatus"],500);
}

#[test]
fn bh_upload_submit_and_resume_queries_only_original_id() {
    let url="https://fixture.oss-cn-shenzhen.aliyuncs.com/api-assets/image.png?x-oss-signature=complete-signature&x-oss-date=20261004";
    let fixture = fixture(vec![
        catalog(),
        json!({"url":url,"kind":"image"}),
        catalog(),
        json!({"task_id":"task-original","status":"processing"}),
        json!({"task_id":"task-original","status":"processing","progress":50}),
        json!({"task_id":"task-original","status":"succeeded","result_url":"https://example.test/result.mp4?signature=keep"}),
    ]);
    let config = config(fixture.endpoint.clone());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("reference.png");
    let bytes = b"\x89PNG\r\n\x1a\nfixture";
    fs::write(&path, bytes).unwrap();
    let mut input = request();
    input.references.push(MediaReference {
        media_id: "image-1".into(),
        kind: "image".into(),
        path,
        sha256: hex_hash(bytes),
        order: 0,
        usage: "reference".into(),
        duration_seconds: None,
    });
    let prepared = prepare(&config, &input).unwrap();
    assert_eq!(prepared.upload_receipts[0].url.as_deref(), Some(url));
    assert!(!prepared.upload_receipts[0].upstream_read_confirmed);
    assert_eq!(prepared.estimated_cost, Some(2.5));
    let saved: PreparedRequest =
        serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    let receipt = submit(&config, &saved).unwrap();
    assert_eq!(receipt.task_id, "task-original");
    assert_eq!(
        query_prepared(&config, &saved, &receipt.task_id)
            .unwrap()
            .status,
        "processing"
    );
    assert_eq!(
        query_prepared(&config, &saved, &receipt.task_id)
            .unwrap()
            .result_urls,
        vec!["https://example.test/result.mp4?signature=keep"]
    );
    fixture.thread.join().unwrap();
    let rows = fixture.requests.lock().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|(route, _)| route.starts_with("POST /v1/video/generations "))
            .count(),
        1
    );
    let submit_body: Value = serde_json::from_str(
        &rows
            .iter()
            .find(|(route, _)| route.starts_with("POST /v1/video/generations "))
            .unwrap()
            .1,
    )
    .unwrap();
    assert_eq!(submit_body["images"][0], url);
    assert!(submit_body.get("reference_videos").is_some());
    assert!(submit_body.get("reference_audios").is_some());
    std::env::remove_var(config.credential_env.unwrap());
}

#[test]
fn malformed_submit_is_unknown_without_automatic_resubmit() {
    let fixture = fixture(vec![catalog(), catalog(), json!("invalid-json")]);
    let config = config(fixture.endpoint.clone());
    let prepared = prepare(&config, &request()).unwrap();
    let err = submit(&config, &prepared).unwrap_err();
    assert!(err.submission_unknown);
    fixture.thread.join().unwrap();
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(route, _)| route.starts_with("POST /v1/video/generations "))
            .count(),
        1
    );
    std::env::remove_var(config.credential_env.unwrap());
}

#[test]
fn media_change_prevents_any_submit() {
    let config = config("http://127.0.0.1:1/v1".into());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("image.png");
    fs::write(&path, b"changed").unwrap();
    let mut input = request();
    input.references.push(MediaReference {
        media_id: "i".into(),
        kind: "image".into(),
        path,
        sha256: hex_hash(b"original"),
        order: 0,
        usage: "".into(),
        duration_seconds: None,
    });
    assert_eq!(prepare(&config, &input).unwrap_err().code, "MEDIA_CHANGED");
    std::env::remove_var(config.credential_env.unwrap());
}

#[test]
fn no_unknown_submission_recovery_generates() {
    let config = config("http://127.0.0.1:1/v1".into());
    assert_eq!(
        reconcile_unknown(&config, "local-1").unwrap_err().code,
        "MANUAL_RECONCILIATION_REQUIRED"
    );
    std::env::remove_var(config.credential_env.unwrap());
}
