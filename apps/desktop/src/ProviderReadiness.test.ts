import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import type { PolicyRequest } from './executionPolicy.ts';
import type { ProviderReadinessReport } from './types.ts';

// Run actual component handlers and effects with React's hook dispatcher and
// synthetic IPC. No browser, native executable, account or model is contacted.
// SSR alone cannot observe the effects which used to auto-check accounts.
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
function result(provider: ProviderReadinessReport['provider'], model = 'synthetic-model'): ProviderReadinessReport {
  return { schemaVersion: 'provider-readiness/v1', provider, checkedAt: '2026-10-07T00:00:00Z', installation: { state: 'ready', version: 'synthetic-1', executable: '/synthetic/program' }, protocol: { state: 'ready', name: 'synthetic/protocol' }, authentication: { state: provider === 'codex_sdk' ? 'not_checked' : 'authenticated', identityVerified: provider !== 'codex_sdk' }, catalog: { state: provider === 'codex_sdk' ? 'not_checked' : 'available', models: provider === 'codex_sdk' ? [] : provider === 'dreamina_canvas' ? [{ id: model, name: 'Synthetic Canvas Image', kind: 'image' }] : [{ id: model, name: model, supportedReasoningEfforts: ['medium', 'high'] }] }, network: { state: 'not_tested', proof: [] }, generation: { enabled: false, testPerformed: false, calls: 0 }, messages: ['Synthetic read-only evidence'], details: provider === 'dreamina' ? { credits: { total: 50 }, freeImageEligibility: 'unverified' } : {} };
}
const settle = () => new Promise(resolve => setImmediate(resolve));

test('actual readiness controls and effects use only the explicit read-only IPC', async t => {
  const calls: PolicyRequest[] = [];
  const readiness = { version: 'provider-readiness/v1', enabled: true, providers: ['codex_cli', 'dreamina', 'dreamina_canvas', 'codex_sdk'], generationAllowed: false };
  let capability = { mode: 'offline_prepare_only', liveExecutionAllowed: false, stage3CreativePackStatus: 'inactive', reasons: ['synthetic capability'], readiness };
  let respond = async (input: PolicyRequest): Promise<unknown> => result(input.provider as ProviderReadinessReport['provider']);
  let pick = async (): Promise<string[]> => [];
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, window: { __TAURI_INTERNALS__: { invoke: async (_command: string, args: { request: PolicyRequest }) => { calls.push(args.request); return args.request.action === 'getExecutionPolicy' ? capability : args.request.action === 'files.choose' ? pick() : respond(args.request); } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false }, appType: 'custom' });
  try {
    const api = await server.ssrLoadModule('/src/api.ts');
    const policy = await server.ssrLoadModule('/src/executionPolicy.ts');
    const { TextExecutionSettings } = await server.ssrLoadModule('/src/TextExecutionSettings.tsx');
    const { SettingsPanel, DreaminaAccountCard } = await server.ssrLoadModule('/src/Panels.tsx');
    const { validateProviderReadiness, ProviderReadinessResult } = await server.ssrLoadModule('/src/ProviderReadiness.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    const workspace = { ...structuredClone(previewWorkspace), project: { id: 'readiness-project', title: 'Synthetic' }, settings: { codexPath: '/saved/codex', dreaminaPath: '/saved/dreamina', dreaminaCanvasPath: '/saved/canvas', imageProvider: 'dreamina', videoProvider: 'dreamina' } };
    const props = { workspace, settings: { textProvider: 'codex', codexPath: '/unsaved/CLI path' }, writable: true, update() {}, saveSettings: async () => { throw new Error('readiness must not save settings'); } };
    const button = (nodes: Host[], provider: string) => { const found = nodes.find(node => node.type === 'button' && node.props['data-readiness-provider'] === provider); assert.ok(found, `${provider} button exists`); return found; };
    const canvasInput = (nodes: Host[]) => { const found = nodes.find(node => node.type === 'input' && node.props['data-canvas-path']); assert.ok(found, 'Canvas input exists'); return found; };
    const picker = (nodes: Host[]) => { const found = nodes.find(node => node.type === 'button' && node.props['data-canvas-picker']); assert.ok(found, 'Canvas native picker exists'); return found; };
    const tab = (nodes: Host[], label: string) => { const found = nodes.find(node => node.type === 'button' && content(node) === label); assert.ok(found, `${label} tab exists`); return found; };
    await t.test('default and startup-probe capabilities stay disabled and run no automatic checks', async () => {
      const view = hookRenderer();
      try {
        policy.installExecutionPolicy(policy.OFFLINE_EXECUTION_POLICY); calls.length = 0;
        for (const provider of ['codex', 'codex_sdk']) {
          const nodes = view.render(React.createElement(TextExecutionSettings, { ...props, settings: { ...props.settings, textProvider: provider } }));
          const check = button(nodes, provider === 'codex' ? 'codex_cli' : 'codex_sdk');
          assert.equal(check.props.disabled, true); check.props.onClick();
        }
        capability = { ...capability, readiness: { ...readiness, enabled: false } };
        await api.request({ action: 'getExecutionPolicy' }); calls.length = 0;
        const nodes = view.render(React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('unexpected mutation'); } }));
        assert.equal(button(nodes, 'dreamina').props.disabled, true);
        assert.equal(button(nodes, 'dreamina_canvas').props.disabled, true); button(nodes, 'dreamina_canvas').props.onClick();
        await settle(); assert.equal(calls.length, 0);
      } finally { view.unmount(); }
    });
    await t.test('generation-off capability enables actual CLI and SDK click handlers using unsaved paths', async () => {
      capability = { ...capability, readiness }; await api.request({ action: 'getExecutionPolicy' }); calls.length = 0;
      const view = hookRenderer();
      try {
        for (const [provider, name, key, path] of [['codex', 'codex_cli', 'codexPath', '/unsaved/CLI path'], ['codex_sdk', 'codex_sdk', 'textSdkNodePath', '/unsaved/Node path']]) {
          const element = React.createElement(TextExecutionSettings, { ...props, settings: { ...props.settings, textProvider: provider, [key]: path } });
          let nodes = view.render(element);
          const check = button(nodes, name); assert.equal(check.props.disabled, false);
          const before = calls.length; await settle(); assert.equal(calls.length, before, 'mount and switch have no IPC');
          check.props.onClick(); check.props.onClick();
          assert.equal(calls.length, before + 1, 'duplicate clicks while pending are ignored');
          assert.deepEqual(calls.at(-1), { action: 'readiness.check', projectId: workspace.project.id, provider: name, settings: { [key]: path } });
          nodes = view.render(element); assert.equal(button(nodes, name).props.disabled, true);
          await settle(); nodes = view.render(element);
          assert.equal(button(nodes, name).props.disabled, false);
          const text = nodes.map(content).join(' ');
          for (const label of ['程序安装', '协议 / 环境', '账户', '模型目录', '网络', '真实模型生成', '未测试 · 本次检查生成调用 0 次']) assert.ok(text.includes(label), label);
          assert.ok(!text.includes('连接成功') && !text.includes('可执行'));
          if (name === 'codex_cli') {
            assert.ok(nodes.some(node => node.type === 'option' && node.props.value === 'synthetic-model'));
            const login = nodes.find(node => node.type === 'button' && node.props['data-execution-action'] === 'textCli.login'); assert.ok(login); assert.equal(login.props.disabled, true); login.props.onClick();
            await settle(); assert.equal(calls.length, before + 1, 'direct blocked login handler cannot save or call IPC');
          } else assert.ok(text.includes('仅证明环境与协议；真实模型未测试'));
        }
      } finally { view.unmount(); }
    });
    await t.test('the main Jimeng check targets Canvas and its OAuth belongs to the same connection', () => {
      const view = hookRenderer();
      try {
        const nodes = view.render(React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('no writes during render'); } }));
        const main = nodes.find(node => node.type === 'button' && content(node) === '检查即梦 CLI、账户与模型');
        assert.ok(main);
        assert.equal(main.props['data-readiness-provider'], 'dreamina_canvas');
        const oauth = nodes.find(node => node.type === 'button' && content(node) === 'OAuth 连接即梦');
        assert.ok(oauth);
        assert.equal(oauth.props['data-execution-action'], 'canvas.login');
        assert.ok(nodes.some(node => node.type === 'h3' && content(node) === '即梦 Canvas 账户与连接'));
        assert.ok(nodes.some(node => node.type === 'h3' && content(node) === '即梦生成 CLI'));
      } finally { view.unmount(); }
    });
    await t.test('Jimeng account and provider buttons use readiness with the current unsaved path', async () => {
      calls.length = 0; const view = hookRenderer();
      try {
        const element = React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('readiness must not call providers.refresh or dreamina.status'); } });
        let nodes = view.render(element); assert.equal(calls.length, 0);
        const input = nodes.find(node => node.type === 'input' && node.props.value === '/saved/dreamina'); assert.ok(input);
        assert.equal(input.props.placeholder, '填写已安装即梦生成 CLI 的完整路径');
        input.props.onChange({ target: { value: '/unsaved/Jimeng path' } }); nodes = view.render(element);
        const checks = nodes.filter(node => node.type === 'button' && node.props['data-readiness-provider'] === 'dreamina'); assert.equal(checks.length, 2);
        for (const check of checks) assert.equal(check.props.disabled, false);
        tab(nodes, '检查账户与模型').props.onClick(); tab(nodes, '更新模型目录').props.onClick();
        nodes = view.render(element);
        assert.ok(nodes.filter(node => node.type === 'button' && node.props['data-readiness-provider'] === 'dreamina').every(node => node.props.disabled), 'both entries share the pending request');
        await settle(); nodes = view.render(element);
        assert.deepEqual(calls, [{ action: 'readiness.check', projectId: workspace.project.id, provider: 'dreamina', settings: { dreaminaPath: '/unsaved/Jimeng path' } }]);
        tab(nodes, '更新模型目录').props.onClick(); await settle(); nodes = view.render(element);
        assert.deepEqual(calls, Array.from({ length: 2 }, () => ({ action: 'readiness.check', projectId: workspace.project.id, provider: 'dreamina', settings: { dreaminaPath: '/unsaved/Jimeng path' } })));
        assert.ok(nodes.map(content).join(' ').includes('图片免费资格尚未核验'));
        for (const action of ['dreamina.login', 'providers.refresh']) for (const node of nodes.filter(node => node.type === 'button' && node.props['data-execution-action'] === action)) assert.equal(node.props.disabled, true);
      } finally { view.unmount(); }
    });
    await t.test('advanced legacy model check uses a draft Windows executable when the saved path is empty', async () => {
      calls.length = 0; const view = hookRenderer(), initial = { ...workspace, settings: { ...workspace.settings, dreaminaPath: '' } };
      const before = structuredClone(initial), draftPath = 'D:\\Installed Apps\\即梦\\dreamina.exe';
      const element = React.createElement(SettingsPanel, { workspace: initial, writable: true, mutate: async () => { throw new Error('draft model check must not save, refresh saved providers or authorize'); } });
      try {
        let nodes = view.render(element);
        const input = nodes.find(node => node.type === 'input' && node.props.placeholder === '填写已安装即梦生成 CLI 的完整路径'); assert.ok(input);
        assert.equal(input.props.value, '');
        input.props.onChange({ target: { value: draftPath } }); nodes = view.render(element);
        assert.ok(nodes.map(content).join(' ').includes('使用当前输入路径检查账户与模型，无需先保存'));
        tab(nodes, '更新模型目录').props.onClick(); await settle(); nodes = view.render(element);
        assert.deepEqual(calls, [{ action: 'readiness.check', projectId: initial.project.id, provider: 'dreamina', settings: { dreaminaPath: draftPath } }]);
        assert.ok(nodes.some(node => node.props['data-readiness-result'] === 'dreamina'));
        assert.deepEqual(initial, before, 'saved paths, channel selections and history remain unchanged');
      } finally { view.unmount(); }
    });
    await t.test('Canvas uses only its unsaved input and never mutates legacy settings, history, providers or selectors', async () => {
      calls.length = 0; const view = hookRenderer(), before = structuredClone(workspace);
      const element = React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('read-only Canvas must not mutate'); } });
      try {
        let nodes = view.render(element); await settle(); assert.equal(calls.length, 0, 'mount has no checks');
        assert.equal(canvasInput(nodes).props.placeholder, '选择 dreamina-canvas，或填写可执行文件的完整路径');
        assert.ok(nodes.map(content).join(' ').includes('检查范围：default 配置 / cn 区域'));
        canvasInput(nodes).props.onChange({ target: { value: '/unsaved/Canvas CLI' } }); nodes = view.render(element);
        const textProvider = nodes.find(node => node.type === 'select' && node.children.some(child => typeof child !== 'string' && child.props.value === 'codex_sdk')); assert.ok(textProvider);
        textProvider.props.onChange({ target: { value: 'codex_sdk' } }); nodes = view.render(element);
        await settle(); assert.equal(calls.length, 0, 'path edit and provider switch have no checks');
        const check = button(nodes, 'dreamina_canvas'); assert.equal(check.props.disabled, false); check.props.onClick(); check.props.onClick();
        assert.deepEqual(calls, [{ action: 'readiness.check', projectId: workspace.project.id, provider: 'dreamina_canvas', settings: { dreaminaCanvasPath: '/unsaved/Canvas CLI' } }]);
        assert.equal(button(view.render(element), 'dreamina_canvas').props.disabled, true);
        await settle(); nodes = view.render(element);
        assert.equal(button(nodes, 'dreamina_canvas').props.disabled, false);
        const text = nodes.map(content).join(' ');
        for (const label of ['schema', 'auth status', 'account', 'image models', 'Synthetic Canvas Image', 'image', '积分 / 额度未知', '没有额度接口', '免费资格未核验', '尚未接入生成桥', '未测试 · 本次检查生成调用 0 次']) assert.ok(text.includes(label), label);
        assert.ok(!nodes.some(node => node.type === 'option' && ['dreamina_canvas', 'synthetic-model'].includes(node.props.value)), 'Canvas catalog never becomes a generation option');
        assert.ok(nodes.some(node => node.type === 'input' && node.props.value === '/saved/dreamina'), 'legacy path is unchanged');
        assert.equal(nodes.filter(node => node.type === 'button' && node.props['data-readiness-provider'] === 'dreamina').length, 2, 'legacy account and advanced entries share one check');
        assert.deepEqual(workspace, before, 'settings, saved account, history and capability cache are untouched');
        tab(nodes, '执行与保存').props.onClick(); nodes = view.render(element);
        assert.ok(!nodes.some(node => node.type === 'option' && ['dreamina_canvas', 'synthetic-model'].includes(node.props.value)));
        const providerSelectors = nodes.filter(node => node.type === 'select' && node.children.some(child => typeof child !== 'string' && child.props.value === 'bh'));
        assert.equal(providerSelectors.length, 2);
        assert.deepEqual(providerSelectors.map(node => node.children.filter(child => typeof child !== 'string').map(child => (child as Host).props.value)), [['dreamina', 'bh', 'chrome'], ['dreamina', 'bh']]);
        tab(nodes, '渠道').props.onClick(); nodes = view.render(element); await settle();
        assert.equal(calls.length, 1, 'tab return does not check');
        assert.equal(canvasInput(nodes).props.value, '/unsaved/Canvas CLI');
        assert.ok(!nodes.some(node => node.props['data-readiness-result'] === 'dreamina_canvas'), 'tab return cannot revive results');
      } finally { view.unmount(); }
    });
    await t.test('legacy and Canvas checks preserve separate unsaved paths and results in the same panel', async () => {
      calls.length = 0; const view = hookRenderer(), before = structuredClone(workspace);
      const element = React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('connection checks must not write settings or log in'); } });
      try {
        let nodes = view.render(element);
        const legacy = nodes.find(node => node.type === 'input' && node.props.value === '/saved/dreamina'); assert.ok(legacy);
        legacy.props.onChange({ target: { value: '/unsaved/legacy-generation' } });
        canvasInput(nodes).props.onChange({ target: { value: '/unsaved/canvas-readiness' } }); nodes = view.render(element);
        await settle(); assert.equal(calls.length, 0, 'editing both paths has no IPC');
        button(nodes, 'dreamina').props.onClick(); await settle(); nodes = view.render(element);
        button(nodes, 'dreamina_canvas').props.onClick(); await settle(); nodes = view.render(element);
        assert.deepEqual(calls, [
          { action: 'readiness.check', projectId: workspace.project.id, provider: 'dreamina', settings: { dreaminaPath: '/unsaved/legacy-generation' } },
          { action: 'readiness.check', projectId: workspace.project.id, provider: 'dreamina_canvas', settings: { dreaminaCanvasPath: '/unsaved/canvas-readiness' } },
        ]);
        for (const provider of ['dreamina', 'dreamina_canvas']) {
          assert.equal(nodes.filter(node => node.type === 'button' && node.props['data-readiness-provider'] === provider).length, provider === 'dreamina' ? 2 : 1);
          assert.ok(nodes.some(node => node.props['data-readiness-result'] === provider), `${provider} retains its own result`);
        }
        assert.deepEqual(workspace, before, 'saved paths, accounts and generation history are unchanged');
      } finally { view.unmount(); }
    });
    await t.test('legacy OAuth renewal keeps its execution action and never dispatches Canvas authorization', async () => {
      const writes: PolicyRequest[] = [], view = hookRenderer();
      // Exercise only synthetic component callbacks; no provider or account is contacted.
      policy.installExecutionPolicy(policy.validateExecutionPolicy({ ...capability, version: 'execution-policy/v3', mode: 'standard_native', liveExecutionAllowed: true, allowedActions: ['dreamina.login', 'dreamina.open_authorization'] }));
      try {
        const connected = { ...workspace, dreaminaAccount: { status: 'authenticated', identityVerified: true, ready: true, credits: { total: 0 } } };
        const mutate = async (action: string, values: Record<string, unknown> = {}) => { writes.push({ action, ...values }); return null; };
        let nodes = view.render(React.createElement(DreaminaAccountCard, { workspace: connected, writable: true, mutate }));
        const renewal = tab(nodes, '重新登录');
        assert.equal(renewal.props['data-execution-action'], 'dreamina.login'); assert.equal(renewal.props.disabled, false);
        renewal.props.onClick(); await settle(); assert.deepEqual(writes, [{ action: 'dreamina.login' }]);
        nodes = view.render(React.createElement(DreaminaAccountCard, { workspace: { ...connected, dreaminaAccount: { status: 'authorizing', identityVerified: false, ready: false } }, writable: true, mutate }));
        const open = tab(nodes, '打开授权页'); assert.equal(open.props['data-execution-action'], 'dreamina.open_authorization');
        open.props.onClick(); await settle(); assert.deepEqual(writes, [{ action: 'dreamina.login' }, { action: 'dreamina.open_authorization' }]);
      } finally { view.unmount(); policy.installExecutionPolicy(policy.validateExecutionPolicy(capability)); }
    });
    await t.test('Canvas native picker handles repeated clicks, cancel, errors and successful selection without automatic checks', async () => {
      calls.length = 0; const view = hookRenderer();
      const element = React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('picker must not save'); } });
      let resolvePick: (paths: string[]) => void = () => {};
      try {
        pick = () => new Promise(resolve => { resolvePick = resolve; });
        let nodes = view.render(element); const chooseButton = picker(nodes); chooseButton.props.onClick(); chooseButton.props.onClick();
        assert.deepEqual(calls, [{ action: 'files.choose', kind: 'file', multiple: false }]);
        nodes = view.render(element); assert.equal(picker(nodes).props.disabled, true);
        resolvePick([]); await settle(); nodes = view.render(element);
        assert.equal(canvasInput(nodes).props.value, '/saved/canvas'); assert.equal(picker(nodes).props.disabled, false);
        pick = async () => { throw new Error('synthetic picker failure'); }; picker(nodes).props.onClick(); await settle(); nodes = view.render(element);
        assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('synthetic picker failure')));
        assert.equal(canvasInput(nodes).props.value, '/saved/canvas'); assert.equal(picker(nodes).props.disabled, false);
        pick = async () => ['/chosen/Canvas CLI']; picker(nodes).props.onClick(); await settle(); nodes = view.render(element);
        assert.equal(canvasInput(nodes).props.value, '/chosen/Canvas CLI');
        assert.ok(!nodes.some(node => node.props.role === 'alert'));
        assert.ok(calls.every(call => call.action === 'files.choose'), 'picker completion never checks or saves');
        button(nodes, 'dreamina_canvas').props.onClick(); await settle(); view.render(element);
        assert.deepEqual(calls.at(-1), { action: 'readiness.check', projectId: workspace.project.id, provider: 'dreamina_canvas', settings: { dreaminaCanvasPath: '/chosen/Canvas CLI' } });
      } finally { view.unmount(); pick = async () => []; }
    });
    await t.test('Canvas stale native picker cannot overwrite edits or a newer project, tab or unmounted screen', async () => {
      let resolvePick: (paths: string[]) => void = () => {};
      pick = () => new Promise(resolve => { resolvePick = resolve; });
      try {
        for (const change of ['path', 'project', 'tab', 'unmount']) {
          calls.length = 0; const view = hookRenderer();
          const props = { workspace, writable: true, mutate: async () => { throw new Error('picker must not mutate'); } };
          let element: React.ReactElement = React.createElement(SettingsPanel, props);
          try {
            let nodes = view.render(element); picker(nodes).props.onClick();
            if (change === 'path') canvasInput(nodes).props.onChange({ target: { value: '/newer/edit' } });
            if (change === 'project') element = React.createElement(SettingsPanel, { ...props, workspace: { ...workspace, project: { id: 'newer-project' }, settings: { ...workspace.settings, dreaminaCanvasPath: '/newer/project' } } });
            if (change === 'tab') tab(nodes, '执行与保存').props.onClick();
            if (change === 'unmount') view.unmount(); else nodes = view.render(element);
            resolvePick(['/stale/picker']); await settle(); nodes = view.render(element);
            if (change === 'tab') { tab(nodes, '渠道').props.onClick(); nodes = view.render(element); }
            assert.equal(canvasInput(nodes).props.value, change === 'path' ? '/newer/edit' : change === 'project' ? '/newer/project' : '/saved/canvas');
            assert.equal(picker(nodes).props.disabled, false); assert.equal(calls.length, 1, 'stale completion makes no new IPC');
          } finally { view.unmount(); }
        }
      } finally { pick = async () => []; }
    });
    await t.test('Canvas pending checks ignore old path, project, tab, and read-only mode responses and permit a newer check', async () => {
      const pending: ((value: unknown) => void)[] = [];
      respond = () => new Promise(resolve => { pending.push(resolve); });
      try {
        for (const change of ['path', 'project', 'tab', 'readonly']) {
          calls.length = 0; const view = hookRenderer();
          const props = { workspace, writable: true, mutate: async () => { throw new Error('check must not mutate'); } };
          let element: React.ReactElement = React.createElement(SettingsPanel, props);
          try {
            let nodes = view.render(element); button(nodes, 'dreamina_canvas').props.onClick();
            if (change === 'path') canvasInput(nodes).props.onChange({ target: { value: '/newer/check/path' } });
            if (change === 'project') element = React.createElement(SettingsPanel, { ...props, workspace: { ...workspace, project: { id: 'newer-project' } } });
            if (change === 'tab') tab(nodes, '执行与保存').props.onClick();
            if (change === 'readonly') element = React.createElement(SettingsPanel, { ...props, writable: false });
            nodes = view.render(element);
            if (change === 'tab') { tab(nodes, '渠道').props.onClick(); nodes = view.render(element); }
            if (change === 'readonly') {
              assert.equal(button(nodes, 'dreamina_canvas').props.disabled, true);
              button(nodes, 'dreamina_canvas').props.onClick(); assert.equal(calls.length, 1);
              element = React.createElement(SettingsPanel, props); nodes = view.render(element);
            }
            assert.equal(calls.length, 1, 'navigation and edits do not start a check');
            button(nodes, 'dreamina_canvas').props.onClick(); assert.equal(calls.length, 2);
            const old = pending.shift()!, latest = pending.shift()!;
            old({ ...result('dreamina_canvas'), messages: ['stale Canvas response'] }); await settle(); nodes = view.render(element);
            assert.ok(!nodes.map(content).join(' ').includes('stale Canvas response'));
            assert.equal(button(nodes, 'dreamina_canvas').props.disabled, true, 'old response cannot clear newer busy state');
            latest({ ...result('dreamina_canvas'), messages: ['current Canvas response'] }); await settle(); nodes = view.render(element);
            assert.ok(nodes.map(content).join(' ').includes('current Canvas response'));
            assert.equal(button(nodes, 'dreamina_canvas').props.disabled, false);
          } finally { view.unmount(); }
        }
      } finally { respond = async input => result(input.provider as ProviderReadinessReport['provider']); }
    });
    await t.test('Canvas unsupported protocol and partial catalogs remain truthful, and mismatches permit explicit retry', async () => {
      calls.length = 0; const view = hookRenderer();
      const element = React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('must not mutate'); } });
      try {
        const unsupported = { ...result('dreamina_canvas'), protocol: { state: 'unsupported', name: 'unrecognized/schema' }, authentication: { state: 'not_checked', identityVerified: false }, catalog: { state: 'not_checked', models: [] } };
        assert.equal(validateProviderReadiness(unsupported, 'dreamina_canvas'), unsupported);
        respond = async () => unsupported; button(view.render(element), 'dreamina_canvas').props.onClick(); await settle();
        let nodes = view.render(element); assert.ok(nodes.map(content).join(' ').includes('协议不受支持')); assert.equal(button(nodes, 'dreamina_canvas').props.disabled, false);
        for (const state of ['available', 'unavailable', 'not_checked']) assert.doesNotThrow(() => validateProviderReadiness({ ...result('dreamina_canvas'), catalog: { state, models: [] } }, 'dreamina_canvas'));
        respond = async () => result('dreamina'); button(nodes, 'dreamina_canvas').props.onClick(); await settle(); nodes = view.render(element);
        assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('只读检查结果无效')));
        respond = async () => result('dreamina_canvas'); button(nodes, 'dreamina_canvas').props.onClick(); await settle(); nodes = view.render(element);
        assert.ok(!nodes.some(node => node.props.role === 'alert')); assert.equal(calls.length, 3);
        for (const invalid of [
          { ...result('dreamina_canvas'), protocol: { state: 'compatible', name: null } },
          { ...result('dreamina_canvas'), catalog: { state: 'available', models: [{ name: 'no id' }] } },
          { ...result('dreamina_canvas'), generation: { enabled: true, testPerformed: false, calls: 0 } },
        ]) assert.throws(() => validateProviderReadiness(invalid, 'dreamina_canvas'));
      } finally { view.unmount(); respond = async input => result(input.provider as ProviderReadinessReport['provider']); }
    });
    await t.test('Canvas catalog shows canonical IDs and optional model modes without inferring missing capabilities', async () => {
      calls.length = 0; const view = hookRenderer();
      const element = React.createElement(SettingsPanel, { workspace, writable: true, mutate: async () => { throw new Error('catalog is display only'); } });
      const report = { ...result('dreamina_canvas', 'canonical-image-id'), details: { accountQuery: { state: 'unavailable' }, quota: { state: 'unsupported', remaining: null }, freeImageEligibility: 'unverified', modelSpecifications: [
        { model: 'canonical-image-id', modes: [{ name: 'synthetic-text-to-image', flags: [{ flag: '--ratio', values: ['1:1', '3:2'] }] }], vipRequired: false },
        { model: 'unreported-modes' }, { model: 'empty-modes', modes: [] },
        { model: 'bad-modes', modes: [null, 'ignored', { name: 42 }] }, null, { wrong: 'ignored' },
      ] } };
      const before = structuredClone(report);
      try {
        respond = async () => report; button(view.render(element), 'dreamina_canvas').props.onClick(); await settle();
        const nodes = view.render(element), text = nodes.map(content).join(' ');
        for (const label of ['canonical-image-id', 'Synthetic Canvas Image', 'synthetic-text-to-image', '模式未提供', '无模式记录', '缺少的能力仍未知', 'Canvas account 查询', '读取未完成', '已登录', '积分 / 额度未知']) assert.ok(text.includes(label), label);
        assert.deepEqual(report, before, 'rendering does not annotate or mutate backend evidence');
        assert.ok(!nodes.some(node => node.type === 'option' && node.props.value === 'canonical-image-id'));
        assert.equal(calls.length, 1);
        respond = async () => ({ ...report, details: { ...report.details, accountQuery: { state: 'unsupported' } } });
        button(nodes, 'dreamina_canvas').props.onClick(); await settle();
        assert.ok(view.render(element).map(content).join(' ').includes('当前协议未提供只读账户查询'));
        assert.equal(calls.length, 2);
      } finally { view.unmount(); respond = async input => result(input.provider as ProviderReadinessReport['provider']); }
    });
    await t.test('Canvas supervisor diagnostics are collapsed historical observations and never issue IPC', () => {
      calls.length = 0; const view = hookRenderer();
      const supervisor = {
        phase: 'thread_joined', pid: 1234, elapsedMs: 805, budgetMs: 800,
        milestones: { created: 0, spawn_end: 2, exit_observed: 804, cleanup_end: 804, thread_joined: 805 },
        stdout: { bytes: 12, eof: true, readError: false, firstByteMs: 10, eofMs: 30 },
        stderr: { bytes: 0, eof: false, readError: false, firstByteMs: null, eofMs: null },
        processExit: { code: 0, signal: null, success: true, observedAtMs: 804 },
        timeout: { kind: 'run', elapsedMs: 804, overshootMs: 4, exitObserved: false },
        preCleanup: { state: 'running', exitCode: null, signal: null },
        cleanup: { killRequested: true, reapConfirmed: true }, controlTicks: 40, maxControlGapMs: 21,
      };
      const report = { ...result('dreamina_canvas'), details: { processDiagnostics: Object.fromEntries(['version', 'schema', 'auth status', 'auth account', 'model list --type image'].map(command => [command, { supervisor }])) } };
      const before = structuredClone(report);
      try {
        const nodes = view.render(React.createElement(ProviderReadinessResult, { check: { result: report, error: '', busy: false, blocked: null, provider: 'dreamina_canvas' } }));
        const panel = nodes.find(node => node.type === 'details' && node.props['data-canvas-process-diagnostics']);
        assert.ok(panel); assert.ok(!panel.props.open, 'diagnostics start collapsed');
        const text = content(panel);
        for (const label of ['version', 'schema', 'auth status', 'auth account', 'model list --type image', '1234', '最后记录阶段thread_joined', 'created0 ms', 'spawn_end2 ms', 'thread_joined805 ms', '805 ms / 800 ms', '40 / 21 ms', '字节：12', 'EOF：是', '读取错误：否', '首字节：10 ms', 'EOF 时间：30 ms', '退出码：0', '成功：是', '类型：run', '超出：4 ms', '已观察退出：否', '状态：running', '已请求终止：是', '已确认回收：是', 'PID 不表示进程当前仍在运行', '不能单独确定故障原因或证明问题已修复']) assert.ok(text.includes(label), label);
        assert.equal(nodes.filter(node => node.props['data-canvas-command']).length, 5);
        assert.ok(!nodes.some(node => node.type === 'button'), 'diagnostics have no probe or retry controls');
        assert.deepEqual(report, before, 'diagnostics do not change evidence');
        assert.deepEqual(calls, []);
      } finally { view.unmount(); }
    });
    await t.test('Canvas diagnostics keep missing and malformed fields unknown without claiming exit or cleanup', () => {
      calls.length = 0; const view = hookRenderer();
      try {
        for (const supervisor of [undefined, null, [], {}, { pid: 0, phase: 'future-phase', elapsedMs: -1, budgetMs: Infinity, stdout: { bytes: '12', eof: 'true' }, cleanup: { reapConfirmed: null } }]) {
          const report = { ...result('dreamina_canvas'), details: { processDiagnostics: { version: { supervisor } } } };
          const nodes = view.render(React.createElement(ProviderReadinessResult, { check: { result: report, error: '', busy: false, provider: 'dreamina_canvas' } }));
          const panel = nodes.find(node => node.props['data-canvas-command'] === 'version'); assert.ok(panel);
          const text = content(panel);
          for (const label of ['检查时 PID（历史）未知', '最后记录阶段未知', '已用时间 / 预算未知 / 未知', '字节：未知', 'EOF：未知', '未知 / 无退出记录', '已请求终止：未知', '已确认回收：未知']) assert.ok(text.includes(label), label);
          assert.ok(!text.includes('future-phase'));
        }
        for (const details of [{}, { processDiagnostics: null }, { processDiagnostics: [] }]) {
          const nodes = view.render(React.createElement(ProviderReadinessResult, { check: { result: { ...result('dreamina_canvas'), details }, error: '', busy: false } }));
          assert.ok(nodes.map(content).join(' ').includes('进程阶段诊断未知'));
        }
        assert.deepEqual(calls, []);
      } finally { view.unmount(); }
    });
    await t.test('Canvas telemetry rejects arbitrary keys, private strings and non-Canvas providers', () => {
      const view = hookRenderer(), secret = '/private/synthetic-token-environment-and-output';
      const injected = { phase: secret, pid: secret, elapsedMs: secret, budgetMs: secret, milestones: { [secret]: 1, created: secret }, stdout: { bytes: secret, eof: secret, readError: secret, firstByteMs: secret, eofMs: secret, raw: secret }, stderr: secret, processExit: { code: secret, signal: secret, success: secret, observedAtMs: secret }, timeout: { kind: secret, elapsedMs: secret, overshootMs: secret, exitObserved: secret }, preCleanup: { state: secret, exitCode: secret, signal: secret }, cleanup: { killRequested: secret, reapConfirmed: secret }, controlTicks: secret, maxControlGapMs: secret, environment: secret };
      try {
        const diagnostics = { [secret]: { supervisor: injected }, version: { supervisor: injected, rawOutput: secret } };
        const nodes = view.render(React.createElement(ProviderReadinessResult, { check: { result: { ...result('dreamina_canvas'), details: { processDiagnostics: diagnostics } }, error: '', busy: false } }));
        assert.ok(!nodes.map(content).join(' ').includes(secret));
        assert.ok(!JSON.stringify(nodes.map(node => ({ type: node.type, label: node.props['aria-label'], command: node.props['data-canvas-command'] }))).includes(secret));
        assert.equal(nodes.filter(node => node.props['data-canvas-command']).length, 1, 'unknown command keys are never rendered');
        const other = view.render(React.createElement(ProviderReadinessResult, { check: { result: { ...result('dreamina'), details: { processDiagnostics: diagnostics } }, error: '', busy: false } }));
        assert.ok(!other.some(node => node.props['data-canvas-process-diagnostics']), 'display is Canvas only');
      } finally { view.unmount(); }
    });
    await t.test('Canvas path is saved only on explicit save, preserving legacy and generation settings', async () => {
      calls.length = 0; const view = hookRenderer(), writes: { action: string; settings: Record<string, unknown> }[] = [];
      const element = React.createElement(SettingsPanel, { workspace, writable: true, mutate: async (action: string, values: { settings: Record<string, unknown> }) => { writes.push({ action, settings: values.settings }); return { ...workspace, settings: values.settings }; } });
      try {
        let nodes = view.render(element); canvasInput(nodes).props.onChange({ target: { value: '/saved-after-click/canvas' } }); nodes = view.render(element);
        button(nodes, 'dreamina_canvas').props.onClick(); await settle(); nodes = view.render(element); assert.deepEqual(writes, []);
        const form = nodes.find(node => node.type === 'form'); assert.ok(form); form.props.onSubmit({ preventDefault() {} }); await settle(); nodes = view.render(element);
        assert.deepEqual(writes, [{ action: 'settings.save', settings: { ...workspace.settings, dreaminaCanvasPath: '/saved-after-click/canvas' } }]);
        assert.equal(canvasInput(nodes).props.value, '/saved-after-click/canvas');
        assert.equal(calls.length, 1, 'saving does not run another check');
      } finally { view.unmount(); }
    });
    await t.test('path, provider and project changes discard pending responses and clear busy', async () => {
      const view = hookRenderer(); let resolve: (value: unknown) => void = () => {};
      respond = () => new Promise(done => { resolve = done; }); calls.length = 0;
      try {
        const initial = React.createElement(TextExecutionSettings, props);
        for (const changed of [
          React.createElement(TextExecutionSettings, { ...props, settings: { ...props.settings, codexPath: '/new/path' } }),
          React.createElement(TextExecutionSettings, { ...props, settings: { textProvider: 'codex_sdk', textSdkNodePath: '/new/node' } }),
          React.createElement(TextExecutionSettings, { ...props, workspace: { ...workspace, project: { id: 'other-project' } } }),
        ]) {
          let nodes = view.render(initial); button(nodes, 'codex_cli').props.onClick();
          nodes = view.render(changed); resolve(result('codex_cli', 'stale-model')); await settle(); nodes = view.render(changed);
          assert.ok(!nodes.map(content).join(' ').includes('stale-model'));
          assert.ok(nodes.filter(node => node.props['data-readiness-provider']).every(node => node.props.disabled === false));
          nodes = view.render(initial); assert.ok(!nodes.map(content).join(' ').includes('stale-model'), 'returning to an earlier path cannot revive results');
        }
        const nodes = view.render(initial); button(nodes, 'codex_cli').props.onClick(); view.unmount(); resolve(result('codex_cli', 'after-unmount')); await settle();
      } finally { view.unmount(); respond = async input => result(input.provider as ProviderReadinessReport['provider']); }
    });
    await t.test('failed and mismatched checks clear busy and allow an explicit retry', async () => {
      const view = hookRenderer(); calls.length = 0;
      const element = React.createElement(TextExecutionSettings, props);
      try {
        respond = async () => { throw new Error('synthetic read-only failure'); };
        button(view.render(element), 'codex_cli').props.onClick(); await settle();
        let nodes = view.render(element);
        assert.equal(button(nodes, 'codex_cli').props.disabled, false);
        assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('synthetic read-only failure')));
        respond = async () => result('dreamina');
        button(nodes, 'codex_cli').props.onClick(); await settle(); nodes = view.render(element);
        assert.equal(button(nodes, 'codex_cli').props.disabled, false);
        assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('只读检查结果无效')));
        respond = async input => result(input.provider as ProviderReadinessReport['provider']);
        button(nodes, 'codex_cli').props.onClick(); await settle(); nodes = view.render(element);
        assert.ok(!nodes.some(node => node.props.role === 'alert'));
        assert.equal(calls.length, 3);
      } finally { view.unmount(); respond = async input => result(input.provider as ProviderReadinessReport['provider']); }
    });
    await t.test('Canvas OAuth polls authorization, refreshes the main result and confirms force explicitly', async () => {
      const { DreaminaCanvasReadiness } = await server.ssrLoadModule('/src/DreaminaCanvasReadiness.tsx');
      policy.installExecutionPolicy(policy.validateExecutionPolicy({ ...capability, canvasOAuth: { version: 'canvas-oauth/v1', enabled: true } }));
      const view = hookRenderer();
      const element = React.createElement(DreaminaCanvasReadiness, { projectId: 'oauth-project', path: '/synthetic/canvas', writable: true, update() {} });
      let state = 'idle';
      respond = async input => {
        if (input.action === 'readiness.check') return result('dreamina_canvas');
        if (input.action === 'canvas.login') {
          state = input.force ? 'failed' : 'authorizing';
          return { state, userCode: 'PUBLIC_SYNTHETIC', message: input.force ? 'synthetic denied' : '请在官方网页确认' };
        }
        if (input.action === 'canvas.cancelLogin') return { state: 'cancelled' };
        return { state };
      };
      try {
        view.render(element); await settle(); view.render(element); await settle();
        let nodes = view.render(element);
        assert.ok(nodes.map(content).join(' ').includes('已连接即梦'));
        calls.length = 0;
        tab(nodes, 'OAuth 连接即梦').props.onClick(); await settle(); nodes = view.render(element);
        assert.equal(calls[0].action, 'canvas.login'); assert.equal(calls[0].force, undefined);
        assert.ok(nodes.map(content).join(' ').includes('PUBLIC_SYNTHETIC'));
        assert.ok(!nodes.map(content).join(' ').includes('已连接即梦'), 'old success must be cleared during authorization');
        assert.equal(button(nodes, 'dreamina_canvas').props.disabled, true);
        state = 'authenticated';
        await new Promise(resolve => setTimeout(resolve, 1100)); await settle(); view.render(element); await settle(); nodes = view.render(element);
        assert.ok(calls.some(call => call.action === 'canvas.loginStatus'));
        assert.ok(calls.some(call => call.action === 'readiness.check' && call.provider === 'dreamina_canvas'));
        assert.ok(nodes.map(content).join(' ').includes('已连接即梦'));
        const before = calls.length;
        tab(nodes, '重新登录').props.onClick(); nodes = view.render(element);
        assert.equal(calls.length, before, 'force must not run before the explicit in-app confirmation');
        tab(nodes, '确认重新授权').props.onClick(); await settle(); nodes = view.render(element);
        assert.equal(calls.at(-1)?.force, true); assert.equal(calls.at(-1)?.confirmedForce, true);
        assert.ok(!nodes.map(content).join(' ').includes('已连接即梦'), 'failed renewal must not retain stale success');
        assert.ok(nodes.map(content).join(' ').includes('synthetic denied'));
        tab(nodes, 'OAuth 连接即梦').props.onClick(); await settle(); nodes = view.render(element);
        tab(nodes, '取消等待').props.onClick(); await settle(); nodes = view.render(element);
        assert.equal(calls.at(-1)?.action, 'canvas.cancelLogin');
        assert.equal(button(nodes, 'dreamina_canvas').props.disabled, false);
      } finally {
        view.unmount(); policy.installExecutionPolicy(policy.validateExecutionPolicy(capability));
        respond = async input => result(input.provider as ProviderReadinessReport['provider']);
      }
    });
    await t.test('Canvas initial and local-only login records never display a verified login or connection', async () => {
      const { DreaminaCanvasReadiness } = await server.ssrLoadModule('/src/DreaminaCanvasReadiness.tsx');
      policy.installExecutionPolicy(policy.validateExecutionPolicy({ ...capability, canvasOAuth: { version: 'canvas-oauth/v1', enabled: true } }));
      const view = hookRenderer();
      const element = React.createElement(DreaminaCanvasReadiness, { projectId: 'unverified-account-project', path: '/synthetic/canvas', writable: true, update() {} });
      const report = { ...result('dreamina_canvas'), authentication: { state: 'authenticated', identityVerified: false }, catalog: { state: 'unavailable', models: [] }, details: { accountQuery: { state: 'unavailable' }, checks: { 'auth account': { state: 'failed', code: 'CANVAS_OAUTH_SCOPE_INSUFFICIENT' } } } };
      const before = structuredClone(report);
      respond = async input => input.action === 'canvas.loginStatus' ? { state: 'idle' } : report;
      try {
        let nodes = view.render(element), text = nodes.map(content).join(' ');
        assert.ok(text.includes('尚未核验连接'));
        assert.ok(!text.includes('已登录') && !text.includes('已连接即梦'), 'initial render has no successful auth evidence');
        await settle(); view.render(element); await settle(); nodes = view.render(element); text = nodes.map(content).join(' ');
        assert.ok(text.includes('本机有登录记录 · 身份待核验'));
        assert.ok(text.includes('本机有登录记录 · 身份未核验'), 'expanded details use the same evidence boundary');
        assert.ok(text.includes('尚未连接，请检查或重新授权'));
        assert.ok(!text.includes('已登录') && !text.includes('已连接即梦'), 'local auth status cannot establish verified login');
        assert.equal(tab(nodes, '退出登录').props.disabled, false, 'an existing local session still supports explicit logout');
        assert.deepEqual(report, before, 'UI labeling preserves the original account evidence');
      } finally { view.unmount(); policy.installExecutionPolicy(policy.validateExecutionPolicy(capability)); respond = async input => result(input.provider as ProviderReadinessReport['provider']); }
    });
    await t.test('Canvas account panel shows verified membership, unknown credits and explicit logout confirmation', async () => {
      const { DreaminaCanvasReadiness } = await server.ssrLoadModule('/src/DreaminaCanvasReadiness.tsx');
      policy.installExecutionPolicy(policy.validateExecutionPolicy({ ...capability, canvasOAuth: { version: 'canvas-oauth/v1', enabled: true } }));
      const view = hookRenderer();
      const element = React.createElement(DreaminaCanvasReadiness, { projectId: 'account-project', path: '/synthetic/canvas', writable: true, update() {} });
      let loggedOut = false;
      respond = async input => {
        if (input.action === 'canvas.loginStatus') return { state: 'idle' };
        if (input.action === 'canvas.logout') { loggedOut = true; return { state: 'logged_out', message: '已退出登录' }; }
        const report = result('dreamina_canvas');
        report.details = { accountQuery: { state: 'available', isVip: true, vipLevel: 'maestro' } };
        if (loggedOut) { report.authentication = { state: 'not_logged_in', identityVerified: false }; report.catalog = { state: 'not_checked', models: [] }; report.details = {}; }
        return report;
      };
      try {
        view.render(element); await settle(); view.render(element); await settle(); let nodes = view.render(element);
        assert.ok(nodes.map(content).join(' ').includes('会员（maestro）'));
        assert.ok(nodes.map(content).join(' ').includes('已登录 · 身份已核验'));
        assert.ok(nodes.map(content).join(' ').includes('已连接即梦'));
        assert.ok(nodes.map(content).join(' ').includes('账户额度'));
        assert.ok(nodes.map(content).join(' ').includes('当前接口未提供'));
        assert.ok(nodes.some(node => node.type === 'details' && node.props['data-connection-diagnostics'] && !node.props.open));
        calls.length = 0;
        tab(nodes, '退出登录').props.onClick(); nodes = view.render(element);
        assert.equal(calls.length, 0);
        tab(nodes, '确认退出登录').props.onClick(); await settle(); view.render(element); await settle(); nodes = view.render(element);
        assert.equal(calls[0].action, 'canvas.logout'); assert.equal(calls[0].confirmedLogout, true);
        assert.ok(!nodes.map(content).join(' ').includes('会员（maestro）'));
        assert.ok(nodes.map(content).join(' ').includes('未登录'));
        assert.ok(!nodes.map(content).join(' ').includes('已连接即梦'));
      } finally { view.unmount(); policy.installExecutionPolicy(policy.validateExecutionPolicy(capability)); respond = async input => result(input.provider as ProviderReadinessReport['provider']); }
    });
    await t.test('invalid readiness and model-generation claims are rejected; other live actions remain closed', async () => {
      for (const invalid of [null, {}, { ...result('codex_cli'), provider: 'dreamina' }, { ...result('codex_cli'), schemaVersion: 'other' }, { ...result('codex_cli'), generation: { enabled: true, testPerformed: false, calls: 0 } }, { ...result('codex_cli'), generation: { enabled: false, testPerformed: true, calls: 1 } }]) assert.throws(() => validateProviderReadiness(invalid, 'codex_cli'));
      calls.length = 0;
      for (const action of ['textCli.status', 'textCli.login', 'dreamina.status', 'dreamina.login', 'dreamina.logout', 'providers.refresh', 'foundation.run', 'images.run', 'videos.submit']) await assert.rejects(api.request({ action }));
      await assert.rejects(api.request({ action: 'settings.save', settings: { textApiKey: 'synthetic' } }));
      assert.equal(calls.length, 0);
    });
  } finally { await server.close(); }
});
