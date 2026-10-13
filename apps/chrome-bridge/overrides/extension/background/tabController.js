function createTab(options = {}) {
  return new Promise((resolve, reject) => {
    try {
      chrome.tabs.create(options, (tab) => {
        if (chrome.runtime.lastError) reject(new Error(chrome.runtime.lastError.message));
        else resolve(tab || null);
      });
    } catch (error) {
      reject(error);
    }
  });
}

function updateTab(tabId, options = {}) {
  return new Promise((resolve, reject) => {
    try {
      chrome.tabs.update(tabId, options, (tab) => {
        if (chrome.runtime.lastError) reject(new Error(chrome.runtime.lastError.message));
        else resolve(tab || null);
      });
    } catch (error) {
      reject(error);
    }
  });
}

function removeTab(tabId) {
  return new Promise((resolve, reject) => {
    try {
      chrome.tabs.remove(tabId, () => {
        if (chrome.runtime.lastError) reject(new Error(chrome.runtime.lastError.message));
        else resolve(true);
      });
    } catch (error) {
      reject(error);
    }
  });
}

function safeChatUrl(value = '') {
  const parsed = new URL(String(value || 'https://chatgpt.com/'));
  if (!['https://chatgpt.com', 'https://chat.openai.com'].includes(parsed.origin.toLowerCase()) || parsed.username || parsed.password) {
    throw new Error(`Refusing to open non-ChatGPT URL: ${parsed.toString()}`);
  }
  return parsed.toString();
}

const CONTROLLER_LAUNCH_RE = /^bridge-director-control-([a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12})$/;
const CONTROLLER_STATE_PREFIX = 'directorDesktop:controlTab:v1:';
const CONTROLLER_BROWSER_SESSION_KEY = 'directorDesktop:controllerBrowserSession:v1';
const CONTROLLER_SESSION_UUID_RE = /^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const LAUNCHED_TAB_PREFIX = 'chatgptBridgeLaunchedTab:';

function controllerTabState(tab = {}) {
  return Object.fromEntries(['active', 'autoDiscardable', 'discarded']
    .filter((key) => typeof tab[key] === 'boolean').map((key) => [key, tab[key]]));
}

function controllerRootUrl(value = '') {
  try { const url = new URL(value); return url.origin === 'https://chatgpt.com' && url.pathname === '/' && !url.search && !url.hash && !url.username && !url.password; }
  catch { return false; }
}

export function createTabController({
  connections,
  safeBridgeServerUrl,
  rememberLaunchedTab,
  readLaunchedTab,
  forgetLaunchedTab,
  isStableLaunchToken,
  readInventoryIdentity,
  readExecutionInventory,
} = {}) {
  if (!connections || typeof connections.get !== 'function') throw new TypeError('Tab controller requires connections');
  if (typeof safeBridgeServerUrl !== 'function') throw new TypeError('Tab controller requires safeBridgeServerUrl');
  if (typeof rememberLaunchedTab !== 'function' || typeof readLaunchedTab !== 'function' || typeof forgetLaunchedTab !== 'function') {
    throw new TypeError('Tab controller requires launched-tab storage adapters');
  }
  if (typeof isStableLaunchToken !== 'function') throw new TypeError('Tab controller requires launch-token validation');
  let controllerQueue = Promise.resolve();

  async function provePreviousControllerRecovery(port, prior, profileId, stateKey, bridgeServerUrl) {
    if (typeof readInventoryIdentity !== 'function' || !Number.isSafeInteger(prior?.tabId) || prior.tabId <= 0
      || !CONTROLLER_SESSION_UUID_RE.test(prior.browserSession || '')) return null;
    const connection = connections.get(port);
    if (!connection || connection.closed || connection.protocolReady !== true || connection.ws?.readyState !== 1
      || connection.activeRequest || safeBridgeServerUrl(connection.serverUrl) !== bridgeServerUrl) return null;
    const before = await readInventoryIdentity(port);
    if (before.profileId !== profileId || !/^[a-f0-9]{32}$/.test(before.bundleId || '') || !before.backgroundEpoch
      || !before.serverInstanceId || !before.contentEpoch || before.sourceTabId !== port?.sender?.tab?.id
      || before.sourceClientId !== connection.clientId || port?.sender?.tab?.incognito !== false) return null;
    const read = async () => {
      const [tabs, records, local] = await Promise.all([chrome.tabs.query({}), chrome.storage.session.get(null),
        chrome.storage.local.get(['directorDesktop:browserProfileId:v1', stateKey])]);
      if (!Array.isArray(tabs) || !records || typeof records !== 'object' || Array.isArray(records)
        || tabs.some(tab => !Number.isSafeInteger(tab.id) || tab.id <= 0 || typeof tab.url !== 'string'
          || typeof tab.incognito !== 'boolean' || typeof tab.active !== 'boolean')
        || new Set(tabs.map(tab => tab.id)).size !== tabs.length) throw new Error('Previous controller complete inventory unavailable');
      const source = tabs.find(tab => tab.id === before.sourceTabId);
      if (!source || !controllerRootUrl(source.url) || source.incognito || source.active
        || local['directorDesktop:browserProfileId:v1'] !== profileId || JSON.stringify(local[stateKey]) !== JSON.stringify(prior)) return null;
      const launched = Object.entries(records).filter(([key]) => key.startsWith(LAUNCHED_TAB_PREFIX)).sort(([a], [b]) => a.localeCompare(b));
      // Any ambiguous ownership is a hold, including orphaned/duplicate tokens.
      for (const [key, record] of launched) {
        const raw = key.slice(LAUNCHED_TAB_PREFIX.length), id = Number(raw);
        if (!Number.isSafeInteger(id) || id <= 0 || String(id) !== raw || !record || typeof record !== 'object'
          || !isStableLaunchToken(record.launchToken)) return null;
        if (record.launchToken === prior.launchToken && (id !== prior.tabId || record.serverUrl !== bridgeServerUrl
          || record.requestedUrl !== prior.requestedUrl)) return null;
      }
      if (launched.filter(([, record]) => record.launchToken === prior.launchToken).length > 1) return null;
      // A live root with missing session ownership could be a reused user page.
      // Persistent tabId alone never authorizes adopting or navigating it.
      const existing = tabs.find(tab => tab.id === prior.tabId);
      let targetRuntime = null;
      if (existing) {
        const record = launched.find(([key]) => key === LAUNCHED_TAB_PREFIX + prior.tabId)?.[1];
        const target = Array.from(connections.entries()).find(([, state]) => state.tabId === prior.tabId
          && !state.closed && state.protocolReady && state.ws?.readyState === 1 && !state.activeRequest
          && safeBridgeServerUrl(state.serverUrl) === bridgeServerUrl);
        if (!record || record.launchToken !== prior.launchToken || !controllerRootUrl(existing.url)
          || existing.active || existing.incognito || !target) return null;
        targetRuntime = await readInventoryIdentity(target[0]);
        if (targetRuntime.sourceTabId !== prior.tabId || targetRuntime.sourceClientId !== target[1].clientId
          || !targetRuntime.contentEpoch || !['profileId', 'bundleId', 'backgroundEpoch', 'serverInstanceId'].every(key => targetRuntime[key] === before[key])) return null;
      }
      return { tabs: tabs.map(tab => ({ id: tab.id, url: tab.url, incognito: tab.incognito, active: tab.active,
        discarded: tab.discarded === true })).sort((a, b) => a.id - b.id), launched,
        browserSession: records[CONTROLLER_BROWSER_SESSION_KEY], targetRuntime };
    };
    const first = await read(), second = await read(), after = await readInventoryIdentity(port);
    if (!first || !second || JSON.stringify(first) !== JSON.stringify(second) || JSON.stringify(before) !== JSON.stringify(after)) return null;
    const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(JSON.stringify(second))))]
      .map(byte => byte.toString(16).padStart(2, '0')).join('');
    return { reason: second.targetRuntime ? 'previous_session_live_owned_rebound' : 'previous_session_closed_inventory_verified',
      matched: Boolean(second.targetRuntime), oldTabId: prior.tabId, oldBrowserSession: prior.browserSession,
      browserSession: second.browserSession, runtime: before, inventorySha256: hash, observedAt: Date.now(),
      enumerations: 2, ...(second.targetRuntime ? { targetRuntime: second.targetRuntime } : { absenceScope: 'current_browser_inventory_only' }) };
  }

  async function lookupOwnedLaunch(port,options) {
    const version='director-owned-launch-inventory/v1';
    const expected=options.expectedRuntime||{},requestId=String(options.lookupRequestId||''),launchToken=`bridge-director-desktop-${requestId}`;
    const unknown=reason=>({version,status:'unknown',reason,requestId,lookupRequestId:requestId,launchToken,complete:false});
    if(!CONTROLLER_SESSION_UUID_RE.test(requestId))return unknown('invalid_lookup_request_identity');
    if(options.launchToken!==launchToken)return unknown('lookup_launch_token_mismatch');
    if(typeof readInventoryIdentity!=='function')return unknown('inventory_runtime_reader_unavailable');
    if(typeof readExecutionInventory!=='function')return unknown('inventory_execution_reader_unavailable');
    if(typeof globalThis.crypto?.subtle?.digest!=='function')return unknown('inventory_webcrypto_digest_unavailable');
    if(typeof chrome.storage?.session?.get!=='function')return unknown('inventory_session_reader_unavailable');
    if(typeof chrome.tabs?.query!=='function')return unknown('inventory_tab_enumerator_unavailable');
    if(typeof globalThis.crypto?.randomUUID!=='function')return unknown('inventory_generation_uuid_unavailable');
    if(typeof globalThis.TextEncoder!=='function')return unknown('inventory_text_encoder_unavailable');
    try {
      const before=await readInventoryIdentity(port);
      if(!CONTROLLER_SESSION_UUID_RE.test(before.profileId||'') || !/^[a-f0-9]{32}$/.test(before.bundleId||'')
        || !before.backgroundEpoch || !before.serverInstanceId || !before.contentEpoch
        || !['profileId','bundleId','backgroundEpoch','serverInstanceId','sourceClientId','sourceTabId','contentEpoch'].every(key=>before[key]===expected[key]))return unknown('runtime_mismatch');
      const serverUrl=safeBridgeServerUrl(connections.get(port)?.serverUrl||'');
      if(!serverUrl || port?.sender?.tab?.incognito!==false)return unknown('source_transport_unproved');
      const read=async()=>{
        const records=await chrome.storage.session.get(null),tabs=await chrome.tabs.query({});
        if(!records || typeof records!=='object' || Array.isArray(records) || !Array.isArray(tabs)
          || tabs.some(tab=>!Number.isInteger(tab.id)||tab.id<0||typeof tab.url!=='string'||typeof tab.incognito!=='boolean')
          || new Set(tabs.map(tab=>tab.id)).size!==tabs.length)throw new Error('incomplete inventory');
        const launched=Object.entries(records).filter(([key])=>key.startsWith(LAUNCHED_TAB_PREFIX)).map(([key,record])=>({key,record})).sort((a,b)=>a.key.localeCompare(b.key));
        if(launched.some(({key,record})=>key!==LAUNCHED_TAB_PREFIX+Number(key.slice(LAUNCHED_TAB_PREFIX.length))
          || !Number.isInteger(Number(key.slice(LAUNCHED_TAB_PREFIX.length))) || !record || typeof record!=='object'
          || !isStableLaunchToken(record.launchToken) || typeof record.requestedUrl!=='string' || typeof record.serverUrl!=='string'
          || !Number.isSafeInteger(record.createdAt)))throw new Error('malformed launched inventory');
        // Compare full tab identity plus all launched records before deriving
        // absence. No connected-client list stands in for Chrome's inventory.
        const tabFacts=tabs.map(tab=>({id:tab.id,url:tab.url,incognito:tab.incognito,active:tab.active===true,discarded:tab.discarded===true,
          windowId:Number.isInteger(tab.windowId)?tab.windowId:null})).sort((a,b)=>a.id-b.id);
        const source=tabFacts.find(tab=>tab.id===before.sourceTabId);
        let sourceWindow=null,sourceWindowError='';
        if(source?.active){
          if(typeof chrome.windows?.get!=='function')sourceWindowError='source_native_window_reader_unavailable';
          else if(!Number.isInteger(source.windowId)||source.windowId<=0)sourceWindowError='source_native_window_identity_missing';
          else {
            const window=await chrome.windows.get(source.windowId).catch(()=>null);
            if(!window || window.id!==source.windowId || typeof window.focused!=='boolean'
              || !['normal','minimized','maximized','fullscreen'].includes(window.state))sourceWindowError='source_native_window_unproved';
            else sourceWindow={id:window.id,focused:window.focused,state:window.state};
          }
        }
        const execution=await readExecutionInventory(port,{inventoryCommandId:options.inventoryCommandId,rpcRequestId:options.requestId});
        return {launched,tabs:tabFacts,sourceWindow,sourceWindowError,execution};
      };
      const first=await read(),second=await read(),after=await readInventoryIdentity(port);
      if(first.sourceWindowError||second.sourceWindowError)return unknown(first.sourceWindowError||second.sourceWindowError);
      if(JSON.stringify(before)!==JSON.stringify(after) || JSON.stringify(first)!==JSON.stringify(second))return unknown('inventory_changed_during_read');
      const source=second.tabs.find(tab=>tab.id===before.sourceTabId),sourceRecord=second.launched.find(item=>item.key===LAUNCHED_TAB_PREFIX+before.sourceTabId)?.record;
      if(!source || source.incognito || !controllerRootUrl(source.url) || !sourceRecord
        || sourceRecord.serverUrl!==serverUrl || !(sourceRecord.launchToken===`bridge-director-control-${before.profileId}`
          || /^bridge-director-desktop-[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i.test(sourceRecord.launchToken||'')))return unknown('source_owned_idle_root_unproved');
      const execution=second.execution, own=execution?.ownCommand, queue=execution?.queue;
      const inventoryCommandId=String(options.inventoryCommandId||''),rpcRequestId=String(options.requestId||'');
      const ownAcceptedMessageIds=execution?.ownAcceptedMessageIds;
      const ownAck=item=>item && item.label?.startsWith('server:transport.ack:')
        && typeof item.meta?.ackMessageId==='string' && ownAcceptedMessageIds.includes(item.meta.ackMessageId);
      if(!inventoryCommandId || !/^ext-[a-z0-9-]{8,127}$/i.test(rpcRequestId)
        || execution?.sourceTabId!==source.id || execution.lease!==null
        || own?.commandId!==inventoryCommandId || own.commandType!=='browser.tab.identify' || own.scope!=='standalone'
        || own.mode!=='result' || own.operation!=='control' || own.status!=='dispatched' || own.requestId!==''
        || !Array.isArray(execution.otherUnresolvedCommands) || execution.otherUnresolvedCommands.length
        || !Array.isArray(execution.otherPendingOutbox) || execution.otherPendingOutbox.length
        || !Array.isArray(ownAcceptedMessageIds) || ownAcceptedMessageIds.some(id=>typeof id!=='string'||!id)
        || queue?.version!=='tab-operation-snapshot/v1' || queue.tabId!==source.id
        || !Number.isSafeInteger(queue.running?.sequence) || queue.running.sequence<=0
        || queue.running.label!=='content:bridge.tab.open' || queue.running.meta?.type!=='bridge.tab.open'
        || queue.running.meta.requestId!==rpcRequestId || !Array.isArray(queue.queued)
        || queue.queued.some(item=>!ownAck(item)))return unknown('source_native_execution_busy');
      const sourceExecutionProof={version:'director-source-execution-idle/v1',sourceTabId:source.id,inventoryCommandId,rpcRequestId,
        nativeSamples:2,observedAt:Date.now(),lease:null,ownCommand:own,otherUnresolvedCommands:[],otherPendingOutbox:[],ownAcceptedMessageIds,queue};
      let sourceBackgroundProof;
      if(source.active){
        const document=options.sourceDocument;
        if(!second.sourceWindow || second.sourceWindow.focused!==false)return unknown('source_native_window_foreground');
        if(!document || document.visibilityState!=='hidden' || document.focused!==false
          || !Number.isSafeInteger(document.sampledAt) || Date.now()-document.sampledAt<0
          || Date.now()-document.sampledAt>30000)return unknown('source_current_document_background_unproved');
        sourceBackgroundProof={version:'director-source-background-proof/v1',sourceTabId:source.id,sourceWindowId:source.windowId,nativeSamples:2,
          native:{tabActive:true,windowFocused:false,windowState:second.sourceWindow.state},
          content:{visibilityState:'hidden',focused:false,contentEpoch:before.contentEpoch,sampledAt:document.sampledAt}};
      }
      const matches=second.launched.filter(item=>item.record?.launchToken===launchToken);
      if(matches.length>1)return unknown('duplicate_launch_token');
      let match,status='confirmed_absent';
      if(matches.length===1){
        const entry=matches[0],tabId=Number(entry.key.slice(LAUNCHED_TAB_PREFIX.length)),tab=second.tabs.find(item=>item.id===tabId);
        if(!Number.isInteger(tabId)||entry.key!==LAUNCHED_TAB_PREFIX+tabId || !tab)return unknown('orphaned_launch_record');
        if(entry.record.serverUrl!==serverUrl || !controllerRootUrl(entry.record.requestedUrl) || !Number.isSafeInteger(entry.record.createdAt)
          || tab.incognito || !/^https:\/\/chatgpt\.com\/(?:$|c\/)/.test(tab.url))return unknown('launch_record_or_tab_mismatch');
        match={tabId,url:tab.url,launchToken,createdAt:entry.record.createdAt,serverUrl,
          connected:Array.from(connections.values()).some(state=>state.tabId===tabId && !state.closed && state.protocolReady && state.ws?.readyState===1)};
        status='matched';
      }
      const digest=async value=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(value)))].map(byte=>byte.toString(16).padStart(2,'0')).join('');
      // Audit unrelated tabs by URL digest; never disclose their URLs/titles.
      const inventory={tabs:await Promise.all(second.tabs.map(async tab=>({id:tab.id,urlSha256:await digest(tab.url),incognito:tab.incognito,active:tab.active,discarded:tab.discarded,windowId:tab.windowId}))),
        launchRecords:second.launched.map(item=>({key:item.key,recordSha256:null})),sourceTabId:before.sourceTabId,
        sourceExecutionProof,...(sourceBackgroundProof?{sourceBackgroundProof}:{})};
      for(let index=0;index<second.launched.length;index++)inventory.launchRecords[index].recordSha256=await digest(JSON.stringify(second.launched[index].record));
      if(JSON.stringify(before)!==JSON.stringify(await readInventoryIdentity(port)))return unknown('runtime_changed_during_digest');
      const inventoryCanonicalJson=JSON.stringify(inventory);
      return {version,status,reason:status==='matched'?'unique_record_and_live_tab':'complete_inventory_no_matching_launch',requestId,lookupRequestId:requestId,inventoryCommandId,rpcRequestId,launchToken,complete:true,
        runtime:before,inventoryGeneration:crypto.randomUUID(),inventorySha256:await digest(inventoryCanonicalJson),inventoryCanonicalJson,inventory,observedAt:Date.now(),
        absenceScope:'current_browser_inventory_only',...(match?{match}:{})};
    }catch{return unknown('inventory_enumeration_unavailable');}
  }

  async function openControllerTab(port, options, launchToken, requestedUrl, bridgeServerUrl) {
    const profileId = CONTROLLER_LAUNCH_RE.exec(launchToken)?.[1];
    if (!profileId || options.active !== false || !controllerRootUrl(requestedUrl) || !bridgeServerUrl
      || safeBridgeServerUrl(connections.get(port)?.serverUrl || '') !== bridgeServerUrl || port?.sender?.tab?.incognito === true) {
      throw new Error('Controller tab requires an explicit background root page in the original bridge profile');
    }
    const local = chrome.storage?.local;
    const session = chrome.storage?.session;
    if (!local?.get || !local?.set || !session?.get || !session?.set || !chrome.tabs?.query) throw new Error('Controller tab durable identity storage is unavailable');
    const stored = await local.get(['directorDesktop:browserProfileId:v1', CONTROLLER_STATE_PREFIX + profileId]);
    if (stored['directorDesktop:browserProfileId:v1'] !== profileId) throw new Error('Controller token does not belong to this browser profile');
    const stateKey = CONTROLLER_STATE_PREFIX + profileId;
    const prior = stored[stateKey];
    let retiredControllers = Array.isArray(prior?.retiredControllers) ? prior.retiredControllers : [];
    const saveState = async (state) => await local.set({ [stateKey]: { ...state, ...(retiredControllers.length ? { retiredControllers } : {}) } });
    if (prior && (prior.launchToken !== launchToken || prior.serverUrl !== bridgeServerUrl || !controllerRootUrl(prior.requestedUrl))) {
      throw new Error('Controller persistent identity changed; original state was preserved');
    }
    if (prior && prior.status !== 'opened') throw new Error('Controller creation remains unresolved; original reservation was preserved');
    const sessionRecords = await session.get(null);
    let browserSession = sessionRecords[CONTROLLER_BROWSER_SESSION_KEY];
    if (browserSession === undefined) {
      browserSession = crypto.randomUUID().toLowerCase();
      if (!CONTROLLER_SESSION_UUID_RE.test(browserSession)) throw new Error('Controller browser session identity is invalid');
      await session.set({ [CONTROLLER_BROWSER_SESSION_KEY]: browserSession });
      const verifiedSession = await session.get(CONTROLLER_BROWSER_SESSION_KEY);
      if (verifiedSession[CONTROLLER_BROWSER_SESSION_KEY] !== browserSession) throw new Error('Controller browser session identity was not persisted');
    } else if (!CONTROLLER_SESSION_UUID_RE.test(browserSession)) {
      throw new Error('Controller browser session identity is damaged; original state was preserved');
    }
    const invalidated = prior?.reloadInvalidated;
    const reloadProved = prior?.status === 'opened' && typeof invalidated?.operationId === 'string' && invalidated.operationId.length > 0
      && invalidated.tabId === prior.tabId && invalidated.launchToken === launchToken
      && invalidated.browserSession === prior.browserSession && invalidated.serverUrl === bridgeServerUrl
      && invalidated.requestedUrl === requestedUrl;
    let previousClosedProof = null;
    if (prior?.browserSession && prior.browserSession !== browserSession && !reloadProved) {
      previousClosedProof = await provePreviousControllerRecovery(port, prior, profileId, stateKey, bridgeServerUrl);
      if (!previousClosedProof || previousClosedProof.browserSession !== browserSession) {
        throw new Error('Controller belongs to a previous browser session; no replacement was opened');
      }
      const { retiredControllers: _history, ...retired } = prior;
      retiredControllers = [...retiredControllers, { ...retired, retiredAt: Date.now(),
        reason: previousClosedProof.reason, recoveryProof: previousClosedProof }];
      if (!previousClosedProof.matched) {
        await saveState({ launchToken, serverUrl: bridgeServerUrl, requestedUrl, browserSession, status: 'reserved' });
        const reservation = (await local.get(stateKey))[stateKey];
        if (reservation?.status !== 'reserved' || reservation.browserSession !== browserSession
          || reservation.retiredControllers?.at(-1)?.recoveryProof?.inventorySha256 !== previousClosedProof.inventorySha256) {
          throw new Error('Previous controller archival reservation was not persisted');
        }
      }
    }
    const records = Object.entries(sessionRecords || {}).filter(([key, record]) => key.startsWith(LAUNCHED_TAB_PREFIX) && record?.launchToken === launchToken);
    if (records.length > 1) throw new Error('Controller ownership is not unique; no page was opened');
    const tabs = await chrome.tabs.query({});
    if (!Array.isArray(tabs)) throw new Error('Controller tab inventory is unavailable');
    if (records.length) {
      const [key, record] = records[0];
      const rawId = key.slice(LAUNCHED_TAB_PREFIX.length);
      const tabId = Number(rawId);
      if (!Number.isSafeInteger(tabId) || tabId <= 0 || String(tabId) !== rawId
        || safeBridgeServerUrl(record.serverUrl) !== bridgeServerUrl || !controllerRootUrl(record.requestedUrl)
        || prior?.tabId && prior.tabId !== tabId) throw new Error('Controller launch identity is invalid; no page was changed');
      const existing = tabs.find((tab) => tab.id === tabId);
      if (existing) {
        if (!controllerRootUrl(existing.url) || existing.incognito === true) throw new Error('Controller page has changed; no navigation or replacement was attempted');
        const connected = Array.from(connections.values()).some((state) => state?.tabId === tabId
          && state.closed !== true && state.protocolReady === true && state.ws?.readyState === 1
          && safeBridgeServerUrl(state.serverUrl || '') === bridgeServerUrl);
        if (reloadProved && !connected) {
          // runtime.reload invalidated the old content script. Preserve a user
          // viewing this page; retire only its confirmed idle controller role.
          // A new browser-session nonce is never replaced with the old nonce.
          const { retiredControllers: _history, ...retired } = prior;
          retiredControllers = [...retiredControllers, { ...retired, retiredAt: Date.now(), reason: 'extension_runtime_invalidated' }];
          await saveState({ launchToken, serverUrl: bridgeServerUrl, requestedUrl, browserSession, status: 'reserved' });
          const retirement = (await local.get(stateKey))[stateKey];
          const archived = retirement?.retiredControllers?.at(-1);
          if (retirement?.status !== 'reserved' || retirement.launchToken !== launchToken || retirement.browserSession !== browserSession
            || archived?.tabId !== tabId || archived.browserSession !== prior.browserSession
            || archived.reloadInvalidated?.operationId !== invalidated.operationId) throw new Error('Original controller retirement was not persisted');
          await forgetLaunchedTab(tabId);
          const removed = await session.get(LAUNCHED_TAB_PREFIX + tabId);
          if (removed[LAUNCHED_TAB_PREFIX + tabId]) throw new Error('Retired controller ownership removal was not persisted');
        } else {
          await saveState({ launchToken, serverUrl: bridgeServerUrl, requestedUrl, tabId, browserSession, status: 'opened' });
          // Reuse never navigates or activates the existing page.
          const updated = await updateTab(tabId, { autoDiscardable: false });
          return { tabId, launchToken, requestedUrl, bridgeServerUrl, reused: true, openerTabId: port?.sender?.tab?.id ?? null,
            ...controllerTabState(updated), tabState: controllerTabState(updated) };
        }
      } else {
        // The complete Chrome inventory proves that this known controller closed.
        await forgetLaunchedTab(tabId);
      }
    } else if (prior) {
      // onRemoved clears launched identity. Only a known opened tab from this
      // same browser session may be recreated after complete inventory proves
      // its exact tab id disappeared. Absence alone cannot resolve an unknown.
      if (prior.status !== 'opened' || (prior.browserSession !== browserSession && !previousClosedProof)
        || !Number.isSafeInteger(prior.tabId) || prior.tabId <= 0
        || tabs.some((tab) => tab.id === prior.tabId)) {
        throw new Error('Controller launch identity is missing or creation remains unknown; no replacement was opened');
      }
    }
    await saveState({ launchToken, serverUrl: bridgeServerUrl, requestedUrl, browserSession, status: 'reserved' });
    const verified = await local.get(stateKey);
    if (verified[stateKey]?.status !== 'reserved' || verified[stateKey]?.launchToken !== launchToken) throw new Error('Controller reservation was not persisted');
    const tab = await createTab({ url: 'about:blank', active: false });
    if (!Number.isInteger(tab?.id)) throw new Error('Controller creation has unknown tab identity');
    // Preserve even a navigation/storage failure as an unknown owned create.
    await saveState({ launchToken, serverUrl: bridgeServerUrl, requestedUrl, tabId: tab.id, browserSession, status: 'reserved' });
    await rememberLaunchedTab(tab.id, { launchToken, requestedUrl, createdAt: Date.now(), serverUrl: bridgeServerUrl });
    const recordsAfter = await session.get(LAUNCHED_TAB_PREFIX + tab.id);
    if (recordsAfter[LAUNCHED_TAB_PREFIX + tab.id]?.launchToken !== launchToken) throw new Error('Controller launch ownership was not persisted');
    const updated = await updateTab(tab.id, { url: requestedUrl, active: false, autoDiscardable: false });
    await saveState({ launchToken, serverUrl: bridgeServerUrl, requestedUrl, tabId: tab.id, browserSession, status: 'opened' });
    return { tabId: tab.id, launchToken, requestedUrl, bridgeServerUrl, openerTabId: port?.sender?.tab?.id ?? null,
      ...controllerTabState(updated), tabState: controllerTabState(updated) };
  }

  async function openBridgeTab(port, options = {}) {
    if(options.lookupOnly===true)return lookupOwnedLaunch(port,options);
    const requestedUrl = safeChatUrl(options.url || 'https://chatgpt.com/');
    const launchToken = String(options.launchToken || `bridge-tab-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`);
    const connectionServerUrl = connections.get(port)?.serverUrl || '';
    const bridgeServerUrl = safeBridgeServerUrl(options.bridgeServerUrl || connectionServerUrl);
    const active = options.active !== false;
    if (CONTROLLER_LAUNCH_RE.test(launchToken)) {
      const pending = controllerQueue.then(() => openControllerTab(port, options, launchToken, requestedUrl, bridgeServerUrl));
      controllerQueue = pending.catch(() => {});
      return await pending;
    }
    const directorOwned = /^bridge-director-desktop-[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i.test(launchToken);
    let sourceWindowId;
    if(directorOwned){
      // Chrome otherwise chooses the last focused window, which can belong to
      // the user. Keep every work page in its already-owned controller window.
      const sourceId=port?.sender?.tab?.id,record=await readLaunchedTab(sourceId);
      const source=await chrome.tabs.get(sourceId);
      if(active||!source||source.id!==sourceId||!Number.isSafeInteger(source.windowId)||source.windowId<=0
        ||source.incognito!==false||!controllerRootUrl(source.url)||record?.serverUrl!==bridgeServerUrl
        ||!(CONTROLLER_LAUNCH_RE.test(record?.launchToken||'')||/^bridge-director-desktop-[a-f0-9-]{36}$/i.test(record?.launchToken||'')))throw Error('Owned background source window is unproved');
      sourceWindowId=source.windowId;
    }
    const tab = await createTab({ url: 'about:blank', active, ...(directorOwned?{windowId:sourceWindowId}:{}) });
    if (!Number.isInteger(tab?.id)) throw new Error('Chrome did not return a tab id for the new ChatGPT tab');
    try {
      // Persist ownership before navigation so a fast content connection cannot
      // announce without its one-time launch identity.
      await rememberLaunchedTab(tab.id, { launchToken, requestedUrl, createdAt: Date.now(), serverUrl: bridgeServerUrl });
      if(directorOwned&&tab.windowId!==sourceWindowId)throw Error('Chrome returned a different owned work window');
      await updateTab(tab.id, { url: requestedUrl, active, ...(directorOwned ? { autoDiscardable: false } : {}) });
    } catch (error) {
      await forgetLaunchedTab(tab.id);
      await removeTab(tab.id).catch(() => {});
      throw error;
    }
    return { tabId: tab.id, launchToken, requestedUrl, bridgeServerUrl, active, openerTabId: port?.sender?.tab?.id ?? null };
  }

  async function closeOwnBridgeTab(port, options = {}) {
    const tabId = port?.sender?.tab?.id;
    if (!Number.isInteger(tabId)) throw new Error('The content-script port is not associated with a browser tab');
    const launch = await readLaunchedTab(tabId);
    const expectedLaunchToken = String(options.expectedLaunchToken || '');
    if (expectedLaunchToken && launch?.launchToken !== expectedLaunchToken) {
      throw new Error('Refusing to close tab because its launch token does not match');
    }
    setTimeout(() => { void removeTab(tabId).catch(() => {}); }, 150);
    return { tabId, closing: true, launchToken: launch?.launchToken || '' };
  }

  async function closeOwnedBridgeTab(_port, options = {}) {
    const tabId = Number(options.tabId);
    const expectedLaunchToken = String(options.expectedLaunchToken || '');
    if (!Number.isInteger(tabId)) throw new Error('A numeric owned tab id is required');
    if (!isStableLaunchToken(expectedLaunchToken)) {
      throw new Error('A stable expected launch token is required to close another owned tab');
    }
    const launch = await readLaunchedTab(tabId);
    if (!launch || launch.launchToken !== expectedLaunchToken) {
      throw new Error('Refusing to close owned tab because its launch token does not match');
    }
    if(options.expectedUrl){const tab=await chrome.tabs.get(tabId);if(tab.url!==options.expectedUrl||tab.incognito!==false)throw Error('Refusing to close owned tab because its current URL or profile changed');}
    setTimeout(() => { void removeTab(tabId).catch(() => {}); }, 150);
    return { tabId, closing: true, launchToken: launch.launchToken };
  }

  async function navigateTab(tabId, url) {
    if (!Number.isInteger(tabId)) throw new Error('A numeric tab id is required');
    const targetUrl = String(url || '');
    if (!targetUrl) throw new Error('A target URL is required');
    await updateTab(tabId, { url: targetUrl });
    return { tabId, url: targetUrl };
  }

  async function reloadTab(tabId) {
    if (!Number.isInteger(tabId)) throw new Error('A numeric tab id is required');
    await chrome.tabs.reload(tabId);
    return { tabId, reloading: true };
  }

  async function reloadOwnBridgeTab(port, options = {}) {
    const tabId = port?.sender?.tab?.id;
    if (!Number.isInteger(tabId)) throw new Error('The content-script port is not associated with a browser tab');
    setTimeout(() => { void chrome.tabs.reload(tabId).catch(() => {}); }, 150);
    return { tabId, reloading: true, reason: String(options.reason || '') };
  }

  return Object.freeze({ openBridgeTab, closeOwnBridgeTab, closeOwnedBridgeTab, navigateTab, reloadTab, reloadOwnBridgeTab });
}
