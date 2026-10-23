import { useRef, useState } from 'react';
import { choose, request } from './api';
import { Button, Icon, messageOf, Modal } from './components';
import { lingmoImportBlocked } from './lingmo';
import type { LingmoPreview, Workspace } from './types';
import './lingmoImport.css';

type Preview = LingmoPreview & { revisionComparison?: { existingDocumentIds: string[]; changes?: unknown } };
type Props = { projectId?: string; writable: boolean; onImported: (workspace: Workspace) => void | Promise<void>; onClose: () => void };
const filename = (path: string) => path.split(/[\\/]/).at(-1) || path;

export function LingmoImportDialog({ projectId, writable, onImported, onClose }: Props) {
  const [path, setPath] = useState(''), [title, setTitle] = useState('');
  const [catalogPath, setCatalogPath] = useState(''), [imageZipPath, setImageZipPath] = useState(''), [imageDirectory, setImageDirectory] = useState('');
  const [preview, setPreview] = useState<Preview | null>(null), [phase, setPhase] = useState<'idle' | 'preview' | 'import'>('idle');
  const [error, setError] = useState(''), [allowNewRevision, setAllowNewRevision] = useState(false);
  const [completed, setCompleted] = useState<Workspace | null>(null);
  const inFlight = useRef(false), sequence = useRef(0);
  const busy = phase !== 'idle', newProject = !projectId;
  const close = () => { if (!inFlight.current) { sequence.current++; if (completed) void onImported(completed); else onClose(); } };
  async function readPreview(nextPath: string, nextCatalog = catalogPath) {
    const current = ++sequence.current;
    setPhase('preview'); setError(''); setPreview(null); setAllowNewRevision(false);
    try {
      const result = await request<Preview>({ action: projectId ? 'lingmo.preview' : 'lingmo.preview_file', ...(projectId ? { projectId } : {}), path: nextPath, ...(nextCatalog ? { catalogPath: nextCatalog } : {}) });
      if (sequence.current === current) setPreview(result);
    } catch (reason) { if (sequence.current === current) setError(messageOf(reason)); }
    finally { if (sequence.current === current) setPhase('idle'); }
  }
  async function pick(kind: 'md' | 'catalog' | 'zip' | 'directory') {
    if (inFlight.current || busy) return;
    try {
      const [selected] = await choose(kind === 'directory' ? 'directory' : 'file');
      if (!selected) return;
      if (kind === 'md') { setPath(selected); setTitle(filename(selected).replace(/\.md$/i, '').slice(0, 160)); await readPreview(selected); }
      if (kind === 'catalog') { setCatalogPath(selected); if (path) await readPreview(path, selected); }
      if (kind === 'zip') { setImageZipPath(selected); setImageDirectory(''); }
      if (kind === 'directory') { setImageDirectory(selected); setImageZipPath(''); }
    } catch (reason) { setError(messageOf(reason)); }
  }
  async function importFile() {
    if (!preview || !writable || busy || inFlight.current || lingmoImportBlocked(preview.diagnostics) || (newProject && !title.trim()) || (preview.revisionComparison?.existingDocumentIds.length && !allowNewRevision)) return;
    inFlight.current = true; setPhase('import'); setError('');
    try {
      const workspace = await request<Workspace>({ action: projectId ? 'lingmo.import' : 'lingmo.create_project', ...(projectId ? { projectId } : { title: title.trim() }), path, ...(catalogPath ? { catalogPath } : {}), ...(imageZipPath ? { imageZipPath } : {}), ...(imageDirectory ? { imageDirectory } : {}), expectedSha256: preview.markdownSha256, previewToken: preview.previewToken, allowNewRevision });
      const report = (workspace as Workspace & { lingmoImportReport?: { media?: { missing?: number; failures?: unknown[] } } }).lingmoImportReport;
      if (report?.media?.missing || report?.media?.failures?.length) setCompleted(workspace);
      else await onImported(workspace);
    } catch (reason) { setError(messageOf(reason)); }
    finally { inFlight.current = false; setPhase('idle'); }
  }
  const revision = !!preview?.revisionComparison?.existingDocumentIds.length;
  const mediaReport = (completed as (Workspace & { lingmoImportReport?: { media?: { imported?: number; missing?: number; failures?: { filename?: string; message?: string }[] } } }) | null)?.lingmoImportReport?.media;
  return <Modal title={newProject ? '导入分镜，开始制作' : '导入分镜 MD'} onClose={close}>
    <div className="lingmo-import-dialog">
      <p className="lingmo-import-lead">{newProject ? '选择已有分镜文档，自动建立项目并打开分镜工作台。' : '将已有分镜加入当前项目，按集和场次继续制作。'}</p>
      <button type="button" className={`lingmo-file-picker ${path ? 'has-file' : ''}`} disabled={busy || !writable || !!completed} onClick={() => void pick('md')}>
        <span className="lingmo-file-icon"><Icon name={path ? 'source' : 'upload'} size={26}/></span>
        <span><strong>{path ? filename(path) : '选择分镜 MD 文件'}</strong><small>{path ? '点击更换文件' : '灵墨格式 · 自动识别剧集、场次、片段与资产标签'}</small></span>
        {path && <Icon name="check" size={19}/>}
      </button>
      {phase === 'preview' && <p className="lingmo-import-progress" role="status"><span className="spinner"/>正在读取文档结构…</p>}
      {preview && <>
        <div className="lingmo-import-counts" aria-label="文档结构预览">{[[preview.counts.episodes, '集'], [preview.counts.scenes, '场'], [preview.counts.fragments, '片段'], [preview.counts.shots, '镜']].map(([count, label]) => <div key={label}><strong>{count}</strong><span>{label}</span></div>)}</div>
        <p className="lingmo-import-match">{preview.counts.assets} 项资产 · {preview.catalogMatched ? '已匹配同版资产目录' : '无资产目录也可浏览，图片可稍后补充'}</p>
        {newProject && <label>项目名称<input aria-label="导入项目名称" autoComplete="off" maxLength={160} disabled={busy} value={title} onChange={event => setTitle(event.target.value)}/></label>}
        {!!preview.diagnostics.length && <details className="lingmo-import-diagnostics" open={lingmoImportBlocked(preview.diagnostics)}><summary>{lingmoImportBlocked(preview.diagnostics) ? '文档结构需要处理' : `${preview.diagnostics.length} 项待核对信息`}</summary>{preview.diagnostics.map((item, index) => <p key={index}>{item.line ? `第 ${item.line} 行：` : ''}{item.message}</p>)}</details>}
        {revision && <label className="check-label"><input type="checkbox" checked={allowNewRevision} disabled={busy} onChange={event => setAllowNewRevision(event.target.checked)}/>作为新修订导入，保留原版本和已有编辑</label>}
      </>}
      {!completed && <details className="lingmo-import-attachments"><summary>附带已有图片或资产目录（可选）</summary><p>同文件夹的同名资产目录会自动读取。没有图片也能先导入。</p>
        <div><Button disabled={busy} onClick={() => void pick('catalog')}>资产目录</Button><span title={catalogPath}>{catalogPath ? filename(catalogPath) : '自动读取或手动选择 .assets.json'}</span>{catalogPath && <Button variant="ghost" disabled={busy} onClick={() => { setCatalogPath(''); if (path) void readPreview(path, ''); }}>移除</Button>}</div>
        <div><Button disabled={busy} onClick={() => void pick('zip')}>图片 ZIP</Button><Button disabled={busy} onClick={() => void pick('directory')}>图片文件夹</Button><span title={imageZipPath || imageDirectory}>{filename(imageZipPath || imageDirectory) || '稍后也可补图'}</span>{(imageZipPath || imageDirectory) && <Button variant="ghost" disabled={busy} onClick={() => { setImageZipPath(''); setImageDirectory(''); }}>移除</Button>}</div>
      </details>}
      {completed && <div className="lingmo-import-partial" role="alert"><strong>分镜已保存，部分图片需要补充</strong><p>已有图片入库 {mediaReport?.imported || 0} 张{mediaReport?.missing ? `，缺少 ${mediaReport.missing} 张目录图片` : ''}。可打开工作台后继续补图。</p>{mediaReport?.failures?.map((failure, index) => <p key={index}>{failure.filename || ''} {failure.message || '素材导入失败'}</p>)}</div>}
      {error && <p className="warning" role="alert">{error}{path && !preview && <Button variant="ghost" disabled={busy} onClick={() => void readPreview(path)}>重新读取</Button>}</p>}
      <div className="lingmo-import-footer"><small>{phase === 'import' ? '正在本地保存，请稍候…' : '保留完整原文 · 不启动生成'}</small><div>{completed ? <Button variant="primary" onClick={() => void onImported(completed)}>打开工作台，稍后补图</Button> : <><Button disabled={phase === 'import'} onClick={close}>取消</Button><Button executionAction={newProject ? 'lingmo.create_project' : 'lingmo.import'} variant="primary" disabled={!writable || busy || !preview || lingmoImportBlocked(preview?.diagnostics || []) || (newProject && !title.trim()) || (revision && !allowNewRevision)} onClick={() => void importFile()}>{phase === 'import' ? '正在导入…' : newProject ? '导入并打开项目' : '确认导入'}</Button></>}</div></div>
    </div>
  </Modal>;
}
