import { useRef, useState } from 'react';
import { request } from './api';
import { Badge, Button, messageOf, Modal } from './components';
import { imageScopeAssets } from './assetWorkflow';
import { imageReferenceRegistrations } from './referenceRegistrationPlan';
import type { ReferenceRegistrationPlan } from './referenceRegistrationPlan';
import type { Mutation, Workspace } from './types';

export type ImageReferenceRegistrationProps = { workspace: Workspace; writable: boolean; mutate: Mutation; onError: (error: string) => void; onNotice: (message: string) => void; onSelect: (id: string) => void };
type RegistrationReceipt = { assetId: string; addedImageCount: number; reusedAdoptedMedia?: boolean; adoptedMediaId?: string };

export function ImageReferenceRegistration({ workspace, writable, mutate, onError, onNotice, onSelect }: ImageReferenceRegistrationProps) {
  const plans = imageReferenceRegistrations(workspace);
  const [review, setReview] = useState<ReferenceRegistrationPlan | null>(null), [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const formalCount = imageScopeAssets(workspace).length, referenceCount = workspace.imageProductionScope?.referenceOnlyIds?.length || 0;
  if (!plans.length) return null;
  async function register() {
    if (!writable || !review?.asset || savingRef.current) return;
    const current = imageReferenceRegistrations(workspace).find(plan => plan.assetId === review.assetId);
    if (!current?.asset || current.registered || current.currentMediaId || current.asset.version !== review.asset.version || current.servesAssetIds.join('\0') !== review.servesAssetIds.join('\0')) {
      setReview(null); onError('参考母版或使用范围已变化，请核对当前状态后重新登记。'); return;
    }
    savingRef.current = true; setSaving(true);
    try {
      const receipt = await request<RegistrationReceipt>({ action: 'images.registerReference', projectId: workspace.project.id, assetId: review.assetId, expectedVersion: review.asset.version, servesAssetIds: review.servesAssetIds, reason: review.reason, confirmed: true });
      setReview(null);
      await mutate('workspace.get', {}, '');
      onNotice(receipt.addedImageCount === 0 ? '已核对参考母版；未增加图片生成项，也未提交生成。' : `已登记 ${receipt.addedImageCount} 张仅参考母版，尚未生成；可先编写提示词，再单独启动出图。`);
    } catch (error) { onError(messageOf(error)); }
    finally { savingRef.current = false; setSaving(false); }
  }
  return <section className="asset-reference-registration" aria-label="范围外参考母版">
    <div className="section-heading compact"><h3>需要的参考母版</h3><small>正式图片 {formalCount} 张 ＋ 已登记参考 {referenceCount} 张</small></div>
    <p className="muted">检查正式图片及已登记母版声明的直接参考。已有采用图直接复用，缺图母版逐项登记；不会自动补生祖先。</p>
    <div className="asset-reference-list">{plans.map(plan => <article key={plan.assetId}>
      <div><strong>{plan.asset?.name || plan.assetId}</strong><Badge>{!plan.asset ? '母版身份缺失' : plan.currentMediaId ? '采用图可复用' : plan.registered ? '已登记参考母版' : '范围外缺图'}</Badge></div>
      <p>{plan.reason}</p><small>服务于：{plan.servesNames.join('、')}</small>
      <div className="actions">{plan.asset && <Button variant="ghost" onClick={() => onSelect(plan.assetId)}>查看母版</Button>}{plan.asset && !plan.currentMediaId && !plan.registered && <Button executionAction="images.registerReference" disabled={!writable || saving || !Number.isInteger(plan.asset.version)} onClick={() => setReview({ ...plan, asset: { ...plan.asset! }, servesAssetIds: [...plan.servesAssetIds] })}>登记 1 张参考母版</Button>}</div>
      {!plan.asset && <p className="warning">无法按名字猜测母版，请先恢复或明确绑定此资产；其他就绪项可以继续。</p>}
    </article>)}</div>
    {review && <Modal title="登记仅参考母版" onClose={() => { if (!saving) setReview(null); }}>
      <p><strong>{review.asset?.name}</strong>仅为 {review.servesNames.join('、')} 提供参考，不作为新增分镜入画资产。</p>
      <p>{review.reason}</p><p className="reference-registration-count">正式图片 {formalCount} 张 ＋ 参考母版 {referenceCount + review.addedImageCount} 张（新增 {review.addedImageCount} 张）</p>
      <p className="muted">本次只登记用途与数量，不会生成图片。实际出图前仍需核对渠道和费用；参考图保存在内部参考库，不混入正式图片目录。</p>
      <div className="form-actions"><Button disabled={saving} onClick={() => setReview(null)}>取消</Button><Button executionAction="images.registerReference" variant="primary" disabled={!writable || saving} onClick={() => void register()}>{saving ? '正在登记…' : '确认登记，不生成'}</Button></div>
    </Modal>}
  </section>;
}
