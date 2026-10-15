const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const vm = require('node:vm');
const { pathToFileURL } = require('node:url');
const { createHash } = require('node:crypto');
const overlay = path.join(__dirname, '../overrides/extension/background');
const component = path.join(process.env.LOCALAPPDATA || 'C:/Users/EDY/AppData/Local', 'Programs/Director-3008-ChatGPT-Bridge/671815e/extension/background');
const beforeSha = 'c95ffc3741afd55643073c6a621ee6368e5c03c6f12cb85792a1fa8c31f23d91';
const OPERATION = 'ba655c90-7868-421e-a82d-5c5f461b21ab', COMMAND = 'reload-exact', EPOCH = 'content-original';
function compactor(source, observedAt) {
  const context = vm.createContext({ TextEncoder, Date: class extends Date { static now() { return observedAt; } } });
  vm.runInContext(source.replace(/export /g, '') + ';globalThis.compact=compactRuntimeState;', context);
  return state => context.compact(state, { aggressive: true, targetBytes: 256000 });
}
const source = fs.readFileSync(path.join(overlay, 'stateV6Compaction.js'), 'utf8');
function command(at = 1000000) {
  return { commandId: COMMAND, commandType: 'extension.reload', scope: 'standalone', mode: 'result', operation: 'maintenance', requestId: '', leaseId: '', ownerServerInstanceId: '',
    status: 'succeeded', registeredContentEpoch: EPOCH, dispatchedContentEpoch: EPOCH, createdAt: at - 100, dispatchedAt: at - 50, updatedAt: at,
    resultType: 'extension.reload.accepted', resultPayload: { commandId: COMMAND, requestId: '', resultType: 'extension.reload.accepted', maintenanceOperationId: OPERATION } };
}
function state(cmd) { return { commands: { [COMMAND]: cmd }, commandOrder: [COMMAND], effects: {}, effectOrder: [], downloads: {}, outbox: [], lease: null, journal: [], metrics: {} }; }

test('aggressive compaction preserves only the original exact reload command within its two-minute window', () => {
  const original = state(command());
  const compacted = compactor(source, 1000010)(original).state;
  assert.deepEqual(JSON.parse(JSON.stringify(compacted.commands[COMMAND])), original.commands[COMMAND]);
  assert.deepEqual(Object.keys(compacted.commands), [COMMAND]);
  assert.equal(Object.hasOwn(compacted, 'reloadReceipt'), false);
  assert.equal(compactor(source, 1120000)(original).state.commands[COMMAND].commandId, COMMAND);
  assert.equal(compactor(source, 1120001)(original).state.commands[COMMAND], undefined);
});

test('wrong maintenance operation contract time identity and query/generation commands receive no new pin', () => {
  const invalid = [
    c => c.commandId = 'other', c => c.commandType = 'prompt.send', c => c.scope = 'request', c => c.mode = 'effect', c => c.operation = 'control',
    c => c.requestId = 'generation', c => c.leaseId = 'lease', c => c.ownerServerInstanceId = 'server', c => c.status = 'rejected',
    c => c.resultType = 'query.completed', c => c.resultPayload.resultType = 'query.completed', c => c.resultPayload.commandId = 'other',
    c => c.resultPayload.maintenanceOperationId = 'wrong', c => c.resultPayload.maintenanceOperationId = '',
    c => c.createdAt = 0, c => c.dispatchedAt = '999950', c => c.updatedAt = 1000001, c => c.updatedAt = NaN,
    c => c.createdAt = c.dispatchedAt + 1, c => c.dispatchedContentEpoch = 'changed', c => c.registeredContentEpoch = '',
  ];
  for (const mutate of invalid) {
    const cmd = command(); mutate(cmd);
    assert.equal(compactor(source, 1000000)(state(cmd)).state.commands[COMMAND], undefined);
  }
  const query = command(); query.commandType = 'browser.query'; query.operation = 'read';
  assert.equal(compactor(source, 1000000)(state(query)).state.commands[COMMAND], undefined);
});

// These tests execute the real component Store, reducers and compaction code in
// a disposable module tree. No installed file, browser or production DB changes.
const realAvailable = fs.existsSync(path.join(component, 'stateV6Store.js'));
let moduleTree;
async function realModules() {
  if (moduleTree) return moduleTree;
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'director-maintenance-state-'));
  fs.writeFileSync(path.join(dir, 'package.json'), '{"type":"module"}');
  const seen = new Set();
  function copy(name) {
    if (seen.has(name)) return; seen.add(name);
    assert.match(name, /^stateV6[A-Za-z]+\.js$/);
    const text = name === 'stateV6Compaction.js' ? source : fs.readFileSync(path.join(component, name), 'utf8');
    for (const match of text.matchAll(/from\s+['"]\.\/([^'"]+)['"]/g)) copy(match[1]);
    fs.writeFileSync(path.join(dir, name), text);
  }
  copy('stateV6Store.js');
  const frozenFixture = fs.readFileSync(path.join(__dirname, 'fixtures/stateV6Compaction.upstream-2.4.5.js'), 'utf8').replace(/\r?\n/g, '\r\n');
  const originalCompaction = frozenFixture.split('// Fixture body follows; original code bytes are unchanged.\r\n')[1];
  assert.equal(typeof originalCompaction, 'string');
  assert.equal(createHash('sha256').update(originalCompaction).digest('hex'), beforeSha, 'update explicit upstream evidence before changing the fixture source');
  moduleTree = { dir, originalCompaction, ...(await import(pathToFileURL(path.join(dir, 'stateV6Store.js')).href)) };
  return moduleTree;
}
test.after(() => { if (moduleTree) fs.rmSync(moduleTree.dir, { recursive: true, force: true }); });
function envelope(type, id, sequence) { return { protocolVersion: 5, messageId: id, messageType: type, commandId: COMMAND, source: { clientId: 'source', tabId: 1, backgroundEpoch: 'original-bg', contentEpoch: EPOCH, sequence }, body: { commandId: COMMAND } }; }
async function integration() {
  const modules = await realModules(), local = {}, events = [], scheduled = [];
  let clock = Date.now(), rejectWrites = false;
  const storage = { get: async key => ({ [key]: structuredClone(local[key]) }), set: async patch => { if (rejectWrites) throw Error('synthetic quota capacity'); Object.assign(local, structuredClone(patch)); }, remove: async key => { delete local[key]; } };
  const store = new modules.BackgroundStateStore(storage, 'original-bg');
  const tx = event => store.transition(1, { ...event, contentEpoch: EPOCH });
  await tx({ type: 'command.registered', commandId: COMMAND, commandType: 'extension.reload', scope: 'standalone', mode: 'result', operation: 'maintenance' });
  const acceptance = envelope('command.accepted', 'accepted-exact', 1), terminal = envelope('command.result', 'terminal-exact', 2);
  await tx({ type: 'command.dispatched', commandId: COMMAND, acceptedEnvelope: acceptance });
  const context = vm.createContext({ URL, URLSearchParams, Math, Date: class extends Date { static now() { return clock; } }, console: { info() {}, warn() {}, error() {} },
    setTimeout(fn, delay) { scheduled.push(() => { clock += delay; fn(); }); return scheduled.length; },
    chrome: { tabs: { get: async () => ({ id: 1, url: 'https://chatgpt.com/', active: false, incognito: false }), query: async () => [] }, storage: { local: storage, session: { get: async () => ({}) } } } });
  vm.runInContext(fs.readFileSync(path.join(overlay, 'extensionReloadCoordinator.js'), 'utf8').replace(/export /g, '') + ';globalThis.make=createExtensionReloadCoordinator;', context);
  const c = context.make({ backgroundState: store, safeBridgeServerUrl: x => x, launchTokenPattern: /^bridge-/,
    readLaunchedTab: async () => null, rememberLaunchedTab: async () => {}, navigateTab: async () => events.push('navigate'), reloadTab: async () => events.push('page-reload'),
    maintenanceOperations: { plan: async () => ({ accepted: true, state: { active: { operationId: OPERATION } } }), dispatch: async () => ({ accepted: true }), fail: async (_id, error) => events.push(error.code) },
    reloadRuntime: () => events.push('runtime-reload'), scheduleRecoveryWake: async () => ({ armed: false }), ackTimeoutMs: 1000 });
  async function settle() {
    const outcome = await tx({ type: 'command.succeeded', commandId: COMMAND, resultType: 'extension.reload.accepted', resultPayload: command().resultPayload, terminalEnvelope: terminal });
    assert.equal(outcome.accepted, true);
  }
  async function ack(message) { return tx({ type: 'outbox.acknowledged', messageId: message.messageId, sequence: message.source.sequence }); }
  const schedule = () => c.scheduleExtensionReload({ sourceTabId: 1, commandId: COMMAND, reloadTabs: false });
  async function drain() { for (let i = 0; i < 100; i++) { await new Promise(setImmediate); if (!scheduled.length) return; scheduled.shift()(); } assert.fail('maintenance timer did not settle'); }
  return { modules, store, tx, events, schedule, settle, ack, acceptance, terminal, drain, rejectWrites: value => { rejectWrites = value; } };
}

test('real Store early acceptance ACK still permits original dispatched maintenance without a new receipt pipeline', { skip: !realAvailable }, async () => {
  const f = await integration(); await f.ack(f.acceptance); await f.schedule(); await f.drain();
  assert.equal(f.events.filter(e => e === 'runtime-reload').length, 1);
  assert.equal((await f.store.read(1)).commands[COMMAND].status, 'dispatched');
});

test('real Store terminal ACK compaction retains exact maintenance and coordinator reloads once after scheduling', { skip: !realAvailable }, async () => {
  const f = await integration(); await f.schedule(); await f.settle();
  await f.ack(f.acceptance); await f.ack(f.terminal);
  const actual = await f.store.read(1);
  assert.equal(actual.outbox.length, 0); assert.equal(actual.commands[COMMAND].status, 'succeeded');
  assert.equal(compactor(f.modules.originalCompaction, Date.now())(actual).state.commands[COMMAND], undefined, 'unpatched real aggressive policy drops this exact terminal command');
  await f.drain(); assert.equal(f.events.filter(e => e === 'runtime-reload').length, 1);
  assert.equal(f.events.includes('MAINTENANCE_ACK_TIMEOUT'), false);
});

test('failed real Store ACK persistence never removes the true pending acceptance or starts reload', { skip: !realAvailable }, async () => {
  const f = await integration(); await f.schedule(); await f.settle(); f.rejectWrites(true);
  await assert.rejects(f.ack(f.acceptance), error => error.code === 'BACKGROUND_STATE_PERSIST_FAILED');
  assert.ok((await f.store.read(1)).outbox.some(e => e.messageId === f.acceptance.messageId));
  f.rejectWrites(false); await f.drain(); assert.equal(f.events.includes('runtime-reload'), false); assert.ok(f.events.includes('MAINTENANCE_ACK_TIMEOUT'));
});

test('missing real command and empty outbox do not become an invented maintenance acceptance', { skip: !realAvailable }, async () => {
  const f = await integration();
  const runtime = await f.store.read(1); runtime.commands = {}; runtime.commandOrder = []; runtime.outbox = [];
  await f.schedule(); await f.drain(); assert.equal(f.events.includes('runtime-reload'), false); assert.ok(f.events.includes('MAINTENANCE_ACK_TIMEOUT'));
});

test('real Store cross-tab quota reclaim preserves the exact reload pin and still fails closed at hard capacity', { skip: !realAvailable }, async () => {
  const modules = await realModules(), removed = [], now = Date.now();
  const pinned = { schemaVersion: 6, tabId: 2, backgroundEpoch: 'original-bg', transport: {}, ...state(command(now)) };
  const generation = { schemaVersion: 6, tabId: 3, backgroundEpoch: 'original-bg', transport: {}, ...state({ ...command(now), commandType: 'prompt.send', operation: 'generation' }) };
  const local = { 'chatgptBridgeV6:tab:2': pinned, 'chatgptBridgeV6:tab:3': generation };
  const storage = {
    get: async key => key == null ? structuredClone(local) : { [key]: structuredClone(local[key]) },
    set: async () => { throw Error('synthetic storage quota exceeded'); },
    remove: async keys => { for (const key of [].concat(keys)) { removed.push(key); delete local[key]; } },
  };
  const store = new modules.BackgroundStateStore(storage, 'original-bg');
  await assert.rejects(store.transition(1, { type: 'transport.outbound.next' }), error => error.code === 'BACKGROUND_STATE_PERSIST_FAILED');
  assert.deepEqual(removed, ['chatgptBridgeV6:tab:3']);
  assert.equal(local['chatgptBridgeV6:tab:2'].commands[COMMAND].resultPayload.maintenanceOperationId, OPERATION);
  assert.equal(Object.keys(local['chatgptBridgeV6:tab:2'].commands).length, 1);
});

test('real Store idle cleanup reclaims expired reloads and ordinary generation while retaining only the current pin', { skip: !realAvailable }, async () => {
  const modules = await realModules(), at = Date.now(), local = {};
  for (const [tab, cmd] of [[1, command(at)], [2, command(at - 120001)], [3, { ...command(at), commandType: 'prompt.send', operation: 'generation' }]]) {
    local['chatgptBridgeV6:tab:' + tab] = { schemaVersion: 6, tabId: tab, backgroundEpoch: 'original-bg', transport: {}, ...state(cmd) };
  }
  const storage = { get: async key => key == null ? structuredClone(local) : { [key]: structuredClone(local[key]) }, set: async patch => Object.assign(local, patch), remove: async keys => { for (const key of [].concat(keys)) delete local[key]; } };
  const store = new modules.BackgroundStateStore(storage, 'original-bg');
  const cleaned = await store.cleanupIdleState();
  assert.deepEqual(cleaned.removed, ['chatgptBridgeV6:tab:2', 'chatgptBridgeV6:tab:3']);
  assert.deepEqual(Object.keys(local), ['chatgptBridgeV6:tab:1']);
  assert.equal(cleaned.retainedCritical, 1);
});
