import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('CLI summaries use the actual standard stage capability and keep blocked modes closed', async () => {
  Object.assign(globalThis, { location: { search: '' }, window: {} });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  try {
    const policy = await server.ssrLoadModule('/src/executionPolicy.ts');
    const { TextExecutionSummary } = await server.ssrLoadModule('/src/TextExecutionSettings.tsx');
    const standard = { mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: [], allowedActions: ['foundation.run', 'images.design'] };
    const render = (settings = {}, stage?: 'foundation' | 'images' | 'storyboard') => renderToStaticMarkup(React.createElement(TextExecutionSummary, { settings, stage, onConfigure() {} }));
    policy.installExecutionPolicy(standard);
    for (const stage of [undefined, 'foundation', 'images'] as const) {
      assert.match(render({}, stage), /标准版 CLI 已开放/);
      assert.doesNotMatch(render({}, stage), /当前关闭/);
    }
    assert.match(render({}, 'storyboard'), /当前关闭/);
    assert.match(render({ textProvider: 'codex_sdk' }), /当前关闭/);
    policy.installExecutionPolicy({ ...standard, textBackends: [{ id: 'codex_cli', status: 'preserved_not_enabled' }, { id: 'codex_sdk', status: 'explicit_selection_when_ready' }] });
    assert.match(render(), /当前关闭/, 'explicit closed CLI capability is preserved');
    assert.match(render({ textProvider: 'codex_sdk' }), /标准版 SDK · 执行前核验/);
    assert.doesNotMatch(render({ textProvider: 'codex_sdk' }), /已就绪|已核验/, 'selecting SDK never proves actual readiness');
    assert.match(render({ textProvider: 'codex_sdk' }, 'storyboard'), /当前关闭/);
    policy.installExecutionPolicy({ ...standard, liveExecutionAllowed: false });
    assert.match(render(), /当前关闭/);
    policy.installExecutionPolicy({ ...standard, mode: 'native_cli_acceptance' });
    assert.match(render(), /受控 CLI 验收/);
    policy.installExecutionPolicy(policy.OFFLINE_EXECUTION_POLICY);
    assert.match(render(), /当前关闭/);
  } finally { await server.close(); }
});
