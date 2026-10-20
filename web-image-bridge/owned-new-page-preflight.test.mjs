import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';
import { createHash } from 'node:crypto';
import { ownedNewPagePreflight, ownedNewPagePreflightCode, verifyOwnedNewPagePreflight } from './bridge.mjs';
const prompt='Generate one image only. 16:9；原文\n\n资产正文，保留空白。\n\n无额外水印。';
const expected=()=>({newPageVerified:true,page:'NEW_PAGE',requestId:'new-request',accountKey:'original-account',prompt});
const observation=()=>({page:'NEW_PAGE',ownerToken:null,accountKey:'original-account',url:'https://chatgpt.com/',userCount:0,assistantCount:0,generating:false,visibleComposers:1,embeddedChips:0,logical:{ok:true,text:prompt}});
class Element {
  constructor(visible=true){this.isContentEditable=true;this.visible=visible;this.childNodes=[];}
  getClientRects(){return this.visible?[{}]:[];}
  closest(){return null;}
  querySelector(){return null;}
  querySelectorAll(){return[];}
  getAttribute(){return null;}
}
function paragraph(value){return{nodeType:1,tagName:'P',childNodes:value===''?[{nodeType:1,tagName:'BR',childNodes:[],attributes:[{name:'class',value:'ProseMirror-trailingBreak'}],getAttribute:()=> 'ProseMirror-trailingBreak'}]:[{nodeType:3,data:value}],getAttribute:name=>name==='data-empty-paragraph'&&value===''?'true':null};}
test('the real preflight counterexample shape has one visible exact logical draft despite nonempty visual text',()=>{
  const hidden=new Element(false),composer=new Element();composer.childNodes=prompt.split('\n').map(paragraph);
  composer.innerText=prompt.replaceAll('\n\n','\n\n\n\n\n');
  const document={querySelectorAll:selector=>selector.startsWith('#prompt-textarea')?[hidden,composer,composer]:[]};
  const view=vm.runInNewContext(ownedNewPagePreflightCode(),{document,HTMLElement:Element,location:{href:'https://chatgpt.com/'},getComputedStyle:()=>({display:'block',visibility:'visible'})});
  assert.equal(!composer.innerText.trim(),false,'the old empty-only predicate rejects the captured shape');
  assert.equal(view.visibleComposers,1);assert.equal(view.logical.text,prompt);
  assert.equal(ownedNewPagePreflight({...view,page:'NEW_PAGE',accountKey:'original-account'},expected()).mode,'exact_prompt_draft');
});
test('only exact empty or exact compiled prompt passes without modifying either input',()=>{
  for(const value of ['',prompt]){const view=observation();view.logical.text=value;const original=JSON.stringify(view),gate=expected(),gateBefore=JSON.stringify(gate);
    const result=ownedNewPagePreflight(view,gate);assert.equal(result.ok,true);assert.equal(result.mode,value===''?'empty':'exact_prompt_draft');assert.equal(JSON.stringify(view),original);assert.equal(JSON.stringify(gate),gateBefore);}
});
test('different drafts, real whitespace and Unicode differences are never trimmed or matched by containment',()=>{
  for(const value of [' ', '\n',prompt+' ', ' '+prompt,prompt+'\n',prompt.replace('空白','空\u00a0白'),prompt.replace('原文','原\u200b文'),'prefix '+prompt+' suffix']){
    const view=observation();view.logical.text=value;assert.equal(ownedNewPagePreflight(view,expected()).reason,'different_draft');
  }
});
test('a reused or mismatched page, other owner or changed account cannot qualify as this request new page',()=>{
  for(const [target,key,value]of [['expected','newPageVerified',false],['expected','page','OLD_PAGE'],['observed','page','OTHER_PAGE'],['observed','ownerToken','other-request'],['observed','accountKey','other-account'],['expected','accountKey','']]){
    const gate=expected(),view=observation();(target==='expected'?gate:view)[key]=value;assert.equal(ownedNewPagePreflight(view,gate).ok,false);
  }
});
test('existing conversations, foreign URLs and generation activity always hold the request',()=>{
  for(const [key,value]of [['userCount',1],['assistantCount',1],['url','https://chatgpt.com/c/synthetic'],['url','https://chatgpt.com.evil.test/'],['generating',true]]){
    const view=observation();view[key]=value;assert.equal(ownedNewPagePreflight(view,expected()).ok,false);
  }
});
test('attachments, ambiguous composers and unknown logical markup are rejected',()=>{
  for(const [key,value]of [['visibleComposers',0],['visibleComposers',2],['embeddedChips',1],['logical',{ok:false}],['logical',{ok:true,text:null}]]){
    const view=observation();view[key]=value;assert.equal(ownedNewPagePreflight(view,expected()).ok,false);
  }
});
function fixture(t,view){
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'director-new-page-preflight-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const saved={requestId:'new-request',request:{prompt:'original canonical'},requestHash:'original-wire-hash',page:'NEW_PAGE',accountKey:'original-account',prompt,status:'preparing'};
  const file=path.join(root,'request.json');fs.writeFileSync(file,JSON.stringify(saved));let reads=0,sleeps=0;
  return{saved,loc:{file},page:{getActivePage:()=> 'NEW_PAGE',evaluate:async()=>{reads++;return{...view,logical:{...view.logical}};},sleep:async()=>{sleeps++;}},counts:()=>({reads,sleeps})};
}
test('the live preparation helper records exact diagnostics but never fills, clicks or rewrites the original request',async t=>{
  for(const different of [false,true]){const view=observation();if(different)view.logical.text='other draft';const f=fixture(t,view),before=JSON.stringify(f.saved.request);
    if(different)await assert.rejects(verifyOwnedNewPagePreflight(f.page,f.saved,f.loc,true,async()=>({accountKey:'original-account'})),/EMPTY_PREFLIGHT/);
    else assert.equal((await verifyOwnedNewPagePreflight(f.page,f.saved,f.loc,true,async()=>({accountKey:'original-account'}))).mode,'exact_prompt_draft');
    const persisted=JSON.parse(fs.readFileSync(f.loc.file,'utf8'));assert.equal(persisted.preSendValidation.reason,different?'different_draft':undefined);
    assert.equal(persisted.preSendValidation.logicalSha256,createHash('sha256').update(view.logical.text).digest('hex'));assert.equal(JSON.stringify(f.saved.request),before);
    for(const marker of ['sentAt','sendIntent','sendConfirmed'])assert.equal(Object.hasOwn(persisted,marker),false);assert.deepEqual(f.counts(),{reads:1,sleeps:0});
  }
});
test('a missing composer waits a bounded fifteen observations and never acquires send authority',async t=>{
  const view=observation();view.visibleComposers=0;view.logical={ok:false};const f=fixture(t,view);
  await assert.rejects(verifyOwnedNewPagePreflight(f.page,f.saved,f.loc,true,()=>assert.fail('no account probe without composer')),/EMPTY_PREFLIGHT/);
  assert.deepEqual(f.counts(),{reads:15,sleeps:14});assert.equal(f.saved.preSendValidation.ok,false);
});
