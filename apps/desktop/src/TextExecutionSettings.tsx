import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { choose, request } from './api';
import { actionBlockReason, currentExecutionPolicy, hasSdkExecutionCapability, subscribeExecutionPolicy } from './executionPolicy';
import { Badge, Button, messageOf } from './components';
import type { Settings, TextCliLimitWindow, TextCliLogin, Workspace } from './types';

import { ProviderReadinessResult, ReadinessButton, sdkReadinessAllowsExecution, useProviderReadiness } from './ProviderReadiness';

type Props = { workspace: Workspace; settings: Settings; writable: boolean; update: (key: string, value: string) => void; saveSettings: () => Promise<boolean>; onError?: (message: string) => void };

export function TextExecutionSummary({ settings, onConfigure, stage }: { settings: Settings; onConfigure: () => void; stage?: 'foundation' | 'images' | 'storyboard' }) {
  const policy = useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const action = stage === 'images' ? 'images.design' : stage === 'storyboard' ? 'storyboard.run' : 'foundation.run';
  const cliCapability = policy.textBackends?.find(backend => backend.id === 'codex_cli');
  const enabled = policy.liveExecutionAllowed && ['standard_native', 'native_cli_acceptance'].includes(policy.mode)
    && (!settings.textProvider || settings.textProvider === 'codex') && (!cliCapability || cliCapability.status === 'enabled')
    && policy.allowedActions?.includes(action) === true;
  const sdkAvailable = settings.textProvider === 'codex_sdk' && hasSdkExecutionCapability(policy) && policy.allowedActions?.includes(action) === true;
  const label = enabled ? policy.mode === 'standard_native' ? '标准版 CLI 已开放' : '受控 CLI 验收' : sdkAvailable ? '标准版 SDK · 执行前核验' : '当前关闭';
  return <div className="text-execution-summary"><span>{stage === 'images' ? '提示词设计' : '文本执行'}（{label}）：{settings.textProvider === 'api' ? '文本 API' : settings.textProvider === 'codex_sdk' ? sdkAvailable ? '官方 Codex SDK' : '官方 Codex SDK（离线核验）' : 'Codex CLI'} · {settings.textModel || '未选择模型'}{settings.textProvider !== 'api' ? ` · ${settings.reasoning || 'medium'}` : ''}{stage === 'images' ? '；图片按出图渠道生成' : ''}</span><Button variant="ghost" icon="settings" onClick={onConfigure}>调整执行设置</Button></div>;
}

function windowText(window?: TextCliLimitWindow | null): string | undefined {
  if (!window) return;
  const minutes = window.windowDurationMins;
  const period = minutes && minutes >= 1440 ? `${Math.round(minutes / 1440)} 天` : minutes && minutes >= 60 ? `${Math.round(minutes / 60)} 小时` : minutes ? `${minutes} 分钟` : '当前';
  const remaining = typeof window.remainingPercent === 'number' ? window.remainingPercent : typeof window.usedPercent === 'number' ? 100 - window.usedPercent : undefined;
  return remaining === undefined ? undefined : `${period}额度剩余 ${Math.max(0, Math.min(100, remaining)).toFixed(0)}%`;
}

export function TextExecutionSettings({ workspace, settings, writable, update, saveSettings, onError }: Props) {
  const policy = useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const [login, setLogin] = useState<TextCliLogin>({ state: 'idle' }), [loginBusy, setLoginBusy] = useState(false), [error, setError] = useState('');
  const mounted = useRef(true), sequence = useRef(0), projectId = workspace.project.id;
  const provider = settings.textProvider || 'codex', path = String(settings.codexPath || ''), sdkPath = String(settings.textSdkNodePath || '');
  const sdkAvailable = hasSdkExecutionCapability(policy);
  const cliCheck = useProviderReadiness(projectId, 'codex_cli', path, writable && provider === 'codex');
  const sdkCheck = useProviderReadiness(projectId, 'codex_sdk', sdkPath, writable && provider === 'codex_sdk', typeof settings.codexPath === 'string' ? { codexPath: path } : undefined);
  const sdkReady = sdkReadinessAllowsExecution(sdkCheck.result, policy);
  const status = cliCheck.result?.details.cliStatus;
  const activeCheck = provider === 'codex_sdk' ? sdkCheck : cliCheck;
  const models = (activeCheck.result?.catalog.state === 'available' ? activeCheck.result.catalog.models : []).map(model => {
    const detail = status?.models?.find(item => item.id === model.id);
    return { id: model.id, displayName: model.name || model.id, supportedReasoningEfforts: model.supportedReasoningEfforts || (provider === 'codex' ? detail?.supportedReasoningEfforts : undefined) || [], defaultReasoningEffort: model.defaultReasoningEffort || (provider === 'codex' ? detail?.defaultReasoningEffort : undefined) || 'medium', isDefault: model.isDefault || (provider === 'codex' && detail?.isDefault) || false };
  });
  const selectedModel = models.find(model => model.id === settings.textModel), efforts = selectedModel?.supportedReasoningEfforts || [], reasoning = settings.reasoning || 'medium';
  const busy = loginBusy || (provider === 'codex' ? cliCheck.busy : sdkCheck.busy);
  useEffect(() => { mounted.current = true; sequence.current++; setLogin({ state: 'idle' }); setLoginBusy(false); setError(''); return () => { mounted.current = false; sequence.current++; }; }, [projectId, provider, path, sdkPath]);

  async function loginAction(action: 'textCli.login' | 'textCli.cancelLogin' | 'textCli.openLogin') {
    if (actionBlockReason(action)) return;
    const current = ++sequence.current;
    setLoginBusy(true); setError('');
    try {
      if (action === 'textCli.login' && !await saveSettings()) return;
      const next = await request<{ login?: TextCliLogin }>({ action, projectId });
      if (mounted.current && current === sequence.current && next.login) setLogin(next.login);
    } catch (reason) { if (mounted.current && current === sequence.current) setError(messageOf(reason)); }
    finally { if (mounted.current && current === sequence.current) setLoginBusy(false); }
  }
  async function selectExecutable() { try { const paths = await choose('file'); if (paths[0]) update('codexPath', paths[0]); } catch (reason) { onError?.(messageOf(reason)); } }
  async function selectNode() { try { const paths = await choose('file'); if (paths[0]) update('textSdkNodePath', paths[0]); } catch (reason) { onError?.(messageOf(reason)); } }
  const labels = cliCheck.blocked ? '只读检查未开放' : busy ? '正在检查' : cliCheck.result ? '已完成只读检查' : '尚未检查';
  const modelControls = <div className="form-grid"><label>文本模型<select data-text-model value={settings.textModel || ''} onChange={event => { update('textModel', event.target.value); const model = models.find(item => item.id === event.target.value); if (model && !model.supportedReasoningEfforts.includes(reasoning)) update('reasoning', model.defaultReasoningEffort); }}><option value="" disabled>选择已核实的文本模型</option>{settings.textModel && !models.some(model => model.id === settings.textModel) && <option value={settings.textModel}>{settings.textModel} · 尚未核实</option>}{models.map(model => <option key={model.id} value={model.id}>{model.displayName}{model.isDefault ? ' · 目录默认' : ''}</option>)}</select></label><label>推理强度<select data-text-reasoning value={reasoning} onChange={event => update('reasoning', event.target.value)}>{!efforts.includes(reasoning) && <option value={reasoning}>{reasoning} · 尚未核实</option>}{efforts.map(effort => <option key={effort} value={effort}>{effort}</option>)}</select></label></div>;

  return <section className="surface text-execution-settings"><div className="section-heading compact"><h3>文本执行 · 阶段 1 / 2 / 3</h3>{provider === 'codex' && <Badge status={busy || login.state === 'pending' ? 'running' : 'not_configured'}>{labels}</Badge>}</div>
    <p className="muted">制作基础、图片提示词设计、分镜文字共用此设置。{policy.mode === 'standard_native' ? sdkAvailable ? '标准版可显式选择 CLI 或 SDK；账户与模型目录需检查，发起流程时会重新核验并冻结实际入口。' : '标准版 CLI 文字执行已开放，账户与模型目录需检查；SDK 真实调用尚未开放。' : policy.mode === 'native_cli_acceptance' ? '当前为固定源、固定模型与范围的受控 CLI 文字验收；SDK、API、图片和视频真实生成保持关闭。' : policy.readiness?.enabled ? '程序、账户和模型目录可按按钮单独只读检查；真实生成与登录、凭据变更仍受独立门禁控制。' : '当前环境的只读连接检查尚未开放；可保存本地执行偏好，真实生成与登录、凭据变更保持关闭。'}</p>
    <label>文本渠道<select value={provider} onChange={event => update('textProvider', event.target.value)}><option value="codex">Codex CLI</option><option value="codex_sdk">{sdkAvailable ? '官方 Codex SDK（需原生就绪检查）' : '官方 Codex SDK（离线核验，真实调用关闭）'}</option><option value="api">文本 API</option></select></label>
    {provider === 'codex' ? <>
      <label>Codex CLI 程序<div className="text-cli-path"><input value={path} onChange={event => update('codexPath', event.target.value)} placeholder="自动查找本机 Codex CLI，或填写完整路径"/><Button icon="source" disabled={!writable || busy} onClick={() => void selectExecutable()}>选择程序</Button></div></label>
      {status?.executable && <p className="control-hint text-cli-resolved">{status.version || 'Codex CLI'} · {status.executable}</p>}
      <div className="text-cli-account"><strong>{cliCheck.result?.authentication.state === 'authenticated' ? '已登录本机 Codex 账户' : cliCheck.result?.authentication.state === 'not_logged_in' ? '本机 Codex 账户未登录' : '本机 Codex 账户尚未核验'}</strong>{status?.account?.planType && <span className="badge">{status.account.planType}</span>}</div>
      <div className="actions"><ReadinessButton check={cliCheck} writable={writable && !loginBusy}>检查 CLI 与账户</ReadinessButton>{login.state === 'pending' ? <><Button executionAction="textCli.openLogin" disabled={!writable || busy} onClick={() => void loginAction('textCli.openLogin')}>打开登录网页</Button><Button executionAction="textCli.cancelLogin" disabled={!writable || busy} onClick={() => void loginAction('textCli.cancelLogin')}>取消等待</Button></> : <Button executionAction="textCli.login" disabled={!writable || busy || status?.available === false} onClick={() => void loginAction('textCli.login')}>{status?.loggedIn ? '重新登录' : '登录 Codex'}</Button>}</div>
      {login.state === 'pending' && <p className="control-hint">请在已打开的官方网页完成登录，再点击只读检查核对账户。</p>}
      {status?.limits?.length ? <div className="text-cli-limits">{status.limits.map(limit => { const windows = [windowText(limit.primary), windowText(limit.secondary)].filter(Boolean); return <div key={limit.id}><strong>{limit.name || '账户额度'}</strong>{windows.map((value, index) => <span key={index}>{value}</span>)}{limit.credits?.unlimited ? <span>额度不限</span> : limit.credits?.balance != null && <span>余额 {limit.credits.balance}</span>}</div>; })}</div> : status?.loggedIn ? <p className="control-hint">账户未返回额度明细。</p> : null}
      {error && <p className="warning" role="alert">{error}</p>}
      <ProviderReadinessResult check={cliCheck}/>
      {status?.warnings?.map((warning, index) => <p className="control-hint" key={index}>{warning}</p>)}
      {modelControls}
      {models.length > 0 && (!selectedModel || !efforts.includes(reasoning)) && <p className="warning">当前模型或推理强度未获此 CLI 确认，请从目录中重新选择。</p>}
      <p className="control-hint">检查使用输入框里的当前路径，无需先保存；修改路径后请重新检查。保存设置只影响新流程。</p>
    </> : provider === 'codex_sdk' ? <>
      <p className="control-hint">官方 SDK 共用工作流、原始回包、领域校验、缓存与持久修订预算。{sdkAvailable ? '后端已开放显式 SDK 入口，须核验 Node.js、官方运行器、账户与模型；不会自动改用 CLI。' : '当前只支持离线环境核验，SDK 真实调用保持关闭；不会自动改用 CLI。'}</p>
      <strong role="status" data-sdk-execution-state={sdkReady ? 'ready' : sdkAvailable ? 'unverified' : 'closed'}>{sdkReady ? 'SDK 环境、账户与目录已核验；发起流程前仍会核对所选模型和强度' : sdkAvailable ? 'SDK 尚未就绪核验，请检查环境、账户与模型' : 'SDK 真实执行当前关闭'}</strong>
      <label>SDK Node.js 程序<div className="text-cli-path"><input value={sdkPath} onChange={event => update('textSdkNodePath', event.target.value)} placeholder="自动查找 Node.js，或填写本机程序绝对路径"/><Button icon="source" disabled={!writable || busy} onClick={() => void selectNode()}>选择 Node.js</Button></div></label>
      <label>已有 Codex CLI 程序<div className="text-cli-path"><input value={path} onChange={event => update('codexPath', event.target.value)} placeholder="复用 CLI 路径，留空自动查找官方运行器"/><Button icon="source" disabled={!writable || busy} onClick={() => void selectExecutable()}>选择程序</Button></div></label>
      {typeof sdkCheck.result?.details.runtimeExecutable === 'string' && <p className="control-hint text-cli-resolved">{String(sdkCheck.result.details.runtimeVersion || 'Codex')} · {sdkCheck.result.details.runtimeExecutable}</p>}
      <ReadinessButton check={sdkCheck} writable={writable}>{sdkAvailable ? '检查 SDK 环境、账户与模型' : '检查 SDK 环境与协议'}</ReadinessButton>
      <ProviderReadinessResult check={sdkCheck}/>
      {sdkAvailable ? modelControls : <><label>文本模型<input value={settings.textModel || ''} onChange={event => update('textModel', event.target.value)} placeholder="gpt-6.1-sol"/></label><label>推理强度<input value={reasoning} onChange={event => update('reasoning', event.target.value)} placeholder="medium"/></label></>}
      {sdkAvailable && models.length > 0 && (!selectedModel || !efforts.includes(reasoning)) && <p className="warning">当前模型或推理强度未获本次 SDK 目录确认，请重新选择。</p>}
      <p className="control-hint">检查使用当前 Node.js 与 Codex 路径，无需先保存；更改任一路径会清除旧检查。保存设置只影响新流程，不启动模型请求。</p>
      <p className="warning">固定 SDK 仅支持本地回执核对，无法证明远端停止。未知任务保留占位，不自动重发或切换 CLI；保存偏好不会启动模型请求。</p>
    </> : <>
      <label>文本模型<input value={settings.textModel || ''} onChange={event => update('textModel', event.target.value)} placeholder="填写 API 支持的模型"/></label>
      <label>API 地址<input value={settings.textEndpoint || ''} onChange={event => update('textEndpoint', event.target.value)} placeholder="https://…"/></label><label>本机凭据<input disabled={!!actionBlockReason('textCli.login')} title={actionBlockReason('textCli.login') || undefined} type="password" autoComplete="new-password" value={settings.textApiKey || ''} onChange={event => update('textApiKey', event.target.value)} placeholder="留空保留已保存凭据"/></label>
      <p className="control-hint">Rust 核心直接调用已保存的文本 API，推理策略使用服务默认值。</p>
    </>}
    <p className="note">全局文本上限 4 路，两入口共用；SDK 单 worker 上限 1 路，未知执行持续占位。同集上限 2 路；连续依赖顺序执行。更改设置只影响新流程，历史任务保留原入口。</p>
  </section>;
}
