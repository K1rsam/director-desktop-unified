'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),{EventEmitter}=require('node:events');
const TOKEN='bridge-director-desktop-12345678-1234-4234-8234-123456789abc';
const TARGET='https://chatgpt.com/c/11111111-1111-4111-8111-111111111111';
const turn=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(options={}){
  const text=fs.readFileSync(path.join(__dirname,'../overrides/src/bridge/coordinator/browserTabCoordinator.js'),'utf8')
    .replace(/^import[\s\S]*?from [^;]+;\r?\n/gm,'').replace(/export (?=function|class)/g,'');
  const context=vm.createContext({URL,Date,makeRequestId:()=>assert.fail('must preserve the supplied launch identity'),
    BROWSER_LAUNCH_TOKEN_RE:/^bridge-/,browserLaunchMetadataFromUrl:()=>({launchToken:TOKEN}),safeChatGptUrl:value=>new URL(value).toString(),normalizeLaunchedClient:value=>value,
    setTimeout(callback){const timer=setTimeout(callback,80);timer.unref=()=>timer;return timer;},clearTimeout});
  vm.runInContext(text+';this.Coordinator=BrowserTabCoordinator',context);
  const hub=new EventEmitter(),source={id:'controller',browserTabId:7,ready:true,compatible:true,capabilities:{browserTabs:true},url:'https://chatgpt.com/'};
  hub.clients=[source,...(options.clients||[])];hub.activeClient=source;
  const calls=[],receipt={tabId:31,launchToken:TOKEN,requestedUrl:TARGET,...options.receipt};
  const coordinator=new context.Coordinator({hub,rankClients:value=>value,runtimeOptions:{publicBaseUrl:'http://127.0.0.1:53179',openExternalUrl:()=>assert.fail('no system fallback')},
    sendCommand:async(...args)=>{calls.push(args);return receipt;}});
  coordinator.openSystemBrowserTab=()=>assert.fail('must not replace an unresolved open');
  return {hub,calls,source,coordinator,open:()=>coordinator.openBrowserTab({url:TARGET,launchToken:TOKEN,sourceClientId:'controller',active:false,allowSystemFallback:false}),
    emit:(event,value)=>hub.emit(event,event==='client.activity'?{client:value}:value)};
}
function page(change={}){return {id:'extension:tab:31',browserTabId:31,launchToken:TOKEN,url:TARGET,ready:true,compatible:true,...change};}
function noListeners(f){for(const event of ['client.ready','client.changed','client.activity'])assert.equal(f.hub.listenerCount(event),0,event);}

test('already connected exact created page completes one background open and leaves selection unchanged',async()=>{
  const f=fixture({clients:[page()]}),result=await f.open();assert.equal(result.client.url,TARGET);assert.equal(result.client.browserTabId,31);
  assert.equal(f.calls.length,1);assert.equal(f.calls[0][0],'browser.tab.open');assert.equal(f.calls[0][1].active,false);assert.equal(f.calls[0][1].url,TARGET);
  assert.equal(f.hub.activeClient,f.source);noListeners(f);
});

test('homepage hello cannot finish the open before the same created page reaches its target',async()=>{
  const f=fixture({clients:[page({url:'https://chatgpt.com/'})]});let settled=false;
  const pending=f.open().then(value=>{settled=true;return value;});await turn();f.emit('client.ready',page({url:'https://chatgpt.com/'}));await turn();assert.equal(settled,false);
  f.emit('client.ready',page());assert.equal((await pending).client.url,TARGET);assert.equal(f.calls.length,1);noListeners(f);
});

test('same page navigation observations complete readiness without requiring another hello',async()=>{
  for(const event of ['client.changed','client.activity']){
    const f=fixture({clients:[page({url:'https://chatgpt.com/'})]}),pending=f.open();await turn();f.emit(event,page());
    assert.equal((await pending).client.url,TARGET);assert.equal(f.calls.length,1);noListeners(f);
  }
});

test('wrong conversation, query, fragment, or origin cannot finish the saved exact URL open',async()=>{
  const f=fixture(),pending=f.open();const rejected=assert.rejects(pending,/Timed out waiting/);await turn();
  for(const url of ['https://chatgpt.com/',TARGET+'-other',TARGET+'?other=1',TARGET+'#other','https://chat.openai.com/c/11111111-1111-4111-8111-111111111111'])f.emit('client.ready',page({url}));
  await rejected;assert.equal(f.calls.length,1);noListeners(f);
});

test('same token in another tab or a nonnumeric tab ID never adopts that client',async()=>{
  const f=fixture({clients:[page({browserTabId:99})]});let settled=false;
  const pending=f.open().then(value=>{settled=true;return value;});await turn();
  for(const browserTabId of [99,'31',null])f.emit('client.ready',page({browserTabId}));await turn();assert.equal(settled,false);
  f.emit('client.ready',page());assert.equal((await pending).client.browserTabId,31);noListeners(f);
});

test('wrong or absent launch token, incompatible or unready clients remain pending',async()=>{
  const f=fixture(),pending=f.open();const rejected=assert.rejects(pending,/Timed out waiting/);await turn();
  for(const change of [{launchToken:''},{launchToken:'other'},{ready:false},{compatible:false},{compatibility:{compatible:false}}])f.emit('client.ready',page(change));
  await rejected;assert.equal(f.calls.length,1);noListeners(f);
});

test('invalid native creation receipt fails closed without waiting or a replacement open',async()=>{
  for(const receipt of [{tabId:null},{tabId:0},{tabId:'31'},{tabId:1.5},{launchToken:'other'},{requestedUrl:'https://chatgpt.com/'}]){
    const f=fixture({receipt});await assert.rejects(f.open(),{code:'BROWSER_TAB_OPEN_RECEIPT_MISMATCH'});assert.equal(f.calls.length,1);noListeners(f);
  }
});
