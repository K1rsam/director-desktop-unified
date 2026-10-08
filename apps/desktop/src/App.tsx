import { useCallback, useEffect, useRef, useState } from 'react';
import { choose, desktop, isWorkspace, mediaUrl, preview, request } from './api';
import { createCatalogDiscovery, missingCatalogs, normalizeWorkspace as normalize } from './contracts';
import type { CatalogDiscoveryState } from './contracts';
import { AssetPanel } from './AssetPanel';
import { ExecutionPolicyBanner } from './ExecutionPolicyBanner';
import { actionBlockReason } from './executionPolicy';
import { FoundationPanel, SettingsPanel, TasksPanel } from './Panels';
import { UnitPanel } from './UnitPanel';
import { TextExecutionSummary } from './TextExecutionSettings';
import { ProjectLibrary } from './ProjectLibrary';
import { InlineProjectCreate } from './InlineProjectCreate';
import { createWorkspacePoller } from './workflowProgressModel';
import type { ProjectFilter, ProjectLifecycleAction } from './ProjectLibrary';
import { Button, Empty, Icon, jobActive, MediaPreview, messageOf, Modal, providerName } from './components';
import type { IconName } from './components';
import type { Media, Mutation, Page, ProjectSummary, Workspace } from './types';

const stages: { id: Page; number: string; title: string; subtitle: string; icon: IconName }[] = [
  { id: 'foundation', number: '01', title: '剧本与基础', subtitle: '原文 · 方向 · 资产目录', icon: 'source' },
  { id: 'images', number: '02', title: '图片资产', subtitle: '形象设计 · 当前图 · 历史', icon: 'image' },
  { id: 'storyboard', number: '03', title: '分镜提示词', subtitle: '情绪规划 · 主创 · 整编', icon: 'story' },
  { id: 'videos', number: '04', title: '单元视频', subtitle: '配图绑定 · 生成 · 验收', icon: 'video' },
];
function freshWorkspace(value: Workspace, checkedAt = new Date().toISOString()): Workspace {
  return { ...normalize(value), readState: { checkedAt } };
}

export default function App() {
  const [projects, setProjects] = useState<ProjectSummary[]>([]), [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [page, setPage] = useState<Page>('projects'), [busy, setBusy] = useState(''), [loading, setLoading] = useState(true), [notice, setNotice] = useState(''), [error, setError] = useState('');
  const [projectFilter, setProjectFilter] = useState<ProjectFilter>('active');
  const [inspected, setInspected] = useState<Media | null>(null), [mobileNav, setMobileNav] = useState(false);
  const [catalogStates, setCatalogStates] = useState<CatalogDiscoveryState[]>([]);
  const workspaceRef = useRef(workspace), busyRef = useRef(''), requestSequence = useRef(0), projectListSequence = useRef(0);
  const mounted = useRef(true), discoverCatalogs = useRef<ReturnType<typeof createCatalogDiscovery> | null>(null);
  workspaceRef.current = workspace;
  if (!discoverCatalogs.current) discoverCatalogs.current = createCatalogDiscovery({
    enabled: () => desktop && mounted.current && !actionBlockReason('providers.refresh'), current: () => workspaceRef.current, busy: () => !!busyRef.current, revision: () => requestSequence.current,
    refresh: (projectId, provider) => request({ action: 'providers.refresh', projectId, provider }), read: projectId => request<Workspace>({ action: 'workspace.get', projectId }),
    apply: next => { ++requestSequence.current; const fresh = freshWorkspace(next); workspaceRef.current = fresh; setWorkspace(fresh); },
    report: state => setCatalogStates(current => [...current.filter(item => item.key !== state.key), state]),
  });
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { void discoverCatalogs.current?.(); }, [workspace, busy]);
  const writable = desktop && !busy;

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

  const loadProject = useCallback(async (project: ProjectSummary, target: Page = 'foundation') => {
    if (project.archivedAt || busyRef.current && !['projects.create', 'project.import'].includes(busyRef.current)) return;
    const sequence = ++requestSequence.current;
    setLoading(true); setError(''); setNotice('');
    try { const next = await request<Workspace>({ action: 'workspace.get', projectId: project.id }); if (sequence === requestSequence.current) { setWorkspace(freshWorkspace(next)); setPage(target); setMobileNav(false); } } catch (reason) { if (sequence === requestSequence.current) setError(messageOf(reason)); } finally { if (sequence === requestSequence.current) setLoading(false); }
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
  }, [listProjects]);

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
    const timer = window.setInterval(() => void poll(), 8000);
    const visible = () => { if (!document.hidden) void poll(true); };
    window.addEventListener('focus', visible);
    document.addEventListener('visibilitychange', visible);
    return () => { alive = false; window.clearInterval(timer); window.removeEventListener('focus', visible); document.removeEventListener('visibilitychange', visible); };
  }, [workspace?.project.id]);

  async function refresh() { if (busy || !workspace) return; await loadProject(workspace.project, page); setNotice('已读取最新保存状态。'); }
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
  async function exportProject() { const paths = await choose('directory'); if (paths[0] && workspace) { const filename = `${workspace.project.title.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_')}-${new Date().toISOString().replace(/[:.]/g, '-')}.director.zip`; await mutate('project.export', { path: `${paths[0].replace(/[\\/]$/, '')}/${filename}` }, `项目包已导出：${filename}`); } }
  const selectedStage = stages.find(stage => stage.id === page), activeJobs = workspace?.jobs.filter(jobActive).length || 0;
  const pageTitle = selectedStage?.title || ({ projects: '所有项目', jobs: '任务记录', settings: '渠道与执行设置' } as Partial<Record<Page, string>>)[page];
  const missingCatalogKeys = new Set(workspace ? missingCatalogs(workspace).map(item => item.key) : []), catalogNotices = catalogStates.filter(item => missingCatalogKeys.has(item.key) && item.status !== 'ready');
  function navigate(target: Page) { setPage(target); setMobileNav(false); setError(''); setNotice(''); if (target === 'projects') void listProjects(); }

  return <div className="app-shell">
    <aside className={`sidebar ${mobileNav ? 'open' : ''}`}>
      <div className="brand"><span className="brand-mark"><Icon name="story" size={25}/></span><div><strong>分镜导演 Next</strong><span>独立本地制作工作台</span></div></div>
      <button className={`sidebar-link ${page === 'projects' ? 'selected' : ''}`} onClick={() => navigate('projects')}><Icon name="home"/><span>所有项目</span><small>{projects.filter(project => !project.archivedAt).length}</small></button>
      <div className="sidebar-project"><span className="eyebrow">当前项目</span><strong title={workspace?.project.title}>{workspace?.project.title || '选择或创建项目'}</strong></div>
      <nav className="stage-nav" aria-label="制作阶段">{stages.map(stage => <button key={stage.id} disabled={!workspace} className={`stage-link ${page === stage.id ? 'selected' : ''}`} onClick={() => navigate(stage.id)}><span className="stage-number">{stage.number}</span><span className="stage-label"><strong>{stage.title}</strong><small>{stage.subtitle}</small></span></button>)}</nav>
      <div className="sidebar-bottom"><button className={`sidebar-link ${page === 'jobs' ? 'selected' : ''}`} disabled={!workspace} onClick={() => navigate('jobs')}><Icon name="tasks"/><span>任务记录</span>{activeJobs > 0 && <small>{activeJobs}</small>}</button><button className={`sidebar-link ${page === 'settings' ? 'selected' : ''}`} disabled={!workspace} onClick={() => navigate('settings')}><Icon name="settings"/><span>渠道与设置</span></button><div className="local-status"><span className={`status-dot ${desktop ? 'online' : ''}`}/>{desktop ? '本地核心已连接' : preview ? '浏览器预览 · 示例资料' : '浏览器 · 未连接本地核心'}<small>v0.1.0</small></div></div>
    </aside>
    {mobileNav && <button className="nav-shade" aria-label="关闭导航" onClick={() => setMobileNav(false)}/>}
    <main className="main-area">
      <header className="topbar"><div className="topbar-title"><Button icon="story" variant="ghost" className="mobile-nav-toggle" aria-label="打开导航" onClick={() => setMobileNav(!mobileNav)}/><div><span className="eyebrow">{selectedStage ? `阶段 ${selectedStage.number} / 04` : 'DIRECTOR DESKTOP NEXT'}{workspace && page !== 'projects' ? ` · ${workspace.project.title}` : ''}</span><h1>{pageTitle}</h1></div></div><div className="topbar-actions">{busy && <span className="pending-text" role="status"><span className="spinner"/>正在处理</span>}{workspace && page !== 'projects' && <><Button icon="refresh" disabled={!!busy || loading} onClick={() => void refresh()}>刷新</Button><Button icon="download" disabled={!writable} onClick={() => void exportProject().catch(reason => setError(messageOf(reason)))}>导出项目</Button></>}{page === 'projects' && <Button icon="upload" disabled={!writable} onClick={() => void importProject()}>导入项目包</Button>}</div></header>
      <ExecutionPolicyBanner/>
      {!desktop && <div className="mode-banner">{preview ? '这是只读界面预览，图像与剧本均为示例；没有连接项目数据，也不会生成或提交任务。' : '浏览器预览未连接本地核心。制作与文件管理请在桌面客户端操作。'}{!preview && <a href="?preview=1">查看界面示例 <Icon name="arrow" size={15}/></a>}</div>}
      {(error || notice) && <div className={`notification ${error ? 'error' : ''}`} role={error ? 'alert' : 'status'}><span>{error || notice}</span><Button variant="ghost" icon="close" aria-label="关闭提示" onClick={() => { setError(''); setNotice(''); }}/></div>}
      {desktop && page !== 'projects' && catalogNotices.map(item => <div key={item.key} className={`notification ${item.status === 'error' ? 'error' : ''}`} role="status"><span>{providerName(item.provider)}：{item.status === 'checking' ? '正在读取渠道能力与模型目录。' : `自动读取未完成：${item.error} 请到渠道与设置点击“检查连接与模型”重试。`}</span>{page !== 'settings' && <Button variant="ghost" icon="settings" onClick={() => navigate('settings')}>渠道与设置</Button>}</div>)}
      {workspace?.readState?.error && page !== 'projects' && <div className="notification error" role="status"><span>最新制作状态读取失败，以下为上次保存状态。正在继续读取：{workspace.readState.error}</span></div>}
      <div className="page-content">
        {workspace && ['foundation', 'images', 'storyboard'].includes(page) && <TextExecutionSummary settings={workspace.settings} stage={page as 'foundation' | 'images' | 'storyboard'} onConfigure={() => navigate('settings')}/>}
        {loading && !workspace ? <Empty title="正在读取项目…"/> : page === 'projects' ? <><ProjectLibrary projects={projects} filter={projectFilter} writable={writable} busy={!!busy} onFilterChange={setProjectFilter} onOpen={project => void loadProject(project)} createForm={<InlineProjectCreate writable={writable} busy={!!busy} onCreate={createProject}/>} onLifecycle={projectLifecycle}/><div className="flow-guide">{stages.map(stage => <div key={stage.id}><span>{stage.number}</span><Icon name={stage.icon}/><strong>{stage.title}</strong><small>{stage.subtitle}</small></div>)}</div></> : workspace ? <>
          {page === 'foundation' && <FoundationPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} onNotice={setNotice} onError={setError}/>}
          {page === 'images' && <AssetPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} inspect={setInspected} onNotice={setNotice} onError={setError} onTasks={() => setPage('jobs')}/>}
          {(page === 'storyboard' || page === 'videos') && <UnitPanel key={`${workspace.project.id}-${page}`} workspace={workspace} mode={page} writable={writable} mutate={mutate} inspect={setInspected} onNotice={setNotice} onError={setError}/>}
          {page === 'jobs' && <TasksPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} inspect={setInspected}/>}
          {page === 'settings' && <SettingsPanel key={workspace.project.id} workspace={workspace} writable={writable} mutate={mutate} onError={setError}/>}
        </> : <Empty title="先选择一个项目"/>}
      </div>
    </main>
    {inspected && <Modal title={inspected.name || '原始媒体'} onClose={() => setInspected(null)}><div className="full-media"><MediaPreview media={inspected}/></div><div className="form-actions"><small className="muted">{inspected.sha256 || inspected.hash ? `SHA-256 ${inspected.sha256 || inspected.hash}` : '原始文件'}</small>{mediaUrl(inspected) && <a className="button secondary" href={mediaUrl(inspected)} download={inspected.name || inspected.relativePath?.split('/').at(-1) || 'media'}>下载原文件</a>}</div></Modal>}
  </div>;
}
