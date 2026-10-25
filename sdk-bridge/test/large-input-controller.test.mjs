// Pinned official SDK with a synthetic child only. No account or network calls.
import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { createLargeInputBridge, INPUT_FRAME_BYTES, OUTPUT_FRAME_BYTES } from '../src/large-input-controller.mjs';
import { offlineFixture } from './fixture-runtime.mjs';

const hash = body => createHash('sha256').update(body).digest('hex');
const resources = fileURLToPath(new URL('../', import.meta.url));
const controller = fileURLToPath(new URL('../src/large-input-controller.mjs', import.meta.url));
let serial = 0;
const command = (method, params = {}, attemptId = null) => ({ protocolVersion: 'director-sdk-bridge/v1',
  requestId: `large_${++serial}`, attemptId, method, params });
async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-large-input-'));
  t.after(async () => {
    assert.equal(path.dirname(path.resolve(root)), path.resolve(os.tmpdir()));
    assert.match(path.basename(root), /^director-sdk-large-input-/);
    await fs.rm(root, { recursive: true, force: true });
  });
  const identity = { version: 'official-sdk-identity/v2', provider: 'codex_sdk', permissionProfile: 'sdk-text-only-native/v1',
    resourceDirectory: resources, sdkImplementationSha256: 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f',
    bridgeSha256: hash(Buffer.concat(await Promise.all(['main.mjs', 'bridge.mjs', 'native-runtime.mjs'].map(n => fs.readFile(path.join(resources, 'src', n)))))),
    runtimeExecutable: process.execPath, runtimeSha256: 'e'.repeat(64), runtimePackageVersion: 'synthetic-do-not-execute',
    supervisorExecutable: process.execPath, supervisorSha256: 'e'.repeat(64),
    credentialHome: path.join(root, 'never-read-home'), codexHome: path.join(root, 'never-read-codex') };
  const nativeConfig = { version: 'sdk-native-config/v1', identity };
  const schema = { type: 'object', properties: { ok: { type: 'boolean' }, text: { type: 'string' } }, required: ['ok', 'text'], additionalProperties: false };
  const compiledPrompt = '完整原文与规则保持不变。'.repeat(24_000);
  const compiledSchema = JSON.stringify(schema);
  const request = { stage: 'fixture', skill: 'fixture', model: 'gpt-6.1-sol', reasoning: 'medium',
    input: { full: compiledPrompt }, schema, ruleSnapshot: { fixture: true } };
  const message = command('execute', { attempt: { version: 'text-attempt/v2', attemptId: 'large_attempt', workflowId: 'large_workflow',
    logicalNodeId: 'large_node', backend: 'codex_sdk', request, semanticRequestHash: hash(JSON.stringify(request)),
    executionFingerprint: 'f'.repeat(64), runtimeIdentity: identity, state: 'prepared', threadId: null, turnId: null, lastSequence: 0, rawResult: null },
    compiledPrompt, compiledSchema, compiledPromptSha256: hash(compiledPrompt), compiledSchemaSha256: hash(compiledSchema) }, 'large_attempt');
  return { root, nativeConfig, message };
}

test('large frozen request executes once with original bytes and bounded outputs, then reopens without replay', async t => {
  const f = await fixture(t), events = [];
  const inputSize = Buffer.byteLength(JSON.stringify(f.message));
  assert(inputSize > OUTPUT_FRAME_BYTES && inputSize < INPUT_FRAME_BYTES);
  let invocations = 0;
  const create = () => createLargeInputBridge({ stateDirectory: f.root, nativeConfig: f.nativeConfig, emit: e => events.push(e),
    runtimeFactory: async dir => {
      invocations++;
      const runtime = await offlineFixture('happy').create(dir);
      const temporaryDirectory = path.join(dir, 'schema-temp'); await fs.mkdir(temporaryDirectory);
      return { ...runtime, temporaryDirectory };
    } });
  const bridge = await create();
  try {
    const inspect = (await bridge.handle(command('inspect'))).result;
    assert.equal(inspect.limits.frameBytes, INPUT_FRAME_BYTES);
    assert.equal(inspect.limits.eventBytes, OUTPUT_FRAME_BYTES);
    assert.equal(inspect.limits.outputQueueBytes, 2_097_152);
    assert.equal(inspect.inputEnvelope.controllerSha256, hash(await fs.readFile(controller)));
    const ack = await bridge.handle(f.message);
    assert.deepEqual(ack.result.inputEnvelope, inspect.inputEnvelope);
    await bridge.active?.task;
    assert.equal(bridge.observe('large_attempt').state, 'transport_completed', JSON.stringify(bridge.observe('large_attempt').error));
    assert.equal(invocations, 1);
    assert(events.every(e => Buffer.byteLength(JSON.stringify(e)) <= OUTPUT_FRAME_BYTES));
  } finally { await bridge.disconnect(); }
  const receiptFile = path.join(f.root, 'large_attempt/receipt.json');
  const before = await fs.readFile(receiptFile), receipt = JSON.parse(before);
  assert.deepEqual(receipt.attempt, f.message.params.attempt);
  assert.equal(receipt.compiledPrompt, f.message.params.compiledPrompt);
  assert.equal(receipt.compiledSchema, f.message.params.compiledSchema);
  const reopened = await create();
  try {
    assert.equal(reopened.observe('large_attempt').state, 'transport_completed');
    await assert.rejects(reopened.handle(f.message), { code: 'ATTEMPT_EXISTS' });
    assert.equal(invocations, 1);
    assert.deepEqual(await fs.readFile(receiptFile), before);
  } finally { await reopened.disconnect(); }
});

test('oversized request fails before attempt directory or runtime; source drift is never imported', async t => {
  const f = await fixture(t); let invocations = 0;
  const bridge = await createLargeInputBridge({ stateDirectory: f.root, nativeConfig: f.nativeConfig,
    runtimeFactory: async () => { invocations++; throw Error('must not run'); } });
  try {
    f.message.params.padding = 'x'.repeat(INPUT_FRAME_BYTES);
    await assert.rejects(bridge.handle(f.message), { code: 'FRAME_LIMIT' });
    assert.equal(invocations, 0);
    await assert.rejects(fs.stat(path.join(f.root, 'large_attempt')), { code: 'ENOENT' });
  } finally { await bridge.disconnect(); }
  f.nativeConfig.identity.bridgeSha256 = '0'.repeat(64);
  await assert.rejects(createLargeInputBridge({ stateDirectory: f.root, nativeConfig: f.nativeConfig }), { code: 'SDK_LARGE_INPUT_IDENTITY' });
});

test('standalone input reader accepts split UTF8 over 1 MiB, enforces 4 MiB and cannot start an invalid task', async t => {
  const f = await fixture(t), config = path.join(f.root, 'native-config.json');
  await fs.writeFile(config, JSON.stringify(f.nativeConfig));
  const state = path.join(f.root, 'worker'); await fs.mkdir(state);
  const child = spawn(process.execPath, [controller, '--state-dir', state, '--native-config', config],
    { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const messages = []; let pending = '', stderr = '';
  child.stdout.on('data', bytes => {
    pending += bytes.toString('utf8'); let at;
    while ((at = pending.indexOf('\n')) >= 0) { messages.push(JSON.parse(pending.slice(0, at))); pending = pending.slice(at + 1); }
  });
  child.stdin.on('error', () => {});
  child.stderr.on('data', bytes => { stderr += bytes.toString('utf8'); });
  const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({ code, signal })));
  t.after(async () => { if (child.exitCode === null) child.kill(); await exited; });
  async function waitFor(predicate) {
    const until = Date.now() + 10_000;
    while (!messages.some(predicate)) { assert(Date.now() < until, `controller timeout: ${stderr}`); await new Promise(r => setTimeout(r, 20)); }
    return messages.find(predicate);
  }
  const inspect = command('inspect'); child.stdin.write(JSON.stringify(inspect) + '\n');
  assert.equal((await waitFor(m => m.requestId === inspect.requestId)).result.inputEnvelope.outputFrameBytes, OUTPUT_FRAME_BYTES);
  const unknown = command('unknown', { fixture: '测'.repeat(360_000) });
  const line = Buffer.from(JSON.stringify(unknown) + '\n'), mark = line.indexOf(Buffer.from('测'));
  child.stdin.write(line.subarray(0, mark + 1)); child.stdin.write(line.subarray(mark + 1));
  assert.equal((await waitFor(m => m.requestId === unknown.requestId)).error.code, 'UNKNOWN_METHOD');
  const invalid = command('execute', {}, 'invalid_attempt'); child.stdin.write(JSON.stringify(invalid) + '\n');
  assert.equal((await waitFor(m => m.requestId === invalid.requestId)).error.code, 'FROZEN_ATTEMPT');
  child.stdin.write(Buffer.alloc(INPUT_FRAME_BYTES + 1, 120));
  assert.equal((await waitFor(m => m.error?.code === 'FRAME_LIMIT')).requestId, null);
  assert.equal((await exited).code, 0);
  assert.deepEqual(await fs.readdir(state), []);
});
