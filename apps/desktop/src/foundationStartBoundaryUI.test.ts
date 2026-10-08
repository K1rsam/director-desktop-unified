import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

Object.assign(globalThis, { isTauri: false, location: { search: '' } });

test('failed foundation scope offers original continuation while completed scope permits a new run', async () => {
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const { Stage1Panel, foundationStartContinuation } = await server.ssrLoadModule('/src/Stage1Panel.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const workspace = structuredClone(previewWorkspace);
    workspace.source.version = 9; workspace.source.sha256 = 'a'.repeat(64);
    workspace.source.episodes = [{ id: 'e1', title: '第一集' }, { id: 'e2', title: '第二集' }];
    const job = { id: 'original-foundation', kind: 'workflow', entityId: 'foundation.run', provider: 'local', status: 'partially_blocked', snapshot: { runnerVersion: 'desktop-runner/v1', stage: 'foundation.run', sourceVersion: 9, selection: { episodeIds: ['e1', 'e2'] } } };
    workspace.jobs = [job];
    assert.equal(foundationStartContinuation(workspace, ['e2', 'e1'])?.id, 'original-foundation');
    assert.equal(foundationStartContinuation(workspace, ['e1']), null);
    assert.equal(foundationStartContinuation(workspace, ['e1', 'e1']), null);
    let mutations = 0;
    const html = renderToStaticMarkup(createElement(Stage1Panel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; } }));
    assert.match(html, /继续原制作基础流程/);
    assert.match(html, /已核对步骤按原冻结输入复用/);
    assert.equal(mutations, 0);
    workspace.jobs = [{ ...job, status: 'completed' }, job];
    assert.equal(foundationStartContinuation(workspace, ['e1', 'e2']), null);
    const completed = renderToStaticMarkup(createElement(Stage1Panel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; } }));
    assert.match(completed, /整理全剧制作基础/);
    assert.doesNotMatch(completed, /同范围制作基础尚未完成/);
    assert.equal(mutations, 0);
  } finally { await server.close(); }
});

test('foundation unknown overlaps stay held despite newer completion and other stages or sources are excluded', async () => {
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const { Stage1Panel, foundationStartContinuation } = await server.ssrLoadModule('/src/Stage1Panel.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const workspace = structuredClone(previewWorkspace);
    workspace.source.version = 9; workspace.source.sha256 = 'a'.repeat(64);
    workspace.source.episodes = [{ id: 'e1', title: '第一集' }, { id: 'e2', title: '第二集' }];
    const job = { id: 'unknown-foundation', kind: 'workflow', entityId: 'foundation.run', provider: 'local', status: 'submission_unknown', submissionUnknown: true, snapshot: { runnerVersion: 'desktop-runner/v1', stage: 'foundation.run', sourceVersion: 9, selection: { episodeIds: ['e1'] } } };
    workspace.jobs = [{ ...job, entityId: 'storyboard.run' }];
    assert.equal(foundationStartContinuation(workspace, ['e1']), null);
    workspace.jobs = [{ ...job, snapshot: { ...job.snapshot, sourceVersion: 8 } }];
    assert.equal(foundationStartContinuation(workspace, ['e1']), null);
    workspace.jobs = [{ ...job, snapshot: { ...job.snapshot, input: { source: { version: 9, sha256: 'b'.repeat(64) }, selection: { episodeIds: ['e1'] } } } }];
    assert.equal(foundationStartContinuation(workspace, ['e1']), null);
    workspace.jobs = [{ ...job, id: 'completed-all', status: 'completed', submissionUnknown: false, snapshot: { ...job.snapshot, selection: { episodeIds: ['e1', 'e2'] } } }, job];
    assert.equal(foundationStartContinuation(workspace, ['e1', 'e2'])?.id, 'unknown-foundation');
    assert.equal(foundationStartContinuation(workspace, ['e2']), null);
    let mutations = 0;
    const html = renderToStaticMarkup(createElement(Stage1Panel, { workspace, writable: true, mutate: async () => { mutations++; return workspace; } }));
    const held = (html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g) || []).find(button => button.includes('保留原制作基础任务'));
    assert.ok(held); assert.match(held, /^<button[^>]*disabled=""/);
    assert.match(html, /原范围有未知提交/);
    assert.equal(mutations, 0);
  } finally { await server.close(); }
});
