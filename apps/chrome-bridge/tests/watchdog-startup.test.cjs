const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
function fixture(){
 const source=fs.readFileSync(path.join(__dirname,'../overrides/extension/background.js'),'utf8');
 const start=source.indexOf('let backgroundRecovery=null;'),end=source.indexOf('chrome.runtime.onMessage.addListener',start);
 assert.ok(start>0&&end>start);let settle;const events=[],listeners={};const pending=new Promise(r=>settle=r);
 const context=vm.createContext({connectionWatchdog:{arm:async()=>events.push('arm'),run:async()=>events.push('run'),handlesAlarm:n=>n==='watchdog'},
  recoverPendingExtensionReload:async()=>{events.push('maintenance');return pending;},maintenanceOperations:{recover:async()=>events.push('recover_missing')},
  reportMaintenanceRecoveryFailure:error=>events.push(error.message),isExtensionReloadAlarm:()=>false,console,
  chrome:{runtime:{onInstalled:{addListener:f=>listeners.install=f},onStartup:{addListener:f=>listeners.startup=f}},alarms:{onAlarm:{addListener:f=>listeners.alarm=f}}}});
 vm.runInContext(source.slice(start,end),context);
 return {events,listeners,settle,done:()=>vm.runInContext('recoverBackgroundConnections()',context)};
}
test('ordinary cold worker wake recovers once after maintenance settles without requiring runtime.onStartup',async()=>{
 const f=fixture();await new Promise(setImmediate);assert.deepEqual(f.events,['arm','maintenance']);
 f.listeners.startup();f.listeners.install();f.listeners.alarm({name:'watchdog'});assert.deepEqual(f.events,['arm','maintenance']);
 f.settle({reason:'missing'});await f.done();assert.deepEqual(f.events,['arm','maintenance','recover_missing','run']);
});
test('a failed durable restoration never runs the controller watchdog against partial maintenance state',async()=>{
 const f=fixture();f.settle(Promise.reject(Error('restore unavailable')));await assert.rejects(f.done(),/restore unavailable/);
 assert.equal(f.events.includes('run'),false);
});
