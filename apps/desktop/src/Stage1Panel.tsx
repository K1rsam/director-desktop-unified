import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { choose, request } from './api';
import { Badge, Button, Empty, kindName, messageOf, Modal } from './components';
import { productionDirection, projectProductionBusy, records, sourceEpisodes, sourceEvidence, stageJobs, words } from './stageProduction';
import { SERIES_LITE_WORKFLOW_PROFILE } from './workflowNavigation';
import { foundationCoverage, foundationRunOptions } from './foundationCoverage';
import { foundationSavedProgress } from './foundationProgress';
import { foundationParallelResume } from './foundationTrial';
import { WorkflowProgress } from './WorkflowProgress';
import { SourceStructure, sourceAnalysisState, sourceContextRows, sourceFormatNeedsAnalysis, sourceManualBlockers } from './SourceStructure';
import { FoundationChoices } from './FoundationChoices';
import type { SourceReadScope } from './SourceStructure';
import type { Job, Mutation, Workspace } from './types';
import './stage-production.css';

type Props = { workspace: Workspace; writable: boolean; mutate: Mutation; onError?: (error: string) => void; onNotice?: (notice: string) => void; onStoryboard?: () => void };
type Tab = 'source' | 'direction' | 'assets' | 'issues';

/** Summary-only hint. The guarded foundation.run entry verifies full source,
 * scope and original checkpoints; this helper does not authorize generation. */
export function foundationStartContinuation(workspace: Workspace, episodeIds: string[]): Job | null {
  const ids = new Set(episodeIds), version = workspace.source?.version;
  if (!ids.size || ids.size !== episodeIds.length || !version) return null;
  let continuation: Job | null = null, superseded = false;
  for (const job of workspace.jobs) {
    const snapshot = job.snapshot && typeof job.snapshot === 'object' ? job.snapshot as Record<string, unknown> : {};
    const input = snapshot.input && typeof snapshot.input === 'object' ? snapshot.input as Record<string, unknown> : {};
    if (job.kind !== 'workflow' || job.entityId !== 'foundation.run' || snapshot.runnerVersion !== 'desktop-runner/v1') continue;
    const source = input.source && typeof input.source === 'object' ? input.source as Record<string, unknown> : {};
    if ((source.version ?? snapshot.sourceVersion) !== version || source.sha256 && source.sha256 !== workspace.source?.sha256) continue;
    const selection = (input.selection ?? snapshot.selection) as { episodeIds?: unknown } | undefined;
    const selected = selection?.episodeIds;
    if (!Array.isArray(selected) || !selected.length || new Set(selected).size !== selected.length || selected.some(id => typeof id !== 'string' || !id)) continue;
    if (!selected.some(id => ids.has(id))) continue;
    // A newer completed scope cannot settle an older unknown submission, even
    // when the current selection is a subset or superset of its original scope.
    if (job.submissionUnknown || job.status === 'submission_unknown') return job;
    if (selected.length !== ids.size || selected.some(id => !ids.has(id))) continue;
    if (job.status === 'completed') { superseded = true; continue; }
    if (['cancelled', 'closed_by_user'].includes(job.status)) continue;
    if (!superseded && !continuation && ['queued', 'running', 'failed', 'interrupted', 'waiting_decision', 'partially_blocked', 'blocked'].includes(job.status)) continuation = job;
  }
  return continuation;
}
function originalFoundationScope(job: Job | null): string[] {
  const snapshot = job?.snapshot as { input?: { selection?: { episodeIds?: unknown } }; selection?: { episodeIds?: unknown } } | undefined;
  const ids = snapshot?.input?.selection?.episodeIds ?? snapshot?.selection?.episodeIds;
  return Array.isArray(ids) && ids.every(id => typeof id === 'string') ? ids : [];
}
function pendingFoundation(workspace: Workspace): Job | null {
  const full = foundationStartContinuation(workspace, foundationRunOptions(workspace).selection.episodeIds);
  if (full) return full;
  for (const job of stageJobs(workspace, 'foundation')) {
    const found = foundationStartContinuation(workspace, originalFoundationScope(job));
    if (found) return found;
  }
  return null;
}
export function Stage1Panel({ workspace, writable, mutate, onError, onNotice, onStoryboard }: Props) {
  const savedSource = workspace.source?.text || workspace.source?.content || '', episodes = sourceEpisodes(workspace), rows = workspace.source?.rows || [], foundation = workspace.foundation;
  const [episode, setEpisode] = useState('all'), [scene, setScene] = useState(''), [tab, setTab] = useState<Tab>('source');
  const [readingContext, setReadingContext] = useState(false);
  const startingRef = useRef(false), [starting, setStarting] = useState(false);
  const [firstBatch, setFirstBatch] = useState('1');
  const analysisPendingRef = useRef(false), [analysisPending, setAnalysisPending] = useState(false);
  const [editingSource, setEditingSource] = useState(false), [sourceDraft, setSourceDraft] = useState(savedSource);
  const savedDirection = productionDirection(workspace), [directionDraft, setDirectionDraft] = useState(savedDirection), [directionDirty, setDirectionDirty] = useState(false);
  const [directionVersion, setDirectionVersion] = useState({ source: workspace.source?.version || 0, foundation: Number(foundation?.version || 0) });
  const [search, setSearch] = useState(''), [evidence, setEvidence] = useState<{ title: string; refs: unknown } | null>(null);
  const [legacy, setLegacy] = useState<{ path: string; projects: { id: string; title: string }[]; dbPath?: string } | null>(null), [legacyProject, setLegacyProject] = useState('');
  useEffect(() => { if (!editingSource) setSourceDraft(savedSource); }, [savedSource, editingSource]);
  useEffect(() => { if (!directionDirty) { setDirectionDraft(savedDirection); setDirectionVersion({ source: workspace.source?.version || 0, foundation: Number(foundation?.version || 0) }); } }, [savedDirection, workspace.source?.version, foundation?.version, directionDirty]);
  useEffect(() => { if (episode !== 'all' && !episodes.some(item => item.id === episode)) { setEpisode('all'); setScene(''); } }, [episodes, episode]);
  const blocking = (workspace.source?.diagnostics || []).filter(issue => issue.blocking);
  const needsAnalysis = sourceFormatNeedsAnalysis(workspace.source), manualBlockers = sourceManualBlockers(workspace.source), analysis = sourceAnalysisState(workspace.source, workspace.jobs);
  const formatBusy = analysisPending || analysis.pending;
  const issues: Record<string, unknown>[] = [...(workspace.source?.diagnostics || []).map(issue => ({ ...issue, origin: '原文范围' })), ...records(foundation?.issues).filter(issue => (issue.details as Record<string, unknown> | undefined)?.kind !== 'APPLIED_INTERPRETATION').map(issue => ({ ...issue, origin: '制作基础' }))];
  const coverage = foundationCoverage(workspace);
  const continuation = pendingFoundation(workspace), unknownContinuation = continuation?.submissionUnknown === true || continuation?.status === 'submission_unknown';
  const parallelContinuation = foundationParallelResume(workspace, continuation);
  const currentContinuation = episode === 'all' ? null : foundationStartContinuation(workspace, foundationRunOptions(workspace, episode).selection.episodeIds), currentUnknown = currentContinuation?.submissionUnknown === true || currentContinuation?.status === 'submission_unknown';
  const selection = episodes.filter(item => episode === 'all' || item.id === episode).map(item => item.id), creating = projectProductionBusy(workspace);
  const firstBatchIds = (firstBatch === 'all' ? episodes : episodes.slice(0, Number(firstBatch))).map(item => item.id);
  const textUnknown = workspace.jobs.some(job => job.cliSubmissionUnknown === true || ['text', 'workflow'].includes(job.kind || '') && (job.submissionUnknown === true || job.status === 'submission_unknown'));
  const contextRows = sourceContextRows(workspace.source);
  const selectedRows = rows.filter(row => (episode === 'all' || row.episodeId === episode) && (!scene || row.sceneId === scene));
  const displayedSource = readingContext ? contextRows.map(row => row.text).join('') : episode === 'all' && !scene ? savedSource : selectedRows.map(row => row.text).join('');
  function readScope(scope: SourceReadScope) { setEditingSource(false); setReadingContext(scope.kind === 'context'); setEpisode(scope.kind === 'context' ? 'all' : scope.episodeId || 'all'); setScene(scope.sceneId || ''); setTab('source'); }
  const selectedAssets = workspace.assets.filter(asset => {
    const scope = asset.applicability?.episodes || asset.applicability?.episodeIds || [];
    return (episode === 'all' || !scope.length || scope.includes(episode)) && (!search.trim() || `${asset.name} ${asset.description || ''}`.includes(search.trim()));
  });
  async function copy(value: string, notice: string) { try { await navigator.clipboard.writeText(value); onNotice?.(notice); } catch { onError?.('复制失败，请手动选择正文。'); } }
  async function importSource() { try { const paths = await choose('file'); if (paths[0] && await mutate('source.import', { path: paths[0] }, '原文已冻结保存。')) setEditingSource(false); } catch (error) { onError?.(messageOf(error)); } }
  async function importLegacy(json = false) { try { const paths = await choose(json ? 'file' : 'directory'); if (!paths[0]) return; if (json) { await mutate('legacy.import', { path: paths[0] }, '成果已导入，请核对身份和来源。'); return; } const result = await request<{ projects: { id: string; title: string }[]; dbPath?: string }>({ action: 'legacy.inspect', path: paths[0] }); setLegacy({ ...result, path: paths[0] }); setLegacyProject(result.projects[0]?.id || ''); } catch (error) { onError?.(messageOf(error)); } }
  async function saveSource(event: FormEvent) { event.preventDefault(); if (await mutate('source.import', { text: sourceDraft }, '完整原文已保存为新版本。')) setEditingSource(false); }
  async function saveDirection(event: FormEvent) { event.preventDefault(); if (!directionDraft.trim()) return; const next = await mutate('production.direction.save', { direction: directionDraft, expectedSourceVersion: directionVersion.source, expectedFoundationVersion: directionVersion.foundation }, '制作方向已确认保存，后续文字任务会读取此版本。'); if (next) setDirectionDirty(false); }
  async function run(explicitEpisodeId?: string) {
    const options = !explicitEpisodeId && continuation ? { selection: { episodeIds: originalFoundationScope(continuation) } } : foundationRunOptions(workspace, explicitEpisodeId), original = foundationStartContinuation(workspace, options.selection.episodeIds);
    if (!writable || creating || startingRef.current || analysisPendingRef.current || analysis.pending || original?.submissionUnknown || original?.status === 'submission_unknown') return;
    if (manualBlockers.length) { onError?.(manualBlockers.map(issue => issue.message).join('；') + '。这些记录需要先确认，AI 集场识别不能替代处理。'); return; }
    if (!savedSource || !options.selection.episodeIds.length && !needsAnalysis) { onError?.('请先导入原文，并确认未识别的集场结构。'); return; }
    startingRef.current = true; setStarting(true);
    // Keep the ordinary action name: the engine chooses an original workflow
    // only after validating its full source/scope, before preflight/reservation.
    try {
      if (needsAnalysis) await mutate('source.analyze', { expectedSourceVersion: workspace.source?.version || 0, episodeIds: options.selection.episodeIds, continueFoundation: !!original }, original ? '已启动原任务的格式识别续接，原制作基础合同保留。' : '已启动原文格式识别，不自动整理全剧资产。');
      else {
        const trial = foundationParallelResume(workspace, original);
        if (trial) await mutate('workflows.resume', trial, '已请求三路加速继续；保留原批次，只处理尚未提交的范围。');
        else await mutate('foundation.run', { options }, original ? '已继续原范围的制作基础流程，原请求与已核对步骤将保留复用。' : explicitEpisodeId ? '已开始整理当前一集的制作基础。' : '已开始整理全剧制作基础与资产目录。');
      }
    }
    finally { startingRef.current = false; setStarting(false); }
  }
  async function startStoryboard() {
    if (!writable || creating || startingRef.current || analysisPendingRef.current || analysis.pending || textUnknown) return;
    if (manualBlockers.length) { onError?.(manualBlockers.map(issue => issue.message).join('；')); return; }
    if (!savedSource) return;
    startingRef.current = true; setStarting(true);
    try {
      if (needsAnalysis) {
        await mutate('source.analyze', { expectedSourceVersion: workspace.source?.version || 0, episodeIds: firstBatchIds, continueFoundation: false }, '已启动原文格式识别；保留原文，不启动全剧资产整理。识别完成后即可确认首批创作范围。');
        return;
      }
      if (!firstBatchIds.length) { onError?.('请先确认原文集场。'); return; }
      if (!directionDraft.trim()) { setTab('direction'); onNotice?.('请确认制作方向，再开始首批分镜；无需整理全剧资产。'); return; }
      if (directionDirty || !savedDirection.trim()) {
        const next = await mutate('production.direction.save', { direction: directionDraft, expectedSourceVersion: directionVersion.source, expectedFoundationVersion: directionVersion.foundation }, '制作方向已确认。');
        if (!next) return;
        setDirectionDirty(false);
      }
      const next = await mutate('storyboard.run', { workflowProfile: SERIES_LITE_WORKFLOW_PROFILE, episodeIds: firstBatchIds, options: { selection: { episodeIds: firstBatchIds } } }, `已开始 ${firstBatchIds.length} 集分镜；本集所需资料按需补齐，图片另行制作。`);
      if (next) onStoryboard?.();
    } finally { startingRef.current = false; setStarting(false); }
  }
  const savedFoundation = foundationSavedProgress(workspace);
  const evidenceRows = evidence ? sourceEvidence(workspace, evidence.refs) : [];
  return <div className="production-workbench stage-one-workbench">
    <div className="stage-intro"><div><h2>{continuation ? '继续原制作基础' : '确认原文与制作方向'}</h2><p>{continuation ? '原任务按冻结范围继续，已保存批次、回包和恢复记录保留。' : '确认原文和方向后即可开始首批分镜；只补本集需要的资料，无需等待全剧资产整理。'}</p></div><div className="actions"><Button icon="upload" disabled={!writable} onClick={() => void importSource()}>导入原文</Button>{!continuation && <label className="production-range-label">首批范围<select aria-label="首批创作范围" value={firstBatch} onChange={event => setFirstBatch(event.target.value)}><option value="1">先制作第 1 集</option><option value="3">前 3 集</option><option value="10">前 10 集</option><option value="all">全部 {episodes.length} 集</option></select></label>}<Button executionAction={needsAnalysis ? 'source.analyze' : continuation ? 'foundation.run' : 'storyboard.run'} variant="primary" icon="spark" disabled={!writable || !savedSource || !coverage.total && !needsAnalysis || !!manualBlockers.length || creating || starting || formatBusy || unknownContinuation || !continuation && textUnknown} onClick={() => void (continuation ? run() : startStoryboard())}>{analysis.unknown ? '核对原格式识别任务' : formatBusy || starting && needsAnalysis ? '正在识别原文格式' : unknownContinuation ? '保留原制作基础任务' : !continuation && textUnknown ? '保留原任务，先核对' : creating || starting ? '本项目正在制作' : needsAnalysis ? '识别原文格式' : continuation ? parallelContinuation ? '三路加速继续（试用）' : '继续原制作基础流程' : `确认并开始${firstBatchIds.length === 1 ? '首集' : `${firstBatchIds.length} 集分镜`}`}</Button></div></div>
    <WorkflowProgress workspace={workspace} stage="foundation" writable={writable} mutate={mutate} onError={onError}/>
    {analysis.message && <p role="status" className="production-inline-warning">{analysis.message}</p>}
    {needsAnalysis && !manualBlockers.length && !formatBusy && <p className="control-hint">将使用当前文字渠道识别未定片段的用途与集场，原文保持不变；识别后确认创作范围。无法确定的内容保留待确认。</p>}
    {manualBlockers.length > 0 && <p className="production-inline-warning">请先确认：{manualBlockers.map(issue => issue.message).join('；')}。这些问题不能只靠集场格式识别解决。</p>}
    {parallelContinuation && <p className="control-hint">保留已保存批次，未提交范围最多三路同时整理；随后并行核对，最后统一合并。原请求、回包与恢复记录保留。</p>}
    {continuation && <p className="production-inline-warning">{unknownContinuation ? '原范围有未知提交，先核对原执行；不会新建或重发。' : '同范围制作基础尚未完成；将继续原流程，已核对步骤按原冻结输入复用。'}</p>}
    {coverage.total > 0 && <p className={coverage.complete ? 'muted' : 'production-inline-warning'}>{`最终采用 ${coverage.count} / ${coverage.total} 集`}{coverage.stale ? ' · 原文已更新，请重新整理制作基础。' : coverage.complete ? ' · 全剧制作基础已完成。' : savedFoundation.hasSaved ? ' · 已保存批次仍在原任务中，核对与收口后最终采用；不需要从头整理。' : ` · 尚有 ${coverage.missingIds.length} 集未最终采用，整理与核对进度见上方。`}{coverage.count > 0 && !coverage.complete && <span>已采用：{coverage.coveredLabel}。</span>}</p>}
    {issues.some(item => item.blocking) && tab !== 'issues' && <div className="production-inline-warning"><span>{blocking.length ? '原文结构有待处理记录。' : '制作判断由 CLI 随整理自动处理，原文保持完整。'}</span><Button onClick={() => setTab('issues')}>查看记录</Button></div>}
    <div className="production-layout"><aside className="surface production-navigation" aria-label="集场目录"><div className="production-nav-title"><strong>集场目录</strong><span>{episodes.length} 集</span></div><button className={`production-nav-item ${episode === 'all' && !readingContext ? 'selected' : ''}`} onClick={() => { setEpisode('all'); setScene(''); setReadingContext(false); }}>完整原文与全部集数</button>{episodes.map(item => <div className="production-episode" key={item.id}><button className={`production-nav-item ${episode === item.id && !scene ? 'selected' : ''}`} onClick={() => { setEpisode(item.id); setScene(''); setReadingContext(false); }}><span>{item.title}</span><small>{workspace.source?.scenes?.filter(scene => scene.episodeId === item.id).length || 0} 场</small></button>{episode === item.id && workspace.source?.scenes?.filter(scene => scene.episodeId === item.id).map(item => <button key={item.id} className={`production-scene-item ${scene === item.id ? 'selected' : ''}`} onClick={() => { setScene(item.id); setReadingContext(false); setTab('source'); }}>{item.title}</button>)}</div>)}{contextRows.length > 0 && <button className={`production-nav-item ${readingContext ? 'selected' : ''}`} onClick={() => readScope({ kind: 'context' })}>背景资料</button>}{!episodes.length && <p className="muted">导入原文后自动识别分集与场景。</p>}<div className="production-nav-footer"><Button variant="ghost" icon="upload" disabled={!writable} onClick={() => void importLegacy()}>读取旧项目</Button><Button variant="ghost" disabled={!writable} onClick={() => void importLegacy(true)}>导入成果 JSON</Button></div></aside>
      <section className="surface production-reading"><div className="production-reading-heading"><div><h3>{readingContext && tab === 'source' ? '背景资料' : episode === 'all' ? '全部制作基础' : episodes.find(item => item.id === episode)?.title}</h3><small>原文 V{workspace.source?.version || 0} · 基础 V{Number(foundation?.version || 0)} · 当前阅读 {selection.length} 集 · {`最终采用 ${coverage.count} / ${coverage.total} 集`}</small></div><div className="actions">{episode !== 'all' && !readingContext && <Button executionAction={needsAnalysis ? "source.analyze" : "foundation.run"} disabled={!writable || creating || starting || formatBusy || currentUnknown || !!manualBlockers.length} onClick={() => void run(episode)}>{analysis.unknown ? '核对原格式识别任务' : formatBusy || starting && needsAnalysis ? '正在识别原文格式' : currentUnknown ? '保留原制作基础任务' : needsAnalysis ? '识别格式并整理当前集' : currentContinuation ? '继续原集制作基础流程' : '仅整理当前集'}</Button>}<Badge status={blocking.length ? 'blocked' : coverage.complete ? 'completed' : 'draft'}>{blocking.length ? '有片段待处理' : coverage.stale ? '原文已更新' : coverage.complete ? '全剧已整理' : coverage.count ? '部分已采用' : savedFoundation.hasSaved ? '批次已保存 · 待收口' : '等待整理'}</Badge></div></div>
        <div className="production-tabs" role="tablist" aria-label="制作基础内容">{([['source', '原文与集场'], ['direction', '制作方向'], ['assets', '资产目录'], ['issues', `核对与记录${issues.filter(item => item.blocking === true).length ? ` · 待处理 ${issues.filter(item => item.blocking === true).length}` : ''}`]] as [Tab, string][]).map(([id, label]) => <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? 'selected' : ''} onClick={() => setTab(id)}>{label}</button>)}</div>
        {tab === 'source' && <div className="production-tab-body">{workspace.source && savedSource && <SourceStructure source={workspace.source} writable={writable} mutate={mutate} onReadScope={readScope} jobs={workspace.jobs} analysisPending={analysisPending || starting && needsAnalysis} onAnalysisPendingChange={pending => { analysisPendingRef.current = pending; setAnalysisPending(pending); }}/>}<div className="production-reading-actions"><span className="muted">{readingContext ? '背景资料保留原文，作为后续制作依据。' : '完整原文逐字保留；左侧按集场阅读。'}</span><div className="actions"><Button variant="ghost" icon="copy" disabled={!displayedSource} onClick={() => void copy(displayedSource, '当前原文范围已复制。')}>复制原文</Button><Button disabled={!writable} onClick={() => { setEditingSource(!editingSource); setSourceDraft(savedSource); }}>{editingSource ? '返回阅读' : savedSource ? '保存新原文' : '粘贴原文'}</Button></div></div>{editingSource ? <form onSubmit={event => void saveSource(event)}><label>完整剧本原文<textarea className="production-source-input" rows={20} value={sourceDraft} onChange={event => setSourceDraft(event.target.value)}/></label><div className="form-actions"><Button type="submit" variant="primary" disabled={!writable || !sourceDraft.trim()}>冻结并保存</Button></div></form> : displayedSource ? <pre className="production-source-document">{displayedSource}</pre> : <Empty icon="source" title="尚未导入原文"><p>导入剧本文档，或粘贴完整原文。</p></Empty>}{savedSource && <details className="production-secondary"><summary>原文身份与制作规则</summary><p className="hash-value">SHA-256 {workspace.source?.sha256}</p>{workspace.rules.filter(rule => ['screenplay-fidelity', 'story-foundation'].includes(rule.id)).map(rule => <p className="muted" key={rule.id}>{rule.title || (rule.id === 'screenplay-fidelity' ? '原文保真' : '制作基础')} · V{rule.version || '未知'}</p>)}</details>}</div>}
        {tab === 'direction' && <div className="production-tab-body"><form onSubmit={event => void saveDirection(event)}><label>已确认制作方向<textarea rows={9} className="production-direction-input" placeholder="确认整剧表达、视觉基调和必须保留的制作约束。" value={directionDraft} onChange={event => { setDirectionDraft(event.target.value); setDirectionDirty(true); }}/></label><p className="muted">保存后作为后续创作的共同依据。模型提出的方向保留在下方，供你比较。</p><div className="form-actions"><Button type="submit" variant="primary" icon="check" disabled={!writable || !workspace.source || !directionDraft.trim() || !directionDirty}>保存制作方向</Button></div></form>{words(foundation?.direction) && <div className="production-proposal"><h4>制作方向提案</h4><p className="prose">{words(foundation?.direction)}</p></div>}{words(foundation?.overview) && <div className="production-proposal"><h4>全剧概要</h4><p className="prose">{words(foundation?.overview)}</p></div>}{records(foundation?.episodes).filter(item => episode === 'all' || words(item.episodeId) === episode).map(item => <div className="production-proposal" key={words(item.episodeId)}><h4>{episodes.find(episode => episode.id === item.episodeId)?.title || '分集概要'}</h4><p className="prose">{words(item.summary)}</p><p className="muted">{words(item.context)}</p></div>)}</div>}
        {tab === 'assets' && <div className="production-tab-body"><div className="production-reading-actions"><span className="muted">{selectedAssets.length} 个资产形象 · 按身份和适用集场管理</span><input aria-label="搜索资产目录" placeholder="搜索人物、场景、道具" value={search} onChange={event => setSearch(event.target.value)}/></div>{selectedAssets.length ? <div className="production-asset-directory">{selectedAssets.map(asset => { const facts = records(foundation?.facts).filter(fact => words(fact.assetId) === (asset.baseAssetId || asset.id) && (!words(fact.variantId) || words(fact.variantId) === asset.variantId)); return <article key={asset.id}><div><Badge status="draft">{kindName(asset.kind)}</Badge><strong>{asset.name}</strong><small>形象 V{asset.version || 1}</small></div><p className="prose">{asset.description || '形象说明待核对。'}</p><div className="actions"><Button variant="ghost" icon="source" onClick={() => setEvidence({ title: `${asset.name} · 原文来源`, refs: asset.sourceRefs })}>回读原文</Button></div>{facts.length > 0 && <details className="production-secondary"><summary>{facts.length} 项制作事实</summary>{facts.map((fact, index) => <div className="production-fact" key={words(fact.id) || index}><strong>{words(fact.attribute)}</strong><p>{words(fact.value)}</p><small>{fact.claim === 'CHARACTER_CLAIM' ? '角色自述 / 判断' : '原文事实'}{fact.scope === 'STATE' ? ' · 当前状态' : fact.scope === 'TRANSITION' ? ' · 状态变化' : ''}</small><Button variant="ghost" icon="source" onClick={() => setEvidence({ title: `${asset.name} · ${words(fact.attribute)}`, refs: fact.sourceRefs })}>事实回读</Button></div>)}</details>}</article>; })}</div> : <Empty icon="folder" title="尚无此范围的资产目录"><p>先整理制作基础，身份与形象会列在这里。</p></Empty>}</div>}
        {tab === 'issues' && <div className="production-tab-body"><FoundationChoices workspace={workspace} writable={writable} mutate={mutate} onError={onError} onNotice={onNotice} onRead={(title, refs) => setEvidence({ title, refs })}/>{issues.length ? issues.map((item, index) => <article className="production-issue" key={index}><Badge status={item.blocking ? 'blocked' : 'draft'}>{words(item.origin)}{item.blocking ? '待处理' : '记录'}</Badge><p>{messageOf(item.message || item.problem)}</p><div className="actions"><Button variant="ghost" icon="source" onClick={() => setEvidence({ title: '制作记录 · 来源回读', refs: item.rowIds || item.sourceRefs })}>回读来源</Button>{item.origin === '原文范围' && ['BOUNDARY_UNCONFIRMED', 'SCENE_BOUNDARY_UNCONFIRMED'].includes(words(item.code)) && <Button onClick={() => setTab('source')}>查看待确认片段</Button>}</div></article>) : <Empty icon="check" title="当前没有已记录的待处理事项"><p>整理后仍需核对来源、形象适用范围和制作方向。</p></Empty>}{workspace.lastImportReport && <div className="production-proposal"><h4>最近导入核对</h4>{records(workspace.lastImportReport.diagnostics).map((item, index) => <p key={index}>{messageOf(item)}</p>)}<p className="muted">{words(workspace.lastImportReport.note)}</p></div>}</div>}
      </section></div>
    {evidence && <Modal title={evidence.title} onClose={() => setEvidence(null)}><p className="muted">来自当前锁定原文 V{workspace.source?.version || 0}。</p>{evidenceRows.length ? evidenceRows.map(row => <div className="production-evidence-row" key={row.id}><small>原文第 {row.number} 行</small><pre>{row.text}</pre></div>) : <p className="warning">此项尚无当前可回读的源引用，不能以说明文字替代原文证据。</p>}</Modal>}
    {legacy && <Modal title="选择要导入的旧项目" onClose={() => setLegacy(null)}><label>旧项目<select value={legacyProject} onChange={event => setLegacyProject(event.target.value)}>{legacy.projects.map(item => <option key={item.id} value={item.id}>{item.title}</option>)}</select></label><p className="path-value">{legacy.dbPath || legacy.path}</p><div className="form-actions"><Button onClick={() => setLegacy(null)}>取消</Button><Button executionAction="legacy.import" variant="primary" disabled={!writable || !legacyProject} onClick={async () => { if (await mutate('legacy.import', { path: legacy.path, legacyProjectId: legacyProject }, '旧成果已导入，请核对来源与身份。')) setLegacy(null); }}>导入所选项目</Button></div></Modal>}

  </div>;
}
