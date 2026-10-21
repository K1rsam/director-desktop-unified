/** Native capabilities come from the Rust core, with a separate isolated policy.
 * Backend dispatch independently enforces feature and recovery boundaries.
 */
export const READINESS_PROVIDERS = ['codex_cli', 'dreamina', 'dreamina_canvas', 'codex_sdk'] as const;
export type ReadinessProvider = typeof READINESS_PROVIDERS[number];
export type ReadinessCapability = { version: 'provider-readiness/v1'; enabled: boolean; providers: ReadinessProvider[]; generationAllowed: false };
export type TextBackendCapability =
  | { id: 'codex_cli'; status: 'enabled' | 'preserved_not_enabled' }
  | { id: 'codex_sdk'; status: 'explicit_selection_when_ready' | 'offline_contract_only' };

export type ExecutionPolicy = {
  mode: 'offline_prepare_only' | 'native_cli_acceptance' | 'standard_native';
  liveExecutionAllowed: boolean;
  stage3CreativePackStatus: 'inactive' | 'verified_active';
  allowedActions?: string[];
  acceptance?: Record<string, unknown>;
  reasons: string[];
  readiness?: ReadinessCapability;
  canvasOAuth?: { version: 'canvas-oauth/v1'; enabled: boolean };
  textBackends?: TextBackendCapability[];
};
export type PolicyRequest = { action: string; [key: string]: unknown };
export type ActionCategory = 'local_read' | 'local_write' | 'generation' | 'readiness' | 'provider' | 'credentials' | 'unknown';

export const OFFLINE_EXECUTION_POLICY: ExecutionPolicy = {
  mode: 'offline_prepare_only',
  liveExecutionAllowed: false,
  stage3CreativePackStatus: 'inactive',
  reasons: [
    '当前仅开放本地资料准备与离线验证，产品端真实文本、图片和视频生成保持关闭。',
    '第三阶段创作原件包尚未启用，须完成实际采用来源与适配核验。',
    '渠道、CLI、账户登录及凭据操作保持关闭；视频素材准备尚未形成独立离线路径。',
  ],
};

let activePolicy: ExecutionPolicy = OFFLINE_EXECUTION_POLICY;
const policyListeners = new Set<() => void>();
export function currentExecutionPolicy(): ExecutionPolicy { return activePolicy; }
export function subscribeExecutionPolicy(listener: () => void): () => void {
  policyListeners.add(listener); return () => { policyListeners.delete(listener); };
}
export function installExecutionPolicy(policy: ExecutionPolicy): void {
  activePolicy = { ...policy, readiness: validateReadinessCapability(policy.readiness), textBackends: validateTextBackends(policy.textBackends, policy.mode), reasons: [...policy.reasons], allowedActions: policy.allowedActions ? [...policy.allowedActions] : undefined };
  for (const listener of policyListeners) listener();
}
const NATIVE_EXTRA_ACTIONS = new Set(['foundation.run', 'images.design', 'workflows.resume', 'textCli.status', 'storyboard.run', 'storyboard.decision.resolve']);

const LOCAL_READ_ACTIONS = new Set([
  'getExecutionPolicy', 'settings.global.get', 'runtime.status', 'chrome.status', 'chrome.managed.status', 'textSdk.status', 'images.previewPrompt', 'projects.list', 'files.choose', 'legacy.inspect', 'workspace.get',
  'units.get', 'units.versions.list', 'foundation.choices.list', 'assets.get', 'assets.usage', 'jobs.detail', 'storyboard.refinement.continuity.preview',
]);
const LOCAL_WRITE_ACTIONS = new Set([
  'projects.create', 'projects.archive', 'projects.restore', 'projects.delete',
  'legacy.import', 'project.import', 'project.export', 'units.version.restore', 'units.save',
  'production.direction.save', 'foundation.choice.confirm', 'assets.save', 'assets.delete',
  'assets.restore', 'jobs.close_unknown', 'jobs.archive', 'jobs.restore', 'source.import', 'source.resolve', 'source.reparse',
  'media.import', 'media.adopt', 'media.adopt_batch', 'images.ensure_current', 'videos.adopt',
  'settings.save', 'settings.global.save', 'backup.create', 'backup.restore',
]);
const GENERATION_ACTIONS = new Set([
  'foundation.run', 'foundation.choices.apply', 'images.design', 'images.produce', 'images.run',
  'storyboard.run', 'storyboard.decision.resolve', 'units.regenerate', 'storyboard.refine', 'workflows.resume', 'videos.submit',
]);
const PROVIDER_ACTIONS = new Set([
  'runtime.prepare', 'chrome.connect', 'chrome.check', 'chrome.open', 'chrome.bind', 'chrome.managed.start', 'chrome.managed.recover', 'providers.refresh', 'videos.preflight', 'videos.prepare', 'jobs.query', 'jobs.download', 'jobs.reconcile',
]);
const CREDENTIAL_ACTIONS = new Set([
  'chrome.managed.login',
  'canvas.login', 'canvas.loginStatus', 'canvas.openAuthorization', 'canvas.cancelLogin', 'canvas.logout',
  'textCli.status', 'textCli.login', 'textCli.loginStatus', 'textCli.cancelLogin', 'textCli.openLogin',
  'dreamina.status', 'dreamina.login', 'dreamina.logout', 'dreamina.open_authorization',
]);
const STANDARD_ACTIONS = new Set([...LOCAL_READ_ACTIONS, ...LOCAL_WRITE_ACTIONS,
  ...GENERATION_ACTIONS, ...PROVIDER_ACTIONS, ...CREDENTIAL_ACTIONS, 'readiness.check', 'storyboard.refinement.adopt']);
const STAGE3_ACTIONS = new Set(['storyboard.run', 'storyboard.decision.resolve', 'units.regenerate', 'storyboard.refine', 'storyboard.refinement.adopt']);
export const CREDENTIAL_SETTING_KEYS = ['bhToken', 'chromeToken', 'textApiKey'] as const;

export function classifyAction(input: PolicyRequest | string): ActionCategory {
  const request = typeof input === 'string' ? { action: input } : input;
  if (['settings.save', 'settings.global.save'].includes(request.action) && request.settings && typeof request.settings === 'object') {
    const settings = request.settings as Record<string, unknown>;
    if (CREDENTIAL_SETTING_KEYS.some(key => settings[key] != null && settings[key] !== '')) return 'credentials';
  }
  if (request.action === 'readiness.check') return 'readiness';
  if (LOCAL_READ_ACTIONS.has(request.action)) return 'local_read';
  if (LOCAL_WRITE_ACTIONS.has(request.action)) return 'local_write';
  if (GENERATION_ACTIONS.has(request.action)) return 'generation';
  if (PROVIDER_ACTIONS.has(request.action)) return 'provider';
  if (CREDENTIAL_ACTIONS.has(request.action)) return 'credentials';
  return 'unknown';
}

export function actionBlockReason(input: PolicyRequest | string): string | null {
  const action = typeof input === 'string' ? input : input.action;
  const category = classifyAction(input);
  if (['canvas.login', 'canvas.loginStatus', 'canvas.openAuthorization', 'canvas.cancelLogin', 'canvas.logout'].includes(action)
      && activePolicy.canvasOAuth?.enabled === true) return null;
  if (activePolicy.mode === 'standard_native') {
    if (activePolicy.allowedActions?.includes(action)) return null;
    if (STAGE3_ACTIONS.has(action)) return '第三阶段暂不可生成：最新 3008 运行装配尚未接入，已有分镜和历史保留。';
    return '本机核心尚未开放此操作，请检查对应渠道或功能状态。';
  }
  if (activePolicy.mode === 'native_cli_acceptance' && NATIVE_EXTRA_ACTIONS.has(action) && activePolicy.allowedActions?.includes(action)) return null;
  switch (category) {
    case 'local_read': case 'local_write': return null;
    case 'readiness': {
      const capability = activePolicy.readiness;
      const provider = typeof input === 'string' ? undefined : input.provider;
      if (capability?.enabled && capability.providers.length > 0
          && (typeof input === 'string' || capability.providers.includes(provider as ReadinessProvider))) return null;
      return '离线准备模式：当前环境未开放手动只读连接检查；真实生成与登录操作保持关闭。';
    }
    case 'generation': return '离线准备模式：产品端真实文本、图片与视频生成尚未开放。';
    case 'provider': return '离线准备模式：渠道访问、远端查询与渠道素材准备尚未开放。';
    case 'credentials': return '离线准备模式：CLI、账户登录与凭据读写尚未开放。';
    default: return '离线准备模式：此操作尚未纳入已核验的本地操作范围。';
  }
}

export function assertOfflineAction(input: PolicyRequest): void {
  const reason = actionBlockReason(input);
  if (reason) throw new Error(`${reason}（${input.action}）`);
}

function validateReadinessCapability(value: unknown): ReadinessCapability | undefined {
  if (value === undefined) return;
  if (!value || typeof value !== 'object') throw new Error('只读检查策略无效。');
  const capability = value as Record<string, unknown>;
  if (capability.version !== 'provider-readiness/v1' || typeof capability.enabled !== 'boolean'
      || capability.generationAllowed !== false || !Array.isArray(capability.providers)
      || !capability.providers.every(provider => READINESS_PROVIDERS.includes(provider as ReadinessProvider))
      || new Set(capability.providers).size !== capability.providers.length) throw new Error('只读检查策略无效；外部执行保持关闭。');
  return { version: 'provider-readiness/v1', enabled: capability.enabled, providers: [...capability.providers] as ReadinessProvider[], generationAllowed: false };
}

function validateTextBackends(value: unknown, mode: unknown): TextBackendCapability[] | undefined {
  if (value === undefined) return;
  if (!Array.isArray(value) || value.length > 2 || new Set(value.map(item => item?.id)).size !== value.length
      || !value.every(item => item && typeof item === 'object'
        && (item.id === 'codex_cli' && ['enabled', 'preserved_not_enabled'].includes(item.status)
          || item.id === 'codex_sdk' && ['explicit_selection_when_ready', 'offline_contract_only'].includes(item.status))
        && (mode === 'standard_native' || item.id === 'codex_cli' && mode === 'native_cli_acceptance'
          || !['enabled', 'explicit_selection_when_ready'].includes(item.status)))) {
    throw new Error('文本入口能力无效，真实 SDK 执行保持关闭。');
  }
  return value.map(item => ({ id: item.id, status: item.status })) as TextBackendCapability[];
}

export function hasSdkExecutionCapability(policy: ExecutionPolicy = currentExecutionPolicy()): boolean {
  return policy.mode === 'standard_native' && policy.liveExecutionAllowed
    && policy.textBackends?.some(backend => backend.id === 'codex_sdk' && backend.status === 'explicit_selection_when_ready') === true;
}

export function validateExecutionPolicy(value: unknown): ExecutionPolicy {
  if (!value || typeof value !== 'object') throw new Error('本地核心未返回有效的执行策略。');
  const policy = value as Record<string, unknown>;
  if (!Array.isArray(policy.reasons) || !policy.reasons.every(reason => typeof reason === 'string')) throw new Error('执行策略缺少说明，外部执行保持关闭。');
  const readiness = validateReadinessCapability(policy.readiness);
  const textBackends = validateTextBackends(policy.textBackends, policy.mode);
  const auth = policy.canvasOAuth as ExecutionPolicy['canvasOAuth'];
  if (auth !== undefined && (!auth || auth.version !== 'canvas-oauth/v1' || typeof auth.enabled !== 'boolean')) throw new Error('即梦授权能力无效。');
  const canvasOAuth = auth ? { version: 'canvas-oauth/v1' as const, enabled: auth.enabled } : undefined;
  if (policy.mode === 'standard_native') {
    const stage3 = policy.stage3CreativePackStatus === 'verified_active';
    const actions = policy.allowedActions;
    if (policy.version !== 'execution-policy/v3' || policy.liveExecutionAllowed !== true
        || !['inactive', 'verified_active'].includes(String(policy.stage3CreativePackStatus))
        || !Array.isArray(actions) || !actions.every(action => typeof action === 'string' && STANDARD_ACTIONS.has(action))
        || new Set(actions).size !== actions.length
        || (!stage3 && actions.some(action => STAGE3_ACTIONS.has(action)))) {
      throw new Error('标准版功能策略无效，未开放外部执行。');
    }
    return { mode: 'standard_native', liveExecutionAllowed: true, stage3CreativePackStatus: stage3 ? 'verified_active' : 'inactive',
      reasons: [...policy.reasons], allowedActions: [...actions], ...(readiness ? { readiness } : {}), ...(canvasOAuth ? { canvasOAuth } : {}), ...(textBackends ? { textBackends } : {}) };
  }
  if (policy.mode === 'offline_prepare_only' && policy.liveExecutionAllowed === false && policy.stage3CreativePackStatus === 'inactive') {
    return { mode: 'offline_prepare_only', liveExecutionAllowed: false, stage3CreativePackStatus: 'inactive', reasons: [...policy.reasons], ...(readiness ? { readiness } : {}), ...(canvasOAuth ? { canvasOAuth } : {}), ...(textBackends ? { textBackends } : {}) };
  }
  const scope = policy.acceptance && typeof policy.acceptance === 'object' ? policy.acceptance as Record<string, unknown> : {};
  const safeActions = new Set([...LOCAL_READ_ACTIONS, ...LOCAL_WRITE_ACTIONS, ...NATIVE_EXTRA_ACTIONS, 'readiness.check']);
  const actions = policy.allowedActions;
  const stage3 = scope.stage3AdapterVerified === true;
  if (policy.version !== 'execution-policy/v2' || policy.mode !== 'native_cli_acceptance' || policy.liveExecutionAllowed !== true
      || scope.backend !== 'codex' || scope.model !== 'gpt-6.1-sol' || scope.reasoning !== 'medium'
      || scope.sdkLiveAllowed !== false || scope.realVideoAllowed !== false || scope.newWorkflowsPerPhase !== 1
      || !/^[a-f0-9]{64}$/.test(String(scope.sourceSha256 || ''))
      || JSON.stringify(scope.storyboardEpisodeNumbers) !== '[1,2]'
      || policy.stage3CreativePackStatus !== (stage3 ? 'verified_active' : 'inactive')
      || !Array.isArray(actions) || !actions.every(action => typeof action === 'string' && safeActions.has(action))
      || (!stage3 && actions.some(action => ['storyboard.run', 'storyboard.decision.resolve'].includes(action)))) {
    throw new Error('本地核心执行策略与有界验收合同不一致；外部执行保持关闭。');
  }
  return { mode: 'native_cli_acceptance', liveExecutionAllowed: true, stage3CreativePackStatus: stage3 ? 'verified_active' : 'inactive', reasons: [...policy.reasons], allowedActions: [...actions], acceptance: { ...scope }, ...(readiness ? { readiness } : {}), ...(canvasOAuth ? { canvasOAuth } : {}), ...(textBackends ? { textBackends } : {}) };
}

/** Keep the guard ahead of IPC, including direct callers that bypass a disabled button. */
export async function dispatchOfflineRequest<T>(input: PolicyRequest, transport: (input: PolicyRequest) => Promise<T>): Promise<T> {
  assertOfflineAction(input);
  const result = await transport(input);
  if (input.action === 'getExecutionPolicy') {
    try { const policy = validateExecutionPolicy(result); installExecutionPolicy(policy); return policy as T; }
    catch (error) { installExecutionPolicy(OFFLINE_EXECUTION_POLICY); throw error; }
  }
  return result;
}
