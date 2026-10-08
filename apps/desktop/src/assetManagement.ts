import type { Asset, Source, SourceRef } from './types.ts';

export type AssetEditValues = {
  name: string;
  kind: string;
  sourceAssetId: string;
  sourceError?: { value: string; message: string };
  episodeIds: string[];
  sceneIds: string[];
  sourceRefs: SourceRef[];
};

export type AssetUsage = {
  assetId: string;
  version: number;
  name: string;
  deletedAt?: string | null;
  units: { id: string; title?: string; episodeId?: string; sceneId?: string; version?: number }[];
  derivedAssets: { id: string; name: string; version?: number }[];
  activeJobs: { id: string; kind: string; status: string; providerTaskId?: string | null }[];
  submittedJobs: { id: string; kind: string; status: string; providerTaskId?: string | null }[];
  imageCount: number;
};

function directParentEdit(asset: Asset, assets?: Asset[]): Pick<AssetEditValues, 'sourceAssetId' | 'sourceError'> {
  const explicit = typeof asset.sourceAssetId === 'string' ? asset.sourceAssetId : '';
  const variant = typeof asset.sourceVariantId === 'string' ? asset.sourceVariantId : '';
  const reference = explicit || variant;
  if (!reference) return { sourceAssetId: '' };
  const problem = (message: string) => ({ sourceAssetId: reference, sourceError: { value: reference, message } });
  if (!assets) return explicit ? { sourceAssetId: explicit } : problem('尚未载入直接父形象目录，原来源关系已保留，请重新选择。');
  const exact = assets.filter(candidate => candidate.id === reference);
  let matches = exact;
  if (!explicit && !exact.length) {
    const variants = assets.filter(candidate => candidate.variantId === variant);
    const sameSubject = variants.filter(candidate => !!asset.baseAssetId && candidate.baseAssetId === asset.baseAssetId);
    matches = sameSubject.length ? sameSubject : variants;
  }
  if (!matches.length) return problem('原直接父形象已删除或无法定位，请重新选择；原来源关系已保留。');
  if (matches.length > 1) return problem('原直接父形象有多个匹配，请明确选择具体形象；原来源关系已保留。');
  const parent = matches[0];
  if (parent.deletedAt) return problem('原直接父形象已删除，请恢复或重新选择；原来源关系已保留。');
  if (parent.id === asset.id) return problem('直接父形象不能是当前形象，请重新选择；原来源关系已保留。');
  return { sourceAssetId: parent.id };
}

export function initialAssetEdit(asset: Asset, assets?: Asset[]): AssetEditValues {
  const storedScope = asset.scope && typeof asset.scope === 'object' ? asset.scope as { kind?: string; scenes?: { episodeId?: string; sceneId?: string }[] } : undefined;
  const places = storedScope?.kind === 'SCENES' && Array.isArray(storedScope.scenes) ? storedScope.scenes : [];
  const episodes = asset.applicability?.episodes || asset.applicability?.episodeIds || [];
  const scenes = asset.applicability?.scenes || asset.applicability?.sceneIds || [];
  return {
    name: asset.name,
    kind: asset.kind || asset.type || 'character',
    ...directParentEdit(asset, assets),
    episodeIds: [...new Set(episodes.length ? episodes : places.map(place => place.episodeId).filter((id): id is string => !!id))],
    sceneIds: [...new Set(scenes.length ? scenes : places.map(place => place.sceneId).filter((id): id is string => !!id))],
    sourceRefs: [...(asset.sourceRefs || [])],
  };
}

/** Library filtering narrows the visible list; Rust still validates bindings. */
export function assetMatchesEpisode(asset: Asset, episodeId: string, source?: Source | null): boolean {
  if (episodeId === 'all') return true;
  if (!source?.episodes?.some(episode => episode.id === episodeId)) return false;
  const episodes = asset.applicability?.episodes || asset.applicability?.episodeIds || [];
  const sceneIds = asset.applicability?.scenes || asset.applicability?.sceneIds || [];
  if (episodes.length && !episodes.includes(episodeId)) return false;
  if (sceneIds.length && !source.scenes?.some(scene => scene.episodeId === episodeId && sceneIds.includes(scene.id))) return false;
  const scope = asset.scope && typeof asset.scope === 'object' ? asset.scope as { kind?: string; scenes?: { episodeId?: string; sceneId?: string }[] } : undefined;
  if (scope?.kind === 'UNRESOLVED') return false;
  if (scope?.kind === 'SCENES') return !!scope.scenes?.some(place => place.episodeId === episodeId && source.scenes?.some(scene => scene.id === place.sceneId && scene.episodeId === episodeId));
  return true;
}

/** Editing labels and scope must never change the permanent binding identity. */
export function assetScopeProblems(values: AssetEditValues, scenes: Source['scenes'] = []): string[] {
  const problems: string[] = [];
  for (const episodeId of values.episodeIds) {
    if (!scenes.some(scene => scene.episodeId === episodeId)) problems.push('所选集数尚未识别场景，请先在第一阶段明确该集场景。');
  }
  for (const sceneId of values.sceneIds) {
    const scene = scenes.find(item => item.id === sceneId);
    if (!scene) problems.push('部分场景引用已失效，请重新选择适用场景。');
    else if (values.episodeIds.length && !values.episodeIds.includes(scene.episodeId)) problems.push('部分所选场景不属于当前集数，请重新选择适用场景。');
  }
  return [...new Set(problems)];
}

export function assetEditPayload(asset: Asset, values: AssetEditValues, assets: Asset[], scenes: Source['scenes'] = []): Asset {
  if (values.sourceAssetId && values.sourceError?.value === values.sourceAssetId) throw new Error(values.sourceError.message);
  const problems = assetScopeProblems(values, scenes);
  if (problems.length) throw new Error(problems.join(' '));
  const global = !values.episodeIds.length && !values.sceneIds.length;
  const places = (scenes || []).filter(scene => values.sceneIds.length
    ? values.sceneIds.includes(scene.id)
    : values.episodeIds.includes(scene.episodeId)).map(scene => ({ episodeId: scene.episodeId, sceneId: scene.id }));
  const source = assets.find(item => item.id === values.sourceAssetId);
  return {
    ...asset,
    expectedVersion: asset.version,
    name: values.name.trim(),
    kind: values.kind,
    sourceAssetId: values.sourceAssetId || null,
    sourceVariantId: values.sourceAssetId
      ? source?.variantId || (values.sourceAssetId === asset.sourceAssetId ? asset.sourceVariantId : null)
      : null,
    sourceRefs: [...values.sourceRefs],
    applicability: { episodes: [...new Set(places.map(place => place.episodeId))], scenes: places.map(place => place.sceneId) },
    scope: { kind: global ? 'GLOBAL' : 'SCENES', scenes: places, reason: global ? '用户在资产编辑中确认该形象全剧通用。' : '用户在资产编辑中确认该形象适用于所选集场。' },
    needsReview: false,
  };
}

export function toggleAssetScope(ids: string[], id: string, selected: boolean): string[] {
  return selected ? [...new Set([...ids, id])] : ids.filter(value => value !== id);
}

export function deletedAssetsFrom(value: unknown): Asset[] {
  if (!Array.isArray(value)) return [];
  return value.filter((asset): asset is Asset => !!asset && typeof asset === 'object'
    && typeof asset.id === 'string' && typeof asset.name === 'string' && !!asset.deletedAt);
}

export function assetDeletionReady(asset: Asset, usage: AssetUsage | null): boolean {
  return !!usage && usage.assetId === asset.id && usage.version === asset.version && !usage.deletedAt;
}
