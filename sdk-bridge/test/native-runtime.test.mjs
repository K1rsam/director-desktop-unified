import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { nativeRuntime } from '../src/native-runtime.mjs';
import { OfficialSdkBridge } from '../src/bridge.mjs';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-native-contract-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const sha = hash(await fs.readFile(process.execPath));
  const identity = { version: 'official-sdk-identity/v2', provider: 'codex_sdk', permissionProfile: 'sdk-text-only-native/v1',
    runtimeExecutable: process.execPath, runtimeSha256: sha, runtimePackageVersion: 'synthetic-do-not-execute',
    supervisorExecutable: process.execPath, supervisorSha256: sha,
    credentialHome: path.join(root, 'never-read-home'), codexHome: path.join(root, 'never-read-codex') };
  const config = { version: 'sdk-native-config/v1', identity };
  return { root, identity, config };
}
test('native SDK config cannot be mixed with fixture or carry credential/environment overrides', async t => {
  const { root, config } = await fixture(t);
  for (const extra of [{ apiKey: 'synthetic-forbidden' }, { env: { NODE_OPTIONS: 'synthetic-forbidden' } }, { executable: process.execPath }]) {
    assert.throws(() => nativeRuntime({ ...config, ...extra }, root), /SDK_NATIVE_IDENTITY/);
  }
  assert.throws(() => nativeRuntime(config, 'relative'), /SDK_NATIVE_IDENTITY/);
  assert.throws(() => new OfficialSdkBridge({ stateDirectory: root, offlineFixture: {}, nativeRuntime: {} }), /Live execution cannot use an offline fixture/);
  assert.deepEqual(await fs.readdir(root), []);
});
test('native runtime freezes prompt/schema/identity and supplies account locations with no credential copy or host env', async t => {
  const { root, identity, config } = await fixture(t);
  const directory = path.join(root, 'attempt-native-contract'); await fs.mkdir(directory);
  const runtime = nativeRuntime(config, root);
  const schema = '{ "type": "object" }';
  const record = { attempt: { attemptId: 'attempt-native-contract', runtimeIdentity: identity,
    request: { model: 'gpt-6.1-sol', reasoning: 'medium' }, semanticRequestHash: 'a'.repeat(64), executionFingerprint: 'b'.repeat(64) },
    compiledPromptSha256: hash('synthetic only'), compiledSchema: schema, compiledSchemaSha256: hash(schema) };
  const result = await runtime.create(directory, record);
  assert.equal(result.executable, process.execPath);
  const bytes = await fs.readFile(result.env.DIRECTOR_SDK_RUNTIME_PLAN);
  assert.equal(hash(bytes), result.env.DIRECTOR_SDK_RUNTIME_PLAN_SHA256);
  const plan = JSON.parse(bytes);
  assert.equal(plan.compiledPromptSha256, record.compiledPromptSha256);
  assert.equal(plan.compiledSchemaSha256, record.compiledSchemaSha256);
  assert.equal(plan.runtimeSchemaSha256, hash(JSON.stringify(JSON.parse(schema))));
  assert.deepEqual(plan.identity, identity);
  assert.equal(result.env.HOME, identity.credentialHome);
  for (const key of Object.keys(result.env)) assert(['DIRECTOR_SDK_RUNTIME_PLAN', 'DIRECTOR_SDK_RUNTIME_PLAN_SHA256', 'USERPROFILE', 'HOME', 'TEMP', 'TMP', 'TMPDIR', 'SystemRoot', 'WINDIR'].includes(key));
  await assert.rejects(fs.stat(identity.credentialHome), { code: 'ENOENT' });
  await assert.rejects(fs.stat(identity.codexHome), { code: 'ENOENT' });
  await assert.rejects(runtime.create(directory, record), { code: 'EEXIST' });
  assert.deepEqual(await fs.readFile(result.env.DIRECTOR_SDK_RUNTIME_PLAN), bytes);
});
test('native runtime rejects changed frozen runtime before dispatch and preserves prior attempt', async t => {
  const { root, identity, config } = await fixture(t); const directory = path.join(root, 'attempt-contract'); await fs.mkdir(directory);
  const runtime = nativeRuntime(config, root);
  await assert.rejects(runtime.create(directory, { attempt: { runtimeIdentity: { ...identity, runtimeSha256: '0'.repeat(64) } } }), /frozen SDK identity changed/);
  await assert.rejects(runtime.create(path.join(root, '..', 'escape'), {}), /attempt directory/);
  assert.deepEqual(await fs.readdir(directory), []);
});
