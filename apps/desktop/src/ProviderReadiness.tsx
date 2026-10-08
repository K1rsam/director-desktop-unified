import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { request } from './api';
import { actionBlockReason, currentExecutionPolicy, hasSdkExecutionCapability, subscribeExecutionPolicy } from './executionPolicy';
import type { ExecutionPolicy, ReadinessProvider } from './executionPolicy';
import { Button, formatDate, messageOf } from './components';
import type { ProviderReadinessReport } from './types';

const pathKeys = { codex_cli: 'codexPath', dreamina: 'dreaminaPath', dreamina_canvas: 'dreaminaCanvasPath', codex_sdk: 'textSdkNodePath' } as const;

export function sdkReadinessAllowsExecution(report: ProviderReadinessReport | null, policy: ExecutionPolicy = currentExecutionPolicy()): boolean {
  return hasSdkExecutionCapability(policy) && report?.provider === 'codex_sdk'
    && report.installation.state === 'ready' && report.protocol.state === 'ready'
    && report.authentication.state === 'authenticated' && report.authentication.identityVerified
    && report.catalog.state === 'available' && report.catalog.models.length > 0
    && report.details.officialRuntimeChecked === true && report.details.accountChecked === true
    && typeof report.details.runtimeExecutable === 'string' && report.details.runtimeExecutable.length > 0
    && report.generation.enabled === true && report.generation.testPerformed === false && report.generation.calls === 0;
}

/** Reject mismatched or contradictory results instead of displaying an execution claim. */
export function validateProviderReadiness(value: unknown, provider: ReadinessProvider): ProviderReadinessReport {
  const report = value as ProviderReadinessReport | null;
  const nullableText = (text: unknown) => text === null || typeof text === 'string';
  if (!report || report.schemaVersion !== 'provider-readiness/v1' || report.provider !== provider
      || typeof report.checkedAt !== 'string'
      || !['ready', 'missing', 'invalid', 'not_checked'].includes(report.installation?.state)
      || !nullableText(report.installation.version) || !nullableText(report.installation.executable)
      || !['ready', 'failed', 'unsupported', 'not_checked'].includes(report.protocol?.state) || !nullableText(report.protocol.name)
      || !['authenticated', 'not_logged_in', 'unknown', 'not_checked'].includes(report.authentication?.state)
      || typeof report.authentication.identityVerified !== 'boolean'
      || !['available', 'unavailable', 'not_checked'].includes(report.catalog?.state)
      || !Array.isArray(report.catalog.models) || !report.catalog.models.every(model => model && typeof model.id === 'string'
        && (model.name === undefined || typeof model.name === 'string')
        && (model.kind === undefined || typeof model.kind === 'string')
        && (model.defaultReasoningEffort === undefined || typeof model.defaultReasoningEffort === 'string')
        && (model.isDefault === undefined || typeof model.isDefault === 'boolean')
        && (model.supportedReasoningEfforts === undefined || Array.isArray(model.supportedReasoningEfforts) && model.supportedReasoningEfforts.every(effort => typeof effort === 'string')))
      || !['verified_read_only', 'not_tested', 'failed'].includes(report.network?.state) || !Array.isArray(report.network.proof)
      || typeof report.generation?.enabled !== 'boolean' || report.generation?.testPerformed !== false || report.generation?.calls !== 0
      || !Array.isArray(report.messages) || !report.messages.every(message => typeof message === 'string')
      || !report.details || typeof report.details !== 'object' || Array.isArray(report.details)
      || report.generation.enabled && !sdkReadinessAllowsExecution(report)) throw new Error('本地核心返回的只读检查结果无效，请重新检查。');
  return report;
}

type CheckState = { scope: string; attempt: number; busy: boolean; result: ProviderReadinessReport | null; error: string };
export function useProviderReadiness(projectId: string, provider: ReadinessProvider, path: string, enabled = true, sdkSettings?: { codexPath: string }) {
  useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const codexPath = provider === 'codex_sdk' ? sdkSettings?.codexPath : undefined;
  const scope = JSON.stringify([projectId, provider, path, enabled, codexPath]);
  const currentScope = useRef(scope), sequence = useRef(0), inFlight = useRef<string | null>(null), mounted = useRef(true);
  if (currentScope.current !== scope) { currentScope.current = scope; sequence.current++; inFlight.current = null; }
  const [state, setState] = useState<CheckState>({ scope, attempt: sequence.current, busy: false, result: null, error: '' });
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; sequence.current++; inFlight.current = null; }; }, []);
  const blocked = actionBlockReason({ action: 'readiness.check', provider });
  const check = useCallback(async () => {
    if (!enabled || !mounted.current || currentScope.current !== scope || inFlight.current === scope
        || actionBlockReason({ action: 'readiness.check', provider })) return;
    const attempt = ++sequence.current;
    inFlight.current = scope;
    setState({ scope, attempt, busy: true, result: null, error: '' });
    try {
      const response = await request<unknown>({ action: 'readiness.check', projectId, provider, settings: { [pathKeys[provider]]: path, ...(codexPath !== undefined ? { codexPath } : {}) } });
      if (!mounted.current || attempt !== sequence.current || currentScope.current !== scope) return;
      setState({ scope, attempt, busy: false, result: validateProviderReadiness(response, provider), error: '' });
    } catch (reason) {
      if (mounted.current && attempt === sequence.current && currentScope.current === scope) setState({ scope, attempt, busy: false, result: null, error: messageOf(reason) });
    } finally {
      if (attempt === sequence.current) inFlight.current = null;
    }
  }, [enabled, scope, projectId, provider, path, codexPath]);
  const current = state.scope === scope && state.attempt === sequence.current ? state : { busy: false, result: null, error: '' };
  const reset = useCallback(() => {
    const attempt = ++sequence.current; inFlight.current = null;
    setState({ scope, attempt, busy: false, result: null, error: '' });
  }, [scope]);
  return { ...current, check, reset, blocked, provider };
}
export type ReadinessCheck = ReturnType<typeof useProviderReadiness>;

export function ReadinessButton({ check, writable, children }: { check: ReadinessCheck; writable: boolean; children: string }) {
  return <Button executionAction="readiness.check" data-readiness-provider={check.provider} icon="refresh" disabled={!writable || check.busy || !!check.blocked} title={check.blocked || undefined} onClick={() => void check.check()}>{check.busy ? '正在只读检查…' : children}</Button>;
}

function CanvasModelSpecifications({ value }: { value: unknown }) {
  if (!Array.isArray(value)) return null;
  const records = value.filter(item => item && typeof item === 'object' && typeof item.model === 'string').slice(0, 100);
  if (!records.length) return null;
  return <details className="details"><summary>查看 Canvas 只读模型模式</summary><ul className="model-list">{records.map((item, index) => {
    const modes = Array.isArray(item.modes) ? item.modes.filter((mode: unknown): mode is { name: string } => !!mode && typeof mode === 'object' && 'name' in mode && typeof mode.name === 'string').slice(0, 100).map((mode: { name: string }) => mode.name) : null;
    return <li key={`${item.model}:${index}`}>{item.model}<small>{modes === null ? '模式未提供' : modes.length ? `模式：${modes.join(' / ')}` : '无模式记录'}</small></li>;
  })}</ul><p className="control-hint">仅展示 CLI 返回的目录与模式；缺少的能力仍未知，不推断模型映射、账户资格、免费额度或生成已适配。</p></details>;
}

// This view is an allowlisted projection of one completed check, never a live
// process monitor. Do not iterate backend keys or echo free-form diagnostics.
const canvasCommands = ['version', 'schema', 'auth status', 'auth account', 'model list --type image'] as const;
const canvasPhases = ['created', 'thread_started', 'runtime_ready', 'spawn_start', 'spawn_end', 'observing', 'exit_observed', 'timeout', 'cleanup_start', 'cleanup_end', 'completed', 'runtime_finished', 'runtime_dropped', 'thread_joined'] as const;
const diagnosticRecord = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const diagnosticNumber = (value: unknown, signed = false): string => typeof value === 'number' && Number.isSafeInteger(value) && (signed || value >= 0) ? String(value) : '未知';
const diagnosticMs = (value: unknown): string => diagnosticNumber(value) === '未知' ? '未知' : `${diagnosticNumber(value)} ms`;
const diagnosticFlag = (value: unknown): string => value === true ? '是' : value === false ? '否' : '未知';
const diagnosticEnum = (value: unknown, allowed: readonly string[]): string => typeof value === 'string' && allowed.includes(value) ? value : '未知';

function CanvasProcessDiagnostics({ value }: { value: unknown }) {
  const diagnostics = diagnosticRecord(value);
  const commands = canvasCommands.filter(command => Object.hasOwn(diagnostics, command));
  return <details className="details" data-canvas-process-diagnostics>
    <summary>查看 Canvas 进程阶段诊断</summary>
    <p className="control-hint">以下为最近一次只读检查的历史记录。PID 不表示进程当前仍在运行；缺少记录时保持未知。阶段与超时记录不能单独确定故障原因或证明问题已修复。</p>
    {!commands.length && <p className="control-hint">进程阶段诊断未知：本次结果未提供可用记录。</p>}
    {commands.map(command => {
      const supervisor = diagnosticRecord(diagnosticRecord(diagnostics[command]).supervisor);
      const exit = diagnosticRecord(supervisor.processExit), timeout = diagnosticRecord(supervisor.timeout);
      const preCleanup = diagnosticRecord(supervisor.preCleanup), cleanup = diagnosticRecord(supervisor.cleanup);
      const milestones = diagnosticRecord(supervisor.milestones);
      return <section key={command} data-canvas-command={command} aria-label={`Canvas ${command} 进程诊断`}>
        <h4>{command}</h4>
        <dl>
          <div><dt>检查时 PID（历史）</dt><dd>{typeof supervisor.pid === 'number' && supervisor.pid > 0 ? diagnosticNumber(supervisor.pid) : '未知'}</dd></div>
          <div><dt>最后记录阶段</dt><dd>{diagnosticEnum(supervisor.phase, [...canvasPhases, 'observation_unavailable'])}</dd></div>
          <div><dt>已用时间 / 预算</dt><dd>{diagnosticMs(supervisor.elapsedMs)} / {diagnosticMs(supervisor.budgetMs)}</dd></div>
          <div><dt>控制循环次数 / 最大间隔</dt><dd>{diagnosticNumber(supervisor.controlTicks)} / {diagnosticMs(supervisor.maxControlGapMs)}</dd></div>
          {(['stdout', 'stderr'] as const).map(name => {
            const stream = diagnosticRecord(supervisor[name]);
            return <div key={name}><dt>{name}</dt><dd>字节：{diagnosticNumber(stream.bytes)} · EOF：{diagnosticFlag(stream.eof)} · 读取错误：{diagnosticFlag(stream.readError)} · 首字节：{diagnosticMs(stream.firstByteMs)} · EOF 时间：{diagnosticMs(stream.eofMs)}</dd></div>;
          })}
          <div><dt>观察到的退出</dt><dd>{supervisor.processExit == null ? '未知 / 无退出记录' : `退出码：${diagnosticNumber(exit.code, true)} · 信号：${diagnosticNumber(exit.signal)} · 成功：${diagnosticFlag(exit.success)} · 观察时间：${diagnosticMs(exit.observedAtMs)}`}</dd></div>
          <div><dt>超时记录</dt><dd>{supervisor.timeout == null ? '未知 / 无超时记录' : `类型：${diagnosticEnum(timeout.kind, ['run', 'drain', 'overall'])} · 已用：${diagnosticMs(timeout.elapsedMs)} · 超出：${diagnosticMs(timeout.overshootMs)} · 已观察退出：${diagnosticFlag(timeout.exitObserved)}`}</dd></div>
          <div><dt>清理前观察</dt><dd>状态：{diagnosticEnum(preCleanup.state, ['not_checked', 'running', 'exited', 'query_failed'])} · 退出码：{diagnosticNumber(preCleanup.exitCode, true)} · 信号：{diagnosticNumber(preCleanup.signal)}</dd></div>
          <div><dt>清理记录</dt><dd>已请求终止：{diagnosticFlag(cleanup.killRequested)} · 已确认回收：{diagnosticFlag(cleanup.reapConfirmed)}</dd></div>
        </dl>
        <details className="details"><summary>阶段时间（相对本次命令开始）</summary><dl>{canvasPhases.map(phase => <div key={phase}><dt>{phase}</dt><dd>{diagnosticMs(milestones[phase])}</dd></div>)}</dl></details>
      </section>;
    })}
  </details>;
}

export function ProviderReadinessResult({ check }: { check: ReadinessCheck }) {
  const result = check.result;
  if (check.error) return <p className="warning" role="alert">{check.error}</p>;
  if (!result) return <p className="control-hint">{check.busy ? '正在读取程序与连接状态，不发起生成。' : check.blocked || '点击检查后读取当前程序路径；打开页面和切换渠道不会访问账户。'}</p>;
  const installed = { ready: '已找到', missing: '未找到', invalid: '路径或程序无效', not_checked: '未检查' };
  const protocol = { ready: '握手通过', failed: '握手失败', unsupported: '协议不受支持', not_checked: '未检查' };
  const authentication = { authenticated: '已登录', not_logged_in: '未登录', unknown: '状态未知', not_checked: '未检查' };
  const authenticationLabel = result.provider === 'dreamina_canvas' && result.authentication.state === 'authenticated' && !result.authentication.identityVerified ? '本机有登录记录' : authentication[result.authentication.state];
  const catalog = { available: '已读取', unavailable: '读取未完成', not_checked: '未检查' };
  const network = { verified_read_only: '只读接口已验证', not_tested: '未测试', failed: '只读接口检查失败' };
  const canvasAccount = result.details.accountQuery;
  const canvasAccountState = canvasAccount && typeof canvasAccount === 'object' && 'state' in canvasAccount ? canvasAccount.state : undefined;
  const canvasAccountLabel = canvasAccountState === 'available' ? '已读取' : canvasAccountState === 'unavailable' ? '读取未完成' : canvasAccountState === 'unsupported' ? '当前协议未提供只读账户查询' : canvasAccountState === 'not_checked' ? '未检查' : '状态未知';
  return <div className="provider-readiness-result" role="status" data-readiness-result={result.provider}>
    <p className="control-hint">最近只读检查：{formatDate(result.checkedAt)}</p>
    <dl><div><dt>程序安装</dt><dd>{installed[result.installation.state]}{result.installation.version ? ` · ${result.installation.version}` : ''}</dd></div>
      <div><dt>协议 / 环境</dt><dd>{protocol[result.protocol.state]}{result.protocol.name ? ` · ${result.protocol.name}` : ''}</dd></div>
      <div><dt>账户</dt><dd>{authenticationLabel} · {result.authentication.identityVerified ? '身份已核验' : '身份未核验'}</dd></div>
      {result.provider === 'dreamina_canvas' && <div><dt>Canvas account 查询</dt><dd>{canvasAccountLabel}</dd></div>}
      <div><dt>模型目录</dt><dd>{catalog[result.catalog.state]}{result.catalog.state === 'available' ? ` · ${result.catalog.models.length} 个模型` : ''}</dd></div>
      <div><dt>网络</dt><dd>{network[result.network.state]}</dd></div>
      <div><dt>真实模型生成</dt><dd>未测试 · 本次检查生成调用 0 次</dd></div></dl>
    {result.installation.executable && <p className="control-hint path-value">{result.installation.executable}</p>}
    {result.messages.map((message, index) => <p className="control-hint" key={index}>{message}</p>)}
    {result.provider === 'codex_sdk' && <p className="control-hint">{sdkReadinessAllowsExecution(result) ? '已核验 SDK 原生环境、账户与模型目录，可显式选择 SDK 发起文字流程；本次检查没有模型生成，成稿质量尚未验收。' : 'SDK 包导入和 worker 握手仅证明环境与协议；真实模型未测试，SDK 真实生成保持关闭。'}</p>}
    {result.provider === 'dreamina' && <p className="control-hint">图片免费资格尚未核验；积分余额和模型目录不能证明本次生成免费。</p>}
    {result.provider === 'dreamina_canvas' && <p className="control-hint">Canvas CLI 仅核验版本、schema、auth status、account 与 image models。积分 / 额度未知：当前适配的只读协议没有额度接口；免费资格未核验。尚未接入生成桥，模型目录不会写入图片或视频生成选项。</p>}
    {result.provider === 'codex_cli' && <p className="control-hint">账户与模型目录读取不代表真实模型生成已通过。</p>}
    {result.catalog.models.length > 0 && <details className="details"><summary>查看本次模型目录</summary><ul className="model-list">{result.catalog.models.map(model => <li key={model.id}>{model.name || model.id}{result.provider === 'dreamina_canvas' && model.name && model.name !== model.id ? ` · ${model.id}` : ''}{model.kind ? ` · ${model.kind}` : ''}</li>)}</ul></details>}
    {result.provider === 'dreamina_canvas' && <CanvasModelSpecifications value={result.details.modelSpecifications}/>}
    {result.provider === 'dreamina_canvas' && <CanvasProcessDiagnostics value={result.details.processDiagnostics}/>}
  </div>;
}
