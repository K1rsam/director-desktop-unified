import type { AssetRef, Unit, Workspace } from './types';

export function assetMarker(reference: AssetRef): string {
  return `{{asset:${reference.assetId}:${reference.variantId || ''}}}`;
}

// Change exact identity markers only. Character names and creative text remain
// the user's responsibility; two characters sharing a name are never merged.
export function replaceUnitAsset(prompt: string, references: AssetRef[], index: number, next: AssetRef) {
  const previous = references[index];
  if (!previous) throw new Error('这处配图已变化，请重新打开单元。');
  const updated = references.map((reference, position) => position === index ? next : reference);
  const stillUsed = updated.some(reference => assetMarker(reference) === assetMarker(previous));
  return { references: updated, prompt: stillUsed ? prompt : prompt.split(assetMarker(previous)).join(assetMarker(next)) };
}

export function unitAssetEditingProblems(workspace: Workspace, unit: Unit, references: AssetRef[], prompt: string): string[] {
  const problems: string[] = [];
  if (!prompt.trim()) problems.push('提示词不能为空。');
  references.forEach((reference, index) => {
    const asset = workspace.assets.find(item => item.id === reference.assetId);
    if (!asset) { problems.push(`配图 ${index + 1} 的资产已删除或不存在，请替换或移除。`); return; }
    if (!reference.role.trim()) problems.push(`配图 ${index + 1} 请填写素材用途。`);
    if ((reference.variantId || '') !== (asset.variantId || '')) problems.push(`配图 ${index + 1} 的形象已变化，请重新选择。`);
    if (reference.mediaId && reference.mediaId !== asset.adoptedMediaId) problems.push(`配图 ${index + 1} 指定的是旧图片，请改用当前图片；历史图片可在第三阶段重新选用。`);
    const scope = asset.scope as { kind?: string; scenes?: { episodeId: string; sceneId: string }[] } | undefined;
    const episodes = asset.applicability?.episodes || asset.applicability?.episodeIds || [];
    const scenes = asset.applicability?.scenes || asset.applicability?.sceneIds || [];
    if (scope?.kind === 'SCENES' && !scope.scenes?.some(scene => scene.episodeId === unit.episodeId && scene.sceneId === unit.sceneId)
      || episodes.length > 0 && !episodes.includes(unit.episodeId || '') || scenes.length > 0 && !scenes.includes(unit.sceneId || ''))
      problems.push(`配图 ${index + 1} 的形象不适用于本集场，请调整资产范围或更换形象。`);
  });
  const markers = new Set(references.map(assetMarker));
  for (const match of prompt.matchAll(/\{\{asset:[^{}]*\}\}/g)) if (!markers.has(match[0])) problems.push('文字中还保留已移除素材的标记，请修改提示词。');
  if ([...prompt.matchAll(/@图片(\d+)/g)].some(match => Number(match[1]) > references.length)) problems.push('文字中的图片编号超出配图数量，请核对提示词。');
  return [...new Set(problems)];
}
