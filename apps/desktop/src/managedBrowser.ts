export type ManagedBrowserAction = 'chrome.managed.status' | 'chrome.managed.start' | 'chrome.managed.login' | 'chrome.managed.recover';
export type ManagedBrowserStatus = {
  schemaVersion: 'managed-browser/v1';
  ok: boolean;
  state: string;
  reason: string;
  message: string;
  installed: boolean;
  running: boolean;
  connected: boolean;
  authenticated: boolean;
  ready: boolean;
  extensionReady: boolean;
  process: { pid?: number; owned: boolean; running: boolean };
  profileId?: string;
  requiresLogin?: boolean;
  newBinding: false;
  modelSubmissions: 0;
  videoSubmissions: 0;
};

export function managedBrowserStatus(value: unknown): ManagedBrowserStatus {
  const status = value as ManagedBrowserStatus | null;
  if (!status || status.schemaVersion !== 'managed-browser/v1'
    || ['ok', 'installed', 'running', 'connected', 'authenticated', 'ready', 'extensionReady'].some(key => typeof status[key as keyof ManagedBrowserStatus] !== 'boolean')
    || typeof status.state !== 'string' || !status.state || typeof status.reason !== 'string' || typeof status.message !== 'string'
    || !status.process || typeof status.process.owned !== 'boolean' || typeof status.process.running !== 'boolean'
    || status.process.pid !== undefined && (!Number.isSafeInteger(status.process.pid) || status.process.pid <= 0)
    || status.requiresLogin !== undefined && typeof status.requiresLogin !== 'boolean'
    || status.profileId !== undefined && (typeof status.profileId !== 'string' || !/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/.test(status.profileId))
    || status.newBinding !== false || status.modelSubmissions !== 0 || status.videoSubmissions !== 0) {
    throw new Error('专属浏览器状态无效，请重新检查。');
  }
  return status;
}

export function managedBrowserReady(status: ManagedBrowserStatus | null): boolean {
  return !!status && status.state === 'ready' && status.ok && status.ready && status.installed
    && status.running && status.process.running && status.process.owned && status.extensionReady
    && status.connected && status.authenticated && status.requiresLogin !== true && !!status.profileId;
}
