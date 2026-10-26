import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

let networkCalls = 0;
Object.assign(globalThis, { location: { search: '?preview=1' }, fetch: async () => { networkCalls++; throw Error('Network forbidden in synthetic UI test'); } });
const shared = globalThis as typeof globalThis & { __imageReviewHooks?: Host };
class Host {
  cells: unknown[] = []; cursor = 0;
  useState(initial: unknown) { const i = this.cursor++; if (!(i in this.cells)) this.cells[i] = initial; return [this.cells[i], (value: unknown) => { this.cells[i] = value; }]; }
  render(component: any, props: any) { this.cursor = 0; shared.__imageReviewHooks = this; return component(props); }
}
const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom', plugins: [{ name: 'isolated-image-review-hooks', enforce: 'pre', transform(code, id) {
  if (id.replaceAll('\\', '/').split('?')[0].endsWith('/src/ImageReviewControl.tsx')) return code.replace("import { useState } from 'react';", "import { useState } from '/@review-hooks';");
}, resolveId(id) { if (id === '/@review-hooks') return '\0review-hooks'; }, load(id) { if (id === '\0review-hooks') return 'export function useState(...args) { return globalThis.__imageReviewHooks.useState(...args); }'; } }] });
const { ImageReviewControl, CandidateImageReview } = await server.ssrLoadModule('/src/ImageReviewControl.tsx');
after(async () => { assert.equal(networkCalls, 0); delete shared.__imageReviewHooks; await server.close(); });
function nodes(tree: any): any[] { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(nodes)]; }
function fixture(capability: any = { verified: false, supportsVisualInput: false }, settings: any = { textProvider: 'codex_sdk' }) {
  const calls: any[] = [], errors: string[] = [], host = new Host();
  const workspace = { imageReviewCapability: capability, settings };
  const props = { workspace, writable: true, mutate: async (action: string, payload: any) => { calls.push({ action, payload }); return null; }, onError: (error: string) => errors.push(error) };
  const render = () => host.render(ImageReviewControl, props);
  return { calls, errors, props, render };
}
const flush = () => new Promise(done => setImmediate(done));

test('unverified visual capability cannot be enabled by calling the checkbox handler', async () => {
  const ui = fixture(), checkbox = nodes(ui.render()).find(node => node.type === 'input');
  assert.equal(checkbox.props.disabled, true);
  checkbox.props.onChange({ target: { checked: true } }); await flush();
  assert.deepEqual(ui.calls, []);
});
test('visual capability uses the explicit SDK action and holds checking or unknown results', async () => {
  const ui = fixture(), button = nodes(ui.render()).find(node => node.type?.name === 'Button');
  assert.equal(button.props.executionAction, 'images.reviewCapability');
  button.props.onClick(); await flush();
  assert.deepEqual(ui.calls, [{ action: 'images.reviewCapability', payload: {} }]);
  for (const state of ['checking', 'unknown']) {
    const held = fixture({ verified: false, supportsVisualInput: false, status: state });
    const action = nodes(held.render()).find(node => node.type?.name === 'Button');
    assert.equal(action.props.disabled, true); action.props.onClick(); await flush(); assert.deepEqual(held.calls, []);
  }
  const cli = fixture(undefined, { textProvider: 'codex' });
  nodes(cli.render()).find(node => node.type?.name === 'Button').props.onClick(); await flush(); assert.deepEqual(cli.calls, []);
});
test('verified opt-in is a project authorization action and stale authorization can be switched off', async () => {
  const ui = fixture({ verified: true, supportsVisualInput: true });
  nodes(ui.render()).find(node => node.type === 'input').props.onChange({ target: { checked: true } }); await flush();
  assert.deepEqual(ui.calls, [{ action: 'images.reviewDelegation.set', payload: { enabled: true } }]);
  const stale = fixture(undefined, { textProvider: 'codex_sdk', imageAdoptionPolicy: 'delegated-review' });
  const checkbox = nodes(stale.render()).find(node => node.type === 'input');
  assert.equal(checkbox.props.checked, true); assert.equal(checkbox.props.disabled, false);
  checkbox.props.onChange({ target: { checked: false } }); await flush();
  assert.deepEqual(stale.calls, [{ action: 'images.reviewDelegation.set', payload: { enabled: false } }]);
});
test('candidate review binds the selected media and never invokes generation or adoption', async () => {
  const ui = fixture({ verified: true, supportsVisualInput: true });
  const props = { ...ui.props, assetId: 'asset-A', media: { id: 'candidate-A', kind: 'image', path: 'synthetic.png' } as any };
  const host = new Host();
  nodes(host.render(CandidateImageReview, props)).find(node => node.type?.name === 'Button').props.onClick(); await flush();
  assert.deepEqual(ui.calls, [{ action: 'images.reviewCandidate', payload: { assetId: 'asset-A', mediaId: 'candidate-A' } }]);
  for (const status of ['unknown', 'reviewing', 'passed']) {
    props.media.visualReview = { status }; const button = nodes(host.render(CandidateImageReview, props)).find(node => node.type?.name === 'Button');
    assert.equal(button.props.disabled, true); button.props.onClick(); await flush();
  }
  assert.equal(ui.calls.length, 1);
});
test('unknown capability and candidate reconcile only their original local evidence', async () => {
  const ui = fixture({ verified: false, supportsVisualInput: false, status: 'unknown' });
  const action = nodes(ui.render()).find(node => node.props?.executionAction === 'images.reviewReconcile');
  action.props.onClick(); await flush();
  const props = { ...ui.props, assetId: 'asset-A', media: { id: 'candidate-A', kind: 'image', path: 'synthetic.png', visualReview: { status: 'unknown' } } };
  const tree = new Host().render(CandidateImageReview, props);
  nodes(tree).find(node => node.props?.executionAction === 'images.reviewReconcile').props.onClick(); await flush();
  assert.deepEqual(ui.calls, [{ action: 'images.reviewReconcile', payload: {} }, { action: 'images.reviewReconcile', payload: { mediaId: 'candidate-A' } }]);
});

test('one new capability probe requires unchecked explicit acknowledgement bound to the original unknown', async () => {
  const explicitRestart = { available: true, expectedAttemptId: 'old-attempt', expectedManifestSha256: 'a'.repeat(64), requiresAcknowledgement: true, maximumNewCalls: 1 };
  const ui = fixture({ verified: false, supportsVisualInput: false, status: 'unknown', explicitRestart });
  const restartButton = () => nodes(ui.render()).find(node => node.props?.executionAction === 'images.reviewCapability.restartUnknown');
  const ack = () => nodes(ui.render()).filter(node => node.type === 'input')[1];
  assert.equal(ack().props.checked, false);
  assert.equal(restartButton().props.disabled, true);
  restartButton().props.onClick(); await flush(); assert.deepEqual(ui.calls, []);
  ack().props.onChange({ target: { checked: true } });
  assert.equal(restartButton().props.disabled, false);
  restartButton().props.onClick(); await flush();
  assert.deepEqual(ui.calls, [{ action: 'images.reviewCapability.restartUnknown', payload: { expectedAttemptId: 'old-attempt', expectedManifestSha256: 'a'.repeat(64), acknowledgePossiblePriorUsage: true } }]);
  ui.props.workspace.imageReviewCapability.explicitRestart = { ...explicitRestart, expectedAttemptId: 'another-attempt' };
  assert.equal(ack().props.checked, false, 'changing the original attempt never carries over the acknowledgement');
  assert.equal(restartButton().props.disabled, true);
  restartButton().props.onClick(); await flush(); assert.equal(ui.calls.length, 1);
});

test('unknown without a backend restart allowance has no new-probe action', () => {
  for (const explicitRestart of [undefined, { available: false, expectedAttemptId: 'old' }]) {
    const ui = fixture({ verified: false, supportsVisualInput: false, status: 'unknown', explicitRestart });
    assert.equal(nodes(ui.render()).some(node => node.props?.executionAction === 'images.reviewCapability.restartUnknown'), false);
  }
});
