import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { mediaUrl } from './api';
import { actionBlockReason, currentExecutionPolicy, subscribeExecutionPolicy } from './executionPolicy';
import { assetImages } from './contracts';
import type { Asset, Media, Provider, Workspace } from './types';

export type IconName = 'home' | 'source' | 'image' | 'story' | 'video' | 'tasks' | 'settings' | 'plus' | 'arrow' | 'upload' | 'download' | 'refresh' | 'check' | 'close' | 'folder' | 'play' | 'copy' | 'spark' | 'search' | 'chevron';
const paths: Record<IconName, ReactNode> = {
  home: <><path d="m3 10 9-7 9 7v10H3z"/><path d="M9 20v-7h6v7"/></>,
  source: <><path d="M6 3h9l4 4v14H6z"/><path d="M14 3v5h5M9 12h7M9 16h7"/></>,
  image: <><rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="9" cy="9" r="2"/><path d="m3 18 6-6 4 4 3-3 5 5"/></>,
  story: <><rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 10h18M10 10v10M14 14h4M14 17h4"/></>,
  video: <><rect x="3" y="5" width="13" height="14" rx="3"/><path d="m16 10 5-3v10l-5-3"/></>,
  tasks: <><path d="M10 5h11M10 12h11M10 19h11m-18-14 2 2 3-3m-5 8 2 2 3-3m-5 8 2 2 3-3"/></>,
  settings: <><path d="m10 3 4 0 .7 3 2 .9 2.8-1 2 3.5-2.1 2.2v2.3l2.1 2.2-2 3.5-2.8-1-2 .9-.7 3h-4l-.7-3-2-.9-2.8 1-2-3.5 2.1-2.2v-2.3L2.5 8.5l2-3.5 2.8 1 2-.9z"/><circle cx="12" cy="12" r="3"/></>,
  plus: <path d="M12 5v14M5 12h14"/>, arrow: <path d="M4 12h16m-6-6 6 6-6 6"/>,
  upload: <><path d="M12 16V3m-5 5 5-5 5 5M4 15v5h16v-5"/></>,
  download: <><path d="M12 3v13m-5-5 5 5 5-5M4 16v5h16v-5"/></>,
  refresh: <><path d="M20 8a8 8 0 0 0-13-3L3 8m0-5v5h5M4 16a8 8 0 0 0 13 3l4-3m0 5v-5h-5"/></>,
  check: <path d="m5 12 4 4L19 6"/>, close: <path d="m6 6 12 12M6 18 18 6"/>,
  folder: <path d="M3 7V4h7l3 3h8v13H3z"/>, play: <path d="m9 4 12 8-12 8z"/>,
  copy: <><rect x="8" y="8" width="12" height="13" rx="2"/><path d="M15 8V3H3v13h5"/></>,
  spark: <><path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5zM20 3v4m-2-2h4"/></>,
  search: <><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/></>,
  chevron: <path d="m9 5 7 7-7 7"/>,
};
export function Icon({ name, size = 18 }: { name: IconName; size?: number }) { return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name]}</svg>; }
export function Button({ children, icon, executionAction, variant = 'secondary', ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { icon?: IconName; executionAction?: string; variant?: 'primary' | 'secondary' | 'ghost' | 'caution' }) {
  useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const blocked = executionAction ? actionBlockReason(executionAction) : null;
  return <button type="button" {...props} disabled={props.disabled || !!blocked} title={blocked || props.title} data-execution-action={executionAction} aria-describedby={blocked ? 'offline-action-reason' : props['aria-describedby']} className={`button ${variant} ${props.className || ''}`}>{icon && <Icon name={icon} />}{children}</button>;
}
export function Empty({ icon = 'folder', title, children }: { icon?: IconName; title: string; children?: ReactNode }) { return <div className="empty"><span className="empty-icon"><Icon name={icon} size={30}/></span><h3>{title}</h3>{children && <div className="muted">{children}</div>}</div>; }
export function Modal({ title, children, onClose }: { title: string; children: ReactNode; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { const previous = document.activeElement as HTMLElement | null; ref.current?.showModal(); return () => { previous?.focus(); }; }, []);
  return <dialog ref={ref} onCancel={event => { event.preventDefault(); onClose(); }} className="modal" onClick={event => { if (event.target === event.currentTarget) onClose(); }}><div className="modal-header"><h2>{title}</h2><Button variant="ghost" icon="close" aria-label="关闭" onClick={onClose}/></div><div className="modal-body">{children}</div></dialog>;
}
export function MediaPreview({ media, className = '', onInspect, compact = false }: { media?: Media; className?: string; onInspect?: (media: Media) => void; compact?: boolean }) {
  const thumbnailUrl = mediaUrl(media, compact), originalUrl = mediaUrl(media);
  const [failed, setFailed] = useState(false), [original, setOriginal] = useState(false);
  useEffect(() => { setFailed(false); setOriginal(false); }, [thumbnailUrl, originalUrl]);
  const url = original ? originalUrl : thumbnailUrl;
  function failImage() { if (compact && !original && originalUrl && thumbnailUrl !== originalUrl) setOriginal(true); else setFailed(true); }
  if (!media || !url || failed) return <div className={`media-placeholder ${className}`}><Icon name={media?.kind === 'video' ? 'video' : 'image'} size={compact ? 22 : 36}/><span>{failed ? '无法读取文件' : media ? '文件待核对' : '暂无图片'}</span></div>;
  if (media.kind === 'video') return <video controls preload="metadata" className={`media-preview ${className}`} src={mediaUrl(media)} onError={() => setFailed(true)}/>;
  if (media.kind === 'audio') return <audio controls preload="metadata" src={mediaUrl(media)} onError={() => setFailed(true)}/>;
  const image = <img loading="lazy" className={`media-preview ${className}`} alt={media.name || '资产参考图'} src={url} onError={failImage}/>;
  return onInspect ? <button type="button" className="image-open" onClick={() => onInspect(media)} aria-label="查看原图">{image}</button> : image;
}
const STATUS: Record<string, string> = {
  recovery_paused: '恢复已暂停',
  closed_by_user: '已人工关闭', draft: '草稿', ready: '准备就绪', adopted: '已采用', needs_image: '待补参考图', needs_media: '待补素材', prepared: '素材已冻结', stale: '来源已更新', blocked: '需要处理', completed: '已完成', succeeded: '生成完成', downloaded: '已下载', queued: '排队中', running: '正在生成', submitting: '正在提交', submitted: '已提交', uploading: '正在上传', pending: '等待结果', failed: '未完成', cancelled: '已取消', unknown: '提交结果未知', submission_unknown: '提交结果未知', auth_required: '需要登录', paused: '已暂停', waiting_dependencies: '等待前置结果', needs_review: '待审看', result_expired: '结果已过期', not_configured: '尚未配置', available: '可用', unavailable: '不可用', configured: '已配置', executing: '执行中', downloading: '下载中', download_failed: '下载待恢复', planned: '规划完成', preparing: '正在准备', completed_with_issues: '部分完成', processing: '处理中', waiting: '等待执行', success: '已完成', expired: '结果已过期', submission_rejected: '渠道拒绝提交', failed_before_submit: '未提交成功', waiting_dependency: '等待来源图片', query_failed: '查询待恢复', awaitingtrial: '等待试稿确认',
};
export function statusLabel(status?: string) { return status ? STATUS[status.toLowerCase()] || status : '尚未开始'; }
export function Badge({ status, children }: { status?: string; children?: ReactNode }) { const key = status?.toLowerCase() || 'draft'; return <span className={`badge ${/fail|unknown|block|expired|auth|recovery_paused/.test(key) ? 'attention' : /ready|adopt|complete|success|downloaded|available/.test(key) ? 'success' : /run|submit|upload|process/.test(key) ? 'active' : ''}`}>{children || statusLabel(status)}</span>; }
export function messageOf(value: unknown): string {
  if (!value) return '';
  if (typeof value === 'string') return value;
  if (value instanceof Error) return value.message;
  if (typeof value === 'object') { const object = value as Record<string, unknown>; return [object.message, object.error, object.reason, object.code].filter(Boolean).map(String).join(' · ') || JSON.stringify(value); }
  return String(value);
}
export function providerList(workspace: Pick<Workspace, 'providers'>): Provider[] { const stored = Array.isArray(workspace.providers) ? workspace.providers : Object.entries(workspace.providers || {}).map(([id, provider]) => ({ ...provider, id: provider.id || id })); return ['dreamina', 'bh', 'chrome'].map(id => { const value = stored.find(provider => providerId(provider) === id); return value ? { ...value, id, available: value.available ?? !!value.models?.length } : { id, name: providerName(id), available: false, configured: false, message: '保存本机配置后检查连接' }; }); }
export function providerId(provider: Provider): string { return provider.id || provider.provider || ''; }
export const PROVIDER_NAMES: Record<string, string> = { dreamina: '即梦官方 CLI', bh: 'BH API', chrome: 'ChatGPT 网页', codex: 'Codex CLI', api: '文本 API', local: '本地流程' };
export function providerName(id: string) { return PROVIDER_NAMES[id] || id; }
export function kindName(value?: string) { const names: Record<string, string> = { CH: '人物', GROUP: '群像', BEAST: '非人角色', LOC: '场景', PROP: '道具', character: '人物', scene: '场景', prop: '道具', beast: '非人角色', image: '图片', video: '视频', audio: '音频', text: '文本', workflow: '制作流程' }; return names[value || ''] || value || '资产'; }
export function adoptedImage(workspace: Workspace, asset: Asset): Media | undefined { return assetImages(workspace, asset).current; }
export { jobActive } from './generationState';
export function formatDate(value?: string): string { if (!value) return ''; const date = new Date(value); return Number.isNaN(date.valueOf()) ? value : date.toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }); }
export function costText(value: unknown): string { if (value === undefined || value === null || value === '') return '未知'; if (typeof value === 'number') return value.toFixed(3); if (typeof value === 'string') return value; if (typeof value === 'object') { const cost = value as Record<string, unknown>; return cost.amount !== undefined ? `${cost.currency || '计价单位待核对'} ${cost.amount}` : cost.total !== undefined ? `${cost.currency || '计价单位待核对'} ${cost.total}` : '未知'; } return '未知'; }
export function JsonDetails({ title, value, open = false }: { title: string; value: unknown; open?: boolean }) { return <details className="details" open={open}><summary>{title}</summary><pre className="json-document">{typeof value === 'string' ? value : JSON.stringify(value, null, 2) || '暂无记录'}</pre></details>; }
