const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),{EventEmitter}=require('node:events');
function fixture(options={}){
  const source=fs.readFileSync(path.join(__dirname,'../overrides/src/bridge/coordinator/browserTabCoordinator.js'),'utf8').replace(/^import[\s\S]*?from [^;]+;\r?\n/gm,'').replace(/export (?=function|class)/g,'');
  const context=vm.createContext({URL,Date,makeRequestId:()=>assert.fail('must not allocate replacement identity'),
    BROWSER_LAUNCH_TOKEN_RE:/^bridge-/,browserLaunchMetadataFromUrl:()=>({}),safeChatGptUrl:x=>x,normalizeLaunchedClient:x=>x,
    setTimeout:(callback,ms)=>setTimeout(callback,ms>=20000?100:2),clearTimeout});
  vm.runInContext(source+';this.Coordinator=BrowserTabCoordinator',context);
  const hub=new EventEmitter(),before={id:'owned-source',browserTabId:7,launchToken:'bridge-director-desktop-12345678-1234-4234-8234-123456789abc',url:'https://chatgpt.com/',ready:true,connectedAt:'2000-01-01T00:00:00Z'};
  hub.clients=[before];hub.activeClient=before;const commands=[];
  const controller=new context.Coordinator({hub,rankClients:x=>x,runtimeOptions:{publicBaseUrl:'http://127.0.0.1:53179',openExternalUrl:()=>assert.fail('must not open foreground bootstrap')},
    sendCommand:async(type,payload)=>{commands.push({type,payload});if(options.commandError)throw Object.assign(new Error('original guard rejected'),{code:'COMMAND_REJECTED'});
      if(options.reconnect)queueMicrotask(()=>hub.emit('client.ready',{...before,extensionVersion:'2.4.5',extensionBundleId:'original-bundle'}));
      return {scheduled:true,pageReload:{armed:true,delayMs:2500}};}});
  controller.openSystemBrowserTab=()=>assert.fail('must not open OS fallback');
  return {controller,commands};
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
test('original owned page reconnect completes the existing reload without another action',async()=>{
  const f=fixture({reconnect:true}),result=await f.controller.reloadExtension({sourceClientId:'owned-source',ownedTabsOnly:true,allowMaintenancePageBootstrap:false,timeoutMs:30000});
  assert.equal(result.reconnected.id,'owned-source');assert.equal(f.commands.length,1);
});
