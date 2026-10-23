import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import type { Workspace } from './types.ts';

test('local materials retain document identities, paginate and safely preview, map and commit a batch without generation', async () => {
  const calls: any[] = [], writes: any[] = []; let closes = 0;
  const sha = 'test-document', assets = Array.from({ length: 83 }, (_, index) => ({ id: `lingmo-${sha}-asset-${index}`, variantId: `v${index}`, name: `人物${index}`, lingmoType: 'CH', ...(index === 0 ? { adoptedMediaId: 'existing' } : {}) }));
  const workspace: Workspace = { project: { id: 'p', title: '合成' }, assets: [...assets, { id: 'other-revision', name: '人物0', lingmoType: 'CH' }], media: [{ id: 'existing', kind: 'image', path: 'x.png', assetId: assets[0].id, variantId: 'v0' }, { id: 'white', kind: 'image', path: 'white.png', assetId: assets[1].id, variantId: 'v1', placeholder: true } as any], units: [], jobs: [], settings: {}, providers: [], rules: [], lingmoDocuments: [{ id: 'doc', markdownSha256: sha, filename: '合成.md', counts: { episodes: 1, scenes: 1, fragments: 1, shots: 1, mentionOccurrences: 1, assets: 83 }, episodes: [], scenes: [{ id: 's', episodeId: 'e', number: '150-1', unitIds: [], assetIds: assets.slice(0, 2).map(asset => asset.id) }], diagnostics: [], parserVersion: 'test' }] };
  const rows = [
    { id: 'r0', path: 'C:/batch/a.png', filename: 'a.png', relativePath: 'a.png', size: 2000, kind: 'image', status: 'matched', assetId: assets[0].id, variantId: 'v0', candidateAssetIds: [] },
    { id: 'r1', path: 'C:/batch/b.png', filename: 'b.png', relativePath: 'b.png', size: 3000, kind: 'image', status: 'unmatched', candidateAssetIds: [] },
    { id: 'r2', path: 'C:/batch/x.txt', filename: 'x.txt', relativePath: 'x.txt', size: 1, kind: 'image', status: 'unsupported', candidateAssetIds: [] },
  ];
  const preview = { previewToken: 'immutable-preview', documentId: 'doc', rows, counts: { files: 3, matched: 1, unmatched: 1, ambiguous: 0, unsupported: 1 }, milliseconds: 1 };
  const policy = { mode: 'standard_native', reasons: [], liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', allowedActions: ['files.choose', 'lingmo.media.preview', 'lingmo.media.commit'] };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { convertFileSrc: (path: string) => `asset://${path}`, invoke: async (_: string, args: { request: any }) => { const input = args.request; calls.push(input); if (input.action === 'getExecutionPolicy') return policy; if (input.action === 'files.choose') return ['C:/batch']; if (input.action === 'lingmo.media.preview') return preview; throw Error('Unexpected action: ' + input.action); } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const view = hookRenderer();
  let finish: (value: Workspace) => void = () => {};
  try {
    const { LingmoMaterialsDialog, documentMaterials, materialSelections } = await server.ssrLoadModule('/src/LingmoMaterialsDialog.tsx');
    assert.equal(documentMaterials(workspace, 'doc').length, 83); assert.equal(documentMaterials(workspace, 'missing').length, 0);
    const pure = materialSelections(rows, { r0: assets[0].id, r1: assets[1].id }, assets, workspace, false);
    assert.deepEqual(pure.selections[0], { rowId: 'r0', assetId: assets[0].id, variantId: 'v0', adopt: false, expectedAdoptedMediaId: 'existing' });
    assert.equal(pure.selections[1].adopt, true); assert.equal(pure.selections[1].expectedAdoptedMediaId, null);
    assert.equal(materialSelections(rows, { r0: assets[0].id, r1: assets[0].id }, assets, workspace, false).duplicates.size, 1);
    const audio = materialSelections([{ ...rows[1], kind: 'audio' }], { r1: assets[1].id }, assets, workspace, true); assert.equal(audio.selections.length, 0, 'image-only assets cannot receive audio');
    assert.equal(materialSelections([{ ...rows[0], expectedAdoptedMediaId: null }], { r0: assets[0].id }, assets, workspace, true).selections[0].expectedAdoptedMediaId, null, 'explicit preview null must survive newer workspace adoption');
    const props = { workspace, documentId: 'doc', writable: true, inspect() {}, onClose() { closes++; }, mutate: async (action: string, values: any) => { writes.push({ action, ...values }); return new Promise<Workspace>(resolve => { finish = resolve; }); } };
    let element: React.ReactElement = React.createElement(LingmoMaterialsDialog, props);
    const render = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await render();
    assert.equal(nodes.filter(node => node.type === 'article' && node.props.className === 'lingmo-materials-card').length, 80);
    assert.ok(nodes.some(node => content(node).includes('白图占位 · 待补图')));
    nodes.find(node => node.type === 'button' && content(node) === '下一页')!.props.onClick(); nodes = await render(); assert.equal(nodes.filter(node => node.type === 'article').length, 3);
    nodes.find(node => node.props['aria-label'] === '搜索素材名称或场次')!.props.onChange({ target: { value: '150-1' } }); nodes = await render(); assert.equal(nodes.filter(node => node.type === 'article').length, 2);
    nodes.find(node => node.props['aria-label'] === '素材类型')!.props.onChange({ target: { value: 'SC' } }); nodes = await render(); assert.equal(nodes.filter(node => node.type === 'article').length, 0);
    nodes.find(node => node.type === 'button' && content(node).includes('导入文件夹'))!.props.onClick(); nodes = await render(); nodes = await render();
    assert.equal(calls.find(call => call.action === 'files.choose').kind, 'directory'); assert.equal(calls.find(call => call.action === 'lingmo.media.preview').directory, 'C:/batch');
    element = React.createElement(LingmoMaterialsDialog, { ...props, workspace: { ...workspace, assets: workspace.assets.map(asset => asset.id === assets[0].id ? { ...asset, adoptedMediaId: 'later-adoption' } : asset) } });
    nodes = await render();
    assert.equal(nodes.filter(node => node.type === 'select' && String(node.props['aria-label']).startsWith('绑定资产')).length, 0, 'batch preview does not mount a full asset selector for every file');
    nodes.find(node => node.props['aria-label'] === '更改绑定 b.png')!.props.onClick(); nodes = await render();
    const select = () => nodes.find(node => node.props['aria-label'] === '绑定资产 b.png')!;
    select().props.onChange({ target: { value: assets[0].id } }); nodes = await render(); assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('多个文件'))); assert.equal(nodes.find(node => node.type === 'button' && content(node).startsWith('确认导入'))!.props.disabled, true);
    select().props.onChange({ target: { value: assets[1].id } }); nodes = await render(); const submit = nodes.find(node => node.type === 'button' && content(node).startsWith('确认导入'))!; submit.props.onClick(); submit.props.onClick(); nodes = await render();
    assert.equal(writes.length, 1); assert.equal(writes[0].action, 'lingmo.media.commit'); assert.equal(writes[0].previewToken, 'immutable-preview'); assert.equal(writes[0].selections[0].adopt, false); assert.equal(writes[0].selections[0].expectedAdoptedMediaId, 'existing'); assert.equal(writes[0].selections[1].assetId, assets[1].id);
    nodes.find(node => node.type === 'dialog')!.props.onCancel({ preventDefault() {} }); assert.equal(closes, 0, 'Esc must not dismiss an in-flight local import');
    finish({ ...workspace, lingmoMediaImportReport: { imported: 1, adopted: 0, placeholders: 0, skipped: 1, failed: 1, rows: [{ rowId: 'r0', status: 'imported', mediaId: 'new' }, { rowId: 'r1', status: 'adoption_conflict', message: '文件已变化，请重新扫描' }], milliseconds: 30 } } as Workspace); nodes = await render();
    assert.ok(nodes.some(node => content(node).includes('文件已变化，请重新扫描'))); assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('失败 1')));
    nodes.find(node => node.type === 'button' && content(node) === '清除已完成')!.props.onClick(); nodes = await render(); assert.equal(nodes.some(node => node.props['aria-label'] === '绑定资产 a.png'), false); assert.equal(nodes.some(node => node.props['aria-label'] === '绑定资产 b.png'), true);
    assert.ok(calls.every(call => ['getExecutionPolicy', 'files.choose', 'lingmo.media.preview', 'lingmo.media.latest'].includes(call.action))); assert.ok(writes.every(call => call.action === 'lingmo.media.commit'));
  } finally { view.unmount(); await server.close(); }
});

test('shared audio candidates remain visible and audio mapping offers only SE or OM', async () => {
  const se = { id: 'se', name: '风声', variantId: 'v-se', lingmoType: 'SE', candidates: ['shared-audio'] }, om = { id: 'om', name: '群演', variantId: 'v-om', lingmoType: 'OM' }, ch = { id: 'ch', name: '主角', lingmoType: 'CH' };
  const audio = { id: 'shared-audio', kind: 'audio', path: 'sound.wav', assetId: 'another-owner', variantId: 'another-variant' };
  const workspace: Workspace = { project: { id: 'p', title: '合成' }, assets: [se, om, ch], media: [audio], units: [], jobs: [], settings: {}, providers: [], rules: [], lingmoDocuments: [{ id: 'doc', markdownSha256: 'sha', filename: '合成.md', counts: { episodes: 1, scenes: 1, fragments: 1, shots: 1, mentionOccurrences: 1, assets: 3 }, episodes: [], scenes: [{ id: 's', episodeId: 'e', number: '1-1', unitIds: [], assetIds: ['se', 'om', 'ch'] }], diagnostics: [], parserVersion: 'test' }] };
  const row = { id: 'audio-row', filename: '风声.wav', path: 'C:/sound.wav', relativePath: 'sound.wav', kind: 'audio', status: 'unmatched', candidateAssetIds: [], size: 100 };
  const policy = { mode: 'standard_native', reasons: [], liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', allowedActions: ['files.choose', 'lingmo.media.preview'] };
  const inspected: any[] = [];
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { convertFileSrc: (path: string) => `asset://${path}`, invoke: async (_: string, args: { request: any }) => { if (args.request.action === 'getExecutionPolicy') return policy; if (args.request.action === 'files.choose') return ['C:/sound.wav']; if (args.request.action === 'lingmo.media.preview') return { previewToken: 'audio-token', documentId: 'doc', rows: [row], counts: { files: 1, matched: 0, unmatched: 1, ambiguous: 0, unsupported: 0 }, milliseconds: 1 }; throw Error('Unexpected action'); } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' }), view = hookRenderer();
  try {
    const { LingmoMaterialsDialog, materialAudio, materialCanBind, materialSelections } = await server.ssrLoadModule('/src/LingmoMaterialsDialog.tsx');
    assert.equal(materialAudio(workspace, se)?.id, 'shared-audio');
    assert.equal(materialCanBind('audio', se), true); assert.equal(materialCanBind('audio', om), true); assert.equal(materialCanBind('audio', ch), false); assert.equal(materialCanBind('image', se), false);
    assert.equal(materialSelections([row], { 'audio-row': 'se' }, workspace.assets, workspace, true).selections[0].adopt, false);
    const props = { workspace, documentId: 'doc', writable: true, inspect: (media: unknown) => inspected.push(media), onClose() {}, mutate: async () => workspace };
    const element = React.createElement(LingmoMaterialsDialog, props), render = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await render(); assert.ok(nodes.some(node => content(node).includes('已入库音频，片段中选择')));
    nodes.find(node => node.type === 'button' && node.props['aria-label'] === '播放音频 风声')!.props.onClick(); assert.equal(inspected[0].id, 'shared-audio');
    nodes.find(node => node.type === 'button' && content(node).includes('导入音频 / 文件'))!.props.onClick(); nodes = await render(); nodes = await render();
    nodes.find(node => node.props['aria-label'] === '更改绑定 风声.wav')!.props.onClick(); nodes = await render();
    const select = nodes.find(node => node.props['aria-label'] === '绑定资产 风声.wav')!;
    const options = select.children.filter((child): child is Exclude<typeof child, string> => typeof child !== 'string' && child.type === 'option');
    assert.deepEqual(options.map(option => option.props.value), ['', 'se', 'om']);
  } finally { view.unmount(); await server.close(); }
});

test('shared white and unadopted real image candidates are shown accurately while OM audio is playable', async () => {
  const assets = [{ id: 'white-asset', name: '占位角色', lingmoType: 'CH', candidates: ['white'] }, { id: 'real-asset', name: '候选角色', lingmoType: 'CH', candidates: ['real'] }, { id: 'om', name: '人群声音', lingmoType: 'OM', candidates: ['sound'] }];
  const workspace: Workspace = { project: { id: 'p', title: '合成' }, assets, media: [{ id: 'white', kind: 'image', path: 'white.png', placeholder: true } as any, { id: 'real', kind: 'image', path: 'real.png' }, { id: 'sound', kind: 'audio', path: 'sound.wav' }], units: [], jobs: [], settings: {}, providers: [], rules: [], lingmoDocuments: [{ id: 'doc', markdownSha256: 'sha', filename: '合成.md', counts: { episodes: 1, scenes: 1, fragments: 1, shots: 1, mentionOccurrences: 1, assets: 3 }, episodes: [], scenes: [{ id: 's', episodeId: 'e', number: '1-1', unitIds: [], assetIds: assets.map(asset => asset.id) }], diagnostics: [], parserVersion: 'test' }] };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { convertFileSrc: (path: string) => `asset://${path}` } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' }), view = hookRenderer();
  const inspected: any[] = [], writes: any[] = [];
  try {
    const { LingmoMaterialsDialog } = await server.ssrLoadModule('/src/LingmoMaterialsDialog.tsx');
    const props = { workspace, documentId: 'doc', writable: true, inspect: (media: unknown) => inspected.push(media), onClose() {}, mutate: async (...values: unknown[]) => { writes.push(values); return workspace; } };
    const element = React.createElement(LingmoMaterialsDialog, props), render = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await render();
    const cards = nodes.filter(node => node.type === 'article'); assert.equal(cards.length, 3);
    assert.ok(content(cards[0]).includes('白图占位 · 待补图')); assert.ok(content(cards[1]).includes('候选图，待采用')); assert.ok(!content(cards[1]).includes('已配图'));
    assert.ok(content(cards[2]).includes('已入库音频，片段中选择')); assert.ok(nodes.some(node => content(node).includes('2 项待补图')));
    nodes.find(node => node.type === 'button' && node.props['aria-label'] === '播放音频 人群声音')!.props.onClick(); assert.equal(inspected[0].id, 'sound');
    nodes.find(node => node.type === 'input' && node.props.type === 'checkbox')!.props.onChange({ target: { checked: true } }); nodes = await render(); assert.equal(nodes.filter(node => node.type === 'article').length, 2);
    assert.equal(writes.length, 0, 'rendering candidates never adopts media');
  } finally { view.unmount(); await server.close(); }
});
