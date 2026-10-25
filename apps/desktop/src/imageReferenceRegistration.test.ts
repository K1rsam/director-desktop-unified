import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { content, hookRenderer, settle } from './test-support/hookRenderer.ts';
import { imageReferenceRegistrations } from './referenceRegistrationPlan.ts';
import type { Workspace } from './types.ts';
import type { ImageReferenceRegistrationProps } from './ImageReferenceRegistration.tsx';

function fixture(): Workspace {
  return { project: { id: 'synthetic', title: '参考登记' }, assets: [
    { id: 'child', name: '战斗装', version: 2, referenceRequirements: [{ targetAssetId: 'mother', reason: '锁定同一身份三脸' }, { targetAssetId: 'mother' }, { targetAssetId: 'optional', required: false }], sourceAssetId: 'ancestor' },
    { id: 'mother', name: '基础母版', version: 4, variantId: 'base', referenceRequirements: [{ targetAssetId: 'ancestor' }] },
    { id: 'ancestor', name: '身份祖先', version: 1 },
    { id: 'unrelated', name: '无关场景', version: 1, referenceRequirements: [{ targetAssetId: 'outside' }] },
  ], units: [], media: [], jobs: [], settings: {}, providers: [], rules: [], imageProductionScope: { assetIds: ['child'], referenceOnlyIds: [] } };
}

test('reference registration only follows required direct media edges from the formal range', () => {
  const workspace = fixture(), before = structuredClone(workspace);
  const plans = imageReferenceRegistrations(workspace);
  assert.equal(plans.length, 1); assert.equal(plans[0].assetId, 'mother');
  assert.deepEqual(plans[0].servesAssetIds, ['child']); assert.equal(plans[0].addedImageCount, 1);
  assert.equal(plans[0].reason, '锁定同一身份三脸'); assert.deepEqual(workspace, before);
  delete workspace.imageProductionScope;
  assert.deepEqual(imageReferenceRegistrations(workspace), [], 'do not infer a production expansion without explicit scope');
});

test('adopted mothers are reusable and candidates or missing identities cannot impersonate an adopted reference', () => {
  const workspace = fixture();
  workspace.media = [{ id: 'm', kind: 'image', path: 'synthetic.png', assetId: 'mother', variantId: 'base' }];
  assert.equal(imageReferenceRegistrations(workspace)[0].addedImageCount, 1);
  workspace.assets[1].adoptedMediaId = 'm';
  assert.equal(imageReferenceRegistrations(workspace)[0].addedImageCount, 0);
  assert.equal(imageReferenceRegistrations(workspace)[0].currentMediaId, 'm');
  workspace.media[0].placeholder = true;
  assert.equal(imageReferenceRegistrations(workspace)[0].addedImageCount, 1, 'white placeholders require real reference production');
  assert.equal(imageReferenceRegistrations(workspace)[0].currentMediaId, undefined);
  workspace.media[0].placeholder = false;
  workspace.assets[1].imageStatus = 'needs_review';
  assert.equal(imageReferenceRegistrations(workspace)[0].currentMediaId, undefined, 'a stale adopted mother is not automatically reusable');
  workspace.assets[1].imageStatus = 'adopted';
  workspace.imageProductionScope!.referenceOnlyIds = ['mother'];
  assert.equal(imageReferenceRegistrations(workspace)[0].registered, true);
  workspace.assets.splice(1, 1);
  assert.equal(imageReferenceRegistrations(workspace)[0].asset, undefined);
});

test('reference registration advances one explicit level and removes stale support scope without duplicating count', () => {
  const workspace=fixture();
  assert.deepEqual(imageReferenceRegistrations(workspace).map(plan=>plan.assetId),['mother']);
  workspace.imageProductionScope!.referenceOnlyIds=['mother'];
  let plans=imageReferenceRegistrations(workspace);
  assert.deepEqual(plans.map(plan=>plan.assetId),['mother','ancestor']);
  assert.equal(plans[0].registered,true);assert.equal(plans[0].addedImageCount,0);
  assert.deepEqual(plans[1].servesAssetIds,['mother']);assert.equal(plans[1].addedImageCount,1);
  workspace.imageProductionScope!.referenceOnlyIds=['mother','ancestor'];
  plans=imageReferenceRegistrations(workspace);
  assert.ok(plans.every(plan=>plan.registered&&plan.addedImageCount===0));
  assert.equal(workspace.imageProductionScope!.assetIds.length,1,'registration never adds formal visual use');
  workspace.imageProductionScope={assetIds:[],referenceOnlyIds:[]};
  assert.deepEqual(imageReferenceRegistrations(workspace),[],'backend-revoked support scope is not retained by frontend ancestry');
});

test('reference UI previews added count then sends one explicit local registration and refresh, without generating', async () => {
  const calls: any[] = [], mutations: string[] = [], notices: string[] = [], errors: string[] = [];
  const workspace = fixture(); let finish: (value: any) => void = () => {};
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { invoke: async (_: string, args: { request: any }) => {
    const input = args.request; calls.push(input);
    if (input.action === 'getExecutionPolicy') return { mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: 'verified_active', allowedActions: ['images.registerReference', 'workspace.get'], reasons: [] };
    if (input.action === 'images.registerReference') return new Promise(resolve => { finish = resolve; });
    throw Error('Unexpected request: ' + input.action);
  } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const view = hookRenderer();
  try {
    const { ImageReferenceRegistration } = await server.ssrLoadModule('/src/ImageReferenceRegistration.tsx');
    const element = () => React.createElement<ImageReferenceRegistrationProps>(ImageReferenceRegistration, { workspace, writable: true, mutate: async (action: string) => { mutations.push(action); return workspace; }, onNotice: (message: string) => notices.push(message), onError: (message: string) => errors.push(message), onSelect() {} });
    let nodes = view.render(element());
    const open = nodes.find(node => node.type === 'button' && content(node) === '登记 1 张参考母版')!;
    assert.ok(open); assert.equal(calls.length, 0); open.props.onClick(); nodes = view.render(element());
    const dialog = nodes.find(node => node.type === 'dialog')!;
    assert.match(content(dialog), /正式图片 1 张 ＋ 参考母版 1 张（新增 1 张）/);
    assert.match(content(dialog), /本次只登记用途与数量，不会生成图片/);
    assert.equal(calls.length, 0);
    const confirm = nodes.find(node => node.type === 'button' && content(node) === '确认登记，不生成')!;
    confirm.props.onClick(); confirm.props.onClick(); await settle(); await settle();
    assert.equal(calls.filter(input => input.action === 'images.registerReference').length, 1);
    assert.deepEqual(calls.find(input => input.action === 'images.registerReference'), { action: 'images.registerReference', projectId: 'synthetic', assetId: 'mother', expectedVersion: 4, servesAssetIds: ['child'], reason: '锁定同一身份三脸', confirmed: true });
    assert.equal(mutations.length, 0);
    finish({ assetId: 'mother', addedImageCount: 1, reusedAdoptedMedia: false }); await settle(); await settle();
    assert.deepEqual(mutations, ['workspace.get']); assert.equal(notices.length, 1); assert.match(notices[0], /尚未生成/); assert.equal(errors.length, 0);
    assert.ok(calls.every(input => ['getExecutionPolicy', 'images.registerReference'].includes(input.action)));
    assert.ok(!view.render(element()).some(node => node.type === 'dialog'));
  } finally { view.unmount(); await server.close(); }
});

test('reference confirmation rejects changed mother versions instead of silently reusing the new version', async () => {
  const workspace = fixture(), errors: string[] = [];
  Object.assign(globalThis, { isTauri: true, location: { search: '' }, document: { activeElement: null }, window: { __TAURI_INTERNALS__: { invoke: async () => { throw Error('No request may be sent for stale registration'); } } } });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  const view = hookRenderer();
  try {
    const { ImageReferenceRegistration } = await server.ssrLoadModule('/src/ImageReferenceRegistration.tsx');
    const element = () => React.createElement<ImageReferenceRegistrationProps>(ImageReferenceRegistration, { workspace, writable: true, mutate: async () => { throw Error('No mutation permitted'); }, onNotice() {}, onError: (message: string) => errors.push(message), onSelect() {} });
    view.render(element()).find(node => node.type === 'button' && content(node) === '登记 1 张参考母版')!.props.onClick();
    workspace.assets[1].version = 5;
    view.render(element()).find(node => node.type === 'button' && content(node) === '确认登记，不生成')!.props.onClick(); await settle();
    assert.equal(errors.length, 1); assert.match(errors[0], /已变化/);
    assert.ok(!view.render(element()).some(node => node.type === 'dialog'));
  } finally { view.unmount(); await server.close(); }
});
