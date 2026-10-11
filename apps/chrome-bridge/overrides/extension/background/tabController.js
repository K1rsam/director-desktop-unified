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
} = {}) {
  if (!connections || typeof connections.get !== 'function') throw new TypeError('Tab controller requires connections');
  if (typeof safeBridgeServerUrl !== 'function') throw new TypeError('Tab controller requires safeBridgeServerUrl');
  if (typeof rememberLaunchedTab !== 'function' || typeof readLaunchedTab !== 'function' || typeof forgetLaunchedTab !== 'function') {
    throw new TypeError('Tab controller requires launched-tab storage adapters');
  }
  if (typeof isStableLaunchToken !== 'function') throw new TypeError('Tab controller requires launch-token validation');
  let controllerQueue = Promise.resolve();

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
    if (prior?.browserSession && prior.browserSession !== browserSession && !reloadProved) {
      throw new Error('Controller belongs to a previous browser session; no replacement was opened');
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
      if (prior.status !== 'opened' || prior.browserSession !== browserSession
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
    const tab = await createTab({ url: 'about:blank', active });
    if (!Number.isInteger(tab?.id)) throw new Error('Chrome did not return a tab id for the new ChatGPT tab');
    try {
      // Persist ownership before navigation so a fast content connection cannot
      // announce without its one-time launch identity.
      await rememberLaunchedTab(tab.id, { launchToken, requestedUrl, createdAt: Date.now(), serverUrl: bridgeServerUrl });
      const directorOwned = /^bridge-director-desktop-[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i.test(launchToken);
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
