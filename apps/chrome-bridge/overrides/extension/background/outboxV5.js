import { isCriticalMessageType, makeEnvelope, messageDefinition } from './protocolV5.js';

// Background-owned identity survives worker/browser restarts within this profile.
// It identifies the extension installation, never a ChatGPT account or a tab.
export function createBrowserProfileIdentity(storage, randomUUID = () => crypto.randomUUID()) {
  let pending;
  return () => pending || (pending = (async () => {
    const key = 'directorDesktop:browserProfileId:v1';
    if (!storage?.get || !storage?.set) throw new Error('Persistent browser identity storage is unavailable');
    const stored = await storage.get(key);
    if (stored[key] !== undefined) {
      if (typeof stored[key] !== 'string' || !/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i.test(stored[key])) throw new Error('Persistent browser identity is invalid; storage was preserved');
      return stored[key].toLowerCase();
    }
    const value = randomUUID().toLowerCase();
    if (!/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i.test(value)) throw new Error('Browser identity generator returned an invalid UUID');
    await storage.set({ [key]: value });
    const verified = await storage.get(key);
    if (verified[key] !== value) throw new Error('Persistent browser identity could not be verified');
    return value;
  })());
}

export function createProtocolOutbox({ backgroundEpoch, backgroundState, post, summarize }) {
  const activeFlushes = new Map();
  const retryTimers = new Map();
  // Native `sent` only confirms a local write. Durable Protocol5 ACK remains
  // the sole outbox removal authority; a new socket replays persisted entries.
  const socketFlights = new WeakMap();
  function flightsFor(ws) {
    let flights = socketFlights.get(ws);
    if (!flights) {
      flights = { sent: new Map(), sending: new Set() };
      socketFlights.set(ws, flights);
    }
    return flights;
  }
  function pruneFlights(ws, runtime) {
    const flights = socketFlights.get(ws);
    if (!flights || !Array.isArray(runtime?.outbox)) return;
    const persisted = new Set(runtime.outbox.map(entry => entry.messageId));
    const snapshotSequence = Number(runtime.transport?.outboundSequence);
    for (const [id, sequence] of flights.sent) {
      // A stale read must not erase a write registered after that snapshot.
      if (!persisted.has(id) && sequence <= snapshotSequence) flights.sent.delete(id);
    }
  }
  async function sendCriticalOnce(state, ws, messageId, prepare) {
    const flights = flightsFor(ws);
    if (flights.sent.has(messageId) || flights.sending.has(messageId)) return;
    flights.sending.add(messageId);
    try {
      const envelope = await prepare();
      if (!envelope || state.ws !== ws || ws.readyState !== WebSocket.OPEN) return;
      ws.send(JSON.stringify(envelope));
      flights.sent.set(messageId, Number(envelope.source.sequence));
    } finally {
      // A thrown send is not a successful write and must remain recoverable.
      flights.sending.delete(messageId);
    }
  }
  const profileIdentity = createBrowserProfileIdentity(globalThis.chrome?.storage?.local);
  async function materialize(state, messageType, body, options = {}) {
    if (messageType === 'transport.hello') {
      let profileId = '';
      try { profileId = await profileIdentity(); } catch { /* Original source-bound recovery remains connected; new generation cannot use an unproved identity. */ }
      const incognito = state.port?.sender?.tab?.incognito;
      body = { ...body, browserProfileId: profileId, browserIncognito: typeof incognito === 'boolean' ? incognito : null };
    }
    const sequenceOutcome = await backgroundState.transition(state.tabId, {
      type: 'transport.outbound.next', contentEpoch: state.contentEpoch || '',
    });
    if (!sequenceOutcome.accepted) throw new Error(`Unable to advance protocol sequence: ${sequenceOutcome.reason}`);
    const runtime = sequenceOutcome.state;
    const envelopeLease = Object.hasOwn(options, 'lease') ? options.lease : runtime.lease;
    return makeEnvelope(messageType, body, {
      clientId: state.clientId,
      tabId: state.tabId,
      backgroundEpoch,
      contentEpoch: runtime.contentEpoch,
      lease: envelopeLease,
    }, runtime.transport.outboundSequence, options);
  }

  function createEnvelopeDraft(state, messageType, body, options = {}) {
    const definition = messageDefinition(messageType);
    if (!definition) throw new Error(`Unsupported protocol messageType: ${messageType}`);
    const envelopeLease = Object.hasOwn(options, 'lease') ? options.lease : options.runtimeLease;
    return makeEnvelope(messageType, body, {
      clientId: state.clientId,
      tabId: state.tabId,
      backgroundEpoch,
      contentEpoch: state.contentEpoch || '',
      lease: envelopeLease || null,
    }, 0, options);
  }

  async function sendProtocolMessage(state, messageType, body, options = {}) {
    const envelope = await materialize(state, messageType, body, options);
    if (messageType === 'transport.hello') {
      // Retain the exact materialized identity until the server accepts this
      // hello. A local send or protocolReady flag is not a remote receipt.
      state.pendingHelloReceipt = envelope;
    }
    const critical = options.critical === true || isCriticalMessageType(messageType);
    let persistedRuntime;
    if (critical && options.persisted !== true) {
      const stored = await backgroundState.transition(state.tabId, {
        type: 'outbox.enqueued', envelope, contentEpoch: state.contentEpoch || '',
      });
      if (!stored.accepted) throw new Error(`Unable to persist critical extension message: ${stored.reason}`);
      persistedRuntime = stored.state;
    }
    if (state.ws?.readyState === WebSocket.OPEN) {
      const ws = state.ws;
      if (critical) {
        pruneFlights(ws, persistedRuntime || await backgroundState.read(state.tabId));
        await sendCriticalOnce(state, ws, envelope.messageId, () => envelope);
      } else ws.send(JSON.stringify(envelope));
    }
    else post(state.port, { type: 'extension.status', status: 'extension queueing', detail: JSON.stringify(summarize(body)) });
    return envelope;
  }

  async function replayCriticalOutbox(state) {
    const ws = state.ws;
    const runtime = await backgroundState.read(state.tabId);
    if (!ws || state.ws !== ws || ws.readyState !== WebSocket.OPEN) return;
    pruneFlights(ws, runtime);
    for (const persisted of runtime.outbox) {
      if (state.ws !== ws || ws.readyState !== WebSocket.OPEN) return;
      await sendCriticalOnce(state, ws, persisted.messageId, async () => {
        const sequenceOutcome = await backgroundState.transition(state.tabId, {
          type: 'transport.outbound.next', contentEpoch: state.contentEpoch || runtime.contentEpoch || '',
        });
        if (!sequenceOutcome.accepted) throw new Error(`Unable to resequence critical extension message: ${sequenceOutcome.reason}`);
        const current = sequenceOutcome.state;
        const envelope = {
          ...persisted,
          sentAt: Date.now(),
          source: {
            ...persisted.source,
            clientId: state.clientId,
            backgroundEpoch,
            contentEpoch: current.contentEpoch,
            sequence: current.transport.outboundSequence,
          },
        };
        const resequenced = await backgroundState.transition(state.tabId, {
          type: 'outbox.resequenced', envelope, contentEpoch: current.contentEpoch,
        });
        if (!resequenced.accepted) {
          if (resequenced.reason === 'outbox_message_missing') return;
          throw new Error(`Unable to persist replay sequence: ${resequenced.reason}`);
        }
        return envelope;
      });
    }
  }

  function scheduleFlushRetry(state, error) {
    const tabId = state.tabId;
    if (retryTimers.has(tabId) || state.closed) return;
    post(state.port, {
      type: 'extension.status',
      status: 'extension queueing',
      detail: `Critical outbox flush deferred: ${error?.message || error}`,
      code: String(error?.code || 'CRITICAL_OUTBOX_FLUSH_DEFERRED'),
    });
    const timer = setTimeout(() => {
      retryTimers.delete(tabId);
      if (!state.closed) void flushCriticalOutbox(state);
    }, 500);
    timer.unref?.();
    retryTimers.set(tabId, timer);
  }

  async function flushCriticalOutbox(state) {
    const tabId = state.tabId;
    let active = activeFlushes.get(tabId);
    if (active) {
      active.state = state;
      active.dirty = true;
      return active.promise;
    }
    active = { state, dirty: true, promise: null };
    active.promise = (async () => {
      try {
        do {
          active.dirty = false;
          await replayCriticalOutbox(active.state);
        } while (active.dirty);
        const retry = retryTimers.get(tabId);
        if (retry) clearTimeout(retry);
        retryTimers.delete(tabId);
        return { flushed: true };
      } catch (error) {
        scheduleFlushRetry(active.state, error);
        return {
          flushed: false,
          error: {
            code: String(error?.code || 'CRITICAL_OUTBOX_FLUSH_DEFERRED'),
            message: String(error?.message || error),
          },
        };
      }
    })().finally(() => {
      if (activeFlushes.get(tabId) === active) activeFlushes.delete(tabId);
    });
    activeFlushes.set(tabId, active);
    return active.promise;
  }

  return Object.freeze({ createEnvelopeDraft, replayCriticalOutbox, flushCriticalOutbox, sendProtocolMessage });
}
