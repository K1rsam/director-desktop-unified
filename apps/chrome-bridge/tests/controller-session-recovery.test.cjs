'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),crypto=require('node:crypto');
const PROFILE='5948d446-fd4e-4f69-bb6d-6eb7c204d148',TOKEN='bridge-director-control-'+PROFILE,BASE='http://127.0.0.1:53179',PREFIX='chatgptBridgeLaunchedTab:',STATE='directorDesktop:controlTab:v1:'+PROFILE,SESSION='directorDesktop:controllerBrowserSession:v1',OLD='1136648d-9df6-482d-b2dd-f0faad3df911';
function fixture(options={}){
  const record={launchToken:TOKEN,requestedUrl:'https://chatgpt.com/',serverUrl:BASE,createdAt:1};
  const prior={...record,status:'opened',tabId:122,browserSession:OLD,...options.prior};
  const local={'directorDesktop:browserProfileId:v1':PROFILE,[STATE]:prior,...options.local},session={...options.session},calls=[],tabs=new Map([[1,{id:1,url:'https://chatgpt.com/',active:false,incognito:false}],...(options.tabs||[]).map(t=>[t.id,t])]);
  let reads=0;
  const store=data=>({get:async key=>key===null?{...data}:Object.fromEntries((Array.isArray(key)?key:[key]).map(k=>[k,data[k]])),set:async value=>{calls.push(['persist']);Object.assign(data,value);}});
  const chrome={runtime:{},storage:{local:store(local),session:store(session)},tabs:{
    query:async()=>{reads++;if(options.queryError)throw Error('unavailable');const result=[...tabs.values()].map(t=>({...t}));if(options.inventoryChanges&&reads===2)result.push({id:900,url:'https://example.com/',active:false,incognito:false});return result;},
    create:(opt,done)=>{calls.push(['create',opt]);if(options.unknownCreate)return done({});const t={id:123,url:opt.url,active:false,incognito:false};tabs.set(t.id,t);done(t);},
    update:(id,opt,done)=>{calls.push(['update',id,opt]);Object.assign(tabs.get(id),opt);done({...tabs.get(id)});}}};
  const port={sender:{tab:{id:1,incognito:false}}},connections=new Map([[port,{tabId:1,clientId:'client:1',serverUrl:BASE,closed:false,protocolReady:true,ws:{readyState:1},...options.sourceConnection}]]);
  if(options.targetConnected)connections.set({sender:{tab:{id:122,incognito:false}}},{tabId:122,clientId:'client:122',serverUrl:BASE,closed:false,protocolReady:true,ws:{readyState:1}});
  const context=vm.createContext({URL,URLSearchParams,Date,Math,console,setTimeout,chrome,crypto,TextEncoder});
  vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/tabController.js'),'utf8').replace(/export /g,''),context);
  let identityReads=0;
  const c=context.createTabController({connections,safeBridgeServerUrl:u=>u===BASE?u:'',rememberLaunchedTab:async(id,r)=>{session[PREFIX+id]=r;},readLaunchedTab:async id=>session[PREFIX+id],forgetLaunchedTab:async id=>{delete session[PREFIX+id];},isStableLaunchToken:t=>/^bridge-/.test(t||''),
    readInventoryIdentity:async p=>{identityReads++;const id=connections.get(p).tabId;return {profileId:PROFILE,bundleId:'a'.repeat(32),backgroundEpoch:'bg-current',serverInstanceId:'server-current',contentEpoch:'content:'+id,sourceClientId:'client:'+id,sourceTabId:id,...options.identity,...(options.identityChanges&&identityReads>1?{backgroundEpoch:'bg-changed'}:{})};}});
  return {open:()=>c.openBridgeTab(port,{active:false,launchToken:TOKEN,url:'https://chatgpt.com/'}),calls,local,session,tabs,record,prior};
}
test('manual extension reload recovers only a known closed controller after stable double native inventory',async()=>{
  const f=fixture(),[a,b]=await Promise.all([f.open(),f.open()]);assert.equal(a.tabId,123);assert.equal(b.reused,true);assert.equal(f.calls.filter(x=>x[0]==='create').length,1);
  const state=f.local[STATE],retired=state.retiredControllers[0];assert.equal(state.status,'opened');assert.notEqual(state.browserSession,OLD);assert.equal(f.session[SESSION],state.browserSession);
  assert.equal(retired.tabId,122);assert.equal(retired.browserSession,OLD);assert.equal(retired.status,'opened');assert.equal(retired.reason,'previous_session_closed_inventory_verified');
  assert.equal(retired.recoveryProof.enumerations,2);assert.match(retired.recoveryProof.inventorySha256,/^[a-f0-9]{64}$/);assert.equal(f.calls.find(x=>x[0]==='create')[1].active,false);
});
test('still-live original root with missing session ownership is never adopted, navigated, closed or replaced',async()=>{
  const f=fixture({tabs:[{id:122,url:'https://chatgpt.com/',active:false,incognito:false}],targetConnected:true});await assert.rejects(f.open());assert.ok(f.calls.every(x=>!['create','update'].includes(x[0])));assert.equal(f.local[STATE].browserSession,OLD);
});
test('still-live exact owned connected controller recovers its role without navigation or new page',async()=>{
  const rec={launchToken:TOKEN,requestedUrl:'https://chatgpt.com/',serverUrl:BASE,createdAt:1};
  const f=fixture({tabs:[{id:122,url:'https://chatgpt.com/',active:false,incognito:false}],session:{[PREFIX+122]:rec},targetConnected:true});const r=await f.open();
  assert.equal(r.reused,true);assert.equal(r.tabId,122);assert.equal(f.calls.some(x=>x[0]==='create'),false);assert.ok(f.calls.filter(x=>x[0]==='update').every(x=>!Object.hasOwn(x[2],'url')&&!Object.hasOwn(x[2],'active')));
  assert.equal(f.local[STATE].retiredControllers[0].reason,'previous_session_live_owned_rebound');assert.notEqual(f.local[STATE].browserSession,OLD);
});
test('changed user page, foreground root, orphan/duplicate ownership and unknown prior create stay held',async()=>{
  for(const opt of [{tabs:[{id:122,url:'https://chatgpt.com/c/user',active:false,incognito:false}]},{tabs:[{id:122,url:'https://chatgpt.com/',active:true,incognito:false}]},
    {prior:{status:'reserved'}},{prior:{browserSession:'damaged'}},{session:{[PREFIX+121]:{launchToken:TOKEN,serverUrl:BASE,requestedUrl:'https://chatgpt.com/'}}},
    {session:{[PREFIX+122]:{launchToken:TOKEN,serverUrl:BASE,requestedUrl:'https://chatgpt.com/'},[PREFIX+123]:{launchToken:TOKEN,serverUrl:BASE,requestedUrl:'https://chatgpt.com/'}}}]){
    const f=fixture(opt);await assert.rejects(f.open());assert.ok(f.calls.every(x=>!['create','update'].includes(x[0])));
  }
});
test('copied/foreign profile, wrong server, unready or busy source and runtime drift cannot prove recovery',async()=>{
  for(const opt of [{identity:{profileId:'f913ed04-6d73-4f40-bd74-040418327d79'}},{identity:{sourceTabId:9}},{identity:{sourceClientId:'other'}},{identity:{bundleId:'damaged'}},
    {sourceConnection:{serverUrl:'http://127.0.0.1:9999'}},{sourceConnection:{protocolReady:false}},{sourceConnection:{activeRequest:{requestId:'unknown'}}},{identityChanges:true},{inventoryChanges:true},{queryError:true}]){
    const f=fixture(opt);await assert.rejects(f.open());assert.ok(f.calls.every(x=>!['create','update'].includes(x[0])));assert.equal(f.local[STATE].browserSession,OLD);
  }
});
test('unknown replacement create preserves archived predecessor and one reservation without retry creation',async()=>{
  const f=fixture({unknownCreate:true});await assert.rejects(f.open());assert.equal(f.local[STATE].status,'reserved');assert.equal(f.local[STATE].retiredControllers.length,1);
  await assert.rejects(f.open());assert.equal(f.calls.filter(x=>x[0]==='create').length,1);assert.equal(f.local[STATE].retiredControllers[0].browserSession,OLD);
});
