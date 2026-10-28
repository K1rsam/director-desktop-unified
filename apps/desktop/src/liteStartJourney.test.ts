import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import React from 'react';
import { createServer } from 'vite';

Object.assign(globalThis, { isTauri: false, location: { search: '' } });
const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
after(async () => { await server.close(); });
const { Stage1Panel } = await server.ssrLoadModule('/src/Stage1Panel.tsx');
const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
const { installExecutionPolicy } = await server.ssrLoadModule('/src/executionPolicy.ts');
installExecutionPolicy({ mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', reasons: [], allowedActions: ['storyboard.run', 'production.direction.save', 'source.analyze', 'foundation.run', 'workflows.resume'] });
function nodes(tree: any): any[] { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(nodes)]; }
function text(tree: any): string { return typeof tree === 'string' || typeof tree === 'number' ? String(tree) : React.Children.toArray(tree?.props?.children).map(text).join(''); }
function view() {
  const workspace = structuredClone(previewWorkspace);
  workspace.source.version = 9; workspace.source.sha256 = 'a'.repeat(64); workspace.source.diagnostics = [];
  workspace.source.episodes = Array.from({ length: 150 }, (_, i) => ({ id: `e${i + 1}`, title: `第${i + 1}集`, number: i + 1 }));
  workspace.foundation = null; workspace.assets = []; workspace.units = []; workspace.jobs = []; workspace.workflowProgress = [];
  return workspace;
}
function harness(initial: any) {
  let workspace = initial, opened = 0, index = 0; const hooks: any[] = [], calls: any[][] = [], notices: string[] = [];
  const mutate = async (...args: any[]) => {
    calls.push(args);
    if (args[0] === 'production.direction.save') workspace = { ...workspace, foundation: { version: 1, sourceVersion: 9, sourceSha256: 'a'.repeat(64), productionDirection: args[1].direction, episodes: [] } };
    if (args[0] === 'storyboard.run') workspace = { ...workspace, jobs: [{ id: 'new-lite', kind: 'workflow', entityId: 'storyboard.run', status: 'running', snapshot: { stage: 'storyboard.run', input: { workflowProfile: args[1].workflowProfile, selection: args[1].options.selection, source: workspace.source } } }] };
    return workspace;
  };
  function render() {
    index = 0;
    const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE, previous = internals.H;
    internals.H = {
      useState(initial: any) { const slot = index++; if (!(slot in hooks)) hooks[slot] = typeof initial === 'function' ? initial() : initial; return [hooks[slot], (v: any) => { hooks[slot] = typeof v === 'function' ? v(hooks[slot]) : v; }]; },
      useEffect() {}, useRef(initial: any) { const slot = index++; return hooks[slot] ||= { current: initial }; },
    };
    try { return Stage1Panel({ workspace, writable: true, mutate, onStoryboard: () => { opened++; }, onNotice: (s: string) => notices.push(s) }); }
    finally { internals.H = previous; }
  }
  const main = () => nodes(render()).find(node => node.props?.variant === 'primary' && ['storyboard.run', 'source.analyze', 'foundation.run'].includes(node.props?.executionAction));
  return { render, main, calls, notices, get opened() { return opened; }, get workspace() { return workspace; } };
}
async function settled() { for (let i = 0; i < 8; i++) await Promise.resolve(); }

test('150集新项目确认方向后直接从原主入口启动首集，不先整理全剧资产', async () => {
  const h = harness(view()), source = JSON.stringify(h.workspace.source);
  assert.equal(text(h.main()), '确认并开始首集');
  h.main().props.onClick(); await settled();
  assert.equal(h.calls.length, 0, 'an empty direction opens its confirmation form without any model call');
  let textarea = nodes(h.render()).find(node => node.type === 'textarea' && node.props.className === 'production-direction-input');
  assert.ok(textarea); textarea.props.onChange({ target: { value: '原文守源，以人物行动推进，使用已确认视觉基准。' } });
  h.main().props.onClick(); await settled();
  assert.deepEqual(h.calls.map(call => call[0]), ['production.direction.save', 'storyboard.run']);
  assert.deepEqual(h.calls[1][1], { workflowProfile: 'desktop-series-lite/v1', episodeIds: ['e1'], options: { selection: { episodeIds: ['e1'] } } });
  assert.equal(h.opened, 1); assert.equal(h.workspace.assets.length, 0); assert.equal(h.workspace.foundation.episodes.length, 0);
  assert.equal(JSON.stringify(h.workspace.source), source);
  assert.equal(h.main().props.disabled, true); h.main().props.onClick(); await settled(); assert.equal(h.calls.length, 2, 'the active project lease is respected');
});
test('首批范围可选，格式识别不自动启动foundation或制作', async () => {
  const workspace = view(); workspace.foundation = { version: 1, productionDirection: '已确认方向', sourceSha256: workspace.source.sha256, sourceVersion: 9, episodes: [] };
  const h = harness(workspace);
  nodes(h.render()).find(node => node.props?.['aria-label'] === '首批创作范围').props.onChange({ target: { value: '3' } });
  h.main().props.onClick(); await settled();
  assert.equal(h.calls.length, 1); assert.deepEqual(h.calls[0][1].episodeIds, ['e1', 'e2', 'e3']);
  const unresolved = view(); unresolved.source.diagnostics = [{ code: 'BOUNDARY_UNCONFIRMED', blocking: true, message: '格式待识别', rowIds: [] }];
  const format = harness(unresolved); format.main().props.onClick(); await settled();
  assert.equal(format.calls.length, 1); assert.equal(format.calls[0][0], 'source.analyze'); assert.equal(format.calls[0][1].continueFoundation, false); assert.equal(format.opened, 0);
});
test('精神小妹原foundation范围及UNKNOWN保留原恢复合同，不切换成lite', async () => {
  for (const unknown of [false, true]) {
    const workspace = view(); workspace.project.title = '精神小妹';
    workspace.jobs = [{ id: 'original-foundation', kind: 'workflow', entityId: 'foundation.run', status: unknown ? 'submission_unknown' : 'partially_blocked', submissionUnknown: unknown, snapshot: { runnerVersion: 'desktop-runner/v1', stage: 'foundation.run', input: { source: workspace.source, selection: { episodeIds: ['e28', 'e29'] } } } }];
    const h = harness(workspace), main = h.main(); assert.equal(main.props.executionAction, 'foundation.run'); assert.equal(main.props.disabled, unknown);
    main.props.onClick(); await settled();
    assert.equal(h.calls.length, unknown ? 0 : 1);
    if (!unknown) { assert.equal(h.calls[0][0], 'foundation.run'); assert.deepEqual(h.calls[0][1], { options: { selection: { episodeIds: ['e28', 'e29'] } } }); }
    assert.equal(h.opened, 0);
  }
});
