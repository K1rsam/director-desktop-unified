const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const {webcrypto}=require('node:crypto');
const file=path.join(__dirname,'../overrides/extension/background/connectionWatchdog.js');
const PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7',SESSION='4d285f21-27f0-4a5e-8c29-8a2d13f8a982';
function fixture(change=()=>{}){
 const key='directorDesktop:controlTab:v1:'+PROFILE,jkey='directorDesktop:controlReconnect:v1:'+PROFILE;
 const control={status:'opened',launchToken:'bridge-director-control-'+PROFILE,requestedUrl:'https://chatgpt.com/',serverUrl:'http://127.0.0.1:53179',browserSession:SESSION,tabId:7};
 const local={'directorDesktop:browserProfileId:v1':PROFILE,[key]:control};
 const session={'directorDesktop:controllerBrowserSession:v1':SESSION,'chatgptBridgeLaunchedTab:7':{launchToken:control.launchToken,requestedUrl:control.requestedUrl,serverUrl:control.serverUrl},'chatgptBridgeV6:tab:7':{schemaVersion:6,lease:null,outbox:[],commands:{old:{status:'succeeded'}},effects:{},downloads:{}}};
 const tabs=[{id:7,windowId:9,url:'https://chatgpt.com/',incognito:false,active:true,discarded:false},{id:20,windowId:21,url:'https://example.com/user-page',incognito:false,active:true}],window={id:9,focused:false,state:'minimized'},connections=new Map();
 let reloads=0,writes=0,queries=0;
 const copy=v=>structuredClone(v),get=(data,keys)=>copy(keys===null?data:Object.fromEntries((Array.isArray(keys)?keys:[keys]).filter(k=>Object.hasOwn(data,k)).map(k=>[k,data[k]])));
 const browser={storage:{local:{get:async keys=>get(local,keys),set:async values=>{writes++;Object.assign(local,copy(values));}},session:{get:async keys=>get(session,keys)}},tabs:{query:async()=>{queries++;return copy(tabs);},reload:async id=>{assert.equal(id,7);reloads++;}},windows:{get:async id=>{assert.equal(id,9);return copy(window);}}};
 const facts={local,session,tabs,window,connections,browser,key,jkey};change(facts);
 const context=vm.createContext({chrome:browser,WebSocket:{CONNECTING:0,OPEN:1},crypto:webcrypto,TextEncoder,URL,URLSearchParams,Date,console,clearTimeout,structuredClone});
 const api=vm.runInContext(fs.readFileSync(file,'utf8').replace(/^export /gm,'')+'\n({recoverOwnedController,bootstrapOwnedController,reconcileBootstrapWindow,confirmOwnedControllerReady,createConnectionWatchdog});',context);
 return {...facts,api,run:()=>api.recoverOwnedController({browser,connections,now:()=>1000000}),reloads:()=>reloads,writes:()=>writes,queries:()=>queries};
}
test('zero content connections recovers only proved original minimized controller without focus or generation',async()=>{
 const f=fixture(),before=structuredClone(f.session),out=await f.run();assert.equal(out.status,'reload_requested');assert.equal(out.foreground,false);assert.equal(out.generationCalled,false);
 assert.equal(f.reloads(),1);assert.equal(f.writes(),2);assert.equal(f.queries(),3);assert.deepEqual(f.session,before);assert.equal(f.local[f.jkey].status,'reload_requested');
 await f.run();assert.equal(f.reloads(),1,'cooldown prevents repeated reload');
});
test('controller ownership focus lease commands effects outbox or native read uncertainty never reload user or unknown page',async()=>{
 for(const change of [f=>f.local[f.key].status='reserved',f=>f.local[f.key].tabId=20,f=>f.session['directorDesktop:controllerBrowserSession:v1']='another',
 f=>f.session['chatgptBridgeLaunchedTab:8']=structuredClone(f.session['chatgptBridgeLaunchedTab:7']),f=>delete f.session['chatgptBridgeLaunchedTab:7'],
 f=>f.tabs[0].url='https://chatgpt.com/c/existing',f=>f.tabs[0].incognito=true,f=>f.window.focused=true,f=>f.window.state='normal',
 f=>f.session['chatgptBridgeV6:tab:7'].lease={requestId:'unknown'},f=>f.session['chatgptBridgeV6:tab:7'].commands.old.status='uncertain',
 f=>f.session['chatgptBridgeV6:tab:7'].effects={old:{status:'dispatched'}},f=>f.session['chatgptBridgeV6:tab:7'].outbox=[{}],
 f=>f.session['chatgptBridgeV6:tab:7'].downloads={old:{}},f=>delete f.session['chatgptBridgeV6:tab:7'],f=>f.browser.tabs.query=async()=>{throw Error('native unavailable');}]){
  const f=fixture(change),out=await f.run();assert.equal(out.status,'held');assert.equal(f.reloads(),0);assert.equal(f.writes(),0);
 }
});
test('live controller is never reloaded and unconfirmed reload outcome is never replayed',async()=>{
 const live=fixture(f=>f.connections.set('owned',{tabId:7,closed:false,ws:{readyState:1}}));assert.equal((await live.run()).status,'connected');assert.equal(live.reloads(),0);
 const f=fixture(f=>f.browser.tabs.reload=async()=>{throw Error('lost response');});assert.equal((await f.run()).reason,'reload_outcome_unknown');
 assert.equal(f.local[f.jkey].status,'outcome_unknown');await f.run();assert.equal(f.writes(),2,'no replay after uncertain response');
});
test('native drift or failed intent readback remains held before any reload',async()=>{
 const drift=fixture(f=>{let n=0;f.browser.tabs.query=async()=>{if(++n===2)f.tabs[0].url='https://chatgpt.com/c/user';return structuredClone(f.tabs);};});assert.equal((await drift.run()).status,'held');assert.equal(drift.reloads(),0);
 const persist=fixture(f=>f.browser.storage.local.set=async()=>{});assert.equal((await persist.run()).reason,'intent_not_persisted');assert.equal(persist.reloads(),0);
});
test('native reservation survives worker interruption and cannot become a duplicate reload',async()=>{
 const f=fixture(f=>f.browser.tabs.reload=async()=>{throw Error('uncertain');});await f.run();f.local[f.jkey].status='reserved';assert.equal((await f.run()).reason,'original_reload_unconfirmed');
});
test('background watchdog serializes ticks and runs independent recovery when content connections are absent',async()=>{
 const f=fixture();let calls=0,finish;const pending=new Promise(r=>finish=r);const dog=f.api.createConnectionWatchdog({connections:new Map(),openConnection:()=>assert.fail('no transport'),recoverController:async()=>{calls++;await pending;}});
 const a=dog.run(),b=dog.run();await Promise.resolve();await Promise.resolve();finish();await Promise.all([a,b]);assert.equal(calls,1);
});
function bootstrapFixture(change=()=>{}){
 let creates=0;const f=fixture(f=>{
  delete f.session['chatgptBridgeLaunchedTab:7'];delete f.session['chatgptBridgeV6:tab:7'];delete f.session['directorDesktop:controllerBrowserSession:v1'];
  f.browser.storage.session.set=async values=>Object.assign(f.session,structuredClone(values));
  f.browser.windows.create=async options=>{assert.equal(options.focused,false);assert.equal(options.state,'minimized');assert.equal(options.incognito,false);assert.match(options.url,/chatgpt-bridge-launch/);creates++;return{id:10,focused:false,state:'minimized',incognito:false,tabs:[{id:31}]};};
  f.browser.tabs.update=async(id,options)=>{assert.equal(id,31);assert.deepEqual(JSON.parse(JSON.stringify(options)),{autoDiscardable:false});};
  f.browser.windows.get=async id=>({id,focused:false,state:'minimized',incognito:false});change(f);
 });return {...f,creates:()=>creates};
}
test('lost session allocates exactly one new minimized controller while retaining old page and receipt',async()=>{
 const f=bootstrapFixture(),original=structuredClone(f.local[f.key]),tabs=structuredClone(f.tabs),out=await f.run();
 assert.equal(out.status,'bootstrap_completed');assert.equal(out.generationCalled,false);assert.equal(out.foreground,false);assert.equal(f.creates(),1);assert.equal(f.reloads(),0);assert.deepEqual(f.tabs,tabs);
 assert.equal(f.local[f.key].tabId,31);assert.equal(f.local[f.key].retiredControllers[0].tabId,original.tabId);
 await f.run();assert.equal(f.creates(),1,'new receipt cannot cause another allocation');
});
test('unknown creation invalid old reservation owner drift or unresolved execution never allocates replacement',async()=>{
 for(const change of [f=>f.local[f.key].status='reserved',f=>f.local[f.key].serverUrl='https://remote.invalid',f=>f.local[f.jkey]={status:'reserved'},
 f=>f.session['chatgptBridgeV6:tab:99']={schemaVersion:6,lease:{requestId:'unknown'},outbox:[],commands:{},effects:{},downloads:{}},
 f=>f.connections.set('other',{tabId:9,closed:false,protocolReady:true})]){
  const f=bootstrapFixture(change);assert.equal((await f.run()).status,'held');assert.equal(f.creates(),0);
 }
 const lost=bootstrapFixture(f=>f.browser.windows.create=async()=>{throw Error('lost receipt');});assert.equal((await lost.run()).reason,'bootstrap_create_unknown');
 await lost.run();assert.equal(lost.local[lost.jkey].status,'outcome_unknown');assert.equal(lost.reloads(),0);
});
function chromeSortedStorage(f){
 const sorted=v=>Array.isArray(v)?v.map(sorted):v&&typeof v==='object'?Object.fromEntries(Object.keys(v).sort().map(k=>[k,sorted(v[k])])):v;
 for(const area of ['local','session']){const get=f.browser.storage[area].get;f.browser.storage[area].get=async keys=>sorted(await get(keys));}
}
test('Chrome dictionary key reordering preserves both reload intent and new-window reservation semantics',async()=>{
 const f=fixture(chromeSortedStorage);assert.equal((await f.run()).status,'reload_requested');assert.equal(f.reloads(),1);
 const fresh=bootstrapFixture(chromeSortedStorage);assert.equal((await fresh.run()).status,'bootstrap_completed');assert.equal(fresh.creates(),1);
});
test('pinned v1 sorted-readback rejection repairs zero-create reservation once and archives original',async()=>{
 const f=bootstrapFixture(f=>{
  f.local[f.jkey]={version:'director-owned-controller-bootstrap/v1',status:'reserved',at:900000,originalController:structuredClone(f.local[f.key]),browserSession:SESSION,foreground:false,generationCalled:false};chromeSortedStorage(f);
 });const original=structuredClone(f.local[f.jkey]),out=await f.run();assert.equal(out.status,'bootstrap_completed');assert.equal(f.creates(),1);
 assert.deepEqual(JSON.parse(JSON.stringify(f.local[f.jkey].zeroCreateOrderingRepair.previousIntent)),original);await f.run();assert.equal(f.creates(),1);
});
test('uncertain v2 or changed v1 reservation cannot be interpreted as the known ordering rejection',async()=>{
 for(const change of [r=>r.version='director-owned-controller-bootstrap/v2',r=>r.status='outcome_unknown',r=>r.originalController.tabId=99,r=>r.foreground=true]){
  const f=bootstrapFixture(f=>{const r={version:'director-owned-controller-bootstrap/v1',status:'reserved',at:900000,originalController:structuredClone(f.local[f.key]),browserSession:SESSION,foreground:false,generationCalled:false};change(r);f.local[f.jkey]=r;chromeSortedStorage(f);});
  assert.equal((await f.run()).status,'held');assert.equal(f.creates(),0);
 }
});

test('Windows normal create acknowledgement is explicitly minimized and read back before commit',async()=>{
 let updates=0,minimized=false;
 const f=bootstrapFixture(f=>{
  f.browser.windows.create=async()=>({id:10,focused:false,state:'normal',incognito:false,tabs:[{id:31}]});
  f.browser.windows.update=async(id,options)=>{assert.equal(id,10);assert.deepEqual(JSON.parse(JSON.stringify(options)),{state:'minimized',focused:false});updates++;minimized=true;};
  f.browser.windows.get=async id=>({id,focused:false,state:minimized?'minimized':'normal',incognito:false});
 });assert.equal((await f.run()).status,'bootstrap_completed');assert.equal(updates,1);
});

function interruptedWindowFixture(change=()=>{}){
 let updates=0;const f=fixture(f=>{
  f.local[f.jkey]={version:'director-owned-controller-bootstrap/v2',status:'outcome_unknown',at:900000,originalController:structuredClone(f.local[f.key]),browserSession:SESSION,foreground:false,generationCalled:false};
  delete f.session['chatgptBridgeLaunchedTab:7'];delete f.session['chatgptBridgeV6:tab:7'];
  f.session['chatgptBridgeLaunchedTab:31']={launchToken:f.local[f.key].launchToken,requestedUrl:'https://chatgpt.com/',serverUrl:f.local[f.key].serverUrl,createdAt:900001};
  f.session['chatgptBridgeV6:tab:31']={schemaVersion:6,lease:null,outbox:[],commands:{},effects:{},downloads:{}};
  f.tabs.push({id:31,windowId:10,url:'https://chatgpt.com/',incognito:false,active:true});
  f.connections.set('new',{tabId:31,protocolReady:true,closed:false,activeRequest:null,ws:{readyState:1}});
  f.window={id:10,focused:false,state:'normal',incognito:false};
  f.browser.windows.get=async id=>{assert.equal(id,10);return structuredClone(f.window);};
  f.browser.windows.update=async(id,options)=>{assert.equal(id,10);assert.deepEqual(JSON.parse(JSON.stringify(options)),{state:'minimized',focused:false});updates++;f.window.state='minimized';};
  f.browser.tabs.update=async id=>assert.equal(id,31);
  f.browser.windows.create=async()=>assert.fail('never recreate original window');
  change(f);
 });return {...f,updates:()=>updates};
}
test('connected original unknown creation reconciles exclusively owned window without resubmission or new window',async()=>{
 const f=interruptedWindowFixture(),old=structuredClone(f.local[f.jkey]);
 assert.equal((await f.run()).status,'bootstrap_reconciled');assert.equal(f.updates(),1);assert.equal(f.local[f.key].tabId,31);
 assert.deepEqual(JSON.parse(JSON.stringify(f.local[f.jkey].windowReconciliation.originalIntent)),old);
 assert.equal((await f.run()).status,'connected');assert.equal(f.updates(),1);
});
test('unknown creation cannot adopt user window, stale launch, active execution or duplicate ownership',async()=>{
 for(const change of [f=>f.window.focused=true,f=>f.tabs.push({id:99,windowId:10,url:'https://example.com/user',incognito:false}),
  f=>f.session['chatgptBridgeLaunchedTab:31'].createdAt=899999,
  f=>f.session['chatgptBridgeV6:tab:31'].lease={requestId:'remote'},
  f=>f.session['chatgptBridgeLaunchedTab:32']=structuredClone(f.session['chatgptBridgeLaunchedTab:31']),
  f=>f.local[f.key].tabId=99]){
  const f=interruptedWindowFixture(change),old=structuredClone(f.local);
  assert.equal((await f.api.reconcileBootstrapWindow({browser:f.browser,connections:f.connections,now:()=>1000000})).status,'held');
  assert.equal(f.updates(),0);assert.deepEqual(f.local,old);
 }
});
test('watchdog checks uncertain native creation even while transport is connected',async()=>{
 const f=interruptedWindowFixture();let repairs=0;
 const dog=f.api.createConnectionWatchdog({connections:f.connections,openConnection:()=>assert.fail('already connected'),recoverController:async()=>{repairs++;await f.run();}});
 await dog.run();assert.equal(repairs,1);assert.equal(f.updates(),1);
});
function readyProof(tabId,contentEpoch='fresh'){
 return {accepted:true,helloMessageId:'hello-1',ackMessageId:'hello-1',acceptedSequence:1,launchRecordMatches:true,
  profileId:PROFILE,browserIncognito:false,backgroundEpoch:'current-worker',contentEpoch,serverInstanceId:'server',transportServerInstanceId:'server',bundleId:'a'.repeat(32),
  tabId,launchToken:'bridge-director-control-'+PROFILE,serverUrl:'http://127.0.0.1:53179'};
}
test('independent acknowledged disconnect incidents do not exhaust a lifetime reconnect budget',async()=>{
 const f=fixture();f.session['chatgptBridgeV6:tab:7'].contentEpoch='old';let clock=1000000;
 for(let round=0;round<4;round++){
  assert.equal((await f.api.recoverOwnedController({browser:f.browser,connections:f.connections,now:()=>clock})).status,'reload_requested');
  assert.equal(f.local[f.jkey].attempts,1);
  assert.equal(await f.api.confirmOwnedControllerReady({browser:f.browser,proof:readyProof(7,'fresh-'+round),now:()=>clock}),true);
  assert.equal(f.local[f.jkey].status,'connection_confirmed');assert.equal(f.local[f.jkey].attempts,0);clock+=120001;
 }assert.equal(f.reloads(),4);assert.equal(f.local[f.jkey].incidentHistory.length,7);
});
test('socket readiness wrong ACK epoch identity or unknown operation cannot reset reconnect budget',async()=>{
 for(const change of [p=>p.accepted=false,p=>p.ackMessageId='wrong',p=>p.acceptedSequence=0,p=>p.contentEpoch='old',p=>p.serverInstanceId='other',p=>p.profileId=SESSION,p=>p.tabId=99]){
  const f=fixture();f.session['chatgptBridgeV6:tab:7'].contentEpoch='old';await f.run();const journal=structuredClone(f.local[f.jkey]),p=readyProof(7);change(p);
  assert.equal(await f.api.confirmOwnedControllerReady({browser:f.browser,proof:p}),false);assert.deepEqual(f.local[f.jkey],journal);
 }
 const f=fixture();await f.run();f.local[f.jkey].status='outcome_unknown';assert.equal(await f.api.confirmOwnedControllerReady({browser:f.browser,proof:readyProof(7)}),false);
});
test('confirmed creation supports a later session-loss incident and preserves prior confirmations',async()=>{
 const f=bootstrapFixture();await f.run();
 f.tabs.push({id:31,windowId:10,url:'https://chatgpt.com/',active:true,incognito:false});
 assert.equal(await f.api.confirmOwnedControllerReady({browser:f.browser,proof:readyProof(31),now:()=>1000000}),true);
 delete f.session['directorDesktop:controllerBrowserSession:v1'];delete f.session['chatgptBridgeLaunchedTab:31'];
 f.browser.windows.create=async()=>({id:11,focused:false,state:'minimized',incognito:false,tabs:[{id:32}]});f.browser.tabs.update=async()=>{};
 assert.equal((await f.run()).status,'bootstrap_completed');assert.equal(f.local[f.key].tabId,32);assert.equal(f.local[f.jkey].incidentHistory.at(-1).status,'connection_confirmed');
});
test('reconciliation after owned maintenance uses a fresh session nonce instead of resurrecting journal nonce',async()=>{
 const f=interruptedWindowFixture(f=>{delete f.session['directorDesktop:controllerBrowserSession:v1'];f.browser.storage.session.set=async v=>Object.assign(f.session,structuredClone(v));});
 assert.equal((await f.run()).status,'bootstrap_reconciled');assert.notEqual(f.local[f.key].browserSession,SESSION);
 assert.equal(f.local[f.key].browserSession,f.session['directorDesktop:controllerBrowserSession:v1']);
});
