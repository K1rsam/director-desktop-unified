import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { actionBlockReason } from './executionPolicy.ts';

function actionButtons(html: string): { action: string; disabled: boolean; title: string }[] {
  return [...html.matchAll(/<button\b([^>]*)>/g)].flatMap(([, attributes]) => {
    const action = /data-execution-action="([^"]+)"/.exec(attributes)?.[1];
    return action ? [{ action, disabled: /\bdisabled=""/.test(attributes), title: /\btitle="([^"]+)"/.exec(attributes)?.[1] || '' }] : [];
  });
}

// This test renders the actual shared components with a synthetic desktop IPC.
// It does not constitute browser-layout, native-dialog, or real Tauri verification.
test('rendered four-stage controls and real frontend API enforce the offline boundary', async () => {
  const requests: string[] = [];
  Object.assign(globalThis, {
    isTauri: true,
    location: { search: '' },
    window: { __TAURI_INTERNALS__: { invoke: async (_command: string, args: { request: { action: string } }) => {
      requests.push(args.request.action);
      return args.request.action === 'getExecutionPolicy'
        ? { mode: 'offline_prepare_only', liveExecutionAllowed: false, stage3CreativePackStatus: 'inactive', reasons: ['Synthetic offline core'] }
        : { saved: true };
    } } },
  });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const workspace = structuredClone(previewWorkspace);
    workspace.jobs = [{ id: 'saved-job', kind: 'image', provider: 'dreamina', status: 'download_failed', providerTaskId: 'synthetic-task' }, { id: 'unknown-job', kind: 'video', provider: 'bh', status: 'submission_unknown', submissionUnknown: true }];
    const props = { workspace, writable: true, mutate: async (): Promise<unknown> => workspace, inspect: () => {}, onNotice: () => {}, onError: () => {}, onTasks: () => {} };
    const stages = [
      ['/src/Stage1Panel.tsx', 'Stage1Panel', 'stage-one-workbench'],
      ['/src/AssetPanel.tsx', 'AssetPanel', 'asset-workspace'],
      ['/src/Stage3Panel.tsx', 'Stage3Panel', 'stage-three-workbench'],
      ['/src/UnitPanel.tsx', 'UnitPanel', 'units-page'],
      ['/src/Panels.tsx', 'SettingsPanel', 'settings-page'],
      ['/src/Panels.tsx', 'TasksPanel', 'tasks-page'],
    ];
    const blocked = new Set<string>();
    for (const [path, name, className] of stages) {
      const module = await server.ssrLoadModule(path);
      const html = renderToStaticMarkup(createElement(module[name], name === 'SettingsPanel' ? { appSettings: { schemaVersion: 'app-settings/v1', revision: 0, settings: workspace.settings, providers: workspace.providers, rules: workspace.rules }, writable: true, mutate: async (): Promise<null> => null } : { ...props, mode: 'videos' }));
      assert.ok(html.includes(className), `${name} mature layout retained`);
      for (const button of actionButtons(html)) {
        if (actionBlockReason(button.action)) {
          assert.equal(button.disabled, true, `${name}: ${button.action}`);
          assert.ok(button.title.includes('离线准备模式'), button.action);
          blocked.add(button.action);
        }
      }
      if (name === 'AssetPanel') {
        assert.ok(html.includes('asset-library') && html.includes('asset-editor'), 'Stage2 library/editor split retained');
        const add = [...html.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)].find(([, , body]) => body.replace(/<[^>]+>/g, '') === '新增资产');
        assert.ok(add); assert.doesNotMatch(add[1], /disabled=""/);
      }
      if (name === 'SettingsPanel') {
        const credentials = [...html.matchAll(/<input\b([^>]*type="password"[^>]*)>/g)];
        assert.equal(credentials.length, 2);
        for (const [, attributes] of credentials) assert.match(attributes, /disabled=""/);
        assert.ok(html.includes('当前环境未开放手动只读连接检查'));
        assert.ok(!html.includes('即梦 Canvas 账户与连接'), 'unused Canvas account is not mounted');
      }
    }
    for (const action of ['foundation.run', 'images.design', 'images.produce', 'images.run', 'storyboard.run', 'units.regenerate', 'videos.prepare', 'videos.submit', 'providers.refresh', 'readiness.check', 'textCli.login', 'dreamina.login', 'jobs.query', 'jobs.download', 'jobs.reconcile']) assert.ok(blocked.has(action), `${action} is represented and blocked`);
    const { TextExecutionSettings } = await server.ssrLoadModule('/src/TextExecutionSettings.tsx');
    const apiSettings = renderToStaticMarkup(createElement(TextExecutionSettings, { ...props, settings: { textProvider: 'api' }, update: () => {}, saveSettings: async () => true }));
    assert.doesNotMatch(apiSettings, /type="password"/);
    assert.ok(apiSettings.includes('请选择执行方式'), 'legacy text API configuration must not be silently changed');
    const sdkSettings = renderToStaticMarkup(createElement(TextExecutionSettings, { ...props, settings: { textProvider: 'codex_sdk', textModel: 'gpt-6.1-sol', reasoning: 'medium' }, update: () => {}, saveSettings: async () => true }));
    assert.ok(sdkSettings.includes('value="codex_sdk" selected=""') && sdkSettings.includes('data-sdk-execution-state="closed"'));
    assert.ok(sdkSettings.includes('value="codex"') && sdkSettings.includes('value="codex_sdk"'));
    assert.doesNotMatch(sdkSettings, /type="password"/);

    const { Button } = await server.ssrLoadModule('/src/components.tsx');
    assert.match(renderToStaticMarkup(createElement(Button, { executionAction: 'future.execution', disabled: false }, 'Unknown')), /disabled=""/);
    assert.doesNotMatch(renderToStaticMarkup(createElement(Button, { executionAction: 'units.save' }, 'Local save')), /disabled=""/);
    const { ExecutionPolicyBanner } = await server.ssrLoadModule('/src/ExecutionPolicyBanner.tsx');
    const banner = renderToStaticMarkup(createElement(ExecutionPolicyBanner));
    assert.ok(banner.includes('offline_prepare_only') && banner.includes('Stage 3 创作包未启用'));
    const api = await server.ssrLoadModule('/src/api.ts');
    for (const action of [...blocked, 'workflows.resume', 'foundation.choices.apply', 'storyboard.decision.resolve', 'future.execution']) {
      await assert.rejects(api.request({ action }), /离线准备模式/, action);
    }
    await assert.rejects(api.request({ action: 'settings.save', settings: { textApiKey: 'synthetic-value' } }), /凭据/);
    assert.equal(requests.length, 0, 'direct calls and component rendering make no external IPC calls');
    await api.request({ action: 'units.save', unit: { id: 'synthetic-unit', prompt: '原话😀' } });
    const policy = await api.request({ action: 'getExecutionPolicy' });
    assert.equal(policy.liveExecutionAllowed, false);
    assert.deepEqual(requests, ['units.save', 'getExecutionPolicy']);
    assert.equal(api.mediaUrl({ kind: 'image', path: 'https://example.invalid/private.png' }), '');
    assert.equal(api.mediaUrl({ kind: 'image', path: '//example.invalid/private.png' }), '');
    assert.equal(api.mediaUrl({ kind: 'image', path: 'data:image/png;base64,AA==' }), 'data:image/png;base64,AA==');
    assert.equal(api.mediaUrl({ kind: 'image', previewUrl: 'https://example.invalid/private.png', path: 'data:image/png;base64,AA==' }), 'data:image/png;base64,AA==');
  } finally { await server.close(); }
});

test('browser preview stays read-only at the real API boundary', async () => {
  Object.assign(globalThis, { isTauri: false, location: { search: '?preview=1' } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const api = await server.ssrLoadModule('/src/api.ts');
    assert.equal(api.desktop, false); assert.equal(api.preview, true);
    assert.equal((await api.request({ action: 'projects.list' })).length, 1);
    assert.equal((await api.request({ action: 'workspace.get' })).project.id, 'preview-project');
    assert.equal((await api.request({ action: 'getExecutionPolicy' })).liveExecutionAllowed, false);
    for (const action of ['projects.create', 'source.import', 'units.save', 'settings.save', 'images.run', 'videos.prepare', 'files.choose']) {
      await assert.rejects(api.request({ action }), /浏览器预览使用示例资料/, action);
    }
  } finally { await server.close(); }
});
