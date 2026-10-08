#!/usr/bin/env node
/** Frozen source bytes, exact compatible-adapter identity and migration hygiene. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, realpathSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const DEFAULT_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const DESKTOP_COMMIT = '2ac7c216183f936aa08f16a93b2a24130cf5ebcd';
const CREATIVE_COMMIT = '00093c632c45369106c7ef79a47b684fd5e77526';
const CREATIVE_PREFIX = 'skill-packs/3008-stage3/2026-10-06/stage3-storyboard-director/references/原版/';
const CANDIDATE_PREFIX = 'rules/stage3/candidates/mac-3008-20261006/';
const PACK_ID = 'mac-3008-original-export-20261006';
const PROFILE = 'mac-3008-compatible-rust/v1';
const ADAPTER = 'mac-3008-rust-adapter/v1';
const SELECTOR = 'source-bound-rust-selector/v1';
const DIALOGUE = 'locked-utf16-reference/v1';
const IMPLEMENTATION = 'reconstructed_compatible_rust_not_original_mac_runtime';
const SHA256 = /^[a-f0-9]{64}$/;
const SHA1 = /^[a-f0-9]{40}$/;

function withinRoot(root, relative) {
  assert.equal(typeof relative, 'string', 'Source path must be a string');
  assert(relative && !relative.includes('\\') && !path.win32.parse(relative).root
    && !path.posix.isAbsolute(relative)
    && relative.split('/').every(part => part && part !== '.' && part !== '..'),
  `Non-canonical source path: ${relative}`);
  const resolved = realpathSync(path.join(root, relative));
  const offset = path.relative(root, resolved);
  assert(offset && !path.isAbsolute(offset) && offset !== '..'
    && !offset.startsWith(`..${path.sep}`), `Source path escapes repository: ${relative}`);
  assert(statSync(resolved).isFile(), `Source file missing: ${relative}`);
  return resolved;
}

function jsonFile(root, relative) {
  return JSON.parse(readFileSync(withinRoot(root, relative), 'utf8'));
}

function uniqueRows(rows, field, count, description) {
  assert(Array.isArray(rows), `${description} must be an array`);
  assert.equal(rows.length, count, `${description} count changed`);
  const mapped = new Map(rows.map(row => [row[field], row]));
  assert.equal(mapped.size, rows.length, `${description} contains duplicate ${field}`);
  return mapped;
}

function* filesIn(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    // Do not traverse directory symlinks/junctions. Explicit locked files still
    // undergo realpath containment above before any bytes are accepted.
    if (entry.isDirectory()) yield* filesIn(full);
    else yield full;
  }
}

export function verifySourceLock(repository = DEFAULT_ROOT) {
  const root = realpathSync(repository);
  const manifest = jsonFile(root, 'rules/stage3/manifest.json');
  assert.equal(manifest.schemaVersion, 'creative-candidate-manifest/v1');
  assert.equal(manifest.packId, PACK_ID);
  assert.equal(manifest.status, 'production_rules_active_on_reconstructed_compatible_rust_adapter');
  assert.equal(manifest.active, true, 'Compatible Rust profile must be explicitly active');
  assert.equal(manifest.activePackId, PACK_ID);
  assert.equal(manifest.source.repository, 'K1rsam/director-web');
  assert.equal(manifest.source.ref, CREATIVE_COMMIT);
  assert.equal(manifest.source.path, CREATIVE_PREFIX);
  assert.equal(manifest.source.excludeIntegratedSkill, true);
  const adapter = manifest.adapter;
  assert.equal(adapter.profile, PROFILE);
  assert.equal(adapter.version, ADAPTER);
  assert.equal(adapter.implementation, IMPLEMENTATION);
  assert.equal(adapter.sourceSelector, SELECTOR);
  assert.equal(adapter.dialogueProfile, DIALOGUE);
  assert.deepEqual(adapter.normalRoleOrder, [
    'episode-emotion-planner', 'unit-performance-director', 'episode-finishing-editor',
  ]);
  assert.equal(adapter.independentRefinement, 'optional_candidate_then_explicit_atomic_adoption');
  assert.equal(adapter.offlineRegressionVerified, true);
  assert.equal(adapter.originalMacRuntimeBodyTransferred, false);
  assert.equal(adapter.liveGenerationVerified, false);
  assert.equal(adapter.episode1And2QualityCompared, false);
  assert.equal(adapter.evidence, 'docs/handoff/STAGE3_COMPATIBLE_RUST_ADAPTER_20261008.md');
  withinRoot(root, adapter.evidence);

  // Historical records remain historical; their old pending status is not used
  // to rewrite the new runtime's state. Their pinned hashes remain authorities.
  const index = jsonFile(root, 'docs/SOURCE_INDEX.json');
  assert.equal(index.version, 'source-lock/v1');
  assert.equal(index.desktopSource.ref, DESKTOP_COMMIT);
  assert.equal(index.creativeSource.ref, CREATIVE_COMMIT);
  assert.equal(index.creativeSource.path, CREATIVE_PREFIX);
  assert.equal(new Set(index.readSources.map(row => `${row.source}\0${row.path}`)).size,
    index.readSources.length, 'Independent source index contains ambiguous records');
  const originalFiles = uniqueRows(index.creativeFiles, 'sourceRelativePath', 22, 'Original creative index');
  const manifestFiles = uniqueRows(manifest.files, 'path', 22, 'Frozen candidates');
  const originalReads = new Map(index.readSources
    .filter(row => row.source === 'creativeSource').map(row => [row.path, row]));
  for (const entry of manifest.files) {
    assert(entry.path.startsWith(CANDIDATE_PREFIX), `Wrong candidate root: ${entry.path}`);
    const relative = entry.path.slice(CANDIDATE_PREFIX.length);
    const original = originalFiles.get(relative);
    assert(original, `Unregistered frozen candidate: ${relative}`);
    assert.equal(entry.sourcePath, CREATIVE_PREFIX + relative);
    assert.equal(entry.bytes, original.bytes, `Source byte count differs: ${relative}`);
    assert.equal(entry.sha256, original.sha256, `Pinned source hash differs: ${relative}`);
    assert(SHA256.test(entry.sha256), `Invalid source hash: ${relative}`);
    assert(Number.isSafeInteger(entry.bytes) && entry.bytes > 0, `Invalid source size: ${relative}`);
    const payload = readFileSync(withinRoot(root, entry.path));
    assert.equal(payload.length, entry.bytes, `Frozen bytes changed: ${relative}`);
    assert.equal(createHash('sha256').update(payload).digest('hex'), entry.sha256,
      `Frozen source hash changed: ${relative}`);
    const read = originalReads.get(entry.sourcePath);
    assert(read && SHA1.test(read.gitBlobSha), `Original Git blob record missing: ${relative}`);
    assert.equal(read.sha256, entry.sha256);
    assert.equal(read.bytes, entry.bytes);
    const gitBlob = createHash('sha1').update(Buffer.from(`blob ${payload.length}\0`)).update(payload).digest('hex');
    assert.equal(gitBlob, read.gitBlobSha, `Original Git blob hash changed: ${relative}`);
  }

  // Verify the actual registered source carries the same explicit recipe and
  // all 22 pinned hashes. This does not substitute for the Rust runtime tests.
  const runtime = readFileSync(withinRoot(root, 'apps/desktop/src-tauri/src/pipeline/stage3_pack.rs'), 'utf8');
  for (const [name, value] of Object.entries({
    PACK_ID, PROFILE, ADAPTER_VERSION: ADAPTER, SELECTOR_VERSION: SELECTOR, DIALOGUE_PROFILE: DIALOGUE,
  })) {
    assert(runtime.includes(`pub const ${name}: &str = "${value}";`), `Runtime constant differs: ${name}`);
  }
  const frozen = [...runtime.matchAll(/frozen!\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*"([a-f0-9]{64})"\s*\)/g)];
  assert.equal(frozen.length, 22, 'Runtime frozen registry count differs');
  assert.equal(new Set(frozen.map(row => row[1])).size, 22, 'Runtime rule IDs duplicate');
  assert.equal(new Set(frozen.map(row => row[2])).size, 22, 'Runtime rule paths duplicate');
  for (const [, , relative, hash] of frozen) {
    assert.equal(manifestFiles.get(CANDIDATE_PREFIX + relative)?.sha256, hash,
      `Runtime frozen hash differs: ${relative}`);
  }

  const transfer = jsonFile(root, 'docs/handoff/MAC_RUNTIME_TRANSFER_REQUIREMENTS.json');
  assert.equal(transfer.bodyTransferAuthorized, false);
  assert.equal(transfer.newStage3RuntimeIntegrated, false);
  assert.equal(transfer.files.length, 42);
  assert(transfer.files.every(row => row.bodyTransferred === false),
    'Compatible adapter must not claim original Mac runtime transfer');

  const source = jsonFile(root, 'docs/MIGRATED_SOURCE.json');
  assert.equal(source.schema, 'selective-source-migration/v1');
  assert.equal(source.repository, 'K1rsam/director-web');
  assert.equal(source.commit, DESKTOP_COMMIT);
  uniqueRows(source.files, 'destinationPath', 144, 'Pinned migration');
  assert.equal(new Set(source.files.map(row => row.sourcePath)).size, source.files.length,
    'Pinned migration source paths duplicate');
  const historicalReads = new Map(index.readSources
    .filter(row => row.source === 'desktopSource').map(row => [row.path, row]));
  let overlappingHistoricalHashes = 0;
  for (const row of source.files) {
    assert.equal(row.sourceCommit, DESKTOP_COMMIT);
    assert(SHA1.test(row.sourceGitBlobSha1) && SHA256.test(row.sourceSha256),
      `Invalid migration source hashes: ${row.sourcePath}`);
    assert(Number.isSafeInteger(row.bytes) && row.bytes >= 0, `Invalid source bytes: ${row.sourcePath}`);
    withinRoot(root, row.destinationPath);
    // Adapted destinations intentionally evolve. Compare historical source
    // records, never force current code back to its pre-adaptation byte hash.
    const original = historicalReads.get(row.sourcePath);
    if (original) {
      assert.equal(row.sourceGitBlobSha1, original.gitBlobSha, `Historical Git hash differs: ${row.sourcePath}`);
      assert.equal(row.sourceSha256, original.sha256, `Historical SHA-256 differs: ${row.sourcePath}`);
      assert.equal(row.bytes, original.bytes, `Historical byte count differs: ${row.sourcePath}`);
      overlappingHistoricalHashes += 1;
    }
  }
  assert.equal(overlappingHistoricalHashes, 46, 'Historical source hash records were omitted');

  for (const full of filesIn(root)) {
    const name = path.basename(full).toLowerCase();
    assert(!name.endsWith('.sqlite') && !name.endsWith('.sqlite-wal') && !name.endsWith('.sqlite-shm')
      && !['.env', '.env.local', 'auth.json'].includes(name), `Private runtime file found: ${full}`);
    const relative = path.relative(path.join(root, 'apps/desktop/src-tauri'), full);
    if (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`)
      && name.endsWith('.rs')) {
      assert(!readFileSync(full, 'utf8').includes('foundation-real-a-20261005'),
        `Private report dependency: ${full}`);
    }
  }
  const config = jsonFile(root, 'apps/desktop/src-tauri/tauri.conf.json');
  assert.equal(config.identifier, 'app.director.next');
  assert.deepEqual(config.app.security.assetProtocol.scope, []);
  return {
    status: 'passed', frozenCandidates: manifest.files.length, pinnedSourceFiles: source.files.length,
    overlappingHistoricalHashes, candidateRuntimeActive: manifest.active,
    runtimeProfile: PROFILE, runtimeImplementation: IMPLEMENTATION, originalMacRuntimePorted: false,
    liveGenerationVerified: false, episode1And2QualityCompared: false,
    checks: ['candidate bytes and SHA-256', 'original Git blob and independent source index',
      'exact runtime profile and frozen registry', 'pinned migration index', 'private fixture exclusion',
      'new app identity', 'empty default asset scope'],
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try { console.log(JSON.stringify(verifySourceLock())); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
