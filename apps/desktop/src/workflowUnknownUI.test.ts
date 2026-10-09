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
