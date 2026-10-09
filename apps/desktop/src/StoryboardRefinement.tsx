import { useEffect, useRef, useState } from 'react';
import { request } from './api';
import { Button, formatDate, JsonDetails, messageOf, Modal } from './components';
import type { Job, Mutation, Unit, Workspace } from './types';
import { projectProductionBusy } from './stageProduction';

type Candidate = {
  state?: string; candidateSha256?: string; episodeId?: string;
  draft?: { units?: Unit[] }; reviewDecisions?: unknown; productionFeedback?: unknown;
};
type Props = { workspace: Workspace; episodeId?: string; selectedUnit?: Unit; writable: boolean; mutate: Mutation; onError?: (error: string) => void };

export function StoryboardRefinement({ workspace, episodeId, selectedUnit, writable, mutate, onError }: Props) {
  const [opened, setOpened] = useState(false), [scope, setScope] = useState('episode');
  const [notes, setNotes] = useState(''), [preserve, setPreserve] = useState(''), [busy, setBusy] = useState(false);
  const [detail, setDetail] = useState<Job | null>(null), epoch = useRef(0);
  useEffect(() => { epoch.current++; setDetail(null); setOpened(false); setBusy(false); }, [workspace.project.id, episodeId]);
  const active = projectProductionBusy(workspace);
  const history = workspace.jobs.filter(job => job.entityId === 'storyboard.refine');
  const candidate = detail?.result as Candidate | undefined;
  const targetEpisode = candidate?.episodeId ? workspace.source?.episodes?.find(episode => episode.id === candidate.episodeId) : undefined;
  const targetLabel = targetEpisode?.title || (targetEpisode?.number ? `第 ${targetEpisode.number} 集` : candidate?.episodeId || '原任务所属集');
  const available = !!episodeId && workspace.units.some(unit => unit.episodeId === episodeId);
  const load = async (job: Job) => {
    const generation = ++epoch.current; setBusy(true);
    try { const found = await request<Job>({ action: 'jobs.detail', projectId: workspace.project.id, jobId: job.id }); if (generation === epoch.current) setDetail(found); }
    catch (error) { if (generation === epoch.current) onError?.(messageOf(error)); }
    finally { if (generation === epoch.current) setBusy(false); }
  };
  const start = async () => {
    if (busy || !writable || !available || active) return; setBusy(true);
    try {
      const result = await mutate('storyboard.refine', { episodeId, refinementScope: scope === 'unit' && selectedUnit ? { unitIds: [selectedUnit.id] } : {}, reviewNotes: notes, preserveNotes: preserve }, '已开始导演精修；完成后可预览并选择采用。');
      if (result) setOpened(false);
    } finally { setBusy(false); }
  };
  const adopt = async () => {
    if (busy || !writable || active || !detail || !targetEpisode || !candidate?.candidateSha256) return; setBusy(true);
    try { const result = await mutate('storyboard.refinement.adopt', { workflowId: detail.id, candidateSha256: candidate.candidateSha256 }, '已采用精修稿，原分镜保留在版本历史。'); if (result) setDetail(null); }
    finally { setBusy(false); }
  };
  return <section className="surface refinement-panel">
    <div className="section-heading compact"><div><h3>导演精修</h3><p className="muted">读取完整集稿，保留已完成的设计，只调整指定范围。</p></div><Button executionAction="storyboard.refine" icon="spark" disabled={!writable || !available || active || busy} onClick={() => { if (!writable || !available || active || busy) return; setScope('episode'); setOpened(true); }}>精修当前集</Button></div>
    {active && <p className="control-hint">本项目正在制作，请等待当前流程完成后再精修或采用；已有稿件和精修记录仍可查看。</p>}
    {!available && <p className="control-hint">选择一集已有分镜后，可对整集或当前单元精修。</p>}
    {history.length > 0 && <details className="details"><summary>全项目精修记录 · {history.length}</summary>{history.map(job => <div className="refinement-history" key={job.id}><span>{formatDate(job.createdAt)} · {job.status === 'completed' ? '可查看结果' : ['running', 'queued'].includes(job.status) ? '处理中' : '已中断，可继续原流程'}</span><Button disabled={busy} onClick={() => void load(job)}>查看</Button>{['failed', 'interrupted', 'blocked'].includes(job.status) && <Button executionAction="workflows.resume" disabled={!writable || busy || active} onClick={() => { if (!writable || busy || active) return; void mutate('workflows.resume', { workflowId: job.id }, '正在继续原精修流程。'); }}>继续</Button>}</div>)}</details>}
    {opened && <Modal title="导演精修" onClose={() => setOpened(false)}><label>调整范围<select value={scope} onChange={event => setScope(event.target.value)}><option value="episode">当前整集</option>{selectedUnit && selectedUnit.episodeId === episodeId && <option value="unit">当前单元 · {selectedUnit.title}</option>}</select></label><label>希望改进的地方（可选）<textarea rows={4} value={notes} onChange={event => setNotes(event.target.value)} placeholder="例如：增强人物动作衔接，缩短无效停顿"/></label><label>需要保留的优点（可选）<textarea rows={3} value={preserve} onChange={event => setPreserve(event.target.value)} placeholder="例如：保留现有情绪推进和结尾构图"/></label><p className="muted">精修结果先供预览，采用后保存为新版本。</p><div className="form-actions"><Button onClick={() => setOpened(false)}>取消</Button><Button executionAction="storyboard.refine" variant="primary" disabled={busy || !writable || active || !available} onClick={() => void start()}>开始精修</Button></div></Modal>}
    {detail && <Modal title={`导演精修结果 · ${targetLabel}`} onClose={() => { epoch.current++; setDetail(null); }}>
      <p className="control-hint">本次结果属于：{targetLabel}。{candidate?.episodeId && candidate.episodeId !== episodeId ? '与当前阅读集不同，采用会更新上述集数。' : ''}</p>
      {detail.error && <p className="warning">{messageOf(detail.error)}</p>}
      {candidate?.state === 'no_improvement' && <p>本次没有必要修改，保留当前稿。</p>}
      {candidate?.state === 'blocked' && <p className="warning">本次精修仍有未解决项，当前稿已保留。</p>}
      {candidate?.draft?.units?.map(unit => <article className="refinement-unit" key={unit.id}><h4>{unit.title}</h4><pre>{unit.prompt}</pre></article>)}
      <JsonDetails title="精修说明" value={candidate?.reviewDecisions}/><JsonDetails title="制作反馈" value={candidate?.productionFeedback}/>
      <div className="form-actions"><Button onClick={() => setDetail(null)}>关闭</Button>{candidate?.state === 'candidate' && <Button executionAction="storyboard.refinement.adopt" variant="primary" disabled={!writable || busy || active || !targetEpisode} onClick={() => void adopt()}>采用到{targetLabel}</Button>}</div>
    </Modal>}
  </section>;
}
