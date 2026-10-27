import { foundationSavedProgress } from './foundationProgress.ts';
import { record, words } from './stageProduction.ts';
import type { Job, Workspace } from './types.ts';

// UI eligibility is a hint. Rust derives and binds the actual prefix under the
// project lease; no task/result is migrated or resubmitted by this projection.
export function foundationParallelResume(workspace: Workspace, job: Job | null | undefined) {
  if (!job || job.kind !== 'workflow' || job.entityId !== 'foundation.run'
    || !['failed', 'interrupted'].includes(job.status) || job.submissionUnknown) return null;
  const error = typeof job.error === 'string' ? job.error : words(record(job.error).message);
  if (!error.includes('F2完整请求超过200k应用预算')) return null;
  const progress = foundationSavedProgress(workspace);
  if (progress.workflowId !== job.id || !progress.catalogue.count || progress.catalogue.partialIds.length
    || progress.review.batches || progress.unboundBatches || progress.catalogue.count >= (workspace.source?.episodes?.length || 0)) return null;
  return { workflowId: job.id, foundationParallelTrial: { version: 'foundation-three-lane-trial/v1', concurrency: 3 } };
}
