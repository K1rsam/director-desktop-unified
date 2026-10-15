const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const source=fs.readFileSync(path.join(__dirname,'../overrides/src/browserExtensionHub.js'),'utf8');
const ctx=vm.createContext({Date});vm.runInContext(source.slice(source.indexOf('export function acceptTransportPong'),source.indexOf('export class BrowserExtensionHub')).replace(/export /g,''),ctx);
function client(){return {lastSeenAt:1000,ready:false,pageReady:false,activeRequest:{requestId:'unknown-original'},pendingTransportPing:{nonce:'exact-native-ping',at:100000}};}
test('native socket pong keeps a hidden transport alive without refreshing DOM or settling an unknown request',()=>{
 const c=client(),before=JSON.stringify(c.activeRequest);assert.equal(ctx.transportClientAlive(c,110000,90000),false);
 assert.equal(ctx.acceptTransportPong(c,Buffer.from('exact-native-ping'),110000),true);
 assert.equal(ctx.transportClientAlive(c,180000,90000),true);assert.equal(c.lastSeenAt,1000);assert.equal(c.pageReady,false);assert.equal(c.ready,false);assert.equal(JSON.stringify(c.activeRequest),before);
 assert.equal(ctx.transportClientAlive(c,210001,90000),false);
});
test('unsolicited, replayed, stale or future socket pongs cannot extend transport liveness',()=>{
 for(const [data,at]of [['other-ping',110000],['exact-native-ping',140000],['exact-native-ping',99000]]){
  const c=client();assert.equal(ctx.acceptTransportPong(c,data,at),false);assert.equal(c.lastTransportPongAt,undefined);
 }
 const c=client();assert.equal(ctx.acceptTransportPong(c,'exact-native-ping',110000),true);assert.equal(ctx.acceptTransportPong(c,'exact-native-ping',110001),false);
});
