const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),{EventEmitter}=require('node:events');
function fixture(options={}){
  const source=fs.readFileSync(path.join(__dirname,'../overrides/src/bridge/coordinator/browserTabCoordinator.js'),'utf8').replace(/^import[\s\S]*?from [^;]+;\r?\n/gm,'').replace(/export (?=function|class)/g,'');
  const context=vm.createContext({URL,Date,makeRequestId:()=>assert.fail('must not allocate replacement identity'),
    BROWSER_LAUNCH_TOKEN_RE:/^bridge-/,browserLaunchMetadataFromUrl:()=>({}),safeChatGptUrl:x=>x,normalizeLaunchedClient:x=>x,
    setTimeout:(callback,ms)=>setTimeout(callback,ms>=20000?100:2),clearTimeout});
  vm.runInContext(source+';this.Coordinator=BrowserTabCoordinator',context);
  const hub=new EventEmitter(),before={id:'owned-source',browserTabId:7,launchToken:'bridge-director-desktop-12345678-1234-4234-8234-123456789abc',url:'https://chatgpt.com/',ready:true,connectedAt:'2000-01-01T00:00:00Z',
    browserProfileId:'11111111-1111-4111-8111-111111111111',extensionVersion:'2.4.5',extensionBundleId:'a'.repeat(32),serverInstanceId:'original-server',backgroundEpoch:'old-worker',contentEpoch:'old-content',...options.before};
  hub.clients=[before];hub.activeClient=before;const commands=[];
  const controller=new context.Coordinator({hub,rankClients:x=>x,runtimeOptions:{publicBaseUrl:'http://127.0.0.1:53179',openExternalUrl:()=>assert.fail('must not open foreground bootstrap')},
    sendCommand:async(type,payload)=>{commands.push({type,payload});if(options.commandError)throw Object.assign(new Error('original guard rejected'),{code:'COMMAND_REJECTED'});
      if(options.reconnect)queueMicrotask(()=>hub.emit('client.ready',{...before,backgroundEpoch:'new-worker',contentEpoch:'new-content',...options.reconnect}));
      return {scheduled:true,pageReload:{armed:true,delayMs:2500}};}});
  controller.openSystemBrowserTab=()=>assert.fail('must not open OS fallback');
  return {controller,commands,hub,before};
}
test('accepted owned-only reload preserves its pending outcome without OS fallback or tab replacement',async()=>{
  for(const allow of [false,true]){
    const f=fixture();await assert.rejects(f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true,allowMaintenancePageBootstrap:allow,timeoutMs:30000}),error=>{
      assert.equal(error.code,'EXTENSION_RELOAD_RECONNECT_PENDING');assert.equal(error.reloadAccepted,true);assert.equal(error.accepted.scheduled,true);return true;
    });assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'extension.reload');
  }
});
test('explicitly disabled bootstrap also prevents replacement fallback after accepted reload',async()=>{
  const f=fixture();await assert.rejects(f.controller.reloadExtension({sourceClientId:'owned-source',allowMaintenancePageBootstrap:false,timeoutMs:30000}),{code:'EXTENSION_RELOAD_RECONNECT_PENDING'});
  assert.equal(f.commands.length,1);
});
test('owned-only command rejection remains the original rejection without any fallback',async()=>{
  const f=fixture({commandError:true});await assert.rejects(f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true,allowMaintenancePageBootstrap:true,timeoutMs:30000}),{code:'COMMAND_REJECTED'});
  assert.equal(f.commands.length,1);
});

test('bootstrap-only native health cannot dispatch another extension reload or allocate reconnect listeners',async()=>{
  const f=fixture({before:{transportHealth:{kind:'connect_snapshot',backgroundOnly:true,transportKind:'native_messaging'}}});
  await assert.rejects(f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true}),{code:'EXTENSION_RELOAD_CONTENT_NOT_READY'});
  assert.deepEqual(f.commands,[]);assert.equal(f.hub.listenerCount('client.ready'),0);
});
test('original owned page reconnect completes the existing reload without another action',async()=>{
  const f=fixture({reconnect:true}),result=await f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true,allowMaintenancePageBootstrap:false,timeoutMs:30000});
  assert.equal(result.reconnected.id,'owned-source');assert.equal(f.commands.length,1);
});

test('content-only refresh and wrong runtime identity cannot complete an accepted reload',async t=>{
  for(const [name,reconnect] of [
    ['same worker',{backgroundEpoch:'old-worker'}],['same document',{contentEpoch:'old-content'}],
    ['missing worker',{backgroundEpoch:''}],['missing document',{contentEpoch:''}],
    ['foreign profile',{browserProfileId:'22222222-2222-4222-8222-222222222222'}],
    ['foreign bundle',{extensionBundleId:'b'.repeat(32)}],['foreign server',{serverInstanceId:'other-server'}],
    ['foreign launch',{launchToken:'bridge-director-desktop-other'}],['same client ID different tab',{browserTabId:8}],
    ['string tab ID',{browserTabId:'7'}],['not ready',{ready:false}],
  ])await t.test(name,async()=>{
    const f=fixture({reconnect});await assert.rejects(f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true,allowMaintenancePageBootstrap:false,timeoutMs:30000}),{code:'EXTENSION_RELOAD_RECONNECT_PENDING'});
    assert.equal(f.commands.length,1);assert.equal(f.hub.listenerCount('client.ready'),0);
  });
});

test('missing original identity fails before command dispatch or reconnect listeners',async t=>{
  for(const field of ['browserProfileId','extensionBundleId','serverInstanceId','backgroundEpoch','contentEpoch','launchToken','browserTabId'])await t.test(field,async()=>{
    const f=fixture({before:{[field]:undefined}});await assert.rejects(f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true}),{code:'EXTENSION_RELOAD_SOURCE_IDENTITY_REQUIRED'});
    assert.deepEqual(f.commands,[]);assert.equal(f.hub.listenerCount('client.ready'),0);
  });
});

test('explicit target bundle and version still require the original tab and two fresh runtime epochs',async()=>{
  const f=fixture({reconnect:{id:'fresh-client-for-original-tab',extensionBundleId:'b'.repeat(32),extensionVersion:'2.4.6'}});
  const result=await f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true,expectedVersion:'2.4.6',expectedBundleId:'b'.repeat(32),timeoutMs:30000});
  assert.equal(result.reconnected.browserTabId,7);assert.equal(result.reconnected.extensionBundleId,'b'.repeat(32));assert.equal(f.commands.length,1);
});

test('unchanged worker readiness before terminal result never infers successful runtime reload',async()=>{
  const f=fixture({reconnect:{backgroundEpoch:'old-worker'}});
  const original=f.controller.sendCommand;let settle;
  f.controller.sendCommand=(...args)=>{void original(...args);return new Promise(resolve=>{settle=resolve;});};
  const result=f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true,timeoutMs:30000});
  await new Promise(setImmediate);settle({scheduled:true});
  await assert.rejects(result,{code:'EXTENSION_RELOAD_RECONNECT_PENDING'});assert.equal(f.commands.length,1);
});

test('legacy fallback permission cannot replace or close the original tab when its runtime proof is missing',async()=>{
  const f=fixture({reconnect:{backgroundEpoch:'old-worker'}});
  await assert.rejects(f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:false,allowMaintenancePageBootstrap:true,timeoutMs:30000}),{code:'EXTENSION_RELOAD_RECONNECT_PENDING'});
  assert.equal(f.commands.length,1);assert.equal(f.commands[0].type,'extension.reload');
});
