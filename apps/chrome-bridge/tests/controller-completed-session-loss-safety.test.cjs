const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { createHash, webcrypto } = require('node:crypto');

const PROFILE = '11111111-1111-4111-8111-111111111111';
const SESSION = '22222222-2222-4222-8222-222222222222';
const PROFILE_KEY = 'directorDesktop:browserProfileId:v1';
const CONTROL_KEY = 'directorDesktop:controlTab:v1:' + PROFILE;
const JOURNAL_KEY = 'directorDesktop:controlReconnect:v1:' + PROFILE;
const SOURCE = fs.readFileSync(path.join(__dirname,
  '../overrides/extension/background/connectionWatchdog.js'), 'utf8');

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') return Object.fromEntries(
    Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  return value;
}

function fixture(change = () => {}) {
  const original = {
    status: 'opened', tabId: 7, browserSession: SESSION,
    launchToken: 'bridge-director-control-' + PROFILE,
    requestedUrl: 'https://chatgpt.com/', serverUrl: 'http://127.0.0.1:53179',
  };
  const control = {
    ...original, tabId: 31,
    retiredControllers: [{ ...original, retiredAt: 1000,
      reason: 'session_ownership_lost_old_page_untouched' }],
  };
  const journal = {
    version: 'director-owned-controller-bootstrap/v2', status: 'bootstrap_completed',
    at: 1000, originalController: original, browserSession: SESSION,
    foreground: false, generationCalled: false, zeroCreateOrderingRepair: null,
    incidentHistory: [], controllerSha256: createHash('sha256')
      .update(JSON.stringify(canonical(control))).digest('hex'),
    tabId: 31, windowId: 10, attempts: 1,
  };
  const local = { [PROFILE_KEY]: PROFILE, [CONTROL_KEY]: control, [JOURNAL_KEY]: journal };
  const session = {};
  const tabs = [{ id: 31, windowId: 10, url: 'https://chatgpt.com/',
    active: true, incognito: false, discarded: false }];
  const window = { id: 10, focused: false, state: 'minimized', incognito: false };
  const effects = [];
  const forbidden = label => async (...args) => {
    effects.push({ label, args: structuredClone(args) });
    throw new Error('Unexpected side effect: ' + label);
  };
  const storage = (data, area) => ({
    get: async keys => structuredClone(keys === null ? data : Object.fromEntries(
      (Array.isArray(keys) ? keys : [keys]).filter(key => Object.hasOwn(data, key))
        .map(key => [key, data[key]]))),
    set: forbidden(area + '.set'), remove: forbidden(area + '.remove'),
    clear: forbidden(area + '.clear'),
  });
  const browser = {
    storage: { local: storage(local, 'local'), session: storage(session, 'session') },
    tabs: {
      query: async () => structuredClone(tabs),
      create: forbidden('tabs.create'), update: forbidden('tabs.update'),
      reload: forbidden('tabs.reload'), remove: forbidden('tabs.remove'),
      sendMessage: forbidden('tabs.sendMessage'),
    },
    windows: {
      get: async id => { assert.equal(id, window.id); return structuredClone(window); },
      create: forbidden('windows.create'), update: forbidden('windows.update'),
      remove: forbidden('windows.remove'),
    },
    runtime: { reload: forbidden('runtime.reload'), sendMessage: forbidden('runtime.sendMessage') },
    scripting: { executeScript: forbidden('scripting.executeScript') },
  };
  const facts = { local, session, tabs, window, browser, effects, connections: new Map() };
  change(facts);
  const snapshot = () => structuredClone({ local, session, tabs, window });
  const before = snapshot();
  return { ...facts, before, snapshot,
    // New VM for each call models a fresh worker with no retained module memory.
    async run(epoch = 'worker-after-manual-reload') {
      const context = vm.createContext({ chrome: browser, crypto: webcrypto, TextEncoder,
        URL, URLSearchParams, Date, clearTimeout, WebSocket: { CONNECTING: 0, OPEN: 1 } });
      const recover = vm.runInContext(SOURCE.replace(/^export /gm, '') +
        '\nrecoverOwnedController;', context);
      return recover({ browser, connections: facts.connections,
        backgroundEpoch: epoch, now: () => 1000000 });
    },
  };
}

async function assertHeldWithoutEffects(f, expectedReason) {
  const result = await f.run();
  assert.equal(result.status, 'held');
  if (expectedReason) assert.equal(result.reason, expectedReason);
  assert.deepEqual(f.effects, [], 'no storage, browser, content or business mutation is permitted');
  assert.deepEqual(f.snapshot(), f.before, 'preserve the original journal and every unresolved state');
  return result;
}

test('completed bootstrap with all session ownership lost stays held across fresh workers', async () => {
  const f = fixture();
  assert.deepEqual(f.session, {});
  assert.equal(f.local[JOURNAL_KEY].controllerSha256, createHash('sha256')
    .update(JSON.stringify(canonical(f.local[CONTROL_KEY]))).digest('hex'));
  await assertHeldWithoutEffects(f, 'bootstrap_existing_reservation');
  const second = await f.run('another-fresh-worker');
  assert.equal(second.status, 'held');
  assert.deepEqual(f.effects, []);
  assert.deepEqual(f.snapshot(), f.before);
  assert.equal(f.local[JOURNAL_KEY].status, 'bootstrap_completed', 'creation is not a hello ACK');
});

test('the original hidden root and a same-ID replacement user root cannot authorize adoption', async () => {
  // These histories have identical evidence after session loss. Provenance is
  // intentionally not supplied to production: tab/window IDs and a root URL
  // cannot distinguish the original creation from a replacement user page.
  const originalWindow = fixture();
  const replacementUserWindow = fixture();
  assert.deepEqual(originalWindow.before, replacementUserWindow.before);
  for (const f of [originalWindow, replacementUserWindow]) {
    await assertHeldWithoutEffects(f, 'bootstrap_existing_reservation');
  }
});

test('completed journal never bypasses visible, shared, changed or differently numbered native windows', async t => {
  const variants = [
    ['foreground', f => { f.window.focused = true; f.window.state = 'normal'; }],
    ['shared', f => f.tabs.push({ id: 32, windowId: 10,
      url: 'https://example.com/user', active: false, incognito: false })],
    ['changed URL', f => { f.tabs[0].url = 'https://chatgpt.com/c/user-conversation'; }],
    ['different window', f => { f.tabs[0].windowId = 11; f.window.id = 11; }],
  ];
  for (const [name, change] of variants) await t.test(name, async () => {
    await assertHeldWithoutEffects(fixture(change));
  });
});

test('session loss never fabricates idle state or discards surviving lease and outbox evidence', async t => {
  for (const [name, change] of [
    ['lease', state => { state.lease = { requestId: 'synthetic-unresolved-request' }; }],
    ['outbox', state => { state.outbox.push({ messageId: 'synthetic-unacknowledged-envelope' }); }],
    ['uncertain command', state => { state.commands.pending = { status: 'uncertain' }; }],
  ]) await t.test(name, async () => {
    const f = fixture(facts => {
      const state = { schemaVersion: 6, lease: null, outbox: [], commands: {}, effects: {}, downloads: {} };
      change(state);
      facts.session['chatgptBridgeV6:tab:31'] = state;
    });
    await assertHeldWithoutEffects(f, 'bootstrap_execution_unresolved');
  });
});

test('unknown original creation remains reserved for reconciliation without another create', async t => {
  for (const status of ['reserved', 'outcome_unknown']) await t.test(status, async () => {
    const f = fixture(facts => { facts.local[JOURNAL_KEY].status = status; });
    await assertHeldWithoutEffects(f);
    const result = await f.run('yet-another-worker');
    assert.equal(result.status, 'held');
    assert.deepEqual(f.effects, []);
    assert.deepEqual(f.snapshot(), f.before);
  });
});
