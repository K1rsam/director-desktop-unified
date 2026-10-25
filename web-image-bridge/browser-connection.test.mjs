import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';
import {browserIdentity, waitForIdentity, connectAvailable} from './bridge.mjs';

const profiles={profiles:[{contextId:'edgeProfile',extensionConnected:true},{contextId:'chromeProfile',extensionConnected:true}]};
const wrong=()=>Object.assign(new Error('BROWSER_KIND_MISMATCH'),{code:'BROWSER_KIND_MISMATCH'});
test('explicit Chrome selection skips saved Edge and preserves the account check',async()=>{
  const calls=[];
  const result=await connectAvailable({profile:'edgeProfile',accountKey:'same',requestedBrowser:'chrome'},async()=>profiles,async c=>{
    calls.push(c);
    if(c.profile==='edgeProfile')throw wrong();
    return{profile:c.profile,browserKind:'chrome',accountKey:'same',ready:true};
  });
  assert.equal(result.profile,'chromeProfile');assert.equal(calls.length,2);
  assert.ok(calls.every(c=>c.accountKey==='same'));
});
test('different Chrome account requires explicit confirmation and never silently changes the saved account',async()=>{
  const config={profile:'edgeProfile',accountKey:'original',requestedBrowser:'chrome'};
  const inspect=async c=>{if(c.profile==='edgeProfile')throw wrong();return{profile:c.profile,browserKind:'chrome',accountKey:'new',ready:true};};
  await assert.rejects(connectAvailable(config,async()=>profiles,inspect),/ACCOUNT_CHANGED/);
  assert.equal(config.accountKey,'original');
  const result=await connectAvailable({...config,allowAccountChange:true},async()=>profiles,async c=>{assert.equal(c.accountKey,'');return inspect(c);});
  assert.equal(result.accountKey,'new');assert.equal(config.accountKey,'original');
  await assert.rejects(connectAvailable({allowAccountChange:true},async()=>profiles,inspect),/INVALID_CONFIG/);
});
test('selected Chrome cannot fall back to Edge, and multiple Chrome profiles require a choice',async()=>{
  await assert.rejects(connectAvailable({requestedBrowser:'chrome'},async()=>profiles,async()=>({browserKind:'edge',accountKey:'same'})),/OPENCLI_BROWSER_NOT_CONNECTED/);
  await assert.rejects(connectAvailable({requestedBrowser:'chrome'},async()=>profiles,async c=>({profile:c.profile,browserKind:'chrome',accountKey:'same'})),/BROWSER_SELECTION_REQUIRED/);
});
function page({protocol='https:',origin='https://chatgpt.com',status=200,networkError=false,composer=true,elements=[]}={}){
  const calls=[];
  const document={querySelector:()=>composer?{}:null,querySelectorAll:()=>elements};
  Object.defineProperty(document,'body',{get:()=>assert.fail('identity must not read body or sidebar text')});
  const globals={location:{protocol,origin},navigator:{userAgent:'Chrome/155.0'},document,getComputedStyle:e=>({display:'block',visibility:e.visibility??'visible'}),crypto:webcrypto,TextEncoder,AbortSignal,
    fetch:async(url,options)=>{calls.push({url,options});if(networkError)throw Error('Failed to fetch');return{status,ok:status===200,json:async()=>({user:{id:'synthetic-id'},accessToken:'must-not-leave-page'})};}};
  return{calls,evaluate:code=>vm.runInNewContext(code,globals),sleep:async()=>{}};
}
test('browser error document and fetch failures stop immediately as network errors, without login retries',async()=>{
  for(const options of [{protocol:'chrome-error:',origin:'null'},{networkError:true},{status:502}]){
    const p=page(options);let sleeps=0;p.sleep=async()=>{sleeps++;};
    await assert.rejects(waitForIdentity(p),e=>e.code==='BROWSER_NETWORK_UNAVAILABLE');assert.equal(sleeps,0);
    assert.ok(p.calls.length<=1);
  }
});

function label(text,{excluded=false,visible=true,hidden=false,visibility='visible'}={}){
  return {visibility,closest:selector=>selector.startsWith('nav')?(excluded?{}:null):(hidden?{}:null),getClientRects:()=>visible?[{}]:[],
    get childNodes(){if(excluded)assert.fail('navigation/conversation text must not be read');return[{nodeType:3,data:text}];},
    get innerText(){assert.fail('whole subtree text must not be read');}};
}
test('official account loading failure is immediate and recoverable even with a valid auth session',async()=>{
  for(const title of ['无法加载你的账户','无法加载您的账户。','Unable to load your account',"We couldn't load your account"]){
    const p=page({composer:false,elements:[label('private navigation',{excluded:true}),label(title)]});
    p.sleep=async()=>assert.fail('an explicit page failure must not wait through hydration');
    p.click=p.fill=async()=>assert.fail('preflight must not submit, reload or log out');
    await assert.rejects(waitForIdentity(p),e=>e.code==='CHATGPT_ACCOUNT_LOAD_FAILED'&&!e.message.includes('private'));
    assert.equal(p.calls.length,0,'no need to fetch tokens when the page itself gives this exact failure');
  }
});
test('ordinary hydration, hidden stale errors and quoted conversation/sidebar labels are not logout or account failure',async()=>{
  for(const elements of [[],[label('无法加载你的账户',{excluded:true})],[label('无法加载你的账户',{visible:false})],
    [label('无法加载你的账户',{hidden:true})],[label('无法加载你的账户',{visibility:'hidden'})],[label('正在加载账户')]]){
    const p=page({composer:false,elements});await assert.rejects(waitForIdentity(p,'',2),e=>e.code==='BRIDGE_BUSY');
    assert.equal(p.calls.length,2);
  }
  const recovered=page({composer:true,elements:[label('无法加载你的账户')]});
  assert.equal((await waitForIdentity(recovered)).composer,true,'a restored composer is usable; no automatic retry click');
});
test('account failure scan is bounded and never returns page text or account tokens',async()=>{
  const elements=Array.from({length:1200},()=>label('ordinary UI'));elements.push(label('无法加载你的账户'));
  const identity=await browserIdentity(page({composer:false,elements}));
  assert.equal(identity.composer,false);assert.deepEqual(Object.keys(identity).sort(),['accountKey','browserKind','composer']);
  assert.ok(!JSON.stringify(identity).includes('must-not-leave-page'));
});
test('website checks and request limits are not reported as logout',async()=>{
  await assert.rejects(browserIdentity(page({status:403})),e=>e.code==='BROWSER_CHECK_REQUIRED');
  await assert.rejects(browserIdentity(page({status:429})),e=>e.code==='BRIDGE_BUSY');
  await assert.rejects(browserIdentity(page({status:401})),e=>e.code==='AUTH_REQUIRED');
  await assert.rejects(waitForIdentity(page({protocol:'about:',origin:'null'}),'',2),e=>e.code==='CHATGPT_PAGE_LOADING');
});
test('identity response exposes only a digest and actual browser kind, with a bounded session fetch',async()=>{
  const p=page();const identity=await browserIdentity(p);
  assert.match(identity.accountKey,/^[a-f0-9]{64}$/);assert.equal(identity.browserKind,'chrome');assert.equal(identity.composer,true);
  assert.equal(identity.accessToken,undefined);assert.ok(!JSON.stringify(identity).includes('synthetic-id'));
  assert.equal(p.calls[0].options.credentials,'include');assert.ok(p.calls[0].options.signal instanceof AbortSignal);
});
