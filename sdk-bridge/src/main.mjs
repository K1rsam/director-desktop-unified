import { OfficialSdkBridge, BridgeError, LIMITS, PROTOCOL_VERSION } from './bridge.mjs';
import { readFile } from 'node:fs/promises';
import { nativeRuntime } from './native-runtime.mjs';

const args = process.argv.slice(2);
let stateDirectory, scenario, nativeConfig;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--state-dir' && args[i + 1]) stateDirectory = args[++i];
  else if (args[i] === '--offline-fixture' && args[i + 1]) scenario = args[++i];
  else if (args[i] === '--native-config' && args[i + 1]) nativeConfig = args[++i];
  else throw new Error('Expected --state-dir <dedicated absolute path> [--offline-fixture <known scenario>]');
}
if (!stateDirectory) throw new Error('--state-dir is required; no user home or production directory is inferred');
if (scenario && nativeConfig) throw new Error('Native mode cannot use an offline fixture');
let queuedBytes = 0, stopped = false, pendingRequests = 0, commandCount = 0, flushTimer;
const seenRequestIds = new Set();
const send = value => {
  if (stopped || !process.stdout.writable) throw new BridgeError('OUTPUT_CLOSED', 'Parent output is closed');
  const line = JSON.stringify(value) + '\n';
  const length = Buffer.byteLength(line);
  if (length > LIMITS.frameBytes || queuedBytes + length > LIMITS.outputQueueBytes) throw new BridgeError('OUTPUT_LIMIT', 'Bounded output queue exceeded');
  queuedBytes += length;
  if (!flushTimer) flushTimer = setTimeout(() => { void stop('OUTPUT_TIMEOUT'); }, 2000);
  process.stdout.write(line, error => {
    queuedBytes -= length;
    if (!queuedBytes) { clearTimeout(flushTimer); flushTimer = null; }
    if (error) void stop('OUTPUT_CLOSED');
  });
};
const fixture = scenario ? (await import('../test/fixture-runtime.mjs')).offlineFixture(scenario) : null;
const runtime = nativeConfig ? nativeRuntime(JSON.parse(await readFile(nativeConfig, 'utf8')), stateDirectory) : null;
const bridge = new OfficialSdkBridge({ stateDirectory, emit: send, offlineFixture: fixture, nativeRuntime: runtime });
await bridge.initialize();
async function stop(reason) {
  if (stopped) return;
  stopped = true;
  clearTimeout(flushTimer);
  process.stdin.destroy();
  try { await bridge.disconnect(reason); } catch { process.exitCode = 1; }
  // No process-tree claim: the supervisor must keep unknown capacity reserved.
  setTimeout(() => process.exit(process.exitCode ?? 0), 50);
}
process.on('SIGTERM', () => { void stop('SIGTERM'); });
process.on('SIGINT', () => { void stop('SIGINT'); });
process.stdout.on('error', () => { void stop('OUTPUT_CLOSED'); });
process.stdin.on('error', () => { void stop('INPUT_CLOSED'); });
let buffered = Buffer.alloc(0), chain = Promise.resolve();
const respondError = (message, error) => send({ protocolVersion: PROTOCOL_VERSION, requestId: message?.requestId ?? null, attemptId: message?.attemptId ?? null, type: 'response', ok: false, error: { code: error.code ?? 'INVALID_REQUEST', message: String(error.message).slice(0, 4096) } });
function queueLine(line) {
  if (++pendingRequests > 32 || ++commandCount > 2048) { void stop('INPUT_QUEUE_LIMIT'); return; }
  chain = chain.then(async () => {
    if (stopped) return;
    let message;
    try {
      message = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(line));
      if (seenRequestIds.has(message.requestId)) throw new BridgeError('DUPLICATE_REQUEST_ID', 'Request ids are unique per connection; observe an existing attempt');
      const response = await bridge.handle(message);
      seenRequestIds.add(message.requestId);
      send(response);
      if (message.method === 'shutdown') await stop('SHUTDOWN');
    } catch (error) { try { respondError(message, error); } catch { await stop('OUTPUT_CLOSED'); } }
    finally { pendingRequests--; }
  });
}
process.stdin.on('data', chunk => {
  if (stopped) return;
  // Split before concatenating to keep even a hostile pipe chunk bounded per frame.
  let offset = 0;
  while (offset < chunk.length) {
    const newline = chunk.indexOf(10, offset);
    const end = newline === -1 ? chunk.length : newline;
    const part = chunk.subarray(offset, end);
    if (buffered.length + part.length > LIMITS.frameBytes) {
      try { respondError(null, new BridgeError('FRAME_LIMIT', 'Input frame byte limit exceeded')); } finally { void stop('FRAME_LIMIT'); }
      return;
    }
    buffered = Buffer.concat([buffered, part]);
    if (newline === -1) return;
    if (buffered.length) queueLine(buffered);
    buffered = Buffer.alloc(0);
    offset = newline + 1;
  }
});
process.stdin.on('end', () => { void chain.then(() => stop(buffered.length ? 'TRUNCATED_INPUT' : 'INPUT_CLOSED')); });
