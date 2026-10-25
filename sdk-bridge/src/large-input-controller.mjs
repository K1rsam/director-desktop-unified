// Content-pinned native entry for large, frozen full-episode requests. Original
// bridge/SDK bytes and their durable receipt protocol are loaded unchanged.
// Only incoming envelopes grow; event, output, history and execution bounds do not.
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const INPUT_FRAME_BYTES = 4_194_304;
export const OUTPUT_FRAME_BYTES = 1_048_576;
const SDK_SHA = 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
function check(ok, message) {
  if (!ok) throw Object.assign(new Error(`SDK_LARGE_INPUT_IDENTITY: ${message}`), { code: 'SDK_LARGE_INPUT_IDENTITY' });
}
async function fileBytes(file, max = 4_194_304) {
  check(path.isAbsolute(file), 'absolute file required');
  const stat = await fs.lstat(file);
  check(stat.isFile() && !stat.isSymbolicLink() && stat.size <= max, 'bounded regular file required');
  for (let dir = path.dirname(file); ; dir = path.dirname(dir)) {
    const stat = await fs.lstat(dir);
    check(stat.isDirectory() && !stat.isSymbolicLink(), 'linked resource directory');
    if (path.dirname(dir) === dir) break;
  }
  return fs.readFile(file);
}

export async function createLargeInputBridge({ stateDirectory, nativeConfig, emit = () => {}, runtimeFactory = null }) {
  check(path.isAbsolute(stateDirectory) && nativeConfig?.version === 'sdk-native-config/v1'
    && Object.keys(nativeConfig).sort().join(',') === 'identity,version', 'native configuration required');
  const identity = nativeConfig.identity;
  check(path.isAbsolute(identity?.resourceDirectory ?? '') && identity.sdkImplementationSha256 === SDK_SHA, 'pinned SDK resources required');
  const resources = identity.resourceDirectory;
  const source = await Promise.all(['main.mjs', 'bridge.mjs', 'native-runtime.mjs']
    .map(name => fileBytes(path.join(resources, 'src', name))));
  check(hash(Buffer.concat(source)) === identity.bridgeSha256, 'original bridge source changed');
  check(hash(await fileBytes(path.join(resources, 'node_modules/@openai/codex-sdk/dist/index.js'))) === SDK_SHA, 'official SDK changed');
  const descriptor = Object.freeze({ version: 'sdk-large-input-controller/v1',
    controllerSha256: hash(await fileBytes(fileURLToPath(import.meta.url))),
    inputFrameBytes: INPUT_FRAME_BYTES, outputFrameBytes: OUTPUT_FRAME_BYTES });
  const { OfficialSdkBridge } = await import(pathToFileURL(path.join(resources, 'src/bridge.mjs')));
  const { nativeRuntime } = await import(pathToFileURL(path.join(resources, 'src/native-runtime.mjs')));
  const originalRuntime = nativeRuntime(nativeConfig, stateDirectory);
  // Programmatic tests may supply a synthetic subprocess. The executable entry
  // below never accepts a runtime factory or an offline override from argv.
  const runtime = runtimeFactory ? { identity, create: runtimeFactory } : originalRuntime;
  const bridge = new OfficialSdkBridge({ stateDirectory, emit, nativeRuntime: runtime,
    limits: { frameBytes: INPUT_FRAME_BYTES } });
  const handle = bridge.handle.bind(bridge);
  bridge.handle = async message => {
    const response = await handle(message);
    if (['hello', 'inspect', 'execute'].includes(message.method) && response.ok) {
      response.result.inputEnvelope = descriptor;
    }
    return response;
  };
  await bridge.initialize();
  return bridge;
}

async function main() {
  const args = process.argv.slice(2);
  let stateDirectory, nativeConfigFile;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--state-dir' && args[i + 1] && !stateDirectory) stateDirectory = args[++i];
    else if (args[i] === '--native-config' && args[i + 1] && !nativeConfigFile) nativeConfigFile = args[++i];
    else throw new Error('Expected --state-dir <absolute path> --native-config <absolute file>');
  }
  check(stateDirectory && nativeConfigFile, 'state directory and native config required');
  let queuedBytes = 0, stopped = false, pendingRequests = 0, pendingInputBytes = 0, commandCount = 0, flushTimer;
  const seen = new Set();
  const send = value => {
    if (stopped || !process.stdout.writable) throw new Error('OUTPUT_CLOSED');
    const line = JSON.stringify(value) + '\n', length = Buffer.byteLength(line);
    if (length > OUTPUT_FRAME_BYTES || queuedBytes + length > 2_097_152) throw Object.assign(new Error('Bounded output queue exceeded'), { code: 'OUTPUT_LIMIT' });
    queuedBytes += length;
    if (!flushTimer) flushTimer = setTimeout(() => { void stop('OUTPUT_TIMEOUT'); }, 2000);
    process.stdout.write(line, error => {
      queuedBytes -= length;
      if (!queuedBytes) { clearTimeout(flushTimer); flushTimer = null; }
      if (error) void stop('OUTPUT_CLOSED');
    });
  };
  const bridge = await createLargeInputBridge({ stateDirectory,
    nativeConfig: JSON.parse(await fileBytes(nativeConfigFile)), emit: send });
  async function stop(reason) {
    if (stopped) return;
    stopped = true; clearTimeout(flushTimer); process.stdin.destroy();
    try { await bridge.disconnect(reason); } catch { process.exitCode = 1; }
    setTimeout(() => process.exit(process.exitCode ?? 0), 50);
  }
  for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => { void stop(signal); });
  process.stdout.on('error', () => { void stop('OUTPUT_CLOSED'); });
  process.stdin.on('error', () => { void stop('INPUT_CLOSED'); });
  const respondError = (message, error) => send({ protocolVersion: 'director-sdk-bridge/v1',
    requestId: message?.requestId ?? null, attemptId: message?.attemptId ?? null,
    type: 'response', ok: false, error: { code: error.code ?? 'INVALID_REQUEST', message: String(error.message).slice(0, 4096) } });
  let buffered = Buffer.alloc(0), chain = Promise.resolve();
  function queueLine(line) {
    if (++pendingRequests > 32 || ++commandCount > 2048 || (pendingInputBytes += line.length) > 8_388_608) {
      void stop('INPUT_QUEUE_LIMIT'); return;
    }
    chain = chain.then(async () => {
      if (stopped) return;
      let message;
      try {
        message = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(line));
        if (seen.has(message.requestId)) throw Object.assign(new Error('Request ids are unique per connection'), { code: 'DUPLICATE_REQUEST_ID' });
        const response = await bridge.handle(message); seen.add(message.requestId); send(response);
        if (message.method === 'shutdown') await stop('SHUTDOWN');
      } catch (error) { try { respondError(message, error); } catch { await stop('OUTPUT_CLOSED'); } }
      finally { pendingRequests--; pendingInputBytes -= line.length; }
    });
  }
  process.stdin.on('data', chunk => {
    if (stopped) return;
    let offset = 0;
    while (offset < chunk.length) {
      const newline = chunk.indexOf(10, offset), end = newline === -1 ? chunk.length : newline;
      const part = chunk.subarray(offset, end);
      if (buffered.length + part.length > INPUT_FRAME_BYTES) {
        try { respondError(null, Object.assign(new Error('Input frame byte limit exceeded'), { code: 'FRAME_LIMIT' })); }
        finally { void stop('FRAME_LIMIT'); }
        return;
      }
      buffered = Buffer.concat([buffered, part]);
      if (newline === -1) return;
      if (buffered.length) queueLine(buffered);
      buffered = Buffer.alloc(0); offset = newline + 1;
    }
  });
  process.stdin.on('end', () => { void chain.then(() => stop(buffered.length ? 'TRUNCATED_INPUT' : 'INPUT_CLOSED')); });
}
if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) await main();
