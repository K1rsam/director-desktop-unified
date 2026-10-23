import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { choose, desktop, isAppSettings, isWorkspace, mediaUrl, preview, request } from './api';
import { createCatalogDiscovery, missingCatalogs, normalizeWorkspace as normalize } from './contracts';
import type { CatalogDiscoveryState } from './contracts';
import { AssetPanel } from './AssetPanel';
import { ExecutionPolicyBanner } from './ExecutionPolicyBanner';
import { actionBlockReason } from './executionPolicy';
import { FoundationPanel, ProjectSettingsPanel, SettingsPanel, TasksPanel } from './Panels';
import { UnitPanel } from './UnitPanel';
import { TextExecutionSummary } from './TextExecutionSettings';
import { ProjectLibrary } from './ProjectLibrary';
import { InlineProjectCreate } from './InlineProjectCreate';
import { LingmoImportDialog } from './LingmoImportDialog';
import { createWorkspacePoller } from './workflowProgressModel';
import { workspacePollIntervalMs } from './generationState';
import type { ProjectFilter, ProjectLifecycleAction } from './ProjectLibrary';
import { Button, Empty, Icon, jobActive, MediaPreview, messageOf, Modal, providerName } from './components';
import type { IconName } from './components';
import type { AppSettings, GlobalMutation, Media, Mutation, Page, ProjectSummary, Workspace } from './types';

const stages: { id: Page; number: string; title: string; subtitle: string; icon: IconName }[] = [
  { id: 'foundation', number: '01', title: '剧本与基础', subtitle: '原文 · 方向 · 资产目录', icon: 'source' },
  { id: 'images', number: '02', title: '图片资产', subtitle: '形象设计 · 当前图 · 历史', icon: 'image' },
  { id: 'storyboard', number: '03', title: '分镜提示词', subtitle: '情绪规划 · 主创 · 整编', icon: 'story' },
  { id: 'videos', number: '04', title: '单元视频', subtitle: '配图绑定 · 生成 · 验收', icon: 'video' },
];
function freshWorkspace(value: Workspace, checkedAt = new Date().toISOString()): Workspace {
  return { ...normalize(value), readState: { checkedAt } };
}
const SOFTWARE_DISPLAY_FIELDS = ['textProvider', 'textModel', 'reasoning', 'textEndpoint', 'codexPath', 'textSdkNodePath', 'dreaminaPath', 'dreaminaCanvasPath', 'bhEndpoint', 'bhAccountId', 'chromeEndpoint', 'chromeAccountId', 'chromeProfileId', 'chromeProfileEndpoint', 'chromeProfileBundleId', 'providerTimeoutSeconds', 'textConcurrency', 'episodeConcurrency', 'videoConcurrency', 'chromeConcurrency'] as const;
/** Render current software connections without changing project records or frozen jobs. */
export function projectWorkspaceView(workspace: Workspace | null, appSettings: AppSettings | null): Workspace | null {
  if (!workspace || !appSettings || !isAppSettings(appSettings)) return workspace;
  const settings = { ...workspace.settings };
  for (const key of SOFTWARE_DISPLAY_FIELDS) if (Object.hasOwn(appSettings.settings, key)) Object.assign(settings, { [key]: appSettings.settings[key] });
  return { ...workspace, settings, providers: appSettings.providers, dreaminaAccount: appSettings.dreaminaAccount };
}

type DreaminaRefresh = { schemaVersion: 'dreamina-refresh/v1'; refreshState: 'pending' | 'completed'; settingsRevision: number; configuredPath?: string | null; afterCheckedAt?: string | null; afterStatus?: string; authorizationExpiresAt?: string | null; accountKey?: string | null; message: string };
function dreaminaRefreshResult(value: unknown): value is DreaminaRefresh {
  if (!value || typeof value !== 'object') return false;
  const item = value as Partial<DreaminaRefresh>;
  return item.schemaVersion === 'dreamina-refresh/v1' && ['pending', 'completed'].includes(item.refreshState || '') && typeof item.settingsRevision === 'number' && typeof item.message === 'string';
}
function completedDreaminaRefresh(receipt: DreaminaRefresh, settings: AppSettings): boolean {
  if (settings.revision !== receipt.settingsRevision || (settings.settings.dreaminaPath || '') !== (receipt.configuredPath || '')) throw new Error('等待期间即梦连接设置已变化，请按当前设置刷新。');
  const account = settings.dreaminaAccount;
  if (receipt.accountKey && account?.accountKey && receipt.accountKey !== account.accountKey) throw new Error('等待期间即梦登录账户已切换，请按当前账户刷新。');
  if (account?.status === 'authorizing') {
    if (receipt.afterStatus !== 'authorizing' || (account.authorization?.expiresAt || '') !== (receipt.authorizationExpiresAt || '')) throw new Error('等待期间即梦授权流程已切换，请按当前账户继续。');
    return false;
  }
  const checked = Date.parse(account?.checkedAt || ''), previous = Date.parse(receipt.afterCheckedAt || '');
  return Number.isFinite(checked) && (!Number.isFinite(previous) || checked > previous);
}

export default function App() {
  const [projects, setProjects] = useState<ProjectSummary[]>([]), [storedWorkspace, setWorkspace] = useState<Workspace | null>(null);
  const [page, setPage] = useState<Page>('projects'), [busy, setBusy] = useState(''), [loading, setLoading] = useState(true), [notice, setNotice] = useState(''), [error, setError] = useState('');
  const [projectFilter, setProjectFilter] = useState<ProjectFilter>('active');
  const [lingmoImportOpen, setLingmoImportOpen] = useState(false);
  const [inspected, setInspected] = useState<Media | null>(null), [mobileNav, setMobileNav] = useState(false);
  const [catalogStates, setCatalogStates] = useState<CatalogDiscoveryState[]>([]);
  const [appSettings, setAppSettings] = useState<AppSettings | null>(null), [settingsLoading, setSettingsLoading] = useState(false), [settingsBusy, setSettingsBusy] = useState('');
  const workspace = useMemo(() => projectWorkspaceView(storedWorkspace, appSettings), [storedWorkspace, appSettings]);
  const globalSequence = useRef(0), globalBusyRef = useRef(''), pageRef = useRef(page);
  const globalQueue = useRef<Promise<unknown>>(Promise.resolve()), globalPending = useRef(0), globalViewRevision = useRef(0);
  if (pageRef.current !== page) globalViewRevision.current++;
  pageRef.current = page;
  const workspaceRef = useRef(workspace), busyRef = useRef(''), requestSequence = useRef(0), projectListSequence = useRef(0);
  const pageScroll = useRef<HTMLDivElement>(null);
  useEffect(() => { if (page === 'projects') pageScroll.current?.scrollTo?.(0, 0); }, [page]);
  const mounted = useRef(true), discoverCatalogs = useRef<ReturnType<typeof createCatalogDiscovery> | null>(null);
  workspaceRef.current = workspace;
  if (!discoverCatalogs.current) discoverCatalogs.current = createCatalogDiscovery({
    enabled: () => desktop && mounted.current && pageRef.current !== 'settings' && !actionBlockReason('providers.refresh'), current: () => workspaceRef.current, busy: () => !!busyRef.current, revision: () => requestSequence.current,
    refresh: async (_projectId, provider) => {
      const next = await globalMutate('providers.refresh', { provider }, '');
      if (!next) throw new Error('软件渠道检查未完成，请到软件设置重试。');
      return next;
    }, read: projectId => request<Workspace>({ action: 'workspace.get', projectId }),
    apply: next => { ++requestSequence.current; const fresh = freshWorkspace(next); workspaceRef.current = fresh; setWorkspace(fresh); },
    report: state => setCatalogStates(current => [...current.filter(item => item.key !== state.key), state]),
  });
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { void discoverCatalogs.current?.(); }, [workspace, busy]);
  const writable = desktop && !busy;
  // Connection controls own their pending state. Their own save must never revoke
  // permission and cancel the following readiness/login step.
  const connectionAllowed = desktop;

  const loadAppSettings = useCallback(async () => {
    if (globalPending.current) return;
    const sequence = ++globalSequence.current;
    setSettingsLoading(true);
    try {
      const migrationSourceProjectId = workspaceRef.current?.project.id;
      const next = await request<unknown>({ action: 'settings.global.get', ...(migrationSourceProjectId ? { migrationSourceProjectId } : {}) });
      if (!isAppSettings(next)) throw new Error('软件设置读取结果无效，请刷新重试。');
      if (mounted.current && sequence === globalSequence.current) setAppSettings(next);
    } catch (reason) { if (mounted.current && sequence === globalSequence.current && pageRef.current === 'settings') setError(messageOf(reason)); }
    finally { if (mounted.current && sequence === globalSequence.current) setSettingsLoading(false); }
  }, []);

  const globalMutate: GlobalMutation = useCallback(async (action, values = {}, success = '软件设置已保存。') => {
    if (!desktop) { if (mounted.current) setError('请在桌面客户端配置软件连接。'); return null; }
    const viewRevision = globalViewRevision.current;
    globalPending.current++; setSettingsBusy(action);
    const run = async () => {
      try {
        if (!mounted.current || viewRevision !== globalViewRevision.current) return null;
        ++globalSequence.current; globalBusyRef.current = action; setSettingsBusy(action);
        setError(''); setNotice('');
        const result = await request<unknown>({ ...values, action, scope: 'global' });
        const initial = isAppSettings(result) ? result : await request<unknown>({ action: 'settings.global.get' });
        if (!isAppSettings(initial)) throw new Error('软件连接状态读取结果无效，请刷新重试。');
        let next: AppSettings = initial;
        let message = action === 'dreamina.status' && result && typeof result === 'object' && 'message' in result && typeof result.message === 'string' ? result.message : success;
        if (action === 'dreamina.status' && dreaminaRefreshResult(result)) {
          // The busy request never retries a CLI check. Poll only the in-memory
          // account projection, with the original settings/account binding.
          let completed = completedDreaminaRefresh(result, next);
          for (let attempt = 0; !completed && attempt < 45; attempt++) {
            if (!mounted.current || viewRevision !== globalViewRevision.current) return null;
            setAppSettings(next); setNotice(result.message);
            await new Promise<void>(resolve => window.setTimeout(resolve, 1000));
            if (!mounted.current || viewRevision !== globalViewRevision.current) return null;
            const snapshot = await request<unknown>({ action: 'settings.global.get' });
            if (!isAppSettings(snapshot)) throw new Error('软件连接状态读取结果无效，请刷新重试。');
            next = snapshot;
            completed = completedDreaminaRefresh(result, next);
          }
          message = completed ? next.dreaminaAccount?.message || '已读取已有操作完成后的最新账户状态。' : '本次尚未取得新的即梦账户检查结果；已有操作仍可继续，请稍后查看账户状态。';
        }
        if (mounted.current) {
          setAppSettings(next);
          if (viewRevision === globalViewRevision.current) setNotice(message);
        }
        return next;
      } catch (reason) { if (mounted.current && viewRevision === globalViewRevision.current) setError(messageOf(reason)); return null; }
      finally {
        globalBusyRef.current = ''; globalPending.current--;
        if (mounted.current) { setSettingsBusy(globalPending.current ? 'queued' : ''); setSettingsLoading(false); }
      }
    };
    const result = globalQueue.current.then(run, run);
    globalQueue.current = result;
    return result;
  }, []);
  useEffect(() => { if (page === 'settings') void loadAppSettings(); }, [page, loadAppSettings]);
  useEffect(() => {
    if (!desktop || page !== 'settings' || appSettings?.dreaminaAccount?.status !== 'authorizing') return;
    const interval = Math.max(1500, Math.min(10000, Number(appSettings.dreaminaAccount.authorization?.pollIntervalSeconds || 3) * 1000));
    const timer = window.setInterval(() => { if (!document.hidden && !globalPending.current) void globalMutate('dreamina.status', {}, ''); }, interval);
    return () => window.clearInterval(timer);
  }, [page, appSettings?.dreaminaAccount?.status, appSettings?.dreaminaAccount?.authorization?.pollIntervalSeconds, globalMutate]);

  const listProjects = useCallback(async () => {
    const sequence = ++projectListSequence.current;
    try { const list = await request<ProjectSummary[]>({ action: 'projects.list' }); if (sequence === projectListSequence.current) setProjects(list); } catch (reason) { if (desktop && sequence === projectListSequence.current) setError(messageOf(reason)); } finally { if (sequence === projectListSequence.current) setLoading(false); }
  }, []);
  useEffect(() => { void listProjects(); }, [listProjects]);
  useEffect(() => {
    if (page !== 'projects') return;
    const refreshLibrary = () => { if (!document.hidden && !busyRef.current) void listProjects(); };
    window.addEventListener('focus', refreshLibrary);
    document.addEventListener('visibilitychange', refreshLibrary);
    return () => { window.removeEventListener('focus', refreshLibrary); document.removeEventListener('visibilitychange', refreshLibrary); };
  }, [page, listProjects]);

  const loadProject = useCallback(async (project: ProjectSummary, target?: Page) => {
    if (project.archivedAt || busyRef.current && !['projects.create', 'project.import'].includes(busyRef.current)) return;
    const sequence = ++requestSequence.current;
    setLoading(true); setError(''); setNotice('');
    try { const next = await request<Workspace>({ action: 'workspace.get', projectId: project.id }); if (sequence === requestSequence.current) { setWorkspace(freshWorkspace(next)); setPage(target || (next.lingmoDocuments?.length && !next.source ? 'videos' : 'foundation')); setMobileNav(false); } } catch (reason) { if (sequence === requestSequence.current) setError(messageOf(reason)); } finally { if (sequence === requestSequence.current) setLoading(false); }
  }, []);

  const projectLifecycle = useCallback(async (action: ProjectLifecycleAction, project: ProjectSummary): Promise<boolean> => {
    if (busyRef.current || !desktop || (action !== 'projects.archive' && !project.archivedAt) || (action === 'projects.archive' && project.archivedAt)) return false;
    busyRef.current = action; setBusy(action); setError(''); setNotice('');
    ++projectListSequence.current;
    try {
      const result = await request<{ archivedAt?: string | null; lifecycleVersion?: number }>({ action, projectId: project.id, expectedVersion: project.lifecycleVersion ?? 0 });
      if (action === 'projects.archive' || action === 'projects.delete') { ++requestSequence.current; setLoading(false); }
      if ((action === 'projects.archive' || action === 'projects.delete') && workspaceRef.current?.project.id === project.id) {
        workspaceRef.current = null; setWorkspace(null); setPage('projects'); setMobileNav(false);
      }
      setProjects(current => action === 'projects.delete' ? current.filter(item => item.id !== project.id) : current.map(item => item.id === project.id ? { ...item, archivedAt: result.archivedAt ?? null, lifecycleVersion: result.lifecycleVersion ?? (project.lifecycleVersion ?? 0) + 1 } : item));
      setNotice(action === 'projects.archive' ? '项目已归档，可在已归档列表恢复或删除。' : action === 'projects.restore' ? '项目已恢复，可以继续制作。' : '已从项目列表移除，原项目目录与文件保留。');
      await listProjects(); return true;
    } catch (reason) { throw new Error(messageOf(reason)); }
    finally { busyRef.current = ''; setBusy(''); }
  }, [listProjects]);

  const mutate: Mutation = useCallback(async (action, values = {}, success = '已保存。') => {
    if (busyRef.current || !desktop) return null;
    const projectId = workspaceRef.current?.project.id;
    if (action === 'providers.refresh') {
      const next = await globalMutate(action, values, success);
      return next && projectId === workspaceRef.current?.project.id ? projectWorkspaceView(workspaceRef.current, next) : null;
    }
    ++requestSequence.current; // An earlier poll must not overwrite this action's newer result.
    busyRef.current = action; setBusy(action); setError(''); setNotice('');
    try {
      const result = await request<unknown>({ action, ...(projectId ? { projectId } : {}), ...values });
      let next: Workspace | null = null;
      if (isWorkspace(result)) next = freshWorkspace(result);
      else if (projectId) next = freshWorkspace(await request<Workspace>({ action: 'workspace.get', projectId }));
      if (next && (next.project.id === workspaceRef.current?.project.id || action === 'backup.restore')) setWorkspace(next);
      const imageQueue = next?.imageQueueReport;
      const videoQueue = next?.videoQueueReport;
      const videoNotice = videoQueue?.blockedCount ? `视频${action === 'videos.submit' ? '提交' : '素材准备'}：${videoQueue.queuedCount + (videoQueue.reusedCount || 0)} 个单元已接收，${videoQueue.blockedCount} 个尚未就绪。具体原因见对应单元的配图与问题。` : '';
      setNotice(videoNotice || (imageQueue?.blockedCount ? `已排队 ${imageQueue.queuedCount} 项图片，${imageQueue.blockedCount} 项尚未就绪。具体原因见资产的任务状态与任务记录。` : success)); void listProjects(); return next;
    } catch (reason) { setError(messageOf(reason)); return null; } finally { busyRef.current = ''; setBusy(''); }
  }, [listProjects, globalMutate]);

  useEffect(() => {
    if (!desktop || !workspace?.project.id) return;
    let alive = true;
    const poll = createWorkspacePoller({
      current: () => workspaceRef.current, busy: () => !!busyRef.current,
      hidden: () => document.hidden, revision: () => requestSequence.current, alive: () => alive,
      // This view reads persisted tasks; it never queries or submits a generation channel.
      read: projectId => request<Workspace>({ action: 'workspace.get', projectId }),
      apply: (next, checkedAt) => setWorkspace(freshWorkspace(next, checkedAt)),
      fail: (reason, projectId, failedAt) => setWorkspace(current => current?.project.id === projectId ? { ...current, readState: { ...current.readState, failedAt, error: messageOf(reason) } } : current),
    });
    // A hidden desktop is still producing work. Only idle projects may pause background reads.
    // Keep one poller (and its in-flight lock) when job activity changes.
    // The lightweight clock ticks every 2s; idle workspace reads remain every 8s.
    let lastRead = Date.now();
    const timer = window.setInterval(() => {
      const now = Date.now();
      if (now - lastRead < workspacePollIntervalMs(workspaceRef.current?.jobs || [])) return;
      lastRead = now;
      void poll();
    }, 2000);
    const visible = () => { if (!document.hidden) void poll(true); };
    window.addEventListener('focus', visible);
    document.addEventListener('visibilitychange', visible);
    return () => { alive = false; window.clearInterval(timer); window.removeEventListener('focus', visible); document.removeEventListener('visibilitychange', visible); };
  }, [workspace?.project.id]);

  async function refresh() { if (page === 'settings') { await loadAppSettings(); return; } if (busy || !workspace) return; await loadProject(workspace.project, page); setNotice('已读取最新保存状态。'); }
  async function createProject(title: string): Promise<boolean> {
    const cleanTitle = title.trim();
    if (!cleanTitle || cleanTitle.length > 160 || busyRef.current || !desktop) return false;
    busyRef.current = 'projects.create'; setBusy('projects.create'); setError('');
    try { const project = await request<ProjectSummary>({ action: 'projects.create', title: cleanTitle }); setProjectFilter('active'); await listProjects(); await loadProject(project, 'foundation'); return true; } catch (reason) { setError(messageOf(reason)); return false; } finally { busyRef.current = ''; setBusy(''); }
  }
  async function importProject() {
    if (busyRef.current || !desktop) return;
    busyRef.current = 'project.import'; setBusy('project.import'); setError('');
    try { const paths = await choose('file'); if (!paths[0]) return; const result = await request<Workspace | ProjectSummary>({ action: 'project.import', path: paths[0] }); setProjectFilter('active'); await listProjects(); if (isWorkspace(result)) { setWorkspace(normalize(result)); setPage('videos'); } else if (result?.id) await loadProject(result, 'videos'); setNotice('项目包已导入，可继续整理资料和制作。'); } catch (reason) { setError(messageOf(reason)); } finally { busyRef.current = ''; setBusy(''); }
  }
  async function openImportedStoryboard(result: Workspace) {
    ++requestSequence.current;
    const next = freshWorkspace(result);
    workspaceRef.current = next; setWorkspace(next); setProjectFilter('active');
    setPage('videos'); setMobileNav(false); setLingmoImportOpen(false); setError('');
    setNotice('分镜已导入，现在可以按集场浏览、配图和继续制作。');
    await listProjects();
  }
  async function exportProject() { const paths = await choose('directory'); if (paths[0] && workspace) { const filename = `${workspace.project.title.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_')}-${new Date().toISOString().replace(/[:.]/g, '-')}.director.zip`; await mutate('project.export', { path: `${paths[0].replace(/[\\/]$/, '')}/${filename}` }, `项目包已导出：${filename}`); } }
  const selectedStage = stages.find(stage => stage.id === page), activeJobs = workspace?.jobs.filter(jobActive).length || 0;
  const pageTitle = selectedStage?.title || ({ projects: '所有项目', jobs: '任务记录', settings: '软件设置', 'project-settings': '项目制作设置' } as Partial<Record<Page, string>>)[page];
  const missingCatalogKeys = new Set(workspace ? missingCatalogs(workspace).map(item => item.key) : []), catalogNotices = catalogStates.filter(item => missingCatalogKeys.has(item.key) && item.status !== 'ready');
  function navigate(target: Page) { setPage(target); setMobileNav(false); setError(''); setNotice(''); if (target === 'projects') void listProjects(); }

  return <div className="app-shell">
    <aside className={`sidebar ${mobileNav ? 'open' : ''}`}>
      <div className="brand"><span className="brand-mark"><Icon name="story" size={25}/></span><div><strong>分镜导演 Next</strong><span>独立本地制作工作台</span></div></div>
      <button className={`sidebar-link ${page === 'projects' ? 'selected' : ''}`} onClick={() => navigate('projects')}><Icon name="home"/><span>所有项目</span><small>{projects.filter(project => !project.archivedAt).length}</small></button>
      {page !== 'settings' && <><div className="sidebar-project"><span className="eyebrow">当前项目</span><strong title={workspace?.project.title}>{workspace?.project.title || '选择或创建项目'}</strong></div>
      <nav className="stage-nav" aria-label="制作阶段">{stages.map(stage => <button key={stage.id} disabled={!workspace} className={`stage-link ${page === stage.id ? 'selected' : ''}`} onClick={() => navigate(stage.id)}><span className="stage-number">{stage.number}</span><span className="stage-label"><strong>{stage.title}</strong><small>{stage.subtitle}</small></span></button>)}</nav></>}
      <div className="sidebar-bottom"><button className={`sidebar-link ${page === 'jobs' ? 'selected' : ''}`} disabled={!workspace} onClick={() => navigate('jobs')}><Icon name="tasks"/><span>任务记录</span>{activeJobs > 0 && <small>{activeJobs}</small>}</button><button className={`sidebar-link ${page === 'settings' ? 'selected' : ''}`} onClick={() => navigate('settings')}><Icon name="settings"/><span>软件设置</span></button><div className="local-status"><span className={`status-dot ${desktop ? 'online' : ''}`}/>{desktop ? '本地核心已连接' : preview ? '浏览器预览 · 示例资料' : '浏览器 · 未连接本地核心'}<small>v0.1.0</small></div></div>
    </aside>
    {mobileNav && <button className="nav-shade" aria-label="关闭导航" onClick={() => setMobileNav(false)}/>}
    <main className="main-area">
      <header className="topbar"><div className="topbar-title"><Button icon="story" variant="ghost" className="mobile-nav-toggle" aria-label="打开导航" onClick={() => setMobileNav(!mobileNav)}/><div><span className="eyebrow">{page === 'settings' ? '本机所有项目共用' : selectedStage ? `阶段 ${selectedStage.number} / 04` : 'DIRECTOR DESKTOP NEXT'}{workspace && !['projects', 'settings'].includes(page) ? ` · ${workspace.project.title}` : ''}</span><h1>{pageTitle}</h1></div></div><div className="topbar-actions">{(page === 'settings' ? settingsBusy : busy) && <span className="pending-text" role="status"><span className="spinner"/>正在处理</span>}{page === 'settings' && <Button icon="refresh" disabled={!!settingsBusy || settingsLoading} onClick={() => void refresh()}>刷新设置</Button>}{workspace && !['projects', 'settings'].includes(page) && <><Button icon="settings" disabled={!!busy} onClick={() => navigate('project-settings')}>制作设置</Button><Button icon="refresh" disabled={!!busy || loading} onClick={() => void refresh()}>刷新</Button><Button icon="download" disabled={!writable} onClick={() => void exportProject().catch(reason => setError(messageOf(reason)))}>导出项目</Button></>}{page === 'projects' && <Button icon="upload" disabled={!writable} onClick={() => void importProject()}>导入项目包</Button>}</div></header>
      <ExecutionPolicyBanner globalSettings={page === 'settings'}/>
      {!desktop && <div className="mode-banner">{preview ? '这是只读界面预览，图像与剧本均为示例；没有连接项目数据，也不会生成或提交任务。' : '浏览器预览未连接本地核心。制作与文件管理请在桌面客户端操作。'}{!preview && <a href="?preview=1">查看界面示例 <Icon name="arrow" size={15}/></a>}</div>}
      {(error || notice) && <div className={`notification ${error ? 'error' : ''}`} role={error ? 'alert' : 'status'}><span>{error || notice}</span><Button variant="ghost" icon="close" aria-label="关闭提示" onClick={() => { setError(''); setNotice(''); }}/></div>}
      {desktop && !['projects', 'settings'].includes(page) && catalogNotices.map(item => <div key={item.key} className={`notification ${item.status === 'error' ? 'error' : ''}`} role="status"><span>{providerName(item.provider)}：{item.status === 'checking' ? '正在读取渠道能力与模型目录。' : `自动读取未完成：${item.error} 请到软件设置检查连接。`}</span><Button variant="ghost" icon="settings" onClick={() => navigate('settings')}>软件设置</Button></div>)}
      {workspace?.readState?.error && !['projects', 'settings'].includes(page) && <div className="notification error" role="status"><span>最新制作状态读取失败，以下为上次保存状态。正在继续读取：{workspace.readState.error}</span></div>}
      <div className="page-content" ref={pageScroll}>
        {workspace && ['foundation', 'images', 'storyboard'].includes(page) && <TextExecutionSummary settings={workspace.settings} stage={page as 'foundation' | 'images' | 'storyboard'} onConfigure={() => navigate('settings')}/>}
        {page === 'settings' ? appSettings ? <SettingsPanel appSettings={appSettings} writable={connectionAllowed} mutate={globalMutate} onError={setError}/> : <Empty title={settingsLoading ? '正在读取软件设置…' : '软件设置尚未读取'}><Button icon="refresh" disabled={settingsLoading} onClick={() => void loadAppSettings()}>重新读取</Button></Empty> : loading && !workspace ? <Empty title="正在读取项目…"/> : page === 'projects' ? <><section className="lingmo-home-import" aria-label="从现成分镜开始"><span className="lingmo-file-icon"><Icon name="story" size={25}/></span><div><h2>已经有分镜？直接开始制作</h2><p>导入分镜 MD，自动建立项目，直接进入集场工作台。</p></div><Button icon="upload" variant="primary" disabled={!writable} onClick={() => setLingmoImportOpen(true)}>导入分镜 MD</Button></section><ProjectLibrary projects={projects} filter={projectFilter} writable={writable} busy={!!busy} onFilterChange={setProjectFilter} onOpen={project => void loadProject(project)} createForm={<InlineProjectCreate writable={writable} busy={!!busy} onCreate={createProject}/>} onLifecycle={projectLifecycle}/><div className="flow-guide">{stages.map(stage => <div key={stage.id}><span>{stage.number}</span><Icon name={stage.icon}/><strong>{stage.title}</strong><small>{stage.subtitle}</small></div>)}</div></> : workspace ? <>
          {page === 'foundation' && <FoundationPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} onNotice={setNotice} onError={setError}/>}
          {page === 'images' && <AssetPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} inspect={setInspected} onNotice={setNotice} onError={setError} onTasks={() => setPage('jobs')}/>}
          {(page === 'storyboard' || page === 'videos') && <UnitPanel key={`${workspace.project.id}-${page}`} workspace={workspace} mode={page} writable={writable} mutate={mutate} inspect={setInspected} onNotice={setNotice} onError={setError}/>}
          {page === 'jobs' && <TasksPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} inspect={setInspected}/>}
          {page === 'project-settings' && <ProjectSettingsPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} onError={setError}/>}
        </> : <Empty title="先选择一个项目"/>}
      </div>
    </main>
    {lingmoImportOpen && <LingmoImportDialog writable={writable} onClose={() => setLingmoImportOpen(false)} onImported={openImportedStoryboard}/>}
    {inspected && <Modal title={inspected.name || '原始媒体'} onClose={() => setInspected(null)}><div className="full-media"><MediaPreview media={inspected}/></div><div className="form-actions"><small className="muted">{inspected.sha256 || inspected.hash ? `SHA-256 ${inspected.sha256 || inspected.hash}` : '原始文件'}</small>{mediaUrl(inspected) && <a className="button secondary" href={mediaUrl(inspected)} download={inspected.name || inspected.relativePath?.split('/').at(-1) || 'media'}>下载原文件</a>}</div></Modal>}
  </div>;
}
