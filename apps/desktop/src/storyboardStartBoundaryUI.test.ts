import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

Object.assign(globalThis, { isTauri: false, location: { search: '' } });

test('same frozen source and episode scope offers original continuation, not a new generation label', async () => {
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const { Stage3Panel, storyboardStartContinuation } = await server.ssrLoadModule('/src/Stage3Panel.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const workspace = structuredClone(previewWorkspace);
    workspace.source.version = 9; workspace.source.sha256 = 'a'.repeat(64);
    workspace.source.episodes = [{ id: 'e1', title: '第一集' }, { id: 'e2', title: '第二集' }];
    const job = { id: 'original', kind: 'workflow', entityId: 'storyboard.run', provider: 'local', status: 'partially_blocked', snapshot: { runnerVersion: 'desktop-runner/v1', sourceVersion: 9, selection: { episodeIds: ['e1', 'e2'] } } };
    workspace.jobs = [job];
    assert.equal(storyboardStartContinuation(workspace, ['e2', 'e1'])?.id, 'original');
    assert.equal(storyboardStartContinuation(workspace, ['e1']), null);
    assert.equal(storyboardStartContinuation(workspace, ['e1', 'e1']), null);
    let mutations = 0;
    const html = renderToStaticMarkup(createElement(Stage3Panel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; } }));
    assert.match(html, /继续原分镜流程/);
    assert.match(html, /已核对的步骤将复用/);
    assert.equal(mutations, 0);
    workspace.jobs = [{ ...job, status: 'completed' }, job];
    assert.equal(storyboardStartContinuation(workspace, ['e1', 'e2']), null);
    const completed = renderToStaticMarkup(createElement(Stage3Panel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; } }));
    assert.match(completed, /生成分镜提示词/);
    assert.doesNotMatch(completed, /同范围的分镜尚未完成/);
    assert.equal(mutations, 0);
  } finally { await server.close(); }
});

test('different source/units do not masquerade as continuation and known unknown remains disabled', async () => {
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const { Stage3Panel, storyboardStartContinuation } = await server.ssrLoadModule('/src/Stage3Panel.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const workspace = structuredClone(previewWorkspace);
    workspace.source.version = 9; workspace.source.sha256 = 'a'.repeat(64);
    workspace.source.episodes = [{ id: 'e1', title: '第一集' }];
    const job = { id: 'unknown-origin', kind: 'workflow', entityId: 'storyboard.run', provider: 'local', status: 'submission_unknown', submissionUnknown: true, snapshot: { runnerVersion: 'desktop-runner/v1', sourceVersion: 9, selection: { episodeIds: ['e1'] } } };
    workspace.jobs = [{ ...job, snapshot: { ...job.snapshot, sourceVersion: 8 } }];
    assert.equal(storyboardStartContinuation(workspace, ['e1']), null);
    workspace.jobs = [{ ...job, snapshot: { ...job.snapshot, input: { source: { version: 9, sha256: 'b'.repeat(64) }, selection: { episodeIds: ['e1'] } } } }];
    assert.equal(storyboardStartContinuation(workspace, ['e1']), null);
    workspace.jobs = [{ ...job, snapshot: { ...job.snapshot, selection: { episodeIds: ['e1'], unitIds: ['specific-unit'] } } }];
    assert.equal(storyboardStartContinuation(workspace, ['e1']), null);
    workspace.jobs = [{ ...job, id: 'newer-completed', status: 'completed', submissionUnknown: false }, job];
    assert.equal(storyboardStartContinuation(workspace, ['e1'])?.id, 'unknown-origin');
    let mutations = 0;
    const html = renderToStaticMarkup(createElement(Stage3Panel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; } }));
    const held = (html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g) || []).find(button => button.includes('保留原分镜任务'));
    assert.ok(held); assert.match(held, /^<button[^>]*disabled=""/);
    assert.match(html, /本项目有原提交待核对/);
    assert.equal(mutations, 0);
  } finally { await server.close(); }
});
