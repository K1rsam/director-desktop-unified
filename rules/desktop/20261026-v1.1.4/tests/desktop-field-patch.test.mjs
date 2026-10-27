import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtempSync, readFileSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {applyDelta, PATCH_VERSION} from '../scripts/apply-delta.mjs';

const bytes = value => Buffer.from(JSON.stringify(value, null, 2) + '\n');
const sha = value => createHash('sha256').update(value).digest('hex');
function fixture() {
  return {schemaVersion: 'performance-core-v4', unresolved: [], extra: {keep: '源外元数据'},
    fantasyRealization: [{candidateId: 'F1', status: 'realized', unitIds: ['U1', 'U2'], shotIds: ['S1', 'S2'], entry: '源许可内', exit: '接回原位'}],
    units: [1, 2, 3].map(n => ({id: `U${n}`, title: '原题', unitDirection: '原演出', initialState: '原入口', exitState: '原出口',
      shots: [{id: `S${n}`, picture: '原画面', camera: '原取景', duration: 3, sound: '原声音',
        dialogueParts: [{lineId: n < 3 ? 'L1' : 'L2', text: n === 1 ? '别急，' : '我在。', delivery: '自然', voiceStart: 0, voiceEnd: 2}]}]}))};
}
function patch(base, patches, extra = {}) {
  return {version: PATCH_VERSION, baseHash: sha(bytes(base)), patches, revisionNotes: '修取景与具体演出', unresolved: [], ...extra};
}
const change = (path, before, after) => ({path, before, after});

test('五处字段修订保留同句/幻想的未改邻镜、全部源文本与额外数据', () => {
  const base = fixture(), before = bytes(base);
  const result = applyDelta(before, patch(base, [
    change('/units/0/shots/0/picture', '原画面', '听到落词后眼神停住。'),
    change('/units/0/shots/0/camera', '原取景', '手与脸同框。'),
    change('/units/0/initialState', '原入口', '左手扶桌，右手空着。'),
    change('/units/0/shots/0/duration', 3, 3.4),
    change('/units/0/shots/0/dialogueParts/0/voiceEnd', 2, 2.4),
  ]));
  assert.deepEqual(result.units.slice(1), base.units.slice(1));
  assert.deepEqual(result.fantasyRealization, base.fantasyRealization);
  assert.deepEqual(result.extra, base.extra);
  assert.equal(result.units[0].shots[0].dialogueParts[0].text, '别急，');
  assert.deepEqual(bytes(base), before);
  const recovered = structuredClone(result);
  recovered.units[0] = structuredClone(base.units[0]); delete recovered.revisionNotes;
  assert.deepEqual(recovered, base);
});

test('直接修bundle只触及显式集号的draft，源/计划/资产及另一集原值不变', () => {
  const base = {version: 'example', sourceHash: 'immutable', assets: [{assetRevision: 3}],
    episodes: [{number: 59, source: {text: '原文'}, plan: {keep: true}, draft: fixture()}, {number: 60, draft: fixture()}]};
  const result = applyDelta(bytes(base), patch(base, [change('/units/0/shots/0/picture', '原画面', '修后画面')], {episode: 59}));
  assert.deepEqual(result.episodes[1], base.episodes[1]);
  assert.deepEqual(result.assets, base.assets);
  assert.deepEqual(result.episodes[0].source, base.episodes[0].source);
  assert.deepEqual(result.episodes[0].plan, base.episodes[0].plan);
  assert.throws(() => applyDelta(bytes(base), patch(base, [], {episode: 61})), {code: 'PATCH_EPISODE'});
});

test('无实质改动返回原值，不为精修造新正文', () => {
  const base = fixture();
  assert.deepEqual(applyDelta(bytes(base), patch(base, [])), base);
  assert.deepEqual(applyDelta(bytes(base), patch(base, [change('/units/0/title', '原题', '原题')])), base);
});

test('拒绝陈旧基稿、错误before和重复路径，不能覆盖后来编辑', () => {
  const base = fixture(), delta = patch(base, [change('/units/0/title', '原题', '新题')]);
  assert.throws(() => applyDelta(Buffer.concat([bytes(base), Buffer.from(' ')]), delta), {code: 'DELTA_STALE'});
  delta.patches[0].before = '别人的稿';
  assert.throws(() => applyDelta(bytes(base), delta), {code: 'PATCH_BEFORE'});
  delta.patches[0].before = '原题'; delta.patches.push(delta.patches[0]);
  assert.throws(() => applyDelta(bytes(base), delta), {code: 'PATCH_PATH'});
});

for (const path of ['/units/0/id', '/units/0/shots/0/dialogueParts/0/text', '/units/0/shots/0/sourceRefs',
  '/units/0/bindings', '/units/0/shots/0/dialogueParts/0/presentation', '/units/0/__proto__/polluted', '/units/00/title']) {
  test(`小修接口不能改源、身份、结构或表演分类：${path}`, () => {
    const base = fixture();
    assert.throws(() => applyDelta(bytes(base), patch(base, [change(path, '原值', '新值')])), {code: 'PATCH_PATH'});
  });
}

test('CLI直接生成完整bundle候选并拒绝覆盖，不修改基稿', () => {
  const dir = mkdtempSync(join(tmpdir(), 'desktop-field-patch-'));
  try {
    const base = {episodes: [{number: 1, draft: fixture()}]}, input = join(dir, 'base.json'), delta = join(dir, 'delta.json'), output = join(dir, 'candidate.json');
    writeFileSync(input, bytes(base)); writeFileSync(delta, bytes(patch(base, [change('/units/0/title', '原题', '新题')], {episode: 1})));
    const args = [fileURLToPath(new URL('../scripts/apply-delta.mjs', import.meta.url)), '--base', input, '--delta', delta, '--output', output];
    const run = spawnSync(process.execPath, args, {encoding: 'utf8'});
    assert.equal(run.status, 0, run.stderr);
    assert.equal(JSON.parse(readFileSync(output)).episodes[0].draft.units[0].title, '新题');
    assert.deepEqual(readFileSync(input), bytes(base));
    assert.notEqual(spawnSync(process.execPath, args, {encoding: 'utf8'}).status, 0);
  } finally { rmSync(dir, {recursive: true, force: true}); }
});
