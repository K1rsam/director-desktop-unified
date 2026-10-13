import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import React from 'react';
import { createServer } from 'vite';

let networkRequests = 0;
Object.assign(globalThis, { location: { search: '?preview=1' }, fetch: async () => { networkRequests++; throw Error('Real network is forbidden'); } });
const shared = globalThis as typeof globalThis & { __refineHooks?: HookHost; __refineApi?: (value: any) => Promise<any> };
class HookHost {
  cells: any[] = []; cursor = 0; effects: (() => void)[] = [];
  useState(initial: any) { const i = this.cursor++; if (!(i in this.cells)) this.cells[i] = typeof initial === 'function' ? initial() : initial; return [this.cells[i], (next: any) => { this.cells[i] = typeof next === 'function' ? next(this.cells[i]) : next; }]; }
  useRef(initial: any) { const i = this.cursor++; return this.cells[i] ??= { current: initial }; }
  useEffect(callback: () => void | (() => void), dependencies: any[]) { const i = this.cursor++, old = this.cells[i]; if (!old || dependencies.some((value, n) => value !== old.dependencies[n])) this.effects.push(() => { old?.cleanup?.(); this.cells[i].cleanup = callback(); }); this.cells[i] = { dependencies, cleanup: old?.cleanup }; }
  render(Component: any, props: any) { this.cursor = 0; this.effects = []; shared.__refineHooks = this; return Component(props); }
  async flush() { for (const effect of this.effects) effect(); this.effects = []; await new Promise(done => setImmediate(done)); }
}
let replacements = 0;
const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false }, appType: 'custom', plugins: [{ name: 'refinement-isolated-hooks', enforce: 'pre', transform(code, id) {
  if (!['/src/StoryboardRefinement.tsx', '/src/Stage3Panel.tsx'].some(path => id.replaceAll('\\', '/').split('?')[0].endsWith(path))) return;
  for (const [from, to] of [["import { useEffect, useRef, useState } from 'react';", "import { useEffect, useRef, useState } from '/@refine-hooks';"], ["import { request } from './api';", "import { request } from '/@refine-api';"]]) {
    assert.equal(code.split(from).length, 2); code = code.replace(from, to);
  }
  replacements++; return code;
}, resolveId(id) { if (id === '/@refine-hooks' || id === '/@refine-api') return '\0' + id; }, load(id) {
  if (id === '\0/@refine-hooks') return ['useState', 'useEffect', 'useRef'].map(name => `export function ${name}(...args) { return globalThis.__refineHooks.${name}(...args); }`).join('\n');
  if (id === '\0/@refine-api') return 'export async function request(value) { return globalThis.__refineApi(value); }';
} }] });
const { StoryboardRefinement } = await server.ssrLoadModule('/src/StoryboardRefinement.tsx');
const { projectProductionBusy } = await server.ssrLoadModule('/src/stageProduction.ts');
assert.equal(replacements, 1);
const { Stage3Panel } = await server.ssrLoadModule('/src/Stage3Panel.tsx');
assert.equal(replacements, 2);
after(async () => { assert.equal(networkRequests, 0); delete shared.__refineHooks; delete shared.__refineApi; await server.close(); });
function nodes(tree: any): any[] { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(nodes)]; }
function text(tree: any): string { return typeof tree === 'string' || typeof tree === 'number' ? String(tree) : React.Children.toArray(tree?.props?.children).map(text).join(''); }
function button(tree: any, label: string) { const node = nodes(tree).find(node => node.type?.name === 'Button' && text(node) === label); assert.ok(node, label); return node.props; }
const tick = () => new Promise(done => setImmediate(done));
const job = (entityId: string, status = 'running') => ({ id: 'other-workflow', kind: 'workflow', provider: 'local', entityId, status });
async function view() {
  const workspace = { project: { id: 'isolated-refinement' }, source: { episodes: [{ id: 'e1', title: '第一集' }] }, units: [{ id: 'u1', episodeId: 'e1', title: '原已采用单元' }], jobs: [] as any[] };
  const calls: any[] = [], reads: any[] = [], host = new HookHost();
  shared.__refineApi = async request => { reads.push(request); assert.equal(request.action, 'jobs.detail'); return { id: request.jobId, result: { state: 'candidate', candidateSha256: 'original-candidate-hash', episodeId: 'e1', draft: { units: [] } } }; };
  const props = { workspace, episodeId: 'e1', selectedUnit: workspace.units[0], writable: true, mutate: async (action: string, payload: any) => { calls.push({ action, payload }); return workspace; } };
  const render = () => host.render(StoryboardRefinement, props); render(); await host.flush();
  return { workspace, props, calls, reads, render };
}

test('同项目互斥生产禁用精修，历史失败和图片队列不冒充文字流程占用', async () => {
  const ui = await view();
  for (const stage of ['foundation.run', 'storyboard.run', 'storyboard.refine', 'images.design:', 'images.produce:', 'units.regenerate']) {
    ui.workspace.jobs = [job(stage)]; assert.equal(projectProductionBusy(ui.workspace), true);
    const action = button(ui.render(), '精修当前集'); assert.equal(action.disabled, true); action.onClick();
    assert.ok(text(ui.render()).includes('已有稿件和精修记录仍可查看'));
    assert.equal(ui.calls.length, 0);
  }
  for (const other of [{ ...job('storyboard.run'), status: 'interrupted' }, { ...job('storyboard.run'), status: 'failed' }, { ...job('storyboard.run'), status: 'completed' }, { ...job('asset'), kind: 'image' }, { ...job('other'), kind: 'video' }]) {
    ui.workspace.jobs = [other]; assert.equal(projectProductionBusy(ui.workspace), false);
    assert.equal(button(ui.render(), '精修当前集').disabled, false);
  }
  ui.workspace.jobs = [{ ...job('other'), snapshot: { stage: 'storyboard.run' } }];
  assert.equal(button(ui.render(), '精修当前集').disabled, true);
});

test('精修弹窗已打开后出现生产占用仍拦截提交，释放后保留原范围与用户说明', async () => {
  const ui = await view(); button(ui.render(), '精修当前集').onClick();
  let tree = ui.render();
  nodes(tree).find(node => node.type === 'select').props.onChange({ target: { value: 'unit' } });
  nodes(tree).filter(node => node.type === 'textarea')[0].props.onChange({ target: { value: '只核对当前单元动作衔接' } });
  nodes(tree).filter(node => node.type === 'textarea')[1].props.onChange({ target: { value: '保留原文与已有身份' } });
  ui.workspace.jobs = [job('storyboard.run')];
  const blocked = button(ui.render(), '开始精修'); assert.equal(blocked.disabled, true); blocked.onClick(); await tick();
  assert.deepEqual(ui.calls, []);
  ui.workspace.jobs = []; tree = ui.render(); assert.equal(button(tree, '开始精修').disabled, false); button(tree, '开始精修').onClick(); await tick();
  assert.deepEqual(ui.calls, [{ action: 'storyboard.refine', payload: { episodeId: 'e1', refinementScope: { unitIds: ['u1'] }, reviewNotes: '只核对当前单元动作衔接', preserveNotes: '保留原文与已有身份' } }]);
});

test('占用中仍可查看候选，但采用与原流程继续均不派发，释放后继续原 workflow', async () => {
  const ui = await view();
  const original = { ...job('storyboard.refine', 'interrupted'), id: 'original-refinement' };
  ui.workspace.jobs = [job('storyboard.run'), original];
  assert.equal(button(ui.render(), '查看').disabled, false); button(ui.render(), '查看').onClick(); await tick();
  assert.equal(ui.reads.length, 1);
  for (const label of ['继续', '采用到第一集']) { const action = button(ui.render(), label); assert.equal(action.disabled, true); action.onClick(); await tick(); }
  assert.deepEqual(ui.calls, []);
  ui.workspace.jobs = [original];
  assert.equal(button(ui.render(), '继续').disabled, false); button(ui.render(), '继续').onClick(); await tick();
  assert.deepEqual(ui.calls, [{ action: 'workflows.resume', payload: { workflowId: 'original-refinement' } }]);
  ui.props.writable = false;
  const adopt = button(ui.render(), '采用到第一集'); assert.equal(adopt.disabled, true); adopt.onClick(); await tick();
  assert.equal(ui.calls.length, 1);
});

test('分镜重做弹窗在其他制作启动后同时阻止按钮与表单提交，释放后只提交原单元', async () => {
  const ui = await view(), host = new HookHost();
  Object.assign(ui.workspace, { assets: [], providers: [], rules: [], settings: {}, foundation: {} });
  Object.assign(ui.workspace.units[0], { version: 3, prompt: '合成原完整稿' });
  shared.__refineApi = async request => { assert.equal(request.action, 'units.get'); return structuredClone(ui.workspace.units[0]); };
  const render = () => host.render(Stage3Panel, ui.props);
  render(); await host.flush();
  ui.workspace.jobs = [job('foundation.run')];
  const blockedOpen = button(render(), '重做单元'); assert.equal(blockedOpen.disabled, true); blockedOpen.onClick();
  assert.ok(text(render()).includes('本项目正在制作'));
  assert.ok(!nodes(render()).some(node => node.type === 'form'));
  ui.workspace.jobs = []; button(render(), '重做单元').onClick();
  nodes(render()).find(node => node.type === 'textarea').props.onChange({ target: { value: '保留原文，优化动作衔接' } });
  ui.workspace.jobs = [job('images.design:')];
  const tree = render(); assert.equal(button(tree, '开始重做文字').disabled, true);
  nodes(tree).find(node => node.type === 'form').props.onSubmit({ preventDefault() {} }); await tick();
  assert.deepEqual(ui.calls, []);
  ui.workspace.jobs = []; const ready = render(); assert.equal(button(ready, '开始重做文字').disabled, false);
  nodes(ready).find(node => node.type === 'form').props.onSubmit({ preventDefault() {} }); await tick();
  assert.deepEqual(ui.calls, [{ action: 'units.regenerate', payload: { unitId: 'u1', instruction: '保留原文，优化动作衔接', expectedVersion: 3 } }]);
});

test('镜表仅在已冻结声音合同及精确事件枚举同时满足时显示非人声提示', async () => {
  const ui = await view();
  Object.assign(ui.workspace, { assets: [], providers: [], rules: [], settings: {}, foundation: {} });
  const parts = [
    { speaker: 'Vo', channel: '现场', presentation: '声音事件' },
    { speaker: '旁白', channel: 'VO', presentation: 'VO' },
    { speaker: '小王', channel: 'OS', presentation: 'OS' },
    { speaker: '小王', channel: '现场', presentation: '画内' },
    { speaker: 'Vo', channel: '现场', presentation: '声音事件 ' },
  ].map(part => ({ ...part, text: '群众笑声戛然而止。', delivery: '声音事件', voiceStart: 0, voiceEnd: 0.3 }));
  Object.assign(ui.workspace.units[0], { version: 3, prompt: '合成原完整稿', shots: [{ id: 's1', duration: 2, sound: '环境声', dialogueParts: parts }] });
  shared.__refineApi = async request => { assert.equal(request.action, 'units.get'); return structuredClone(ui.workspace.units[0]); };
  for (const profile of [undefined, 'unknown', 'source-bound-sound-events/v1']) {
    Object.assign(ui.workspace.units[0], { storyboardSoundEventProfile: profile });
    const before = structuredClone(ui.workspace), host = new HookHost();
    const render = () => host.render(Stage3Panel, ui.props);
    render(); await host.flush();
    nodes(render()).find(node => node.props?.role === 'tab' && text(node) === '镜表').props.onClick();
    const table = nodes(render()).find(node => node.type === 'table');
    const rows = nodes(table).filter(node => node.type === 'p' && nodes(node).some(child => child.type === 'strong'));
    assert.equal(rows.length, parts.length);
    assert.equal(text(rows[0]), profile === 'source-bound-sound-events/v1'
      ? '声音事件：群众笑声戛然而止。镜内 0.0—0.3 秒 · 按原文执行声音变化，不作人声朗读'
      : 'Vo：群众笑声戛然而止。镜内 0.0—0.3 秒');
    assert.deepEqual(rows.slice(1).map(text), [
      '旁白：群众笑声戛然而止。镜内 0.0—0.3 秒',
      '小王 OS：群众笑声戛然而止。镜内 0.0—0.3 秒',
      '小王：群众笑声戛然而止。镜内 0.0—0.3 秒',
      'Vo：群众笑声戛然而止。镜内 0.0—0.3 秒',
    ]);
    assert.deepEqual(ui.workspace, before, '展示不得重分类或修改原单元数据');
  }
  assert.deepEqual(ui.calls, []);
});
