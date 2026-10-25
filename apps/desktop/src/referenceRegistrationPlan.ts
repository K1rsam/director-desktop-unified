import { imageScopeAssets, realAdoptedImage } from './assetWorkflow.ts';
import type { Asset, Workspace } from './types.ts';

export type ReferenceRegistrationPlan = {
  assetId: string;
  asset?: Asset;
  servesAssetIds: string[];
  servesNames: string[];
  reason: string;
  registered: boolean;
  currentMediaId?: string;
  addedImageCount: 0 | 1;
};

/** Expose one direct registration frontier from formal items and references
 * already registered against them. Never auto-register an ancestor closure. */
export function imageReferenceRegistrations(workspace: Workspace): ReferenceRegistrationPlan[] {
  if (!workspace.imageProductionScope) return [];
  const formal = imageScopeAssets(workspace), formalIds = new Set(formal.map(asset => asset.id));
  const registeredIds = new Set(workspace.imageProductionScope.referenceOnlyIds || []);
  const consumers = [...formal, ...workspace.assets.filter(asset => registeredIds.has(asset.id) && !formalIds.has(asset.id))];
  const grouped = new Map<string, { children: Asset[]; reasons: Set<string> }>();
  for (const child of consumers) {
    for (const requirement of child.referenceRequirements || []) {
      const id = requirement.targetAssetId;
      if (requirement.required === false || typeof id !== 'string' || !id || formalIds.has(id)) continue;
      const group = grouped.get(id) || { children: [], reasons: new Set<string>() };
      if (!group.children.some(asset => asset.id === child.id)) group.children.push(child);
      const reason = typeof requirement.reason === 'string' ? requirement.reason.trim() : '';
      if (reason) group.reasons.add(reason);
      grouped.set(id, group);
    }
  }
  return [...grouped].map(([assetId, group]) => {
    const asset = workspace.assets.find(item => item.id === assetId);
    const currentMediaId = asset && !['needs_review', 'stale', 'rejected', 'invalid'].includes(String(asset.imageStatus || '')) ? realAdoptedImage(workspace, asset)?.id : undefined;
    const servesNames = group.children.map(child => child.name);
    return {
      assetId, asset, servesAssetIds: group.children.map(child => child.id), servesNames,
      reason: [...group.reasons].join('；') || `为${servesNames.join('、')}提供其已声明的直接参考图。`,
      registered: registeredIds.has(assetId),
      currentMediaId, addedImageCount: currentMediaId || registeredIds.has(assetId) ? 0 : 1,
    };
  });
}
