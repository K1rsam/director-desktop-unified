import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { OfficialSdkBridge, PROTOCOL_VERSION, sha256, schemaRejection, schemaRejectionJournal } from '../src/bridge.mjs';
import { offlineFixture } from './fixture-runtime.mjs';

const refusal = () => ({ type: 'error', message: JSON.stringify({ type: 'error', status: 400, error: { type: 'invalid_request_error', code: 'invalid_json_schema', param: 'text.format.schema', message: "Invalid schema for response_format 'codex_output_schema': 'schema.properties.assets.uniqueItems' is not permitted when strict is true." } }) });
const thread = { type: 'thread.started', thread_id: 'synthetic-thread-1' };
const turn = { type: 'turn.started' };
test('strict refusal requires typed HTTP 400 JSON and no output or uncertain predecessor', () => {
  assert.ok(schemaRejection(refusal()));
  assert.equal(schemaRejectionJournal([thread, turn, refusal()]), true);
  for (const key of ['status', 'type', 'param', 'code', 'message']) {
    const event = refusal(), value = JSON.parse(event.message);
    if (key === 'status') value.status = 500; else value.error[key] = 'wrong';
    event.message = JSON.stringify(value); assert.equal(schemaRejection(event), null, key);
  }
  for (const event of [
    { type: 'error', message: 'tls handshake eof' },
    { type: 'error', message: 'Reconnecting... 2/5 (stream disconnected before completion: tls handshake eof)' },
    { type: 'item.started', item: { id: 'r', type: 'reasoning', text: 'partial' } },
    { type: 'item.completed', item: { id: 'a', type: 'agent_message', text: 'partial' } },
    { type: 'turn.completed', usage: {} },
  ]) assert.equal(schemaRejectionJournal([thread, turn, event, refusal()]), false);
  assert.equal(schemaRejectionJournal([thread, turn, refusal(), turn]), false);
});

function command(id = 'original', v4 = false) {
  const compiledPrompt = JSON.stringify({ rules: {}, input: { synthetic: true } });
  const compiledSchema = JSON.stringify({ type: 'object' });
  return { protocolVersion: PROTOCOL_VERSION, requestId: `request_${id}`, attemptId: id, method: 'execute', params: {
    attempt: { version: v4 ? 'text-attempt/v4' : 'text-attempt/v3', compiledInputFormat: v4 ? 'rules-first-json-strict/v2' : 'rules-first-json/v1',
      attemptId: id, workflowId: 'workflow', logicalNodeId: 'node', backend: 'codex_sdk', request: { stage: 'fixture', skill: 'fixture', model: 'gpt-6.1-sol', reasoning: 'medium', input: {}, schema: {}, ruleSnapshot: {} },
      semanticRequestHash: 'semantic', executionFingerprint: 'execution', attemptEnvelopeHash: 'envelope', state: 'prepared', threadId: null, turnId: null, lastSequence: 0, rawResult: null },
    compiledPrompt, compiledSchema, compiledPromptSha256: sha256(compiledPrompt), compiledSchemaSha256: sha256(compiledSchema),
  } };
}
for (const scenario of ['schema-refusal', 'schema-refusal-after-text', 'schema-refusal-after-reasoning', 'schema-refusal-after-reconnect']) {
  test(`${scenario}: terminal classification, old evidence unchanged and no implicit retry`, async t => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-rejection-'));
    const bridge = new OfficialSdkBridge({ stateDirectory: root, offlineFixture: offlineFixture(scenario) });
    t.after(async () => { await bridge.disconnect(); assert.equal(path.dirname(root), path.resolve(os.tmpdir())); assert.match(path.basename(root), /^director-sdk-rejection-/); await fs.rm(root, { recursive: true, force: true }); });
    await bridge.initialize(); await bridge.handle(command()); await bridge.active?.task;
    const expected = scenario === 'schema-refusal' ? 'transport_rejected' : 'submission_unknown';
    assert.equal(bridge.observe('original').state, expected);
    await bridge.disconnect();
    const file = path.join(root, 'original', 'receipt.json');
    const record = JSON.parse(await fs.readFile(file, 'utf8'));
    if (scenario === 'schema-refusal') {
      // Simulate the exact previous build's error classification, retaining seal.
      record.state = 'submission_unknown'; record.error.code = 'SDK_TURN_FAILED';
      delete record.receiptEnvelopeSha256;
      record.receiptEnvelopeSha256 = sha256(JSON.stringify(record));
      await fs.writeFile(file, JSON.stringify(record));
    }
    const receipt = await fs.readFile(file), journal = await fs.readFile(path.join(root, 'original', 'events.ndjson'));
    const resumed = new OfficialSdkBridge({ stateDirectory: root, offlineFixture: offlineFixture('happy') });
    await resumed.initialize();
    try {
      assert.equal(resumed.observe('original').state, expected);
      await assert.rejects(resumed.handle(command()), { code: expected === 'transport_rejected' ? 'ATTEMPT_EXISTS' : 'WORKER_OCCUPIED' });
      if (expected === 'transport_rejected') {
        await resumed.handle(command('corrected', true)); await resumed.active?.task;
        assert.equal(resumed.observe('corrected').state, 'transport_completed');
        assert.deepEqual(await fs.readFile(file), receipt);
      } else await assert.rejects(resumed.handle(command('corrected', true)), { code: 'WORKER_OCCUPIED' });
      assert.deepEqual(await fs.readFile(path.join(root, 'original', 'events.ndjson')), journal);
    } finally { await resumed.disconnect(); }
  });
}
