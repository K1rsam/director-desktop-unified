import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { Settings, ProviderReadinessReport } from './types.ts';
import type { PolicyRequest } from './executionPolicy.ts';

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
const settleText = async () => { for (let i = 0; i < 8; i++) await new Promise(resolve => setImmediate(resolve)); };
function connectedReport(provider: ProviderReadinessReport['provider']): ProviderReadinessReport {
  return {
    schemaVersion: 'provider-readiness/v1', provider, checkedAt: '2026-10-09T06:00:00Z',
    installation: { state: 'ready', version: 'synthetic', executable: provider === 'codex_sdk' ? '/auto/node' : '/auto/codex' }, protocol: { state: 'ready', name: 'synthetic' },
    authentication: { state: 'authenticated', identityVerified: true },
    catalog: { state: 'available', models: [{ id: 'test-model', supportedReasoningEfforts: ['medium', 'high'], defaultReasoningEffort: 'medium' }] },
    network: { state: 'not_tested', proof: [] }, generation: { enabled: provider === 'codex_sdk', testPerformed: false, calls: 0 }, messages: [],
    details: provider === 'codex_sdk' ? { officialRuntimeChecked: true, accountChecked: true, runtimeExecutable: '/auto/codex' } : {},
  };
}
function visibleText(host: Host | string): string {
  if (typeof host === 'string') return host;
  const children = host.type === 'details' && !host.props.open ? host.children.filter(child => typeof child !== 'string' && child.type === 'summary') : host.children;
  return children.map(visibleText).join('');
}

function localStatus(provider: string, installed = true) {
  const ids = provider === 'codex_sdk' ? ['node', 'codex'] : ['codex'];
  const key = (id: string) => id === 'node' ? 'textSdkNodePath' : 'codexPath';
  return { schemaVersion: 'runtime-status/v1', provider, installed, installSupported: true, checkedAt: '2026-10-09T06:00:00Z',
    components: ids.map(id => ({ id, name: id === 'node' ? '文本运行环境' : 'Codex', state: installed ? 'installed' : 'missing', version: null, ...(installed ? { path: `/auto/${id}` } : {}) })),
    settings: installed ? Object.fromEntries(ids.map(id => [key(id), `/auto/${id}`])) : {} };
}

test('OpenAI components and shared account have separate, scoped setup and connection lifecycles', async t => {
  const calls: PolicyRequest[] = [], saves: Partial<Settings>[] = [];
  let respond: (input: PolicyRequest) => Promise<unknown>;
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, window: { __TAURI_INTERNALS__: { invoke: async (_command: string, args: { request: PolicyRequest }) => { calls.push(args.request); return respond(args.request); } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  try {
    const policy = await server.ssrLoadModule('/src/executionPolicy.ts');
    const { TextExecutionSettings } = await server.ssrLoadModule('/src/TextExecutionSettings.tsx');
    const { invalidateProviderReadiness } = await server.ssrLoadModule('/src/ProviderReadiness.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const standard = { version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: [], allowedActions: ['runtime.status', 'runtime.prepare', 'readiness.check', 'textCli.login', 'textCli.loginStatus', 'textCli.openLogin', 'textCli.cancelLogin'], textBackends: [{ id: 'codex_cli', status: 'enabled' }, { id: 'codex_sdk', status: 'explicit_selection_when_ready' }] };
    let installed = true;
    const defaultRespond = async (input: PolicyRequest) => {
      if (input.action === 'runtime.status') return localStatus(String(input.provider), installed);
      if (input.action === 'runtime.prepare') { installed = true; return { schemaVersion: 'runtime-prepare/v1', provider: input.provider, settings: localStatus(String(input.provider)).settings }; }
      if (input.action === 'readiness.check') return connectedReport(input.provider as ProviderReadinessReport['provider']);
      if (input.action === 'textCli.login' || input.action === 'textCli.loginStatus') return { login: { state: 'pending', loginId: 'synthetic-login' } };
      if (input.action === 'textCli.cancelLogin') return { login: { state: 'cancelled' } };
      return { login: { state: 'pending', loginId: 'synthetic-login' } };
    };
    async function fixture(provider = 'codex_sdk', options: { installed?: boolean; saveSuccess?: boolean; appBusy?: boolean; scope?: 'global'|'project'; stalePaths?: boolean } = {}) {
      policy.installExecutionPolicy(standard); calls.length = 0; saves.length = 0; respond = defaultRespond; installed = options.installed !== false;
      invalidateProviderReadiness(['codex_cli', 'codex_sdk']);
      const view = hookRenderer();
      let settings: Settings = { textProvider: provider, textModel: 'test-model', reasoning: 'medium', codexPath: options.stalePaths ? '/old/codex' : '/auto/codex', textSdkNodePath: options.stalePaths ? '/old/node' : '/auto/node' };
      let projectId = 'text-setup-project', writable = true, connectionAllowed = true, scopeMode = options.scope || 'global';
      function render() {
        const props = { ...(scopeMode === 'global' ? { requestScope: { scope: 'global' } } : { workspace: { ...previewWorkspace, project: { ...previewWorkspace.project, id: projectId } } }), settings, writable, connectionAllowed,
          update(key: string, value: string) { settings = { ...settings, [key]: value }; },
          async saveSettings(patch: Partial<Settings> = {}) {
            saves.push(patch);
            if (options.appBusy) { writable = false; render(); await settleText(); }
            if (options.saveSuccess !== false) { settings = { ...settings, ...patch }; render(); }
            if (options.appBusy) { writable = true; render(); }
            return options.saveSuccess !== false;
          } };
        return view.render(React.createElement<typeof props>(TextExecutionSettings, props));
      }
      render(); await settleText(); render();
      const openingCalls = [...calls]; calls.length = 0;
      return { render, view, openingCalls, settings: () => settings,
        changeProvider(next: string) { settings = { ...settings, textProvider: next }; return render(); },
        changeProject(next: string) { projectId = next; return render(); },
        changeScope(next: 'global'|'project') { scopeMode = next; return render(); },
        changePaths(patch: Partial<Settings>) { settings = { ...settings, ...patch }; return render(); },
        makeReadOnly() { writable = false; connectionAllowed = false; return render(); } };
    }
    const button = (nodes: Host[], action: string) => { const found = nodes.find(node => node.type === 'button' && node.props['data-execution-action'] === action); assert.ok(found, action); return found; };
    const summary = (nodes: Host[]) => visibleText(nodes.find(node => node.type === 'section')!);
    const actions = () => calls.map(call => call.action);

    await t.test('one OpenAI card opens with local inventory only and exposes no program paths or API credentials', async () => {
      const f = await fixture(); try {
        const nodes = f.render();
        assert.deepEqual(f.openingCalls.map(input => input.action), ['runtime.status']);
        assert.equal(f.openingCalls[0].scope, 'global'); assert.equal('projectId' in f.openingCalls[0], false);
        assert.equal(nodes.filter(node => node.type === 'input').length, 0);
        assert.equal(nodes.filter(node => node.type === 'select').length, 3);
        assert.deepEqual(nodes.find(node => node.props['data-text-provider'])?.children.map(item => typeof item === 'string' ? item : item.props.value), ['codex', 'codex_sdk']);
        assert.match(summary(nodes), /OpenAI.*SDK 与 CLI 共用 OpenAI 账户/s);
        assert.match(summary(nodes), /已安装/); assert.doesNotMatch(summary(nodes), /Node\.js|\/auto\/|文本 API|握手/);
        assert.ok(!nodes.find(node => node.type === 'details')?.props.open);
        assert.equal(calls.length, 0);
      } finally { f.view.unmount(); }
    });
    await t.test('legacy API settings remain unchanged while normal OpenAI card does not expose API form', async () => {
      const f = await fixture('api'); try { assert.equal(f.openingCalls.length, 0); assert.equal(f.settings().textProvider, 'api'); assert.equal(f.render().filter(n => n.type === 'input').length, 0); assert.match(summary(f.render()), /请选择执行方式|原设置已保留/); } finally { f.view.unmount(); }
    });
    for (const provider of ['codex', 'codex_sdk']) await t.test(`${provider} installed components check without installation and survive parent's own save busy`, async () => {
      const f = await fixture(provider, { appBusy: true, stalePaths: true }); try {
        const trigger = button(f.render(), 'readiness.check'); trigger.props.onClick(); trigger.props.onClick();
        await settleText(); await settleText(); const nodes = f.render();
        assert.equal(actions().filter(action => action === 'readiness.check').length, 1);
        assert.ok(!actions().includes('runtime.prepare'));
        assert.equal(saves.length, 1); assert.deepEqual(saves[0], localStatus(provider === 'codex' ? 'codex_cli' : 'codex_sdk').settings);
        assert.match(summary(nodes), /账户与模型已连接/);
        assert.doesNotMatch(summary(nodes), /版本待检查/, 'a completed check does not keep displaying an unperformed version check');
        assert.equal(button(nodes, 'readiness.check').props.disabled, false);
        assert.equal(f.settings().textModel, 'test-model'); assert.equal(f.settings().reasoning, 'medium');
        for (const call of calls) { assert.equal(call.scope, 'global'); assert.equal('projectId' in call, false); }
      } finally { f.view.unmount(); }
    });
    await t.test('missing components cannot log in; only explicit install prepares, verifies, saves, then checks', async () => {
      const f = await fixture('codex_sdk', { installed: false, appBusy: true }); try {
        assert.equal(button(f.render(), 'textCli.login').props.disabled, true);
        assert.match(summary(f.render()), /重新检测组件/);
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); f.render();
        assert.deepEqual(actions(), ['runtime.status']); assert.equal(saves.length, 0); calls.length = 0;
        button(f.render(), 'textCli.login').props.onClick(); await settleText(); f.render();
        assert.ok(!actions().includes('textCli.login')); assert.ok(!actions().includes('runtime.prepare'));
        calls.length = 0;
        const install = button(f.render(), 'runtime.prepare'); install.props.onClick(); install.props.onClick();
        await settleText(); await settleText(); const nodes = f.render();
        assert.deepEqual(actions(), ['runtime.prepare', 'runtime.status', 'readiness.check']);
        assert.equal(saves.length, 1); assert.match(summary(nodes), /账户与模型已连接/);
        assert.equal(button(nodes, 'textCli.login').props.disabled, false);
        assert.ok(!nodes.some(node => node.props['data-execution-action'] === 'runtime.prepare'));
      } finally { f.view.unmount(); }
    });
    await t.test('damaged managed components show a real error and disable installation and login', async () => {
      const f = await fixture();
      respond = async input => input.action === 'runtime.status' ? { ...localStatus(String(input.provider), false), installSupported: false, components: localStatus(String(input.provider), false).components.map(component => ({ ...component, state: 'invalid', message: '安装文件校验失败，已保留原文件。' })) } : defaultRespond(input);
      try {
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); const nodes = f.render();
        assert.match(summary(nodes), /安装文件校验失败/);
        assert.equal(button(nodes, 'runtime.prepare').props.disabled, true);
        assert.equal(button(nodes, 'textCli.login').props.disabled, true);
        button(nodes, 'runtime.prepare').props.onClick(); await settleText();
        assert.ok(!actions().includes('runtime.prepare')); assert.equal(saves.length, 0);
      } finally { f.view.unmount(); }
    });
    await t.test('returning to global settings keeps last connection proof; explicit recheck is never skipped', async () => {
      const f = await fixture(); try {
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); f.render();
        f.view.unmount(); calls.length = 0; f.render(); await settleText(); const nodes = f.render();
        assert.deepEqual(actions(), ['runtime.status']); assert.match(summary(nodes), /账户与模型已连接/); assert.match(summary(nodes), /最近检查：/);
        button(nodes, 'readiness.check').props.onClick(); await settleText(); f.render();
        assert.equal(actions().filter(action => action === 'readiness.check').length, 1);
        assert.match(summary(f.changeProject('another-production-project')), /账户与模型已连接/);
      } finally { f.view.unmount(); }
    });
    await t.test('changed paths clear global proof and reverting cannot revive it', async () => {
      const f = await fixture(); try {
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); f.render();
        f.changePaths({ codexPath: '/different/codex' }); await settleText();
        assert.doesNotMatch(summary(f.render()), /账户与模型已连接/);
        f.changePaths({ codexPath: '/auto/codex' }); await settleText(); assert.doesNotMatch(summary(f.render()), /账户与模型已连接/);
      } finally { f.view.unmount(); }
    });
    for (const kind of ['provider', 'scope', 'project', 'permission'] as const) await t.test(`${kind} change drops late component discovery before save`, async () => {
      const f = await fixture('codex_sdk', { scope: kind === 'project' ? 'project' : 'global' }); let finish!: (value: unknown) => void;
      respond = input => input.action === 'runtime.status' ? new Promise(resolve => { finish = resolve; }) : defaultRespond(input);
      try {
        button(f.render(), 'readiness.check').props.onClick(); const first = finish;
        if (kind === 'provider') f.changeProvider('codex'); else if (kind === 'scope') f.changeScope('project'); else if (kind === 'project') f.changeProject('other'); else f.makeReadOnly();
        first(localStatus('codex_sdk')); await settleText(); f.render();
        assert.equal(saves.length, 0); assert.ok(!actions().includes('readiness.check'));
      } finally { f.view.unmount(); }
    });
    await t.test('actual permission withdrawal after save drops late readiness', async () => {
      const f = await fixture('codex_sdk', { appBusy: true }); let finish!: (value: unknown) => void;
      respond = input => input.action === 'readiness.check' ? new Promise(resolve => { finish = resolve; }) : defaultRespond(input);
      try { button(f.render(), 'readiness.check').props.onClick(); await settleText(); await settleText(); f.makeReadOnly(); finish(connectedReport('codex_sdk')); await settleText(); assert.doesNotMatch(summary(f.render()), /账户与模型已连接/); } finally { f.view.unmount(); }
    });
    await t.test('failed saves stop readiness and authorization', async () => {
      const f = await fixture('codex_sdk', { saveSuccess: false }); try {
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); f.render();
        button(f.render(), 'textCli.login').props.onClick(); await settleText(); f.render();
        assert.equal(saves.length, 2); assert.ok(!actions().includes('readiness.check')); assert.ok(!actions().includes('textCli.login'));
      } finally { f.view.unmount(); }
    });
    await t.test('external logout invalidates cached success and an earlier in-flight response', async () => {
      const f = await fixture(); let finish!: (value: unknown) => void;
      try {
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); f.render();
        respond = input => input.action === 'readiness.check' ? new Promise(resolve => { finish = resolve; }) : defaultRespond(input);
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); invalidateProviderReadiness(['codex_cli', 'codex_sdk']);
        finish(connectedReport('codex_sdk')); await settleText(); assert.doesNotMatch(summary(f.render()), /账户与模型已连接/);
        f.view.unmount(); f.render(); await settleText(); assert.doesNotMatch(summary(f.render()), /账户与模型已连接/);
      } finally { f.view.unmount(); }
    });
    await t.test('failed recheck cannot fall back to a previous success after remount', async () => {
      const f = await fixture(); try {
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); f.render();
        respond = async input => { if (input.action === 'readiness.check') throw new Error('synthetic check failed'); return defaultRespond(input); };
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); assert.match(summary(f.render()), /synthetic check failed/);
        f.view.unmount(); f.render(); await settleText(); assert.doesNotMatch(summary(f.render()), /账户与模型已连接/);
      } finally { f.view.unmount(); }
    });
    await t.test('shared OpenAI login is scoped, never installs, and completion triggers one read-only check', async () => {
      const f = await fixture(); const originalSet = globalThis.setTimeout, originalClear = globalThis.clearTimeout;
      let poll: (() => void) | undefined;
      globalThis.setTimeout = ((fn: () => void, ms: number, ...args: unknown[]) => ms === 2000 ? (poll = fn, 7654321) : originalSet(fn, ms, ...args)) as any;
      globalThis.clearTimeout = ((id: any) => { if (id === 7654321) poll = undefined; else originalClear(id); }) as any;
      try {
        button(f.render(), 'textCli.login').props.onClick(); await settleText(); f.render();
        assert.deepEqual(actions(), ['runtime.status', 'textCli.login']);
        assert.ok(poll); assert.match(summary(f.render()), /等待网页登录/);
        respond = async input => input.action === 'textCli.loginStatus' ? { login: { state: 'completed', loginId: 'synthetic-login' } } : defaultRespond(input);
        poll!(); await settleText(); f.render();
        assert.equal(actions().filter(action => action === 'readiness.check').length, 1);
        assert.match(summary(f.render()), /账户与模型已连接/);
        for (const call of calls) { assert.equal(call.scope, 'global'); assert.equal('projectId' in call, false); }
      } finally { f.view.unmount(); globalThis.setTimeout = originalSet; globalThis.clearTimeout = originalClear; }
    });
    await t.test('pending login exposes open and cancel actions; relogin clears previous proof', async () => {
      const f = await fixture(); try {
        button(f.render(), 'readiness.check').props.onClick(); await settleText(); f.render();
        button(f.render(), 'textCli.login').props.onClick(); await settleText(); const nodes = f.render();
        assert.doesNotMatch(summary(nodes), /账户与模型已连接/);
        button(nodes, 'textCli.openLogin').props.onClick(); await settleText(); f.render();
        button(f.render(), 'textCli.cancelLogin').props.onClick(); await settleText(); f.render();
        assert.ok(actions().includes('textCli.openLogin')); assert.ok(actions().includes('textCli.cancelLogin')); assert.ok(!actions().includes('runtime.prepare'));
      } finally { f.view.unmount(); }
    });
    await t.test('authorization polling has an elapsed deadline and never survives leaving the card', async () => {
      const f = await fixture(), originalSet = globalThis.setTimeout, originalClear = globalThis.clearTimeout, originalNow = Date.now;
      let poll: (() => void) | undefined, now = originalNow();
      globalThis.setTimeout = ((fn: () => void, ms: number, ...args: unknown[]) => ms === 2000 ? (poll = fn, 7654321) : originalSet(fn, ms, ...args)) as any;
      globalThis.clearTimeout = ((id: any) => { if (id === 7654321) poll = undefined; else originalClear(id); }) as any;
      Date.now = () => now;
      try {
        button(f.render(), 'textCli.login').props.onClick(); await settleText(); f.render();
        assert.ok(poll); now += 5 * 60 * 1000 + 1; poll!(); await settleText();
        assert.match(summary(f.render()), /登录等待已超时/); assert.equal(actions().filter(a => a === 'textCli.loginStatus').length, 0);
        button(f.render(), 'textCli.login').props.onClick(); await settleText(); f.render();
        const abandoned = poll; assert.ok(abandoned); const before = calls.length;
        f.view.unmount(); abandoned(); await settleText(); assert.equal(calls.length, before);
      } finally { f.view.unmount(); globalThis.setTimeout = originalSet; globalThis.clearTimeout = originalClear; Date.now = originalNow; }
    });
    await t.test('offline policy never installs or authorizes while allowing a local inspection', async () => {
      const f = await fixture(); try {
        policy.installExecutionPolicy(policy.OFFLINE_EXECUTION_POLICY); button(f.render(), 'textCli.login').props.onClick(); await settleText();
        assert.equal(calls.length, 0); assert.equal(saves.length, 0);
      } finally { f.view.unmount(); }
    });
    assert.ok(calls.every(call => !/(generate|foundation\.run|storyboard\.run|images\.design|videos\.)/.test(call.action)));
  } finally { await server.close(); }
});
