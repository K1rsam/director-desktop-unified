import type { Asset, AssetRef, LingmoAssetType, LingmoDiagnostic, LingmoMention, Unit } from './types.ts';

export const lingmoTypes: LingmoAssetType[] = ['CH', 'PR', 'SC', 'SE', 'OM'];
export const lingmoTypeNames: Record<LingmoAssetType, string> = { CH: '人物', PR: '道具', SC: '场景', SE: '声音', OM: '其他群演' };
export function lingmoAssetType(asset: Asset): LingmoAssetType | undefined {
  const source = asset.importSource as Record<string, unknown> | undefined;
  const raw = String(asset.lingmoType || source?.type || asset.type || asset.kind || '');
  return ({ character: 'CH', prop: 'PR', scene: 'SC', sound: 'SE', audio: 'SE', extra: 'OM' } as Record<string, LingmoAssetType>)[raw] || (lingmoTypes.includes(raw as LingmoAssetType) ? raw as LingmoAssetType : undefined);
}
export function lingmoAssetToken(asset: Asset): string | undefined {
  const source = asset.importSource as Record<string, unknown> | undefined;
  const token = asset.lingmoToken || source?.token || asset.token;
  if (typeof token === 'string' && /^@\[[^\[\]\r\n]+(?:CH|PR|SC|SE|OM)\]$/.test(token)) return token;
  const type = lingmoAssetType(asset);
  return type && asset.name && !/[\[\]\r\n]/.test(asset.name) ? `@[${asset.name}${type}]` : undefined;
}
// Byte positions match the native parser, even before a token containing Chinese or emoji.
export function lingmoMentions(text: string, assets: Asset[] = []): LingmoMention[] {
  const encoder = new TextEncoder();
  return [...text.matchAll(/@\[([^\[\]\r\n]+?)(CH|PR|SC|SE|OM)\]/g)].map(match => {
    const before = text.slice(0, match.index), token = match[0];
    const candidates = assets.filter(asset => lingmoAssetToken(asset) === token);
    const asset = candidates.length === 1 ? candidates[0] : undefined;
    const start = encoder.encode(before).length;
    return { token, name: match[1], type: match[2] as LingmoAssetType, start, end: start + encoder.encode(token).length, line: before.split('\n').length, ...(asset ? { assetId: asset.id, variantId: asset.variantId } : {}) };
  });
}
export function lingmoPromptParts(text: string, mentions: LingmoMention[]) {
  const bytes = new TextEncoder().encode(text), decoder = new TextDecoder();
  const parts: { text: string; mention?: LingmoMention }[] = []; let cursor = 0;
  for (const mention of [...mentions].sort((a, b) => a.start - b.start)) {
    if (mention.start < cursor || mention.end > bytes.length || decoder.decode(bytes.slice(mention.start, mention.end)) !== mention.token) continue;
    if (mention.start > cursor) parts.push({ text: decoder.decode(bytes.slice(cursor, mention.start)) });
    parts.push({ text: mention.token, mention }); cursor = mention.end;
  }
  if (cursor < bytes.length) parts.push({ text: decoder.decode(bytes.slice(cursor)) });
  return parts;
}
export function insertLingmoToken(text: string, start: number, end: number, token: string) {
  const from = start > 0 && text[start - 1] === '@' ? start - 1 : start;
  return { text: text.slice(0, from) + token + text.slice(end), cursor: from + token.length };
}
export function lingmoUnitDocument(unit: Unit): string {
  const source = unit.importSource as Record<string, unknown> | undefined;
  return String(source?.documentId || source?.markdownSha256 || '');
}
export function importedUnit(unit: Unit): boolean { return unit.origin === 'imported-storyboard'; }
export function lingmoImportBlocked(diagnostics: LingmoDiagnostic[]): boolean {
  return diagnostics.some(item => ['duplicate_number', 'scene_without_episode', 'scene_episode_mismatch', 'unit_without_scene'].includes(item.code));
}

// Preserve a saved identity when a later document revision adds an identically named asset.
export function boundLingmoMentions(text: string, assets: Asset[], references: AssetRef[] = [], explicit: Record<string, Pick<AssetRef, 'assetId' | 'variantId'>> = {}): LingmoMention[] {
  return lingmoMentions(text, assets).map(mention => {
    const selected = explicit[mention.token], target = selected && assets.find(asset => asset.id === selected.assetId && (asset.variantId || '') === (selected.variantId || '') && lingmoAssetToken(asset) === mention.token);
    if (target) return { ...mention, assetId: target.id, variantId: target.variantId };
    const bound = references.filter((reference, index) => references.findIndex(item => item.assetId === reference.assetId && (item.variantId || '') === (reference.variantId || '')) === index).filter(reference => {
      const asset = assets.find(item => item.id === reference.assetId && (item.variantId || '') === (reference.variantId || ''));
      return asset && lingmoAssetToken(asset) === mention.token;
    });
    return bound.length === 1 ? { ...mention, assetId: bound[0].assetId, variantId: bound[0].variantId } : mention;
  });
}
export function replaceLingmoReference(text: string, mentions: LingmoMention[], assetId: string, token: string): string {
  const bytes = new TextEncoder().encode(text), decoder = new TextDecoder();
  const matching = mentions.filter(mention => mention.assetId === assetId).sort((a, b) => a.start - b.start);
  let result = '', cursor = 0;
  for (const mention of matching) {
    if (mention.start < cursor || decoder.decode(bytes.slice(mention.start, mention.end)) !== mention.token) continue;
    result += decoder.decode(bytes.slice(cursor, mention.start)) + token; cursor = mention.end;
  }
  return result + decoder.decode(bytes.slice(cursor));
}
