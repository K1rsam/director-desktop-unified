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
const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom', plugins: [{ name: 'asset-prompt-isolated-hooks', enforce: 'pre', transform(code, id) {
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
async function view(options: { writable?: boolean; prompt?: string; mode?: string; jobs?: any[]; detail?: 'missing' | 'stale'; imageReady?: boolean; imageProvider?: string; afterMutation?: (workspace: any) => void } = {}) {
  const workspace = fixture(), calls: { action: string; payload: any }[] = [], reads: any[] = [], notices: string[] = [];
  if (options.prompt !== undefined) workspace.assets[0].prompt = options.prompt;
  if (options.mode) workspace.assets[0].preparation.mode = options.mode;
  if (options.jobs) workspace.jobs = options.jobs;
  if (options.imageReady) Object.assign(workspace, { settings: { imageProvider: options.imageProvider || 'bh', imageModel: 'synthetic-image', imageResolution: '2k', ratio: '16:9', bhAccountId: 'offline-account', trialApproved: true }, providers: [{ id: options.imageProvider || 'bh', models: [{ id: 'synthetic-image', kind: 'image', adapted: true, ratios: ['16:9', '1:1'], resolutions: ['2k'] }] }] });
  shared.__promptApi = async value => { reads.push(value); assert.equal(value.action, 'assets.get'); const asset = structuredClone(workspace.assets.find(asset => asset.id === value.assetId)); if (options.detail === 'missing') throw Error('Synthetic missing detail'); if (asset && options.detail === 'stale') asset.version--; return asset; };
  const props = { workspace, writable: options.writable ?? true, mutate: async (action: string, payload: any) => { calls.push({ action, payload }); options.afterMutation?.(workspace); return workspace; }, inspect() {}, onNotice(message: string) { notices.push(message); }, onError(message: string) { throw Error(message); }, onTasks() {} };
  const host = new HookHost(); const render = () => host.render(AssetPanel, props);
  render(); await host.flush();
  return { workspace, calls, reads, notices, host, render, async click(label: string) { const action = button(render(), label); assert.equal(action.disabled, false, label); action.onClick(); await new Promise(done => setImmediate(done)); } };
}

test('单项重新设计只传当前子形象，不传父资产、不保存图片设置或生成媒体', async () => {
  const ui = await view(), action = button(ui.render(), '重写当前提示词');
  assert.equal(action.executionAction, 'images.design');
  await ui.click('重写当前提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { workflowProfile: 'desktop-series672-assets112/v1', assetIds: ['child-design'] } }]);
  assert.deepEqual(ui.reads, [{ action: 'assets.get', projectId: ui.workspace.project.id, assetId: 'child-design' }]);
});
test('尚无设计正文时显示设计提示词并走同一个精确单项入口', async () => {
  const ui = await view({ prompt: '' }); await ui.click('编写当前提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { workflowProfile: 'desktop-series672-assets112/v1', assetIds: ['child-design'] } }]);
});
test('只读、未读完整详情、版本漂移、运行中及无需独立图片形象不能单项设计', async () => {
  for (const options of [{ writable: false }, { detail: 'missing' as const }, { detail: 'stale' as const }, { jobs: [{ id: 'designing', kind: 'workflow', provider: 'local', entityId: 'images.design:', status: 'running' }] }, { mode: 'INLINE' }, { mode: 'AUDIO' }, { mode: 'UNRESOLVED' }]) {
    const ui = await view(options), action = button(ui.render(), '重写当前提示词');
    assert.equal(action.disabled, true); action.onClick(); await new Promise(done => setImmediate(done)); assert.equal(ui.calls.length, 0);
  }
});
test('未保存正文阻止单项和顶层设计，保留正文编辑与最终请求文案', async () => {
  const ui = await view();
  nodes(ui.render()).find(node => node.type === 'textarea' && node.props.className === 'asset-prompt-input').props.onChange({ target: { value: '尚未保存的编辑正文。' } });
  const tree = ui.render();
  for (const label of ['重写当前提示词', '编写全部提示词']) { const action = button(tree, label); assert.equal(action.disabled, true); action.onClick(); }
  assert.equal(ui.calls.length, 0); assert.equal(button(tree, '保存提示词').disabled, false);
  assert.ok(nodes(tree).some(node => node.type === 'textarea' && node.props.value === '尚未保存的编辑正文。'));
  assert.ok(button(tree, '复制提示词')); assert.ok(button(tree, '复制请求预览'));
});
test('批量设计按当前选择只发子形象，父依赖由后端处理', async () => {
  const ui = await view(); await ui.click('批量选择');
  nodes(ui.render()).find(node => node.type === 'input' && node.props['aria-label'] === '批量选择 基础形象').props.onChange();
  await ui.click('编写所选提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { workflowProfile: 'desktop-series672-assets112/v1', assetIds: ['child-design'] } }]);
});

test('第三阶段保持一个主操作，单项与批量出图仍可见且不会混进写词动作', async () => {
  const ui = await view({ imageReady: true });
  const primary = () => nodes(ui.render()).filter(node => node.type?.name === 'Button' && node.props.variant === 'primary').map(text);
  assert.deepEqual(primary(), ['编写全部提示词']);
  const imageExport = nodes(ui.render()).find(node => node.type?.name === 'AdoptedImageExport');
  assert.ok(imageExport, 'current adopted-image export remains visible beside image production');
  assert.equal(imageExport.props.workspace, ui.workspace);
  assert.equal(imageExport.props.writable, true);
  assert.ok(button(ui.render(), '生成就绪图片 · 2'));
  assert.ok(button(ui.render(), '生成图片'));
  await ui.click('批量选择');
  assert.deepEqual(primary(), ['编写所选提示词']);
  assert.ok(button(ui.render(), '生成所选图片'));
  assert.equal(ui.calls.length, 0);
});

test('正式与已登记参考一起批量写词和出图，未登记项可读但不混进派发范围', async () => {
  const ui = await view({ imageReady: true });
  Object.assign(ui.workspace, { imageProductionScope: { assetIds: ['child-design'], referenceOnlyIds: ['parent-design'] } });
  ui.workspace.assets.push({ id: 'outside', name: '范围外祖先', kind: 'CH', variantId: 'outside', version: 1, prompt: '已有祖先词', preparation: { mode: 'REFERENCE' } } as any);
  assert.match(text(ui.render()), /1 项正式图片 ＋ 1 项已登记参考母版（提示词 1\/1）/);
  const readyCost = nodes(ui.render()).find(node => node.type?.name === 'ImageCostSummary' && node.props.summaryLabel === '就绪图片总估算');
  assert.deepEqual(readyCost.props.ids, ['child-design', 'parent-design'], 'the global-ready fee preview covers every asset sent by that action');
  await ui.click('编写全部提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { workflowProfile: 'desktop-series672-assets112/v1', assetIds: ['child-design', 'parent-design'] } }]);
  ui.calls.length = 0; await ui.click('批量选择');
  const tree = ui.render();
  assert.match(text(tree), /已选 2 项 · 提示词 2\/2/);
  assert.equal(nodes(tree).find(node => node.type === 'input' && node.props['aria-label'] === '批量选择 范围外祖先').props.disabled, true);
  await ui.click('生成所选图片');
  assert.deepEqual(ui.calls.map(call => call.action), ['settings.save', 'images.run']);
  assert.deepEqual(ui.calls[1].payload.assetIds, ['child-design', 'parent-design']);
  assert.ok(!ui.calls[1].payload.assetIds.includes('outside'));
});

test('登记母版后进入就绪范围，白色占位不阻止首次真实出图', async () => {
  const ui = await view({ imageReady: true });
  Object.assign(ui.workspace, { imageProductionScope: { assetIds: ['child-design'], referenceOnlyIds: [] } });
  Object.assign(ui.workspace.assets[0], { referenceRequirements: [{ targetAssetId: 'parent-design', role: 'face' }] });
  Object.assign(ui.workspace.assets[1], { adoptedMediaId: 'white' });
  Object.assign(ui.workspace, { media: [{ id: 'white', assetId: 'parent-design', variantId: 'base', kind: 'image', path: 'synthetic-white.png', placeholder: true }] });
  assert.equal(button(ui.render(), '生成就绪图片 · 0').disabled, true);
  (ui.workspace as any).imageProductionScope.referenceOnlyIds = ['parent-design'];
  assert.equal(button(ui.render(), '生成就绪图片 · 1').disabled, false);
  await ui.click('生成就绪图片 · 1');
  assert.deepEqual(ui.calls[1].payload.assetIds, ['parent-design']);
  assert.equal(ui.calls[1].payload.onlyReady, true);
});

test('范围外母版查看清除过滤并进入真实采用图的锁脸编辑入口', async () => {
  const ui = await view();
  Object.assign(ui.workspace, { imageProductionScope: { assetIds: ['child-design'], referenceOnlyIds: [] } });
  Object.assign(ui.workspace.assets[0], { referenceRequirements: [{ targetAssetId: 'parent-design', role: 'face' }] });
  Object.assign(ui.workspace.assets[1], { adoptedMediaId: 'real-mother' });
  Object.assign(ui.workspace, { media: [{ id: 'real-mother', assetId: 'parent-design', variantId: 'base', kind: 'image', path: 'data:image/png;base64,c3ludGhldGlj', sha256: 'a'.repeat(64) }] });
  nodes(ui.render()).find(node => node.type === 'input' && node.props['aria-label'] === '搜索资产').props.onChange({ target: { value: '不存在的搜索结果' } });
  const reference = nodes(ui.render()).find(node => node.type?.name === 'ImageReferenceRegistration'); assert.ok(reference);
  reference.props.onSelect('parent-design');
  ui.render(); await ui.host.flush(); const tree = ui.render();
  assert.equal(nodes(tree).find(node => node.type === 'input' && node.props['aria-label'] === '搜索资产').props.value, '');
  const editor = nodes(tree).find(node => node.type?.name === 'IdentityLayoutEditor');
  assert.equal(editor.props.asset.id, 'parent-design');
  assert.equal(editor.props.media.id, 'real-mother');
  assert.equal(editor.props.writable, true, 'loading full detail leaves real adopted-image inspection reachable');
  assert.equal(button(tree, '重写当前提示词').disabled, true, 'an unregistered reference can be inspected without silently authorizing regeneration');
  assert.equal(button(tree, '重新生成图片').disabled, true);
  assert.match(text(tree), /此资产不在本次制作范围/);
  assert.deepEqual(ui.calls, []);
});
test('批量空选及仅包含INLINE的选择均不能退回全目录设计', async () => {
  const ui = await view(); await ui.click('批量选择'); await ui.click('清空');
  for (const inlineOnly of [false, true]) {
    if (inlineOnly) nodes(ui.render()).find(node => node.type === 'input' && node.props['aria-label'] === '批量选择 随镜表现').props.onChange();
    const action = button(ui.render(), '编写所选提示词'); assert.equal(action.disabled, true); action.onClick();
  }
  assert.equal(ui.calls.length, 0);
});
test('非批量仍保留全目录一键设计且仅调用提示词设计', async () => {
  const ui = await view(); await ui.click('编写全部提示词');
  assert.deepEqual(ui.calls, [{ action: 'images.design', payload: { workflowProfile: 'desktop-series672-assets112/v1' } }]);
});
test('单项及批量设计继续受实际executionAction离线门禁约束', async () => {
  const ui = await view();
  try {
    installExecutionPolicy(OFFLINE_EXECUTION_POLICY);
    for (const batch of [false, true]) {
      if (batch) button(ui.render(), '批量选择').onClick();
      const html = renderToStaticMarkup(ui.render());
      for (const label of ['重写当前提示词', batch ? '编写所选提示词' : '编写全部提示词']) {
        const markup = [...html.matchAll(/<button\b[^>]*>[\s\S]*?<\/button>/g)].map(match => match[0]).find(value => value.replace(/<[^>]+>/g, '') === label);
        assert.ok(markup?.includes('disabled=""')); assert.ok(markup?.includes('data-execution-action="images.design"'));
      }
    }
    assert.equal(ui.calls.length, 0);
  } finally { installExecutionPolicy(standard); }
});

test('其他阶段占用项目只阻止文字设计，仍可阅读和使用独立图片队列', async () => {
  const ui = await view({ imageReady: true });
  assert.equal(button(ui.render(), '生成就绪图片 · 2').disabled, false);
  for (const entityId of ['foundation.run', 'storyboard.run', 'storyboard.refine']) {
    ui.workspace.jobs = [{ id: 'busy-project', kind: 'workflow', provider: 'local', entityId, status: 'running' }];
    for (const label of ['重写当前提示词', '编写全部提示词']) {
      const action = button(ui.render(), label); assert.equal(action.disabled, true); action.onClick(); await new Promise(done => setImmediate(done));
    }
    assert.equal(button(ui.render(), '复制提示词').disabled, false);
    assert.equal(button(ui.render(), '生成图片').disabled, false);
    assert.equal(button(ui.render(), '生成就绪图片 · 2').disabled, false);
    assert.deepEqual(ui.calls, []);
  }
  ui.workspace.jobs = [];
  await ui.click('生成就绪图片 · 2');
  assert.deepEqual(ui.calls.map(call => call.action), ['settings.save', 'images.run']);
  assert.deepEqual(ui.calls[1].payload.assetIds, ['child-design', 'parent-design']);
  assert.equal(ui.calls[1].payload.onlyReady, true);
  assert.equal(ui.calls[1].payload.imageAdoptionPolicy, 'manual');
  assert.equal(ui.calls[0].payload.settings.ratio, undefined, 'image settings must not overwrite video ratio');
});

test('Chrome and BH image actions use authoritative credential readiness without requiring an account label', async () => {
  for (const provider of ['chrome', 'bh']) {
    const ui = await view({ imageReady: true, imageProvider: provider });
    Object.assign(ui.workspace.settings, { imageProvider: provider, [`${provider}AccountId`]: '' });
    (ui.workspace as any).credentialStatus = { [provider]: { configured: true } };
    let tree = ui.render();
    assert.equal(button(tree, '生成图片').disabled, false);
    assert.equal(button(tree, '生成就绪图片 · 2').disabled, false);
    assert.ok(!text(tree).includes('稳定账户标签'));
    await ui.click('生成图片');
    assert.deepEqual(ui.calls.map(call => call.action), ['settings.save', 'images.run']);
    assert.equal(ui.calls[1].payload.provider, provider);
    ui.calls.length = 0;
    (ui.workspace as any).credentialStatus[provider].configured = false;
    Object.assign(ui.workspace.settings, { [`${provider}AccountId`]: 'stale-label' });
    tree = ui.render();
    for (const label of ['生成图片', '生成就绪图片 · 2']) {
      const action = button(tree, label); assert.equal(action.disabled, true); action.onClick();
    }
    await new Promise(done => setImmediate(done));
    assert.deepEqual(ui.calls, [], 'a stale label cannot override missing current credentials');
    assert.ok(text(tree).includes(provider === 'chrome' ? '连接本机 Chrome' : 'BH API 地址和凭据'));
  }
});

test('新版提示词编辑只修改imagePrompt，完整采用设计和旧手写正文保持原样', async () => {
  const ui = await view();
  Object.assign(ui.workspace.assets[0], { adoptedDesign: { full: '服装结构、动作条件及源事实完整设计', revision: 4 }, imagePrompt: { text: '新版七视图提示词', ratio: '1:1', basedOnDesignRevision: 4 }, version: 5 });
  ui.render(); await ui.host.flush();
  let tree = ui.render();
  assert.ok(text(tree).includes('服装结构、动作条件及源事实完整设计'));
  nodes(tree).find(node => node.type === 'textarea' && node.props.className === 'asset-prompt-input').props.onChange({ target: { value: '用户只调整图片材质说明' } });
  await ui.click('保存提示词');
  assert.equal(ui.calls.length, 1);
  const saved = ui.calls[0].payload.asset;
  assert.equal(saved.imagePrompt.text, '用户只调整图片材质说明');
  assert.equal(saved.prompt, '已有设计正文。');
  assert.equal(saved.adoptedDesign.full, '服装结构、动作条件及源事实完整设计');
  assert.equal(saved.imagePrompt.basedOnDesignRevision, 4);
  assert.equal(saved.expectedVersion, 5);
});

test('明确母图等待不挡全量写词；就绪按钮只提交独立项且默认不自动采用', async () => {
  const ui = await view({ imageReady: true });
  Object.assign(ui.workspace.assets[0], { referenceRequirements: [{ targetAssetId: 'parent-design', kind: 'face' }], version: 4 });
  ui.render(); await ui.host.flush();
  const tree = ui.render();
  assert.ok(text(tree).includes('等待母版：基础形象'));
  assert.equal(button(tree, '编写全部提示词').disabled, false);
  await ui.click('生成就绪图片 · 1');
  assert.deepEqual(ui.calls.map(call => call.action), ['settings.save', 'images.run']);
  assert.deepEqual(ui.calls[1].payload.assetIds, ['parent-design']);
  assert.equal(ui.calls[1].payload.onlyReady, true);
  assert.equal(ui.calls[1].payload.imageAdoptionPolicy, 'manual');
  assert.deepEqual(ui.calls[1].payload.assetRatios, { 'parent-design': '1:1' });
  assert.equal(ui.calls[0].payload.settings.ratio, undefined);
  assert.ok(!ui.calls.some(call => call.action === 'images.produce'));
});

test('没有真实视觉能力时不能从设置伪造委托审核采用', async () => {
  const ui = await view({ imageReady: true });
  Object.assign(ui.workspace.settings, { imageAdoptionPolicy: 'delegated-review' });
  const policy = nodes(ui.render()).find(node => node.type === 'label' && node.props.className?.includes('asset-review-policy'));
  const control = nodes(policy).find(node => node.type === 'input');
  assert.ok(control); assert.equal(control.props.checked, false); assert.equal(control.props.disabled, true);
  control.props.onChange({ target: { checked: true } }); await new Promise(done => setImmediate(done)); assert.equal(ui.calls.length, 0);
  await ui.click('生成图片');
  assert.equal(ui.calls.find(call => call.action === 'images.run')!.payload.imageAdoptionPolicy, 'manual');
});

test('待复核和无效旧图保持可见，完成数与缺图筛选不再把它们当作有效采用', async () => {
  const ui = await view({ imageReady: true });
  Object.assign(ui.workspace.assets[0], { version: 3, adoptedMediaId: 'old-child', imageStatus: 'needs_review' });
  Object.assign(ui.workspace.assets[1], { adoptedMediaId: 'valid-mother', imageStatus: 'adopted' });
  ui.workspace.assets.push(
    { id: 'rejected', name: '质量退回图', kind: 'SC', variantId: 'base', version: 1, prompt: '原提示词', adoptedMediaId: 'old-rejected', imageStatus: 'rejected', preparation: { mode: 'REFERENCE' } } as any,
    { id: 'invalid', name: '已失效图', kind: 'PROP', variantId: 'base', version: 1, prompt: '原提示词', adoptedMediaId: 'old-invalid', imageStatus: 'invalid', preparation: { mode: 'REFERENCE' } } as any,
  );
  Object.assign(ui.workspace, { media: [
    { id: 'old-child', kind: 'image', assetId: 'child-design', variantId: 'casual', path: 'old-child.png' },
    { id: 'valid-mother', kind: 'image', assetId: 'parent-design', variantId: 'base', path: 'mother.png' },
    { id: 'old-rejected', kind: 'image', assetId: 'rejected', variantId: 'base', path: 'old-rejected.png' },
    { id: 'old-invalid', kind: 'image', assetId: 'invalid', variantId: 'base', path: 'old-invalid.png' },
  ] });
  const before = structuredClone(ui.workspace);
  ui.render(); await ui.host.flush(); let tree = ui.render();
  assert.match(text(tree), /4 项正式图片 · 提示词 4\/4 · 有效采用 1\/4 · 待复核 1 · 待修\/无效 2/);
  assert.match(text(tree), /当前图片待复核，不计入有效采用；原图和历史记录仍保留/);
  assert.ok(nodes(tree).some(node => node.type?.name === 'MediaPreview' && node.props.media?.id === 'old-child'), 'retained media is still inspectable');
  for (const [label, status] of [['待复核', 'needs_review'], ['待修', 'rejected'], ['图片无效', 'invalid']]) {
    const badge = nodes(tree).find(node => node.type?.name === 'Badge' && text(node) === label);
    assert.ok(badge, label); assert.equal(badge.props.status, status);
    assert.ok(!renderToStaticMarkup(badge).includes('badge success'), 'an invalid adoption cannot appear green');
  }
  const filter = nodes(tree).find(node => node.type === 'label' && text(node) === '缺图或待处理');
  nodes(filter).find(node => node.type === 'input').props.onChange({ target: { checked: true } });
  tree = ui.render();
  assert.deepEqual(nodes(tree).filter(node => node.type === 'button' && node.props.className === 'asset-card-open').map(node => node.props['aria-label']), ['选择 衣装子形象', '选择 质量退回图', '选择 已失效图']);
  assert.equal(button(tree, '生成就绪图片 · 0').disabled, true, 'unknown review reasons do not authorize regeneration');
  assert.deepEqual(ui.workspace, before); assert.deepEqual(ui.calls, []);
});

test('就绪批量只重做已确认媒体漂移，设计冲突、质量拒绝、候选与未知任务留在各自处理入口', async () => {
  const ui = await view({ imageReady: true });
  Object.assign(ui.workspace.assets[0], { version: 3, adoptedMediaId: 'old-child', imageStatus: 'needs_review', referenceRequirements: [{ targetAssetId: 'parent-design', targetVariantId: 'base', role: 'appearance' }], imageReviewReasons: [{ kind: 'reference-media-changed', assetId: 'parent-design' }] });
  Object.assign(ui.workspace.assets[1], { adoptedMediaId: 'valid-mother', imageStatus: 'adopted' });
  const additions = [
    { id: 'conflict', name: '身份冲突', imageStatus: 'needs_review', imageReviewReasons: [{ kind: 'own-design-changed', assetId: 'conflict' }], adoptedMediaId: 'old-conflict' },
    { id: 'rejected', name: '待修质量', imageStatus: 'rejected', adoptedMediaId: 'old-rejected' },
    { id: 'candidate', name: '待选候选', imageStatus: 'candidate' },
    { id: 'unknown', name: '结果未知', imageStatus: 'submission_unknown' },
    { id: 'job-unknown', name: '原任务待核对' },
    { id: 'independent', name: '独立场景' },
  ].map(item => ({ kind: 'SC', variantId: 'base', version: 1, prompt: '已保存的完整提示词', preparation: { mode: 'REFERENCE' }, ...item }));
  ui.workspace.assets.push(...additions as any);
  Object.assign(ui.workspace, { media: [
    { id: 'old-child', kind: 'image', assetId: 'child-design', variantId: 'casual', path: 'old-child.png' },
    { id: 'valid-mother', kind: 'image', assetId: 'parent-design', variantId: 'base', path: 'mother.png' },
    { id: 'old-conflict', kind: 'image', assetId: 'conflict', variantId: 'base', path: 'old-conflict.png' },
    { id: 'old-rejected', kind: 'image', assetId: 'rejected', variantId: 'base', path: 'old-rejected.png' },
    { id: 'new-candidate', kind: 'image', assetId: 'candidate', variantId: 'base', path: 'candidate.png' },
  ], jobs: [{ id: 'unknown-original', kind: 'image', assetId: 'job-unknown', provider: 'bh', status: 'submission_unknown' }] });
  const before = structuredClone(ui.workspace);
  ui.render(); await ui.host.flush();
  const cost = nodes(ui.render()).find(node => node.type?.name === 'ImageCostSummary' && node.props.summaryLabel === '就绪图片总估算');
  assert.deepEqual(cost.props.ids, ['child-design', 'independent']);
  await ui.click('生成就绪图片 · 2');
  assert.deepEqual(ui.calls.map(call => call.action), ['settings.save', 'images.run']);
  assert.deepEqual(ui.calls[1].payload.assetIds, ['child-design', 'independent']);
  assert.equal(ui.calls[1].payload.onlyReady, true);
  assert.equal(ui.calls[1].payload.imageAdoptionPolicy, 'manual');
  assert.deepEqual(ui.workspace, before, 'the action preserves old adoption and unresolved receipts');
  ui.calls.length = 0;
  Object.assign(ui.workspace.assets[1], { imageStatus: 'needs_review' });
  await ui.click('生成就绪图片 · 1');
  assert.deepEqual(ui.calls[1].payload.assetIds, ['independent'], 'an invalid mother blocks only its own branch');
});


async function withMediaReview(ui: Awaited<ReturnType<typeof view>>) {
  Object.assign(ui.workspace.assets[0], { version: 3, workflowProfile: 'desktop-series672-assets112/v1', adoptedMediaId: 'old-child', imageStatus: 'needs_review', imageReviewReasons: [{ kind: 'reference-media-changed', assetId: 'parent-design' }] });
  Object.assign(ui.workspace, { media: [{ id: 'old-child', kind: 'image', assetId: 'child-design', variantId: 'casual', path: 'old-child.png' }] });
  ui.render(); await ui.host.flush(); return ui;
}

test('媒体依赖待复核的当前图可按原记录再核对，不保存设置或发起生成', async () => {
  const ui = await withMediaReview(await view());
  const before = structuredClone(ui.workspace);
  assert.match(text(ui.render()), /仅按原生成记录重新核对参考依赖，不生成图片/);
  await ui.click('重新核对当前图');
  assert.deepEqual(ui.calls, [{ action: 'media.adopt', payload: { assetId: 'child-design', variantId: 'casual', mediaId: 'old-child' } }]);
  assert.deepEqual(ui.workspace, before, 'the UI cannot clear the review status itself');
  assert.deepEqual(ui.notices, ['原依赖仍未全部匹配，当前图继续保留待处理；未生成新图。']);
});

test('重新核对的成功提示只依据后端返回的有效采用状态', async () => {
  const ui = await withMediaReview(await view({ afterMutation(workspace) { Object.assign(workspace.assets[0], { imageStatus: 'adopted', imageReviewReasons: [] }); } }));
  await ui.click('重新核对当前图');
  assert.deepEqual(ui.calls.map(call => call.action), ['media.adopt']);
  assert.deepEqual(ui.notices, ['原依赖核对通过，当前图恢复有效采用；未生成新图。']);
  const tree = ui.render();
  assert.equal(nodes(tree).some(node => node.type?.name === 'Button' && text(node) === '重新核对当前图'), false, 'same-version lifecycle changes must replace cached detail status');
  assert.doesNotMatch(text(tree), /当前图片待复核，不计入有效采用/);
});

test('重新核对保留只读、未保存编辑和详情未就绪门禁', async () => {
  for (const options of [{ writable: false }, { detail: 'missing' as const }, { detail: 'stale' as const }]) {
    const ui = await withMediaReview(await view(options));
    assert.equal(button(ui.render(), '重新核对当前图').disabled, true);
    assert.deepEqual(ui.calls, []);
  }
  const ui = await withMediaReview(await view());
  nodes(ui.render()).find(node => node.type === 'textarea' && node.props.className === 'asset-prompt-input').props.onChange({ target: { value: '未保存文字' } });
  assert.equal(button(ui.render(), '重新核对当前图').disabled, true);
  assert.deepEqual(ui.calls, []);
});

test('非媒体失效、质量退回和未知任务不提供重新核对当前图按钮', async () => {
  const patches = [
    { imageStatus: 'rejected' }, { imageStatus: 'invalid' }, { imageStatus: 'submission_unknown' },
    { imageReviewReasons: [{ kind: 'own-design-changed' }] }, { imageReviewReasons: [{ kind: 'unverified-identity' }] },
    { imageReviewReasons: [] }, { imageReviewReasons: [{ kind: 'unknown-cause' }] }, { designStatus: 'needs_review' },
  ];
  for (const patch of patches) {
    const ui = await withMediaReview(await view()); Object.assign(ui.workspace.assets[0], patch, { version: 4 });
    ui.render(); await ui.host.flush();
    assert.equal(nodes(ui.render()).some(node => node.type?.name === 'Button' && text(node) === '重新核对当前图'), false, JSON.stringify(patch));
    assert.deepEqual(ui.calls, []);
  }
  const ui = await withMediaReview(await view({ jobs: [{ id: 'unknown', kind: 'image', assetId: 'child-design', status: 'submission_unknown' }] }));
  assert.equal(nodes(ui.render()).some(node => node.type?.name === 'Button' && text(node) === '重新核对当前图'), false);
  assert.deepEqual(ui.calls, []);
});
