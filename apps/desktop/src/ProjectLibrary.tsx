import { useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Badge, Button, Empty, formatDate, Icon, messageOf, Modal } from './components';
import type { ProjectSummary } from './types';
import './projectLibrary.css';

export type ProjectFilter = 'active' | 'archived';
export type ProjectLifecycleAction = 'projects.archive' | 'projects.restore' | 'projects.delete';

export function findProjects(projects: ProjectSummary[], query: string): ProjectSummary[] {
  const text = query.trim().toLocaleLowerCase();
  const timestamp = (project: ProjectSummary) => Date.parse(project.createdAt || project.updatedAt || '') || 0;
  return projects.filter(project => !text || project.title.toLocaleLowerCase().includes(text))
    .sort((a, b) => timestamp(b) - timestamp(a));
}
type Operation = { action: 'projects.archive' | 'projects.delete'; project: ProjectSummary };
type LibraryProps = {
  projects: ProjectSummary[]; filter: ProjectFilter; writable: boolean; busy: boolean;
  onFilterChange: (filter: ProjectFilter) => void; onOpen: (project: ProjectSummary) => void;
  createForm?: ReactNode; onLifecycle: (action: ProjectLifecycleAction, project: ProjectSummary) => Promise<boolean>;
};

export function ProjectLifecycleConfirmation({ operation, pending, writable, error, onClose, onConfirm }: {
  operation: Operation; pending: boolean; writable: boolean; error: string; onClose: () => void; onConfirm: () => void;
}) {
  const deleting = operation.action === 'projects.delete';
  return <Modal title={deleting ? '删除已归档项目' : '归档项目'} onClose={onClose}>
    <div className="project-lifecycle-dialog">
      <div className="project-confirm-title"><span className="project-icon"><Icon name="folder" size={24}/></span><strong>{operation.project.title}</strong></div>
      {deleting ? <><p>从本机项目列表移除。原目录的剧本、数据库、图片、视频与任务记录保留。</p><div className="project-retained-path"><span>保留的项目目录</span><code>{operation.project.rootPath || '原项目目录'}</code></div></> : <p>归档后暂停本地队列与查询。云端已提交的任务不会取消，恢复项目后继续查询原任务。</p>}
      {error && <div className="warning" role="alert">{error}</div>}
      <div className="form-actions"><Button disabled={pending} onClick={onClose}>取消</Button><Button variant={deleting ? 'caution' : 'primary'} disabled={!writable || pending} onClick={onConfirm}>{pending ? '正在处理…' : deleting ? '删除项目' : '归档项目'}</Button></div>
    </div>
  </Modal>;
}

export function ProjectLibrary({ projects, filter, writable, busy, onFilterChange, onOpen, createForm, onLifecycle }: LibraryProps) {
  const [operation, setOperation] = useState<Operation | null>(null), [error, setError] = useState(''), [pending, setPending] = useState(false), [query, setQuery] = useState('');
  const inFlight = useRef(false);
  const active = projects.filter(project => !project.archivedAt), archived = projects.filter(project => !!project.archivedAt);
  const visible = findProjects(filter === 'archived' ? archived : active, query);
  function close() { if (!inFlight.current) { setOperation(null); setError(''); } }
  function confirm(action: Operation['action'], project: ProjectSummary) {
    if (!writable || busy || inFlight.current || (action === 'projects.delete' && !project.archivedAt)) return;
    setOperation({ action, project }); setError('');
  }
  async function execute(action: ProjectLifecycleAction, project: ProjectSummary) {
    if (!writable || busy || inFlight.current || (action !== 'projects.archive' && !project.archivedAt) || (action === 'projects.archive' && project.archivedAt)) return;
    inFlight.current = true; setPending(true); setError('');
    try { if (await onLifecycle(action, project)) setOperation(null); }
    catch (reason) { setError(messageOf(reason)); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <div className="projects-page project-library">
    <div className="section-heading"><div><h2>你的制作项目</h2><p>原文、当前图片、分镜和视频，共用一份项目记录。</p></div></div>
    {createForm}
    <div className="project-library-bar"><div className="segmented project-library-tabs" role="tablist" aria-label="项目状态">{(['active', 'archived'] as const).map(value => <button key={value} type="button" role="tab" aria-selected={filter === value} aria-pressed={filter === value} disabled={pending || busy} onClick={() => onFilterChange(value)}>{value === 'active' ? '进行中' : '已归档'}<span>{value === 'active' ? active.length : archived.length}</span></button>)}</div><input className="project-library-search" aria-label="搜索项目" placeholder="搜索项目名称 · 新项目优先" value={query} onChange={event => setQuery(event.target.value)}/>{filter === 'archived' && <small className="muted">恢复后继续制作，删除只移除列表记录。</small>}</div>
    {visible.length ? <div className="project-grid">{visible.map(project => <article className={`project-card ${project.archivedAt ? 'is-archived' : ''}`} key={project.id} data-project-id={project.id}>
      {project.archivedAt ? <div className="project-card-summary"><span className="project-icon"><Icon name="story" size={27}/></span><h3>{project.title}</h3><p>{summary(project)}</p></div> : <button type="button" className="project-card-open" disabled={busy || pending} aria-label={`打开项目：${project.title}`} onClick={() => { if (!busy && !pending) onOpen(project); }}><span className="project-icon"><Icon name="story" size={27}/></span><h3>{project.title}</h3><p>{summary(project)}</p><span className="project-open-hint">打开工作台<Icon name="arrow" size={16}/></span></button>}
      <div className="project-card-actions"><Badge status={project.archivedAt ? 'paused' : 'ready'}>{project.archivedAt ? '已归档' : '制作中'}</Badge>{project.archivedAt ? <span className="project-actions"><Button disabled={!writable || busy || pending} aria-label={`恢复项目：${project.title}`} onClick={() => void execute('projects.restore', project)}>恢复项目</Button><Button variant="ghost" className="project-delete" disabled={!writable || busy || pending} aria-label={`删除项目：${project.title}`} onClick={() => confirm('projects.delete', project)}>删除</Button></span> : <Button variant="ghost" disabled={!writable || busy || pending} aria-label={`归档项目：${project.title}`} onClick={() => confirm('projects.archive', project)}>归档</Button>}</div>
      {project.archivedAt && <small className="project-archived-date">{formatDate(project.archivedAt)} 归档</small>}
    </article>)}</div> : query.trim() ? <Empty title="没有找到匹配的项目" icon="search"><Button onClick={() => setQuery('')}>清除搜索</Button></Empty> : filter === 'archived' ? <Empty title="没有已归档项目" icon="folder"><p>完成制作的项目可以归档，之后随时恢复。</p></Empty> : <Empty title="从一份剧本开始" icon="source"><p>在上方输入项目名称，或导入已保存的制作成果。</p></Empty>}
    {!operation && error && <div className="warning project-library-error" role="alert">{error}</div>}
    {operation && <ProjectLifecycleConfirmation operation={operation} pending={pending} writable={writable && !busy} error={error} onClose={close} onConfirm={() => void execute(operation.action, operation.project)}/>}
  </div>;
}

function summary(project: ProjectSummary) {
  return project.counts ? Object.entries(project.counts).filter(([key]) => ['assets', 'units', 'videos'].includes(key)).map(([key, value]) => `${value} ${key === 'assets' ? '项资产' : key === 'units' ? '个单元' : '个视频'}`).join(' · ') : '剧本、资产与分镜的独立记录';
}
