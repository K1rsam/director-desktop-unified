import { Codex } from '@openai/codex-sdk';
import { createHash, randomUUID } from 'node:crypto';
import { promises as fs } from 'node:fs';
import path from 'node:path';

export const PROTOCOL_VERSION = 'director-sdk-bridge/v1';
export const SDK_VERSION = '0.160.1';
export const LIMITS = Object.freeze({ frameBytes: 1_048_576, eventBytes: 1_048_576, journalBytes: 16_777_216, receiptBytes: 4_194_304, events: 2048, attempts: 128, executionMs: 1_800_000, cancelMs: 500, outputQueueBytes: 2_097_152 });
const REASONING = new Set(['minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra', 'persistent']);
const UNRESOLVED = new Set(['dispatching', 'running', 'cancel_requested', 'submission_unknown']);
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
export const sha256 = value => createHash('sha256').update(value).digest('hex');
const validId = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
export class BridgeError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}
function requireThat(value, code, message) { if (!value) throw new BridgeError(code, message); }
// The pinned official SDK defines ErrorItem as a non-fatal diagnostic, distinct
// from a fatal top-level error. Runtime startup can emit it before turn.started.
// Validate the event contract, not the human-readable message. It grants no
// turn, final text, tool permission or completion; the raw event stays durable.
function preTurnDiagnostic(event) {
  const item = event.item;
  if (event.type !== 'item.completed' || !object(item) || item.type !== 'error'
    || !validId(item.id) || typeof item.message !== 'string'
    || !item.message.length || Buffer.byteLength(item.message) > 4096) return false;
  return Object.keys(event).sort().join(',') === 'item,type'
    && Object.keys(item).sort().join(',') === 'id,message,type';
}
function receiptPayload(value) { const { receiptEnvelopeSha256, ...payload } = value; return payload; }
async function atomicJson(file, value) {
  const payload = receiptPayload(value);
  const envelope = { ...payload, receiptEnvelopeSha256: sha256(JSON.stringify(payload)) };
  const temp = `${file}.${randomUUID()}.tmp`;
  const handle = await fs.open(temp, 'wx', 0o600);
  try { await handle.writeFile(JSON.stringify(envelope)); await handle.sync(); } finally { await handle.close(); }
  await fs.rename(temp, file);
  // Directory fsync is unavailable on some platforms. Native durability remains a release gate.
  let dir;
  try { dir = await fs.open(path.dirname(file), 'r'); await dir.sync(); } catch (e) { if (!['EINVAL', 'EISDIR', 'EPERM', 'EACCES'].includes(e.code)) throw e; } finally { await dir?.close(); }
}

/** Real official SDK adapter. No mock SDK object, resume, alternate model or provider. */
export class OfficialSdkBridge {
  constructor({ stateDirectory, emit = () => {}, offlineFixture = null, nativeRuntime = null, limits = {} }) {
    requireThat(path.isAbsolute(stateDirectory), 'INVALID_DIRECTORY', 'stateDirectory must be absolute and dedicated to this worker');
    this.root = stateDirectory;
    this.emit = emit;
    this.fixture = offlineFixture;
    requireThat(!(offlineFixture && nativeRuntime), 'RUNTIME_MODE_CONFLICT', 'Live execution cannot use an offline fixture');
    this.nativeRuntime = nativeRuntime;
    this.limits = { ...LIMITS, ...limits };
    this.records = new Map();
    this.active = null;
    this.closed = false;
    this.initialized = false;
    this.lock = null;
  }
  async initialize() {
    await fs.mkdir(this.root, { recursive: true, mode: 0o700 });
    requireThat(!(await fs.lstat(this.root)).isSymbolicLink(), 'INVALID_DIRECTORY', 'Worker directory must not be a symlink');
    const lockPath = path.join(this.root, 'worker.lock');
    let recovery = null;
    const recoveryPath = `${lockPath}.recovery`;
    try {
      try { this.lock = await fs.open(lockPath, 'wx', 0o600); }
      catch (error) {
        if (error.code !== 'EEXIST') throw error;
        // Serialize stale recovery. Never unlink a lock using an owner read before
        // acquiring this guard: another contender could have already replaced it.
        try { recovery = await fs.open(recoveryPath, 'wx', 0o600); }
        catch (e) { if (e.code === 'EEXIST') throw new BridgeError('WORKER_LOCKED', 'Worker lock recovery is in progress or requires inspection'); throw e; }
        const owner = JSON.parse(await fs.readFile(lockPath, 'utf8'));
        requireThat(Number.isSafeInteger(owner.pid) && owner.pid > 0, 'WORKER_LOCKED', 'Invalid worker lock; manual inspection required');
        let dead = false;
        try { process.kill(owner.pid, 0); } catch (e) { dead = e.code === 'ESRCH'; }
        requireThat(dead, 'WORKER_LOCKED', 'A worker may still own this directory');
        await fs.unlink(lockPath);
        this.lock = await fs.open(lockPath, 'wx', 0o600);
      }
      await this.lock.writeFile(JSON.stringify({ pid: process.pid, instanceId: randomUUID() }));
      await this.lock.sync();
    } catch (error) { await this.releaseLock(); throw error; }
    finally {
      if (recovery) { await recovery.close(); await fs.unlink(recoveryPath); }
    }
    try {
      for (const entry of await fs.readdir(this.root, { withFileTypes: true })) {
        if (!entry.isDirectory() || !validId(entry.name)) continue;
        const file = path.join(this.root, entry.name, 'receipt.json');
        let record;
        try {
          requireThat((await fs.stat(file)).size <= this.limits.receiptBytes, 'RECEIPT_LIMIT', 'Persisted receipt exceeds bound');
          record = JSON.parse(await fs.readFile(file, 'utf8'));
        } catch (error) {
          if (error.code === 'ENOENT') throw new BridgeError('INCOMPLETE_RECEIPT', 'Partial attempt directory requires inspection; no new dispatch');
          throw error;
        }
        requireThat(record.receiptEnvelopeSha256 === sha256(JSON.stringify(receiptPayload(record))), 'CORRUPT_RECEIPT', 'Receipt envelope hash mismatch; do not resend');
        requireThat(record.attempt?.attemptId === entry.name && record.protocolVersion === PROTOCOL_VERSION, 'CORRUPT_RECEIPT', 'Receipt identity/version mismatch; do not resend');
        requireThat(record.attempt.version === 'text-attempt/v2' && record.attempt.backend === 'codex_sdk' && record.attempt.state === 'prepared', 'CORRUPT_RECEIPT', 'Frozen attempt changed; do not resend');
        requireThat(record.frozenAttemptSha256 === sha256(JSON.stringify(record.attempt)) && record.requestSnapshotSha256 === sha256(JSON.stringify(record.attempt.request)), 'CORRUPT_RECEIPT', 'Frozen attempt/request snapshot hash mismatch; do not resend');
        requireThat(typeof record.compiledPrompt === 'string' && typeof record.compiledSchema === 'string' && record.compiledPromptSha256 === sha256(record.compiledPrompt) && record.compiledSchemaSha256 === sha256(record.compiledSchema), 'CORRUPT_RECEIPT', 'Compiled input hash mismatch; do not resend');
        requireThat(record.rawResult === null ? record.rawResultSha256 === null : record.rawResultSha256 === sha256(JSON.stringify(record.rawResult)), 'CORRUPT_RECEIPT', 'Raw result hash mismatch; do not resend');
        requireThat(['dispatching', 'running', 'cancel_requested', 'submission_unknown', 'transport_completed'].includes(record.state) && Number.isSafeInteger(record.lastSequence) && record.lastSequence >= 0, 'CORRUPT_RECEIPT', 'Invalid receipt state/sequence; do not resend');
        const journalPath = path.join(this.root, entry.name, 'events.ndjson');
        const journalStat = await fs.stat(journalPath).catch(e => { if (e.code === 'ENOENT') return { size: 0 }; throw e; });
        requireThat(journalStat.size <= this.limits.journalBytes, 'JOURNAL_LIMIT', 'Persisted journal exceeds bound');
        const journal = await fs.readFile(journalPath, 'utf8').catch(e => { if (e.code === 'ENOENT') return ''; throw e; });
        requireThat(Buffer.byteLength(journal) <= this.limits.journalBytes, 'JOURNAL_LIMIT', 'Persisted journal exceeds bound');
        if (record.journalSha256 !== sha256(journal)) {
          record.state = 'submission_unknown';
          record.error = { code: 'JOURNAL_HASH', message: 'Journal changed or write was interrupted; do not resend' };
        }
        // A torn final line is evidence of an interrupted write, never permission to dispatch.
        let journalSequence = 0;
        for (const line of journal.split('\n').filter(Boolean)) {
          try {
            const event = JSON.parse(line);
            requireThat(event.protocolVersion === PROTOCOL_VERSION && event.attemptId === entry.name && event.sequence === journalSequence + 1, 'JOURNAL_IDENTITY', 'Journal identity or sequence mismatch');
            journalSequence = event.sequence;
            record.lastSequence = Math.max(record.lastSequence, event.sequence);
          } catch {
            record.state = 'submission_unknown';
            record.error = { code: 'TORN_JOURNAL', message: 'Interrupted or inconsistent journal; do not resend' };
          }
        }
        if (record.state === 'transport_completed' && (!record.rawResult || record.lastSequence !== journalSequence)) {
          record.state = 'submission_unknown';
          record.error = { code: 'INCOMPLETE_COMPLETION', message: 'Completion lacks matching durable evidence; do not resend' };
        }
        if (UNRESOLVED.has(record.state)) {
          record.state = 'submission_unknown';
          record.error ??= { code: 'WORKER_RESTARTED', message: 'SDK has no passive remote observation; do not resend' };
          await atomicJson(file, record);
        }
        this.records.set(entry.name, record);
      }
      requireThat(this.records.size <= this.limits.attempts, 'ATTEMPT_LIMIT', 'Worker history limit reached');
      this.initialized = true;
    } catch (error) { await this.releaseLock(); throw error; }
  }
  inspect() {
    return {
      backend: 'codex_sdk', bridgeVersion: '0.1.0', protocolVersion: PROTOCOL_VERSION,
      sdk: { package: '@openai/codex-sdk', version: SDK_VERSION, transport: 'exec --experimental-json' },
      runtime: { version: this.fixture ? 'synthetic-fixture' : this.nativeRuntime?.identity.runtimePackageVersion ?? null, verified: !!this.nativeRuntime },
      available: !!this.nativeRuntime, liveCallsAllowed: !!this.nativeRuntime, offlineFixtureAvailable: !!this.fixture,
      capabilities: { execute: this.fixture ? 'offline_fixture_only' : this.nativeRuntime ? 'verified_native_text_only' : 'disabled', passiveRemoteObserve: false, localReceiptObserve: true, turnId: false, rawRuntimeBytes: false, cancel: this.nativeRuntime ? 'AbortSignal_with_native_supervisor' : 'AbortSignal_best_effort', processTreeStopProven: false, domainAcceptance: false, maxActiveAttempts: 1 },
      blockers: this.nativeRuntime ? ['Pinned SDK has no passive remote observation or turn ID; interrupted submission remains unknown'] : ['SDK live execution needs verified native supervision', 'Native runtime isolation and process-tree ownership are required'],
      limits: this.limits,
    };
  }
  envelope(message, content) { return { protocolVersion: PROTOCOL_VERSION, requestId: message.requestId, attemptId: message.attemptId ?? null, ...content }; }
  async handle(message) {
    requireThat(this.initialized, 'NOT_INITIALIZED', 'Worker must initialize first');
    requireThat(object(message) && message.protocolVersion === PROTOCOL_VERSION, 'PROTOCOL_VERSION', 'Unsupported protocolVersion');
    requireThat(validId(message.requestId), 'REQUEST_ID', 'requestId must be a bounded identifier');
    requireThat(message.attemptId === null || validId(message.attemptId), 'ATTEMPT_ID', 'Every request needs attemptId (null for control methods)');
    requireThat(!this.closed || message.method === 'observe', 'WORKER_CLOSED', 'Worker is closed');
    let result;
    switch (message.method) {
      case 'hello': case 'inspect': result = this.inspect(); break;
      case 'execute': result = await this.execute(message); break;
      case 'observe': result = this.observe(message.attemptId); break;
      case 'cancel': result = await this.cancel(message.attemptId); break;
      case 'shutdown': await this.disconnect('SHUTDOWN'); result = { shutdown: true, processTreeStopProven: false }; break;
      default: throw new BridgeError('UNKNOWN_METHOD', 'Unsupported bridge method');
    }
    return this.envelope(message, { type: 'response', ok: true, result });
  }
  observe(id) {
    const record = this.records.get(id);
    requireThat(record, 'ATTEMPT_NOT_FOUND', 'No local receipt for attempt; remote state unknown');
    return this.publicReceipt(record);
  }
  publicReceipt(record) {
    return { attemptId: record.attempt.attemptId, semanticRequestHash: record.attempt.semanticRequestHash, executionFingerprint: record.attempt.executionFingerprint, attemptEnvelopeHash: record.attempt.attemptEnvelopeHash ?? null, compiledPromptSha256: record.compiledPromptSha256, compiledSchemaSha256: record.compiledSchemaSha256, state: record.state, backend: 'codex_sdk', threadId: record.threadId, turnId: null, lastSequence: record.lastSequence, rawResult: record.rawResult, error: record.error, usage: record.usage, domainAccepted: false, processTreeStopProven: false, remoteObservationSupported: false, receiptRelativePath: `${record.attempt.attemptId}/receipt.json` };
  }
  async execute(message) {
    requireThat(this.fixture || this.nativeRuntime, 'SDK_LIVE_DISABLED', 'Live SDK execution is disabled; no fallback or runtime was started');
    requireThat(!this.active && ![...this.records.values()].some(r => UNRESOLVED.has(r.state)), 'WORKER_OCCUPIED', 'An active or unknown attempt still owns this worker budget');
    const { attempt, compiledPrompt, compiledSchema, compiledPromptSha256, compiledSchemaSha256 } = message.params ?? {};
    requireThat(object(attempt) && attempt.version === 'text-attempt/v2' && attempt.backend === 'codex_sdk' && attempt.state === 'prepared' && attempt.threadId === null && attempt.turnId === null && attempt.lastSequence === 0 && attempt.rawResult === null, 'FROZEN_ATTEMPT', 'Expected a prepared text-attempt/v2 frozen to codex_sdk');
    requireThat(attempt.attemptId === message.attemptId && validId(attempt.attemptId), 'ATTEMPT_ID', 'Envelope and frozen attempt identities must match');
    requireThat(!this.records.has(attempt.attemptId), 'ATTEMPT_EXISTS', 'Attempt already exists; use observe, never execute again');
    requireThat(this.records.size < this.limits.attempts, 'ATTEMPT_LIMIT', 'Worker history limit reached');
    requireThat(['workflowId', 'logicalNodeId', 'semanticRequestHash', 'executionFingerprint'].every(k => typeof attempt[k] === 'string' && attempt[k].length > 0 && attempt[k].length <= 512), 'FROZEN_IDENTITY', 'Frozen identity fields are required');
    requireThat(object(attempt.request) && typeof attempt.request.model === 'string' && attempt.request.model.trim() && REASONING.has(attempt.request.reasoning), 'MODEL_IDENTITY', 'Explicit frozen model and supported reasoning required');
    requireThat(typeof compiledPrompt === 'string' && compiledPrompt.length > 0 && typeof compiledSchema === 'string', 'COMPILED_INPUT', 'Caller must provide unchanged compiled prompt/schema strings');
    requireThat(sha256(compiledPrompt) === compiledPromptSha256 && sha256(compiledSchema) === compiledSchemaSha256, 'INPUT_HASH', 'Compiled prompt or schema hash mismatch');
    const schema = JSON.parse(compiledSchema);
    requireThat(object(schema), 'SCHEMA', 'Compiled schema must be a JSON object');
    requireThat(Buffer.byteLength(JSON.stringify(message)) <= this.limits.frameBytes, 'FRAME_LIMIT', 'Execute frame exceeds byte limit');
    const frozen = structuredClone(attempt);
    const dir = path.join(this.root, attempt.attemptId);
    // mkdir is intentionally exclusive: a partial previous write must block replay.
    await fs.mkdir(dir, { mode: 0o700 });
    const record = { protocolVersion: PROTOCOL_VERSION, attempt: frozen, compiledPrompt, compiledSchema, compiledPromptSha256, compiledSchemaSha256, requestSnapshotSha256: sha256(JSON.stringify(frozen.request)), frozenAttemptSha256: sha256(JSON.stringify(frozen)), journalSha256: sha256(''), rawResultSha256: null, state: 'dispatching', threadId: null, turnId: null, lastSequence: 0, rawResult: null, usage: null, error: null, domainAccepted: false };
    await atomicJson(path.join(dir, 'receipt.json'), record);
    this.records.set(attempt.attemptId, record);
    const active = { record, requestId: message.requestId, controller: new AbortController(), canceled: false, seen: new Set(), bytes: 0, count: 0, threadStarted: false, turnStarted: false, completed: false, finalItem: null, task: null, abortable: false, finished: false, journalHash: createHash('sha256') };
    this.active = active;
    active.task = this.run(active, schema).catch(async error => {
      record.state = 'submission_unknown';
      record.error = { code: 'PERSISTENCE_OR_EMIT_FAILURE', message: String(error.message).slice(0, 4096) };
      try { await atomicJson(path.join(dir, 'receipt.json'), record); } catch { /* Parent must retain its own dispatch intent. */ }
    }).finally(async () => {
      active.finished = true;
      if (this.active === active) this.active = null;
      if (this.closed) await this.releaseLock();
    });
    return this.publicReceipt(record);
  }
  async persistEvent(active, rawEvent, duplicate = false) {
    const record = active.record;
    const next = { protocolVersion: PROTOCOL_VERSION, requestId: active.requestId, attemptId: record.attempt.attemptId, sequence: record.lastSequence + 1, threadId: record.threadId, turnId: null, type: 'sdk_event', duplicate, rawEvent };
    const line = JSON.stringify(next) + '\n';
    requireThat(Buffer.byteLength(line) <= this.limits.eventBytes, 'EVENT_LIMIT', 'SDK event exceeds byte bound');
    requireThat(active.bytes + Buffer.byteLength(line) <= this.limits.journalBytes && active.count < this.limits.events, 'JOURNAL_LIMIT', 'SDK journal limit reached');
    const file = await fs.open(path.join(this.root, record.attempt.attemptId, 'events.ndjson'), 'a', 0o600);
    try { await file.writeFile(line); await file.sync(); } finally { await file.close(); }
    active.bytes += Buffer.byteLength(line); active.count++;
    active.journalHash.update(line);
    record.journalSha256 = active.journalHash.copy().digest('hex');
    record.lastSequence++;
    await atomicJson(path.join(this.root, record.attempt.attemptId, 'receipt.json'), record);
    this.emit(next);
  }
  async run(active, schema) {
    const record = active.record;
    let temporaryEnvironment = null;
    const timeout = setTimeout(() => { active.canceled = true; if (active.abortable) active.controller.abort(); }, this.limits.executionMs);
    try {
      // Both modes construct the genuine pinned SDK. Production uses only the
      // native supervisor frozen by Rust, never a fixture or provider fallback.
      const fixture = this.fixture
        ? await this.fixture.create(path.join(this.root, record.attempt.attemptId))
        : await this.nativeRuntime.create(path.join(this.root, record.attempt.attemptId), record);
      requireThat(!active.canceled, 'CANCEL_UNCONFIRMED', 'Canceled before SDK dispatch');
      if (this.nativeRuntime) {
        // The official SDK creates output-schema files in this Node process,
        // not in its runtime child's env. This worker permits one active turn.
        requireThat(path.isAbsolute(fixture.temporaryDirectory ?? ''), 'SDK_NATIVE_TEMP', 'Dedicated schema directory required');
        temporaryEnvironment = ['TEMP', 'TMP', 'TMPDIR'].map(key => [key, process.env[key]]);
        for (const [key] of temporaryEnvironment) process.env[key] = fixture.temporaryDirectory;
      }
      const sdk = new Codex({ codexPathOverride: fixture.executable, env: fixture.env, ...(this.fixture ? { config: { tools: { shell: false }, features: { web_search_request: false }, model_provider: 'openai' } } : {}) });
      const thread = sdk.startThread({ model: record.attempt.request.model, modelReasoningEffort: record.attempt.request.reasoning, workingDirectory: fixture.workingDirectory, sandboxMode: 'read-only', approvalPolicy: 'never', networkAccessEnabled: false, webSearchMode: 'disabled', skipGitRepoCheck: true });
      const { events } = await thread.runStreamed(record.compiledPrompt, { outputSchema: schema, signal: active.controller.signal });
      active.abortable = true;
      try {
      for await (const event of events) {
        try {
        const signature = JSON.stringify(event);
        const duplicate = active.seen.has(signature);
        // Persist raw evidence before validating it. Invalid events must remain inspectable.
        if (event.type === 'thread.started' && !record.threadId && typeof event.thread_id === 'string') record.threadId = event.thread_id;
        await this.persistEvent(active, event, duplicate);
        if (duplicate) continue;
        active.seen.add(signature);
        requireThat(!active.completed, 'OUT_OF_ORDER', 'New event after terminal completion');
        if (event.type === 'thread.started') {
          requireThat(!active.threadStarted && typeof event.thread_id === 'string' && record.threadId === event.thread_id, 'THREAD_IDENTITY', 'Thread identity changed or is invalid');
          active.threadStarted = true; record.state = active.canceled ? 'cancel_requested' : 'running';
        } else if (event.type === 'turn.started') {
          requireThat(active.threadStarted && !active.turnStarted, 'OUT_OF_ORDER', 'Turn started without exactly one thread'); active.turnStarted = true;
        } else if (active.threadStarted && !active.turnStarted && preTurnDiagnostic(event)) {
          // The notice is already durably recorded. A real turn and final
          // agent_message are still mandatory before transport completion.
        } else if (['item.started', 'item.updated', 'item.completed'].includes(event.type)) {
          requireThat(active.turnStarted && object(event.item), 'OUT_OF_ORDER', 'Item arrived before turn start');
          requireThat(['agent_message', 'reasoning'].includes(event.item.type), 'POLICY_VIOLATION', 'Text-only bridge received a tool, file, search, or unsupported item');
          if (event.type === 'item.completed' && event.item.type === 'agent_message') {
            requireThat(typeof event.item.text === 'string', 'INVALID_TEXT', 'Agent text must be a string');
            if (active.finalItem?.id === event.item.id) requireThat(active.finalItem.text === event.item.text, 'ITEM_IDENTITY', 'Completed item mutated');
            active.finalItem = event.item;
          }
        } else if (event.type === 'turn.completed') {
          requireThat(active.turnStarted && active.finalItem && object(event.usage), 'OUT_OF_ORDER', 'Completion lacks prior final text or usage');
          active.completed = true; record.usage = event.usage;
        } else if (event.type === 'turn.failed' || event.type === 'error') {
          throw new BridgeError('SDK_TURN_FAILED', event.error?.message ?? event.message ?? 'SDK reported failure');
        } else throw new BridgeError('UNSUPPORTED_EVENT', `Unknown SDK event type: ${String(event.type)}`);
        } catch (error) {
          // Abort while the SDK still owns its error listener, before iterator.return().
          active.controller.abort();
          throw error;
        }
      }
      } finally { active.abortable = false; }
      requireThat(active.completed && active.finalItem, 'INCOMPLETE_STREAM', 'Stream ended without a complete turn');
      const rawText = active.finalItem.text;
      let parsedJson = null, parseError = null;
      try { parsedJson = JSON.parse(rawText); } catch (error) { parseError = String(error.message); }
      record.rawResult = { rawText, parsedJson, parseError, domainAccepted: false };
      record.rawResultSha256 = sha256(JSON.stringify(record.rawResult));
      record.state = active.canceled ? 'submission_unknown' : 'transport_completed';
      if (active.canceled) record.error = { code: 'CANCEL_UNCONFIRMED', message: 'Completion crossed cancellation barrier; never publish automatically' };
    } catch (error) {
      record.state = 'submission_unknown';
      record.error = { code: active.canceled ? 'CANCEL_UNCONFIRMED' : (error.code ?? 'SDK_STREAM_ERROR'), message: String(error.message).slice(0, this.limits.eventBytes / 2) };
    } finally {
      clearTimeout(timeout);
      for (const [key, value] of temporaryEnvironment ?? []) {
        if (value === undefined) delete process.env[key]; else process.env[key] = value;
      }
      await atomicJson(path.join(this.root, record.attempt.attemptId, 'receipt.json'), record);
      this.emit({ protocolVersion: PROTOCOL_VERSION, requestId: active.requestId, attemptId: record.attempt.attemptId, type: 'observation', ...this.publicReceipt(record) });
    }
  }
  async cancel(id) {
    const record = this.records.get(id);
    requireThat(record, 'ATTEMPT_NOT_FOUND', 'No receipt for cancellation');
    const active = this.active;
    if (!active || active.record !== record) return { ...this.publicReceipt(record), cancelSupported: false };
    active.canceled = true;
    record.state = 'cancel_requested';
    await atomicJson(path.join(this.root, id, 'receipt.json'), record);
    if (active.abortable) active.controller.abort();
    await Promise.race([active.task, new Promise(resolve => setTimeout(resolve, this.limits.cancelMs))]);
    record.state = 'submission_unknown';
    record.error = { code: 'CANCEL_UNCONFIRMED', message: 'AbortSignal requested; process-tree termination is not proven' };
    await atomicJson(path.join(this.root, id, 'receipt.json'), record);
    return this.publicReceipt(record);
  }
  async disconnect(reason = 'DISCONNECTED') {
    if (this.closed) return;
    this.closed = true;
    if (this.active) {
      const active = this.active;
      active.canceled = true;
      active.record.state = 'submission_unknown';
      active.record.error = { code: reason, message: 'Connection ended after dispatch; do not create a new turn' };
      await atomicJson(path.join(this.root, active.record.attempt.attemptId, 'receipt.json'), active.record);
      if (active.abortable) active.controller.abort();
      await Promise.race([active.task, new Promise(resolve => setTimeout(resolve, this.limits.cancelMs))]);
    }
    // A detached unfinished task must keep directory ownership until it settles or
    // this worker process exits. Never let two bridge instances write one receipt.
    if (!this.active || this.active.finished) await this.releaseLock();
  }
  async releaseLock() {
    const lock = this.lock; this.lock = null;
    if (lock) { await lock.close(); await fs.unlink(path.join(this.root, 'worker.lock')).catch(e => { if (e.code !== 'ENOENT') throw e; }); }
  }
}
