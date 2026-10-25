import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import { rustStandardActionCatalogue } from './test-support/rustExecutionPolicy.ts';
import type { Workspace } from './types.ts';

test('file import previews without a project, then creates one idempotent request with the reviewed identity', async () => {
  const calls: any[] = [], imported: Workspace[] = [], destinations: string[] = []; let closes = 0, failPreview = false;
  let finishImport: (workspace: Workspace) => void = () => {};
  const workspace: Workspace = { project: { id: 'created', title: '新项目' }, assets: [], media: [], units: [], jobs: [], settings: {}, providers: [], rules: [] };
  const counts = { episodes: 150, scenes: 371, fragments: 883, shots: 5407, mentionOccurrences: 4736, assets: 518 };
  const policy = { mode: 'standard_native', reasons: [], liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', allowedActions: rustStandardActionCatalogue() };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { invoke: async (_: string, args: { request: any }) => {
    const input = args.request; calls.push(input);
    if (input.action === 'getExecutionPolicy') return policy;
    if (input.action === 'files.choose') return ['C:/synthetic/合成分镜.md'];
    if (input.action === 'lingmo.preview_file' || input.action === 'lingmo.preview') {
      if (failPreview && input.handoffMode !== 'catalog-only') throw new Error('完整接续包文档不可读');
      return { counts, markdownSha256: 'reviewed-sha', previewToken: input.handoffMode === 'catalog-only' ? 'catalog-only-token' : 'catalog-bound-token', diagnostics: [], catalogMatched: true,
        handoff: input.handoffMode === 'catalog-only' ? { sourceKind: 'imported-md', completePackageVerified: false, level: 'md-and-catalog' } : { sourceKind: 'external-package', completePackageVerified: true, proof: { explicitImageCount: 518, registryAssetCount: 700 } } };
    }
    if (input.action === 'lingmo.create_project') return await new Promise<Workspace>(resolve => { finishImport = resolve; });
    if (input.action === 'lingmo.import') return workspace;
    throw Error('Unexpected action: ' + input.action);
  } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  let view = hookRenderer();
  try {
    const { LingmoImportDialog } = await server.ssrLoadModule('/src/LingmoImportDialog.tsx');
    const props = { writable: true, onImported(next: Workspace, target?: string) { imported.push(next); destinations.push(target || 'videos'); }, onClose() { closes++; } };
    let element: React.ReactElement = React.createElement(LingmoImportDialog, props);
    const render = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await render();
    assert.equal(calls.filter(call => call.action !== 'getExecutionPolicy').length, 0, 'opening the dialog must not create a project');
    nodes.find(node => node.type === 'button' && content(node).includes('选择分镜 MD 文件'))!.props.onClick();
    nodes = await render(); nodes = await render();
    const preview = calls.find(call => call.action === 'lingmo.preview_file');
    assert.ok(preview); assert.equal(preview.projectId, undefined);
    assert.equal(preview.handoffMode, 'auto');
    assert.ok(nodes.some(node => content(node).includes('完整接续包已核验') && content(node).includes('700')));
    assert.ok(nodes.some(node => node.props['aria-label'] === '文档结构预览' && content(node).includes('150')));
    const title = nodes.find(node => node.type === 'input' && node.props['aria-label'] === '导入项目名称');
    assert.equal(title?.props.value, '合成分镜');
    title!.props.onChange({ target: { value: '' } }); nodes = await render();
    assert.equal(nodes.find(node => node.type === 'button' && content(node) === '导入并打开项目')!.props.disabled, true);
    nodes.find(node => node.type === 'input' && node.props['aria-label'] === '导入项目名称')!.props.onChange({ target: { value: '新的成稿项目' } }); nodes = await render();
    const submit = nodes.find(node => node.type === 'button' && content(node) === '导入并打开项目')!;
    submit.props.onClick(); submit.props.onClick(); await settle();
    assert.equal(calls.filter(call => call.action === 'lingmo.create_project').length, 1, 'a double click must not create two projects');
    const input = calls.find(call => call.action === 'lingmo.create_project');
    assert.equal(input.projectId, undefined); assert.equal(input.expectedSha256, 'reviewed-sha'); assert.equal(input.previewToken, 'catalog-bound-token'); assert.equal(input.title, '新的成稿项目');
    nodes = await render(); nodes.find(node => node.type === 'button' && node.props['aria-label'] === '关闭')!.props.onClick();
    let prevented = false;
    nodes.find(node => node.type === 'dialog')!.props.onCancel({ preventDefault() { prevented = true; } });
    assert.equal(prevented, true, 'Esc must not bypass the controlled in-flight close guard');
    assert.equal(closes, 0, 'do not dismiss an unknown in-flight import');
    const partial = { ...workspace, lingmoImportReport: { media: { imported: 0, missing: 2, failures: [{ message: '图片包损坏' }] } } };
    finishImport(partial); await settle(); nodes = await render();
    assert.equal(imported.length, 0, 'partial media failures must be shown before leaving the importer');
    assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('图片包损坏')));
    nodes.find(node => node.type === 'button' && content(node) === '打开工作台，稍后补图')!.props.onClick();
    assert.deepEqual(imported, [partial]); assert.deepEqual(destinations, ['videos']);
    view.unmount(); view = hookRenderer(); element = React.createElement(LingmoImportDialog, { ...props, projectId: 'existing-project' });
    nodes = await render(); nodes.find(node => node.type === 'button' && content(node).includes('选择分镜 MD 文件'))!.props.onClick(); nodes = await render(); nodes = await render();
    assert.equal(calls.find(call => call.action === 'lingmo.preview')?.projectId, 'existing-project');
    nodes.find(node => node.type === 'input' && node.props.type === 'radio' && node.props.checked === false)!.props.onChange(); nodes = await render();
    nodes.find(node => node.type === 'button' && content(node) === '确认导入')!.props.onClick(); await settle();
    assert.equal(destinations.at(-1), 'images', 'MD-only can enter prompt work without image generation');
    assert.equal(calls.find(call => call.action === 'lingmo.import')?.projectId, 'existing-project');
    view.unmount(); view = hookRenderer(); failPreview = true; element = React.createElement(LingmoImportDialog, props);
    nodes = await render(); nodes.find(node => node.type === 'button' && content(node).includes('选择分镜 MD 文件'))!.props.onClick(); nodes = await render(); nodes = await render();
    assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('文档不可读')));
    assert.equal(nodes.find(node => node.type === 'button' && content(node) === '导入并打开项目')!.props.disabled, true);
    const explicitFallback = nodes.find(node => node.type === 'button' && content(node) === '仅按 MD 导入，不验证完整接续包');
    assert.ok(explicitFallback, 'a failed package must expose an explicit lower-evidence import choice');
    const beforeFallback = calls.filter(call => call.action === 'lingmo.preview_file').length;
    explicitFallback.props.onClick(); nodes = await render(); nodes = await render();
    assert.equal(calls.filter(call => call.action === 'lingmo.preview_file').length, beforeFallback + 1);
    assert.equal(calls.filter(call => call.action === 'lingmo.preview_file').at(-1)?.handoffMode, 'catalog-only');
    assert.ok(nodes.some(node => node.props.role === 'status' && content(node).includes('不继承未验证包')));
    assert.equal(nodes.some(node => content(node).includes('完整接续包已核验')), false);
    nodes.find(node => node.type === 'button' && content(node) === '导入并打开项目')!.props.onClick(); await settle();
    assert.equal(calls.filter(call => call.action === 'lingmo.create_project').at(-1)?.handoffMode, 'catalog-only');
    assert.equal(calls.filter(call => call.action === 'lingmo.create_project').at(-1)?.previewToken, 'catalog-only-token');
    finishImport(workspace); await settle();
    assert.ok(calls.every(call => ['getExecutionPolicy', 'files.choose', 'lingmo.preview_file', 'lingmo.preview', 'lingmo.create_project', 'lingmo.import'].includes(call.action)), 'no generation or preliminary empty-project action');
  } finally { view.unmount(); await server.close(); }
});

test('native handoff attachments are re-previewed and frozen into the same import request', async () => {
  const calls: any[] = []; let selected = 'C:/source/adopted.md'; let rejectDirection = false;
  const workspace: Workspace = { project: { id: 'imported', title: '接续项目' }, assets: [], media: [], units: [], jobs: [], settings: {}, providers: [], rules: [] };
  const policy = { mode: 'standard_native', reasons: [], liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', allowedActions: rustStandardActionCatalogue() };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { invoke: async (_: string, args: { request: any }) => {
    const input = args.request; calls.push(input);
    if (input.action === 'getExecutionPolicy') return policy;
    if (input.action === 'files.choose') return [selected];
    if (input.action === 'lingmo.preview_file') {
      if (rejectDirection && input.module22DirectionPath) throw Error('方向附件与所选包不一致');
      return { counts: { episodes: 1, scenes: 1, fragments: 1, shots: 3, assets: 2 }, markdownSha256: 'same-md', previewToken: JSON.stringify([input.module22Root || '', input.module22DirectionPath || '', input.handoffMode]), diagnostics: [], catalogMatched: true };
    }
    if (input.action === 'lingmo.create_project') return workspace;
    throw Error('Unexpected action: ' + input.action);
  } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const view = hookRenderer();
  try {
    const { LingmoImportDialog } = await server.ssrLoadModule('/src/LingmoImportDialog.tsx');
    const props = { writable: true, onImported() {}, onClose() {} };
    const element = React.createElement(LingmoImportDialog, props);
    const render = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await render();
    nodes.find(node => node.type === 'button' && content(node).includes('选择分镜 MD 文件'))!.props.onClick(); nodes = await render(); nodes = await render();
    assert.equal(calls.filter(c => c.action === 'lingmo.preview_file').at(-1)?.module22Root, undefined, 'MD-only has no mandatory source or package');
    selected = 'C:/source/完整接续包';
    nodes.find(node => node.type === 'button' && content(node) === '完整接续资料文件夹')!.props.onClick(); nodes = await render(); nodes = await render();
    assert.equal(calls.filter(c => c.action === 'lingmo.preview_file').at(-1)?.module22Root, selected);
    selected = 'C:/source/完整接续包/方向.md'; rejectDirection = true;
    nodes.find(node => node.type === 'button' && content(node) === '制作方向文件（可选）')!.props.onClick(); nodes = await render(); nodes = await render();
    assert.equal(nodes.find(node => node.type === 'button' && content(node) === '导入并打开项目')!.props.disabled, true, 'an attachment error invalidates the old preview');
    nodes.find(node => node.type === 'button' && content(node) === '移除方向文件')!.props.onClick(); nodes = await render(); nodes = await render();
    assert.equal(calls.filter(c => c.action === 'lingmo.preview_file').at(-1)?.module22DirectionPath, undefined);
    rejectDirection = false;
    nodes.find(node => node.type === 'button' && content(node) === '制作方向文件（可选）')!.props.onClick(); nodes = await render(); nodes = await render();
    const latest = calls.filter(c => c.action === 'lingmo.preview_file').at(-1)!;
    nodes.find(node => node.type === 'button' && content(node) === '导入并打开项目')!.props.onClick(); await settle();
    const imported = calls.find(c => c.action === 'lingmo.create_project');
    assert.equal(imported.module22Root, latest.module22Root); assert.equal(imported.module22DirectionPath, latest.module22DirectionPath);
    assert.equal(imported.previewToken, JSON.stringify([latest.module22Root, latest.module22DirectionPath, latest.handoffMode]));
    assert.ok(calls.every(c => ['getExecutionPolicy', 'files.choose', 'lingmo.preview_file', 'lingmo.create_project'].includes(c.action)), 'attachments never start generation');
  } finally { view.unmount(); await server.close(); }
});
