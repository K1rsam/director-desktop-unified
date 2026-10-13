import test from 'node:test';
import assert from 'node:assert/strict';
import { assetGenerationJob, generationBlocksNewJob, generationProgress, isLocalVideoPreparation, jobActive, videoPreparationLabel, workspacePollIntervalMs } from './generationState.ts';
import type { Job } from './types.ts';

const job = (status: string, extra: Partial<Job> = {}): Job => ({ id: status, provider: 'dreamina', kind: 'image', assetId: 'a', status, ...extra });
test('Chrome queued and ready images say not submitted instead of pretending to poll an original task', () => {
  for (const status of ['queued', 'ready', 'preparing']) {
    const pending = job(status, { provider: 'chrome', result: { raw: { progress: { message: 'old signal' } } } });
    const shown = generationProgress(pending);
    assert.match(shown, /尚未提交/);assert.doesNotMatch(shown, /old signal|正在核对.*原任务|云端正在生成/);
  }
  assert.match(generationProgress(job('queued', { provider: 'chrome', providerTaskId: 'original-task' })), /核对.*原任务/);
  assert.equal(generationProgress(job('queued')), '图片任务已排队，等待当前渠道。');
});
test('explicit local video preparation is distinguished from any submit, identity or unknown state', () => {
  const local = job('ready', { kind: 'video', snapshot: { autoSubmit: false } });
  assert.equal(isLocalVideoPreparation(local), true);
  assert.equal(videoPreparationLabel(local), '本地素材就绪 · 未生成');
  for (const status of ['queued', 'preparing', 'blocked', 'stale', 'failed', 'auth_required']) {
    assert.equal(isLocalVideoPreparation({ ...local, status }), true, status);
    assert.match(videoPreparationLabel({ ...local, status }), /未生成/);
  }
  for (const status of ['submitting', 'submission_unknown', 'submitted', 'processing', 'succeeded', 'downloading', 'downloaded', 'query_failed', 'recovery_paused']) assert.equal(isLocalVideoPreparation({ ...local, status }), false, status);
  for (const snapshot of [undefined, null, {}, [], { autoSubmit: true }, { autoSubmit: 'false' }]) assert.equal(isLocalVideoPreparation({ ...local, snapshot }), false);
  assert.equal(isLocalVideoPreparation({ ...local, providerTaskId: 'original-cloud-task' }), false);
  assert.equal(isLocalVideoPreparation({ ...local, submissionUnknown: true }), false);
  assert.equal(isLocalVideoPreparation({ ...local, kind: 'image' }), false);
});

const progressNow = Date.parse('2026-10-10T06:30:00Z');
const chromeJob = (status = 'processing', extra: Partial<Job> = {}): Job => job(status, { provider: 'chrome', createdAt: '2026-10-10T06:15:00Z', ...extra });
test('legacy failed Chrome receipt retains original ownership until recovery, without treating unsubmitted failure as accepted', () => {
  const original = chromeJob('failed', { providerTaskId: 'original-task' });
  assert.equal(generationBlocksNewJob(original), true);
  assert.equal(assetGenerationJob([original], 'a'), original);
  assert.match(generationProgress(original), /查询原任务或恢复下载.*不要重复生成/);
  assert.equal(generationBlocksNewJob(chromeJob('failed')), false);
  assert.equal(generationBlocksNewJob(job('failed', { providerTaskId: 'confirmed-failed' })), false);
});
test('Chrome progress shows original bridge phase and elapsed time without changing saved evidence', () => {
  const current = chromeJob('processing', { result: { raw: { progress: { phase: 'fetching_original', message: '网页已返回图片，正在读取原图。', checkedAt: '2026-10-10T06:29:59Z' } } } });
  const before = JSON.stringify(current), shown = generationProgress(current, progressNow);
  assert.match(shown, /^网页已返回图片，正在读取原图。 · 已等待 15 分 0 秒$/);
  assert.doesNotMatch(shown, /云端正在生成|原图已保存|暂无/);
  assert.equal(JSON.stringify(current), before);
  assert.match(generationProgress(current, progressNow + 121000), /已等待 17 分 1 秒.*仍在核对原任务，不会重复提交/);
});
test('Chrome waiting warns after two minutes without a fresh bridge signal, but never guesses completion', () => {
  const current = chromeJob();
  assert.match(generationProgress(current, progressNow), /已等待 15 分 0 秒.*暂未收到明确的新进度.*不会重复提交/);
  assert.doesNotMatch(generationProgress(current, progressNow), /云端正在生成|原图已保存|已完成/);
  assert.doesNotMatch(generationProgress(current, Date.parse('2026-10-10T06:16:59Z')), /暂未收到/);
  for (const result of [null, [], { raw: [] }, { raw: { progress: [] } }, { raw: { progress: { message: 123, checkedAt: 'invalid' } } }]) {
    assert.match(generationProgress({ ...current, result }, progressNow), /正在核对 ChatGPT 网页.*不会重复提交/);
  }
  assert.doesNotMatch(generationProgress(chromeJob('processing', { createdAt: 'invalid' }), progressNow), /已等待|NaN|暂未收到/);
  assert.match(generationProgress(chromeJob('downloading'), progressNow), /^正在下载原图/);
});
test('Chrome bridge hints cannot override unknown, failed or saved task states and other providers keep their contract', () => {
  const result = { raw: { progress: { phase: 'fetching_original', message: '正在读取原图。', checkedAt: '2026-10-10T06:29:59Z' } } };
  assert.match(generationProgress(chromeJob('processing', { result, submissionUnknown: true }), progressNow), /^上次提交未取得/);
  assert.match(generationProgress(chromeJob('submission_unknown', { result }), progressNow), /^上次提交未取得/);
  assert.equal(generationProgress(chromeJob('downloaded', { result }), progressNow), '原图已保存，可查看当前图片与历史。');
  assert.match(generationProgress(chromeJob('query_failed', { result }), progressNow), /^原任务查询失败/);
  assert.match(generationProgress(chromeJob('auth_required', { result }), progressNow), /^需要恢复/);
  assert.equal(generationProgress(job('processing', { result }), progressNow), '云端正在生成图片。');
});
test('only live Chrome image ownership speeds persisted-workspace reads, idle and unknown keep eight seconds', () => {
  assert.equal(workspacePollIntervalMs([]), 8000);
  for (const status of ['queued', 'preparing', 'submitting', 'submitted', 'processing', 'downloading', 'succeeded']) assert.equal(workspacePollIntervalMs([chromeJob(status)]), 2000, status);
  for (const status of ['downloaded', 'failed', 'query_failed', 'auth_required', 'submission_unknown', 'recovery_paused']) assert.equal(workspacePollIntervalMs([chromeJob(status)]), 8000, status);
  assert.equal(workspacePollIntervalMs([chromeJob('processing', { submissionUnknown: true })]), 8000);
  assert.equal(workspacePollIntervalMs([chromeJob('processing', { kind: 'video' }), job('processing')]), 8000);
});
test('only explicitly manual-ready videos are idle; unknown or submitted identities stay conservative', () => {
  const manual = job('ready', { kind: 'video', snapshot: { autoSubmit: false } });
  assert.equal(jobActive(manual), false);
  assert.equal(jobActive({ ...manual, providerTaskId: null }), false);
  for (const snapshot of [undefined, null, {}, { autoSubmit: null }, { autoSubmit: 'false' }, { autoSubmit: true }]) {
    assert.equal(jobActive({ ...manual, snapshot }), true, JSON.stringify(snapshot));
  }
  assert.equal(jobActive({ ...manual, providerTaskId: 'original-cloud-task' }), true, 'a saved cloud identity still needs recovery');
  assert.equal(jobActive({ ...manual, kind: 'image' }), true, 'image generation readiness is not a manual video gate');
  assert.equal(generationBlocksNewJob({ ...manual, kind: 'image' }), true, 'idle display must not release generation ownership');
  for (const status of ['queued', 'preparing', 'submitting', 'submitted', 'processing', 'downloading']) assert.equal(jobActive({ ...manual, status }), true, status);
  assert.equal(jobActive({ ...manual, status: 'processing', submissionUnknown: true }), false);
});

test('preparation, paused recovery and unknown submission retain asset generation ownership', () => {
  for (const status of ['queued', 'preparing', 'ready', 'submitting', 'submitted', 'processing', 'downloading', 'auth_required', 'submission_unknown', 'query_failed', 'download_failed', 'recovery_paused', 'waiting_dependency', 'succeeded']) assert.equal(generationBlocksNewJob(job(status)), true, status);
  for (const status of ['downloaded', 'failed', 'blocked', 'stale', 'closed_by_user']) assert.equal(generationBlocksNewJob(job(status)), false, status);
  assert.equal(generationBlocksNewJob(job('running', { kind: 'workflow' })), false);
});
test('asset status prioritizes unresolved ownership and isolates other assets', () => {
  const unknown = job('submission_unknown', { createdAt: '2026-10-03' });
  assert.equal(assetGenerationJob([job('downloaded', { createdAt: '2026-10-04' }), unknown, job('preparing', { assetId: 'b' })], 'a'), unknown);
  assert.equal(assetGenerationJob([job('failed', { createdAt: '2026-10-03' }), job('closed_by_user', { createdAt: '2026-10-04' })], 'a')?.status, 'closed_by_user');
  assert.match(generationProgress(unknown), /未取得云端回执/);
  assert.match(generationProgress(job('preparing')), /尚未进入生成提交/);
  assert.match(generationProgress(job('recovery_paused')), /暂停.*恢复原任务/);
  assert.equal(generationProgress(job('waiting_dependency')), '等待来源图片，完成后自动继续。');
  assert.equal(generationProgress(job('downloaded')), '原图已保存，可查看当前图片与历史。');
});
