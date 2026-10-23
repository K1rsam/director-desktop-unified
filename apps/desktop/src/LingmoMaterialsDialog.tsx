import { useEffect, useMemo, useRef, useState } from 'react';
import { choose, request } from './api';
import { adoptedImage, Button, Icon, MediaPreview, messageOf, Modal } from './components';
import { lingmoAssetType, lingmoTypeNames } from './lingmo';
import type { Asset, LingmoAssetType, Media, Mutation, Workspace } from './types';
import './lingmoMaterials.css';

export type MaterialRow = { id: string; path: string; relativePath: string; filename: string; kind: 'image' | 'audio'; status: 'matched' | 'unmatched' | 'ambiguous' | 'unsupported'; assetId?: string; variantId?: string; expectedAdoptedMediaId?: string | null; candidateAssetIds: string[]; size: number };
export type MaterialPreview = { previewToken: string; documentId: string; rows: MaterialRow[]; counts: { files: number; matched: number; unmatched: number; ambiguous: number; unsupported: number }; milliseconds: number };
type LastScan = { preview: MaterialPreview; fileCount: number; importedFiles: number; pendingFiles: number; pendingRows: MaterialRow[] };
type ImportResult = { imported: number; adopted: number; placeholders: number; skipped: number; failed: number; rows: { rowId: string; status: string; mediaId?: string; message?: string }[]; milliseconds: number };
type Selection = { rowId: string; assetId: string; variantId: string; adopt: boolean; expectedAdoptedMediaId: string | null };
type Props = { workspace: Workspace; documentId: string; writable: boolean; mutate: Mutation; inspect: (media: Media) => void; onClose: () => void };
const pageSize = 80;
const assetKey = (id: string, variant?: string) => `${id}\u0000${variant || ''}`;
const placeholder = (media?: Media) => !!media && (media as Media & { placeholder?: boolean }).placeholder === true;
export function materialCanBind(kind: string, asset: Asset): boolean {
  const type = lingmoAssetType(asset);
  return kind === 'audio' ? type === 'SE' || type === 'OM' : kind === 'image' && type !== 'SE';
}
export function materialCandidate(workspace: Workspace, asset: Asset, kind: 'audio' | 'image'): Media | undefined {
  const ids = new Set((asset.candidates || []).map(candidate => typeof candidate === 'string' ? candidate : candidate.id));
  const candidates = workspace.media.filter(media => media.kind === kind && (ids.has(media.id) || media.assetId === asset.id && (media.variantId || '') === (asset.variantId || '')));
  return candidates.find(media => !placeholder(media)) || candidates[0]
    || (asset.candidates || []).find((candidate): candidate is Media => typeof candidate !== 'string' && candidate.kind === kind);
}
export function materialAudio(workspace: Workspace, asset: Asset): Media | undefined {
  return materialCandidate(workspace, asset, 'audio');
}

export function documentMaterials(workspace: Workspace, documentId: string): { asset: Asset; scenes: string[] }[] {
  const doc = workspace.lingmoDocuments?.find(item => item.id === documentId);
  if (!doc) return [];
  const places = new Map<string, Set<string>>();
  const add = (id: string, scene: string) => { if (!places.has(id)) places.set(id, new Set()); if (scene) places.get(id)!.add(scene); };
  for (const scene of doc.scenes) {
    for (const id of scene.assetIds || []) add(id, scene.number);
    for (const unit of workspace.units.filter(unit => scene.unitIds.includes(unit.id))) for (const ref of unit.assetRefs || []) add(ref.assetId, scene.number);
  }
  return workspace.assets.filter(asset => places.has(asset.id) || asset.id.startsWith(`lingmo-${doc.markdownSha256}-asset-`)).map(asset => ({ asset, scenes: [...(places.get(asset.id) || [])] }));
}
export function materialSelections(rows: MaterialRow[], selected: Record<string, string>, assets: Asset[], workspace: Workspace, replace: boolean) {
  const selections: Selection[] = [], duplicates = new Set<string>(), targets = new Set<string>();
  for (const row of rows) {
    const asset = assets.find(asset => asset.id === selected[row.id]);
    if (!asset || row.status === 'unsupported' || !materialCanBind(row.kind, asset)) continue;
    const key = assetKey(asset.id, asset.variantId);
    if (targets.has(key)) duplicates.add(key); targets.add(key);
    const baseline = workspace.assets.find(item => item.id === asset.id && (item.variantId || '') === (asset.variantId || '')) || asset;
    const current = adoptedImage(workspace, baseline);
    const expected = row.assetId === asset.id && (row.variantId || '') === (asset.variantId || '') && row.expectedAdoptedMediaId !== undefined ? row.expectedAdoptedMediaId : typeof baseline.adoptedMediaId === 'string' ? baseline.adoptedMediaId : current?.id || null;
    selections.push({ rowId: row.id, assetId: asset.id, variantId: asset.variantId || '', adopt: row.kind === 'image' && (replace || !current || placeholder(current)), expectedAdoptedMediaId: expected });
  }
  return { selections, duplicates };
}

export function LingmoMaterialsDialog({ workspace, documentId, writable, mutate, inspect, onClose }: Props) {
  const materials = useMemo(() => documentMaterials(workspace, documentId), [workspace.assets, workspace.units, workspace.lingmoDocuments, documentId]);
  const assets = materials.map(item => item.asset);
  const [type, setType] = useState(''), [query, setQuery] = useState(''), [missingOnly, setMissingOnly] = useState(false), [page, setPage] = useState(0);
  const [preview, setPreview] = useState<MaterialPreview | null>(null), [selected, setSelected] = useState<Record<string, string>>({}), [replace, setReplace] = useState(false);
  const [previewWorkspace, setPreviewWorkspace] = useState<Workspace | null>(null);
  const [bindingRow, setBindingRow] = useState('');
  const [lastScan, setLastScan] = useState<LastScan | null>(null);
  const scanRevision = useRef(0);
  const [phase, setPhase] = useState(''), [error, setError] = useState(''), [result, setResult] = useState<ImportResult | null>(null), [rowPage, setRowPage] = useState(0);
  const busy = useRef(false);
  useEffect(() => {
    const revision = ++scanRevision.current;
    void request<LastScan | null>({ action: 'lingmo.media.latest', projectId: workspace.project.id, documentId }).then(scan => {
      if (revision === scanRevision.current) setLastScan(scan);
    }).catch(() => {});
    return () => { scanRevision.current++; };
  }, [workspace.project.id, documentId]);
  const needsImage = (asset: Asset) => lingmoAssetType(asset) !== 'SE' && !(lingmoAssetType(asset) === 'OM' && materialAudio(workspace, asset)) && (!adoptedImage(workspace, asset) || placeholder(adoptedImage(workspace, asset)));
  const missing = materials.filter(item => needsImage(item.asset)).length;
  const filtered = materials.filter(item => (!type || lingmoAssetType(item.asset) === type) && (!missingOnly || needsImage(item.asset)) && (!query.trim() || `${item.asset.name} ${item.scenes.join(' ')}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())));
  const visible = filtered.slice(page * pageSize, (page + 1) * pageSize);
  const mappingWorkspace = previewWorkspace || workspace;
  const mappingAssets = previewWorkspace ? documentMaterials(previewWorkspace, documentId).map(item => item.asset) : assets;
  const { selections, duplicates } = materialSelections(preview?.rows || [], selected, mappingAssets, mappingWorkspace, replace);
  const close = () => { if (!busy.current) onClose(); };
  async function prepare(kind: 'directory' | 'file') {
    if (busy.current) return;
    busy.current = true; setPhase('选择本地素材…'); setError('');
    try {
      const paths = await choose(kind, kind === 'file'); if (!paths.length) return;
      scanRevision.current++;
      setPhase('扫描文件并精确匹配资产名称…');
      const next = await request<MaterialPreview>({ action: 'lingmo.media.preview', projectId: workspace.project.id, documentId, ...(kind === 'directory' ? { directory: paths[0] } : { paths }) });
      setPreview(next); setPreviewWorkspace(workspace); setResult(null); setRowPage(0); setReplace(false); setBindingRow(''); setLastScan(null);
      setSelected(Object.fromEntries(next.rows.filter(row => row.status === 'matched' && assets.some(asset => asset.id === row.assetId)).map(row => [row.id, row.assetId!])));
    } catch (reason) { setError(messageOf(reason)); } finally { busy.current = false; setPhase(''); }
  }
  async function commit() {
    if (busy.current || !preview || !selections.length || duplicates.size) return;
    busy.current = true; setPhase('校验绑定并将素材写入本机项目…'); setError('');
    try {
      const next = await mutate('lingmo.media.commit', { documentId, previewToken: preview.previewToken, selections }, '本地素材已处理，分镜及资产身份保持不变。');
      const report = (next as (Workspace & { lingmoMediaImportReport?: ImportResult }) | null)?.lingmoMediaImportReport;
      if (report) setResult(report); else setError('未收到导入结果，请刷新项目核对；暂不重复提交。');
      if (report) {
        try { setLastScan(await request<LastScan | null>({ action: 'lingmo.media.latest', projectId: workspace.project.id, documentId })); } catch { /* The completed import receipt remains authoritative. */ }
      }
    } catch (reason) { setError(messageOf(reason)); } finally { busy.current = false; setPhase(''); }
  }
  async function restorePending() {
    if (busy.current) return;
    busy.current = true; setPhase('读取上次未入库文件…'); setError('');
    try {
      const scan = await request<LastScan | null>({ action: 'lingmo.media.latest', projectId: workspace.project.id, documentId });
      scanRevision.current++; setLastScan(scan);
      if (!scan?.pendingRows.length) return;
      const rows = scan.pendingRows;
      setPreview({ ...scan.preview, rows, counts: { files: rows.length, matched: rows.filter(row => row.status === 'matched').length, unmatched: rows.filter(row => row.status === 'unmatched').length, ambiguous: rows.filter(row => row.status === 'ambiguous').length, unsupported: rows.filter(row => row.status === 'unsupported').length } });
      setPreviewWorkspace(workspace); setResult(null); setRowPage(0); setReplace(false); setBindingRow('');
      setSelected(Object.fromEntries(rows.filter(row => row.status === 'matched').map(row => [row.id, row.assetId!])));
    } catch (reason) { setError(messageOf(reason)); } finally { busy.current = false; setPhase(''); }
  }
  function filterChange(update: () => void) { update(); setPage(0); }
  const done = new Set(result?.rows.filter(row => !['failed', 'error', 'adoption_conflict'].includes(row.status)).map(row => row.rowId));
  return <Modal title="本剧素材" onClose={close}><div className="lingmo-materials" aria-label="本剧素材管理">
    <header className="lingmo-materials-toolbar"><div><strong>{materials.length} 项剧本资产</strong><span>{missing ? `${missing} 项待补图` : '图片素材已齐备'}</span><small>本地匹配与入库，不触发生成</small></div><div className="actions"><Button icon="folder" disabled={!writable || !!phase} onClick={() => void prepare('directory')}>导入文件夹</Button><Button icon="upload" disabled={!writable || !!phase} onClick={() => void prepare('file')}>导入音频 / 文件</Button></div></header>
    {lastScan && <div className="lingmo-materials-scan" role="status"><div><strong>上次文件夹：{lastScan.fileCount} 个文件</strong><span>已入库 {lastScan.importedFiles} · 未入库 {lastScan.pendingFiles}</span><small>剧本资产数与文件数分别统计。未匹配的文件保留原件，不自动合并资产版本。</small></div>{lastScan.pendingFiles > 0 && <Button disabled={!!phase} onClick={() => void restorePending()}>查看 {lastScan.pendingFiles} 个未入库文件</Button>}</div>}
    {phase && <p className="lingmo-materials-progress" role="status"><Icon name="refresh"/>{phase} 请保持此窗口打开。</p>}{error && <p className="warning" role="alert">{error}</p>}
    <div className="lingmo-materials-filters"><select aria-label="素材类型" value={type} onChange={event => filterChange(() => setType(event.target.value))}><option value="">全部（{materials.length}）</option>{(['CH', 'SC', 'PR', 'SE', 'OM'] as LingmoAssetType[]).map(value => <option key={value} value={value}>{lingmoTypeNames[value]}（{materials.filter(item => lingmoAssetType(item.asset) === value).length}）</option>)}</select><input aria-label="搜索素材名称或场次" placeholder="搜索素材名称或场次" value={query} onChange={event => filterChange(() => setQuery(event.target.value))}/><label><input type="checkbox" checked={missingOnly} onChange={event => filterChange(() => setMissingOnly(event.target.checked))}/>仅待补图</label></div>
    <div className="lingmo-materials-grid" aria-label="全剧素材列表">{visible.map(({ asset, scenes }) => {
      const current = adoptedImage(workspace, asset), audio = materialAudio(workspace, asset), type = lingmoAssetType(asset);
      const image = current && !placeholder(current) ? current : materialCandidate(workspace, asset, 'image') || current;
      const candidate = type === 'SE' ? audio : type === 'OM' && audio && (!current || placeholder(current)) ? audio : image;
      const state = type === 'SE' || type === 'OM' && candidate?.kind === 'audio' ? audio ? '已入库音频，片段中选择' : '待补声音' : placeholder(candidate) ? '白图占位 · 待补图' : current && !placeholder(current) ? '已配图' : candidate ? '候选图，待采用' : '待补图';
      return <article className="lingmo-materials-card" key={assetKey(asset.id, asset.variantId)}><div className="lingmo-materials-thumb">{candidate?.kind === 'audio' ? <Button icon="play" aria-label={`播放音频 ${asset.name}`} onClick={() => inspect(candidate)}/> : candidate ? <MediaPreview media={candidate} compact onInspect={inspect}/> : <span><Icon name={type === 'SE' ? 'source' : 'image'}/></span>}</div><div className="lingmo-materials-copy"><strong title={asset.name}>{asset.name}</strong><small>{lingmoTypeNames[type || 'OM']} · {asset.externalId != null ? `资产版本 ${String(asset.assetRevision ?? '1')}` : '本地版本'}</small><p title={scenes.join('、')}>场次 {scenes.length ? scenes.join('、') : '按片段引用'}</p><span className={`lingmo-materials-state ${current && !placeholder(current) || candidate?.kind === 'audio' ? 'ready' : ''}`}>{state}</span></div></article>;
    })}{!filtered.length && <p className="muted">没有符合条件的素材。</p>}</div>
    <div className="lingmo-materials-pagination"><small>共 {filtered.length} 项 · 每页最多 {pageSize} 项</small><Button disabled={page <= 0} onClick={() => setPage(value => value - 1)}>上一页</Button><span>{Math.min(page + 1, Math.max(1, Math.ceil(filtered.length / pageSize)))} / {Math.max(1, Math.ceil(filtered.length / pageSize))}</span><Button disabled={(page + 1) * pageSize >= filtered.length} onClick={() => setPage(value => value + 1)}>下一页</Button></div>
    {preview && <section className="lingmo-materials-import" aria-label="待回填文件"><header><div><h3>待回填文件 · {preview.rows.length}</h3><p>精确匹配 {preview.counts.matched} · 未匹配 {preview.counts.unmatched} · 需区分 {preview.counts.ambiguous} · 不支持 {preview.counts.unsupported}</p></div><Button disabled={!!phase || !done.size} onClick={() => { setPreview({ ...preview, rows: preview.rows.filter(row => !done.has(row.id)) }); setResult(null); setRowPage(0); }}>清除已完成</Button></header>
      <p className="muted">仅自动绑定完整名称唯一匹配的资产。未匹配项可手动选择；音频只入库，需在片段中选择。清除列表不会删除项目素材。</p>
      <div className="lingmo-materials-file-list">{preview.rows.slice(rowPage * pageSize, (rowPage + 1) * pageSize).map(row => { const outcome = result?.rows.find(item => item.rowId === row.id); return <div className="lingmo-materials-file" key={row.id}><div><strong title={row.relativePath || row.path}>{row.filename}</strong><small>{row.kind === 'audio' ? '音频' : '图片'} · {Math.ceil(row.size / 1024)} KiB · {row.status === 'matched' ? '精确匹配' : row.status === 'ambiguous' ? '请选择同名版本' : row.status === 'unsupported' ? '不支持此文件' : '未匹配'}</small></div><div className="lingmo-materials-binding">{bindingRow === row.id ? <select autoFocus aria-label={`绑定资产 ${row.filename}`} disabled={!!phase || row.status === 'unsupported' || !!outcome && done.has(row.id)} value={selected[row.id] || ''} onChange={event => setSelected(current => ({ ...current, [row.id]: event.target.value }))}><option value="">不导入此文件</option>{mappingAssets.filter(asset => materialCanBind(row.kind, asset)).map(asset => <option key={asset.id} value={asset.id}>{asset.name} · {lingmoAssetType(asset)} · {asset.variantId ? asset.variantId.slice(-10) : '本地版本'}</option>)}</select> : <Button aria-label={`更改绑定 ${row.filename}`} disabled={!!phase || row.status === 'unsupported' || !!outcome && done.has(row.id)} onClick={() => setBindingRow(row.id)}>{mappingAssets.find(asset => asset.id === selected[row.id])?.name || '选择绑定资产'}<Icon name="chevron"/></Button>}</div><span className={outcome && ['failed', 'error', 'adoption_conflict'].includes(outcome.status) ? 'warning' : 'muted'}>{outcome ? outcome.message || (done.has(row.id) ? '已处理' : '处理失败') : selected[row.id] ? '待入库' : '未选择资产'}</span></div>; })}</div>
      {preview.rows.length > pageSize && <div className="lingmo-materials-pagination"><Button disabled={rowPage <= 0} onClick={() => setRowPage(value => value - 1)}>上一批文件</Button><span>{rowPage + 1} / {Math.ceil(preview.rows.length / pageSize)}</span><Button disabled={(rowPage + 1) * pageSize >= preview.rows.length} onClick={() => setRowPage(value => value + 1)}>下一批文件</Button></div>}
      <footer><label><input type="checkbox" checked={replace} disabled={!!phase} onChange={event => setReplace(event.target.checked)}/>替换已有配图</label><p>默认保留已有配图，新文件进入候选；缺图资产自动采用真图，白图只作占位。</p>{duplicates.size > 0 && <p className="warning" role="alert">多个文件选中了同一资产版本，请每个版本保留一个文件后再导入。</p>}<Button variant="primary" disabled={!writable || !!phase || !selections.length || duplicates.size > 0 || !!result} onClick={() => void commit()}>确认导入 {selections.length} 项</Button></footer>
      {result && <div className="lingmo-materials-result" role={result.failed ? 'alert' : 'status'}>已入库 {result.imported} · 已采用 {result.adopted} · 白图占位 {result.placeholders} · 跳过 {result.skipped} · 失败 {result.failed} · {(result.milliseconds / 1000).toFixed(2)} 秒{result.failed > 0 && <p>逐项原因见文件列表。已成功项保留，请重新扫描未完成文件后重试。</p>}</div>}
    </section>}
  </div></Modal>;
}
