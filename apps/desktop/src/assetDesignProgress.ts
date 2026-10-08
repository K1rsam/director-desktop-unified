import { imageDesignJob } from './contracts.ts';
import { jobActive } from './generationState.ts';
import { record, records, words } from './stageProduction.ts';
import type { Job, Workspace } from './types.ts';

function created(job: Job): number { const value = Date.parse(job.createdAt || ''); return Number.isFinite(value) ? value : 0; }
function ids(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.some(item => typeof item !== 'string' || !item.trim()) || new Set(value).size !== value.length) return null;
  return value as string[];
}
function factPacket(node: Record<string, unknown>): string {
  // A repaired packet and its stopped transport retry are one extraction batch.
  return /^assetVisualFacts:packet:([a-f0-9]{64}):repair:[a-f0-9]{64}(?::transport-retry:[a-f0-9]{64})?$/i.exec(words(node.id))?.[1] || '';
}

/** Read current-run checkpoints only. This never publishes or adopts an asset. */
export function selectAssetDesignProgress(workspace: Workspace, now = Date.now()) {
  const job = workspace.jobs.filter(item => item.kind === 'workflow' && imageDesignJob(item)).sort((left, right) => created(right) - created(left))[0];
  if (!job) return null;
  const unknown = job.submissionUnknown === true || job.status === 'submission_unknown';
  const active = jobActive(job);
  if (!active && !unknown && !['failed', 'interrupted', 'blocked', 'partially_blocked', 'auth_required'].includes(job.status)) return null;
  const progress = records(workspace.workflowProgress).find(item => item.workflowId === job.id && /^images\.(design|produce)(:|$)/.test(words(item.stage)));
  const nodes = records(progress?.nodes), designNodes = nodes.filter(node => node.stage === 'assetDesign'), factNodes = nodes.filter(node => node.stage === 'assetVisualFacts');
  const plan = record(progress?.assetDesignPlan), designIds = ids(plan.designAssetIds), inlineIds = ids(plan.inlineAssetIds);
  const designRun = record(job.snapshot).stage === 'images.design' || /^images\.design(:|$)/.test(words(job.entityId));
  const exactPlan = designRun && designIds !== null && inlineIds !== null && inlineIds.every(id => !designIds.includes(id));
  const target = exactPlan ? new Set(designIds) : null;
  const acceptedDesigns = designNodes.filter(node => node.status === 'accepted');
  const designCountAvailable = !!progress && acceptedDesigns.every(node => !!words(record(node.input).assetId));
  const acceptedAssetIds = new Set(acceptedDesigns.map(node => words(record(node.input).assetId)).filter(id => !target || target.has(id)));
  const acceptedFacts = factNodes.filter(node => node.status === 'accepted'), factCountAvailable = !!progress && acceptedFacts.every(node => !!factPacket(node));
  const acceptedPackets = new Set(acceptedFacts.map(factPacket).filter(Boolean));
  const runningFacts = factNodes.filter(node => ['running', 'queued', 'received'].includes(words(node.status))).length;
  const runningDesigns = designNodes.filter(node => ['running', 'queued', 'received'].includes(words(node.status))).length;
  const readFailed = !!workspace.readState?.error, startedAt = Date.parse(job.createdAt || '');
  const endedAt = readFailed ? Date.parse(workspace.readState?.checkedAt || '') : active || unknown ? now : Date.parse(job.updatedAt || '');
  const elapsedSeconds = Number.isFinite(startedAt) && Number.isFinite(endedAt) ? Math.max(0, Math.floor((endedAt - startedAt) / 1000)) : null;
  const designed = designCountAvailable ? acceptedAssetIds.size : null, total = exactPlan ? designIds.length : null;
  const phase = unknown ? '图片制作执行身份待核对' : !active ? '图片制作需要处理' : runningFacts ? '提取原文视觉事实' : runningDesigns ? '设计资产提示词' : total !== null && designed === total ? '核对并保存整批设计' : designed ? '设计资产提示词' : '准备图片制作';
  const designLabel = designed === null ? '正在读取本次设计进度' : total === null ? `本次已设计 ${designed} 项提示词` : `已设计 ${designed}/${total} 项提示词`;
  const inlineLabel = exactPlan && inlineIds.length ? ` · ${inlineIds.length} 项复用来源或仅作文字表现` : '';
  const factLabel = !factCountAvailable ? '原文事实进度待刷新' : runningFacts ? `原文事实：本次已提取 ${acceptedPackets.size} 批，正在提取` : acceptedPackets.size ? `原文事实：本次已提取 ${acceptedPackets.size} 批` : '原文事实：正在准备或复用有效缓存';
  return { job, active, unknown, readFailed, workflowId: job.id, designed, total, inlineCount: exactPlan ? inlineIds.length : null, factBatches: factCountAvailable ? acceptedPackets.size : null, runningFacts, runningDesigns, elapsedSeconds, phase, designLabel: designLabel + inlineLabel, factLabel };
}
