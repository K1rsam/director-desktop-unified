'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),{EventEmitter}=require('node:events');
const ORIGINAL='12345678-1234-4234-8234-123456789abc',ALIAS='bridge-director-desktop-22345678-1234-4234-8234-123456789abc';
const PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7',TARGET='https://chatgpt.com/c/11111111-1111-4111-8111-111111111111';
const turn=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(options={}){
  const calls=[],hub=new EventEmitter(),source={id:'extension:tab:31',browserTabId:31,launchToken:ALIAS,browserProfileId:PROFILE,extensionBundleId:'a'.repeat(32),backgroundEpoch:'background-original',serverInstanceId:'server-original',
    transport:'extension',ready:true,compatible:true,browserIncognito:false,activeRequest:null,quarantined:false,releasePending:false,releaseState:'idle',focused:false,visibilityState:'hidden',contentEpoch:'content-original',url:'https://chatgpt.com/',requestedUrl:TARGET,...options.source};
  hub.clients=[source];hub.activeClient={id:'user-page'};
  const health={pendingRequests:0,pendingCommands:0,...options.health};
  const native={status:'confirmed_absent',complete:true,requestId:ORIGINAL,sourceClientId:source.id,runtime:{sourceTabId:31},inventory:{sourceExecutionProof:{observedAt:Date.now()}},...options.native};
  const text=fs.readFileSync(path.join(__dirname,'../overrides/src/browserBridge.js'),'utf8'),start=text.indexOf('  async recoverOwnedConversation('),end=text.indexOf('  async closeBrowserTab(',start),method=text.slice(start,end);
  const tabText=fs.readFileSync(path.join(__dirname,'../overrides/src/bridge/coordinator/browserTabCoordinator.js'),'utf8').replace(/^import[\s\S]*?from [^;]+;\r?\n/gm,'').replace(/export (?=function|class)/g,'');
  const context=vm.createContext({URL,Date,makeRequestId:()=>assert.fail('no new identity'),BROWSER_LAUNCH_TOKEN_RE:/^bridge-/,browserLaunchMetadataFromUrl:()=>({}),safeChatGptUrl:x=>x,normalizeLaunchedClient:x=>x,
    setTimeout(callback){const timer=setTimeout(callback,80);timer.unref=()=>timer;return timer;},clearTimeout});
  vm.runInContext(tabText+`;class TestBridge { #serverInstanceId='server-original'; #conversationNavigations=new Set(); #browserClients;
    constructor(deps){Object.assign(this,deps);this.#browserClients={tabs:new BrowserTabCoordinator({hub:deps.hub,runtimeOptions:{},sendCommand:()=>{throw Error('no tab operations')},rankClients:x=>x})};}
    health(){return {...this.current,clients:this.hub.clients.map(client=>({...client}))};}
    ownedLaunchInventory(...args){return this.inventory(...args);}
    #sendCommand(...args){return this.command(...args);}
    ${method}
  };this.TestBridge=TestBridge;`,context);
  const navigate=(change={})=>{Object.assign(source,{url:TARGET,contentEpoch:'content-after-navigation'},change);const event={...source};delete event.releasePending;delete event.releaseState;hub.emit('client.activity',{client:event});};
  const bridge=new context.TestBridge({hub,current:health,inventory:async(...args)=>{calls.push(['inventory',...args]);if(options.afterInventory)await options.afterInventory(source,health);return native;},
    command:async(...args)=>{calls.push(args);if(options.beforeReadback)await options.beforeReadback(source,health);if(options.navigate!==false)navigate(options.afterNavigation);if(options.interrupted)throw Error('old content disconnected');return {selected:true};}});
  const request={sourceClientId:source.id,expectedLaunchToken:ALIAS,expectedCurrentUrl:'https://chatgpt.com/',targetUrl:TARGET,originalRequestId:ORIGINAL,timeoutMs:30000};
  return {bridge,hub,source,health,native,calls,request,navigate,run:(change={})=>bridge.recoverOwnedConversation({...request,...change})};
}
function noNavigation(f){assert.equal(f.calls.filter(call=>call[0]==='sessions.select').length,0);}

test('native idle original-page absence precedes one same-alias navigation and full URL readback',async()=>{
  const f=fixture(),out=await f.run();assert.equal(out.status,'restored');assert.equal(out.version,'director-owned-conversation-navigation/v1');
  assert.equal(out.tabId,31);assert.equal(out.launchToken,ALIAS);assert.equal(out.client.url,TARGET);assert.equal(out.commandOutcome,'received');assert.equal(out.submitCalled,false);assert.equal(out.openedNewTab,false);
  assert.equal(f.calls.length,2);assert.equal(f.calls[0][0],'inventory');assert.equal(f.calls[0][1],ORIGINAL);assert.equal(f.calls[0][2].sourceClientId,f.source.id);
  assert.equal(f.calls[1][0],'sessions.select');assert.equal(f.calls[1][1].sessionId,TARGET);assert.equal(f.calls[1][2].sourceClientId,f.source.id);assert.equal(f.hub.activeClient.id,'user-page');
});

test('navigation command disconnect still completes by observing the same owned page without a second command',async()=>{
  const f=fixture({interrupted:true}),out=await f.run();assert.equal(out.status,'restored');assert.equal(out.commandOutcome,'interrupted');assert.equal(f.calls.filter(call=>call[0]==='sessions.select').length,1);
});

test('invalid identity or nonexact conversation target is rejected before native inspection',async()=>{
  for(const change of [{sourceClientId:''},{expectedLaunchToken:'user-page'},{expectedLaunchToken:'bridge-director-desktop-'+ORIGINAL},{originalRequestId:''},{expectedCurrentUrl:TARGET},
    {targetUrl:'https://example.com/c/11111111-1111-4111-8111-111111111111'},{targetUrl:TARGET+'?x=1'},{targetUrl:TARGET+'#x'},{targetUrl:'https://chatgpt.com/c/not-a-uuid'}]){
    const f=fixture();await assert.rejects(f.run(change),{code:'CHROME_OWNED_NAVIGATION_HELD'});assert.equal(f.calls.length,0);
  }
});

test('foreground, busy, unowned, changed request target, or duplicate launch clients cannot navigate',async()=>{
  const cases=[{source:{focused:true}},{source:{visibilityState:'visible'}},{source:{browserIncognito:true}},{source:{activeRequest:{requestId:'busy'}}},{source:{releasePending:true}},{source:{quarantined:true}},
    {source:{transport:'userscript'}},{source:{ready:false}},{source:{compatible:false}},{source:{requestedUrl:'https://chatgpt.com/'}},{source:{url:TARGET}},{source:{launchToken:'foreign'}},
    {source:{browserTabId:'31'}},{source:{contentEpoch:''}},{source:{backgroundEpoch:''}},{source:{serverInstanceId:'foreign'}},{health:{pendingRequests:1}},{health:{pendingCommands:1}}];
  for(const change of cases){const f=fixture(change);await assert.rejects(f.run(),{code:'CHROME_OWNED_NAVIGATION_HELD'});assert.equal(f.calls.length,0);}
  const f=fixture();f.hub.clients.push({...f.source,id:'duplicate',browserTabId:32});await assert.rejects(f.run(),{code:'CHROME_OWNED_NAVIGATION_HELD'});assert.equal(f.calls.length,0);
});

test('unknown native queue, original still present, stale proof, or changed alias blocks dispatch',async()=>{
  for(const change of [{native:{status:'unknown'}},{native:{status:'matched',match:{tabId:99}}},{native:{complete:false}},{native:{requestId:'other'}},{native:{runtime:{sourceTabId:32}}},
    {native:{inventory:{sourceExecutionProof:{observedAt:1}}}},{afterInventory:source=>{source.contentEpoch='different-document';}},
    {afterInventory:(_source,health)=>{health.pendingCommands=1;}}]){
    const f=fixture(change);await assert.rejects(f.run(),{code:'CHROME_OWNED_NAVIGATION_HELD'});noNavigation(f);assert.equal(f.calls.length,1);
  }
});

test('wrong post-navigation URL, tab, launch, profile or runtime never passes readback and never repeats navigation',async()=>{
  for(const afterNavigation of [{url:'https://chatgpt.com/'},{browserTabId:32},{launchToken:'foreign'},{browserProfileId:'foreign'},{backgroundEpoch:'foreign'},{focused:true}]){
    const f=fixture({afterNavigation});await assert.rejects(f.run(),{code:'CHROME_OWNED_NAVIGATION_HELD'});assert.equal(f.calls.filter(call=>call[0]==='sessions.select').length,1);
    for(const event of ['client.ready','client.changed','client.activity'])assert.equal(f.hub.listenerCount(event),0);
  }
});

test('concurrent recovery of the same alias is held while the first call can finish its original readback',async()=>{
  const f=fixture({navigate:false}),pending=f.run();await turn();await assert.rejects(f.run(),error=>error.reason==='original_navigation_in_progress');
  f.navigate();assert.equal((await pending).status,'restored');assert.equal(f.calls.length,2);
});

test('HTTP route forwards only explicit recovery fields and never selects a global client',async()=>{
  const source=fs.readFileSync(path.join(__dirname,'../overrides/src/routes.js'),'utf8'),start=source.indexOf("  router.post('/browser/tabs/recover-owned-conversation'"),end=source.indexOf("  router.post('/browser/tabs/reload'",start);let handler,received;
  vm.runInNewContext(source.slice(start,end),{router:{post:(route,fn)=>{assert.equal(route,'/browser/tabs/recover-owned-conversation');handler=fn;}},bridge:{recoverOwnedConversation:async request=>{received=request;return {status:'restored'};}}});
  const f=fixture(),body={...f.request,select:true,active:true,prompt:'must never pass'};let result;
  await handler({body},{json:value=>{result=value;}},error=>{throw error;});assert.equal(result.ok,true);assert.equal(result.status,'restored');assert.deepEqual(JSON.parse(JSON.stringify(received)),f.request);
});
