import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { selectAssetDesignProgress } from './assetDesignProgress.ts';
import type { Workspace } from './types.ts';

Object.assign(globalThis, { isTauri: false, location: { search: '' } });

const start = Date.parse('2026-10-08T10:00:00Z'), now = start + 242000;
const design = (assetId: string, status = 'accepted', id = assetId) => ({ id, stage: 'assetDesign', status, input: { assetId } });
const packet = (id: string, status = 'accepted', repair = 'a') => ({ id: `assetVisualFacts:packet:${id.repeat(64)}:repair:${repair.repeat(64)}`, stage: 'assetVisualFacts', status, input: {} });
function fixture(): Workspace {
  const designAssetIds = Array.from({ length: 57 }, (_, n) => `shape-${n}`);
  return { project: { id: 'synthetic-progress', title: '合成设计进度' }, assets: [...designAssetIds, 'inline'].map(id => ({ id, name: id, kind: 'PROP', prompt: '', version: 1, preparation: { mode: id === 'inline' ? 'INLINE' : 'REFERENCE' } })), media: [], units: [], settings: {}, providers: [], rules: [], jobs: [
    { id: 'flow', kind: 'workflow', provider: 'local', entityId: 'images.design:', status: 'running', createdAt: new Date(start).toISOString() },
    { id: 'last-child', kind: 'text', provider: 'codex_sdk', entityId: 'assetDesign:shape-2', status: 'submitting', createdAt: new Date(now - 2000).toISOString() },
  ], workflowProgress: [{ workflowId: 'flow', stage: 'images.design:', status: 'running', completedSteps: 0, assetDesignPlan: { designAssetIds, inlineAssetIds: ['inline'] }, nodes: [packet('b'), design('shape-0'), design('shape-1'), design('shape-2', 'running')] }] };
}

test('已核验设计在原子发布前显示2/57，1项INLINE不算独立设计，父流程耗时不随子任务归零', () => {
  const workspace = fixture(), before = JSON.stringify(workspace), value = selectAssetDesignProgress(workspace, now)!;
  assert.equal(value.designed, 2); assert.equal(value.total, 57); assert.equal(value.inlineCount, 1); assert.equal(value.factBatches, 1);
  assert.equal(value.elapsedSeconds, 242); assert.equal(value.phase, '设计资产提示词');
  assert.match(value.designLabel, /已设计 2\/57 项提示词.*1 项复用来源/);
  assert.equal(workspace.assets.filter(asset => asset.prompt).length, 0);
  workspace.jobs[1].createdAt = new Date(now).toISOString();
  assert.equal(selectAssetDesignProgress(workspace, now + 3000)!.elapsedSeconds, 245);
  workspace.jobs[1].createdAt = new Date(now - 2000).toISOString();
  assert.equal(JSON.stringify(workspace), before, '投影不修改提示词、采用或冻结任务');
});

test('只计算当前workflow的accepted形象，重复恢复按assetId去重，拒绝及received不能冒充完成', () => {
  const workspace = fixture(), progress = workspace.workflowProgress![0];
  progress.nodes = [design('shape-0'), design('shape-0', 'accepted', 'retry'), design('shape-1', 'rejected'), design('shape-2', 'received'), design('outside')];
  workspace.workflowProgress!.unshift({ workflowId: 'older', stage: 'images.design', nodes: [design('shape-1')] });
  assert.equal(selectAssetDesignProgress(workspace, now)!.designed, 1);
  assert.equal(selectAssetDesignProgress(workspace, now)!.runningDesigns, 1);
});

test('事实提取修复按同packet去重并显示正在提取，不将repair轮次当更多事实批次', () => {
  const workspace = fixture();
  workspace.workflowProgress![0].nodes = [packet('b'), packet('b', 'accepted', 'c'), packet('d', 'running'), packet('e', 'rejected')];
  const value = selectAssetDesignProgress(workspace, now)!;
  assert.equal(value.factBatches, 1); assert.equal(value.runningFacts, 1); assert.equal(value.phase, '提取原文视觉事实');
  assert.match(value.factLabel, /已提取 1 批，正在提取/);
});

test('冻结计划不随当前库58→1变化；缺失或畸形分母降级为本次数量且不猜目录长度', () => {
  const workspace = fixture(); workspace.assets = [];
  assert.equal(selectAssetDesignProgress(workspace, now)!.total, 57);
  delete workspace.workflowProgress![0].assetDesignPlan;
  assert.equal(selectAssetDesignProgress(workspace, now)!.total, null);
  assert.equal(selectAssetDesignProgress(workspace, now)!.designLabel, '本次已设计 2 项提示词');
  for (const plan of [{ designAssetIds: ['shape-0', 'shape-0'], inlineAssetIds: [] }, { designAssetIds: ['shape-0'], inlineAssetIds: ['shape-0'] }, { designAssetIds: '57', inlineAssetIds: [] }]) {
    workspace.workflowProgress![0].assetDesignPlan = plan;
    assert.equal(selectAssetDesignProgress(workspace, now)!.total, null);
  }
});

test('新流程未到首个checkpoint不借旧流程/已发布提示词显示进度，旧字段缺失不冒称零完成', () => {
  const workspace = fixture(); workspace.jobs.unshift({ id: 'new-flow', kind: 'workflow', entityId: 'images.design:', provider: 'local', status: 'running', createdAt: new Date(now - 1000).toISOString() });
  workspace.assets.forEach(asset => { asset.prompt = '旧已发布提示词'; });
  const value = selectAssetDesignProgress(workspace, now)!;
  assert.equal(value.workflowId, 'new-flow'); assert.equal(value.designed, null); assert.equal(value.factBatches, null); assert.equal(value.total, null);
  assert.equal(value.designLabel, '正在读取本次设计进度');
  workspace.jobs.shift(); workspace.workflowProgress![0].nodes = [{ id: 'old-shape-key', stage: 'assetDesign', status: 'accepted', input: {} }];
  assert.equal(selectAssetDesignProgress(workspace, now)!.designed, null);
});

test('produce自动缓存跳过时不套design分母，缺开始时间显示待刷新', () => {
  const workspace = fixture(); workspace.jobs[0].entityId = 'images.produce:'; workspace.workflowProgress![0].stage = 'images.produce:';
  assert.equal(selectAssetDesignProgress(workspace, now)!.total, null);
  assert.equal(selectAssetDesignProgress(workspace, now)!.designLabel, '本次已设计 2 项提示词');
  workspace.jobs[0].createdAt = 'invalid';
  assert.equal(selectAssetDesignProgress(workspace, now)!.elapsedSeconds, null);
});

test('未知提交不计时假跑、失败保留accepted数量，读取失败停在上次核对时间，完成后隐藏旧失败', () => {
  const workspace = fixture(); workspace.jobs[0].submissionUnknown = true;
  let value = selectAssetDesignProgress(workspace, now)!;
  assert.equal(value.active, false); assert.equal(value.unknown, true); assert.match(value.phase, /执行身份待核对/);
  workspace.jobs[0].submissionUnknown = false; workspace.jobs[0].status = 'failed'; workspace.jobs[0].updatedAt = new Date(start + 180000).toISOString();
  value = selectAssetDesignProgress(workspace, now)!; assert.equal(value.designed, 2); assert.equal(value.elapsedSeconds, 180);
  workspace.jobs[0].status = 'running'; workspace.readState = { error: '合成读取失败', checkedAt: new Date(start + 120000).toISOString() };
  value = selectAssetDesignProgress(workspace, now)!; assert.equal(value.readFailed, true); assert.equal(value.elapsedSeconds, 120);
  workspace.jobs[0].status = 'completed'; workspace.jobs.push({ id: 'old-failure', kind: 'workflow', entityId: 'images.design:', provider: 'local', status: 'failed', createdAt: new Date(start - 1000).toISOString() });
  assert.equal(selectAssetDesignProgress(workspace, now), null);
});

test('真实AssetPanel渲染当前checkpoint数量及整批保存边界，不读账户或执行模型/媒体', async () => {
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const { AssetPanel } = await server.ssrLoadModule('/src/AssetPanel.tsx');
    let mutations = 0; const workspace = fixture();
    const html = renderToStaticMarkup(createElement(AssetPanel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; }, inspect() {}, onNotice() {}, onError() {}, onTasks() {} }));
    assert.match(html, /aria-label="图片制作进度"/); assert.match(html, /已设计 2\/57 项提示词/); assert.match(html, /1 项复用来源或仅作文字表现/);
    assert.match(html, /整个流程已用时/); assert.match(html, /原文事实：本次已提取 1 批/); assert.match(html, /完整流程结束后整批保存到资产库/);
    assert.doesNotMatch(html, /本步已运行|已有 0 项提示词/); assert.equal(mutations, 0); assert.ok(workspace.assets.every(asset => !asset.prompt));
  } finally { await server.close(); }
});
