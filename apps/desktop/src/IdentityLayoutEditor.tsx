import { useEffect, useRef, useState } from 'react';
import { mediaUrl } from './api';
import { Button, messageOf, Modal } from './components';
import { canMarkIdentityLayout, identityLayoutFor } from './identityLayout';
import type { Asset, Media, Mutation } from './types';

export type IdentityLayoutEditorProps = { asset: Asset; media?: Media; writable: boolean; mutate: Mutation; onError: (error: string) => void };
export function IdentityLayoutEditor({ asset, media, writable, mutate, onError }: IdentityLayoutEditorProps) {
  const [opened, setOpened] = useState(false), [size, setSize] = useState({ width: 0, height: 0 }), [topHeight, setTopHeight] = useState(0);
  const [confirmed, setConfirmed] = useState(false), [failed, setFailed] = useState(false), [saving, setSaving] = useState(false);
  const savingRef = useRef(false), openedIdentity = useRef('');
  const identity = `${asset.id}:${asset.variantId || ''}:${media?.id || ''}:${media?.sha256 || ''}`;
  useEffect(() => { setOpened(false); setConfirmed(false); setSize({ width: 0, height: 0 }); setTopHeight(0); setFailed(false); openedIdentity.current = ''; }, [identity]);
  if (!canMarkIdentityLayout(asset, media)) return null;
  const layout = identityLayoutFor(asset, media), source = mediaUrl({ ...media!, previewUrl: undefined, thumbnailPath: undefined });
  const square = size.width > 1 && size.width === size.height;
  const valid = square && Number.isInteger(topHeight) && topHeight > 0 && topHeight < size.height && !failed;
  async function save() {
    if (!writable || !confirmed || !valid || savingRef.current || openedIdentity.current !== identity || !canMarkIdentityLayout(asset, media)) return;
    const submittedIdentity = identity;
    savingRef.current = true; setSaving(true);
    try {
      const next = await mutate('assets.identityLayout.save', { assetId: asset.id, variantId: asset.variantId || '', mediaId: media!.id, expectedSha256: media!.sha256, topHeight, confirmed: true }, '锁脸区域已登记；这不是图片质检或自动采用证明。');
      if (next && openedIdentity.current === submittedIdentity) setOpened(false);
    } catch (error) { onError(messageOf(error)); }
    finally { savingRef.current = false; setSaving(false); }
  }
  return <div className="identity-layout-entry">
    <Button executionAction="assets.identityLayout.save" disabled={!writable || !source} onClick={() => { openedIdentity.current = identity; setOpened(true); setConfirmed(false); setSize({ width: 0, height: 0 }); setTopHeight(0); setFailed(false); }}>{layout ? '调整锁脸区域' : '标记锁脸区域'}</Button>
    {layout && <small>当前采用图已登记上排区域；换图后须重新核对。</small>}
    {opened && <Modal title={`标记锁脸区域 · ${asset.name}`} onClose={() => { if (!saving) setOpened(false); }}>
      <p>移动分界线，完整保留上排三张脸，避开下排服装视图。这里仅登记复制范围，不判断图片质量。</p>
      <div className="identity-layout-preview">
        <img src={source} alt={`${asset.name}当前采用原图`} onLoad={event => {
          if (openedIdentity.current !== identity) return;
          const width = event.currentTarget.naturalWidth, height = event.currentTarget.naturalHeight;
          setSize({ width, height }); setFailed(false); setConfirmed(false);
          const strip = layout?.identityStrip as { height?: number } | undefined;
          setTopHeight(strip?.height && layout?.width === width && layout?.height === height ? strip.height : Math.max(1, Math.floor(height * 0.4)));
        }} onError={() => { if (openedIdentity.current === identity) { setFailed(true); setConfirmed(false); } }}/>
        {valid && <><div className="identity-layout-upper" style={{ height: `${topHeight / size.height * 100}%` }}><span>保留上排原像素</span></div><div className="identity-layout-lower" style={{ top: `${topHeight / size.height * 100}%` }}/></>}
      </div>
      {failed ? <p className="warning">原图读取失败，不能登记；请先检查当前图片。</p> : size.height > 0 && !square ? <p className="warning">当前原图为 {size.width}×{size.height}，不是 1:1 人物母版，不能登记锁脸区域。</p> : !size.height ? <p className="muted">正在读取原图尺寸…</p> : <>
        <div className="identity-layout-controls"><label>上排保留高度（原图像素）<input type="number" min={1} max={size.height - 1} step={1} value={topHeight} disabled={saving} onChange={event => { setTopHeight(Number(event.target.value)); setConfirmed(false); }}/></label><input aria-label="调整上排锁脸区域" type="range" min={1} max={size.height - 1} step={1} value={topHeight} disabled={saving} onChange={event => { setTopHeight(Number(event.target.value)); setConfirmed(false); }}/></div>
        <p className="muted">原图 {size.width}×{size.height} · 保留顶部 {topHeight} 像素；显示缩放不改变保存坐标。</p>
        <label className="check-label identity-layout-confirm"><input type="checkbox" checked={confirmed} disabled={!valid || saving} onChange={event => setConfirmed(event.target.checked)}/>我已查看原图，确认选区完整包含上排三脸且不含下排服装视图。</label>
      </>}
      <div className="form-actions"><Button disabled={saving} onClick={() => setOpened(false)}>取消</Button><Button executionAction="assets.identityLayout.save" variant="primary" disabled={!writable || !confirmed || !valid || saving} onClick={() => void save()}>{saving ? '正在保存…' : '确认并保存区域'}</Button></div>
    </Modal>}
  </div>;
}
