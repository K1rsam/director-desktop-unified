import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { activeOwner, dispatch, ownedPreSendJournal, requestHash, reuseProvenEmptyPage, notSubmittedMessage, verifyOwnedNewPagePreflight } from './bridge.mjs';
import { composerAdapterSha256 } from './composer-transport.mjs';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
function fixture(t,{current=false,priorStable=false,priorDom=false}={}) {
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'director-old-terminal-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const request={requestId:'prior-owned-request',kind:'image',model:'chatgpt-image',prompt:'synthetic exact\n原文',ratio:'16:9',resolution:'native',references:[],options:{}};
  const adapter=priorDom?'34c8b2f90ab81e15dab5df56e5c21a89b2be1c1b9393cc4527bfa34c8a6b3766':priorStable?'5c850d49dba9164a61b499565be9eb21833da724aa6aad6f91a67ddbe86b717b':current?composerAdapterSha256:'0183767038c20ddebf218ddd297b2cf166aceee4d99bb7c03b7f73d7d3654e65';
  const check={page:'SYNTHETIC_PAGE',url:'https://chatgpt.com/',accountKey:'synthetic-account',userCount:0,assistantCount:0,composerEmpty:false,generating:false};
  const saved={version:'opencli-task/v1',requestId:request.requestId,requestHash:requestHash(request),request,profile:'synthetic-profile',accountKey:'synthetic-account',prompt:'Generate one image only. '+request.prompt,status:'not_submitted',createdAt:'2026-10-20T09:40:02.926Z',page:check.page,conversationUrl:null,
    ownedSend:{version:'owned-composer-send/v2',adapterSha256:adapter,token:request.requestId},error:'COMPOSER_TEXT_UNCONFIRMED',settledAt:'2026-10-20T09:40:07.918Z'};
  const before=Buffer.from(JSON.stringify({...saved,status:'preparing'})),beforeSha=hash(before),dir=path.join(root,request.requestId);fs.mkdirSync(dir);
  fs.writeFileSync(path.join(dir,`owned-pre-send-${beforeSha}.json`),before);
  saved.notSubmittedProof={version:'opencli-owned-pre-send-not-submitted/v2',sendProtocol:'owned-composer-send/v2',composerAdapterSha256:adapter,requestId:request.requestId,requestHash:saved.requestHash,
    runtimeSha256:priorDom?'21d7cc8069dd9563ecc2fa54299e777cb21fc96e87e97229025acc925639ba59':priorStable?'54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0':current?fs.readFileSync(new URL('./runtime.sha256',import.meta.url),'utf8').trim():'f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70',journalBeforeSha256:beforeSha,journalBeforeArchive:`owned-pre-send-${beforeSha}.json`,sendMarkerAbsent:true,sendConfirmedAbsent:true,conversationUrlAbsent:true,clickIntentAbsent:true,clickCalls:0,page:check.page,checks:[{...check},{...check}]};
  const file=path.join(dir,'request.json'),active=path.join(root,'active.json');
  const write=()=>fs.writeFileSync(file,JSON.stringify(saved,null,2));write();fs.writeFileSync(active,JSON.stringify({requestId:request.requestId,profile:saved.profile}));
  return{root,request,saved,file,active,loc:{root,active},write};
}
test('an exact prior owned no-click terminal opens the reservation gate without changing old records or calling a browser',async t=>{
  const f=fixture(t),before=fs.readFileSync(f.file),active=fs.readFileSync(f.active);
  assert.equal(ownedPreSendJournal(f.saved),false,'prior adapter never receives current settlement authority');
  assert.doesNotThrow(()=>activeOwner(f.loc,'new-request'));
  const result=await dispatch({operation:'query',config:{stateRoot:f.root,profile:f.saved.profile,accountKey:f.saved.accountKey},request:f.request});
  assert.equal(result.status,'failed');assert.equal(result.raw.opencliNotSubmitted.clickCalls,0);
  assert.equal(result.raw.opencliNotSubmitted.runtimeSha256,f.saved.notSubmittedProof.runtimeSha256);
  assert.deepEqual(fs.readFileSync(f.file),before);assert.deepEqual(fs.readFileSync(f.active),active);
  assert.equal(fs.existsSync(path.join(f.root,'new-request')),false);
});
test('prior unknown or preparing journals never acquire terminal or reservation-release authority',t=>{
  for(const status of ['submission_unknown','preparing','failed']){const f=fixture(t);f.saved.status=status;f.write();const before=fs.readFileSync(f.file);assert.throws(()=>activeOwner(f.loc,'new-request'),/BRIDGE_BUSY/);assert.deepEqual(fs.readFileSync(f.file),before);}
});
test('old terminal compatibility rejects every actual send marker including false, null and nested proof markers',t=>{
  for(const [key,value]of [['sentAt',0],['sentAt',null],['sendConfirmed',false],['sendIntent',null],['conversationUrl','https://chatgpt.com/c/synthetic']]){
    for(const nested of [false,true]){const f=fixture(t);if(nested)f.saved.notSubmittedProof.checks[0][key]=value;else f.saved[key]=value;f.write();const before=fs.readFileSync(f.file);assert.throws(()=>activeOwner(f.loc,'new-request'));assert.deepEqual(fs.readFileSync(f.file),before);}
  }
});
test('old terminal compatibility is exact and current-adapter terminal proof remains valid',t=>{
  for(const change of [f=>f.saved.notSubmittedProof.runtimeSha256='0'.repeat(64),f=>f.saved.notSubmittedProof.composerAdapterSha256='0'.repeat(64),f=>f.saved.ownedSend.token='other',f=>f.saved.notSubmittedProof.clickCalls=1,f=>f.saved.notSubmittedProof.sendMarkerAbsent=false,f=>f.saved.notSubmittedProof.checks[0].userCount=1,f=>f.saved.requestHash='0'.repeat(64)]){
    const f=fixture(t);change(f);f.write();assert.throws(()=>activeOwner(f.loc,'new-request'),/OWNED_NO_CLICK_UNPROVEN/);
  }
  const current=fixture(t,{current:true});assert.doesNotThrow(()=>activeOwner(current.loc,'new-request'));
  for(const pin of ['54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0','afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6','c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057','6b85411ff480f21dd3a36a0f26a33f2761a73d591cc810f58ff7beb0710d1aa2']){
    const previous=fixture(t,{priorStable:true});previous.saved.notSubmittedProof.runtimeSha256=pin;previous.write();assert.doesNotThrow(()=>activeOwner(previous.loc,'new-request'));assert.equal(ownedPreSendJournal(previous.saved),false);
  }
  current.saved.status='submission_unknown';current.write();assert.throws(()=>activeOwner(current.loc,'new-request'),/BRIDGE_BUSY/);
  current.saved.status='not_submitted';
  current.saved.notSubmittedProof.runtimeSha256='f'.repeat(64);current.write();assert.throws(()=>activeOwner(current.loc,'new-request'),/OWNED_NO_CLICK_UNPROVEN/);
});

test('passive DOM reader terminal proof survives a download-only reader upgrade',t=>{
  const f=fixture(t,{priorDom:true});f.saved.notSubmittedProof.runtimeSha256='5cee68750aef037cb816505a0e584efc5fb778eb1ab2aa1aca5b72c9964082ab';f.write();
  const before=fs.readFileSync(f.file);assert.doesNotThrow(()=>activeOwner(f.loc,'new-request'));assert.deepEqual(fs.readFileSync(f.file),before);
  f.saved.status='submission_unknown';f.write();assert.throws(()=>activeOwner(f.loc,'new-request'),/BRIDGE_BUSY/);
});

test('rendered-main upgrade preserves the exact prior adapter terminal but cannot settle its unknown journal',async t=>{
  const f=fixture(t,{priorDom:true}),before=fs.readFileSync(f.file),activeBefore=fs.readFileSync(f.active);
  assert.equal(ownedPreSendJournal(f.saved),false);assert.doesNotThrow(()=>activeOwner(f.loc,'new-request'));
  const result=await dispatch({operation:'query',config:{stateRoot:f.root,profile:f.saved.profile,accountKey:f.saved.accountKey},request:f.request});
  assert.equal(result.status,'failed');assert.equal(result.raw.opencliNotSubmitted.runtimeSha256,'21d7cc8069dd9563ecc2fa54299e777cb21fc96e87e97229025acc925639ba59');
  assert.equal(result.raw.opencliNotSubmitted.composerAdapterSha256,'34c8b2f90ab81e15dab5df56e5c21a89b2be1c1b9393cc4527bfa34c8a6b3766');
  assert.deepEqual(fs.readFileSync(f.file),before);assert.deepEqual(fs.readFileSync(f.active),activeBefore);
  f.saved.status='submission_unknown';f.write();assert.equal(ownedPreSendJournal(f.saved),false);assert.throws(()=>activeOwner(f.loc,'new-request'),/BRIDGE_BUSY/);
});

test('website-check no-send failure keeps its exact cause without blaming the prompt or rewriting the journal',async t=>{
  const f=fixture(t,{priorStable:true});f.saved.error='BROWSER_CHECK_REQUIRED';f.saved.notSubmittedProof.runtimeSha256='6b85411ff480f21dd3a36a0f26a33f2761a73d591cc810f58ff7beb0710d1aa2';f.write();
  const before=fs.readFileSync(f.file);
  const result=await dispatch({operation:'query',config:{stateRoot:f.root,profile:f.saved.profile,accountKey:f.saved.accountKey},request:f.request});
  assert.equal(result.raw.preSendFailureCode,'BROWSER_CHECK_REQUIRED');assert.match(result.error,/网站检查/);assert.match(result.error,/无需修改提示词/);
  assert.doesNotMatch(result.error,/提示词或发送按钮准备未通过/);assert.deepEqual(fs.readFileSync(f.file),before);
  assert.match(notSubmittedMessage('BROWSER_NETWORK_UNAVAILABLE'),/网络/);assert.doesNotMatch(notSubmittedMessage('unsafe private value'),/unsafe/);
});
function reuseFixture(t) {
  const f=fixture(t,{current:true}),request={...f.request,requestId:'new-request',options:{retryOfRequestId:f.request.requestId}};
  const view={url:'https://chatgpt.com/new',userCount:0,assistantCount:0,generating:false,visibleComposers:1,embeddedChips:0,
    ownerToken:f.request.requestId,logical:{ok:true,text:''}};
  let pageId='',reads=0,accounts=0;const counts=()=>({reads,accounts});
  const page={tabs:async()=>[{page:f.saved.page}],setActivePage:id=>{pageId=id;},getActivePage:()=>pageId,
    evaluate:async()=>{reads++;return structuredClone(view);}};
  const identity=async()=>{accounts++;return{accountKey:f.saved.accountKey};};
  return{...f,request,view,page,identity,counts,config:{stateRoot:f.root,profile:f.saved.profile,accountKey:f.saved.accountKey}};
}
test('a public retry reuses only twice-proven empty original page with no new tab, fill, click or old journal writes',async t=>{
  const f=reuseFixture(t),before=fs.readFileSync(f.file),activeBefore=fs.readFileSync(f.active);
  const reuse=await reuseProvenEmptyPage(f.config,f.request,f.page,f.identity);
  assert.equal(reuse.version,'opencli-reused-empty-page/v1');assert.equal(reuse.page,f.saved.page);assert.equal(reuse.previousRequestId,f.saved.requestId);
  assert.equal(reuse.previousJournalSha256,hash(before));assert.equal(reuse.clickCalls,0);assert.equal(reuse.checks.length,2);assert.deepEqual(f.counts(),{reads:2,accounts:2});
  assert.deepEqual(fs.readFileSync(f.file),before);assert.deepEqual(fs.readFileSync(f.active),activeBefore);assert.equal(fs.existsSync(path.join(f.root,'new-request')),false);
  const newFile=path.join(f.root,'new-observation.json'),next={requestId:f.request.requestId,page:f.saved.page,accountKey:f.saved.accountKey,prompt:'new complete prompt'};
  assert.equal((await verifyOwnedNewPagePreflight(f.page,next,{file:newFile},false,f.identity,reuse)).mode,'empty');
  const recorded=JSON.parse(fs.readFileSync(newFile));assert.equal(recorded.preSendValidation.newPageVerified,false);assert.equal(recorded.preSendValidation.pageOrigin,'proven_not_submitted_empty_page');
  assert.deepEqual(fs.readFileSync(f.file),before);
});
test('original empty page may be absent or nonempty but no unsafe page is reused',async t=>{
  const changes=[f=>f.page.tabs=async()=>[],f=>f.view.ownerToken='foreign-request',f=>f.view.logical.text='draft',f=>f.view.logical.text=' ',
    f=>f.view.userCount=1,f=>f.view.assistantCount=1,f=>f.view.generating=true,f=>f.view.visibleComposers=2,
    f=>f.view.embeddedChips=1,f=>f.view.logical={ok:false},f=>f.view.url='https://chatgpt.com/c/real-conversation',f=>f.view.url='https://chatgpt.com.evil.test/'];
  for(const change of changes){const f=reuseFixture(t);change(f);const before=fs.readFileSync(f.file);
    assert.equal(await reuseProvenEmptyPage(f.config,f.request,f.page,f.identity),null);assert.deepEqual(fs.readFileSync(f.file),before);}
});
test('unknowns, sends, foreign accounts, corrupted proof and changed journals cannot gain retry page ownership',async t=>{
  const changes=[f=>{f.saved.status='submission_unknown';f.write();},f=>{f.saved.sendIntent=null;f.write();},f=>{f.saved.notSubmittedProof.clickCalls=1;f.write();},
    f=>{f.saved.notSubmittedProof.runtimeSha256='f'.repeat(64);f.write();},f=>{f.config.profile='other';},f=>{f.config.accountKey='other';},
    f=>{f.identity=async()=>({accountKey:'other'});},f=>{f.identity=async()=>{throw Object.assign(new Error('BROWSER_CHECK_REQUIRED'),{code:'BROWSER_CHECK_REQUIRED'});};},
    f=>fs.writeFileSync(path.join(path.dirname(f.file),f.saved.notSubmittedProof.journalBeforeArchive),'altered'),
    f=>{let n=0;const read=f.page.evaluate;f.page.evaluate=async()=>{const view=await read();if(++n===2)fs.appendFileSync(f.file,' ');return view;};}];
  for(const change of changes){const f=reuseFixture(t);change(f);await assert.rejects(reuseProvenEmptyPage(f.config,f.request,f.page,f.identity));}
});
test('retry reference is optional and invalid references never create old state directories or query a browser',async t=>{
  const f=reuseFixture(t),before=fs.readdirSync(f.root).sort();
  assert.equal(await reuseProvenEmptyPage(f.config,{...f.request,options:{}},{},f.identity),null);
  assert.equal(await reuseProvenEmptyPage(f.config,{...f.request,options:{retryOfRequestId:'absent-id'}},{},f.identity),null);
  for(const id of ['../outside','new-request',null,5])await assert.rejects(reuseProvenEmptyPage(f.config,{...f.request,options:{retryOfRequestId:id}},{},f.identity),/INVALID_RETRY_IDENTITY/);
  assert.deepEqual(fs.readdirSync(f.root).sort(),before);assert.deepEqual(f.counts(),{reads:0,accounts:0});
});
