import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { OfficialSdkBridge, PROTOCOL_VERSION, sha256, LEGACY_PATH_BRIDGE_SHA, WRAPPER_FAILED_BRIDGE_SHA, WRAPPER_FAILED_SUPERVISOR_SHA } from '../src/bridge.mjs';
import { offlineFixture } from './fixture-runtime.mjs';

const main = fileURLToPath(new URL('../src/main.mjs', import.meta.url));
const syntheticSchema = { type: 'object', properties: { ok: { type: 'boolean' }, text: { type: 'string' } }, required: ['ok', 'text'], additionalProperties: false };
let serial = 0;
function command(method, attemptId = null, params) { return { protocolVersion: PROTOCOL_VERSION, requestId: `request_${++serial}`, attemptId, method, ...(params ? { params } : {}) }; }
function executeCommand(attemptId = 'attempt_1') {
  const compiledPrompt = 'Synthetic offline fixture only. 保留原句，不修复。';
  const compiledSchema = JSON.stringify(syntheticSchema);
  return command('execute', attemptId, {
    attempt: { version: 'text-attempt/v2', attemptId, workflowId: 'workflow_fixture', logicalNodeId: 'node_fixture', backend: 'codex_sdk', request: { stage: 'fixture', skill: 'fixture', model: 'gpt-6.1-sol', reasoning: 'medium', input: { synthetic: true }, schema: syntheticSchema, ruleSnapshot: { synthetic: true } }, semanticRequestHash: 'frozen-semantic-test-hash', executionFingerprint: 'frozen-execution-test-hash', state: 'prepared', threadId: null, turnId: null, lastSequence: 0, rawResult: null },
    compiledPrompt, compiledSchema, compiledPromptSha256: sha256(compiledPrompt), compiledSchemaSha256: sha256(compiledSchema),
  });
}
async function fixture(t, scenario = 'happy', extra = {}) {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-test-'));
  const events = [];
  const bridge = new OfficialSdkBridge({ stateDirectory: dir, emit: event => events.push(event), offlineFixture: scenario ? offlineFixture(scenario) : null, ...extra });
  await bridge.initialize();
  t.after(async () => { await bridge.disconnect(); await fs.rm(dir, { recursive: true, force: true }); });
  return { dir, bridge, events };
}
async function completed(bridge, message = executeCommand()) {
  const dispatched = await bridge.handle(message);
  const task = bridge.active?.task;
  if (task) await task;
  return { dispatched, receipt: bridge.observe(message.attemptId) };
}

async function localPreparationFixture(t, fault = null) {
  const identity = { version: 'official-sdk-identity/v2', bridgeSha256: LEGACY_PATH_BRIDGE_SHA, accountFingerprint: 'synthetic-account', runtimePackageVersion: 'synthetic-runtime', supervisorExecutable: 'synthetic-old', supervisorSha256: 'a'.repeat(64), resourceDirectory: 'synthetic-old-resources' };
  const initial = await fixture(t, null, { nativeRuntime: { identity, async create() { throw new Error('SDK_NATIVE_IDENTITY: attempt directory'); } } });
  const message = executeCommand(); message.params.attempt.runtimeIdentity = identity;
  await completed(initial.bridge, message); await initial.bridge.disconnect();
  const file = path.join(initial.dir, 'attempt_1', 'receipt.json');
  const before = await fs.readFile(file, 'utf8');
  let record = JSON.parse(before);
  if (fault === 'network') record.error.message = 'tls handshake eof';
  if (fault === 'thread') record.threadId = 'existing-provider-thread';
  if (fault === 'journal') await fs.writeFile(path.join(initial.dir, 'attempt_1/events.ndjson'), '');
  if (fault === 'runtime') await fs.mkdir(path.join(initial.dir, 'attempt_1/native-runtime'));
  if (fault === 'reserved') await fs.writeFile(path.join(initial.dir, 'attempt_1/local-preparation-recovery.json'), '{}');
  if (fault === 'network' || fault === 'thread') { const { receiptEnvelopeSha256, ...payload } = record; record = { ...payload, receiptEnvelopeSha256: sha256(JSON.stringify(payload)) }; await fs.writeFile(file, JSON.stringify(record)); }
  const current = { ...identity, bridgeSha256: 'b'.repeat(64), supervisorExecutable: 'synthetic-new', resourceDirectory: 'synthetic-new-resources' };
  if (fault === 'account') current.accountFingerprint = 'different-account';
  let invocations = 0;
  const runtime = { identity: current, async create(directory) {
    invocations++;
    if (fault === 'recover-fails') throw new Error('synthetic preparation still failed');
    const result = await offlineFixture('happy').create(directory);
    const temporaryDirectory = path.join(directory, 'schema'); await fs.mkdir(temporaryDirectory);
    return { ...result, temporaryDirectory };
  } };
  const bridge = new OfficialSdkBridge({ stateDirectory: initial.dir, nativeRuntime: runtime });
  await bridge.initialize(); t.after(() => bridge.disconnect());
  record = JSON.parse(await fs.readFile(file, 'utf8'));
  const proof = { version: 'sdk-local-preparation-recovery/v1', attemptId: 'attempt_1', attemptEnvelopeHash: record.attempt.attemptEnvelopeHash,
    originalReceiptSeal: record.receiptEnvelopeSha256, sdkEvents: 0, processTreeStopProven: true,
    effectiveIdentitySha256: sha256(JSON.stringify(current)), evidenceSha256: 'c'.repeat(64) };
  if (fault === 'stop-proof') proof.processTreeStopProven = false;
  if (fault === 'seal') proof.originalReceiptSeal = 'f'.repeat(64);
  return { ...initial, bridge, runtime, proof, before, invocations: () => invocations, message };
}

test('sealed local preparation failure resumes original attempt exactly once with original receipt and frozen request retained', async t => {
  const f = await localPreparationFixture(t);
  await assert.rejects(f.bridge.handle(f.message), { code: 'WORKER_OCCUPIED' });
  const { receipt } = await completed(f.bridge, command('recover-local-preparation', 'attempt_1', { proof: f.proof }));
  assert.equal(receipt.state, 'transport_completed'); assert.equal(f.invocations(), 1);
  assert.equal(await fs.readFile(path.join(f.dir, 'attempt_1/local-preparation-original.json'), 'utf8'), f.before);
  assert.deepEqual(JSON.parse(await fs.readFile(path.join(f.dir, 'attempt_1/receipt.json'), 'utf8')).attempt, JSON.parse(f.before).attempt);
  await assert.rejects(f.bridge.handle(command('recover-local-preparation', 'attempt_1', { proof: f.proof })), { code: 'LOCAL_RECOVERY_HELD' });
  assert.equal(f.invocations(), 1);
});

async function wrapperPreparationFixture(t, fault = null) {
  const prior = await localPreparationFixture(t); await prior.bridge.disconnect();
  const dir = path.join(prior.dir,'attempt_1'), file = path.join(dir,'receipt.json');
  const previous = { ...prior.runtime.identity, bridgeSha256: WRAPPER_FAILED_BRIDGE_SHA, supervisorSha256: WRAPPER_FAILED_SUPERVISOR_SHA };
  const current = { ...previous, bridgeSha256: 'd'.repeat(64), supervisorSha256: 'e'.repeat(64), supervisorExecutable: 'fixed-synthetic-supervisor' };
  let record = JSON.parse(prior.before); delete record.receiptEnvelopeSha256;
  const oldProof = { ...prior.proof, effectiveIdentitySha256:sha256(JSON.stringify(previous)) };
  record.preDispatchRecovery={proof:oldProof,effectiveIdentity:previous};record.state='dispatching';record.error=null;
  await fs.writeFile(path.join(dir,'local-preparation-original.json'),prior.before);
  await fs.writeFile(path.join(dir,'local-preparation-recovery.json'),JSON.stringify(oldProof));
  const native=path.join(dir,'native-runtime');await fs.mkdir(native);
  const plan=JSON.stringify({identity:previous,attemptId:'attempt_1'});await fs.writeFile(path.join(native,'runtime-plan.json'),plan);
  const inventory=[['runtime-plan.json',Buffer.byteLength(plan),sha256(plan)]];
  if(fault==='old-supervisor')record.preDispatchRecovery.effectiveIdentity={...previous,supervisorSha256:'f'.repeat(64)};
  if(fault==='events')record.lastSequence=1;
  const failed=JSON.stringify({...record,receiptEnvelopeSha256:sha256(JSON.stringify(record))});await fs.writeFile(file,failed);
  const proof={version:'sdk-wrapper-preparation-repair/v1',attemptId:'attempt_1',attemptEnvelopeHash:record.attempt.attemptEnvelopeHash,
    priorRecoveryProofSha256:sha256(JSON.stringify(oldProof)),failedReceiptSha256:sha256(failed),runtimeInventorySha256:sha256(JSON.stringify(inventory)),
    effectiveIdentitySha256:sha256(JSON.stringify(current)),evidenceSha256:'a'.repeat(64),sdkEvents:0,processTreeStopProven:true,beforeOfficialRuntimeSpawn:true};
  if(fault==='proof')proof.beforeOfficialRuntimeSpawn=false;
  if(fault==='account')current.accountFingerprint='different-account';
  if(fault==='inventory')await fs.writeFile(path.join(native,'runtime-plan.json'),'{}');
  if(fault==='intent')await fs.writeFile(path.join(dir,'wrapper-preparation-repair.json'),'{}');
  let invocations=0;
  const runtime={identity:current,async create(directory){invocations++;if(fault==='fails')throw Error('synthetic repair failed');
    const result=await offlineFixture('happy').create(directory);const temporaryDirectory=path.join(directory,'native-runtime-wrapper-repair');await fs.mkdir(temporaryDirectory);return {...result,temporaryDirectory};}};
  const bridge=new OfficialSdkBridge({stateDirectory:prior.dir,nativeRuntime:runtime});await bridge.initialize();t.after(()=>bridge.disconnect());
  return {dir:prior.dir,attemptDir:dir,bridge,runtime,proof,failed,oldIntent:JSON.stringify(oldProof),oldPlan:plan,invocations:()=>invocations};
}
for(const fault of [null,'fails'])test(`wrapper preparation repair ${fault??'completion'} preserves lineage and restart never invokes twice`,async t=>{
  const f=await wrapperPreparationFixture(t,fault);
  const {receipt}=await completed(f.bridge,command('repair-wrapper-preparation','attempt_1',{proof:f.proof}));
  assert.equal(receipt.state,fault?'submission_unknown':'transport_completed');assert.equal(f.invocations(),1);
  assert.equal(await fs.readFile(path.join(f.attemptDir,'wrapper-preparation-original.json'),'utf8'),f.failed);
  assert.equal(await fs.readFile(path.join(f.attemptDir,'local-preparation-recovery.json'),'utf8'),f.oldIntent);
  assert.equal(await fs.readFile(path.join(f.attemptDir,'native-runtime/runtime-plan.json'),'utf8'),f.oldPlan);
  await assert.rejects(f.bridge.handle(command('repair-wrapper-preparation','attempt_1',{proof:f.proof})),{code:'WRAPPER_REPAIR_HELD'});
  await f.bridge.disconnect();const reopened=new OfficialSdkBridge({stateDirectory:f.dir,nativeRuntime:f.runtime});await reopened.initialize();
  try{assert.equal(reopened.observe('attempt_1').state,receipt.state);await assert.rejects(reopened.handle(command('repair-wrapper-preparation','attempt_1',{proof:f.proof})),{code:'WRAPPER_REPAIR_HELD'});assert.equal(f.invocations(),1);}finally{await reopened.disconnect();}
});
for(const fault of ['old-supervisor','events','proof','account','inventory','intent'])test(`wrapper preparation repair rejects ${fault} without invoking runtime`,async t=>{
  const f=await wrapperPreparationFixture(t,fault);await assert.rejects(f.bridge.handle(command('repair-wrapper-preparation','attempt_1',{proof:f.proof})),{code:'WRAPPER_REPAIR_HELD'});assert.equal(f.invocations(),0);
});
for (const fault of ['network', 'thread', 'journal', 'runtime', 'reserved', 'account', 'stop-proof', 'seal']) {
  test(`local preparation recovery rejects ${fault} and makes zero SDK invocations`, async t => {
    const f = await localPreparationFixture(t, fault);
    await assert.rejects(f.bridge.handle(command('recover-local-preparation', 'attempt_1', { proof: f.proof })), { code: 'LOCAL_RECOVERY_HELD' });
    assert.equal(f.invocations(), 0);
  });
}
test('a consumed local recovery intent stays unknown after another failure and cannot dispatch again', async t => {
  const f = await localPreparationFixture(t, 'recover-fails');
  const { receipt } = await completed(f.bridge, command('recover-local-preparation', 'attempt_1', { proof: f.proof }));
  assert.equal(receipt.state, 'submission_unknown'); assert.equal(f.invocations(), 1);
  await assert.rejects(f.bridge.handle(command('recover-local-preparation', 'attempt_1', { proof: f.proof })), { code: 'LOCAL_RECOVERY_HELD' });
  assert.equal(f.invocations(), 1);
});
for (const scenario of [null, 'recover-fails']) {
  test(`restart observes consumed local recovery ${scenario ?? 'completion'} with zero new invocation`, async t => {
    const f=await localPreparationFixture(t,scenario);
    const { receipt }=await completed(f.bridge,command('recover-local-preparation','attempt_1',{proof:f.proof}));
    await f.bridge.disconnect();
    const reopened=new OfficialSdkBridge({stateDirectory:f.dir,nativeRuntime:f.runtime});await reopened.initialize();
    try {
      assert.equal((await reopened.handle(command('observe','attempt_1'))).result.state,receipt.state);
      await assert.rejects(reopened.handle(command('recover-local-preparation','attempt_1',{proof:f.proof})),{code:'LOCAL_RECOVERY_HELD'});
      assert.equal(f.invocations(),1);
      assert.equal(await fs.readFile(path.join(f.dir,'attempt_1/local-preparation-original.json'),'utf8'),f.before);
    } finally {await reopened.disconnect();}
  });
}

test('hello/inspect/execute default stays disabled and starts zero runtime processes', async t => {
  const { bridge, dir } = await fixture(t, null);
  for (const method of ['hello', 'inspect']) {
    const result = (await bridge.handle(command(method))).result;
    assert.equal(result.liveCallsAllowed, false);
    assert.equal(result.available, false);
    assert.equal(result.sdk.package, '@openai/codex-sdk');
    assert.equal(result.sdk.version, '0.160.1');
    assert.equal(result.capabilities.passiveRemoteObserve, false);
  }
  await assert.rejects(bridge.handle(executeCommand()), { code: 'SDK_LIVE_DISABLED' });
  assert.deepEqual(await fs.readdir(dir), ['worker.lock']);
});

test('genuine SDK native mode creates schema inside the dedicated attempt temp and restores parent env', async t => {
  const previous = Object.fromEntries(['TEMP', 'TMP', 'TMPDIR'].map(key => [key, process.env[key]]));
  let temporaryDirectory;
  const nativeRuntime = { identity: { runtimePackageVersion: 'offline-schema-contract-only' }, async create(directory) {
    const fixture = await offlineFixture('happy').create(directory);
    temporaryDirectory = path.join(directory, 'dedicated-sdk-schema'); await fs.mkdir(temporaryDirectory);
    return { ...fixture, temporaryDirectory };
  } };
  const { bridge, dir } = await fixture(t, null, { nativeRuntime });
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'transport_completed');
  const captured = JSON.parse(await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8'));
  const schemaPath = captured.args[captured.args.indexOf('--output-schema') + 1];
  assert(path.resolve(schemaPath).startsWith(path.resolve(temporaryDirectory) + path.sep));
  assert.deepEqual(Object.fromEntries(['TEMP', 'TMP', 'TMPDIR'].map(key => [key, process.env[key]])), previous);
  assert.deepEqual(await fs.readdir(temporaryDirectory), []);
});

test('actual SDK preserves frozen model/reasoning/prompt/schema and uses no inherited credentials', async t => {
  const { bridge, dir, events } = await fixture(t);
  process.env.DIRECTOR_SYNTHETIC_SECRET_MUST_NOT_INHERIT = 'not-a-real-secret';
  t.after(() => { delete process.env.DIRECTOR_SYNTHETIC_SECRET_MUST_NOT_INHERIT; });
  const message = executeCommand();
  const { receipt } = await completed(bridge, message);
  assert.equal(receipt.state, 'transport_completed');
  assert.equal(receipt.domainAccepted, false);
  assert.equal(receipt.threadId, 'synthetic-thread-1');
  assert.equal(receipt.turnId, null);
  assert.equal(receipt.rawResult.rawText, '{"ok":true,"text":"合成测试"}');
  assert.deepEqual(receipt.rawResult.parsedJson, { ok: true, text: '合成测试' });
  assert.equal(receipt.rawResult.parseError, null);
  const captured = JSON.parse(await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8'));
  assert.deepEqual(captured.args.slice(0, 2), ['exec', '--experimental-json']);
  assert.equal(captured.args[captured.args.indexOf('--model') + 1], message.params.attempt.request.model);
  assert.ok(captured.args.includes('model_reasoning_effort="medium"'));
  assert.ok(captured.args.includes('approval_policy="never"'));
  assert.ok(captured.args.includes('web_search="disabled"'));
  assert.ok(captured.args.includes('sandbox_workspace_write.network_access=false'));
  assert.equal(captured.args[captured.args.indexOf('--sandbox') + 1], 'read-only');
  assert.equal(captured.args.includes('resume'), false);
  assert.equal(captured.prompt, message.params.compiledPrompt);
  assert.deepEqual(captured.schema, syntheticSchema);
  assert.equal(captured.env.DIRECTOR_SYNTHETIC_SECRET_MUST_NOT_INHERIT, undefined);
  assert.equal(captured.env.CODEX_API_KEY, undefined);
  assert.equal(captured.env.OPENAI_API_KEY, undefined);
  assert.equal(captured.env.NODE_OPTIONS, `--require=${JSON.stringify(path.join(dir, 'attempt_1/synthetic-runtime/fake-codex-bootstrap.cjs'))}`);
  assert.equal(captured.env.CODEX_INTERNAL_ORIGINATOR_OVERRIDE, 'codex_sdk_ts');
  assert.equal(captured.env.HOME, captured.env.CODEX_HOME);
  const raw = events.filter(e => e.type === 'sdk_event');
  assert.deepEqual(raw.map(e => e.sequence), [1, 2, 3, 4]);
  assert.ok(raw.every(e => e.attemptId === message.attemptId && e.threadId === receipt.threadId));
  const frozen = JSON.parse(await fs.readFile(path.join(dir, 'attempt_1/receipt.json'), 'utf8'));
  assert.deepEqual(frozen.attempt, message.params.attempt);
  assert.equal(frozen.compiledPromptSha256, message.params.compiledPromptSha256);
});

test('local observe/replay uses no new SDK call, duplicate attempt rejected, next new attempt allowed after completion', async t => {
  const { bridge, dir } = await fixture(t);
  await completed(bridge);
  const before = await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8');
  for (let i = 0; i < 3; i++) assert.equal((await bridge.handle(command('observe', 'attempt_1'))).result.state, 'transport_completed');
  await assert.rejects(bridge.handle(executeCommand()), { code: 'ATTEMPT_EXISTS' });
  assert.equal(await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8'), before);
  assert.equal((await completed(bridge, executeCommand('attempt_2'))).receipt.state, 'transport_completed');
});

test('identical duplicates are durable and do not overwrite or double-complete', async t => {
  const { bridge, events } = await fixture(t, 'duplicates');
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'transport_completed');
  assert.equal(events.filter(e => e.duplicate).length, 2);
  assert.equal(events.filter(e => e.type === 'observation').length, 1);
});

test('observed pre-turn configuration notice is durable, never model text, and still requires a complete turn', async t => {
  const { bridge, dir, events } = await fixture(t, 'configuration-notice');
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'transport_completed');
  assert.equal(receipt.rawResult.rawText, '{"ok":true,"text":"合成测试"}');
  const warning = events.filter(event => event.rawEvent?.item?.type === 'error');
  assert.equal(warning.length, 1);
  assert.equal(warning[0].sequence, 2);
  assert.match(warning[0].rawEvent.item.message, /tools\.view_image/);
  const restarted = new OfficialSdkBridge({ stateDirectory: dir });
  await bridge.disconnect(); await restarted.initialize();
  try {
    assert.equal(restarted.observe('attempt_1').rawResult.rawText, receipt.rawResult.rawText);
    assert.equal(restarted.inspect().liveCallsAllowed, false);
  } finally { await restarted.disconnect(); }
});

for (const scenario of ['unstable-feature-notice', 'ordinary-error-item']) test(`${scenario} follows the official non-fatal ErrorItem contract without inspecting warning prose`, async t => {
  const { bridge, dir, events } = await fixture(t, scenario);
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'transport_completed');
  assert.equal(receipt.rawResult.rawText, '{"ok":true,"text":"合成测试"}');
  const raw = events.filter(event => event.type === 'sdk_event');
  assert.deepEqual(raw.map(event => event.rawEvent.type), ['thread.started', 'item.completed', 'turn.started', 'item.completed', 'turn.completed']);
  assert.equal(raw[1].rawEvent.item.type, 'error');
  assert.equal(raw[1].sequence, 2);
  const journal = await fs.readFile(path.join(dir, 'attempt_1/events.ndjson'), 'utf8');
  assert.deepEqual(JSON.parse(journal.split('\n')[1]).rawEvent, raw[1].rawEvent);
});

for (const [scenario, expected] of [
  ['out-of-order', 'OUT_OF_ORDER'], ['early-complete', 'OUT_OF_ORDER'], ['changed-thread', 'THREAD_IDENTITY'],
  ['policy-violation', 'POLICY_VIOLATION'], ['failed-after-text', 'SDK_TURN_FAILED'],
  ['same-item-mutated', 'ITEM_IDENTITY'], ['oversized-event', 'EVENT_LIMIT'],
  ['configuration-notice-only', 'INCOMPLETE_STREAM'], ['configuration-notice-before-thread', 'OUT_OF_ORDER'],
  ['configuration-notice-malformed', 'OUT_OF_ORDER'], ['configuration-notice-after-turn', 'POLICY_VIOLATION'],
  ['agent-before-turn', 'OUT_OF_ORDER'],
  ['notice-missing-id', 'OUT_OF_ORDER'], ['notice-too-long', 'OUT_OF_ORDER'],
  ['notice-extra-event-field', 'OUT_OF_ORDER'], ['tool-before-turn', 'OUT_OF_ORDER'],
  ['fatal-error-before-turn', 'SDK_TURN_FAILED'],
]) test(`${scenario} fails closed, persists evidence, blocks retry/fallback and reserves unknown capacity`, async t => {
  const { bridge, dir } = await fixture(t, scenario);
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'submission_unknown');
  assert.equal(receipt.error.code, expected);
  assert.equal(receipt.domainAccepted, false);
  assert.equal(receipt.processTreeStopProven, false);
  await assert.rejects(bridge.handle(executeCommand('attempt_2')), { code: 'WORKER_OCCUPIED' });
  const journal = await fs.readFile(path.join(dir, 'attempt_1/events.ndjson'), 'utf8');
  assert.ok(journal.length > 0);
});

test('malformed runtime JSON remains raw error evidence and is never repaired/resubmitted', async t => {
  const { bridge, dir } = await fixture(t, 'malformed-json');
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'submission_unknown');
  assert.equal(receipt.error.code, 'SDK_STREAM_ERROR');
  assert.match(receipt.error.message, /\{this is not runtime JSON\}/);
  assert.match(await fs.readFile(path.join(dir, 'attempt_1/receipt.json'), 'utf8'), /this is not runtime JSON/);
});

test('invalid final domain JSON is preserved verbatim with parse error; transport completion is not acceptance', async t => {
  const { bridge } = await fixture(t, 'invalid-text-json');
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'transport_completed');
  assert.equal(receipt.rawResult.rawText, '{not repaired');
  assert.equal(receipt.rawResult.parsedJson, null);
  assert.equal(typeof receipt.rawResult.parseError, 'string');
  assert.equal(receipt.domainAccepted, false);
});

test('hash mismatch, backend change, identity mismatch and reasoning fallback are rejected before SDK start', async t => {
  const { bridge, dir } = await fixture(t);
  for (const [mutate, code] of [
    [m => m.params.compiledPrompt += 'altered', 'INPUT_HASH'],
    [m => m.params.attempt.backend = 'codex_cli', 'FROZEN_ATTEMPT'],
    [m => m.params.attempt.attemptId = 'other_attempt', 'ATTEMPT_ID'],
    [m => m.params.attempt.request.reasoning = 'invented_reasoning', 'MODEL_IDENTITY'],
  ]) { const message = executeCommand(); mutate(message); await assert.rejects(bridge.handle(message), { code }); }
  assert.deepEqual(await fs.readdir(dir), ['worker.lock']);
});

test('one active attempt, bounded AbortSignal cancel, no process-tree success claim', async t => {
  const { bridge } = await fixture(t, 'hang');
  await bridge.handle(executeCommand());
  await assert.rejects(bridge.handle(executeCommand('attempt_2')), { code: 'WORKER_OCCUPIED' });
  const start = Date.now();
  const canceled = (await bridge.handle(command('cancel', 'attempt_1'))).result;
  assert.ok(Date.now() - start < 1000);
  assert.equal(canceled.state, 'submission_unknown');
  assert.equal(canceled.processTreeStopProven, false);
  await assert.rejects(bridge.handle(executeCommand('attempt_2')), { code: 'WORKER_OCCUPIED' });
});

test('restart recovers in-flight receipt as unknown; observe never creates a new turn', async t => {
  const { bridge, dir } = await fixture(t, 'hang');
  await bridge.handle(executeCommand());
  await bridge.disconnect('SYNTHETIC_DISCONNECT');
  const before = await fs.readFile(path.join(dir, 'attempt_1/receipt.json'), 'utf8');
  const restarted = new OfficialSdkBridge({ stateDirectory: dir, offlineFixture: offlineFixture('happy') });
  await restarted.initialize();
  t.after(() => restarted.disconnect());
  assert.equal(restarted.observe('attempt_1').state, 'submission_unknown');
  await assert.rejects(restarted.handle(executeCommand('attempt_2')), { code: 'WORKER_OCCUPIED' });
  assert.equal(restarted.observe('attempt_1').rawResult, null);
  assert.equal(JSON.parse(before).attempt.attemptId, 'attempt_1');
});

test('second worker cannot own same state directory', async t => {
  const { dir } = await fixture(t, null);
  const second = new OfficialSdkBridge({ stateDirectory: dir });
  await assert.rejects(second.initialize(), { code: 'WORKER_LOCKED' });
});

function childClient(directory, scenario) {
  const child = spawn(process.execPath, [main, '--state-dir', directory, ...(scenario ? ['--offline-fixture', scenario] : [])], { stdio: ['pipe', 'pipe', 'pipe'], env: { PATH: process.env.PATH ?? '' } });
  let pending = '', stderr = '';
  const messages = [], waiters = [];
  child.stderr.on('data', data => { stderr += data.toString(); });
  child.stdout.on('data', data => {
    pending += data.toString();
    while (pending.includes('\n')) {
      const i = pending.indexOf('\n'); const msg = JSON.parse(pending.slice(0, i)); pending = pending.slice(i + 1); messages.push(msg);
      for (const waiter of [...waiters]) if (waiter.predicate(msg)) { waiters.splice(waiters.indexOf(waiter), 1); clearTimeout(waiter.timer); waiter.resolve(msg); }
    }
  });
  const wait = predicate => {
    const found = messages.find(predicate); if (found) return Promise.resolve(found);
    return new Promise((resolve, reject) => { const waiter = { predicate, resolve, timer: setTimeout(() => reject(new Error(`Timed out; stderr=${stderr}; messages=${JSON.stringify(messages)}`)), 5000) }; waiters.push(waiter); });
  };
  const exited = new Promise(resolve => child.on('exit', (code, signal) => resolve({ code, signal })));
  return { child, messages, wait, exited, request(message) { child.stdin.write(JSON.stringify(message) + '\n'); return wait(m => m.type === 'response' && m.requestId === message.requestId); } };
}

test('stdio fragmented UTF-8, malformed JSON, request version and duplicate IDs are bounded and deterministic', async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-stdio-'));
  const client = childClient(dir);
  t.after(async () => { client.child.kill(); await client.exited; await fs.rm(dir, { recursive: true, force: true }); });
  client.child.stdin.write('{malformed}\n');
  const bad = await client.wait(m => m.ok === false);
  assert.equal(bad.error.code, 'INVALID_REQUEST');
  const hello = command('hello'); hello.extra = '分片UTF-8';
  const bytes = Buffer.from(JSON.stringify(hello) + '\n');
  for (let i = 0; i < bytes.length; i++) client.child.stdin.write(bytes.subarray(i, i + 1));
  assert.equal((await client.wait(m => m.requestId === hello.requestId)).result.liveCallsAllowed, false);
  client.child.stdin.write(JSON.stringify(hello) + '\n');
  assert.equal((await client.wait(m => m.error?.code === 'DUPLICATE_REQUEST_ID')).ok, false);
  const version = command('hello'); version.protocolVersion = 'future/v999';
  assert.equal((await client.request(version)).error.code, 'PROTOCOL_VERSION');
  assert.equal((await client.request(command('shutdown'))).result.shutdown, true);
  assert.equal((await client.exited).code, 0);
});

test('stdio fixture uses actual official SDK and cleanly returns a transport observation', async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-stdio-'));
  const client = childClient(dir, 'happy');
  t.after(async () => { client.child.kill(); await client.exited; await fs.rm(dir, { recursive: true, force: true }); });
  assert.equal((await client.request(executeCommand())).ok, true);
  const result = await client.wait(m => m.type === 'observation');
  assert.equal(result.state, 'transport_completed');
  assert.equal(result.rawResult.parsedJson.ok, true);
  await client.request(command('shutdown')); await client.exited;
});

test('cancel after runtime starts is bounded even when synthetic child ignores SIGTERM', async t => {
  const { bridge, events } = await fixture(t, 'ignore-abort', { limits: { cancelMs: 100 } });
  await bridge.handle(executeCommand());
  const deadline = Date.now() + 3000;
  while (!events.some(e => e.rawEvent?.type === 'turn.started')) {
    assert.ok(Date.now() < deadline, 'synthetic runtime should start');
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  const start = Date.now();
  const result = await bridge.handle(command('cancel', 'attempt_1'));
  assert.ok(Date.now() - start < 600);
  assert.equal(result.result.state, 'submission_unknown');
  assert.equal(result.result.processTreeStopProven, false);
  await assert.rejects(bridge.handle(executeCommand('attempt_2')), { code: 'WORKER_OCCUPIED' });
  await bridge.active?.task; // Fixture self-exits after 1.6s; do not leave a synthetic orphan.
  assert.equal(bridge.observe('attempt_1').state, 'submission_unknown');
});

test('hard worker crash preserves unknown attempt and passive restart starts zero new runtimes', async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director SDK 合成 '));
  const first = childClient(dir, 'hang');
  let second, fakePid;
  t.after(async () => {
    first.child.kill('SIGKILL'); if (second) { second.child.kill(); await second.exited; }
    if (fakePid) try { process.kill(fakePid, 'SIGTERM'); } catch { /* synthetic process already stopped */ }
    await fs.rm(dir, { recursive: true, force: true });
  });
  assert.equal((await first.request(executeCommand())).ok, true);
  await first.wait(m => m.rawEvent?.type === 'turn.started');
  const capturedPath = path.join(dir, 'attempt_1/fixture-invocation.json');
  const before = await fs.readFile(capturedPath, 'utf8');
  fakePid = JSON.parse(before).pid;
  first.child.kill('SIGKILL'); await first.exited;
  // This is test cleanup of the captured synthetic child, never claimed as bridge tree-stop support.
  try { process.kill(fakePid, 'SIGTERM'); } catch { /* already exited */ }
  second = childClient(dir, 'happy');
  const observed = await second.request(command('observe', 'attempt_1'));
  assert.equal(observed.result.state, 'submission_unknown');
  assert.equal(observed.result.threadId, 'synthetic-thread-1');
  assert.equal((await second.request(executeCommand('attempt_2'))).error.code, 'WORKER_OCCUPIED');
  assert.equal(await fs.readFile(capturedPath, 'utf8'), before);
  assert.equal((await fs.readdir(dir)).includes('attempt_2'), false);
  await second.request(command('shutdown')); await second.exited;
});

test('input frame bound closes stdio without creating an attempt', async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-bounds-'));
  const client = childClient(dir);
  // Broken pipe is expected once the bounded parser closes input.
  client.child.stdin.on('error', () => {});
  t.after(async () => { client.child.kill(); await client.exited; await fs.rm(dir, { recursive: true, force: true }); });
  client.child.stdin.write('x'.repeat(1_048_577));
  const error = await client.wait(m => m.error?.code === 'FRAME_LIMIT');
  assert.equal(error.ok, false);
  await client.exited;
  assert.deepEqual(await fs.readdir(dir), []);
});

test('partial durable attempt directory blocks initialization rather than silently resetting intent', async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-partial-'));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  await fs.mkdir(path.join(dir, 'attempt_partial'));
  const bridge = new OfficialSdkBridge({ stateDirectory: dir });
  await assert.rejects(bridge.initialize(), { code: 'INCOMPLETE_RECEIPT' });
});

test('torn event journal cannot retain a trusted completed state after restart', async t => {
  const { dir, bridge } = await fixture(t);
  await completed(bridge); await bridge.disconnect();
  await fs.appendFile(path.join(dir, 'attempt_1/events.ndjson'), '{torn');
  const restarted = new OfficialSdkBridge({ stateDirectory: dir });
  await restarted.initialize();
  t.after(() => restarted.disconnect());
  assert.equal(restarted.observe('attempt_1').state, 'submission_unknown');
  assert.equal(restarted.observe('attempt_1').error.code, 'TORN_JOURNAL');
});

test('installed official SDK version and executable implementation match pinned provenance', async () => {
  const pkg = JSON.parse(await fs.readFile(new URL('../node_modules/@openai/codex-sdk/package.json', import.meta.url), 'utf8'));
  const body = await fs.readFile(new URL('../node_modules/@openai/codex-sdk/dist/index.js', import.meta.url));
  assert.equal(pkg.version, '0.160.1');
  assert.equal(pkg.dependencies['@openai/codex'], '0.160.1');
  assert.equal(sha256(body), 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f');
});

test('bounded disconnect retains directory ownership until the synthetic task actually settles', async t => {
  const { bridge, dir, events } = await fixture(t, 'ignore-abort', { limits: { cancelMs: 30 } });
  await bridge.handle(executeCommand());
  const deadline = Date.now() + 3000;
  while (!events.some(e => e.rawEvent?.type === 'turn.started')) {
    assert.ok(Date.now() < deadline); await new Promise(resolve => setTimeout(resolve, 10));
  }
  await bridge.disconnect();
  const second = new OfficialSdkBridge({ stateDirectory: dir });
  if (bridge.active && !bridge.active.finished) {
    await assert.rejects(second.initialize(), { code: 'WORKER_LOCKED' });
    await bridge.active.task;
  }
  await second.initialize();
  assert.equal(second.observe('attempt_1').state, 'submission_unknown');
  await second.disconnect();
});

for (const field of ['model', 'compiledPrompt', 'compiledSchema', 'semanticRequestHash', 'executionFingerprint', 'rawResult']) {
  test(`persisted ${field} tampering fails closed before local replay`, async t => {
    const { dir, bridge } = await fixture(t);
    await completed(bridge); await bridge.disconnect();
    const file = path.join(dir, 'attempt_1/receipt.json');
    const record = JSON.parse(await fs.readFile(file, 'utf8'));
    if (field === 'model') record.attempt.request.model = 'changed-model';
    else if (field === 'semanticRequestHash' || field === 'executionFingerprint') record.attempt[field] += '-changed';
    else if (field === 'rawResult') record.rawResult.rawText = '{"silently_repaired":true}';
    else record[field] += 'changed';
    await fs.writeFile(file, JSON.stringify(record));
    const restarted = new OfficialSdkBridge({ stateDirectory: dir });
    await assert.rejects(restarted.initialize(), { code: 'CORRUPT_RECEIPT' });
  });
}

test('parallel stale-lock contenders cannot both own the worker', async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-lock-race-'));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  // A known dead process identity for this controlled regression; native CI is still required.
  const dead = spawn(process.execPath, ['-e', 'process.exit(0)']);
  await new Promise(resolve => dead.on('exit', resolve));
  await fs.writeFile(path.join(dir, 'worker.lock'), JSON.stringify({ pid: dead.pid }));
  const first = new OfficialSdkBridge({ stateDirectory: dir });
  const second = new OfficialSdkBridge({ stateDirectory: dir });
  const results = await Promise.allSettled([first.initialize(), second.initialize()]);
  assert.equal(results.filter(result => result.status === 'fulfilled').length, 1);
  assert.equal([first, second].filter(bridge => bridge.lock !== null).length, 1);
  await first.disconnect(); await second.disconnect();
});

test('stranded recovery guard fails closed and is never auto-reclaimed', async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-lock-guard-'));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  const dead = spawn(process.execPath, ['-e', 'process.exit(0)']);
  await new Promise(resolve => dead.on('exit', resolve));
  await fs.writeFile(path.join(dir, 'worker.lock'), JSON.stringify({ pid: dead.pid }));
  await fs.writeFile(path.join(dir, 'worker.lock.recovery'), 'stranded recovery guard');
  const bridge = new OfficialSdkBridge({ stateDirectory: dir });
  await assert.rejects(bridge.initialize(), { code: 'WORKER_LOCKED' });
  assert.equal(await fs.readFile(path.join(dir, 'worker.lock.recovery'), 'utf8'), 'stranded recovery guard');
});

test('unknown persisted state cannot release a worker budget', async t => {
  const { dir, bridge } = await fixture(t);
  await completed(bridge); await bridge.disconnect();
  const file = path.join(dir, 'attempt_1/receipt.json');
  const record = JSON.parse(await fs.readFile(file, 'utf8'));
  record.state = 'future_state_not_understood';
  await fs.writeFile(file, JSON.stringify(record));
  const restarted = new OfficialSdkBridge({ stateDirectory: dir });
  await assert.rejects(restarted.initialize(), { code: 'CORRUPT_RECEIPT' });
});

for (const field of ['threadId', 'state', 'usage', 'error', 'lastSequence', 'domainAccepted']) {
  test(`persisted ${field} metadata is bound by the full receipt envelope checksum`, async t => {
    const { dir, bridge } = await fixture(t);
    await completed(bridge); await bridge.disconnect();
    const file = path.join(dir, 'attempt_1/receipt.json');
    const record = JSON.parse(await fs.readFile(file, 'utf8'));
    const changes = { threadId: 'other-thread', state: 'running', usage: { input_tokens: 0 }, error: { code: 'altered' }, lastSequence: 0, domainAccepted: true };
    record[field] = changes[field];
    await fs.writeFile(file, JSON.stringify(record));
    const restarted = new OfficialSdkBridge({ stateDirectory: dir });
    await assert.rejects(restarted.initialize(), { code: 'CORRUPT_RECEIPT' });
  });
}

test('official reconnect diagnostic stays on one original SDK stream until completion and survives local replay', async t => {
  const { bridge, dir, events } = await fixture(t, 'reconnect-recovered');
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'transport_completed');
  assert.equal(receipt.rawResult.rawText, '{"ok":true,"text":"合成测试"}');
  assert.equal(receipt.threadId, 'synthetic-thread-1');
  assert.equal(receipt.domainAccepted, false);
  const raw = events.filter(event => event.type === 'sdk_event');
  assert.deepEqual(raw.map(event => event.rawEvent.type), ['thread.started', 'turn.started', 'error', 'item.completed', 'turn.completed']);
  assert.deepEqual(raw.map(event => event.sequence), [1, 2, 3, 4, 5]);
  assert.match(raw[2].rawEvent.message, /Reconnecting\.\.\. 2\/5/);
  const invocation = await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8');
  assert.equal(JSON.parse(invocation).args.includes('resume'), false);
  await assert.rejects(bridge.handle(executeCommand()), { code: 'ATTEMPT_EXISTS' });
  await bridge.disconnect();
  const restarted = new OfficialSdkBridge({ stateDirectory: dir }); await restarted.initialize();
  try {
    assert.equal(restarted.observe('attempt_1').state, 'transport_completed');
    assert.equal(await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8'), invocation);
    assert.equal(JSON.parse((await fs.readFile(path.join(dir, 'attempt_1/events.ndjson'), 'utf8')).split('\n')[2]).rawEvent.type, 'error');
  } finally { await restarted.disconnect(); }
});

for (const [scenario, expected] of [
  ['reconnect-only', 'INCOMPLETE_STREAM'], ['reconnect-failed', 'SDK_TURN_FAILED'],
  ['fatal-error-during-turn', 'SDK_TURN_FAILED'], ['reconnect-before-turn', 'SDK_TURN_FAILED'],
  ['reconnect-invalid-counter', 'SDK_TURN_FAILED'], ['reconnect-extra-field', 'SDK_TURN_FAILED'],
  ['reconnect-too-long', 'SDK_TURN_FAILED'],
]) test(`${scenario} never fabricates completion or authorizes another dispatch`, async t => {
  const { bridge, dir } = await fixture(t, scenario);
  const { receipt } = await completed(bridge);
  assert.equal(receipt.state, 'submission_unknown'); assert.equal(receipt.error.code, expected);
  assert.equal(receipt.rawResult, null); assert.equal(receipt.usage, null); assert.equal(receipt.domainAccepted, false);
  const original = await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8');
  await assert.rejects(bridge.handle(executeCommand('attempt_2')), { code: 'WORKER_OCCUPIED' });
  assert.equal(await fs.readFile(path.join(dir, 'attempt_1/fixture-invocation.json'), 'utf8'), original);
});

test('reconnect diagnostics do not extend the existing execution deadline', async t => {
  const { bridge, events } = await fixture(t, 'reconnect-hang', { limits: { executionMs: 1000 } });
  const start = Date.now(); const { receipt } = await completed(bridge);
  assert.ok(Date.now() - start < 5000);
  assert.ok(events.some(event => event.rawEvent?.type === 'error' && event.rawEvent.message.startsWith('Reconnecting...')));
  assert.equal(receipt.state, 'submission_unknown'); assert.equal(receipt.error.code, 'CANCEL_UNCONFIRMED');
  assert.equal(receipt.rawResult, null); assert.equal(receipt.processTreeStopProven, false);
  await assert.rejects(bridge.handle(executeCommand('attempt_2')), { code: 'WORKER_OCCUPIED' });
});
