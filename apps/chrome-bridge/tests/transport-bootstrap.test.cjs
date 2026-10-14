const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const ctx=vm.createContext({URL,Date});
vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/authPreflight.js'),'utf8').replace(/export /g,''),ctx);
const setup=()=>({state:{tabId:7,clientId:'extension:tab:7',contentEpoch:'same',initialPage:{type:'hello',clientVersion:'4.3.12',pageReady:true,composerReady:true,chatMainReady:true,url:'https://chatgpt.com/c/stale',tabObservation:{old:true}}},runtime:{tabId:7,contentEpoch:'same',lease:null},browser:{tabs:{get:async()=>({id:7,windowId:2,url:'https://chatgpt.com/',incognito:false,active:false,title:'ChatGPT'})},windows:{get:async()=>({id:2,focused:false,state:'minimized'})}}});
test('background hello uses native identity but never claims page or model readiness',async()=>{
 const f=setup(),h=await ctx.readOnlyBootstrapHello(f.state,f.runtime,{launchToken:'owned'},f.browser);
 assert.equal(h.url,'https://chatgpt.com/');assert.equal(h.launchToken,'owned');for(const k of ['pageReady','composerReady','chatMainReady'])assert.equal(h[k],false);
 assert.equal(h.tabObservation,null);assert.equal(h.session,null);assert.equal(h.transportHealth.backgroundOnly,true);assert.equal(h.clientVersion,'4.3.12');assert.equal(h.type,undefined);
});
test('original unresolved lease survives bootstrap without allocating or replacing a request',async()=>{
 const f=setup();f.runtime.lease={requestId:'original',leaseId:'original-lease',ownerServerInstanceId:'original-server'};
 const before=JSON.stringify(f.runtime),h=await ctx.readOnlyBootstrapHello(f.state,f.runtime,null,f.browser);
 assert.equal(h.activeRequest,f.runtime.lease);assert.equal(JSON.stringify(f.runtime),before);
});
test('wrong tab content epoch or website cannot bootstrap a source',async()=>{
 for(const kind of ['epoch','runtime-tab','native-tab','origin']){const f=setup();if(kind==='epoch')f.runtime.contentEpoch='changed';if(kind==='runtime-tab')f.runtime.tabId=8;if(kind==='native-tab')f.browser.tabs.get=async()=>({id:8,url:'https://chatgpt.com/',incognito:false});if(kind==='origin')f.browser.tabs.get=async()=>({id:7,url:'https://example.invalid/',incognito:false});await assert.rejects(()=>ctx.readOnlyBootstrapHello(f.state,f.runtime,null,f.browser));}
});
test('native foreground state is reported truthfully without focus or navigation mutations',async()=>{
 const f=setup();f.browser.tabs.get=async()=>({id:7,windowId:2,url:'https://chatgpt.com/',incognito:false,active:true});f.browser.windows.get=async()=>({id:2,focused:true,state:'normal'});
 const h=await ctx.readOnlyBootstrapHello(f.state,f.runtime,null,f.browser);assert.equal(h.focused,true);assert.equal(h.visibilityState,'visible');assert.equal(h.pageReady,false);
});
test('only exact accepted native handshake acknowledgement enables background health',()=>{
 const state={clientId:'extension:tab:7',contentEpoch:'same',pendingHelloReceipt:{messageId:'hello-7',body:{transportHealth:{backgroundOnly:true}},source:{clientId:'extension:tab:7',contentEpoch:'same',backgroundEpoch:'worker',sequence:14}}};
 const e={messageType:'transport.ack',body:{accepted:true,ackMessageId:'hello-7',acceptedSequence:14}};
 assert.equal(ctx.bootstrapAckMatches(state,e,'worker'),true);assert.equal(ctx.bootstrapAckMatches(state,e,'old-worker'),false);
 for(const field of ['accepted','ackMessageId','acceptedSequence']){const wrong=structuredClone(e);wrong.body[field]=field==='accepted'?false:field==='acceptedSequence'?15:'other';assert.equal(ctx.bootstrapAckMatches(state,wrong,'worker'),false);}
 state.contentEpoch='different';assert.equal(ctx.bootstrapAckMatches(state,e,'worker'),false);
});
test('persistence errors retain fixed diagnostic code without copying raw causes',()=>{
 assert.equal(ctx.transportFailureCode({code:'BACKGROUND_STATE_PERSIST_FAILED',cause:'secret'}),'background_state_persist_failed');assert.equal(ctx.transportFailureCode({code:'unexpected secret value'}),'transport_open_failed');
});

test('bootstrap heartbeat cannot take over a real page heartbeat',()=>{
 const e={messageType:'transport.ping'};
 assert.equal(ctx.backgroundPongEligible({backgroundHandshakeReady:true,protocolReady:false},e),true);
 for(const state of [{backgroundHandshakeReady:false},{backgroundHandshakeReady:true,protocolReady:true},{backgroundHandshakeReady:true,closed:true}])assert.equal(ctx.backgroundPongEligible(state,e),false);
 assert.equal(ctx.backgroundPongEligible({backgroundHandshakeReady:true},{messageType:'command.execute'}),false);
});

function integrationFixture({persistError,hung=false}={}){
 const timers=new Map(),messages=[],health=[],sockets=[],posts=[];let timerId=0,sequence=0,operation=Promise.resolve(),transitions=0;
 const state={port:{},clientId:'extension:tab:7',tabId:7,closed:false,contentEpoch:'same',initialPage:{clientVersion:'4.3.12'},launchMetaPromise:Promise.resolve({launchToken:'owned',requestedUrl:'https://chatgpt.com/'})};
 const runtime={tabId:7,contentEpoch:'same',lease:{requestId:'original-request',leaseId:'original-lease'},effectOrder:[],effects:{},outbox:[],metrics:{},transport:{ackCursor:0,serverEpoch:'server'}};
 class WS{static OPEN=1;static CONNECTING=0;constructor(){this.readyState=0;this.handlers={};sockets.push(this);}addEventListener(k,fn){this.handlers[k]=fn;}close(){this.readyState=3;} }
 const context=vm.createContext({URL,Date,Promise,Number,String,Boolean,console,WebSocket:WS,
  chrome:setup().browser,connections:new Map([[state.port,state]]),backgroundEpoch:'worker',backgroundManifestVersion:'2.4.5',
  checkBridgeAuth:async()=>({ok:true}),wsUrl:()=>'',post:(_port,m)=>posts.push(m),
  setTimeout:fn=>{const id=++timerId;timers.set(id,fn);return id;},clearTimeout:id=>timers.delete(id),
  TabOperationPriority:{OWNER_INVALIDATION:1},tabOperations:{metrics:()=>({}),run:(_id,fn)=>operation=operation.then(fn)},
  backgroundState:{read:async()=>runtime,transition:async()=>{transitions++;if(persistError)throw persistError;if(hung)return new Promise(()=>{});return {accepted:true,state:runtime};}},
  reportBackgroundHealth:async body=>health.push(body),isProtocol5Envelope:()=>true,serverEnvelopeQueueOptions:()=>({}),
  handleServerEnvelope:async()=>{},createEnvelopeDraft:()=>{},flushCriticalOutbox:async()=>{},scheduleReleaseDeadline:()=>{},
  readLaunchedTab:async()=>({launchToken:'owned'}),confirmSourceReady:async()=>{},confirmOwnedControllerReady:async()=>{},
  sendProtocolMessage:async(st,type,body)=>{const envelope={messageId:'message-'+(++sequence),messageType:type,source:{clientId:st.clientId,contentEpoch:st.contentEpoch,backgroundEpoch:'worker',sequence},body};messages.push(envelope);if(type==='transport.hello')st.pendingHelloReceipt=envelope;return envelope;},
  MessageType:{TRANSPORT_HELLO:'transport.hello',TRANSPORT_ACK:'transport.ack',TRANSPORT_PONG:'transport.pong'},
 });
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/authPreflight.js'),'utf8').replace(/export /g,''),context);
 // Keep the real pure helpers but intercept the HTTP reporter and auth preflight.
 context.reportBackgroundHealth=async body=>health.push(body);context.checkBridgeAuth=async()=>({ok:true});
 const bg=fs.readFileSync(path.join(__dirname,'../overrides/extension/background.js'),'utf8');vm.runInContext(bg.slice(bg.indexOf('function openConnection(state)'),bg.indexOf('const connectionWatchdog ='))+'\nglobalThis.open=openConnection;',context);
 const tick=async()=>{for(let i=0;i<20;i++)await Promise.resolve();};
 return {state,runtime,messages,health,sockets,posts,timers,transitions:()=>transitions,tick,
  start:async()=>{await context.open(state);sockets[0].readyState=1;sockets[0].handlers.open();await tick();},
  receive:async e=>{sockets[0].handlers.message({data:JSON.stringify(e)});await tick();},reopen:()=>context.open(state)};
}
test('full background socket handshake and heartbeat preserve the same request without prompt commands',async()=>{
 const f=integrationFixture();await f.start();const h=f.messages[0];assert.equal(h.messageType,'transport.hello');assert.equal(h.body.pageReady,false);assert.equal(f.state.protocolReady,false);
 await f.receive({messageType:'transport.ack',source:{backgroundEpoch:'server'},body:{accepted:true,ackMessageId:h.messageId,acceptedSequence:h.source.sequence}});
 assert.equal(f.state.backgroundHandshakeReady,true);await f.receive({messageType:'transport.ping',source:{backgroundEpoch:'server'},body:{type:'ping'}});
 assert.equal(f.messages.at(-1).messageType,'transport.pong');assert.equal(f.messages.at(-1).body.activeRequest.requestId,'original-request');assert.equal(f.messages.at(-1).body.pageReady,false);
 f.state.protocolReady=true;const count=f.messages.length;await f.receive({messageType:'transport.ping',source:{backgroundEpoch:'server'},body:{type:'ping'}});assert.equal(f.messages.length,count);
 assert.equal(f.messages.some(m=>m.messageType.startsWith('command.')),false);
});
test('wrong bootstrap ACK cannot enable the fallback heartbeat',async()=>{
 const f=integrationFixture();await f.start();await f.receive({messageType:'transport.ack',source:{backgroundEpoch:'server'},body:{accepted:true,ackMessageId:'other',acceptedSequence:99}});
 await f.receive({messageType:'transport.ping',source:{backgroundEpoch:'server'},body:{type:'ping'}});assert.equal(f.state.backgroundHandshakeReady,false);assert.equal(f.messages.length,1);
});
test('storage rejection reports exact error class independently of the page port',async()=>{
 const error=Object.assign(Error('persist rejected'),{code:'BACKGROUND_STATE_PERSIST_FAILED',eventType:'transport.connected',stateBytes:9000000});const f=integrationFixture({persistError:error});await f.start();
 assert.equal(f.messages.length,0);assert.equal(f.health.at(-1).recovery.reason,'background_state_persist_failed');assert.equal(f.health.at(-1).connectionCount,0);assert.equal(f.posts.at(-1).eventType,'transport.connected');assert.equal(f.posts.at(-1).stateBytes,9000000);assert.equal(f.runtime.lease.requestId,'original-request');
});
test('hung storage has a diagnostic deadline without releasing uncertain write or duplicating socket',async()=>{
 const f=integrationFixture({hung:true});await f.start();for(const fn of f.timers.values())fn();await f.tick();await f.reopen();
 assert.equal(f.health.at(-1).recovery.reason,'transport_open_timeout');assert.equal(f.messages.length,0);assert.equal(f.sockets.length,1);assert.equal(f.transitions(),1);assert.equal(f.runtime.lease.requestId,'original-request');
});
