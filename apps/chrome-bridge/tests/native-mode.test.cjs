const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.resolve(__dirname,'..'),background=fs.readFileSync(path.join(root,'overrides/extension/background.js'),'utf8');
function allocate(native,fail=false){
 let legacy=0,nativeCalls=0;
 const socket={readyState:0},state={serverUrl:'http://127.0.0.1:53179',token:'synthetic-private-token'};
 const code=background.slice(background.indexOf('    const nativeEnabled='),background.indexOf('    state.ws = ws;'));
 const ctx=vm.createContext({state,nativeTransportFactory:null,chrome:{runtime:{getManifest:()=>({permissions:native?['nativeMessaging']:[]})}},
  createNativeTransportFactory:()=>({connect:options=>{nativeCalls++;assert.equal(options.serverUrl,state.serverUrl);assert.equal(options.token,state.token);if(fail)throw Error('native_host_unavailable');return socket;}}),
  WebSocket:class{constructor(){legacy++;}},wsUrl:()=>''});
 const run=()=>vm.runInContext('(function(){let ws;'+code+';return ws;})()',ctx);
 return {run,state,socket,counts:()=>({legacy,nativeCalls})};
}
test('explicit native permission uses the native port and preserves connection credentials',()=>{
 const f=allocate(true);assert.equal(f.run(),f.socket);assert.equal(f.state.transportKind,'native_messaging');assert.deepEqual(f.counts(),{legacy:0,nativeCalls:1});
});
test('native failure cannot silently fall back to a second WebSocket transport',()=>{
 const f=allocate(true,true);assert.throws(f.run,/native_host_unavailable/);assert.deepEqual(f.counts(),{legacy:0,nativeCalls:1});
});
test('old permission manifest retains its original transport path',()=>{
 const f=allocate(false);f.run();assert.equal(f.state.transportKind,'websocket');assert.deepEqual(f.counts(),{legacy:1,nativeCalls:0});
});
test('native reconnect backs off while preserving the same state and performs no prompt writes',()=>{
 const code=background.slice(background.indexOf('function scheduleReconnect('),background.indexOf('const connectionWatchdog ='));
 const state={port:{},closed:false,ws:{},transportKind:'native_messaging'},delays=[];
 const ctx=vm.createContext({connections:new Map([[state.port,state]]),Math,clearTimeout:()=>{},setTimeout:(_fn,delay)=>{delays.push(delay);return 1;},openConnection:()=>assert.fail('timer must not execute during scheduling')});
 vm.runInContext(code+';globalThis.schedule=scheduleReconnect;',ctx);for(let i=0;i<8;i++)ctx.schedule(state);assert.deepEqual(delays,[1500,3000,6000,12000,24000,30000,30000,30000]);assert.equal(state.reconnectAttempts,8);
});
test('the deployed native adapter exactly matches the independently tested module',()=>{
 const auth=fs.readFileSync(path.join(root,'overrides/extension/background/authPreflight.js'),'utf8').replace(/\r\n/g,'\n');
 const module=fs.readFileSync(path.join(root,'native/extension/background/nativeTransport.js'),'utf8').replace(/\r\n/g,'\n').trimEnd();
 assert.ok(auth.includes('// BEGIN DIRECTOR NATIVE TRANSPORT ADAPTER (generated)\n'+module));
 const m=JSON.parse(fs.readFileSync(path.join(root,'overrides/extension/manifest.json'),'utf8'));assert.ok(m.permissions.includes('nativeMessaging'));
});
