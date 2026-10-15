const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { webcrypto, createHash } = require('node:crypto');

function fixture(options = {}) {
  let sequence = 0, now = 0;
  const scheduled = new Map(), ports = [];
  const timers = {
    setTimeout(fn, ms) { const id = ++sequence; scheduled.set(id, { fn, at: now + ms }); return id; },
    clearTimeout(id) { scheduled.delete(id); },
  };
  const event = () => {
    const handlers = new Set();
    return { addListener: fn => handlers.add(fn), removeListener: fn => handlers.delete(fn), emit: value => { for (const fn of [...handlers]) fn(value); } };
  };
  const browser = { runtime: { connectNative(name) {
    const port = { name, onMessage: event(), onDisconnect: event(), sent: [], disconnected: false,
      postMessage(message) { this.sent.push(message); },
      disconnect() { this.disconnected = true; this.onDisconnect.emit(); },
    };
    ports.push(port);
    return port;
  } } };
  const context = vm.createContext({ TextEncoder, TextDecoder, Uint8Array, atob, globalThis: {}, });
  const source = fs.readFileSync(path.join(__dirname, '../native/extension/background/nativeTransport.js'), 'utf8');
  vm.runInContext(source.replace('export function', 'function') + '\nglobalThis.factory=createNativeTransportFactory;', context);
  const factory = context.globalThis.factory({ browser, timers, crypto: options.crypto || webcrypto, timeoutMs: 100 });
  const connect = id => {
    const socket = factory.connect({ serverUrl: 'http://127.0.0.1:53179', token: 'private-token', channelId: id });
    const events = [];
    for (const type of ['open', 'message', 'close', 'error']) socket.addEventListener(type, e => events.push(e));
    return { socket, events };
  };
  const advance = ms => {
    const until = now + ms;
    for (;;) {
      const next = [...scheduled].filter(([, x]) => x.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
      if (!next) break;
      now = next[1].at; scheduled.delete(next[0]); next[1].fn();
    }
    now = until;
  };
  const receive = message => ports.at(-1).onMessage.emit(message);
  const ready = () => receive({ type: 'ready', version: 'director-native-transport/v1' });
  const open = id => receive({ type: 'open', channelId: id });
  return { factory, ports, scheduled, connect, advance, receive, ready, open };
}
const tick = () => new Promise(resolve => setImmediate(resolve));
const chunks = (id, text) => {
  const bytes = Buffer.from(text), size = 192 * 1024, hash = createHash('sha256').update(bytes).digest('hex');
  return Array.from({ length: Math.ceil(bytes.length / size) }, (_, index) => ({
    type: 'chunk', channelId: id, transferId: 'transfer-1', index,
    count: Math.ceil(bytes.length / size), byteLength: bytes.length, sha256: hash,
    dataBase64: bytes.subarray(index * size, (index + 1) * size).toString('base64'),
  }));
};

test('two channels share one native port and preserve raw protocol data without replay', async () => {
  const f = fixture(), a = f.connect('a'), b = f.connect('b');
  assert.equal(f.ports.length, 1); assert.equal(a.socket.readyState, 0);
  assert.equal(f.ports[0].name, 'app.director.browser_bridge');
  assert.deepEqual(f.ports[0].sent.map(x => x.type), ['open', 'open']);
  f.ready(); f.open('a'); f.open('b');
  a.socket.send('{"requestId":"same","body":"原文"}');
  f.receive({ type: 'message', channelId: 'b', data: 'reply-b' }); await tick();
  assert.equal(a.events.filter(x => x.type === 'message').length, 0);
  assert.equal(b.events.find(x => x.type === 'message').data, 'reply-b');
  assert.equal(f.ports[0].sent.filter(x => x.type === 'send').length, 1);
  a.socket.close(); b.socket.close();
  assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});

test('readiness version and early messages fail closed without exposing raw host diagnostics', () => {
  for (const message of [{ type: 'ready', version: 'wrong' }, { type: 'open', channelId: 'a' }]) {
    const f = fixture(), a = f.connect('a'); f.receive(message);
    assert.equal(a.socket.readyState, 3);
    assert.equal(f.ports[0].disconnected, true);
    assert.equal(f.scheduled.size, 0);
    assert.throws(() => a.socket.send('again'), /native_channel_not_open/);
  }
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
  f.receive({ type: 'error', channelId: 'a', code: 'SECRET_TOKEN_VALUE' });
  assert.equal(a.events.at(-1).reason, 'native_host_error'); f.factory.shutdown();
});

test('disconnect closes all channels; only a new connect creates a port and no send is replayed', () => {
  const f = fixture(), a = f.connect('a'), b = f.connect('b'); f.ready(); f.open('a'); f.open('b'); a.socket.send('one');
  f.ports[0].onDisconnect.emit();
  assert.equal(a.socket.readyState, 3); assert.equal(b.socket.readyState, 3);
  assert.equal(f.scheduled.size, 0); assert.equal(f.ports.length, 1);
  const c = f.connect('c'); assert.equal(f.ports.length, 2);
  assert.deepEqual(f.ports[1].sent.map(x => x.type), ['open']);
  assert.equal(c.socket.readyState, 0); f.factory.shutdown(); assert.equal(f.scheduled.size, 0);
});

test('host readiness and channel opening deadlines release timers without resending', () => {
  const f = fixture(), a = f.connect('a'); f.advance(100);
  assert.equal(a.events.at(-1).reason, 'native_ready_timeout'); assert.equal(f.scheduled.size, 0);
  const b = f.connect('b'); f.ready(); f.advance(100);
  assert.equal(b.events.at(-1).reason, 'native_open_timeout');
  assert.equal(f.ports[1].sent.filter(x => x.type === 'open').length, 1);
  assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});

test('channel limits and duplicate ids never detach the existing channel', () => {
  const f = fixture(), sockets = Array.from({ length: 8 }, (_, i) => f.connect('c' + i));
  f.ready(); for (let i = 0; i < 8; i++) f.open('c' + i);
  const duplicate = f.connect('c0'), overflow = f.connect('overflow'); f.advance(0);
  assert.equal(duplicate.events.at(-1).reason, 'native_channel_exists');
  assert.equal(overflow.events.at(-1).reason, 'native_channel_limit');
  assert.equal(sockets[0].socket.readyState, 1); assert.equal(f.ports.length, 1);
  f.factory.shutdown(); assert.equal(f.scheduled.size, 0);
});

test('Unicode chunk reassembly verifies bytes and sha256 before ordered delivery', async () => {
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
  const text = '角色：柳凡🐢\n'.repeat(40000), parts = chunks('a', text);
  for (const part of parts) f.receive(part);
  f.receive({ type: 'message', channelId: 'a', data: 'after' });
  for (let i = 0; i < 20 && a.events.filter(x => x.type === 'message').length < 2; i++) await tick();
  assert.deepEqual(a.events.filter(x => x.type === 'message').map(x => x.data), [text, 'after']);
  a.socket.close(); assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});

test('out of order, inconsistent lengths and hashes close the channel without delivering partial data', async () => {
  for (const kind of ['order', 'length', 'hash', 'interleave', 'duplicate', 'metadata']) {
    const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
    const parts = chunks('a', '甲'.repeat(100000));
    if (kind === 'order') f.receive(parts[1]);
    if (kind === 'length') { parts[0].dataBase64 = 'YQ=='; f.receive(parts[0]); }
    if (kind === 'hash') for (const part of parts) f.receive({ ...part, sha256: '0'.repeat(64) });
    if (kind === 'interleave') { f.receive(parts[0]); f.receive({ type: 'message', channelId: 'a', data: 'skip' }); }
    if (kind === 'duplicate') { f.receive(parts[0]); f.receive(parts[0]); }
    if (kind === 'metadata') f.receive({ ...parts[0], byteLength: 32 * 1024 * 1024 + 1 });
    for (let i = 0; i < 20 && a.socket.readyState !== 3; i++) await tick();
    assert.equal(a.socket.readyState, 3, kind); assert.equal(a.events.filter(x => x.type === 'message').length, 0, kind);
    assert.equal(f.scheduled.size, 0, kind); f.factory.shutdown();
  }
});

test('an incomplete chunk transfer has a bounded deadline and cannot be completed after closure', async () => {
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
  const parts = chunks('a', '图'.repeat(100000)); f.receive(parts[0]); await tick(); f.advance(100);
  assert.equal(a.events.at(-1).reason, 'native_chunk_timeout');
  f.receive(parts[1]); await tick();
  assert.equal(a.events.filter(x => x.type === 'message').length, 0); assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});

test('heartbeat is channel-scoped, validates exact pong, and disconnects without reconnecting', () => {
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a'); f.advance(20000);
  const ping = f.ports[0].sent.at(-1); assert.equal(ping.type, 'ping');
  f.receive({ type: 'pong', nonce: 'different' }); f.advance(100);
  assert.equal(a.events.at(-1).reason, 'native_heartbeat_timeout'); assert.equal(f.ports.length, 1);
  assert.equal(f.scheduled.size, 0);
  const b = f.connect('b'); f.ready(); f.open('b'); f.advance(20000);
  f.receive({ type: 'pong', nonce: f.ports[1].sent.at(-1).nonce }); f.advance(100);
  assert.equal(b.socket.readyState, 1); b.socket.close(); assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});

test('shutdown and local close during pending digest cannot deliver late messages or leave timers', async () => {
  let resolveDigest;
  const f = fixture({ crypto: { subtle: { digest: () => new Promise(resolve => { resolveDigest = resolve; }) } } });
  const a = f.connect('a'); f.ready(); f.open('a');
  for (const part of chunks('a', '图'.repeat(100000))) f.receive(part);
  await tick(); f.factory.shutdown(); resolveDigest(new Uint8Array(32)); await tick();
  assert.equal(a.events.filter(x => x.type === 'message').length, 0); assert.equal(f.scheduled.size, 0);
  const b = f.connect('b'); b.socket.close(); assert.equal(f.scheduled.size, 0);
});

test('all sockets closed before ready release the native readiness deadline', () => {
  const f = fixture(), a = f.connect('a'); a.socket.close();
  assert.equal(f.scheduled.size, 0); assert.equal(f.ports[0].disconnected, true);
  const b = f.connect('b'); assert.equal(f.ports.length, 2);
  f.factory.shutdown(); assert.equal(b.socket.readyState, 3); assert.equal(f.scheduled.size, 0);
});

test('remote close waits for already delivered chunks to be verified', async () => {
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
  for (const part of chunks('a', '图'.repeat(100000))) f.receive(part);
  f.receive({ type: 'close', channelId: 'a', code: 1000, reason: 'remote_close' });
  for (let i = 0; i < 20 && a.socket.readyState !== 3; i++) await tick();
  assert.deepEqual(a.events.map(e => e.type), ['open', 'message', 'close']);
  assert.equal(a.events.at(-1).reason, 'remote_close'); assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});

test('global host errors acknowledge only fixed diagnostics and close the shared port', () => {
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
  f.receive({ type: 'error', code: 'invalid_json', message: 'private-payload' });
  assert.equal(a.events.at(-1).reason, 'invalid_json');
  assert.equal(f.ports[0].disconnected, true); assert.equal(f.scheduled.size, 0);
});

test('local protocol refusal preserves its safe close code without transmitting raw reason', () => {
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
  a.socket.close(1002, 'private protocol failure');
  assert.equal(a.events.at(-1).code, 1002); assert.equal(a.events.at(-1).wasClean, false);
  assert.equal(a.events.at(-1).reason, 'native_channel_closed');
  assert.deepEqual(Object.keys(f.ports[0].sent.at(-1)).sort(), ['channelId', 'type']);
  assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});

test('small control-character payload may arrive as one verified chunk to avoid JSON expansion limits', async () => {
  const f = fixture(), a = f.connect('a'); f.ready(); f.open('a');
  const text = '\u0000'.repeat(180 * 1024), parts = chunks('a', text);
  assert.equal(parts.length, 1); assert.ok(Buffer.byteLength(JSON.stringify({ type: 'message', channelId: 'a', data: text })) > 1024 * 1024);
  f.receive(parts[0]);
  for (let i = 0; i < 20 && a.events.filter(x => x.type === 'message').length < 1; i++) await tick();
  assert.equal(a.events.find(x => x.type === 'message').data, text);
  a.socket.close(); assert.equal(f.scheduled.size, 0); f.factory.shutdown();
});
