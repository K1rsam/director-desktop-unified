import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import type { Host } from './test-support/hookRenderer.ts';
import { canMarkIdentityLayout, identityLayoutFor, requiresIdentityLayout } from './identityLayout.ts';
import type { IdentityLayoutEditorProps } from './IdentityLayoutEditor.tsx';
import type { Asset, Media, Workspace } from './types.ts';

const sha = 'a'.repeat(64);
function fixture() {
  const asset: Asset = { id: 'mother', variantId: 'base', kind: 'CH', name: '合成人物母版', adoptedMediaId: 'adopted' };
  const media: Media = { id: 'adopted', assetId: 'mother', variantId: 'base', kind: 'image', path: 'data:image/png;base64,c3ludGhldGlj', previewUrl: 'data:image/png;base64,dGh1bWI=', thumbnailPath: 'thumb.png', sha256: sha };
  return { asset, media };
}
const receipt = () => ({ version: 'asset-identity-layout/v1', basis: 'user-inspected', confirmed: true, mediaId: 'adopted', sha256: sha, width: 1024, height: 1024, identityStrip: { x: 0, y: 0, width: 1024, height: 333 } });
test('identity regions are valid only for the currently adopted, exact image and explicit inspected rectangle', () => {
  const { asset, media } = fixture();
  assert.equal(canMarkIdentityLayout(asset, media), true);
  assert.equal(identityLayoutFor(asset, media), null);
  asset.identityLayout = receipt();
  assert.deepEqual(identityLayoutFor(asset, media), receipt());
  assert.equal(identityLayoutFor(asset, { ...media, sha256: 'b'.repeat(64) }), null);
  assert.equal(identityLayoutFor({ ...asset, adoptedMediaId: 'replacement' }, media), null);
  for (const patch of [{ placeholder: true }, { kind: 'video' }, { sha256: 'bad' }]) assert.equal(canMarkIdentityLayout(asset, { ...media, ...patch }), false);
  for (const kind of ['SC', 'person', '人物', 'ch']) assert.equal(canMarkIdentityLayout({ ...asset, kind }, media), false);
  for (const patch of [{ confirmed: false }, { basis: 'guessed' }, { height: 512 }, { identityStrip: { x: 0, y: 0, width: 1024, height: 1024 } }, { identityStrip: { x: 1, y: 0, width: 1024, height: 333 } }]) {
    assert.equal(identityLayoutFor({ ...asset, identityLayout: { ...receipt(), ...patch } }, media), null);
  }
  assert.equal(requiresIdentityLayout({ ...asset, imagePresentation: { layoutKind: 'costume' } }, { role: 'face' }), true);
  assert.equal(requiresIdentityLayout({ ...asset, imageTask: { kind: 'costume' } }, { kind: 'identity-faces' }), true);
  assert.equal(requiresIdentityLayout(asset, { role: 'face' }), false);
});

const findButton = (nodes: Host[], label: string) => { const button = nodes.find(node => node.type === 'button' && content(node) === label); assert.ok(button, label); return button; };
async function withEditor(run: (ui: Awaited<ReturnType<typeof editor>>) => Promise<void>) {
  const ui = await editor();
  try { await run(ui); } finally { ui.view.unmount(); await ui.server.close(); }
}
async function editor() {
  Object.assign(globalThis, { isTauri: false, location: { search: '?preview=1' }, document: { activeElement: null }, fetch: async () => { throw Error('Real network forbidden'); } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const { IdentityLayoutEditor } = await server.ssrLoadModule('/src/IdentityLayoutEditor.tsx');
  const { asset, media } = fixture(), calls: any[] = [], errors: string[] = [], view = hookRenderer();
  const props: IdentityLayoutEditorProps = { asset, media, writable: true, mutate: async (action, payload, notice) => { calls.push({ action, payload, notice }); return { project: {}, assets: [], units: [] } as unknown as Workspace; }, onError: error => errors.push(error) };
  const render = () => view.render(React.createElement<IdentityLayoutEditorProps>(IdentityLayoutEditor, props));
  const open = () => { findButton(render(), '标记锁脸区域').props.onClick(); return render(); };
  const load = (width = 1024, height = 1024) => { render().find(node => node.type === 'img')!.props.onLoad({ currentTarget: { naturalWidth: width, naturalHeight: height } }); return render(); };
  return { server, view, props, calls, errors, render, open, load };
}

test('layout registration displays original media, requires visual confirmation and saves unscaled original pixels once', async () => {
  await withEditor(async ui => {
    let finish: (value: Workspace | null) => void = () => {};
    ui.props.mutate = async (action, payload, notice) => { ui.calls.push({ action, payload, notice }); return new Promise(resolve => { finish = resolve; }); };
    const opened = ui.open();
    assert.equal(opened.find(node => node.type === 'img')!.props.src, ui.props.media!.path, 'must not display the thumbnail as the inspection source');
    assert.equal(ui.calls.length, 0);
    let nodes = ui.load();
    assert.equal(nodes.find(node => node.type === 'input' && node.props.type === 'checkbox')!.props.checked, false);
    assert.equal(findButton(nodes, '确认并保存区域').props.disabled, true, 'square dimensions do not constitute review');
    nodes.find(node => node.type === 'input' && node.props.type === 'number')!.props.onChange({ target: { value: '333' } });
    nodes = ui.render();
    assert.equal(nodes.find(node => node.props.className === 'identity-layout-upper')!.props.style.height, `${333 / 1024 * 100}%`);
    nodes.find(node => node.type === 'input' && node.props.type === 'checkbox')!.props.onChange({ target: { checked: true } });
    const save = findButton(ui.render(), '确认并保存区域'); assert.equal(save.props.disabled, false);
    save.props.onClick(); save.props.onClick(); await settle();
    assert.equal(ui.calls.length, 1);
    assert.deepEqual(ui.calls[0].payload, { assetId: 'mother', variantId: 'base', mediaId: 'adopted', expectedSha256: sha, topHeight: 333, confirmed: true });
    assert.equal(ui.calls[0].action, 'assets.identityLayout.save');
    assert.match(ui.calls[0].notice, /不是图片质检或自动采用证明/);
    assert.ok(!content(ui.render().find(node => node.type === 'dialog')!).includes(sha));
    finish({} as Workspace); await settle(); assert.ok(!ui.render().some(node => node.type === 'dialog'));
    assert.deepEqual(ui.errors, []);
  });
});

test('changed crop, failed original or non-square media cannot submit a registration', async () => {
  await withEditor(async ui => {
    ui.open(); let nodes = ui.load();
    nodes.find(node => node.type === 'input' && node.props.type === 'checkbox')!.props.onChange({ target: { checked: true } });
    ui.render().find(node => node.type === 'input' && node.props.type === 'range')!.props.onChange({ target: { value: '400' } });
    nodes = ui.render(); assert.equal(nodes.find(node => node.type === 'input' && node.props.type === 'checkbox')!.props.checked, false);
    findButton(nodes, '确认并保存区域').props.onClick(); await settle(); assert.equal(ui.calls.length, 0);
    nodes = ui.load(1024, 768); assert.equal(findButton(nodes, '确认并保存区域').props.disabled, true);
    assert.match(content(nodes.find(node => node.type === 'dialog')!), /不是 1:1/);
    nodes.find(node => node.type === 'img')!.props.onError(); nodes = ui.render();
    assert.match(content(nodes.find(node => node.type === 'dialog')!), /原图读取失败/);
    findButton(nodes, '确认并保存区域').props.onClick(); await settle(); assert.equal(ui.calls.length, 0);
  });
});

test('changing adopted media closes the previous inspection and old callbacks cannot register another image', async () => {
  await withEditor(async ui => {
    ui.open(); const nodes = ui.load();
    nodes.find(node => node.type === 'input' && node.props.type === 'checkbox')!.props.onChange({ target: { checked: true } });
    const staleSave = findButton(ui.render(), '确认并保存区域');
    const staleImage = ui.render().find(node => node.type === 'img')!;
    ui.props.asset = { ...ui.props.asset, adoptedMediaId: 'replacement' };
    ui.props.media = { ...ui.props.media!, id: 'replacement', sha256: 'b'.repeat(64) };
    assert.ok(!ui.render().some(node => node.type === 'dialog'));
    staleSave.props.onClick(); staleImage.props.onLoad({ currentTarget: { naturalWidth: 4096, naturalHeight: 4096 } }); await settle();
    assert.equal(ui.calls.length, 0);
    const fresh = ui.open(); assert.match(content(fresh.find(node => node.type === 'dialog')!), /正在读取原图尺寸/);
    ui.props.media.placeholder = true;
    assert.equal(ui.render().length, 0, 'placeholder media never offers an inspection registration');
  });
});
