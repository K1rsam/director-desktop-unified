import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { createHash } from 'node:crypto';
import { logicalComposerText, prepareOwnedComposer, prepareComposerCode, readyComposerCode, clickComposerCode, OWNED_SEND_PROTOCOL } from './composer-transport.mjs';

// Minimal DOM-shaped fixtures for the selectors and node contracts used by the
// owned adapter. No parser, browser, private capture or production dependency.
const text=data=>({nodeType:3,data});
class Element {
  nodeType=1;isConnected=true;isContentEditable=false;disabled=false;childNodes=[];parentElement=null;
  constructor(tag,attributes={},children=[]){this.tagName=tag.toUpperCase();this.attrs=new Map(Object.entries(attributes));this.append(...children);}
  get attributes(){return[...this.attrs].map(([name,value])=>({name,value}));}
  get firstChild(){return this.childNodes[0]??null;}
  get innerHTML(){return JSON.stringify(this.childNodes.map(n=>n.nodeType===3?{text:n.data}:n.nodeType===1?{tag:n.tagName,attrs:[...n.attrs],children:n.innerHTML}:{unknown:n.nodeType}));}
  append(...children){for(const child of children){child.parentElement=this;this.childNodes.push(child);}}
  setAttribute(name,value){this.attrs.set(name,String(value));}
  getAttribute(name){return this.attrs.get(name)??null;}
  hasAttribute(name){return this.attrs.has(name);}
  getClientRects(){return[{}];}
  matches(selector){return selector.split(',').some(part=>{
    if(part.startsWith('#'))return this.getAttribute('id')===part.slice(1);
    const tag=/^[a-z]+/i.exec(part)?.[0];if(tag&&this.tagName!==tag.toUpperCase())return false;
    const attributes=[...part.matchAll(/\[([^=\]]+)(?:="([^"]*)")?\]/g)];
    if(!tag&&!attributes.length)throw Error(`Unsupported synthetic selector: ${part}`);
    return attributes.every(([,name,value])=>value===undefined?this.hasAttribute(name):this.getAttribute(name)===value);
  });}
  closest(selector){for(let node=this;node;node=node.parentElement)if(node.matches(selector))return node;return null;}
  querySelectorAll(selector){const found=[];for(const child of this.childNodes)if(child.nodeType===1){if(child.matches(selector))found.push(child);found.push(...child.querySelectorAll(selector));}return found;}
  querySelector(selector){return this.querySelectorAll(selector)[0]??null;}
}
const p=(...children)=>new Element('p',{},children);
const br=(attributes={})=>new Element('br',attributes);
const emptyP=()=>new Element('p',{'data-empty-paragraph':'true'},[br({class:'ProseMirror-trailingBreak'})]);
const paragraphs=['Generate one image only. 16:9；原文','资产正文：灰铁钥匙，保留全部真实空白。','无额外标签水印。'];
const expected=paragraphs.join('\n\n'),visual=paragraphs.join('\n\n\n\n\n');
const capturedShape=()=>[p(text(paragraphs[0])),emptyP(),p(text(paragraphs[1])),emptyP(),p(text(paragraphs[2]))];
function dom(children){
  const composer=new Element('div',{id:'prompt-textarea',role:'textbox',contenteditable:'true'},children);composer.isContentEditable=true;
  const button=new Element('button',{'data-testid':'send-button','aria-label':'Send prompt'}),form=new Element('form',{},[composer,button]),document=new Element('document',{},[form]);
  const context=vm.createContext({document,HTMLElement:Element,getComputedStyle:()=>({display:'block',visibility:'visible'}),location:{href:'https://chatgpt.com/'}});
  return{composer,button,context};
}
const hash=value=>createHash('sha256').update(value).digest('hex');
const gate={version:OWNED_SEND_PROTOCOL,token:'request'};
function own(d,value){assert.equal(vm.runInContext(prepareComposerCode(value,'request'),d.context).ok,true);}

test('ProseMirror paragraph shape reconstructs exact prompt bytes rather than visual paragraph margins',()=>{
  const d=dom(capturedShape()),before=d.composer.innerHTML,result=logicalComposerText(d.composer);
  assert.equal(result.ok,true);assert.equal(result.text,expected);assert.equal(hash(result.text),hash(expected));
  assert.notEqual(visual,expected);assert.equal(d.composer.innerHTML,before);
});

test('logical paragraphs preserve every real space, tab, CR, LF, Unicode and trailing empty paragraph',()=>{
  const parts=[' \t\u00a0A\u200b😀e\u0301\r\nline  ','','最后',''];
  const d=dom(parts.map(value=>value===''?emptyP():p(text(value))));
  assert.equal(logicalComposerText(d.composer).text,parts.join('\n'));
  assert.equal(logicalComposerText(d.composer).text.endsWith('\n'),true);
});

test('ordinary br and every empty paragraph remain meaningful newlines',()=>{
  const d=dom([p(text('A'),br(),text('B')),emptyP(),p(br()),p(text('C'))]);
  assert.equal(logicalComposerText(d.composer).text,'A\nB\n\n\n\nC');
});

test('a changed space, newline or Unicode code point never becomes equal by normalization',()=>{
  for(const children of [[p(text(' text'))],[p(text('text '))],[p(text('te\u00a0xt'))],[p(text('te\u200bxt'))],[p(text('text'),br())],[p(text('text')),p()]]){
    assert.notEqual(logicalComposerText(dom(children).composer).text,'text');
  }
});

test('unknown markup and malformed placeholders fail closed rather than dropping content',()=>{
  const cases=[
    [p(text('A'),br({class:'ProseMirror-trailingBreak'}))],
    [p(br({class:'ProseMirror-trailingBreak'}),br())],
    [new Element('p',{'data-empty-paragraph':'true'},[text('actual')])],
    [p(new Element('span',{},[text('text')]))],
    [p(br({class:'ProseMirror-trailingBreak extra'}))],
    [p(br({class:'ProseMirror-trailingBreak','aria-label':'text'}))],
    [text('text')],[new Element('div',{},[p(text('text'))])],[p({nodeType:8},text('text'))],
  ];
  for(const children of cases)assert.equal(logicalComposerText(dom(children).composer).ok,false);
});

test('ready and click use the same logical-byte contract and do not read innerText visual spacing',()=>{
  const d=dom(capturedShape());Object.defineProperty(d.composer,'innerText',{get(){throw Error('visual text must not be read');}});
  own(d,expected);let clicks=0;d.button.click=()=>clicks++;
  assert.equal(vm.runInContext(readyComposerCode(expected,'request'),d.context).ok,true);
  assert.equal(vm.runInContext(clickComposerCode(expected,gate),d.context).clicked,true);assert.equal(clicks,1);
  assert.equal(vm.runInContext(clickComposerCode(expected,gate),d.context).recovered,true);assert.equal(clicks,1);
});

test('exact DOM bytes can validate fillText verified=false while inaccurate DOM never trusts verified=true',async()=>{
  for(const mismatch of [false,true]){
    const d=dom(mismatch?[p(text('changed'))]:capturedShape());
    const page={evaluate:code=>vm.runInContext(code,d.context),fillText:async()=>({verified:mismatch,actual:mismatch?expected:visual}),sleep:async()=>{}};
    if(mismatch)await assert.rejects(prepareOwnedComposer(page,expected,'request'),/COMPOSER_TEXT_UNCONFIRMED/);
    else assert.equal((await prepareOwnedComposer(page,expected,'request')).token,'request');
  }
});

test('ready-to-click DOM text mutation is held with zero clicks',()=>{
  const d=dom([p(text('text'))]);own(d,'text');let clicks=0;d.button.click=()=>clicks++;
  assert.equal(vm.runInContext(readyComposerCode('text','request'),d.context).ok,true);
  d.composer.querySelector('p').firstChild.data='text ';
  assert.equal(vm.runInContext(clickComposerCode('text',gate),d.context).clicked,false);assert.equal(clicks,0);
});

test('embedded chips added after preparation block ready and click without editing the chip',()=>{
  const d=dom([p(text('text'))]);own(d,'text');let clicks=0;d.button.click=()=>clicks++;
  assert.equal(vm.runInContext(readyComposerCode('text','request'),d.context).ok,true);
  d.composer.append(new Element('span',{contenteditable:'false'},[text('reference')]));const before=d.composer.innerHTML;
  assert.equal(vm.runInContext(readyComposerCode('text','request'),d.context).ok,false);
  assert.equal(vm.runInContext(clickComposerCode('text',gate),d.context).clicked,false);
  assert.equal(d.composer.innerHTML,before);assert.equal(clicks,0);
});
