import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { choose, request } from './api';
import { actionBlockReason } from './executionPolicy';
import { Stage1Panel } from './Stage1Panel';
import { CreativeRules } from './CreativeRules';
import { TextExecutionSettings } from './TextExecutionSettings';
import { ManagedBrowserCard, managedBrowserStatus } from './ManagedBrowserCard';
import { WebImageConnection } from './WebImageConnection';
import type { ManagedBrowserAction, ManagedBrowserStatus } from './ManagedBrowserCard';
import { invalidateProviderReadiness, ProviderReadinessResult, useProviderReadiness } from './ProviderReadiness';
import type { ReadinessCheck } from './ProviderReadiness';
import { prepareRuntime, readRuntimeStatus } from './runtimeSetup';
import type { RuntimeStatus } from './runtimeSetup';
import './ChannelsSettings.css';
import './ProjectSettings.css';
import { ProductionSettings } from './ProductionSettings';
import { generationProgress, isLocalVideoPreparation, videoPreparationLabel } from './generationState';
import { taskRecordsForView, TaskRecordActions } from './TaskRecordManagement';
import { generationChoices, imageOutputLabel, imageOutputValueLabel, ratioLabel, resolutionLabel } from './contracts';
import { Badge, Button, costText, Empty, formatDate, Icon, JsonDetails, kindName, MediaPreview, messageOf, Modal, providerId, providerList, providerName } from './components';
import type { AppSettings, DreaminaAccount, GlobalMutation, Job, Media, Mutation, Settings, Workspace } from './types';

type Props = { workspace: Workspace; writable: boolean; mutate: Mutation; onError?: (error: string) => void; onNotice?: (notice: string) => void };
type ChromeSetup = { schemaVersion: 'chrome-setup/v1'; installed: boolean; configured: boolean; status: 'unconfigured' | 'offline' | 'waiting_browser' | 'waiting_page' | 'network_unavailable' | 'ready' | 'busy' | 'needs_selection' | 'error'; message: string; clients: number; usableClients: number; componentVersion?: string; extensionPath?: string; checkedAt?: string | null; boundProfileId?: string; transport?: string; errorCode?: string; browserKind?: 'chrome' | 'edge'; browserProfiles?: { profileId: string; bundleId: string; collision: boolean }[] };
function chromeSetup(value: unknown): ChromeSetup {
  const result = value as ChromeSetup | null;
  if (!result || result.schemaVersion !== 'chrome-setup/v1' || typeof result.installed !== 'boolean' || typeof result.configured !== 'boolean' || !['unconfigured', 'offline', 'waiting_browser', 'waiting_page', 'network_unavailable', 'ready', 'busy', 'needs_selection', 'error'].includes(result.status) || typeof result.message !== 'string' || !Number.isSafeInteger(result.clients) || result.clients < 0 || !Number.isSafeInteger(result.usableClients) || result.usableClients < 0 || result.usableClients > result.clients || (result.browserKind !== undefined && !['chrome', 'edge'].includes(result.browserKind))) throw new Error('连接状态读取失败，请点击“连接并检查”重新核对。');
  if (result.browserProfiles !== undefined && (!Array.isArray(result.browserProfiles) || result.browserProfiles.length > 64 || result.browserProfiles.some(profile => !profile || typeof profile.profileId !== 'string' || !/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/.test(profile.profileId) || typeof profile.bundleId !== 'string' || !profile.bundleId || typeof profile.collision !== 'boolean'))) throw new Error('Chrome 浏览器身份列表无效，请重新检查。');
  return result;
}

export function FoundationPanel(props: Props) { return <Stage1Panel {...props}/>; }

export function TasksPanel({ workspace, writable, mutate, inspect }: Props & { inspect: (media: Media) => void }) {
  const [filter, setFilter] = useState('all'), [reconciling, setReconciling] = useState<Job | null>(null), [taskId, setTaskId] = useState('');
  const [detail, setDetail] = useState<Job | null>(null), [detailError, setDetailError] = useState('');
  const [closing, setClosing] = useState<Job | null>(null), [closeConfirmed, setCloseConfirmed] = useState(false), [closeReason, setCloseReason] = useState('');
  async function closeUnknown(event: FormEvent) { event.preventDefault(); if (!closing || !closeConfirmed || !closeReason.trim()) return; const next = await mutate('jobs.close_unknown', { jobId: closing.id, confirmedNotAccepted: true, reason: closeReason.trim() }, '已按你的核对声明关闭旧记录；回执与原材料保留。新生成需由你另行发起。'); if (next) setClosing(null); }
  async function loadDetail(job: Job) { setDetailError(''); try { setDetail(await request<Job>({ action: 'jobs.detail', projectId: workspace.project.id, jobId: job.id })); } catch (reason) { setDetailError(messageOf(reason)); } }
  const jobs = taskRecordsForView(workspace.jobs, filter);
  async function reconcile(event: FormEvent) { event.preventDefault(); if (!reconciling || !taskId.trim()) return; const next = await mutate('jobs.reconcile', { jobId: reconciling.id, providerTaskId: taskId.trim() }, '已开始核对原任务回执，请刷新完整记录确认结果。'); if (next) { setReconciling(null); setTaskId(''); } }
  return <div className="tasks-page"><div className="stage-intro"><div><h2>生成任务与素材准备</h2><p>本地素材准备不会提交视频生成；已有云端任务按原回执恢复。</p><p>删除记录后，已生成图片与执行历史保留，可在“已删除记录”中恢复。</p></div><div className="segmented">{[['all', '全部'], ['image', '图片'], ['video', '视频生成'], ['preparation', '素材准备'], ['text', '文本'], ['unknown', '结果未知'], ['archived', '已删除记录']].map(([id, label]) => <button key={id} aria-pressed={filter === id} onClick={() => setFilter(id)}>{label}</button>)}</div></div>
    {jobs.length ? <div className="job-list">{jobs.map(job => {
      const localPreparation = isLocalVideoPreparation(job), unknown = job.submissionUnknown || /unknown/i.test(job.status), sync = String(job.providerTaskId || '').startsWith('sync:'), title = workspace.units.find(unit => unit.id === job.unitId)?.title || workspace.assets.find(asset => asset.id === job.assetId)?.name || ({ 'foundation.run': '制作基础', 'images.design': '图片提示词设计', 'images.produce': '图片一键制作', 'storyboard.run': '分镜创作' } as Record<string, string>)[String(job.entityId || '').split(':')[0]] || job.unitId || job.assetId || kindName(job.kind), media = workspace.media.filter(item => job.resultMediaIds?.includes(item.id));
      return <article className="surface job-card" key={job.id}><div className="job-heading"><span className="job-icon"><Icon name={localPreparation ? 'story' : job.kind === 'video' ? 'video' : job.kind === 'image' ? 'image' : 'source'}/></span><div><h3>{title} <small>{localPreparation ? '本地素材准备 · ' : ''}{providerName(job.provider)}{job.model ? ` · ${job.model}` : ''}</small></h3><span className="muted">{formatDate(job.updatedAt || job.createdAt)}</span></div><Badge status={unknown ? 'submission_unknown' : job.status}>{localPreparation ? videoPreparationLabel(job) : undefined}</Badge></div><div className="job-body">{job.kind === 'image' && job.provider === 'chrome' && <p className="muted" role="status">{generationProgress(job)}</p>}{localPreparation && <p className="muted">仅保存本地素材包，尚未提交视频生成。启动、刷新和恢复不会自动生成视频。</p>}<dl><div><dt>{localPreparation ? '准备记录' : '本地任务'}</dt><dd>{job.id}</dd></div><div><dt>云端回执</dt><dd>{localPreparation ? '未提交，无云端任务' : job.providerTaskId || '尚未取得'}</dd></div><div><dt>费用估算</dt><dd>{costText(job.estimate)}</dd></div><div><dt>{localPreparation ? '视频生成' : '实际费用'}</dt><dd>{localPreparation ? '尚未生成' : costText(job.actualCost)}</dd></div></dl>{['download_failed', 'recovery_paused'].includes(job.status) && job.result && typeof job.result === 'object' && 'status' in job.result && job.result.status === 'succeeded' ? <p className="muted">云端已完成，原任务下载待恢复；无需重新生成。</p> : null}{job.error && <p className="warning" role="status">{messageOf(job.error)}</p>}{job.kind === 'workflow' && job.result != null && <JsonDetails title="阶段结果与待处理事项" value={job.result}/>} {unknown && <p className="warning">提交结果尚未确认。此任务不会自动重发，请先核对渠道记录。</p>}{media.length > 0 && <div className="task-media">{media.map(item => <div key={item.id}><MediaPreview media={item} compact onInspect={inspect}/></div>)}</div>}</div><div className="job-actions">{job.taskRecordArchived !== true && !localPreparation && <><Button executionAction="jobs.query" icon="refresh" disabled={!writable || !job.providerTaskId || unknown || sync} onClick={() => void mutate('jobs.query', { jobId: job.id }, '已开始查询原任务，稍后刷新核对结果。')}>查询原任务</Button><Button executionAction="jobs.download" icon="download" disabled={!writable || !job.providerTaskId || unknown} onClick={() => void mutate('jobs.download', { jobId: job.id }, '已开始恢复原任务下载，稍后核对文件。')}>恢复下载</Button></>}{job.taskRecordArchived !== true && unknown && <Button executionAction="jobs.reconcile" variant="caution" disabled={!writable} onClick={() => { setReconciling(job); setTaskId(job.providerTaskId || ''); }}>核对任务回执</Button>}{job.taskRecordArchived !== true && unknown && !job.providerTaskId && <Button disabled={!writable} onClick={() => { setClosing(job); setCloseConfirmed(false); setCloseReason(''); }}>确认未受理并关闭记录</Button>}<TaskRecordActions job={job} writable={writable} mutate={mutate}/><Button variant="ghost" icon="source" onClick={() => void loadDetail(job)}>完整记录</Button></div></article>;
    })}</div> : <Empty icon="tasks" title={filter === 'archived' ? '当前没有已删除记录' : '当前没有任务记录'}><p>生成、查询与下载记录会保存到项目中。</p></Empty>}
    {reconciling && <Modal title="核对未知提交" onClose={() => setReconciling(null)}><form onSubmit={event => void reconcile(event)}><p>到 {providerName(reconciling.provider)} 核对本次请求，填写已存在的任务 ID，工作台将查询该任务。</p><label>渠道任务 ID<input required autoFocus value={taskId} onChange={event => setTaskId(event.target.value)} placeholder="粘贴已存在的任务回执"/></label><div className="form-actions"><Button onClick={() => setReconciling(null)}>取消</Button><Button executionAction="jobs.reconcile" type="submit" variant="primary" disabled={!writable || !taskId.trim()}>核对并绑定回执</Button></div></form></Modal>}
    {closing && <Modal title="确认未受理，关闭旧记录" onClose={() => setClosing(null)}><form onSubmit={event => void closeUnknown(event)}><p>请先到 {providerName(closing.provider)} 核对本次请求。如果找到已生成或仍在处理的任务，请返回并绑定任务 ID。</p><p>关闭仅记录你的核对声明，保留原始材料和错误；不会取消云端任务或发起新生成。</p><label className="checkbox-row"><input type="checkbox" checked={closeConfirmed} onChange={event => setCloseConfirmed(event.target.checked)}/>我已核对渠道记录，确认这次请求没有受理</label><label>核对依据<textarea required rows={3} value={closeReason} onChange={event => setCloseReason(event.target.value)} placeholder="例如：核对本次时间范围，官方记录中没有对应任务"/></label><div className="form-actions"><Button onClick={() => setClosing(null)}>返回核对</Button><Button type="submit" variant="caution" disabled={!writable || !closeConfirmed || !closeReason.trim()}>关闭旧记录</Button></div></form></Modal>}
    {detailError && <p role="alert" className="warning">{detailError}</p>}
    {detail && <Modal title="完整任务记录" onClose={() => setDetail(null)}><div className="form-actions"><Button icon="refresh" onClick={() => void loadDetail(detail)}>刷新完整记录</Button></div>{Array.isArray(detail.events) && detail.events.filter(event => event && typeof event === 'object' && ['operation_error', 'local_error'].includes(String(event.type))).map((event, index) => { let value: unknown = event.data; if (typeof value === 'string') { try { value = JSON.parse(value); } catch {} } return <p className="warning" key={index}>{formatDate(event.createdAt)} · {messageOf(value)}</p>; })}<JsonDetails open title="任务身份、快照、回执与结果" value={detail}/></Modal>}
  </div>;
}

export function DreaminaAccountCard({ account: suppliedAccount, workspace, writable, mutate, onError, readiness, setupBusy = false, setupError = '', componentStatus, componentBusy = false, onRefreshComponents, onInstallComponents }: { account?: DreaminaAccount; workspace?: Pick<Workspace, 'dreaminaAccount'>; writable: boolean; mutate: Mutation | GlobalMutation; onError?: (error: string) => void; readiness?: ReadinessCheck; setupBusy?: boolean; setupError?: string; componentStatus?: RuntimeStatus | null; componentBusy?: boolean; onRefreshComponents?: () => void; onInstallComponents?: () => void }) {
  const [confirmLogout, setConfirmLogout] = useState(false);
  const account = suppliedAccount || workspace?.dreaminaAccount;
  const authorizing = account?.status === 'authorizing';
  const authenticated = account?.status === 'authenticated' && account.ready && account.identityVerified;
  const status = authenticated ? '已登录' : authorizing ? '等待网页授权' : account?.status === 'error' ? '连接失败' : account?.status === 'logged_out' ? '未登录' : '尚未检查';
  const credits = authenticated ? account?.credits : undefined;
  const total = typeof credits?.total === 'number' ? credits.total.toLocaleString() : authenticated ? '暂未读取' : '登录后显示';
  const details = [['member', '会员'], ['gift', '赠送'], ['purchase', '购买']].flatMap(([key, label]) => {
    const value = credits?.[key as keyof typeof credits];
    return typeof value === 'number' ? [label + ' ' + value.toLocaleString()] : [];
  });
  const busy = setupBusy || componentBusy || !!readiness?.busy;
  const componentsReady = onRefreshComponents ? componentStatus?.installed === true : true;
  const invalid = componentStatus?.components.some(component => component.state === 'invalid');
  return <section className="surface dreamina-account-card" aria-label="即梦生成 CLI">
    <div className="section-heading compact"><h3>即梦</h3><span className="muted">图片与视频渠道 · 官方 OAuth 登录</span></div>
    {onRefreshComponents && <div className="channel-component-status" data-runtime-provider="dreamina"><div><span className="muted">即梦生成组件</span><strong>{componentBusy ? '正在检查组件…' : componentsReady ? `已安装${componentStatus?.components[0]?.version ? ' · ' + componentStatus.components[0].version : ''}` : invalid ? '组件需要修复' : componentStatus ? '未安装' : '尚未检查'}</strong></div><div className="actions"><Button executionAction="runtime.status" disabled={!writable || busy} onClick={onRefreshComponents}>检查组件</Button>{componentStatus && !componentsReady && !invalid && <Button executionAction="runtime.prepare" variant="primary" disabled={!writable || busy || !componentStatus.installSupported} onClick={onInstallComponents}>{setupBusy ? '正在安装…' : '安装组件'}</Button>}</div></div>}
    {invalid && <p className="warning" role="status">{componentStatus?.components.find(component => component.state === 'invalid')?.message || '组件校验失败，请更新或修复客户端后重新检查。'}</p>}
    <div className="dreamina-session-summary">
      <div><span className="muted">登录状态</span><Badge status={authenticated ? 'available' : authorizing ? 'running' : 'not_configured'}>{status}</Badge></div>
      <div><span className="muted">账户额度</span><strong>{typeof credits?.total === 'number' ? '总额度 ' + total : total}</strong>{details.length > 0 && <small>（{details.join(' / ')}）</small>}</div>
    </div>
    {authorizing && <p className="oauth-authorizing" role="status">请在即梦官方网页确认授权，完成后这里会自动更新。</p>}
    {account?.status === 'error' && <p className="control-hint" role="status">暂时无法读取账户，请刷新状态或点击 OAuth 登录重新授权。</p>}
    <div className="actions">
      <Button executionAction={authorizing ? "dreamina.open_authorization" : "dreamina.login"} variant="primary" disabled={!writable || busy || !componentsReady} onClick={() => void mutate(authorizing ? 'dreamina.open_authorization' : 'dreamina.login', {}, '已调用默认浏览器，请在即梦官方网页确认授权。')}>{authorizing ? '打开授权页' : authenticated ? '重新登录' : 'OAuth 登录'}</Button>
      {authorizing && account?.authorization?.verificationUrl && <Button icon="copy" onClick={() => void navigator.clipboard.writeText(account.authorization!.verificationUrl).catch(() => onError?.('复制失败，请点击打开授权页重试。'))}>复制授权链接</Button>}
      {(authenticated || authorizing) && <Button disabled={!writable || busy} onClick={() => setConfirmLogout(true)}>退出登录</Button>}
      <Button executionAction="dreamina.status" icon="refresh" disabled={!writable || busy || !componentsReady} onClick={() => void mutate('dreamina.status', {}, '已刷新即梦登录状态，请查看账户信息。')}>刷新登录状态</Button>
      {readiness && <Button executionAction="readiness.check" data-readiness-provider="dreamina" icon="refresh" disabled={!writable || busy || !componentsReady || !!readiness.blocked} title={readiness.blocked || undefined} onClick={() => void readiness.check()}>{setupBusy && componentsReady ? '正在检查…' : '检查账户与模型'}</Button>}
    </div>
    {setupBusy && <p className="control-hint" role="status">{componentsReady ? '正在读取账户与模型…' : '正在安装官方组件，请稍候。'}</p>}
    {(setupError || readiness?.error) && <p className="warning" role="alert">{setupError || readiness?.error}</p>}
    {readiness?.result?.catalog.state === 'available' && <p className="control-hint" role="status">已读取 {readiness.result.catalog.models.length} 个模型</p>}
    <p className="control-hint">确认官方网页授权后，账户状态与额度会自动同步。</p>
    {readiness && <details className="details channel-diagnostics"><summary>连接详情与模型目录</summary><ProviderReadinessResult check={readiness}/></details>}
    {confirmLogout && <Modal title="退出即梦登录" onClose={() => setConfirmLogout(false)}><p>即梦 CLI 与 SHUO 可能共享登录状态，退出后需要重新授权。项目和任务记录会保留。</p><div className="form-actions"><Button onClick={() => setConfirmLogout(false)}>取消</Button><Button executionAction="dreamina.logout" variant="caution" disabled={!writable} onClick={() => { setConfirmLogout(false); void mutate('dreamina.logout', {}, '已退出即梦登录。'); }}>退出登录</Button></div></Modal>}
  </section>;
}

export function SettingsPanel({ appSettings, writable, mutate, onError }: { appSettings: AppSettings; writable: boolean; mutate: GlobalMutation; onError?: (error: string) => void }) {
  const cleanSettings = (value: Settings): Settings => ({ ...value, bhToken: '', chromeToken: '', textApiKey: '' });
  const [settings, setSettings] = useState<Settings>(cleanSettings(appSettings.settings));
  const [tab, setTab] = useState('channels');
  const [channelBusy, setChannelBusy] = useState(''), [channelError, setChannelError] = useState<{ provider: string; message: string } | null>(null), [saving, setSaving] = useState(false);
  const [dreaminaComponents, setDreaminaComponents] = useState<RuntimeStatus | null>(null), [componentBusy, setComponentBusy] = useState(false);
  const [chrome, setChrome] = useState<ChromeSetup | null>(null);
  const [managed, setManaged] = useState<ManagedBrowserStatus | null>(null);
  const chromeSequence = useRef(0);
  const componentSequence = useRef(0);
  const settingsRef = useRef(settings), writableRef = useRef(writable), revision = useRef(0), mounted = useRef(true);
  const savedBase = useRef({ revision: appSettings.revision, settings: cleanSettings(appSettings.settings) });
  const channelOperation = useRef<string | null>(null);
  const saveQueue = useRef<Promise<void> | null>(null), savePending = useRef(0);
  if (writableRef.current !== writable) {
    writableRef.current = writable; revision.current++; channelOperation.current = null;
  }
  const dreaminaCheck = useProviderReadiness({ scope: 'global' }, 'dreamina', String(settings.dreaminaPath || ''), writable && tab === 'channels');
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; revision.current++; }; }, []);
  useEffect(() => { if (writable && tab === 'channels') void refreshDreaminaComponents(); return () => { componentSequence.current++; }; }, [writable, tab, settings.dreaminaPath]);
  useEffect(() => {
    const sequence = ++chromeSequence.current, currentRevision = revision.current;
    if (writable && tab === 'channels') void request({ action: 'chrome.status', scope: 'global' }).then(value => {
      const status = chromeSetup(value);
      if (mounted.current && sequence === chromeSequence.current && currentRevision === revision.current) setChrome(status);
    }).catch(reason => { if (mounted.current && sequence === chromeSequence.current && currentRevision === revision.current) setChannelError({ provider: 'chrome', message: messageOf(reason) }); });
    return () => { chromeSequence.current++; };
  }, [writable, tab]);
  useEffect(() => { setSettings(settingsRef.current); setChannelBusy(''); setChannelError(null); setSaving(savePending.current > 0); }, [writable]);
  useEffect(() => {
    if (savedBase.current.revision === appSettings.revision || savePending.current > 0) return;
    if (JSON.stringify(settingsRef.current) !== JSON.stringify(savedBase.current.settings)) return;
    savedBase.current = { revision: appSettings.revision, settings: cleanSettings(appSettings.settings) };
    settingsRef.current = savedBase.current.settings; setSettings(settingsRef.current);
  }, [appSettings.revision, appSettings.settings, saving]);
  function updateFields(patch: Partial<Settings>) { settingsRef.current = { ...settingsRef.current, ...patch }; setSettings(settingsRef.current); }
  function update(key: string, value: string) { updateFields({ [key]: value }); }
  async function refreshDreaminaComponents() {
    if (!mounted.current || !writableRef.current || channelOperation.current !== null) return;
    const sequence = ++componentSequence.current, currentRevision = revision.current;
    setComponentBusy(true); setChannelError(null);
    try {
      const status = await readRuntimeStatus({ scope: 'global' }, 'dreamina', settingsRef.current);
      if (mounted.current && currentRevision === revision.current && sequence === componentSequence.current) setDreaminaComponents(status);
    } catch (reason) {
      if (mounted.current && currentRevision === revision.current && sequence === componentSequence.current) { setDreaminaComponents(null); setChannelError({ provider: 'dreamina', message: messageOf(reason) }); }
    } finally {
      if (mounted.current && currentRevision === revision.current && sequence === componentSequence.current) setComponentBusy(false);
    }
  }
  function queueSettingsWrite<T>(operation: (currentRevision: number) => Promise<T>, cancelled: T): Promise<T> {
    if (!mounted.current || !writableRef.current) return Promise.resolve(cancelled);
    const currentRevision = revision.current;
    savePending.current++; setSaving(true);
    const run = async () => {
      try {
        if (!mounted.current || !writableRef.current || currentRevision !== revision.current) return cancelled;
        return await operation(currentRevision);
      } finally { savePending.current--; if (mounted.current) setSaving(savePending.current > 0); }
    };
    const result = saveQueue.current ? saveQueue.current.then(run, run) : run();
    const tail = result.then(() => {}, () => {});
    saveQueue.current = tail;
    void tail.then(() => { if (saveQueue.current === tail) saveQueue.current = null; });
    return result;
  }
  function saveSettings(override?: Partial<Settings>) {
    return queueSettingsWrite(async currentRevision => {
      // Read the draft and CAS revision when this write starts, after earlier
      // channel saves/auto-discovery have adopted their acknowledged revision.
      const submitted = { ...settingsRef.current, ...override }, values = { ...submitted };
      settingsRef.current = submitted; setSettings(submitted);
      for (const key of ['bhToken', 'chromeToken', 'textApiKey']) if (!values[key]) delete values[key];
      try {
        const next = await mutate('settings.global.save', { settings: values, expectedRevision: savedBase.current.revision }, '软件渠道与执行设置已保存。');
        if (!next || !mounted.current || currentRevision !== revision.current) return false;
        // Keep edits made while the save was in flight, including newly typed credentials.
        const laterEdits = Object.fromEntries(Object.entries(settingsRef.current).filter(([key, value]) => !Object.is(value, submitted[key])));
        savedBase.current = { revision: next.revision, settings: cleanSettings(next.settings) };
        settingsRef.current = { ...savedBase.current.settings, ...laterEdits };
        setSettings(settingsRef.current); return true;
      } catch (reason) {
        if (mounted.current && currentRevision === revision.current) onError?.(messageOf(reason));
        return false;
      }
    }, false);
  }
  async function connectDreamina(action?: Parameters<Mutation>[0], values?: Parameters<Mutation>[1], notice?: string) {
    if (!mounted.current || !writableRef.current || channelOperation.current !== null || (action ? actionBlockReason(action) : dreaminaCheck.blocked)) return null;
    const currentRevision = revision.current;
    if (action === 'dreamina.login') { invalidateProviderReadiness(['dreamina']); dreaminaCheck.reset(); }
    channelOperation.current = 'dreamina'; setChannelBusy('dreamina'); setChannelError(null);
    try {
      const status = await readRuntimeStatus({ scope: 'global' }, 'dreamina', settingsRef.current);
      if (!mounted.current || currentRevision !== revision.current) return null;
      setDreaminaComponents(status);
      if (!status.installed) throw new Error('请先安装或修复即梦生成组件，再登录或检查账户。');
      const patch = status.settings;
      if (!mounted.current || currentRevision !== revision.current || !await saveSettings(patch)) return null;
      if (!mounted.current || currentRevision !== revision.current) return null;
      if (action) return await mutate(action, values, notice);
      const report = await dreaminaCheck.checkPrepared(patch);
      if (!report || !mounted.current || currentRevision !== revision.current) return null;
      return await mutate('dreamina.status', {}, '已刷新即梦账户与模型。');
    } catch (reason) {
      if (mounted.current && currentRevision === revision.current) setChannelError({ provider: 'dreamina', message: messageOf(reason) });
      return null;
    } finally {
      if (mounted.current && currentRevision === revision.current) { channelOperation.current = null; setChannelBusy(''); }
    }
  }
  async function installDreaminaComponents() {
    if (!mounted.current || !writableRef.current || channelOperation.current !== null || actionBlockReason('runtime.prepare') || !dreaminaComponents || dreaminaComponents.installed || !dreaminaComponents.installSupported || dreaminaComponents.components.some(component => component.state === 'invalid')) return;
    const currentRevision = revision.current;
    channelOperation.current = 'dreamina'; setChannelBusy('dreamina'); setChannelError(null);
    try {
      const patch = await prepareRuntime({ scope: 'global' }, 'dreamina', settingsRef.current);
      if (!mounted.current || currentRevision !== revision.current) return;
      const status = await readRuntimeStatus({ scope: 'global' }, 'dreamina', { ...settingsRef.current, ...patch });
      if (!mounted.current || currentRevision !== revision.current) return;
      setDreaminaComponents(status);
      if (!status.installed) throw new Error('组件安装尚未通过校验，请重新检查组件。');
      if (!await saveSettings(status.settings) || !mounted.current || currentRevision !== revision.current) return;
      await mutate('dreamina.status', {}, '即梦组件已安装，已刷新登录状态。');
    } catch (reason) {
      if (mounted.current && currentRevision === revision.current) setChannelError({ provider: 'dreamina', message: messageOf(reason) });
    } finally {
      if (mounted.current && currentRevision === revision.current) { channelOperation.current = null; setChannelBusy(''); }
    }
  }
  const dreaminaMutation: GlobalMutation = (action, values, notice) => {
    if (['dreamina.login', 'dreamina.status'].includes(action)) return connectDreamina(action, values, notice);
    if (action === 'dreamina.logout') { invalidateProviderReadiness(['dreamina']); dreaminaCheck.reset(); }
    return mutate(action, values, notice);
  };
  async function checkProvider(provider: string) {
    if (!mounted.current || !writableRef.current || channelOperation.current !== null || actionBlockReason(provider === 'chrome' ? 'chrome.check' : 'providers.refresh')) return;
    const currentRevision = revision.current;
    channelOperation.current = provider; setChannelBusy(provider); setChannelError(null);
    try {
      if (!await saveSettings() || !mounted.current || currentRevision !== revision.current) return;
      if (provider === 'chrome') {
        const status = chromeSetup(await request({ action: 'chrome.check', scope: 'global' }));
        if (!mounted.current || currentRevision !== revision.current) return;
        setChrome(status);
        if (!['ready', 'busy'].includes(status.status)) return;
      }
      await mutate('providers.refresh', { provider }, '已检查当前配置并更新模型目录。');
    } catch (reason) {
      if (mounted.current && currentRevision === revision.current) setChannelError({ provider, message: messageOf(reason) });
    } finally {
      if (mounted.current && currentRevision === revision.current) { channelOperation.current = null; setChannelBusy(''); }
    }
  }
  async function connectChrome(action: 'chrome.connect' | 'chrome.open' | 'chrome.bind', values: { sourceConfigPath?: string; profileId?: string; managed?: boolean; browser?: 'chrome' | 'edge'; transport?: 'opencli'; allowAccountChange?: boolean } = {}) {
    if (!mounted.current || !writableRef.current || channelOperation.current !== null || actionBlockReason(action)) return;
    const currentRevision = revision.current;
    channelOperation.current = 'chrome'; setChannelBusy('chrome'); setChannelError(null);
    try {
      const connected = await queueSettingsWrite(async () => {
        const next = await mutate(action, values, action === 'chrome.open' ? '已请求打开普通浏览器的 ChatGPT 页面，请检查登录及扩展连接。' : '已检查本机网页桥接，请查看连接状态。');
        if (!next || !mounted.current || currentRevision !== revision.current) return false;
        // Connecting can update the global revision itself. Adopt it inside
        // the write queue before another channel saves its discovered paths.
        const edits = Object.fromEntries(Object.entries(settingsRef.current).filter(([key, value]) => !Object.is(value, savedBase.current.settings[key]) && (action === 'chrome.open' || !['chromeEndpoint', 'chromeAccountId', 'chromeToken', 'chromeProfileId', 'chromeProfileEndpoint', 'chromeProfileBundleId', 'chromeTransport', 'opencliProfileId', 'opencliAccountKey'].includes(key))));
        savedBase.current = { revision: next.revision, settings: cleanSettings(next.settings) };
        settingsRef.current = { ...savedBase.current.settings, ...edits }; setSettings(settingsRef.current);
        return true;
      }, false);
      if (!connected || !mounted.current || currentRevision !== revision.current) return;
      const status = chromeSetup(await request({ action: 'chrome.status', scope: 'global' }));
      if (mounted.current && currentRevision === revision.current) setChrome(status);
      if (action === 'chrome.connect' && values.transport === 'opencli' && status.status === 'ready' && mounted.current && currentRevision === revision.current) {
        await mutate('providers.refresh', { provider: 'chrome' }, '网页出图已连接，可以到第三阶段批量制作图片。');
      }
    } catch (reason) { if (mounted.current && currentRevision === revision.current) setChannelError({ provider: 'chrome', message: messageOf(reason) }); }
    finally { if (mounted.current && currentRevision === revision.current) { channelOperation.current = null; setChannelBusy(''); } }
  }
  async function manageBrowser(action: ManagedBrowserAction) {
    if (!mounted.current || !writableRef.current || saving || channelOperation.current !== null || actionBlockReason(action)) return;
    const currentRevision = revision.current;
    channelOperation.current = 'managed'; setChannelBusy('managed'); setChannelError(null);
    try {
      const status = managedBrowserStatus(await request({ action, scope: 'global' }));
      if (!mounted.current || currentRevision !== revision.current) return;
      setManaged(status);
      const chromeStatus = chromeSetup(await request({ action: 'chrome.status', scope: 'global' }));
      if (mounted.current && currentRevision === revision.current) setChrome(chromeStatus);
    } catch (reason) {
      if (mounted.current && currentRevision === revision.current) setChannelError({ provider: 'managed', message: messageOf(reason) });
    } finally {
      if (mounted.current && currentRevision === revision.current) { channelOperation.current = null; setChannelBusy(''); }
    }
  }
  async function importChromeConfiguration() {
    if (!mounted.current || !writableRef.current || channelOperation.current !== null || actionBlockReason('chrome.connect')) return;
    const currentRevision = revision.current;
    channelOperation.current = 'chrome-import'; setChannelBusy('chrome'); setChannelError(null);
    try {
      const paths = await choose('file');
      if (!mounted.current || currentRevision !== revision.current) return;
      channelOperation.current = null; setChannelBusy('');
      if (paths[0]) await connectChrome('chrome.connect', { sourceConfigPath: paths[0] });
    } catch (reason) { if (mounted.current && currentRevision === revision.current) setChannelError({ provider: 'chrome', message: messageOf(reason) }); }
    finally { if (mounted.current && currentRevision === revision.current && channelOperation.current === 'chrome-import') { channelOperation.current = null; setChannelBusy(''); } }
  }
  async function save(event: FormEvent) { event.preventDefault(); await saveSettings(); }
  const settingsConflict = !saving && savedBase.current.revision !== appSettings.revision && JSON.stringify(settings) !== JSON.stringify(savedBase.current.settings);
  function reloadSettingsDraft() {
    if (saving || channelBusy) return;
    savedBase.current = { revision: appSettings.revision, settings: cleanSettings(appSettings.settings) };
    settingsRef.current = savedBase.current.settings; setSettings(settingsRef.current); setChannelError(null);
  }
  return <div className="settings-page"><div className="stage-intro"><div><h2>软件渠道与账户</h2><p>本机所有项目共用连接设置。项目的画风、尺寸与备份在“制作设置”中调整。</p></div><div className="segmented">{[['channels', '渠道'], ['rules', '规则版本']].map(([id, label]) => <button key={id} aria-pressed={tab === id} onClick={() => setTab(id)}>{label}</button>)}</div></div>
    {settingsConflict && <div className="warning" role="status">软件设置已在别处更新，当前未保存内容仍保留。<Button disabled={!!channelBusy} onClick={reloadSettingsDraft}>放弃本页修改并载入最新设置</Button></div>}
    <form onSubmit={event => void save(event)}>
    {tab === 'channels' && <div className="channel-settings-layout">
      <TextExecutionSettings requestScope={{ scope: 'global' }} settings={settings} writable={writable && !channelBusy && !saving} connectionAllowed={writable} update={update} saveSettings={saveSettings} onError={onError}/>
      <DreaminaAccountCard account={appSettings.dreaminaAccount} writable={writable && !saving && (!channelBusy || channelBusy === 'dreamina')} mutate={dreaminaMutation} onError={onError} readiness={{ ...dreaminaCheck, check: async () => { await connectDreamina(); return undefined; } }} setupBusy={channelBusy === 'dreamina'} setupError={channelError?.provider === 'dreamina' ? channelError.message : ''} componentStatus={dreaminaComponents} componentBusy={componentBusy} onRefreshComponents={() => void refreshDreaminaComponents()} onInstallComponents={() => void installDreaminaComponents()}/>
      <div className="channel-provider-grid">{providerList(appSettings).filter(provider => ['bh', 'chrome'].includes(providerId(provider))).sort((a, b) => (providerId(a) === 'bh' ? 0 : 1) - (providerId(b) === 'bh' ? 0 : 1)).map(provider => {
        const id = providerId(provider), isBh = id === 'bh', title = isBh ? 'BH API' : 'ChatGPT 网页';
        const accountKey = isBh ? 'bhAccountId' : 'chromeAccountId', endpointKey = isBh ? 'bhEndpoint' : 'chromeEndpoint', tokenKey = isBh ? 'bhToken' : 'chromeToken';
        const changed = [accountKey, endpointKey].some(key => String(settings[key] || '') !== String(appSettings.settings[key] || '')) || !!settings[tokenKey];
        const credentialBlocked = actionBlockReason({ action: 'settings.global.save', settings: { [tokenKey]: 'credential' } });
        const configuration = <div className="channel-configuration">
          <label>服务地址<input aria-label={`${title} 服务地址`} value={String(settings[endpointKey] || '')} onChange={event => update(endpointKey, event.target.value)} placeholder={isBh ? 'https://…' : 'http://127.0.0.1:…'}/></label>
          <label>{isBh ? 'API 凭据' : '执行器凭据'}<input aria-label={`${title} 凭据`} disabled={!writable || !!credentialBlocked} title={credentialBlocked || undefined} type="password" autoComplete="new-password" value={String(settings[tokenKey] || '')} onChange={event => update(tokenKey, event.target.value)} placeholder="留空保留已保存凭据"/></label>
          {isBh && <details className="details"><summary>账户标识</summary><label>账户名称<input aria-label="BH API 账户名称" value={String(settings.bhAccountId || '')} onChange={event => update('bhAccountId', event.target.value)} placeholder="例如：BH 主账户"/></label><p className="control-hint">同一账户保留这个名称；更换账户时改用新名称。</p></details>}
        </div>;
        if (!isBh) return <WebImageConnection key={id} status={chrome} busy={channelBusy === 'chrome'} disabled={!writable || saving || !!channelBusy} error={channelError?.provider === 'chrome' ? channelError.message : ''} onConnect={(browser, allowAccountChange) => void connectChrome('chrome.connect', { transport: 'opencli', browser, allowAccountChange })} onOpen={browser => void connectChrome('chrome.open', { browser })} advanced={<>
          <p className="control-hint">组件版本：{chrome?.componentVersion || '尚未读取'}</p>
          <details className="details"><summary>旧版连接参数</summary>{configuration}<Button executionAction="chrome.connect" data-chrome-import icon="upload" disabled={!writable || saving || !!channelBusy} onClick={() => void importChromeConfiguration()}>导入已有连接配置</Button><Button executionAction="chrome.check" data-provider-check="chrome" disabled={!writable || saving || !!channelBusy} onClick={() => void checkProvider('chrome')}>核对保存的连接</Button></details>
          {!!chrome?.browserProfiles?.length && <label>旧版执行浏览器<select aria-label="网页出图执行浏览器" disabled={!writable || saving || !!channelBusy} value={chrome.boundProfileId || ''} onChange={event => { if (event.target.value) void connectChrome('chrome.bind', { profileId: event.target.value }); }}><option value="">请选择浏览器配置</option>{chrome.browserProfiles.map((profile, index) => <option key={profile.profileId} value={profile.profileId} disabled={profile.collision}>浏览器配置 {index + 1}{profile.collision ? '（身份冲突）' : ''}</option>)}</select></label>}
          {chrome?.extensionPath && chrome.transport !== 'opencli' && <details className="details"><summary>首次连接浏览器扩展</summary><p>旧版任务组件位于下方目录；Edge 使用 edge://extensions，Chrome 使用 chrome://extensions。</p><p className="path-value">{chrome.extensionPath}</p></details>}
          <details className="details"><summary>旧版专属浏览器恢复</summary><ManagedBrowserCard status={managed} writable={writable && !saving} busy={!!channelBusy} boundProfileId={chrome?.boundProfileId || ''} error={channelError?.provider === 'managed' ? channelError.message : ''} onAction={action => void manageBrowser(action)} onBind={profileId => void connectChrome('chrome.bind', { profileId, managed: true })}/></details>
          {!!provider.capabilities && <JsonDetails title="连接诊断" value={provider.capabilities}/>}
        </>}/>;
        return <section className="surface channel-provider-card" key={id} aria-label={title}>
          <div className="section-heading compact"><h3>{title}</h3><Badge status={changed ? 'draft' : !isBh && chrome ? chrome.status === 'ready' ? 'available' : chrome.status : provider.available ? 'available' : provider.status || 'not_configured'}>{changed ? '有未检查的修改' : !isBh && chrome ? ({ ready: '已连接', busy: '网页正忙', waiting_browser: '等待网页连接', waiting_page: '待建立工作页', network_unavailable: '浏览器网络异常', offline: '连接待检查', unconfigured: '尚未连接', needs_selection: '需要选择浏览器', error: '连接异常' }[chrome.status]) : undefined}</Badge></div>
          <p className="muted">{changed ? '保存并检查后更新连接状态。' : !isBh && chrome ? chrome.message : provider.message || (provider.configured ? '已保存配置' : isBh ? '填写服务地址和凭据后连接。' : '连接本机 Chrome 组件，在专用浏览器登录后用于网页出图。')}</p>
          {!isBh && <><p className="muted">新任务使用 OpenCLI 控制已登录浏览器，后台生成并核对官方原图后入库。每个资产独立会话，衍生图等待直接父图；断线只查询原任务，不重复发送。旧任务继续使用原连接。</p><div className="channel-component-status"><span>{chrome?.installed ? `本机桥接已安装${chrome.componentVersion ? ' · ' + chrome.componentVersion : ''}` : chrome ? '未找到本机桥接' : '正在检查本机桥接…'}</span><span className="muted">{chrome?.usableClients ? `${chrome.usableClients} 个网页已连接` : '尚未连接浏览器扩展'}</span></div><div className="actions"><Button executionAction="chrome.connect" data-chrome-connect icon="refresh" disabled={!writable || saving || !!channelBusy} onClick={() => void connectChrome('chrome.connect', { transport: 'opencli' })}>{channelBusy === 'chrome' ? '正在处理…' : '连接 OpenCLI 网页出图'}</Button><Button executionAction="chrome.open" data-browser-open="edge" disabled={!writable || saving || !!channelBusy} onClick={() => void connectChrome('chrome.open', { browser: 'edge' })}>打开普通 Edge</Button><Button executionAction="chrome.open" data-browser-open="chrome" disabled={!writable || saving || !!channelBusy} onClick={() => void connectChrome('chrome.open', { browser: 'chrome' })}>打开普通 Chrome</Button></div></>}
          {!isBh && chrome?.checkedAt && <p className="control-hint">上次检查：{formatDate(chrome.checkedAt)}</p>}
          {!isBh && !!chrome?.browserProfiles?.length && <label>执行浏览器<select aria-label="网页出图执行浏览器" disabled={!writable || saving || !!channelBusy} value={chrome.boundProfileId || ''} onChange={event => { if (event.target.value) void connectChrome('chrome.bind', { profileId: event.target.value }); }}><option value="">请选择浏览器配置</option>{chrome.boundProfileId && !chrome.browserProfiles.some(profile => profile.profileId === chrome.boundProfileId) && <option value={chrome.boundProfileId}>已绑定浏览器（未连接）</option>}{chrome.browserProfiles.map((profile, index) => <option key={profile.profileId} value={profile.profileId} disabled={profile.collision}>浏览器配置 {index + 1} · {profile.profileId.slice(0, 8)}{profile.collision ? '（身份冲突）' : ''}</option>)}</select><small className="control-hint">浏览器在全局绑定。后台休眠和断线恢复尚未通过稳定性验收，暂不建议依赖此渠道批量制作。已有任务结果未知时先回收原图，不重复提交。</small></label>}
          {isBh ? configuration : <details className="details"><summary>手动连接设置</summary><Button executionAction="chrome.connect" data-chrome-import icon="upload" disabled={!writable || saving || !!channelBusy} onClick={() => void importChromeConfiguration()}>导入已有连接配置</Button><p className="control-hint">选择旧版的 bridge.env，直接恢复连接，无需复制凭据。</p>{configuration}</details>}
          {channelError?.provider === id && <p className="warning" role="alert">{channelError.message}</p>}
          <div className="actions"><Button executionAction={isBh ? 'providers.refresh' : 'chrome.check'} data-provider-check={id} icon="refresh" disabled={!writable || saving || !!channelBusy} onClick={() => void checkProvider(id)}>{channelBusy === id ? '正在检查…' : changed ? '保存并检查连接' : '检查连接与模型'}</Button></div>
          {!isBh && chrome?.extensionPath && <details className="details" open={chrome.clients === 0 || chrome.status === 'waiting_browser'}><summary>首次连接浏览器扩展</summary><p className="control-hint">本机桥接与浏览器扩展分别连接。请在已登录的浏览器中加载下面的扩展目录：Edge 使用 edge://extensions，Chrome 使用 chrome://extensions。</p><p className="path-value">{chrome.extensionPath}</p><p className="control-hint">加载后刷新 ChatGPT，在实际 ChatGPT 页的 Bridge 面板中填入本机桥接 /setup 页面提供的服务地址和 Bridge token，点击“Save &amp; connect”，然后返回“连接已登录浏览器”。setup 页的绿色状态汇总所有浏览器连接，不表示当前 Edge 已接入或已验证登录。现有登录保留。</p></details>}
          {!!provider.models?.length && <details className="details"><summary>模型目录 · {provider.models.length} 个模型{changed ? '（上次检查）' : ''}</summary><ul className="model-list">{provider.models.map((model, index) => <li key={typeof model === 'string' ? model : model.id || index}>{typeof model === 'string' ? model : model.name || model.id}<small>{typeof model === 'object' ? costText(model.price) : '价格未知'}</small></li>)}</ul></details>}
          {!!provider.capabilities && <JsonDetails title="连接诊断" value={provider.capabilities}/>}
        </section>;
      })}</div>
    </div>}
    {tab === 'rules' && <CreativeRules rules={appSettings.rules}/>}
    {tab !== 'rules' && <div className="form-actions settings-save"><Button type="submit" variant="primary" icon="check" disabled={!writable || saving || !!channelBusy}>{saving ? '正在保存…' : '保存软件设置'}</Button></div>}
    </form>
  </div>;
}

export function ProjectSettingsPanel({ workspace, writable, mutate, onError }: Props) {
  const [settings, setSettings] = useState<Settings>({ ...workspace.settings });
  function updateFields(patch: Partial<Settings>) { setSettings(current => ({ ...current, ...patch })); }
  function update(key: string, value: string) { updateFields({ [key]: value }); }
  const imageParameters = { model: String(settings.imageModel || ''), ratio: String(settings.ratio ?? '16:9'), resolution: String(settings.imageResolution ?? '2k') }, videoParameters = { model: String(settings.videoModel || ''), ratio: String(settings.ratio ?? '16:9'), resolution: String(settings.videoResolution ?? '720p') };
  const imageChoices = generationChoices(workspace, String(settings.imageProvider || 'dreamina'), 'image', imageParameters), videoChoices = generationChoices(workspace, String(settings.videoProvider || 'dreamina'), 'video', videoParameters);
  const defaultProblems = [...(imageParameters.model ? imageChoices.problems.map(problem => '图片：' + problem) : []), ...(videoParameters.model ? videoChoices.problems.map(problem => '视频：' + problem) : [])], commonRatios = imageChoices.selected && videoChoices.selected ? imageChoices.ratios.filter(ratio => videoChoices.ratios.includes(ratio)) : imageChoices.selected ? imageChoices.ratios : videoChoices.ratios;
  async function save(event: FormEvent) {
    event.preventDefault(); if (!writable || defaultProblems.length) return;
    const keys = ['projectVisualStyle', 'visualStyle', 'productionDirection', 'productionDirectionConfirmed', 'imagePresentation', 'fantasyMode', 'requireFantasyReview', 'imageProvider', 'imageModel', 'imageResolution', 'videoProvider', 'videoModel', 'videoResolution', 'ratio'];
    const patch = Object.fromEntries(keys.filter(key => Object.hasOwn(settings, key)).map(key => [key, settings[key]]));
    const next = await mutate('settings.save', { settings: patch }, '此项目的制作设置已保存。');
    if (next) setSettings({ ...next.settings });
  }
  async function restoreBackup() { try { const paths = await choose('file'); if (paths[0]) await mutate('backup.restore', { path: paths[0] }, '项目备份已恢复，请核对版本与媒体。'); } catch (reason) { onError?.(messageOf(reason)); } }
  return <div className="settings-page project-settings-page"><div className="stage-intro"><div><h2>项目制作设置</h2><p>仅用于当前项目。渠道和账户请在软件设置中配置。</p></div></div><form onSubmit={event => void save(event)}>
    <div className="settings-grid"><section className="surface"><h3>制作默认值</h3><ProductionSettings settings={settings} writable={writable} onChange={updateFields}/><div className="form-grid">
      <label>图片渠道<select value={settings.imageProvider || 'dreamina'} onChange={event => updateFields({ imageProvider: event.target.value, imageModel: '', imageResolution: '' })}><option value="dreamina">即梦官方 CLI</option><option value="bh">BH API</option><option value="chrome">ChatGPT 网页</option></select></label>
      <label>图片模型<select value={imageParameters.model} onChange={event => update('imageModel', event.target.value)}><option value="">选择图片模型</option>{imageParameters.model && !imageChoices.models.some(item => item.id === imageParameters.model) && <option value={imageParameters.model}>{imageParameters.model} · 请重新选择</option>}{imageChoices.models.map(item => <option key={item.id} value={item.id} disabled={item.adapted !== true || item.available === false}>{item.name || item.id}{item.adapted !== true ? ' · 待适配' : ''}</option>)}</select></label>
      <label>{imageOutputLabel(imageChoices.selected)}<select value={imageParameters.resolution} onChange={event => update('imageResolution', event.target.value)}><option value="">选择{imageOutputLabel(imageChoices.selected)}</option>{imageParameters.resolution && !imageChoices.resolutions.includes(imageParameters.resolution) && <option value={imageParameters.resolution}>{imageParameters.resolution} · 请重新选择</option>}{imageChoices.resolutions.map(value => <option key={value} value={value}>{imageOutputValueLabel(value, imageChoices.selected)}</option>)}</select></label>
      <label>视频渠道<select value={settings.videoProvider || 'dreamina'} onChange={event => updateFields({ videoProvider: event.target.value, videoModel: '', videoResolution: '' })}><option value="dreamina">即梦官方 CLI</option><option value="bh">BH API</option></select></label>
      <label>视频模型<select value={videoParameters.model} onChange={event => update('videoModel', event.target.value)}><option value="">选择视频模型</option>{videoParameters.model && !videoChoices.models.some(item => item.id === videoParameters.model) && <option value={videoParameters.model}>{videoParameters.model} · 请重新选择</option>}{videoChoices.models.map(item => <option key={item.id} value={item.id} disabled={item.adapted !== true || item.available === false}>{item.name || item.id}{item.adapted !== true ? ' · 待适配' : ''}</option>)}</select></label>
      <label>视频尺寸<select value={videoParameters.resolution} onChange={event => update('videoResolution', event.target.value)}><option value="">选择视频尺寸</option>{videoParameters.resolution && !videoChoices.resolutions.includes(videoParameters.resolution) && <option value={videoParameters.resolution}>{videoParameters.resolution} · 请重新选择</option>}{videoChoices.resolutions.map(value => <option key={value} value={value}>{resolutionLabel(value, 'video')}</option>)}</select></label>
    </div><label>画面比例<select value={imageParameters.ratio} onChange={event => update('ratio', event.target.value)}><option value="">选择共同可用比例</option>{imageParameters.ratio && !commonRatios.includes(imageParameters.ratio) && <option value={imageParameters.ratio}>{imageParameters.ratio} · 请重新选择</option>}{commonRatios.map(value => <option key={value} value={value}>{ratioLabel(value)}</option>)}</select></label>{defaultProblems.length > 0 && <p className="warning">{defaultProblems.join(' ')}</p>}<p className="control-hint">比例按所选图片与视频模型的共同能力列出。尺寸无效时请明确重新选择；保存不会自动降级。</p><div className="note">文本全局 4 路，同集最多 2 路；视频与 Chrome 图片初始每渠道 1 路。连续状态依赖优先。</div></section><section className="surface"><h3>备份与项目目录</h3><p className="path-value">{workspace.project.rootPath || '本机项目目录'}</p><p className="muted">内容变化后每 15 分钟备份。保留 48 份状态快照、2 个检查点，自动备份总量不超过 2 GiB。</p><div className="actions"><Button executionAction="backup.create" icon="download" disabled={!writable} onClick={() => void mutate('backup.create', {}, '一致性备份已创建。')}>立即备份</Button><Button icon="upload" disabled={!writable} onClick={() => void restoreBackup()}>恢复备份</Button></div><p className="muted">原始图片、音频与视频不参与自动清理。</p></section></div>
    <div className="form-actions settings-save"><Button type="submit" variant="primary" icon="check" disabled={!writable || defaultProblems.length > 0}>保存项目设置</Button></div>
  </form></div>;
}
