import test from 'node:test';
import assert from 'node:assert/strict';
import { assetEpisodes, assetImageStatus, assetInEpisode, assetPromptStatus, fullDesignText, imageAssetRatio, imageExecutionAssets, imageProductionCounts, imagePromptText, imageReadyForGeneration, imageRecheckAvailable, imageReferenceIds, imageScopeAssets, imageWaitingNames, realAdoptedImage, usableAdoptedImage, sortAssetsByFirstUse } from './assetWorkflow.ts';
import { imageBatchEstimate } from './contracts.ts';
import { PRODUCTION_PAGES, projectOpeningPage, rememberProjectPage } from './workflowNavigation.ts';
import type { Asset, Workspace } from './types.ts';

function fixture(): Workspace {
  return { project: { id: 'project', title: '合成项目' }, assets: [], units: [], media: [], jobs: [], settings: { ratio: '9:16' }, providers: [], rules: [] };
}
test('stage routing retains stable IDs and remembers each project without persisting global settings as a work page', () => {
  assert.deepEqual(PRODUCTION_PAGES, ['foundation', 'storyboard', 'images', 'videos']);
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage'), saved = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: (key: string) => saved.get(key), setItem: (key: string, value: string) => saved.set(key, value) } });
  try {
    const workspace = fixture();
    workspace.lingmoDocuments = [{ id: 'md' } as any];
    assert.equal(projectOpeningPage(workspace), 'videos', 'existing MD-only projects keep their familiar fallback');
    rememberProjectPage('project', 'images');
    rememberProjectPage('project', 'settings');
    rememberProjectPage('other-project', 'storyboard');
    assert.equal(projectOpeningPage(workspace), 'images', 'prompt work must survive a project reopen');
    assert.equal(projectOpeningPage(workspace, 'videos'), 'videos', 'an explicit import destination wins');
    assert.equal(projectOpeningPage({ ...workspace, project: { id: 'other-project', title: '其他项目' } }), 'storyboard');
    saved.set('director.project-page.v1:project', 'obsolete-stage');
    assert.equal(projectOpeningPage(workspace), 'videos');
  } finally { if (descriptor) Object.defineProperty(globalThis, 'localStorage', descriptor); else Reflect.deleteProperty(globalThis, 'localStorage'); }
});
test('unavailable page storage cannot block opening or navigating a project', () => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw Error('Storage unavailable'); } });
  try { assert.doesNotThrow(() => rememberProjectPage('project', 'images')); assert.equal(projectOpeningPage(fixture()), 'foundation'); }
  finally { if (descriptor) Object.defineProperty(globalThis, 'localStorage', descriptor); else Reflect.deleteProperty(globalThis, 'localStorage'); }
});
test('image prompt text never substitutes or modifies adopted full design, and an explicitly cleared prompt stays empty', () => {
  const asset: Asset = { id: 'a', name: '人物', prompt: '旧手写词', adoptedDesign: { full: '完整服装结构、源事实和身份设计', revision: 7 }, imagePrompt: { text: '新的七视图图片提示词', status: 'ready' } };
  const before = structuredClone(asset);
  assert.equal(imagePromptText(asset), '新的七视图图片提示词');
  assert.equal(fullDesignText(asset), '完整服装结构、源事实和身份设计');
  assert.equal(imagePromptText({ ...asset, imagePrompt: { text: '' } }), '');
  assert.equal(imagePromptText({ ...asset, imagePrompt: undefined }), '旧手写词');
  assert.deepEqual(asset, before);
});
test('identity and design parents do not become pixel dependencies; only explicit required references wait', () => {
  const workspace = fixture(), parent: Asset = { id: 'parent', name: '身份母版', kind: 'CH', variantId: 'base' };
  const child: Asset = { id: 'child', name: '换装', sourceAssetId: 'parent', designDependencies: [{ assetId: 'parent' }], imagePrompt: { text: '已写齐换装词' } };
  workspace.assets = [parent, child];
  assert.deepEqual(imageReferenceIds(child), []);
  assert.deepEqual(imageWaitingNames(workspace, child), []);
  child.referenceRequirements = [{ targetAssetId: 'parent', role: 'face' }, { targetAssetId: 'optional', required: false }];
  assert.deepEqual(imageWaitingNames(workspace, child), ['身份母版']);
  assert.equal(assetPromptStatus(child), '提示词已编写');
  assert.equal(assetImageStatus(workspace, child), '等待参考');
  workspace.media = [{ id: 'candidate', kind: 'image', path: 'synthetic.png', assetId: 'parent', variantId: 'base' }];
  assert.deepEqual(imageWaitingNames(workspace, child), ['身份母版'], 'a downloaded candidate does not satisfy adoption');
  parent.adoptedMediaId = 'candidate';
  assert.deepEqual(imageWaitingNames(workspace, child), []);
  for (const imageStatus of ['needs_review', 'stale', 'rejected', 'invalid']) {
    parent.imageStatus = imageStatus;
    assert.deepEqual(imageWaitingNames(workspace, child), ['身份母版'], imageStatus + ' cannot satisfy an actual media dependency');
  }
});
test('formal scope, prompt completion, candidate coverage and adoption have separate counts', () => {
  const workspace = fixture();
  workspace.assets = [
    { id: 'formal', name: '正式资产', kind: 'CH', prompt: '已完成词', variantId: 'v' },
    { id: 'empty', name: '待编写', kind: 'SC', prompt: '' },
    { id: 'reference', name: '参考母版', kind: 'CH', prompt: '参考专用', adoptedMediaId: 'ref-image' },
    { id: 'voice', name: '系统声音', preparation: { mode: 'AUDIO' }, prompt: '纯声音' },
  ];
  workspace.imageProductionScope = { assetIds: ['formal', 'empty', 'voice'], referenceOnlyIds: ['reference'] };
  workspace.media = [{ id: 'candidate', kind: 'image', path: 'candidate.png', assetId: 'formal', variantId: 'v' }];
  assert.deepEqual(imageScopeAssets(workspace).map(asset => asset.id), ['formal', 'empty']);
  assert.deepEqual(imageProductionCounts(workspace), { total: 2, prompts: 1, adopted: 0, candidates: 1, waiting: 0, needsReview: 0, invalid: 0 });
  assert.equal(assetImageStatus(workspace, workspace.assets[0]), '候选待选');
});

test('costume waits for the exact adopted mother crop while unrelated ready images continue', () => {
  const workspace = fixture(), sha = 'a'.repeat(64);
  const mother: Asset = { id: 'mother', name: '身份母版', kind: 'CH', variantId: 'base', adoptedMediaId: 'm' };
  const child: Asset = { id: 'costume', name: '换装', kind: 'CH', imagePrompt: { text: '换装词' }, imagePresentation: { layoutKind: 'costume' }, referenceRequirements: [{ targetAssetId: 'mother', role: 'identity' }] };
  const independent: Asset = { id: 'scene', name: '独立场景', kind: 'SC', imagePrompt: { text: '场景词' } };
  workspace.assets = [mother, child, independent];
  workspace.media = [{ id: 'm', assetId: 'mother', variantId: 'base', kind: 'image', path: 'synthetic.png', sha256: sha }];
  workspace.imageProductionScope = { assetIds: ['costume', 'scene'] };
  const ready = () => imageScopeAssets(workspace).filter(asset => imagePromptText(asset).trim() && !imageWaitingNames(workspace, asset).length).map(asset => asset.id);
  assert.deepEqual(imageWaitingNames(workspace, child), ['身份母版（锁脸区域待登记）']);
  assert.equal(assetImageStatus(workspace, child), '锁脸区域待登记');
  assert.deepEqual(ready(), ['scene']);
  mother.identityLayout = { version: 'asset-identity-layout/v1', basis: 'user-inspected', confirmed: true, mediaId: 'm', sha256: sha, width: 1024, height: 1024, identityStrip: { x: 0, y: 0, width: 1024, height: 400 } };
  assert.deepEqual(imageWaitingNames(workspace, child), []);
  assert.deepEqual(ready(), ['costume', 'scene']);
  workspace.media[0].sha256 = 'b'.repeat(64);
  assert.deepEqual(ready(), ['scene'], 'a replaced mother cannot reuse an old crop confirmation');
  for (const [status, label] of [['submitted', '生成中'], ['submission_unknown', '原结果待核对'], ['candidate', '候选待选']]) {
    child.imageStatus = status;
    assert.equal(assetImageStatus(workspace, child), label, 'future-send requirements do not replace an existing request or result status');
  }
  child.imageStatus = 'waiting_reference';
  assert.equal(assetImageStatus(workspace, child), '锁脸区域待登记');
});
test('asset ratios remain independent from project video ratio and preserve a specific asset override', () => {
  const workspace = fixture();
  assert.equal(workspace.settings.ratio, '9:16');
  assert.equal(imageAssetRatio({ id: 'person', name: '甲', kind: 'CH' }), '1:1');
  assert.equal(imageAssetRatio({ id: 'scene', name: '院落', kind: 'SC' }), '16:9');
  assert.equal(imageAssetRatio({ id: 'group', name: '群演', kind: 'CH', imagePrompt: { ratio: '3:2' } }), '3:2');
  assert.equal(workspace.settings.ratio, '9:16');
});

test('unresolved formal image assets remain in completion denominators and display the classification issue', () => {
  const workspace = fixture();
  workspace.assets = [{ id: 'unresolved', name: '待核系统载体', preparation: { mode: 'UNRESOLVED' } }, { id: 'voice', name: '纯声音', preparation: { mode: 'AUDIO' } }];
  workspace.imageProductionScope = { assetIds: ['unresolved', 'voice'] };
  assert.equal(imageProductionCounts(workspace).total, 1);
  assert.deepEqual(imageScopeAssets(workspace).map(asset => asset.id), ['unresolved']);
  assert.equal(assetImageStatus(workspace, workspace.assets[0]), '类别待核对');
});
test('MD-only episode filtering uses explicit scene asset identities, never matching names', () => {
  const workspace = fixture();
  workspace.assets = [{ id: 'a', name: '甲' }, { id: 'b', name: '甲' }];
  workspace.lingmoDocuments = [{ id: 'md', episodes: [{ id: 'e150', number: '150', sceneIds: ['s'] }], scenes: [{ id: 's', episodeId: 'e150', number: '150-1', unitIds: [], assetIds: ['b'] }] } as any];
  assert.deepEqual(assetEpisodes(workspace), [{ id: 'e150', title: '第 150 集' }]);
  assert.equal(assetInEpisode(workspace, workspace.assets[0], 'e150'), false);
  assert.equal(assetInEpisode(workspace, workspace.assets[1], 'e150'), true);
});

test('modern MD imageBindings exclude spoken and mentioned assets even when scene declarations include them', () => {
  const workspace = fixture();
  workspace.assets = [{ id: 'visible', name: '入画人物' }, { id: 'voice', name: '只闻其声' }];
  workspace.lingmoDocuments = [{ id: 'md', episodes: [{ id: 'e', number: 1, sceneIds: ['s'] }], scenes: [{ id: 's', episodeId: 'e', unitIds: ['u'], assetIds: ['visible', 'voice'] }] } as any];
  workspace.units = [{ id: 'u', episodeId: 'e', prompt: '原稿', imageBindings: [{ assetId: 'visible' }], mentions: [{ assetId: 'voice' }], voiceBindings: [{ assetId: 'voice' }] }];
  assert.equal(assetInEpisode(workspace, workspace.assets[0], 'e'), true);
  assert.equal(assetInEpisode(workspace, workspace.assets[1], 'e'), false);
  workspace.units[0].imageBindings = [];
  assert.equal(assetInEpisode(workspace, workspace.assets[0], 'e'), false, 'an explicit empty image list cannot fall back to broader declarations');
});
test('first-use ordering is stable and never mutates the registry array', () => {
  const workspace = fixture();
  workspace.units = [{ id: 'u1', prompt: '' }, { id: 'u2', prompt: '' }];
  workspace.assets = [{ id: 'later', name: '后用', uses: [{ unitId: 'u2' }] }, { id: 'first', name: '先用', uses: [{ unitId: 'u1' }] }, { id: 'same', name: '同片段', uses: [{ unitId: 'u1' }] }];
  assert.deepEqual(sortAssetsByFirstUse(workspace, workspace.assets).map(asset => asset.id), ['first', 'same', 'later']);
  assert.equal(workspace.assets[0].id, 'later');
});
test('new image cost preview reports only explicit media requirements without treating a family parent as an extra generation', () => {
  const workspace = fixture();
  workspace.assets = [{ id: 'root', name: '身份母版' }, { id: 'child', name: '后续状态', sourceAssetId: 'root', imagePrompt: { text: '词' } }];
  assert.deepEqual(imageBatchEstimate(workspace, ['child'], 'chrome', undefined, { strictReferences: true }).dependencies, []);
  workspace.assets[1].referenceRequirements = [{ targetAssetId: 'root', kind: 'face' }];
  const estimate = imageBatchEstimate(workspace, ['child'], 'chrome', undefined, { strictReferences: true });
  assert.equal(estimate.items.length, 1);
  assert.deepEqual(estimate.dependencies, [{ assetId: 'root', name: '身份母版' }]);
});

test('production union includes only formal and explicitly registered references without changing formal totals', () => {
  const workspace = fixture();
  workspace.assets = [{ id: 'child', name: '正式', prompt: '词', referenceRequirements: [{ targetAssetId: 'mother' }] }, { id: 'mother', name: '参考', prompt: '母词', sourceAssetId: 'ancestor' }, { id: 'ancestor', name: '范围外祖先' }];
  workspace.imageProductionScope = { assetIds: ['child'], referenceOnlyIds: ['mother', 'child'] };
  assert.deepEqual(imageExecutionAssets(workspace).map(asset => asset.id), ['child', 'mother']);
  assert.deepEqual(imageScopeAssets(workspace).map(asset => asset.id), ['child']);
  assert.equal(imageProductionCounts(workspace).total, 1);
  workspace.imageProductionScope.referenceOnlyIds = [];
  assert.deepEqual(imageExecutionAssets(workspace).map(asset => asset.id), ['child']);
});

test('adopted placeholder pixels do not count as finished images or satisfy child references', () => {
  const workspace = fixture();
  workspace.assets = [{ id: 'mother', name: '母图', kind: 'CH', adoptedMediaId: 'white', imageStatus: 'adopted', prompt: '母词' }, { id: 'child', name: '子图', prompt: '子词', referenceRequirements: [{ targetAssetId: 'mother' }] }];
  workspace.media = [{ id: 'white', assetId: 'mother', kind: 'image', path: 'white.png', placeholder: true }];
  assert.equal(assetImageStatus(workspace, workspace.assets[0]), '占位待补图');
  assert.equal(imageProductionCounts(workspace).adopted, 0);
  assert.equal(imageProductionCounts(workspace).candidates, 0);
  assert.deepEqual(imageWaitingNames(workspace, workspace.assets[1]), ['母图']);
  assert.equal(workspace.media.length, 1, 'the imported placeholder remains saved and viewable');
});

test('retained review and invalid images stay viewable without inflating usable adoption counts', () => {
  const workspace = fixture();
  const statuses = ['adopted', 'needs_review', 'stale', 'rejected', 'invalid', 'needs_repair'];
  workspace.assets = statuses.map((imageStatus, i) => ({ id: `a${i}`, name: imageStatus, kind: 'CH', variantId: 'v', prompt: '保留文字', adoptedMediaId: `m${i}`, imageStatus }));
  workspace.media = statuses.map((_, i) => ({ id: `m${i}`, assetId: `a${i}`, variantId: 'v', kind: 'image', path: `retained-${i}.png` }));
  const before = structuredClone(workspace);
  assert.deepEqual(imageProductionCounts(workspace), { total: 6, prompts: 6, adopted: 1, candidates: 6, waiting: 0, needsReview: 2, invalid: 3 });
  workspace.assets.forEach((asset, i) => {
    assert.equal(realAdoptedImage(workspace, asset)?.id, `m${i}`, 'old media remains viewable');
    assert.equal(usableAdoptedImage(workspace, asset)?.id, i ? undefined : 'm0');
  });
  assert.equal(assetImageStatus(workspace, workspace.assets[3]), '待修');
  assert.equal(assetImageStatus(workspace, workspace.assets[4]), '图片无效');
  assert.deepEqual(workspace, before, 'counting never rewrites an adoption or old media');
});

test('ready selection rebuilds only known media drift with ready mothers and preserves review barriers', () => {
  const workspace = fixture();
  const mother: Asset = { id: 'mother', name: '母版', kind: 'CH', variantId: 'v', identityId: 'person', adoptedMediaId: 'mother-image' };
  const child: Asset = { id: 'child', name: '待复核子图', kind: 'CH', variantId: 'v', identityId: 'person', imagePrompt: { text: '已写齐目标', status: 'ready' }, adoptedMediaId: 'old-child', imageStatus: 'needs_review', referenceRequirements: [{ targetAssetId: 'mother', targetVariantId: 'v', role: 'appearance' }], imageReviewReasons: [{ kind: 'reference-media-changed', assetId: 'mother', frozenMediaId: 'old-mother' }] };
  const independent: Asset = { id: 'independent', name: '独立场景', kind: 'SC', prompt: '已就绪' };
  workspace.assets = [mother, child, independent];
  workspace.media = [{ id: 'mother-image', assetId: 'mother', variantId: 'v', kind: 'image', path: 'mother.png' }, { id: 'old-child', assetId: 'child', variantId: 'v', kind: 'image', path: 'old-child.png' }];
  const before = structuredClone(workspace);
  assert.equal(imageReadyForGeneration(workspace, child), true, 'known media-only drift can rebuild against the current real mother');
  mother.imageStatus = 'needs_review';
  assert.equal(imageReadyForGeneration(workspace, child), false);
  assert.equal(imageReadyForGeneration(workspace, independent), true, 'one branch does not block unrelated work');
  delete mother.imageStatus;
  for (const patch of [
    { imageReviewReasons: [] }, { imageReviewReasons: [{ kind: 'own-design-changed', assetId: 'child' }] },
    { imageStatus: 'rejected' }, { imageStatus: 'invalid' }, { imageStatus: 'needs_repair' },
    { designStatus: 'needs_review' }, { promptStatus: 'ready', imagePrompt: { text: '旧正文', status: 'needs_review' } },
    { mediaPlan: { status: 'needs_review' } }, { imageStatus: 'submission_unknown' },
  ]) assert.equal(imageReadyForGeneration(workspace, { ...child, ...patch }), false, JSON.stringify(patch));
  workspace.jobs = [{ id: 'unknown', kind: 'image', assetId: 'child', provider: 'chrome', status: 'submission_unknown' }];
  assert.equal(imageReadyForGeneration(workspace, child), false, 'never re-send unknown work');
  workspace.jobs = [];
  assert.deepEqual(workspace, before, 'readiness leaves old images and adopted facts intact');
});

test('ready selection rejects unreviewed candidates, stale variants, cross-identity faces and dependency cycles', () => {
  const workspace = fixture();
  const mother: Asset = { id: 'mother', name: '母版', variantId: 'v', identityId: 'person', adoptedMediaId: 'm' };
  const child: Asset = { id: 'child', name: '子图', variantId: 'v', identityId: 'person', prompt: '提示词', referenceRequirements: [{ targetAssetId: 'mother', targetVariantId: 'v', role: 'identity' }] };
  workspace.assets = [mother, child]; workspace.media = [{ id: 'm', kind: 'image', assetId: 'mother', variantId: 'v', path: 'mother.png' }];
  assert.equal(imageReadyForGeneration(workspace, child), true);
  workspace.media.push({ id: 'candidate', kind: 'image', assetId: 'child', variantId: 'v', path: 'candidate.png' });
  assert.equal(imageReadyForGeneration(workspace, child), false, 'a returned candidate should be inspected instead of regenerated in bulk');
  workspace.media.pop();
  mother.variantId = 'changed'; workspace.media[0].variantId = 'changed';
  assert.equal(imageReadyForGeneration(workspace, child), false, 'ready media cannot override a frozen target variant');
  mother.variantId = 'v'; workspace.media[0].variantId = 'v'; mother.identityId = 'different-person';
  assert.equal(imageReadyForGeneration(workspace, child), false);
  mother.identityId = 'person'; mother.referenceRequirements = [{ targetAssetId: 'child' }];
  assert.equal(imageReadyForGeneration(workspace, child), false, 'adopted parents do not hide a cyclic future media graph');
});


test('current-image recheck is limited to retained modern media dependency invalidation', () => {
  const workspace = fixture();
  const child: Asset = { id: 'child', name: '子形象', kind: 'SC', variantId: 'base', workflowProfile: 'desktop-series672-assets112/v1', imageStatus: 'needs_review', adoptedMediaId: 'old-child', imageReviewReasons: [{ kind: 'reference-media-awaits-review', assetId: 'mother' }] };
  workspace.assets = [child];
  workspace.media = [{ id: 'old-child', assetId: 'child', variantId: 'base', kind: 'image', path: 'child.png' }];
  assert.equal(imageRecheckAvailable(workspace, child), true, 'parent may still need review: backend reports the original reference mismatch');
  const baseline = structuredClone(child);
  for (const patch of [{ imageStatus: 'adopted' }, { imageStatus: 'needs_repair' }, { imageStatus: 'unknown' }, { imageReviewReasons: [] }, { imageReviewReasons: [{ kind: 'own-design-changed' }] }, { imageReviewReasons: [{ kind: 'reference-media-changed' }, { kind: 'unverified-identity' }] }, { imagePrompt: { status: 'needs_review' } }, { designStatus: 'rejected' }, { workflowProfile: 'legacy' }]) {
    assert.equal(imageRecheckAvailable(workspace, { ...baseline, ...patch } as Asset), false, JSON.stringify(patch));
  }
  workspace.media[0].placeholder = true; assert.equal(imageRecheckAvailable(workspace, child), false);
  workspace.media[0].placeholder = false;
  workspace.jobs = [{ id: 'unknown', kind: 'image', assetId: 'child', provider: 'dreamina', status: 'submission_unknown' }];
  assert.equal(imageRecheckAvailable(workspace, child), false);
});
