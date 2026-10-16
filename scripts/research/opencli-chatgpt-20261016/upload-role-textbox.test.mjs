import{pathToFileURL}from"node:url";
const source=process.env.OPENCLI_CHECKOUT;if(!source)throw Error("Set OPENCLI_CHECKOUT to the pinned patched checkout");const sourceUrl=pathToFileURL(source.replace(/[\\\/]+$/,"")+"/");
import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import{createRequire}from'node:module';
const{uploadChatGPTImages}=await import(new URL("clis/chatgpt/utils.js",sourceUrl));
const {JSDOM}=createRequire(new URL("package.json",sourceUrl))('jsdom');
for(const present of [true,false])test('role textbox upload must '+(present?'recognize current attachment':'reject historical attachment'),async()=>{
 const dir=fs.mkdtempSync(path.join(os.tmpdir(),'director-upload-fixture-'));const file=path.join(dir,'parent.png');fs.writeFileSync(file,'synthetic-test');
 const chip='<div role="group" aria-label="parent.png"></div>';
 const dom=new JSDOM(`<article>${present?'':chip}</article><form><div contenteditable="true" role="textbox"></div>${present?chip:''}</form>`,{runScripts:'outside-only',url:'https://chatgpt.com/'});
 for(const n of dom.window.document.querySelectorAll('*'))n.getBoundingClientRect=()=>({width:100,height:100});
 try{const result=await uploadChatGPTImages({setFileInput:async()=>{},sleep:async()=>{},evaluate:async code=>dom.window.eval(code)},[file]);assert.equal(result.ok,present);}finally{dom.window.close();fs.unlinkSync(file);fs.rmdirSync(dir);}
});
