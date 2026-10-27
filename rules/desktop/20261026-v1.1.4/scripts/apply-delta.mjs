#!/usr/bin/env node
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync, writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {isDeepStrictEqual, TextDecoder} from 'node:util';

export const DELTA_VERSION = 'desktop-draft-delta-v1';
export const PATCH_VERSION = 'desktop-field-patch-v1';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const identifier = value => typeof value === 'string' && value.length > 0 && value.trim() === value;
const fail = (code, message) => { throw Object.assign(new Error(message), {code}); };
const requireValue = (condition, code, message) => { if (!condition) fail(code, message); };
const has = (value, key) => Object.hasOwn(value, key);

function bytesOf(value) {
  if (typeof value === 'string' || value instanceof Uint8Array) return Buffer.from(value);
  fail('DELTA_BASE_BYTES', 'base 必须是原文件的 Buffer、Uint8Array 或原始 UTF-8 字符串，不能用对象重算文件 hash');
}

function parseJson(bytes, label) {
  try {
    const text = new TextDecoder('utf-8', {fatal: true}).decode(bytes);
    return JSON.parse(text);
  } catch {
    fail('DELTA_JSON', `${label} 必须是有效的 UTF-8 JSON`);
  }
}

function assertJson(value, ancestors = new Set()) {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return;
  if (typeof value === 'number' && Number.isFinite(value)) return;
  requireValue(record(value) || Array.isArray(value), 'DELTA_SHAPE', '差量只能包含 JSON 值');
  requireValue(!ancestors.has(value), 'DELTA_SHAPE', '差量不能包含循环引用');
  requireValue(Array.isArray(value) || [Object.prototype, null].includes(Object.getPrototypeOf(value)),
    'DELTA_SHAPE', '差量对象不能使用非 JSON 类型');
  ancestors.add(value);
  for (const child of Object.values(value)) assertJson(child, ancestors);
  ancestors.delete(value);
}

function idArray(value, code, label, nonempty = false) {
  requireValue(Array.isArray(value) && (!nonempty || value.length > 0)
    && value.every(identifier) && new Set(value).size === value.length,
  code, `${label} 必须是无重复的有效 ID 数组${nonempty ? '且不能为空' : ''}`);
  return value;
}

// This indexes structural references only. It does not validate source fidelity,
// timings, complete six-column content, assets, or whether a revision is better.
function indexUnits(units, label) {
  requireValue(Array.isArray(units) && units.length > 0, 'DELTA_UNITS', `${label}.units 不能为空`);
  const unitById = new Map(), shotOwner = new Map(), lineUnits = new Map(), allIds = new Set();
  for (const unit of units) {
    requireValue(record(unit) && identifier(unit.id) && !allIds.has(unit.id),
      'DELTA_ID', `${label} 的 unit/shot ID 必须全局唯一`);
    allIds.add(unit.id);
    unitById.set(unit.id, unit);
    requireValue(Array.isArray(unit.shots) && unit.shots.length > 0,
      'DELTA_UNITS', `${label}/${unit.id} 必须提供完整 shots 数组`);
    for (const shot of unit.shots) {
      requireValue(record(shot) && identifier(shot.id) && !allIds.has(shot.id),
        'DELTA_ID', `${label} 的 unit/shot ID 必须全局唯一`);
      allIds.add(shot.id);
      shotOwner.set(shot.id, unit.id);
      requireValue(Array.isArray(shot.dialogueParts), 'DELTA_DIALOGUE',
        `${label}/${unit.id}/${shot.id} 必须显式提供 dialogueParts 数组`);
      for (const part of shot.dialogueParts) {
        requireValue(record(part) && identifier(part.lineId), 'DELTA_DIALOGUE',
          `${label}/${unit.id}/${shot.id} 的对白片段缺少 lineId`);
        if (!lineUnits.has(part.lineId)) lineUnits.set(part.lineId, new Set());
        lineUnits.get(part.lineId).add(unit.id);
      }
    }
  }
  return {unitById, shotOwner, lineUnits, allIds};
}

function indexFantasy(value, units, label) {
  requireValue(Array.isArray(value), 'DELTA_FANTASY', `${label} 必须是完整幻想兑现数组`);
  const records = new Map(), owners = new Map();
  for (const item of value) {
    requireValue(record(item) && identifier(item.candidateId) && !records.has(item.candidateId),
      'DELTA_FANTASY_SELECTION', `${label} 的 candidateId 不能为空或重复`);
    requireValue(['realized', 'not_realized'].includes(item.status),
      'DELTA_FANTASY', `${label}/${item.candidateId} 缺少有效 status`);
    const unitIds = idArray(item.unitIds, 'DELTA_FANTASY_REF', `${label}/${item.candidateId}.unitIds`,
      item.status === 'realized');
    requireValue(unitIds.every(id => units.unitById.has(id)), 'DELTA_FANTASY_REF',
      `${label}/${item.candidateId} 引用了不存在的单元`);
    requireValue(has(item, 'shotIds') || has(item, 'screenSpans'), 'DELTA_FANTASY_REF',
      `${label}/${item.candidateId} 必须保留 shotIds 或 screenSpans 引用`);
    let shotIds = has(item, 'shotIds')
      ? idArray(item.shotIds, 'DELTA_FANTASY_REF', `${label}/${item.candidateId}.shotIds`) : null;
    if (has(item, 'screenSpans')) {
      requireValue(Array.isArray(item.screenSpans) && item.screenSpans.every(span =>
        record(span) && identifier(span.shotId) && (span.range === null
          || Array.isArray(span.range) && span.range.length === 2 && span.range.every(Number.isFinite)
          && span.range[0] >= 0 && span.range[1] > span.range[0])),
      'DELTA_FANTASY_REF', `${label}/${item.candidateId} 的 screenSpans 无效`);
      const spans = [...new Set(item.screenSpans.map(span => span.shotId))];
      requireValue(shotIds === null || shotIds.length === spans.length && spans.every(id => shotIds.includes(id)),
        'DELTA_FANTASY_REF', `${label}/${item.candidateId} 的 shotIds 与 screenSpans 不一致`);
      shotIds ??= spans;
    }
    requireValue(item.status !== 'realized' || shotIds.length > 0, 'DELTA_FANTASY_REF',
      `${label}/${item.candidateId} 的已兑现记录不能清空镜头引用`);
    requireValue(shotIds.every(id => units.shotOwner.has(id) && unitIds.includes(units.shotOwner.get(id))),
      'DELTA_FANTASY_REF', `${label}/${item.candidateId} 的镜头不存在或不属于所列单元`);
    records.set(item.candidateId, item);
    owners.set(item.candidateId, new Set([...unitIds, ...shotIds.map(id => units.shotOwner.get(id))]));
  }
  return {records, owners};
}

function linkedGroup(unitIds, replacementByOldId, code, label) {
  const touched = [...unitIds].map(id => replacementByOldId.get(id)).filter(Boolean);
  if (!touched.length) return null;
  const group = touched[0];
  requireValue([...unitIds].every(id => replacementByOldId.get(id) === group),
    code, `${label} 跨旧单元的关联范围必须纳入同一个完整连续替换组`);
  return group;
}

/**
 * Apply a desktop draft delta to the exact bytes of its base file.
 * Returns a full candidate draft, not an adopted or content-validated result.
 * Retained units and unaffected fantasy records are reused without edits.
 * The caller must still run the full bundle check/export/verify and read the work.
 */
export function applyDelta(baseBytes, delta) {
  if (delta?.version === PATCH_VERSION) return applyFieldPatch(baseBytes, delta);
  const bytes = bytesOf(baseBytes);
  assertJson(delta);
  const fields = ['version', 'baseDraftHash', 'retainedUnitIds', 'replacements', 'revisionNotes',
    'unresolved', 'fantasyRealization'];
  requireValue(record(delta) && Object.keys(delta).every(key => fields.includes(key))
    && delta.version === DELTA_VERSION, 'DELTA_SHAPE', `差量 version 必须是 ${DELTA_VERSION}，且不能含未知字段`);
  requireValue(typeof delta.baseDraftHash === 'string' && /^[a-f0-9]{64}$/i.test(delta.baseDraftHash)
    && delta.baseDraftHash.toLowerCase() === hash(bytes), 'DELTA_STALE', 'base 文件实际字节已变化，保留原稿和差量');
  requireValue(typeof delta.revisionNotes === 'string' && delta.revisionNotes.trim().length > 0
    && Array.isArray(delta.unresolved), 'DELTA_SHAPE', '必须提供非空 revisionNotes 字符串和 unresolved 数组');
  const base = parseJson(bytes, 'base');
  requireValue(record(base), 'DELTA_BASE', 'base 必须是包含 units 的完整 draft 对象');
  const old = indexUnits(base.units, 'base');
  const baseFantasy = has(base, 'fantasyRealization') ? base.fantasyRealization : [];
  const oldFantasy = indexFantasy(baseFantasy, old, 'base.fantasyRealization');
  const retained = idArray(delta.retainedUnitIds, 'DELTA_PARTITION', 'retainedUnitIds');
  requireValue(Array.isArray(delta.replacements), 'DELTA_SHAPE', 'replacements 必须是数组');
  const oldIds = [...old.unitById.keys()], claimed = new Set(), replacementByOldId = new Map(), groups = [];
  const claim = id => {
    requireValue(old.unitById.has(id) && !claimed.has(id), 'DELTA_PARTITION',
      `旧单元必须恰好保留或替换一次：${id}`);
    claimed.add(id);
  };
  for (const id of retained) claim(id);
  for (const [index, replacement] of delta.replacements.entries()) {
    requireValue(record(replacement) && Object.keys(replacement).length === 2
      && has(replacement, 'unitIds') && has(replacement, 'units'),
    'DELTA_SHAPE', `replacements[${index}] 只接受 unitIds 和完整 units`);
    const unitIds = idArray(replacement.unitIds, 'DELTA_ORDER', `replacements[${index}].unitIds`, true);
    const positions = unitIds.map(id => oldIds.indexOf(id));
    requireValue(positions.every((position, i) => position >= 0 && (i === 0 || position === positions[i - 1] + 1)),
      'DELTA_ORDER', '替换组须按 base 源序连续排列，不能跨越保留单元或倒序');
    const next = indexUnits(replacement.units, `replacements[${index}]`);
    const group = {unitIds, units: replacement.units, index: next};
    for (const id of unitIds) {
      claim(id);
      replacementByOldId.set(id, group);
    }
    for (const id of next.unitById.keys()) requireValue(!old.unitById.has(id) || unitIds.includes(id),
      'DELTA_ID', `新单元不能复用其它保留/替换组的旧 ID：${id}`);
    for (const id of next.shotOwner.keys()) requireValue(!old.shotOwner.has(id) || unitIds.includes(old.shotOwner.get(id)),
      'DELTA_ID', `新镜头不能复用其它保留/替换组的旧 ID：${id}`);
    groups.push(group);
  }
  requireValue(claimed.size === oldIds.length, 'DELTA_PARTITION', '旧单元覆盖不完整，不能静默丢弃未声明单元');

  // Always inspect the base ledger first: deleting new line/fantasy records
  // cannot evade the associations that the accepted input already established.
  for (const [lineId, ids] of old.lineUnits) linkedGroup(ids, replacementByOldId, 'DELTA_DIALOGUE_GROUP', `原句 ${lineId}`);
  const fantasyGroups = new Map();
  for (const [candidateId, ids] of oldFantasy.owners) {
    fantasyGroups.set(candidateId, linkedGroup(ids, replacementByOldId, 'DELTA_FANTASY_GROUP', `幻想 ${candidateId}`));
  }

  const units = [], emitted = new Set();
  for (const unit of base.units) {
    const group = replacementByOldId.get(unit.id);
    if (!group) units.push(unit);
    else if (!emitted.has(group)) {
      emitted.add(group);
      units.push(...group.units);
    }
  }
  const next = indexUnits(units, 'candidate');
  for (const group of groups) {
    for (const lineId of group.index.lineUnits.keys()) {
      const oldOwners = old.lineUnits.get(lineId);
      requireValue(!oldOwners || [...oldOwners].every(id => group.unitIds.includes(id)),
        'DELTA_DIALOGUE_GROUP', `原句 ${lineId} 不能由替换组移入仍保留或属于其它组的原句范围`);
    }
  }
  const affected = [...fantasyGroups.values()].some(Boolean);
  requireValue(!affected || has(delta, 'fantasyRealization'), 'DELTA_FANTASY_REQUIRED',
    '受替换影响的幻想必须提供完整新版 fantasyRealization，不能沿用旧镜号或省略账目');
  let fantasyRealization = baseFantasy;
  if (has(delta, 'fantasyRealization')) {
    const revised = indexFantasy(delta.fantasyRealization, next, 'delta.fantasyRealization');
    requireValue(revised.records.size === oldFantasy.records.size
      && [...oldFantasy.records.keys()].every(id => revised.records.has(id)),
    'DELTA_FANTASY_SELECTION', '差量工具不能新增或删除幻想 candidateId；计划和选择须另走同版采用');
    fantasyRealization = [...oldFantasy.records].map(([id, original]) => {
      const item = revised.records.get(id), group = fantasyGroups.get(id);
      if (!group) {
        requireValue(isDeepStrictEqual(item, original), 'DELTA_FANTASY_RETAINED',
          `未受影响的幻想记录必须逐值保留：${id}`);
        return original;
      }
      requireValue(original.status !== 'realized' || item.status === 'realized', 'DELTA_FANTASY_DROPPED',
        `不能通过 not_realized 撤掉已兑现幻想：${id}`);
      requireValue([...revised.owners.get(id)].every(unitId => group.index.unitById.has(unitId)),
        'DELTA_FANTASY_SCOPE', `幻想 ${id} 只能引用对应完整替换组的新单元和镜头`);
      return item;
    });
  }
  return {...base, units, fantasyRealization, revisionNotes: delta.revisionNotes, unresolved: delta.unresolved};
}

/** Small, compare-before-write edits. Paths are relative to the selected draft.
 * Source, IDs, bindings and dialogue text cannot be edited through this route.
 * Whole-unit restructuring continues to use the existing partitioned delta.
 */
export function applyFieldPatch(baseBytes, patch) {
  const bytes = bytesOf(baseBytes);
  assertJson(patch);
  const fields = ['version', 'baseHash', 'episode', 'patches', 'revisionNotes', 'unresolved'];
  requireValue(record(patch) && patch.version === PATCH_VERSION
    && Object.keys(patch).every(key => fields.includes(key)), 'PATCH_SHAPE', '字段修订合同无效或含未知字段');
  requireValue(patch.baseHash === hash(bytes), 'DELTA_STALE', 'base 文件字节已变化，保留原稿和修订');
  requireValue(Array.isArray(patch.patches) && typeof patch.revisionNotes === 'string'
    && patch.revisionNotes.trim() && Array.isArray(patch.unresolved), 'PATCH_SHAPE', '须提供 patches、实际 revisionNotes 和 unresolved');
  const base = parseJson(bytes, 'base');
  requireValue(record(base), 'DELTA_BASE', 'base 必须是完整 draft 或 bundle');
  const bundle = Array.isArray(base.episodes);
  let episodeIndex = -1;
  if (bundle) {
    const matches = base.episodes.map((episode, index) => episode.number === patch.episode ? index : -1).filter(index => index >= 0);
    requireValue(Number.isSafeInteger(patch.episode) && matches.length === 1, 'PATCH_EPISODE', 'bundle 修订须显式选择唯一集号');
    episodeIndex = matches[0];
  } else requireValue(!has(patch, 'episode'), 'PATCH_EPISODE', '单 draft 修订不接受 episode');
  const draft = bundle ? base.episodes[episodeIndex].draft : base;
  const old = indexUnits(draft?.units, 'base');
  const baseFantasy = has(draft, 'fantasyRealization') ? draft.fantasyRealization : [];
  const fantasy = indexFantasy(baseFantasy, old, 'base.fantasyRealization');
  const revised = structuredClone(draft), seen = new Set(), changed = new Set();
  const allowed = /^\/units\/(0|[1-9]\d*)\/(?:title|unitDirection|scene|lighting|initialState|exitState|shots\/(0|[1-9]\d*)\/(?:camera|picture|lighting|sound|transition|duration|dialogueParts\/(0|[1-9]\d*)\/(?:delivery|voiceStart|voiceEnd)))$/;
  for (const change of patch.patches) {
    requireValue(record(change) && Object.keys(change).length === 3
      && has(change, 'path') && has(change, 'before') && has(change, 'after'), 'PATCH_SHAPE', '每处修订只含 path/before/after');
    requireValue(typeof change.path === 'string' && allowed.test(change.path) && !seen.has(change.path),
      'PATCH_PATH', '只能修改既有演出/声窗字段；源文、ID、绑定、数组结构须走完整修订合同，路径不得重复');
    seen.add(change.path);
    const parts = change.path.slice(1).split('/');
    let original = draft, target = revised;
    for (const part of parts.slice(0, -1)) {
      requireValue(original !== null && typeof original === 'object' && has(original, part), 'PATCH_PATH', '修订路径不在基稿中');
      original = original[part]; target = target[part];
    }
    const key = parts.at(-1);
    requireValue(original !== null && typeof original === 'object' && has(original, key), 'PATCH_PATH', '不能用字段修订新增不存在的字段');
    requireValue(isDeepStrictEqual(original[key], change.before), 'PATCH_BEFORE', 'before 与基稿字段不一致');
    requireValue(['string', 'number'].includes(typeof change.before) && typeof change.after === typeof change.before
      && (typeof change.after !== 'number' || Number.isFinite(change.after)), 'PATCH_VALUE', '字段类型须保持，数值须有限');
    if (!isDeepStrictEqual(change.before, change.after)) {
      target[key] = change.after;
      changed.add(draft.units[Number(parts[1])].id);
    }
  }
  if (!changed.size) return base; // No new creative version is needed.
  // The program copies linked neighbours; the model does not have to rewrite
  // an unchanged half of a cross-unit sentence or fantasy to patch one field.
  const ids = draft.units.map(unit => unit.id), scope = new Set(changed);
  let again = true;
  while (again) {
    const before = scope.size;
    for (const group of [...old.lineUnits.values(), ...fantasy.owners.values()]) {
      if ([...group].some(id => scope.has(id))) for (const id of group) scope.add(id);
    }
    const positions = [...scope].map(id => ids.indexOf(id));
    for (let index = Math.min(...positions); index <= Math.max(...positions); index++) scope.add(ids[index]);
    again = scope.size !== before;
  }
  const innerBytes = Buffer.from(JSON.stringify(draft));
  const candidate = applyDelta(innerBytes, {version: DELTA_VERSION, baseDraftHash: hash(innerBytes),
    retainedUnitIds: ids.filter(id => !scope.has(id)),
    replacements: [{unitIds: ids.filter(id => scope.has(id)), units: revised.units.filter(unit => scope.has(unit.id))}],
    fantasyRealization: structuredClone(baseFantasy),
    revisionNotes: patch.revisionNotes, unresolved: patch.unresolved});
  if (!bundle) return candidate;
  return {...base, episodes: base.episodes.map((episode, index) => index === episodeIndex ? {...episode, draft: candidate} : episode)};
}

function main(args) {
  if (args.length === 1 && args[0] === '--help') {
    process.stdout.write('用法：node scripts/apply-delta.mjs --base draft-or-bundle.json --delta delta-or-field-patch.json --output candidate.json\n完整单元差量 desktop-draft-delta-v1；小字段修订 desktop-field-patch-v1（bundle须给episode）。输出排他创建，仅生成候选；仍须内容与导出核验。\n');
    return;
  }
  const options = new Map();
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i], value = args[i + 1];
    requireValue(['--base', '--delta', '--output'].includes(key) && !options.has(key)
      && typeof value === 'string' && value.length > 0 && !value.startsWith('--'),
    'DELTA_ARGS', '须各提供一次 --base、--delta、--output，不接受其它参数');
    options.set(key, value);
  }
  requireValue(options.size === 3, 'DELTA_ARGS', '缺少 --base、--delta 或 --output');
  const baseBytes = readFileSync(options.get('--base'));
  const delta = parseJson(readFileSync(options.get('--delta')), 'delta');
  const candidate = applyDelta(baseBytes, delta), output = resolve(options.get('--output'));
  const body = JSON.stringify(candidate, null, 2) + '\n';
  writeFileSync(output, body, {encoding: 'utf8', flag: 'wx'});
  const draft = Array.isArray(candidate.episodes) ? candidate.episodes.find(episode => episode.number === delta.episode).draft : candidate;
  process.stdout.write(JSON.stringify({ok: true, status: 'candidate', contentValidated: false, adopted: false,
    output, baseHash: hash(baseBytes), candidateHash: hash(body),
    baseDraftHash: hash(baseBytes), candidateDraftHash: hash(body), units: draft.units.length,
    unresolved: draft.unresolved?.length ?? 0, bodyUnchanged: isDeepStrictEqual(candidate, parseJson(baseBytes, 'base'))}) + '\n');
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  try { main(process.argv.slice(2)); }
  catch (error) {
    process.stderr.write(JSON.stringify({ok: false, code: error.code ?? 'DELTA_ERROR', message: error.message}) + '\n');
    process.exitCode = 1;
  }
}
