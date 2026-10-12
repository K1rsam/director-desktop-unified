'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const KEY='bridgePendingExtensionReload',PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7',TOKEN='bridge-director-desktop-e651be83-e892-4a0e-92c4-a094ae523b68',SERVER='http://127.0.0.1:53179';
function fixture(options={}){
  let now=1000000,operation=options.operation;const events=[],local={...options.local},launch={launchToken:TOKEN,serverUrl:SERVER,requestedUrl:'https://chatgpt.com/'},session={'chatgptBridgeLaunchedTab:1':launch};
  const identity={profileId:PROFILE,bundleId:'bundle-fixed',extensionVersion:'2.4.5',backgroundEpoch:'old-bg',serverInstanceId:'server-fixed',...options.identity};
  const tab={id:1,url:'https://chatgpt.com/#chatgpt-bridge-launch='+TOKEN,active:false,incognito:false,...options.tab};
  const context=vm.createContext({URL,URLSearchParams,Date:class extends Date{static now(){return now;}},Math,console:{info(){},warn(){},error(){}},setTimeout(){return 0;},chrome:{
    tabs:{get:async()=>tab,query:async()=>[tab],reload:async()=>{}},storage:{local:{get:async key=>({[key]:local[key]}),set:async value=>{Object.assign(local,value);events.push('persist');},remove:async key=>{delete local[key];events.push('remove');}},session:{get:async()=>({...session})}}}});
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/extensionReloadCoordinator.js'),'utf8').replace(/export /g,''),context);
  const c=context.createExtensionReloadCoordinator({backgroundState:{read:async()=>({lease:options.lease||null,commands:options.commands||{},outbox:[]})},safeBridgeServerUrl:u=>u,launchTokenPattern:/^bridge-/,
    maintenanceOperations:{read:async()=>({active:operation}),plan:async()=>({accepted:true,state:{active:{operationId:'original-operation'}}}),dispatch:async()=>({accepted:true}),succeed:async(id,result)=>{events.push('succeed');if(options.failSucceed)throw Error('persist failed');operation={operationId:id,status:'succeeded',result};}},
    readRuntimeIdentity:async()=>identity,readLaunchedTab:async()=>launch,rememberLaunchedTab:async()=>{},navigateTab:async()=>events.push('navigate'),reloadTab:async()=>events.push('reload'),reloadRuntime(){},scheduleRecoveryWake:async name=>({armed:true,alarmName:name}),clearRecoveryWake:async()=>events.push('disarm')});
  return {c,local,events,advance:ms=>{now+=ms;},identity,setOperation:value=>{operation=value;}};
}
async function schedule(f){await f.c.scheduleExtensionReload({sourceTabId:1,sourceLaunchToken:TOKEN,commandId:'command-original',reloadTabs:false,temporaryServerUrl:SERVER});return f.c.recoverPendingExtensionReload();}
function proof(overrides={}){return {accepted:true,helloMessageId:'hello-exact',tabId:1,launchToken:TOKEN,profileId:PROFILE,serverInstanceId:'server-fixed',transportServerInstanceId:'server-fixed',serverUrl:SERVER,bundleId:'bundle-fixed',extensionVersion:'2.4.5',backgroundEpoch:'new-bg',contentEpoch:'new-content',browserIncognito:false,launchRecordMatches:true,...overrides};}
test('same-document target navigation explicitly reloads the original document and remains pending',async()=>{
  const f=fixture(),r=await schedule(f);assert.equal(r.pending,true);assert.equal(r.recovered,false);
  assert.ok(f.events.indexOf('navigate')<f.events.indexOf('reload'));assert.equal(f.events.includes('succeed'),false);assert.equal(f.events.includes('remove'),false);
  await f.c.recoverPendingExtensionReload();assert.equal(f.events.filter(x=>x==='reload').length,1);
});
test('only accepted exact source hello with new runtime settles maintenance before anchor removal',async()=>{
  const f=fixture();await schedule(f);const r=await f.c.confirmSourceReady(proof());assert.equal(r.verified,true);assert.equal(r.result.connectionVerified,true);
  assert.ok(f.events.indexOf('succeed')<f.events.indexOf('remove'));assert.equal(f.local[KEY],undefined);
});
test('wrong source, nonce, profile, server, version, bundle and unaccepted proof cannot complete',async()=>{
  for(const change of [{accepted:false},{helloMessageId:''},{tabId:2},{launchToken:'wrong'},{profileId:'wrong'},{serverInstanceId:'wrong'},{serverUrl:'wrong'},{bundleId:'wrong'},{extensionVersion:'wrong'},{backgroundEpoch:'old-bg'},{contentEpoch:''},{browserIncognito:true},{launchRecordMatches:false}]){
    const f=fixture();await schedule(f);assert.equal((await f.c.confirmSourceReady(proof(change))).verified,false);assert.ok(f.local[KEY]);assert.equal(f.events.includes('succeed'),false);
  }
});
test('reconnect timeout preserves pending and forbids repeated navigation across later wakeups',async()=>{
  const f=fixture();await schedule(f);f.advance(130000);const r=await f.c.recoverPendingExtensionReload();assert.equal(r.reason,'source_ready_timeout');assert.equal(f.local[KEY].recoveryStatus,'held');
  await f.c.recoverPendingExtensionReload();assert.equal(f.events.filter(x=>x==='reload').length,1);assert.equal(f.events.includes('remove'),false);
});
test('failure to persist maintenance success retains the original recovery anchor',async()=>{
  const f=fixture({failSucceed:true});await schedule(f);await assert.rejects(f.c.confirmSourceReady(proof()),/persist failed/);assert.ok(f.local[KEY]);assert.equal(f.events.includes('remove'),false);
});
test('authenticated replacement server uses its real new nonce while preserving the old runtime proof',async()=>{
  const f=fixture();await schedule(f);const result=await f.c.confirmSourceReady(proof({serverInstanceId:'new-server',transportServerInstanceId:'new-server'}));
  assert.equal(result.verified,true);assert.equal(result.result.previousRuntime.serverInstanceId,'server-fixed');assert.equal(result.result.connectionProof.serverInstanceId,'new-server');
});
test('crash after durable source proof only cleans its own anchor without repeating navigation or rewriting history',async()=>{
  const f=fixture();await schedule(f);const pending=f.local[KEY],done=await f.c.confirmSourceReady(proof());f.local[KEY]=pending;
  assert.equal((await f.c.recoverPendingExtensionReload()).reason,'durable_source_ready');assert.equal(f.local[KEY],undefined);assert.equal(f.events.filter(x=>x==='reload').length,1);assert.equal(f.events.filter(x=>x==='succeed').length,1);
  f.local[KEY]=pending;f.setOperation({operationId:'original-operation',status:'succeeded',result:{...done.result,connectionProof:{...proof(),launchToken:'other'}}});
  assert.equal((await f.c.recoverPendingExtensionReload()).reason,'awaiting_source_ready');assert.ok(f.local[KEY]);assert.equal(f.events.filter(x=>x==='reload').length,1);
});
function legacy(){return {operationId:'legacy-original',kind:'extension.reload',status:'succeeded',updatedAt:999900,preconditions:{tabIds:[1,2],expectedVersion:'2.4.5'},result:{recovered:true,sourceTabId:1,tabCount:2}};}
test('legacy premature success restores only its original idle owned root once and preserves terminal history',async()=>{
  const op=legacy(),f=fixture({operation:op});const r=await f.c.recoverPendingExtensionReload();assert.equal(r.pending,true);assert.equal(f.local[KEY].legacyOperation,true);
  assert.deepEqual(Array.from(f.local[KEY].tabIds),[1]);await f.c.recoverPendingExtensionReload();assert.equal(f.events.filter(x=>x==='reload').length,1);
  assert.equal((await f.c.confirmSourceReady(proof())).verified,true);assert.ok(f.local['chatgptBridge:reloadConnectionProof:legacy-original']);assert.equal(op.status,'succeeded');assert.equal(op.result.connectionVerified,undefined);assert.equal(f.events.includes('succeed'),false);
  await f.c.recoverPendingExtensionReload();assert.equal(f.events.filter(x=>x==='reload').length,1);
});
test('legacy adoption rejects user/nonroot/foreground/incognito pages and unresolved source work',async()=>{
  for(const extra of [{tab:{url:'https://chatgpt.com/c/user'}},{tab:{active:true}},{tab:{incognito:true}},{lease:{requestId:'unknown'}},{commands:{x:{status:'uncertain'}}},{identity:{profileId:'damaged'}},{identity:{backgroundEpoch:''}}]){
    const f=fixture({operation:legacy(),...extra});assert.equal((await f.c.recoverPendingExtensionReload()).reason,'missing');assert.equal(f.events.includes('reload'),false);assert.equal(f.local[KEY],undefined);
  }
});
test('legacy source with compacted old transport still requires a real authenticated server nonce at completion',async()=>{
  const f=fixture({operation:legacy(),identity:{serverInstanceId:''}});assert.equal((await f.c.recoverPendingExtensionReload()).pending,true);
  assert.equal((await f.c.confirmSourceReady(proof({serverInstanceId:'',transportServerInstanceId:''}))).verified,false);
  const result=await f.c.confirmSourceReady(proof());assert.equal(result.verified,true);assert.equal(result.result.previousRuntime.serverInstanceId,'');
});
test('legacy adoption rejects unrelated, already verified, stale or inconsistent terminal operations',async()=>{
  for(const change of [{kind:'download'},{status:'failed'},{updatedAt:0},{result:{recovered:true,connectionVerified:true,sourceTabId:1,tabCount:2}},{result:{recovered:true,sourceTabId:3,tabCount:2}},{result:{recovered:true,sourceTabId:1,tabCount:99}}]){
    const f=fixture({operation:{...legacy(),...change}});assert.equal((await f.c.recoverPendingExtensionReload()).reason,'missing');assert.equal(f.events.includes('reload'),false);
  }
});
