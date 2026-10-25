import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { activeOwner, dispatch, ownedPreSendJournal, requestHash } from './bridge.mjs';
import { composerAdapterSha256 } from './composer-transport.mjs';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
function fixture(t,{current=false,priorStable=false}={}) {
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'director-old-terminal-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const request={requestId:'prior-owned-request',kind:'image',model:'chatgpt-image',prompt:'synthetic exact\n原文',ratio:'16:9',resolution:'native',references:[],options:{}};
  const adapter=priorStable?'5c850d49dba9164a61b499565be9eb21833da724aa6aad6f91a67ddbe86b717b':current?composerAdapterSha256:'0183767038c20ddebf218ddd297b2cf166aceee4d99bb7c03b7f73d7d3654e65';
  const check={page:'SYNTHETIC_PAGE',url:'https://chatgpt.com/',accountKey:'synthetic-account',userCount:0,assistantCount:0,composerEmpty:false,generating:false};
  const saved={version:'opencli-task/v1',requestId:request.requestId,requestHash:requestHash(request),request,profile:'synthetic-profile',accountKey:'synthetic-account',prompt:'Generate one image only. '+request.prompt,status:'not_submitted',createdAt:'2026-10-20T09:40:02.926Z',page:check.page,conversationUrl:null,
    ownedSend:{version:'owned-composer-send/v2',adapterSha256:adapter,token:request.requestId},error:'COMPOSER_TEXT_UNCONFIRMED',settledAt:'2026-10-20T09:40:07.918Z'};
  const before=Buffer.from(JSON.stringify({...saved,status:'preparing'})),beforeSha=hash(before),dir=path.join(root,request.requestId);fs.mkdirSync(dir);
  fs.writeFileSync(path.join(dir,`owned-pre-send-${beforeSha}.json`),before);
  saved.notSubmittedProof={version:'opencli-owned-pre-send-not-submitted/v2',sendProtocol:'owned-composer-send/v2',composerAdapterSha256:adapter,requestId:request.requestId,requestHash:saved.requestHash,
    runtimeSha256:priorStable?'54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0':current?fs.readFileSync(new URL('./runtime.sha256',import.meta.url),'utf8').trim():'f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70',journalBeforeSha256:beforeSha,journalBeforeArchive:`owned-pre-send-${beforeSha}.json`,sendMarkerAbsent:true,sendConfirmedAbsent:true,conversationUrlAbsent:true,clickIntentAbsent:true,clickCalls:0,page:check.page,checks:[{...check},{...check}]};
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
