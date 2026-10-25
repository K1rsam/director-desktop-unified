import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import type { Workspace } from './types.ts';

test('storyboard delivery export is explicit, scope-bound and coalesced without invoking text or media generation', async () => {
  const calls: any[] = [], notices: string[] = [], errors: string[] = [];
  let releaseExport: (value: unknown) => void = () => {};
  const unit = { id: 'u1', episodeId: 'e1', title: '已采用合成稿', prompt: '原话和分镜保留。', version: 3 };
  const workspace: Workspace = { project: { id: 'p', title: '合成' }, source: { version: 1, episodes: [{ id: 'e1', title: '第一集' }], rows: [] }, units: [unit], assets: [], media: [], jobs: [], settings: {}, providers: [], rules: [] };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { invoke: async (_: string, args: { request: any }) => {
    const input = args.request; calls.push(input);
    if (input.action === 'getExecutionPolicy') return { mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', allowedActions: ['storyboard.exportDelivery', 'units.get', 'files.choose'], reasons: [] };
    if (input.action === 'units.get') return unit;
    if (input.action === 'files.choose') return ['C:/synthetic-export'];
    if (input.action === 'storyboard.exportDelivery') return new Promise(resolve => { releaseExport = resolve; });
    throw Error('Unexpected action: ' + input.action);
  } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const view = hookRenderer();
  try {
    const { Stage3Panel } = await server.ssrLoadModule('/src/Stage3Panel.tsx');
    const element = React.createElement(Stage3Panel, { workspace, writable: true, mutate: async () => { throw Error('Export must not use generation/workspace mutations'); }, onNotice: (message: string) => notices.push(message), onError: (message: string) => errors.push(message) });
    view.render(element); await settle(); let nodes = view.render(element); await settle(); nodes = view.render(element);
    const button = nodes.find(node => node.type === 'button' && content(node) === '导出分镜与接续资料');
    assert.ok(button); assert.equal(button.props.disabled, false);
    button.props.onClick(); button.props.onClick(); await settle(); await settle();
    assert.equal(calls.filter(input => input.action === 'storyboard.exportDelivery').length, 1);
    assert.deepEqual(calls.find(input => input.action === 'storyboard.exportDelivery'), { action: 'storyboard.exportDelivery', projectId: 'p', path: 'C:/synthetic-export', episodeIds: ['e1'] });
    assert.equal(notices.length, 0, 'do not claim export completion before the actual core receipt');
    releaseExport({ path: 'C:/synthetic-export/delivery', count: 1, manifestPath: 'C:/synthetic-export/delivery/manifest.json' }); await settle();
    assert.equal(notices.length, 1); assert.match(notices[0], /1 集同版分镜/); assert.equal(errors.length, 0);
    assert.ok(calls.every(input => ['getExecutionPolicy', 'files.choose', 'units.get', 'storyboard.exportDelivery'].includes(input.action)));
  } finally { view.unmount(); await server.close(); }
});
