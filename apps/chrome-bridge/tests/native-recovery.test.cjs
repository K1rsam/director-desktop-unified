'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { webcrypto, createHash } = require('node:crypto');

const requestId = '12345678-1234-1234-1234-123456789abc';
const conversationId = 'owned-conversation';
const userId = 'native-user';
const assistantId = 'native-assistant';
const imageId = 'file-original';
const originalUrl = `https://chatgpt.com/backend-api/estuary/content?id=${imageId}`;
const png = Uint8Array.from([137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 10, 20, 30, 40]);
const prompt = `请生成一张原图。任务关联码：${requestId}。`;
const hash = value => createHash('sha256').update(value).digest('hex');
const script = name => fs.readFileSync(path.join(__dirname, '../overrides/extension/content', name), 'utf8');

function record() {
  return { conversation_id: conversationId, current_node: 'assistant-node', mapping: {
    root: { parent: null, message: null },
    'user-node': { parent: 'root', message: { id: userId, author: { role: 'user' }, content: { content_type: 'text', parts: [prompt] } } },
    'assistant-node': { parent: 'user-node', message: { id: assistantId, author: { role: 'assistant' }, status: 'finished_successfully', end_turn: true, content: { content_type: 'multimodal_text', parts: [{ asset_pointer: `sediment://${imageId}` }] } } },
  } };
}

async function fixture(options = {}) {
  let active = options.active || null;
  let launch = options.launch === undefined ? `director-desktop-${requestId}` : options.launch;
  const calls = [], observed = [], messages = [];
  const image = { isConnected: true, src: options.blob ? 'blob:https://chatgpt.com/synthetic' : '', currentSrc: '', naturalWidth: 0, naturalHeight: 0, closest: () => ({}), getAttribute: () => '' };
  const user = { isConnected: true, innerText: options.domPrompt || prompt, querySelectorAll: () => [] };
  const assistant = { isConnected: true, innerText: '', querySelectorAll: () => [image] };
  const turns = [user, ...(options.noAssistant ? [] : [assistant]), ...(options.extraUser ? [user] : []), ...(options.extraAssistant ? [assistant] : [])];
  let reads = 0;
  const context = vm.createContext({ URL, AbortController, TextEncoder, TextDecoder, Uint8Array, Response, crypto: webcrypto, setTimeout, clearTimeout,
    location: { href: `https://chatgpt.com/c/${conversationId}`, pathname: `/c/${conversationId}` }, document: { title: 'Synthetic owned tab' },
    ChatGptTurnDom: { createTurnDom: () => ({ getTurnNodes: () => turns, role: node => node === user ? 'user' : 'assistant' }) },
    fetch: async (url, init) => {
      calls.push({ url, init });
      assert.equal(init.method || 'GET', 'GET', 'recovery cannot submit prompts or create jobs');
      if (url.endsWith(`/backend-api/conversation/${conversationId}`)) {
        const value = record();
        if(options.noAssistant){value.current_node='user-node';delete value.mapping['assistant-node'];}
        reads++;
        if (options.backendPrompt) value.mapping['user-node'].message.content.parts = [options.backendPrompt];
        if (options.status) value.mapping['assistant-node'].message.status = options.status;
        if (reads === 2 && options.changedBranch) value.mapping['assistant-node'].message.id = 'different-assistant';
        return new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json' } });
      }
      if (url.endsWith('/api/auth/session')) return Response.json({ accessToken: 'synthetic-token-never-persisted' });
      if (url.includes(`/backend-api/files/download/${imageId}`)) return Response.json({ download_url: options.wrongFile ? originalUrl.replace(imageId, 'file-other') : originalUrl });
      if (url === originalUrl || url === image.src && options.blob) {
        if (options.newActiveDuringDownload) active = { requestId: 'other-request' };
        if (options.newLaunchDuringDownload) launch = 'different-owned-tab';
        if (options.changePromptDuringDownload) user.innerText = 'different prompt';
        const bytes = url.startsWith('blob:') && options.differentBlob ? Uint8Array.from([...png.slice(0, -1), 99]) : png;
        return new Response(bytes, { headers: { 'content-type': 'image/png' } });
      }
      throw new Error(`Unexpected mock fetch: ${url}`);
    },
  });
  vm.runInContext(script('conversationReconciliation.js'), context);
  const proof = context.ChatGptImageRecordProof;
  const snapshot = () => {
    observed.push({ turnKey: proof.key(assistant), source: proof.imageSource(image) });
    return { answer: '', artifacts: proof.imageSource(image) ? [{ id: 'original-artifact', kind: 'image', generatedImage: true, downloadUrl: proof.imageSource(image) }] : [], turnKey: proof.key(assistant) || 'dom-turn-ordinal', messageId: proof.key(assistant), hasFinalMessage: true };
  };
  const expected = { mode: 'image_record_v1', requestId, conversationId, promptSha256: await proof.digest(prompt), imageCount: 1, allowReleasedRequest: options.allowReleasedRequest !== false, allowReloadedOriginalTab: options.allowReloadedOriginalTab === true };
  const deps = { getActiveRequest: () => active, getBrowserLaunchToken: () => launch, getContentEpoch: () => 'content-epoch', readLatestAssistantSnapshot: snapshot, onBound: () => {} };
  return { context, proof, expected, deps, snapshot, calls, observed, messages, user, assistant, image, get reads() { return reads; } };
}

test('released task downloads its native original bytes only after exact branch recheck and binds canonical identities', async () => {
  for (const prefix of ['director-desktop-', 'bridge-director-desktop-']) {
    const f = await fixture({ launch: prefix + requestId });
    const result = await f.proof.read(f.expected, f.snapshot(), f.deps);
    assert.equal(result.status, 'matched_complete');
    assert.equal(result.reason, 'exact_single_user_native_download');
    assert.equal(result.requestId, requestId);
    assert.equal(result.assistantMessageId, assistantId);
    assert.equal(f.reads, 2, 'must reread branch after receiving the bytes');
    assert.equal(f.proof.key(f.assistant), assistantId);
    assert.equal(f.proof.key(f.user), userId);
    assert.equal(f.proof.imageReady(f.image), true);
    assert.equal(result.bindings[0].bytesSha256, hash(png));
    assert.equal(result.bindings[0].recordSha256, result.bindings[0].recordRecheckSha256);
    const artifact = { sourceTurnKey: assistantId, requestId };
    const download = f.proof.downloadBytes(artifact, originalUrl);
    assert.equal(hash(download.bytes), hash(png));
    assert.equal(download.mime, 'image/png');
    assert.equal(f.proof.downloadBytes({ ...artifact, requestId: 'different-request' }, originalUrl), null);
    assert.equal(f.proof.downloadBytes({ ...artifact, sourceTurnKey: 'different-turn' }, originalUrl), null);
    assert.ok(f.calls.every(call => call.init.method === 'GET'));
    assert.doesNotMatch(JSON.stringify(result), /synthetic-token/);
    f.user.innerText = 'edited after proof';
    assert.equal(f.proof.downloadBytes(artifact, originalUrl), null, 'edited DOM revokes byte binding');
    assert.equal(f.proof.key(f.user), '');
  }
});

test('exact active user turn waiting for its first assistant is processing, never image success', async () => {
  const f=await fixture({noAssistant:true,active:{requestId}});
  const r=await f.proof.read(f.expected,f.snapshot(),f.deps);
  assert.equal(r.status,'matched_incomplete');assert.equal(r.reason,'assistant_not_created');
  assert.equal(f.calls.length,1);assert.equal(f.proof.imageReady(f.image),false);
  for(const options of [{noAssistant:true},{noAssistant:true,active:{requestId},backendPrompt:'other'},
    {noAssistant:true,active:{requestId},domPrompt:'other'},{extraUser:true,active:{requestId}},
    {extraAssistant:true,active:{requestId}}]){
    const rejected=await fixture(options),proof=await rejected.proof.read(rejected.expected,rejected.snapshot(),rejected.deps);
    assert.equal(proof.status,'mismatch');assert.equal(rejected.proof.imageReady(rejected.image),false);
  }
});

test('released request needs explicit opt-in, exact own launch token, valid UUID and no other active task', async () => {
  for (const options of [{ launch: 'unrelated' }, { launch: `director-desktop-${requestId}-suffix` }, { active: { requestId: 'other-request' } }, { allowReleasedRequest: false }]) {
    const f = await fixture(options);
    const result = await f.proof.read(f.expected, f.snapshot(), f.deps);
    assert.equal(result.reason, 'active_request_identity');
    assert.equal(f.calls.length, 0);
    assert.equal(f.proof.imageReady(f.image), false);
  }
  const f = await fixture({ launch: 'director-desktop-not-a-uuid' });
  assert.equal((await f.proof.read({ ...f.expected, requestId: 'not-a-uuid' }, f.snapshot(), f.deps)).reason, 'active_request_identity');
  assert.equal(f.calls.length, 0);
});

test('original tab after reload requires explicit source-bound opt-in and still rejects other ownership', async () => {
  for (const launch of ['', 'bridge-reload-original']) {
    const denied = await fixture({ launch });
    assert.equal((await denied.proof.read(denied.expected, denied.snapshot(), denied.deps)).reason, 'active_request_identity');
    assert.equal(denied.calls.length, 0);
    const f = await fixture({ launch, allowReloadedOriginalTab: true });
    const result = await f.proof.read(f.expected, f.snapshot(), f.deps);
    assert.equal(result.status, 'matched_complete');
    assert.equal(result.bindings[0].bytesSha256, hash(png));
    assert.equal(f.reads, 2);
  }
  for (const options of [{launch:'bridge-director-desktop-other'}, {active:{requestId:'other-request'},launch:'bridge-reload-original'}, {domPrompt:'wrong prompt',launch:'bridge-reload-original'}]) {
    const f = await fixture({...options,allowReloadedOriginalTab:true});
    assert.notEqual((await f.proof.read(f.expected, f.snapshot(), f.deps)).status,'matched_complete');
    assert.equal(f.calls.length,0);
  }
});

test('DOM and backend prompt, completion, file identity and reread branch mismatches never bind original bytes', async () => {
  for (const [options, reason] of [
    [{ domPrompt: 'changed prompt' }, 'dom_user_boundary'],
    [{ backendPrompt: 'changed prompt' }, 'backend_prompt'],
    [{ status: 'in_progress' }, 'backend_generation_not_complete'],
    [{ changedBranch: true }, 'native_branch_changed'],
    [{ wrongFile: true }, 'native_file_identity'],
    [{ changePromptDuringDownload: true }, 'dom_changed_during_proof'],
    [{ newActiveDuringDownload: true }, 'active_request_identity'],
    [{ newLaunchDuringDownload: true }, 'active_request_identity'],
  ]) {
    const f = await fixture(options);
    const result = await f.proof.read(f.expected, f.snapshot(), f.deps);
    assert.equal(result.reason, reason, JSON.stringify(options));
    assert.notEqual(result.status, 'matched_complete');
    assert.equal(f.proof.imageReady(f.image), false);
    assert.equal(f.proof.downloadBytes({ sourceTurnKey: assistantId, requestId }, originalUrl), null);
    assert.equal(f.proof.key(f.assistant), '');
  }
});

test('blob image bytes must equal the native original before trust is granted', async () => {
  const equal = await fixture({ blob: true });
  const proof = await equal.proof.read(equal.expected, equal.snapshot(), equal.deps);
  assert.equal(proof.status, 'matched_complete');
  assert.equal(proof.proofSource, 'dom_blob_bytes_equal');
  const different = await fixture({ blob: true, differentBlob: true });
  assert.equal((await different.proof.read(different.expected, different.snapshot(), different.deps)).reason, 'blob_original_bytes_differ');
  assert.equal(different.proof.imageReady(different.image), false);
});

test('response recovery rereads the proof-bound snapshot and returns original request and native turn identity', async () => {
  const f = await fixture();
  vm.runInContext(script('responseRecovery.js'), f.context);
  const observations = [];
  const recovery = f.context.ChatGptResponseRecovery.createResponseRecovery({
    ...f.deps,
    readRecentAssistantSnapshots: () => [f.snapshot()],
    readLatestAssistantSnapshot: f.snapshot,
    normalizeText: value => String(value || '').trim(),
    getCurrentSession: () => ({ id: 'synthetic-session' }),
    refreshRequestTurnAnchors: () => assert.fail('released tasks cannot acquire a new active request'),
    scheduleTabObservation: (...args) => observations.push(args),
    diagnostic: () => {},
    send: message => f.messages.push(message),
  });
  await recovery.handleResponseRecoverLatest({ commandId: 'recover-original', reconcileConversation: f.expected });
  assert.equal(f.messages.length, 1);
  const response = f.messages[0];
  assert.equal(response.type, 'response.recovered');
  assert.equal(response.turnKey, assistantId, 'must reread after DOM ordinals are replaced by native IDs');
  assert.equal(response.messageId, assistantId);
  assert.equal(response.requestId, requestId);
  assert.equal(response.reconciliation.status, 'matched_complete');
  assert.equal(response.artifacts[0].requestId, requestId);
  assert.equal(response.artifacts[0].sourceTurnKey, assistantId);
  assert.equal(response.artifacts[0].downloadUrl, originalUrl);
  assert.equal(f.observed[0].turnKey, '');
  assert.ok(f.observed.some(snapshot => snapshot.turnKey === assistantId));
  assert.equal(observations.length, 1);
});
