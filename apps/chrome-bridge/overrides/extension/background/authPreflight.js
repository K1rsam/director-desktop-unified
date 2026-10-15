const BRIDGE_TOKEN_MARKER = '__chatgpt_bridge_secret_in_extension_storage_v1__';
const BRIDGE_TOKEN_STORAGE_KEY = 'chatgptBridge:secret:bridge.token';

function httpUrl(serverUrl, pathname) {
  const base = String(serverUrl || 'http://127.0.0.1:8080')
    .replace(/\/$/, '')
    .replace(/^ws:/, 'http:')
    .replace(/^wss:/, 'https:');
  return new URL(pathname, base);
}

function bridgeAuthCheckUrl(serverUrl, token) {
  const url = httpUrl(serverUrl, '/extension/auth/check');
  if (token) url.searchParams.set('token', token);
  url.searchParams.set('runtime', 'extension');
  return url.toString();
}

function responseDetailText(status, bodyText = '') {
  const text = String(bodyText || '').trim();
  if (!text) return `HTTP ${status}`;
  try {
    const json = JSON.parse(text);
    return String(json.detail || json.error || json.message || text).slice(0, 300);
  } catch {
    return text.slice(0, 300);
  }
}

async function resolveBridgeToken(token) {
  const candidate = String(token || '');
  if (candidate !== BRIDGE_TOKEN_MARKER) return candidate;
  try {
    const stored = await chrome.storage?.local?.get?.(BRIDGE_TOKEN_STORAGE_KEY);
    return String(stored?.[BRIDGE_TOKEN_STORAGE_KEY] || '');
  } catch {
    return '';
  }
}

export async function checkBridgeAuth(state, fetchImpl = globalThis.fetch, {timeoutMs=10000}={}) {
  const token = await resolveBridgeToken(state.token);
  if (!token) {
    return {
      ok: false,
      authError: true,
      status: 401,
      message: 'BRIDGE_TOKEN is not configured in extension-private storage.',
    };
  }

  // Resolve the marker before the WebSocket URL is constructed. The
  // background runtime owns the real token after this point; page-local storage
  // never needs to contain it.
  state.token = token;
  const url = bridgeAuthCheckUrl(state.serverUrl, token);
  const controller=new AbortController();
  const timer=setTimeout(()=>controller.abort(),Math.max(1,Math.min(Number(timeoutMs)||10000,10000)));
  try {
    const response = await fetchImpl(url, {
      method: 'GET',
      credentials: 'omit',
      cache: 'no-store',
      signal: controller.signal,
    });
    if (response.ok) return { ok: true };
    const text = await response.text().catch(() => '');
    const detail = responseDetailText(response.status, text);
    if (response.status === 401 || response.status === 403) {
      return {
        ok: false,
        authError: true,
        status: response.status,
        message: `BRIDGE_TOKEN was rejected by the bridge server (${response.status}). ${detail}`,
      };
    }
    return {
      ok: false,
      status: response.status,
      message: `Bridge auth preflight failed: ${detail}`,
    };
  } catch (err) {
    return { ok: false, offline: true, message: err?.message || String(err) };
  } finally {
    clearTimeout(timer);
  }
}

// This health path does not depend on a content-script port. It only reports
// why the owned controller cannot reconnect; it cannot dispatch browser work.
export async function reportBackgroundHealth({browser=chrome, backgroundEpoch, bundleId, extensionVersion,
  recovery, connectionCount=0, fetchImpl=globalThis.fetch, timeoutMs=5000}={}) {
  const profileKey='directorDesktop:browserProfileId:v1';
  const validId=/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
  let timer;
  try {
    const first=await browser.storage.local.get(profileKey), profileId=first[profileKey];
    if(!validId.test(profileId||''))return {ok:false,reason:'profile_unbound'};
    const controlKey='directorDesktop:controlTab:v1:'+profileId;
    const stored=await browser.storage.local.get([profileKey,controlKey,BRIDGE_TOKEN_STORAGE_KEY]);
    const control=stored[controlKey],token=stored[BRIDGE_TOKEN_STORAGE_KEY];
    if(stored[profileKey]!==profileId || control?.launchToken!=='bridge-director-control-'+profileId
      || typeof token!=='string'||!token)return {ok:false,reason:'binding_unavailable'};
    const server=new URL(control.serverUrl);
    if(server.protocol!=='http:'||!['127.0.0.1','localhost'].includes(server.hostname)
      ||server.username||server.password||server.pathname!=='/'||server.search||server.hash)return {ok:false,reason:'server_invalid'};
    if(!/^[a-f0-9]{32}$/.test(bundleId||'')||typeof backgroundEpoch!=='string'||!backgroundEpoch||backgroundEpoch.length>160
      ||typeof extensionVersion!=='string'||extensionVersion.length>40)return {ok:false,reason:'runtime_invalid'};
    const safeWord=value=>typeof value==='string'&&/^[a-z0-9_]{1,96}$/.test(value)?value:'unknown';
    const payload={schemaVersion:'extension-background-health/v1',profileId,bundleId,backgroundEpoch,extensionVersion,
      status:safeWord(recovery?.status),reason:recovery?.reason?safeWord(recovery.reason):'',
      connectionCount:Number.isSafeInteger(connectionCount)&&connectionCount>=0?Math.min(connectionCount,1000):0};
    const abort=new AbortController();timer=setTimeout(()=>abort.abort(),Math.max(1,Math.min(Number(timeoutMs)||5000,5000)));
    const response=await fetchImpl(new URL('/extension/background/status',server),{method:'POST',credentials:'omit',cache:'no-store',redirect:'error',
      headers:{'Content-Type':'application/json','X-Bridge-Token':token},body:JSON.stringify(payload),signal:abort.signal});
    return {ok:response.ok,reason:response.ok?'reported':'server_unavailable'};
  }catch{return {ok:false,reason:'report_unavailable'};}
  finally{clearTimeout(timer);}
}

// Background transport can be addressable before a page is ready for generation.
export async function readOnlyBootstrapHello(state, runtime, launch, browser = chrome) {
  if (!Number.isInteger(state.tabId) || state.tabId <= 0 || !state.contentEpoch
      || runtime.contentEpoch !== state.contentEpoch || runtime.tabId !== state.tabId) {
    throw new Error('Bootstrap content identity is unproved');
  }
  const tab = await browser.tabs.get(state.tabId);
  const url = new URL(tab.url || '');
  if (tab.id !== state.tabId || url.origin !== 'https://chatgpt.com'
      || url.username || url.password || typeof tab.incognito !== 'boolean') {
    throw new Error('Bootstrap native tab identity is unproved');
  }
  const window=await browser.windows.get(tab.windowId);
  if(window.id!==tab.windowId||typeof window.focused!=='boolean'||typeof tab.active!=='boolean')throw new Error('Bootstrap native window identity is unproved');
  const {type, ...page} = state.initialPage || {};
  return {...page, url:tab.url, title:tab.title||'', browserTabId:state.tabId,
    launchToken:launch?.launchToken||'', requestedUrl:launch?.requestedUrl||'',
    documentReadyState:'loading',pageReady:false,composerReady:false,chatMainReady:false,
    focused:tab.active&&window.focused,visibilityState:tab.active&&window.state!=='minimized'?'visible':'hidden',session:null,tabObservation:null,
    activeRequest:runtime.lease||null,
    transportHealth:{kind:'connect_snapshot',observedAt:Date.now(),backgroundOnly:true,transportKind:state.transportKind||'websocket'}};
}
export function bootstrapAckMatches(state, envelope, backgroundEpoch) {
  const hello=state.pendingHelloReceipt;
  return Boolean(hello && hello.body?.transportHealth?.backgroundOnly===true
    && envelope.messageType==='transport.ack' && envelope.body?.accepted===true
    && envelope.body?.ackMessageId===hello.messageId
    && Number(envelope.body.acceptedSequence)===Number(hello.source.sequence)
    && hello.source.clientId===state.clientId && hello.source.contentEpoch===state.contentEpoch
    && hello.source.backgroundEpoch===backgroundEpoch);
}
export function transportFailureCode(error) {
  const code=typeof error?.code==='string'?error.code.toLowerCase():'transport_open_failed';
  return /^[a-z0-9_]{1,96}$/.test(code)?code:'transport_open_failed';
}

export function backgroundPongEligible(state, envelope) {
  return Boolean(!state.closed && state.backgroundHandshakeReady && !state.protocolReady
    && envelope.messageType === 'transport.ping');
}

// BEGIN DIRECTOR NATIVE TRANSPORT ADAPTER (generated)
// Transport only: no command persistence, replay, page readiness or generation policy.
export function createNativeTransportFactory({
  browser = globalThis.chrome,
  timeoutMs = 8000,
  hostName = 'app.director.browser_bridge',
  timers = globalThis,
  crypto = globalThis.crypto,
} = {}) {
  const VERSION = 'director-native-transport/v2';
  const CHUNK = 192 * 1024, MAX = 32 * 1024 * 1024;
  const encoder = new TextEncoder();
  const channels = new Map();
  const rejected = new Set();
  let current = null, serial = 0, stopped = false;
  const later = (fn, ms) => {
    const id = timers.setTimeout(fn, ms);
    id?.unref?.();
    return id;
  };
  const cancel = id => { if (id != null) timers.clearTimeout(id); };
  const fixedError = code => Object.assign(new Error(code), { code });
  const emit = (s, type, body = {}) => {
    const event = { type, target: s.socket, ...body };
    for (const fn of [...(s.listeners.get(type) || [])]) {
      try { fn(event); } catch { /* Consumer exceptions must not break other channels. */ }
    }
  };
  const sendPort = (owner, message) => {
    if (!owner || owner !== current || owner.closed) return false;
    try { owner.port.postMessage(message); return true; }
    catch { breakPort(owner, 'native_host_disconnected'); return false; }
  };
  function finish(s, reason, code = 1006, notifyHost = false) {
    if (s.socket.readyState === 3) return;
    s.socket.readyState = 3;
    cancel(s.rejectTimer);
    rejected.delete(s);
    cancel(s.deadline);
    cancel(s.writeDeadline);
    s.pending = [];
    s.pendingBytes = 0;
    s.inflight = null;
    cancel(s.chunk?.deadline);
    s.chunk = null;
    if (channels.get(s.id) === s) channels.delete(s.id);
    if (notifyHost) sendPort(s.owner, { type: 'close', channelId: s.id });
    emit(s, 'close', { code, reason, wasClean: code === 1000 });
    if (!channels.size && current) {
      cancel(current.heartbeat);
      cancel(current.pongDeadline);
      current.heartbeat = current.pongDeadline = null;
      current.nonce = null;
      if (!current.ready && !current.waiters.size) breakPort(current, 'native_channel_closed');
    }
  }
  function fail(s, reason) {
    if (s.socket.readyState === 3) return;
    emit(s, 'error', { code: reason });
    finish(s, reason, 1006, true);
  }
  function breakPort(owner, reason) {
    if (owner.closed) return;
    owner.closed = true;
    cancel(owner.readyDeadline);
    cancel(owner.heartbeat);
    cancel(owner.pongDeadline);
    for (const waiter of owner.waiters) waiter.reject(fixedError(reason));
    owner.waiters.clear();
    if (current === owner) current = null;
    owner.port.onMessage.removeListener?.(owner.onMessage);
    owner.port.onDisconnect.removeListener?.(owner.onDisconnect);
    for (const s of [...channels.values()]) if (s.owner === owner) fail(s, reason);
    try { owner.port.disconnect(); } catch { /* Already disconnected. */ }
  }
  function schedulePing(owner) {
    if (owner !== current || owner.closed || !owner.ready || !channels.size || owner.heartbeat || owner.nonce) return;
    owner.heartbeat = later(() => {
      owner.heartbeat = null;
      if (owner !== current || !channels.size) return;
      owner.nonce = `health-${++serial}`;
      owner.pongDeadline = later(() => breakPort(owner, 'native_heartbeat_timeout'), timeoutMs);
      sendPort(owner, { type: 'ping', nonce: owner.nonce });
    }, 20000);
  }
  function flushWrite(s) {
    if (s.socket.readyState !== 1 || s.inflight || !s.pending.length) return;
    const next = s.pending[0];
    s.inflight = next.writeId;
    s.writeDeadline = later(() => fail(s, 'native_send_ack_timeout'), timeoutMs);
    sendPort(s.owner, { type: 'send', channelId: s.id, writeId: next.writeId, data: next.data });
  }
  const fixedHostReason = value => new Set([
    'native_channel_closed', 'native_host_error', 'native_host_disconnected',
    'invalid_request', 'invalid_channel', 'channel_exists', 'channel_limit',
    'invalid_server_url', 'invalid_token', 'upstream_connect_failed',
    'upstream_closed', 'upstream_error', 'message_too_large', 'send_failed',
    'invalid_message', 'truncated_frame', 'frame_limit', 'invalid_json', 'input_failed',
    'nonce_limit', 'channel_rejected', 'server_rejected', 'token_rejected',
    'channel_not_open', 'queue_full', 'message_limit', 'connect_failed', 'socket_failed',
    'origin_rejected', 'handshake_failed', 'channel_closing', 'local_close',
    'remote_close', 'transport_failed', 'binary_rejected', 'invalid_write',
  ]).has(value) ? value : 'native_host_error';
  async function receiveData(s, message) {
    if (s.socket.readyState !== 1) { if (s.socket.readyState !== 3) fail(s, 'native_channel_not_open'); return; }
    if (message.type === 'message') {
      if (s.chunk) return fail(s, 'native_chunk_order');
      if (typeof message.data !== 'string' || encoder.encode(message.data).length > CHUNK) return fail(s, 'native_message_invalid');
      emit(s, 'message', { data: message.data });
      return;
    }
    const m = message;
    if (typeof m.transferId !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(m.transferId) ||
        !Number.isInteger(m.index) || !Number.isInteger(m.count) || !Number.isInteger(m.byteLength) ||
        m.byteLength <= 0 || m.byteLength > MAX || m.count !== Math.ceil(m.byteLength / CHUNK) ||
        m.index < 0 || m.index >= m.count || !/^[a-f0-9]{64}$/.test(m.sha256) ||
        typeof m.dataBase64 !== 'string' || m.dataBase64.length > Math.ceil(CHUNK / 3) * 4 ||
        !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(m.dataBase64)) return fail(s, 'native_chunk_invalid');
    if (!s.chunk) {
      if (m.index !== 0) return fail(s, 'native_chunk_order');
      s.chunk = { id: m.transferId, index: 0, count: m.count, length: m.byteLength, sha: m.sha256, parts: [], deadline: null };
      s.chunk.deadline = later(() => fail(s, 'native_chunk_timeout'), timeoutMs);
    }
    const c = s.chunk;
    if (m.transferId !== c.id || m.index !== c.index || m.count !== c.count || m.byteLength !== c.length || m.sha256 !== c.sha) return fail(s, 'native_chunk_order');
    let bytes;
    try { bytes = Uint8Array.from(atob(m.dataBase64), ch => ch.charCodeAt(0)); }
    catch { return fail(s, 'native_chunk_invalid'); }
    const expected = m.index === m.count - 1 ? m.byteLength - m.index * CHUNK : CHUNK;
    if (bytes.length !== expected) return fail(s, 'native_chunk_length');
    c.parts.push(bytes);
    c.index++;
    if (c.index !== c.count) return;
    const all = new Uint8Array(c.length);
    let offset = 0;
    for (const part of c.parts) { all.set(part, offset); offset += part.length; }
    let text;
    try {
      const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', all));
      if (s.chunk !== c || s.socket.readyState !== 1) return;
      if ([...digest].map(x => x.toString(16).padStart(2, '0')).join('') !== c.sha) return fail(s, 'native_chunk_hash');
      text = new TextDecoder('utf-8', { fatal: true }).decode(all);
    } catch { return fail(s, 'native_chunk_integrity'); }
    cancel(c.deadline);
    s.chunk = null;
    emit(s, 'message', { data: text });
  }
  function getPort() {
    if (current) return current;
    const owner = { port: browser.runtime.connectNative(hostName), closed: false, ready: false, waiters: new Set() };
    current = owner;
    owner.onDisconnect = () => {
      // Read lastError to acknowledge Chrome's error; never expose its raw message.
      void browser.runtime.lastError;
      breakPort(owner, 'native_host_disconnected');
    };
    owner.onMessage = message => {
      if (owner !== current || owner.closed) return;
      if (!message || typeof message !== 'object') return breakPort(owner, 'native_protocol_invalid');
      if (message.type === 'ready') {
        if (owner.ready || message.version !== VERSION) return breakPort(owner, 'native_version_mismatch');
        owner.ready = true;
        cancel(owner.readyDeadline);
        for (const waiter of owner.waiters) waiter.resolve({ version: VERSION });
        owner.waiters.clear();
        schedulePing(owner);
        return;
      }
      if (!owner.ready) return breakPort(owner, 'native_ready_required');
      if (message.type === 'pong') {
        if (owner.nonce && message.nonce === owner.nonce) {
          cancel(owner.pongDeadline); owner.pongDeadline = null; owner.nonce = null; schedulePing(owner);
        }
        return;
      }
      if (message.type === 'error' && message.channelId == null) return breakPort(owner, fixedHostReason(message.code));
      const s = channels.get(message.channelId);
      if (!s || s.owner !== owner) return;
      if (message.type === 'open') {
        if (s.socket.readyState !== 0) return fail(s, 'native_protocol_invalid');
        cancel(s.deadline); s.socket.readyState = 1; emit(s, 'open'); return;
      }
      if (message.type === 'sent') {
        if (s.socket.readyState === 1 && s.inflight && message.writeId === s.inflight && s.pending[0]?.writeId === message.writeId) {
          cancel(s.writeDeadline); s.writeDeadline = null;
          s.pendingBytes -= s.pending.shift().bytes;
          s.inflight = null;
          flushWrite(s);
        }
        return;
      }
      if (message.type === 'close') {
        s.inbound = s.inbound.then(() => finish(s, fixedHostReason(message.reason), message.code === 1000 ? 1000 : 1006));
        return;
      }
      if (message.type === 'error') return fail(s, fixedHostReason(message.code));
      if (message.type !== 'message' && message.type !== 'chunk') return fail(s, 'native_protocol_invalid');
      s.inbound = s.inbound.then(() => receiveData(s, message)).catch(() => fail(s, 'native_protocol_invalid'));
    };
    owner.port.onMessage.addListener(owner.onMessage);
    owner.port.onDisconnect.addListener(owner.onDisconnect);
    owner.readyDeadline = later(() => breakPort(owner, 'native_ready_timeout'), timeoutMs);
    return owner;
  }
  function connect({ serverUrl, token, channelId } = {}) {
    const id = channelId ?? `channel-${++serial}`;
    const s = { id, owner: null, listeners: new Map(), inbound: Promise.resolve(), chunk: null, pending: [], pendingBytes: 0, inflight: null };
    s.socket = {
      readyState: 0,
      addEventListener(type, fn) { if (typeof fn === 'function') { if (!s.listeners.has(type)) s.listeners.set(type, new Set()); s.listeners.get(type).add(fn); } },
      send(data) {
        if (s.socket.readyState !== 1) throw new Error('native_channel_not_open');
        if (typeof data !== 'string') { fail(s, 'native_message_invalid'); return; }
        const bytes = encoder.encode(data).length;
        if (s.pending.length >= 256 || bytes > MAX - s.pendingBytes) { fail(s, 'native_send_queue_full'); return; }
        s.pending.push({ writeId: `write-${++serial}`, data, bytes });
        s.pendingBytes += bytes;
        flushWrite(s);
      },
      close(code = 1000) {
        const safeCode = Number.isInteger(code) && ([1000, 1002, 1006].includes(code) || (code >= 3000 && code <= 4999)) ? code : 1000;
        finish(s, 'native_channel_closed', safeCode, true);
      },
    };
    const reject = reason => {
      rejected.add(s);
      s.rejectTimer = later(() => fail(s, reason), 0);
      return s.socket;
    };
    if (stopped) return reject('native_factory_shutdown');
    if (typeof id !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(id) || typeof serverUrl !== 'string' || typeof token !== 'string') return reject('native_connect_invalid');
    if (channels.has(id)) return reject('native_channel_exists');
    if (channels.size >= 8) return reject('native_channel_limit');
    try { s.owner = getPort(); } catch { return reject('native_host_unavailable'); }
    channels.set(id, s);
    s.deadline = later(() => fail(s, 'native_open_timeout'), timeoutMs);
    sendPort(s.owner, { type: 'open', channelId: id, serverUrl, token });
    schedulePing(s.owner);
    return s.socket;
  }
  return {
    connect,
    ensureConnected() {
      if (stopped) return Promise.reject(fixedError('native_factory_shutdown'));
      let owner;
      try { owner = getPort(); }
      catch { return Promise.reject(fixedError('native_host_unavailable')); }
      if (owner.ready) return Promise.resolve({ version: VERSION });
      return new Promise((resolve, reject) => owner.waiters.add({ resolve, reject }));
    },
    shutdown() {
      stopped = true;
      if (current) breakPort(current, 'native_factory_shutdown');
      for (const s of [...rejected]) finish(s, 'native_factory_shutdown');
    },
  };
}
// END DIRECTOR NATIVE TRANSPORT ADAPTER
