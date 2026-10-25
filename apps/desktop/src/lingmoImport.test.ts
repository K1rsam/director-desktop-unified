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
      if (failPreview) throw new Error('文档不可读');
      return { counts, markdownSha256: 'reviewed-sha', previewToken: 'catalog-bound-token', diagnostics: [], catalogMatched: true };
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
    assert.ok(calls.every(call => ['getExecutionPolicy', 'files.choose', 'lingmo.preview_file', 'lingmo.preview', 'lingmo.create_project', 'lingmo.import'].includes(call.action)), 'no generation or preliminary empty-project action');
  } finally { view.unmount(); await server.close(); }
});
