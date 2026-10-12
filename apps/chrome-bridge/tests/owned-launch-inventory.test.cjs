const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),{webcrypto,createHash}=require('node:crypto');
const ID='12345678-1234-4234-8234-123456789abc',PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7',TOKEN='bridge-director-desktop-'+ID,BASE='http://127.0.0.1:53179',PREFIX='chatgptBridgeLaunchedTab:';
const runtime={profileId:PROFILE,bundleId:'a'.repeat(32),backgroundEpoch:'background-original',serverInstanceId:'server-original',sourceClientId:'extension:tab:1',sourceTabId:1,extensionVersion:'2.4.5',contentEpoch:'content-original'};
const sourceRecord={launchToken:'bridge-director-desktop-22345678-1234-4234-8234-123456789abc',requestedUrl:'https://chatgpt.com/',createdAt:1,serverUrl:BASE};
function fixture(options={}){
  let reads=0,windowReads=0;const writes=[],records={[PREFIX+'1']:sourceRecord,...options.records},tabs=[{id:1,url:'https://chatgpt.com/',incognito:false,active:false,...options.sourceTab},...(options.tabs||[])];
  const port={sender:{tab:{id:1,incognito:false}}},connections=new Map([[port,{serverUrl:BASE,tabId:1,protocolReady:true,ws:{readyState:1}}]]);
  const context=vm.createContext({URL,URLSearchParams,TextEncoder,Uint8Array,crypto:webcrypto,Date,Math,setTimeout,console,
    chrome:{runtime:{},storage:{session:{get:async()=>{if(options.storageError)throw Error('unavailable');return {...records};},set:()=>assert.fail('readonly')},local:{set:()=>assert.fail('readonly')}},
      tabs:{query:async()=>{reads++;if(options.queryError)throw Error('unavailable');return options.inventoryTransform?options.inventoryTransform(tabs.map(x=>({...x})),reads):tabs.map(x=>({...x}));},create:()=>assert.fail('must not open'),update:()=>assert.fail('must not navigate'),remove:()=>assert.fail('must not close')}}});
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/tabController.js'),'utf8').replace(/export /g,''),context);
  if(options.windowInfo)context.chrome.windows={get:async id=>{windowReads++;if(options.windowError)throw Error('window unavailable');return {id,focused:false,state:'normal',...options.windowInfo,...(options.windowChanges&&windowReads>1?{focused:true}:{})};}};
  if(options.missingDigest)context.crypto={randomUUID:()=>ID};
  if(options.missingSessionGet)delete context.chrome.storage.session.get;
  if(options.missingTabQuery)delete context.chrome.tabs.query;
  const controller=context.createTabController({connections,safeBridgeServerUrl:x=>x===BASE?x:'',readLaunchedTab:async id=>records[PREFIX+id],
    rememberLaunchedTab:()=>assert.fail('no writes'),forgetLaunchedTab:()=>assert.fail('no cleanup'),isStableLaunchToken:()=>true,
    readInventoryIdentity:options.missingRuntimeReader?undefined:async()=>({...runtime,...options.actualRuntime}),
    readExecutionInventory:options.missingExecutionReader?undefined:async(_port,binding)=>executionFacts(binding,options.execution)});
  return {context,controller,port,reads:()=>reads,windowReads:()=>windowReads,lookup:()=>controller.openBridgeTab(port,{lookupOnly:true,inventoryCommandId:'inventory-read-original',requestId:'ext-rpc-correlation',lookupRequestId:ID,launchToken:TOKEN,expectedRuntime:{...runtime,...options.expectedRuntime},...options.lookupOptions})};
}
function executionFacts(binding,change={}){return {sourceTabId:1,lease:null,ownCommand:{commandId:binding.inventoryCommandId,commandType:'browser.tab.identify',scope:'standalone',mode:'result',operation:'control',status:'dispatched',requestId:''},
  otherUnresolvedCommands:[],ownAcceptedMessageIds:[],otherPendingOutbox:[],queue:{version:'tab-operation-snapshot/v1',tabId:1,
    running:{sequence:1,label:'content:bridge.tab.open',meta:{type:'bridge.tab.open',requestId:binding.rpcRequestId}},queued:[]},...change};}
const targetRecord={launchToken:TOKEN,requestedUrl:'https://chatgpt.com/',createdAt:2,serverUrl:BASE};

test('lookup contract reports exact missing capability or identity without weakening guards or enumerating',async()=>{
  for(const [options,reason] of [[{lookupOptions:{lookupRequestId:''}},'invalid_lookup_request_identity'],
    [{lookupOptions:{launchToken:'other'}},'lookup_launch_token_mismatch'],[{missingRuntimeReader:true},'inventory_runtime_reader_unavailable'],
    [{missingExecutionReader:true},'inventory_execution_reader_unavailable'],
    [{missingDigest:true},'inventory_webcrypto_digest_unavailable'],[{missingSessionGet:true},'inventory_session_reader_unavailable'],
    [{missingTabQuery:true},'inventory_tab_enumerator_unavailable']]){
    const f=fixture(options),out=await f.lookup();assert.equal(out.status,'unknown');assert.equal(out.complete,false);assert.equal(out.reason,reason);assert.equal(f.reads(),0);
  }
});
test('complete current inventory proves absence without opening, navigating, editing storage or relying on clients',async()=>{
  const f=fixture({tabs:[{id:3,url:'https://example.com/private-user-page',incognito:false,active:true}]});const out=await f.lookup();
  assert.equal(out.status,'confirmed_absent');assert.equal(out.complete,true);assert.equal(f.reads(),2);
  assert.equal(out.runtime.profileId,PROFILE);assert.equal(out.runtime.extensionVersion,'2.4.5');assert.equal(out.absenceScope,'current_browser_inventory_only');
  assert.match(out.inventoryGeneration,/^[a-f0-9-]{36}$/);assert.equal(out.inventorySha256,createHash('sha256').update(JSON.stringify(out.inventory)).digest('hex'));
  assert.deepEqual(JSON.parse(out.inventoryCanonicalJson),JSON.parse(JSON.stringify(out.inventory)));
  assert.doesNotMatch(JSON.stringify(out),/private-user-page/);
});
test('persisted unique tab is matched even when its content client is not connected',async()=>{
  const out=await fixture({records:{[PREFIX+'7']:targetRecord},tabs:[{id:7,url:'https://chatgpt.com/',incognito:false,active:false}]}).lookup();
  assert.equal(out.status,'matched');assert.equal(out.match.tabId,7);assert.equal(out.match.connected,false);assert.equal(out.match.createdAt,2);
});
test('duplicate tokens, orphaned records, wrong servers and inaccessible inventories remain unknown',async()=>{
  for(const options of [{records:{[PREFIX+'7']:targetRecord}},
    {records:{[PREFIX+'7']:targetRecord,[PREFIX+'8']:targetRecord}},
    {records:{[PREFIX+'7']:{...targetRecord,serverUrl:'http://other.invalid'}},tabs:[{id:7,url:'https://chatgpt.com/',incognito:false}]},
    {queryError:true},{storageError:true},{tabs:[{id:7,url:'https://chatgpt.com/'}]}]){
    const out=await fixture(options).lookup();assert.equal(out.status,'unknown');assert.equal(out.complete,false);
  }
});
test('foreign profile, bundle, background or server nonce and changing complete inventory cannot prove absence',async()=>{
  for(const key of ['profileId','bundleId','backgroundEpoch','serverInstanceId','sourceClientId','sourceTabId']){
    const out=await fixture({expectedRuntime:{[key]:'foreign'}}).lookup();assert.equal(out.status,'unknown',key);assert.equal(out.reason,'runtime_mismatch',key);
  }
  const out=await fixture({inventoryTransform:(tabs,n)=>n===1?tabs:[...tabs,{id:9,url:'https://chatgpt.com/',incognito:false}]}).lookup();
  assert.equal(out.status,'unknown');assert.equal(out.reason,'inventory_changed_during_read');
});
test('inventory identify mode only calls the readonly lookup branch and keeps transport RPC identity separate',async()=>{
  const f=fixture(),messages=[],calls=[];
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/content/sessionCommands.js'),'utf8'),f.context);
  const api=f.context.ChatGptSessionCommands.createSessionCommands({send:x=>messages.push(x),extensionRequest:async(type,payload)=>{
    calls.push({type,payload});return f.controller.openBridgeTab(f.port,{...payload,requestId:'ext-rpc-overwrites-requestId'});
  }});
  await api.handleBrowserTabIdentify({mode:'owned_launch_inventory_v1',requestId:'',lookupRequestId:ID,launchToken:TOKEN,expectedRuntime:runtime,commandId:'read-original'});
  assert.equal(calls.length,1);assert.equal(calls[0].payload.lookupOnly,true);assert.equal(calls[0].payload.lookupRequestId,ID);
  assert.equal(messages[0].type,'browser.tab.identified');assert.equal(messages[0].requestId,ID);assert.equal(messages[0].status,'confirmed_absent');
});

function serverFixture(result,override={}){
  const text=fs.readFileSync(path.join(__dirname,'../overrides/src/browserBridge.js'),'utf8'),start=text.indexOf('  async ownedLaunchInventory('),end=text.indexOf('  async requestForcedSnapshot(',start),method=text.slice(start,end);
  const context=vm.createContext({createHash,Date,BRIDGE_VERSION:'6.4.0'});
  vm.runInContext(`class InventoryBridge { #serverInstanceId='server-original'; constructor(health,result){this.current=health;this.result=result;this.calls=[];} health(){return this.current;} async #sendCommand(...args){this.calls.push(args);const result=await this.result;return {commandId:result.inventoryCommandId,...result};} ${method} };this.InventoryBridge=InventoryBridge`,context);
  const source={id:runtime.sourceClientId,browserTabId:1,ready:true,compatible:true,browserIncognito:false,activeRequest:null,quarantined:false,focused:false,visibilityState:'hidden',
    url:'https://chatgpt.com/',launchToken:sourceRecord.launchToken,browserProfileId:PROFILE,extensionBundleId:runtime.bundleId,backgroundEpoch:runtime.backgroundEpoch,contentEpoch:runtime.contentEpoch,serverInstanceId:runtime.serverInstanceId,...override};
  return new context.InventoryBridge({clients:[source],pendingRequests:0,pendingCommands:0},result);
}
test('server verifies exact runtime, complete hashed receipt and declared package before forwarding absence',async()=>{
  const receipt=await fixture().lookup(),bridge=serverFixture(receipt);
  const result=await bridge.ownedLaunchInventory(ID,runtime);
  assert.equal(result.status,'confirmed_absent');assert.equal(result.componentVersion,'6.4.0');
  assert.equal(bridge.calls.length,1);assert.equal(bridge.calls[0][0],'browser.tab.identify');
  assert.equal(bridge.calls[0][1].mode,'owned_launch_inventory_v1');assert.equal(bridge.calls[0][2].sourceClientId,runtime.sourceClientId);
  assert.equal(bridge.calls[0][1].lookupRequestId,ID);assert.equal(Object.hasOwn(bridge.calls[0][1],'requestId'),false);
  for(const change of [r=>{r.complete=false},r=>{r.runtime.backgroundEpoch='foreign'},r=>{r.lookupRequestId='other'},r=>{r.inventory.tabs=[]},r=>{r.inventoryGeneration=''},r=>{r.observedAt=1}]){
    const bad=JSON.parse(JSON.stringify(receipt));change(bad);
    assert.equal((await serverFixture(bad).ownedLaunchInventory(ID,runtime)).status,'unknown');
  }
});
test('server does not dispatch readonly lookup through an active, visible, user-owned or mismatched controller',async()=>{
  const receipt=await fixture().lookup();
  for(const override of [{activeRequest:{requestId:'another'}},{focused:true},{visibilityState:'visible'},{launchToken:''},{backgroundEpoch:'foreign'},{quarantined:true}]){
    const bridge=serverFixture(receipt,override),result=await bridge.ownedLaunchInventory(ID,runtime);
    assert.equal(result.status,'unknown');assert.equal(bridge.calls.length,0);
  }
});

// Immutable production Protocol 5 router fixture. It intentionally preserves
// standalone requestId='', so this regression exercises the real overwrite
// that the earlier direct server/content fixtures skipped.
test('HTTP domain lookup identity survives actual standalone outbound and portRouter return requestId overwrites',async()=>{
  const f=fixture(backgroundActiveOptions()),router=fs.readFileSync(path.join(__dirname,'fixtures/serverEnvelopeRouter.upstream-2.4.5.js'),'utf8');
  f.context.document={visibilityState:'hidden',hasFocus:()=>false};
  assert.equal(createHash('sha256').update(router.replace(/\r\n/g,'\n')).digest('hex'),'3938d4d9f4e289c9661c1eefcfb26e5f5ca45bf26a7dd300027f2c98f4c98ad2');
  const definition={mode:'result',operation:'control',retryPolicy:'always',allowDuringLease:true};
  Object.assign(f.context,{MessageType:{TRANSPORT_ACK:'transport.ack',COMMAND_EXECUTE:'command.execute',COMMAND_ACCEPTED:'command.accepted',COMMAND_RESULT:'command.result'},
    ChatGptBridgeCommandManifest:{commandDefinition:()=>definition,validateCommandPayload:()=>({valid:true,errors:[],definition})}});
  vm.runInContext(router.replace(/^import .*;\r?\n/gm,'').replace(/export /g,''),f.context);
  const portRouter=fs.readFileSync(path.join(__dirname,'fixtures/portRouter.upstream-2.4.5.js'),'utf8');
  assert.equal(createHash('sha256').update(portRouter.replace(/\r\n/g,'\n')).digest('hex'),'d56e3c14659f5801afef0995e106bacd03a8231f79a22c6c112a5b8e644cbf73');
  vm.runInContext(portRouter.replace(/^import[\s\S]*?;\r?\n/gm,'').replace(/export /g,''),f.context);
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/content/sessionCommands.js'),'utf8'),f.context);
  let resolveReceipt,rejectReceipt;const receipt=new Promise((resolve,reject)=>{resolveReceipt=resolve;rejectReceipt=reject;}),bridge=serverFixture(receipt),resultPromise=bridge.ownedLaunchInventory(ID,runtime);
  const [commandType,serverPayload]=bridge.calls[0],posted=[];
  const state={tabId:1,clientId:runtime.sourceClientId,contentEpoch:runtime.contentEpoch,connectionEpoch:'connection-original',protocolReady:true};
  const persisted={transport:{serverEpoch:runtime.serverInstanceId},commands:{},commandOrder:[],lease:null};
  let terminalEnvelope;
  const backgroundState={read:async()=>persisted,transition:async(_id,event)=>{
    if(event.type==='command.registered')persisted.commands[event.commandId]={...event,requestId:'',status:'registered'};
    if(event.type==='command.dispatched')persisted.commands[event.commandId].status='dispatched';
    if(event.type==='command.succeeded'){persisted.commands[event.commandId].status='succeeded';terminalEnvelope=event.terminalEnvelope;}
    return {accepted:true,state:persisted};
  }};
  const createEnvelopeDraft=(_state,messageType,body)=>({messageType,body});
  await f.context.handleServerEnvelope({state,envelope:{messageType:'command.execute',body:{...serverPayload,type:commandType,commandId:'transport-read'},
      source:{backgroundEpoch:runtime.serverInstanceId,sequence:1},messageId:'server-envelope-exact',request:null},
    backgroundState,
    sendProtocolMessage:()=>assert.fail('must not reject'),createEnvelopeDraft,flushCriticalOutbox:async()=>{},post:(_port,message)=>posted.push(message)});
  assert.equal(posted.length,1);assert.equal(posted[0].payload.requestId,'');assert.equal(posted[0].payload.lookupRequestId,ID);
  const api=f.context.ChatGptSessionCommands.createSessionCommands({send:payload=>{
    void f.context.handlePayload({backgroundState,createEnvelopeDraft,sendProtocolMessage:()=>assert.fail('not ephemeral'),flushCriticalOutbox:async()=>{
      assert.equal(terminalEnvelope.messageType,'command.result');assert.equal(terminalEnvelope.body.requestId,'');assert.equal(terminalEnvelope.body.lookupRequestId,ID);
      resolveReceipt(JSON.parse(JSON.stringify(terminalEnvelope.body)));
    }},f.port,state,payload).catch(rejectReceipt);
  },extensionRequest:async(type,payload)=>{
    assert.equal(type,'bridge.tab.open');assert.equal(payload.lookupRequestId,ID);
    assert.equal(payload.sourceDocument.visibilityState,'hidden');assert.equal(payload.sourceDocument.focused,false);assert.ok(Number.isSafeInteger(payload.sourceDocument.sampledAt));
    return f.controller.openBridgeTab(f.port,{...payload,requestId:'ext-transport-correlation'});
  }});
  await api.handleBrowserTabIdentify(posted[0].payload);
  const out=await resultPromise;assert.equal(out.status,'confirmed_absent');assert.equal(out.requestId,ID);assert.equal(out.launchToken,TOKEN);assert.equal(f.reads(),2);
  assert.equal(out.lookupRequestId,ID);assert.equal(out.resultType,'browser.tab.identified');
  assert.equal(out.inventory.sourceBackgroundProof.native.windowFocused,false);assert.equal(out.inventory.tabs.find(tab=>tab.id===1).active,true);
});

test('server receipt validator diagnoses each failed field and never derives lookup identity from transport requestId',async()=>{
  const receipt=await fixture().lookup();
  const cases=[[r=>{r.lookupRequestId=''},'inventory_receipt_lookup_identity_mismatch'],[r=>{r.version='wrong'},'inventory_receipt_version_mismatch'],
    [r=>{r.status='wrong'},'inventory_receipt_status_invalid'],[r=>{r.complete=false},'inventory_receipt_incomplete'],
    [r=>{r.launchToken='wrong'},'inventory_receipt_launch_token_mismatch'],[r=>{r.inventoryGeneration=''},'inventory_receipt_generation_invalid'],
    [r=>{r.observedAt='wrong'},'inventory_receipt_observed_at_invalid'],[r=>{r.observedAt=1},'inventory_receipt_stale'],
    [r=>{r.inventory=null},'inventory_receipt_inventory_missing'],[r=>{r.inventory.tabs=[]},'inventory_receipt_canonical_value_mismatch'],
    [r=>{r.inventorySha256='wrong'},'inventory_receipt_sha256_mismatch'],[r=>{r.inventoryCanonicalJson='{'},'inventory_receipt_canonical_json_invalid']];
  for(const field of ['profileId','bundleId','backgroundEpoch','serverInstanceId','sourceClientId','sourceTabId'])cases.push([r=>{r.runtime[field]='wrong'},`inventory_receipt_runtime_${field}_mismatch`]);
  for(const [mutate,reason] of cases){const bad=JSON.parse(JSON.stringify(receipt));mutate(bad);const out=await serverFixture(bad).ownedLaunchInventory(ID,runtime);assert.equal(out.status,'unknown');assert.equal(out.reason,reason);assert.equal(out.complete,false);}
  const transportEmpty={...receipt,requestId:''},out=await serverFixture(transportEmpty).ownedLaunchInventory(ID,runtime);
  assert.equal(out.status,'confirmed_absent');assert.equal(out.requestId,ID);assert.equal(out.lookupRequestId,ID);
});

test('missing independent lookup field remains held despite valid launch token and unrelated standalone requestId',async()=>{
  const f=fixture(),sent=[];
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/content/sessionCommands.js'),'utf8'),f.context);
  const api=f.context.ChatGptSessionCommands.createSessionCommands({send:x=>sent.push(x),extensionRequest:async(type,payload)=>f.controller.openBridgeTab(f.port,{...payload,requestId:'ext-correlation'})});
  await api.handleBrowserTabIdentify({mode:'owned_launch_inventory_v1',requestId:ID,launchToken:TOKEN,expectedRuntime:runtime,commandId:'unknown-field'});
  assert.equal(sent[0].status,'unknown');assert.equal(sent[0].reason,'invalid_lookup_request_identity');assert.equal(f.reads(),0);
});

function backgroundActiveOptions(extra={}){return {sourceTab:{active:true,windowId:5},windowInfo:{},lookupOptions:{sourceDocument:{visibilityState:'hidden',focused:false,sampledAt:Date.now()}},...extra};}
test('selected source tab in natively unfocused background window is readonly with canonical double-sample proof',async()=>{
  for(const state of ['normal','minimized','maximized','fullscreen']){
    const f=fixture(backgroundActiveOptions({windowInfo:{state}})),receipt=await f.lookup();assert.equal(receipt.status,'confirmed_absent');assert.equal(f.reads(),2);assert.equal(f.windowReads(),2);
    assert.equal(receipt.inventory.tabs.find(t=>t.id===1).active,true);assert.equal(receipt.inventory.tabs.find(t=>t.id===1).windowId,5);
    const proof=receipt.inventory.sourceBackgroundProof;assert.equal(proof.nativeSamples,2);assert.equal(proof.native.windowFocused,false);assert.equal(proof.content.visibilityState,'hidden');assert.equal(proof.content.focused,false);assert.equal(proof.content.contentEpoch,runtime.contentEpoch);
    assert.equal(JSON.parse(receipt.inventoryCanonicalJson).sourceBackgroundProof.sourceWindowId,5);assert.equal(receipt.inventorySha256,createHash('sha256').update(receipt.inventoryCanonicalJson).digest('hex'));
    assert.equal((await serverFixture(receipt).ownedLaunchInventory(ID,runtime)).status,'confirmed_absent');
  }
});
test('selected tab stays held without native window proof or current document background evidence',async()=>{
  for(const extra of [{windowInfo:undefined},{sourceTab:{active:true}},{windowInfo:{id:6}},{windowInfo:{focused:true}},{windowInfo:{state:'unknown'}},{windowError:true},
    {lookupOptions:{sourceDocument:{visibilityState:'visible',focused:false,sampledAt:Date.now()}}},{lookupOptions:{sourceDocument:{visibilityState:'hidden',focused:true,sampledAt:Date.now()}}},
    {lookupOptions:{sourceDocument:{visibilityState:'hidden',focused:false,sampledAt:Date.now()-31000}}},{lookupOptions:{sourceDocument:{visibilityState:'hidden',focused:false,sampledAt:Date.now()+31000}}},{lookupOptions:{}}]){
    const f=fixture(backgroundActiveOptions(extra)),out=await f.lookup();assert.equal(out.status,'unknown');assert.equal(out.complete,false);
  }
  const unowned=fixture(backgroundActiveOptions({records:{[PREFIX+'1']:{...sourceRecord,launchToken:'user-page'}}}));assert.equal((await unowned.lookup()).status,'unknown');
});
test('window focus changes during double native inventory cannot prove a background selected source',async()=>{
  const f=fixture(backgroundActiveOptions({windowChanges:true})),out=await f.lookup();assert.equal(out.status,'unknown');assert.equal(out.reason,'inventory_changed_during_read');assert.equal(f.windowReads(),2);
});
test('server validates canonical active-source window/content proof and continues holding visible or active requests',async()=>{
  const receipt=await fixture(backgroundActiveOptions()).lookup();
  const mutations=[r=>{delete r.inventory.sourceBackgroundProof},r=>{r.inventory.sourceBackgroundProof.nativeSamples=1},r=>{r.inventory.sourceBackgroundProof.sourceWindowId=9},
    r=>{r.inventory.sourceBackgroundProof.sourceTabId=9},r=>{r.inventory.sourceBackgroundProof.version='wrong'},r=>{r.inventory.sourceBackgroundProof.native.windowFocused=true},
    r=>{r.inventory.sourceBackgroundProof.native.tabActive=false},r=>{r.inventory.sourceBackgroundProof.native.windowState='unknown'},r=>{r.inventory.sourceBackgroundProof.content.visibilityState='visible'},
    r=>{r.inventory.sourceBackgroundProof.content.focused=true},r=>{r.inventory.sourceBackgroundProof.content.contentEpoch='old'},r=>{r.inventory.sourceBackgroundProof.content.sampledAt=1}];
  for(const mutate of mutations){const bad=JSON.parse(JSON.stringify(receipt));mutate(bad);bad.inventoryCanonicalJson=JSON.stringify(bad.inventory);bad.inventorySha256=createHash('sha256').update(bad.inventoryCanonicalJson).digest('hex');
    const out=await serverFixture(bad).ownedLaunchInventory(ID,runtime);assert.equal(out.status,'unknown');assert.equal(out.reason,'inventory_receipt_source_background_unproved');}
  for(const override of [{focused:true},{visibilityState:'visible'},{activeRequest:{requestId:'unknown'}},{quarantined:true},{contentEpoch:'other'}]){
    const b=serverFixture(receipt,override);assert.equal((await b.ownedLaunchInventory(ID,runtime)).status,'unknown');assert.equal(b.calls.length,0);
  }
});

test('live execution proof excludes only the exact current readonly RPC and its own accepted ACK',async()=>{
  const rpc='ext-rpc-correlation',own=executionFacts({inventoryCommandId:'inventory-read-original',rpcRequestId:rpc});
  own.ownAcceptedMessageIds=['ack-own-read'];own.queue.queued=[{sequence:2,label:'server:transport.ack:unknown',meta:{ackMessageId:'ack-own-read'}}];
  const receipt=await fixture({execution:own}).lookup();assert.equal(receipt.status,'confirmed_absent');
  assert.equal(receipt.inventory.sourceExecutionProof.rpcRequestId,rpc);assert.equal(receipt.inventory.sourceExecutionProof.ownCommand.commandId,receipt.inventoryCommandId);
  assert.equal((await serverFixture(receipt).ownedLaunchInventory(ID,runtime)).status,'confirmed_absent');
  for(const change of [{lease:{requestId:'unknown'}},{otherUnresolvedCommands:[{commandId:'other',status:'uncertain'}]},{otherPendingOutbox:[{messageId:'old-result'}]},
    {ownCommand:{...own.ownCommand,commandId:'different'}},{ownCommand:{...own.ownCommand,scope:'request'}},{ownCommand:{...own.ownCommand,status:'uncertain'}},
    {queue:{...own.queue,running:{...own.queue.running,meta:{type:'bridge.tab.open',requestId:'ext-other-rpc-id'}}}},
    {queue:{...own.queue,queued:[{sequence:2,label:'content:bridge.effect.begin',meta:{requestId:'unknown'}}]}},
    {queue:{...own.queue,queued:[{sequence:2,label:'server:transport.ack:unknown',meta:{ackMessageId:'foreign-ack'}}]}}]){
    const f=fixture({execution:{...own,...change}}),out=await f.lookup();assert.equal(out.status,'unknown');assert.equal(out.reason,'source_native_execution_busy');assert.equal(out.complete,false);
  }
});

test('server live idle receipt rejects stale, self-asserted or altered execution proof even after canonical hash recomputation',async()=>{
  const receipt=await fixture().lookup();
  const cases=[r=>{delete r.inventory.sourceExecutionProof},r=>{r.inventory.sourceExecutionProof.observedAt=Date.now()-6000},
    r=>{r.inventory.sourceExecutionProof.nativeSamples=1},r=>{r.commandId='other'},r=>{r.inventoryCommandId='other'},r=>{r.rpcRequestId='ext-other-request'},
    r=>{r.inventory.sourceExecutionProof.lease={requestId:'unknown'}},r=>{r.inventory.sourceExecutionProof.otherUnresolvedCommands=[{}]},
    r=>{r.inventory.sourceExecutionProof.otherPendingOutbox=[{}]},r=>{r.inventory.sourceExecutionProof.ownCommand.status='uncertain'},
    r=>{r.inventory.sourceExecutionProof.queue.running.meta.requestId='ext-wrong-request'},
    r=>{r.inventory.sourceExecutionProof.queue.queued=[{label:'server:transport.ack:unknown',meta:{ackMessageId:'foreign'}}]}];
  for(const mutate of cases){const bad=JSON.parse(JSON.stringify(receipt));mutate(bad);bad.inventoryCanonicalJson=JSON.stringify(bad.inventory);bad.inventorySha256=createHash('sha256').update(bad.inventoryCanonicalJson).digest('hex');
    const out=await serverFixture(bad).ownedLaunchInventory(ID,runtime);assert.equal(out.status,'unknown');assert.equal(out.reason,'inventory_receipt_source_execution_unproved');assert.equal(out.complete,false);}
});
