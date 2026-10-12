export const CONNECTION_WATCHDOG_ALARM = 'bridge.connection.watchdog';
const PROFILE_KEY='directorDesktop:browserProfileId:v1';
const SESSION_KEY='directorDesktop:controllerBrowserSession:v1';
const UUID=/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const terminal=new Set(['succeeded','rejected','cancelled']);
function socketAlive(state={}) {return state.ws?.readyState===WebSocket.CONNECTING || state.ws?.readyState===WebSocket.OPEN;}
function root(url) {try{const u=new URL(url);return u.origin==='https://chatgpt.com'&&u.pathname==='/'&&!u.search&&!u.hash&&!u.username&&!u.password;}catch{return false;}}
function idle(runtime) {
  return runtime?.schemaVersion===6 && runtime.lease===null && Array.isArray(runtime.outbox)&&runtime.outbox.length===0
    && runtime.commands&&Object.values(runtime.commands).every(c=>terminal.has(c.status))
    && runtime.effects&&Object.values(runtime.effects).every(e=>terminal.has(e.status))
    && runtime.downloads&&Object.keys(runtime.downloads).length===0;
}

// A worker/extension reload may clear session ownership. Never relabel or reload
// the old page in that case. Allocate one NEW empty, minimized control window;
// preserve the old opened receipt in history, and hold uncertain creation.
export async function bootstrapOwnedController({browser=chrome,connections,now=Date.now}={}) {
  const held=reason=>({status:'held',reason});
  try{
    if(!browser.windows?.create||!browser.tabs?.update||!browser.storage?.session?.set)return held('bootstrap_native_capability_missing');
    if(Array.from(connections.values()).some(s=>!s.closed&&(socketAlive(s)||s.activeRequest||s.protocolReady)))return held('bootstrap_transport_present');
    const read=async()=>{
      const identity=await browser.storage.local.get(PROFILE_KEY),profile=identity[PROFILE_KEY];if(!UUID.test(profile||''))throw Error('bootstrap_profile');
      const key='directorDesktop:controlTab:v1:'+profile,journalKey='directorDesktop:controlReconnect:v1:'+profile;
      const [local,session,tabs]=await Promise.all([browser.storage.local.get([PROFILE_KEY,key,journalKey]),browser.storage.session.get(null),browser.tabs.query({})]);
      const control=local[key],launch='bridge-director-control-'+profile;
      if(local[PROFILE_KEY]!==profile||control?.status!=='opened'||control.launchToken!==launch||!root(control.requestedUrl)
        ||!UUID.test(control.browserSession||'')||!Number.isSafeInteger(control.tabId)||control.tabId<=0)throw Error('bootstrap_original_not_opened');
      const u=new URL(control.serverUrl);if(u.protocol!=='http:'||!['127.0.0.1','localhost'].includes(u.hostname)||u.username||u.password||u.pathname!=='/'||u.search||u.hash)throw Error('bootstrap_server');
      if(!Array.isArray(tabs)||tabs.some(t=>!Number.isSafeInteger(t.id)||t.id<=0||typeof t.url!=='string'||typeof t.incognito!=='boolean')||new Set(tabs.map(t=>t.id)).size!==tabs.length)throw Error('bootstrap_native_inventory');
      if(Object.entries(session).some(([k,v])=>k.startsWith('chatgptBridgeLaunchedTab:')&&v?.launchToken===launch))throw Error('bootstrap_owner_exists');
      if(Object.entries(session).filter(([k])=>k.startsWith('chatgptBridgeV6:tab:')).some(([,v])=>!idle(v)))throw Error('bootstrap_execution_unresolved');
      const sid=session[SESSION_KEY];if(sid!==undefined&&!UUID.test(sid||''))throw Error('bootstrap_session_damaged');
      if(local[journalKey])throw Error('bootstrap_existing_reservation');
      return {profile,key,journalKey,control,session,tabs:tabs.map(t=>({id:t.id,url:t.url,incognito:t.incognito})),sessionId:sid||null};
    };
    const first=await read(),second=await read();if(JSON.stringify(first)!==JSON.stringify(second))return held('bootstrap_native_drift');
    const sid=first.sessionId||crypto.randomUUID().toLowerCase();if(!UUID.test(sid))return held('bootstrap_session_identity');
    const intent={version:'director-owned-controller-bootstrap/v1',status:'reserved',at:now(),originalController:first.control,browserSession:sid,foreground:false,generationCalled:false};
    await browser.storage.local.set({[first.journalKey]:intent});
    if(JSON.stringify((await browser.storage.local.get(first.journalKey))[first.journalKey])!==JSON.stringify(intent))return held('bootstrap_intent_not_persisted');
    const actual=(await browser.storage.local.get(first.key))[first.key];if(JSON.stringify(actual)!==JSON.stringify(first.control))return held('bootstrap_controller_changed');
    if(first.sessionId===null){await browser.storage.session.set({[SESSION_KEY]:sid});if((await browser.storage.session.get(SESSION_KEY))[SESSION_KEY]!==sid)return held('bootstrap_session_not_persisted');}
    const url=new URL('https://chatgpt.com/');url.hash=new URLSearchParams({'chatgpt-bridge-launch':first.control.launchToken,'chatgpt-bridge-server':first.control.serverUrl}).toString();
    let created;
    try{created=await browser.windows.create({url:url.toString(),focused:false,state:'minimized',type:'normal',incognito:false});}
    catch{await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('bootstrap_create_unknown');}
    const pages=created?.tabs;
    if(!Number.isSafeInteger(created?.id)||created.focused!==false||created.state!=='minimized'||created.incognito!==false||!Array.isArray(pages)||pages.length!==1||!Number.isSafeInteger(pages[0]?.id)||pages[0].id<=0||first.tabs.some(t=>t.id===pages[0].id)){await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('bootstrap_receipt_unknown');}
    const tabId=pages[0].id;
    await browser.tabs.update(tabId,{autoDiscardable:false});
    await browser.storage.session.set({['chatgptBridgeLaunchedTab:'+tabId]:{launchToken:first.control.launchToken,requestedUrl:'https://chatgpt.com/',serverUrl:first.control.serverUrl,createdAt:now()}});
    const control={...first.control,tabId,browserSession:sid,status:'opened',retiredControllers:[...(first.control.retiredControllers||[]),{...first.control,retiredAt:now(),reason:'session_ownership_lost_old_page_untouched'}]};
    await browser.storage.local.set({[first.key]:control,[first.journalKey]:{...intent,status:'bootstrap_completed',controllerSha256:await digest(control),tabId,windowId:created.id,attempts:1}});
    return {status:'bootstrap_completed',tabId,windowId:created.id,foreground:false,generationCalled:false};
  }catch(error){return held(String(error.message||'bootstrap_read_failed'));}
}

// Runs in the extension, independently of content ports or server clients. No
// tab creation, extension reload, model submission or user-page navigation.
export async function recoverOwnedController({browser=chrome,connections,now=Date.now}={}) {
  const held=reason=>({status:'held',reason});
  if (!browser.storage?.local?.get || !browser.storage?.local?.set || !browser.storage?.session?.get
    || !browser.tabs?.query || !browser.tabs?.reload || !browser.windows?.get) return held('native_capability_missing');
  const sample=async()=>{
    const first=await browser.storage.local.get(PROFILE_KEY),profile=first[PROFILE_KEY];
    if(!UUID.test(profile||''))throw Error('profile_identity');
    const key='directorDesktop:controlTab:v1:'+profile,journalKey='directorDesktop:controlReconnect:v1:'+profile;
    const [local,session,tabs]=await Promise.all([browser.storage.local.get([PROFILE_KEY,key,journalKey]),browser.storage.session.get(null),browser.tabs.query({})]);
    const control=local[key],launch='bridge-director-control-'+profile;
    if(local[PROFILE_KEY]!==profile || control?.status!=='opened' || control.launchToken!==launch || !root(control.requestedUrl)
      || !UUID.test(control.browserSession||'') || control.browserSession!==session[SESSION_KEY]
      || !Number.isSafeInteger(control.tabId) || control.tabId<=0 || !Array.isArray(tabs)) throw Error('controller_identity');
    const owned=Object.entries(session).filter(([k,v])=>k.startsWith('chatgptBridgeLaunchedTab:')&&v?.launchToken===launch);
    const record=owned[0]?.[1],matches=tabs.filter(t=>t.id===control.tabId);
    if(owned.length!==1 || owned[0][0]!=='chatgptBridgeLaunchedTab:'+control.tabId || record.serverUrl!==control.serverUrl
      || record.requestedUrl!==control.requestedUrl || matches.length!==1)throw Error('session_ownership');
    const tab=matches[0];const window=await browser.windows.get(tab.windowId);
    if(!root(tab.url) || tab.incognito!==false || window.id!==tab.windowId || window.focused!==false
      || typeof tab.active!=='boolean' || tab.active && window.state!=='minimized')throw Error('user_visible_or_changed');
    const states=[...connections.values()].filter(c=>c.tabId===control.tabId&&!c.closed);
    if(states.some(socketAlive))return {connected:true};
    if(states.some(c=>c.activeRequest || c.protocolReady))throw Error('transport_outcome_unresolved');
    const runtime=session['chatgptBridgeV6:tab:'+control.tabId];
    if(!idle(runtime))throw Error('controller_execution_unresolved');
    return {profile,key,journalKey,control,record,runtime,tab:{id:tab.id,windowId:tab.windowId,url:tab.url,active:tab.active,incognito:tab.incognito,discarded:tab.discarded},window:{id:window.id,state:window.state,focused:window.focused},journal:local[journalKey]||null};
  };
  try {
    const first=await sample();if(first.connected)return {status:'connected'};
    const old=first.journal;
    // A reload response is not proof that content reconnected. Never replay an
    // interrupted/failed reservation; wait for the original page's handshake.
    if(old && old.controllerSha256!==await digest(first.control))return held('journal_identity_changed');
    if(old?.status==='reserved'||old?.status==='outcome_unknown')return held('original_reload_unconfirmed');
    if(old && now()-old.at<120000)return held('reconnect_cooldown');
    if((old?.attempts||0)>=2)return held('reconnect_budget_exhausted');
    const second=await sample();if(second.connected)return {status:'connected'};
    if(JSON.stringify(first)!==JSON.stringify(second))return held('native_state_changed');
    const intent={version:'director-owned-controller-reconnect/v1',controllerSha256:await digest(first.control),tabId:first.tab.id,
      browserSession:first.control.browserSession,launchToken:first.control.launchToken,status:'reserved',attempts:(old?.attempts||0)+1,at:now(),foreground:false,generationCalled:false};
    await browser.storage.local.set({[first.journalKey]:intent});
    const confirmed=(await browser.storage.local.get(first.journalKey))[first.journalKey];
    if(JSON.stringify(confirmed)!==JSON.stringify(intent))return held('intent_not_persisted');
    const after=await sample();const expected={...first,journal:intent};
    if(after.connected||JSON.stringify(after)!==JSON.stringify(expected))return held('pre_reload_state_changed');
    try{await browser.tabs.reload(first.tab.id);}
    catch{await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('reload_outcome_unknown');}
    await browser.storage.local.set({[first.journalKey]:{...intent,status:'reload_requested'}});
    return {status:'reload_requested',tabId:first.tab.id,foreground:false,generationCalled:false};
  }catch(error){const reason=String(error.message||'native_read_failed');
    if(reason==='controller_identity'||reason==='session_ownership')return bootstrapOwnedController({browser,connections,now});
    return held(reason);}
}
async function digest(value){const bytes=new TextEncoder().encode(JSON.stringify(value));return [...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(b=>b.toString(16).padStart(2,'0')).join('');}

export function createConnectionWatchdog({alarms,connections,openConnection,periodInMinutes=1,recoverController}={}) {
  if(!connections||typeof openConnection!=='function')throw new TypeError('Connection watchdog requires connections and openConnection');
  let running=null;
  async function run(){
    if(running)return running;
    running=(async()=>{const retries=[];for(const state of connections.values()){
      if(!state||state.closed||socketAlive(state))continue;clearTimeout(state.reconnectTimer);state.reconnectTimer=null;retries.push(Promise.resolve(openConnection(state)));
    }await Promise.allSettled(retries);
    if(typeof recoverController==='function'&&!Array.from(connections.values()).some(s=>!s.closed&&socketAlive(s)))await recoverController();
    return retries.length;})();try{return await running;}finally{running=null;}
  }
  async function arm(){if(!alarms?.create)return false;await alarms.create(CONNECTION_WATCHDOG_ALARM,{periodInMinutes:Math.max(1,Number(periodInMinutes)||1)});return true;}
  return Object.freeze({arm,run,handlesAlarm:name=>String(name||'')===CONNECTION_WATCHDOG_ALARM});
}
