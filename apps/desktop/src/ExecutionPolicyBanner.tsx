import { useEffect, useState } from 'react';
import { desktop, preview, request } from './api';
import { Button, messageOf } from './components';
import { OFFLINE_EXECUTION_POLICY } from './executionPolicy';
import type { ExecutionPolicy } from './executionPolicy';
import './executionPolicy.css';

export function ExecutionPolicyBanner({ globalSettings = false }: { globalSettings?: boolean }) {
  const [policy, setPolicy] = useState<ExecutionPolicy>(OFFLINE_EXECUTION_POLICY);
  const [state, setState] = useState<'checking' | 'verified' | 'failed'>(desktop ? 'checking' : 'verified');
  const [error, setError] = useState(''), [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    setState('checking'); setError('');
    void request<ExecutionPolicy>({ action: 'getExecutionPolicy' }).then(next => {
      if (active) { setPolicy(next); setState('verified'); }
    }).catch(reason => { if (active) { setPolicy(OFFLINE_EXECUTION_POLICY); setError(messageOf(reason)); setState('failed'); } });
    return () => { active = false; };
  }, [retry]);
  if (globalSettings) return state === 'failed' ? <div className="notification error" role="alert"><span>本地连接状态读取失败：{error}</span><Button icon="refresh" onClick={() => setRetry(value => value + 1)}>重新连接</Button></div> : null;
  const native = policy.mode === 'native_cli_acceptance';
  if (policy.mode === 'standard_native' || native) return <section className="execution-policy-banner" aria-label="产品执行状态" data-execution-mode={policy.mode}>
    <div className="execution-policy-heading"><strong>Director Desktop Next · 标准版</strong><span>{native ? '当前绑定有界文字验收范围' : '本机渠道与制作功能'} · {policy.stage3CreativePackStatus === 'inactive' ? '分镜创作暂不可生成' : '分镜创作已接入'}</span></div>
    <details><summary>查看功能状态</summary>
      <ul>{policy.reasons.map((reason, index) => <li key={index}>{reason}</li>)}</ul>
      {native && <p>当前文字验收固定 gpt-6.1-sol / medium，保留原文绑定、工作流预算和恢复记录。</p>}
    </details>
  </section>;
  return <section className="execution-policy-banner" aria-label="产品执行状态" data-execution-mode={policy.mode}>
    <div className="execution-policy-heading"><strong>Director Desktop Next · 离线准备模式</strong><span>真实生成关闭 · {policy.stage3CreativePackStatus === 'inactive' ? '分镜创作暂不可生成' : '保留已有分镜与创作规则'}</span></div>
    <p id="offline-action-reason">可整理本地原文、编辑资产与分镜、查看历史和导入导出。此离线执行模式的文本、图片和视频真实生成保持关闭。</p>
    <p>{policy.readiness?.enabled ? '可在渠道与设置手动检查 CLI / SDK 环境、账户和模型目录；登录、凭据变更与生成权限独立控制。' : '当前环境的手动连接检查尚未开放。'}</p>
    <details><summary>查看执行边界{desktop ? state === 'checking' ? ' · 正在核对本地核心' : state === 'verified' ? ' · 本地核心已核验' : ' · 策略核验未完成' : preview ? ' · 只读示例' : ''}</summary>
      <ul>{policy.reasons.map((reason, index) => <li key={index}>{reason}</li>)}</ul>
      <p>“准备所选素材”仍依赖渠道核验，此版本也保持关闭。浏览器示例仅供阅读。</p>
      {error && <div className="execution-policy-error" role="status"><span>{error}</span><Button icon="refresh" onClick={() => setRetry(value => value + 1)}>重新核对策略</Button></div>}
    </details>
  </section>;
}
