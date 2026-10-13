import type { Job } from './types.ts';

const ACTIVE = /^(queued|preparing|ready|running|submitted|pending|processing|uploading|submitting|executing|downloading)$/i;
const LOCAL_VIDEO_STATES = new Set(['queued', 'preparing', 'ready', 'blocked', 'stale', 'failed', 'auth_required']);
/** Only explicit local-only records may be described as not submitted. */
export function isLocalVideoPreparation(job: Job): boolean {
  const snapshot = job.snapshot;
  return job.kind === 'video' && !job.providerTaskId && job.submissionUnknown !== true
    && LOCAL_VIDEO_STATES.has(job.status.toLowerCase())
    && snapshot !== null && typeof snapshot === 'object' && !Array.isArray(snapshot)
    && (snapshot as Record<string, unknown>).autoSubmit === false;
}
export function videoPreparationLabel(job: Job): string {
  const labels: Record<string, string> = {
    queued: '等待素材准备 · 未生成', preparing: '正在准备素材 · 未生成', ready: '本地素材就绪 · 未生成',
    blocked: '素材待处理 · 未生成', stale: '素材准备已过期 · 未生成', failed: '素材准备失败 · 未生成', auth_required: '素材检查需登录 · 未生成',
  };
  return labels[job.status.toLowerCase()] || '本地素材准备 · 未生成';
}
/** Running work is separate from the ownership that prevents duplicate generation. */
export function jobActive(job: Job): boolean {
  if (job.submissionUnknown || !ACTIVE.test(job.status)) return false;
  const snapshot = job.snapshot;
  const awaitingVideoSubmission = job.kind === 'video' && job.status.toLowerCase() === 'ready'
    && !job.providerTaskId && snapshot !== null && typeof snapshot === 'object' && !Array.isArray(snapshot)
    && (snapshot as Record<string, unknown>).autoSubmit === false;
  return !awaitingVideoSubmission;
}

// These states retain ownership of a generation request, even while local work is paused.
const BLOCKING = new Set(['queued', 'preparing', 'ready', 'submitting', 'submitted', 'processing', 'downloading', 'auth_required', 'submission_unknown', 'query_failed', 'download_failed', 'recovery_paused', 'waiting_dependency', 'succeeded']);
export function generationBlocksNewJob(job: Job): boolean {
  return job.kind === 'image' && (BLOCKING.has(job.status.toLowerCase()) || job.submissionUnknown === true
    || job.provider === 'chrome' && job.status === 'failed' && !!job.providerTaskId);
}
export function assetGenerationJob(jobs: Job[], assetId: string): Job | undefined {
  const related = jobs.filter(job => job.kind === 'image' && job.assetId === assetId);
  return related.find(generationBlocksNewJob) || related.reduce<Job | undefined>((latest, job) => !latest || String(job.createdAt || '') > String(latest.createdAt || '') ? job : latest, undefined);
}
function activeChromeImage(job: Job): boolean {
  return job.provider === 'chrome' && job.kind === 'image' && !job.submissionUnknown
    && (jobActive(job) || job.status === 'succeeded');
}
/** Faster local state reads do not query or submit work to a generation channel. */
export function workspacePollIntervalMs(jobs: Job[]): number {
  return jobs.some(activeChromeImage) ? 2000 : 8000;
}
function object(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
}
function chromeProgress(job: Job, now: number): string {
  const progress = object(object(object(job.result).raw).progress);
  const message = typeof progress.message === 'string' ? progress.message.trim() : '';
  const phase = typeof progress.phase === 'string' ? progress.phase.trim() : '';
  const checked = Date.parse(typeof progress.checkedAt === 'string' ? progress.checkedAt : '');
  const started = Date.parse(job.createdAt || '');
  const elapsed = Number.isFinite(started) && Number.isFinite(now) ? Math.max(0, Math.floor((now - started) / 1000)) : null;
  const recent = !!message && !!phase && Number.isFinite(checked) && checked <= now && now - checked < 120000;
  let result = message || (job.status === 'downloading' ? '正在下载原图到项目目录。' : job.status === 'succeeded' ? '正在获取并保存原图。' : '正在核对 ChatGPT 网页中的原任务。');
  if (elapsed !== null) result += ` · 已等待 ${elapsed < 60 ? `${elapsed} 秒` : `${Math.floor(elapsed / 60)} 分 ${elapsed % 60} 秒`}`;
  if (elapsed !== null && elapsed >= 120 && !recent) result += ' · 暂未收到明确的新进度，仍在核对原任务，不会重复提交。';
  return result;
}
export function generationProgress(job: Job, now = Date.now()): string {
  if (job.submissionUnknown || job.status === 'submission_unknown') return '上次提交未取得云端回执，暂时不能再生成。到任务记录核对：找到任务 ID 后绑定原任务；确认未受理后可关闭这条记录，再明确发起新试稿。';
  if (job.provider === 'chrome' && job.kind === 'image' && job.status === 'failed' && job.providerTaskId) return '网页原任务需要恢复核对；请查询原任务或恢复下载，不要重复生成。';
  if (job.provider === 'chrome' && job.kind === 'image' && !job.providerTaskId) {
    if (job.status === 'queued') return '等待浏览器工作队列 · 尚未提交。前一任务完成并保存后自动继续；未决原任务会优先保留。';
    if (job.status === 'ready') return '图片参数已冻结，等待浏览器工作队列 · 尚未提交。';
    if (job.status === 'preparing') return '正在后台准备独立工作页与素材 · 尚未提交。';
  }
  if (activeChromeImage(job)) return chromeProgress(job, now);
  const messages: Record<string, string> = {
    recovery_paused: '自动恢复已多次失败并暂停；检查连接后，到任务记录恢复原任务，不能重新提交生成。',
    queued: '图片任务已排队，等待当前渠道。', preparing: '正在检查模型与素材，尚未进入生成提交。', ready: '参数准备完成，等待生成提交。', submitting: '正在向渠道提交，等待保存任务回执。', submitted: '云端已受理，正在查询原任务。', processing: '云端正在生成图片。', downloading: '正在下载原图到项目目录。', auth_required: '需要恢复原渠道账户，登录后继续原任务。', query_failed: '原任务查询失败，请到任务记录恢复查询。', download_failed: '图片下载失败，请到任务记录恢复下载。', waiting_dependency: '等待来源图片，完成后自动继续。', succeeded: '云端图片已完成，正在获取并保存原图。', downloaded: '原图已保存，可查看当前图片与历史。', closed_by_user: '已按你的核对声明关闭旧记录，可以明确发起新的试稿。',
  };
  return messages[job.status] || '此图片任务已结束，可在任务记录核对详情。';
}
