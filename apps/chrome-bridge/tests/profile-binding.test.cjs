'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7';
function load(relative,extras={}) {
  const source=fs.readFileSync(path.join(__dirname,'../overrides',relative),'utf8').replace(/^import .*;\r?\n/gm,'').replace(/export (?=function|class)/g,'');
  const context=vm.createContext({console,URL,structuredClone,setTimeout,clearTimeout,Date,...extras});vm.runInContext(source,context);return context;
}
function storage(initial={}) {let value={...initial},writes=0;return {get:async key=>({[key]:value[key]}),set:async patch=>{writes++;Object.assign(value,patch)},get writes(){return writes}};}
test('background profile UUID persists across worker restarts and concurrent initialization, without rewriting damaged identity',async()=>{
  const context=load('extension/background/outboxV5.js'),store=storage();
  const first=context.createBrowserProfileIdentity(store,()=>PROFILE);
  assert.deepEqual(await Promise.all([first(),first(),first()]),[PROFILE,PROFILE,PROFILE]);assert.equal(store.writes,1);
  assert.equal(await context.createBrowserProfileIdentity(store,()=>{throw Error('must not rotate')})(),PROFILE);
  const damaged=storage({'directorDesktop:browserProfileId:v1':'broken'});
  await assert.rejects(context.createBrowserProfileIdentity(damaged,()=>PROFILE)(),/invalid/);assert.equal(damaged.writes,0);
  await assert.rejects(context.createBrowserProfileIdentity(null,()=>PROFILE)(),/unavailable/);
});
test('hello identity comes from background storage and port incognito state, retaining runtime/lease and non-hello behavior',async()=>{
  const sent=[],store=storage();
  const context=load('extension/background/outboxV5.js',{chrome:{storage:{local:store}},crypto:{randomUUID:()=>PROFILE},WebSocket:{OPEN:1},
    makeEnvelope:(type,body,source,sequence)=>({messageType:type,body,source,sequence}),messageDefinition:()=>({}),isCriticalMessageType:()=>false});
  const outbox=context.createProtocolOutbox({backgroundEpoch:'runtime-original',backgroundState:{transition:async()=>({accepted:true,state:{lease:{requestId:'original-task'},contentEpoch:'content',transport:{outboundSequence:1}}})},post:()=>{},summarize:x=>x});
  const state={tabId:123,clientId:'tab:123',port:{sender:{tab:{incognito:true}}},ws:{readyState:1,send:value=>sent.push(JSON.parse(value))}};
  await outbox.sendProtocolMessage(state,'transport.hello',{browserProfileId:'page-spoof',browserIncognito:false});
  assert.equal(sent[0].body.browserProfileId,PROFILE);assert.equal(sent[0].body.browserIncognito,true);assert.equal(sent[0].source.backgroundEpoch,'runtime-original');assert.equal(sent[0].source.lease.requestId,'original-task');
  await outbox.sendProtocolMessage(state,'tab.observation',{value:'unchanged'});assert.deepEqual(sent[1].body,{value:'unchanged'});assert.equal(store.writes,1);
});
test('server hello validates profile identity and projects unknown legacy/incognito truthfully',()=>{
  const projection=load('src/bridge/hub/clientProjection.js');
  const context=load('src/bridge/hub/clientMessageRouter.js',{browserLaunchMetadataFromUrl:()=>({}),evaluateExtensionCompatibility:()=>({compatible:true}),log:()=>{},
    activeRequestFromPayload:()=>null,normalizeClientSession:()=>null,normalizeTabObservation:()=>null});
  const Router=vm.runInContext('HubClientMessageRouter',context),client={id:'tab',connectedAt:Date.now(),lastSeenAt:Date.now()},clients=new Map([['tab',client]]);
  const router=new Router({clients,getSelectedClientId:()=>'',setSelectedClientId:()=>{},recordDebugEvent:()=>{},publicClient:x=>x,emit:()=>{},sendCompatibility:()=>{}});
  const envelope={messageType:'transport.hello',source:{clientId:'tab',backgroundEpoch:'original-runtime',contentEpoch:'content'}};
  router.handle(client,{type:'hello',browserProfileId:PROFILE,browserIncognito:false},envelope);
  assert.equal(client.browserProfileId,PROFILE);assert.equal(client.backgroundEpoch,'original-runtime');
  assert.equal(projection.publicClientProjection(client).browserIncognito,false);
  router.handle(client,{type:'hello'},envelope);assert.equal(projection.publicClientProjection(client).browserProfileId,'');assert.equal(projection.publicClientProjection(client).browserIncognito,null);
  router.handle(client,{type:'hello',browserProfileId:'broken',browserIncognito:false},envelope);assert.equal(client.browserProfileId,'');
});
test('identity storage failure preserves source-bound transport but removes spoofed profile proof for new work',async()=>{
  const sent=[];
  const context=load('extension/background/outboxV5.js',{chrome:{storage:{local:{get:async()=>{throw Error('offline')}}}},WebSocket:{OPEN:1},
    makeEnvelope:(type,body,source)=>({messageType:type,body,source}),messageDefinition:()=>({}),isCriticalMessageType:()=>false});
  const outbox=context.createProtocolOutbox({backgroundEpoch:'original-runtime',backgroundState:{transition:async()=>({accepted:true,state:{contentEpoch:'content',transport:{outboundSequence:1},lease:{requestId:'original-task'}}})},post:()=>{},summarize:x=>x});
  await outbox.sendProtocolMessage({tabId:123,clientId:'original-tab',port:{sender:{tab:{incognito:false}}},ws:{readyState:1,send:value=>sent.push(JSON.parse(value))}},'transport.hello',{browserProfileId:PROFILE});
  assert.equal(sent[0].body.browserProfileId,'');assert.equal(sent[0].body.browserIncognito,false);assert.equal(sent[0].source.clientId,'original-tab');assert.equal(sent[0].source.lease.requestId,'original-task');
});
test('only owned Director work tabs disable auto discard; normal bridge/user pages stay unchanged and background',async()=>{
  const calls=[];
  const context=load('extension/background/tabController.js',{chrome:{runtime:{},tabs:{create:(options,done)=>{calls.push(['create',options]);done({id:123})},update:(id,options,done)=>{calls.push(['update',id,options]);done({id})}}}});
  const port={sender:{tab:{id:1}}};
  const controller=context.createTabController({connections:new Map([[port,{serverUrl:'http://127.0.0.1:53179'}]]),safeBridgeServerUrl:x=>x,rememberLaunchedTab:async()=>{},readLaunchedTab:async()=>null,forgetLaunchedTab:async()=>{},isStableLaunchToken:()=>true});
  await controller.openBridgeTab(port,{active:false,launchToken:'bridge-director-desktop-'+PROFILE});
  assert.equal(calls[0][1].active,false);assert.equal(calls[1][2].active,false);assert.equal(calls[1][2].autoDiscardable,false);
  for (const launchToken of ['bridge-other-task','bridge-director-desktop-fake','bridge-director-desktop-'+PROFILE+'-suffix']) {
    calls.length=0;await controller.openBridgeTab(port,{active:false,launchToken});assert.equal(Object.hasOwn(calls[1][2],'autoDiscardable'),false);
  }
});
