// App-owned Edge lifecycle only. This module never submits prompts, selects a
// bridge client, copies login data, or changes an existing project's binding.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash, randomUUID } from 'node:crypto';
import { spawn, execFileSync } from 'node:child_process';

const PROFILE_KEY = 'directorDesktop:browserProfileId:v1';
const SESSION_KEY = 'directorDesktop:controllerBrowserSession:v1';
const SECRET_KEY = 'chatgptBridge:secret:bridge.token';
const SECRET_MARKER = '__chatgpt_bridge_secret_in_extension_storage_v1__';
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const actions = new Set(['status', 'start-background', 'open-login', 'recover', 'stop']);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const sha = value => createHash('sha256').update(value).digest('hex');
const samePath = (a, b) => path.resolve(a).toLowerCase() === path.resolve(b).toLowerCase();
const fail = code => { throw new Error(code); };

// Query parameters from normal site routing/verification do not change the
// controller's identity. Conversations, other paths and origins still do.
export function isControllerRoot(value) {
  try { const url = new URL(value); return url.origin === 'https://chatgpt.com' && url.pathname === '/' && !url.username && !url.password; }
  catch { return false; }
}
function isChatGptPage(value) {
  try { const url = new URL(value); return url.origin === 'https://chatgpt.com' && !url.username && !url.password; }
  catch { return false; }
}

export function managedPresentation(report, mode) {
  const ready = report.connected === true && report.authenticated === true && report.pageReady === true && report.pageState === 'ready'
    && report.visibility?.visibilityState === 'hidden' && report.visibility?.focused === false && mode === 'headless';
  const result = { ready, generationReady: ready, requiresLogin: report.authState === 'signed_out', state: 'bridge_connecting', reason: '专属浏览器已启动，正在等待扩展建立连接。' };
  if (ready) return { ...result, state: 'ready', reason: '专属后台浏览器已连接，登录和生成页面已就绪。' };
  if (report.pageState === 'site_verification') return { ...result, state: 'site_check_required', reason: mode === 'visible' ? '正在显示 ChatGPT 官方站点校验，请在已打开的专属 Edge 中正常完成校验。' : 'ChatGPT 正在等待站点校验；请打开专属 Edge 登录页完成官方校验。' };
  if (report.pageState === 'loading') return { ...result, state: 'page_loading', reason: report.connected ? '扩展已连接，ChatGPT 页面仍在加载。' : '专属 Edge 已启动，正在加载 ChatGPT 页面。' };
  if (report.pageState === 'page_unavailable') return { ...result, state: 'page_unavailable', reason: '专属页面尚不可读取，请检查网络或打开登录页查看。' };
  if (mode === 'visible') return { ...result, state: 'login_window_open', reason: report.authenticated ? '已确认登录成功，请点击启动后台继续。' : report.authState === 'signed_out' ? '官方登录页已打开，请在专属 Edge 中完成登录。' : '专属 Edge 登录窗口已打开，请完成官方登录；登录后点击启动后台。' };
  if (result.requiresLogin) return { ...result, state: 'login_required', reason: report.connected ? '扩展已连接，请先在专属 Edge 完成一次官方登录。' : '请先在专属 Edge 完成一次官方登录。' };
  if (!report.connected) return result;
  if (report.authState === 'unknown') return { ...result, state: 'authentication_unverified', reason: report.connected ? '扩展已连接，暂未确认 ChatGPT 登录状态；请打开登录页查看。' : '正在检查专属页面和登录状态，尚未连接完成。' };
  return { ...result, state: 'page_loading', reason: '已确认登录，正在等待 ChatGPT 生成页面就绪。' };
}

export function validateRequest(input) {
  if (!input || input.schemaVersion !== 'managed-browser-request/v1' || !actions.has(input.action)) fail('INVALID_REQUEST');
  for (const key of ['home', 'componentHome']) {
    if (typeof input[key] !== 'string' || !path.isAbsolute(input[key]) || /[\0\r\n]/.test(input[key])) fail('INVALID_PATH');
    safePath(input[key]);
  }
  if (input.action === 'open-login' && input.visibleLogin !== true) fail('VISIBLE_LOGIN_NOT_AUTHORIZED');
  if (input.action !== 'open-login' && input.visibleLogin === true) fail('UNEXPECTED_VISIBLE_LOGIN');
  const connection = input.connection;
  let url;
  try { url = new URL(connection?.endpoint); } catch { fail('INVALID_BRIDGE_ORIGIN'); }
  if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || !url.port || url.username || url.password || url.pathname !== '/' || url.search || url.hash) fail('INVALID_BRIDGE_ORIGIN');
  for (const name of ['apiToken', 'bridgeToken']) if (typeof connection[name] !== 'string' || !connection[name] || connection[name].length > 4096 || /[\r\n]/.test(connection[name])) fail('INVALID_CONNECTION_SECRET');
  if (input.action !== 'status' && (input.safety?.verified !== true || input.safety.activeJobs !== 0 || input.safety.unknownJobs !== 0)) fail('JOBS_NOT_PROVED_IDLE');
  return { ...input, home: path.resolve(input.home), componentHome: path.resolve(input.componentHome), connection: { ...connection, endpoint: url.origin } };
}

export function safePath(target) {
  let current = path.resolve(target);
  for (;;) {
    if (fs.existsSync(current) && fs.lstatSync(current).isSymbolicLink()) fail('LINKED_PATH_REJECTED');
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
  return target;
}

export function bridgeIdle(health, listing) {
  // /health.ok means "a selected execution client exists", not HTTP/server
  // health. A verified, authenticated zero-client bridge is exactly the state
  // in which the managed browser must be allowed to start.
  if (typeof health?.ok !== 'boolean' || health.pendingRequests !== 0 || health.pendingCommands !== 0 || listing?.ok !== true || !Array.isArray(listing.clients)
    || !Number.isSafeInteger(health.clients) || health.clients !== listing.clients.length || typeof health.needsSelection !== 'boolean'
    || !Array.isArray(health.activeRequests) || health.activeRequests.length !== 0 || typeof health.transport !== 'string') return false;
  const busy = value => value === true || typeof value === 'number' && value > 0 || Array.isArray(value) && value.length > 0 || value && typeof value === 'object' && Object.keys(value).length > 0 || typeof value === 'string' && !['', 'idle', 'none', '0'].includes(value);
  return listing.clients.every(client => !client.activeRequest && !client.releasePending && client.releaseState !== 'pending' && (client.transportHealth?.kind === 'connect_snapshot' || !['running', 'pending', 'activeCommand'].some(key => busy(client.transportHealth?.tabQueue?.[key]))));
}

export function sessionIdle(session, local = {}) {
  if (!session || typeof session !== 'object' || Array.isArray(session)) return false;
  if (local.bridgePendingExtensionReload && !['completed', 'recovered'].includes(local.bridgePendingExtensionReload.recoveryStatus)) return false;
  for (const [key, value] of Object.entries(session)) {
    if (!key.startsWith('chatgptBridgeV6:tab:')) continue;
    if (value?.schemaVersion !== 6 || value.lease !== null || !Array.isArray(value.outbox) || value.outbox.length || !value.commands || !value.effects || !value.downloads) return false;
    if (!Object.values(value.commands).every(c => ['succeeded', 'rejected', 'cancelled'].includes(c.status)) || !Object.values(value.effects).every(e => ['succeeded', 'rejected', 'cancelled'].includes(e.status)) || Object.keys(value.downloads).length) return false;
  }
  return true;
}

function readJson(file) {
  safePath(file);
  if (!fs.existsSync(file)) return null;
  if (fs.statSync(file).size > 128 * 1024) fail('STATE_TOO_LARGE');
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { fail('STATE_INVALID'); }
}
function atomicJson(file, data) {
  safePath(file);
  const temporary = `${file}.${randomUUID()}.tmp`;
  const fd = fs.openSync(temporary, 'wx', 0o600);
  try { fs.writeFileSync(fd, JSON.stringify(data, null, 2)); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  fs.renameSync(temporary, file);
}
function ps(script, env = {}) {
  return execFileSync(path.join(process.env.SystemRoot || 'C:/Windows', 'System32/WindowsPowerShell/v1.0/powershell.exe'), ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(`$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';[Console]::OutputEncoding=[Text.UTF8Encoding]::new();${script}`, 'utf16le').toString('base64')], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8', timeout: 6500, maxBuffer: 1024 * 1024, env: { ...process.env, ...env } }).trim();
}
function processInfo(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) fail('INVALID_PID');
  const raw = ps(`$p=Get-CimInstance Win32_Process -Filter ('ProcessId='+$env:DIRECTOR_MANAGED_PID);if($p){@{pid=[int]$p.ProcessId;created=$p.CreationDate.ToUniversalTime().ToString('o');executable=$p.ExecutablePath;commandLine=$p.CommandLine}|ConvertTo-Json -Compress}`, { DIRECTOR_MANAGED_PID: String(pid) });
  return raw ? JSON.parse(raw) : null;
}
function profileProcesses(profile) {
  const raw = ps(`@(Get-CimInstance Win32_Process -Filter "name='msedge.exe'"|Where-Object{$_.CommandLine -and $_.CommandLine.IndexOf($env:DIRECTOR_MANAGED_PROFILE,[StringComparison]::OrdinalIgnoreCase)-ge 0}|ForEach-Object{@{pid=[int]$_.ProcessId;created=$_.CreationDate.ToUniversalTime().ToString('o');executable=$_.ExecutablePath;commandLine=$_.CommandLine}})|ConvertTo-Json -Compress`, { DIRECTOR_MANAGED_PROFILE: profile });
  return raw ? [JSON.parse(raw)].flat().filter(Boolean) : [];
}
export function processMatches(record, actual) {
  return Boolean(actual && actual.pid === record.pid && actual.created === record.created && samePath(actual.executable || '.', record.executable) && typeof actual.commandLine === 'string' && actual.commandLine.includes(record.profilePath) && actual.commandLine.includes(`--director-managed-instance=${record.instanceId}`));
}
export function verifyListeners(listeners, pid) {
  if (!Array.isArray(listeners) || !listeners.length || listeners.some(item => !['127.0.0.1', '::1'].includes(item.LocalAddress) || item.OwningProcess !== pid)) fail('CDP_LISTENER_NOT_OWNED_LOOPBACK');
}
function findEdge() {
  if (process.platform !== 'win32') return null;
  for (const root of [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA].filter(Boolean)) {
    const file = path.join(root, 'Microsoft/Edge/Application/msedge.exe');
    if (!fs.existsSync(file)) continue;
    safePath(file);
    const fd = fs.openSync(file, 'r');
    const magic = Buffer.alloc(2);
    try { fs.readSync(fd, magic); } finally { fs.closeSync(fd); }
    if (magic.toString() === 'MZ') return file;
  }
  return null;
}
export function componentIdentity(componentHome) {
  const extension = path.join(componentHome, 'extension');
  safePath(extension);
  const manifest = readJson(path.join(extension, 'manifest.json'));
  if (manifest?.name !== 'ChatGPT Browser Bridge' || manifest.manifest_version !== 3 || manifest.background?.service_worker !== 'background.js' || !manifest.key || !manifest.permissions?.includes('storage')) fail('EXTENSION_IDENTITY_INVALID');
  const code = fs.readFileSync(safePath(path.join(extension, 'shared/buildIdentity.js')), 'utf8');
  const bundleId = /bundleId:\s*['"]([a-f0-9]{32})['"]/.exec(code)?.[1];
  const config = fs.readFileSync(safePath(path.join(extension, 'content/automationConfig.js')), 'utf8');
  if (!bundleId || !/serverUrl:\s*''/.test(config) || !/token:\s*''/.test(config)) fail('EXTENSION_CONFIGURATION_UNSAFE');
  const extensionId = sha(Buffer.from(manifest.key, 'base64')).slice(0, 32).replace(/[0-9a-f]/g, c => String.fromCharCode(97 + parseInt(c, 16)));
  return { extension, extensionId, bundleId, version: manifest.version, manifestSha256: sha(JSON.stringify(manifest)) };
}

class Cdp {
  constructor(socket) {
    this.socket = socket; this.next = 0; this.pending = new Map();
    socket.addEventListener('message', event => {
      let message; try { message = JSON.parse(event.data); } catch { return; }
      const pending = this.pending.get(message.id); if (!pending) return;
      this.pending.delete(message.id); clearTimeout(pending.timer);
      if (message.error) pending.reject(new Error('CDP_COMMAND_FAILED')); else pending.resolve(message.result || {});
    });
  }
  async send(method, params = {}, sessionId) {
    if (this.socket.readyState !== WebSocket.OPEN) fail('CDP_CLOSED');
    return await new Promise((resolve, reject) => {
      const id = ++this.next;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error('CDP_COMMAND_TIMEOUT')); }, 3500);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }
  async evaluate(expression, sessionId) {
    const result = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId);
    if (result.exceptionDetails || result.result?.subtype === 'error') fail('CONTROL_OPERATION_FAILED');
    return result.result?.value;
  }
  async attach(targetId) { return (await this.send('Target.attachToTarget', { targetId, flatten: true })).sessionId; }
  async targets() { return (await this.send('Target.getTargets')).targetInfos || []; }
  close() {
    this.socket.close();
    for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(new Error('CDP_CLOSED')); }
    this.pending.clear();
  }
}

export function parseActivePort(text) {
  const [raw, browserPath] = text.trim().split(/\r?\n/);
  const port = Number(raw);
  if (!Number.isSafeInteger(port) || port < 1024 || port > 65535 || !/^\/devtools\/browser\/[a-f0-9-]+$/.test(browserPath)) fail('CDP_IDENTITY_INVALID');
  return { port, browserPath };
}
async function attachOwned(record) {
  if (!processMatches(record, processInfo(record.pid))) fail('PROCESS_OWNERSHIP_UNPROVED');
  const active = safePath(path.join(record.profilePath, 'DevToolsActivePort'));
  if (fs.statSync(active).size > 1024) fail('CDP_IDENTITY_INVALID');
  const { port, browserPath } = parseActivePort(fs.readFileSync(active, 'utf8'));
  const raw = ps(`@(Get-NetTCPConnection -State Listen -LocalPort ([int]$env:DIRECTOR_MANAGED_PORT)|Select-Object LocalAddress,OwningProcess)|ConvertTo-Json -Compress`, { DIRECTOR_MANAGED_PORT: String(port) });
  verifyListeners(raw ? [JSON.parse(raw)].flat() : [], record.pid);
  const response = await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(2000), redirect: 'error' });
  if (!response.ok) fail('CDP_VERSION_UNAVAILABLE');
  const version = await response.json();
  const expected = `ws://127.0.0.1:${port}${browserPath}`;
  if (version.webSocketDebuggerUrl !== expected) fail('CDP_TARGET_MISMATCH');
  const socket = new WebSocket(expected);
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { socket.close(); reject(new Error('CDP_CONNECT_TIMEOUT')); }, 2000);
    socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
    socket.addEventListener('error', () => { clearTimeout(timer); reject(new Error('CDP_CONNECT_FAILED')); }, { once: true });
  });
  const cdp = new Cdp(socket);
  try {
    const args = (await cdp.send('Browser.getBrowserCommandLine')).arguments;
    if (!Array.isArray(args) || !args.includes(`--user-data-dir=${record.profilePath}`) || !args.includes(`--director-managed-instance=${record.instanceId}`) || !args.includes(`--load-extension=${record.extension}`) || args.includes('--headless=new') !== (record.mode === 'headless')) fail('CDP_COMMAND_IDENTITY_MISMATCH');
    if (!processMatches(record, processInfo(record.pid))) fail('PROCESS_CHANGED_DURING_ATTACH');
    return cdp;
  } catch (error) { cdp.close(); throw error; }
}
async function worker(cdp, component, waitMs = 0) {
  const until = Date.now() + waitMs;
  let wakeRequested = false;
  do {
    const candidates = (await cdp.targets()).filter(t => t.type === 'service_worker' && t.url === `chrome-extension://${component.extensionId}/background.js`);
    if (candidates.length > 1) fail('EXTENSION_WORKER_AMBIGUOUS');
    if (candidates.length) {
      const sessionId = await cdp.attach(candidates[0].targetId);
      const identity = await cdp.evaluate('({id:chrome.runtime.id,name:chrome.runtime.getManifest().name,version:chrome.runtime.getManifest().version,bundleId:globalThis.ChatGptBridgeBuildIdentity?.bundleId})', sessionId);
      if (identity?.id !== component.extensionId || identity.name !== 'ChatGPT Browser Bridge' || identity.version !== component.version || identity.bundleId !== component.bundleId) fail('RUNNING_EXTENSION_MISMATCH');
      return { sessionId, targetId: candidates[0].targetId };
    }
    if (Date.now() >= until) return null;
    if (!wakeRequested) {
      wakeRequested = true;
      // Edge can disable a command-line unpacked extension on the *second*
      // start of a persistent profile. Its external Extensions API reloads the
      // exact verified code in this owned instance, without requiring a content
      // port or user reload. This path is never used by read-only status.
      await enableOwnedDeveloperMode(cdp, component);
      const loaded = await cdp.send('Extensions.loadUnpacked', { path: component.extension });
      if (loaded.id !== component.extensionId) fail('LOADED_EXTENSION_IDENTITY_MISMATCH');
    }
    await sleep(150);
  } while (true);
}
async function enableOwnedDeveloperMode(cdp, component) {
  const installed = (await cdp.send('Extensions.getExtensions')).extensions;
  if (!Array.isArray(installed) || installed.some(e => e.id !== component.extensionId || !samePath(e.path || '.', component.extension))) fail('UNEXPECTED_OWNED_PROFILE_EXTENSION');
  // Use Edge's own profile settings API, never edit Preferences/Secure
  // Preferences, enterprise policy, or the user's normal browser profile.
  const { targetId } = await cdp.send('Target.createTarget', { url: 'edge://extensions/', background: true });
  if (!targetId) fail('COMPONENT_SETTINGS_TARGET_UNKNOWN');
  try {
    const session = await cdp.attach(targetId);
    let available = false;
    for (let i = 0; i < 20; i++) {
      available = await cdp.evaluate("location.href==='edge://extensions/'&&typeof chrome.developerPrivate?.updateProfileConfiguration==='function'", session).catch(() => false);
      if (available) break;
      await sleep(100);
    }
    if (!available) fail('COMPONENT_SETTINGS_API_UNAVAILABLE');
    await cdp.evaluate("new Promise((resolve,reject)=>chrome.developerPrivate.updateProfileConfiguration({inDeveloperMode:true},()=>chrome.runtime.lastError?reject(Error('settings_failed')):resolve(true)))", session);
  } finally {
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {});
  }
}
async function extensionSnapshot(cdp, runtime) {
  return await cdp.evaluate(`(async()=>{const local=await chrome.storage.local.get([${JSON.stringify(PROFILE_KEY)},'bridgePendingExtensionReload']);const session=await chrome.storage.session.get(null);return {local,session};})()`, runtime.sessionId);
}
async function bridgeSnapshot(connection) {
  const get = async route => {
    const response = await fetch(connection.endpoint + route, { headers: { Authorization: `Bearer ${connection.apiToken}` }, signal: AbortSignal.timeout(2000), redirect: 'error' });
    if (!response.ok) fail('BRIDGE_READ_FAILED');
    return await response.json();
  };
  const [health, listing] = await Promise.all([get('/health'), get('/browser/clients')]);
  return { health, listing };
}
function validateRecord(record, paths, component) {
  if (!record) return;
  if (record.schemaVersion !== 'managed-edge-instance/v1' || !UUID.test(record.instanceId || '') || !samePath(record.profilePath || '.', paths.profile) || !samePath(record.extension || '.', component.extension) || !['headless', 'visible'].includes(record.mode)) fail('SAVED_IDENTITY_MISMATCH');
}

export function createManager(overrides = {}) {
  const deps = { findEdge, processInfo, profileProcesses, attachOwned, worker, bridgeSnapshot, sleep, ...overrides };
  const execute = async raw => {
    let cdp, locked = false, lockPath, paths, report;
    const deadline = Date.now() + 33000;
    try {
      const input = validateRequest(raw);
      paths = { root: path.join(input.home, 'managed-edge'), profile: path.join(input.home, 'managed-edge/profile'), state: path.join(input.home, 'managed-edge/instance.json') };
      for (const target of Object.values(paths)) safePath(target);
      const edge = deps.findEdge();
      report = { schemaVersion: 'managed-browser/v1', browser: 'edge', ok: false, state: 'not_started', reason: '', installed: !!edge, componentInstalled: false, process: { running: false, owned: false }, connected: false, authenticated: false, ready: false, extensionReady: false, requiresLogin: false, profilePath: paths.profile, newBinding: false, modelSubmissions: 0, videoSubmissions: 0 };
      if (!edge) return { ...report, state: 'not_installed', reason: '未找到 Microsoft Edge。' };
      const component = componentIdentity(input.componentHome);
      report.componentInstalled = true;
      if (input.action !== 'status') {
        fs.mkdirSync(paths.root, { recursive: true });
        lockPath = path.join(paths.root, 'lifecycle.lock');
        // OS process identity makes a leftover lock recoverable without a time
        // heuristic. An unreadable/reused owner remains held.
        const stale = readJson(lockPath);
        if (stale) {
          const owner = Number.isSafeInteger(stale.pid) ? deps.processInfo(stale.pid) : null;
          if (!Number.isSafeInteger(stale.pid) || !stale.created || owner && owner.created === stale.created) fail('LIFECYCLE_LOCK_HELD');
          fs.unlinkSync(lockPath);
        }
        const self = deps.processInfo(process.pid);
        if (!self?.created) fail('LOCK_OWNER_UNVERIFIED');
        const fd = fs.openSync(lockPath, 'wx', 0o600);
        try { fs.writeFileSync(fd, JSON.stringify({ pid: process.pid, created: self.created })); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
        locked = true;
      }
      let record = readJson(paths.state);
      validateRecord(record, paths, component);
      let actual = record?.pid ? deps.processInfo(record.pid) : null;
      // Windows can reuse the old PID. It is no longer our browser, so never
      // attach to it; a separate exact-profile inventory still gates startup.
      if (actual && actual.created !== record.created) actual = null;
      if (actual && !processMatches(record, actual)) fail('PROCESS_OWNERSHIP_UNPROVED');
      if (!actual) {
        const owners = deps.profileProcesses(paths.profile);
        if (owners.length) fail('UNRECORDED_PROFILE_PROCESS');
      }
      if (actual) {
        cdp = await deps.attachOwned(record);
        report.process = { running: true, owned: true, pid: record.pid };
        report.browserMode = record.mode;
      }
      const snapshot = await deps.bridgeSnapshot(input.connection);
      if (input.action !== 'status' && !bridgeIdle(snapshot.health, snapshot.listing)) fail('BRIDGE_PENDING_WORK');
      let runtime = cdp ? await deps.worker(cdp, component, input.action === 'status' ? 0 : 1500) : null;
      if (cdp && input.action !== 'status') {
        if (!runtime) fail('EXTENSION_STATE_UNAVAILABLE');
        const stored = await extensionSnapshot(cdp, runtime);
        if (!sessionIdle(stored.session, stored.local)) fail('EXTENSION_PENDING_WORK');
      }
      // A missing worker is diagnosed, never treated as proof that it is idle.
      // The app can still recover a verified idle worker through this external
      // browser-level lifecycle, independent of all ChatGPT content ports.
      const desired = input.action === 'open-login' ? 'visible' : 'headless';
      const restart = cdp && (input.action === 'recover' || input.action === 'start-background' && record.mode !== desired || input.action === 'open-login' && record.mode !== desired);
      if (cdp && (restart || input.action === 'stop')) {
        record = { ...record, phase: 'closing', closedAt: new Date().toISOString() };
        atomicJson(paths.state, record);
        await cdp.send('Browser.close').catch(() => {});
        cdp.close(); cdp = null; runtime = null;
        for (let i = 0; i < 30; i++) {
          if (Date.now() >= deadline) fail('LIFECYCLE_DEADLINE');
          if (!deps.processInfo(record.pid)) break;
          await deps.sleep(100);
        }
        if (deps.processInfo(record.pid) || deps.profileProcesses(paths.profile).length) fail('OWNED_BROWSER_CLOSE_UNCONFIRMED');
        actual = null;
        record = { ...record, phase: 'stopped', closedAt: new Date().toISOString() };
        atomicJson(paths.state, record);
        report.process = { running: false, owned: false };
      }
      if (input.action === 'stop') return { ...report, ok: true, state: 'stopped', reason: '专属浏览器已停止；登录数据和任务记录保留。' };
      if (!actual && input.action === 'status') return { ...report, ok: true, state: 'not_started', reason: '专属 Edge 未运行。' };
      if (!actual) {
        fs.mkdirSync(paths.profile, { recursive: true });
        record = { schemaVersion: 'managed-edge-instance/v1', instanceId: randomUUID(), profilePath: paths.profile, extension: component.extension, executable: edge, mode: desired, phase: 'launching', startedAt: new Date().toISOString(), previous: record ? { instanceId: record.instanceId, pid: record.pid, phase: record.phase } : null };
        atomicJson(paths.state, record);
        const args = ['--enable-automation', '--no-first-run', '--no-default-browser-check', '--disable-sync', '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0', `--user-data-dir=${paths.profile}`, '--profile-directory=Default', `--disable-extensions-except=${component.extension}`, `--load-extension=${component.extension}`, `--director-managed-instance=${record.instanceId}`, ...(desired === 'headless' ? ['--headless=new'] : []), 'about:blank'];
        const child = (overrides.spawn || spawn)(edge, args, { windowsHide: desired !== 'visible', detached: true, stdio: 'ignore' });
        let spawnError = false; child.once('error', () => { spawnError = true; }); child.unref();
        if (!child.pid) fail('EDGE_START_FAILED');
        actual = deps.processInfo(child.pid);
        if (!actual || spawnError) fail('EDGE_START_UNCONFIRMED');
        record = { ...record, pid: child.pid, created: actual.created, phase: 'started' };
        if (!processMatches(record, actual)) fail('STARTED_PROCESS_IDENTITY_MISMATCH');
        atomicJson(paths.state, record);
        report.process = { running: true, owned: true, pid: record.pid }; report.browserMode = desired;
        const active = path.join(paths.profile, 'DevToolsActivePort');
        for (let i = 0; i < 50; i++) {
          if (Date.now() >= deadline) fail('LIFECYCLE_DEADLINE');
          if (fs.existsSync(active)) {
            try { cdp = await deps.attachOwned(record); break; } catch (error) { if (i === 49) throw error; }
          }
          await deps.sleep(100);
        }
        if (!cdp) fail('EDGE_START_CDP_UNAVAILABLE');
        report.process = { running: true, owned: true, pid: record.pid }; report.browserMode = desired;
        runtime = await deps.worker(cdp, component, 4000);
        if (!runtime) fail('EXTENSION_NOT_LOADED');
      }
      if (!runtime) return { ...report, state: 'recovery_held', reason: '专属浏览器仍在运行，但扩展状态不可读；保留原实例。' };
      report.extensionReady = true;
      if (input.action !== 'status') {
        const configured = await cdp.evaluate(`(async()=>{const k=${JSON.stringify(PROFILE_KEY)},s=${JSON.stringify(SECRET_KEY)};const old=await chrome.storage.local.get([k,s]);if(old[k]!==undefined&&!${UUID}.test(old[k]))throw Error('profile');const id=old[k]||crypto.randomUUID().toLowerCase();if(old[s]!==undefined&&old[s]!==${JSON.stringify(input.connection.bridgeToken)})throw Error('secret_changed');await chrome.storage.local.set({[k]:id,[s]:${JSON.stringify(input.connection.bridgeToken)}});const check=await chrome.storage.local.get([k,s]);if(check[k]!==id||check[s]!==${JSON.stringify(input.connection.bridgeToken)})throw Error('unconfirmed');return {profileId:id};})()`, runtime.sessionId);
        if (!UUID.test(configured?.profileId || '') || record.profileId && record.profileId !== configured.profileId) fail('PROFILE_IDENTITY_CHANGED');
        record.profileId = configured.profileId;
        record = await ensureController(cdp, runtime, record, input.connection.endpoint, paths.state);
      }
      report.profileId = record.profileId || '';
      report.bundleId = component.bundleId;
      const observed = await inspectController(cdp, runtime, record, component, input.connection, deps);
      Object.assign(report, observed);
      report.ok = true;
      Object.assign(report, managedPresentation(report, record.mode));
      return report;
    } catch (error) {
      // No raw exception/URLs/command output can expose local credentials.
      const code = /^[A-Z][A-Z0-9_]{1,80}$/.test(error?.message || '') ? error.message : 'MANAGED_BROWSER_OPERATION_FAILED';
      return { ...(report || { schemaVersion: 'managed-browser/v1', installed: false, componentInstalled: false, process: { running: false, owned: false }, connected: false, authenticated: false, extensionReady: false, newBinding: false, modelSubmissions: 0, videoSubmissions: 0 }), ok: false, ready: false, state: 'recovery_held', reason: code, errorCode: code };
    } finally {
      cdp?.close();
      if (locked) { try { fs.unlinkSync(lockPath); } catch {} }
    }
  };
  return async raw => {
    const result = await execute(raw);
    return { ...result, status: result.state, message: result.reason || '', running: result.process?.running === true && result.process?.owned === true };
  };
}

async function ensureController(cdp, runtime, record, endpoint, statePath) {
  const launch = `bridge-director-control-${record.profileId}`;
  if (record.controller?.targetId) {
    const found = (await cdp.targets()).find(t => t.targetId === record.controller.targetId);
    if (found) {
      // A visible login target can redirect through the official identity
      // provider, or return to a ChatGPT history page. Preserve it; only a
      // background controller is required to remain on the root URL.
      if (record.mode !== 'visible' && !isControllerRoot(found.url)) fail('CONTROLLER_PAGE_CHANGED');
      if (record.mode === 'visible' && !isControllerRoot(found.url)) return record;
      return await configureController(cdp, record, endpoint, statePath);
    }
    // An intent without its target must never be replayed.
    fail('CONTROLLER_TARGET_MISSING');
  }
  if (record.controller?.phase) fail('CONTROLLER_CREATE_UNKNOWN');
  record = { ...record, controller: { phase: 'reserved', marker: `about:blank#director-managed-${randomUUID()}` } };
  atomicJson(statePath, record);
  const native = await cdp.evaluate(`(async()=>{const tab=await chrome.tabs.create({url:${JSON.stringify(record.controller.marker)},active:false});return {tabId:tab.id,windowId:tab.windowId,incognito:tab.incognito};})()`, runtime.sessionId);
  if (!Number.isSafeInteger(native?.tabId) || native.tabId <= 0 || native.incognito !== false) fail('CONTROLLER_CREATE_UNKNOWN');
  const targets = (await cdp.targets()).filter(t => t.type === 'page' && t.url === record.controller.marker);
  if (targets.length !== 1) fail('CONTROLLER_TARGET_AMBIGUOUS');
  record.controller = { ...record.controller, ...native, targetId: targets[0].targetId };
  atomicJson(statePath, record);
  const page = await cdp.attach(record.controller.targetId);
  const publicConfig = controllerConfigurationScript(endpoint);
  const preload = await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: publicConfig }, page);
  await cdp.evaluate(`(async()=>{const p=${JSON.stringify(record.profileId)},key='directorDesktop:controlTab:v1:'+p;const prior=(await chrome.storage.local.get(key))[key];const session=(await chrome.storage.session.get(${JSON.stringify(SESSION_KEY)}))[${JSON.stringify(SESSION_KEY)}]||crypto.randomUUID().toLowerCase();const next={launchToken:${JSON.stringify(launch)},serverUrl:${JSON.stringify(endpoint)},requestedUrl:'https://chatgpt.com/',tabId:${native.tabId},browserSession:session,status:'opened',managedInstance:${JSON.stringify(record.instanceId)},...(prior?{retiredControllers:[...(prior.retiredControllers||[]),{...prior,retiredControllers:undefined,retiredAt:Date.now(),reason:'managed_process_replaced_after_idle_check'}]}:{})};await chrome.storage.session.set({[${JSON.stringify(SESSION_KEY)}]:session,['chatgptBridgeLaunchedTab:'+${native.tabId}]:{launchToken:next.launchToken,serverUrl:next.serverUrl,requestedUrl:next.requestedUrl,createdAt:Date.now()}});await chrome.storage.local.set({[key]:next});return true;})()`, runtime.sessionId);
  await cdp.send('Page.navigate', { url: 'https://chatgpt.com/' }, page);
  record.controller.phase = 'opened';
  atomicJson(statePath, record);
  record = await configureController(cdp, record, endpoint, statePath, { session: page, preload: preload.identifier });
  if (record.mode === 'visible') await cdp.evaluate(`chrome.tabs.update(${native.tabId},{active:true}).then(()=>true)`, runtime.sessionId);
  return record;
}

function controllerConfigurationScript(endpoint) {
  return `if(location.origin==='https://chatgpt.com'&&window===top){localStorage.setItem('chatgptBridge:bridge.serverUrl',${JSON.stringify(JSON.stringify(endpoint))});localStorage.setItem('chatgptBridge:bridge.token',${JSON.stringify(JSON.stringify(SECRET_MARKER))});}`;
}
export async function configureController(cdp, record, endpoint, statePath, attached = {}) {
  const session = attached.session || await cdp.attach(record.controller.targetId);
  let committed = false;
  for (let i = 0; i < 80; i++) {
    const here = await cdp.evaluate('location.origin', session).catch(() => '');
    if (here === 'https://chatgpt.com') { committed = true; break; }
    await (attached.sleep || sleep)(100);
  }
  if (!committed) {
    // No new target and no page reload: a later lifecycle call repairs this
    // same known controller once its navigation commits. Status stays read-only.
    record.controller.configured = false;
    atomicJson(statePath, record);
    return record;
  }
  const check = `({endpoint:localStorage.getItem('chatgptBridge:bridge.serverUrl'),marker:localStorage.getItem('chatgptBridge:bridge.token')})`;
  const expected = { endpoint: JSON.stringify(endpoint), marker: JSON.stringify(SECRET_MARKER) };
  const before = await cdp.evaluate(check, session);
  const changed = before?.endpoint !== expected.endpoint || before?.marker !== expected.marker;
  if (changed) {
    await cdp.evaluate(controllerConfigurationScript(endpoint), session);
    const after = await cdp.evaluate(check, session);
    if (after?.endpoint !== expected.endpoint || after?.marker !== expected.marker) fail('CONTROLLER_CONFIGURATION_UNCONFIRMED');
  }
  record.controller.configured = true;
  atomicJson(statePath, record);
  if (attached.preload) await cdp.send('Page.removeScriptToEvaluateOnNewDocument', { identifier: attached.preload }, session);
  if (changed) await cdp.send('Page.reload', {}, session);
  return record;
}

// Executed only in an owned ChatGPT target. Return booleans and states, never
// session contents, tokens, conversation text or challenge parameters.
export async function readManagedPage() {
  const visibility = { visibilityState: document.visibilityState, focused: document.hasFocus(), readyState: document.readyState };
  const base = { visibility, authenticated: false, authState: 'unknown', pageState: 'loading', composerReady: false, chatMainReady: false, pageReady: false };
  if (location.origin !== 'https://chatgpt.com') return { ...base, pageState: 'page_unavailable' };
  const composer = document.querySelector('#prompt-textarea, [data-testid="composer"] [contenteditable="true"], textarea[data-id="root"]');
  base.composerReady = Boolean(composer && !composer.disabled && composer.getAttribute('aria-disabled') !== 'true');
  base.chatMainReady = Boolean(document.querySelector('main, [role="main"]'));
  const challengeDOM = Boolean(document.querySelector('#challenge-running, #challenge-form, #cf-challenge-running, iframe[src*="challenges.cloudflare.com"]')) || /^(just a moment|checking your browser|请稍候|正在验证)/i.test(document.title || '');
  const challengeQuery = Array.from(new URL(location.href).searchParams.keys()).some(key => /^__cf_chl_/.test(key));
  if (challengeDOM || challengeQuery && !base.composerReady) return { ...base, pageState: 'site_verification' };
  if (document.readyState === 'loading') return base;
  try {
    const response = await fetch('/api/auth/session', { credentials: 'include', signal: AbortSignal.timeout(1800) });
    if (!response.ok) return { ...base, authState: response.status === 401 ? 'signed_out' : 'unknown', pageState: response.status === 401 ? 'login_required' : 'authentication_unverified' };
    const session = await response.json();
    const authenticated = Boolean(session?.user?.id && session?.accessToken);
    return { ...base, authenticated, authState: authenticated ? 'signed_in' : 'signed_out', pageState: authenticated ? base.composerReady && base.chatMainReady ? 'ready' : 'loading' : 'login_required', pageReady: authenticated && base.composerReady && base.chatMainReady };
  } catch { return { ...base, pageState: 'authentication_unverified' }; }
}

export async function inspectController(cdp, runtime, record, component, connection, deps) {
  const local = await cdp.evaluate(`chrome.storage.local.get(${JSON.stringify(PROFILE_KEY)}).then(v=>({profileId:v[${JSON.stringify(PROFILE_KEY)}]}))`, runtime.sessionId);
  if (!record.profileId || local?.profileId !== record.profileId || !record.controller?.targetId) return { connected: false, authenticated: false, authState: 'unknown', pageState: 'page_unavailable', pageReady: false };
  const target = (await cdp.targets()).find(t => t.targetId === record.controller.targetId);
  if (!target || !(record.mode === 'visible' ? isChatGptPage(target.url) : isControllerRoot(target.url))) return { connected: false, authenticated: false, authState: 'unknown', pageState: target && record.mode === 'visible' ? 'login_in_progress' : 'page_unavailable', pageReady: false };
  const sessionId = await cdp.attach(target.targetId);
  const page = await cdp.evaluate(`(${readManagedPage.toString()})()`, sessionId).catch(() => ({ authenticated: false, authState: 'unknown', pageState: 'loading', pageReady: false }));
  const snapshot = await deps.bridgeSnapshot(connection);
  const clients = snapshot.listing?.clients;
  const matching = Array.isArray(clients) ? clients.filter(c => c.browserProfileId === record.profileId && c.extensionBundleId === component.bundleId && c.browserTabId === record.controller.tabId && c.launchToken === `bridge-director-control-${record.profileId}` && c.ready === true && c.compatible !== false && c.quarantined !== true && c.browserIncognito === false && c.transport === 'extension' && (isControllerRoot(target.url) ? isControllerRoot(c.url) : c.url === target.url)) : [];
  return { ...page, connected: matching.length === 1, pageReady: page.pageReady === true && matching.length === 1 && matching[0].pageReady === true && matching[0].composerReady === true && matching[0].chatMainReady === true, ...(matching.length === 1 ? { clientId: matching[0].id } : {}) };
}

export const run = createManager();
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  let input = '', tooLarge = false;
  process.stdin.setEncoding('utf8');
  for await (const chunk of process.stdin) { input += chunk; if (Buffer.byteLength(input) > 32768) { tooLarge = true; break; } }
  let report;
  try { report = tooLarge ? await run(null) : await run(JSON.parse(input)); } catch { report = await run(null); }
  process.stdout.write(JSON.stringify(report) + '\n');
}
