// Genuine pinned SDK, synthetic subprocess only. No account/network/model call.
import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { OfficialSdkBridge } from '../src/bridge.mjs';
import { nativeRuntime } from '../src/native-runtime.mjs';
import { createSchemaRecoveryBridge } from '../src/schema-preparation-controller.mjs';
import { offlineFixture } from './fixture-runtime.mjs';
const h = body => createHash('sha256').update(body).digest('hex');
const sourceResources = fileURLToPath(new URL('../', import.meta.url));
const bridgeSha = '48af55f1835bd6959ede5801fa8ff2ca9068b267b36ec9f751949efdc9481dc7';
const sdkSha = 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f';
let sequence = 0;
const command = (method, params = {}, id = 'schema-attempt') => ({ protocolVersion: 'director-sdk-bridge/v1', requestId: `schema_${++sequence}`, attemptId: id, method, params });
async function fixture(t, outcome = 'happy') {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-schema-controller-'));
  t.after(async () => { assert.equal(path.dirname(root),path.resolve(os.tmpdir())); assert.match(path.basename(root),/^director-sdk-schema-controller-/); await fs.rm(root, { recursive: true, force: true }); });
  // This recovery contract belongs to the exact old package. Never substitute
  // today's bridge or relax its content pin just because the compiler evolved.
  const resources = path.join(root, 'pinned-resources');
  await fs.mkdir(path.join(resources, 'src'), { recursive: true });
  for (const name of ['main.mjs', 'native-runtime.mjs']) await fs.copyFile(path.join(sourceResources, 'src', name), path.join(resources, 'src', name));
  await fs.copyFile(fileURLToPath(new URL('./fixtures/bridge-schema-preparation-v1.mjs',import.meta.url)), path.join(resources,'src/bridge.mjs'));
  await fs.cp(path.join(sourceResources,'node_modules/@openai/codex-sdk'),path.join(resources,'node_modules/@openai/codex-sdk'),{recursive:true});
  assert.equal(h(Buffer.concat(await Promise.all(['main.mjs','bridge.mjs','native-runtime.mjs'].map(name=>fs.readFile(path.join(resources,'src',name)))))),bridgeSha);
  const suffix = path.join('schema-attempt', 'native-runtime', 'tmp');
  const worker = path.join(root, 'x'.repeat(233 - root.length - suffix.length - 2));
  await fs.mkdir(worker);
  const executableSha = h(await fs.readFile(process.execPath));
  const identity = { version: 'official-sdk-identity/v2', provider: 'codex_sdk', permissionProfile: 'sdk-text-only-native/v1',
    sdkImplementationSha256: sdkSha, bridgeSha256: bridgeSha, resourceDirectory: resources,
    runtimeExecutable: process.execPath, runtimeSha256: executableSha, supervisorExecutable: process.execPath, supervisorSha256: executableSha,
    credentialHome: path.join(root, 'never-read-home'), codexHome: path.join(root, 'never-read-codex') };
  const config = { version: 'sdk-native-config/v1', identity };
  const original = new OfficialSdkBridge({ stateDirectory: worker, nativeRuntime: nativeRuntime(config, worker) });
  await original.initialize();
  const schema = JSON.stringify({ type: 'object', properties: { ok: { type: 'boolean' }, text: { type: 'string' } }, required: ['ok', 'text'], additionalProperties: false });
  const prompt = 'Synthetic schema preparation contract only, 保留原句。';
  const attempt = { version: 'text-attempt/v2', attemptId: 'schema-attempt', workflowId: 'schema-workflow', logicalNodeId: 'schema-node', backend: 'codex_sdk',
    request: { stage: 'fixture', skill: 'fixture', model: 'gpt-6.1-sol', reasoning: 'medium', input: { synthetic: true }, schema: JSON.parse(schema), ruleSnapshot: {} },
    runtimeIdentity: identity, semanticRequestHash: 'a'.repeat(64), executionFingerprint: 'b'.repeat(64), attemptEnvelopeHash: 'c'.repeat(64),
    state: 'prepared', threadId: null, turnId: null, lastSequence: 0, rawResult: null };
  const start = command('execute', { attempt, compiledPrompt: prompt, compiledSchema: schema, compiledPromptSha256: h(prompt), compiledSchemaSha256: h(schema) });
  const ack = await original.handle(start); await original.active?.task;
  const observation = { ...original.observe('schema-attempt'), requestId: start.requestId, protocolVersion: 'director-sdk-bridge/v1' };
  assert.equal(observation.state, 'submission_unknown'); assert.equal(observation.error.code, 'ENOENT');
  await original.disconnect();
  const directory = path.join(worker, 'schema-attempt'), failed = await fs.readFile(path.join(directory, 'receipt.json'));
  const record = JSON.parse(failed), runtime = path.join(directory, 'native-runtime'), inventory = [];
  for (const name of ['runtime-plan.json', 'work/.director-text-root']) { const bytes = await fs.readFile(path.join(runtime, name)); inventory.push([name, bytes.length, h(bytes)]); }
  const events = [{ id: 1, type: 'sdk_dispatch_receipt', data: ack }, { id: 2, type: 'sdk_observation', data: observation },
    { id: 3, type: 'sdk_process_tree', data: { processTreeStopProven: true, activeProcesses: 0, totalProcesses: 2 } },
    { id: 4, type: 'sdk_stderr', data: { bytes: 0, retainedBytes: 0, truncated: false } }, { id: 5, type: 'state', data: { state: 'submission_unknown' } }];
  const evidence = { version: 'sdk-schema-preparation-evidence/v1', attemptId: attempt.attemptId, attemptEnvelopeHash: attempt.attemptEnvelopeHash,
    originalReceiptSha256: h(failed), originalReceiptSeal: record.receiptEnvelopeSha256,
    runtimeInventory: inventory, runtimeInventorySha256: h(JSON.stringify(inventory)), events, eventsSha256: h(JSON.stringify(events)),
    sdkImplementationSha256: sdkSha, schemaPreparationBeforeExec: true, sdkEvents: 0, processTreeStopProven: true, beforeOfficialRuntimeSpawn: true };
  const proof = { version: 'sdk-schema-preparation-recovery/v1', attemptId: attempt.attemptId, attemptEnvelopeHash: attempt.attemptEnvelopeHash,
    originalReceiptSha256: h(failed), originalReceiptSeal: record.receiptEnvelopeSha256, runtimeInventorySha256: evidence.runtimeInventorySha256,
    evidenceSha256: h(JSON.stringify(evidence)), effectiveIdentitySha256: h(JSON.stringify(identity)),
    controllerSha256: h(await fs.readFile(fileURLToPath(new URL('../src/schema-preparation-controller.mjs', import.meta.url)))),
    sdkEvents: 0, processTreeStopProven: true, beforeOfficialRuntimeSpawn: true };
  let invocations = 0;
  const runtimeFactory = async dir => {
    invocations++; if (outcome === 'fails') throw new Error('synthetic failure after consumed schema recovery');
    // Node's main-module loader requires an ordinary script spelling; the
    // separate schema temp remains namespaced, just like the Rust launcher.
    const ordinary = dir.startsWith('\\\\?\\') ? dir.slice(4) : dir;
    const runtime = await offlineFixture('happy').create(ordinary), temp = path.join(dir, 'native-runtime-schema-recovery');
    await fs.mkdir(temp); return { ...runtime, temporaryDirectory: temp };
  };
  const create = () => createSchemaRecoveryBridge({ stateDirectory: path.toNamespacedPath(worker), nativeConfig: config, runtimeFactory });
  return { root, worker, directory, failed, record, proof, evidence, identity, config, create, invocations: () => invocations };
}
test('schema recovery continues the same original attempt once and preserves failed receipt, request and runtime', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t), bridge = await f.create();
  try {
    const result = await bridge.handle(command('recover-schema-preparation', { proof: f.proof, evidence: f.evidence })); await bridge.active?.task;
    assert.equal(result.ok, true); assert.equal(bridge.observe('schema-attempt').state, 'transport_completed', JSON.stringify(bridge.observe('schema-attempt').error)); assert.equal(f.invocations(), 1);
    assert.deepEqual(await fs.readFile(path.join(f.directory, 'schema-preparation-original.json')), f.failed);
    assert.deepEqual(JSON.parse(await fs.readFile(path.join(f.directory, 'receipt.json'))).attempt, f.record.attempt);
    assert.equal(h(await fs.readFile(path.join(f.directory, 'native-runtime/runtime-plan.json'))), f.evidence.runtimeInventory[0][2]);
    await assert.rejects(bridge.handle(command('recover-schema-preparation', { proof: f.proof, evidence: f.evidence })), { code: 'SDK_SCHEMA_PREPARATION_RECOVERY_HELD' });
    await assert.rejects(bridge.handle(command('execute')), { code: 'SDK_SCHEMA_PREPARATION_RECOVERY_HELD' });
  } finally { await bridge.disconnect(); }
  const reopened = await f.create(); try { assert.equal(reopened.observe('schema-attempt').state, 'transport_completed'); assert.equal(f.invocations(), 1); } finally { await reopened.disconnect(); }
});
test('schema recovery failure consumes its intent and stays unknown after restart without another invocation', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t, 'fails'), bridge = await f.create();
  await bridge.handle(command('recover-schema-preparation', { proof: f.proof, evidence: f.evidence })); await bridge.active?.task;
  assert.equal(bridge.observe('schema-attempt').state, 'submission_unknown'); await bridge.disconnect();
  const reopened = await f.create(); try { await assert.rejects(reopened.handle(command('recover-schema-preparation', { proof: f.proof, evidence: f.evidence })), { code: 'SDK_SCHEMA_PREPARATION_RECOVERY_HELD' }); assert.equal(f.invocations(), 1); } finally { await reopened.disconnect(); }
});
for (const fault of ['proof', 'controller', 'receipt', 'runtime', 'event', 'stopped', 'active', 'process-count', 'order', 'source', 'account']) {
  test(`schema recovery rejects ${fault} before an SDK invocation`, { skip: process.platform !== 'win32' }, async t => {
    const f = await fixture(t);
    if (fault === 'proof') f.proof.originalReceiptSha256 = 'f'.repeat(64);
    if (fault === 'controller') f.proof.controllerSha256 = 'f'.repeat(64);
    if (fault === 'receipt') await fs.writeFile(path.join(f.directory, 'receipt.json'), '{}');
    if (fault === 'runtime') await fs.writeFile(path.join(f.directory, 'native-runtime/tmp/foreign'), '{}');
    if (fault === 'event') f.evidence.events.push({ id: 6, type: 'sdk_event', data: {} });
    if (fault === 'stopped') f.evidence.events[2].data.processTreeStopProven = false;
    if (fault === 'active') f.evidence.events[2].data.activeProcesses = 1;
    if (fault === 'process-count') f.evidence.events[2].data.totalProcesses = 3;
    if (fault === 'order') f.evidence.events[2].id = 1;
    if (fault === 'source') f.config.identity = { ...f.identity, resourceDirectory: f.root };
    if (fault === 'account') f.config.identity = { ...f.identity, credentialHome: f.root };
    if (['event', 'stopped', 'active', 'process-count', 'order'].includes(fault)) { f.evidence.eventsSha256 = h(JSON.stringify(f.evidence.events)); f.proof.evidenceSha256 = h(JSON.stringify(f.evidence)); }
    let bridge;
    try { bridge = await f.create(); await assert.rejects(bridge.handle(command('recover-schema-preparation', { proof: f.proof, evidence: f.evidence }))); }
    catch (error) { if (!['receipt', 'source'].includes(fault)) throw error; }
    finally { await bridge?.disconnect(); }
    assert.equal(f.invocations(), 0);
    await assert.rejects(fs.stat(path.join(f.directory, 'schema-preparation-recovery.json')), { code: 'ENOENT' });
  });
}

test('standalone schema controller accepts bounded observation but cannot execute an ordinary or unproven request', { skip: process.platform !== 'win32' }, async t => {
  const f = await fixture(t), configFile = path.join(f.root, 'native-config.json');
  await fs.writeFile(configFile, JSON.stringify(f.config));
  const child = spawn(process.execPath, [fileURLToPath(new URL('../src/schema-preparation-controller.mjs', import.meta.url)),
    '--state-dir', path.toNamespacedPath(f.worker), '--native-config', configFile], {
    windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], env: { SystemRoot: process.env.SystemRoot, WINDIR: process.env.SystemRoot },
  });
  let pending = '', stderr = ''; const messages = [], waiters = [];
  child.stdout.on('data', bytes => {
    pending += bytes.toString('utf8'); let end;
    while ((end = pending.indexOf('\n')) >= 0) { const value = JSON.parse(pending.slice(0, end)); pending = pending.slice(end + 1); messages.push(value); for (const notify of waiters.splice(0)) notify(); }
  });
  child.stderr.on('data', bytes => { stderr += bytes.toString('utf8'); });
  const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({ code, signal })));
  t.after(async () => { if (child.exitCode === null) child.kill(); await exited; });
  const request = async message => {
    const line = Buffer.from(JSON.stringify(message) + '\n');
    // Split inside Chinese synthetic metadata to verify UTF-8 stream handling.
    const mark = line.indexOf(Buffer.from('测')); if (mark >= 0) { child.stdin.write(line.subarray(0, mark + 1)); child.stdin.write(line.subarray(mark + 1)); } else child.stdin.write(line);
    const deadline = Date.now() + 5000;
    while (true) {
      const result = messages.find(m => m.requestId === message.requestId); if (result) return result;
      assert(Date.now() < deadline, `controller response timed out: ${stderr}`);
      await Promise.race([new Promise(resolve => waiters.push(resolve)), new Promise(resolve => setTimeout(resolve, 25))]);
    }
  };
  const hello = command('hello', { syntheticNote: '中文测试' }, null); assert.equal((await request(hello)).ok, true);
  assert.equal((await request(command('observe'))).result.state, 'submission_unknown');
  assert.equal((await request(command('execute'))).ok, false);
  assert.equal((await request(command('recover-schema-preparation', { proof: { version: 'unproven' } }))).ok, false);
  assert.equal((await request(command('shutdown', {}, null))).ok, true); assert.equal((await exited).code, 0);
  assert.deepEqual(await fs.readFile(path.join(f.directory, 'receipt.json')), f.failed);
  await assert.rejects(fs.stat(path.join(f.directory, 'schema-preparation-recovery.json')), { code: 'ENOENT' });
  assert.equal(f.invocations(), 0);
});
