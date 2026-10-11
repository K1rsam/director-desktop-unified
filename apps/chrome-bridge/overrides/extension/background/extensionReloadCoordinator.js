const PENDING_EXTENSION_RELOAD_KEY = 'bridgePendingExtensionReload';
const PENDING_EXTENSION_RELOAD_TTL_MS = 2 * 60_000;
const LAUNCHED_TAB_STORAGE_PREFIX = 'chatgptBridgeLaunchedTab:';
const CONTROLLER_STATE_PREFIX = 'directorDesktop:controlTab:v1:';
const CONTROLLER_SESSION_UUID_RE = /^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const DIRECTOR_LAUNCH_TOKEN_RE = /^bridge-director-desktop-[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i;
const CONTROLLER_LAUNCH_TOKEN_RE = /^bridge-director-control-[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const ownedDesktopLaunch = (token) => DIRECTOR_LAUNCH_TOKEN_RE.test(token) || CONTROLLER_LAUNCH_TOKEN_RE.test(token);
export const EXTENSION_RELOAD_ALARM_PREFIX = 'chatgptBridge:extensionReload:';

export function extensionReloadAlarmName(operationId = '') {
  const id = String(operationId || '').trim();
  return id ? `${EXTENSION_RELOAD_ALARM_PREFIX}${id}` : '';
}

export function isExtensionReloadAlarm(name = '') {
  return String(name || '').startsWith(EXTENSION_RELOAD_ALARM_PREFIX);
}

export function createExtensionReloadCoordinator({
  backgroundState,
  maintenanceOperations,
  safeBridgeServerUrl,
  readLaunchedTab,
  rememberLaunchedTab,
  navigateTab,
  reloadTab,
  launchTokenPattern,
  reloadRuntime = () => chrome.runtime.reload(),
  scheduleRecoveryWake = null,
  clearRecoveryWake = null,
  ackTimeoutMs = 7_000,
} = {}) {
  let pendingRecovery = null;

  async function armRecoveryWake(operationId) {
    const alarmName = extensionReloadAlarmName(operationId);
    if (!alarmName) return { armed: false, reason: 'missing_operation_id' };
    if (typeof scheduleRecoveryWake === 'function') return await scheduleRecoveryWake(alarmName);
    if (!chrome.alarms?.create) return { armed: false, reason: 'alarms_unavailable', alarmName };
    const when = Date.now() + 750;
    await chrome.alarms.create(alarmName, { when });
    if (chrome.alarms.get) {
      const alarm = await chrome.alarms.get(alarmName);
      if (!alarm) {
        const error = new Error('Extension reload recovery alarm was not persisted');
        error.code = 'MAINTENANCE_ALARM_NOT_PERSISTED';
        throw error;
      }
    }
    return { armed: true, alarmName, when };
  }

  async function disarmRecoveryWake(alarmName = '') {
    const name = String(alarmName || '');
    if (!name) return false;
    if (typeof clearRecoveryWake === 'function') return await clearRecoveryWake(name);
    if (!chrome.alarms?.clear) return false;
    return await chrome.alarms.clear(name);
  }

  function temporaryReloadUrl(rawUrl = '', serverUrl = '', launchToken = '') {
    try {
      const url = new URL(String(rawUrl || ''));
      const safeServerUrl = safeBridgeServerUrl(serverUrl);
      if (!safeServerUrl || !['https://chatgpt.com', 'https://chat.openai.com'].includes(url.origin)) return '';
      const params = new URLSearchParams(url.hash.replace(/^#/, ''));
      const stableLaunchToken = launchTokenPattern.test(String(launchToken || '')) && !String(launchToken).startsWith('bridge-reload-')
        ? String(launchToken)
        : `bridge-reload-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
      params.set('chatgpt-bridge-launch', stableLaunchToken);
      params.set('chatgpt-bridge-server', safeServerUrl);
      url.hash = params.toString();
      return url.toString();
    } catch {
      return '';
    }
  }

  function reloadTrampolineUrl(rawUrl = '', serverUrl = '', launchToken = '', delayMs = 2_500) {
    const safeServer = safeBridgeServerUrl(serverUrl);
    const targetUrl = temporaryReloadUrl(rawUrl, safeServer, launchToken);
    if (!safeServer || !targetUrl) return { targetUrl: '', trampolineUrl: '' };
    try {
      const trampoline = new URL('/extension/reload-trampoline', safeServer);
      trampoline.searchParams.set('target', targetUrl);
      trampoline.searchParams.set('delayMs', String(Math.max(500, Math.min(Number(delayMs) || 2_500, 15_000))));
      return { targetUrl, trampolineUrl: trampoline.toString() };
    } catch {
      return { targetUrl: '', trampolineUrl: '' };
    }
  }

  async function reloadTabWithTemporaryConnection(tabId, serverUrl, launchToken = '') {
    if (!Number.isInteger(tabId) || !serverUrl || !chrome.tabs?.get || !chrome.tabs?.update) return false;
    const tab = await chrome.tabs.get(tabId).catch(() => null);
    const url = temporaryReloadUrl(tab?.url || '', serverUrl, launchToken);
    if (!url) return false;
    try {
      await navigateTab(tabId, url);
      return true;
    } catch {
      return false;
    }
  }

  function pendingLaunchRecord(pending = {}, tabId) {
    const record = pending.launchRecords?.[String(tabId)] || null;
    const launchToken = String(record?.launchToken || '');
    if (!launchTokenPattern.test(launchToken) || launchToken.startsWith('bridge-reload-')) return null;
    return {
      launchToken,
      requestedUrl: String(record.requestedUrl || ''),
      createdAt: Number(record.createdAt || pending.requestedAt || Date.now()),
      serverUrl: safeBridgeServerUrl(record.serverUrl || ''),
    };
  }

  async function restorePendingLaunchRecords(pending = {}) {
    const refreshTabIds = new Set(Array.isArray(pending.tabIds) ? pending.tabIds : []);
    for (const rawTabId of Object.keys(pending.launchRecords || {})) {
      const tabId = Number(rawTabId);
      if (!Number.isSafeInteger(tabId) || tabId <= 0 || String(tabId) !== rawTabId) continue;
      const record = pendingLaunchRecord(pending, tabId);
      if (record && (ownedDesktopLaunch(record.launchToken) || refreshTabIds.has(tabId))) {
        await rememberLaunchedTab(tabId, record);
      }
    }
  }

  async function snapshotIdleControllers(launchRecords) {
    const controllers = {};
    for (const [rawId, record] of Object.entries(launchRecords)) {
      if (!CONTROLLER_LAUNCH_TOKEN_RE.test(String(record?.launchToken || ''))) continue;
      const tabId = Number(rawId);
      const runtime = await backgroundState.read(tabId);
      if (runtime.lease || Object.values(runtime.commands || {}).some((command) => !['succeeded', 'rejected'].includes(command.status))) {
        const error = new Error('Controller recovery is blocked while original browser work is unresolved');
        error.code = 'MAINTENANCE_BLOCKED_BY_ACTIVE_LEASE';
        throw error;
      }
      const stateKey = CONTROLLER_STATE_PREFIX + record.launchToken.slice('bridge-director-control-'.length);
      const stored = await chrome.storage.local.get(stateKey);
      const prior = stored?.[stateKey];
      const tab = chrome.tabs?.get ? await chrome.tabs.get(tabId).catch(() => null) : null;
      let rootPage = false;
      try { const url = new URL(tab?.url || ''); rootPage = url.origin === 'https://chatgpt.com' && url.pathname === '/' && !url.search && !url.hash && !url.username && !url.password; } catch {}
      // Unknown creates are not made replaceable by maintenance. Only the
      // original confirmed idle control page can receive invalidation proof.
      if (prior?.status !== 'opened' || prior.tabId !== tabId || prior.launchToken !== record.launchToken
        || !CONTROLLER_SESSION_UUID_RE.test(prior.browserSession || '') || !rootPage || tab.incognito === true
        || prior.serverUrl !== record.serverUrl || prior.requestedUrl !== record.requestedUrl) continue;
      // This is proof of an already completed, currently idle creation, not
      // permission to restore its old browser-session nonce after a restart.
      controllers[stateKey] = { tabId, launchToken: prior.launchToken, browserSession: prior.browserSession,
        serverUrl: prior.serverUrl, requestedUrl: prior.requestedUrl };
    }
    return controllers;
  }

  async function markReloadedControllers(pending) {
    for (const [stateKey, proof] of Object.entries(pending.idleControllers || {})) {
      const expectedKey = CONTROLLER_STATE_PREFIX + String(proof.launchToken || '').slice('bridge-director-control-'.length);
      if (stateKey !== expectedKey || !CONTROLLER_LAUNCH_TOKEN_RE.test(proof.launchToken || '')) continue;
      const stored = await chrome.storage.local.get(stateKey);
      const prior = stored?.[stateKey];
      if (prior?.status !== 'opened' || prior.tabId !== proof.tabId || prior.launchToken !== proof.launchToken
        || prior.browserSession !== proof.browserSession || prior.serverUrl !== proof.serverUrl
        || prior.requestedUrl !== proof.requestedUrl) continue;
      await chrome.storage.local.set({ [stateKey]: { ...prior, reloadInvalidated: { ...proof, operationId: pending.operationId } } });
    }
  }

  async function snapshotOwnedLaunchRecords() {
    const storage = chrome.storage?.session;
    if (!storage?.get) {
      const error = new Error('Extension reload requires a complete session launch identity snapshot');
      error.code = 'MAINTENANCE_SESSION_STORAGE_UNAVAILABLE';
      throw error;
    }
    let stored;
    try {
      stored = await storage.get(null);
    } catch (cause) {
      const error = new Error('Extension reload was blocked because session launch identities could not be preserved', { cause });
      error.code = 'MAINTENANCE_SESSION_READ_FAILED';
      throw error;
    }
    const records = {};
    for (const [key, record] of Object.entries(stored || {})) {
      if (!key.startsWith(LAUNCHED_TAB_STORAGE_PREFIX)) continue;
      const rawTabId = key.slice(LAUNCHED_TAB_STORAGE_PREFIX.length);
      const tabId = Number(rawTabId);
      if (!Number.isSafeInteger(tabId) || tabId <= 0 || String(tabId) !== rawTabId) continue;
      if (!ownedDesktopLaunch(String(record?.launchToken || ''))) continue;
      records[rawTabId] = record;
    }
    return records;
  }

  async function reloadChatGptTabsAfterExtensionRestart() {
    const storage = chrome.storage?.local;
    if (!storage?.get || !storage?.remove || !storage?.set) {
      const error = new Error('Extension reload recovery requires durable local storage');
      error.code = 'MAINTENANCE_STORAGE_UNAVAILABLE';
      throw error;
    }
    const stored = await storage.get(PENDING_EXTENSION_RELOAD_KEY);
    const pending = stored?.[PENDING_EXTENSION_RELOAD_KEY];
    if (!pending || !Array.isArray(pending.tabIds)) return { recovered: false, reason: 'missing' };
    console.info('[chatgpt-bridge] Restoring extension reload', {
      operationId: String(pending.operationId || ''),
      commandId: String(pending.commandId || ''),
      expectedVersion: String(pending.expectedVersion || ''),
      tabCount: pending.tabIds.length,
    });
    const requestedAt = Number(pending.requestedAt || 0);
    if (!requestedAt || Date.now() - requestedAt > PENDING_EXTENSION_RELOAD_TTL_MS) {
      await storage.remove(PENDING_EXTENSION_RELOAD_KEY);
      await disarmRecoveryWake(pending.recoveryAlarmName);
      if (pending.operationId) await maintenanceOperations.fail(pending.operationId, {
        code: 'MAINTENANCE_EXPIRED',
        message: 'Extension reload maintenance expired before recovery',
      });
      return { recovered: false, reason: 'expired' };
    }
    const sourceTabId = Number.isInteger(pending.sourceTabId) ? pending.sourceTabId : null;
    const serverUrl = safeBridgeServerUrl(pending.temporaryServerUrl);
    await restorePendingLaunchRecords(pending);
    await markReloadedControllers(pending);
    for (const tabId of pending.tabIds) {
      const targetUrl = String(pending.targetUrls?.[String(tabId)] || '');
      if (targetUrl) {
        try { await navigateTab(tabId, targetUrl); continue; } catch {}
      }
      const launchRecord = pendingLaunchRecord(pending, tabId);
      if (tabId === sourceTabId && await reloadTabWithTemporaryConnection(tabId, serverUrl, launchRecord?.launchToken || '')) continue;
      if (chrome.tabs?.reload) await reloadTab(tabId);
    }
    await storage.remove(PENDING_EXTENSION_RELOAD_KEY);
    await disarmRecoveryWake(pending.recoveryAlarmName);
    const result = { recovered: true, tabCount: pending.tabIds.length, sourceTabId };
    if (pending.operationId) await maintenanceOperations.succeed(pending.operationId, result);
    console.info('[chatgpt-bridge] Extension reload recovery completed', {
      operationId: String(pending.operationId || ''),
      commandId: String(pending.commandId || ''),
      tabCount: pending.tabIds.length,
      sourceTabId,
    });
    return result;
  }

  function recoverPendingExtensionReload() {
    if (pendingRecovery) return pendingRecovery;
    pendingRecovery = reloadChatGptTabsAfterExtensionRestart().finally(() => { pendingRecovery = null; });
    return pendingRecovery;
  }

  async function clearPendingExtensionReload() {
    try {
      await chrome.storage?.local?.remove?.(PENDING_EXTENSION_RELOAD_KEY);
    } catch {}
  }

  async function reloadAfterAcceptanceAck({ tabId, commandId, operationId, pending }) {
    const deadline = Date.now() + Math.max(1_000, Number(ackTimeoutMs) || 7_000);
    while (Date.now() < deadline) {
      const runtime = await backgroundState.read(tabId);
      const command = runtime.commands?.[commandId] || null;
      const commandFailed = ['rejected', 'uncertain'].includes(String(command?.status || ''));
      const dispatchCommitted = ['dispatched', 'succeeded'].includes(String(command?.status || ''));
      const acceptancePending = runtime.outbox.some((entry) => String(entry.commandId || '') === commandId && entry.messageType === 'command.accepted');
      if (dispatchCommitted && !acceptancePending) {
        const wake = await armRecoveryWake(operationId);
        if (wake.armed) {
          const storage = chrome.storage?.local;
          const stored = await storage?.get?.(PENDING_EXTENSION_RELOAD_KEY);
          const pending = stored?.[PENDING_EXTENSION_RELOAD_KEY] || null;
          if (pending) await storage.set({
            [PENDING_EXTENSION_RELOAD_KEY]: { ...pending, recoveryAlarmName: wake.alarmName, recoveryWakeAt: wake.when },
          });
        }
        let trampolineCount = 0;
        for (const [rawTabId, trampolineUrl] of Object.entries(pending?.trampolineUrls || {})) {
          const targetTabId = Number(rawTabId);
          if (!Number.isInteger(targetTabId) || !trampolineUrl) continue;
          try {
            await navigateTab(targetTabId, trampolineUrl);
            trampolineCount += 1;
          } catch (error) {
            console.warn('[chatgpt-bridge] Extension reload trampoline navigation failed', {
              operationId: String(operationId || ''), tabId: targetTabId, message: error?.message || String(error),
            });
          }
        }
        if (trampolineCount) await new Promise((resolve) => setTimeout(resolve, 120));
        console.info('[chatgpt-bridge] Restarting extension runtime after acknowledged command', {
          commandId: String(commandId || ''), operationId: String(operationId || ''), tabId,
          recoveryAlarm: wake.alarmName || '', recoveryWakeArmed: wake.armed === true, trampolineCount,
        });
        reloadRuntime();
        return { reloading: true, recoveryWake: wake, trampolineCount };
      }
      if (commandFailed) {
        const error = new Error('Extension reload command was rejected before the accepted dispatch was acknowledged');
        error.code = 'MAINTENANCE_COMMAND_REJECTED';
        await clearPendingExtensionReload();
        await maintenanceOperations.fail(operationId, { code: error.code, message: error.message });
        throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    const error = new Error('Extension reload was not started because command acceptance was not acknowledged by the server');
    error.code = 'MAINTENANCE_ACK_TIMEOUT';
    await clearPendingExtensionReload();
    await maintenanceOperations.fail(operationId, { code: error.code, message: error.message });
    throw error;
  }

  async function scheduleExtensionReload({
    reloadTabs = true,
    ownedTabsOnly = false,
    expectedVersion = '',
    sourceTabId = null,
    sourceLaunchToken = '',
    temporaryServerUrl = '',
    commandId = '',
  } = {}) {
    const terminalCommandId = String(commandId || '');
    if (!Number.isInteger(sourceTabId) || !terminalCommandId) {
      const error = new Error('Extension reload requires the source tab and correlated command identity');
      error.code = 'MAINTENANCE_COMMAND_IDENTITY_REQUIRED';
      throw error;
    }
    let tabs = reloadTabs && chrome.tabs?.query
      ? await chrome.tabs.query({ url: ['https://chatgpt.com/*', 'https://chat.openai.com/*'] }).catch(() => [])
      : [];
    if (ownedTabsOnly) {
      const owned = [];
      for (const tab of tabs) {
        if ((await backgroundState.read(tab.id)).lease) {
          const error = new Error('Extension maintenance is blocked while browser leases are active');
          error.code = 'MAINTENANCE_BLOCKED_BY_ACTIVE_LEASE';
          throw error;
        }
        const record = await readLaunchedTab(tab.id);
        if (ownedDesktopLaunch(String(record?.launchToken || ''))) owned.push(tab);
      }
      tabs = owned;
      const sourceRecord = await readLaunchedTab(sourceTabId);
      if (!ownedDesktopLaunch(String(sourceRecord?.launchToken || ''))) {
        throw new Error('Owned-only maintenance requires an original desktop work page');
      }
    }
    if (Number.isInteger(sourceTabId) && !tabs.some((tab) => tab?.id === sourceTabId)) {
      const sourceTab = chrome.tabs?.get ? await chrome.tabs.get(sourceTabId).catch(() => null) : null;
      tabs.push(sourceTab && Number.isInteger(sourceTab.id) ? sourceTab : { id: sourceTabId });
    }
    const activeLeases = [];
    for (const tab of tabs) {
      if (!Number.isInteger(tab?.id)) continue;
      const runtime = await backgroundState.read(tab.id);
      if (runtime.lease) activeLeases.push({ tabId: tab.id, requestId: runtime.lease.requestId, leaseId: runtime.lease.leaseId });
    }
    if (activeLeases.length) {
      const error = new Error('Extension maintenance is blocked while browser leases are active');
      error.code = 'MAINTENANCE_BLOCKED_BY_ACTIVE_LEASE';
      error.activeLeases = activeLeases;
      throw error;
    }
    // Runtime reload clears session storage for every work page, even when only
    // the source page is refreshed. Preserve ownership without expanding tabs.
    const launchRecords = await snapshotOwnedLaunchRecords();
    const idleControllers = await snapshotIdleControllers(launchRecords);
    const planned = await maintenanceOperations.plan('extension.reload', {
      preconditions: { expectedVersion: String(expectedVersion || ''), tabIds: tabs.map((tab) => tab.id).filter(Number.isInteger) },
    });
    if (!planned.accepted) throw new Error(`Extension maintenance plan rejected: ${planned.reason}`);
    const operationId = planned.state.active.operationId;
    const targetUrls = {};
    const trampolineUrls = {};
    for (const tab of tabs) {
      if (!Number.isInteger(tab?.id)) continue;
      const record = await readLaunchedTab(tab.id);
      if (record?.launchToken && !String(record.launchToken).startsWith('bridge-reload-')) launchRecords[String(tab.id)] = record;
      const currentTab = tab?.url ? tab : (chrome.tabs?.get ? await chrome.tabs.get(tab.id).catch(() => null) : null);
      const stableToken = String(record?.launchToken || (tab.id === sourceTabId ? sourceLaunchToken : ''));
      const trampoline = reloadTrampolineUrl(currentTab?.url || '', temporaryServerUrl, stableToken);
      if (trampoline.targetUrl) targetUrls[String(tab.id)] = trampoline.targetUrl;
      if (trampoline.trampolineUrl) trampolineUrls[String(tab.id)] = trampoline.trampolineUrl;
    }
    const pending = {
      tabIds: tabs.map((tab) => tab.id).filter(Number.isInteger),
      expectedVersion: String(expectedVersion || ''),
      sourceTabId: Number.isInteger(sourceTabId) ? sourceTabId : null,
      temporaryServerUrl: safeBridgeServerUrl(temporaryServerUrl),
      launchRecords,
      idleControllers,
      targetUrls,
      trampolineUrls,
      requestedAt: Date.now(),
      operationId,
      commandId: terminalCommandId,
    };
    if (Number.isInteger(pending.sourceTabId)
      && launchTokenPattern.test(String(sourceLaunchToken || ''))
      && !String(sourceLaunchToken).startsWith('bridge-reload-')) {
      pending.launchRecords[String(pending.sourceTabId)] ||= {
        launchToken: String(sourceLaunchToken),
        requestedUrl: '',
        createdAt: pending.requestedAt,
        serverUrl: pending.temporaryServerUrl,
      };
    }
    if (!chrome.storage?.local?.set) {
      const error = new Error('Extension reload scheduling requires durable local storage');
      error.code = 'MAINTENANCE_STORAGE_UNAVAILABLE';
      await maintenanceOperations.fail(operationId, { code: error.code, message: error.message });
      throw error;
    }
    try {
      await chrome.storage.local.set({ [PENDING_EXTENSION_RELOAD_KEY]: pending });
    } catch (error) {
      await maintenanceOperations.fail(operationId, { code: 'MAINTENANCE_PENDING_WRITE_FAILED', message: error?.message || String(error) });
      throw error;
    }
    const dispatched = await maintenanceOperations.dispatch(operationId);
    if (!dispatched.accepted) throw new Error(`Extension maintenance dispatch rejected: ${dispatched.reason}`);
    console.info('[chatgpt-bridge] Extension reload scheduled', {
      operationId, commandId: terminalCommandId, expectedVersion: String(expectedVersion || ''),
      reloadTabs: Boolean(reloadTabs), tabCount: tabs.length, sourceTabId,
    });
    void reloadAfterAcceptanceAck({ tabId: sourceTabId, commandId: terminalCommandId, operationId, pending })
      .catch((error) => console.error('[chatgpt-bridge] extension reload acceptance barrier failed', error));
    return {
      operationId,
      scheduled: true,
      reloadTabs,
      tabCount: tabs.length,
      preservedLaunchCount: Object.keys(launchRecords).length,
      trampolinePlannedCount: Object.keys(trampolineUrls).length,
      expectedVersion,
    };
  }

  return Object.freeze({ recoverPendingExtensionReload, scheduleExtensionReload });
}
