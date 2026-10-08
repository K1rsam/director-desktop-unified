import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

let networkRequests = 0;
Object.assign(globalThis, { location: { search: '?preview=1' }, fetch: async () => { networkRequests++; throw Error('Real network is forbidden'); } });
const shared = globalThis as typeof globalThis & { __promptHooks?: HookHost; __promptApi?: (value: any) => Promise<any> };
class HookHost {
  cells: any[] = []; cursor = 0; effects: (() => void)[] = [];
  useState(initial: any) { const i = this.cursor++; if (!(i in this.cells)) this.cells[i] = typeof initial === 'function' ? initial() : initial; return [this.cells[i], (next: any) => { this.cells[i] = typeof next === 'function' ? next(this.cells[i]) : next; }]; }
  useEffect(callback: () => void | (() => void), dependencies: any[]) { const i = this.cursor++, old = this.cells[i]; if (!old || dependencies.some((value, n) => value !== old.dependencies[n])) this.effects.push(() => { old?.cleanup?.(); this.cells[i].cleanup = callback(); }); this.cells[i] = { dependencies, cleanup: old?.cleanup }; }
  render(Component: any, props: any) { this.cursor = 0; this.effects = []; shared.__promptHooks = this; return Component(props); }
  async flush() { for (const effect of this.effects) effect(); this.effects = []; await new Promise(done => setImmediate(done)); }
}
let replacements = 0;
const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false }, appType: 'custom', plugins: [{ name: 'asset-prompt-isolated-hooks', enforce: 'pre', transform(code, id) {
  if (!id.replaceAll('\\', '/').split('?')[0].endsWith('/src/AssetPanel.tsx')) return;
  for (const [from, to] of [["import { useEffect, useState } from 'react';", "import { useEffect, useState } from '/@prompt-hooks';"], ["import { choose, request } from './api';", "import { choose, request } from '/@prompt-api';"]]) {
    assert.equal(code.split(from).length, 2, 'The actual AssetPanel hook/API import must be replaced exactly once');
    code = code.replace(from, to);
  }
  replacements++; return code;
}, resolveId(id) { if (id === '/@prompt-hooks' || id === '/@prompt-api') return '\0' + id; }, load(id) {
  if (id === '\0/@prompt-hooks') return ['useState', 'useEffect'].map(name => `export function ${name}(...args) { return globalThis.__promptHooks.${name}(...args); }`).join('\n');
  if (id === '\0/@prompt-api') return 'export async function request(value) { return globalThis.__promptApi(value); } export async function choose() { throw Error("Media selection is forbidden"); }';
} }] });
const { AssetPanel } = await server.ssrLoadModule('/src/AssetPanel.tsx');
assert.equal(replacements, 1);
const { installExecutionPolicy, validateExecutionPolicy, OFFLINE_EXECUTION_POLICY } = await server.ssrLoadModule('/src/executionPolicy.ts');
const standard = validateExecutionPolicy({ version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', allowedActions: ['images.design'], reasons: ['Isolated prompt UI test only'] });
installExecutionPolicy(standard);
after(async () => { assert.equal(networkRequests, 0); delete shared.__promptHooks; delete shared.__promptApi; await server.close(); });

function nodes(tree: any): any[] { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(nodes)]; }
function text(tree: any): string { return typeof tree === 'string' || typeof tree === 'number' ? String(tree) : React.Children.toArray(tree?.props?.children).map(text).join(''); }
function button(tree: any, label: string) { const node = nodes(tree).find(node => node.type?.name === 'Button' && text(node) === label); assert.ok(node, label); return node.props; }
function fixture() {
  return { project: { id: 'isolated-prompt-ui', title: '合成提示词按钮核验' }, source: { text: '第一集\n场景：门前\n甲走到门口。', version: 1, sha256: 'synthetic-source', episodes: [{ id: 'e1', title: '第一集' }], scenes: [], rows: [] }, assets: [
    { id: 'child-design', name: '衣装子形象', kind: 'CH', variantId: 'casual', sourceAssetId: 'parent-design', version: 2, prompt: '已有设计正文。', preparation: { mode: 'REFERENCE' } },
    { id: 'parent-design', name: '基础形象', kind: 'CH', variantId: 'base', version: 3, prompt: '已采用父设计。', preparation: { mode: 'REFERENCE' } },
    { id: 'inline-design', name: '随镜表现', kind: 'PROP', variantId: 'inline', version: 1, prompt: '', preparation: { mode: 'INLINE' } },
  ], media: [], units: [], jobs: [] as any[], settings: {}, providers: [], rules: [] };
}
async function view(options: { writable?: boolean; prompt?: string; mode?: string; jobs?: any[]; detail?: 'missing' | 'stale' } = {}) {
  const workspace = fixture(), calls: { action: string; payload: any }[] = [], reads: any[] = [];
  if (options.prompt !== undefined) workspace.assets[0].prompt = options.prompt;
  if (options.mode) workspace.assets[0].preparation.mode = options.mode;
  if (options.jobs) workspace.jobs = options.jobs;
  shared.__promptApi = async value => { reads.push(value); assert.equal(value.action, 'assets.get'); const asset = structuredClone(workspace.assets.find(asset => asset.id === value.assetId)); if (options.detail === 'missing') throw Error('Synthetic missing detail'); if (asset && options.detail === 'stale') asset.version--; return asset; };
  const props = { workspace, writable: options.writable ?? true, mutate: async (action: string, payload: any) => { calls.push({ action, payload }); return workspace; }, inspect() {}, onNotice() {}, onError(message: string) { throw Error(message); }, onTasks() {} };
  const host = new HookHost(); const render = () => host.render(AssetPanel, props);
  render(); await host.flush();
  return { workspace, calls, reads, host, render, async click(label: string) { const action = button(render(), label); assert.equal(action.disabled, false, label); action.onClick(); await new Promise(done => setImmediate(done)); } };
}

test('单项重新设计只传当前子形象，不传父资产、不保存图片设置或生成媒体', async () => {
  const ui = await view(), action = button(ui.render(), '重新设计提示词');
  assert.equal(action.executionAction, 'images.design');
  await ui.click('重新设计提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { assetIds: ['child-design'] } }]);
  assert.deepEqual(ui.reads, [{ action: 'assets.get', projectId: ui.workspace.project.id, assetId: 'child-design' }]);
});
test('尚无设计正文时显示设计提示词并走同一个精确单项入口', async () => {
  const ui = await view({ prompt: '' }); await ui.click('设计提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { assetIds: ['child-design'] } }]);
});
test('只读、未读完整详情、版本漂移、运行中及无需独立图片形象不能单项设计', async () => {
  for (const options of [{ writable: false }, { detail: 'missing' as const }, { detail: 'stale' as const }, { jobs: [{ id: 'designing', kind: 'workflow', provider: 'local', entityId: 'images.design:', status: 'running' }] }, { mode: 'INLINE' }, { mode: 'AUDIO' }, { mode: 'UNRESOLVED' }]) {
    const ui = await view(options), action = button(ui.render(), '重新设计提示词');
    assert.equal(action.disabled, true); action.onClick(); await new Promise(done => setImmediate(done)); assert.equal(ui.calls.length, 0);
  }
});
test('未保存正文阻止单项和顶层设计，保留正文编辑与最终请求文案', async () => {
  const ui = await view();
  nodes(ui.render()).find(node => node.type === 'textarea' && node.props.className === 'asset-prompt-input').props.onChange({ target: { value: '尚未保存的编辑正文。' } });
  const tree = ui.render();
  for (const label of ['重新设计提示词', '一键设计提示词']) { const action = button(tree, label); assert.equal(action.disabled, true); action.onClick(); }
  assert.equal(ui.calls.length, 0); assert.equal(button(tree, '保存提示词').disabled, false);
  assert.ok(nodes(tree).some(node => node.type === 'textarea' && node.props.value === '尚未保存的编辑正文。'));
  assert.ok(button(tree, '复制设计正文')); assert.ok(button(tree, '复制最终请求'));
});
test('批量设计按当前选择只发子形象，父依赖由后端处理', async () => {
  const ui = await view(); await ui.click('批量选择');
  nodes(ui.render()).find(node => node.type === 'input' && node.props['aria-label'] === '批量选择 基础形象').props.onChange();
  await ui.click('设计所选提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { assetIds: ['child-design'] } }]);
});
test('批量空选及仅包含INLINE的选择均不能退回全目录设计', async () => {
  const ui = await view(); await ui.click('批量选择'); await ui.click('清空');
  for (const inlineOnly of [false, true]) {
    if (inlineOnly) nodes(ui.render()).find(node => node.type === 'input' && node.props['aria-label'] === '批量选择 随镜表现').props.onChange();
    const action = button(ui.render(), '设计所选提示词'); assert.equal(action.disabled, true); action.onClick();
  }
  assert.equal(ui.calls.length, 0);
});
test('非批量仍保留全目录一键设计且仅调用提示词设计', async () => {
  const ui = await view(); await ui.click('一键设计提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: {} }]);
});
test('单项及批量设计继续受实际executionAction离线门禁约束', async () => {
  const ui = await view();
  try {
    installExecutionPolicy(OFFLINE_EXECUTION_POLICY);
    for (const batch of [false, true]) {
      if (batch) button(ui.render(), '批量选择').onClick();
      const html = renderToStaticMarkup(ui.render());
      for (const label of ['重新设计提示词', batch ? '设计所选提示词' : '一键设计提示词']) {
        const markup = [...html.matchAll(/<button\b[^>]*>[\s\S]*?<\/button>/g)].map(match => match[0]).find(value => value.replace(/<[^>]+>/g, '') === label);
        assert.ok(markup?.includes('disabled=""')); assert.ok(markup?.includes('data-execution-action="images.design"'));
      }
    }
    assert.equal(ui.calls.length, 0);
  } finally { installExecutionPolicy(standard); }
});
