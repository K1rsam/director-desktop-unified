'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { File } = require('node:buffer');

function fixture(options = {}) {
  let now = 0;
  const events = [], diagnostics = [], reconciliations = [];
  const img = { visible: true, complete: options.complete !== false,
    naturalWidth: options.broken ? 0 : 32, naturalHeight: options.broken ? 0 : 32,
    currentSrc: options.noSource ? '' : 'blob:synthetic-parent-preview',
    getAttribute: key => key === 'src' && !options.noSource ? 'blob:synthetic-parent-preview' : null };
  const attachment = { visible: options.hidden !== true,
    getAttribute: key => key === 'aria-label' ? options.name || 'parent.png' : null,
    querySelectorAll: selector => selector === 'img' && !options.noImage ? [img] : [] };
  const progress = { visible: true, getAttribute: () => 'upload-progress' };
  const uploadControl = { visible: true, getAttribute: key => key === 'data-testid' ? 'upload-file-button' : null };
  const root = { text: options.error || options.composerText || '',
    getAttribute: () => null,
    querySelectorAll: selector => {
      const attachmentSelector = options.legacyComposerAttachment
        ? '[role="button"].composer-attachment-surface' : 'button.image-attachment';
      if (selector.split(', ').includes(attachmentSelector)) return options.noAttachment ? [] : [attachment];
      if (selector.startsWith('[aria-busy')) return options.busy ? [progress] : [];
      if (selector.includes('data-testid*="upload"')) return [uploadControl];
      return [];
    } };
  const input = { disabled: false, dispatchEvent: () => {} };
  const body = { text: options.bodyText || 'parent.png', querySelectorAll: () => [] };
  const context = vm.createContext({ File, Uint8Array, Blob, atob, btoa, Event,
    Date: { now: () => now },
    DataTransfer: class { constructor() { this.files = []; this.items = { add: file => this.files.push(file) }; } },
    document: { body, querySelectorAll: selector => selector === 'input[type="file"]' ? [input] : [] } });
  const deps = {
    CONFIG: { attachmentUploadTimeoutMs: 700 }, EXTENSION_API: {},
    delay: async ms => { now += ms; if (options.afterDelay) options.afterDelay({ img, root }); },
    diagnostic: (name, detail) => diagnostics.push({ name, detail }),
    emitChatEvent: (_, name, detail) => events.push({ name, detail }),
    findComposerRoot: () => options.bodyRoot ? body : options.missingRoot ? null : root,
    findComposerRootStrict: () => options.bodyRoot ? body : options.missingRoot ? null : root,
    findSendButton: () => ({ disabled: options.sendDisabled === true }),
    isUsableButton: node => !node.disabled,
    isVisible: node => node.visible === true,
    visibleText: node => node.text || '', send: () => {},
    getActiveRequest: () => ({ requestId: 'synthetic-request', phase: options.phase || 'attachments_uploading' }),
    getCurrentSession: () => ({ id: 'synthetic-session' }),
    settleEffectReconciliation: async result => reconciliations.push(result),
  };
  context.ChatGptDomUtilities = { isVisible: deps.isVisible, visibleText: deps.visibleText };
  for (const name of ['attachmentCommands.js', 'requestEffectReconciliation.js']) {
    vm.runInContext(fs.readFileSync(path.join(__dirname, '../overrides/extension/content', name), 'utf8'), context);
  }
  const runtime = context.ChatGptAttachmentCommands.createAttachmentCommands(deps);
  const state = (attachments = [{ name: 'parent.png', mime: 'image/png' }]) => context.ChatGptAttachmentCommands.readUploadState(deps.findComposerRoot(), attachments, deps);
  async function reconcile(evidence = { attachments: [{ name: 'parent.png', mime: 'image/png' }], attachmentCount: 1 }) {
    const recovery = context.ChatGptRequestEffectReconciliation.createRequestEffectReconciliation(deps);
    await recovery.handleEffectReconcile({ requestId: 'synthetic-request', effectType: 'attachments.upload', evidence });
    return reconciliations.at(-1);
  }
  return { runtime, state, reconcile, events, diagnostics, img, root, get elapsed() { return now; } };
}

const files = [{ name: 'parent.png', mime: 'image/png', contentBase64: 'AA==', size: 1 }];
const request = { requestId: 'synthetic-request' };

test('exact composer image aria-label with loaded preview completes without waiting or upload-button false busy', async () => {
  const f = fixture();
  assert.equal(f.state().visibleNames[0], 'parent.png');
  assert.equal(f.state().busy, false);
  await f.runtime.attachFiles(files, request);
  assert.equal(f.elapsed, 0);
  assert.equal(f.events.at(-1).name, 'files.attach.done');
  assert.equal((await f.reconcile()).reconciliationOutcome, 'succeeded');
});

test('legacy composer attachment surface role=button with exact aria-label and nested loaded img completes and reconciles', async () => {
  const f = fixture({ legacyComposerAttachment: true });
  assert.equal(f.state().visibleNames[0], 'parent.png');
  await f.runtime.attachFiles(files, request);
  assert.equal(f.elapsed, 0);
  assert.equal((await f.reconcile()).reconciliationOutcome, 'succeeded');
  for (const options of [{ noImage: true }, { complete: false }, { broken: true }, { name: 'parent.png.other' }, { hidden: true }]) {
    const blocked = fixture({ ...options, legacyComposerAttachment: true });
    assert.equal(blocked.state().visibleNames.length, 0);
    await assert.rejects(blocked.runtime.attachFiles(files, request), /Timed out/);
    assert.notEqual((await blocked.reconcile()).reconciliationOutcome, 'succeeded');
  }
});

test('page prose, composer prose and prefix filename cannot prove image attachment', async () => {
  for (const options of [{ noAttachment: true }, { noAttachment: true, composerText: 'parent.png' }, { name: 'parent.png.other' }, { bodyRoot: true }, { missingRoot: true }]) {
    const f = fixture(options);
    assert.equal(f.state().visibleNames.length, 0);
    assert.notEqual((await f.reconcile()).reconciliationOutcome, 'succeeded');
    let submitted = false;
    await assert.rejects(async () => { await f.runtime.attachFiles(files, request); submitted = true; }, /Timed out/);
    assert.equal(submitted, false);
    assert.ok(!f.events.some(event => event.name === 'files.attach.done'));
    assert.equal(f.events.at(-1).name, 'files.attach.failed');
  }
});

test('same name without loaded image proof or visible attachment stays blocked', async () => {
  for (const options of [{ noImage: true }, { broken: true }, { noSource: true }, { complete: false }, { hidden: true }]) {
    const f = fixture(options);
    assert.equal(f.state().visibleNames.length, 0);
    await assert.rejects(f.runtime.attachFiles(files, request), /Timed out/);
    assert.notEqual((await f.reconcile()).reconciliationOutcome, 'succeeded');
  }
});

test('real upload progress blocks waiting and recovery until image load is complete', async () => {
  const busy = fixture({ busy: true });
  assert.equal(busy.state().busy, true);
  await assert.rejects(busy.runtime.attachFiles(files, request), /Timed out/);
  assert.notEqual((await busy.reconcile()).reconciliationOutcome, 'succeeded');
  const loading = fixture({ complete: false, afterDelay: ({ img }) => { img.complete = true; } });
  await loading.runtime.attachFiles(files, request);
  assert.equal(loading.elapsed, 350);
});

test('upload error fails closed immediately instead of swallowing warning and submitting', async () => {
  for (const error of ['Upload failed', '上传失败']) {
    const f = fixture({ error });
    await assert.rejects(f.runtime.attachFiles(files, request), /Attachment upload failed/);
    assert.equal(f.elapsed, 0);
    assert.ok(!f.events.some(event => event.name === 'files.attach.done'));
    assert.equal((await f.reconcile()).reconciliationOutcome, 'failed');
  }
});

test('interrupted upload cannot recover by count alone, partial names or duplicate names', async () => {
  const f = fixture();
  for (const evidence of [
    { attachments: [], attachmentCount: 1 },
    { attachments: [{ name: 'parent.png' }], attachmentCount: 2 },
    { attachments: [{ name: 'parent.png' }, { name: 'parent.png' }], attachmentCount: 2 },
  ]) assert.notEqual((await f.reconcile(evidence)).reconciliationOutcome, 'succeeded');
  assert.equal((await f.reconcile({ attachments: [], attachmentCount: 0 })).reconciliationOutcome, 'succeeded');
});

test('text file attachment retains exact composer-local name contract without image preview', async () => {
  const f = fixture({ name: 'notes.txt', noImage: true });
  assert.equal(f.state([{ name: 'notes.txt', mime: 'text/plain' }]).visibleNames[0], 'notes.txt');
  await f.runtime.attachFiles([{ name: 'notes.txt', mime: 'text/plain', content: 'synthetic' }], request);
  assert.equal(f.elapsed, 0);
});
