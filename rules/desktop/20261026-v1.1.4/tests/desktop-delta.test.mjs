import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync} from 'node:fs';
import {basename, dirname, join} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {applyDelta, DELTA_VERSION} from '../scripts/apply-delta.mjs';

const entry = fileURLToPath(new URL('../scripts/apply-delta.mjs', import.meta.url));
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const copy = value => structuredClone(value);
const bytes = value => Buffer.from(JSON.stringify(value, null, 2) + '\n');
const makeUnit = (number, lineId = `L${number}`) => ({
  id: `U${number}`, title: `单元${number}`, scene: '门厅', unitDirection: '保持具体观看进程。',
  initialState: '站在门边。', exitState: '仍在门边。',
  nested: {keep: ['原标点，', {text: '多行\n正文', empty: null}]},
  shots: [{id: `S${number}`, duration: 2.5, camera: '近景固定', picture: `人物${number}看向门边。`,
    lighting: '窗侧光', sound: '室内底声', transition: '接门边视线。',
    dialogueParts: [{lineId, text: `原声${number}。`, presentation: '画内', voiceStart: 0, voiceEnd: 2}]}],
});
const fantasy = (id, unitIds, shotIds) => ({candidateId: id, status: 'realized', unitIds, shotIds,
  screenSeconds: 2, entry: '随原刺激进入。', exit: '接回原动作。', domain: 'subjective', preservedFunction: '保留关系反差。'});
function fixture({dialogue = true, fantasies = true} = {}) {
  const units = [makeUnit(1, dialogue ? 'L12' : 'L1'), makeUnit(2, dialogue ? 'L12' : 'L2'), makeUnit(3), makeUnit(4)];
  return {schemaVersion: 'performance-core-v4', extra: {source: '不改源字段'}, units,
    fantasyRealization: fantasies ? [fantasy('F12', ['U1', 'U2'], ['S1', 'S2']), fantasy('F4', ['U4'], ['S4'])] : [],
    revisionNotes: '基稿说明', unresolved: []};
}
function patch(base, groupIds = [['U1', 'U2']]) {
  const replaced = new Set(groupIds.flat());
  return {version: DELTA_VERSION, baseDraftHash: sha256(bytes(base)),
    retainedUnitIds: base.units.filter(unit => !replaced.has(unit.id)).map(unit => unit.id),
    replacements: groupIds.map(unitIds => ({unitIds, units: base.units.filter(unit => unitIds.includes(unit.id)).map(unit => {
      const result = copy(unit); result.shots[0].picture += '听到落词时目光骤停。'; return result;
    })})),
    revisionNotes: '只修实际观看落点，保留原声与关联范围。', unresolved: [],
    fantasyRealization: copy(base.fantasyRealization)};
}
function rejects(base, delta, code) {
  assert.throws(() => applyDelta(bytes(base), delta), error => error.code === code,
    `expected ${code}`);
}

test('完整关联组生成候选；未改单元序列化逐字一致，原输入无写入', () => {
  const base = fixture(), delta = patch(base), before = JSON.stringify({base, delta});
  delta.fantasyRealization[0].entry = '关键词出口时进入同一主观关系。';
  const deltaBefore = JSON.stringify(delta), result = applyDelta(bytes(base), delta);
  assert.deepEqual(result.units.map(unit => unit.id), ['U1', 'U2', 'U3', 'U4']);
  assert.equal(JSON.stringify(result.units.slice(2)), JSON.stringify(base.units.slice(2)));
  assert.deepEqual(result.extra, base.extra);
  assert.deepEqual(result.fantasyRealization[1], base.fantasyRealization[1]);
  assert.equal(result.revisionNotes, delta.revisionNotes);
  assert.equal(JSON.stringify(delta), deltaBefore);
  assert.equal(JSON.stringify(base), JSON.stringify(JSON.parse(before).base));
});

test('hash 使用原文件 bytes，接受 Buffer/Uint8Array/字符串；仅空白变化也过期', () => {
  const base = fixture(), delta = patch(base), raw = bytes(base);
  for (const data of [raw, new Uint8Array(raw), raw.toString('utf8')]) {
    assert.equal(applyDelta(data, delta).units.length, 4);
  }
  assert.throws(() => applyDelta(Buffer.concat([raw, Buffer.from('\n')]), delta), {code: 'DELTA_STALE'});
  assert.throws(() => applyDelta(base, delta), {code: 'DELTA_BASE_BYTES'});
});

test('替换数组顺序不决定成稿顺序，输出沿 base；允许完整关联组拆合单元', () => {
  const base = fixture(), delta = patch(base, [['U4'], ['U1', 'U2']]);
  const merged = copy(base.units[0]);
  merged.id = 'U12'; merged.shots = [copy(base.units[0].shots[0]), copy(base.units[1].shots[0])];
  delta.replacements[1].units = [merged];
  delta.fantasyRealization[0].unitIds = ['U12'];
  const result = applyDelta(bytes(base), delta);
  assert.deepEqual(result.units.map(unit => unit.id), ['U12', 'U3', 'U4']);
  assert.deepEqual(result.units[0].shots.map(shot => shot.id), ['S1', 'S2']);
});

for (const [name, mutate] of [
  ['缺覆盖', delta => delta.retainedUnitIds.pop()],
  ['重复保留', delta => delta.retainedUnitIds.push('U3')],
  ['同时保留和替换', delta => delta.retainedUnitIds.push('U1')],
  ['替换组重叠', delta => delta.replacements.push(copy(delta.replacements[0]))],
  ['未知保留 ID', delta => delta.retainedUnitIds.push('OTHER')],
]) test(`拒绝${name}`, () => {
  const base = fixture(), delta = patch(base); mutate(delta); rejects(base, delta, 'DELTA_PARTITION');
});

for (const unitIds of [['U1', 'U3'], ['U2', 'U1'], ['U1', 'MISSING']]) test(`拒绝非连续源序组 ${unitIds}`, () => {
  const base = fixture(), delta = patch(base); delta.replacements[0].unitIds = unitIds;
  rejects(base, delta, 'DELTA_ORDER');
});

for (const [name, mutate] of [
  ['新单元重复', delta => delta.replacements[0].units[1].id = 'U1'],
  ['撞保留单元', delta => delta.replacements[0].units[0].id = 'U3'],
  ['撞保留镜头', delta => delta.replacements[0].units[0].shots[0].id = 'S3'],
  ['新镜头重复', delta => delta.replacements[0].units[1].shots[0].id = 'S1'],
  ['unit/shot 跨层同名', delta => delta.replacements[0].units[0].shots[0].id = 'U2'],
]) test(`拒绝 ID collision：${name}`, () => {
  const base = fixture(), delta = patch(base); mutate(delta); rejects(base, delta, 'DELTA_ID');
});

test('分别合法的两个替换组也不能共享新 ID', () => {
  const base = fixture({dialogue: false, fantasies: false}), delta = patch(base, [['U1'], ['U2']]);
  delta.replacements[0].units[0].shots[0].id = 'S-new';
  delta.replacements[1].units[0].shots[0].id = 'S-new';
  rejects(base, delta, 'DELTA_ID');
});

test('旧稿 ID collision 不能借替换掩盖', () => {
  const base = fixture(); base.units[3].shots[0].id = 'S3';
  rejects(base, patch(base), 'DELTA_ID');
});

test('原句关联从旧稿计算，删掉新稿 dialogueParts 不能只改半句所在单元', () => {
  const base = fixture({fantasies: false}), delta = patch(base, [['U1']]);
  delta.replacements[0].units[0].shots[0].dialogueParts = [];
  rejects(base, delta, 'DELTA_DIALOGUE_GROUP');
});

test('同一原句跨单元不能拆成两个独立替换组', () => {
  const base = fixture({fantasies: false}), delta = patch(base, [['U1'], ['U2']]);
  rejects(base, delta, 'DELTA_DIALOGUE_GROUP');
});

test('完整原句关联必须递归闭合到另一句关联的旧单元', () => {
  const base = fixture({fantasies: false});
  base.units[1].shots[0].dialogueParts.push({lineId: 'L23', text: '续句前半'});
  base.units[2].shots[0].dialogueParts = [{lineId: 'L23', text: '续句后半'}];
  rejects(base, patch(base), 'DELTA_DIALOGUE_GROUP');
  assert.equal(applyDelta(bytes(base), patch(base, [['U1', 'U2', 'U3']])).units.length, 4);
});

test('新片段不能挪用属于保留单元的原句', () => {
  const base = fixture({fantasies: false}), delta = patch(base);
  delta.replacements[0].units[0].shots[0].dialogueParts.push({lineId: 'L3', text: '挪入旧句'});
  rejects(base, delta, 'DELTA_DIALOGUE_GROUP');
});

test('幻想关联先查旧账，删除新账不能绕过跨单元范围', () => {
  const base = fixture({dialogue: false}), delta = patch(base, [['U1']]);
  delta.fantasyRealization = [];
  rejects(base, delta, 'DELTA_FANTASY_GROUP');
});

test('只有 screenSpans 的幻想同样保护完整组，完整修订可保留', () => {
  const base = fixture({dialogue: false});
  for (const item of base.fantasyRealization) {
    item.screenSpans = item.shotIds.map(shotId => ({shotId, range: null}));
    delete item.shotIds;
  }
  rejects(base, patch(base, [['U1']]), 'DELTA_FANTASY_GROUP');
  assert.deepEqual(applyDelta(bytes(base), patch(base)).fantasyRealization, base.fantasyRealization);
});

test('跨幻想组必须一次完整替换，不能把两个单元分给不同组', () => {
  const base = fixture({dialogue: false}), delta = patch(base, [['U1'], ['U2']]);
  rejects(base, delta, 'DELTA_FANTASY_GROUP');
});

test('幻想 unitIds 漏掉 shotIds 的归属单元则拒绝，不能凭不全 ledger 放行', () => {
  const base = fixture({dialogue: false});
  base.fantasyRealization[0].unitIds = ['U1'];
  rejects(base, patch(base), 'DELTA_FANTASY_REF');
});

test('受替换影响的幻想缺少完整新版账目时拒绝', () => {
  const base = fixture(), delta = patch(base); delete delta.fantasyRealization;
  rejects(base, delta, 'DELTA_FANTASY_REQUIRED');
});

for (const [name, mutate] of [
  ['删候选', delta => delta.fantasyRealization.pop()],
  ['加候选', delta => delta.fantasyRealization.push({...copy(delta.fantasyRealization[0]), candidateId: 'EXTRA'})],
  ['候选重复', delta => delta.fantasyRealization.push(copy(delta.fantasyRealization[0]))],
]) test(`不允许${name}改变幻想选择集合`, () => {
  const base = fixture(), delta = patch(base); mutate(delta); rejects(base, delta, 'DELTA_FANTASY_SELECTION');
});

test('未受影响幻想逐值只读，不能借完整新版数组改写', () => {
  const base = fixture(), delta = patch(base);
  delta.fantasyRealization[1].entry = '越界改写';
  rejects(base, delta, 'DELTA_FANTASY_RETAINED');
});

test('已兑现幻想不能降为 not_realized 或清空引用', () => {
  const base = fixture(), delta = patch(base);
  delta.fantasyRealization[0].status = 'not_realized';
  delta.fantasyRealization[0].unitIds = []; delta.fantasyRealization[0].shotIds = [];
  rejects(base, delta, 'DELTA_FANTASY_DROPPED');
  delta.fantasyRealization[0].status = 'realized';
  rejects(base, delta, 'DELTA_FANTASY_REF');
});

test('受影响幻想不能转移到保留单元或另一替换组', () => {
  const base = fixture(), delta = patch(base);
  delta.fantasyRealization[0].unitIds = ['U4']; delta.fantasyRealization[0].shotIds = ['S4'];
  rejects(base, delta, 'DELTA_FANTASY_SCOPE');
});

test('幻想保持全部候选及未影响记录；输入数组改序不重排旧候选身份', () => {
  const base = fixture(), delta = patch(base);
  delta.fantasyRealization.reverse();
  assert.deepEqual(applyDelta(bytes(base), delta).fantasyRealization.map(item => item.candidateId), ['F12', 'F4']);
});

test('无受影响幻想时可省略新版 ledger；非空 unresolved 留在候选，不自动通过', () => {
  const base = fixture(), delta = patch(base, [['U3']]);
  delete delta.fantasyRealization;
  delta.unresolved = [{coordinate: 'U3', problem: '仍需源核对'}];
  const result = applyDelta(bytes(base), delta);
  assert.deepEqual(result.fantasyRealization, base.fantasyRealization);
  assert.deepEqual(result.unresolved, delta.unresolved);
});

test('全保留的零改动仍只返回候选，不改未影响内容', () => {
  const base = fixture(), delta = patch(base, []);
  delete delta.fantasyRealization;
  const result = applyDelta(bytes(base), delta);
  assert.equal(JSON.stringify(result.units), JSON.stringify(base.units));
  assert.equal(JSON.stringify(result.fantasyRealization), JSON.stringify(base.fantasyRealization));
});

test('null 旧幻想账不能被当成空数组悄悄清除', () => {
  const base = fixture(); base.fantasyRealization = null;
  const delta = {version: DELTA_VERSION, baseDraftHash: sha256(bytes(base)),
    retainedUnitIds: base.units.map(unit => unit.id), replacements: [], revisionNotes: '核对', unresolved: []};
  rejects(base, delta, 'DELTA_FANTASY');
});

for (const [name, mutate, code] of [
  ['错误版本', delta => delta.version = 'unknown', 'DELTA_SHAPE'],
  ['空修订说明', delta => delta.revisionNotes = '  ', 'DELTA_SHAPE'],
  ['缺 unresolved', delta => delete delta.unresolved, 'DELTA_SHAPE'],
  ['根字段夹带改稿', delta => delta.units = [], 'DELTA_SHAPE'],
  ['空替换单元', delta => delta.replacements[0].units = [], 'DELTA_UNITS'],
  ['不完整镜头数组', delta => delete delta.replacements[0].units[0].shots, 'DELTA_UNITS'],
  ['隐藏原句账', delta => delete delta.replacements[0].units[0].shots[0].dialogueParts, 'DELTA_DIALOGUE'],
  ['非 JSON 值', delta => delta.unresolved.push(undefined), 'DELTA_SHAPE'],
]) test(`拒绝无效差量：${name}`, () => {
  const base = fixture(), delta = patch(base); mutate(delta); rejects(base, delta, code);
});

function sandbox(t) {
  const root = realpathSync(tmpdir()), dir = mkdtempSync(join(root, 'desktop-delta-test-'));
  t.after(() => {
    const target = realpathSync(dir);
    assert.equal(dirname(target), root);
    assert.ok(basename(target).startsWith('desktop-delta-test-'));
    rmSync(target, {recursive: true, force: true});
  });
  return dir;
}
const cli = args => spawnSync(process.execPath, [entry, ...args], {encoding: 'utf8'});
function files(dir) {
  const base = fixture(), delta = patch(base), baseFile = join(dir, 'base.json'), deltaFile = join(dir, 'delta.json');
  writeFileSync(baseFile, bytes(base)); writeFileSync(deltaFile, bytes(delta));
  return {base, delta, baseFile, deltaFile, output: join(dir, 'candidate.json')};
}

test('CLI 排他创建候选，不覆盖已有输出或输入，明确尚未采用/内容验证', t => {
  const data = files(sandbox(t)), args = ['--base', data.baseFile, '--delta', data.deltaFile, '--output', data.output];
  const beforeBase = readFileSync(data.baseFile), beforeDelta = readFileSync(data.deltaFile);
  const first = cli(args);
  assert.equal(first.status, 0, first.stderr);
  const result = JSON.parse(first.stdout);
  assert.equal(result.status, 'candidate'); assert.equal(result.contentValidated, false); assert.equal(result.adopted, false);
  const candidate = readFileSync(data.output);
  assert.equal(result.candidateDraftHash, sha256(candidate));
  assert.deepEqual(JSON.parse(candidate).units[2], data.base.units[2]);
  const repeated = cli(args);
  assert.notEqual(repeated.status, 0); assert.equal(JSON.parse(repeated.stderr).code, 'EEXIST');
  assert.deepEqual(readFileSync(data.output), candidate);
  assert.notEqual(cli(['--base', data.baseFile, '--delta', data.deltaFile, '--output', data.baseFile]).status, 0);
  assert.deepEqual(readFileSync(data.baseFile), beforeBase);
  assert.deepEqual(readFileSync(data.deltaFile), beforeDelta);
});

test('CLI 无效或过期差量不会创建输出文件', t => {
  const data = files(sandbox(t)); data.delta.baseDraftHash = '0'.repeat(64);
  writeFileSync(data.deltaFile, bytes(data.delta));
  const result = cli(['--base', data.baseFile, '--delta', data.deltaFile, '--output', data.output]);
  assert.notEqual(result.status, 0); assert.equal(JSON.parse(result.stderr).code, 'DELTA_STALE');
  assert.equal(existsSync(data.output), false);
});

test('CLI 参数及非法 UTF-8 JSON 失败清晰；--help 可直接运行', t => {
  assert.equal(cli(['--help']).status, 0);
  assert.equal(JSON.parse(cli(['--base', 'x']).stderr).code, 'DELTA_ARGS');
  const data = files(sandbox(t));
  writeFileSync(data.deltaFile, Buffer.from([0xff]));
  const result = cli(['--base', data.baseFile, '--delta', data.deltaFile, '--output', data.output]);
  assert.notEqual(result.status, 0); assert.equal(JSON.parse(result.stderr).code, 'DELTA_JSON');
  assert.equal(existsSync(data.output), false);
});
