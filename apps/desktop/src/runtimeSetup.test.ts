import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';

test('runtime setup transports only paths and rejects cross-provider or credential settings', async () => {
  const calls: any[] = [];
  let response: unknown;
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, window: { __TAURI_INTERNALS__: { invoke: async (_command: string, args: any) => { calls.push(args.request); return response; } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  try {
    const policy = await server.ssrLoadModule('/src/executionPolicy.ts');
    const { prepareRuntime, runtimeSettingsPatch, settingsRequestScope } = await server.ssrLoadModule('/src/runtimeSetup.ts');
    policy.installExecutionPolicy(policy.validateExecutionPolicy({ version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: [], allowedActions: ['runtime.prepare'] }));
    response = { schemaVersion: 'runtime-prepare/v1', provider: 'codex_sdk', settings: { codexPath: 'D:\\Programs\\codex.exe', textSdkNodePath: 'C:\\Program Files\\nodejs\\node.exe' } };
    assert.deepEqual(await prepareRuntime('project', 'codex_sdk', { textProvider: 'codex_sdk', textApiKey: 'not-for-setup', textModel: 'chosen', codexPath: 'old', textSdkNodePath: '' }), (response as any).settings);
    assert.deepEqual(calls, [{ action: 'runtime.prepare', projectId: 'project', provider: 'codex_sdk', settings: { codexPath: 'old', textSdkNodePath: '' } }]);
    assert.deepEqual(await prepareRuntime({ scope: 'global' }, 'codex_sdk', {}), (response as any).settings);
    assert.deepEqual(calls.at(-1), { action: 'runtime.prepare', scope: 'global', provider: 'codex_sdk', settings: { codexPath: '', textSdkNodePath: '' } });
    assert.deepEqual(settingsRequestScope({ projectId: 'project' }), { projectId: 'project' });
    for (const invalidScope of ['', ' ', undefined, null, {}, { scope: 'project' }, { projectId: '' }, { scope: 'global', projectId: 'project' }]) assert.throws(() => settingsRequestScope(invalidScope));
    for (const invalid of [null, {}, { ...(response as any), provider: 'dreamina' }, { ...(response as any), settings: { codexPath: 'relative', textSdkNodePath: 'C:\\node.exe' } }, { ...(response as any), settings: { ...(response as any).settings, textModel: 'changed' } }, { ...(response as any), settings: { codexPath: 'C:\\codex.exe' } }]) assert.throws(() => runtimeSettingsPatch(invalid, 'codex_sdk'));
    policy.installExecutionPolicy(policy.OFFLINE_EXECUTION_POLICY);
    await assert.rejects(prepareRuntime('project', 'codex_sdk', {}));
    assert.equal(calls.length, 2, 'offline runtime setup cannot download or contact a provider');
  } finally { await server.close(); }
});

test('component inventory validates discovered paths and never transports account credentials', async () => {
  const calls: any[] = [];
  const ready = { schemaVersion: 'runtime-status/v1', provider: 'codex_sdk', installed: true, installSupported: true, checkedAt: '2026-10-09T10:00:00Z', settings: { codexPath: '/native/codex', textSdkNodePath: '/native/node' }, components: [{ id: 'node', name: 'Node', state: 'installed', path: '/native/node', version: null }, { id: 'codex', name: 'Codex', state: 'installed', path: '/native/codex', version: null }] };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, window: { __TAURI_INTERNALS__: { invoke: async (_command: string, args: any) => { calls.push(args.request); return ready; } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  try {
    const policy = await server.ssrLoadModule('/src/executionPolicy.ts');
    const { readRuntimeStatus, runtimeStatus } = await server.ssrLoadModule('/src/runtimeSetup.ts');
    policy.installExecutionPolicy(policy.validateExecutionPolicy({ version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: [], allowedActions: ['runtime.status'] }));
    assert.deepEqual(await readRuntimeStatus({ scope: 'global' }, 'codex_sdk', { codexPath: '/draft/codex', textApiKey: 'PRIVATE', textModel: 'chosen' }), ready);
    assert.deepEqual(calls, [{ action: 'runtime.status', scope: 'global', provider: 'codex_sdk', settings: { codexPath: '/draft/codex', textSdkNodePath: '' } }]);
    const missing = { ...ready, installed: false, settings: { codexPath: '/native/codex' }, components: [{ id: 'node', name: 'Node', state: 'missing', version: null }, ready.components[1]] };
    assert.deepEqual(runtimeStatus(missing, 'codex_sdk'), missing);
    for (const invalid of [null, {}, { ...ready, provider: 'dreamina' }, { ...ready, installed: false }, { ...missing, installed: true }, { ...ready, settings: { ...ready.settings, textApiKey: 'secret' } }, { ...ready, settings: { ...ready.settings, codexPath: '/different' } }, { ...ready, components: [ready.components[0], ready.components[0]] }, { ...ready, checkedAt: 'unknown' }, { ...missing, components: [{ ...missing.components[0], state: 'invalid' }, ready.components[1]] }]) assert.throws(() => runtimeStatus(invalid, 'codex_sdk'));
  } finally { await server.close(); }
});
