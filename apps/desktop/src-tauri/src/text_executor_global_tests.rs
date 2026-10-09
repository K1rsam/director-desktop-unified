//! Local loopback only; real API execution code, synthetic credentials.
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

#[test]
fn api_executor_uses_frozen_global_key_after_rotation_and_preserves_legacy_key() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let store = Store::open(root.path().join("project")).unwrap();
    let pid = crate::store::id();
    store.create("api fixture", &pid).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut captured = Vec::new();
        while captured.len() < 2 && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                assert!(bytes.len() < 65536);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse().ok())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            captured.push(String::from_utf8(bytes).unwrap());
            let body = r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}],"usage":{}}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
        captured
    });
    crate::app_settings::initialize(&home, &json!([]), None).unwrap();
    let first=crate::app_settings::save(&home,&json!({"textProvider":"api","textEndpoint":endpoint,"textModel":"frozen-model","textApiKey":"frozen-global-token"}),1).unwrap();
    let mut frozen = crate::app_settings::effective_from(&first, &json!({}));
    crate::app_settings::freeze_credentials(&mut frozen);
    crate::app_settings::save(&home,&json!({"textEndpoint":"http://127.0.0.1:9","textModel":"new-model","textApiKey":"current-global-token"}),2).unwrap();
    crate::platform::write_credential(&format!("{pid}:text"), "legacy-token").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (name, settings) in [
        ("frozen", frozen),
        (
            "legacy",
            json!({"textEndpoint":endpoint,"textModel":"legacy-model"}),
        ),
    ] {
        let job = store
            .add_job("text", name, "api", "local", &json!({}), "running")
            .unwrap();
        let request = TextRequest {
            stage: "fixture".into(),
            skill: "fixture".into(),
            model: s(&settings, "textModel").into(),
            reasoning: "medium".into(),
            input: json!({"input":"synthetic"}),
            schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),
            rule_snapshot: json!({}),
        };
        let executor = SavedTextExecutor {
            store: store.clone(),
            settings,
            workflow_id: None,
        };
        assert_eq!(
            runtime.block_on(executor.api(&request, &job)).unwrap(),
            json!({"ok":true})
        );
    }
    let captured = server.join().unwrap();
    assert_eq!(captured.len(), 2);
    assert!(captured[0].contains("Bearer frozen-global-token"));
    assert!(captured[0].contains("frozen-model"));
    assert!(captured[1].contains("Bearer legacy-token"));
    assert!(captured[1].contains("legacy-model"));
    assert!(captured.iter().all(|s| !s.contains("current-global-token")));
}
