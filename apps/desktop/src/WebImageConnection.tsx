import { useState } from 'react';
import type { ReactNode } from 'react';
import { Badge, Button, formatDate } from './components';

export function WebImageConnection({ status, busy, disabled, error, onConnect, onOpen, advanced }: {
  status: { status: string; message: string; configured?: boolean; checkedAt?: string | null; extensionPath?: string; transport?: string; errorCode?: string; browserKind?: 'chrome' | 'edge' } | null;
  busy: boolean; disabled: boolean; error: string;
  onConnect: (browser: 'edge' | 'chrome', allowAccountChange: boolean) => void; onOpen: (browser: 'edge' | 'chrome') => void; advanced: ReactNode;
}) {
  const [choice, setBrowser] = useState<'edge' | 'chrome'>();
  const [allowAccountChange, setAllowAccountChange] = useState(false);
  const browser = choice || status?.browserKind || 'chrome';
  const accountChangeRequired = /账户.*不同/.test(error) || status?.errorCode === 'ACCOUNT_CHANGED';
  const ready = !error && status?.status === 'ready';
  const label = busy ? '正在连接检查' : error ? '检查未完成' : ready ? '已连接' : status?.status === 'waiting_page' ? '待建立工作页' : status?.status === 'network_unavailable' ? '网络异常' : status?.status === 'busy' ? '连接正忙' : status ? '等待连接' : '正在读取状态';
  return <section className="surface channel-provider-card web-image-connection" aria-label="ChatGPT 网页" data-web-image-setup>
    <div className="section-heading compact"><h3>ChatGPT 网页出图</h3><Badge status={ready ? 'available' : error ? 'error' : 'draft'}>{label}</Badge></div>
    <p className="muted">使用你已登录的浏览器生成图片，完成后自动下载到项目。连接一次，所有项目共用。</p>
    <ol className="web-image-steps"><li>打开浏览器，登录 ChatGPT</li><li>点击“连接并检查”</li><li>到第三阶段选择资产，批量制作</li></ol>
    <div className={`web-image-result ${ready ? 'ready' : error ? 'error' : ''}`} role={error ? 'alert' : 'status'}>
      {busy ? '正在核对浏览器、登录状态和图片渠道，请稍候…' : error || (ready ? '已连接，可以到第三阶段生成图片。' : status?.message || '正在读取连接状态…')}
    </div>
    <div className="actions">
      <label className="web-image-browser">浏览器<select aria-label="网页出图浏览器" value={browser} disabled={disabled || busy} onChange={event => { setBrowser(event.target.value as 'edge' | 'chrome'); setAllowAccountChange(false); }}><option value="edge">Edge</option><option value="chrome">Chrome</option></select></label>
      <Button executionAction="chrome.open" data-browser-open={browser} disabled={disabled || busy} onClick={() => onOpen(browser)}>打开 ChatGPT</Button>
      <Button executionAction="chrome.connect" data-chrome-connect variant="primary" icon="refresh" disabled={disabled || busy} onClick={() => onConnect(browser, accountChangeRequired && allowAccountChange)}>{busy ? '正在检查…' : '连接并检查'}</Button>
    </div>
    {accountChangeRequired && <label><input type="checkbox" aria-label="使用所选浏览器的当前账户" checked={allowAccountChange} disabled={disabled || busy} onChange={event => setAllowAccountChange(event.target.checked)}/>使用所选浏览器的当前账户（仅影响新任务，旧任务保留原账户）</label>}
    {status?.browserKind ? <p className="control-hint">本次核对的执行浏览器：{status.browserKind === 'chrome' ? 'Chrome' : 'Edge'}。上方选择在连接成功后生效。</p> : status?.configured && <p className="control-hint">已有保存的连接，当前浏览器类型待核对。上方选择在连接成功后生效。</p>}
    <p className="control-hint">登录后保留浏览器窗口即可；连接会自动保存，无需填写地址或凭据。{status?.checkedAt ? ` 上次检查：${formatDate(status.checkedAt)}` : ''}</p>
    {!ready && status?.extensionPath && status.transport === 'opencli' && status.errorCode === 'OPENCLI_BROWSER_NOT_CONNECTED' && <details className="details"><summary>首次安装浏览器组件</summary><p>在浏览器打开 edge://extensions 或 chrome://extensions，开启开发人员模式，选择“加载解压缩的扩展”，加载下面目录。安装后刷新 ChatGPT，再点击上方“连接并检查”。</p><p className="path-value">{status.extensionPath}</p></details>}
    <details className="details web-image-advanced"><summary>高级诊断与旧版任务恢复</summary><p className="control-hint">旧版连接只用于核对历史任务；新图片使用上方的连接流程。</p>{advanced}</details>
  </section>;
}
