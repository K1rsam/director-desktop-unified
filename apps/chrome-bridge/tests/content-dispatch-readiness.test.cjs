const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../overrides/extension/background/serverEnvelopeRouter.js'), 'utf8');
function fixture({ ready = false, closed = false } = {}) {
  const messages = [], transitions = [], posts = [];
  const definition = { mode: 'result', operation: 'maintenance', retryPolicy: 'never', reconcile: 'background_epoch' };
  const context = vm.createContext({
    MessageType: { COMMAND_EXECUTE: 'command.execute', COMMAND_REJECTED: 'command.rejected', COMMAND_ACCEPTED: 'command.accepted', TRANSPORT_ACK: 'transport.ack' },
    ChatGptBridgeCommandManifest: { commandDefinition: () => definition, validateCommandPayload: () => ({ valid: true, definition }) },
  });
  vm.runInContext(source.replace(/^import .*;\r?\n/gm, '').replace(/export /g, '') + ';this.handle=handleServerEnvelope;', context);
  const state = { tabId: 7, contentEpoch: 'original-content', clientId: 'extension:tab:7', protocolReady: ready, closed, backgroundHandshakeReady: true, connectionEpoch: 'same-connection', port: {} };
  const runtime = { transport: { serverEpoch: 'original-server' }, commands: { original: { commandId: 'original', commandType: 'extension.reload', scope: 'standalone', status: 'dispatched' } }, commandOrder: ['original'], lease: null, effects: { image: { status: 'uncertain', effectId: 'original-image' } }, outbox: [] };
  const protectedBefore = JSON.stringify([runtime.commands, runtime.effects, runtime.lease]);
  const backgroundState = {
    read: async () => runtime,
    transition: async (_tab, event) => { transitions.push(event); return { accepted: true, state: runtime }; },
  };
  let sequence = 0;
  const execute = (type = 'extension.reload', request = null) => context.handle({
    state, envelope: { messageId: 'new-envelope', messageType: 'command.execute', commandId: 'new-command', request, source: { backgroundEpoch: 'original-server', sequence: ++sequence }, body: { type, commandId: 'new-command' } },
    backgroundState, sendProtocolMessage: async (_state, type, body) => messages.push({ type, body }),
    createEnvelopeDraft: (_state, type, body) => ({ type, body }), flushCriticalOutbox: async () => {}, post: (_port, body) => posts.push(body),
  });
  return { context, state, runtime, messages, transitions, posts, execute, backgroundState, protectedBefore };
}

test('bootstrap-only Native connection rejects content commands without acceptance, registration, lease or page effects', async () => {
  for (const type of ['extension.reload', 'browser.tab.open', 'debug.layout.capture', 'response.recover.latest', 'command.cancel', 'prompt.send']) {
    const f = fixture();
    await f.execute(type, type === 'prompt.send' ? { requestId: 'new-image', leaseId: 'new-lease', ownerServerInstanceId: 'server', responseEpoch: 1 } : null);
    assert.equal(f.messages.length, 1, type);
    assert.equal(f.messages[0].type, 'command.rejected');
    assert.equal(f.messages[0].body.code, 'CONTENT_RUNTIME_NOT_READY');
    assert.equal(f.messages[0].body.commandId, 'new-command');
    assert.equal(f.transitions.some(e => e.type.startsWith('command.') || e.type.startsWith('lease.') || e.type.startsWith('effect.')), false);
    assert.equal(f.posts.length, 0);
    assert.equal(JSON.stringify([f.runtime.commands, f.runtime.effects, f.runtime.lease]), f.protectedBefore);
  }
});

test('missing real page handshake and closed runtime cannot be mistaken for an acknowledged content runtime', async () => {
  for (const flags of [{ ready: undefined }, { ready: true, closed: true }]) {
    const f = fixture(flags); if (flags.ready === undefined) delete f.state.protocolReady;
    await f.execute();
    assert.equal(f.messages[0].body.code, 'CONTENT_RUNTIME_NOT_READY');
    assert.equal(f.posts.length, 0);
  }
});

test('original dispatched maintenance remains protected when the real content receiver exists', async () => {
  const f = fixture({ ready: true }); await f.execute();
  assert.equal(f.messages[0].body.code, 'BROWSER_COMMAND_REJECTED');
  assert.match(f.messages[0].body.message, /original/);
  assert.equal(JSON.stringify([f.runtime.commands, f.runtime.effects, f.runtime.lease]), f.protectedBefore);
});

test('ready content follows normal standalone registration and dispatch exactly once', async () => {
  const f = fixture({ ready: true }); f.runtime.commands = {}; f.runtime.commandOrder = [];
  await f.execute();
  assert.deepEqual(f.transitions.map(e => e.type), ['transport.inbound', 'command.registered', 'command.dispatched']);
  assert.equal(f.posts.length, 1); assert.equal(f.posts[0].payload.commandId, 'new-command');
  assert.equal(f.messages.length, 0);
});

test('bootstrap ACK still drains the original durable outbox without a page receiver or new commands', async () => {
  const f = fixture();
  await f.context.handle({ state: f.state, envelope: { messageType: 'transport.ack', source: { backgroundEpoch: 'original-server', sequence: 1 }, body: { accepted: true, ackMessageId: 'original-envelope', acceptedSequence: 4 } },
    backgroundState: f.backgroundState, flushCriticalOutbox: async () => {}, post: () => assert.fail('no page input') });
  assert.deepEqual(f.transitions.map(e => e.type), ['transport.inbound', 'outbox.acknowledged']);
  assert.equal(f.transitions[1].messageId, 'original-envelope');
  assert.equal(JSON.stringify([f.runtime.commands, f.runtime.effects, f.runtime.lease]), f.protectedBefore);
});
