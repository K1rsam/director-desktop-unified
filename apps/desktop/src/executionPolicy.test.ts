import test from 'node:test';
import assert from 'node:assert/strict';
import { actionBlockReason, assertOfflineAction, classifyAction, CREDENTIAL_SETTING_KEYS, dispatchOfflineRequest, OFFLINE_EXECUTION_POLICY, validateExecutionPolicy } from './executionPolicy.ts';
import { rustStandardActionCatalogue } from './test-support/rustExecutionPolicy.ts';

test('the complete native Rust action catalogue installs without disabling SDK or Dreamina authorization', async () => {
  const { installExecutionPolicy, hasSdkExecutionCapability } = await import('./executionPolicy.ts');
  const allowedActions = rustStandardActionCatalogue();
  const policy = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true,
    stage3CreativePackStatus: 'verified_active', reasons: ['complete source-bound native catalogue'], allowedActions,
    readiness: { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'dreamina', 'dreamina_canvas', 'codex_sdk'], generationAllowed: false },
    textBackends: [{ id: 'codex_cli', status: 'enabled' }, { id: 'codex_sdk', status: 'explicit_selection_when_ready' }] };
  try {
    assert.ok(allowedActions.includes('storyboard.refinement.continuity.preview'));
    const checked = await dispatchOfflineRequest({ action: 'getExecutionPolicy' }, async () => policy);
    assert.equal(checked.mode, 'standard_native');
    assert.equal(hasSdkExecutionCapability(), true);
    for (const action of ['dreamina.login', 'dreamina.status', 'dreamina.open_authorization', 'textCli.login', 'readiness.check', 'storyboard.refinement.continuity.preview']) assert.equal(actionBlockReason(action), null, action);
    assert.equal(classifyAction('storyboard.refinement.continuity.preview'), 'local_read');
    assert.throws(() => validateExecutionPolicy({ ...policy, allowedActions: [...allowedActions, 'future.unknown.action'] }), /标准版功能策略无效/);
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
  assert.equal(actionBlockReason('storyboard.refinement.continuity.preview'), null);
  assert.ok(actionBlockReason('storyboard.refinement.adopt'));
  assert.ok(actionBlockReason('dreamina.login'));
});

test('standard native capabilities enable existing flows and keep unported Stage3 closed', async () => {
  const { installExecutionPolicy } = await import('./executionPolicy.ts');
  const policy = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true,
    stage3CreativePackStatus: 'inactive', reasons: ['synthetic standard capabilities'],
    allowedActions: ['getExecutionPolicy', 'foundation.run', 'images.design', 'settings.save', 'providers.refresh', 'textCli.login', 'workflows.resume'] };
  try {
    installExecutionPolicy(validateExecutionPolicy(policy));
    const sent: string[] = [];
    for (const action of ['foundation.run', 'images.design', 'textCli.login']) {
      await dispatchOfflineRequest({ action }, async input => { sent.push(input.action); return null; });
    }
    assert.deepEqual(sent, ['foundation.run', 'images.design', 'textCli.login']);
    assert.equal(actionBlockReason({ action: 'settings.save', settings: { textApiKey: 'synthetic' } }), null);
    for (const action of ['storyboard.run', 'units.regenerate']) {
      await assert.rejects(dispatchOfflineRequest({ action }, async () => assert.fail('unported Stage3 reached IPC')), /第三阶段暂不可生成/);
    }
    assert.throws(() => validateExecutionPolicy({ ...policy, allowedActions: ['storyboard.run'] }), /标准版功能策略无效/);
    assert.throws(() => validateExecutionPolicy({ ...policy, allowedActions: ['unknown.action'] }), /标准版功能策略无效/);
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
});
import type { PolicyRequest } from './executionPolicy.ts';

test('text backend selection retains exact core capability and cannot open SDK in isolated or CLI acceptance modes', async () => {
  const { hasSdkExecutionCapability } = await import('./executionPolicy.ts');
  const input = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: [], allowedActions: ['foundation.run'],
    textBackends: [{ id: 'codex_cli', status: 'enabled' }, { id: 'codex_sdk', status: 'explicit_selection_when_ready' }] };
  const checked = validateExecutionPolicy(input);
  assert.equal(hasSdkExecutionCapability(checked), true);
  assert.deepEqual(checked.textBackends, input.textBackends); assert.notEqual(checked.textBackends, input.textBackends);
  assert.equal(hasSdkExecutionCapability(validateExecutionPolicy({ ...input, textBackends: undefined })), false);
  assert.equal(hasSdkExecutionCapability(validateExecutionPolicy({ ...input, textBackends: [{ id: 'codex_sdk', status: 'offline_contract_only' }] })), false);
  for (const textBackends of [[{ id: 'codex_sdk', status: 'enabled' }], [{ id: 'codex_api', status: 'explicit_selection_when_ready' }], [input.textBackends[1], input.textBackends[1]], 'codex_sdk']) {
    assert.throws(() => validateExecutionPolicy({ ...input, textBackends }), /文本入口能力/);
  }
  assert.throws(() => validateExecutionPolicy({ ...OFFLINE_EXECUTION_POLICY, textBackends: input.textBackends }), /文本入口能力/);
  assert.throws(() => validateExecutionPolicy({ ...input, mode: 'native_cli_acceptance', textBackends: [input.textBackends[1]] }), /文本入口能力/);
});

test('director refinement requires the verified pack and cannot unlock video generation', async () => {
  const { installExecutionPolicy } = await import('./executionPolicy.ts');
  const actions = ['storyboard.refine', 'storyboard.refinement.adopt'];
  const policy = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true,
    stage3CreativePackStatus: 'verified_active', reasons: ['synthetic verified adapter'], allowedActions: actions };
  try {
    assert.throws(() => validateExecutionPolicy({ ...policy, stage3CreativePackStatus: 'inactive' }));
    installExecutionPolicy(validateExecutionPolicy(policy));
    const sent: string[] = [];
    for (const action of actions) await dispatchOfflineRequest({ action }, async input => { sent.push(input.action); });
    assert.deepEqual(sent, actions);
    await assert.rejects(dispatchOfflineRequest({ action: 'videos.submit' }, async () => { throw new Error('video reached transport'); }), /尚未开放/);
    installExecutionPolicy(OFFLINE_EXECUTION_POLICY);
    for (const action of actions) await assert.rejects(dispatchOfflineRequest({ action }, async () => { throw new Error('offline reached transport'); }), /离线准备模式/);
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
});
import { createCatalogDiscovery } from './contracts.ts';
import type { Workspace } from './types.ts';

const localReads = ['getExecutionPolicy', 'projects.list', 'files.choose', 'legacy.inspect', 'workspace.get', 'units.get', 'units.versions.list', 'foundation.choices.list', 'assets.get', 'assets.usage', 'jobs.detail'];
const localWrites = ['projects.create', 'projects.archive', 'projects.restore', 'projects.delete', 'legacy.import', 'project.import', 'project.export', 'units.version.restore', 'units.save', 'production.direction.save', 'foundation.choice.confirm', 'assets.save', 'assets.delete', 'assets.restore', 'jobs.close_unknown', 'source.import', 'source.resolve', 'source.reparse', 'media.import', 'media.adopt', 'media.adopt_batch', 'images.ensure_current', 'videos.adopt', 'settings.save', 'backup.create', 'backup.restore'];
const generations = ['foundation.run', 'foundation.choices.apply', 'images.design', 'images.produce', 'images.run', 'storyboard.run', 'storyboard.decision.resolve', 'units.regenerate', 'workflows.resume', 'videos.submit'];
const providers = ['providers.refresh', 'videos.preflight', 'videos.prepare', 'jobs.query', 'jobs.download', 'jobs.reconcile'];
const accounts = ['textCli.status', 'textCli.login', 'textCli.loginStatus', 'textCli.cancelLogin', 'textCli.openLogin', 'dreamina.status', 'dreamina.login', 'dreamina.logout', 'dreamina.open_authorization'];

test('offline build preserves explicit local reads and editing actions', () => {
  for (const action of localReads) { assert.equal(classifyAction(action), 'local_read', action); assert.equal(actionBlockReason(action), null, action); }
  for (const action of localWrites) { assert.equal(classifyAction(action), 'local_write', action); assert.equal(actionBlockReason(action), null, action); }
});

test('model, image, video and resume routes are generation actions', () => {
  for (const action of generations) { assert.equal(classifyAction(action), 'generation', action); assert.match(actionBlockReason(action)!, /真实文本、图片与视频生成/, action); }
});

test('prepare is not mislabeled local while it depends on channel and account checks', () => {
  for (const action of providers) { assert.equal(classifyAction(action), 'provider', action); assert.match(actionBlockReason(action)!, /渠道访问/, action); }
});

test('CLI probes, login, status and logout remain inactive', () => {
  for (const action of accounts) { assert.equal(classifyAction(action), 'credentials', action); assert.match(actionBlockReason(action)!, /CLI、账户登录与凭据/, action); }
});

test('credential payloads are denied without blocking ordinary local settings', () => {
  assert.equal(actionBlockReason({ action: 'settings.save', settings: { textProvider: 'codex', textModel: 'test-model', visualStyle: '合成测试', textApiKey: '', bhToken: null } }), null);
  for (const key of CREDENTIAL_SETTING_KEYS) {
    for (const value of ['synthetic-value', ' ', {}, 1, true]) {
      const request = { action: 'settings.save', settings: { [key]: value } };
      assert.equal(classifyAction(request), 'credentials', key);
      assert.throws(() => assertOfflineAction(request), /凭据/, key);
    }
  }
});

test('unknown operations and near-match names fail closed', () => {
  for (const action of ['', 'projects.list ', 'PROJECTS.LIST', 'sdk.execute', 'images.generate', 'workflows.retry', 'settings.credentials.save']) {
    assert.equal(classifyAction(action), 'unknown', action);
    assert.throws(() => assertOfflineAction({ action }), /尚未纳入/, action);
  }
});

test('blocked operations make zero IPC calls, including repeated attempts', async () => {
  let calls = 0;
  const transport = async () => { calls++; return {}; };
  for (const action of [...generations, ...providers, ...accounts, 'future.external.action']) {
    for (let attempt = 0; attempt < 2; attempt++) await assert.rejects(dispatchOfflineRequest({ action }, transport), /离线准备模式/, action);
  }
  await assert.rejects(dispatchOfflineRequest({ action: 'settings.save', settings: { textApiKey: 'synthetic-value' } }, transport), /凭据/);
  assert.equal(calls, 0);
});

test('guard does not alter local data, identities, raw dialogue or history', async () => {
  const input = { action: 'units.save', unit: { id: 'unit-1', expectedVersion: 3, prompt: '角色 OS：原句😀', sourceRefs: ['line-1'], raw: { original: '未改原件' } } };
  const before = structuredClone(input);
  const output = { saved: true };
  let received: PolicyRequest | undefined;
  assert.equal(await dispatchOfflineRequest(input, async request => { received = request; return output; }), output);
  assert.equal(received, input);
  assert.deepEqual(input, before);
});

test('autoSubmit setting cannot turn preparation or resumed tasks into live dispatch', async () => {
  const calls: string[] = [];
  const transport = async (request: PolicyRequest) => { calls.push(request.action); return {}; };
  await dispatchOfflineRequest({ action: 'settings.save', settings: { autoSubmit: true } }, transport);
  for (const action of ['videos.prepare', 'videos.submit', 'workflows.resume', 'images.produce']) {
    await assert.rejects(dispatchOfflineRequest({ action, autoSubmit: true }, transport), /离线准备模式/);
  }
  assert.deepEqual(calls, ['settings.save']);
});

test('policy response must independently prove offline mode and inactive creative pack', async () => {
  const policy = validateExecutionPolicy(OFFLINE_EXECUTION_POLICY);
  assert.deepEqual(policy, OFFLINE_EXECUTION_POLICY);
  assert.notEqual(policy.reasons, OFFLINE_EXECUTION_POLICY.reasons);
  const response = await dispatchOfflineRequest({ action: 'getExecutionPolicy' }, async () => OFFLINE_EXECUTION_POLICY);
  assert.deepEqual(response, OFFLINE_EXECUTION_POLICY);
  for (const value of [null, {}, { ...OFFLINE_EXECUTION_POLICY, liveExecutionAllowed: true }, { ...OFFLINE_EXECUTION_POLICY, mode: 'live' }, { ...OFFLINE_EXECUTION_POLICY, stage3CreativePackStatus: 'active' }, { ...OFFLINE_EXECUTION_POLICY, reasons: [1] }]) {
    assert.throws(() => validateExecutionPolicy(value), /策略/);
    await assert.rejects(dispatchOfflineRequest({ action: 'getExecutionPolicy' }, async () => value), /策略/);
  }
  assert.throws(() => assertOfflineAction({ action: 'storyboard.run' }), /离线准备模式/);
});

test('automatic catalog discovery does not read providers in offline mode', async () => {
  let calls = 0;
  const workspace: Workspace = { project: { id: 'offline-project', title: '合成项目' }, settings: { imageProvider: 'bh', videoProvider: 'dreamina' }, providers: [], assets: [], units: [], media: [], jobs: [], rules: [] };
  const discover = createCatalogDiscovery({ enabled: () => !actionBlockReason('providers.refresh'), current: () => workspace, busy: () => false, revision: () => 0, refresh: async () => { calls++; }, read: async () => { calls++; return workspace; }, apply: () => { calls++; }, report: () => { calls++; } });
  await discover(); await discover();
  assert.equal(calls, 0);
});

test('operator native scope only opens approved CLI actions and never SDK or media', async () => {
  const { installExecutionPolicy, OFFLINE_EXECUTION_POLICY } = await import('./executionPolicy.ts');
  const scope = { version: 'execution-policy/v2', mode: 'native_cli_acceptance', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: ['synthetic bounded native scope'], allowedActions: ['getExecutionPolicy', 'foundation.run', 'images.design', 'textCli.status'], acceptance: { sourceSha256: 'a'.repeat(64), backend: 'codex', model: 'gpt-6.1-sol', reasoning: 'medium', storyboardEpisodeNumbers: [1, 2], stage3AdapterVerified: false, sdkLiveAllowed: false, realVideoAllowed: false, newWorkflowsPerPhase: 1 } };
  try {
    installExecutionPolicy(validateExecutionPolicy(scope));
    assert.equal(actionBlockReason('foundation.run'), null);
    assert.equal(actionBlockReason('images.design'), null);
    for (const action of ['storyboard.run', 'images.run', 'images.produce', 'videos.prepare', 'videos.submit', 'textCli.login']) assert.ok(actionBlockReason(action));
    assert.ok(actionBlockReason({ action: 'settings.save', settings: { textApiKey: 'synthetic' } }));
    for (const changed of [ { ...scope, allowedActions: [...scope.allowedActions, 'videos.submit'] }, { ...scope, allowedActions: [...scope.allowedActions, 'storyboard.run'] }, { ...scope, acceptance: { ...scope.acceptance, reasoning: 'max' } }, { ...scope, acceptance: { ...scope.acceptance, sdkLiveAllowed: true } } ]) assert.throws(() => validateExecutionPolicy(changed));
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
});

test('validated readiness capability opens only explicit provider checks with generation off', async () => {
  const { installExecutionPolicy } = await import('./executionPolicy.ts');
  const readiness = { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'dreamina', 'codex_sdk'], generationAllowed: false };
  const policy = { ...OFFLINE_EXECUTION_POLICY, readiness };
  const calls: PolicyRequest[] = [];
  try {
    assert.equal(classifyAction('readiness.check'), 'readiness');
    assert.ok(actionBlockReason('readiness.check'), 'initial state fails closed');
    await dispatchOfflineRequest({ action: 'getExecutionPolicy' }, async () => policy);
    for (const provider of readiness.providers) {
      const input = { action: 'readiness.check', provider, settings: { codexPath: '/synthetic/unsaved path' } };
      assert.equal(actionBlockReason(input), null);
      await dispatchOfflineRequest(input, async request => { calls.push(request); return {}; });
    }
    for (const provider of [undefined, '', 'codex', 'bh', 'dreamina ', 'codex_sdk.execute']) {
      await assert.rejects(dispatchOfflineRequest({ action: 'readiness.check', provider }, async request => { calls.push(request); return {}; }));
    }
    for (const action of [...generations, ...providers, ...accounts]) assert.ok(actionBlockReason(action));
    assert.ok(actionBlockReason({ action: 'settings.save', settings: { textApiKey: 'synthetic' } }));
    assert.equal(calls.length, 3);
    for (const invalid of [null, {}, { ...readiness, version: 'unknown' }, { ...readiness, enabled: 'true' }, { ...readiness, generationAllowed: true }, { ...readiness, providers: ['codex_cli', 'codex_cli'] }, { ...readiness, providers: ['codex_cli', 'bh'] }]) {
      await assert.rejects(dispatchOfflineRequest({ action: 'getExecutionPolicy' }, async () => ({ ...policy, readiness: invalid })), /策略/);
      assert.ok(actionBlockReason('readiness.check'), 'invalid backend capability resets to fail closed');
    }
    installExecutionPolicy(validateExecutionPolicy({ ...policy, readiness: { ...readiness, enabled: false } }));
    assert.ok(actionBlockReason('readiness.check'), 'isolated startup capability remains off');
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
});


test('full native backend policy accepts readiness action without enlarging account or generation gates', async () => {
  const { installExecutionPolicy } = await import('./executionPolicy.ts');
  const readiness = { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'dreamina', 'codex_sdk'], generationAllowed: false };
  const scope = { version: 'execution-policy/v2', mode: 'native_cli_acceptance', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: ['Synthetic full backend native policy'], allowedActions: [...localReads, ...localWrites, 'textSdk.status', 'images.previewPrompt', 'readiness.check', 'foundation.run', 'images.design', 'workflows.resume', 'textCli.status'], readiness, acceptance: { sourceSha256: 'a'.repeat(64), backend: 'codex', model: 'gpt-6.1-sol', reasoning: 'medium', storyboardEpisodeNumbers: [1, 2], stage3AdapterVerified: false, sdkLiveAllowed: false, realVideoAllowed: false, newWorkflowsPerPhase: 1 } };
  try {
    const policy = await dispatchOfflineRequest({ action: 'getExecutionPolicy' }, async () => scope);
    assert.ok(policy.allowedActions.includes('readiness.check'));
    for (const provider of readiness.providers) assert.equal(actionBlockReason({ action: 'readiness.check', provider }), null);
    for (const action of ['foundation.run', 'images.design', 'workflows.resume', 'textCli.status']) assert.equal(actionBlockReason(action), null, 'previous bounded native authorization is retained');
    for (const action of ['storyboard.run', 'images.run', 'images.produce', 'videos.prepare', 'videos.submit', 'textCli.login', 'dreamina.login', 'dreamina.logout', 'providers.refresh']) assert.ok(actionBlockReason(action), action);
    assert.ok(actionBlockReason({ action: 'settings.save', settings: { textApiKey: 'synthetic' } }));
    assert.ok(actionBlockReason({ action: 'readiness.check', provider: 'bh' }));
    installExecutionPolicy(validateExecutionPolicy({ ...scope, readiness: { ...readiness, enabled: false } }));
    assert.ok(actionBlockReason({ action: 'readiness.check', provider: 'codex_cli' }), 'allowedActions cannot override the disabled readiness capability');
    for (const action of ['dreamina.status', 'textCli.login', 'videos.submit']) assert.throws(() => validateExecutionPolicy({ ...scope, allowedActions: [...scope.allowedActions, action] }), 'native policy cannot whitelist unapproved actions');
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
});

test('Canvas readiness requires its explicit backend capability and never opens a generation or login route', async () => {
  const { installExecutionPolicy } = await import('./executionPolicy.ts');
  const capability = { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'dreamina', 'codex_sdk'], generationAllowed: false };
  const calls: PolicyRequest[] = [];
  const request: PolicyRequest = { action: 'readiness.check', provider: 'dreamina_canvas', settings: { dreaminaCanvasPath: '/synthetic/canvas' } };
  try {
    installExecutionPolicy(validateExecutionPolicy({ ...OFFLINE_EXECUTION_POLICY, readiness: capability }));
    await assert.rejects(dispatchOfflineRequest(request, async input => { calls.push(input); }));
    installExecutionPolicy(validateExecutionPolicy({ ...OFFLINE_EXECUTION_POLICY, readiness: { ...capability, providers: [...capability.providers, 'dreamina_canvas'] } }));
    await dispatchOfflineRequest(request, async input => { calls.push(input); });
    assert.deepEqual(calls, [request]);
    for (const action of ['dreamina_canvas.login', 'dreamina_canvas.generate', 'dreamina_canvas.submit', 'dreamina.login', 'providers.refresh', 'images.run', 'videos.submit']) {
      await assert.rejects(dispatchOfflineRequest({ action }, async input => { calls.push(input); }));
    }
    assert.equal(calls.length, 1);
    installExecutionPolicy(validateExecutionPolicy({ ...OFFLINE_EXECUTION_POLICY, readiness: { ...capability, enabled: false, providers: ['dreamina_canvas'] } }));
    await assert.rejects(dispatchOfflineRequest(request, async input => { calls.push(input); }));
    assert.equal(calls.length, 1, 'disabled startup policy remains fail closed');
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
});


test('native Canvas OAuth is independently gated and cannot unlock generation', async () => {
  const { installExecutionPolicy } = await import('./executionPolicy.ts');
  try {
    for (const enabled of [false, true]) {
      installExecutionPolicy(validateExecutionPolicy({ ...OFFLINE_EXECUTION_POLICY, canvasOAuth: { version: 'canvas-oauth/v1', enabled } }));
      for (const action of ['canvas.login', 'canvas.loginStatus', 'canvas.openAuthorization', 'canvas.cancelLogin', 'canvas.logout']) assert.equal(!actionBlockReason(action), enabled);
      for (const action of ['canvas.loginShell', 'canvas.generate', 'dreamina.login', 'images.run', 'videos.submit']) assert.ok(actionBlockReason(action));
    }
    assert.throws(() => validateExecutionPolicy({ ...OFFLINE_EXECUTION_POLICY, canvasOAuth: { version: 'unknown', enabled: true } }));
  } finally { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); }
});
