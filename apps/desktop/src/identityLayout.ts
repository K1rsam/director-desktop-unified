import type { Asset, JsonObject, Media } from './types.ts';

const object = (value: unknown): JsonObject => value && typeof value === 'object' && !Array.isArray(value) ? value as JsonObject : {};
export function canMarkIdentityLayout(asset: Asset, media?: Media): boolean {
  return !!media && media.kind === 'image' && media.placeholder !== true && asset.adoptedMediaId === media.id
    && ['character', 'CH'].includes(String(asset.kind || ''))
    && typeof media.sha256 === 'string' && /^[a-f\d]{64}$/i.test(media.sha256);
}
export function identityLayoutFor(asset: Asset, media?: Media): JsonObject | null {
  if (!canMarkIdentityLayout(asset, media)) return null;
  const layout = object(asset.identityLayout), strip = object(layout.identityStrip);
  const width = Number(layout.width), height = Number(layout.height), topHeight = Number(strip.height);
  return layout.version === 'asset-identity-layout/v1' && layout.basis === 'user-inspected' && layout.confirmed === true
    && layout.mediaId === media!.id && layout.sha256 === media!.sha256
    && Number.isInteger(width) && width > 1 && width === height
    && strip.x === 0 && strip.y === 0 && strip.width === width && Number.isInteger(topHeight) && topHeight > 0 && topHeight < height ? layout : null;
}
export function requiresIdentityLayout(asset: Asset, requirement: JsonObject): boolean {
  const kind = object(asset.imagePresentation).layoutKind || object(asset.imageTask).kind;
  return kind === 'costume' && ['face', 'identity', 'identity-faces'].includes(String(requirement.role || requirement.kind || ''));
}
