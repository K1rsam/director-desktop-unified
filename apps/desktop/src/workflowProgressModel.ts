import { matchesStage, record, records, stageJobs, words } from './stageProduction.ts';
import { jobActive } from './generationState.ts';
import type { Job, Workspace } from './types.ts';

type Node = Record<string, unknown>;
type Stage = 'foundation' | 'storyboard';
const accepted = (node: Node) => words(node.status) === 'accepted';

function progressEntries(workspace: Workspace): Node[] {
  const value: unknown = workspace.workflowProgress;
  return Array.isArray(value) ? records(value) : words(record(value).stage) ? [record(value)] : Object.values(record(value)).map(record);
}
function runId(value: Node): string { return words(value.workflowId) || words(value.runId); }
function created(job: Job): number { const stamp = Date.parse(job.createdAt || ''); return Number.isFinite(stamp) ? stamp : 0; }

/** Consume only Rust's bound resolution proof; titles and eventual completion aren't evidence. */
export function workflowNodeResolution(node: Node, nodes: Node[], workflowId: string): 'author-scope-repair' | 'continuity-superseded' | 'text-transport-recovered' | null {
  const proof = record(node.resolution), kind = words(proof.kind), originId = words(node.id), originHash = words(node.requestHash);
  if (!workflowId || proof.version !== 'workflow-step-resolution/v1' || proof.verified !== true || proof.workflowId !== workflowId
    || !['author-scope-repair', 'continuity-superseded', 'text-transport-recovered'].includes(kind) || !originId || proof.originNodeId !== originId
    || !/^[a-f0-9]{64}$/i.test(originHash) || proof.originRequestHash !== originHash || proof.resolverNodeId === originId) return null;
  if (kind === 'author-scope-repair' && words(node.status) !== 'rejected') return null;
  if (kind === 'text-transport-recovered' && words(node.status) !== 'failed') return null;
  const origins = nodes.filter(item => words(item.id) === originId), resolvers = nodes.filter(item => words(item.id) === words(proof.resolverNodeId));
  if (origins.length !== 1 || origins[0].requestHash !== originHash || resolvers.length !== 1) return null;
  const resolver = resolvers[0], resolverHash = words(resolver.requestHash);
  if (!/^[a-f0-9]{64}$/i.test(resolverHash) || proof.resolverRequestHash !== resolverHash || !accepted(resolver)) return null;
  if (kind === 'author-scope-repair' && (words(resolver.stage) !== 'storyboardFinish' || words(record(resolver.input).executionBatchVersion) !== 'author-domain-repair/v1'
    || words(node.stage) !== 'storyboardCreate' && !(words(node.stage) === 'storyboardFinish' && words(record(node.input).executionBatchVersion) === 'author-domain-repair/v1'))) return null;
  // Only Planner supersession currently has a complete core proof. Carrying a
  // tail revision on an Author/Finisher does not prove that its failure expired.
  if (kind === 'continuity-superseded' && (words(node.stage) !== 'storyboardPlan' || words(resolver.stage) !== 'storyboardPlan')) return null;
  if (kind === 'text-transport-recovered' && words(node.stage) !== words(resolver.stage)) return null;
  return kind as 'author-scope-repair' | 'continuity-superseded' | 'text-transport-recovered';
}

export function selectWorkflowProgress(workspace: Workspace, stage: Stage) {
  const entries = progressEntries(workspace).filter(item => matchesStage(item.stage, stage));
  const jobs = [...stageJobs(workspace, stage)].sort((left, right) => created(right) - created(left));
  const job = jobs[0];
  // A new run can precede its first checkpoint. Never show an older run's
  // accepted nodes or heartbeat underneath the new run's status.
  const progress = job ? entries.find(item => runId(item) === job.id) : entries[0];
  const projectedJob = job || workspace.jobs.find(item => item.id === runId(progress || {}));
  const nodes = records(progress?.nodes), id = runId(progress || {}) || projectedJob?.id || '';
  const currentNodes = nodes.filter(node => !workflowNodeResolution(node, nodes, id));
  return { progress, job: projectedJob, id, state: words(progress?.status) || projectedJob?.status || 'queued', nodes, currentNodes, completed: currentNodes.filter(accepted).length };
}

function episodeOf(node: Node, workspace: Workspace) {
  const input = record(node.input), request = record(node.request), requested = record(request.input);
  const explicit = words(input.episodeId) || words(record(input.episode).id) || words(requested.episodeId);
  const episodes = workspace.source?.episodes || [];
  if (explicit) return episodes.find(episode => episode.id === explicit);
  const stage = words(node.stage) || words(request.stage), key = words(node.id);
  return episodes.find(episode => episode.id && (key === `${stage}:${episode.id}` || key.startsWith(`${stage}:${episode.id}:`)));
}
export function storyboardNodeTitle(node: Node, workspace: Workspace): string {
  const phase = words(node.stage), titles: Record<string, string> = { storyboardPlan: '情绪与事件规划', storyboardCreate: '单元主创', storyboardFinish: '关联范围整编' };
  const title = titles[phase] || words(node.title) || words(node.label);
  const episode = episodeOf(node, workspace), episodeTitle = episode?.title || (episode?.number ? `第${episode.number}集` : '');
  const key = words(node.id), marker = key.lastIndexOf(':batch:');
  const packageId = marker >= 0 ? key.slice(marker + ':batch:'.length) : '';
  // Show a real package label when present; hashed editor keys are not a
  // promised batch count and must not become an invented B01/B02 sequence.
  const batch = phase === 'storyboardCreate' ? packageId.match(/(?:^|[-_:])([A-Za-z]\d{2,})$/)?.[1] : '';
  return [title, episodeTitle, batch ? `批次 ${batch}` : ''].filter(Boolean).join(' · ');
}
export function storyboardCompletion(nodes: Node[]): string | null {
  const phases = [['storyboardPlan', '情绪规划'], ['storyboardCreate', '单元主创'], ['storyboardFinish', '关联范围整编']];
  const result = phases.flatMap(([phase, title]) => {
    const scoped = nodes.filter(node => words(node.stage) === phase);
    return scoped.length ? [`${scoped.filter(accepted).length} 批${title}`] : [];
  });
  return result.length ? result.join(' · ') : null;
}
export function secondsLabel(value: number): string {
  const total = Math.max(0, Math.floor(value));
  return total < 60 ? `${total} 秒` : `${Math.floor(total / 60)} 分 ${total % 60} 秒`;
}
export function executionSummary(node: Node, now = Date.now(), readFailed = false): string {
  if (words(node.status) !== 'running') return '';
  const execution = record(node.execution);
  if (execution.backend === 'codex_sdk') {
    const started = Date.parse(words(execution.startedAt)), saved = execution.elapsedSeconds;
    const elapsed = readFailed ? typeof saved === 'number' && Number.isFinite(saved) && saved >= 0 ? saved : NaN
      : Number.isFinite(started) && Number.isFinite(now) ? Math.max(0, (now - started) / 1000) : NaN;
    if (!Number.isFinite(elapsed)) return readFailed ? 'SDK 执行状态待刷新' : '正在读取 SDK 本批开始时间';
    const phase = words(execution.phase), event = words(execution.lastEventType), eventAt = Date.parse(words(execution.lastEventAt));
    const eventLabels: Record<string, string> = { 'thread.started': '会话已开始', 'turn.started': '本轮已开始', 'turn.completed': '本轮已结束', 'turn.failed': '本轮失败', 'item.started': '输出项开始', 'item.updated': '输出项更新', 'item.completed': '输出项完成', error: '执行错误记录', 'sdk.observation': '传输终态记录' };
    const label = readFailed ? 'SDK 执行状态待刷新 · 上次记录' : phase === 'output_received' ? 'SDK 已收到返回，正在核对'
      : phase === 'unknown' ? 'SDK 提交状态待核对，请查看任务记录'
      : event === 'turn.failed' || phase === 'failed' ? 'SDK 本轮执行失败，请查看任务记录'
        : event === 'turn.completed' ? 'SDK 本轮已结束，结果待核对' : '等待 SDK 返回';
    let result = label + ' · 已用时 ' + secondsLabel(elapsed);
    if (eventLabels[event] && Number.isFinite(eventAt)) {
      result += ' · 最后执行事件：' + eventLabels[event] + (readFailed ? '' : `（${secondsLabel(Math.max(0, (now - eventAt) / 1000))}前）`);
    } else result += ' · 尚无可核对的执行事件记录';
    if (!readFailed && phase === 'executing' && !['turn.completed', 'turn.failed', 'error'].includes(event)
      && (now - (Number.isFinite(eventAt) ? eventAt : started)) / 1000 >= 120) {
      result += ' · 较长时间未收到新执行事件，可查看任务记录';
    }
    return result;
  }
  const elapsed = Number(execution.elapsedSeconds), quiet = Number(execution.outputQuietSeconds);
  if (!Object.keys(execution).length || !Number.isFinite(elapsed) || elapsed < 0) return readFailed ? '当前执行状态待刷新' : '正在读取本批执行状态';
  const stamp = Date.parse(words(execution.observedAt)), age = Number.isFinite(stamp) ? Math.max(0, (now - stamp) / 1000) : Infinity;
  const alive = execution.processAlive === true || execution.processAlive === 1;
  const phase = words(execution.phase), fresh = !readFailed && age <= 90;
  const event = words(execution.lastEventType), turnCompleted = event === 'turn.completed', turnFailed = event === 'turn.failed';
  const failure = turnFailed ? 'CLI 本轮执行失败，请查看任务错误记录' : phase === 'failed' ? 'CLI 本地执行失败，请查看任务错误记录' : '';
  if (!fresh) return '执行状态暂未更新' + (failure ? ' · 最后记录：' + failure : '') + ' · 最后记录用时 ' + secondsLabel(elapsed);
  // A terminal turn event is stronger evidence than a historical stderr label.
  // Process exit alone does not prove that a usable result was received.
  const label = failure || (phase === 'cleanup' ? '正在结束本地 CLI，结果尚待确认'
    : phase === 'output_received' ? '已收到结果，正在核对'
      : turnCompleted ? 'CLI 本轮已完成，正在核对结果'
        : phase === 'writing_stdin' ? '正在发送制作资料'
          : phase === 'read_result' ? '正在读取 CLI 结果'
            : phase === 'draining' || !alive ? 'CLI 已退出，结果尚待核对' : '等待 CLI 返回');
  const liveAge = alive ? age : 0;
  const waiting = alive && phase === 'executing' && !turnCompleted && !turnFailed;
  const eventAge = execution.lastEventAgeSeconds, hasEventAge = typeof eventAge === 'number' && Number.isFinite(eventAge) && eventAge >= 0;
  // Diagnostic stderr activity is not a new semantic execution event.
  const quietAge = hasEventAge ? eventAge : quiet;
  const diagnosis = waiting && (execution.needsDiagnosis === true || execution.needsDiagnosis === 1 || Number.isFinite(quietAge) && quietAge + liveAge >= 120);
  const diagnosticLabels: Record<string, string> = { NETWORK_RETRY: 'CLI 报告网络重连', NETWORK_TRANSPORT: 'CLI 报告网络连接异常', TIMEOUT: 'CLI 日志含超时提示（来源待核对）', RATE_LIMIT: 'CLI 报告请求限流', AUTHENTICATION: 'CLI 报告鉴权异常', CLI_ERROR: 'CLI 有运行错误记录' };
  const diagnostic = waiting ? diagnosticLabels[words(execution.stderrCategory)] : '';
  return label + ' · 已用时 ' + secondsLabel(elapsed + liveAge) + (diagnostic ? ' · ' + diagnostic : '') + (diagnosis ? ` · 较长时间未收到新${hasEventAge ? '执行事件' : '输出'}，执行监控持续跟踪` : '');
}

export function hasActiveWorkspaceTasks(workspace: Workspace): boolean {
  return workspace.jobs.some(jobActive)
    || progressEntries(workspace).some(progress => ['queued', 'running'].includes(words(progress.status)));
}

type PollOptions = {
  current: () => Workspace | null;
  busy: () => boolean;
  hidden: () => boolean;
  revision: () => number;
  alive: () => boolean;
  read: (projectId: string) => Promise<Workspace>;
  apply: (workspace: Workspace, checkedAt: string) => void;
  fail: (error: unknown, projectId: string, failedAt: string) => void;
  now?: () => number;
  readTimeoutMs?: number;
};
/** Only reads persisted workspaces. This function has no mutation/generation route. */
export function createWorkspacePoller(options: PollOptions) {
  type ReadIdentity = { projectId: string; revision: number };
  let inFlight: ReadIdentity | null = null, foreground: ReadIdentity | null = null;
  return async function poll(force = false): Promise<void> {
    const workspace = options.current();
    if (!options.alive() || !workspace || options.busy() || !force && options.hidden() && !hasActiveWorkspaceTasks(workspace)) return;
    const projectId = workspace.project.id, revision = options.revision();
    if (inFlight) {
      // Focus/visibility may arrive while a background read is still returning.
      // Coalesce one same-workspace read after it, without creating concurrent IPCs.
      if (force && inFlight.projectId === projectId && inFlight.revision === revision) foreground = { projectId, revision };
      return;
    }
    const current = () => options.alive() && options.current()?.project.id === projectId && options.revision() === revision && !options.busy();
    const fail = (error: unknown) => { if (current()) options.fail(error, projectId, new Date((options.now || Date.now)()).toISOString()); };
    inFlight = { projectId, revision };
    // An unreturned IPC read must become visible, but must not create a pile
    // of additional reads. Keep owning this one request until it settles.
    const watchdog = setTimeout(() => fail(new Error('本地进度读取超过期限，正在等待本次读取结束；以下为上次保存状态。')), options.readTimeoutMs ?? 15000);
    try {
      const next = await options.read(projectId);
      if (next.project.id !== projectId) throw new Error('读取结果与当前项目不一致，保留上次状态。');
      if (current() && !foreground) options.apply(next, new Date((options.now || Date.now)()).toISOString());
    } catch (error) {
      fail(error);
    } finally {
      clearTimeout(watchdog); inFlight = null;
      const pending = foreground; foreground = null;
      // A write, project switch, or unmount invalidates both the old response
      // and the coalesced foreground request. It must never read a new scope.
      if (pending && pending.projectId === projectId && pending.revision === revision && current()) await poll(true);
    }
  };
}
