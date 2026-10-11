const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),crypto=require('node:crypto');
const PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7',TOKEN='bridge-director-control-'+PROFILE,BASE='http://127.0.0.1:53179',PREFIX='chatgptBridgeLaunchedTab:',STATE='directorDesktop:controlTab:v1:'+PROFILE;
function fixture(options={}) {
  const local={'directorDesktop:browserProfileId:v1':PROFILE,...options.local},session={...options.session},tabs=new Map((options.tabs||[]).map(t=>[t.id,{...t}])),calls=[];
  const store=data=>({get:async key=>{if(options.storageError)throw Error('storage unavailable');if(key===null)return {...data};return Object.fromEntries((Array.isArray(key)?key:[key]).map(k=>[k,data[k]]));},set:async value=>{if(options.storageError)throw Error('storage unavailable');Object.assign(data,value);}});
  function controller(){
    const chrome={runtime:{},storage:{local:store(local),session:store(session)},tabs:{
      query:async()=>{calls.push(['query']);return [...tabs.values()].map(t=>({...t}));},
      create:(opt,done)=>{calls.push(['create',opt]);if(options.unknownCreate){done({});return;}const tab={id:123,url:opt.url,active:opt.active,autoDiscardable:true,discarded:false};tabs.set(123,tab);done({...tab});},
      update:(id,opt,done)=>{calls.push(['update',id,opt]);if(options.updateError){chrome.runtime.lastError={message:'navigation unknown'};done();delete chrome.runtime.lastError;return;}Object.assign(tabs.get(id),opt);let tab={...tabs.get(id)};if(options.omitState)for(const k of ['active','autoDiscardable','discarded'])delete tab[k];done(tab);},
    }};
    const context=vm.createContext({URL,URLSearchParams,Date,Math,console,setTimeout,chrome,crypto});
    vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/tabController.js'),'utf8').replace(/export /g,''),context);
    const port={sender:{tab:{id:1,incognito:false}}};
    const instance=context.createTabController({connections:new Map([[port,{serverUrl:BASE}]]),safeBridgeServerUrl:u=>u===BASE?u:'',
      rememberLaunchedTab:async(id,record)=>{session[PREFIX+id]={...record};},readLaunchedTab:async id=>session[PREFIX+id],forgetLaunchedTab:async id=>{delete session[PREFIX+id];},isStableLaunchToken:()=>true});
    return {open:opt=>instance.openBridgeTab(port,{active:false,launchToken:TOKEN,...opt}),instance,port};
  }
  return {local,session,tabs,calls,controller};
}
test('controller is created once in background, returns actual tab state and reuses across worker restart',async()=>{
  const f=fixture(),first=f.controller();const [a,b]=await Promise.all([first.open(),first.open()]);
  assert.equal(a.tabId,123);assert.equal(b.tabId,123);assert.equal(b.reused,true);
  assert.equal(f.calls.filter(c=>c[0]==='create').length,1);assert.equal(a.tabState.active,false);assert.equal(a.tabState.autoDiscardable,false);assert.equal(a.tabState.discarded,false);
  assert.equal(f.local[STATE].status,'opened');assert.equal(f.session[PREFIX+123].launchToken,TOKEN);
  const restarted=await f.controller().open();assert.equal(restarted.reused,true);assert.equal(f.calls.filter(c=>c[0]==='create').length,1);
  const reuseUpdates=f.calls.filter(c=>c[0]==='update').slice(1);assert.ok(reuseUpdates.every(c=>!Object.hasOwn(c[2],'url')&&!Object.hasOwn(c[2],'active')));
});
test('uncertain create or navigation keeps reservation and never creates again after worker restart',async()=>{
  for(const options of [{unknownCreate:true},{updateError:true}]){
    const f=fixture(options);await assert.rejects(f.controller().open());assert.equal(f.local[STATE].status,'reserved');
    await assert.rejects(f.controller().open());assert.equal(f.calls.filter(c=>c[0]==='create').length,1);
  }
});
test('missing storage, copied profile, conflicting server and changed user conversation never navigate or replace',async()=>{
  const record={launchToken:TOKEN,requestedUrl:'https://chatgpt.com/',serverUrl:BASE};
  const cases=[{storageError:true},{local:{'directorDesktop:browserProfileId:v1':'f913ed04-6d73-4f40-bd74-040418327d79'}},
    {session:{[PREFIX+123]:{...record,serverUrl:'http://127.0.0.1:9999'}},tabs:[{id:123,url:'https://chatgpt.com/'}]},
    {session:{[PREFIX+123]:record},tabs:[{id:123,url:'https://chatgpt.com/c/user'}]},
    {local:{[STATE]:{...record,status:'opened',tabId:123}},tabs:[{id:123,url:'https://chatgpt.com/'}]},
    {session:{[PREFIX+123]:record,[PREFIX+124]:record},tabs:[{id:123,url:'https://chatgpt.com/'},{id:124,url:'https://chatgpt.com/'}]}];
  for(const options of cases){const f=fixture(options);await assert.rejects(f.controller().open());assert.ok(f.calls.every(c=>!['create','update'].includes(c[0])));}
});
test('controller refuses foreground or non-root requests and never invents missing actual tab state',async()=>{
  for(const opt of [{active:true},{url:'https://chatgpt.com/c/user'},{url:'https://chat.openai.com/'},{bridgeServerUrl:'http://127.0.0.1:9999'}]){
    const f=fixture();await assert.rejects(f.controller().open(opt));assert.ok(f.calls.every(c=>c[0]!=='create'));
  }
  const f=fixture({omitState:true});const result=await f.controller().open();
  assert.deepEqual(Object.keys(result.tabState),[]);assert.equal(Object.hasOwn(result,'active'),false);
});
test('known controller disappearance is proved by Chrome inventory before one replacement',async()=>{
  const record={launchToken:TOKEN,requestedUrl:'https://chatgpt.com/',serverUrl:BASE};
  const f=fixture({session:{[PREFIX+122]:record},local:{[STATE]:{...record,status:'opened',tabId:122}}});
  await f.controller().open();assert.equal(f.calls.filter(c=>c[0]==='create').length,1);assert.equal(Object.hasOwn(f.session,PREFIX+122),false);
});
test('onRemoved identity cleanup permits one recreation only in the same proved browser session',async()=>{
  const f=fixture();await f.controller().open();const browserSession=f.local[STATE].browserSession;
  f.tabs.delete(123);delete f.session[PREFIX+123];
  await f.controller().open();assert.equal(f.calls.filter(c=>c[0]==='create').length,2);
  assert.equal(f.local[STATE].browserSession,browserSession);assert.equal(f.local[STATE].status,'opened');
  await f.controller().open();assert.equal(f.calls.filter(c=>c[0]==='create').length,2);
});
test('closed controller still holds on browser restart, reserved unknown or live tab with missing ownership',async()=>{
  for(const change of ['browser-restart','reserved','live-tab']){
    const f=fixture();await f.controller().open();delete f.session[PREFIX+123];
    if(change!=='live-tab')f.tabs.delete(123);
    if(change==='browser-restart')delete f.session['directorDesktop:controllerBrowserSession:v1'];
    if(change==='reserved')f.local[STATE].status='reserved';
    await assert.rejects(f.controller().open());assert.equal(f.calls.filter(c=>c[0]==='create').length,1,change);
  }
});
