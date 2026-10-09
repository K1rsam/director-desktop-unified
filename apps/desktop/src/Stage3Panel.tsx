import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { request } from './api';
import { Button, Empty, formatDate, messageOf, Modal } from './components';
import { productionDirection, projectProductionBusy, records, sourceEpisodes, sourceEvidence, words } from './stageProduction';
import { WorkflowProgress } from './WorkflowProgress';
import type { Job, Mutation, Unit, Workspace } from './types';
import './stage-production.css';
import { StoryboardRefinement } from './StoryboardRefinement';

type Props = { workspace: Workspace; writable: boolean; mutate: Mutation; onError?: (error: string) => void; onNotice?: (notice: string) => void };
type Tab = 'prompt' | 'shots' | 'source' | 'history';
// Zero units means unproduced, not a claim that other episodes are complete.
export function storyboardEpisodeScope(episodes: { id: string }[], units: { episodeId?: string }[], scope: string) {
  const emptyEpisodes = episodes.filter(item => !units.some(unit => unit.episodeId === item.id));
  const allEpisodesView = scope === 'all' || scope === 'empty';
  const selection = (scope === 'empty' ? emptyEpisodes : episodes.filter(item => scope === 'all' || item.id === scope)).map(item => item.id);
  return { emptyEpisodes, allEpisodesView, selection };
}

/** Summary-only UI hint. The engine verifies full source/range and original
 * checkpoints before resuming; this hint never authorizes a new generation. */
export function storyboardStartContinuation(workspace: Workspace, episodeIds: string[]): Job | null {
  const ids = new Set(episodeIds), version = workspace.source?.version;
  if (!ids.size || !version) return null;
  let continuation: Job | null = null, superseded = false;
  for (const job of workspace.jobs) {
    const snapshot = job.snapshot && typeof job.snapshot === 'object' ? job.snapshot as Record<string, unknown> : {};
    const input = snapshot.input && typeof snapshot.input === 'object' ? snapshot.input as Record<string, unknown> : {};
    if (job.kind !== 'workflow' || job.entityId !== 'storyboard.run' || snapshot.runnerVersion !== 'desktop-runner/v1') continue;
    const source = input.source && typeof input.source === 'object' ? input.source as Record<string, unknown> : {};
    if ((source.version ?? snapshot.sourceVersion) !== version || source.sha256 && source.sha256 !== workspace.source?.sha256) continue;
    const selection = (input.selection ?? snapshot.selection) as { episodeIds?: unknown; unitIds?: unknown } | undefined;
    const selected = selection?.episodeIds;
    if (!Array.isArray(selected) || selected.length !== ids.size || new Set(selected).size !== ids.size || selected.some(id => typeof id !== 'string' || !ids.has(id))) continue;
    if (Array.isArray(selection?.unitIds) && selection.unitIds.length) continue;
    // Finish scanning even after a newer completion: it cannot settle an older
    // unknown submission. The backend independently checks uncapped history.
    if (job.submissionUnknown || job.status === 'submission_unknown') return job;
    if (['completed', 'cancelled', 'closed_by_user'].includes(job.status)) { superseded = true; continue; }
    if (!superseded && !continuation && ['queued', 'running', 'failed', 'interrupted', 'waiting_decision', 'partially_blocked', 'blocked'].includes(job.status)) continuation = job;
  }
  return continuation;
}

type Version = { version: number; data: Unit; createdAt?: string };
export function Stage3Panel({ workspace, writable, mutate, onError, onNotice }: Props) {
  const episodes = sourceEpisodes(workspace), [episode, setEpisode] = useState('all'), [selectedId, setSelectedId] = useState(workspace.units[0]?.id || ''), [tab, setTab] = useState<Tab>('prompt');
  const [full, setFull] = useState<Unit | null>(workspace.units[0] || null), [loading, setLoading] = useState(false), [detailError, setDetailError] = useState('');
  const [editing, setEditing] = useState<Unit | null>(null), [promptDraft, setPromptDraft] = useState(''), [titleDraft, setTitleDraft] = useState('');
  const [redo, setRedo] = useState<Unit | null>(null), [instruction, setInstruction] = useState('');
  const [versions, setVersions] = useState<Version[]>([]), [historyLoading, setHistoryLoading] = useState(false), [historyError, setHistoryError] = useState(''), [versionPreview, setVersionPreview] = useState<Version | null>(null);
  const historyRequest = useRef(0), detailRequest = useRef(0), startingRef = useRef(false);
  const [starting, setStarting] = useState(false);
  const { emptyEpisodes, allEpisodesView, selection } = storyboardEpisodeScope(episodes, workspace.units, episode);
  const visibleUnits = workspace.units.filter(unit => allEpisodesView || unit.episodeId === episode), selected = visibleUnits.find(unit => unit.id === selectedId) || visibleUnits[0];
  const unit = full && selected && full.id === selected.id && full.version === selected.version ? full : selected;
  useEffect(() => { if (!allEpisodesView && !episodes.some(item => item.id === episode)) setEpisode('all'); }, [episode, episodes, allEpisodesView]);
  useEffect(() => { if (!visibleUnits.some(unit => unit.id === selectedId)) setSelectedId(visibleUnits[0]?.id || ''); }, [visibleUnits, selectedId]);
  useEffect(() => {
    let active = true;
    setDetailError(''); setVersionPreview(null); setVersions([]); setHistoryError(''); setHistoryLoading(false); historyRequest.current++; detailRequest.current++;
    if (!selected) { setFull(null); setLoading(false); return; }
    setLoading(true);
    void request<Unit>({ action: 'units.get', projectId: workspace.project.id, unitId: selected.id }).then(value => { if (active) setFull(value); }).catch(error => { if (active) setDetailError(messageOf(error)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [workspace.project.id, selected?.id, selected?.version]);
  const creating = projectProductionBusy(workspace);
  const continuation = storyboardStartContinuation(workspace, selection), unknownContinuation = continuation?.submissionUnknown === true || continuation?.status === 'submission_unknown';
  const blocking = (workspace.source?.diagnostics || []).filter(issue => issue.blocking);
  const sourceRows = unit ? sourceEvidence(workspace, unit.sourceRefs) : [], shots = records(unit?.shots);
  const direction = productionDirection(workspace), title = unit?.title || '分镜单元';
  async function copy() { if (!unit?.prompt) return; try { await navigator.clipboard.writeText(unit.prompt); onNotice?.('完整单元提示词已复制。'); } catch { onError?.('复制失败，请在正文中手动选择。'); } }
  async function run() {
    if (!writable || creating || startingRef.current || unknownContinuation) return;
    if (!selection.length || blocking.length) { onError?.('请先在第一阶段确认原文集场，再选择创作范围。'); return; }
    startingRef.current = true; setStarting(true);
    // Both new/continue intents use the guarded start entry. A summary alone
    // cannot select a historical workflow or bypass the backend source check.
    try { await mutate('storyboard.run', { options: { selection: { episodeIds: selection } }, episodeIds: selection }, continuation ? '已继续原范围的分镜流程，原请求与已核对步骤将保留复用。' : '所选范围的分镜文字创作已启动。'); }
    finally { startingRef.current = false; setStarting(false); }
  }
  async function edit() { if (!unit) return; const sequence = detailRequest.current; try { const value = await request<Unit>({ action: 'units.get', projectId: workspace.project.id, unitId: unit.id }); if (sequence !== detailRequest.current) return; if (value.id !== unit.id) throw new Error('完整文字与所选单元身份不一致，请刷新。'); setEditing(value); setPromptDraft(value.prompt); setTitleDraft(value.title || ''); } catch (error) { if (sequence === detailRequest.current) onError?.(messageOf(error)); } }
  async function save(event: FormEvent) { event.preventDefault(); if (!editing || !promptDraft.trim()) return; const next = await mutate('units.save', { unit: { ...editing, title: titleDraft, prompt: promptDraft, promptOverride: promptDraft !== editing.prompt || editing.promptOverride === true, expectedVersion: editing.version } }, '分镜文字已保存为新版本，受影响的未提交素材包需要重新准备。'); if (next) { setEditing(null); setFull(null); } }
  async function regenerate(event: FormEvent) { event.preventDefault(); if (!writable || creating || loading || !redo || !instruction.trim()) return; const next = await mutate('units.regenerate', { unitId: redo.id, instruction: instruction.trim(), expectedVersion: redo.version }, '已开始重做此单元及必要关联范围，原版本与已有视频任务保留。'); if (next) setRedo(null); }
  async function loadHistory() {
    setTab('history'); if (!unit) return;
    const sequence = ++historyRequest.current; setHistoryLoading(true); setHistoryError('');
    try { const response = await request<{ versions: { version: number; data: Unit | string; createdAt?: string }[] }>({ action: 'units.versions.list', projectId: workspace.project.id, unitId: unit.id }); const values = response.versions.map(item => ({ ...item, data: typeof item.data === 'string' ? JSON.parse(item.data) as Unit : item.data })); if (sequence === historyRequest.current) setVersions(values); } catch (error) { if (sequence === historyRequest.current) setHistoryError(messageOf(error)); } finally { if (sequence === historyRequest.current) setHistoryLoading(false); }
  }
  async function restore(version: Version) { if (!unit || version.data.id !== unit.id) { onError?.('版本记录与当前单元身份不一致。'); return; } const next = await mutate('units.version.restore', { unitId: unit.id, version: version.version, expectedVersion: unit.version }, '已恢复为新的当前版本，原版本和已提交任务保留。'); if (next) { setFull(null); setVersionPreview(null); setTab('prompt'); } }
  let offset = 0;
  return <div className="production-workbench stage-three-workbench">
    <div className="stage-intro"><div><h2>生成每个分镜单元的完整文字提示词</h2><p>按原文创作，逐单元阅读、修改与复制；配图和视频制作在第四阶段完成。</p></div><div className="actions"><label className="production-range-label">创作范围<select aria-label="分镜创作范围" value={episode} onChange={event => { setEpisode(event.target.value); setTab('prompt'); }}><option value="all">全部已确认集数 · {episodes.length} 集</option><option value="empty">还没有分镜的集数 · {emptyEpisodes.length} 集</option>{episodes.map(item => <option key={item.id} value={item.id}>{item.title}</option>)}</select></label><Button executionAction="storyboard.run" variant="primary" icon="spark" disabled={!writable || !selection.length || !!blocking.length || creating || starting || unknownContinuation} onClick={() => void run()}>{creating ? '本项目正在制作' : starting ? '正在启动创作' : unknownContinuation ? '保留原分镜任务' : continuation ? '继续原分镜流程' : '生成分镜提示词'}</Button></div></div>
    {continuation && !creating && <p className="muted" role="status">{unknownContinuation ? '原范围有未知提交，请先核对原执行状态；不会另建或重发。' : '同范围的分镜尚未完成。此按钮继续原流程，已核对的步骤将复用。'}</p>}
    {episode === 'empty' && <p className="muted" role="status">仅创作当前零单元的 {emptyEpisodes.length} 集；已有单元的集数仍需各自核对是否完整。</p>}
    <WorkflowProgress workspace={workspace} stage="storyboard" writable={writable} mutate={mutate} onError={onError}/>
    {blocking.length > 0 && <div className="production-inline-warning">{blocking.map((item, index) => <p key={index}>{item.message}</p>)}<p>在第一阶段确认范围后，即可生成文字单元。</p></div>}
    <StoryboardRefinement workspace={workspace} episodeId={allEpisodesView ? undefined : episode} selectedUnit={unit} writable={writable} mutate={mutate} onError={onError}/>
    <div className="production-layout"><aside className="surface production-navigation" aria-label="分镜集场单元"><div className="production-nav-title"><strong>分镜目录</strong><span>{workspace.units.length} 个单元</span></div><button className={`production-nav-item ${episode === 'all' ? 'selected' : ''}`} onClick={() => setEpisode('all')}>全部集数</button>{episodes.filter(item => allEpisodesView || episode === item.id).map(item => { const units = workspace.units.filter(unit => unit.episodeId === item.id), scenes = workspace.source?.scenes?.filter(scene => scene.episodeId === item.id) || []; return <div className="production-episode" key={item.id}><button className="production-nav-item" onClick={() => { setEpisode(item.id); setSelectedId(units[0]?.id || ''); }}><strong>{item.title}</strong><small>{units.length} 单元</small></button>{scenes.map(scene => { const sceneUnits = units.filter(unit => unit.sceneId === scene.id); return <div className="production-scene-group" key={scene.id}><span>{scene.title}</span>{sceneUnits.map((item, index) => <button className={`production-unit-item ${unit?.id === item.id ? 'selected' : ''}`} key={item.id} onClick={() => { setSelectedId(item.id); setTab('prompt'); }}><span>{String(index + 1).padStart(2, '0')}</span><strong>{item.title || '未命名单元'}</strong><small>{item.durationSeconds || item.duration || '—'} 秒</small></button>)}</div>; })}{units.filter(item => !scenes.some(scene => scene.id === item.sceneId)).map(item => <button className={`production-unit-item ${unit?.id === item.id ? 'selected' : ''}`} key={item.id} onClick={() => { setSelectedId(item.id); setTab('prompt'); }}><strong>{item.title || '未命名单元'}</strong></button>)}{!units.length && <p className="production-nav-empty">此集尚无文字单元，可以直接启动创作。</p>}</div>; })}{!episodes.length && <p className="muted">先导入并确认原文，再按集创作。</p>}</aside>
      <section className="surface production-reading">{unit ? <><div className="production-reading-heading"><div><h3>{title}</h3><small>{episodes.find(item => item.id === unit.episodeId)?.title || '集数待核对'} · {unit.scene || workspace.source?.scenes?.find(scene => scene.id === unit.sceneId)?.title || '场景待核对'} · {unit.durationSeconds || unit.duration || '—'} 秒 · V{unit.version || 1}</small></div><div className="actions"><Button variant="ghost" icon="copy" disabled={!unit.prompt} onClick={() => void copy()}>复制全文</Button><Button disabled={!writable || loading} onClick={() => void edit()}>编辑文字</Button><Button executionAction="units.regenerate" icon="refresh" disabled={!writable || loading || creating} onClick={() => { if (!writable || loading || creating) return; setRedo(unit); setInstruction(''); }}>重做单元</Button></div></div><div className="production-tabs" role="tablist" aria-label="分镜单元内容">{([['prompt', '完整提示词'], ['shots', '镜表'], ['source', '来源与连续性'], ['history', '版本历史']] as [Tab, string][]).map(([id, label]) => <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? 'selected' : ''} onClick={() => { if (id === 'history') void loadHistory(); else setTab(id); }}>{label}</button>)}</div>{detailError && <p className="warning">完整单元读取失败：{detailError}</p>}{unit.promptOverride === true && <div className="production-inline-warning">正文经过手动修改，镜表仍保留原结构。重做相关单元后再核对两者，当前不会假定它们已经同步。</div>}
        {tab === 'prompt' && <div className="production-tab-body production-prompt-body">{unit.continuityStale === true && <div className="production-inline-warning" role="status">前置单元状态已变化，请重做此单元，核对连续状态。</div>}{direction && <details className="production-secondary"><summary>共同制作方向</summary><p className="prose">{direction}</p></details>}<pre className="production-full-prompt">{unit.prompt || '此单元尚无完整文字提示词。'}</pre>{records(unit.capacityIssues).map((item, index) => <p className="warning" key={index}>{words(item.message)}</p>)}</div>}
        {tab === 'shots' && <div className="production-tab-body">{shots.length ? <div className="production-shot-scroll"><table className="production-shot-table"><thead><tr><th>时间</th><th>画面与表演</th><th>摄影</th><th>光线</th><th>声音与原话</th><th>衔接</th></tr></thead><tbody>{shots.map((shot, index) => { const duration = Number(shot.duration || 0), start = offset; offset += duration; return <tr key={words(shot.id) || index}><td><strong>{index + 1}</strong><span>{start.toFixed(1)}—{offset.toFixed(1)} 秒</span></td><td>{words(shot.picture)}</td><td>{words(shot.camera)}</td><td>{words(shot.lighting)}</td><td><p>{words(shot.sound)}</p>{records(shot.dialogueParts).map((part, index) => <p key={index}><strong>{words(part.speaker)}{part.channel === 'OS' ? ' OS' : ''}：</strong>{words(part.text)}<small>镜内 {Number(part.voiceStart || 0).toFixed(1)}—{Number(part.voiceEnd || 0).toFixed(1)} 秒</small></p>)}</td><td>{words(shot.transition)}</td></tr>; })}</tbody></table></div> : <Empty icon="story" title={loading ? '正在读取完整镜表' : '此单元尚无结构化镜表'}><p>保留已有正文，重做单元可重新形成可核对的结构。</p></Empty>}</div>}
        {tab === 'source' && <div className="production-tab-body"><div className="production-continuity"><article><h4>单元初态</h4><p className="prose">{words(unit.initialState) || '尚未记录初态。'}</p></article><article><h4>单元末态</h4><p className="prose">{words(unit.exitState) || '尚未记录末态。'}</p></article></div>{words(unit.unitDirection) && <div className="production-proposal"><h4>单元表达</h4><p className="prose">{words(unit.unitDirection)}</p></div>}<h4>锁定原文回读</h4>{sourceRows.length ? sourceRows.map(row => <div className="production-evidence-row" key={row.id}><small>第 {row.number} 行 · 原文 V{workspace.source?.version || 0}</small><pre>{row.text}</pre></div>) : <p className="warning">未找到当前原文引用，需要核对来源。</p>}{(unit.assetRefs || []).length > 0 && <div className="production-proposal"><h4>结构化资产需求</h4>{unit.assetRefs?.map((ref, index) => <p className="muted" key={`${ref.assetId}:${index}`}>{index + 1}. {workspace.assets.find(asset => asset.id === ref.assetId && (!ref.variantId || ref.variantId === asset.variantId))?.name || '资产身份待核对'} · {ref.use || ref.role || '参考'} · {ref.variantId || '基础形象'}</p>)}</div>}</div>}
        {tab === 'history' && <div className="production-tab-body">{historyLoading ? <p className="muted">正在读取此单元的版本记录…</p> : historyError ? <div className="production-inline-warning"><p>{historyError}</p><Button onClick={() => void loadHistory()}>重新读取</Button></div> : versions.length ? <div className="production-version-list">{versions.map(version => <article key={version.version}><div><strong>V{version.version}</strong><small>{formatDate(version.createdAt)}{version.version === unit.version ? ' · 当前版本' : ''}</small></div><p>{version.data.title || '分镜单元'}</p><div className="actions"><Button onClick={() => setVersionPreview(version)}>阅读此版本</Button><Button icon="refresh" disabled={!writable || version.version === unit.version} onClick={() => void restore(version)}>恢复为当前版本</Button></div></article>)}</div> : <p className="muted">此单元尚无可读取的版本记录。</p>}</div>}</> : <Empty icon="story" title="尚未有分镜文字单元"><p>{selection.length ? `已选择 ${selection.length} 集原文。即使没有图片或既有单元，也可以开始创作文字提示词。` : '先在第一阶段导入并确认原文集场范围。'}</p></Empty>}</section></div>
    {editing && <Modal title="编辑单元文字" onClose={() => setEditing(null)}><form onSubmit={event => void save(event)}><p className="muted">仅编辑标题与完整文字，保留此版本的原文引用、结构和资产身份。手改正文不会自动改写镜表，保存后会明确标记。</p><label>单元标题<input value={titleDraft} onChange={event => setTitleDraft(event.target.value)}/></label><label>完整提示词<textarea className="production-text-editor" rows={20} value={promptDraft} onChange={event => setPromptDraft(event.target.value)}/></label><div className="form-actions"><Button onClick={() => setEditing(null)}>取消</Button><Button type="submit" variant="primary" disabled={!writable || !promptDraft.trim()}>保存文字新版本</Button></div></form></Modal>}
    {redo && <Modal title="重做分镜单元" onClose={() => setRedo(null)}><form onSubmit={event => void regenerate(event)}><p className="muted">基于此单元的原文范围、已确认方向和必要连续状态重做。仅修订关联范围；已有版本和已提交视频保持历史记录。</p><label>修改要求<textarea autoFocus rows={6} required placeholder="例如：加强人物躲闪的动作过程，保留全部原话和当前伤态。" value={instruction} onChange={event => setInstruction(event.target.value)}/></label><div className="form-actions"><Button executionAction="units.regenerate" onClick={() => setRedo(null)}>取消</Button><Button executionAction="units.regenerate" type="submit" variant="primary" icon="refresh" disabled={!writable || creating || loading || !instruction.trim()}>开始重做文字</Button></div></form></Modal>}
    {versionPreview && <Modal title={`${versionPreview.data.title || '分镜单元'} · V${versionPreview.version}`} onClose={() => setVersionPreview(null)}><pre className="production-full-prompt">{versionPreview.data.prompt}</pre><div className="form-actions"><Button onClick={() => setVersionPreview(null)}>关闭</Button><Button disabled={!writable || versionPreview.version === unit?.version} onClick={() => void restore(versionPreview)}>恢复为当前版本</Button></div></Modal>}
  </div>;
}
