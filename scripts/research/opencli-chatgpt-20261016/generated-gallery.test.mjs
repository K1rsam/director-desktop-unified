import{pathToFileURL}from"node:url";
const source=process.env.OPENCLI_CHECKOUT;if(!source)throw Error("Set OPENCLI_CHECKOUT to the pinned patched checkout");const sourceUrl=pathToFileURL(source.replace(/[\\\/]+$/,"")+"/");
import test from'node:test';import assert from'node:assert/strict';import{createRequire}from'node:module';
const{getChatGPTVisibleImageUrls}=await import(new URL("clis/chatgpt/utils.js",sourceUrl));
const{JSDOM}=createRequire(new URL("package.json",sourceUrl))('jsdom');
for(const hasGenerated of [false,true])test('strict gallery excludes unlabelled uploaded parent '+hasGenerated,async()=>{
process.env.OPENCLI_CHATGPT_REQUIRE_GENERATED_GALLERY='1';
const dom=new JSDOM(`<main><div data-turn="user"><img src="blob:https://chatgpt.com/parent"></div>${hasGenerated?'<div data-testid="generated-image-gallery"><img src="blob:https://chatgpt.com/result"></div>':''}</main>`,{runScripts:'outside-only',url:'https://chatgpt.com/'});
for(const n of dom.window.document.querySelectorAll('*'))n.getBoundingClientRect=()=>({width:400,height:250});for(const n of dom.window.document.querySelectorAll('img')){Object.defineProperty(n,'naturalWidth',{value:1672});Object.defineProperty(n,'naturalHeight',{value:941});}
try{const urls=await getChatGPTVisibleImageUrls({evaluate:async code=>dom.window.eval(code)});assert.deepEqual(Array.from(urls),hasGenerated?['blob:https://chatgpt.com/result']:[]);}finally{dom.window.close();delete process.env.OPENCLI_CHATGPT_REQUIRE_GENERATED_GALLERY;}
});
