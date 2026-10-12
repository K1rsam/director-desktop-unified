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
  const fixturePrompt=options.markerless?'请生成一张原图。':prompt;
  let active = options.active || null;
  let launch = options.launch === undefined ? `director-desktop-${requestId}` : options.launch;
  const calls = [], observed = [], messages = [];
  const image = { isConnected: true, src: options.blob ? 'blob:https://chatgpt.com/synthetic' : '', currentSrc: '', naturalWidth: 0, naturalHeight: 0, closest: () => ({}), getAttribute: () => '' };
  const user = { isConnected: true, innerText: options.domPrompt || fixturePrompt, querySelectorAll: () => [] };
  if(options.presentationControls){
    user.innerText=prompt+'\n显示更多';user.matches=selector=>selector==='.bg-user-message';
    const body={isConnected:true,textContent:options.bodyPrompt||prompt,querySelector:()=>null};
    user.querySelectorAll=selector=>selector==='div.whitespace-pre-wrap'?(options.multiplePromptBodies?[body,{...body}]:[body]):[];
    if(options.changeBodyDuringDownload)options.onBody=()=>{body.textContent='changed source text';};
  }
  const assistant = { isConnected: true, innerText: options.reply||'', querySelectorAll: () => options.noImage?[]:[image] };
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
        value.mapping['user-node'].message.content.parts=[fixturePrompt];
        if(options.noImage)value.mapping['assistant-node'].message.content={content_type:'text',parts:[options.reply||'未能生成图片。']};
        if(options.noAssistant){value.current_node='user-node';delete value.mapping['assistant-node'];}
        reads++;
        if(reads===2&&options.replyAfterRecheck){options.reply=options.replyAfterRecheck;assistant.innerText=options.reply;}
        if(reads===2&&options.oversizedRecheck)return new Response(new Uint8Array(4*1024*1024+1));
        if (options.backendPrompt) value.mapping['user-node'].message.content.parts = [options.backendPrompt];
        if (options.status) value.mapping['assistant-node'].message.status = options.status;
        options.recordTransform?.(value);
        if (reads === 2 && options.changedBranch) value.mapping['assistant-node'].message.id = 'different-assistant';
        return new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json' } });
      }
      if (url.endsWith('/api/auth/session')) return Response.json({ accessToken: 'synthetic-token-never-persisted' });
      if (url.includes(`/backend-api/files/download/${imageId}`)) return Response.json({ download_url: options.wrongFile ? originalUrl.replace(imageId, 'file-other') : originalUrl });
      if (url === originalUrl || url === image.src && options.blob) {
        options.onBody?.();
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
    const value={ answer: options.noImage?(options.reply||'未能生成图片。'):'', artifacts: proof.imageSource(image) && (!options.hidden || proof.imageReady(image)) ? [{ id: options.artifactIdsFollowTurnKey ? (proof.key(assistant)?'canonical-artifact':'observed-artifact') : 'original-artifact', kind: 'image', generatedImage: true, downloadable:true, mime:'image/png',size:png.length, downloadUrl: proof.imageSource(image) }] : [], turnKey: proof.key(assistant) || 'dom-turn-ordinal', userTurnKey:proof.key(user), messageId: proof.key(assistant), hasFinalMessage: true };
    options.snapshotTransform?.(value,{nativeBound:!!proof.key(assistant),user,image});
    return value;
  };
  const expected = { mode: 'image_record_v1', requestId, conversationId, promptSha256: await proof.digest(fixturePrompt), imageCount: 1, allowReleasedRequest: options.allowReleasedRequest !== false, allowReloadedOriginalTab: options.allowReloadedOriginalTab === true };
  if(options.readOnlyRecoveryLaunch)expected.readOnlyRecoveryLaunch=options.readOnlyRecoveryLaunch;
  if(options.savedConversation)expected.savedConversation={version:'chrome-conversation-locator/v1',requestId,conversationId,promptSha256:expected.promptSha256,userMessageId:userId,...options.savedConversation};
  const deps = { getActiveRequest: () => active, getBrowserLaunchToken: () => launch, getContentEpoch: () => 'content-epoch', readLatestAssistantSnapshot: snapshot, onBound: () => options.onBound?.({user,assistant,image}) };
  return { context, proof, expected, deps, snapshot, calls, observed, messages, user, assistant, image, get reads() { return reads; } };
}

test('verified final text without an image is a stable terminal receipt, never an image result',async()=>{
  const f=await fixture({noImage:true,reply:'由于我这边发生了错误，我未能生成图片。'});
  const p=await f.proof.read(f.expected,f.snapshot(),f.deps);
  assert.equal(p.status,'matched_no_image');assert.equal(p.backendImageCount,0);assert.equal(p.userMessageId,userId);assert.equal(p.assistantMessageId,assistantId);
  assert.equal(p.recordSha256,p.recordRecheckSha256);assert.equal(f.reads,2);assert.equal(p.bindings.length,0);
  assert.ok(f.calls.every(c=>c.init.method==='GET'));assert.ok(!f.calls.some(c=>c.url.includes('/files/download/')));
});
test('no-image settlement rejects a changed branch, streaming, text mismatch, extra turns, or diagnostic-only read',async()=>{
  for(const options of [{changedBranch:true},{replyAfterRecheck:'late changed answer'},{snapshotTransform:s=>{s.streamingVisible=true}},{snapshotTransform:s=>{s.answer='different'}},{extraUser:true},{extraAssistant:true}]){
    const f=await fixture({noImage:true,...options});const p=await f.proof.read(f.expected,f.snapshot(),f.deps);assert.notEqual(p.status,'matched_no_image');
  }
  const f=await fixture({noImage:true});f.expected.mode='image_record_probe';assert.notEqual((await f.proof.read(f.expected,f.snapshot(),f.deps)).status,'matched_no_image');
});
test('saved exact conversation and native user identity recover a marker-free image after release',async()=>{
  const f=await fixture({markerless:true,savedConversation:{},artifactIdsFollowTurnKey:true});const p=await f.proof.read(f.expected,f.snapshot(),f.deps);
  assert.equal(p.status,'matched_complete');assert.equal(p.userMessageId,userId);assert.equal(f.reads,2);
  for(const savedConversation of [{userMessageId:'other'},{requestId:'other'},{conversationId:'other'},{promptSha256:'a'.repeat(64)}]){
    const bad=await fixture({markerless:true,savedConversation});assert.notEqual((await bad.proof.read(bad.expected,bad.snapshot(),bad.deps)).status,'matched_complete');
  }
  const missing=await fixture({markerless:true});assert.equal((await missing.proof.read(missing.expected,missing.snapshot(),missing.deps)).reason,'released_task_marker');
});
test('pending native user binding is retained while the image is still rendering',async()=>{
  const f=await fixture({snapshotTransform:s=>{s.streamingVisible=true}});const p=await f.proof.read(f.expected,f.snapshot(),f.deps);
  assert.equal(p.status,'matched_incomplete');assert.equal(p.userMessageId,userId);assert.equal(p.backendPromptVerified,true);assert.equal(f.reads,1);
});
test('saved marker-free conversation is readable in a separate recovery alias without replacing original request identity',async()=>{
  const alias='bridge-director-desktop-22222222-2222-4222-8222-222222222222';
  const f=await fixture({markerless:true,savedConversation:{},launch:alias,readOnlyRecoveryLaunch:alias});
  const p=await f.proof.read(f.expected,f.snapshot(),f.deps);assert.equal(p.status,'matched_complete');assert.equal(p.requestId,requestId);
  assert.ok(f.calls.every(c=>c.init.method==='GET'));
  for(const options of [{launch:alias,readOnlyRecoveryLaunch:'bridge-director-desktop-33333333-3333-4333-8333-333333333333'},
    {launch:alias},{launch:'user-page',readOnlyRecoveryLaunch:'user-page'},{launch:alias,readOnlyRecoveryLaunch:alias,savedConversation:{requestId:'other'}}]){
    const bad=await fixture({markerless:true,savedConversation:{},...options});assert.equal((await bad.proof.read(bad.expected,bad.snapshot(),bad.deps)).reason,'active_request_identity');assert.equal(bad.calls.length,0);
  }
});
test('no-image second record read is bounded before parsing or settling',async()=>{
  const f=await fixture({noImage:true,oversizedRecheck:true});const p=await f.proof.read(f.expected,f.snapshot(),f.deps);
  assert.equal(p.status,'unavailable');assert.equal(p.reason,'record_limit');assert.equal(f.reads,2);
});
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

test('presentation bubble controls do not enter full prompt hash or prevent original image recovery',async()=>{
 const f=await fixture({presentationControls:true});const result=await f.proof.read(f.expected,f.snapshot(),f.deps);
 assert.equal(result.status,'matched_complete');assert.equal(result.promptSha256,f.expected.promptSha256);assert.equal(f.reads,2);
});
test('presentation prompt extraction still rejects changed multiple or drifting source bodies',async()=>{
 for(const options of [{presentationControls:true,bodyPrompt:'different prompt'},{presentationControls:true,multiplePromptBodies:true},{presentationControls:true,changeBodyDuringDownload:true}]){
  const f=await fixture(options);assert.notEqual((await f.proof.read(f.expected,f.snapshot(),f.deps)).status,'matched_complete');
 }
});

test('latest image delivery survives intermediate renders and user reference images, with exact bytes rechecked', async () => {
  const f = await fixture({recordTransform: value => {
    value.mapping['user-node'].message.content.parts.push({asset_pointer:'sediment://file-reference'});
    value.mapping['old-render']={parent:'user-node',message:{id:'old-render',author:{role:'tool'},status:'finished_successfully',content:{parts:[{asset_pointer:'sediment://file-intermediate'}]}}};
    value.mapping['latest-render']={parent:'old-render',message:{id:'latest-render',author:{role:'tool'},status:'finished_successfully',content:{parts:[{asset_pointer:`sediment://${imageId}`}]}}};
    value.mapping['assistant-node'].parent='latest-render';
    value.mapping['assistant-node'].message.content={content_type:'text',parts:['']};
  }});
  const r=await f.proof.read(f.expected,f.snapshot(),f.deps);
  assert.equal(r.status,'matched_complete');assert.equal(r.deliveryMessageId,'latest-render');
  assert.equal(r.bindings[0].imageFileId,imageId);assert.equal(r.bindings[0].bytesSha256,hash(png));
  assert.equal(f.reads,2);assert.equal(r.branchIdentity[0].nodeKey,'assistant-node');
});

test('latest multiple, incomplete, changed or wrong-branch image delivery never falls back to an older matching image', async () => {
  for(const mode of ['multiple','incomplete','wrong-latest','wrong-branch']){
    const f=await fixture({recordTransform:value=>{
      value.mapping['latest-render']={parent:'assistant-node',message:{id:'latest-render',author:{role:'tool'},status:mode==='incomplete'?'in_progress':'finished_successfully',content:{parts:[{asset_pointer:'sediment://file-other'},...(mode==='multiple'?[{asset_pointer:`sediment://${imageId}`}]:[])]}}};
      value.mapping['final']={parent:'latest-render',message:{...value.mapping['assistant-node'].message,id:'new-final',content:{content_type:'text',parts:['']}}};
      value.current_node=mode==='wrong-branch'?'latest-render':'final';
    }});
    const r=await f.proof.read(f.expected,f.snapshot(),f.deps);
    assert.notEqual(r.status,'matched_complete',mode);assert.equal(f.proof.imageReady(f.image),false,mode);
    assert.equal(f.proof.downloadBytes({sourceTurnKey:assistantId,requestId},originalUrl),null,mode);
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

test('diagnostic cached blob bytes cannot poison later background recovery or grant adoption by themselves', async () => {
  const f=await fixture({blob:true,hidden:true});
  const diagnostic=await f.proof.read({...f.expected,mode:'image_record_probe'},f.snapshot(),f.deps);
  assert.notEqual(diagnostic.status,'matched_complete');assert.equal(f.proof.imageReady(f.image),false);
  const priorReads=f.reads;
  const real=await f.proof.read(f.expected,f.snapshot(),f.deps);
  assert.equal(real.status,'matched_complete');assert.equal(f.proof.imageReady(f.image),true);
  assert.equal(real.bindings[0].bytesSha256,hash(png));assert.equal(f.reads,priorReads+2,'real recovery must authenticate and reread the branch again');
});

test('response recovery rereads the proof-bound snapshot and returns original request and native turn identity', async () => {
  const f = await fixture({blob:true,artifactIdsFollowTurnKey:true});
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
  assert.equal(response.artifacts[0].id,'canonical-artifact');
  assert.equal(response.reconciliation.bindings[0].artifactId,response.artifacts[0].id);
  assert.equal(response.reconciliation.bindings[0].originalObservedArtifactId,'observed-artifact');
  assert.equal(f.observed[0].turnKey, '');
  assert.ok(f.observed.some(snapshot => snapshot.turnKey === assistantId));
  assert.equal(observations.length, 1);
});

test('native key rebinding publishes one detached final snapshot and preserves its original observed artifact ID',async()=>{
  const f=await fixture({blob:true,artifactIdsFollowTurnKey:true});let delivered;
  const proof=await f.proof.read(f.expected,f.snapshot(),{...f.deps,onVerifiedSnapshot:value=>{delivered=value;}});
  assert.equal(proof.status,'matched_complete');
  assert.equal(proof.bindings[0].artifactId,'canonical-artifact');
  assert.equal(proof.bindings[0].originalObservedArtifactId,'observed-artifact');
  assert.equal(delivered.artifacts[0].id,proof.bindings[0].artifactId);
  assert.equal(delivered.turnKey,assistantId);assert.equal(delivered.userTurnKey,userId);
  assert.equal(proof.bindings[0].bytesSha256,hash(png));assert.equal(f.reads,2,'same record, no extra backend generation or lookup');
  f.user.innerText='edited after transfer';
  assert.equal(delivered.artifacts[0].id,'canonical-artifact','detached publication remains the verified snapshot');
  assert.equal(f.proof.downloadBytes({sourceTurnKey:assistantId,requestId},originalUrl),null,'live byte binding is still revoked by edits');
});

test('final key rebinding rejects switched files, duplicate images and changed DOM without delivering a snapshot',async()=>{
  for(const change of ['file','duplicate','prompt']){
    const f=await fixture({blob:true,artifactIdsFollowTurnKey:true,
      onBound:change==='prompt'?({user})=>{user.innerText='changed at native bind'}:undefined,
      snapshotTransform:(value,{nativeBound})=>{if(!nativeBound||!value.artifacts.length)return;
        if(change==='file')value.artifacts[0].downloadUrl=originalUrl.replace(imageId,'file-other');
        if(change==='duplicate')value.artifacts.push({...value.artifacts[0],id:'another-artifact'});
      }});
    let delivered=false;
    const proof=await f.proof.read(f.expected,f.snapshot(),{...f.deps,onVerifiedSnapshot:()=>{delivered=true;}});
    assert.notEqual(proof.status,'matched_complete',change);assert.equal(delivered,false,change);
  }
});

test('final coherent recovery reaches the unchanged strict native completion guard',async()=>{
  const f=await fixture({blob:true,artifactIdsFollowTurnKey:true,launch:'bridge-director-desktop-'+requestId});let snapshot;
  const proof=await f.proof.read(f.expected,f.snapshot(),{...f.deps,onVerifiedSnapshot:value=>{snapshot=value;}});
  const source=fs.readFileSync(path.join(__dirname,'../overrides/src/browserBridge.js'),'utf8');
  const body=source.slice(source.indexOf('export function nativeImageCompletionPayload'),source.indexOf('export class BrowserBridge')).replace('export function','function');
  const guardContext=vm.createContext({createHash,URL});vm.runInContext(body+';this.complete=nativeImageCompletionPayload',guardContext);
  const identity={requestId,leaseId:'original-lease',ownerServerInstanceId:'original-server',responseEpoch:0};
  const client={id:'owned-source',ready:true,compatible:true,launchToken:'bridge-director-desktop-'+requestId,url:`https://chatgpt.com/c/${conversationId}`,activeRequest:identity};
  const state={requestId,clientId:client.id,promptPayload:{requestId,message:prompt},runtime:{finished:false,cancellationRequested:false}};
  const response={...snapshot,requestId,sourceClientId:client.id,url:client.url,session:{id:conversationId},reconciliation:proof,
    artifacts:snapshot.artifacts.map(a=>({...a,requestId,sourceClientId:client.id,sourceTurnKey:assistantId}))};
  const options={sourceClientId:client.id,reconcileConversation:f.expected};
  assert.equal(guardContext.complete(state,identity,client,options,response).finishReason,'native_image_record_complete');
  const wrong=JSON.parse(JSON.stringify(response));wrong.artifacts[0].id=proof.bindings[0].originalObservedArtifactId;
  assert.equal(guardContext.complete(state,identity,client,options,wrong),null,'old artifact identity remains rejected');
});

test('request.snapshot also returns the same verified final artifact identity instead of its pre-bind observation',async()=>{
  const f=await fixture({blob:true,artifactIdsFollowTurnKey:true});
  vm.runInContext(script('responseRecovery.js'),f.context);
  const recovery=f.context.ChatGptResponseRecovery.createResponseRecovery({...f.deps,
    readAssistantSnapshotByTurnKey:()=>f.snapshot(),readRecentAssistantSnapshots:()=>[f.snapshot()],
    getCurrentSession:()=>({id:conversationId}),normalizeText:String,isGenerating:()=>false,findStopButton:()=>null,
    scheduleTabObservation:()=>{},diagnostic:()=>{},send:value=>f.messages.push(value)});
  await recovery.handleResponseSnapshotRequest({commandId:'snapshot-original',requestId,turnKey:assistantId,reconcileConversation:f.expected});
  assert.equal(f.messages[0].type,'request.snapshot');
  assert.equal(f.messages[0].artifacts[0].id,'canonical-artifact');
  assert.equal(f.messages[0].reconciliation.bindings[0].artifactId,f.messages[0].artifacts[0].id);
  assert.equal(f.messages[0].reconciliation.bindings[0].originalObservedArtifactId,'observed-artifact');
});
