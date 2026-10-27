import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { foundationSavedProgress } from './foundationProgress.ts';
import { foundationCoverage } from './foundationCoverage.ts';
import type { Workspace } from './types.ts';

function node(key: string, first: number, last: number, phase = 'F2', status = 'accepted') {
  return { id: key, requestHash: key, stage: `foundation:${phase}`, status, input: { scope: Array.from({ length: last - first + 1 }, (_, i) => `e${first + i}`), delivery: { method: 'foundation-delivery/v2', batchIndex: 0, batchCount: 8, fragment: null } } };
}
function fixture(): Workspace {
  return { project: { id: 'p', title: '整理进度' }, source: { version: 1, sha256: 'source', episodes: Array.from({ length: 150 }, (_, i) => ({ id: `e${i + 1}`, number: i + 1, title: `第${i + 1}集` })) },
    foundation: { version: 1, sourceVersion: 1, sourceSha256: 'source', episodes: [] }, assets: [], media: [], units: [], rules: [], providers: [], settings: {},
    jobs: [{ id: 'run', kind: 'workflow', entityId: 'foundation.run', status: 'interrupted', provider: 'local', createdAt: '2026-10-27T10:00:00Z', snapshot: { runnerVersion: 'desktop-runner/v1', stage: 'foundation.run', sourceVersion: 1, selection: { episodeIds: Array.from({ length: 150 }, (_, i) => `e${i + 1}`) } } }],
    workflowProgress: [{ workflowId: 'run', stage: 'foundation.run', status: 'interrupted', nodes: [node('a', 1, 27), node('b', 28, 38), node('c', 39, 49), node('next', 50, 60, 'F2', 'running')] }] };
}

test('三批已保存1–49集与最终采用0集分别展示，读取投影不改变任务', () => {
  const workspace = fixture(), before = JSON.stringify(workspace), result = foundationSavedProgress(workspace);
  assert.equal(result.catalogue.batches, 3); assert.equal(result.catalogue.count, 49);
  assert.equal(result.catalogue.scopeLabel, '第1–49集'); assert.equal(result.review.count, 0);
  assert.match(result.catalogue.summary, /资产整理已保存 49 \/ 150 集 · 3 批/);
  assert.equal(foundationCoverage(workspace).count, 0); assert.equal(foundationCoverage(workspace).complete, false);
  assert.equal(JSON.stringify(workspace), before);
});

test('核对覆盖独立计数，重复缓存和未接受回包不累计', () => {
  const workspace = fixture(); workspace.workflowProgress![0].nodes = [node('a', 1, 27), node('a', 1, 27), node('b', 20, 49), node('q', 1, 12, 'Q1'), node('received', 50, 75, 'F2', 'received'), node('bad', 76, 100, 'F2', 'rejected')];
  const result = foundationSavedProgress(workspace);
  assert.equal(result.catalogue.batches, 2); assert.equal(result.catalogue.count, 49);
  assert.equal(result.review.batches, 1); assert.equal(result.review.count, 12);
  assert.match(result.recoveryLabel, /整理核对：第1–12集/);
});

test('三路冲突修订和统一收口单独显示，不重复增加集数与批数', () => {
  const workspace = fixture();
  const repaired = node('repair', 1, 27);
  const final = node('final', 1, 150, 'Q1');
  Object.assign(repaired.input, { parallelTrial: { version: 'foundation-three-lane-trial/v1', laneId: 1, kind: 'rebase' } });
  Object.assign(final.input, { parallelTrial: { version: 'foundation-three-lane-trial/v1', laneId: 0, kind: 'final' } });
  workspace.workflowProgress![0].nodes = [node('a', 1, 27), repaired, node('q', 1, 12, 'Q1'), final];
  const result = foundationSavedProgress(workspace);
  assert.equal(result.catalogue.batches, 1);
  assert.equal(result.catalogue.count, 27);
  assert.equal(result.catalogue.repairsSaved, 1);
  assert.match(result.catalogue.summary, /1 次冲突修订已保存/);
  assert.equal(result.review.batches, 1);
  assert.equal(result.review.count, 12);
  assert.equal(result.review.finalizationsSaved, 1);
  assert.match(result.review.summary, /统一收口已保存/);
});

test('分片只报告可恢复片段，不把动态count当作整集覆盖证明', () => {
  const workspace = fixture(), partial = node('part', 50, 50);
  Object.assign(partial.input.delivery, { fragment: { episodeId: 'e50', index: 0, count: 1 } });
  workspace.workflowProgress![0].nodes = [node('a', 1, 27), partial];
  const result = foundationSavedProgress(workspace);
  assert.equal(result.catalogue.count, 27); assert.equal(result.catalogue.batches, 2);
  assert.deepEqual(result.catalogue.partialIds, ['e50']); assert.match(result.recoveryLabel, /第50集的片段/);
});

test('新流程不会继承旧进度；源变更和未知集号只保留历史提示', () => {
  const workspace = fixture(); workspace.jobs.unshift({ id: 'new', kind: 'workflow', entityId: 'foundation.run', provider: 'local', status: 'queued', createdAt: '2026-10-27T11:00:00Z' });
  assert.equal(foundationSavedProgress(workspace).hasSaved, false);
  workspace.jobs.shift(); workspace.source!.version = 2;
  assert.equal(foundationSavedProgress(workspace).catalogue.count, 0);
  assert.equal(foundationSavedProgress(workspace).unboundBatches, 3);
  workspace.source!.version = 1; workspace.source!.episodes = [{ id: 'different', number: 1 }];
  assert.equal(foundationSavedProgress(workspace).catalogue.count, 0);
});

test('真实轻量节点不含source哈希，以父任务input.source身份绑定；缺身份不推断', () => {
  const workspace = fixture();
  workspace.jobs[0].snapshot = { sourceVersion: 0, input: { source: { version: 1, sha256: 'source' } } };
  assert.equal(foundationSavedProgress(workspace).catalogue.count, 49);
  workspace.jobs[0].snapshot = { sourceVersion: 1, input: { source: { version: 2, sha256: 'source' } } };
  assert.equal(foundationSavedProgress(workspace).catalogue.count, 0);
  workspace.jobs[0].snapshot = { sourceVersion: 1, input: { source: { version: 1, sha256: 'different' } } };
  assert.equal(foundationSavedProgress(workspace).catalogue.count, 0);
  workspace.jobs[0].snapshot = {};
  assert.equal(foundationSavedProgress(workspace).catalogue.count, 0);
});

test('实际第一阶段组件显示保存范围、核对和最终采用，不再暗示150集全部未整理', async () => {
  Object.assign(globalThis, { isTauri: false, location: { search: '' } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const { Stage1Panel } = await server.ssrLoadModule('/src/Stage1Panel.tsx');
    const workspace = fixture(); let mutations = 0;
    const html = renderToStaticMarkup(createElement(Stage1Panel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; } }));
    assert.match(html, /资产整理已保存 49 \/ 150 集 · 3 批/);
    assert.match(html, /整理核对已保存 0 \/ 150 集 · 0 批/);
    assert.match(html, /最终采用 0 \/ 150 集/);
    assert.match(html, /已保存范围：资产整理：第1–49集/);
    assert.match(html, /不需要从头整理/); assert.match(html, /本批已保存/);
    assert.doesNotMatch(html, /已整理 0 \/ 150|150 集未整理|已完成 3 批资产整理/);
    assert.match(html, /继续原制作基础流程/); assert.equal(mutations, 0);
    const { WorkflowProgress } = await server.ssrLoadModule('/src/WorkflowProgress.tsx');
    const parallelNodes = [node('lane', 50, 60, 'F2', 'running'), node('repair', 28, 44), node('final', 1, 150, 'Q1')];
    workspace.workflowProgress![0].nodes = parallelNodes;
    for (const [index, kind] of ['batch', 'rebase', 'final'].entries()) {
      Object.assign(parallelNodes[index].input, { parallelTrial: { version: 'foundation-three-lane-trial/v1', laneId: index, kind } });
    }
    const progressHtml = renderToStaticMarkup(createElement(WorkflowProgress, { workspace, stage: 'foundation', writable: true, mutate: async () => { mutations++; return workspace; } }));
    assert.match(progressHtml, /第50–60集 · 通道 1/);
    assert.match(progressHtml, /冲突修订 · 通道 2/);
    assert.match(progressHtml, /统一收口 · 通道 3/);
    assert.equal(mutations, 0);
    workspace.jobs[0].status = 'completed';
    workspace.workflowProgress![0].status = 'completed';
    workspace.foundationConflictReview = { pendingCount: 2 };
    const pendingHtml = renderToStaticMarkup(createElement(WorkflowProgress, { workspace, stage: 'foundation', writable: true, mutate: async () => { mutations++; return workspace; } }));
    assert.match(pendingHtml, /整理完成 · 2 项待核对/);
    assert.match(pendingHtml, /未计为全部通过/);
    assert.doesNotMatch(pendingHtml, /全剧已整理/);
    assert.equal(mutations, 0);
  } finally { await server.close(); }
});
