import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { verifySourceLock } from './verify_source_lock.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const MANIFEST = 'rules/stage3/manifest.json';
const MIGRATION = 'docs/MIGRATED_SOURCE.json';
const INDEX = 'docs/SOURCE_INDEX.json';
const RUNTIME = 'apps/desktop/src-tauri/src/pipeline/stage3_pack.rs';
const CONFIG = 'apps/desktop/src-tauri/tauri.conf.json';

function fixture(t) {
  const temporaryRoot = realpathSync(tmpdir());
  const root = mkdtempSync(path.join(temporaryRoot, 'director-source-lock-'));
  const json = relative => JSON.parse(readFileSync(path.join(root, relative), 'utf8'));
  const write = (relative, content) => {
    const target = path.resolve(root, relative);
    const offset = path.relative(root, target);
    assert(offset && !path.isAbsolute(offset) && offset !== '..' && !offset.startsWith(`..${path.sep}`));
    mkdirSync(path.dirname(target), { recursive: true });
    writeFileSync(target, content);
  };
  const setJson = (relative, value) => write(relative, JSON.stringify(value));
  const mutate = (relative, update) => { const value = json(relative); update(value); setJson(relative, value); };
  const migration = JSON.parse(readFileSync(path.join(ROOT, MIGRATION), 'utf8'));
  for (const entry of migration.files) write(entry.destinationPath, 'synthetic adapted destination\n');
  for (const relative of [MANIFEST, MIGRATION, INDEX, CONFIG, RUNTIME,
    'docs/handoff/MAC_RUNTIME_TRANSFER_REQUIREMENTS.json']) {
    write(relative, readFileSync(path.join(ROOT, relative)));
  }
  const manifest = json(MANIFEST);
  write(manifest.adapter.evidence, 'Synthetic evidence document; never production acceptance.\n');
  for (const entry of manifest.files) write(entry.path, readFileSync(path.join(ROOT, entry.path)));
  const marker = path.join(root, 'source-lock-fixture-marker');
  writeFileSync(marker, 'synthetic-source-lock-fixture');
  t.after(() => {
    // Only this explicitly allocated isolated fixture may be removed.
    const actual = realpathSync(root);
    assert.equal(path.dirname(actual), temporaryRoot);
    assert(path.basename(actual).startsWith('director-source-lock-'));
    assert.equal(readFileSync(marker, 'utf8'), 'synthetic-source-lock-fixture');
    rmSync(actual, { recursive: true, force: true });
  });
  return { root, json, write, mutate, verify: () => verifySourceLock(root) };
}

test('active compatible adapter passes without claiming original Mac runtime or live quality', t => {
  const f = fixture(t);
  const report = f.verify();
  assert.equal(report.status, 'passed');
  assert.equal(report.frozenCandidates, 22);
  assert.equal(report.pinnedSourceFiles, 144);
  assert.equal(report.overlappingHistoricalHashes, 46);
  assert.equal(report.candidateRuntimeActive, true);
  assert.equal(report.runtimeProfile, 'mac-3008-compatible-rust/v1');
  assert.equal(report.originalMacRuntimePorted, false);
  assert.equal(report.liveGenerationVerified, false);
  assert.equal(report.episode1And2QualityCompared, false);
});

test('old inactive switch and unknown runtime profile cannot pass the new source lock', t => {
  const f = fixture(t);
  const original = f.json(MANIFEST);
  f.mutate(MANIFEST, value => { value.active = false; });
  assert.throws(f.verify, /explicitly active/);
  f.write(MANIFEST, JSON.stringify(original));
  f.mutate(MANIFEST, value => { value.adapter.profile = 'unknown/v1'; });
  assert.throws(f.verify);
  f.write(MANIFEST, JSON.stringify(original));
  f.mutate(MANIFEST, value => { value.adapter.normalRoleOrder.reverse(); });
  assert.throws(f.verify);
});

test('missing offline proof or false original-runtime and live-quality claims are rejected', t => {
  const f = fixture(t);
  const original = f.json(MANIFEST);
  for (const field of ['originalMacRuntimeBodyTransferred', 'liveGenerationVerified', 'episode1And2QualityCompared']) {
    f.write(MANIFEST, JSON.stringify(original));
    f.mutate(MANIFEST, value => { value.adapter[field] = true; });
    assert.throws(f.verify, field);
  }
  f.write(MANIFEST, JSON.stringify(original));
  f.mutate(MANIFEST, value => { value.adapter.offlineRegressionVerified = false; });
  assert.throws(f.verify);
});

test('changed frozen byte, omitted rule, and changed source hash never become acceptable', t => {
  const f = fixture(t);
  const original = f.json(MANIFEST);
  const relative = original.files[0].path;
  const bytes = readFileSync(path.join(f.root, relative));
  const corrupt = Buffer.from(bytes); corrupt[0] ^= 1;
  f.write(relative, corrupt);
  assert.throws(f.verify, /Frozen source hash changed/);
  f.write(relative, bytes);
  f.mutate(MANIFEST, value => { value.files.pop(); });
  assert.throws(f.verify, /count changed/);
  f.write(MANIFEST, JSON.stringify(original));
  f.mutate(MANIFEST, value => { value.files[0].sha256 = '0'.repeat(64); });
  assert.throws(f.verify, /Pinned source hash differs/);
  f.write(MANIFEST, JSON.stringify(original));
  f.mutate(INDEX, value => { value.creativeFiles[0].sha256 = '0'.repeat(64); });
  assert.throws(f.verify, /Pinned source hash differs/);
});

test('wrong actual runtime registry hash cannot hide behind a valid manifest', t => {
  const f = fixture(t);
  const hash = f.json(MANIFEST).files[0].sha256;
  const runtime = readFileSync(path.join(f.root, RUNTIME), 'utf8');
  f.write(RUNTIME, runtime.replace(hash, '0'.repeat(64)));
  assert.throws(f.verify, /Runtime frozen hash differs/);
});

test('historical migration commit, source hashes, duplicates and omitted records stay guarded', t => {
  const f = fixture(t);
  const original = f.json(MIGRATION);
  const historicalPath = f.json(INDEX).readSources
    .find(row => row.source === 'desktopSource' && original.files.some(file => file.sourcePath === row.path)).path;
  f.mutate(MIGRATION, value => { value.commit = '0'.repeat(40); });
  assert.throws(f.verify);
  f.write(MIGRATION, JSON.stringify(original));
  f.mutate(MIGRATION, value => {
    value.files.find(row => row.sourcePath === historicalPath).sourceSha256 = '0'.repeat(64);
  });
  assert.throws(f.verify, /Historical SHA-256 differs/);
  f.write(MIGRATION, JSON.stringify(original));
  f.mutate(MIGRATION, value => { value.files[1].destinationPath = value.files[0].destinationPath; });
  assert.throws(f.verify, /duplicate/);
  f.write(MIGRATION, JSON.stringify(original));
  f.mutate(INDEX, value => {
    value.readSources = value.readSources.filter(row => row.path !== historicalPath);
  });
  assert.throws(f.verify, /Historical source hash records were omitted/);
});

test('escaping paths and private runtime files remain forbidden', t => {
  const f = fixture(t);
  const original = f.json(MIGRATION);
  f.mutate(MIGRATION, value => { value.files[0].destinationPath = '../escape.txt'; });
  assert.throws(f.verify, /Non-canonical source path/);
  f.write(MIGRATION, JSON.stringify(original));
  f.write('synthetic/PROJECT.SQLITE', 'synthetic not a production database');
  assert.throws(f.verify, /Private runtime file found/);
});

test('private fixture dependency and native app isolation still fail closed', t => {
  const f = fixture(t);
  const runtime = readFileSync(path.join(f.root, RUNTIME), 'utf8');
  f.write(RUNTIME, runtime + '\n// foundation-real-a-20261005\n');
  assert.throws(f.verify, /Private report dependency/);
  f.write(RUNTIME, runtime);
  const config = f.json(CONFIG);
  f.mutate(CONFIG, value => { value.identifier = 'old.app.identity'; });
  assert.throws(f.verify);
  f.write(CONFIG, JSON.stringify(config));
  f.mutate(CONFIG, value => { value.app.security.assetProtocol.scope = ['**']; });
  assert.throws(f.verify);
});
