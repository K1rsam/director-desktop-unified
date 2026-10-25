import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import type { Host } from './test-support/hookRenderer.ts';
import type { AdoptedImageExportProps } from './AdoptedImageExport.tsx';
import type { Workspace } from './types.ts';

const issue = (assetId: string, message: string) => ({ assetId, variantId: 'base', name: `资产 ${assetId}`, importName: `${assetId}CH`, filename: `${assetId}CH.png`, reason: 'missing_adoption', message });
function receipt() {
  return { version: 'desktop-current-image-export/v1', path: 'C:/synthetic-export/package', imageDirectory: 'C:/synthetic-export/package/图片', manifestPath: 'C:/synthetic-export/package/manifest.json',
    status: 'partial', scopeCount: 3, count: 1, missingCount: 1, excludedCount: 1, missing: [issue('missing', '尚无正式采用图片')], excluded: [issue('reference', '仅供母版参考，不进入正式交付')], generationSubmissions: 0, videoSubmissions: 0 };
}
function fixture(): Workspace {
  return { project: { id: 'image-export-project', title: '合成导出核验' }, source: null, assets: [
    { id: 'formal', variantId: 'base', name: '正式范围', kind: 'CH' },
    { id: 'reference', variantId: 'base', name: '仅参考母版', kind: 'CH' },
  ], media: [], units: [], jobs: [], settings: {}, providers: [], rules: [], imageProductionScope: { assetIds: ['formal'], referenceOnlyIds: ['reference'] } };
}
const findButton = (nodes: Host[], label = '导出已采用图片') => { const button = nodes.find(node => node.type === 'button' && content(node) === label); assert.ok(button, label); return button; };
async function withExport(run: (ui: Awaited<ReturnType<typeof exporter>>) => Promise<void>) {
  const ui = await exporter();
  try { await run(ui); } finally { ui.view.unmount(); await ui.server.close(); }
}
async function exporter() {
  const calls: any[] = [], notices: string[] = [], errors: string[] = [];
  const handlers = { choose: async (): Promise<string[]> => ['C:/synthetic-export'], export: async (): Promise<unknown> => receipt() };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, fetch: async () => { throw Error('Real network forbidden'); }, window: { __TAURI_INTERNALS__: { invoke: async (_: string, args: { request: any }) => {
    const input = args.request; calls.push(input);
    if (input.action === 'files.choose') return handlers.choose();
    if (input.action === 'images.exportCurrent') return handlers.export();
    throw Error('Unexpected action: ' + input.action);
  } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const { AdoptedImageExport } = await server.ssrLoadModule('/src/AdoptedImageExport.tsx');
  const view = hookRenderer();
  const props: AdoptedImageExportProps = { workspace: fixture(), writable: true, onNotice: message => notices.push(message), onError: message => errors.push(message) };
  const render = () => view.render(React.createElement(AdoptedImageExport, props));
  const click = async () => { findButton(render()).props.onClick(); await settle(); await settle(); return render(); };
  return { server, view, props, handlers, calls, notices, errors, render, click };
}

test('adopted image export snapshots full formal scope once without requiring accounts or passing selections', async () => {
  await withExport(async ui => {
    let finish: (value: unknown) => void = () => {};
    ui.handlers.export = () => new Promise(resolve => { finish = resolve; });
    const button = findButton(ui.render());
    assert.equal(button.props.disabled, false, 'local export must work with no providers, account or image');
    assert.match(button.props.title, /不受当前列表筛选或勾选影响/);
    button.props.onClick(); button.props.onClick(); await settle(); await settle();
    assert.deepEqual(ui.calls, [{ action: 'files.choose', kind: 'directory', multiple: false }, { action: 'images.exportCurrent', projectId: 'image-export-project', path: 'C:/synthetic-export' }]);
    assert.equal(findButton(ui.render(), '正在导出图片…').props.disabled, true);
    assert.deepEqual(ui.notices, [], 'no completion claim until a validated core receipt');
    finish(receipt()); await settle();
    const dialog = ui.render().find(node => node.type === 'dialog'); assert.ok(dialog);
    assert.match(content(dialog), /已部分导出 1\/3 张/);
    assert.match(content(dialog), /资产 missing：尚无正式采用图片/);
    assert.match(content(dialog), /missingCH.png/);
    assert.match(content(dialog), /仅供母版参考，不进入正式交付/);
    assert.match(content(dialog), /C:\/synthetic-export\/package\/图片/);
    assert.match(content(dialog), /C:\/synthetic-export\/package\/manifest.json/);
    assert.equal(ui.notices.length, 1); assert.deepEqual(ui.errors, []);
  });
});

test('complete and zero-ready exports have distinct truthful results', async () => {
  await withExport(async ui => {
    ui.handlers.export = async () => ({ ...receipt(), status: 'complete', scopeCount: 2, count: 2, missingCount: 0, excludedCount: 0, missing: [], excluded: [] });
    let nodes = await ui.click(); assert.match(ui.notices[0], /已全部导出，共 2 张/);
    findButton(nodes, '关闭').props.onClick();
    ui.handlers.export = async () => ({ ...receipt(), status: 'no_ready_images', scopeCount: 1, count: 0, excludedCount: 0, excluded: [] });
    nodes = await ui.click();
    assert.match(ui.notices[1], /没有可导出的正式采用图/);
    assert.ok(!ui.notices[1].includes('全部导出'));
    assert.match(content(nodes.find(node => node.type === 'dialog')!), /已保存待补清单/);
    assert.equal(ui.errors.length, 0);
  });
});

test('cancelled folder selection and read-only or reference-only scopes cannot export', async () => {
  await withExport(async ui => {
    ui.handlers.choose = async () => [];
    await ui.click(); assert.deepEqual(ui.calls.map(call => call.action), ['files.choose']);
    assert.deepEqual(ui.notices, []); assert.deepEqual(ui.errors, []);
    ui.calls.length = 0; ui.props.writable = false;
    assert.equal(findButton(ui.render()).props.disabled, true); await ui.click(); assert.deepEqual(ui.calls, []);
    ui.props.writable = true; ui.props.workspace.imageProductionScope!.assetIds = [];
    assert.equal(findButton(ui.render()).props.disabled, true); await ui.click(); assert.deepEqual(ui.calls, []);
  });
});

test('explicit formal scope remains exportable for backend diagnostics even when all items are excluded', async () => {
  await withExport(async ui => {
    ui.props.workspace.assets[0].imageRequired = false;
    ui.handlers.export = async () => ({ ...receipt(), status: 'no_ready_images', scopeCount: 1, count: 0, missingCount: 0, missing: [] });
    assert.equal(findButton(ui.render()).props.disabled, false);
    const nodes = await ui.click(); assert.equal(ui.calls.filter(call => call.action === 'images.exportCurrent').length, 1);
    assert.match(content(nodes.find(node => node.type === 'dialog')!), /已排除 · 1 项/);
  });
});

test('malformed receipts and changed-snapshot errors never claim a successful export', async () => {
  await withExport(async ui => {
    for (const patch of [{ status: 'complete' }, { scopeCount: 5 }, { missing: [] }, { imageDirectory: '' }, { generationSubmissions: 1 }, { count: -1 }, { excluded: [{ ...issue('r', 'reference'), name: {} }] }]) {
      ui.handlers.export = async () => ({ ...receipt(), ...patch });
      const nodes = await ui.click(); assert.ok(!nodes.some(node => node.type === 'dialog'));
    }
    assert.equal(ui.errors.length, 7); assert.equal(ui.notices.length, 0);
    ui.handlers.export = async () => { throw Error('导出期间正式采用图已更新，请重新导出。'); };
    await ui.click(); assert.match(ui.errors.at(-1)!, /正式采用图已更新/); assert.equal(ui.notices.length, 0);
    ui.handlers.export = async () => receipt(); await ui.click(); assert.equal(ui.notices.length, 1, 'failure releases the local click guard');
  });
});

test('leaving the workspace during folder selection does not dispatch a stale export', async () => {
  await withExport(async ui => {
    let select: (paths: string[]) => void = () => {};
    ui.handlers.choose = () => new Promise(resolve => { select = resolve; });
    findButton(ui.render()).props.onClick(); await settle(); ui.view.unmount();
    select(['C:/synthetic-export']); await settle(); await settle();
    assert.deepEqual(ui.calls.map(call => call.action), ['files.choose']); assert.deepEqual(ui.notices, []); assert.deepEqual(ui.errors, []);
  });
});

test('leaving after dispatch preserves the backend operation without showing results in another workspace', async () => {
  await withExport(async ui => {
    let finish: (value: unknown) => void = () => {};
    ui.handlers.export = () => new Promise(resolve => { finish = resolve; });
    findButton(ui.render()).props.onClick(); await settle(); await settle(); ui.view.unmount();
    finish(receipt()); await settle(); await settle();
    assert.equal(ui.calls.filter(call => call.action === 'images.exportCurrent').length, 1); assert.deepEqual(ui.notices, []); assert.deepEqual(ui.errors, []);
  });
});
