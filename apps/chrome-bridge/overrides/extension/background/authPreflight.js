const BRIDGE_TOKEN_MARKER = '__chatgpt_bridge_secret_in_extension_storage_v1__';
const BRIDGE_TOKEN_STORAGE_KEY = 'chatgptBridge:secret:bridge.token';

function httpUrl(serverUrl, pathname) {
  const base = String(serverUrl || 'http://127.0.0.1:8080')
    .replace(/\/$/, '')
    .replace(/^ws:/, 'http:')
    .replace(/^wss:/, 'https:');
  return new URL(pathname, base);
}

function bridgeAuthCheckUrl(serverUrl, token) {
  const url = httpUrl(serverUrl, '/extension/auth/check');
  if (token) url.searchParams.set('token', token);
  url.searchParams.set('runtime', 'extension');
  return url.toString();
}

function responseDetailText(status, bodyText = '') {
  const text = String(bodyText || '').trim();
  if (!text) return `HTTP ${status}`;
  try {
    const json = JSON.parse(text);
    return String(json.detail || json.error || json.message || text).slice(0, 300);
  } catch {
    return text.slice(0, 300);
  }
}

async function resolveBridgeToken(token) {
  const candidate = String(token || '');
  if (candidate !== BRIDGE_TOKEN_MARKER) return candidate;
  try {
    const stored = await chrome.storage?.local?.get?.(BRIDGE_TOKEN_STORAGE_KEY);
    return String(stored?.[BRIDGE_TOKEN_STORAGE_KEY] || '');
  } catch {
    return '';
  }
}

export async function checkBridgeAuth(state, fetchImpl = globalThis.fetch, {timeoutMs=10000}={}) {
  const token = await resolveBridgeToken(state.token);
  if (!token) {
    return {
      ok: false,
      authError: true,
      status: 401,
      message: 'BRIDGE_TOKEN is not configured in extension-private storage.',
    };
  }

  // Resolve the marker before the WebSocket URL is constructed. The
  // background runtime owns the real token after this point; page-local storage
  // never needs to contain it.
  state.token = token;
  const url = bridgeAuthCheckUrl(state.serverUrl, token);
  const controller=new AbortController();
  const timer=setTimeout(()=>controller.abort(),Math.max(1,Math.min(Number(timeoutMs)||10000,10000)));
  try {
    const response = await fetchImpl(url, {
      method: 'GET',
      credentials: 'omit',
      cache: 'no-store',
      signal: controller.signal,
    });
    if (response.ok) return { ok: true };
    const text = await response.text().catch(() => '');
    const detail = responseDetailText(response.status, text);
    if (response.status === 401 || response.status === 403) {
      return {
        ok: false,
        authError: true,
        status: response.status,
        message: `BRIDGE_TOKEN was rejected by the bridge server (${response.status}). ${detail}`,
      };
    }
    return {
      ok: false,
      status: response.status,
      message: `Bridge auth preflight failed: ${detail}`,
    };
  } catch (err) {
    return { ok: false, offline: true, message: err?.message || String(err) };
  } finally {
    clearTimeout(timer);
  }
}

// This health path does not depend on a content-script port. It only reports
// why the owned controller cannot reconnect; it cannot dispatch browser work.
export async function reportBackgroundHealth({browser=chrome, backgroundEpoch, bundleId, extensionVersion,
  recovery, connectionCount=0, fetchImpl=globalThis.fetch, timeoutMs=5000}={}) {
  const profileKey='directorDesktop:browserProfileId:v1';
  const validId=/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
  let timer;
  try {
    const first=await browser.storage.local.get(profileKey), profileId=first[profileKey];
    if(!validId.test(profileId||''))return {ok:false,reason:'profile_unbound'};
    const controlKey='directorDesktop:controlTab:v1:'+profileId;
    const stored=await browser.storage.local.get([profileKey,controlKey,BRIDGE_TOKEN_STORAGE_KEY]);
    const control=stored[controlKey],token=stored[BRIDGE_TOKEN_STORAGE_KEY];
    if(stored[profileKey]!==profileId || control?.launchToken!=='bridge-director-control-'+profileId
      || typeof token!=='string'||!token)return {ok:false,reason:'binding_unavailable'};
    const server=new URL(control.serverUrl);
    if(server.protocol!=='http:'||!['127.0.0.1','localhost'].includes(server.hostname)
      ||server.username||server.password||server.pathname!=='/'||server.search||server.hash)return {ok:false,reason:'server_invalid'};
    if(!/^[a-f0-9]{32}$/.test(bundleId||'')||typeof backgroundEpoch!=='string'||!backgroundEpoch||backgroundEpoch.length>160
      ||typeof extensionVersion!=='string'||extensionVersion.length>40)return {ok:false,reason:'runtime_invalid'};
    const safeWord=value=>typeof value==='string'&&/^[a-z0-9_]{1,96}$/.test(value)?value:'unknown';
    const payload={schemaVersion:'extension-background-health/v1',profileId,bundleId,backgroundEpoch,extensionVersion,
      status:safeWord(recovery?.status),reason:recovery?.reason?safeWord(recovery.reason):'',
      connectionCount:Number.isSafeInteger(connectionCount)&&connectionCount>=0?Math.min(connectionCount,1000):0};
    const abort=new AbortController();timer=setTimeout(()=>abort.abort(),Math.max(1,Math.min(Number(timeoutMs)||5000,5000)));
    const response=await fetchImpl(new URL('/extension/background/status',server),{method:'POST',credentials:'omit',cache:'no-store',redirect:'error',
      headers:{'Content-Type':'application/json','X-Bridge-Token':token},body:JSON.stringify(payload),signal:abort.signal});
    return {ok:response.ok,reason:response.ok?'reported':'server_unavailable'};
  }catch{return {ok:false,reason:'report_unavailable'};}
  finally{clearTimeout(timer);}
}

// Background transport can be addressable before a page is ready for generation.
export async function readOnlyBootstrapHello(state, runtime, launch, browser = chrome) {
  if (!Number.isInteger(state.tabId) || state.tabId <= 0 || !state.contentEpoch
      || runtime.contentEpoch !== state.contentEpoch || runtime.tabId !== state.tabId) {
    throw new Error('Bootstrap content identity is unproved');
  }
  const tab = await browser.tabs.get(state.tabId);
  const url = new URL(tab.url || '');
  if (tab.id !== state.tabId || url.origin !== 'https://chatgpt.com'
      || url.username || url.password || typeof tab.incognito !== 'boolean') {
    throw new Error('Bootstrap native tab identity is unproved');
  }
  const window=await browser.windows.get(tab.windowId);
  if(window.id!==tab.windowId||typeof window.focused!=='boolean'||typeof tab.active!=='boolean')throw new Error('Bootstrap native window identity is unproved');
  const {type, ...page} = state.initialPage || {};
  return {...page, url:tab.url, title:tab.title||'', browserTabId:state.tabId,
    launchToken:launch?.launchToken||'', requestedUrl:launch?.requestedUrl||'',
    documentReadyState:'loading',pageReady:false,composerReady:false,chatMainReady:false,
    focused:tab.active&&window.focused,visibilityState:tab.active&&window.state!=='minimized'?'visible':'hidden',session:null,tabObservation:null,
    activeRequest:runtime.lease||null,
    transportHealth:{kind:'connect_snapshot',observedAt:Date.now(),backgroundOnly:true}};
}
export function bootstrapAckMatches(state, envelope, backgroundEpoch) {
  const hello=state.pendingHelloReceipt;
  return Boolean(hello && hello.body?.transportHealth?.backgroundOnly===true
    && envelope.messageType==='transport.ack' && envelope.body?.accepted===true
    && envelope.body?.ackMessageId===hello.messageId
    && Number(envelope.body.acceptedSequence)===Number(hello.source.sequence)
    && hello.source.clientId===state.clientId && hello.source.contentEpoch===state.contentEpoch
    && hello.source.backgroundEpoch===backgroundEpoch);
}
export function transportFailureCode(error) {
  const code=typeof error?.code==='string'?error.code.toLowerCase():'transport_open_failed';
  return /^[a-z0-9_]{1,96}$/.test(code)?code:'transport_open_failed';
}

export function backgroundPongEligible(state, envelope) {
  return Boolean(!state.closed && state.backgroundHandshakeReady && !state.protocolReady
    && envelope.messageType === 'transport.ping');
}
