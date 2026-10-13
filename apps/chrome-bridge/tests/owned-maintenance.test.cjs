const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const token=id=>`bridge-director-desktop-00000000-0000-4000-8000-${String(id).padStart(12,'0')}`;
function fixture(options={}){
  const writes=[],plans=[],queries=[],remembered=[],navigated=[],reloaded=[],tabs=[{id:1,url:'https://chatgpt.com/c/own-one'},{id:2,url:'https://chatgpt.com/c/own-two'},{id:3,url:'https://chatgpt.com/c/user'},...(options.tabs||[])],records={1:{launchToken:token(1)},2:{launchToken:token(2)},...options.records};
  const local={...options.local},session={
    'chatgptBridgeLaunchedTab:1':records[1],
    'chatgptBridgeLaunchedTab:2':records[2],
    ...options.session,
  };
  const context=vm.createContext({URL,URLSearchParams,Date,Math,console:{info(){},warn(){},error(){}},setTimeout(){return 0;},
    chrome:{tabs:{query:async q=>{queries.push(q);return tabs;},get:async id=>tabs.find(t=>t.id===id),reload:async()=>{}},storage:{
      local:{set:async value=>{writes.push(value);Object.assign(local,value);},get:async key=>({[key]:local[key]}),remove:async key=>{delete local[key];}},
      session:{get:async()=>{if(options.sessionReadError)throw Error('session unavailable');return {...session};}},
    }}});
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/extensionReloadCoordinator.js'),'utf8').replace(/export /g,''),context);
  const coordinator=context.createExtensionReloadCoordinator({backgroundState:{read:async id=>({lease:options.busyId===id?{requestId:'original'}:null,commands:options.commands?.[id]||{},outbox:[],...options.runtimes?.[id]})},
    maintenanceOperations:{plan:async(type,p)=>{plans.push(p);return {accepted:true,state:{active:{operationId:'operation'}}};},dispatch:async()=>({accepted:true}),succeed:async()=>{},fail:async()=>{}},
    safeBridgeServerUrl:u=>u,readLaunchedTab:async id=>records[id],launchTokenPattern:/^bridge-/,
    rememberLaunchedTab:async(id,record)=>{remembered.push({id,record});records[id]=record;session[`chatgptBridgeLaunchedTab:${id}`]=record;},
    navigateTab:async(id,url)=>navigated.push({id,url}),reloadTab:async id=>reloaded.push(id),reloadRuntime(){}});
  return {coordinator,writes,plans,queries,local,records,session,remembered,navigated,reloaded};
}
test('owned-only extension maintenance refreshes its original pages while leaving user pages untouched',async()=>{
  const f=fixture();const r=await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:true,ownedTabsOnly:true,temporaryServerUrl:'http://127.0.0.1:53179'});
  assert.equal(r.tabCount,2);assert.deepEqual(Array.from(f.writes[0].bridgePendingExtensionReload.tabIds),[1,2]);
  assert.ok(!Object.hasOwn(f.writes[0].bridgePendingExtensionReload.targetUrls,'3'));
});
test('owned-only maintenance rejects a user source and any active original lease before write',async()=>{
  for(const options of [{source:3},{busyId:3,source:1},{busyId:2,source:1}]){
    const f=fixture(options);await assert.rejects(f.coordinator.scheduleExtensionReload({sourceTabId:options.source,commandId:'command',reloadTabs:true,ownedTabsOnly:true}));
    assert.equal(f.writes.length,0);assert.equal(f.plans.length,0);
  }
});
test('initial source-only reload never enumerates or refreshes other ChatGPT pages',async()=>{
  const f=fixture();const r=await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false});
  assert.equal(r.tabCount,1);assert.equal(f.queries.length,0);assert.deepEqual(Array.from(f.writes[0].bridgePendingExtensionReload.tabIds),[1]);
  assert.equal(r.preservedLaunchCount,2);
  assert.deepEqual(Object.keys(f.writes[0].bridgePendingExtensionReload.launchRecords),['1','2']);
});

test('source-only recovery restores every owned identity but refreshes only its source',async()=>{
  const f=fixture();await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false,temporaryServerUrl:'http://127.0.0.1:53179'});
  for(const key of Object.keys(f.records))delete f.records[key];
  for(const key of Object.keys(f.session))delete f.session[key];
  const result=await f.coordinator.recoverPendingExtensionReload();
  assert.equal(result.recovered,false);assert.equal(result.pending,true);assert.equal(result.tabCount,1);
  assert.deepEqual(f.remembered.map(r=>r.id),[1,2]);
  assert.equal(f.records[2].launchToken,token(2));
  assert.equal(f.session['chatgptBridgeLaunchedTab:2'].launchToken,token(2));
  assert.deepEqual(f.navigated.map(t=>t.id),[1]);assert.deepEqual(f.reloaded,[1]);
  assert.equal(f.queries.length,0);assert.equal(f.local.bridgePendingExtensionReload.recoveryStatus,'awaiting_source_ready');
});

test('session snapshot and recovery reject user tokens, fake prefixes and invalid tab keys',async()=>{
  const f=fixture({session:{
    'chatgptBridgeLaunchedTab:3':{launchToken:'user-page'},
    'chatgptBridgeLaunchedTab:4':{launchToken:'bridge-director-desktop-fake'},
    'chatgptBridgeLaunchedTab:5':{launchToken:'bridge-reload-temporary'},
    'chatgptBridgeLaunchedTab:6':{launchToken:'bridge-director-desktop-00000000-0000-0000-0000-000000000006'},
    'chatgptBridgeLaunchedTab:01':{launchToken:token(1)},
    'chatgptBridgeLaunchedTab:-6':{launchToken:token(6)},
    'chatgptBridgeLaunchedTab:7.5':{launchToken:token(7)},
    'chatgptBridgeLaunchedTab:9007199254740992':{launchToken:token(8)},
    'other:9':{launchToken:token(9)},
  }});
  await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false});
  const pending=f.local.bridgePendingExtensionReload;
  assert.deepEqual(Object.keys(pending.launchRecords),['1','2']);
  Object.assign(pending.launchRecords,{'3':{launchToken:'user-page'},'4':{launchToken:'bridge-director-desktop-fake'},'01':{launchToken:token(1)},'-6':{launchToken:token(6)}});
  await f.coordinator.recoverPendingExtensionReload();
  assert.deepEqual(f.remembered.map(r=>r.id),[1,2]);
  assert.deepEqual(f.reloaded,[1]);assert.deepEqual(f.navigated,[]);
});

test('failed session snapshot blocks runtime maintenance before any plan or durable write',async()=>{
  const f=fixture({sessionReadError:true});
  await assert.rejects(f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false}),error=>error.code==='MAINTENANCE_SESSION_READ_FAILED');
  assert.equal(f.plans.length,0);assert.equal(f.writes.length,0);
  assert.deepEqual(f.navigated,[]);assert.deepEqual(f.reloaded,[]);
});
test('controller ownership survives source-only runtime reload without refreshing its page',async()=>{
  const controller='bridge-director-control-b84d4521-56ba-4f84-9948-090f82d639e7';
  const f=fixture({session:{'chatgptBridgeLaunchedTab:4':{launchToken:controller}}});
  const scheduled=await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false});
  assert.equal(scheduled.preservedLaunchCount,3);assert.deepEqual(Array.from(f.local.bridgePendingExtensionReload.tabIds),[1]);
  await f.coordinator.recoverPendingExtensionReload();assert.deepEqual(f.remembered.map(r=>r.id),[1,2,4]);assert.deepEqual(f.reloaded,[1]);
});
test('owned-only maintenance accepts the idle original controller and still excludes user pages',async()=>{
  const controller='bridge-director-control-b84d4521-56ba-4f84-9948-090f82d639e7';
  const f=fixture({records:{4:{launchToken:controller}},session:{'chatgptBridgeLaunchedTab:4':{launchToken:controller}},tabs:[{id:4,url:'https://chatgpt.com/'}]});
  await f.coordinator.scheduleExtensionReload({sourceTabId:4,commandId:'command',reloadTabs:true,ownedTabsOnly:true});
  assert.deepEqual(Array.from(f.local.bridgePendingExtensionReload.tabIds),[1,2,4]);
  assert.ok(!Object.hasOwn(f.local.bridgePendingExtensionReload.targetUrls,'3'));
});

const CONTROL='bridge-director-control-b84d4521-56ba-4f84-9948-090f82d639e7',CONTROL_STATE='directorDesktop:controlTab:v1:b84d4521-56ba-4f84-9948-090f82d639e7',OLD_SESSION='1136648d-9df6-482d-b2dd-f0faad3df911';
function controllerFixture(options={}){
  const record={launchToken:CONTROL,requestedUrl:'https://chatgpt.com/',serverUrl:'http://127.0.0.1:53179'};
  return fixture({...options,records:{4:record,...options.records},session:{'chatgptBridgeLaunchedTab:4':record,...options.session},
    local:{[CONTROL_STATE]:{...record,tabId:4,browserSession:OLD_SESSION,status:'opened'},...options.local},tabs:[{id:4,url:'https://chatgpt.com/',active:true},...(options.tabs||[])]});
}
test('source-only runtime recovery proves idle controller invalidation without copying its old session or touching its page',async()=>{
  const f=controllerFixture();await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false});
  const proof=f.local.bridgePendingExtensionReload.idleControllers[CONTROL_STATE];assert.equal(proof.tabId,4);assert.equal(proof.browserSession,OLD_SESSION);
  for(const key of Object.keys(f.session))delete f.session[key];
  await f.coordinator.recoverPendingExtensionReload();assert.equal(f.local[CONTROL_STATE].reloadInvalidated.operationId,'operation');
  assert.equal(f.session['directorDesktop:controllerBrowserSession:v1'],undefined);assert.deepEqual(f.reloaded,[1]);assert.deepEqual(f.navigated,[]);
  assert.equal(f.session['chatgptBridgeLaunchedTab:4'].launchToken,CONTROL);
});
test('idle controller snapshot blocks active leases and unconfirmed commands even on source-only maintenance',async()=>{
  for(const options of [{busyId:4},{commands:{4:{open:{status:'uncertain'}}}},{commands:{4:{open:{status:'dispatched'}}}}]){
    const f=controllerFixture(options);await assert.rejects(f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false}),e=>e.code==='MAINTENANCE_BLOCKED_BY_ACTIVE_LEASE');
    assert.equal(f.plans.length,0);assert.equal(f.writes.length,0);
  }
});
test('reserved, missing, changed or conflicting controller identity never receives runtime replacement proof',async()=>{
  for(const field of ['status','tabId','launchToken','browserSession','serverUrl','requestedUrl']){
    const f=controllerFixture();f.local[CONTROL_STATE][field]=field==='tabId'?99:'';
    await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false});
    assert.equal(Object.keys(f.local.bridgePendingExtensionReload.idleControllers).length,0);
    await f.coordinator.recoverPendingExtensionReload();assert.equal(f.local[CONTROL_STATE].reloadInvalidated,undefined);
  }
  const f=controllerFixture();await f.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false});
  f.local[CONTROL_STATE].serverUrl='http://127.0.0.1:9999';await f.coordinator.recoverPendingExtensionReload();assert.equal(f.local[CONTROL_STATE].reloadInvalidated,undefined);
});

const ownReloadCommand={commandId:'command',commandType:'extension.reload',scope:'standalone',requestId:'',mode:'result',operation:'maintenance',status:'dispatched'};
test('current exact standalone extension reload can snapshot its own idle controller without self-deadlock',async()=>{
  const command={...ownReloadCommand},f=controllerFixture({commands:{4:{command}}}),before=JSON.stringify(command);
  const result=await f.coordinator.scheduleExtensionReload({sourceTabId:4,commandId:'command',reloadTabs:false,temporaryServerUrl:'http://127.0.0.1:53179'});
  assert.equal(result.scheduled,true);assert.equal(f.plans.length,1);assert.equal(f.local.bridgePendingExtensionReload.idleControllers[CONTROL_STATE].tabId,4);
  assert.equal(JSON.stringify(command),before);assert.equal(f.local[CONTROL_STATE].browserSession,OLD_SESSION);
});
test('self-maintenance exception never ignores other commands, unknown outcomes, leases or mismatched command contracts',async()=>{
  const failures=[];
  for(const [field,value] of [['commandId','other'],['commandType','prompt.send'],['scope','request'],['requestId','unknown-model'],['mode','effect'],['operation','control'],['status','uncertain'],['status','registered']])failures.push({commands:{4:{command:{...ownReloadCommand,[field]:value}}}});
  failures.push({commands:{4:{command:{...ownReloadCommand},other:{commandId:'other',commandType:'extension.reload',scope:'standalone',requestId:'',mode:'result',operation:'maintenance',status:'dispatched'}}}});
  failures.push({commands:{4:{other:{...ownReloadCommand}}}});
  failures.push({busyId:4,commands:{4:{command:{...ownReloadCommand}}}});
  for(const options of failures){const f=controllerFixture(options);await assert.rejects(f.coordinator.scheduleExtensionReload({sourceTabId:4,commandId:'command',reloadTabs:false}),e=>e.code==='MAINTENANCE_BLOCKED_BY_ACTIVE_LEASE');assert.equal(f.plans.length,0);assert.equal(f.writes.length,0);}
  const other=controllerFixture({commands:{4:{command:{...ownReloadCommand}}}});await assert.rejects(other.coordinator.scheduleExtensionReload({sourceTabId:1,commandId:'command',reloadTabs:false}),e=>e.code==='MAINTENANCE_BLOCKED_BY_ACTIVE_LEASE');assert.equal(other.plans.length,0);
});

test('idle maintenance proof cannot discard pending effect, download or foreign outbox on runtime reset',async()=>{
 for(const runtime of [{effects:{upload:{status:'uncertain'}}},{downloads:{capture:{}}},{outbox:[{commandId:'foreign',messageType:'command.accepted'}]},
  {outbox:[{commandId:'command',messageType:'effect.completed'}]}]){
  const f=controllerFixture({commands:{4:{command:{...ownReloadCommand}}},runtimes:{4:runtime}});
  await assert.rejects(f.coordinator.scheduleExtensionReload({sourceTabId:4,commandId:'command',reloadTabs:false}),e=>e.code==='MAINTENANCE_BLOCKED_BY_ACTIVE_LEASE');
  assert.equal(f.plans.length,0);assert.equal(f.writes.length,0);
 }
 const f=controllerFixture({commands:{4:{command:{...ownReloadCommand}}},runtimes:{4:{outbox:[{commandId:'command',messageType:'command.accepted'}],effects:{old:{status:'succeeded'}},downloads:{}}}});
 assert.equal((await f.coordinator.scheduleExtensionReload({sourceTabId:4,commandId:'command',reloadTabs:false})).scheduled,true);
});
