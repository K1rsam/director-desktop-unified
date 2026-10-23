import test from 'node:test';
import assert from 'node:assert/strict';
import * as React from 'react';
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import type { Workspace } from './types.ts';

test('library surfaces newly imported projects first and searches without removing saved records', async () => {
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { invoke: async () => ({ mode: 'standard_native', allowedActions: [], reasons: [] }) } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const view = hookRenderer();
  try {
    const { ProjectLibrary, findProjects } = await server.ssrLoadModule('/src/ProjectLibrary.tsx');
    const projects = [{ id: 'old', title: '旧项目', createdAt: '2026-01-01' }, { id: 'imported', title: '新导入全剧', createdAt: '2026-10-23' }];
    assert.equal(findProjects(projects, '')[0].id, 'imported');
    assert.equal(projects[0].id, 'old', 'sorting must not mutate the registry');
    const opened: string[] = [];
    const element = React.createElement(ProjectLibrary, { projects, filter: 'active', writable: true, busy: false, onFilterChange() {}, onOpen(project: { id: string }) { opened.push(project.id); }, onLifecycle: async () => true });
    let nodes = view.render(element);
    assert.equal(nodes.find(node => node.props.className === 'project-card-open')?.props['aria-label'], '打开项目：新导入全剧');
    nodes.find(node => node.props['aria-label'] === '搜索项目')!.props.onChange({ target: { value: '全剧' } });
    nodes = view.render(element);
    assert.equal(nodes.filter(node => node.props.className === 'project-card-open').length, 1);
    nodes.find(node => node.props.className === 'project-card-open')!.props.onClick(); assert.deepEqual(opened, ['imported']);
  } finally { view.unmount(); await server.close(); }
});

test('reopened materials distinguish asset and file counts and recover unmatched files without guessing bindings', async () => {
  const asset = { id: 'asset', name: '甲', variantId: 'v', lingmoType: 'CH' };
  const workspace: Workspace = { project: { id: 'p', title: '合成' }, assets: [asset], media: [], units: [], jobs: [], settings: {}, providers: [], rules: [], lingmoDocuments: [{ id: 'doc', markdownSha256: 'sha', filename: '合成.md', counts: { episodes: 1, scenes: 1, fragments: 1, shots: 1, mentionOccurrences: 1, assets: 1 }, episodes: [], scenes: [{ id: 's', episodeId: 'e', number: '1-1', unitIds: [], assetIds: ['asset'] }], diagnostics: [], parserVersion: 'test' }] };
  const row = { id: 'extra', path: 'C:/files/角色-甲受伤.png', filename: '角色-甲受伤.png', relativePath: '角色-甲受伤.png', kind: 'image', status: 'unmatched', candidateAssetIds: [], size: 100 };
  const scan = { preview: { previewToken: 'original-preview', documentId: 'doc', rows: [row], counts: { files: 3, matched: 2, unmatched: 1, ambiguous: 0, unsupported: 0 }, milliseconds: 1 }, fileCount: 3, importedFiles: 2, pendingFiles: 1, pendingRows: [row] };
  const calls: string[] = [], writes: unknown[] = [];
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { invoke: async (_: string, args: { request: any }) => {
    calls.push(args.request.action);
    if (args.request.action === 'getExecutionPolicy') return { mode: 'standard_native', allowedActions: ['lingmo.media.latest'], reasons: [] };
    if (args.request.action === 'lingmo.media.latest') return scan;
    throw Error('unexpected request');
  } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' }), view = hookRenderer();
  try {
    const { LingmoMaterialsDialog } = await server.ssrLoadModule('/src/LingmoMaterialsDialog.tsx');
    const props = { workspace, documentId: 'doc', writable: true, inspect() {}, onClose() {}, mutate: async (...args: unknown[]) => { writes.push(args); return workspace; } };
    const element = React.createElement(LingmoMaterialsDialog, props);
    const render = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await render(); nodes = await render();
    assert.ok(nodes.some(node => content(node).includes('1 项剧本资产')));
    assert.ok(nodes.some(node => content(node).includes('上次文件夹：3 个文件')));
    assert.ok(nodes.some(node => content(node).includes('已入库 2 · 未入库 1')));
    nodes.find(node => node.type === 'button' && content(node) === '查看 1 个未入库文件')!.props.onClick();
    nodes = await render(); nodes = await render();
    assert.ok(nodes.some(node => content(node).includes('角色-甲受伤.png')));
    assert.equal(nodes.find(node => node.type === 'button' && content(node).startsWith('确认导入'))!.props.disabled, true);
    assert.equal(writes.length, 0); assert.equal(calls.includes('files.choose'), false);
  } finally { view.unmount(); await server.close(); }
});
