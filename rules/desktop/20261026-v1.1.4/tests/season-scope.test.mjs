import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, rmSync, existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {validateSeasonScope, hashSeasonScope, assertSeasonUse, checkSeasonBundle} from '../scripts/season-scope.mjs';

const base = fileURLToPath(new URL('..', import.meta.url));
const cli = args => spawnSync(process.execPath, [join(base, 'scripts/season-scope.mjs'), ...args], {encoding: 'utf8'});
const scope = () => ({
  version: 'series-season-scope-v1', id: 'season-1', revision: 1, seasonNumber: 1, status: 'locked',
  sourceHash: 'a'.repeat(64),
  readingScope: [{number: 8, sceneIds: ['8-A', '8-B']}, {number: 3, sceneIds: ['3-X']}, {number: 21, sceneIds: ['21-Y']}],
  episodes: [{number: 8, sceneIds: ['8-A', '8-B']}, {number: 3, sceneIds: ['3-X']}],
  target: {minSeconds: 7200, maxSeconds: 9000, planningSeconds: 8100},
  ending: {episode: 3, sceneId: '3-X', sourceRef: 'source-body.txt:L22-L26', closure: '源文中的本季主冲突在此兑现。', hook: '此场原生悬念指向后续。', estimate: {minSeconds: 7500, maxSeconds: 8500, basis: '人工逐段计时区间，尚非实测。'}},
  confirmation: {kind: 'delegated', basis: '本会话已授权按原生钩子选整集季终。'}
});

test('a wholly off-target estimate remains proposed until an explicit duration exception', async () => {
  const {validateSeasonScope} = await import('../scripts/season-scope.mjs');
  for (const estimate of [{minSeconds:600,maxSeconds:1200},{minSeconds:10000,maxSeconds:11000}]) {
    const s=scope();s.ending.estimate=estimate;
    assert.throws(()=>validateSeasonScope(s), {code:'SEASON_ESTIMATE_OUTSIDE_TARGET'});
    s.status='proposed';assert.equal(validateSeasonScope(s,{requireLocked:false}),s);
    s.status='locked';s.durationException={basis:'test_fixture: 用户明确接受此候选时长偏离',confirmation:{kind:'user',basis:'test_fixture 本次偏离裁决'}};
    assert.equal(validateSeasonScope(s),s);
  }
});
const index = s => ({version: 'series-source-v1', sourceHash: s.sourceHash, episodes: s.readingScope.map(e => ({number: e.number, scenes: e.sceneIds.map(id => ({id, sourceRefs: ['source-body.txt:L1-L26']}))}))});
const nextScope = previous => {
  const s = structuredClone(previous); s.id = 'season-2'; s.seasonNumber = 2; s.episodes = [structuredClone(s.readingScope[2])];
  s.ending.episode = 21; s.ending.sceneId = '21-Y';
  s.continuation = {previousScopeHash: hashSeasonScope(previous), previousSeasonId: previous.id, nextEpisode: 21};
  return s;
};
const bundle = (s = scope()) => ({
  productionScope: structuredClone(s), sourceHash: s.sourceHash,
  scope: structuredClone(s.episodes),
  episodes: s.episodes.map(e => ({number: e.number, scenes: e.sceneIds.map(id => ({id})), draft: {units: e.sceneIds.map(sceneId => ({sceneId, shots: [{duration: 2700}]}))}}))
});
const code = expected => error => error.code === expected;
const subset = b => { b.episodes = b.episodes.slice(0, 1); b.episodes[0].scenes = b.episodes[0].scenes.slice(0, 1); b.episodes[0].draft.units = b.episodes[0].draft.units.slice(0, 1); b.scope = [{number: 8, sceneIds: ['8-A']}]; return b; };

test('locked validation returns the same object; proposed cannot dispatch', () => {
  const s = scope(); assert.equal(validateSeasonScope(s), s); assert.equal(assertSeasonUse(s, {episode: 8, sceneId: '8-A'}, 'author'), s);
  s.status = 'proposed'; delete s.confirmation;
  assert.equal(validateSeasonScope(s, {requireLocked: false}), s);
  assert.throws(() => validateSeasonScope(s), code('SEASON_NOT_LOCKED'));
  assert.throws(() => assertSeasonUse(s, {episode: 8}), code('SEASON_NOT_LOCKED'));
});
test('locked confirmation needs a recorded kind and basis', () => {
  for (const confirmation of [undefined, {kind: 'user', basis: ''}, {kind: 'automatic', basis: 'guess'}]) {
    const s = scope(); s.confirmation = confirmation;
    assert.throws(() => validateSeasonScope(s), code('SEASON_CONFIRMATION'));
  }
});
test('source order is authoritative even for non-monotonic episode numbers', () => {
  const s = scope(); assert.equal(validateSeasonScope(s, {index: index(s)}), s);
  for (const episodes of [[s.readingScope[0], s.readingScope[2]], [s.readingScope[1], s.readingScope[0]], [s.readingScope[1]]]) {
    assert.throws(() => validateSeasonScope({...s, episodes}), code('SEASON_CONTINUITY'));
  }
});
test('whole-episode boundary refuses partial scenes and non-final ending scenes', () => {
  const partial = scope(); partial.episodes[0].sceneIds.pop();
  assert.throws(() => validateSeasonScope(partial), code('SEASON_WHOLE_EPISODE'));
  const s = scope(); s.ending.sceneId = '8-B';
  assert.throws(() => validateSeasonScope(s), code('SEASON_ENDING'));
});
test('second season requires a previous-season hash and the recorded next episode', () => {
  const s = scope(); s.id = 'season-2'; s.seasonNumber = 2; s.episodes = [s.readingScope[2]];
  s.ending.episode = 21; s.ending.sceneId = '21-Y';
  s.continuation = {previousScopeHash: 'b'.repeat(64), previousSeasonId: 'season-1', nextEpisode: 21};
  assert.equal(validateSeasonScope(s), s);
  s.continuation.nextEpisode = 3; assert.throws(() => validateSeasonScope(s), code('SEASON_CONTINUATION'));
  delete s.continuation; assert.throws(() => validateSeasonScope(s), code('SEASON_CONTINUATION'));
});
test('scope fields, source hash, complete reading catalog and approved target cannot drift', () => {
  const s = scope();
  assert.throws(() => validateSeasonScope(s, {sourceHash: 'b'.repeat(64)}), code('SEASON_SOURCE_HASH'));
  const changed = index(s); changed.episodes[2].scenes[0].id = 'wrong';
  assert.throws(() => validateSeasonScope(s, {index: changed}), code('SEASON_READING_SCOPE'));
  const otherSource = index(s); otherSource.sourceHash = 'b'.repeat(64);
  assert.throws(() => validateSeasonScope(s, {index: otherSource}), code('SEASON_SOURCE_HASH'));
  for (const mutation of [v => v.target.minSeconds = 1, v => v.target.planningSeconds = 8000]) {
    const v = scope(); mutation(v); assert.throws(() => validateSeasonScope(v), code('SEASON_TARGET'));
  }
  const duplicate = scope(); duplicate.readingScope.push(structuredClone(duplicate.readingScope[0]));
  assert.throws(() => validateSeasonScope(duplicate), code('SEASON_READING_SCOPE'));
});
test('a full source index may embed productionScope before its first projection', () => {
  const s = scope(), raw = {...index(s), productionScope: structuredClone(s)};
  assert.equal(validateSeasonScope(s, {index: raw}), s);
  raw.productionScope.revision++;
  assert.throws(() => validateSeasonScope(s, {index: raw}), code('SEASON_SCOPE_HASH'));
});
test('a next season can reuse the full original index bound to its exact preceding scope', () => {
  const previous = scope(), next = nextScope(previous), original = {...index(previous), productionScope: structuredClone(previous)};
  assert.equal(validateSeasonScope(next, {index: original}), next);
  assert.equal(original.productionScope.id, previous.id);
  for (const mutate of [
    s => s.continuation.previousScopeHash = 'b'.repeat(64),
    s => s.continuation.previousSeasonId = 'unrelated-season'
  ]) {
    const changed = structuredClone(next); mutate(changed);
    assert.throws(() => validateSeasonScope(changed, {index: original}), code('SEASON_SCOPE_HASH'));
  }
  const wrongSource = structuredClone(original); wrongSource.productionScope.sourceHash = 'b'.repeat(64);
  const wrongSourceNext = nextScope(wrongSource.productionScope);
  wrongSourceNext.sourceHash = previous.sourceHash;
  assert.throws(() => validateSeasonScope(wrongSourceNext, {index: wrongSource}), code('SEASON_SOURCE_HASH'));
  const invalidPrevious = structuredClone(original); invalidPrevious.productionScope.ending.hook = '';
  const invalidNext = nextScope(invalidPrevious.productionScope); invalidNext.ending.hook = '续季自己的合法原生悬念。';
  assert.throws(() => validateSeasonScope(invalidNext, {index: invalidPrevious}), code('SEASON_ENDING'));
  const wrongReading = structuredClone(original); wrongReading.productionScope.readingScope[2].sceneIds = ['altered-scene'];
  const wrongReadingNext = structuredClone(next); wrongReadingNext.continuation.previousScopeHash = hashSeasonScope(wrongReading.productionScope);
  assert.throws(() => validateSeasonScope(wrongReadingNext, {index: wrongReading}), code('SEASON_READING_SCOPE'));
  const incomplete = structuredClone(original); incomplete.episodes.pop();
  assert.throws(() => validateSeasonScope(next, {index: incomplete}), code('SEASON_READING_SCOPE'));
  const projected = {...original, readingScope: previous.readingScope, scopeHash: hashSeasonScope(previous)};
  assert.throws(() => validateSeasonScope(next, {index: projected}), code('SEASON_SCOPE_HASH'));
});
test('stable scope hashes ignore object key order and preserve array order', () => {
  const s = scope(); const reversed = Object.fromEntries(Object.entries(s).reverse());
  assert.equal(hashSeasonScope(s), hashSeasonScope(reversed));
  const changed = scope(); changed.readingScope.reverse(); assert.notEqual(hashSeasonScope(s), hashSeasonScope(changed));
  assert.throws(() => hashSeasonScope({...s, extra: undefined}), code('SEASON_JSON'));
  assert.throws(() => hashSeasonScope({...s, extra: NaN}), code('SEASON_JSON'));
});
test('episode and scene consumers cannot use outside the locked selection', () => {
  const s = scope();
  assert.throws(() => assertSeasonUse(s, {episode: 21}), code('SEASON_OUT_OF_SCOPE'));
  assert.throws(() => assertSeasonUse(s, {episode: 8, sceneId: '3-X'}), code('SEASON_OUT_OF_SCOPE'));
});
test('preview permits in-season subsets and reports uncovered scenes and actual seconds', () => {
  const result = checkSeasonBundle(subset(bundle()));
  assert.equal(result.legacy, false); assert.equal(result.coverage.complete, false);
  assert.deepEqual(result.coverage.selectedEpisodes, [8, 3]);
  assert.deepEqual(result.coverage.missingScenes, [{episode: 8, sceneId: '8-B'}, {episode: 3, sceneId: '3-X'}]);
  assert.equal(result.duration.actualSeconds, 2700); assert.equal(result.duration.status, 'outside-target');
});
test('final and requireComplete both refuse incomplete selected-season coverage', () => {
  const b = subset(bundle()); b.seasonDelivery = 'final';
  assert.throws(() => checkSeasonBundle(b), code('SEASON_INCOMPLETE'));
  delete b.seasonDelivery; assert.throws(() => checkSeasonBundle(b, {requireComplete: true}), code('SEASON_INCOMPLETE'));
  const emptyScene = bundle(); emptyScene.episodes[0].draft.units[1].shots = []; emptyScene.seasonDelivery = 'final';
  assert.throws(() => checkSeasonBundle(emptyScene), code('SEASON_INCOMPLETE'));
});
test('final complete bundle measures shot durations rather than declared container totals', () => {
  const b = bundle(); b.seasonDelivery = 'final'; b.totalSeconds = 1;
  const result = checkSeasonBundle(b);
  assert.equal(result.coverage.complete, true); assert.deepEqual(result.coverage.coveredEpisodes, [8, 3]);
  assert.equal(result.duration.actualSeconds, 8100); assert.equal(result.duration.status, 'within-target');
});
test('source, scope declaration and actual scenes cannot drift or silently downgrade', () => {
  const s = scope(); const b = bundle(s);
  const noDeclaration = structuredClone(b); delete noDeclaration.productionScope;
  assert.throws(() => checkSeasonBundle(noDeclaration, {scope: s}), code('SEASON_SCOPE_REQUIRED'));
  noDeclaration.scopeHash = hashSeasonScope(s); assert.throws(() => checkSeasonBundle(noDeclaration), code('SEASON_SCOPE_REQUIRED'));
  delete noDeclaration.scopeHash; noDeclaration.seasonDelivery = 'final'; assert.throws(() => checkSeasonBundle(noDeclaration), code('SEASON_SCOPE_REQUIRED'));
  const changed = structuredClone(b); changed.productionScope.revision++;
  assert.throws(() => checkSeasonBundle(changed, {scope: s}), code('SEASON_SCOPE_HASH'));
  b.scopeHash = 'b'.repeat(64); assert.throws(() => checkSeasonBundle(b), code('SEASON_SCOPE_HASH'));
  delete b.scopeHash; b.sourceHash = 'b'.repeat(64); assert.throws(() => checkSeasonBundle(b), code('SEASON_SOURCE_HASH'));
  delete b.sourceHash; assert.equal(checkSeasonBundle(b, {scope: s}).legacy, false);
  b.productionScope = null; assert.throws(() => checkSeasonBundle(b), code('SEASON_SCOPE'));
});
test('out-of-season episodes/scenes, duplicates, ordering and misleading local scope are refused', () => {
  for (const mutate of [
    b => b.episodes.push({number: 21, scenes: [{id: '21-Y'}], draft: {units: []}}),
    b => b.episodes[0].scenes[0].id = '21-Y',
    b => b.episodes.push(structuredClone(b.episodes[0])),
    b => b.episodes.reverse(),
    b => b.episodes[0].scenes.reverse(),
    b => b.episodes[0].draft.units[0].sceneId = '21-Y',
    b => b.scope[0].sceneIds.pop()
  ]) { const b = bundle(); mutate(b); assert.throws(() => checkSeasonBundle(b), /范围|顺序|重复|目录/); }
});
test('final actual duration outside approved range is refused without padding', () => {
  for (const seconds of [7000, 9200]) {
    const b = bundle(); b.seasonDelivery = 'final'; b.episodes[0].draft.units[0].shots[0].duration = seconds - 5400;
    b.paddingSeconds = 2000; b.totalSeconds = 8100;
    assert.throws(() => checkSeasonBundle(b), code('SEASON_DURATION_TARGET'));
  }
});
test('an explicit duration exception reports actual out-of-range seconds and its basis', () => {
  const s = scope(); s.durationException = {basis: '本季在原生钩子收束，授权保留不足120分钟。', confirmation: {kind: 'delegated', basis: '已获用户委托处理本项差异。'}};
  const b = bundle(s); b.seasonDelivery = 'final'; b.episodes[0].draft.units[0].shots[0].duration = 1600;
  const result = checkSeasonBundle(b);
  assert.equal(result.duration.actualSeconds, 7000); assert.equal(result.duration.status, 'exception-authorized');
  assert.deepEqual(result.duration.exception, s.durationException); assert.equal(result.duration.target.minSeconds, 7200);
  b.productionScope.durationException.confirmation.basis = '';
  assert.throws(() => checkSeasonBundle(b), code('SEASON_DURATION_EXCEPTION'));
});
test('actual duration retains nine decimal places and rejects invalid shot durations', () => {
  const b = bundle(); b.seasonDelivery = 'final';
  for (const e of b.episodes) for (const u of e.draft.units) u.shots[0].duration = 2400.000000001;
  assert.equal(checkSeasonBundle(b).duration.actualSeconds, 7200.000000003);
  for (const duration of [0, -1, Infinity, '2700', 1e-10]) {
    const invalid = bundle(); invalid.episodes[0].draft.units[0].shots[0].duration = duration;
    assert.throws(() => checkSeasonBundle(invalid), /时间|时长|精度/);
  }
});
test('pre-existing fixtures without a production scope keep explicit legacy compatibility', () => {
  for (const name of ['export-bundle.json', 'lingmo-v7-bundle.json']) {
    const old = JSON.parse(readFileSync(join(base, 'tests/fixtures', name), 'utf8'));
    assert.deepEqual(checkSeasonBundle(old), {legacy: true, scopeHash: null, coverage: null, duration: null});
  }
});

test('CLI accepts an explicitly locked continuous whole-episode season', () => {
  const dir = mkdtempSync(join(tmpdir(), 'season-scope-'));
  try {
    const input = join(dir, 'scope.json');
    writeFileSync(input, JSON.stringify(scope()));
    const result = cli(['check', '--input', input]);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout).scopeHash.length, 64);
  } finally { rmSync(dir, {recursive: true, force: true}); }
});

test('CLI projects only selected whole episodes, preserves reading metadata and refuses overwrite', () => {
  const dir = mkdtempSync(join(tmpdir(), 'season-projection-'));
  try {
    const s = scope(), original = index(s);
    original.title = '保留原索引元信息'; original.episodes[0].scenes[0].note = '源场说明原样保留';
    const input = join(dir, 'scope.json'), source = join(dir, 'index.json'), output = join(dir, 'selected.json');
    writeFileSync(input, JSON.stringify(s)); writeFileSync(source, JSON.stringify(original));
    const args = ['index', '--input', input, '--index', source, '--output', output];
    const first = cli(args); assert.equal(first.status, 0, first.stderr);
    const projected = JSON.parse(readFileSync(output, 'utf8'));
    assert.deepEqual(projected.episodes.map(e => e.number), [8, 3]);
    assert.equal(projected.episodes[0].scenes[0].note, '源场说明原样保留');
    assert.equal(projected.title, original.title); assert.deepEqual(projected.readingScope, s.readingScope);
    assert.deepEqual(projected.productionScope, s); assert.equal(projected.scopeHash, hashSeasonScope(s));
    assert.equal(validateSeasonScope(s, {index: projected}), s);
    const again = cli(args); assert.notEqual(again.status, 0); assert.match(again.stderr, /EEXIST/);
    assert.deepEqual(JSON.parse(readFileSync(output, 'utf8')), projected);
    const stdout = cli(['index', '--input', input, '--index', output, '--output', '-']);
    assert.equal(stdout.status, 0, stdout.stderr); assert.deepEqual(JSON.parse(stdout.stdout), projected);
    const drift = structuredClone(projected); drift.episodes.pop();
    assert.throws(() => validateSeasonScope(s, {index: drift}), code('SEASON_READING_SCOPE'));
    const hashDrift = structuredClone(projected); hashDrift.scopeHash = 'b'.repeat(64);
    assert.throws(() => validateSeasonScope(s, {index: hashDrift}), code('SEASON_SCOPE_HASH'));
  } finally { rmSync(dir, {recursive: true, force: true}); }
});

test('CLI can inspect proposed scope but refuses proposed dispatch indexes and source drift', () => {
  const dir = mkdtempSync(join(tmpdir(), 'season-proposed-'));
  try {
    const s = scope(); s.status = 'proposed'; delete s.confirmation;
    const input = join(dir, 'scope.json'), source = join(dir, 'index.json'), output = join(dir, 'selected.json');
    writeFileSync(input, JSON.stringify(s)); writeFileSync(source, JSON.stringify(index(s)));
    let result = cli(['check', '--input', input, '--index', source]);
    assert.equal(result.status, 0, result.stderr); assert.equal(JSON.parse(result.stdout).status, 'proposed');
    result = cli(['index', '--input', input, '--index', source, '--output', output]);
    assert.notEqual(result.status, 0); assert.match(result.stderr, /SEASON_NOT_LOCKED/); assert.equal(existsSync(output), false);
    const changed = index(s); changed.sourceHash = 'b'.repeat(64); writeFileSync(source, JSON.stringify(changed));
    result = cli(['check', '--input', input, '--index', source]);
    assert.notEqual(result.status, 0); assert.match(result.stderr, /SEASON_SOURCE_HASH/);
  } finally { rmSync(dir, {recursive: true, force: true}); }
});

test('CLI creates a new next-season projection from the unchanged full prior-scope index', () => {
  const dir = mkdtempSync(join(tmpdir(), 'season-next-index-'));
  try {
    const previous = scope(), next = nextScope(previous);
    const original = {...index(previous), productionScope: structuredClone(previous)};
    const input = join(dir, 'season-2.json'), source = join(dir, 'original-index.json'), output = join(dir, 'season-2-index.json');
    const originalBytes = JSON.stringify(original, null, 2) + '\n';
    writeFileSync(input, JSON.stringify(next)); writeFileSync(source, originalBytes);
    let result = cli(['check', '--input', input, '--index', source]);
    assert.equal(result.status, 0, result.stderr);
    result = cli(['index', '--input', input, '--index', source, '--output', output]);
    assert.equal(result.status, 0, result.stderr);
    const projected = JSON.parse(readFileSync(output, 'utf8'));
    assert.deepEqual(projected.episodes.map(e => e.number), [21]);
    assert.deepEqual(projected.readingScope, previous.readingScope);
    assert.deepEqual(projected.productionScope, next); assert.equal(projected.scopeHash, hashSeasonScope(next));
    assert.equal(readFileSync(source, 'utf8'), originalBytes);
    assert.equal(validateSeasonScope(next, {index: projected}), next);
    result = cli(['index', '--input', input, '--index', source, '--output', output]);
    assert.notEqual(result.status, 0); assert.match(result.stderr, /EEXIST/);
  } finally { rmSync(dir, {recursive: true, force: true}); }
});
