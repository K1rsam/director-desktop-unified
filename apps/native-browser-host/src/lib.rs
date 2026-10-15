//! Narrow, account-agnostic Native Messaging transport. This process owns no
//! production state and never reconnects or retries application messages.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};
use url::Url;

pub const VERSION: &str = "director-native-transport/v2";
pub const MAX_FRAME: usize = 32 * 1024 * 1024;
pub const CHUNK_BYTES: usize = 192 * 1024;
pub const MAX_CHANNELS: usize = 8;
const NATIVE_OUTPUT_LIMIT: usize = 1024 * 1024;
const OUTPUT_QUEUE: usize = 32;
const CHANNEL_QUEUE: usize = 8;
const IO_TIMEOUT: Duration = Duration::from_millis(20);
const SEND_BATCH: usize = 8;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub schema_version: String,
    pub allowed_origins: Vec<String>,
    pub server_url: String,
}

impl Config {
    pub fn read(path: &Path) -> Result<Self, &'static str> {
        let bytes = std::fs::read(path).map_err(|_| "config_unavailable")?;
        if bytes.len() > 16 * 1024 {
            return Err("invalid_config");
        }
        let config: Self = serde_json::from_slice(&bytes).map_err(|_| "invalid_config")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != "director-native-host-config/v1"
            || self.allowed_origins.is_empty()
            || self.allowed_origins.len() > 16
            || self
                .allowed_origins
                .iter()
                .any(|origin| !valid_origin(origin))
        {
            return Err("invalid_config");
        }
        loopback_url(&self.server_url).map(|_| ())
    }

    pub fn validate_arguments(&self, arguments: &[String]) -> Result<String, &'static str> {
        self.validate()?;
        if !(1..=2).contains(&arguments.len()) || !self.allowed_origins.contains(&arguments[0]) {
            return Err("origin_rejected");
        }
        if let Some(parent) = arguments.get(1) {
            let digits = parent
                .strip_prefix("--parent-window=")
                .ok_or("arguments_rejected")?;
            if digits.is_empty() || digits.len() > 20 || !digits.bytes().all(|c| c.is_ascii_digit())
            {
                return Err("arguments_rejected");
            }
        }
        Ok(arguments[0].clone())
    }

    fn socket_url(&self, requested: &str, token: &str) -> Result<Url, &'static str> {
        if requested != self.server_url {
            return Err("server_rejected");
        }
        let mut url = loopback_url(requested)?;
        if token.is_empty()
            || token.len() > 4096
            || !token.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err("token_rejected");
        }
        url.set_scheme("ws").map_err(|_| "server_rejected")?;
        url.set_path("/extension/ws");
        url.query_pairs_mut()
            .append_pair("token", token)
            .append_pair("runtime", "extension");
        Ok(url)
    }
}

fn valid_origin(origin: &str) -> bool {
    origin
        .strip_prefix("chrome-extension://")
        .and_then(|s| s.strip_suffix('/'))
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|c| (b'a'..=b'p').contains(&c)))
}

fn loopback_url(value: &str) -> Result<Url, &'static str> {
    let url = Url::parse(value).map_err(|_| "server_rejected")?;
    // Require the canonical literal spelling too: URL normalization must not
    // turn hexadecimal hosts, backslashes, dot segments or credentials into a
    // permitted endpoint. Config and request are both exact strings.
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || value != format!("http://127.0.0.1:{}", url.port().ok_or("server_rejected")?)
    {
        return Err("server_rejected");
    }
    Ok(url)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum Input {
    Open {
        #[serde(rename = "channelId")]
        channel_id: String,
        #[serde(rename = "serverUrl")]
        server_url: String,
        token: String,
    },
    Send {
        #[serde(rename = "channelId")]
        channel_id: String,
        #[serde(rename = "writeId")]
        write_id: String,
        data: String,
    },
    Close {
        #[serde(rename = "channelId")]
        channel_id: String,
    },
    Ping {
        nonce: String,
    },
}

/// EOF between frames is clean; any partial header/body is rejected. Limits
/// are checked before allocating a body buffer.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Value>, &'static str> {
    let mut header = [0u8; 4];
    let mut offset = 0;
    while offset < 4 {
        match reader.read(&mut header[offset..]) {
            Ok(0) if offset == 0 => return Ok(None),
            Ok(0) => return Err("truncated_frame"),
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err("input_failed"),
        }
    }
    let size = u32::from_le_bytes(header) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err("frame_limit");
    }
    let mut bytes = vec![0; size];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "truncated_frame")?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "invalid_json")
}

pub fn write_frame<W: Write>(writer: &mut W, value: &Value) -> Result<(), &'static str> {
    write_frame_with_limit(writer, value, MAX_FRAME)
}

fn write_frame_with_limit<W: Write>(
    writer: &mut W,
    value: &Value,
    limit: usize,
) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(value).map_err(|_| "output_failed")?;
    if bytes.len() > limit {
        return Err("frame_limit");
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .map_err(|_| "output_failed")?;
    writer.write_all(&bytes).map_err(|_| "output_failed")?;
    writer.flush().map_err(|_| "output_failed")
}

#[derive(Clone)]
struct Output {
    sender: SyncSender<Value>,
    stopped: Arc<AtomicBool>,
}
impl Output {
    fn emit(&self, value: Value) -> bool {
        // Backpressure is bounded and explicit; it must never block EOF
        // shutdown or silently drop an individual message while continuing.
        let mut pending = value;
        let deadline = std::time::Instant::now() + CONNECT_TIMEOUT;
        loop {
            match self.sender.try_send(pending) {
                Ok(()) => return true,
                Err(TrySendError::Full(value))
                    if !self.stopped.load(Ordering::Acquire)
                        && std::time::Instant::now() < deadline =>
                {
                    pending = value;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => {
                    self.stopped.store(true, Ordering::Release);
                    return false;
                }
            }
        }
    }
    fn error(&self, channel: Option<&str>, code: &'static str) {
        self.emit(json!({"type":"error","channelId":channel,"code":code}));
    }
    fn close(&self, channel: &str, code: u16, reason: &'static str) {
        self.emit(json!({"type":"close","channelId":channel,"code":code,"reason":reason}));
    }
}

struct Channel {
    sender: SyncSender<Command>,
    pending: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}
enum Command {
    Send { write_id: String, data: String },
}

/// Run after config/origin authentication. The writer thread is intentionally
/// detached at final shutdown: a browser that stops consuming stdout cannot
/// prevent this process and its socket workers from exiting on stdin EOF.
pub fn run<R: Read, W: Write + Send + 'static>(
    mut input: R,
    mut writer: W,
    config: Config,
    origin: String,
) -> Result<(), &'static str> {
    config.validate_arguments(&[origin.clone()])?;
    let stopped = Arc::new(AtomicBool::new(false));
    let (output_tx, output_rx) = mpsc::sync_channel(OUTPUT_QUEUE);
    let output = Output {
        sender: output_tx,
        stopped: stopped.clone(),
    };
    let writer_stopped = stopped.clone();
    let writer_thread = thread::spawn(move || {
        while let Ok(value) = output_rx.recv() {
            if write_frame_with_limit(&mut writer, &value, NATIVE_OUTPUT_LIMIT).is_err() {
                writer_stopped.store(true, Ordering::Release);
                break;
            }
        }
    });
    output.emit(json!({"type":"ready","version":VERSION}));
    let mut channels: HashMap<String, Channel> = HashMap::new();
    let mut result = Ok(());
    while !stopped.load(Ordering::Acquire) {
        let value = match read_frame(&mut input) {
            Ok(Some(value)) => value,
            Ok(None) => break,
            Err(code) => {
                output.error(None, code);
                result = Err(code);
                break;
            }
        };
        let message: Input = match serde_json::from_value(value) {
            Ok(message) => message,
            Err(_) => {
                output.error(None, "invalid_message");
                continue;
            }
        };
        channels.retain(|_, channel| !channel.done.load(Ordering::Acquire));
        match message {
            Input::Ping { nonce } => {
                if nonce.len() > 256 {
                    output.error(None, "nonce_limit");
                } else {
                    output.emit(json!({"type":"pong","nonce":nonce}));
                }
            }
            Input::Open {
                channel_id,
                server_url,
                token,
            } => {
                if !valid_channel(&channel_id) {
                    output.error(None, "channel_rejected");
                    continue;
                }
                if channels.contains_key(&channel_id) {
                    output.error(Some(&channel_id), "channel_exists");
                    continue;
                }
                if channels.len() >= MAX_CHANNELS {
                    output.error(Some(&channel_id), "channel_limit");
                    continue;
                }
                let url = match config.socket_url(&server_url, &token) {
                    Ok(url) => url,
                    Err(code) => {
                        output.error(Some(&channel_id), code);
                        continue;
                    }
                };
                let (sender, receiver) = mpsc::sync_channel(CHANNEL_QUEUE);
                let pending = Arc::new(AtomicUsize::new(0));
                let done = Arc::new(AtomicBool::new(false));
                let cancel = Arc::new(AtomicBool::new(false));
                let (worker_pending, worker_done, worker_cancel) =
                    (pending.clone(), done.clone(), cancel.clone());
                let (worker_output, worker_origin, worker_id) =
                    (output.clone(), origin.clone(), channel_id.clone());
                let worker = thread::spawn(move || {
                    channel_worker(
                        &worker_id,
                        url,
                        &worker_origin,
                        receiver,
                        worker_pending,
                        worker_cancel,
                        &worker_output,
                    );
                    worker_done.store(true, Ordering::Release);
                });
                channels.insert(
                    channel_id,
                    Channel {
                        sender,
                        pending,
                        done,
                        cancel,
                        thread: worker,
                    },
                );
            }
            Input::Send {
                channel_id,
                write_id,
                data,
            } => {
                if !valid_channel(&channel_id) {
                    output.error(None, "channel_rejected");
                    continue;
                }
                if !valid_write(&write_id) {
                    output.error(Some(&channel_id), "invalid_write");
                    continue;
                }
                let Some(channel) = channels.get(&channel_id) else {
                    output.error(Some(&channel_id), "channel_not_open");
                    continue;
                };
                if channel.cancel.load(Ordering::Acquire) {
                    output.error(Some(&channel_id), "channel_closing");
                    continue;
                }
                let bytes = data.len();
                if bytes > MAX_FRAME {
                    output.error(Some(&channel_id), "message_limit");
                    continue;
                }
                if channel
                    .pending
                    .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                        current.checked_add(bytes).filter(|sum| *sum <= MAX_FRAME)
                    })
                    .is_err()
                {
                    output.error(Some(&channel_id), "queue_full");
                    continue;
                }
                if let Err(error) = channel.sender.try_send(Command::Send { write_id, data }) {
                    channel.pending.fetch_sub(bytes, Ordering::AcqRel);
                    output.error(
                        Some(&channel_id),
                        match error {
                            TrySendError::Full(_) => "queue_full",
                            TrySendError::Disconnected(_) => "channel_not_open",
                        },
                    );
                }
            }
            Input::Close { channel_id } => {
                if !valid_channel(&channel_id) {
                    output.error(None, "channel_rejected");
                    continue;
                }
                if let Some(channel) = channels.get(&channel_id) {
                    channel.cancel.store(true, Ordering::Release);
                } else {
                    output.error(Some(&channel_id), "channel_not_open");
                }
            }
        }
    }
    stopped.store(true, Ordering::Release);
    for channel in channels.values() {
        channel.cancel.store(true, Ordering::Release);
    }
    for (_, channel) in channels {
        let _ = channel.thread.join();
    }
    drop(output);
    // Normally drain ready/error events before return. Never join a writer
    // blocked by a non-consuming browser pipe.
    for _ in 0..20 {
        if writer_thread.is_finished() {
            let _ = writer_thread.join();
            return result;
        }
        thread::sleep(Duration::from_millis(5));
    }
    result
}

fn valid_channel(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
}

fn valid_write(value: &str) -> bool {
    value.len() <= 64
        && value
            .strip_prefix("write-")
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit()))
}

fn connect(url: &Url, origin: &str) -> Result<WebSocket<TcpStream>, &'static str> {
    let port = url.port().ok_or("server_rejected")?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let stream =
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|_| "connect_failed")?;
    stream.set_nodelay(true).map_err(|_| "socket_failed")?;
    stream
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|_| "socket_failed")?;
    stream
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|_| "socket_failed")?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| "connect_failed")?;
    // Chromium's Native Messaging caller argument includes a trailing slash;
    // the WebSocket HTTP Origin is the serialized origin without that slash.
    // Authorization remains an exact match against the original caller above.
    let http_origin = origin.strip_suffix('/').ok_or("origin_rejected")?;
    request.headers_mut().insert(
        "origin",
        http_origin.parse().map_err(|_| "origin_rejected")?,
    );
    let settings = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME))
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_FRAME + 1024);
    let (socket, _) = tungstenite::client::client_with_config(request, stream, Some(settings))
        .map_err(|_| "handshake_failed")?;
    socket
        .get_ref()
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "socket_failed")?;
    socket
        .get_ref()
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "socket_failed")?;
    Ok(socket)
}

fn channel_worker(
    channel_id: &str,
    url: Url,
    origin: &str,
    input: Receiver<Command>,
    pending: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    output: &Output,
) {
    let mut socket = match connect(&url, origin) {
        Ok(socket) => socket,
        Err(code) => {
            output.error(Some(channel_id), code);
            output.close(channel_id, 1006, code);
            return;
        }
    };
    if !output.emit(json!({"type":"open","channelId":channel_id})) {
        return;
    }
    let mut transfer_sequence = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) || output.stopped.load(Ordering::Acquire) {
            let _ = socket.close(None);
            output.close(channel_id, 1000, "local_close");
            return;
        }
        // Drain only a finite batch, preserving inbound fairness and FIFO.
        // `sent` acknowledges this local socket write, never remote Protocol5
        // acceptance or generation. Failed writes have no ACK and no retry.
        for _ in 0..SEND_BATCH {
            if cancel.load(Ordering::Acquire) || output.stopped.load(Ordering::Acquire) {
                break;
            }
            match input.try_recv() {
                Ok(Command::Send { write_id, data }) => {
                    pending.fetch_sub(data.len(), Ordering::AcqRel);
                    if socket.send(Message::Text(data.into())).is_err() {
                        output.close(channel_id, 1006, "send_failed");
                        return;
                    }
                    if !output
                        .emit(json!({"type":"sent","channelId":channel_id,"writeId":write_id}))
                    {
                        return;
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    output.close(channel_id, 1000, "local_close");
                    return;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                transfer_sequence += 1;
                if !emit_text(output, channel_id, text.as_str(), transfer_sequence) {
                    return;
                }
            }
            Ok(Message::Close(frame)) => {
                let code = frame.map_or(1000, |frame| u16::from(frame.code));
                // Never echo server close text: it may contain credentials.
                output.close(channel_id, code, "remote_close");
                return;
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                if socket.flush().is_err() {
                    output.close(channel_id, 1006, "transport_failed");
                    return;
                }
            }
            Ok(Message::Binary(_)) | Ok(Message::Frame(_)) => {
                output.close(channel_id, 1003, "binary_rejected");
                return;
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::Capacity(_)) => {
                output.close(channel_id, 1009, "message_limit");
                return;
            }
            Err(_) => {
                output.close(channel_id, 1006, "transport_failed");
                return;
            }
        }
    }
}

fn emit_text(output: &Output, channel: &str, text: &str, sequence: u64) -> bool {
    if text.len() > MAX_FRAME {
        output.close(channel, 1009, "message_limit");
        return false;
    }
    if text.len() <= CHUNK_BYTES {
        let event = json!({"type":"message","channelId":channel,"data":text});
        // Chromium caps host-to-browser native messages at 1 MiB. A small
        // all-control-character string can exceed it after JSON escaping.
        if serde_json::to_vec(&event).is_ok_and(|bytes| bytes.len() <= NATIVE_OUTPUT_LIMIT) {
            return output.emit(event);
        }
    }
    let bytes = text.as_bytes();
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let count = bytes.len().div_ceil(CHUNK_BYTES);
    let transfer_id = format!("chunk-{sequence}");
    for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
        if !output.emit(
            json!({"type":"chunk","channelId":channel,"transferId":transfer_id,
            "index":index,"count":count,"byteLength":bytes.len(),"sha256":sha256,
            "dataBase64":base64::engine::general_purpose::STANDARD.encode(chunk)}),
        ) {
            return false;
        }
    }
    true
}
