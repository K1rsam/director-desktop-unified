import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { rustStandardActionCatalogue } from './test-support/rustExecutionPolicy.ts';
type Slot = { value?: any; deps?: unknown[]; cleanup?: () => void };
type Frame = { type: unknown; slots: Slot[]; index: number };
type Host = { type: string; props: Record<string, any>; children: (Host | string)[] };
function hookRenderer() {
  const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
  const frames = new Map<string, Frame>();
  let frame: Frame, changed = false, effects: (() => void)[] = [], hosts: Host[] = [];
  const same = (a?: unknown[], b?: unknown[]) => a !== undefined && b !== undefined && a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
  const slot = () => frame.slots[frame.index] || (frame.slots[frame.index] = {});
  const dispatcher = {
    useState(initial: unknown) { const item = slot(); frame.index++; if (!('value' in item)) item.value = typeof initial === 'function' ? initial() : initial; return [item.value, (update: any) => { const next = typeof update === 'function' ? update(item.value) : update; if (!Object.is(item.value, next)) { item.value = next; changed = true; } }]; },
    useRef(initial: unknown) { const item = slot(); frame.index++; if (!('value' in item)) item.value = { current: initial }; return item.value; },
    useMemo(create: () => unknown, deps?: unknown[]) { const item = slot(); frame.index++; if (!same(item.deps, deps)) { item.value = create(); item.deps = deps; } return item.value; },
    useCallback(callback: unknown, deps?: unknown[]) { return this.useMemo(() => callback, deps); },
    useEffect(create: () => void | (() => void), deps?: unknown[]) { const item = slot(); frame.index++; if (!same(item.deps, deps)) { item.deps = deps; effects.push(() => { item.cleanup?.(); item.cleanup = create() || undefined; }); } },
    useSyncExternalStore(_subscribe: unknown, snapshot: () => unknown) { return snapshot(); },
  };
  function render(element: any) {
    let pass = 0;
    do {
      changed = false; effects = []; hosts = [];
      const visited = new Set<string>();
      function visit(value: any, path: string): (Host | string)[] {
        if (value == null || typeof value === 'boolean') return [];
        if (Array.isArray(value)) return value.flatMap((item, index) => visit(item, `${path}.${item?.key || index}`));
        if (typeof value !== 'object') return [String(value)];
        if (typeof value.type === 'function') {
          let entry = frames.get(path);
          if (entry && entry.type !== value.type) { entry.slots.forEach(item => item.cleanup?.()); frames.delete(path); entry = undefined; }
          if (!entry) { entry = { type: value.type, slots: [], index: 0 }; frames.set(path, entry); }
          visited.add(path); entry.index = 0; frame = entry;
          const previous = internals.H; internals.H = dispatcher;
          let result: unknown;
          try { result = value.type(value.props); } finally { internals.H = previous; }
          return visit(result, path + '.render');
        }
        const children = visit(value.props?.children, path + '.children');
        if (typeof value.type !== 'string') return children;
        const host = { type: value.type, props: value.props, children }; hosts.push(host); return [host];
      }
      visit(element, 'root');
      for (const [path, entry] of frames) if (!visited.has(path)) { entry.slots.forEach(item => item.cleanup?.()); frames.delete(path); }
      effects.forEach(effect => effect());
      assert.ok(++pass < 20, 'effects must settle');
    } while (changed);
    return hosts;
  }
  return { render, unmount() { frames.forEach(entry => entry.slots.forEach(item => item.cleanup?.())); frames.clear(); } };
}
function content(host: Host | string): string { return typeof host === 'string' ? host : host.children.map(content).join(''); }
const settle = () => new Promise(resolve => setImmediate(resolve));

test('a busy Dreamina refresh waits only for local snapshots, with bounded and view-bound feedback', async t => {
for (const mode of ['already-completed', 'pending-completed', 'timeout', 'leave', 'unmount', 'changed-settings', 'changed-account', 'changed-authorization', 'direct-error']) await t.test(mode, async () => {
  const calls: any[] = [], timeouts: (() => void)[] = [], intervals = new Map<number, () => void>(); let timer = 0;
  const prior = '2026-10-10T00:00:00Z', fresh = '2026-10-10T00:00:01Z';
  let software: any = { schemaVersion: 'app-settings/v1', revision: 4, settings: { textProvider: 'codex_cli', dreaminaPath: '/auto/dreamina' }, providers: [], rules: [], dreaminaAccount: { status: 'authenticated', ready: true, identityVerified: true, accountKey: 'synthetic-account', checkedAt: prior, credits: { total: 10 }, message: '旧状态' } };
  const complete = () => { software = { ...software, dreaminaAccount: { ...software.dreaminaAccount, checkedAt: fresh, credits: { total: 20 }, message: '已有检查已完成，账户身份与积分已核对。' } }; };
  const policy = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', reasons: [], allowedActions: rustStandardActionCatalogue(), readiness: { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'dreamina'], generationAllowed: false }, textBackends: [{ id: 'codex_cli', status: 'enabled' }] };
  const respond = async (input: any) => {
    calls.push(input);
    if (input.action === 'getExecutionPolicy') return policy;
    if (input.action === 'projects.list') return [];
    if (input.action === 'settings.global.get') return structuredClone(software);
    assert.equal(input.scope, 'global'); assert.equal(input.projectId, undefined);
    if (input.action === 'chrome.status') return { schemaVersion: 'chrome-setup/v1', installed: false, configured: false, status: 'unconfigured', message: '尚未安装', clients: 0, usableClients: 0 };
    if (input.action === 'runtime.status') {
      const id = input.provider === 'dreamina' ? 'dreamina' : 'codex', key = id === 'dreamina' ? 'dreaminaPath' : 'codexPath';
      return { schemaVersion: 'runtime-status/v1', provider: input.provider, installed: true, installSupported: true, checkedAt: prior, settings: { [key]: '/auto/' + id }, components: [{ id, name: id, state: 'installed', version: 'synthetic', path: '/auto/' + id }] };
    }
    if (input.action === 'settings.global.save') { assert.equal(input.expectedRevision, software.revision); software = { ...software, revision: software.revision + 1, settings: input.settings }; return structuredClone(software); }
    if (input.action === 'dreamina.status') {
      if (mode === 'direct-error') { software = { ...software, dreaminaAccount: { status: 'error', ready: false, identityVerified: false, checkedAt: fresh, message: '账户检查网络中断，请稍后重试。' } }; return software.dreaminaAccount; }
      if (mode === 'already-completed') complete();
      return { schemaVersion: 'dreamina-refresh/v1', refreshState: mode === 'already-completed' ? 'completed' : 'pending', settingsRevision: software.revision, configuredPath: software.settings.dreaminaPath, accountKey: 'synthetic-account', afterStatus: 'authenticated', afterCheckedAt: prior, account: software.dreaminaAccount, message: mode === 'already-completed' ? software.dreaminaAccount.message : '即梦已有操作正在进行，正在等待最新账户状态；不会重复发起检查。' };
    }
    throw new Error(`Unexpected action: ${input.action}`);
  };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { hidden: false, addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {}, setInterval(callback: () => void) { intervals.set(++timer, callback); return timer; }, clearInterval(id: number) { intervals.delete(id); }, setTimeout(callback: () => void, ms: number) { assert.equal(ms, 1000); timeouts.push(callback); return ++timer; }, __TAURI_INTERNALS__: { invoke: async (_command: string, args: { request: any }) => respond(args.request) } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' }), view = hookRenderer();
  try {
    const { default: App } = await server.ssrLoadModule('/src/App.tsx'); const element = React.createElement(App);
    const rendered = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await rendered();
    nodes.find(node => node.type === 'button' && content(node) === '软件设置')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    const oauth = nodes.find(node => node.type === 'button' && node.props['data-execution-action'] === 'dreamina.login'); assert.ok(oauth); assert.equal(oauth.props.disabled, false, 'the full native policy must leave Dreamina authorization usable');
    assert.ok(!nodes.map(content).join(' ').includes('标准版功能策略无效'));
    const refresh = nodes.find(node => node.type === 'button' && content(node) === '刷新登录状态'); assert.ok(refresh); assert.equal(refresh.props.disabled, false);
    refresh.props.onClick(); nodes = await rendered(); nodes = await rendered(); nodes = await rendered();
    assert.equal(calls.filter(input => input.action === 'dreamina.status').length, 1);
    const callsAfterStatus = calls.findIndex(input => input.action === 'dreamina.status') + 1;
    if (!['already-completed', 'direct-error'].includes(mode)) {
      assert.ok(nodes.map(content).join(' ').includes('正在等待最新账户状态'));
      assert.equal(timeouts.length, 1);
      if (mode === 'leave' || mode === 'unmount') {
        const reads = calls.filter(input => input.action === 'settings.global.get').length;
        if (mode === 'leave') { nodes.find(node => node.type === 'button' && content(node).startsWith('所有项目'))!.props.onClick(); nodes = await rendered(); }
        else view.unmount();
        complete(); timeouts.shift()!(); await settle();
        assert.equal(calls.filter(input => input.action === 'settings.global.get').length, reads, 'leaving cancels unstarted snapshot reads');
        assert.equal(timeouts.length, 0);
        if (mode === 'leave') { nodes = view.render(element); assert.ok(!nodes.map(content).join(' ').includes('账户身份与积分已核对')); }
      } else {
        if (mode === 'pending-completed' || mode === 'changed-account') complete();
        if (mode === 'changed-settings') software = { ...software, revision: software.revision + 1, settings: { ...software.settings, dreaminaPath: '/other/dreamina' } };
        if (mode === 'changed-account') software.dreaminaAccount.accountKey = 'other-synthetic-account';
        if (mode === 'changed-authorization') software.dreaminaAccount = { status: 'authorizing', ready: false, identityVerified: false, checkedAt: fresh, authorization: { expiresAt: '2026-10-10T00:10:00Z' } };
        const limit = mode === 'timeout' ? 45 : 1;
        for (let count = 0; count < limit; count++) { assert.equal(timeouts.length, 1); timeouts.shift()!(); nodes = await rendered(); }
        assert.equal(timeouts.length, 0, 'waiting has a fixed bound');
      }
    }
    if (mode.includes('completed')) {
      assert.ok(nodes.map(content).join(' ').includes('账户身份与积分已核对'));
      assert.equal(software.dreaminaAccount.checkedAt, fresh);
    }
    if (mode === 'timeout') { assert.ok(nodes.map(content).join(' ').includes('尚未取得新的即梦账户检查结果')); assert.equal(software.dreaminaAccount.checkedAt, prior); }
    if (mode.startsWith('changed-')) assert.ok(nodes.some(node => node.props.role === 'alert' && /等待期间即梦/.test(content(node))), 'a different config/account never satisfies the old refresh');
    if (mode === 'direct-error') assert.ok(nodes.map(content).join(' ').includes('账户检查网络中断'));
    assert.ok(!nodes.map(content).join(' ').includes('已刷新即梦登录状态'), 'fixed success must not replace pending/error/actual result');
    assert.ok(calls.slice(callsAfterStatus).every(input => input.action === 'settings.global.get' || input.action === 'projects.list'), 'waiting performs only local reads, no second status/readiness/login/generation');
    assert.equal(calls.filter(input => input.action === 'dreamina.status').length, 1);
    assert.ok(!calls.some(input => ['readiness.check', 'runtime.prepare', 'dreamina.login', 'images.run', 'videos.submit', 'workspace.get'].includes(input.action)));
  } finally { view.unmount(); await server.close(); }
});
});

test('App software settings preserve the whole save/check chain without a project', async t => {
for (const mode of ['ordinary', 'poll', 'leave-while-queued']) await t.test(mode === 'ordinary' ? 'ordinary SDK save does not revoke its own permissions' : mode === 'poll' ? 'SDK check waits for an existing Dreamina account poll, then completes' : 'leaving settings cancels a save queued behind an account poll', async () => {
  const concurrentPoll = mode !== 'ordinary';
  const calls: any[] = [], timers = new Map<number, () => void>();
  let timerId = 0, resolveSave!: (value: unknown) => void, resolvePoll!: (value: unknown) => void;
  const policy = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', reasons: [], allowedActions: rustStandardActionCatalogue(), readiness: { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'codex_sdk', 'dreamina'], generationAllowed: false }, textBackends: [{ id: 'codex_cli', status: 'enabled' }, { id: 'codex_sdk', status: 'explicit_selection_when_ready' }] };
  let appSettings = { schemaVersion: 'app-settings/v1', revision: 0, settings: { textProvider: 'codex_sdk', textModel: 'test-model', reasoning: 'medium' }, providers: [], rules: [], ...(concurrentPoll ? { dreaminaAccount: { status: 'authorizing', ready: false, identityVerified: false, authorization: { pollIntervalSeconds: 3 } } } : {}) };
  const respond = async (input: any) => {
    calls.push(input);
    if (input.action === 'getExecutionPolicy') return policy;
    if (input.action === 'projects.list') return [];
    if (input.action === 'settings.global.get') return appSettings;
    assert.equal(input.scope, 'global'); assert.equal(input.projectId, undefined, 'software connections must not manufacture a project');
    if (input.action === 'dreamina.status') return new Promise(resolve => { resolvePoll = value => resolve(value); });
    if (input.action === 'chrome.status') return { schemaVersion: 'chrome-setup/v1', installed: false, configured: false, status: 'unconfigured', message: '尚未安装', clients: 0, usableClients: 0 };
    if (input.action === 'runtime.status') {
      const ids = input.provider === 'codex_sdk' ? ['node', 'codex'] : [input.provider === 'codex_cli' ? 'codex' : 'dreamina'];
      const keys: Record<string, string> = { codex: 'codexPath', node: 'textSdkNodePath', dreamina: 'dreaminaPath' };
      return { schemaVersion: 'runtime-status/v1', provider: input.provider, installed: true, installSupported: true, checkedAt: '2026-10-09T06:00:00Z', settings: Object.fromEntries(ids.map(id => [keys[id], '/auto/' + id])), components: ids.map(id => ({ id, name: id, state: 'installed', version: 'synthetic', path: '/auto/' + id })) };
    }
    if (input.action === 'runtime.prepare') return { schemaVersion: 'runtime-prepare/v1', provider: 'codex_sdk', settings: { codexPath: '/auto/codex', textSdkNodePath: '/auto/node' } };
    if (input.action === 'settings.global.save') {
      assert.equal(input.expectedRevision, 0); assert.equal(input.settings.textModel, 'test-model');
      assert.equal(input.settings.textApiKey, undefined, 'blank token never clears credentials');
      return new Promise(resolve => { resolveSave = value => { appSettings = value as typeof appSettings; resolve(value); }; });
    }
    if (input.action === 'readiness.check') return { schemaVersion: 'provider-readiness/v1', provider: 'codex_sdk', checkedAt: '2026-10-09T06:00:00Z', installation: { state: 'ready', version: 'synthetic', executable: '/auto/node' }, protocol: { state: 'ready', name: 'synthetic' }, authentication: { state: 'authenticated', identityVerified: true }, catalog: { state: 'available', models: [{ id: 'test-model', supportedReasoningEfforts: ['medium'] }] }, network: { state: 'not_tested', proof: [] }, generation: { enabled: true, testPerformed: false, calls: 0 }, messages: [], details: { officialRuntimeChecked: true, accountChecked: true, runtimeExecutable: '/auto/codex' } };
    throw new Error(`Unexpected action: ${input.action}`);
  };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { hidden: false, addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {}, setInterval(callback: () => void) { timers.set(++timerId, callback); return timerId; }, clearInterval(id: number) { timers.delete(id); }, __TAURI_INTERNALS__: { invoke: async (_command: string, args: { request: any }) => respond(args.request) } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const view = hookRenderer();
  try {
    const { default: App } = await server.ssrLoadModule('/src/App.tsx');
    const element = React.createElement(App);
    view.render(element); await settle();
    let nodes = view.render(element);
    const settingsButton = nodes.find(node => node.type === 'button' && content(node) === '软件设置'); assert.ok(settingsButton);
    assert.equal(settingsButton.props.disabled, undefined, 'no-project setup entry remains available');
    settingsButton.props.onClick(); view.render(element); await settle(); nodes = view.render(element); await settle(); nodes = view.render(element);
    assert.ok(nodes.some(node => node.type === 'h1' && content(node) === '软件设置'));
    assert.ok(!nodes.some(node => node.props['data-execution-mode']), 'global settings never displays stage status banner');
    assert.ok(!nodes.some(node => node.type === 'button' && content(node) === '导出项目'));
    assert.ok(!nodes.some(node => node.type === 'nav' && node.props['aria-label'] === '制作阶段'));
    assert.ok(!nodes.map(content).join(' ').includes('先选择一个项目'));
    const connect = nodes.find(node => node.type === 'button' && node.props['data-readiness-provider'] === 'codex_sdk'); assert.ok(connect);
    if (concurrentPoll) {
      assert.equal(timers.size, 1); [...timers.values()][0](); await settle(); view.render(element);
      assert.equal(calls.filter(item => item.action === 'dreamina.status').length, 1);
    }
    connect.props.onClick(); await settle(); nodes = view.render(element);
    assert.equal(calls.filter(item => item.action === 'readiness.check').length, 0, 'waiting save must not start checking');
    assert.ok(nodes.some(node => node.type === 'span' && content(node) === '正在处理'));
    if (concurrentPoll) {
      assert.equal(calls.filter(item => item.action === 'settings.global.save').length, 0, 'the interactive save queues instead of cancelling silently');
      [...timers.values()][0](); await settle(); assert.equal(calls.filter(item => item.action === 'dreamina.status').length, 1, 'background polls never queue behind a pending click');
      if (mode === 'leave-while-queued') {
        nodes.find(node => node.type === 'button' && content(node).startsWith('所有项目'))!.props.onClick(); nodes = view.render(element);
        resolvePoll(appSettings); await settle(); nodes = view.render(element);
        assert.ok(!calls.some(item => ['settings.global.save', 'readiness.check'].includes(item.action)), 'unstarted requests from the old page cannot run');
        assert.ok(!nodes.some(node => node.props.role === 'alert'));
        return;
      }
      appSettings = { ...appSettings, dreaminaAccount: { status: 'authenticated', ready: true, identityVerified: true, authorization: { pollIntervalSeconds: 3 } } };
      resolvePoll(appSettings); await settle(); nodes = view.render(element);
    }
    const saved = calls.find(item => item.action === 'settings.global.save'); assert.ok(saved);
    resolveSave({ ...appSettings, revision: 1, settings: saved.settings }); await settle(); nodes = view.render(element); await settle(); nodes = view.render(element);
    assert.deepEqual(calls.filter(item => ['runtime.prepare', 'settings.global.save', 'readiness.check'].includes(item.action)).map(item => item.action), ['settings.global.save', 'readiness.check']);
    assert.ok(nodes.some(node => node.props['data-sdk-execution-state'] === 'ready'), 'App save busy does not revoke component permission or drop its result');
    assert.ok(calls.every(item => !['workspace.get', 'projects.create', 'foundation.run', 'images.run', 'videos.prepare', 'videos.submit'].includes(item.action)));
    assert.equal(timers.size, 0, 'opening software settings creates no project or generation polling');
  } finally { view.unmount(); await server.close(); }
});
});

test('saved software choices immediately project into stages and automatic discovery uses global scope', async () => {
  const calls: any[] = [], timers = new Map<number, () => void>(); let timerId = 0;
  let inspectionFailure = '', rejectInspection: ((reason: Error) => void) | undefined;
  const catalog = { id: 'dreamina', available: true, configured: true, models: [{ id: '5.0', kind: 'image', adapted: true, ratios: ['9:16'], resolutions: ['2k'] }, { id: 'video-model', kind: 'video', adapted: true, ratios: ['9:16'], resolutions: ['720p'] }] };
  const frozenJob = { id: 'old-job', kind: 'workflow', status: 'completed', provider: 'codex', snapshot: { settings: { textProvider: 'codex', textModel: 'old-model', reasoning: 'low' } } };
  const stored = { project: { id: 'existing-project', title: '已存在的剧本' }, source: null, assets: [{ id: 'asset-1', name: '角色', kind: 'CH', prompt: '既有提示词' }], media: [], units: [], jobs: [frozenJob], settings: { textProvider: 'codex', textModel: 'old-model', reasoning: 'low', ratio: '9:16', projectVisualStyle: '水墨', fantasyMode: 'off', imageProvider: 'dreamina', videoProvider: 'dreamina' }, providers: [], rules: [] };
  let software: any = { schemaVersion: 'app-settings/v1', revision: 0, settings: { textProvider: 'codex', textModel: 'old-model', reasoning: 'low' }, providers: [], rules: [] };
  const policy = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', reasons: [], allowedActions: ['projects.list', 'workspace.get', 'assets.get', 'settings.global.get', 'settings.global.save', 'runtime.status', 'chrome.status', 'providers.refresh', 'readiness.check', 'foundation.run'], readiness: { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'codex_sdk', 'dreamina'], generationAllowed: false }, textBackends: [{ id: 'codex_cli', status: 'enabled' }, { id: 'codex_sdk', status: 'explicit_selection_when_ready' }] };
  const respond = async (input: any) => {
    calls.push(input);
    if (input.action === 'getExecutionPolicy') return policy;
    if (input.action === 'projects.list') return [stored.project];
    if (input.action === 'workspace.get') return structuredClone(stored); // Deliberately stale software fields.
    if (input.action === 'assets.get') return stored.assets[0];
    if (input.action === 'settings.global.get') return software;
    assert.equal(input.scope, 'global'); assert.equal(input.projectId, undefined);
    if (input.action === 'providers.refresh') {
      if (inspectionFailure === 'pending') return new Promise((_resolve, reject) => { rejectInspection = reject; });
      if (inspectionFailure) throw new Error(inspectionFailure);
      software = { ...software, providers: [catalog] }; return software;
    }
    if (input.action === 'chrome.status') return { schemaVersion: 'chrome-setup/v1', installed: false, configured: false, status: 'unconfigured', message: '尚未安装', clients: 0, usableClients: 0 };
    if (input.action === 'runtime.status') {
      const ids = input.provider === 'codex_sdk' ? ['node', 'codex'] : [input.provider === 'codex_cli' ? 'codex' : 'dreamina']; const keys: Record<string, string> = { node: 'textSdkNodePath', codex: 'codexPath', dreamina: 'dreaminaPath' };
      return { schemaVersion: 'runtime-status/v1', provider: input.provider, installed: true, installSupported: true, checkedAt: '2026-10-09T06:00:00Z', settings: Object.fromEntries(ids.map(id => [keys[id], '/auto/' + id])), components: ids.map(id => ({ id, name: id, state: 'installed', path: '/auto/' + id, version: 'synthetic' })) };
    }
    if (input.action === 'settings.global.save') { assert.equal(input.expectedRevision, software.revision); software = { ...software, revision: software.revision + 1, settings: input.settings }; return software; }
    if (input.action === 'readiness.check') return { schemaVersion: 'provider-readiness/v1', provider: 'codex_sdk', checkedAt: '2026-10-09T06:00:00Z', installation: { state: 'ready', version: 'synthetic', executable: '/auto/node' }, protocol: { state: 'ready', name: 'synthetic' }, authentication: { state: 'authenticated', identityVerified: true }, catalog: { state: 'available', models: [{ id: 'new-model', supportedReasoningEfforts: ['medium', 'high'] }] }, network: { state: 'not_tested', proof: [] }, generation: { enabled: true, testPerformed: false, calls: 0 }, messages: [], details: { officialRuntimeChecked: true, accountChecked: true, runtimeExecutable: '/auto/codex' } };
    throw new Error(`Unexpected action: ${input.action}`);
  };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { hidden: false, addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {}, setInterval(callback: () => void) { timers.set(++timerId, callback); return timerId; }, clearInterval(id: number) { timers.delete(id); }, __TAURI_INTERNALS__: { invoke: async (_command: string, args: { request: any }) => respond(args.request) } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' }), view = hookRenderer();
  try {
    const { default: App, projectWorkspaceView } = await server.ssrLoadModule('/src/App.tsx'); const element = React.createElement(App);
    const rendered = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await rendered();
    nodes.find(node => node.props['aria-label'] === '打开项目：已存在的剧本')!.props.onClick(); nodes = await rendered(); await settle(); nodes = await rendered();
    assert.ok(calls.some(item => item.action === 'providers.refresh'), `missing stage models are inspected: ${calls.map(item => item.action).join(', ')}`);
    assert.ok(calls.filter(item => item.action === 'providers.refresh').every(item => item.scope === 'global' && item.projectId === undefined), 'automatic inspection cannot enter project task recovery');
    nodes.find(node => node.type === 'button' && content(node) === '软件设置')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    nodes.find(node => node.props['data-text-provider'])!.props.onChange({ target: { value: 'codex_sdk' } }); nodes = await rendered();
    nodes.find(node => node.props['data-readiness-provider'] === 'codex_sdk')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    nodes.find(node => node.props['data-text-model'])!.props.onChange({ target: { value: 'new-model' } }); nodes = view.render(element);
    nodes.find(node => node.props['data-text-reasoning'])!.props.onChange({ target: { value: 'high' } }); nodes = view.render(element);
    const form = nodes.find(node => node.type === 'form'); assert.ok(form); form.props.onSubmit({ preventDefault() {} }); nodes = await rendered();
    nodes.find(node => node.type === 'button' && content(node).startsWith('所有项目'))!.props.onClick(); nodes = await rendered();
    nodes.find(node => node.props['aria-label'] === '打开项目：已存在的剧本')!.props.onClick(); nodes = await rendered();
    const summary = nodes.find(node => node.props.className === 'text-execution-summary'); assert.ok(summary);
    assert.match(content(summary), /Codex SDK.*new-model.*high/);
    assert.doesNotMatch(content(summary), /old-model/);
    const projected = projectWorkspaceView(stored, { ...software, settings: { ...software.settings, ratio: '16:9', projectVisualStyle: '不得覆盖', bhToken: 'must-not-project' } });
    assert.equal(projected.settings.ratio, '9:16'); assert.equal(projected.settings.projectVisualStyle, '水墨'); assert.equal(projected.settings.fantasyMode, 'off');
    assert.equal(projected.settings.bhToken, undefined); assert.equal(projected.jobs, stored.jobs); assert.equal(projected.jobs[0].snapshot.settings.textModel, 'old-model'); assert.equal(stored.settings.textProvider, 'codex');
    assert.equal(projected.providers, software.providers);
    const inspectionsBefore = calls.filter(item => item.action === 'providers.refresh').length;
    nodes.find(node => node.type === 'button' && node.props.className?.includes('stage-link') && content(node).includes('图片资产'))!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    const manualCheck = nodes.find(node => node.type === 'button' && content(node) === '核实模型'); assert.ok(manualCheck); manualCheck.props.onClick(); nodes = await rendered();
    assert.equal(calls.filter(item => item.action === 'providers.refresh').length, inspectionsBefore + 1);
    assert.ok(calls.filter(item => item.action === 'providers.refresh').every(item => item.scope === 'global' && item.projectId === undefined), 'manual stage checks also remain software scoped');
    inspectionFailure = 'synthetic model connection failure';
    nodes.find(node => node.type === 'button' && content(node) === '核实模型')!.props.onClick(); nodes = await rendered();
    assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes(inspectionFailure)), 'a manual check failure is visible on the asset page');
    inspectionFailure = 'pending';
    nodes.find(node => node.type === 'button' && content(node) === '核实模型')!.props.onClick(); nodes = await rendered();
    assert.ok(!nodes.some(node => node.props.role === 'alert'), 'retry clears the old error');
    nodes.find(node => node.type === 'button' && node.props.className?.includes('stage-link') && content(node).includes('剧本与基础'))!.props.onClick(); nodes = view.render(element);
    rejectInspection!(new Error('late error from asset page')); await settle(); nodes = view.render(element);
    assert.ok(!nodes.some(node => node.props.role === 'alert' && content(node).includes('late error')), 'a response from the page left behind cannot overwrite current feedback');
    assert.ok(!calls.some(item => ['settings.save', 'foundation.run', 'images.run', 'videos.submit'].includes(item.action)), 'no project record or generation is written');
  } finally { view.unmount(); await server.close(); }
});
