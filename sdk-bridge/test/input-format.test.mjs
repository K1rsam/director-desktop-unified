import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { OfficialSdkBridge, PROTOCOL_VERSION, sha256 } from '../src/bridge.mjs';
import { offlineFixture } from './fixture-runtime.mjs';

function command(format) {
  const compiledPrompt = JSON.stringify({ rules: { text: '完整创作规则' }, instruction: 'Synthetic offline only', stage: 'fixture', skill: 'fixture', input: { text: '原文不变\n😀' } });
  const compiledSchema = JSON.stringify({ type: 'object' });
  return { protocolVersion: PROTOCOL_VERSION, requestId: 'request_format', method: 'execute', attemptId: 'attempt_format', params: {
    attempt: { version: format ? 'text-attempt/v3' : 'text-attempt/v2', ...(format ? { compiledInputFormat: 'rules-first-json/v1' } : {}),
      attemptId: 'attempt_format', workflowId: 'workflow_fixture', logicalNodeId: 'node_fixture', backend: 'codex_sdk',
      request: { stage: 'fixture', skill: 'fixture', model: 'gpt-6.1-sol', reasoning: 'medium', input: {}, schema: {}, ruleSnapshot: {} },
      semanticRequestHash: 'synthetic-semantic', executionFingerprint: 'synthetic-execution', attemptEnvelopeHash: 'synthetic-envelope',
      state: 'prepared', threadId: null, turnId: null, lastSequence: 0, rawResult: null },
    compiledPrompt, compiledSchema, compiledPromptSha256: sha256(compiledPrompt), compiledSchemaSha256: sha256(compiledSchema),
  } };
}
async function fixture(t, scenario = 'happy') {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-format-'));
  const bridge = new OfficialSdkBridge({ stateDirectory: directory, offlineFixture: offlineFixture(scenario) });
  await bridge.initialize();
  t.after(async () => {
    await bridge.disconnect();
    assert.equal(path.dirname(path.resolve(directory)), path.resolve(os.tmpdir()));
    assert.match(path.basename(directory), /^director-sdk-format-/);
    await fs.rm(directory, { recursive: true, force: true });
  });
  return { directory, bridge };
}

for (const format of [false, true]) for (const scenario of ['happy', 'configuration-notice-only']) {
  test(`${format ? 'v3 rules-first' : 'v2 legacy'} ${scenario} retains frozen input and reopens with zero dispatch`, async t => {
    const { directory, bridge } = await fixture(t, scenario);
    assert.deepEqual(bridge.inspect().capabilities.compiledInputFormats, ['legacy-json/v1', 'rules-first-json/v1', 'rules-first-json-strict/v2']);
    const message = command(format);
    await bridge.handle(message); await bridge.active?.task;
    const original = bridge.observe(message.attemptId);
    const expected = scenario === 'happy' ? 'transport_completed' : 'submission_unknown';
    assert.equal(original.state, expected);
    const attemptDirectory = path.join(directory, message.attemptId);
    const invocation = await fs.readFile(path.join(attemptDirectory, 'fixture-invocation.json'));
    assert.equal(JSON.parse(invocation).prompt, message.params.compiledPrompt);
    const events = await fs.readFile(path.join(attemptDirectory, 'events.ndjson'));
    const record = JSON.parse(await fs.readFile(path.join(attemptDirectory, 'receipt.json'), 'utf8'));
    assert.deepEqual(record.attempt, message.params.attempt);
    await bridge.disconnect();
    const resumed = new OfficialSdkBridge({ stateDirectory: directory }); await resumed.initialize();
    try {
      assert.equal(resumed.inspect().liveCallsAllowed, false);
      assert.equal(resumed.observe(message.attemptId).state, expected);
      assert.deepEqual(resumed.observe(message.attemptId).rawResult, original.rawResult);
      assert.deepEqual(await fs.readFile(path.join(attemptDirectory, 'fixture-invocation.json')), invocation);
      assert.deepEqual(await fs.readFile(path.join(attemptDirectory, 'events.ndjson')), events);
      assert.deepEqual((await fs.readdir(directory)).filter(name => name !== 'worker.lock'), [message.attemptId]);
    } finally { await resumed.disconnect(); }
  });
}

for (const [name, change] of [
  ['legacy format injection', attempt => { attempt.compiledInputFormat = 'rules-first-json/v1'; }],
  ['unversioned null', attempt => { attempt.compiledInputFormat = null; }],
  ['v3 missing format', attempt => { attempt.version = 'text-attempt/v3'; }],
  ['unknown format', attempt => { attempt.version = 'text-attempt/v3'; attempt.compiledInputFormat = 'rules-first-json/v2'; }],
]) test(`${name} rejects before creating a worker attempt or runtime`, async t => {
  const { directory, bridge } = await fixture(t); const message = command(false); change(message.params.attempt);
  await assert.rejects(bridge.handle(message), { code: 'FROZEN_ATTEMPT' });
  assert.deepEqual(await fs.readdir(directory), ['worker.lock']);
});
