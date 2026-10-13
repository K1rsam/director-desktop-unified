const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const PROFILE='5948d446-fd4e-4f69-bb6d-6eb7c204d148', key='directorDesktop:controlTab:v1:'+PROFILE;
function fixture(change={}){
 const local={'directorDesktop:browserProfileId:v1':PROFILE,[key]:{launchToken:'bridge-director-control-'+PROFILE,serverUrl:'http://127.0.0.1:53179'},'chatgptBridge:secret:bridge.token':'synthetic-secret',...change};
 const browser={storage:{local:{get:async keys=>Object.fromEntries((Array.isArray(keys)?keys:[keys]).filter(k=>k in local).map(k=>[k,local[k]]))}}};
 const context=vm.createContext({URL,AbortController,setTimeout,clearTimeout,console});
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/authPreflight.js'),'utf8').replace(/^export /gm,''),context);
 return {run:options=>context.reportBackgroundHealth({browser,backgroundEpoch:'worker-1',bundleId:'a'.repeat(32),extensionVersion:'2.4.5',recovery:{status:'held',reason:'bootstrap_owner_exists'},...options})};
}
test('background reports without a content port or browser operation and carries no task text',async()=>{
 let calls=0;const f=fixture();const result=await f.run({fetchImpl:async(url,opts)=>{calls++;assert.equal(url.href,'http://127.0.0.1:53179/extension/background/status');
  assert.equal(opts.headers['X-Bridge-Token'],'synthetic-secret');assert.equal(opts.redirect,'error');const body=JSON.parse(opts.body);assert.equal(body.reason,'bootstrap_owner_exists');assert.equal(body.connectionCount,0);
  assert.deepEqual(Object.keys(body).sort(),['backgroundEpoch','bundleId','connectionCount','extensionVersion','profileId','reason','schemaVersion','status']);
  assert.ok(!opts.body.includes('synthetic-secret'));return {ok:true};}});assert.equal(result.ok,true);assert.equal(calls,1);
});
test('unbound, foreign server and missing secret do not send diagnostics',async()=>{
 for(const change of [{'directorDesktop:browserProfileId:v1':'bad'},{[key]:{launchToken:'bridge-director-control-'+PROFILE,serverUrl:'https://example.com/'}},{[key]:{launchToken:'wrong',serverUrl:'http://127.0.0.1:53179'}},{'chatgptBridge:secret:bridge.token':''}]){
  assert.equal((await fixture(change).run({fetchImpl:()=>assert.fail('must not transmit')})).ok,false);
 }
});
test('health reporting bounds network time and failure text is not leaked',async()=>{
 const r=await fixture().run({timeoutMs:5,fetchImpl:async(_url,{signal})=>new Promise((_,reject)=>signal.addEventListener('abort',()=>reject(Error('synthetic-secret')),{once:true}))});
 assert.equal(r.ok,false);assert.equal(r.reason,'report_unavailable');assert.ok(!JSON.stringify(r).includes('synthetic-secret'));
});
test('arbitrary recovery text is reduced to a code',async()=>{
 await fixture().run({recovery:{status:'held',reason:'private prompt or URL'},fetchImpl:async(_url,options)=>{assert.equal(JSON.parse(options.body).reason,'unknown');return {ok:true};}});
});
test('a loopback redirect cannot forward the bridge token to another server',async t=>{
 const http=require('node:http');let redirectedRequests=0,sourceRequests=0;
 const target=http.createServer((_req,res)=>{redirectedRequests++;res.end('unexpected');});
 await new Promise(resolve=>target.listen(0,'127.0.0.1',resolve));
 t.after(()=>new Promise(resolve=>target.close(resolve)));
 const source=http.createServer((req,res)=>{sourceRequests++;assert.equal(req.headers['x-bridge-token'],'synthetic-secret');
  res.writeHead(302,{Location:`http://127.0.0.1:${target.address().port}/unexpected`});res.end();});
 await new Promise(resolve=>source.listen(0,'127.0.0.1',resolve));
 t.after(()=>new Promise(resolve=>source.close(resolve)));
 const result=await fixture({[key]:{launchToken:'bridge-director-control-'+PROFILE,serverUrl:`http://127.0.0.1:${source.address().port}`}}).run({fetchImpl:fetch});
 assert.equal(sourceRequests,1);assert.equal(redirectedRequests,0);assert.equal(result.ok,false);assert.equal(result.reason,'report_unavailable');
});
