import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { createServer } from 'vite';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import type { Host } from './test-support/hookRenderer.ts';

const button = (hosts: Host[], text: string) => {
  const result = hosts.find(host => host.type === 'button' && content(host) === text);
  assert.ok(result, `missing button ${text}`); return result;
};
test('source analysis real handlers preserve text, selected scope, durable pending and offline boundaries', async t => {
  Object.assign(globalThis, { isTauri: false, location: { search: '' }, window: {} });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  try {
    const { Stage1Panel } = await server.ssrLoadModule('/src/Stage1Panel.tsx');
    const { SourceStructure, sourceAnalysisState } = await server.ssrLoadModule('/src/SourceStructure.tsx');
    const policy = await server.ssrLoadModule('/src/executionPolicy.ts');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const live = () => policy.installExecutionPolicy({ mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: [], allowedActions: ['foundation.run', 'source.analyze', 'source.resolve', 'source.reparse'] });
    const fixture = (code = 'BOUNDARY_UNCONFIRMED', episodes = true) => {
      const workspace = structuredClone(previewWorkspace);
      workspace.jobs = []; workspace.assets = []; workspace.units = []; workspace.foundation = null; workspace.rules = [];
      workspace.source = { version: 4, sha256: 'a'.repeat(64), parserVersion: 'source-parser/v11', text: '人物简介\n甲推开门。\n', episodes: episodes ? [{ id: 'e1', number: 1, title: '第一集' }, { id: 'e2', number: 2, title: '第二集' }] : [], scenes: [], rows: [{ id: 'r1', number: 1, text: '人物简介\n', kind: 'unresolved', range: { start: 0, end: 5 } }, { id: 'r2', number: 2, text: '甲推开门。\n', kind: 'unresolved', range: { start: 5, end: 11 } }], diagnostics: [{ code, blocking: true, rowIds: ['r1', 'r2'], message: code === 'DIALOGUE_MULTI_SPEAKER_UNCONFIRMED' ? '这行有多个说者待确认' : '局部内容用途待确认' }] };
      return workspace;
    };
    const task = (workspace: any, status: string) => ({ id: 'analysis-1', kind: 'workflow', entityId: 'source.analyze', provider: 'local', status, snapshot: { runnerVersion: 'source-analysis-runner/v1', sourceVersion: workspace.source.version, input: { source: workspace.source } } });

    await t.test('unrecognized structure can analyze even before any episode exists, with one submit', async () => {
      live();
      const workspace = fixture('BOUNDARY_UNCONFIRMED', false), before = structuredClone(workspace.source), calls: any[] = [], errors: string[] = [], view = hookRenderer();
      let finish: (value: unknown) => void = () => {};
      const element = createElement(Stage1Panel, { workspace, writable: true, onError: (message: string) => errors.push(message), mutate: (action: string, payload: any) => { calls.push({ action, payload }); return new Promise(resolve => { finish = resolve; }); } });
      try {
        const start = button(view.render(element), '识别格式并整理'); assert.equal(start.props.disabled, false);
        start.props.onClick(); start.props.onClick();
        assert.deepEqual(calls, [{ action: 'source.analyze', payload: { expectedSourceVersion: 4, episodeIds: [], continueFoundation: true } }]);
        let hosts = view.render(element);
        assert.equal(button(hosts, '正在识别原文格式').props.disabled, true);
        assert.equal(button(hosts, '格式识别处理中').props.disabled, true);
        workspace.jobs = [task(workspace, 'queued')]; finish(workspace); await settle();
        hosts = view.render(element);
        assert.equal(button(hosts, '正在识别原文格式').props.disabled, true);
        assert.ok(hosts.some(host => host.props.role === 'status' && content(host).includes('任务记录')));
        assert.deepEqual(workspace.source, before); assert.deepEqual(errors, []);
      } finally { view.unmount(); }
    });

    await t.test('current-episode action sends the selected episode, not all episodes', async () => {
      live(); const workspace = fixture('SCENE_BOUNDARY_UNCONFIRMED'), calls: any[] = [], view = hookRenderer();
      const element = createElement(Stage1Panel, { workspace, writable: true, mutate: async (action: string, payload: any) => { calls.push({ action, payload }); return workspace; } });
      try {
        const navigation = view.render(element).find(host => host.type === 'button' && String(host.props.className).includes('production-nav-item') && content(host).startsWith('第二集'));
        assert.ok(navigation); navigation.props.onClick();
        button(view.render(element), '识别格式并整理当前集').props.onClick(); await settle();
        assert.deepEqual(calls, [{ action: 'source.analyze', payload: { expectedSourceVersion: 4, episodeIds: ['e2'], continueFoundation: true } }]);
      } finally { view.unmount(); }
    });

    await t.test('standalone AI analysis is explicit and does not start foundation or overwrite text', async () => {
      live(); const workspace = fixture(), calls: any[] = [], pending: boolean[] = [], view = hookRenderer();
      let finish: (value: unknown) => void = () => {};
      const element = createElement(SourceStructure, { source: workspace.source, writable: true, jobs: [], onReadScope: () => {}, onAnalysisPendingChange: (value: boolean) => pending.push(value), mutate: (action: string, payload: any) => { calls.push({ action, payload }); return new Promise(resolve => { finish = resolve; }); } });
      try {
        let hosts = view.render(element); assert.equal(calls.length, 0);
        assert.ok(hosts.some(host => content(host).includes('文字渠道额度')));
        const analyze = button(hosts, 'AI识别格式'); analyze.props.onClick(); analyze.props.onClick();
        assert.deepEqual(calls, [{ action: 'source.analyze', payload: { expectedSourceVersion: 4, continueFoundation: false } }]);
        hosts = view.render(element);
        assert.equal(button(hosts, '重新识别').props.disabled, true);
        assert.equal(button(hosts, '确认这段用途').props.disabled, true);
        finish(workspace); await settle(); assert.deepEqual(pending, [true, false]);
      } finally { view.unmount(); }
    });

    await t.test('unsupported speaker ambiguities remain manual and known structures use original foundation flow', async () => {
      live(); const workspace = fixture('DIALOGUE_MULTI_SPEAKER_UNCONFIRMED'), calls: any[] = [], view = hookRenderer();
      const element = createElement(Stage1Panel, { workspace, writable: true, mutate: async (action: string, payload: any) => { calls.push({ action, payload }); return workspace; } });
      try {
        let hosts = view.render(element);
        assert.equal(button(hosts, '整理全剧制作基础').props.disabled, true);
        assert.ok(hosts.some(host => content(host).includes('这行有多个说者待确认')));
        assert.equal(calls.length, 0);
        workspace.source.diagnostics = [];
        hosts = view.render(element); const start = button(hosts, '整理全剧制作基础'); assert.equal(start.props.disabled, false);
        start.props.onClick(); await settle();
        assert.deepEqual(calls, [{ action: 'foundation.run', payload: { options: { selection: { episodeIds: ['e1', 'e2'] } } } }]);
      } finally { view.unmount(); }
    });

    await t.test('durable unknown is held; partial analysis stays reviewable, with old-version jobs excluded', () => {
      live(); const workspace = fixture(), view = hookRenderer();
      const props = { workspace, writable: true, mutate: async () => { throw new Error('must not submit on render'); } };
      try {
        workspace.jobs = [task(workspace, 'submission_unknown')];
        let hosts = view.render(createElement(Stage1Panel, props));
        assert.equal(button(hosts, '核对原格式识别任务').props.disabled, true);
        assert.ok(hosts.some(host => host.props.role === 'status' && content(host).includes('不会重复提交')));
        workspace.jobs = [task(workspace, 'partially_blocked')];
        hosts = view.render(createElement(Stage1Panel, props));
        assert.ok(hosts.some(host => host.props.role === 'status' && content(host).includes('仍有部分结构待确认')));
        assert.equal(button(hosts, '识别格式并整理').props.disabled, false);
        const old = task(workspace, 'running'); old.snapshot.sourceVersion = 3;
        assert.equal(sourceAnalysisState(workspace.source, [old]).pending, false);
        const published = { ...workspace.source, version: 5 };
        const partial = { ...task(workspace, 'partially_blocked'), result: { source: published } };
        assert.match(sourceAnalysisState(published, [partial]).message, /仍有部分结构待确认/);
      } finally { view.unmount(); }
    });

    await t.test('AI analysis is generation and cannot run under offline policy', () => {
      policy.installExecutionPolicy(policy.OFFLINE_EXECUTION_POLICY);
      assert.equal(policy.classifyAction('source.analyze'), 'generation');
      assert.match(policy.actionBlockReason('source.analyze'), /离线准备模式/);
      assert.throws(() => policy.assertOfflineAction({ action: 'source.analyze' }));
      const view = hookRenderer(), workspace = fixture();
      try {
        const hosts = view.render(createElement(Stage1Panel, { workspace, writable: true, mutate: async () => { throw new Error('offline'); } }));
        for (const text of ['识别格式并整理', 'AI识别格式']) assert.equal(button(hosts, text).props.disabled, true);
        assert.equal(button(hosts, '重新识别').props.disabled, false);
      } finally { view.unmount(); }
    });
  } finally { await server.close(); }
});
