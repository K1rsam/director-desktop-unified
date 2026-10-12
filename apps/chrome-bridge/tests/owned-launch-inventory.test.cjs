const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),{webcrypto,createHash}=require('node:crypto');
const ID='12345678-1234-4234-8234-123456789abc',PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7',TOKEN='bridge-director-desktop-'+ID,BASE='http://127.0.0.1:53179',PREFIX='chatgptBridgeLaunchedTab:';
const runtime={profileId:PROFILE,bundleId:'a'.repeat(32),backgroundEpoch:'background-original',serverInstanceId:'server-original',sourceClientId:'extension:tab:1',sourceTabId:1,extensionVersion:'2.4.5',contentEpoch:'content-original'};
const sourceRecord={launchToken:'bridge-director-desktop-22345678-1234-4234-8234-123456789abc',requestedUrl:'https://chatgpt.com/',createdAt:1,serverUrl:BASE};
function fixture(options={}){
  let reads=0;const writes=[],records={[PREFIX+'1']:sourceRecord,...options.records},tabs=[{id:1,url:'https://chatgpt.com/',incognito:false,active:false},...(options.tabs||[])];
  const port={sender:{tab:{id:1,incognito:false}}},connections=new Map([[port,{serverUrl:BASE,tabId:1,protocolReady:true,ws:{readyState:1}}]]);
  const context=vm.createContext({URL,URLSearchParams,TextEncoder,Uint8Array,crypto:webcrypto,Date,Math,setTimeout,console,
    chrome:{runtime:{},storage:{session:{get:async()=>{if(options.storageError)throw Error('unavailable');return {...records};},set:()=>assert.fail('readonly')},local:{set:()=>assert.fail('readonly')}},
      tabs:{query:async()=>{reads++;if(options.queryError)throw Error('unavailable');return options.inventoryTransform?options.inventoryTransform(tabs.map(x=>({...x})),reads):tabs.map(x=>({...x}));},create:()=>assert.fail('must not open'),update:()=>assert.fail('must not navigate'),remove:()=>assert.fail('must not close')}}});
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/tabController.js'),'utf8').replace(/export /g,''),context);
  const controller=context.createTabController({connections,safeBridgeServerUrl:x=>x===BASE?x:'',readLaunchedTab:async id=>records[PREFIX+id],
    rememberLaunchedTab:()=>assert.fail('no writes'),forgetLaunchedTab:()=>assert.fail('no cleanup'),isStableLaunchToken:()=>true,
    readInventoryIdentity:async()=>({...runtime,...options.actualRuntime})});
  return {context,controller,port,reads:()=>reads,lookup:()=>controller.openBridgeTab(port,{lookupOnly:true,requestId:'ext-rpc-correlation',lookupRequestId:ID,launchToken:TOKEN,expectedRuntime:{...runtime,...options.expectedRuntime}})};
}
const targetRecord={launchToken:TOKEN,requestedUrl:'https://chatgpt.com/',createdAt:2,serverUrl:BASE};
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
  await api.handleBrowserTabIdentify({mode:'owned_launch_inventory_v1',requestId:ID,launchToken:TOKEN,expectedRuntime:runtime,commandId:'read-original'});
  assert.equal(calls.length,1);assert.equal(calls[0].payload.lookupOnly,true);assert.equal(calls[0].payload.lookupRequestId,ID);
  assert.equal(messages[0].type,'browser.tab.identified');assert.equal(messages[0].requestId,ID);assert.equal(messages[0].status,'confirmed_absent');
});

function serverFixture(result,override={}){
  const text=fs.readFileSync(path.join(__dirname,'../overrides/src/browserBridge.js'),'utf8'),start=text.indexOf('  async ownedLaunchInventory('),end=text.indexOf('  async requestForcedSnapshot(',start),method=text.slice(start,end);
  const context=vm.createContext({createHash,Date,BRIDGE_VERSION:'6.4.0'});
  vm.runInContext(`class InventoryBridge { #serverInstanceId='server-original'; constructor(health,result){this.current=health;this.result=result;this.calls=[];} health(){return this.current;} async #sendCommand(...args){this.calls.push(args);return this.result;} ${method} };this.InventoryBridge=InventoryBridge`,context);
  const source={id:runtime.sourceClientId,browserTabId:1,ready:true,compatible:true,browserIncognito:false,activeRequest:null,quarantined:false,focused:false,visibilityState:'hidden',
    url:'https://chatgpt.com/',launchToken:sourceRecord.launchToken,browserProfileId:PROFILE,extensionBundleId:runtime.bundleId,backgroundEpoch:runtime.backgroundEpoch,serverInstanceId:runtime.serverInstanceId,...override};
  return new context.InventoryBridge({clients:[source],pendingRequests:0,pendingCommands:0},result);
}
test('server verifies exact runtime, complete hashed receipt and declared package before forwarding absence',async()=>{
  const receipt=await fixture().lookup(),bridge=serverFixture(receipt);
  const result=await bridge.ownedLaunchInventory(ID,runtime);
  assert.equal(result.status,'confirmed_absent');assert.equal(result.componentVersion,'6.4.0');
  assert.equal(bridge.calls.length,1);assert.equal(bridge.calls[0][0],'browser.tab.identify');
  assert.equal(bridge.calls[0][1].mode,'owned_launch_inventory_v1');assert.equal(bridge.calls[0][2].sourceClientId,runtime.sourceClientId);
  for(const change of [r=>{r.complete=false},r=>{r.runtime.backgroundEpoch='foreign'},r=>{r.requestId='other'},r=>{r.inventory.tabs=[]},r=>{r.inventoryGeneration=''},r=>{r.observedAt=1}]){
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
