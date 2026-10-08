import test from 'node:test';
import assert from 'node:assert/strict';
import { createWorkspacePoller, executionSummary, hasActiveWorkspaceTasks, selectWorkflowProgress, storyboardCompletion, storyboardNodeTitle, workflowNodeResolution } from './workflowProgressModel.ts';
import type { Workspace } from './types.ts';

const stamp = Date.parse('2026-10-05T23:00:00Z');
const episodeId = 'episode:source:508';
const node = (stage: string, status: string, batch = '') => ({ id: `${stage}:${episodeId}${batch ? ':batch:package:awakening-' + batch : ':'}`, stage, status, input: { episodeId }, execution: status === 'running' ? { elapsedSeconds: 135, outputQuietSeconds: 125, processAlive: true, phase: 'executing', observedAt: new Date(stamp).toISOString() } : null });
function fixture(fresh = true): Workspace {
  const nodes = fresh ? [node('storyboardPlan', 'accepted'), node('storyboardCreate', 'accepted', 'B01'), node('storyboardCreate', 'running', 'B02')] : [node('storyboardPlan', 'running')];
  return { project: { id: 'p', title: '离线进度测试' }, source: { episodes: [{ id: episodeId, number: 1, title: '第一集' }] }, assets: [], media: [], units: [], settings: {}, providers: [], rules: [], jobs: [{ id: 'current', kind: 'workflow', entityId: 'storyboard.run', provider: 'local', status: 'running', createdAt: '2026-10-05T22:49:00Z' }], workflowProgress: [{ workflowId: 'current', stage: 'storyboard.run', status: 'running', completedSteps: 0, nodes }] };
}
function harness(initial: Workspace, reader: (id: string) => Promise<Workspace> = async () => fixture()) {
  let current: Workspace | null = initial, hidden = true, busy = false, revision = 0, alive = true, reads = 0, applied = 0, failures = 0;
  const poll = createWorkspacePoller({
    current: () => current, hidden: () => hidden, busy: () => busy, revision: () => revision, alive: () => alive, now: () => stamp + 5000,
    read: async id => { reads++; assert.equal(id, 'p'); return reader(id); },
    apply: (next, checkedAt) => { applied++; current = { ...next, readState: { checkedAt } }; },
    fail: (error, id, failedAt) => { failures++; if (current?.project.id === id) current = { ...current, readState: { ...current.readState, failedAt, error: String(error) } }; },
  });
  return { poll, get current() { return current; }, get reads() { return reads; }, get applied() { return applied; }, get failures() { return failures; }, setCurrent(value: Workspace | null) { current = value; }, setHidden(value: boolean) { hidden = value; }, setBusy(value: boolean) { busy = value; }, changeRevision() { revision++; }, close() { alive = false; } };
}
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }

test('后台活跃工作流继续只读刷新，旧Plan切换为真实已完成批次与当前B02', async () => {
  const h = harness(fixture(false));
  await h.poll();
  assert.equal(h.reads, 1); assert.equal(h.applied, 1);
  const selected = selectWorkflowProgress(h.current!, 'storyboard');
  assert.equal(selected.completed, 2, 'accepted nodes override an inconsistent zero summary');
  assert.equal(storyboardCompletion(selected.nodes), '1 批情绪规划 · 1 批单元主创');
  const current = selected.nodes.find(item => item.status === 'running')!;
  assert.equal(storyboardNodeTitle(current, h.current!), '单元主创 · 第一集 · 批次 B02');
  assert.match(executionSummary(current, stamp + 5000), /已用时 2 分 20 秒/);
  assert.match(executionSummary(current, stamp + 5000), /执行监控持续跟踪/);
});
test('没有活跃任务才暂停后台轮询，回到前台强制立即读取', async () => {
  const idle = fixture(); idle.jobs = []; idle.workflowProgress = [];
  const h = harness(idle); await h.poll(); assert.equal(h.reads, 0);
  await h.poll(true); assert.equal(h.reads, 1);
});
test('CLI诊断只使用固定分类，旧心跳和不可信分类不作为当前网络状态', () => {
  const current = selectWorkflowProgress(fixture(), 'storyboard').nodes[2];
  const execution = current.execution as Record<string, unknown>;
  execution.stderrCategory = 'NETWORK_RETRY';
  assert.match(executionSummary(current, stamp + 5000), /CLI 报告网络重连/);
  assert.doesNotMatch(executionSummary(current, stamp + 180000), /报告网络重连/);
  execution.stderrCategory = 'https://PRIVATE_HOST/?token=PRIVATE_KEY';
  assert.doesNotMatch(executionSummary(current, stamp + 5000), /PRIVATE/);
});

test('通用TIMEOUT只显示来源待核对日志，不推断模型连接失败且不改原记录', () => {
  const current = node('storyboardCreate', 'running', 'B02');
  const execution = current.execution as Record<string, unknown>;
  Object.assign(execution, { stderrCategory: 'TIMEOUT', lastEventType: 'turn.started', lastEventAgeSeconds: 358, outputQuietSeconds: 79, needsDiagnosis: false });
  const before = JSON.stringify(current), summary = executionSummary(current, stamp + 5000);
  assert.match(summary, /等待 CLI 返回/);
  assert.match(summary, /CLI 日志含超时提示（来源待核对）/);
  assert.match(summary, /较长时间未收到新执行事件/);
  assert.doesNotMatch(summary, /连接超时|连接失败|执行失败/);
  assert.equal(JSON.stringify(current), before);
});

test('stderr活跃不能重置明确的执行事件静默，旧格式仍用输出静默', () => {
  const current = node('storyboardCreate', 'running', 'B02');
  const execution = current.execution as Record<string, unknown>;
  Object.assign(execution, { outputQuietSeconds: 0, lastEventType: 'turn.started', lastEventAgeSeconds: 140, needsDiagnosis: false });
  assert.match(executionSummary(current, stamp + 5000), /较长时间未收到新执行事件/);
  execution.lastEventAgeSeconds = 10;
  assert.doesNotMatch(executionSummary(current, stamp + 5000), /较长时间/);
  delete execution.lastEventAgeSeconds;
  execution.outputQuietSeconds = 140;
  assert.match(executionSummary(current, stamp + 5000), /较长时间未收到新输出/);
});

test('turn.completed与已接收结果优先显示核对状态，不把历史TIMEOUT当成当前故障', () => {
  const current = node('storyboardCreate', 'running', 'B02');
  const execution = current.execution as Record<string, unknown>;
  Object.assign(execution, { lastEventType: 'turn.completed', stderrCategory: 'TIMEOUT', needsDiagnosis: true, lastEventAgeSeconds: 140 });
  assert.match(executionSummary(current, stamp + 5000), /CLI 本轮已完成，正在核对结果/);
  assert.doesNotMatch(executionSummary(current, stamp + 5000), /等待 CLI 返回|超时|较长时间/);
  Object.assign(execution, { phase: 'output_received', processAlive: false });
  assert.match(executionSummary(current, stamp + 5000), /已收到结果，正在核对/);
  assert.doesNotMatch(executionSummary(current, stamp + 5000), /超时|较长时间/);
  for (const status of ['accepted', 'completed']) assert.equal(executionSummary({ ...current, status }, stamp + 5000), '');
});

test('明确turn.failed或本地failed不因进程退出或先前turn.completed而误显示成功', () => {
  const current = node('storyboardCreate', 'running', 'B02');
  const execution = current.execution as Record<string, unknown>;
  Object.assign(execution, { processAlive: false, lastEventType: 'turn.failed', stderrCategory: 'TIMEOUT' });
  assert.match(executionSummary(current, stamp + 5000), /CLI 本轮执行失败/);
  assert.doesNotMatch(executionSummary(current, stamp + 5000), /已收到结果|本轮已完成/);
  Object.assign(execution, { lastEventType: 'turn.completed', phase: 'failed' });
  assert.match(executionSummary(current, stamp + 5000), /CLI 本地执行失败/);
  assert.doesNotMatch(executionSummary(current, stamp + 5000), /已收到结果|本轮已完成/);
  assert.match(executionSummary(current, stamp + 180000), /执行状态暂未更新.*最后记录：CLI 本地执行失败/);
  assert.match(executionSummary(current, stamp + 5000, true), /最后记录：CLI 本地执行失败/);
});

test('无完成事件的退出和cleanup不宣称已收到结果，正式失败节点仍留在当前失败范围', () => {
  const current = node('storyboardCreate', 'running', 'B02');
  const execution = current.execution as Record<string, unknown>;
  Object.assign(execution, { processAlive: false, lastEventType: 'turn.started' });
  assert.match(executionSummary(current, stamp + 5000), /CLI 已退出，结果尚待核对/);
  assert.doesNotMatch(executionSummary(current, stamp + 5000), /已收到结果|本轮已完成/);
  execution.phase = 'cleanup';
  assert.match(executionSummary(current, stamp + 5000), /正在结束本地 CLI，结果尚待确认/);
  const workspace = fixture(), failed = { ...current, status: 'failed', error: '合成明确传输失败' };
  workspace.workflowProgress![0].nodes = [failed];
  const selected = selectWorkflowProgress(workspace, 'storyboard');
  assert.equal(selected.completed, 0);
  assert.equal(selected.currentNodes[0].status, 'failed');
  assert.equal(selected.currentNodes[0].error, '合成明确传输失败');
});

test('文本恢复仅凭核心的完整绑定证明移出当前失败，不能掩盖未采用或跨岗结果', () => {
  const origin = { id: 'original', stage: 'storyboardCreate', status: 'failed', requestHash: 'a'.repeat(64), resolution: { version: 'workflow-step-resolution/v1', kind: 'text-transport-recovered', verified: true, workflowId: 'run', originNodeId: 'original', originRequestHash: 'a'.repeat(64), resolverNodeId: 'retry', resolverRequestHash: 'b'.repeat(64) } };
  const resolver = { id: 'retry', stage: 'storyboardCreate', status: 'accepted', requestHash: 'b'.repeat(64) };
  assert.equal(workflowNodeResolution(origin, [origin, resolver], 'run'), 'text-transport-recovered');
  for (const changed of [{ ...resolver, status: 'failed' }, { ...resolver, stage: 'storyboardPlan' }, { ...resolver, requestHash: 'c'.repeat(64) }]) assert.equal(workflowNodeResolution(origin, [origin, changed], 'run'), null);
  assert.equal(workflowNodeResolution(origin, [origin, resolver, resolver], 'run'), null);
  assert.equal(workflowNodeResolution({ ...origin, status: 'rejected' }, [origin, resolver], 'run'), null);
});
test('后台活跃媒体任务也更新，未知提交不能被当成待执行任务', () => {
  const workspace = fixture(); workspace.workflowProgress = []; workspace.jobs = [{ id: 'video', kind: 'video', provider: 'dreamina', status: 'processing' }];
  assert.equal(hasActiveWorkspaceTasks(workspace), true);
  workspace.jobs[0].submissionUnknown = true; assert.equal(hasActiveWorkspaceTasks(workspace), false);
});
test('读取失败明确保留上次状态，成功后清除过期警告而不改任务', async () => {
  let fail = true;
  const h = harness(fixture(), async () => { if (fail) throw new Error('offline database read failed'); return fixture(); });
  const before = JSON.stringify(h.current!.workflowProgress);
  await h.poll(); assert.equal(h.failures, 1); assert.match(h.current!.readState!.error!, /read failed/);
  assert.equal(JSON.stringify(h.current!.workflowProgress), before);
  const current = selectWorkflowProgress(h.current!, 'storyboard').nodes[2];
  assert.match(executionSummary(current, stamp + 5000, true), /最后记录用时/);
  assert.doesNotMatch(executionSummary(current, stamp + 5000, true), /等待 CLI/);
  fail = false; await h.poll(); assert.equal(h.current!.readState!.error, undefined); assert.equal(h.applied, 1);
});
test('同一只读请求未返回时不会再启动另一轮', async () => {
  const pending = deferred<Workspace>(), h = harness(fixture(), () => pending.promise);
  const first = h.poll(); await h.poll(); assert.equal(h.reads, 1);
  pending.resolve(fixture()); await first; assert.equal(h.applied, 1);
});
function videoReadiness(ready: number): Workspace {
  const workspace = fixture(); workspace.workflowProgress = [];
  workspace.jobs = Array.from({ length: 37 }, (_, index) => ({ id: `offline-video-${index}`, kind: 'video', provider: 'dreamina', status: index < ready ? 'ready' : 'queued', snapshot: { autoSubmit: false } }));
  return workspace;
}
test('后台取得最后一批准备结果后停止空闲读取，37条等待提交不再算活跃', async () => {
  const h = harness(videoReadiness(22), async () => videoReadiness(37));
  assert.equal(hasActiveWorkspaceTasks(h.current!), true);
  await h.poll(); assert.equal(h.current!.jobs.filter(job => job.status === 'ready').length, 37);
  assert.equal(hasActiveWorkspaceTasks(h.current!), false);
  await h.poll(); assert.equal(h.reads, 1, 'cached queued tasks must allow the final completion read before pausing');
  await h.poll(true); assert.equal(h.reads, 2, 'foreground refresh remains available while waiting for explicit submit');
});
test('恢复前台合并一次后续读取，旧22/37响应不能覆盖最新37/37且始终单读', async () => {
  const old = deferred<Workspace>(), latest = deferred<Workspace>();
  let calls = 0, concurrent = 0, peak = 0;
  const h = harness(videoReadiness(22), async () => {
    peak = Math.max(peak, ++concurrent);
    try { return await (++calls === 1 ? old.promise : latest.promise); } finally { concurrent--; }
  });
  const first = h.poll();
  await Promise.all(Array.from({ length: 30 }, () => h.poll(true)));
  assert.equal(h.reads, 1, 'foreground events never start parallel IPC reads');
  old.resolve(videoReadiness(22));
  await new Promise<void>(done => setImmediate(done));
  assert.equal(h.applied, 0, 'the response begun before foreground refresh is superseded');
  assert.equal(h.reads, 2, 'focus and visibility events coalesce into exactly one new read');
  latest.resolve(videoReadiness(37)); await first;
  assert.equal(h.applied, 1); assert.equal(peak, 1);
  assert.equal(h.current!.jobs.filter(job => job.status === 'ready').length, 37);
});
test('排队的前台读取随写revision、项目切换或卸载失效，不串读新scope', async () => {
  for (const change of ['revision', 'project', 'unmount']) {
    const old = deferred<Workspace>(), h = harness(videoReadiness(22), () => old.promise);
    const first = h.poll(); await h.poll(true);
    if (change === 'revision') h.changeRevision();
    if (change === 'project') h.setCurrent({ ...videoReadiness(37), project: { id: 'other', title: '另一离线项目' } });
    if (change === 'unmount') h.close();
    old.resolve(videoReadiness(22)); await first;
    assert.equal(h.reads, 1, change); assert.equal(h.applied, 0, change); assert.equal(h.failures, 0, change);
  }
});
test('不同revision的前台事件不能借旧请求排队，保留更新操作的结果', async () => {
  const old = deferred<Workspace>(), h = harness(videoReadiness(22), () => old.promise);
  const first = h.poll(); h.changeRevision(); h.setCurrent(videoReadiness(37)); await h.poll(true);
  old.resolve(videoReadiness(22)); await first;
  assert.equal(h.reads, 1); assert.equal(h.applied, 0);
  assert.equal(h.current!.jobs.filter(job => job.status === 'ready').length, 37);
});
test('旧读取失败后合并的前台读取仍执行，成功清除失败提示', async () => {
  const old = deferred<Workspace>(); let reads = 0;
  const h = harness(videoReadiness(22), async () => {
    if (++reads === 1) { await old.promise; throw new Error('old foreground race read failed'); }
    return videoReadiness(37);
  });
  const first = h.poll(); await h.poll(true); old.resolve(videoReadiness(22)); await first;
  assert.equal(h.reads, 2); assert.equal(h.failures, 1); assert.equal(h.applied, 1);
  assert.equal(h.current!.readState!.error, undefined);
});
test('前台事件不能让未返回读请求堆积，超时只提示一次并等原请求结束', async () => {
  const old = deferred<Workspace>(), warned = deferred<void>();
  let workspace = videoReadiness(22), reads = 0, failures = 0, applied = 0;
  const poll = createWorkspacePoller({ current: () => workspace, busy: () => false, hidden: () => true, revision: () => 0, alive: () => true, readTimeoutMs: 20,
    read: async () => ++reads === 1 ? old.promise : videoReadiness(37),
    apply: next => { workspace = next; applied++; }, fail: () => { failures++; warned.resolve(); } });
  const first = poll(); await Promise.all(Array.from({ length: 20 }, () => poll(true))); await warned.promise;
  await Promise.all(Array.from({ length: 20 }, () => poll(true)));
  assert.equal(reads, 1); assert.equal(failures, 1); assert.equal(applied, 0);
  old.resolve(videoReadiness(22)); await first;
  assert.equal(reads, 2); assert.equal(applied, 1); assert.equal(failures, 1);
});
test('未返回的本地读取有超时提示，仍只持有原读请求并在完成后恢复', async () => {
  const pending = deferred<Workspace>(), warned = deferred<void>();
  let reads = 0, failed = '', applied = 0;
  const poll = createWorkspacePoller({ current: () => fixture(), busy: () => false, hidden: () => true, revision: () => 0, alive: () => true, readTimeoutMs: 20,
    read: async () => { reads++; return pending.promise; }, apply: () => { applied++; }, fail: error => { failed = String(error); warned.resolve(); } });
  const first = poll(); await warned.promise;
  assert.match(failed, /超过期限/); await poll(); assert.equal(reads, 1); assert.equal(applied, 0);
  pending.resolve(fixture()); await first; assert.equal(applied, 1);
});
test('写操作或项目切换期间旧轮询返回不能覆盖新结果', async () => {
  for (const changeProject of [false, true]) {
    const pending = deferred<Workspace>(), h = harness(fixture(), () => pending.promise);
    const first = h.poll();
    if (changeProject) h.setCurrent({ ...fixture(), project: { id: 'other', title: '另一项目' } }); else h.changeRevision();
    pending.resolve(fixture(false)); await first; assert.equal(h.applied, 0); assert.equal(h.failures, 0);
  }
});
test('忙于前端操作或卸载后不启动进度请求', async () => {
  const h = harness(fixture()); h.setBusy(true); await h.poll(true); assert.equal(h.reads, 0);
  h.setBusy(false); h.close(); await h.poll(true); assert.equal(h.reads, 0);
});
test('投影顺序混杂仍按最新workflow身份绑定，不串用旧心跳', () => {
  const workspace = fixture(), old = { workflowId: 'old', stage: 'storyboard.run', status: 'running', nodes: [node('storyboardPlan', 'running')] };
  workspace.jobs.unshift({ id: 'old', kind: 'workflow', entityId: 'storyboard.run', provider: 'local', status: 'running', createdAt: '2026-10-01T00:00:00Z' });
  workspace.workflowProgress!.unshift(old);
  const before = JSON.stringify(workspace), selected = selectWorkflowProgress(workspace, 'storyboard');
  assert.equal(selected.id, 'current'); assert.equal(selected.nodes[0].status, 'accepted'); assert.equal(JSON.stringify(workspace), before);
});
test('新流程尚无投影时不借用旧流程完成数或执行状态', () => {
  const workspace = fixture(); workspace.jobs.unshift({ id: 'new', kind: 'workflow', entityId: 'storyboard.run', provider: 'local', status: 'queued', createdAt: '2026-10-06T00:00:00Z' });
  const selected = selectWorkflowProgress(workspace, 'storyboard');
  assert.equal(selected.id, 'new'); assert.equal(selected.state, 'queued'); assert.equal(selected.nodes.length, 0); assert.equal(selected.completed, 0);
});
test('只有已正式采用的步骤计入完成批次，未收到未来批次不猜总量', () => {
  const nodes = [node('storyboardCreate', 'accepted', 'B01'), node('storyboardCreate', 'received', 'B02'), node('storyboardCreate', 'rejected', 'B03')];
  assert.equal(storyboardCompletion(nodes), '1 批单元主创');
  assert.equal(storyboardCompletion([]), null);
});
test('Plan从精确集身份回读名称，整编哈希不伪造成主创批次号', () => {
  const workspace = fixture(), plan = node('storyboardPlan', 'accepted'); plan.input.episodeId = '';
  assert.equal(storyboardNodeTitle(plan, workspace), '情绪与事件规划 · 第一集');
  const finish = { ...node('storyboardFinish', 'running'), id: `storyboardFinish:${episodeId}:batch:finish:0123456789abcdef` };
  assert.equal(storyboardNodeTitle(finish, workspace), '关联范围整编 · 第一集');
});
test('过期心跳不外推用时或伪称当前进程存活，完成步骤不显示旧CLI', () => {
  const current = node('storyboardCreate', 'running', 'B03');
  assert.equal(executionSummary(current, stamp + 100000), '执行状态暂未更新 · 最后记录用时 2 分 15 秒');
  assert.equal(executionSummary({ ...current, status: 'accepted' }, stamp + 5000), '');
  assert.equal(executionSummary({ ...current, execution: null }, stamp), '正在读取本批执行状态');
});

function repairedFixture(kind = 'author-scope-repair') {
  const workspace = fixture(), originHash = 'a'.repeat(64), resolverHash = 'b'.repeat(64);
  const originStage = kind === 'continuity-superseded' ? 'storyboardPlan' : 'storyboardCreate', resolverStage = kind === 'continuity-superseded' ? 'storyboardPlan' : 'storyboardFinish';
  const origin = { ...node(originStage, 'rejected', 'B01'), requestHash: originHash, error: 'ASSET_SCENE_SCOPE', resolution: { version: 'workflow-step-resolution/v1', kind, workflowId: 'current', originNodeId: node(originStage, 'rejected', 'B01').id, originRequestHash: originHash, resolverNodeId: `${resolverStage}:bound-repair`, resolverRequestHash: resolverHash, verified: true } };
  const resolver = { ...node(resolverStage, 'accepted'), id: `${resolverStage}:bound-repair`, requestHash: resolverHash, input: { episodeId, executionBatchVersion: 'author-domain-repair/v1' } };
  const nodes = [origin, resolver, node('storyboardCreate', 'running', 'B02')];
  workspace.workflowProgress![0].nodes = nodes;
  return { workspace, origin, resolver, nodes };
}
test('已采用范围修订只消除绑定原节点的当前警告，保留原rejected及错误记录', () => {
  const { workspace, origin, nodes } = repairedFixture(), before = JSON.stringify(workspace);
  assert.equal(workflowNodeResolution(origin, nodes, 'current'), 'author-scope-repair');
  const selected = selectWorkflowProgress(workspace, 'storyboard');
  assert.equal(selected.nodes.length, 3); assert.equal(selected.currentNodes.length, 2); assert.equal(selected.completed, 1);
  assert.equal(selected.nodes[0].status, 'rejected'); assert.equal(selected.nodes[0].error, 'ASSET_SCENE_SCOPE');
  assert.equal(selected.currentNodes[0].status, 'accepted'); assert.equal(selected.currentNodes[1].status, 'running');
  assert.equal(JSON.stringify(workspace), before, 'projection never rewrites history or an original author acceptance');
});
test('修订证明逐项绑定workflow及原节点/修订节点哈希，缺字段或非accepted仍需要处理', () => {
  const { origin, resolver, nodes } = repairedFixture();
  for (const change of [{ version: 'unknown/v1' }, { kind: 'unknown' }, { verified: false }, { workflowId: 'other-workflow' }, { originNodeId: 'other-origin' }, { originRequestHash: 'c'.repeat(64) }, { resolverNodeId: 'missing' }, { resolverNodeId: origin.id }, { resolverRequestHash: 'c'.repeat(64) }]) {
    const changed = { ...origin, resolution: { ...origin.resolution, ...change } };
    assert.equal(workflowNodeResolution(changed, [changed, resolver], 'current'), null, JSON.stringify(change));
  }
  for (const status of ['completed', 'received', 'running', 'rejected']) assert.equal(workflowNodeResolution(origin, [origin, { ...resolver, status }], 'current'), null);
  assert.equal(workflowNodeResolution({ ...origin, resolution: undefined }, nodes, 'current'), null);
  assert.equal(workflowNodeResolution(origin, [origin], 'current'), null);
  assert.equal(workflowNodeResolution(origin, [...nodes, { ...resolver }], 'current'), null, 'ambiguous resolver identity cannot hide an error');
  assert.equal(workflowNodeResolution({ ...origin, requestHash: 'legacy-short-hash' }, nodes, 'current'), null);
  assert.equal(workflowNodeResolution(origin, [origin, { ...resolver, input: { episodeId, executionBatchVersion: 'related-range-finish/v1' } }], 'current'), null);
  const rejectedRepair = { ...origin, stage: 'storyboardFinish', input: { episodeId, executionBatchVersion: 'author-domain-repair/v1' } };
  assert.equal(workflowNodeResolution(rejectedRepair, [rejectedRepair, resolver], 'current'), 'author-scope-repair');
  assert.equal(workflowNodeResolution({ ...rejectedRepair, input: { episodeId } }, [rejectedRepair, resolver], 'current'), null);
});
test('同标题、相似修订键和流程completed不能替代修订证明，也不能借其他workflow的采用节点', () => {
  const { workspace, origin, resolver } = repairedFixture();
  const unresolved = { ...origin, resolution: undefined }, unrelated = { ...resolver, title: '单元主创', id: origin.id + ':author-domain-repair', status: 'completed' };
  assert.equal(workflowNodeResolution(unresolved, [unresolved, unrelated], 'current'), null);
  workspace.workflowProgress![0].status = 'completed'; workspace.workflowProgress![0].nodes = [unresolved, unrelated];
  assert.equal(selectWorkflowProgress(workspace, 'storyboard').currentNodes[0].error, 'ASSET_SCENE_SCOPE');
  workspace.jobs.unshift({ ...workspace.jobs[0], id: 'new-workflow', createdAt: '2026-10-06T01:00:00Z' });
  workspace.workflowProgress!.unshift({ workflowId: 'new-workflow', stage: 'storyboard.run', status: 'running', nodes: [origin, resolver] });
  const selected = selectWorkflowProgress(workspace, 'storyboard');
  assert.equal(selected.id, 'new-workflow'); assert.equal(selected.currentNodes.length, 2, 'proof from a different run cannot resolve this run');
});
test('只有正式末态变化证明才将旧分支移出当前计数，后续未采用不掩盖当前失败', () => {
  const { workspace, origin, resolver } = repairedFixture('continuity-superseded');
  const old = { ...origin, status: 'accepted', error: null }, current = resolver;
  workspace.workflowProgress![0].nodes = [old, current];
  assert.equal(workflowNodeResolution(old, [old, current], 'current'), 'continuity-superseded');
  assert.equal(selectWorkflowProgress(workspace, 'storyboard').completed, 1);
  assert.equal(storyboardCompletion(selectWorkflowProgress(workspace, 'storyboard').currentNodes), '1 批情绪规划');
  const failed = { ...origin, error: 'old branch failure' }, pending = { ...current, status: 'running' };
  assert.equal(workflowNodeResolution(failed, [failed, pending], 'current'), null);
  const author = { ...failed, stage: 'storyboardCreate' };
  assert.equal(workflowNodeResolution(author, [author, current], 'current'), null, 'a tail marker alone cannot retire an Author failure');
  workspace.workflowProgress![0].nodes = [{ ...old, resolution: undefined }, current];
  assert.equal(selectWorkflowProgress(workspace, 'storyboard').completed, 2, 'without provenance, history is not inferred from completion or titles');
});
