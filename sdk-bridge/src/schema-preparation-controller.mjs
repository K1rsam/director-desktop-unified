// Separate, Rust-content-pinned recovery entry. The frozen legacy bridge and
// SDK resources remain byte-for-byte unchanged; ordinary execute is unavailable.
import { promises as fs } from 'node:fs';
import { createReadStream } from 'node:fs';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const VERSION = 'sdk-schema-preparation-recovery/v1';
const BRIDGE_SHA = '48af55f1835bd6959ede5801fa8ff2ca9068b267b36ec9f751949efdc9481dc7';
const SDK_SHA = 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const held = (ok, text) => { if (!ok) throw Object.assign(new Error(`SDK_SCHEMA_PREPARATION_RECOVERY_HELD: ${text}`), { code: 'SDK_SCHEMA_PREPARATION_RECOVERY_HELD' }); };
async function bytes(file) {
  const m = await fs.lstat(file); held(m.isFile() && !m.isSymbolicLink() && m.size <= 4194304, 'bounded regular file required');
  return fs.readFile(file);
}
async function fileHash(file) {
  const m = await fs.lstat(file); held(m.isFile() && !m.isSymbolicLink() && m.size <= 1073741824, 'regular executable required');
  const digest = createHash('sha256'); for await (const chunk of createReadStream(file)) digest.update(chunk); return digest.digest('hex');
}
async function directory(file, names) {
  const m = await fs.lstat(file); held(m.isDirectory() && !m.isSymbolicLink(), 'linked directory');
  held(JSON.stringify((await fs.readdir(file)).sort()) === JSON.stringify([...names].sort()), 'unexpected directory contents');
}
async function inventory(root) {
  await directory(root, ['runtime-plan.json', 'work', 'tmp']);
  await directory(path.join(root, 'work'), ['.director-text-root']); await directory(path.join(root, 'tmp'), []);
  const out = [];
  for (const name of ['runtime-plan.json', 'work/.director-text-root']) {
    const body = await bytes(path.join(root, name));
    if (name.startsWith('work/')) held(body.equals(Buffer.from('director SDK text execution\n')), 'work marker changed');
    out.push([name, body.length, hash(body)]);
  }
  return out;
}
async function exclusive(file, body) {
  const handle = await fs.open(file, 'wx', 0o600);
  try { await handle.writeFile(body); await handle.sync(); } finally { await handle.close(); }
}
async function atomicRecord(file, record) {
  const { receiptEnvelopeSha256, ...payload } = record;
  const body = JSON.stringify({ ...payload, receiptEnvelopeSha256: hash(JSON.stringify(payload)) });
  const temp = `${file}.schema-recovery-${process.pid}`;
  await exclusive(temp, body); await fs.rename(temp, file);
}
function identityBound(old, current) {
  const expected = { ...old };
  for (const key of ['supervisorExecutable', 'supervisorSha256', 'resourceDirectory']) expected[key] = current[key];
  return old.bridgeSha256 === BRIDGE_SHA && current.bridgeSha256 === BRIDGE_SHA
    && JSON.stringify(expected) === JSON.stringify(current);
}

export async function createSchemaRecoveryBridge({ stateDirectory, nativeConfig, emit = () => {}, runtimeFactory = null }) {
  held(path.isAbsolute(stateDirectory) && nativeConfig?.version === 'sdk-native-config/v1'
    && Object.keys(nativeConfig).sort().join(',') === 'identity,version', 'native configuration');
  const identity = nativeConfig.identity, resources = identity.resourceDirectory;
  const controllerSha = hash(await bytes(fileURLToPath(import.meta.url)));
  held(path.isAbsolute(resources) && identity.bridgeSha256 === BRIDGE_SHA && identity.sdkImplementationSha256 === SDK_SHA, 'pinned original execution version');
  const source = await Promise.all(['main.mjs', 'bridge.mjs', 'native-runtime.mjs'].map(name => bytes(path.join(resources, 'src', name))));
  held(hash(Buffer.concat(source)) === BRIDGE_SHA, 'legacy bridge source changed');
  const sdk = await bytes(path.join(resources, 'node_modules/@openai/codex-sdk/dist/index.js'));
  held(hash(sdk) === SDK_SHA, 'official SDK changed');
  const text = sdk.toString('utf8'), prepare = text.indexOf('await createOutputSchemaFile(turnOptions.outputSchema)');
  held(prepare >= 0 && text.indexOf('this._exec.run(', prepare) > prepare, 'pinned pre-exec schema order');
  const { OfficialSdkBridge } = await import(pathToFileURL(path.join(resources, 'src/bridge.mjs')));
  const state = path.resolve(stateDirectory);
  const runtime = { identity, async create(attemptDirectory, record) {
    if (runtimeFactory) return runtimeFactory(attemptDirectory, record);
    held(path.dirname(path.resolve(attemptDirectory)) === state && path.basename(attemptDirectory) === record.attempt.attemptId, 'owned original attempt');
    const recovery = record.schemaPreparationRecovery;
    held(recovery && JSON.stringify(recovery.effectiveIdentity) === JSON.stringify(identity), 'recovery identity');
    const root = path.join(attemptDirectory, 'native-runtime-schema-recovery'), work = path.join(root, 'work'), temp = path.join(root, 'tmp');
    await fs.mkdir(root, { mode: 0o700 }); await fs.mkdir(work); await fs.mkdir(temp);
    await exclusive(path.join(work, '.director-text-root'), 'director SDK text execution\n');
    const plan = { version: 'sdk-native-runtime/v1', attemptId: record.attempt.attemptId, identity,
      model: record.attempt.request.model, reasoning: record.attempt.request.reasoning,
      compiledPromptSha256: record.compiledPromptSha256, compiledSchemaSha256: record.compiledSchemaSha256,
      runtimeSchemaSha256: hash(JSON.stringify(JSON.parse(record.compiledSchema))),
      semanticRequestHash: record.attempt.semanticRequestHash, executionFingerprint: record.attempt.executionFingerprint,
      workingDirectory: work, tempDirectory: temp };
    const body = JSON.stringify(plan), file = path.join(root, 'runtime-plan.json'); await exclusive(file, body);
    held(await fileHash(identity.supervisorExecutable) === identity.supervisorSha256
      && await fileHash(identity.runtimeExecutable) === identity.runtimeSha256, 'runtime bytes changed');
    const env = { DIRECTOR_SDK_RUNTIME_PLAN: file, DIRECTOR_SDK_RUNTIME_PLAN_SHA256: hash(body),
      HOME: identity.credentialHome, USERPROFILE: identity.credentialHome, TEMP: temp, TMP: temp, TMPDIR: temp };
    held(process.env.SystemRoot && path.isAbsolute(process.env.SystemRoot), 'Windows system directory');
    env.SystemRoot = process.env.SystemRoot; env.WINDIR = process.env.SystemRoot;
    return { executable: identity.supervisorExecutable, workingDirectory: work, temporaryDirectory: temp, env };
  } };
  const bridge = new OfficialSdkBridge({ stateDirectory: state, nativeRuntime: runtime, emit });
  await bridge.initialize();
  const handle = bridge.handle.bind(bridge);
  bridge.handle = async message => {
    if (message.method !== 'recover-schema-preparation') {
      held(['hello', 'inspect', 'observe', 'cancel', 'shutdown'].includes(message.method), 'ordinary execute is disabled');
      return handle(message);
    }
    held(message.protocolVersion === 'director-sdk-bridge/v1' && typeof message.requestId === 'string' && message.requestId.length > 0, 'request protocol');
    const record = bridge.records.get(message.attemptId), proof = message.params?.proof, evidence = message.params?.evidence;
    held(record && !bridge.active && !bridge.closed && [...bridge.records.values()].every(r => r === record || r.state === 'transport_completed'), 'unknown worker budget');
    held(record.state === 'submission_unknown' && record.error?.code === 'ENOENT' && !record.schemaPreparationRecovery
      && !record.preDispatchRecovery && !record.wrapperPreparationRepair && record.lastSequence === 0 && record.threadId === null
      && record.turnId === null && record.rawResult === null && record.rawResultSha256 === null && record.usage === null
      && record.journalSha256 === hash('') && identityBound(record.attempt.runtimeIdentity, identity), 'original pinned preparation failure');
    held(proof?.version === VERSION && proof.attemptId === message.attemptId && proof.attemptEnvelopeHash === record.attempt.attemptEnvelopeHash
      && proof.controllerSha256 === controllerSha
      && proof.evidenceSha256 === hash(JSON.stringify(evidence)) && proof.effectiveIdentitySha256 === hash(JSON.stringify(identity))
      && proof.sdkEvents === 0 && proof.processTreeStopProven === true && proof.beforeOfficialRuntimeSpawn === true
      && evidence?.sdkImplementationSha256 === SDK_SHA && evidence.schemaPreparationBeforeExec === true, 'versioned proof');
    const dir = path.join(state, message.attemptId), file = path.join(dir, 'receipt.json'), old = await bytes(file), original = JSON.parse(old);
    await directory(dir, ['receipt.json', 'native-runtime']);
    held(Object.keys(original).length === 19 && hash(old) === proof.originalReceiptSha256 && hash(old) === evidence.originalReceiptSha256
      && original.receiptEnvelopeSha256 === proof.originalReceiptSeal && original.receiptEnvelopeSha256 === evidence.originalReceiptSeal, 'original receipt bytes');
    const { receiptEnvelopeSha256, ...payload } = original;
    held(hash(JSON.stringify(payload)) === receiptEnvelopeSha256 && JSON.stringify(payload) === JSON.stringify(Object.fromEntries(Object.entries(record).filter(([k]) => k !== 'receiptEnvelopeSha256'))), 'original sealed payload');
    const list = await inventory(path.join(dir, 'native-runtime')), plan = JSON.parse(await bytes(path.join(dir, 'native-runtime/runtime-plan.json')));
    held(hash(JSON.stringify(list)) === proof.runtimeInventorySha256 && proof.runtimeInventorySha256 === evidence.runtimeInventorySha256
      && JSON.stringify(list) === JSON.stringify(evidence.runtimeInventory), 'original runtime inventory');
    held(Object.keys(plan).length === 12 && plan.attemptId === message.attemptId && JSON.stringify(plan.identity) === JSON.stringify(record.attempt.runtimeIdentity)
      && plan.semanticRequestHash === record.attempt.semanticRequestHash && plan.executionFingerprint === record.attempt.executionFingerprint
      && plan.compiledPromptSha256 === record.compiledPromptSha256 && plan.compiledSchemaSha256 === record.compiledSchemaSha256, 'original plan');
    const template = path.join(plan.tempDirectory, 'codex-output-schema-XXXXXX');
    held(template.length >= 260 && original.error.message === `ENOENT: no such file or directory, mkdtemp '${template}'`, 'exact local schema failure');
    const rows = evidence.events;
    held(Array.isArray(rows) && hash(JSON.stringify(rows)) === evidence.eventsSha256, 'original event evidence');
    const ack = rows.filter(r => r.type === 'sdk_dispatch_receipt'), observed = rows.filter(r => r.type === 'sdk_observation'), stopped = rows.filter(r => r.type === 'sdk_process_tree'), stderr = rows.filter(r => r.type === 'sdk_stderr');
    held(ack.length === 1 && observed.length === 1 && stopped.length === 1 && stderr.length === 1
      && ack[0].id < observed[0].id && observed[0].id < stopped[0].id && stopped[0].id < stderr[0].id
      && stopped[0].data.processTreeStopProven === true && stopped[0].data.activeProcesses === 0 && stopped[0].data.totalProcesses === 2
      && stderr[0].data.bytes === 0 && stderr[0].data.retainedBytes === 0 && stderr[0].data.truncated === false
      && rows.every((r, i) => (!i || rows[i - 1].id < r.id) && ['sdk_pre_dispatch_queued', 'sdk_dispatch_receipt', 'sdk_observation', 'sdk_process_tree', 'sdk_stderr', 'state'].includes(r.type)), 'ordered stopped process tree evidence');
    held(ack[0].data.attemptId === message.attemptId && observed[0].data.attemptId === message.attemptId
      && ack[0].data.requestId === observed[0].data.requestId && ack[0].data.result.state === 'dispatching', 'original parent acknowledgement');
    for (const key of ['semanticRequestHash', 'executionFingerprint', 'attemptEnvelopeHash']) held(ack[0].data.result[key] === record.attempt[key] && observed[0].data[key] === record.attempt[key], 'parent request hashes');
    for (const key of ['state', 'error', 'lastSequence', 'threadId', 'turnId', 'rawResult', 'usage']) held(JSON.stringify(observed[0].data[key]) === JSON.stringify(record[key]), 'original observation');
    // Each exclusive file consumes the attempt even if a later write crashes.
    // A reserved recovery is observed after restart; it is never dispatched twice.
    await exclusive(path.join(dir, 'schema-preparation-original.json'), old);
    await exclusive(path.join(dir, 'schema-preparation-evidence.json'), JSON.stringify(evidence));
    await exclusive(path.join(dir, 'schema-preparation-recovery.json'), JSON.stringify(proof));
    record.schemaPreparationRecovery = { proof, effectiveIdentity: identity };
    record.state = 'dispatching'; record.error = null; await atomicRecord(file, record);
    const result = bridge.dispatchRecord(record, message.requestId, JSON.parse(record.compiledSchema));
    return { protocolVersion: 'director-sdk-bridge/v1', requestId: message.requestId, attemptId: message.attemptId, type: 'response', ok: true, result };
  };
  return bridge;
}

async function main() {
  const args = process.argv.slice(2); held(args.length === 4 && args[0] === '--state-dir' && args[2] === '--native-config', 'explicit original state/config arguments');
  const config = JSON.parse(await bytes(args[3]));
  let stopped = false, buffered = '', chain = Promise.resolve(), pending = 0;
  const decoder = new TextDecoder('utf-8', { fatal: true });
  const send = value => { const line = JSON.stringify(value) + '\n'; held(Buffer.byteLength(line) <= 2097152, 'bounded output'); process.stdout.write(line); };
  const bridge = await createSchemaRecoveryBridge({ stateDirectory: args[1], nativeConfig: config, emit: send });
  const stop = async () => { if (stopped) return; stopped = true; await bridge.disconnect('SHUTDOWN'); setTimeout(() => process.exit(0), 25); };
  process.on('SIGTERM', () => void stop()); process.on('SIGINT', () => void stop());
  const seen = new Set();
  process.stdin.on('data', chunk => {
    try { buffered += decoder.decode(chunk, { stream: true }); } catch { void stop(); return; }
    if (Buffer.byteLength(buffered) > 2097152) { void stop(); return; }
    let newline;
    while ((newline = buffered.indexOf('\n')) >= 0) {
      const line = buffered.slice(0, newline); buffered = buffered.slice(newline + 1);
      if (++pending > 32) { void stop(); return; }
      chain = chain.then(async () => {
        let message;
        try { message = JSON.parse(line); held(message.protocolVersion === 'director-sdk-bridge/v1' && typeof message.requestId === 'string' && !seen.has(message.requestId) && seen.size < 2048, 'request identity'); seen.add(message.requestId); send(await bridge.handle(message)); if (message.method === 'shutdown') await stop(); }
        catch (error) { send({ protocolVersion: 'director-sdk-bridge/v1', requestId: message?.requestId ?? null, attemptId: message?.attemptId ?? null, type: 'response', ok: false, error: { code: error.code ?? 'SDK_SCHEMA_PREPARATION_RECOVERY_HELD', message: String(error.message).slice(0, 4096) } }); }
        finally { pending--; }
      });
    }
  });
  process.stdin.on('end', () => void chain.then(stop));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
