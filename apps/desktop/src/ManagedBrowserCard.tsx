import { Badge, Button } from './components';

import { managedBrowserReady } from './managedBrowser';
import type { ManagedBrowserAction, ManagedBrowserStatus } from './managedBrowser';
export { managedBrowserStatus } from './managedBrowser';
export type { ManagedBrowserAction, ManagedBrowserStatus } from './managedBrowser';

const labels: Record<string, string> = {
  not_installed: '组件未就绪', not_started: '尚未启动', login_required: '需要登录',
  login_window_open: '等待登录确认', recovery_held: '恢复已暂停', stopped: '已停止', error: '连接异常',
  site_check_required: '需要网站校验', page_loading: '页面加载中', bridge_connecting: '正在连接',
  authentication_unverified: '登录状态待确认', page_unavailable: '页面暂不可读',
};

export function ManagedBrowserCard({ status, writable, busy, boundProfileId, error, onAction, onBind }: {
  status: ManagedBrowserStatus | null;
  writable: boolean;
  busy: boolean;
  boundProfileId: string;
  error: string;
  onAction: (action: ManagedBrowserAction) => void;
  onBind: (profileId: string) => void;
}) {
  const ready = managedBrowserReady(status), bound = ready && status?.profileId === boundProfileId;
  const label = ready ? '已就绪' : status ? labels[status.state] || '状态待检查' : '尚未检查';
  return <section className="surface channel-provider-card" aria-label="专属 Edge">
    <div className="section-heading compact"><h3>专属 Edge（推荐）</h3><Badge status={ready ? 'available' : status?.state === 'error' ? 'error' : 'not_configured'}>{label}</Badge></div>
    <p className="muted">首次登录 ChatGPT 后，可在后台完成图片任务。登录信息仅保存在本机专属浏览器中。</p>
    {status?.message && <p className="control-hint" role="status">{status.message}</p>}
    <div className="actions">
      <Button executionAction="chrome.managed.login" variant={status?.requiresLogin ? 'primary' : 'secondary'} disabled={!writable || busy} onClick={() => onAction('chrome.managed.login')}>官方登录</Button>
      <Button executionAction="chrome.managed.start" disabled={!writable || busy || ready} onClick={() => onAction('chrome.managed.start')}>后台启动</Button>
      <Button executionAction="chrome.managed.recover" disabled={!writable || busy} onClick={() => onAction('chrome.managed.recover')}>检查与恢复</Button>
      <Button executionAction="chrome.managed.status" icon="refresh" disabled={!writable || busy} onClick={() => onAction('chrome.managed.status')}>刷新状态</Button>
    </div>
    {busy && <p className="control-hint" role="status">正在处理专属浏览器…</p>}
    {error && <p className="warning" role="alert">{error}</p>}
    {bound ? <p className="control-hint" role="status">已用于新图片任务。</p> : ready && <div className="actions"><Button executionAction="chrome.bind" variant="primary" disabled={!writable || busy} onClick={() => { if (status?.profileId) onBind(status.profileId); }}>用于后续新图片</Button></div>}
    <p className="control-hint">后台启动和恢复不会弹出登录页。已有图片任务继续使用原浏览器记录。</p>
  </section>;
}
