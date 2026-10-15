import { makeRequestId } from '../../protocol.js';
import {
  BROWSER_LAUNCH_TOKEN_RE,
  browserLaunchMetadataFromUrl,
  browserLaunchUrl,
  safeChatGptUrl,
} from '../../browserLaunch.js';
import { normalizeLaunchedClient } from '../clientSelection.js';

const EXTENSION_ORIGIN_RE = /^chrome-extension:\/\/[a-p]{32}$/i;
const MAINTENANCE_RELOAD_CONFIRMATION = 'chatgpt-bridge-maintenance-reload-v1';

function extensionMaintenanceReloadUrl(client = {}, options = {}) {
  const origin = String(client.origin || '').replace(/\/$/, '');
  if (!EXTENSION_ORIGIN_RE.test(origin)) return '';
  const url = new URL(`${origin}/maintenance-reload.html`);
  url.searchParams.set('confirm', MAINTENANCE_RELOAD_CONFIRMATION);
  url.searchParams.set('expectedVersion', String(options.expectedVersion || ''));
  url.searchParams.set('reloadTabs', options.reloadTabs === false ? '0' : '1');
  url.searchParams.set('serverUrl', String(options.serverUrl || ''));
  url.searchParams.set('commandId', String(options.commandId || `maintenance-${makeRequestId()}`));
  if (Number.isInteger(client.browserTabId)) url.searchParams.set('sourceTabId', String(client.browserTabId));
  if (BROWSER_LAUNCH_TOKEN_RE.test(String(client.launchToken || ''))) url.searchParams.set('sourceLaunchToken', String(client.launchToken));
  if (String(client.url || '')) url.searchParams.set('requestedUrl', String(client.url));
  return url.toString();
}

/**
 * Owns server-side browser tab operations and extension reload handoff. Prompt
 * selection remains in BrowserClientCoordinator and calls this controller only
 * after deciding that a new tab is required.
 */
export class BrowserTabCoordinator {
  constructor({ hub, runtimeOptions, sendCommand, rankClients }) {
    this.hub = hub;
    this.runtimeOptions = runtimeOptions;
    this.sendCommand = sendCommand;
    this.rankClients = rankClients;
  }

  browserControlClients() {
    return Array.from(this.hub.clients || []).filter((client) => client?.ready
      && client.compatible !== false
      && client.compatibility?.compatible !== false
      && client.capabilities?.browserTabs === true);
  }

  browserControlClient(options = {}) {
    const explicitClientId = String(options.sourceClientId || options.clientId || '').trim();
    const clients = this.browserControlClients();
    if (explicitClientId) {
      const explicit = clients.find((client) => client.id === explicitClientId);
      if (!explicit) throw new Error(`Browser extension client cannot control tabs: ${explicitClientId}`);
      return explicit;
    }
    const active = this.hub.activeClient;
    if (active && clients.some((client) => client.id === active.id)) return active;
    if (clients.length === 1) return clients[0];
    throw new Error(clients.length
      ? 'Multiple extension clients can control tabs. Select one with /tab <clientId>.'
      : 'No connected extension client supports browser tab control.');
  }

  async waitForBrowserClient(predicate, timeoutMs = 20_000, { observeNavigation = false } = {}) {
    const existing = Array.from(this.hub.clients || []).find(predicate);
    if (existing) return existing;
    return await new Promise((resolve, reject) => {
      let settled = false;
      const finish = (err, client = null) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        this.hub.off?.('client.ready', handler);
        if (observeNavigation) {
          this.hub.off?.('client.changed', handler);
          this.hub.off?.('client.activity', activityHandler);
        }
        if (err) reject(err);
        else resolve(client);
      };
      const handler = (client) => {
        if (!predicate(client)) return;
        finish(null, client);
      };
      const activityHandler = (event) => handler(event?.client);
      this.hub.on?.('client.ready', handler);
      if (observeNavigation) {
        this.hub.on?.('client.changed', handler);
        this.hub.on?.('client.activity', activityHandler);
      }
      const timer = setTimeout(() => finish(new Error(`Timed out waiting for the new ChatGPT browser tab after ${timeoutMs}ms`)), Math.max(250, Number(timeoutMs) || 20_000));
      timer.unref?.();
      handler();
    });
  }

  async waitForBrowserControlClient(timeoutMs = 0) {
    const existing = this.rankClients(this.browserControlClients())[0] || null;
    if (existing || timeoutMs <= 0) return existing;
    try {
      return await this.waitForBrowserClient(
        (client) => client?.ready
          && client.compatible !== false
          && client.compatibility?.compatible !== false
          && client.capabilities?.browserTabs === true,
        timeoutMs,
      );
    } catch {
      return null;
    }
  }

  async openSystemBrowserTab({ url, launchToken, timeoutMs, bridgeServerUrl, allowIncompatibleClient = false }) {
    const targetUrl = browserLaunchUrl(url, launchToken, { bridgeServerUrl: bridgeServerUrl || this.runtimeOptions.publicBaseUrl });
    await this.runtimeOptions.openExternalUrl(targetUrl);
    const client = await this.waitForBrowserClient(
      (candidate) => candidate?.ready
        && ((candidate.compatible !== false && candidate.compatibility?.compatible !== false)
          || (allowIncompatibleClient && Number(candidate.extensionProtocolVersion) === 5))
        && (candidate.launchToken === launchToken || browserLaunchMetadataFromUrl(candidate.url).launchToken === launchToken),
      timeoutMs,
    ).catch((err) => {
      const observed = Array.from(this.hub.clients || []).map((candidate) => {
        const urlToken = browserLaunchMetadataFromUrl(candidate.url).launchToken;
        return `${candidate.id || 'unknown'} url=${candidate.url || '(empty)'} reportedToken=${candidate.launchToken ? 'yes' : 'no'} urlToken=${urlToken ? 'yes' : 'no'} extension=${candidate.extensionVersion || '?'} content=${candidate.clientVersion || '?'}`;
      });
      const suffix = observed.length ? ` Observed clients: ${observed.join('; ')}` : ' No clients connected to this bridge instance.';
      throw new Error(`${err.message}. The default browser must have ChatGPT Bridge extension 2.3.14 with content runtime 4.3.12 installed and configured for this server. Protocol 5 is required; clients that do not complete its handshake are rejected. Reload the unpacked extension and then reload the ChatGPT tab.${suffix}`);
    });
    const launchedClient = normalizeLaunchedClient(client, launchToken);
    return {
      tabId: launchedClient.browserTabId ?? null,
      launchToken,
      requestedUrl: url,
      targetUrl,
      active: true,
      openedBy: 'system',
      sourceClientId: '',
      client: launchedClient,
    };
  }

  async openBrowserTab(options = {}) {
    const url = safeChatGptUrl(options.url || 'https://chatgpt.com/');
    const launchToken = String(options.launchToken || `bridge-tab-${makeRequestId()}`);
    const timeoutMs = Math.max(5_000, Number(options.timeoutMs) || this.runtimeOptions.autoOpenTabTimeoutMs || 30_000);
    const explicitClientId = String(options.sourceClientId || options.clientId || '').trim();
    let source = null;

    if (explicitClientId) {
      source = this.browserControlClient({ sourceClientId: explicitClientId });
    } else {
      source = this.rankClients(this.browserControlClients())[0] || null;
      if (!source && options.allowSystemFallback) {
        const bootstrapWaitMs = Math.max(0, Math.min(timeoutMs, Number(options.bootstrapWaitMs ?? this.runtimeOptions.autoOpenTabBootstrapWaitMs) || 0));
        source = await this.waitForBrowserControlClient(bootstrapWaitMs);
      }
      if (!source && !options.allowSystemFallback) source = this.browserControlClient(options);
    }

    if (!source) return await this.openSystemBrowserTab({
      url,
      launchToken,
      timeoutMs,
      bridgeServerUrl: options.bridgeServerUrl || this.runtimeOptions.publicBaseUrl,
      allowIncompatibleClient: options.allowIncompatibleClient === true,
    });

    const response = await this.sendCommand('browser.tab.open', {
      url,
      active: options.active !== false,
      launchToken,
      timeoutMs,
      bridgeServerUrl: options.bridgeServerUrl || this.runtimeOptions.publicBaseUrl,
    }, { sourceClientId: source.id, timeoutMs: Math.min(timeoutMs, 15_000) });
    // A launch hello may arrive before navigation finishes. Bind readiness to
    // the native creation receipt and the full requested URL, not just a token.
    if (!Number.isSafeInteger(response?.tabId) || response.tabId <= 0
      || response.launchToken !== launchToken || response.requestedUrl !== url) {
      throw Object.assign(new Error('Created browser tab receipt does not match the requested page; preserving the original launch without reopening'), {
        code: 'BROWSER_TAB_OPEN_RECEIPT_MISMATCH',
      });
    }
    const client = await this.waitForBrowserClient(
      (candidate) => candidate?.ready
        && candidate.compatible !== false
        && candidate.compatibility?.compatible !== false
        && candidate.browserTabId === response.tabId
        && candidate.launchToken === launchToken
        && candidate.url === url,
      timeoutMs,
      { observeNavigation: true },
    );
    return { ...response, launchToken, client: normalizeLaunchedClient(client, launchToken), sourceClientId: source.id, openedBy: 'extension' };
  }

  async closeBrowserTab(options = {}) {
    const sourceClientId = String(options.sourceClientId || options.clientId || '').trim();
    if (!sourceClientId) throw new Error('sourceClientId is required to close a browser tab safely');
    return await this.sendCommand('browser.tab.close', {
      expectedLaunchToken: String(options.expectedLaunchToken || ''),
      expectedUrl: String(options.expectedUrl || ''),
      timeoutMs: Number(options.timeoutMs) || 10_000,
    }, { sourceClientId, timeoutMs: Number(options.timeoutMs) || 10_000 });
  }

  async reloadExtension(options = {}) {
    const sourceClientId = String(options.sourceClientId || options.clientId || '');
    const source = sourceClientId
      ? (this.hub.clients || []).find((client) => client.id === sourceClientId)
      : this.hub.activeClient;
    const before = source ? { ...source } : null;
    if (!before?.id) throw new Error('No browser extension client is available for reload');
    if (!Number.isInteger(before.browserTabId) || before.browserTabId <= 0
      || !BROWSER_LAUNCH_TOKEN_RE.test(String(before.launchToken || ''))
      || ['browserProfileId', 'extensionBundleId', 'serverInstanceId', 'backgroundEpoch', 'contentEpoch']
        .some(key => typeof before[key] !== 'string' || !before[key])) {
      const error = new Error('Extension reload requires the original tab profile, bundle, server and runtime epochs');
      error.code = 'EXTENSION_RELOAD_SOURCE_IDENTITY_REQUIRED';
      throw error;
    }
    const expectedVersion = String(options.expectedVersion || '');
    const expectedBundleId = String(options.expectedBundleId || '');
    const timeoutMs = Math.max(2_000, Number(options.timeoutMs) || 20_000);
    const requestedAt = Date.now();
    const reloadServerUrl = options.serverUrl || this.runtimeOptions.publicBaseUrl;
    const maintenanceBootstrapUrl = options.allowMaintenancePageBootstrap === true && options.ownedTabsOnly !== true && !before.activeRequest?.requestId
      ? extensionMaintenanceReloadUrl(before, {
          expectedVersion,
          reloadTabs: options.reloadTabs !== false,
          serverUrl: reloadServerUrl,
        })
      : '';
    let cancelWait = () => {};
    const reconnectPromise = new Promise((resolve, reject) => {
      const check = (client) => {
        if (!client?.ready) return false;
        if (expectedVersion && String(client.extensionVersion || '') !== expectedVersion) return false;
        if (String(client.extensionBundleId || '') !== (expectedBundleId || before.extensionBundleId)) return false;
        return client.browserTabId === before.browserTabId
          && client.browserProfileId === before.browserProfileId
          && client.serverInstanceId === before.serverInstanceId
          && client.launchToken === before.launchToken
          && typeof client.backgroundEpoch === 'string' && !!client.backgroundEpoch
          && client.backgroundEpoch !== before.backgroundEpoch
          && typeof client.contentEpoch === 'string' && !!client.contentEpoch
          && client.contentEpoch !== before.contentEpoch;
      };
      const handler = (client) => {
        if (!check(client)) return;
        cleanup();
        resolve(client);
      };
      const timer = setTimeout(() => {
        cleanup();
        reject(new Error(`Timed out waiting for extension ${expectedVersion || '(any version)'} to reconnect after reload`));
      }, timeoutMs);
      timer.unref?.();
      const cleanup = () => { clearTimeout(timer); this.hub.off?.('client.ready', handler); };
      cancelWait = cleanup;
      this.hub.on?.('client.ready', handler);
      const existing = (this.hub.clients || []).find((client) => check(client) && Date.parse(client.connectedAt || 0) >= requestedAt);
      if (existing) {
        cleanup();
        resolve(existing);
      }
    });

    // Prefer the canonical Protocol 5 reload command. Opening a chrome-extension://
    // maintenance page through the operating system is only a compatibility
    // fallback: process spawn does not prove that the browser accepted that URL,
    // and making it the primary path caused updates to stall before any command
    // reached the already-connected extension.
    const commandPromise = this.sendCommand('extension.reload', {
      reloadTabs: options.reloadTabs !== false,
      ownedTabsOnly: options.ownedTabsOnly === true,
      expectedVersion,
      sourceTabId: Number.isInteger(before.browserTabId) ? before.browserTabId : null,
      sourceLaunchToken: BROWSER_LAUNCH_TOKEN_RE.test(String(before.launchToken || '')) ? before.launchToken : '',
      temporaryServerUrl: String(reloadServerUrl || ''),
      connection: { serverUrl: reloadServerUrl },
      pageReloadDelayMs: 2_500,
    }, {
      sourceClientId: before.id,
      timeoutMs: Math.min(timeoutMs, 8_000),
      allowIncompatibleReload: true,
    });
    const first = await Promise.race([
      commandPromise.then((value) => ({ kind: 'command', value }), (error) => ({ kind: 'command_error', error })),
      reconnectPromise.then((value) => ({ kind: 'reconnected', value }), (error) => ({ kind: 'reconnect_error', error })),
    ]);
    if (first.kind === 'reconnected') {
      commandPromise.catch(() => {});
      return {
        accepted: { scheduled: true, inferredFromReconnect: true },
        reconnected: first.value,
        recovery: { used: false, reason: 'reconnected_before_terminal_result' },
      };
    }
    if (first.kind === 'reconnect_error') {
      commandPromise.catch(() => {});
      throw first.error;
    }
    if (first.kind === 'command_error') {
      if (!maintenanceBootstrapUrl) {
        cancelWait();
        reconnectPromise.catch(() => {});
        throw first.error;
      }
      try {
        await this.runtimeOptions.openExternalUrl(maintenanceBootstrapUrl, { allowExtensionMaintenance: true });
        return {
          accepted: { scheduled: true, bootstrapPage: true, inferredFromReconnect: true, commandError: first.error?.message || String(first.error) },
          reconnected: await reconnectPromise,
          recovery: { used: true, reason: 'maintenance_page_after_command_failure' },
        };
      } catch (error) {
        cancelWait();
        reconnectPromise.catch(() => {});
        const wrapped = new Error(`Extension reload command failed and the maintenance bootstrap did not reconnect: ${error?.message || error}`);
        wrapped.code = 'EXTENSION_RELOAD_AND_BOOTSTRAP_FAILED';
        wrapped.cause = first.error;
        throw wrapped;
      }
    }

    const accepted = first.value;
    const pageReloadArmed = accepted?.pageReload?.armed === true;
    const recoveryWakeArmed = accepted?.recoveryWake?.armed === true || accepted?.recoveryAlarm?.armed === true;
    const trampolinePlanned = accepted?.reloadTrampoline?.planned === true;
    const armedPageDelayMs = Math.max(300, Number(accepted?.pageReload?.delayMs) || 2_500);
    const desiredGraceMs = trampolinePlanned
      ? Math.max(20_000, armedPageDelayMs + 12_000)
      : pageReloadArmed ? armedPageDelayMs + 8_000
        : recoveryWakeArmed ? 12_000 : 2_000;
    const graceMs = Math.max(1_000, Math.min(timeoutMs - 750, desiredGraceMs));
    const originalReconnect = await Promise.race([
      reconnectPromise,
      new Promise((resolve) => setTimeout(() => resolve(null), graceMs)),
    ]);
    if (originalReconnect) return { accepted, reconnected: originalReconnect };

    // Accepted runtime reload is an existing side effect, even when no content
    // client reconnects. Owned-only/background recovery must retain that outcome
    // rather than spawn an OS browser or replace its original work page.
    if(options.ownedTabsOnly===true || options.allowMaintenancePageBootstrap!==true) {
      cancelWait();
      reconnectPromise.catch(()=>{});
      const error=new Error('Extension reload was accepted, but its original owned page has not reconnected; no system browser was opened');
      error.code='EXTENSION_RELOAD_RECONNECT_PENDING';
      error.reloadAccepted=true;
      error.accepted=accepted;
      error.recovery={used:false,reason:'original_owned_reconnect_pending'};
      throw error;
    }

    // A successful command result only proves that the old service worker
    // scheduled its own restart. It does not prove that Chrome injected the
    // updated content script into the already-open ChatGPT tab. Older bundles
    // could acknowledge the command and then disappear before their page timer
    // fired, leaving the user with a stale page that required a manual refresh.
    // Open the deployed maintenance page after a short reconnect grace even
    // when the protocol command succeeded. The page runs in the new extension
    // package and can reload the source tab independently of the dead worker.
    let maintenanceBootstrapError = null;
    if (maintenanceBootstrapUrl) {
      try {
        await this.runtimeOptions.openExternalUrl(maintenanceBootstrapUrl, { allowExtensionMaintenance: true });
        const maintenanceGraceMs = Math.max(750, Math.min(10_000, timeoutMs - (Date.now() - requestedAt) - 500));
        const maintenanceReconnect = await Promise.race([
          reconnectPromise,
          new Promise((resolve) => setTimeout(() => resolve(null), maintenanceGraceMs)),
        ]);
        if (maintenanceReconnect) {
          return {
            accepted: { ...accepted, bootstrapPage: true },
            reconnected: maintenanceReconnect,
            recovery: { used: true, reason: 'maintenance_page_after_stalled_command' },
          };
        }
      } catch (error) {
        maintenanceBootstrapError = error;
      }
    }

    // Another tab cannot prove that this tab loaded a new extension runtime.
    // Preserve the accepted operation instead of replacing or closing its page.
    cancelWait();
    reconnectPromise.catch(() => {});
    const error = new Error('Extension reload was accepted, but the original tab has no verified new background and content runtime');
    error.code = 'EXTENSION_RELOAD_RECONNECT_PENDING';
    error.reloadAccepted = true;
    error.accepted = accepted;
    error.recovery = { used: Boolean(maintenanceBootstrapUrl), reason: 'original_runtime_reconnect_pending' };
    if (maintenanceBootstrapError) error.cause = maintenanceBootstrapError;
    throw error;
  }
}
