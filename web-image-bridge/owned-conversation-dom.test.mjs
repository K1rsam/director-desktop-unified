import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {createHash,webcrypto} from 'node:crypto';
import {ownedConversationStateCode,observeOwnedSubmission,OWNED_SEND_PROTOCOL} from './composer-transport.mjs';
import {activeCode,originalImageReadCode,recordPassiveObservation} from './bridge.mjs';
const url='https://chatgpt.com/c/1234-abcd',prompt='Generate one image only. exact\n完整正文\n保留全部字符';
const hash=s=>createHash('sha256').update(s).digest('hex');
// Minimal DOM with real containment/document-order semantics. No browser,
// account, prompt submission, network, or production journal is used here.
class Element {
  constructor(tag,attrs={},children=[]) {this.nodeType=1;this.tagName=tag.toUpperCase();this.attrs=attrs;this.childNodes=[];this.disabled=false;for(const child of children)this.append(child);}
  append(child) {if(typeof child==='string')child={nodeType:3,data:child};child.parentElement=this;this.childNodes.push(child);return child;}
  get children(){return this.childNodes.filter(n=>n.nodeType===1);}
  get textContent(){return this.childNodes.map(n=>n.nodeType===3?n.data:n.textContent).join('');}
  getAttribute(k){return this.attrs[k]??null;}
  matches(selector){
    return selector.split(',').some(s=>{
      s=s.trim();const tag=s.match(/^[a-z]+/i)?.[0];if(tag&&this.tagName!==tag.toUpperCase())return false;
      for(const m of s.matchAll(/\.([a-z0-9_-]+)/gi))if(!(this.attrs.class||'').split(' ').includes(m[1]))return false;
      for(const m of s.matchAll(/\[([^=\]*]+)(\*=|=)?(?:"([^"]*)"|([^\]]+))?\]/g)){
        const value=this.getAttribute(m[1]),wanted=m[3]??m[4];if(value===null)return false;
        if(m[2]==='='&&value!==wanted)return false;if(m[2]==='*='&&!value.includes(wanted))return false;
      }return true;
    });
  }
  querySelectorAll(selector){
    const descendants=[];const walk=n=>{for(const c of n.children){descendants.push(c);walk(c);}};walk(this);
    return descendants.filter(n=>selector.split(',').some(s=>{
      s=s.trim();if(s.endsWith(' img'))return n.tagName==='IMG'&&!!n.parentElement?.closest(s.slice(0,-4));
      return n.matches(s);
    }));
  }
  closest(selector){for(let n=this;n;n=n.parentElement)if(n.matches(selector))return n;return null;}
  contains(n){for(;n;n=n.parentElement)if(n===this)return true;return false;}
  compareDocumentPosition(n){let root=this;while(root.parentElement)root=root.parentElement;if(!root.contains(n))return 1;const nodes=[root,...root.querySelectorAll('*')];return nodes.indexOf(this)<nodes.indexOf(n)?4:2;}
  getClientRects(){return this.hiddenRects?[]:[{}];}
}
const e=(tag,attrs={},children=[])=>new Element(tag,attrs,children);
function fixture({nested=true,collapse=true,gallery=true}={}){
  const text=e('div',{class:'whitespace-pre-wrap'},[prompt]),bubble=e('div',{class:'bg-user-message'},[text]);
  if(collapse){bubble.append(e('span',{},['…']));bubble.append(e('button',{},['显示更多']));}
  const user=nested?e('article',{'data-turn':'user'},[e('div',{'data-message-author-role':'user'},[bubble])]):bubble;
  const img=e('img');Object.assign(img,{complete:true,naturalWidth:1672,naturalHeight:941,src:'blob:https://chatgpt.com/generated-one'});
  const preview=e('button',{'data-testid':'generated-image-preview'},[img]);
  const imageNode=gallery?e('div',{'data-testid':'generated-image-gallery'},[preview]):preview;
  const main=e('main',{},[user,imageNode]),document=e('document',{},[main]);
  const context={document,location:{href:url,origin:'https://chatgpt.com'},getComputedStyle:()=>({visibility:'visible'})};
  return {text,bubble,user,img,preview,imageNode,main,document,context};
}
const state=f=>vm.runInNewContext(ownedConversationStateCode(prompt,activeCode),f.context);
test('nested user selectors and Show more UI preserve the exact logical prompt',()=>{
  const f=fixture(),s=state(f);assert.equal(s.userCount,1);assert.equal(s.promptMatched,true);assert.equal(s.count,1);assert.equal(s.imageCandidates,1);assert.notEqual(f.bubble.textContent,prompt);
});
test('new role-less bubble and preview-only image are recognized without assistant roles',()=>{
  const s=state(fixture({nested:false,gallery:false}));assert.equal(s.userCount,1);assert.equal(s.promptMatched,true);assert.equal(s.count,1);
});
test('old plain user message remains readable, but unknown structured message is held',()=>{
  const f=fixture();f.main.childNodes[0]=e('div',{'data-message-author-role':'user'},[prompt]);f.main.childNodes[0].parentElement=f.main;
  assert.equal(state(f).promptMatched,true);f.main.childNodes[0].childNodes=[e('em',{},[prompt])];assert.equal(state(f).promptMatched,false);
});
test('prefix, suffix, whitespace changes, multiple bodies and multiple real users cannot pass',()=>{
  for(const changed of [prompt+' ',prompt+' extra','prefix '+prompt,prompt.replace('\n',' '),'']){const f=fixture();f.text.childNodes[0].data=changed;assert.equal(state(f).promptMatched,false);}
  const f=fixture();f.bubble.append(e('div',{class:'whitespace-pre-wrap'},['extra']));assert.equal(state(f).promptMatched,false);
  const g=fixture();g.main.append(e('div',{'data-turn':'user'},[prompt]));assert.equal(state(g).userCount,2);assert.equal(state(g).count,0);
});
test('user attachments, earlier images, other blobs, dialogs and sidebars cannot become the generated image',()=>{
  const f=fixture();const extra=()=>{const n=e('img');Object.assign(n,{complete:true,naturalWidth:500,naturalHeight:500,src:'blob:unrelated'});return n;};
  f.bubble.append(e('div',{'data-testid':'generated-image-gallery'},[extra()]));f.main.append(extra());
  f.main.append(e('aside',{},[e('div',{'data-testid':'generated-image-gallery'},[extra()])]));
  f.main.append(e('div',{role:'dialog'},[e('button',{'data-testid':'generated-image-preview'},[extra()])]));
  const before=e('div',{'data-testid':'generated-image-gallery'},[extra()]);before.parentElement=f.main;f.main.childNodes.unshift(before);
  assert.equal(state(f).count,1);assert.equal(state(f).imageCandidates,1);
});
test('a second unfinished generated image is counted and does not permit early single-image recovery',()=>{
  const f=fixture(),loading=e('img');Object.assign(loading,{complete:false,naturalWidth:0,naturalHeight:0});f.imageNode.append(loading);
  assert.equal(state(f).count,1);assert.equal(state(f).imageCandidates,2);
});
test('only visible live generation controls block; hidden stop and voice start do not',()=>{
  const f=fixture(),stop=e('button',{'data-testid':'stop-button'});stop.hiddenRects=true;f.main.append(stop);f.main.append(e('button',{'aria-label':'开始语音'}));assert.equal(state(f).generating,false);
  stop.hiddenRects=false;assert.equal(state(f).generating,true);stop.attrs['aria-hidden']='true';assert.equal(state(f).generating,false);
});
test('send confirmation uses the same exact logical body and stable conversation, without clicking',async()=>{
  const f=fixture(),page={evaluate:code=>vm.runInNewContext(code,f.context),sleep:async()=>{}};
  assert.equal((await observeOwnedSubmission(page,prompt)).promptSha256,hash(prompt));f.context.location.href='https://chatgpt.com/new';
  await assert.rejects(observeOwnedSubmission(page,prompt),/SENT_PROMPT_UNCONFIRMED/);
});
function originalContext(f,{multiple=false,wrong=false,changed=false,busy=false}={}){
  const bytes=Buffer.from('synthetic-original-image-bytes'),file='https://chatgpt.com/backend-api/files/download/file_one';
  const calls=[];const context={...f.context,URL,Blob,TextEncoder,crypto:webcrypto,AbortSignal,performance:{getEntriesByType:()=>[{name:file},...(multiple?[{name:file.replace('one','two')}]:[])]},
    FileReader:class{readAsDataURL(blob){blob.arrayBuffer().then(b=>{this.result='data:image/png;base64,'+Buffer.from(b).toString('base64');this.onload();});}},
    fetch:async(u,options={})=>{calls.push(u);assert.equal(options.method,undefined);if(u.startsWith('blob:'))return{ok:true,arrayBuffer:async()=>bytes};
      if(u==='/api/auth/session')return{ok:true,json:async()=>({accessToken:'synthetic-secret-never-returned',user:{id:'synthetic-user'}})};
      if(u.startsWith('https://chatgpt.com/backend-api/files/download/'))return{ok:true,json:async()=>({download_url:'https://files.oaiusercontent.com/'+u.split('/').pop()})};
      if(u.startsWith('https://files.oaiusercontent.com/')){if(changed)f.text.childNodes[0].data+=' changed';if(busy)f.main.append(e('button',{'data-testid':'stop-button'}));return{ok:true,blob:async()=>new Blob([wrong?'unrelated bytes':bytes],{type:'image/png'})};}
      assert.fail('unexpected read endpoint');}};
  return{context,calls,bytes};
}
test('original read matches exact preview bytes to one official file and never returns credentials',async()=>{
  const f=fixture(),c=originalContext(f),result=await vm.runInNewContext(originalImageReadCode(prompt,url,hash('synthetic-user')),c.context);
  assert.equal(result.sha256,hash(c.bytes));assert.equal(result.fileId,'file_one');assert.equal(JSON.stringify(result).includes('synthetic-secret'),false);assert.equal(c.calls.length,4);
});
test('original read rejects duplicate originals, wrong bytes and DOM/busy drift',async()=>{
  for(const [options,reason]of [[{multiple:true},'ORIGINAL_FILE_AMBIGUOUS'],[{wrong:true},'ORIGINAL_FILE_AMBIGUOUS'],[{changed:true},'ORIGINAL_PROMPT_UNCONFIRMED'],[{busy:true},'ORIGINAL_GENERATION_ACTIVE']]){
    const f=fixture(),c=originalContext(f,options);await assert.rejects(vm.runInNewContext(originalImageReadCode(prompt,url,hash('synthetic-user')),c.context),new RegExp(reason));
  }
  const f=fixture(),c=originalContext(f);await assert.rejects(vm.runInNewContext(originalImageReadCode(prompt,url+'-changed',hash('synthetic-user')),c.context),/ORIGINAL_PROMPT_UNCONFIRMED/);assert.equal(c.calls.length,0);
});
function journalFixture(t){
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'director-passive-dom-'));t.after(()=>fs.rmSync(dir,{recursive:true,force:true}));
  const saved={version:'opencli-task/v1',requestId:'original-task',requestHash:'a'.repeat(64),request:{frozen:'unchanged'},prompt,status:'submission_unknown',error:'SENT_PROMPT_UNCONFIRMED',profile:'profile',accountKey:'digest',page:'original-page',conversationUrl:null,sentAt:'2026-10-25T00:00:00Z',ownedSend:{version:OWNED_SEND_PROTOCOL,token:'original-task'}};
  saved.sendIntent={version:'owned-click-intent/v1',token:saved.requestId,requestId:saved.requestId,requestHash:saved.requestHash,promptSha256:hash(prompt),page:saved.page};
  const loc={file:path.join(dir,'request.json')};const before=Buffer.from(JSON.stringify(saved));fs.writeFileSync(loc.file,before);return{saved,loc,before,state:{userCount:1,promptMatched:true,url}};
}
test('passive UNKNOWN recovery archives exact original journal and records no resend',t=>{
  const f=journalFixture(t);recordPassiveObservation({runtimeSha256:'b'.repeat(64)},f.saved,f.loc,f.state,f.before);
  const result=JSON.parse(fs.readFileSync(f.loc.file)),proof=result.passiveObservation;
  assert.equal(result.status,'submission_unknown');assert.equal(result.error,'SENT_PROMPT_UNCONFIRMED');assert.equal(result.sendConfirmed,true);assert.equal(proof.resubmitted,false);assert.equal(proof.readOnly,true);
  assert.equal(proof.promptSha256,hash(prompt));assert.deepEqual(result.request,{frozen:'unchanged'});assert.deepEqual(fs.readFileSync(path.join(path.dirname(f.loc.file),proof.journalBeforeArchive)),f.before);
});
test('wrong intent, wrong original page, changed conversation or journal cannot be recovered',t=>{
  for(const field of ['token','requestId','requestHash','promptSha256','page']){const f=journalFixture(t);f.saved.sendIntent[field]='changed';assert.throws(()=>recordPassiveObservation({},f.saved,f.loc,f.state,f.before),/ORIGINAL_SEND_INTENT_UNCONFIRMED/);assert.deepEqual(fs.readFileSync(f.loc.file),f.before);}
  const f=journalFixture(t);f.saved.conversationUrl=url+'-other';assert.throws(()=>recordPassiveObservation({},f.saved,f.loc,f.state,f.before),/ORIGINAL_PROMPT_UNCONFIRMED/);
  const g=journalFixture(t);fs.writeFileSync(g.loc.file,'changed');assert.throws(()=>recordPassiveObservation({},g.saved,g.loc,g.state,g.before),/ORIGINAL_JOURNAL_CHANGED/);
});

test('original byte read rechecks the frozen account before official file access',async()=>{
  const f=fixture(),c=originalContext(f);await assert.rejects(vm.runInNewContext(originalImageReadCode(prompt,url,hash('other-account')),c.context),/ACCOUNT_CHANGED/);assert.equal(c.calls.some(u=>u.includes('/files/download/')),false);
});
