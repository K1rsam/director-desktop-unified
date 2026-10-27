// Separate, Rust-content-pinned recovery entry. The frozen legacy bridge and
// SDK resources remain byte-for-byte unchanged; ordinary execute is unavailable.
import { promises as fs } from 'node:fs';
import { createReadStream } from 'node:fs';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const VERSION = 'sdk-lane-preparation-recovery/v1';
const BRIDGE_SHA = 'acf04b60785355135627fe791ba35e771c8ad59937dc9df0cfe365190c43e7f2';
const FAILED_SUPERVISOR_SHA = '80349b58f08959b2461c3e24e51caa64869913f3c73b5428c9328e9a4fb96db7';
const SDK_SHA = 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const held = (ok, text) => { if (!ok) throw Object.assign(new Error(`SDK_LANE_PREPARATION_RECOVERY_HELD: ${text}`), { code: 'SDK_LANE_PREPARATION_RECOVERY_HELD' }); };
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
  await directory(root, ['runtime-plan.json','work','tmp']); await directory(path.join(root,'work'), ['.director-text-root']);
  const entries = await fs.readdir(path.join(root,'tmp'));
  held(entries.length === 1 && /^codex-output-schema-[A-Za-z0-9_-]+$/.test(entries[0]), 'schema directory');
  await directory(path.join(root,'tmp',entries[0]), ['schema.json']);
  const out=[];
  for (const name of ['runtime-plan.json', `tmp/${entries[0]}/schema.json`, 'work/.director-text-root']) {
    const body=await bytes(path.join(root,name)); if(name.startsWith('work/')) held(body.equals(Buffer.from('director SDK text execution\n')), 'work marker changed');
    out.push([name,body.length,hash(body)]);
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
  const temp = `${file}.lane-recovery-${process.pid}`;
  await exclusive(temp, body); await fs.rename(temp, file);
}
function identityBound(old, current) {
  const expected = { ...old };
  for (const key of ['supervisorExecutable', 'supervisorSha256', 'resourceDirectory']) expected[key] = current[key];
  return old.supervisorSha256 === FAILED_SUPERVISOR_SHA && current.supervisorSha256 !== FAILED_SUPERVISOR_SHA && old.bridgeSha256 === BRIDGE_SHA && current.bridgeSha256 === BRIDGE_SHA
    && JSON.stringify(expected) === JSON.stringify(current);
}

export async function createLaneRecoveryBridge({ stateDirectory, nativeConfig, emit = () => {}, runtimeFactory = null }) {
  held(path.isAbsolute(stateDirectory) && nativeConfig?.version === 'sdk-native-config/v1'
    && Object.keys(nativeConfig).sort().join(',') === 'identity,version', 'native configuration');
  const identity = nativeConfig.identity, resources = identity.resourceDirectory;
  const controllerSha = hash(await bytes(fileURLToPath(import.meta.url)));
  held(path.isAbsolute(resources) && identity.bridgeSha256 === BRIDGE_SHA && identity.sdkImplementationSha256 === SDK_SHA, 'pinned original execution version');
  const source = await Promise.all(['main.mjs', 'bridge.mjs', 'native-runtime.mjs'].map(name => bytes(path.join(resources, 'src', name))));
  held(hash(Buffer.concat(source)) === BRIDGE_SHA, 'legacy bridge source changed');
  const sdk = await bytes(path.join(resources, 'node_modules/@openai/codex-sdk/dist/index.js'));
  held(hash(sdk) === SDK_SHA, 'official SDK changed');
  const { OfficialSdkBridge } = await import(pathToFileURL(path.join(resources, 'src/bridge.mjs')));
  const state = path.resolve(stateDirectory);
  const runtime = { identity, async create(attemptDirectory, record) {
    if (runtimeFactory) return runtimeFactory(attemptDirectory, record);
    held(path.dirname(path.resolve(attemptDirectory)) === state && path.basename(attemptDirectory) === record.attempt.attemptId, 'owned original attempt');
    const recovery = record.lanePreparationRecovery;
    held(recovery && JSON.stringify(recovery.effectiveIdentity) === JSON.stringify(identity), 'recovery identity');
    const root = path.join(attemptDirectory, 'native-runtime-lane-recovery'), work = path.join(root, 'work'), temp = path.join(root, 'tmp');
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
  const handle = bridge.handle.bind(bridge);
  bridge.handle = async message => {
    if (message.method !== 'recover-lane-preparation') {
      held(['hello', 'inspect', 'observe', 'cancel', 'shutdown'].includes(message.method), 'ordinary execute is disabled');
      return handle(message);
    }
    held(message.protocolVersion === 'director-sdk-bridge/v1' && typeof message.requestId === 'string' && message.requestId.length > 0, 'request protocol');
    held(!bridge.initialized && !bridge.active && !bridge.closed, 'one-use controller');
    const proof=message.params?.proof, evidence=message.params?.evidence;
    held(typeof message.attemptId === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(message.attemptId), 'attempt identity');
    const dir=path.join(state,message.attemptId),file=path.join(dir,'receipt.json'),old=await bytes(file),original=JSON.parse(old);
    await directory(dir,['receipt.json','native-runtime']);
    const {receiptEnvelopeSha256,...payload}=original;
    held(Object.keys(original).length===19 && hash(JSON.stringify(payload))===receiptEnvelopeSha256
      && original.state==='dispatching' && original.error===null && original.lastSequence===0
      && ['threadId','turnId','rawResult','rawResultSha256','usage'].every(k=>original[k]===null)
      && original.journalSha256===hash('') && identityBound(original.attempt.runtimeIdentity,identity), 'pinned original receipt');
    const marker=original.attempt.request.input.foundationParallelTrial;
    held(marker?.version==='foundation-three-lane-trial/v1' && marker.concurrency===3
      && marker.workflowId===original.attempt.workflowId && /^[a-f0-9]{64}$/.test(marker.authorizationSha256)
      && [0,1,2].includes(marker.laneId) && ['foundation:F2','foundation:Q1'].includes(original.attempt.request.stage)
      && path.basename(state)===`lane-${marker.laneId}` && path.basename(path.dirname(state))===marker.authorizationSha256
      && path.basename(path.dirname(path.dirname(state)))==='sdk-worker-trials', 'frozen lane directory');
    held(proof?.version===VERSION && proof.attemptId===message.attemptId && proof.attemptEnvelopeHash===original.attempt.attemptEnvelopeHash
      && proof.controllerSha256===controllerSha && proof.evidenceSha256===hash(JSON.stringify(evidence))
      && proof.effectiveIdentitySha256===hash(JSON.stringify(identity)) && proof.sdkEvents===0
      && proof.processTreeStopProven===true && proof.beforeOfficialRuntimeSpawn===true
      && evidence?.failedSupervisorSha256===FAILED_SUPERVISOR_SHA && evidence.sdkImplementationSha256===SDK_SHA
      && proof.originalReceiptSha256===hash(old) && evidence.originalReceiptSha256===hash(old)
      && proof.originalReceiptSeal===receiptEnvelopeSha256 && evidence.originalReceiptSeal===receiptEnvelopeSha256, 'bound recovery proof');
    const list=await inventory(path.join(dir,'native-runtime')),plan=JSON.parse(await bytes(path.join(dir,'native-runtime/runtime-plan.json')));
    held(hash(JSON.stringify(list))===proof.runtimeInventorySha256 && proof.runtimeInventorySha256===evidence.runtimeInventorySha256
      && JSON.stringify(list)===JSON.stringify(evidence.runtimeInventory) && list[1][2]===plan.runtimeSchemaSha256, 'original runtime inventory');
    held(Object.keys(plan).length===12 && plan.attemptId===message.attemptId && JSON.stringify(plan.identity)===JSON.stringify(original.attempt.runtimeIdentity)
      && plan.semanticRequestHash===original.attempt.semanticRequestHash && plan.executionFingerprint===original.attempt.executionFingerprint
      && plan.compiledPromptSha256===original.compiledPromptSha256 && plan.compiledSchemaSha256===original.compiledSchemaSha256
      && path.resolve(plan.workingDirectory)===path.resolve(dir,'native-runtime/work')
      && path.resolve(plan.tempDirectory)===path.resolve(dir,'native-runtime/tmp'), 'original plan');
    const rows=evidence.events;
    held(Array.isArray(rows) && hash(JSON.stringify(rows))===evidence.eventsSha256
      && rows.every((r,i)=>(!i||rows[i-1].id<r.id)&&['state','sdk_supervisor_handoff_frozen','sdk_pre_dispatch_queued','sdk_dispatch_receipt','sdk_transport_unknown','sdk_process_tree','sdk_stderr'].includes(r.type)), 'ordered original events');
    const tail=rows.filter(r=>!['state','sdk_supervisor_handoff_frozen','sdk_pre_dispatch_queued'].includes(r.type));
    held(JSON.stringify(tail.map(r=>r.type))===JSON.stringify(['sdk_dispatch_receipt','sdk_transport_unknown','sdk_process_tree','sdk_stderr']), 'event types');
    const [ack,unknown,stopped,stderr]=tail.map(r=>r.data);
    held(ack.ok===true&&ack.protocolVersion==='director-sdk-bridge/v1'&&ack.attemptId===message.attemptId&&ack.result.state==='dispatching'&&ack.result.lastSequence===0
      && unknown.attemptId===message.attemptId&&unknown.lastSdkSequence===0&&unknown.error==='SDK_STDERR_CLOSED'
      && stopped.processTreeStopProven===true&&stopped.activeProcesses===0&&stopped.totalProcesses===3
      && stderr.truncated===false&&stderr.bytes===stderr.retainedBytes&&stderr.bytes>0&&stderr.bytes<16384, 'zero-runtime stopped tree');
    for(const key of ['semanticRequestHash','executionFingerprint','attemptEnvelopeHash']) held(ack.result[key]===original.attempt[key],'parent request hash');
    for(const key of ['compiledPromptSha256','compiledSchemaSha256']) held(ack.result[key]===original[key],'parent input hash');
    held((await bytes(file)).equals(old), 'receipt drift');
    // Each exclusive file consumes the attempt even if a later write crashes.
    // A reserved recovery is observed after restart; it is never dispatched twice.
    await exclusive(path.join(dir, 'lane-preparation-original.json'), old);
    await exclusive(path.join(dir, 'lane-preparation-evidence.json'), JSON.stringify(evidence));
    await exclusive(path.join(dir, 'lane-preparation-recovery.json'), JSON.stringify(proof));
    await bridge.initialize();
    const record=bridge.records.get(message.attemptId);
    held(record && !bridge.active && [...bridge.records.values()].every(r=>r===record||r.state==='transport_completed'), 'worker budget');
    record.lanePreparationRecovery = { proof, effectiveIdentity: identity };
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
  const bridge = await createLaneRecoveryBridge({ stateDirectory: args[1], nativeConfig: config, emit: send });
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
        catch (error) { send({ protocolVersion: 'director-sdk-bridge/v1', requestId: message?.requestId ?? null, attemptId: message?.attemptId ?? null, type: 'response', ok: false, error: { code: error.code ?? 'SDK_LANE_PREPARATION_RECOVERY_HELD', message: String(error.message).slice(0, 4096) } }); }
        finally { pending--; }
      });
    }
  });
  process.stdin.on('end', () => void chain.then(stop));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
