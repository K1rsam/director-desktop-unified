import type { Asset, JsonObject, Workspace } from './types.ts';
import { assetImages, imageBatchBlocked } from './contracts.ts';
import { identityLayoutFor, requiresIdentityLayout } from './identityLayout.ts';
import { SERIES_WORKFLOW_PROFILE } from './workflowNavigation.ts';

const record = (value: unknown): JsonObject => value && typeof value === 'object' && !Array.isArray(value) ? value as JsonObject : {};
const text = (value: unknown) => typeof value === 'string' ? value : '';

export function imagePromptText(asset?: Asset): string {
  const prompt = record(asset?.imagePrompt);
  return typeof prompt.text === 'string' ? prompt.text : asset?.prompt || '';
}
export function imageAssetRatio(asset?: Asset): string {
  const explicit = text(record(asset?.imagePrompt).ratio) || text(asset?.imageRatio);
  if (explicit) return explicit;
  return ['character', 'ch', 'person', '人物'].includes(String(asset?.kind || asset?.type || '').toLowerCase()) ? '1:1' : '16:9';
}
export function imageAssetNeeded(asset: Asset): boolean {
  const preparation = record(asset.preparation), usage = text(asset.imagePurpose || asset.purpose).toLowerCase();
  return !['INLINE', 'AUDIO'].includes(String(preparation.mode || '')) && !['audio-only', 'voice-only', 'mention-only'].includes(usage) && asset.imageRequired !== false;
}
export function imageClassificationPending(asset: Asset): boolean { return record(asset.preparation).mode === 'UNRESOLVED'; }
export function imageIdentityPending(asset?: Asset): boolean { return record(asset?.foundationConflictReview).pending === true; }
export function realAdoptedImage(workspace: Workspace, asset: Asset) {
  const current = assetImages(workspace, asset).current;
  return current?.placeholder === true ? undefined : current;
}
export function imageReviewState(asset: Asset): 'review' | 'invalid' | undefined {
  const status = text(asset.imageStatus);
  if (['needs_review', 'stale'].includes(status)) return 'review';
  if (['needs_repair', 'rejected', 'invalid'].includes(status)) return 'invalid';
}
/** Keep the retained current image viewable, while completion and new references
 * only use an adoption whose image has not been invalidated. */
export function usableAdoptedImage(workspace: Workspace, asset: Asset) {
  const pending = ['unknown', 'submission_unknown', 'candidate', 'pending_review'].includes(text(asset.imageStatus));
  return imageReviewState(asset) || pending || imageIdentityPending(asset) ? undefined : realAdoptedImage(workspace, asset);
}
/** Recheck the retained image against its original frozen references; this is
 * never a quality approval or permission to submit another generation. */
export function imageRecheckAvailable(workspace: Workspace, asset: Asset): boolean {
  if (asset.workflowProfile !== SERIES_WORKFLOW_PROFILE && (!asset.imagePrompt || typeof asset.imagePrompt !== 'object' || Array.isArray(asset.imagePrompt))) return false;
  if (imageReviewState(asset) !== 'review' || !realAdoptedImage(workspace, asset) || imageBatchBlocked(workspace.jobs, [asset.id])) return false;
  if ([asset.designStatus, asset.promptStatus, record(asset.imagePrompt).status, record(asset.mediaPlan).status].some(value => ['needs_review', 'stale', 'rejected', 'invalid', 'blocked'].includes(text(value)))) return false;
  const reasons = Array.isArray(asset.imageReviewReasons) ? asset.imageReviewReasons.map(record) : [];
  return reasons.length > 0 && reasons.every(reason => ['reference-media-changed', 'reference-media-awaits-review'].includes(text(reason.kind)));
}
export function imageReferenceIds(asset: Asset): string[] {
  // A family/design parent is not necessarily a media dependency.
  const requirements = Array.isArray(asset.referenceRequirements) ? asset.referenceRequirements : [];
  return [...new Set(requirements.flatMap(value => {
    const ref = record(value);
    return ref.required === false ? [] : [text(ref.targetAssetId || ref.assetId)].filter(Boolean);
  }))];
}
export function imageWaitingNames(workspace: Workspace, asset: Asset): string[] {
  return imageReferenceIds(asset).flatMap(id => {
    const parent = workspace.assets.find(item => item.id === id);
    const media = parent ? usableAdoptedImage(workspace, parent) : undefined;
    if (!parent || !media) return [parent?.name || id];
    const needsLayout = asset.referenceRequirements?.some(ref => (ref.targetAssetId || ref.assetId) === id && ref.required !== false && requiresIdentityLayout(asset, ref));
    return needsLayout && !identityLayoutFor(parent, media) ? [`${parent.name}（锁脸区域待登记）`] : [];
  });
}
export function assetPromptStatus(asset: Asset): string {
  const status = text(asset.promptStatus || record(asset.imagePrompt).status);
  const labels: Record<string, string> = { draft: '待编写', pending: '待编写', writing: '编写中', generating: '编写中', ready: '提示词已编写', completed: '提示词已编写', needs_review: '提示词待复核', stale: '提示词待复核', blocked: '提示词待补料' };
  return labels[status] || (imagePromptText(asset).trim() ? '提示词已编写' : '待编写');
}
export function imagePromptNeedsReview(asset?: Asset): boolean {
  return [asset?.promptStatus, record(asset?.imagePrompt).status].some(value => ['needs_review', 'stale', 'rejected', 'invalid'].includes(text(value)));
}
export function imagePromptDesignBlocked(asset?: Asset): boolean {
  return ['needs_review', 'stale', 'rejected', 'invalid'].includes(text(asset?.designStatus));
}
export function assetImageStatus(workspace: Workspace, asset: Asset): string {
  const status = text(asset.imageStatus), images = assetImages(workspace, asset);
  const waiting = imageWaitingNames(workspace, asset), waitingLayout = waiting.some(name => name.endsWith('（锁脸区域待登记）'));
  if (imageIdentityPending(asset)) return '身份待核对';
  if (imageClassificationPending(asset)) return '类别待核对';
  if (status === 'approved' || status === 'adopted') return realAdoptedImage(workspace, asset) ? '正式采用' : images.current?.placeholder === true ? '占位待补图' : status === 'approved' ? '合格待选' : '待核对当前图';
  const labels: Record<string, string> = { waiting_reference: '等待参考', waiting_dependency: '等待参考', waitingAdoption: '等待参考', queued: '排队中', running: '生成中', submitted: '生成中', candidate: '候选待选', pending_review: '候选待选', needs_repair: '待修', rejected: '待修', invalid: '图片无效', qualified: '合格待选', needs_review: '待复核', stale: '待复核', unknown: '原结果待核对', submission_unknown: '原结果待核对' };
  if (labels[status] && !(waitingLayout && ['waiting_reference', 'waiting_dependency', 'waitingAdoption'].includes(status))) return labels[status];
  if (images.current) return images.current.placeholder === true ? '占位待补图' : '当前采用图';
  if (waitingLayout) return '锁脸区域待登记';
  if (waiting.length) return '等待参考';
  if (images.images.length) return '候选待选';
  return imageAssetNeeded(asset) ? '待生成' : '无需独立图片';
}
export function assetUseLabel(asset: Asset): string {
  const uses = Array.isArray(asset.uses) ? asset.uses.map(record) : [];
  const values = uses.map(use => [text(use.episode || use.episodeId), text(use.scene || use.sceneId), text(use.unit || use.unitId), text(use.purpose)].filter(Boolean).join(' · '));
  return [...new Set(values)].slice(0, 3).join('；') || text(asset.importName) || '用途随实际分镜绑定';
}
export function fullDesignText(asset?: Asset): string {
  const full = record(asset?.adoptedDesign).full;
  return typeof full === 'string' ? full : full && typeof full === 'object' ? JSON.stringify(full, null, 2) : '';
}
export function imageProductionCounts(workspace: Workspace) {
  const assets = imageScopeAssets(workspace);
  return { total: assets.length, prompts: assets.filter(asset => imagePromptText(asset).trim()).length, adopted: assets.filter(asset => usableAdoptedImage(workspace, asset)).length, candidates: assets.filter(asset => assetImages(workspace, asset).images.some(media => media.placeholder !== true)).length, waiting: assets.filter(asset => imageWaitingNames(workspace, asset).length).length, needsReview: assets.filter(asset => imageReviewState(asset) === 'review').length, invalid: assets.filter(asset => imageReviewState(asset) === 'invalid').length };
}
/** This is a conservative UI selection, not a frozen request. Rust still checks
 * exact media hashes, identities and the provider before anything is submitted. */
export function imageReadyForGeneration(workspace: Workspace, asset: Asset): boolean {
  if (!imageAssetNeeded(asset) || imageIdentityPending(asset) || imageClassificationPending(asset) || !imagePromptText(asset).trim() || usableAdoptedImage(workspace, asset)) return false;
  const blocked = ['blocked', 'stale', 'needs_review', 'rejected', 'invalid'];
  if ([asset.promptStatus, record(asset.imagePrompt).status, asset.designStatus, record(asset.mediaPlan).status].some(value => blocked.includes(text(value)))) return false;
  if (imageBatchBlocked(workspace.jobs, [asset.id]) || imageWaitingNames(workspace, asset).length) return false;
  const status = text(asset.imageStatus);
  if (['submission_unknown', 'unknown', 'queued', 'running', 'submitting', 'submitted', 'processing', 'downloading', 'candidate', 'qualified', 'approved'].includes(status)) return false;
  if (imageReviewState(asset)) {
    // A known media-only invalidation can be rebuilt against ready current
    // mothers. Design/identity conflicts, rejected quality and unknown reasons
    // require explicit review instead of silently buying another image.
    const reasons = Array.isArray(asset.imageReviewReasons) ? asset.imageReviewReasons.map(record) : [];
    const references = imageReferenceIds(asset);
    if (imageReviewState(asset) !== 'review' || !reasons.length || reasons.some(reason => !['reference-media-changed', 'reference-media-awaits-review'].includes(text(reason.kind)) || !references.includes(text(reason.assetId)))) return false;
  } else if (!realAdoptedImage(workspace, asset) && assetImages(workspace, asset).images.some(media => media.placeholder !== true)) {
    return false; // Returned candidates must be reviewed, not batch-resubmitted.
  }
  const visit = (current: Asset, ancestors: Set<string>): boolean => {
    if (imageIdentityPending(current)) return false;
    if (ancestors.has(current.id)) return false;
    if (current.referenceRequirements != null && !Array.isArray(current.referenceRequirements)) return false;
    const path = new Set(ancestors); path.add(current.id);
    for (const requirement of current.referenceRequirements || []) {
      if (requirement.required === false) continue;
      const id = text(requirement.targetAssetId), parent = workspace.assets.find(item => item.id === id && !item.deletedAt);
      if (!parent || requirement.targetVariantId != null && requirement.targetVariantId !== parent.variantId) return false;
      const identity = text(current.identityId || current.baseAssetId), parentIdentity = text(parent.identityId || parent.baseAssetId);
      if (['face', 'identity', 'identity-faces'].includes(text(requirement.role || requirement.kind)) && identity && parentIdentity && identity !== parentIdentity) return false;
      if (!visit(parent, path)) return false;
    }
    return true;
  };
  return visit(asset, new Set());
}
export function imageScopeAssets(workspace: Workspace): Asset[] {
  const explicit = workspace.imageProductionScope?.assetIds;
  return workspace.assets.filter(asset => imageAssetNeeded(asset) && (!explicit || explicit.includes(asset.id)));
}
/** Explicitly registered support images are authorized production, but do not
 * change the formal-delivery denominator or introduce ancestor closure. */
export function imageExecutionAssets(workspace: Workspace): Asset[] {
  const scope = workspace.imageProductionScope;
  if (!scope) return imageScopeAssets(workspace);
  const allowed = new Set([...scope.assetIds, ...(scope.referenceOnlyIds || [])]);
  return workspace.assets.filter(asset => imageAssetNeeded(asset) && allowed.has(asset.id));
}
export function sortAssetsByFirstUse(workspace: Workspace, assets: readonly Asset[]): Asset[] {
  const episodes = assetEpisodes(workspace).map(item => item.id);
  const units = workspace.units.map(item => item.id);
  const rank = (asset: Asset) => {
    const uses = Array.isArray(asset.uses) ? asset.uses.map(record) : [];
    const indexes = uses.map(use => {
      const unit = units.indexOf(text(use.unitId || use.unit));
      if (unit >= 0) return unit;
      const episode = episodes.indexOf(text(use.episodeId || use.episode));
      return episode < 0 ? Infinity : units.length + episode;
    });
    const imported = (workspace.lingmoDocuments || []).flatMap(doc => doc.scenes).findIndex(scene => scene.assetIds.includes(asset.id));
    return Math.min(...indexes, imported >= 0 ? units.length + episodes.length + imported : Infinity);
  };
  return assets.map((asset, index) => ({ asset, index, rank: rank(asset) })).sort((a, b) => a.rank - b.rank || a.index - b.index).map(item => item.asset);
}
export function assetEpisodes(workspace: Workspace) {
  const episodes = [...(workspace.source?.episodes || []), ...(workspace.lingmoDocuments || []).flatMap(doc => doc.episodes)];
  return [...new Map(episodes.filter(item => item.id).map(item => [item.id!, { id: item.id!, title: item.title || `第 ${item.number || item.id} 集` }])).values()];
}
export function assetInEpisode(workspace: Workspace, asset: Asset, episodeId: string): boolean {
  if (episodeId === 'all') return true;
  const units = workspace.units.filter(unit => unit.retired !== true && unit.episodeId === episodeId);
  // A complete modern projection is authoritative, including an explicit empty
  // image list. Spoken/mentioned characters must not become image requirements.
  if (units.length && units.every(unit => Array.isArray(unit.imageBindings))) {
    return units.some(unit => (unit.imageBindings as unknown[]).some(value => record(value).assetId === asset.id));
  }
  const uses = Array.isArray(asset.uses) ? asset.uses.map(record) : [];
  if (uses.length) return uses.some(use => use.episodeId === episodeId || use.episode === episodeId);
  const importedScenes = (workspace.lingmoDocuments || []).flatMap(doc => doc.scenes).filter(scene => scene.episodeId === episodeId);
  if (importedScenes.length) return importedScenes.some(scene => scene.assetIds.includes(asset.id));
  const ids = asset.applicability?.episodes || asset.applicability?.episodeIds || [];
  const scenes = asset.applicability?.scenes || asset.applicability?.sceneIds || [];
  return (!ids.length || ids.includes(episodeId)) && (!scenes.length || !!workspace.source?.scenes?.some(scene => scene.episodeId === episodeId && scenes.includes(scene.id)));
}
