// Transport only: no command persistence, replay, page readiness or generation policy.
export function createNativeTransportFactory({
  browser = globalThis.chrome,
  timeoutMs = 8000,
  hostName = 'app.director.browser_bridge',
  timers = globalThis,
  crypto = globalThis.crypto,
} = {}) {
  const VERSION = 'director-native-transport/v1';
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
      if (!current.ready) breakPort(current, 'native_channel_closed');
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
  const fixedHostReason = value => new Set([
    'native_channel_closed', 'native_host_error', 'native_host_disconnected',
    'invalid_request', 'invalid_channel', 'channel_exists', 'channel_limit',
    'invalid_server_url', 'invalid_token', 'upstream_connect_failed',
    'upstream_closed', 'upstream_error', 'message_too_large', 'send_failed',
    'invalid_message', 'truncated_frame', 'frame_limit', 'invalid_json', 'input_failed',
    'nonce_limit', 'channel_rejected', 'server_rejected', 'token_rejected',
    'channel_not_open', 'queue_full', 'message_limit', 'connect_failed', 'socket_failed',
    'origin_rejected', 'handshake_failed', 'channel_closing', 'local_close',
    'remote_close', 'transport_failed', 'binary_rejected',
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
    const owner = { port: browser.runtime.connectNative(hostName), closed: false, ready: false };
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
    const s = { id, owner: null, listeners: new Map(), inbound: Promise.resolve(), chunk: null };
    s.socket = {
      readyState: 0,
      addEventListener(type, fn) { if (typeof fn === 'function') { if (!s.listeners.has(type)) s.listeners.set(type, new Set()); s.listeners.get(type).add(fn); } },
      send(data) {
        if (s.socket.readyState !== 1) throw new Error('native_channel_not_open');
        if (typeof data !== 'string' || encoder.encode(data).length > MAX) { fail(s, 'native_message_invalid'); return; }
        sendPort(s.owner, { type: 'send', channelId: s.id, data });
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
    shutdown() {
      stopped = true;
      if (current) breakPort(current, 'native_factory_shutdown');
      for (const s of [...rejected]) finish(s, 'native_factory_shutdown');
    },
  };
}
