import{pathToFileURL}from"node:url";
const source=process.env.OPENCLI_CHECKOUT;if(!source)throw Error("Set OPENCLI_CHECKOUT to the pinned patched checkout");const sourceUrl=pathToFileURL(source.replace(/[\\\/]+$/,"")+"/");
import test from 'node:test';import assert from 'node:assert/strict';
const{waitForChatGPTImages}=await import(new URL("clis/chatgpt/utils.js",sourceUrl));
test('preserved pending document is never reopened through provisional conversation id',async()=>{
process.env.OPENCLI_CHATGPT_PRESERVE_PENDING_DOCUMENT='1';const navigations=[];
const page={sleep:async()=>{},goto:async url=>navigations.push(url),evaluate:async script=>{if(script==='window.location.href')return'https://chatgpt.com/';if(script.includes('const urls = []'))return['https://chatgpt.com/backend-api/estuary/content?id=file-test'];if(script.includes('stop-button'))return false;throw Error('Unexpected observation script');}};
const files=await waitForChatGPTImages(page,[],12,'https://chatgpt.com/c/local-chatgpt%3Atest-id');assert.equal(files.length,1);assert.deepEqual(navigations,[]);
});
