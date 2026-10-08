#!/usr/bin/env node
// Read-only provenance checks. No provider, account, model or project is opened.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const refs = {
  migrated: '2ac7c216183f936aa08f16a93b2a24130cf5ebcd',
  windows: '1e2bf93553c1235ba56fddc2141058e719393d1c',
  macBase: '2c952f67b96878a90338ab5770c7d864bcf1b78f',
  macCode: '05b64c920384200ce45be51ac9962c39714178cd',
  mac: '8b07ca276a82fe6d91cc1c3b3c0db58ed76e24af',
  history: 'cb4f728e1f22dbf20b88e0eae0cf0a11c6c85f13',
};
const windowsCommits = [
  '8548e9f5917b8960f693a052e395f3b6e4048b1e', 'aecbd7902ac548c4f922cbd7d1aae611012a3690',
  'dc1daef54e063933cc970dc01e72301de8b2b5b9', 'e4db55a2739fbf32088b14667aa1024a08a865d3',
  'd53099729c68b75b1792ae1269356e0be8b3863d', '57176ee6d5a315c962df6ded514b2f4980de20c2',
  'bee5f85862159bb2bf402aadfd5a3e718450d507', '0b4dfb6270188f36ae4c9cb435374d212ca1eb15',
  '147b517a50c37d764d797173eb31818293478ca1', 'c2faa5386a78fc94278135d4e14c6d7c492c4091',
  '87cfdaff0a272dfa47c482292a4055b62f330f62', '11c70a292d9fed96775826a073f25e206241fd3f',
  '97b0ca9dfca67afded70b4733e606e788c625e18', '390088c33cb5dafd776513378e2c73de3cd6c7f4',
  '6bc49ff9776bd930a8b5e1dde8f5d22a9dfbd7ef', 'a4e203d049e8d1e01431fd11b4f9a05f859847fa',
  '3fda9a4df091ff2155f3cecaa539a80253cbf051', '621e49e564f1251a71da9f5643dec04debdb8b79',
  refs.windows,
];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const lf = bytes => bytes.toString('utf8').replace(/\r\n/g, '\n');
let receiptPath, windowsRepo = repo;
for (let i = 2; i < process.argv.length; i++) {
  const flag = process.argv[i];
  if (flag === '--help') {
    console.log('node scripts/verify-integration-lineage.mjs [--windows-source-repo <repo>] [--receipt <new absolute file>]');
    process.exit(0);
  }
  assert.ok(['--receipt', '--windows-source-repo'].includes(flag), 'Unknown lineage option');
  assert.ok(process.argv[i + 1], 'Missing lineage option value');
  const value = process.argv[++i];
  if (flag === '--receipt') {
    assert.ok(path.isAbsolute(value) && !fs.existsSync(value), 'Receipt path must be absolute and new');
    receiptPath = value;
  } else windowsRepo = path.resolve(value);
}
function git(args, directory = repo, required = true) {
  const result = spawnSync('git', ['-C', directory, ...args], { maxBuffer: 16 * 1024 * 1024, windowsHide: true });
  if (required) assert.equal(result.status, 0, `Required Git evidence unavailable: ${args[0]}`);
  return result;
}
function gitBytes(ref, file, directory = repo) { return git(['show', `${ref}:${file}`], directory).stdout; }
function local(file) {
  assert.ok(typeof file === 'string' && !path.isAbsolute(file) && !file.includes('\\'), 'Invalid repository-relative path');
  const resolved = path.resolve(repo, file), physical = fs.realpathSync(resolved);
  for (const value of [resolved, physical]) {
    const relative = path.relative(repo, value);
    assert.ok(relative && !relative.startsWith('..') && !path.isAbsolute(relative), 'Lineage path leaves repository');
  }
  assert.ok(fs.statSync(physical).isFile(), 'Lineage evidence must be a file');
  return fs.readFileSync(physical);
}
function json(file) { return JSON.parse(local(file).toString('utf8')); }

const parent = ref => git(['rev-parse', `${ref}^`]).stdout.toString('utf8').trim();
assert.equal(parent(refs.mac), refs.macCode, 'Full guide is a documentation child of Mac code');
assert.equal(parent(refs.macCode), refs.macBase);
assert.equal(parent(refs.history), refs.macBase, 'Code and history handoffs share the same parent');
const macChanges = git(['diff-tree', '--no-commit-id', '--name-status', '-r', '-z', refs.macCode]).stdout.toString('utf8').split('\0').filter(Boolean);
const macChangeCounts = { modified: 0, added: 0 };
assert.equal(macChanges.length, 104 * 2, 'Mac release changed-file count');
for (let i = 0; i < macChanges.length; i += 2) {
  assert.ok(['M', 'A'].includes(macChanges[i]), 'Unexpected Mac change status');
  macChangeCounts[macChanges[i] === 'M' ? 'modified' : 'added']++;
}
assert.deepEqual(macChangeCounts, { modified: 24, added: 80 });
function verifyRows(rows, count, label) {
  assert.equal(rows.length, count, `${label} count`);
  const seen = new Set();
  let totalBytes = 0;
  for (const row of rows) {
    assert.ok(!seen.has(row.path), `${label} duplicate path`); seen.add(row.path);
    assert.match(row.sha256, /^[a-f0-9]{64}$/, `${label} SHA256`);
    const bytes = local(row.path);
    assert.equal(bytes.length, row.bytes, `${label} byte length: ${row.path}`);
    assert.equal(digest(bytes), row.sha256, `${label} original bytes: ${row.path}`);
    totalBytes += bytes.length;
  }
  return { files: seen, bytes: totalBytes };
}
function tree(directory) {
  const result = [];
  for (const item of fs.readdirSync(path.join(repo, directory), { withFileTypes: true })) {
    assert.ok(!item.isSymbolicLink(), 'Frozen archive must not contain symbolic links');
    const file = `${directory}/${item.name}`;
    if (item.isDirectory()) result.push(...tree(file));
    else { assert.ok(item.isFile(), 'Unexpected frozen archive object'); result.push(file); }
  }
  return result.sort();
}

const index = json('reference-skills/index.json');
const macIndex = JSON.parse(gitBytes(refs.mac, 'reference-skills/index.json').toString('utf8'));
assert.deepEqual(index, macIndex, 'Historical Mac index must remain unchanged');
const snapshots = verifyRows(index.snapshots, 72, 'Reference snapshots');
assert.equal(snapshots.bytes, index.referenceSnapshotBytes);
const expectedGroups = { 'desktop-original-2ac7c216': 29, 'mac-3008-git-2511c56': 21, 'mac-3008-production-20261005': 22 };
const referenceRoles = { 'desktop-original-2ac7c216': 'reference_only_never_loaded', 'mac-3008-git-2511c56': 'earlier_git_reference_never_loaded', 'mac-3008-production-20261005': 'verified_legacy_production_not_loaded_by_next' };
for (const [group, count] of Object.entries(expectedGroups)) {
  const rows = index.snapshots.filter(row => row.group === group);
  assert.equal(rows.length, count, 'Independent reference group count');
  assert.ok(rows.every(row => row.runtimeRole === referenceRoles[group]));
  assert.deepEqual(tree(`reference-skills/${group}`), rows.map(row => row.path).sort());
}

const candidate = json('rules/stage3/manifest.json');
const macCandidate = JSON.parse(gitBytes(refs.mac, 'rules/stage3/manifest.json').toString('utf8'));
assert.equal(candidate.packId, macCandidate.packId, 'Mac production pack identity');
assert.equal(candidate.source.ref, macCandidate.source.ref, 'Mac production export source');
assert.deepEqual(candidate.files, macCandidate.files, 'Candidate path/hash/length locks must remain unchanged');
const candidates = verifyRows(candidate.files, 22, 'Mac candidate rules');
const candidateRoot = 'rules/stage3/candidates/mac-3008-20261006';
assert.deepEqual(tree(candidateRoot), [...candidates.files].sort());
for (const row of candidate.files) {
  const suffix = row.path.slice(candidateRoot.length + 1);
  const reference = `reference-skills/mac-3008-production-20261005/${suffix}`;
  assert.ok(snapshots.files.has(reference), 'Candidate has no production reference');
  assert.ok(local(row.path).equals(local(reference)), 'Production reference and candidate differ');
}

const historyManifest = json('docs/handoff/SKILL_ARTIFACT_MANIFEST.json');
const historySource = JSON.parse(gitBytes(refs.history, 'docs/handoff/SKILL_ARTIFACT_MANIFEST.json').toString('utf8'));
assert.deepEqual(historyManifest, historySource, 'History artifact index must remain unchanged');
const archive = verifyRows(historyManifest.records, 60, 'History original archives');
assert.deepEqual(tree('reference-archive'), [...archive.files].sort());
const historyPaths = git(['diff-tree', '--no-commit-id', '--name-only', '-r', '-z', refs.history]).stdout.toString('utf8').split('\0').filter(Boolean);
assert.equal(historyPaths.length, 65, 'Sibling history commit has 65 changed files');
assert.equal(historyPaths.filter(file => file.startsWith('reference-archive/')).length, 60);
for (const row of historyManifest.records) assert.ok(local(row.path).equals(gitBytes(refs.history, row.path)), 'History archive differs from its original Git blob');

// Parse the actual narrow Rust registration table, not archive folder names.
const registrySource = local('apps/desktop/src-tauri/src/pipeline/mod.rs').toString('utf8');
const registryStart = registrySource.indexOf('pub fn rule_snapshots() -> Vec<RuleSnapshot> {');
const registryEnd = registrySource.indexOf('.into_iter()', registryStart);
assert.ok(registryStart >= 0 && registryEnd > registryStart, 'Rust registration table not found');
const registration = [...registrySource.slice(registryStart, registryEnd).matchAll(/\(\s*"([^"]+)"\s*,\s*include_str!\("([^"]+)"\)\s*,?\s*\)/g)];
assert.equal(registration.length, 29, 'Desktop compiled registration count');
const known = new Map(index.registeredFiles.map(row => [row.id, row]));
const actual = [], rawMacUnchanged = [], newlineOnly = [];
for (const [, id, include] of registration) {
  const file = path.relative(repo, path.resolve(repo, 'apps/desktop/src-tauri/src/pipeline', include)).split(path.sep).join('/');
  const old = known.get(id); assert.ok(old && old.path === file, 'Unknown/moved desktop registration');
  const bytes = local(file), macBytes = gitBytes(refs.mac, file);
  assert.equal(digest(macBytes), old.sha256, 'Mac historical registration hash');
  const version = bytes.toString('utf8').split(/\r?\n/).map(line => line.trim()).find(line => line.startsWith('version:'))?.slice(8).trim() || '1.0.0';
  actual.push({ id, path: file, version, sha256: digest(bytes), bytes: bytes.length });
  if (bytes.equals(macBytes)) rawMacUnchanged.push(id);
  else if (lf(bytes) === lf(macBytes)) newlineOnly.push(id);
  else assert.equal(id, 'asset-image-production', 'Unrecorded semantic rule change');
}
assert.equal(new Set(actual.map(row => row.id)).size, 29);
const art = actual.find(row => row.id === 'asset-art-director');
assert.equal(art.version, '2.0.0');
assert.equal(art.sha256, '9193cd59b44bdc77fb413fb8c40bb6531c2cfb9377ca17fafc0749808ba6cd82', 'Mac art rule is byte-for-byte unchanged');
const production = actual.find(row => row.id === 'asset-image-production');
assert.equal(production.version, '1.2.0');
assert.equal(production.sha256, '7d4d32682a6261bbcff3a7f5cbffc3a34a6c313068fa0f39a3f34b454935194d', 'Windows image policy lock');
assert.notEqual(production.sha256, known.get(production.id).sha256);

const windowsAvailable = git(['cat-file', '-e', `${refs.windows}^{commit}`], windowsRepo, false).status === 0;
if (windowsAvailable) {
  const commits = git(['rev-list', '--reverse', `${refs.migrated}..${refs.windows}`], windowsRepo).stdout.toString('utf8').trim().split(/\r?\n/);
  assert.deepEqual(commits, windowsCommits, 'Windows follow-up history');
  assert.ok(local(production.path).equals(gitBytes(refs.windows, 'director-desktop/rules/asset-image-production/SKILL.md', windowsRepo)));
}

let adapterEntries = 0;
const adapterPath = 'apps/desktop/src-tauri/src/pipeline/stage3_pack.rs';
if (fs.existsSync(path.join(repo, adapterPath))) {
  const entries = [...local(adapterPath).toString('utf8').matchAll(/frozen!\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*"([a-f0-9]{64})"\s*\)/g)];
  assert.equal(entries.length, 22, 'Rust Mac-pack compiled source entries');
  assert.ok(registrySource.includes('pub mod stage3_pack;'), 'Mac-pack module not declared');
  assert.equal(new Set(entries.map(([, id]) => id)).size, 22);
  for (const [, , suffix, sha] of entries) assert.ok(candidate.files.some(row => row.path === `${candidateRoot}/${suffix}` && row.sha256 === sha), 'Mac-pack registration differs from frozen candidate');
  adapterEntries = entries.length;
}

const receipt = {
  schemaVersion: 'windows-mac-integration-lineage/v1', ok: true,
  refs, windowsHistory: { declaredCommits: 19, sourceGitVerified: windowsAvailable },
  macHandoff: { codeChanges: 104, ...macChangeCounts, historyWasSiblingBranch: true },
  referenceSnapshots: { count: 72, bytes: snapshots.bytes, groups: expectedGroups, loadedByFolderScan: false },
  candidateOriginals: { count: 22, productionReferenceMatches: 22, originalBytesVerified: true },
  siblingHistory: { changedFiles: 65, originalArchivesVerified: 60, documentationAndIndexes: 5 },
  desktopRegistry: { count: 29, digest: digest(JSON.stringify(actual)), macRawUnchangedCount: rawMacUnchanged.length, macNewlineOnlyCount: newlineOnly.length, semanticChanges: [production.id], art, production },
  stage3: { manifestActive: candidate.active, manifestStatus: candidate.status, activePackId: candidate.activePackId, compiledAdapterEntries: adapterEntries, runtimeAcceptanceProven: false },
  generationCalls: 0, accountCalls: 0, productionProjectsOpened: false,
};
const output = JSON.stringify(receipt);
if (receiptPath) fs.writeFileSync(receiptPath, `${output}\n`, { flag: 'wx' });
console.log(output);
