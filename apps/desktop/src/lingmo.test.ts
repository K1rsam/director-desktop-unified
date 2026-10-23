import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { rustStandardActionCatalogue } from './test-support/rustExecutionPolicy.ts';
import { lingmoImportBlocked, boundLingmoMentions, insertLingmoToken, lingmoAssetToken, lingmoMentions, lingmoPromptParts, replaceLingmoReference } from './lingmo.ts';
import type { Asset, Workspace } from './types.ts';

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


test('tokens use UTF-8 offsets, exact type suffixes and preserve every source byte', () => {
  const text = '原句😀\r\n@[小白CH]说：@[小白PR]，@[声音SE]@[群演OM]，@[缺失SC]，@[非法XX]@[不闭合';
  const assets: Asset[] = [ { id: 'a', name: '小白', lingmoType: 'CH' }, { id: 'b', name: '小白', lingmoType: 'PR' } ];
  const mentions = lingmoMentions(text, assets);
  assert.equal(mentions.length, 5); assert.equal(mentions[0].start, Buffer.byteLength('原句😀\r\n')); assert.equal(mentions[0].line, 2);
  assert.equal(mentions[0].assetId, 'a'); assert.equal(mentions[1].assetId, 'b'); assert.equal(mentions[4].assetId, undefined);
  assert.equal(lingmoPromptParts(text, mentions).map(part => part.text).join(''), text);
  assert.equal(lingmoPromptParts(text, [{ ...mentions[0], start: 1 }]).map(part => part.text).join(''), text);
});
test('same-name variants stay unresolved until an explicit saved identity is supplied', () => {
  const assets: Asset[] = [{ id: 'a', name: '小白', type: 'CH', variantId: 'v1' }, { id: 'b', name: '小白', type: 'CH', variantId: 'v2' }];
  assert.equal(lingmoMentions('@[小白CH]', assets)[0].assetId, undefined);
  const references = [{ assetId: 'b', variantId: 'v2', role: 'character', order: 1 }];
  assert.equal(boundLingmoMentions('@[小白CH]', assets, references)[0].assetId, 'b');
  assert.equal(boundLingmoMentions('@[小白CH]', assets, [{ ...references[0], variantId: 'wrong' }])[0].assetId, undefined);
  assert.equal(lingmoAssetToken({ id: 'x', name: '坏]名字', type: 'CH' }), undefined);
});
test('exact bound-tag replacement and insertion never rewrite ordinary names or other types', () => {
  const assets: Asset[] = [{ id: 'a', name: '小白', type: 'CH' }, { id: 'b', name: '小白', type: 'PR' }];
  const text = '小白😀@[小白CH] @[小白PR] @[小白CH]';
  assert.equal(replaceLingmoReference(text, lingmoMentions(text, assets), 'a', '@[小白西装CH]'), '小白😀@[小白西装CH] @[小白PR] @[小白西装CH]');
  assert.deepEqual(insertLingmoToken('开头@后面', 3, 3, '@[声音SE]'), { text: '开头@[声音SE]后面', cursor: 9 });
});
test('saved audio identities survive revisions and explicit picker choices resolve duplicate tokens', () => {
  const assets: Asset[] = [{ id: 'old-sound', name: '旁白', type: 'SE', variantId: 'v1' }, { id: 'new-sound', name: '旁白', type: 'SE', variantId: 'v2' }];
  const old = { assetId: 'old-sound', variantId: 'v1', role: 'sound', order: 0 };
  assert.equal(boundLingmoMentions('@[旁白SE]', assets, [old, old])[0].assetId, 'old-sound', 'saved audioAssetRefs and mentions may describe the same identity');
  assert.equal(boundLingmoMentions('@[旁白SE]', assets, [old], { '@[旁白SE]': { assetId: 'new-sound', variantId: 'v2' } })[0].assetId, 'new-sound');
  assert.equal(boundLingmoMentions('@[旁白SE]', assets, [], { '@[旁白SE]': { assetId: 'wrong', variantId: 'v2' } })[0].assetId, undefined, 'invalid explicit identities cannot bind a different token');
});

test('workbench loads only the selected scene, requires import preview, and never calls a generator', async () => {
  const calls: any[] = [], mutations: any[] = [], scopes: any[] = [], errors: string[] = [];
  let diagnostics: any[] = [], previousDocuments: string[] = [];
  const policy = { schemaVersion: 'execution-policy/v1', mode: 'standard', blockedActions: [], allowedActions: rustStandardActionCatalogue(), reason: 'synthetic' };
  const counts = { episodes: 2, scenes: 2, fragments: 2, shots: 2, mentionOccurrences: 2, assets: 1 };
  const workspace: Workspace = { project: { id: 'project', title: '合成项目' }, assets: [], media: [], units: [], jobs: [], settings: {}, providers: [], rules: [], lingmoDocuments: [{ id: 'doc', markdownSha256: 'doc', filename: '合成.md', parserVersion: 'test', counts, diagnostics: [], episodes: [{ id: 'e1', number: 1, sceneIds: ['s1'] }, { id: 'e2', number: 2, sceneIds: ['s2'] }], scenes: [{ id: 's1', episodeId: 'e1', number: '1-1', unitIds: ['u1'], assetIds: [] }, { id: 's2', episodeId: 'e2', number: '2-1', unitIds: ['u2'], assetIds: [] }] }] };
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { getElementById() { return null; } }, window: { __TAURI_INTERNALS__: { invoke: async (_: string, args: { request: any }) => {
    const input = args.request; calls.push(input);
    if (input.action === 'getExecutionPolicy') return policy;
    if (input.action === 'lingmo.scene') return { scene: workspace.lingmoDocuments![0].scenes.find(scene => scene.id === input.sceneId), units: [{ id: input.sceneId === 's1' ? 'u1' : 'u2', prompt: '合成正文', origin: 'imported-storyboard' }], assets: [] };
    if (input.action === 'files.choose') return ['/synthetic.md'];
    if (input.action === 'lingmo.preview') return { markdownSha256: 'new-sha', previewToken: 'new-sha', counts, diagnostics, catalogMatched: false, revisionComparison: { existingDocumentIds: previousDocuments, previousCounts: [], newCounts: counts } };
    throw new Error('Unexpected action ' + input.action);
  } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' }), view = hookRenderer();
  try {
    const { LingmoPanel } = await server.ssrLoadModule('/src/LingmoPanel.tsx');
    let element = React.createElement(LingmoPanel, { workspace, writable: true, mutate: async (action: string, values: any) => { mutations.push({ action, ...values }); return null; }, inspect() {}, onError(message: string) { errors.push(message); }, onScope(scope: any) { scopes.push(scope); } });
    const rendered = async () => { view.render(element); await settle(); return view.render(element); };
    let nodes = await rendered(); nodes = await rendered();
    assert.deepEqual(calls.filter(call => call.action === 'lingmo.scene').map(call => call.sceneId), ['s1']);
    assert.equal(scopes.at(-1).units[0].id, 'u1');
    const settledScopes = scopes.length;
    for (let poll = 0; poll < 3; poll++) {
      const polled = structuredClone(workspace);
      // Polling and unrelated media changes must not unload the selected scene.
      polled.media.push({ id: `unrelated-${poll}`, kind: 'image', path: '/synthetic.png' });
      element = React.cloneElement(element, { workspace: polled });
      nodes = await rendered();
      assert.equal(scopes.length, settledScopes, 'unchanged workspace polls must preserve the loaded scope');
      assert.equal(calls.filter(call => call.action === 'lingmo.scene').length, 1, 'media array identity is not a scene revision');
    }
    element = React.cloneElement(element, { workspace: { ...workspace, units: [{ id: 'u1', version: 2, prompt: '' }] } });
    view.render(element);
    assert.equal(scopes.at(-1).loading, false, 'same-scene revision refresh keeps the previous complete content');
    assert.equal(scopes.at(-1).units[0].id, 'u1');
    nodes = await rendered();
    assert.equal(calls.filter(call => call.action === 'lingmo.scene').length, 2, 'changed unit versions still reload scene content');
    const episodeSelect = nodes.find(node => node.type === 'select' && node.props.value === 'e1'); assert.ok(episodeSelect); episodeSelect.props.onChange({ target: { value: 'e2' } });
    nodes = await rendered(); nodes = await rendered();
    assert.deepEqual(calls.filter(call => call.action === 'lingmo.scene').map(call => call.sceneId), ['s1', 's1', 's2']);
    assert.equal(scopes.at(-1).units[0].id, 'u2');
    assert.equal(mutations.length, 0);
    nodes.find(node => node.type === 'button' && content(node) === '选择分镜 MD')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    nodes.find(node => node.type === 'button' && content(node) === '预览结构')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    assert.equal(mutations.length, 0); assert.ok(nodes.some(node => content(node).includes('2 集 · 2 场 · 2 片段 · 2 镜')));
    const confirm = nodes.find(node => node.type === 'button' && content(node) === '确认导入'); assert.ok(confirm); assert.equal(confirm.props.disabled, false); confirm.props.onClick(); nodes = await rendered();
    assert.equal(mutations.length, 1); assert.equal(mutations[0].action, 'lingmo.import'); assert.equal(mutations[0].expectedSha256, 'new-sha');
    assert.equal(mutations[0].previewToken, 'new-sha');
    diagnostics = [{ code: 'catalog_hash_mismatch', message: '目录版本不匹配', blocking: true, line: 1 }];
    nodes.find(node => node.type === 'button' && content(node) === '预览结构')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    assert.equal(nodes.find(node => node.type === 'button' && content(node) === '确认导入')!.props.disabled, false, 'unmatched catalogs permit text import');
    previousDocuments = ['old-sha'];
    nodes.find(node => node.type === 'button' && content(node) === '预览结构')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    assert.equal(nodes.find(node => node.type === 'button' && content(node) === '确认导入')!.props.disabled, true);
    nodes.find(node => node.type === 'input' && node.props.type === 'checkbox')!.props.onChange({ target: { checked: true } }); nodes = await rendered();
    assert.equal(nodes.find(node => node.type === 'button' && content(node) === '确认导入')!.props.disabled, false);
    diagnostics = [{ code: 'unit_without_scene', message: '片段缺少场次', blocking: true, line: 2 }];
    nodes.find(node => node.type === 'button' && content(node) === '预览结构')!.props.onClick(); nodes = await rendered(); nodes = await rendered();
    assert.equal(nodes.find(node => node.type === 'button' && content(node) === '确认导入')!.props.disabled, true);
    assert.deepEqual(errors, []); assert.ok(calls.every(call => ['getExecutionPolicy', 'lingmo.scene', 'files.choose', 'lingmo.preview'].includes(call.action)));
  } finally { view.unmount(); await server.close(); }
});

test('structural ownership failures block import, while production-only diagnostics preserve text browsing', () => {
  for (const code of ['duplicate_number', 'scene_without_episode', 'scene_episode_mismatch', 'unit_without_scene']) assert.equal(lingmoImportBlocked([{ code, message: code, blocking: true }]), true);
  for (const code of ['catalog_hash_mismatch', 'ambiguous_token', 'invalid_token', 'missing_shots']) assert.equal(lingmoImportBlocked([{ code, message: code, blocking: true }]), false);
});
