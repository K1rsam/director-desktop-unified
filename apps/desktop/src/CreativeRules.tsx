import { useSyncExternalStore } from 'react';
import manifest from '../../../rules/stage3/manifest.json';
import { currentExecutionPolicy, subscribeExecutionPolicy } from './executionPolicy';
import { Badge } from './components';
import type { Rule } from './types';

export function CreativeRules({ rules }: { rules: Rule[] }) {
  const policy = useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const enabled = policy.stage3CreativePackStatus === 'verified_active' && manifest.active;
  return <section className="surface">
    <h3>创作规则与版本</h3>
    <p className={enabled ? 'muted' : 'warning'}>{enabled ? '第三阶段使用 3008 原规则与 Rust 兼容适配：情绪统筹、完整主创、统一整编；导演精修先预览再采用。' : '当前执行模式未开放第三阶段；已有稿件与历史规则保留。'}</p>
    <p className="control-hint">每次任务保存实际加载的规则快照。历史任务保留原版本；离线接线验证不代表新旧成稿质量对比已完成。</p>
    <details className="details" open><summary>第三阶段 · 本版本原件 {manifest.files.length} 份</summary><p className="muted">{manifest.adapter.profile}</p><div className="rules-table">{manifest.files.map(file => <div key={file.path}><span className="rule-stage">03</span><div><strong>{file.path.split('/').slice(-2).join('/')}</strong><small>{file.path.replace('rules/stage3/candidates/mac-3008-20261006/', '')}</small></div><Badge>3008</Badge><code title={file.sha256}>{file.sha256.slice(0, 12)}</code></div>)}</div></details>
    <details className="details"><summary>桌面通用与历史规则 · {rules.length} 份</summary><div className="rules-table">{rules.map(rule => <div key={rule.id}><span className="rule-stage">{rule.stage ? `0${rule.stage}` : '—'}</span><div><strong>{rule.title || rule.id}</strong><small>{rule.id}</small></div><Badge>v{rule.version || '1'}</Badge>{rule.sha256 && <code title={rule.sha256}>{rule.sha256.slice(0, 12)}</code>}</div>)}</div></details>
  </section>;
}
