import { request } from './api';
import type { Settings } from './types';

export type RuntimeProvider = 'codex_cli' | 'codex_sdk' | 'dreamina';
export type SettingsRequestScope = { scope: 'global' } | { projectId: string };
export type RuntimeComponent = { id: 'codex' | 'node' | 'dreamina'; name: string; state: 'installed' | 'missing' | 'invalid'; version?: string | null; path?: string; message?: string };
export type RuntimeStatus = { schemaVersion: 'runtime-status/v1'; provider: RuntimeProvider; installed: boolean; installSupported: boolean; components: RuntimeComponent[]; settings: Partial<Settings>; checkedAt: string };

/** Software settings have an explicit scope; no synthetic project is required. */
export function settingsRequestScope(value: SettingsRequestScope | string): SettingsRequestScope {
  if (typeof value === 'string' && value.trim()) return { projectId: value };
  if (value && typeof value === 'object' && Object.keys(value).length === 1) {
    if ('scope' in value && value.scope === 'global') return { scope: 'global' };
    if ('projectId' in value && typeof value.projectId === 'string' && value.projectId.trim()) return { projectId: value.projectId };
  }
  throw new Error('设置范围无效，请重新打开设置页面。');
}
const pathKeys: Record<RuntimeProvider, string[]> = {
  codex_cli: ['codexPath'], codex_sdk: ['codexPath', 'textSdkNodePath'], dreamina: ['dreaminaPath'],
};
const validPath = (value: unknown): value is string => typeof value === 'string' && !!value.trim()
  && /^(?:[A-Za-z]:[\\/]|\/|\\\\)/.test(value) && !/[\x00-\x1f]/.test(value);

export function runtimeStatus(value: unknown, provider: RuntimeProvider): RuntimeStatus {
  const response = value as RuntimeStatus | null, expectedIds = provider === 'codex_sdk' ? ['node', 'codex'] : [provider === 'codex_cli' ? 'codex' : 'dreamina'];
  const componentKey = { codex: 'codexPath', node: 'textSdkNodePath', dreamina: 'dreaminaPath' };
  const invalid = () => new Error('组件检测结果无效，请重新检查。');
  if (!response || response.schemaVersion !== 'runtime-status/v1' || response.provider !== provider
      || typeof response.installed !== 'boolean' || typeof response.installSupported !== 'boolean'
      || typeof response.checkedAt !== 'string' || !Number.isFinite(Date.parse(response.checkedAt))
      || !Array.isArray(response.components) || response.components.length !== expectedIds.length
      || !response.settings || typeof response.settings !== 'object' || Array.isArray(response.settings)
      || Object.keys(response.settings).some(key => !pathKeys[provider].includes(key) || !validPath(response.settings[key]))) throw invalid();
  const ids = new Set<string>();
  for (const component of response.components) {
    if (!component || !expectedIds.includes(component.id) || ids.has(component.id) || typeof component.name !== 'string'
        || !['installed', 'missing', 'invalid'].includes(component.state)
        || (component.version != null && typeof component.version !== 'string')
        || (component.message !== undefined && typeof component.message !== 'string')) throw invalid();
    ids.add(component.id);
    const key = componentKey[component.id];
    if (component.state === 'installed' ? !validPath(component.path) || component.path !== response.settings[key]
      : component.path !== undefined || response.settings[key] !== undefined) throw invalid();
  }
  if (response.installed !== response.components.every(component => component.state === 'installed')
      || response.installSupported && response.components.some(component => component.state === 'invalid')) throw invalid();
  return response;
}

/** Inventory only. The backend never starts a program, reads an account or installs. */
export async function readRuntimeStatus(target: SettingsRequestScope | string, provider: RuntimeProvider, settings: Settings): Promise<RuntimeStatus> {
  const paths = Object.fromEntries(pathKeys[provider].map(key => [key, String(settings[key] || '')]));
  return runtimeStatus(await request({ action: 'runtime.status', ...settingsRequestScope(target), provider, settings: paths }), provider);
}

/** Only resolved executable paths may flow back into settings. Never accept credentials or model changes. */
export function runtimeSettingsPatch(value: unknown, provider: RuntimeProvider): Partial<Settings> {
  const response = value as { schemaVersion?: unknown; provider?: unknown; settings?: unknown } | null;
  if (!response || response.schemaVersion !== 'runtime-prepare/v1' || response.provider !== provider
      || !response.settings || typeof response.settings !== 'object' || Array.isArray(response.settings)) {
    throw new Error('程序配置结果无效，请重新连接。');
  }
  const settings = response.settings as Record<string, unknown>, keys = pathKeys[provider];
  if (Object.keys(settings).length !== keys.length || keys.some(key => typeof settings[key] !== 'string'
      || !String(settings[key]).trim() || !/^(?:[A-Za-z]:[\\/]|\/|\\\\)/.test(String(settings[key]))
      || /[\x00-\x1f]/.test(String(settings[key])))) throw new Error('程序未能完成自动配置，请重新连接。');
  return Object.fromEntries(keys.map(key => [key, settings[key]])) as Partial<Settings>;
}

export async function prepareRuntime(target: SettingsRequestScope | string, provider: RuntimeProvider, settings: Settings): Promise<Partial<Settings>> {
  const paths = Object.fromEntries(pathKeys[provider].map(key => [key, String(settings[key] || '')]));
  return runtimeSettingsPatch(await request({ action: 'runtime.prepare', ...settingsRequestScope(target), provider, settings: paths }), provider);
}
