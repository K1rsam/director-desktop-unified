import { record, words } from './stageProduction.ts';
import { selectWorkflowProgress } from './workflowProgressModel.ts';
import type { Workspace } from './types.ts';

/** Saved model checkpoints are recoverable work, not final foundation adoption. */
export function foundationSavedProgress(workspace: Workspace) {
  const { id, job, currentNodes } = selectWorkflowProgress(workspace, 'foundation');
  const snapshot = record(job?.snapshot);
  const source = record(record(snapshot.input).source);
  const version = source.version ?? snapshot.sourceVersion;
  const sha256 = words(source.sha256) || words(snapshot.sourceSha256);
  // workflow_progress intentionally omits the large source input. Current
  // job summaries retain this frozen source identity; scopes alone do not
  // establish that a same-text reimport still uses this source version.
  const currentRun = version != null && version === workspace.source?.version
    && (!sha256 || sha256 === workspace.source?.sha256);
  const allIds = [...new Set((workspace.source?.episodes || []).flatMap(episode => episode.id ? [episode.id] : []))];
  const phases = (['F2', 'Q1'] as const).map(phase => {
    const full = new Set<string>(), partial = new Set<string>(), seen = new Set<string>();
    let batches = 0, unboundBatches = 0, repairsSaved = 0, finalizationsSaved = 0;
    for (const node of currentNodes) {
      const request = record(node.request), input = record(request.input || node.input), delivery = record(input.delivery || node.delivery);
      if ((words(request.stage) || words(node.stage)) !== `foundation:${phase}` || node.status !== 'accepted') continue;
      const key = words(node.requestHash) || words(node.id);
      if (!key || seen.has(key)) continue;
      seen.add(key);
      const scope = request.scope ?? node.scope ?? input.scope ?? delivery.scope;
      const matchingSource = currentRun && (!words(input.sourceSha256) || input.sourceSha256 === workspace.source?.sha256)
        && (input.sourceVersion == null || input.sourceVersion === workspace.source?.version);
      if (!matchingSource || !Array.isArray(scope) || !scope.length || scope.some(value => typeof value !== 'string' || !allIds.includes(value))) {
        unboundBatches++; continue;
      }
      const parallel = record(input.parallelTrial);
      if (parallel.version === 'foundation-three-lane-trial/v1' && parallel.kind === 'rebase') { repairsSaved++; continue; }
      if (parallel.version === 'foundation-three-lane-trial/v1' && parallel.kind === 'final') { finalizationsSaved++; continue; }
      batches++;
      // Dynamic fragment counts are plans, not a coverage proof. Never promote
      // accepted fragments into a whole episode from count/index arithmetic.
      const fragmented = delivery.fragment != null;
      for (const episodeId of scope as string[]) (fragmented ? partial : full).add(episodeId);
    }
    const episodeIds = allIds.filter(episodeId => full.has(episodeId));
    const partialIds = allIds.filter(episodeId => partial.has(episodeId) && !full.has(episodeId));
    const label = phase === 'F2' ? '资产整理' : '整理核对';
    const scopeLabel = foundationScopeLabel(episodeIds, workspace);
    const partialLabel = foundationScopeLabel(partialIds, workspace);
    return { phase, batches, unboundBatches, repairsSaved, finalizationsSaved, episodeIds, partialIds, count: episodeIds.length, scopeLabel,
      summary: `${label}已保存 ${episodeIds.length} / ${allIds.length} 集 · ${batches} 批${partialIds.length ? `（另有 ${partialIds.length} 集的片段）` : ''}${repairsSaved ? ` · ${repairsSaved} 次冲突修订已保存` : ''}${finalizationsSaved ? ' · 统一收口已保存' : ''}`,
      recoveryLabel: batches ? `${label}：${scopeLabel}${partialIds.length ? `${scopeLabel ? '；' : ''}${partialLabel}的片段` : ''}` : '',
    };
  });
  const [catalogue, review] = phases;
  return { workflowId: id, catalogue, review, hasSaved: phases.some(phase => phase.batches > 0),
    recoveryLabel: phases.filter(phase => phase.batches > 0).map(phase => phase.recoveryLabel).join('；'),
    unboundBatches: phases.reduce((sum, phase) => sum + phase.unboundBatches, 0),
  };
}

export function foundationScopeLabel(ids: string[], workspace: Workspace): string {
  if (!ids.length) return '';
  const episodes = ids.map(id => workspace.source?.episodes?.find(episode => episode.id === id));
  if (episodes.some(episode => !episode)) return '范围待核对';
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
