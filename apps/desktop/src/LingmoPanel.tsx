import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { choose, request } from './api';
import { moveItem } from './contracts';
import { adoptedImage, Button, Icon, JsonDetails, MediaPreview, messageOf, Modal } from './components';
import { LingmoImportDialog } from './LingmoImportDialog';
import { boundLingmoMentions, replaceLingmoReference, importedUnit, insertLingmoToken, lingmoAssetToken, lingmoAssetType, lingmoMentions, lingmoPromptParts, lingmoTypeNames, lingmoTypes, lingmoUnitDocument } from './lingmo';
import type { Asset, AssetRef, LingmoMention, LingmoScene, LingmoScope, Media, Mutation, Unit, Workspace } from './types';
import './lingmo.css';

type Props = { workspace: Workspace; writable: boolean; mutate: Mutation; inspect: (media: Media) => void; onError: (message: string) => void; onScope: (scope: LingmoScope | null) => void; children?: ReactNode };
function assetVersionLabel(asset: Asset): string {
  const revision = asset.assetRevision;
  return asset.externalId != null && (typeof revision === 'number' || typeof revision === 'string') ? `资产版本 ${revision}` : '本地版本';
}

export function LingmoPanel({ workspace, writable, mutate, inspect, onError, onScope, children }: Props) {
  const documents = workspace.lingmoDocuments || [];
  const [documentId, setDocumentId] = useState(documents[0]?.id || ''), [episodeId, setEpisodeId] = useState(''), [sceneId, setSceneId] = useState('');
  const [showImport, setShowImport] = useState(false), [selectedAsset, setSelectedAsset] = useState<string | null>(null);
  const [sceneData, setSceneData] = useState<{ units: Unit[]; scene: LingmoScene; assets: Asset[] } | null>(null);
  const [loadError, setLoadError] = useState(''), [reload, setReload] = useState(0);
  const doc = documents.find(item => item.id === documentId);
  const episode = doc?.episodes.find(item => item.id === episodeId) || doc?.episodes[0];
  const scenes = doc?.scenes.filter(item => item.episodeId === episode?.id) || [];
  const scene = scenes.find(item => item.id === sceneId) || scenes[0];
  const onScopeRef = useRef(onScope); onScopeRef.current = onScope;
  const requestedScene = useRef('');
  // Polls update media in place; only changed unit versions reload scene text.
  // Preserve the mounted content while a same-scene version refresh is pending.
  const sceneVersions = scene?.unitIds.map(id => { const unit = workspace.units.find(item => item.id === id); return `${id}:${unit?.version}`; }).join('|') || '';
  useEffect(() => {
    if (!doc || !scene) { requestedScene.current = ''; setSceneData(null); onScopeRef.current(null); return; }
    let alive = true;
    const scope = { documentId: doc.id, sceneId: scene.id, label: `第 ${episode?.number} 集 · ${scene.number} 场`, units: [], loading: true };
    const key = `${workspace.project.id}:${doc.id}:${scene.id}`;
    const changedScene = requestedScene.current !== key;
    requestedScene.current = key;
    if (changedScene) { setSceneData(null); setSelectedAsset(null); onScopeRef.current(scope); }
    setLoadError('');
    void request<{ units: Unit[]; scene: LingmoScene; assets: Asset[] }>({ action: 'lingmo.scene', projectId: workspace.project.id, documentId: doc.id, sceneId: scene.id }).then(data => {
      if (!alive) return; setSceneData(data); onScopeRef.current({ ...scope, units: data.units, loading: false });
    }).catch(error => { if (alive) { setLoadError(messageOf(error)); if (changedScene) onScopeRef.current({ ...scope, loading: false }); } });
    return () => { alive = false; };
  }, [workspace.project.id, doc?.id, scene?.id, sceneVersions, reload]);
  async function importImage(asset: Asset, kind: 'image' | 'audio' | 'video' = lingmoAssetType(asset) === 'SE' ? 'audio' : 'image') {
    try { const paths = await choose('file', true); if (!paths.length) return; await mutate('media.import', { paths, kind, assetId: asset.id, variantId: asset.variantId || '' }, '素材已入库；请核对并采用正确版本。'); }
    catch (reason) { onError(messageOf(reason)); }
  }
  function changeDocument(id: string) { setDocumentId(id); setEpisodeId(''); setSceneId(''); }
  function changeEpisode(id: string) { setEpisodeId(id); setSceneId(''); }
  const asset = selectedAsset ? workspace.assets.find(item => item.id === selectedAsset) || sceneData?.assets.find(item => item.id === selectedAsset) : undefined;
  const currentMedia = asset ? adoptedImage(workspace, asset) : undefined;
  const assetCandidates = asset ? workspace.media.filter(item => item.assetId === asset.id && (item.variantId || '') === (asset.variantId || '') || (asset.candidates || []).some(candidate => (typeof candidate === 'string' ? candidate : candidate.id) === item.id)) : [];
  const missingImages = sceneData?.assets.filter(item => lingmoAssetType(item) !== 'SE' && !adoptedImage(workspace, workspace.assets.find(current => current.id === item.id) || item)).length || 0;
  const scenePosition = scenes.findIndex(item => item.id === scene?.id);
  return <section className={`lingmo-panel ${doc ? 'lingmo-workbench' : 'lingmo-no-document'}`} aria-label="灵墨分镜导入与集场浏览">
    <aside className="lingmo-sidebar">
      <div className="lingmo-sidebar-heading"><span><Icon name="story"/>分镜目录</span><Button variant="ghost" icon="plus" aria-label="导入分镜 MD" title="导入现成分镜" disabled={!writable} onClick={() => setShowImport(true)}/></div>
      {!!documents.length ? <>
        <div className="lingmo-navigation">
          <label>分镜文档<select aria-label="分镜文档" value={documentId} onChange={event => changeDocument(event.target.value)}><option value="">原项目单元</option>{documents.map(item => <option value={item.id} key={item.id}>{item.filename}</option>)}</select></label>
          {doc && <><label>剧集<select aria-label="剧集" value={episode?.id || ''} onChange={event => changeEpisode(event.target.value)}>{doc.episodes.map(item => <option key={item.id} value={item.id}>{item.title || `第 ${item.number} 集`}</option>)}</select></label><label className="lingmo-scene-select">场次<select aria-label="场次" value={scene?.id || ''} onChange={event => setSceneId(event.target.value)}>{scenes.map(item => <option key={item.id} value={item.id}>{item.number} · {item.title || '场次'} · {item.unitIds.length} 片段</option>)}</select></label></>}
        </div>
        {doc && <>
          <div className="lingmo-scene-list" aria-label="本集场次"><div className="lingmo-list-caption">本集场次 <span>{scenes.length}</span></div>{scenes.map(item => <button type="button" key={item.id} className={`lingmo-scene-item ${item.id === scene?.id ? 'selected' : ''}`} aria-current={item.id === scene?.id ? 'true' : undefined} onClick={() => setSceneId(item.id)}>{item.title && item.title !== item.number && <span className="lingmo-scene-number">{item.number}</span>}<strong>{item.title && item.title !== item.number ? item.title : `场次 ${item.number}`}</strong><small>{item.unitIds.length} 个片段</small><Icon name="chevron" size={14}/></button>)}</div>
          <div className="lingmo-document-summary"><span>{doc.counts.episodes} 集</span><span>{doc.counts.scenes} 场</span><span>{doc.counts.fragments} 片段</span></div>
          <details className="lingmo-source-details"><summary>文档信息{doc.diagnostics.length ? ` · ${doc.diagnostics.length} 条提示` : ''}</summary><p>{doc.filename}</p><p>{doc.counts.shots} 个镜头 · 解析版本 {doc.parserVersion}</p><p className="lingmo-source-hash">SHA-256：{doc.markdownSha256}</p>{doc.diagnostics.filter(item => !item.line || !scene || item.line >= (scene.lineStart || 0) && item.line <= (scene.lineEnd || Infinity)).map((item, index) => <p className={item.blocking ? 'warning' : 'muted'} key={index}>{item.line ? `第 ${item.line} 行 · ` : ''}{item.message}</p>)}</details>
        </>}
      </> : <div className="lingmo-empty-import"><Icon name="source" size={28}/><h3>已有分镜，直接开始制作</h3><p>导入灵墨 MD，按集、场次浏览分镜并补充素材。</p><Button variant="primary" icon="upload" disabled={!writable} onClick={() => setShowImport(true)}>导入分镜 MD</Button></div>}
    </aside>
    <div className="lingmo-main">
      {doc && scene && <header className="lingmo-scene-heading"><div><span className="lingmo-breadcrumb">第 {episode?.number} 集 <span>/</span> 分镜工作台</span><h2>{scene.title && scene.title !== scene.number ? scene.title : `场次 ${scene.number}`}</h2><p>{scene.unitIds.length} 个片段 · 点击资产查看素材，点击正文标签定位绑定</p></div><div className="lingmo-scene-stepper"><Button aria-label="上一场" disabled={scenePosition <= 0} onClick={() => setSceneId(scenes[scenePosition - 1].id)}>←</Button><span>{scenePosition + 1} / {scenes.length}</span><Button aria-label="下一场" disabled={scenePosition < 0 || scenePosition >= scenes.length - 1} onClick={() => setSceneId(scenes[scenePosition + 1].id)}>→</Button></div></header>}
      {loadError && <p className="warning">本场读取失败：{loadError} <Button onClick={() => setReload(value => value + 1)}>重试读取</Button></p>}
      {doc && scene && !sceneData && !loadError && <p className="lingmo-loading" role="status">正在读取当前场次…</p>}
      {sceneData && <div className="lingmo-scene-assets"><div className="lingmo-assets-heading"><strong>本场素材 <span>{sceneData.assets.length}</span></strong><small>{missingImages ? `${missingImages} 项待补图` : '配图已齐备'} · 点击管理</small></div><div className="lingmo-assets-strip">{lingmoTypes.map(type => { const items = sceneData.assets.filter(item => lingmoAssetType(item) === type); return items.length ? <section className="lingmo-asset-group" key={type}><h5>{lingmoTypeNames[type]} <span>{items.length}</span></h5><div className="lingmo-asset-grid">{items.map(item => { const current = workspace.assets.find(current => current.id === item.id) || item, media = adoptedImage(workspace, current); return <button type="button" id={`lingmo-asset-${item.id}`} key={item.id} className={`lingmo-asset-card ${media ? 'has-media' : 'needs-media'}`} onClick={() => setSelectedAsset(item.id)} title={`${item.name} · ${assetVersionLabel(item)} · 点击管理素材`}><span className="lingmo-asset-thumb">{media ? <MediaPreview media={media} compact/> : <Icon name={type === 'SE' ? 'source' : 'image'} size={20}/>}</span><span className="lingmo-asset-copy"><strong>{item.name}</strong><small>{media ? '已配图' : type === 'SE' ? '待配音' : '待补图'}</small></span></button>; })}</div></section> : null; })}</div></div>}
      <div className="lingmo-fragment-workspace">{children}</div>
    </div>
    {showImport && <LingmoImportDialog projectId={workspace.project.id} writable={writable} onClose={() => setShowImport(false)} onImported={async next => { await mutate('workspace.get', {}, '分镜已导入，可按集场浏览。'); changeDocument((next as Workspace & { lingmoImportReport?: { documentId?: string } }).lingmoImportReport?.documentId || next.lingmoDocuments?.at(-1)?.id || ''); setReload(value => value + 1); setShowImport(false); }}/>}
    {asset && <Modal title={`${lingmoTypeNames[lingmoAssetType(asset) || 'OM']}素材 · ${asset.name}`} onClose={() => setSelectedAsset(null)}><div className="lingmo-asset-detail"><div className="lingmo-asset-current"><MediaPreview media={currentMedia} onInspect={inspect}/><p>{currentMedia ? '当前采用素材' : '尚未配图，补充后采用即可用于本场片段。'}</p></div><div><p className="lingmo-asset-detail-type">{assetVersionLabel(asset)} · {lingmoAssetToken(asset)}</p><div className="actions"><Button disabled={!writable} icon="upload" onClick={() => void importImage(asset)}>{lingmoAssetType(asset) === 'SE' ? '导入声音' : '补充 / 替换图片'}</Button>{lingmoAssetType(asset) === 'OM' && <><Button disabled={!writable} onClick={() => void importImage(asset, 'audio')}>导入音频</Button><Button disabled={!writable} onClick={() => void importImage(asset, 'video')}>导入视频参考</Button></>}</div><p className="muted">补图与替换保留资产身份、版本及已有分镜引用。</p><details className="lingmo-asset-identity"><summary>资产身份详情</summary><p>资产 ID：{asset.id}</p><p>版本 ID：{asset.variantId || '未设置'}</p>{asset.externalId != null && <p>目录 ID：{String(asset.externalId)}</p>}</details></div></div>{assetCandidates.length > 0 && <><h3>已有素材 · {assetCandidates.length}</h3><div className="lingmo-candidate-grid">{assetCandidates.map(candidate => <article key={candidate.id}><MediaPreview media={candidate} compact onInspect={inspect}/><small>{candidate.name || candidate.id.slice(0, 8)}{(candidate as Media & { placeholder?: boolean }).placeholder ? ' · 白图占位' : ''}</small>{candidate.kind === 'image' && <Button executionAction="media.adopt" disabled={!writable || candidate.id === asset.adoptedMediaId || (candidate as Media & { placeholder?: boolean }).placeholder === true} onClick={() => void mutate('media.adopt', { assetId: asset.id, variantId: asset.variantId || '', mediaId: candidate.id }, '已采用该资产图片；引用身份保持不变。')}>{candidate.id === asset.adoptedMediaId ? '当前图片' : '采用此图片'}</Button>}</article>)}</div></>}{sceneData?.scene.declaredAssets != null && <JsonDetails title="原场级资产清单" value={sceneData.scene.declaredAssets}/>}</Modal>}
  </section>;
}

export function LingmoPrompt({ text, mentions, assets }: { text: string; mentions?: LingmoMention[]; assets: Asset[] }) {
  const refs = mentions || lingmoMentions(text, assets);
  return <pre className="prompt-document lingmo-prompt">{lingmoPromptParts(text, refs).map((part, index) => part.mention ? <button type="button" className={`lingmo-token ${part.mention.assetId ? '' : 'unbound'}`} key={index} title={part.mention.assetId ? `查看绑定资产 ${part.mention.name}` : '尚未唯一绑定，保存时会再次校验'} onClick={() => { if (!part.mention?.assetId) return; const card = document.getElementById(`lingmo-asset-${part.mention.assetId}`); card?.focus({ preventScroll: true }); card?.click(); }}>{part.text}</button> : part.text)}</pre>;
}

export function LingmoShotDetails({ unit }: { unit: Unit }) {
  const shots = Array.isArray(unit.shots) ? unit.shots as Record<string, unknown>[] : [];
  return <><p className="muted">容器时长 {String(unit.duration ?? '—')} 秒 · 实际镜头时间线 {String(unit.actualDurationSeconds ?? '—')} 秒 · {shots.length} 镜</p><details className="details"><summary>逐镜内容 · {shots.length} 镜</summary>{shots.map((shot, index) => <div key={index}><strong>分镜 {String(shot.number ?? index + 1)} · {String(shot.durationSeconds ?? shot.duration ?? '—')} 秒</strong><pre className="prose">{String(shot.rawText ?? shot.text ?? JSON.stringify(shot, null, 2))}</pre></div>)}</details></>;
}

export function LingmoUnitEditor({ unit, workspace, writable, mutate, onError, onClose }: { unit: Unit; workspace: Workspace; writable: boolean; mutate: Mutation; onError: (message: string) => void; onClose: () => void }) {
  const [draft, setDraft] = useState(unit.prompt), [busy, setBusy] = useState(false), [original, setOriginal] = useState(''), [showPicker, setShowPicker] = useState(false), [query, setQuery] = useState('');
  const textarea = useRef<HTMLTextAreaElement>(null), cursor = useRef({ start: draft.length, end: draft.length });
  const [references, setReferences] = useState<AssetRef[]>([...(unit.assetRefs || [])].sort((a, b) => a.order - b.order));
  const [mediaRefs, setMediaRefs] = useState<(NonNullable<Unit['mediaRefs']>[number] & { assetId?: string; variantId?: string })[]>([...(unit.mediaRefs || [])]);
  const [selectedBindings, setSelectedBindings] = useState<Record<string, Pick<AssetRef, 'assetId' | 'variantId'>>>({});
  const preservedBindings = useMemo(() => [...(Array.isArray(unit.audioAssetRefs) ? unit.audioAssetRefs as AssetRef[] : []), ...(Array.isArray(unit.mentions) ? (unit.mentions as LingmoMention[]).filter(mention => mention.assetId).map(mention => ({ assetId: mention.assetId!, variantId: mention.variantId, role: mention.type, order: 0 })) : [])], [unit]);
  const mentions = useMemo(() => boundLingmoMentions(draft, workspace.assets, [...references, ...preservedBindings], selectedBindings), [draft, workspace.assets, references, preservedBindings, selectedBindings]);
  const candidates = workspace.assets.filter(asset => lingmoAssetToken(asset) && (!query || asset.name.includes(query))).slice(0, 60);
  async function save() {
    setBusy(true);
    try { const next = await mutate('units.save', { unit: { ...unit, expectedVersion: unit.version, prompt: draft, mentions, assetRefs: references.map((ref, index) => ({ ...ref, order: index + 1 })), mediaRefs } }, '片段编辑稿已保存，原始导入正文保留。'); if (next) onClose(); }
    catch (reason) { onError(messageOf(reason)); } finally { setBusy(false); }
  }
  async function viewOriginal() {
    try { const result = await request<{ text: string; sha256: string }>({ action: 'lingmo.original', projectId: workspace.project.id, documentId: lingmoUnitDocument(unit), unitId: unit.id }); setOriginal(result.text); }
    catch (reason) { onError(messageOf(reason)); }
  }
  function insert(asset: Asset) { const token = lingmoAssetToken(asset); if (!token) return; const next = insertLingmoToken(draft, cursor.current.start, cursor.current.end, token); setSelectedBindings(current => ({ ...current, [token]: { assetId: asset.id, variantId: asset.variantId } })); setDraft(next.text); if (lingmoAssetType(asset) !== 'SE' && !references.some(ref => ref.assetId === asset.id)) setReferences(current => [...current, { assetId: asset.id, variantId: asset.variantId, role: lingmoAssetType(asset) === 'SC' ? 'scene' : lingmoAssetType(asset) === 'PR' ? 'prop' : 'character', order: current.length + 1 }]); setShowPicker(false); queueMicrotask(() => { textarea.current?.focus(); textarea.current?.setSelectionRange(next.cursor, next.cursor); }); }
  function replaceReference(index: number, assetId: string) { const asset = workspace.assets.find(item => item.id === assetId), previous = references[index]; if (!asset || !previous) return; const token = lingmoAssetToken(asset); if (!token) return; setSelectedBindings(current => ({ ...current, [token]: { assetId: asset.id, variantId: asset.variantId } })); setDraft(replaceLingmoReference(draft, mentions, previous.assetId, token)); setReferences(current => current.map((ref, position) => position === index ? { ...ref, assetId, variantId: asset.variantId, mediaId: undefined } : ref)); }
  return <Modal title={`编辑导入片段 · ${unit.title || unit.id}`} onClose={onClose}><p className="muted">原集场、编号和原件保留；保存后重新解析镜头与引用，不重跑前面阶段。</p><label>完整片段正文<textarea ref={textarea} rows={18} value={draft} disabled={!writable || busy} onSelect={event => { const target = event.currentTarget; cursor.current = { start: target.selectionStart, end: target.selectionEnd }; }} onChange={event => { setDraft(event.target.value); cursor.current = { start: event.target.selectionStart, end: event.target.selectionEnd }; if (event.target.value[event.target.selectionStart - 1] === '@') setShowPicker(true); }}/></label><Button disabled={!writable || busy} onClick={() => setShowPicker(value => !value)}>插入资产标签 @</Button>{showPicker && <div className="lingmo-picker"><input aria-label="查找资产标签" value={query} onChange={event => setQuery(event.target.value)} placeholder="按完整名称查找资产"/>{candidates.map(asset => <Button key={asset.id} onClick={() => insert(asset)}>{lingmoAssetToken(asset)} · {assetVersionLabel(asset)}</Button>)}{!candidates.length && <p>没有可插入的资产。</p>}</div>}<details className="details" open><summary>标签预览 · {mentions.length} 处</summary><LingmoPrompt text={draft} mentions={mentions} assets={workspace.assets}/></details>{mentions.some(item => !item.assetId) && <p className="warning">存在未匹配或同名歧义标签；可保存文本，制作前需核对绑定。</p>}<details className="details"><summary>本片段配图与顺序 · {references.length} 项</summary><p className="muted">更换资产将同步其完整标签；排序只调整渠道图片顺序，保留正文顺序。</p>{references.map((reference, index) => <div className="lingmo-import-row" key={`${reference.assetId}-${index}`}><span>{index + 1}</span><select aria-label={`片段配图 ${index + 1}`} disabled={!writable || busy} value={reference.assetId} onChange={event => replaceReference(index, event.target.value)}>{workspace.assets.filter(asset => lingmoAssetToken(asset) && lingmoAssetType(asset) !== 'SE').map(asset => <option key={asset.id} value={asset.id}>{lingmoAssetToken(asset)} · {assetVersionLabel(asset)}</option>)}</select><Button disabled={!writable || busy || index === 0} onClick={() => setReferences(current => moveItem(current, index, index - 1))}>↑</Button><Button disabled={!writable || busy || index === references.length - 1} onClick={() => setReferences(current => moveItem(current, index, index + 1))}>↓</Button></div>)}</details><details className="details"><summary>本片段声音与其他素材</summary>{[...new Set(mentions.filter(mention => mention.type === 'SE' || mention.type === 'OM').map(mention => mention.assetId).filter(Boolean))].map(assetId => { const asset = workspace.assets.find(item => item.id === assetId); if (!asset) return null; const candidates = workspace.media.filter(media => media.assetId === asset.id && (media.variantId || '') === (asset.variantId || '') && (media.kind === 'audio' || media.kind === 'video')); return <label key={asset.id}>{lingmoAssetToken(asset)}<select value={mediaRefs.find(ref => ref.assetId === asset.id)?.mediaId || ''} disabled={!writable || busy} onChange={event => { const mediaId = event.target.value; setMediaRefs(current => [...current.filter(ref => ref.assetId !== asset.id), ...(mediaId ? [{ assetId: asset.id, variantId: asset.variantId, mediaId, role: lingmoAssetType(asset) === 'SE' ? 'sound' : 'reference' }] : [])]); }}><option value="">尚未绑定音视频</option>{candidates.map(media => <option key={media.id} value={media.id}>{media.name || media.id} · {media.kind}</option>)}</select>{!candidates.length && <small>请先在本场资产区导入声音或视频参考。</small>}</label>; })}</details><Button onClick={() => void viewOriginal()}>查看不可变导入原文</Button>{original && <details className="details" open><summary>导入原文</summary><pre className="prompt-document">{original}</pre></details>}<div className="form-actions"><Button onClick={onClose}>取消</Button><Button variant="primary" disabled={!writable || busy || !draft.trim() || !importedUnit(unit)} onClick={() => void save()}>保存编辑版本</Button></div></Modal>;
}
