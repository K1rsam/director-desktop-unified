import test from 'node:test';
import assert from 'node:assert/strict';
import { assetDeletionReady, assetEditPayload, assetMatchesEpisode, assetScopeProblems, deletedAssetsFrom, initialAssetEdit, toggleAssetScope } from './assetManagement.ts';
import type { Asset, SourceRef } from './types.ts';
import type { AssetUsage } from './assetManagement.ts';

const sourceRefs: SourceRef[] = ['row-1', { start: 4, end: 12, sourceVersion: 3, speaker: '苏远' }];
const asset: Asset = { id: 'test-asset', name: '林夏', kind: 'CH', baseAssetId: 'permanent-base', variantId: 'permanent-variant', version: 7, prompt: '图片提示词', sourceRefs, adoptedMediaId: 'image-1', candidates: ['image-old'], applicability: { episodeIds: ['episode-1'], sceneIds: ['scene-1'] } };
const scenes = [{ id: 'scene-1', episodeId: 'episode-1', title: '校园' }, { id: 'scene-2', episodeId: 'episode-1', title: '家中' }, { id: 'scene-3', episodeId: 'episode-2', title: '操场' }];

test('图片目录按真实集场筛选，场景限定与集数限定取交集，不把未知范围当全剧通用', () => {
  const source = { episodes: [{ id: 'episode-1' }, { id: 'episode-2' }], scenes };
  assert.equal(assetMatchesEpisode(asset, 'episode-1', source), true);
  assert.equal(assetMatchesEpisode(asset, 'episode-2', source), false);
  const global = { ...asset, applicability: {} };
  assert.equal(assetMatchesEpisode(global, 'episode-2', source), true);
  assert.equal(assetMatchesEpisode(global, 'unknown', source), false);
  assert.equal(assetMatchesEpisode({ ...global, applicability: { scenes: ['scene-3'] } }, 'episode-1', source), false);
  assert.equal(assetMatchesEpisode({ ...global, applicability: { episodes: ['episode-1'], scenes: ['scene-3'] } }, 'episode-1', source), false);
  assert.equal(assetMatchesEpisode({ ...global, scope: { kind: 'SCENES', scenes: [{ episodeId: 'episode-2', sceneId: 'scene-3' }] } }, 'episode-2', source), true);
  assert.equal(assetMatchesEpisode({ ...global, scope: { kind: 'SCENES', scenes: [{ episodeId: 'episode-1', sceneId: 'scene-3' }] } }, 'episode-1', source), false);
  assert.equal(assetMatchesEpisode({ ...global, scope: { kind: 'UNRESOLVED' } }, 'episode-1', source), false);
  assert.equal(assetMatchesEpisode({ ...global, scope: { kind: 'UNRESOLVED' } }, 'all', source), true);
});

test('编辑名称分类范围保留固定绑定身份、当前图、历史图和结构化原文引用', () => {
  const values = initialAssetEdit(asset);
  values.name = ' 苏远 · 觉醒装 '; values.kind = 'beast';
  const edited = assetEditPayload(asset, values, [], scenes);
  assert.equal(edited.name, '苏远 · 觉醒装'); assert.equal(edited.kind, 'beast');
  assert.equal(edited.id, asset.id); assert.equal(edited.baseAssetId, asset.baseAssetId); assert.equal(edited.variantId, asset.variantId);
  assert.equal(edited.expectedVersion, 7); assert.equal(edited.prompt, asset.prompt); assert.equal(edited.adoptedMediaId, 'image-1');
  assert.deepEqual(edited.candidates, ['image-old']); assert.deepEqual(edited.sourceRefs, sourceRefs);
  assert.deepEqual(edited.applicability, { episodes: ['episode-1'], scenes: ['scene-1'] });
  assert.equal(edited.needsReview, false); assert.equal(asset.name, '林夏');
});

test('修改来源绑定对应形象身份，清除来源时同步清除版本；失效旧来源不静默换绑', () => {
  const values = { ...initialAssetEdit(asset), sourceAssetId: 'parent' };
  assert.equal(assetEditPayload(asset, values, [{ id: 'parent', name: '基础形象', variantId: 'parent-v' }], scenes).sourceVariantId, 'parent-v');
  const missing: Asset = { ...asset, sourceAssetId: 'parent', sourceVariantId: 'old-parent-v' };
  assert.equal(assetEditPayload(missing, values, [], scenes).sourceVariantId, 'old-parent-v');
  const cleared = assetEditPayload(missing, { ...values, sourceAssetId: '' }, [], scenes);
  assert.equal(cleared.sourceAssetId, null); assert.equal(cleared.sourceVariantId, null);
});

test('删除操作必须绑定当前资产身份和版本；检查回来后版本漂移不能继续删', () => {
  const usage: AssetUsage = { assetId: asset.id, version: 7, name: asset.name, units: [], derivedAssets: [], activeJobs: [], submittedJobs: [], imageCount: 2 };
  assert.equal(assetDeletionReady(asset, null), false); assert.equal(assetDeletionReady(asset, usage), true);
  assert.equal(assetDeletionReady({ ...asset, version: 8 }, usage), false); assert.equal(assetDeletionReady(asset, { ...usage, assetId: 'other' }), false);
  assert.equal(assetDeletionReady(asset, { ...usage, deletedAt: '2026-10-04' }), false);
});

test('删除列表只显示真实删除标记；选择集场去重而不修改原数组', () => {
  assert.deepEqual(deletedAssetsFrom([{ ...asset, deletedAt: '2026-10-04' }, asset, null, { id: 'invalid', deletedAt: 'date' }]).map(item => item.id), [asset.id]);
  const ids = ['episode-1']; assert.deepEqual(toggleAssetScope(ids, 'episode-1', true), ['episode-1']);
  assert.deepEqual(toggleAssetScope(ids, 'episode-2', true), ['episode-1', 'episode-2']); assert.deepEqual(ids, ['episode-1']);
  assert.deepEqual(toggleAssetScope(ids, 'episode-1', false), []);
});

test('编辑原 UNRESOLVED 形象可明确 GLOBAL，程序提供理由，无原文测试资产也可确认', () => {
  const unresolved = { ...asset, scope: { kind: 'UNRESOLVED', scenes: [], reason: null } };
  const saved = assetEditPayload(unresolved, { ...initialAssetEdit(unresolved), episodeIds: [], sceneIds: [] }, []);
  assert.deepEqual(saved.scope, { kind: 'GLOBAL', scenes: [], reason: '用户在资产编辑中确认该形象全剧通用。' });
  assert.deepEqual(saved.applicability, { episodes: [], scenes: [] }); assert.equal(saved.needsReview, false); assert.deepEqual(saved.sourceRefs, sourceRefs);
});

test('只选集数展开所属全部场景，变更 SCENES 不沿用旧范围', () => {
  const saved = assetEditPayload({ ...asset, scope: { kind: 'SCENES', scenes: [{ episodeId: 'episode-2', sceneId: 'scene-3' }] } }, { ...initialAssetEdit(asset), sceneIds: [] }, [], scenes);
  assert.deepEqual(saved.scope, { kind: 'SCENES', scenes: [{ episodeId: 'episode-1', sceneId: 'scene-1' }, { episodeId: 'episode-1', sceneId: 'scene-2' }], reason: '用户在资产编辑中确认该形象适用于所选集场。' });
  assert.deepEqual(saved.applicability, { episodes: ['episode-1'], scenes: ['scene-1', 'scene-2'] });
});

test('场景必须属于所选集，未知场景和未识别场景集不能标记已核对', () => {
  const values = initialAssetEdit(asset);
  assert.deepEqual(assetScopeProblems(values, scenes), []);
  assert.throws(() => assetEditPayload(asset, { ...values, sceneIds: ['scene-3'] }, [], scenes), /不属于当前集数/);
  assert.throws(() => assetEditPayload(asset, { ...values, sceneIds: ['missing'] }, [], scenes), /引用已失效/);
  assert.throws(() => assetEditPayload(asset, { ...values, episodeIds: ['missing-episode'], sceneIds: [] }, [], scenes), /尚未识别场景/);
});

test('只有 SCENES 存储范围的导入资产仍显示原集场，打开编辑不误变全剧通用', () => {
  const values = initialAssetEdit({ ...asset, applicability: { episodes: [], scenes: [] }, scope: { kind: 'SCENES', scenes: [{ episodeId: 'episode-2', sceneId: 'scene-3' }] } });
  assert.deepEqual(values.episodeIds, ['episode-2']); assert.deepEqual(values.sceneIds, ['scene-3']);
  const saved = assetEditPayload(asset, values, [], scenes); assert.equal((saved.scope as { kind: string }).kind, 'SCENES');
});

test('仅 sourceVariantId 的编辑先按资产行身份解析，不猜显示名称', () => {
  const child = { ...asset, sourceVariantId: 'direct-row' };
  const exact: Asset = { id: 'direct-row', name: '不同显示名', baseAssetId: 'another-subject', variantId: 'actual-parent-variant' };
  const sameVariant: Asset = { id: 'other-row', name: child.name, baseAssetId: child.baseAssetId, variantId: 'direct-row' };
  const values = initialAssetEdit(child, [child, exact, sameVariant]);
  assert.equal(values.sourceAssetId, exact.id); assert.equal(values.sourceError, undefined);
  const saved = assetEditPayload(child, values, [child, exact, sameVariant], scenes);
  assert.equal(saved.sourceAssetId, exact.id); assert.equal(saved.sourceVariantId, exact.variantId);
});

test('直接父形象优先同主体 variantId，只有跨主体唯一时才可回显', () => {
  const child = { ...asset, sourceVariantId: 'clothes' };
  const parent: Asset = { id: 'clothes-row', name: '换装形象', baseAssetId: child.baseAssetId, variantId: 'clothes' };
  const unrelated: Asset = { id: 'other-clothes', name: child.name, baseAssetId: 'other-subject', variantId: 'clothes' };
  assert.equal(initialAssetEdit(child, [child, parent, unrelated]).sourceAssetId, parent.id);
  assert.equal(initialAssetEdit(child, [child, unrelated]).sourceAssetId, unrelated.id);
});

test('多层衍生编辑保留直接父形象，不回退到基础形象；显式 sourceAssetId 优先', () => {
  const base: Asset = { id: 'base-row', name: '基础形象', baseAssetId: asset.baseAssetId, variantId: 'base' };
  const clothes: Asset = { id: 'clothes-row', name: '换装形象', baseAssetId: asset.baseAssetId, variantId: 'clothes', sourceAssetId: base.id };
  const injury = { ...asset, sourceVariantId: clothes.variantId };
  const catalogue = [base, clothes, injury];
  const values = initialAssetEdit(injury, catalogue);
  assert.equal(values.sourceAssetId, clothes.id);
  assert.equal(assetEditPayload(injury, values, catalogue, scenes).sourceAssetId, clothes.id);
  const explicit = { ...injury, sourceAssetId: clothes.id, sourceVariantId: base.variantId };
  const explicitValues = initialAssetEdit(explicit, catalogue);
  assert.equal(explicitValues.sourceAssetId, clothes.id);
  assert.equal(assetEditPayload(explicit, explicitValues, catalogue, scenes).sourceVariantId, clothes.variantId);
});

test('父形象缺失或歧义保留非空错误项，禁止误保存独立；用户手动清空明确生效', () => {
  const child = { ...asset, sourceVariantId: 'clothes' };
  const parent: Asset = { id: 'parent-1', name: '相同名字', baseAssetId: child.baseAssetId, variantId: 'clothes' };
  for (const catalogue of [[], [parent, { ...parent, id: 'parent-2' }], [{ ...parent, baseAssetId: 'other-1' }, { ...parent, id: 'parent-2', baseAssetId: 'other-2' }]]) {
    const values = initialAssetEdit(child, catalogue);
    assert.equal(values.sourceAssetId, 'clothes'); assert.ok(values.sourceError);
    assert.throws(() => assetEditPayload(child, values, catalogue, scenes), /直接父形象/);
    const cleared = assetEditPayload(child, { ...values, sourceAssetId: '' }, catalogue, scenes);
    assert.equal(cleared.sourceAssetId, null); assert.equal(cleared.sourceVariantId, null);
    assert.equal(child.sourceVariantId, 'clothes');
  }
});

test('未载入目录不静默清除 variant-only 父形象；显式缺失父资产不回退其他 variant', () => {
  const child = { ...asset, sourceVariantId: 'clothes' };
  const unloaded = initialAssetEdit(child);
  assert.equal(unloaded.sourceAssetId, 'clothes'); assert.ok(unloaded.sourceError);
  assert.throws(() => assetEditPayload(child, unloaded, [], scenes), /尚未载入/);
  const unrelated: Asset = { id: 'valid-row', name: '同名', variantId: 'clothes' };
  const explicit = { ...child, sourceAssetId: 'missing-row' };
  const values = initialAssetEdit(explicit, [unrelated]);
  assert.equal(values.sourceAssetId, 'missing-row'); assert.ok(values.sourceError);
  const fixed = assetEditPayload(explicit, { ...values, sourceAssetId: unrelated.id }, [unrelated], scenes);
  assert.equal(fixed.sourceAssetId, unrelated.id); assert.equal(fixed.sourceVariantId, unrelated.variantId);
});

test('明确父资产已删除、自身引用或身份重复均回显错误，不另猜父形象', () => {
  const child = { ...asset, sourceAssetId: 'parent' };
  const parent: Asset = { id: 'parent', name: '基础', variantId: 'base' };
  assert.ok(initialAssetEdit(child, [{ ...parent, deletedAt: '2026-10-07' }]).sourceError);
  assert.ok(initialAssetEdit(child, [parent, parent]).sourceError);
  assert.ok(initialAssetEdit({ ...asset, sourceAssetId: asset.id }, [asset]).sourceError);
});
