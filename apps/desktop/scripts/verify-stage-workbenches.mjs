import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'vite';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { installSyntheticExecutionPolicy } from './synthetic-execution-policy.mjs';

globalThis.location = { search: '?preview=1' };
let networkRequests = 0;
globalThis.fetch = async () => { networkRequests++; throw new Error('Real network is forbidden in workbench verification.'); };
const unit = { id: 'unit-1', title: '门前告别', episodeId: 'ep-1', sceneId: 'scene-1', scene: '门前', version: 3, duration: 6, prompt: '完整单元正文。\n小王说：“先走，别回头😀。”', initialState: '小王站在门前，右手握伞。', exitState: '小王仍握伞，转身离开。', sourceRefs: ['row-3'], assetRefs: [{ assetId: 'asset-1', variantId: 'base', order: 1, role: 'character' }], shots: [{ id: 'shot-1', duration: 6, camera: '近景侧拍', picture: '小王转身', lighting: '门边自然光', sound: '雨声', transition: '切', dialogueParts: [{ speaker: '小王', text: '先走，别回头😀。', voiceStart: 0, voiceEnd: 3, channel: '现场' }] }] };
const base = { project: { id: 'offline-project', title: '独立界面核验' }, source: { version: 2, text: '第一集\r\n场景：门前\r\n小王：“先走，别回头😀。”\r\n', sha256: 'offline-source', episodes: [{ id: 'ep-1', title: '第一集' }, { id: 'ep-2', title: '第二集' }], scenes: [{ id: 'scene-1', episodeId: 'ep-1', title: '门前' }], rows: [{ id: 'row-1', number: 1, text: '第一集\r\n', kind: 'episode', episodeId: 'ep-1', range: { start: 0, end: 5 } }, { id: 'row-3', number: 3, text: '小王：“先走，别回头😀。”\r\n', kind: 'body', episodeId: 'ep-1', sceneId: 'scene-1', range: { start: 12, end: 31 } }], diagnostics: [] }, foundation: { version: 4, direction: '模型提案', productionDirection: '已确认真人方向', facts: [{ id: 'fact-1', assetId: 'base-asset', variantId: 'base', attribute: '持物', value: '握伞', sourceRefs: ['row-3'], quotes: ['不能用这段存量文字代替当前原文。'], claim: 'SOURCE', scope: 'STATE' }] }, assets: [{ id: 'asset-1', baseAssetId: 'base-asset', variantId: 'base', version: 2, name: '小王基础形象', description: '门前持伞', sourceRefs: ['row-3'], applicability: { episodes: ['ep-1'] }, kind: 'CH' }], units: [unit], jobs: [], media: [], rules: [{ id: 'story-foundation', version: '1.0.0' }], settings: {}, providers: [] };
const cases = [], calls = [], reads = [], errors = [], notices = [];
const creativeManifest = JSON.parse(await readFile(new URL('../../../rules/stage3/manifest.json', import.meta.url), 'utf8'));
async function test(name, task) { try { await task(); cases.push({ name, passed: true }); } catch (error) { cases.push({ name, passed: false, error: error.message }); } }
const props = { workspace: base, writable: true, mutate: async (action, payload) => { calls.push({ action, payload }); return base; }, onError: value => errors.push(value), onNotice: value => notices.push(value) };
function descendants(tree) { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(descendants)]; }
function text(value) { return typeof value === 'string' || typeof value === 'number' ? String(value) : React.Children.toArray(value?.props?.children).map(text).join(''); }
function button(tree, label) { const result = descendants(tree).find(node => (node.type === 'button' || node.type?.name === 'Button') && text(node) === label); assert.ok(result, label); return result.props; }
function input(tree, label, type = 'textarea') { const node = descendants(tree).find(node => node.type === 'label' && text(node).startsWith(label)); assert.ok(node, label); const field = descendants(node).find(item => item.type === type); assert.ok(field, label); return field.props; }
function form(tree) { const result = descendants(tree).find(node => node.type === 'form'); assert.ok(result); return result.props; }
function disabled(html, label) { const result = [...html.matchAll(/<button\b[^>]*>[\s\S]*?<\/button>/g)].map(match => match[0]).find(value => value.replace(/<[^>]+>/g, '') === label); assert.ok(result, label); return result.includes('disabled=""'); }
class HookHost {
  cells = []; cursor = 0; effects = [];
  useState(initial) { const index = this.cursor++; if (!(index in this.cells)) this.cells[index] = typeof initial === 'function' ? initial() : initial; return [this.cells[index], next => this.cells[index] = typeof next === 'function' ? next(this.cells[index]) : next]; }
  useRef(value) { const index = this.cursor++; if (!(index in this.cells)) this.cells[index] = { current: value }; return this.cells[index]; }
  useMemo(callback) { this.cursor++; return callback(); }
  useEffect(callback, dependencies) { const index = this.cursor++, old = this.cells[index]; if (!old || dependencies.some((value, index) => value !== old[index])) this.effects.push(callback); this.cells[index] = dependencies; }
  render(Component, values) { this.cursor = 0; this.effects = []; globalThis.__productionHooks = this; return Component(values); }
}
const server = await createServer({ logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true }, appType: 'custom' });
try {
  const { Stage1Panel } = await server.ssrLoadModule('/src/Stage1Panel.tsx'), { Stage3Panel } = await server.ssrLoadModule('/src/Stage3Panel.tsx');
  const policy = await installSyntheticExecutionPolicy(server,
    ['foundation.run', 'storyboard.run', 'units.regenerate', 'workflows.resume', 'storyboard.decision.resolve', 'storyboard.refine', 'storyboard.refinement.adopt']);
  await test('Offline permissions keep text generation disabled while preserving the workspace view', () => {
    try {
      policy.offline();
      assert.equal(disabled(renderToStaticMarkup(React.createElement(Stage3Panel, props)), '生成分镜提示词'), true);
      assert.equal(disabled(renderToStaticMarkup(React.createElement(Stage1Panel, props)), '整理全剧制作基础'), true);
      assert.equal(calls.length, 0); assert.equal(networkRequests, 0);
    } finally { policy.standard(); }
  });
  const { CreativeRules } = await server.ssrLoadModule('/src/CreativeRules.tsx');
  const historicalRules = Array.from({ length: 29 }, (_, index) => ({ id: `synthetic-history-${index}`, version: '1.0.0' }));
  await test('Active rules view separates the original 22 paths and full hashes from the supplied historical registry', () => {
    const html = renderToStaticMarkup(React.createElement(CreativeRules, { rules: historicalRules }));
    assert.ok(html.includes('3008 原规则与 Rust 兼容适配'));
    assert.ok(html.includes('本版本原件 22 份')); assert.ok(html.includes('桌面通用与历史规则 · 29 份'));
    assert.ok(html.includes('mac-3008-compatible-rust/v1'));
    assert.ok(html.includes('离线接线验证不代表新旧成稿质量对比已完成'));
    for (const file of creativeManifest.files) {
      assert.ok(html.includes(file.path.replace('rules/stage3/candidates/mac-3008-20261006/', '')));
      assert.ok(html.includes(`title="${file.sha256}"`), file.path);
    }
  });
  await test('Offline rules view preserves all original fingerprints and explains the current execution restriction', () => {
    try {
      policy.offline();
      const html = renderToStaticMarkup(React.createElement(CreativeRules, { rules: historicalRules }));
      assert.ok(html.includes('当前执行模式未开放第三阶段'));
      assert.ok(!html.includes('第三阶段使用 3008 原规则'));
      for (const file of creativeManifest.files) assert.ok(html.includes(`title="${file.sha256}"`));
      assert.equal(networkRequests, 0); assert.equal(calls.length, 0);
    } finally { policy.standard(); }
  });
  await test('Stage 1 uses source/direction/assets/issues tabs and real episode/scene navigation', () => { const html = renderToStaticMarkup(React.createElement(Stage1Panel, props)); for (const label of ['原文与集场', '制作方向', '资产目录', '核对与记录', '集场目录', '第一集', '第二集']) assert.ok(html.includes(label), label); assert.ok(html.includes('小王：“先走，别回头😀。”')); assert.ok(!html.includes('textModel')); });
  await test('Stage 1 blocks creation for unresolved source scope', () => { const html = renderToStaticMarkup(React.createElement(Stage1Panel, { ...props, workspace: { ...base, source: { ...base.source, diagnostics: [{ code: 'BOUNDARY_UNCONFIRMED', rowIds: ['row-1'], message: '原文范围未定', blocking: true }] } } })); assert.ok(disabled(html, '整理全剧制作基础')); assert.ok(html.includes('确认这段用途')); assert.ok(!html.includes('明确原文的集场范围')); assert.ok(!html.includes('确认依据')); });
  await test('Stage 3 is wide text and exposes prompt/shots/source/history without video parameters', () => { const html = renderToStaticMarkup(React.createElement(Stage3Panel, props)); for (const label of ['完整提示词', '镜表', '来源与连续性', '版本历史', '复制全文', '编辑文字', '重做单元']) assert.ok(html.includes(label), label); assert.ok(html.includes('完整单元正文。')); assert.ok(!html.includes('视频模型')); assert.ok(!html.includes('准备所选素材')); });
  await test('Zero-unit Stage 3 can create selected source episodes without images or video model', () => { const html = renderToStaticMarkup(React.createElement(Stage3Panel, { ...props, workspace: { ...base, units: [], assets: [], settings: {} } })); assert.equal(disabled(html, '生成分镜提示词'), false); assert.ok(html.includes('即使没有图片或既有单元')); });
  await test('No-source and read-only modes disable actual generation', () => { const noSource = renderToStaticMarkup(React.createElement(Stage3Panel, { ...props, workspace: { ...base, source: null, units: [] } })); assert.equal(disabled(noSource, '生成分镜提示词'), true); const readonly = renderToStaticMarkup(React.createElement(Stage3Panel, { ...props, writable: false })); assert.equal(disabled(readonly, '编辑文字'), true); assert.equal(disabled(readonly, '重做单元'), true); });
  await test('Manual prompt overrides are explicitly distinguished from original shot structure', () => { const html = renderToStaticMarkup(React.createElement(Stage3Panel, { ...props, workspace: { ...base, units: [{ ...unit, promptOverride: true }] } })); assert.ok(html.includes('镜表仍保留原结构')); });
  await test('An affected successor displays continuity review beside unchanged prompt', () => { const html = renderToStaticMarkup(React.createElement(Stage3Panel, { ...props, workspace: { ...base, units: [{ ...unit, continuityStale: true }] } })); assert.ok(html.includes('前置单元状态已变化，请重做此单元，核对连续状态。')); assert.ok(html.includes('完整单元正文。')); assert.ok(html.indexOf('前置单元状态已变化') < html.indexOf('完整单元正文。')); const current = renderToStaticMarkup(React.createElement(Stage3Panel, props)); assert.ok(!current.includes('前置单元状态已变化')); });
} finally { await server.close(); }

globalThis.__productionRead = async payload => { reads.push(payload); if (payload.action === 'units.get') return unit; if (payload.action === 'units.versions.list') return { versions: [{ version: 1, data: { ...unit, version: 1, prompt: '历史完整正文' }, createdAt: '2026-10-04T00:00:00Z' }, { version: 3, data: unit }] }; if (payload.action === 'jobs.detail') return { id: 'flow-1', result: { waitingDecision: { candidates: [{ id: 'choice-1', title: '原文情绪机会', recommended: true, proposal: '保留原文因果' }] } } }; throw new Error(`Unexpected read ${payload.action}`); };
Object.defineProperty(globalThis, 'navigator', { value: { clipboard: { writeText: async value => notices.push(value) } }, configurable: true });
const replay = await createServer({ logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true }, appType: 'custom', plugins: [{ name: 'isolated-production-hooks', enforce: 'pre', transform(code, id) { if (/\/src\/(Stage1Panel|Stage3Panel|WorkflowProgress|StoryboardRefinement)\.tsx$/.test(id.replaceAll('\\', '/').split('?')[0])) return code.replaceAll("from 'react'", "from '/@production-hooks'"); }, resolveId(id, importer) { if (id === '/@production-hooks') return '\0production-hooks'; if (id === './api' && /\/(Stage1Panel|Stage3Panel|WorkflowProgress|StoryboardRefinement)\.tsx$/.test((importer || '').replaceAll('\\', '/'))) return '\0production-api'; }, load(id) { if (id === '\0production-hooks') return ['useState', 'useRef', 'useMemo', 'useEffect'].map(name => `export function ${name}(...args) { return globalThis.__productionHooks.${name}(...args); }`).join('\n'); if (id === '\0production-api') return 'export async function request(input) { return globalThis.__productionRead(input); } export async function choose() { throw new Error("Native chooser is not used in this verification"); }'; } }] });
const event = { preventDefault() {} }, settle = async () => { await new Promise(resolve => setImmediate(resolve)); };
try {
  const { Stage1Panel } = await replay.ssrLoadModule('/src/Stage1Panel.tsx'), { Stage3Panel } = await replay.ssrLoadModule('/src/Stage3Panel.tsx'), { WorkflowProgress } = await replay.ssrLoadModule('/src/WorkflowProgress.tsx');
  const { StoryboardRefinement } = await replay.ssrLoadModule('/src/StoryboardRefinement.tsx');
  const replayPolicy = await installSyntheticExecutionPolicy(replay,
    ['foundation.run', 'storyboard.run', 'units.regenerate', 'workflows.resume', 'storyboard.decision.resolve', 'storyboard.refine', 'storyboard.refinement.adopt']);
  const setup = (Component, extra = {}) => { calls.length = 0; reads.length = 0; errors.length = 0; const host = new HookHost(), values = { ...props, ...extra }; return { values, render: () => host.render(Component, values), flushEffects: async () => { const effects = [...host.effects]; host.effects = []; for (const callback of effects) callback(); await settle(); } }; };
  const refinementJob = { id: 'synthetic-refine-ep-2', kind: 'workflow', entityId: 'storyboard.refine', status: 'completed', result: { state: 'candidate', candidateSha256: 'c'.repeat(64), episodeId: 'ep-2', draft: { units: [{ id: 'synthetic-refined-unit', episodeId: 'ep-2', title: '第二集候选', prompt: '合成第二集精修正文。' }] }, reviewDecisions: [], productionFeedback: [] } };
  const refinementProps = { workspace: { ...base, jobs: [refinementJob] }, episodeId: 'ep-1', selectedUnit: unit };
  async function withRefinementRead(task, candidate = refinementJob) {
    const previous = globalThis.__productionRead;
    try {
      globalThis.__productionRead = async payload => { if (payload.action === 'jobs.detail' && payload.jobId === refinementJob.id) { reads.push(payload); return candidate; } return previous(payload); };
      await task();
    } finally { globalThis.__productionRead = previous; }
  }
  await test('Refinement preview identifies its saved episode and adopts only the original workflow ID and candidate hash', async () => withRefinementRead(async () => {
    const item = setup(StoryboardRefinement, refinementProps); item.render(); await item.flushEffects();
    button(item.render(), '查看').onClick(); await settle();
    const tree = item.render();
    assert.ok(text(tree).includes('本次结果属于：第二集'));
    assert.ok(text(tree).includes('与当前阅读集不同，采用会更新上述集数'));
    assert.ok(!descendants(tree).some(node => node.type?.name === 'Button' && text(node) === '采用到第一集'));
    assert.deepEqual(reads[0], { action: 'jobs.detail', projectId: base.project.id, jobId: refinementJob.id });
    assert.equal(calls.length, 0, 'Opening candidate must not adopt or regenerate');
    button(tree, '采用到第二集').onClick(); await settle();
    assert.deepEqual(calls, [{ action: 'storyboard.refinement.adopt', payload: { workflowId: refinementJob.id, candidateSha256: refinementJob.result.candidateSha256 } }]);
  }));
  await test('Offline policy disables refinement start and adoption while preserving candidate preview', async () => withRefinementRead(async () => {
    const item = setup(StoryboardRefinement, refinementProps); item.render(); await item.flushEffects();
    button(item.render(), '精修当前集').onClick();
    try {
      replayPolicy.offline();
      const html = renderToStaticMarkup(item.render());
      assert.equal(disabled(html, '精修当前集'), true); assert.equal(disabled(html, '开始精修'), true);
    } finally { replayPolicy.standard(); }
    button(item.render(), '取消').onClick(); button(item.render(), '查看').onClick(); await settle();
    try {
      replayPolicy.offline();
      const html = renderToStaticMarkup(item.render());
      assert.equal(disabled(html, '采用到第二集'), true); assert.equal(disabled(html, '查看'), false);
      assert.ok(html.includes('合成第二集精修正文。'));
      assert.equal(calls.length, 0); assert.equal(networkRequests, 0);
    } finally { replayPolicy.standard(); }
  }));
  await test('Blocked and no-improvement refinement results offer no adoption action', async () => {
    for (const state of ['blocked', 'no_improvement']) await withRefinementRead(async () => {
      const item = setup(StoryboardRefinement, refinementProps); item.render(); await item.flushEffects();
      button(item.render(), '查看').onClick(); await settle();
      assert.ok(!descendants(item.render()).some(node => node.props?.executionAction === 'storyboard.refinement.adopt'));
      assert.equal(calls.length, 0);
    }, { ...refinementJob, result: { ...refinementJob.result, state } });
  });
  await test('Stage 1 explicit current-episode action freezes its selection while the main action still covers the whole source', async () => { const item = setup(Stage1Panel); button(item.render(), '第一集1 场').onClick(); button(item.render(), '仅整理当前集').onClick(); await settle(); assert.deepEqual(calls, [{ action: 'foundation.run', payload: { options: { selection: { episodeIds: ['ep-1'] } } } }]); calls.length = 0; button(item.render(), '整理全剧制作基础').onClick(); await settle(); assert.deepEqual(calls, [{ action: 'foundation.run', payload: { options: { selection: { episodeIds: ['ep-1', 'ep-2'] } } } }]); });
  await test('Confirmed direction saves separately from proposal with source/foundation CAS', async () => { const item = setup(Stage1Panel); button(item.render(), '制作方向').onClick(); const tree = item.render(); assert.equal(input(tree, '已确认制作方向').value, '已确认真人方向'); assert.ok(text(tree).includes('模型提案')); input(tree, '已确认制作方向').onChange({ target: { value: '手动确认方向' } }); await form(item.render()).onSubmit(event); assert.deepEqual(calls[0], { action: 'production.direction.save', payload: { direction: '手动确认方向', expectedSourceVersion: 2, expectedFoundationVersion: 4 } }); });
  await test('First direction confirmation preserves explicit zero-version CAS', async () => { const item = setup(Stage1Panel, { workspace: { ...base, foundation: null } }); button(item.render(), '制作方向').onClick(); input(item.render(), '已确认制作方向').onChange({ target: { value: '首次确认方向' } }); await form(item.render()).onSubmit(event); assert.equal(calls[0].payload.expectedFoundationVersion, 0); });
  await test('Fact readback materializes current source rows, never copies stored quotes as evidence', () => { const item = setup(Stage1Panel); button(item.render(), '资产目录').onClick(); button(item.render(), '事实回读').onClick(); const tree = item.render(); assert.ok(text(tree).includes('小王：“先走，别回头😀。”')); assert.ok(!text(tree).includes('不能用这段存量文字')); });
  await test('Stage 3 zero-unit start uses source selection and no media submit actions', async () => { const item = setup(Stage3Panel, { workspace: { ...base, units: [], assets: [] } }); input(item.render(), '创作范围', 'select').onChange({ target: { value: 'ep-2' } }); button(item.render(), '生成分镜提示词').onClick(); await settle(); assert.deepEqual(calls[0], { action: 'storyboard.run', payload: { options: { selection: { episodeIds: ['ep-2'] } }, episodeIds: ['ep-2'] } }); });
  await test('Complete prompt copy preserves newline, punctuation and source dialogue', async () => { const item = setup(Stage3Panel); const before = notices.length; button(item.render(), '复制全文').onClick(); await settle(); assert.equal(notices[before], unit.prompt); assert.equal(calls.length, 0); });
  await test('Text editor saves CAS version and preserves source/asset/shot structure', async () => { const item = setup(Stage3Panel); button(item.render(), '编辑文字').onClick(); await settle(); input(item.render(), '完整提示词').onChange({ target: { value: '手改完整正文\n原话仍保留。' } }); await form(item.render()).onSubmit(event); const saved = calls[0]; assert.equal(saved.action, 'units.save'); assert.equal(saved.payload.unit.expectedVersion, 3); assert.equal(saved.payload.unit.promptOverride, true); assert.deepEqual(saved.payload.unit.sourceRefs, unit.sourceRefs); assert.deepEqual(saved.payload.unit.assetRefs, unit.assetRefs); assert.deepEqual(saved.payload.unit.shots, unit.shots); });
  await test('Unit regeneration sends exact unit/version and explicit instruction', async () => { const item = setup(Stage3Panel); button(item.render(), '重做单元').onClick(); input(item.render(), '修改要求').onChange({ target: { value: ' 保留持伞末态，加强转身动作。 ' } }); await form(item.render()).onSubmit(event); assert.deepEqual(calls[0], { action: 'units.regenerate', payload: { unitId: unit.id, instruction: '保留持伞末态，加强转身动作。', expectedVersion: 3 } }); });
  await test('Version list reads original unit and restoration creates explicit CAS request', async () => { const item = setup(Stage3Panel); button(item.render(), '版本历史').onClick(); await settle(); assert.equal(reads[0].action, 'units.versions.list'); assert.equal(reads[0].unitId, unit.id); const restores = descendants(item.render()).filter(node => node.type?.name === 'Button' && text(node) === '恢复为当前版本'); assert.equal(restores.length, 2); assert.equal(restores[1].props.disabled, true); await restores[0].props.onClick(); await settle(); assert.deepEqual(calls[0], { action: 'units.version.restore', payload: { unitId: unit.id, version: 1, expectedVersion: 3 } }); });
  await test('An old history response cannot repopulate records after the selected unit changes', async () => { const read = globalThis.__productionRead; let release; try { globalThis.__productionRead = async payload => payload.action === 'units.versions.list' ? new Promise(resolve => release = resolve) : read(payload); const item = setup(Stage3Panel); item.render(); await item.flushEffects(); button(item.render(), '版本历史').onClick(); await settle(); input(item.render(), '创作范围', 'select').onChange({ target: { value: 'ep-2' } }); item.render(); await item.flushEffects(); release({ versions: [{ version: 1, data: { ...unit, version: 1, prompt: '迟到的旧历史' } }] }); await settle(); input(item.render(), '创作范围', 'select').onChange({ target: { value: 'ep-1' } }); item.render(); await item.flushEffects(); assert.ok(!text(item.render()).includes('迟到的旧历史')); assert.equal(calls.length, 0); } finally { globalThis.__productionRead = read; } });
  await test('An old edit read cannot open an editor after the selected unit changes', async () => { const read = globalThis.__productionRead; let release; try { const item = setup(Stage3Panel); item.render(); await item.flushEffects(); globalThis.__productionRead = async payload => payload.action === 'units.get' ? new Promise(resolve => release = resolve) : read(payload); button(item.render(), '编辑文字').onClick(); await settle(); input(item.render(), '创作范围', 'select').onChange({ target: { value: 'ep-2' } }); item.render(); await item.flushEffects(); release(unit); await settle(); assert.ok(!text(item.render()).includes('保存文字新版本')); assert.equal(calls.length, 0); } finally { globalThis.__productionRead = read; } });
  await test('Shots and continuity tabs render actual structure and locked evidence', () => { const item = setup(Stage3Panel); button(item.render(), '镜表').onClick(); let tree = item.render(); assert.ok(text(tree).includes('近景侧拍')); assert.ok(text(tree).includes('0.0—6.0 秒')); button(tree, '来源与连续性').onClick(); tree = item.render(); assert.ok(text(tree).includes(unit.exitState)); assert.ok(text(tree).includes(base.source.rows[1].text)); });
  await test('Interrupted progress continues the original workflow ID without generation API', async () => { const workspace = { ...base, jobs: [{ id: 'flow-1', kind: 'workflow', provider: 'local', status: 'interrupted', snapshot: { stage: 'storyboard.run' } }] }; const item = setup(WorkflowProgress, { workspace, stage: 'storyboard' }); await button(item.render(), '继续原流程').onClick(); await settle(); assert.deepEqual(calls[0], { action: 'workflows.resume', payload: { workflowId: 'flow-1' } }); });
  await test('Accepted nodes report checked count without implying known whole-run progress', () => { const workspace = { ...base, workflowProgress: [{ workflowId: 'flow-1', stage: 'storyboard.run', status: 'partially_blocked', completedSteps: 1, totalSteps: 2, nodes: [{ id: 'plan', title: '事件与情绪规划', status: 'accepted' }, { id: 'unit', title: '文字单元创作', status: 'rejected' }], blocked: [{ reason: '仅第二集需要处理' }] }] }; const item = setup(WorkflowProgress, { workspace, stage: 'storyboard' }), tree = item.render(); assert.ok(text(tree).includes('已核对 1 个步骤')); assert.ok(text(tree).includes('关联范围待处理')); assert.ok(text(tree).includes('仅第二集需要处理')); assert.ok(!text(tree).includes('1 / 2')); assert.ok(!descendants(tree).some(node => node.type === 'progress')); const history = descendants(tree).find(node => node.type === 'details' && text(node).startsWith('步骤与历史')); assert.ok(history); assert.ok(descendants(history).some(node => node.type?.name === 'Badge' && node.props.status === 'accepted' && text(node) === '本批完成')); });
  const foundationNode = (phase, ids, index, count, status = 'running', extra = {}) => ({ id: `${phase}-${index}`, title: '整理制作内容', status, request: { stage: `foundation:${phase}`, scope: ids, input: { delivery: { method: 'foundation-delivery/v2', stage: phase, phase: phase === 'F2' ? 'GENERATE_F2' : 'EDIT', scope: ids, batchIndex: index, batchCount: count, fragment: null } } }, ...extra });
  const foundationProgress = (nodes, extra = {}) => ({ ...base, source: { ...base.source, episodes: Array.from({ length: 10 }, (_, index) => ({ id: `ep-${index + 1}`, number: index + 1, title: `第${index + 1}集` })) }, workflowProgress: [{ workflowId: 'foundation-flow', stage: 'foundation.run', status: 'running', completedSteps: 999, nodes, ...extra }] });
  await test('Foundation F2 progress names actual cross-episode batches and counts accepted batches only', () => {
    const workspace = foundationProgress([foundationNode('F2', ['ep-1', 'ep-2', 'ep-3'], 0, 4, 'accepted'), foundationNode('F2', ['ep-4', 'ep-5', 'ep-6'], 1, 4, 'running')]); const tree = setup(WorkflowProgress, { workspace, stage: 'foundation' }).render(), value = text(tree);
    assert.ok(value.includes('已完成 1 批资产整理')); assert.ok(value.includes('资产整理 · 第1–3集 · 本批 1（当前规划 4 批）')); assert.ok(value.includes('资产整理 · 第4–6集 · 本批 2（当前规划 4 批）')); assert.ok(value.includes('本批完成')); assert.ok(!value.includes('整理制作内容')); assert.ok(!value.includes('999')); assert.ok(!value.includes('个步骤'));
  });
  await test('Foundation Q1 material editing remains a separate phase from F2 batch success', () => {
    const editor = foundationNode('Q1', ['ep-1', 'ep-2', 'ep-3', 'ep-4'], 0, 2, 'accepted');
    const workspace = foundationProgress([foundationNode('F2', ['ep-1', 'ep-2', 'ep-3', 'ep-4'], 0, 2, 'accepted'), { id: editor.id, stage: editor.request.stage, input: editor.request.input, scope: editor.request.scope, status: editor.status }]); const value = text(setup(WorkflowProgress, { workspace, stage: 'foundation' }).render());
    assert.ok(value.includes('已完成 1 批资产整理 · 1 批资料整编')); assert.ok(value.includes('资料整编 · 第1–4集')); assert.ok(!value.includes('整理制作内容'));
  });
  await test('Cached foundation batch without delivery metadata counts success without inventing its scope', () => {
    const workspace = foundationProgress([{ id: 'cached-f2', stage: 'foundation:F2', status: 'accepted', input: { delivery: null, scope: null, episodeId: null } }]); const value = text(setup(WorkflowProgress, { workspace, stage: 'foundation' }).render());
    assert.ok(value.includes('已完成 1 批资产整理')); assert.ok(value.includes('资产整理 · 本批范围待核对')); assert.ok(!value.includes('第1集')); assert.ok(!value.includes('本批 1')); assert.ok(!value.includes('当前规划'));
  });
  await test('Foundation dynamic capacity replanning is described as current planning rather than a fixed total', () => {
    const workspace = foundationProgress([foundationNode('F2', ['ep-1', 'ep-2'], 0, 2, 'accepted'), foundationNode('F2', ['ep-3'], 1, 5, 'running')]); const value = text(setup(WorkflowProgress, { workspace, stage: 'foundation' }).render());
    assert.ok(value.includes('本批 1（当前规划 2 批）')); assert.ok(value.includes('本批 2（当前规划 5 批）')); assert.ok(value.includes('已完成 1 批资产整理')); assert.ok(!value.includes('1 / 5')); assert.ok(!value.includes('共 5')); assert.ok(!value.includes('总调用'));
  });
  await test('Foundation fragments and noncontiguous episodes show their real scope without inventing coverage', () => {
    const fragmented = foundationNode('F2', ['ep-1'], 1, 3); fragmented.request.input.delivery.fragment = { episodeId: 'ep-1', index: 1, count: 3, rowIds: ['row-3'] };
    const workspace = foundationProgress([fragmented, foundationNode('Q1', ['ep-2', 'ep-4', 'ep-5'], 0, 1)]), value = text(setup(WorkflowProgress, { workspace, stage: 'foundation' }).render());
    assert.ok(value.includes('资产整理 · 第1集')); assert.ok(value.includes('本集片段 2（当前规划 3 段）')); assert.ok(value.includes('资料整编 · 第2、4–5集')); assert.ok(!value.includes('第2–5集'));
  });
  await test('Legacy foundation v1 stays readable as its real old stage and receives no fabricated batch phases', () => {
    const workspace = foundationProgress([{ id: 'old-node', title: '整理制作内容', stage: 'foundation', status: 'accepted' }], { completedSteps: 1 }); const value = text(setup(WorkflowProgress, { workspace, stage: 'foundation' }).render());
    assert.ok(value.includes('整理制作内容')); assert.ok(value.includes('已核对 1 个步骤')); assert.ok(!value.includes('资产整理')); assert.ok(!value.includes('资料整编')); assert.ok(!value.includes('当前规划'));
  });
  await test('Foundation v2 keeps failures visible and resumes only the original workflow', async () => {
    const workspace = foundationProgress([foundationNode('F2', ['ep-7', 'ep-8'], 2, 4, 'rejected', { error: '本批形象证据不足' })], { status: 'failed', blocked: [{ reason: '仅第7–8集需要处理' }] }); const item = setup(WorkflowProgress, { workspace, stage: 'foundation' }); const value = text(item.render()); assert.ok(value.includes('本批形象证据不足')); assert.ok(value.includes('仅第7–8集需要处理')); assert.ok(value.includes('已完成 0 批资产整理')); await button(item.render(), '继续原流程').onClick(); await settle(); assert.deepEqual(calls, [{ action: 'workflows.resume', payload: { workflowId: 'foundation-flow' } }]);
  });
  await test('A single whole-scope F2 batch is not presented as ten per-episode full generations', () => {
    const workspace = foundationProgress([foundationNode('F2', Array.from({ length: 10 }, (_, index) => `ep-${index + 1}`), 0, 1, 'accepted')]), value = text(setup(WorkflowProgress, { workspace, stage: 'foundation' }).render());
    assert.ok(value.includes('资产整理 · 第1–10集 · 本批 1（当前规划 1 批）')); assert.ok(value.includes('已完成 1 批资产整理')); assert.equal(descendants(setup(WorkflowProgress, { workspace, stage: 'foundation' }).render()).filter(node => node.type?.name === 'Badge' && text(node) === '本批完成').length, 1);
  });
  await test('Partial progress resumes its own projected run and excludes other-run errors', async () => { const workspace = { ...base, jobs: [{ id: 'other-flow', kind: 'workflow', status: 'failed', provider: 'local', createdAt: '2026-10-01T00:00:00Z', error: '不属于当前流程的错误', snapshot: { stage: 'storyboard.run' } }, { id: 'flow-1', kind: 'workflow', status: 'partially_blocked', provider: 'local', createdAt: '2026-10-06T00:00:00Z', snapshot: { stage: 'storyboard.run' } }], workflowProgress: [{ workflowId: 'flow-1', stage: 'storyboard.run', status: 'partially_blocked', nodes: [] }] }; const item = setup(WorkflowProgress, { workspace, stage: 'storyboard' }); assert.ok(!text(item.render()).includes('不属于当前流程的错误')); await button(item.render(), '继续原流程').onClick(); await settle(); assert.deepEqual(calls[0], { action: 'workflows.resume', payload: { workflowId: 'flow-1' } }); });
  await test('Single-unit regeneration shares storyboard progress and blocks duplicate starts', () => { const workspace = { ...base, jobs: [{ id: 'rev-1', kind: 'workflow', status: 'running', provider: 'local', entityId: 'units.regenerate', snapshot: { stage: 'units.regenerate' } }], workflowProgress: [{ workflowId: 'rev-1', stage: 'units.regenerate', status: 'running', nodes: [] }] }; const item = setup(Stage3Panel, { workspace }); assert.equal(button(item.render(), '正在创作提示词').disabled, true); assert.equal(button(item.render(), '重做单元').disabled, true); const progress = setup(WorkflowProgress, { workspace, stage: 'storyboard' }); assert.ok(text(progress.render()).includes('分镜文字进度')); });
  await test('Decision selection sends only current candidate IDs and permits empty choice', async () => { const workspace = { ...base, workflowProgress: [{ workflowId: 'flow-1', stage: 'storyboard', status: 'waiting_decision' }] }; const item = setup(WorkflowProgress, { workspace, stage: 'storyboard' }); button(item.render(), '选择创作机会').onClick(); await settle(); const check = descendants(item.render()).find(node => node.type === 'input' && node.props.type === 'checkbox'); assert.ok(check); check.props.onChange({ target: { checked: false } }); await button(item.render(), '保存选择并继续').onClick(); await settle(); assert.deepEqual(calls[0], { action: 'storyboard.decision.resolve', payload: { workflowId: 'flow-1', selectedIds: [] } }); });
} finally { await replay.close(); delete globalThis.__productionHooks; delete globalThis.__productionRead; }
assert.equal(networkRequests, 0);
const report = { method: 'Actual React SSR + actual component callback replay with isolated hooks and read/mutation stubs', passed: cases.filter(item => item.passed).length, failed: cases.filter(item => !item.passed).length, networkRequests, productionDatabaseAccess: false, channelsCalled: false, installersBuilt: false, limitations: ['Callback replay does not prove backend acceptance or native browser layout', 'Real stage model output and workflow recovery require controller integration acceptance'], cases };
console.log(JSON.stringify(report, null, 2));
if (report.failed) process.exitCode = 1;
