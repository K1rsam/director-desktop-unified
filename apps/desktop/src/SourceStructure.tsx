import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Button, Modal } from './components';
import type { Job, Mutation, Source } from './types';

type Row = NonNullable<Source['rows']>[number];
type Diagnostic = NonNullable<Source['diagnostics']>[number];
export type SourceFragment = { id: string; code: string; message: string; first: number; last: number; rows: Row[] };
const LOCAL_SCOPE_CODES = new Set(['BOUNDARY_UNCONFIRMED', 'SCENE_BOUNDARY_UNCONFIRMED']);

export function sourceFormatNeedsAnalysis(source?: Source | null): boolean {
  return (source?.diagnostics || []).some(issue => issue.blocking && LOCAL_SCOPE_CODES.has(issue.code));
}
export function sourceManualBlockers(source?: Source | null): Diagnostic[] {
  return (source?.diagnostics || []).filter(issue => issue.blocking && !LOCAL_SCOPE_CODES.has(issue.code));
}
export function sourceAnalysisState(source: Source | null | undefined, jobs: Job[] = []) {
  const matches = jobs.filter(job => {
    if (job.kind !== 'workflow' || job.entityId !== 'source.analyze' || !source?.version) return false;
    const snapshot = job.snapshot as { sourceVersion?: number; input?: { source?: Source } } | undefined;
    const original = snapshot?.input?.source;
    const published = (job.result as { source?: Source } | undefined)?.source;
    const sameOriginal = (snapshot?.sourceVersion ?? original?.version) === source.version && (!original?.sha256 || original.sha256 === source.sha256);
    const samePublished = published?.version === source.version && published?.sha256 === source.sha256;
    return sameOriginal || samePublished;
  });
  const unknown = matches.find(job => job.submissionUnknown || job.status === 'submission_unknown');
  const pending = unknown || matches.find(job => ['queued', 'running', 'pending', 'submitting'].includes(job.status));
  const latest = pending || matches[0];
  const message = unknown ? '格式识别的提交结果尚未确认，请到任务记录核对原任务；不会重复提交。'
    : pending ? '正在后台识别原文格式，可继续查看原文；进度与回执保存在任务记录。'
    : latest?.status === 'partially_blocked' ? 'AI 已完成本轮判读，仍有部分结构待确认；请查看下方原文记录或任务记录。'
    : latest?.status === 'failed' ? '本轮格式识别未完成，原文保持不变；请查看任务记录中的具体原因。' : '';
  return { pending: !!pending, unknown: !!unknown, message };
}

// A diagnostic can name several disconnected fragments. Empty original lines may
// join a fragment; recognized headings and body must never be swallowed by it.
export function sourceFragments(source?: Source | null): SourceFragment[] {
  const rows = source?.rows || [], fragments: SourceFragment[] = [];
  for (const issue of source?.diagnostics || []) {
    if (!issue.blocking || !LOCAL_SCOPE_CODES.has(issue.code)) continue;
    const ids = new Set(issue.rowIds || []), indices = rows.flatMap((row, index) => ids.has(row.id) ? [index] : []);
    let first = -1, last = -1;
    function append() { if (first >= 0) fragments.push({ id: `${issue.code}:${rows[first].id}:${rows[last].id}`, code: issue.code, message: issue.message, first, last, rows: rows.slice(first, last + 1) }); }
    for (const index of indices) {
      if (first < 0) first = last = index;
      else if (rows.slice(last + 1, index).every(row => !row.text.trim())) last = index;
      else { append(); first = last = index; }
    }
    append();
  }
  return fragments;
}

export function sourceContextRows(source?: Source | null): Row[] {
  const rows = source?.rows || [], indices = rows.flatMap((row, index) => row.kind === 'context' ? [index] : []);
  if (!indices.length) return [];
  return rows.filter((row, index) => row.kind === 'context' || index > indices[0] && index < indices[indices.length - 1] && !row.text.trim() && !row.episodeId);
}

function originalEpisodeNumber(value?: string | number): string {
  if (typeof value === 'number') return value > 0 ? String(value) : '';
  if (!value) return '';
  if (/^\d+$/.test(value)) return Number(value) > 0 ? String(Number(value)) : '';
  const digits: Record<string, number> = { 零: 0, 〇: 0, 一: 1, 二: 2, 两: 2, 三: 3, 四: 4, 五: 5, 六: 6, 七: 7, 八: 8, 九: 9 }, units: Record<string, number> = { 十: 10, 百: 100, 千: 1000 };
  let total = 0, digit = 0;
  for (const char of value) { if (char in digits) digit = digits[char]; else if (char in units) { total += (digit || 1) * units[char]; digit = 0; } else return ''; }
  return total + digit > 0 ? String(total + digit) : '';
}

export type DuplicateEpisodeConfirmation = {
  sourceVersion: number; sourceSha256: string; parserVersion: string;
  headingRowId: string; diagnosticCode: 'DUPLICATE_EPISODE_NUMBER';
  start: number; end: number; episodeNumber: number; title: string;
};

export function duplicateEpisodeConfirmations(source: Source): DuplicateEpisodeConfirmation[] {
  if (!Number.isInteger(source.version) || Number(source.version) <= 0 || !/^[a-f0-9]{64}$/.test(source.sha256 || '') || !source.parserVersion) return [];
  return (source.diagnostics || []).flatMap(issue => {
    if (!issue.blocking || issue.code !== 'DUPLICATE_EPISODE_NUMBER' || issue.rowIds.length !== 1) return [];
    const row = source.rows?.find(row => row.id === issue.rowIds[0]);
    const episode = source.episodes?.find(episode => episode.id === row?.episodeId);
    const episodeNumber = Number(originalEpisodeNumber(episode?.sourceNumber));
    if (!row || !episode || !episodeNumber || row.range.start >= row.range.end) return [];
    return [{ sourceVersion: source.version!, sourceSha256: source.sha256!, parserVersion: source.parserVersion!, headingRowId: row.id, diagnosticCode: 'DUPLICATE_EPISODE_NUMBER' as const, start: row.range.start, end: row.range.end, episodeNumber, title: row.text }];
  });
}

export function independentEpisodeRequest(source: Source, selected: DuplicateEpisodeConfirmation) {
  const match = duplicateEpisodeConfirmations(source).find(candidate => JSON.stringify(candidate) === JSON.stringify(selected));
  if (!match) return null;
  const { sourceVersion, sourceSha256, parserVersion, headingRowId, diagnosticCode, start, end, episodeNumber } = match;
  return { expectedSourceVersion: sourceVersion, decisions: [{ start, end, kind: 'independent_episode', episodeNumber, sceneTitle: null, reason: '用户明确将当前同号标题保留为独立制作段，原文与身份保持不变。', confirmation: { sourceVersion, sourceSha256, parserVersion, headingRowId, diagnosticCode } }] };
}

function fragmentDefaults(source: Source, fragment: SourceFragment) {
  const episodeIds = [...new Set(fragment.rows.flatMap(row => row.episodeId ? [row.episodeId] : []))], sceneIds = [...new Set(fragment.rows.flatMap(row => row.sceneId ? [row.sceneId] : []))];
  const episode = episodeIds.length === 1 ? source.episodes?.find(item => item.id === episodeIds[0]) : undefined;
  const scene = sceneIds.length === 1 ? source.scenes?.find(item => item.id === sceneIds[0]) : undefined;
  return { number: originalEpisodeNumber(episode?.sourceNumber || episode?.number), sceneTitle: scene?.title || '' };
}

export type SourceReadScope = { kind: 'context' | 'episode' | 'scene'; episodeId?: string; sceneId?: string };
type Props = { source: Source; writable: boolean; mutate: Mutation; onReadScope: (scope: SourceReadScope) => void; jobs?: Job[]; analysisPending?: boolean; onAnalysisPendingChange?: (pending: boolean) => void };
export function SourceStructure({ source, writable, mutate, onReadScope, jobs = [], analysisPending = false, onAnalysisPendingChange }: Props) {
  const fragments = sourceFragments(source), otherIssues = (source.diagnostics || []).filter(issue => issue.blocking && !LOCAL_SCOPE_CODES.has(issue.code));
  const episodes = source.episodes || [], scenes = source.scenes || [], contextRows = sourceContextRows(source);
  const [expanded, setExpanded] = useState(false), [active, setActive] = useState<{ fragment: SourceFragment; version: number; sha256?: string; parserVersion?: string } | null>(null);
  const [duplicate, setDuplicate] = useState<DuplicateEpisodeConfirmation | null>(null), [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const analysis = sourceAnalysisState(source, jobs), formatBusy = analysisPending || analysis.pending;
  const duplicateOptions = duplicateEpisodeConfirmations(source);
  const [kind, setKind] = useState(''), [number, setNumber] = useState(''), [sceneTitle, setSceneTitle] = useState(''), [startLine, setStartLine] = useState(0), [endLine, setEndLine] = useState(0);
  useEffect(() => { setActive(null); setDuplicate(null); setKind(''); }, [source.version, source.sha256, source.parserVersion]);
  function open(fragment: SourceFragment) {
    const defaults = fragmentDefaults(source, fragment);
    setActive({ fragment, version: source.version || 0, sha256: source.sha256, parserVersion: source.parserVersion }); setKind(''); setNumber(defaults.number); setSceneTitle(defaults.sceneTitle);
    setStartLine(fragment.rows[0].number); setEndLine(fragment.rows[fragment.rows.length - 1].number);
  }
  const selectedRows = active?.fragment.rows.filter(row => row.number >= startLine && row.number <= endLine) || [];
  const validRange = !!active && active.fragment.rows.some(row => row.number === startLine) && active.fragment.rows.some(row => row.number === endLine) && startLine <= endLine;
  const current = !!active && active.version === (source.version || 0) && active.sha256 === source.sha256 && active.parserVersion === source.parserVersion;
  const validBody = /^[1-9]\d*$/.test(number) && !!sceneTitle.trim();
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!writable || savingRef.current || formatBusy || !current || !validRange || !['body', 'context'].includes(kind) || kind === 'body' && !validBody) return;
    const first = selectedRows[0], last = selectedRows[selectedRows.length - 1];
    const reason = kind === 'context' ? `用户将原文第 ${first.number}–${last.number} 行标记为补充资料，原文保留。` : `用户将原文第 ${first.number}–${last.number} 行归入第 ${Number(number)} 集「${sceneTitle.trim()}」，原文保留。`;
    savingRef.current = true; setSaving(true);
    try {
    const next = await mutate('source.resolve', { expectedSourceVersion: active!.version, decisions: [{ start: first.range.start, end: last.range.end, kind, episodeNumber: kind === 'body' ? Number(number) : null, sceneTitle: kind === 'body' ? sceneTitle.trim() : null, reason }] }, kind === 'context' ? '这段已作为补充资料保留，原文没有改动。' : '这段已归入指定集场，原文没有改动。');
    if (next) setActive(null);
    } finally { savingRef.current = false; setSaving(false); }
  }
  async function confirmDuplicate(event: FormEvent) {
    event.preventDefault();
    if (!writable || savingRef.current || formatBusy || !duplicate) return;
    const request = independentEpisodeRequest(source, duplicate);
    if (!request) { setDuplicate(null); return; }
    savingRef.current = true; setSaving(true);
    try { const next = await mutate('source.resolve', request, '已保留为独立制作段，原文与集场身份没有改动。'); if (next) setDuplicate(null); }
    finally { savingRef.current = false; setSaving(false); }
  }
  async function analyze() {
    if (!writable || savingRef.current || formatBusy || !source.version) return;
    savingRef.current = true; setSaving(true); onAnalysisPendingChange?.(true);
    try { await mutate('source.analyze', { expectedSourceVersion: source.version, continueFoundation: false }, '已启动后台格式识别，进度见任务记录；仅判定原文用途与集场，不改写正文。'); }
    finally { savingRef.current = false; setSaving(false); onAnalysisPendingChange?.(false); }
  }
  const pendingCount = fragments.length + otherIssues.length;
  return <div className="source-structure">
    <div className="source-structure-summary"><div><strong>已识别 {episodes.length} 集 / {scenes.length} 场</strong><span>{contextRows.length ? '背景资料已保留' : '完整原文已保留'}{pendingCount ? ` · ${pendingCount} 处待确认` : ' · 可直接整理制作基础'}</span></div><div className="actions"><Button variant="ghost" onClick={() => setExpanded(!expanded)}>{expanded ? '收起分集结构' : '查看分集结构'}</Button><Button executionAction="source.analyze" icon="spark" disabled={!writable || saving || formatBusy || !source.version} onClick={() => void analyze()}>{formatBusy ? '格式识别处理中' : 'AI识别格式'}</Button><Button executionAction="source.reparse" icon="refresh" disabled={!writable || saving || formatBusy} onClick={() => void mutate('source.reparse', { expectedSourceVersion: source.version || 0 }, '集场结构已重新识别，原文保持原样。')}>重新识别</Button></div></div>
    <p className="control-hint">常见格式先在本地识别。点击“AI识别格式”会使用当前文字渠道额度，只判定原文用途与集场，不改写原文，也不启动制作；无法确定的片段仍会保留待确认。</p>
    {analysis.message && <p role="status" className="production-inline-warning">{analysis.message}</p>}
    {expanded && <div className="source-structure-directory" aria-label="原文识别结构">{contextRows.length > 0 && <button className="source-context-link" onClick={() => onReadScope({ kind: 'context' })}><strong>背景资料</strong><span>故事简介、人物资料等 · 原文保留</span></button>}{episodes.map(episode => <article key={episode.id || episode.title}><button onClick={() => onReadScope({ kind: 'episode', episodeId: episode.id })}><strong>{episode.title || `第 ${episode.number} 集`}</strong><span>{scenes.filter(scene => scene.episodeId === episode.id).length} 场</span></button><div>{scenes.filter(scene => scene.episodeId === episode.id).map(scene => <button key={scene.id} onClick={() => onReadScope({ kind: 'scene', episodeId: episode.id, sceneId: scene.id })}>{scene.title}</button>)}</div></article>)}{!episodes.length && <p className="muted">暂未识别到分集标题；可重新识别或确认下方的局部片段。</p>}</div>}
    {fragments.length > 0 && <div className="source-fragment-list">{fragments.map(fragment => <article key={fragment.id}><div><strong>{fragment.code === 'SCENE_BOUNDARY_UNCONFIRMED' ? '这段正文缺少场景' : '这段内容的用途待确认'}</strong><span>原文第 {fragment.rows[0].number}–{fragment.rows[fragment.rows.length - 1].number} 行</span></div><p>{fragment.rows.map(row => row.text).join('').trim().slice(0, 140)}</p><Button disabled={!writable || saving || formatBusy} onClick={() => open(fragment)}>确认这段用途</Button></article>)}</div>}
    {otherIssues.map((issue: Diagnostic, index) => <div className="production-inline-warning" key={`${issue.code}:${index}`}><strong>原文仍有待处理事项</strong><p>{issue.message}</p>{issue.code === 'DUPLICATE_EPISODE_NUMBER' && duplicateOptions.filter(option => issue.rowIds.includes(option.headingRowId)).map(option => <Button key={option.headingRowId} disabled={!writable || saving || formatBusy} onClick={() => setDuplicate(option)}>保留为独立制作段</Button>)}{issue.code === 'DIALOGUE_MULTI_SPEAKER_UNCONFIRMED' && <p>这是说者分段问题，不能通过调整集场或改成资料解决。</p>}</div>)}
    {duplicate && independentEpisodeRequest(source, duplicate) && <Modal title="确认独立的同号制作段" onClose={() => setDuplicate(null)}><form onSubmit={event => void confirmDuplicate(event)}><p>仅确认下方这一处同号标题。它和前一段分别制作，保留现有集场身份与原文。</p><pre className="production-source-document">{duplicate.title}</pre><p className="muted">原文 V{duplicate.sourceVersion} · UTF-16 {duplicate.start}–{duplicate.end}</p><div className="form-actions"><Button onClick={() => setDuplicate(null)}>取消</Button><Button type="submit" variant="primary" disabled={!writable || saving || formatBusy}>保留为独立制作段</Button></div></form></Modal>}
    {active && current && <Modal title="确认这段内容的用途" onClose={() => setActive(null)}><form onSubmit={event => void save(event)}><p className="muted">仅处理下方片段，已识别的其他集场保持原样。</p><pre className="production-source-document source-fragment-preview">{selectedRows.map(row => row.text).join('')}</pre><fieldset className="source-fragment-options"><legend>这段内容是</legend><label><input type="radio" name="fragment-kind" checked={kind === 'context'} onChange={() => setKind('context')}/>补充资料</label><label><input type="radio" name="fragment-kind" checked={kind === 'body'} onChange={() => setKind('body')}/>正文剧情</label></fieldset>{kind === 'context' && <p className="muted">保留为故事与人物资料，后续整理时会读取。</p>}{kind === 'body' && <div className="form-grid"><label>所属集号<input type="number" min={1} step={1} required value={number} onChange={event => setNumber(event.target.value)} placeholder="填写原文所属集号"/></label><label>场景名称<input required value={sceneTitle} onChange={event => setSceneTitle(event.target.value)} placeholder="填写这段发生的场景"/></label></div>}<details className="production-secondary"><summary>调整片段范围</summary><p>只在当前待确认片段内选择完整原行。</p><div className="form-grid"><label>起始行<input type="number" min={active.fragment.rows[0].number} max={active.fragment.rows[active.fragment.rows.length - 1].number} value={startLine} onChange={event => setStartLine(Number(event.target.value))}/></label><label>结束行<input type="number" min={active.fragment.rows[0].number} max={active.fragment.rows[active.fragment.rows.length - 1].number} value={endLine} onChange={event => setEndLine(Number(event.target.value))}/></label></div>{!validRange && <p className="warning">请选择当前片段内的有效起止行。</p>}</details><div className="form-actions"><Button onClick={() => setActive(null)}>取消</Button><Button type="submit" variant="primary" disabled={!writable || saving || !current || !validRange || !kind || kind === 'body' && !validBody}>{kind === 'context' ? '保留为补充资料' : kind === 'body' ? '归入这集正文' : '请选择用途'}</Button></div></form></Modal>}
  </div>;
}
