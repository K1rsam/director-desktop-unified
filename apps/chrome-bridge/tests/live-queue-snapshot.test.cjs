'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
function queue(){const context=vm.createContext({AbortController,Promise,Number,Math,Object,Map,Error});vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/tabOperationQueue.js'),'utf8').replace(/export /g,''),context);return vm.runInContext('new TabOperationQueue()',context);}
test('native queue snapshot observes exact own RPC and foreign queued work without clearing or changing scheduling',async()=>{
  const q=queue(),events=[];let finish;
  const original=q.run(7,async()=>{events.push('first');await new Promise(resolve=>{finish=resolve;});},{label:'content:bridge.tab.open',meta:{type:'bridge.tab.open',requestId:'ext-current-request',privatePrompt:'not exportable'}});
  await Promise.resolve();
  const next=q.run(7,()=>events.push('other'),{label:'content:bridge.effect.begin',meta:{requestId:'foreign'}});
  const snapshot=q.snapshot(7);assert.equal(snapshot.version,'tab-operation-snapshot/v1');assert.equal(snapshot.tabId,7);assert.equal(snapshot.running.label,'content:bridge.tab.open');
  assert.equal(snapshot.running.meta.requestId,'ext-current-request');assert.equal(snapshot.queued.length,1);assert.equal(q.pending(7),2);assert.equal(Object.hasOwn(snapshot.running.meta,'privatePrompt'),false);
  snapshot.queued.length=0;snapshot.running.meta.requestId='changed-copy';assert.equal(q.snapshot(7).queued.length,1);assert.equal(q.snapshot(7).running.meta.requestId,'ext-current-request');
  finish();await Promise.all([original,next]);assert.deepEqual(events,['first','other']);assert.equal(q.snapshot(7).running,null);assert.equal(q.snapshot(7).queued.length,0);
});
test('hello transportHealth is explicitly a historical connect snapshot rather than live idle authority',()=>{
  const text=fs.readFileSync(path.join(__dirname,'../overrides/extension/background.js'),'utf8');
  assert.match(text,/health:\s*\{\s*kind:\s*'connect_snapshot',\s*observedAt:\s*Date\.now\(\)/);
  assert.match(text,/readExecutionInventory:[\s\S]*queue:tabOperations\.snapshot\(state\.tabId\)/);
});

test('execution inventory reads persisted command, lease and outbox facts without exporting private command bodies',async()=>{
  const text=fs.readFileSync(path.join(__dirname,'../overrides/extension/background.js'),'utf8');
  const start=text.indexOf('  readExecutionInventory: async '),end=text.indexOf('\n});',start);
  const port={},state={tabId:7,protocolReady:true},q=queue();
  const own={commandId:'own-read',commandType:'browser.tab.identify',scope:'standalone',mode:'result',operation:'control',status:'dispatched',requestId:'',privatePrompt:'never export'};
  const persisted={lease:{requestId:'other-request'},commands:{'own-read':own,done:{...own,commandId:'done',status:'succeeded'},rejected:{...own,commandId:'rejected',status:'rejected'},unknown:{...own,commandId:'unknown',status:'uncertain'}},
    outbox:[{messageId:'own-accepted',messageType:'command.accepted',commandId:'own-read',body:{privatePrompt:'never export'}},
      {messageId:'foreign-result',messageType:'command.result',body:{commandId:'unknown',privatePrompt:'never export'}},
      {messageId:'own-result',messageType:'command.result',commandId:'own-read'}]};
  const context=vm.createContext({connections:new Map([[port,state]]),backgroundState:{read:async id=>{assert.equal(id,7);return persisted;}},tabOperations:q});
  const reader=vm.runInContext(`({${text.slice(start,end)}}).readExecutionInventory`,context);
  const fact=await reader(port,{inventoryCommandId:'own-read'});
  assert.equal(fact.ownCommand.commandId,'own-read');assert.equal(fact.otherUnresolvedCommands.length,1);assert.equal(fact.otherUnresolvedCommands[0].commandId,'unknown');
  assert.deepEqual(Array.from(fact.ownAcceptedMessageIds),['own-accepted']);assert.equal(fact.otherPendingOutbox.length,2);
  assert.equal(fact.otherPendingOutbox[0].commandId,'unknown');assert.equal(fact.lease.requestId,'other-request');assert.equal(fact.queue.running,null);
  assert.doesNotMatch(JSON.stringify(fact),/privatePrompt|never export/);
  fact.lease.requestId='changed-copy';assert.equal(persisted.lease.requestId,'other-request');
  state.protocolReady=false;await assert.rejects(reader(port,{inventoryCommandId:'own-read'}),/not ready/);
});
