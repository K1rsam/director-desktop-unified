import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';
import { createManager, validateRequest, bridgeIdle, sessionIdle, processMatches, verifyListeners, parseActivePort, componentIdentity, safePath, configureController, isControllerRoot, managedPresentation, readManagedPage, inspectController } from './managed-browser-bridge.mjs';

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'director-managed-edge-test-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const home = path.join(root, 'home'), componentHome = path.join(root, 'component'), ext = path.join(componentHome, 'extension');
  fs.mkdirSync(path.join(ext, 'shared'), { recursive: true });
  fs.mkdirSync(path.join(ext, 'content'));
  fs.writeFileSync(path.join(ext, 'manifest.json'), JSON.stringify({ name: 'ChatGPT Browser Bridge', manifest_version: 3, version: '2.4.5', background: { service_worker: 'background.js' }, key: Buffer.from('synthetic key').toString('base64'), permissions: ['storage'] }));
  fs.writeFileSync(path.join(ext, 'shared/buildIdentity.js'), "({bundleId:'11111111111111111111111111111111'})");
  fs.writeFileSync(path.join(ext, 'content/automationConfig.js'), "({serverUrl:'',token:''})");
  const request = { schemaVersion: 'managed-browser-request/v1', action: 'status', home, componentHome, connection: { endpoint: 'http://127.0.0.1:55123', apiToken: 'private-api-value', bridgeToken: 'private-bridge-value' } };
  return { root, home, componentHome, ext, request };
}
const idleBridge = () => ({ health: { ok: false, pendingRequests: 0, pendingCommands: 0, clients: 0, activeRequests: [], activeClient: null, transport: 'extension:disconnected', needsSelection: false }, listing: { ok: true, clients: [] } });
const effect = () => ({ verified: true, activeJobs: 0, unknownJobs: 0 });
const native = () => ({ pid: process.pid, created: 'synthetic-process-start' });
const defaults = () => ({ findEdge: () => path.resolve('fake-edge.exe'), processInfo: native, profileProcesses: () => [], bridgeSnapshot: async () => idleBridge() });

test('status inspects without creating a home, profile, runtime, state or lock', async t => {
  const f = fixture(t);
  const result = await createManager(defaults())(f.request);
  assert.equal(result.state, 'not_started');
  assert.equal(result.running, false);
  assert.equal(fs.existsSync(f.home), false);
  assert.equal(result.modelSubmissions, 0);
});

test('uninstalled and malformed requests still return complete independent public flags', async t => {
  const f = fixture(t);
  for (const raw of [f.request, null]) {
    const result = await createManager({ findEdge: () => null })(raw);
    for (const field of ['installed', 'running', 'connected', 'authenticated', 'ready', 'extensionReady']) assert.equal(typeof result[field], 'boolean', field);
    assert.equal(typeof result.reason, 'string'); assert.equal(typeof result.message, 'string');
    assert.equal(result.videoSubmissions, 0); assert.equal(result.newBinding, false);
  }
});

test('credentials cannot leave the helper through an error message', async t => {
  const f = fixture(t);
  const result = await createManager({ ...defaults(), bridgeSnapshot: async () => { throw Error(f.request.connection.apiToken + f.request.connection.bridgeToken); } })(f.request);
  assert.equal(result.errorCode, 'MANAGED_BROWSER_OPERATION_FAILED');
  assert.doesNotMatch(JSON.stringify(result), /private-api-value|private-bridge-value/);
});

test('lifecycle requests require explicit clean app jobs, never coerce counts', t => {
  const { request } = fixture(t);
  for (const safety of [null, {}, { verified: true, activeJobs: 0 }, { verified: true, activeJobs: '0', unknownJobs: 0 }, { verified: true, activeJobs: 0, unknownJobs: 1 }]) {
    assert.throws(() => validateRequest({ ...request, action: 'recover', safety }), /JOBS_NOT_PROVED_IDLE/);
  }
});

test('only explicitly authorized login may produce visible Edge', t => {
  const { request } = fixture(t);
  assert.throws(() => validateRequest({ ...request, action: 'open-login', safety: effect() }), /VISIBLE_LOGIN_NOT_AUTHORIZED/);
  assert.throws(() => validateRequest({ ...request, action: 'start-background', safety: effect(), visibleLogin: true }), /UNEXPECTED_VISIBLE_LOGIN/);
  assert.equal(validateRequest({ ...request, action: 'open-login', safety: effect(), visibleLogin: true }).action, 'open-login');
});

test('bridge origin never accepts remote addresses, redirect paths, userinfo or queries', t => {
  const { request } = fixture(t);
  for (const endpoint of ['https://127.0.0.1:55123', 'http://example.com:55123', 'http://127.0.0.1:55123/redirect', 'http://user:pass@127.0.0.1:55123', 'http://127.0.0.1:55123/?token=secret', 'http://localhost:55123']) {
    assert.throws(() => validateRequest({ ...request, connection: { ...request.connection, endpoint } }), /INVALID_BRIDGE_ORIGIN/);
  }
});

test('missing bridge counters, pending commands and unknown clients prevent lifecycle changes', () => {
  const { health, listing } = idleBridge();
  assert.equal(bridgeIdle(health, listing), true);
  for (const changed of [{ ...health, pendingCommands: undefined }, { ...health, pendingRequests: 1 }, { ...health, pendingCommands: 1 }]) assert.equal(bridgeIdle(changed, listing), false);
  for (const client of [{ activeRequest: { requestId: 'old' } }, { releasePending: true }, { transportHealth: { tabQueue: { pending: 1 } } }]) assert.equal(bridgeIdle(health, { ok: true, clients: [client] }), false);
});

test('real zero-client health is idle while unavailable or malformed responses fail closed', () => {
  const { health, listing } = idleBridge();
  assert.equal(health.ok, false); assert.equal(bridgeIdle(health, listing), true);
  for (const response of [null, {}, { ...health, ok: undefined }, { ...health, clients: 1 }, { ...health, activeRequests: undefined }, { ...health, activeRequests: [{ id: 'unknown' }] }, { ...health, pendingCommands: undefined }]) assert.equal(bridgeIdle(response, listing), false);
  assert.equal(bridgeIdle(health, { ok: false, clients: [] }), false);
});

test('extension session leases, uncertain effects, outbox and downloads remain protected', () => {
  const idle = { schemaVersion: 6, lease: null, outbox: [], commands: {}, effects: {}, downloads: {} };
  assert.equal(sessionIdle({ 'chatgptBridgeV6:tab:1': idle }), true);
  for (const change of [{ lease: { id: 'lease' } }, { outbox: [{}] }, { effects: { one: { status: 'uncertain' } } }, { commands: { one: { status: 'dispatched' } } }, { downloads: { one: { status: 'completed' } } }, { schemaVersion: 5 }]) assert.equal(sessionIdle({ 'chatgptBridgeV6:tab:1': { ...idle, ...change } }), false);
  assert.equal(sessionIdle({}, { bridgePendingExtensionReload: { recoveryStatus: 'held' } }), false);
});

test('pending bridge work prevents spawning a browser and releases only the acquired lock', async t => {
  const f = fixture(t); let spawned = 0;
  const run = createManager({ ...defaults(), bridgeSnapshot: async () => ({ ...idleBridge(), health: { ok: true, pendingRequests: 1, pendingCommands: 0 } }), spawn: () => { spawned++; throw Error('must not spawn'); } });
  const result = await run({ ...f.request, action: 'start-background', safety: effect() });
  assert.equal(result.errorCode, 'BRIDGE_PENDING_WORK'); assert.equal(spawned, 0);
  assert.equal(fs.existsSync(path.join(f.home, 'managed-edge/lifecycle.lock')), false);
  assert.equal(fs.existsSync(path.join(f.home, 'managed-edge/profile')), false);
});

test('an unrecorded process using the dedicated profile is held without attaching or killing', async t => {
  const f = fixture(t); let attached = 0;
  const result = await createManager({ ...defaults(), profileProcesses: () => [{ pid: 9 }], attachOwned: async () => { attached++; } })(f.request);
  assert.equal(result.errorCode, 'UNRECORDED_PROFILE_PROCESS'); assert.equal(attached, 0);
  assert.equal(fs.existsSync(f.home), false);
});

test('active lifecycle lock is retained and never stolen merely because of age', async t => {
  const f = fixture(t), dir = path.join(f.home, 'managed-edge'); fs.mkdirSync(dir, { recursive: true });
  const lock = { pid: process.pid, created: 'synthetic-process-start', at: '2000-01-01' };
  fs.writeFileSync(path.join(dir, 'lifecycle.lock'), JSON.stringify(lock));
  const result = await createManager(defaults())({ ...f.request, action: 'recover', safety: effect() });
  assert.equal(result.errorCode, 'LIFECYCLE_LOCK_HELD');
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(dir, 'lifecycle.lock'))), lock);
});

test('old PID with a changed creation time cannot claim the owned Edge process', () => {
  const record = { pid: 123, created: 'start-1', executable: path.resolve('edge.exe'), profilePath: path.resolve('managed/profile'), instanceId: 'id-one' };
  const actual = { ...record, commandLine: `edge --user-data-dir=${record.profilePath} --director-managed-instance=id-one` };
  assert.equal(processMatches(record, actual), true);
  for (const change of [{ created: 'start-2' }, { pid: 456 }, { commandLine: 'edge' }, { executable: path.resolve('other.exe') }]) assert.equal(processMatches(record, { ...actual, ...change }), false);
});

test('CDP must be a loopback listener owned by exactly the recorded process', () => {
  assert.doesNotThrow(() => verifyListeners([{ LocalAddress: '127.0.0.1', OwningProcess: 2 }], 2));
  for (const rows of [[], [{ LocalAddress: '0.0.0.0', OwningProcess: 2 }], [{ LocalAddress: '127.0.0.1', OwningProcess: 3 }]]) assert.throws(() => verifyListeners(rows, 2), /CDP_LISTENER_NOT_OWNED_LOOPBACK/);
});

test('DevTools file cannot redirect control to remote or non-browser endpoints', () => {
  assert.deepEqual(parseActivePort('55555\n/devtools/browser/abcd-1234\n'), { port: 55555, browserPath: '/devtools/browser/abcd-1234' });
  for (const text of ['80\n/devtools/browser/abc', '55555\nws://attacker/a', '55555\n/devtools/page/abc', 'NaN\n/devtools/browser/abc']) assert.throws(() => parseActivePort(text), /CDP_IDENTITY_INVALID/);
});

test('component scripts may not embed production credentials', t => {
  const f = fixture(t);
  assert.match(componentIdentity(f.componentHome).extensionId, /^[a-p]{32}$/);
  fs.writeFileSync(path.join(f.ext, 'content/automationConfig.js'), "({serverUrl:'',token:'private'})");
  assert.throws(() => componentIdentity(f.componentHome), /EXTENSION_CONFIGURATION_UNSAFE/);
});

test('junctions or symlinks cannot redirect a profile outside its app home', t => {
  const f = fixture(t), link = path.join(f.root, 'link');
  fs.symlinkSync(f.componentHome, link, process.platform === 'win32' ? 'junction' : 'dir');
  assert.throws(() => safePath(path.join(link, 'extension/new-dir')), /LINKED_PATH_REJECTED/);
});

test('a persisted profile identity from another home is rejected before native attach', async t => {
  const f = fixture(t), dir = path.join(f.home, 'managed-edge'); fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'instance.json'), JSON.stringify({ schemaVersion: 'managed-edge-instance/v1', instanceId: '11111111-1111-4111-8111-111111111111', mode: 'headless', profilePath: path.join(f.root, 'other-profile'), extension: f.ext }));
  const result = await createManager(defaults())(f.request);
  assert.equal(result.errorCode, 'SAVED_IDENTITY_MISMATCH');
});

test('a reused old PID does not attach to the unrelated process or permanently block status', async t => {
  const f = fixture(t), dir = path.join(f.home, 'managed-edge'); fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'instance.json'), JSON.stringify({ schemaVersion: 'managed-edge-instance/v1', instanceId: '11111111-1111-4111-8111-111111111111', mode: 'headless', profilePath: path.join(dir, 'profile'), extension: f.ext, pid: 123, created: 'prior-start', executable: path.resolve('edge.exe') }));
  let attached = false;
  const result = await createManager({ ...defaults(), processInfo: () => ({ pid: 123, created: 'unrelated-start', executable: path.resolve('unrelated.exe') }), attachOwned: () => { attached = true; throw Error('never'); } })(f.request);
  assert.equal(result.state, 'not_started'); assert.equal(attached, false);
});

test('slow controller navigation retains its preload until the expected origin and config are proven', async t => {
  const f = fixture(t), state = path.join(f.root, 'state.json'), calls = []; let originReads = 0;
  const endpoint = f.request.connection.endpoint;
  const cdp = { evaluate: async expression => {
    if (expression === 'location.origin') return ++originReads < 4 ? 'about:blank' : 'https://chatgpt.com';
    return { endpoint: JSON.stringify(endpoint), marker: JSON.stringify('__chatgpt_bridge_secret_in_extension_storage_v1__') };
  }, send: async (method, params) => { calls.push({ method, params, originReads }); } };
  const result = await configureController(cdp, { controller: { targetId: 'original' } }, endpoint, state, { session: 'session', preload: 'preload', sleep: async () => {} });
  assert.equal(result.controller.configured, true);
  assert.deepEqual(calls.map(c => c.method), ['Page.removeScriptToEvaluateOnNewDocument']);
  assert.equal(calls[0].originReads, 4);
});

test('an uncommitted navigation is bounded and preserves the same controller without reload or new page', async t => {
  const f = fixture(t), calls = [];
  const cdp = { evaluate: async () => 'about:blank', send: async method => calls.push(method) };
  const result = await configureController(cdp, { controller: { targetId: 'original' } }, f.request.connection.endpoint, path.join(f.root, 'state.json'), { session: 'session', preload: 'preload', sleep: async () => {} });
  assert.equal(result.controller.targetId, 'original'); assert.equal(result.controller.configured, false);
  assert.deepEqual(calls, []);
});

test('late configuration repairs only the original idle controller and reloads once after readback', async t => {
  const f = fixture(t), calls = []; let configured = false;
  const cdp = { attach: async target => { assert.equal(target, 'original'); return 'session'; }, evaluate: async expression => {
    if (expression === 'location.origin') return 'https://chatgpt.com';
    if (expression.startsWith('if(location.origin')) { assert.doesNotMatch(expression, /private-api-value|private-bridge-value/); configured = true; return; }
    return configured ? { endpoint: JSON.stringify(f.request.connection.endpoint), marker: JSON.stringify('__chatgpt_bridge_secret_in_extension_storage_v1__') } : { endpoint: null, marker: null };
  }, send: async method => calls.push(method) };
  const result = await configureController(cdp, { controller: { targetId: 'original' } }, f.request.connection.endpoint, path.join(f.root, 'state.json'));
  assert.equal(result.controller.configured, true); assert.deepEqual(calls, ['Page.reload']);
});

test('controller roots accept site query parameters without accepting conversation paths or foreign origins', () => {
  for (const url of ['https://chatgpt.com/', 'https://chatgpt.com/?__cf_chl_rt_tk=opaque', 'https://chatgpt.com/?temporary-chat=true#section']) assert.equal(isControllerRoot(url), true);
  for (const url of ['https://chatgpt.com/c/abc', 'https://chatgpt.com/auth/login', 'https://chatgpt.com.example.org/', 'https://user:secret@chatgpt.com/', 'http://chatgpt.com/', 'https://chatgpt.com:8443/']) assert.equal(isControllerRoot(url), false);
});

test('connection, site verification, login and generation readiness remain independent', () => {
  const base = { connected: true, authenticated: false, authState: 'unknown', visibility: { visibilityState: 'hidden', focused: false }, pageReady: false };
  assert.equal(managedPresentation({ ...base, pageState: 'site_verification' }, 'headless').state, 'site_check_required');
  assert.equal(managedPresentation({ ...base, pageState: 'loading' }, 'headless').state, 'page_loading');
  assert.equal(managedPresentation({ ...base, authState: 'signed_out', pageState: 'login_required' }, 'headless').state, 'login_required');
  assert.equal(managedPresentation({ ...base, pageState: 'login_in_progress' }, 'visible').state, 'login_window_open');
  const authenticated = { ...base, authenticated: true, authState: 'signed_in', pageState: 'ready' };
  assert.equal(managedPresentation(authenticated, 'headless').ready, false);
  assert.equal(managedPresentation({ ...authenticated, pageReady: true }, 'visible').ready, false);
  assert.equal(managedPresentation({ ...authenticated, pageReady: true }, 'headless').ready, true);
  assert.equal(managedPresentation({ ...authenticated, pageReady: true, connected: false }, 'headless').ready, false);
});

function pageEnvironment({ url = 'https://chatgpt.com/', title = '', loading = false, composer = false, challenge = false, main = false, response = { ok: true, json: async () => ({}) } } = {}) {
  let fetches = 0;
  const context = { URL, AbortSignal, location: new URL(url), document: { title, readyState: loading ? 'loading' : 'complete', visibilityState: 'hidden', hasFocus: () => false, querySelector: selector => selector.startsWith('#prompt-textarea') ? composer ? { disabled: false, getAttribute: () => null } : null : selector.startsWith('main,') ? main ? {} : null : challenge ? {} : null }, fetch: async () => { fetches++; return response; } };
  return { inspect: async () => await vm.runInNewContext(`(${readManagedPage.toString()})()`, context), fetches: () => fetches };
}

test('site challenge is reported from existing page evidence without making an auth request or bypass attempt', async () => {
  for (const settings of [{ url: 'https://chatgpt.com/?__cf_chl_rt_tk=opaque' }, { challenge: true }, { title: 'Just a moment...' }]) {
    const page = pageEnvironment(settings), result = await page.inspect();
    assert.equal(result.pageState, 'site_verification'); assert.equal(result.authState, 'unknown');
    assert.equal(result.authenticated, false); assert.equal(page.fetches(), 0);
    assert.doesNotMatch(JSON.stringify(result), /opaque/);
  }
});

test('loading pages do not produce a false logged-out or recovery error', async () => {
  const page = pageEnvironment({ loading: true }), result = await page.inspect();
  assert.equal(result.pageState, 'loading'); assert.equal(result.authState, 'unknown'); assert.equal(page.fetches(), 0);
});

test('read-only session check returns only authentication booleans and never token or user contents', async () => {
  const page = pageEnvironment({ composer: true, main: true, response: { ok: true, json: async () => ({ user: { id: 'private-account-id' }, accessToken: 'private-access-token' }) } });
  const result = await page.inspect();
  assert.equal(result.pageState, 'ready'); assert.equal(result.authenticated, true); assert.equal(result.pageReady, true);
  assert.doesNotMatch(JSON.stringify(result), /private-account-id|private-access-token/);
  const signedOut = await pageEnvironment({ response: { ok: false, status: 401 } }).inspect();
  assert.equal(signedOut.authState, 'signed_out'); assert.equal(signedOut.pageState, 'login_required');
  const denied = await pageEnvironment({ response: { ok: false, status: 403 } }).inspect();
  assert.equal(denied.authState, 'unknown'); assert.equal(denied.pageState, 'authentication_unverified');
});

test('a verified bridge client on a root challenge query is connected but cannot generate', async () => {
  const profileId = '11111111-1111-4111-8111-111111111111', record = { mode: 'headless', profileId, controller: { targetId: 'own', tabId: 7 } };
  const component = { bundleId: 'bundle' }, url = 'https://chatgpt.com/?__cf_chl_rt_tk=opaque';
  const cdp = { evaluate: async (_expression, session) => session === 'worker' ? { profileId } : { pageState: 'site_verification', authState: 'unknown', authenticated: false, pageReady: false }, targets: async () => [{ targetId: 'own', url }], attach: async () => 'page' };
  const client = { id: 'client', browserProfileId: profileId, extensionBundleId: 'bundle', browserTabId: 7, launchToken: 'bridge-director-control-' + profileId, ready: true, browserIncognito: false, transport: 'extension', url, pageReady: false, composerReady: false, chatMainReady: false };
  const result = await inspectController(cdp, { sessionId: 'worker' }, record, component, {}, { bridgeSnapshot: async () => ({ listing: { clients: [client] } }) });
  assert.equal(result.connected, true); assert.equal(result.pageReady, false); assert.equal(result.pageState, 'site_verification');
});

test('visible login redirects are preserved without inspecting an external identity-provider page', async () => {
  const profileId = '11111111-1111-4111-8111-111111111111'; let attached = false;
  const cdp = { evaluate: async () => ({ profileId }), targets: async () => [{ targetId: 'own', url: 'https://auth.openai.com/log-in' }], attach: async () => { attached = true; } };
  const result = await inspectController(cdp, { sessionId: 'worker' }, { mode: 'visible', profileId, controller: { targetId: 'own' } }, {}, {}, {});
  assert.equal(result.pageState, 'login_in_progress'); assert.equal(result.authState, 'unknown'); assert.equal(attached, false);
  assert.equal(managedPresentation(result, 'visible').state, 'login_window_open');
});
