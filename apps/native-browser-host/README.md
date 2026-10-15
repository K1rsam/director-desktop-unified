# Native browser transport host

Prototype transport for a thin Chromium/Edge extension. The Rust process moves
unchanged WebSocket text between the extension's Native Messaging port and the
existing loopback bridge. It does not inspect accounts, submit generation,
own project state, reconnect WebSockets, replay messages, or register itself.

The executable uses the Windows GUI subsystem, so Chromium can launch it
without a console window. Native stdin/stdout remain inherited binary pipes;
the integration tests exercise the actual Windows executable this way.

## Build and test

```powershell
& C:\Users\EDY\.cargo\bin\cargo.exe test --manifest-path apps/native-browser-host/Cargo.toml -- --test-threads=1
& C:\Users\EDY\.cargo\bin\cargo.exe test --release --manifest-path apps/native-browser-host/Cargo.toml -- --test-threads=1
& C:\Users\EDY\.cargo\bin\cargo.exe build --release --manifest-path apps/native-browser-host/Cargo.toml
```

Binary: `target/release/director-native-browser-host.exe` (relative to this
crate). The lockfile is committed; `target/` and isolated test fixtures are
ignored. Tests use synthetic local WebSockets and copied executables under
`target/test-host-fixtures/`, and never use a browser or provider account.

## Launch contract

The installer must create `native-host-config.json` next to the executable:

```json
{
  "schemaVersion": "director-native-host-config/v1",
  "allowedOrigins": ["chrome-extension://abcdefghijklmnopabcdefghijklmnop/"],
  "serverUrl": "http://127.0.0.1:53179"
}
```

The example extension ID is synthetic. Installers must discover and register
the actual permitted extension origins. The first argument must match one
configured origin exactly. The only permitted second argument is
`--parent-window=` followed by 1–20 decimal digits. All other argument lists
fail closed, before a protocol response or network call. This process neither
registers policy nor changes browser profiles.

The HTTP WebSocket `Origin` header serializes the same authorized extension
origin **without its trailing slash**, matching Chromium and the existing
bridge's strict equality policy. Only this transport-header spelling changes;
the config and Native Messaging caller argument retain their exact trailing
slash and no additional origins are accepted.

The configured endpoint must be canonical `http://127.0.0.1:<port>` with an
explicit nondefault port, no slash, credentials, path, query or fragment.
Requests must match that string exactly. `localhost`, IPv6, integer/hex hosts
and normalized alternate spellings are rejected. Production installation uses
the existing bridge port; random loopback ports are allowed for isolated tests.
Config contains no token. Each Open token is used only in the authenticated
`/extension/ws?token=...&runtime=extension` WebSocket handshake.

## Framing and events

Every stdin/stdout item is a 4-byte little-endian unsigned byte length followed
by UTF-8 JSON. Native input and WebSocket text have a 32 MiB hard limit. Partial
headers, partial bodies and invalid JSON fail closed. Startup emits
`{"type":"ready","version":"director-native-transport/v2"}`.

Input:

- `{ "type":"open", "channelId":"id", "serverUrl":"...", "token":"..." }`
- `{ "type":"send", "channelId":"id", "writeId":"write-1", "data":"unchanged JSON text" }`
- `{ "type":"close", "channelId":"id" }`
- `{ "type":"ping", "nonce":"caller nonce" }`

Output:

- `{ "type":"open", "channelId":"id" }` after a successful WS handshake.
- `{ "type":"sent", "channelId":"id", "writeId":"write-1" }` only after
  the corresponding `socket.send` succeeds. This is a **local transport write**
  acknowledgment, not remote Protocol5 acceptance or a generation receipt.
  Failed writes have no `sent` ACK and are never retried. `writeId` must be
  `write-` followed by decimal digits, at most 64 characters in total.
- `{ "type":"message", "channelId":"id", "data":"..." }` for text of at
  most 192 KiB. A small string whose JSON escaping exceeds Chromium's 1 MiB
  host-to-browser limit uses one chunk instead. Every native output frame is
  capped at 1 MiB.
- Larger text is sliced from the **original UTF-8 bytes**, up to 192 KiB per
  chunk, and emitted as `{ "type":"chunk", "channelId":"id",
  "transferId":"chunk-1", "index":0, "count":2, "byteLength":250000,
  "sha256":"whole-message lower-case hex SHA256", "dataBase64":"..." }`.
  A chunk may end inside a UTF-8 code point. Reassemble bytes and verify the
  entire hash before decoding. Indices are zero-based; FIFO is per channel.
  Transfer IDs use the channel-scoped `chunk-<sequence>` spelling and do not
  embed channel IDs, so long channel IDs cannot break adapter validation.
- `{ "type":"close", "channelId":"id", "code":1000,
  "reason":"local_close" }`; server close reason text is never echoed.
- `{ "type":"error", "channelId":"id or null", "code":"fixed_code" }`.
- `{ "type":"pong", "nonce":"caller nonce" }`.

Each channel has at most 8 queued messages and 32 MiB queued data, and at most
8 channels may exist. The output queue holds at most 32 frames. A congested
output waits at most 2 seconds and then stops transport; it never silently
continues after dropping a frame. Channel queue overflow emits `queue_full`;
the caller must keep uncertain application requests unchanged. A write failure
closes the channel without retrying, because partial transmission is unknown.
The v2 adapter keeps exactly one write in flight per channel until its exact
`sent` ACK, with a separate finite FIFO (256 messages / 32 MiB including the
in-flight item). The host still limits its own queue to 8; normal v2 traffic
cannot burst arbitrary unacknowledged writes into it. ACK loss/disconnect does
not trigger native replay; the unchanged persistent Protocol5 outbox retains
recovery ownership.

The worker drains at most 8 queued commands before reading, enables TCP_NODELAY,
and uses a 20 ms read/write timeout; connection/handshake use 2 seconds.
stdin EOF cancels every worker (including an incomplete handshake), and the
process exits without waiting indefinitely for a non-consuming stdout pipe.

Fixed error codes: `invalid_message`, `truncated_frame`, `frame_limit`,
`invalid_json`, `input_failed`, `nonce_limit`, `channel_rejected`,
`channel_exists`, `channel_limit`, `server_rejected`, `token_rejected`,
`channel_not_open`, `channel_closing`, `queue_full`, `message_limit`,
`connect_failed`, `socket_failed`, `origin_rejected`, `handshake_failed`.
`invalid_write` rejects a malformed v2 write identity without echoing its value.

Fixed close reasons: `local_close`, `remote_close`, `send_failed`,
`transport_failed`, `binary_rejected`, `message_limit`, `connect_failed`,
`socket_failed`, `origin_rejected`, `handshake_failed`, `server_rejected`.
Normal local/error close codes use 1000/1003/1006/1009; remote numeric close
codes are retained. Startup/config errors produce exit code 1 and no raw
diagnostic text; tokens and backend close reasons are never logged.

This is transport-level evidence, not an end-to-end webpage/image-generation
acceptance result. Extension installation, real browser loading and provider
generation/download require separate acceptance by the main integration task.

## Windows evidence (2026-10-15)

Native Windows MSVC toolchain: Cargo `1.99.0 (5f94df478 2026-08-27)`.

- `cargo check`: passed.
- Initial debug integration suite: 15 passed, 0 failed (6.24 s).
- Final v2 `cargo test --release ... -- --test-threads=1`: **21 passed, 0 failed**
  (5.38 s), using actual copied GUI-subsystem executables and redirected pipes.
- Release executable: 718848 bytes, PE subsystem 2 (Windows GUI), SHA-256
  `de234a0ed621f31a344c47b5b74d3225ecfdfaf7f638a85ca1e180cbe4f2bae8`.
- `cargo clippy`: not run successfully; this machine's active toolchain does
  not have `cargo-clippy.exe` installed. No toolchain component was installed.

Coverage includes three real simultaneous local WebSocket channels with five
FIFO echoes each, exact origin and endpoint rejection, independent channel
close, eight-channel and bounded-queue limits, remote close with zero reconnect
or replay, Unicode/native frame fragmentation, truncated/oversized framing,
cross-code-point chunk boundaries, a full 32 MiB response reassembled from 171
chunks with exact SHA-256, a >32 MiB frame rejected from its header, the small
control-character escape case, and stdin EOF during both a live socket and an
incomplete handshake. No account, browser, image or video generation was used.
The strict-origin regression separately confirms slash-suffixed caller
authorization, rejection of a no-slash caller argument, and the server's exact
no-slash HTTP Origin handshake followed by a real local WebSocket roundtrip.
The v2 suite additionally sends 64 real local WebSocket messages one per exact
`sent` ACK and checks FIFO, validates required write identities, asserts real
chunk IDs against the adapter alphabet, and exports a real host-output fixture
to `target/native-v2-wire-fixture.json`. That synthetic fixture contains ready,
open and two chunk frames for 210000 Unicode UTF-8 bytes, plus expected total
length and SHA-256, so the adapter can consume host output without recreating
its own assumed wire fields.
