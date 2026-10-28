import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import React from 'react';
import { createServer } from 'vite';
import { executionSummary, storyboardCompletion, storyboardNodeTitle } from './workflowProgressModel.ts';
import { SERIES_LITE_WORKFLOW_PROFILE, SERIES_WORKFLOW_PROFILE } from './workflowNavigation.ts';

Object.assign(globalThis, { isTauri: false, location: { search: '' } });
const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
after(async () => { await server.close(); });
const { Stage3Panel } = await server.ssrLoadModule('/src/Stage3Panel.tsx');
const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
const { installExecutionPolicy } = await server.ssrLoadModule('/src/executionPolicy.ts');
installExecutionPolicy({ mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', reasons: [], allowedActions: ['storyboard.run'] });
function nodes(tree: any): any[] { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(nodes)]; }
function view() {
  const workspace = structuredClone(previewWorkspace);
  workspace.source.version = 9; workspace.source.sha256 = 'a'.repeat(64);
  workspace.source.episodes = [{ id: 'e1', number: 1, title: '第一集' }, { id: 'e2', number: 2, title: '第二集' }];
  workspace.source.diagnostics = []; workspace.units = []; workspace.jobs = []; workspace.workflowProgress = [];
  return workspace;
}
function render(workspace: any, calls: unknown[][]) {
  const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE, previous = internals.H;
  internals.H = { useState(initial: any) { return [typeof initial === 'function' ? initial() : initial, () => {}]; }, useEffect() {}, useRef(initial: any) { return { current: initial }; } };
  try { return Stage3Panel({ workspace, writable: true, mutate: async (...args: unknown[]) => { calls.push(args); return workspace; } }); }
  finally { internals.H = previous; }
}
test('新分镜主按钮明确使用 lite 路由，图片阶段旧常量保持不变', async () => {
  const workspace = view(), before = JSON.stringify(workspace), calls: unknown[][] = [];
  const button = nodes(render(workspace, calls)).find(node => node.props?.executionAction === 'storyboard.run');
  assert.ok(button); assert.equal(button.props.disabled, false);
  button.props.onClick(); await Promise.resolve();
  assert.equal(calls.length, 1); assert.equal(calls[0][0], 'storyboard.run');
  assert.deepEqual(calls[0][1], { workflowProfile: SERIES_LITE_WORKFLOW_PROFILE, options: { selection: { episodeIds: ['e1'] } }, episodeIds: ['e1'] });
  assert.equal(SERIES_WORKFLOW_PROFILE, 'desktop-series672-assets112/v1');
  assert.equal(JSON.stringify(workspace), before);
});
test('旧版或新版原范围恢复都不指定新规则；UNKNOWN 连直接调用按钮也不提交', async () => {
  for (const profile of [SERIES_WORKFLOW_PROFILE, SERIES_LITE_WORKFLOW_PROFILE]) for (const unknown of [false, true]) {
    const workspace = view(), calls: unknown[][] = [];
    workspace.jobs = [{ id: 'original', kind: 'workflow', entityId: 'storyboard.run', status: 'interrupted', submissionUnknown: unknown,
      snapshot: { runnerVersion: 'desktop-runner/v1', input: { workflowProfile: profile, source: { version: 9, sha256: 'a'.repeat(64) }, selection: { episodeIds: ['e1', 'e2'] } } } }];
    const before = JSON.stringify(workspace), button = nodes(render(workspace, calls)).find(node => node.props?.executionAction === 'storyboard.run');
    assert.ok(button); assert.equal(button.props.disabled, unknown);
    button.props.onClick(); await Promise.resolve();
    assert.equal(calls.length, unknown ? 0 : 1);
    if (!unknown) assert.deepEqual(calls[0][1], { options: { selection: { episodeIds: ['e1', 'e2'] } }, episodeIds: ['e1', 'e2'] });
    assert.equal(JSON.stringify(workspace), before);
  }
});
test('lite 岗位名称和实际集数可读，SDK 最近活动仍由真实事件显示', () => {
  const workspace = view();
  const stages = [['litePrepare', '本集资料准备'], ['series-lite:author', '整集主创'], ['series-lite:finisher', '整集整编'], ['series-lite:reviewer', '独立审稿']];
  for (const [stage, title] of stages) {
    assert.equal(storyboardNodeTitle({ stage, input: stage === 'litePrepare' ? { episodeId: 'e2' } : { roleContext: { episode: '2' } } }, workspace), `${title} · 第二集`);
  }
  assert.equal(storyboardCompletion(stages.map(([stage]) => ({ stage, status: 'accepted' }))), '1 项资料准备 · 1 项整集主创 · 1 项整集整编 · 1 项独立审稿');
  const stamp = Date.parse('2026-10-28T08:00:00Z');
  const node = { stage: 'series-lite:author', status: 'running', execution: { backend: 'codex_sdk', startedAt: new Date(stamp).toISOString(), phase: 'executing', lastEventType: 'item.updated', lastEventAt: new Date(stamp + 1000).toISOString() } };
  assert.match(executionSummary(node, stamp + 6000), /最后执行事件：输出项更新（5 秒前）/);
});
