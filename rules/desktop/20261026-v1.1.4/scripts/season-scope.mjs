#!/usr/bin/env node
import {createHash} from 'node:crypto';
import {readFile, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const VERSION = 'series-season-scope-v1';
const TARGET = {minSeconds: 7200, maxSeconds: 9000, planningSeconds: 8100};
const TICKS = 1e9;
const own = (value, key) => Object.hasOwn(value, key);
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const text = value => typeof value === 'string' && value.trim().length > 0 && !value.includes('\0');
const label = value => text(value) && !/[\r\n]/.test(value);
const sha = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const positiveInteger = value => Number.isSafeInteger(value) && value > 0;
const need = (ok, code, message) => { if (!ok) throw Object.assign(new Error(message), {code}); };
const equal = (left, right) => canonical(left) === canonical(right);

// Canonical JSON sorts object keys; array order remains the original source order.
function canonical(value, seen = new Set()) {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return JSON.stringify(value);
  if (typeof value === 'number') {
    need(Number.isFinite(value), 'SEASON_JSON', '范围哈希只接受有限JSON数值');
    return JSON.stringify(value);
  }
  need(object(value) || Array.isArray(value), 'SEASON_JSON', '范围哈希只接受JSON数据，不能含undefined或函数');
  need(!seen.has(value), 'SEASON_JSON', '范围JSON不能有循环引用');
  need(Array.isArray(value) || [Object.prototype, null].includes(Object.getPrototypeOf(value)), 'SEASON_JSON', '范围哈希只接受普通JSON对象');
  seen.add(value);
  let result;
  if (Array.isArray(value)) {
    need(Object.keys(value).length === value.length, 'SEASON_JSON', '范围JSON数组不能有空位或附加属性');
    result = '[' + value.map(item => canonical(item, seen)).join(',') + ']';
  } else result = '{' + Object.keys(value).sort().map(key => JSON.stringify(key) + ':' + canonical(value[key], seen)).join(',') + '}';
  seen.delete(value);
  return result;
}

export function hashSeasonScope(scope) {
  return createHash('sha256').update(canonical(scope), 'utf8').digest('hex');
}

function catalog(value, code, name, allowEmpty = false) {
  need(Array.isArray(value) && (allowEmpty || value.length > 0), code, `${name}须包含完整集场目录`);
  const numbers = new Set();
  for (const episode of value) {
    need(object(episode) && Number.isSafeInteger(episode.number) && !numbers.has(episode.number), code, `${name}集号须为源序唯一整数`);
    numbers.add(episode.number);
    need(Array.isArray(episode.sceneIds) && episode.sceneIds.length > 0 && episode.sceneIds.every(label) && new Set(episode.sceneIds).size === episode.sceneIds.length, code, `${name}场ID须为本集唯一非空单行标签`);
  }
  return value.map(({number, sceneIds}) => ({number, sceneIds}));
}

function confirmation(value, code) {
  need(object(value) && ['user', 'delegated'].includes(value.kind) && text(value.basis), code, '须记录user或delegated确认种类及具体依据；字段本身不证明批准真实性');
}

export function validateSeasonScope(scope, {sourceHash, index, requireLocked = true} = {}) {
  need(object(scope) && scope.version === VERSION, 'SEASON_SCOPE', `productionScope须为${VERSION}`);
  need(label(scope.id) && positiveInteger(scope.revision) && positiveInteger(scope.seasonNumber), 'SEASON_SCOPE', '范围id、正整数revision/seasonNumber无效');
  need(['proposed', 'locked'].includes(scope.status), 'SEASON_SCOPE', '范围status须为proposed或locked');
  need(!requireLocked || scope.status === 'locked', 'SEASON_NOT_LOCKED', '制作范围尚未locked，不能派发或生成投影索引');
  need(sha(scope.sourceHash) && (sourceHash === undefined || sourceHash === scope.sourceHash), 'SEASON_SOURCE_HASH', '首季范围与原始剧本sourceHash不一致');
  const reading = catalog(scope.readingScope, 'SEASON_READING_SCOPE', '全剧readingScope');
  const selected = catalog(scope.episodes, 'SEASON_EPISODES', '制作episodes');
  const start = reading.findIndex(episode => episode.number === selected[0].number);
  need(start >= 0 && start + selected.length <= reading.length, 'SEASON_CONTINUITY', '制作范围须为全剧源序内的连续整集');
  need(selected.every((episode, offset) => episode.number === reading[start + offset].number), 'SEASON_CONTINUITY', '制作范围不能跳过中段或改变源顺序');
  need(selected.every((episode, offset) => equal(episode.sceneIds, reading[start + offset].sceneIds)), 'SEASON_WHOLE_EPISODE', '首版只支持整集尾，不能删场、重排场或制作半集');
  if (scope.seasonNumber === 1) need(start === 0, 'SEASON_CONTINUITY', '首季必须从全剧readingScope第一集开始');
  else {
    const continuation = scope.continuation;
    need(object(continuation) && sha(continuation.previousScopeHash) && label(continuation.previousSeasonId) && continuation.previousSeasonId !== scope.id && continuation.nextEpisode === selected[0].number, 'SEASON_CONTINUATION', '续季须有前季hash/id，且从continuation.nextEpisode承接');
  }
  need(object(scope.target) && Object.entries(TARGET).every(([key, value]) => scope.target[key] === value), 'SEASON_TARGET', '批准目标须保留120–150分钟与135分钟规划中心，不能改字段绕过');
  const last = selected.at(-1), ending = scope.ending;
  need(object(ending) && ending.episode === last.number && ending.sceneId === last.sceneIds.at(-1) && text(ending.sourceRef) && text(ending.closure) && text(ending.hook), 'SEASON_ENDING', '季终须记录所选最后一集最后场的源依据、闭合与原生钩子');
  need(object(ending.estimate) && Number.isFinite(ending.estimate.minSeconds) && ending.estimate.minSeconds > 0 && Number.isFinite(ending.estimate.maxSeconds) && ending.estimate.maxSeconds >= ending.estimate.minSeconds, 'SEASON_ESTIMATE', '季终估时须为正数区间；估时不等于实际时长');
  if (ending.sourceRefs !== undefined) need(Array.isArray(ending.sourceRefs) && ending.sourceRefs.length > 0 && ending.sourceRefs.every(text) && new Set(ending.sourceRefs).size === ending.sourceRefs.length, 'SEASON_ENDING', '附加季终源引用须非空且不重复');
  if (scope.status === 'locked' || scope.confirmation !== undefined) confirmation(scope.confirmation, 'SEASON_CONFIRMATION');
  if (scope.durationException !== undefined) {
    need(object(scope.durationException) && text(scope.durationException.basis), 'SEASON_DURATION_EXCEPTION', '时长偏离须记录具体依据');
    confirmation(scope.durationException.confirmation, 'SEASON_DURATION_EXCEPTION');
  }
  if (scope.status === 'locked') need(scope.durationException !== undefined || ending.estimate.maxSeconds >= TARGET.minSeconds && ending.estimate.minSeconds <= TARGET.maxSeconds, 'SEASON_ESTIMATE_OUTSIDE_TARGET', '候选估时区间与120–150分钟不相交；保留proposed，先调整季终或记录针对该偏离的明确授权，不能先投入资产与分镜');
  if (index !== undefined) {
    need(object(index) && index.version === 'series-source-v1', 'SEASON_INDEX', '源索引version须为series-source-v1');
    need(index.sourceHash === scope.sourceHash, 'SEASON_SOURCE_HASH', '源索引与productionScope原件sourceHash不一致');
    need(Array.isArray(index.episodes), 'SEASON_INDEX', '源索引缺少episodes');
    const indexed = catalog(index.episodes.map(episode => ({number: episode?.number, sceneIds: Array.isArray(episode?.scenes) ? episode.scenes.map(scene => scene?.id) : undefined})), 'SEASON_READING_SCOPE', '源索引');
    if (own(index, 'readingScope') || own(index, 'scopeHash')) {
      need(equal(catalog(index.readingScope, 'SEASON_READING_SCOPE', '投影全剧readingScope'), reading), 'SEASON_READING_SCOPE', '投影索引完整readingScope与范围漂移');
      need(object(index.productionScope) && hashSeasonScope(index.productionScope) === hashSeasonScope(scope) && index.scopeHash === hashSeasonScope(scope), 'SEASON_SCOPE_HASH', '投影索引productionScope或scopeHash不一致');
      need(equal(indexed, selected), 'SEASON_READING_SCOPE', '投影索引所选集场与productionScope不一致');
    } else {
      need(equal(indexed, reading), 'SEASON_READING_SCOPE', 'readingScope须完整对应原始源索引全部集场及源顺序');
      if (own(index, 'productionScope')) {
        need(object(index.productionScope), 'SEASON_SCOPE_HASH', '完整源索引嵌入的productionScope无效');
        const embeddedHash = hashSeasonScope(index.productionScope);
        if (embeddedHash !== hashSeasonScope(scope)) {
          need(scope.seasonNumber > 1 && embeddedHash === scope.continuation.previousScopeHash && index.productionScope.id === scope.continuation.previousSeasonId, 'SEASON_SCOPE_HASH', '完整源索引须绑定当前范围或continuation精确引用的前季范围');
          // Only a full source index may carry the preceding scope; a projected index stays fixed.
          validateSeasonScope(index.productionScope, {sourceHash: scope.sourceHash, index: {version: index.version, sourceHash: index.sourceHash, episodes: index.episodes}});
        }
      }
    }
  }
  canonical(scope);
  return scope;
}

export function assertSeasonUse(scope, {episode, sceneId} = {}, consumer = '消费者') {
  validateSeasonScope(scope);
  const selected = scope.episodes.find(item => item.number === episode);
  need(selected && (sceneId === undefined || selected.sceneIds.includes(sceneId)), 'SEASON_OUT_OF_SCOPE', `${consumer}使用的集场不在已锁定制作范围内：${episode}/${sceneId ?? '*'}`);
  return scope;
}

function durationTicks(value) {
  need(Number.isFinite(value) && value > 0, 'SEASON_DURATION', '实际镜头时长须为正有限数');
  const scaled = value * TICKS, ticks = Math.round(scaled);
  need(Number.isSafeInteger(ticks) && ticks > 0 && Math.abs(scaled - ticks) <= Number.EPSILON * Math.max(1, Math.abs(scaled)) * 4, 'SEASON_DURATION', '实际镜头时间超出9位小数精度，不能静默改时长');
  return ticks;
}

export function checkSeasonBundle(bundle, {scope, requireComplete = false} = {}) {
  need(object(bundle), 'SEASON_BUNDLE', '分镜包须为JSON对象');
  const embedded = own(bundle, 'productionScope');
  if (!embedded) {
    need(scope === undefined && !own(bundle, 'scopeHash') && !own(bundle, 'seasonDelivery'), 'SEASON_SCOPE_REQUIRED', '已声明季范围不能删除productionScope后降级为旧稿');
    return {legacy: true, scopeHash: null, coverage: null, duration: null};
  }
  validateSeasonScope(bundle.productionScope, {sourceHash: bundle.sourceHash});
  const active = scope === undefined ? bundle.productionScope : validateSeasonScope(scope, {sourceHash: bundle.sourceHash});
  const scopeHash = hashSeasonScope(active);
  need(hashSeasonScope(bundle.productionScope) === scopeHash && (!own(bundle, 'scopeHash') || bundle.scopeHash === scopeHash), 'SEASON_SCOPE_HASH', '分镜包productionScope或scopeHash与已锁定范围不一致');
  if (own(bundle, 'readingScope')) need(equal(catalog(bundle.readingScope, 'SEASON_READING_SCOPE', '分镜包readingScope'), catalog(active.readingScope, 'SEASON_READING_SCOPE', '范围readingScope')), 'SEASON_READING_SCOPE', '分镜包完整阅读范围漂移');
  need(bundle.seasonDelivery === undefined || ['preview', 'final'].includes(bundle.seasonDelivery), 'SEASON_DELIVERY', 'seasonDelivery只支持preview或final');
  need(Array.isArray(bundle.episodes), 'SEASON_BUNDLE', '分镜包缺少集场目录');
  const selectedEpisodes = active.episodes.map(episode => episode.number);
  const selectedScenes = active.episodes.flatMap(episode => episode.sceneIds.map(sceneId => ({episode: episode.number, sceneId})));
  const covered = new Set(), seenEpisodes = new Set();
  let previousEpisode = -1, actualTicks = 0;
  const local = [];
  for (const episode of bundle.episodes) {
    const position = selectedEpisodes.indexOf(episode?.number);
    need(position >= 0, 'SEASON_OUT_OF_SCOPE', '分镜包出现制作范围外的集');
    need(!seenEpisodes.has(episode.number) && position > previousEpisode, 'SEASON_BUNDLE_ORDER', '分镜包集目录重复或未按锁定范围源顺序');
    seenEpisodes.add(episode.number); previousEpisode = position;
    const selected = active.episodes[position];
    need(Array.isArray(episode.scenes), 'SEASON_BUNDLE', '分镜包缺少场目录');
    let previousScene = -1;
    const scenes = new Set();
    for (const scene of episode.scenes) {
      const scenePosition = selected.sceneIds.indexOf(scene?.id);
      need(scenePosition >= 0, 'SEASON_OUT_OF_SCOPE', '分镜包出现制作范围外的场');
      need(!scenes.has(scene.id) && scenePosition > previousScene, 'SEASON_BUNDLE_ORDER', '分镜包场目录重复或未按源顺序');
      scenes.add(scene.id); previousScene = scenePosition;
    }
    local.push({number: episode.number, sceneIds: episode.scenes.map(scene => scene.id)});
    const units = episode.draft?.units ?? [];
    need(Array.isArray(units), 'SEASON_BUNDLE', '分镜包实际镜头单元须为数组');
    let previousUnitScene = -1;
    for (const unit of units) {
      need(object(unit) && scenes.has(unit.sceneId), 'SEASON_OUT_OF_SCOPE', '实际镜头单元不属于包内所列制作范围场目录');
      const unitScene = selected.sceneIds.indexOf(unit.sceneId);
      need(unitScene >= previousUnitScene, 'SEASON_BUNDLE_ORDER', '实际镜头单元未按源场顺序');
      previousUnitScene = unitScene;
      need(Array.isArray(unit.shots), 'SEASON_BUNDLE', '实际镜头须为数组');
      for (const shot of unit.shots) {
        actualTicks += durationTicks(shot?.duration);
        need(Number.isSafeInteger(actualTicks), 'SEASON_DURATION', '实际总时长超出可安全计算的时间精度');
        covered.add(JSON.stringify([episode.number, unit.sceneId]));
      }
    }
  }
  if (own(bundle, 'scope')) need(equal(catalog(bundle.scope, 'SEASON_BUNDLE_SCOPE', '分镜包scope', true), local), 'SEASON_BUNDLE_SCOPE', '分镜包scope与实际集场目录不一致');
  const coveredScenes = selectedScenes.filter(item => covered.has(JSON.stringify([item.episode, item.sceneId])));
  const missingScenes = selectedScenes.filter(item => !covered.has(JSON.stringify([item.episode, item.sceneId])));
  const coveredEpisodes = active.episodes.filter(episode => episode.sceneIds.every(sceneId => covered.has(JSON.stringify([episode.number, sceneId])))).map(episode => episode.number);
  const coverage = {selectedEpisodes, coveredEpisodes, missingEpisodes: selectedEpisodes.filter(number => !coveredEpisodes.includes(number)), selectedScenes, coveredScenes, missingScenes, complete: missingScenes.length === 0};
  const actualSeconds = actualTicks / TICKS;
  const within = actualTicks >= active.target.minSeconds * TICKS && actualTicks <= active.target.maxSeconds * TICKS;
  const duration = {actualSeconds, target: {...active.target}, status: within ? 'within-target' : active.durationException ? 'exception-authorized' : 'outside-target'};
  if (!within && active.durationException) duration.exception = active.durationException;
  if (requireComplete || bundle.seasonDelivery === 'final') {
    need(coverage.complete, 'SEASON_INCOMPLETE', '最终交付须有选季全部集场的实际分镜，目录或局部预览不能算完整');
    need(within || active.durationException !== undefined, 'SEASON_DURATION_TARGET', `实际分镜总时长${actualSeconds}秒未在7200–9000秒内；须保留原生季终并记录明确时长偏离授权，不能padding`);
  }
  return {legacy: false, scopeHash, coverage, duration};
}

async function main(args) {
  const [command, ...rest] = args, flags = {};
  need(['check', 'index'].includes(command), 'SEASON_CLI', '用法：season-scope.mjs check|index --input SCOPE.json [--index SOURCE-INDEX.json] [--output projected-index.json|-]');
  for (let offset = 0; offset < rest.length; offset += 2) {
    const key = rest[offset];
    need(['--input', '--index', '--output'].includes(key) && rest[offset + 1] && !own(flags, key), 'SEASON_CLI', '参数缺失、重复或未知');
    flags[key] = rest[offset + 1];
  }
  need(flags['--input'], 'SEASON_CLI', '必须提供--input');
  const scope = JSON.parse(await readFile(flags['--input'], 'utf8'));
  const index = flags['--index'] ? JSON.parse(await readFile(flags['--index'], 'utf8')) : undefined;
  validateSeasonScope(scope, {index, requireLocked: command === 'index'});
  const scopeHash = hashSeasonScope(scope);
  if (command === 'check') {
    need(!flags['--output'], 'SEASON_CLI', 'check不写文件，请用index输出投影索引');
    process.stdout.write(JSON.stringify({ok: true, status: scope.status, sourceHash: scope.sourceHash, scopeHash, episodes: scope.episodes}) + '\n');
  } else {
    need(index && flags['--output'], 'SEASON_CLI', 'index必须提供--index与--output；--output -输出JSON');
    const numbers = new Set(scope.episodes.map(episode => episode.number));
    const projected = {...index, episodes: index.episodes.filter(episode => numbers.has(episode.number)), readingScope: scope.readingScope, productionScope: scope, scopeHash};
    if (flags['--output'] === '-') process.stdout.write(JSON.stringify(projected) + '\n');
    else {
      await writeFile(flags['--output'], JSON.stringify(projected, null, 2) + '\n', {flag: 'wx'});
      process.stdout.write(JSON.stringify({ok: true, output: flags['--output'], scopeHash, episodes: scope.episodes}) + '\n');
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { process.stderr.write(`[${error.code ?? 'SEASON_INPUT'}] ${error.message}\n`); process.exitCode = 1; });
}
