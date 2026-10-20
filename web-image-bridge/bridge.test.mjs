import vm from'node:vm';
import test from'node:test';import assert from'node:assert/strict';import{validateRequest,checkIdentity,requestHash,stableConversation,dispatch,waitForIdentity,ensureLocalService,connectedProfile,connectAvailable,activeCode}from'./bridge.mjs';
const request={requestId:'synthetic-request',kind:'image',model:'chatgpt-image',prompt:'synthetic',ratio:'16:9',resolution:'native',references:[],options:{}};
test('video and other channels are rejected before any browser access',()=>{assert.throws(()=>validateRequest({...request,kind:'video'}));assert.throws(()=>validateRequest({...request,model:'api-image'}));});
test('request id traversal and empty prompt are rejected',()=>{assert.throws(()=>validateRequest({...request,requestId:'../other'}));assert.throws(()=>validateRequest({...request,prompt:''}));});
test('original request is immutable across recovery',()=>{checkIdentity({requestHash:requestHash(request)},request);assert.throws(()=>checkIdentity({requestHash:requestHash(request)},{...request,prompt:'changed'}));});
test('provisional conversation cannot be reopened as a stable task',()=>{assert.equal(stableConversation('https://chatgpt.com/c/local-chatgpt%3Atest'),false);assert.equal(stableConversation('https://chatgpt.com/c/saved-id'),true);assert.equal(stableConversation('https://evil.test/c/saved-id'),false);});
test('unknown operation cannot send a prompt',async()=>{await assert.rejects(dispatch({operation:'generate-video',config:{stateRoot:process.cwd()}}),/UNSUPPORTED_OPERATION/);});
test('authenticated page hydration is not a logout',async()=>{let n=0;const p={evaluate:async()=>({accountKey:'original',composer:++n>1}),sleep:async()=>{}};assert.equal((await waitForIdentity(p,'original',3)).composer,true);});
test('slow authenticated composer keeps the queue busy',async()=>{const p={evaluate:async()=>({accountKey:'original',composer:false}),sleep:async()=>{}};await assert.rejects(waitForIdentity(p,'original',2),/BRIDGE_BUSY/);});
test('different account is held without waiting or generation',async()=>{const p={evaluate:async()=>({accountKey:'changed',composer:true}),sleep:async()=>assert.fail()};await assert.rejects(waitForIdentity(p,'original'),/ACCOUNT_CHANGED/);});
test('connecting an existing local service never restarts it',async()=>{await ensureLocalService(async()=>({pending:1}),()=>assert.fail('must preserve existing daemon'),async()=>assert.fail());});
test('connecting starts an absent service once and waits with a bound',async()=>{let checks=0,starts=0;await ensureLocalService(async()=>++checks>2?{}:null,()=>starts++,async()=>{});assert.equal(starts,1);assert.equal(checks,3);});
test('an unavailable local service stops waiting without browser or generation commands',async()=>{let checks=0,starts=0;await assert.rejects(ensureLocalService(async()=>{checks++;return null;},()=>starts++,async()=>{}),/LOCAL_SERVICE_UNAVAILABLE/);assert.equal(starts,1);assert.equal(checks,25);});
test('another connected browser does not invalidate the saved execution browser',()=>{const s={extensionConnected:false,profiles:[{contextId:'original',extensionConnected:true},{contextId:'other',extensionConnected:true}]};assert.equal(connectedProfile(s,'original').contextId,'original');assert.throws(()=>connectedProfile(s),/BROWSER_SELECTION_REQUIRED/);assert.throws(()=>connectedProfile(s,'missing'),/OPENCLI_BROWSER_NOT_CONNECTED/);});
test('explicit reconnect can use a responsive browser only with the same saved account',async()=>{const calls=[];const status={profiles:[{contextId:'original',extensionConnected:true},{contextId:'new',extensionConnected:true}]};const ready=await connectAvailable({profile:'original',accountKey:'same'},async()=>status,async c=>{calls.push(c.profile);if(c.profile==='original')throw Error('read-only timeout');return {profile:c.profile,accountKey:'same',ready:true};});assert.equal(ready.profile,'new');assert.deepEqual(calls,['original','new']);await assert.rejects(connectAvailable({accountKey:'same'},async()=>status,async()=>({accountKey:'changed'})),/ACCOUNT_CHANGED/);});
test('first connection never silently chooses among multiple accounts or browsers',async()=>{await assert.rejects(connectAvailable({},async()=>({profiles:[{extensionConnected:true},{extensionConnected:true}]}),async()=>assert.fail()),/BROWSER_SELECTION_REQUIRED/);});

test('hidden retained stop controls do not hold a completed image indefinitely',()=>{
 const button=(visible,hidden=false,visibility='visible')=>({getClientRects:()=>visible?[{}]:[],closest:()=>hidden?{}:null,visibility});
 const evaluate=buttons=>vm.runInNewContext(activeCode,{document:{querySelectorAll:()=>buttons},getComputedStyle:b=>({visibility:b.visibility})});
 assert.equal(evaluate([button(false)]),false);
 assert.equal(evaluate([button(true,true),button(true,false,'hidden')]),false);
 assert.equal(evaluate([]),false);
 assert.equal(evaluate([button(false),button(true)]),true);
});
