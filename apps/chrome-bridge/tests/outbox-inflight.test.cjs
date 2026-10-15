'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const clone = value => structuredClone(value);
function fixture(count = 0) {
  const registrations = [];
  class ObservedWeakMap extends WeakMap {
    constructor() { super(); registrations.push(this); }
  }
  const source = fs.readFileSync(path.join(__dirname, '../overrides/extension/background/outboxV5.js'), 'utf8')
    .replace(/^import .*;\r?\n/gm, '').replace(/export (?=function|class)/g, '');
  const lease = { requestId: 'original-request', status: 'submission_unknown', originalHash: 'unchanged' };
  const runtime = { contentEpoch: 'content-original', lease, transport: { outboundSequence: count }, outbox: [] };
  for (let i = 0; i < count; i++) runtime.outbox.push({
    messageId: `critical-${i}`, messageType: 'command.result', sentAt: 1,
    source: { clientId: 'original-client', backgroundEpoch: 'old-worker', contentEpoch: 'content-original', sequence: i + 1, lease: clone(lease) },
    request: { requestId: 'original-request', attemptId: 'original-attempt' },
    body: { raw: `原文🦉\\\"\n${i}`, canonical: { originalResult: true }, unknown: true },
  });
  const transitions = [], timers = new Map();
  let nextMessage = 0, nextTimer = 0, transitionHook = null, readHook = null;
  const context = vm.createContext({
    console, URL, structuredClone, Date, WebSocket: { OPEN: 1 }, WeakMap: ObservedWeakMap,
    setTimeout(fn) { const id = { id: ++nextTimer, unref() {} }; timers.set(id, fn); return id; },
    clearTimeout(id) { timers.delete(id); },
    isCriticalMessageType: type => type === 'command.result', messageDefinition: () => ({}),
    makeEnvelope: (type, body, source, sequence, options = {}) => ({
      messageId: options.messageId || `live-${++nextMessage}`, messageType: type,
      body: clone(body), source: { ...source, sequence }, request: options.request || null, sentAt: 2,
    }),
  });
  vm.runInContext(source, context);
  const backgroundState = {
    async read() { const value = clone(runtime); if (readHook) await readHook(value); return value; },
    async transition(_tab, event) {
      transitions.push(clone(event));
      if (transitionHook) await transitionHook(event);
      if (event.type === 'transport.outbound.next') runtime.transport.outboundSequence++;
      else if (event.type === 'outbox.enqueued') {
        if (!runtime.outbox.some(entry => entry.messageId === event.envelope.messageId)) runtime.outbox.push(clone(event.envelope));
      } else if (event.type === 'outbox.resequenced') {
        const index = runtime.outbox.findIndex(entry => entry.messageId === event.envelope.messageId);
        if (index < 0) return { accepted: false, reason: 'outbox_message_missing', state: clone(runtime) };
        runtime.outbox[index] = clone(event.envelope);
      } else throw Error(`Unexpected transition: ${event.type}`);
      return { accepted: true, state: clone(runtime) };
    },
  };
  function socket() {
    return { readyState: 1, sent: [], failNext: false,
      send(raw) { if (this.failNext) { this.failNext = false; throw Error('synthetic-send-failure'); } this.sent.push(raw); },
    };
  }
  const state = { tabId: 123, clientId: 'current-client', contentEpoch: 'content-original', port: {}, ws: socket() };
  const outbox = context.createProtocolOutbox({ backgroundEpoch: 'current-worker', backgroundState, post() {}, summarize: value => value });
  return { state, runtime, outbox, socket, transitions, timers,
    ack(id) { runtime.outbox = runtime.outbox.filter(entry => entry.messageId !== id); },
    flights(ws = state.ws) { return registrations[0].get(ws); },
    setTransitionHook(fn) { transitionHook = fn; }, setReadHook(fn) { readHook = fn; },
  };
}
function protectedBytes(envelope) {
  return JSON.stringify({ messageId: envelope.messageId, messageType: envelope.messageType,
    request: envelope.request, body: envelope.body, lease: envelope.source.lease });
}
function deferred() { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; }

test('64 persisted messages with a flush after every Protocol5 ACK send exactly 64 times', async () => {
  const f = fixture(64), ids = f.runtime.outbox.map(entry => entry.messageId);
  const originals = new Map(f.runtime.outbox.map(entry => [entry.messageId, protectedBytes(entry)]));
  await f.outbox.flushCriticalOutbox(f.state);
  assert.equal(f.state.ws.sent.length, 64);
  for (const id of ids) {
    f.ack(id); await f.outbox.flushCriticalOutbox(f.state);
  }
  assert.equal(f.state.ws.sent.length, 64);
  assert.equal(f.transitions.filter(event => event.type === 'outbox.resequenced').length, 64);
  assert.equal(f.runtime.outbox.length, 0); assert.equal(f.flights().sent.size, 0);
  for (const raw of f.state.ws.sent) {
    const envelope = JSON.parse(raw); assert.equal(protectedBytes(envelope), originals.get(envelope.messageId));
  }
});

test('repeated same-socket flushes retain persisted entries and never treat Native sent as Protocol5 ACK', async () => {
  const f = fixture(12); await f.outbox.flushCriticalOutbox(f.state);
  const persisted = JSON.stringify(f.runtime.outbox);
  await Promise.all(Array.from({ length: 10 }, () => f.outbox.flushCriticalOutbox(f.state)));
  assert.equal(f.state.ws.sent.length, 12); assert.equal(f.runtime.outbox.length, 12);
  assert.equal(JSON.stringify(f.runtime.outbox), persisted);
  assert.equal(f.flights().sent.size, 12);
});

test('a new socket replays only remaining persisted messages once with original bodies leases and unknown state', async () => {
  const f = fixture(8), originals = new Map(f.runtime.outbox.map(entry => [entry.messageId, protectedBytes(entry)]));
  await f.outbox.flushCriticalOutbox(f.state); const first = f.state.ws;
  for (let i = 0; i < 3; i++) f.ack(`critical-${i}`);
  await f.outbox.flushCriticalOutbox(f.state);
  assert.equal(f.flights(first).sent.size, 5);
  first.readyState = 3; f.state.ws = f.socket();
  await f.outbox.flushCriticalOutbox(f.state); await f.outbox.flushCriticalOutbox(f.state);
  assert.equal(first.sent.length, 8); assert.equal(f.state.ws.sent.length, 5);
  assert.equal(f.runtime.outbox.length, 5);
  for (const raw of f.state.ws.sent) {
    const envelope = JSON.parse(raw); assert.equal(protectedBytes(envelope), originals.get(envelope.messageId));
  }
});

test('direct critical sends register on their socket so flush cannot double-send them', async () => {
  const f = fixture();
  const body = { raw: '原文不改🦉', status: 'submission_unknown' };
  const envelope = await f.outbox.sendProtocolMessage(f.state, 'command.result', body, { messageId: 'direct-original', request: { requestId: 'original-request' } });
  const persisted = JSON.stringify(f.runtime.outbox);
  await f.outbox.flushCriticalOutbox(f.state); await f.outbox.replayCriticalOutbox(f.state);
  assert.equal(f.state.ws.sent.length, 1); assert.equal(f.flights().sent.size, 1);
  assert.equal(JSON.stringify(f.runtime.outbox), persisted);
  assert.equal(protectedBytes(JSON.parse(f.state.ws.sent[0])), protectedBytes(envelope));
});

test('a throwing socket send is not registered and the same durable task remains recoverable', async () => {
  const f = fixture(); f.state.ws.failNext = true;
  await assert.rejects(f.outbox.sendProtocolMessage(f.state, 'command.result', { raw: '原回包', status: 'submission_unknown' }, { messageId: 'original-failed' }), /synthetic-send-failure/);
  assert.equal(f.state.ws.sent.length, 0); assert.equal(f.flights().sent.size, 0); assert.equal(f.flights().sending.size, 0);
  assert.equal(f.runtime.outbox.length, 1); const protectedOriginal = protectedBytes(f.runtime.outbox[0]);
  f.state.ws.readyState = 3; f.state.ws = f.socket();
  await f.outbox.replayCriticalOutbox(f.state); await f.outbox.replayCriticalOutbox(f.state);
  assert.equal(f.state.ws.sent.length, 1); assert.equal(f.runtime.outbox.length, 1);
  assert.equal(protectedBytes(JSON.parse(f.state.ws.sent[0])), protectedOriginal);
});

test('noncritical messages do not enter the critical registry and continue to send normally', async () => {
  const f = fixture();
  for (let i = 0; i < 2; i++) await f.outbox.sendProtocolMessage(f.state, 'tab.observation', { same: true }, { messageId: 'not-critical' });
  assert.equal(f.state.ws.sent.length, 2); assert.equal(f.runtime.outbox.length, 0);
  assert.equal(f.flights(), undefined);
});

test('concurrent replay and direct send reserve one critical identity across persistence awaits', async () => {
  const f = fixture(1), gate = deferred(), reached = deferred();
  let blocked = false;
  f.setTransitionHook(async event => {
    if (event.type === 'outbox.resequenced' && !blocked) { blocked = true; reached.resolve(); await gate.promise; }
  });
  const replay = f.outbox.replayCriticalOutbox(f.state); await reached.promise;
  await f.outbox.sendProtocolMessage(f.state, 'command.result', clone(f.runtime.outbox[0].body), { messageId: 'critical-0', persisted: true, request: clone(f.runtime.outbox[0].request) });
  assert.equal(f.state.ws.sent.length, 0);
  gate.resolve(); await replay; await f.outbox.flushCriticalOutbox(f.state);
  assert.equal(f.state.ws.sent.length, 1); assert.equal(f.runtime.outbox.length, 1);
});

test('a stale durable read cannot erase a newer direct write flight', async () => {
  const f = fixture(), gate = deferred(), reached = deferred(); let once = false;
  f.setReadHook(async () => { if (!once) { once = true; reached.resolve(); await gate.promise; } });
  const replay = f.outbox.replayCriticalOutbox(f.state); await reached.promise;
  await f.outbox.sendProtocolMessage(f.state, 'command.result', { raw: 'newer-original' }, { messageId: 'newer-critical' });
  gate.resolve(); await replay; await f.outbox.flushCriticalOutbox(f.state);
  assert.equal(f.state.ws.sent.length, 1); assert.equal(f.flights().sent.size, 1);
});

test('socket replacement during resequencing never writes a stale attempt onto the replacement', async () => {
  const f = fixture(1), gate = deferred(), reached = deferred(); let once = false;
  f.setTransitionHook(async event => {
    if (event.type === 'outbox.resequenced' && !once) { once = true; reached.resolve(); await gate.promise; }
  });
  const first = f.state.ws, replay = f.outbox.replayCriticalOutbox(f.state); await reached.promise;
  first.readyState = 3; f.state.ws = f.socket(); gate.resolve(); await replay;
  assert.equal(first.sent.length, 0); assert.equal(f.state.ws.sent.length, 0);
  await f.outbox.replayCriticalOutbox(f.state); assert.equal(f.state.ws.sent.length, 1);
  assert.equal(f.runtime.outbox[0].messageId, 'critical-0');
});

test('failed persisted replay releases its reservation and preserves the original recovery identity', async () => {
  const f = fixture(1), original = protectedBytes(f.runtime.outbox[0]);
  f.state.ws.failNext = true;
  await assert.rejects(f.outbox.replayCriticalOutbox(f.state), /synthetic-send-failure/);
  assert.equal(f.flights().sent.size, 0); assert.equal(f.flights().sending.size, 0);
  assert.equal(f.runtime.outbox.length, 1); assert.equal(protectedBytes(f.runtime.outbox[0]), original);
  await f.outbox.replayCriticalOutbox(f.state); await f.outbox.replayCriticalOutbox(f.state);
  assert.equal(f.state.ws.sent.length, 1); assert.equal(protectedBytes(JSON.parse(f.state.ws.sent[0])), original);
});

test('a Protocol5 ACK during replay preparation prevents sending an entry already removed durably', async () => {
  const f = fixture(1);
  f.setTransitionHook(async event => { if (event.type === 'outbox.resequenced') f.ack(event.envelope.messageId); });
  await f.outbox.replayCriticalOutbox(f.state);
  assert.equal(f.state.ws.sent.length, 0); assert.equal(f.runtime.outbox.length, 0);
  assert.equal(f.flights().sent.size, 0); assert.equal(f.flights().sending.size, 0);
});
