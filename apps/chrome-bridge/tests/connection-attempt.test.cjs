const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
test('auth preflight aborts a hung network call and remains retryable without leaking token',async()=>{
 const context=vm.createContext({URL,AbortController,setTimeout,clearTimeout,console});
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/authPreflight.js'),'utf8').replace(/^export /gm,''),context);
 let signal;
 const result=await context.checkBridgeAuth({token:'synthetic-test-secret',serverUrl:'http://127.0.0.1:53179'},async(_url,options)=>{
  signal=options.signal;return new Promise((_,reject)=>signal.addEventListener('abort',()=>reject(new Error('request timed out')),{once:true}));
 },{timeoutMs:5});assert.equal(result.ok,false);assert.equal(result.offline,true);assert.equal(signal.aborted,true);assert.ok(!result.message.includes('synthetic-test-secret'));
 assert.equal((await context.checkBridgeAuth({token:'synthetic-test-secret',serverUrl:'http://127.0.0.1:53179'},async()=>({ok:true}))).ok,true);
});
function transportFixture(){
 const text=fs.readFileSync(path.join(__dirname,'../overrides/extension/background.js'),'utf8');
 const source=text.slice(text.indexOf('function openConnection(state)'),text.indexOf('const connectionWatchdog ='));
 let finish,authCalls=0,transitions=0,timer;
 const sockets=[];
 class WS{static CONNECTING=0;static OPEN=1;constructor(){this.readyState=0;this.handlers={};sockets.push(this);}addEventListener(type,handler){this.handlers[type]=handler;}close(){this.handlers.close?.({code:1000});}}
 const state={port:{},tabId:7,closed:false,serverUrl:'http://127.0.0.1:53179',token:'fixture'},connections=new Map([[state.port,state]]);
 const context=vm.createContext({WebSocket:WS,Promise,console,Date,Number,String,connections,post:()=>{},checkBridgeAuth:()=>{authCalls++;return new Promise(r=>finish=r);},wsUrl:()=>'',
  clearTimeout:()=>{},setTimeout:f=>{timer=f;return 1;},tabOperations:{run:async(_id,fn)=>{transitions++;return fn();}},backgroundState:{transition:async()=>({accepted:true})},TabOperationPriority:{OWNER_INVALIDATION:1},
 });vm.runInContext(source+'\n globalThis.api={openConnection,scheduleReconnect};',context);
 return {api:context.api,state,sockets,authCalls:()=>authCalls,transitions:()=>transitions,finish:()=>finish({ok:true}),fireTimer:()=>timer()};
}
test('same-port timer and alarm share one auth attempt and one socket',async()=>{
 const f=transportFixture(),one=f.api.openConnection(f.state),two=f.api.openConnection(f.state);assert.equal(one,two);assert.equal(f.authCalls(),1);
 f.finish();await Promise.all([one,two]);assert.equal(f.sockets.length,1);await f.api.openConnection(f.state);assert.equal(f.authCalls(),1);
});
test('old socket callbacks and delayed timer cannot disconnect or overwrite its replacement',async()=>{
 const f=transportFixture(),p=f.api.openConnection(f.state);f.finish();await p;const old=f.sockets[0];
 f.api.scheduleReconnect(f.state,old);const current={readyState:1,close:()=>assert.fail('new socket must survive')};f.state.ws=current;
 old.handlers.close({code:1000});old.handlers.error();old.handlers.message({data:'invalid old frame'});old.handlers.open();f.fireTimer();
 assert.equal(f.state.ws,current);assert.equal(f.transitions(),0);assert.equal(f.authCalls(),1);
});
