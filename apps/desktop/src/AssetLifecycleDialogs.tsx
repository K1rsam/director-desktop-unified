import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { request } from './api';
import { assetImages } from './contracts';
import { assetDeletionReady, assetEditPayload, assetScopeProblems, initialAssetEdit, toggleAssetScope } from './assetManagement';
import type { AssetEditValues, AssetUsage } from './assetManagement';
import { Button, Empty, formatDate, kindName, MediaPreview, messageOf, Modal } from './components';
import type { Asset, Mutation, Workspace } from './types';
import './assetManagement.css';

const kinds = [['character', '人物'], ['beast', '非人角色'], ['scene', '场景'], ['prop', '道具'], ['GROUP', '群像']];

type EditProps = { asset: Asset; workspace: Workspace; writable: boolean; mutate: Mutation; onClose: () => void };
export function AssetEditDialog({ asset, workspace, writable, mutate, onClose }: EditProps) {
  const [values, setValues] = useState<AssetEditValues>(() => initialAssetEdit(asset, workspace.assets));
  const [sourceSearch, setSourceSearch] = useState('');
  const [sourceOpen, setSourceOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const rowIds = new Set((workspace.source?.rows || []).map(row => row.id));
  const invalidRefs = values.sourceRefs.filter(ref => typeof ref === 'string' && !rowIds.has(ref));
  const sourceOptions = workspace.assets.filter(item => item.id !== asset.id);
  const hasSource = sourceOptions.some(item => item.id === values.sourceAssetId);
  const sourceProblem = values.sourceError && values.sourceError.value === values.sourceAssetId ? values.sourceError.message : '';
  const scenes = (workspace.source?.scenes || []).filter(scene => !values.episodeIds.length || values.episodeIds.includes(scene.episodeId));
  const scopeProblems = assetScopeProblems(values, workspace.source?.scenes);
  const sourceRows = (workspace.source?.rows || []).filter(row => row.text.trim() && ['body', 'context'].includes(row.kind)
    && (sourceSearch.trim() ? row.text.includes(sourceSearch.trim()) : values.sourceRefs.includes(row.id)));
  const change = <K extends keyof AssetEditValues>(key: K, value: AssetEditValues[K]) => setValues(current => ({ ...current, [key]: value }));
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!writable || saving || !values.name.trim() || invalidRefs.length || scopeProblems.length || sourceProblem) return;
    setSaving(true);
    try {
      const next = await mutate('assets.save', { asset: assetEditPayload(asset, values, workspace.assets, workspace.source?.scenes) }, '资产已更新，受影响的未提交素材需要重新准备。');
      if (next) onClose();
    } finally { setSaving(false); }
  }
  return <Modal title={`编辑资产 · ${asset.name}`} onClose={onClose}><form className="asset-management-form" onSubmit={event => void save(event)}>
    {(asset.scope as { kind?: string } | undefined)?.kind === 'UNRESOLVED' && <p className="warning">此形象的适用范围尚未确认。未限定集场时，保存后按全剧通用；服装、伤态等局部形象请先选择适用集场。</p>}
    <div className="form-grid"><label>资产名称<input autoFocus required value={values.name} onChange={event => change('name', event.target.value)}/></label><label>资产类型<select value={values.kind} onChange={event => change('kind', event.target.value)}>{!kinds.some(([id]) => id === values.kind) && <option value={values.kind}>{kindName(values.kind)}</option>}{kinds.map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label></div>
    <label>直接父形象<select value={values.sourceAssetId} onChange={event => setValues(current => ({ ...current, sourceAssetId: event.target.value, sourceError: undefined }))}><option value="">独立形象，不指定父形象</option>{values.sourceAssetId && (!hasSource || sourceProblem) && <option value={values.sourceAssetId} disabled>原直接父形象待修正，原来源保留</option>}{sourceOptions.filter(item => !sourceProblem || item.id !== values.sourceAssetId).map(item => <option key={item.id} value={item.id}>{item.name} · {kindName(item.kind || item.type)}</option>)}</select><small>可选基础形象或另一衍生形象，例如换装后的伤态；留空表示独立形象。</small>{sourceProblem && <span className="warning">{sourceProblem}</span>}</label>
    <section className="asset-edit-scope"><div className="section-heading compact"><h4>适用集数</h4><small>{values.episodeIds.length ? `已选 ${values.episodeIds.length} 集` : values.sceneIds.length ? '按所选场景' : '全剧通用'}</small></div>{workspace.source?.episodes?.length ? <div className="scope-editor">{workspace.source.episodes.map((episode, index) => episode.id && <label className="check-label" key={episode.id}><input type="checkbox" checked={values.episodeIds.includes(episode.id)} onChange={event => setValues(current => ({ ...current, episodeIds: toggleAssetScope(current.episodeIds, episode.id!, event.target.checked), sceneIds: [] }))}/>{episode.title || `第${episode.number || index + 1}集`}</label>)}</div> : <p className="muted">导入剧本后可以指定适用集数。</p>}{scopeProblems.length > 0 && <p className="warning">{scopeProblems.join(' ')}<Button variant="ghost" onClick={() => setValues(current => ({ ...current, episodeIds: [], sceneIds: [] }))}>重新设置范围</Button></p>}</section>
    <details className="details asset-edit-scenes"><summary>适用场景 · {values.sceneIds.length ? `已选 ${values.sceneIds.length} 场` : values.episodeIds.length ? '所选集数全部场景' : '未限定'}</summary><p className="muted">改变集数后，使用这些集数的全部场景；可在此继续细分。</p><div className="scope-editor">{scenes.map(scene => <label className="check-label" key={scene.id}><input type="checkbox" checked={values.sceneIds.includes(scene.id)} onChange={event => change('sceneIds', toggleAssetScope(values.sceneIds, scene.id, event.target.checked))}/>{workspace.source?.episodes?.find(episode => episode.id === scene.episodeId)?.title} · {scene.title}</label>)}</div>{!scenes.length && <p className="muted">当前没有可选场景。</p>}</details>
    <details className="details asset-source-edit" open={sourceOpen || invalidRefs.length > 0} onToggle={event => setSourceOpen(event.currentTarget.open)}><summary>原文依据 · {values.sourceRefs.length} 处</summary>{(sourceOpen || invalidRefs.length > 0) && <>{invalidRefs.length > 0 && <p className="warning">{invalidRefs.length} 处原文行引用已失效。<Button variant="ghost" onClick={() => change('sourceRefs', values.sourceRefs.filter(ref => typeof ref !== 'string' || rowIds.has(ref)))}>清除失效引用</Button></p>}<label>查找原文<input value={sourceSearch} placeholder="输入角色、地点或原文片段" onChange={event => setSourceSearch(event.target.value)}/></label><div className="source-row-choices">{sourceRows.slice(0, 80).map(row => <label key={row.id}><input type="checkbox" checked={values.sourceRefs.includes(row.id)} onChange={event => change('sourceRefs', event.target.checked ? [...values.sourceRefs, row.id] : values.sourceRefs.filter(ref => ref !== row.id))}/><span><small>第{row.number}行</small>{row.text}</span></label>)}</div>{sourceRows.length > 80 && <p className="muted">当前显示前 80 行，请增加搜索词缩小范围。</p>}<p className="muted">输入名称或片段查找原文；已有选择会保留。手动测试资产可以不选。</p></>}</details>
    <details className="details asset-identity-details"><summary>内部身份信息</summary><dl><dt>资产身份</dt><dd>{asset.id}</dd><dt>基础主体</dt><dd>{asset.baseAssetId || asset.id}</dd><dt>形象身份</dt><dd>{asset.variantId || 'base'}</dd></dl><p className="muted">关联使用固定身份；修改名称和类型会保留已有图片与记录。</p></details>
    <div className="form-actions"><Button onClick={onClose} disabled={saving}>取消</Button><Button type="submit" variant="primary" disabled={!writable || saving || !values.name.trim() || invalidRefs.length > 0 || scopeProblems.length > 0 || !!sourceProblem}>{saving ? '正在保存' : '保存资产'}</Button></div>
  </form></Modal>;
}

type DeleteProps = { asset: Asset; workspace: Workspace; writable: boolean; mutate: Mutation; dirty: boolean; onClose: () => void; onDeleted: () => void };
export function AssetDeleteDialog({ asset, workspace, writable, mutate, dirty, onClose, onDeleted }: DeleteProps) {
  const [usage, setUsage] = useState<AssetUsage | null>(null);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [deleting, setDeleting] = useState(false);
  useEffect(() => {
    let active = true;
    setUsage(null); setError('');
    void request<AssetUsage>({ action: 'assets.usage', projectId: workspace.project.id, assetId: asset.id })
      .then(value => { if (active) setUsage(value); })
      .catch(reason => { if (active) setError(messageOf(reason)); });
    return () => { active = false; };
  }, [workspace.project.id, asset.id, asset.version, retry]);
  const ready = assetDeletionReady(asset, usage);
  async function remove() {
    if (!writable || deleting || !ready || !usage) return;
    setDeleting(true);
    try {
      const next = await mutate('assets.delete', { assetId: asset.id, expectedVersion: usage.version }, '资产已移出制作目录，历史图片与已提交任务记录保留。');
      if (next) { onDeleted(); onClose(); }
    } finally { setDeleting(false); }
  }
  return <AssetDeleteConfirmation asset={asset} workspace={workspace} writable={writable} usage={usage} error={error} dirty={dirty} deleting={deleting} onClose={onClose} onRetry={() => setRetry(value => value + 1)} onDelete={() => void remove()}/>;
}

type ConfirmationProps = { asset: Asset; workspace: Workspace; writable: boolean; usage: AssetUsage | null; error?: string; dirty?: boolean; deleting?: boolean; onClose: () => void; onRetry: () => void; onDelete: () => void };
export function AssetDeleteConfirmation({ asset, workspace, writable, usage, error = '', dirty = false, deleting = false, onClose, onRetry, onDelete }: ConfirmationProps) {
  const ready = assetDeletionReady(asset, usage);
  return <Modal title={`删除资产 · ${asset.name}`} onClose={onClose}><div className="asset-delete-content">
    <p>从当前制作目录移除这个资产。图片和历史记录会保留，可在“已删除资产”中恢复。</p>
    {dirty && <p className="warning">当前未保存的图片提示词将舍弃。</p>}
    {error ? <p className="warning">无法读取关联情况：{error}<Button variant="ghost" onClick={onRetry}>重试</Button></p> : !usage ? <p className="muted" role="status">正在读取关联情况…</p> : <>
      {!ready && <p className="warning">资产版本已变化，请关闭此窗口并刷新后再删除。</p>}
      {usage.units.length || usage.derivedAssets.length ? <div className="asset-delete-impact"><strong>以下关联需要重新指定资产</strong>{usage.units.length > 0 && <ul>{usage.units.map(unit => <li key={unit.id}>{workspace.source?.episodes?.find(episode => episode.id === unit.episodeId)?.title || '分镜单元'} · {unit.title || unit.id}<small>可在第四阶段修改配图。</small></li>)}</ul>}{usage.derivedAssets.length > 0 && <ul>{usage.derivedAssets.map(item => <li key={item.id}>派生形象 · {item.name}<small>需要重新指定来源形象。</small></li>)}</ul>}</div> : <p className="asset-delete-clear">没有分镜单元或派生形象关联，可以直接删除。</p>}
      {usage.activeJobs.length > 0 && <p className="muted">关联本地任务 {usage.activeJobs.length} 项将停止使用这个资产；已提交云端的任务继续查询与下载。</p>}
      {usage.submittedJobs.length > 0 && <p className="muted">保留 {usage.submittedJobs.length} 项已提交任务当时的素材与回执。</p>}
    </>}
    <div className="form-actions"><Button onClick={onClose} disabled={deleting}>取消</Button><Button variant="caution" disabled={!writable || deleting || !ready} onClick={onDelete}>{deleting ? '正在删除' : '删除资产'}</Button></div>
  </div></Modal>;
}

type DeletedProps = { assets: Asset[]; workspace: Workspace; writable: boolean; mutate: Mutation; onClose: () => void; onRestored: (id: string) => void };
export function DeletedAssetsDialog({ assets, workspace, writable, mutate, onClose, onRestored }: DeletedProps) {
  const [restoring, setRestoring] = useState('');
  async function restore(asset: Asset) {
    if (!writable || restoring) return;
    setRestoring(asset.id);
    try {
      const next = await mutate('assets.restore', { assetId: asset.id, expectedVersion: asset.version }, '资产已恢复，当前图片与历史记录保留。');
      if (next) onRestored(asset.id);
    } finally { setRestoring(''); }
  }
  return <Modal title="已删除资产" onClose={onClose}>{assets.length ? <div className="deleted-asset-list">{assets.map(asset => <article key={asset.id}><div className="deleted-asset-thumbnail"><MediaPreview compact media={assetImages(workspace, asset).current}/></div><div><strong>{asset.name}</strong><span>{kindName(asset.kind || asset.type)} · V{asset.version || 1}</span><small>删除于 {formatDate(String(asset.deletedAt || ''))}</small></div><Button disabled={!writable || !!restoring} onClick={() => void restore(asset)}>{restoring === asset.id ? '正在恢复' : '恢复资产'}</Button></article>)}</div> : <Empty icon="image" title="没有已删除的资产"/>}</Modal>;
}
