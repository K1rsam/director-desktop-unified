export const CONNECTION_WATCHDOG_ALARM = 'bridge.connection.watchdog';
const PROFILE_KEY='directorDesktop:browserProfileId:v1';
const SESSION_KEY='directorDesktop:controllerBrowserSession:v1';
const UUID=/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const terminal=new Set(['succeeded','rejected','cancelled']);
function canonical(v){if(Array.isArray(v))return v.map(canonical);if(v&&typeof v==='object')return Object.fromEntries(Object.keys(v).sort().map(k=>[k,canonical(v[k])]));return v;}
function same(a,b){return JSON.stringify(canonical(a))===JSON.stringify(canonical(b));}
function nativeInventory(tabs){
  if(!Array.isArray(tabs)||tabs.some(t=>!Number.isSafeInteger(t.id)||t.id<=0||typeof t.url!=='string'||typeof t.incognito!=='boolean')
    ||new Set(tabs.map(t=>t.id)).size!==tabs.length)throw Error('maintenance_rebind_native_inventory');
  return tabs.map(t=>({id:t.id,url:t.url,incognito:t.incognito})).sort((a,b)=>a.id-b.id);
}
function socketAlive(state={}) {return state.ws?.readyState===WebSocket.CONNECTING || state.ws?.readyState===WebSocket.OPEN;}
function root(url) {try{const u=new URL(url);return u.origin==='https://chatgpt.com'&&u.pathname==='/'&&!u.search&&!u.hash&&!u.username&&!u.password;}catch{return false;}}
function idle(runtime) {
  return runtime?.schemaVersion===6 && runtime.lease===null && Array.isArray(runtime.outbox)&&runtime.outbox.length===0
    && runtime.commands&&Object.values(runtime.commands).every(c=>terminal.has(c.status))
    && runtime.effects&&Object.values(runtime.effects).every(e=>terminal.has(e.status))
    && runtime.downloads&&Object.keys(runtime.downloads).length===0;
}
function archiveJournal(journal){if(!journal)return [];const {incidentHistory,...current}=journal;return [...(incidentHistory||[]),current];}
async function maintenancePreimage(control,journal) {
  const {reloadInvalidated,...plain}=control;
  if(journal&&!['connection_confirmed','bootstrap_completed','reload_requested'].includes(journal.status))throw Error('maintenance_rebind_journal_status_unresolved');
  if(journal&&!/^[a-f0-9]{64}$/.test(journal.controllerSha256||''))throw Error('maintenance_rebind_journal_hash_unresolved');
  const pinned=reloadInvalidated.preMaintenanceController;
  if(pinned!==undefined){
    if(!pinned||!same(Object.fromEntries(Object.entries(pinned).filter(([k])=>k!=='reloadInvalidated')),plain)
      ||reloadInvalidated.preMaintenanceControllerSha256!==await digest(pinned)
      ||journal&&journal.controllerSha256!==reloadInvalidated.preMaintenanceControllerSha256)throw Error('maintenance_rebind_journal_hash_unresolved');
    return {controller:pinned,source:'maintenance_snapshot'};
  }
  if(!journal)return {controller:plain,source:'no_previous_journal'};
  if(await digest(plain)===journal.controllerSha256)return {controller:plain,source:'marker_free_preimage'};
  // Older bootstrap code inherited its predecessor's marker. Maintenance then
  // replaced that marker, while the confirmed journal still hashes the original
  // complete controller. Only retained history can recover that exact preimage.
  const candidates=[],seen=new Set();let examined=0;
  const visit=(value,path,depth=0)=>{
    if(!value||typeof value!=='object')return;
    if(depth>32||++examined>10000)throw Error('maintenance_rebind_history_bounds');
    if(value.reloadInvalidated&&typeof value.reloadInvalidated==='object'){
      const marker=value.reloadInvalidated,key=JSON.stringify(canonical(marker));
      if(!seen.has(key)){seen.add(key);candidates.push({controller:{...plain,reloadInvalidated:marker},source:path+'.reloadInvalidated'});}
    }
    for(const [key,child]of Object.entries(value))visit(child,path+'.'+key,depth+1);
  };
  visit(control.retiredControllers,'retiredControllers');visit(journal.incidentHistory,'incidentHistory');
  for(const candidate of candidates)if(await digest(candidate.controller)===journal.controllerSha256)return candidate;
  throw Error('maintenance_rebind_journal_hash_unresolved');
}
function maintenanceEpochProved(proof,epoch){return proof?.version==='director-controller-maintenance-rebind/v1'
  && typeof epoch==='string'&&epoch.length>0&&proof.backgroundEpoch===epoch
  && typeof proof.previousBackgroundEpoch==='string'&&proof.previousBackgroundEpoch.length>0&&proof.previousBackgroundEpoch!==epoch;}

// Maintenance restores launch receipts after storage.session is cleared. Those
// receipts alone cannot resurrect a browser-session nonce. Rebind only the
// exact, proved idle controller in its still-hidden original native window.
export async function reconcileReloadedControllerIdentity({browser=chrome,connections,backgroundEpoch,now=Date.now}={}) {
  const held=reason=>({status:'held',reason});
  try {
    const read=async()=>{
      const identity=await browser.storage.local.get(PROFILE_KEY),profile=identity[PROFILE_KEY];
      if(!UUID.test(profile||''))return null;
      const key='directorDesktop:controlTab:v1:'+profile,journalKey='directorDesktop:controlReconnect:v1:'+profile;
      const keys=[PROFILE_KEY,key,journalKey,'bridgePendingExtensionReload'];
      const local=await browser.storage.local.get(keys);
      const control=local[key],invalidated=control?.reloadInvalidated;if(!invalidated)return null;
      const [session,tabs]=await Promise.all([browser.storage.session.get(null),browser.tabs.query({})]);
      if(!browser.storage.session.set||!backgroundEpoch)throw Error('maintenance_rebind_capability_missing');
      if(local[PROFILE_KEY]!==profile||control.status!=='opened'||!UUID.test(control.browserSession||'')
        ||control.launchToken!=='bridge-director-control-'+profile||!root(control.requestedUrl)
        ||!Number.isSafeInteger(control.tabId)||control.tabId<=0||!invalidated.operationId
        ||['tabId','launchToken','browserSession','serverUrl','requestedUrl'].some(k=>invalidated[k]!==control[k]))throw Error('maintenance_rebind_identity');
      const pending=local.bridgePendingExtensionReload,original=pending?.idleControllers?.[key];
      const anchored=pending?.operationId===invalidated.operationId&&original
        && ['tabId','launchToken','browserSession','serverUrl','requestedUrl'].every(k=>original[k]===control[k])
        && (pending.recoveryStatus==='awaiting_source_ready'||pending.recoveryStatus==='held'&&pending.recoveryReason==='source_ready_timeout');
      const evidence=invalidated.maintenanceRecovery||(anchored?{version:'director-controller-maintenance-idle/v1',
        profileId:pending.expectedRuntime?.profileId,previousBackgroundEpoch:pending.expectedRuntime?.backgroundEpoch,
        requestedAt:pending.requestedAt,contentEpoch:original.contentEpoch||null}:null);
      if(evidence?.version!=='director-controller-maintenance-idle/v1'||evidence.profileId!==profile
        ||!evidence.previousBackgroundEpoch||evidence.previousBackgroundEpoch===backgroundEpoch
        ||!Number.isSafeInteger(evidence.requestedAt)||evidence.requestedAt<=0||evidence.requestedAt>now())throw Error('maintenance_rebind_proof_missing');
      const journal=local[journalKey],preimage=await maintenancePreimage(control,journal);
      const sid=session[SESSION_KEY];if(sid!==undefined&&!UUID.test(sid||''))throw Error('maintenance_rebind_session_damaged');
      const owners=Object.entries(session).filter(([k,v])=>k.startsWith('chatgptBridgeLaunchedTab:')&&v?.launchToken===control.launchToken);
      const ownerKey='chatgptBridgeLaunchedTab:'+control.tabId;let ownerRestore=null;
      if(owners.length===0&&session[ownerKey]===undefined){
        const receipt=pending?.launchRecords?.[String(control.tabId)];
        const pendingOwners=Object.entries(pending?.launchRecords||{}).filter(([,v])=>v?.launchToken===control.launchToken);
        if(!anchored||!journal||pendingOwners.length!==1||pendingOwners[0][0]!==String(control.tabId)
          ||receipt?.launchToken!==control.launchToken||receipt.serverUrl!==control.serverUrl||receipt.requestedUrl!==control.requestedUrl
          ||!Number.isSafeInteger(receipt.createdAt)||receipt.createdAt<=0||receipt.createdAt>evidence.requestedAt
          ||Object.keys(receipt).some(k=>!['launchToken','serverUrl','requestedUrl','createdAt'].includes(k)))throw Error('maintenance_rebind_owner_receipt_unproved');
        const server=new URL(receipt.serverUrl);if(server.protocol!=='http:'||!['127.0.0.1','localhost'].includes(server.hostname)
          ||server.username||server.password||server.pathname!=='/'||server.search||server.hash)throw Error('maintenance_rebind_owner_receipt_unproved');
        ownerRestore={key:ownerKey,receipt,sha256:await digest(receipt),operationId:pending.operationId};
      }else if(owners.length!==1||owners[0][0]!==ownerKey
        ||owners[0][1].serverUrl!==control.serverUrl||owners[0][1].requestedUrl!==control.requestedUrl)throw Error('maintenance_rebind_owner');
      if(Object.entries(session).filter(([k])=>k.startsWith('chatgptBridgeV6:tab:')).some(([,v])=>!idle(v))
        ||[...connections.values()].some(s=>!s.closed&&s.activeRequest))throw Error('maintenance_rebind_execution_unresolved');
      if(!Array.isArray(tabs))throw Error('maintenance_rebind_tab_missing');
      const matches=tabs.filter(t=>t.id===control.tabId);
      if(matches.length===0&&ownerRestore){
        return {profile,key,journalKey,control,journal,session,evidence,preimage,ownerRestore,missingController:true,
          nativeInventory:nativeInventory(tabs),pendingSha256:await digest(pending)};
      }
      if(matches.length!==1)throw Error('maintenance_rebind_tab_missing');
      if(ownerRestore)nativeInventory(tabs);
      const tab=tabs.find(t=>t.id===control.tabId),window=await browser.windows.get(tab.windowId);
      if(!root(tab.url)||tab.incognito!==false||typeof tab.active!=='boolean'||window.id!==tab.windowId
        ||window.focused!==false||tab.active&&window.state!=='minimized')throw Error('maintenance_rebind_user_visible_or_changed');
      return {profile,key,journalKey,control,journal:journal||null,session,tab:{id:tab.id,windowId:tab.windowId,url:tab.url,active:tab.active,incognito:tab.incognito},
        window:{id:window.id,focused:window.focused,state:window.state},evidence,preimage,ownerRestore,
        navigationPendingUntil:anchored&&pending.recoveryStatus==='awaiting_source_ready'&&pending.tabIds?.includes(control.tabId)?pending.requestedAt+120000:0};
    };
    const first=await read();if(!first)return {status:'not_applicable'};
    if(!same(first,await read()))return held('maintenance_rebind_native_drift');
    if(first.missingController)return {status:'controller_missing_proved',proof:{version:'director-maintained-controller-missing/v1',
      profileId:first.profile,operationId:first.ownerRestore.operationId,tabId:first.control.tabId,backgroundEpoch,
      currentControlSha256:await digest(first.control),journalSha256:await digest(first.journal),preimageSha256:await digest(first.preimage.controller),
      nativeInventorySha256:await digest(first.nativeInventory),pendingSha256:first.pendingSha256,enumerations:2,observedAt:now(),foreground:false,generationCalled:false}};
    const oldSid=first.session[SESSION_KEY],sid=oldSid&&oldSid!==first.control.browserSession?oldSid:crypto.randomUUID().toLowerCase();
    if(!UUID.test(sid)||sid===first.control.browserSession)return held('maintenance_rebind_nonce');
    const sessionWrites={...(sid!==oldSid?{[SESSION_KEY]:sid}:{}),...(first.ownerRestore?{[first.ownerRestore.key]:first.ownerRestore.receipt}:{})};
    if(Object.keys(sessionWrites).length)await browser.storage.session.set(sessionWrites);
    const after=await read(),expected={...first,ownerRestore:null,session:{...first.session,...sessionWrites}};
    if(!after||!same(after,expected))return held('maintenance_rebind_session_write_unconfirmed');
    const {reloadInvalidated,retiredControllers:history,...retired}=first.control;
    const control={...retired,browserSession:sid,retiredControllers:[...(history||[]),{...retired,reloadInvalidated,retiredAt:now(),reason:'maintenance_session_rebound_same_hidden_controller'}]};
    const recovery={version:'director-controller-maintenance-rebind/v1',operationId:reloadInvalidated.operationId,
      profileId:first.profile,tabId:control.tabId,browserSession:sid,backgroundEpoch,
      previousBackgroundEpoch:first.evidence.previousBackgroundEpoch,beforeContentEpoch:first.evidence.contentEpoch||null,
      navigationPendingUntil:first.navigationPendingUntil,
      originalInvalidation:reloadInvalidated,originalControllerSha256:await digest(first.control),
      ...(first.ownerRestore?{launchReceiptRestoration:first.ownerRestore}:{}),
      journalPreimage:{source:first.preimage.source,controller:first.preimage.controller,sha256:await digest(first.preimage.controller)}};
    const journal={version:'director-owned-controller-maintenance-rebind/v1',status:'session_rebound',at:now(),
      controllerSha256:await digest(control),tabId:control.tabId,browserSession:sid,maintenanceRecovery:recovery,
      attempts:first.journal?.status==='connection_confirmed'?0:(first.journal?.attempts||0),
      incidentHistory:archiveJournal(first.journal),foreground:false,generationCalled:false};
    await browser.storage.local.set({[first.key]:control,[first.journalKey]:journal});
    const committed=await browser.storage.local.get([first.key,first.journalKey]);
    if(!same(committed[first.key],control)||!same(committed[first.journalKey],journal))return held('maintenance_rebind_commit_unconfirmed');
    return {status:'session_rebound',tabId:control.tabId,foreground:false,generationCalled:false};
  }catch(error){return held(String(error.message||'maintenance_rebind_read_failed'));}
}

export async function confirmOwnedControllerReady({browser=chrome,proof,now=Date.now}={}) {
  try {
    if(proof?.accepted!==true||!proof.helloMessageId||proof.ackMessageId!==proof.helloMessageId
      ||!Number.isSafeInteger(proof.acceptedSequence)||proof.acceptedSequence<=0||proof.launchRecordMatches!==true
      ||!UUID.test(proof.profileId||'')||proof.browserIncognito!==false||!proof.backgroundEpoch||!proof.contentEpoch
      ||!proof.serverInstanceId||proof.serverInstanceId!==proof.transportServerInstanceId||!/^[a-f0-9]{32}$/.test(proof.bundleId||''))return false;
    const key='directorDesktop:controlTab:v1:'+proof.profileId,journalKey='directorDesktop:controlReconnect:v1:'+proof.profileId;
    const local=await browser.storage.local.get([PROFILE_KEY,key,journalKey]),control=local[key],journal=local[journalKey];
    if(local[PROFILE_KEY]!==proof.profileId||control?.status!=='opened'||control.tabId!==proof.tabId
      ||control.launchToken!==proof.launchToken||proof.launchToken!=='bridge-director-control-'+proof.profileId
      ||control.serverUrl!==proof.serverUrl||!root(control.requestedUrl)||!journal
      ||!['bootstrap_completed','reload_requested','connection_confirmed','session_rebound'].includes(journal.status)
      ||journal.controllerSha256!==await digest(control))return false;
    if(journal.status==='connection_confirmed')return true;
    if(journal.version==='director-owned-controller-reconnect/v2'&&(!journal.beforeContentEpoch||journal.beforeContentEpoch===proof.contentEpoch)){
      if(journal.beforeContentEpoch||!maintenanceEpochProved(journal.maintenanceRecovery,proof.backgroundEpoch))return false;
    }
    if(journal.status==='session_rebound'&&!maintenanceEpochProved(journal.maintenanceRecovery,proof.backgroundEpoch))return false;
    const session=await browser.storage.session.get(null),record=session['chatgptBridgeLaunchedTab:'+proof.tabId];
    if(session[SESSION_KEY]!==control.browserSession||record?.launchToken!==control.launchToken
      ||record.serverUrl!==control.serverUrl||record.requestedUrl!==control.requestedUrl)return false;
    const tabs=await browser.tabs.query({}),tab=tabs.find(t=>t.id===proof.tabId);
    if(!tab||!root(tab.url)||tab.incognito!==false)return false;
    if(tab.active){const window=await browser.windows.get(tab.windowId);if(window.focused!==false||window.state!=='minimized')return false;}
    if(!same((await browser.storage.local.get([key,journalKey])),localSubset(local,[key,journalKey])))return false;
    await browser.storage.local.set({[journalKey]:{version:'director-owned-controller-confirmation/v1',status:'connection_confirmed',at:now(),
      controllerSha256:journal.controllerSha256,tabId:control.tabId,browserSession:control.browserSession,attempts:0,
      incidentHistory:archiveJournal(journal),readyProof:proof,foreground:false,generationCalled:false}});
    return true;
  }catch{return false;}
}
function localSubset(value,keys){return Object.fromEntries(keys.filter(k=>Object.hasOwn(value,k)).map(k=>[k,value[k]]));}

// windows.create can return a normal window on Windows even when minimized was
// requested. Reconcile the exact original creation; never allocate a second one.
export async function reconcileBootstrapWindow({browser=chrome,connections,now=Date.now}={}) {
  const held=reason=>({status:'held',reason});
  try {
    const sample=async()=>{
      const identity=await browser.storage.local.get(PROFILE_KEY),profile=identity[PROFILE_KEY];
      if(!UUID.test(profile||''))throw Error('bootstrap_window_profile');
      const key='directorDesktop:controlTab:v1:'+profile,journalKey='directorDesktop:controlReconnect:v1:'+profile;
      const local=await browser.storage.local.get([key,journalKey]);
      const journal=local[journalKey],control=local[key];
      if(journal?.version!=='director-owned-controller-bootstrap/v2'||!['reserved','outcome_unknown'].includes(journal.status))return null;
      if(!browser.windows?.update||!browser.windows?.get)throw Error('bootstrap_window_capability_missing');
      const [session,tabs]=await Promise.all([browser.storage.session.get(null),browser.tabs.query({})]);
      if(!same(journal.originalController,control)||journal.foreground!==false||journal.generationCalled!==false
        ||!Number.isSafeInteger(journal.at)||!UUID.test(journal.browserSession||'')
        ||session[SESSION_KEY]!==undefined&&session[SESSION_KEY]!==journal.browserSession)throw Error('bootstrap_window_intent_unproved');
      const launch='bridge-director-control-'+profile;
      const owned=Object.entries(session).filter(([k,v])=>k.startsWith('chatgptBridgeLaunchedTab:')&&v?.launchToken===launch);
      if(owned.length!==1)throw Error('bootstrap_window_owner_not_unique');
      const [ownerKey,record]=owned[0],id=Number(ownerKey.slice('chatgptBridgeLaunchedTab:'.length));
      if(!Number.isSafeInteger(id)||id<=0||id===control.tabId||ownerKey!=='chatgptBridgeLaunchedTab:'+id
        ||record.requestedUrl!=='https://chatgpt.com/'||record.serverUrl!==control.serverUrl
        ||!Number.isSafeInteger(record.createdAt)||record.createdAt<journal.at)throw Error('bootstrap_window_creation_unproved');
      const tab=tabs.find(t=>t.id===id);
      if(!tab||!root(tab.url)||tab.incognito!==false||!Number.isSafeInteger(tab.windowId)
        ||tabs.filter(t=>t.windowId===tab.windowId).length!==1)throw Error('bootstrap_window_not_exclusive');
      const window=await browser.windows.get(tab.windowId);
      if(window.id!==tab.windowId||window.focused!==false||window.incognito!==false)throw Error('bootstrap_window_foreground');
      const live=[...connections.values()].filter(s=>s.tabId===id&&!s.closed&&s.protocolReady&&s.ws?.readyState===1);
      if(live.length!==1||live[0].activeRequest)throw Error('bootstrap_window_connection_unproved');
      if(Object.entries(session).filter(([k])=>k.startsWith('chatgptBridgeV6:tab:')).some(([,v])=>!idle(v)))throw Error('bootstrap_window_execution_unresolved');
      return {profile,key,journalKey,journal,control,record,sessionId:session[SESSION_KEY]||null,tab:{id,windowId:tab.windowId,url:tab.url,incognito:tab.incognito},window:{id:window.id,focused:window.focused,state:window.state}};
    };
    const first=await sample();if(!first)return {status:'not_applicable'};
    if(!same(first,await sample()))return held('bootstrap_window_drift');
    await browser.windows.update(first.window.id,{state:'minimized',focused:false});
    const after=await sample();if(!after||after.window.state!=='minimized'||!same({...first,window:after.window},after))return held('bootstrap_window_hide_unconfirmed');
    // Reload may have cleared session storage. Allocate a fresh nonce; never
    // resurrect the original browser-session nonce from the durable journal.
    const sessionId=first.sessionId||crypto.randomUUID().toLowerCase();
    if(!first.sessionId){await browser.storage.session.set({[SESSION_KEY]:sessionId});if((await browser.storage.session.get(SESSION_KEY))[SESSION_KEY]!==sessionId)return held('bootstrap_window_session_unconfirmed');}
    const {retiredControllers:history,...retired}=first.control;
    const {reloadInvalidated:oldInvalidation,...current}=retired;
    const control={...current,status:'opened',tabId:first.tab.id,browserSession:sessionId,
      retiredControllers:[...(history||[]),{...retired,retiredAt:now(),reason:'reconciled_original_bootstrap_window'}]};
    await browser.tabs.update(first.tab.id,{autoDiscardable:false});
    const journal={...first.journal,status:'bootstrap_completed',controllerSha256:await digest(control),tabId:first.tab.id,windowId:first.window.id,attempts:1,
      windowReconciliation:{at:now(),originalIntent:first.journal,nativeStateBefore:first.window,nativeStateAfter:after.window,createdNewWindow:false}};
    await browser.storage.local.set({[first.key]:control,[first.journalKey]:journal});
    const readback=await browser.storage.local.get([first.key,first.journalKey]);
    if(!same(readback[first.key],control)||!same(readback[first.journalKey],journal))return held('bootstrap_window_commit_unconfirmed');
    return {status:'bootstrap_reconciled',tabId:first.tab.id,windowId:first.window.id,foreground:false,generationCalled:false};
  }catch(error){return held(String(error.message||'bootstrap_window_read_failed'));}
}

// A worker/extension reload may clear session ownership. Never relabel or reload
// the old page in that case. Allocate one NEW empty, minimized control window;
// preserve the old opened receipt in history, and hold uncertain creation.
export async function bootstrapOwnedController({browser=chrome,connections,now=Date.now,maintenanceMissing}={}) {
  const held=reason=>({status:'held',reason});
  try{
    if(!browser.windows?.create||!browser.tabs?.update||!browser.storage?.session?.set)return held('bootstrap_native_capability_missing');
    if(Array.from(connections.values()).some(s=>!s.closed&&(socketAlive(s)||s.activeRequest||s.protocolReady)))return held('bootstrap_transport_present');
    const read=async()=>{
      const identity=await browser.storage.local.get(PROFILE_KEY),profile=identity[PROFILE_KEY];if(!UUID.test(profile||''))throw Error('bootstrap_profile');
      const key='directorDesktop:controlTab:v1:'+profile,journalKey='directorDesktop:controlReconnect:v1:'+profile;
      const [local,session,tabs]=await Promise.all([browser.storage.local.get([PROFILE_KEY,key,journalKey,'bridgePendingExtensionReload']),browser.storage.session.get(null),browser.tabs.query({})]);
      const control=local[key],launch='bridge-director-control-'+profile;
      if(local[PROFILE_KEY]!==profile||control?.status!=='opened'||control.launchToken!==launch||!root(control.requestedUrl)
        ||!UUID.test(control.browserSession||'')||!Number.isSafeInteger(control.tabId)||control.tabId<=0)throw Error('bootstrap_original_not_opened');
      const u=new URL(control.serverUrl);if(u.protocol!=='http:'||!['127.0.0.1','localhost'].includes(u.hostname)||u.username||u.password||u.pathname!=='/'||u.search||u.hash)throw Error('bootstrap_server');
      if(!Array.isArray(tabs)||tabs.some(t=>!Number.isSafeInteger(t.id)||t.id<=0||typeof t.url!=='string'||typeof t.incognito!=='boolean')||new Set(tabs.map(t=>t.id)).size!==tabs.length)throw Error('bootstrap_native_inventory');
      if(Object.entries(session).some(([k,v])=>k.startsWith('chatgptBridgeLaunchedTab:')&&v?.launchToken===launch))throw Error('bootstrap_owner_exists');
      if(Object.entries(session).filter(([k])=>k.startsWith('chatgptBridgeV6:tab:')).some(([,v])=>!idle(v)))throw Error('bootstrap_execution_unresolved');
      const sid=session[SESSION_KEY];if(sid!==undefined&&!UUID.test(sid||''))throw Error('bootstrap_session_damaged');
      let orderingRepair=null;const prior=local[journalKey];
      if(maintenanceMissing){
        const proof=maintenanceMissing,pending=local.bridgePendingExtensionReload,invalidated=control.reloadInvalidated;
        const original=pending?.idleControllers?.[key],receipt=pending?.launchRecords?.[String(control.tabId)];
        if(proof.version!=='director-maintained-controller-missing/v1'||proof.enumerations!==2||proof.profileId!==profile||proof.tabId!==control.tabId
          ||!proof.backgroundEpoch||!Number.isSafeInteger(proof.observedAt)||proof.observedAt<=0||proof.observedAt>now()||now()-proof.observedAt>10000||proof.foreground!==false||proof.generationCalled!==false
          ||!prior||!invalidated||pending?.operationId!==proof.operationId||invalidated.operationId!==proof.operationId
          ||pending.recoveryStatus!=='held'||pending.recoveryReason!=='source_ready_timeout'
          ||pending.expectedRuntime?.profileId!==profile||!pending.expectedRuntime.backgroundEpoch||pending.expectedRuntime.backgroundEpoch===proof.backgroundEpoch
          ||!original||['tabId','launchToken','browserSession','serverUrl','requestedUrl'].some(k=>original[k]!==control[k])
          ||receipt?.launchToken!==launch||receipt.serverUrl!==control.serverUrl||receipt.requestedUrl!==control.requestedUrl
          ||!Number.isSafeInteger(receipt.createdAt)||receipt.createdAt<=0||receipt.createdAt>pending.requestedAt
          ||Object.values(pending.launchRecords||{}).filter(r=>r?.launchToken===launch).length!==1
          ||proof.currentControlSha256!==await digest(control)||proof.journalSha256!==await digest(prior)
          ||proof.pendingSha256!==await digest(pending)||tabs.some(t=>t.id===control.tabId)
          ||proof.nativeInventorySha256!==await digest(nativeInventory(tabs))
          ||proof.preimageSha256!==await digest((await maintenancePreimage(control,prior)).controller))throw Error('bootstrap_missing_controller_proof_changed');
      }else if(prior?.status==='connection_confirmed'){
        if(prior.controllerSha256!==await digest(control)||prior.tabId!==control.tabId||prior.browserSession!==control.browserSession)throw Error('bootstrap_previous_confirmation_changed');
      }else if(prior){
        const order=['version','status','at','originalController','browserSession','foreground','generationCalled'];
        const expected={version:'director-owned-controller-bootstrap/v1',status:'reserved',at:prior.at,originalController:control,browserSession:prior.browserSession,foreground:false,generationCalled:false};
        // The pinned v1 implementation compared this exact insertion order to
        // Chrome's sorted dictionary before its first native create call. Only
        // that zero-create rejection is repairable; all other reservations hold.
        if(!same(prior,expected)||!Number.isSafeInteger(prior.at)||!UUID.test(prior.browserSession||'')
          ||sid!==undefined&&sid!==prior.browserSession||Object.keys(prior).length!==order.length
          ||Object.keys(prior).join('|')===order.join('|'))throw Error('bootstrap_existing_reservation');
        orderingRepair={version:'director-controller-zero-create-ordering-repair/v1',previousIntent:prior,
          expectedKeyOrder:order,observedReadbackKeyOrder:Object.keys(prior),reason:'pinned_v1_readback_guard_rejected_before_create',previousContract:'director-owned-controller-bootstrap/v1'};
      }
      return {profile,key,journalKey,control,session,tabs:tabs.map(t=>({id:t.id,url:t.url,incognito:t.incognito})),sessionId:sid||null,orderingRepair,prior:prior||null};
    };
    const first=await read(),second=await read();if(!same(first,second))return held('bootstrap_native_drift');
    const sid=first.sessionId||crypto.randomUUID().toLowerCase();if(!UUID.test(sid))return held('bootstrap_session_identity');
    const intent={version:'director-owned-controller-bootstrap/v2',status:'reserved',at:now(),originalController:first.control,browserSession:sid,foreground:false,generationCalled:false,zeroCreateOrderingRepair:first.orderingRepair,incidentHistory:archiveJournal(first.prior),
      ...(maintenanceMissing?{maintenanceMissingProof:maintenanceMissing}:{})};
    await browser.storage.local.set({[first.journalKey]:intent});
    if(!same((await browser.storage.local.get(first.journalKey))[first.journalKey],intent))return held('bootstrap_intent_not_persisted');
    const actual=(await browser.storage.local.get(first.key))[first.key];if(!same(actual,first.control))return held('bootstrap_controller_changed');
    if(first.sessionId===null){await browser.storage.session.set({[SESSION_KEY]:sid});if((await browser.storage.session.get(SESSION_KEY))[SESSION_KEY]!==sid)return held('bootstrap_session_not_persisted');}
    const url=new URL('https://chatgpt.com/');url.hash=new URLSearchParams({'chatgpt-bridge-launch':first.control.launchToken,'chatgpt-bridge-server':first.control.serverUrl}).toString();
    let created;
    try{created=await browser.windows.create({url:url.toString(),focused:false,state:'minimized',type:'normal',incognito:false});}
    catch{await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('bootstrap_create_unknown');}
    const pages=created?.tabs;
    if(!Number.isSafeInteger(created?.id)||created.focused!==false||created.incognito!==false||!Array.isArray(pages)||pages.length!==1||!Number.isSafeInteger(pages[0]?.id)||pages[0].id<=0||first.tabs.some(t=>t.id===pages[0].id)){await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('bootstrap_receipt_unknown');}
    const tabId=pages[0].id;
    await browser.tabs.update(tabId,{autoDiscardable:false});
    await browser.storage.session.set({['chatgptBridgeLaunchedTab:'+tabId]:{launchToken:first.control.launchToken,requestedUrl:'https://chatgpt.com/',serverUrl:first.control.serverUrl,createdAt:now()}});
    // A create acknowledgement is not a visibility acknowledgement. Verify the
    // exact exclusive window again after an explicit, non-focusing update.
    if(created.state!=='minimized') {
      if(!browser.windows.update){await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('bootstrap_window_update_missing');}
      await browser.windows.update(created.id,{state:'minimized',focused:false});
    }
    const actualWindow=await browser.windows.get(created.id);
    if(actualWindow.id!==created.id||actualWindow.focused!==false||actualWindow.state!=='minimized'){
      await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('bootstrap_window_hide_unconfirmed');
    }
    const {retiredControllers:history,...retired}=first.control;
    const {reloadInvalidated:oldInvalidation,...current}=retired;
    const control={...current,tabId,browserSession:sid,status:'opened',retiredControllers:[...(history||[]),{...retired,retiredAt:now(),reason:'session_ownership_lost_old_page_untouched'}]};
    await browser.storage.local.set({[first.key]:control,[first.journalKey]:{...intent,status:'bootstrap_completed',controllerSha256:await digest(control),tabId,windowId:created.id,attempts:1}});
    return {status:'bootstrap_completed',tabId,windowId:created.id,foreground:false,generationCalled:false};
  }catch(error){return held(String(error.message||'bootstrap_read_failed'));}
}

// Runs in the extension, independently of content ports or server clients. No
// tab creation, extension reload, model submission or user-page navigation.
export async function recoverOwnedController({browser=chrome,connections,backgroundEpoch,now=Date.now}={}) {
  const held=reason=>({status:'held',reason});
  // An interrupted replacement must reconcile its original creation before
  // maintenance preimage checks see the new reserved/unknown bootstrap journal.
  const reconciliation=await reconcileBootstrapWindow({browser,connections,now});
  if(reconciliation.status==='bootstrap_reconciled'||reconciliation.status==='held')return reconciliation;
  const maintenance=await reconcileReloadedControllerIdentity({browser,connections,backgroundEpoch,now});
  if(maintenance.status==='held')return maintenance;
  if(maintenance.status==='controller_missing_proved')return bootstrapOwnedController({browser,connections,now,maintenanceMissing:maintenance.proof});
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
    if(states.some(socketAlive))return {connected:true,readyProof:states.find(s=>s.directorReadyProof)?.directorReadyProof};
    if(states.some(c=>c.activeRequest || c.protocolReady))throw Error('transport_outcome_unresolved');
    const runtime=session['chatgptBridgeV6:tab:'+control.tabId],journal=local[journalKey];
    // Absence after a proved maintenance reset is not a fabricated empty
    // runtime. Keep it absent and carry the old/new worker proof to hello ACK.
    const resetIdle=runtime===undefined&&['session_rebound','reserved','reload_requested','outcome_unknown'].includes(journal?.status)
      &&journal.controllerSha256===await digest(control)&&maintenanceEpochProved(journal.maintenanceRecovery,backgroundEpoch)
      &&Object.entries(session).filter(([k])=>k.startsWith('chatgptBridgeV6:tab:')).every(([,v])=>idle(v));
    if(!idle(runtime)&&!resetIdle)throw Error('controller_execution_unresolved');
    return {profile,key,journalKey,control,record,runtime,tab:{id:tab.id,windowId:tab.windowId,url:tab.url,active:tab.active,incognito:tab.incognito,discarded:tab.discarded},window:{id:window.id,state:window.state,focused:window.focused},journal:local[journalKey]||null};
  };
  try {
    const first=await sample();if(first.connected){if(first.readyProof)await confirmOwnedControllerReady({browser,proof:first.readyProof,now});return {status:'connected'};}
    const old=first.journal;
    // A reload response is not proof that content reconnected. Never replay an
    // interrupted/failed reservation; wait for the original page's handshake.
    if(old && old.controllerSha256!==await digest(first.control))return held('journal_identity_changed');
    if(old?.status==='reserved'||old?.status==='outcome_unknown')return held('original_reload_unconfirmed');
    if(old?.status==='session_rebound'&&now()<Number(old.maintenanceRecovery?.navigationPendingUntil||0))return held('maintenance_navigation_pending');
    if(old && old.status!=='session_rebound' && now()-old.at<120000)return held('reconnect_cooldown');
    const attempts=old?.status==='connection_confirmed'?0:(old?.attempts||0);
    if(attempts>=2)return held('reconnect_budget_exhausted');
    const second=await sample();if(second.connected)return {status:'connected'};
    if(!same(first,second))return held('native_state_changed');
    const intent={version:'director-owned-controller-reconnect/v2',incidentId:old?.status==='reload_requested'?old.incidentId:crypto.randomUUID().toLowerCase(),
      beforeContentEpoch:[...connections.values()].find(s=>s.tabId===first.tab.id&&!s.closed)?.contentEpoch||first.runtime?.contentEpoch||old?.maintenanceRecovery?.beforeContentEpoch||null,
      ...(old?.maintenanceRecovery?{maintenanceRecovery:old.maintenanceRecovery}:{}),
      controllerSha256:await digest(first.control),tabId:first.tab.id,
      browserSession:first.control.browserSession,launchToken:first.control.launchToken,status:'reserved',attempts:attempts+1,at:now(),foreground:false,generationCalled:false,incidentHistory:archiveJournal(old)};
    await browser.storage.local.set({[first.journalKey]:intent});
    const confirmed=(await browser.storage.local.get(first.journalKey))[first.journalKey];
    if(!same(confirmed,intent))return held('intent_not_persisted');
    const after=await sample();const expected={...first,journal:intent};
    if(after.connected||!same(after,expected))return held('pre_reload_state_changed');
    try{await browser.tabs.reload(first.tab.id);}
    catch{await browser.storage.local.set({[first.journalKey]:{...intent,status:'outcome_unknown'}});return held('reload_outcome_unknown');}
    await browser.storage.local.set({[first.journalKey]:{...intent,status:'reload_requested'}});
    return {status:'reload_requested',tabId:first.tab.id,foreground:false,generationCalled:false};
  }catch(error){const reason=String(error.message||'native_read_failed');
    if(reason==='controller_identity'||reason==='session_ownership')return bootstrapOwnedController({browser,connections,now});
    return held(reason);}
}
async function digest(value){const bytes=new TextEncoder().encode(JSON.stringify(canonical(value)));return [...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(b=>b.toString(16).padStart(2,'0')).join('');}

export function createConnectionWatchdog({alarms,connections,openConnection,periodInMinutes=1,recoverController}={}) {
  if(!connections||typeof openConnection!=='function')throw new TypeError('Connection watchdog requires connections and openConnection');
  let running=null;
  async function run(){
    if(running)return running;
    running=(async()=>{const retries=[];for(const state of connections.values()){
      if(!state||state.closed||socketAlive(state))continue;clearTimeout(state.reconnectTimer);state.reconnectTimer=null;retries.push(Promise.resolve(openConnection(state)));
    }await Promise.allSettled(retries);
    // Creation reconciliation must run even when its new page has connected;
    // connected does not prove its native window became hidden.
    if(typeof recoverController==='function')await recoverController();
    return retries.length;})();try{return await running;}finally{running=null;}
  }
  let arming=null;
  async function arm(){
    if(arming)return arming;
    arming=(async()=>{if(!alarms?.create)return false;
      // Creating an existing Chrome alarm replaces its schedule. Re-arming on
      // every worker wake can otherwise postpone recovery indefinitely.
      if(alarms.get&&await alarms.get(CONNECTION_WATCHDOG_ALARM))return true;
      await alarms.create(CONNECTION_WATCHDOG_ALARM,{periodInMinutes:Math.max(1,Number(periodInMinutes)||1)});return true;
    })();try{return await arming;}finally{arming=null;}
  }
  return Object.freeze({arm,run,handlesAlarm:name=>String(name||'')===CONNECTION_WATCHDOG_ALARM});
}
