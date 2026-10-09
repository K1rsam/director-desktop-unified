import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';

// Render actual handlers and effects against synthetic state and IPC. No account
// or native executable is touched, including while exercising asynchronous saves.
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
const input = (nodes: Host[], name: string) => { const item = nodes.find(node => node.type === 'input' && node.props['aria-label'] === name); assert.ok(item, name); return item; };
const check = (nodes: Host[], id: string) => { const item = nodes.find(node => node.type === 'button' && node.props['data-provider-check'] === id); assert.ok(item, id); return item; };
function runtimeFixture(provider: string, installed = true, path = '/synthetic/dreamina') {
  const ids = provider === 'codex_sdk' ? ['codex', 'node'] : [provider === 'codex_cli' ? 'codex' : 'dreamina'];
  const key: Record<string, string> = { codex: 'codexPath', node: 'textSdkNodePath', dreamina: 'dreaminaPath' };
  return { schemaVersion: 'runtime-status/v1', provider, installed, installSupported: true, checkedAt: '2026-10-09T00:00:00Z', settings: installed ? Object.fromEntries(ids.map(id => [key[id], id === 'dreamina' ? path : '/synthetic/' + id])) : {}, components: ids.map(id => ({ id, name: id, state: installed ? 'installed' : 'missing', ...(installed ? { version: 'synthetic', path: id === 'dreamina' ? path : '/synthetic/' + id } : {}) })) };
}
function readinessFixture(provider: string) {
  return { schemaVersion: 'provider-readiness/v1', provider, checkedAt: '2026-10-09T00:00:00Z', installation: { state: 'ready', version: 'synthetic', executable: '/synthetic/codex' }, protocol: { state: 'ready', name: 'synthetic' }, authentication: { state: 'authenticated', identityVerified: true }, catalog: { state: 'available', models: [{ id: 'synthetic-model' }] }, network: { state: 'not_tested', proof: [] }, generation: { enabled: false, testPerformed: false, calls: 0 }, messages: [], details: {} };
}

test('channel cards keep one connection flow and save current input before checking', async t => {
  let respond = async (_request: any): Promise<unknown> => { throw new Error('unexpected synthetic IPC'); };
  let runtimeRespond = async (input: any): Promise<unknown> => runtimeFixture(input.provider);
  const chromeReport = { schemaVersion: 'chrome-setup/v1', installed: true, configured: true, status: 'ready', message: '网页已连接', clients: 1, usableClients: 1, componentVersion: 'synthetic' };
  let chromeRespond = async (_input: any): Promise<unknown> => chromeReport;
  t.beforeEach(() => { runtimeRespond = async input => runtimeFixture(input.provider); chromeRespond = async () => chromeReport; });
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, window: { __TAURI_INTERNALS__: { invoke: async (_command: string, args: { request: any }) => args.request.action === 'runtime.status' ? runtimeRespond(args.request) : args.request.action.startsWith('chrome.') ? chromeRespond(args.request) : respond(args.request) } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  try {
    const policy = await server.ssrLoadModule('/src/executionPolicy.ts');
    const { SettingsPanel, ProjectSettingsPanel } = await server.ssrLoadModule('/src/Panels.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    policy.installExecutionPolicy({ mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'inactive', reasons: [], allowedActions: ['files.choose', 'runtime.status', 'chrome.status', 'chrome.check', 'chrome.connect', 'chrome.open', 'settings.global.save', 'providers.refresh', 'textCli.login', 'dreamina.login', 'dreamina.status', 'readiness.check', 'runtime.prepare'] });
    const makeSettings = () => ({ schemaVersion: 'app-settings/v1', revision: 0, providers: structuredClone(previewWorkspace.providers), rules: [], settings: { textProvider: 'api', bhEndpoint: 'https://old.example.test', bhAccountId: 'stable-bh', chromeAccountId: 'stable-chrome', codexPath: '/synthetic/codex', textSdkNodePath: '/synthetic/node', dreaminaPath: '/synthetic/dreamina' } });

    await t.test('one card for each channel, with no Canvas or editable program paths', async () => {
      const view = hookRenderer();
      try {
        const nodes = view.render(React.createElement(SettingsPanel, { appSettings: makeSettings(), writable: true, mutate: async () => { throw new Error('opening settings must not mutate'); } }));
        for (const name of ['即梦生成 CLI', 'BH API', 'Chrome 网页']) assert.equal(nodes.filter(node => node.type === 'section' && node.props['aria-label'] === name).length, 1, name);
        assert.equal(nodes.filter(node => node.type === 'button' && node.props['data-readiness-provider'] === 'dreamina').length, 1);
        assert.ok(!nodes.some(node => node.props['data-readiness-provider'] === 'dreamina_canvas' || node.props['data-canvas-path']));
        assert.ok(!nodes.some(node => node.type === 'input' && String(node.props.value || '').startsWith('/synthetic/')));
        assert.ok(!nodes.some(node => node.type === 'summary' && content(node) === '即梦生成 CLI 高级设置'));
      } finally { view.unmount(); }
    });

    for (const id of ['bh', 'chrome']) await t.test(`${id} saves typed configuration and preserves the stable account identity before checking`, async () => {
      const view = hookRenderer(), appSettings = makeSettings(), writes: any[] = [];
      const title = id === 'bh' ? 'BH API' : 'Chrome 网页';
      const mutate = async (action: string, values: any) => { writes.push({ action, ...values }); return { ...appSettings, settings: values.settings || appSettings.settings }; };
      const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        const nodes = view.render(element);
        input(nodes, `${title} 服务地址`).props.onChange({ target: { value: 'https://new.example.test' } });
        input(nodes, `${title} 凭据`).props.onChange({ target: { value: 'synthetic-token' } });
        check(view.render(element), id).props.onClick(); await settle();
        assert.deepEqual(writes.map(item => item.action), ['settings.global.save', 'providers.refresh']);
        assert.equal(writes[0].settings[`${id}Endpoint`], 'https://new.example.test');
        assert.equal(writes[0].settings[`${id}Token`], 'synthetic-token');
        assert.equal(writes[0].settings[`${id}AccountId`], `stable-${id}`);
        assert.equal(writes[1].provider, id);
        assert.equal(input(view.render(element), `${title} 凭据`).props.value, '');
      } finally { view.unmount(); }
    });

    await t.test('a failed save never checks stale stored configuration', async () => {
      const view = hookRenderer(), writes: string[] = [];
      try {
        const element = React.createElement(SettingsPanel, { appSettings: makeSettings(), writable: true, mutate: async (action: string) => { writes.push(action); return null; } });
        check(view.render(element), 'bh').props.onClick(); await settle();
        assert.deepEqual(writes, ['settings.global.save']);
      } finally { view.unmount(); }
    });

    await t.test('Chrome connection adopts discovered identity and preserves other channel drafts', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), calls: any[] = [];
      chromeRespond = async input => { calls.push(input); return chromeReport; };
      const mutate = async (action: string, values: any) => { calls.push({ action, ...values }); return { ...appSettings, revision: 1, settings: { ...appSettings.settings, chromeEndpoint: 'http://127.0.0.1:3010', chromeAccountId: 'discovered-chrome' } }; };
      const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        view.render(element); await settle(); let nodes = view.render(element);
        assert.deepEqual(calls.map(item => item.action), ['chrome.status'], 'opening only inspects locally');
        input(nodes, 'BH API 服务地址').props.onChange({ target: { value: 'https://draft-bh.example.test' } });
        input(nodes, 'BH API 凭据').props.onChange({ target: { value: 'draft-token' } });
        const connect = view.render(element).find(node => node.type === 'button' && node.props['data-chrome-connect']); assert.ok(connect);
        connect.props.onClick(); await settle(); nodes = view.render(element);
        assert.equal(input(nodes, 'Chrome 网页 服务地址').props.value, 'http://127.0.0.1:3010');
        assert.ok(!nodes.some(node => node.type === 'input' && node.props['aria-label'] === 'Chrome 网页 账户名称'), 'Chrome identity is discovered, not user entered');
        assert.equal(input(nodes, 'BH API 服务地址').props.value, 'https://draft-bh.example.test');
        assert.equal(input(nodes, 'BH API 凭据').props.value, 'draft-token');
        assert.deepEqual(calls.map(item => item.action), ['chrome.status', 'chrome.connect', 'chrome.status']);
        assert.ok(!calls.some(item => /generate|submit|images\./.test(item.action)));
      } finally { view.unmount(); }
    });

    await t.test('Chrome manual check waits for save and never refreshes models when the webpage is absent', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), calls: string[] = [];
      chromeRespond = async input => { calls.push(input.action); return { ...chromeReport, status: 'waiting_browser', clients: 0, usableClients: 0 }; };
      const mutate = async (action: string, values: any) => { calls.push(action); return { ...appSettings, revision: 1, settings: values?.settings || appSettings.settings }; };
      try {
        const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
        view.render(element); await settle(); check(view.render(element), 'chrome').props.onClick(); await settle();
        assert.deepEqual(calls, ['chrome.status', 'settings.global.save', 'chrome.check']);
      } finally { view.unmount(); }
    });

    await t.test('Chrome can import an explicitly selected bridge config without copying its token', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), calls: any[] = [];
      respond = async input => { calls.push(input); assert.equal(input.action, 'files.choose'); return ['C:\\synthetic\\bridge.env']; };
      const mutate = async (action: string, values: any) => { calls.push({ action, ...values }); return { ...appSettings, revision: 1, settings: { ...appSettings.settings, chromeEndpoint: 'http://127.0.0.1:3010', chromeAccountId: 'imported-identity' } }; };
      try {
        const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
        view.render(element); await settle();
        const button = view.render(element).find(node => node.type === 'button' && node.props['data-chrome-import']); assert.ok(button);
        button.props.onClick(); await settle(); view.render(element);
        assert.deepEqual(calls, [{ action: 'files.choose', kind: 'file', multiple: false }, { action: 'chrome.connect', sourceConfigPath: 'C:\\synthetic\\bridge.env' }]);
        assert.ok(!JSON.stringify(calls).includes('chromeToken'));
      } finally { view.unmount(); }
    });

    await t.test('Dreamina checks installed components and persists their path without implicit install', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), calls: any[] = [];
      const patch = { dreaminaPath: '/automatic/dreamina' };
      runtimeRespond = async request => { calls.push(request); return runtimeFixture(request.provider, true, patch.dreaminaPath); };
      respond = async request => {
        calls.push(request);
        if (request.action === 'runtime.prepare') return { schemaVersion: 'runtime-prepare/v1', provider: 'dreamina', settings: patch };
        assert.equal(request.action, 'readiness.check');
        assert.deepEqual(request.settings, patch);
        return { schemaVersion: 'provider-readiness/v1', provider: 'dreamina', checkedAt: '2026-10-09T00:00:00Z', installation: { state: 'ready', version: 'synthetic', executable: patch.dreaminaPath }, protocol: { state: 'ready', name: 'synthetic' }, authentication: { state: 'authenticated', identityVerified: true }, catalog: { state: 'available', models: [{ id: 'synthetic-model' }] }, network: { state: 'not_tested', proof: [] }, generation: { enabled: false, testPerformed: false, calls: 0 }, messages: [], details: {} };
      };
      const mutate = async (action: string, values: any) => { calls.push({ action, ...values }); return { ...appSettings, settings: values.settings || appSettings.settings }; };
      const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        view.render(element); await settle();
        const button = view.render(element).find(node => node.type === 'button' && node.props['data-readiness-provider'] === 'dreamina'); assert.ok(button); assert.equal(button.props.disabled, false);
        button.props.onClick(); await settle();
        const nodes = view.render(element);
        assert.deepEqual(calls.filter(item => item.action !== 'runtime.status').map(item => item.action), ['settings.global.save', 'readiness.check', 'dreamina.status']);
        assert.equal(calls.find(item => item.action === 'settings.global.save').settings.dreaminaPath, patch.dreaminaPath);
        assert.ok(nodes.some(node => node.props['data-readiness-result'] === 'dreamina'), 'result survives the discovered path update');
        assert.ok(!calls.some(item => /login|submit|generate/.test(item.action)));
      } finally { view.unmount(); }
    });

    await t.test('Dreamina unknown is unchecked, and a failed model check never claims account refresh', async () => {
      const view = hookRenderer(), appSettings = { ...makeSettings(), dreaminaAccount: { status: 'unknown', ready: false, identityVerified: false } }, calls: string[] = [];
      respond = async request => { calls.push(request.action); throw new Error('synthetic model check failed'); };
      const mutate = async (action: string, values: any) => { calls.push(action); return { ...appSettings, revision: 1, settings: values.settings || appSettings.settings }; };
      try {
        const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
        view.render(element); await settle(); let nodes = view.render(element);
        const card = nodes.find(node => node.type === 'section' && node.props['aria-label'] === '即梦生成 CLI'); assert.ok(card);
        assert.match(content(card), /尚未检查/); assert.doesNotMatch(content(card), /登录状态未登录/);
        nodes.find(node => node.props['data-readiness-provider'] === 'dreamina')!.props.onClick(); await settle(); nodes = view.render(element);
        assert.deepEqual(calls, ['settings.global.save', 'readiness.check']);
        assert.ok(nodes.some(node => node.props.role === 'alert' && content(node).includes('synthetic model check failed')));
      } finally { view.unmount(); }
    });

    for (const leave of [false, true]) await t.test(leave ? 'leaving during Dreamina inspection cancels the later account read' : 'Dreamina model inspection refreshes the main account card', async () => {
      const view = hookRenderer(), calls: string[] = [];
      let appSettings: any = { ...makeSettings(), dreaminaAccount: { status: 'unknown', ready: false, identityVerified: false } }, finishCheck!: (value: unknown) => void;
      respond = async request => { calls.push(request.action); return new Promise(resolve => { finishCheck = resolve; }); };
      const mutate = async (action: string, values: any) => {
        calls.push(action);
        if (action === 'settings.global.save') appSettings = { ...appSettings, revision: 1, settings: values.settings };
        if (action === 'dreamina.status') appSettings = { ...appSettings, dreaminaAccount: { status: 'authenticated', ready: true, identityVerified: true, credits: { total: 321 } } };
        return appSettings;
      };
      const element = () => React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        view.render(element()); await settle(); let nodes = view.render(element());
        nodes.find(node => node.props['data-readiness-provider'] === 'dreamina')!.props.onClick(); await settle(); view.render(element());
        if (leave) view.unmount();
        finishCheck(readinessFixture('dreamina')); await settle();
        assert.deepEqual(calls, ['settings.global.save', 'readiness.check', ...(leave ? [] : ['dreamina.status'])]);
        if (!leave) {
          nodes = view.render(element());
          const card = nodes.find(node => node.type === 'section' && node.props['aria-label'] === '即梦生成 CLI'); assert.ok(card);
          assert.match(content(card), /登录状态已登录/); assert.match(content(card), /总额度 321/);
        }
      } finally { view.unmount(); }
    });

    for (const otherChannel of ['bh', 'dreamina', 'chrome']) await t.test(`OpenAI check queues behind ${otherChannel} and saves against the acknowledged revision`, async () => {
      const view = hookRenderer(), calls: any[] = [], appSettings = { ...makeSettings(), settings: { ...makeSettings().settings, textProvider: 'codex' } };
      let current = appSettings, cliLookups = 0, resolveLookup!: (value: unknown) => void, resolveFirst!: (value: unknown) => void, resolveSecond!: (value: unknown) => void;
      runtimeRespond = async request => request.provider === 'codex_cli' && cliLookups++ > 0 ? new Promise(resolve => { resolveLookup = resolve; }) : runtimeFixture(request.provider);
      respond = async request => { calls.push(request); assert.equal(request.action, 'readiness.check'); return readinessFixture(request.provider); };
      const mutate = async (action: string, values: any) => {
        calls.push({ action, ...values });
        if (action === 'settings.global.save' || action === 'chrome.connect') {
          if (action === 'settings.global.save') assert.equal(values.expectedRevision, current.revision, 'the queued write uses the preceding acknowledged CAS revision');
          return new Promise(resolve => {
            const complete = (value: unknown) => { current = value as typeof appSettings; resolve(value); };
            if (!resolveFirst) resolveFirst = complete; else resolveSecond = complete;
          });
        }
        return current;
      };
      const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        view.render(element); await settle(); let nodes = view.render(element);
        nodes.find(node => node.props['data-readiness-provider'] === 'codex_cli')!.props.onClick(); await settle(); nodes = view.render(element);
        const other = otherChannel === 'bh' ? check(nodes, 'bh') : nodes.find(node => otherChannel === 'dreamina' ? node.props['data-readiness-provider'] === 'dreamina' : node.props['data-chrome-connect']); assert.ok(other); assert.equal(other.props.disabled, false);
        other.props.onClick(); await settle(); nodes = view.render(element);
        assert.equal(calls.filter(item => ['settings.global.save', 'chrome.connect'].includes(item.action)).length, 1);
        resolveLookup(runtimeFixture('codex_cli')); await settle(); nodes = view.render(element);
        assert.equal(calls.filter(item => ['settings.global.save', 'chrome.connect'].includes(item.action)).length, 1, 'the OpenAI save waits rather than disappearing');
        input(nodes, 'BH API 凭据').props.onChange({ target: { value: 'later-draft-token' } });
        const first = calls.find(item => ['settings.global.save', 'chrome.connect'].includes(item.action));
        resolveFirst({ ...current, revision: 1, settings: first.settings || { ...current.settings, chromeEndpoint: 'http://127.0.0.1:53200' } }); await settle(); nodes = view.render(element);
        const saves = calls.filter(item => item.action === 'settings.global.save'), queued = saves.at(-1); assert.ok(queued);
        assert.equal(queued.expectedRevision, 1); assert.equal(queued.settings.bhToken, 'later-draft-token'); assert.equal(queued.settings.codexPath, '/synthetic/codex');
        if (otherChannel === 'chrome') assert.equal(queued.settings.chromeEndpoint, 'http://127.0.0.1:53200', 'component discovery is preserved');
        resolveSecond({ ...current, revision: 2, settings: { ...queued.settings, bhToken: '' } }); await settle(); nodes = view.render(element);
        assert.equal(calls.filter(item => item.action === 'readiness.check' && item.provider === 'codex_cli').length, 1, 'the original OpenAI connection completes');
        assert.equal(input(nodes, 'BH API 凭据').props.value, '');
        assert.ok(!calls.some(item => /generate|submit|\.login$/.test(item.action)));
      } finally { view.unmount(); }
    });

    for (const cancellation of ['unmount', 'permission']) await t.test(`queued connection save is cancelled on ${cancellation}`, async () => {
      const view = hookRenderer(), writes: any[] = [], appSettings = { ...makeSettings(), settings: { ...makeSettings().settings, textProvider: 'codex' } };
      let cliLookups = 0, resolveLookup!: (value: unknown) => void, resolveSave!: (value: unknown) => void;
      runtimeRespond = async request => request.provider === 'codex_cli' && cliLookups++ > 0 ? new Promise(resolve => { resolveLookup = resolve; }) : runtimeFixture(request.provider);
      const mutate = async (action: string, values: any) => { writes.push({ action, ...values }); return new Promise(resolve => { resolveSave = resolve; }); };
      const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        view.render(element); await settle(); let nodes = view.render(element);
        nodes.find(node => node.props['data-readiness-provider'] === 'codex_cli')!.props.onClick(); await settle(); nodes = view.render(element);
        check(nodes, 'bh').props.onClick(); await settle();
        resolveLookup(runtimeFixture('codex_cli')); await settle(); view.render(element);
        if (cancellation === 'unmount') view.unmount(); else view.render(React.createElement(SettingsPanel, { appSettings, writable: false, mutate }));
        resolveSave({ ...appSettings, revision: 1, settings: writes[0].settings }); await settle();
        assert.deepEqual(writes.map(item => item.action), ['settings.global.save'], 'only the write already sent may finish');
        if (cancellation === 'permission') { nodes = view.render(element); assert.equal(check(nodes, 'bh').props.disabled, false, 'cancelled queues release their busy state'); }
      } finally { view.unmount(); }
    });

    await t.test('missing Dreamina disables login and only explicit installation can download components', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), calls: any[] = [];
      let installed = false;
      runtimeRespond = async request => { calls.push(request); return runtimeFixture(request.provider, request.provider === 'dreamina' ? installed : true); };
      respond = async request => { calls.push(request); assert.equal(request.action, 'runtime.prepare'); assert.equal(request.provider, 'dreamina'); installed = true; return { schemaVersion: 'runtime-prepare/v1', provider: 'dreamina', settings: { dreaminaPath: '/synthetic/dreamina' } }; };
      const mutate = async (action: string, values: any) => { calls.push({ action, ...values }); return { ...appSettings, revision: 1, settings: values?.settings || appSettings.settings }; };
      const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        view.render(element); await settle(); let nodes = view.render(element);
        const login = nodes.find(node => node.type === 'button' && node.props['data-execution-action'] === 'dreamina.login'); assert.ok(login); assert.equal(login.props.disabled, true);
        assert.ok(calls.every(item => item.action === 'runtime.status'), 'page entry checks local installation only');
        const card = nodes.find(node => node.type === 'section' && node.props['aria-label'] === '即梦生成 CLI'); assert.ok(card);
        const install = nodes.find(node => node.type === 'button' && content(node) === '安装组件'); assert.ok(install); assert.equal(install.props.disabled, false);
        install.props.onClick(); await settle(); nodes = view.render(element);
        assert.deepEqual(calls.filter(item => item.action !== 'runtime.status').map(item => item.action), ['runtime.prepare', 'settings.global.save', 'dreamina.status']);
        assert.equal(nodes.find(node => node.type === 'button' && node.props['data-execution-action'] === 'dreamina.login')?.props.disabled, false);
        assert.ok(!calls.some(item => /submit|generate|\.login$/.test(item.action)));
      } finally { view.unmount(); }
    });

    await t.test('invalid Dreamina components expose a repair message without attempting replacement', async () => {
      const view = hookRenderer();
      runtimeRespond = async request => request.provider === 'dreamina' ? { ...runtimeFixture('dreamina', false), installSupported: false, components: [{ id: 'dreamina', name: 'Dreamina', state: 'invalid', message: '签名校验失败，请修复客户端。' }] } : runtimeFixture(request.provider);
      try {
        const element = React.createElement(SettingsPanel, { appSettings: makeSettings(), writable: true, mutate: async () => { throw new Error('no automatic mutation'); } });
        view.render(element); await settle(); const nodes = view.render(element);
        assert.ok(nodes.some(node => node.type === 'p' && content(node).includes('签名校验失败')));
        assert.ok(!nodes.some(node => node.type === 'button' && content(node) === '安装组件'));
        assert.equal(nodes.find(node => node.type === 'button' && node.props['data-execution-action'] === 'dreamina.login')?.props.disabled, true);
      } finally { view.unmount(); }
    });

    await t.test('revoking write access while programs are being located prevents saving or login', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), writes: string[] = [];
      let resolveSetup!: (value: unknown) => void;
      let statusCalls = 0;
      runtimeRespond = async request => request.provider === 'dreamina' && statusCalls++ > 0 ? new Promise(resolve => { resolveSetup = resolve; }) : runtimeFixture(request.provider);
      const mutate = async (action: string) => { writes.push(action); return appSettings; };
      try {
        const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
        view.render(element); await settle(); const nodes = view.render(element);
        const login = nodes.find(node => node.type === 'button' && node.props['data-execution-action'] === 'dreamina.login'); assert.ok(login);
        login.props.onClick(); await settle();
        view.render(React.createElement(SettingsPanel, { appSettings, writable: false, mutate }));
        resolveSetup(runtimeFixture('dreamina')); await settle();
        assert.deepEqual(writes, []);
      } finally { view.unmount(); }
    });

    await t.test('edits made during save are retained and cannot be cleared by its stale response', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), writes: any[] = [];
      let resolveSave!: (value: unknown) => void;
      const mutate = async (action: string, values: any) => { writes.push({ action, ...values }); return action === 'settings.global.save' ? new Promise(resolve => { resolveSave = resolve; }) : appSettings; };
      const element = React.createElement(SettingsPanel, { appSettings, writable: true, mutate });
      try {
        let nodes = view.render(element);
        input(nodes, 'BH API 服务地址').props.onChange({ target: { value: 'https://submitted.example.test' } });
        check(view.render(element), 'bh').props.onClick();
        nodes = view.render(element);
        input(nodes, 'Chrome 网页 服务地址').props.onChange({ target: { value: 'http://localhost:9123' } });
        input(nodes, 'BH API 凭据').props.onChange({ target: { value: 'typed-during-save' } });
        resolveSave({ ...appSettings, settings: writes[0].settings }); await settle();
        nodes = view.render(element);
        assert.equal(input(nodes, 'Chrome 网页 服务地址').props.value, 'http://localhost:9123');
        assert.equal(input(nodes, 'BH API 凭据').props.value, 'typed-during-save');
        assert.equal(writes[0].settings.chromeToken, undefined, 'empty credentials do not overwrite stored ones');
        assert.equal(writes.filter(item => item.action === 'providers.refresh').length, 1);
      } finally { view.unmount(); }
    });

    await t.test('leaving global settings during a pending save prevents its follow-up check', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), writes: string[] = [];
      let resolveSave!: (value: unknown) => void;
      const mutate = async (action: string) => { writes.push(action); return new Promise(resolve => { resolveSave = resolve; }); };
      check(view.render(React.createElement(SettingsPanel, { appSettings, writable: true, mutate })), 'bh').props.onClick();
      view.unmount(); resolveSave(appSettings); await settle();
      assert.deepEqual(writes, ['settings.global.save']);
    });
    await t.test('unsaved edits keep the original revision for conflict detection', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), writes: any[] = [];
      const mutate = async (action: string, values: any) => { writes.push({ action, ...values }); return null; };
      try {
        const nodes = view.render(React.createElement(SettingsPanel, { appSettings, writable: true, mutate }));
        input(nodes, 'BH API 服务地址').props.onChange({ target: { value: 'https://draft.example.test' } });
        const latest = { ...appSettings, revision: 2, settings: { ...appSettings.settings, bhEndpoint: 'https://other-writer.example.test' } };
        check(view.render(React.createElement(SettingsPanel, { appSettings: latest, writable: true, mutate })), 'bh').props.onClick(); await settle();
        assert.equal(writes[0].expectedRevision, 0);
        assert.equal(writes[0].settings.bhEndpoint, 'https://draft.example.test');
        assert.equal(writes.length, 1, 'a rejected stale save never advances to a connection check');
      } finally { view.unmount(); }
    });
    await t.test('revoking write access during a save cancels its later connection step', async () => {
      const view = hookRenderer(), appSettings = makeSettings(), writes: string[] = [];
      let resolveSave!: (value: unknown) => void;
      const mutate = async (action: string) => { writes.push(action); return new Promise(resolve => { resolveSave = resolve; }); };
      try {
        check(view.render(React.createElement(SettingsPanel, { appSettings, writable: true, mutate })), 'bh').props.onClick();
        view.render(React.createElement(SettingsPanel, { appSettings, writable: false, mutate }));
        resolveSave(appSettings); await settle();
        assert.deepEqual(writes, ['settings.global.save']);
        const nodes = view.render(React.createElement(SettingsPanel, { appSettings, writable: true, mutate }));
        assert.equal(check(nodes, 'bh').props.disabled, false, 'cancelled operation cannot leave the controls stuck busy');
      } finally { view.unmount(); }
    });
    await t.test('project production saves include fantasy preferences and exclude software connections', async () => {
      const view = hookRenderer(), writes: any[] = [];
      const workspace = { ...structuredClone(previewWorkspace), settings: { fantasyMode: 'review', requireFantasyReview: true, productionDirection: '冷色', productionDirectionConfirmed: true, projectVisualStyle: '3D', ratio: '16:9', textProvider: 'codex_sdk', codexPath: '/private/codex', bhToken: 'never-send' } };
      try {
        const nodes = view.render(React.createElement(ProjectSettingsPanel, { workspace, writable: true, mutate: async (action: string, values: any) => { writes.push({ action, ...values }); return workspace; } }));
        const form = nodes.find(node => node.type === 'form'); assert.ok(form); await form.props.onSubmit({ preventDefault() {} }); await settle();
        assert.equal(writes.length, 1); assert.equal(writes[0].action, 'settings.save');
        assert.equal(writes[0].settings.fantasyMode, 'review'); assert.equal(writes[0].settings.requireFantasyReview, true); assert.equal(writes[0].settings.productionDirectionConfirmed, true);
        for (const key of ['textProvider', 'codexPath', 'bhToken']) assert.equal(writes[0].settings[key], undefined, key);
      } finally { view.unmount(); }
    });
  } finally { await server.close(); }
});
