import type { Workspace } from './types';

/** Count published episodes only. Asset count and the latest job do not prove full-script coverage. */
export function foundationCoverage(workspace: Workspace) {
  const episodes = (workspace.source?.episodes || []).filter(episode => !!episode.id);
  const allIds = [...new Set(episodes.map(episode => episode.id!))];
  const foundation = workspace.foundation;
  const current = !!workspace.source?.sha256 && foundation?.sourceSha256 === workspace.source.sha256
    && Number(workspace.source.version) > 0 && foundation?.sourceVersion === workspace.source.version;
  const published = new Set(current && Array.isArray(foundation?.episodes)
    ? foundation.episodes.flatMap(item => item && typeof item === 'object' && typeof item.episodeId === 'string' ? [item.episodeId] : []) : []);
  const completedIds = allIds.filter(id => published.has(id));
  const missingIds = allIds.filter(id => !published.has(id));
  const coveredTitles = episodes.filter(episode => completedIds.includes(episode.id!)).map(episode => episode.title || `第 ${episode.number} 集`);
  return { allIds, completedIds, missingIds, total: allIds.length, count: completedIds.length, current,
    stale: !!foundation && !current, complete: allIds.length > 0 && missingIds.length === 0,
    summary: `已整理 ${completedIds.length} / ${allIds.length} 集`,
    coveredTitles, coveredLabel: coveredTitles.slice(0, 3).join('、') + (coveredTitles.length > 3 ? `等 ${completedIds.length} 集` : ''),
  };
}

/** Reading navigation never supplies the default production scope. */
export function foundationRunOptions(workspace: Workspace, explicitEpisodeId?: string) {
  const { allIds } = foundationCoverage(workspace);
  return { selection: { episodeIds: explicitEpisodeId ? allIds.filter(id => id === explicitEpisodeId) : allIds } };
}
