import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {mkdtemp, readFile, rm} from 'node:fs/promises';
import {join, resolve, dirname, basename} from 'node:path';
import {tmpdir} from 'node:os';
import {checkBundle, projectBundle, renderProjection, verifyMarkdown} from '../scripts/storyboard.mjs';
import {initializeRegistry, readRegistry, createImageQueue} from '../scripts/assets.mjs';

// Synthetic data: checks reference selection and handoff, not media quality.
const normalKey = '值守者·面罩CH';
const pairName = '值守者·面罩带Q版';
const pairKey = pairName + 'CH';
function fixture() {
  const b = JSON.parse(readFileSync(new URL('./fixtures/export-bundle.json', import.meta.url), 'utf8'));
  b.exportProfile = 'lingmo-v7';
  b.assets.push({key: pairKey, name: pairName, type: 'CH', status: '正常与3D Q版同图对照'});
  b.assets.forEach((a, i) => {a.importName = a.name.replaceAll('·', ''); a.placeholderOrdinal = i;});
  for (const e of b.episodes) for (const u of e.draft.units) u.imageBindings = [...u.bindings];
  return b;
}
function exportAndVerify(b) {
  assert.equal(checkBundle(b).ok, true);
  const md = renderProjection(projectBundle(b));
  assert.equal(verifyMarkdown(b, md).ok, true);
  return md;
}

test('无Q片段只选正常图，即使总库已存在带Q对照资产', () => {
  const md = exportAndVerify(fixture());
  assert.ok(md.includes('@[值守者面罩CH]'));
  assert.ok(!md.includes('@[值守者面罩带Q版CH]'));
});

test('正常转主观Q共用一张对照图，正常身份和原声绑定不变', () => {
  const b = fixture(), e = b.episodes[0], u = e.draft.units[0];
  const source = structuredClone(e.source), voices = structuredClone(u.voiceBindings);
  u.visualSubjects = [{name: '值守者Q化身', domain: 'subjective'}];
  u.visualSubjectBindings = [{name: '值守者Q化身', domain: 'subjective', assetKey: pairKey}];
  u.bindings.push(pairKey);
  e.scenes.find(s => s.id === u.sceneId).bindings.push(pairKey);
  u.imageBindings = u.imageBindings.filter(k => k !== normalKey).concat(pairKey);
  u.shots[0].picture += '主观表现转为同一人的3D Q形态，形态交替出现。';
  u.shots[1].picture += '恢复正常形态，沿原动作继续。';
  const md = exportAndVerify(b);
  assert.equal((md.match(/@\[值守者面罩带Q版CH\]/g) || []).length, 1);
  assert.ok(!md.includes('@[值守者面罩CH]'));
  assert.deepEqual(u.participants, ['值守者·面罩']);
  assert.deepEqual(u.voiceBindings, voices);
  assert.deepEqual(e.source, source);
  assert.ok(u.bindings.includes(normalKey));
});

test('默认到婚礼到婚礼带Q的直接父设计与祖先链分别保留，母项按显式需求入队', async t => {
  const root = await mkdtemp(join(tmpdir(), 'q-pair-assets-'));
  assert.equal(dirname(resolve(root)), resolve(tmpdir()));
  assert.ok(basename(root).startsWith('q-pair-assets-'));
  t.after(() => rm(root, {recursive: true, force: true}));
  const names = ['主角X默认形象', '主角X婚礼形象', '主角X婚礼形象带Q版'];
  const assets = names.map((name, i) => ({
    id: `A000${i + 1}`, revision: 1, name, key: name + 'CH', type: 'CH',
    originalName: '主角X', familyId: 'X', ...(i ? {parentId: `A000${i}`} : {}),
    status: i === 2 ? '婚礼正常与3D Q同图' : '正常形象', aliases: [],
    origin: {kind: 'design', sourceRefs: ['E01:R1'], basis: 'synthetic fixture'},
    design: {state: 'TEXT_ADOPTED', text: name + '的合成测试文字卡',
      full: {body: {composition: i === 2 ? '同图正常婚礼形象及对应3D Q版' : '单正常形象'}},
      dependencies: i ? [{assetId: `A000${i}`, assetRevision: 1, purpose: '继承直接父项的当前采用造型'}] : []}
  }));
  await initializeRegistry(root, {sourceHash: 'd'.repeat(64), importProfile: 'lingmo-v7', assets});
  const loaded = await readRegistry(root), snapshot = await readFile(loaded.file);
  const use = assetId => ({assetId, assetRevision: 1, episode: 1, sceneId: '1-1', purpose: '合成测试明确制图需求'});
  const request = uses => ({version: 'series-image-request-v1', baseRevision: loaded.registry.revision, uses});
  const leafOnly = await createImageQueue(root, request([use('A0003')]));
  assert.deepEqual(leafOnly.items.map(x => x.assetId), ['A0003']);
  assert.deepEqual(leafOnly.items[0].design.dependencies.map(x => x.assetId), ['A0002']);
  assert.deepEqual(leafOnly.items[0].dependencies.map(x => x.assetId), ['A0002', 'A0001']);
  assert.equal(loaded.registry.assets[2].parentId, 'A0002');
  assert.equal(loaded.registry.assets[1].parentId, 'A0001');
  const all = await createImageQueue(root, request(['A0001', 'A0002', 'A0003'].map(use)));
  assert.equal(all.items.length, 3);
  assert.equal(all.items[2].design.full.body.composition, '同图正常婚礼形象及对应3D Q版');
  assert.deepEqual(await readFile(loaded.file), snapshot);
});
