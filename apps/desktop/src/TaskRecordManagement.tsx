import { Button } from './components';
import { isLocalVideoPreparation } from './generationState';
import type { Job, Mutation } from './types';

export function taskRecordsForView(jobs: Job[], filter: string): Job[] {
  return jobs.filter(job => {
    if (filter === 'archived') return job.taskRecordArchived === true;
    if (job.taskRecordArchived === true) return false;
    if (filter === 'all') return true;
    if (filter === 'preparation') return isLocalVideoPreparation(job);
    if (filter === 'unknown') return job.submissionUnknown || /unknown/i.test(job.status);
    return job.kind === filter && (filter !== 'video' || !isLocalVideoPreparation(job));
  });
}

export function TaskRecordActions({ job, writable, mutate }: { job: Job; writable: boolean; mutate: Mutation }) {
  if (job.taskRecordArchived === true) return <Button executionAction="jobs.restore" icon="refresh" disabled={!writable}
    onClick={() => void mutate('jobs.restore', { jobId: job.id }, '记录已恢复到任务列表；原执行结果与图片保持不变。')}>恢复记录</Button>;
  // The backend independently checks the saved terminal proof. A failed UI
  // projection can never override an unknown or still-running submission.
  if (job.kind !== 'image' || job.status !== 'failed' || job.submissionUnknown) return null;
  return <Button executionAction="jobs.archive" variant="caution" disabled={!writable}
    title="从当前任务列表移除，可在已删除记录中恢复；已生成图片与执行历史保留。"
    onClick={() => void mutate('jobs.archive', { jobId: job.id }, '记录已移到已删除记录；已生成图片与执行历史保留。')}>删除记录</Button>;
}
