import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';
import { createHash } from 'node:crypto';
import { OWNED_SEND_PROTOCOL, composerAdapterSha256, prepareComposerCode, clickComposerCode, clickOwnedComposer } from './composer-transport.mjs';
import { requestHash, submitOwnedComposer, ownedPreSendJournal, settleOwnedPreSendFailure } from './bridge.mjs';
const hash=s=>createHash('sha256').update(s).digest('hex');
function fixture(t) {
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'director-owned-send-'));
  t.after(()=>fs.rmSync(directory,{recursive:true,force:true}));
  const request={requestId:'owned-fixture',kind:'image',model:'chatgpt-image',prompt:'exact\n原文',references:[]};
  const saved={version:'opencli-task/v1',requestId:request.requestId,requestHash:requestHash(request),request,prompt:request.prompt,status:'preparing',page:'OWNED_PAGE',profile:'profile',accountKey:'account',conversationUrl:null,ownedSend:{version:OWNED_SEND_PROTOCOL,adapterSha256:composerAdapterSha256,token:request.requestId}};
  const loc={file:path.join(directory,'request.json')};fs.writeFileSync(loc.file,JSON.stringify(saved));return{request,saved,loc};
}
function pageFor(saved,loc,{filled=true,ready=true,confirm=true,lostClickAck=false}={}) {
  const state={clickCalls:0,fillCalls:0,intentObserved:false};
  return{state,async sleep(){},async fillText(_selector,text){state.fillCalls++;return{verified:filled,actual:filled?text:''};},async evaluate(code){
    if(code.includes("reason:'composer_not_unique'"))return{ok:true};
    if(code.includes('return{ok:actual.ok,token:s.token,prompt:actual.text}'))return{ok:filled,token:saved.requestId,prompt:filled?saved.prompt:''};
    if(code.includes("reason:'send_button_not_unique_or_ready'"))return ready?{ok:true,token:saved.requestId,prompt:saved.prompt,url:'https://chatgpt.com/'}:{ok:false};
    if(code.includes('s.clicked=true;')){state.intentObserved=!!JSON.parse(fs.readFileSync(loc.file)).sendIntent;assert.equal(state.intentObserved,true);state.clickCalls++;if(lostClickAck)throw Error('synthetic click ack lost');return{clicked:true};}
    if(code.includes('const readOwnedConversationDom='))return{url:confirm?'https://chatgpt.com/c/1234-abcd':'https://chatgpt.com/',userCount:confirm?1:0,promptMatched:confirm};
    throw Error('Unexpected synthetic evaluation');
  }};
}
test('fill mismatch and unavailable send button never persist intent or call click',async t=>{
  for(const options of [{filled:false},{ready:false}]){const{saved,loc}=fixture(t),page=pageFor(saved,loc,options);await assert.rejects(submitOwnedComposer(page,saved,loc));assert.equal(page.state.clickCalls,0);const journal=JSON.parse(fs.readFileSync(loc.file));assert.equal(Object.hasOwn(journal,'sentAt'),false);assert.equal(Object.hasOwn(journal,'sendIntent'),false);}
});
test('one click follows durable intent and confirmation requires original prompt and conversation',async t=>{
  const{saved,loc}=fixture(t),page=pageFor(saved,loc);await submitOwnedComposer(page,saved,loc);assert.equal(page.state.clickCalls,1);assert.equal(saved.sendConfirmed,true);assert.equal(saved.conversationUrl,'https://chatgpt.com/c/1234-abcd');
});
test('click acknowledgement loss or absent original prompt retains intent and cannot obtain no-click settlement',async t=>{
  for(const options of [{lostClickAck:true},{confirm:false}]){const{request,saved,loc}=fixture(t),page=pageFor(saved,loc,options);await assert.rejects(submitOwnedComposer(page,saved,loc));assert.equal(page.state.clickCalls,1);assert.equal(ownedPreSendJournal(saved),false);assert.equal(await settleOwnedPreSendFailure({},request,saved,loc,null,()=>assert.fail('no observation after intent')),null);}
});
test('owned no-click proof accepts partial composer without altering original request and rejects legacy send markers',async t=>{
  const{request,saved,loc}=fixture(t);const check={page:saved.page,url:'https://chatgpt.com/',accountKey:saved.accountKey,userCount:0,assistantCount:0,composerEmpty:false,generating:false};
  const result=await settleOwnedPreSendFailure({runtimeSha256:fs.readFileSync(new URL('./runtime.sha256',import.meta.url),'utf8').trim()},request,saved,loc,null,async()=>({...check}));assert.equal(result.status,'failed');assert.equal(result.raw.opencliNotSubmitted.clickCalls,0);assert.deepEqual(JSON.parse(fs.readFileSync(loc.file)).request,request);
  for(const patch of [{sentAt:''},{sendConfirmed:false},{sendIntent:null}]){const f=fixture(t);Object.assign(f.saved,patch);assert.equal(await settleOwnedPreSendFailure({},f.request,f.saved,f.loc,null,()=>assert.fail()),null);}
  const legacy=fixture(t);delete legacy.saved.ownedSend;assert.equal(ownedPreSendJournal(legacy.saved),false);
});
class Element {isConnected=true;isContentEditable=true;innerText='exact';disabled=false;childNodes=[{nodeType:1,tagName:'P',childNodes:[{nodeType:3,data:'exact'}],getAttribute(){return null;}}];querySelector(){return null;}getClientRects(){return[{}];}closest(){return null;}getAttribute(){return null;}}
test('embedded reference or tool chips are rejected before any fill mutation',()=>{
  const composer=new Element();composer.querySelector=()=>({referenceChip:true});composer.setAttribute=()=>assert.fail('must not mutate embedded chips');const context={HTMLElement:Element,getComputedStyle:()=>({display:'block',visibility:'visible'}),document:{querySelectorAll:()=>[composer]}};
  assert.equal(vm.runInNewContext(prepareComposerCode('exact','request'),context).reason,'embedded_content_requires_review');
});
test('page-local click token survives lost ACK while owner loss or navigation cannot click again',async()=>{
  let clicks=0;const composer=new Element(),button=new Element();button.click=()=>{clicks++;};const gate={version:OWNED_SEND_PROTOCOL,token:'request',promptSha256:hash('exact'),adapterSha256:composerAdapterSha256};
  const owner={version:OWNED_SEND_PROTOCOL,token:'request',prompt:'exact',composer,button,clicked:false};const context=vm.createContext({HTMLElement:Element,getComputedStyle:()=>({display:'block',visibility:'visible'}),__director_owned_composer_request:owner});const code=clickComposerCode('exact',gate);
  assert.equal(vm.runInContext(code,context).clicked,true);button.isConnected=false;assert.equal(vm.runInContext(code,context).recovered,true);assert.equal(clicks,1);
  delete context.__director_owned_composer_request;assert.equal(vm.runInContext(code,context).clicked,false);assert.equal(clicks,1);
  await assert.rejects(clickOwnedComposer({evaluate:()=>assert.fail()},'exact',gate,null),/CLICK_INTENT_UNBOUND/);
});
