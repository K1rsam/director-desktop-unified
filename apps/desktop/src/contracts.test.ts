import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { assetImages, assetImageSelection, createCatalogDiscovery, frozenReference, generationChoices, imageOutputLabel, imageOutputValueLabel, imageBatchBlocked, imageBatchEstimate, imageDesignBusy, imageDesignJob, initializeImageParameters, mediaMatchesAsset, missingCatalogs, moveItem, normalizeWorkspace, packageEstimate, packageReady, ratioLabel, referenceTimingProblems, resolutionLabel, selectedUnitsForScope, storyboardRange, toggleUnitSelection, unitResultIds, unitsForScope, videoAdoptionProblems, videoContainerVerified, workflowResult } from './contracts.ts';
import type { PreparedPackage, SourcePromptProvenance, Unit, Workspace } from './types.ts';

function fixture(): Workspace { return { project: { id: 'p', title: '项目' }, assets: [{ id: 'a', name: '形象', variantId: 'v', adoptedMediaId: 'image', candidates: ['shared'] }], media: [{ id: 'image', kind: 'image', path: 'a.png', assetId: 'a', variantId: 'v' }, { id: 'shared', kind: 'image', path: 'b.png', assetId: 'other', variantId: 'other' }, { id: 'old-video', kind: 'video', path: 'old.mp4' }], units: [{ id: 'u', prompt: '完整提示词', duration: 12, version: 2, assetRefs: [{ assetId: 'a', variantId: 'v', use: 'scene', role: '', order: 1 }], preparedPackage: { provider: 'bh', model: 'm', unitVersion: 2, ratio: '16:9', resolution: '720p', duration: 12, prompt: '完整提示词', estimatedCost: 0, references: [{ assetId: 'a', variantId: 'v', role: 'scene', order: 0, mediaId: 'image' }] }, resultMediaIds: ['new-video'], adoptedMediaId: 'old-video' }], jobs: [{ id: 'j', kind: 'video', entityId: 'u', provider: 'bh', status: 'downloaded', snapshot: { model: 'm', unitVersion: 2 }, prepared: { estimatedCost: 0.25 }, result: { mediaIds: ['new-video'], actualCost: 0.2 } }, { id: 'design', kind: 'workflow', entityId: 'images.design:', provider: 'local', status: 'completed', result: { blocked: [{ reason: '来源缺失' }], imagePlan: [{ status: 'awaitingTrial' }] } }], settings: {}, providers: [], rules: [] }; }

test('Rust任务摘要投影保留云端结果、身份与费用', () => { const workspace = normalizeWorkspace(fixture()); const job = workspace.jobs[0]; assert.equal(job.unitId, 'u'); assert.equal(job.model, 'm'); assert.deepEqual(job.resultMediaIds, ['new-video']); assert.equal(job.estimate, 0.25); assert.equal(job.actualCost, 0.2); assert.equal(workspace.units[0].assetRefs?.[0].role, 'scene'); assert.equal(workspace.media[0].adopted, true); });
test('准备包精确匹配渠道模型、版本、比例尺寸、时长与编译提示词，并保留零费用', () => { const unit = fixture().units[0]; assert.equal(packageEstimate(unit), 0); assert.equal(packageReady(unit, 'bh', 'm'), true); assert.equal(packageReady(unit, 'dreamina', 'm'), false); assert.equal(packageReady(unit, 'bh', 'wrong'), false); assert.equal(packageReady({ ...unit, version: 3 }, 'bh', 'm'), false); assert.equal(packageReady({ ...unit, problems: ['缺图'] }, 'bh', 'm'), false); assert.equal(packageReady(unit, 'bh', 'm', { ratio: '16:9', resolution: '720p' }), true); assert.equal(packageReady(unit, 'bh', 'm', { ratio: '9:16', resolution: '720p' }), false); assert.equal(packageReady(unit, 'bh', 'm', { ratio: '16:9', resolution: '1080p' }), false); assert.equal(packageReady({ ...unit, duration: 15 }, 'bh', 'm'), false); assert.equal(packageReady({ ...unit, prompt: '已经修改' }, 'bh', 'm'), false); assert.equal(packageReady({ ...unit, preparedPackage: { ...unit.preparedPackage!, prompt: undefined } }, 'bh', 'm'), false); assert.equal(packageReady({ ...unit, prompt: '使用{{asset:a:v}}', preparedPackage: { ...unit.preparedPackage!, prompt: '使用@图片1' } }, 'bh', 'm'), true); });

function compiledPackageUnit(): Unit {
  const unit = fixture().units[0], sourcePrompt = '人物甲推门，门牌显示“继续”。\n[参考素材] 人物甲：character参考；人物甲伤态：character参考；门牌：prop参考\n';
  const prompt = `${sourcePrompt}\n[本次已绑定参考素材顺序]\n@图片1：人物甲（character参考）\n@图片1：人物甲伤态（character参考）\n【文字素材：门牌；字样“继续”；贴合门牌表面】：门牌（prop参考）\n`;
  const sourcePromptProvenance: SourcePromptProvenance = { version: 'unit-source-prompt/v1', jobId: 'prepared', unitId: unit.id, unitVersion: unit.version!, sourceVersion: 3, compiledPromptVerified: true, compiledPrompt: prompt, sourcePromptSha256: createHash('sha256').update(sourcePrompt).digest('hex'), origin: 'unit-version-and-reference-compilation' };
  return { ...unit, sourceVersion: 3, storyboardCreativeProfile: 'original-production-contract/v1', prompt: sourcePrompt, assetRefs: [{ assetId: 'a', variantId: 'v', role: 'character', order: 0 }, { assetId: 'hurt', variantId: 'hurt', role: 'character', order: 1 }, { assetId: 'label', variantId: 'text', role: 'prop', order: 2 }], preparedPackage: { ...unit.preparedPackage!, jobId: 'prepared', prompt, sourcePrompt, sourcePromptProvenance, references: [{ assetId: 'parent', variantId: 'base', mediaId: 'image', kind: 'image', role: 'character', order: 0, semanticBindings: [{ assetId: 'a', variantId: 'v' }, { assetId: 'hurt', variantId: 'hurt' }] }], inlineReferences: [{ assetId: 'label', variantId: 'text', kind: 'inline-text', name: '门牌', description: '字样“继续”', preparation: { mode: 'INLINE', note: '贴合门牌表面' } }] } };
}

test('核心原提示词证明支持人名模板、INLINE文字与祖先共图，同时不放开冻结参数', () => {
  const unit = compiledPackageUnit();
  assert.notEqual(unit.prompt, unit.preparedPackage!.prompt);
  assert.equal(packageReady(unit, 'bh', 'm', { ratio: '16:9', resolution: '720p' }), true);
  assert.equal(packageReady(unit, 'dreamina', 'm'), false);
  assert.equal(packageReady(unit, 'bh', 'other'), false);
  assert.equal(packageReady(unit, 'bh', 'm', { ratio: '9:16', resolution: '720p' }), false);
  assert.equal(packageReady(unit, 'bh', 'm', { ratio: '16:9', resolution: '1080p' }), false);
  for (const changed of [{ version: 3 }, { sourceVersion: 4 }, { sourceVersion: undefined }, { duration: 15 }, { prompt: unit.prompt + ' ' }, { problems: ['缺图'] }]) assert.equal(packageReady({ ...unit, ...changed }, 'bh', 'm'), false);
});

test('原提示词证明逐字绑定冻结文本与任务身份，缺失或伪证明不能回退放行', () => {
  const unit = compiledPackageUnit(), pkg = unit.preparedPackage!, proof = pkg.sourcePromptProvenance!;
  const withPackage = (change: Partial<PreparedPackage>) => ({ ...unit, preparedPackage: { ...pkg, ...change } });
  for (const change of [{ jobId: 'other-job' }, { sourcePrompt: unit.prompt + '\n' }, { prompt: pkg.prompt + ' ' }, { sourcePromptProvenance: undefined }, { sourcePrompt: undefined }, { sourcePrompt: undefined, sourcePromptProvenance: undefined }]) assert.equal(packageReady(withPackage(change), 'bh', 'm'), false);
  for (const change of [{ jobId: 'other-job' }, { unitId: 'other-unit' }, { unitVersion: 3 }, { sourceVersion: 2 }, { compiledPromptVerified: false }, { compiledPrompt: unit.prompt }, { sourcePromptSha256: '' }, { version: 'unverified/v1' }, { origin: 'current-unit-version-only' }]) assert.equal(packageReady(withPackage({ sourcePromptProvenance: { ...proof, ...change } as SourcePromptProvenance }), 'bh', 'm'), false, JSON.stringify(change));
  const partialPlain = { ...fixture().units[0], sourceVersion: 3, preparedPackage: { ...fixture().units[0].preparedPackage!, sourcePrompt: '完整提示词' } };
  assert.equal(packageReady(partialPlain, 'bh', 'm'), false, 'a present but incomplete proof cannot use the matching legacy text fallback');
});

test('所有集数就绪数接受11个核心编译证明而保留8个真实阻塞，不使用额外UI编译猜测', () => {
  const ready = compiledPackageUnit();
  const units = [...Array.from({ length: 11 }, () => structuredClone(ready)), ...Array.from({ length: 8 }, () => ({ ...structuredClone(ready), problems: ['原文版本需要更新'] }))].map((unit, index) => {
    unit.id = `unit-${index}`; unit.preparedPackage!.sourcePromptProvenance!.unitId = unit.id; return unit;
  });
  assert.equal(selectedUnitsForScope(units, 'all', null).length, 19);
  assert.equal(selectedUnitsForScope(units, 'all', null).filter(unit => packageReady(unit, 'bh', 'm', { ratio: '16:9', resolution: '720p' })).length, 11);
  assert.ok(units.slice(11).every(unit => !packageReady(unit, 'bh', 'm')));
});
test('冻结配图不依赖UI一基序号，视频保留旧采用与新候选', () => { const workspace = normalizeWorkspace(fixture()), unit = workspace.units[0]; assert.equal(frozenReference(unit.preparedPackage, unit.assetRefs![0]), 'image'); assert.deepEqual(unitResultIds(workspace, unit).sort(), ['new-video', 'old-video']); });
test('共用图片、设计阻塞与计划可读取，未知参考时长必须阻止提交', () => { const workspace = fixture(); assert.equal(mediaMatchesAsset(workspace.assets[0], workspace.media[1]), true); assert.equal(workflowResult(workspace, 'images.design')?.blocked instanceof Array, true); const unit = { ...workspace.units[0], mediaRefs: [{ mediaId: 'sound', role: 'reference' }] }; assert.equal(referenceTimingProblems(workspace, unit).length, 1); workspace.media.push({ id: 'sound', kind: 'audio', path: 'sound.wav', durationSeconds: 8, metadataVerified: false }); assert.equal(referenceTimingProblems(workspace, unit).length, 1); workspace.media.at(-1)!.metadataVerified = true; assert.equal(referenceTimingProblems(workspace, unit).length, 0); workspace.media.at(-1)!.durationSeconds = 0; assert.equal(referenceTimingProblems(workspace, unit).length, 1); });

test('引用重排改变提交顺序并保留完整身份字段', () => { const refs = [{ mediaId: 'a', role: 'character' }, { mediaId: 'b', role: 'sound' }, { mediaId: 'c', role: 'video' }]; const moved = moveItem(refs, 2, 0); assert.deepEqual(moved.map(item => item.mediaId), ['c', 'a', 'b']); assert.equal(moved[0].role, 'video'); assert.deepEqual(refs.map(item => item.mediaId), ['a', 'b', 'c']); assert.equal(moveItem(refs, -1, 0), refs); });

test('图片计划使用最新相关任务而不会被旧设计结果遮蔽', () => { const workspace = fixture(); workspace.jobs.unshift({ id: 'produce', kind: 'workflow', entityId: 'images.produce:', provider: 'local', status: 'completed', result: { imagePlan: [{ status: 'ready' }] } }); assert.deepEqual(workflowResult(workspace, ['images.design', 'images.produce'])?.imagePlan, [{ status: 'ready' }]); });

test('分镜与制作基础文字任务不阻塞图片设计，相关图片流程保留执行所有权', () => {
  const job = { id: 'workflow', kind: 'workflow', entityId: 'storyboard.run', provider: 'local', status: 'running' };
  const unrelated = [job, { ...job, id: 'foundation', entityId: 'foundation.run' }, { ...job, id: 'author', kind: 'text', entityId: 'storyboardCreate:unit-performance-director:request' }];
  assert.equal(imageDesignBusy(unrelated), false);
  for (const entityId of ['images.design:', 'images.produce:']) {
    assert.equal(imageDesignBusy([...unrelated, { ...job, entityId }]), true);
    assert.equal(imageDesignBusy([{ ...job, entityId, status: 'completed' }]), false);
    assert.equal(imageDesignBusy([{ ...job, entityId, status: 'submission_unknown' }]), true);
  }
  for (const entityId of ['assetDesign:asset-art-director:request', 'assetVisualFacts:asset-art-director:request']) assert.equal(imageDesignBusy([{ ...job, kind: 'text', entityId }]), true);
  assert.equal(imageDesignBusy([{ ...job, entityId: 'other', snapshot: { stage: 'images.produce' } }]), true);
  assert.equal(imageDesignJob({ ...job, entityId: 'images.production-other:' }), false);
  assert.equal(imageDesignJob({ ...job, kind: 'text', entityId: 'assetDesignOther:request' }), false);
});

test('图片一键制作保留同一资产未决提交阻塞，其他资产及已终态任务互不遮蔽', () => {
  const job = { id: 'image', kind: 'image', assetId: 'a', provider: 'dreamina', status: 'submission_unknown' };
  assert.equal(imageBatchBlocked([job], ['a']), true);
  assert.equal(imageBatchBlocked([{ ...job, status: 'downloaded', submissionUnknown: true }], ['a']), true);
  assert.equal(imageBatchBlocked([job], ['other']), false);
  assert.equal(imageBatchBlocked([job], ['a', 'other']), false, 'unblocked items may proceed; the core still rejects each unresolved asset');
  for (const status of ['queued', 'waiting_dependency', 'query_failed', 'download_failed', 'recovery_paused']) assert.equal(imageBatchBlocked([{ ...job, status }], ['a']), true);
  for (const status of ['downloaded', 'failed', 'closed_by_user']) assert.equal(imageBatchBlocked([{ ...job, status }], ['a']), false);
  assert.equal(imageBatchBlocked([{ ...job, assetId: undefined, entityId: 'a' }], ['a']), true);
  assert.equal(imageBatchBlocked([{ ...job, kind: 'video' }], ['a']), false);
  assert.equal(imageBatchBlocked([job], []), false);
});

test('图片只采用已适配单图报价，未知计价与缺图依赖不会伪造总费用', () => {
  const workspace = fixture(); workspace.assets.push({ id: 'parent', name: '基础形象', variantId: 'base', prompt: '基础设计' }); workspace.assets[0].sourceAssetId = 'parent';
  const model = { id: 'image-m', kind: 'image', adapted: true, price: 0.25, billing: 'per_image', currency: 'CNY' };
  const estimate = imageBatchEstimate(workspace, ['a', 'a'], 'bh', model);
  assert.equal(estimate.items.length, 1); assert.equal(estimate.items[0].count, 1); assert.equal(estimate.items[0].amount, 0.25); assert.equal(estimate.selectedTotal, 0.25); assert.deepEqual(estimate.dependencies.map(item => item.assetId), ['parent']);
  for (const provider of ['chrome', 'dreamina']) assert.equal(imageBatchEstimate(workspace, ['a'], provider, model).selectedTotal, undefined);
  for (const invalid of [{ ...model, kind: 'video' }, { ...model, adapted: false }, { ...model, billing: 'per_second' }, { ...model, currency: undefined }, { ...model, price: NaN }, { ...model, price: -1 }]) assert.equal(imageBatchEstimate(workspace, ['a'], 'bh', invalid).selectedTotal, undefined);
  assert.equal(imageBatchEstimate(workspace, ['a'], 'bh', { ...model, price: 0, billing: 'per_call' }).selectedTotal, 0);
  workspace.assets.at(-1)!.adoptedMediaId = 'parent-image'; assert.equal(imageBatchEstimate(workspace, ['a'], 'bh', model).dependencies.length, 0);
  workspace.assets[0].sourceAssetId = 'missing'; assert.equal(imageBatchEstimate(workspace, ['a'], 'bh', model).unresolved, true);
});

test('视频采用要求可信运动视频容器与当前生成版本，历史成品仍可预览', () => {
  const workspace = normalizeWorkspace(fixture()), unit = workspace.units[0];
  const media = { id: 'new-video', kind: 'video', path: 'new.mp4', metadataVerified: true, streams: [{ kind: 'video', attachedPicture: false }] };
  assert.equal(videoContainerVerified(media), true); assert.deepEqual(videoAdoptionProblems(workspace, unit, media), []);
  assert.equal(videoContainerVerified({ ...media, metadataVerified: false }), false); assert.equal(videoContainerVerified({ ...media, streams: [{ kind: 'video', attachedPicture: true }] }), false); assert.equal(videoContainerVerified({ ...media, streams: [{ kind: 'audio' }] }), false);
  assert.equal(videoAdoptionProblems(workspace, { ...unit, version: 3 }, media).length, 1); assert.equal(videoAdoptionProblems(workspace, { ...unit, version: undefined }, media).length, 1);
  unit.adoptedVideoVersion = 1; assert.equal(unit.adoptedVideoVersion === unit.version, false); assert.equal(unitResultIds(workspace, unit).includes('old-video'), true);
});

test('模型、比例与尺寸只来自对应类型能力，非法旧值不会自动替换或提交', () => {
  const workspace = fixture(); workspace.providers = [{ id: 'dreamina', models: ['unverified', { id: '5.0', kind: 'image', adapted: true, ratios: ['1:1', '9:16', '16:9'], resolutions: ['2k', '4k', '1080p'] }, { id: '2.0', kind: 'video', adapted: true, ratios: ['16:9', '9:16'], resolutions: ['720p', '1080p'] }, { id: 'new', kind: 'image', adapted: false, ratios: ['1:1'], resolutions: ['2k'] }] }];
  const image = generationChoices(workspace, 'dreamina', 'image', { model: '5.0', ratio: '16:9', resolution: '1080p' }); assert.equal(image.valid, true); assert.deepEqual(image.ratios, ['16:9', '9:16', '1:1']); assert.deepEqual(image.resolutions, ['2k', '4k', '1080p']); assert.equal(image.models.some(model => model.id === '2.0'), false);
  for (const invalid of [{ model: '5.0', ratio: '21:9', resolution: '2k' }, { model: '5.0', ratio: '16:9', resolution: '2K' }, { model: 'new', ratio: '1:1', resolution: '2k' }, { model: 'unverified', ratio: '16:9', resolution: '2k' }]) assert.equal(generationChoices(workspace, 'dreamina', 'image', invalid).valid, false);
  assert.equal(generationChoices(workspace, 'dreamina', 'video', { model: '2.0', ratio: '9:16', resolution: '1080p' }).valid, true); assert.equal(generationChoices(workspace, 'dreamina', 'video', { model: '2.0', ratio: '9:16', resolution: '4k' }).valid, false);
  assert.equal(resolutionLabel('1080p', 'image'), '1080P（像素尺寸）'); assert.equal(resolutionLabel('4k', 'image'), '4K'); assert.equal(ratioLabel('9:16'), '9:16 · 竖屏');
});

function imageCatalogFixture(): Workspace {
  return { ...fixture(), providers: [{ id: 'dreamina', models: [{ id: '5.0', kind: 'image', adapted: true, available: true, ratios: ['16:9', '9:16'], resolutions: ['2k', '4k'] }, { id: 'video-known', kind: 'video', adapted: true, available: true }] }] };
}
test('迟到的真实目录只初始化即梦5.0空模型，保留有效比例尺寸，不猜未知或其他渠道模型', () => {
  const workspace = imageCatalogFixture(), empty = { model: '', ratio: '', resolution: '' };
  assert.deepEqual(initializeImageParameters(fixture(), 'dreamina', empty), empty);
  assert.deepEqual(initializeImageParameters(workspace, 'dreamina', empty), { model: '5.0', ratio: '16:9', resolution: '2k' });
  assert.deepEqual(initializeImageParameters(workspace, 'dreamina', { ...empty, ratio: '9:16', resolution: '4k' }), { model: '5.0', ratio: '9:16', resolution: '4k' });
  const rustCatalog = structuredClone(workspace); delete (rustCatalog.providers as { models: { available?: boolean }[] }[])[0].models[0].available;
  assert.deepEqual(initializeImageParameters(rustCatalog, 'dreamina', empty), { model: '5.0', ratio: '16:9', resolution: '2k' }, 'Rust ModelCapability omits available; adapted typed capability remains selectable');
  assert.equal(generationChoices(rustCatalog, 'dreamina', 'image', initializeImageParameters(rustCatalog, 'dreamina', empty)).valid, true);
  for (const change of [{ id: '5.0-Pro' }, { kind: 'video' }, { adapted: false }, { available: false }]) {
    const changed = structuredClone(workspace); (changed.providers as { models: object[] }[])[0].models = [{ ...(workspace.providers as { models: object[] }[])[0].models[0], ...change }];
    assert.deepEqual(initializeImageParameters(changed, 'dreamina', empty), empty);
  }
  workspace.providers = [{ id: 'bh', models: [{ id: '5.0', kind: 'image', adapted: true, available: true, ratios: ['16:9'], resolutions: ['2k'] }] }];
  assert.deepEqual(initializeImageParameters(workspace, 'bh', empty), empty);
});
test('已保存图片模型优先，非法旧模型和用户当前参数不会被默认值悄悄替换', () => {
  const workspace = imageCatalogFixture(), parameters = { model: '', ratio: '9:16', resolution: '4k' };
  workspace.settings = { imageProvider: 'dreamina', imageModel: 'retired-model', ratio: '16:9', imageResolution: '2k' };
  assert.deepEqual(initializeImageParameters(workspace, 'dreamina', parameters), { ...parameters, model: 'retired-model' });
  assert.deepEqual(initializeImageParameters(workspace, 'dreamina', { model: 'chosen-model', ratio: 'old-ratio', resolution: 'old-size' }), { model: 'chosen-model', ratio: 'old-ratio', resolution: 'old-size' });
  workspace.settings.imageModel = '5.0';
  assert.deepEqual(initializeImageParameters(workspace, 'dreamina', { model: '', ratio: '', resolution: '' }), { model: '5.0', ratio: '16:9', resolution: '2k' });
  assert.deepEqual(initializeImageParameters(workspace, 'dreamina', { model: '', ratio: 'old-ratio', resolution: 'old-size' }), { model: '5.0', ratio: 'old-ratio', resolution: 'old-size' });
  workspace.settings.ratio = 'old-ratio'; workspace.settings.imageResolution = 'old-size';
  assert.deepEqual(initializeImageParameters(workspace, 'dreamina', { model: '', ratio: '', resolution: '' }), { model: '5.0', ratio: 'old-ratio', resolution: 'old-size' });
});
test('Chrome 真实唯一可用图片目录补齐空模型与原生尺寸，缺失和歧义目录不猜测', () => {
  const empty = { model: '', ratio: '', resolution: '' };
  const capability = { id: 'chatgpt-image', kind: 'image', adapted: true, ratios: ['16:9', '1:1'], resolutions: ['native'] };
  const workspace = { ...fixture(), settings: { imageProvider: 'chrome' }, providers: [{ id: 'chrome', models: [capability] }] };
  const initialized = { model: 'chatgpt-image', ratio: '16:9', resolution: 'native' };
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', empty), initialized);
  assert.equal(generationChoices(workspace, 'chrome', 'image', initialized).valid, true);
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', { ...empty, ratio: '1:1' }), { ...initialized, ratio: '1:1' });
  for (const models of [[], ['chatgpt-image'], [{ ...capability, kind: 'video' }], [{ ...capability, adapted: false }], [{ ...capability, available: false }], [{ ...capability, id: 'unrecognized' }], [capability, capability], [capability, { ...capability, id: 'another-image' }]]) {
    const changed: Workspace = { ...workspace, providers: [{ id: 'chrome', models }] };
    assert.deepEqual(initializeImageParameters(changed, 'chrome', empty), empty);
  }
  assert.deepEqual(initializeImageParameters({ ...workspace, providers: [{ id: 'bh', models: [capability] }] }, 'chrome', empty), empty);
  assert.deepEqual(initializeImageParameters({ ...workspace, providers: [{ id: 'bh', models: [capability] }] }, 'bh', empty), empty);
});
test('Chrome 默认值保留已保存非法值、用户选择及已有尺寸，不将其他渠道旧值带入', () => {
  const empty = { model: '', ratio: '', resolution: '' };
  const workspace: Workspace = { ...fixture(), settings: { imageProvider: 'chrome' }, providers: [{ id: 'chrome', models: [{ id: 'chatgpt-image', kind: 'image', adapted: true, available: true, ratios: ['16:9', '1:1'], resolutions: ['native'] }] }] };
  workspace.settings.imageModel = 'retired-model';
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', empty), { ...empty, model: 'retired-model' });
  workspace.settings.imageModel = 'chatgpt-image'; workspace.settings.imageResolution = 'old-size'; workspace.settings.ratio = 'old-ratio';
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', empty), { model: 'chatgpt-image', ratio: 'old-ratio', resolution: 'old-size' });
  const selected = { model: 'chosen-model', ratio: 'chosen-ratio', resolution: 'chosen-size' };
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', selected), selected);
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', { model: 'chatgpt-image', ratio: '1:1', resolution: 'chosen-size' }), { model: 'chatgpt-image', ratio: '1:1', resolution: 'chosen-size' });
  workspace.settings = { imageProvider: 'bh', imageModel: 'bh-model', imageResolution: '2k' };
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', empty), { model: 'chatgpt-image', ratio: '16:9', resolution: 'native' });
  (workspace.providers as { models: { resolutions: string[] }[] }[])[0].models[0].resolutions = ['2k'];
  assert.deepEqual(initializeImageParameters(workspace, 'chrome', empty), { model: 'chatgpt-image', ratio: '16:9', resolution: '' });
});
test('缺目录发现合并图片视频渠道，字符串与错误类型目录不能冒充核实能力', () => {
  const workspace = fixture();
  assert.deepEqual(missingCatalogs(workspace).map(item => [item.provider, item.kinds]), [['dreamina', ['image', 'video']]]);
  workspace.providers = { dreamina: { models: ['5.0', { id: 'video-known', kind: 'video' }] } };
  assert.equal(missingCatalogs(workspace).length, 1);
  workspace.providers = imageCatalogFixture().providers;
  assert.equal(missingCatalogs(workspace).length, 0);
  workspace.settings.imageProvider = 'bh';
  assert.deepEqual(missingCatalogs(workspace).map(item => item.provider), ['bh']);
});
test('Chrome 绑定变更允许目录重新核实，其他渠道和轮询状态不触发重复检查', () => {
  const workspace=fixture();workspace.settings.imageProvider='chrome';
  const key=(provider: string)=>missingCatalogs(workspace).find(item=>item.provider===provider)?.key;
  const before=key('chrome'),dreamina=key('dreamina');
  workspace.settings.chromeProfileId='b84d4521-56ba-4f84-9948-090f82d639e7';workspace.settings.chromeProfileEndpoint='http://127.0.0.1:53179';workspace.settings.chromeProfileBundleId='bundle';
  const bound=key('chrome');assert.notEqual(bound,before);assert.equal(key('dreamina'),dreamina);
  workspace.settings.chromeToken='synthetic-never-real';workspace.settings.chromeCheckedAt='later';assert.equal(key('chrome'),bound);
  workspace.settings.chromeProfileId='f913ed04-6d73-4f40-bd74-040418327d79';assert.notEqual(key('chrome'),bound);
});
test('目录自动读取失败只执行一次，项目与稳定配置变更可独立检查，预览不访问渠道', async () => {
  let workspace = fixture(), enabled = false, calls = 0; const states: string[] = [];
  const inspect = createCatalogDiscovery({ enabled: () => enabled, current: () => workspace, busy: () => false, revision: () => 0, refresh: async () => { calls++; throw new Error('offline capability failure'); }, read: async () => { throw new Error('must not read after failure'); }, apply: () => assert.fail('must not apply'), report: state => states.push(state.status) });
  await inspect(); assert.equal(calls, 0);
  enabled = true; await Promise.all([inspect(), inspect(), inspect()]); await inspect(); assert.equal(calls, 1); assert.deepEqual(states, ['checking', 'error']);
  workspace.settings.bhToken = 'test-only-not-a-real-token'; workspace.dreaminaAccount = { status: 'unknown', checkedAt: 'later', identityVerified: false, ready: false }; await inspect();
  assert.equal(calls, 1); workspace.dreaminaAccount.status = 'error'; workspace.dreaminaAccount.checkedAt = 'latest'; await inspect(); assert.equal(calls, 1, 'error-state polling is not an automatic retry');
  workspace.dreaminaAccount.status = 'authenticated'; workspace.dreaminaAccount.ready = true; await inspect(); assert.equal(calls, 2, 'a newly connected account can inspect once without automatic login');
  workspace.settings.dreaminaPath = 'offline-cli-fixture'; await inspect(); assert.equal(calls, 3);
  workspace = { ...workspace, project: { id: 'other-project', title: '其他项目' } }; await inspect(); assert.equal(calls, 4);
});
test('目录读取在新操作期间等待，恢复只补读项目，不重复检查渠道或覆盖新操作', async () => {
  let workspace = fixture(), busy = false, revision = 0, refreshes = 0, reads = 0;
  let finishRefresh!: () => void; const waiting = new Promise<void>(resolve => { finishRefresh = resolve; });
  const inspect = createCatalogDiscovery({ enabled: () => true, current: () => workspace, busy: () => busy, revision: () => revision, refresh: async () => { refreshes++; await waiting; }, read: async () => { reads++; return { ...imageCatalogFixture(), foundation: workspace.foundation }; }, apply: next => { workspace = next; revision++; }, report: () => {} });
  const first = inspect(); busy = true; revision++; workspace = { ...workspace, foundation: { result: 'newer-operation' } }; finishRefresh(); await first;
  assert.equal(refreshes, 1); assert.equal(reads, 0); assert.deepEqual(workspace.foundation, { result: 'newer-operation' });
  busy = false; await inspect(); assert.equal(refreshes, 1); assert.equal(reads, 1); assert.deepEqual(workspace.foundation, { result: 'newer-operation' });
  await inspect(); assert.equal(refreshes, 1);
});
test('打开其他项目时丢弃旧目录读结果并继续新项目，空结果不自动循环重试', async () => {
  let workspace = fixture(), revision = 0, refreshed: string[] = []; const applied: string[] = [], states: string[] = [];
  let finishRead!: (workspace: Workspace) => void; const waiting = new Promise<Workspace>(resolve => { finishRead = resolve; });
  const inspect = createCatalogDiscovery({ enabled: () => true, current: () => workspace, busy: () => false, revision: () => revision, refresh: async projectId => { refreshed.push(projectId); }, read: async projectId => projectId === 'p' ? waiting : workspace, apply: next => { applied.push(next.project.id); workspace = next; }, report: state => states.push(state.status) });
  const first = inspect(); await Promise.resolve(); workspace = { ...fixture(), project: { id: 'new-project', title: '新项目' } }; revision++; finishRead(imageCatalogFixture()); await first;
  assert.deepEqual(refreshed, ['p', 'new-project']); assert.deepEqual(applied, ['new-project']); assert.equal(states.at(-1), 'error');
  await inspect(); assert.equal(refreshed.length, 2);
});

test('分镜文字创作从当前锁定原文选择制作集，零单元和未配置视频模型也可启动', () => {
  const workspace = fixture(); workspace.units = []; workspace.source = { episodes: [{ id: 'ep-2', number: 2, title: '第二集' }, { id: 'ep-1', number: 1 }, { id: 'ep-2', title: '重复' }, { title: '未确认' }] };
  assert.equal(storyboardRange(workspace).valid, true);
  assert.deepEqual(storyboardRange(workspace).episodeIds, ['ep-2', 'ep-1']);
  assert.deepEqual(storyboardRange(workspace, 'ep-1').episodeIds, ['ep-1']);
  assert.equal(storyboardRange(workspace, 'old-episode').valid, false);
  workspace.source = { episodes: [] }; workspace.units = [{ id: 'old-unit', episodeId: 'old-episode', prompt: '旧分镜' }];
  assert.equal(storyboardRange(workspace).valid, false);
  assert.deepEqual(storyboardRange(workspace).episodeIds, []);
  workspace.source = null;
  assert.equal(storyboardRange(workspace).valid, false);
});

test('视频尺寸随核实的型号改变，VIP多尺寸与普通固定尺寸不会串用', () => {
  const workspace = fixture(); workspace.providers = [{ id: 'dreamina', models: [
    { id: 'seedance2.5', kind: 'video', adapted: true, ratios: ['16:9', '9:16'], resolutions: ['480p', '720p', '1080p'] },
    { id: 'seedance2.0_vip', kind: 'video', adapted: true, ratios: ['16:9', '9:16'], resolutions: ['720p', '1080p', '4k'] },
    { id: 'seedance2.0', kind: 'video', adapted: true, ratios: ['16:9', '9:16'], resolutions: ['720p'] }
  ] }];
  const choice = (model: string, resolution = '720p') => generationChoices(workspace, 'dreamina', 'video', { model, ratio: '16:9', resolution });
  assert.deepEqual(choice('seedance2.0_vip').resolutions, ['720p', '1080p', '4k']);
  assert.deepEqual(choice('seedance2.0').resolutions, ['720p']);
  assert.equal(choice('seedance2.0', '4k').valid, false);
  assert.equal(choice('seedance2.0_vip', '4k').valid, true);
});

test('图片当前与历史使用精确形象关联，历史从新到旧且排除当前图', () => {
  const workspace = fixture(), asset = workspace.assets[0];
  workspace.media = [
    { id: 'old', kind: 'image', path: 'old.png', assetId: 'a', variantId: 'v', createdAt: '2026-10-01T00:00:00Z' },
    { id: 'current', kind: 'image', path: 'current.png', assetId: 'a', variantId: 'v', createdAt: '2026-10-02T00:00:00Z' },
    { id: 'latest', kind: 'image', path: 'latest.png', assetId: 'a', variantId: 'v', createdAt: '2026-10-03T00:00:00Z' },
    { id: 'other-variant', kind: 'image', path: 'other.png', assetId: 'a', variantId: 'other', adopted: true },
    { id: 'other-asset', kind: 'image', path: 'other-asset.png', assetId: 'other', variantId: 'v', adopted: true },
    { id: 'video', kind: 'video', path: 'video.mp4', assetId: 'a', variantId: 'v' },
  ];
  asset.adoptedMediaId = 'current'; asset.candidates = [];
  const images = assetImages(workspace, asset);
  assert.equal(images.current?.id, 'current');
  assert.deepEqual(images.history.map(media => media.id), ['latest', 'old']);
  assert.deepEqual(assetImageSelection(workspace, asset, 'old'), { assetId: 'a', variantId: 'v', mediaId: 'old' });
  assert.throws(() => assetImageSelection(workspace, asset, 'other-variant'), /不属于当前资产形象/);
  assert.throws(() => assetImageSelection(workspace, asset, 'other-asset'), /不属于当前资产形象/);
  const switched = assetImages(workspace, { ...asset, adoptedMediaId: 'old' });
  assert.equal(switched.current?.id, 'old');
  assert.deepEqual(switched.history.map(media => media.id), ['latest', 'current']);
  assert.equal(assetImages(workspace, { ...asset, adoptedMediaId: 'other-asset' }).current, undefined);
  assert.equal(assetImages(workspace, { ...asset, adoptedMediaId: undefined }).current, undefined);
});

test('共用原图只通过当前形象明确关联显示，全局采用标记不推断当前图', () => {
  const workspace = fixture(), asset = workspace.assets[0];
  asset.adoptedMediaId = 'shared';
  assert.equal(assetImages(workspace, asset).current?.id, 'shared');
  asset.candidates = [];
  assert.equal(assetImages(workspace, asset).current, undefined);
  asset.candidates = [{ ...workspace.media[1], assetId: 'a', variantId: 'different' }];
  assert.equal(assetImages(workspace, asset).current, undefined);
  asset.candidates = ['shared', 'old-video'];
  assert.equal(assetImages(workspace, asset).images.some(media => media.kind === 'video'), false);
  const empty = assetImages({ ...workspace, media: [] }, asset);
  assert.equal(empty.current, undefined);
  assert.deepEqual(empty.history, []);
});

function publishedUnits(): Unit[] {
  return [
    ...Array.from({ length: 10 }, (_, index) => ({ id: `retired-${index}`, episodeId: 'ep-1', prompt: '旧稿', retired: true })),
    ...Array.from({ length: 11 }, (_, index) => ({ id: `current-${index}`, episodeId: 'ep-1', prompt: '当前稿', sourceVersion: 3 })),
    ...Array.from({ length: 8 }, (_, index) => ({ id: `other-${index}`, episodeId: 'ep-2', prompt: '其他集待处理稿', sourceVersion: 2 })),
  ];
}
test('全选意图从零单元及旧10单元跟随发布后的当前19单元，不提交retired', () => {
  const units = publishedUnits();
  assert.deepEqual(selectedUnitsForScope([], 'all', null), []);
  assert.equal(selectedUnitsForScope(units.slice(0, 10).map(unit => ({ ...unit, retired: false })), 'all', null).length, 10);
  const selected = selectedUnitsForScope(units, 'all', null);
  assert.equal(selected.length, 19);
  assert.ok(selected.every(unit => unit.retired !== true));
  assert.equal(selected.filter(unit => unit.sourceVersion === 2).length, 8, 'other active episodes remain available; source age is not a retirement decision');
  assert.equal(units.length, 29, 'selection is read-only and preserves history');
});
test('切换制作集时计数、就绪候选和请求ID来自同一范围，隐藏集ID不混入', () => {
  const units = publishedUnits();
  assert.equal(unitsForScope(units, 'ep-1').length, 11);
  assert.equal(unitsForScope(units, 'ep-2').length, 8);
  assert.equal(selectedUnitsForScope(units, 'ep-1', null).length, 11);
  const manual = ['current-0', 'other-0', 'retired-0', 'missing', 'current-0'];
  assert.deepEqual(selectedUnitsForScope(units, 'ep-1', manual).map(unit => unit.id), ['current-0']);
  assert.deepEqual(selectedUnitsForScope(units, 'ep-2', manual).map(unit => unit.id), ['other-0']);
  assert.deepEqual(selectedUnitsForScope(units, 'all', manual).map(unit => unit.id), ['current-0', 'other-0']);
  assert.deepEqual(selectedUnitsForScope(units, 'ep-1', []), []);
});
test('手动取消后保留精确筛选，不自动选新稿；再次全选恢复跟随', () => {
  const units = publishedUnits(), manual = toggleUnitSelection(units, null, 'current-0');
  assert.equal(manual.length, 18);
  assert.ok(!manual.includes('current-0'));
  const updated = [...units.map(unit => unit.id === 'current-1' ? { ...unit, retired: true } : unit), { id: 'new', episodeId: 'ep-1', prompt: '新稿' }];
  const selected = selectedUnitsForScope(updated, 'all', manual);
  assert.equal(selected.length, 17);
  assert.ok(!selected.some(unit => unit.id === 'new' || unit.id === 'current-1'));
  assert.equal(selectedUnitsForScope(updated, 'all', null).length, 19);
  assert.ok(selectedUnitsForScope(updated, 'all', null).some(unit => unit.id === 'new'));
});

 test('BH 质量字段来自显式合同，旧尺寸必须重新选择，空值采用已核实默认质量', () => {
 const model = { id: 'gpt-image-2', kind: 'image', adapted: true, ratios: ['16:9'], resolutions: ['low','medium','high'], raw: { desktopParameterContract: { outputParameter: 'quality_level', defaultOutput: 'medium' } } };
 const workspace = fixture(); workspace.settings = { imageProvider: 'bh', imageModel: model.id }; workspace.providers = [{id:'bh',models:[model]}];
 assert.equal(imageOutputLabel(model), '图片质量'); assert.equal(imageOutputValueLabel('medium', model), '标准');
 assert.equal(imageOutputLabel({...model,raw:{}}), '图片尺寸'); assert.equal(imageOutputValueLabel('2k', {...model,raw:{}}), '2K');
 const initialized = initializeImageParameters(workspace,'bh',{model:model.id,ratio:'16:9',resolution:''});
 assert.equal(initialized.resolution,'medium'); assert.equal(generationChoices(workspace,'bh','image',initialized).valid,true);
 workspace.settings.imageResolution='2k'; const historical=initializeImageParameters(workspace,'bh',{model:model.id,ratio:'16:9',resolution:''});
 assert.equal(historical.resolution,'2k'); const choices=generationChoices(workspace,'bh','image',historical);
 assert.equal(choices.valid,false); assert.match(choices.problems.join(' '),/质量 2k/);
 assert.equal(imageOutputValueLabel('unrecognized',model),'unrecognized');
});
