import test from 'node:test';import assert from 'node:assert/strict';import { recoverSavedImage } from './recover-saved-image.mjs';
const receipt = { conversationUrl:'https://chatgpt.com/c/test-saved-id',sha256:'a'.repeat(64),bytes:100,originalBytesVerified:true };
test('a read timeout reopens the same verified link once without send capability', async()=>{
 let calls=0;const urls=[];const page={goto:async u=>urls.push(u),sleep:async()=>new Promise(resolve=>setTimeout(resolve,3)),evaluate:async code=>{if(code.startsWith('!!'))return false;calls++;return calls===1?{ready:false,url:receipt.conversationUrl}:{ready:true,url:receipt.conversationUrl,sha256:receipt.sha256,bytes:100,userTurns:1};}};
 const r=await recoverSavedImage(page,receipt,{timeoutMs:1,pollMs:1});assert.equal(r.noSubmission,true);assert.deepEqual(urls,[receipt.conversationUrl,receipt.conversationUrl]);
});
test('unknown/unverified submissions are never opened',async()=>{let nav=0;await assert.rejects(recoverSavedImage({goto:async()=>nav++},{...receipt,originalBytesVerified:false}));assert.equal(nav,0);});
test('identity mismatch is never retried',async()=>{let nav=0;const page={goto:async()=>nav++,evaluate:async c=>c.startsWith('!!')?false:{ready:true,url:receipt.conversationUrl,sha256:'b'.repeat(64),bytes:100,userTurns:1}};await assert.rejects(recoverSavedImage(page,receipt),/identity mismatch/);assert.equal(nav,1);});
test('active generation prevents any navigation',async()=>{let nav=0;await assert.rejects(recoverSavedImage({evaluate:async()=>true,goto:async()=>nav++},receipt),/Active/);assert.equal(nav,0);});
test('unavailable result stops after two reads',async()=>{let nav=0;const page={goto:async()=>nav++,sleep:async()=>new Promise(r=>setTimeout(r,3)),evaluate:async c=>c.startsWith('!!')?false:{ready:false,url:receipt.conversationUrl}};await assert.rejects(recoverSavedImage(page,receipt,{timeoutMs:1,pollMs:1}),/two read attempts/);assert.equal(nav,2);});
