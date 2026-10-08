import test from 'node:test';
import assert from 'node:assert/strict';
import { assetGenerationJob, generationBlocksNewJob, generationProgress, isLocalVideoPreparation, jobActive, videoPreparationLabel } from './generationState.ts';
import type { Job } from './types.ts';

const job = (status: string, extra: Partial<Job> = {}): Job => ({ id: status, provider: 'dreamina', kind: 'image', assetId: 'a', status, ...extra });
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
