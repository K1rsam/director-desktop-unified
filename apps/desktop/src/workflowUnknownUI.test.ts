import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

Object.assign(globalThis, { isTauri: false, location: { search: '' } });
const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
after(async () => { await server.close(); });
const { WorkflowProgress } = await server.ssrLoadModule('/src/WorkflowProgress.tsx');
const { Stage3Panel } = await server.ssrLoadModule('/src/Stage3Panel.tsx');
const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
const { installExecutionPolicy } = await server.ssrLoadModule('/src/executionPolicy.ts');
installExecutionPolicy({ mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', reasons: [], allowedActions: ['workflows.resume', 'storyboard.run'] });
function nodes(tree: any): any[] { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(nodes)]; }
function content(tree: any): string { return typeof tree === 'string' || typeof tree === 'number' ? String(tree) : React.Children.toArray(tree?.props?.children).map(content).join(''); }
function view(unknown: boolean) {
  const workspace = structuredClone(previewWorkspace);
  workspace.source.version = 9; workspace.source.sha256 = 'a'.repeat(64);
  workspace.source.episodes = [{ id: 'e1', title: '第一集' }, { id: 'e2', title: '第二集' }];
  workspace.workflowProgress = [];
  workspace.jobs = [{ id: 'original', kind: 'workflow', provider: 'local', entityId: 'storyboard.run', status: 'partially_blocked', submissionUnknown: unknown,
    error: 'CLI_SUBMISSION_UNKNOWN is only historical text, not classification evidence',
    snapshot: { runnerVersion: 'desktop-runner/v1', sourceVersion: 9, selection: { episodeIds: ['e1', 'e2'] } } }];
  return workspace;
}

test('unknown original results keep a truthful replay action without promising generation or parsing error strings', async () => {
  for (const unknown of [false, true]) {
    const workspace = view(unknown), before = JSON.stringify(workspace), calls: unknown[][] = [];
    const mutate = async (...args: unknown[]) => { calls.push(args); return workspace; };
    // Invoke the actual component and its actual button handler; hooks are local
    // only, and the mutation boundary is captured rather than contacting a host.
    const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
    const previous = internals.H;
    internals.H = { useState(initial: any) { return [typeof initial === 'function' ? initial() : initial, () => {}]; }, useEffect() {} };
    let tree: any;
    try { tree = WorkflowProgress({ workspace, stage: 'storyboard', writable: true, mutate }); } finally { internals.H = previous; }
    const label = unknown ? '核对原结果后继续' : '继续原流程';
    const button = nodes(tree).find(node => node.props?.executionAction === 'workflows.resume');
    assert.ok(button); assert.equal(content(button), label); assert.equal(button.props.disabled, false);
    assert.equal(content(tree).includes('原提交待核对'), unknown);
    assert.equal(content(tree).includes('先核对已收到的原结果；原提交仍未知时不会重新发送'), unknown);
    button.props.onClick(); await Promise.resolve();
    assert.equal(calls.length, 1); assert.equal(calls[0][0], 'workflows.resume');
    assert.deepEqual(calls[0][1], { workflowId: 'original' });
    assert.equal(String(calls[0][2]).includes('已继续原流程'), !unknown);
    assert.equal(JSON.stringify(workspace), before, 'original history is not rewritten by presentation or replay intent');
  }
});

test('legacy CLI projection blocks fresh Stage3 across episode selections while leaving original result checking available', () => {
  const workspace = view(true);
  workspace.jobs[0].cliSubmissionUnknown = true;
  workspace.jobs[0].snapshot.selection.episodeIds = ['e1'];
  let requests = 0;
  const html = renderToStaticMarkup(React.createElement(Stage3Panel, { workspace, writable: true, mutate: async () => { requests++; return workspace; } }));
  const buttons = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g) || [];
  const fresh = buttons.find(button => button.includes('保留原分镜任务'));
  assert.ok(fresh); assert.match(fresh, /^<button[^>]*disabled=""/);
  assert.match(html, /本项目有原提交待核对/); assert.match(html, /原提交待核对/);
  const replay = buttons.find(button => button.includes('核对原结果后继续'));
  assert.ok(replay); assert.doesNotMatch(replay, /^<button[^>]*disabled=""/);
  assert.equal(requests, 0);
});

test('unknown submission keeps the original shutdown error only inside collapsed history', () => {
  const oldError = '客户端正在关闭，未启动新的文本任务';
  for (const [unknown, state, label] of [
    [true, 'partially_blocked', '原中断记录（保留原文）'],
    [true, 'running', '原中断记录（保留原文）'],
    [false, 'running', '上次中断记录 · 当前正在继续'],
  ] as const) {
    const workspace = view(unknown);
    workspace.jobs[0].status = state; workspace.jobs[0].error = oldError;
    const before = JSON.stringify(workspace); let requests = 0;
    const html = renderToStaticMarkup(React.createElement(WorkflowProgress, { workspace, stage: 'storyboard', writable: true, mutate: async () => { requests++; return workspace; } }));
    const history = (html.match(/<details\b[^>]*>[\s\S]*?<\/details>/g) || []).find(value => value.includes(`<summary>${label}</summary>`));
    assert.ok(history, label);
    assert.doesNotMatch(history, /^<details[^>]*\bopen(?:=|\s|>)/, 'old interruption is collapsed initially');
    assert.ok(history.includes(oldError), 'preserve the exact original error');
    const current = html.replace(history, '');
    assert.ok(!current.includes(oldError), 'historical shutdown does not masquerade as the current warning');
    assert.equal(current.includes('原提交待核对'), unknown);
    assert.equal(JSON.stringify(workspace), before); assert.equal(requests, 0);
  }
});

test('原流程恢复尊重同项目真实制作占用，释放后保留原ID和UNKNOWN对账语义', async () => {
  for (const unknown of [false, true]) {
    const workspace = view(unknown), calls: unknown[][] = [];
    const mutate = async (...args: unknown[]) => { calls.push(args); return workspace; };
    const render = () => {
      const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
      const previous = internals.H;
      internals.H = { useState(initial: any) { return [typeof initial === 'function' ? initial() : initial, () => {}]; }, useEffect() {} };
      try { return WorkflowProgress({ workspace, stage: 'storyboard', writable: true, mutate }); }
      finally { internals.H = previous; }
    };
    for (const active of [
      { id: 'real-worker', kind: 'workflow', entityId: 'foundation.run', provider: 'local', status: 'running' },
      { id: 'real-worker', kind: 'workflow', entityId: 'images.design:batch', provider: 'local', status: 'queued' },
    ]) {
      workspace.jobs.push(active);
      const before = JSON.stringify(workspace), tree = render();
      const resume = nodes(tree).find(node => node.props?.executionAction === 'workflows.resume');
      assert.ok(resume); assert.equal(resume.props.disabled, true);
      assert.equal(content(resume), '等待当前制作完成');
      assert.match(content(tree), /已保存成果仍可阅读/);
      resume.props.onClick(); await Promise.resolve();
      assert.deepEqual(calls, [], 'even a stale or programmatic click does not dispatch against the occupied pipeline');
      assert.equal(JSON.stringify(workspace), before);
      workspace.jobs.pop();
    }
    workspace.jobs.push({ id: 'media-only', kind: 'image', entityId: 'asset', provider: 'dreamina', status: 'processing' });
    const tree = render(), resume = nodes(tree).find(node => node.props?.executionAction === 'workflows.resume');
    assert.ok(resume); assert.equal(resume.props.disabled, false, 'media polling does not own the text pipeline');
    assert.equal(content(resume), unknown ? '核对原结果后继续' : '继续原流程');
    resume.props.onClick(); await Promise.resolve();
    assert.equal(calls.length, 1); assert.equal(calls[0][0], 'workflows.resume');
    assert.deepEqual(calls[0][1], { workflowId: 'original' });
    assert.equal(String(calls[0][2]).includes('原提交仍未知时不会重新发送'), unknown);
  }
});
