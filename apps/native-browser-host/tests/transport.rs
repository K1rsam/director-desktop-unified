use base64::Engine;
use director_native_browser_host::{read_frame, write_frame, Config, CHUNK_BYTES, MAX_FRAME};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

const ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop/";
const HTTP_ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
const TOKEN: &str = "synthetic-test-only-not-a-real-credential";
const WAIT: Duration = Duration::from_secs(6);
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn config(port: u16) -> Config {
    Config {
        schema_version: "director-native-host-config/v1".into(),
        allowed_origins: vec![ORIGIN.into()],
        server_url: format!("http://127.0.0.1:{port}"),
    }
}

struct Host {
    child: Child,
    input: Option<ChildStdin>,
    received: Receiver<Value>,
    directory: PathBuf,
}

impl Host {
    fn new(port: u16) -> Self {
        Self::with_arguments(port, &[ORIGIN, "--parent-window=0"])
    }
    fn with_arguments(port: u16, arguments: &[&str]) -> Self {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-host-fixtures");
        let directory = base.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let binary = directory.join(if cfg!(windows) { "host.exe" } else { "host" });
        std::fs::copy(env!("CARGO_BIN_EXE_director-native-browser-host"), &binary).unwrap();
        std::fs::write(
            directory.join("native-host-config.json"),
            serde_json::to_vec(&config(port)).unwrap(),
        )
        .unwrap();
        let mut command = Command::new(binary);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW; preserves redirected pipes.
        }
        let mut child = command.spawn().unwrap();
        let input = child.stdin.take();
        let mut stdout = child.stdout.take().unwrap();
        let (sender, received) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(Some(value)) = read_frame(&mut stdout) {
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            received,
            directory,
        }
    }
    fn ready(&self) {
        assert_eq!(
            self.next(),
            json!({"type":"ready","version":"director-native-transport/v1"})
        );
    }
    fn next(&self) -> Value {
        self.received
            .recv_timeout(WAIT)
            .expect("native response deadline")
    }
    fn send(&mut self, value: Value) {
        write_frame(self.input.as_mut().unwrap(), &value).unwrap();
    }
    fn open(&mut self, id: &str, port: u16) {
        self.send(json!({"type":"open","channelId":id,"serverUrl":format!("http://127.0.0.1:{port}"),"token":TOKEN}));
    }
    fn finish(&mut self, success: bool) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.success(), success, "unexpected exit status {status}");
                let mut diagnostics = Vec::new();
                self.child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_end(&mut diagnostics)
                    .unwrap();
                assert!(diagnostics.is_empty(), "host emitted stderr");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "host failed to exit after stdin EOF"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        drop(self.input.take());
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-host-fixtures");
        assert!(self.directory.starts_with(base));
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}
fn accept(listener: &TcpListener) -> WebSocket<TcpStream> {
    let deadline = Instant::now() + WAIT;
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "websocket accept deadline");
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept failed {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    stream.set_write_timeout(Some(WAIT)).unwrap();
    tungstenite::accept_hdr(
        stream,
        |request: &tungstenite::handshake::server::Request,
         response: tungstenite::handshake::server::Response| {
            // Mirror the existing bridge's exact Origin equality policy. The
            // Native Messaging argument/config remain the slash-suffixed form.
            assert_eq!(request.headers().get("origin").unwrap(), HTTP_ORIGIN);
            let url = url::Url::parse(&format!("http://127.0.0.1{}", request.uri())).unwrap();
            assert_eq!(url.path(), "/extension/ws");
            assert_eq!(
                url.query_pairs().collect::<Vec<_>>(),
                vec![
                    ("token".into(), TOKEN.into()),
                    ("runtime".into(), "extension".into())
                ]
            );
            Ok(response)
        },
    )
    .unwrap()
}

#[test]
fn frames_roundtrip_unicode_and_fragmented_input() {
    let original = json!({"type":"send","channelId":"中文","data":"资产🦉\n\"雪\""});
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &original).unwrap();
    struct Bytewise(Cursor<Vec<u8>>);
    impl Read for Bytewise {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            let len = bytes.len().min(1);
            self.0.read(&mut bytes[..len])
        }
    }
    let mut reader = Bytewise(Cursor::new(bytes));
    assert_eq!(read_frame(&mut reader), Ok(Some(original)));
    assert_eq!(read_frame(&mut reader), Ok(None));
}

#[test]
fn frames_reject_partial_header_body_invalid_utf8_and_limit_before_allocation() {
    assert_eq!(
        read_frame(&mut Cursor::new(vec![1, 0])),
        Err("truncated_frame")
    );
    assert_eq!(
        read_frame(&mut Cursor::new(vec![2, 0, 0, 0, b'{'])),
        Err("truncated_frame")
    );
    assert_eq!(
        read_frame(&mut Cursor::new(vec![1, 0, 0, 0, 0xff])),
        Err("invalid_json")
    );
    assert_eq!(
        read_frame(&mut Cursor::new(((MAX_FRAME + 1) as u32).to_le_bytes())),
        Err("frame_limit")
    );
    assert_eq!(
        read_frame(&mut Cursor::new([0, 0, 0, 0])),
        Err("frame_limit")
    );
}

#[test]
fn origins_and_arguments_are_exact_and_fail_closed() {
    let configuration = config(53179);
    for bad in [
        "chrome-extension://abcdefghijklmnopabcdefghijklmnop",
        "chrome-extension://ABCDEFGHIJKLMNOPabcdefghijklmnop/",
        "https://chatgpt.com/",
        "chrome-extension://abcdefghijklmnopabcdefghijklmnop/x",
    ] {
        assert!(configuration.validate_arguments(&[bad.into()]).is_err());
    }
    for bad in [
        "--parent-window=",
        "--parent-window=-1",
        "--parent-window=1x",
        "--server-url=http://evil",
    ] {
        assert!(configuration
            .validate_arguments(&[ORIGIN.into(), bad.into()])
            .is_err());
    }
    assert!(configuration.validate_arguments(&[]).is_err());
    assert!(configuration
        .validate_arguments(&[ORIGIN.into(), "--parent-window=123".into()])
        .is_ok());
    let mut host = Host::with_arguments(
        53179,
        &["chrome-extension://pppppppppppppppppppppppppppppppp/"],
    );
    host.finish(false);
    assert!(host.received.try_recv().is_err());
}

#[test]
fn server_config_rejects_external_and_normalized_endpoint_spellings() {
    for bad in [
        "https://127.0.0.1:53179",
        "http://localhost:53179",
        "http://127.1:53179",
        "http://2130706433:53179",
        "http://0x7f000001:53179",
        "http://[::1]:53179",
        "http://127.0.0.1:53179/",
        "http://127.0.0.1:53179/x",
        "http://a@127.0.0.1:53179",
        "http://127.0.0.1:53179?token=secret",
        "http://127.0.0.1:53179#x",
        "http://127.0.0.1:53179/../",
    ] {
        let mut configuration = config(53179);
        configuration.server_url = bad.into();
        assert!(configuration.validate().is_err(), "accepted {bad}");
    }
    assert!(config(53179).validate().is_ok());
}

#[test]
fn hidden_windows_binary_stdio_ping_and_clean_eof() {
    let mut host = Host::new(53179);
    host.ready();
    host.send(json!({"type":"ping","nonce":"验证🦉"}));
    assert_eq!(host.next(), json!({"type":"pong","nonce":"验证🦉"}));
    host.finish(true);
}

#[test]
fn native_origin_argument_stays_exact_while_http_origin_has_no_trailing_slash() {
    let configuration = config(53179);
    assert!(configuration.validate_arguments(&[ORIGIN.into()]).is_ok());
    assert!(configuration
        .validate_arguments(&[HTTP_ORIGIN.into()])
        .is_err());
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        // accept() rejects any Origin other than the exact no-slash string.
        let mut socket = accept(&listener);
        let message = socket.read().unwrap();
        assert_eq!(message.to_text().unwrap(), "strict-origin-roundtrip");
        socket.send(message).unwrap();
        loop {
            match socket.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("origin-contract", port);
    assert_eq!(host.next()["type"], "open");
    host.send(
        json!({"type":"send","channelId":"origin-contract","data":"strict-origin-roundtrip"}),
    );
    assert_eq!(host.next()["data"], "strict-origin-roundtrip");
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn requested_endpoint_must_match_config_and_errors_do_not_leak_token() {
    let mut host = Host::new(53179);
    host.ready();
    for endpoint in ["http://127.0.0.1:53180", "http://example.com:53179"] {
        host.send(json!({"type":"open","channelId":"reject","serverUrl":endpoint,"token":TOKEN}));
        assert_eq!(
            host.next(),
            json!({"type":"error","channelId":"reject","code":"server_rejected"})
        );
    }
    host.send(json!({"type":"open","channelId":"reject","serverUrl":"http://127.0.0.1:53179","token":"contains\ncontrol"}));
    assert_eq!(host.next()["code"], "token_rejected");
    host.finish(true);
}

#[test]
fn three_real_loopback_channels_roundtrip_fifo_and_independent_close() {
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        let mut workers = Vec::new();
        for _ in 0..3 {
            let mut socket = accept(&listener);
            workers.push(thread::spawn(move || {
                for expected in 0..5 {
                    let message = socket.read().unwrap();
                    assert_eq!(
                        message.to_text().unwrap().split(':').next().unwrap(),
                        expected.to_string()
                    );
                    socket.send(message).unwrap();
                }
                loop {
                    match socket.read() {
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let mut host = Host::new(port);
    host.ready();
    for id in ["one", "two", "three"] {
        host.open(id, port);
        assert_eq!(host.next()["type"], "open");
    }
    for index in 0..5 {
        for id in ["one", "two", "three"] {
            host.send(json!({"type":"send","channelId":id,"data":format!("{index}:资产🦉:{id}")}));
        }
    }
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for _ in 0..15 {
        let message = host.next();
        assert_eq!(message["type"], "message");
        let id = message["channelId"].as_str().unwrap();
        let expected = seen.entry(id.to_string()).or_default();
        assert_eq!(message["data"], format!("{expected}:资产🦉:{id}"));
        *expected += 1;
    }
    for id in ["one", "two", "three"] {
        host.send(json!({"type":"close","channelId":id}));
        let close = host.next();
        assert_eq!(close["type"], "close");
        assert_eq!(close["channelId"], id);
    }
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn unicode_chunks_preserve_original_bytes_hash_and_order() {
    let (listener, port) = listener();
    // First boundary deliberately cuts inside a 4-byte UTF-8 code point.
    let text = format!("{}{}", "x".repeat(CHUNK_BYTES - 1), "🦉雪".repeat(120000));
    let expected = text.as_bytes().to_vec();
    let expected_hash = format!("{:x}", Sha256::digest(&expected));
    let server = thread::spawn(move || {
        let mut socket = accept(&listener);
        socket.send(Message::Text(text.into())).unwrap();
        loop {
            match socket.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("large", port);
    assert_eq!(host.next()["type"], "open");
    let mut bytes = Vec::new();
    let mut index = 0;
    let mut transfer = None;
    loop {
        let chunk = host.next();
        assert_eq!(chunk["type"], "chunk");
        assert_eq!(chunk["index"], index);
        assert_eq!(chunk["byteLength"], expected.len());
        assert_eq!(chunk["sha256"], expected_hash);
        if transfer.is_none() {
            transfer = Some(chunk["transferId"].clone());
        }
        assert_eq!(chunk["transferId"], transfer.as_ref().unwrap().clone());
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(chunk["dataBase64"].as_str().unwrap())
            .unwrap();
        assert!(decoded.len() <= CHUNK_BYTES);
        bytes.extend(decoded);
        index += 1;
        if index == chunk["count"].as_u64().unwrap() {
            break;
        }
    }
    assert_eq!(bytes, expected);
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn remote_disconnect_does_not_reconnect_or_resend() {
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        let mut socket = accept(&listener);
        assert_eq!(socket.read().unwrap().to_text().unwrap(), "single-submit");
        socket
            .close(Some(tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                reason: "secret-provider-cause-never-echo".into(),
            }))
            .unwrap();
        drop(socket);
        let until = Instant::now() + Duration::from_millis(600);
        while Instant::now() < until {
            assert!(listener.accept().is_err(), "host auto-reconnected");
            thread::sleep(Duration::from_millis(5));
        }
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("once", port);
    assert_eq!(host.next()["type"], "open");
    host.send(json!({"type":"send","channelId":"once","data":"single-submit"}));
    let close = host.next();
    assert_eq!(close["reason"], "remote_close");
    assert!(!close.to_string().contains("secret-provider"));
    thread::sleep(Duration::from_millis(100));
    host.send(json!({"type":"send","channelId":"once","data":"must-not-resend"}));
    assert_eq!(host.next()["code"], "channel_not_open");
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn eof_closes_live_socket_workers_without_browser_or_process_kill() {
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        let mut socket = accept(&listener);
        assert!(matches!(socket.read(), Ok(Message::Close(_)) | Err(_)));
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("eof", port);
    assert_eq!(host.next()["type"], "open");
    let start = Instant::now();
    host.finish(true);
    assert!(start.elapsed() < Duration::from_secs(3));
    server.join().unwrap();
}

#[test]
fn channel_limit_and_duplicate_open_are_bounded() {
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        let mut sockets = Vec::new();
        for _ in 0..8 {
            sockets.push(accept(&listener));
        }
        for mut socket in sockets {
            let _ = socket.read();
        }
    });
    let mut host = Host::new(port);
    host.ready();
    for index in 0..8 {
        host.open(&index.to_string(), port);
        assert_eq!(host.next()["type"], "open");
    }
    host.open("0", port);
    assert_eq!(host.next()["code"], "channel_exists");
    host.open("9", port);
    assert_eq!(host.next()["code"], "channel_limit");
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn malformed_native_frame_exits_without_leaking_input() {
    let mut host = Host::new(53179);
    host.ready();
    host.input
        .as_mut()
        .unwrap()
        .write_all(&((MAX_FRAME + 1) as u32).to_le_bytes())
        .unwrap();
    assert_eq!(
        host.next(),
        json!({"type":"error","channelId":null,"code":"frame_limit"})
    );
    host.finish(false);
}

#[test]
fn full_32_mib_response_is_chunked_with_exact_boundary_and_hash() {
    let (listener, port) = listener();
    let text = "x".repeat(MAX_FRAME);
    let expected_hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    let server = thread::spawn(move || {
        let mut socket = accept(&listener);
        socket.send(Message::Text(text.into())).unwrap();
        loop {
            match socket.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("limit", port);
    assert_eq!(host.next()["type"], "open");
    let mut digest = Sha256::new();
    let mut total = 0;
    let count = MAX_FRAME.div_ceil(CHUNK_BYTES);
    for index in 0..count {
        let chunk = host.next();
        assert_eq!(chunk["type"], "chunk");
        assert_eq!(chunk["index"], index);
        assert_eq!(chunk["count"], count);
        assert_eq!(chunk["byteLength"], MAX_FRAME);
        assert_eq!(chunk["sha256"], expected_hash);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(chunk["dataBase64"].as_str().unwrap())
            .unwrap();
        total += bytes.len();
        digest.update(bytes);
    }
    assert_eq!(total, MAX_FRAME);
    assert_eq!(format!("{:x}", digest.finalize()), expected_hash);
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn websocket_over_32_mib_is_rejected_without_transport_retry() {
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        let mut socket = accept(&listener);
        // The limit is enforced from the frame header, before buffering all data.
        let mut header = vec![0x81, 127];
        header.extend_from_slice(&((MAX_FRAME + 1) as u64).to_be_bytes());
        socket.get_mut().write_all(&header).unwrap();
        thread::sleep(Duration::from_millis(100));
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("over", port);
    assert_eq!(host.next()["type"], "open");
    let close = host.next();
    assert_eq!(close["code"], 1009);
    assert_eq!(close["reason"], "message_limit");
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn bounded_channel_queue_reports_overload_and_eof_stops_incomplete_handshake() {
    let (listener, port) = listener();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + WAIT;
        let stream = loop {
            if let Ok((stream, _)) = listener.accept() {
                break stream;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        };
        // Deliberately leave the WebSocket handshake unanswered: every Send
        // must remain in the host's finite pre-open queue.
        thread::sleep(Duration::from_millis(500));
        drop(stream);
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("slow", port);
    for _ in 0..9 {
        host.send(json!({"type":"send","channelId":"slow","data":"synthetic-queued-payload"}));
    }
    assert_eq!(host.next()["code"], "queue_full");
    let start = Instant::now();
    host.finish(true);
    assert!(start.elapsed() < Duration::from_secs(3));
    server.join().unwrap();
}

#[test]
fn small_control_character_text_uses_one_chunk_to_keep_native_frame_under_one_mib() {
    let (listener, port) = listener();
    let text = "\u{1}".repeat(CHUNK_BYTES);
    let expected = text.as_bytes().to_vec();
    let server = thread::spawn(move || {
        let mut socket = accept(&listener);
        socket.send(Message::Text(text.into())).unwrap();
        loop {
            match socket.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });
    let mut host = Host::new(port);
    host.ready();
    host.open("controls", port);
    assert_eq!(host.next()["type"], "open");
    let event = host.next();
    assert_eq!(event["type"], "chunk");
    assert_eq!(event["count"], 1);
    assert_eq!(event["index"], 0);
    assert_eq!(event["byteLength"], CHUNK_BYTES);
    assert!(serde_json::to_vec(&event).unwrap().len() < 1024 * 1024);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(event["dataBase64"].as_str().unwrap())
            .unwrap(),
        expected
    );
    host.finish(true);
    server.join().unwrap();
}

#[test]
fn invalid_channel_identity_is_not_echoed_in_error_output() {
    let mut host = Host::new(53179);
    host.ready();
    let invalid = "\u{1}".repeat(10000);
    for kind in ["send", "close"] {
        let mut message = json!({"type":kind,"channelId":invalid});
        if kind == "send" {
            message["data"] = "never-sent".into();
        }
        host.send(message);
        assert_eq!(
            host.next(),
            json!({"type":"error","channelId":null,"code":"channel_rejected"})
        );
    }
    host.finish(true);
}
