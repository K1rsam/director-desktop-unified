const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function fixture({ rejectSchedule = false } = {}) {
  const listeners = new Set(), pageMessages = [], requests = [], results = [], diagnostics = [];
  const window = {
    addEventListener(type, handler) { assert.equal(type, 'message'); listeners.add(handler); },
    removeEventListener(type, handler) { assert.equal(type, 'message'); listeners.delete(handler); },
    postMessage(message) {
      pageMessages.push(structuredClone(message));
      if (message.type === 'page.reload.arm') queueMicrotask(() => {
        for (const handler of listeners) handler({ source: window, data: {
          source: 'chatgpt-browser-bridge-artifact-main-v1', type: 'page.reload.armed',
          reloadId: message.reloadId, delayMs: message.delayMs,
        } });
      });
    },
  };
  const context = vm.createContext({ window, Date, Math, setTimeout, clearTimeout,
    location: { href: 'https://chatgpt.com/' } });
  vm.runInContext(fs.readFileSync(path.join(__dirname,
    '../overrides/extension/content/sessionCommands.js'), 'utf8'), context);
  const commands = context.ChatGptSessionCommands.createSessionCommands({
    CONFIG: { serverUrl: 'http://127.0.0.1:53179' }, EXTENSION_VERSION: '2.4.5',
    CONTENT_SCRIPT_VERSION: '4.4.5', safeLaunchBridgeServerUrl: value => value,
    stageTemporaryConnectionOverride: () => ({ staged: false, reason: 'synthetic_saved_connection' }),
    async extensionRequest(type, payload) {
      requests.push({ type, payload: structuredClone(payload) });
      if (rejectSchedule) throw new Error('synthetic original schedule rejection');
      return { operationId: 'synthetic-maintenance-operation', trampolinePlannedCount: 1 };
    },
    send: payload => results.push(structuredClone(payload)),
    diagnostic: (...args) => diagnostics.push(args),
  });
  return { commands, pageMessages, requests, results, diagnostics, listeners };
}

test('owned-only maintenance delegates page recovery to its durable background without arming an early timer', async () => {
  const f = fixture();
  await f.commands.handleExtensionReload({ commandId: 'synthetic-command', reloadTabs: true,
    ownedTabsOnly: true, sourceTabId: 7, sourceLaunchToken: 'bridge-synthetic-owned' });
  assert.deepEqual(f.pageMessages, []);
  assert.equal(f.requests.length, 1);
  assert.equal(f.requests[0].type, 'bridge.extension.reload');
  assert.equal(f.requests[0].payload.reloadTabs, true, 'background must retain the original page recovery plan');
  assert.equal(f.requests[0].payload.ownedTabsOnly, true);
  assert.equal(f.results[0].pageReload.armed, false);
  assert.equal(f.results[0].pageReload.reason, 'owned_background_recovery');
  assert.equal(f.results[0].maintenanceOperationId, 'synthetic-maintenance-operation');
  assert.equal(f.results[0].reloadTrampoline.planned, true);
  assert.equal(f.listeners.size, 0);
});

test('non-owned legacy maintenance still arms the configured page timer and reports its actual acknowledgement', async () => {
  const f = fixture();
  await f.commands.handleExtensionReload({ commandId: 'synthetic-command', reloadTabs: true,
    ownedTabsOnly: false, pageReloadDelayMs: 3500 });
  assert.equal(f.pageMessages.length, 1);
  assert.equal(f.pageMessages[0].type, 'page.reload.arm');
  assert.equal(f.pageMessages[0].delayMs, 3500);
  assert.equal(f.results[0].pageReload.armed, true);
  assert.equal(f.results[0].pageReload.delayMs, 3500);
  assert.equal(f.listeners.size, 0);
});

test('source-only maintenance keeps timers disabled and a rejected owned schedule never reports success', async () => {
  const sourceOnly = fixture();
  await sourceOnly.commands.handleExtensionReload({ commandId: 'synthetic-command', reloadTabs: false });
  assert.deepEqual(sourceOnly.pageMessages, []);
  assert.equal(sourceOnly.results[0].pageReload.reason, 'tabs_not_reloaded');
  const rejected = fixture({ rejectSchedule: true });
  await assert.rejects(rejected.commands.handleExtensionReload({ commandId: 'synthetic-command',
    reloadTabs: true, ownedTabsOnly: true }), /synthetic original schedule rejection/);
  assert.deepEqual(rejected.pageMessages, []);
  assert.deepEqual(rejected.results, []);
});

test('a rejected non-owned schedule cancels its original legacy page timer', async () => {
  const f = fixture({ rejectSchedule: true });
  await assert.rejects(f.commands.handleExtensionReload({ commandId: 'synthetic-command',
    reloadTabs: true, ownedTabsOnly: false }), /synthetic original schedule rejection/);
  assert.deepEqual(f.pageMessages.map(message => message.type), ['page.reload.arm', 'page.reload.cancel']);
  assert.equal(f.pageMessages[1].reloadId, f.pageMessages[0].reloadId);
  assert.deepEqual(f.results, []);
});
