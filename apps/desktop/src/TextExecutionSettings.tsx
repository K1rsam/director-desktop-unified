import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { request } from './api';
import { prepareRuntime, readRuntimeStatus, settingsRequestScope, type RuntimeStatus, type SettingsRequestScope } from './runtimeSetup';
import './TextExecutionSettings.css';
import { actionBlockReason, currentExecutionPolicy, hasSdkExecutionCapability, subscribeExecutionPolicy } from './executionPolicy';
import { Badge, Button, formatDate, messageOf } from './components';
import type { Settings, TextCliLimitWindow, TextCliLogin, Workspace } from './types';

import { invalidateProviderReadiness, ProviderReadinessResult, sdkReadinessAllowsExecution, useProviderReadiness } from './ProviderReadiness';

type Props = { settings: Settings; writable: boolean; connectionAllowed?: boolean; update: (key: string, value: string) => void; saveSettings: (override?: Partial<Settings>) => Promise<boolean>; onError?: (message: string) => void }
  & ({ requestScope: SettingsRequestScope; workspace?: Workspace } | { workspace: Workspace; requestScope?: SettingsRequestScope });

export function TextExecutionSummary({ settings, onConfigure, stage }: { settings: Settings; onConfigure: () => void; stage?: 'foundation' | 'images' | 'storyboard' }) {
  const policy = useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const action = stage === 'images' ? 'images.design' : stage === 'storyboard' ? 'storyboard.run' : 'foundation.run';
  const cliCapability = policy.textBackends?.find(backend => backend.id === 'codex_cli');
  const enabled = policy.liveExecutionAllowed && ['standard_native', 'native_cli_acceptance'].includes(policy.mode)
    && (!settings.textProvider || settings.textProvider === 'codex') && (!cliCapability || cliCapability.status === 'enabled')
    && policy.allowedActions?.includes(action) === true;
  const sdkAvailable = settings.textProvider === 'codex_sdk' && hasSdkExecutionCapability(policy) && policy.allowedActions?.includes(action) === true;
  const label = enabled ? policy.mode === 'standard_native' ? '标准版 CLI 已开放' : '受控 CLI 验收' : sdkAvailable ? '标准版 SDK · 执行前核验' : '当前关闭';
  return <div className="text-execution-summary"><span>{stage === 'images' ? '新提示词任务设置' : '新文本任务设置'}（{label}）：{settings.textProvider === 'api' ? '文本 API' : settings.textProvider === 'codex_sdk' ? sdkAvailable ? '官方 Codex SDK' : '官方 Codex SDK（离线核验）' : 'Codex CLI'} · {settings.textModel || '未选择模型'}{settings.textProvider !== 'api' ? ` · ${settings.reasoning || 'medium'}` : ''}{stage === 'images' ? '；图片按出图渠道生成' : ''}；继续原流程时沿用原渠道与模型。</span><Button variant="ghost" icon="settings" onClick={onConfigure}>调整执行设置</Button></div>;
}

function windowText(window?: TextCliLimitWindow | null): string | undefined {
  if (!window) return;
  const minutes = window.windowDurationMins;
  const period = minutes && minutes >= 1440 ? `${Math.round(minutes / 1440)} 天` : minutes && minutes >= 60 ? `${Math.round(minutes / 60)} 小时` : minutes ? `${minutes} 分钟` : '当前';
  const remaining = typeof window.remainingPercent === 'number' ? window.remainingPercent : typeof window.usedPercent === 'number' ? 100 - window.usedPercent : undefined;
  return remaining === undefined ? undefined : `${period}额度剩余 ${Math.max(0, Math.min(100, remaining)).toFixed(0)}%`;
}

export function TextExecutionSettings({ workspace, requestScope, settings, writable, connectionAllowed = writable, update, saveSettings }: Props) {
  const policy = useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const [login, setLogin] = useState<TextCliLogin>({ state: 'idle' });
  const [operation, setOperation] = useState<'connecting' | 'installing' | 'login' | null>(null), [error, setError] = useState('');
  const [components, setComponents] = useState<RuntimeStatus | null>(null), [componentBusy, setComponentBusy] = useState(false), [componentError, setComponentError] = useState('');
  const target = settingsRequestScope(requestScope ?? workspace!.project.id), provider = settings.textProvider || 'codex';
  const path = String(settings.codexPath || ''), sdkPath = String(settings.textSdkNodePath || '');
  const operationScope = JSON.stringify([target, provider, connectionAllowed]);
  const runtimeProvider = provider === 'codex_sdk' ? 'codex_sdk' : 'codex_cli';
  const componentScope = JSON.stringify([operationScope, path, provider === 'codex_sdk' ? sdkPath : '']);
  const componentEpoch = useRef(0), currentComponentScope = useRef(componentScope);
  if (currentComponentScope.current !== componentScope) { currentComponentScope.current = componentScope; componentEpoch.current++; }
  const mounted = useRef(true), scope = useRef(operationScope), sequence = useRef(0), inFlight = useRef(false);
  // Identity or permission changes invalidate work. A parent's temporary busy
  // state during our own settings save is only a button lock, not revocation.
  if (scope.current !== operationScope) { scope.current = operationScope; sequence.current++; inFlight.current = false; }
  useEffect(() => { setLogin({ state: 'idle' }); setOperation(null); setError(''); }, [operationScope]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; sequence.current++; inFlight.current = false; }; }, []);
  const sdkAvailable = hasSdkExecutionCapability(policy);
  const cliCheck = useProviderReadiness(target, 'codex_cli', path, connectionAllowed && provider === 'codex');
  const sdkCheck = useProviderReadiness(target, 'codex_sdk', sdkPath, connectionAllowed && provider === 'codex_sdk', typeof settings.codexPath === 'string' ? { codexPath: path } : undefined);
  const activeCheck = provider === 'codex_sdk' ? sdkCheck : cliCheck;
  const automaticSetup = !actionBlockReason('runtime.prepare');
  useEffect(() => {
    setComponents(null); setComponentError('');
    if (!connectionAllowed || !automaticSetup || provider === 'api' || actionBlockReason('runtime.status')) { setComponentBusy(false); return; }
    const epoch = ++componentEpoch.current; let active = true;
    setComponentBusy(true);
    void readRuntimeStatus(target, runtimeProvider, settings).then(result => {
      if (active && epoch === componentEpoch.current) setComponents(result);
    }).catch(reason => { if (active && epoch === componentEpoch.current) setComponentError(messageOf(reason)); })
      .finally(() => { if (active && epoch === componentEpoch.current) setComponentBusy(false); });
    return () => { active = false; };
  }, [componentScope, automaticSetup]);
  const sdkReady = sdkReadinessAllowsExecution(sdkCheck.result, policy);
  const status = cliCheck.result?.details.cliStatus;
  const models = (activeCheck.result?.catalog.state === 'available' ? activeCheck.result.catalog.models : []).map(model => {
    const detail = status?.models?.find(item => item.id === model.id);
    return { id: model.id, displayName: model.name || model.id, supportedReasoningEfforts: model.supportedReasoningEfforts || (provider === 'codex' ? detail?.supportedReasoningEfforts : undefined) || [], defaultReasoningEffort: model.defaultReasoningEffort || (provider === 'codex' ? detail?.defaultReasoningEffort : undefined) || 'medium', isDefault: model.isDefault || (provider === 'codex' && detail?.isDefault) || false };
  });
  const selectedModel = models.find(model => model.id === settings.textModel), efforts = selectedModel?.supportedReasoningEfforts || [], reasoning = settings.reasoning || 'medium';
  const busy = !!operation || activeCheck.busy;
  const connected = components?.installed && (provider === 'codex_sdk' ? sdkReady : activeCheck.result?.installation.state === 'ready'
    && activeCheck.result?.protocol.state === 'ready' && activeCheck.result?.authentication.state === 'authenticated'
    && activeCheck.result.authentication.identityVerified && models.length > 0);
  const needsLogin = activeCheck.result?.authentication.state === 'not_logged_in';
  const validOperation = (attempt: number) => mounted.current && sequence.current === attempt && scope.current === operationScope;

  async function connect(install = false) {
    if (!writable || !connectionAllowed || inFlight.current || busy || activeCheck.blocked) return;
    if (install && (!components || components.installed || !components.installSupported || !automaticSetup)) return;
    // Offline acceptance can inspect existing programs, without install or writes.
    if (!automaticSetup) { await activeCheck.check(); return; }
    const attempt = ++sequence.current; inFlight.current = true; setOperation(install ? 'installing' : 'connecting'); setError(''); activeCheck.reset();
    try {
      const prepared = install ? await prepareRuntime(target, runtimeProvider, settings) : {};
      if (!validOperation(attempt)) return;
      const found = await readRuntimeStatus(target, runtimeProvider, { ...settings, ...prepared });
      if (!validOperation(attempt)) return;
      setComponents(found); setComponentError('');
      if (!found.installed) { setError(found.components.find(component => component.state === 'invalid')?.message || (install ? '组件尚未安装，请重试安装组件。' : '')); return; }
      const patch = found.settings;
      if (!validOperation(attempt)) return;
      if (!await saveSettings(patch) || !validOperation(attempt)) return;
      await activeCheck.checkPrepared(patch);
    } catch (reason) { if (validOperation(attempt)) setError(messageOf(reason)); }
    finally { if (validOperation(attempt)) { inFlight.current = false; setOperation(null); } }
  }
  async function loginAction(action: 'textCli.login' | 'textCli.cancelLogin' | 'textCli.openLogin') {
    if (!writable || !connectionAllowed || inFlight.current || busy || actionBlockReason(action)) return;
    const attempt = ++sequence.current; inFlight.current = true; setOperation('login'); setError('');
    try {
      if (action === 'textCli.login') {
        invalidateProviderReadiness(['codex_cli', 'codex_sdk']);
        activeCheck.reset();
        const found = await readRuntimeStatus(target, runtimeProvider, settings);
        if (!validOperation(attempt)) return;
        setComponents(found); setComponentError('');
        if (!found.installed) { setError('请先安装组件，再登录 OpenAI 账户。'); return; }
        const patch = found.settings;
        if (!validOperation(attempt) || !await saveSettings(patch) || !validOperation(attempt)) return;
      }
      const next = await request<{ login?: TextCliLogin }>({ action, ...target });
      if (validOperation(attempt) && next.login) setLogin(next.login);
    } catch (reason) { if (validOperation(attempt)) setError(messageOf(reason)); }
    finally { if (validOperation(attempt)) { inFlight.current = false; setOperation(null); } }
  }
  useEffect(() => {
    if (login.state !== 'pending' || !connectionAllowed || actionBlockReason('textCli.loginStatus')) return;
    let cancelled = false, attempts = 0, timer: ReturnType<typeof setTimeout>;
    const deadline = Date.now() + 5 * 60 * 1000;
    const expectedScope = operationScope;
    async function poll() {
      if (cancelled || scope.current !== expectedScope) return;
      if (Date.now() >= deadline) { setLogin({ state: 'failed' }); setError('登录等待已超时，可重新登录。'); return; }
      try {
        const next = await request<{ login?: TextCliLogin }>({ action: 'textCli.loginStatus', ...target });
        if (cancelled || scope.current !== expectedScope) return;
        if (!next.login) throw new Error('登录状态暂不可用，请重新检查。');
        if (next.login.state === 'pending') {
          if (++attempts < 150) timer = setTimeout(() => void poll(), 2000);
          else { setLogin({ state: 'failed', error: '登录等待已超时，可重新登录。' }); setError('登录等待已超时，可重新登录。'); }
        } else {
          setLogin(next.login);
          if (next.login.state === 'completed') await activeCheck.check();
          else if (next.login.error) setError(next.login.error);
        }
      } catch (reason) { if (!cancelled && scope.current === expectedScope) { setLogin({ state: 'failed' }); setError(messageOf(reason)); } }
    }
    timer = setTimeout(() => void poll(), 2000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [login.state, login.loginId, operationScope]);
  const modelControls = <div className="form-grid text-model-controls"><label>文本模型<select data-text-model disabled={!writable} value={settings.textModel || ''} onChange={event => { update('textModel', event.target.value); const model = models.find(item => item.id === event.target.value); if (model && !model.supportedReasoningEfforts.includes(reasoning)) update('reasoning', model.defaultReasoningEffort); }}><option value="" disabled>连接后选择模型</option>{settings.textModel && !models.some(model => model.id === settings.textModel) && <option value={settings.textModel}>{settings.textModel} · 待检查</option>}{models.map(model => <option key={model.id} value={model.id}>{model.displayName}{model.isDefault ? ' · 默认' : ''}</option>)}</select></label><label>推理强度<select data-text-reasoning disabled={!writable} value={reasoning} onChange={event => update('reasoning', event.target.value)}>{!efforts.includes(reasoning) && <option value={reasoning}>{reasoning} · 待检查</option>}{efforts.map(effort => <option key={effort} value={effort}>{effort}</option>)}</select></label></div>;
  function componentVersion(component: RuntimeStatus['components'][number]): string | undefined | null {
    if (component.version) return component.version;
    const report = activeCheck.result;
    if (!report) return;
    if (component.id === 'codex' && report.provider === 'codex_cli' && report.installation.executable === component.path) return report.installation.version;
    const field = component.id === 'node' ? 'nodeVersion' : 'runtimeVersion';
    const executable = component.id === 'node' ? report.installation.executable : report.details.runtimeExecutable;
    return report.provider === 'codex_sdk' && executable === component.path && typeof report.details[field] === 'string' ? report.details[field] as string : undefined;
  }
  const connectionLabel = busy ? operation === 'installing' ? '正在安装组件…' : '正在检查连接…' : login.state === 'pending' && !connected ? '等待网页登录' : connected ? '账户与模型已连接' : needsLogin ? '请登录 OpenAI 账户' : error || activeCheck.error ? '连接未完成' : activeCheck.result ? '连接尚未就绪' : '尚未检查登录状态';

  return <section className="surface text-execution-settings"><div className="section-heading compact"><h3>OpenAI</h3>{provider !== 'api' && <Badge status={busy || login.state === 'pending' && !connected ? 'running' : connected ? 'available' : needsLogin ? 'auth_required' : 'not_configured'}>{busy ? '连接中' : connected ? '已连接' : needsLogin ? '待登录' : '待连接'}</Badge>}</div>
    <p className="muted">SDK 与 CLI 共用 OpenAI 账户，用于制作基础、图片提示词和分镜脚本。</p>
    <label>执行方式<select disabled={!writable || busy} data-text-provider value={provider === 'api' ? '' : provider} onChange={event => update('textProvider', event.target.value)}>{provider === 'api' && <option value="" disabled>请选择执行方式</option>}<option value="codex">Codex CLI</option><option value="codex_sdk">Codex SDK</option></select></label>
    {provider !== 'api' ? <>
      <div className="text-component-status" role="status"><strong>组件</strong><span>{componentBusy ? '正在检测本机组件…' : components?.installed ? '已安装' : components ? components.components.some(item => item.state === 'invalid') ? '需要修复' : '未安装' : '检测未完成'}</span>{components?.installed && components.components.map(component => { const version = componentVersion(component); return <span key={component.id}>{component.name}{version ? ` ${version}` : activeCheck.result ? '' : ' · 版本待检查'}</span>; })}</div>
      {(componentError || components?.components.find(component => component.state === 'invalid')?.message) && <p className="warning" role="alert">{componentError || components?.components.find(component => component.state === 'invalid')?.message}</p>}
      {modelControls}
      <div className="text-connection-status" role="status" data-sdk-execution-state={provider === 'codex_sdk' ? sdkReady ? 'ready' : sdkAvailable ? 'unverified' : 'closed' : undefined}>
        <strong>{connectionLabel}</strong>{provider === 'codex' && status?.account?.planType && <span className="badge">{status.account.planType}</span>}
      </div>
      {activeCheck.result && <p className="control-hint">最近检查：{formatDate(activeCheck.result.checkedAt)}</p>}
      <div className="actions text-connection-actions">{components && !components.installed && <Button executionAction="runtime.prepare" icon="download" disabled={!writable || busy || !components.installSupported || !automaticSetup} onClick={() => void connect(true)}>{operation === 'installing' ? '正在安装…' : '安装组件'}</Button>}<Button executionAction="readiness.check" data-readiness-provider={activeCheck.provider} icon="refresh" disabled={!writable || busy || componentBusy || !!activeCheck.blocked} title={activeCheck.blocked || undefined} onClick={() => void connect()}>{busy ? '正在检查…' : components && !components.installed ? '重新检测组件' : activeCheck.result ? '重新检查' : '检查连接'}</Button>
        {login.state === 'pending' && !connected ? <><Button executionAction="textCli.openLogin" disabled={!writable || busy} onClick={() => void loginAction('textCli.openLogin')}>打开登录网页</Button><Button executionAction="textCli.cancelLogin" disabled={!writable || busy} onClick={() => void loginAction('textCli.cancelLogin')}>取消等待</Button></> : <Button executionAction="textCli.login" disabled={!writable || busy || !components?.installed} onClick={() => void loginAction('textCli.login')}>{connected ? '重新登录' : '登录'}</Button>}
      </div>
      <p className="control-hint">{login.state === 'pending' && !connected ? '请在官方网页完成登录，完成后自动更新连接状态。' : components?.installed ? '组件已就绪，可登录或检查账户与模型。' : automaticSetup ? '先安装缺少的组件，再登录账户。' : '当前环境仅支持检查已有程序。'}</p>
      {(error || activeCheck.error) && <p className="warning" role="alert">{error || activeCheck.error}</p>}
      {activeCheck.blocked && <p className="warning">{activeCheck.blocked}</p>}
      {provider === 'codex_sdk' && !sdkAvailable && <p className="warning">此环境暂未开放 SDK 执行。</p>}
      {models.length > 0 && (!selectedModel || !efforts.includes(reasoning)) && <p className="warning">请从已连接的模型目录中选择模型与推理强度。</p>}
      <details className="details text-execution-diagnostics"><summary>连接诊断</summary>
        <ProviderReadinessResult check={activeCheck}/>
        {provider === 'codex_sdk' && typeof sdkCheck.result?.details.runtimeExecutable === 'string' && <p className="control-hint path-value">Codex：{sdkCheck.result.details.runtimeExecutable}</p>}
        {provider === 'codex_sdk' && sdkPath && <p className="control-hint path-value">Node.js：{sdkPath}</p>}
        {path && <p className="control-hint path-value">Codex 程序：{path}</p>}
        {provider === 'codex' && status?.warnings?.map((warning, index) => <p className="control-hint" key={index}>{warning}</p>)}
        {provider === 'codex' && status?.limits?.length ? <div className="text-cli-limits">{status.limits.map(limit => { const windows = [windowText(limit.primary), windowText(limit.secondary)].filter(Boolean); return <div key={limit.id}><strong>{limit.name || '账户额度'}</strong>{windows.map((value, index) => <span key={index}>{value}</span>)}{limit.credits?.unlimited ? <span>额度不限</span> : limit.credits?.balance != null && <span>余额 {limit.credits.balance}</span>}</div>; })}</div> : null}
        <p className="control-hint">检查连接不会生成内容。更改设置仅影响新流程；历史任务保留原入口，结果未知的任务不会自动重发。</p>
      </details>
    </> : <p className="warning">此前使用了其他执行方式，请选择 CLI 或 SDK。原设置已保留。</p>}
  </section>;
}
