import { useEffect, useState } from 'react';
import { request } from './api';
import { Badge, Button, messageOf, Modal } from './components';
import { foundationCoverage } from './foundationCoverage';
import { record, records, words } from './stageProduction';
import { executionSummary, selectWorkflowProgress, storyboardCompletion, storyboardNodeTitle, workflowNodeResolution } from './workflowProgressModel';
import type { Job, Mutation, Workspace } from './types';

type Props = { workspace: Workspace; stage: 'foundation' | 'storyboard'; writable: boolean; mutate: Mutation; onError?: (message: string) => void };
const labels: Record<string, string> = { queued: '等待开始', running: '正在制作', received: '已收到文字', accepted: '已核对', validated: '已核对', rejected: '需要重做', completed: '制作完成', failed: '需要处理', interrupted: '已中断', waiting_decision: '等待你的选择', waitingDecision: '等待你的选择', partially_blocked: '关联范围待处理', blocked: '关联范围待处理', stale: '输入已变化' };
function foundationBatch(node: Record<string, unknown>) {
  const request = record(node.request), input = record(request.input || node.input), delivery = record(input.delivery || node.delivery);
  const value = words(request.stage) || words(node.stage);
  const phase = value === 'foundation:F2' ? 'F2' : value === 'foundation:Q1' ? 'Q1' : '';
  return phase ? { phase, title: phase === 'F2' ? '资产整理' : '资料整编', delivery: delivery.method === 'foundation-delivery/v2' ? delivery : {}, scope: request.scope ?? node.scope ?? input.scope ?? delivery.scope } : null;
}
function batchScope(value: unknown, workspace: Workspace): string {
  const ids = Array.isArray(value) ? [...new Set(value.filter((id): id is string => typeof id === 'string'))] : [];
  if (!ids.length) return '本批范围待核对';
  const episodes = ids.map(id => workspace.source?.episodes?.find(episode => episode.id === id));
  if (episodes.some(episode => !episode)) return '本批范围待核对';
  const numbers = episodes.map(episode => Number(episode?.number ?? episode?.sourceNumber));
  if (numbers.every(number => Number.isInteger(number) && number > 0) && new Set(numbers).size === numbers.length) {
    const sorted = [...numbers].sort((a, b) => a - b), ranges: string[] = [];
    for (let index = 0; index < sorted.length; index++) {
      const first = sorted[index]; let last = first;
      while (index + 1 < sorted.length && sorted[index + 1] === last + 1) last = sorted[++index];
      ranges.push(first === last ? String(first) : `${first}–${last}`);
    }
    return `第${ranges.join('、')}集`;
  }
  return episodes.map(episode => episode?.title || '未命名集数').join('、');
}
function batchTitle(node: Record<string, unknown>, workspace: Workspace): string {
  const batch = foundationBatch(node);
  if (!batch) return words(node.title) || words(node.label);
  const delivery = batch.delivery, index = Number(delivery.batchIndex), count = Number(delivery.batchCount), fragment = record(delivery.fragment);
  const parts = [batch.title, batchScope(batch.scope, workspace)];
  if (Number.isInteger(index) && index >= 0) parts.push(`本批 ${index + 1}${Number.isInteger(count) && count > 0 ? `（当前规划 ${count} 批）` : ''}`);
  if (words(fragment.episodeId) && Number.isInteger(fragment.index) && Number(fragment.index) >= 0) {
    const fragments = Number(fragment.count);
    parts.push(`本集片段 ${Number(fragment.index) + 1}${Number.isInteger(fragments) && fragments > 0 ? `（当前规划 ${fragments} 段）` : ''}`);
  }
  return parts.join(' · ');
}
function foundationCompletion(nodes: Record<string, unknown>[]): string | null {
  const batches = nodes.map(node => ({ node, batch: foundationBatch(node) })).filter(item => item.batch);
  if (!batches.length) return null;
  return (['F2', 'Q1'] as const).flatMap(phase => {
    const scoped = batches.filter(item => item.batch?.phase === phase);
    return scoped.length ? [`${scoped.filter(item => words(item.node.status) === 'accepted').length} 批${phase === 'F2' ? '资产整理' : '资料整编'}`] : [];
  }).join(' · ');
}
function candidatesOf(value: unknown): Record<string, unknown>[] {
  const data = record(value), decision = record(data.waitingDecision);
  const direct = records(decision.candidates).length ? records(decision.candidates) : records(data.candidates).length ? records(data.candidates) : records(data.fantasyCandidates);
  return direct.length ? direct : records(data.blocked).flatMap(item => records(item.fantasyCandidates));
}
function blockedSummaries(items: Record<string, unknown>[]) {
  const counts = new Map<string, number>();
  for (const item of items) {
    const reason = words(item.reason) || words(item.message) || '关联范围需要处理。';
    counts.set(reason, (counts.get(reason) || 0) + 1);
  }
  return [...counts].map(([reason, count]) => ({ reason, count }));
}
export function WorkflowProgress({ workspace, stage, writable, mutate, onError }: Props) {
  const [decision, setDecision] = useState<{ id: string; candidates: Record<string, unknown>[] } | null>(null), [selected, setSelected] = useState<string[]>([]), [loading, setLoading] = useState(false);
  const [clock, setClock] = useState(Date.now);
  const { progress, job, id, state, nodes, currentNodes, completed } = selectWorkflowProgress(workspace, stage);
  const unknown = job?.submissionUnknown === true || job?.status === 'submission_unknown' || progress?.submissionUnknown === true;
  const ticking = currentNodes.some(node => words(node.status) === 'running');
  useEffect(() => { if (!ticking) return; setClock(Date.now()); const timer = window.setInterval(() => setClock(Date.now()), 1000); return () => window.clearInterval(timer); }, [id, ticking]);
  if (!job && !progress) return null;
  const coverage = foundationCoverage(workspace);
  const readFailed = !!workspace.readState?.error;
  const batchesCompleted = stage === 'foundation' ? foundationCompletion(currentNodes) : storyboardCompletion(currentNodes);
  const views = nodes.map((node, index) => { const resolution = workflowNodeResolution(node, nodes, id); return { node, resolution, title: (stage === 'foundation' ? batchTitle(node, workspace) : storyboardNodeTitle(node, workspace)) || `制作步骤 ${index + 1}`, summary: resolution ? '' : executionSummary(node, clock, readFailed) }; });
  const currentTitles = views.filter(view => !view.resolution && words(view.node.status) === 'running').map(view => view.title);
  const activeViews = views.filter(view => !view.resolution && ['running', 'queued', 'received'].includes(words(view.node.status)));
  const blocked = records(progress?.blocked).length ? records(progress?.blocked) : records(record(job?.result).blocked);
  // A resumed run retains the previous attempt's result for evidence. Its
  // saved blockers are history until this attempt reaches a new checkpoint.
  const continuing = !readFailed && ['running', 'queued'].includes(state);
  const waiting = !continuing && (candidatesOf(progress).length || candidatesOf(job?.result).length || /waiting.?decision/i.test(state) || blocked.some(item => words(item.code) === 'AWAITING_FANTASY_SELECTION'));
  const interruption = <>{job?.error && <p className="warning">{messageOf(job.error)}</p>}{blocked.length > 0 && <div className="production-blocked">{blockedSummaries(blocked).map(({ reason, count }) => <p key={reason}>{reason}{count > 1 && <span className="muted">（{count} 个关联范围）</span>}</p>)}</div>}</>;
  async function openDecision() {
    if (!id) return;
    setLoading(true);
    try {
      const detail = await request<Job>({ action: 'jobs.detail', projectId: workspace.project.id, jobId: id });
      const candidates = candidatesOf(detail.result).length ? candidatesOf(detail.result) : candidatesOf(progress);
      if (!candidates.length) { onError?.('本次流程尚未提供可选择的创作机会，请核对待处理原因。'); return; }
      setDecision({ id, candidates }); setSelected(candidates.filter(item => item.recommended === true).map(item => words(item.id)).filter(Boolean));
    } catch (reason) { onError?.(messageOf(reason)); } finally { setLoading(false); }
  }
  async function confirmDecision() {
    if (!decision || unknown) return;
    const known = new Set(decision.candidates.map(item => words(item.id)));
    const next = await mutate('storyboard.decision.resolve', { workflowId: decision.id, selectedIds: selected.filter(id => known.has(id)) }, '选择已保存，继续制作相关文字单元。');
    if (next) setDecision(null);
  }
  return <section className="surface production-progress" aria-label="制作进度">
    <div className="production-progress-heading"><div><strong>{stage === 'foundation' ? '制作基础进度' : '分镜文字进度'}</strong><span>{readFailed ? '上次记录：' : ''}{stage === 'foundation' ? coverage.summary + ' · ' : ''}{batchesCompleted ? `已完成 ${batchesCompleted}` : nodes.length > 0 ? `已核对 ${completed} 个步骤` : '正在读取制作步骤，已完成结果会保留。'}</span>{currentTitles.length > 0 && <span>{readFailed ? '上次执行范围：' : '当前：'}{currentTitles.join('、')}</span>}</div><Badge status={unknown ? 'submission_unknown' : stage === 'foundation' && state === 'completed' && !coverage.complete ? 'draft' : state}>{readFailed ? '上次状态：' : ''}{unknown ? '原提交待核对' : stage === 'foundation' && state === 'completed' ? coverage.complete ? '全剧已整理' : '本次范围完成 · 全剧未完成' : labels[state] || '制作状态待核对'}</Badge></div>
    {activeViews.length > 0 && <div className="production-step-list">{activeViews.map(({ node, title, summary }, index) => <div key={words(node.id) || index}><span>{title}</span><Badge status={words(node.status)}>{labels[words(node.status)] || '正在制作'}</Badge>{summary && <span className="muted">{summary}</span>}</div>)}</div>}
    {nodes.length > 0 && <details className="details production-step-history"><summary>步骤与历史 · {nodes.length} 项记录{stage === 'storyboard' ? ` · 已发布 ${workspace.units.length} 个单元` : ''}</summary><div className="production-step-list">{views.map(({ node, title, summary, resolution }, index) => <div key={words(node.id) || index}><span>{title}</span><Badge status={resolution ? 'historical' : words(node.status)}>{readFailed ? '上次：' : ''}{resolution === 'author-scope-repair' ? '已修订 · 历史失败' : resolution === 'continuity-superseded' ? '历史分支 · 输入已变化' : resolution === 'text-transport-recovered' ? '已恢复 · 历史中断' : node.status === 'accepted' && (stage === 'storyboard' || foundationBatch(node)) ? '本批完成' : labels[words(node.status)] || '等待开始'}</Badge>{summary && <span className="muted">{summary}</span>}{resolution ? <details className="details"><summary>历史状态与原记录</summary><p className="muted">原状态：{labels[words(node.status)] || words(node.status)}。{resolution === 'author-scope-repair' ? '已由经过核验并采用的关联范围修订解决。' : resolution === 'text-transport-recovered' ? '原文本 CLI 已退出且未收到结果，经过有界恢复与正常校验后完成；原记录保留。' : '真实连续末态变化已触发新分支，后续对应步骤已核对；此记录不再属于当前范围。'}</p>{node.error != null && <p className="muted">原错误：{messageOf(node.error)}</p>}</details> : node.error != null && <p className="warning">{messageOf(node.error)}</p>}</div>)}</div></details>}
    {unknown && <p className="warning" role="status">先核对已收到的原结果；原提交仍未知时不会重新发送。历史记录与已完成内容保留。</p>}
    {(continuing || unknown) ? (job?.error || blocked.length > 0) && <details className="details production-step-history"><summary>{unknown ? '原中断记录（保留原文）' : '上次中断记录 · 当前正在继续'}</summary>{interruption}</details> : interruption}<div className="actions">{['interrupted', 'failed', 'partially_blocked', 'blocked', 'waiting_decision', 'waitingDecision'].includes(state) && <Button executionAction="workflows.resume" icon="play" disabled={!writable || !id} onClick={() => void mutate('workflows.resume', { workflowId: id }, unknown ? '已请求核对原结果；原提交仍未知时不会重新发送。' : '已继续原流程，已核对步骤将按有效输入复用。')}>{unknown ? '核对原结果后继续' : '继续原流程'}</Button>}{stage === 'storyboard' && !!waiting && <Button icon="check" disabled={!writable || loading || unknown} onClick={() => void openDecision()}>选择创作机会</Button>}</div>{decision && <Modal title="本次创作机会" onClose={() => setDecision(null)}><p className="muted">仅采用你在本次计划中选定的机会。可以全部取消，保留原文自身的表达。</p><div className="production-decision-list">{decision.candidates.map((item, index) => { const candidateId = words(item.id); return <label key={candidateId || index}><input type="checkbox" disabled={!candidateId || !writable} checked={selected.includes(candidateId)} onChange={event => setSelected(current => event.target.checked ? [...new Set([...current, candidateId])] : current.filter(id => id !== candidateId))}/><div><strong>{words(item.title) || words(item.subject) || `创作机会 ${index + 1}`}</strong><p>{words(item.proposal) || words(item.description) || words(item.reason) || words(item.emotionPurpose)}</p></div></label>; })}</div><div className="form-actions"><Button onClick={() => setDecision(null)}>取消</Button><Button executionAction="storyboard.decision.resolve" variant="primary" disabled={!writable || unknown} onClick={() => void confirmDecision()}>保存选择并继续</Button></div></Modal>}</section>;
}
