const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const body=()=>({schemaVersion:'extension-background-health/v1',profileId:'5948d446-fd4e-4f69-bb6d-6eb7c204d148',bundleId:'a'.repeat(32),backgroundEpoch:'worker-1',extensionVersion:'2.4.5',status:'held',reason:'controller_identity',connectionCount:0});
function fixture(){
 let now=1000;const routes=new Map(),middleware=[];
 const register=(method,route,...handlers)=>routes.set(method+' '+route,[...middleware,...handlers]);
 const router={use:fn=>middleware.push(fn),get:(...args)=>register('GET',...args),post:(...args)=>register('POST',...args),delete(){}};
 class HttpError extends Error{constructor(statusCode,message){super(message);this.statusCode=statusCode;}}
 const context=vm.createContext({URL,path,fs:fs.promises,Date:{now:()=>now},process:{env:{}},HttpError,express:{Router:()=>router},fileURLToPath:()=>'/synthetic/src/routes.js',
  config:{apiToken:'api-test-secret'},secureTokenEqual:(a,b)=>a===b,registerWorkflowRoutes(){},registerFullPowerBridgeRoutes(){},registerPassivePromptRoutes(){}});
 const source=fs.readFileSync(path.join(__dirname,'../overrides/src/routes.js'),'utf8').replace(/^import[\s\S]*?from [^;]+;\r?\n/gm,'').replace(/export (?=function|class)/g,'').replace(/import\.meta\.url/g,JSON.stringify('file:///synthetic/src/routes.js'));
 vm.runInContext(source+';this.registry=createBackgroundHealthRegistry;this.router=createRouter;',context);
 const bridge={isLocalRequest:r=>r.local!==false,validateBridgeToken:t=>t==='bridge-test-secret',health:()=>({ok:false,clients:[],pendingRequests:0,pendingCommands:0})};context.router(bridge,{});
 async function request(method,route,changes={}){
  const result={};const req={app:{locals:{}},headers:{},query:{},body:body(),...changes};const res={json:data=>{result.data=data;}};
  for(const handler of routes.get(method+' '+route)){let next=false;await handler(req,res,error=>{if(error)result.error=error;else next=true;});if(result.error||!next)break;}return result;
 }
 return {request,registry:()=>context.registry(()=>now),advance:ms=>now+=ms};
}
test('background report requires local bridge authentication and status read requires API authentication',async()=>{
 const f=fixture();assert.equal((await f.request('POST','/extension/background/status')).error.statusCode,403);
 assert.equal((await f.request('POST','/extension/background/status',{local:false,headers:{'x-bridge-token':'bridge-test-secret'}})).error.statusCode,403);
 assert.equal((await f.request('POST','/extension/background/status',{headers:{'x-bridge-token':'bridge-test-secret'}})).data.ok,true);
 assert.equal((await f.request('GET','/browser/background/status')).error.statusCode,401);
 const r=await f.request('GET','/health',{headers:{authorization:'Bearer api-test-secret'}});assert.equal(r.data.ok,false);assert.equal(r.data.clients,0);assert.equal(r.data.backgroundRuntimes.length,1);
 assert.ok(!JSON.stringify(r.data).includes('secret'));
});
test('registry rejects private or malformed fields and reports server-calculated age without making it ready',()=>{
 const f=fixture(),r=f.registry();for(const change of [{prompt:'private'},{profileId:'invalid'},{connectionCount:-1},{reason:'full error text with a credential'},{backgroundEpoch:''}])assert.throws(()=>r.accept({...body(),...change}),{statusCode:400});
 r.accept(body());f.advance(90_001);assert.equal(r.snapshot()[0].ageMs,90_001);assert.equal(r.snapshot()[0].status,'held');f.advance(300_000);assert.equal(r.snapshot().length,0);
});
test('different live worker identities remain visible and cannot silently replace each other',()=>{
 const f=fixture(),r=f.registry();r.accept(body());r.accept({...body(),backgroundEpoch:'worker-2'});assert.equal(r.snapshot().length,2);
 r.accept({...body(),reason:'native_state_changed'});assert.equal(r.snapshot().length,2);assert.equal(r.snapshot().find(x=>x.backgroundEpoch==='worker-1').reason,'native_state_changed');
});
test('array identities cannot coerce into and overwrite an existing worker record',()=>{
 const f=fixture(),r=f.registry();r.accept(body());
 for(const change of [{profileId:[body().profileId]},{bundleId:[body().bundleId]}]) {
  assert.throws(()=>r.accept({...body(),...change,reason:'replacement'}),{statusCode:400});
 }
 assert.equal(r.snapshot().length,1);assert.equal(r.snapshot()[0].reason,'controller_identity');
 assert.equal(typeof r.snapshot()[0].profileId,'string');assert.equal(typeof r.snapshot()[0].bundleId,'string');
});
