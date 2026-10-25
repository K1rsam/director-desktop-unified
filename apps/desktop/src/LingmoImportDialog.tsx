import { useRef, useState } from 'react';
import { choose, request } from './api';
import { Button, Icon, messageOf, Modal } from './components';
import { lingmoImportBlocked } from './lingmo';
import type { LingmoPreview, Workspace } from './types';
import './lingmoImport.css';
import './assetWorkflow.css';

type Preview = LingmoPreview & { revisionComparison?: { existingDocumentIds: string[]; changes?: unknown }; handoff?: { sourceKind: string; completePackageVerified: boolean; level?: string; proof?: { explicitImageCount?: number; registryAssetCount?: number } } };
type Props = { projectId?: string; writable: boolean; onImported: (workspace: Workspace, target?: 'images' | 'videos') => void | Promise<void>; onClose: () => void };
const filename = (path: string) => path.split(/[\\/]/).at(-1) || path;

export function LingmoImportDialog({ projectId, writable, onImported, onClose }: Props) {
  const [path, setPath] = useState(''), [title, setTitle] = useState('');
  const [catalogPath, setCatalogPath] = useState(''), [imageZipPath, setImageZipPath] = useState(''), [imageDirectory, setImageDirectory] = useState('');
  const [module22Root, setModule22Root] = useState(''), [module22DirectionPath, setModule22DirectionPath] = useState('');
  const [preview, setPreview] = useState<Preview | null>(null), [phase, setPhase] = useState<'idle' | 'preview' | 'import'>('idle');
  const [error, setError] = useState(''), [allowNewRevision, setAllowNewRevision] = useState(false);
  const [completed, setCompleted] = useState<Workspace | null>(null);
  const [destination, setDestination] = useState<'images' | 'videos'>('videos');
  const [handoffMode, setHandoffMode] = useState<'auto' | 'catalog-only'>('auto');
  const inFlight = useRef(false), sequence = useRef(0);
  const busy = phase !== 'idle', newProject = !projectId;
  const close = () => { if (!inFlight.current) { sequence.current++; if (completed) void onImported(completed, destination); else onClose(); } };
  async function readPreview(nextPath: string, nextCatalog = catalogPath, nextMode = handoffMode, nextRoot = module22Root, nextDirection = module22DirectionPath) {
    const current = ++sequence.current;
    setPhase('preview'); setError(''); setPreview(null); setAllowNewRevision(false);
    try {
      const result = await request<Preview>({ action: projectId ? 'lingmo.preview' : 'lingmo.preview_file', ...(projectId ? { projectId } : {}), path: nextPath, handoffMode: nextMode, ...(nextCatalog ? { catalogPath: nextCatalog } : {}), ...(nextRoot ? { module22Root: nextRoot } : {}), ...(nextDirection ? { module22DirectionPath: nextDirection } : {}) });
      if (sequence.current === current) setPreview(result);
    } catch (reason) { if (sequence.current === current) setError(messageOf(reason)); }
    finally { if (sequence.current === current) setPhase('idle'); }
  }
  async function pick(kind: 'md' | 'catalog' | 'zip' | 'directory' | 'handoff' | 'direction') {
    if (inFlight.current || busy) return;
    try {
      const [selected] = await choose(kind === 'directory' || kind === 'handoff' ? 'directory' : 'file');
      if (!selected) return;
      if (kind === 'md') { setPath(selected); setHandoffMode('auto'); setTitle(filename(selected).replace(/\.md$/i, '').slice(0, 160)); await readPreview(selected, catalogPath, 'auto'); }
      if (kind === 'catalog') { setCatalogPath(selected); if (path) await readPreview(path, selected); }
      if (kind === 'zip') { setImageZipPath(selected); setImageDirectory(''); }
      if (kind === 'directory') { setImageDirectory(selected); setImageZipPath(''); }
      if (kind === 'handoff') { setModule22Root(selected); setModule22DirectionPath(''); setHandoffMode('auto'); if (path) await readPreview(path, catalogPath, 'auto', selected, ''); }
      if (kind === 'direction') { setModule22DirectionPath(selected); if (path) await readPreview(path, catalogPath, handoffMode, module22Root, selected); }
    } catch (reason) { setError(messageOf(reason)); }
  }
  async function importFile() {
    if (!preview || !writable || busy || inFlight.current || lingmoImportBlocked(preview.diagnostics) || (newProject && !title.trim()) || (preview.revisionComparison?.existingDocumentIds.length && !allowNewRevision)) return;
    inFlight.current = true; setPhase('import'); setError('');
    try {
      const workspace = await request<Workspace>({ action: projectId ? 'lingmo.import' : 'lingmo.create_project', ...(projectId ? { projectId } : { title: title.trim() }), path, handoffMode, ...(catalogPath ? { catalogPath } : {}), ...(module22Root ? { module22Root } : {}), ...(module22DirectionPath ? { module22DirectionPath } : {}), ...(imageZipPath ? { imageZipPath } : {}), ...(imageDirectory ? { imageDirectory } : {}), expectedSha256: preview.markdownSha256, previewToken: preview.previewToken, allowNewRevision });
      const report = (workspace as Workspace & { lingmoImportReport?: { media?: { missing?: number; failures?: unknown[] } } }).lingmoImportReport;
      if (report?.media?.missing || report?.media?.failures?.length) setCompleted(workspace);
      else await onImported(workspace, destination);
    } catch (reason) { setError(messageOf(reason)); }
    finally { inFlight.current = false; setPhase('idle'); }
  }
  const revision = !!preview?.revisionComparison?.existingDocumentIds.length;
  const mediaReport = (completed as (Workspace & { lingmoImportReport?: { media?: { imported?: number; missing?: number; failures?: { filename?: string; message?: string }[] } } }) | null)?.lingmoImportReport?.media;
  return <Modal title={newProject ? '导入分镜，开始制作' : '导入分镜 MD'} onClose={close}>
    <div className="lingmo-import-dialog">
      <p className="lingmo-import-lead">{newProject ? '选择已有分镜文档，自动建立项目，再选择补齐图片或打开工作台。' : '将已有分镜加入当前项目，按集和场次继续制作。'}</p>
      <button type="button" className={`lingmo-file-picker ${path ? 'has-file' : ''}`} disabled={busy || !writable || !!completed} onClick={() => void pick('md')}>
        <span className="lingmo-file-icon"><Icon name={path ? 'source' : 'upload'} size={26}/></span>
        <span><strong>{path ? filename(path) : '选择分镜 MD 文件'}</strong><small>{path ? '点击更换文件' : '灵墨格式 · 自动识别剧集、场次、片段与资产标签'}</small></span>
        {path && <Icon name="check" size={19}/>}
      </button>
      {phase === 'preview' && <p className="lingmo-import-progress" role="status"><span className="spinner"/>正在读取文档结构…</p>}
      {preview && <>
        <div className="lingmo-import-counts" aria-label="文档结构预览">{[[preview.counts.episodes, '集'], [preview.counts.scenes, '场'], [preview.counts.fragments, '片段'], [preview.counts.shots, '镜']].map(([count, label]) => <div key={label}><strong>{count}</strong><span>{label}</span></div>)}</div>
        <p className="lingmo-import-match">{preview.counts.assets} 项资产 · {preview.catalogMatched ? '已匹配同版资产目录' : '无资产目录也可浏览，图片可稍后补充'}</p>
        {preview.handoff?.completePackageVerified && <p className="lingmo-import-match">完整接续包已核验 · 本次图片 {preview.handoff.proof?.explicitImageCount ?? preview.counts.assets} 项{preview.handoff.proof?.registryAssetCount != null ? ` · 保留总库 ${preview.handoff.proof.registryAssetCount} 项供设计查阅` : ''}。不会自动生成或采用占位图。</p>}
        {handoffMode === 'catalog-only' && <p className="lingmo-import-match" role="status">仅按 MD 与同版资产目录导入；不继承未验证包中的源锁、方向、总库和独审证明。</p>}
        <fieldset className="lingmo-import-destination"><legend>导入后前往</legend><label><input type="radio" name="lingmo-destination" checked={destination === 'images'} disabled={busy} onChange={() => setDestination('images')}/><span><strong>补齐资产提示词</strong><small>进入第三阶段，先写提示词，再独立出图</small></span></label><label><input type="radio" name="lingmo-destination" checked={destination === 'videos'} disabled={busy} onChange={() => setDestination('videos')}/><span><strong>打开配图工作台</strong><small>进入第四阶段，没有图片也能浏览分镜</small></span></label></fieldset>
        {newProject && <label>项目名称<input aria-label="导入项目名称" autoComplete="off" maxLength={160} disabled={busy} value={title} onChange={event => setTitle(event.target.value)}/></label>}
        {!!preview.diagnostics.length && <details className="lingmo-import-diagnostics" open={lingmoImportBlocked(preview.diagnostics)}><summary>{lingmoImportBlocked(preview.diagnostics) ? '文档结构需要处理' : `${preview.diagnostics.length} 项待核对信息`}</summary>{preview.diagnostics.map((item, index) => <p key={index}>{item.line ? `第 ${item.line} 行：` : ''}{item.message}</p>)}</details>}
        {revision && <label className="check-label"><input type="checkbox" checked={allowNewRevision} disabled={busy} onChange={event => setAllowNewRevision(event.target.checked)}/>作为新修订导入，保留原版本和已有编辑</label>}
      </>}
      {!completed && <details className="lingmo-import-attachments"><summary>附带接续资料、图片或资产目录（可选）</summary><p>同文件夹的同名资产目录会自动读取。已有完整接续资料时，可选择资料文件夹，保留原设计、方向和图片范围。只有 MD 也能先导入。</p>
        <div><Button disabled={busy} onClick={() => void pick('handoff')}>完整接续资料文件夹</Button><span title={module22Root}>{module22Root ? filename(module22Root) : '已有分镜、设计总库与图片清单'}</span>{module22Root && <Button variant="ghost" disabled={busy} onClick={() => { setModule22Root(''); setModule22DirectionPath(''); if (path) void readPreview(path, catalogPath, handoffMode, '', ''); }}>移除接续资料</Button>}</div>
        {module22Root && <div><Button disabled={busy} onClick={() => void pick('direction')}>制作方向文件（可选）</Button><span title={module22DirectionPath}>{module22DirectionPath ? filename(module22DirectionPath) : '自动读取；缺少时再补选原方向文件'}</span>{module22DirectionPath && <Button variant="ghost" disabled={busy} onClick={() => { setModule22DirectionPath(''); if (path) void readPreview(path, catalogPath, handoffMode, module22Root, ''); }}>移除方向文件</Button>}</div>}
        {path && <label className="check-label"><input aria-label="仅按 MD 导入，不验证完整接续包" type="checkbox" checked={handoffMode === 'catalog-only'} disabled={busy} onChange={event => { const mode = event.target.checked ? 'catalog-only' : 'auto'; setHandoffMode(mode); void readPreview(path, catalogPath, mode); }}/>仅按 MD 导入（不验证完整接续包）</label>}
        <div><Button disabled={busy} onClick={() => void pick('catalog')}>资产目录</Button><span title={catalogPath}>{catalogPath ? filename(catalogPath) : '自动读取或手动选择 .assets.json'}</span>{catalogPath && <Button variant="ghost" disabled={busy} onClick={() => { setCatalogPath(''); if (path) void readPreview(path, ''); }}>移除</Button>}</div>
        <div><Button disabled={busy} onClick={() => void pick('zip')}>图片 ZIP</Button><Button disabled={busy} onClick={() => void pick('directory')}>图片文件夹</Button><span title={imageZipPath || imageDirectory}>{filename(imageZipPath || imageDirectory) || '稍后也可补图'}</span>{(imageZipPath || imageDirectory) && <Button variant="ghost" disabled={busy} onClick={() => { setImageZipPath(''); setImageDirectory(''); }}>移除</Button>}</div>
      </details>}
      {completed && <div className="lingmo-import-partial" role="alert"><strong>分镜已保存，部分图片需要补充</strong><p>已有图片入库 {mediaReport?.imported || 0} 张{mediaReport?.missing ? `，缺少 ${mediaReport.missing} 张目录图片` : ''}。可打开工作台后继续补图。</p>{mediaReport?.failures?.map((failure, index) => <p key={index}>{failure.filename || ''} {failure.message || '素材导入失败'}</p>)}</div>}
      {error && <div className="warning" role="alert">{error}{path && !preview && <><Button variant="ghost" disabled={busy} onClick={() => void readPreview(path)}>重新读取</Button>{handoffMode === 'auto' && /接续包|岗位包|SERIES_/.test(error) && <Button variant="ghost" disabled={busy} onClick={() => { setHandoffMode('catalog-only'); void readPreview(path, catalogPath, 'catalog-only'); }}>仅按 MD 导入，不验证完整接续包</Button>}</>}</div>}
      <div className="lingmo-import-footer"><small>{phase === 'import' ? '正在本地保存，请稍候…' : '保留完整原文 · 不启动生成'}</small><div>{completed ? <><Button onClick={() => void onImported(completed, 'images')}>补齐资产提示词</Button><Button variant="primary" onClick={() => void onImported(completed, 'videos')}>打开工作台，稍后补图</Button></> : <><Button disabled={phase === 'import'} onClick={close}>取消</Button><Button executionAction={newProject ? 'lingmo.create_project' : 'lingmo.import'} variant="primary" disabled={!writable || busy || !preview || lingmoImportBlocked(preview?.diagnostics || []) || (newProject && !title.trim()) || (revision && !allowNewRevision)} onClick={() => void importFile()}>{phase === 'import' ? '正在导入…' : newProject ? '导入并打开项目' : '确认导入'}</Button></>}</div></div>
    </div>
  </Modal>;
}
