import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'vite';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { installSyntheticExecutionPolicy } from './synthetic-execution-policy.mjs';

globalThis.location = { search: '?preview=1' };
let networkRequests = 0;
globalThis.fetch = async () => { networkRequests++; throw new Error('Real network access is forbidden.'); };
const proof = JSON.parse(await readFile('src-tauri/src/providers/verification.json', 'utf8'));
const sizes = { ...Object.fromEntries(proof.videoFixed720pModelsVerified.map(id => [id, ['720p']])), ...proof.videoSelectableSizesVerified };
assert.equal(Object.keys(sizes).length, 6);
const ratios = ['1:1', '3:4', '16:9', '4:3', '9:16', '21:9'];
const models = Object.entries(sizes).map(([id, resolutions]) => ({ id, name: id, kind: 'video', adapted: true, available: true, resolutions, ratios }));
const ref = { assetId: 'asset-1', variantId: 'variant-1', role: 'character', order: 1 };
const unit = { id: 'unit-1', title: '已准备单元', episodeId: 'ep-1', prompt: '完整单元文字提示词。', duration: 10, version: 1, sourceRefs: ['row-1'], assetRefs: [ref], initialState: '起始连续状态', exitState: '结束连续状态', preparedPackage: { provider: 'dreamina', model: 'seedance2.5', ratio: '16:9', resolution: '720p', duration: 10, unitVersion: 1, prompt: '完整单元文字提示词。', references: [{ ...ref, mediaId: 'image-1' }], estimatedCost: { currency: 'CNY', amount: 0.25 } } };
const blocked = { ...unit, id: 'unit-2', title: '缺图单元', preparedPackage: null, problems: ['缺少场景当前图片'] };
const base = {
  project: { id: 'fixture-project', title: '第四阶段界面验证' },
  source: { episodes: [{ id: 'ep-1', title: '第一集', number: 1 }], rows: [{ id: 'row-1', number: 1, text: '锁定原文。', kind: 'body', episodeId: 'ep-1' }] },
  assets: [{ id: 'asset-1', variantId: 'variant-1', name: '当前形象', adoptedMediaId: 'image-1' }],
  media: [{ id: 'image-1', kind: 'image', assetId: 'asset-1', variantId: 'variant-1', name: '当前图文件', path: '', previewUrl: 'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg"/%3E' }],
  units: [unit], jobs: [], rules: [],
  settings: { videoProvider: 'dreamina', videoModel: 'seedance2.5', videoResolution: '720p', ratio: '16:9' },
  providers: [{ id: 'dreamina', models }],
  dreaminaAccount: { status: 'unknown', ready: false, identityVerified: false },
};
const cases = [];
async function test(name, callback) { try { await callback(); cases.push({ name, passed: true }); } catch (error) { cases.push({ name, passed: false, error: error.message }); } }
function buttons(html) { return [...html.matchAll(/<button\b[^>]*>[\s\S]*?<\/button>/g)].map(item => item[0]); }
function button(html, title) { const found = buttons(html).find(item => item.replace(/<[^>]+>/g, '').trim() === title); assert.ok(found, title); return found; }
function disabled(html, title, expected) { assert.equal(button(html, title).includes('disabled=""'), expected, title); }
function select(html, label) { const match = html.match(new RegExp('<span>' + label + '<\\/span><select([^>]*)>([\\s\\S]*?)<\\/select>')); assert.ok(match, label); return { disabled: match[1].includes('disabled=""'), values: [...match[2].matchAll(/<option\b[^>]*value="([^"]*)"[^>]*>/g)].map(item => item[1]).filter(Boolean), html: match[0] }; }
let unexpectedSsrMutations = 0;
const props = { writable: true, mutate: async () => { unexpectedSsrMutations++; throw new Error('SSR must not call the desktop core.'); }, inspect() {}, onNotice() {}, onError() {} };
const server = await createServer({ logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false }, appType: 'custom' });
try {
  const { UnitPanel } = await server.ssrLoadModule('/src/UnitPanel.tsx');
  const policy = await installSyntheticExecutionPolicy(server,
    ['storyboard.run', 'videos.preflight', 'videos.prepare', 'videos.submit']);
  const render = (workspace, extra = {}) => renderToStaticMarkup(React.createElement(UnitPanel, { ...props, workspace, mode: 'videos', ...extra }));
  await test('Offline policy disables production buttons without changing readable materials or calling a transport', () => {
    try {
      policy.offline();
      const html = render(base);
      disabled(html, '准备所选素材', true);
      disabled(html, '生成所选视频', true);
      disabled(render(base, { mode: 'storyboard' }), '一键创作分镜', true);
      assert.equal(networkRequests, 0); assert.equal(unexpectedSsrMutations, 0);
    } finally { policy.standard(); }
  });
  await test('Parameters use four labeled grid fields and a two-action footer', () => {
    const html = render(base);
    assert.ok(html.includes('video-production-header'));
    assert.ok(html.includes('video-parameter-grid'));
    assert.equal((html.match(/class="video-parameter-field"/g) || []).length, 4);
    assert.ok(html.includes('video-production-footer'));
    assert.ok(html.includes('video-production-summary'));
    assert.ok(!html.includes('核实模型'));
    assert.ok(!html.includes('保存参数'));
    disabled(html, '准备所选素材', false);
    disabled(html, '生成所选视频', false);
    assert.ok(html.includes('0.250'));
  });
  await test('An unknown cached account does not block automatic prepare preflight', () => {
    disabled(render({ ...base, units: [{ ...unit, preparedPackage: null }] }), '准备所选素材', false);
  });
  await test('Missing capabilities allow complete saved parameters to enter prepare, never submit', () => {
    const html = render({ ...base, providers: [] });
    disabled(html, '准备所选素材', false);
    disabled(html, '生成所选视频', true);
    assert.ok(html.includes('待检查'));
  });
  await test('One ready and one blocked unit permit explicit partial generation', () => {
    const html = render({ ...base, units: [unit, blocked] });
    disabled(html, '生成所选视频', false);
    assert.ok(html.includes('1 / 2 个素材包可提交'));
    assert.ok(html.includes('其余单元需要处理'));
  });
  await test('Zero ready units require preparing before generation', () => {
    const html = render({ ...base, units: [blocked] });
    disabled(html, '准备所选素材', false);
    disabled(html, '生成所选视频', true);
    assert.ok(html.includes('0 / 1 个素材包可提交'));
  });
  await test('No units has a clear empty state and disabled production actions', () => {
    const html = render({ ...base, units: [] });
    disabled(html, '准备所选素材', true);
    disabled(html, '生成所选视频', true);
    assert.ok(html.includes('可直接导入现成分镜 MD，或完成第二阶段创作。'));
    assert.ok(!html.includes('先完成第三阶段'));
    assert.ok(html.includes('尚未有分镜单元'));
  });
  await test('Invalid saved size is retained and blocks both actions without downgrade', () => {
    const html = render({ ...base, settings: { ...base.settings, videoModel: 'seedance2.0', videoResolution: '4k' } });
    assert.ok(select(html, '视频尺寸（固定）').html.includes('value="4k"'));
    assert.ok(/4[kK] · 请重新选择/.test(html));
    disabled(html, '准备所选素材', true);
    disabled(html, '生成所选视频', true);
  });
  await test('Invalid saved model remains visible and is not automatically substituted', () => {
    const html = render({ ...base, settings: { ...base.settings, videoModel: 'removed-model' } });
    assert.ok(select(html, '视频模型').html.includes('removed-model · 请重新选择'));
    disabled(html, '准备所选素材', true);
    disabled(html, '生成所选视频', true);
  });
  await test('Busy/read-only mode locks all four editable parameters', () => {
    const html = render(base, { writable: false });
    for (const label of ['渠道', '视频模型', '视频尺寸', '画面比例']) assert.equal(select(html, label).disabled, true);
    disabled(html, '准备所选素材', true);
    disabled(html, '生成所选视频', true);
  });
  for (const model of models) await test(`Verified ${model.id} keeps its own sizes and fixed/editable mode`, () => {
    const html = render({ ...base, settings: { ...base.settings, videoModel: model.id, videoResolution: model.resolutions[0] } });
    const field = select(html, '视频尺寸(?:（固定）)?');
    assert.deepEqual(field.values, model.resolutions);
    assert.equal(field.disabled, model.resolutions.length === 1);
    assert.deepEqual(new Set(select(html, '画面比例').values), new Set(ratios));
  });
  await test('Stage 2 remains a text workflow independent of video model settings', () => {
    const html = render({ ...base, providers: [], settings: {}, units: [] }, { mode: 'storyboard' });
    disabled(html, '一键创作分镜', false);
    for (const forbidden of ['video-production-panel', '准备所选素材', '视频模型', '视频尺寸', '核实模型', '保存参数']) assert.ok(!html.includes(forbidden), forbidden);
  });
  await test('Reference labels follow the asset current ID, not a global adopted flag', () => {
    const historical = { ...base.media[0], id: 'history', name: '旧共享文件', adopted: true };
    const html = render({ ...base, media: [...base.media, historical], units: [{ ...unit, assetRefs: [{ ...ref, mediaId: 'history' }], problems: ['场景尚未采用参考图'] }] });
    assert.ok(html.includes('自动使用当前图'));
    assert.ok(html.includes('当前图文件 · 当前图'));
    assert.ok(html.includes('旧共享文件 · 历史图'));
    assert.ok(html.includes('场景尚无当前参考图'));
    assert.ok(!html.includes('自动匹配已采用图'));
  });
} finally { await server.close(); }

// Replay the actual component handlers with a small hook host. Only mutate is stubbed.
// This checks request routing and draft/ready guards without opening native dialogs or channels.
class HookHost {
  cells = []; cursor = 0; effects = [];
  useState(initial) { const i = this.cursor++; if (!(i in this.cells)) this.cells[i] = typeof initial === 'function' ? initial() : initial; return [this.cells[i], next => { this.cells[i] = typeof next === 'function' ? next(this.cells[i]) : next; }]; }
  useRef(initial) { const i = this.cursor++; if (!(i in this.cells)) this.cells[i] = { current: initial }; return this.cells[i]; }
  useMemo(callback) { this.cursor++; return callback(); }
  useEffect(callback, dependencies) { const i = this.cursor++, old = this.cells[i]; if (!old || !dependencies || dependencies.some((value, n) => value !== old[n])) this.effects.push(callback); this.cells[i] = dependencies; }
  render(Component, values) { this.cursor = 0; this.effects = []; globalThis.__unitPanelHooks = this; return Component(values); }
  flushEffects() { for (const effect of this.effects) effect(); this.effects = []; }
}
function descendants(tree) { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(descendants)]; }
function textOf(value) { return typeof value === 'string' || typeof value === 'number' ? String(value) : React.Children.toArray(value?.props?.children).map(textOf).join(''); }
function action(tree, label) { const found = descendants(tree).find(node => node.type?.name === 'Button' && React.Children.toArray(node.props.children).map(textOf).join('') === label); assert.ok(found, label); return found.props; }
function field(tree, label) { const found = descendants(tree).find(node => node.type === 'label' && textOf(node).startsWith(label)); assert.ok(found, label); const input = descendants(found).find(node => node.type === 'select'); assert.ok(input, label); return input.props; }
const replayServer = await createServer({ logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false }, appType: 'custom', plugins: [{ name: 'isolated-unitpanel-hook-host', enforce: 'pre', transform(code, id) { if (id.replaceAll('\\', '/').split('?')[0].endsWith('/src/UnitPanel.tsx')) return code.replaceAll("from 'react'", "from '/@unitpanel-hook-host'"); }, resolveId(id) { if (id === '/@unitpanel-hook-host') return '\0isolated-unitpanel-react'; }, load(id) { if (id === '\0isolated-unitpanel-react') return ['useState', 'useRef', 'useMemo', 'useEffect'].map(name => `export function ${name}(...args) { return globalThis.__unitPanelHooks.${name}(...args); }`).join('\n'); } }] });
let simulatedMutations = 0;
try {
  const { UnitPanel } = await replayServer.ssrLoadModule('/src/UnitPanel.tsx');
  const setup = (workspace = base) => { const host = new HookHost(), calls = [], errors = [], values = { ...props, workspace, mode: 'videos', mutate: async (name, input) => { simulatedMutations++; calls.push({ name, input }); return workspace; }, onError: error => errors.push(error) }; return { host, calls, errors, values, render: () => host.render(UnitPanel, values) }; };
  await test('Prepare callback sends one explicit parameter request without save/submit calls', async () => {
    const item = setup({ ...base, units: [{ ...unit, preparedPackage: null }] }), tree = item.render();
    action(tree, '准备所选素材').onClick(); await Promise.resolve();
    assert.deepEqual(item.calls, [{ name: 'videos.prepare', input: { unitIds: ['unit-1'], provider: 'dreamina', model: 'seedance2.5', ratio: '16:9', resolution: '720p' } }]);
  });
  await test('Changing a dropdown sends no request and requires preparing changed parameters', () => {
    const item = setup(), tree = item.render();
    field(tree, '画面比例').onChange({ target: { value: '9:16' } });
    const changed = item.render();
    assert.equal(item.calls.length, 0);
    assert.equal(action(changed, '准备所选素材').disabled, false);
    assert.equal(action(changed, '生成所选视频').disabled, true);
    action(changed, '生成所选视频').onClick();
    assert.equal(item.calls.length, 0);
    assert.ok(item.errors[0].includes('重新准备'));
  });
  await test('Opening review sends zero requests; explicit confirmation submits only reviewed ready units', async () => {
    const item = setup({ ...base, units: [unit, blocked] }), tree = item.render();
    assert.equal(action(tree, '生成所选视频').disabled, false);
    action(tree, '生成所选视频').onClick(); await Promise.resolve();
    assert.equal(item.calls.length, 0);
    const review = item.render();
    assert.ok(textOf(review).includes('1 个视频单元'));
    assert.ok(textOf(review).includes('1 个单元尚未就绪，本次不会提交'));
    assert.ok(textOf(review).includes('0.250'));
    action(review, '确认提交并生成视频').onClick(); await Promise.resolve();
    assert.deepEqual(item.calls, [{ name: 'videos.submit', input: { unitIds: ['unit-1'], provider: 'dreamina', model: 'seedance2.5', ratio: '16:9', resolution: '720p' } }]);
  });
  await test('Cancelling a video review sends zero requests and removes the submit action', async () => {
    const item = setup(); action(item.render(), '生成所选视频').onClick();
    action(item.render(), '取消生成').onClick(); await Promise.resolve();
    assert.equal(item.calls.length, 0);
    assert.ok(!textOf(item.render()).includes('确认提交并生成视频'));
  });
  await test('Unknown estimates are explicitly not represented as free generation', () => {
    const item = setup({ ...base, units: [{ ...unit, preparedPackage: { ...unit.preparedPackage, estimatedCost: null } }] });
    action(item.render(), '生成所选视频').onClick(); const review = item.render();
    assert.ok(textOf(review).includes('合计未知'));
    assert.ok(textOf(review).includes('不代表免费'));
    assert.equal(item.calls.length, 0);
  });
  for (const [name, change] of [
    ['unit version', workspace => ({ ...workspace, units: [{ ...unit, version: 2 }] })],
    ['estimated cost', workspace => ({ ...workspace, units: [{ ...unit, preparedPackage: { ...unit.preparedPackage, estimatedCost: { currency: 'CNY', amount: 9 } } }] })],
    ['frozen media hash', workspace => ({ ...workspace, media: [{ ...workspace.media[0], sha256: 'changed-hash' }] })],
    ['selected range expanded', workspace => ({ ...workspace, units: [...workspace.units, { ...unit, id: 'new-ready-unit' }] })],
    ['blocked unit became ready', workspace => ({ ...workspace, units: [unit, { ...unit, id: blocked.id }] })],
    ['official account', workspace => ({ ...workspace, dreaminaAccount: { ...workspace.dreaminaAccount, accountKey: 'different-account' } })],
  ]) await test(`Changed ${name} invalidates review without submitting`, async () => {
    const item = setup({ ...base, units: [unit, blocked] });
    action(item.render(), '生成所选视频').onClick();
    item.values.workspace = change(item.values.workspace);
    action(item.render(), '确认提交并生成视频').onClick(); await Promise.resolve();
    assert.equal(item.calls.length, 0);
    assert.ok(item.errors[0].includes('已变化'));
    assert.ok(!textOf(item.render()).includes('确认提交并生成视频'));
  });
  await test('Rapid duplicate confirmation starts only one isolated submission', async () => {
    const item = setup(); let finish;
    item.values.mutate = async (name, input) => { simulatedMutations++; item.calls.push({ name, input }); return new Promise(resolve => { finish = resolve; }); };
    action(item.render(), '生成所选视频').onClick();
    const confirm = action(item.render(), '确认提交并生成视频');
    confirm.onClick(); confirm.onClick();
    assert.equal(item.calls.length, 1);
    assert.equal(item.calls[0].name, 'videos.submit');
    assert.ok(!textOf(item.render()).includes('确认提交并生成视频'));
    finish(base); await Promise.resolve(); await Promise.resolve();
  });
  await test('Programmatic zero-ready submit is rejected before calling core', () => {
    const item = setup({ ...base, units: [blocked] }), tree = item.render();
    assert.equal(action(tree, '生成所选视频').disabled, true);
    action(tree, '生成所选视频').onClick();
    assert.equal(item.calls.length, 0);
    assert.ok(item.errors[0].includes('尚未按当前参数准备完成'));
  });
  await test('Opening video page uses the core cache and never starts a duplicate preflight', async () => {
    const item = setup(); item.render(); item.host.flushEffects();
    assert.deepEqual(item.calls, []);
    item.values.writable = false; item.render(); item.host.flushEffects();
    await Promise.resolve();
    item.values.writable = true; const complete = item.render(); item.host.flushEffects();
    assert.equal(item.calls.length, 0);
    assert.ok(!textOf(complete).includes('正在自动检查渠道'));
  });
  await test('An empty startup catalog does not create a page error or block explicit preparation', async () => {
    const item = setup({ ...base, providers: [] }); item.values.mutate = async (name, input) => { simulatedMutations++; item.calls.push({ name, input }); return null; };
    item.render(); item.host.flushEffects(); await Promise.resolve();
    const complete = item.render(); item.host.flushEffects();
    assert.equal(item.calls.length, 0);
    assert.equal(item.errors.length, 0);
    assert.ok(!textOf(complete).includes('正在自动检查渠道'));
    assert.equal(action(complete, '准备所选素材').disabled, false);
  });
  await test('Stage 2 never executes video bootstrap effects', () => {
    const item = setup(); item.values.mode = 'storyboard'; item.render(); item.host.flushEffects();
    assert.equal(item.calls.length, 0);
  });
} finally { await replayServer.close(); delete globalThis.__unitPanelHooks; }

assert.equal(networkRequests, 0);
assert.equal(unexpectedSsrMutations, 0);
const report = { verifiedAt: new Date().toISOString(), method: 'Actual React SSR plus actual UnitPanel callback replay with isolated hook/mutation host', passed: cases.filter(item => item.passed).length, failed: cases.filter(item => !item.passed).length, networkRequests, unexpectedSsrMutations, simulatedMutations, userDatabaseReadOrWritten: false, channelsCalled: false, limitations: ['Hook replay checks frontend callbacks and lifecycle boundaries; it does not execute native React browser effects', 'Preflight/prepare/submit results are isolated stubs, not evidence of upstream acceptance or charging', 'Actual browser layout is reviewed separately by the controller'], cases };
console.log(JSON.stringify(report, null, 2));
if (report.failed) process.exitCode = 1;
