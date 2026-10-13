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
function frozenRecord(identity, attemptId) {
  const compiledSchema = '{ "type": "object" }';
  return { attempt: { attemptId, runtimeIdentity: identity,
    request: { model: 'gpt-6.1-sol', reasoning: 'medium' }, semanticRequestHash: 'a'.repeat(64), executionFingerprint: 'b'.repeat(64) },
    compiledPromptSha256: hash('synthetic only'), compiledSchema, compiledSchemaSha256: hash(compiledSchema) };
}

test('native runtime accepts normalized state roots with trailing separators', async t => {
  const { root, identity, config } = await fixture(t);
  const id = 'normalized-attempt', directory = path.join(root, id); await fs.mkdir(directory);
  const result = await nativeRuntime(config, root + path.sep).create(directory + path.sep, frozenRecord(identity, id));
  assert.equal(result.workingDirectory, path.join(directory, 'native-runtime', 'work'));
});

test('Windows Rust forward-slash state roots accept Node backslash attempt paths', { skip: process.platform !== 'win32' }, async t => {
  const { root, identity, config } = await fixture(t);
  for (const suffix of ['', '/', '//']) {
    const id = `slash-attempt-${suffix.length}`, directory = path.join(root, id); await fs.mkdir(directory);
    const result = await nativeRuntime(config, root.replaceAll('\\', '/') + suffix).create(directory, frozenRecord(identity, id));
    assert.equal(result.workingDirectory, path.join(directory, 'native-runtime', 'work'));
    assert.equal(JSON.parse(await fs.readFile(result.env.DIRECTOR_SDK_RUNTIME_PLAN)).attemptId, id);
  }
});

test('native runtime rejects parent traversal, nested/sibling directories and linked attempts before writing', async t => {
  const { root, identity, config } = await fixture(t);
  const directory = path.join(root, 'attempt'); await fs.mkdir(directory);
  const runtime = nativeRuntime(config, root + path.sep);
  for (const candidate of [path.join(root, 'nested', 'attempt'), `${root}${path.sep}other${path.sep}..${path.sep}attempt`, path.join(root, '..', 'sibling'), 'attempt']) {
    await assert.rejects(runtime.create(candidate, frozenRecord(identity, 'attempt')), /attempt directory/);
  }
  const target = path.join(root, 'target'); await fs.mkdir(target);
  const link = path.join(root, 'linked'); await fs.symlink(target, link, process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(runtime.create(link, frozenRecord(identity, 'linked')), /must not be linked/);
  await fs.mkdir(path.join(target, 'attempt'));
  await assert.rejects(nativeRuntime(config, link).create(path.join(link, 'attempt'), frozenRecord(identity, 'attempt')), /must not be linked/);
  await assert.rejects(runtime.create(directory, frozenRecord(identity, 'different-attempt')), /attempt directory identity changed/);
  assert.deepEqual(await fs.readdir(directory), []);
  assert.deepEqual(await fs.readdir(path.join(target, 'attempt')), []);
});
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
test('wrapper repair runtime uses an exclusive separate directory and preserves original runtime bytes',async t=>{
  const {root,identity,config}=await fixture(t),id='wrapper-repair',directory=path.join(root,id);await fs.mkdir(directory);
  await fs.mkdir(path.join(directory,'native-runtime'));await fs.writeFile(path.join(directory,'native-runtime/runtime-plan.json'),'immutable failed plan');
  const record=frozenRecord({...identity,supervisorSha256:'a'.repeat(64)},id);
  record.preDispatchRecovery={effectiveIdentity:{...identity,supervisorSha256:'b'.repeat(64)}};
  record.wrapperPreparationRepair={effectiveIdentity:identity};
  const runtime=nativeRuntime(config,root);const result=await runtime.create(directory,record);
  assert.equal(result.workingDirectory,path.join(directory,'native-runtime-wrapper-repair/work'));
  assert.equal(await fs.readFile(path.join(directory,'native-runtime/runtime-plan.json'),'utf8'),'immutable failed plan');
  await assert.rejects(runtime.create(directory,record),{code:'EEXIST'});
});
