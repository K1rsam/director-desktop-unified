const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const PROFILE='b84d4521-56ba-4f84-9948-090f82d639e7',REQUEST='c6a483f1-ad60-4e7c-90ce-dac6dba54267';
function fixture(change=()=>{}){
 const facts={source:{id:7,windowId:10,incognito:false,url:'https://chatgpt.com/'},record:{launchToken:'bridge-director-control-'+PROFILE,serverUrl:'http://127.0.0.1:53179'},calls:[],returnedWindow:10};change(facts);
 const chrome={runtime:{},tabs:{get:async()=>({...facts.source}),create:(options,done)=>{facts.calls.push(['create',options]);done({id:31,windowId:facts.returnedWindow});},
  update:(id,options,done)=>{facts.calls.push(['update',id,options]);done({id,windowId:10});},remove:(id,done)=>{facts.calls.push(['remove',id]);done();}}};
 const port={sender:{tab:{id:7,incognito:false}}},connections=new Map([[port,{serverUrl:facts.record.serverUrl}]]);
 const context=vm.createContext({chrome,URL,URLSearchParams,Date,console,setTimeout});
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/background/tabController.js'),'utf8').replace(/^export /gm,''),context);
 const api=context.createTabController({connections,safeBridgeServerUrl:u=>u,rememberLaunchedTab:async()=>{},readLaunchedTab:async()=>facts.record,forgetLaunchedTab:async()=>{},isStableLaunchToken:()=>true});
 return {...facts,open:options=>api.openBridgeTab(port,{active:false,launchToken:'bridge-director-desktop-'+REQUEST,bridgeServerUrl:'http://127.0.0.1:53179',...options})};
}
test('owned work page explicitly uses controller window instead of Chrome last-focused user window',async()=>{
 const f=fixture();await f.open();assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0])),['create',{url:'about:blank',active:false,windowId:10}]);
 assert.equal(f.calls.filter(c=>c[0]==='update')[0][2].active,false);
});
test('unproved source foreground request or wrong creation window cannot submit or navigate',async()=>{
 for(const change of [f=>f.source.windowId=null,f=>f.source.incognito=true,f=>f.source.url='https://chatgpt.com/c/user',f=>f.record.launchToken='',f=>f.record.serverUrl='http://elsewhere']){
  const f=fixture(change);await assert.rejects(f.open());assert.equal(f.calls.length,0);
 }
 const active=fixture();await assert.rejects(active.open({active:true}));assert.equal(active.calls.length,0);
 const mismatch=fixture(f=>f.returnedWindow=99);await assert.rejects(mismatch.open());assert.ok(mismatch.calls.every(c=>c[0]!=='update'));assert.equal(mismatch.calls.at(-1)[0],'remove');
});
