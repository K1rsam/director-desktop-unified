import { useState } from 'react';
import { Button, messageOf } from './components';
import type { Media, Mutation, Workspace } from './types';

export function ImageReviewControl({ workspace, writable, mutate, onError }: { workspace: Workspace; writable: boolean; mutate: Mutation; onError: (message: string) => void }) {
  const [pending, setPending] = useState(false);
  const [acknowledgedFor, setAcknowledgedFor] = useState('');
  const capability = workspace.imageReviewCapability;
  const verified = capability?.verified === true && capability.supportsVisualInput === true;
  const delegated = workspace.settings.imageAdoptionPolicy === 'delegated-review';
  const sdkSelected = workspace.settings.textProvider === 'codex_sdk';
  const checking = capability?.status === 'checking';
  const unknown = capability?.status === 'unknown';
  const restart = capability?.explicitRestart;
  const restartBinding = restart?.available ? `${restart.expectedAttemptId}:${restart.expectedManifestSha256}` : '';
  const acknowledged = !!restartBinding && acknowledgedFor === restartBinding;
  async function execute(action: string, fields: Record<string, unknown>, notice: string) {
    if (pending || !writable) return;
    if (action === 'images.reviewCapability' && (!sdkSelected || checking || unknown)) return;
    if (action === 'images.reviewCapability.restartUnknown' && (!sdkSelected || !unknown || !acknowledged || !restart?.available)) return;
    if (action === 'images.reviewDelegation.set' && fields.enabled === true && !verified) return;
    setPending(true);
    try { await mutate(action, fields, notice); }
    catch (error) { onError(messageOf(error)); }
    finally { setPending(false); }
  }
  return <div className="asset-review-controls">
    <label className="check-label asset-review-policy"><input type="checkbox" checked={delegated} disabled={!writable || pending || (!verified && !delegated)} onChange={event => void execute('images.reviewDelegation.set', { enabled: event.target.checked }, event.target.checked ? '已为本项目开启委托审图；合格图片才会采用。' : '已关闭委托审图，新候选由你选择采用。')}/>委托审图并采用</label>
    <p className="control-hint">{delegated && !verified ? '委托暂不可执行：请先完成读图能力验证。候选和当前采用图都会保留。' : verified ? '仅对本项目生效。软件实际读取候选和必要参考图；无法判断或不合格的图片保留待处理。' : '默认由你选择采用。先验证读图能力，才能开启自动审核。'}</p>
    {!verified && <div className="actions"><Button executionAction="images.reviewCapability" disabled={!writable || pending || !sdkSelected || checking || unknown} onClick={() => void execute('images.reviewCapability', {}, '已提交读图能力验证，请查看任务进度。')}>{checking ? '正在验证读图能力…' : unknown ? '验证结果待核对' : pending ? '正在提交验证…' : '验证读图能力'}</Button><small>{unknown ? '原验证结果未知，先核对原记录，避免重复调用。' : sdkSelected ? '使用当前 SDK 模型额度，不生成图片。' : '先在渠道设置中选择可用的 SDK 文字渠道。'}</small></div>}
    {!verified && capability?.reason && <p className="control-hint">{capability.reason}</p>}
    {(unknown || checking) && <Button executionAction="images.reviewReconcile" disabled={!writable || pending} onClick={() => void execute('images.reviewReconcile', {}, '已核对原验证记录；不会重新调用模型。')}>核对原验证结果</Button>}
    {unknown && restart?.available && <div className="visual-restart-confirmation">
      <p className="control-hint">旧组件中断已修复，原验证仍保留“结果未知”。此前可能已消耗文字额度；另开验证会新增一次额度使用，不生成图片。</p>
      <label className="check-label"><input type="checkbox" checked={acknowledged} disabled={!writable || pending || !sdkSelected} onChange={event => setAcknowledgedFor(event.target.checked ? restartBinding : '')}/>我确认新增一次读图验证</label>
      <Button executionAction="images.reviewCapability.restartUnknown" disabled={!writable || pending || !sdkSelected || !acknowledged} onClick={() => void execute('images.reviewCapability.restartUnknown', { expectedAttemptId: restart.expectedAttemptId, expectedManifestSha256: restart.expectedManifestSha256, acknowledgePossiblePriorUsage: true }, '已明确提交一次新的读图验证；原未知记录完整保留。')}>另开一次读图验证</Button>
    </div>}
  </div>;
}

export function CandidateImageReview({ workspace, assetId, media, writable, mutate, onError }: { workspace: Workspace; assetId: string; media: Media; writable: boolean; mutate: Mutation; onError: (message: string) => void }) {
  const [pending, setPending] = useState(false);
  const review = media.visualReview;
  const enabled = writable && !pending && !media.placeholder && workspace.settings.textProvider === 'codex_sdk' && workspace.imageReviewCapability?.verified === true && workspace.imageReviewCapability.supportsVisualInput === true && !['reviewing', 'unknown', 'passed'].includes(review?.status || '');
  async function run() {
    if (!enabled) return;
    setPending(true);
    try { await mutate('images.reviewCandidate', { assetId, mediaId: media.id }, '已提交此候选的视觉审核；不会重新生成图片。'); }
    catch (error) { onError(messageOf(error)); }
    finally { setPending(false); }
  }
  async function reconcile() {
    if (!writable || pending) return;
    setPending(true);
    try { await mutate('images.reviewReconcile', { mediaId: media.id }, '已核对原审图记录；不会重新调用模型。'); }
    catch (error) { onError(messageOf(error)); }
    finally { setPending(false); }
  }
  const labels: Record<string, string> = { reviewing: '正在审图…', passed: '视觉审核通过', needs_repair: '审图发现问题', manual_required: '待人工核对', unknown: '审图结果待核对' };
  return <div className="candidate-image-review">
    {review?.status && <p role="status">{labels[review.status] || '待核对'}{review.adopted ? ' · 已采用' : ''}</p>}
    {review?.reason && <p className="control-hint">{review.reason}</p>}
    {review?.output && <details><summary>查看审图记录</summary><pre>{JSON.stringify(review.output, null, 2)}</pre></details>}
    <Button executionAction="images.reviewCandidate" disabled={!enabled} onClick={() => void run()}>{pending ? '正在提交…' : '审核此候选'}</Button>
    {['unknown', 'reviewing'].includes(review?.status || '') && <Button executionAction="images.reviewReconcile" disabled={!writable || pending} onClick={() => void reconcile()}>核对原审图结果</Button>}
  </div>;
}
