import { useSyncExternalStore } from 'react';
import manifest from '../../../rules/stage3/manifest.json';
import { currentExecutionPolicy, subscribeExecutionPolicy } from './executionPolicy';
import { Badge } from './components';
import type { Rule } from './types';

export function CreativeRules({ rules }: { rules: Rule[] }) {
  const policy = useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const enabled = policy.stage3CreativePackStatus === 'verified_active';
  const seriesRules = rules.filter(rule => /series|6\.7\.2|assets112|asset.*1\.1\.2/i.test(`${rule.id} ${rule.version || ''}`));
  return <section className="surface">
    <h3>创作规则与版本</h3>
    <p className={enabled ? 'muted' : 'warning'}>{enabled ? '第二阶段完成完整设计、分镜主创、整集整编和独立审稿；第三阶段编写图片提示词与管理采用图。' : '当前执行模式未开放分镜创作；已有稿件与历史规则保留。'}</p>
    <p className="control-hint">每次任务保存实际加载的规则快照。历史任务保留原版本；离线接线验证不代表新旧成稿质量对比已完成。</p>
    <details className="details" open><summary>当前新工作流规则 · {seriesRules.length} 份</summary>{seriesRules.length ? <div className="rules-table">{seriesRules.map(rule => <div key={rule.id}><strong>{rule.title || rule.id}</strong><Badge>{rule.version || '当前冻结版'}</Badge>{rule.sha256 && <code title={rule.sha256}>{rule.sha256.slice(0, 12)}</code>}</div>)}</div> : <p className="muted">规则清单以核心实际加载结果为准；尚未返回新版清单。</p>}</details>
    <details className="details"><summary>历史 3008 冻结原件 {manifest.files.length} 份</summary><p className="muted">{manifest.adapter.profile}</p><div className="rules-table">{manifest.files.map(file => <div key={file.path}><span className="rule-stage">历史</span><div><strong>{file.path.split('/').slice(-2).join('/')}</strong><small>{file.path.replace('rules/stage3/candidates/mac-3008-20261006/', '')}</small></div><Badge>3008</Badge><code title={file.sha256}>{file.sha256.slice(0, 12)}</code></div>)}</div></details>
    <details className="details"><summary>桌面通用与历史规则 · {rules.length} 份</summary><div className="rules-table">{rules.map(rule => <div key={rule.id}><span className="rule-stage">{rule.stage ? `0${rule.stage}` : '—'}</span><div><strong>{rule.title || rule.id}</strong><small>{rule.id}</small></div><Badge>v{rule.version || '1'}</Badge>{rule.sha256 && <code title={rule.sha256}>{rule.sha256.slice(0, 12)}</code>}</div>)}</div></details>
  </section>;
}
